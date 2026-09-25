# satz beyond Google Cloud: other HCL providers, other clouds, multicloud

## Context

A top-down estimate of what satz would need to change and add to serve other HCL providers and
clouds (AWS, Azure), what happens with multicloud in one estate, how hard each problem is, and a
list of do / don't for current work so today's changes do not make that extension more expensive.
It rests on three read-only surveys of the code (one each from the Google, AWS and Azure side).
It is an estimate, not an implementation design, and nothing in it is decided.

## Verdict in five lines

1. **The language is almost provider-neutral already.** The fold, `use`/`suppress`/`claim`, packs,
   params, questions, `fmt`, schema-typed bodies (ADR 0047), interfaces (ADR 0070), MCP and the
   claim → catalog → `require` machinery need no redesign. About six hard-wired string matches in
   `crates/satz-core` tie it to Google.
2. **The cost is outside the language.** It is in identity, discovery and adoption, live compliance
   verification, bootstrap and, above all, content: 46 packs, the catalogs, `import-config.yaml`,
   the prerequisites table and every derived data file. All of that has to be rebuilt for each
   cloud, and none of it is shared.
3. **There are three structural problems:**
   - **Identity.** Satz's "one command, one identity" rule does not survive AWS or Azure unchanged.
   - **Plan-time topology.** An AWS account id or Azure subscription id that doesn't exist yet
     cannot be targeted by a provider in the same apply.
   - **Policy semantics.** GCP policies are compared by value; AWS SCPs are policy documents;
     Azure policies work through an effect plus an assignment.
4. **Multicloud in ONE estate is the wrong unit.** Keep one estate per cloud root and link estates
   through interfaces. Multiple Satz files are already the norm; multiple *clouds* per estate breaks
   identity, state, blast radius and the "estate = one organisation" rule (ADR 0069).
5. **Non-cloud HCL providers are the cheap first step.** Examples: github, cloudflare, datadog,
   azuread next to a GCP estate. They have no hierarchy and no identity model, and they expose the
   prefix couplings at low cost.

## Top-down map: layer, what changes, toughness

