# Preset packs — changelog

One row per pack version. The in-file `pack <name> version "<n>"` line is the
source of truth; the smoke matrix fails when a pack's current version has no
row here, so a bump and its reason ship together. Newest first within a pack.
Dates before 2026-08-28 predate the public repository and are given to the day
the private history recorded them.

| pack | version | date | change |
|---|---|---|---|
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
| `s1_group_permissions` | 1.0 | 2026-08-20 | org-level role grants for the S1 groups; `roles/viewer` for the security-viewers group is the fleet standard |
| `essential_contacts_organization` | 1.1 | 2026-08-23 | `essential_contacts_email` param — a customer pins its contact without a fork; content pack |
| `essential_contacts_organization` | 1.0 | 2026-08-20 | organization-wide essential contact, all categories |
| `monitoring.organization_audit_logsink` | 1.1 | 2026-08-21 | claims for CIS 2.1/2.2 (both 4.0 and 5.0), the writer-identity bucket grant, retention lifecycle rules |
| `monitoring.organization_audit_logsink` | 1.0 | 2026-08-21 | org audit log sink → bucket |
| `monitoring.organization_cis_log_alerts_central` | 1.2 | 2026-08-24 | the log metric + alert stack for CIS 2.5–2.12 in one central logging project |
| `monitoring.organization_cis_log_alerts_central` | 1.1 | 2026-08-22 | notification channel param; alert policy display names carry the control id |
| `monitoring.organization_cis_log_alerts_central` | 1.0 | 2026-08-21 | first version |
| `project_cis_log_alerts` | 1.0 | 2026-08-21 | per-project variant of the CIS 2.5–2.12 metrics + alerts |
| `sa_security_audit` | 1.0 | 2026-08-21 | read-only security-audit service account with its custom role |
| `billing_account_permissions` | 1.0 | 2026-08-20 | billing-account IAM for the S1 groups and the IaC service account |
| `organization_budget` | 1.0 | 2026-08-20 | organization budget with threshold alerts (`"import-id"` example) |
