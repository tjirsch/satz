# Preset library

Every pack is a `.satz` file: `pack <name> version "<v>"`, a `params { … }` block
of overridable defaults, and the resources it contributes. Estates `use` them.

Presets are **read-only building blocks**: use them from a customer's estate and
set every org-specific value there — never by editing a preset.

`use "presets/<pack>.satz"` at top level for packs that declare their own
resource-type maps, under a key (`use … as google_org_policy_policy`) or inside a
resource map for content packs. Pack `params` are overridable defaults; define
the same name in the estate `params` block to override (the using document
always wins). When a needed customization is not expressible as a param, fork:
copy to `<pack>.local.satz`, repoint the `use` — `merge-presets` maintains the
`.diff.satz` adoption ledger. **Rule of thumb: a fork whose whole diff could be a
param is upstream debt — lift the param into the pack instead** (that is how
`allowed_policy_member_*` and `essential_contacts_email` came to exist).

Optional packs are gated on a single param: `use "presets/x.satz" when
logsink_project_name` — undefined or falsy skips the pack entirely (no resources,
no params, no claims).

Multi-resource-type packs (marked below) rely on **hoisted scopes**: org/customer/
billing-scoped types (`google_cloud_identity_group`, `google_organization_iam_member`,
`google_billing_account_iam_member`) may sit anywhere in the tree and are emitted
once at their intrinsic scope. Projects land wherever the pack is used — root or
inside a `google_folder { … }` block. Two packs contributing to the same
resource-type map merge label by label (the ⊕ fold); the same label with a
different body is a hard error naming both files.

**Adopting what already exists** (groups, org policies, folders, the state bucket
…) is `satz adopt <estate>` — it resolves live ids and `--execute` writes them
back as `"import-id"`. Pack headers that mention adoption mean that command.

---

## monitoring/organization-audit-logsink.satz

Organization-wide audit trail: enables Data Access audit logs for **all** services
org-wide, creates its own destination project + GCS archive bucket, and routes all Cloud
Audit Logs from every current and future project into that bucket via an aggregated
org-level sink (project owners cannot bypass it). Self-contained — multi-resource-type.

**Use** (root, or inside a folder block to place the project there):

```
google_folder {
  shared_services {
    display_name = "Shared Services"
    use "presets/monitoring/organization-audit-logsink.satz" when logsink_project_name
  }
}
```

**Required from the estate:** `customer_organization_id`, `billing_account_infra`,
`customer_shortname`, `default_region`

**Overridable defaults** (names are derived from `customer_shortname`, so they are
globally unique without overrides):

| Param | Default | Meaning |
|---|---|---|
| `logsink_project_name` | `"{customer_shortname}-log-infra-001"` | project_id of the destination project |
| `logsink_bucket_name` | `"{customer_shortname}-organization-audit-logs"` | GCS archive bucket |
| `logsink_bucket_location` | `default_region` | bucket region |
| `logsink_retention_days` | `400` | lifecycle delete age |
| `logsink_name` | `"{customer_shortname}-organization-audit-gcs"` | display name of the sink |
| `logsink_filter` | the four Cloud Audit log streams | sink filter — extend to archive more (e.g. VPC flow logs), never narrow below the audit streams |

**Notes:**
Retention lock (`retention_policy.is_locked`) is deliberately not set; see the preset
header. DATA_READ org-wide can be voluminous — measure a week before pruning.

## monitoring/ — CIS 2.5–2.12 (log metrics + alerts)

Two variants of the same eight controls. **Prefer the central one**; the per-project file is
the exception, not the default. Both may coexist — the control passes as soon as either path
is satisfied.

| | central | per project |
|---|---|---|
| File | `organization-cis-log-alerts-central.satz` | `project-cis-log-alerts.satz` |
| Covers | every project in the org, current and future | one named project |
| Resources | 1 logging bucket, 1 sink, 8 metrics, 8 policies, 1 channel | 8 metrics, 8 policies, 1 channel — **per project** |
| New project | covered on creation, no config change | needs its own `use`, or it silently fails 2.5–2.12 |
| Recipients | one org-wide channel; per-*control* routing possible | own channel per project |
| Cost | audit logs stored twice (GCS archive + logging bucket) | none beyond the metrics |

