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

use rmcp::schemars;

use std::path::{Path, PathBuf};

use crate::config::ImportConfig;
use crate::gcp::iam_policy::PolicyApi;
use crate::manifest::{EmittedResource, Manifest};
use crate::{configure_estate_impersonation, estate_path, load_import_config, pipeline_b_generate, reject_yaml_dialect, PipelineBOut};
use crate::settings::{ToolConfig};

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
    /// Is a policy on `constraint` live under `parent`? `Some(holds_rules)` when it is,
    /// `None` when it is not.
    async fn org_policy(&mut self, parent: &str, constraint: &str) -> Result<Option<bool>, String>;
    /// Every live asset of `asset_type` under `scope` (`organizations/<n>`,
    /// `folders/<n>`, `projects/<id>`): its resource path (the CAI name without
    /// the `//<service>/` prefix — which is the Terraform import id for the
    /// types that carry one) and its resource data.
    async fn search(&mut self, scope: &str, asset_type: &str) -> Result<Vec<(String, serde_json::Value)>, String>;
    /// Every budget of a billing account: (resource name
    /// `billingAccounts/<id>/budgets/<uuid>`, display name). Budgets are not
    /// in Cloud Asset Inventory — the Billing Budgets API is the only lookup.
    async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String>;
    /// The IAM policy of the resource a grant is made on (`organizations/<n>`,
    /// `b/<bucket>`, `projects/<p>/serviceAccounts/<email>`, …), read through
    /// `api`. `Ok(None)` when that resource provably does not exist (404), `Err`
    /// when the policy could not be read — never an empty policy by error.
    async fn iam_policy(&mut self, api: PolicyApi, resource: &str) -> Result<Option<serde_json::Value>, String>;
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
    /// A second line the table prints under the verdict.
    pub note: Option<String>,
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

/// The types `resolve` looks up by their natural key in code rather than by a rule.
const NATIVE: &[&str] = &[
    "google_folder",
    "google_project",
    "google_cloud_identity_group",
    "google_cloud_identity_group_membership",
    "google_org_policy_policy",
    "google_billing_budget",
];

/// Whether `adopt` can resolve a live object of this type at all: a native lookup, an
/// `import_id` template or a `match_on` rule. A type with none has no recovery once
/// the object exists — a console click, a partial apply — except `tofu import` by hand.
pub(crate) fn adoptable(rules: &ImportConfig, tf_type: &str) -> bool {
    NATIVE.contains(&tf_type) || !matches!(rule_for(rules, tf_type), Rule::None)
}

/// The id an `import_id` rule renders for `r` offline, or why it cannot; `None` for a
/// type without a template rule. What the library gate holds each template to.
#[cfg(test)]
pub(crate) fn render_rule(rules: &ImportConfig, r: &EmittedResource, manifest: &Manifest) -> Option<Result<String, String>> {
    let Rule::Template(template) = rule_for(rules, &r.tf_type) else { return None };
    Some(match render_template(&template, r, manifest, &Known::default()) {
        (_, Outcome::Resolved { id, .. }) => Ok(id),
        (_, Outcome::Unresolvable(why)) => Err(why),
        (_, other) => Err(format!("{other:?}")),
    })
}

/// What the run has decided about the resources it has already reached: the live
/// id of each one that has one, and the verdict of every one — a later resource
/// that references an earlier one reads both here.
#[derive(Default)]
struct Known {
    ids: BTreeMap<String, String>,
    verdicts: BTreeMap<String, Outcome>,
}

impl Known {
    fn record(&mut self, address: &str, outcome: &Outcome) {
        if let Outcome::Resolved { id, .. } | Outcome::NeedsActivation { id, .. } | Outcome::AlreadyAdopted(id) = outcome {
            self.ids.insert(address.to_string(), id.clone());
        }
        self.verdicts.insert(address.to_string(), outcome.clone());
    }
}

/// Why a value the resolution needs could not be produced, and what that means
/// for the resource that needed it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Missing {
    /// The resource the value comes from is not live: `apply` creates it, and
    /// whatever needs its id is created with it.
    NotLive(String),
    /// The lookup behind the value failed — never read as absence.
    Failed(String),
    /// Nothing satz can follow: an expression, a resource the estate does not
    /// emit, an attribute that is neither a literal nor a live id.
    Unresolvable(String),
}

impl Missing {
    fn outcome(self) -> Outcome {
        match self {
            Missing::NotLive(why) => Outcome::ParentOnApply(why),
            Missing::Failed(why) => Outcome::Failed(why),
            Missing::Unresolvable(why) => Outcome::Unresolvable(why),
        }
    }
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Missing::NotLive(why) | Missing::Failed(why) | Missing::Unresolvable(why) => f.write_str(why),
        }
    }
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
    let mut known = Known::default();
    // Projects that are not live (absent, or unanswerable), by address and by
    // project id — a child names its project either by reference or by the
    // literal id, and both must read the same verdict.
    let mut not_live: BTreeMap<String, Outcome> = BTreeMap::new();
    // IAM policies by the resource they are set on, read once per run — the
    // answer, a missing resource or the failure alike
    let mut iam_policies: IamPolicies = BTreeMap::new();
    let mut out = Vec::new();
    for r in ordered(manifest) {
        let mut res = Resolution {
            address: r.address(),
            tf_type: r.tf_type.clone(),
            natural_key: String::new(),
            outcome: Outcome::NoRule,
            origin: r.origin.clone(),
            org_policy: None,
        note: None,
        };
        if let Some(id) = &r.import_id {
            res.outcome = Outcome::AlreadyAdopted(id.clone());
        } else if !opts.only.is_empty() && !opts.only.contains(&r.tf_type) {
            res.outcome = Outcome::Skipped;
        } else if let Some(parent_verdict) = parent_not_live(r, manifest, &not_live) {
            // A resource inside a project that is not live inherits that verdict
            // before any rule runs: nothing under it can be adopted yet.
            res.outcome = parent_verdict;
        } else {
            let outcome = match r.tf_type.as_str() {
                "google_folder" => resolve_folder(r, manifest, &known, live).await,
                "google_project" => resolve_project(r, live).await,
                "google_cloud_identity_group" => resolve_group(r, live).await,
                "google_cloud_identity_group_membership" => resolve_membership(r, &known, live).await,
                "google_org_policy_policy" => resolve_org_policy(r, manifest, &known, opts, live, &mut res).await,
                "google_billing_budget" => resolve_budget(r, live).await,
                _ => match rule_for(rules, &r.tf_type) {
                    Rule::Template(t) => match render_template(&t, r, manifest, &known) {
                        (_, Outcome::Resolved { id, verified: false }) if grant_parent(&r.tf_type, "").is_some() => {
                            resolve_grant(r, &id, manifest, &known, &mut iam_policies, live).await
                        }
                        other => other,
                    },
                    Rule::Match(on, asset_type) => resolve_match(r, &on, asset_type.as_deref(), manifest, &known, live).await,
                    Rule::None => (String::new(), Outcome::NoRule),
                },
            };
            res.natural_key = outcome.0;
            res.outcome = outcome.1;
        }
        known.record(&res.address, &res.outcome);
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

/// Folders by depth (parent chain length), then everything else by address, and
/// the IAM grants last: a grant is decided against the live IAM policy of the
/// resource it is made on, and when the estate declares that resource the grant
/// names it by reference — so its verdict and its live id must be in hand before
/// the grant is reached, whatever the two addresses sort like.
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
    let is_grant = |r: &EmittedResource| grant_parent(&r.tf_type, "").is_some();
    let others = manifest
        .resources
        .values()
        .filter(|r| r.tf_type != "google_folder" && r.tf_type != "google_project");
    let rest = others.clone().filter(|r| !is_grant(r));
    let grants = others.filter(|r| is_grant(r));
    folders.into_iter().chain(projects).chain(rest).chain(grants).collect()
}

/// `google_T.L.A` → (`google_T.L`, `A`).
fn ref_target(traversal: &str) -> Option<(String, String)> {
    let (addr, attr) = traversal.rsplit_once('.')?;
    if addr.matches('.').count() != 1 {
        return None;
    }
    Some((addr.to_string(), attr.to_string()))
}

/// The attributes whose value IS the referenced resource's live id — the id
/// adoption resolves for that resource, not an attribute the estate declares.
/// A reference to one of them is answered from what this run has resolved, so
/// the id comes from the lookup and is never derived a second time.
fn denotes_live_id(tf_type: &str, attr: &str) -> bool {
    matches!(
        (tf_type, attr),
        ("google_folder", "name" | "id")
            | ("google_cloud_identity_group", "name" | "id")
            // `projects/<project>/serviceAccounts/<email>` — what the account's
            // own import id is, and how a grant on it names its parent
            | ("google_service_account", "name" | "id")
    )
}

