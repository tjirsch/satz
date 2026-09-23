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
- `--silence <KIND[:SUBJECT]>`: leave a finding out of this run's output, by what it is (see [Silencing a finding](#silencing-a-finding-silence)). Repeatable; `SATZ_SILENCE` takes the same selectors comma-separated. The finding is still produced, still counted and still in `--format json`; an error is never silenced, and a `--silence` that names one refuses the run. Refused for `satz mcp` and `satz lsp`, which serve many estates in one process.
- `--no-impersonate`: run as the Application Default Credentials themselves, without becoming the estate's IaC service account — for a check that must answer as the human.
- `--no-api-preflight`: `plan` and `apply` start the tool without asking Service Usage which of the APIs the estate declares are off, and enable none — for a run that must reach the tool without satz calling Google.

### User settings (~/.config/satz/satz.toml)

User-level **parameters** (e.g. when to check for updates) live in **`~/.config/satz/satz.toml`**. This file is **created on first run** with default values (e.g. `self_update_frequency = "always"`). If the file is missing on load, it is created with defaults.

| Option | Default | Description |
|--------|---------|-------------|
| `self_update_frequency` | `"always"` | When to check for updates on normal runs: `never`, `always`, or `daily` (at most once per 24 hours). The check only reports a newer version; it installs nothing. |
| `[[silence]]` | none | Finding kinds this machine leaves out of every estate's printed output, each with a `reason`. Whole kinds only — a `subject` here is refused, because it belongs to one estate. Managed by `satz silence add --machine`; see [Silencing a finding](#silencing-a-finding-silence). |

