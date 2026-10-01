# satz changelog

What a satz release refuses that the release before it compiled, with the edit that
satisfies it, and one row per pack version: the history of [the library](README.md).
A change that refuses a form an estate may hold adds its entry under Breaking changes,
and a pack's version bump adds its row under Changelog, in the same change;
`satz doc-packs` reads the table and refuses a version with no row.

## Breaking changes

What a satz release refuses that the release before it compiled, and the edit that
satisfies it. Newest first. Each entry says what is refused, how to find it in an
estate, what to write instead, and whether the plan moves; the error satz prints
names the file and the line.

### v0.90.0

**A live import adds to a project's `project_service` list every API the resources it
writes inside that project need and the project has off, and given an estate it bills
its reads to the estate's infrastructure project.** `satz import <scope>` wrote such a
project's resources and left the API off, and `satz plan` stopped with a 403 per resource
("Cloud Logging API has not been used in project … or it is disabled"). The import now
writes the API with its import id (`{ service = "logging.googleapis.com" "import-id" =
"<project>/logging.googleapis.com" }`) and names it under "API(s) added to a project's
`project_service` list"; `satz plan` and `satz apply` switch it on in that project before
`tofu` starts, and the plan imports it. Given an estate (`--into`, `--as`), the run bills
every read to `infra_project_name` and switches on there any of `cloudasset`,
`cloudbilling`, `cloudidentity`, `cloudresourcemanager`, `orgpolicy` and `serviceusage`
that is off, before the sweep.

Find it: the import's output names each added API and each API it switched on in the
infrastructure project. **The edit:** none. The next `satz import --into` rewrites the
packs with the added entries, and the next `satz plan` switches each API on and plans
the service as an import where it stopped on a 403 before. A `tofu plan` run directly,
not through `satz plan`, needs `gcloud services enable <api> --project <project>` first
for each named API.

**`satz add-project` is gone.** A project is onboarded by an entry of `projects` in
`presets/project-onboarding.satz`, which writes the project's Google project, IaC service
account, state bucket, grants and `interface "<name>"` per entry.

An estate that holds a section `add-project` wrote keeps it: the section is plain Satz and
compiles as it did, and its plan does not move. Find one: `grep -n 'written by .satz
add-project' satz/*.satz`. A new project is an entry, not a section:

```
params {
  use_project_onboarding    = true
  project_onboarding_folder = "google_folder.workload_folder.name" // "" when the workload folder is the organisation
  projects = [
    { name = "billing" owner_group = "billing-owners@example.com" },
  ]
}

use "presets/project-onboarding.satz" when use_project_onboarding
```

A project that already has a section gets no entry as well: the entry declares the section's
resources again, under the same labels and names. What
`--use-interface` and `--export` added is a block of the interface's name in the estate's
own file, `interface "billing" { use interface "network" }`; what `--interface-only` wrote
is an `interface "<name>" { … }` block written by hand.

### v0.89.0

**A live import keys a resource whose asset data states no name by its asset name's own
segment.** `satz import <scope>` wrote such a resource under its whole Cloud Asset name
(`"--compute-googleapis-com-projects-acme-infra-001-zones-europe-west3-b-instancesettings-instancesettings"`);
it now writes the last segment, and a singleton by what it is the singleton of
(`"europe-west3-b-instancesettings"`, prefixed with the project where two projects hold the
same one). A `google_compute_instance_settings` is the resource this is known for.

Find it: a pack an earlier `satz import --into` wrote (`imported-*.satz`) that holds a key
starting `--`. A discovered estate written by a plain `satz import` keeps its keys; only a
new import writes the new ones. **The edit:** none until the next `satz import --into`
rewrites that pack. If the resource was already imported into the state under the old
label, the next plan after that rewrite shows the old address destroyed and the new one
imported: move it first, `tofu state mv '<type>.<old label>' '<type>.<new label>'`, and the
plan does not move.

**An estate that uses both the CIS baseline and the Defender foundation plans one subject
fewer.** `presets/integrations/microsoft-defender-for-cloud.satz` no longer contributes
`serviceAccount:mdc-agentless-scanning@guardians-prod-diskscanning.iam.gserviceaccount.com`
to `allowed_policy_member_subjects`: the pack grants that account nothing, and the grant it
needs belongs to Defender's agentless-scanning plan, which the library does not ship. The
next plan updates `google_org_policy_policy.iam_managed_allowedPolicyMembers` with that
entry removed from `allowedMemberSubjects` — one list entry, nothing else.

