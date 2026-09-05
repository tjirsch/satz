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

### Bootstrap the organisation

`bootstrap` creates the day-0 infrastructure — the infrastructure folder, the
management project, the billing link, the foundation APIs (which is the
chicken-and-egg problem) and the state bucket — then runs `transpile`, `init` and
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
operations are out of scope by design — a folder-granted operator is never asked to
become org admin. One caveat: Google allows `roles/orgpolicy.policyAdmin` only at
organization level, so a missing `orgpolicy.policies.create` is reported as advisory
on a folder scope — folder-level org policies need an organization-level grant
before their first apply.

**Dry run.** `satz bootstrap <estate> --dry-run` is read-only: it prints the plan,
verifies the identity and runs the same pre-flight (a would-be self-grant is
reported, not executed). Without credentials the plan still prints and the skipped
pre-flight is named (`pre-flight: SKIPPED`).

**What bootstrap does NOT do:** it creates no service account and grants no IAM
beyond the self-grant above — the IaC service account and its grants are declared in
the estate and come into being on the first `tofu apply`.

**Credential line.** Every live command prints one line before its first API call —
`credentials: <identity> (user ADC | impersonated service account | service account
key), quota project <p>` — so a wrong per-customer login surfaces immediately
instead of as a downstream 403. `satz whoami` is the explicit check (`--offline` for
the file-only view; a user ADC file stores no identity, so the online form resolves
it via token introspection). `satz whoami <estate>` answers the other question — the
identity that estate's live commands actually run as, which on a cloud-mode estate is
its IaC service account and not you. It reads the estate file alone, so it needs
neither a network nor the right to impersonate yet.

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

This first apply is where the identity layer comes into being: the Cloud Identity
groups, the IAM roles that hang off them (`Token Creator` among them) and the
management project's finishing touches.

### Verify

Switch the state to the GCS bucket and the identity to impersonation:

```bash
satz migrate C0example.satz --mode cloud
```

`migrate` rewrites the estate's `deployment_mode`, switches to service-account
impersonation and runs `tofu init -migrate-state`. Impersonation applies to both
halves of the run: every provider block gets `impersonate_service_account`, and so
does the `gcs` backend, so the state bucket is read and written as the service
account rather than as the human who happens to be logged in.

> An estate that was ALREADY in cloud mode before this shipped gains the backend
> attribute on its next transpile. That is a backend configuration change, so
> `tofu` refuses the next command until it is re-initialised — run
> `tofu init -reconfigure` once in `hcl/`. Estates migrated by the command above
> need nothing; it re-initialises for you.

Then prove the restricted identity can do the work:

```bash
cd hcl/
tofu plan
```

### The params `init` writes

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
resources (IAM members, org-policy v1 shapes; state shape only), 209 still
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

The discovered estate compiles as-is, but it is as found. Give it the shape Satz
rewards:

- Move projects into their folders.
- Nest resources (buckets, networks, …) inside their projects, so attribute
  inheritance can do its work.
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

Once `tofu plan` shows no changes — or only changes you intended — the migration is
done, and the estate is the only way the infrastructure is managed from here.

---

## Keeping presets current

How to tell whether a newer preset exists, what to do about it, and which command to
reach for.

### The mental model

There is **one** `presets/` folder per estate, and the **filename suffix declares
who owns the file**:

| file | owner | what may happen to it |
|---|---|---|
| `X.satz` | upstream | overwritable — this is a pristine copy |
| `X.local.satz` | you | never touched by any command |
| `X.diff.satz` | the tool | the current fork-vs-pristine delta, rewritten each `merge-presets` run |
| `<own>.satz` | you | no upstream counterpart, kept as-is |

Two more facts that decide everything below:

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
**refused**, naming the two commands that fit instead — because changing it changes
what the org enforces, and a `tofu plan` would be the first place you noticed.
`--force` overrides, listing each in-use pack as it overwrites it.

**`check-presets <estate>`** is the read-only report. It walks the estate's `use`
graph, so packs the estate actually includes are tagged `[included]`, and drift in
an included pack exits non-zero — that is the CI gate.

