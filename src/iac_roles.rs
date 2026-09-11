//! What the IaC service account needs, per resource type an estate emits, and what
//! the estate grants it.
//!
//! The table is satz's own knowledge — which predefined role carries the permission
//! a resource type needs — so it is compiled in, like the bootstrap pre-flight's
//! `required_permissions`. `satz iac-roles --format json` prints it, and
//! `scripts/check_iac_roles.py` checks every entry against Google's role
//! definitions.
//!
//! The roles are granted at the organization, so every folder and project inherits
//! them — the projects that existed before the estate, and the ones created by
//! hand, included. `roles/owner` at the organization covers the organization and
//! project entries; the billing account keeps its own grants.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rmcp::schemars;

use crate::manifest::Manifest;

/// Where a role is granted, and where the live check tests it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    /// granted at the organization; tested there
    Organization,
    /// granted at the organization as well, inherited by every project; tested on
    /// the estate's infra project
    Project,
    /// granted on the billing account
    BillingAccount,
    /// a Google Workspace admin role, not IAM: named, never checked
    Workspace,
}

/// One capability: the permission that proves it, and the predefined roles that
/// carry it. Any one of them suffices; the first is the one `iac-roles --execute`
/// writes.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub(crate) struct Entry {
    pub permission: Option<&'static str>,
    pub roles: &'static [&'static str],
    pub scope: Scope,
}

