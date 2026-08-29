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
    async fn group(&mut self, email: &str) -> Result<Option<String>, String>;
    async fn membership(&mut self, group_name: &str, email: &str) -> Result<Option<String>, String>;
    async fn org_policy_exists(&mut self, parent: &str, constraint: &str) -> Result<bool, String>;
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
    Match(Vec<String>),
    None,
}

fn rule_for(rules: &ImportConfig, tf_type: &str) -> Rule {
    match rules.resource_types.get(tf_type) {
        Some(r) if r.import_id.is_some() => Rule::Template(r.import_id.clone().unwrap()),
        Some(r) if r.match_on.is_some() => Rule::Match(r.match_on.clone().unwrap()),
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
        let outcome = match r.tf_type.as_str() {
            "google_folder" => resolve_folder(r, manifest, &resolved_ids, live).await,
            "google_cloud_identity_group" => resolve_group(r, live).await,
            "google_cloud_identity_group_membership" => resolve_membership(r, &resolved_ids, live).await,
            "google_org_policy_policy" => resolve_org_policy(r, manifest, &resolved_ids, opts, live, &mut res).await,
            _ => match rule_for(rules, &r.tf_type) {
                Rule::Template(t) => render_template(&t, r, manifest, &resolved_ids),
                Rule::Match(on) => (
                    on.iter().filter_map(|k| r.attrs.get(k).cloned()).collect::<Vec<_>>().join(" / "),
                    Outcome::NeedsLookup(format!("match on {} via Cloud Asset Inventory is not supported yet", on.join(", "))),
                ),
                Rule::None => (String::new(), Outcome::NoRule),
            },
        };
        res.natural_key = outcome.0;
        res.outcome = outcome.1;
        if let Outcome::Resolved { id, .. } | Outcome::NeedsActivation { id, .. } = &res.outcome {
            resolved_ids.insert(r.address(), id.clone());
        }
        out.push(res);
    }
    out
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
    let rest = manifest.resources.values().filter(|r| r.tf_type != "google_folder");
    folders.into_iter().chain(rest).collect()
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
    let parent = match value_of(r, "parent", manifest, resolved_ids) {
        Ok(p) => crate::org_policy::normalize_parent(&p),
        Err(_) => match name.rsplit_once("/policies/") {
            Some((p, _)) if !p.contains("${") => p.to_string(),
            _ => return (constraint, Outcome::Unresolvable(format!("{} has no resolvable parent", r.address()))),
        },
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
        "adopt: {} to import ({} verified live, {} derived), {} need activation, {} already adopted, {} on apply, {} ambiguous, {} without a rule, {} unresolvable, {} failed",
        count(&|o| matches!(o, Outcome::Resolved { .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: true, .. })),
        count(&|o| matches!(o, Outcome::Resolved { verified: false, .. })),
        count(&|o| matches!(o, Outcome::NeedsActivation { .. })),
        count(&|o| matches!(o, Outcome::AlreadyAdopted(_))),
        count(&|o| matches!(o, Outcome::OnApply)),
        count(&|o| matches!(o, Outcome::Ambiguous(_))),
        count(&|o| matches!(o, Outcome::NoRule)),
        count(&|o| matches!(o, Outcome::Unresolvable(_))),
        count(&|o| matches!(o, Outcome::Failed(_))),
    )
}

