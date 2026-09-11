# satz workflows

Three walkthroughs, in the order most estates meet them: standing an organisation
up from nothing, bringing one that already exists under management, and keeping the
preset library current afterwards. Every command has its own reference section in
[the README](../README.md#cli-usage); this page is the order to run them in.

---

## From nothing to applied

### Prerequisites

The executing user needs:

- **Superadmin** access to the Google Workspace / Cloud Identity account.
- **Organization Administrator** on the Google Cloud organization.
- **Billing Account Administrator** on the target billing account (granted in the
  reseller console).

Authenticate, then write the estate and the folder structure around it:

```bash
gcloud auth application-default login

satz init \
  --customer-id "C01234567" \
  --customer-shortname "example-org" \
  --billing-account-infra "A12345-B67890-C12345" \
  --customer-domain "example.com" \
  --customer-organization-id "123456789012" \
  --iac-user "admin@example.com"
```

**Without the flags**, `satz interview yaml/<name>.satz --create` writes the estate and
asks for the
same seventeen values one question at a time, offering the derived ones as defaults; an
agent does the same over MCP. Both end at the file `init` would have written, and
[satz interview](interview.md) describes the rules they share: an answer is a param the
estate binds, and nothing below runs while one is missing.

### Bootstrap the organisation

`bootstrap` refuses while any question the estate's packs declare is unanswered;
`--dry-run` warns instead.

`bootstrap` creates the day-0 infrastructure — the infrastructure folder, the
management project, the billing link, the foundation APIs (which `tofu` needs
enabled before its first run) and the state bucket — then runs `transpile`, `init` and
the first imports, so what it created is under management from the start.

```bash
satz bootstrap C0example.satz
```

**Pre-flight.** Before anything is created, bootstrap verifies the ADC identity
against `first_admin` and tests the required PERMISSIONS — never roles — with
`testIamPermissions`:

| Where | Permission | Supplied by |
|---|---|---|
| scope root | `resourcemanager.folders.create` (only when `infra_folder_name` is set) | `roles/resourcemanager.folderAdmin` |
| scope root | `resourcemanager.projects.create` | `roles/resourcemanager.projectCreator` |
| scope root | `orgpolicy.policies.create` (the estate's policies, at first apply) | `roles/orgpolicy.policyAdmin` |
| billing account | `billing.resourceAssociations.create` | `roles/billing.user` |

- Everything granted → bootstrap proceeds.
- Something missing and the caller holds `setIamPolicy` on the scope root — the
  normal state of a fresh organization, whose creating super admin is auto-granted
  Organization Administrator (that role carries `setIamPolicy` but none of the
  create permissions) → bootstrap **self-grants** the missing roles to the caller,
  prints each grant with the exact `remove-iam-policy-binding` undo command, waits
  for IAM propagation and re-tests before proceeding.
- Something missing and no `setIamPolicy` (or the billing permission, which is
  never self-granted) → bootstrap prints the exact
  `gcloud … add-iam-policy-binding` commands for an administrator and stops
  **before creating anything**.

**Folder-scoped installs.** Set `customer_organization_id = "folders/<id>"` and the
estate installs under that folder: permissions are tested there, and org-root
operations are skipped, so an operator granted on the folder is not asked for
organization admin. Google grants `roles/orgpolicy.policyAdmin` only at organization
level: on a folder scope a missing `orgpolicy.policies.create` is reported as
advisory, and folder-level org policies need an organization-level grant before
their first apply.

**Dry run.** `satz bootstrap <estate> --dry-run` is read-only: it prints the plan,
verifies the identity and runs the same pre-flight (a would-be self-grant is
reported, not executed). Without credentials the plan still prints and the skipped
pre-flight is named (`pre-flight: SKIPPED`).

**What bootstrap does NOT do:** it creates no service account and grants no IAM
beyond the self-grant above — the IaC service account and its grants are declared in
the estate and come into being on the first `tofu apply`.

**Credential line.** Every live command prints one line before its first API call —
`credentials: <identity> (user ADC | impersonated service account | service account
key), quota project <p>` — so a wrong per-customer login shows before the first call
instead of as a later 403. `satz whoami` is the explicit check (`--offline` for
the file-only view; a user ADC file stores no identity, so the online form resolves
it via token introspection). `satz whoami <estate>` answers the other question — the
identity that estate's live commands actually run as, which on a cloud-mode estate is
its IaC service account and not you. Both halves print together, because a live
command uses both, and online it CHECKS them: one `generateAccessToken` (token
discarded) for whether this credential may become that account, and one
`projects.get` for whether the quota project is reachable. Given an estate that
compiles, it also tests the permissions the estate's resource types need
(`testIamPermissions` on the organization, the infra project and the billing account)
and names each missing one with the role that carries it. `--offline` reads the
estate file alone and says the checks were not made rather than implying they
passed.

**Quota project.** Set the ADC's quota project to a project the caller can reach,
normally the infra project:

```bash
gcloud auth application-default set-quota-project <infra_project_name>
```

A quota project the caller cannot see — for example a typo — does not work: commands
that only print it accept it, and every API call then fails with `UserProjectInvalid`,
or `report-compliance` with "live inventory unavailable". Live commands check it once
before the work and refuse with the project named.

**Impersonation.** On a `deployment_mode = "cloud"` estate, every live command
impersonates the estate's IaC service account
(`{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`) — exactly the
identity `tofu` applies with — so the human needs no org-wide read roles, only
`roles/iam.serviceAccountTokenCreator` on the SA (normally via membership in
`svc-iac-users`). `--no-impersonate` opts out; `bootstrap` never impersonates (day
0, the SA may not exist yet); an ADC that already impersonates is used as-is. The
credential line names the SA the calls actually run as.

**Greenfield: a tenant with no organization yet.** Google creates the Organization
resource for a Workspace/Cloud Identity domain when a NEW Google Cloud user signs in
to the console and accepts the terms, or when an EXISTING user creates their first
project or billing account
([documented](https://docs.cloud.google.com/resource-manager/docs/creating-managing-organization)).
satz uses the second trigger:

1. `satz init --from-live --customer-id <C0…>` — derives every derivable init value
   from the ADC alone (identity → `first_admin` + `customer_domain`,
   `organizations:search` → org id + directory customer id,
   `billingAccounts.list` → the single open account; explicit flags always win,
   nothing is guessed). With no organization visible, the estate is written with an
   empty `customer_organization_id`.
2. `satz bootstrap <estate> --greenfield` — creates the infra project WITHOUT a
   parent (the trigger), polls `organizations:search` until the new organization
   appears (matched by its `directoryCustomerId`, never "the first org"), moves the
   project under it, writes the id back into the estate, and continues with the
   normal pre-flight and build. If the organization never appears (the ADC user has
   not accepted the console terms), the timeout names the one-time console sign-in
   as the fallback.

An estate whose `customer_organization_id` is empty fails with exactly this guidance
instead of a bare "missing org id".

### Transpile, plan, apply

Bootstrap already transpiled and initialised. After any later edit to the estate,
compile it again and read the plan:

```bash
satz transpile C0example.satz

cd hcl/
tofu plan
tofu apply
```

The first apply creates the identity layer: the Cloud Identity groups, their IAM roles
(`Token Creator` among them) and the rest of the management project.

A pack added later can emit a resource type the IaC service account holds no role for.
The compile names the role, and `satz iac-roles C0example.satz --execute` writes it into
the estate's grant list. The apply then creates the grant and the resources that need it
in one pass; a resource that meets a permission error on the role just granted is
created by running the apply again once the grant has taken effect, which takes up to a
few minutes.

### Verify

Switch the state to the GCS bucket and the identity to impersonation:

```bash
satz migrate C0example.satz --mode cloud
```

`migrate` rewrites the estate's `deployment_mode`, switches to service-account
impersonation and runs `tofu init -migrate-state`. Impersonation applies to both
halves of the run: every provider block gets `impersonate_service_account`, and so
does the `gcs` backend, so the state bucket is read and written as the service
account rather than as the logged-in user.

> When the emitted backend changes, `tofu` refuses the next command until it is
> re-initialised: run `tofu init -reconfigure` once in `hcl/`. `migrate` re-initialises
> by itself.

Then check that the service account can run the plan:

```bash
cd hcl/
tofu plan
```

### The params `init` writes

The same seventeen, each with its question, are `presets/estate-core.satz` — what
`satz interview` asks when there are no flags. An estate `init` wrote binds all of
them and is complete; one the interview wrote is complete when it says so.

| Param | Default | Description |
|-------|---------|-------------|
| `infra_folder_name` | `"Infrastructure"` | Display name for the top-level folder. Leave `""` to create the project in the root. |
| `infra_project_name` | `""` | The unique id for the management (IaC) project. |
| `infra_bucket_name` | `""` | The GCS bucket for Terraform state. |
| `customer_id` | (from CLI) | The Workspace customer id (e.g. `C01234567`). |
| `customer_organization_id` | `"123456789012"` | The numeric Google Cloud organization id. |
| `customer_domain` | `""` | The customer's primary domain (e.g. `example.com`). |
| `first_admin` | (from `--iac-user`) | Local part of the first admin's address; members are built as `user:{first_admin}@{customer_domain}`. |
| `customer_longname` | `""` | The full legal name of the customer entity. |
| `customer_shortname` | `""` | A unique slug for the customer. Names that must be globally unique derive from it. |
| `svc_iac_account` | `"svc-iac-001"` | The primary IaC service account. |
| `svc_iac_users_group` | `"svc-iac-users"` | The Cloud Identity group for IaC administrators. |
| `billing_account_infra` | `""` | The billing account id (e.g. `012345-6789AB-CDEF01`). |
| `deployment_engine` | `"tofu"` | The IaC tool: `tofu` or `terraform`. |
| `deployment_mode` | `"local"` | `local` for day 0 (user ADC); `cloud` for day 1+ (impersonation). Switched by `satz migrate`. |
| `default_region` | `"europe-west3"` | Default region for regional resources. |
| `default_zone` | `"europe-west3-a"` | Default zone for zonal resources. |

---

## Adopting an organisation that already exists

### Discover

Capture what is there. From an existing Terraform/OpenTofu state:

```bash
tofu show -json > state.json
satz import state.json -o migration-discovery.satz
```

Or straight from Google Cloud, with no state at all:

```bash
satz import organizations/123456789012 -o migration-discovery.satz
```

Only the resource types marked `import: true` in `presets/import-config.yaml` are
taken (`--only` narrows further); enable more rows as needed — every row with an
`asset_type` can be switched on. The table covers the provider's 895 resource types:
389 with their Cloud Asset Inventory name (derived from the type name and checked
against Google's list, `presets/cai-asset-types.txt`), 296 that are not Cloud Asset
resources (IAM members, org-policy v1 shapes; state shape only), 209 marked
`TODO/UNKNOWN` (Cloud Asset does not inventory them, or the name could not be
derived — `scripts/update_import_config.py` prints what it tried).

A live resource whose provider block would not plan is never written: a required
attribute the asset data lacks is derived where it can be (`parent`,
`org_id`/`folder`/`project` from the asset path, a service account's `account_id`
from its email) and the resource is otherwise skipped with the attribute named.
Import ids of live resources are the asset path, with the project named by id (the
provider keeps a project NUMBER on import and the declared id would then force a
replacement). Verified on a test organization with folders, projects, services,
buckets, IAM, org policies, org/folder/project log sinks, a service account and an
essential contact: `tofu plan` = every resource imported, nothing added or
destroyed.

### Refine the hierarchy

The discovered estate compiles as-is and mirrors the live layout. Restructure it so
resources inherit their scope:

- Move projects into their folders.
- Nest resources (buckets, networks, …) inside their projects.
- Drop the attributes that are now inherited from context (`project_id` and its
  kind).

Then compress the repetitive parts into the language's own forms: group
`google_project_service` resources into a single `project_service` list, combine
individual IAM members into compact `project_iam_member` / `folder_iam_member`
blocks, and indent sub-structures (`project_service` with `disable_on_destroy`, for
one) where they belong.

### Reconcile

Generate the HCL and hold it against the live organisation:

```bash
satz transpile migration-discovery.satz
cd hcl/ && tofu plan
```

A plan that says *replace* where it should say *no changes* means the labels or the
resource ids do not line up. Two ways to fix it: declare `"import-id"` in the estate
to bind the existing resource, or `tofu state mv` to move the existing state onto
the new address. `satz adopt` resolves those ids for you where it can — natural-key
lookups for folders, groups, memberships and org policies, `import_id`/`match_on`
rules for the rest — and never guesses: one candidate resolves, several is
ambiguous.

### Hand over to satz

When `tofu plan` shows no changes, or only intended ones, the migration is complete;
from then on the infrastructure is changed through the estate.

The IaC service account reaches the adopted folders and projects, hand-made ones
included, through the roles it holds at the organization: every folder and project
inherits them. `satz iac-roles <estate>` names each role the adopted resource types need
that the estate does not grant it yet, and `--execute` writes them.

---

## Continuous verification

`scripts/fleet-v1.sh` checks, when run, whether every estate compiles on the current
binary. Two Cloud Build triggers check each estate continuously, and also whether the
live organisation matches it:

| trigger | when | runs | fails when |
|---|---|---|---|
| `satz-check` | every push to `main` | `satz transpile --check <estate>` | the estate no longer compiles |
| `satz-compliance` | nightly, Cloud Scheduler | `satz report-compliance <framework> <estate> --fail-on <statuses>` | a witness is DRIFTED or NOT ENFORCED |

Both are one pack:

```
use "presets/ci/verification-runner.satz"
use "presets/ci/verification-runner-grant.satz"
```

The runner is a service account of its own and holds nothing about the estate.
Inside the build, satz's first act is to exchange the runner's identity for the
estate's IaC service account — exactly what it does on a workstation — and the
grant pack is the one IAM binding that permits it. `satz whoami` inside the build
reports the same identities it reports on a workstation.

**Where the runner lives:**

- **Customer-hosted:** `ci_runner_project` is the estate's own infra project; both
  packs in the same estate; nothing to wire, the grant's default names the runner's
  own account.
- **MSP-hosted:** the runner pack in the MSP's estate, the grant pack in the
  customer's, with `ci_runner_service_account` set to the MSP runner's email. No
  credential is shared; one IAM binding grants the access. A Cloud Source Repositories
  trigger can only watch a repository in its own project, so the MSP runner must live in
  the project the estate repositories are hosted in.

**The build steps are inline in the trigger** — there is no `cloudbuild.yaml` in the
estate repository. Whoever controls the build file controls what runs as the runner;
in the hosted shape that file would sit in a repository the MSP does not own. Here
the pipeline is defined by whoever applies the pack. The steps install satz from the
release at build time (`ci_satz_release`, default `latest`) and, for the nightly run,
tofu, because `update-schema` needs it.

**Before the first nightly run,** enable Cloud Asset Inventory on the estate's infra
project and grant an asset-viewer role at organization level: `report-compliance`
needs both. `unverified` is not in the default `--fail-on` set, so a run without them
fails only on drift. The pack writes nothing back; the result is the exit code and the
build log. Committing evidence to the watched repository needs write access the runner
does not have by default.

## Keeping presets current

How to tell whether a newer preset exists, what to do about it, and which command to
reach for.

### Files and owners

There is **one** `presets/` folder per estate, and the **filename suffix declares
who owns the file**:

| file | owner | what may happen to it |
|---|---|---|
| `X.satz` | upstream | overwritable — this is a pristine copy |
| `X.local.satz` | you | never touched by any command |
| `X.diff.satz` | the tool | the current fork-vs-pristine delta, rewritten each `merge-presets` run |
| `<own>.satz` | you | no upstream counterpart, kept as-is |

Two more facts:

- **Pack versions live inside the file** — `pack CIS_GCP_Foundation_4_0 version "2.1"`.
  Filenames carry only the *framework* version (`CIS-GCP-Foundation-**4.0**`).
- **Upstream is the `presets/` tree on the repo's `main` branch**, fetched over the
  GitHub API. Not a release tag — a push to `main` publishes a preset immediately.

### The three commands

| command | reads | writes | estate-aware | protects you |
|---|---|---|---|---|
| `get-presets` | upstream | missing + unused files | yes | yes (refuses in-use; `--force` overrides) |
| `check-presets <estate>` | both | nothing | yes | n/a (read-only) |
| `merge-presets` | both | pristine names + forks + diffs | yes | yes |

**`get-presets`** populates the library: it installs what is missing and refreshes
what the estate does **not** use. A pristine pack the estate **does** use is
**refused**, naming the two commands that fit instead, because changing it changes
what the organisation enforces.
`--force` overrides, listing each in-use pack as it overwrites it.

**`check-presets <estate>`** is the read-only report. It walks the estate's `use`
graph, so packs the estate actually includes are tagged `[included]`, and drift in
an included pack exits non-zero — that is the CI gate.

**`merge-presets`** updates the library, and **a preset the estate includes never
changes silently.** When upstream has moved *semantically*, it
preserves your current content as `X.local.satz`, repoints the estate's `use` at
that fork, proves the repoint by transpile identity, refreshes the pristine
`X.satz`, and writes `X.diff.satz` — the exact delta adopting upstream would make.
Comment and formatting churn upgrades silently instead of forking.

### Is it stale, or edited?

Find out first, without touching anything:

```bash
satz --config <estate-dir> check-presets yaml/<ESTATE>.satz
```

The GitHub API allows 60 unauthenticated requests an hour, and each run spends one.
Without network access to GitHub, or with the quota spent, compare against a local
checkout:

```bash
satz --config <estate-dir> check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

`check-presets` reports two independent things: the **version line** says whether a
newer release exists; the **content comparison** says whether anyone edited this
copy.

- **clean** — identical, or only comments/formatting differ, and the version matches
  upstream.
- **STALE** — the version differs. A newer release exists. Printed with the pair,
  `local v1.5, upstream v2.1`, and with what moved. If the change is comment-only it
  says so and does not fail the gate.
- **EDITED (variables only)** — same version, only scalar defaults differ. The report
  prints the exact lines to lift into your estate's params.
- **EDITED (structural)** — same version, resource bodies or the variable set differ.
  A local edit — or an upstream release that changed without a version bump. Review
  by hand.
- **fork** — an `X.local.*` file. Never an error. If a pristine file reads STALE but
  its `.local` sibling exists, the report says so and tells you to leave the pristine
  copy alone: the estate runs the fork, and that copy is the fork's baseline.
- **missing locally** / **local-only** — new upstream preset / your own file.

Drift in an **`[included]`** preset exits non-zero — that is the CI gate.

To choose between adopting and merging, compare the local file with the release it
claims to be:

```bash
# what release does the local file claim to be?
grep -m1 '^pack' <estate>/presets/CIS-GCP-Foundation-4.0.satz     # -> version "1.5"

# is it byte-identical to that release?
cd ~/projects/satz
git log --format=%H -- presets/CIS-GCP-Foundation-4.0.satz \
  | while read c; do
      v=$(git show $c:presets/CIS-GCP-Foundation-4.0.satz | grep -m1 '^pack')
      echo "$c $v"
    done | head           # find the commit that carried v1.5
git show <that-commit>:presets/CIS-GCP-Foundation-4.0.satz > /tmp/pristine-1.5.satz
diff /tmp/pristine-1.5.satz <estate>/presets/CIS-GCP-Foundation-4.0.satz
```

| result | meaning | what to run |
|---|---|---|
| no diff | **STALE** — unchanged since that release | **adopt**: copy the pristine file in |
| diff | **EDITED** — a real local change | **`merge-presets`** — let it fork and give you `X.diff.satz` |

Without a baseline `merge-presets` cannot tell the two apart, so it **forks**. For an
edited pack that is correct; for a stale one it moves the estate onto a fork it does
not need. Adopt stale packs explicitly.

### Adopt, merge, or fork

#### Your copy is stale — adopt

```bash
satz --config <estate-dir> merge-presets --adopt CIS-GCP-Foundation-4.0 --report-only
satz --config <estate-dir> merge-presets --adopt CIS-GCP-Foundation-4.0
```

It overwrites the pristine name in place, leaves the estate's `use` alone, and prints
the **emission** delta — which resources appear or disappear, by address. `--adopt
all` does every pack that is merely BEHIND, and refuses one that differs at the
*same* version: that is an edit, and it has to be named. A fork+repoint needed in the
same run is **deferred** to a separate run: the repoint is proven by transpile
identity, and an adoption changes the output, so the two cannot share a run.

`merge-presets` does **not** regenerate `hcl/`. Continue with the normal gates:

```bash
satz --config <estate-dir> transpile yaml/<ESTATE>.satz

cd <estate-dir>
git status --short          # only presets/ + hcl/ should move
git diff hcl/main.tf        # THIS is the real review — the emission delta
satz --config . require cis-gcp-4.0 yaml/<ESTATE>.satz   # compare with the previous verdicts
satz --config . check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

Then read the plan **before** applying:

```bash
cd hcl && tofu plan
```

Adoption is only a no-op when the moved default is one your estate overrides, or the
pack is not `use`d at all. Otherwise expect a real plan and gate it with a runbook.

#### Your copy is edited — merge

```bash
cd <estate-dir>
git status --short                      # must be clean: auto-repoints refuse a dirty estate
satz --config . merge-presets --report-only   # preview every planned action
satz --config . merge-presets
```

Afterwards you have `X.local.satz` (your content, now the thing the estate uses), a
refreshed pristine `X.satz`, and `X.diff.satz` telling you exactly what adopting
upstream would change. Read the diff; adopt when you are ready by pointing the
estate's `use` back at the pristine name and deleting the fork.

Exit code is non-zero when anything needs attention — a fork was created, a fork's
upstream moved, or a repoint was refused. That is the CI signal.

#### The estate runs a fork already

If the estate `use`s `X.local.satz`, **copying pristine over `X.satz` changes nothing
it emits.** The change has to be made in the fork. Do not refresh the pristine
sibling: it is the fork's baseline for the next merge, and overwriting it loses the
record of where the fork branched.

### Rules of thumb

- **Never run `get-presets` on an estate whose packs are in use.** Use it to populate
  a new estate, or to fetch packs that are missing entirely.
- **`check-presets` in CI, `merge-presets` by hand.** The first is a gate, the second
  edits your estate and repoints `use` lines.
- **Read `git diff hcl/main.tf`, not the preset diff.** The preset diff tells you what
  changed upstream; the emission diff tells you what happens to the org.
- **Refreshing an unused pack changes nothing emitted**, and keeps the file from
  reading as a customer fork later.
- **`check-presets` answers "am I behind?" directly** — it prints the local and
  upstream version and a STALE verdict.
- **Declaring a legacy constraint off replaces its policy.** Each legacy constraint the
  CIS pack declares off (`spec { reset = true }`) carries a `-superseded` address, so the
  plan shows one **destroy + create** per legacy policy that exists live with rules. An
  in-place update does not work: the provider PATCHes the rules it holds in state
  together with `reset`, and the API refuses the pair — `400 Cannot set PolicyRules if
  reset is true`. The managed replacement enforces the control throughout, so the
  control stays in force between the destroy and the create.
- **`main.tf` says which satz emitted it.** The first line is
  `# Generated by satz vX — do not edit; re-emit from <estate>.satz.`, so one `grep`
  across a fleet finds every estate last emitted by an old binary:

  ```bash
  grep -h "Generated by satz" ~/estates/*/hcl/main.tf | sort -u
  ```

  The stamp shows which binary wrote the file, not whether the estate still compiles: a
  stricter release can break an estate whose stamp is current, and an old stamp can
  re-emit byte-identically. To check an estate, re-transpile and compare, which
  `scripts/fleet-v1.sh` does. The stamp is added when the file is written, not by the
  emitter, so snapshots do not move with a release, and as a comment it never reads as a
  delta in a block-level comparison.
- **`triage --fix` turns findings into the estate edit they imply.** `use` lines for
  bucket A (one per pack, with the controls each closes), the resources to bring under
  management for bucket D, and a line saying why B, C and E have *nothing* to edit. It
  proposes; it never writes the estate and never touches the cloud.
- **A pack that ADDS org policies may add ones Google already set.** The apply then
  fails with `already exists` on exactly those, because the organisation has the
  constraint and the state does not. Adopt them before applying:

  ```bash
  satz adopt <estate>.satz --only google_org_policy_policy            # read the table
  satz adopt <estate>.satz --only google_org_policy_policy --execute --import
  satz plan
  ```

  The dry run consults the state, so an address the state already manages reads
  `already managed in the state — skipped` and the summary counts it separately. A
  state that cannot be read is a note on the
  dry run — a first adopt has none — and a hard error on `--execute --import`, where
  every import would fail the same way.
- **A pack that RENAMES a block moves the state; it does not import again.** Renaming
  changes the name the estate gives an object, never the object in the cloud. Adopt
  therefore asks whether this *live object* is already managed — by an exact match on
  the resource type and the live id — and not merely whether this *address* is:

  ```
  google_org_policy_policy.compute_restrictProtocolForwarding_superseded
      MOVE   in state as google_org_policy_policy.compute_restrictProtocolForwarding —
             the same live object, so `state mv`, not an import
  ```

  `--execute --import` then runs `tofu state mv` for that row and reports it as
  `moved`; the summary counts moves apart from imports. Importing instead would put
  one live object into the state twice, and the next plan would propose to **destroy**
  it under its old address — which deletes it in the cloud. Only an exact type-and-id
  match is a move; anything else is an import.

  If the estate declares **both** ends — the old address as well as the new one — adopt
  stops and names the pair. A move cannot resolve one live object with two
  declarations; drop one from the estate first.

### When upstream stops answering: the GitHub quota

All three commands read the preset library from GitHub, and GitHub's unauthenticated
REST quota is **60 requests per hour, per IP** — shared with `satz self-update`. A
sweep across a fleet can exhaust it, which also blocks `self-update` until the quota
resets.

When the quota is exhausted, satz says:

```
GitHub API rate limit reached (60 requests/hour, unauthenticated). Retry in ~48
minutes, set GITHUB_TOKEN, or compare against a local checkout with
`--pristine-dir <checkout>/presets`.
```

Three ways around it:

- **`--pristine-dir <checkout>/presets`** — all three commands take it, and it makes
  no network request. Use it during a sweep when the satz repository is checked out.
- **`export GITHUB_TOKEN=…`** — any token, even one with no scopes, raises the quota
  to 5,000/hour. It is sent only to the API, never to the download host.
- **Wait.** The message says for how long, read from the reset the API reports.

One invocation costs **one** API request: the whole preset subtree arrives in a single
tree response, and the files themselves come from a host that is not rate-limited.

A 403 that is *not* the quota — a private repo, a bad token — reports as a plain
status instead, because waiting an hour would not fix it.
