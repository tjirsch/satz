//! `satz adopt`: bring resources the estate declares, and that already exist
//! live, under Terraform management — the general case of which
//! `adopt-org-policies` was the first instance.
//!
//! Shape, decided 2026-08-29 (private roadmap, "Adoption & import"):
//! - the input is the **emission manifest** — the resources `apply` will act
//!   on, with their attributes and references — never the source text;
//! - identity splits: a type whose import id is user-chosen renders it from a
//!   template in `import-config.yaml` (`import_id:`), offline; a type whose
//!   id GCP assigns is looked up by its natural key (folder by display name
//!   under the resolved parent, group by email, membership by group + email,
//!   org policy by constraint under its parent);
//! - resolution is top-down so scope is always exact, and it **never
//!   guesses**: exactly one candidate resolves, zero is "apply will create
//!   it", more than one is ambiguous and stops that subtree;
//! - the only language surface is `"import-id"`, the carried result. `--write`
//!   persists verified ids into the declaring `.satz`; `--import` runs
//!   `tofu import` now; the default is a dry run.
//!
//! The engine is pure over a `Live` trait so every rule is unit-tested without
//! a network; `RealLive` binds it to the GCP clients.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::ImportConfig;
use crate::manifest::{EmittedResource, Manifest};

/// What a natural-key lookup found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    One(String),
    Absent,
    Many(Vec<String>),
}

/// The live questions the engine asks. `async fn` in a trait: one concrete
/// implementation per binary, generics at the call site, no `dyn`.
pub(crate) trait Live {
    async fn folder(&mut self, parent: &str, display_name: &str) -> Result<Lookup, String>;
    /// Does the project exist? `Ok(Some(projects/<number>))` when it does,
    /// `Ok(None)` when it provably does not (404), `Err` when the question
    /// itself could not be answered (denied, quota) — never absent-by-error.
    async fn project(&mut self, project_id: &str) -> Result<Option<String>, String>;
    async fn group(&mut self, email: &str) -> Result<Option<String>, String>;
    async fn membership(&mut self, group_name: &str, email: &str) -> Result<Option<String>, String>;
    async fn org_policy_exists(&mut self, parent: &str, constraint: &str) -> Result<bool, String>;
    /// Every live asset of `asset_type` under `scope` (`organizations/<n>`,
    /// `folders/<n>`, `projects/<id>`): its resource path (the CAI name without
    /// the `//<service>/` prefix — which is the Terraform import id for the
    /// types that carry one) and its resource data.
    async fn search(&mut self, scope: &str, asset_type: &str) -> Result<Vec<(String, serde_json::Value)>, String>;
    /// Every budget of a billing account: (resource name
    /// `billingAccounts/<id>/budgets/<uuid>`, display name). Budgets are not
    /// in Cloud Asset Inventory — the Billing Budgets API is the only lookup.
    async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The estate already carries an `"import-id"` for it.
    AlreadyAdopted(String),
    /// Found live (natural-key lookup), or its id is user-chosen and was rendered
    /// from the rule (`verified: false` — existence is not known offline).
    Resolved { id: String, verified: bool },
    /// A managed org-policy constraint the organisation has never had: it must
    /// be activated before it can be imported (`--activate`).
    NeedsActivation { id: String, enforce: Option<bool> },
    /// Looked up, provably absent: `apply` will create it, nothing to import.
    OnApply,
    /// Its parent (the project it lives in) is not live — it will be created
    /// together with the parent. Never written, never imported: an import id
    /// derived from a non-existent parent is a guess, not a finding.
    ParentOnApply(String),
    /// More than one live candidate. Never guessed; pin `"import-id"` by hand.
    Ambiguous(Vec<String>),
    /// A rule exists but needs a lookup this version cannot do yet.
    NeedsLookup(String),
    /// No adoption rule for the type — add `import_id` or `match_on` to the
    /// discovery config row.
    NoRule,
    /// A template placeholder or a parent reference could not be resolved.
    Unresolvable(String),
    /// The lookup itself failed.
    Failed(String),
    /// Filtered out by `--only`.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolution {
    pub address: String,
    pub tf_type: String,
    /// What it was matched on (display name, email, constraint, or the rendered template).
    pub natural_key: String,
    pub outcome: Outcome,
    pub origin: Option<(String, u32)>,
    /// For org policies: (parent, constraint) — needed by activation and import.
    pub org_policy: Option<(String, String)>,
}

pub(crate) struct Options {
    pub only: BTreeSet<String>,
    pub activate: bool,
}

/// Rule for one resource type, from `import-config.yaml`.
enum Rule {
    Template(String),
    /// (attributes to match on, CAI asset type to list)
    Match(Vec<String>, Option<String>),
    None,
}

fn rule_for(rules: &ImportConfig, tf_type: &str) -> Rule {
    match rules.resource_types.get(tf_type) {
        Some(r) if r.import_id.is_some() => Rule::Template(r.import_id.clone().unwrap()),
        Some(r) if r.match_on.is_some() => Rule::Match(r.match_on.clone().unwrap(), r.asset_type.clone()),
        _ => Rule::None,
    }
}

/// Resolve every resource in the manifest. Folders first, outermost first, so
/// a child's parent reference is already answered when it is reached.
pub(crate) async fn resolve<L: Live>(
    manifest: &Manifest,
    rules: &ImportConfig,
    opts: &Options,
    live: &mut L,
) -> Vec<Resolution> {
    let mut resolved_ids: BTreeMap<String, String> = BTreeMap::new();
    // Projects that are not live (absent, or unanswerable), by address and by
    // project id — a child names its project either by reference or by the
    // literal id, and both must read the same verdict.
    let mut not_live: BTreeMap<String, Outcome> = BTreeMap::new();
    let mut out = Vec::new();
    for r in ordered(manifest) {
        let mut res = Resolution {
            address: r.address(),
            tf_type: r.tf_type.clone(),
            natural_key: String::new(),
            outcome: Outcome::NoRule,
            origin: r.origin.clone(),
            org_policy: None,
        };
        if let Some(id) = &r.import_id {
            resolved_ids.insert(r.address(), id.clone());
            res.outcome = Outcome::AlreadyAdopted(id.clone());
            out.push(res);
            continue;
        }
        if !opts.only.is_empty() && !opts.only.contains(&r.tf_type) {
            res.outcome = Outcome::Skipped;
            out.push(res);
            continue;
        }
        // A resource inside a project that is not live inherits that verdict
        // before any rule runs: nothing under it can be adopted yet.
        if let Some(parent_verdict) = parent_not_live(r, manifest, &not_live) {
            res.outcome = parent_verdict;
            out.push(res);
            continue;
        }
        let outcome = match r.tf_type.as_str() {
            "google_folder" => resolve_folder(r, manifest, &resolved_ids, live).await,
            "google_project" => resolve_project(r, live).await,
            "google_cloud_identity_group" => resolve_group(r, live).await,
            "google_cloud_identity_group_membership" => resolve_membership(r, &resolved_ids, live).await,
            "google_org_policy_policy" => resolve_org_policy(r, manifest, &resolved_ids, opts, live, &mut res).await,
            "google_billing_budget" => resolve_budget(r, live).await,
            _ => match rule_for(rules, &r.tf_type) {
                Rule::Template(t) => render_template(&t, r, manifest, &resolved_ids),
                Rule::Match(on, asset_type) => resolve_match(r, &on, asset_type.as_deref(), manifest, &resolved_ids, live).await,
                Rule::None => (String::new(), Outcome::NoRule),
            },
        };
        res.natural_key = outcome.0;
        res.outcome = outcome.1;
        if let Outcome::Resolved { id, .. } | Outcome::NeedsActivation { id, .. } = &res.outcome {
            resolved_ids.insert(r.address(), id.clone());
        }
        if r.tf_type == "google_project" {
            if let Some(verdict) = project_verdict_for_children(r, &res.outcome) {
                not_live.insert(r.address(), verdict.clone());
                if let Some(pid) = r.attrs.get("project_id") {
                    not_live.insert(pid.clone(), verdict);
                }
            }
        }
        out.push(res);
    }
    out
}

