//! The two grants a new organisation carries by default — `roles/resourcemanager.projectCreator`
//! and `roles/billing.creator` on `domain:<customer domain>`, every user in the directory —
//! removed by `apply`, once every holder the estate declares for that role on the organisation
//! is live.
//!
//! Terraform has no resource that says a member is absent, and an authoritative binding would
//! replace the role's whole member list — every grant a customer made by hand, every holder an
//! organisation that pre-dates satz already carries (ADR 0081). So this step knows exactly one
//! member on exactly two roles and touches nothing else: it reads the organisation's IAM policy,
//! takes the domain out of the unconditional binding of each role whose declared holders are all
//! in it, writes the policy back under its etag, and prints each removal with the command that
//! restores it. `plan` reads and says what `apply` would do; only `apply` writes. A role the
//! estate declares no holder for is left alone, a domain entry held under a condition is reported
//! and not touched, and a policy that cannot be read fails the command — an unreadable policy is
//! never an empty one.

use std::collections::BTreeMap;

use crate::gcp::{same_member, ErrorClass};
use crate::manifest::Manifest;

/// The roles a new organisation grants to its whole domain.
pub(crate) const ROLES: [&str; 2] = ["roles/resourcemanager.projectCreator", "roles/billing.creator"];

/// The unconditional holders the estate declares on the organisation, per role of `ROLES`
/// that has any. `org_id` is the organisation's number, as `main.tf` writes it; a grant
/// whose `org_id` is a literal naming another organisation is not one of these.
pub(crate) fn declared_holders(manifest: &Manifest, org_id: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in manifest.of_type("google_organization_iam_member") {
        if r.attrs.get("org_id").is_some_and(|o| o != org_id) || r.nested.contains_key("condition.expression") {
            continue;
        }
        let (Some(role), Some(member)) = (r.attrs.get("role"), r.attrs.get("member")) else { continue };
        if ROLES.contains(&role.as_str()) {
            out.entry(role.clone()).or_default().push(member.clone());
        }
    }
    out
}

/// What the organisation's policy says about one role, read against the declared holders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finding {
    /// the domain does not hold the role: nothing to do
    Clean { role: String },
    /// the estate itself declares the domain a holder of the role: kept, whatever is live
    Declared { role: String },
    /// the domain holds it and every declared holder is live: `apply` removes the domain's entry
    Removable { role: String },
    /// the domain holds it, and these declared holders are not live yet: the apply grants them
    NotYetLive { role: String, missing: Vec<String> },
    /// the domain holds it under a condition only, `title: expression` each: reported, not touched
    Conditional { role: String, conditions: Vec<String> },
}

impl Finding {
    fn role(&self) -> &str {
        match self {
            Finding::Clean { role }
            | Finding::Declared { role }
            | Finding::Removable { role }
            | Finding::NotYetLive { role, .. }
            | Finding::Conditional { role, .. } => role,
        }
    }
}

/// Judge one role of `policy` (an IAM policy, version 3): whether `domain` holds it, and
/// whether every member of `declared` holds it unconditionally beside the domain.
pub(crate) fn judge(policy: &serde_json::Value, role: &str, domain: &str, declared: &[String]) -> Finding {
    let role_s = role.to_string();
    if declared.iter().any(|d| same_member(d, domain)) {
        return Finding::Declared { role: role_s };
    }
    let bindings: Vec<&serde_json::Value> = policy
        .get("bindings")
        .and_then(|b| b.as_array())
        .into_iter()
        .flatten()
        .filter(|b| b.get("role").and_then(|r| r.as_str()) == Some(role))
        .collect();
    let members = |b: &serde_json::Value| -> Vec<String> {
        b.get("members")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect()
    };
    let holds = |b: &serde_json::Value| members(b).iter().any(|m| same_member(m, domain));
    if let Some(unconditional) = bindings.iter().find(|b| b.get("condition").is_none() && holds(b)) {
        let live = members(unconditional);
        let missing: Vec<String> = declared.iter().filter(|d| !live.iter().any(|m| same_member(m, d))).cloned().collect();
        return if missing.is_empty() { Finding::Removable { role: role_s } } else { Finding::NotYetLive { role: role_s, missing } };
    }
    let text = |c: &serde_json::Value, k: &str| c.get(k).and_then(|s| s.as_str()).unwrap_or("").to_string();
    let conditions: Vec<String> = bindings
        .iter()
        .filter(|b| holds(b))
        .filter_map(|b| b.get("condition"))
        .map(|c| format!("{}: {}", text(c, "title"), text(c, "expression")))
        .collect();
    if conditions.is_empty() {
        Finding::Clean { role: role_s }
    } else {
        Finding::Conditional { role: role_s, conditions }
    }
}