Find it: an estate is affected when it uses the Defender foundation pack and the CIS
baseline, and its own `allowed_policy_member_subjects` does not name the account. **The
edit:** none when the estate does not use agentless scanning — the entry admitted a
principal nothing in the estate grants a role to. An estate that DOES use agentless
scanning, onboarded outside the library, adds the account to its own
`allowed_policy_member_subjects` and its plan does not move; without it the managed §1.1
constraint refuses the scanner's grants. An estate that already names the account keeps
it and its plan does not move.

**`satz import <dir>` over `.tf` files writes a different estate when the configuration's
default `google` provider sets a `project`, `region` or `zone`, uses a variable with no
default as the organisation, or expands a `count` over a list that names an entry twice.**
The new estate plans as the source does. An estate imported before is not read again and
needs no edit; a re-import is the one that differs. Find a configuration it applies to:
`grep -nE '^provider|org_id *= *var\.|element\(' <dir>/*.tf`.

- The resources that name no project take the provider's default project: they sit in the
  imported project of that id, or write it as their own `project`. The default region and
  zone are in the estate's `providers { google { … } }`.
- A `provider` block that a verbatim block still uses is carried verbatim, and
  `--wrap-all` carries every `provider` block.
- `org_id = var.org_id` with no default is written `org_id = customer_organization_id`, the
  organisation `--organization` names. Before, the import wrapped the project and refused
  the input.
- A repeated list entry is one resource.

**A translated resource that references a resource relying on a provider default satz
cannot write is refused.** The default is a `data.` reference, for example. The import
names both sides, the same refusal as any reference into a verbatim block:

```
the import would write an estate `satz transpile` refuses, so nothing was written: a translated resource references an address this estate does not emit.
  main.tf:9 `google_storage_bucket.b` references `google_service_account.sa`, which stays verbatim inside `hcl trust` (main.tf:5 — it names no `project` and relies on the provider's default project `data.google_project.current.project_id`, which satz cannot write: a reference to `data`, which is neither a promoted param nor a resource of the provider schema)
Either make the referenced block translatable (its reason is above), import the file that declares it too, or carry everything verbatim with `--wrap-all`.
```

**The edit:** give the referenced resource its own `project` in the `.tf` file, or import
with `--wrap-all`.

### v0.88.0

**An API a resource inside a `google_project` node needs is judged on THAT project.** The
node's provider alias bills its calls to the project itself, so Google tests the API
there; the compile, `bootstrap` and `transpile --apply` judged every API against the
infrastructure project's `project_service` list instead. A resource inside a project
node whose API is missing from that project's list is now a `prerequisites` finding at
every compile, a warning on `transpile --plan`, and a refusal on `bootstrap` and
`transpile --apply`:

```
apply refused: 1 API(s) this estate's resources need are not enabled on the project their calls are billed to — storage.googleapis.com on corp-data-001. `satz update-prerequisites e.satz` writes them into the estate.
```

Find it: `satz update-prerequisites <estate> --report-only` lists each missing API with
its project. **The edit:** add the API to that project's `project_service = [ … ]` list
— `satz update-prerequisites <estate>` writes it when the project is declared in the
estate file, and names it for a project declared in a pack. An API that only the
infrastructure project enabled for a project node's resources may stay there; nothing is
removed. The plan moves by the `google_project_service` each added entry emits.

`update-prerequisites --format json` and `satz_update_prerequisites` carry `project` on
every entry of `apis` and `missing_apis`, `enable_missing_apis` is a list with one
`gcloud services enable` line per project, and `default_billing_project` and `unbilled`
are new; `infra_project` and `infra_services` are what `bootstrap` creates and enables. A
script that read `enable_missing_apis` as a string reads the list.

### v0.87.0

**`satz check-request <file> <estate>` refuses a request file the estate's compile would
refuse once it is vendored.** After the shape checks the estate is compiled with the file
in place, so an entry without a field the pack's `each` body reads, which passed before
and broke the estate's compile after the pull request, is refused at the check:

```
check-request: requests/payments.satz fits the request points of satz/e.satz, and the estate refuses it once used — presets/shared-network.satz:97 `google_compute_subnetwork`, entry `payments`: `each.cidr`: the entry has no field `cidr` — it has name
```

**The edit:** give the entry every field the pack reads; the interface README's *What you
may request* section lists them.

