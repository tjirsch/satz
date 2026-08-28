# satz

**Infrastructure with a constitution — proven live.** Satz compiles an estate — written in a language whose resource types and attributes are the Terraform provider's — to OpenTofu/Terraform HCL, and proves the controls it declares against the live Google Cloud organisation. It succeeds the tool that began as `cfg2hcl`.
Builtin functions to bootstrap a Google Cloud Organization and do state import, migration and discovery of an existing GCP Organization from state or live infrastructure.

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
- `--verbose`: Enable verbose output. When invoked without a subcommand (e.g. `satz --verbose`), prints full recursive help listing all subcommands and their options.

### User settings (~/.config/satz/satz.toml)

User-level **parameters** (e.g. when to check for updates) live in **`~/.config/satz/satz.toml`**. This file is **created on first run** with default values (e.g. `self_update_frequency = "always"`). If the file is missing on load, it is created with defaults.

| Option | Default | Description |
|--------|---------|-------------|
| `self_update_frequency` | `"always"` | When to check for updates on normal runs: `never`, `always`, or `daily` (at most once per 24 hours). The check is check-only (no install, no README). |
| `preferred_editor` | *(none)* | Editor command used to open files (e.g. `"zed"`, `"code"`, `"vim"`). Falls back to `$EDITOR` env var, then the OS default app. String values must be quoted. |

