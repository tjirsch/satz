//! The emission manifest: what the emitter emitted, as structure.
//!
//! Every consumer that needs to know "which resources does this estate emit, and
//! with which attributes" — the compliance plane's witness matching, org-policy
//! adoption — used to re-parse the rendered `main.tf` with its own line scanner.
//! Four scanners, four opinions about HCL, and one shared blind spot: the raw
//! `hcl { … }` passthrough is appended to `main.tf` after emission, so a text
//! scan counted resources the proof layer is documented as unable to see.
//!
//! The manifest is built from the `hcl::Block`s the emitter constructs, before
//! they are rendered. Same compile, same blocks, no text — and the passthrough
//! never enters it, because it never was a block.

use std::collections::{BTreeMap, BTreeSet};

/// One emitted `resource` block, reduced to what the consumers match on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmittedResource {
    pub tf_type: String,
    pub label: String,
    /// Top-level attributes whose value is a plain or template string — the
    /// identifiers witnesses are matched on (`name`, `display_name`, `parent`).
    /// Nested blocks are deliberately not flattened in: an alert policy's
    /// `conditions { display_name }` must not shadow the resource's own.
    pub attrs: BTreeMap<String, String>,
    /// Top-level attributes whose value is a traversal, rendered as text
    /// (`parent = google_folder.x.name` → `"google_folder.x.name"`). These are
    /// the references adoption follows to resolve a parent before a child.
    pub refs: BTreeMap<String, String>,
    /// String attributes of nested blocks, dotted (`group_key.id`,
    /// `preferred_member_key.id`); first occurrence wins. Adoption's natural
    /// keys for groups and memberships live one level down.
    pub nested: BTreeMap<String, String>,
    /// Every value a repeated nested attribute carries, in emission order
    /// (`audit_log_config.log_type` → ADMIN_READ, DATA_READ, DATA_WRITE).
    /// `nested` keeps the first for natural keys; a live check that compares a
    /// declared SET against the live one needs them all.
    pub nested_all: BTreeMap<String, Vec<String>>,
    /// The single `enforce` the block's `spec` declares, when it declares exactly
    /// one. Several, none, or a list constraint yield `None`: no verdict is better
    /// than a wrong one. A `dry_run_spec` is not read here — see `dry_run`.
    pub enforce: Option<bool>,
    /// An org policy whose `spec` declares `reset = true`: the constraint is
    /// declared off, back to Google's default.
    pub reset: bool,
    /// The policy declares a `dry_run_spec`: Google evaluates the rule, writes a
    /// violation to the audit log for every action it WOULD have blocked, and
    /// blocks nothing. Never a witness that a control is enforced.
    pub dry_run: bool,
    /// The `import { to id }` block emitted for this resource, if any — i.e.
    /// the estate already adopted it.
    pub import_id: Option<String>,
    /// Where the declaring block starts in the source: (file, 1-based line of
    /// the `label {` line). `None` for resources derived from another block
    /// (memberships, exploded grants, project services).
    pub origin: Option<(String, u32)>,
}

impl EmittedResource {
    pub fn address(&self) -> String {
        format!("{}.{}", self.tf_type, self.label)
    }
}

/// Every emitted resource, keyed by Terraform address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub resources: BTreeMap<String, EmittedResource>,
}