**`satz add-project --name <n>` is refused when a pack the estate uses declares
`interface "<n>"`**, as it was for the estate's own text; before, the section merged into
the pack's common interface and reached every project's folder. **The edit:** another name.

**A `{param}` inside a claim's `reason`, `interpretation` or `duty_<id>` is refused.** These
strings are literal, as every other statement string is; before, the interpolation was
dropped and the text kept the words around it. Find it: `grep -nE '^ *(reason|interpretation|duty_[a-z_]+) *= *".*\{' satz/*.satz`.

```
error    front-end  satz/e.satz:12
    claim: reason: no interpolation allowed
```

**The edit:** write the value out (`reason = "the customer's own SIEM"`), or drop the brace.
The plan does not move: a claim emits nothing.

**A bare `duty = "…"` in a claim is refused.** A duty carries its id in its key,
`duty_<id> = "…"`; the bare key was read as a duty named `duty`.

```
error    front-end  satz/e.satz:13
    duty: write it as an attribute, `duty_<id> = "text"`
```

**The edit:** name the duty, `duty_review = "…"`. The plan does not move.

**`satz init --interface <path>` without `--project <name>` is refused** by the command
line (the flag was accepted and ignored). **The edit:** name the project, or drop the flag.

### v0.86.6

**An estate that uses `estate-map.satz` and does not bind `use_shared_network` is refused by
`transpile --apply` and `bootstrap`.** estate-map 2.6 asks a new question, and an apply needs
every question answered, default or not. Find it: `satz questions <estate> --unanswered
--format text --out -`.

```
apply refused: 1 question(s) unanswered — use_shared_network. Every question must be answered before the estate touches an organisation. `satz questions satz/<estate>.satz --unanswered --format text --out -` lists them with their defaults; write the answer (or the default) into the estate's params.
```

**The edit:** `use_shared_network = false` in the estate's `params` (`true` asks the pack's own
questions next). The plan does not move with `false`.

### v0.86.5

**A resource labelled `request`, written bare inside a resource type map, is refused.**
`request` is a statement word. Find it: `grep -n '^ *request {' satz/*.satz`.

```
error    front-end  satz/e.satz:6
    `request` is a Satz statement: it is written at the top level of a file, where it declares what a project may add to a list param. Directly inside `google_folder { … }` it is read as a folder named `request`. Move it to the top level of the file; one that really is called `request` is written quoted, `"request" { … }`
```

**The edit:** write the label quoted, `"request" { … }`. The plan does not move: the address
is the same.

### v0.86.2

**A resource labelled `private`, written bare inside a resource type map, is refused.**
`private` is a statement word. Find it: `grep -n '^ *private {' satz/*.satz`.

```
error    front-end  satz/e.satz:6
    `private` is a Satz statement: it is written at the top level of a file, where it keeps a resource out of every export. Directly inside `google_folder { … }` it is read as a folder named `private`. Move it to the top level of the file; one that really is called `private` is written quoted, `"private" { … }`
```

**The edit:** write the label quoted, `"private" { … }`. The plan does not move: the address
is the same.

### v0.86.0

**A `${{interface.<export>}}` inside an `hcl { }` block is refused.** The passthrough is
appended to `main.tf` after emission and nothing in it is replaced, so the reference
reached Terraform as `${interface.x}`, an unknown object at plan. Find it: `grep -n
'interface\.' satz/*.satz` and look for hits inside `hcl {` blocks.

```
error    interface-use  satz/payments.satz:40
    the `hcl` block writes `${interface.<export>}` — a central estate's value is replaced in resource bodies only, and the block is emitted verbatim. Write what needs the value as a resource, where satz replaces it
```

**The edit:** move what reads the value out of the block into a resource of the estate,
where `${{interface.<export>}}` is replaced. The plan does not move: the block never
planned.

### v0.85.0

**The interfaces move from `hcl/interfaces/<name>/` to `interfaces/`, beside `hcl/`.**
`satz transpile` writes `interfaces_dir` (default `interfaces`, a new `config.toml` key)
whole: `interfaces/common/<name>/` holds `core` and every common interface, and
`interfaces/<project>/<name>/` holds one project's own interface and the whole library.
Each interface is `README.md`, `hcl/` — the module that stood in `hcl/interfaces/<name>/`,
the same files — and `satz/interface.satz`. An interface a pack declares is common, so it
is under `common/` and in every project's folder. `hcl/interfaces/` is removed on the
next transpile. The estate's own plan does not move, and neither does a project's.

