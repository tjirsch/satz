//! What the IaC service account needs, per resource type an estate emits, and what
//! the estate grants it.
//!
//! The table is satz's own knowledge — which predefined role carries the permission
//! a resource type needs — so it is compiled in, like the bootstrap pre-flight's
//! `required_permissions`. `satz update-prerequisites --format json` prints it, and
//! `scripts/check_prerequisites.py` checks every role entry against Google's role
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
/// carry it. Any one of them suffices; the first is the one `update-prerequisites`
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
/// No row carries `resourcemanager.projects.delete`. Google makes a project's
/// creator its owner, so the account deletes the projects it created and no other;
/// a project it did not create needs `roles/resourcemanager.projectDeleter`, which is
/// not a standing grant. The provider's `deletion_policy` refuses the delete by default.
pub(crate) const TYPES: &[(&str, &[Entry], &[&str])] = &[
    ("google_artifact_registry_repository", &[project("artifactregistry.repositories.create", &["roles/artifactregistry.admin"])], &["artifactregistry.googleapis.com"]),
    ("google_bigquery_dataset", &[project("bigquery.datasets.create", &["roles/bigquery.dataEditor"])], &["bigquery.googleapis.com"]),
    // dataEditor can create a dataset; handing out access on one needs admin
    ("google_bigquery_dataset_iam_member", &[project("bigquery.datasets.setIamPolicy", &["roles/bigquery.admin"])], &["bigquery.googleapis.com"]),
    ("google_billing_account_iam_member", &[billing("billing.accounts.setIamPolicy", &["roles/billing.admin"])], &["cloudbilling.googleapis.com"]),
    ("google_billing_budget", &[billing("billing.budgets.create", &["roles/billing.admin", "roles/billing.costsManager"])], &["billingbudgets.googleapis.com"]),
    ("google_cloud_identity_group", &[GROUPS_ADMIN], &["cloudidentity.googleapis.com"]),
    ("google_cloud_identity_group_membership", &[GROUPS_ADMIN], &["cloudidentity.googleapis.com"]),
    ("google_cloud_scheduler_job", &[project("cloudscheduler.jobs.create", &["roles/cloudscheduler.admin"])], &["cloudscheduler.googleapis.com"]),
    ("google_cloudbuild_trigger", &[project("cloudbuild.builds.create", &["roles/cloudbuild.builds.editor"])], &["cloudbuild.googleapis.com"]),
    ("google_compute_address", &[project("compute.addresses.create", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_firewall", &[project("compute.firewalls.create", &["roles/compute.securityAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_firewall_policy", &[org("compute.firewallPolicies.create", &["roles/compute.orgFirewallPolicyAdmin"])], &["compute.googleapis.com"]),
    (
        "google_compute_firewall_policy_association",
        &[org("compute.organizations.setFirewallPolicy", &["roles/compute.orgSecurityResourceAdmin"])],
        &["compute.googleapis.com"],
    ),
    ("google_compute_firewall_policy_rule", &[org("compute.firewallPolicies.update", &["roles/compute.orgFirewallPolicyAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_global_address", &[project("compute.globalAddresses.create", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_network", &[project("compute.networks.create", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_router", &[project("compute.routers.create", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_router_nat", &[project("compute.routers.update", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_compute_subnetwork", &[project("compute.subnetworks.create", &["roles/compute.networkAdmin"])], &["compute.googleapis.com"]),
    ("google_dns_managed_zone", &[project("dns.managedZones.create", &["roles/dns.admin"])], &["dns.googleapis.com"]),
    ("google_dns_record_set", &[project("dns.changes.create", &["roles/dns.admin"])], &["dns.googleapis.com"]),
    ("google_essential_contacts_contact", &[org("essentialcontacts.contacts.create", &["roles/essentialcontacts.admin"])], &["essentialcontacts.googleapis.com"]),
    ("google_folder", &[org("resourcemanager.folders.create", &["roles/resourcemanager.folderAdmin"])], &["cloudresourcemanager.googleapis.com"]),
    (
        "google_folder_iam_member",
        &[org("resourcemanager.folders.setIamPolicy", &["roles/resourcemanager.folderAdmin", "roles/resourcemanager.organizationAdmin"])],
        &["cloudresourcemanager.googleapis.com"],
    ),
    (
        "google_iam_workload_identity_pool",
        &[project("iam.googleapis.com/workloadIdentityPools.create", &["roles/iam.workloadIdentityPoolAdmin"])],
        &["iam.googleapis.com"],
    ),
    (
        "google_iam_workload_identity_pool_provider",
        &[project("iam.googleapis.com/workloadIdentityPoolProviders.create", &["roles/iam.workloadIdentityPoolAdmin"])],
        &["iam.googleapis.com"],
    ),
    ("google_kms_crypto_key", &[project("cloudkms.cryptoKeys.create", &["roles/cloudkms.admin"])], &["cloudkms.googleapis.com"]),
    ("google_kms_key_ring", &[project("cloudkms.keyRings.create", &["roles/cloudkms.admin"])], &["cloudkms.googleapis.com"]),
    ("google_logging_folder_sink", &[org("logging.sinks.create", &["roles/logging.configWriter"])], &["logging.googleapis.com"]),
    ("google_logging_metric", &[project("logging.logMetrics.create", &["roles/logging.configWriter"])], &["logging.googleapis.com"]),
    ("google_logging_organization_sink", &[org("logging.sinks.create", &["roles/logging.configWriter"])], &["logging.googleapis.com"]),
    ("google_logging_project_bucket_config", &[project("logging.buckets.create", &["roles/logging.configWriter"])], &["logging.googleapis.com"]),
    ("google_logging_project_sink", &[project("logging.sinks.create", &["roles/logging.configWriter"])], &["logging.googleapis.com"]),
    ("google_monitoring_alert_policy", &[project("monitoring.alertPolicies.create", &["roles/monitoring.alertPolicyEditor"])], &["monitoring.googleapis.com"]),
    (
        "google_monitoring_notification_channel",
        &[project("monitoring.notificationChannels.create", &["roles/monitoring.notificationChannelEditor"])],
        &["monitoring.googleapis.com"],
    ),
    ("google_org_policy_custom_constraint", &[org("orgpolicy.customConstraints.create", &["roles/orgpolicy.policyAdmin"])], &["orgpolicy.googleapis.com"]),
    ("google_org_policy_policy", &[org("orgpolicy.policies.create", &["roles/orgpolicy.policyAdmin"])], &["orgpolicy.googleapis.com"]),
    (
        "google_organization_access_approval_settings",
        &[org("accessapproval.settings.update", &["roles/accessapproval.configEditor"])],
        &["accessapproval.googleapis.com"],
    ),
    (
        "google_organization_iam_audit_config",
        &[org("resourcemanager.organizations.setIamPolicy", &["roles/resourcemanager.organizationAdmin"])],
        &["cloudresourcemanager.googleapis.com"],
    ),
    ("google_organization_iam_custom_role", &[org("iam.roles.create", &["roles/iam.organizationRoleAdmin"])], &["iam.googleapis.com"]),
    (
        "google_organization_iam_member",
        &[org("resourcemanager.organizations.setIamPolicy", &["roles/resourcemanager.organizationAdmin"])],
        &["cloudresourcemanager.googleapis.com"],
    ),
    (
        "google_project",
        &[
            org("resourcemanager.projects.create", &["roles/resourcemanager.projectCreator"]),
            org("resourcemanager.projects.update", &["roles/resourcemanager.projectMover"]),
            org("resourcemanager.projects.createBillingAssignment", &["roles/billing.projectManager"]),
            billing("billing.resourceAssociations.create", &["roles/billing.user", "roles/billing.admin"]),
        ],
        // creating the project is Resource Manager; linking it to the billing account is
        // Cloud Billing, and a project with no billing link cannot enable a paid API
        &["cloudresourcemanager.googleapis.com", "cloudbilling.googleapis.com"],
    ),
    (
        "google_project_iam_member",
        &[org(
            "resourcemanager.projects.setIamPolicy",
            &["roles/resourcemanager.projectIamAdmin", "roles/resourcemanager.organizationAdmin", "roles/resourcemanager.folderAdmin"],
        )],
        &["cloudresourcemanager.googleapis.com"],
    ),
    ("google_project_service", &[project("serviceusage.services.enable", &["roles/serviceusage.serviceUsageAdmin"])], &["serviceusage.googleapis.com"]),
    ("google_pubsub_subscription", &[project("pubsub.subscriptions.create", &["roles/pubsub.editor"])], &["pubsub.googleapis.com"]),
    // Same as the topic below: `roles/pubsub.editor` does not carry setIamPolicy.
    ("google_pubsub_subscription_iam_member", &[project("pubsub.subscriptions.setIamPolicy", &["roles/pubsub.admin"])], &["pubsub.googleapis.com"]),
    ("google_pubsub_topic", &[project("pubsub.topics.create", &["roles/pubsub.editor"])], &["pubsub.googleapis.com"]),
    // the SCC notification chain: the grant on the topic, and the config at the
    // organisation. `roles/pubsub.editor` does not carry setIamPolicy — admin does.
    ("google_pubsub_topic_iam_member", &[project("pubsub.topics.setIamPolicy", &["roles/pubsub.admin"])], &["pubsub.googleapis.com"]),
    (
        "google_scc_v2_organization_notification_config",
        &[org(
            "securitycenter.notificationconfig.create",
            &["roles/securitycenter.settingsEditor", "roles/securitycenter.admin"],
        )],
        &["securitycenter.googleapis.com"],
    ),
    (
        "google_scc_v2_organization_scc_big_query_export",
        &[org(
            "securitycenter.bigQueryExports.create",
            &["roles/securitycenter.settingsEditor", "roles/securitycenter.admin"],
        )],
        &["securitycenter.googleapis.com"],
    ),
    ("google_secret_manager_secret", &[project("secretmanager.secrets.create", &["roles/secretmanager.admin"])], &["secretmanager.googleapis.com"]),
    ("google_service_account", &[project("iam.serviceAccounts.create", &["roles/iam.serviceAccountAdmin"])], &["iam.googleapis.com"]),
    ("google_service_account_iam_member", &[project("iam.serviceAccounts.setIamPolicy", &["roles/iam.serviceAccountAdmin"])], &["iam.googleapis.com"]),
    ("google_storage_bucket", &[project("storage.buckets.create", &["roles/storage.admin"])], &["storage.googleapis.com"]),
    ("google_storage_bucket_iam_member", &[project("storage.buckets.setIamPolicy", &["roles/storage.admin"])], &["storage.googleapis.com"]),
    // Resource Manager tags. The key and its values are organisation-scoped; a binding
    // attaches a value to one resource and is the exemption itself, which is why it is
    // listed separately: an estate may be allowed to define the vocabulary without being
    // allowed to hand out exemptions with it.
    ("google_tags_tag_binding", &[org("resourcemanager.tagValueBindings.create", &["roles/resourcemanager.tagUser"])], &["cloudresourcemanager.googleapis.com"]),
    ("google_tags_tag_key", &[org("resourcemanager.tagKeys.create", &["roles/resourcemanager.tagAdmin"])], &["cloudresourcemanager.googleapis.com"]),
    ("google_tags_tag_value", &[org("resourcemanager.tagValues.create", &["roles/resourcemanager.tagAdmin"])], &["cloudresourcemanager.googleapis.com"]),
    (
        "google_tags_tag_value_iam_member",
        &[org("resourcemanager.tagValues.setIamPolicy", &["roles/resourcemanager.tagAdmin"])],
        &["cloudresourcemanager.googleapis.com"],
    ),
];

/// The table's role entries for one resource type.
pub(crate) fn entries_for(tf_type: &str) -> Option<&'static [Entry]> {
    TYPES.iter().find(|(t, _, _)| *t == tf_type).map(|(_, e, _)| *e)
}

/// The APIs one resource type is served by. Every call the provider makes for it
/// is billed to the infra project (`user_project_override`), so these have to be
/// enabled THERE whatever the resource's own scope is — a budget hangs off the
/// billing account and an org policy off the organisation, and both still need
/// their API on the project being billed for the call.
pub(crate) fn apis_for(tf_type: &str) -> Option<&'static [&'static str]> {
    TYPES.iter().find(|(t, _, _)| *t == tf_type).map(|(_, _, a)| *a)
}

/// The whole table as data — what `--format json` prints and
/// `scripts/check_prerequisites.py` reads.
pub(crate) fn table_json() -> serde_json::Value {
    let types: BTreeMap<&str, serde_json::Value> = TYPES
        .iter()
        .map(|(t, e, a)| (*t, serde_json::json!({ "roles": e, "apis": a })))
        .collect();
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
    for (t, entries, apis) in TYPES {
        out.push_str(&format!("{}:\n", t));
        entries.iter().for_each(|e| out.push_str(&line(e)));
        out.push_str(&format!("  {:<56} {}\n", "(api)", apis.join(", ")));
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

/// One API an estate's emitted resources are served by, and the types that put it
/// there. `declared` is whether a `google_project_service` in the estate enables it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct ApiNeed {
    pub api: String,
    /// the resource types that need it, sorted
    pub reason: Vec<String>,
    pub declared: bool,
}

/// The APIs the estate's emitted types need, each marked with whether the INFRA
/// project enables it. That project is the one that counts whatever the resource's
/// own scope is: every provider block carries `user_project_override` with
/// `billing_project = infra_project_name`, so Google bills the call there and wants
/// the API enabled there. A pack that enables an API on a project of its own has
/// not satisfied this — that project needs it for its own calls, which is the
/// pack's business; the billed project is the estate's.
pub(crate) fn apis(manifest: &Manifest, infra_project: &str) -> Vec<ApiNeed> {
    let declared: BTreeSet<&str> = manifest
        .of_type("google_project_service")
        .filter(|r| manifest.project_of(r).as_deref() == Some(infra_project))
        .filter_map(|r| r.attrs.get("service").map(String::as_str))
        .collect();
    let mut by_api: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for t in manifest.resources.values().map(|r| r.tf_type.as_str()).collect::<BTreeSet<_>>() {
        for api in apis_for(t).unwrap_or(&[]) {
            by_api.entry(api).or_default().insert(t.to_string());
        }
    }
    by_api
        .into_iter()
        .map(|(api, reason)| ApiNeed {
            declared: declared.contains(api),
            api: api.to_string(),
            reason: reason.into_iter().collect(),
        })
        .collect()
}

/// Every service the infra project declares, in the estate and in the packs it
/// uses. This is what `bootstrap` enables imperatively before `tofu` runs: the
/// race it dodges is the same one the emitter's ordering pass handles inside an
/// apply, and on day 0 there is no apply yet.
pub(crate) fn declared_apis(manifest: &Manifest, infra_project: &str) -> Vec<String> {
    manifest
        .of_type("google_project_service")
        .filter(|r| manifest.project_of(r).as_deref() == Some(infra_project))
        .filter_map(|r| r.attrs.get("service").cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The APIs the infra project does not enable.
pub(crate) fn missing_apis(manifest: &Manifest, infra_project: &str) -> Vec<ApiNeed> {
    apis(manifest, infra_project).into_iter().filter(|a| !a.declared).collect()
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
    /// the IaC service account and the directory customer (`C0…`), for Groups Admin
    pub service_account: Option<String>,
    pub customer: Option<String>,
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
        // Not an IAM role, so testIamPermissions cannot see it: the Admin SDK can, when
        // the login carries its scope. Read-only here — `migrate --mode cloud` assigns it.
        match &probe.service_account {
            Some(sa) => {
                let customer = probe.customer.as_deref().unwrap_or("my_customer");
                match crate::gcp::workspace::groups_admin(customer, sa, false).await {
                    crate::gcp::workspace::GroupsAdmin::Held | crate::gcp::workspace::GroupsAdmin::Assigned => check.tested += 1,
                    crate::gcp::workspace::GroupsAdmin::NotDone(why) => {
                        check.not_tested.push(format!("Groups Admin (Google Workspace, not an IAM role): {}", why))
                    }
                }
            }
            None => check.not_tested.push("Groups Admin (Google Workspace): the estate names no IaC service account".to_string()),
        }
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
            // An appended billing block grants on `billing_account_infra`; an estate
            // that binds none says nothing about which billing account its projects use.
            None if block == "google_billing_account_iam_member"
                && params.get("billing_account_infra").is_none_or(|v| v.is_empty()) =>
            {
                return Err(format!(
                    "{}: the resource types need {} on a billing account, and the estate binds no \
                     billing_account_infra to grant it on — nothing was written",
                    estate.display(),
                    roles.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            None => append_block(&out, block, roles),
        };
        written.extend(roles.iter().map(|r| format!("{} in {}", r, block)));
    }
    crate::fsx::write_edited_satz(estate, &text, &out).map_err(|e| e.to_string())?;
    Ok(written)
}

/// Write the APIs into the estate's own `google_project` block for the infra
/// project: into its `project_service` list when it has one, else a list created
/// right after `project_id`. The list is the estate's record of what is switched
/// on, and it is where the emitter derives `google_project_service.<label>_<service>`
/// from — the address the CIS pack claims 5.0 §2.14 against — so this only ever
/// ADDS, in the list's own order.
///
/// It refuses rather than guesses: an estate that binds no `infra_project_name`,
/// or whose infra project is declared somewhere other than this file (a pack, most
/// likely), is named and nothing is written. A splice into a pristine pack would be
/// overwritten by the next `merge-presets`.
pub(crate) fn write_apis(
    estate: &Path,
    params: &HashMap<String, String>,
    apis: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if apis.is_empty() {
        return Ok(Vec::new());
    }
    let infra = params.get("infra_project_name").filter(|v| !v.is_empty()).ok_or_else(|| {
        format!(
            "{}: the estate binds no infra_project_name, so there is no project to enable {} on — \
             nothing was written",
            estate.display(),
            apis.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;
    let text = std::fs::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let out = add_to_service_list(&text, infra, params, apis).ok_or_else(|| {
        format!(
            "{}: no `google_project` block in this file declares project_id {} — the infra project \
             is declared elsewhere (a pack is never edited: the next merge-presets would overwrite \
             it). Add these to its project_service list by hand: {}",
            estate.display(),
            infra,
            apis.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;
    crate::fsx::write_edited_satz(estate, &text, &out).map_err(|e| e.to_string())?;
    Ok(apis.iter().map(|a| format!("{} on {}", a, infra)).collect())
}

/// Splice the services into the `project_service` list of the `google_project`
/// block whose `project_id` is this project — the value read after `{param}`
/// interpolation, and after a bare param reference, since `project_id =
/// infra_project_name` is how every estate the template writes names it. `None`
/// when no block in this text declares that project.
fn add_to_service_list(
    text: &str,
    project: &str,
    params: &HashMap<String, String>,
    apis: &BTreeSet<String>,
) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // find the `project_id` line that names this project, then the block it is in
    let mut target: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let Some(value) = t.strip_prefix("project_id").and_then(|r| r.trim_start().strip_prefix('=')) else {
            continue;
        };
        let value = value.trim();
        let named = match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            Some(literal) => interpolate(literal, params) == project,
            // a bare param reference: `project_id = infra_project_name`
            None => params.get(value).is_some_and(|v| v == project),
        };
        if named {
            target = Some(i);
            break;
        }
    }
    let at = target?;
    let indent: String = lines[at].chars().take_while(|c| c.is_whitespace()).collect();

    // the list, if this block already has one: from `project_id` to the end of its
    // block, the first `project_service = [` at the same depth
    let mut depth: i32 = 0;
    let mut list: Option<usize> = None;
    for (i, line) in lines.iter().enumerate().skip(at) {
        let t = line.trim();
        if t.starts_with("project_service") && t.contains('[') && depth == 0 {
            list = Some(i);
            break;
        }
        for c in t.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        if depth < 0 {
            break;
        }
    }

    match list {
        Some(i) if lines[i].trim_end().trim_end_matches(',').ends_with(']') => {
            // one line: `project_service = ["a", "b"]`
            let close = lines[i].rfind(']')?;
            let inner = lines[i][..close].trim_end().to_string();
            let sep = if inner.ends_with('[') { "" } else { ", " };
            let add = apis.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", ");
            lines[i] = format!("{}{}{}{}", inner, sep, add, &lines[i][close..]);
        }
        Some(i) => {
            // multi-line: insert before the closing `]`, in the list's own order
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim_start().starts_with(']') {
                j += 1;
            }
            if j == lines.len() {
                return None;
            }
            let item_indent: String = if j > i + 1 {
                lines[i + 1].chars().take_while(|c| c.is_whitespace()).collect()
            } else {
                format!("{}  ", indent)
            };
            if j > i + 1 && !lines[j - 1].trim_end().ends_with(',') && !lines[j - 1].trim().is_empty() {
                lines[j - 1].push(',');
            }
            for (k, a) in apis.iter().enumerate() {
                lines.insert(j + k, format!("{}\"{}\",", item_indent, a));
            }
        }
        None => {
            // no list at all: write one under `project_id`
            let mut block = vec![format!("{}project_service = [", indent)];
            block.extend(apis.iter().map(|a| format!("{}  \"{}\",", indent, a)));
            block.push(format!("{}]", indent));
            for (k, l) in block.into_iter().enumerate() {
                lines.insert(at + 1 + k, l);
            }
        }
    }
    let mut s = lines.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    Some(s)
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
    s.push_str("\n\n// The IaC service account's roles for this estate's resource types (`satz update-prerequisites`).\n");
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

    /// The API column is hand-kept, and a typo in it is invisible: the estate
    /// declares a service Google does not have and the apply fails on the one it
    /// does. Cloud Asset Inventory namespaces every asset type by the service that
    /// serves it (`bigquery.googleapis.com/Dataset`), and
    /// `presets/import-config.yaml` carries that column for 841 types — refreshed
    /// from Google's own list, never by hand. Where both know a type, they must
    /// agree.
    #[test]
    fn the_api_column_agrees_with_the_import_table() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/import-config.yaml");
        let cfg: crate::config::ImportConfig =
            serde_yaml::from_str(&std::fs::read_to_string(&path).expect("the import table")).expect("parses");
        let mut checked = 0;
        for (t, _, apis) in TYPES {
            let Some(asset) = cfg.resource_types.get(*t).and_then(|r| r.asset_type.as_deref()) else { continue };
            if asset.starts_with("TODO") {
                continue;
            }
            let Some((host, _)) = asset.split_once('/') else { continue };
            assert!(
                apis.contains(&host),
                "{}: the import table serves it from {} and the row names {:?} — one of the two is wrong",
                t,
                host,
                apis
            );
            checked += 1;
        }
        assert!(checked >= 40, "only {} rows could be cross-checked", checked);
    }

    #[test]
    fn the_table_is_well_formed_and_sorted() {
        let mut seen = BTreeSet::new();
        let mut last = "";
        for (t, entries, apis) in TYPES {
            assert!(seen.insert(*t), "{} listed twice", t);
            assert!(*t > last, "{} is out of order", t);
            last = t;
            assert!(!entries.is_empty(), "{} has no entry", t);
            assert!(!apis.is_empty(), "{} names no API", t);
            for e in *entries {
                assert!(!e.roles.is_empty(), "{}: an entry names no role", t);
                assert_eq!(e.permission.is_none(), e.scope == Scope::Workspace, "{}: only a Workspace entry has no permission", t);
                if e.scope != Scope::Workspace {
                    assert!(e.roles.iter().all(|r| r.starts_with("roles/")), "{}: {:?}", t, e.roles);
                }
            }
        }
        let json = table_json();
        assert!(json["types"]["google_project"]["roles"].as_array().is_some_and(|a| a.len() == 4));
        assert_eq!(json["types"]["google_billing_budget"]["apis"][0], "billingbudgets.googleapis.com");
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

    fn apis(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    /// The infra project is named by a bare param reference in every estate the
    /// template writes, and by a literal in one somebody hand-wrote. Both resolve.
    #[test]
    fn an_api_is_spliced_into_the_infra_projects_service_list() {
        let src = "estate e\n\ngoogle_folder {\n  infra_folder {\n    google_project {\n      infra {\n        project_id      = infra_project_name\n        project_service = [\n          \"cloudasset.googleapis.com\",\n          \"iam.googleapis.com\",\n        ]\n      }\n    }\n  }\n}\n";
        let edited = add_to_service_list(src, "corp-infra-001", &params(), &apis(&["monitoring.googleapis.com"])).unwrap();
        assert!(
            edited.contains("          \"iam.googleapis.com\",\n          \"monitoring.googleapis.com\",\n        ]"),
            "{}",
            edited
        );
        // a literal project id, and a one-line list
        let literal = "google_project {\n  infra {\n    project_id = \"corp-infra-001\"\n    project_service = [\"iam.googleapis.com\"]\n  }\n}\n";
        let edited = add_to_service_list(literal, "corp-infra-001", &params(), &apis(&["storage.googleapis.com"])).unwrap();
        assert!(edited.contains("= [\"iam.googleapis.com\", \"storage.googleapis.com\"]"), "{}", edited);
    }

    /// An estate whose infra project has no list at all gets one, under the line
    /// that names the project.
    #[test]
    fn a_project_without_a_service_list_gets_one() {
        let src = "google_project {\n  infra {\n    project_id      = infra_project_name\n    billing_account = billing_account_infra\n  }\n}\n";
        let edited = add_to_service_list(src, "corp-infra-001", &params(), &apis(&["billingbudgets.googleapis.com"])).unwrap();
        assert!(
            edited.contains(
                "    project_id      = infra_project_name\n    project_service = [\n      \"billingbudgets.googleapis.com\",\n    ]\n"
            ),
            "{}",
            edited
        );
    }

    /// Another project's list is not the infra project's, and a file that declares
    /// neither is not edited at all — the caller names the file to fix by hand.
    #[test]
    fn a_project_that_is_not_the_infra_project_is_never_edited() {
        let other = "google_project {\n  logsink {\n    project_id      = \"corp-logs-001\"\n    project_service = [\n      \"logging.googleapis.com\",\n    ]\n  }\n}\n";
        assert!(add_to_service_list(other, "corp-infra-001", &params(), &apis(&["monitoring.googleapis.com"])).is_none());
    }

    #[test]
    fn write_apis_refuses_an_estate_with_no_infra_project_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("satz-apis-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let estate = dir.join("e.satz");
        let src = "estate e\n\ngoogle_project {\n  other {\n    project_id = \"corp-other-001\"\n  }\n}\n";
        std::fs::write(&estate, src).unwrap();
        // the param is bound, but no block in this file declares that project
        let err = write_apis(&estate, &params(), &apis(&["monitoring.googleapis.com"])).unwrap_err();
        assert!(err.contains("corp-infra-001") && err.contains("monitoring.googleapis.com"), "{}", err);
        assert_eq!(std::fs::read_to_string(&estate).unwrap(), src, "nothing may be written");
        // and with no infra project bound at all, it says that instead
        let err = write_apis(&estate, &HashMap::new(), &apis(&["monitoring.googleapis.com"])).unwrap_err();
        assert!(err.contains("infra_project_name"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refuses_a_billing_block_with_no_billing_account_to_name() {
        let dir = std::env::temp_dir().join(format!("satz-iac-billing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let estate = dir.join("e.satz");
        let src = "estate e\n\ngoogle_organization_iam_member {\n  \"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\" = [\n    \"roles/viewer\",\n  ]\n}\n";
        std::fs::write(&estate, src).unwrap();
        let err = write_grants(&estate, &params(), SA, &roles(&["roles/browser"]), &roles(&["roles/billing.admin"])).unwrap_err();
        assert!(err.contains("billing_account_infra") && err.contains("roles/billing.admin"), "{}", err);
        assert_eq!(std::fs::read_to_string(&estate).unwrap(), src, "nothing may be written");
        // with the billing account bound, the block is appended
        let mut with_billing = params();
        with_billing.insert("billing_account_infra".into(), "01AA-BB-CC".into());
        write_grants(&estate, &with_billing, SA, &roles(&["roles/browser"]), &roles(&["roles/billing.admin"])).unwrap();
        let written = std::fs::read_to_string(&estate).unwrap();
        assert!(written.contains("billing_account_id = billing_account_infra") && written.contains("\"roles/browser\","), "{}", written);
        let _ = std::fs::remove_dir_all(&dir);
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