const fn org(permission: &'static str, roles: &'static [&'static str]) -> Entry {
    Entry { permission: Some(permission), roles, scope: Scope::Organization }
}
const fn project(permission: &'static str, roles: &'static [&'static str]) -> Entry {
    Entry { permission: Some(permission), roles, scope: Scope::Project }
}
const fn billing(permission: &'static str, roles: &'static [&'static str]) -> Entry {
    Entry { permission: Some(permission), roles, scope: Scope::BillingAccount }
}
const GROUPS_ADMIN: Entry = Entry {
    permission: None,
    roles: &["Groups Admin (Google Workspace admin console)"],
    scope: Scope::Workspace,
};

/// Needed whatever the estate emits: `import`, `adopt` and `report-compliance`
/// read every project, the hand-made ones included.
pub(crate) const READ: &[Entry] = &[
    org("resourcemanager.projects.get", &["roles/viewer"]),
    org("resourcemanager.folders.list", &["roles/browser"]),
    org("resourcemanager.organizations.getIamPolicy", &["roles/iam.securityReviewer"]),
    org("cloudasset.assets.searchAllResources", &["roles/cloudasset.viewer"]),
    // API calls are billed to the infra project (user_project_override)
    project("serviceusage.services.use", &["roles/serviceusage.serviceUsageConsumer"]),
];

/// Per resource type: every type the preset library and the `init` template emit
/// (a test holds the library to it), and the types `satz import` brings in.
/// Deleting a project needs `roles/resourcemanager.projectDeleter`, which is not a
/// standing grant: the provider's `deletion_policy` refuses the delete by default.
pub(crate) const TYPES: &[(&str, &[Entry])] = &[
    ("google_artifact_registry_repository", &[project("artifactregistry.repositories.create", &["roles/artifactregistry.admin"])]),
    ("google_bigquery_dataset", &[project("bigquery.datasets.create", &["roles/bigquery.dataEditor"])]),
    ("google_billing_account_iam_member", &[billing("billing.accounts.setIamPolicy", &["roles/billing.admin"])]),
    ("google_billing_budget", &[billing("billing.budgets.create", &["roles/billing.admin", "roles/billing.costsManager"])]),
    ("google_cloud_identity_group", &[GROUPS_ADMIN]),
    ("google_cloud_identity_group_membership", &[GROUPS_ADMIN]),
    ("google_cloud_scheduler_job", &[project("cloudscheduler.jobs.create", &["roles/cloudscheduler.admin"])]),
    ("google_cloudbuild_trigger", &[project("cloudbuild.builds.create", &["roles/cloudbuild.builds.editor"])]),
    ("google_compute_address", &[project("compute.addresses.create", &["roles/compute.networkAdmin"])]),
    ("google_compute_firewall", &[project("compute.firewalls.create", &["roles/compute.securityAdmin"])]),
    ("google_compute_global_address", &[project("compute.globalAddresses.create", &["roles/compute.networkAdmin"])]),
    ("google_compute_network", &[project("compute.networks.create", &["roles/compute.networkAdmin"])]),
    ("google_compute_router", &[project("compute.routers.create", &["roles/compute.networkAdmin"])]),
    ("google_compute_router_nat", &[project("compute.routers.update", &["roles/compute.networkAdmin"])]),
    ("google_compute_subnetwork", &[project("compute.subnetworks.create", &["roles/compute.networkAdmin"])]),
    ("google_dns_managed_zone", &[project("dns.managedZones.create", &["roles/dns.admin"])]),
    ("google_dns_record_set", &[project("dns.changes.create", &["roles/dns.admin"])]),
    ("google_essential_contacts_contact", &[org("essentialcontacts.contacts.create", &["roles/essentialcontacts.admin"])]),
    ("google_folder", &[org("resourcemanager.folders.create", &["roles/resourcemanager.folderAdmin"])]),
    (
        "google_folder_iam_member",
        &[org("resourcemanager.folders.setIamPolicy", &["roles/resourcemanager.folderAdmin", "roles/resourcemanager.organizationAdmin"])],
    ),
    ("google_iam_workload_identity_pool", &[project("iam.googleapis.com/workloadIdentityPools.create", &["roles/iam.workloadIdentityPoolAdmin"])]),
    (
        "google_iam_workload_identity_pool_provider",
        &[project("iam.googleapis.com/workloadIdentityPoolProviders.create", &["roles/iam.workloadIdentityPoolAdmin"])],
    ),
    ("google_kms_crypto_key", &[project("cloudkms.cryptoKeys.create", &["roles/cloudkms.admin"])]),
    ("google_kms_key_ring", &[project("cloudkms.keyRings.create", &["roles/cloudkms.admin"])]),
    ("google_logging_folder_sink", &[org("logging.sinks.create", &["roles/logging.configWriter"])]),
    ("google_logging_metric", &[project("logging.logMetrics.create", &["roles/logging.configWriter"])]),
    ("google_logging_organization_sink", &[org("logging.sinks.create", &["roles/logging.configWriter"])]),
    ("google_logging_project_bucket_config", &[project("logging.buckets.create", &["roles/logging.configWriter"])]),
    ("google_logging_project_sink", &[project("logging.sinks.create", &["roles/logging.configWriter"])]),
    ("google_monitoring_alert_policy", &[project("monitoring.alertPolicies.create", &["roles/monitoring.alertPolicyEditor"])]),
    (
        "google_monitoring_notification_channel",
        &[project("monitoring.notificationChannels.create", &["roles/monitoring.notificationChannelEditor"])],
    ),
    ("google_org_policy_policy", &[org("orgpolicy.policies.create", &["roles/orgpolicy.policyAdmin"])]),
    (
        "google_organization_iam_audit_config",
        &[org("resourcemanager.organizations.setIamPolicy", &["roles/resourcemanager.organizationAdmin"])],
    ),
    ("google_organization_iam_custom_role", &[org("iam.roles.create", &["roles/iam.organizationRoleAdmin"])]),
    (
        "google_organization_iam_member",
        &[org("resourcemanager.organizations.setIamPolicy", &["roles/resourcemanager.organizationAdmin"])],
    ),
    (
        "google_project",
        &[
            org("resourcemanager.projects.create", &["roles/resourcemanager.projectCreator"]),
            org("resourcemanager.projects.update", &["roles/resourcemanager.projectMover"]),
            org("resourcemanager.projects.createBillingAssignment", &["roles/billing.projectManager"]),
            billing("billing.resourceAssociations.create", &["roles/billing.user", "roles/billing.admin"]),
        ],
    ),
    (
        "google_project_iam_member",
        &[org(
            "resourcemanager.projects.setIamPolicy",
            &["roles/resourcemanager.projectIamAdmin", "roles/resourcemanager.organizationAdmin", "roles/resourcemanager.folderAdmin"],
        )],
    ),
    ("google_project_service", &[project("serviceusage.services.enable", &["roles/serviceusage.serviceUsageAdmin"])]),
    ("google_pubsub_subscription", &[project("pubsub.subscriptions.create", &["roles/pubsub.editor"])]),
    ("google_pubsub_topic", &[project("pubsub.topics.create", &["roles/pubsub.editor"])]),
    ("google_secret_manager_secret", &[project("secretmanager.secrets.create", &["roles/secretmanager.admin"])]),
    ("google_service_account", &[project("iam.serviceAccounts.create", &["roles/iam.serviceAccountAdmin"])]),
    ("google_service_account_iam_member", &[project("iam.serviceAccounts.setIamPolicy", &["roles/iam.serviceAccountAdmin"])]),
    ("google_storage_bucket", &[project("storage.buckets.create", &["roles/storage.admin"])]),
    ("google_storage_bucket_iam_member", &[project("storage.buckets.setIamPolicy", &["roles/storage.admin"])]),
];

/// The table's entries for one resource type.
pub(crate) fn entries_for(tf_type: &str) -> Option<&'static [Entry]> {
    TYPES.iter().find(|(t, _)| *t == tf_type).map(|(_, e)| *e)
}

/// The whole table as data — what `iac-roles --format json` prints and
/// `scripts/check_iac_roles.py` reads.
pub(crate) fn table_json() -> serde_json::Value {
    let types: BTreeMap<&str, &[Entry]> = TYPES.iter().map(|(t, e)| (*t, *e)).collect();
    serde_json::json!({ "read": READ, "types": types })
}

/// The table for a terminal: per type, each permission, the roles that carry it
/// and where they are granted.
pub(crate) fn render_table() -> String {
    let line = |e: &Entry| {
        let scope = match e.scope {
            Scope::Organization => "organization",
            Scope::Project => "project (granted at the organization)",
            Scope::BillingAccount => "billing account",
            Scope::Workspace => "Google Workspace, not IAM",
        };
        format!("  {:<56} {} — {}\n", e.permission.unwrap_or("—"), e.roles.join(" | "), scope)
    };
    let mut out = String::from("always (import, adopt, report):\n");
    READ.iter().for_each(|e| out.push_str(&line(e)));
    for (t, entries) in TYPES {
        out.push_str(&format!("{}:\n", t));
        entries.iter().for_each(|e| out.push_str(&line(e)));
    }
    out
}

/// One capability the estate needs its IaC service account to hold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Need {
    /// `read`, or the resource types that need it
    pub reason: Vec<String>,
    /// absent for a Workspace role
    pub permission: Option<String>,
    /// any one of these carries it
    pub roles: Vec<String>,
    pub scope: Scope,
}

/// What an estate's emitted resources need — `read` first, then one `Need` per
/// capability, merged across the types that share it — and the emitted types the
/// table has no entry for.
pub(crate) fn needs(manifest: &Manifest) -> (Vec<Need>, BTreeSet<String>) {
    let types: BTreeSet<&str> = manifest.resources.values().map(|r| r.tf_type.as_str()).collect();
    let mut out: Vec<Need> = Vec::new();
    let mut add = |reason: &str, e: &Entry| {
        let same = |n: &Need| {
            n.permission.as_deref() == e.permission
                && n.scope == e.scope
                && n.roles.iter().map(String::as_str).eq(e.roles.iter().copied())
        };
        match out.iter_mut().find(|n| same(n)) {
            Some(n) => {
                if !n.reason.iter().any(|r| r == reason) {
                    n.reason.push(reason.to_string());
                }
            }
            None => out.push(Need {
                reason: vec![reason.to_string()],
                permission: e.permission.map(str::to_string),
                roles: e.roles.iter().map(|r| r.to_string()).collect(),
                scope: e.scope,
            }),
        }
    };
    for e in READ {
        add("read", e);
    }
    let mut unknown = BTreeSet::new();
    for t in &types {
        match entries_for(t) {
            Some(es) => es.iter().for_each(|e| add(t, e)),
            None => {
                unknown.insert(t.to_string());
            }
        }
    }
    (out, unknown)
}

/// The roles an estate grants its IaC service account, by where.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Granted {
    pub organization: BTreeSet<String>,
    pub billing_account: BTreeSet<String>,
}

