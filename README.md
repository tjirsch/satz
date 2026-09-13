# satz

satz compiles an estate — written in a language whose resource types and attributes are the Terraform provider's — to OpenTofu/Terraform HCL, and verifies the controls it declares against the live Google Cloud organisation.
It also bootstraps a new Google Cloud organization, imports an existing one from Terraform state or from the live organization, and migrates Terraform state between backends.

> **📖 Documentation: <https://tjirsch.github.io/satz/>** — this README, the
> [language reference](https://tjirsch.github.io/satz/docs/language.html) and the
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

Two different files are involved:

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

**`--config` takes a path; the positional takes a bare filename inside
`yaml_dir` — or any existing path, which is taken as given** (so
`satz transpile yaml/C01.satz` works; if a file of the same relative path also
exists inside `yaml_dir`, the current-directory one wins and the shadowing is
named).

Two forms that do not work:

| Command | What happens |
|---|---|
| `satz bootstrap C01` | no extension is appended → looks for `yaml/C01` |
| `satz bootstrap C01.satz --config yaml/C01.satz` | the estate is parsed as TOML → `key with no value, expected =` |

### Global Options

These options can be placed anywhere in the command (e.g., before or after subcommands):

- `--config <FILE>`: Path to the **project** config file (`config.toml`, TOML — not the estate file). Mandatory for most commands if `config.toml` is not in the current directory. Every relative path inside it resolves from its own directory.
- `--validation <LEVEL>`: what a missing required argument does (`warn`, `error`, `none`; see [Schema Validation](#schema-validation)). Default from project config or `warn`.
- `--html-help`: open the documentation site in the browser at the invoked command's section (`satz transpile --html-help`); alone (`satz --html-help`) the front page. Commands without a section of their own open the command table.
- `--verbose`: Enable verbose output. When invoked without a subcommand (e.g. `satz --verbose`), prints full recursive help listing all subcommands and their options.
- `--no-actions`: never execute a declared [`action`](#run-actions-run-actions), whatever `run-actions` was asked to do.
- `--no-pack-actions`: consider only the estate's own actions; ignore any a `use`d pack declares.
- `--no-action-warnings`: silence the warning every declared action raises on a compile.
- `--no-impersonate`: run as the Application Default Credentials themselves, without becoming the estate's IaC service account — for a check that must answer as the human.

### User settings (~/.config/satz/satz.toml)

User-level **parameters** (e.g. when to check for updates) live in **`~/.config/satz/satz.toml`**. This file is **created on first run** with default values (e.g. `self_update_frequency = "always"`). If the file is missing on load, it is created with defaults.

| Option | Default | Description |
|--------|---------|-------------|
| `self_update_frequency` | `"always"` | When to check for updates on normal runs: `never`, `always`, or `daily` (at most once per 24 hours). The check only reports a newer version; it installs nothing. |

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

### From Source

Install directly with cargo:
```bash
cargo install --path .
```
This builds the release binary and installs it to `~/.cargo/bin` (no sudo required); ensure that directory is on your `PATH`.

## CLI Usage

All commands accept the [global options](#global-options) (`--config`, `--validation`, `--verbose`, and the three `--no-*action*` switches below). `satz <command> -h` is the one-line-per-option summary, `--help` the full text (both wrap to your terminal), `--html-help` opens the command's section on the documentation site. The groups below are the ones `satz --help` prints, in the same order:

**Estate**

| Command | Options / Arguments |
|---------|---------------------|
| `init` | `--defaults`, `--providers`, `--tf-tool`, `--customer-id`, `--customer-shortname`, `--billing-account-infra`, `--customer-organization-id`, `--customer-domain`, `--iac-user`, `--default-region`, `--infra-project-name`, `--infra-bucket-name`, `--from-live` (derive the missing values from the ADC alone) |
| `bootstrap <CONFIG_FILE>` | `--dry-run` (read-only incl. the permission pre-flight), `--greenfield` (materialize a not-yet-existing organization) |
| `transpile <INPUT>` | `--output`, `--schema-dir`, `--print-variables`, `--check` (compile in memory, write nothing), the first line of `main.tf` names the satz that emitted it, `--plan` / `--apply` (then run the tool in `hcl_dir`), `--scan` (then Checkov) |
| `import [SOURCE]` | `--from` (`state`\|`org`\|`yaml`\|`hcl`), `--all`, `--only <types>`, `--exclude <types>`, `--output` (default: `discovered.satz`), `--import-config`, `--into <estate>` (live: only the delta); yaml shape: `--kind`, `--gate`, `--fork`; hcl shape: `--wrap-all` |
| `adopt <INPUT>` | `--execute`, `--import`, `--activate`, `--only <types>` — dry run by default, and the dry run reads the state so a resource it already manages says so instead of counting as an import; exits non-zero on any failed/unresolvable/ambiguous row; `--import` reads `state list` first and skips already-managed addresses |
| `iac-roles [INPUT]` | `--execute`, `--format` (`text`\|`json`) — the roles the estate's IaC service account needs for the resource types the estate emits, against the roles the estate grants it; exits non-zero when one is missing; `--execute` writes the missing roles into the estate file. Without an estate: the table of resource types and roles. See [IaC service account roles](#iac-service-account-roles-iac-roles) |

**HCL**

| Command | Options / Arguments |
|---------|---------------------|
| `hcl-init [ARGS]` | runs `<tf_tool> init` in `hcl_dir`; everything after the command is handed to the tool verbatim, so `--config` must come before it |
| `plan [ARGS]` | runs `<tf_tool> plan` in `hcl_dir`, arguments passed through; an org policy the state holds with rules and the estate declares reset gets `-replace` |
| `apply [ARGS]` | runs `<tf_tool> apply` in `hcl_dir`, arguments passed through; an org policy the state holds with rules and the estate declares reset gets `-replace` |
| `migrate <INPUT>` | `--mode` |
| `scan-plan <plan_json>` | `--output` (default: `mapping.yaml`) |
| `generate-migration <mapping>` | `--output` (default: `migrate.sh`) |
| `run-actions <INPUT>` | `--check` (each action's own dry-run form), `--execute` (the form that writes), `--only <names>`, `--phase <before-apply\|after-apply>` — prints what it would run and stops by default |

**Presets**

| Command | Options / Arguments |
|---------|---------------------|
| `get-presets` | `--force` — overwrite presets the estate uses too; `--pristine-dir` |
| `merge-presets` | `--pristine-dir`, `--estate`, `--report-only`, `--adopt <stem\|all>` — reconciling update; `--adopt` upgrades in place instead of forking |
| `check-presets <INPUT>` | `--format` (`text`\|`json`), `--pristine-dir` |
| `doc-packs` | `--out <DIR>` (default `<presets_dir>/docs`), `--check` — one Markdown page per pristine pack, derived from the pack file, plus a grouped index; `--check` fails when the pages are behind, a claim names a control its catalog lacks, a pack header says nothing the index can print, or a pack version has no changelog row |

**Policies**

| Command | Options / Arguments |
|---------|---------------------|
| `export-organizational-policies <CONFIG_FILE>` (alias `export-org-policies`) | `--customer-organization-id`, `--output` |
| `diff-organizational-policies <CONFIG_FILE>` (alias `diff-org-policies`) | `--customer-organization-id`, `--report`, `--format` (`text`\|`markdown`\|`json`), `-r/--recursive` (every folder and project below) |
| `report-organizational-policies <CONFIG_FILE>` (alias `report-org-policies`) | `--customer-organization-id`, `--scope` (`active`\|`inactive`\|`full`), `--format` (`markdown`\|`json`\|`pdf`), `--report`, `-r/--recursive` |
| `adopt-org-policies <INPUT>` | `--dry-run` — alias of `adopt --only google_org_policy_policy --activate --execute --import` |

**Compliance and audit**

| Command | Options / Arguments |
|---------|---------------------|
| `questions <INPUT>` | `--format` (`text`\|`json`\|`markdown`), `--unanswered` — every question the estate's packs declare with its state: `answered` when the estate's own params bind it, else `unanswered` with the default the pack offers or `blocking` when none is possible. `markdown` is the decisions sheet; `summary.complete` is the gate `bootstrap` and `transpile --apply` refuse on |
| `interview <INPUT>` | `--create`, `--all`, `--accept-defaults` — asks the open questions one at a time at the terminal and writes each answer into the estate's params; `--create` writes the estate first from `presets/estate-core.satz`. See [satz interview](docs/interview.md) |
| `require <FRAMEWORK> <INPUT>` | `--format` (`text`\|`json`), *(catalog id, e.g. `cis-gcp-4.0`)* |
| `report-compliance <FRAMEWORK> <INPUT>` | `--format` (`markdown`\|`json`\|`pdf`), `--report`, `--prowler`, `--checkov`, `--no-live`, `--fail-on <statuses>` |
| `scan [<INPUT>]` | Checkov over `hcl_dir`; with the estate, each finding is pointed at the Satz block that declared the resource; failed checks exit 1 |
| `prowler <INPUT>` | `--format` (`text`\|`json`) — the Prowler invocation this estate needs, printed. The organisation and the project ids the estate declares, the frameworks it CLAIMS as `--compliance` (which also filters which checks run), `--output-formats json-ocsf`, and an output path under `evidence/prowler/<UTC date>/`. satz never runs Prowler: the scan spends API quota in every project, and Prowler reads as whoever is logged in rather than as the estate's service account |
| `triage <FRAMEWORK> <INPUT>` | `--prowler <file>` (required), `--format` (`markdown`\|`json`), `--report`, `--fix` — every Prowler FAIL sorted into who-fixes-it buckets against the estate's claims, and the checks Prowler maps to no control of the framework counted in a section of their own; `--fix` adds the estate delta they imply (proposed, never written) |
| `remediation-plan <FRAMEWORK> <INPUT>` | `--prowler <file>` (required), `--checkov`, `--out <dir>`, `--merge <authored.json>` — the remediation dossier: triage joined with Checkov per resource, counted, written as `dossier.json` + `findings.csv` + `findings.xlsx` (mechanical columns filled, `[Authored]` columns and who authored them, Review dropdown) + `meta.json` under `evidence/plan/`; offline and deterministic (the dossier hash names the run). `--merge` fills the `[Authored]` columns from an `authored.json` written against this run's hash — every entry names `authored_by` and `authored_at` — and keeps it beside the run; `dossier.json` and its hash do not change |

**Tool**

| Command | Options / Arguments |
|---------|---------------------|
| `update-schema` | `--providers`, `--version`, `--tf-tool` |
| `map-types` | `--only <types>`, `--import-config` — derive the API→Terraform field map per type into `presets/type-map.yaml` |
| `fmt <PATHS…>` | `--check`, `--stdin` — rewrite Satz files in their canonical layout (indentation, spacing, `=` alignment, list commas); `--check` names the files that are not and exits 1; `--stdin` formats one file from stdin to stdout, for editors |
| `lsp` | the language server behind an editor's Satz support, on stdio, started by the editor: diagnostics from the parser on every change and from the pipeline on every save, completion and hover from the provider schema, go-to-definition for `use` paths and params, formatting |
| `self-update` | `--no-open-readme`, `--check-only`, `--skip-checksum` |
| `completion [SHELL]` | `--install` |
| `open-readme` | *(none)* — opens the documentation site |
| `mcp` | `--allow` (`read`\|`write`\|`exec`, comma-separated; default `read`), `--self-gated`, `--root <DIR>` (the directory the server may work under; default the current one) — serve the estate over the Model Context Protocol on stdio, so an agent drives satz. Nineteen tools: each data tool returns structured content with a published output schema, and every tool is annotated so a client knows which are safe to run unattended. satz calls no model; the agent calls satz. See [docs/mcp.md](docs/mcp.md) |
| `whoami [INPUT]` | `--offline` — print BOTH halves of the identity: the ADC account and its file, and (with an estate) the service account that estate's live commands run as, checked — may this credential become it, is the quota project reachable, and does it hold the permissions the estate's resource types need |

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

**Without the flags:** `satz interview yaml/<name>.satz --create` writes an estate that
asks for the same sixteen values one at a time and offers the derived ones as defaults;
an agent does the same over MCP with `satz_interview`. Either way `bootstrap` refuses until
every question is answered — [satz interview](docs/interview.md).

### Day 0 Bootstrap (`bootstrap`)
`bootstrap` runs the day-0 onboarding of a new customer organization.

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

### IaC service account roles (`iac-roles`)

The estate's IaC service account — `svc_iac_account` in `infra_project_name` — holds
named roles at the organization, not `roles/owner`. A role granted at the organization
is inherited by every folder and project under it, including those created by hand or
before the estate, so the same roles reach them. `iac-roles` compares the roles the
estate grants that account with the roles the resource types it emits need.

```bash
satz iac-roles <INPUT>             # the report; exits non-zero when a role is missing
satz iac-roles <INPUT> --execute   # writes the missing roles into the estate file
satz iac-roles --format json       # the table: per resource type, a permission and the roles that carry it
```

**What the account needs.**
- Reads, whatever the estate emits: `roles/viewer`, `roles/browser`,
  `roles/iam.securityReviewer`, `roles/cloudasset.viewer` and
  `roles/serviceusage.serviceUsageConsumer`. `import`, `adopt` and
  `report-compliance` read every folder and project with them.
- Per resource type, one permission and the predefined roles that carry it:
  `google_folder` needs `resourcemanager.folders.create`
  (`roles/resourcemanager.folderAdmin`), `google_project` needs
  `resourcemanager.projects.create` (`roles/resourcemanager.projectCreator`),
  `resourcemanager.projects.update` for a project it did not create
  (`roles/resourcemanager.projectMover`) and the billing link. `satz iac-roles`
  without an estate prints the whole table; its source is `src/iac_roles.rs`.
- Organization and project needs are met by a role granted at the organization, and
  `roles/owner` there meets all of them. Billing-account needs are met by a grant on
  the billing account (`google_billing_account_iam_member`).
- `google_cloud_identity_group` needs the Groups Admin role of the Google Workspace
  admin console. It is not an IAM role, so it is named and not checked.
- No role in the table deletes a project. Google makes a project's creator its owner,
  so the account deletes the projects it created; a project it did not create is
  deleted by a person, or with `roles/resourcemanager.projectDeleter` granted for the
  deletion. `google_project` refuses a delete unless its `deletion_policy` is
  `"DELETE"`.

The roles granted are the `google_organization_iam_member` and
`google_billing_account_iam_member` grants to
`serviceAccount:<svc_iac_account>@<infra_project_name>.iam.gserviceaccount.com`, in the
estate and in every pack it uses.

**`--execute`** adds each missing role to the account's existing grant list — the list
whose key names the account once `{param}`s are interpolated — or appends a new block
when the estate has none. It writes the fewest roles: a need only one role meets takes
that role, and a need with alternatives takes a role already chosen. A new
billing-account block is itself a `google_billing_account_iam_member` and needs
`roles/billing.admin`, which also carries the billing link, so that is the role written
there. The command then compiles the estate again, and restores the file when a role
is still missing.

**Every compile checks the same**, at the [validation level](#schema-validation): `warn`
(the default) prints the missing roles and the `iac-roles --execute` command that writes
them, `error` refuses the compile, `none` skips the check. An estate that names no IaC
service account is not checked. A resource type the table has no row for is named in a
note.

**`satz whoami <estate>`** tests the same needs live — the permissions themselves, with
the credential the estate's live commands run as, on the organization, the infra
project and the billing account. See
[Which identity a command runs as](#which-identity-a-command-runs-as).

`roles/resourcemanager.organizationAdmin` carries
`resourcemanager.organizations.setIamPolicy`, so the account can grant itself any role
at the organization. The named roles state what an apply uses; they do not limit what
the account can grant. The reasoning is in
[ADR 0009](docs/adr/0009-iac-service-account-named-roles.md).

### Transpile (`transpile`)
Compiles the estate to HCL. Input is a `.satz` estate; a legacy `.yaml` estate is refused with a pointer to `satz import`.

```bash
satz transpile <INPUT> [options]
```

**Parameters:**
- `<INPUT>`: Name of the estate file. This is resolved relative to the `yaml_dir` defined in your config.
- `--output, -o <FILE>`: Optional output subdirectory or absolute path. By default, output goes to `hcl_dir`.
- `--schema-dir, -s <DIR>`: Override the schema directory.
- `--print-variables`: After transpilation, print the resolved variable table (`terraform.tfvars`) to stdout. Useful for debugging variable resolution across multiple include files.
- `--scan`: after transpiling, run Checkov (terraform framework) over `hcl_dir` — `checkov` on PATH, else `uvx checkov` — and print every failed check under the resource it hit, with the Satz file and line that declared it (from the emission manifest) and Checkov's guideline link. Failed checks exit 1, so it gates like a test. `satz scan [<estate>]` does the same without transpiling first.
- `--plan` / `--apply`: after transpiling, run `<tf_tool> plan` / `apply` in `hcl_dir` — one command from estate to plan. The dir is initialised first when it has no `.terraform`. The same as `satz transpile … && satz plan`; `satz plan`, `satz apply` and `satz hcl-init` remain for running the tool on its own (extra arguments pass through).

**Running from subdirectories:**
You can run the transpile command from any directory (e.g., from within the `hcl/` folder) by specifying the config path. Both styles are supported:
```bash
# Global option before subcommand
satz --config ../config.toml transpile my-infra.satz

# Global option after subcommand (Recommended)
satz transpile my-infra.satz --config ../config.toml
```
This reads `../yaml/my-infra.satz` and writes the HCL into `hcl_dir`, here the current directory.

**Satz estates — the fragment pipeline:**
A `.satz` input compiles through the fragment pipeline: every source
file (estate + each `use`d pack) becomes its own fragment, the composition
algebra folds them (grant union, deep-equal idempotence, conflicts reported
with every contributing origin), and HCL is emitted from the folded result,
with deterministic ordering (snapshot-gated by `tests/corpus/`).

**Subtractive overrides (`suppress`, Satz estates only):**
An estate can remove something a used pack contributes — without forking:

```
use "presets/CIS-GCP-Foundation-4.0.satz" as google_org_policy_policy

// drop one pack resource; grant-edge form removes a single role
suppress google_org_policy_policy "compute-skipDefaultNetworkCreation"
suppress google_organization_iam_member "group:sec@example.com" role "roles/viewer"
```

Rules: interpolations (`{param}`) work in the name; a suppress that matches
nothing is a **hard error**, so a stale suppression is reported instead of
deployed; a suppressed resource that was the witness of a compliance claim shows
up as broken in `require`.

**Raw HCL passthrough (`hcl { … }`, satz only):**
For Terraform the resource model does not cover: it composes and deploys, but the
compliance plane cannot see into it.

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
entity, so it does not take part in the fold, cannot conflict and cannot carry a
claim.

**Under the Hood:**
- Parses the estate and every pack it `use`s into per-file fragments; params are declarations in one document-ordered namespace (the using file's binding wins over a pack's default), sorted by dependency.
- Folds the fragments by Terraform address (⊕): the same address with the same body collapses, with a different body the transpile aborts naming both files.
- Schema-typed: every resource key and block key is checked against `schemas/*.json` at parse time.
- Generates these files in the output directory:
    - `main.tf`: Resources.
    - `providers.tf`: Provider configurations and aliases.
    - `variables.tf`: Variable declarations.
    - `terraform.tfvars`: Variable values.
    - `imports.tf`: (Optional) OpenTofu `import` blocks for existing resources.

### Resource Imports

`satz` emits OpenTofu/Terraform 1.5+ `import` blocks, so existing cloud resources come under management without CLI `import` commands.

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
- **A reference must name something the estate emits.** Every `${{…}}` is checked
  against the emitted set at compile time; one that names nothing fails with the
  site, the reference, and the labels of that type that do exist.
- **References and adoption.** A value that is exactly one reference
  (`project = "${{google_project.x.project_id}}"`) is recorded as a reference and
  followed — `adopt` resolves through it, and a witness scoped by it verifies.
  A reference *embedded* in a longer string is only known after apply; those are
  reported as unresolvable, naming the reference, instead of being matched
  against live state as literal text.
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
like any other. GCP **managed** constraints (their name contains `.managed.`, e.g.
`iam.managed.disableServiceAccountKeyCreation`) differ: depending on org state they
must be *activated*, then *imported as-is*, and only then *modified*.
`satz adopt --activate` (see [Adopting what already exists](#adopting-what-already-exists-adopt-brownfield))
does the first two; three read-only commands (`export`, `diff`, `report`) read the
live policies.

All Org Policy API calls use the identity the estate's live commands run as — its
IaC service account, minted from the Application Default Credentials
([Which identity a command runs as](#which-identity-a-command-runs-as)) — and send
the quota project as `x-goog-user-project` (resolved from
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
hoisted — their position in the tree is their parent.

Every other `*_iam_member` type — bucket, service account, KMS key, Pub/Sub topic — writes
its scope in the grant map beside the members (`bucket = …`, `service_account_id = …`); a
key that cannot be a member is that scope, a member being always `<type>:<value>` or one of
the two reserved forms `allUsers` / `allAuthenticatedUsers`. It namespaces
the grant, so two maps pinning two scopes are two grants even with the same member and role.
One scope per map; repeat the map for the next scope. The labelled-resource form works for
these types too and is the better fit for a single edge.

Every other resource type is position-independent whenever its parent is explicit in the
source (`org_id`, `parent`, `billing_account`), so it needs no hoisting. What all types
get instead is the **duplicate-address guard**: a Terraform address may be emitted once.
Byte-identical duplicate definitions — the same singleton resource (org audit config,
sink, contact) included from several fragments — collapse to a single emission with a
printed note; the same address with *different* content aborts the transpile, naming the
address and the first differing line. Attributes are not merged: a merge would meet
the same conflict one level down.

**Cross-file merging is the fold.** Two packs (or the estate and a pack) may declare
the same resource type at the same position — the fragments compose by address:
distinct labels union, the same label with an identical body collapses, the same label
with a different body aborts the transpile naming both files. This is how the
audit-logsink pack and the CIS central-monitoring pack each declare their own
`google_logging_organization_sink` and coexist. The fold merges labels, never
attributes.

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

Generates a `google_cloud_identity_group` with `group_key { id = "my-group@<domain>" }`,
`parent = "customers/<id>"`, the discussion-forum labels, `initial_group_config`, and a
`lifecycle` block carrying `ignore_changes = [initial_group_config]` merged with the one
declared (`prevent_destroy = true` here).

**Notes:**
- `ignore_changes` and `replace_triggered_by` entries are emitted as **bare** HCL identifiers/expressions (e.g. `initial_group_config`, `labels["env"]`), not quoted strings.
- Use the string form `ignore_changes = "all"` to ignore changes to every attribute (renders the bare keyword `all`).
- Boolean meta-arguments such as `create_before_destroy` and `prevent_destroy` are passed through as-is.

### Mode Switching & State Migration (`migrate`)
Moves a project between development (`local`) and production (`cloud`) mode.

```bash
satz migrate <INPUT> --mode <MODE>
```

**Parameters:**
- `<INPUT>`: Name of the estate file (`.satz`).
- `--mode, -m <MODE>`: Target mode (`local` or `cloud`).

**Under the Hood:**
- **Update the estate**: Rewrites the `deployment_mode` param in the `.satz` file (an estate without one is refused).
- **Regenerate**: Runs `transpile` to update the backend configuration (Local vs GCS) and provider authentication (ADC vs Impersonation).
- **Migrate State**: Executes `tofu init -migrate-state` to move the Terraform state to the new backend.

### Creating an estate from what exists (`import`)

One verb, four input shapes: a state file, the live organization, a legacy YAML
estate, existing `.tf` files. The result is a Satz estate that compiles as-is: a local backend, `customer_organization_id`, every
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
satz import ./terraform                      # existing .tf: variables → params, resources → Satz, the rest verbatim in `hcl trust`
satz import ./terraform --wrap-all           # …or every block verbatim, nothing promoted
satz import                                  # live, root taken from the import config
satz import organizations/123456789012 --into C0example.satz   # only what the estate does not declare
```

**Parameters:**
- `SOURCE`: what to import from; the shape is read off its form (`--from state|org|yaml|hcl` when it cannot tell). Omit it to use the import config's `root`.
- `--all`: every type the source can deliver, not only the rows marked `import: true` — at the live shape every row with a Cloud Asset Inventory name, from a state file every row. `--only` and `--exclude` apply after it.
- `--only <types>`: comma-separated resource types, `*` wildcards allowed (`google_*_iam_member`); everything else is switched off for this run. Overrides `only` in the import config.
- `--exclude <types>`: comma-separated resource types, `*` wildcards allowed; these are switched off for this run. Overrides `exclude` in the import config.
- `--output, -o <FILE>`: output inside `yaml_dir` (default `discovered.satz`; the extension is always `.satz`).
- `--import-config <FILE>`: the import configuration (default `presets/import-config.yaml`, or `import_config` in `config.toml`).
- yaml shape: `--kind estate|pack`, `--gate <estate>.satz` (compile a converted pack in context), `--fork` (write `<stem>.local.satz`).
- Tier-2 files, written for CDKTF, convert too: unanchored top-level scalars become `params` entries (kebab→snake_case) and the top-level `version:` dialect marker is dropped — both mappings are named in the converted file's header, and a scalar that duplicates a `variables:` entry is refused rather than merged.

**The import config** (`presets/import-config.yaml`, YAML — it is data that
configures an import, not an estate) is the repeatable form of the command line:

```yaml
root:                              # live shape; the command-line SOURCE overrides it
  organization: "123456789012"
  folder: { path: "Shared Services/Prod" }   # or { id: "456789" } — exactly one
  project: my-prj                  # narrows further
only: [google_folder, google_project, "google_*_iam_member"]
exclude: [google_project_iam_member]   # left out; --exclude overrides it
resource_types:                    # per type: import on/off, attribute include/exclude,
  google_project:                  # asset_type, and the rules `satz adopt` reads
    import: true
```

A folder `path` is resolved live from the organization, one segment at a time —
exactly one folder may carry each name, otherwise the run stops and lists the
candidates; nothing is guessed. The run prints the effective root and filter.

**An import may be partial; every run ends with the skipped list** — each resource the source had and the estate does not, with
its reason: `type off (import: false)`, `filtered by --only/--exclude`, `unmapped` (no
import-config row fits the asset), or `parent not imported`. Counts by reason
always; every name with `--verbose`. The levers are the `import:` rows and
`--only`.

A resource's key is its name, sanitized. Where two containers hold the same
name for one type — every project has a `_Default` log sink — the copies take
their project or folder as a prefix, because an address is `<type>.<key>`
across the estate. A folder nested under another folder carries no `parent`
attribute: the nesting is the parent.

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
`tofu plan` over the estate and its packs then shows N to import, 0 to add, 0 to
destroy.

Live imports carry the API's vocabulary. Keys are snake-cased (`storageClass` →
`storage_class`); where the names differ — `lifecycle.rule[]` is
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
import — nothing is written from a partial sweep. A nested value the API does not
return while it holds the provider's default — a subnet's `log_config.filter_expr`,
default `"true"` — is read back into state as empty, so the first plan after the
import shows a one-time in-place update to the default; the first apply writes it and
the subnet's flow logs do not change.

**Under the Hood:**
- state: reads `tofu show -json` (file, stdin, or run now); only the types with `import: true` are taken; read-only/computed fields are dropped against the provider schema.
- live: one Cloud Asset Inventory sweep under the root; needs `cloudasset.assets.searchAllResources`; useful for infrastructure nobody manages with Terraform yet. Only asset types the config maps are seen.
- yaml: the legacy-dialect converter (`!include` → `use`, anchors → params, `!format` → interpolation), compiled through the fragment pipeline afterwards and reporting what it emits; an old `!import-include` becomes `use` plus `satz adopt`.
- hcl (`satz import ./hcl-dir`): three tiers. A `variable` with a literal `default` and a `locals` entry with a literal value are **promoted to params** — params are Satz's variables, so the imported estate stays re-parameterisable instead of carrying baked-in literals; `var.x` becomes a bare param reference and `"a-${var.x}"` the interpolation `"a-{x}"`. A `variable` with no `default` is named in the header and given no value, so `satz transpile` stops with `unknown param` until it is bound — the same gate the source had. A `resource` block of a schema-known type is **translated** when every value is a literal, a promoted param, or a reference to a managed resource (carried verbatim as `${{…}}`, which emits back byte-identically); folders, projects, services and grants are **placed** by the folder/project they reference, so the tree comes back and `customer_organization_id` is inferred, and a resource that named no project of its own inherits a dropped `provider` block's default when that resolves to one of the imported projects. A `*_iam_member` whose scope is neither project, folder nor organisation — a service account's, a bucket's — becomes a labelled resource with its scope attribute, because the member map has no room for one. A block whose `count` is `length()` of a promoted list of scalars, and whose every `count.index` indexes THAT list, is **expanded**: one Satz resource per entry, each taking the entry where the source wrote `var.list[count.index]`, labelled after it (`state_europe_west3`) or by its position when the entry makes no identifier. Terraform's idiom for "one of these per entry" IS one resource per entry in Satz. Any other `count` — over a list this import cannot resolve, or with a `count.index` that indexes something else — leaves the block verbatim, because half an expansion would be a guess about what the source meant. Everything else — `module`, `data`, `output`, blocks using `for_each`/`dynamic`/`provider`/`depends_on` or a `count` of another shape, function calls, conditionals, groups, memberships, billing grants, authoritative IAM bindings, unknown types, labels that are not identifiers — is carried verbatim inside `hcl trust "imported from <file>:<line>" { … }` and the report says why, per block. A promoted declaration that a wrapped block still reads is carried verbatim too, so its `var.x` keeps resolving. `terraform`/`provider` blocks are dropped with a note; the emitter owns `providers.tf`. `--wrap-all` wraps everything and promotes nothing. Either way the estate deploys exactly as the source did: `tofu plan` against the source's state shows no changes. A translated block is not verified: a `${…}` reference is opaque to the compliance plane. Also the way in for `gcloud beta resource-config bulk-export --resource-format=terraform` and `tofu plan -generate-config-out` output.

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

### Get presets (`get-presets`)
Download the `presets` folder from the repository into your project's `presets_dir` (default: `presets/` beside `config.toml`). The library holds everything available for copying; `yaml_dir` stays reserved for the files you actually use and adapt. Requires a valid config so the tool knows where to write files. Each preset's purpose, include line and variables (required vs. overridable defaults) are documented in [presets/README.md](presets/README.md) — presets are read-only building blocks; all per-org values belong in the estate's `params { … }` block.

```bash
satz get-presets
satz get-presets --force                       # overwrite in-use packs too
satz get-presets --pristine-dir ~/src/satz/presets   # skip the download
```

**Parameters:** `--force`, `--pristine-dir`. Accepts global options `--config`, `--validation`, `--verbose`.

**Under the Hood:**
- Fetches the `presets` directory from the GitHub repo (main branch) in **one** API request (a recursive tree), preserving subdirectories (e.g. `presets/security-group-models/`). The files themselves come from raw.githubusercontent.com, which does not count against the API quota.
- GitHub's unauthenticated quota is 60 requests/hour and is shared with `self-update`. Set `GITHUB_TOKEN` to raise it, or pass `--pristine-dir` to skip the network entirely; exhaustion is reported as a rate limit with the wait, not as a parse error. See [docs/workflows.md](docs/workflows.md#when-upstream-stops-answering-the-github-quota).
- Then decides per file: **missing** → installed; **identical** → skipped; **differs but the estate does not use it** → refreshed; **differs and the estate USES it** → **refused**, naming `merge-presets` / `merge-presets --adopt <stem>` instead. A changed pack the estate deploys changes the organization; `merge-presets` reports that change before any `tofu plan`. `--force` overwrites anyway, listing each in-use pack as it does.
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

Every reporting command answers in one vocabulary — `--format text|markdown|json|pdf`,
each command accepting the subset it can produce and **refusing the rest by name**.
`require --format json` gives the same verdicts as data:

```bash
satz require cis-gcp-4.0 C0example.satz --format json | jq '.summary'
#   { "satisfied": 18, "partial": 5, "deviations": 0, "unmet": 14, "broken": 0, "contradicted": 0, … }
```

**stdout carries the answer and nothing else.** The version banner, the schema-loader
line and every other progress message go to stderr, so a report can be piped into a
parser without filtering, and `satz mcp`, which speaks JSON-RPC over stdout, is not
corrupted by a stray line.

A catalog can also be a **cross-walk** over another one. `iso27001-2022` carries the 93
Annex A controls and, for those a landing zone can evidence, the CIS controls that stand
as that evidence; `require iso27001-2022` folds those verdicts rather than asking packs
to claim a second framework:

```bash
satz require iso27001-2022 C0example.satz
#   ✓ A.8.3  Information access restriction  — google_org_policy_policy.storage_publicAccessPrevention, …
#   ◐ A.5.3  Segregation of duties           — open duties: role-matrix-reviewed
#   ○ A.5.1  Policies for information security — organizational control (no IaC witness)
#   ◇ A.7.1  Physical security perimeters    — inherited from the provider (shared responsibility)
```

The fold is pessimistic: every source satisfied and no duty open gives ✓, any deviation
below surfaces as a deviation above with its reason, a broken claim stays broken, and
anything else is partial. The ISO view therefore follows the CIS coverage beneath it.

Per control: **✓ satisfied** (an `implements` claim from an included pack, every witness
emitted by the compiler — a resource written inside a raw `hcl { … }` block never counts), **◐ partial** (witnesses present but manual duties
open, or only `contributes` claims), **⚠ deviation** (the estate declares that it does not
meet this control, with a reason — see below), **✗ unmet** (with the packs that would
provide it — remediation as suggestion), **‼ broken claim** (a pack claims witnesses the
estate does not emit; never reported as satisfied), **‼ contradicted claim** (the
witnesses are emitted and one of them does the opposite of what the claim says — an
`implements` over an org policy declared `enforce = "FALSE"` or `reset = true`, or a
`deviates` over one that enforces), **○ organizational**
(no IaC witness possible). Exit code is non-zero on unmet/broken/contradicted, so it
gates CI — deviations are disclosed decisions and do not fail it.

### The Satz language

Driving satz from an agent? **[`docs/llms.md`](docs/llms.md)** is
the working subset written for that — the MCP server serves it as `satz://guide`, so an
agent gets it without a repository.

Full specification, grammar and lookup: **`docs/language.md`**.

### Adopting what already exists (`adopt`, brownfield)

A first `apply` against an organisation that already has folders, groups, org
policies or a state bucket fails one resource at a time with `409 … already
exists`; a resource whose id Google assigns and whose name need not be unique — an
alert policy, a notification channel — is created a second time. `satz adopt`
resolves the live id of every resource the estate declares and brings it under
management:

```bash
satz adopt C0example.satz                                   # dry run: the resolution table
satz adopt C0example.satz --execute                         # write verified "import-id"s into the estate
satz adopt C0example.satz --execute --import --activate     # tofu import now; activate managed constraints
satz adopt C0example.satz --only google_folder,google_cloud_identity_group
```

How a resource is resolved depends on who chose its identity:

- **Projects** are an existence check, not a template: the project id is
  looked up live — exists → verified import; provably absent → *on apply*,
  and every resource inside it (IAM, services, project-scoped policies, …)
  reads *on apply (parent)*: created together with the project, never given a
  derived id, never written or imported. A misspelled project id therefore
  shows as a finding. Google answers a lookup of a project id that does not
  exist (or that the caller cannot see) with **403, not 404**, so a typo reads
  as *FAILED* with the API's denial rather than *on apply*; either way the run
  stops and nothing is written.
- **User-chosen id** (bucket, service account, IAM bindings, sinks,
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

Exactly one live candidate resolves; none means *on apply* (Terraform will create
it); more than one is **AMBIGUOUS**, the candidates are listed, and you pin
`"import-id"` by hand. A FAILED lookup (denied, quota), an unresolvable or
ambiguous resource, or a type without a rule makes the command exit non-zero after
printing the table —
nothing is changed in that case; with `--import`, every row that is not
imported says why. Managed org-policy constraints the
organisation has never had need `--activate` (they cannot be imported before
activation; this mutates the org). `--execute` writes the ids into the `.satz`:
a resource with a block of its own gets an `"import-id"` line; an entry-level
resource (an IAM grant, a project service, a membership) has its list entry
rewritten into the object form (`{ role = "…" "import-id" = "…" }`, see the
language reference §6.7). Derived ids are written too — `tofu plan` verifies
them through the import block, and says so if one does not exist. An entry
that cannot be found in the source (interpolated) and a resource declared in a
**pristine pack** (upstream-owned, never edited) come back as hints: import
those with `--execute --import`, or fork the pack first.

`adopt-org-policies` is an alias of
`adopt --only google_org_policy_policy --activate --execute --import`.

It is a separate command because it makes live API calls and, with
`--activate`, changes the organisation, so adoption is never a side effect of
`transpile`. The only trace it leaves in the estate is the `"import-id"` it
writes.

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

`plan`, `apply` and `hcl-init` run the configured `tf_tool` (OpenTofu by default) in the
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
prints the corrected command.

`plan` and `apply` add one argument of their own: `-replace=<address>` for each org
policy the state holds with rules while the estate declares it `spec { reset = true }`,
with a note naming it. The provider would update such a policy by sending its rules
together with `reset`, which the API refuses (`400 Cannot set PolicyRules if reset is
true`); the replace deletes the policy and creates it reset. They read the state for
this only when `main.tf` declares a reset policy. Nothing is added to an apply of a
saved plan, to `-destroy` or `-refresh-only`, or for an address the arguments already
replace. `tofu plan` run directly shows the in-place update instead.

They do not transpile first, so the generated diff can be reviewed between `transpile`
and `plan`; `transpile --plan` / `--apply` does both in one command.

### Live verification checks enforcement, not existence

`report-compliance` verifies witnesses against the live estate through Cloud Asset
Inventory. For most resource types that is an existence check. For an org policy,
existence is not the control, because a policy switched off in the console still
exists, so org-policy witnesses are compared by VALUE. The estate's declared
`spec { rules { enforce = "TRUE" } }` is checked against the live policy's
`spec.rules[].enforce`, and a mismatch reports **NOT ENFORCED**, which outranks
DRIFTED: a missing resource is absent from the inventory, while a switched-off
policy is listed like an enforced one.

The verdict is the policy's one unconditional rule. Its conditional rules — a
tag-conditional `enforce: false` that exempts tagged resources, for one — are listed
beside the verdict on the row and in the evidence (`conditional`). A policy with no
unconditional rule or more than one, or a list constraint with no boolean, yields no
verdict; a policy whose live enforcement cannot be read reports *unverifiable*, never
*verified*.

### Deviations: declining a control

An organisation that declines a control declares a `deviates` claim, in a fork
(`X.local.satz`) or in the estate. A claim witnesses that a resource *exists*: an org
policy declared with `enforce = "FALSE"` still emits its resource, so a copied
`implements` claim reports **satisfied** for a control nobody enforces, and dropping
the claim reports **unmet**. The deviation states the decision:

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
deleting the policy outright reports a broken claim, not a deviation. A deviation outranks the claims it contradicts, and can be declared by a
pack fork or by the estate itself.

The output never says "compliant": `require` judges the *declared* estate;
verification against the *live* estate is the evidence report (next section). Catalogs carry no framework text (CIS/ISO prose is license-restricted),
only IDs and paraphrases.

### Exemptions: keeping the control on and letting one resource out

A deviation says *we do not meet this control*. An exemption says *we meet it
everywhere except here* — and an org policy cannot say that by itself, because it is
all-or-nothing per node. Letting one service account hold a key means lowering the
policy for its whole project or folder and raising it again afterwards: a window during
which nothing under that node is enforced, and which nobody remembers to close.

A **Resource Manager tag** closes the window. Unlike a label it is IAM-governed, and an
org policy rule can condition on it, so the policy stays enforced and named resources
are let out one at a time. Google ships `iam.disableServiceAccountKeyCreation` this way
on new organisations. `presets/exemptions/exemption-tag.satz` creates the tag;
[the library page](presets/README.md#exemptions) has the whole mechanism.

**Exemptions come in classes, because IAM is set per tag value.** One blanket value
would mean anyone allowed to exempt anything may exempt everything, so the pack ships a
value per kind of risk — `service-account-keys`, `public-endpoint`, `public-storage`,
`vm-image`, `vm-access`, `data-residency`, `encryption`, `network-appliance` — each
narrow enough that granting it hands over one thing. Audit logging and
domain-restricted sharing carry no class on purpose: exempting the record of what
happened, or letting an outside identity in, is a decision for whoever owns the
baseline rather than something to delegate.

**Who may exempt what, and where, is two grants and both are required:**
`roles/resourcemanager.tagUser` on the *tag value* decides which class a principal may
grant at all, and the `createTagBinding` permission on the *target* decides whether
they may apply it to the organisation, a folder or one project. A team holding the
first on `public-endpoint` and the second on their own folder can exempt public
endpoints there and nothing else anywhere else. The first half is declarable in the
estate (`google_tags_tag_value_iam_member`), so *who may exempt what* is in the
repository rather than in somebody's console history.

That is the exception path. **Standing** authority — who administers projects, networks,
guardrails, billing — is the [security group model](presets/README.md#security-group-models)
the estate adopts, S1 or S2. The two are deliberately separate: the point of the tag is
that "may grant a narrow exemption" need not imply `roles/orgpolicy.policyAdmin`, which
is what the security-admins group holds and which can rewrite any policy outright.

**Tag bindings are inherited**, which is the part that surprises people. Bound to a
service account an exemption reaches that account; bound to a *project* it reaches
everything in it, including every resource created afterwards, for as long as the
binding exists; bound to a folder, everything below. Prefer binding the individual
resource.

`require` prints an exemption under the control it belongs to rather than letting a
conditional policy read as plain "enforced" — the control is met *and* something is let
out, and both are facts an auditor reads together:

```
  ✓ 1.4   Only GCP-managed service account keys  — google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation
      ↳ exempted: google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation: enforce OFF where exempted service accounts
```

An exemption declared in the estate is the permanent, reviewed kind: it sits in the
repository with its owner and its reason. A binding somebody adds out of band is not
visible to satz yet; Cloud Asset Inventory serves tag bindings, so reporting an
undeclared one as drift is the piece that makes temporary lifts auditable.

### Evidence report (`report-compliance`)

The goal view joined with the **live estate**: every witness of a satisfied/partial
control is verified against Cloud Asset Inventory (org sinks, log metrics, alert
policies, notification channels, buckets — matched by name/display name extracted from
the generated HCL). Two witnesses live in an IAM policy rather than in resource data
and are read from it: the organization's audit config is verified when the live policy
audits the declared service with every declared log type (a missing one is named), and a
bucket IAM member when the live bucket policy binds the declared role to that member —
where the member is a sink's `writer_identity`, the value comes from the live sink,
since Google issues it and no estate file holds it. Manual duties merge with `attestations.yaml` beside config.toml
(`duty-id: {by, date, note}`), and a Prowler export can be ingested as
corroboration (`--prowler findings.json` — the OCSF export of Prowler 5, `prowler gcp --output-formats json-ocsf`; a FAIL on one of a control's *verified* witnesses marks the row **CONTESTED**, a FAIL elsewhere is an unmanaged finding beside it). The report names the Prowler version that wrote the export; an export from an older Prowler, or with no version in `metadata.product`, is refused with the version it carries. FAIL findings whose check Prowler maps to no control of the framework are in no row; the report counts them per check in a section after the table and under `prowler_unmapped` in the JSON, and `triage` and `remediation-plan` (`meta.json`, the Provenance sheet) do the same.

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
types have no live check), **unverified** (no witness could be checked — no ADC,
inventory unavailable), **DRIFTED** (declared but not live),
partial (open/attested duties), unmet, broken claim. Each run appends
`evidence/<framework>-<timestamp>.json` beside the config — the evidence history —
and writes the report (pandoc PDF like `report-organizational-policies`). Without
credentials or with `--no-live`, the report shows declared-estate status and
records why: `live` says whether the inventory was actually read,
`live_status` says why not (`skipped`, `no-organization-id`, `no-witnesses`,
`unavailable`) and `warnings` carries the reasons the command prints to stderr. The report states check semantics ("a resource with these properties was
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

**When upstream changes a preset the estate includes** and the change is
*semantic* (the canonical form of the parsed pack differs; comment, formatting and
version-line changes upgrade in place), merge-presets:

1. preserves your current content as `X.local.satz`,
2. repoints the estate's `use` to it — then **checks the edit**: the transpiled
   output must be byte-identical, else everything rolls back,
3. updates pristine `X.satz` and writes `X.diff.satz` — exactly what adopting
   upstream would change, with a `local -> upstream` version header.

**Adoption** is `--adopt <stem>`: the pristine
name is overwritten in place, the estate's `use` is left alone, and the run prints
the **emission** delta (which resources appear or disappear) rather than the preset
diff. `--adopt all` covers every pack that is only BEHIND and refuses one that differs
at the same version, which is an edit and needs `--adopt <stem>`. A fork+repoint needed in
the same run is DEFERRED: the repoint is checked by transpile identity, which an
adoption changes. To adopt an existing fork instead, point the
`use` back at the pristine name and delete `X.local.satz` (the next run removes the
orphaned diff). Presets *not*
included by the estate are overwritten when they differ (git history keeps
tracked files). The estate file must be git-clean for auto-repoints — commit or
stash first so the repoint stays an isolated, reviewable edit. `--report-only`
prints every planned action without writing; `--estate <file>` overrides the
default discovery (the single `estate` .satz in yaml_dir).

A semantic change without a version bump warns (an upstream release bug); a bump
with identical semantics upgrades in place.

The exit is non-zero when anything needs attention (a fork was created or its
upstream moved, or a repoint was refused), so CI can gate on it.

> **Which command when?** [docs/workflows.md](docs/workflows.md)
> walks the whole decision — how to tell a newer preset exists, whether your copy
> is *stale* or *edited* (they need different commands), and what to check before
> applying.

### Check presets for drift (`check-presets`)

Presets are read-only building blocks — per-org values belong in the estate's `params` block as
[overridable defaults](presets/README.md). `check-presets` finds preset copies that were
edited locally and prints how to migrate them:

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
- **fork** — `X.local.*` files are forks, reported as such (never an error);
  their upstream deltas live in `X.diff.satz`.
- **local-only** / **missing locally** — customer-own files and new upstream presets.

Presets actually used by `<INPUT>` (via `use`) are tagged
`[included]`; drift in an included preset makes the command exit non-zero, so it can
gate CI. `use … when` is followed unconditionally here: a pack whose switch is off
still counts as included, so drift is over-reported rather than missed.

### Self-update (`self-update`)
Checks for and installs a new release from GitHub. After a successful install it prints the documentation URL and opens the site, unless `--no-open-readme` is given.

```bash
# Check for and install a new release (same installer as curl)
satz self-update

# Only check if an update is available (no install)
satz self-update --check-only

# Do not open the documentation site after installing
satz self-update --no-open-readme
```

**Self-update options:** `--no-open-readme` (do not open the documentation site after installing), `--check-only`, `--skip-checksum`. The program can also check for updates on start-up (`self_update_frequency` in the global settings).

**Under the Hood:**
- Fetches the latest release from the GitHub API and compares versions. When a newer version is available it downloads `satz-installer.sh` and `satz-installer.sh.sha256` from that same release, verifies the SHA-256 digest, and only then runs the installer. A checksum mismatch aborts; a release without the sidecar aborts too, unless you pass `--skip-checksum`. On success, prints the documentation URL and opens it unless `--no-open-readme` is given.

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

### Formatting (`fmt`)

`satz fmt` rewrites Satz files in their canonical layout without changing what they
mean. The layout is the one this repository's own files carry: two spaces per brace or
bracket that spans lines, `=` aligned over a run of attributes at one depth (a comment
line inside the run is transparent, a blank line or a block ends it), every item of a
list laid out over lines ending in a comma, an inline list written `[a, b]`, and a
construct that spans lines opening at the end of its line and closing on a line of its
own. The author's line breaks stay: a block written on one line stays on one line.
Strings and `hcl { … }` bodies are verbatim, blank lines collapse to one and never sit
against a brace. A file the parser refuses is reported with the parser's error and left
alone.

```bash
satz fmt yaml/                     # every .satz under the directory (*.diff.satz skipped)
satz fmt yaml/acme.satz --check    # name the files that are not formatted, exit 1, write nothing
satz fmt --stdin < in.satz         # one file from stdin to stdout, for an editor
```

**Parameters:**
- `PATHS…` — files or directories; a directory is walked
- `--check` — report instead of rewrite; the exit code is the answer
- `--stdin` — read one file from stdin, write it formatted to stdout

**Under the Hood:**
- Works on the token stream with its comments and line ends, not on the AST, so nothing
  the parser drops is lost ([ADR 0017](docs/adr/0017-the-formatter-keeps-the-authors-line-breaks.md)).
- Meaning is proven, not assumed: `cargo test` formats every Satz file in the repository
  and checks that the canonical form `check-presets` compares is unchanged and that a
  second pass changes nothing. The smoke matrix runs `satz fmt --check` over the
  repository, so every file here is formatted.
- A reformatted pristine pack is not drift: `merge-presets` compares canonical forms and
  upgrades comment and format churn in place.
- In Zed, until the language server serves formatting, an external formatter does:
  `"languages": { "Satz": { "formatter": { "external": { "command": "satz", "arguments": ["fmt", "--stdin"] } } } }`.

### Language server (`lsp`)

`satz lsp` is the server behind an editor's Satz support, speaking the Language Server
Protocol over stdio. What the editor gets is what satz knows, from satz's own front end:

- **Diagnostics.** `satz::parse` on every change — the parser's error at its line — and
  the whole fragment pipeline on every open and save: the same errors `transpile --check`
  prints, at the file and line they name, published to that file. The pipeline reads the
  editor's open buffers, so an estate with an unsaved pack compiles as you see it. A pack
  has no estate of its own: its pipeline diagnostics come from the `estate` files beside
  it that name it.
- **Completion.** Inside a resource type, its attributes and nested blocks (from the
  provider schema in `schema_dir`), then `use`, then every resource type; at the top
  level the statements and every type; after `=`, the params in scope with their values,
  and `true`/`false`; inside `question` and `action` bodies, their keys.
- **Hover.** An attribute's type, whether it is required, and the provider's description;
  a resource type's provider and size; a param's bound value; a `use` path's resolution;
  a keyword's one-line meaning.
- **Go to definition.** A `use "…"` string opens the file the compiler would load
  (beside the estate first, then `include_dirs`); a param reference opens the
  `params {}` line that declares it, in this file or in the pack it comes from.
- **Formatting.** `satz fmt`, as one edit over the document.

```bash
satz lsp        # started by the editor; nothing to type
```

The server finds an estate through its `config.toml`, walking up from the file. Without
one, or with an empty `schema_dir`, it gives parse diagnostics only; `satz update-schema`
fills the schema. stdout is the protocol; the registry's loading progress goes to stderr,
where the editor's log shows it.

**Under the Hood:**
- `src/lsp.rs`, on `lsp-server` (rust-analyzer's transport, a synchronous loop) and
  `lsp-types` ([ADR 0018](docs/adr/0018-editor-intelligence-comes-from-the-satz-binary.md)).
- Where the cursor is — the enclosing type, the nested blocks, key or value — is read from
  the same token stream the formatter uses.
- The smoke matrix drives the server the way an editor does (`tests/smoke/lsp_client.py`):
  initialize, open, completion, hover, definition, formatting, a parse error and a
  pipeline error, shutdown.

## Playbooks

Standing an organisation up from nothing, adopting one that already exists, and
keeping the preset library current are three walkthroughs on one page:
**[docs/workflows.md](docs/workflows.md)**. The command reference is above. Design
decisions, with the alternatives and what each would have cost, are in
[docs/adr/](docs/adr/).

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
| `tf_tool` | `"tofu"` | The OpenTofu/Terraform binary satz runs (schemas, `plan`, `apply`) |
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

Every compile checks each emitted resource against the provider schema: the
arguments the schema marks `required` (a custom role's `role_id`) and the blocks
with `min_items > 0` (a VM's `boot_disk`) must be present. The check reads what is
emitted, so an argument satz derives — a project from its position, a group's
`parent` — counts. A resource type the loaded schemas do not know is not checked.

`validation_level` in `config.toml`, or `--validation`, sets what a missing one does:
`warn` (the default) prints one warning per resource with the file and line that
declares it, `error` refuses the compile, `none` skips the check. Any other value
is refused. `tofu plan` refuses such a resource either way.

The same level governs the check that the IaC service account holds the roles the
emitted resource types need — see
[IaC service account roles](#iac-service-account-roles-iac-roles).

## Satz

Estates are written in **Satz** (`.satz` files) — the language reference is
[docs/language.md](docs/language.md). Params are declarations in one
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
so a claim naming a witness the compile does not emit is reported as broken. Coverage is `implements`, `contributes` or
`deviates`; witnesses are mandatory on the first two. The coverage word is also an
assertion about what the witnesses DO: `implements` over an org policy that is switched
off, or `deviates` over one that enforces, is reported as a contradicted claim. Literal Terraform `${…}` references
inside strings need doubled braces (`"${{google_project.x.project_id}}"`) since `{…}`
interpolates params. Every command reads `.satz`. The legacy YAML dialect is
accepted only as input to `satz import <file>.yaml`, which converts an estate or a pack,
gated by compiling the result through the fragment pipeline and reporting what it emits —
a migrated estate may need a manual edit, and `tofu plan` is the final check
(see [docs/language.md §12](docs/language.md)).

## Core Principles

Resources are placed by **Hierarchy Context** and **Attribute Inheritance**, and checked by **Strict Validation**.

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
Nested blocks are validated:
- **Attribute vs. Resource**: Every key within a `Project` or `Folder` block must be either:
    - A valid native attribute/block of the parent resource (e.g., `name` for a project).
    - A valid resource type from the cloud provider schema.
- **Error Detection**: Any key that is neither a known attribute nor a known resource type is a **hard error** naming the file and line (ignoring it could drop a resource from the emitted HCL and plan its deletion).
- **Missing Context**: Resources that require a project or folder identifier but are defined outside such a context (without an explicit identifier provided) trigger a warning on stderr; `tofu validate` then fails on the missing attribute.

### 4. Flexible Placement
Cross-context resources (like `google_cloud_identity_group`) may sit inside a Project block, e.g. to keep a project's groups beside the project. The transpiler ignores the project context where the resource's schema has no project attribute.

## Handling Resource Renames (State Migration)

Renaming a resource in the estate changes its HCL label, which OpenTofu plans as a delete and a create. To move the state instead:

1.  **Iterate Locally**: Use `tofu plan -out=plan.binary` and `tofu show -json plan.binary > plan.json` to identify changes.
2.  **Map Moves**: Use `satz scan-plan plan.json` to generate a `mapping.yaml`.
3.  **Apply Renames**: Run `satz generate-migration mapping.yaml` and execute the resulting script to run the `mv` commands.

Switching between local and cloud backends is `satz migrate`.

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

### Run Actions (`run-actions`)

Some cloud steps have **no provider resource at all** — Security Command Center
service enablement is one: provider 7.14.1 ships 35
`google_scc_*` / `google_securityposture_*` types and none of them is enablement
or tier activation. Those steps stay scripts, and `action` is how an estate
declares one so that satz can run it with the estate's own parameters instead of
a human retyping the organisation id.

```
action "scc-services" {
  reason       = "SCC service enablement has no provider resource (google 7.14.1)"
  run          = "scc-enable-all.sh"
  args         = ["--organization", "{customer_organization_id}"]
  execute_args = ["--apply"]
  phase        = "before-apply"
}
```

The library ships it: `use "presets/scc/scc-service-enablement.satz"` binds it,
and the pack contains that action and nothing else.

```bash
satz run-actions estate.satz              # resolve and print; runs nothing
satz run-actions estate.satz --check      # run each action's own dry-run form
satz run-actions estate.satz --execute    # run the form that writes (adds execute_args)
```

`phase` says whether the step is a **prerequisite** (`before-apply`, as above —
the services must be on before the resources that need them) or whether it needs
**what the apply created** (`after-apply`, the default — a per-project setting
has no project to act on until the apply made one). `satz apply` does not run
actions; the operator runs each phase around the apply:

```bash
satz run-actions estate.satz --phase before-apply --execute
satz apply
satz run-actions estate.satz --phase after-apply --execute
```

Nothing runs while compiling: the output of `transpile` depends only on its
sources, so the corpus snapshots and the preset drift check compare like with like,
and compiling a cloned estate runs no script. An action emits nothing,
enters no manifest, and can carry no claim; satz records that the step exists and
never says what it did.

satz does not know whether an action's `--check` form has side effects; **the
action defines it**. A failed action stops the run and its exit code is
propagated; the remaining actions do not run.

What a script can rely on: the interpreter is whatever its shebang says (sh,
bash, Python, a binary — satz executes the file, and it must already be
executable); the working directory is always the one holding `config.toml`,
whatever directory satz was invoked from; and the environment holds exactly five
variables — `SATZ_ACTION`, `SATZ_PHASE`, `SATZ_MODE`
(`check` or `execute`), `SATZ_ESTATE`, `SATZ_HCL_DIR`. Params are not exported,
so anything a script needs must be named in `args` and the declaration stays the
complete record of what the action was told. Every action is located and checked
before any is spawned, so a missing `+x` on the fourth script stops the run before
the first one starts. §6.13 has a worked script.

Packs may declare actions too. Because `get-presets` downloads packs from a
public repository, every compile warns, naming the declaring file, and three
global switches exist:

| switch | effect |
|---|---|
| `--no-actions` | never execute an action, whatever `run-actions` was asked to do |
| `--no-pack-actions` | consider only the estate's own actions |
| `--no-action-warnings` | silence the warning every declared action raises on a compile |

A downloaded script arrives without its executable bit and satz does not set
it: the error names the `chmod +x` to run once the script has been read. The full reference is
[§6.13 of the language spec](docs/language.md#613-action--a-step-with-no-provider-resource);
for a step that must run *between* two resources inside one apply, use a
`terraform_data` provisioner in an `hcl trust` block instead, with the costs
§6.12 lists.

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
- Generates a shell script with `tofu state mv` commands that rename resources in the state.
- The script can be reviewed and executed manually to perform the state migration.

## Development

**Prerequisites:** latest stable **Rust**, and **OpenTofu** (or Terraform) on your `PATH`.

```bash
cargo build --release                              # build
cargo run -- --config config.toml transpile C0example.satz   # run a command
cargo test --workspace                             # run unit tests (all crates)
cargo fmt && cargo clippy --workspace --all-targets  # format + lint
cargo install --path .                             # install the release binary (see Installation)
```

### Editor support (Zed)

`editors/zed/` is a Zed extension for Satz: syntax highlighting, the outline panel,
bracket matching and indentation for `.satz` files, and the language server
(`satz lsp`, see [Language server](#language-server-lsp)) for diagnostics, completion,
hover, go-to-definition and format-on-save. Its grammar is a
[tree-sitter](https://tree-sitter.github.io) grammar in a separate repository,
`satz-tree-sitter`, pinned by commit in `editors/zed/extension.toml`; Zed fetches and
compiles it itself. That repository is private, so the extension is not in Zed's
registry and only someone with access to it can install the extension.

Install it once per machine as a dev extension: in Zed, run `zed: install dev extension`
from the command palette and pick `editors/zed` in this checkout. Zed compiles the
extension's Rust glue for `wasm32-wasip2` (rustup installs the target). After a change to
the pin, to the queries in `editors/zed/languages/satz/`, or to `editors/zed/src/`, run
`zed: rebuild dev extension`. The server is the `satz` binary on the PATH Zed's shell
sees; `lsp.satz.binary.path` in Zed's settings overrides it.

The `hcl { … }` passthrough is highlighted as HCL when Zed's `terraform` extension is
installed. A `*.diff.satz` file is a unified diff, not Satz; to open it as one, add
`"file_types": { "Diff": ["*.diff.satz"] }` to Zed's settings.

`scripts/check-grammar.sh` parses every `.satz` file under `presets/` and `tests/` with
the pinned grammar and fails on any error. Run it before a PR that changes the language;
the grammar is a commit in its repository first, then a pin bump here.

## Releasing

Releases are built by GitHub Actions (cargo-dist) when a **version tag** is pushed. Pushing only `main` does not trigger a release. From a clean `main`:

```bash
cargo release patch --execute --no-confirm    # or: minor
```

A release is `minor` when the same estate or input, run through the new binary,
needs an edit, is refused, or plans differently: a language change, a removed or
renamed command or flag, an input format no longer read, an emission change that
moves a plan. Every other release is `patch`. An upgrade across a minor version
brings estate work; an upgrade across patches does not.

`cargo-release` (config in `release.toml`) bumps `Cargo.toml`, commits `version bump`, tags `vX.Y.Z` and pushes commit and tag. The tag runs `.github/workflows/release.yml`: build the four targets, create the GitHub release with archives, `sha256.sum` and `satz-installer.sh`, then the `attach-checksum` post-announce job (`dist-workspace.toml`, `.github/workflows/attach-checksum.yml`) uploads `satz-installer.sh.sha256` — the sidecar `self-update` verifies against. `prune-releases.yml` afterwards keeps the five newest releases. Re-run `dist generate` after editing `dist-workspace.toml`; the `plan` job runs `dist generate --check` and fails on a hand-edited `release.yml`.

The tag pattern is `**[0-9]+.[0-9]+.[0-9]+*`; the tagged commit must carry that exact `version` in `Cargo.toml`. A release does not run when only `main` was pushed, when the tag predates the bump commit, or when the tag and `Cargo.toml` versions differ.

## Architecture

`satz` compiles Satz estates into OpenTofu/Terraform HCL.

### Core Components

#### 1. Fragment pipeline (`crates/satz-core`, `src/emitter.rs`)
`satz.rs` parses each `.satz` file, `pipeline.rs` resolves params
and `use`s into per-file fragments, `algebra.rs` folds them by Terraform address (⊕), and
the emitter renders the folded IR as `main.tf`, `providers.tf`, `variables.tf`,
`terraform.tfvars` and `imports.tf`.
- **Context Awareness**: a nested resource inherits its parent's identifier (`project`, `folder_id`, `org_id`) from the enclosing block.
- **Intrinsic scopes**: groups, org grants and billing grants hoist to their real scope wherever they are written.

#### 2. Schema Registry (`src/schema.rs`)
Manages Terraform provider schemas (loaded as JSON).
- **Typing**: every resource key and block key is checked against the schema at parse time — an unknown key is an error, not a guess.

#### 3. Template Generator (`src/template.rs`)
Writes the day-0 estate for a new customer.
- **Declarative Bootstrap**: Generates the Satz estate representing the Day 0 infrastructure (Project, Services, Bucket, SA) under the labels `bootstrap` imports by name.

#### 4. Migration (`crates/satz-core/src/migrate.rs`)
The only reader of the legacy YAML dialect: `satz import <file>.yaml` converts an estate or a pack
(`!include` → `use`, anchors → params, `!format` → `"{param}"` interpolation, `!expr` →
`"${{…}}"`), then compiles the result through the fragment pipeline and reports what it
emits. An old `!import-include` becomes `use` plus a `NEEDS ADOPTION` note — its job is
`satz adopt`. The dialect's older, addressless spelling of org policies — a list of
entries identified by `constraint:` — becomes addressed resources, the constraint with
dots turned into dashes ([docs/language.md §12.2](docs/language.md)).

#### 5. Discovery Engine
`satz import organizations/<n>` (and the `folders/`, `projects/`, `state.json` shapes) write a Satz estate from what exists.
- **Asset Ingestion**: reads the resources under the root in one Cloud Asset Inventory sweep.
- **Configurable Filtering**: Uses `import-config.yaml` to include/exclude resources and attribute fields.
- **Schema Validation**: Validates discovered data against Terraform schemas and drops read-only and computed fields, so the HCL plans.
- **IAM mapping**: maps IAM policies to member resources (e.g. `google_storage_bucket_iam_member`) and generates their keys.

#### 6. Organization Policy Engine (`src/org_policy.rs`)
Aligns curated Org Policy sets (e.g. `presets/CIS-GCP-Foundation-4.0.satz`) with the live organization via the GCP Org Policy API v2.
- **Adoption** (`satz adopt --activate`): activates managed constraints that are missing (API create), then imports the existing policies into state — no manual console activation and no `import-id` editing. Adoption is a separate command, never part of `transpile` (`src/adopt.rs` drives it through this module's `OrgPolicyClient`).
- **CLI commands**: `export-organizational-policies` (snapshot live state to a re-importable preset), `diff-organizational-policies` (semantic current-vs-desired report), `report-organizational-policies` (markdown/JSON/PDF inventory with constraint descriptions).
- **Managed constraints**: constraints whose name contains `.managed.` must be *activated* (API create), then *imported as-is* (`tofu import`), then *modified* (`tofu apply`). `satz adopt --activate` sequences the activate+import; `tofu apply` does the modify.
- **Pure diff core**: classification + `normalize_spec` are IO-free and unit-tested; they reconcile `enforce "TRUE"`↔`true`, `allowed_values` ordering, and `parameters` JSON-string↔object so semantically-equal policies don't show as diffs.

#### 7. Cloud Identity Group Lookup (`src/cloud_identity.rs`)
The group and membership resolvers `satz adopt` uses. A groups pack declares groups by name; adopting the ones that already exist needs their opaque `groups/<id>`.
- **Lookup**: the group email the emitted HCL carries (`group_key.id`) is resolved via `cloudidentity.googleapis.com/v1/groups:lookup`; a member email via `memberships:lookup`. Existing ones are imported; missing ones are left for `tofu apply`.
- **403 is ambiguous**: some tenants return it for a nonexistent group as well as for a permission problem, so a denied lookup falls back to listing `customers/<customer-id>` once and answers from that. If that fails too, the resolution is reported as FAILED with a hint, not treated as absent.
- **Declared memberships only**: `adopt` resolves the memberships the estate emits — live members the estate does not mention are never looked at, so adopting a group cannot make `apply` propose removing a member. The membership label is a `DefaultHasher` digest of `(group key, raw member string)` computed by the same `membership_resource_label` helper the emitter uses; `membership_address_matches_the_generated_resource` pins the two together.
- **Quota project**: every request sends `x-goog-user-project`, resolved from `GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT` or the ADC file's `quota_project_id`. Every Cloud Asset sweep sends it too, so every command gets the same answer from an organisation whose credentials carry no default quota project.

### Bootstrap Workflow (Declarative Tofu)
Bootstrap runs in two phases:
1. **Local Phase**: `deployment_mode = "local"`. Runs under User ADC. Creates the management project and initial Service Account.
2. **Cloud Phase**: `deployment_mode = "cloud"` (`satz migrate <estate> --mode cloud`). Uses Service Account impersonation and a GCS backend for all subsequent operations.

### Which identity a command runs as

satz owns no OAuth client and writes no credential of its own. The Application Default
Credentials are gcloud's; satz reads them and, for a `deployment_mode = "cloud"` estate,
impersonates that estate's IaC service account — the same identity the emitted provider
block gives `tofu`, derived from the same two params.

The rule: **post-init, anything that reads or writes a customer's estate runs as that
estate's service account.** Every exception is listed here with its reason.

| Command | Runs as | |
|---|---|---|
| `export-`/`diff-`/`report-organizational-policies`, `report-compliance`, `adopt`, `adopt-org-policies` | the estate's service account | |
| `import --into <estate>` | the estate's service account | it runs `adopt`'s read path, so it runs `adopt`'s identity |
| `import` without `--into` | the human's ADC | the output is a new file; there is no estate to be |
| `bootstrap`, `init --from-live` | the human's ADC | day 0 — the service account does not exist yet |
| `whoami` | the human's ADC | the question *is* who the human is |
| `whoami <estate>` | the estate's service account | a different question — who that estate acts as — so a different answer |
| `map-types` | no credential at all | Discovery documents are public |
| `mcp` | per tool call, from the estate that is open | one server, a fleet: `satz_open` moves to the next estate, and the identity follows it |
| `plan`, `apply`, `hcl-init` | `tofu`'s own resolution | satz passes it no token; the provider block impersonates |

`--no-impersonate` pins the process to the plain ADC and outranks every estate.

**Where the credential comes from.** AIP-4110 and nothing else:
`GOOGLE_APPLICATION_CREDENTIALS`, then the well-known path
(`~/.config/gcloud/application_default_credentials.json`). satz, `google-cloud-auth`
and the Go SDK behind `tofu` all resolve those two, so all three agree.

To work against a second customer without disturbing the first, give gcloud its own
configuration directory and name the file it writes:

```bash
CLOUDSDK_CONFIG=~/.gcloud-acme gcloud auth application-default login
export GOOGLE_APPLICATION_CREDENTIALS=~/.gcloud-acme/application_default_credentials.json
satz whoami          # names that file, and mints from it
satz whoami e.satz   # who that estate's live commands run as
```

`CLOUDSDK_CONFIG` alone is not enough: gcloud reads it, satz and `tofu` do not.

**`whoami` reports both halves, and checks them.** The ADC account is who you are to
Google; the estate's service account is who satz then acts as, and after init that
is what every read and write runs as. `whoami <estate>` prints both, and — online
— makes the two calls that decide whether the next command will work at all: one
`generateAccessToken` to see whether this credential may become that service
account (the token is discarded), and one `projects.get` on the quota project.
The quota project must be one the credentials can reach
(`gcloud auth application-default set-quota-project <project>`). One they cannot
reach is accepted by every command that only prints it, and then fails every API
call with `UserProjectInvalid` or "cannot create the authentication headers", an
error that names neither the project nor the fix. Every live command therefore
checks it once before its first call and refuses; `whoami` reports it and exits
non-zero.

Online, given an estate that compiles, `whoami <estate>` also tests the permissions the
estate's resource types need (`testIamPermissions`) with the credential the estate's
live commands run as: organization needs on the organization, project needs on the infra
project, which inherits the organization's grants as every project does, and
billing-account needs on the billing account. A missing permission is named with the
role that carries it, and `whoami` exits non-zero. The Workspace Groups Admin role is
named as not tested.

**If your ADC already impersonates** — `gcloud auth application-default login
--impersonate-service-account` — satz uses it as-is when it names the estate's own
service account, and refuses when it names a different one; it neither chains the
two nor picks one.

**One command serves one identity.** On the command line the identity is bound for the
process, and a second, different binding is refused, naming both. `satz mcp` is
long-lived and works through estates in turn, so it scopes the identity to each tool
call instead: the call runs as the service account of the estate that is open. It is
a scope, not a process-wide binding, because the server dispatches calls
concurrently, and a binding that changed under a call in flight would run one
estate's tool as another estate's service account.

**The state bucket runs as the same identity.** In cloud mode the emitted `gcs` backend
carries `impersonate_service_account` too, so state reads and writes use the estate's
service account, not the human: one `tofu apply` authenticates as one principal,
and no operator needs standing object access on the state bucket. An
estate that declares its own
`impersonate_service_account` on the backend keeps it.

> **When the emitted backend changes,** `tofu` refuses the next command until the
> backend is re-initialised: run `tofu init -reconfigure` once in `hcl/`.
> `satz migrate --mode cloud` re-initialises by itself.

**Who may become the IaC service account.** The estate `satz init` writes grants the
`svc-iac-users` group `roles/iam.serviceAccountTokenCreator` and
`roles/iam.serviceAccountUser` on the IaC service account itself
(`google_service_account_iam_member`), so a member can act as that account and no other
service account in the organization. Membership of the group is the operator's grant.

**What the IaC service account holds.** Named roles at the organization and
`roles/billing.admin` on the billing account — the reads and the roles its resource
types need, as [IaC service account roles](#iac-service-account-roles-iac-roles)
describes. `satz iac-roles --execute` adds the ones a new pack brings.

## License

This project is licensed under the MIT License - see the [LICENSE](https://github.com/tjirsch/satz/blob/main/LICENSE) file for details.