/// What a project's outcome means for the resources inside it: `None` when
/// they can be resolved normally; the verdict they inherit otherwise.
fn project_verdict_for_children(r: &EmittedResource, outcome: &Outcome) -> Option<Outcome> {
    match outcome {
        Outcome::Resolved { .. } | Outcome::AlreadyAdopted(_) | Outcome::Skipped => None,
        Outcome::OnApply => Some(Outcome::ParentOnApply(format!("{} is not live — created with it", r.address()))),
        Outcome::Failed(e) => Some(Outcome::Failed(format!("{}: {}", r.address(), e))),
        other => Some(Outcome::Unresolvable(format!("{}: {:?}", r.address(), other))),
    }
}

/// The verdict `r` inherits from a not-live project it belongs to, if any:
/// by reference (`project = google_project.x.project_id`, or `parent` on a
/// project-scoped policy) or by the literal project id.
fn parent_not_live(r: &EmittedResource, manifest: &Manifest, not_live: &BTreeMap<String, Outcome>) -> Option<Outcome> {
    if r.tf_type == "google_project" || not_live.is_empty() {
        return None;
    }
    for key in ["project", "parent"] {
        if let Some((target, _)) = r.refs.get(key).and_then(|t| ref_target(t)) {
            if manifest.resources.get(&target).is_some_and(|t| t.tf_type == "google_project") {
                if let Some(v) = not_live.get(&target) {
                    return Some(v.clone());
                }
            }
        }
        if let Some(lit) = r.attrs.get(key) {
            let pid = lit.trim_start_matches("projects/");
            if let Some(v) = not_live.get(pid) {
                return Some(v.clone());
            }
        }
    }
    None
}

/// A project is looked up by its id — an existence check, no natural-key
/// matching (project ids are user-chosen and global). Exists → the import
/// id IS the project id, verified; provably absent → `apply` creates it.
async fn resolve_project<L: Live>(r: &EmittedResource, live: &mut L) -> (String, Outcome) {
    let Some(project_id) = r.attrs.get("project_id").cloned() else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no literal project_id", r.address())));
    };
    match live.project(&project_id).await {
        Ok(Some(_number)) => (project_id.clone(), Outcome::Resolved { id: project_id, verified: true }),
        Ok(None) => (project_id, Outcome::OnApply),
        Err(e) => (project_id, Outcome::Failed(e)),
    }
}

/// Folders by depth (parent chain length), then everything else by address.
fn ordered(manifest: &Manifest) -> Vec<&EmittedResource> {
    let depth = |r: &EmittedResource| -> usize {
        let mut d = 0;
        let mut cur = r;
        loop {
            match cur.refs.get("parent").and_then(|p| ref_target(p)).and_then(|(a, _)| manifest.resources.get(&a)) {
                Some(p) if p.tf_type == "google_folder" && d < 64 => {
                    d += 1;
                    cur = p;
                }
                _ => return d,
            }
        }
    };
    let mut folders: Vec<&EmittedResource> = manifest.of_type("google_folder").collect();
    folders.sort_by_key(|r| (depth(r), r.address()));
    // Projects next: their existence decides whether anything inside them
    // can be adopted at all.
    let mut projects: Vec<&EmittedResource> = manifest.of_type("google_project").collect();
    projects.sort_by_key(|r| r.address());
    let rest = manifest
        .resources
        .values()
        .filter(|r| r.tf_type != "google_folder" && r.tf_type != "google_project");
    folders.into_iter().chain(projects).chain(rest).collect()
}

/// `google_T.L.A` → (`google_T.L`, `A`).
fn ref_target(traversal: &str) -> Option<(String, String)> {
    let (addr, attr) = traversal.rsplit_once('.')?;
    if addr.matches('.').count() != 1 {
        return None;
    }
    Some((addr.to_string(), attr.to_string()))
}

/// The value of attribute `key` on `r`: a literal, or a reference followed to
/// the resource it names — its resolved live id when that is what the
/// reference denotes (`google_folder.x.name`, `google_cloud_identity_group.x.id`),
/// else that resource's own attribute (`google_project.x.project_id`).
fn value_of(
    r: &EmittedResource,
    key: &str,
    manifest: &Manifest,
    resolved_ids: &BTreeMap<String, String>,
) -> Result<String, String> {
    if let Some(v) = r.attrs.get(key) {
        return Ok(v.clone());
    }
    let Some(traversal) = r.refs.get(key) else {
        return Err(format!("{} has no `{}`", r.address(), key));
    };
    let Some((target, attr)) = ref_target(traversal) else {
        return Err(format!("{}: `{}` = {} is not a resource reference", r.address(), key, traversal));
    };
    let Some(t) = manifest.resources.get(&target) else {
        return Err(format!("{}: `{}` references {}, which is not emitted", r.address(), key, target));
    };
    let denotes_live_id = matches!(
        (t.tf_type.as_str(), attr.as_str()),
        ("google_folder", "name") | ("google_folder", "id") | ("google_cloud_identity_group", "id") | ("google_cloud_identity_group", "name")
    );
    if denotes_live_id {
        return resolved_ids
            .get(&target)
            .cloned()
            .ok_or_else(|| format!("{} is not resolved yet ({} on it must be adopted or pinned first)", target, key));
    }
    t.attrs
        .get(&attr)
        .cloned()
        .ok_or_else(|| format!("{}: `{}` references {}.{}, which is not a literal", r.address(), key, target, attr))
}

/// A GCP-assigned id looked up through Cloud Asset Inventory: the assets of
/// the rule's `asset_type` under the resource's own scope, matched on the
/// `match_on` attributes — declared value against the asset data, dotted
/// keys walking the data (`group_key.id` → `groupKey.id`). One candidate
/// resolves, none is on-apply, several are ambiguous. Never a guess.
async fn resolve_match<L: Live>(
    r: &EmittedResource,
    on: &[String],
    asset_type: Option<&str>,
    manifest: &Manifest,
    resolved_ids: &BTreeMap<String, String>,
    live: &mut L,
) -> (String, Outcome) {
    let key_text = on.join(", ");
    // `TODO/UNKNOWN` is the auto-generated placeholder of an unfilled row
    let asset_type = asset_type.filter(|a| !a.starts_with("TODO"));
    let Some(asset_type) = asset_type else {
        return (key_text, Outcome::Unresolvable(format!("{} has match_on but no asset_type in import-config.yaml", r.tf_type)));
    };
    let scope = match match_scope(r, manifest, resolved_ids) {
        Ok(s) => s,
        Err(e) => return (key_text, Outcome::Unresolvable(e)),
    };
    let mut wanted: Vec<(String, String)> = Vec::new();
    for k in on {
        let v = r.attrs.get(k).or_else(|| r.nested.get(k)).cloned();
        let v = match v {
            Some(v) => v,
            None => match value_of(r, k, manifest, resolved_ids) {
                Ok(v) => v,
                Err(e) => return (key_text, Outcome::Unresolvable(e)),
            },
        };
        wanted.push((k.clone(), v));
    }
    let natural_key = format!(
        "{} under {}",
        wanted.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(", "),
        scope
    );
    let assets = match live.search(&scope, asset_type).await {
        Ok(a) => a,
        Err(e) => return (natural_key, Outcome::Failed(e)),
    };
    let hits: Vec<String> = assets
        .into_iter()
        .filter(|(_, data)| wanted.iter().all(|(k, v)| data_at(data, k).as_deref() == Some(v.as_str())))
        .map(|(path, _)| path)
        .collect();
    let outcome = match hits.as_slice() {
        [] => Outcome::OnApply,
        [one] => Outcome::Resolved { id: one.clone(), verified: true },
        many => Outcome::Ambiguous(many.to_vec()),
    };
    (natural_key, outcome)
}