/// What one run read and did, one finding per declared role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    /// the organisation's number
    pub org_id: String,
    /// `domain:<customer domain>`
    pub domain: String,
    /// `apply` (the removable entries were removed) or `plan` (nothing was written)
    pub wrote: bool,
    pub findings: Vec<Finding>,
}

/// The command that puts a removed entry back.
pub(crate) fn restore_command(org_id: &str, domain: &str, role: &str) -> String {
    format!("gcloud organizations add-iam-policy-binding {} --member={} --role={}", org_id, domain, role)
}

impl Report {
    /// One line for the organisation, then one per declared role saying what was found and done.
    pub fn render(&self) -> String {
        let mut out = format!("default grants on organizations/{}:\n", self.org_id);
        for f in &self.findings {
            let line = match f {
                Finding::Clean { role } => format!("{} does not hold {}", self.domain, role),
                Finding::Declared { role } => format!("kept: the estate declares {} a holder of {}", self.domain, role),
                Finding::Removable { role } if self.wrote => {
                    format!("removed {} from {} — restore with: {}", self.domain, role, restore_command(&self.org_id, &self.domain, role))
                }
                Finding::Removable { role } => format!("apply would remove {} from {} — plan changes nothing", self.domain, role),
                Finding::NotYetLive { role, missing } => format!(
                    "left in place: {} holds {} — {} declared and not live yet; the apply grants {} and the next apply removes the domain's entry",
                    self.domain,
                    role,
                    missing.join(", "),
                    if missing.len() == 1 { "it" } else { "them" }
                ),
                Finding::Conditional { role, conditions } => format!(
                    "left in place: {} holds {} under a condition only ({}) — satz removes no conditional grant",
                    self.domain,
                    role,
                    conditions.join("; ")
                ),
            };
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

/// Read the organisation's policy and judge each declared role; on `write`, take the domain
/// out of every removable role in one `setIamPolicy` under the policy's etag, re-reading and
/// re-judging on a concurrent change, three attempts.
pub(crate) async fn reconcile(
    client: &reqwest::Client,
    token: &str,
    org_id: &str,
    customer_domain: &str,
    declared: &BTreeMap<String, Vec<String>>,
    write: bool,
) -> Result<Report, String> {
    let resource = format!("organizations/{}", org_id);
    let domain = format!("domain:{}", customer_domain);
    let mut attempts = 0;
    loop {
        attempts += 1;
        let mut policy = crate::gcp::resourcemanager::get_iam_policy(client, token, &resource)
            .await
            .map_err(|e| format!("could not read the IAM policy of {}: {}", resource, e))?;
        let findings: Vec<Finding> = declared.iter().map(|(role, holders)| judge(&policy, role, &domain, holders)).collect();
        let removable: Vec<&str> = findings.iter().filter(|f| matches!(f, Finding::Removable { .. })).map(Finding::role).collect();
        if !write || removable.is_empty() {
            return Ok(Report { org_id: org_id.to_string(), domain, wrote: false, findings });
        }
        for role in &removable {
            crate::gcp::remove_member(&mut policy, role, &domain);
        }
        match crate::gcp::resourcemanager::set_iam_policy(client, token, &resource, &policy).await {
            Ok(()) => return Ok(Report { org_id: org_id.to_string(), domain, wrote: true, findings }),
            // A concurrent IAM change invalidated the etag: re-read, re-judge, retry.
            Err(e) if e.class() == ErrorClass::Conflict && attempts < 3 => continue,
            Err(e) => return Err(format!("could not remove {} from {} on {}: {}", domain, removable.join(" and "), resource, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PC: &str = "roles/resourcemanager.projectCreator";
    const BC: &str = "roles/billing.creator";
    const DOMAIN: &str = "domain:example.com";

    fn declared(members: &[&str]) -> Vec<String> {
        members.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn a_role_every_declared_holder_holds_beside_the_domain_is_removable() {
        let policy = serde_json::json!({ "bindings": [
            { "role": PC, "members": ["domain:Example.com", "group:gcp-organization-admins@example.com", "user:admin@example.com", "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com"] },
        ]});
        let holders = declared(&["group:gcp-organization-admins@example.com", "user:admin@example.com", "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com"]);
        assert_eq!(judge(&policy, PC, DOMAIN, &holders), Finding::Removable { role: PC.into() });
    }

    #[test]
    fn a_declared_holder_the_policy_lacks_leaves_the_domain_in_place_and_is_named() {
        let policy = serde_json::json!({ "bindings": [
            { "role": PC, "members": ["domain:example.com", "group:gcp-organization-admins@example.com"] },
        ]});
        let holders = declared(&["group:gcp-organization-admins@example.com", "user:admin@example.com"]);
        assert_eq!(judge(&policy, PC, DOMAIN, &holders), Finding::NotYetLive { role: PC.into(), missing: vec!["user:admin@example.com".into()] });
    }

    #[test]
    fn a_domain_the_estate_itself_declares_a_holder_is_kept() {
        let policy = serde_json::json!({ "bindings": [
            { "role": BC, "members": ["domain:example.com", "group:gcp-billing-admins@example.com"] },
        ]});
        let holders = declared(&["group:gcp-billing-admins@example.com", "domain:Example.com"]);
        assert_eq!(judge(&policy, BC, DOMAIN, &holders), Finding::Declared { role: BC.into() });
        let report = Report { org_id: "123456789012".into(), domain: DOMAIN.into(), wrote: true, findings: vec![Finding::Declared { role: BC.into() }] };
        assert!(report.render().contains("  kept: the estate declares domain:example.com a holder of roles/billing.creator\n"), "{}", report.render());
    }

    #[test]
    fn a_domain_that_does_not_hold_the_role_is_clean() {
        let policy = serde_json::json!({ "bindings": [
            { "role": PC, "members": ["group:gcp-organization-admins@example.com"] },
            { "role": "roles/viewer", "members": ["domain:example.com"] },
        ]});
        assert_eq!(judge(&policy, PC, DOMAIN, &declared(&["group:gcp-organization-admins@example.com"])), Finding::Clean { role: PC.into() });
        assert_eq!(judge(&serde_json::json!({ "etag": "x" }), BC, DOMAIN, &declared(&["user:admin@example.com"])), Finding::Clean { role: BC.into() });
    }

    #[test]
    fn a_domain_entry_held_under_a_condition_only_is_reported_not_removable() {
        let policy = serde_json::json!({ "bindings": [
            { "role": BC, "members": ["domain:example.com"], "condition": { "title": "until-march", "expression": "request.time < timestamp('2027-03-01T00:00:00Z')" } },
        ]});
        assert_eq!(
            judge(&policy, BC, DOMAIN, &declared(&["group:gcp-billing-admins@example.com"])),
            Finding::Conditional { role: BC.into(), conditions: vec!["until-march: request.time < timestamp('2027-03-01T00:00:00Z')".into()] }
        );
    }

    #[test]
    fn the_report_says_what_was_done_per_role_and_how_to_undo_it() {
        let findings = vec![
            Finding::Removable { role: PC.into() },
            Finding::NotYetLive { role: BC.into(), missing: vec!["user:admin@example.com".into()] },
        ];
        let applied = Report { org_id: "123456789012".into(), domain: DOMAIN.into(), wrote: true, findings: findings.clone() };
        let out = applied.render();
        assert!(out.starts_with("default grants on organizations/123456789012:\n"), "{out}");
        assert!(
            out.contains("  removed domain:example.com from roles/resourcemanager.projectCreator — restore with: gcloud organizations add-iam-policy-binding 123456789012 --member=domain:example.com --role=roles/resourcemanager.projectCreator\n"),
            "{out}"
        );
        assert!(
            out.contains("  left in place: domain:example.com holds roles/billing.creator — user:admin@example.com declared and not live yet; the apply grants it and the next apply removes the domain's entry\n"),
            "{out}"
        );
        let planned = Report { wrote: false, ..applied };
        assert!(planned.render().contains("  apply would remove domain:example.com from roles/resourcemanager.projectCreator — plan changes nothing\n"), "{}", planned.render());
        let clean = Report { org_id: "123456789012".into(), domain: DOMAIN.into(), wrote: true, findings: vec![Finding::Clean { role: PC.into() }] };
        assert!(clean.render().contains("  domain:example.com does not hold roles/resourcemanager.projectCreator\n"), "{}", clean.render());
    }

    #[test]
    fn declared_holders_reads_the_unconditional_grants_of_the_two_roles_on_the_organisation() {
        let main_tf = r#"
resource "google_organization_iam_member" "a" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.projectCreator"
  member = "group:gcp-organization-admins@example.com"
}
resource "google_organization_iam_member" "b" {
  org_id = "123456789012"
  role   = "roles/billing.creator"
  member = "user:admin@example.com"
}
resource "google_organization_iam_member" "c" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.projectCreator"
  member = "user:admin@example.com"
  condition {
    title      = "t"
    expression = "e"
  }
}
resource "google_organization_iam_member" "d" {
  org_id = "123456789012"
  role   = "roles/viewer"
  member = "domain:example.com"
}
resource "google_organization_iam_member" "e" {
  org_id = "222222222222"
  role   = "roles/billing.creator"
  member = "group:other@example.org"
}
"#;
        let body = hcl::parse(main_tf).unwrap();
        let manifest = Manifest::from_blocks(body.blocks());
        let holders = declared_holders(&manifest, "123456789012");
        assert_eq!(holders.get(PC), Some(&vec!["group:gcp-organization-admins@example.com".to_string()]), "the conditional grant is not a holder");
        assert_eq!(holders.get(BC), Some(&vec!["user:admin@example.com".to_string()]), "another organisation's grant is not one either");
        assert_eq!(holders.len(), 2);
    }
}