impl Manifest {
    /// From the blocks the emitter built. Non-`resource` blocks are ignored.
    pub fn from_blocks<'a>(blocks: impl IntoIterator<Item = &'a hcl::Block>) -> Self {
        let mut resources = BTreeMap::new();
        for b in blocks {
            if let Some(r) = resource_from_block(b) {
                resources.insert(r.address(), r);
            }
        }
        Manifest { resources }
    }

    /// Record the `import` blocks the emitter built beside the resources.
    pub fn attach_imports<'a>(&mut self, imports: impl IntoIterator<Item = &'a hcl::Block>) {
        for b in imports {
            if b.identifier() != "import" {
                continue;
            }
            let mut to = None;
            let mut id = None;
            for a in b.body().attributes() {
                match a.key() {
                    "to" => to = hcl::format::to_string(a.expr()).ok(),
                    "id" => id = string_value(a.expr()),
                    _ => {}
                }
            }
            if let (Some(to), Some(id)) = (to, id) {
                if let Some(r) = self.resources.get_mut(to.trim()) {
                    r.import_id = Some(id);
                }
            }
        }
    }

    /// Record where a resource was declared. Called by the emitter per fold
    /// entity for the block(s) that entity produced directly.
    pub fn set_origin(&mut self, address: &str, file: &str, line: u32) {
        if let Some(r) = self.resources.get_mut(address) {
            r.origin = Some((file.to_string(), line));
        }
    }

    /// The same reduction over `hcl::parse` output. Production goes through
    /// `from_blocks`; this exists so tests can state their fixtures as HCL
    /// text, exactly as the scanners' tests did.
    #[cfg(test)]
    pub fn parse(text: &str) -> Self {
        Self::from_blocks(hcl::parse(text).expect("fixture is valid HCL").blocks())
    }

    pub fn addresses(&self) -> BTreeSet<String> {
        self.resources.keys().cloned().collect()
    }

    /// Per address, the top-level string attributes (see `EmittedResource::attrs`).
    pub fn witness_attrs(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        self.resources.iter().map(|(a, r)| (a.clone(), r.attrs.clone())).collect()
    }

    /// Per address, the references it carries — placement traversals the walk
    /// emitted and whole-value `${…}` references the author wrote. Read by the
    /// gate that holds the manifest to what the old text scanners produced.
    #[cfg(test)]
    pub fn witness_refs(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        self.resources.iter().map(|(a, r)| (a.clone(), r.refs.clone())).collect()
    }

    /// Per org-policy address, the single `enforce` it declares.
    pub fn declared_enforcement(&self) -> BTreeMap<String, bool> {
        self.resources
            .values()
            .filter(|r| r.tf_type == "google_org_policy_policy")
            .filter_map(|r| r.enforce.map(|e| (r.address(), e)))
            .collect()
    }

    pub fn of_type<'a>(&'a self, tf_type: &'a str) -> impl Iterator<Item = &'a EmittedResource> + 'a {
        self.resources.values().filter(move |r| r.tf_type == tf_type)
    }
}

fn resource_from_block(b: &hcl::Block) -> Option<EmittedResource> {
    if b.identifier() != "resource" {
        return None;
    }
    let labels = b.labels();
    let (tf_type, label) = match labels {
        [t, l] => (t.as_str(), l.as_str()),
        _ => return None,
    };
    if tf_type.is_empty() || label.is_empty() {
        return None;
    }
    let mut attrs = BTreeMap::new();
    let mut refs = BTreeMap::new();
    for a in b.body().attributes() {
        // A value that is NOTHING BUT one interpolation is a reference, the
        // same as the bare traversal the walk emits for placement — it just
        // reached HCL through a Satz `${{…}}`. Consumers follow `refs`; left in
        // `attrs` it would be read as the literal text `${…}` and matched
        // against live state, which never matches (R9).
        if let Some(t) = whole_value_ref(a.expr()) {
            refs.insert(a.key().to_string(), t);
        } else if let Some(v) = string_value(a.expr()) {
            attrs.insert(a.key().to_string(), v);
        } else if let hcl::Expression::Traversal(_) = a.expr() {
            if let Ok(t) = hcl::format::to_string(a.expr()) {
                refs.insert(a.key().to_string(), t.trim().to_string());
            }
        }
    }
    let mut nested = BTreeMap::new();
    let mut nested_all: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for nb in b.body().blocks() {
        for a in nb.body().attributes() {
            if let Some(v) = string_value(a.expr()) {
                let key = format!("{}.{}", nb.identifier(), a.key());
                nested_all.entry(key.clone()).or_default().push(v.clone());
                nested.entry(key).or_insert(v);
            }
        }
    }
    // `enforce` is read from `spec` ALONE. A `dry_run_spec` carries the same
    // attribute and means the opposite: Google evaluates the rule, logs what it
    // would have blocked, and blocks nothing. Reading both reports a dry run as
    // enforcing, which is the one thing a dry run is not.
    let mut found = Vec::new();
    for spec in b.body().blocks().filter(|nb| nb.identifier() == "spec") {
        collect_enforce(spec.body(), &mut found);
    }
    let enforce = match found.as_slice() {
        [only] => Some(*only),
        _ => None,
    };
    let reset = b.body().blocks().filter(|nb| nb.identifier() == "spec").any(|spec| {
        spec.body().attributes().any(|a| a.key() == "reset" && matches!(a.expr(), hcl::Expression::Bool(true)))
    });
    let dry_run = b.body().blocks().any(|nb| nb.identifier() == "dry_run_spec");
    Some(EmittedResource {
        tf_type: tf_type.to_string(),
        label: label.to_string(),
        attrs,
        refs,
        nested,
        nested_all,
        enforce,
        reset,
        dry_run,
        import_id: None,
        origin: None,
    })
}

