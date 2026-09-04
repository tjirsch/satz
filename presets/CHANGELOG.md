# Preset packs — changelog

One row per pack version. The in-file `pack <name> version "<n>"` line is the
source of truth; the smoke matrix fails when a pack's current version has no
row here, so a bump and its reason ship together. Newest first within a pack.
Dates before 2026-08-28 predate the public repository and are given to the day
the private history recorded them.

| pack | version | date | change |
|---|---|---|---|
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
