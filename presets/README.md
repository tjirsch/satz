# satz library

Every pack is a `.satz` file: `pack <name> version "<v>"`, a `params { … }` block
of overridable defaults, and the resources it contributes. Estates `use` them.

Presets are **read-only building blocks**: use them from a customer's estate and
set every org-specific value there — never by editing a preset.

`use "presets/<pack>.satz"` at top level for packs that declare their own
resource-type maps — the CIS baseline and its extensions, and most of the library —
or inside a resource map for the packs that are a bare list of labels
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
sentence the index can print, its version has a changelog row below, it carries no value
shaped like private data that is not a documented example value, it declares no
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

**Use** — `logsink_project_folder` says where the destination project is created, so
the line stands at the top level:

```
params {
  logsink_project_folder = "google_folder.shared_services.name"
}

google_folder {
  shared_services {
    display_name = "Shared Services"
  }
}

use "presets/monitoring/organization-audit-logsink.satz" when logsink_project_id
```

**Overridable defaults** (names are derived from `customer_shortname`, so they are
globally unique without overrides):

| Param | Default | Meaning |
|---|---|---|
| `logsink_project_id` | `"{customer_shortname}-log-infra-001"` | project_id of the destination project |
| `logsink_project_folder` | `""` | the folder the destination project is created in — a folder the estate declares by reference (`google_folder.<label>.name`), one that already exists by its id. Empty says nothing and the project is created under the organisation |
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
seventeen `satz init` writes, each with a `question`: what to ask, why,
and what changing the answer later costs. Which packs make up the estate is the next
pack, `estate-map.satz`; this one is the day-0 params and nothing else.

```
use "presets/estate-core.satz"
```

The pack **declares no resource**; it holds the day-0 questions and the core
exports. `satz interview <estate> --create` and the MCP tool `satz_interview` write an
estate that uses it, with every question open. An estate written by `init` does not
need it for its params: `init` binds every param from its flags, and a bound param is an
answered question.