/// True when a manifest value still carries an interpolation, i.e. it is not a
/// literal and must not be matched against live state as one. An EMBEDDED
/// reference (`"${google_folder.x.name}/policies/…"`) stays in `attrs` because
/// consumers read a fixed part of it — but never the whole.
pub(crate) fn has_interpolation(s: &str) -> bool {
    s.contains("${")
}

/// The traversal of a value that is exactly one interpolation and nothing else
/// (`"${google_project.x.project_id}"` -> `google_project.x.project_id`).
/// Anything with literal text around it is not a reference to a resource, it is
/// a string that mentions one.
fn whole_value_ref(expr: &hcl::Expression) -> Option<String> {
    let hcl::Expression::TemplateExpr(_) = expr else { return None };
    let rendered = hcl::format::to_string(expr).ok()?;
    let inner = rendered.trim().trim_matches('"');
    let inner = inner.strip_prefix("${")?.strip_suffix('}')?.trim();
    if inner.is_empty() || inner.contains("${") {
        return None;
    }
    // a dotted path of identifiers - not a function call, index or operator
    let shaped = inner.contains('.')
        && !inner.starts_with('.')
        && !inner.ends_with('.')
        && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
    shaped.then(|| inner.to_string())
}

/// A plain string, or a template string rendered back to its `${…}` text —
/// the two shapes an identifier attribute takes (an org policy under a folder
/// emits `name = "${google_folder.x.name}/policies/…"`).
fn string_value(expr: &hcl::Expression) -> Option<String> {
    match expr {
        hcl::Expression::String(s) => Some(s.clone()),
        hcl::Expression::TemplateExpr(_) => {
            let rendered = hcl::format::to_string(expr).ok()?;
            Some(rendered.trim_matches('"').to_string())
        }
        _ => None,
    }
}