**Project config** (paths, providers, etc.) stays in **`config.toml`** per project; see [Configuration](#configuration) below.

Example (optional; the file is created automatically when needed):

```toml
self_update_frequency = "daily"
preferred_editor = "zed"
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
| `diff-organizational-policies <CONFIG_FILE>` | `--preset`, `--customer-organization-id`, `--report`, `--format` (`console`\|`markdown`\|`json`) |
| `report-organizational-policies <CONFIG_FILE>` | `--customer-organization-id`, `--scope` (`active`\|`inactive`\|`full`), `--format` (`markdown`\|`json`\|`pdf`), `--report` |
| `transpile <INPUT>` | `--output`, `--schema-dir`, `--print-variables` (runs the `!import-include` live lookup + import as a side effect when present) |
| `scan-plan <plan_json>` | `--output` (default: `mapping.yaml`) |
| `generate-migration <mapping>` | `--output` (default: `migrate.sh`) |
| `update-schema` | `--providers`, `--version`, `--tf-tool` |
| `discover-from-state` | `--state-json`, `--output`, `--add-import-id`, `--add-import-id-as-comment`, `--discovery-config` |
| `discover-from-organization` | `--customer-organization-id` (required), `--output`, `--add-import-id`, `--add-import-id-as-comment`, `--discovery-config` |
| `migrate <INPUT>` | `--mode` |
| `get-presets` | `--force` — overwrite presets the estate uses too; `--pristine-dir` |
| `require <FRAMEWORK> <INPUT>` | *(catalog id, e.g. `cis-gcp-4.0`)* |
| `report-compliance <FRAMEWORK> <INPUT>` | `--format` (`markdown`\|`json`\|`pdf`), `--report`, `--prowler`, `--no-live` |
| `merge-presets` | `--pristine-dir`, `--estate`, `--report-only`, `--adopt <stem\|all>` — reconciling update; `--adopt` upgrades in place instead of forking |
| `check-presets <INPUT>` | `--pristine-dir` |
| `self-update` | `--no-download-readme`, `--no-open-readme`, `--check-only`, `--skip-checksum` |
| `open-readme` | *(none)* |
| `completion [SHELL]` | `--install` |
| `set-preferred-editor [EDITOR]` | `--clear` |

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
- If customer details are provided, generates a template estate in `yaml/` (still written in the legacy YAML dialect — run `migrate-to-satz` on it).
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
2.  **Infrastructure Folder**: Checks availability or creates the top-level folder (requires `Folder Admin`).
3.  **Project Shell**: Creates the management project (project-id defaults to `shortname-iac-infra`) inside the folder.
4.  **Billing Link**: Links the project to the specified Billing Account.
5.  **Enable APIs**: Enables the foundation APIs (Service Usage, Cloud Resource Manager, IAM, IAM Credentials, Storage, Cloud Billing, Cloud Identity, Cloud Asset, Logging, Org Policy, Essential Contacts).
6.  **State Bucket**: Creates the GCS bucket for Terraform state (with versioning, uniform access).
7.  **Automated Setup**:
    - **Transpile**: Compiles the estate to HCL.
    - **Init**: Runs `tofu init` to download plugins.
    - **Import**: Automatically imports the created Folder, Project, and Bucket into the local state.

### Transpile (`transpile`)
Compile your estate to production-ready HCL. Input is `.satz`; the legacy `.yaml` dialect is still accepted.

```bash
satz transpile <INPUT> [options]
```

**Parameters:**
- `<INPUT>`: Name of the estate file. This is resolved relative to the `yaml_dir` defined in your config.
- `--output, -o <FILE>`: Optional output subdirectory or absolute path. By default, output goes to `hcl_dir`.
- `--schema-dir, -s <DIR>`: Override the schema directory.
- `--print-variables`: After transpilation, print the resolved variable table (`terraform.tfvars`) to stdout. Useful for debugging variable resolution across multiple include files.

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
with every contributing origin), and HCL is emitted from the folded result.
Output is equivalent to the legacy path (differential-gated) with deterministic
ordering.

**Subtractive overrides (`suppress`, satz estates only):**
An estate can remove something a used pack contributes — without forking:

```
use "presets/CIS-GCP-Foundation-4.0.satz" as org_policy_policy

// drop one pack resource; grant-edge form removes a single role
suppress google_org_policy_policy "iam-allowedPolicyMemberDomains"
suppress google_organization_iam_member "group:sec@example.com" role "roles/viewer"
```

Rules: interpolations (`{param}`) work in the name; a suppress that matches
nothing is a **hard error** (stale subtractive config must surface, never
silently deploy); a suppressed resource that was the witness of a compliance
claim shows up as broken in `require` — deliberately. The YAML dialect cannot
express suppressions; the legacy path refuses such files.

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
never carry a claim. The YAML dialect cannot carry raw HCL; the legacy path
refuses such files.

**Under the Hood (YAML dialect):**
- Reads the YAML file and processes any `!include` tags recursively.
- Collects all `variables:` blocks found anywhere in the document tree (including from included files) into a single global variable table. The main file's `variables:` block takes precedence over variables from included files on key conflicts.
- Strict validation: Checks the YAML against the loaded provider schemas `schemas/*.json` to ensure all required fields are present.
- Merges variables from the global variable table into the configuration.
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
  iam.disableServiceAccountKeyCreation {
    "import-id" = "organizations/12345/policies/iam.disableServiceAccountKeyCreation"
    spec { rules = [{ enforce = "TRUE" }] }
  }
}
```

**How it works:**
- **`"import-id" = "<ID>"`**: Provide the full GCP resource ID.
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
they must be *activated*, then *imported as-is*, and only then *modified*. The
`!import-include` directive automates exactly that during `transpile`; three read-only CLI
commands (`export`, `diff`, `report`) round out the workflow.

All Org Policy API calls authenticate via Application Default Credentials (same as
`bootstrap`) and send your quota project as `x-goog-user-project` (resolved from
`GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT` or the ADC file's `quota_project_id`).
Run `gcloud auth application-default login` and set a quota project first.

#### Activate + import via `!import-include` (the main workflow)

To bring a preset's policies under management — activating managed constraints and
importing existing ones into state, with **no console clicking and no `import-id`
editing** — change the one include line in your main config from `!include` to
`!import-include`:

```yaml
# C01234567.yaml
org_policy_policy: !import-include presets/CIS-GCP-Foundation-4.0.satz
```

Then run the normal transpile and apply:

```bash
satz transpile C0example.satz --config config.toml   # renders HCL, then:
                                                         #   - activates managed constraints (API)
                                                         #   - tofu import of existing policies into state
cd hcl && tofu apply                                     # rolls out spec changes / creates the rest
```

What `transpile` does when it sees `!import-include`:
1. Renders the HCL exactly as a plain `!include` would (the policies become normal
   `google_org_policy_policy` resources).
2. Runs `tofu init`, then for each included policy: if it is **managed and missing**, it is
   activated via the Org Policy API so it becomes importable; every policy that **exists**
   live is `tofu import`ed into state. Non-managed policies that don't exist yet are left
   for your `tofu apply` to create.

It is idempotent (re-running is safe). Once imported, change `!import-include` back to plain
`!include` — subsequent transpiles are pure local codegen with no GCP/tofu side effects.
A plain `!include` never triggers any of this.

See [What `!import-include` supports](#what-import-include-supports) for the other resource
kinds it handles.

#### Export current state (`export-organizational-policies`)

Snapshot the live policies into a re-importable preset (same schema the transpiler
consumes):

```bash
satz export-organizational-policies C01234567.yaml --customer-organization-id 123456789012
# -> yaml/<customer-id>-orgpolicies.yaml
```

#### Diff desired vs. live (`diff-organizational-policies`)

```bash
# Compare a preset against live:
satz diff-organizational-policies C01234567.yaml \
  --preset presets/CIS-GCP-Foundation-4.0.satz --format markdown --report diff.md

# Or, with no --preset, compare everything the config declares against live:
satz diff-organizational-policies C01234567.yaml
```

Each constraint is classified: `MISSING (needs activation)` (managed), `MISSING
(creatable)`, `MATCHES`, `DIFFERS`, or `CURRENT-ONLY`. The diff is semantic — it normalizes
`enforce: "TRUE"` vs `true`, `allowed_values` ordering, and `parameters` JSON-string vs
object so it doesn't report false changes.

#### Report with explanatory text (`report-organizational-policies`)

```bash
satz report-organizational-policies C01234567.yaml --scope full --format markdown
```

`--scope`: `active` (set policies), `inactive` (available but unset), or `full` (both,
with constraint descriptions pulled from the Org Policy constraints API). `--format pdf`
converts the markdown via `pandoc` if it is on `PATH` (otherwise the markdown is kept).

### What `!import-include` supports

`!import-include` is not org-policy-specific. **The YAML key the preset is included under
decides which live lookup runs**, so one directive covers several resource kinds:

| Include key | What `transpile` does | Import id used |
|---|---|---|
| `org_policy_policy` (or `google_org_policy_policy`) | Activates missing **managed** constraints, imports every policy that exists live | `organizations/<id>/policies/<constraint>` |
| `cloud_identity_group` | Looks each group up by email over the Cloud Identity API, imports the ones that exist, then imports the **declared** memberships of those groups | `groups/<id>`, `groups/<id>/memberships/<id>` |
| anything else / no key | Treated as org policies (the directive's original meaning) | — |

Both spellings of the directive work and mean the same thing:

```yaml
cloud_identity_group: !import-include presets/security-group-models/s1-group-definitions.yaml

# or, indented under the key:
cloud_identity_group:
  !import-include presets/security-group-models/s1-group-definitions.yaml
```

#### Cloud Identity groups

```bash
satz transpile C0example.satz --config config.toml
# !import-include: importing Cloud Identity groups from 1 preset(s)...
# !import-include: groups imported=3, create-on-apply=2, skipped=0;
#                  memberships imported=5, create-on-apply=1, skipped=0.
cd hcl && tofu apply    # creates the groups and members that did not exist yet
```

Each group's key is derived exactly as the generated HCL derives it — an explicit `id`,
else an explicit `email`, else `<yaml-key>@<customer-domain>` — and looked up with
`cloudidentity.googleapis.com/v1/groups:lookup`. Groups that exist are `tofu import`ed;
groups that do not are left for `tofu apply`. Re-running is safe.

##### Memberships: only the ones you declared

For every group that already exists, each `member` / `manager` / `owner` entry **in your
config** is looked up with `memberships:lookup` and imported if that person is already in
the group. Nothing else is touched:

| Situation | Result |
|---|---|
| Declared in YAML, already in the group | Imported — `apply` sees no change |
| Declared in YAML, not in the group | Created by `apply` |
| **In the group but not in your YAML** | **Left alone** — not in state, never proposed for deletion |

That last row is the point. Importing *every* live member would put people your config
never mentions under Terraform's control, and the next `apply` would propose removing them.
Importing only the declared ones keeps the group's other members entirely out of scope.

Requirements: `cloudidentity.googleapis.com` enabled (`bootstrap` already enables it) and a
principal that may read groups (`roles/cloudidentity.groups.readonly` or Workspace Groups
Reader). If `groups:lookup` is denied — some tenants answer `403` even for a group that
simply does not exist — the tool falls back to listing `customers/<customer-id>` once and
answers from that. If both are denied it warns, counts the group as `skipped`, and carries
on rather than aborting a transpile whose HCL is already written.

> [!NOTE]
> An imported group always shows a diff on `initial_group_config`, because the live value
> cannot be read back. That is what the `lifecycle.ignore_changes` block in the shipped
> group presets is for.

> [!NOTE]
> The long `google_cloud_identity_group:` form renders through the generic schema path with
> raw Terraform attributes, which the group importer cannot address. Using
> `!import-include` with it is a hard error pointing you at the compact form above.

Preset paths in a `!import-include` resolve like any other include — relative to the
including file, then along `include_dirs` — **not** against `presets_dir`. With the default
layout those coincide; if you point `presets_dir` somewhere else, add it to `include_dirs`
too.

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

```yaml
# logging-project.yaml — one file, one concern
project:
  logging-prj:
    project_id: !format ["{}-logging", *customer-prefix]
organization_iam_member:
  "group:log-admins@example.com":
    - roles/logging.admin
cloud_identity_group:
  log-admins:
    display_name: Log Admins
```

```yaml
# main config — the hierarchy decides where the project lands
folder:
  shared-services:
    display_name: "Shared Services"
    !include logging-project.yaml
```

The project is created in `shared-services`; the group and the grant are hoisted to their
intrinsic scopes. Placement is always **by include position** — fragments never name their
parent folder, so the main file stays the single source of hierarchy truth.

Merge rules when several fragments declare the same thing:

- **IAM grants are additive.** Same member from two fragments → role lists union; a
  deep-equal (member, role, condition) entry is deduped to one resource. There is no
  conflict state.
- **Groups must agree.** The same group key with a deep-equal body is deduped (including
  the same fragment twice is idempotent); with a *different* body the transpile aborts
  before writing any file: `cloud_identity_group 'log-admins' is defined differently at
  folder 'observability' and at folder 'shared_services'`.
- Hoisted output is sorted, so moving a fragment between folders does not churn the
  generated HCL.

Project- and folder-scoped IAM types (`project_iam_member`, `folder_iam_member`) are **not**
hoisted — their position in the tree is their parent, as before.

Every other resource type is position-independent whenever its parent is explicit in the
YAML (`org_id:`, `parent:`, `billing_account:`), so it needs no hoisting. What all types
get instead is the **duplicate-address guard**: a Terraform address may be emitted once.
Byte-identical duplicate definitions — the same "highlander" resource (org audit config,
sink, contact) included from several fragments — collapse to a single emission with a
printed note; the same address with *different* content aborts the transpile, naming the
address and the first differing line. Attribute-level merging is deliberately not
attempted — it would only hand the same conflict one recursion level down.

**Cross-file resource-key merging:** two Form A includes (or the main file and an
include) may declare the *same top-level resource-type key* — the entries merge id by
id: distinct ids union, a deep-equal id collapses with a printed note, the same id with
different content aborts naming the entry. This is how e.g. the audit-logsink preset and
the CIS central monitoring preset each declare their own
`google_logging_organization_sink:` and coexist at root level. Two conditions: the key
must be a resource type known to the provider schemas (schema-driven, no name
heuristics — without schemas, `satz update-schema`, the strict duplicate-key error
remains), and structural keys (`folder`, `project`, `terraform`, `providers`) never
merge. Merging steps into the ids, deliberately not into attributes — attribute-level
merging would only hand the same conflict one recursion level down.

### Resource Lifecycle

Any resource may declare a `lifecycle` block, which is rendered as a top-level
[`lifecycle` meta-argument](https://developer.hashicorp.com/terraform/language/meta-arguments/lifecycle) block in the generated HCL:

```yaml
google_cloud_identity_group:
  my_group:
    display_name: "My Group"
    initial_group_config: "EMPTY"
    lifecycle:
      ignore_changes:
        - initial_group_config
      prevent_destroy: true
```

Generates:

```hcl
resource "google_cloud_identity_group" "my_group" {
  display_name         = "My Group"
  initial_group_config = "EMPTY"

  lifecycle {
    ignore_changes        = [initial_group_config]
    prevent_destroy       = true
  }
}
```

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
- `<INPUT>`: Name of the input YAML file.
- `--mode, -m <MODE>`: Target mode (`local` or `cloud`).

**Under the Hood:**
- **Update YAML**: Modifies the `deployment-mode` anchor in the source YAML file.
- **Regenerate**: Runs `transpile` to update the backend configuration (Local vs GCS) and provider authentication (ADC vs Impersonation).
- **Migrate State**: Executes `tofu init -migrate-state` to safely move your terraform state to the new backend.

### Infrastructure Discovery

`satz` provides two discovery commands to generate YAML configurations from existing infrastructure.

#### Discover from Terraform State (`discover-from-state`)
Read an existing Terraform/OpenTofu state and generate a corresponding YAML configuration.

```bash
satz discover-from-state --output discovered.yaml
```

**Parameters:**
- `--state-json <FILE>`: Path to Terraform state JSON file (optional). If omitted, runs `tofu show -json`.
- `--output, -o <FILE>`: Path to output YAML file (default: `discovered.yaml`).
- `--add-import-id`: Add `import-id` tag to every resource for declarative imports.
- `--add-import-id-as-comment`: Add `import-id` as a comment to every resource.
- `--discovery-config <FILE>`: Path to discovery configuration YAML file (default: `presets/discovery-config.yaml`).

**Under the Hood:**
- Reads the current state (either from a file or by running `tofu show -json`).
- Reverse-engineers the resources to match the `satz` YAML structure.
- **Configurable Filtering**: respects `presets/discovery-config.yaml` to include/exclude specific resources and attributes.
  - Resource types can be globally enabled/disabled (`import: true/false`).
  - Specific attributes can be filtered via `exclude` and `include` lists per resource.
- **Schema Validation**: Automatically validates discovered data against the Terraform Provider Schema, dropping read-only or computed fields that would cause HCL generation errors.
- **IAM Heuristics**: Intelligently maps complex IAM resources (like `google_storage_bucket_iam_member`) to simplified, project-nested YAML structures.

#### Discover from GCP Organization (`discover-from-organization`)
Discover infrastructure directly from a GCP Organization using the Cloud Asset API and generate a YAML configuration.

```bash
satz discover-from-organization --customer-organization-id "123456789012" --output discovered.yaml
```

**Parameters:**
- `--customer-organization-id <ID>`: Numeric GCP Organization ID (required).
- `--output, -o <FILE>`: Path to output YAML file (default: `discovered.yaml`).
- `--add-import-id`: Add `import-id` tag to every resource for declarative imports.
- `--add-import-id-as-comment`: Add `import-id` as a comment to every resource.
- `--discovery-config <FILE>`: Path to discovery configuration YAML file (default: `presets/discovery-config.yaml`).

**Under the Hood:**
- Uses Google Cloud Asset API to enumerate all resources in the organization.
- Requires appropriate IAM permissions (`cloudasset.assets.searchAllResources`).
- Applies the same filtering and validation as `discover-from-state`.
- Useful for discovering infrastructure that isn't managed by Terraform/OpenTofu yet.

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
present in the transpiled estate), **◐ partial** (witnesses present but manual duties
open, or only `contributes` claims), **⚠ deviation** (the estate deliberately does not
meet this control and says why — see below), **✗ unmet** (with the packs that would
provide it — remediation as suggestion), **‼ broken claim** (a pack claims witnesses the
estate does not emit — reported loudly, never silently satisfied), **○ organizational**
(no IaC witness possible). Exit code is non-zero on unmet/broken, so it gates CI —
deviations are disclosed decisions and do not fail it.

### The Satz language

Full specification, grammar and lookup: **`docs/satz-language.md`** — derived from
the parser (`crates/satz-core/src/satz.rs`), with every example verified to compile.

### Adopting existing org policies (brownfield)

A first `apply` against an organisation that already has org policies fails one
policy at a time with `Error 409: A Policy of constraint ... already exists`. Worse,
GCP **managed** constraints cannot be imported at all until the organisation has
activated them. Both are one command:

```bash
satz adopt-org-policies C0example1.satz --dry-run   # show the plan
satz adopt-org-policies C0example1.satz             # activate + import
```

It reads the policies out of the emitted `main.tf` — the same list `apply` acts on —
then for each: activates a managed constraint the org has never had (using the
enforcement the estate declares, so the following apply is a no-op), imports anything
that exists, and leaves a not-yet-existing legacy constraint for `apply` to create.
Imports are idempotent; re-running is safe.

It is a separate command on purpose. It makes live API calls and activates
constraints, so it is never a side effect of `transpile`, which stays pure.

### Running from anywhere

`--config` takes the `config.toml` **or** the estate directory that holds it, and every
path inside the config resolves against the config's own directory — so any command runs
from any working directory:

```bash
satz transpile C0example1.satz --config ~/estates/acme
satz require cis-gcp-4.0 C0example1.satz --config ~/estates/acme
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
(roadmap). Catalogs carry no framework text (CIS/ISO prose is license-restricted),
only IDs and paraphrases.

### Evidence report (`report-compliance`)

The goal view joined with the **live estate**: every witness of a satisfied/partial
control is verified against Cloud Asset Inventory (org sinks, log metrics, alert
policies, notification channels, buckets — matched by name/display name extracted from
the generated HCL). Manual duties merge with `attestations.yaml` beside config.toml
(`duty-id: {by, date, note}`), and a Prowler native-JSON export can be ingested as
corroboration (`--prowler findings.json`).

```bash
satz report-compliance cis-gcp-4.0 C0example.satz            # markdown + history
satz report-compliance cis-gcp-4.0 C0example.satz --format pdf --prowler prowler.json
```

Row statuses: **verified** (all witnesses live), `verified*` (some witness types have
no live check yet — stated, never faked), **DRIFTED** (declared but not live),
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
| `X.satz` (+ generated `.yaml` twin) | upstream-owned, pristine | always overwritable |
| `X.local.satz` | your fork — the *rename is the fork declaration* | **never touched** |
| `X.diff.satz` | the CURRENT adoption delta: `diff(X.local, pristine X)` | rewritten on every run |
| `<own>.satz` | no upstream counterpart | local-only, kept |

Pack **versions live inside the file** (`pack <name> version "1.2"`); filenames
carry only framework versions (CIS-GCP-Foundation-**4.0**, catalogs). Never a
`X.local.<n>.satz`, never more than one diff per pack — history lives in git.

**A preset that your estate includes never changes silently.** When upstream's
version differs *semantically* (the compiled canonical YAML — comment and
formatting churn upgrades silently), merge-presets:

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

Presets actually used by `<INPUT>` (via `use`, or any `!include` form in a YAML estate) are tagged
`[included]`; drift in an included preset makes the command exit non-zero, so it can
gate CI. `use … when` packs whose condition is off count as not included.

### Self-update (`self-update`)
Check for and install a new release from GitHub. After a successful install, the tool downloads the release README and prints its full path, then opens it unless you pass the options below.

```bash
# Check for and install a new release (same installer as curl)
satz self-update

# Only check if an update is available (no install, no README)
satz self-update --check-only

# Skip downloading README after install, or skip opening it
satz self-update --no-download-readme
satz self-update --no-download-readme --no-open-readme
```

**Self-update options:** `--no-download-readme`, `--no-open-readme`, `--check-only`, `--skip-checksum`. The program can also check for updates automatically when you run other commands; this is controlled by the [user settings](#user-settings-configsatzsatztoml) file `~/.config/satz/satz.toml` (`self_update_frequency`: `never`, `always`, or `daily`).

**Under the Hood:**
- Fetches the latest release from the GitHub API and compares versions. When a newer version is available it downloads `satz-installer.sh` and `satz-installer.sh.sha256` from that same release, verifies the SHA-256 digest, and only then runs the installer. A checksum mismatch aborts; a release without the sidecar aborts too, unless you pass `--skip-checksum`. On success, optionally downloads `README.md` from the repo and prints its path (e.g. `README: /Users/you/Downloads/satz-0.4.9-README.md`).

### Open README (`open-readme`)
Download the latest `README.md` from the main branch and open it with your configured editor (see [user settings](#user-settings-configsatzsatztoml)).

```bash
satz open-readme
```

The README is saved to your Downloads folder (e.g. `~/Downloads/satz-latest-README.md`). The editor used follows the same priority as all file-open operations: `preferred_editor` in `~/.config/satz/satz.toml` → `$EDITOR` env var → OS default app.

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

### Set Preferred Editor (`set-preferred-editor`)
Set, clear, or show the `preferred_editor` option in `~/.config/satz/satz.toml` without editing the file manually.

```bash
# Set the editor
satz set-preferred-editor code
satz set-preferred-editor zed
satz set-preferred-editor /usr/local/bin/vim

# Clear the setting (fall back to $EDITOR / OS default)
satz set-preferred-editor --clear

# Show the current setting
satz set-preferred-editor
```

The editor is used when opening files from `open-readme` and `self-update` (post-install README). The priority chain is: `preferred_editor` config → `$EDITOR` env var → OS default app.

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
The `bootstrap` command automates the entire process: creating the infrastructure folder, project, bucket, linking billing, enabling foundation APIs (fixing the "chicken-and-egg" problem), and initializing the state.

```bash
satz bootstrap C0example.satz
```

**What this does:**
- Creates Folder, Project, Bucket, Service Account.
- Enables Service Usage, IAM, and other core APIs.
- Assigns `Folder Admin` to the user executing the bootstrap (if missing).
- Automatically runs `transpile`, `init`, and `import` to bring resources under Terraform management.

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
satz migrate C01234567.yaml --mode cloud
```
The tool automatically updates the YAML, switches to **Service Account Impersonation**, and runs `tofu init -migrate-state`.

#### 2. Verification
In `cloud` mode, verify that you can plan/apply using the restricted service account identity:
```bash
tofu plan
```

#### Template Variables Reference

When you run `init`, the following variables are generated in the template:

| Variable | Default | Description |
|----------|---------|-------------|
| `infra-folder-name` | `Infrastructure` | Display name for the top-level folder. Leave `""` to create the project in the root. |
| `infra-project-name` | `""` | The unique ID for the management (IaC) project. |
| `infra-bucket-name` | `""` | The name of the GCS bucket for Terraform state. |
| `customer-id` | (from CLI) | The Workspace Organization ID (e.g., `C01234567...`). |
| `customer-organization-id` | `"123456789012"` | The numeric Google Cloud Organization ID. **Note:** Always use quotes, otherwise YAML interprets this as a number. |
| `customer-domain` | `""` | The customer's primary domain (e.g., `example.com`). |
| `customer-longname` | `""` | The full legal name of the customer entity. |
| `customer-shortname` | `""` | A unique slug or shortname for the customer. |
| `svc-iac-account` | `svc-iac-001` | The name/ID of the primary IaC Service Account. |
| `svc-iac-users-group` | `svc-iac-users` | The Cloud Identity group for IaC administrators. |
| `billing-account-infra` | `""` | The Billing Account ID (e.g., `A12345-B67890-C12345`). |
| `deployment-engine` | `tofu` | The IaC tool: `tofu` or `terraform`. |
| `deployment-mode` | `local` | `local` for Day 0 (User ADC); `cloud` for Day 1+ (Impersonation). |
| `default-region` | `europe-west3` | Default region for regional resources. |
| `default-zone` | `europe-west3-a` | Default zone for zonal resources. |

### 3. Transpile
Compile an estate to HCL. Run this from within the customer repository directory.
```bash
satz transpile my-infra.satz
```
- Input is read from `yaml_dir` (e.g., `./yaml/my-infra.satz`).
- Output is written directly to the `hcl_dir` defined in your config.
- **Run from anywhere**: All paths are resolved relative to the configuration file's directory.
- **Automatic Schema Sync**: The tool will automatically fetch missing provider schemas via `tofu/terraform` during transpilation.

## The legacy YAML dialect

Everything in this chapter and in "YAML dialect features" below describes the **legacy `.yaml` input**, which `transpile` and `migrate-to-satz` still accept. New estates are written in Satz — see [docs/satz-language.md](docs/satz-language.md). The input file is the source of truth for your infrastructure.

### Terraform & Backend
The `terraform` block is mandatory and used primarily for backend configuration.

```yaml
terraform:
  backend:
    gcs:
      bucket: "my-infra-bucket"
      prefix: "project-a"
```

### Providers
Define one or more provider instances.

```yaml
providers:
  google:
    region: "europe-west3"
    zone: "europe-west3-a"
  google: # Support for multiple aliased providers
    - alias: "secondary"
      region: "us-central1"
```

### Variables
Declare variables in a `variables` block. They are automatically merged to the root context and can be referenced anywhere in the file with YAML anchors.

```yaml
variables:
  customer-id: &customer-id "C34projectroot"
  region: &region "europe-west3"

google_project:
  my-project:
    project_id: *customer-id
```
- Variables are declared as `string` types in `_variables.tf`.
- Values are written to `.tfvars`.

#### Variables in Included Files

`variables:` blocks defined inside included files are merged into the same global variable table. This works for both include forms:

```yaml
# shared-vars.yaml — a standalone include (Form A)
variables:
  shared-region: &shared-region "europe-west3"
  shared-project: &shared-project "my-infra-project"
```

```yaml
# main.yaml
!include shared-vars.yaml

variables:
  customer-id: &customer-id "C01234567"  # overrides any same-named key from includes

google_project:
  my-project:
    project_id: *shared-project   # resolved from shared-vars.yaml
    region: *shared-region
```

**Priority rules:**
- The main file's `variables:` block has the **highest** priority.
- Variables from Form A included files (inserted at the root level) have medium priority.
- Variables from Form B included files (`key: !include file.yaml`, nested under a key) have the lowest priority.
- On key conflicts, shallower (closer to root) definitions always win.

Use `--print-variables` to inspect the resolved variable table after a transpile:

```bash
satz transpile my-infra.yaml --print-variables
```

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
| `presets_dir` | `"presets"` | Preset library downloaded by `get-presets`; `--preset` and the discovery-config default resolve here |
| `include_dirs` | `[".", "yaml"]` | Search paths for `use` / `!include` files |
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
[docs/satz-language.md](docs/satz-language.md). Params are lexically scoped declarations
(no anchors), `"{param}"` interpolates (no `!format`), `use "pack.satz" [as key] [when param]`
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
interpolates params. `require` and `report-compliance` accept only `.satz`; `transpile`
also accepts the legacy YAML dialect, and `migrate-to-satz` converts it with a proof that
the transpiled output is identical.

## YAML dialect features (legacy input)

### Custom YAML Tags
Enhance your configuration with dynamic logic:
- **`!include <file>`**: Recursively include other YAML snippets. Two forms are supported:
  - **Form A** — standalone, inserts content at the same level: `!include shared.yaml`
  - **Form B** — under a key, inserts content indented under that key: `defaults: !include defaults.yaml`

  `variables:` blocks in included files are automatically hoisted into the global variable table regardless of which form is used. See [Variables in Included Files](#variables-in-included-files) for details.

  Included files' `variables:` blocks provide **overridable defaults** — see
  [Overriding variable defaults](#overriding-variable-defaults) for the full rules.
- **`!include-if <anchor> <file>`**: include the file only if `<anchor>` is defined
  earlier in the document (the sigil is optional: `!include-if logsink-project-name …`
  and `!include-if *logsink-project-name …` are equivalent). When the anchor is not
  defined, the line is replaced by a `# satz:skipped:` marker and the file does not
  even need to exist. This turns a main config into a template whose optional parts
  activate by defining a single variable:

  ```yaml
  # Pulls in the full audit-logsink stack only when a logsink project is named:
  !include-if logsink-project-name presets/monitoring/organization-audit-logsink.yaml
  ```
- **`!format [template, arg1, arg2]`**: Dynamic string formatting using placeholders (`{}`).
  ```yaml
  member: !format
    - "serviceAccount:svc-iac-001@{}.iam.gserviceaccount.com"
    - *infra-project-name
  ```
- **`!join [arg1, arg2, ...]`**: Concatenate multiple values into a single string.
- **`!expr <expression>`**: Emit a Terraform expression reference instead of a literal string. See [Expression References](#expression-references-expr) below.

### Overriding variable defaults

An included file's (typically a preset's) top-level `variables:` block provides
**defaults**. The including file overrides one by defining the **same anchor name**
*before* the include line — first definition wins:

```yaml
# main.yaml
variables:
  customer-shortname: &customer-shortname "acme"
  # override — same anchor name the preset uses:
  logsink-bucket-location: &logsink-bucket-location "europe-west1"

!include presets/monitoring/organization-audit-logsink.yaml
```

When the anchor is already defined above the include, the preset's own redefinition is
stripped during include expansion, so every alias inside the preset resolves to your
value. Undefined anchors keep the preset's default. Each variable is independent —
override two, leave the rest on defaults. (Plain YAML would do the opposite: aliases
bind to the *nearest preceding* anchor, which would make preset defaults
unoverridable.)

**The rules:**

1. **Position** — the override must be textually *above* the `!include` line, and any
   anchors it references must be defined above *it* (YAML aliases only look backward).
   One `variables:` block at the top of the main file, base values first, derived
   values after, includes below, satisfies both.
2. **Exact anchor name** — the same name the preset uses; each preset's names are
   listed in [presets/README.md](presets/README.md).
3. **Full anchor syntax** — `key: &key value`. A plain `key: value` line defines no
   anchor and overrides nothing.

**Override values may be composed** from other anchors with `!format`:

```yaml
variables:
  customer-shortname: &customer-shortname "acme"
  logsink-bucket-name: &logsink-bucket-name !format ["{}-org-audit-logs", *customer-shortname]
```

**Renaming a variable (anchor-of-an-alias) needs the identity wrapper.** This is
invalid YAML — the spec forbids putting an anchor on an alias node, and every parser
rejects it:

```yaml
variables:
  cis-bucket-project: &cis-bucket-project *logsink-project-name     # ✗ parse error
```

Wrap the alias in a single-argument `!format` instead. The tagged node may legally
carry the anchor, and `{}` with one argument passes the value through unchanged:

```yaml
variables:
  cis-bucket-project: &cis-bucket-project !format ["{}", *logsink-project-name]   # ✓
```

No wrapper is needed for plain *use* of a variable (`project: *logsink-project-name`)
or for a same-name override with a literal value — only for defining a **new** anchor
whose value is another anchor.

### Expression References (`!expr`)

YAML values are literals: `member: google_service_account.x.member` puts that exact
*text* into the generated HCL, and Terraform will try to bind the IAM role to a
"user" of that name. When you need a **reference** to another resource — so Terraform
resolves the real value at apply time *and orders the operations accordingly* — mark
the value with `!expr`:

```yaml
member: !expr google_service_account.otel_collector.member
```

renders as

```hcl
member = "${google_service_account.otel_collector.member}"
```

which is semantically the bare expression: Terraform sees the dependency and creates
the service account before the binding.

**As a mapping key.** The compact IAM style keys on the member, and the member is often
exactly the thing that must be a reference. `!expr` works in key position:

```yaml
google_project_service_identity:
  pubsub:
    provider: google-beta
    service: pubsub.googleapis.com

google_project_iam_member:
  !expr 'google_project_service_identity.pubsub.member':
    - roles/bigquery.dataEditor
    - roles/pubsub.publisher
```

Each role explodes into its own `google_project_iam_member` resource whose `member`
references the service identity — guaranteed to exist before the bindings are applied.
(Quote the expression when it is used as a key, as above: a bare `key:` containing
dots is fine for YAML, but the quotes keep the `:` after the tag unambiguous.)

**Choosing between the three string mechanisms:**

| You want | Use | Renders as |
|---|---|---|
| Plain text assembled from variables | `!format ["user:{}@{}", *first-admin, *customer-domain]` | `"user:alice@example.com"` — a literal; no dependency |
| A reference to another resource's attribute | `!expr google_service_account.x.member` | `"${google_service_account.x.member}"` — dependency tracked |
| Text and references mixed | a plain string with `${...}` inside: `"serviceAccount:${google_service_account.x.email}"` | kept as-is — dependency tracked |

Rules of thumb:

- `!format` resolves at **transpile time** from your `variables:` — the output is fixed text.
  If the value only exists after `tofu apply` (emails of created service accounts, service
  identity members, project numbers), it must be `!expr` or a `${...}` string.
- `!expr` and `${...}` strings are equivalent; `!expr` is the explicit form for a bare
  reference, `${...}` embeds references inside longer text. Input to `!expr` that already
  contains `${...}` is passed through unchanged, never double-wrapped.
- The result is always string-typed in HCL — which is what members, names and ids expect.

### Conditional Folding
Setting a folder's `display_name` to an empty string (`""`) will skip the `google_folder` resource and "implode" its contents into the parent context. This is useful for conditionally creating folders based on variables.

### Compact Explosion (CEX)
Resources named with a `CEX_` prefix (or listed in `auto_explode`) support compact definition styles:
- **IAM**: Define many roles for one member in a simple block.
- **Services**: Enable lists of GCP services in one block.

## Core Principles

The tool follows a central design philosophy based on **Hierarchy Context**, **Attribute Inheritance**, and **Strict Validation**.

### 1. Hierarchy Context & Nesting
Resources are defined within the context of their parent in the organization hierarchy:
- **Project Context**: Resources that require a project (e.g., Buckets, VMs, Networks) are usually nested directly within a `google_project` definition.
- **Folder Context**: Resources belonging to a folder (e.g., Folder IAM members) are usually nested within a `google_folder` block.
- **Organization Context**: Organization-wide resources (e.g., Group memberships, Org IAM) are defined at the root level of the YAML.
- **Explicit Placement**: Any resource can be defined outside its logical hierarchy container if the identifying parameter (e.g., `project_id`, `folder`) is provided explicitly.

### 2. Attribute Inheritance (Narrowest Context)
Nested resources automatically inherit identity attributes from their surrounding context if not explicitly defined:
- **Automatic Matching**: The tool identifies which identifier a resource needs based on its schema (e.g., `project_id`, `project`, `folder_id`, `org_id`).
- **Inheritance**:
    - A resource inside a Project context inherits the Project ID.
    - A resource inside a Folder context inherits the Folder ID.
- **Narrowest First**: If a resource is defined in a scope where multiple contexts apply (e.g., inside a Project which is inside a Folder), it inherits from the **most specific (narrowest)** context available.
- **Explicit Override**: Explicitly provided attributes in the YAML always take precedence over inherited context values.

### 3. Context Validation & Typo Detection
To ensure configuration correctness, nested blocks are strictly validated:
- **Attribute vs. Resource**: Every key within a `Project` or `Folder` block must be either:
    - A valid native attribute/block of the parent resource (e.g., `name` for a project).
    - A valid resource type from the cloud provider schema.
- **Error Detection**: Any key that is neither a known attribute nor a known resource type is treated as a typo and triggers a **Warning**.
- **Missing Context**: Resources that require a project or folder identifier but are defined outside such a context (without an explicit identifier provided) will trigger a **Warning**.

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
satz discover-from-state --state-json state.json --output yaml/migration-discovery.yaml
```

Alternatively, if you want to discover infrastructure directly from GCP without Terraform state:
```bash
satz discover-from-organization --customer-organization-id "123456789012" --output yaml/migration-discovery.yaml
```

### 2. Hierarchical Refinement
The discovery tool produces a relatively flat YAML-dialect file. Convert it with `satz migrate-to-satz migration-discovery.yaml`, then organize the result into the `satz` hierarchical format:
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
cargo test                                         # run unit tests
cargo fmt && cargo clippy                          # format + lint
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

`satz` compiles Satz estates (and, for `transpile`, the legacy YAML dialect) into production-ready OpenTofu/Terraform HCL. It prioritizes structure, inheritance, and validation.

### Core Components

#### 1. Transpiler (`src/transpiler.rs`)
The heart of the tool. It processes the YAML tree and generates `main.tf`, `providers.tf`, `variables.tf`, and `terraform.tfvars`.
- **Context Awareness**: Tracks the current Organization, Folder, and Project context as it descends the YAML tree.
- **Attribute Inheritance**: Automatically injects identifiers (like `project_id`) into nested resources based on the closest parent context.
- **Conditional Folding**: Implements "implosion" logic where folders with empty display names are skipped, promoting their children to the parent context.

#### 2. Schema Registry (`src/schema.rs`)
Manages Terraform provider schemas (loaded as JSON).
- **Validation**: Ensures that all required attributes and blocks are present in the YAML.
- **Translation**: Maps YAML keys to correct HCL resource types (e.g., automatically adding the `google_` prefix).

#### 3. Template Generator (`src/template.rs`)
Provides a consistent starting point for new customer rollouts.
- **Declarative Bootstrap**: Generates a YAML structure representing the Day 0 infrastructure (Project, Services, Bucket, SA).

#### 4. Custom YAML Processing (`src/main.rs`, `src/include_processor.rs`)
Implements custom tags to extend YAML's expressiveness:
- `!include`: Recursive file inclusion.
- `!format`: Placeholder-based string construction.
- `!join`: String concatenation.
- `!expr`: Terraform expression references (resolved to `"${...}"` interpolation strings).
- `!import-include`: like `!include`, but at transpile time it also looks the included resources up live and `tofu import`s the ones that already exist. Recorded in an include manifest together with the YAML key it sits under, and that key selects the importer — org policies or Cloud Identity groups (see [What `!import-include` supports](#what-import-include-supports)). Plain `!include` has no side effects.

#### 5. Discovery Engine
The `discover` commands reverse-engineer YAML from existing Google Cloud assets.
- **Asset Ingestion**: Consumes CAI (Cloud Asset Inventory) export streams.
- **Configurable Filtering**: Uses `discovery-config.yaml` to include/exclude resources and attribute fields.
- **Schema Validation**: Validates discovered data against Terraform schemas, automatically filtering read-only or computed fields to ensure valid HCL generation.
- **Heuristics**: Intelligent mapping of IAM policies (e.g., `google_storage_bucket_iam_member`) and key generation.

#### 6. Organization Policy Engine (`src/org_policy.rs`)
Aligns curated Org Policy sets (e.g. `presets/CIS-GCP-Foundation-4.0.satz`) with the live organization via the GCP Org Policy API v2 (ADC auth, reusing the `bootstrap` pattern).
- **`!import-include` (transpile-time)**: after the HCL is rendered, `transpile` activates managed constraints that are missing (API create), then `tofu import`s the existing policies into state — no manual console activation and no `import-id` editing. The user then runs `tofu apply` and flips the directive back to `!include`.
- **CLI commands**: `export-organizational-policies` (snapshot live state to a re-importable preset), `diff-organizational-policies` (semantic current-vs-desired report), `report-organizational-policies` (markdown/JSON/PDF inventory with constraint descriptions).
- **Managed constraints**: constraints whose name contains `.managed.` must be *activated* (API create), then *imported as-is* (`tofu import`), then *modified* (`tofu apply`). `!import-include` sequences the activate+import; `tofu apply` does the modify.
- **Pure diff core**: classification + `normalize_spec` are IO-free and unit-tested; they reconcile `enforce "TRUE"`↔`true`, `allowed_values` ordering, and `parameters` JSON-string↔object so semantically-equal policies don't show as diffs.

#### 7. Cloud Identity Group Import (`src/cloud_identity.rs`)
The second `!import-include` target. A groups preset declares groups by name; adopting the ones that already exist needs their opaque `groups/<id>`, which used to be pasted in by hand.
- **Lookup, not guesswork**: each group's key is derived by the *same* helpers the transpiler emits HCL with (`group_email` / `group_resource_address` in `src/transpiler.rs`), then resolved via `cloudidentity.googleapis.com/v1/groups:lookup`. Existing groups are `tofu import`ed; missing ones are left for `tofu apply`.
- **403 is ambiguous**: some tenants return it for a nonexistent group as well as for a permission problem, so a denied lookup falls back to listing `customers/<customer-id>` once. If that fails too the group is counted `skipped` with an actionable hint rather than aborting. Membership lookups use the same fallback, per group.
- **Declared memberships only**: for a group that exists, each `member`/`manager`/`owner` entry in the config is resolved with `memberships:lookup` and imported if present. Live members the config does not mention are never looked at, so adopting a group cannot make `apply` propose deleting somebody. The membership label is a `DefaultHasher` digest of `(group key, raw member string)`; the importer computes it through the same `membership_resource_label` helper the transpiler emits with, and `membership_label_helper_matches_emitted_hcl` pins the two together.
- **Quota project**: every Org Policy API request sends `x-goog-user-project`, resolved from `GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT` or the ADC file's `quota_project_id`.

### Bootstrap Workflow (Declarative Tofu)
Instead of hardcoded setup scripts, `satz` uses a two-phase Tofu approach:
1. **Local Phase**: `deployment-mode: local`. Runs under User ADC. Creates the management project and initial Service Account.
2. **Cloud Phase**: `deployment-mode: cloud`. Uses Service Account impersonation and a GCS backend for all subsequent operations.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