**Project config** (paths, providers, etc.) stays in **`config.toml`** per project; see [Configuration](#configuration) below.

Example (optional; the file is created automatically when needed):

```toml
self_update_frequency = "daily"
```

## Installation

### Using cargo-dist Installer (Recommended)

Releases carry binaries for macOS on Apple silicon, Linux on x86_64 and ARM64, and
Windows on x86_64. On an Intel Mac or on ARM64 Windows the installers stop with an error;
build from source there with `cargo install --git https://github.com/tjirsch/satz --locked`.

Install the latest release using the cargo-dist installer:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tjirsch/satz/releases/latest/download/satz-installer.sh | sh
```

This will install `satz` to `~/.local/bin` and automatically add it to your PATH if needed.
The installer checks the archive it downloads against its sha256 with `sha256sum`, and
skips that check where the command is missing; check the archive yourself then, against
the release's `sha256.sum`, with `shasum -a 256 <file>`.

> **Note:** The installer will:
> - Install the binary to `~/.local/bin`, and its PATH helper to `~/.config/satz/env.sh`
>   (`env.fish` for fish), beside its install receipt
> - Source that helper from your shell profiles (`.zshrc`, `.bashrc`, `.bash_profile`,
>   `.profile`, and fish's `conf.d`). A profile line that sources `~/.local/bin/env` — a
>   helper other installers into `~/.local/bin` share — is rewritten to the new one, and
>   `~/.local/bin/env` is moved there
> - Provide instructions to refresh your shell
>
> Removing satz means `~/.local/bin/satz`, `~/.config/satz/`, and those profile lines.
>
> If you prefer a different location, you can override it:
> ```bash
> curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tjirsch/satz/releases/latest/download/satz-installer.sh | CARGO_DIST_FORCE_INSTALL_DIR=/your/custom/path sh
> ```

### On Windows

satz builds for Windows on x86_64 and installs with PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/tjirsch/satz/releases/latest/download/satz-installer.ps1 | iex"
```

It installs `satz.exe` to `%USERPROFILE%\.local\bin` and adds that directory to your user
PATH. The PowerShell installer does not verify the archive it downloads; check it against
the release's `sha256.sum` with `Get-FileHash <file> -Algorithm SHA256`.

What differs on Windows:

- **Settings** are read from `%USERPROFILE%\.config\satz\satz.toml`.
- **`satz self-update`** refuses and names the PowerShell command above; `--check-only` works.
- **`--out -`** writes a report to stdout; a `/dev/…` path is refused.
- **An action** whose `run` is a script (`.sh`) is refused before it runs, naming the
  `bash …` line to run it by hand from Git Bash or WSL; an `.exe`, `.cmd` or `.bat` runs.
- **`generate-migration`** writes a bash script.
- **Line endings:** a CRLF checkout compiles, formats and compares like its LF twin.

### From Source

Install directly with cargo:
```bash
cargo install --path .
```
This builds the release binary and installs it to `~/.cargo/bin` (no sudo required); ensure that directory is on your `PATH`.

## CLI Usage

All commands accept the [global options](#global-options) (`--config`, `--validation`, `--verbose`, and the three `--no-*action*` switches below), before the command or after it. `satz --help` lists them; a command's own help lists only that command's options. `satz <command> -h` is the one-line-per-option summary, `--help` the full text (both wrap to your terminal), `--html-help` opens the command's section on the documentation site. The groups below are the ones `satz --help` prints, in the same order:

Every reporting command takes the same two arguments: `--format`, the rendering, and `--out`, the file it lands in. A command's `--help` lists exactly the formats it writes and anything else is refused naming them; a command that writes `markdown` writes `pdf` too, the same document typeset. A table in a PDF is laid out from what it holds: a column of status glyphs is as wide as a glyph and the prose columns share the rest in proportion to their text, the first row is a header that repeats on every page the table spans, and a document that holds a table of five columns or more is landscape from its first page to its last. `--out` may name the file with its extension or without one — `--format pdf --out evidence/cis` writes `evidence/cis.pdf` — and a name ending in another format's extension (`--format pdf --out cis.md`) is refused. One invocation produces exactly one artefact at exactly one named path and says on stderr where it went, so nothing reaches the console that nobody asked for and `--format json --out - | jq` is a clean pipe. Three commands answer on the console instead, because what they produce is not a document: `update-prerequisites`, which edits the estate and reports what it wrote, `prowler`, which prints a command line to paste, and `mcp-config`, which prints the block an MCP client reads. `remediation-plan` and `doc-packs` write several files each, so they take `--out-dir <DIR>`.

**Estate**

| Command | Options / Arguments |
|---------|---------------------|
| `init` | `--defaults`, `--providers`, `--tf-tool`, `--customer-id`, `--customer-shortname`, `--billing-account-infra`, `--customer-organization-id`, `--customer-domain`, `--iac-user`, `--default-region`, `--infra-project-name`, `--infra-bucket-name`, `--force` (rewrite an existing estate instead of merging into it), `--interview` (ask for what is still unbound) |
| `bootstrap <ESTATE>` | `--dry-run` (read-only incl. the permission pre-flight), `--greenfield` (materialize an organization for a tenant nobody has signed in to the console with), `--no-default-grants` (never widen the caller's own IAM) |
| `transpile <INPUT>` | `--output`, `--schema-dir`, `--print-variables`, `--check` (compile in memory, write nothing), `--format` (`text`\|`json` — `json` prints the compile as data, see [How a finding is printed](#how-a-finding-is-printed)), the first line of `main.tf` names the satz that emitted it, `--plan` / `--apply` (then run the tool in `hcl_dir`), `--scan` (then Checkov) |
| `import [SOURCE]` | `--from` (`state`\|`org`\|`hcl`), `--all`, `--only <types>`, `--exclude <types>`, `--output` (default: `discovered.satz`), `--import-config`, `--into <estate>` (live: only the delta), `--as <estate>` (live: read as that estate's service account), `--on-collision error|counter`, `--customer-shortname`; live shape: `--generate-unmapped`; hcl shape: `--wrap-all` |
| `adopt <INPUT>` | `--execute`, `--import`, `--activate`, `--only <types>` — dry run by default, and the dry run reads the state so a resource it already manages says so instead of counting as an import; exits non-zero on any failed/unresolvable/ambiguous row; `--import` reads `state list` first and skips already-managed addresses, and a run over every type that finishes with nothing unresolved acknowledges the packs' notices that name `satz adopt` |
| `update-prerequisites [INPUT]` (alias `prerequisites`) | `--report-only`, `--format` (`text`\|`json`) — what the estate's resource types oblige it to declare and it does not: the roles its IaC service account is missing, and the APIs its infrastructure project does not enable. Writes both into the estate file and re-checks; `--report-only` lists them and exits non-zero. Without an estate: the table of resource types, roles and APIs. See [What an estate must declare](#what-an-estate-must-declare-update-prerequisites) |
| `packs <INPUT>` | `--format` (`text`\|`markdown`\|`pdf`\|`json`), `--out <FILE>` — every pack the pack graph in `presets_dir` offers, as this estate has it: the gate's answer and default, the line (`active`, `ungated`, `commented`, `absent`, `forked`, `misplaced`), whether the pack deploys, what it needs and what needs it, the notices it carries with their severity and their state, and the compile's pack findings. A `use` the graph does not know is listed as `unmanaged`. See [the pack graph](docs/language.md#616-offers--what-the-library-offers-an-estate) |
| `add-pack <INPUT> <PACK>` | `--with-requirements`, `--format` (`text`\|`json`) — `<PACK>` is a gate or a pack path. Binds the gate true (an option of a choice sets its siblings false) and makes the pack's line active where the pack graph places it, with the packs whose gate follows it; prints the questions and the notices that opened. Refused, naming them, while a pack it needs is off — `--with-requirements` switches those on where the graph names one — or a pack it excludes is on. The edited estate is compiled and restored when it does not compile |
| `remove-pack <INPUT> <PACK>` | `--cascade`, `--format` (`text`\|`json`) — binds the gate false and leaves the line: a gated line with a false gate deploys nothing. Refused, naming them, while a pack that needs it is on — `--cascade` switches those off too — or while the pack's line is not gated on its gate. The edited estate is compiled and restored when it does not compile |

**HCL**

| Command | Options / Arguments |
|---------|---------------------|
| `hcl-init [ARGS]` | runs `<tf_tool> init` in `hcl_dir`; everything after the command is handed to the tool verbatim, so `--config` must come before it and the estate is no argument — it is the one `--config` names |
| `plan [ARGS]` | runs `<tf_tool> plan` in `hcl_dir`, arguments passed through; an org policy the state holds with rules and the estate declares reset gets `-replace` |
| `apply [ARGS]` | runs `<tf_tool> apply` in `hcl_dir`, arguments passed through; an org policy the state holds with rules and the estate declares reset gets `-replace`, which `main.tf` also names in a comment above that policy for an apply without satz |
| `migrate <INPUT>` | `--mode` (`local`\|`cloud`; without it, the other of the two) |
| `scan-plan <plan_json>` | `--output` (default: `mapping.yaml`) |
| `generate-migration <mapping>` | `--output` (default: `migrate.sh`) |
| `run-actions <INPUT>` | `--check` (each action's own dry-run form), `--execute` (the form that writes), `--only <names>`, `--phase <before-apply\|after-apply>` — prints what it would run and stops by default |

**Presets**

| Command | Options / Arguments |
|---------|---------------------|
| `get-presets` | `--force` — overwrite presets the estate uses too; `--pristine-dir` |
| `merge-presets` | `--pristine-dir`, `--estate`, `--report-only`, `--adopt <stem\|all>` — reconciling update; `--adopt` upgrades in place instead of forking. Writes the commented line for every pack the pack graph of the pristine source offers and the estate lacks — the whole menu into an estate that has none — and gates every active line of a gated pack written without `when`, binding its gate `true` |
| `check-presets <INPUT>` | `--format` (`text`\|`json`), `--out <FILE>`, `--pristine-dir` |
| `review-pack <PACK>` | `--against <ESTATE>`, `--format` (`text`\|`json`), `--out <FILE>` — one pack against the library's bar, as the same findings the compile and the editor read: it parses, it is formatted, its header says what it is, its version has a changelog row, it carries no value shaped like private data, it declares no membership, it runs no legacy org-policy constraint beside its managed replacement, every resource type it emits has a prerequisite row, and it compiles. Exits non-zero when it does not clear the bar. See [Reviewing a pack](#reviewing-a-pack-review-pack) |
| `pack-graph` | `--presets-dir <DIR>` (default `presets_dir` from the config), `--check` — checks the library and writes `<presets_dir>/pack-graph.json`, the pack graph that ships with the presets: every pack with its gate, phase, block and adoption order from the map's `offers` entries, and the edges between packs — derived from their param references and `ask_when`, declared on the entries where the packs do not show them. Nothing is written while a check fails; `--check` fails when the file is behind the library. See [The pack graph](docs/language.md#616-offers--what-the-library-offers-an-estate) |
| `doc-packs` | `--out-dir <DIR>` (default `<presets_dir>/docs`), `--check` — one Markdown page per pristine pack, derived from the pack file, plus a grouped index; `--check` fails when the pages are behind, a claim names a control its catalog lacks, a pack header says nothing the index can print, or a pack version has no changelog row |

**Policies**

| Command | Options / Arguments |
|---------|---------------------|
| `export-organizational-policies <ESTATE>` (alias `export-org-policies`) | `--customer-organization-id`, `--output` |
| `diff-organizational-policies <ESTATE>` (alias `diff-org-policies`) | `--customer-organization-id`, `--format` (`text`\|`markdown`\|`json`), `--out <FILE>`, `-r/--recursive` (every folder and project below) |
| `report-organizational-policies <ESTATE>` (alias `report-org-policies`) | `--customer-organization-id`, `--scope` (`active`\|`inactive`\|`full`), `--format` (`markdown`\|`json`\|`pdf`), `--out <FILE>`, `-r/--recursive` |
| `adopt-org-policies <INPUT>` | `--dry-run` — alias of `adopt --only google_org_policy_policy --activate --execute --import` |

**Compliance and audit**

| Command | Options / Arguments |
|---------|---------------------|
| `questions <INPUT>` | `--format` (`text`\|`json`\|`markdown`\|`xlsx`, the decisions catalog as a workbook a customer fills in and sends back), `--out <FILE>`, `--unanswered` — every question the estate's packs declare with its state: `answered` when the estate's own params bind it, else `unanswered` with the default the pack offers or `blocking` when none is possible. `markdown` is the decisions sheet; `summary.complete` is the gate `bootstrap` and `transpile --apply` refuse on |
| `interview <INPUT>` | `--create`, `--all`, `--accept-defaults` — asks the open questions one at a time at the terminal and writes each answer into the estate's params; `--create` writes the estate first from `presets/estate-core.satz`. A yes to a pack's question switches its line on as `add-pack` does — refused, naming it, while a pack it needs is off — and prints the notice that pack carries. An answer that would leave an estate satz refuses is refused and writes nothing. See [satz interview](docs/interview.md) |
| `require <FRAMEWORK> <INPUT>` | `--format` (`text`\|`json`), `--out <FILE>`, *(catalog id, e.g. `cis-gcp-4.0`)* |
| `report-compliance [<FRAMEWORK>] <INPUT>` | `--format` (`markdown`\|`json`\|`pdf`), `--out <FILE>`, `--prowler`, `--checkov`, `--no-live`, `--fail-on <statuses>` |
| `scan [<INPUT>]` | Checkov over `hcl_dir`; with the estate, each finding is pointed at the Satz block that declared the resource; failed checks exit 1 |
| `prowler <INPUT>` | `--format` (`text`\|`json`) — the Prowler invocation this estate needs, printed. `text` puts the command line on stdout and NOTHING else, so it can be pasted into a shell or piped to a clipboard; what the line cannot say — a scan not narrowed to projects, a project left out of `--project-ids` because its id is built from a reference to another resource (which only an apply resolves), a framework the estate names that Prowler has no equivalent of, an estate that binds no `compliance_frameworks`, the command to run afterwards — goes to stderr. The organisation and the project ids the estate declares (a `{param}` in an id is its value), as `--compliance` the union of the frameworks it is HELD TO (`compliance_frameworks`) and the frameworks its packs CLAIM (which also filters which checks run), `--output-formats json-ocsf`, and an output file under `evidence/prowler/<UTC date>/` named for the scope and the UTC minute (`org-2026-09-13T08-30Z.ocsf.json`) — Prowler appends to an output file that already exists, so each scan needs a name of its own: run `satz prowler` again for the next scan. satz never runs Prowler: the scan spends API quota in every project, and Prowler reads as whoever is logged in rather than as the estate's service account |
| `triage <FRAMEWORK> <INPUT>` | `--prowler <file>` (required), `--format` (`markdown`\|`json` — `{"rows": […]}`, the value the MCP tool `satz_triage` returns), `--out <FILE>`, `--fix` — every Prowler FAIL sorted into who-fixes-it buckets against the estate's claims, and the checks Prowler maps to no control of the framework counted in a section of their own; `--fix` adds the estate delta they imply to the report (markdown only, proposed, never written) |
| `remediation-plan <FRAMEWORK> <INPUT>` | `--prowler <file>` (required), `--checkov`, `--out-dir <DIR>`, `--merge <authored.json>` — the remediation dossier: triage joined with Checkov per resource, counted, written as `dossier.json` + `findings.csv` + `findings.xlsx` (mechanical columns filled, `[Authored]` columns and who authored them, Review dropdown) + `meta.json` under `evidence/plan/<framework>-<UTC minute>/` (`cis-gcp-4.0-2026-09-13T08-30Z`, dashes for colons so the name is valid on every platform) unless `--out-dir` names another — a run in a minute that already has a folder takes the next free name (`…_002`), created by that run, so no run writes into another's; offline and deterministic (the dossier hash names the run). `--merge` fills the `[Authored]` columns from an `authored.json` written against this run's hash — every entry names `authored_by` and `authored_at` — and keeps it beside the run; `dossier.json` and its hash do not change |

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
| `mcp` | `--allow` (`read`\|`write`\|`exec`, comma-separated; default `read`), `--self-gated`, `--root <DIR>` (the directory the server may work under; default the current one) — serve the estate over the Model Context Protocol on stdio, so an agent drives satz. 25 tools: each data tool returns structured content with a published output schema, and every tool is annotated so a client knows which are safe to run unattended. satz calls no model; the agent calls satz. See [docs/mcp.md](docs/mcp.md) |
| `mcp-config <INPUT>` | `--client` (`claude-code`\|`claude-desktop`; default `claude-code`), `--allow` (`read`\|`write`\|`exec`, comma-separated; default `read`), `--write`, `--force`, `--file <FILE>`, `--name <KEY>` — the MCP client configuration this estate needs, printed on stdout and nothing else, so it can be piped into a file or a clipboard; what the block cannot say goes to stderr. See [Configuring an MCP client](#configuring-an-mcp-client-mcp-config) |
| `whoami [INPUT]` | `--offline` — print BOTH halves of the identity: the ADC account and its file, and what the estate's live commands run as — in cloud mode its IaC service account, impersonated by the ADC account; in local mode the ADC account itself, with the `satz migrate` that switches to the declared account — checked: may this credential become that account, is the quota project reachable, and does it hold the permissions the estate's resource types need |

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
- Values come from three places and no fourth: what you state on the command line, what the Application Default Credentials can answer, or empty. Derivation is automatic and needs no flag — the identity gives `first_admin` and `customer_domain`, `organizations:search` gives `customer_organization_id` and the `C0…` directory customer, `billingAccounts.list` gives the account when exactly one is open, and `infra_project_name` / `infra_bucket_name` follow from `customer_shortname`. Each derived value is printed with the source it came from. What nothing can answer is written `""` and named, and `satz bootstrap` refuses by param until it is set — a placeholder would look like an answer. `--from-live` is accepted and ignored; it is what init does now.
- `--interview`: a day-0 param is stated, derived or ASKED — there is no fourth state where the estate is simply born incomplete. With this flag init hands the estate it just wrote to `satz interview`, which asks for whatever is still unbound. It is a flag rather than the default because the interview is interactive and a scripted run must not block on it.
- The pack menu — one commented `use … when` line per pack, under the phase it can be adopted in — is written from `pack-graph.json` in `presets_dir`, the pack graph that ships with the presets. With no graph there the estate is written without pack lines, and init says so: `satz get-presets`, then `satz merge-presets`, write them where the menu goes. A graph that places a pack in a block this binary's scaffold does not have is refused before anything is written; `satz self-update` is the way through.
- Running `init` again on an estate that exists MERGES: the params this command line names are written in, every other line is left exactly as it is, and each one is reported as set, changed or kept. `--force` rewrites the file instead.

**Under the Hood:**
- Creates the standardized directory structure: `yaml/`, `hcl/`, `schemas/`.
- Generates a default `config.toml` and `.gitignore`.
- If customer details are provided, generates the Day-0 estate `yaml/<customer-id>.satz` (params, providers, the IaC group and service account, the management folder/project/state bucket — the labels `bootstrap` imports by name).
- Fetches the latest provider schemas for the configured providers.

**Without the flags:** `satz interview yaml/<name>.satz --create` writes an estate that
asks for the same seventeen values one at a time and offers the derived ones as defaults;
an agent does the same over MCP with `satz_interview`. Either way `bootstrap` refuses until
every question is answered — [satz interview](docs/interview.md).

### Day 0 Bootstrap (`bootstrap`)
`bootstrap` runs the day-0 onboarding of a new customer organization.

```bash
satz bootstrap <ESTATE> [options]
```

**Parameters:**
- `<ESTATE>`: The estate file (e.g. `C0example.satz`). Relative paths are looked up inside `yaml_dir`, so pass the bare filename — **not** `yaml/C0example.satz`, which would resolve to `yaml/yaml/C0example.satz`. This is not the tool config; that is `--config`.
- `--dry-run`: Simulation mode; does not create resources.
- `--no-default-grants`: satz never widens the caller's own IAM. Where the pre-flight would self-grant the missing roles at the scope root, it prints the `add-iam-policy-binding` commands for an administrator and stops before creating anything — the same path a caller who cannot self-grant already takes. In a change process that audits organisation-level IAM, acquiring `folderAdmin` and `orgPolicyAdmin` at the root is the reportable event, and printing the undo afterwards does not unmake it. Without the flag the self-grant stays the default, announced with its `remove-iam-policy-binding` undo.
- `--greenfield`: materialize an organisation that does not exist yet, for a tenant whose admin has not signed in to the Google Cloud console — a sign-in that accepts the terms creates the organisation, and then `init` derives its id and plain `bootstrap` is the path. The infrastructure project is created without a parent, the organisation Google creates for the estate's directory customer is found by polling, the project is moved under it and the organisation id is written back into the estate.
**Tip:** Use `--dry-run` to see what resources would be created without making changes.

**Tip:** What `bootstrap` creates is declared in the estate — the folder, the management project and the state bucket, under the labels it imports them to — so after the import the estate manages them like any other resource. `deployment_mode` is `local` or `cloud`: `local` until `satz migrate --mode cloud`.

**Under the Hood:**
1.  **Authentication**: Uses Application Default Credentials (ADC).
2.  **Infrastructure Folder**: Lists every folder under the parent (all pages) and reuses the one whose display name matches — exactly one; two folders with that name is an error, not a guess — or creates it (requires `Folder Admin`).
3.  **Project Shell**: Creates the management project — its id is the estate's `infra_project_name`, which bootstrap refuses to go without — inside the folder, or reuses an existing one, and prints its **project number**.
4.  **Billing Link**: Links the project to the specified Billing Account.
5.  **Enable APIs**: Enables the foundation APIs (Service Usage, Cloud Resource Manager, IAM, IAM Credentials, Storage, Cloud Billing, Cloud Identity, Cloud Asset, Logging, Org Policy, Essential Contacts).
6.  **State Bucket**: Creates the GCS bucket for Terraform state (with versioning, uniform access).
7.  **Automated Setup**:
    - **Transpile**: Compiles the estate to HCL.
    - **Init**: Runs `tofu init` to download plugins.
    - **Import**: Automatically imports the created Folder, Project, and Bucket into the local state.

### Reviewing a pack (`review-pack`)

A pack is the unit everyone extends satz with, and the bar it has to clear is real: it
is what the library's own gates enforce on every pack in `presets/`. Those gates need a
satz checkout and a Rust toolchain, so a pack written anywhere else could not be checked
at all — its author found out by opening a pull request, or never. `review-pack` is that
bar as a command.

```bash
satz review-pack my-pack.satz --format text --out -
satz review-pack my-pack.satz --format json --out review.json      # the findings as data
satz review-pack my-pack.satz --against C0example.satz --format text --out -
```

It checks, in the order a pack fails them: it **parses**; it is **formatted** (`satz fmt
<file>` is the whole fix); its **header** opens with a sentence saying what it is, which
is what the pack index prints; it declares a **version** in-file and that version has a
row in the library's `## Changelog`; it carries **no private data** — no token shaped like
an organisation, folder or project number, a directory id, a billing account, a GUID, a
project id, an e-mail address, a domain or a repository URL that is not one of the
documented example values (`docs/examples.md`); it declares **no membership** — presets define
groups, humans grant membership; it runs **no legacy org-policy constraint beside its
managed replacement** (`presets/managed-constraint-equivalents.txt`); every **resource
type it emits has a row** in satz's prerequisite table, so the roles and the API it needs
are known; and it **compiles**. A type `satz adopt` has no rule for (no `import_id:` or
`match_on:` in `import-config.yaml`) is a warning: once such an object exists — a console
click, a partial apply — only `tofu import` by hand brings it under management. It also says what adopting the pack costs an estate — the
roles and the APIs `satz update-prerequisites` would write — and names the questions a
customer must answer because no default is possible.

A pack is a fragment, so satz folds it into an estate to see what it emits: a synthesised
one, with the documented example values for the estate's own params and the pack's own
declared defaults for its questions, which checks the pack the way a customer first meets
it. `--against <estate>` judges it inside a real estate instead.

The findings are the same `Finding` the compile, `satz lsp` and `satz_transpile_check`
produce — severity, kind, file, line, message, fix — so an editor or an app that already
reads those needs nothing new to show them, and the text report prints them as every
command does ([How a finding is printed](#how-a-finding-is-printed)). `satz_review_pack` serves the same review over MCP,
read-only. The command exits non-zero when the pack does not clear the bar.

Each private-looking token is an error of kind `private-shape` at its line, naming the
token: a pack goes upstream with a param in its place, or with the documented example
value. The rules and their allow-lists are the ones `scripts/check-names.sh` holds this
repository to ([housekeeping](docs/housekeeping.md#check-namessh--the-privacy-gate)): what
`review-pack` flags in a pack is what that gate flags in it.

### What an estate must declare (`update-prerequisites`)

The estate's IaC service account — `svc_iac_account` in `infra_project_name` — holds
named roles at the organization, not `roles/owner`. A role granted at the organization
is inherited by every folder and project under it, including those created by hand or
before the estate, so the same roles reach them.

A resource type obliges the estate to declare two things, and `update-prerequisites`
derives both from the types the estate emits: the ROLES that account needs, against the
roles the estate grants it, and the APIs that serve those types, against the
`project_service` list of the infrastructure project. It writes what is missing into
the estate file, because it has to be there either way.

```bash
satz update-prerequisites <INPUT>                # writes the missing roles and APIs, then re-checks
satz update-prerequisites <INPUT> --report-only  # lists them instead; exits non-zero while anything is missing
satz update-prerequisites --format json          # the table: per resource type, its permission, roles and API
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
  (`roles/resourcemanager.projectMover`) and the billing link. `satz
  update-prerequisites` without an estate prints the whole table; its source is
  `src/prerequisites.rs`.
- Organization and project needs are met by a role granted at the organization, and
  `roles/owner` there meets all of them. Billing-account needs are met by a grant on
  the billing account (`google_billing_account_iam_member`).
- `google_cloud_identity_group` needs the Groups Admin role of the Google Workspace
  admin console. It is not an IAM role, so `testIamPermissions` cannot see it: `whoami
  <estate>` asks the IaC service account first, as itself, and your login only when the
  service account lacks the role; `migrate --mode cloud` then assigns it, when your login
  carries the role-management scope.
- No role in the table deletes a project. Google makes a project's creator its owner,
  so the account deletes the projects it created; a project it did not create is
  deleted by a person, or with `roles/resourcemanager.projectDeleter` granted for the
  deletion. `google_project` refuses a delete unless its `deletion_policy` is
  `"DELETE"`.

The roles granted are the `google_organization_iam_member` and
`google_billing_account_iam_member` grants to
`serviceAccount:<svc_iac_account>@<infra_project_name>.iam.gserviceaccount.com`, in the
estate and in every pack it uses.

**What the estate must enable.** The estate's `google` provider carries
`user_project_override` with `billing_project = infra_project_name`, so Google bills
every call it makes to the infrastructure project and requires the API enabled THERE,
whatever the resource's own scope is — a budget hangs off the billing account and an
org policy off the organization, and both still need their API on that project. A
resource written inside a `google_project { … }` is served by that project's provider
alias, which is billed to the project itself and needs the API on it. The APIs are
judged against the infrastructure project's `project_service` list.

**The emitted HCL carries the ordering.** A resource waits for the
`google_project_service` that enables its API — `depends_on`, added by the compiler for
the resource's own project and for the infrastructure project — because
`google_project_service` has no ordering of its own and one apply can otherwise create
a resource before its API is on. A service is never ordered behind a service, and no
edge is added into a service block's own dependencies, so the project a service is
declared on and the folder above it never wait for it.

**Declaring an API does not switch it on.** `tofu apply` refreshes every resource in
state before it creates anything, and that refresh is billed to the infrastructure
project too, so an API the estate declares and the project has off stops the run before
the `google_project_service` that would enable it is created. `satz plan` and `satz
apply` enable what is off first — see [The API preflight](docs/workflows.md#the-api-preflight).
`update-prerequisites` writes the declaration and enables nothing; it prints the line
that does, for an apply run with `tofu` directly:

```
gcloud services enable monitoring.googleapis.com --project corp-infra-001
```

The same line is `enable_missing_apis` in `--format json`.

**The write** adds each missing role to the account's existing grant list — the list
whose key names the account once `{param}`s are interpolated — or appends a new block
when the estate has none. It writes the fewest roles: a need only one role meets takes
that role, and a need with alternatives takes a role already chosen. A new
billing-account block is itself a `google_billing_account_iam_member` and needs
`roles/billing.admin`, which also carries the billing link, so that is the role written
there. Each missing API is spliced into the infrastructure project's `project_service`
list, or a list is created under its `project_id`; the list is only ever added to,
because the emitter derives `google_project_service.<project label>_<service>` from it
and the CIS pack claims 5.0 §2.14 against one of those addresses. The command then
compiles the estate again and restores the file when anything is still missing — both
halves are written before either is verified, so the estate is never left half-edited.
An infrastructure project declared outside the estate file (in a pack, which the next
`merge-presets` would overwrite) is named and nothing is written.

**Every compile checks the same**, at the [validation level](#schema-validation): `warn`
(the default) prints the missing roles and APIs with the `update-prerequisites` command
that writes them, `error` refuses the compile, `none` skips the check. The two commands
that write the gap, `update-prerequisites` and `merge-presets`, report it as what they
write, so neither refuses on it. An estate that
names no IaC service account is not role-checked; one that binds no
`infra_project_name` is not API-checked. A resource type the table has no row for is
named in a note.

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
Compiles the estate to HCL. Input is a `.satz` estate; a `.yaml` estate is written in the pre-Satz YAML dialect and is refused by name, with the release that converts it.

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

Above each org policy the estate declares `reset = true`, `main.tf` carries two comment lines: the API refuses to switch a policy with rules to reset in place (`Cannot set PolicyRules if reset is true`), and an apply that does not run through satz needs `tofu apply -replace=<address>` while the state holds that policy with rules. `satz plan` and `satz apply` add that `-replace` themselves, and enable the APIs the estate declares on the billed project before the tool starts. See [Transpile, plan, apply](docs/workflows.md#transpile-plan-apply) and [The API preflight](docs/workflows.md#the-api-preflight).

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
use "presets/cis/CIS-GCP-Foundation-4.0.satz" when use_cis_baseline

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

// once reviewed, state why — the warning becomes an info
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

Curated Organization Policy sets (e.g. `presets/cis/CIS-GCP-Foundation-4.0.satz`) are normally
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
satz diff-organizational-policies C0example.satz --format markdown --out diff.md
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
satz report-organizational-policies C0example.satz --scope full --format markdown --out policies.md
```

`--scope`: `active` (set policies), `inactive` (available but unset), or `full` (both,
with constraint descriptions pulled from the Org Policy constraints API). `--format pdf`
typesets the markdown itself — Typst is compiled into satz with its fonts, so a PDF needs no tool on `PATH` and renders the same bytes on every machine.

### Hoisted scopes: where resource types may live

Some resource types have one intrinsic scope no matter where they are written:

| Type | Intrinsic scope | Emitted with |
|---|---|---|
| `cloud_identity_group` | Customer | `parent = customers/<customer-id>` |
| `organization_iam_member` | Organization | `org_id = <customer-organization-id>` |
| `google_billing_account_iam_member` | Billing account | `billing_account_id` — an explicit `billing_account_id:` entry in any fragment, else `*billing-account-infra` |

Declaring these inside a folder block is therefore **grouping for humans, not
placement**: during transpile they are collected from everywhere in the tree and emitted
exactly once at their real scope. That makes a fragment file *cohesive* — a project can
travel with its org-level companions in one file, used at the top level:

```
// logging-project.satz — one file, one concern
pack logging_project version "1.0"

params {
  logging_prj_folder = ""
}

google_project {
  logging_prj {
    project_id = "{customer_shortname}-logging"
    folder_id  = logging_prj_folder
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
// the estate — the pack names the folder its project lands in
params {
  logging_prj_folder = "google_folder.shared_services.name"
}

google_folder {
  shared_services {
    display_name = "Shared Services"
  }
}

use "logging-project.satz"
```

The project is created in `shared-services`, which the file names through a param of its
own; the group and the grant are hoisted to their intrinsic scopes. A folder's and a
project's body hold the estate's own resources, and a `use` stands at the top level of a
file, in `google_folder { … }` or in a resource type map.

The position decides what a used file may hold: resource type maps at the top level, named
folders in `google_folder { … }`, labelled bodies in a resource type map. A file whose
entries do not fit where its `use` stands is refused, naming the entry. Its `params`,
`question`s and `claim`s reach the estate from every position and are never emitted into
the map the `use` stands in
([§6.9](docs/language.md#69-use--composition)). What a release refuses that the one before
it compiled, and the edit to make, is on
[the library page](presets/README.md#breaking-changes).

Merge rules when several fragments declare the same thing:

- **IAM grants are additive.** Same member from two fragments → role lists union; a
  deep-equal (member, role, condition) entry is deduped to one resource. There is no
  conflict state.
- **Groups must agree.** The same group key with a deep-equal body is deduped (including
  the same fragment twice is idempotent); with a *different* body the transpile aborts
  before writing any file, under `composition conflicts`:
  `google_cloud_identity_group.log-admins: 2 disagreeing definitions`, at both files and
  lines.
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
- `--mode <MODE>`: Target mode, `local` or `cloud`; without it, the other of the two. Any other value is refused before the estate is read.

**Under the Hood:**
- **Update the estate**: Binds `deployment_mode` in the estate's `params` block — the value is replaced where the estate binds it, and the line is added where it does not. An estate with no mode in its params — neither its own nor a pack's default — runs in `local` mode, as the emitter and `whoami` read it. `--mode cloud` on an estate that sets no `svc_iac_account` or no `infra_project_name` is refused before the file is touched, naming the param: cloud mode runs as the account the two name.
- **Regenerate**: Runs `transpile` to update the backend configuration (Local vs GCS) and provider authentication (ADC vs Impersonation).
- **Groups Admin** (`--mode cloud`, when the estate manages Cloud Identity groups): from here the IaC service account manages them, which needs the Workspace Groups Admin role. `migrate` checks in this order. First the service account, as itself: it lists the groups of the estate's `customer_id` directory through the Cloud Identity API, and when that succeeds the step ends there and asks nothing of your login. Only when the service account is refused is your login tested the same way; when your login lacks the role too, `migrate` names the admin-console path for a Workspace super admin. When your login holds it, `migrate` assigns the role to the service account through the Admin SDK Directory API, as you — the service account cannot give itself an admin role. A test that gives no answer (a disabled API, a quota project the caller may not use) is printed as that, never read as a missing role. The assignment needs a login carrying the role-management scope — `gcloud auth application-default login --scopes=https://www.googleapis.com/auth/cloud-platform,https://www.googleapis.com/auth/admin.directory.rolemanagement` — the Admin SDK API (`admin.googleapis.com`) on your quota project, and a Workspace admin who may assign roles. When any is missing, the migration goes on and prints which one, with the admin-console path (Account → Admin roles → Groups Admin → Admins → Assign service accounts).
- **Migrate State**: Executes `tofu init -migrate-state` to move the Terraform state to the new backend.

### Creating an estate from what exists (`import`)

One verb, three input shapes: a state file, the live organization, existing `.tf`
files. The result is a Satz estate that compiles as-is: a local backend, `customer_organization_id`, every
resource carrying its `"import-id"`, keys normalised to provider type names.
Review it, `satz transpile`, then `tofu plan` — the plan is the check: no destroy,
no unexpected create.

```bash
satz import state.json                       # a state file (tofu show -json / *.tfstate)
tofu show -json | satz import -              # …or on stdin
satz import organizations/123456789012       # live, whole org (Cloud Asset Inventory)
satz import folders/456789                   # live, one folder
satz import projects/my-prj                  # live, one project
satz import ./terraform                      # existing .tf: variables → params, resources → Satz, the rest verbatim in `hcl trust`
satz import ./terraform --wrap-all           # …or every block verbatim, nothing promoted
satz import                                  # live, root taken from the import config
satz import organizations/123456789012 --as C0example.satz     # read the organization as that estate's IaC service account
satz import organizations/123456789012 --into C0example.satz   # only what the estate does not declare
satz import organizations/123456789012 --generate-unmapped     # …and ask the provider for what satz cannot map (--into takes it too)
```

**Parameters:**
- `SOURCE`: what to import from; the shape is read off its form (`--from state|org|hcl` when it cannot tell). Omit it to use the import config's `root`.
- `--all`: every type the source can deliver, not only the rows marked `import: true` — at the live shape every row with a Cloud Asset Inventory name, from a state file every row. `--only` and `--exclude` apply after it.
- `--only <types>`: comma-separated resource types, `*` wildcards allowed (`google_*_iam_member`); everything else is switched off for this run. Overrides `only` in the import config.
- `--exclude <types>`: comma-separated resource types, `*` wildcards allowed; these are switched off for this run. Overrides `exclude` in the import config.
- `--output, -o <FILE>`: output inside `yaml_dir` (default `discovered.satz`, `imported-hcl.satz` for hcl; the extension is always `.satz`).
- `--import-config <FILE>`: the import configuration (default `presets/import-config.yaml`, or `import_config` in `config.toml`).
- `--as <ESTATE>` (live shape): read the scope as that estate's IaC service account, writing a new file rather than into the estate. `roles/cloudasset.viewer` on an organization satz set up is that account's, so the sweep is refused on your own credentials; naming the estate binds the account `tofu` applies with. `--into` names an estate already and binds the same way, so the two are refused together. Without either, the sweep reads as your own Application Default Credentials.
- `--customer-shortname <NAME>` (state and live shapes): the customer's short name, which no platform fact carries; it wins over the inference from the leading token of the project and bucket names.
- `--on-collision error|counter` (state and live shapes): a grant one principal holds on two folders or two projects would emit one address, because the map form's label is member and role. `error` (the default) refuses the import and names them; `counter` keeps the first in the map form and writes the second and later as labelled resources with a running number (`folderAdmin_alice_2`), one line of output each.
- `--generate-unmapped` (live shape): the resources the sweep reports as `unmapped` — a required attribute that is not in the asset data and cannot be derived, data that holds nothing the provider schema knows, a content type or scope no row covers — are handed to the provider instead of being left out. satz writes `<base>-generate/imports.tf` with one `import` block per resource (the id is the asset's relative resource name), runs `tofu init` and `tofu plan -generate-config-out=generated.tf` there, and reads the result back through the hcl shape into `<base>-generated.satz`. `<base>` is the file the run is named after: the estate a plain sweep writes (`discovered-generated.satz`), the scope's top-level pack with `--into` (`imported-organizations-123456789012-generated.satz`). With `--into`, what the estate already declares by that live id is named and left out, and the `tofu` child reads as the estate's IaC service account, through the `impersonate_service_account` of the provider block satz writes. The generated file is never `use`d from the estate: joining it is one line, and which of it belongs there is a reading decision. What satz cannot write a block for is listed with the reason; what the provider refuses fails the command with the tool's own output, and `imports.tf` stays for the ids to be corrected by hand. Refused on the state and hcl shapes.
- A `.yaml` source is the pre-Satz YAML dialect: it is refused by name, with the release that converts it (see [docs/language.md §12.3](docs/language.md#123-the-pre-satz-yaml-dialect)).

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
import-config row fits the asset), `ambiguous` (several fit), or `parent not imported`. Counts by reason
always — the filtered types as a count too; every name with `--verbose`. A resource
dropped because `--only` or `--exclude` left its parent's TYPE out is named in the normal
output with the type to add: `9 google_monitoring_alert_policy need google_project, which
--only/--exclude left out — add google_project to --only`. The levers are the `import:`
rows and `--only`.

**An asset type Cloud Asset Inventory does not serve is left out and named.** The
API answers `INVALID_ARGUMENT` for a whole request of a hundred types when one of
them is a type it has retired, naming none of them; satz asks the request again in
halves until the refusal is down to a single type, leaves that type out and fetches
the rest. The run ends with each one named, the Terraform rows that asked for it and
what the API said. Nothing of those types is in the estate: refresh the table with
`uv run scripts/update_import_config.py --cai-types presets/cai-asset-types.txt`, or
leave the rows out with `--exclude`. Any other failure — a scope the credential may
not read, a connection that broke — still ends the run with nothing written.

**Which Terraform type an asset becomes.** Several types share one Cloud Asset
type: `logging.googleapis.com/LogSink` is four (`google_logging_project_sink`,
`_folder_sink`, `_organization_sink`, `_billing_account_sink`), and
`storage.googleapis.com/Bucket` is the bucket and its IAM policy. Two things
decide, in this order: the content type — an asset carrying the resource is the
resource, one carrying an IAM policy is the grant — and the parent the asset
hangs under, read off its own name (`projects/…`, `folders/…`,
`organizations/…`, `billingAccounts/…`) and compared with the parent each type
is for. A type that names no parent serves any, and is used only where no type
that names one fits. Where several types are still left — the provider has four
for `compute.googleapis.com/Router` and none of them names a parent — the
resource is reported `ambiguous`, naming the types; `--only <type>` picks one.

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
`storage_class`), and what the API nests the import **flattens onto the attribute
the provider names**: a value whose name is an attribute of the resource, or an
object of that name holding a single `enabled` / `value`, is carried up —
`iamConfiguration.uniformBucketLevelAccess.enabled` is `uniform_bucket_level_access`,
`iamConfiguration.publicAccessPrevention` is `public_access_prevention`,
`billing.requesterPays` is `requester_pays`. Two fields claiming one attribute
with different values carry neither and are named. Where the API states a fact in
other terms than the provider, the pair is written out: a lifecycle condition's
`isLive` is the provider's `with_state` (`LIVE` / `ARCHIVED`).

A renamed BLOCK — `lifecycle.rule[]` is Terraform's `lifecycle_rule`, a
reserved-word collision — is a row in the `map:` of its `import-config.yaml`
entry, and **`satz map-types` derives those rows**: for every `import: true` row it
fetches the API's Discovery Document (cached under `presets/.discovery/`), aligns
its schema against the provider schema (exact after snake_case; flattened leaves;
renamed blocks by property overlap; the rest unmatched) and writes
`presets/type-map.yaml`, which the live import applies before the schema filter,
with the hand-maintained rows of `import-config.yaml` winning where both name a
field. Review the rows it marks `renamed`; re-run after a provider bump; an
ambiguous schema name is pinned with `api_schema:` on the row.

What the provider does not speak is **dropped and reported** — a count per type,
the names with `--verbose` — rather than written into HCL that would not plan. An
attribute the provider schema does name, whose value the import could not place,
is a different line: it is printed per resource on every run, with the attribute
and the reason, because an apply of the estate would reset it on the live
resource. `force_destroy` has no counterpart in any API and is read from nothing:
it is a Terraform-only switch, never imported, and an estate that wants it
declares it. A fetch that fails aborts the
import — nothing is written from a partial sweep. A nested value the API does not
return while it holds the provider's default — a subnet's `log_config.filter_expr`,
default `"true"` — is read back into state as empty, so the first plan after the
import shows a one-time in-place update to the default; the first apply writes it and
the subnet's flow logs do not change.

**Under the Hood:**
- state: reads `tofu show -json` (file, stdin, or run now); only the types with `import: true` are taken; read-only/computed fields are dropped against the provider schema.
- live: one Cloud Asset Inventory sweep under the root; needs `cloudasset.assets.searchAllResources`; useful for infrastructure nobody manages with Terraform yet. Only asset types the config maps are seen. Folders are labelled by display name; the built-in `_Default`/`_Required` sinks, service agents' grants, the legacy bucket grants, Google-created service accounts and projects that are no longer ACTIVE are skipped and listed — each row's `skip:` patterns in `import-config.yaml` say what, and a copy of the table without a pattern imports it. The providers' quota project is the first project that enables the Org Policy and Service Usage APIs (organization-scoped reads are billed to it and fail with 403 elsewhere), and the report names it.
- every shape: the document is written in the language's forms — one line per grant edge and per service, a single nested block as a block, an org policy as its bare constraint, the organization referenced as `customer_organization_id` — so the file reads like an estate a person wrote. The `params` block is the day-0 vocabulary `init` writes, bound from what the ADC states (live shape) and what the resources imply: the service account granted organizationAdmin names `svc_iac_account` and `infra_project_name`, that project its folder, versioned bucket and billing account, the members `svc_iac_users_group`, the regional resources `default_region`, the leading name token `customer_shortname`. An inferred value carries `// inferred:` with its rule; a value nothing states is left out and reported; every bound literal is referenced wherever the body repeats it.
- hcl (`satz import ./hcl-dir`): three tiers. A `variable` with a literal `default` and a `locals` entry with a literal value are **promoted to params** — params are Satz's variables, so the imported estate stays re-parameterisable instead of carrying baked-in literals; `var.x` becomes a bare param reference and `"a-${var.x}"` the interpolation `"a-{x}"`. A `variable` with no `default` is named in the header and given no value, so `satz transpile` stops with `unknown param` until it is bound — the same gate the source had. A `resource` block of a schema-known type is **translated** when every value is a literal, a promoted param, or a reference to a managed resource (carried verbatim as `${{…}}`, which emits back byte-identically); folders, projects, services and grants are **placed** by the folder/project they reference, so the tree comes back and `customer_organization_id` is inferred, and a resource that named no project of its own inherits a dropped `provider` block's default when that resolves to one of the imported projects. A `*_iam_member` whose scope is neither project, folder nor organisation — a service account's, a bucket's — becomes the scope-pinned member map (`bucket = …` beside the members, one map per scope), the same rule the live and state shapes write by; a resource naming its project by a literal id that a `google_project` in the input carries is placed under it, as a reference would be. A block whose `count` is `length()` of a promoted list of scalars, and whose every `count.index` indexes THAT list, is **expanded**: one Satz resource per entry, each taking the entry where the source wrote `var.list[count.index]`, labelled after it (`state_europe_west3`) or by its position when the entry makes no identifier. Terraform's idiom for "one of these per entry" IS one resource per entry in Satz. Any other `count` — over a list this import cannot resolve, or with a `count.index` that indexes something else — leaves the block verbatim, because half an expansion would be a guess about what the source meant. Everything else — `module`, `data`, `output`, blocks using `for_each`/`dynamic`/`provider`/`depends_on` or a `count` of another shape, function calls, conditionals, groups, memberships, billing grants, authoritative IAM bindings, unknown types, labels that are not identifiers — is carried verbatim inside `hcl trust "imported from <file>:<line>" { … }` and the report says why, per block. A promoted declaration that a wrapped block still reads is carried verbatim too, so its `var.x` keeps resolving. `terraform`/`provider` blocks are dropped with a note; the emitter owns `providers.tf`. `--wrap-all` wraps everything and promotes nothing. Either way the estate deploys exactly as the source did: `tofu plan` against the source's state shows no changes. A translated block is not verified: a `${…}` reference is opaque to the compliance plane. Also the way in for `gcloud beta resource-config bulk-export --resource-format=terraform` and `tofu plan -generate-config-out` output — run by hand, or by `satz import <scope> --generate-unmapped`, which runs it for the live resources the sweep could not map.

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
- A pack the library **moved** is installed at its new path like any missing file. The copy at the old path, a `.local.satz` fork and a `.diff.satz` delta beside it, and the estate's `use` lines stay as they are: a `use` of the old path is refused naming the new one and the edit — see [docs/workflows.md](docs/workflows.md#when-a-release-moves-a-pack).

### The decisions sheet (`questions --format markdown`)

Every decision the estate rests on, as one page a customer can read: what was asked,
what it is set to, whether that was **chosen for this estate** or taken as offered, and
what changing it later costs. Under each row sits the italic line saying why the
question exists at all — which is the half that makes it readable by someone who did
not write the estate.

```bash
satz questions C0example.satz --format markdown --out decisions.md
```

It opens with the gate verdict, so the sheet answers "may we start?" before it answers
anything else:

> **All 25 questions are answered.** Nothing below is undecided; this estate may be
> bootstrapped.

One table per pack, opened by that pack's own description, in the estate's order.

**Read it at two moments**, because it is two different documents:

1. **After choosing the packs, before the organisation is touched** — the
   "these are your decisions, shall we start?" page. Pair it with `--unanswered` for the
   interview's worklist, with `--format pdf` for the same sheet typeset, and with
   `--format xlsx` when the customer should fill the answers in and send them back:
   ```bash
   satz questions C0example.satz --format markdown --unanswered --out open.md
   satz questions C0example.satz --format xlsx --out decisions.xlsx
   ```
   `xlsx` is a format like any other, so the workbook a customer fills in and returns is
   chosen the same way the sheet is — one invocation, one artefact.
2. **After the rollout, as documentation** — the same command against the applied estate
   is the record of what was decided and why, with the cost of reversing each choice
   already written down. Regenerate it after any estate change and keep it beside the
   compliance report:
   ```bash
   satz questions C0example.satz --format markdown --out decisions.md
   ```
   `--format` and `--out` are both required and the command writes exactly one file
   ([ADR 0021](docs/adr/0021-one-format-one-file-one-artefact.md)); the line naming the
   path goes to stderr, so `--out -` is a clean pipe. `--format pdf` writes the same
   sheet typeset.

Because the estate is the answer record ([ADR 0006](docs/adr/0006-an-answer-is-a-param-the-estate-binds.md)),
the sheet is always derived, never maintained — it cannot drift from what will actually
be applied. See [satz interview](docs/interview.md#the-decisions-sheet).

### Compliance goal view (`require`)

Frameworks are data: a **catalog** (`presets/catalogs/cis-gcp-4.0.yaml`) lists control
IDs with this project's own paraphrases; preset packs declare **claims** inline
(`claim "cis-gcp" "4.0" "2.2" implements { … }`): "including me discharges control §x.y, witnessed by
these resources". `require` is the goal view over both:

```bash
satz require cis-gcp-4.0 C0example.satz --format text --out -
#   ✓ 2.2  Sinks for all log entries    — google_logging_organization_sink.…
#   ◐ 2.3  Retention on the log bucket  — open duty: validate-then-lock
#   ✗ 2.11 Storage IAM change alerts    — unmet. Provides: monitoring/organization-cis-log-alerts-central
```

Every reporting command answers in one vocabulary — `--format text|markdown|pdf|json|xlsx`,
each command accepting the subset it can produce, listing exactly that subset in its
`--help` and **refusing the rest by name**.
`require --format json` gives the same verdicts as data:

```bash
satz require cis-gcp-4.0 C0example.satz --format json --out - | jq '.summary'
#   { "satisfied": 18, "partial": 5, "deviations": 0, "unmet": 14, "broken": 0, "contradicted": 0, … }
```

**stdout carries the answer and nothing else.** The version banner, the line saying
where a report went and every other progress message go to stderr, so a report can be piped into a
parser without filtering, and `satz mcp`, which speaks JSON-RPC over stdout, is not
corrupted by a stray line.

A catalog can also be a **cross-walk** over another one. `iso27001-2022` carries the 93
Annex A controls and, for those a landing zone can evidence, the CIS controls that stand
as that evidence; `require iso27001-2022` folds those verdicts rather than asking packs
to claim a second framework:

```bash
satz require iso27001-2022 C0example.satz --format text --out -
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

A pack can say so itself: the CIS org-policy packs carry a `notice` naming this run
([§6.15](docs/language.md#615-notice--the-command-a-pack-asks-for-once-it-is-on)). satz
shows it when the pack is switched on, the compile warns at the pack's `use` line until
the estate binds the notice's param `true`, and every command that writes to the
organisation refuses while it is open — the notice declares `severity = error`. `satz adopt --execute --import` over every type binds those
params itself when the run finishes with nothing unresolved.

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
- **User-chosen id** (bucket, service account, sinks, metrics, custom roles,
  `project_service`, …): the import id is rendered offline from a template on
  the type's row in `presets/import-config.yaml`
  (`import_id: "projects/{project}/serviceAccounts/{account_id}@…"`), with
  `{placeholders}` filled from the emitted attributes and resolved references.
  Reported as *derived*; its existence is verified by the import itself.
- **IAM grants** (`google_organization_iam_member`, `google_folder_iam_member`,
  `google_project_iam_member`, `google_billing_account_iam_member`,
  `google_service_account_iam_member`, `google_storage_bucket_iam_member`,
  `google_pubsub_topic_iam_member`, `google_pubsub_subscription_iam_member`,
  `google_bigquery_dataset_iam_member`): the id `<parent> <role> <member>` is
  rendered from the template, then checked against the live IAM policy of the
  parent, read once per parent (a dataset's access list for a BigQuery dataset).
  A binding of the role that holds the member → verified import; no such binding
  → *on apply*, and `--execute --import` runs no `tofu import` for it; a parent
  that does not exist (404) → *on apply (parent)*; a policy that cannot be read →
  *FAILED* with the API's answer. Grants are resolved after everything else, so a
  grant whose parent is a resource the estate declares — `service_account_id =
  "${google_service_account.<label>.name}"`, `folder = google_folder.<label>.name`
  — is followed to the id this run resolved for that resource; when that resource
  is itself not live, the grant reads *on apply (parent)* and is created with it.
  A reference satz cannot follow — an attribute that is neither a literal nor the
  resource's live id, a resource the estate does not emit — is **unresolvable**,
  naming both. A grant with a `condition` imports only when a
  live binding carries the same condition title and expression, under the
  provider's id `<parent> <role> <member> <title>`; when the member holds the role
  live only under a different condition, the grant is **AMBIGUOUS** and the live
  conditions are listed.
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
imported says why, and the closing line counts what was activated, imported,
moved, already managed, left to apply and failed. Managed org-policy constraints the
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
satz require cis-gcp-4.0 C0example.satz --config ~/estates/acme --format text --out -
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
prints the corrected command. The estate itself is no argument of these three: they run
in `hcl_dir`, which the config names, so a `.satz` file among the arguments is refused
with the command that works instead of reaching the tool as a positional argument.

`plan` and `apply` add one argument of their own: `-replace=<address>` for each org
policy the state holds with rules while the estate declares it `spec { reset = true }`,
with a note naming it — whatever `reset` the state records beside the rules. An `adopt`
of a policy a new organisation already enforces puts exactly that pair in the state. The provider would update such a policy by sending its rules
together with `reset`, which the API refuses (`400 Cannot set PolicyRules if reset is
true`); the replace deletes the policy and creates it reset. They read the state for
this only when `main.tf` declares a reset policy. Nothing is added to an apply of a
saved plan, to `-destroy` or `-refresh-only`, or for an address the arguments already
replace. `tofu plan` run directly shows the in-place update instead.

They do not transpile first, so the generated diff can be reviewed between `transpile`
and `plan`; `transpile --plan` / `--apply` does both in one command.

A destroy needs two estate edits first: the provider refuses to delete a project
whose `deletion_policy` is `PREVENT` and a folder whose `deletion_protection` is
`true`, which is what both are without the estate saying otherwise, and satz writes
neither attribute by itself. Declare `deletion_policy = "DELETE"` on each project and
`deletion_protection = false` on each folder, apply each edit on its own, then destroy
— [the workflow page](docs/workflows.md#tear-the-estate-down) has the sequence.

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
domain-restricted sharing carry no class: exempting the record of what
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
the estate adopts, S1 or S2. The two are separate so that "may grant a narrow
exemption" need not imply `roles/orgpolicy.policyAdmin`, which is what the
security-admins group holds and which can rewrite any policy outright.

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
repository with its owner and its reason. A binding added out of band is the temporary
kind, and `report-compliance` reports it: it lists every live binding of the estate's
exemption key through Cloud Asset Inventory, subtracts the ones the estate declares, and
prints the rest in a section of their own — value, target, and the claimed controls whose
policy conditions on that value — and under each of those controls. The control's status
stays what its witnesses make it.

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
corroboration (`--prowler findings.json` — the OCSF export of Prowler 5, `prowler gcp --output-formats json-ocsf`; a FAIL on one of a control's *verified* witnesses marks the row **CONTESTED**, a FAIL elsewhere is an unmanaged finding beside it). The report names the Prowler version that wrote the export; an export from an older Prowler, or with no version in `metadata.product`, is refused with the version it carries. An export that is not one JSON document is refused with the line, column and byte offset where it breaks; two scans written into one file — Prowler appends the second after the first's closing `]` — are named as the cause. FAIL findings whose check Prowler maps to no control of the framework are in no row; the report counts them per check in a section after the table and under `prowler_unmapped` in the JSON, and `triage` and `remediation-plan` (`meta.json`, the Provenance sheet) do the same.

An estate that declares the exemption key of `presets/exemptions/exemption-tag.satz` gets
one more section: the live bindings of that key the estate does not declare
([Exemptions](#exemptions-keeping-the-control-on-and-letting-one-resource-out)), each
marked on the claimed controls whose policy conditions on its value, and `exemption_bindings`
in the JSON. A refused read says **NOT CHECKED** with the reason, never "none".

The exit code is 0 whatever the verdicts — the report is the deliverable;
`--fail-on not-enforced,drifted` (any status word; `any` = everything that is
not verified/declared) makes the run fail for CI after the report is written;
`--fail-on undeclared-exemption` fails it on a live binding of the estate's exemption tag
key the estate does not declare, and on a run that could not check them. `any` does not
include it.

The framework is optional: named, the report is that catalog's; left out, it is every
framework the estate is HELD TO — the catalog ids its `compliance_frameworks` param
names — one section per framework in the one file, each section the report that framework
alone produces. `--format json` then answers `{frameworks, reports}`. An estate that
binds no `compliance_frameworks` is refused with the catalogs it could name.

```bash
satz report-compliance C0example.satz --format markdown --out evidence/held-to.md   # every framework this customer answers to
satz report-compliance cis-gcp-4.0 C0example.satz --format markdown --out evidence/cis-4.0.md   # + history
satz report-compliance cis-gcp-4.0 C0example.satz --format pdf --out evidence/cis-4.0.pdf --prowler prowler.json
satz report-compliance cis-gcp-4.0 C0example.satz --format markdown --out evidence/cis-4.0.md --checkov   # + a Checkov column: failed checks on a control's witnesses
satz triage cis-gcp-4.0 C0example.satz --prowler prowler.json --format markdown --out triage.md  # the remediation-plan skeleton: A pack covers it / B Satz declares it / C accepted exception / D bring under management / E manual
```

Each row carries the catalog's own one-line `paraphrase` of the control under
its title and, under the witnesses, the `interpretation` the included claims
give of what their resources prove; open duties print their text beside the id.

Row statuses: **verified** (all witnesses live), `verified* (n of m)` (some witness
types have no live check), **unverified** (no witness could be checked — no ADC,
inventory unavailable), **DRIFTED** (declared but not live),
partial (open/attested duties), unmet, broken claim. Each run appends
`evidence/<framework>-<timestamp>.json` beside the config — the evidence history, where
a second run in the same minute takes the next free name (`…_002.json`) and no record is
replaced — and writes the report (`--format pdf` typeset by satz itself, like `report-organizational-policies`). Without
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

That check needs the estate to compile BEFORE anything is written, so merge-presets
refuses an estate that does not. When what fails is a pack copy binding a param the
estate renamed, `satz get-presets --force` refreshes the pristine copies of the packs the
estate uses without compiling (it lists them first); then merge-presets runs. A
`X.local.satz` fork is the estate's own file and is edited by hand.

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

**A pack line without its gate** is not written by this command. The compile reports an
active `use` of a gated pack that has no `when` as an `ungated-pack` finding, which names
the line to write — see [docs/workflows.md](docs/workflows.md#when-a-pack-line-has-no-gate).

**Last, the prerequisites.** A pack the run adopts can emit a type whose role or API
the estate does not declare yet, and so can a release whose prerequisite table grew. The
run ends with the check and the write `update-prerequisites` makes, and reports each role
and API it wrote; `--report-only` lists them instead, and a gap listed or left unwritten
needs attention. Its compiles do not report that gap as a finding, so at `--validation
error` it is written rather than refused.

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
satz check-presets C0example.satz --format text --out -   # compares against upstream (downloads a pristine copy)
satz check-presets C0example.satz --pristine-dir /path/to/pristine/presets --format text --out drift.txt
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
- Fetches the latest release from the GitHub API and compares versions. When a newer version is available it downloads `satz-installer.sh` and `satz-installer.sh.sha256` from that same release, verifies the SHA-256 digest, and only then runs the installer. A checksum mismatch aborts; a release without the sidecar aborts too, unless you pass `--skip-checksum`. The installer verifies the archive it downloads with `sha256sum`: where that command is missing (macOS before 14) and `shasum` is present, `self-update` puts a `sha256sum` that runs `shasum -a 256` first on the installer's PATH; with neither it refuses, unless `--skip-checksum`. The documented `curl … | sh` install has no such shim, and there the installer prints that it skipped the check. On success, prints the documentation URL and opens it unless `--no-open-readme` is given.

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
- `PATHS…` — files or directories; a directory is walked. They are paths, not estate
  names: `fmt` walks the whole tree — packs and library files as much as estates — and
  rewrites in place, so the file it touches is the one named. A name that exists only
  inside `yaml_dir` is refused, and the refusal names the path that works
  ([ADR 0049](docs/adr/0049-fmt-takes-paths-and-resolves-no-estate-name.md))
- `--check` — report instead of rewrite; the exit code is the answer
- `--stdin` — read one file from stdin, write it formatted to stdout

**Under the Hood:**
- An answer written by `satz interview` replaces the value alone, so the line keeps its
  indentation, its `=` column and its trailing comment; a value that does not finish on
  its line is refused rather than half-rewritten.
- Works on the token stream with its comments and line ends, not on the AST, so nothing
  the parser drops is lost ([ADR 0017](docs/adr/0017-the-formatter-keeps-the-authors-line-breaks.md)).
- Meaning is proven, not assumed: `cargo test` formats every Satz file in the repository
  and checks that the canonical form `check-presets` compares is unchanged and that a
  second pass changes nothing. The smoke matrix runs `satz fmt --check` over the
  repository, so every file here is formatted.
- A reformatted pristine pack is not drift: `merge-presets` compares canonical forms and
  upgrades comment and format churn in place.
- Every Satz file satz writes is in the canonical layout: what `init`, `import`,
  `export-organizational-policies` and a skeleton compose whole is formatted as it is
  written. An in-place edit — an answer from `interview`, an `"import-id"` from
  `adopt --execute`, a role or an API from `update-prerequisites`, a `use` line from `merge-presets`
  — keeps the author's layout, and keeps a formatted file formatted. A pristine pack from
  upstream and the `.local.satz` fork of an author's file are copied byte for byte.
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
  it that name it. Everything `transpile --check` checks is in — a required attribute
  the provider needs, a reference to a resource the estate does not emit, an IaC role
  the service account lacks, a pack the estate's own true answer asks for that is still
  commented out,
  a resource outside the project its type takes, a `deployment_mode` other than `local`
  or `cloud`, a `cloud` one without `svc_iac_account` and `infra_project_name`, the actions and passthrough blocks the compile warns about — each at the
  line it names, as a warning or an error. A compile that stops on `unknown param` carries
  the requirement the pack graph says is off, in the refusal itself, so it is marked at
  the line that refused.
- **Completion.** Inside a resource type, its attributes and nested blocks (from the
  provider schema in `schema_dir`), then `use`, then every resource type; at the top
  level the statements and every type; after `=`, the params in scope with their values,
  and `true`/`false`; inside `question`, `notice` and `action` bodies, their keys.
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

### Configuring an MCP client (`mcp-config`)

`satz mcp` is the server an MCP client starts. `satz mcp-config` prints the block that
client reads to start it, for one estate:

```bash
satz mcp-config C0example.satz                              # Claude Code's .mcp.json
satz mcp-config C0example.satz --client claude-desktop      # Claude Desktop's block
satz mcp-config C0example.satz --allow read,write --write   # write it where the client reads it
```

```json
{
  "mcpServers": {
    "satz": {
      "type": "stdio",
      "command": "/Users/you/.local/bin/satz",
      "args": [
        "mcp",
        "--root",
        "/Users/you/estates/acme",
        "--allow",
        "read"
      ]
    }
  }
}
```

Three things are decided for you, and each is why the command exists:

- **The binary** is the satz that prints the block, by absolute path
  (`std::env::current_exe`). A client starts a process, not a shell, so its `PATH` is
  not the terminal's — a desktop application on macOS has almost none. Run the satz you
  want the client to run: a binary in `target/release` names itself, and so writes a
  configuration that starts that build.
- **The root** is the estate's own directory — the one holding its `config.toml`,
  `yaml/`, `presets/` and `schemas/` — canonicalised, because the client starts the
  server from a working directory of its own. One server serves everything under that
  root, and a call opens an estate inside it.
- **The ceiling** is always written out, whatever `--allow` says and even when it says
  what `satz mcp` already defaults to. A block that leaves `--allow` out grants `read`
  without saying so, and the next person to read it cannot tell that anyone chose.

`--client claude-code` (the default) writes the `.mcp.json` shape under the key `satz`:
one file per project, one satz server in it. `--client claude-desktop` writes the shape
that file takes — command and args, no transport key — under `satz-<estate>`, derived
from the estate as you named it (`C0example.satz` → `satz-C0example`), because Claude
Desktop keeps every server a person has in one file. `--name <KEY>` writes another key.

`--write` puts the block where the client reads it: `.mcp.json` in the estate's
directory, or Claude Desktop's own file
(`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS,
`%APPDATA%\Claude\claude_desktop_config.json` on Windows,
`~/.config/Claude/claude_desktop_config.json` elsewhere); `--file` names another. satz
owns one key of that file and merges into it: every other server is read, kept and
written back. A key of satz's own that is already there with other arguments is refused,
printing what is there — `--force` replaces it. A file that is not JSON, or whose
`mcpServers` is not an object, is refused and left alone; `--force` does not cover it,
because satz cannot merge into what it cannot read. Running the same command twice
writes nothing the second time.

The command reads the estate's name and its directory, compiles nothing, calls no Google
API and touches no credential.

### How a finding is printed

What a compile finds — the errors it refuses on, the warnings and the infos — is printed
in blocks, one per finding, and findings that say the same thing share one. Every command
that prints a finding prints this block: `transpile`, every command that compiles,
`review-pack` and the findings of `packs`. This is `tests/corpus/cis-packs/main.satz` as
`estate.satz`, the CIS baseline and nine of its extensions on, on a terminal 100 columns
wide:

```
packs on while a pack they need is off (1)

warning  pack-requirement  estate.satz:45  presets/cis/CIS-GCP-Foundation-4.0.satz
    `presets/cis/CIS-GCP-Foundation-4.0.satz` needs `presets/estate-map.satz`, which is off
    fix: satz add-pack estate.satz presets/estate-map.satz

notices open — what a pack asks to be run once it is on (10)

warning  notice            estate.satz:45  cis_baseline_adopted
    `presets/cis/CIS-GCP-Foundation-4.0.satz`: Google sets some of these policies on every new
    organisation, and an administrator may have set others; an apply that creates a policy that
    exists stops on 409 POLICY_ALREADY_EXISTS. Run satz adopt once the baseline is on, so every live
    policy is in the state before the apply; with --import it binds this param itself when it has
    run.
    Once the command has run, bind `cis_baseline_adopted = true` in the estate's params; every
    command that writes to the organisation refuses until then.
    fix: satz adopt estate.satz --execute --import

warning  notice            estate.satz:47  cis_block_project_ssh_keys_adopted
warning  notice            estate.satz:48  cis_require_shielded_vm_adopted
warning  notice            estate.satz:49  cis_dns_logging_adopted
warning  notice            estate.satz:50  cis_confidential_computing_adopted
warning  notice            estate.satz:51  cis_cloud_sql_hardening_adopted
warning  notice            estate.satz:52  cis_cmek_required_adopted
warning  notice            estate.satz:53  cis_api_key_services_adopted
warning  notice            estate.satz:54  cis_bucket_retention_adopted
warning  notice            estate.satz:57  cis_cloud_sql_iam_and_deletion_protection_adopted
    Google may already hold a policy this pack declares on the organisation, set by an administrator
    or by default, and an apply that creates a policy that exists stops on 409
    POLICY_ALREADY_EXISTS. Run satz adopt once the pack is on, so every live policy is in the state
    before the apply; with --import it binds this param itself when it has run.
    Once the command has run, bind each param named above `true` in the estate's params; every
    command that writes to the organisation refuses until then.
    fix: satz adopt estate.satz --execute --import

11 warnings
```

- **The first line** is the severity, the `kind`, `file:line` and the subject — the pack, the
  param, the action's name — in columns that line up over the run. Kind and subject are
  what a [silence](#silencing-a-finding-silence) names.
- **The message** stands under it, indented. On a terminal it is wrapped to the terminal's
  width, 110 columns at most. Into a pipe, a file or a CI log nothing is wrapped: each
  paragraph is one line, so a `grep` for a phrase finds it.
- **`fix:`** is the last line, where one command answers the finding: the command as it is
  typed, with the estate in it as a command takes it — the path under `yaml_dir`, so an
  estate in a subdirectory of it reads `pk/e.satz`. It is never wrapped.
- **A group** of findings stands under its title with its count — `(7 of 10, 3 silenced)`
  when a silence left some of it out. Findings of no group come first.
- **Findings of a group that say the same thing are one block:** their first lines as a
  table, one row per finding, then the sentence once and `fix:` once. Above, nine packs
  carry the same text and ask for the same command and are nine rows; the baseline words
  its text differently and is a block of its own. Two findings say the same thing when
  their severity, kind, `fix` and sentence are equal — the sentence without what the row
  carries, the pack's `use` line and the param. A sentence one word apart is another
  block. A finding alone in its block prints its whole message. One thing found at several
  sites — a composition conflict, at each file involved — is a row per site over the one
  message. The counts in the title and in the last line are of findings, and a silenced
  finding is no row. A pipe gets the same blocks, unwrapped.
- **The last line** counts the run by severity, and what was silenced by tier:
  `1 error, 10 warnings; 3 silenced (3 estate) — …`. A refused compile prints its warnings,
  then its errors, then this line, and exits 1.
- There is no colour.

`satz transpile <estate> --check --format json` prints the same run as data on stdout, and
nothing on stderr but the version line: the estate, the `addresses` it emits, the files
`written` (none under `--check`) and every finding — `severity`, `kind`, `group`, `file`,
`line`, `subject`, `message`, `fix`, and `silenced` where a tier silenced it. Every
finding is an object of its own with its whole `message`, the ten notices above included;
only the printed form shares a block. It is the object the MCP tools
`satz_transpile_check` and `satz_transpile` return. A refused compile prints the same object with no address and its errors among the findings — a parse error
too, as one finding of kind `front-end` — and exits 1. `message` is the sentence and `fix`
the command; the command is in `fix` alone.

The language server shows the same finding as a diagnostic: the range is the location, the
diagnostic's `code` is the kind, and its message is the group's title, the sentence and
`fix: <command>` as the last line. A diagnostic is one per location: each of the ten
notices is its own, with its whole message.

### Silencing a finding (`silence`)

A compile reports what it finds as findings — the errors it refuses on, the warnings and
the infos. A finding that has been seen and acted on is silenced by **what it is**, never
by its wording: its `kind`, as `--format json` spells it, and optionally its `subject` —
the pack a pack finding judges, the param a notice is acknowledged by, the action's name,
the `hcl` block's `file:line`. Both are in every finding's JSON, so a selector is read out
of a report and pasted into a rule.

Three tiers hold those rules, each belonging to a different person:

| tier | where | what it may name | who reads it |
|---|---|---|---|
| estate | `[[silence]]` in the estate's `config.toml` | a kind, or one subject of a kind | the CLI, the editor, an agent over MCP |
| machine | `[[silence]]` in `~/.config/satz/satz.toml` | a whole kind only | the CLI, the editor, an agent over MCP |
| run | `--silence <kind>[:<subject>]`, `SATZ_SILENCE` | a kind, or one subject of a kind | this one CLI run |

Every row carries a `reason`; a row without one is a TOML error naming the file and the
line. The run tier is refused for `satz mcp` and `satz lsp`: both serve many estates in
one process, and a silence given once on their command line would hold for all of them.

A silenced finding is **still produced**: it stays in the list, in `--format json` and in
what MCP returns, marked with the tier that silenced it and that tier's reason. Only the
printed output leaves it out: a group's title says how many of it were silenced, and the
run's last line counts them by tier. An **error is never silenced** by any tier, and a
`--silence` that names one refuses the run.

```bash
satz silence list                     # every rule in force, with its reason
satz silence list estate.satz         # …and what each one still silences, or STALE
satz silence add notice --reason "adopt has run; the params are bound"
satz silence add "hcl-passthrough:yaml/estate.satz:192" --reason "reviewed 2026-09-20"
satz silence add action --machine --reason "reviewed once per estate, not per compile"
satz silence remove notice
satz transpile estate.satz --check --silence pack-requirement   # this run only
```

```toml
# the estate's config.toml
[[silence]]
kind = "notice"
subject = "cis_baseline_adopted"
reason = "satz adopt --execute --import has run; every live policy is in the state"
```

A rule nothing answers to any more reads `STALE` in `satz silence list <estate>`, with the
command to remove it — the estate changed, and the rule outlived what it was about.

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
| `provider_version` | `"7.14.1"` | Provider version to use |
| `auto_explode` | `["google_project_service", ".*_iam_member"]` | Resources that use compact explosion |
| `validation_level` | `"warn"` | Validation level for mandatory parameters |
| `[[silence]]` | none | Findings this estate leaves out of its printed output: `kind`, optional `subject`, mandatory `reason`. Managed by `satz silence add`; see [Silencing a finding](#silencing-a-finding-silence). |

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

It also checks each emitted literal against the type the schema gives its attribute,
nested blocks included: a string where the schema types a list, set or map, a list or
map where it types a scalar, and a string that spells no number or bool where one is
wanted. What Terraform converts — a number or a bool into a string — passes, and a
reference or an interpolation is known only at plan and is not judged. Where the
declaring file binds the attribute to a bare param, the finding names the param, which
is what the estate changes.

A resource whose type takes a `project`, declared outside any project and setting none,
goes to the project the provider block names; the compile says so in a warning at its
declaring block, at every validation level. A type that takes a `folder` gets the same
warning outside a folder.

`validation_level` in `config.toml`, or `--validation`, sets what a missing argument or a
refused value does: `warn` (the default) prints one warning per resource with the file
and line that declares it, `error` refuses the compile, `none` skips the check. Any other
value is refused. `tofu plan` refuses such a resource either way.

The same level governs the check that the IaC service account holds the roles the
emitted resource types need — see
[What an estate must declare](#what-an-estate-must-declare-update-prerequisites).

## Satz

Estates are written in **Satz** (`.satz` files) — the language reference is
[docs/language.md](docs/language.md). Params are declarations in one
document-ordered namespace (no anchors), `"{param}"` interpolates (no `!format`), `use "pack.satz" [as key] [when param]`
includes, blocks nest with braces, and resource attribute names are **1:1 the Terraform
provider names** — the registry docs are the docs. A `.satz` estate is parsed directly by
the fragment pipeline (per-file fragments, folded by address, emitted as HCL); packs are
Satz-native, pack params are overridable defaults — a pack that has to ADD to another
pack's list param rather than replace it declares `contributes_<param> = [ … ]` in its own
`params`, and its entries leave with it ([§6.3](docs/language.md#contributes_param--a-packs-entries-in-another-files-list)) —
and **control claims are language
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
interpolates params. Every command reads `.satz`; a `.yaml` estate or pack is the
pre-Satz YAML dialect and is refused by name, with the release that converts it
(see [docs/language.md §12.3](docs/language.md#123-the-pre-satz-yaml-dialect)).

Driving satz from an agent? **[`docs/llms.md`](docs/llms.md)** is the working subset
written for that — the MCP server serves it as `satz://guide`, so an agent gets it without
a repository.

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
| `--silence action` | leave the action findings out of this run's output; `satz silence add action --reason "…"` does it for every run ([Silencing a finding](#silencing-a-finding-silence)) |

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
compiles it itself. The extension is submitted to Zed's registry
(`zed-industries/extensions`, as the submodule `extensions/satz` with
`path = "editors/zed"`); until it is listed there, install it as a dev extension.

To install it as a dev extension: in Zed, run `zed: install dev extension`
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

`cargo-release` (config in `release.toml`) bumps `Cargo.toml`, commits `version bump`, tags `vX.Y.Z` and pushes commit and tag. The tag runs `.github/workflows/release.yml`: build the four targets (macOS on Apple silicon, Linux on x86_64 and ARM64, Windows on x86_64; ADR 0029), each archive carrying `LICENSE`, `NOTICE`, `THIRD-PARTY-LICENSES.md` and the README, create the GitHub release with archives, `sha256.sum`, `satz-installer.sh` and `satz-installer.ps1`, then the `attach-checksum` post-announce job (`dist-workspace.toml`, `.github/workflows/attach-checksum.yml`) uploads `satz-installer.sh.sha256` — the sidecar `self-update` verifies against — and `satz-installer.ps1.sha256`. `prune-releases.yml` afterwards keeps the five newest releases, and the `prune-artifacts` post-announce job (`.github/workflows/prune-artifacts.yml`) deletes the artifacts that run uploaded. Those are only cargo-dist's transport between its own jobs — each target's build goes up, the installer job pulls them all down, `host` pulls them down again to attach them — so once the release exists they are a second copy of the payload, and unlike release assets they count against the Actions storage quota for 90 days. A full multi-platform payload per release kills a 500 MB quota in four releases, and then every workflow in the account fails on artifact upload. `retention-days` cannot be set on those uploads instead: they are steps of `release.yml`, which cargo-dist generates from `dist-workspace.toml`, and the next `dist generate` would drop the line. A release that broke before announcing keeps its artifacts, so it can still be debugged.

`release.yml` is generated, so it is never edited by hand: change `dist-workspace.toml`, run `dist generate`, and commit both. The `release-workflow` job of `.github/workflows/smoke.yml` installs the version `cargo-dist-version` names and runs `dist generate --check` on every pull request and every push to `main`; it fails, printing the diff, when `release.yml` is not what `dist-workspace.toml` generates — a hand-edited line, or a `dist-workspace.toml` change committed without regenerating. A `dist-workspace.toml` change that does not reach the workflow file, such as the `targets` list the `plan` job computes at run time, leaves the check green. The local `dist` must be the version `cargo-dist-version` names — a different one fails the check naming both versions; `dist selfupdate` moves it.

The tag pattern is `**[0-9]+.[0-9]+.[0-9]+*`; the tagged commit must carry that exact `version` in `Cargo.toml`. A release does not run when only `main` was pushed, when the tag predates the bump commit, or when the tag and `Cargo.toml` versions differ.

## Architecture

`satz` compiles Satz estates into OpenTofu/Terraform HCL.

### Core Components

#### 1. Fragment pipeline (`crates/satz-core`, `src/emitter.rs`)
`satz.rs` parses each `.satz` file, `pipeline.rs` resolves params
and `use`s into per-file fragments, `algebra.rs` folds them by Terraform address (⊕), and
the emitter renders the folded IR as `main.tf`, `providers.tf`, `variables.tf`,
`terraform.tfvars` and `imports.tf`.
- **Context Awareness**: a nested resource inherits its parent's identifier (`project`, `folder_id`, `org_id`) from the enclosing block. A project that writes one says its own parent — a reference to a folder the estate declares, or an id — and an empty value says nothing, so the enclosing block decides.
- **Intrinsic scopes**: groups, org grants and billing grants hoist to their real scope wherever they are written.

#### 2. Schema Registry (`src/schema.rs`)
Manages Terraform provider schemas (loaded as JSON).
- **Typing**: every resource key and block key is checked against the schema at parse time — an unknown key is an error naming the file, the line and the key, not a guess. The keys satz reads itself are the exception, and they are a closed list: `"import-id"`, `lifecycle`, `provider`, a project's `project_service` and `org`, a group's `member` / `manager` / `owner` / `email`.

#### 3. Template Generator (`src/template.rs`)
Writes the day-0 estate for a new customer.
- **Declarative Bootstrap**: Generates the Satz estate representing the Day 0 infrastructure (Project, Services, Bucket, SA) under the labels `bootstrap` imports by name.

#### 4. The Satz printer (`crates/satz-core/src/migrate.rs`)
A `serde_yaml` document in, Satz text out. Every import writes through it: the state and
live shapes hand it the discovered configuration, the HCL importer the blocks it
translated, `export-organizational-policies` the pack it snapshots. A caller builds param
references and interpolations with `param_ref`, `interpolation` and `interpolated`, so the
printer owns the rendering; `normalize_type_keys` then gives a container key written
without the provider prefix its full Terraform type name, against the schemas.

#### 5. Discovery Engine
`satz import organizations/<n>` (and the `folders/`, `projects/`, `state.json` shapes) write a Satz estate from what exists.
- **Asset Ingestion**: reads the resources under the root in one Cloud Asset Inventory sweep.
- **Configurable Filtering**: Uses `import-config.yaml` to include/exclude resources and attribute fields.
- **Schema Validation**: Validates discovered data against Terraform schemas and drops read-only and computed fields, so the HCL plans.
- **IAM mapping**: maps IAM policies to member resources (e.g. `google_storage_bucket_iam_member`) and generates their keys.

#### 6. Organization Policy Engine (`src/org_policy.rs`)
Aligns curated Org Policy sets (e.g. `presets/cis/CIS-GCP-Foundation-4.0.satz`) with the live organization via the GCP Org Policy API v2.
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
| `import --into <estate>`, `import --as <estate>` | the estate's service account | the sweep reads a customer's organization, and `roles/cloudasset.viewer` on it is that account's — `--generate-unmapped` writes the account into the provider block its `tofu` child reads with |
| `import` given no estate | the human's ADC | the output is a new file; there is no estate to be — including the `tofu plan -generate-config-out` child of `--generate-unmapped`, which inherits it and impersonates nobody. The scope must be readable by the credentials themselves; `--as <estate>` is how it is read by the account that holds the role |
| `bootstrap`, `init` | the human's ADC | day 0 — the service account does not exist yet |
| `whoami` | the human's ADC | the question *is* who the human is |
| `whoami <estate>` | the estate's service account in cloud mode; the human's ADC in local mode | a different question — who that estate acts as — so a different answer |
| `map-types` | no credential at all | Discovery documents are public |
| `mcp` | per tool call, from the estate the call works on — the one it names, else the open one | one server, a fleet: `satz_open` moves to the next estate, and the identity follows it |
| `plan`, `apply` | the estate's service account, read from the emitted provider block | the [API preflight](docs/workflows.md#the-api-preflight) runs as what `tofu` is about to act as; the tool then resolves its own credential |
| `transpile --plan` / `--apply` | the same | the flags run `plan`/`apply`, preflight included; a transpile without them calls nothing |
| `hcl-init` | `tofu`'s own resolution | satz passes it no token; the provider block impersonates |

`--no-impersonate` pins the process to the plain ADC and outranks every estate.

An estate whose params do not parse, whose `deployment_mode` is neither `local` nor
`cloud`, or whose `deployment_mode = "cloud"` has no value for `svc_iac_account` or
`infra_project_name`, names no identity: `whoami <estate>`, `migrate` and every command in the table that
runs as the estate refuse it, naming the estate and the reason, and run nothing as the login
in its place. Under `satz mcp`, `satz_open` refuses to open it and a live tool refuses a call
that names it.

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
is what every read and write runs as. The `runs as:` line names the identity the calls
use and how it relates to the credential, in one of three forms:

```text
runs as:     you@example.com — no estate given; name one to see what it runs as
runs as:     you@example.com — local mode; `satz migrate e.satz --mode cloud` makes every run impersonate svc-iac-001@acme-infra-001.iam.gserviceaccount.com
runs as:     svc-iac-001@acme-infra-001.iam.gserviceaccount.com — impersonated by you@example.com, checked: allowed
```

A local-mode estate — every estate from `bootstrap` until `satz migrate --mode cloud`,
because its first apply is what creates the service account — runs as the credential
itself. Under `--no-impersonate` a cloud-mode estate does too, and the line says so. An
estate that declares no `svc_iac_account` and `infra_project_name` pair impersonates
nothing, and the line says that instead of naming an account — with `satz migrate
<estate> --mode cloud` named as what it is there: refused until both params are bound. `--offline` reads the
mode and the account off the estate, so it answers all three without a token, and
without an ADC file once an estate is given.

`whoami <estate>` prints both halves, and — online
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
checked by asking the IaC service account first, as itself, whether it may list the
groups of the estate's `customer_id` directory; your login is asked the same way only when
the service account is refused. It counts as tested when the service account holds it, and
is named as not tested with the reason and the next step otherwise: your login holds it and
`migrate --mode cloud` assigns it, neither holds it and a super admin assigns it in the
admin console, the estate sets no `customer_id`, or a test gave no answer (the Cloud
Identity API off on the quota project, a quota project the caller may not use).

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
types need, as [What an estate must declare](#what-an-estate-must-declare-update-prerequisites)
describes. `satz update-prerequisites` adds the roles and the APIs a new pack brings.

## License

satz is licensed under the Apache License, Version 2.0 — see
[LICENSE](https://github.com/tjirsch/satz/blob/main/LICENSE). Redistribution carries
[NOTICE](https://github.com/tjirsch/satz/blob/main/NOTICE) with it: it names the
third-party material in this repository — the provider schema fixture, the release
workflow cargo-dist generates, and the control identifiers the catalogs carry.
Every release archive carries both, and
[THIRD-PARTY-LICENSES.md](https://github.com/tjirsch/satz/blob/main/THIRD-PARTY-LICENSES.md)
beside them: the licence text of every crate compiled into the binary, generated
from `Cargo.lock` by `cargo about` (`scripts/update-third-party-licenses.sh`, and
the `checks` job fails on a stale file or a licence outside `about.toml`'s
allow-list).