A project written in HCL that sources the old path finds no module. Find it: `grep -rn
'hcl/interfaces/' <project>/*.tf`. **The edit:** point `source` at the interface's
`hcl/` in the new place — `../estate/hcl/interfaces/payments` becomes
`../estate/interfaces/payments/payments/hcl`, a common interface or `core`
`../estate/interfaces/common/<name>/hcl`; a `git::…//hcl/interfaces/payments?ref=…` URL
becomes `git::…//interfaces/payments/payments/hcl?ref=…`. Or take the folder
`interfaces/<project>/` whole into the project's repository and source
`./<folder>/<name>/hcl`. `satz check-consumer` reads a module as an interface when its
`source` ends in `<interface>/hcl`.

**`interface "common" { … }` is refused**: `interfaces/common/` holds the library, so no
interface takes its name. Find it: `grep -n 'interface "common"' <estate>.satz` and the
packs it uses. **The edit:** rename the interface, and every `use interface "common"`
that names it; the root outputs `common__<export>` take the new name.

**A common interface that uses one project's interface is refused**, naming both: every
project's folder carries a common interface, and with it the used one's values. An
interface declared in a pack is common. Find it: an `interface` block in a pack, or one
marked `common`, whose `use interface` names an interface only the estate declares.
**The edit:** mark the used interface `common`, or move the `use interface` line into the
project's own interface.

An interface change that breaks the projects reading it is a breaking change of the pack
or the estate that makes it, with its own entry here.

### v0.84.0

**`use_interface_notice` is refused; the change notice is the choice `interface_notice`.**
The map (`presets/estate-map.satz`) asks how the projects hear of a changed export as
`question oneof interface_notice`, whose Pub/Sub option is `interface_notice_pubsub`.
The old boolean is refused wherever it stands — bound in `params {}`, `true` or `false`,
or named by the `use … when` line. Find it: `grep -n use_interface_notice <estate>.satz`.

```
error    front-end  satz/acme.satz:32
    param `use_interface_notice` was renamed to `interface_notice_pubsub` — the change notice is a choice of delivery forms, `question oneof interface_notice`, and Pub/Sub is its option; the `use "presets/interface-notice.satz" when …` line names the new param too. Rename it here; a param no pack reads is not an error, so leaving it would silently take the new param's default instead.
```

**The edit:** rename the param in both places — `interface_notice_pubsub = true` (or
`false`) in `params {}`, and `use "presets/interface-notice.satz" when
interface_notice_pubsub`. An estate that turns the notice off may bind
`interface_notice_pubsub = false` or answer the choice `none` in `satz interview`. The
pack is the same pack; the plan does not move.

**An estate that uses `presets/estate-core.satz` must answer `workload_folder_name`.**
It is the folder where the customer's and the projects' folders live; `""` is the
organisation itself. `bootstrap` and `transpile --apply` refuse while a question is
unanswered. Find it: an estate with `use "presets/estate-core.satz"` and no
`workload_folder_name =` in its `params {}`.

```
error: bootstrap refused: 1 question(s) unanswered — workload_folder_name. Every question must be answered before the estate touches an organisation. `satz questions satz/acme.satz --unanswered --format text --out -` lists them with their defaults; write the answer (or the default) into the estate's params.
```

