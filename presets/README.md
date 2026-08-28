# Preset library

**Single-source packs:** packs with a `.satz` sibling are AUTHORED in Satz; their
`.yaml` and `.claims.yaml` files are BUILT artifacts (`satz build-packs`) carrying a
GENERATED header — edit the `.satz`, never the twins. YAML estates keep including the
same `.yaml` filenames as always; Satz estates `use` the `.satz` directly.

Presets are **read-only building blocks**: include them from a customer's estate and
set every org-specific value there — never by editing a preset.

**Satz estates (the standard since v0.29):** `use "presets/<pack>.satz"` — at top level
for packs that declare their own resource-type maps, under a key
(`use … as org_policy_policy`) or inside a resource map for content packs. Pack `params`
are overridable defaults; define the same name in the estate `params` block to override
(the using document always wins). When a needed customization is not expressible as a
param, fork: copy to `<pack>.local.satz`, repoint the `use` — `merge-presets` maintains
the `.diff.satz` adoption ledger. **Rule of thumb: a fork whose whole diff could be a
param is upstream debt — lift the param into the pack instead** (that is how
`allowed_policy_member_*` and `essential_contacts_email` came to exist).

For YAML estates, customization flows through two mechanisms (satz >= v0.14.0):

- **Required anchors**: the preset references anchors it does not define (e.g.
  `*customer-organization-id`). The main file must define them in its `variables:`
  block *before* the include line, or the transpile fails with an unknown-anchor error.
- **Overridable defaults**: the preset's own `variables:` block holds defaults.
  Define the same anchor in the main file (before the include) to override —
  first definition wins. Undefined = default applies. Full rules, composition via
  `!format`, and the anchor-of-an-alias pitfall: see "Overriding variable defaults"
  in the main README.

Optional presets can be gated on a single variable:

```yaml
!include-if logsink-project-name presets/monitoring/organization-audit-logsink.yaml
```

If `logsink-project-name` is not defined in the main file, the whole preset is skipped.

Multi-resource-type presets (marked below) rely on **hoisted scopes**: org/customer/
billing-scoped types (`cloud_identity_group`, `organization_iam_member`,
`google_billing_account_iam_member`) may sit anywhere in the tree and are emitted once at
their intrinsic scope. Projects land wherever the file is included —
root or inside a `folder:` block. Two presets declaring the same top-level resource-type
key (both defining a `google_logging_organization_sink:`, say) merge id by id when
provider schemas are present — see "Hoisted scopes" in the main README.

---

## monitoring/organization-audit-logsink.yaml

Organization-wide audit trail: enables Data Access audit logs for **all** services
org-wide, creates its own destination project + GCS archive bucket, and routes all Cloud
Audit Logs from every current and future project into that bucket via an aggregated
org-level sink (project owners cannot bypass it). Self-contained — multi-resource-type.

**Include** (root, or inside a folder block to place the project there):

```yaml
folder:
  shared-services:
    display_name: "Shared Services"
    !include-if logsink-project-name presets/monitoring/organization-audit-logsink.yaml
```

**Required from main:** `*customer-organization-id`, `*billing-account-infra`,
`*customer-shortname`, `*default-region`

**Overridable defaults** (names are derived from `*customer-shortname`, so they are
globally unique without overrides):

| Anchor | Default | Meaning |
|---|---|---|
| `logsink-project-name` | `<shortname>-log-infra-001` | project_id of the destination project |
| `logsink-bucket-name` | `<shortname>-organization-audit-logs` | GCS archive bucket |
| `logsink-bucket-location` | `<default-region>` | bucket region |
| `logsink-retention-days` | `400` | lifecycle delete age |
| `logsink-name` | `<shortname>-organization-audit-gcs` | display name of the sink |
| `logsink-filter` (`logsink_filter`) | the four Cloud Audit log streams | sink filter — extend to archive more (e.g. VPC flow logs), never narrow below the audit streams |

**Notes:**
Retention lock (`retention_policy.is_locked`) is deliberately not set; see the preset
header. DATA_READ org-wide can be voluminous — measure a week before pruning.

## monitoring/ — CIS 2.5–2.12 (log metrics + alerts)

Two variants of the same eight controls. **Prefer the central one**; the per-project file is
the exception, not the default. Both may coexist — the control passes as soon as either path
is satisfied.

| | central | per project |
|---|---|---|
| File | `organization-cis-log-alerts-central.yaml` | `project-cis-log-alerts.yaml` |
| Covers | every project in the org, current and future | one named project |
| Resources | 1 logging bucket, 1 sink, 8 metrics, 8 policies, 1 channel | 8 metrics, 8 policies, 1 channel — **per project** |
| New project | covered on creation, no config change | needs its own include, or it silently fails 2.5–2.12 |
| Recipients | one org-wide channel; per-*control* routing possible | own channel per project |
| Cost | audit logs stored twice (GCS archive + logging bucket) | none beyond the metrics |