### monitoring/organization-cis-log-alerts-central.satz

One Cloud Logging bucket, a second organization sink into it, and eight **bucket-scoped**
metrics with alert policies — covering the whole organization. Multi-resource-type.

**Use** (root level):

```
use "presets/monitoring/organization-cis-log-alerts-central.satz" when cis_central_bucket_project
```

**Required from the estate:** `customer_organization_id`, `customer_domain`,
`customer_shortname`, `default_region`

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `cis_central_bucket_project` | `"{customer_shortname}-organization-log-alerts"` | project hosting bucket, metrics, policies, channel |
| `cis_central_bucket_id` | `"{customer_shortname}-organization-log-alerts"` | Cloud Logging bucket id |
| `cis_central_bucket_location` | `default_region` | bucket location |
| `cis_central_bucket_retention_days` | `30` | short on purpose — the archive lives in GCS |
| `cis_central_sink_name` | `"cis-central-metrics-sink"` | second org sink |
| `cis_central_email` | `"gcp-security@{customer_domain}"` | recipient — a FULL address since pack v1.2, any domain. The mailbox must exist and receive external mail (Monitoring sends from alerting-noreply@google.com); a group whose members have no mailboxes silently drops everything |
| `cis_central_channel_name` | `"CIS Security Alerts (org)"` | channel display name |
| `cis_central_alert_window` | `"300s"` | alert alignment period |

**How the credit works.** Prowler's CIS metric checks are written per-project, but it credits
a child project when an org sink with `include_children` routes its logs to a Cloud Logging
bucket carrying a matching bucket-scoped metric with an alert
(`logging_service.get_projects_covered_by_aggregated_metric`). Eight metrics therefore cover
the whole organization.

**Why a second sink is needed — "bucket" means two different products.** A **GCS bucket**
(`storage.googleapis.com/<name>`) is Cloud Storage: the sink drops hourly JSON files into a
folder path, and to Cloud Logging those are files, not logs — no metric can count them. A
**Logging bucket** (`logging.googleapis.com/projects/…/buckets/…`) is a container *inside*
Cloud Logging: entries stay indexed, searchable in Log Explorer, and usable by bucket-scoped
metrics. Every project already has `_Default` and `_Required`.

Two controls demand different destinations, hence both sinks:

- **CIS 2.3/2.4** — `cloudstorage_bucket_log_retention_policy_lock` only inspects sinks whose
  destination contains `storage.googleapis.com`, then tests `retention_policy.is_locked` on
  that GCS bucket. A Logging-bucket sink is never examined. (Logging buckets *can* be locked
  too — they have a `locked` field; GCS is required by the check's wording, not by a missing
  capability.)
- **CIS 2.5–2.12** — log-based metrics require a Logging bucket.

GCS is the tamper-evident archive, the logging bucket the queryable surface for metrics and
alerting. Audit logs are therefore stored twice — keep the logging bucket's retention short.

**Check your Prowler version first.** The credit depends on
`get_projects_covered_by_aggregated_metric` and on org-level sink collection
(`_get_org_sinks`), both recent additions. Older versions do not see organization sinks in
these checks: the central setup would not be credited, and CIS 2.3/2.4 return no result even
though a GCS sink and bucket exist.

```bash
prowler --version
python3 -c "
from prowler.providers.gcp.services.logging import logging_service as m
print('org-sinks:', hasattr(m.Logging, '_get_org_sinks'))
print('central-credit:', hasattr(m, 'get_projects_covered_by_aggregated_metric'))"
```

Two `True` → good. Otherwise upgrade, or use the per-project variant until then.

