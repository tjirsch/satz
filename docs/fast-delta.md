# FAST delta audit (Phase 0)

Date: 2026-08-22. Source: `GoogleCloudPlatform/cloud-foundation-fabric` @ master,
`fast/stages/0-org-setup/datasets/classic` (the full reference dataset).
Purpose: bound the content debt vs Google's Fabric FAST and generate interview
questions. This is an **audit, not an adoption plan** — FAST is greenfield
landing-zone tooling; Cloud Cockpit is CIS-mapped, brownfield-importing org
foundation. The deltas below are classified accordingly.

## FAST has been restructured (finding 0)

The old `0-bootstrap` + `1-resman` pair is gone. Current stages:

| Stage | Scope |
|---|---|
| `0-org-setup` | org policies, hierarchy, IAM, logging, billing export, automation projects, CI/CD — **all YAML-factory-driven** |
| `1-vpcsc` | VPC Service Controls perimeters (optional) |
| `2-networking` | shared VPC, hub-and-spoke/VPN/NVA designs, DNS, hierarchical firewall |
| `2-security` | KMS keyrings/keys, Certificate Authority Service, per-env security projects |
| `2-project-factory` | YAML project vending (services, IAM, budgets, shared-VPC attach) |
| `3-secops-dev` | Google SecOps instance config |

Strategically notable: Google moved FAST to **YAML factories with context
interpolation** (`$defaults:…`, `$iam_principals:…`, `${organization.id}`) —
structurally the same design as our params + interpolation. Validates the Satz
param model; their "datasets" are roughly our estates.

## Org-policy constraint diff (0-org-setup classic vs CIS pack v1.3)

**Overlap (14):** requireOsLogin, vmExternalIpAccess,
restrictProtocolForwardingCreationForTypes (both INTERNAL-only),
setNewProjectDefaultToZonalDNSOnly, skipDefaultNetworkCreation,
allowedContactDomains, resourceLocations (FAST ships allow-all placeholder; we
ship a real value), allowedPolicyMemberDomains, automaticIamGrants…,
disableAuditLoggingExemption, disableServiceAccountKeyCreation/-Upload,
publicAccessPrevention, uniformBucketLevelAccess. Note: FAST still uses the
**legacy** constraints where we use managed twins (osLogin, externalIp, SA
keys) — we are ahead on the managed migration.

**Ours only (5):** compute.managed.vmCanIpForward, compute.requireVpcFlowLogs,
gcp.detailedAuditLoggingMode (a CIS control FAST skips!),
iam.managed.allowedPolicyMembers (principal-sets twin, our v1.2/v1.3 work),
iam.managed.preventPrivilegedBasicRolesForDefaultServiceAccounts.

**FAST only (~22)**, grouped by adoption class:

- *Cheap org-level hardening, candidate CIS-pack/hardening-pack adds (5):*
  `iam.serviceAccountKeyExposureResponse = DISABLE_KEY` (reactive control,
  enforce-and-forget), `iam.managed.disableServiceAccountApiKeyCreation`,
  `iam.workloadIdentityPoolAwsAccounts` deny-all + `…PoolProviders` deny-all
  (WIF supply-chain lock — needs interview gate: breaks orgs using WIF),
  `storage.secureHttpTransport`.
- *Workload-scoped — bite only when the service is used; belong in **optional
  workload packs** gated by interview questions, not in the foundation pack
  (14):* compute.disableGuestAttributesAccess, disableInternetNetworkEndpointGroup,
  disableNestedVirtualization, disableSerialPortAccess, disableVpcExternalIpv6,
  restrictLoadBalancerCreationForTypes (INTERNAL), trustedImageProjects
  (25-entry Google-images allowlist), container.managed.enablePrivateNodes,
  run.allowedIngress, run.managed.requireInvokerIam,
  sql.restrictAuthorizedNetworks, sql.restrictPublicIp,
  storage.restrictAuthTypes (deny HMAC), cloudbuild ×3
  (disableCreateDefaultServiceAccount, useBuildServiceAccount,
  useComputeServiceAccount).
- *Custom constraints:* custom.denyBridgePerimeters, plus ACM/GKE custom
  constraint factory — out of scope until VPC-SC is.

## The pattern worth stealing: tag-conditional escape hatches

FAST's domain lock is a **dual-rule policy**: rule 1 allows only
`is:${customer_id}` *unless* the resource carries the org tag
`org-policies/allowed-policy-member-domains-all`; rule 2 allows all *when* it
does. Same for essentialcontacts.allowedContactDomains. The default stays
enforced org-wide; exemption is per-subtree tagging, no lift-and-retighten
window.

This is directly relevant to **task #12 (SCC activation)** and the DRS
service-agent caveat: instead of temporarily disabling the legacy domains
policy, a v1.4 CIS pack could ship the dual-rule shape + an `org-policies` tag
key, and the SCC runbook becomes "tag, activate, untag". Open question before
adopting: confirm a CIS auditor accepts conditional enforcement as compliant
(the benchmark tests the effective policy on untagged resources — it should).

## Non-policy content FAST has that we don't

