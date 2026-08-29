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
    /// The single `enforce` the block declares anywhere in its body, when it
    /// declares exactly one. Several, none, or a list constraint yield `None`:
    /// no verdict is better than a wrong one.
    pub enforce: Option<bool>,
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
    let attrs = b
        .body()
        .attributes()
        .filter_map(|a| string_value(a.expr()).map(|v| (a.key().to_string(), v)))
        .collect();
    let mut found = Vec::new();
    collect_enforce(b.body(), &mut found);
    let enforce = match found.as_slice() {
        [only] => Some(*only),
        _ => None,
    };
    Some(EmittedResource { tf_type: tf_type.to_string(), label: label.to_string(), attrs, enforce })
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
        assert_eq!(r.attrs.get("parent"), None);
    }
}