**Do not narrow the sink filter.** The credit is only granted when the filter provably carries
the Admin Activity stream: empty, `all`, or OR-combined Cloud Audit selectors. A single `AND`,
`NOT` or `!=` forfeits it — for *every* project at once.

**Notes:** enable `logging.googleapis.com` and `monitoring.googleapis.com` in the logging
project. Verify the credit after apply by running Prowler: 2.5–2.12 must pass for **every**
scanned project, not only the logging project — if only that one passes, the sink filter or
destination is wrong. Freshly created logging buckets can 404 the metrics for a minute
(propagation); a plain re-apply resolves stragglers.

### Smoke test — prove the pipeline end to end

One alert proves the whole chain (org sink → central logging bucket → bucket-scoped
metric → policy → email channel), so trigger the *cheapest* control: **§2.7 custom role
changes**. No VPC, no API enablement, zero infrastructure footprint:

```bash
# any project of the org works — the sink is org-wide with include_children
gcloud iam roles create smoke_test_2_7 --project=<any-project> \
  --permissions=resourcemanager.projects.get --title="smoke test"
# expect the email at the channel recipient within ~5–10 minutes, then clean up
# (the delete fires §2.7 again — a free second sample):
gcloud iam roles delete smoke_test_2_7 --project=<any-project>
```

Org-level `--organization=<org-id>` also works if the caller holds
`iam.organizationRoleAdmin` (the IaC service account does — impersonating it is an
equally valid test).

If no email arrives in ~15 minutes, trace the stages:

```bash
# 1. did the audit event happen?
gcloud logging read 'protoPayload.methodName="google.iam.admin.v1.CreateRole"' \
  --project=<any-project> --freshness=15m --format='value(timestamp)'
# 2. did the org sink deliver it into the central alerts bucket?
gcloud logging read 'protoPayload.methodName="google.iam.admin.v1.CreateRole"' \
  --bucket=<cis_central_bucket_id> --location=<cis_central_bucket_location> \
  --view=_AllLogs --project=<cis_central_bucket_project> --freshness=15m \
  --format='value(timestamp)'
# 3. stages 1+2 fine but no mail → check the channel address is real and
#    mail-enabled, and look for an incident on the policy in Cloud Monitoring.
```

The §2.8 (firewall) variant needs a project with the compute API enabled and an
existing VPC: `gcloud compute firewall-rules create smoke-test-2-8 --project=<p>
--network=<vpc> --action=deny --rules=tcp:9999 --source-ranges=192.0.2.0/24`, then
delete it. Do **not** enable the compute API just for the test — with the CIS §1.1
domain locks live, first-time API enablement can trip on the service-agent
auto-grant (see the CIS pack notes); §2.7 tests the identical pipeline.

### monitoring/project-cis-log-alerts.satz

CIS GCP Foundations **2.5 – 2.12** for one project: eight log-based metric filters, one
alert policy each, and the email notification channel they fire into. Multi-resource-type.

**Use** (root level — the target project comes from the resources' `project` attribute,
not from where the `use` sits):

```
use "presets/monitoring/project-cis-log-alerts.satz" when cis_alert_project
```

**Required from the estate:** `customer_domain`, `infra_project_name`

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `cis_alert_project` | `"{infra_project_name}"` | project hosting metrics, policies and channel |
| `cis_alert_email_local` | `"gcp-security"` | local part of the recipient group address |
| `cis_alert_channel_name` | `"CIS Security Alerts"` | channel display name |
| `cis_alert_window` | `"300s"` | alert alignment period |

**One project per use — no parameterisation of labels.** The resource labels are fixed,
so using the pack twice folds the same addresses with different bodies — a hard error.
For a second project either fork the pack (`.local.satz`) and prefix every label and
metric `name`, or use the central variant above.

**Alerts cannot go to Essential Contacts.** That is Google's channel for notifying the
customer (security bulletins, billing, suspension), not a Cloud Monitoring channel — alert
policies cannot target it. Use the *same group mailbox* in both systems instead: one inbox,
both sources.