impl Granted {
    pub(crate) fn owner(&self) -> bool {
        self.organization.contains(OWNER)
    }
}

pub(crate) const OWNER: &str = "roles/owner";

/// The roles the estate — every fragment of it, packs included — grants the
/// service account at the organization and on the billing account.
pub(crate) fn granted(manifest: &Manifest, service_account: &str) -> Granted {
    let member = format!("serviceAccount:{}", service_account);
    let roles_of = |tf_type: &str| -> BTreeSet<String> {
        manifest
            .of_type(tf_type)
            .filter(|r| r.attrs.get("member") == Some(&member))
            .filter_map(|r| r.attrs.get("role").cloned())
            .collect()
    };
    Granted {
        organization: roles_of("google_organization_iam_member"),
        billing_account: roles_of("google_billing_account_iam_member"),
    }
}

/// The needs no granted role meets. `roles/owner` at the organization meets every
/// organization and project need; Workspace roles are never checked.
pub(crate) fn missing(needs: &[Need], granted: &Granted) -> Vec<Need> {
    needs
        .iter()
        .filter(|n| match n.scope {
            Scope::Organization | Scope::Project => {
                !granted.owner() && !n.roles.iter().any(|r| granted.organization.contains(r))
            }
            Scope::BillingAccount => !n.roles.iter().any(|r| granted.billing_account.contains(r)),
            Scope::Workspace => false,
        })
        .cloned()
        .collect()
}

