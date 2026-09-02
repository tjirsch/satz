# satz

**Infrastructure with a constitution — proven live.** Satz compiles an estate — written in a language whose resource types and attributes are the Terraform provider's — to OpenTofu/Terraform HCL, and proves the controls it declares against the live Google Cloud organisation. It succeeds the tool that began as `cfg2hcl`.
Builtin functions to bootstrap a Google Cloud Organization and do state import, migration and discovery of an existing GCP Organization from state or live infrastructure.

> **📖 Documentation: <https://tjirsch.github.io/satz/>** — this README, the
> [language reference](https://tjirsch.github.io/satz/docs/satz-language.html) and the
> [preset pack pages](https://tjirsch.github.io/satz/presets/docs/index.html), searchable,
> rebuilt on every release. Also: `satz open-readme`, or `satz <command> --html-help`.

## Folder Structure

The project is structured such that `satz` (the tool) is kept separate from customer-specific definitions. Each customer repository follows this layout:

```text
customer-repo/ (e.g. project-root/)
├── config.toml          # Tool configuration for this customer
├── schemas/             # JSON schemas for used cloud providers
├── presets/             # Preset library (get-presets) — everything available for copying
├── yaml/                # Infrastructure definitions — only files actually used/adapted
└── hcl/                 # Generated .tf files
```

### Which config? (`--config` vs the positional argument)

Two different files are involved, and both are easy to mistake for each other.

| | `--config <FILE>` | positional `<ESTATE>` |
|---|---|---|
| **Is** | the **project** config — TOML | the **estate** — Satz |
| **Example** | `config.toml`, `../config.toml` | `C0example.satz` |
| **Holds** | `yaml_dir`, `hcl_dir`, `schema_dir`, `include_dirs`, providers | params, `terraform` block, folders, projects, resources |
| **Default** | `./config.toml` (error if missing) | none — required |
| **Path resolves against** | your current directory | **`yaml_dir`** |

`config.toml` is the anchor for everything else: `yaml_dir`, `hcl_dir`, `schema_dir` and `include_dirs` resolve relative to **the config file's own directory**, not your current one. So you can work from anywhere as long as you point `--config` at it:

```bash
# from the project root (config.toml is in the current directory)
satz transpile C0example.satz
satz bootstrap  C0example.satz

# from a subdirectory such as hcl/ — yaml_dir still resolves from config.toml's directory
satz transpile C0example.satz --config ../config.toml
```

**Rule of thumb: `--config` takes a path; the positional takes a bare filename.**

Common mistakes:

| Mistake | What happens |
|---|---|
| `satz bootstrap yaml/C01.satz` | resolves to `yaml/yaml/C01.satz` → not found |
| `satz bootstrap C01` | no extension is appended → looks for `yaml/C01` |
| `satz bootstrap C01.satz --config yaml/C01.satz` | the estate is parsed as TOML → `key with no value, expected =` |

### Global Options

These options can be placed anywhere in the command (e.g., before or after subcommands):

- `--config <FILE>`: Path to the **project** config file (`config.toml`, TOML — not the estate file). Mandatory for most commands if `config.toml` is not in the current directory. Every relative path inside it resolves from its own directory.
- `--validation <LEVEL>`: Validation level for mandatory parameters (`warn`, `error`, `none`). Default from project config or `warn`.
- `--html-help`: open the documentation site in the browser at the invoked command's section (`satz transpile --html-help`); alone (`satz --html-help`) the front page. Commands without a section of their own open the command table.
- `--verbose`: Enable verbose output. When invoked without a subcommand (e.g. `satz --verbose`), prints full recursive help listing all subcommands and their options.

### User settings (~/.config/satz/satz.toml)

User-level **parameters** (e.g. when to check for updates) live in **`~/.config/satz/satz.toml`**. This file is **created on first run** with default values (e.g. `self_update_frequency = "always"`). If the file is missing on load, it is created with defaults.

| Option | Default | Description |
|--------|---------|-------------|
| `self_update_frequency` | `"always"` | When to check for updates on normal runs: `never`, `always`, or `daily` (at most once per 24 hours). The check is check-only (no install, no README). |

**Project config** (paths, providers, etc.) stays in **`config.toml`** per project; see [Configuration](#configuration) below.

Example (optional; the file is created automatically when needed):

```toml
self_update_frequency = "daily"
```

## Installation

### Using cargo-dist Installer (Recommended)

Install the latest release using the cargo-dist installer:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tjirsch/satz/releases/latest/download/satz-installer.sh | sh
```

This will install `satz` to `~/.local/bin` and automatically add it to your PATH if needed.

> **Note:** The installer will:
> - Install the binary to `~/.local/bin`
> - Check if this directory is on your PATH
> - If not, add it to your shell profile (e.g., `.bashrc`, `.zshrc`)
> - Provide instructions to refresh your shell
>
> If you prefer a different location, you can override it:
> ```bash
> curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tjirsch/satz/releases/latest/download/satz-installer.sh | CARGO_DIST_FORCE_INSTALL_DIR=/your/custom/path sh
> ```

> **Note:** The installer script is generated automatically when releases are created. If you get a 404 error, it means no releases have been published yet. Use the "From Source" method below instead.

### From Source

Install directly with cargo:
```bash
cargo install --path .
```
This builds the release binary and installs it to `~/.cargo/bin` (no sudo required); ensure that directory is on your `PATH`.

## CLI Usage

All commands accept the [global options](#global-options) (`--config`, `--validation`, `--verbose`). Commands and their options:

| Command | Options / Arguments |
|---------|---------------------|
| `init` | `--defaults`, `--providers`, `--tf-tool`, `--customer-id`, `--customer-shortname`, `--billing-account-infra`, `--customer-organization-id`, `--customer-domain`, `--iac-user`, `--default-region`, `--infra-project-name`, `--infra-bucket-name` |
| `bootstrap <CONFIG_FILE>` | `--dry-run` |
| `export-organizational-policies <CONFIG_FILE>` | `--customer-organization-id`, `--output` |
| `diff-organizational-policies <CONFIG_FILE>` | `--customer-organization-id`, `--report`, `--format` (`console`\|`markdown`\|`json`), `-r/--recursive` (every folder and project below) |
| `report-organizational-policies <CONFIG_FILE>` | `--customer-organization-id`, `--scope` (`active`\|`inactive`\|`full`), `--format` (`markdown`\|`json`\|`pdf`), `--report`, `-r/--recursive` |
| `transpile <INPUT>` | `--output`, `--schema-dir`, `--print-variables`, `--plan` / `--apply` (then run the tool in `hcl_dir`), `--scan` (then Checkov) |
| `triage <FRAMEWORK> <INPUT>` | `--prowler <file>` (required), `--format` (`markdown`\|`json`), `--report` — every Prowler FAIL sorted into who-fixes-it buckets against the estate's claims |
| `doc-packs` | `--out <DIR>` (default `<presets_dir>/docs`), `--check` — one Markdown page per pristine pack, derived from the pack file; `--check` fails when the pages are behind |
| `scan [<INPUT>]` | Checkov over `hcl_dir`; with the estate, each finding is pointed at the Satz block that declared the resource; failed checks exit 1 |
| `scan-plan <plan_json>` | `--output` (default: `mapping.yaml`) |
| `generate-migration <mapping>` | `--output` (default: `migrate.sh`) |
| `update-schema` | `--providers`, `--version`, `--tf-tool` |
| `import [SOURCE]` | `--from` (`state`\|`org`\|`yaml`\|`hcl`), `--only <types>`, `--output` (default: `discovered.satz`), `--import-config`, `--into <estate>` (live: only the delta); yaml shape: `--kind`, `--gate`, `--fork`; hcl shape: `--wrap-all` |
| `map-types` | `--only <types>`, `--import-config` — derive the API→Terraform field map per type into `presets/type-map.yaml` |
| `migrate <INPUT>` | `--mode` |
| `get-presets` | `--force` — overwrite presets the estate uses too; `--pristine-dir` |
| `require <FRAMEWORK> <INPUT>` | *(catalog id, e.g. `cis-gcp-4.0`)* |
| `report-compliance <FRAMEWORK> <INPUT>` | `--format` (`markdown`\|`json`\|`pdf`), `--report`, `--prowler`, `--checkov`, `--no-live`, `--fail-on <statuses>` |
| `merge-presets` | `--pristine-dir`, `--estate`, `--report-only`, `--adopt <stem\|all>` — reconciling update; `--adopt` upgrades in place instead of forking |
| `check-presets <INPUT>` | `--pristine-dir` |
| `adopt <INPUT>` | `--execute`, `--import`, `--activate`, `--only <types>` — dry run by default; `adopt-org-policies <INPUT> [--dry-run]` is an alias |
| `self-update` | `--no-open-readme`, `--check-only`, `--skip-checksum` |
| `open-readme` | *(none)* — opens the documentation site |
| `whoami` | `--offline` — print which identity, credential type and quota project the ADC resolves to |
| `completion [SHELL]` | `--install` |

Details for each command are below.

### Initialize Project (`init`)
Bootstrap a new project directory with default folders, config, .gitignore, and schemas.

```bash
satz init \
  --customer-id C01234567 \
  --customer-shortname example-org \
  --billing-account-infra A12345-B67890-C12345 \
  --customer-domain example.com \
  --customer-organization-id "123456789012"
```

**Parameters:**
- `--defaults <LIST>`: Default provider sets to include (e.g., `google`).
- `--providers <LIST>`: Explicit providers to include (e.g., `aws`, `azure`, `google`).
- `--tf-tool <TOOL>`: Terraform binary to use (default: `tofu`).
- `--customer-id <ID>`: Workspace Organization ID (e.g., `C01234567`).
- `--customer-shortname <NAME>`: Short slug for the customer.
- `--billing-account-infra <ID>`: Billing account ID.
- `--customer-organization-id <ID>`: GCP Organization ID.
- `--customer-domain <DOMAIN>`: Primary domain name.
- `--iac-user <EMAIL>`: Initial Admin User (default: `first.admin@<customer-domain>`).
- `--default-region <REGION>`: Default GCP region (default: `europe-west3`).
- `--infra-project-name <ID>`: Override for the infrastructure project ID.
- `--infra-bucket-name <NAME>`: Override for the state bucket name.

**Under the Hood:**
- Creates the standardized directory structure: `yaml/`, `hcl/`, `schemas/`.
- Generates a default `config.toml` and `.gitignore`.
- If customer details are provided, generates the Day-0 estate `yaml/<customer-id>.satz` (params, providers, the IaC group and service account, the management folder/project/state bucket — the labels `bootstrap` imports by name).
- Fetches the latest provider schemas for the configured providers.

### Day 0 Bootstrap (`bootstrap`)
The `bootstrap` command automates the entire onboarding process for a new customer organization.

```bash
satz bootstrap <CONFIG_FILE> [options]
```

**Parameters:**
- `<ESTATE>`: The estate file (e.g. `C0example.satz`). Relative paths are looked up inside `yaml_dir`, so pass the bare filename — **not** `yaml/C0example.satz`, which would resolve to `yaml/yaml/C0example.satz`. This is not the tool config; that is `--config`.
- `--dry-run`: Simulation mode; does not create resources.
**Tip:** Use `--dry-run` to see what resources would be created without making changes.

**Tip:** For a declarative approach, set `deployment_mode = "boot"` in the estate's `params` block and run `transpile`.

**Under the Hood:**
1.  **Authentication**: Uses Application Default Credentials (ADC).
2.  **Infrastructure Folder**: Lists every folder under the parent (all pages) and reuses the one whose display name matches — exactly one; two folders with that name is an error, not a guess — or creates it (requires `Folder Admin`).
3.  **Project Shell**: Creates the management project (project-id defaults to `shortname-iac-infra`) inside the folder, or reuses an existing one, and prints its **project number**.
4.  **Billing Link**: Links the project to the specified Billing Account.
5.  **Enable APIs**: Enables the foundation APIs (Service Usage, Cloud Resource Manager, IAM, IAM Credentials, Storage, Cloud Billing, Cloud Identity, Cloud Asset, Logging, Org Policy, Essential Contacts).
6.  **State Bucket**: Creates the GCS bucket for Terraform state (with versioning, uniform access).
7.  **Automated Setup**:
    - **Transpile**: Compiles the estate to HCL.
    - **Init**: Runs `tofu init` to download plugins.
    - **Import**: Automatically imports the created Folder, Project, and Bucket into the local state.

### Transpile (`transpile`)
Compile your estate to production-ready HCL. Input is a `.satz` estate; a legacy `.yaml` estate is refused with a pointer to `satz import`.

```bash
satz transpile <INPUT> [options]
```

**Parameters:**
- `<INPUT>`: Name of the estate file. This is resolved relative to the `yaml_dir` defined in your config.
- `--output, -o <FILE>`: Optional output subdirectory or absolute path. By default, output goes to `hcl_dir`.
- `--schema-dir, -s <DIR>`: Override the schema directory.
- `--print-variables`: After transpilation, print the resolved variable table (`terraform.tfvars`) to stdout. Useful for debugging variable resolution across multiple include files.
- `--scan`: after transpiling, run Checkov (terraform framework) over `hcl_dir` — `checkov` on PATH, else `uvx checkov` — and print every failed check under the resource it hit, with the Satz file and line that declared it (from the emission manifest) and Checkov's guideline link. Failed checks exit 1, so it gates like a test. `satz scan [<estate>]` does the same without transpiling first.
- `--plan` / `--apply`: after transpiling, run `<tf_tool> plan` / `apply` in `hcl_dir` — one command from estate to plan. The dir is initialised first when it has no `.terraform`. The same as `satz transpile … && satz plan`; `satz plan`, `satz apply` and `satz tf-init` remain for running the tool on its own (extra arguments pass through).

**Running from subdirectories:**
You can run the transpile command from any directory (e.g., from within the `hcl/` folder) by specifying the config path. Both styles are supported:
```bash
# Global option before subcommand
satz --config ../config.toml transpile my-infra.satz

# Global option after subcommand (Recommended)
satz transpile my-infra.satz --config ../config.toml
```
This will correctly look for `../yaml/my-infra.satz` and update the files in the current directory.

**Satz estates — the fragment pipeline:**
A `.satz` input compiles through the stage-B fragment pipeline: every source
file (estate + each `use`d pack) becomes its own fragment, the composition
algebra folds them (grant union, deep-equal idempotence, conflicts reported
with every contributing origin), and HCL is emitted from the folded result,
with deterministic ordering (snapshot-gated by `tests/corpus/`).

**Subtractive overrides (`suppress`, satz estates only):**
An estate can remove something a used pack contributes — without forking:

```
use "presets/CIS-GCP-Foundation-4.0.satz" as google_org_policy_policy

// drop one pack resource; grant-edge form removes a single role
suppress google_org_policy_policy "iam-allowedPolicyMemberDomains"
suppress google_organization_iam_member "group:sec@example.com" role "roles/viewer"
```

Rules: interpolations (`{param}`) work in the name; a suppress that matches
nothing is a **hard error** (stale subtractive config must surface, never
silently deploy); a suppressed resource that was the witness of a compliance
claim shows up as broken in `require` — deliberately.

**Raw HCL passthrough (`hcl { … }`, satz only):**
The escape hatch for anything the resource model does not cover yet — Terraform
that composes and deploys, but that the compliance plane cannot see into. Think
Rust's `unsafe`: allowed, local, and marked.

```
hcl {
  resource "google_compute_router" "nat_router" {
    name    = "nat-${var.customer-id}"
    network = "default"
  }
}

// once reviewed, state why — the warning becomes a note
hcl trust "vendor snippet, reviewed 2026-08 by TJ" {
  output "router_name" { value = google_compute_router.nat_router.name }
}
```

Rules: the body is emitted **verbatim** at the end of `main.tf` under a
provenance header (source file and line), and is never interpolated — braces,
quotes, comments and heredocs pass through untouched. Reach params through the
Terraform variables satz already emits: the param `customer_id` becomes
`var.customer-id` (underscores render as dashes). Blocks may sit at the top
level of an estate **or a pack**, and are appended in visit order. Every block
**warns on each transpile**; `trust "<reason>"` downgrades that to a note
without changing what is emitted. Nothing inside an `hcl` block becomes an
entity, so it never participates in the fold, can never conflict — and can
never carry a claim.

**Under the Hood:**
- Parses the estate and every pack it `use`s into per-file fragments; params are declarations in one document-ordered namespace (the using file's binding wins over a pack's default), sorted by dependency.
- Folds the fragments by Terraform address (⊕): the same address with the same body collapses, with a different body the transpile aborts naming both files.
- Schema-typed: every resource key and block key is checked against `schemas/*.json` at parse time.
- Generates four files in the output directory:
    - `main.tf`: Resources.
    - `providers.tf`: Provider configurations and aliases.
    - `variables.tf`: Variable declarations.
    - `terraform.tfvars`: Variable values.
    - `imports.tf`: (Optional) OpenTofu `import` blocks for existing resources.

### Resource Imports

`satz` supports declarative resource imports using the OpenTofu/Terraform 1.5+ `import` block logic. This allows you to bring existing cloud resources under management without manually running CLI `import` commands.

#### Declarative Imports (via `import-id`)

To import an existing resource, add the `"import-id"` attribute to its definition in your estate:

```
google_org_policy_policy {
  "iam-managed-disableServiceAccountKeyCreation" {
    "import-id" = "organizations/123456789012/policies/iam.managed.disableServiceAccountKeyCreation"
    name   = "iam.managed.disableServiceAccountKeyCreation"
    parent = "organizations/{customer_organization_id}"
    spec { rules = [{ enforce = "TRUE" }] }
  }
}
```

**How it works:**
- **`"import-id" = "<ID>"`**: Provide the full GCP resource ID. Honoured on every emitted resource; where the resource is an *entry* — a role in an IAM grant list, a service in `project_service`, a member of a group — write the entry as an object and put the id there (`{ role = "roles/x" "import-id" = "…" }`, `{ service = "…" "import-id" = "…" }`, `{ id = "user:…" "import-id" = "…" }`). See the language reference §6.7.
- **`imports.tf` Generation**: The transpiler detects the `import-id` and generates a corresponding OpenTofu `import` block in `hcl/imports.tf`.
- **Automatic Lifecycle**: `imports.tf` is automatically deleted before each `transpile` run and only recreated if `import-id` tags are found.
- **Execution**: Running `tofu plan` (or `apply`) will show these resources as "to be imported".

#### Automatic Imports during Bootstrap

The `bootstrap` command automatically handles the import of core infrastructure resources (Folder, Project, and State Bucket) into your initial state so you don't have to manually link them.

> [!NOTE]
> Declarative imports require **OpenTofu** or **Terraform 1.5.0+**. For older versions, traditional CLI `tofu import` must be used.

### Organization Policy Alignment

Curated Organization Policy sets (e.g. `presets/CIS-GCP-Foundation-4.0.satz`) are normally
pulled into an estate with `use` and rendered as `google_org_policy_policy` resources
like any other. The wrinkle is GCP **managed** constraints (their name contains
`.managed.`, e.g. `iam.managed.disableServiceAccountKeyCreation`): depending on org state
they must be *activated*, then *imported as-is*, and only then *modified*.
`satz adopt --activate` (see [Adopting what already exists](#adopting-what-already-exists-adopt-brownfield))
does the first two; three read-only CLI commands (`export`, `diff`, `report`) round
out the workflow.

All Org Policy API calls authenticate via Application Default Credentials (same as
`bootstrap`) and send your quota project as `x-goog-user-project` (resolved from
`GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT` or the ADC file's `quota_project_id`).
Run `gcloud auth application-default login` and set a quota project first.

#### Export current state (`export-organizational-policies`)

Snapshot the live policies into a Satz pack — one quoted block per constraint, the
shape the shipped CIS packs use, `parent` written as
`"organizations/{customer_organization_id}"` so the file carries no customer number:

```bash
satz export-organizational-policies C0example.satz --customer-organization-id 123456789012
# -> yaml/<customer-id>-orgpolicies.satz
```

`use` it from the estate inside a `google_org_policy_policy { … }` block, diff it, or
run `satz adopt` to import what it describes.

#### Diff desired vs. live (`diff-organizational-policies`)

```bash
# Everything the estate declares (its own blocks and the packs it uses) against live:
satz diff-organizational-policies C0example.satz --format markdown --report diff.md
```

The desired set is read off the compiled estate — the same `google_org_policy_policy`
resources `transpile` emits. To diff one pack, write an estate that `use`s only that pack.

Each policy — a constraint **at a parent**: the same constraint declared on the
organization and again on a folder is two policies, each compared with the live
policy at its own parent — is classified: `MISSING (needs activation)` (managed),
`MISSING (creatable)`, `MATCHES`, `DIFFERS`, or `CURRENT-ONLY`. The diff is semantic — it normalizes
`enforce: "TRUE"` vs `true`, `allowed_values` ordering, and `parameters` JSON-string vs
object so it doesn't report false changes.

#### Report with explanatory text (`report-organizational-policies`)

```bash
satz report-organizational-policies C0example.satz --scope full --format markdown
```

`--scope`: `active` (set policies), `inactive` (available but unset), or `full` (both,
with constraint descriptions pulled from the Org Policy constraints API). `--format pdf`
converts the markdown via `pandoc` if it is on `PATH` (otherwise the markdown is kept).

### Hoisted scopes: where resource types may live

Some resource types have one intrinsic scope no matter where they are written:

| Type | Intrinsic scope | Emitted with |
|---|---|---|
| `cloud_identity_group` | Customer | `parent = customers/<customer-id>` |
| `organization_iam_member` | Organization | `org_id = <customer-organization-id>` |
| `google_billing_account_iam_member` | Billing account | `billing_account_id` — an explicit `billing_account_id:` entry in any fragment, else `*billing-account-infra` |

Declaring these inside a folder or project block is therefore **grouping for humans, not
placement**: during transpile they are collected from everywhere in the tree and emitted
exactly once at their real scope. That makes a fragment file *cohesive* — a project can
travel with its org-level companions in one file, included at one position:

```
// logging-project.satz — one file, one concern
pack logging_project version "1.0"

google_project {
  logging_prj {
    project_id = "{customer_shortname}-logging"
  }
}
google_organization_iam_member {
  "group:log-admins@{customer_domain}" = ["roles/logging.admin"]
}
google_cloud_identity_group {
  "log-admins" {
    display_name = "Log Admins"
  }
}
```

```
// the estate — the hierarchy decides where the project lands
google_folder {
  shared_services {
    display_name = "Shared Services"
    use "logging-project.satz"
  }
}
```

The project is created in `shared-services`; the group and the grant are hoisted to their
intrinsic scopes. Placement is always **by `use` position** — fragments never name their
parent folder, so the estate stays the single source of hierarchy truth.

Merge rules when several fragments declare the same thing:

- **IAM grants are additive.** Same member from two fragments → role lists union; a
  deep-equal (member, role, condition) entry is deduped to one resource. There is no
  conflict state.
- **Groups must agree.** The same group key with a deep-equal body is deduped (including
  the same fragment twice is idempotent); with a *different* body the transpile aborts
  before writing any file: `composition conflicts: google_cloud_identity_group.log-admins:
  2 disagreeing definitions`, naming both files and lines.
- Hoisted output is sorted, so moving a fragment between folders does not churn the
  generated HCL.

Project- and folder-scoped IAM types (`project_iam_member`, `folder_iam_member`) are **not**
hoisted — their position in the tree is their parent, as before.

Every other resource type is position-independent whenever its parent is explicit in the
source (`org_id`, `parent`, `billing_account`), so it needs no hoisting. What all types
get instead is the **duplicate-address guard**: a Terraform address may be emitted once.
Byte-identical duplicate definitions — the same "highlander" resource (org audit config,
sink, contact) included from several fragments — collapse to a single emission with a
printed note; the same address with *different* content aborts the transpile, naming the
address and the first differing line. Attribute-level merging is deliberately not
attempted — it would only hand the same conflict one recursion level down.

**Cross-file merging is the fold.** Two packs (or the estate and a pack) may declare
the same resource type at the same position — the fragments compose by address:
distinct labels union, the same label with an identical body collapses, the same label
with a different body aborts the transpile naming both files. This is how the
audit-logsink pack and the CIS central-monitoring pack each declare their own
`google_logging_organization_sink` and coexist. Merging steps into the labels,
deliberately not into attributes — attribute-level merging would only hand the same
conflict one recursion level down.

### Resource Lifecycle

Any resource may declare a `lifecycle` block, which is rendered as a top-level
[`lifecycle` meta-argument](https://developer.hashicorp.com/terraform/language/meta-arguments/lifecycle) block in the generated HCL:

```
google_cloud_identity_group {
  my_group {
    display_name = "My Group"
    initial_group_config = "EMPTY"
    lifecycle {
      ignore_changes = ["initial_group_config"]
      prevent_destroy = true
    }
  }
}
```

Generates:

Generates a `google_cloud_identity_group` with `group_key { id = "my-group@<domain>" }`,
`parent = "customers/<id>"`, the discussion-forum labels, `initial_group_config`, and a
`lifecycle` block carrying `ignore_changes = [initial_group_config]` merged with the one
declared (`prevent_destroy = true` here).

**Notes:**
- `ignore_changes` and `replace_triggered_by` entries are emitted as **bare** HCL identifiers/expressions (e.g. `initial_group_config`, `labels["env"]`), not quoted strings.
- Use the scalar form `ignore_changes: all` to ignore changes to every attribute (renders the bare keyword `all`).
- Boolean meta-arguments such as `create_before_destroy` and `prevent_destroy` are passed through as-is.

### Mode Switching & State Migration (`migrate`)
Seamlessly move your project between development (`local`) and production (`cloud`) modes.

```bash
satz migrate <INPUT> --mode <MODE>
```

**Parameters:**
- `<INPUT>`: Name of the estate file (`.satz`).
- `--mode, -m <MODE>`: Target mode (`local` or `cloud`).

**Under the Hood:**
- **Update the estate**: Rewrites the `deployment_mode` param in the `.satz` file (an estate without one is refused).
- **Regenerate**: Runs `transpile` to update the backend configuration (Local vs GCS) and provider authentication (ADC vs Impersonation).
- **Migrate State**: Executes `tofu init -migrate-state` to safely move your terraform state to the new backend.

### Creating an estate from what exists (`import`)

One verb, three input shapes — you pick by what you have. The result is a Satz
estate that compiles as-is: a local backend, `customer_organization_id`, every
resource carrying its `"import-id"`, keys normalised to provider type names.
Review it, `satz transpile`, then `tofu plan` — the plan is the check: no destroy,
no unexpected create.

```bash
satz import state.json                       # a state file (tofu show -json / *.tfstate)
tofu show -json | satz import -              # …or on stdin
satz import organizations/123456789012       # live, whole org (Cloud Asset Inventory)
satz import folders/456789                   # live, one folder
satz import projects/my-prj                  # live, one project
satz import old-estate.yaml --kind estate    # the legacy YAML dialect (until the last org is moved)
satz import ./terraform                      # existing .tf: literal resources → Satz, the rest verbatim in `hcl trust`
satz import ./terraform --wrap-all           # …or every block verbatim
satz import                                  # live, root taken from the import config
satz import organizations/123456789012 --into C0example.satz   # only what the estate does not declare
```

**Parameters:**
- `SOURCE`: what to import from; the shape is read off its form (`--from state|org|yaml|hcl` when it cannot tell). Omit it to use the import config's `root`.
- `--only <types>`: comma-separated resource types, `*` wildcards allowed (`google_*_iam_member`); everything else is switched off for this run. Overrides `only` in the import config.
- `--output, -o <FILE>`: output inside `yaml_dir` (default `discovered.satz`; the extension is always `.satz`).
- `--import-config <FILE>`: the import configuration (default `presets/import-config.yaml`, or `import_config` in `config.toml`).
- yaml shape: `--kind estate|pack`, `--gate <estate>.satz` (compile a converted pack in context), `--fork` (write `<stem>.local.satz`).

**The import config** (`presets/import-config.yaml`, YAML — it is data that
configures an import, not an estate) is the repeatable form of the command line:

```yaml
root:                              # live shape; the command-line SOURCE overrides it
  organization: "123456789012"
  folder: { path: "Shared Services/Prod" }   # or { id: "456789" } — exactly one
  project: my-prj                  # narrows further
only: [google_folder, google_project, "google_*_iam_member"]
resource_types:                    # per type: import on/off, attribute include/exclude,
  google_project:                  # asset_type, and the rules `satz adopt` reads
    import: true
```

A folder `path` is resolved live from the organization, one segment at a time —
exactly one folder may carry each name, otherwise the run stops and lists the
candidates; nothing is guessed. The run prints the effective root and filter.

**An import may be partial; it is never silent about it.** Every run ends with
the skipped list — each resource the source had and the estate does not, with
its reason: `type off (import: false)`, `filtered by --only`, `unmapped` (no
import-config row fits the asset), or `parent not imported`. Counts by reason
always; every name with `--verbose`. The levers are the `import:` rows and
`--only`.

**Delta import (`--into <estate>`).** Identity is the live id, never the label
(the import names a folder `folder-<n>`, your estate calls it `infra_folder`).
The estate's declared resources are resolved to their live ids the way `adopt`
does (dry, nothing changes in the cloud); everything the sweep found with one of
those ids is subtracted; the remainder is written as packs the estate `use`s —
`imported-<scope>.satz` for the top level, `imported-<scope>-<container>.satz`
for what sits under a folder or project the estate already declares, `use`d
from inside that block so the fold places it. The estate is never rewritten
beyond those `use` lines; move entries from a pack into the estate as you adopt
them and the next run subtracts them. The report names what was already
declared (live id → address), what is new, and what is declared but not live.
On a real org: estate + packs → `tofu plan` = N to import, 0 to add, 0 to destroy.

Live imports carry the API's vocabulary. Keys are snake-cased (`storageClass` →
`storage_class`); where the names genuinely differ — `lifecycle.rule[]` is
Terraform's `lifecycle_rule` (a reserved-word collision),
`iamConfiguration.uniformBucketLevelAccess.enabled` is `uniform_bucket_level_access`
(a flattening) — no rule relates the two, so **`satz map-types` derives the map**:
for every `import: true` row it fetches the API's Discovery Document (cached under
`presets/.discovery/`), aligns its schema against the provider schema (exact after
snake_case; flattened leaves; renamed blocks by property overlap; the rest
unmatched) and writes `presets/type-map.yaml`, which the live import applies before
the schema filter. Review the rows it marks `renamed`; re-run after a provider
bump; an ambiguous schema name is pinned with `api_schema:` on the row. What the
schema still does not know is **dropped and reported** (names with `--verbose`)
rather than written into HCL that would not plan. A fetch that fails aborts the
import — nothing is written from a partial sweep.

**Under the Hood:**
- state: reads `tofu show -json` (file, stdin, or run now); only the types with `import: true` are taken; read-only/computed fields are dropped against the provider schema.
- live: one Cloud Asset Inventory sweep under the root; needs `cloudasset.assets.searchAllResources`; useful for infrastructure nobody manages with Terraform yet. Only asset types the config maps are seen.
- yaml: the legacy-dialect converter (`!include` → `use`, anchors → params, `!format` → interpolation), compiled through the fragment pipeline afterwards and reporting what it emits; an old `!import-include` becomes `use` plus `satz adopt`.
- hcl (`satz import ./hcl-dir`): a `resource` block of a schema-known, non-positional type whose values are all literals becomes a Satz resource (attributes as written, repeated nested blocks as a list, the label kept so references from wrapped blocks still resolve); folders, projects, services and grants are **placed** by the folder/project they reference (`parent = google_folder.x.name`, `project = google_project.y.project_id` — the one expression a translated block may contain), so the tree comes back and `customer_organization_id` is inferred; everything else — `module`, `locals`, `data`, `variable`, `output`, blocks using `count`/`for_each`/`dynamic`/`provider`/`depends_on`, expressions and references, groups, memberships, bucket/billing grants and IAM bindings (special Satz forms not derived yet), unknown types, labels that are not identifiers — is carried verbatim inside `hcl trust "imported from <file>:<line>" { … }` and the report says why. `terraform`/`provider` blocks are dropped with a note; the emitter owns `providers.tf`. `--wrap-all` wraps everything (the zero-risk form). Either way the estate deploys exactly as the source did: `tofu plan` against the source's state shows no changes. Also the way in for `gcloud beta resource-config bulk-export --resource-format=terraform` and `tofu plan -generate-config-out` output.

### Update Schemas (`update-schema`)
Refresh local provider schemas to get the latest resource definitions.

```bash
satz update-schema --providers google,google-beta
```

**Parameters:**
- `--providers, -p <LIST>`: Comma-separated list of providers to update.
- `--version, -v <VERSION>`: Provider version to fetch (default: from config).
- `--tf-tool, -t <TOOL>`: Terraform/OpenTofu binary to use.

**Under the Hood:**
- runs `tofu init` in a temporary directory.
- runs `tofu providers schema -json` to export the latest definitions.
- Updates the JSON files in `schemas/`.

### Get presets (`get-presets`) — bootstrap, not an updater
Download the `presets` folder from the repository into your project's `presets_dir` (default: `presets/` beside `config.toml`). The library holds everything available for copying; `yaml_dir` stays reserved for the files you actually use and adapt. Requires a valid config so the tool knows where to write files. Each preset's purpose, include line and variables (required vs. overridable defaults) are documented in [presets/README.md](presets/README.md) — presets are read-only building blocks; all per-org values belong in the estate's `params { … }` block.

```bash
satz get-presets
satz get-presets --force                       # overwrite in-use packs too
satz get-presets --pristine-dir ~/src/satz/presets   # skip the download
```

**Parameters:** `--force`, `--pristine-dir`. Accepts global options `--config`, `--validation`, `--verbose`.

**Under the Hood:**
- Fetches the `presets` directory from the GitHub repo (main branch) in **one** API request (a recursive tree), preserving subdirectories (e.g. `presets/security-group-models/`). The files themselves come from raw.githubusercontent.com, which is not rate-limited.
- GitHub's unauthenticated quota is 60 requests/hour and is shared with `self-update`. Set `GITHUB_TOKEN` to raise it, or pass `--pristine-dir` to skip the network entirely; exhaustion is reported as a rate limit with the wait, not as a parse error. See [docs/presets-workflow.md](docs/presets-workflow.md#5-when-upstream-stops-answering-the-github-quota).
- Then decides per file: **missing** → installed; **identical** → skipped; **differs but the estate does not use it** → refreshed; **differs and the estate USES it** → **refused**, naming `merge-presets` / `merge-presets --adopt <stem>` instead. Changing a pack the estate deploys changes the org, and a `tofu plan` should not be the first place you learn that. `--force` overwrites anyway, listing each in-use pack as it does.
- `X.local.*` files have no upstream counterpart, so nothing here can touch them.

### Compliance goal view (`require`)

Frameworks are data: a **catalog** (`presets/catalogs/cis-gcp-4.0.yaml`) lists control
IDs with this project's own paraphrases; preset packs declare **claims** inline
(`claim "cis-gcp" "4.0" "2.2" implements { … }`): "including me discharges control §x.y, witnessed by
these resources". `require` is the goal view over both:

```bash
satz require cis-gcp-4.0 C0example.satz
#   ✓ 2.2  Sinks for all log entries    — google_logging_organization_sink.…
#   ◐ 2.3  Retention on the log bucket  — open duty: validate-then-lock
#   ✗ 2.11 Storage IAM change alerts    — unmet. Provides: monitoring/organization-cis-log-alerts-central
```

Per control: **✓ satisfied** (an `implements` claim from an included pack, every witness
emitted by the compiler — a resource written inside a raw `hcl { … }` block never counts), **◐ partial** (witnesses present but manual duties
open, or only `contributes` claims), **⚠ deviation** (the estate deliberately does not
meet this control and says why — see below), **✗ unmet** (with the packs that would
provide it — remediation as suggestion), **‼ broken claim** (a pack claims witnesses the
estate does not emit — reported loudly, never silently satisfied), **○ organizational**
(no IaC witness possible). Exit code is non-zero on unmet/broken, so it gates CI —
deviations are disclosed decisions and do not fail it.

### The Satz language

Full specification, grammar and lookup: **`docs/satz-language.md`** — derived from
the parser (`crates/satz-core/src/satz.rs`), with every example verified to compile.

### Adopting what already exists (`adopt`, brownfield)

A first `apply` against an organisation that already has folders, groups, org
policies or a state bucket fails one resource at a time with `409 … already
exists` — or, worse, recreates them. `satz adopt` resolves the live id of every
resource the estate declares and brings it under management:

```bash
satz adopt C0example.satz                                   # dry run: the resolution table
satz adopt C0example.satz --execute                         # write verified "import-id"s into the estate
satz adopt C0example.satz --execute --import --activate     # tofu import now; activate managed constraints
satz adopt C0example.satz --only google_folder,google_cloud_identity_group
```

How a resource is resolved depends on who chose its identity:

- **User-chosen id** (project, bucket, service account, IAM bindings, sinks,
  metrics, custom roles, `project_service`, …): the import id is rendered
  offline from a template on the type's row in `presets/import-config.yaml`
  (`import_id: "projects/{project}/serviceAccounts/{account_id}@…"`), with
  `{placeholders}` filled from the emitted attributes and resolved references.
  Reported as *derived*; its existence is verified by the import itself.
- **GCP-assigned id**: looked up by natural key under the resolved parent —
  folders by display name, groups by email, memberships by group + email, org
  policies by constraint; every other type with a `match_on:` row (essential
  contacts by email, alert policies and notification channels by display
  name, …) through one Cloud Asset Inventory listing of the row's `asset_type`
  under the resource's own scope. Resolution is top-down, so a folder's number
  is known before its children ask for it.
- A type with neither rule is reported as **no rule** — add `import_id:` or
  `match_on:` to its row; that is a one-line data change, not code.

It **never guesses**: exactly one live candidate resolves; none means *on apply*
(Terraform will create it); more than one is **AMBIGUOUS**, the candidates are
listed, and you pin `"import-id"` by hand. Managed org-policy constraints the
organisation has never had need `--activate` (they cannot be imported before
activation; this mutates the org). `--execute` writes the ids into the `.satz`:
a resource with a block of its own gets an `"import-id"` line; an entry-level
resource (an IAM grant, a project service, a membership) has its list entry
rewritten into the object form (`{ role = "…" "import-id" = "…" }`, see the
language reference §6.7). Derived ids are written too — `tofu plan` verifies
them through the import block, and says so if one does not exist. An entry
that cannot be found in the source (interpolated) and a resource declared in a
**pristine pack** (upstream-owned, never edited) come back as hints: import
those with `--execute --import`, or fork the pack first. On the test org write
mode reached 112 of 117 resources; plan = 112 to import, 0 to destroy.

`adopt-org-policies` remains as an alias of
`adopt --only google_org_policy_policy --activate --execute --import`.

It is a separate command on purpose: it makes live API calls and, with
`--activate`, changes the organisation, so it is never a side effect of
`transpile`, which stays pure. The only trace it leaves in the language is the
`"import-id"` it writes.

### Running from anywhere

`--config` takes the `config.toml` **or** the estate directory that holds it, and every
path inside the config resolves against the config's own directory — so any command runs
from any working directory:

```bash
satz transpile C0example.satz --config ~/estates/acme
satz require cis-gcp-4.0 C0example.satz --config ~/estates/acme
satz plan  --config ~/estates/acme
satz apply --config ~/estates/acme
```

`plan`, `apply` and `tf-init` run the configured `tf_tool` (OpenTofu by default) in the
estate's `hcl_dir`, inheriting stdio — so apply's approval prompt and the usual coloured
output behave exactly as when run by hand — and propagating the tool's exit code, so
`plan -detailed-exitcode` still returns 2 for "changes present". Everything after the
subcommand is passed through verbatim:

```bash
satz plan  --config <estate> -target=google_org_policy_policy.foo -out=tf.plan
satz apply --config <estate> tf.plan
```

Because the pass-through is verbatim, **`--config` must come before those arguments** —
written after, it would be handed to OpenTofu instead. satz detects that case and
prints the corrected command rather than a confusing "config.toml not found".

They deliberately do not transpile first. `hcl/` is generated, but coupling generation to
the deploy step would hide a diff the operator should see: transpile, look, then plan.

### Live verification checks enforcement, not existence

`report-compliance` verifies witnesses against the live estate through Cloud Asset
Inventory. For most resource types the question is "does it exist" — but for an org
policy that is not the control: a policy switched off in the console still exists.
So org-policy witnesses are compared by VALUE. The estate's declared
`spec { rules { enforce = "TRUE" } }` is checked against the live policy's
`spec.rules[].enforce`, and a mismatch reports **NOT ENFORCED**, which outranks
DRIFTED — a missing resource is visibly absent, one that is present and switched
off looks healthy in every inventory.

The comparison refuses to guess. A policy with several rules, with none, or a list
constraint with no boolean yields no verdict rather than a wrong one; and a policy
whose live enforcement cannot be read reports *unverifiable*, never *verified*.

### Deviations: declining a control on purpose

A customer fork (`X.local.satz`) exists so an organisation can decline a control
deliberately. Saying nothing is not an option, because a claim witnesses that a resource
*exists*: an org policy declared with `enforce = "FALSE"` still emits its resource, so a
copied `implements` claim would report **satisfied** for a control nobody enforces.
Dropping the claim instead reports **unmet**, which reads as an oversight. Neither is
true, so state the decision:

```
claim "cis-gcp" "4.0" "4.4" deviates {
  resources = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  reason  = "A service here depends on metadata SSH keys; enforcing OS Login breaks it."
  duty_reassess = "Re-assess when that service supports OS Login."
}
```

`reason` is mandatory on a deviation and rejected on the other kinds. Witnesses are
optional — the resource may be present-but-not-enforcing, or absent because the estate
`suppress`ed it — but any witness the claim *does* declare must still be emitted, so
deleting the policy outright resurfaces as a broken claim rather than staying silently
"deviated". A deviation outranks the claims it contradicts, and can be declared by a
pack fork or by the estate itself.

The vocabulary is deliberate: the output never says "compliant" — this judges the
*declared* estate; verification against the *live* estate is the evidence report
(next section). Catalogs carry no framework text (CIS/ISO prose is license-restricted),
only IDs and paraphrases.

### Evidence report (`report-compliance`)

The goal view joined with the **live estate**: every witness of a satisfied/partial
control is verified against Cloud Asset Inventory (org sinks, log metrics, alert
policies, notification channels, buckets — matched by name/display name extracted from
the generated HCL). Manual duties merge with `attestations.yaml` beside config.toml
(`duty-id: {by, date, note}`), and a Prowler native-JSON export can be ingested as
corroboration (`--prowler findings.json` — Prowler's OCSF output or its legacy JSON; a FAIL on one of a control's *verified* witnesses marks the row **CONTESTED**, a FAIL elsewhere is an unmanaged finding beside it).

The exit code is 0 whatever the verdicts — the report is the deliverable;
`--fail-on not-enforced,drifted` (any status word; `any` = everything that is
not verified/declared) makes the run fail for CI after the report is written.

```bash
satz report-compliance cis-gcp-4.0 C0example.satz            # markdown + history
satz report-compliance cis-gcp-4.0 C0example.satz --format pdf --prowler prowler.json
satz report-compliance cis-gcp-4.0 C0example.satz --checkov   # + a Checkov column: failed checks on a control's witnesses
satz triage cis-gcp-4.0 C0example.satz --prowler prowler.json  # the remediation-plan skeleton: A pack covers it / B Satz declares it / C accepted exception / D bring under management / E manual
```

Each row carries the catalog's own one-line `paraphrase` of the control under
its title and, under the witnesses, the `interpretation` the included claims
give of what their resources prove; open duties print their text beside the id.

Row statuses: **verified** (all witnesses live), `verified* (n of m)` (some witness
types have no live check yet — stated, never faked), **unverified** (no witness
could be checked at all — no ADC, inventory unavailable — never spelled
"verified"), **DRIFTED** (declared but not live),
partial (open/attested duties), unmet, broken claim. Each run appends
`evidence/<framework>-<timestamp>.json` beside the config — the evidence history —
and writes the report (pandoc PDF like `report-organizational-policies`). Without
credentials or with `--no-live`, the report degrades honestly to declared-estate
status. The report states check semantics ("a resource with these properties was
verified at this time"), never legal conformity.

### Reconciling preset updates (`merge-presets`) — provenance by suffix

One `presets/` folder; the **filename suffix declares provenance**:

| file | meaning | on `merge-presets` |
|---|---|---|
| `X.satz` | upstream-owned, pristine | always overwritable |
| `X.local.satz` | your fork — the *rename is the fork declaration* | **never touched** |
| `X.diff.satz` | the CURRENT adoption delta: `diff(X.local, pristine X)` | rewritten on every run |
| `<own>.satz` | no upstream counterpart | local-only, kept |

Pack **versions live inside the file** (`pack <name> version "1.2"`); filenames
carry only framework versions (CIS-GCP-Foundation-**4.0**, catalogs). Never a
`X.local.<n>.satz`, never more than one diff per pack — history lives in git.

**A preset that your estate includes never changes silently.** When upstream's
version differs *semantically* (the canonical form of the parsed pack — comment,
formatting and version-line churn upgrades silently), merge-presets:

1. preserves your current content as `X.local.satz`,
2. repoints the estate's `use` to it — then **proves the edit**: the transpiled
   output must be byte-identical (it is, by construction), else everything rolls
   back,
3. updates pristine `X.satz` and writes `X.diff.satz` — exactly what adopting
   upstream would change, with a `local -> upstream` version header.

**Adoption** is the deliberate act, and `--adopt <stem>` performs it: the pristine
name is overwritten in place, the estate's `use` is left alone, and the run prints
the **emission** delta (which resources appear or disappear) rather than the preset
diff. `--adopt all` covers every pack merely BEHIND and refuses one that differs at
the same version — that is an edit, and it must be named. A fork+repoint needed in
the same run is DEFERRED: the repoint proves itself by transpile identity, which an
adoption legitimately invalidates. To adopt an existing fork instead, point the
`use` back at the pristine name and delete `X.local.satz` (the next run removes the
orphaned diff). Presets *not*
included by the estate are simply overwritten when they differ (git history keeps
tracked files). The estate file must be git-clean for auto-repoints — commit or
stash first so the repoint stays an isolated, reviewable edit. `--report-only`
prints every planned action without writing; `--estate <file>` overrides the
default discovery (the single `estate` .satz in yaml_dir).

Version hygiene is cross-checked: semantic change without a version bump warns
(upstream release bug); a bump with identical semantics upgrades silently.

Non-zero exit when anything needs attention (a fork was created or its upstream
moved, or a repoint was refused) — CI-friendly.

> **Which command when?** [docs/presets-workflow.md](docs/presets-workflow.md)
> walks the whole decision — how to tell a newer preset exists, whether your copy
> is *stale* or *edited* (they need different commands), and what to check before
> applying. A proposal for closing the gap between these three commands is in
> [docs/presets-commands-proposal.md](docs/presets-commands-proposal.md).

### Check presets for drift (`check-presets`)

Presets are read-only building blocks — per-org values belong in the estate's `params` block as
[overridable defaults](presets/README.md). `check-presets` finds preset copies that were
edited locally anyway, and tells you how to migrate:

```bash
satz check-presets C0example.satz            # compares against upstream (downloads a pristine copy)
satz check-presets C0example.satz --pristine-dir /path/to/pristine/presets
```

Every local preset is compared against its pristine upstream version and classified:

- **clean** — identical, or only comments/formatting differ, *and* the in-file
  `pack … version` matches upstream.
- **STALE** — a newer release exists. Printed with the version pair
  (`local v1.5, upstream v2.1`) and what moved. Comment-only version bumps say so
  and do not fail the gate; a stale **included** pack does. A pristine file whose
  `.local` sibling exists is exempt from the adopt advice — the estate runs the
  fork, and that copy is the fork's baseline.
- **EDITED (variables only)** — same version, only default values changed.
  Mechanically migratable: the report prints the exact lines to add to the estate's
  `params` block (params the estate already overrides are flagged as redundant
  instead). After adding them, `get-presets` restores the pristine preset — the
  transpiled output is unchanged.
- **EDITED (structural)** — same version, resource bodies or the variable set itself
  differ; not mechanically migratable, review by hand. Same version with different
  content means a local edit, or an upstream release that moved without a bump.
- **fork** — `X.local.*` files are deliberate forks, reported as such (never an error);
  their upstream deltas live in `X.diff.satz`.
- **local-only** / **missing locally** — customer-own files and new upstream presets.

Presets actually used by `<INPUT>` (via `use`) are tagged
`[included]`; drift in an included preset makes the command exit non-zero, so it can
gate CI. `use … when` is followed unconditionally here: a pack whose switch is off
still counts as included (over-reporting drift is the safe direction).

### Self-update (`self-update`)
Check for and install a new release from GitHub. After a successful install, the tool downloads the release README and prints its full path, then opens it unless you pass the options below.

```bash
# Check for and install a new release (same installer as curl)
satz self-update

# Only check if an update is available (no install, no README)
satz self-update --check-only

# Skip downloading README after install, or skip opening it
satz self-update --no-open-readme
```

**Self-update options:** `--no-open-readme` (do not open the documentation site after installing), `--check-only`, `--skip-checksum`. The program can also check for updates on start-up (`self_update_frequency` in the global settings).

**Under the Hood:**
- Fetches the latest release from the GitHub API and compares versions. When a newer version is available it downloads `satz-installer.sh` and `satz-installer.sh.sha256` from that same release, verifies the SHA-256 digest, and only then runs the installer. A checksum mismatch aborts; a release without the sidecar aborts too, unless you pass `--skip-checksum`. On success, optionally downloads `README.md` from the repo and prints its path (e.g. `README: /Users/you/Downloads/satz-0.4.9-README.md`).

### Open the documentation (`open-readme`)

```bash
satz open-readme
```

Opens the documentation site — <https://tjirsch.github.io/satz/> — in the
browser: this README, the language reference and the preset docs, rendered
from the repository's Markdown on every release tag (`.github/workflows/pages.yml`).

### Shell Completion (`completion`)
Generate a tab-completion script for your shell. Supports `bash`, `zsh`, `fish`, and `powershell`.

The shell argument is optional: when omitted it is auto-detected from `$SHELL`
(falling back to `zsh` on macOS and `powershell` on Windows). On macOS, running
`completion` with no shell also auto-installs the script.

```bash
# Easiest: detect the shell from $SHELL; on macOS this also installs
satz completion

# Print completion script to stdout and add to shell config manually
satz completion bash >> ~/.bash_completion
satz completion zsh >> ~/.zshrc

# Auto-install to the canonical location for the shell
satz completion zsh --install
# → installs to ~/.zsh/completions/_satz
# → prints fpath setup instructions

satz completion fish --install
# → installs to ~/.config/fish/completions/satz.fish
```

**Install locations for `--install`:**

| Shell | Path |
|-------|------|
| bash | `~/.local/share/bash-completion/completions/satz` |
| zsh | `~/.zsh/completions/_satz` |
| fish | `~/.config/fish/completions/satz.fish` |
| powershell | `%USERPROFILE%\Documents\PowerShell\Completions\satz.ps1` |

For zsh, add this to `~/.zshrc` if not already present:
```zsh
fpath=(~/.zsh/completions $fpath)
autoload -Uz compinit && compinit
```

## Day 0 Onboarding Playbook

This section outlines the step-by-step process for onboarding a new Google Cloud Organization.

### Phase 1: Preparation

#### Prerequisites
Ensure the executing user has:
- **Superadmin** access to the Google Workspace / Cloud Identity.
- **Organization Administrator** role on the GCP Organization.
- **Billing Account Administrator** on the target billing account (must be granted in the Reseller Console).

#### Workspace Setup
1. Authenticate with Google Cloud:
   ```bash
   gcloud auth application-default login
   ```
2. Initialize the tool configuration and folder structure:
   ```bash
   satz init \
     --customer-id "C01234567" \
     --customer-shortname "example-org" \
     --billing-account-infra "A12345-B67890-C12345" \
     --customer-domain "example.com" \
     --customer-organization-id "123456789012" \
     --iac-user "admin@example.com"
   ```

### Phase 2: Fundamental Infrastructure

#### 1. Bootstrap Core Resources
The `bootstrap` command creates the day-0 infrastructure: the infrastructure
folder, the management project, the billing link, the foundation APIs (fixing
the "chicken-and-egg" problem) and the Terraform state bucket — then runs
`transpile`, `init` and the first imports so what it created is under
management from the start.

```bash
satz bootstrap C0example.satz
```

**Pre-flight.** Before anything is created, bootstrap verifies the ADC
identity against `first_admin` and tests the REQUIRED PERMISSIONS — never
roles — with `testIamPermissions`:

| Where | Permission | Supplied by |
|---|---|---|
| scope root | `resourcemanager.folders.create` (only when `infra_folder_name` is set) | `roles/resourcemanager.folderAdmin` |
| scope root | `resourcemanager.projects.create` | `roles/resourcemanager.projectCreator` |
| scope root | `orgpolicy.policies.create` (the estate's policies, at first apply) | `roles/orgpolicy.policyAdmin` |
| billing account | `billing.resourceAssociations.create` | `roles/billing.user` |

- Everything granted → bootstrap proceeds.
- Something missing and the caller holds `setIamPolicy` on the scope root —
  the normal state of a fresh organization, whose creating super admin is
  auto-granted Organization Administrator (that role carries `setIamPolicy`
  but none of the create permissions) → bootstrap **self-grants** the missing
  roles to the caller, prints each grant with the exact
  `remove-iam-policy-binding` undo command, waits for IAM propagation and
  re-tests before proceeding.
- Something missing and no `setIamPolicy` (or the billing permission, which
  is never self-granted) → bootstrap prints the exact
  `gcloud … add-iam-policy-binding` commands for an administrator and stops
  **before creating anything**.

**Folder-scoped installs.** Set `customer_organization_id = "folders/<id>"`
and the estate installs under that folder: permissions are tested there, and
org-root operations are out of scope by design — a folder-granted operator is
never asked to become org admin. One caveat: Google allows
`roles/orgpolicy.policyAdmin` only at organization level, so a missing
`orgpolicy.policies.create` is reported as advisory on a folder scope —
folder-level org policies need an organization-level grant before their first
apply.

**Dry run.** `satz bootstrap <estate> --dry-run` is read-only: it prints the
plan, verifies the identity and runs the same pre-flight (a would-be
self-grant is reported, not executed). Without credentials the plan still
prints and the skipped pre-flight is named (`pre-flight: SKIPPED`).

**What bootstrap does NOT do:** it creates no service account and grants no
IAM beyond the self-grant above — the IaC service account and its grants are
declared in the estate and come into being on the first `tofu apply`.

**Credential line.** Every live command prints one line before its first API
call — `credentials: <identity> (user ADC | impersonated service account |
service account key), quota project <p>` — so a wrong per-customer login
surfaces immediately instead of as a downstream 403. `satz whoami` is the
explicit check (`--offline` for the file-only view; a user ADC file stores no
identity, so the online form resolves it via token introspection).

#### 2. (Optional) Customize & Transpile
*Only needed if you modify the estate after bootstrap.*

Modify `yaml/C0example.satz` as needed, then manually generate the HCL:
```bash
satz transpile C0example.satz
```

#### 3. (Optional) Configure Identity
*Only needed if the default identity setup from bootstrap was insufficient.*

If customization was done, re-run initialization:
```bash
cd hcl
tofu init
tofu apply
```



### Phase 3: Identity & Access Rollout

#### 1. Apply Management Infrastructure
Run the first Tofu apply. This creates the **Identity Groups**, attaches the necessary **IAM roles** (including `Token Creator`), and finalizes the management project.

```bash
cd hcl/
tofu plan
tofu apply
```

### Phase 4: Cloud Migration

#### 1. Perform State Migration
Toggle to `cloud` mode and move state to the GCS bucket:
```bash
satz migrate C0example.satz --mode cloud
```
The tool rewrites the estate's `deployment_mode`, switches to **Service Account Impersonation**, and runs `tofu init -migrate-state`.

#### 2. Verification
In `cloud` mode, verify that you can plan/apply using the restricted service account identity:
```bash
tofu plan
```

#### Template Params Reference

When you run `init`, the following params are generated in the estate's `params { … }` block:

| Param | Default | Description |
|-------|---------|-------------|
| `infra_folder_name` | `"Infrastructure"` | Display name for the top-level folder. Leave `""` to create the project in the root. |
| `infra_project_name` | `""` | The unique ID for the management (IaC) project. |
| `infra_bucket_name` | `""` | The name of the GCS bucket for Terraform state. |
| `customer_id` | (from CLI) | The Workspace customer ID (e.g., `C01234567`). |
| `customer_organization_id` | `"123456789012"` | The numeric Google Cloud Organization ID. |
| `customer_domain` | `""` | The customer's primary domain (e.g., `example.com`). |
| `first_admin` | (from `--iac-user`) | Local part of the first admin's address; members are built as `user:{first_admin}@{customer_domain}`. |
| `customer_longname` | `""` | The full legal name of the customer entity. |
| `customer_shortname` | `""` | A unique slug or shortname for the customer. |
| `svc_iac_account` | `"svc-iac-001"` | The name/ID of the primary IaC Service Account. |
| `svc_iac_users_group` | `"svc-iac-users"` | The Cloud Identity group for IaC administrators. |
| `billing_account_infra` | `""` | The Billing Account ID (e.g., `012345-6789AB-CDEF01`). |
| `deployment_engine` | `"tofu"` | The IaC tool: `tofu` or `terraform`. |
| `deployment_mode` | `"local"` | `local` for Day 0 (User ADC); `cloud` for Day 1+ (Impersonation). Switched by `satz migrate`. |
| `default_region` | `"europe-west3"` | Default region for regional resources. |
| `default_zone` | `"europe-west3-a"` | Default zone for zonal resources. |

### 3. Transpile
Compile an estate to HCL. Run this from within the customer repository directory.
```bash
satz transpile my-infra.satz
```
- Input is read from `yaml_dir` (e.g., `./yaml/my-infra.satz`).
- Output is written directly to the `hcl_dir` defined in your config.
- **Run from anywhere**: All paths are resolved relative to the configuration file's directory.
- **Automatic Schema Sync**: The tool will automatically fetch missing provider schemas via `tofu/terraform` during transpilation.

### 3. Update Schemas
Refresh provider schemas manually.
```bash
satz update-schema --providers google,google-beta
```

## Configuration

There are two separate configuration concepts:

1. **Project config** (`config.toml`) — per-project paths and provider settings (see below).
2. **User settings** (`~/.config/satz/satz.toml`) — user-level program behavior (e.g. update checks); see [Global Options → User settings](#global-options).

### Project config (config.toml)

Per-project settings are read from **`config.toml`** in the project root (or the path given by `--config`). This file defines `yaml_dir`, `hcl_dir`, providers, and other project-specific options. Default values are:

| Key | Default | Description |
|-----|---------|-------------|
| `yaml_dir` | `"yaml"` | Source directory for estate files |
| `hcl_dir` | `"hcl"` | Target directory for generated HCL |
| `schema_dir` | `"schemas"` | Directory where provider schemas are cached |
| `presets_dir` | `"presets"` | Preset library downloaded by `get-presets`; the import-config default resolves here |
| `include_dirs` | `[".", "yaml"]` | Search paths for `use`d packs |
| `tf_tool` | `"tofu"` | The binary used to fetch schemas |
| `google_providers` | `["google", "google-beta"]` | List of Google providers |
| `provider_version` | `"7.12.0"` | Provider version to use |
| `auto_explode` | `["google_project_service", ".*_iam_member"]` | Resources that use compact explosion |
| `validation_level` | `"warn"` | Validation level for mandatory parameters |

### File locations

| Path | Description |
|------|-------------|
| `~/.config/satz/satz.toml` | User parameters (e.g. `self_update_frequency`). Created on first run with defaults. |
| `config.toml` | Project config (paths, providers). Per project; use `--config` to override path. |

## Schema Validation

The tool automatically checks your estate against the provider schemas to ensure all mandatory parameters and blocks are present.

- **Attributes**: Checks for `required` fields (e.g., `project_id`).
- **Blocks**: Checks for mandatory blocks with `min_items > 0` (e.g., `boot_disk` for a VM).

You can control the strictness via CLI `--validation` or `config.toml`.

## Satz

Estates are written in **Satz** (`.satz` files) — the language reference is
[docs/satz-language.md](docs/satz-language.md). Params are declarations in one
document-ordered namespace (no anchors), `"{param}"` interpolates (no `!format`), `use "pack.satz" [as key] [when param]`
includes, blocks nest with braces, and resource attribute names are **1:1 the Terraform
provider names** — the registry docs are the docs. A `.satz` estate is parsed directly by
the fragment pipeline (per-file fragments, folded by address, emitted as HCL); packs are
Satz-native, pack params are overridable defaults, and **control claims are language
syntax** —

```
claim "cis-gcp" "4.0" "2.2" implements {
  resources = ["google_logging_organization_sink.archive", …]
  duty_validate_then_lock = "…"
}
```

— read by `require`/`report-compliance` from the same compile that produces the witnesses,
so claims and witnesses can never disagree. Coverage is `implements`, `contributes` or
`deviates`; witnesses are mandatory on the first two. Literal Terraform `${…}` references
inside strings need doubled braces (`"${{google_project.x.project_id}}"`) since `{…}`
interpolates params. Every command reads `.satz`. The legacy YAML dialect that
preceded Satz exists only as input to `satz import <file>.yaml`, which converts an estate or a pack,
gated by compiling the result through the fragment pipeline and reporting what it emits —
a migrated estate may need a manual edit; `tofu plan` has the last word
(see [docs/satz-language.md §12](docs/satz-language.md)).

## Core Principles

The tool follows a central design philosophy based on **Hierarchy Context**, **Attribute Inheritance**, and **Strict Validation**.

### 1. Hierarchy Context & Nesting
Resources are defined within the context of their parent in the organization hierarchy:
- **Project Context**: Resources that require a project (e.g., Buckets, VMs, Networks) are usually nested directly within a `google_project` definition.
- **Folder Context**: Resources belonging to a folder (e.g., Folder IAM members) are usually nested within a `google_folder` block.
- **Organization Context**: Organization-wide resources (e.g., Group memberships, Org IAM) are defined at the root level of the estate.
- **Explicit Placement**: Any resource can be defined outside its logical hierarchy container if the identifying parameter (e.g., `project_id`, `folder`) is provided explicitly.

### 2. Attribute Inheritance (Narrowest Context)
Nested resources automatically inherit identity attributes from their surrounding context if not explicitly defined:
- **Automatic Matching**: The tool identifies which identifier a resource needs based on its schema (e.g., `project_id`, `project`, `folder_id`, `org_id`).
- **Inheritance**:
    - A resource inside a Project context inherits the Project ID.
    - A resource inside a Folder context inherits the Folder ID.
- **Narrowest First**: If a resource is defined in a scope where multiple contexts apply (e.g., inside a Project which is inside a Folder), it inherits from the **most specific (narrowest)** context available.
- **Explicit Override**: Explicitly provided attributes in the source always take precedence over inherited context values.

### 3. Context Validation & Typo Detection
To ensure configuration correctness, nested blocks are strictly validated:
- **Attribute vs. Resource**: Every key within a `Project` or `Folder` block must be either:
    - A valid native attribute/block of the parent resource (e.g., `name` for a project).
    - A valid resource type from the cloud provider schema.
- **Error Detection**: Any key that is neither a known attribute nor a known resource type is a **hard error** naming the file and line (a typo never deletes infrastructure silently).
- **Missing Context**: Resources that require a project or folder identifier but are defined outside such a context (without an explicit identifier provided) trigger a warning on stderr; `tofu validate` then fails on the missing attribute.

### 4. Flexible Placement
While the tool encourages a clean hierarchy, it allows placing cross-context resources (like `google_cloud_identity_group`) inside a Project block for configuration convenience (e.g., defining project-relevant groups near the project). The transpiler will process these correctly, ignoring the project context where it doesn't apply to the resource's schema.

## Handling Resource Renames (State Migration)

If you rename a resource in your estate, the transpiler will generate a new HCL label. OpenTofu will see this as a "delete and recreate" action. To avoid downtime, you can use the built-in migration suite:

1.  **Iterate Locally**: Use `tofu plan -out=plan.binary` and `tofu show -json plan.binary > plan.json` to identify changes.
2.  **Map Moves**: Use `satz scan-plan plan.json` to generate a `mapping.yaml`.
3.  **Apply Renames**: Run `satz generate-migration mapping.yaml` and execute the resulting script to perform the `mv` commands safely.

For switching between local and cloud backends, always use the high-level `satz migrate` command.

### Scan Plan (`scan-plan`)
Analyze a Terraform/OpenTofu plan JSON file to identify resource renames and generate a mapping file.

```bash
satz scan-plan plan.json --output mapping.yaml
```

**Parameters:**
- `<plan_json>`: Path to the plan JSON file (required).
- `--output <FILE>`: Path to output mapping YAML file (default: `mapping.yaml`).

**Under the Hood:**
- Parses the plan JSON to identify resources that are being destroyed and recreated with new addresses.
- Generates a mapping file that correlates old and new resource addresses.
- The mapping file can be used with `generate-migration` to create state move commands.

### Generate Migration (`generate-migration`)
Generate a shell script with `tofu state mv` commands from a mapping YAML file.

```bash
satz generate-migration mapping.yaml --output migrate.sh
```

**Parameters:**
- `<mapping>`: Path to the mapping YAML file (default: `mapping.yaml`).
- `--output <FILE>`: Path to output shell script (default: `migrate.sh`).

**Under the Hood:**
- Reads the mapping file generated by `scan-plan`.
- Generates a shell script with `tofu state mv` commands to safely rename resources in the state.
- The script can be reviewed and executed manually to perform the state migration.

## Day 0: Migration Playbook

This section outlines the general process for migrating existing infrastructure into `satz` management.

### 1. State Discovery
Begin by capturing the current infrastructure state. If you have an existing Terraform/OpenTofu project, generate a JSON state file and use the discovery tool:
```bash
tofu show -json > state.json
satz import state.json -o migration-discovery.satz
```

Alternatively, if you want to discover infrastructure directly from GCP without Terraform state:
```bash
satz import organizations/123456789012 -o migration-discovery.satz
```

Only the resource types marked `import: true` in `presets/import-config.yaml` are
taken (`--only` narrows further); enable more rows as needed — every row with an
`asset_type` can be switched on. The table covers the provider's 895 resource
types: 389 with their Cloud Asset Inventory name (derived from the type name
and checked against Google's list, `presets/cai-asset-types.txt`), 296 that are
not Cloud Asset resources (IAM members, org-policy v1 shapes; state shape only),
209 still `TODO/UNKNOWN` (Cloud Asset does not inventory them, or the name
could not be derived — `scripts/update_import_config.py` prints what it tried).
A live resource whose provider block would not plan is never written: a
required attribute the asset data lacks is derived where it can be (`parent`,
`org_id`/`folder`/`project` from the asset path, a service account's
`account_id` from its email) and the resource is otherwise skipped with the
attribute named. Import ids of live resources are the asset path, with the
project named by id (the provider keeps a project NUMBER on import and the
declared id would then force a replacement). Verified on a test organization
with folders, projects, services, buckets, IAM, org policies, org/folder/project
log sinks, a service account and an essential contact: `tofu plan` = every
resource imported, nothing added or destroyed.

### 2. Hierarchical Refinement
The discovered estate compiles as-is, but it is as found. Organize it into the `satz` hierarchical format:
- Move projects into their respective folders.
- Nest resources (Buckets, Networks, etc.) inside their projects to leverage **Attribute Inheritance**.
- Remove redundant attributes (like `project_id`) that are now inherited from the context.

### 3. Resource Optimization
Convert standard resource definitions into optimized `satz` patterns:
- **Services**: Group `google_project_service` resources into a single `project_service` list.
- **IAM**: Combine individual IAM members into compact `project_iam_member` or `folder_iam_member` blocks.
- **Formatting**: Ensure attributes with sub-structures (e.g., `project_service` with `disable_on_destroy`) are correctly indented.

### 4. Validation & Reconciliation
Generate the HCL and compare it with the live environment:
1. Run `satz transpile migration-discovery.satz`.
2. Run `tofu plan` in the `hcl/` directory.
3. **Reconcile**: If the plan shows "replace" instead of "no changes", it means the HCL labels or resource IDs don't match.
   - Use `"import-id"` in the estate to link existing resources.
   - Or use `tofu state mv` to align the existing state with the new HCL labels.

### 5. Transition to Management
Once `tofu plan` shows no changes (or only intended updates), the migration is complete. You can now manage the infrastructure exclusively through the estate.

## Development

**Prerequisites:** latest stable **Rust**, and **OpenTofu** (or Terraform) on your `PATH`.

```bash
cargo build --release                              # build
cargo run -- --config config.toml transpile C0example.satz   # run a command
cargo test --workspace                             # run unit tests (all crates)
cargo fmt && cargo clippy --workspace --all-targets  # format + lint
cargo install --path .                             # install the release binary (see Installation)
```

## Releasing

Releases are built by GitHub Actions (cargo-dist) when a **version tag** is pushed. Pushing only `main` does not trigger a release. From a clean `main`:

```bash
cargo release patch --execute --no-confirm    # or: minor
```

`cargo-release` (config in `release.toml`) bumps `Cargo.toml`, commits `version bump`, tags `vX.Y.Z` and pushes commit and tag. The tag runs `.github/workflows/release.yml`: build the four targets, create the GitHub release with archives, `sha256.sum` and `satz-installer.sh`, then the `attach-checksum` post-announce job (`dist-workspace.toml`, `.github/workflows/attach-checksum.yml`) uploads `satz-installer.sh.sha256` — the sidecar `self-update` verifies against. `prune-releases.yml` afterwards keeps the five newest releases. Re-run `dist generate` after editing `dist-workspace.toml`; the `plan` job runs `dist generate --check` and fails on a hand-edited `release.yml`.

The tag pattern is `**[0-9]+.[0-9]+.[0-9]+*`; the tagged commit must carry that exact `version` in `Cargo.toml`. Common reasons a release doesn't run: only `main` was pushed, the tag predates the bump commit, or the tag/`Cargo.toml` versions differ.

## Architecture

`satz` compiles Satz estates into production-ready OpenTofu/Terraform HCL. It prioritizes structure, inheritance, and validation.

### Core Components

#### 1. Fragment pipeline (`crates/satz-core`, `src/emitter.rs`)
The heart of the tool. `satz.rs` parses each `.satz` file, `pipeline.rs` resolves params
and `use`s into per-file fragments, `algebra.rs` folds them by Terraform address (⊕), and
the emitter renders the folded IR as `main.tf`, `providers.tf`, `variables.tf`,
`terraform.tfvars` and `imports.tf`.
- **Context Awareness**: a nested resource inherits its parent's identifier (`project`, `folder_id`, `org_id`) from the enclosing block.
- **Intrinsic scopes**: groups, org grants and billing grants hoist to their real scope wherever they are written.

#### 2. Schema Registry (`src/schema.rs`)
Manages Terraform provider schemas (loaded as JSON).
- **Typing**: every resource key and block key is checked against the schema at parse time — an unknown key is an error, not a guess.

#### 3. Template Generator (`src/template.rs`)
Provides a consistent starting point for new customer rollouts.
- **Declarative Bootstrap**: Generates the Satz estate representing the Day 0 infrastructure (Project, Services, Bucket, SA) under the labels `bootstrap` imports by name.

#### 4. Migration (`crates/satz-core/src/migrate.rs`)
The only reader of the legacy YAML dialect: `satz import <file>.yaml` converts an estate or a pack
(`!include` → `use`, anchors → params, `!format` → `"{param}"` interpolation, `!expr` →
`"${{…}}"`), then compiles the result through the fragment pipeline and reports what it
emits. An old `!import-include` becomes `use` plus a `NEEDS ADOPTION` note — its job is
`satz adopt`.

#### 5. Discovery Engine
`satz import organizations/<n>` (and the `folders/`, `projects/`, `state.json` shapes) reverse-engineer a Satz estate from what exists.
- **Asset Ingestion**: Consumes CAI (Cloud Asset Inventory) export streams.
- **Configurable Filtering**: Uses `import-config.yaml` to include/exclude resources and attribute fields.
- **Schema Validation**: Validates discovered data against Terraform schemas, automatically filtering read-only or computed fields to ensure valid HCL generation.
- **Heuristics**: Intelligent mapping of IAM policies (e.g., `google_storage_bucket_iam_member`) and key generation.

#### 6. Organization Policy Engine (`src/org_policy.rs`)
Aligns curated Org Policy sets (e.g. `presets/CIS-GCP-Foundation-4.0.satz`) with the live organization via the GCP Org Policy API v2 (ADC auth, reusing the `bootstrap` pattern).
- **Adoption** (`satz adopt --activate`): activates managed constraints that are missing (API create), then imports the existing policies into state — no manual console activation and no `import-id` editing. `transpile` stays pure; adoption is a separate, explicit command (`src/adopt.rs` drives it through this module's `OrgPolicyClient`).
- **CLI commands**: `export-organizational-policies` (snapshot live state to a re-importable preset), `diff-organizational-policies` (semantic current-vs-desired report), `report-organizational-policies` (markdown/JSON/PDF inventory with constraint descriptions).
- **Managed constraints**: constraints whose name contains `.managed.` must be *activated* (API create), then *imported as-is* (`tofu import`), then *modified* (`tofu apply`). `satz adopt --activate` sequences the activate+import; `tofu apply` does the modify.
- **Pure diff core**: classification + `normalize_spec` are IO-free and unit-tested; they reconcile `enforce "TRUE"`↔`true`, `allowed_values` ordering, and `parameters` JSON-string↔object so semantically-equal policies don't show as diffs.

#### 7. Cloud Identity Group Lookup (`src/cloud_identity.rs`)
The group and membership resolvers `satz adopt` uses. A groups pack declares groups by name; adopting the ones that already exist needs their opaque `groups/<id>`, which used to be pasted in by hand.
- **Lookup, not guesswork**: the group email the emitted HCL carries (`group_key.id`) is resolved via `cloudidentity.googleapis.com/v1/groups:lookup`; a member email via `memberships:lookup`. Existing ones are imported; missing ones are left for `tofu apply`.
- **403 is ambiguous**: some tenants return it for a nonexistent group as well as for a permission problem, so a denied lookup falls back to listing `customers/<customer-id>` once and answers from that. If that fails too the resolution is reported as FAILED with an actionable hint rather than treated as absent.
- **Declared memberships only**: `adopt` resolves the memberships the estate emits — live members the estate does not mention are never looked at, so adopting a group cannot make `apply` propose deleting somebody. The membership label is a `DefaultHasher` digest of `(group key, raw member string)` computed by the same `membership_resource_label` helper the emitter uses; `membership_address_matches_the_generated_resource` pins the two together.
- **Quota project**: every request sends `x-goog-user-project`, resolved from `GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT` or the ADC file's `quota_project_id`.

### Bootstrap Workflow (Declarative Tofu)
Instead of hardcoded setup scripts, `satz` uses a two-phase Tofu approach:
1. **Local Phase**: `deployment_mode = "local"`. Runs under User ADC. Creates the management project and initial Service Account.
2. **Cloud Phase**: `deployment_mode = "cloud"` (`satz migrate <estate> --mode cloud`). Uses Service Account impersonation and a GCS backend for all subsequent operations.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