| # | Layer | GCP coupling today | What AWS / Azure need | Toughness |
|---|---|---|---|---|
| 1 | Grammar, fold, packs, fmt | none | nothing | — |
| 2 | Schema typing (ADR 0047) | `ResourceRegistry` (`src/schema.rs:215`) already loads several providers; `find_resource` / `normalized_tf_type` (`crates/satz-core/src/pipeline.rs:1251`, `:475`) auto-prefix `google_`; `note_key` (`satz.rs:1654`) treats only `google_*` as resources | provider-aware prefix rules; aws / azurerm / azuread / azapi schema fixtures | **Easy**. Removing the auto-prefix is a language break (MINOR release, Breaking-changes entry). |
| 3 | satz's own body keys | `satz_body_key` (`pipeline.rs:66`): `project_service`, `org`, `member`/`manager`/`owner`/`email` have GCP meanings | a per-provider closed list | Easy–medium |
| 4 | Hierarchy as a language construct | `google_folder`/`google_project` NodeMaps (`pipeline.rs:1655–1903`, `pipeline/position.rs`), `Scope {Customer, Org, Billing, Node}` (`crates/satz-core/src/lib.rs:90`), parent attributes hard-coded (`condense.rs`, `emitter.rs::emit_project`), `scoped_by_node` (`pipeline.rs:2136`) | AWS: root → OU → account, plus sub-resources nested under their parent (`aws_s3_bucket_*`). Azure: management group → subscription → **resource group** (a mandatory 4th level that also carries inherited `location`). | **Medium, structural**. Nodes become per-provider data (type → role → inherited attribute). |
| 5 | Grants | `*_iam_member` suffix → member-map form (`type_facts`/`grant_form`, `pipeline.rs:1293–1340`) | Azure: `azurerm_role_assignment` (one type, scope = ARM id, principal = object GUID, not an email). AWS: Identity Center assignment (group × permission set × account). Plain IAM is not a grant. | Medium (Azure), hard (AWS) |
| 6 | Estate vocabulary / anchor | `customer_organization_id`, `infra_project_name`, `svc_iac_account` (~194 uses in 16 `src/` files), `derive_from_shortname` (`src/main.rs:1477`), day-0 skeleton (`src/template.rs`) | a per-cloud anchor: root scope, infra container, IaC principal, state location | Medium (spread out, not deep) |
| 7 | **Identity** | one ADC credential + `impersonate_service_account`; `src/gcp/identity.rs`, `IDENTITIES` (`src/main.rs:5203`), MCP per-call scope | AWS: entry principal + `assume_role` into a fixed role in **each account**, one provider per account. Azure: one service principal needing **two token audiences** (ARM for azurerm/azapi, Graph for azuread) with separate permission systems; a human cannot impersonate a service principal without federation. | **Structural**. The rule survives only restated as "one declared principal plus a declared role chain". |
| 8 | **Plan-time topology** | `project_id` is chosen in advance, so a project and its contents plan in one apply | the AWS account id is assigned by AWS; an Azure subscription comes from an asynchronous billing alias. A provider cannot target either in the same apply. | **Structural**: staged applies (vend, then populate) or pre-created containers. satz has no notion of stages today. |
| 9 | Emission | provider alias per project (ADR 0059), `configure_google_provider` (`src/emit_shared.rs:870`), gcs backend tied to impersonation (`emitter.rs:1150`), `google_project_service` (ADR 0036) | alias per account / subscription (same pattern, different keys); S3 or azurerm backend; Azure resource-provider registration (turn off azurerm's auto-registration); nothing to enable on AWS | Medium |
| 10 | Adoption / discovery | CAI sweep engine (`src/discovery.rs` 4.8K lines, `adopt.rs`, `import.rs`, `delta.rs`, `align.rs`); `presets/import-config.yaml` (283K), `cai-asset-types.txt` | AWS: Resource Explorer or Config aggregator, per account × region, and **one live resource = N Terraform resources** (breaks ADR 0068's one id per resource and ADR 0056's flattening). Azure: Resource Graph; ARM ids are uniform and are the import id, so it is **easier than GCP**. | Hard (AWS), medium (Azure). The `Live` trait (`src/adopt.rs:45`) is the right seam. |
| 11 | **Compliance semantics** | only `google_org_policy_policy` is judged (`src/compliance.rs:157–196`); verdict = the value of the unconditional rule; the org-policy tooling (`org_policy.rs` 1.8K, `policy_tree.rs` 1.3K, managed-constraint equivalents) | AWS: SCP/RCP are IAM JSON documents, so a verdict needs statement analysis or a narrower claim kind (declarative and tag policies can still be compared by value). Azure: definition + initiative + assignment + **effect** (Deny/Audit/DINE/Modify/Disabled); exemptions are resources (restates ADR 0015); live compliance state is better evidence than a value. | **Hard**. The proof core (claims, catalogs, require, findings, silence) stays. |
| 12 | Live verification & external evidence | `live_matcher` TF → CAI type `match` (`compliance.rs:1847`); Prowler `"gcp"` + framework map (`src/prowler.rs:35,148`); SCC packs | Security Hub / Config / Audit Manager; Policy Insights / Defender; Prowler aws / azure (same OCSF) | Medium; Prowler is easy |
| 13 | Prerequisites (ADR 0009) | `src/prerequisites.rs` rows = (type, roles, APIs); `Scope` = Organization / Project / BillingAccount / Workspace | AWS: IAM actions, evaluated per account, with SCPs able to deny. Azure: RBAC at management-group / subscription scope, plus Graph app roles, plus resource-provider namespaces. | Medium. Make each row a record. |
| 14 | Bootstrap / day 0 | `bootstrap.rs`, `preflight.rs`, `init` (all gcloud / GCP) | AWS: management vs delegated-admin account, a role in every member account (StackSet), Control Tower or not. Azure: Global Admin elevation + billing role + Graph app + federated credential, which are three separate privilege domains. | **Hard**, per cloud |
| 15 | Packs & catalogs (the product) | 46 packs, 49 `google_*` types, CIS GCP catalogs, monitoring, SCC, billing | CIS AWS / FSBP, CIS Azure / MCSB, the equivalent baselines, log sinks, budget packs, and so on, all written from scratch | **Large**: the biggest item in time and money; mechanically easy |
| 16 | Derived data & housekeeping | schema fixture, CAI types, import-config, constraint pairs, catalogs, prerequisites, plus the GCP refresh scripts | each file × 3 clouds, each with its own refresh script and gate (the CLAUDE.md "script, else gate, else a line" rule) | Medium, recurring |
| 17 | Privacy gate | GCP id shapes in `scripts/check-names.sh` + `src/privacy_shapes.rs` | 12-digit AWS account ids (already caught by the 11–13-digit rule, so false positives are likely), `o-…` / `r-…` / `ou-…` ids, ARNs, Azure subscription and tenant GUIDs (already rejected), ARM resource ids, Entra default domains | Medium: new shapes + new example values in `docs/examples.md` |
| 18 | Test infrastructure | one GCP test organisation, smoke matrix | an AWS test org and an Azure test tenant with near-zero cost, their own ADC-equivalent credentials, and live smoke steps | Medium, ongoing cost |

## Multicloud in one estate

**At the language level it would work**: fragments fold across providers, and the schema registry
holds several providers. **Everything else pushes against it**:

- **Identity.** One command would need a GCP service account, an AWS role chain and an Azure
  service principal at once, which breaks "one command, one identity" and the MCP per-call scope.
- **State and blast radius.** One state and one apply across clouds means a failed apply in one
  cloud blocks the others, and the lock is shared.
- **Scope rules.** "The estate's own organisation" (ADR 0069, `--into` sweeps) no longer has one
  meaning. `compliance_frameworks` and `customer_shortname` naming rules differ per cloud (Azure
  storage accounts allow 3–24 lowercase alphanumeric characters).
- **Staging.** Staged applies (row 8) multiply.

**Recommendation: one estate = one cloud root** (a GCP organisation, an AWS Organization, an Entra
tenant). A customer owns N estates. Cross-cloud wiring uses what already exists:
- **interfaces** (ADR 0070) export ids from one estate for another to read. Example: the GCP
  workload identity pool that trusts an Azure tenant, today in `presets/integrations/microsoft-*`.
- **claims against generic catalogs** (e.g. `iso27001-2022`) give the cross-cloud compliance view.

The later extension is a **customer level above estates**: a report or dossier that aggregates
several estates' `report-compliance`, not one estate spanning clouds.

**Rejected outright: a cross-cloud "unified resource" abstraction** (one `bucket` that emits GCS, S3
or Blob). It contradicts Satz's defining rule, "the provider's types, to the underscore". Cross-cloud
intent belongs in claims and catalogs, not in resource types.

## Suggested order, if it is ever pursued

0. **Non-cloud providers inside a GCP estate** (github, cloudflare, azuread for the Microsoft
   integrations). Remove the `google_` auto-prefix and make `satz_body_key` and `type_facts`
   per-provider. **S–M; one MINOR release.** This is useful on its own and flushes out the language
   couplings.
1. **Core tables**: hierarchy roles, grant forms, parent attributes, estate anchor, identity binding,
   prerequisites record shape. **M.** Worth doing only once a second cloud is committed.
2. **First cloud: Azure before AWS.**
   - For Azure: the existing Microsoft integration packs (Defender, Sentinel) already reach
     into it, and adoption is easier.
   - Against Azure: the resource-group level and the dual token plane.
   - AWS has two structural problems (account ids unknown at plan time; 1:N import) plus the
     Control Tower question.
   - **L–XL for each cloud**, dominated by packs, catalogs and import-config content.

## Do / don't in current work (the actual ask)

**Do: cheap now, expensive later.**
1. New type knowledge goes in **data tables, not `"google_…" =>` match arms**: `type_facts`,
   `grant_form` (`crates/satz-core/src/pipeline.rs:1293`), `live_matcher` (`src/compliance.rs:1847`),
   policy effects (`compliance.rs:157`), the parent-attribute tables (`condense.rs`,
   `emitter.rs::emit_project`).
2. New code reads the hierarchy parameters (`customer_organization_id`, `infra_project_name`,
   `svc_iac_account`) through **one resolved estate anchor**, not by re-deriving
   `…@….iam.gserviceaccount.com` in place (today in `emitter.rs:1057`, `main.rs:3222–3537`,
   `main.rs:4457`).
3. Keep the identity binding behind **one function that returns provider attributes plus
   credentials**, and state rules in terms of a *principal*, not an SA email. Rename
   `Identity::NoGoogleApi` → neutral when that code is next touched.
4. In new claim and compliance code, model enforcement as an **effect enum** (deny / audit /
   remediate / off), not `enforce: bool`.
5. Keep every Google API client inside `src/gcp/`. Discovery and verification go through the
   `Live`/sweep seam (`src/adopt.rs:45`), never through direct calls from command code.
6. Move the Prowler framework map and the `"gcp"` argument (`src/prowler.rs:35,148`) into catalog
   data when it is next edited.
7. Treat ADR 0059 as "one provider alias per container", and use that wording in the ADR when it is
   next revised.

**Don't.**
1. Don't add new uses of the `google_` auto-prefix (`pipeline.rs:475,1251`, `schema.rs:283`,
   `emit_shared.rs:870`).