/// One role that meets missing needs: where it is granted — `organization` for
/// organization and project needs, which inherit it, or `billing_account` — and
/// what needs it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Pick {
    pub role: String,
    pub scope: Scope,
    pub reason: BTreeSet<String>,
}

/// The fewest roles that meet `needs`: a need only one role meets picks that role
/// first, and a need with alternatives takes a role already picked before its
/// first. Workspace needs are not IAM roles and pick nothing.
pub(crate) fn cover(needs: &[Need]) -> Vec<Pick> {
    let mut order: Vec<&Need> = needs.iter().filter(|n| n.scope != Scope::Workspace).collect();
    order.sort_by_key(|n| n.roles.len());
    let mut picks: Vec<Pick> = Vec::new();
    for n in order {
        let scope = if n.scope == Scope::BillingAccount { Scope::BillingAccount } else { Scope::Organization };
        match picks.iter_mut().find(|p| p.scope == scope && n.roles.contains(&p.role)) {
            Some(p) => p.reason.extend(n.reason.iter().cloned()),
            None => picks.push(Pick { role: n.roles[0].clone(), scope, reason: n.reason.iter().cloned().collect() }),
        }
    }
    picks.sort_by(|a, b| (a.scope == Scope::BillingAccount, &a.role).cmp(&(b.scope == Scope::BillingAccount, &b.role)));
    picks
}

/// What `--execute` writes for the estate's gaps. A write grants on the
/// organization or the billing account, and so emits that grant's resource type —
/// `google_billing_account_iam_member` for a billing account the estate grants
/// nothing on yet — whose own needs the same write meets.
pub(crate) fn plan(gaps: &[Need], granted: &Granted) -> Vec<Pick> {
    let mut all = gaps.to_vec();
    for (billing, grant_type) in [(false, "google_organization_iam_member"), (true, "google_billing_account_iam_member")] {
        let writes_here = gaps.iter().any(|n| n.scope != Scope::Workspace && (n.scope == Scope::BillingAccount) == billing);
        if !writes_here {
            continue;
        }
        let introduced: Vec<Need> = entries_for(grant_type)
            .unwrap_or(&[])
            .iter()
            .map(|e| Need {
                reason: vec![grant_type.to_string()],
                permission: e.permission.map(str::to_string),
                roles: e.roles.iter().map(|r| r.to_string()).collect(),
                scope: e.scope,
            })
            .collect();
        all.extend(missing(&introduced, granted));
    }
    cover(&all)
}

/// The roles of `picks`, by where they are granted — (organization, billing account).
pub(crate) fn to_write(picks: &[Pick]) -> (BTreeSet<String>, BTreeSet<String>) {
    let of = |scope: Scope| picks.iter().filter(|p| p.scope == scope).map(|p| p.role.clone()).collect();
    (of(Scope::Organization), of(Scope::BillingAccount))
}

/// One line per picked role, with what needs it.
pub(crate) fn describe(picks: &[Pick]) -> Vec<String> {
    picks
        .iter()
        .map(|p| {
            let at = if p.scope == Scope::BillingAccount { "on the billing account" } else { "at the organization" };
            format!("{} {} — for {}", p.role, at, p.reason.iter().cloned().collect::<Vec<_>>().join(", "))
        })
        .collect()
}