**The edit:** one line in `params {}` — `workload_folder_name = ""` for the organisation,
which creates nothing and does not move the plan — or run `satz interview <estate>
--accept-defaults`, which binds that line and writes the organisation's export. For a
folder, bind its display name and add the section `satz init` writes
([estate-core.satz](README.md#estate-coresatz)): the `google_folder { workload_folder { … } }` block
and its export; a folder the customer already has is imported with `satz adopt <estate>
--execute --import` before the apply. To publish the organisation as `workload_folder`,
add the one line `export "workload_folder" = "organizations/{customer_organization_id}"`,
which adds one output and moves no resource.

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
apply refused: 1 question(s) unanswered — <question>. Every question must be answered before the estate touches an organisation. `satz questions <estate> --unanswered --format text --out -` lists them with their defaults; write the answer (or the default) into the estate's params.
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
| `project_onboarding` | 1.1 | 2026-09-30 | the request point `projects` carries patterns: an entry whose `name` is not lowercase letters, digits and `-` starting with a letter, or whose `owner_group` is not an address `<name>@<domain>`, is refused at compile, naming the entry and the value |
| `project_onboarding` | 1.0 | 2026-09-30 | first version: one entry of the request point `projects`, `{ name owner_group }`, per project — its Google project under `project_onboarding_folder`, its IaC service account and state bucket, the grants, and `interface "<name>"` written per entry with `project_id`, `project_number`, `iac_account` and `state_bucket`; what `satz add-project` wrote as a section |
| `estate_map` | 2.7 | 2026-09-30 | offers `project-onboarding` on `use_project_onboarding`, off by default, with its question; nothing already on changes |
| `shared_network` | 1.0 | 2026-09-26 | first version: a shared VPC in a host project, a network firewall policy, and two request points — `shared_vpc_subnets` and `shared_firewall_rules` — whose entries become one subnet and one policy rule each, every subnet with flow logs; the common interface `network` publishes the host project, the network and every subnet |
| `estate_map` | 2.6 | 2026-09-26 | offers `shared-network` on `use_shared_network`, off by default, with its question; nothing already on changes |
| `interface_notice` | 1.2 | 2026-09-26 | the header and the question's text say project where they said team, and `interfaces/` where they said `hcl/interfaces/`; the resources are unchanged |
| `estate_map` | 2.5 | 2026-09-26 | the `interface_notice` question and the notice's `offers` phase say project where they said team, and `interfaces/` where they said `hcl/interfaces/`; nothing it offers changes |
| `estate_core` | 2.4 | 2026-09-26 | the header, the `workload_folder_name` question and the section comments say project where they said team, and name the interfaces under `interfaces/`; params and exports are unchanged |
| `interface_notice` | 1.1 | 2026-09-25 | gated on `interface_notice_pubsub`, the Pub/Sub option of the map's choice `interface_notice`; the header says the choice is where a further delivery form joins. The resources are unchanged |
| `estate_map` | 2.4 | 2026-09-25 | the change notice is `question oneof interface_notice`, not required, with the option `interface_notice_pubsub` (default `false`) in place of the boolean `use_interface_notice`, which is refused by name; `offers "presets/interface-notice.satz"` is gated on the option |
| `estate_core` | 2.3 | 2026-09-25 | `workload_folder_name`, the folder where the customer's and the teams' folders live, default `""` — the organisation, for which nothing is created — with its question, whose `empty` says so, so `""` is an answer. The estate publishes it as `workload_folder` in the section `satz init` or an interview writes. An estate that uses the pack answers the question before `bootstrap` or an apply |
| `interface_notice` | 1.0 | 2026-09-25 | first version: tells the teams whose HCL reads the estate's interface when an exported value changes. A bucket, a Pub/Sub topic, the grant that lets Cloud Storage's service agent publish to it, and a storage notification in the infrastructure project; the object `interface.json` holds the exported values, rewritten only when one changes, so each apply that changes an export publishes one message. Exports `interface_topic` and `interface_object` |
| `estate_map` | 2.3 | 2026-09-25 | offers `interface-notice` on `use_interface_notice`, off by default, with the question that asks for it |
| `estate_core` | 2.2 | 2026-09-25 | the core exports: `organization_id`, `customer_domain`, `customer_shortname`, `default_region`, `infra_project_id` and `iac_service_account`, each a core export — an output of the root module and of every module under `hcl/interfaces/` — all known at compile time. An estate that uses the pack gains `outputs.tf` and `hcl/interfaces/`; its resources do not change |
| `estate_map` | 2.2 | 2026-09-22 | the S1 model is offered as one entry, `security-group-models/s1-security-groups.satz`, at the top level: the two `by_hand` entries for `s1-group-definitions.satz` and `s1-group-permissions.satz` are gone with the packs, and the billing grants require one of the two models rather than one of three files |
| `billing_export` | 1.0 | 2026-09-22 | Cloud Billing usage and cost data exported to BigQuery: a project of its own, the BigQuery API on it, the dataset, and Google's export account's dataEditor on it — with that account contributed to `allowed_policy_member_subjects`, and a notice for the console step Cloud Billing has no API for |
| `estate_map` | 2.1 | 2026-09-22 | offers `billing-export` on `use_billing_export`, off by default |
| `integrations.microsoft_defender_for_cloud` | 0.6 | 2026-09-30 | contributes nothing to `allowed_policy_member_subjects`: the agentless disk-scanning account it contributed is granted nothing by this pack — the grant belongs to the agentless-scanning plan, which the library does not ship. The header and the plan note say the per-plan audiences, service accounts and role sets are Microsoft's constants, identical in generated scripts for different tenants |
| `integrations.microsoft_defender_for_cloud` | 0.5 | 2026-09-22 | contributes the agentless disk-scanning account to `allowed_policy_member_subjects` instead of naming it as a manual prerequisite in the header |
| `estate_core` | 2.1 | 2026-09-21 | `compliance_frameworks`, the catalogs this customer is HELD TO — a contract, an auditor, a regulator — as a list of catalog ids, with the question that asks for them. What an estate CLAIMS comes from its packs and is a different fact: an estate can claim CIS controls while its customer is audited against ISO 27001. The default is `["cis-gcp-5.0"]`; the values are the ids of the catalogs in `presets/catalogs/` (`cis-gcp-4.0`, `cis-gcp-5.0`, `iso27001-2022`) and a value that names no catalog is refused by the compile, with the list. `satz report-compliance <estate>` reports one section per framework named here, `satz prowler` scans for them beside the frameworks the packs claim, and a pack reads the param like any other. An estate that binds nothing keeps working: `report-compliance <framework> <estate>` is unchanged |
| `monitoring.organization_audit_logsink` | 1.7 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `monitoring.organization_audit_logsink` | 1.6 | 2026-09-21 | a question for `logsink_project_folder`: the folder the audit-archive project is created in. The param is now the only thing that decides — a `use` line no longer stands in a folder's body — so the interview asks for it. Answering it empty creates the project under the organisation; an estate `satz init` wrote answers `"google_folder.infra_folder.name"`, which init binds itself. Nothing emitted changes for an estate that already binds the param |
| `integrations.microsoft_defender_for_cloud` | 0.4 | 2026-09-21 | a question for `mdc_mgmt_project_folder`: the folder the Defender management project is created in. The param is now the only thing that decides — a `use` line no longer stands in a folder's body — so the interview asks for it. Answering it empty creates the project under the organisation, which is where every estate that binds nothing has it today |
| `monitoring.organization_audit_logsink` | 1.5 | 2026-09-21 | `logsink_project_folder`: the folder the audit-archive project is created in, said by the estate instead of read from the node the `use` line stands in. The default is empty, which says nothing — the enclosing node decides, exactly as before — so no estate's plan moves. An estate whose `use "presets/monitoring/organization-audit-logsink.satz"` stands inside a folder's body writes `logsink_project_folder = "google_folder.<label>.name"` for that folder (`satz init` writes the line into `google_folder.infra_folder`, so `"google_folder.infra_folder.name"`) and may then move the `use` line to the top level: the emitted HCL is byte-identical either way. A folder that already exists rather than being declared here is named by its id, `"123456789012"` |
| `integrations.microsoft_defender_for_cloud` | 0.3 | 2026-09-21 | `mdc_mgmt_project_folder`: the folder the Defender management project is created in, said by the estate instead of read from the node the `use` line stands in. The default is empty, which says nothing — the enclosing node decides, exactly as before — so no estate's plan moves. An estate whose `use "presets/integrations/microsoft-defender-for-cloud.satz"` stands inside a folder's body writes `mdc_mgmt_project_folder = "google_folder.<label>.name"` for that folder and may then move the `use` line to the top level: the emitted HCL is byte-identical either way. A folder that already exists rather than being declared here is named by its id, `"123456789012"` |
| `CIS_GCP_Foundation_4_0` | 2.19 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `CIS_GCP_Foundation_4_0` | 2.18 | 2026-09-30 | the header comment on the extensions says what `compute.requireVpcFlowLogs` does: it refuses a subnet without flow logs, so every subnet the estate or another pack declares carries a `log_config`. Nothing emitted changes |
| `CIS_GCP_Foundation_4_0` | 2.17 | 2026-09-22 | the `cis_access_approval` question states Access Transparency (CIS 4.0 §2.14 / 5.0 §2.15) as the manual prerequisite it is: an organisation administrator with `roles/axt.admin` switches it on in the Cloud console before the apply, there is no gcloud command, API or provider resource for it, and it needs a Standard, Enhanced or Premium support plan. The catalogs now carry that control as organizational, so `report-compliance` lists it. Nothing emitted changes |
| `CIS_GCP_Foundation_4_0` | 2.16 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.block_project_ssh_keys` | 1.3 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.block_project_ssh_keys` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.shielded_vm` | 1.3 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.shielded_vm` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.dns_logging` | 1.3 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.dns_logging` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.confidential_computing` | 1.3 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.confidential_computing` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cloud_sql` | 1.4 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.cloud_sql` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cmek` | 1.4 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.cmek` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.api_key_services` | 1.4 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.api_key_services` | 1.3 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.bucket_retention` | 1.5 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.bucket_retention` | 1.4 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.cloud_sql_iam_and_deletion_protection` | 1.3 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.cloud_sql_iam_and_deletion_protection` | 1.2 | 2026-09-20 | the notice's `before = apply` becomes `severity = error`: the same refusal, stated once by the pack instead of inside the two commands that read it — every command that writes to the organisation refuses while it is open. Nothing emitted changes, and the param that acknowledges it is unchanged |
| `cis_extensions.block_project_ssh_keys_dry_run` | 1.3 | 2026-09-30 | GENERATED from `block-project-ssh-keys.satz` 1.3: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
| `cis_extensions.block_project_ssh_keys_dry_run` | 1.2 | 2026-09-20 | GENERATED from `block-project-ssh-keys.satz` 1.2: the same notice, now `severity = error`, on its own param |
| `cis_extensions.confidential_computing_dry_run` | 1.3 | 2026-09-30 | GENERATED from `confidential-computing.satz` 1.3: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
| `cis_extensions.confidential_computing_dry_run` | 1.2 | 2026-09-20 | GENERATED from `confidential-computing.satz` 1.2: the same notice, now `severity = error`, on its own param |
| `cis_extensions.cloud_sql_dry_run` | 1.4 | 2026-09-30 | GENERATED from `cloud-sql.satz` 1.4: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
| `cis_extensions.cloud_sql_dry_run` | 1.3 | 2026-09-20 | GENERATED from `cloud-sql.satz` 1.3: the same notice, now `severity = error`, on its own param |
| `cis_extensions.api_key_services_dry_run` | 1.4 | 2026-09-30 | GENERATED from `api-key-services.satz` 1.4: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
| `cis_extensions.api_key_services_dry_run` | 1.3 | 2026-09-20 | GENERATED from `api-key-services.satz` 1.3: the same notice, now `severity = error`, on its own param |
| `cis_extensions.bucket_retention_dry_run` | 1.5 | 2026-09-30 | GENERATED from `bucket-retention.satz` 1.5: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
| `cis_extensions.bucket_retention_dry_run` | 1.4 | 2026-09-20 | GENERATED from `bucket-retention.satz` 1.4: the same notice, now `severity = error`, on its own param |
| `cis_extensions.cloud_sql_iam_and_deletion_protection_dry_run` | 1.3 | 2026-09-30 | GENERATED from `cloud-sql-iam-and-deletion-protection.satz` 1.3: the version follows the source pack, whose claims gained their measure without satz; a twin carries no claim, so nothing here changes but the version |
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
| `integrations.microsoft_sentinel` | 1.1 | 2026-09-12 | follows the rename: the Sentinel project defaults to `logsink_project_id` |
| `integrations.microsoft_sentinel` | 1.0 | 2026-09-12 | first version: Sentinel's GCP federation — pool, the provider trusting Microsoft's commercial tenant with the `api://` audience, the connector's service account and `roles/iam.workloadIdentityUser` for the pool's principal set. Transcribed from Microsoft's own Terraform against the pinned provider: upstream pins google 3.73.0 and uses authoritative `google_project_iam_binding`, which removes grants an estate made |
| `integrations.microsoft_sentinel_network_logs` | 1.0 | 2026-09-12 | first version: the four network streams — VPC flow logs, firewall rules logging, DNS queries, Cloud NAT — each with its own organisation sink, topic, subscription, publisher grant for the sink's writer identity and subscriber grant for the connector. On by default with Sentinel: each stream is empty until the feature is enabled per subnet, rule, policy or gateway, and routing costs nothing, so switching them off saves nothing and risks the day somebody enables flow logs. Filters select one stream each (`log_id` where Google publishes the log name, the documented `dns_query` resource type for DNS) rather than Microsoft's mix of stream plus the same service's audit records, which the audit fragment already carries. Grants are non-authoritative: upstream's `google_project_iam_binding` would have had the second stream applied remove the first's publisher grant, stopping delivery silently |
| `integrations.microsoft_sentinel_auditlogs` | 1.0 | 2026-09-12 | first version: the first log source — an organisation sink with `include_children` for the four audit streams, its topic, the subscription Sentinel pulls from, `roles/pubsub.publisher` for the sink's writer identity and `roles/pubsub.subscriber` for the connector on that one subscription. Tighter than upstream, which grants a project-level custom role over every subscription in the project. The filter is asked: Data Access logs are most of the volume and Sentinel bills by the gigabyte |
| `monitoring.organization_audit_logsink` | 1.4 | 2026-09-12 | `logsink_project_name` becomes **`logsink_project_id`**, because that is what it is — it feeds `project_id`, and a project id is immutable while a name is not. The project's display name is its own optional param, `logsink_project_display_name`, defaulting to the id exactly as Google does, so nothing changes in the emitted HCL. An estate still binding the old name is REFUSED by name with the new one: nothing refuses a param no pack reads, so leaving it would have silently taken this pack's default project instead — a second logging project and an orphaned archive |
| `monitoring.organization_cis_log_alerts_central` | 1.7 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `monitoring.organization_cis_log_alerts_central` | 1.6 | 2026-09-12 | follows the rename: the alert project defaults to `logsink_project_id` |
| `cis_extensions.internet_ssh_rdp` | 1.2 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
| `cis_extensions.internet_ssh_rdp` | 1.1 | 2026-09-12 | ON by default (CIS pack 2.10), with the corrections a default-on pack needs. The pass list gains the three RFC1918 blocks beside the IAP range, because the deny matches `0.0.0.0/0` — every address, private ones included — and a hierarchical policy is read before the VPC rules: with IAP alone, SSH between two instances in one subnet was denied. IPv6 gets its own pass rule (IAP's `2600:2d00:1:7::/64` and `fc00::/7`), since a rule's sources may not mix families. Every rule now carries the control's whole protocol set — SSH on TCP 22 and SCTP 22, RDP on TCP 3389 and UDP 3389 — a TCP-only deny left UDP 3389 open. And the two DENY rules log: Google forbids logging on `goto_next`, so an accepted IAP session leaves no firewall record and only refusals do |
| `cis_extensions.dns_logging` | 1.0 | 2026-09-12 | first version: CIS 5.0 §2.13, the half an org policy can carry — a custom constraint on `dns.googleapis.com/Policy` requiring `enableLogging`. ON by default. `contributes`, not `implements`: no org policy can require that a network HAS a DNS policy, only that a policy which exists logs, so the missing half is named as a duty and verified live |
| `CIS_GCP_Foundation_4_0` | 2.10 | 2026-09-12 | `cis_block_internet_ssh_rdp` defaults to TRUE, the second flag to do so. An estate taking this version emits an organisation firewall policy it did not have: ports 22 and 3389 are denied from public addresses, the private ranges and IAP pass to the VPC rules, and the denies log. Answering no is a deviation whose reason the compliance report carries (ADR 0012) |
| `CIS_GCP_Foundation_4_0` | 2.9 | 2026-09-12 | one new flag, and the first that defaults to TRUE: `cis_dns_logging`, for the new `cis-extensions/dns-logging.satz`. It asks what the control covers and what it breaks; answering no is a deviation whose reason the compliance report carries |
| `s2_security_groups` | 1.2 | 2026-09-11 | the security-admins group's description says what its roles do — organisation policies, folder IAM, Security Command Center, logging and monitoring, read access — instead of the Security Admin role and organisation, folder and project IAM admin, which the group never held. An in-place description update on the group; no role changes |
| `s1_security_groups` | 1.2 | 2026-09-11 | the security-admins group's description says what its roles do — organisation policies, folder IAM, Security Command Center, logging and monitoring, read access — instead of the Security Admin role and organisation, folder and project IAM admin, which the group never held. An in-place description update on the group; no role changes |
| `CIS_GCP_Foundation_4_0` | 2.8 | 2026-09-11 | three more opt-in flags with their questions — `cis_access_approval`, `cis_block_internet_ssh_rdp`, `cis_cloud_sql_iam_and_deletion_protection` — for the three new `cis-extensions/` fragments. Nothing emitted changes; an estate using the pack has three more questions, all defaulting to off |
| `cis_extensions.access_approval` | 1.1 | 2026-09-30 | every claim states its measure without satz: `gcloud` (the commands that meet the control the way the claim's resources do), `gcloud_check` (the commands that show whether it is met) and `risk` (what goes wrong without it); `require` and `report-compliance` carry them in each control's `measures`. Nothing emitted changes |
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