**`merge-presets`** is the safe write path. Its contract is: **a preset your estate
includes never changes silently.** When upstream has moved *semantically*, it
preserves your current content as `X.local.satz`, repoints the estate's `use` at
that fork, proves the repoint by transpile identity, refreshes the pristine
`X.satz`, and writes `X.diff.satz` — the exact delta adopting upstream would make.
Comment and formatting churn upgrades silently instead of forking.

### Is it stale, or edited?

Find out first, without touching anything:

```bash
satz --config <estate-dir> check-presets yaml/<ESTATE>.satz
```

Rate-limited? The GitHub API allows 60 unauthenticated requests an hour and this
command spends about fifteen. Compare against a local checkout instead:

```bash
satz --config <estate-dir> check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

`check-presets` reports two independent axes, and keeping them apart is the whole
point: the **version line** says whether a newer release exists; the **content
comparison** says whether anyone edited this copy.

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

Then the decision the tool cannot make for you, because it changes what you run:

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
| no diff | **STALE** — nobody edited it, it is simply old | **adopt**: copy the pristine file in |
| diff | **EDITED** — a real local change | **`merge-presets`** — let it fork and give you `X.diff.satz` |

Getting this wrong in the safe direction is what `merge-presets` does by default:
without a baseline it cannot distinguish the two, so it **forks**. That is right when
you edited the pack, and wrong when the file is merely old — it takes an estate off
the pristine track for nothing.

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
same run is **deferred**, not done silently — the repoint proves itself by transpile
identity, and an adoption legitimately changes the output, so the two cannot share a
run.

`merge-presets` does **not** regenerate `hcl/`. Continue with the normal gates:

```bash
satz --config <estate-dir> transpile yaml/<ESTATE>.satz

cd <estate-dir>
git status --short          # only presets/ + hcl/ should move
git diff hcl/main.tf        # THIS is the real review — the emission delta
satz --config . require cis-gcp-4.0 yaml/<ESTATE>.satz   # verdicts should not surprise you
satz --config . check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

Then read the plan **before** applying:

```bash
cd hcl && tofu plan
```

Adoption is only a no-op when the moved default is one your estate overrides, or the
pack is not `use`d at all. Otherwise expect a real plan and gate it with a runbook.
Nine estates went through exactly this on 2026-08-24; seven produced `1 to change, 1
to destroy`.

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
it emits.** The change has to be made in the fork. Do not "refresh" the pristine
sibling either — it is the fork's historical baseline for the eventual merge, and
overwriting it destroys the only record of where the fork branched.

### Rules of thumb

- **Never run `get-presets` on an estate whose packs are in use.** Use it to populate
  a new estate, or to fetch packs that are missing entirely.
- **`check-presets` in CI, `merge-presets` by hand.** The first is a gate, the second
  edits your estate and repoints `use` lines.
- **Read `git diff hcl/main.tf`, not the preset diff.** The preset diff tells you what
  changed upstream; the emission diff tells you what happens to the org.
- **An unused pack is free to refresh** — zero emission delta, and it keeps the file
  from later reading as a customer fork.
- **`check-presets` answers "am I behind?" directly** — it prints the local and
  upstream version and a STALE verdict.

### When upstream stops answering: the GitHub quota

All three commands read the preset library from GitHub, and GitHub's unauthenticated
REST quota is **60 requests per hour, per IP** — shared with `satz self-update`. A
sweep across a fleet can exhaust it, and then be the reason your own `self-update`
stops working.

Exhaustion says so plainly:

```
GitHub API rate limit reached (60 requests/hour, unauthenticated). Retry in ~48
minutes, set GITHUB_TOKEN, or compare against a local checkout with
`--pristine-dir <checkout>/presets`.
```

Three ways out, cheapest first:

- **`--pristine-dir <checkout>/presets`** — all three commands take it, and it makes
  no network request at all. If you have the tool's repository checked out, this is
  the fastest answer and the one to reach for during a sweep.
- **`export GITHUB_TOKEN=…`** — any token, even one with no scopes, raises the quota
  to 5,000/hour. It is sent only to the API, never to the download host.
- **Wait.** The message says for how long, read from the reset the API reports.

One invocation costs **one** API request: the whole preset subtree arrives in a single
tree response, and the files themselves come from a host that is not rate-limited.

A 403 that is *not* the quota — a private repo, a bad token — reports as a plain
status instead, because waiting an hour would not fix it.