**Notification channels are project resources.** `google_monitoring_notification_channel`
lives in a project and an alert policy can only reference channels from its *own* project —
there is no org-level channel and no cross-project reference. N projects therefore mean N
channels pointing at the same mailbox, N×8 metrics and N×8 policies, and every new project
needs the pack again or it silently fails 2.5–2.12.

**Notes:** enable `monitoring.googleapis.com` (and `logging.googleapis.com`) in the target
project's `project_service` list. The recipient group must exist in Cloud Identity before
apply — Google accepts unverified email channels, but they stay silent. Filter strings are
compared by *substring* against Prowler's expectation: reformatting a filter keeps the
alert working but silently breaks the compliance check — see the preset header for the
source of truth and the end-to-end test. Prowler also never checks whether a policy has a
recipient; `notification_channels = []` passes CIS and notifies nobody.

## security-group-models/

The S1 security group model: five admin groups plus their org-level role grants.

- **s1-group-definitions.satz** — the groups (`gcp-organization-admins`,
  `gcp-project-admins`, `gcp-security-admins`, `gcp-security-viewers`,
  `gcp-billing-admins`), each with the IaC service account as owner. Lifecycle
  ignores `initial_group_config` (imported groups always diff on it).
  **Since v1.1 the pack ships NO human memberships** — presets define groups,
  humans grant membership (console or gcloud, deliberately unmanaged). Estates
  that must manage a membership declare it on their own estate-level groups.
- **s1-group-permissions.satz** — the `google_organization_iam_member` grants for
  those groups plus a domain-wide `organizationViewer`.

**Use:**

```
google_cloud_identity_group { use "presets/security-group-models/s1-group-definitions.satz" }
google_organization_iam_member { use "presets/security-group-models/s1-group-permissions.satz" }
```

To adopt groups (and their declared members) that already exist in the tenant, run
`satz adopt <estate> --only google_cloud_identity_group,google_cloud_identity_group_membership`
— each group is looked up by email, each declared member by email in that group, and
`--execute` writes the verified ids back as `"import-id"`. Members not declared in the
estate stay unmanaged.

**Required from the estate:** `customer_domain`, `first_admin`, `svc_iac_account`,
`infra_project_name`

**Overridable defaults:** the five `gcp_*_name` group names
(`gcp_organization_admins_name`, `gcp_project_admins_name`, `gcp_security_admins_name`,
`gcp_security_viewers_name`, `gcp_billing_admins_name`).

## security-audit/sa-security-audit.satz

Read-only security-audit service account + impersonation group + org-level IAM in one
pack (Security Toolset §6.6). Access is impersonation-only: no SA keys,
no remediation rights (`roles/viewer`, `iam.securityReviewer`,
`securitycenter.adminViewer`, `cloudasset.viewer`; auditors group gets
`serviceAccountTokenCreator`). Multi-resource-type.

**Use** (root level):

```
use "presets/security-audit/sa-security-audit.satz"
```

**Required from the estate:** `customer_domain`, `first_admin`

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `security_audit_sa_project` | `""` | **effectively required** — project hosting the SA |
| `security_audit_sa_name` | `"sa-security-audit"` | SA account_id |
| `security_audit_sa_display_name` | `"Security Audit (read-only)"` | |
| `security_audit_auditors_group` | `"grp-security-auditors"` | impersonation group |

**Notes:** enable `iamcredentials.googleapis.com` in the SA's project manually after
apply — impersonation fails without it.

## CIS-GCP-Foundation-4.0.satz

The CIS GCP Foundation 4.0 organization-policy set as `google_org_policy_policy`
resources — managed constraints included. Since pack v1.1–v1.3 the §1.1
Domain Restricted Sharing pair is **parameterized with compliant defaults**: every
estate is locked to its own directory/org out of the box, and cross-org needs are
visible one-line overrides — never forks.

**§1.1 params (v1.3):**