/// The IaC service account an estate declares, whatever its deployment mode:
/// `svc_iac_account` in `infra_project_name`. `None` when either is unset.
pub(crate) fn service_account_of(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    match (get("svc_iac_account"), get("infra_project_name")) {
        (Some(a), Some(p)) if !a.is_empty() && !p.is_empty() => Some(format!("{}@{}.iam.gserviceaccount.com", a, p)),
        _ => None,
    }
}

/// What `satz whoami <estate>` tests live: the estate's needs, and where each
/// scope is tested.
#[derive(Debug, Clone)]
pub(crate) struct Probe {
    pub needs: Vec<Need>,
    /// `organizations/N`, or `folders/N` for a folder-scoped estate
    pub scope_root: Option<String>,
    /// `projects/<infra project>` — it inherits the organization's grants like any
    /// project, hand-made ones included
    pub project: Option<String>,
    pub billing_account: Option<String>,
}

/// The permissions an estate's resource types need, tested with the credential
/// its live commands run as.
#[derive(Debug, Clone, Default, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PermissionCheck {
    /// permissions tested
    pub tested: usize,
    /// the needs whose permission the credential does not hold
    pub missing: Vec<Need>,
    /// what was not tested, and why
    pub not_tested: Vec<String>,
}

/// Test the probe's permissions with `testIamPermissions`, as the identity the
/// estate's live commands run as — the IaC service account in cloud mode.
pub(crate) async fn test_live(probe: &Probe) -> PermissionCheck {
    let mut check = PermissionCheck::default();
    let token = match crate::gcp::access_token().await {
        Ok(t) => t,
        Err(e) => {
            check.not_tested.push(format!("no token: {}", e));
            return check;
        }
    };
    let client = reqwest::Client::new();
    for (scope, target, what) in [
        (Scope::Organization, &probe.scope_root, "customer_organization_id"),
        (Scope::Project, &probe.project, "infra_project_name"),
        (Scope::BillingAccount, &probe.billing_account, "billing_account_infra"),
    ] {
        let needs: Vec<&Need> = probe.needs.iter().filter(|n| n.scope == scope && n.permission.is_some()).collect();
        if needs.is_empty() {
            continue;
        }
        let Some(target) = target else {
            check.not_tested.push(format!("{} permission(s): the estate sets no {}", needs.len(), what));
            continue;
        };
        let perms: Vec<&str> = needs.iter().filter_map(|n| n.permission.as_deref()).collect::<BTreeSet<_>>().into_iter().collect();
        let held = match scope {
            Scope::BillingAccount => crate::gcp::billing::test_billing_permissions(&client, &token, target, &perms).await,
            _ => crate::gcp::resourcemanager::test_permissions(&client, &token, target, &perms).await,
        };
        match held {
            Ok(held) => {
                check.tested += perms.len();
                check.missing.extend(
                    needs.into_iter().filter(|n| !held.iter().any(|p| Some(p.as_str()) == n.permission.as_deref())).cloned(),
                );
            }
            Err(e) => check.not_tested.push(format!("{}: {}", target, e)),
        }
    }
    if probe.needs.iter().any(|n| n.scope == Scope::Workspace) {
        check.not_tested.push("Groups Admin (Google Workspace admin console) — not an IAM role".to_string());
    }
    check
}

/// The grant-map key the estate writes for its service account.
const SA_KEY: &str = "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com";

/// Write the roles into the estate's main file: into the service account's
/// existing grant list in a `google_organization_iam_member` /
/// `google_billing_account_iam_member` block when there is one — its key matched
/// after `{param}` interpolation — else as a new block at the end of the file.
/// Returns what it wrote, one line per role.
pub(crate) fn write_grants(
    estate: &Path,
    params: &HashMap<String, String>,
    service_account: &str,
    org_roles: &BTreeSet<String>,
    billing_roles: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let member = format!("serviceAccount:{}", service_account);
    let mut out = text.clone();
    let mut written = Vec::new();
    for (block, roles) in [("google_organization_iam_member", org_roles), ("google_billing_account_iam_member", billing_roles)] {
        if roles.is_empty() {
            continue;
        }
        out = match add_to_existing(&out, block, &member, params, roles) {
            Some(edited) => edited,
            None => append_block(&out, block, roles),
        };
        written.extend(roles.iter().map(|r| format!("{} in {}", r, block)));
    }
    std::fs::write(estate, out).map_err(|e| format!("{}: {}", estate.display(), e))?;
    Ok(written)
}