/// The scope to list under: the resource's `parent`, else its project,
/// folder or organization attribute.
fn match_scope(r: &EmittedResource, manifest: &Manifest, resolved_ids: &BTreeMap<String, String>) -> Result<String, String> {
    // the first scope attribute the resource HAS decides; a present attribute
    // that cannot be resolved (its folder is ambiguous, say) is an error, not a
    // reason to try the next one and search the wrong scope
    let has = |k: &str| r.attrs.contains_key(k) || r.refs.contains_key(k);
    if has("parent") {
        return value_of(r, "parent", manifest, resolved_ids);
    }
    if has("project") {
        return value_of(r, "project", manifest, resolved_ids).map(|p| format!("projects/{}", p.trim_start_matches("projects/")));
    }
    if has("folder") {
        return value_of(r, "folder", manifest, resolved_ids).map(|f| format!("folders/{}", f.trim_start_matches("folders/")));
    }
    if has("org_id") {
        return value_of(r, "org_id", manifest, resolved_ids).map(|o| format!("organizations/{}", o.trim_start_matches("organizations/")));
    }
    Err(format!("{} has no parent, project, folder or org_id to scope the lookup", r.address()))
}

/// `group_key.id` → data["groupKey"]["id"], as text.
fn data_at(data: &serde_json::Value, dotted: &str) -> Option<String> {
    let mut cur = data;
    for part in dotted.split('.') {
        let camel = snake_to_camel(part);
        cur = cur.get(&camel).or_else(|| cur.get(part))?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

async fn resolve_folder<L: Live>(
    r: &EmittedResource,
    manifest: &Manifest,
    resolved_ids: &BTreeMap<String, String>,
    live: &mut L,
) -> (String, Outcome) {
    let display_name = r.attrs.get("display_name").cloned().unwrap_or_default();
    let parent = match value_of(r, "parent", manifest, resolved_ids) {
        Ok(p) => p,
        Err(e) => return (display_name, Outcome::Unresolvable(e)),
    };
    let key = format!("{} under {}", display_name, parent);
    match live.folder(&parent, &display_name).await {
        Ok(Lookup::One(id)) => (key, Outcome::Resolved { id, verified: true }),
        Ok(Lookup::Absent) => (key, Outcome::OnApply),
        Ok(Lookup::Many(c)) => (key, Outcome::Ambiguous(c)),
        Err(e) => (key, Outcome::Failed(e)),
    }
}

async fn resolve_group<L: Live>(r: &EmittedResource, live: &mut L) -> (String, Outcome) {
    let Some(email) = r.nested.get("group_key.id").cloned() else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no group_key.id", r.address())));
    };
    match live.group(&email).await {
        Ok(Some(id)) => (email, Outcome::Resolved { id, verified: true }),
        Ok(None) => (email, Outcome::OnApply),
        Err(e) => (email, Outcome::Failed(e)),
    }
}

/// A budget's id is a UUID Google assigns; the natural key is the display
/// name under the billing account it is declared for. One live budget with
/// that name resolves, several are ambiguous, none means `apply` creates it.
async fn resolve_budget<L: Live>(r: &EmittedResource, live: &mut L) -> (String, Outcome) {
    let Some(account) = r.attrs.get("billing_account").map(|a| a.trim_start_matches("billingAccounts/").to_string()) else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no literal billing_account", r.address())));
    };
    let Some(display_name) = r.attrs.get("display_name").cloned() else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no display_name to match on", r.address())));
    };
    let key = format!("{} @ billingAccounts/{}", display_name, account);
    match live.budgets(&account).await {
        Err(e) => (key, Outcome::Failed(e)),
        Ok(list) => {
            let hits: Vec<String> = list.into_iter().filter(|(_, dn)| *dn == display_name).map(|(name, _)| name).collect();
            match hits.as_slice() {
                [] => (key, Outcome::OnApply),
                [one] => (key, Outcome::Resolved { id: one.clone(), verified: true }),
                many => (key, Outcome::Ambiguous(many.to_vec())),
            }
        }
    }
}

async fn resolve_membership<L: Live>(
    r: &EmittedResource,
    resolved_ids: &BTreeMap<String, String>,
    live: &mut L,
) -> (String, Outcome) {
    let Some(email) = r.nested.get("preferred_member_key.id").cloned() else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no preferred_member_key.id", r.address())));
    };
    let Some((group_addr, _)) = r.refs.get("group").and_then(|g| ref_target(g)) else {
        return (email, Outcome::Unresolvable(format!("{} has no group reference", r.address())));
    };
    let Some(group_name) = resolved_ids.get(&group_addr) else {
        // The group is not live (OnApply) or could not be resolved: neither can
        // its memberships be.
        return (email, Outcome::OnApply);
    };
    let key = format!("{} in {}", email, group_name);
    match live.membership(group_name, &email).await {
        Ok(Some(id)) => (key, Outcome::Resolved { id, verified: true }),
        Ok(None) => (key, Outcome::OnApply),
        Err(e) => (key, Outcome::Failed(e)),
    }
}

async fn resolve_org_policy<L: Live>(
    r: &EmittedResource,
    manifest: &Manifest,
    resolved_ids: &BTreeMap<String, String>,
    opts: &Options,
    live: &mut L,
    res: &mut Resolution,
) -> (String, Outcome) {
    let name = r.attrs.get("name").cloned().unwrap_or_default();
    let constraint = crate::org_policy::constraint_name(&name);
    // The compile guarantees `parent` is a literal or a reference; an
    // unresolvable one is reported as such — never scraped out of the policy
    // name, which is how a wrong parent once became a confident lookup.
    let parent = match value_of(r, "parent", manifest, resolved_ids) {
        Ok(p) => match crate::org_policy::qualify_parent(&p) {
            Ok(q) => q,
            Err(e) => return (constraint, Outcome::Unresolvable(format!("{}: {}", r.address(), e))),
        },
        Err(e) => return (constraint, Outcome::Unresolvable(e)),
    };
    res.org_policy = Some((parent.clone(), constraint.clone()));
    let id = crate::org_policy::full_policy_name(&parent, &constraint);
    match live.org_policy_exists(&parent, &constraint).await {
        Ok(true) => (constraint, Outcome::Resolved { id, verified: true }),
        Ok(false) if crate::org_policy::is_managed(&constraint) => {
            if opts.activate {
                (constraint, Outcome::NeedsActivation { id, enforce: r.enforce })
            } else {
                (constraint, Outcome::NeedsLookup("managed constraint is not live — activate it first (--activate)".into()))
            }
        }
        Ok(false) => (constraint, Outcome::OnApply),
        Err(e) => (constraint, Outcome::Failed(e)),
    }
}