2. Don't grow `satz_body_key` (ADR 0047's closed list) with more GCP-only meanings.
3. Don't deepen the one-live-resource = one-import-id assumption (ADR 0068) in `import.rs` and
   `adopt.rs` beyond what GCP needs.
4. Don't hard-code more provider-block lines (`impersonate_service_account`, `user_project_override`,
   `billing_project`) or the gcs backend in new places.
5. Don't add `Scope` variants tied to GCP concepts (`src/prerequisites.rs:24`,
   `crates/satz-core/src/lib.rs:90`) without first turning the prerequisites row into a record.
6. **Don't pre-abstract whole modules now.** An abstraction with one implementation is a guess.
   Rows 7, 8, 10 and 11 cannot be designed without a real second cloud, and the housekeeping and
   test cost of a speculative layer is paid on every change. Only the data-shaped choices above are
   free.
7. Don't generalise the docs' wording ("container" for "project"). Per the docs rule, they say what
   satz does now, which is GCP.
8. Don't plan multicloud-in-one-estate and don't design a unified cross-cloud resource type.

## Open decisions

- **D1: pursue at all?** If yes, Phase 0 is independently worth it. If no, the do/don't list still
  costs almost nothing.
- **D2: unit of multicloud.** The recommendation is one estate per cloud root, plus a customer-level
  aggregation later.
- **D3: first cloud.** Recommended: Azure. Settle before Phase 2.
- **D4 (AWS only): Control Tower or plain Organizations.** Blocks any AWS design.
- **D5: staged apply as a satz concept** (vend, then populate). Needed for both AWS and Azure, and
  blocks Phase 2.

## Basis

The analysis rests on three read-only code surveys (grep baselines: `google_`
1,730 hits, `organizations/`|`folders/`|`projects/` 800+, `impersonate` 186 in `src/` +
`crates/satz-core/src`). Before any Phase 0 or Phase 1 work, re-check the cited line numbers
against current `main`.