| Area | FAST | Classification |
|---|---|---|
| Automation foundation | iac-0 project: per-stage rw/ro SAs, state + outputs buckets, WIF CI/CD (GitHub/GitLab/AzDO/Okta), generated workflows | **Interview question** (what runs tofu?), not preset debt — our model is owner-run today |
| Custom roles | 9 (org_admin_viewer, org_iam_admin, project_iam_viewer, tag_viewer, storage_viewer, network/NGFW ×4) | Low-value for us until least-privilege automation SAs exist |
| Tags | context/environment/org-policies keys, tag bindings, tag-conditional IAM | **Adopt the org-policies tag key** with the escape-hatch pattern; rest deferred |
| Billing export | billing-0 project, BQ dataset export | Preset candidate (small); we only have budgets |
| Log sinks | 3 sinks → log buckets: audit (incl. `access_transparency`, `policy` log_ids), dedicated **iam sink** (iam/iamcredentials/sts), vpc-sc sink | Filter delta absorbable via `logsink_filter` param; iam sink is a nice-to-have preset add |
| Hierarchy model | networking/security/teams × dev/prod folders with per-folder IAM/policies | **Interview question** (hierarchy shape) — estates own this today, correctly |
| Whole stages 1/2/3 | VPC-SC, shared-VPC networking designs, KMS/CAS, project factory, SecOps | **Deliberate scope boundary** — landing-zone content, not compliance foundation. Not debt. Project factory overlaps our future interview→derive flow conceptually |

## What we have that FAST doesn't (kill-check inputs)

- **Compliance plane**: claims → CIS 4.0/5.0 catalogs → `require` goal view →
  `report-compliance` live evidence. FAST has zero control mapping.
- **CIS §2.4–2.12 alert stack shipped working** (metrics, policies, channels,
  central bucket). FAST's observability factory exists but ships **empty, all
  commented out** (one sa-impersonation example).
- **Data-access audit logs (§2.1)**: FAST has a literal `# TODO: data access
  logs` and only sts ADMIN_READ; we ship allServices ADMIN_READ/DATA_READ/WRITE.
- **Brownfield import**: import-ids/import blocks throughout; FAST is
  greenfield-only (prefix-named new projects).
- **Group creation** (s1 model creates the 5 groups; FAST requires
  gcp-organization-admins to pre-exist). Role sets are near-identical —
  independent confirmation of the s1 role model.
- Managed-constraint twins (iam.managed.allowedPolicyMembers etc.).
- Org budget preset with threshold rules (FAST does budgets only per-project in
  the project factory).

## Content-debt bound & recommendation

The org-foundation gap is **small and cheap**: ~5 hardening constraints
(≈half a day incl. claims wiring) plus optional workload packs (sql/run/gke/
cloudbuild/compute-hardening — a day each at most, mostly enforce-true
singletons). The big FAST surface (networking, KMS, VPC-SC, project vending)
is a **scope decision, not debt** — adopting it would make us a landing-zone
clone and dilute the compliance differentiator.

Concrete follow-ups, in value order:

1. CIS pack v1.4 spike: tag-conditional dual-rule shape for the §1.1 locks
   (fixes task #12 structurally).
2. Hardening additions: serviceAccountKeyExposureResponse,
   disableServiceAccountApiKeyCreation, secureHttpTransport (+ WIF deny pair
   behind an interview gate).
3. Workload packs as interview modules ("Do you run Cloud SQL / Cloud Run /
   GKE / Cloud Build?") — this is the Phase 3 hook.
4. Small preset adds when convenient: iam log sink, access_transparency/policy
   log_ids in the audit filter, BQ billing export.

## Interview questions this audit generates

1. Which workload services are in use: Compute VMs, GKE, Cloud Run, Cloud SQL,
   Cloud Build? → selects workload policy packs.
2. Data residency requirement? → gcp.resourceLocations values.
3. Any workload identity federation / external CI? Which platform? → WIF
   deny-all vs configured providers; automation model (owner-run vs CI).
4. Escape-hatch policy: are tag-based per-subtree exemptions acceptable to
   your auditors, or hard org-wide enforcement only?
5. Folder hierarchy shape and environment split (dev/prod folders?).
6. Billing export to BigQuery wanted? Budget thresholds?
7. Trusted image sources (Google-only allowlist vs custom image projects)?

## Kill-check (same date)

Question: does anyone ship CIS/OSCAL-mapped, brownfield-importing GCP org
presets? **Verdict: no kill — but the category now has a Google entrant.**

- **`GoogleCloudPlatform/gcp-hardening-toolkit` (GHT)** — the closest thing,
  and new (created 2025-12, actively pushed, ~41 stars). Positioning overlaps
  ours almost word-for-word: "remediate complex brownfield environments,"
  incremental guardrails, state-aware IaC. BUT the mechanism is a **Gemini CLI
  LLM agent** that reads Cloud Asset Inventory / SCC exports from BigQuery and
  free-form-generates Terraform blueprints from a module library. Zero CIS
  references in the entire repo, no OSCAL, no `import` machinery, no
  goal-vs-live evidence plane; "compliance" = HIPAA/SOC2/PCI blueprint
  bundles. Its SCC blueprint is plain bash `gcloud` scripts (service + SHA
  module enablement) and is silent on the §1.1 lock collision we documented.
- **FAST**: greenfield-only, no control mapping (see above).
- **compliance.tf**: paid CIS-enforced module library — **AWS-only**,
  module-level, no org foundation, no import.
- **OSCAL世界** (iac2oscal, ScaleSec writings, awesome-oscal): mapping
  examples and GRC tooling, nothing that *provisions* GCP org foundations.

Reading: the thesis is validated — Google itself just created the
"brownfield GCP hardening via interactive agent" category — and nobody
occupies our exact square: **deterministic** compile (interview → derive →
folded-IR check, reproducible, auditable) + CIS-mapped claims with live
evidence (`require` / `report-compliance`) + brownfield **import into an
ongoing managed estate** rather than one-off remediation blueprints. GHT is
the competitor to watch and the sharpest articulation of why determinism
matters: an auditor can replay our derivation; a Gemini transcript they
cannot. The Phase 3 interview layer is now also a competitive response, not
just a UX feature.