/// Render `{placeholder}`s from the resource's attributes and resolved
/// references. `{parent}` follows the same rules as any other key.
fn render_template(
    template: &str,
    r: &EmittedResource,
    manifest: &Manifest,
    resolved_ids: &BTreeMap<String, String>,
) -> (String, Outcome) {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            return (template.to_string(), Outcome::Unresolvable(format!("unterminated placeholder in rule `{}`", template)));
        };
        let key = &rest[start + 1..start + end];
        match value_of(r, key, manifest, resolved_ids) {
            Ok(v) => out.push_str(&v),
            Err(e) => return (template.to_string(), Outcome::Unresolvable(e)),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    (out.clone(), Outcome::Resolved { id: out, verified: false })
}

// ---------------------------------------------------------------------------
// Report and sinks
// ---------------------------------------------------------------------------

pub(crate) fn render_table(resolutions: &[Resolution]) -> String {
    let mut s = String::new();
    let w = resolutions.iter().map(|r| r.address.len()).max().unwrap_or(20).min(72);
    for r in resolutions {
        let (verdict, detail) = match &r.outcome {
            Outcome::AlreadyAdopted(id) => ("adopted", id.clone()),
            Outcome::Resolved { id, verified: true } => ("IMPORT", id.clone()),
            Outcome::Resolved { id, verified: false } => ("import (derived, unverified)", id.clone()),
            Outcome::NeedsActivation { id, .. } => ("ACTIVATE + IMPORT", id.clone()),
            Outcome::OnApply => ("on apply", "not live — apply creates it".into()),
            Outcome::ParentOnApply(why) => ("on apply (parent)", why.clone()),
            Outcome::Ambiguous(c) => ("AMBIGUOUS", format!("{} candidates: {} — pin \"import-id\" by hand", c.len(), c.join(", "))),
            Outcome::NeedsLookup(why) => ("needs lookup", why.clone()),
            Outcome::NoRule => ("no rule", format!("add import_id or match_on for {} to import-config.yaml", r.tf_type)),
            Outcome::Unresolvable(why) => ("unresolvable", why.clone()),
            Outcome::Failed(e) => ("FAILED", e.clone()),
            Outcome::Skipped => continue,
        };
        s.push_str(&format!("  {:w$}  {:30}  {}\n", r.address, verdict, detail, w = w));
        if !r.natural_key.is_empty() && !matches!(r.outcome, Outcome::Resolved { verified: false, .. } | Outcome::AlreadyAdopted(_)) {
            s.push_str(&format!("  {:w$}  {:30}  matched on: {}\n", "", "", r.natural_key, w = w));
        }
    }
    s
}

pub(crate) fn summary(resolutions: &[Resolution]) -> String {
    let count = |f: &dyn Fn(&Outcome) -> bool| resolutions.iter().filter(|r| f(&r.outcome)).count();
    format!(
        "adopt: {} to import ({} verified live, {} derived), {} need activation, {} already adopted, {} on apply, {} on apply with their project, {} ambiguous, {} without a rule, {} unresolvable, {} failed",
        count(&|o| matches!(o, Outcome::Resolved { .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: true, .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: false, .. })),
        count(&|o| matches!(o, Outcome::NeedsActivation { .. })),
        count(&|o| matches!(o, Outcome::AlreadyAdopted(_))),
        count(&|o| matches!(o, Outcome::OnApply)),
        count(&|o| matches!(o, Outcome::ParentOnApply(_))),
        count(&|o| matches!(o, Outcome::Ambiguous(_))),
        count(&|o| matches!(o, Outcome::NoRule)),
        count(&|o| matches!(o, Outcome::Unresolvable(_))),
        count(&|o| matches!(o, Outcome::Failed(_))),
    )
}

/// The resolutions that mean the run did not answer its question: a failed
/// lookup, an unresolvable or ambiguous resource, a type without a rule.
/// Zero means the table is complete; anything else is a non-zero exit.
pub(crate) fn unanswered(resolutions: &[Resolution]) -> usize {
    resolutions
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Failed(_) | Outcome::Unresolvable(_) | Outcome::Ambiguous(_) | Outcome::NoRule))
        .count()
}

/// Insert `"import-id" = "<id>"` into the source file right after the
/// declaring `label {` line, for every resolution that has an origin —
/// derived (unverified) ids included: `tofu plan` on the import block is the
/// validator of their existence. Returns (written, not written) — the latter
/// with the exact snippet to add by hand.
/// One id to write: (line, address, id, (tf_type, natural_key)).
type Edit = (u32, String, String, (String, String));

/// `presets_dir`: a pristine pack there (no `.local.` in its name) is
/// upstream-owned and never edited — its resources are reported with the
/// remedy (`--execute --import`, or fork the pack) instead.
pub(crate) fn write_import_ids(resolutions: &[Resolution], presets_dir: Option<&std::path::Path>) -> Result<(Vec<String>, Vec<String>), String> {
    let mut by_file: BTreeMap<String, Vec<Edit>> = BTreeMap::new();
    let mut hints = Vec::new();
    for r in resolutions {
        // Derived ids (`verified: false`) are written too: the id's existence
        // is checked by `tofu plan` on the import block — the validator.
        let id = match &r.outcome {
            Outcome::Resolved { id, .. } => id,
            _ => continue,
        };
        match &r.origin {
            Some((file, _)) if is_pristine_pack(file, presets_dir) => hints.push(format!(
                "{}: declared in the pristine pack {} — packs are upstream-owned; import it with `--execute --import`, or fork the pack (`merge-presets`) and re-run",
                r.address, file
            )),
            Some((file, line)) => by_file.entry(file.clone()).or_default().push((*line, r.address.clone(), id.clone(), (r.tf_type.clone(), r.natural_key.clone()))),
            None => hints.push(format!("{}: add \"import-id\" = \"{}\" to its entry by hand (no declaring line)", r.address, id)),
        }
    }
    let mut written = Vec::new();
    for (file, mut edits) in by_file {
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {}", file, e))?;
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        // bottom-up so earlier line numbers stay valid
        edits.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (line, address, id, (tf_type, natural_key)) in edits {
            let idx = line as usize - 1;
            let Some(decl) = lines.get(idx) else {
                hints.push(format!("{}: {}:{} is past the end of the file", address, file, line));
                continue;
            };
            match derived_entry(&tf_type, &id, &natural_key) {
                // a derived resource: rewrite the list entry inside the block
                // that starts at `line` into `{ <key> = "<value>" "import-id" = "<id>" }`
                Some((key, needles)) => match rewrite_list_entry(&mut lines, idx, &needles, key, &id) {
                    Some(at) => written.push(format!("{} → {}:{}", address, file, at + 1)),
                    None => hints.push(format!(
                        "{}: no entry {} found under {}:{} (interpolated, or already an object) — add \"import-id\" = \"{}\" to it by hand",
                        address,
                        needles.iter().map(|n| format!("\"{}\"", n)).collect::<Vec<_>>().join(" / "),
                        file,
                        line,
                        id
                    )),
                },
                None => {
                    if !decl.trim_end().ends_with('{') {
                        hints.push(format!(
                            "{}: {}:{} does not open a block on its own line — add \"import-id\" = \"{}\" by hand",
                            address, file, line, id
                        ));
                        continue;
                    }
                    let indent: String = decl.chars().take_while(|c| c.is_whitespace()).collect();
                    lines.insert(idx + 1, format!("{}  \"import-id\" = \"{}\"", indent, id));
                    written.push(format!("{} → {}:{}", address, file, line + 1));
                }
            }
        }
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(&file, out).map_err(|e| format!("{}: {}", file, e))?;
    }
    Ok((written, hints))
}