/// `{name}` replaced by the param's value — the key as the compiler reads it.
fn interpolate(key: &str, params: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = key;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &rest[open + 1..open + close];
        match params.get(name) {
            Some(v) => out.push_str(v),
            None => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

/// Add the roles to the service account's list inside the first top-level
/// `block { … }` that holds it. `None` when no such list exists.
fn add_to_existing(text: &str, block: &str, member: &str, params: &HashMap<String, String>, roles: &BTreeSet<String>) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut depth: i32 = 0;
    let mut in_block = false;
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim().to_string();
        if depth == 0 && t.starts_with(block) && t.ends_with('{') {
            in_block = true;
        } else if in_block && depth == 1 {
            if let Some(key) = t.strip_prefix('"').and_then(|r| r.split_once('"')).map(|(k, _)| k) {
                if interpolate(key, params) == member && t.contains("= [") {
                    let new: Vec<&String> = roles.iter().collect();
                    if t.trim_end_matches(',').ends_with(']') {
                        // one-line list: `"key" = ["a", "b"]`
                        let close = lines[i].rfind(']')?;
                        let inner = lines[i][..close].trim_end().to_string();
                        let sep = if inner.ends_with('[') { "" } else { ", " };
                        let add = new.iter().map(|r| format!("\"{}\"", r)).collect::<Vec<_>>().join(", ");
                        lines[i] = format!("{}{}{}{}", inner, sep, add, &lines[i][close..]);
                    } else {
                        // multi-line list: insert before its closing `]`
                        let key_indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
                        let mut j = i + 1;
                        while j < lines.len() && !lines[j].trim_start().starts_with(']') {
                            j += 1;
                        }
                        if j == lines.len() {
                            return None;
                        }
                        let item_indent = if j > i + 1 {
                            lines[i + 1].chars().take_while(|c| c.is_whitespace()).collect()
                        } else {
                            format!("{}  ", key_indent)
                        };
                        if j > i + 1 && !lines[j - 1].trim_end().ends_with(',') && !lines[j - 1].trim().is_empty() {
                            lines[j - 1].push(',');
                        }
                        for (k, r) in new.iter().enumerate() {
                            lines.insert(j + k, format!("{}\"{}\",", item_indent, r));
                        }
                    }
                    let mut s = lines.join("\n");
                    if text.ends_with('\n') {
                        s.push('\n');
                    }
                    return Some(s);
                }
            }
        }
        for c in t.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 {
            in_block = false;
        }
        i += 1;
    }
    None
}