/// Every `enforce = "TRUE"|"FALSE"` at any depth of the body, in document order.
fn collect_enforce(body: &hcl::Body, out: &mut Vec<bool>) {
    for a in body.attributes() {
        if a.key() == "enforce" {
            if let Some(v) = string_value(a.expr()) {
                match v.to_ascii_uppercase().as_str() {
                    "TRUE" => out.push(true),
                    "FALSE" => out.push(false),
                    _ => {}
                }
            }
        }
    }
    for b in body.blocks() {
        collect_enforce(b.body(), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body carrying both a `spec` and a `dry_run_spec` used to collapse to no
    /// verdict at all, because `collect_enforce` walked the whole body and found two
    /// `enforce` attributes. Worse, a dry-run-ONLY policy read as enforcing. Both are
    /// how a policy that blocks nothing would satisfy a claim that says it does.
    #[test]
    fn enforce_is_read_from_spec_and_never_from_a_dry_run() {
        let one = |hcl_src: &str| {
            let body: hcl::Body = hcl::from_str(hcl_src).expect("parses");
            let b = body.blocks().next().expect("one block").clone();
            resource_from_block(&b).expect("a resource")
        };

        let dry = one(r#"
resource "google_org_policy_policy" "p" {
  dry_run_spec {
    rules { enforce = "TRUE" }
  }
}
"#);
        assert_eq!(dry.enforce, None, "a dry run declares no enforcement");
        assert!(dry.dry_run);

        let both = one(r#"
resource "google_org_policy_policy" "p" {
  spec {
    rules { enforce = "TRUE" }
  }
  dry_run_spec {
    rules { enforce = "FALSE" }
  }
}
"#);
        assert_eq!(both.enforce, Some(true), "the spec decides; the dry run is not a second opinion");
        assert!(both.dry_run);

        let plain = one(r#"
resource "google_org_policy_policy" "p" {
  spec {
    rules { enforce = "FALSE" }
  }
}
"#);
        assert_eq!(plain.enforce, Some(false));
        assert!(!plain.dry_run);
    }

    #[test]
    fn addresses_come_from_resource_blocks_only() {
        let m = Manifest::parse(
            r#"
resource "google_logging_metric" "cis_central_2_5_project_ownership" {
  name = "x"
}
resource "google_storage_bucket" "org_audit_logs" {}
import {
  to = google_storage_bucket.org_audit_logs
  id = "org-audit-logs"
}
"#,
        );
        let set = m.addresses();
        assert!(set.contains("google_logging_metric.cis_central_2_5_project_ownership"));
        assert!(set.contains("google_storage_bucket.org_audit_logs"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn a_whole_value_interpolation_is_a_reference_an_embedded_one_is_not() {
        let m = Manifest::parse(
            r#"
resource "google_service_account_iam_member" "assignment" {
  service_account_id = "${google_service_account.onboarding.name}"
  role = "roles/iam.workloadIdentityUser"
  member = "principalSet://iam.googleapis.com/projects/${google_project.mgmt.number}/locations/global/workloadIdentityPools/p/*"
}

resource "google_org_policy_policy" "under_a_folder" {
  name = "${google_folder.workloads.name}/policies/compute.skipDefaultNetworkCreation"
  parent = google_folder.workloads.name
}
"#,
        );
        let r = &m.resources["google_service_account_iam_member.assignment"];
        // nothing but the interpolation: a reference the consumers can follow
        assert_eq!(r.refs.get("service_account_id").map(String::as_str), Some("google_service_account.onboarding.name"));
        assert!(!r.attrs.contains_key("service_account_id"), "a reference must not also be a literal");
        // a plain literal stays a literal
        assert_eq!(r.attrs.get("role").map(String::as_str), Some("roles/iam.workloadIdentityUser"));
        // literal text around the interpolation: not a reference to a resource,
        // a string that mentions one — it stays in attrs, and is flagged
        let member = r.attrs.get("member").expect("an embedded reference stays an attr");
        assert!(has_interpolation(member));
        assert!(!r.refs.contains_key("member"));
        // the org-policy name is the load-bearing embedded case: consumers read
        // the part after /policies/, so it must not move
        let policy = &m.resources["google_org_policy_policy.under_a_folder"];
        assert!(policy.attrs.get("name").is_some_and(|n| n.ends_with("/policies/compute.skipDefaultNetworkCreation")));
        assert_eq!(policy.refs.get("parent").map(String::as_str), Some("google_folder.workloads.name"));
    }

    #[test]
    fn an_interpolation_that_is_not_a_plain_traversal_stays_an_attr() {
        let m = Manifest::parse(
            r#"
resource "google_storage_bucket" "b" {
  name = "${lower(var.x)}"
  location = "${google_project.a.b[0]}"
}
"#,
        );
        let r = &m.resources["google_storage_bucket.b"];
        for k in ["name", "location"] {
            assert!(!r.refs.contains_key(k), "{} is not a resource reference", k);
            assert!(r.attrs.contains_key(k), "{} must still be recorded", k);
        }
    }

    #[test]
    fn witness_attrs_ignore_nested_block_shadowing() {
        let m = Manifest::parse(
            r#"
resource "google_monitoring_alert_policy" "cis_central_2_8_firewall_rule" {
  display_name = "CIS 2.8 — VPC firewall rule changes (org-wide)"
  combiner = "OR"
  provider = google.google
  conditions {
    display_name = "Firewall rule changed"
    condition_matched_log {
      filter = "x"
    }
  }
}
"#,
        );
        let attrs = m.witness_attrs();
        let policy = &attrs["google_monitoring_alert_policy.cis_central_2_8_firewall_rule"];
        assert_eq!(
            policy.get("display_name").map(String::as_str),
            Some("CIS 2.8 — VPC firewall rule changes (org-wide)")
        );
        // a traversal is not an identifier string
        assert_eq!(policy.get("provider"), None);
    }

    /// `enforce` sits two levels down in `spec { rules { … } }`; a list
    /// constraint has no single boolean and must be absent rather than guessed.
    #[test]
    fn declared_enforcement_reads_nested_enforce_per_policy() {
        let m = Manifest::parse(
            r#"
resource "google_org_policy_policy" "on" {
  name = "organizations/1/policies/compute.managed.requireOsLogin"
  spec {
    rules {
      enforce = "TRUE"
    }
  }
}
resource "google_org_policy_policy" "off" {
  name = "organizations/1/policies/compute.managed.vmCanIpForward"
  spec {
    rules {
      enforce = "FALSE"
    }
  }
}
resource "google_org_policy_policy" "listy" {
  name = "organizations/1/policies/iam.allowedPolicyMemberDomains"
  spec {
    rules {
      values {
        allowed_values = ["C0example"]
      }
    }
  }
}
resource "google_org_policy_policy" "many" {
  name = "organizations/1/policies/x.y"
  spec {
    rules {
      enforce = "TRUE"
    }
    rules {
      enforce = "FALSE"
    }
  }
}
"#,
        );
        let d = m.declared_enforcement();
        assert_eq!(d.get("google_org_policy_policy.on"), Some(&true));
        assert_eq!(d.get("google_org_policy_policy.off"), Some(&false));
        assert_eq!(d.get("google_org_policy_policy.listy"), None);
        assert_eq!(d.get("google_org_policy_policy.many"), None);
        assert_eq!(d.len(), 2);
    }

    /// A folder-scoped policy's name is a template; the consumer needs its text.
    #[test]
    fn template_names_render_to_their_text() {
        let m = Manifest::parse(
            r#"
resource "google_org_policy_policy" "p" {
  name = "${google_folder.x.name}/policies/compute.managed.requireOsLogin"
  parent = google_folder.x.name
}
"#,
        );
        let r = &m.resources["google_org_policy_policy.p"];
        assert_eq!(
            r.attrs.get("name").map(String::as_str),
            Some("${google_folder.x.name}/policies/compute.managed.requireOsLogin")
        );
        // a traversal is not a string attr — it is a reference
        assert_eq!(r.attrs.get("parent"), None);
        assert_eq!(r.refs.get("parent").map(String::as_str), Some("google_folder.x.name"));
    }

    #[test]
    fn imports_attach_to_their_resource() {
        let body = hcl::parse(
            r#"
resource "google_folder" "x" {
  display_name = "X"
}
import {
  to = google_folder.x
  id = "folders/123"
}
"#,
        )
        .unwrap();
        let mut m = Manifest::from_blocks(body.blocks());
        m.attach_imports(body.blocks());
        assert_eq!(m.resources["google_folder.x"].import_id.as_deref(), Some("folders/123"));
    }
}