fn is_pristine_pack(file: &str, presets_dir: Option<&std::path::Path>) -> bool {
    let Some(dir) = presets_dir else { return false };
    let f = std::path::Path::new(file);
    let under = match (std::fs::canonicalize(f), std::fs::canonicalize(dir)) {
        (Ok(a), Ok(b)) => a.starts_with(&b),
        _ => f.starts_with(dir),
    };
    under && !file.contains(".local.")
}

/// For a derived resource: the key of the object form and the source strings
/// its list entry may be written as. `None` for a resource with a block of
/// its own.
fn derived_entry(tf_type: &str, id: &str, natural_key: &str) -> Option<(&'static str, Vec<String>)> {
    if tf_type.ends_with("_iam_member") {
        // `<parent> <role> <member>` — the entry is the role
        let role = id.split_whitespace().nth(1)?;
        return Some(("role", vec![role.to_string()]));
    }
    match tf_type {
        "google_project_service" => {
            let svc = id.rsplit('/').next()?;
            Some(("service", vec![svc.to_string()]))
        }
        "google_cloud_identity_group_membership" => {
            // natural key `<email> in groups/<n>`; the entry is the member as
            // written — bare, or with its principal prefix
            let email = natural_key.split(" in ").next()?.trim();
            if email.is_empty() {
                return None;
            }
            Some(("id", vec![
                email.to_string(),
                format!("user:{}", email),
                format!("serviceAccount:{}", email),
                format!("group:{}", email),
            ]))
        }
        _ => None,
    }
}