/// Insert `"import-id" = "<id>"` into the source file right after the
/// declaring `label {` line, for every VERIFIED resolution that has an origin.
/// Derived (unverified) ids are not written: an `import` block for an object
/// that does not exist fails the whole `tofu plan`, so those go through
/// `--import`, which verifies per resource. Returns (written, not written) —
/// the latter with the exact snippet to add by hand.
pub(crate) fn write_import_ids(resolutions: &[Resolution]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut by_file: BTreeMap<String, Vec<(u32, String, String)>> = BTreeMap::new();
    let mut hints = Vec::new();
    for r in resolutions {
        let id = match &r.outcome {
            Outcome::Resolved { id, verified: true } => id,
            _ => continue,
        };
        match &r.origin {
            Some((file, line)) => by_file.entry(file.clone()).or_default().push((*line, r.address.clone(), id.clone())),
            None => hints.push(format!(
                "{}: add \"import-id\" = \"{}\" to its entry (derived resource — object form, see language reference §6.7)",
                r.address, id
            )),
        }
    }
    let mut written = Vec::new();
    for (file, mut edits) in by_file {
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {}", file, e))?;
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        // bottom-up so earlier line numbers stay valid
        edits.sort_by(|a, b| b.0.cmp(&a.0));
        for (line, address, id) in edits {
            let idx = line as usize - 1;
            let Some(decl) = lines.get(idx) else {
                hints.push(format!("{}: {}:{} is past the end of the file", address, file, line));
                continue;
            };
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
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(&file, out).map_err(|e| format!("{}: {}", file, e))?;
    }
    Ok((written, hints))
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

    async fn org_policy_exists(&mut self, parent: &str, constraint: &str) -> Result<bool, String> {
        if !self.policies.contains_key(parent) {
            let client = self.org_policy_client().await?;
            let current = crate::org_policy::fetch_current(client, parent).await.map_err(|e| e.to_string())?;
            self.policies.insert(parent.to_string(), current);
        }
        Ok(self.policies[parent].contains_key(constraint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ImportResourceConfig;

    struct Fake {
        folders: BTreeMap<(String, String), Lookup>,
        groups: BTreeMap<String, String>,
        memberships: BTreeMap<(String, String), String>,
        policies: BTreeSet<(String, String)>,
        calls: Vec<String>,
    }

    impl Live for Fake {
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
                    asset_type: None,
                    content_type: None,
                    exclude: None,
                    include: None,
                    derive_yaml_key_from: None,
                    deprecated: None,
                    import_id: template.map(|s| s.to_string()),
                    match_on: on.map(|v| v.iter().map(|s| s.to_string()).collect()),
                    activate: None,
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
        let mut f = Fake { folders: BTreeMap::new(), groups: BTreeMap::new(), memberships: BTreeMap::new(), policies: BTreeSet::new(), calls: vec![] };
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
        assert_eq!(outcome(&rs, "google_project.infra"), &Outcome::Resolved { id: "acme-infra-001".into(), verified: false });
        assert_eq!(outcome(&rs, "google_storage_bucket.b"), &Outcome::AlreadyAdopted("acme-state".into()));
        assert!(matches!(outcome(&rs, "google_monitoring_alert_policy.alert"), Outcome::NeedsLookup(_)));
        assert_eq!(outcome(&rs, "google_widget.w"), &Outcome::NoRule);
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
            assert!(matches!(rule_for(&cfg, t), Rule::Match(_)), "{} needs a match_on rule", t);
        }
        assert_eq!(cfg.resource_types["google_org_policy_policy"].activate.as_deref(), Some("managed"));
    }

    #[test]
    fn write_inserts_after_the_declaring_line_and_skips_the_unverified() {
        let tmp = std::env::temp_dir().join("satz-adopt-write.satz");
        std::fs::write(&tmp, "google_folder {\n  workloads {\n    display_name = \"Workloads\"\n  }\n  one { display_name = \"x\" }\n}\n").unwrap();
        let file = tmp.to_string_lossy().to_string();
        let rs = vec![
            Resolution { address: "google_folder.workloads".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111".into(), verified: true }, origin: Some((file.clone(), 2)), org_policy: None },
            Resolution { address: "google_folder.one".into(), tf_type: "google_folder".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/1".into(), verified: true }, origin: Some((file.clone(), 5)), org_policy: None },
            Resolution { address: "google_project.p".into(), tf_type: "google_project".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "p".into(), verified: false }, origin: Some((file.clone(), 2)), org_policy: None },
            Resolution { address: "google_folder_iam_member.g".into(), tf_type: "google_folder_iam_member".into(), natural_key: String::new(), outcome: Outcome::Resolved { id: "folders/111 r m".into(), verified: true }, origin: None, org_policy: None },
        ];
        let (written, hints) = write_import_ids(&rs).unwrap();
        let text = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(text, "google_folder {\n  workloads {\n    \"import-id\" = \"folders/111\"\n    display_name = \"Workloads\"\n  }\n  one { display_name = \"x\" }\n}\n");
        assert_eq!(written.len(), 1);
        assert_eq!(hints.len(), 2, "{:?}", hints);
        assert!(hints.iter().any(|h| h.contains("google_folder.one") && h.contains("by hand")));
        assert!(hints.iter().any(|h| h.contains("google_folder_iam_member.g") && h.contains("object form")));
        let _ = std::fs::remove_file(&tmp);
    }
}
