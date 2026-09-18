# satz library

Every pack is a `.satz` file: `pack <name> version "<v>"`, a `params { … }` block
of overridable defaults, and the resources it contributes. Estates `use` them.

Presets are **read-only building blocks**: use them from a customer's estate and
set every org-specific value there — never by editing a preset.

`use "presets/<pack>.satz"` at top level for packs that declare their own
resource-type maps — the CIS baseline and its extensions, and most of the library —
or inside a resource map for the content packs that are a bare list of labels
(`google_essential_contacts_contact { use … }`, or `use … as <type>` written flat).
Each pack's header states its own line and `presets/docs/` prints it; the shapes are
not interchangeable, and using one the wrong way round is refused rather than
emitted. Pack `params` are overridable defaults; define
the same name in the estate `params` block to override (the using document
always wins). When a needed customization is not expressible as a param, fork:
copy to `<pack>.local.satz`, repoint the `use` — `merge-presets` maintains the
`.diff.satz` adoption ledger. **If a fork's whole diff could be a param, add the
param to the pack instead.**

Optional packs are gated on a single param: `use "presets/x.satz" when
logsink_project_id` — a falsy value skips the pack entirely (no resources, no
params, no claims). The param must be DECLARED somewhere (`params { … }` of the
estate or a pack): a `when` on a param nobody declares is an error, not `false`.

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

**Writing one:** `satz review-pack <file> --format text --out -` judges a pack
against everything on this page — it parses, it is formatted, its header opens with a
sentence the index can print, its version has a changelog row below, it declares no
membership, it runs no legacy constraint beside its managed replacement, every resource
type it emits has a row in satz's prerequisite table, and it compiles inside an estate —
and says what adopting it would cost that estate in roles and APIs. It warns on a type
`satz adopt` has no rule for. It is the same bar
this repository's gates hold, reachable without a checkout.

**Per-pack reference:** [`docs/README.md`](docs/README.md) — one page per pack,
derived from the pack file by `satz doc-packs`: what it does, the
copy-pasteable `use` block with its params and the params it needs from outside,
the resources, the claims with their control titles from the catalog, the duties,
which contributed resources no claim witnesses, and the pack's own version
history. Hand-written notes live in each page's notes region. **History:** [the changelog](#changelog) at the foot of this page, one
row per pack version.

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
    use "presets/monitoring/organization-audit-logsink.satz" when logsink_project_id
  }
}
```

**Overridable defaults** (names are derived from `customer_shortname`, so they are
globally unique without overrides):

| Param | Default | Meaning |
|---|---|---|
| `logsink_project_id` | `"{customer_shortname}-log-infra-001"` | project_id of the destination project |
| `logsink_bucket_name` | `"{customer_shortname}-organization-audit-logs"` | GCS archive bucket |
| `logsink_bucket_location` | `default_region` | bucket region |
| `logsink_retention_days` | `400` | lifecycle delete age |
| `logsink_name` | `"{customer_shortname}-organization-audit-gcs"` | display name of the sink |
| `logsink_filter` | the four Cloud Audit log streams | sink filter — extend to archive more (e.g. VPC flow logs), never narrow below the audit streams |

**Questions.** The project, the bucket, its location and the retention are asked;
each is a recreate or an irreversible deletion if changed later. The sink name and the
filter are technical defaults. `satz interview <estate> --accept-defaults` binds all
four once `customer_shortname` and `default_region` are answered — the names derive
from them, and a derived default is offered only when its inputs are in.

**Notes:**
Retention lock (`retention_policy.is_locked`) is not set: a lock cannot be undone, so
the claim carries a duty to set it once the pipeline is validated. DATA_READ
org-wide can be voluminous — measure a week of volume before narrowing it.

## monitoring/ — CIS 2.5–2.12 (log metrics + alerts)

The eight alert controls are numbered **§2.5–2.12 in CIS 5.0** and **§2.4–2.11 in
CIS 4.0** (5.0 inserted a new §2.2; 4.0's §2.12 is DNS logging). Resource labels
and this document use the 5.0 numbers; each pack claims both versions with the
right id, and `doc-packs --check` refuses a claim on an id its catalog lacks.

Two variants of the same eight controls; **the central one is the default**. Both may
coexist: a control passes when either path is satisfied.

| | central | per project |
|---|---|---|
| File | `organization-cis-log-alerts-central.satz` | `project-cis-log-alerts.satz` |
| Covers | every project in the org, current and future | one named project |
| Resources | 1 logging bucket, 1 sink, 8 metrics, 8 policies, 1 channel | 8 metrics, 8 policies, 1 channel — **per project** |
| New project | covered on creation, no config change | needs its own `use`; without it the project fails 2.5–2.12 |
| Recipients | one org-wide channel; per-*control* routing possible | own channel per project |
| Cost | audit logs stored twice (GCS archive + logging bucket) | none beyond the metrics |

### monitoring/organization-cis-log-alerts-central.satz

One Cloud Logging bucket, a second organization sink into it, and eight **bucket-scoped**
metrics with alert policies — covering the whole organization. Multi-resource-type.

**Use** (root level):

```
use "presets/monitoring/organization-cis-log-alerts-central.satz" when cis_central_bucket_project
```

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `cis_central_bucket_project` | `logsink_project_id` | project hosting bucket, metrics, policies, channel: the audit-logsink pack's own project, **by reference** — an estate using both packs sets nothing |
| `cis_central_bucket_id` | `"{customer_shortname}-organization-log-alerts"` | Cloud Logging bucket id |
| `cis_central_bucket_location` | `default_region` | bucket location |
| `cis_central_bucket_retention_days` | `30` | short, because the archive lives in GCS |
| `cis_central_sink_name` | `"cis-central-metrics-sink"` | second org sink |
| `cis_central_email` | `"gcp-security@{customer_domain}"` | recipient — a full address, any domain. The mailbox must exist and receive external mail (Monitoring sends from alerting-noreply@google.com); a group whose members have no mailboxes drops every alert |
| `cis_central_channel_name` | `"CIS Security Alerts (org)"` | channel display name |
| `cis_central_alert_window` | `"300s"` | alert alignment period |

**Questions.** `cis_central_email` — the mailbox must exist and accept external mail,
or every alert is dropped — and `cis_central_bucket_project`, which defaults to the
logsink pack's project by reference. The rest are technical defaults and are not
asked.

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

**Prowler version.** The credit needs a Prowler that has
`get_projects_covered_by_aggregated_metric` and org-level sink collection
(`_get_org_sinks`). A Prowler without them does not see organization sinks in these
checks: the central setup is not credited, and CIS 2.3/2.4 return no result although a
GCS sink and bucket exist. Check both:

```bash
prowler --version
python3 -c "
from prowler.providers.gcp.services.logging import logging_service as m
print('org-sinks:', hasattr(m.Logging, '_get_org_sinks'))
print('central-credit:', hasattr(m, 'get_projects_covered_by_aggregated_metric'))"
```

Both must print `True`; otherwise upgrade Prowler or use the per-project variant.

**Keep the sink filter wide.** Prowler credits the sink only when the filter provably
carries the Admin Activity stream: empty, `all`, or OR-combined Cloud Audit selectors. A
filter with an `AND`, `NOT` or `!=` loses the credit for every project.

**Notes:** enable `logging.googleapis.com` and `monitoring.googleapis.com` in the logging
project. Verify the credit after apply by running Prowler: 2.5–2.12 must pass for **every**
scanned project, not only the logging project — if only that one passes, the sink filter or
destination is wrong. A freshly created logging bucket can answer 404 to the metrics for
about a minute while it propagates; a second apply creates the metrics that failed.

### Smoke test — prove the pipeline end to end

One alert tests the whole chain (org sink → central logging bucket → bucket-scoped
metric → policy → email channel). **§2.7, custom role changes,** needs no VPC and no API
enablement, and leaves nothing behind:

```bash
# any project of the org works — the sink is org-wide with include_children
gcloud iam roles create smoke_test_2_7 --project=<any-project> \
  --permissions=resourcemanager.projects.get --title="smoke test"
# expect the email at the channel recipient within ~5–10 minutes, then clean up
# (the delete fires §2.7 again — a free second sample):
gcloud iam roles delete smoke_test_2_7 --project=<any-project>
```

Org-level `--organization=<org-id>` also works if the caller holds
`iam.organizationRoleAdmin`; the IaC service account holds it, so impersonating it works
too.

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
delete it. Do not enable the compute API for this test: under the CIS §1.1 lock,
first-time API enablement can fail on the service agent's role grant
([service agents](#cis-gcp-foundation-40satz)), and §2.7 tests the same pipeline.

### monitoring/project-cis-log-alerts.satz

CIS GCP Foundations **2.5 – 2.12** for one project: eight log-based metric filters, one
alert policy each, and the email notification channel they fire into. Multi-resource-type.

**Use** (root level — the target project comes from the resources' `project` attribute,
not from where the `use` sits):

```
use "presets/monitoring/project-cis-log-alerts.satz" when cis_alert_project
```

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `cis_alert_project` | `"{infra_project_name}"` | project hosting metrics, policies and channel |
| `cis_alert_email_local` | `"gcp-security"` | local part of the recipient group address |
| `cis_alert_channel_name` | `"CIS Security Alerts"` | channel display name |
| `cis_alert_window` | `"300s"` | alert alignment period |

**Questions.** `cis_alert_project` (one project per use) and `cis_alert_email_local`
(the mailbox rule above). The channel name and window are not asked.

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
needs the pack again; without it the project fails 2.5–2.12.

**Notes:** enable `monitoring.googleapis.com` (and `logging.googleapis.com`) in the target
project's `project_service` list. The recipient group must exist in Cloud Identity before
apply — Google accepts unverified email channels, but sends nothing to them. Filter
strings are compared by *substring* against Prowler's expectation: a reformatted filter
keeps the alert working and fails the compliance check — the preset header has the
expected strings and the end-to-end test. Prowler does not check whether a policy has a
recipient; `notification_channels = []` passes CIS and notifies nobody.

## estate-core.satz

The questions every estate has to answer on day 0, with the params they answer — the
sixteen `satz init` writes, each with a `question`: what to ask, why,
and what changing the answer later costs. Which packs make up the estate is the next
pack, `estate-map.satz`; this one is the day-0 params and nothing else.

```
use "presets/estate-core.satz"
```

The pack **emits nothing**; it holds the day-0 questions. `satz interview <estate>
--create` and the MCP tool `satz_interview` write an estate that uses it, with every
question open. An estate written by `init` does not need it: `init` binds every param
from its flags, and a bound param is an answered question.

Two kinds of param, and the interview treats them differently:

| kind | params | in the report |
|---|---|---|
| **no possible default** | `customer_id`, `customer_organization_id`, `customer_domain`, `customer_shortname`, `customer_longname`, `first_admin`, `billing_account_infra` | `blocking: true` — a value has to be typed |
| **derived or conventional** | `infra_folder_name`, `infra_project_name`, `infra_bucket_name`, `svc_iac_account`, `svc_iac_users_group`, `deployment_engine`, `deployment_mode`, `default_region`, `default_zone`, the security model | `default` offered — accepting it is an answer, recorded by writing it |

A derived default is offered only once what it derives from is answered:
`infra_project_name` is `"{customer_shortname}-infra-001"`, which with the short name
still open would be `-infra-001`, so it blocks until the short name is typed, then
offers `acme-infra-001`. See [satz interview](../docs/interview.md).

## estate-map.satz

Which packs make up the estate, asked as questions — the map an interview follows
after the day-0 params. One boolean per optional pack plus the S1/S2 model as a
`question oneof`, each with what the pack is for and what turning it off later
destroys. The map declares the choices and nothing else; the estate carries one
`use … when` line per choice, in the map's order — `satz interview --create` writes
them, and a test keeps the two lists equal ([ADR 0007](../docs/adr/0007-the-map-is-a-pack-of-choices-and-the-estate-carries-the-lines.md)).

```
use "presets/estate-core.satz"
use "presets/estate-map.satz"
use "presets/security-group-models/s1-security-groups.satz" when security_model_s1
use "presets/security-group-models/s2-security-groups.satz" when security_model_s2
use "presets/billing-account-permissions.satz" when use_billing_permissions
…
```

| choice | default | pack |
|---|---|---|
| `use_cis_baseline` | on | `cis/CIS-GCP-Foundation-4.0` — the thirty organisation policies the estate exists for; its opt-in extensions and their dry-run twins are the baseline's own questions |
| `security_model` (oneof) | S1 | `security-group-models/s1-security-groups` or `s2-security-groups` |
| `use_audit_logsink` | on | `monitoring/organization-audit-logsink`, in the infrastructure folder |
| `use_central_alerts` | on | `monitoring/organization-cis-log-alerts-central`, beside it — needs the archive |
| `use_billing_permissions` | on | `billing-account-permissions` |
| `use_essential_contacts` | on | `essential-contacts-organization` |
| `use_budget` | off | `organization-budget` |
| `use_scc_enablement` | off | `scc/scc-service-enablement` — recommended; the three below are asked only when it is on |
| `use_scc_notifications` | off | `scc/scc-notifications` — the Pub/Sub chain findings travel on |
| `use_scc_findings_mail` | off | `scc/scc-findings-mail` — asked only when the topic is on: the subscription, mailbox and alert that tell somebody |
| `use_scc_findings_siem` | off | `scc/scc-findings-siem` — asked with it: the connector's own subscription and the grant it reads with. Both may be on |
| `use_scc_export` | off | `scc/scc-export` — the BigQuery dataset findings are kept in |
| `use_security_audit_sa` | off | `security-audit/sa-security-audit` |
| `use_defender` | off | `integrations/microsoft-defender-for-cloud` — its plan fragments by hand |
| `use_sentinel` | off | `integrations/microsoft-sentinel` — the federation half |
| `use_sentinel_auditlogs` | follows `use_sentinel` | `integrations/microsoft-sentinel-auditlogs`, asked only when Sentinel is on |
| `use_sentinel_network_logs` | follows `use_sentinel` | `integrations/microsoft-sentinel-network-logs` — flow logs, firewall, DNS, NAT: free until the feature is enabled |
| `use_verification_runner` | off | `ci/verification-runner` and its grant, the customer-hosted shape |
| `use_exemption_tag` | off | `exemptions/exemption-tag` — the tag an exemption is bound to; it exempts nothing on its own |

The CIS baseline is the map's first choice: the skeleton writes its line commented under
its phase, like every pack's, and answering `use_cis_baseline` yes puts it in. It is
asked rather than assumed because its thirty policies reach the organisation in one
apply. **Not on the map:** the per-project alert pack, Defender's plan fragments, the
MSP-hosted runner shape; each is wired by hand, and the map's header says so.

## security-group-models/

**This is where STANDING authority is modelled** — who administers projects, networks,
guardrails, billing, org-wide and continuously. Its counterpart is
[exemptions/](#exemptions), which says who may make a narrow, named EXCEPTION to a
control this authority set, without holding this authority. Keeping the two apart is
deliberate: `gcp-security-admins` holds `roles/orgpolicy.policyAdmin` and can rewrite any
policy, and that is a much bigger thing to hand someone than "may let one service account
hold a key".

The security group models: admin groups plus their org-level role grants.
Two spellings of S1 exist — an estate takes ONE of them, never both:

- **s1-security-groups.satz** — S1 in ONE typed file (groups AND grants), for
  a top-level `use`. Resource-type sections may repeat across files with
  distinct ids, so the pack's `google_cloud_identity_group { … }` sits beside
  the estate's own.
- **s1-group-definitions.satz** + **s1-group-permissions.satz** — the same S1
  as two content packs `use`d UNDER a resource type.
- **s2-security-groups.satz** — S2: S1 plus a distinct **`gcp-network-admins`**
  group. The network authority moves out of project-admins (which lose
  `compute.networkAdmin` and `compute.xpnAdmin`) into a team that owns VPCs,
  Shared VPC, firewall policies, Cloud DNS, hybrid connectivity and network
  diagnostics — `compute.networkAdmin`, `compute.xpnAdmin`,
  `compute.securityAdmin`, `dns.admin`, `networkconnectivity.hubAdmin`,
  `networkmanagement.admin`, plus viewer roles; no owner, no IAM admin.
  One file, since groups and their grants belong together.

**Which model:** S1 when one platform team does both project administration
and networking (the network roles ride along in project-admins because the
people are the same). S2 as soon as network and project administration are
different people — a connectivity team, Shared VPC with many service
projects, hierarchical firewall policies, hybrid connectivity, or a
separation-of-duties requirement; the network team gets org-wide reach over
connectivity and nothing else. Moving S1 → S2 later is a pack swap plus
adopting the new group, but every project admin loses two roles at that
apply. The role-by-role table and who-sits-where guide are on the
[S2 pack page](docs/s2-security-groups.md).

The groups: `gcp-organization-admins` (break-glass owners of the org tree,
policies and org IAM), `gcp-project-admins` (day-2 workload projects),
`gcp-security-admins` (guardrails: org policies, SCC, folder IAM, log
routing), `gcp-security-viewers` (org-wide read-only for audit and
compliance evidence), `gcp-billing-admins` (billing, budgets, procurement)
— S2 adds `gcp-network-admins` (everything that connects, no ownership, no
IAM) — each with the IaC service account as owner; lifecycle
ignores `initial_group_config` (imported groups always diff on it). **No pack
ships human memberships** — presets define groups, humans grant membership
(in the console or with gcloud, outside satz). Estates that must manage a
membership declare it on their own estate-level groups. Every group name is a
param.

**Use:**

```
// one typed file (S1 or S2)
use "presets/security-group-models/s2-security-groups.satz"