/// Inside the block/list opened at `start`, replace the first line whose entry
/// is one of `needles` (a quoted string item) with the object form. Returns
/// the line index rewritten.
fn rewrite_list_entry(lines: &mut [String], start: usize, needles: &[String], key: &str, id: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, line) in lines.iter_mut().enumerate().skip(start) {
        let raw = line.clone();
        let t = raw.trim();
        if i > start {
            let item = t.trim_end_matches(',').trim();
            if let Some(inner) = item.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                if needles.iter().any(|n| n == inner) {
                    let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();
                    let comma = if t.ends_with(',') { "," } else { "" };
                    *line = format!("{}{{ {} = \"{}\" \"import-id\" = \"{}\" }}{}", indent, key, inner, id, comma);
                    return Some(i);
                }
            }
        }
        for c in t.chars() {
            match c {
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
        }
        if i > start && depth <= 0 {
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The real thing
// ---------------------------------------------------------------------------

pub(crate) struct RealLive {
    http: reqwest::Client,
    token: String,
    customer_id: String,
    groups: Option<crate::cloud_identity::GroupResolver>,
    org_policy: Option<crate::org_policy::OrgPolicyClient>,
    assets: Option<google_cloud_asset_v1::client::AssetService>,
    policies: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl RealLive {
    pub(crate) async fn new(customer_id: &str) -> Result<Self, String> {
        let token = crate::gcp::access_token().await?;
        Ok(Self {
            http: reqwest::Client::new(),
            token,
            customer_id: customer_id.to_string(),
            groups: None,
            org_policy: None,
            assets: None,
            policies: BTreeMap::new(),
        })
    }

    pub(crate) async fn org_policy_client(&mut self) -> Result<&crate::org_policy::OrgPolicyClient, String> {
        if self.org_policy.is_none() {
            self.org_policy = Some(crate::org_policy::OrgPolicyClient::new().await.map_err(|e| e.to_string())?);
        }
        Ok(self.org_policy.as_ref().unwrap())
    }
}

impl Live for RealLive {
    async fn project(&mut self, project_id: &str) -> Result<Option<String>, String> {
        crate::gcp::resourcemanager::get_project_number(&self.http, &self.token, project_id)
            .await
            .map_err(|e| format!("project {}: {}", project_id, e))
    }

    async fn folder(&mut self, parent: &str, display_name: &str) -> Result<Lookup, String> {
        let folders = crate::gcp::resourcemanager::list_folders(&self.http, &self.token, parent).await?;
        let matches: Vec<String> = folders
            .iter()
            .filter(|f| f.get("displayName").and_then(|v| v.as_str()) == Some(display_name))
            .filter_map(|f| f.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();
        Ok(match matches.len() {
            0 => Lookup::Absent,
            1 => Lookup::One(matches.into_iter().next().unwrap()),
            _ => Lookup::Many(matches),
        })
    }

    async fn group(&mut self, email: &str) -> Result<Option<String>, String> {
        if self.groups.is_none() {
            self.groups = Some(crate::cloud_identity::GroupResolver::new(&self.customer_id).await.map_err(|e| e.to_string())?);
        }
        self.groups.as_mut().unwrap().group(email).await
    }

    async fn membership(&mut self, group_name: &str, email: &str) -> Result<Option<String>, String> {
        if self.groups.is_none() {
            self.groups = Some(crate::cloud_identity::GroupResolver::new(&self.customer_id).await.map_err(|e| e.to_string())?);
        }
        self.groups.as_mut().unwrap().membership(group_name, email).await
    }

    async fn search(&mut self, scope: &str, asset_type: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
        use google_cloud_asset_v1::model::ContentType;
        use google_cloud_gax::paginator::ItemPaginator as _;
        if self.assets.is_none() {
            self.assets = Some(crate::gcp::asset_service().await?);
        }
        let client = self.assets.as_ref().unwrap();
        let mut stream = client
            .list_assets()
            .set_parent(scope.to_string())
            .set_asset_types(vec![asset_type.to_string()])
            .set_content_type(ContentType::Resource)
            .set_page_size(1000)
            .by_item();
        let mut out = Vec::new();
        while let Some(asset) = stream.next().await {
            let asset: google_cloud_asset_v1::model::Asset = asset.map_err(|e| e.to_string())?;
            let data = match asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
                Some(d) => serde_json::to_value(d).map_err(|e| format!("{}: asset data is not JSON: {}", asset.name, e))?,
                None => serde_json::Value::Null,
            };
            let path = asset
                .name
                .strip_prefix("//")
                .and_then(|r| r.split_once('/'))
                .map(|(_, p)| p.to_string())
                .unwrap_or_else(|| asset.name.clone());
            out.push((path, data));
        }
        Ok(out)
    }

    async fn org_policy_exists(&mut self, parent: &str, constraint: &str) -> Result<bool, String> {
        if !self.policies.contains_key(parent) {
            let client = self.org_policy_client().await?;
            let current = crate::org_policy::fetch_current(client, parent).await.map_err(|e| e.to_string())?;
            self.policies.insert(parent.to_string(), current);
        }
        Ok(self.policies[parent].contains_key(constraint))
    }

    async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String> {
        crate::gcp::billing::list_budgets(&self.http, &self.token, billing_account).await.map_err(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ImportResourceConfig;

    struct Fake {
        /// project id → projects/<number>; a missing key is "does not exist"
        projects: BTreeMap<String, String>,
        /// project ids whose lookup FAILS (denied), for the error paths
        project_errors: BTreeSet<String>,
        folders: BTreeMap<(String, String), Lookup>,
        groups: BTreeMap<String, String>,
        memberships: BTreeMap<(String, String), String>,
        policies: BTreeSet<(String, String)>,
        searches: BTreeMap<(String, String), Vec<(String, serde_json::Value)>>,
        budgets: BTreeMap<String, Vec<(String, String)>>,
        calls: Vec<String>,
    }

    impl Live for Fake {
        async fn project(&mut self, project_id: &str) -> Result<Option<String>, String> {
            self.calls.push(format!("project {}", project_id));
            if self.project_errors.contains(project_id) {
                return Err(format!("403 Forbidden: no access to {}", project_id));
            }
            Ok(self.projects.get(project_id).cloned())
        }

        async fn folder(&mut self, parent: &str, display_name: &str) -> Result<Lookup, String> {
            self.calls.push(format!("folder {} {}", parent, display_name));
            Ok(self.folders.get(&(parent.to_string(), display_name.to_string())).cloned().unwrap_or(Lookup::Absent))
        }
        async fn group(&mut self, email: &str) -> Result<Option<String>, String> {
            self.calls.push(format!("group {}", email));
            Ok(self.groups.get(email).cloned())
        }
        async fn membership(&mut self, group_name: &str, email: &str) -> Result<Option<String>, String> {
            self.calls.push(format!("membership {} {}", group_name, email));
            Ok(self.memberships.get(&(group_name.to_string(), email.to_string())).cloned())
        }
        async fn org_policy_exists(&mut self, parent: &str, constraint: &str) -> Result<bool, String> {
            self.calls.push(format!("policy {} {}", parent, constraint));
            Ok(self.policies.contains(&(parent.to_string(), constraint.to_string())))
        }
        async fn search(&mut self, scope: &str, asset_type: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
            self.calls.push(format!("search {} {}", scope, asset_type));
            Ok(self.searches.get(&(scope.to_string(), asset_type.to_string())).cloned().unwrap_or_default())
        }
        async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String> {
            self.calls.push(format!("budgets {}", billing_account));
            Ok(self.budgets.get(billing_account).cloned().unwrap_or_default())
        }
    }

    /// (type, import_id template, match_on attrs)
    type RuleRow<'a> = (&'a str, Option<&'a str>, Option<&'a [&'a str]>);

    fn rules(rows: &[RuleRow]) -> ImportConfig {
        let mut resource_types = std::collections::HashMap::new();
        for (t, template, on) in rows {
            resource_types.insert(
                t.to_string(),
                ImportResourceConfig {
                    description: String::new(),
                    import: false,
                    asset_type: on.map(|_| format!("test.googleapis.com/{}", t)),
                    content_type: None,
                    exclude: None,
                    include: None,
                    derive_yaml_key_from: None,
                    deprecated: None,
                    import_id: template.map(|s| s.to_string()),
                    match_on: on.map(|v| v.iter().map(|s| s.to_string()).collect()),
                    activate: None,
                    map: None,
                    api_schema: None,
                },
            );
        }
        ImportConfig { root: None, only: None, resource_types }
    }

    const MAIN_TF: &str = r#"
resource "google_folder" "workloads" {
  display_name = "Workloads"
  parent = "organizations/123456789012"
}
resource "google_folder" "team" {
  display_name = "Team"
  parent = google_folder.workloads.name
}
resource "google_folder" "twin" {
  display_name = "Twin"
  parent = "organizations/123456789012"
}
resource "google_project" "infra" {
  project_id = "acme-infra-001"
  folder_id = google_folder.team.name
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
  project = google_project.infra.project_id
}
resource "google_storage_bucket" "b" {
  name = "acme-state"
}
resource "google_folder_iam_member" "grant" {
  role = "roles/viewer"
  member = "group:x@example.com"
  folder = google_folder.team.name
}
resource "google_cloud_identity_group" "auditors" {
  group_key {
    id = "gcp-auditors@example.com"
  }
}
resource "google_cloud_identity_group_membership" "m1" {
  group = google_cloud_identity_group.auditors.id
  preferred_member_key {
    id = "a@example.com"
  }
}
resource "google_org_policy_policy" "managed" {
  name = "organizations/123456789012/policies/compute.managed.requireOsLogin"
  parent = "organizations/123456789012"
  spec {
    rules {
      enforce = "TRUE"
    }
  }
}
resource "google_org_policy_policy" "legacy" {
  name = "organizations/123456789012/policies/iam.allowedPolicyMemberDomains"
  parent = "organizations/123456789012"
}
resource "google_monitoring_alert_policy" "alert" {
  display_name = "CIS 2.5"
  project = "acme-infra-001"
}
resource "google_widget" "w" {
  name = "w"
}
import {
  to = google_storage_bucket.b
  id = "acme-state"
}
"#;

    fn manifest() -> Manifest {
        let body = hcl::parse(MAIN_TF).unwrap();
        let mut m = Manifest::from_blocks(body.blocks());
        m.attach_imports(body.blocks());
        m.set_origin("google_folder.workloads", "yaml/x.satz", 10);
        m
    }

    fn fake() -> Fake {
        let mut f = Fake {
            projects: BTreeMap::new(),
            project_errors: BTreeSet::new(),
            folders: BTreeMap::new(),
            groups: BTreeMap::new(),
            memberships: BTreeMap::new(),
            policies: BTreeSet::new(),
            searches: BTreeMap::new(),
            budgets: BTreeMap::new(),
            calls: vec![],
        };
        f.projects.insert("acme-infra-001".into(), "projects/100000000001".into());
        f.searches.insert(
            ("projects/acme-infra-001".into(), "test.googleapis.com/google_monitoring_alert_policy".into()),
            vec![
                ("projects/acme-infra-001/alertPolicies/42".into(), serde_json::json!({"displayName": "CIS 2.5"})),
                ("projects/acme-infra-001/alertPolicies/43".into(), serde_json::json!({"displayName": "CIS 2.6"})),
            ],
        );
        f.folders.insert(("organizations/123456789012".into(), "Workloads".into()), Lookup::One("folders/111".into()));
        f.folders.insert(("folders/111".into(), "Team".into()), Lookup::One("folders/222".into()));
        f.folders.insert(("organizations/123456789012".into(), "Twin".into()), Lookup::Many(vec!["folders/8".into(), "folders/9".into()]));
        f.groups.insert("gcp-auditors@example.com".into(), "groups/00g".into());
        f.memberships.insert(("groups/00g".into(), "a@example.com".into()), "groups/00g/memberships/1".into());
        f.policies.insert(("organizations/123456789012".into(), "iam.allowedPolicyMemberDomains".into()));
        f
    }

    fn outcome<'a>(rs: &'a [Resolution], addr: &str) -> &'a Outcome {
        &rs.iter().find(|r| r.address == addr).unwrap_or_else(|| panic!("no resolution for {}", addr)).outcome
    }

    #[tokio::test]
    async fn folders_resolve_top_down_and_children_use_the_resolved_parent() {
        let rules = rules(&[
            ("google_service_account", Some("projects/{project}/serviceAccounts/{account_id}@{project}.iam.gserviceaccount.com"), None),
            ("google_folder_iam_member", Some("{folder} {role} {member}"), None),
            ("google_project", Some("{project_id}"), None),
            ("google_monitoring_alert_policy", None, Some(&["display_name"])),
        ]);
        let mut live = fake();
        let rs = resolve(&manifest(), &rules, &Options { only: BTreeSet::new(), activate: false }, &mut live).await;

        assert_eq!(outcome(&rs, "google_folder.workloads"), &Outcome::Resolved { id: "folders/111".into(), verified: true });
        // the child folder was looked up under the RESOLVED parent, not the traversal text
        assert!(live.calls.contains(&"folder folders/111 Team".to_string()), "{:?}", live.calls);
        assert_eq!(outcome(&rs, "google_folder.team"), &Outcome::Resolved { id: "folders/222".into(), verified: true });
        assert_eq!(outcome(&rs, "google_folder.twin"), &Outcome::Ambiguous(vec!["folders/8".into(), "folders/9".into()]));
        // derived ids render through references: project_id is a literal on the project
        assert_eq!(
            outcome(&rs, "google_service_account.sa"),
            &Outcome::Resolved { id: "projects/acme-infra-001/serviceAccounts/svc-iac@acme-infra-001.iam.gserviceaccount.com".into(), verified: false }
        );
        // the folder grant needs the folder NUMBER, which the lookup supplied
        assert_eq!(
            outcome(&rs, "google_folder_iam_member.grant"),
            &Outcome::Resolved { id: "folders/222 roles/viewer group:x@example.com".into(), verified: false }
        );
        // the project is an existence check, not a template: it exists → verified
        assert_eq!(outcome(&rs, "google_project.infra"), &Outcome::Resolved { id: "acme-infra-001".into(), verified: true });
        assert!(live.calls.contains(&"project acme-infra-001".to_string()), "{:?}", live.calls);
        assert_eq!(outcome(&rs, "google_storage_bucket.b"), &Outcome::AlreadyAdopted("acme-state".into()));
        // a GCP-assigned id: listed under the resource's own scope through CAI,
        // matched on display_name — one hit, verified
        assert_eq!(
            outcome(&rs, "google_monitoring_alert_policy.alert"),
            &Outcome::Resolved { id: "projects/acme-infra-001/alertPolicies/42".into(), verified: true }
        );
        assert!(live.calls.contains(&"search projects/acme-infra-001 test.googleapis.com/google_monitoring_alert_policy".to_string()), "{:?}", live.calls);
        assert_eq!(outcome(&rs, "google_widget.w"), &Outcome::NoRule);
    }

    #[tokio::test]
    async fn a_missing_project_is_on_apply_and_takes_its_children_with_it() {
        // The reported bug: a misspelled / not-yet-existing project produced a
        // confident derived import-id for itself AND every project-scoped
        // child, which --execute then wrote into the estate.
        let rules = rules(&[
            ("google_service_account", Some("projects/{project}/serviceAccounts/{account_id}@{project}.iam.gserviceaccount.com"), None),
            ("google_project", Some("{project_id}"), None),
            ("google_monitoring_alert_policy", None, Some(&["display_name"])),
        ]);
        let mut live = fake();
        live.projects.clear(); // the project does not exist live
        let rs = resolve(&manifest(), &rules, &Options { only: BTreeSet::new(), activate: false }, &mut live).await;

        assert_eq!(outcome(&rs, "google_project.infra"), &Outcome::OnApply);
        let sa = outcome(&rs, "google_service_account.sa");
        assert!(matches!(sa, Outcome::ParentOnApply(why) if why.contains("google_project.infra")), "{:?}", sa);
        let alert = outcome(&rs, "google_monitoring_alert_policy.alert");
        assert!(matches!(alert, Outcome::ParentOnApply(_)), "{:?}", alert);
        // no lookup was attempted under the non-existent project, and nothing
        // is written for it or its children
        assert!(!live.calls.iter().any(|c| c.starts_with("search projects/acme-infra-001")), "{:?}", live.calls);
        assert!(!rs.iter().any(|r| matches!(r.outcome, Outcome::Resolved { verified: false, .. })), "no derived ids may survive a missing parent");
        assert_eq!(unanswered(&rs), rs.iter().filter(|r| matches!(r.outcome, Outcome::Ambiguous(_) | Outcome::NoRule)).count(), "a missing project is a finding, not a failure");
        assert!(summary(&rs).contains("2 on apply with their project"), "{}", summary(&rs));
    }

    #[tokio::test]
    async fn a_project_lookup_that_fails_fails_its_children_with_the_same_cause() {
        let rules = rules(&[
            ("google_service_account", Some("projects/{project}/serviceAccounts/{account_id}@{project}.iam.gserviceaccount.com"), None),
            ("google_project", Some("{project_id}"), None),
        ]);
        let mut live = fake();
        live.project_errors.insert("acme-infra-001".into());
        let rs = resolve(&manifest(), &rules, &Options { only: BTreeSet::new(), activate: false }, &mut live).await;

        let p = outcome(&rs, "google_project.infra");
        assert!(matches!(p, Outcome::Failed(e) if e.contains("403")), "{:?}", p);
        let sa = outcome(&rs, "google_service_account.sa");
        assert!(matches!(sa, Outcome::Failed(e) if e.contains("google_project.infra") && e.contains("403")), "{:?}", sa);
        // a failed run is unanswered → the command exits non-zero
        assert!(unanswered(&rs) >= 2, "{}", summary(&rs));
    }

    #[tokio::test]
    async fn groups_memberships_and_org_policies_use_their_native_lookups() {
        let rules = rules(&[]);
        let mut live = fake();
        let rs = resolve(&manifest(), &rules, &Options { only: BTreeSet::new(), activate: false }, &mut live).await;
        assert_eq!(outcome(&rs, "google_cloud_identity_group.auditors"), &Outcome::Resolved { id: "groups/00g".into(), verified: true });
        assert_eq!(outcome(&rs, "google_cloud_identity_group_membership.m1"), &Outcome::Resolved { id: "groups/00g/memberships/1".into(), verified: true });
        assert_eq!(
            outcome(&rs, "google_org_policy_policy.legacy"),
            &Outcome::Resolved { id: "organizations/123456789012/policies/iam.allowedPolicyMemberDomains".into(), verified: true }
        );
        // managed + not live: activation is opt-in
        assert!(matches!(outcome(&rs, "google_org_policy_policy.managed"), Outcome::NeedsLookup(_)));
        let mut live = fake();
        let rs = resolve(&manifest(), &rules, &Options { only: BTreeSet::new(), activate: true }, &mut live).await;
        assert_eq!(
            outcome(&rs, "google_org_policy_policy.managed"),
            &Outcome::NeedsActivation { id: "organizations/123456789012/policies/compute.managed.requireOsLogin".into(), enforce: Some(true) }
        );
    }

    #[tokio::test]
    async fn only_filters_and_absent_group_makes_memberships_on_apply() {
        let rules = rules(&[]);
        let mut live = fake();
        live.groups.clear();
        let only: BTreeSet<String> = ["google_cloud_identity_group", "google_cloud_identity_group_membership"].iter().map(|s| s.to_string()).collect();
        let rs = resolve(&manifest(), &rules, &Options { only, activate: false }, &mut live).await;
        assert_eq!(outcome(&rs, "google_cloud_identity_group.auditors"), &Outcome::OnApply);
        assert_eq!(outcome(&rs, "google_cloud_identity_group_membership.m1"), &Outcome::OnApply);
        assert_eq!(outcome(&rs, "google_folder.workloads"), &Outcome::Skipped);
        assert!(!live.calls.iter().any(|c| c.starts_with("folder")), "{:?}", live.calls);
    }

    /// The shipped rules are data the engine depends on: they must parse, and
    /// the types every fleet estate declares must have one.
    /// F16: derived resources have no block of their own; their id goes into
    /// the list entry they derive from, as the object form.
    #[test]
    fn derived_ids_rewrite_the_list_entry_in_place() {
        let src = "google_organization_iam_member {\n  \"group:a@example.com\" = [\n    \"roles/viewer\",\n    \"roles/browser\",\n  ]\n}\n\ngoogle_cloud_identity_group {\n  auditors {\n    display_name = \"A\"\n    member = [\n      \"user:b@example.com\",\n    ]\n  }\n}\n";
        let mut lines: Vec<String> = src.lines().map(String::from).collect();
        let (key, needles) = derived_entry("google_organization_iam_member", "1 roles/browser group:a@example.com", "").unwrap();
        let at = rewrite_list_entry(&mut lines, 1, &needles, key, "1 roles/browser group:a@example.com").unwrap();
        assert_eq!(lines[at], "    { role = \"roles/browser\" \"import-id\" = \"1 roles/browser group:a@example.com\" },");
        assert_eq!(lines[2], "    \"roles/viewer\",", "the other entry is untouched");
        // a needle outside the block is not found
        assert!(rewrite_list_entry(&mut lines, 1, &["user:b@example.com".to_string()], "id", "x").is_none());
        let (key, needles) = derived_entry("google_cloud_identity_group_membership", "groups/g/memberships/m", "b@example.com in groups/g").unwrap();
        let at = rewrite_list_entry(&mut lines, 8, &needles, key, "groups/g/memberships/m").unwrap();
        assert_eq!(lines[at], "      { id = \"user:b@example.com\" \"import-id\" = \"groups/g/memberships/m\" },");
        let (key, needles) = derived_entry("google_project_service", "p/storage.googleapis.com", "").unwrap();
        assert_eq!((key, needles), ("service", vec!["storage.googleapis.com".to_string()]));
    }

    #[test]
    fn shipped_import_config_carries_rules_for_the_fleet_types() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/import-config.yaml");
        let cfg: ImportConfig = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).expect("import-config.yaml parses");
        for t in [
            "google_project",
            "google_project_service",
            "google_storage_bucket",
            "google_service_account",
            "google_organization_iam_member",
            "google_project_iam_member",
            "google_folder_iam_member",
            "google_billing_account_iam_member",
            "google_storage_bucket_iam_member",
            "google_logging_organization_sink",
            "google_logging_metric",
            "google_logging_project_bucket_config",
            "google_organization_iam_audit_config",
            "google_org_policy_policy",
        ] {
            assert!(matches!(rule_for(&cfg, t), Rule::Template(_)), "{} needs an import_id rule", t);
        }
        for t in ["google_folder", "google_monitoring_alert_policy", "google_monitoring_notification_channel", "google_billing_budget", "google_essential_contacts_contact"] {
            assert!(matches!(rule_for(&cfg, t), Rule::Match(..)), "{} needs a match_on rule", t);
        }
        assert_eq!(cfg.resource_types["google_org_policy_policy"].activate.as_deref(), Some("managed"));
    }

    #[test]
    fn write_inserts_after_the_declaring_line_and_refuses_pristine_packs() {
        let tmp = std::env::temp_dir().join("satz-adopt-write.satz");
        std::fs::write(&tmp, "google_folder {\n  workloads {\n    display_name = \"Workloads\"\n  }\n  one { display_name = \"x\" }\n}\n").unwrap();
        let file = tmp.to_string_lossy().to_string();
        let presets = std::env::temp_dir().join("satz-adopt-presets");
        std::fs::create_dir_all(&presets).unwrap();
        let pack = presets.join("cis.satz");
        std::fs::write(&pack, "pack cis version \"1.0\"\n\n\"x\" {\n  name = \"compute.x\"\n}\n").unwrap();
        let rs = vec![
            Resolution { address: "google_folder.workloads".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111".into(), verified: true }, origin: Some((file.clone(), 2)), org_policy: None },
            Resolution { address: "google_folder.one".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/1".into(), verified: true }, origin: Some((file.clone(), 5)), org_policy: None },
            Resolution { address: "google_folder_iam_member.g".into(), tf_type: "google_folder_iam_member".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111 r m".into(), verified: false }, origin: None, org_policy: None },
            Resolution { address: "google_org_policy_policy.x".into(), tf_type: "google_org_policy_policy".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "organizations/1/policies/compute.x".into(), verified: false }, origin: Some((pack.to_string_lossy().to_string(), 3)), org_policy: None },
        ];
        let (written, hints) = write_import_ids(&rs, Some(&presets)).unwrap();
        let text = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(text, "google_folder {\n  workloads {\n    \"import-id\" = \"folders/111\"\n    display_name = \"Workloads\"\n  }\n  one { display_name = \"x\" }\n}\n");
        assert_eq!(written.len(), 1);
        assert_eq!(hints.len(), 3, "{:?}", hints);
        assert!(hints.iter().any(|h| h.contains("google_folder.one") && h.contains("by hand")));
        assert!(hints.iter().any(|h| h.contains("google_folder_iam_member.g") && h.contains("no declaring line")));
        assert!(hints.iter().any(|h| h.contains("google_org_policy_policy.x") && h.contains("pristine pack")));
        assert!(!std::fs::read_to_string(&pack).unwrap().contains("import-id"), "a pristine pack is never edited");
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_dir_all(&presets);
    }

    #[tokio::test]
    async fn a_budget_resolves_by_display_name_under_its_billing_account() {
        let manifest = Manifest::parse(
            "resource \"google_billing_budget\" \"infra\" {\n  billing_account = \"012345-6789AB-CDEF01\"\n  display_name = \"Infra monthly\"\n}\n\
             resource \"google_billing_budget\" \"twice\" {\n  billing_account = \"012345-6789AB-CDEF01\"\n  display_name = \"Dup\"\n}\n",
        );
        let mut f = fake();
        f.budgets.insert(
            "012345-6789AB-CDEF01".into(),
            vec![
                ("billingAccounts/012345-6789AB-CDEF01/budgets/aaaa".into(), "Infra monthly".into()),
                ("billingAccounts/012345-6789AB-CDEF01/budgets/bbbb".into(), "Dup".into()),
                ("billingAccounts/012345-6789AB-CDEF01/budgets/cccc".into(), "Dup".into()),
            ],
        );
        let cfg: ImportConfig = serde_yaml::from_str("resource_types: {}").unwrap();
        let rs = resolve(&manifest, &cfg, &Options { only: Default::default(), activate: false }, &mut f).await;
        let by = |a: &str| rs.iter().find(|r| r.address == a).unwrap().outcome.clone();
        assert_eq!(by("google_billing_budget.infra"), Outcome::Resolved { id: "billingAccounts/012345-6789AB-CDEF01/budgets/aaaa".into(), verified: true });
        assert!(matches!(by("google_billing_budget.twice"), Outcome::Ambiguous(ref c) if c.len() == 2));
        assert!(f.calls.iter().any(|c| c == "budgets 012345-6789AB-CDEF01"), "{:?}", f.calls);
    }
}