/// A new top-level block granting the roles.
fn append_block(text: &str, block: &str, roles: &BTreeSet<String>) -> String {
    let mut s = text.trim_end().to_string();
    s.push_str("\n\n// The IaC service account's roles for this estate's resource types (`satz iac-roles`).\n");
    s.push_str(&format!("{} {{\n", block));
    if block == "google_billing_account_iam_member" {
        s.push_str("  billing_account_id = billing_account_infra\n");
    }
    s.push_str(&format!("  \"{}\" = [\n", SA_KEY));
    for r in roles {
        s.push_str(&format!("    \"{}\",\n", r));
    }
    s.push_str("  ]\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SA: &str = "svc-iac-001@corp-infra-001.iam.gserviceaccount.com";

    fn manifest(tf: &str) -> Manifest {
        Manifest::parse(tf)
    }

    fn grant(tf_type: &str, role: &str) -> String {
        let scope = if tf_type == "google_billing_account_iam_member" {
            "billing_account_id = \"01AA-BB-CC\""
        } else {
            "org_id = \"123456789012\""
        };
        format!(
            "resource \"{t}\" \"g_{h}\" {{\n  role = \"{role}\"\n  member = \"serviceAccount:{SA}\"\n  {scope}\n}}\n",
            t = tf_type,
            h = role.replace(['/', '.'], "_"),
        )
    }

    #[test]
    fn the_table_is_well_formed_and_sorted() {
        let mut seen = BTreeSet::new();
        let mut last = "";
        for (t, entries) in TYPES {
            assert!(seen.insert(*t), "{} listed twice", t);
            assert!(*t > last, "{} is out of order", t);
            last = t;
            assert!(!entries.is_empty(), "{} has no entry", t);
            for e in *entries {
                assert!(!e.roles.is_empty(), "{}: an entry names no role", t);
                assert_eq!(e.permission.is_none(), e.scope == Scope::Workspace, "{}: only a Workspace entry has no permission", t);
                if e.scope != Scope::Workspace {
                    assert!(e.roles.iter().all(|r| r.starts_with("roles/")), "{}: {:?}", t, e.roles);
                }
            }
        }
        let json = table_json();
        assert!(json["types"]["google_project"].as_array().is_some_and(|a| a.len() == 4));
        assert_eq!(json["read"].as_array().map(Vec::len), Some(READ.len()));
    }

    #[test]
    fn needs_merge_shared_capabilities_and_name_unknown_types() {
        let m = manifest(
            "resource \"google_organization_iam_member\" \"a\" {\n  role = \"roles/viewer\"\n  member = \"group:x@example.com\"\n  org_id = \"1\"\n}\n\
             resource \"google_organization_iam_audit_config\" \"b\" {\n  org_id = \"1\"\n  service = \"allServices\"\n}\n\
             resource \"google_compute_instance\" \"c\" {\n  name = \"vm\"\n}\n",
        );
        let (needs, unknown) = needs(&m);
        let set_iam: Vec<&Need> = needs.iter().filter(|n| n.permission.as_deref() == Some("resourcemanager.organizations.setIamPolicy")).collect();
        assert_eq!(set_iam.len(), 1);
        assert_eq!(set_iam[0].reason, vec!["google_organization_iam_audit_config", "google_organization_iam_member"]);
        assert!(needs.iter().take(READ.len()).all(|n| n.reason == ["read"]));
        assert_eq!(unknown.into_iter().collect::<Vec<_>>(), ["google_compute_instance"]);
    }

    #[test]
    fn missing_is_per_scope_and_owner_covers_the_organization() {
        let tf = format!(
            "{}{}{}resource \"google_storage_bucket\" \"s\" {{\n  name = \"b\"\n}}\n\
             resource \"google_billing_budget\" \"bud\" {{\n  display_name = \"x\"\n}}\n\
             resource \"google_cloud_identity_group\" \"g\" {{\n  display_name = \"g\"\n}}\n",
            grant("google_organization_iam_member", "roles/viewer"),
            grant("google_organization_iam_member", "roles/storage.admin"),
            grant("google_billing_account_iam_member", "roles/billing.admin"),
        );
        let m = manifest(&tf);
        let (want, _) = needs(&m);
        let g = granted(&m, SA);
        assert!(g.organization.contains("roles/storage.admin") && g.billing_account.contains("roles/billing.admin"));
        let miss = missing(&want, &g);
        let roles: BTreeSet<&str> = miss.iter().map(|n| n.roles[0].as_str()).collect();
        // storage and the budget are covered; the org-level read roles and IAM admin are not
        assert!(!roles.contains("roles/storage.admin") && !roles.contains("roles/billing.admin"), "{:?}", roles);
        assert!(roles.contains("roles/browser") && roles.contains("roles/resourcemanager.organizationAdmin"), "{:?}", roles);
        // the Workspace role is named, never counted as missing
        assert!(miss.iter().all(|n| n.scope != Scope::Workspace));

        // owner at the organization covers every organization and project need,
        // and nothing on the billing account
        let owner = format!(
            "{}resource \"google_storage_bucket\" \"s\" {{\n  name = \"b\"\n}}\n\
             resource \"google_billing_budget\" \"bud\" {{\n  display_name = \"x\"\n}}\n",
            grant("google_organization_iam_member", "roles/owner")
        );
        let m = manifest(&owner);
        let (want, _) = needs(&m);
        let miss = missing(&want, &granted(&m, SA));
        assert_eq!(miss.iter().map(|n| n.scope).collect::<Vec<_>>(), [Scope::BillingAccount]);
    }

    #[test]
    fn the_write_meets_the_grant_it_adds_with_the_fewest_roles() {
        // A project on a billing account the estate grants nothing on: the
        // association alone would take billing.user, but the grant block the write
        // adds needs billing.admin, which carries the association too.
        let tf = format!(
            "{}resource \"google_project\" \"p\" {{\n  project_id = \"p\"\n  billing_account = \"01AA-BB-CC\"\n}}\n",
            grant("google_organization_iam_member", "roles/resourcemanager.organizationAdmin"),
        );
        let m = manifest(&tf);
        let g = granted(&m, SA);
        let gaps = missing(&needs(&m).0, &g);
        let (org, bill) = to_write(&plan(&gaps, &g));
        assert_eq!(bill.into_iter().collect::<Vec<_>>(), ["roles/billing.admin"]);
        assert!(org.contains("roles/resourcemanager.projectCreator") && org.contains("roles/viewer"), "{:?}", org);
        assert!(!org.contains("roles/resourcemanager.organizationAdmin"), "already granted: {:?}", org);
        // the line names the grant type as a reason, so the role is not a surprise
        let lines = describe(&plan(&gaps, &g));
        assert!(
            lines.contains(&"roles/billing.admin on the billing account — for google_billing_account_iam_member, google_project".to_string()),
            "{:?}",
            lines
        );

        // an estate with no grant at all gets organizationAdmin for the block the write adds
        let bare = manifest("resource \"google_storage_bucket\" \"s\" {\n  name = \"b\"\n}\n");
        let g = granted(&bare, SA);
        let (org, bill) = to_write(&plan(&missing(&needs(&bare).0, &g), &g));
        assert!(org.contains("roles/resourcemanager.organizationAdmin") && org.contains("roles/storage.admin"), "{:?}", org);
        assert!(bill.is_empty());
    }

    #[test]
    fn the_service_account_is_derived_whatever_the_mode() {
        let p: HashMap<&str, &str> = [("svc_iac_account", "svc-iac-001"), ("infra_project_name", "corp-infra-001")].into();
        assert_eq!(service_account_of(|k| p.get(k).map(|v| v.to_string())).as_deref(), Some(SA));
        assert_eq!(service_account_of(|_| None), None);
    }

    fn params() -> HashMap<String, String> {
        [("svc_iac_account", "svc-iac-001"), ("infra_project_name", "corp-infra-001")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn roles(r: &[&str]) -> BTreeSet<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn write_adds_to_the_existing_list_matched_by_its_interpolated_key() {
        let src = "estate e\n\ngoogle_organization_iam_member {\n  \"group:x@example.com\" = [\"roles/viewer\"]\n  \"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\" = [\n    \"roles/resourcemanager.organizationAdmin\",\n    \"roles/storage.admin\"\n  ]\n}\n";
        let edited = add_to_existing(src, "google_organization_iam_member", &format!("serviceAccount:{}", SA), &params(), &roles(&["roles/browser", "roles/viewer"])).unwrap();
        assert!(edited.contains("    \"roles/storage.admin\",\n    \"roles/browser\",\n    \"roles/viewer\",\n  ]\n"), "{}", edited);
        // the other member's list is untouched
        assert!(edited.contains("\"group:x@example.com\" = [\"roles/viewer\"]\n"), "{}", edited);
        // a one-line list grows in place
        let one = "google_organization_iam_member {\n  \"serviceAccount:svc-iac-001@corp-infra-001.iam.gserviceaccount.com\" = [\"roles/viewer\"]\n}\n";
        let edited = add_to_existing(one, "google_organization_iam_member", &format!("serviceAccount:{}", SA), &params(), &roles(&["roles/browser"])).unwrap();
        assert!(edited.contains("= [\"roles/viewer\", \"roles/browser\"]\n"), "{}", edited);
    }

    #[test]
    fn write_appends_a_block_when_the_estate_has_no_list_for_the_account() {
        let src = "estate e\n\ngoogle_organization_iam_member {\n  \"group:x@example.com\" = [\"roles/viewer\"]\n}\n";
        assert!(add_to_existing(src, "google_organization_iam_member", &format!("serviceAccount:{}", SA), &params(), &roles(&["roles/browser"])).is_none());
        let appended = append_block(src, "google_billing_account_iam_member", &roles(&["roles/billing.admin"]));
        assert!(appended.ends_with(
            "google_billing_account_iam_member {\n  billing_account_id = billing_account_infra\n  \"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\" = [\n    \"roles/billing.admin\",\n  ]\n}\n"
        ), "{}", appended);
    }
}