**The core exports.** An estate that uses the pack publishes these to the HCL teams
write beside it — outputs of `hcl/outputs.tf` and of every module under `hcl/interfaces/`
([workflows](../docs/workflows.md#customer-teams-beside-the-estate)), all known at
compile time:

| export | value |
|---|---|
| `organization_id` | `customer_organization_id` |
| `customer_domain` | `customer_domain` |
| `customer_shortname` | `customer_shortname` |
| `default_region` | `default_region` |
| `infra_project_id` | `infra_project_name` |
| `iac_service_account` | `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com` |

`satz init` writes one export into the estate itself, `infra_folder`, the
infrastructure folder's `folders/<number>`, which the interface module looks up.

Two kinds of param, and the interview treats them differently:

| kind | params | in the report |
|---|---|---|
| **no possible default** | `customer_id`, `customer_organization_id`, `customer_domain`, `customer_shortname`, `customer_longname`, `first_admin`, `billing_account_infra` | `blocking: true` — a value has to be typed |
| **derived or conventional** | `infra_folder_name`, `infra_project_name`, `infra_bucket_name`, `svc_iac_account`, `svc_iac_users_group`, `deployment_engine`, `deployment_mode`, `default_region`, `default_zone`, `compliance_frameworks`, the security model | `default` offered — accepting it is an answer, recorded by writing it |

`compliance_frameworks` is the one param here that is neither derived nor a
convention: it is what the customer ANSWERS TO — a contract, an auditor, a regulator —
as a list of catalog ids (`cis-gcp-4.0`, `cis-gcp-5.0`, `iso27001-2022`), default
`["cis-gcp-5.0"]`. It is not what the estate CLAIMS, which comes from its packs, and a
value naming no catalog is a compile error with the list. `satz report-compliance
<estate>` reports one section per framework named here, `satz prowler` scans for them
beside the frameworks the packs claim, and a pack reads it like any other param.

A derived default is offered only once what it derives from is answered:
`infra_project_name` is `"{customer_shortname}-infra-001"`, which with the short name
still open would be `-infra-001`, so it blocks until the short name is typed, then
offers `acme-infra-001`. See [satz interview](../docs/interview.md).

## estate-map.satz

Which packs make up the estate, asked as questions — the map an interview follows
after the day-0 params. One boolean per optional pack plus the S1/S2 model as a
`question oneof`, each with what the pack is for and what turning it off later
destroys. Below the choices, one `offers` entry per pack in the library says the gate its
line carries, the phase that has to be finished before it can go in, the block it
belongs in and — by the entries' order — the order the packs can be adopted; `satz
pack-graph` turns them, with the edges it derives from the packs, into
`presets/pack-graph.json` ([ADR 0031](../docs/adr/0031-the-map-offers-every-pack-and-the-graph-ships-with-the-presets.md)).
The estate carries one `use … when` line per choice, each written commented under its
phase. `satz init` and `satz interview --create` write them from the pack graph,
`satz merge-presets` writes a missing one, `satz interview` and `satz add-pack` switch a
line on when its choice is answered yes, `satz packs` lists every choice with its line, and a
test fails on a choice with no line ([ADR 0007](../docs/adr/0007-the-map-is-a-pack-of-choices-and-the-estate-carries-the-lines.md)).

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
| `use_billing_export` | off | `billing-export` — the project and dataset Cloud Billing exports usage and cost into |
| `use_project_cis_log_alerts` | off | `monitoring/project-cis-log-alerts` — the CIS log alerts inside one project of its own, beside the central ones |
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
| `use_verification_runner` | off | `ci/verification-runner`, the customer-hosted shape |
| `use_verification_runner_grant` | follows `use_verification_runner` | `ci/verification-runner-grant` — the binding that lets a runner act as the estate; answered alone when the runner lives in another estate |
| `use_exemption_tag` | off | `exemptions/exemption-tag` — the tag an exemption is bound to; it exempts nothing on its own |
| `use_interface_notice` | off | `interface-notice` — one Pub/Sub message per apply that changes a value the estate exports |

The CIS baseline is the map's first choice: the skeleton writes its line commented under
its phase, like every pack's, and answering `use_cis_baseline` yes puts it in. It is
asked rather than assumed because its thirty policies reach the organisation in one
apply. Defender's plan fragments are offered with `by_hand`: they are gated on
Defender's own params, and their lines are written by hand beside the Defender line, as
their headers show.

## security-group-models/

**This is where STANDING authority is modelled** — who administers projects, networks,
guardrails, billing, org-wide and continuously. Its counterpart is
[exemptions/](#exemptions), which says who may make a narrow, named EXCEPTION to a
control this authority set, without holding this authority. The two stay apart:
`gcp-security-admins` holds `roles/orgpolicy.policyAdmin` and can rewrite any
policy, and that is a much bigger thing to hand someone than "may let one service account
hold a key".

The security group models: admin groups plus their org-level role grants. Each
model is one typed file, used at the top level of the estate:

- **s1-security-groups.satz** — S1 in ONE typed file (groups AND grants).
  Resource-type sections may repeat across files with distinct ids, so the
  pack's `google_cloud_identity_group { … }` sits beside the estate's own.
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
// one line at the top level of the estate — s1-security-groups or s2-security-groups
use "presets/security-group-models/s1-security-groups.satz"
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

**Notice.** Once the baseline is switched on, `satz adopt <estate> --execute --import`
is to run before the apply: Google sets some of these policies on every new organisation,
and an apply that creates a policy that exists stops on `409 POLICY_ALREADY_EXISTS`. satz
prints that notice when the pack goes on and warns at its `use` line until the estate
binds `cis_baseline_adopted = true`, which the adopt run does itself; `transpile --apply`
and `bootstrap` refuse while it is open. Every org-policy extension carries the same
notice on its own param — each is adopted when it is switched on.

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

**A pack brings its own.** `allowed_policy_member_subjects` is the ESTATE's list, for
its own exceptions. A pack that needs an external principal allowed declares it itself,
as a contribution:

```
params {
  contributes_allowed_policy_member_subjects = [
    "serviceAccount:billing-export-bigquery@system.gserviceaccount.com",
  ]
}
```

A `contributes_<param>` declaration is no param of its own: it is never a variable, the
estate never binds it, and its entries are added to whatever `<param>` holds — the
estate's binding, else the declaring pack's default — while the contributing pack is on.
Switch that pack off and its entries go with it. Two packs contributing the same entry
add it once. Where the contributing line stands decides nothing: the entries are merged
before anything is compiled. Where no pack in the estate declares the param at all —
the CIS baseline is off, so nothing is restricting anyway — the entries are dropped, and
`satz packs` names the requirement. Two packs contribute today: `billing-export` and
`integrations/microsoft-defender-for-cloud`, each named on its page below, and
`satz packs <estate>` prints which pack put which entry in this estate's list.

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
  CRITICAL findings by default). The resource is **v2**: the v1
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

A new action is written in Python unless a shell script is simpler for
the job. The extension decides how satz launches the file: a `.py` action is
spawned as `uv run --script <file>`, runs on every platform satz ships for and
needs no executable bit, while a `.sh` action is refused on Windows before the
spawn.

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

## billing-export.satz

Cloud Billing usage and cost data, exported to BigQuery — what the money went on, per
project, per SKU, per day, queryable months later. Where `organization-budget` alarms
on a threshold, this keeps the record. The pack declares a project of its own, BigQuery's
API on it, the dataset, and Google's export account's `roles/bigquery.dataEditor` on that
dataset; it CONTRIBUTES that account to `allowed_policy_member_subjects`, so
domain-restricted sharing lets the grant through while the pack is on.

The dataset lives in a project of its own rather than in the infrastructure project:
whoever reviews spend reads the billing dataset, and that must not also mean reading
the state bucket.

**Use** (root level): `use "presets/billing-export.satz" when use_billing_export`

**Overridable defaults:**

| Param | Default | Meaning |
|---|---|---|
| `billing_export_project_id` | `{customer_shortname}-billing-001` | the export's own project |
| `billing_export_project_folder` | `""` | the folder it is created in; empty is the organisation |
| `billing_export_project_display_name` | the project id | what the console shows |
| `billing_export_dataset_id` | `{customer_shortname}_billing_export` | the dataset; a dataset id takes underscores, never hyphens |
| `billing_export_location` | `default_region` | where the dataset lives; it cannot move |
| `billing_export_description` | a sentence | the dataset's description |

**Notice.** Cloud Billing has no API, no gcloud command and no Terraform resource for
switching the export on. Apply the pack, then in the Cloud console open Billing, pick the
billing account, go to Billing export, BigQuery export, and point Standard usage cost at
the project and dataset the params name. The dataset stays empty until that is done, and
the notice stays open until the estate binds `billing_export_enabled = true`. It is a
warning, not an error: the apply that creates the dataset has to come first.

**Questions.** Four: the project, the folder it lands in, the dataset and its location.
The project, the dataset and the location cannot be changed afterwards.

**No claim.** No catalog control covers billing export.

## essential-contacts-organization.satz

One organization-level Essential Contact subscribed to ALL notification categories.
A bare list of labelled contacts: use it inside the resource map. Its `params` and its
`question` reach the estate from there; the map receives the contacts alone.

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
what Microsoft's wizard uses as the pool id), `mdc_mgmt_project_id`,
`mdc_mgmt_project_folder` (the folder that project is created in — a folder the estate
declares by reference, `google_folder.<label>.name`, or one that already exists by its id;
empty says nothing and the project is created under the organisation), `mdc_plan_cspm`, and
the access-mode pair `mdc_cspm_default_access` / `mdc_cspm_least_privilege`. Everything
Microsoft-side — their tenant as the OIDC issuer, the per-plan `api://` audiences, the
provider ids, the custom role ids, the API list — is an inlined constant, identical for
every customer and not a param.

**No claim.** Defender for Cloud is an external CSPM that reads the estate. It implements
no CIS control and contributes to none, so the pack asserts nothing.

**Two prerequisites before the first apply.** The Defender agentless-scanning service
account lives in a Microsoft project, and the pack CONTRIBUTES it to `allowed_policy_member_subjects`
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

## Notices

A pack can name ONE command to run once it is switched on, beside its questions:

```
params {
  cis_baseline_adopted = false
}

notice cis_baseline_adopted {
  text     = "Google sets some of these policies on every new organisation …"
  run      = "satz adopt <estate> --execute --import"
  severity = error
}
```

satz shows it when the pack goes on — the interview's yes, `satz add-pack`,
`merge-presets` bringing the pack in — and the compile warns at the estate's `use` line
until the estate binds the param `true`. `severity = error` makes every command that
writes to the organisation refuse while it is open (§6.15 of the language reference lists
the three severities). The param is the notice's alone: declared `false` in
the same pack, asked by no question, read by nothing, and never emitted — binding it
moves no line of the HCL. `satz packs` lists every pack's notices with their state, and
`satz doc-packs` gives a pack a Notices section.

The CIS org-policy packs carry one each, naming `satz adopt`; a run over every resource
type that resolves everything binds their params itself.

## Superseded legacy constraints

Where Google replaces a legacy org-policy constraint with a managed one, a pack runs the
**replacement alone** and declares the legacy twin OFF in the same file:

```
"compute-requireOsLogin-superseded" {
  name   = "compute.requireOsLogin"
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

The classes are narrow, and `vm-image` is separate from `vm-access` for that
reason: what a machine can run and who can get into it are different risks and should be
different grants. A wide class is a grant that hands over more than the person asking for
it described.

**What has no class.** Audit logging (`gcp.detailedAuditLoggingMode`,
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

A binding somebody adds out of band — the temporary kind — is reported by
`satz report-compliance`. It lists every `cloudresourcemanager.googleapis.com/TagBinding`
of the organisation through Cloud Asset Inventory, keeps the bindings of this key,
subtracts the `google_tags_tag_binding` resources the estate declares, and prints the rest
in a section of its own, with each one's value and target. A binding whose value a claimed
control's policy conditions on is also printed under that control
(`**undeclared exemption**: <org>/<shortname>-exemption/service-account-keys bound to …`);
the control's status stays what its witnesses make it.

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
customer who wants a custom sampling rate must pick one of the three or widen the policy.


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

**A twin carries no claim.** A dry run discharges no control
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

## interface-notice.satz

Tells the teams whose HCL reads the estate's interface when an exported value changes.
Gated on `use_interface_notice`, off by default.

```
use "presets/interface-notice.satz" when use_interface_notice
```

In the infrastructure project (`interface_notice_project`) it declares a bucket that is
not the state bucket (`{customer_shortname}-infra-001-interface`), a Pub/Sub topic
(`{customer_shortname}-satz-interface`), `roles/pubsub.publisher` on the topic for Cloud
Storage's service agent, a storage notification for `OBJECT_FINALIZE` on the bucket, and
the object `interface.json`, whose content is the root module's `local.satz_interface` —
every exported value, the core ones under `core` and each interface's under
`interfaces."<name>"` — as JSON. Terraform rewrites the object only when its content
changes: an apply that changes an exported value publishes one message, an apply that
changes nothing publishes none.

It exports `interface_topic`, the topic's id, and `interface_object`, the object's
`gs://` URL, as core exports, so every interface module and its README carry them. A team subscribes in its
own state with a `google_pubsub_subscription` on the topic — a push to its CI's webhook,
or a pull from a runner — and reads the new values from the object the message names.

The service agent's address carries the project number, so the pack reads it with the
provider's `google_storage_project_service_account` data source in a trusted `hcl` block,
which the compile notes on every transpile.

## interface-lookups.yaml

Not a pack: how the interface modules under `hcl/interfaces/` read back what an estate emits,
per resource type — the data source, the keys it is looked up by, the read permission the
lookup needs, the attributes it yields, and the attributes that derive from what satz
writes (a service account's `email`, a bucket's `url`). The table is compiled into satz;
an export that needs a lookup of a type it has no row for is refused at compile, naming
the type ([language §6.17](../docs/language.md#617-export-and-interface--what-the-estate-publishes-to-the-hcl-beside-it)).

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
the API's Discovery Document and the provider schema. Its rows join the `map:` rows
of `import-config.yaml`, which win where both name a field, so a correction made in
the table survives a refresh. Overrides go into `import-config.yaml` (`api_schema:`
to pin an ambiguous schema name). The file is what carries a renamed BLOCK
(`lifecycle.rule[]` → `lifecycle_rule`); a nested value whose name is an attribute of
the resource the import flattens by itself, with or without the file.

## Breaking changes

What a satz release refuses that the release before it compiled, and the edit that
satisfies it. Newest first. Each entry says what is refused, how to find it in an
estate, what to write instead, and whether the plan moves; the error satz prints
names the file and the line.

### v0.83.0

**A `config.toml` that does not name `yaml_dir` reads the estate from `satz/`.** The
estate directory `satz init` creates, and the one an omitted `yaml_dir` means, is
`satz/`; `include_dirs` defaults to `[".", "satz"]` the same way. A `config.toml` that
names `yaml_dir` reads the directory it names, so an estate `init` wrote keeps working
unchanged. Find what is affected: a `config.toml` with no `yaml_dir =` line whose
estate sits in `yaml/` (`grep -L '^yaml_dir' config.toml`).

```
error: failed to read file 'satz/acme.satz': No such file or directory (os error 2)
```

**The edit:** either write `yaml_dir = "yaml"` (and `include_dirs = [".", "yaml"]`
if the file does not name it) into `config.toml`, or rename the directory with
`git mv yaml satz`. Nothing an estate compiles to changes; the plan does not move.

### v0.82.0

**`satz import <scope> --as <estate>` refuses an estate that impersonates no service
account, and sweeps nothing.** `--as` borrows the estate's IaC service account; a
local-mode estate (`deployment_mode = "local"`, or none bound) impersonates nobody,
and `--no-impersonate` keeps any estate off its account, so the sweep would read as
your own Application Default Credentials while naming the estate. `satz whoami
<estate>.satz` prints the mode an estate runs in.

```
error: --as yaml/acme.satz: the estate runs in local mode and impersonates no service account, so the sweep would read as your own Application Default Credentials while naming the estate. Drop --as to sweep as your own credentials. Nothing was swept.
```

**The edit:** drop `--as` — `satz import <scope> -o <file>` reads as your own
credentials, which is what the refused run would have done — or drop
`--no-impersonate`, or switch the estate to cloud mode (`satz migrate <estate>.satz
--mode cloud`). Nothing an estate compiles to changes.

**`satz import <scope> --into <estate>` and `--as <estate>` refuse a scope outside
the estate's organisation, and sweep nothing.** The scope must be
`organizations/<customer_organization_id>` or a folder or project inside it; a
folder or project is walked up through Resource Manager as the estate's identity,
and a walk that cannot be read is refused too. An estate that binds no
`customer_organization_id` is refused, because there is nothing to compare.

```
error: import: the scope is organizations/222222222222, and yaml/acme.satz is bound to organizations/123456789012 — a sweep of another organisation would write its resources into this estate. Sweep a scope inside organizations/123456789012, or name the estate bound to organizations/222222222222. Nothing was swept.
```

**The edit:** name the estate that belongs to the organisation you sweep, or sweep
without `--into`/`--as` into a new file. An estate without `customer_organization_id`
binds it in its `params`: `grep -n customer_organization_id <estate>.satz` shows
whether it does. Nothing an estate compiles to changes.

**`satz import <dir or .tf>` refuses to write an estate without an organisation.**
The hcl shape binds the estate to the organisation its configuration names (a
literal `organizations/<n>` parent, an `org_id`, an org policy's parent). A
configuration that names none — and every `--wrap-all` import, which translates
nothing — is refused until `--organization <n>` names it; a flag that contradicts
the configuration, and a configuration that names two organisations, are refused.
Where the hcl shape used to write a header line asking for
`customer_organization_id` by hand, it now writes nothing.

```
error: import: no organization id — --wrap-all translates nothing, so nothing in the configuration names one, and every estate is bound to one: a folder's parent and an organization grant's `org_id` are written from `customer_organization_id`. Nothing written. Name it with `--organization <n>`.
```

**The edit:** add `--organization <n>` to the command; import two organisations'
files in two runs. An estate an earlier import wrote is not re-read.

**`satz import <scope> --generate-unmapped` refuses when `<base>-generated.satz` is
already there, and sweeps nothing.** That file is the provider's output an earlier
run wrote for the operator to merge; a second run replaced it. `ls yaml/*-generated.satz`
lists them.

```
error: yaml/discovered-generated.satz exists — an earlier --generate-unmapped run wrote it, and this run would replace it. Merge what you keep of it into the estate, delete it, and run again. Nothing was swept.
```

**The edit:** merge what you keep into the estate, delete the file, run again.

**A live sweep in which Cloud Asset Inventory refused every asset type it asked for
ends with nothing written.** That is the scope ListAssets cannot read — a
nonexistent or misspelt `folders/<n>` or `projects/<id>` — however few types were
asked; a one-type sweep (`--only`) used to report the type as unserved and write an
empty estate. **The edit:** correct the scope. A type refused while others of the
same sweep are served is still left out and named at the end of the run.

### v0.81.0

**A `.py` action runs through `uv`, and is refused when `uv` is not on PATH.** An
action's `run` is launched by its extension: a `.py` file is spawned as
`uv run --script <file> <args>` instead of being executed directly under its own
shebang, so one file runs on every platform satz ships for. `satz run-actions
<estate>.satz` prints the resolved command line, which now begins `uv run --script`
for such an action; `grep -rn --include='*.satz' 'run *= *".*\.py"' .` finds every
Python action an estate declares. Nothing emitted changes; the plan does not move.

```
error: action "seed-settings" (yaml/main.satz:41): scripts/seed-settings.py is a Python action, and satz runs one with `uv`, which is not on PATH.
      Install uv — `brew install uv`, `pipx install uv`, or the installer uv's own documentation names — and run this again.
      satz does not fall back to `python` or `python3`: that is a different interpreter with different packages.
```

**The edit:** install uv — `brew install uv`, `pipx install uv`, or the installer uv's
own documentation names — on every machine that runs actions, and in CI. satz does not
fall back to a `python` or `python3` on PATH: that is a different interpreter with
different packages. A `.py` action no longer needs its executable bit, and a `.sh`
action is unchanged.

### v0.80.0

**`satz import <state>` refuses a state that names no organisation, and writes nothing.**
Every estate is bound to one: a folder's parent is
`organizations/{customer_organization_id}` and an organisation grant's `org_id` is that
param. A state carries the number only where a resource names it
(`organizations/<n>`, `org_id`) or a top-level folder hangs under the organisation. A
state of folders and projects nested under a folder outside it names none — that import
used to warn and write the estate anyway, with `parent = "organizations/"` on the folder
and `org_id = ""` on the grant, which no apply can use. It now refuses:

```
error: import: no organization id — nothing among the discovered resources names one (an `organizations/<n>` reference or an `org_id`), and a folder's parent and an organization grant's `org_id` are written from `customer_organization_id`. Nothing written. Name it with `--organization <n>` (state shape), or sweep `organizations/<n>` (live shape).
```

**The edit:** name the organisation on the command line —
`satz import state.json --organization 123456789012`. Where the state names one of its
own and the flag says another, the import is refused naming both, and one of the two is
corrected. A live sweep reads the organisation from its own root and from the assets'
ancestors, so it takes no flag.

**A grant in a state that names no scope is refused.** A `*_iam_member` is imported by
`<scope> <role> <member>`, and a resource whose `org_id`, `billing_account_id`, `folder`,
`project` or `bucket` is missing or empty used to be written with an import id beginning
in a space, which imports nothing. The refusal names the resource and the attributes that
carry the scope. **The edit:** none in the estate — the state is wrong, and the resource
is applied or removed with the tool that wrote it before the import is run again.

The import writes nothing, so no plan moves.

**An estate written by an earlier import may carry two values that were wrong.** Nothing
rereads it, so it is read once by hand:

- `google_iam_workload_identity_pool_provider.workload_identity_pool_id` held the
  PROVIDER's id instead of the pool's, from a live import. Re-applied, that provider
  points at a pool that does not exist. **The edit:** set it to the pool's id — the
  provider's own `"import-id"` carries both, as
  `projects/<p>/locations/global/workloadIdentityPools/<pool>/providers/<provider>`.
- A Pub/Sub subscription set to never expire lost its `expiration_policy`, from a state
  and from a live import alike, which restores Google's 31-day default — after which the
  subscription deletes itself. **The edit:** write `expiration_policy { ttl = "" }` into
  the subscription's body.

A provider carrying the provider's id as its pool id plans a replacement, and the
replacement fails because no pool has that id; with the pool's id the plan is empty. A
subscription the import adopted keeps its live `expiration_policy` while the estate
writes none, so writing it back plans no change.

### v0.79.0

**A project's provider alias works in the estate's region and bills to its own
project.** Every `google_project { … }` an estate declares gets a provider alias
(`provider "google" { alias = project_<label> }`) and every resource written inside
that project's body is served by it. Two of its attributes change.

`region` is the region the estate's own `google` provider block names — through
`default_region` in the scaffold — where it was the literal `"europe-west3"` for every
estate. **The plan moves for an estate that works in another region and holds a
regional resource inside a `google_project { … }` that writes no `region` of its own:**
that resource was created in `europe-west3` and is now replaced in the estate's region.
Transpile with the previous and the current binary and compare `providers.tf`: when an
alias's `region` differs, every regional resource inside that `google_project { … }`
that writes no `region` of its own moves, and `tofu plan` lists each one as a
replacement.

**The edit:** none, if `europe-west3` is where the resource belongs — write `region =
"europe-west3"` into that resource's body in the estate and the plan is empty again.
An estate whose `providers { "google" { … } }` block names no `region` at all now
emits aliases without one, and a regional resource inside a project that writes none
is refused by the provider naming the attribute: bind `default_region` and write
`region = default_region` in the provider block, as `satz init` does.

`billing_project` is the alias's own project, where it was the infrastructure project.
Google tests API enablement and quota on the project sent as the quota project, so a
resource inside a project node now needs its API enabled on THAT project, not on the
infrastructure project. **The edit:** for each project whose body holds resources, make
sure its `project_service = [ … ]` list carries the APIs they need —
`satz transpile <estate>` names an API no `google_project_service` enables. The
infrastructure project keeps its own list; nothing is removed from it, and the estate's
`google` and `google-beta` blocks are untouched.

**`satz import <dir>` refuses a `.tf` directory whose translated resource references a
block that stays verbatim.** satz emits no address for an `hcl trust` block — it is text,
and the emission manifest does not hold it — so an estate in which a Satz resource writes
`${google_storage_bucket.state.name}` for a bucket carried verbatim is one `satz transpile`
refuses with `written-reference`. The import wrote that estate and warned; it now refuses,
names both sides, and writes nothing:

```
error: the import would write an estate `satz transpile` refuses, so nothing was written: a translated resource references an address this estate does not emit.
  main.tf:24 `google_storage_bucket_iam_member.state_reader` references `google_storage_bucket.state`, which stays verbatim inside `hcl trust` (main.tf:17 — uses `for_each`)
Either make the referenced block translatable (its reason is above), import the file that declares it too, or carry everything verbatim with `--wrap-all`.
```

**The edit:** three ways out, in the order they are worth trying.

1. Make the named block translatable — the refusal carries its reason (`uses
   `for_each``, `label … is not an identifier`, …). Edit the `.tf` and import again.
2. Import the file that declares the other side too, where the reference points outside
   the directory being imported: `satz import <dir>` reads every `.tf` in one directory.
3. `satz import <dir> --wrap-all`, which carries every block verbatim, translates nothing
   and never crosses the boundary. The estate deploys as written; the compliance plane
   does not see into it.

An estate written by an earlier import is unaffected: nothing rereads it, and what it
already holds still transpiles or already did not.

### v0.77.0

**The S1 security group model ships in one spelling, and the two files of the other one
are gone from the library.** The library no longer carries
`presets/security-group-models/s1-group-definitions.satz` (the five admin groups) or
`presets/security-group-models/s1-group-permissions.satz` (their organization-level role
grants). Neither `get-presets` nor `merge-presets` deletes a file the library dropped, so
an estate whose `presets_dir` still holds the two keeps compiling them, unchanged and
without a finding. `satz check-presets` is the command that names them:

```
  local-only [included]: security-group-models/s1-group-definitions.satz (not an upstream preset — kept as-is)
  local-only [included]: security-group-models/s1-group-permissions.satz (not an upstream preset — kept as-is)
```

Where `presets_dir` does not hold them — a new checkout, a library installed after the
release — every command that reads the estate refuses it, naming the file and the line:

```
error    front-end  main.satz:18
    use "presets/security-group-models/s1-group-definitions.satz": file not found
```

Either way the estate is edited, because no release of the library updates the two
files again.

**The edit:** delete the two lines the estate holds today — one nested inside the
`google_cloud_identity_group` block, one inside `google_organization_iam_member`:

```satz
google_cloud_identity_group { use "presets/security-group-models/s1-group-definitions.satz" }
google_organization_iam_member { use "presets/security-group-models/s1-group-permissions.satz" }
```

and write one line in their place, at the top level of the estate file, where the other
pack lines stand:

```satz
use "presets/security-group-models/s1-security-groups.satz" when security_model_s1
```

(`when security_model_s1` for an estate that uses `presets/estate-map.satz`; without the
map, the bare `use` line.) A `google_cloud_identity_group { }` or
`google_organization_iam_member { }` block left empty by the deletion is deleted with it;
a block that also holds the estate's own groups or grants stays as it is. Then delete
`s1-group-definitions.satz` and `s1-group-permissions.satz` from `presets_dir`, where
`satz check-presets` lists them as `local-only`.

The params do not change: `gcp_organization_admins_name`, `gcp_project_admins_name`,
`gcp_security_admins_name`, `gcp_security_viewers_name` and `gcp_billing_admins_name`
are the same five names with the same defaults, declared by the pack that remains, and
whatever the estate binds keeps applying. The five questions are the same five
questions.

**The plan does not move.** The emitted HCL is byte-identical: the same five
`google_cloud_identity_group` resources and the same six `google_organization_iam_member`
addresses, with the same bodies. `satz transpile` after the edit and `tofu plan` reports
no change.

### v0.76.0

**A param whose name begins with `contributes_` is a CONTRIBUTION, not a param.** The
name after the prefix is the list param whose entries the file adds to, so a pack that
happens to call a param `contributes_<something>` is now read as adding to
`<something>`. The compile refuses one in an estate ("a contribution belongs in a pack"),
one whose value is not a list, one in a file that declares the target itself, and
`contributes_` with nothing after it.

**The edit:** rename the param. Nothing in the preset library carried such a name, so
this reaches only an estate or a `.local` fork that chose one.

**An estate that uses both the CIS baseline and the Defender foundation plans one more
subject.** `presets/integrations/microsoft-defender-for-cloud.satz` now contributes
`serviceAccount:mdc-agentless-scanning@guardians-prod-diskscanning.iam.gserviceaccount.com`
to `allowed_policy_member_subjects`, which the pack's header used to ask an operator to
add by hand. The next plan therefore updates
`google_org_policy_policy.iam_managed_allowedPolicyMembers` with that entry.

**The edit:** none — that is the entry the grant to Defender's scanner needs. An estate
that already added it by hand to its own `allowed_policy_member_subjects` keeps it once:
a contribution is not added twice, and that estate's plan does not move. The hand-added
line may be deleted, and then the entry leaves with the pack.

### v0.75.0

**`satz review-pack` refuses a pack that holds a value shaped like private data.** A
directory id, an organisation, folder or project number, a billing account, a GUID, a project
id, an e-mail address or a domain that is not one of the documented example values
(`docs/examples.md`) is an error of kind `private-shape`, one per value at its line, and the
review no longer passes:

```
error    private-shape     central-logs.satz:7   123456789012
```

**The edit:** make each value a param the estate binds, or replace it with the documented
example value. A pack that stays private — a `.local.satz` in the estate's own library — does
not need to pass `review-pack`; the check is the bar for a pack that goes upstream.

### v0.74.0

**An answer that switches a pack on is refused while a pack it needs is off.** `satz
interview` and `satz_interview` switch a pack on when its question is answered yes; the
switch now refuses what `satz add-pack` refuses, and writes nothing:

```
use_central_alerts = yes: `presets/monitoring/organization-cis-log-alerts-central.satz` needs `presets/monitoring/organization-audit-logsink.satz` (`use_audit_logsink`), which is off (it reads `logsink_project_id`) — `satz add-pack` it first
```

**The edit:** switch the needed pack on first — answer its question yes, or `satz add-pack
<estate> <pack>` — then answer again; or `satz add-pack <estate> <pack> --with-requirements`
switches both.

**A yes to a pack whose commented line stands inside a folder's or a project's body is
refused, naming the move.** An estate written before v0.71.0 keeps its commented pack lines
inside `google_folder { … }`; uncommented there, the line is a `use` the compile refuses. The
answer and `satz add-pack` both say which line it is:

```
`presets/monitoring/organization-audit-logsink.satz`'s line (line 213) is commented inside `google_folder.infra_folder`, and a pack is used at the top level of the file — move the commented line there, then switch the pack on
```

**The edit:** move the commented line out of the folder's body to the top level of the file,
as it is, then answer or `add-pack` again. A pack that creates a project names its folder with
a param of its own (`logsink_project_folder` for the audit archive): bind it to the folder,
`logsink_project_folder = "google_folder.infra_folder.name"`, so the project stays where the line
stood.

**An answer that would leave an estate satz refuses writes nothing.** Before, the answer was
written and the refusal came after it, leaving on disk an estate `satz_open` and the compile
refuse; the file is now as it was, and the refusal says so.

### v0.73.0

**An estate that uses `presets/estate-core.satz` has one more question to answer, and
`apply` refuses until it is.** The pack declares `compliance_frameworks` — the catalogs
this customer is HELD TO, which is not the same fact as what the estate's packs claim.
A question is answered by the estate binding its param, so an estate that used the pack
and answered everything now has one open question:

```
apply refused: 1 question(s) unanswered — satz questions <estate> --unanswered
```

**The edit:** add the param to the estate's `params { }` with the catalog ids the
customer answers to, or run `satz interview <estate>` and answer it:

```satz
params {
  compliance_frameworks = ["cis-gcp-5.0"]
}
```

The values are the catalog ids in `<presets_dir>/catalogs/`: `cis-gcp-4.0`,
`cis-gcp-5.0`, `iso27001-2022`. A value that names no catalog is refused by the compile,
with the list. An estate that does not use `estate-core` is unaffected, and every
`satz report-compliance <framework> <estate>` invocation keeps working unchanged.

**`bootstrap` refuses an estate that binds no `infra_bucket_name`.** The state bucket had
two defaults: `presets/estate-core.satz` declares
`infra_bucket_name = "{customer_shortname}-infra-001-state"`, and `bootstrap` took the
infra project's id when the estate bound no bucket of its own. An estate that binds
`infra_project_name` and not `infra_bucket_name` therefore bootstrapped into a bucket
named after the project. It is now refused by name, before any credential is asked for:

```
the estate is not ready to bootstrap — 1 param(s) are missing or malformed, and nothing was called:
  infra_bucket_name — is not set
      set it with `satz init --infra-bucket-name`
```

**The edit:** bind `infra_bucket_name` in the estate's `params { }` — the value the state
bucket already has, so no state moves — or `use "presets/estate-core.satz"`, whose default
resolves it. `satz init --infra-bucket-name <name>` writes it into an estate you already
have. An estate that uses `estate-core`, and every estate `satz init` wrote, binds it
already and is unaffected.

### v0.72.0

**A key a resource type does not have is refused.** Every key of a resource body is
checked against the provider schema at parse time, block bodies included, and a key the
schema does not name stops the compile: ``google_project: unknown key `parent` — the
provider schema names no such argument or block here``, with the file and the line. It
used to be written into `main.tf` as an argument, where `tofu validate` was the first
thing to object.

**The edit, per key:** write the argument the provider has. A project's parent is
`folder_id` (a reference to a folder the estate declares, `google_folder.infra.name`, or
a numeric id) or `org_id` — never `parent`, which is the Resource Manager path and
belongs to an org policy. For any other type, the provider's registry page lists its
arguments under the name satz uses, to the underscore. Find what is affected before
upgrading: `satz transpile <estate>.satz` names one key per run.

**Nine keys are satz's own and stay**, in the body of every type that takes them:
`"import-id"`, `lifecycle`, `provider`, a project's `project_service` and `org`, and a
group's `member`, `manager`, `owner` and `email`. `depends_on` is not among them — satz
derives the ordering a plan needs itself.

**A `.yaml` estate or pack is refused, and satz no longer converts one.** The pre-Satz
YAML dialect — `variables:` with `&anchor` / `*alias`, `!include`, `!include-if`,
`!import-include`, `!format`, `!join`, `!expr`, resource keys written without the
`google_` prefix — is read by no command. `satz import <file>.yaml` used to convert it;
it now refuses it, as `transpile`, `adopt`, `migrate`, `run-actions` and `scan` already
did. The `--kind`, `--gate` and `--fork` flags and the `yaml` value of `--from` are gone
with it. Find what is affected: a `.yaml` file in your `yaml_dir`, or a `use "….yaml"`
line in an estate.

**The edit:** convert with the last release that reads the dialect, then come back to the
current binary. The refusal prints this sequence:

```bash
cargo install --git https://github.com/tjirsch/satz --tag v0.71.0 --locked
satz import old-estate.yaml --kind estate          # --kind pack for a pack
cargo install --git https://github.com/tjirsch/satz --locked
satz fmt old-estate.satz
satz merge-presets --estate old-estate.satz
```

Convert the packs an estate `use`s before the estate itself, then `satz transpile` and a
`tofu plan` that shows no destroy for what the estate already manages. `import-config.yaml`
and the catalogs under `presets/catalogs/` are data files, not estates — they are YAML
and stay YAML.

### v0.71.0

**A `use` inside a folder's or a project's body is refused.** A folder's and a project's
body hold the estate's own resources; a pack is used at the top level of the estate.
`google_folder { infra_folder { use "presets/…" } }` is refused with ``use "presets/…"`
stands in the body of `google_folder.infra_folder`, which holds the estate's own resources
— a pack is used at the top level of a file``, naming the estate file and the line. Find
every one: `grep -nE '^[[:space:]]+(// *)?use "' <estate>.satz` — an indented `use` line,
commented or not.

**The edit, per line:** move the line to the top level of the estate file — out of every
`{ … }`, at the left margin. Only the text moves: a line that was commented out stays
commented out, a line that was active stays active, and its `when <param>` stays with it.
Keep the order — a pack whose params another pack reads keeps its line above that pack's,
because the compile builds one parameter namespace in file order.

**Two packs need a param bound as well**, because they create a project and the folder the
line used to stand in was what put that project there:

- `presets/monitoring/organization-audit-logsink.satz` → `logsink_project_folder`
- `presets/integrations/microsoft-defender-for-cloud.satz` → `mdc_mgmt_project_folder`

In the estate's `params { … }`, bind the param to the folder the `use` line used to stand
in, as a reference: `logsink_project_folder = "google_folder.infra_folder.name"` for a line
that stood in `google_folder { infra_folder { … } }`. A folder that already exists rather
than being declared in the estate is named by its numeric id, `"123456789012"`. With the
param bound, the emitted HCL is byte-identical to what the nested line emitted — `satz
transpile` before and after the edit produces the same `main.tf`. Without it the project is
created under the organisation instead, which is a project move in the plan. Every other
pack emits the same resources wherever its line stands, so it needs the move and nothing
else.

An estate `satz init` wrote has exactly two nested lines, both in `google_folder {
infra_folder { … } }`: `presets/monitoring/organization-audit-logsink.satz` and
`presets/monitoring/organization-cis-log-alerts-central.satz`. The first takes
`logsink_project_folder = "google_folder.infra_folder.name"`; the second takes nothing.
A new estate `satz init` writes both lines at the top level and binds
`logsink_project_folder` itself.

**`offers … { after_scaffold = true }` is gone, and `block` names a resource type map.**
This is the pack graph, so it matters to a fork of `presets/estate-map.satz` and to
nothing else. Every pack line satz writes now stands at the top level, in the order of the
`offers` entries, bar a pack that is a bare list of labelled bodies, whose line is written
inside the map of its type — that is what `block = "google_essential_contacts_contact"`
says. A `block` naming a node of the estate (`block = "google_folder.infra_folder"`) is
refused; delete the key, and the line joins the menu in its entry's order. Delete
`after_scaffold = true` wherever it stands.

**An organisation-level resource type written inside a project's body is refused.** A
folder, a project, an organisation grant (`google_organization_iam_member`), a Cloud
Identity group (`google_cloud_identity_group`) and a billing grant
(`google_billing_account_iam_member`) all hang off something above the project — the
organisation, a folder, the Cloud Identity customer, the billing account — so standing in
a project's body did not place them: they reached the organisation while reading as "in
this project". Such a block is now refused with ``google_organization_iam_member { … }`
stands in the body of a `google_project`, and it belongs to the organisation — not to the
project``, naming the file and the line. **The edit:** move the block out of the project's
body, to the top level of the same file — the file that declares the project included, and
one file may declare a project together with the groups that go with it. Nothing moves in
the plan: the resource was already emitted at the organisation, and it still is. A folder's
body is unaffected, and so is a `use`: a used file's entries are read at its own top level,
which is what lets one file declare a project together with the groups that go with it.

**An org policy inside a project's body gets the parent the API takes.** It was emitted
with `parent = google_project.<label>.project_id`, the bare project id, which is no
Resource Manager path and no apply could take; it is now
`parent = "projects/${google_project.<label>.project_id}"`, with the policy's `name` built
from it. No pack of the library declares an org policy in a project's body, so nothing
here moves; an estate that does gets a plan that applies where it did not.

### v0.70.0

**`before = apply` on a `notice` is gone; a notice declares a `severity`.** A pack whose
notice carries `before = apply` is refused with ``notice <param>: `before = apply` is
gone — write `severity = error` ``. Find every one, in a fork of a pack as well as in a
pack written here: `grep -rn 'before = apply' presets/`. In each notice block, replace
that line with `severity = error`, then `satz fmt presets` to realign the block. The
three words are `error` — every command that writes to the organisation refuses while
the notice is open — `warning`, which prints and goes on, and `info`; a notice that
declares none is a `warning`. A pack of the library carries the severity already: the
CIS packs' notices are `severity = error`, which is what `before = apply` did.

**The finding severity `note` is now `info`.** `satz transpile --format json`,
`satz_transpile_check` and every other reader return `"severity": "info"` where they
returned `"note"`, the last line of a run counts `1 info` instead of `1 note`, and the
first line of such a finding starts with `info`. A script or a pipeline that matches the
word changes with it: `grep '"severity": "note"'` becomes `grep '"severity": "info"'`.
The three words are now the three a pack declares.

### v0.69.0

**`get-presets` and `merge-presets` rewrite no estate for a breaking change.** Up to
v0.68 they repointed a `use` of a CIS pack at its old path, moved the forks beside it,
lifted the CIS baseline out of its block, and wrote ` when <gate>` on a pack line that
lacked it. They do none of that now. The compile reports both forms, naming the file and
the line, and the two entries below are the edits. The `migrated` field of
`get-presets`' answer (`satz_get_presets`) is gone with it. What `merge-presets` does for
a pack whose upstream CHANGED is untouched: it forks, repoints, proves and adopts as
before.

**A `use` of a CIS pack at its old path** — `presets/cis-extensions/<pack>.satz` or
`presets/CIS-GCP-Foundation-4.0.satz` — is refused with `this pack moved to
"presets/cis/…"`. The CIS packs live in `presets/cis/`, the baseline beside its
extensions. Find every line, commented ones included:
`grep -rn 'presets/cis-extensions/\|presets/CIS-GCP-Foundation' yaml/`.

1. Run `satz get-presets`. It installs the packs at `presets/cis/`, and it reads the
   estate without compiling it, so it runs while the old lines are still there.
2. Move the estate's own files. Each `<pack>.local.satz` and `<pack>.diff.satz` in
   `presets/cis-extensions/`, and `presets/CIS-GCP-Foundation-4.0.local.satz` with its
   `.diff.satz`, moves to `presets/cis/` as it is (`git mv`).
3. Delete the old pristine copies: every other file in `presets/cis-extensions/`, the
   directory, and `presets/CIS-GCP-Foundation-4.0.satz`. The old baseline is deleted, not
   moved: the baseline at the new path is a different shape — it declares its own
   `google_org_policy_policy { … }`.
4. Repoint the lines. In each line the `grep` found, change the text inside the quotes
   and nothing else: `presets/cis-extensions/<x>` becomes `presets/cis/<x>`,
   `presets/CIS-GCP-Foundation-4.0<…>` becomes `presets/cis/CIS-GCP-Foundation-4.0<…>`.
   The indentation, an `as`, a `when` and the `//` of a commented line stay. Never change
   whether a line is commented: an active baseline that comes back commented takes thirty
   organisation policies off the organisation at the next apply, and a commented pack
   made active deploys it.
5. Place the baseline's line. The line of the PRISTINE baseline
   (`use "presets/cis/CIS-GCP-Foundation-4.0.satz"`) moves out of the
   `google_org_policy_policy { … }` block it stood in to the top level of the estate; a
   block left with nothing in it is deleted, one that holds the estate's own policies
   stays. The line of a FORK made at the old path (`…-4.0.local.satz`) stays inside the
   block: that fork is still a bare list of labels, and the block is what gives them
   their type.
6. Check. `satz transpile <estate>.satz`, then `git diff` over the generated HCL. A pack
   whose deleted copy was the version `get-presets` installed emits what it emitted
   before. A copy that was behind upstream shows upstream's change in that diff; to keep
   what the estate deployed instead, restore the deleted copy as
   `presets/cis/<pack>.local.satz` and point the line at it. Then `satz merge-presets`,
   and `tofu plan`.

A baseline line at the top level without ` when use_cis_baseline` is the next entry.

**An active line of a gated pack written without `when`** is reported by the compile as
an `ungated-pack` finding, `satz packs` lists the line as `ungated`, and `satz
remove-pack` refuses to switch the pack off through it. The line deploys the pack
whatever its gate says, so a no does not switch it off.

- On the line the finding names, write the line it prints:
  `use "<path>" when <gate>`, after an `as <type>` where the line has one, before a
  trailing comment.
- Bind `<gate> = true` in the estate's own `params { }` — also where the library's
  default is already `true`, because a `when` is checked where the compile meets the
  line and the file declaring the gate may be used below it. The line deployed the pack;
  left unbound or `false`, the gate now switches it off and the next apply destroys
  what it deployed. If the no was meant, `satz remove-pack <estate> <gate>` switches the
  pack off afterwards.
- A gate whose default follows the one bound (`use_sentinel_auditlogs = use_sentinel`)
  and that the estate leaves unbound: bind it to the value it had before the edit, so
  nothing else switches on.
- The other option of a choice bound `true` here (`security_model_s1` beside
  `security_model_s2`), where the estate leaves it to a default of `true`: bind it
  `false`.
- A commented line is left as it is.
- Check: `satz transpile <estate>.satz`; `main.tf`, `imports.tf` and `variables.tf` are
  unchanged, `terraform.tfvars` changes in the gates bound, and `tofu plan` reads no
  changes.

**A pack header takes a name and a version — the word `content` is gone.**
`pack essential_contacts_organization version "1.3" content` is refused with
`` pack header: `content` is not a header word ``. The word marked one shipped pack
and changed nothing that satz emits.

- The one shipped pack that said it is `presets/essential-contacts-organization.satz`;
  version 1.4 does not. An estate that holds the 1.3 copy does not compile, and
  `satz merge-presets` stops on the same line, because it reads the estate's copy before
  it replaces it. Delete the word `content` from the `pack` line of the estate's copy
  (line 15), then run `satz merge-presets`: it upgrades the copy to 1.4 in place, as a
  change of comments and version only.
- A fork (`presets/essential-contacts-organization.local.satz`) or a pack of your own
  that says `content`: delete the word from its `pack` line. Nothing else changes, and
  the emitted HCL is the same.
- Find them: `grep -rn '^pack .* content' presets/ yaml/`.

**A statement is written at the top level of a file.** `params`, `question`, `claim`,
`notice`, `action`, `offers`, `suppress`, `hcl`, `estate` and `pack` directly inside a
block are refused with `` `params` is a Satz statement ``. satz used to read such a
block as whatever the position takes: `google_x { params { … } }` declared a resource
labelled `params`, `google_folder { params { … } }` a folder called `params`, and
`params { … }` in a folder's or a project's body an attribute `params = { … }` on the
folder or project, which the provider rejects at plan time.

- Move the block to the top level of the file it is in.
- A resource, folder or project that really is called `params` is written with a
  quoted key: `"params" { … }`. The emitted address does not change.
- Nested blocks of a resource's own body are untouched: `action { type = "Delete" }`
  inside a `lifecycle_rule` is the provider's block.

**A resource type map is not written inside a map of names.**
`google_x { google_y { … } }`, `google_folder { google_x { … } }` and
`google_project { google_x { … } }` are refused with `opens a map of its own`. satz
used to read the inner key as a name: a resource `google_x.google_y`, or a folder or
project called `google_x`.

- Inside a folder or a project, the map goes into the BODY of a named folder or
  project: `google_folder { shared { google_x { … } } }`.
- Otherwise it goes beside the outer map, not inside it.

**A pack that declares its own resource types is not used inside `google_folder { … }`.**
`google_folder { use "presets/cis/cmek.satz" }` is refused with `does not belong
there`; satz used to compile it into a FOLDER named after each of the pack's resource
types — `google_folder.google_org_policy_policy` — and to emit none of the pack's
resources. The same pack inside a resource type map (`google_x { use … }`,
`use … as google_x`) was refused before and still is.

- A pack is used at the top level: `use "presets/<pack>.satz"`. Its resources reach the
  organisation, and a pack that creates a project names the folder that project is created
  in with a param of its own.
- `google_folder { use "<file>" }` stays valid for a file whose entries are named
  folders.

**A file of statements alone is used at the top level.** `presets/estate-core.satz` and
`presets/estate-map.satz` hold `params` and `question`s and no resource; inside a
resource type map or `google_folder { … }` they are refused with `that file holds no
entry`. Write `use "presets/estate-core.satz"` at the top level of the estate. The
params and questions of a file reach the estate from every position, so nothing is lost
by moving the line.

**A used file carries no `suppress`.** A `suppress` in a pack or any other `use`d file
is refused with `` is a `suppress`, which is read from the estate alone ``. satz never
applied such a line — the resource it names was emitted all along. Move the line into
the estate's own file, where it takes effect and removes the resource from the plan; or
delete it to keep what is deployed today.

## Changelog

One row per pack version. The in-file `pack <name> version "<n>"` line is the
source of truth; the smoke matrix fails when a pack's current version has no
row here, so a bump and its reason ship together. Newest first within a pack.
Dates before 2026-08-28 predate the public repository and are given to the day
the private history recorded them.

| pack | version | date | change |
|---|---|---|---|
| `interface_notice` | 1.0 | 2026-09-25 | first version: tells the teams whose HCL reads the estate's interface when an exported value changes. A bucket, a Pub/Sub topic, the grant that lets Cloud Storage's service agent publish to it, and a storage notification in the infrastructure project; the object `interface.json` holds the exported values, rewritten only when one changes, so each apply that changes an export publishes one message. Exports `interface_topic` and `interface_object` |
| `estate_map` | 2.3 | 2026-09-25 | offers `interface-notice` on `use_interface_notice`, off by default, with the question that asks for it |
| `estate_core` | 2.2 | 2026-09-25 | the core exports: `organization_id`, `customer_domain`, `customer_shortname`, `default_region`, `infra_project_id` and `iac_service_account`, each a core export — an output of the root module and of every module under `hcl/interfaces/` — all known at compile time. An estate that uses the pack gains `outputs.tf` and `hcl/interfaces/`; its resources do not change |
| `estate_map` | 2.2 | 2026-09-22 | the S1 model is offered as one entry, `security-group-models/s1-security-groups.satz`, at the top level: the two `by_hand` entries for `s1-group-definitions.satz` and `s1-group-permissions.satz` are gone with the packs, and the billing grants require one of the two models rather than one of three files |
| `billing_export` | 1.0 | 2026-09-22 | Cloud Billing usage and cost data exported to BigQuery: a project of its own, the BigQuery API on it, the dataset, and Google's export account's dataEditor on it — with that account contributed to `allowed_policy_member_subjects`, and a notice for the console step Cloud Billing has no API for |
| `estate_map` | 2.1 | 2026-09-22 | offers `billing-export` on `use_billing_export`, off by default |
| `integrations.microsoft_defender_for_cloud` | 0.5 | 2026-09-22 | contributes the agentless disk-scanning account to `allowed_policy_member_subjects` instead of naming it as a manual prerequisite in the header |
| `estate_core` | 2.1 | 2026-09-21 | `compliance_frameworks`, the catalogs this customer is HELD TO — a contract, an auditor, a regulator — as a list of catalog ids, with the question that asks for them. What an estate CLAIMS comes from its packs and is a different fact: an estate can claim CIS controls while its customer is audited against ISO 27001. The default is `["cis-gcp-5.0"]`; the values are the ids of the catalogs in `presets/catalogs/` (`cis-gcp-4.0`, `cis-gcp-5.0`, `iso27001-2022`) and a value that names no catalog is refused by the compile, with the list. `satz report-compliance <estate>` reports one section per framework named here, `satz prowler` scans for them beside the frameworks the packs claim, and a pack reads the param like any other. An estate that binds nothing keeps working: `report-compliance <framework> <estate>` is unchanged |
| `monitoring.organization_audit_logsink` | 1.6 | 2026-09-21 | a question for `logsink_project_folder`: the folder the audit-archive project is created in. The param is now the only thing that decides — a `use` line no longer stands in a folder's body — so the interview asks for it. Answering it empty creates the project under the organisation; an estate `satz init` wrote answers `"google_folder.infra_folder.name"`, which init binds itself. Nothing emitted changes for an estate that already binds the param |
| `integrations.microsoft_defender_for_cloud` | 0.4 | 2026-09-21 | a question for `mdc_mgmt_project_folder`: the folder the Defender management project is created in. The param is now the only thing that decides — a `use` line no longer stands in a folder's body — so the interview asks for it. Answering it empty creates the project under the organisation, which is where every estate that binds nothing has it today |
| `monitoring.organization_audit_logsink` | 1.5 | 2026-09-21 | `logsink_project_folder`: the folder the audit-archive project is created in, said by the estate instead of read from the node the `use` line stands in. The default is empty, which says nothing — the enclosing node decides, exactly as before — so no estate's plan moves. An estate whose `use "presets/monitoring/organization-audit-logsink.satz"` stands inside a folder's body writes `logsink_project_folder = "google_folder.<label>.name"` for that folder (`satz init` writes the line into `google_folder.infra_folder`, so `"google_folder.infra_folder.name"`) and may then move the `use` line to the top level: the emitted HCL is byte-identical either way. A folder that already exists rather than being declared here is named by its id, `"123456789012"` |
| `integrations.microsoft_defender_for_cloud` | 0.3 | 2026-09-21 | `mdc_mgmt_project_folder`: the folder the Defender management project is created in, said by the estate instead of read from the node the `use` line stands in. The default is empty, which says nothing — the enclosing node decides, exactly as before — so no estate's plan moves. An estate whose `use "presets/integrations/microsoft-defender-for-cloud.satz"` stands inside a folder's body writes `mdc_mgmt_project_folder = "google_folder.<label>.name"` for that folder and may then move the `use` line to the top level: the emitted HCL is byte-identical either way. A folder that already exists rather than being declared here is named by its id, `"123456789012"` |
| `CIS_GCP_Foundation_4_0` | 2.17 | 2026-09-22 | the `cis_access_approval` question states Access Transparency (CIS 4.0 §2.14 / 5.0 §2.15) as the manual prerequisite it is: an organisation administrator with `roles/axt.admin` switches it on in the Cloud console before the apply, there is no gcloud command, API or provider resource for it, and it needs a Standard, Enhanced or Premium support plan. The catalogs now carry that control as organizational, so `report-compliance` lists it. Nothing emitted changes |
| `CIS_GCP_Foundation_4_0` | 2.16 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.block_project_ssh_keys` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.shielded_vm` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.dns_logging` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.confidential_computing` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cloud_sql` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cmek` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.api_key_services` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.bucket_retention` | 1.4 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cloud_sql_iam_and_deletion_protection` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.block_project_ssh_keys_dry_run` | 1.2 | 2026-09-20 | GENERATED from `block-project-ssh-keys.satz` 1.2: the same notice, now `severity = error`, on its own param |
| `cis_extensions.confidential_computing_dry_run` | 1.2 | 2026-09-20 | GENERATED from `confidential-computing.satz` 1.2: the same notice, now `severity = error`, on its own param |
| `cis_extensions.cloud_sql_dry_run` | 1.3 | 2026-09-20 | GENERATED from `cloud-sql.satz` 1.3: the same notice, now `severity = error`, on its own param |
| `cis_extensions.api_key_services_dry_run` | 1.3 | 2026-09-20 | GENERATED from `api-key-services.satz` 1.3: the same notice, now `severity = error`, on its own param |
| `cis_extensions.bucket_retention_dry_run` | 1.4 | 2026-09-20 | GENERATED from `bucket-retention.satz` 1.4: the same notice, now `severity = error`, on its own param |
| `cis_extensions.cloud_sql_iam_and_deletion_protection_dry_run` | 1.2 | 2026-09-20 | GENERATED from `cloud-sql-iam-and-deletion-protection.satz` 1.2: the same notice, now `severity = error`, on its own param |
| `essential_contacts_organization` | 1.4 | 2026-09-20 | the header loses the word `content`, which the language no longer has: a pack header is a name and a version. Nothing emitted changes. A copy of 1.3 is refused by its header line — see Breaking changes |
| `CIS_GCP_Foundation_4_0` | 2.15 | 2026-09-19 | a `notice`: once the baseline is switched on, `satz adopt <estate> --execute --import` is to run before the apply — Google sets some of these policies on every new organisation, and the first apply of the 2026-09-17 onboarding stopped on `409 POLICY_ALREADY_EXISTS` for `compute.managed.restrictProtocolForwardingCreationForTypes`. The estate acknowledges it with `cis_baseline_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.block_project_ssh_keys` | 1.1 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_block_project_ssh_keys_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.shielded_vm` | 1.1 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_require_shielded_vm_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.dns_logging` | 1.1 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_dns_logging_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.confidential_computing` | 1.1 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_confidential_computing_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.cloud_sql` | 1.2 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_cloud_sql_hardening_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.cmek` | 1.2 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_cmek_required_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.api_key_services` | 1.2 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_api_key_services_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.bucket_retention` | 1.3 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_bucket_retention_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.cloud_sql_iam_and_deletion_protection` | 1.1 | 2026-09-19 | a `notice`: once the pack is switched on, `satz adopt <estate> --execute --import` is to run before the apply, because a policy it declares may already be live and creating it stops on `409 POLICY_ALREADY_EXISTS`. The estate acknowledges it with `cis_cloud_sql_iam_and_deletion_protection_adopted = true`, which `adopt --execute --import` binds itself when it has run; `transpile --apply` and `bootstrap` refuse while it is open. The param is never emitted, so the plan does not move |
| `cis_extensions.block_project_ssh_keys_dry_run` | 1.1 | 2026-09-19 | GENERATED from `block-project-ssh-keys.satz` 1.1: the same notice, on its own param `cis_block_project_ssh_keys_dry_run_adopted`, because the twin is switched on on its own |
| `cis_extensions.confidential_computing_dry_run` | 1.1 | 2026-09-19 | GENERATED from `confidential-computing.satz` 1.1: the same notice, on its own param `cis_confidential_computing_dry_run_adopted`, because the twin is switched on on its own |
| `cis_extensions.cloud_sql_dry_run` | 1.2 | 2026-09-19 | GENERATED from `cloud-sql.satz` 1.2: the same notice, on its own param `cis_cloud_sql_hardening_dry_run_adopted`, because the twin is switched on on its own |
| `cis_extensions.api_key_services_dry_run` | 1.2 | 2026-09-19 | GENERATED from `api-key-services.satz` 1.2: the same notice, on its own param `cis_api_key_services_dry_run_adopted`, because the twin is switched on on its own |
| `cis_extensions.bucket_retention_dry_run` | 1.3 | 2026-09-19 | GENERATED from `bucket-retention.satz` 1.3: the same notice, on its own param `cis_bucket_retention_dry_run_adopted`, because the twin is switched on on its own |
| `cis_extensions.cloud_sql_iam_and_deletion_protection_dry_run` | 1.1 | 2026-09-19 | GENERATED from `cloud-sql-iam-and-deletion-protection.satz` 1.1: the same notice, on its own param `cis_cloud_sql_iam_and_deletion_protection_dry_run_adopted`, because the twin is switched on on its own |
| `exemptions.exemption_tag` | 2.0 | 2026-09-13 | one value per exemption CLASS instead of a single `not_enforced`: `service-account-keys`, `public-endpoint`, `public-storage`, `vm-image`, `vm-access`, `data-residency`, `encryption`, `network-appliance`. IAM is set on a tag VALUE, so one blanket value meant anyone allowed to exempt anything could exempt everything — the team needing a public bucket could switch off customer-managed encryption just as easily. The classes are deliberately narrow: a wide class is a grant that hands over more than the person asking described. Audit logging, flow logs, DNS logging and domain-restricted sharing carry NO class on purpose — exempting the record of what happened, or letting an outside identity in, is a decision for whoever owns the baseline, not a delegation. The `enforced` value is GONE: its only job was leaving a trace instead of deleting a binding, which the estate's own history already does, and it had no meaning once values became classes |
| `CIS_GCP_Foundation_4_0` | 2.14 | 2026-09-17 | the pack declares its own `google_org_policy_policy { … }` and is `use`d bare at the top level, gated on `use_cis_baseline`, exactly like every CIS extension. Nothing emitted changes — both `use` forms resolve to the same addresses and the same manifest — so an estate's plan does not move. What changes is that the baseline is a pack like the others: the interview can switch it on, the compile reports it when its answer is true and its line is not in, and satz-studio lists it. An estate that keeps the old `google_org_policy_policy { use … }` wrapper is refused, because as a map's content the pack's type key would be read as a label and the whole baseline would collapse into one resource |
| `CIS_GCP_Foundation_4_0` | 2.13 | 2026-09-13 | claims CIS 5.0 §2.14, Cloud Asset Inventory enabled — the last technical control of CIS 5.0 with no claim anywhere in the library. The estate already satisfied it: the scaffold enables `cloudasset.googleapis.com` in every infrastructure project, so the witness is the scaffold's own `google_project_service.infra_cloudasset_googleapis_com` rather than a second `google_project_service` declared here — two resources enabling one API on one project is a duplicate, not a merge. The address depends on the `infra` project label, which is already a contract (`bootstrap` imports by it) and is now held by the init-template test, so renaming it breaks a test rather than a customer's report |
| `CIS_GCP_Foundation_4_0` | 2.12 | 2026-09-13 | `iam.managed.disableServiceAccountKeyCreation` takes its rules from `cis_sa_key_creation_rules` instead of writing them in place, so an estate can let ONE service account out with a tag condition without forking the pack. The default is the plain enforcing rule and the emitted policy is unchanged for an estate that says nothing. The one constraint here with a rules param, because it is the one organisations actually have to exempt — Google ships their own built-in exemption tag for it — and because a param per constraint would put forty list-of-object blocks into every estate's `terraform.tfvars` for a case nobody has |
| `exemptions.exemption_tag` | 1.0 | 2026-09-13 | first version: the VOCABULARY for a tag-conditional exemption — one organisation tag key `<shortname>-exemption` with the values `enforced` and `not_enforced`, and nothing bound to either. An organisation policy is all-or-nothing per node, so letting one service account out of a control means lowering the policy for a whole folder and raising it again — a window during which nothing is enforced. A Resource Manager tag is IAM-governed and a policy rule can condition on it, which is how Google ships `iam.disableServiceAccountKeyCreation` themselves. The pack ships the ABILITY and no exemptions: a library that ships convenient exemptions lowers the baseline by default. The binding that exempts a resource and the condition on the constraint that honours it are the estate's, and the pack header shows both |
| `estate_map` | 2.0 | 2026-09-19 | one `offers` entry per pack in the library — its gate, the phase that has to be finished before it can go in, the block its line belongs in, and its adoption order — from which `satz pack-graph` writes `presets/pack-graph.json`. Every library file is offered: the S1 model's second spelling and Defender's plan fragments with `by_hand`, because their lines are written by hand. The edges the packs do not show are declared on the entries: the billing grants require a security model, the S1 split packs and every dry-run twin exclude what they replace. Two new choices: `use_verification_runner_grant`, following `use_verification_runner` by reference, because in the MSP-hosted shape the runner and its grant live in different estates; and `use_project_cis_log_alerts`, off, for a project that alerts on its own beside the central alerts. The grant's line in a newly written estate is gated on its own choice; an estate that carries it gated on the runner's keeps compiling and planning as before |
| `estate_map` | 1.9 | 2026-09-18 | the header says what order the estate's lines are in: the order the packs can be adopted, each written commented under the phase that has to be finished first, not the map's own order. A comment change: no choice, default or question changes, and an estate that uses the map upgrades without a fork |
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
| `s1_security_groups` | 1.0 | 2026-09-02 | the S1 model in ONE typed file (groups + org grants) for a top-level `use` |
| `s2_security_groups` | 1.0 | 2026-09-02 | S2 = S1 plus a distinct `gcp-network-admins` group (`compute.networkAdmin`, `compute.xpnAdmin`, `compute.securityAdmin`, `dns.admin`, `networkconnectivity.hubAdmin`, `networkmanagement.admin` + viewer roles); project-admins lose `compute.networkAdmin` and `compute.xpnAdmin`; one typed file |
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