/// The value of attribute `key` on `r`: a literal, or a reference followed to
/// the resource it names — its resolved live id when that is what the
/// reference denotes (`google_folder.x.name`, `google_service_account.x.name`),
/// else that resource's own attribute (`google_project.x.project_id`).
fn value_of(r: &EmittedResource, key: &str, manifest: &Manifest, known: &Known) -> Result<String, Missing> {
    if let Some(v) = r.attrs.get(key) {
        // A value carrying an interpolation is not a literal: matching it
        // against live state would search for the text `${…}` and silently
        // find nothing. Say so instead (R9).
        if crate::manifest::has_interpolation(v) {
            return Err(Missing::Unresolvable(format!(
                "{}: `{}` = \"{}\" carries a reference that is only known after apply — it cannot be resolved before the resource exists",
                r.address(),
                key,
                v
            )));
        }
        return Ok(v.clone());
    }
    let Some(traversal) = r.refs.get(key) else {
        return Err(Missing::Unresolvable(format!("{} has no `{}`", r.address(), key)));
    };
    let Some((target, attr)) = ref_target(traversal) else {
        return Err(Missing::Unresolvable(format!("{}: `{}` = {} is not a resource reference", r.address(), key, traversal)));
    };
    let Some(t) = manifest.resources.get(&target) else {
        return Err(Missing::Unresolvable(format!("{}: `{}` references {}, which is not emitted", r.address(), key, target)));
    };
    if denotes_live_id(&t.tf_type, &attr) {
        if let Some(id) = known.ids.get(&target) {
            return Ok(id.clone());
        }
        // The reference is followable and the target was reached — its own
        // verdict is the answer, and the one thing it never becomes is a
        // guessed id.
        return Err(match known.verdicts.get(&target) {
            Some(Outcome::OnApply) | Some(Outcome::ParentOnApply(_)) => Missing::NotLive(format!("{} is not live — created with it", target)),
            Some(Outcome::Failed(e)) => Missing::Failed(format!("{}: {}", target, e)),
            Some(Outcome::Skipped) => {
                Missing::Unresolvable(format!("{} was left out by --only, so its live id is not resolved in this run", target))
            }
            _ => Missing::Unresolvable(format!("{} is not resolved yet ({} on it must be adopted or pinned first)", target, key)),
        });
    }
    match t.attrs.get(&attr) {
        Some(v) if crate::manifest::has_interpolation(v) => Err(Missing::Unresolvable(format!(
            "{}: `{}` references {}.{}, which is itself a reference known only after apply",
            r.address(),
            key,
            target,
            attr
        ))),
        Some(v) => Ok(v.clone()),
        None => Err(Missing::Unresolvable(format!(
            "{}: `{}` references {}.{}, which is not a literal and is not this run's id for it",
            r.address(),
            key,
            target,
            attr
        ))),
    }
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
    known: &Known,
    live: &mut L,
) -> (String, Outcome) {
    let key_text = on.join(", ");
    // `TODO/UNKNOWN` is the auto-generated placeholder of an unfilled row
    let asset_type = asset_type.filter(|a| !a.starts_with("TODO"));
    let Some(asset_type) = asset_type else {
        return (key_text, Outcome::Unresolvable(format!("{} has match_on but no asset_type in import-config.yaml", r.tf_type)));
    };
    let scope = match match_scope(r, manifest, known) {
        Ok(s) => s,
        Err(e) => return (key_text, e.outcome()),
    };
    let mut wanted: Vec<(String, String)> = Vec::new();
    for k in on {
        let v = r.attrs.get(k).or_else(|| r.nested.get(k)).cloned();
        let v = match v {
            Some(v) => v,
            None => match value_of(r, k, manifest, known) {
                Ok(v) => v,
                Err(e) => return (key_text, e.outcome()),
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
fn match_scope(r: &EmittedResource, manifest: &Manifest, known: &Known) -> Result<String, Missing> {
    // the first scope attribute the resource HAS decides; a present attribute
    // that cannot be resolved (its folder is ambiguous, say) is an error, not a
    // reason to try the next one and search the wrong scope
    let has = |k: &str| r.attrs.contains_key(k) || r.refs.contains_key(k);
    if has("parent") {
        return value_of(r, "parent", manifest, known);
    }
    if has("project") {
        return value_of(r, "project", manifest, known).map(|p| format!("projects/{}", p.trim_start_matches("projects/")));
    }
    if has("folder") {
        return value_of(r, "folder", manifest, known).map(|f| format!("folders/{}", f.trim_start_matches("folders/")));
    }
    if has("org_id") {
        return value_of(r, "org_id", manifest, known).map(|o| format!("organizations/{}", o.trim_start_matches("organizations/")));
    }
    Err(Missing::Unresolvable(format!("{} has no parent, project, folder or org_id to scope the lookup", r.address())))
}

/// `group_key.id` → data["groupKey"]["id"], as text.
fn data_at(data: &serde_json::Value, dotted: &str) -> Option<String> {
    let mut cur = data;
    for part in dotted.split('.') {
        let camel = crate::schema::snake_to_camel(part);
        cur = cur.get(&camel).or_else(|| cur.get(part))?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

async fn resolve_folder<L: Live>(
    r: &EmittedResource,
    manifest: &Manifest,
    known: &Known,
    live: &mut L,
) -> (String, Outcome) {
    let display_name = r.attrs.get("display_name").cloned().unwrap_or_default();
    let parent = match value_of(r, "parent", manifest, known) {
        Ok(p) => p,
        Err(e) => return (display_name, e.outcome()),
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
    known: &Known,
    live: &mut L,
) -> (String, Outcome) {
    let Some(email) = r.nested.get("preferred_member_key.id").cloned() else {
        return (String::new(), Outcome::Unresolvable(format!("{} emits no preferred_member_key.id", r.address())));
    };
    let Some((group_addr, _)) = r.refs.get("group").and_then(|g| ref_target(g)) else {
        return (email, Outcome::Unresolvable(format!("{} has no group reference", r.address())));
    };
    let Some(group_name) = known.ids.get(&group_addr) else {
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
    known: &Known,
    opts: &Options,
    live: &mut L,
    res: &mut Resolution,
) -> (String, Outcome) {
    let name = r.attrs.get("name").cloned().unwrap_or_default();
    let constraint = crate::org_policy::constraint_name(&name);
    // The compile guarantees `parent` is a literal or a reference; an
    // unresolvable one is reported as such — never scraped out of the policy
    // name, which is how a wrong parent once became a confident lookup.
    let parent = match value_of(r, "parent", manifest, known) {
        Ok(p) => match crate::org_policy::qualify_parent(&p) {
            Ok(q) => q,
            Err(e) => return (constraint, Outcome::Unresolvable(format!("{}: {}", r.address(), e))),
        },
        Err(e) => return (constraint, e.outcome()),
    };
    res.org_policy = Some((parent.clone(), constraint.clone()));
    let id = crate::org_policy::full_policy_name(&parent, &constraint);
    match live.org_policy(&parent, &constraint).await {
        Ok(Some(holds_rules)) => {
            // Imported, the state holds the live rules under an address that declares
            // reset — the one shape the API refuses to update. A new organisation has
            // such policies before anything was set by hand: Google enforces a
            // secure-by-default set on it.
            if holds_rules && r.reset {
                res.note = Some(format!(
                    "holds rules live and is declared reset — the next `satz plan` / `satz apply` replaces it; a bare `tofu apply` needs `-replace={}`",
                    r.address()
                ));
            }
            (constraint, Outcome::Resolved { id, verified: true })
        }
        Ok(None) if crate::org_policy::is_managed(&constraint) => {
            if opts.activate {
                (constraint, Outcome::NeedsActivation { id, enforce: r.enforce })
            } else {
                (constraint, Outcome::NeedsLookup("managed constraint is not live — activate it first (--activate)".into()))
            }
        }
        Ok(None) => (constraint, Outcome::OnApply),
        Err(e) => (constraint, Outcome::Failed(e)),
    }
}

/// Render `{placeholder}`s from the resource's attributes and resolved
/// references. `{parent}` follows the same rules as any other key.
fn render_template(
    template: &str,
    r: &EmittedResource,
    manifest: &Manifest,
    known: &Known,
) -> (String, Outcome) {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            return (template.to_string(), Outcome::Unresolvable(format!("unterminated placeholder in rule `{}`", template)));
        };
        let key = &rest[start + 1..start + end];
        match value_of(r, key, manifest, known) {
            Ok(v) => out.push_str(&v),
            Err(e) => return (template.to_string(), e.outcome()),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    (out.clone(), Outcome::Resolved { id: out, verified: false })
}

/// IAM policies read in one run, by the resource they are set on.
type IamPolicies = BTreeMap<String, Result<Option<serde_json::Value>, String>>;

/// The grant types whose import id is `<parent> <role> <member>`: the API that
/// holds the parent's IAM policy, and the parent as that API names it. `None`
/// for every other type.
fn grant_parent(tf_type: &str, parent: &str) -> Option<(PolicyApi, String)> {
    use PolicyApi::*;
    let under = |prefix: &str| format!("{}{}", prefix, parent.trim_start_matches(prefix));
    Some(match tf_type {
        "google_organization_iam_member" => (ResourceManager, under("organizations/")),
        "google_folder_iam_member" => (ResourceManager, under("folders/")),
        "google_project_iam_member" => (ResourceManager, under("projects/")),
        "google_billing_account_iam_member" => (Billing, under("billingAccounts/")),
        "google_storage_bucket_iam_member" => (Storage, under("b/")),
        // the provider accepts a bare e-mail; the IAM API wants the full name
        "google_service_account_iam_member" if !parent.contains('/') => {
            (ServiceAccount, format!("projects/-/serviceAccounts/{}", parent))
        }
        "google_service_account_iam_member" => (ServiceAccount, parent.to_string()),
        "google_pubsub_topic_iam_member" | "google_pubsub_subscription_iam_member" => (PubSub, parent.to_string()),
        "google_bigquery_dataset_iam_member" => (BigQueryDataset, parent.to_string()),
        _ => return None,
    })
}

/// The condition a grant declares, as far as it can be compared with a live one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclaredCondition {
    None,
    Literal { title: String, expression: String },
    /// declared, but with a title or expression that is not a literal
    Undecidable,
}

fn declared_condition(r: &EmittedResource) -> DeclaredCondition {
    if !r.nested.keys().any(|k| k.starts_with("condition.")) {
        return DeclaredCondition::None;
    }
    match (r.nested.get("condition.title"), r.nested.get("condition.expression")) {
        (Some(t), Some(e)) if !crate::manifest::has_interpolation(t) && !crate::manifest::has_interpolation(e) => {
            DeclaredCondition::Literal { title: t.clone(), expression: e.clone() }
        }
        _ => DeclaredCondition::Undecidable,
    }
}

/// What a live IAM policy says about one declared grant.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GrantLive {
    /// A binding of the role, under the declared condition (its title when there
    /// is one), holds the member.
    Held(Option<String>),
    /// No such binding holds the member.
    Absent,
    /// The member holds the role live only under conditions that cannot be told
    /// apart from the declared one — each as `title: expression`.
    Unclear(Vec<String>),
}

/// Members compare as IAM compares them: the address of a user, group, service
/// account or domain without regard to case, every other principal exactly.
fn same_member(a: &str, b: &str) -> bool {
    let folds = |m: &str| ["user:", "group:", "serviceAccount:", "domain:"].iter().any(|p| m.starts_with(p));
    if folds(a) && folds(b) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Whether `policy` (an IAM policy, version 3) holds the declared grant.
fn grant_in_policy(policy: &serde_json::Value, role: &str, member: &str, condition: &DeclaredCondition) -> GrantLive {
    let text = |c: &serde_json::Value, k: &str| c.get(k).and_then(|s| s.as_str()).unwrap_or("").to_string();
    // the conditions under which `member` holds `role`, `None` for the unconditional binding
    let held: Vec<Option<(String, String)>> = policy
        .get("bindings")
        .and_then(|b| b.as_array())
        .into_iter()
        .flatten()
        .filter(|b| b.get("role").and_then(|r| r.as_str()) == Some(role))
        .filter(|b| {
            b.get("members").and_then(|m| m.as_array()).into_iter().flatten().any(|m| m.as_str().is_some_and(|m| same_member(m, member)))
        })
        .map(|b| b.get("condition").map(|c| (text(c, "title"), text(c, "expression"))))
        .collect();
    let conditional: Vec<String> = held.iter().flatten().map(|(t, e)| format!("{}: {}", t, e)).collect();
    match condition {
        DeclaredCondition::None if held.iter().any(Option::is_none) => GrantLive::Held(None),
        DeclaredCondition::None => GrantLive::Absent,
        DeclaredCondition::Literal { title, expression } => {
            if held.iter().flatten().any(|(t, e)| t == title && e.trim() == expression.trim()) {
                GrantLive::Held(Some(title.clone()))
            } else if conditional.is_empty() {
                GrantLive::Absent
            } else {
                GrantLive::Unclear(conditional)
            }
        }
        DeclaredCondition::Undecidable if conditional.is_empty() => GrantLive::Absent,
        DeclaredCondition::Undecidable => GrantLive::Unclear(conditional),
    }
}

/// An IAM grant whose import id rendered offline: imported only when the live
/// policy of its parent holds it. A grant the policy does not hold is created by
/// `apply`; a parent that does not exist takes its grants with it; a policy that
/// cannot be read fails the grant — an unreadable policy is never an empty one.
async fn resolve_grant<L: Live>(
    r: &EmittedResource,
    id: &str,
    manifest: &Manifest,
    known: &Known,
    policies: &mut IamPolicies,
    live: &mut L,
) -> (String, Outcome) {
    let (role, member) = match (value_of(r, "role", manifest, known), value_of(r, "member", manifest, known)) {
        (Ok(role), Ok(member)) => (role, member),
        (Err(e), _) | (_, Err(e)) => return (id.to_string(), e.outcome()),
    };
    let Some(parent) = id.strip_suffix(&format!(" {} {}", role, member)) else {
        return (
            id.to_string(),
            Outcome::Unresolvable(format!("{}: the import id `{}` does not end in its role and member", r.address(), id)),
        );
    };
    let Some((api, resource)) = grant_parent(&r.tf_type, parent) else {
        return (id.to_string(), Outcome::NoRule);
    };
    let key = format!("{} {} in the IAM policy of {}", role, member, resource);
    if !policies.contains_key(&resource) {
        let read = live.iam_policy(api, &resource).await;
        policies.insert(resource.clone(), read);
    }
    let policy = match &policies[&resource] {
        Ok(Some(p)) => p,
        Ok(None) => return (key, Outcome::ParentOnApply(format!("{} is not live — created with it", resource))),
        Err(e) => return (key, Outcome::Failed(format!("reading the IAM policy of {}: {}", resource, e))),
    };
    let outcome = match grant_in_policy(policy, &role, &member, &declared_condition(r)) {
        GrantLive::Held(None) => Outcome::Resolved { id: id.to_string(), verified: true },
        // the provider's import id of a conditional grant ends in the condition title
        GrantLive::Held(Some(title)) => Outcome::Resolved { id: format!("{} {}", id, title), verified: true },
        GrantLive::Absent => Outcome::OnApply,
        GrantLive::Unclear(c) => Outcome::Ambiguous(c),
    };
    (key, outcome)
}

// ---------------------------------------------------------------------------
// Report and sinks
// ---------------------------------------------------------------------------

/// What `--execute --import` would actually do to each resource.
///
/// The state comes FIRST, exactly as the import path orders it: an address the
/// state already manages is skipped whatever its outcome, so a dry run that
/// ranked it by outcome answered a question nobody asked. On one organisation
/// that read as 25 resources to import when 22 were already managed and the
/// three that mattered — the ones failing `apply` with "already exists" — were
/// indistinguishable in the list.
/// The address this same live object is ALREADY managed under, when the estate
/// now declares it somewhere else.
///
/// A pack that renames a block does not change the object in the cloud, only
/// the name the estate gives it. Asking "is this address managed" answers no,
/// so adopt used to import — and a live policy that is in state twice is one
/// the next plan proposes to DESTROY under its old name, which deletes it for
/// real (E08, 2026-09-09, the first 2.1→2.6 upgrade). The answer is a
/// `state mv`, and the only safe evidence of sameness is an exact match on
/// both the type and the live id.
pub(crate) fn moved_from<'a>(
    r: &Resolution,
    state: &'a crate::bootstrap::StateIndex,
) -> Option<&'a str> {
    if state.manages(&r.address) {
        return None;
    }
    let id = match &r.outcome {
        Outcome::Resolved { id, .. }
        | Outcome::NeedsActivation { id, .. }
        | Outcome::AlreadyAdopted(id) => id,
        _ => return None,
    };
    state.address_of(&r.tf_type, id).filter(|old| *old != r.address)
}

/// Moves the estate itself blocks, as `(new address, old address)` pairs.
///
/// If the estate still DECLARES the old address, then one live object has two
/// declarations. Moving would not resolve that — it would only change which of
/// the two the next plan wants to create. Naming both and stopping is the only
/// honest answer; the estate has to drop one of them first.
pub(crate) fn move_conflicts(
    resolutions: &[Resolution],
    state: &crate::bootstrap::StateIndex,
) -> Vec<(String, String)> {
    let declared: std::collections::BTreeSet<&str> = resolutions
        .iter()
        .filter(|r| !matches!(r.outcome, Outcome::Skipped))
        .map(|r| r.address.as_str())
        .collect();
    resolutions
        .iter()
        .filter_map(|r| moved_from(r, state).map(|old| (r.address.clone(), old.to_string())))
        .filter(|(_, old)| declared.contains(old.as_str()))
        .collect()
}

/// What a row asks of whoever reads it. The verdict is the sentence; this is the
/// sort key, and the order below is the order of urgency: a row that did not
/// answer first, then the state moves, then the imports, and last the rows that
/// ask for nothing. `satz_adopt` keeps its rows in this order and drops the
/// `None`s, which is how a table of hundreds of rows becomes a result a client
/// can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RowAction {
    /// the lookup did not answer: failed, unresolvable, ambiguous, without a
    /// rule, or waiting on an activation
    Unresolved,
    /// the live object is in the state under another address — `state mv`
    Move,
    /// live and unmanaged — import it, activating the constraint first where the
    /// verdict says so
    Import,
    /// nothing to do: already managed, already adopted, or apply creates it
    None,
}

/// One row of the adopt table: what adopt found for a declared resource and what
/// it would do. The table renders these; `satz_adopt` returns them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct AdoptRow {
    pub address: String,
    /// `IMPORT`, `MOVE`, `adopted`, `on apply`, `AMBIGUOUS`, `FAILED`, …
    pub verdict: String,
    pub detail: String,
    /// what this row asks for, the verdict sorted into four buckets
    pub action: RowAction,
    /// what the live lookup matched on, when it matched on a natural key
    pub matched_on: Option<String>,
    /// the address the same live object is managed under, for a MOVE
    pub move_from: Option<String>,
    /// a second line the row carries
    pub note: Option<String>,
    /// the Satz file and line that declared the resource
    pub declared_at: Option<String>,
}

/// The rows that ask for something, most urgent first, at most `limit` of them —
/// what a result carries when the whole table does not fit. The count returned is
/// how many of `all` are left out, the rows that ask for nothing included: a
/// caller that states both numbers cannot pass the short list off as the table.
pub(crate) fn attention(all: &[AdoptRow], limit: usize) -> (Vec<AdoptRow>, usize) {
    let mut kept: Vec<AdoptRow> = all.iter().filter(|r| r.action != RowAction::None).cloned().collect();
    kept.sort_by(|a, b| a.action.cmp(&b.action).then_with(|| a.address.cmp(&b.address)));
    kept.truncate(limit);
    let omitted = all.len() - kept.len();
    (kept, omitted)
}

/// The rows of the adopt table, one per declared resource adopt says something
/// about: already managed, moved, or its resolution.
pub(crate) fn rows(
    resolutions: &[Resolution],
    in_state: &crate::bootstrap::StateIndex,
    manifest: &crate::manifest::Manifest,
) -> Vec<AdoptRow> {
    let row = |r: &Resolution, verdict: &str, detail: String, action: RowAction| AdoptRow {
        address: r.address.clone(),
        verdict: verdict.to_string(),
        detail,
        action,
        matched_on: None,
        move_from: None,
        note: None,
        declared_at: r.origin.as_ref().map(|(f, l)| format!("{}:{}", f, l)),
    };
    let mut out = Vec::new();
    for r in resolutions {
        if in_state.manages(&r.address) {
            // The same words the import path prints, so the dry run and the run
            // are recognisably the same statement.
            out.push(row(r, "already managed in the state", "skipped".into(), RowAction::None));
            continue;
        }
        if let Some(old) = moved_from(r, in_state) {
            // Said before the outcome, because the outcome is "IMPORT" and
            // importing is precisely the wrong move here.
            let mut moved = row(r, "MOVE", format!("in state as {} — the same live object, so `state mv`, not an import", old), RowAction::Move);
            moved.move_from = Some(old.to_string());
            // After the move the state holds its rules under an address the
            // estate declares reset, which the API refuses as an update.
            if in_state.holds_rules(old) && manifest.resources.get(&r.address).is_some_and(|m| m.reset) {
                moved.note = Some("holds rules and is declared reset — `satz plan` and `satz apply` replace it".into());
            }
            out.push(moved);
            continue;
        }
        let (verdict, detail, action) = match &r.outcome {
            Outcome::AlreadyAdopted(id) => ("adopted", id.clone(), RowAction::None),
            Outcome::Resolved { id, verified: true } => ("IMPORT", id.clone(), RowAction::Import),
            Outcome::Resolved { id, verified: false } => ("import (derived, unverified)", id.clone(), RowAction::Import),
            Outcome::NeedsActivation { id, .. } => ("ACTIVATE + IMPORT", id.clone(), RowAction::Import),
            Outcome::OnApply => ("on apply", "not live — apply creates it".into(), RowAction::None),
            Outcome::ParentOnApply(why) => ("on apply (parent)", why.clone(), RowAction::None),
            Outcome::Ambiguous(c) => (
                "AMBIGUOUS",
                format!("{} candidates: {} — pin \"import-id\" by hand", c.len(), c.join(", ")),
                RowAction::Unresolved,
            ),
            Outcome::NeedsLookup(why) => ("needs lookup", why.clone(), RowAction::Unresolved),
            Outcome::NoRule => (
                "no rule",
                format!("add import_id or match_on for {} to import-config.yaml", r.tf_type),
                RowAction::Unresolved,
            ),
            Outcome::Unresolvable(why) => ("unresolvable", why.clone(), RowAction::Unresolved),
            Outcome::Failed(e) => ("FAILED", e.clone(), RowAction::Unresolved),
            Outcome::Skipped => continue,
        };
        let mut resolved = row(r, verdict, detail, action);
        resolved.note = r.note.clone();
        if !r.natural_key.is_empty() && !matches!(r.outcome, Outcome::Resolved { verified: false, .. } | Outcome::AlreadyAdopted(_)) {
            resolved.matched_on = Some(r.natural_key.clone());
        }
        out.push(resolved);
    }
    out
}

pub(crate) fn render_table(
    resolutions: &[Resolution],
    in_state: &crate::bootstrap::StateIndex,
    manifest: &crate::manifest::Manifest,
) -> String {
    let mut s = String::new();
    let w = resolutions.iter().map(|r| r.address.len()).max().unwrap_or(20).min(72);
    for row in rows(resolutions, in_state, manifest) {
        s.push_str(&format!("  {:w$}  {:30}  {}\n", row.address, row.verdict, row.detail, w = w));
        if let Some(note) = &row.note {
            s.push_str(&format!("  {:w$}  {:30}  {}\n", "", "", note, w = w));
        }
        if let Some(m) = &row.matched_on {
            s.push_str(&format!("  {:w$}  {:30}  matched on: {}\n", "", "", m, w = w));
        }
    }
    s
}

pub(crate) fn summary(
    resolutions: &[Resolution],
    in_state: &crate::bootstrap::StateIndex,
) -> String {
    // Counted over what adopt would ACT on. Leaving the managed ones in every
    // bucket is what made the headline number wrong by an order of magnitude.
    let acts_on: Vec<&Resolution> =
        resolutions.iter().filter(|r| !in_state.manages(&r.address)).collect();
    let managed = resolutions.len() - acts_on.len();
    // A move is not an import and must not be counted as one: the whole point
    // of the row is that importing it would be wrong.
    let (to_move, rest): (Vec<&Resolution>, Vec<&Resolution>) =
        acts_on.into_iter().partition(|r| moved_from(r, in_state).is_some());
    let count = |f: &dyn Fn(&Outcome) -> bool| rest.iter().filter(|r| f(&r.outcome)).count();
    format!(
        "adopt: {} to import ({} verified live, {} derived), {} to move (already in state under another address), {} need activation, {} already adopted, {} on apply, {} on apply with their project, {} ambiguous, {} without a rule, {} unresolvable, {} failed, {} already managed in the state",
        count(&|o| matches!(o, Outcome::Resolved { .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: true, .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: false, .. })),
        to_move.len(),
        count(&|o| matches!(o, Outcome::NeedsActivation { .. })),
        count(&|o| matches!(o, Outcome::AlreadyAdopted(_))),
        count(&|o| matches!(o, Outcome::OnApply)),
        count(&|o| matches!(o, Outcome::ParentOnApply(_))),
        count(&|o| matches!(o, Outcome::Ambiguous(_))),
        count(&|o| matches!(o, Outcome::NoRule)),
        count(&|o| matches!(o, Outcome::Unresolvable(_))),
        count(&|o| matches!(o, Outcome::Failed(_))),
        managed,
    )
}

/// The resolutions that mean the run did not answer its question: a failed
/// lookup, an unresolvable or ambiguous resource, a type without a rule.
/// Zero means the table is complete; anything else is a non-zero exit.
/// Rows that did not answer their question — and that adopt would actually act
/// on. A resource the state already manages is skipped either way, so an
/// unresolved one is not a reason to refuse the run.
pub(crate) fn unanswered(
    resolutions: &[Resolution],
    in_state: &crate::bootstrap::StateIndex,
) -> usize {
    resolutions
        .iter()
        .filter(|r| !in_state.manages(&r.address))
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
            let Some(idx) = (line as usize).checked_sub(1) else {
                hints.push(format!("{}: {}:0 is not a line", address, file));
                continue;
            };
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
        crate::fsx::write_edited_satz(&file, &text, &out).map_err(|e| format!("{}: {}", file, e))?;
    }
    Ok((written, hints))
}

fn is_pristine_pack(file: &str, presets_dir: Option<&std::path::Path>) -> bool {
    let Some(dir) = presets_dir else { return false };
    let f = std::path::Path::new(file);
    let under = match (crate::fsx::canonicalize(f), crate::fsx::canonicalize(dir)) {
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

    async fn org_policy(&mut self, parent: &str, constraint: &str) -> Result<Option<bool>, String> {
        if !self.policies.contains_key(parent) {
            let client = self.org_policy_client().await?;
            let current = crate::org_policy::fetch_current(client, parent).await.map_err(|e| e.to_string())?;
            self.policies.insert(parent.to_string(), current);
        }
        Ok(self.policies[parent].get(constraint).map(|policy| {
            policy.pointer("/spec/rules").and_then(|r| r.as_array()).is_some_and(|r| !r.is_empty())
        }))
    }

    async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String> {
        crate::gcp::billing::list_budgets(&self.http, &self.token, billing_account).await.map_err(String::from)
    }

    async fn iam_policy(&mut self, api: PolicyApi, resource: &str) -> Result<Option<serde_json::Value>, String> {
        crate::gcp::iam_policy::read(&self.http, &self.token, api, resource).await.map_err(String::from)
    }
}

#[cfg(test)]
mod tests {
    /// No state read: nothing is managed, so every row is judged on its outcome
    /// alone — what these tests are about.
    fn none() -> crate::bootstrap::StateIndex {
        crate::bootstrap::StateIndex::default()
    }

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
        /// live policies that hold rules — a subset of `policies`
        policies_with_rules: BTreeSet<(String, String)>,
        searches: BTreeMap<(String, String), Vec<(String, serde_json::Value)>>,
        budgets: BTreeMap<String, Vec<(String, String)>>,
        /// IAM policies by resource; a missing key is a read that fails
        iam: BTreeMap<String, Result<Option<serde_json::Value>, String>>,
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
        async fn org_policy(&mut self, parent: &str, constraint: &str) -> Result<Option<bool>, String> {
            self.calls.push(format!("policy {} {}", parent, constraint));
            let key = (parent.to_string(), constraint.to_string());
            Ok(self.policies.contains(&key).then(|| self.policies_with_rules.contains(&key)))
        }
        async fn search(&mut self, scope: &str, asset_type: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
            self.calls.push(format!("search {} {}", scope, asset_type));
            Ok(self.searches.get(&(scope.to_string(), asset_type.to_string())).cloned().unwrap_or_default())
        }
        async fn budgets(&mut self, billing_account: &str) -> Result<Vec<(String, String)>, String> {
            self.calls.push(format!("budgets {}", billing_account));
            Ok(self.budgets.get(billing_account).cloned().unwrap_or_default())
        }
        async fn iam_policy(&mut self, api: PolicyApi, resource: &str) -> Result<Option<serde_json::Value>, String> {
            self.calls.push(format!("iam {:?} {}", api, resource));
            self.iam.get(resource).cloned().unwrap_or_else(|| Err(format!("no policy fixture for {}", resource)))
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
                    skip: None,
                    asset_type: on.map(|_| format!("test.googleapis.com/{}", t)),
                    content_type: None,
                    exclude: None,
                    include: None,
                    derive_yaml_key_from: None,
                    import_id: template.map(|s| s.to_string()),
                    match_on: on.map(|v| v.iter().map(|s| s.to_string()).collect()),
                    activate: None,
                    map: None,
                    api_schema: None,
                },
            );
        }
        ImportConfig { provider_version: None, root: None, only: None, exclude: None, resource_types }
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
            policies_with_rules: BTreeSet::new(),
            searches: BTreeMap::new(),
            budgets: BTreeMap::new(),
            iam: BTreeMap::new(),
            calls: vec![],
        };
        f.iam.insert(
            "folders/222".into(),
            Ok(Some(serde_json::json!({ "bindings": [{ "role": "roles/viewer", "members": ["group:x@example.com"] }] }))),
        );
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
        // the folder grant needs the folder NUMBER, which the lookup supplied, and
        // is imported because that folder's live policy holds it
        assert_eq!(
            outcome(&rs, "google_folder_iam_member.grant"),
            &Outcome::Resolved { id: "folders/222 roles/viewer group:x@example.com".into(), verified: true }
        );
        assert!(live.calls.contains(&"iam ResourceManager folders/222".to_string()), "{:?}", live.calls);
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
        assert_eq!(unanswered(&rs, &none()), rs.iter().filter(|r| matches!(r.outcome, Outcome::Ambiguous(_) | Outcome::NoRule)).count(), "a missing project is a finding, not a failure");
        assert!(summary(&rs, &none()).contains("2 on apply with their project"), "{}", summary(&rs, &none()));
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
        assert!(unanswered(&rs, &none()) >= 2, "{}", summary(&rs, &none()));
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

    /// Found on a vanilla organisation: adopt imported a live policy with rules onto
    /// the `-superseded` address, which declares reset, and the next apply was refused
    /// with "Cannot set PolicyRules if reset is true". The import is right; the row says
    /// what the apply will have to do with it.
    #[tokio::test]
    async fn importing_live_rules_onto_a_reset_declaration_says_the_apply_replaces_it() {
        let manifest = Manifest::parse(
            "resource \"google_org_policy_policy\" \"iam_disableServiceAccountKeyUpload_superseded\" {\n  name = \"organizations/123456789012/policies/iam.disableServiceAccountKeyUpload\"\n  parent = \"organizations/123456789012\"\n  spec {\n    reset = true\n  }\n}\n\
             resource \"google_org_policy_policy\" \"iam_allowedPolicyMemberDomains_superseded\" {\n  name = \"organizations/123456789012/policies/iam.allowedPolicyMemberDomains\"\n  parent = \"organizations/123456789012\"\n  spec {\n    reset = true\n  }\n}\n",
        );
        let mut live = fake();
        for c in ["iam.disableServiceAccountKeyUpload", "iam.allowedPolicyMemberDomains"] {
            live.policies.insert(("organizations/123456789012".into(), c.into()));
        }
        live.policies_with_rules.insert(("organizations/123456789012".into(), "iam.disableServiceAccountKeyUpload".into()));
        let rs = resolve(&manifest, &rules(&[]), &Options { only: BTreeSet::new(), activate: false }, &mut live).await;
        let table = render_table(&rs, &crate::bootstrap::StateIndex::default(), &manifest);
        assert!(
            table.contains("holds rules live and is declared reset")
                && table.contains("-replace=google_org_policy_policy.iam_disableServiceAccountKeyUpload_superseded"),
            "{table}"
        );
        // a live policy already reset imports without a note
        assert_eq!(table.matches("holds rules live").count(), 1, "{table}");
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
            Resolution { address: "google_folder.workloads".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111".into(), verified: true }, origin: Some((file.clone(), 2)), org_policy: None, note: None },
            Resolution { address: "google_folder.one".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/1".into(), verified: true }, origin: Some((file.clone(), 5)), org_policy: None, note: None },
            Resolution { address: "google_folder_iam_member.g".into(), tf_type: "google_folder_iam_member".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111 r m".into(), verified: false }, origin: None, org_policy: None, note: None },
            Resolution { address: "google_org_policy_policy.x".into(), tf_type: "google_org_policy_policy".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "organizations/1/policies/compute.x".into(), verified: false }, origin: Some((pack.to_string_lossy().to_string(), 3)), org_policy: None, note: None },
        ];
        let (written, hints) = write_import_ids(&rs, Some(&presets)).unwrap();
        let text = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(text, "google_folder {\n  workloads {\n    \"import-id\"  = \"folders/111\"\n    display_name = \"Workloads\"\n  }\n  one { display_name = \"x\" }\n}\n", "written after the declaring line, and the formatted file stays formatted");
        assert_eq!(written.len(), 1);
        assert_eq!(hints.len(), 3, "{:?}", hints);
        assert!(hints.iter().any(|h| h.contains("google_folder.one") && h.contains("by hand")));
        assert!(hints.iter().any(|h| h.contains("google_folder_iam_member.g") && h.contains("no declaring line")));
        assert!(hints.iter().any(|h| h.contains("google_org_policy_policy.x") && h.contains("pristine pack")));
        assert!(!std::fs::read_to_string(&pack).unwrap().contains("import-id"), "a pristine pack is never edited");
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_dir_all(&presets);
    }

    #[test]
    fn a_reference_resolves_through_its_target_and_an_embedded_one_says_it_cannot() {
        let manifest = Manifest::parse(
            "resource \"google_project\" \"mgmt\" {\n  project_id = \"acme-mdc-mgmt\"\n}\n\
             resource \"google_storage_bucket\" \"b\" {\n  name = \"acme-audit\"\n  project = \"${google_project.mgmt.project_id}\"\n}\n\
             resource \"google_service_account_iam_member\" \"a\" {\n  role = \"roles/iam.workloadIdentityUser\"\n  member = \"principalSet://iam.googleapis.com/projects/${google_project.mgmt.number}/locations/global/workloadIdentityPools/p/*\"\n}\n",
        );
        let known = Known::default();
        // whole-value reference: followed to the target's own attribute
        let bucket = &manifest.resources["google_storage_bucket.b"];
        assert_eq!(value_of(bucket, "project", &manifest, &known).unwrap(), "acme-mdc-mgmt");
        // embedded reference: not a literal, and not silently searched for
        let grant = &manifest.resources["google_service_account_iam_member.a"];
        let err = value_of(grant, "member", &manifest, &known).unwrap_err().to_string();
        assert!(err.contains("only known after apply"), "{}", err);
        assert!(!err.contains("has no `member`"), "{}", err);
    }

    /// A grant's import id renders offline, so every declared grant used to be an
    /// import candidate, and `--execute --import` ran `tofu import` for each one the
    /// organisation did not hold — a failure line per grant apply was about to
    /// create. The live policy of the parent decides now.
    #[tokio::test]
    async fn a_grant_is_imported_only_when_the_live_policy_of_its_parent_holds_it() {
        let manifest = Manifest::parse(concat!(
            "resource \"google_organization_iam_member\" \"held\" {\n  org_id = \"123456789012\"\n  role = \"roles/viewer\"\n  member = \"group:Auditors@example.com\"\n}\n",
            "resource \"google_organization_iam_member\" \"absent\" {\n  org_id = \"123456789012\"\n  role = \"roles/billing.creator\"\n  member = \"group:billing-admins@example.com\"\n}\n",
            "resource \"google_organization_iam_member\" \"only_conditional_live\" {\n  org_id = \"123456789012\"\n  role = \"roles/browser\"\n  member = \"group:auditors@example.com\"\n}\n",
            "resource \"google_organization_iam_member\" \"cond_held\" {\n  org_id = \"123456789012\"\n  role = \"roles/browser\"\n  member = \"group:auditors@example.com\"\n  condition {\n    title = \"weekdays\"\n    expression = \"request.time.getDayOfWeek() < 5\"\n  }\n}\n",
            "resource \"google_organization_iam_member\" \"cond_other\" {\n  org_id = \"123456789012\"\n  role = \"roles/browser\"\n  member = \"group:auditors@example.com\"\n  condition {\n    title = \"weekdays-only\"\n    expression = \"request.time.getDayOfWeek() < 5\"\n  }\n}\n",
            "resource \"google_organization_iam_member\" \"cond_absent\" {\n  org_id = \"123456789012\"\n  role = \"roles/viewer\"\n  member = \"group:billing-admins@example.com\"\n  condition {\n    title = \"weekdays\"\n    expression = \"request.time.getDayOfWeek() < 5\"\n  }\n}\n",
            "resource \"google_storage_bucket_iam_member\" \"gone\" {\n  bucket = \"acme-state\"\n  role = \"roles/storage.objectViewer\"\n  member = \"group:auditors@example.com\"\n}\n",
            "resource \"google_project_iam_member\" \"denied\" {\n  project = \"acme-infra-001\"\n  role = \"roles/viewer\"\n  member = \"group:auditors@example.com\"\n}\n",
        ));
        let rules = rules(&[
            ("google_organization_iam_member", Some("{org_id} {role} {member}"), None),
            ("google_storage_bucket_iam_member", Some("b/{bucket} {role} {member}"), None),
            ("google_project_iam_member", Some("{project} {role} {member}"), None),
        ]);
        let mut live = fake();
        live.iam.insert(
            "organizations/123456789012".into(),
            Ok(Some(serde_json::json!({ "version": 3, "bindings": [
                { "role": "roles/viewer", "members": ["group:auditors@example.com"] },
                { "role": "roles/browser", "members": ["group:auditors@example.com"],
                  "condition": { "title": "weekdays", "expression": "request.time.getDayOfWeek() < 5" } },
            ]}))),
        );
        live.iam.insert("b/acme-state".into(), Ok(None));
        live.iam.insert("projects/acme-infra-001".into(), Err("403 Forbidden [PERMISSION_DENIED]: getIamPolicy".into()));
        let rs = resolve(&manifest, &rules, &Options { only: BTreeSet::new(), activate: false }, &mut live).await;

        // held live, the member compared as IAM compares an address
        assert_eq!(
            outcome(&rs, "google_organization_iam_member.held"),
            &Outcome::Resolved { id: "123456789012 roles/viewer group:Auditors@example.com".into(), verified: true }
        );
        // not in the policy: apply creates it, nothing is imported
        assert_eq!(outcome(&rs, "google_organization_iam_member.absent"), &Outcome::OnApply);
        // the member holds the role live only under a condition: the unconditional grant is not live
        assert_eq!(outcome(&rs, "google_organization_iam_member.only_conditional_live"), &Outcome::OnApply);
        // the same condition live: imported under the provider's id, which ends in the title
        assert_eq!(
            outcome(&rs, "google_organization_iam_member.cond_held"),
            &Outcome::Resolved { id: "123456789012 roles/browser group:auditors@example.com weekdays".into(), verified: true }
        );
        // a live condition that is not the declared one is not guessed to be it
        assert_eq!(
            outcome(&rs, "google_organization_iam_member.cond_other"),
            &Outcome::Ambiguous(vec!["weekdays: request.time.getDayOfWeek() < 5".into()])
        );
        assert_eq!(outcome(&rs, "google_organization_iam_member.cond_absent"), &Outcome::OnApply);
        // the bucket does not exist: its grants are created with it
        let gone = outcome(&rs, "google_storage_bucket_iam_member.gone");
        assert!(matches!(gone, Outcome::ParentOnApply(why) if why.contains("b/acme-state")), "{:?}", gone);
        // a policy that cannot be read is a failure, never an empty policy
        let denied = outcome(&rs, "google_project_iam_member.denied");
        assert!(matches!(denied, Outcome::Failed(e) if e.contains("projects/acme-infra-001") && e.contains("403")), "{:?}", denied);

        // one read per parent, however many grants it carries
        assert_eq!(live.calls.iter().filter(|c| *c == "iam ResourceManager organizations/123456789012").count(), 1, "{:?}", live.calls);
        assert!(!rs.iter().any(|r| matches!(r.outcome, Outcome::Resolved { verified: false, .. })), "no grant is left derived");

        let s = summary(&rs, &none());
        assert!(s.starts_with("adopt: 2 to import (2 verified live, 0 derived)"), "{}", s);
        assert!(s.contains("3 on apply, 1 on apply with their project, 1 ambiguous"), "{}", s);
        assert!(s.contains("1 failed"), "{}", s);
        let table = render_table(&rs, &none(), &manifest);
        let absent = table.lines().find(|l| l.contains("iam_member.absent")).unwrap_or_default();
        assert!(absent.contains("on apply") && absent.contains("apply creates it"), "{}", table);
        // the ambiguous and the failed row stop the run before anything is imported
        assert_eq!(unanswered(&rs, &none()), 2);
    }

    /// The estate that declares a service account grants on it by reference:
    /// `service_account_id = "${google_service_account.x.name}"`. `name` is the
    /// account's live id, not an attribute the estate writes, so the reference is
    /// answered from the id this run resolved for that account — and the grant is
    /// then decided against that account's live IAM policy like any other.
    #[tokio::test]
    async fn a_grant_follows_the_reference_to_the_account_it_is_made_on() {
        let manifest = grants_on_an_account();
        let mut live = fake();
        live.iam.insert(
            SA.into(),
            Ok(Some(serde_json::json!({ "version": 3, "bindings": [
                { "role": "roles/iam.serviceAccountTokenCreator", "members": ["group:auditors@example.com"] },
            ]}))),
        );
        live.iam.insert(
            HAND_MADE.into(),
            Ok(Some(serde_json::json!({ "version": 3, "bindings": [
                { "role": "roles/iam.serviceAccountUser", "members": ["group:auditors@example.com"] },
            ]}))),
        );
        let rs = resolve(&manifest, &account_rules(), &Options { only: BTreeSet::new(), activate: false }, &mut live).await;

        assert_eq!(
            outcome(&rs, "google_service_account_iam_member.held"),
            &Outcome::Resolved { id: format!("{} roles/iam.serviceAccountTokenCreator group:auditors@example.com", SA), verified: true }
        );
        // the policy was read on the account the reference names
        assert!(live.calls.contains(&format!("iam ServiceAccount {}", SA)), "{:?}", live.calls);
        // a grant that account's policy does not hold is created by apply
        assert_eq!(outcome(&rs, "google_service_account_iam_member.absent"), &Outcome::OnApply);
        // a literal parent resolves the same way
        assert_eq!(
            outcome(&rs, "google_service_account_iam_member.elsewhere"),
            &Outcome::Resolved { id: format!("{} roles/iam.serviceAccountUser group:auditors@example.com", HAND_MADE), verified: true }
        );
        assert_eq!(unanswered(&rs, &none()), 0, "{}", render_table(&rs, &none(), &manifest));
    }

    /// An account that does not exist yet takes its grants with it — whether the
    /// account is absent live or its project is: "on apply (parent)", and the run
    /// answers everything it was asked.
    #[tokio::test]
    async fn a_grant_on_an_account_that_is_not_live_reads_on_apply_with_it() {
        let manifest = grants_on_an_account();
        let mut live = fake();
        live.iam.insert(SA.into(), Ok(None)); // the account does not exist live
        live.iam.insert(HAND_MADE.into(), Ok(None));
        let rs = resolve(&manifest, &account_rules(), &Options { only: BTreeSet::new(), activate: false }, &mut live).await;
        let held = outcome(&rs, "google_service_account_iam_member.held");
        assert!(matches!(held, Outcome::ParentOnApply(why) if why.contains(SA)), "{:?}", held);
        assert_eq!(unanswered(&rs, &none()), 0, "a grant whose account apply creates is a finding, not a failure");
        let table = render_table(&rs, &none(), &manifest);
        assert!(table.lines().any(|l| l.contains("iam_member.held") && l.contains("on apply (parent)")), "{}", table);

        // the same verdict one step earlier: the account's project is not live, so
        // the account is not either — and no policy is read under an id nothing has
        let mut live = fake();
        live.projects.clear();
        live.iam.insert(HAND_MADE.into(), Ok(None));
        let rs = resolve(&manifest, &account_rules(), &Options { only: BTreeSet::new(), activate: false }, &mut live).await;
        let held = outcome(&rs, "google_service_account_iam_member.held");
        assert!(matches!(held, Outcome::ParentOnApply(why) if why.contains("google_service_account.sa")), "{:?}", held);
        assert!(!live.calls.iter().any(|c| c.contains(SA)), "{:?}", live.calls);
        assert_eq!(unanswered(&rs, &none()), 0, "{}", render_table(&rs, &none(), &manifest));
    }

    /// A reference adoption cannot follow is still unresolvable, and names the
    /// resource and why: the run stops instead of importing a guessed id.
    #[tokio::test]
    async fn a_reference_adopt_cannot_follow_stays_unresolvable() {
        let manifest = Manifest::parse(concat!(
            "resource \"google_service_account\" \"sa\" {\n  account_id = \"svc-iac\"\n  project = \"acme-infra-001\"\n}\n",
            "resource \"google_service_account_iam_member\" \"ghost\" {\n  service_account_id = \"${google_service_account.gone.name}\"\n  role = \"roles/iam.serviceAccountUser\"\n  member = \"group:auditors@example.com\"\n}\n",
            "resource \"google_service_account_iam_member\" \"by_email\" {\n  service_account_id = \"${google_service_account.sa.email}\"\n  role = \"roles/iam.serviceAccountUser\"\n  member = \"group:auditors@example.com\"\n}\n",
        ));
        let mut live = fake();
        let rs = resolve(&manifest, &account_rules(), &Options { only: BTreeSet::new(), activate: false }, &mut live).await;
        let ghost = outcome(&rs, "google_service_account_iam_member.ghost");
        assert!(
            matches!(ghost, Outcome::Unresolvable(why) if why.contains("google_service_account.gone") && why.contains("not emitted")),
            "{:?}",
            ghost
        );
        let by_email = outcome(&rs, "google_service_account_iam_member.by_email");
        assert!(
            matches!(by_email, Outcome::Unresolvable(why) if why.contains("google_service_account.sa.email") && why.contains("not a literal")),
            "{:?}",
            by_email
        );
        assert!(!live.calls.iter().any(|c| c.starts_with("iam ")), "no policy is read for a parent nothing named: {:?}", live.calls);
        assert_eq!(unanswered(&rs, &none()), 2, "{}", render_table(&rs, &none(), &manifest));
    }

    /// Grants are resolved last. Address order alone does not put a grant behind
    /// the resource it is made on — `google_folder_iam_member.grant` sorts before
    /// `google_widget.w` — so the order does, and a grant always reads a parent
    /// whose verdict and live id the run already holds.
    #[test]
    fn grants_are_resolved_after_every_resource_they_can_be_made_on() {
        let m = manifest();
        let order: Vec<String> = ordered(&m).iter().map(|r| r.address()).collect();
        let at = |a: &str| order.iter().position(|x| x == a).unwrap_or_else(|| panic!("{} is not in {:?}", a, order));
        assert!(at("google_folder_iam_member.grant") > at("google_service_account.sa"), "{:?}", order);
        assert!("google_folder_iam_member.grant" < "google_widget.w", "address order would reach the grant first");
        assert!(at("google_folder_iam_member.grant") > at("google_widget.w"), "{:?}", order);
    }

    /// `projects/<project>/serviceAccounts/<email>` — the id the account's own rule
    /// renders, and what `google_service_account.sa.name` denotes.
    const SA: &str = "projects/acme-infra-001/serviceAccounts/svc-iac@acme-infra-001.iam.gserviceaccount.com";
    /// An account the estate does not declare: its grant names it literally.
    const HAND_MADE: &str = "projects/acme-infra-001/serviceAccounts/hand-made@acme-infra-001.iam.gserviceaccount.com";

    fn grants_on_an_account() -> Manifest {
        Manifest::parse(concat!(
            "resource \"google_project\" \"infra\" {\n  project_id = \"acme-infra-001\"\n}\n",
            "resource \"google_service_account\" \"sa\" {\n  account_id = \"svc-iac\"\n  project = \"${google_project.infra.project_id}\"\n}\n",
            "resource \"google_service_account_iam_member\" \"held\" {\n  service_account_id = \"${google_service_account.sa.name}\"\n  role = \"roles/iam.serviceAccountTokenCreator\"\n  member = \"group:auditors@example.com\"\n}\n",
            "resource \"google_service_account_iam_member\" \"absent\" {\n  service_account_id = \"${google_service_account.sa.name}\"\n  role = \"roles/iam.workloadIdentityUser\"\n  member = \"group:auditors@example.com\"\n}\n",
            "resource \"google_service_account_iam_member\" \"elsewhere\" {\n  service_account_id = \"projects/acme-infra-001/serviceAccounts/hand-made@acme-infra-001.iam.gserviceaccount.com\"\n  role = \"roles/iam.serviceAccountUser\"\n  member = \"group:auditors@example.com\"\n}\n",
        ))
    }

    fn account_rules() -> ImportConfig {
        rules(&[
            ("google_project", Some("{project_id}"), None),
            ("google_service_account", Some("projects/{project}/serviceAccounts/{account_id}@{project}.iam.gserviceaccount.com"), None),
            ("google_service_account_iam_member", Some("{service_account_id} {role} {member}"), None),
        ])
    }

    #[test]
    fn a_grant_declared_under_an_unresolvable_condition_is_never_read_as_absent_when_the_member_holds_the_role_conditionally() {
        let policy = serde_json::json!({ "bindings": [
            { "role": "roles/browser", "members": ["user:a@example.com"], "condition": { "title": "t", "expression": "e" } },
        ]});
        let cond = DeclaredCondition::Undecidable;
        assert_eq!(grant_in_policy(&policy, "roles/browser", "user:a@example.com", &cond), GrantLive::Unclear(vec!["t: e".into()]));
        assert_eq!(grant_in_policy(&policy, "roles/viewer", "user:a@example.com", &cond), GrantLive::Absent);
        // a principal that is not an address compares exactly
        assert!(!same_member("principal://x/Subject/A", "principal://x/subject/a"));
        assert!(same_member("serviceAccount:A@example.com", "serviceAccount:a@example.com"));
    }

    /// Every grant row whose id is `<parent> <role> <member>` is checked against the
    /// live policy: a new one without a reader would be imported blind again.
    #[test]
    fn every_shipped_grant_template_has_a_live_policy_reader() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/import-config.yaml");
        let cfg: ImportConfig = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).expect("import-config.yaml parses");
        let grants: Vec<&String> = cfg
            .resource_types
            .iter()
            .filter(|(_, row)| row.import_id.as_deref().is_some_and(|t| t.ends_with(" {role} {member}")))
            .map(|(t, _)| t)
            .collect();
        assert!(grants.len() >= 9, "{:?}", grants);
        for t in grants {
            assert!(grant_parent(t, "x").is_some(), "{} renders a grant id but adopt reads no live policy for it", t);
        }
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

#[cfg(test)]
mod state_aware_tests {
    //! `--execute --import` skips every address the state already manages, so a
    //! dry run that ranks those as "IMPORT" describes a run that will not
    //! happen. On one organisation the table read as 25 resources to import when
    //! 22 were already managed and the three that mattered — the ones failing
    //! `apply` with "already exists" — were indistinguishable in the list.
    use super::{Outcome, Resolution, move_conflicts, moved_from, render_table, summary, unanswered};

    fn res(address: &str, outcome: Outcome) -> Resolution {
        Resolution {
            address: address.to_string(),
            tf_type: "google_org_policy_policy".into(),
            natural_key: String::new(),
            outcome,
            origin: None,
            org_policy: None,
        note: None,
        }
    }

    fn three() -> Vec<Resolution> {
        vec![
            res("google_org_policy_policy.a", Outcome::Resolved { id: "a".into(), verified: true }),
            res("google_org_policy_policy.b", Outcome::Resolved { id: "b".into(), verified: true }),
            res("google_org_policy_policy.c", Outcome::Resolved { id: "c".into(), verified: true }),
        ]
    }

    /// Addresses the state manages, with no live object behind them — enough
    /// for the "already managed" rows, which are decided on the address alone.
    fn managed(addresses: &[&str]) -> crate::bootstrap::StateIndex {
        let objects: Vec<(&str, &str, &str)> =
            addresses.iter().map(|a| (*a, "", "")).collect();
        crate::bootstrap::StateIndex::from_objects(&objects)
    }

    #[test]
    fn a_managed_address_is_reported_as_skipped_not_as_an_import() {
        let rs = three();
        let table = render_table(&rs, &managed(&["google_org_policy_policy.a"]), &crate::manifest::Manifest::default());
        let a = table.lines().find(|l| l.contains(".a ")).unwrap_or_default();
        // The same words the import path prints, so the dry run and the run are
        // recognisably the same statement.
        assert!(a.contains("already managed in the state"), "{}", table);
        assert!(!a.contains("IMPORT"), "{}", table);
        assert!(table.lines().any(|l| l.contains(".b") && l.contains("IMPORT")), "{}", table);
    }

    /// The headline number is the one an operator acts on. Counting managed
    /// resources into it is what made it wrong by an order of magnitude.
    #[test]
    fn the_summary_counts_only_what_adopt_would_act_on() {
        let rs = three();
        let out = summary(&rs, &managed(&["google_org_policy_policy.a", "google_org_policy_policy.b"]));
        assert!(out.starts_with("adopt: 1 to import"), "{}", out);
        assert!(out.contains("2 already managed in the state"), "{}", out);

        // With no state read, nothing is managed and the count is what it was.
        let out = summary(&rs, &managed(&[]));
        assert!(out.starts_with("adopt: 3 to import"), "{}", out);
        assert!(out.contains("0 already managed in the state"), "{}", out);
    }

    /// An address the state manages is skipped either way, so failing to resolve
    /// it is not a reason to refuse the run.
    #[test]
    fn an_unresolved_row_the_state_already_manages_is_not_a_failure() {
        let rs = vec![
            res("google_org_policy_policy.a", Outcome::NoRule),
            res("google_org_policy_policy.b", Outcome::NoRule),
        ];
        assert_eq!(unanswered(&rs, &managed(&[])), 2);
        assert_eq!(unanswered(&rs, &managed(&["google_org_policy_policy.a"])), 1);
        assert_eq!(
            unanswered(&rs, &managed(&["google_org_policy_policy.a", "google_org_policy_policy.b"])),
            0,
            "every unresolved row is already managed — there is nothing to refuse"
        );
    }

    // --- the renamed block: move, never import (E08, 2026-09-09) ------------

    /// The live policy CIS 2.6 renamed. The estate declares the `-superseded`
    /// address; the state still carries the object under the old one.
    const LIVE_ID: &str = "organizations/1/policies/compute.restrictProtocolForwardingCreationForTypes";
    const OLD: &str = "google_org_policy_policy.compute_restrictProtocolForwarding";
    const NEW: &str = "google_org_policy_policy.compute_restrictProtocolForwarding_superseded";

    fn renamed() -> (Vec<Resolution>, crate::bootstrap::StateIndex) {
        let rs = vec![res(NEW, Outcome::Resolved { id: LIVE_ID.into(), verified: true })];
        let state = crate::bootstrap::StateIndex::from_objects(&[(OLD, "google_org_policy_policy", LIVE_ID)]);
        (rs, state)
    }

    /// The defect itself: adopt imported the live policy a second time, and the
    /// next plan then proposed to DESTROY the old address — deleting the policy.
    #[test]
    fn a_renamed_block_is_a_move_not_an_import() {
        let (rs, state) = renamed();
        assert_eq!(moved_from(&rs[0], &state), Some(OLD));

        let table = render_table(&rs, &state, &crate::manifest::Manifest::default());
        assert!(table.contains("MOVE"), "{}", table);
        assert!(table.contains(OLD), "the row names the address to move FROM: {}", table);
        assert!(!table.contains("IMPORT"), "importing it is the defect: {}", table);
        assert!(!table.contains("declared reset"), "{}", table);
        // the rows the table renders are what satz_adopt returns
        let r = crate::adopt::rows(&rs, &state, &crate::manifest::Manifest::default());
        assert_eq!((r[0].verdict.as_str(), r[0].move_from.as_deref()), ("MOVE", Some(OLD)));
        assert_eq!(r[0].matched_on, None);
    }

    /// The move carries the old rules to an address declared reset: the row says
    /// the next plan replaces it, as `satz plan` and `satz apply` do.
    #[test]
    fn a_move_onto_a_reset_declaration_says_it_will_be_replaced() {
        let (rs, state) = renamed();
        let state = state.with_rules(&[OLD]);
        let manifest = crate::manifest::Manifest::parse(
            "resource \"google_org_policy_policy\" \"compute_restrictProtocolForwarding_superseded\" {\n  name = \"x\"\n  spec {\n    reset = true\n  }\n}\n",
        );
        let table = render_table(&rs, &state, &manifest);
        assert!(table.contains("holds rules and is declared reset"), "{}", table);
    }

    #[test]
    fn the_summary_counts_a_move_apart_from_an_import() {
        let (rs, state) = renamed();
        let out = summary(&rs, &state);
        assert!(out.starts_with("adopt: 0 to import"), "{}", out);
        assert!(out.contains("1 to move (already in state under another address)"), "{}", out);
    }

    /// The ordinary case must not become a move: same object, same address, so
    /// there is nothing to rename and the existing "already managed" row stands.
    #[test]
    fn an_object_already_at_its_own_address_is_managed_not_moved() {
        let rs = vec![res(NEW, Outcome::Resolved { id: LIVE_ID.into(), verified: true })];
        let state = crate::bootstrap::StateIndex::from_objects(&[(NEW, "google_org_policy_policy", LIVE_ID)]);
        assert_eq!(moved_from(&rs[0], &state), None);
        assert!(render_table(&rs, &state, &crate::manifest::Manifest::default()).contains("already managed in the state"));
    }

    /// Sameness is proven by an exact match on type AND id. Anything less would
    /// move a resource that only looks like the one being adopted.
    #[test]
    fn only_an_exact_object_match_is_a_move() {
        let rs = [res(NEW, Outcome::Resolved { id: LIVE_ID.into(), verified: true })];

        let other_id = crate::bootstrap::StateIndex::from_objects(&[(
            OLD,
            "google_org_policy_policy",
            "organizations/1/policies/compute.somethingElse",
        )]);
        assert_eq!(moved_from(&rs[0], &other_id), None, "a different live id is a different object");

        let other_type =
            crate::bootstrap::StateIndex::from_objects(&[(OLD, "google_folder", LIVE_ID)]);
        assert_eq!(moved_from(&rs[0], &other_type), None, "an id is only unique within its type");
    }

    /// A row with no live id cannot be shown to be the same object as anything.
    #[test]
    fn an_outcome_without_an_id_never_moves() {
        let state = crate::bootstrap::StateIndex::from_objects(&[(OLD, "google_org_policy_policy", LIVE_ID)]);
        for outcome in [
            Outcome::OnApply,
            Outcome::NoRule,
            Outcome::Ambiguous(vec![LIVE_ID.into()]),
            Outcome::ParentOnApply("project is not live".into()),
        ] {
            let r = res(NEW, outcome);
            assert_eq!(moved_from(&r, &state), None, "{:?} carries no id", r.outcome);
        }
    }

    /// A derived (unverified) id still identifies the object, so a renamed block
    /// whose id was rendered from the rule moves rather than importing twice.
    #[test]
    fn a_derived_id_moves_too() {
        let r = res(NEW, Outcome::Resolved { id: LIVE_ID.into(), verified: false });
        let state = crate::bootstrap::StateIndex::from_objects(&[(OLD, "google_org_policy_policy", LIVE_ID)]);
        assert_eq!(moved_from(&r, &state), Some(OLD));
    }

    /// If the estate still declares the OLD address, moving would only change
    /// which of the two declarations the next plan wants to create. Both ends
    /// get named and the run stops.
    #[test]
    fn declaring_both_ends_of_a_move_is_a_conflict() {
        let (mut rs, state) = renamed();
        assert!(move_conflicts(&rs, &state).is_empty(), "the old address is not declared");

        rs.push(res(OLD, Outcome::OnApply));
        let conflicts = move_conflicts(&rs, &state);
        assert_eq!(conflicts, vec![(NEW.to_string(), OLD.to_string())]);
    }
}

/// The adopt dry run, computed: the estate's compile, every declared resource
/// resolved against the live organisation, the live client (activation needs it),
/// and what the state already manages. No printing — the CLI and `satz_adopt`
/// share it. The identity is the caller's: the CLI binds it for the process, the
/// MCP tool scopes it to the call.
pub(crate) struct AdoptPlan {
    pub out: PipelineBOut,
    pub resolutions: Vec<crate::adopt::Resolution>,
    pub live: crate::adopt::RealLive,
    /// What the state already manages, read ONCE and used by both halves of the
    /// command. The dry run has to know it: `--execute --import` skips those
    /// addresses, so a table that ranks them as "IMPORT" describes a run that
    /// will not happen.
    ///
    /// Unreadable is a NOTE for the dry run, never a failure — a first adopt has
    /// no state, and refusing to describe the estate because of that would be
    /// refusing the only thing a dry run is for. The import path still fails
    /// fast, because there the imports really would all fail the same way.
    pub state: Result<crate::bootstrap::StateIndex, String>,
}

pub(crate) async fn adopt_plan(
    input_path: &Path,
    only: Vec<String>,
    activate: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<AdoptPlan, Box<dyn std::error::Error>> {
    // Same compile the emitter uses, so the adopted addresses are exactly the
    // ones `apply` will act on.
    let out = pipeline_b_generate(input_path, tool_config, runtime_config)?;
    let rules = load_import_config(None, tool_config, &runtime_config.presets_dir)?.ok_or(
        "adoption rules live in <presets_dir>/import-config.yaml — run `satz get-presets` so it exists",
    )?;
    let opts = crate::adopt::Options { only: only.into_iter().collect(), activate };
    let mut live = crate::adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = crate::adopt::resolve(&out.manifest, &rules, &opts, &mut live).await;
    let state = crate::bootstrap::state_index(&runtime_config.tf_tool, Path::new(&runtime_config.hcl_dir));
    Ok(AdoptPlan { out, resolutions, live, state })
}

// ── the command line's own arm ─────────────────────────────────────────────
// Everything ABOVE this line is reached by `satz_adopt` over MCP, where stdout
// carries the JSON-RPC protocol: it must never print there, and the gate in
// `src/mcp.rs` asserts it over exactly this region. `run_adopt` below is the CLI
// arm and prints the table a human reads, so the gate stops here.

/// `satz adopt`: compile, resolve every declared resource against the live
/// org, report, and — only with `--execute` — write the verified ids into the
/// estate or import them into state now.
pub(crate) async fn run_adopt(
    input: &str,
    only: Vec<String>,
    execute: bool,
    import: bool,
    activate: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::adopt::{self, Outcome};
    let input_path = estate_path(PathBuf::from(input), runtime_config);
    reject_yaml_dialect(&input_path, "adopt")?;
    configure_estate_impersonation(&input_path, runtime_config)?;
    // a run over every declared resource is the one a notice naming `satz adopt` asks for
    let whole = only.is_empty();
    let AdoptPlan { out, resolutions, mut live, state } =
        adopt_plan(&input_path, only, activate, tool_config, runtime_config).await?;
    let in_state = state.clone().unwrap_or_default();

    println!("\nadopt {} — {} resources declared\n", input_path.display(), out.manifest.resources.len());
    print!("{}", adopt::render_table(&resolutions, &in_state, &out.manifest));
    println!("\n{}", adopt::summary(&resolutions, &in_state));
    if let Err(e) = &state {
        println!(
            "\nnote: the state could not be read ({}), so nothing below is marked as already \
             managed — run `{} init` in {} for the full picture",
            e.lines().next().unwrap_or("(no output)"),
            runtime_config.tf_tool,
            runtime_config.hcl_dir
        );
    }

    // A table with a FAILED / unresolvable / ambiguous / no-rule row did not
    // answer its question: that is an error exit, not a summary count. The
    // table is above; nothing has been changed at this point.
    let unanswered = adopt::unanswered(&resolutions, &in_state);
    if unanswered > 0 {
        return Err(format!(
            "adopt: {} resolution(s) failed, unresolvable, ambiguous or without a rule — see the rows above; nothing was changed",
            unanswered
        )
        .into());
    }

    // One live object with two declarations. A move would not resolve that —
    // it would only change which of the two the next plan wants to create — so
    // the run stops and names both ends. The estate has to drop one first.
    let conflicts = adopt::move_conflicts(&resolutions, &in_state);
    if !conflicts.is_empty() {
        let mut msg =
            String::from("adopt: the estate declares both ends of a state move; nothing was changed:\n");
        for (new_address, old_address) in &conflicts {
            msg.push_str(&format!(
                "  {} is the same live object as {}, which the estate still declares\n",
                new_address, old_address
            ));
        }
        msg.push_str("  drop one of the two declarations, then re-run adopt");
        return Err(msg.into());
    }

    if !execute {
        println!(
            "\ndry run — nothing was changed. Re-run with --execute to write the verified \"import-id\"s into the estate, \
             or --execute --import to run `{} import` now (derived ids are verified by the import itself).",
            runtime_config.tf_tool
        );
        return Ok(());
    }

    if import {
        let hcl_dir = Path::new(&runtime_config.hcl_dir);
        // E04: with no "import-id" in the estate every resolvable resource
        // counts as "to import", and a re-run then issued `tofu import` for
        // addresses the state already manages (17/18 once) — noisy, slow, and
        // each a needless state write. The read above already has them.
        // A FIRST adopt is fine: an initialized empty state lists nothing and
        // errors nothing. An UNREADABLE state (uninitialized dir, changed
        // backend) means every import below would fail the same way — so this
        // fails fast with the fix instead of printing it 117 times. The dry run
        // above only noted it, because describing an estate needs no state.
        let in_state = state.map_err(|e| {
            format!(
                "could not read the state ({}) — the imports would fail the same way; run `{} init` \
                 (or `init -reconfigure` after a backend change) in {} first",
                e.lines().next().unwrap_or("(no output)"),
                runtime_config.tf_tool,
                runtime_config.hcl_dir
            )
        })?;
        // activation posts the DECLARED spec — parameterized managed
        // constraints (allowedContactDomains, allowedPolicyMembers) require
        // their `parameters` and reject a synthesized enforce-only rule
        let declared_specs: std::collections::BTreeMap<String, serde_yaml::Value> = out
            .org_policies
            .iter()
            .filter_map(|(_, body)| {
                let name = body.get("name")?.as_str()?;
                let spec = body.get("spec")?.clone();
                Some((crate::org_policy::constraint_name(name), spec))
            })
            .collect();
        let (mut activated, mut imported, mut failed) = (0usize, 0usize, 0usize);
        let (mut already_managed, mut moved, mut on_apply) = (0usize, 0usize, 0usize);
        for r in &resolutions {
            if in_state.manages(&r.address) {
                println!("  {:60} already managed in the state — skipped", r.address);
                already_managed += 1;
                continue;
            }
            // Before the outcome is read: the outcome says IMPORT, and for a
            // renamed block importing is what puts one live object in the state
            // twice. The object is already managed — only its name changed.
            if let Some(old_address) = adopt::moved_from(r, &in_state) {
                if crate::bootstrap::run_state_mv(
                    &runtime_config.tf_tool,
                    hcl_dir,
                    old_address,
                    &r.address,
                ) {
                    moved += 1;
                } else {
                    failed += 1;
                }
                continue;
            }
            let id = match &r.outcome {
                Outcome::NeedsActivation { id, .. } => {
                    let Some((parent, constraint)) = &r.org_policy else { continue };
                    println!("  {:60} activating (managed, not live)...", r.address);
                    let spec = match declared_specs
                        .get(constraint)
                        .ok_or_else(|| format!("{} declares no spec", constraint))
                        .and_then(crate::org_policy::declared_spec_to_api)
                    {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("  {:60} activation FAILED: {}", r.address, e);
                            failed += 1;
                            continue;
                        }
                    };
                    let client = live.org_policy_client().await?;
                    match client.create_policy(parent, constraint, spec).await {
                        Ok(()) => activated += 1,
                        Err(e) => {
                            eprintln!("  {:60} activation FAILED: {}", r.address, e);
                            failed += 1;
                            continue;
                        }
                    }
                    id
                }
                Outcome::Resolved { id, .. } => id,
                Outcome::OnApply => {
                    println!("  {:60} on apply — skipped (apply creates it)", r.address);
                    on_apply += 1;
                    continue;
                }
                Outcome::ParentOnApply(why) => {
                    println!("  {:60} on apply — skipped ({})", r.address, why);
                    on_apply += 1;
                    continue;
                }
                Outcome::AlreadyAdopted(_) | Outcome::Skipped => continue,
                other => {
                    // unanswered rows already ended the run above; this arm
                    // only exists so a new Outcome can never be skipped silently
                    println!("  {:60} skipped ({:?})", r.address, other);
                    continue;
                }
            };
            if crate::bootstrap::run_import(&runtime_config.tf_tool, hcl_dir, &r.address, id) {
                imported += 1;
            } else {
                failed += 1;
            }
        }
        println!(
            "\nadopt: {} activated, {} imported, {} moved, {} already managed (skipped), {} on apply (skipped — apply creates them), {} failed.\nnext: `satz transpile {} --plan` — no create for what was imported and no destroy for what was moved.",
            activated, imported, moved, already_managed, on_apply, failed, input
        );
        if failed > 0 {
            return Err(format!("adopt: {} activation(s)/import(s)/move(s) failed — see above", failed).into());
        }
        // Every declared resource answered and nothing failed: the notices that ask for
        // this run are done, and the estate says so.
        if whole {
            let done: Vec<String> = crate::notices::open(&input_path, runtime_config)?
                .into_iter()
                .filter(|n| n.run.split_whitespace().take(2).eq(["satz", "adopt"]))
                .map(|n| n.param)
                .collect();
            crate::notices::acknowledge(&input_path, &done)?;
            for p in &done {
                println!("adopt: acknowledged the notice {} — bound {} = true in {}", p, p, input_path.display());
            }
        }
    } else {
        let (written, hints) = adopt::write_import_ids(&resolutions, Some(Path::new(&runtime_config.presets_dir)))?;
        for w in &written {
            println!("  wrote {}", w);
        }
        for h in &hints {
            println!("  note: {}", h);
        }
        let pending_activation = resolutions.iter().filter(|r| matches!(r.outcome, Outcome::NeedsActivation { .. })).count();
        if pending_activation > 0 {
            println!("  note: {} managed constraint(s) need activation — that is `--execute --import --activate`, activation cannot be written into the estate", pending_activation);
        }
        println!(
            "\nadopt: {} \"import-id\"(s) written. Run `satz transpile {}` to regenerate imports.tf, then `satz plan`.",
            written.len(),
            input
        );
    }
    Ok(())
}