| Param | Default | Meaning |
|---|---|---|
| `allowed_policy_member_customers` | `[customer_id]` | `iam.allowedPolicyMemberDomains`: DIRECTORY customer ids (`C0…`) whose identities may be granted IAM roles. **Never DNS domain names.** |
| `allowed_policy_member_principal_sets` | own org (`//cloudresourcemanager.googleapis.com/organizations/<org-id>`) | `iam.managed.allowedPolicyMembers`: principal sets allowed past the managed lock |
| `allowed_policy_member_subjects` | `[]` | individual principals past both locks — typically Google SYSTEM service accounts that org-level products grant roles to (Firebase Hosting `firebase-hosting@system…`, SCC premium agents `service-org-<id>@gcp-sa-*-hpsa…` / `@security-center-api…`) |
| `essential_contacts_allowed_domain` | `customer_domain` | domain the Essential Contacts constraint allows |

**The two lists must stay consistent**: a directory allowed by
`allowed_policy_member_customers` needs its org in
`allowed_policy_member_principal_sets` too, or grants to its members pass the first
policy and are blocked by the second. Example (a lab org administered by the parent
org's staff):

```
allowed_policy_member_customers      = [customer_id, "C0bolt002"]
allowed_policy_member_principal_sets = [
  "//cloudresourcemanager.googleapis.com/organizations/{customer_organization_id}",
  "//cloudresourcemanager.googleapis.com/organizations/123456789012",
]
```

**Operational caveat — service-agent auto-grants.** With the legacy domains lock
enforced, enabling a NEW Google API can fail when Google auto-grants the service
agent (P4SA) its role: the managed constraint exempts service agents, the legacy one
does not. If it bites: add the agent to `allowed_policy_member_subjects`, or lift
the domains policy for the enablement and re-apply.

### SCC activation under the §1.1 locks (optional documented step)

Activating Security Command Center (premium/enterprise) grants org-level roles to
**four Google service accounts**:

```
service-org-<org-id>@security-center-api.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-csc-hpsa.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-dspm-hpsa.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-ee-hpsa.iam.gserviceaccount.com
```

With the CIS §1.1 locks enforced these grants are blocked. The canonical exceptions
on the **managed** constraint are expressed via the pack param:

```
allowed_policy_member_subjects = [
  "serviceAccount:service-org-<org-id>@security-center-api.iam.gserviceaccount.com",
  "serviceAccount:service-org-<org-id>@gcp-sa-csc-hpsa.iam.gserviceaccount.com",
  "serviceAccount:service-org-<org-id>@gcp-sa-dspm-hpsa.iam.gserviceaccount.com",
  "serviceAccount:service-org-<org-id>@gcp-sa-ee-hpsa.iam.gserviceaccount.com",
]
```

**Known limitation: this is NOT sufficient.** The legacy
`iam.allowedPolicyMemberDomains` constraint accepts only directory customer ids —
individual subjects cannot be whitelisted there, and the Google service accounts do
not belong to the customer's directory. Activation attempts still fail with the
subjects in place (observed 2026-08, estate 1). Working procedure until this is resolved
upstream:

1. Set the subjects param (above) and apply — covers the managed constraint.
2. **Temporarily lift the domains lock:** override
   `allowed_policy_member_customers` with a rule allowing all (or
   `gcloud org-policies delete iam.allowedPolicyMemberDomains --organization=<id>`),
   apply.
3. Activate SCC; verify the four grants exist
   (`gcloud organizations get-iam-policy <id> | grep -A1 hpsa`).
4. Re-tighten: restore the customers param, apply. Existing grants persist —
   the constraint gates new bindings, not existing ones.

CONFIRMED (2026-08, estate 1): temporarily disabling the **legacy domains policy is
required** — no exception shape exists on that constraint. AND the subjects
exceptions alone do not unblock activation even so: either their member format is
wrong (note the two spellings in the wild — `serviceAccount:<email>` vs
`principal://iam.googleapis.com/projects/-/serviceAccounts/<email>`; the managed
constraint may require one specific form) **or a third policy needs tuning**
(candidates: `iam.managed.preventPrivilegedBasicRolesForDefaultServiceAccounts`,
`iam.automaticIamGrantsForDefaultServiceAccounts`). OPEN: reproduce with the exact
activation error, fix the exception format or identify the third constraint, then
harden the pack/docs so the lift window is the only manual part.

### Turning the SCC services on — `scripts/scc-enable-all.sh`

Service (module) enablement has **no provider resource**, so it cannot be
expressed in a preset at all; neither can tier activation. What IS codeable —
custom modules, sources + source IAM, notification configs, BigQuery exports,
mute configs, Security Posture — is everything DOWNSTREAM of this step.

`scripts/scc-enable-all.sh` is that step: every service `ENABLED` at the org,
every folder and project below it `INHERITED`. Dry run by default.

```bash
scripts/scc-enable-all.sh --organization 123456789012              # dry run
scripts/scc-enable-all.sh --organization 123456789012 --apply      # write
```

Expect the §1.1 interaction above — each newly enabled service provisions
another service agent whose auto-grant the domain lock refuses; the script says
so instead of printing the raw API error. Flags, failure modes and the rest:
[`docs/scripts.md`](../docs/scripts.md).

**Use**, then adopt what the organisation already has (`satz adopt --activate`
activates managed constraints via the Org Policy API and imports existing policies
into state — see "Adopting what already exists" in the main README):

```
google_org_policy_policy { use "presets/CIS-GCP-Foundation-4.0.satz" }
```
```bash
satz adopt C0example.satz --only google_org_policy_policy --activate --execute --import
```

**Required from the estate:** `customer_organization_id`, `customer_id`, `customer_domain`

## billing-account-permissions.satz

Billing-account IAM: grants for the billing admins group and the IaC service account
on the billing account (declares its own `google_billing_account_iam_member` map,
pinned to `billing_account_infra`).

**Use** (root level): `use "presets/billing-account-permissions.satz"`

**Required from the estate:** `billing_account_infra`, `customer_domain`,
`svc_iac_account`, `infra_project_name`

## organization-budget.satz

A global budget (1000 EUR, thresholds at 50/80/100% of current spend) on the infra
billing account (declares its own `google_billing_budget` map).

**Use** (root level): `use "presets/organization-budget.satz"`

**Required from the estate:** `billing_account_infra`

**Notes:** contains a placeholder `"import-id"` for adopting an existing budget — remove
it for a fresh budget, or replace it with the real budget id (`satz adopt` cannot resolve
budgets yet: they are matched by display name and need the Budgets API). Not yet migrated
to param-driven defaults.

## essential-contacts-organization.satz

One organization-level Essential Contact subscribed to ALL notification categories.
A content pack: use it inside the resource map.

**Use:**

```
google_essential_contacts_contact { use "presets/essential-contacts-organization.satz" }
```

**Required from the estate:** `customer_organization_id`, `customer_domain`

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `essential_contacts_email` | `"essential-contacts-all@{customer_domain}"` | the contact address |

## catalogs/

Compliance catalogs (`cis-gcp-4.0.yaml`, `cis-gcp-5.0.yaml`): control ids with this
project's own paraphrases, read by `require` and `report-compliance`. YAML data, not
packs.

## import-config.yaml

Not a pack: the configuration `satz import` reads — an optional `root` (organization,
folder by id or display-name path, project) and `only` list, and per resource type the
import filter (`import`, `asset_type`, attribute include/exclude) **plus the adoption
rules `satz adopt` reads** — `import_id` templates for user-chosen ids, `match_on` keys
for GCP-assigned ones, `activate: managed` for org policies. A type without a rule is
reported by `adopt` as "no rule"; adding one is a one-line change here. Referenced
automatically from `presets_dir`, or explicitly via `--import-config`.