### monitoring/organization-cis-log-alerts-central.yaml

One Cloud Logging bucket, a second organization sink into it, and eight **bucket-scoped**
metrics with alert policies — covering the whole organization. Multi-resource-type.

**Include** (root level):

```yaml
!include-if cis-central-bucket-project presets/monitoring/organization-cis-log-alerts-central.yaml
```

**Required from main:** `*customer-organization-id`, `*customer-domain`,
`*logsink-project-name` (reuse the project from `organization-audit-logsink.yaml`)

**Overridable defaults:**

| Anchor | Default | Meaning |
|---|---|---|
| `cis-central-bucket-project` | `*logsink-project-name` | project hosting bucket, metrics, policies, channel |
| `cis-central-bucket-id` | `cis-audit-metrics` | Cloud Logging bucket id |
| `cis-central-bucket-location` | `europe-west3` | bucket location |
| `cis-central-bucket-retention-days` | `30` | short on purpose — the archive lives in GCS |
| `cis-central-sink-name` | `cis-central-metrics-sink` | second org sink |
| `cis-central-email` (`cis_central_email`) | `gcp-security@<customer-domain>` | recipient — a FULL address since pack v1.2, any domain. The mailbox must exist and receive external mail (Monitoring sends from alerting-noreply@google.com); a group whose members have no mailboxes silently drops everything |
| `cis-central-channel-name` | `CIS Security Alerts (org)` | channel display name |
| `cis-central-alert-window` | `300s` | alert alignment period |

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
  --bucket=<cis-central-bucket-id> --location=<cis-central-bucket-location> \
  --view=_AllLogs --project=<cis-central-bucket-project> --freshness=15m \
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

### monitoring/project-cis-log-alerts.yaml

CIS GCP Foundations **2.5 – 2.12** for one project: eight log-based metric filters, one
alert policy each, and the email notification channel they fire into. Multi-resource-type.

**Include** (root level — the target project comes from the resources' `project:` field,
not from where the include sits):

```yaml
!include-if cis-alert-project presets/monitoring/project-cis-log-alerts.yaml
```

**Required from main:** `*customer-domain`

**Overridable defaults:**

| Anchor | Default | Meaning |
|---|---|---|
| `cis-alert-project` | `*infra-project-name` | project hosting metrics, policies and channel |
| `cis-alert-email-local` | `gcp-security` | local part of the recipient group address |
| `cis-alert-channel-name` | `CIS Security Alerts` | channel display name |
| `cis-alert-window` | `300s` | alert alignment period |

**One project per include — no parameterisation.** The YAML keys are fixed, so including
the file twice collides (duplicate keys). For a second project either copy the file and
prefix every key and metric `name`, or use the central variant above.

**Alerts cannot go to Essential Contacts.** That is Google's channel for notifying the
customer (security bulletins, billing, suspension), not a Cloud Monitoring channel — alert
policies cannot target it. Use the *same group mailbox* in both systems instead: one inbox,
both sources.

**Notification channels are project resources.** `google_monitoring_notification_channel`
lives in a project and an alert policy can only reference channels from its *own* project —
there is no org-level channel and no cross-project reference. N projects therefore mean N
channels pointing at the same mailbox, N×8 metrics and N×8 policies, and every new project
needs the include again or it silently fails 2.5–2.12.

**Notes:** enable `monitoring.googleapis.com` (and `logging.googleapis.com`) in the target
project's `project_service` list. The recipient group must exist in Cloud Identity before
apply — Google accepts unverified email channels, but they stay silent. Filter strings are
compared by *substring* against Prowler's expectation: reformatting a filter keeps the
alert working but silently breaks the compliance check — see the preset header for the
source of truth and the end-to-end test. Prowler also never checks whether a policy has a
recipient; `notification_channels: []` passes CIS and notifies nobody.

## security-group-models/

The S1 security group model: five admin groups plus their org-level role grants.

- **s1-group-definitions.yaml** — the groups (`gcp-organization-admins`,
  `gcp-project-admins`, `gcp-security-admins`, `gcp-security-viewers`,
  `gcp-billing-admins`), each with the IaC service account as owner. Lifecycle
  ignores `initial_group_config` (imported groups always diff on it).
  **Since v1.1 the pack ships NO human memberships** — presets define groups,
  humans grant membership (console or gcloud, deliberately unmanaged). Estates
  that must manage a membership declare it on their own estate-level groups.
- **s1-group-permissions.yaml** — the `organization_iam_member` grants for
  those groups plus a domain-wide `organizationViewer`.

**Include:**