// or the two S1 content packs under their resource types
google_cloud_identity_group { use "presets/security-group-models/s1-group-definitions.satz" }
google_organization_iam_member { use "presets/security-group-models/s1-group-permissions.satz" }
```

To adopt groups (and their declared members) that already exist in the tenant, run
`satz adopt <estate> --only google_cloud_identity_group,google_cloud_identity_group_membership`
— each group is looked up by email, each declared member by email in that group, and
`--execute` writes the verified ids back as `"import-id"`. Members not declared in the
estate stay unmanaged.

**Overridable defaults:** the five `gcp_*_name` group names
(`gcp_organization_admins_name`, `gcp_project_admins_name`, `gcp_security_admins_name`,
`gcp_security_viewers_name`, `gcp_billing_admins_name`).

**Questions.** Every group name is asked, because a group's address is its identity:
renaming one later is a new group, members moved by hand and roles re-granted. Each has
its conventional default, so `satz interview <estate> --accept-defaults` settles a model
in one pass. Which model — S1 or S2 — is the `security_model` choice in
`estate-map.satz`, not a question in either model pack: a question that gates a pack
cannot live in the pack it gates.

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

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `security_audit_sa_project` | `""` | **effectively required** — project hosting the SA |
| `security_audit_sa_name` | `"sa-security-audit"` | SA account_id |
| `security_audit_sa_display_name` | `"Security Audit (read-only)"` | |
| `security_audit_auditors_group` | `"grp-security-auditors"` | impersonation group |

**Notes:** enable `iamcredentials.googleapis.com` in the SA's project manually after
apply — impersonation fails without it.

**Questions.** The hosting project (no default — it blocks), the account id and the auditors group are asked; each is a recreate. The display name is not asked.

## CIS-GCP-Foundation-4.0.satz

The CIS GCP Foundation 4.0 organization-policy set as `google_org_policy_policy`
resources, managed constraints included, with the superseded legacy twins declared
off ([below](#superseded-legacy-constraints)). §1.1 Domain Restricted Sharing is the
managed `iam.managed.allowedPolicyMembers` alone, **parameterized with compliant
defaults**: every estate is locked to its own organization, and a cross-org need is a
one-line param override in the estate.

**Params:**

| Param | Default | Meaning |
|---|---|---|
| `allowed_policy_member_principal_sets` | own org (`//cloudresourcemanager.googleapis.com/organizations/<org-id>`) | §1.1: principal sets whose members may be granted IAM roles |
| `allowed_policy_member_subjects` | Security Command Center's five organization-level service agents | §1.1: individual principals allowed past the lock — Google service accounts that org-level products grant roles to (SCC's `service-org-<id>@gcp-sa-*-hpsa…` / `@security-center-api…`, Firebase Hosting `firebase-hosting@system…`). An override replaces the list: keep the five when adding |
| `essential_contacts_allowed_domains` | `["@{customer_domain}"]` | domains the Essential Contacts constraint allows, each entry `@domain` |
| `allowed_resource_locations` | `["in:eu-locations", "in:us-locations"]` | `gcp.resourceLocations` (§2): where resources may be created. Narrow it here instead of forking |

**Questions.** Thirteen are asked: the ten opt-in controls (each can break a running
workload) and the three lists that say where and who — `allowed_resource_locations`,
`allowed_policy_member_principal_sets`, `allowed_policy_member_subjects`. An estate
using the pack has to bind all thirteen before `bootstrap` or `transpile --apply` run;
`satz interview <estate> --accept-defaults` binds every default in one pass, since
none of the thirteen needs a typed value. The technical defaults (protocol-forwarding
schemes, the contacts domain) are not asked. See [satz interview](../docs/interview.md).

**Cross-org grants** need the other organization in
`allowed_policy_member_principal_sets`, beside the estate's own. Example (a lab org
administered by the parent org's staff):

```
allowed_policy_member_principal_sets = [
  "//cloudresourcemanager.googleapis.com/organizations/{customer_organization_id}",
  "//cloudresourcemanager.googleapis.com/organizations/123456789012",
]
```

**Service agents.** Google grants roles to its service agents when a product is
activated or an API is enabled for the first time. An agent that lives outside the
estate's organization is not in its principal set, so the §1.1 lock refuses the grant
unless `allowed_policy_member_subjects` names it. To allow another agent, add it to
the param — keeping the five SCC agents — and apply. The legacy
`iam.allowedPolicyMemberDomains` cannot name a principal as an exception; Google's
remedy there is to disable the constraint, grant, and re-enable it, so the pack
declares it off.

### SCC activation under the §1.1 lock

Activating Security Command Center Premium — the tier this library assumes — grants organization-level
roles to five Google service agents, which `allowed_policy_member_subjects` names by
default:

```
service-org-<org-id>@security-center-api.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-csc-hpsa.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-dspm-hpsa.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-ee-hpsa.iam.gserviceaccount.com
service-org-<org-id>@gcp-sa-ktd-hpsa.iam.gserviceaccount.com
```

They live in Google tenant projects, not in the customer's organization. Each address
derives from the org id inside a Google-owned domain, so no one else can create an
identity that matches.

An organization where the legacy `iam.allowedPolicyMemberDomains` is still set refuses
these grants whatever the param says, because that constraint has no exception for a
principal. Apply the pack before activating: it resets the legacy policy. Where that
policy already exists, `satz adopt --only google_org_policy_policy --execute --import`
imports it first ([Superseded legacy constraints](#superseded-legacy-constraints)).

### Turning the SCC services on — `presets/scc/scc-enable-all.sh`

Service (module) enablement has **no provider resource**, so no preset can express
it; neither can tier activation. Everything downstream of this step has provider
resources: custom modules, sources and source IAM, v2 notification configs, BigQuery
exports, mute configs, Security Posture.

`presets/scc/scc-enable-all.sh` is that step: every service `ENABLED` at the org,
every folder and project below it `INHERITED`. Dry run by default.

```bash
presets/scc/scc-enable-all.sh --organization 123456789012              # dry run
presets/scc/scc-enable-all.sh --organization 123456789012 --apply      # write
```

**`scc/scc-service-enablement.satz` binds it**, so an estate does not retype the
org id — one `use` is the whole thing:

```
use "presets/scc/scc-service-enablement.satz"
```

```bash
satz run-actions estate.satz              # print the resolved command line, run nothing
satz run-actions estate.satz --check      # the dry run above
satz run-actions estate.satz --execute    # adds --apply
```

The pack has **no resources**. The script sits beside the pack rather than in
`scripts/` because `get-presets` downloads `presets/**` and nothing else, and the
action must find its script in the estate's copy of the presets.

### `scc/scc-notifications.satz` — findings out of the console

The first of the downstream resources: where findings GO. A notification is a chain
of three, and two of them sit outside the notification config:

```
use "presets/scc/scc-notifications.satz"
```

- a `google_pubsub_topic` in the project the estate names
  (`scc_notification_project`, asked; the infrastructure project by default);
- `roles/securitycenter.notificationServiceAgent` for
  `service-org-<organisation>@gcp-sa-scc-notification.iam.gserviceaccount.com` on
  that topic — the identity the config reports in its own `serviceAccount` field,
  which is not the `security-center-api` agent SCC activation creates. Without the
  grant the config is created, reports no error, and publishes nothing. SCC adds
  the binding itself when the config is created; the estate declares it anyway, and
  the two converge. `iam.managed.allowedPolicyMembers` does not apply to
  Google-managed service agents, so a domain-restricted organisation permits it;
- `google_scc_v2_organization_notification_config` at `location = "global"`, with
  the filter the customer decides (`scc_notification_filter`, asked; active HIGH and
  CRITICAL findings by default). The **v2** resource is deliberate: the v1
  notification API answers "This API is no longer available" on a live organisation.

Nothing here subscribes to the topic: an organisation with a SIEM points it at the
topic, and one without adds `scc/scc-findings-mail.satz` below. No catalog control
covers SCC, so the pack claims nothing. It needs the detectors switched on to have
anything to publish — use it with `scc/scc-service-enablement.satz`.

### `scc/scc-findings-mail.satz` — the mailbox that is told

For an organisation with nothing subscribed to the topic. A Pub/Sub topic with no
subscription drops every message it receives, so findings would be published into
nothing.

```
use "presets/scc/scc-findings-mail.satz"
```

Three resources on top of the notification pack, whose topic and project it takes:
a subscription that keeps seven days, an e-mail notification channel, and an alert
policy that fires when messages reach the topic. The address is asked
(`scc_findings_email`), and its default is the central alert pack's
`cis_central_email` BY REFERENCE — one security mailbox for the CIS alerts and the
findings. Without that pack and without an answer the compile stops with `unknown
param 'cis_central_email'`, which is the honest failure: the recipient is undecided.

What the mail says is that findings were published, with a link — not the finding
itself. Cloud Monitoring alerts on a metric, and the metric is how many messages
reached the topic; the finding's text is in the topic, the console and the export.
Putting the finding in the body needs a subscriber that formats it, which is code,
and code is not a resource.

The mailbox must exist and accept mail from `alerting-noreply@google.com`. A group
whose members have no mailboxes drops every alert and nothing in the estate can see
it happen — a group, never a person.

### `scc/scc-findings-siem.satz` — the SIEM pulls them

The other destination, for a customer whose security team works in Microsoft Sentinel,
Defender for Cloud, Splunk or QRadar rather than in a mailbox.

```
use "presets/scc/scc-findings-siem.satz"
```

Two resources on the notification pack's topic: a PULL subscription of its own — every
connector in this class pulls, and a push endpoint needs a URL the estate cannot know —
and `roles/pubsub.subscriber` for the identity the connector reads as. The grant is the
half that is forgotten: without it a connector authenticates, finds the subscription and
reads nothing.

The identity is asked (`scc_siem_subscriber`, a full IAM member) and has **no default**.
For Microsoft Sentinel it is the service account
`integrations/microsoft-sentinel.satz` creates; for Defender for Cloud the one its
onboarding script names; for another SIEM whatever its connector authenticates as.
A default would tie Security Command Center to one vendor's pack, and a wrong one is a
subscription nobody can read.

This and the mailbox are not exclusive — each makes its own subscription, so the SIEM
ingests every finding while the mailbox is told they are arriving. Two readers on ONE
subscription would split the findings between them, which is why they do not share.

### `scc/scc-export.satz` — findings kept and queryable

The other half: a notification tells somebody now, an export answers what the
organisation looked like months ago.

```
use "presets/scc/scc-export.satz"
```

Four resources, and their order matters: `bigquery.googleapis.com` in the dataset's
project, the dataset (which takes its project THROUGH that service resource, so the
API is enabled before the dataset is made), `roles/bigquery.dataEditor` for the
exporting agent — the same `gcp-sa-scc-notification` identity that publishes
notifications, and the `principal` the export reports — and the v2 export itself.
Even with the ordering, a first apply can fail with *"The project … has not enabled
BigQuery"*: the API is on and BigQuery's control plane is a minute behind. Run it
again.

The export pins its own `name`, which the server assigns: without it every plan
proposes to null the field and the API refuses ("Field name is immutable"). Its
other fields do not update in place either — a new description, dataset or filter
means replacing the export (`satz apply -replace=…`), which keeps the dataset.

`delete_contents_on_destroy` stays false, so removing the pack from an estate never
deletes the finding history. The dataset's location is asked and cannot be changed
afterwards; where the CIS pack enforces `gcp.resourceLocations`, a location outside
that list is refused at apply.

Not in the library, and why: **mute configs** (which findings to silence is a
customer's noise decision, and a wrong mute hides a real finding — write them in the
estate), **custom modules** for Security Health Analytics and Event Threat Detection
(the rules are content a customer's security team owns; on a 2026 organisation SHA's
modules cannot be set at all, see below), **sources and source IAM** (only for a
customer pushing third-party findings into SCC), and **Security Posture**, which is
a framework of its own rather than part of this preset.

A **pack** may declare an action too, and `satz doc-packs` puts it on the pack's
page. An action is a step satz runs, never a witness: no claim covers what a script
did, and nothing about it reaches `report-compliance`. Because
`get-presets` downloads packs from this public repository, every compile warns
when one declares an action, `--no-pack-actions` ignores pack-declared ones, and
a downloaded script arrives without its executable bit, which satz does not set.

The script enables every service except Web Security Scanner (it actively crawls
the customer's web apps) and Artifact Analysis (billed per image scan), which
`--with-optional` adds, and the AWS/Azure connectors, which `--with-multicloud` adds.
A detector for a workload that does not exist yet costs nothing, so the rest are
enabled before the workload exists.

The §1.1 lock applies here as well: SCC's service agents get their roles at the
organization, and the lock refuses any agent `allowed_policy_member_subjects` does
not name. In the console a refused agent shows as SCC not staying activated and
asking to be activated on every visit.

**Five agents, however many services are enabled.** With every service that can be
enabled turned on — all fourteen GCP-side ones, including the four reachable only
through the API — a live organization's IAM policy carried exactly the five the
baseline lists
(`securitycenter`, `cloudsecuritycompliance`, `dspm`, `externalexposure`,
`containerthreatdetection` service agents). They come with SCC activation, not
per service. A **notification config** adds
`service-org-<ORG_ID>@gcp-sa-scc-notification.iam.gserviceaccount.com` with
`roles/securitycenter.notificationServiceAgent` on the **Pub/Sub topic**, not at the
organization; creating one succeeded with the §1.1 lock enforced and that agent not
in the list. Not measured: Security Health Analytics (it failed a precondition on
that organization) and the AWS/Azure connectors.

Flags, failure modes and the rest: [`docs/housekeeping.md`](../docs/housekeeping.md),
under "The scripts, one by one".

**Use**, then adopt what the organisation already has (`satz adopt --activate`
activates managed constraints via the Org Policy API and imports existing policies
into state — see "Adopting what already exists" in the main README):

```
use "presets/cis/CIS-GCP-Foundation-4.0.satz" when use_cis_baseline
```
```bash
satz adopt C0example.satz --only google_org_policy_policy --activate --execute --import
```

## billing-account-permissions.satz

Billing-account IAM, split by audience: everyone in the domain gets
`billing.user` + `billing.viewer`; the full administration (`billing.admin` +
`billing.costsManager`) goes to ONE group named by the `billing_admins_group`
param (default `gcp-billing-admins@{customer_domain}` — the s1 model's group);
the IaC service account keeps `billing.admin`. Declares its own
`google_billing_account_iam_member` map, pinned to `billing_account_infra`.

**Use** (root level): `use "presets/billing-account-permissions.satz"`

**Question.** `billing_admins_group` is asked: the group can move projects between billing accounts and see every cost.

## organization-budget.satz

A global budget (1000 EUR, thresholds at 50/80/100% of current spend) on the infra
billing account (declares its own `google_billing_budget` map).

**Use** (root level): `use "presets/organization-budget.satz"`

**Notes:** contains a placeholder `"import-id"` for adopting an existing budget — remove
it for a fresh budget, or replace it with the real budget id (`satz adopt` does not
resolve budgets: they are matched by display name, which needs the Budgets API). The
amount and the thresholds are literals, not params.

## essential-contacts-organization.satz

One organization-level Essential Contact subscribed to ALL notification categories.
A content pack: use it inside the resource map.

**Use:**

```
google_essential_contacts_contact { use "presets/essential-contacts-organization.satz" }
```

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `essential_contacts_email` | `"essential-contacts-all@{customer_domain}"` | the contact address |

**Splitting by category:** the pack carries COMMENTED contacts for
each category — `BILLING`, `SUSPENSION`, `SECURITY`, `TECHNICAL`, `LEGAL`,
`PRODUCT_UPDATES`, and a multi-category `oncall` example — each with its own
`essential_contacts_<category>_email` param. Uncomment what you split out
(in a `.local` fork, or the estate declares them directly), give each a
distinct address, and narrow or delete the `all` contact: one address may
appear once per parent, and an address on ALL already receives everything.

**Question.** `essential_contacts_email` is asked: the mailbox must exist and accept external mail; otherwise Google's suspension, security and legal notices reach nobody.

## integrations/microsoft-defender-for-cloud*.satz

Microsoft Defender for Cloud's GCP onboarding: workload identity federation, so no
service-account keys leave the estate. Four files — the foundation plus one fragment per
licensed plan and per access mode — because a fragment cannot add to another fragment's
project (two definitions of one project is a fold conflict, and lists replace rather than
concatenate). The foundation owns the management project and its complete API set; each
plan declares its own resources at top level, naming the project through a param.

**Use** (root level):

```
use "presets/integrations/microsoft-defender-for-cloud.satz"
use "presets/integrations/microsoft-defender-for-cloud-cspm.satz" when mdc_plan_cspm
use "presets/integrations/microsoft-defender-for-cloud-cspm-role-default.satz" when mdc_cspm_default_access
use "presets/integrations/microsoft-defender-for-cloud-cspm-role-least-privilege.satz" when mdc_cspm_least_privilege
```

**Params:** `mdc_workload_pool_id` (the customer's Entra tenant id without dashes — that is
what Microsoft's wizard uses as the pool id), `mdc_mgmt_project_id`, `mdc_plan_cspm`, and
the access-mode pair `mdc_cspm_default_access` / `mdc_cspm_least_privilege`. Everything
Microsoft-side — their tenant as the OIDC issuer, the per-plan `api://` audiences, the
provider ids, the custom role ids, the API list — is an inlined constant, identical for
every customer and not a param.

**No claim.** Defender for Cloud is an external CSPM that reads the estate. It implements
no CIS control and contributes to none, so the pack asserts nothing.

**Two prerequisites before the first apply.** The Defender agentless-scanning service
account lives in a Microsoft project, so it must be in `allowed_policy_member_subjects`
BEFORE any grant to it is applied — the constraints AND together and an incomplete list
refuses the grant. And a deny-all on `iam.workloadIdentityPoolProviders` blocks the
providers: the estate must allow the `sts.windows.net/<microsoft tenant>` issuer or
document the exception.

**Coverage.** The pack ships the two plans whose resources were read from a
Microsoft-generated onboarding script: auto-provisioner (always created, in the
foundation) and CSPM. The other plan ids Microsoft issues — `ciem-discovery`,
`containers`, `containers-streams`, `data-security-posture-storage`,
`defender-for-databases-arc-ap`, `defender-for-servers` — each need their own `api://`
audience, service account and role set, which only that customer's script contains,
so they are not shipped.

**Questions.** `mdc_workload_pool_id` and `mdc_mgmt_project_id` block until typed —
only Microsoft's generated script knows them; `mdc_plan_cspm` asks whether the plan is
licensed; and the access mode is a `question oneof` with `ask_when = mdc_plan_cspm`, so
it is asked only once CSPM is on.

## integrations/microsoft-sentinel*.satz

Microsoft Sentinel's GCP connector: Sentinel PULLS logs from a Pub/Sub subscription,
authenticating through workload identity federation, so no service-account key leaves
the organisation. Two files, because federation is set up once and log sources are
added one at a time.

**Use** (root level):

```
use "presets/integrations/microsoft-sentinel.satz"
use "presets/integrations/microsoft-sentinel-auditlogs.satz" when use_sentinel_auditlogs
```

`microsoft-sentinel.satz` is the federation half: the two APIs, a pool named after the
customer's Entra tenant, the `sentinel-identity-provider` that trusts Microsoft's
commercial tenant as issuer with `api://<Sentinel application id>` as the audience, the
`sentinel-service-account`, and `roles/iam.workloadIdentityUser` for the pool's whole
principal set on that account. It grants nothing else — each log source grants what it
needs, where it needs it.

`microsoft-sentinel-auditlogs.satz` is the first log source: an organisation sink with
`include_children` exporting the four audit streams, the topic it writes to, the
subscription Sentinel pulls from, `roles/pubsub.publisher` for the sink's own writer
identity (without which the sink exists and delivers nothing), and
`roles/pubsub.subscriber` for the Sentinel account on that one subscription. Microsoft's
published configuration grants a project-level custom role carrying
`pubsub.subscriptions.consume` and `.get` instead, which reaches every subscription in
the project; this is the same access confined to the one that exists for it.

**Params:** `sentinel_workload_pool_id` (the Entra tenant id without dashes) and
`sentinel_project_number` block until typed — the project number cannot be derived from
the id and the principal set is built from it. `sentinel_project_id` defaults to the
audit archive's project by reference. `sentinel_auditlogs_filter` is asked: Data Access
logs are most of the volume and Sentinel charges by the gigabyte ingested.

**Transcribed, not imported.** The shape is Microsoft's own Terraform in
`Azure/Azure-Sentinel` (`DataConnectors/GCP/Terraform/sentinel_resources_creation/`),
which pins the Google provider at 3.73.0 and uses authoritative
`google_project_iam_binding` — run beside an estate it removes grants the estate made.
Everything here is non-authoritative `_iam_member` against the pinned provider.

`microsoft-sentinel-network-logs.satz` is the four network streams — VPC flow logs,
firewall rules logging, DNS queries and Cloud NAT — each with its own sink, topic,
subscription and pair of grants. **On by default with Sentinel**, because every one of
them is empty until somebody enables that feature on a subnet, a rule, a DNS policy or a
NAT gateway, and Google charges nothing to route log entries: a sink for a feature nobody
enabled costs nothing, while switching them off costs the day somebody enables flow logs
and finds Sentinel was not watching. The volume once a feature is on lands on Sentinel's
ingestion bill; an estate that wants one stream out `suppress`es that sink, topic and
subscription.

Each filter selects exactly one stream — `log_id` where Google publishes the log name
(flow logs, firewall, NAT) and the documented `dns_query` resource type for DNS, whose
log name Google does not publish. Microsoft's own per-source configurations mix each
stream with the audit records of the same service, which already leave through the
audit fragment: carrying them again exports and bills the same entries twice.

**The nine sources this library does not ship.** Of Microsoft's fourteen
configurations, nine — Apigee, Cloud SQL, Compute, IAM, Resource Manager, and the audit
halves of CDN, NAT, DNS and Cloud IDS — are only `protoPayload.serviceName=…` filters
over the same audit stream the audit fragment exports organisation-wide. As separate
sinks they duplicate entries and pay for them twice. Service-scoped audit routing, if a
customer wants it, is ONE sink with a union filter. Three more are unbounded and stay a
decision rather than a default: Microsoft's audit setup carries NO filter at all (the
entire log estate), its GKE filter matches `.*stdout$`/`.*stderr$` with no resource-type
guard (every application log in scope, not only GKE's), and its Cloud Run filter matches
`cloud_run_revision` (the same for Cloud Run).

**Why the rest cannot be transcribed as published.** Every upstream setup grants
publisher with authoritative `google_project_iam_binding`: the second source applied
REMOVES the first sink's writer identity from the role, and delivery stops without an
error. Its firewall setup names its topic `sentinel-topic` — the same name its audit
setup creates — and its IAM setup subscribes to that topic without creating it. Its NAT
filter's `logName=` is unquoted, so that half of the filter matches nothing.

**Two organisation policies can refuse the first apply**, neither of them set by this
library: a deny-all on `iam.workloadIdentityPoolProviders` blocks the provider unless
the `sts.windows.net/<microsoft tenant>` issuer is allowed, and where
`iam.managed.allowedPolicyMembers` is enforced the principal set must be allowed before
the binding is applied.

**Running Defender too?** They share nothing — separate pools, accounts and topics. If
both are pointed at one project, give them different pool ids: a pool id is unique per
project.

**No claim.** Exporting logs to a SIEM does not satisfy a retention control — the audit
archive pack claims those — and Sentinel is in no catalog.

**Onboarding is two-sided.** This is the Google half; the Sentinel connector in Azure is
configured with the pool, provider and service account it creates, and nothing flows
until both sides are done.

## Questions

A pack declares its params, its claims — and what a human must be asked before those
params can be filled:

```
question customer_shortname {
  prompt   = "Short name identifying this customer"
  why      = "Project ids and bucket names derive from it, and those ids are globally unique."
  reversal = recreate      // edit | state_surgery | recreate — cost to the ESTATE
  blast    = none          // none | low | high — cost to the RUNNING organisation
}
```

The two costs are independent: enforcing OS Login is one boolean to reverse and cuts
every existing SSH path.
`why` is required wherever satz will refuse or warn, so it can quote the pack's own
sentence. An exclusive choice is `question oneof`, whose options name existing boolean
params — so an answer set stays a plain param map and satz can refuse two true branches
by name.

A question must live in the file that declares its param: questions are absorbed after
the `use … when` guard, so a question gating a pack cannot live in the gated pack.

A question is **answered when the estate's own `params {}` binds its param** — to the
pack's default or to anything else; accepting a default is an answer, recorded by
writing the default in. A question whose param the estate does not bind is
`unanswered`, and **every question must be answered before the estate touches an
organisation**: `bootstrap` and `transpile --apply` refuse while one is open,
`transpile --plan` warns. A question with no usable default is `blocking` — a value has
to be typed. `ask_when` names a boolean param; when it is false the question is
`not-applicable` and counts toward nothing.

`satz questions <estate>` lists them with their state (`--unanswered` for the open ones,
`--format markdown` for the decisions sheet a customer signs off); `satz interview` asks
them and writes the answers; `satz doc-packs` gives each pack a Questions section; and
the prompt becomes the generated `variables.tf` description. `check-presets` reports a
pack whose questions changed as `questions`, not as drift: its HCL is identical, so the
estate is not forked, and the change — a `recreate → edit` downgrade included — is
still listed.

## Superseded legacy constraints

Where Google replaces a legacy org-policy constraint with a managed one, a pack runs the
**replacement alone** and declares the legacy twin OFF in the same file:

```
"compute-requireOsLogin-superseded" {
  name = "compute.requireOsLogin"
  parent = "organizations/{customer_organization_id}"
  spec {
    reset = true
  }
}
```

The `-superseded` address suffix is required: switching a policy from rules to `reset`
cannot be an in-place update — the provider PATCHes the rules it still holds together with
`reset`, and the API refuses the pair — so the reset policy has its own address and the
plan is a destroy + create by construction. The policy NAME
is unchanged; it is one policy being reset, not two. [ADR 0002](../docs/adr/0002-superseded-org-policies-replace-by-construction.md).

Both forms in force is a defect. Org-policy constraints AND together, so an exemption
has to lift **two** policies, and for several legacy constraints Google's only
documented exemption path is to disable the constraint org-wide, grant, and re-enable
it, which leaves the control off in between. The CIS pack's `duty_legacy_superseded`
records this for every pair it enables.

**Leaving the legacy policy out does not remove it.** A legacy policy already set on an
organisation is invisible to an apply that does not declare it: it keeps enforcing until
someone deletes it by hand. `reset = true` restores the constraint's default — ALLOW for
every constraint paired this way, verified against a live organisation — so the apply
resets it, and the next apply reverts a later re-enabling. On an organisation where the legacy policy
already exists, run `satz adopt --only google_org_policy_policy --execute --import` first so
it is imported rather than created twice.

Do not `suppress` one of these blocks: suppressing it removes the declaration that the
legacy constraint is off, so a legacy policy already set on the organisation keeps
enforcing beside its managed twin.

**The pairing is data, and the rule is a gate.** Which managed constraint replaces which
lives in `presets/managed-constraint-equivalents.txt`, generated from a live organisation by
`scripts/update_constraint_equivalents.py` and never hand-edited above its `CURATED` marker.
`cargo test` compiles every corpus case against that table and fails when a pack enforces a
legacy constraint that has a replacement, or enables a replacement without declaring its twin
off.

Only pairs need this. Of the 60 managed constraints a live organisation offers, **45 have no
legacy form at all** — nothing to switch off. Google declares the pairing in
`equivalentConstraint`, asymmetrically (15 managed name a legacy twin; only 6 legacy name a
managed one), and not at all for `iam.allowedPolicyMemberDomains` ↔
`iam.managed.allowedPolicyMembers` — that pair is in the file's `CURATED` section.

### The one claim that is not about an org policy

The baseline claims **CIS 5.0 §2.14, Cloud Asset Inventory enabled**, against
`google_project_service.infra_cloudasset_googleapis_com` — a resource the SCAFFOLD
declares, not the pack. Every estate `satz init` writes enables `cloudasset.googleapis.com`
in its infrastructure project, so every estate already satisfied that control and said
nothing about it; it was the last technical control of CIS 5.0 with no claim anywhere in
the library.

The pack does not declare its own `google_project_service` for the API. Two resources
enabling one API on one project is a duplicate Terraform resource, not a merge.

**So the estate's project labels are a contract.** `google_project.infra` is what
`bootstrap` imports by name, and the emitter derives that service address from it
(`<project label>_<service, dots to underscores>`). An estate whose infrastructure
project carries a different label, or whose service list has lost
`cloudasset.googleapis.com`, reports this control as a **broken claim** — which is the
right signal twice over: the witness named is genuinely not there, and
`report-compliance` reads the organisation through that same API, so it could not verify
anything else either.

## exemptions/

**Two features live here, and they are two halves of one question: who may do what.**
The [security group models](#security-group-models) say who holds STANDING authority —
who administers projects, networks, guardrails, billing. This pack says who may make a
narrow, named EXCEPTION to a control that authority set, without holding that authority.

**One pack, and it exempts nothing.** `exemption-tag.satz` creates the organisation tag
an org policy can condition on: the key `<shortname>-exemption` and one value per
exemption CLASS, bound to nothing.

### Why a tag rather than lowering the policy

An org policy is all-or-nothing per node. Letting ONE service account create ONE key
means lowering the policy for its whole project or folder and raising it again — a
window during which nothing under that node is enforced, and which nobody remembers to
close. A Resource Manager tag is IAM-governed and a policy rule can condition on it, so
the policy stays enforced everywhere and named resources are let out one at a time.
Google ships `iam.disableServiceAccountKeyCreation` this way on new organisations.

### The classes, and why there are several

**IAM is set on a tag VALUE.** One blanket `not_enforced` would mean that anyone allowed
to exempt anything may exempt everything — the team that needs a public bucket could
switch off customer-managed encryption just as easily. So each value is a class covering
one kind of risk, and `roles/resourcemanager.tagUser` on one value delegates exactly
that kind:

| class | what it lets out | constraints it is written for |
|---|---|---|
| `service-account-keys` | a principal that must hold a static credential | `iam.managed.disableServiceAccountKeyCreation`, `…KeyUpload`, `iam.managed.disableServiceAccountApiKeyCreation` |
| `public-endpoint` | a workload reachable from the internet on its own address | `compute.managed.vmExternalIpAccess`, `sql.managed.restrictPublicIp`, `sql.managed.restrictAuthorizedNetworks` |
| `public-storage` | an object store published on purpose, and the access model for it | `storage.publicAccessPrevention`, `storage.uniformBucketLevelAccess` |
| `vm-image` | an image or machine family that cannot boot shielded or confidential | `compute.requireShieldedVm`, `compute.managed.restrictNonConfidentialComputing` |
| `vm-access` | an instance reached by metadata SSH keys or the serial console | `compute.managed.requireOsLogin`, `compute.managed.blockProjectSshKeys`, `compute.managed.disableSerialPortAccess` |
| `data-residency` | a resource created outside the permitted locations | `gcp.resourceLocations` |
| `encryption` | a resource that may use Google-managed keys | `gcp.restrictNonCmekServices`, `gcp.restrictCmekCryptoKeyProjects` |
| `network-appliance` | an instance that forwards traffic for others | `compute.managed.vmCanIpForward`, `compute.managed.restrictProtocolForwardingCreationForTypes` |

The classes are deliberately narrow, and `vm-image` is separate from `vm-access` for that
reason: what a machine can run and who can get into it are different risks and should be
different grants. A wide class is a grant that hands over more than the person asking for
it described.

**What has no class, on purpose.** Audit logging (`gcp.detailedAuditLoggingMode`,
`iam.disableAuditLoggingExemption`), VPC flow logs, DNS query logging and
domain-restricted sharing (`iam.managed.allowedPolicyMembers`). Exempting the record of
what happened, or letting an outside identity in, is not a delegation — it is a decision
for whoever owns the baseline, made by editing the policy where the change is visible.
The service-agent caveat for domain-restricted sharing is already a baseline parameter
(`allowed_policy_member_subjects`), which is the route there.

### Who may exempt, and where: two grants, both required

| grant | on what | decides |
|---|---|---|
| `roles/resourcemanager.tagUser` | the tag VALUE | WHICH class this principal may grant at all |
| the `createTagBinding` permission | the TARGET (organisation, folder or project) | WHERE they may apply it |

That pair is the project / folder / organisation restriction — no separate mechanism. A
team holding `tagUser` on `public-endpoint` and `createTagBinding` on their own folder can
exempt public endpoints in that folder, and nothing else, anywhere else.

The first half is declarable in the estate, so *who may exempt what* is in the repository
rather than in somebody's console history:

```
google_tags_tag_value_iam_member {
  "platform_may_exempt_keys" {
    tag_value = "${{google_tags_tag_value.exempt_service_account_keys.name}}"
    role      = "roles/resourcemanager.tagUser"
    member    = "group:gcp-platform-admins@{customer_domain}"
  }
}
```

The second half stays the estate's too, but is not in this pack: which folder a team owns
is the customer's hierarchy, not the library's.

**No new security group is needed for this, and adding one would defeat it.**
`gcp-security-admins` already holds `roles/orgpolicy.policyAdmin` org-wide, so that group
can lift any policy today by editing it; giving it the tag as well buys only an audit
trail. The point of the tag is that "may grant a narrow exemption" need not imply "may
rewrite any policy", and an org-wide exemption-approver group would re-centralise exactly
what the tag decentralises. Grant the node half to whoever owns the project or folder.

### What a binding reaches

**Tag bindings are inherited.** Bound to a service account, an exemption reaches that
account. Bound to a PROJECT it reaches everything in that project — including every
resource created in it afterwards, for as long as the binding exists. Bound to a folder,
everything below. A project-level binding is the widest and quietest exemption available;
prefer binding the individual resource.

### Using one

The constraint has to condition on the class. **One CIS constraint does already, with no
forking involved:** `iam.managed.disableServiceAccountKeyCreation` takes its rules from
the baseline param `cis_sa_key_creation_rules`, which defaults to the plain enforcing
rule. To let one service account out, rebind it:

```
cis_sa_key_creation_rules = [
  {
    enforce = "FALSE"
    condition = {
      title      = "exempted service accounts"
      expression = "resource.matchTagId('${{google_tags_tag_key.exemption.name}}', '${{google_tags_tag_value.exempt_service_account_keys.name}}')"
    }
  },
  { enforce = "TRUE" },
]
```

then bind that value to the one account. `tests/iac/exemption-tag/main.satz` is the whole
thing end to end: the rebound param, the class grant, and the binding.

It is the only constraint with a rules param. The rest are written in place, because a
param per constraint would put forty list-of-object blocks into every estate's
`terraform.tfvars` for a case nobody has; another constraint earns one the way this did,
from a real organisation that needed it. Until then an estate can `suppress` a pack's
policy and declare its own — a fork, and the thing the param exists to avoid.

### Where an exemption is visible

`satz require` prints it under its control rather than letting a conditional policy read
as plain "enforced" — the control is met *and* something is let out:

```
  ✓ 1.4   Only GCP-managed service account keys  — google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation
      ↳ exempted: google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation: enforce OFF where exempted service accounts
```

The estate carries the binding with its owner and reason, which is what makes a permanent
exemption reviewable. And Cloud Asset Inventory answers the org-wide question, per class:

```bash
gcloud asset search-all-resources --scope=organizations/ORG \
  --query='tagValues:<shortname>-exemption/service-account-keys'
```

A binding somebody adds out of band is not visible to satz yet. Cloud Asset Inventory
serves `cloudresourcemanager.googleapis.com/TagBinding`, so reporting an undeclared
binding as drift is the piece that would make temporary lifts auditable.

### Before granting any exemption

Does the consumer need one? A workload inside Google Cloud, a Cloud Run service, and
external CI on GitHub or GitLab can all use Workload Identity Federation or impersonation
and leave the control intact. The tag route is the exception with a named owner, never the
default.

## cis/

**Two of these are ON by default: `cis_dns_logging` and `cis_block_internet_ssh_rdp`.**
Every other fragment here is off until a customer asks for it, because each one restricts
what an organisation may create. These two are what an organisation is expected to have
already.

DNS logging does not restrict anything: it makes a RECORD — of name resolution, which is
how a compromised host asking for its command-and-control domain becomes visible, and
which nothing reconstructs afterwards.

The admin-port policy closes TCP 22 and 3389 to the INTERNET, which is what CIS 3.6 and
3.7 ask for. It does not close them internally: its pass list carries the private blocks
beside Google's IAP ranges, because the deny matches `0.0.0.0/0` — every address, private
ones included — and a hierarchical policy is read before the VPC rules that would
otherwise allow internal traffic. What breaks is a bastion reachable on a PUBLIC address;
that access belongs on IAP TCP forwarding, which reaches an instance without an open
admin port.

Switching either off is a decision, so the estate that makes it writes a `deviates`
claim, whose `reason` is mandatory and which an auditor reads in the compliance report.
The library does not argue; it records who decided and why.

**VPC flow logs are not in this directory**, and do not need to be: the baseline pack
enforces `compute.requireVpcFlowLogs` and claims CIS 4.0 §3.8 / 5.0 §3.10 with it. That
constraint does not refuse a subnet without flow logs — it applies a minimum logging
level to it, which is why it is safe in the baseline. Measured on a live organisation: a
subnet created with no flow-log flags at all came back with `enable: true` and 0.1
sampling. What it DOES refuse is a subnet whose flow-log settings are hand-tuned to
something outside Google's three named levels (ESSENTIAL, LIGHT, COMPREHENSIVE) — so a
customer who wants a custom sampling rate must pick one of the three or widen the policy
deliberately.


CIS coverage beyond the baseline, one fragment per control, all **opt-in**. The base
pack declares the flags (`cis_require_shielded_vm` and friends, all `false`); an estate
turns one on and `use`s its fragment:

```
cis_require_shielded_vm = true
use "presets/cis/shielded-vm.satz" when cis_require_shielded_vm
use "presets/cis/dns-logging.satz" when cis_dns_logging
```

Each is opt-in because it can break a running workload: Confidential Computing is limited to particular machine families,
Shielded VM needs image support, CMEK needs the keys and grants to exist first, Cloud SQL
hardening cuts public-IP connectivity, the bucket-retention constraint applies to
every bucket in the organisation, not only the log sink's, Access Approval makes every
support case that needs the customer's content wait for an approval, the SSH/RDP policy
ends every session that reaches an instance straight from the internet, and the Cloud SQL
IAM/deletion-protection pair refuses every create or update without both settings. The
comment at the top of each fragment says what specifically breaks.

### Measure before you enforce: the dry-run twins

"Will this break us?" has an answer that is not a guess. An org policy can carry
`dry_run_spec` instead of `spec`: Google evaluates every rule, writes a violation to the
audit log for each action it WOULD have blocked, and blocks nothing. Six of these
fragments ship a **dry-run twin** that does exactly that:

```
cis_cloud_sql_hardening_dry_run = true
use "presets/cis/cloud-sql-dry-run.satz" when cis_cloud_sql_hardening_dry_run
```

Apply it, let the organisation run, then read the violations:

```
protoPayload.metadata."@type"="type.googleapis.com/google.cloud.audit.OrgPolicyViolationInfo"
```

Zero violations over a representative period means enforcing costs nothing. Promote by
switching the dry-run param off and `cis_cloud_sql_hardening` on.

**A twin replaces its enforcing fragment, it does not sit beside it.** Both params true
is refused, naming both and what each choice means: the twin declares the same policy
addresses, so an estate asking for both asks for one policy to block and measure at once.

**A twin carries no claim**, and that is deliberate. A dry run discharges no control
while it measures, so `require` reports the control unmet — the truth. A claim over a
dry-run policy would be contradicted by its own witness.

The twins are GENERATED from the enforcing fragments by
`scripts/build_dry_run_fragments.py` and must not be edited: a dry run that measures a
different policy than the one that will be enforced answers a question nobody asked. Six
fragments have a twin; five do not, and the convention tolerates that rather than
pretending otherwise — the legacy constraints (Shielded VM, both CMEK list constraints)
have no dry-run form, Access Approval is not an org policy, and the two on-by-default
extensions have nothing to size.

| fragment | controls | why it is not in the baseline |
|---|---|---|
| `block-project-ssh-keys` | 4.3 | the managed constraint is still PREVIEW, with no legacy equivalent |
| `shielded-vm` | 4.8 | image support; the only one with no managed form and no dry-run |
| `confidential-computing` | 4.11 | machine-family limited; CIS rates it Level 2 |
| `cloud-sql` | 6.5, 6.6 (5.0: 6.7) | existing public-IP instances lose connectivity |
| `cmek` | 7.2, 7.3, 8.1 | keys, key rings and service-agent grants must exist first |
| `api-key-services` | 4.0 1.14 / 5.0 1.15 | narrows what an API key may call |
| `bucket-retention` | 4.0 2.3 / 5.0 2.4 | constrains every bucket's retention duration |
| `access-approval` | 4.0 2.15 / 5.0 2.16 | support cases wait for an approval; Access Transparency must be on first |
| `internet-ssh-rdp` | 3.6, 3.7 | ends SSH and RDP sessions that reach an instance straight from the internet |
| `cloud-sql-iam-and-deletion-protection` | 5.0 6.6, 6.9 | every Cloud SQL create and update must carry both settings |

Three fragments are not org-policy constraints. `access-approval` is an organisation
setting (`google_organization_access_approval_settings`); Access Transparency, which it
needs, has no provider resource and is switched on in the console first.
`internet-ssh-rdp` is a hierarchical firewall policy attached to the organisation, and
one of the two fragments here that are ON by default. Four rules: SSH and RDP from
`admin_port_source_ranges` (1000) and `admin_port_source_ranges_ipv6` (1001) pass to the
VPC firewall rules, which still decide; every other address is denied (1002 for IPv4,
1003 for IPv6) before any VPC rule is read. The families are separate rules because a
rule's sources may not mix IPv4 with IPv6.

The default pass lists are Google's IAP TCP-forwarding ranges — `35.235.240.0/20`, and
`2600:2d00:1:7::/64` for IPv6 VMs — plus the private blocks. Those are there because the
deny matches `0.0.0.0/0`, which is *any* IPv4 address, private ones included, and a
hierarchical policy is read before the VPC rules that would otherwise allow internal
traffic: without them, SSH between two instances in one subnet is denied. An estate that
wants internal SSH denied as well removes them.

Each rule carries the control's full protocol set — SSH on TCP 22 **and SCTP 22**, RDP on
TCP 3389 **and UDP 3389** — because Google's own detectors check all four and a TCP-only
deny leaves UDP 3389 open.

Only the two deny rules log, and that is Google's rule rather than a choice: logging
cannot be enabled on a `goto_next` rule. So the firewall record is of what was REFUSED;
an accepted IAP session leaves no firewall log. That is the stream
`integrations/microsoft-sentinel-network-logs.satz` carries, and it is empty while this
fragment is off.

Prowler's checks read the VPC rules, so a VPC rule allowing 0.0.0.0/0 still fails there
and the row reads CONTESTED until the rule is deleted. The same is true of Security
Command Center's `OPEN_SSH_PORT`, whose supported asset is the VPC firewall rule: a
hierarchical deny shadows such a rule without clearing the finding, so delete
`default-allow-ssh` and `default-allow-rdp` rather than relying on this policy to hide
them. `cloud-sql-iam-and-deletion-protection`
declares two custom constraints (`google_org_policy_custom_constraint`) and a policy
enforcing each; the emitter makes each policy wait for its constraint. A custom
constraint is checked when an instance is created or updated, never against one that
already exists.

**Constraint names and shapes come from a live organisation's OrgPolicy
`ListConstraints`**, not from documentation. The three shapes differ, and a policy in
the wrong shape either does nothing or refuses everything: a plain managed boolean takes `enforce`; a managed boolean with a parameter
takes `enforce` plus `parameters`; a list constraint takes allow/deny values.

**Questions.** The five fragments with a list to fill ask for it: the API services a
key may target (the empty default blocks: an empty list is a valid answer, but the
customer gives it), the allowed retention durations, the CMEK services and key
projects, the addresses that receive access approval requests (empty blocks until
named), and the ranges that may still reach SSH and RDP. Whether a fragment is on at all is the CIS pack's
question, not the fragment's — a question that gates a pack cannot live in the gated pack.

## A big resource is a pack

A resource with a long literal (a custom role with 1,400 permissions, an
allowlist shared by several resources) goes into its own pack — the whole
resource, not just the value:

```
// presets/roles/application-owner-connected.satz
pack roles.application_owner_connected version "1.0"
params { application_owner_connected_role_id = "ApplicationOwnerConnected" }
google_organization_iam_custom_role {
  ApplicationOwnerConnected {
    role_id = application_owner_connected_role_id
    title = "ApplicationOwnerConnected"
    permissions = [ "accessapproval.requests.get", … ]
  }
}
```

and the estate says `use "presets/roles/application-owner-connected.satz"`.
The resource gains a name, a version and a ledger entry, `merge-presets` can
track it, and any estate can share it. A params-only pack
(`permissions = <param>`) is the shape when the LIST itself is the shared thing.
Satz has no value-position include: `use` is a language construct (params,
provenance, claims), not a preprocessor splice.

## ci/ — continuous verification

Two packs that run satz's checks in Cloud Build, on every push and nightly:
`ci/verification-runner.satz` (a Cloud Build trigger `satz-check` on every push running
`transpile --check`, a nightly `satz-compliance` via Cloud Scheduler running
`report-compliance --fail-on`, the runner service account and its two project roles)
and `ci/verification-runner-grant.satz` (one binding on the ESTATE's IaC service
account so the runner may become it). Split along ownership: customer-hosted uses both
in one estate and wires nothing; MSP-hosted puts the runner in the MSP's estate and the
grant in the customer's. The build steps are inline in the trigger, not a file in the
watched repository — whoever controls the build file controls what runs as the runner.
Params and the `use` blocks: [docs/verification-runner.md](docs/verification-runner.md),
[docs/verification-runner-grant.md](docs/verification-runner-grant.md); the workflow and
the hosted shape: [docs/workflows.md](../docs/workflows.md#continuous-verification);
the reasoning: [ADR 0004](../docs/adr/0004-the-verification-runner-is-a-pack-and-its-pipeline-is-inline.md).

**Questions.** The runner asks where it lives (`ci_runner_project`),
whose project it reads as (`ci_target_infra_project`), which repository and which file
it watches (`ci_repo_name`, `ci_estate_file`); the grant asks which runner account may
become the estate's (`ci_runner_service_account`). All five default to the customer-hosted
shape, so an estate hosting its own runner accepts them; an MSP answers the project and
the account. Schedule, time zone, catalog and release are technical defaults, unasked.

## catalogs/

Compliance catalogs (`cis-gcp-4.0.yaml`, `cis-gcp-5.0.yaml`): control ids with this
project's own paraphrases, read by `require` and `report-compliance`. YAML data, not
packs.

`iso27001-2022.yaml` is a **cross-walk**, not a second benchmark. ISO 27001 is a
management-system standard: Annex A names no cloud resource, so a control that the
estate can evidence points at the CIS controls that stand as its evidence
(`evidence: { "cis-gcp/4.0": ["3.1", …] }`) and `require` folds their verdicts. Packs
claim CIS only, so there is one set of witnesses. Two fields exist for
it: `evidence`, and `duties` named on the CONTROL (the human half a config cannot
discharge, which caps the verdict at partial). `automatability: inherited` marks the
provider's own controls under shared responsibility — all of Annex A 7.x — reported so
the Statement of Applicability is complete, never counted as a gap.

The ISO view follows the CIS coverage beneath it: an estate claiming few CIS controls
shows few ISO controls satisfied.

## import-config.yaml

Not a pack: the configuration `satz import` reads — an optional `root` (organization,
folder by id or display-name path, project), `only` and `exclude` lists, and per resource type the
import filter (`import`, `asset_type`, attribute include/exclude) **plus the adoption
rules `satz adopt` reads** — `import_id` templates for user-chosen ids, `match_on` keys
for GCP-assigned ones, `activate: managed` for org policies. A type without a rule is
reported by `adopt` as "no rule"; adding one is a one-line change here. Referenced
automatically from `presets_dir`, or explicitly via `--import-config`.

Its rows are every resource type of the google and google-beta providers at
`provider_version`, the pinned version; `import: true` marks the default set, and
`satz import --all` takes every row the source can deliver.

A row's `skip:` is the live shape's list of what the platform owns and an estate never
declares: glob patterns (`*`) over the resource's own name — a sink's `_Default`, a
service account's email — or, on an IAM row, over the member (`serviceAccount:service-*@gcp-sa-*.iam.gserviceaccount.com`,
`deleted:*`). A match is skipped and listed under its pattern, never dropped in
silence; a copy of the table without the pattern imports it. A key the row does not
know is refused at load, so a misspelt field never passes as an empty one.

`cai-asset-types.txt` beside it is Google's published list of Cloud Asset Inventory
resource types (dated in its header); `scripts/update_import_config.py --cai-types`
fills `asset_type` from it — a derived name is kept only when it is in the list.

`type-map.yaml` beside it is generated by `satz map-types` (never edited by hand):
per resource type the API→Terraform field map the live import applies, aligned from
the API's Discovery Document and the provider schema. Overrides go into
`import-config.yaml` (`api_schema:` to pin an ambiguous schema name).

## Changelog

One row per pack version. The in-file `pack <name> version "<n>"` line is the
source of truth; the smoke matrix fails when a pack's current version has no
row here, so a bump and its reason ship together. Newest first within a pack.
Dates before 2026-08-28 predate the public repository and are given to the day
the private history recorded them.

| pack | version | date | change |
|---|---|---|---|
| `exemptions.exemption_tag` | 2.0 | 2026-09-13 | one value per exemption CLASS instead of a single `not_enforced`: `service-account-keys`, `public-endpoint`, `public-storage`, `vm-image`, `vm-access`, `data-residency`, `encryption`, `network-appliance`. IAM is set on a tag VALUE, so one blanket value meant anyone allowed to exempt anything could exempt everything — the team needing a public bucket could switch off customer-managed encryption just as easily. The classes are deliberately narrow: a wide class is a grant that hands over more than the person asking described. Audit logging, flow logs, DNS logging and domain-restricted sharing carry NO class on purpose — exempting the record of what happened, or letting an outside identity in, is a decision for whoever owns the baseline, not a delegation. The `enforced` value is GONE: its only job was leaving a trace instead of deleting a binding, which the estate's own history already does, and it had no meaning once values became classes |
| `CIS_GCP_Foundation_4_0` | 2.14 | 2026-09-17 | the pack declares its own `google_org_policy_policy { … }` and is `use`d bare at the top level, gated on `use_cis_baseline`, exactly like every CIS extension. Nothing emitted changes — both `use` forms resolve to the same addresses and the same manifest — so an estate's plan does not move. What changes is that the baseline is a pack like the others: the interview can switch it on, the compile reports it when its answer is true and its line is not in, and satz-studio lists it. An estate that keeps the old `google_org_policy_policy { use … }` wrapper is refused, because as a map's content the pack's type key would be read as a label and the whole baseline would collapse into one resource |
| `CIS_GCP_Foundation_4_0` | 2.13 | 2026-09-13 | claims CIS 5.0 §2.14, Cloud Asset Inventory enabled — the last technical control of CIS 5.0 with no claim anywhere in the library. The estate already satisfied it: the scaffold enables `cloudasset.googleapis.com` in every infrastructure project, so the witness is the scaffold's own `google_project_service.infra_cloudasset_googleapis_com` rather than a second `google_project_service` declared here — two resources enabling one API on one project is a duplicate, not a merge. The address depends on the `infra` project label, which is already a contract (`bootstrap` imports by it) and is now held by the init-template test, so renaming it breaks a test rather than a customer's report |
| `CIS_GCP_Foundation_4_0` | 2.12 | 2026-09-13 | `iam.managed.disableServiceAccountKeyCreation` takes its rules from `cis_sa_key_creation_rules` instead of writing them in place, so an estate can let ONE service account out with a tag condition without forking the pack. The default is the plain enforcing rule and the emitted policy is unchanged for an estate that says nothing. The one constraint here with a rules param, because it is the one organisations actually have to exempt — Google ships their own built-in exemption tag for it — and because a param per constraint would put forty list-of-object blocks into every estate's `terraform.tfvars` for a case nobody has |
| `exemptions.exemption_tag` | 1.0 | 2026-09-13 | first version: the VOCABULARY for a tag-conditional exemption — one organisation tag key `<shortname>-exemption` with the values `enforced` and `not_enforced`, and nothing bound to either. An organisation policy is all-or-nothing per node, so letting one service account out of a control means lowering the policy for a whole folder and raising it again — a window during which nothing is enforced. A Resource Manager tag is IAM-governed and a policy rule can condition on it, which is how Google ships `iam.disableServiceAccountKeyCreation` themselves. The pack ships the ABILITY and no exemptions: a library that ships convenient exemptions lowers the baseline by default. The binding that exempts a resource and the condition on the constraint that honours it are the estate's, and the pack header shows both |
| `estate_map` | 1.8 | 2026-09-17 | the CIS baseline joins the map as `use_cis_baseline`, defaulting to true. It was the one pack the map did not declare — the skeleton wrote its line as fixed text, so the interview could not switch it on, `merge-presets` could not add it to an estate that lacked it, and the compile could not report it missing. Framing it as a choice does not make it optional: the question says the estate exists for these thirty policies, and its `why` says what turning it off would take off the organisation. What it buys is that the baseline is adopted, reported and listed by the same machinery as every other pack |
| `estate_map` | 1.7 | 2026-09-13 | one more choice: `use_exemption_tag`, gating `exemptions/exemption-tag.satz`. Its `why` carries the question that usually ends the conversation — does the consumer need a key at all, when a workload in Google Cloud, a Cloud Run service and external CI can all federate instead |
| `CIS_GCP_Foundation_4_0` | 2.11 | 2026-09-13 | six dry-run params and their questions: `cis_api_key_services_dry_run`, `cis_block_project_ssh_keys_dry_run`, `cis_bucket_retention_dry_run`, `cis_cloud_sql_hardening_dry_run`, `cis_cloud_sql_iam_and_deletion_protection_dry_run`, `cis_confidential_computing_dry_run`. Each gates the dry-run twin of the extension it names, to be turned on INSTEAD of the enforcing flag — both at once declares the same policy twice and is refused. The other five extensions have no dry-run form: Shielded VM and both CMEK constraints are legacy, Access Approval is not an org policy, and the two on-by-default extensions have nothing to size |
| `cis_extensions.api_key_services_dry_run` | 1.1 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `api-key-services.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `cis_extensions.block_project_ssh_keys_dry_run` | 1.0 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `block-project-ssh-keys.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `cis_extensions.bucket_retention_dry_run` | 1.2 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `bucket-retention.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `cis_extensions.cloud_sql_dry_run` | 1.1 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `cloud-sql.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `cis_extensions.cloud_sql_iam_and_deletion_protection_dry_run` | 1.0 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `cloud-sql-iam-and-deletion-protection.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `cis_extensions.confidential_computing_dry_run` | 1.0 | 2026-09-13 | first version, GENERATED by `scripts/build_dry_run_fragments.py` from `confidential-computing.satz` — do not edit. The same constraint with `dry_run_spec` instead of `spec` and every claim dropped: Google evaluates each rule, logs every action it would have blocked, and blocks none, so the violation count sizes the control against a live organisation before it bites. It discharges nothing while it runs and carries no claim, so `require` reports the control unmet, which is the truth. Its version tracks the fragment it is derived from |
| `integrations.microsoft_sentinel` | 1.0 | 2026-09-12 | first version: Sentinel's GCP federation — pool, the provider trusting Microsoft's commercial tenant with the `api://` audience, the connector's service account and `roles/iam.workloadIdentityUser` for the pool's principal set. Transcribed from Microsoft's own Terraform against the pinned provider: upstream pins google 3.73.0 and uses authoritative `google_project_iam_binding`, which removes grants an estate made |
| `integrations.microsoft_sentinel_network_logs` | 1.0 | 2026-09-12 | first version: the four network streams — VPC flow logs, firewall rules logging, DNS queries, Cloud NAT — each with its own organisation sink, topic, subscription, publisher grant for the sink's writer identity and subscriber grant for the connector. On by default with Sentinel: each stream is empty until the feature is enabled per subnet, rule, policy or gateway, and routing costs nothing, so switching them off saves nothing and risks the day somebody enables flow logs. Filters select one stream each (`log_id` where Google publishes the log name, the documented `dns_query` resource type for DNS) rather than Microsoft's mix of stream plus the same service's audit records, which the audit fragment already carries. Grants are non-authoritative: upstream's `google_project_iam_binding` would have had the second stream applied remove the first's publisher grant, stopping delivery silently |
| `integrations.microsoft_sentinel_auditlogs` | 1.0 | 2026-09-12 | first version: the first log source — an organisation sink with `include_children` for the four audit streams, its topic, the subscription Sentinel pulls from, `roles/pubsub.publisher` for the sink's writer identity and `roles/pubsub.subscriber` for the connector on that one subscription. Tighter than upstream, which grants a project-level custom role over every subscription in the project. The filter is asked: Data Access logs are most of the volume and Sentinel bills by the gigabyte |
| `monitoring.organization_audit_logsink` | 1.4 | 2026-09-12 | `logsink_project_name` becomes **`logsink_project_id`**, because that is what it is — it feeds `project_id`, and a project id is immutable while a name is not. The project's display name is its own optional param, `logsink_project_display_name`, defaulting to the id exactly as Google does, so nothing changes in the emitted HCL. An estate still binding the old name is REFUSED by name with the new one: nothing refuses a param no pack reads, so leaving it would have silently taken this pack's default project instead — a second logging project and an orphaned archive |
| `monitoring.organization_cis_log_alerts_central` | 1.6 | 2026-09-12 | follows the rename: the alert project defaults to `logsink_project_id` |
| `integrations.microsoft_sentinel` | 1.1 | 2026-09-12 | follows the rename: the Sentinel project defaults to `logsink_project_id` |
| `cis_extensions.internet_ssh_rdp` | 1.1 | 2026-09-12 | ON by default (CIS pack 2.10), with the corrections a default-on pack needs. The pass list gains the three RFC1918 blocks beside the IAP range, because the deny matches `0.0.0.0/0` — every address, private ones included — and a hierarchical policy is read before the VPC rules: with IAP alone, SSH between two instances in one subnet was denied. IPv6 gets its own pass rule (IAP's `2600:2d00:1:7::/64` and `fc00::/7`), since a rule's sources may not mix families. Every rule now carries the control's whole protocol set — SSH on TCP 22 and SCTP 22, RDP on TCP 3389 and UDP 3389 — a TCP-only deny left UDP 3389 open. And the two DENY rules log: Google forbids logging on `goto_next`, so an accepted IAP session leaves no firewall record and only refusals do |
| `cis_extensions.dns_logging` | 1.0 | 2026-09-12 | first version: CIS 5.0 §2.13, the half an org policy can carry — a custom constraint on `dns.googleapis.com/Policy` requiring `enableLogging`. ON by default. `contributes`, not `implements`: no org policy can require that a network HAS a DNS policy, only that a policy which exists logs, so the missing half is named as a duty and verified live |
| `CIS_GCP_Foundation_4_0` | 2.10 | 2026-09-12 | `cis_block_internet_ssh_rdp` defaults to TRUE, the second flag to do so. An estate taking this version emits an organisation firewall policy it did not have: ports 22 and 3389 are denied from public addresses, the private ranges and IAP pass to the VPC rules, and the denies log. Answering no is a deviation whose reason the compliance report carries (ADR 0012) |
| `CIS_GCP_Foundation_4_0` | 2.9 | 2026-09-12 | one new flag, and the first that defaults to TRUE: `cis_dns_logging`, for the new `cis-extensions/dns-logging.satz`. It asks what the control covers and what it breaks; answering no is a deviation whose reason the compliance report carries |
| `s2_security_groups` | 1.2 | 2026-09-11 | the security-admins group's description says what its roles do — organisation policies, folder IAM, Security Command Center, logging and monitoring, read access — instead of the Security Admin role and organisation, folder and project IAM admin, which the group never held. An in-place description update on the group; no role changes |
| `s1_security_groups` | 1.2 | 2026-09-11 | the security-admins group's description says what its roles do — organisation policies, folder IAM, Security Command Center, logging and monitoring, read access — instead of the Security Admin role and organisation, folder and project IAM admin, which the group never held. An in-place description update on the group; no role changes |
| `s1_group_definitions` | 1.4 | 2026-09-11 | the security-admins group's description says what its roles do — organisation policies, folder IAM, Security Command Center, logging and monitoring, read access — instead of the Security Admin role and organisation, folder and project IAM admin, which the group never held. An in-place description update on the group; no role changes |
| `CIS_GCP_Foundation_4_0` | 2.8 | 2026-09-11 | three more opt-in flags with their questions — `cis_access_approval`, `cis_block_internet_ssh_rdp`, `cis_cloud_sql_iam_and_deletion_protection` — for the three new `cis-extensions/` fragments. Nothing emitted changes; an estate using the pack has three more questions, all defaulting to off |
| `cis_extensions.access_approval` | 1.0 | 2026-09-11 | CIS 4.0 2.15 / 5.0 2.16, opt-in: Access Approval at the organisation for every supported service; asks for the notification addresses (blocking until named). Needs Access Transparency, which has no provider resource |
| `cis_extensions.internet_ssh_rdp` | 1.0 | 2026-09-11 | CIS 3.6 and 3.7, opt-in: a hierarchical firewall policy on the organisation denies TCP 22 and 3389 from the IPv4 and IPv6 internet and passes the listed ranges (IAP by default) to the VPC rules |
| `cis_extensions.cloud_sql_iam_and_deletion_protection` | 1.0 | 2026-09-11 | CIS 5.0 6.6 and 6.9, opt-in: two custom constraints on Cloud SQL instances — IAM database authentication on (SQL Server exempt), deletion protection on — each enforced by a policy on the organisation |
| `estate_map` | 1.6 | 2026-09-12 | the two Sentinel log paths default to `use_sentinel` BY REFERENCE, so a customer who connects Sentinel and accepts the defaults gets the logs it exists to read; `use_sentinel_network_logs` is the new one, and either can be answered `false` to leave that path out |
| `estate_map` | 1.5 | 2026-09-12 | `use_sentinel` and, behind it, `use_sentinel_auditlogs`: a customer's SIEM is a choice the interview makes, not a fragment somebody remembers to wire |
| `estate_map` | 1.4 | 2026-09-12 | `use_scc_findings_siem` beside the mailbox choice, asked with it when the topic is on: a customer with a SIEM answers where findings go without being asked for an address nobody reads |
| `estate_map` | 1.3 | 2026-09-12 | Security Command Center is one decision with follow-ups: `use_scc_enablement` carries what the Premium tier costs (per covered resource-hour, not a share of the bill) and that the 30-day trial becomes pay-as-you-go by itself, and `recommend = true` offers it — the param default stays off, so `--accept-defaults` never switches a paid service on. `use_scc_notifications` and `use_scc_export` are asked only when enablement is on (`ask_when`), and `use_scc_findings_mail` only when the topic is |
| `estate_map` | 1.2 | 2026-09-12 | one more choice: `use_scc_export`, the BigQuery dataset findings are kept and queried in |
| `estate_map` | 1.1 | 2026-09-12 | one more choice: `use_scc_notifications`, the Pub/Sub chain that carries Security Command Center findings out of the console. Off by default like the enablement choice beside it — it needs SCC switched on to have findings to publish |
| `estate_map` | 1.0 | 2026-09-10 | first version: which packs make up the estate, as questions — the S1/S2 model as a `oneof` (moved here from estate-core) and one boolean per optional pack, four on by default (audit archive, central alerts, billing permissions, essential contact), five off (budget, SCC enablement, security-audit account, Defender, verification runner). Declares the choices only; the estate carries the `use … when` lines, which the interview skeleton writes and a test keeps in step (ADR 0006) |
| `estate_core` | 2.0 | 2026-09-10 | the security-model choice moves to `estate_map`; this pack is the seventeen day-0 params and their questions, nothing else. A major bump because two params left — no estate in the fleet uses the pack, it exists for interview skeletons |
| `cis_extensions.cmek` | 1.1 | 2026-09-10 | two `question` blocks: the services that must use a CMEK and the projects that may supply keys — both refuse resource creation when wrong. Nothing emitted changes |
| `cis_extensions.bucket_retention` | 1.2 | 2026-09-10 | one `question` block on the allowed durations: every bucket on another duration becomes un-updatable once enforced. Nothing emitted changes |
| `cis_extensions.api_key_services` | 1.1 | 2026-09-10 | one `question` block on the allowed services; the empty default blocks on purpose — it is a legitimate answer, but it has to be the customer's. Nothing emitted changes |
| `sa_security_audit` | 1.1 | 2026-09-10 | three `question` blocks — the hosting project (no default, blocks), the account id, the auditors group; each a recreate. The display name is not asked. Nothing emitted changes |
| `integrations.microsoft_defender_for_cloud` | 0.2 | 2026-09-10 | four `question` blocks: the two ids only Microsoft's wizard knows (both block until typed), whether CSPM is licensed, and — only when it is — the access mode as a `oneof` under `ask_when`, the library's first gated choice. Nothing emitted changes |
| `essential_contacts_organization` | 1.3 | 2026-09-10 | one `question` block on the contact address: Google's suspension, security and legal notices go there and nowhere else. Nothing emitted changes |
| `billing_account_permissions` | 1.2 | 2026-09-10 | one `question` block on the billing-admins group: it can move projects between billing accounts and see every cost. Nothing emitted changes |
| `s2_security_groups` | 1.1 | 2026-09-10 | six `question` blocks, one per group name — each a group's identity, so changing it later is a new group, moved members and re-granted roles. Nothing emitted changes |
| `s1_security_groups` | 1.1 | 2026-09-10 | five `question` blocks, one per group name, same reasoning. Nothing emitted changes |
| `s1_group_definitions` | 1.3 | 2026-09-10 | five `question` blocks, one per group name, same reasoning. Nothing emitted changes |
| `ci.verification_runner` | 1.1 | 2026-09-10 | four `question` blocks — the hosting project, the watched estate's infra project, the repository name, the estate file: what the pack cannot know when an MSP hosts the runner. Schedule, time zone, region, catalog, fail-on and release stay technical defaults. Nothing emitted changes |
| `ci.verification_runner_grant` | 1.1 | 2026-09-10 | one `question` block on the runner service account — the binding IS the pack, and a wrong address hands the estate to an account nobody meant. Nothing emitted changes |
| `monitoring.organization_audit_logsink` | 1.3 | 2026-09-10 | four `question` blocks — the archive project, the bucket, its location, the retention — each with what changing it later costs (the first three are recreates; shortening the retention deletes what is already archived). Sink name and filter stay technical defaults, unasked. Nothing emitted changes |
| `monitoring.organization_cis_log_alerts_central` | 1.5 | 2026-09-10 | two `question` blocks: the alert mailbox (`cis_central_email` — a wrong one drops every alert silently) and the hosting project (`cis_central_bucket_project`, default the logsink pack's project by reference). Nothing emitted changes |
| `project_cis_log_alerts` | 1.1 | 2026-09-10 | two `question` blocks: the one project this use watches, and the alert recipient's local part. Nothing emitted changes |
| `CIS_GCP_Foundation_4_0` | 2.7 | 2026-09-10 | ten `question` blocks: the seven opt-in controls and the three lists a customer decides (locations, principal sets, subjects), each with the sentence that says what breaks when the answer is wrong. Nothing emitted changes; an estate using the pack has ten questions to answer before bootstrap or apply — all with defaults, so `satz interview --accept-defaults` settles them in one pass. Not asked: the protocol-forwarding schemes and the contacts domain, which are technical defaults rather than decisions |
| `estate_core` | 1.0 | 2026-09-09 | first version: the seventeen day-0 params `satz init` writes, each with its `question` — what to ask, why, and what changing it later costs — plus the security-group model as two booleans and a `question oneof`. Emits nothing; exists so an interview (`satz interview --create`, the MCP tool `satz_interview`) has something to ask before an estate exists. Seven params have no possible default and block until typed; the rest offer one, and a derived default (`"{customer_shortname}-infra-001"`) is offered only once its inputs are answered |
| `ci.verification_runner` | 1.0 | 2026-09-09 | first version: continuous verification as a pack. Two Cloud Build triggers in the hosting project — `satz-check` on every push (`transpile --check`) and `satz-compliance` nightly via Cloud Scheduler (`report-compliance --fail-on`) — plus the runner service account and its two project roles. Build steps are INLINE in the trigger, not a file in the watched repository, so control of the pipeline follows ownership of the service account; satz is installed at build time from the release (`ci_satz_release`, default `latest`). The runner never acts as itself — satz exchanges its identity for the estate's IaC account, which the companion grant pack permits. v1 reports through the exit code and log; no evidence write-back |
| `ci.verification_runner_grant` | 1.0 | 2026-09-09 | first version: the one binding a verification runner needs — `roles/iam.serviceAccountTokenCreator` on the estate's IaC service account, and nothing on the organisation. Separate from the runner pack because in the MSP-hosted shape the two resources belong to two parties: the runner in the MSP's project, this grant on the customer's account, applied by the customer. Default names the runner pack's own account, so a customer-hosted estate using both wires nothing |
| `CIS_GCP_Foundation_4_0` | 2.6 | 2026-09-08 | `gcp.resourceLocations` becomes the `allowed_resource_locations` param (default = the two multi-region groups it always emitted, so no estate changes on upgrade) — a hard-coded value silently widened a policy an operator had narrowed by hand. And the six superseded legacy blocks take a `-superseded` address suffix, which makes the switch to `spec { reset = true }` a REPLACE by construction: the provider PATCHes the rules it holds together with `reset` and the API refuses the pair (`400 Cannot set PolicyRules if reset is true`), so the in-place form v2.5 assumed never worked. Estates upgrading from 2.4 or 2.5 see one destroy + create per legacy policy, in the plan, instead of needing `tofu apply -replace=` by hand |
| `monitoring.organization_cis_log_alerts_central` | 1.4 | 2026-09-08 | the alert project defaults to `logsink_project_name` — the audit-logsink pack's own param, BY REFERENCE — so an estate using both packs wires nothing. The old default was the literal `{customer_shortname}-organization-log-alerts`, a project nothing creates, so an estate that did not override it pointed eight alert policies at a project that was never there. Used without the logsink pack the name is undeclared and the pack stops with `unknown param`, which is the honest failure: the alert project is then genuinely undecided |
| `scc_findings_siem` | 1.0 | 2026-09-12 | first version: the SIEM's own pull subscription on the findings topic and `roles/pubsub.subscriber` for the identity it reads as — without that grant a connector authenticates and reads nothing. The identity is asked and has no default: defaulting it would tie SCC to one vendor's pack. Runs alongside the mailbox, each with its own subscription |
| `scc_findings_mail` | 1.0 | 2026-09-12 | first version: who gets told, for an organisation with no SIEM on the topic — a subscription (a topic without one drops every message), an e-mail channel and an alert policy that fires when findings reach the topic. Asks the address; its default is the central alert pack's `cis_central_email` by reference, and without that pack the compile stops rather than mailing a guessed address. The mail says findings arrived and links to them: the finding's text stays in the topic, the console and the export |
| `scc_export` | 1.1 | 2026-09-12 | the export pins its own `name`. The server assigns it and the provider reads it back, so without it in the config every plan proposed to null it and the API refused the update ("Field name is immutable") — a permanent diff. Measured on a live organisation |
| `scc_export` | 1.0 | 2026-09-12 | first version: findings exported to BigQuery — the API in the dataset's project, the dataset (`delete_contents_on_destroy` false, so removing the pack does not delete the history), the exporting agent's `dataEditor` on it, and the v2 export. The dataset takes its project through the service resource, so the API is enabled first; even then a first apply can fail while BigQuery's control plane catches up, and the second succeeds. Asks the project and the location; no claim |
| `scc_notifications` | 1.2 | 2026-09-12 | the two questions say what the answer decides — which project holds the topic (and that moving it later is a new topic with a subscriber to repoint), and whether everything travels or only what somebody would act on tonight. No emission change |
| `scc_notifications` | 1.1 | 2026-09-12 | the grant follows the publisher: `gcp-sa-scc-notification`, the identity the notification config reports, not the `security-center-api` agent. Measured on a live organisation — with the wrong agent the config publishes nothing and says nothing |
| `scc_notifications` | 1.0 | 2026-09-12 | first version: the notification chain downstream of enablement — a Pub/Sub topic, `google_scc_v2_organization_notification_config` (v2: the v1 API answers "This API is no longer available" on a live organisation) and `roles/securitycenter.notificationServiceAgent` for `service-org-<org>@security-center-api.iam.gserviceaccount.com` on that topic, without which the config publishes nothing. Asks the topic's project and the finding filter; sends active HIGH and CRITICAL findings by default. No claim — no catalog control covers SCC |
| `scc_service_enablement` | 1.2 | 2026-09-12 | the optional-detector question names what each of the two does — Web Security Scanner sends real requests at whatever is listening, Artifact Analysis is billed per image — and spells out the four answers. No emission change |
| `scc_service_enablement` | 1.1 | 2026-09-12 | `scc_optional_services` (asked): `leave`, `all`, `none`, or the ones it names, for the two detectors outside the baseline — Web Security Scanner, which crawls the customer's web applications, and Artifact Analysis, billed per image scan. The script could only ever switch services ON, so an opt-in enabled by hand in the console stayed on for ever; `disable` is how an estate takes them back |
| `scc_service_enablement` | 1.0 | 2026-09-04 | first version: no resources, one `action` binding `scc/scc-enable-all.sh`. SCC service enablement and tier activation have no provider resource (7.14.1 ships 35 `google_scc_*`/`google_securityposture_*` types and none of them is enablement), so the estate declares the step and `satz run-actions` runs it with the org id the estate already carries. `phase = "before-apply"`; everything downstream of enablement stays for a later pack |
| `CIS_GCP_Foundation_4_0` | 2.5 | 2026-09-04 | runs the MANAGED protocol-forwarding constraint (`parameters.allowedSchemes`, param `allowed_protocol_forwarding_schemes`) and declares all six superseded legacy twins OFF with `reset = true`, so no estate ends up with both forms enforcing |
| `cis_extensions.cloud_sql` | 1.1 | 2026-09-04 | declares its two superseded legacy twins (`sql.restrictAuthorizedNetworks`, `sql.restrictPublicIp`) off |
| `cis_extensions.bucket_retention` | 1.1 | 2026-09-04 | declares its superseded legacy twin (`storage.retentionPolicySeconds`) off |
| `CIS_GCP_Foundation_4_0` | 2.4 | 2026-09-04 | adds `compute.managed.disableSerialPortAccess` (4.5) to the baseline — safe by default — and declares the seven opt-in flags the `cis-extensions/` fragments are gated on |
| `cis_extensions.block_project_ssh_keys` | 1.0 | 2026-09-04 | CIS 4.3, opt-in: the managed constraint is still PREVIEW and has no legacy equivalent |
| `cis_extensions.shielded_vm` | 1.0 | 2026-09-04 | CIS 4.8, opt-in: image support required, and the only constraint here with no managed form and no dry-run |
| `cis_extensions.confidential_computing` | 1.0 | 2026-09-04 | CIS 4.11, opt-in: Confidential VMs are machine-family limited, so enforcing it org-wide stops ordinary workloads |
| `cis_extensions.cloud_sql` | 1.0 | 2026-09-04 | CIS 6.5 and 6.6/6.7 (renumbered in 5.0), opt-in: existing public-IP instances lose connectivity |
| `cis_extensions.cmek` | 1.0 | 2026-09-04 | CIS 7.2, 7.3 and 8.1, opt-in: two LIST constraints; the keys and grants must exist first, and the key-project value takes a resource PATH |
| `cis_extensions.api_key_services` | 1.0 | 2026-09-04 | CIS 4.0 1.14 / 5.0 1.15, opt-in: a managed constraint with an `allowedServices` parameter, not a bare boolean |
| `cis_extensions.bucket_retention` | 1.0 | 2026-09-04 | CIS 4.0 2.3 / 5.0 2.4 as a `contributes`, opt-in: constrains EVERY bucket's retention duration, and locking stays a human decision |
| `CIS_GCP_Foundation_4_0` | 2.3 | 2026-09-03 | claims the SAME resources against CIS 5.0 as well as 4.0 — no second pack, because 5.0's org-policy content is identical and only renumbered (1.1→1.2, 1.4→1.5, 1.5→1.6, 1.16→1.17, 3.8→3.10; §2, §4, §5 unchanged). Plus a new `5.0 1.1.4 implements` over the whole baseline: the control asks whether the organisation constrains its projects centrally, which is what the pack is |
| `integrations.microsoft_defender_for_cloud` | 0.1 | 2026-09-03 | first cut — the foundation of Microsoft's GCP onboarding as Satz: management project + its API set, the workload identity pool, the auto-provisioner plan and its custom role. Transcribed from a customer's generated wizard Terraform; Microsoft's own tenant, application-id audiences, provider ids and role ids are inlined constants, the customer's Entra tenant and the management project id are params |
| `integrations.microsoft_defender_for_cloud_cspm` | 0.1 | 2026-09-03 | first cut — the CSPM plan behind `mdc_plan_cspm`: its service account, OIDC provider, workload-identity assignment and org grants. The custom role is not here: it depends on the access mode |
| `integrations.microsoft_defender_for_cloud_cspm_role_default` | 0.1 | 2026-09-03 | first cut — the CSPM custom role in DEFAULT access mode: five permissions beside the `roles/viewer` the plan grants |
| `integrations.microsoft_defender_for_cloud_cspm_role_least_privilege` | 0.1 | 2026-09-03 | first cut — the CSPM custom role in LEAST PRIVILEGE mode: the 82 permissions Microsoft's script enumerates in place of viewer's reach. Use this or the default role, never both |
| `CIS_GCP_Foundation_4_0` | 2.2 | 2026-09-01 | `essential_contacts_allowed_domains` becomes a LIST param with structured `parameters` (was the singular `essential_contacts_allowed_domain` inside a JSON string) — several contact domains no longer fork the pack; estates that bound the singular param bind the list instead |
| `CIS_GCP_Foundation_4_0` | 2.1 | 2026-08-24 | `allowed_policy_member_subjects` default gains the fifth SCC service agent; structured `parameters` on the managed §1.1 policy |
| `CIS_GCP_Foundation_4_0` | 2.0 | 2026-08-23 | retires the legacy `iam.allowedPolicyMemberDomains` (and its `allowed_policy_member_customers` param) in favour of the managed `iam.managed.allowedPolicyMembers`; the §1.1 claim carries `duty_legacy_superseded` |
| `CIS_GCP_Foundation_4_0` | 1.6 | 2026-08-23 | `allowed_policy_member_subjects` param: the canonical SCC service agents allowlisted under the managed §1.1 constraint |
| `CIS_GCP_Foundation_4_0` | 1.5 | 2026-08-22 | the 23-control catalog; claims for every control the pack implements |
| `CIS_GCP_Foundation_4_0` | 1.4 | 2026-08-22 | `essential_contacts_allowed_domain` param (driven by E03's conversion) |
| `CIS_GCP_Foundation_4_0` | 1.3 | 2026-08-21 | subjects param on `iam_managed_allowedPolicyMembers` (`allowedMemberSubjects` explicit) |
| `CIS_GCP_Foundation_4_0` | 1.2 | 2026-08-20 | pristine baseline as converted to Satz |
| `s1_group_definitions` | 1.2 | 2026-08-28 | group `lifecycle { ignore_changes = [initial_group_config] }` — an adopted group no longer plans as "must be replaced" |
| `s1_group_definitions` | 1.1 | 2026-08-21 | ships NO human memberships — presets define groups, humans grant membership |
| `s1_group_definitions` | 1.0 | 2026-08-20 | the five S1 admin groups |
| `s1_security_groups` | 1.0 | 2026-09-02 | the S1 model in ONE typed file (groups + org grants) for top-level `use`; content-identical to `s1_group_definitions` 1.2 + `s1_group_permissions` 1.1, which stay for the under-a-type spelling — an estate takes one of the two, never both |
| `s2_security_groups` | 1.0 | 2026-09-02 | S2 = S1 plus a distinct `gcp-network-admins` group (`compute.networkAdmin`, `compute.xpnAdmin`, `compute.securityAdmin`, `dns.admin`, `networkconnectivity.hubAdmin`, `networkmanagement.admin` + viewer roles); project-admins lose `compute.networkAdmin` and `compute.xpnAdmin`; one typed file |
| `s1_group_permissions` | 1.1 | 2026-09-02 | `roles/cloudasset.viewer` for security-admins and security-viewers — `report-compliance` reads witnesses through Cloud Asset Inventory and `iam.securityReviewer` does not carry the search permissions |
| `s1_group_permissions` | 1.0 | 2026-08-20 | org-level role grants for the S1 groups; `roles/viewer` for the security-viewers group is the fleet standard |
| `essential_contacts_organization` | 1.2 | 2026-09-02 | commented per-category contacts (BILLING, SUSPENSION, SECURITY, TECHNICAL, LEGAL, PRODUCT_UPDATES, and a multi-category example) with their own address params, ready to uncomment; the shipped shape is unchanged (one contact on ALL) |
| `essential_contacts_organization` | 1.1 | 2026-08-23 | `essential_contacts_email` param — a customer pins its contact without a fork; content pack |
| `essential_contacts_organization` | 1.0 | 2026-08-20 | organization-wide essential contact, all categories |
| `monitoring.organization_audit_logsink` | 1.2 | 2026-09-03 | CIS 5.0 claim ids corrected: sinks are 5.0 §2.3 and retention §2.4 (5.0 inserted a new §2.2 for Workspace data sharing); 4.0 ids unchanged; "provisional" notes removed — numbering verified against Prowler + Tenable |
| `monitoring.organization_audit_logsink` | 1.1 | 2026-08-21 | claims for CIS 2.1/2.2 (both 4.0 and 5.0), the writer-identity bucket grant, retention lifecycle rules |
| `monitoring.organization_audit_logsink` | 1.0 | 2026-08-21 | org audit log sink → bucket |
| `monitoring.organization_cis_log_alerts_central` | 1.3 | 2026-09-03 | **CIS 4.0 claim ids were off by one** — the eight alert controls are 4.0 §2.4–2.11 (§2.12 is DNS logging), not §2.5–2.12; the invented "§2.4 filters exist" claim is gone and the sink + channel now CONTRIBUTE to the first alert control (4.0 §2.4 / 5.0 §2.5). Resource labels keep the 5.0 numbers. Verified against Prowler, Google's InSpec profile and Tenable |
| `monitoring.organization_cis_log_alerts_central` | 1.2 | 2026-08-24 | the log metric + alert stack for CIS 2.5–2.12 in one central logging project |
| `monitoring.organization_cis_log_alerts_central` | 1.1 | 2026-08-22 | notification channel param; alert policy display names carry the control id |
| `monitoring.organization_cis_log_alerts_central` | 1.0 | 2026-08-21 | first version |
| `project_cis_log_alerts` | 1.0 | 2026-08-21 | per-project variant of the CIS 2.5–2.12 metrics + alerts |
| `sa_security_audit` | 1.0 | 2026-08-21 | read-only security-audit service account with its custom role |
| `billing_account_permissions` | 1.1 | 2026-09-01 | split by audience: the domain gets `billing.user` + `billing.viewer`; a `billing_admins_group` param (default `gcp-billing-admins@{customer_domain}`) gets `billing.admin` + `billing.costsManager`; the IaC SA keeps `billing.admin`. Adoption adds three grants per estate — a real plan |
| `billing_account_permissions` | 1.0 | 2026-08-20 | billing-account IAM for the S1 groups and the IaC service account |
| `organization_budget` | 1.0 | 2026-08-20 | organization budget with threshold alerts (`"import-id"` example) |