```yaml
cloud_identity_group: !include presets/security-group-models/s1-group-definitions.yaml
organization_iam_member: !include presets/security-group-models/s1-group-permissions.yaml
```

To adopt groups (and their declared members) that already exist in the tenant, use
`!import-include` instead of `!include` on the definitions line for one transpile —
each group is looked up by email and imported into state. Members not listed in the
YAML stay unmanaged.

**Required from main:** `*customer-domain`, `*first-admin`, `*svc-iac-account`,
`*infra-project-name`

**Overridable defaults:** the five `gcp-*-name` group names.

## security-audit/sa-security-audit.yaml

Read-only security-audit service account + impersonation group + org-level IAM in one
fragment (YAML form of Security Toolset §6.6). Access is impersonation-only: no SA keys,
no remediation rights (`roles/viewer`, `iam.securityReviewer`,
`securitycenter.adminViewer`, `cloudasset.viewer`; auditors group gets
`serviceAccountTokenCreator`). Multi-resource-type.

**Include** (root level):

```yaml
!include presets/security-audit/sa-security-audit.yaml
```

**Required from main:** `*customer-domain`, `*first-admin`

**Overridable defaults:**

| Anchor | Default | Meaning |
|---|---|---|
| `security-audit-sa-project` | `""` | **effectively required** — project hosting the SA |
| `security-audit-sa-name` | `sa-security-audit` | SA account_id |
| `security-audit-sa-display-name` | `Security Audit (read-only)` | |
| `security-audit-auditors-group` | `grp-security-auditors` | impersonation group |

**Notes:** enable `iamcredentials.googleapis.com` in the SA's project manually after
apply — impersonation fails without it.

## CIS-GCP-Foundation-4.0.yaml

The CIS GCP Foundation 4.0 organization-policy set as `google_org_policy_policy`
fragments — managed constraints included. Since pack v1.1–v1.3 the §1.1
Domain Restricted Sharing pair is **parameterized with compliant defaults**: every
estate is locked to its own directory/org out of the box, and cross-org needs are
visible one-line overrides — never forks.

**§1.1 params (v1.3):**

| Param | Default | Meaning |
|---|---|---|
| `allowed_policy_member_customers` | `[customer_id]` | `iam.allowedPolicyMemberDomains`: DIRECTORY customer ids (`C0…`) whose identities may be granted IAM roles. **Never DNS domain names.** |
| `allowed_policy_member_principal_sets` | own org (`//cloudresourcemanager.googleapis.com/organizations/<org-id>`) | `iam.managed.allowedPolicyMembers`: principal sets allowed past the managed lock |
| `allowed_policy_member_subjects` | `[]` | individual principals past both locks — typically Google SYSTEM service accounts that org-level products grant roles to (Firebase Hosting `firebase-hosting@system…`, SCC premium agents `service-org-<id>@gcp-sa-*-hpsa…` / `@security-center-api…`) |

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

**Include** (the `!import-include` form is the main workflow: it activates managed
constraints via the Org Policy API and imports existing policies into state — see the
"Organization Policy Alignment" section of the main README):

```yaml
org_policy_policy: !import-include presets/CIS-GCP-Foundation-4.0.yaml
# after the first successful transpile+apply, switch back to:
org_policy_policy: !include presets/CIS-GCP-Foundation-4.0.yaml
```

**Required from main:** `*customer-organization-id`, `*customer-id`, `*customer-domain`

**Overridable defaults:** none.

## billing-account-permissions.yaml

Billing-account IAM: grants for the billing admins group and the IaC service account
on the billing account (declares its own `google_billing_account_iam_member:` key).

**Include** (root level): `!include presets/billing-account-permissions.yaml`

**Required from main:** `*billing-account-infra`, `*customer-domain`,
`*svc-iac-account`, `*infra-project-name`

## organization-budget.yaml

A global budget (1000 EUR, thresholds at 50/80/100% of current spend) on the infra
billing account (declares its own `billing_budget:` key).

**Include** (root level): `!include presets/organization-budget.yaml`

**Required from main:** `*billing-account-infra`

**Notes:** contains a placeholder `import-id:` for adopting an existing budget — remove
it for a fresh budget, or replace it with the real budget id. (Not yet migrated to
variable-driven defaults.)

## essential-contacts-organization.yaml

One organization-level Essential Contact (`essential-contacts-all@<customer-domain>`)
subscribed to ALL notification categories.

**Include** (Form B under the resource type):

```yaml
google_essential_contacts_contact: !include presets/essential-contacts-organization.yaml
```

**Required from main:** `*customer-organization-id`, `*customer-domain`

## discovery-config.yaml

Not an includable preset: the resource-type filter configuration consumed by the
`discover-from-*` commands (which asset types to ingest, per-type attribute
include/exclude). Referenced automatically from `presets_dir`, or explicitly via
`--discovery-config`.
