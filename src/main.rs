#![cfg_attr(test, allow(clippy::disallowed_methods))]
mod config;
mod fsx;
mod schema;
mod emit_shared;
mod emitter;
mod manifest;
mod state_migration;
mod discovery;
mod vocabulary;
mod delta;
mod align;
mod scan;
mod actions;
mod template;
mod adopt;
mod bootstrap;
mod preflight;
mod day_zero;
mod init_params;
mod prerequisites;
mod review_pack;
mod gcp;
mod org_policy;
mod cloud_identity;
mod compliance;
mod questions;
mod interview;
mod findings;
mod silence;
mod lsp;
mod mcp;
mod dossier;
mod presets;
mod doc_packs;
mod pack_graph;
mod packs;
mod notices;
mod org_write;
mod github;
mod policy_tree;
mod prowler;
mod out;
mod pdf;

use clap::{Parser, Subcommand, CommandFactory};
// the one output vocabulary: what a caller may ask for, and where it goes
pub(crate) use out::{OutFormat, pdf_from_markdown, write_report};
// the MCP output schema of `IacRolesReport`; schemars reaches the crate through rmcp
use rmcp::schemars;
use clap_complete::Shell as CompletionShell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use crate::schema::ResourceRegistry;
use crate::config::{Config, ImportConfig};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolConfig {
    #[serde(default = "default_yaml_dir")]
    pub yaml_dir: String,
    #[serde(default = "default_hcl_dir")]
    pub hcl_dir: String,
    #[serde(default = "default_include_dirs")]
    pub include_dirs: Vec<String>,
    #[serde(default = "default_schema_dir")]
    pub schema_dir: String,
    /// Library of copyable presets (downloaded by `get-presets`). Lives beside
    /// config.toml by default — yaml_dir is reserved for files that are actually
    /// used and adapted, presets_dir holds everything available for copying.
    #[serde(default = "default_presets_dir")]
    pub presets_dir: String,
    #[serde(default = "default_tf_tool")]
    pub tf_tool: String,
    #[serde(default = "default_google_providers")]
    google_providers: Vec<String>,
    #[serde(default)]
    aws_providers: Vec<String>,
    #[serde(default)]
    azure_providers: Vec<String>,
    #[serde(default)]
    alibaba_providers: Vec<String>,
    #[serde(default = "default_version")]
    pub provider_version: String,
    #[serde(default = "default_auto_explode")]
    pub auto_explode: Vec<String>,
    #[serde(default = "default_validation_level")]
    pub validation_level: String,
    #[serde(default)]
    pub import_config: Option<String>,
    /// What this estate has said "seen, move on" to: `[[silence]]` tables, each naming
    /// a finding's `kind`, optionally its `subject`, and why. Committed with the
    /// estate, so a review sees who silenced what.
    ///
    /// Last in the struct because it is the one array of tables: TOML puts tables after
    /// the scalars, and `save` serializes in this order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) silence: Vec<crate::silence::Rule>,
    /// The directory the config was read from, set by `resolved_config` — never read
    /// from the file and never written back. It is what makes a finding's `file`
    /// estate-relative, so the CLI, the editor and an agent name the same path.
    #[serde(skip)]
    pub(crate) dir: Option<PathBuf>,
}

impl ToolConfig {
    pub fn parsed_providers(&self) -> Vec<(String, String)> {
        let mut providers = Vec::new();
        // default version fallback
        let def_ver = &self.provider_version;
        
        for p in &self.google_providers { providers.push(ToolConfig::parse_provider_string_with_default(p, def_ver)); }
        for p in &self.aws_providers { providers.push(ToolConfig::parse_provider_string_with_default(p, def_ver)); }
        for p in &self.azure_providers { providers.push(ToolConfig::parse_provider_string_with_default(p, def_ver)); }
        for p in &self.alibaba_providers { providers.push(ToolConfig::parse_provider_string_with_default(p, def_ver)); }
        providers
    }

    pub fn parse_provider_string(p: &str) -> (String, Option<String>) {
        if p.contains('|') {
            let parts: Vec<&str> = p.split('|').collect();
            (parts[0].trim().to_string(), Some(parts[1].trim().to_string()))
        } else {
            (p.trim().to_string(), None)
        }
    }

    pub fn parse_provider_string_with_default(p: &str, default_version: &str) -> (String, String) {
        let (name, ver) = Self::parse_provider_string(p);
        (name, ver.unwrap_or_else(|| default_version.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let toml_str = toml::to_string_pretty(self)?;
        fsx::write(path, toml_str)?;
        Ok(())
    }
}

fn default_yaml_dir() -> String { "yaml".to_string() }
fn default_hcl_dir() -> String { "hcl".to_string() }
/// Includes are searched relative to the including file first, then these directories
/// (resolved from config.toml's own directory). `"."` must be present so an `!include`
/// of a file sitting next to config.toml resolves; `init` has always written it, and a
/// hand-written config.toml that omits the key used to silently lose it.
fn default_include_dirs() -> Vec<String> { vec![".".to_string(), "yaml".to_string()] }
fn default_schema_dir() -> String { "schemas".to_string() }
fn default_presets_dir() -> String { "presets".to_string() }
fn default_tf_tool() -> String { "tofu".to_string() }
fn default_google_providers() -> Vec<String> { vec!["google".to_string(), "google-beta".to_string()] }
fn default_version() -> String { "7.14.1".to_string() }
fn default_auto_explode() -> Vec<String> {
    vec![
        "google_project_service".to_string(),
        ".*_iam_member".to_string(),
    ]
}
fn default_validation_level() -> String { "warn".to_string() }


#[derive(Parser)]
#[command(author, version, about, long_about = None, max_term_width = 110)]
pub(crate) struct Cli {
    /// Project config.toml, or the estate directory containing it
    ///
    /// Every path in the config resolves against the config's own directory,
    /// so any command can be run from anywhere.
    #[arg(long, global = true, help_heading = "Global options")]
    config: Option<PathBuf>,

    /// Validation level: warn (default), error, or none
    #[arg(long, global = true, help_heading = "Global options")]
    validation: Option<String>,

    /// Open the documentation site in the browser at this command's section
    /// (`satz transpile --html-help`); alone, the site's front page
    #[arg(long, global = true, help_heading = "Global options")]
    html_help: bool,

    /// Enable verbose output
    #[arg(long, global = true, help_heading = "Global options")]
    verbose: bool,

    /// Live commands: call the APIs as the plain ADC identity instead of
    /// impersonating the estate's IaC service account
    #[arg(long, global = true, help_heading = "Global options")]
    no_impersonate: bool,

    /// `plan` and `apply`: do not ask Service Usage which of the APIs the estate
    /// declares are off, and enable none — for a run that must reach the tool
    /// without satz calling Google
    #[arg(long, global = true, help_heading = "Global options")]
    no_api_preflight: bool,

    /// Never execute a declared `action`, whatever `run-actions` was asked to do
    #[arg(long, global = true, help_heading = "Global options")]
    no_actions: bool,

    /// Consider only the estate's own actions; ignore any a `use`d pack declares
    #[arg(long, global = true, help_heading = "Global options")]
    no_pack_actions: bool,

    /// Leave a finding out of this run's output by what it is: `<kind>` or
    /// `<kind>:<subject>`, repeatable
    ///
    /// The run tier of `satz silence`, for a CI pipeline; `SATZ_SILENCE` takes the
    /// same selectors, comma-separated, where the command line cannot be changed. The
    /// finding is still produced, still counted and still in `--format json`. An error
    /// is never silenced, and a `--silence` that names one refuses the run.
    #[arg(long, global = true, help_heading = "Global options", value_name = "KIND[:SUBJECT]")]
    silence: Vec<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// The `satz --help` groups. The ONE place the command taxonomy lives: the root
/// help renders them in this order, and `satz --verbose` walks the per-command
/// help in it too. A command the CLI has and this table does not — or the other
/// way round — fails `command_groups_cover_the_cli`, so the help cannot drift
/// away from the binary the way a hand-kept list would.
const COMMAND_GROUPS: &[(&str, &[&str])] = &[
    ("Estate", &["init", "bootstrap", "transpile", "import", "adopt", "update-prerequisites", "packs", "add-pack", "remove-pack"]),
    ("HCL", &["hcl-init", "plan", "apply", "migrate", "scan-plan", "generate-migration", "run-actions"]),
    ("Presets", &["get-presets", "merge-presets", "check-presets", "doc-packs", "pack-graph", "review-pack"]),
    (
        "Policies",
        &[
            "export-organizational-policies",
            "diff-organizational-policies",
            "report-organizational-policies",
            "adopt-org-policies",
        ],
    ),
    ("Compliance and audit", &["require", "questions", "interview", "report-compliance", "scan", "prowler", "triage", "remediation-plan"]),
    ("Tool", &["update-schema", "map-types", "fmt", "lsp", "silence", "self-update", "completion", "open-readme", "whoami", "help", "mcp"]),
];

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run the estate's declared `action`s — the deployment steps that have no provider resource
    ///
    /// Prints what it would run and stops. `--check` runs each action's own
    /// dry-run form; `--execute` runs the form that writes. Whether an action's
    /// check form is side-effect-free is the action's contract, not satz's.
    RunActions {
        /// Estate file, .satz (inside yaml_dir if relative)
        input: String,
        /// Run each action's dry-run form (`args` only)
        #[arg(long, conflicts_with = "execute")]
        check: bool,
        /// Run the form that writes (`args` + `execute_args`)
        #[arg(long)]
        execute: bool,
        /// Run only these actions, by name
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
        /// Run only actions of this phase: before-apply or after-apply
        #[arg(long)]
        phase: Option<String>,
    },
    /// Compile an estate to HCL (a `.yaml` estate is migrated with `satz import`, never transpiled)
    Transpile {
        /// Estate file, .satz (inside yaml_dir if relative)
        ///
        /// Not the tool config — that is --config
        input: String,
        /// Name of the output file (inside hcl_dir if relative)
        #[arg(long)]
        output: Option<String>,
        /// Schema directory containing provider JSON files
        #[arg(long)]
        schema_dir: Option<PathBuf>,
        /// Print the resolved variable table (terraform.tfvars) to stdout after transpilation
        #[arg(long)]
        print_variables: bool,
        /// After transpiling, run `<tf_tool> plan` in hcl_dir (initialising it first if needed)
        #[arg(long)]
        plan: bool,
        /// After transpiling, run `<tf_tool> apply` in hcl_dir (initialising it first if needed)
        #[arg(long)]
        apply: bool,
        /// After transpiling, run Checkov over hcl_dir and point each finding at
        /// the Satz block that declared the resource (failed checks exit 1)
        #[arg(long)]
        scan: bool,
        /// Compile only: parse, fold and emit in memory, write nothing —
        /// the estate either transpiles or the error says why
        #[arg(long)]
        check: bool,
        /// `json` prints the compile as data on stdout — the estate, the addresses it
        /// emits, the files written and every finding, silenced ones included — and
        /// nothing on stderr but the version line. A refused compile prints the same
        /// object with its errors in `findings`, and exits 1
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]), default_value = "text")]
        format: OutFormat,
    },
    /// Scan Tofu plan JSON for resource renames
    ScanPlan {
        /// Path to plan JSON file
        plan_json: PathBuf,
        /// Output mapping YAML path
        #[arg(long, default_value = "mapping.yaml")]
        output: PathBuf,
    },
    /// Generate a shell script with state mv commands from mapping
    GenerateMigration {
        /// Path to mapping YAML file
        #[arg(default_value = "mapping.yaml")]
        mapping: PathBuf,
        /// Output shell script path
        #[arg(long, default_value = "migrate.sh")]
        output: PathBuf,
    },
    /// Initialize project structure and config
    Init {
        /// Default sets to include (e.g., google)
        #[arg(long, value_delimiter = ',')]
        defaults: Option<Vec<String>>,
        /// Explicit providers to include
        #[arg(long, value_delimiter = ',')]
        providers: Option<Vec<String>>,
        #[arg(long)]
        tf_tool: Option<String>,
        /// Customer ID (workspace organization ID) to generate template for a new organization
        #[arg(long)]
        customer_id: Option<String>,
        /// Short name for the organization/customer
        #[arg(long)]
        customer_shortname: Option<String>,
        /// Billing account ID
        #[arg(long)]
        billing_account_infra: Option<String>,
        /// GCP Region
        #[arg(long)]
        default_region: Option<String>,
        /// Numeric Organization ID
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Primary Domain
        #[arg(long)]
        customer_domain: Option<String>,
        /// Infrastructure Project ID
        #[arg(long)]
        infra_project_name: Option<String>,
        /// Infrastructure Bucket Name
        #[arg(long)]
        infra_bucket_name: Option<String>,
        /// Initial IaC Admin User (default: first.admin@<domain>)
        #[arg(long)]
        iac_user: Option<String>,
        /// Accepted and ignored: deriving from the Application Default Credentials is what init does by default
        #[arg(long, hide = true)]
        from_live: bool,
        /// Overwrite an existing estate instead of merging the params named here into it
        #[arg(long)]
        force: bool,
        /// Ask for what is still unbound: hand the estate to `satz interview` when a
        /// day-0 param was neither stated nor derivable
        #[arg(long)]
        interview: bool,
    },
    /// Bootstrap day-0 infrastructure (folder, project, billing link, core APIs, state bucket) after a permission pre-flight
    Bootstrap {
        /// Estate file, e.g. C0example.satz (inside yaml_dir if relative)
        ///
        /// Not the tool config — that is --config
        estate: PathBuf,
        /// Read-only: print the plan, verify the ADC identity and run the
        /// permission pre-flight; create nothing
        #[arg(long)]
        dry_run: bool,
        /// Materialize a not-yet-existing organization: create the infra
        /// project WITHOUT a parent (Google's documented auto-provisioning
        /// trigger for a directory user), wait for the organization, move the
        /// project under it and write the id back into the estate
        #[arg(long)]
        greenfield: bool,
        /// Never widen the caller's own IAM: report the roles an administrator
        /// must grant and stop, instead of self-granting them at the scope root
        #[arg(long)]
        no_default_grants: bool,
    },
    /// Export the current live Organization Policies to a re-importable YAML preset
    #[command(visible_alias = "export-org-policies")]
    ExportOrganizationalPolicies {
        /// Estate file providing the parameter table, incl. customer-organization-id (inside yaml_dir if relative)
        ///
        /// Not the tool config — that is --config
        estate: PathBuf,
        /// Organization id override (numeric or organizations/<id>); else read from config
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Output path inside yaml_dir, always .satz (default: <Cxxxx>-orgpolicies.satz)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Diff a desired Org Policy preset against the live organization state
    #[command(visible_alias = "diff-org-policies")]
    DiffOrganizationalPolicies {
        /// Estate file providing the parameter table (inside yaml_dir if relative)
        ///
        /// Not the tool config — that is --config
        estate: PathBuf,
        /// Organization id override; else read from config
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Report format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Audit the whole resource hierarchy (org, folders, projects) via Cloud Asset Inventory, classifying node-level overrides against the baseline
        ///
        /// Needs roles/cloudasset.viewer on the organization
        #[arg(short = 'r', long)]
        recursive: bool,
    },
    /// Produce a human-readable report of Organization Policies with explanatory text
    #[command(visible_alias = "report-org-policies")]
    ReportOrganizationalPolicies {
        /// Estate file providing the parameter table, incl. customer-organization-id (inside yaml_dir if relative)
        ///
        /// Not the tool config — that is --config
        estate: PathBuf,
        /// Organization id override; else read from config
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Which policies to include
        #[arg(long, default_value = "active", value_parser = ["active", "inactive", "full"])]
        scope: String,
        /// Report format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Inventory declared policies across the whole resource hierarchy (org, folders, projects) via Cloud Asset Inventory
        ///
        /// --scope's "available but not set" section stays org-level. Needs
        /// roles/cloudasset.viewer
        #[arg(short = 'r', long)]
        recursive: bool,
    },
    /// Fetch schemas and update config
    UpdateSchema {
        #[arg(long, value_delimiter = ',')]
        providers: Option<Vec<String>>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        tf_tool: Option<String>,
    },
    /// Create a Satz estate from what exists
    ///
    /// The source decides the shape: a state file (`state.json`, `*.tfstate`,
    /// `-` for `tofu show -json` on stdin), a live scope
    /// (`organizations/<n>`, `folders/<n>`, `projects/<id>`), or a legacy
    /// YAML-dialect file. With no source the live root comes from the import
    /// config. Every import ends with `satz transpile` and `tofu plan` — the
    /// plan is the check
    Import {
        /// What to import from (see above); omit to use the import config's `root`
        source: Option<String>,
        /// Force the shape when the source does not tell: state | org | yaml | hcl
        #[arg(long)]
        from: Option<String>,
        /// Resource types to import, comma-separated, `*` wildcards allowed
        /// (overrides `only` in the import config)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Take every type the source can deliver, not only the rows marked
        /// `import: true` (live: every type with a Cloud Asset Inventory name)
        #[arg(long)]
        all: bool,
        /// Resource types to leave out, comma-separated, `*` wildcards allowed
        /// (overrides `exclude` in the import config)
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
        /// Output file inside yaml_dir (state, live and hcl shapes; default discovered.satz,
        /// imported-hcl.satz for hcl — the yaml shape writes beside its source)
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Import configuration (default: <presets_dir>/import-config.yaml)
        #[arg(long)]
        import_config: Option<PathBuf>,
        /// yaml shape: estate used to compile a converted pack in context
        #[arg(long)]
        gate: Option<PathBuf>,
        /// yaml shape: declared kind of the converted file
        #[arg(long, default_value = "pack", value_parser = ["pack", "estate"])]
        kind: String,
        /// yaml shape: write the conversion as a `<stem>.local.satz` fork
        #[arg(long)]
        fork: bool,
        /// live shape: import only what this estate does not already declare
        /// (matched by live id), as packs the estate `use`s
        #[arg(long)]
        into: Option<PathBuf>,
        /// hcl shape: carry every block verbatim inside `hcl trust` (the
        /// zero-risk form; the estate deploys exactly as the source did)
        #[arg(long)]
        wrap_all: bool,
        /// state/live shapes: a grant one principal holds on two folders or
        /// two projects — `error` refuses the import naming them, `counter`
        /// writes the second and later as labelled resources with a running
        /// number (the map form emits one address per member and role)
        #[arg(long, value_enum, default_value_t)]
        on_collision: crate::discovery::OnCollision,
        /// state/live shapes: the customer's short name, which no platform
        /// fact carries — wins over the inference from the names found
        #[arg(long)]
        customer_shortname: Option<String>,
    },

    /// Migrate state and configuration between local and cloud modes
    Migrate {
        /// Estate file (inside yaml_dir if relative): binds `deployment_mode`
        /// in its params
        input: String,
        /// Target mode; without it, the other of the two
        #[arg(long, value_parser = ["local", "cloud"])]
        mode: Option<String>,
    },
    /// Check for and install new releases from GitHub
    SelfUpdate {
        /// Do not open the documentation site after installing
        #[arg(long)]
        no_open_readme: bool,
        /// Only check if an update is available; do not install
        #[arg(long)]
        check_only: bool,
        /// Skip SHA-256 checksum verification (use only if the release predates sidecar support)
        #[arg(long)]
        skip_checksum: bool,
    },
    /// Fetch the upstream preset library into presets_dir: installs what is missing and refreshes what the estate does not use
    ///
    /// Packs the estate DOES use are refused (they deploy — use
    /// `merge-presets`), unless --force.
    GetPresets {
        /// Overwrite presets the estate uses as well
        ///
        /// Lists each one first.
        #[arg(long)]
        force: bool,
        /// Take the library from this directory instead of downloading it
        #[arg(long)]
        pristine_dir: Option<PathBuf>,
    },
    /// Reconcile the library with the estate: refresh a pack that is behind upstream,
    /// turn one edited locally into an `X.local.satz` fork with its delta in
    /// `X.diff.satz` and repoint the estate at it, and write the commented `use` line
    /// for every pack the library has and the estate lacks.
    MergePresets {
        /// Compare against this directory instead of downloading upstream
        #[arg(long)]
        pristine_dir: Option<PathBuf>,
        /// Estate file providing the used-preset context (default: the single
        /// `estate` .satz in yaml_dir)
        #[arg(long)]
        estate: Option<PathBuf>,
        /// Print what would happen without writing anything
        #[arg(long)]
        report_only: bool,
        /// Adopt upstream IN PLACE for these packs instead of forking them — the deliberate upgrade
        ///
        /// Pass a pack stem (`CIS-GCP-Foundation-4.0`), repeatable; or `all`
        /// for every pack that is merely BEHIND. `all` never touches a pack
        /// that differs at the SAME version — that is an edit, and it must be
        /// named explicitly.
        #[arg(long)]
        adopt: Vec<String>,
    },
    /// Goal view against a compliance framework: which catalog controls are
    /// satisfied / partial / unmet by the declared estate, with witnesses and,
    /// for unmet controls, the packs that would provide them.
    Require {
        /// Catalog id, e.g. cis-gcp-4.0 (a file in <presets_dir>/catalogs/)
        framework: String,
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Evidence report: the goal view joined with LIVE verification (Cloud Asset
    /// Inventory), manual-duty attestations and optional Prowler corroboration —
    /// written as an auditor-shaped report plus an append-only evidence history.
    ReportCompliance {
        /// Catalog id, e.g. cis-gcp-4.0
        framework: String,
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Prowler 5 OCSF export to ingest as corroboration (`prowler gcp --output-formats json-ocsf`)
        #[arg(long)]
        prowler: Option<PathBuf>,
        /// Run Checkov over hcl_dir (transpile first) and add a column: failed
        /// checks on a control's witnesses are evidence against it
        #[arg(long)]
        checkov: bool,
        /// Skip live verification (declared-estate report only)
        #[arg(long)]
        no_live: bool,
        /// Exit non-zero when a row's status contains one of these (comma list, e.g. `not-enforced,drifted,unmet`; `any` = anything that is not verified/declared)
        ///
        /// The report is written either way.
        #[arg(long, value_delimiter = ',')]
        fail_on: Vec<String>,
    },
    /// Compare local presets against the pristine upstream library and report drift:
    /// which included presets were edited locally, and the params block to add to
    /// the estate so the pristine preset can be restored.
    CheckPresets {
        /// Estate file (inside yaml_dir if relative) whose `use` graph decides which
        /// presets count as "in use"
        input: String,
        /// Compare against this directory instead of downloading the upstream presets
        #[arg(long)]
        pristine_dir: Option<PathBuf>,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Adopt what already exists: resolve the live ids of the resources this estate declares (folders by name, groups by email, org policies by constraint, everything else by its rule in import-config.yaml) and bring them under management
    ///
    /// A dry run unless --execute
    Adopt {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Resource types to adopt, comma-separated (default: every type)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Apply: write verified "import-id"s into the estate (default), or
        /// with --import run `<tf_tool> import` now
        #[arg(long)]
        execute: bool,
        /// With --execute: import into state now instead of writing "import-id"s
        #[arg(long)]
        import: bool,
        /// Activate GCP managed org-policy constraints the organisation has
        /// never had, so they can be imported (mutates the org)
        #[arg(long)]
        activate: bool,
    },
    /// What the estate's resource types oblige it to declare and does not — the roles its IaC service account is missing, and the APIs its infrastructure project does not enable — written into the estate file
    ///
    /// Writing is the point: the declarations have to be there either way. The
    /// estate must compile and come out complete afterwards, or the file is
    /// restored. `--report-only` lists the gap instead, for an engagement where
    /// satz may not grant those roles itself; it exits non-zero while anything is
    /// missing. Without an estate: the table itself (--format json for
    /// scripts/check_prerequisites.py)
    #[command(visible_alias = "prerequisites")]
    UpdatePrerequisites {
        /// Estate file (.satz, inside yaml_dir if relative); omit to print the table
        input: Option<String>,
        /// List what is missing and write nothing; exits non-zero while anything is
        #[arg(long)]
        report_only: bool,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]), default_value = "text")]
        format: OutFormat,
    },
    /// Derive the API→Terraform field map per resource type from the API's Discovery Document and the provider schema, into <presets_dir>/type-map.yaml — what the live import applies so imported resources plan clean
    ///
    /// Review the rows it marks renamed or unmatched; re-run after a provider
    /// bump
    MapTypes {
        /// Resource types to map, comma-separated (default: every row with import: true)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Import configuration (default: <presets_dir>/import-config.yaml)
        #[arg(long)]
        import_config: Option<PathBuf>,
    },
    /// Alias of `adopt --only google_org_policy_policy --activate --execute --import`
    AdoptOrgPolicies {
        /// Estate file (inside yaml_dir if relative)
        input: String,
        /// Show what would be activated and imported, change nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Sort every Prowler FAIL into the bucket that says who fixes it and how
    /// (a pack covers it / Satz declares it / accepted exception / bring
    /// under management / manual) — the skeleton of the remediation plan
    Triage {
        /// Catalog id, e.g. cis-gcp-4.0
        framework: String,
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Prowler 5 OCSF export (`prowler gcp --output-formats json-ocsf`)
        #[arg(long)]
        prowler: PathBuf,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Add the estate delta the findings imply — `use` lines to add, resources
        /// to bring under management, and what has nothing to edit — to the report.
        /// Proposed only: satz never writes the estate or the cloud from a finding.
        /// Markdown only: the delta is prose, and a JSON caller parses rows
        #[arg(long)]
        fix: bool,
    },
    /// Build the remediation dossier — the findings workbook minus the prose
    ///
    /// Every Prowler FAIL/MANUAL (and Checkov finding) triaged against the
    /// estate's claims, joined per resource, counted, and written under the
    /// estate's evidence/ directory as JSON, CSV and XLSX. The mechanical
    /// columns are filled; the `[Authored]` columns and the Review column are the
    /// consultant's — written back with `--merge`, or over MCP. Offline and
    /// deterministic: the dossier hash names the run
    RemediationPlan {
        /// Catalog id, e.g. cis-gcp-4.0
        framework: String,
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Prowler 5 OCSF export (`prowler gcp --output-formats json-ocsf`)
        #[arg(long)]
        prowler: PathBuf,
        /// Also run Checkov over hcl_dir and join its findings
        #[arg(long)]
        checkov: bool,
        /// Output directory — several files (default: a new folder <config dir>/evidence/plan/<framework>-<UTC minute>, e.g. cis-gcp-4.0-2026-09-13T08-30Z, or cis-gcp-4.0-2026-09-13T08-30Z_002 when that minute has one)
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
        /// An authored.json written against this run's dossier: its values fill the [Authored] columns
        #[arg(long, value_name = "AUTHORED_JSON")]
        merge: Option<PathBuf>,
    },
    /// Run Checkov over the emitted HCL in hcl_dir and point each finding at the Satz block that declared the resource
    ///
    /// Failed checks exit 1
    Scan {
        /// Estate file (inside yaml_dir if relative) — compiled for the source
        /// locations of the findings; without it, findings name the HCL only
        estate: Option<String>,
    },
    /// One Markdown page per pristine pack, derived from the pack file (purpose, params, resources, claims, duties) plus an index — into `<presets_dir>/docs/`
    ///
    /// `--check` fails when the pages are behind the packs
    DocPacks {
        /// Output directory — one page per pack (default: `<presets_dir>/docs`)
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
        /// Verify instead of write: exit 1 when a page is behind its pack
        #[arg(long)]
        check: bool,
    },
    /// Check the library's packs and write `<presets_dir>/pack-graph.json`: every pack, its gate, phase, block and adoption order from the map's `offers` entries, and the edges between them — derived from the packs' param references and `ask_when`, declared in the map where the packs do not show them
    ///
    /// Nothing is written while a check fails; `--check` fails when the file is behind the library
    PackGraph {
        /// The library to read and write into (default: presets_dir from the config)
        #[arg(long, value_name = "DIR")]
        presets_dir: Option<PathBuf>,
        /// Verify instead of write: exit 1 when pack-graph.json is behind the library
        #[arg(long)]
        check: bool,
    },
    /// Every pack the pack graph offers, as this estate has it: the choice (its gate's answer and default), the line (active, ungated, commented, absent, forked or misplaced), whether it deploys, what it needs and what needs it
    ///
    /// Read-only, offline. A `use` the pack graph does not know is listed as
    /// unmanaged; the findings are the compile's own pack findings.
    Packs {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Output format — json is the report an agent or satz-studio reads, markdown and
        /// pdf the table a person reads
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Switch a pack on: bind its gate true and make its `use` line active where the pack graph places it, the packs that follow its gate with it
    ///
    /// Refused, naming them, while a pack it needs is off — `--with-requirements`
    /// switches those on too where there is one to choose — or a pack it excludes is
    /// on. The edited estate is compiled, and restored when it does not compile.
    AddPack {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// The pack: its gate (`use_audit_logsink`) or its path (`presets/monitoring/organization-audit-logsink.satz`)
        pack: String,
        /// Switch on what the pack needs too, where the pack graph names one pack for it
        #[arg(long)]
        with_requirements: bool,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]), default_value = "text")]
        format: OutFormat,
    },
    /// Switch a pack off: bind its gate false and leave its line — a gated line with a false gate deploys nothing
    ///
    /// Refused, naming them, while a pack that needs it is on — `--cascade` switches
    /// those off too — or while its line is not gated on its gate. The edited estate
    /// is compiled, and restored when it does not compile.
    RemovePack {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// The pack: its gate or its path
        pack: String,
        /// Switch off the packs that need it too
        #[arg(long)]
        cascade: bool,
        /// Output format
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]), default_value = "text")]
        format: OutFormat,
    },
    /// Judge one pack against the library's own bar: it parses, it is formatted, its header says what it is, its version has a changelog row, it declares no membership, it runs no legacy constraint beside its managed replacement, every type it emits has a prerequisite row, and it compiles
    ///
    /// A pack is a fragment, so satz folds it into an estate to see what it emits:
    /// a synthesised one — the documented example params, the pack's own declared
    /// defaults — unless `--against` names a real estate. Exits non-zero when the
    /// pack does not clear the bar.
    ReviewPack {
        /// The pack file to review (.satz)
        pack: PathBuf,
        /// Judge it inside this estate instead of a synthesised one
        #[arg(long, value_name = "ESTATE")]
        against: Option<PathBuf>,
        /// Output format — json carries the findings an editor already reads
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Run `<tf_tool> plan` in the estate's hcl dir (extra args are passed through)
    Plan {
        /// Arguments passed straight to the tool, e.g. `-target=…`, `-out=plan.tfplan`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run `<tf_tool> apply` in the estate's hcl dir (extra args are passed through)
    Apply {
        /// Arguments passed straight to the tool, e.g. `-target=…`, a saved plan file
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run `<tf_tool> init` in the estate's hcl dir (extra args are passed through)
    HclInit {
        /// Arguments passed straight to the tool, e.g. `-reconfigure`, `-migrate-state`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// What this estate can be asked: the questions its packs declare, joined with the answers its params already carry
    ///
    /// Read-only. Each question is `answered`, `unanswered` or `not-applicable`, and
    /// `blocking` while no default is possible, with what changing the answer would cost.
    Questions {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Output format — markdown and pdf are the decisions sheet a human reads
        /// before an organisation is touched
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Markdown, OutFormat::Pdf, OutFormat::Json, OutFormat::Xlsx]))]
        format: OutFormat,
        /// Where it goes — the one file this run writes, the format's extension added
        /// when the name has none (`-` for stdout)
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Only the questions the estate has not answered yet — the interview's worklist
        #[arg(long)]
        unanswered: bool,
    },
    /// The Prowler invocation this estate needs — printed, never run
    ///
    /// satz does not run Prowler: the scan spends API quota in every project of the
    /// estate, and Prowler reads as whoever is logged in rather than as the estate's
    /// service account. What this answers is which frameworks, which projects, which
    /// formats and which output path — read from what the estate actually declares.
    Prowler {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Output format — json is for an agent
        #[arg(long, value_parser = crate::out::formats(&[OutFormat::Text, OutFormat::Json]), default_value = "text")]
        format: OutFormat,
    },
    /// Format Satz files in place: indentation, spacing, `=` alignment, list commas
    ///
    /// The layout is the corpus's own — two spaces per brace or bracket that spans
    /// lines, `=` aligned over a run of attributes, every item of a list laid out
    /// over lines ending in a comma, a construct that spans lines opening at the end
    /// of its line and closing on a line of its own. The author's line breaks stay;
    /// strings and `hcl { … }` bodies are verbatim. Meaning never changes: the
    /// canonical form `check-presets` compares is the same before and after.
    Fmt {
        /// Files or directories to format (.satz; a directory is walked, *.diff.satz skipped)
        paths: Vec<PathBuf>,
        /// Name the files that are not formatted and exit 1; write nothing
        #[arg(long)]
        check: bool,
        /// Read one file from stdin and write it formatted to stdout
        #[arg(long, conflicts_with_all = ["check", "paths"])]
        stdin: bool,
    },
    /// What this estate, or this machine, has said "seen, move on" to — and say it
    ///
    /// A finding is silenced by what it IS: its `kind`, as `--format json` spells it,
    /// and optionally its `subject` — the pack, the notice's param, the action's name,
    /// the `hcl` block's `file:line`. Never by its wording. The estate's `config.toml`
    /// holds the rows a review reads, with a reason each; `--machine` holds this
    /// operator's, whole kinds only. A silenced finding is still produced, still
    /// counted and still in `--format json`; only the printed output leaves it out,
    /// and every run says how many. An error is never silenced.
    Silence {
        #[command(subcommand)]
        sub: SilenceSub,
    },
    /// The language server behind an editor's Satz support (Language Server Protocol, stdio)
    ///
    /// Started by the editor, never by hand. Diagnostics from the parser on every
    /// change and from the whole pipeline on every open and save — the errors
    /// `transpile --check` prints, at the file and line they name; completion and
    /// hover from the provider schema the estate's config.toml points at;
    /// go-to-definition for `use` paths and params; formatting is `satz fmt`.
    Lsp,
    /// Answer what the estate's packs ask, one question at a time, writing each answer
    /// into the estate's params. The third way to start an estate: `init` takes every
    /// answer as a flag, an agent asks over MCP, this asks a person at a terminal
    Interview {
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Write the estate first if it does not exist: a skeleton that uses
        /// presets/estate-core.satz, with every question open
        #[arg(long)]
        create: bool,
        /// Ask every question, answered ones too, with the current answer as the default
        #[arg(long)]
        all: bool,
        /// Accept every offered default up front and ask only what needs a value
        #[arg(long)]
        accept_defaults: bool,
    },
    /// Serve this estate over the Model Context Protocol (stdio), so an agent drives satz
    ///
    /// satz never calls a model; this is the other direction. `--allow` sets a ceiling the
    /// client cannot raise: read (compile and report), write (writes files in the estate),
    /// exec (runs external tools or changes a live org).
    Mcp {
        /// Directory the server may work under: every config and estate it opens
        /// must live inside it (default: the current directory). The server has
        /// no estate until a client opens one, so one server serves a fleet
        #[arg(long)]
        root: Option<PathBuf>,
        /// Capability groups to grant, comma-separated: read, write, exec
        #[arg(long, default_value = "read")]
        allow: String,
        /// Let the client LOWER its own level at runtime (never raise it)
        #[arg(long)]
        self_gated: bool,
    },
    /// Open the documentation site in the browser
    OpenReadme,
    /// Show which identity, credential type and quota project the Application
    /// Default Credentials resolve to
    Whoami {
        /// Estate to answer FOR (.satz, inside yaml_dir if relative): a
        /// cloud-mode estate runs as its IaC service account, impersonated by the
        /// credentials; a local-mode one runs as the credentials, and names the
        /// account `satz migrate --mode cloud` switches to. Omit to report the
        /// ambient credentials.
        input: Option<PathBuf>,
        /// Read the ADC file only — no network, no token minted
        #[arg(long)]
        offline: bool,
    },
    /// Generate shell completion script
    Completion {
        /// Shell to generate completions for: bash, zsh, fish, powershell
        /// (auto-detected from $SHELL if omitted)
        shell: Option<String>,
        /// Install the completion script to the default location for the shell
        /// (auto-enabled on macOS when no shell is specified)
        #[arg(long)]
        install: bool,
    },
}

#[derive(Subcommand)]
enum SilenceSub {
    /// Every silence in force, with its reason and what it still silences
    List {
        /// Estate file, .satz (inside yaml_dir if relative) — compile it, and say per
        /// row how many findings it silences, or that nothing answers to it any more
        input: Option<String>,
    },
    /// Silence a kind, or one subject of a kind
    Add {
        /// `<kind>` or `<kind>:<subject>` — `satz silence list` and `--format json`
        /// spell both
        selector: String,
        /// Why — mandatory: this is what a review reads months later
        #[arg(long)]
        reason: String,
        /// Write it to this machine (~/.config/satz/satz.toml) instead of the estate;
        /// whole kinds only
        #[arg(long)]
        machine: bool,
    },
    /// Remove a silence, by the same selector
    Remove {
        /// `<kind>` or `<kind>:<subject>`
        selector: String,
        /// Remove it from this machine instead of the estate
        #[arg(long)]
        machine: bool,
    },
}

/// User-level settings for satz in ~/.config/satz/satz.toml. Created on first run with defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GlobalSettings {
    /// When to check for updates: "never", "always", "daily". Default "always".
    #[serde(default = "default_self_update_frequency")]
    self_update_frequency: String,
    /// Last time we ran an update check (unix timestamp string). Used for "daily" throttle.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_update_check: Option<String>,
    /// What this operator has said "seen, move on" to on every estate they open:
    /// `[[silence]]` tables naming a whole `kind`, never a subject. Managed by
    /// `satz silence add --machine`.
    ///
    /// Last in the struct because TOML writes tables after scalars.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    silence: Vec<crate::silence::Rule>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            self_update_frequency: default_self_update_frequency(),
            last_update_check: None,
            silence: Vec::new(),
        }
    }
}

fn default_self_update_frequency() -> String {
    "always".to_string()
}

fn global_settings_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".config").join("satz").join("satz.toml"))
}

/// The user's home: `HOME`, and on Windows `USERPROFILE` first — native Windows sets no
/// `HOME`, so satz read no settings there and checked for updates on every command.
/// `%USERPROFILE%\.config\satz\satz.toml` is where satz-studio writes them too.
fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    var.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Load global settings. If the file does not exist, create ~/.config/satz/satz.toml with default values.
/// The global settings (`~/.config/satz/satz.toml`), created with defaults
/// when absent. A file that exists but cannot be read or parsed is an error —
/// "defaults" would be a silent reset that the next save writes back.
fn load_global_settings() -> Result<GlobalSettings, Box<dyn std::error::Error>> {
    let path = match global_settings_path() {
        Some(p) => p,
        None => return Ok(GlobalSettings::default()),
    };
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        // a settings file that does not parse is not "defaults": the next
        // save would overwrite what the user wrote
        let settings: GlobalSettings = toml::from_str(&content).map_err(|e| -> Box<dyn std::error::Error> {
            format!("{}: not valid TOML ({}) — fix the file or delete it to start from defaults", path.display(), e).into()
        })?;
        // A machine silences whole kinds. A subject is one estate's business, and a
        // row that names one here would hide that one thing at every customer.
        if let Some(r) = settings.silence.iter().find(|r| r.subject.is_some()) {
            return Err(format!(
                "{}: [[silence]] kind = \"{}\", subject = \"{}\" — this file silences whole kinds only. \
                 Move the row to that estate's config.toml, or drop the subject.",
                path.display(),
                r.kind,
                r.subject.as_deref().unwrap_or_default()
            )
            .into());
        }
        return Ok(settings);
    }
    // First run: create directory and write defaults
    let defaults = GlobalSettings::default();
    save_global_settings(&defaults)?;
    Ok(defaults)
}

fn save_global_settings(settings: &GlobalSettings) -> Result<(), Box<dyn std::error::Error>> {
    let path = match global_settings_path() {
        Some(p) => p,
        None => return Err("HOME not set".into()),
    };
    if let Some(parent) = path.parent() {
        fsx::create_dir_all(parent)?;
    }
    let toml = toml::to_string_pretty(settings)?;
    fsx::write(&path, toml)?;
    Ok(())
}

/// An error is printed as what it says. Returned from `main`, it would be printed by
/// the `Debug` formatter: a message in quotes with its newlines escaped, and a refused
/// compile as a struct dump of every finding.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            match crate::findings::as_refusal(e.as_ref()) {
                // the errors in the layout the warnings above them were printed in
                Some(refusal) => crate::findings::report_refusal(refusal),
                None => eprintln!("error: {}", e),
            }
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // stderr, not stdout: stdout carries the ANSWER. A banner in front of it makes
    // `--format json` unparseable by anything downstream, and once `satz mcp` speaks
    // JSON-RPC over stdout a stray line there is a corrupt protocol stream rather
    // than cosmetic noise. A human still sees it; a pipe no longer does.
    eprintln!("satz v{} (built {})", env!("CARGO_PKG_VERSION"), env!("BUILD_DATE"));
    // A root-level help request is answered before parsing, because clap prints
    // and exits inside get_matches() and would print its own flat command list.
    // A request that names a command stays clap's: `satz help transpile` and
    // `satz transpile --help` never reach here.
    if let Some(long) = root_help_request() {
        print_root_help(long);
        std::process::exit(0);
    }
    // parse into matches first: the subcommand NAME is what --html-help needs,
    // and clap only hands it out at this level
    let matches = cli_command().get_matches();
    let subcommand = matches.subcommand_name().map(|s| s.to_string());
    let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches)?;
    if cli.html_help {
        return open_html_help(subcommand.as_deref());
    }
    // The run tier of `satz silence`. Refused for the two servers: `satz mcp` and
    // `satz lsp` live for many estates and many calls at once, and a silence given
    // once on their command line would hold for all of them. What CI hides, an agent
    // and an editor still see.
    let run_silences = silence::run_tier(&cli.silence, std::env::var("SATZ_SILENCE").ok().as_deref())?;
    if !run_silences.is_empty() && matches!(subcommand.as_deref(), Some("mcp") | Some("lsp")) {
        return Err(format!(
            "satz {}: --silence and SATZ_SILENCE are one run's, and a server serves many estates in one \
             process. Silence it in the estate's config.toml (`satz silence add … --reason \"…\"`) or on \
             this machine (`satz silence add … --machine --reason \"…\"`).",
            subcommand.unwrap_or_default()
        )
        .into());
    }
    silence::set_run(run_silences);

    // Load/create global settings on first run (creates ~/.config/satz/satz.toml with defaults)
    let mut global_settings = load_global_settings()?;
    silence::set_machine(global_settings.silence.clone());

    let cmd_choice = match cli.command {
        Some(c) => c,
        None => {
            if cli.verbose {
                print_recursive_help(false);
            } else {
                print_root_help(false);
            }
            std::process::exit(0);
        }
    };

    let config_file_path = if let Some(path) = &cli.config {
        // Accept either the config FILE or the estate DIRECTORY that holds it, so
        // `--config ~/projects/acme` works as well as `--config ~/projects/acme/config.toml`.
        // Every path inside the config is already resolved against the config's own
        // directory, so with either form the command is fully location-independent.
        if path.is_dir() {
            let candidate = path.join("config.toml");
            if !candidate.exists() {
                return Err(format!(
                    "--config {}: directory has no config.toml (pass the file directly if it is named otherwise)",
                    path.display()
                )
                .into());
            }
            candidate
        } else if path.is_file() || matches!(cmd_choice, Commands::Init { .. }) {
            // `init` writes the file --config names; every other command reads it
            path.clone()
        } else {
            return Err(format!("--config {}: no such file or directory", path.display()).into());
        }
    } else {
        let default_config = PathBuf::from("config.toml");
        if default_config.exists() {
            default_config
        } else {
            // Config is mandatory for Transpile and other commands that need it
            match cmd_choice {
                Commands::Transpile { .. } | Commands::ScanPlan { .. } | Commands::GenerateMigration { .. } | Commands::UpdateSchema { .. } | Commands::Import { .. } | Commands::Migrate { .. } | Commands::Bootstrap { .. } | Commands::ExportOrganizationalPolicies { .. } | Commands::DiffOrganizationalPolicies { .. } | Commands::ReportOrganizationalPolicies { .. } | Commands::GetPresets { .. } | Commands::CheckPresets { .. } | Commands::Require { .. } | Commands::ReportCompliance { .. } | Commands::Adopt { .. } | Commands::MapTypes { .. } | Commands::Scan { .. } | Commands::DocPacks { .. } | Commands::Triage { .. } | Commands::RemediationPlan { .. } | Commands::AdoptOrgPolicies { .. } | Commands::MergePresets { .. } | Commands::RunActions { .. } | Commands::Questions { .. } | Commands::Interview { .. } | Commands::Prowler { .. }
                | Commands::Packs { .. } | Commands::AddPack { .. } | Commands::RemovePack { .. }
                | Commands::Plan { .. } | Commands::Apply { .. } | Commands::HclInit { .. }
                | Commands::ReviewPack { .. }
                | Commands::PackGraph { presets_dir: None, .. }
                // the estate tier lives in the estate's config.toml. `--machine` needs
                // none, and a bare `list` says what this machine silences from anywhere
                | Commands::Silence {
                    sub: SilenceSub::List { input: Some(_) } | SilenceSub::Add { machine: false, .. } | SilenceSub::Remove { machine: false, .. },
                }
                | Commands::UpdatePrerequisites { input: Some(_), .. } => {
                    // plan/apply/hcl-init hand everything after the subcommand to the
                    // tool verbatim, which also swallows a `--config` written after
                    // those args. "config.toml not found" is baffling then, so name
                    // the actual fix.
                    // Printed ahead of the error: the detail first, then the one line `main` closes with.
                    if let Some(hint) = misplaced_config_hint(&cmd_choice) {
                        eprintln!("\n{}\n", hint);
                        return Err("--config came after the pass-through arguments".into());
                    }
                    return Err("Config file 'config.toml' not found in current directory. Please provide it or specify --config <PATH>.".into());
                }
                Commands::Init { .. } | Commands::SelfUpdate { .. } | Commands::Completion { .. } | Commands::OpenReadme | Commands::Whoami { .. } | Commands::Mcp { .. } | Commands::Fmt { .. } | Commands::Lsp
                // what is left of `silence` here is `--machine`, which writes
                // ~/.config/satz/satz.toml and has no estate
                | Commands::Silence { .. }
                | Commands::UpdatePrerequisites { input: None, .. }
                | Commands::PackGraph { presets_dir: Some(_), .. } => {
                    // These commands can proceed without a config file
                    PathBuf::from("config.toml")
                }
            }
        }
    };

    // Optional: check for updates per global settings — never for the commands that
    // own stdout as a protocol or that are the update itself.
    if checks_for_updates(&cmd_choice) {
        if let Err(e) = maybe_check_for_updates(&mut global_settings).await {
            eprintln!("⚠️  update check: {}", e);
        }
    }

    // --no-impersonate wins over everything: pinning the process to the plain
    // ADC here makes every later per-command binding a no-op rather than a
    // conflict — the operator asked for this, so an estate does not override it.
    if cli.no_impersonate {
        crate::gcp::disable_impersonation();
    }

    // `--no-api-preflight`: `plan` and `apply` run the tool without asking
    // Service Usage anything. Carried to `run_tf`, which is the one place that
    // preflights, and ignored by `init`, which preflights nothing.
    let api_preflight = !cli.no_api_preflight;

    let config_dir = config_file_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let tool_config: ToolConfig = match parse_tool_config(&config_file_path) {
        Ok(c) => c,
        // Printed ahead of the error: the detail first, then the one line `main` closes with.
        Err(described) => {
            eprintln!("\n{}\n", described);
            return Err(format!("could not parse '{}' as TOML", config_file_path.display()).into());
        }
    };
    let mut runtime_config = resolved_config(&tool_config, &config_dir);
    if let Some(level) = &cli.validation {
        runtime_config.validation_level = level.clone();
    }
    if !["warn", "error", "none"].contains(&runtime_config.validation_level.as_str()) {
        return Err(format!(
            "validation level `{}`: expected warn, error or none",
            runtime_config.validation_level
        )
        .into());
    }

    // One rule in front of every command that changes a customer's organisation: a pack
    // says what has to be done before its resources are applied, and a message it
    // declared an `error` refuses the run until the estate acknowledges it
    // (`src/org_write.rs`).
    org_write::refuse(&cmd_choice, &tool_config, &runtime_config)?;

    match cmd_choice {
        Commands::Transpile { input, output, schema_dir, print_variables, plan, apply, scan, check, format } => {

            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            if let Some(sd) = &schema_dir {
                runtime_config.schema_dir = if Path::new(sd).is_absolute() {
                    sd.to_string_lossy().to_string()
                } else {
                    config_dir.join(sd).to_string_lossy().to_string()
                };
            }
            // Satz only (M5, 2026-08-29): the legacy walk is gone. A `.yaml`
            // estate is migrated, never transpiled — `reject_yaml_estate` says
            // so and names the converter.
            reject_yaml_estate(&input_path, "transpile")?;
            if format == OutFormat::Json {
                if print_variables || plan || apply || scan {
                    return Err("--format json is the compile as data; --print-variables, --plan, --apply and --scan print their own output — run them without it".into());
                }
                return transpile_as_json(&input_path, &input, output.as_deref(), check, &tool_config, &runtime_config);
            }
            let out = pipeline_b_generate(&input_path, &tool_config, &runtime_config)?;
            if print_variables {
                println!("{}", out.tfvars);
            }
            if check {
                println!(
                    "transpile --check: OK — {} compiles; nothing was written",
                    input_path.display()
                );
                return Ok(());
            }
            let base_output_path = hcl_target(output.as_deref(), &runtime_config);
            for p in write_hcl(&out, &base_output_path, &input)? {
                println!("Created {}", p.display());
            }
            if plan || apply {
                // Same gate as bootstrap: apply refuses, plan warns.
                match crate::questions::require_complete(&input_path, &runtime_config, if apply { "apply" } else { "plan" }) {
                    Ok(()) => {}
                    Err(e) if !apply => eprintln!("warning: {}", e),
                    Err(e) => return Err(e.into()),
                }
                // And the same for an API nothing enables: the apply would run until
                // it reached that resource and then fail, leaving half an estate.
                match prerequisites_report(&input_path, &tool_config, &runtime_config, PrerequisiteFindings::Report)
                    .map_err(|e| e.to_string())
                    .and_then(|r| require_apis_declared(&r, if apply { "apply" } else { "plan" }))
                {
                    Ok(()) => {}
                    Err(e) if !apply => eprintln!("warning: {}", e),
                    Err(e) => return Err(e.into()),
                }
                // one tool: transpile, then the tool, in the estate's hcl dir
                if output.is_some() {
                    return Err("--plan/--apply run in hcl_dir; drop --output".into());
                }
                let hcl_dir = Path::new(&runtime_config.hcl_dir);
                if !hcl_dir.join(".terraform").exists() {
                    run_tf(&runtime_config, "init", &["-input=false".to_string()], api_preflight).await?;
                }
                run_tf(&runtime_config, if apply { "apply" } else { "plan" }, &[], api_preflight).await?;
            }
            if scan {
                if output.is_some() {
                    return Err("--scan runs over hcl_dir; drop --output".into());
                }
                let report = crate::scan::run(Path::new(&runtime_config.hcl_dir))?;
                print!("{}", crate::scan::render(&report, Some(&out.manifest)));
                if report.failed > 0 {
                    std::process::exit(1);
                }
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        Commands::Init {
            defaults,
            providers,
            tf_tool,
            customer_id,
            customer_shortname,
            billing_account_infra,
            default_region,
            customer_organization_id,
            customer_domain,
            infra_project_name,
            infra_bucket_name,
            iac_user,
            from_live,
            force,
            interview,
        } => {
            let mut final_google = Vec::new();
            let mut final_aws = Vec::new();
            let mut final_azure = Vec::new();
            let mut final_alibaba = Vec::new();

            if let Some(defs) = defaults {
                for d in defs {
                    if d.as_str() == "google" {
                        final_google.extend(vec!["google".to_string(), "google-beta".to_string()]);
                    }
                }
            }

            if let Some(provs) = providers {
                // For explicit providers, we'll put them in google for now if they start with google, or general
                for p in provs {
                    if p.starts_with("google") { final_google.push(p); }
                    else if p.starts_with("aws") { final_aws.push(p); }
                    else if p.starts_with("az") { final_azure.push(p); }
                    else if p.starts_with("ali") { final_alibaba.push(p); }
                }
            }

            // Deduplicate
            final_google.sort(); final_google.dedup();

            let tool = tf_tool.unwrap_or_else(|| tool_config.tf_tool.clone());

            // 1. Create Directories
            //
            // Everything init produces is anchored to config.toml: its directory when the
            // file names no locations, or the locations it names when it does. Both are
            // already folded into runtime_config, so using it here keeps the directories,
            // the config file and the customer template in one place. Reading the raw
            // tool_config instead created ./yaml while writing the template to
            // ../yaml/X.yaml, which then failed because that directory never existed.
            let dirs = vec![
                &runtime_config.yaml_dir,
                &runtime_config.hcl_dir,
                &runtime_config.schema_dir,
            ];
            for d in dirs {
                fsx::create_dir_all(d)?;
                println!("Created directory: {}", d);
            }

            // 2. Generate config.toml if missing.
            // Written to the path --config names (default ./config.toml), and holding the
            // raw relative values, since they are interpreted from this file's directory.
            if !config_file_path.exists() {
                let mut config_lines = vec![
                    format!("schema_dir = \"{}\"", tool_config.schema_dir),
                    format!("presets_dir = \"{}\"", tool_config.presets_dir),
                    format!("yaml_dir = \"{}\"", tool_config.yaml_dir),
                    format!("hcl_dir = \"{}\"", tool_config.hcl_dir),
                    // From the same source as the serde default, so the generated file
                    // and an omitted key can never disagree again.
                    format!("include_dirs = {:?}", tool_config.include_dirs),
                    format!("tf_tool = \"{}\"", tool),
                ];

                if !final_google.is_empty() {
                    config_lines.push(format!("google_providers = {:?}", final_google));
                }
                if !final_aws.is_empty() {
                    config_lines.push(format!("aws_providers = {:?}", final_aws));
                }
                if !final_azure.is_empty() {
                    config_lines.push(format!("azure_providers = {:?}", final_azure));
                }
                if !final_alibaba.is_empty() {
                    config_lines.push(format!("alibaba_providers = {:?}", final_alibaba));
                }

                config_lines.push(format!("provider_version = \"{}\"", tool_config.provider_version));
                config_lines.push(format!("auto_explode = {:?}", tool_config.auto_explode));
                config_lines.push(format!("validation_level = \"{}\"", tool_config.validation_level));

                fsx::write(&config_file_path, config_lines.join("\n"))?;
                println!("Generated {}", config_file_path.display());
            }

            // 3. Generate .gitignore if missing, next to config.toml — it is the project
            // root that the generated directories hang off, not wherever init was run.
            let gitignore_path = config_dir.join(".gitignore");
            if !gitignore_path.exists() {
                let gitignore_content = r#"# Terraform / OpenTofu
.terraform/
*.tfstate
*.tfstate.backup

# Tool Cache
schemas/

# OS files
.DS_Store
Thumbs.db
"#;
                fsx::write(&gitignore_path, gitignore_content)?;
                println!("Created {}", gitignore_path.display());
            }

            // 3b. What the operator TYPED, before anything is derived: a re-run
            // merges exactly these into an estate that already exists, and
            // nothing else, so a value somebody put there by hand survives.
            let stated = crate::init_params::Stated {
                customer_id: customer_id.clone(),
                customer_shortname: customer_shortname.clone(),
                billing_account_infra: billing_account_infra.clone(),
                default_region: default_region.clone(),
                customer_organization_id: customer_organization_id.clone(),
                customer_domain: customer_domain.clone(),
                infra_project_name: infra_project_name.clone(),
                infra_bucket_name: infra_bucket_name.clone(),
                iac_user: iac_user.clone(),
            };

            // Derivation from the credentials is the DEFAULT, not a flag: every
            // value below is sitting in the ADC the operator already
            // authenticated with. Stated wins, derived fills the rest and says
            // where it came from, and what nothing can answer stays EMPTY —
            // never a placeholder. `--from-live` is accepted and ignored.
            let _ = from_live;
            let (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user) = {
                let need_org = customer_organization_id.is_none() || customer_id.is_none();
                let need_billing = billing_account_infra.is_none();
                match crate::gcp::identity::live_defaults(need_org, need_billing, None).await {
                    Ok(live) => {
                        let mut note = crate::init_params::Derivations::default();
                        let customer_domain = note.fill(
                            "customer_domain",
                            customer_domain,
                            Some(live.customer_domain.clone()),
                            "the ADC identity",
                        );
                        let iac_user = note.fill(
                            "first_admin",
                            iac_user,
                            Some(format!("{}@{}", live.first_admin, live.customer_domain)),
                            "the ADC identity",
                        );
                        let customer_id =
                            note.fill("customer_id", customer_id, live.customer_id.clone(), "organizations:search");
                        // No organization visible is the greenfield case: the id
                        // stays empty and `bootstrap --greenfield` fills it in.
                        let customer_organization_id = note.fill(
                            "customer_organization_id",
                            customer_organization_id,
                            live.org_id.clone(),
                            "organizations:search",
                        );
                        let billing_account_infra = note.fill(
                            "billing_account_infra",
                            billing_account_infra,
                            live.billing_account.clone(),
                            "billingAccounts.list (the one open account)",
                        );
                        // nothing on the platform names the customer: it is
                        // reported as unanswered rather than guessed at
                        let customer_shortname = note.fill("customer_shortname", customer_shortname, None, "");
                        note.report();
                        (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
                    }
                    Err(why) => {
                        eprintln!("init: nothing could be derived from the credentials — {}", why);
                        eprintln!(
                            "      what you did not pass is written empty; `satz bootstrap` names each one, and \
                             `satz init` merges them in later."
                        );
                        (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
                    }
                }
            };

            // 4. Generate the template estate if customer_id provided
            if let Some(c_id) = customer_id {
                // Every day-0 question lives in presets/estate-core.satz, and the pack lines
                // come from the graph beside it: `--interview` fetches the library before the
                // file is written, so the estate it asks about carries the menu.
                if interview {
                    let core = Path::new(&runtime_config.presets_dir).join("estate-core.satz");
                    if !core.exists() {
                        println!("\nfetching the preset library — the day-0 questions are read from {}", core.display());
                        crate::presets::get_presets(&runtime_config.presets_dir, &runtime_config, false, None)
                            .await
                            .map_err(|e| {
                                format!(
                                    "init --interview: the questions live in {}, which is not here and could not be \
                                     fetched ({}) — run `satz get-presets`, then `satz interview {}.satz`",
                                    core.display(),
                                    e,
                                    c_id
                                )
                            })?;
                    }
                }
                let yaml_path = PathBuf::from(&runtime_config.yaml_dir).join(format!("{}.satz", c_id));
                if yaml_path.exists() && !force {
                    // A re-run MERGES: it used to print "Template already exists",
                    // change nothing, and still sign off with "Initialization
                    // complete" — so a param somebody added to the command line
                    // never landed and nothing said so.
                    let src = crate::fsx::read_to_string(&yaml_path)?;
                    let (merged, log) = crate::init_params::merge(&src, &stated)?;
                    if log.is_empty() {
                        println!(
                            "{} exists and this command named no params to merge into it (`--force` rewrites it).",
                            yaml_path.display()
                        );
                    } else {
                        for entry in &log {
                            match entry {
                                crate::init_params::Merged::Changed { param, from, to } if from.is_empty() => {
                                    println!("  set     {} = {:?}", param, to)
                                }
                                crate::init_params::Merged::Changed { param, from, to } => {
                                    println!("  changed {} = {:?} (was {:?})", param, to, from)
                                }
                                crate::init_params::Merged::Same { param, value } => {
                                    println!("  kept    {} = {:?}", param, value)
                                }
                            }
                        }
                        if merged != src {
                            crate::fsx::write_edited_satz(&yaml_path, &src, &merged)?;
                        }
                        println!(
                            "Merged into {} — every line this command did not name is unchanged.",
                            yaml_path.display()
                        );
                    }
                } else {
                    let domain = customer_domain.clone().unwrap_or_default();
                    // an admin nobody named stays empty: `first.admin` looked
                    // like an answer and was not one
                    let resolved_iac_user = iac_user.unwrap_or_default();

                    // The template and the shipped presets both compose members as
                    // `user:{first-admin}@{customer-domain}`, so `first-admin` holds the
                    // local part only. Emitting it as a variable is what lets the
                    // `*first-admin` anchor resolve at all.
                    let (first_admin, user_domain) = resolved_iac_user
                        .split_once('@')
                        .unwrap_or((resolved_iac_user.as_str(), ""));
                    if !user_domain.is_empty() && !domain.is_empty() && user_domain != domain {
                        eprintln!(
                            "Warning: --iac-user domain '{}' differs from --customer-domain '{}'. \
                             Members are built as first-admin@customer-domain, so they will use '{}@{}'.",
                            user_domain, domain, first_admin, domain
                        );
                    }

                    // the two names that FOLLOW from the short name, by the
                    // defaults `presets/estate-core.satz` documents — derived
                    // when it is known, empty when it is not
                    let shortname = customer_shortname.clone().unwrap_or_default();
                    let derive_from_shortname = |given: Option<String>, suffix: &str| -> String {
                        match given {
                            Some(v) => v,
                            None if !shortname.trim().is_empty() => format!("{}{}", shortname.trim(), suffix),
                            None => String::new(),
                        }
                    };
                    let infra_project_name = derive_from_shortname(infra_project_name, "-infra-001");
                    let infra_bucket_name = derive_from_shortname(infra_bucket_name, "-infra-001-state");
                    let args = crate::template::TemplateArgs {
                        customer_id: c_id.clone(),
                        shortname: customer_shortname.unwrap_or_default(),
                        billing_id: billing_account_infra.unwrap_or_default(),
                        region: default_region.unwrap_or_else(|| "europe-west3".to_string()),
                        // never a placeholder: unset stays empty, and the
                        // bootstrap gate refuses it by name
                        org_id: customer_organization_id.unwrap_or_default(),
                        domain: domain.clone(),
                        project_id: infra_project_name,
                        bucket_id: infra_bucket_name,
                        first_admin: first_admin.to_string(),
                    };
                    let presets_dir = Path::new(&runtime_config.presets_dir);
                    let graph = crate::pack_graph::read(presets_dir)?;
                    crate::template::generate_template(&args, graph.as_ref(), &yaml_path)?;
                    if graph.is_none() {
                        println!("{}", crate::pack_graph::no_menu_note(presets_dir));
                    }
                    println!("Generated estate: {} — next: `satz bootstrap {}.satz --dry-run`", yaml_path.display(), c_id);
                }

                // A day-0 param is either stated, derived, or ASKED — there is
                // no fourth state where an estate is simply born incomplete.
                // Stating it is the flag because the interview is interactive
                // and a scripted run must not block on it.
                if interview {
                    // init writes the estate-core `use` commented out so the estate compiles
                    // before any library is fetched; the interview switches it on, or it
                    // finds no pack declaring a question and asks nothing
                    if crate::template::use_estate_core(&yaml_path)? {
                        println!("switched on `use \"presets/estate-core.satz\"` in {}", yaml_path.display());
                    }
                    println!();
                    let stdin = std::io::stdin();
                    let mut input = stdin.lock();
                    let mut out = std::io::stdout();
                    crate::interview::run(&yaml_path, &runtime_config, false, false, &mut input, &mut out)?;
                }
            }

            // 4. Fetch Schemas
            //
            // For the providers the command line names, else the ones the config does —
            // which defaults to google and google-beta. Only the flags used to count, so a
            // plain `satz init` created an empty schema dir, fetched nothing, said nothing,
            // and printed "Initialization complete" for an estate nothing could compile.
            // A schema already on disk is kept: a re-run merges params, and re-downloading
            // two providers to do that is not what anyone asked for.
            let mut all_provs = final_google;
            all_provs.extend(final_aws);
            all_provs.extend(final_azure);
            all_provs.extend(final_alibaba);
            let wanted: Vec<(String, String)> = if all_provs.is_empty() {
                tool_config.parsed_providers()
            } else {
                all_provs.into_iter().map(|p| (p, runtime_config.provider_version.clone())).collect()
            };
            for (provider, version) in wanted {
                let name = provider.split('/').next_back().unwrap_or(&provider).to_string();
                let out = format!("{}/{}.json", runtime_config.schema_dir, name);
                if Path::new(&out).exists() {
                    println!("Schema for {} already present: {}", name, out);
                    continue;
                }
                println!("Fetching schema for {} {}...", provider, version);
                crate::schema::ResourceRegistry::generate_schema(&tool, &provider, &version, &out).map_err(|e| {
                    format!(
                        "init: the estate is written, but the {} schema could not be fetched ({}) — nothing compiles \
                         without it. Put OpenTofu on PATH and run `satz update-schema`",
                        name, e
                    )
                })?;
            }
            println!("Initialization complete.");
            Ok(())
        }
        Commands::UpdateSchema { providers, version, tf_tool } => {
            let tool = tf_tool.unwrap_or_else(|| tool_config.tf_tool.clone());
            // A fresh clone has no schema_dir yet (it is git-ignored).
            fsx::create_dir_all(&runtime_config.schema_dir)?;
            
            // If explicit providers are given, use them with CLI version or default
            // If not, iterate all providers from config and use their specific versions
            
            if let Some(p_list) = providers {
                 let def_ver = version.unwrap_or_else(|| tool_config.provider_version.clone());
                 for prov in p_list {
                     let (p_name, p_ver) = ToolConfig::parse_provider_string_with_default(&prov, &def_ver);
                     let out = PathBuf::from(format!("{}/{}.json", runtime_config.schema_dir, p_name.split('/').next_back().unwrap_or(&p_name)));
                     println!("Updating schema for {} version {} using {}...", p_name, p_ver, tool);
                     ResourceRegistry::generate_schema(&tool, &p_name, &p_ver, out.to_str().ok_or_else(|| format!("{}: the schema path is not UTF-8", out.display()))?)?;
                 }
            } else {
                 // Use parsed config
                 for (p_name, p_ver) in tool_config.parsed_providers() {
                      // Override if version passed (unlikely for bulk update but possible)
                      let usage_ver = version.clone().unwrap_or(p_ver);
                      let out = PathBuf::from(format!("{}/{}.json", runtime_config.schema_dir, p_name.split('/').next_back().unwrap_or(&p_name)));
                      println!("Updating schema for {} version {} using {}...", p_name, usage_ver, tool);
                      ResourceRegistry::generate_schema(&tool, &p_name, &usage_ver, out.to_str().ok_or_else(|| format!("{}: the schema path is not UTF-8", out.display()))?)?;
                 }
            }
            println!("Done.");
            Ok(())
        }
        Commands::ScanPlan { plan_json, output } => {
            let p_json = if plan_json.is_absolute() { plan_json } else { config_dir.join(plan_json) };
            let mapping = crate::state_migration::scan_plan(&p_json)?;
            let yaml = serde_yaml::to_string(&mapping)?;

            let final_output = if output.is_absolute() { output } else { config_dir.join(output) };
            fsx::write(&final_output, yaml)?;
            println!("Mapping generated: {}", final_output.display());
            Ok(())
        }
        Commands::GenerateMigration { mapping, output } => {
            let m_path = if mapping.is_absolute() { mapping } else { config_dir.join(mapping) };
            let final_output = if output.is_absolute() { output } else { config_dir.join(output) };
            crate::state_migration::generate_migration(&m_path, &final_output, &tool_config.tf_tool, &runtime_config.hcl_dir)?;
            println!("Migration script generated: {}", final_output.display());
            Ok(())
        }
        Commands::Import { source, from, only, all, exclude, output, import_config, gate, kind, fork, into, wrap_all, on_collision, customer_shortname } => {
            let cfg_opt = load_import_config(import_config, &tool_config, &runtime_config.presets_dir)?;
            let shape = match from {
                Some(f) => f,
                None => detect_import_shape(source.as_deref(), cfg_opt.as_ref().and_then(|c| c.root.as_ref()))?,
            };
            match shape.as_str() {
                "yaml" => {
                    let src = source.ok_or("the yaml shape needs a file to convert")?;
                    // the conversion lands beside its source (or as its `.local` fork):
                    // an --output it would ignore is refused rather than dropped
                    if let Some(o) = &output {
                        return Err(format!(
                            "--output {}: the yaml shape writes the conversion beside {} (or its `.local` fork with --fork) — leave --output off",
                            o.display(),
                            src
                        )
                        .into());
                    }
                    convert_yaml_to_satz(PathBuf::from(src), gate, kind, fork, &tool_config, &runtime_config)
                }
                "hcl" => {
                    let src = source.ok_or("the hcl shape needs a directory of .tf files, or one file")?;
                    let output = output.unwrap_or_else(|| PathBuf::from("imported-hcl.satz"));
                    import_hcl(&src, output, wrap_all, cli.verbose, &runtime_config)
                }
                "state" | "org" => {
                    let mut cfg = cfg_opt.ok_or_else(|| missing_import_config(&runtime_config.presets_dir))?;
                    if all {
                        let on = cfg.apply_all(shape == "org");
                        println!("import: --all — {} type(s) switched on beside the table's defaults", on);
                    }
                    let filter: Vec<String> = if only.is_empty() { cfg.only.clone().unwrap_or_default() } else { only };
                    let mut filtered: std::collections::HashSet<String> = std::collections::HashSet::new();
                    if !filter.is_empty() {
                        let off = cfg.apply_only(&filter);
                        println!("import: only {} — {} type(s) switched off by the filter", filter.join(","), off.len());
                        if cli.verbose {
                            for t in &off { println!("  filtered: {}", t); }
                        }
                        filtered = off.into_iter().collect();
                        if !cfg.resource_types.values().any(|r| r.import) {
                            return Err(format!("import: --only {} matches no enabled type — nothing would be imported", filter.join(",")).into());
                        }
                    }
                    let leave_out: Vec<String> = if exclude.is_empty() { cfg.exclude.clone().unwrap_or_default() } else { exclude };
                    if !leave_out.is_empty() {
                        let off = cfg.apply_exclude(&leave_out);
                        println!("import: excluding {} — {} type(s) switched off", leave_out.join(","), off.len());
                        if cli.verbose {
                            for t in &off { println!("  excluded: {}", t); }
                        }
                        filtered.extend(off);
                        if !cfg.resource_types.values().any(|r| r.import) {
                            return Err(format!("import: --exclude {} leaves no enabled type — nothing would be imported", leave_out.join(",")).into());
                        }
                    }
                    let output = output.unwrap_or_else(|| PathBuf::from("discovered.satz"));
                    if into.is_some() && shape != "org" {
                        return Err("--into applies to the live shape (organizations/…, folders/…, projects/…)".into());
                    }
                    if shape == "state" {
                        let state_json = match source.as_deref() {
                            None | Some("-") => None,
                            Some(p) => Some(PathBuf::from(p)),
                        };
                        import_state(state_json, output, cfg, filtered, on_collision, customer_shortname.as_deref(), cli.verbose, &tool_config, &runtime_config)
                    } else {
                        // `--into` names an existing estate, and the delta runs
                        // exactly `adopt`'s read path — the same Cloud Asset
                        // searches and the same natural-key lookups. So it runs
                        // as the same identity: the estate's service account.
                        // Without `--into` there is no estate to be (the output
                        // is a new file), so discovery stays on the human's ADC,
                        // like `init`.
                        let into_path = into.map(|estate| estate_path(estate, &runtime_config));
                        if let Some(estate) = &into_path {
                            configure_estate_impersonation(estate, &runtime_config)?;
                        }
                        let parent = resolve_import_parent(source.as_deref(), cfg.root.as_ref()).await?;
                        match into_path {
                            Some(estate) => import_delta(&parent, estate, cfg, filtered, on_collision, cli.verbose, &tool_config, &runtime_config).await,
                            None => import_org(&parent, output, cfg, filtered, on_collision, customer_shortname.as_deref(), cli.verbose, &runtime_config).await,
                        }
                    }
                }
                other => Err(format!("unknown import shape {:?} — one of state, org, yaml, hcl", other).into()),
            }
        }
        Commands::Bootstrap { estate, dry_run, greenfield, no_default_grants } => {
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            // The quality gate: an estate may not touch an organisation while a
            // question is open. A dry run is how you look, so it warns instead.
            match crate::questions::require_complete(&config_path, &runtime_config, "bootstrap") {
                Ok(()) => {}
                Err(e) if dry_run => eprintln!("warning: {}", e),
                Err(e) => return Err(e.into()),
            }
            // The same shape for the other prerequisite: what bootstrap enables
            // before `tofu` runs is what the estate declares, so an estate whose
            // resources need an API it declares nowhere is refused here rather than
            // halfway through the first apply.
            let services = match prerequisites_report(&config_path, &tool_config, &runtime_config, PrerequisiteFindings::Report) {
                Ok(r) => {
                    if let Err(e) = require_apis_declared(&r, "bootstrap") {
                        if dry_run {
                            eprintln!("warning: {}", e);
                        } else {
                            return Err(e.into());
                        }
                    }
                    r.infra_services
                }
                // An estate that does not compile is refused HERE, before a folder, a
                // project or a bucket exists. This arm used to fall back to an empty list
                // and carry on: bootstrap then enabled only the six APIs it calls itself,
                // created everything, and the compile failure surfaced afterwards, at the
                // first resource that needed a type. A dry run creates nothing, so it warns.
                Err(e) if dry_run => {
                    eprintln!("warning: bootstrap would refuse: the estate does not compile — {}", e);
                    Vec::new()
                }
                Err(e) => {
                    return Err(format!("bootstrap refused before creating anything: the estate does not compile — {}", e).into())
                }
            };
            crate::bootstrap::bootstrap(
                config_path,
                &services,
                dry_run,
                greenfield,
                no_default_grants,
                runtime_config,
                cli.config.clone(),
                cli.validation.clone(),
                cli.verbose,
            )
            .await?;
            Ok(())
        }
        Commands::ExportOrganizationalPolicies { estate, customer_organization_id, output } => {
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            configure_estate_impersonation(&config_path, &runtime_config)?;
            crate::org_policy::export_org_policies(
                config_path,
                customer_organization_id,
                output,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::DiffOrganizationalPolicies { estate, customer_organization_id, out, format, recursive } => {
            let out = crate::out::target(out, format)?;
            // The params table and the declared policy set both come from the
            // fragment pipeline; the desired set is what the estate emits.
            let config_path = estate_path(estate, &runtime_config);
            configure_estate_impersonation(&config_path, &runtime_config)?;
            crate::org_policy::diff_org_policies(
                config_path,
                customer_organization_id,
                &out,
                format,
                recursive,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::ReportOrganizationalPolicies { estate, customer_organization_id, scope, format, out, recursive } => {
            let out = crate::out::target(out, format)?;
            // Satz-native: bootstrap needs the variable table, nothing more.
            let config_path = estate_path(estate, &runtime_config);
            configure_estate_impersonation(&config_path, &runtime_config)?;
            crate::org_policy::report_org_policies(
                config_path,
                customer_organization_id,
                scope,
                format,
                &out,
                recursive,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::Migrate { input, mode } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);

            if !input_path.exists() {
                return Err(format!("Input file not found: {}", input_path.display()).into());
            }

            reject_yaml_estate(&input_path, "migrate")?;
            let switch = mode_switch(&input_path, &runtime_config, mode)?;
            let target_mode = switch.to.clone();
            let Some(after) = &switch.after else {
                println!("Already in {} mode. No changes needed.", target_mode);
                return Ok(());
            };

            println!("Migrating from {} to {} mode...", switch.from, target_mode);
            fsx::write_edited_satz(&input_path, &switch.before, after)?;
            println!("Updated estate: {}", input_path.display());

            // Transpile
            println!("Regenerating HCL...");
            let mut cmd = std::process::Command::new(std::env::current_exe()?);
            if let Some(config_path) = &cli.config {
                cmd.arg("--config").arg(config_path);
            }
            if let Some(validation) = &cli.validation {
                cmd.arg("--validation").arg(validation);
            }
            if cli.verbose {
                cmd.arg("--verbose");
            }
            let res = cmd.arg("transpile")
                .arg(&input)
                .status()?;

            if !res.success() {
                return Err("Failed to regenerate HCL".into());
            }

            // From here the IaC service account manages the estate's Cloud Identity
            // groups, and that needs Groups Admin — a Workspace role IAM can neither grant
            // nor test. Assigned as the operator, because the account cannot give itself
            // an admin role; when the login may not, the migration goes on and says how.
            if target_mode == "cloud" {
                assign_groups_admin(&input_path, &runtime_config).await;
            }

            // Run Init with migrate-state
            println!("Running {} init -migrate-state...", tool_config.tf_tool);
            let res = std::process::Command::new(&tool_config.tf_tool)
                .current_dir(&runtime_config.hcl_dir)
                .arg("init")
                .arg("-migrate-state")
                .arg("-force-copy") // Automate the "yes" for state copy
                .status()?;

            if !res.success() {
                return Err(format!("Failed to migrate state using {}", tool_config.tf_tool).into());
            }

            println!("Migration to {} mode complete.", target_mode);
            match estate_impersonation_target(&input_path, &runtime_config)? {
                Some(sa) => println!(
                    "next: `satz whoami {input}` — must name {sa} — then `satz transpile {input} --plan`, which must plan \"No changes\" as the service account"
                ),
                None => println!("next: `satz transpile {input} --plan` — your own credentials again, against the local state"),
            }
            Ok(())
        }
        Commands::SelfUpdate { no_open_readme, check_only, skip_checksum } => {
            run_self_update(!no_open_readme, check_only, skip_checksum).await
        }
        Commands::GetPresets { force, pristine_dir } => {
            crate::presets::run_get_presets(&runtime_config.presets_dir, &runtime_config, force, pristine_dir).await
        }
        Commands::MergePresets { pristine_dir, estate, report_only, adopt } => {
            let report = crate::presets::run_merge_presets(
                &runtime_config.presets_dir, pristine_dir, estate, &tool_config, &runtime_config, report_only, &adopt,
            ).await?;
            print!("{}", crate::presets::render_merge(&report));
            if report.attention {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Silence { sub } => match sub {
            SilenceSub::List { input } => {
                let estate = input.map(|i| estate_path(PathBuf::from(&i), &runtime_config));
                silence::list(estate.as_deref(), &tool_config, &runtime_config, &config_file_path)
            }
            SilenceSub::Add { selector, reason, machine } => silence::add(&selector, &reason, machine, &config_file_path),
            SilenceSub::Remove { selector, machine } => silence::remove(&selector, machine, &config_file_path),
        },
        Commands::Fmt { paths, check, stdin } => run_fmt(&paths, check, stdin),
        Commands::Lsp => lsp::run().map_err(|e| e as Box<dyn std::error::Error>),
        Commands::Prowler { input, format } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let (manifest, included_claims, org_id) =
                compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            let now = crate::compliance::chrono_free_timestamp();
            let plan = crate::prowler::plan(&manifest, &included_claims, org_id.as_deref(), &now);
            match format {
                OutFormat::Json => println!("{}", serde_json::to_string_pretty(&plan)?),
                // stdout is the command and nothing else, so it can be piped into a
                // shell or a clipboard; what the line does not say goes to stderr.
                _ => {
                    print!("{}", crate::prowler::render(&plan));
                    eprint!("{}", crate::prowler::notes(&plan));
                }
            }
            Ok(())
        }
        Commands::Require { framework, input, format, out } => {
            let out = crate::out::target(out, format)?;
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            // This command REPORTS, it does not emit — it needs `main.tf` as a
            // value, never on disk. The stage-B block belongs in `transpile`
            // only; pasted here it once made the command silently regenerate
            // hcl/ and return without a report.
            let (manifest, included_claims, _org_id) =
                compliance_inputs(&input_path, &tool_config, &runtime_config)?;

            let report = crate::compliance::require_report(
                &framework,
                &input_path,
                &runtime_config.presets_dir,
                &included_claims,
                &manifest,
            )?;
            let text = match format {
                OutFormat::Json => serde_json::to_string_pretty(&report)?,
                _ => crate::compliance::render_require(&report),
            };
            write_report(&out, text.as_bytes(), &format!("{} control(s)", report.controls.len()))?;
            if report.gaps() {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::ReportCompliance { framework, input, format, out, prowler, no_live, checkov, fail_on } => {
            let out = crate::out::target(out, format)?;
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            configure_estate_impersonation(&input_path, &runtime_config)?;
            // Reports, never emits — see the note in `require`.
            let (manifest, included_claims, org_id) =
                compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            let checkov_report = if checkov { Some(crate::scan::run(Path::new(&runtime_config.hcl_dir))?) } else { None };

            crate::compliance::run_report_compliance(
                &framework,
                &input_path,
                &runtime_config.presets_dir,
                &included_claims,
                &manifest,
                org_id.as_deref(),
                &config_dir,
                format,
                &out,
                prowler,
                checkov_report.as_ref(),
                no_live,
                &fail_on,
            )
            .await?;
            Ok(())
        }
        Commands::Adopt { input, only, execute, import, activate } => {
            run_adopt(&input, only, execute, import, activate, &tool_config, &runtime_config).await
        }
        Commands::MapTypes { only, import_config } => {
            let cfg = load_import_config(import_config, &tool_config, &runtime_config.presets_dir)?
                .ok_or_else(|| missing_import_config(&runtime_config.presets_dir))?;
            map_types(cfg, only, cli.verbose, &runtime_config).await
        }
        Commands::AdoptOrgPolicies { input, dry_run } => {
            run_adopt(
                &input,
                vec!["google_org_policy_policy".to_string()],
                !dry_run,
                true,
                true,
                &tool_config,
                &runtime_config,
            )
            .await
        }
        Commands::Triage { framework, input, prowler, format, out, fix } => {
            let out = crate::out::target(out, format)?;
            if fix && format == OutFormat::Json {
                return Err("triage --fix renders the estate delta as prose; \
                            use --format markdown or pdf, or read the rows from --format json"
                    .into());
            }
            let input_path = if Path::new(&input).is_absolute() { PathBuf::from(&input) } else { PathBuf::from(&runtime_config.yaml_dir).join(&input) };
            let (manifest, included_claims, _org_id) = compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            crate::compliance::run_triage(&framework, &runtime_config.presets_dir, &included_claims, &manifest, &prowler, format, &out, fix)
        }
        Commands::RemediationPlan { framework, input, prowler, checkov, out_dir, merge } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let (manifest, included_claims, _org_id) = compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            let checkov_report = if checkov { Some(crate::scan::run(Path::new(&runtime_config.hcl_dir))?) } else { None };
            crate::compliance::run_remediation_dossier(
                &framework,
                &runtime_config.presets_dir,
                &included_claims,
                &manifest,
                &input_path,
                &prowler,
                checkov_report.as_ref(),
                out_dir.as_deref(),
                &config_dir,
                merge.as_deref(),
            )
        }
        Commands::DocPacks { out_dir, check } => {
            let presets = PathBuf::from(&runtime_config.presets_dir);
            let out_dir = out_dir.unwrap_or_else(|| presets.join("docs"));
            crate::doc_packs::run(&presets, &out_dir, check)
        }
        Commands::PackGraph { presets_dir, check } => {
            let presets = presets_dir.unwrap_or_else(|| PathBuf::from(&runtime_config.presets_dir));
            crate::pack_graph::run(&presets, check)
        }
        Commands::RunActions { input, check, execute, only, phase } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            reject_yaml_estate(&input_path, "run-actions")?;
            if let Some(p) = &phase {
                if p != "before-apply" && p != "after-apply" {
                    return Err(format!(
                        "--phase {}: expected before-apply or after-apply",
                        p
                    )
                    .into());
                }
            }
            // Compile first, always. An estate that does not compile is an estate
            // whose parameters cannot be trusted, and an action's arguments are
            // built from them.
            let out = pipeline_b_generate(&input_path, &tool_config, &runtime_config)?;
            let mode = if execute {
                crate::actions::Mode::Execute
            } else if check {
                crate::actions::Mode::Check
            } else {
                crate::actions::Mode::Plan
            };
            let estate_root = config_file_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let include_dirs: Vec<PathBuf> =
                runtime_config.include_dirs.iter().map(PathBuf::from).collect();
            let opts = crate::actions::RunOptions {
                mode,
                only,
                phase,
                no_actions: cli.no_actions,
                no_pack_actions: cli.no_pack_actions,
                estate_root: &estate_root,
                include_dirs: &include_dirs,
                estate_file: &input_path,
                hcl_dir: Path::new(&runtime_config.hcl_dir),
            };
            // Printed, not returned: these messages carry a remedy on its own line
            // (`chmod +x …`, the list of places a script was looked for), and the
            // default `Error: {:?}` would hand the operator an escaped one-liner.
            if let Err(e) = crate::actions::run(&out.actions, &opts) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Scan { estate } => {
            let manifest = match estate {
                Some(e) => {
                    let path = estate_path(PathBuf::from(e), &runtime_config);
                    reject_yaml_estate(&path, "scan")?;
                    Some(pipeline_b_generate(&path, &tool_config, &runtime_config)?.manifest)
                }
                None => None,
            };
            let report = crate::scan::run(Path::new(&runtime_config.hcl_dir))?;
            print!("{}", crate::scan::render(&report, manifest.as_ref()));
            if report.failed > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::ReviewPack { pack, against, format, out } => {
            let out = crate::out::target(out, format)?;
            let review = crate::review_pack::review(&pack, against.as_deref(), &tool_config, &runtime_config)?;
            let text = match format {
                OutFormat::Json => serde_json::to_string_pretty(&review)?,
                _ => crate::review_pack::render(&review, crate::out::width(&out)),
            };
            let what = format!(
                "{} finding(s) on {}",
                review.findings.len(),
                pack.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
            );
            write_report(&out, text.as_bytes(), &what)?;
            if !review.passed() {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Plan { args } => run_tf(&runtime_config, "plan", &args, api_preflight).await,
        Commands::Apply { args } => run_tf(&runtime_config, "apply", &args, api_preflight).await,
        Commands::HclInit { args } => run_tf(&runtime_config, "init", &args, api_preflight).await,
        Commands::CheckPresets { input, pristine_dir, format, out } => {
            let out = crate::out::target(out, format)?;
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let report = crate::presets::check_presets_report(
                &input_path,
                &runtime_config.presets_dir,
                &runtime_config.include_dirs,
                pristine_dir,
            )
            .await?;
            let text = match format {
                OutFormat::Json => serde_json::to_string_pretty(&report)?,
                _ => crate::presets::render_check_presets(&report),
            };
            write_report(&out, text.as_bytes(), &format!("{} clean, {} stale", report.summary.clean, report.summary.stale))?;
            if report.summary.drift_in_use {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Interview { input, create, all, accept_defaults } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            if !input_path.exists() {
                if !create {
                    return Err(format!(
                        "{}: no such estate. `satz interview {} --create` writes it first — a skeleton that uses \
                         presets/estate-core.satz, with every question open",
                        input_path.display(),
                        input
                    )
                    .into());
                }
                // creating the file is where the doubled directory becomes permanent:
                // `yaml/x.satz` lands at `yaml/yaml/x.satz` and nothing looks there again
                if let Some(bare) = redundant_yaml_dir(&input, &runtime_config.yaml_dir) {
                    return Err(format!(
                        "{}: an estate path already resolves inside {}/, so this would create {} — \
                         pass `{}` instead",
                        input,
                        runtime_config.yaml_dir,
                        input_path.display(),
                        bare
                    )
                    .into());
                }
                let stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("estate");
                if let Some(dir) = input_path.parent() {
                    crate::fsx::create_dir_all(dir)?;
                }
                let presets_dir = Path::new(&runtime_config.presets_dir);
                let graph = crate::pack_graph::read(presets_dir)?;
                crate::fsx::write_generated_satz(&input_path, &crate::template::skeleton(stem, graph.as_ref()))?;
                eprintln!("wrote {}", input_path.display());
                if graph.is_none() {
                    eprintln!("{}", crate::pack_graph::no_menu_note(presets_dir));
                }
            }
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            let mut out = std::io::stdout();
            crate::interview::run(&input_path, &runtime_config, all, accept_defaults, &mut input, &mut out)?;
            Ok(())
        }
        Commands::Packs { input, format, out } => {
            let out = crate::out::target(out, format)?;
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let report = crate::packs::report(&input_path, &runtime_config)?;
            let text = match format {
                OutFormat::Json => serde_json::to_string_pretty(&report)?,
                OutFormat::Markdown | OutFormat::Pdf => crate::packs::render_markdown(&report),
                _ => crate::packs::render_text(&report, crate::out::width(&out)),
            };
            let deploying = report.packs.iter().filter(|p| p.deploys).count();
            let what = format!("{} pack(s), {} deploying, {} finding(s)", report.packs.len(), deploying, report.findings.len());
            if format == OutFormat::Pdf {
                pdf_from_markdown(&text, &out, &what)?;
            } else {
                write_report(&out, text.as_bytes(), &what)?;
            }
            Ok(())
        }
        Commands::AddPack { input, pack, with_requirements, format } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            // Printed ahead of the error: the detail first, then the one line `main` closes with.
            let change = match crate::packs::add(&input_path, &tool_config, &runtime_config, &pack, with_requirements) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n{}\n", e);
                    return Err(format!("add-pack {}: nothing changed", pack).into());
                }
            };
            match format {
                OutFormat::Json => println!("{}", serde_json::to_string_pretty(&change)?),
                _ => print!("{}", crate::packs::render_change(&change)),
            }
            Ok(())
        }
        Commands::RemovePack { input, pack, cascade, format } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            // Printed ahead of the error: the detail first, then the one line `main` closes with.
            let change = match crate::packs::remove(&input_path, &tool_config, &runtime_config, &pack, cascade) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n{}\n", e);
                    return Err(format!("remove-pack {}: nothing changed", pack).into());
                }
            };
            match format {
                OutFormat::Json => println!("{}", serde_json::to_string_pretty(&change)?),
                _ => print!("{}", crate::packs::render_change(&change)),
            }
            Ok(())
        }
        Commands::Questions { input, format, out, unanswered } => {
            let out = crate::out::target(out, format)?;
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let mut report = crate::questions::questions_report(&input_path, &runtime_config)?;
            if unanswered {
                // The summary stays whole: it describes the estate, not the filter.
                report.questions.retain(|q| q.state == "unanswered");
            }
            let bytes = match format {
                // The workbook is the catalog a customer fills in and sends back: one
                // format among the others since it stopped being a flag of its own.
                OutFormat::Xlsx => crate::questions::xlsx(&report)?,
                OutFormat::Json => serde_json::to_string_pretty(&report)?.into_bytes(),
                OutFormat::Markdown | OutFormat::Pdf => crate::questions::render_decisions(&report).into_bytes(),
                OutFormat::Text => crate::questions::render_questions(&report).into_bytes(),
            };
            let what = if format == OutFormat::Xlsx {
                format!("{} decision(s); the `your answer` column is the customer's", report.questions.len())
            } else {
                format!("{} question(s)", report.questions.len())
            };
            if format == OutFormat::Pdf {
                pdf_from_markdown(&String::from_utf8(bytes)?, &out, &what)?;
            } else {
                write_report(&out, &bytes, &what)?;
            }
            Ok(())
        }
        Commands::Mcp { root, allow, self_gated } => {
            // A server started the old way would silently root itself at the
            // working directory instead of the estate, and answer confidently
            // about the wrong tree. Refuse and name the replacement.
            if cli.config.is_some() {
                return Err("`satz mcp` takes --root, not --config: the server holds no estate \
                            until a client opens one, so it is given a boundary rather than a \
                            configuration. Use `satz mcp --root <dir>`, and open estates inside \
                            it with the satz_open tool."
                    .into());
            }
            let ceiling = crate::mcp::Level::parse(&allow)?;
            // The server starts with no estate. A client opens one, and with it
            // that estate's own config — presets, schemas, provider version —
            // which is the only way one server can serve estates that do not
            // share a config.toml.
            let root = match root {
                Some(r) => r,
                None => std::env::current_dir()?,
            };
            let root = crate::fsx::canonicalize(&root).map_err(|e| format!("{}: {}", root.display(), e))?;
            crate::mcp::serve(root, ceiling, self_gated).await
        }
        Commands::OpenReadme => open_url(DOCS_URL),
        Commands::UpdatePrerequisites { input, report_only, format } => {
            match input {
                None => {
                    match format {
                        OutFormat::Json => println!("{}", serde_json::to_string_pretty(&crate::prerequisites::table_json())?),
                        _ => print!("{}", crate::prerequisites::render_table()),
                    }
                    Ok(())
                }
                Some(estate) => {
                    let path = estate_path(PathBuf::from(&estate), &runtime_config);
                    run_update_prerequisites(&path, report_only, format, &tool_config, &runtime_config)
                }
            }
        }
        Commands::Whoami { input, offline } => {
            // An estate changes the question from "who is the human" to "who
            // does this estate act as". A path that does not resolve says so, with
            // the form that asks the other question.
            if let Some(estate) = input {
                let named = estate.display().to_string();
                let path = estate_path(estate, &runtime_config);
                if !path.exists() {
                    return Err(format!(
                        "estate not found: {} — `satz whoami` without an estate reports the \
                         ambient credentials",
                        path.display()
                    )
                    .into());
                }
                // An estate whose params cannot be read, or whose mode the compile
                // refuses, has no answer: both refuse, and nothing is bound.
                let declared = estate_declaration(&path, named, &runtime_config)?;
                configure_estate_impersonation(&path, &runtime_config)?;
                // Online, the estate's resource types say which permissions to test.
                // An estate that does not compile still gets its identity answered.
                let probe = if offline {
                    None
                } else {
                    match iac_probe(&path, &tool_config, &runtime_config) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            eprintln!("note: permissions not tested — the estate does not compile: {}", e);
                            None
                        }
                    }
                };
                return crate::gcp::identity::whoami(offline, Some(declared), probe).await;
            }
            crate::gcp::identity::whoami(offline, None, None).await
        }
        Commands::Completion { shell, install } => {
            let using_default = shell.is_none();
            let shell = match shell {
                Some(s) => s,
                None => detect_default_shell()?,
            };
            // Mirror gcloud-switch: a bare `completion` on macOS installs straight away.
            let install = install || (using_default && cfg!(target_os = "macos"));
            run_completion(&shell, install)
        }
    }?;

    Ok(())
}

/// Satz front-end, shared by every command that takes an estate input: a .satz file
/// compiles to its generated .gen.yaml sibling (inspectable, never hand-edited) and
/// the returned path feeds the unchanged YAML pipeline.
/// Generation: satz estate -> fragments -> fold -> emit, schema-driven.
/// Returns every generated file; every command that reads an estate goes
/// through it (transpile, require, report-compliance, adopt, import --into).
struct PipelineBOut {
    main_tf: String,
    /// What `main_tf` contains, as structure (resource blocks only — the raw
    /// `hcl { … }` passthrough is text appended afterwards and is not in it,
    /// which is exactly the "opaque to the proof layer" contract). Every
    /// consumer that needs the emitted resource set reads this, never the text.
    manifest: crate::manifest::Manifest,
    providers_tf: String,
    variables_tf: String,
    tfvars: String,
    imports_tf: String,
    /// Claims declared by the estate and every pack it actually used — the
    /// compliance plane's input, produced by the same compile that produced
    /// main_tf, so witnesses and claims can never come from different reads.
    claims: Vec<satz_core::pipeline::PackClaims>,
    org_id: Option<String>,
    /// Every `google_org_policy_policy` the estate declares, as (label, body)
    /// straight off the folded IR — the same value the emitter renders into
    /// main.tf. The org-policy commands used to recover this by parsing a
    /// generated YAML twin back into a `Config`.
    org_policies: Vec<(String, serde_yaml::Value)>,
    /// The Cloud Identity customer id the estate declares ("" when absent) —
    /// the tenant adoption lists when a group lookup is refused.
    customer_id: String,
    /// Declared `action`s, arguments resolved. Nothing is emitted for them and
    /// nothing runs them here — `run-actions` is the only thing that does.
    actions: Vec<satz_core::pipeline::ResolvedAction>,
    /// What the compile found and did not refuse on — the warnings and notes the
    /// CLI printed — as data, for MCP. An error never reaches here: the compile is
    /// `Err` instead.
    findings: Vec<crate::findings::Finding>,
}

use satz_core::pipeline::ResolvedType;

/// Schema-driven resolver: tf-type facts come from the loaded provider
/// schemas; the intrinsic scopes and grant classes are the same facts
/// HoistTable / the auto-explode list encode in the walk.
pub(crate) struct EstateResolver<'a> {
    pub(crate) registry: &'a ResourceRegistry,
}
impl satz_core::pipeline::TypeResolver for EstateResolver<'_> {
    fn resolve(&self, key: &str) -> Option<ResolvedType> {
        match key {
            "terraform" | "providers" | "variables" | "include" => return None,
            _ => {}
        }
        // Existence is the schema's call. EXACT lookup only: Satz names
        // Terraform types in full, so `org_policy_policy` is not a resource
        // key here. `find_resource` deliberately falls back to a `google_`
        // prefix — that shorthand belongs to the YAML dialect, which keeps
        // it — so this path must not go through it.
        //
        // (`google_cloud_identity_group` used to be special-cased to accept
        // the bare form; it is a real schema type, so the registry answers
        // for it like any other.)
        if !self.registry.resources.contains_key(key) {
            return None;
        }
        let (class, scope) = satz_core::pipeline::type_facts(key);
        Some(ResolvedType { tf_type: key.to_string(), class, scope })
    }
}
impl satz_core::algebra::TypeTable for EstateResolver<'_> {
    fn merge_class(&self, t: &str) -> satz_core::MergeClass {
        use satz_core::pipeline::TypeResolver as _;
        self.resolve(t).map(|r| r.class).unwrap_or(satz_core::MergeClass::Entity)
    }
    fn scope(&self, t: &str) -> satz_core::Scope {
        use satz_core::pipeline::TypeResolver as _;
        self.resolve(t).map(|r| r.scope).unwrap_or(satz_core::Scope::Node)
    }
}

/// Whether a compile reports the estate's missing prerequisites as findings. A caller
/// that reports them as its own output — `whoami`'s live permission check,
/// `update-prerequisites` — compiles with `Quiet`, so the compile neither says the same
/// thing twice nor refuses on the gap the caller is there to name or close.
///
/// A parameter of one compile, never a process-wide flag: `satz mcp` compiles for many
/// calls in one process, concurrently, and a flag one call set would silence every
/// other call's findings — at validation level `error`, their refusal too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrerequisiteFindings {
    Report,
    Quiet,
}

/// Whether a compile prints its warnings and notes to stderr, where the CLI shows them.
/// `review-pack` compiles `Silent`: the findings are its report, and printing them
/// beside it would say everything twice. A refusal reads the same either way.
///
/// A parameter of one compile for the reason `PrerequisiteFindings` is: `satz mcp`
/// serves `satz_review_pack` beside every other call, concurrently, and a flag one
/// review set would stop every other compile in the process from printing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindingsOutput {
    Stderr,
    Silent,
}

/// A finding's `file`, relative to the estate's own directory.
///
/// A pack names itself as the `use` line wrote it, which is already estate-relative; the
/// estate file reaches the compile as the caller spelled it — a path relative to the
/// terminal's directory from the CLI, an absolute one from `satz mcp`. One finding,
/// named two ways, is a finding no `[[silence]]` row and no editor can match, so the
/// estate's directory decides the spelling for every reader. A path that is not under
/// it — a preset read from the library — is left as it is written.
pub(crate) fn estate_relative_file(mut f: crate::findings::Finding, dir: Option<&Path>) -> crate::findings::Finding {
    let (Some(file), Some(dir)) = (f.file.as_deref(), dir) else { return f };
    let (Ok(full), Ok(root)) = (std::fs::canonicalize(file), std::fs::canonicalize(dir)) else { return f };
    if let Ok(rel) = full.strip_prefix(&root) {
        f.file = Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        // an `hcl` block is named by where it stands, so its subject is that same path:
        // one spelling whoever compiles, or a `[[silence]]` row written from an agent's
        // JSON would not answer to the CLI's finding
        if let (crate::findings::Kind::HclPassthrough, Some(file), Some(line)) = (f.kind, &f.file, f.line) {
            f.subject = Some(format!("{}:{}", file, line));
        }
    }
    f
}

/// `transpile --format json`: the compile as the object `satz_transpile_check` and
/// `satz_transpile` return — one run, read by a person as the layout or by a program as
/// this. The compile prints nothing: the findings are in the object, silenced ones
/// marked. A refused compile is the same object with nothing emitted and its errors
/// among the findings, and the exit code says it refused.
fn transpile_as_json(
    input_path: &Path,
    input: &str,
    output: Option<&str>,
    check: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let (summary, refused) = compile_summary(input_path, input, output, check, tool_config, runtime_config)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if refused {
        std::process::exit(1);
    }
    Ok(())
}

/// The compile as data, and whether it refused. `Err` is a failure that is no verdict
/// on the estate — a schema directory that is not there.
fn compile_summary(
    input_path: &Path,
    input: &str,
    output: Option<&str>,
    check: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(crate::mcp::CompileSummary, bool), Box<dyn std::error::Error>> {
    let estate = input_path.display().to_string();
    let compiled = pipeline_b_compile(input_path, tool_config, runtime_config, PrerequisiteFindings::Report, FindingsOutput::Silent);
    match compiled {
        Ok(out) => {
            let written = if check { Vec::new() } else { write_hcl(&out, &hcl_target(output, runtime_config), input)? };
            let summary = crate::mcp::CompileSummary {
                estate,
                addresses: out.manifest.addresses().into_iter().collect(),
                written: written.iter().map(|p| p.display().to_string()).collect(),
                findings: out.findings,
            };
            Ok((summary, false))
        }
        Err(e) => match crate::findings::as_refusal(e.as_ref()) {
            Some(r) => {
                Ok((crate::mcp::CompileSummary { estate, addresses: Vec::new(), written: Vec::new(), findings: r.findings.clone() }, true))
            }
            None => Err(e),
        },
    }
}

/// Where `transpile` writes: `--output` relocates the emitted HCL, relative to hcl_dir.
fn hcl_target(output: Option<&str>, runtime_config: &ToolConfig) -> PathBuf {
    match output {
        Some(o) if Path::new(o).is_absolute() => PathBuf::from(o),
        Some(o) => PathBuf::from(&runtime_config.hcl_dir).join(o),
        None => PathBuf::from(&runtime_config.hcl_dir),
    }
}

/// The compile every command runs: its findings reported in full, and printed.
fn pipeline_b_generate(
    input_path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<PipelineBOut, Box<dyn std::error::Error>> {
    pipeline_b_compile(input_path, tool_config, runtime_config, PrerequisiteFindings::Report, FindingsOutput::Stderr)
}

fn pipeline_b_compile(
    input_path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
    prerequisites: PrerequisiteFindings,
    output: FindingsOutput,
) -> Result<PipelineBOut, Box<dyn std::error::Error>> {
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)?;

    let resolver = EstateResolver { registry: &registry };
    let src = fsx::read_to_string(input_path)?;
    let base_dir = input_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let include_dirs = runtime_config.include_dirs.clone();
    let loader = move |p: &str| -> Result<String, String> {
        let mut candidates = vec![base_dir.join(p)];
        candidates.extend(include_dirs.iter().map(|d| Path::new(d).join(p)));
        for c in candidates {
            if c.exists() {
                return std::fs::read_to_string(&c).map_err(|e| e.to_string());
            }
        }
        Err(format!("use \"{}\": file not found", p))
    };
    let graph = crate::pack_graph::shipped(Path::new(&runtime_config.presets_dir));
    let fe = match satz_core::pipeline::compile_estate(&input_path.to_string_lossy(), &src, &resolver, &loader) {
        Ok(fe) => fe,
        // An `unknown param` is most often a pack whose provider is off: the pack graph
        // names it beside the parser's error, inside the typed error so the location
        // reaches every reader.
        // The parser's error is a finding like any other — one row with its file and
        // line — so every reader gets a front-end refusal in the shape of any refusal.
        Err(e) => {
            let hinted = crate::packs::hinted(e, &graph, &input_path.to_string_lossy(), &src, &runtime_config.validation_level);
            let mut refusal = crate::findings::CompileRefusal::front_end(hinted);
            refusal.findings = refusal.findings.into_iter().map(|f| estate_relative_file(f, runtime_config.dir.as_deref())).collect();
            return Err(Box::new(refusal));
        }
    };
    let tail = compile_tail(&fe, &resolver, &registry, tool_config, &graph, &runtime_config.validation_level, input_path, &src);
    // A caller that reports the prerequisites itself is the one check that DROPS its
    // findings: it is about to say the same thing in its own output. Everything else a
    // reader asked not to see is silenced below — kept, marked and counted.
    let mut findings: Vec<crate::findings::Finding> = tail
        .findings
        .into_iter()
        .filter(|f| !(f.kind == crate::findings::Kind::Prerequisites && prerequisites == PrerequisiteFindings::Quiet))
        .map(|f| estate_relative_file(f, runtime_config.dir.as_deref()))
        .collect();
    let silences = crate::silence::in_force(tool_config);
    silences.apply(&mut findings);
    crate::silence::refuse_run_silence_of_an_error(&silences, &findings)?;
    let verdict = match output {
        FindingsOutput::Stderr => crate::findings::render(&findings),
        FindingsOutput::Silent => crate::findings::refusal(&findings),
    };
    if verdict.is_err() {
        return Err(Box::new(crate::findings::CompileRefusal { findings }));
    }
    let out = tail.out.expect("no error finding, so the emitter ran");
    let providers_tf = tail.providers_tf.expect("no error finding, so the providers were emitted");
    let folded = tail.folded;
    let org_policies: Vec<(String, serde_yaml::Value)> = folded
        .slots
        .iter()
        .filter_map(|(addr, slot)| match slot {
            satz_core::algebra::Slot::Ok(e) if addr.tf_type == "google_org_policy_policy" => {
                match &e.body {
                    satz_core::algebra::Body::Attrs(v) => Some((addr.label.clone(), v.clone())),
                    // A grant-class body is impossible for this type; skip rather
                    // than invent a shape the caller would misread.
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    let ctx = crate::emitter::EmitCtx::from_env(&fe.env);
    // computed before the struct below takes ownership of `fe`'s fields
    let descriptions = question_descriptions(&fe);
    Ok(PipelineBOut {
        actions: fe.actions,
        main_tf: append_hcl_passthrough(out.main_tf, &fe.hcl),
        manifest: out.manifest,
        providers_tf,
        variables_tf: crate::emitter::emit_variables(&fe.tfvars, &descriptions),
        tfvars: crate::emitter::emit_tfvars(&fe.tfvars),
        imports_tf: out.imports_tf,
        claims: fe.claims,
        org_policies,
        customer_id: ctx.customer_id.clone(),
        // EmitCtx defaults it to the empty string when the estate declares no
        // customer_organization_id; the compliance plane wants None there so it
        // reports "no customer-organization-id" instead of querying org "".
        org_id: Some(ctx.org_id.clone()).filter(|s| !s.is_empty()),
        findings,
    })
}

/// Two of the conflicting files are a fragment and its own dry-run twin, if they are.
///
/// The fold reports one address defined twice and names the files; when those files are
/// `X.satz` and `X-dry-run.satz` the real fault is a decision, not a composition
/// accident, and the message says so.
fn dry_run_pair(files: &[String]) -> Option<(String, String)> {
    let stem = |f: &str| f.rsplit('/').next().unwrap_or(f).to_string();
    for f in files {
        let twin = stem(f);
        let Some(base) = twin.strip_suffix("-dry-run.satz") else { continue };
        let want = format!("{}.satz", base);
        if files.iter().any(|o| stem(o) == want) {
            return Some((want, twin));
        }
    }
    None
}

/// Everything the compile checks after the front end, collected rather than
/// stopped at: the CLI renders it (`findings::render`), the language server maps
/// it to diagnostics, MCP returns it. `estate_src` is the estate's text as the
/// caller has it — the editor's buffer or the file — for the checks that read the
/// estate's own lines to say where.
pub(crate) struct Tail {
    pub folded: satz_core::algebra::Folded,
    /// `None` when an error finding stopped the compile before or at the emitter.
    pub out: Option<crate::emitter::EmitOut>,
    pub providers_tf: Option<String>,
    pub findings: Vec<crate::findings::Finding>,
}

/// `graph` is the pack graph of the estate's presets: the estate's pack lines are judged
/// against it by `crate::packs` — two excluded packs both on, a pack answered for and not
/// used, a line without its gate, a pack on while one it needs is off — and a graph that
/// is missing or unreadable is one finding instead.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_tail(
    fe: &satz_core::pipeline::FrontEnd,
    resolver: &EstateResolver,
    registry: &ResourceRegistry,
    tool_config: &ToolConfig,
    graph: &crate::pack_graph::Shipped,
    level: &str,
    estate: &Path,
    estate_src: &str,
) -> Tail {
    use crate::findings::{Finding, Kind, Severity};
    let mut f: Vec<Finding> = Vec::new();
    // The estate's pack lines against the pack graph (`crate::packs`). Two packs that
    // exclude one another are found before the fold, because the fold would refuse the
    // same thing as two disagreeing definitions of one address and name the files instead
    // of the decision; the rest are reported after the emitter.
    let label = estate.to_string_lossy().into_owned();
    let (exclusions, pack_findings) = match graph {
        crate::pack_graph::Shipped::Graph(g, dir) => crate::packs::compile_findings(g, dir, &label, estate_src, level),
        _ => (Vec::new(), Vec::new()),
    };
    f.extend(exclusions);
    if !f.is_empty() {
        return Tail { folded: satz_core::pipeline::fold_fragments(resolver, &[]), out: None, providers_tf: None, findings: f };
    }
    // After the dry-run check, which stops at any finding; before the suppressions, the
    // conflicts and the emitter, so a compile one of them stops still names a mode it
    // has no backend for.
    let mode_ok = deployment_mode_finding(&fe.env, estate, estate_src, &mut f);
    let mut folded = satz_core::pipeline::fold_fragments(resolver, &fe.fragments);
    // Subtractive override channel: estate suppressions apply before conflict
    // reporting (suppressing a conflicted address resolves the conflict).
    if let Err(e) = satz_core::pipeline::apply_suppressions(&mut folded, &fe.suppressions) {
        f.push(Finding::new(Severity::Error, Kind::Suppression, e.msg).located(e.file, e.line as u32));
        return Tail { folded, out: None, providers_tf: None, findings: f };
    }
    if conflict_findings(&folded, &mut f) {
        return Tail { folded, out: None, providers_tf: None, findings: f };
    }
    let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
    ctx.registry = Some(registry);
    let out = match crate::emitter::emit(&folded, &ctx) {
        Ok(o) => o,
        Err(e) => {
            f.push(Finding::new(Severity::Error, Kind::Emit, format!("emit: {}", e)));
            return Tail { folded, out: None, providers_tf: None, findings: f };
        }
    };
    written_reference_findings(&folded, &out.manifest, &mut f);
    missing_required_findings(&out.missing_required, level, &mut f);
    unscoped_findings(&out.unscoped, &mut f);
    wrong_shape_findings(&out.wrong_shapes, &fe.env, level, &mut f);
    prerequisite_findings(&out.manifest, &fe.env, estate, estate_src, level, &mut f);
    f.extend(pack_findings);
    f.extend(crate::notices::compile_findings(&fe.notices, &fe.env, estate, estate_src, crate::notices::Doing::Reading));
    match graph {
        crate::pack_graph::Shipped::Graph(..) => {}
        crate::pack_graph::Shipped::Missing(_) if crate::findings::at_level(level).is_none() => {}
        crate::pack_graph::Shipped::Missing(path) => f.push(Finding::new(
            Severity::Info,
            Kind::UnadoptedPack,
            format!("{} is not here, so no pack line is checked against its answer", path.display()),
        ).fix("satz get-presets")),
        crate::pack_graph::Shipped::Unreadable(why) => {
            f.push(Finding::new(Severity::Warning, Kind::UnadoptedPack, format!("{} — no pack line is checked against its answer", why)))
        }
    }
    let (provider_sources, provider_versions) = provider_maps(tool_config);
    // a mode with no backend is already the finding at its line; the providers are not
    // emitted without one
    let providers_tf = if !mode_ok {
        None
    } else {
        match crate::emitter::emit_providers(&fe.config, &folded, &fe.env, &provider_sources, &provider_versions) {
            Ok(t) => Some(t),
            Err(e) => {
                f.push(Finding::new(Severity::Error, Kind::Providers, format!("emit_providers: {}", e)));
                None
            }
        }
    };
    action_findings(&fe.actions, &mut f);
    hcl_findings(&fe.hcl, &mut f);
    Tail { folded, out: Some(out), providers_tf, findings: f }
}

/// A `deployment_mode` the emitter has no backend for is an error at the line the
/// estate binds it on — at no line when a pack binds it. `false` when it is one.
fn deployment_mode_finding(
    env: &satz_core::pipeline::Env,
    estate: &Path,
    estate_src: &str,
    f: &mut Vec<crate::findings::Finding>,
) -> bool {
    use crate::findings::{Finding, Kind, Severity};
    match crate::emitter::deployment_mode(env) {
        Ok(_) => true,
        Err(e) => {
            f.push(
                Finding::new(Severity::Error, Kind::DeploymentMode, e)
                    .maybe_at(estate.to_string_lossy().into_owned(), crate::findings::param_line(estate_src, "deployment_mode")),
            );
            false
        }
    }
}

/// The fold's conflicts, one finding per site so an editor marks every file involved;
/// a fragment and its dry-run twin get the decision named beside the files.
fn conflict_findings(folded: &satz_core::algebra::Folded, f: &mut Vec<crate::findings::Finding>) -> bool {
    use crate::findings::{Finding, Kind, Severity};
    let conflicts = folded.conflicts();
    if conflicts.is_empty() {
        return false;
    }
    let mut files: Vec<String> = Vec::new();
    for c in &conflicts {
        let sites: Vec<String> =
            c.candidates.iter().flat_map(|(_, spans)| spans.iter().map(|s| format!("{}:{}", s.file, s.line))).collect();
        for (_, spans) in &c.candidates {
            for sp in spans {
                files.push(sp.file.clone());
                f.push(
                    Finding::new(
                        Severity::Error,
                        Kind::Conflict,
                        format!("{}.{}: {} disagreeing definitions — {}", c.addr.tf_type, c.addr.label, c.candidates.len(), sites.join(", ")),
                    )
                    .in_group("composition conflicts")
                    .located(sp.file.clone(), sp.line),
                );
            }
        }
    }
    if let Some(pair) = dry_run_pair(&files) {
        f.push(
            Finding::new(
                Severity::Error,
                Kind::Conflict,
                format!(
                    "`{}` is the dry-run twin of `{}` and declares the same policies with \
                     `dry_run_spec`. A dry run REPLACES enforcement while it measures — use one or the other, \
                     never both.",
                    pair.1, pair.0
                ),
            )
            .in_group("composition conflicts"),
        );
    }
    true
}

/// A `"${{…}}"` reference to an address this estate does not emit — at the line that
/// writes it.
fn written_reference_findings(
    folded: &satz_core::algebra::Folded,
    manifest: &crate::manifest::Manifest,
    f: &mut Vec<crate::findings::Finding>,
) {
    use crate::findings::{Finding, Kind, Severity};
    let emitted = manifest.addresses();
    for r in satz_core::pipeline::written_references(folded).into_iter().filter(|r| !emitted.contains(&r.address)) {
        let (tf_type, _) = r.address.split_once('.').unwrap_or((r.address.as_str(), ""));
        let mut same: Vec<&str> = emitted
            .iter()
            .filter_map(|a| a.strip_prefix(tf_type).and_then(|rest| rest.strip_prefix('.')))
            .collect();
        same.sort();
        let hint = if same.is_empty() {
            format!("no `{}` is emitted here at all", tf_type)
        } else {
            format!("emitted `{}` labels: {}", tf_type, same.join(", "))
        };
        f.push(
            Finding::new(
                Severity::Error,
                Kind::WrittenReference,
                format!("{} writes `${{{}}}`\n  {}", r.site, r.traversal, hint),
            )
            .in_group("references to resources this estate does not emit")
            .located(r.file, r.line),
        );
    }
}

/// An emitted resource missing what its schema requires — at the declaring block,
/// at the validation level: `warn` says so, `error` refuses, `none` skips.
fn missing_required_findings(missing: &[crate::emitter::MissingRequired], level: &str, f: &mut Vec<crate::findings::Finding>) {
    use crate::findings::{Finding, Kind, Severity};
    let Some(sev) = crate::findings::at_level(level) else { return };
    for m in missing {
        let text = format!("{}: the provider requires {}", m.address, m.missing.join(", "));
        let mut finding = if sev == Severity::Error {
            Finding::new(sev, Kind::MissingRequired, text).in_group("required arguments missing")
        } else {
            Finding::new(sev, Kind::MissingRequired, format!("{} — `tofu plan` refuses it (validation_level = \"error\" refuses it here)", text))
        };
        if let Some((fl, l)) = &m.origin {
            finding = finding.located(fl.clone(), *l);
        }
        f.push(finding);
    }
}

/// A resource whose type takes a project or a folder, declared outside one and setting
/// none — a warning at the declaring block, whatever the validation level: a
/// project-scoped resource goes to the project the provider block names, which is a
/// valid plan when that is where it belongs.
fn unscoped_findings(unscoped: &[crate::emitter::Unscoped], f: &mut Vec<crate::findings::Finding>) {
    use crate::findings::{Finding, Kind, Severity};
    for u in unscoped {
        let text = match u.scope {
            "project" => format!(
                "{}: declared outside a project and sets no `project` — it goes to the project the provider block names",
                u.address
            ),
            scope => format!("{}: declared outside a {} and sets no `{}`", u.address, scope, scope),
        };
        let mut finding = Finding::new(Severity::Warning, Kind::MissingScope, text);
        if let Some((fl, l)) = &u.origin {
            finding = finding.located(fl.clone(), *l);
        }
        f.push(finding);
    }
}

/// An emitted attribute whose value the provider refuses by its shape — at the declaring
/// block, at the validation level like a missing argument. The finding names the param
/// where the declaring file binds the attribute to a bare one: the operator edits the
/// param in the estate, and the HCL line `tofu apply` would name is generated.
fn wrong_shape_findings(
    wrong: &[crate::emitter::WrongShape],
    env: &satz_core::pipeline::Env,
    level: &str,
    f: &mut Vec<crate::findings::Finding>,
) {
    use crate::findings::{Finding, Kind, Severity};
    let Some(sev) = crate::findings::at_level(level) else { return };
    for w in wrong {
        let leaf = w.attribute.rsplit('.').next().unwrap_or(&w.attribute);
        let param = w.origin.as_ref().and_then(|(file, line)| bound_param(file, *line, leaf)).filter(|p| env.contains_key(p));
        let via = match &param {
            Some(p) => format!(" — it comes from the param `{}`, which the estate binds as a {}", p, w.got),
            None => String::new(),
        };
        let text = format!("{}: `{}` is a {}, and the provider wants {}{}", w.address, w.attribute, w.got, w.expected, via);
        let mut finding = if sev == Severity::Error {
            Finding::new(sev, Kind::AttributeShape, text).in_group("attribute values the provider refuses")
        } else {
            Finding::new(sev, Kind::AttributeShape, format!("{} — `tofu plan` refuses it (validation_level = \"error\" refuses it here)", text))
        };
        if let Some((fl, l)) = &w.origin {
            finding = finding.located(fl.clone(), *l);
        }
        f.push(finding);
    }
}

/// The param a declaring block binds `attribute` to, when it is a bare one
/// (`notification_emails = access_approval_notification_emails`): the first such line
/// after the block's start. `None` when the file cannot be read or the value is anything
/// else.
fn bound_param(file: &str, line: u32, attribute: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    text.lines().skip(line.saturating_sub(1) as usize).take(200).find_map(|l| {
        let (key, value) = l.split_once('=')?;
        if key.trim() != attribute {
            return None;
        }
        let v = value.trim();
        (!v.is_empty() && v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && v.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
        .then(|| v.to_string())
    })
}

/// What the estate's resource types oblige it to declare and does not: the roles
/// its IaC service account is missing, and the APIs no `google_project_service`
/// enables. Both at the validation level — `warn` names them and the command that
/// writes them, `error` refuses, `none` skips. A type the table does not know is a
/// note, never an error: satz cannot say what it needs. The role line is the
/// estate's `svc_iac_account` param, the nearest thing the grant has to a site.
fn prerequisite_findings(
    manifest: &crate::manifest::Manifest,
    env: &satz_core::pipeline::Env,
    estate: &Path,
    estate_src: &str,
    level: &str,
    f: &mut Vec<crate::findings::Finding>,
) {
    use crate::findings::{Finding, Kind, Severity};
    let Some(sev) = crate::findings::at_level(level) else { return };
    let get = |k: &str| env.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let Some(sa) = crate::prerequisites::service_account_of(get) else { return };
    // Judging the API half needs the project the calls are billed to; an estate
    // that binds no infra project has a louder problem than this check.
    let infra = get("infra_project_name").unwrap_or_default();
    let missing_apis =
        if infra.is_empty() { Vec::new() } else { crate::prerequisites::missing_apis(manifest, &infra) };
    let update = "satz update-prerequisites <estate>";
    if !missing_apis.is_empty() {
        f.push(Finding::new(
            sev,
            Kind::Prerequisites,
            format!(
                "{} API(s) this estate's resources need are not enabled on {} — every call \
                 the provider makes is billed to the infra project, so each has to be a \
                 `project_service` entry on it:\n  {}",
                missing_apis.len(),
                infra,
                missing_apis
                    .iter()
                    .map(|a| format!("{} — needed by {}", a.api, a.reason.join(", ")))
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ),
        ).fix_in(update, estate));
    }
    let (needs, unknown) = crate::prerequisites::needs(manifest);
    let granted = crate::prerequisites::granted(manifest, &sa);
    let missing = crate::prerequisites::missing(&needs, &granted);
    if !missing.is_empty() {
        f.push(
            Finding::new(
                sev,
                Kind::Prerequisites,
                format!(
                    "the IaC service account {} lacks roles this estate's resource types need:\n  {}",
                    sa,
                    crate::prerequisites::describe(&crate::prerequisites::plan(&missing, &granted)).join("\n  ")
                ),
            )
            .fix_in(update, estate)
            .maybe_at(estate.to_string_lossy().into_owned(), crate::findings::param_line(estate_src, "svc_iac_account")),
        );
    }
    if !granted.owner() && !unknown.is_empty() {
        f.push(Finding::new(
            Severity::Info,
            Kind::Prerequisites,
            format!(
                "no role is known for {} — grant the one it needs to the IaC service account in the estate",
                unknown.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
}

/// Every declared action, at its line: `satz run-actions` will execute it, and the
/// difference between "my estate declares this" and "a pack I downloaded declares
/// this" is the whole of the trust story. A `reason` does not downgrade this to a
/// note — HCL only deploys, an action executes.
fn action_findings(actions: &[satz_core::pipeline::ResolvedAction], f: &mut Vec<crate::findings::Finding>) {
    use crate::findings::{Finding, Kind, Severity};
    for a in actions {
        f.push(
            Finding::new(
                Severity::Warning,
                Kind::Action,
                format!(
                    "`satz run-actions` will execute {}{}\n  reason: {}",
                    a.run,
                    if a.from_pack { " — a pack declares it" } else { "" },
                    a.reason
                ),
            )
            .about(a.name.clone())
            .located(a.file.clone(), a.line as u32),
        );
    }
    if actions.iter().any(|a| a.from_pack) {
        f.push(Finding::new(
            Severity::Info,
            Kind::Action,
            "--no-pack-actions ignores pack-declared actions, --no-actions disables all execution, \
             `satz silence add action --reason \"…\"` leaves these findings out of the output.",
        ));
    }
}

/// Every raw `hcl { … }` block, at its line: emitted verbatim, opaque to the
/// compliance plane — a warning until `hcl trust` says it was reviewed, a note after.
fn hcl_findings(blocks: &[satz_core::pipeline::HclPassthrough], f: &mut Vec<crate::findings::Finding>) {
    use crate::findings::{Finding, Kind, Severity};
    for b in blocks {
        let lines = dedent_hcl(&b.body).lines().count();
        let finding = match &b.trust {
            Some(reason) => Finding::new(
                Severity::Info,
                Kind::HclPassthrough,
                format!("raw HCL passthrough ({} lines) — trusted: {}", lines, reason),
            ),
            None => Finding::new(
                Severity::Warning,
                Kind::HclPassthrough,
                format!(
                    "raw HCL passthrough ({} lines) emitted verbatim — opaque to the compliance plane; no claim can cover it. Add `hcl trust \"<reason>\" {{ … }}` once reviewed.",
                    lines
                ),
            ),
        };
        // one block, named by where it stands: two blocks in one file are two findings
        // and two silences, and moving one is a new block to review
        f.push(finding.about(format!("{}:{}", b.file, b.line)).located(b.file.clone(), b.line as u32));
    }
}

/// `migrate --mode cloud`: give the estate's IaC service account Groups Admin when the
/// regenerated HCL manages Cloud Identity groups. Never fails the migration: the state
/// move is independent of it, and every outcome is printed with what to do.
async fn assign_groups_admin(estate: &Path, runtime_config: &ToolConfig) {
    let main_tf = Path::new(&runtime_config.hcl_dir).join("main.tf");
    let manages_groups = std::fs::read_to_string(&main_tf).is_ok_and(|t| {
        t.contains("resource \"google_cloud_identity_group\"") || t.contains("resource \"google_cloud_identity_group_membership\"")
    });
    if !manages_groups {
        return;
    }
    let params = match estate_param_strings(estate, runtime_config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("⚠️  Groups Admin not checked: {}", e);
            return;
        }
    };
    let get = |k: &str| params.get(k).filter(|v| !v.is_empty()).cloned();
    let Some(sa) = crate::prerequisites::service_account_of(get) else {
        eprintln!("⚠️  Groups Admin not checked: the estate names no IaC service account (svc_iac_account, infra_project_name)");
        return;
    };
    let customer = get("customer_id").unwrap_or_else(|| "my_customer".to_string());
    println!("Checking Groups Admin for {} (the groups are its to manage from here)...", sa);
    match crate::gcp::workspace::groups_admin(&customer, &sa, true).await {
        crate::gcp::workspace::GroupsAdmin::Held => println!("Groups Admin: {} holds it", sa),
        crate::gcp::workspace::GroupsAdmin::Assigned => println!("Groups Admin: assigned to {}", sa),
        crate::gcp::workspace::GroupsAdmin::NotDone(why) => {
            eprintln!("⚠️  Groups Admin not assigned — until it is, an apply that touches a group is refused:\n  {}", why)
        }
    }
}

/// What `whoami <estate>` tests live: the estate's needs, and its organization,
/// infra project and billing account to test them on.
pub(crate) fn iac_probe(
    path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<crate::prerequisites::Probe, Box<dyn std::error::Error>> {
    // whoami reports the permissions live; the compile's own note would repeat them
    let out = pipeline_b_compile(path, tool_config, runtime_config, PrerequisiteFindings::Quiet, FindingsOutput::Stderr)?;
    let params = estate_param_strings(path, runtime_config)?;
    let get = |k: &str| params.get(k).filter(|v| !v.is_empty()).cloned();
    Ok(crate::prerequisites::Probe {
        needs: crate::prerequisites::needs(&out.manifest).0,
        scope_root: get("customer_organization_id").map(|o| crate::org_policy::normalize_parent(&o)),
        project: get("infra_project_name").map(|p| format!("projects/{}", p)),
        billing_account: get("billing_account_infra"),
        service_account: crate::prerequisites::service_account_of(get),
        customer: get("customer_id"),
    })
}

/// An estate may not be applied while a resource type it emits is served by an API
/// nothing enables: the apply runs until it reaches that resource and fails there,
/// leaving half an estate. The same two-speed rule as the unanswered-question gate —
/// `apply` and `bootstrap` refuse, `plan` and `--dry-run` warn.
fn require_apis_declared(report: &PrerequisitesReport, action: &str) -> Result<(), String> {
    if report.missing_apis.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} refused: {} API(s) this estate's resources need are not enabled on {} — {}. \
         `satz update-prerequisites {}` writes them into the estate.",
        action,
        report.missing_apis.len(),
        report.infra_project,
        report.missing_apis.iter().map(|a| a.api.as_str()).collect::<Vec<_>>().join(", "),
        Path::new(&report.estate).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
    ))
}

/// What `update-prerequisites <estate>` reports: both halves of what the estate's
/// own resource types oblige it to declare.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PrerequisitesReport {
    pub estate: String,
    pub service_account: String,
    /// the project every provider call is billed to, so the project the APIs are
    /// judged on; empty when the estate binds none
    pub infra_project: String,
    /// every API the emitted types need, each marked declared or not
    pub apis: Vec<crate::prerequisites::ApiNeed>,
    /// the APIs the infra project does not enable — what the write adds
    pub missing_apis: Vec<crate::prerequisites::ApiNeed>,
    /// every service the infra project declares, whatever needs it: what
    /// `bootstrap` enables before `tofu` runs
    pub infra_services: Vec<String>,
    /// the `gcloud services enable` line for `missing_apis` — what an operator
    /// who applies with `tofu` directly runs, since writing the declaration into
    /// the estate does not switch anything on. Empty when nothing is missing.
    /// `satz plan` and `satz apply` do it themselves.
    pub enable_missing_apis: String,
    pub granted: crate::prerequisites::Granted,
    pub needs: Vec<crate::prerequisites::Need>,
    pub missing: Vec<crate::prerequisites::Need>,
    /// the roles `--execute` writes for `missing`
    pub write: Vec<crate::prerequisites::Pick>,
    /// emitted types the table has no entry for
    pub unknown_types: Vec<String>,
}

/// The estate's params as strings, snake_case — what `{param}` interpolation reads.
fn estate_param_strings(path: &Path, runtime_config: &ToolConfig) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    Ok(satz_estate_params(path, &runtime_config.include_dirs)?
        .into_iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.replace('-', "_"), s.to_string())))
        .collect())
}

/// `prerequisites` is what the compile behind the report does with the same gap:
/// `update-prerequisites` (the command and its MCP tool) passes `Quiet`, since the
/// report IS its output; every other caller keeps the compile's findings.
pub(crate) fn prerequisites_report(
    path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
    prerequisites: PrerequisiteFindings,
) -> Result<PrerequisitesReport, Box<dyn std::error::Error>> {
    let out = pipeline_b_compile(path, tool_config, runtime_config, prerequisites, FindingsOutput::Stderr)?;
    let params = estate_param_strings(path, runtime_config)?;
    let sa = crate::prerequisites::service_account_of(|k| params.get(k).cloned()).ok_or_else(|| {
        format!(
            "{}: the estate names no IaC service account (svc_iac_account and infra_project_name)",
            path.display()
        )
    })?;
    let (needs, unknown) = crate::prerequisites::needs(&out.manifest);
    let granted = crate::prerequisites::granted(&out.manifest, &sa);
    let missing = crate::prerequisites::missing(&needs, &granted);
    let infra = params.get("infra_project_name").cloned().unwrap_or_default();
    let apis =
        if infra.is_empty() { Vec::new() } else { crate::prerequisites::apis(&out.manifest, &infra) };
    let missing_apis: Vec<crate::prerequisites::ApiNeed> =
        apis.iter().filter(|a| !a.declared).cloned().collect();
    Ok(PrerequisitesReport {
        estate: path.display().to_string(),
        service_account: sa,
        enable_missing_apis: if missing_apis.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = missing_apis.iter().map(|a| a.api.clone()).collect();
            crate::prerequisites::enable_command(&infra, &ids)
        },
        missing_apis,
        apis,
        infra_services: if infra.is_empty() {
            Vec::new()
        } else {
            crate::prerequisites::declared_apis(&out.manifest, &infra)
        },
        infra_project: infra,
        unknown_types: if granted.owner() { Vec::new() } else { unknown.into_iter().collect() },
        write: crate::prerequisites::plan(&missing, &granted),
        granted,
        needs,
        missing,
    })
}

fn render_prerequisites(r: &PrerequisitesReport) -> String {
    let mut out = format!("IaC service account: {}\n", r.service_account);
    out.push_str(&format!(
        "the estate grants it {} role(s) at the organization and {} on the billing account; satz's reads and its resource types need {} permission(s)\n",
        r.granted.organization.len(),
        r.granted.billing_account.len(),
        r.needs.iter().filter(|n| n.permission.is_some()).count()
    ));
    if r.granted.owner() {
        out.push_str("roles/owner at the organization meets every organization and project need\n");
    }
    if r.missing.is_empty() {
        out.push_str("missing: none\n");
    } else {
        out.push_str("missing:\n");
        for l in crate::prerequisites::describe(&r.write) {
            out.push_str(&format!("  {}\n", l));
        }
    }
    let workspace: std::collections::BTreeSet<String> = r
        .needs
        .iter()
        .filter(|n| n.scope == crate::prerequisites::Scope::Workspace)
        .flat_map(|n| n.reason.iter().map(move |t| format!("{} — {}", t, n.roles[0])))
        .collect();
    for w in workspace {
        out.push_str(&format!("not checked: {} (not an IAM role)\n", w));
    }
    if !r.unknown_types.is_empty() {
        out.push_str(&format!(
            "no prerequisite known for: {} — grant the role it needs, and enable its API, in the estate\n",
            r.unknown_types.join(", ")
        ));
    }
    // the other half: the APIs, on the project every call is billed to
    if r.infra_project.is_empty() {
        out.push_str("APIs: not checked — the estate binds no infra_project_name\n");
    } else {
        out.push_str(&format!(
            "infrastructure project: {} — {} of {} API(s) its resource types need are enabled there\n",
            r.infra_project,
            r.apis.len() - r.missing_apis.len(),
            r.apis.len()
        ));
        if r.missing_apis.is_empty() {
            out.push_str("missing APIs: none\n");
        } else {
            out.push_str("missing APIs:\n");
            for a in &r.missing_apis {
                out.push_str(&format!("  {} — for {}\n", a.api, a.reason.join(", ")));
            }
            // Declaring an API is not enabling it. `satz plan` and `satz apply`
            // switch on what the estate declares before the tool refreshes; an
            // apply run with `tofu` directly needs this line first.
            out.push_str(&format!(
                "declared, not enabled — `satz plan` and `satz apply` enable them; for a bare \
                 tofu run:\n  {}\n",
                r.enable_missing_apis
            ));
        }
    }
    out
}

/// `update-prerequisites <estate>`: both halves reported, and — unless
/// `--report-only` — written into the estate file. The edit proves itself: the
/// estate compiles and nothing is missing afterwards, or the file is restored.
fn run_update_prerequisites(
    path: &Path,
    report_only: bool,
    format: OutFormat,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let report = prerequisites_report(path, tool_config, runtime_config, PrerequisiteFindings::Quiet)?;
    let emit = |r: &PrerequisitesReport| -> Result<(), Box<dyn std::error::Error>> {
        match format {
            OutFormat::Json => println!("{}", serde_json::to_string_pretty(r)?),
            _ => print!("{}", render_prerequisites(r)),
        }
        Ok(())
    };
    // counted as DECLARATIONS the estate is missing — the roles the write would add
    // and the APIs beside them — not as permissions, of which one role can carry many
    let (org, bill) = crate::prerequisites::to_write(&report.write);
    let gap = org.len() + bill.len() + report.missing_apis.len();
    if gap == 0 {
        return emit(&report);
    }
    if report_only {
        emit(&report)?;
        // the escape hatch: an engagement where satz may not grant those roles
        // itself needs the list, not an edit
        return Err(format!(
            "{} prerequisite(s) missing — `satz update-prerequisites {}` writes them into the estate",
            gap,
            path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
        )
        .into());
    }
    let (written, after) = prerequisites_write(path, &report, tool_config, runtime_config, PrerequisiteFindings::Quiet)?;
    for w in &written {
        println!("wrote {} → {}", w, path.display());
    }
    emit(&after)
}

/// Write the missing roles AND the missing APIs into the estate, then re-check:
/// what was written, and the report the edited estate yields. A gap that survives
/// the write, or an estate that no longer compiles, restores the file and is an
/// error — the estate is never left half-edited, which is why both halves are
/// written before anything is verified and one restore covers both. Prints
/// nothing, so the MCP tool shares it. `prerequisites` is passed on to the
/// re-check's compile, as `prerequisites_report` takes it.
pub(crate) fn prerequisites_write(
    path: &Path,
    report: &PrerequisitesReport,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
    prerequisites: PrerequisiteFindings,
) -> Result<(Vec<String>, PrerequisitesReport), Box<dyn std::error::Error>> {
    let (org, bill) = crate::prerequisites::to_write(&report.write);
    let params = estate_param_strings(path, runtime_config)?;
    let before = fsx::read_to_string(path)?;
    let restore = |why: String| -> Box<dyn std::error::Error> {
        match crate::fsx::write_verbatim(path, &before) {
            Ok(()) => format!("{} — {} restored", why, path.display()).into(),
            Err(e) => format!("{} — and restoring {} failed: {}", why, path.display(), e).into(),
        }
    };
    let mut written = crate::prerequisites::write_grants(path, &params, &report.service_account, &org, &bill)?;
    let apis: std::collections::BTreeSet<String> = report.missing_apis.iter().map(|a| a.api.clone()).collect();
    match crate::prerequisites::write_apis(path, &params, &apis) {
        Ok(lines) => written.extend(lines),
        // the roles may already be on disk, so a refusal here restores the file
        // rather than leaving the estate half-written
        Err(e) => return Err(restore(e)),
    }
    match prerequisites_report(path, tool_config, runtime_config, prerequisites) {
        Ok(after) if after.missing.is_empty() && after.missing_apis.is_empty() => Ok((written, after)),
        Ok(after) => Err(restore(format!(
            "the declarations were written and {} prerequisite(s) are still missing",
            after.missing.len() + after.missing_apis.len()
        ))),
        Err(e) => Err(restore(format!("the declarations were written and the estate no longer compiles: {}", e))),
    }
}

/// Write a compile's HCL into `dir`: `main.tf` with its provenance line, the other
/// files when they are non-empty, and no `imports.tf` left from an earlier run.
/// Returns the files written. `estate` is what the provenance line names. The CLI
/// and the MCP `satz_transpile` tool both write through here.
pub(crate) fn write_hcl(out: &PipelineBOut, dir: &Path, estate: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    if !dir.exists() {
        fsx::create_dir_all(dir)?;
    }
    let imports_path = dir.join("imports.tf");
    if imports_path.exists() {
        fsx::remove_file(&imports_path)?;
    }
    // A provenance line, added at WRITE time rather than by the emitter. The
    // emission itself must not depend on the binary version, or every release
    // would move every corpus snapshot — and the corpus exists to show when the
    // OUTPUT changed, not when the version did.
    //
    // What it is for: telling at a glance which estates across a fleet were last
    // emitted by an old satz —
    //   grep -h "Generated by satz" ~/estates/*/hcl/main.tf | sort -u
    //
    // What it is NOT: evidence that an estate still compiles, or that it still
    // emits the same resources. A language tightening can break an estate whose
    // stamp looks current, and leave one alone whose stamp is ancient. Only
    // re-transpiling and comparing answers that. The stamp says where to look
    // first; the fleet check says what is actually true.
    //
    // A comment rather than a value: a block-level comparison strips comment-only
    // lines, so this never reads as a delta.
    let stamped = format!(
        "# Generated by satz v{} — do not edit; re-emit from {}.\n\n{}",
        env!("CARGO_PKG_VERSION"),
        estate,
        out.main_tf
    );
    let mut written = Vec::new();
    for (name, content) in [
        ("main.tf", stamped.as_str()),
        ("providers.tf", out.providers_tf.as_str()),
        ("variables.tf", out.variables_tf.as_str()),
        ("terraform.tfvars", out.tfvars.as_str()),
        ("imports.tf", out.imports_tf.as_str()),
    ] {
        if content.trim().is_empty() {
            continue;
        }
        let p = dir.join(name);
        fsx::write(&p, content)?;
        written.push(p);
    }
    Ok(written)
}

/// Resources the provider will refuse for a missing required argument or block,
/// reported at the validation level: `error` refuses the compile, `warn` (the
/// default) prints one warning per resource, `none` says nothing.
/// `plan -x --config <dir>` puts `--config` inside the pass-through args, where
/// clap never sees it. Detect that and print the command that would have worked.
fn misplaced_config_hint(cmd: &Commands) -> Option<String> {
    let (sub, args) = match cmd {
        Commands::Plan { args } => ("plan", args),
        Commands::Apply { args } => ("apply", args),
        Commands::HclInit { args } => ("hcl-init", args),
        _ => return None,
    };
    let i = args.iter().position(|a| a == "--config" || a.starts_with("--config="))?;
    let mut rest = args.clone();
    let cfg = if args[i].starts_with("--config=") {
        rest.remove(i)
    } else {
        let flag = rest.remove(i);
        let value = if i < rest.len() { rest.remove(i) } else { String::new() };
        format!("{} {}", flag, value).trim_end().to_string()
    };
    Some(
        format!(
            "--config must come BEFORE the arguments passed through to the tool: everything after \
             `{}` is handed over verbatim, so a `--config` there never reaches satz.\n  try: \
             satz {} {} {}",
            sub, sub, cfg, rest.join(" ")
        )
        .trim_end()
        .to_string(),
    )
}

/// Run the configured Terraform tool in the estate's hcl dir.
///
/// A thin wrapper: the point is not to reimplement `plan`/`apply` but to make
/// them location-independent like every other command, so
/// `satz apply --config <estate>` works from anywhere. stdio is inherited, so
/// apply's approval prompt and the usual coloured output behave normally, and the
/// tool's own exit code is propagated — a failed plan must fail the caller.
///
/// Two things it adds, both before the tool starts and both only for `plan` and
/// `apply`:
///
/// - The APIs. Every API the estate declares on the project the provider bills
///   to is switched on if it is off (`api_preflight`, ADR 0036), as the estate's
///   IaC service account. `init` does not: it configures a backend and talks to
///   no Google API of the estate's.
/// - The replacements. An org policy that the state holds with rules and the
///   estate now declares reset is replaced (`reset_replacements`, ADR 0011), and
///   satz says so. The emitted `main.tf` names the same `-replace` in a comment
///   above each policy declared reset, for an apply that does not run through
///   satz.
///
/// It does NOT transpile first: `hcl/` is generated, but coupling generation to
/// the deploy step would change what `plan` means and hide a diff the operator
/// should see. Transpile, look, then plan.
async fn run_tf(
    runtime_config: &ToolConfig,
    subcommand: &str,
    args: &[String],
    api_preflight: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let hcl_dir = Path::new(&runtime_config.hcl_dir);
    if !hcl_dir.is_dir() {
        return Err(format!(
            "hcl dir '{}' does not exist — run `transpile` first (or check hcl_dir in the config)",
            hcl_dir.display()
        )
        .into());
    }
    if subcommand != "init" && !hcl_dir.join(".terraform").exists() {
        return Err(format!(
            "'{}' is not initialised — run `satz hcl-init` (same --config) first",
            hcl_dir.display()
        )
        .into());
    }
    let mut args = args.to_vec();
    if (subcommand == "plan" || subcommand == "apply") && api_preflight {
        enable_declared_apis(hcl_dir).await?;
    }
    if subcommand == "plan" || subcommand == "apply" {
        for address in reset_replacements_for(runtime_config, hcl_dir, &args)? {
            eprintln!(
                "note: {} — the state holds it with rules and the estate declares it reset; \
                 replacing it (-replace), because the API refuses to switch a policy with rules to reset in place",
                address
            );
            args.push(format!("-replace={}", address));
        }
    }
    eprintln!("{} {} (in {})", runtime_config.tf_tool, subcommand, hcl_dir.display());
    let status = std::process::Command::new(&runtime_config.tf_tool)
        .current_dir(hcl_dir)
        .arg(subcommand)
        .args(&args)
        .status()
        .map_err(|e| format!("could not run '{}': {}", runtime_config.tf_tool, e))?;
    match status.code() {
        Some(0) => Ok(()),
        // Propagate rather than wrap: `plan -detailed-exitcode` uses 2 to mean
        // "changes present", which a caller may be keying on.
        Some(code) => std::process::exit(code),
        None => Err(format!("{} {} was terminated by a signal", runtime_config.tf_tool, subcommand).into()),
    }
}

/// The org policies `plan` and `apply` replace instead of updating in place: each
/// one the state holds with rules while the configuration declares it `reset`.
/// The provider updates such a policy by sending the rules it holds together with
/// `reset = true`, and the API refuses the pair (`Cannot set PolicyRules if reset
/// is true`). That is the state after `adopt` moved a legacy twin onto its
/// `-superseded` address. A replace deletes the policy and creates it reset.
fn reset_replacements(manifest: &crate::manifest::Manifest, state: &crate::bootstrap::StateIndex) -> Vec<String> {
    manifest
        .of_type("google_org_policy_policy")
        .filter(|r| r.reset && state.holds_rules(&r.address()))
        .map(|r| r.address())
        .collect()
}

/// Flags of `tofu plan`/`apply` that take their value as the next argument when
/// written without `=`. Every other flag is a switch.
const TF_VALUE_FLAGS: &[&str] =
    &["-var", "-var-file", "-target", "-exclude", "-replace", "-lock-timeout", "-parallelism", "-state", "-state-out", "-backup", "-out", "-generate-config-out"];

/// The addresses these arguments already replace, or `None` when satz must not add
/// `-replace`: a saved plan (a positional argument) cannot take one, and a destroy
/// or a refresh-only run replaces nothing.
fn replace_args(args: &[String]) -> Option<std::collections::BTreeSet<String>> {
    let mut replaced = std::collections::BTreeSet::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let bare = a.trim_start_matches('-');
        if !a.starts_with('-') {
            return None;
        }
        let (flag, value) = match a.split_once('=') {
            Some((f, v)) => (format!("-{}", f.trim_start_matches('-')), Some(v.to_string())),
            None => (format!("-{}", bare), None),
        };
        if flag == "-destroy" || flag == "-refresh-only" {
            return None;
        }
        let value = match value {
            Some(v) => Some(v),
            None if TF_VALUE_FLAGS.contains(&flag.as_str()) => it.next().cloned(),
            None => None,
        };
        if flag == "-replace" {
            replaced.extend(value);
        }
    }
    Some(replaced)
}

/// `reset_replacements` for the estate in `hcl_dir`, minus what the arguments
/// already replace. The state is read only when the emitted configuration declares
/// a reset policy at all.
fn reset_replacements_for(
    runtime_config: &ToolConfig,
    hcl_dir: &Path,
    args: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let Some(already) = replace_args(args) else {
        return Ok(Vec::new());
    };
    let main_tf = hcl_dir.join("main.tf");
    let text = match std::fs::read_to_string(&main_tf) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {}", main_tf.display(), e).into()),
    };
    let body = hcl::parse(&text).map_err(|e| format!("{}: {}", main_tf.display(), e))?;
    let manifest = crate::manifest::Manifest::from_blocks(body.blocks());
    if !manifest.of_type("google_org_policy_policy").any(|r| r.reset) {
        return Ok(Vec::new());
    }
    let state = crate::bootstrap::state_index(&runtime_config.tf_tool, hcl_dir)
        .map_err(|e| format!("reading the state for org policies that must be replaced: {}", e))?;
    Ok(reset_replacements(&manifest, &state).into_iter().filter(|a| !already.contains(a)).collect())
}

/// What the emitted provider configuration says the next `tofu` run will do:
/// which project it bills every call to, and which identity it acts as.
///
/// Read from `providers.tf` rather than derived from the estate again, because
/// this is the file the tool itself reads — `plan` and `apply` are given no
/// estate, they are given a directory, and satz must act as what is in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EmittedProvider {
    /// `billing_project` of the default `google` provider — the project
    /// `user_project_override` bills every call to, so the project the APIs must
    /// be on. `None` when the estate declares no literal one.
    billing_project: Option<String>,
    /// `impersonate_service_account` — `None` in local mode, where `tofu` runs
    /// as the credentials themselves.
    impersonate: Option<String>,
}

/// The default provider's billing project and identity, out of `providers.tf`.
///
/// The DEFAULT provider is the block labelled `google` whose `alias` is
/// `"google"`: the one every emitted resource without a provider of its own
/// uses. The per-project aliases carry the same two values in cloud mode and
/// their own project in local mode, and taking whichever came first would make
/// the answer depend on emission order.
fn emitted_provider(hcl_dir: &Path) -> Result<EmittedProvider, String> {
    let path = hcl_dir.join("providers.tf");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(EmittedProvider::default()),
        Err(e) => return Err(format!("{}: {}", path.display(), e)),
    };
    let body = hcl::parse(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(default_google_provider(body.blocks()))
}

/// [`emitted_provider`]'s reading, over parsed blocks.
fn default_google_provider<'a>(blocks: impl IntoIterator<Item = &'a hcl::Block>) -> EmittedProvider {
    let string_attr = |b: &hcl::Block, key: &str| -> Option<String> {
        b.body.attributes().find(|a| a.key() == key).and_then(|a| match &a.expr {
            hcl::Expression::String(s) => Some(s.clone()),
            _ => None,
        })
    };
    for b in blocks {
        if b.identifier() != "provider" || b.labels().first().map(|l| l.as_str()) != Some("google") {
            continue;
        }
        if string_attr(b, "alias").as_deref() != Some("google") {
            continue;
        }
        return EmittedProvider {
            billing_project: string_attr(b, "billing_project"),
            impersonate: string_attr(b, "impersonate_service_account"),
        };
    }
    EmittedProvider::default()
}

/// Bind the identity `plan` and `apply` run as: the service account the emitted
/// provider impersonates, which is what `tofu` is about to act as in the same
/// directory.
///
/// The estate commands derive it from the estate's params
/// (`configure_estate_impersonation`); these two are handed a directory and no
/// estate, so they read the artefact instead. Same rule, same identity — the
/// emitter writes that attribute from those same params.
fn configure_emitted_impersonation(provider: &EmittedProvider) -> Result<(), String> {
    crate::gcp::configure_impersonation(provider.impersonate.clone())
}

/// Before `plan` or `apply`: every API the estate declares on the project the
/// provider bills to, switched on where it is off.
///
/// Says on stderr what it found and what it changed, beside the tool's own note:
/// stdout belongs to the plan. A refusal prints the APIs and the `gcloud
/// services enable` line that does it by hand, and fails — `tofu` is never
/// started with an API off, because its refresh would stop halfway through,
/// having already reported half an estate as drifted.
async fn enable_declared_apis(hcl_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let provider = emitted_provider(hcl_dir)?;
    let Some(project) = provider.billing_project.clone() else {
        // A local-mode estate with no infrastructure project bills nothing
        // centrally; each resource's own project carries its APIs, and `tofu`
        // creates them.
        return Ok(());
    };
    let main_tf = hcl_dir.join("main.tf");
    let text = match std::fs::read_to_string(&main_tf) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {}", main_tf.display(), e).into()),
    };
    let body = hcl::parse(&text).map_err(|e| format!("{}: {}", main_tf.display(), e))?;
    let manifest = crate::manifest::Manifest::from_blocks(body.blocks());
    let declared = crate::prerequisites::declared_apis(&manifest, &project);
    if declared.is_empty() {
        return Ok(());
    }
    configure_emitted_impersonation(&provider)?;
    match crate::prerequisites::enable_declared_apis(&project, declared).await {
        Ok(done) => {
            eprint!("{}", done.render());
            Ok(())
        }
        Err(refusal) => {
            // Printed ahead of the error: the detail first, then the one line `main` closes with.
            eprint!("{}", refusal.render());
            Err(refusal.summary().into())
        }
    }
}

/// satz reads Satz estates. A `.yaml` estate is not an error the user can fix
/// by editing — it is a file in a dialect the tool no longer speaks — so say what
/// to run instead of failing somewhere deep in a YAML scanner.
fn reject_yaml_estate(input: &Path, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    if input.extension().and_then(|e| e.to_str()) == Some("satz") {
        return Ok(());
    }
    // Printed ahead of the error: the detail first, then the one line `main` closes with.
    eprintln!(
        "\n{}: {} is a YAML-dialect estate. Every command reads Satz; the dialect\n\
         exists only to be converted. Convert once — the conversion compiles the\n\
         result through the fragment pipeline and reports what it emits:\n\n    satz import {} --kind estate\n",
        what,
        input.display(),
        input.file_name().unwrap_or_default().to_string_lossy()
    );
    Err("YAML-dialect estate: convert it with `satz import <file>.yaml` first".into())
}

/// The two facts `require` and `report-compliance` need: the emitted `main.tf`
/// as a value, and the claims the estate actually pulled in.
///
/// `.satz` estates run the fragment pipeline — same compile as `transpile`, so
/// the witnesses the goal view matches against are exactly the ones that would
/// be written to disk. The emission manifest, not the rendered text, is what
/// the compliance plane reads: a witness inside a raw `hcl { … }` block is
/// therefore not a witness, as documented.
/// Where `discover-* --satz` writes: the given path inside `yaml_dir`, with the
/// legacy `.yaml` default turned into `.satz`.
/// The yaml shape of `satz import`: convert a legacy-dialect file (estate or
/// pack) to Satz and compile the result through the fragment pipeline (an
/// estate on itself, a pack in the `gate` estate), reporting what it emits.
/// A migrated estate may need a manual edit; an old `!import-include`
/// becomes `satz adopt`.
fn convert_yaml_to_satz(
    input: PathBuf,
    gate: Option<PathBuf>,
    kind: String,
    fork: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve the file: as given, else under yaml_dir, else presets_dir.
    let resolve = |p: &PathBuf| -> PathBuf {
        if p.exists() { return p.clone(); }
        let y = Path::new(&runtime_config.yaml_dir).join(p);
        if y.exists() { return y; }
        Path::new(&runtime_config.presets_dir).join(p)
    };
    let src_path = resolve(&input);
    let src = fsx::read_to_string(&src_path)?;
    let name = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("converted")
        .replace(['-', '.'], "_");
    // An include of a LIST is a value, not a pack: inline it before converting.
    // Targets resolve the way a use-path does — beside the file, then the
    // include dirs.
    let include_base = src_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let include_dirs = runtime_config.include_dirs.clone();
    let load = |p: &str| -> Option<String> {
        std::iter::once(include_base.join(p))
            .chain(include_dirs.iter().map(|d| Path::new(d).join(p)))
            .find(|c| c.is_file())
            .and_then(|c| std::fs::read_to_string(c).ok())
    };
    let (src, inlined) = satz_core::migrate::inline_sequence_includes(&src, &load)
        .map_err(|e| format!("{} ({})", e, src_path.display()))?;
    for i in &inlined {
        println!("inlined: {} — an include of a list is a value, not a pack", i);
    }
    let satz = satz_core::migrate::convert(&src, &kind, &name)
        .map_err(|e| format!("{} ({})", e, src_path.display()))?;
    // The dialect's implicit `google_` prefix is not Satz, so a verbatim
    // copy of the YAML keys would not compile. Schemas decide.
    let type_registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let satz = satz_core::migrate::normalize_type_keys(&satz, &|t: &str| type_registry.resources.contains_key(t));
    // …and point `use` at converted packs, resolved the way the compiler
    // resolves a use-path: beside the file first, then the include dirs.
    let use_base = src_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let use_dirs = runtime_config.include_dirs.clone();
    let satz = satz_core::migrate::retarget_uses(&satz, &|p: &str| {
        use_base.join(p).exists() || use_dirs.iter().any(|d| Path::new(d).join(p).exists())
    });
    // A `use` that still points at a YAML pack cannot compile; say which and
    // how to fix it rather than letting the parser report `unexpected
    // character ':'` on a line of that pack (live-run F6).
    let yaml_uses: Vec<String> = satz
        .lines()
        .filter_map(|l| l.trim().strip_prefix("use \""))
        .filter_map(|rest| rest.split('"').next())
        .filter(|p| p.ends_with(".yaml") || p.ends_with(".yml"))
        .map(String::from)
        .collect();
    if !yaml_uses.is_empty() {
        // Printed ahead of the error: the detail first, then the one line `main` closes with.
        eprintln!("\n{} still `use`s {} YAML pack(s) — convert them first, then re-run:", src_path.display(), yaml_uses.len());
        for p in &yaml_uses {
            eprintln!("    satz import {} --kind pack", p);
        }
        eprintln!();
        return Err(format!("{} YAML pack(s) still in use — convert them first", yaml_uses.len()).into());
    }
    let satz_path = if fork {
        if kind == "estate" {
            return Err("--fork applies to packs, not estates".into());
        }
        let stem = src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("converted");
        src_path.with_file_name(format!("{}.local.satz", stem))
    } else {
        src_path.with_extension("satz")
    };

    fsx::write_generated_satz(&satz_path, &satz)?;
    println!("converted {} -> {}", src_path.display(), satz_path.display());
    if satz.contains("// NEEDS ADOPTION") {
        println!("note: the source used `!import-include` — run `satz adopt` on the converted estate to import what already exists.");
    }

    // The gate (M5, 2026-08-29): the conversion must compile through the
    // pipeline that will actually read it, and the operator sees what it
    // emits. The old byte-identity proof through the legacy walk is gone
    // with the walk — a conversion may need manual edits, and says so.
    let gate_estate = match &gate {
        Some(g) => Some(resolve(g)),
        None if kind == "estate" => Some(satz_path.clone()),
        None => None,
    };
    match gate_estate {
        Some(estate) if estate.extension().is_some_and(|e| e == "satz") => {
            match pipeline_b_generate(&estate, tool_config, runtime_config) {
                Ok(out) => {
                    let n = out.manifest.resources.len();
                    let mut by_type: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
                    for r in out.manifest.resources.values() {
                        *by_type.entry(r.tf_type.as_str()).or_default() += 1;
                    }
                    println!("CONVERTED: {} compiles — {} resources emitted:", estate.display(), n);
                    for (t, c) in by_type {
                        println!("  {:4} {}", c, t);
                    }
                    println!("Review the .satz, then `satz transpile` and `tofu plan`: the plan must show no destroy for what the old estate managed.");
                    Ok(())
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&satz_path);
                    Err(format!("conversion produced Satz that does not compile (removed): {}", e).into())
                }
            }
        }
        Some(estate) => {
            println!(
                "NEEDS-REVIEW: the gate estate {} is still YAML, so the pack cannot be compiled in context — convert the estate too, then re-run with --gate <estate>.satz.",
                estate.display()
            );
            Ok(())
        }
        None => {
            // A pack on its own: it must at least parse as Satz.
            satz_core::satz::parse(&satz)
                .map_err(|e| format!("conversion produced Satz that does not parse: {} in {}", e, satz_path.display()))?;
            println!("CONVERTED: {} parses — pass --gate <estate>.satz to compile it in context.", satz_path.display());
            if fork {
                println!("fork written; repoint the estate `use` to {}.", satz_path.display());
            }
            Ok(())
        }
    }
}

/// The state shape of `satz import`: `tofu show -json` (a file, or run now).
#[allow(clippy::too_many_arguments)]
fn import_state(
    state_json: Option<PathBuf>,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    customer_shortname: Option<&str>,
    verbose: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let enabled_types = Some(cfg.resource_types.into_iter().filter(|(_, v)| v.import).map(|(k, _)| k).collect());
    println!("Reading infrastructure state...");
    let state_val: serde_json::Value = if let Some(path) = state_json {
        let content = fsx::read_to_string(&path)?;
        serde_json::from_str(&content)?
    } else {
        // Every other spawn sets this. Without it `satz import <state>` reads
        // the state of whatever directory the operator happens to stand in —
        // which is usually none, and the error names neither.
        let out = std::process::Command::new(&tool_config.tf_tool)
            .current_dir(&runtime_config.hcl_dir)
            .arg("show")
            .arg("-json")
            .output()?;
        if !out.status.success() {
            return Err(format!("Failed to run {} show -json: {}", tool_config.tf_tool, String::from_utf8_lossy(&out.stderr)).into());
        }
        serde_json::from_slice(&out.stdout)?
    };
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let discoverer = crate::discovery::Discoverer::new(state_val, Some(registry), enabled_types, filtered, on_collision);
    let mut found = discoverer.discover()?;
    let registry = discoverer.registry.as_ref().ok_or("the registry loaded above is gone")?;
    // the state shape has no ADC: what the data carries, and the flag
    let org = organization_of(&found.config, None);
    let vocab = crate::vocabulary::Vocabulary::infer(&found.config, org.as_deref(), None, customer_shortname);
    vocab.apply_billing(&mut found.config);
    write_imported(&found.config, output, None, registry, &vocab, runtime_config)?;
    crate::discovery::report_skipped(&found, &discoverer.filtered_types, verbose);
    if verbose {
        crate::discovery::Discoverer::print_summary(&found.config);
    }
    Ok(())
}

/// The live shape of `satz import`: one Cloud Asset Inventory sweep under
/// `parent` (`organizations/<n>`, `folders/<n>` or `projects/<id>`).
#[allow(clippy::too_many_arguments)]
async fn import_org(
    parent: &str,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    customer_shortname: Option<&str>,
    verbose: bool,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("import: root {}", parent);
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let org_hint = cfg.root.as_ref().and_then(|r| r.organization.clone())
        .or_else(|| parent.strip_prefix("organizations/").map(String::from));
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(&registry), on_collision).await?;
    // A folder/project root names no organization; the assets' ancestors do.
    let org_hint = org_hint.or(found.organization.clone());
    attach_billing_accounts(&mut found.config).await;
    // what the ADC states, the way `init` reads it — the sweep's
    // organization is the hint, so an identity that sees many is no ambiguity
    let live = crate::gcp::identity::live_defaults(true, true, org_hint.as_deref()).await?;
    let facts = crate::vocabulary::LiveFacts {
        customer_id: live.customer_id.clone(),
        customer_domain: Some(live.org_display_name.clone().unwrap_or_else(|| live.customer_domain.clone())),
        first_admin: Some(live.first_admin.clone()),
        billing_account: live.billing_account.clone(),
    };
    let org = organization_of(&found.config, org_hint.as_deref());
    let vocab = crate::vocabulary::Vocabulary::infer(&found.config, org.as_deref(), Some(&facts), customer_shortname);
    vocab.apply_billing(&mut found.config);
    write_imported(&found.config, output, org_hint.as_deref(), &registry, &vocab, runtime_config)?;
    crate::discovery::report_skipped(&found, &filtered, verbose);
    Ok(())
}

fn write_imported(
    config: &Config,
    output: PathBuf,
    org_hint: Option<&str>,
    registry: &ResourceRegistry,
    vocab: &crate::vocabulary::Vocabulary,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    for line in vocab.report() {
        println!("{}", line);
    }
    let text = discovered_to_satz(config, "discovered", org_hint, registry, vocab)?;
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write_generated_satz(&final_output, &text)?;
    println!("Wrote {} — review it, then `satz transpile` and `tofu plan`.", final_output.display());
    Ok(())
}

fn missing_import_config(presets_dir: &str) -> Box<dyn std::error::Error> {
    format!(
        "import configuration not found. Provide --import-config, or run `satz get-presets` so that '{}/import-config.yaml' exists, or set import_config in config.toml.",
        presets_dir
    )
    .into()
}

/// Which shape a source is, from its form alone. `--from` overrides.
fn detect_import_shape(source: Option<&str>, root: Option<&crate::config::ImportRoot>) -> Result<String, Box<dyn std::error::Error>> {
    let Some(src) = source else {
        return if root.is_some_and(|r| r.organization.is_some() || r.folder.is_some() || r.project.is_some()) {
            Ok("org".into())
        } else {
            Err("nothing to import: give a source (a state file, organizations/<n>, folders/<n>, projects/<id>, a .yaml file) or set `root` in the import config".into())
        };
    };
    if src == "-" {
        return Ok("state".into());
    }
    if src.starts_with("organizations/") || src.starts_with("folders/") || src.starts_with("projects/") {
        return Ok("org".into());
    }
    let path = Path::new(src);
    if path.is_dir() {
        return Ok("hcl".into());
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("yaml") | Some("yml") => Ok("yaml".into()),
        Some("tf") => Ok("hcl".into()),
        Some("json") | Some("tfstate") => Ok("state".into()),
        _ => Err(format!("cannot tell what {:?} is — pass --from state|org|yaml|hcl", src).into()),
    }
}

/// The CAI scope to import from: the command line's source when given, else
/// the import config's `root` (project > folder > organization). A folder by
/// `path` is resolved live, one segment at a time, never guessed.
async fn resolve_import_parent(
    source: Option<&str>,
    root: Option<&crate::config::ImportRoot>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(s) = source {
        return Ok(s.to_string());
    }
    let root = root.ok_or("no live root: pass organizations/<n>, folders/<n> or projects/<id>, or set `root` in the import config")?;
    if let Some(p) = &root.project {
        return Ok(format!("projects/{}", p.trim_start_matches("projects/")));
    }
    if let Some(f) = &root.folder {
        return match (&f.id, &f.path) {
            (Some(id), None) => Ok(format!("folders/{}", id.trim_start_matches("folders/"))),
            (None, Some(path)) => {
                let org = root.organization.as_deref().ok_or("root.folder.path needs root.organization to start from")?;
                let token = crate::gcp::access_token().await?;
                let http = reqwest::Client::new();
                let name = crate::gcp::resourcemanager::resolve_folder_path(&http, &token, org, path).await?;
                println!("import: folder path {:?} is {}", path, name);
                Ok(name)
            }
            _ => Err("root.folder needs exactly one of `id` or `path`".into()),
        };
    }
    let org = root.organization.as_deref().ok_or("root has neither organization, folder nor project")?;
    Ok(format!("organizations/{}", org.trim_start_matches("organizations/")))
}

fn satz_output_path(yaml_dir: &str, output: PathBuf) -> PathBuf {
    let output = if output.extension().and_then(|e| e.to_str()) == Some("yaml") {
        output.with_extension("satz")
    } else {
        output
    };
    if output.is_absolute() {
        output
    } else {
        PathBuf::from(yaml_dir).join(output)
    }
}

/// A discovered `Config` as a Satz estate that compiles as-is: the local
/// backend the emitter requires, `customer_organization_id` inferred from the
/// resources (every `organizations/<n>` reference or `org_id` names it) and
/// referenced wherever the number was written, the document shaped into the
/// language's own forms (`satz_core::condense`), printed by the same printer
/// the yaml import uses, and shorthand type keys (`folder`, `project`)
/// normalised to provider names.
///
/// Discovery emits plain data — no anchors, tags, includes or nulls — so no
/// dialect pre-pass is needed; that is what makes the direct route possible.
fn discovered_to_satz(
    config: &Config,
    name: &str,
    org_hint: Option<&str>,
    registry: &ResourceRegistry,
    vocab: &crate::vocabulary::Vocabulary,
) -> Result<String, Box<dyn std::error::Error>> {
    let is_type = |t: &str| registry.resources.contains_key(t);
    let mut top = match serde_yaml::to_value(config)? {
        serde_yaml::Value::Mapping(m) => m,
        other => return Err(format!("discovered config is not a mapping: {:?}", other).into()),
    };
    if !top.contains_key(serde_yaml::Value::String("terraform".into())) {
        // the scaffold `init` writes: the local backend for day 0 and, once
        // the state bucket is known, the gcs one `deployment_mode` switches to
        let mut backend: serde_yaml::Value =
            serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n")?;
        if vocab.bindings.iter().any(|b| b.name == "infra_bucket_name") {
            let mut gcs = serde_yaml::Mapping::new();
            gcs.insert("bucket".into(), satz_core::migrate::param_ref("infra_bucket_name"));
            gcs.insert("prefix".into(), serde_yaml::Value::String("hcl/state".into()));
            if let Some(b) = backend.get_mut("backend").and_then(|b| b.as_mapping_mut()) {
                b.insert("gcs".into(), serde_yaml::Value::Mapping(gcs));
            }
        }
        top.insert(serde_yaml::Value::String("terraform".into()), backend);
    }
    // Root-scoped resources (org IAM, org policies, groups) use the root
    // provider alias, which an estate declares in `providers { … }`; without
    // it `tofu plan` says "Provider configuration not present" (live-run F10).
    if !top.contains_key(serde_yaml::Value::String("providers".into())) {
        // Org-scoped APIs (Org Policy, Cloud Identity) need a quota project
        // that enables them, the way the Day-0 template uses the infra
        // project. Review it.
        let billing = quota_project(config);
        let quota = billing
            .as_deref()
            .map(|p| format!("  project: {p}\n  billing_project: {p}\n"))
            .unwrap_or_default();
        let providers: serde_yaml::Value = serde_yaml::from_str(&format!(
            "google:\n  alias: google\n  user_project_override: true\n{quota}google-beta:\n  alias: google-beta\n  user_project_override: true\n{quota}"
        ))?;
        top.insert(serde_yaml::Value::String("providers".into()), providers);
        if billing.is_none() {
            eprintln!("warning: no project among the imported resources — set `billing_project` in the estate's `providers` block by hand (org-scoped APIs need a quota project)");
        }
    }
    let org = organization_of(config, org_hint);
    let mut params = vocab.params();
    if org.is_none() {
        eprintln!("warning: no organization id found among the discovered resources — add `customer_organization_id` to `params` by hand");
    } else if !params.iter().any(|(n, _)| n == satz_core::condense::ORG_PARAM) {
        params.insert(0, (satz_core::condense::ORG_PARAM.to_string(), format!("\"{}\"", org.clone().unwrap_or_default())));
    }
    condense_document(&mut top, org.as_deref(), &vocab.substitutions(), registry);
    let header = vec![
        "Discovered estate — review before use: the hierarchy is as found, folders are labelled".to_string(),
        "by display name, every resource carries its \"import-id\", what the platform owns was".to_string(),
        "skipped and listed, and the params are bound from what the platform states — a value".to_string(),
        "marked `// inferred:` names the rule that chose it. `satz transpile`, then `tofu plan`".to_string(),
        "should show imports and no creates.".to_string(),
    ];
    let satz = satz_core::migrate::convert_value(&top, "estate", name, &params, &header)?;
    Ok(satz_core::migrate::normalize_type_keys(&satz, &is_type))
}

/// The organization a discovered estate belongs to: named by its resources
/// (every `organizations/<n>` reference or `org_id`), else the sweep's hint.
fn organization_of(config: &Config, org_hint: Option<&str>) -> Option<String> {
    let top = serde_yaml::to_value(config).ok()?;
    infer_org_id(&top).or_else(|| org_hint.map(String::from))
}

/// The shaping pass over a discovered document, answered from the provider
/// schema: which document keys are resource types (shorthand included), and
/// which nested blocks occur once.
fn condense_document(
    top: &mut serde_yaml::Mapping,
    organization: Option<&str>,
    substitutions: &[satz_core::condense::Substitution],
    registry: &ResourceRegistry,
) {
    let type_of = |k: &str| -> Option<String> {
        if registry.resources.contains_key(k) {
            return Some(k.to_string());
        }
        let prefixed = format!("google_{}", k);
        registry.resources.contains_key(&prefixed).then_some(prefixed)
    };
    let single_block = |tf_type: &str, path: &str| registry.single_block(tf_type, path);
    satz_core::condense::condense(top, &satz_core::condense::Shaping { type_of: &type_of, single_block: &single_block, organization, substitutions });
}

/// `satz map-types`: align every selected row's API schema against the
/// provider schema and write `type-map.yaml` beside the import config.
async fn map_types(cfg: ImportConfig, only: Vec<String>, verbose: bool, runtime_config: &ToolConfig) -> Result<(), Box<dyn std::error::Error>> {
    use crate::gcp::discovery_doc as dd;
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let cache = dd::cache_dir(&runtime_config.presets_dir);
    let http = reqwest::Client::new();
    let mut rows: Vec<(&String, &crate::config::ImportResourceConfig)> = cfg
        .resource_types
        .iter()
        .filter(|(t, r)| if only.is_empty() { r.import } else { only.iter().any(|o| crate::config::glob_match(o, t)) })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    if rows.is_empty() {
        return Err("no rows selected — `import: true` rows, or --only <types>".into());
    }
    let out_path = Path::new(&runtime_config.presets_dir).join("type-map.yaml");
    let mut existing: std::collections::BTreeMap<String, crate::align::TypeMap> = match fsx::read_to_string(&out_path) {
        Ok(t) => serde_yaml::from_str(&t)?,
        // no map yet: the first run writes it
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(format!("{}: {}", out_path.display(), e).into()),
    };
    let (mut mapped, mut skipped) = (0usize, Vec::new());
    for (t, row) in rows {
        if row.content_type.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("IAM_POLICY")) {
            continue; // an IAM binding has no resource schema of its own — its asset is the parent's policy
        }
        let Some(asset_type) = row.asset_type.as_deref().filter(|a| !a.starts_with("TODO")) else {
            skipped.push(format!("{}: no asset_type", t));
            continue;
        };
        let Some((service, type_name)) = dd::split_asset_type(asset_type) else {
            skipped.push(format!("{}: asset_type {} is not <service>.googleapis.com/<Type>", t, asset_type));
            continue;
        };
        let Some((_, tf)) = registry.find_resource(t) else {
            skipped.push(format!("{}: not in the provider schema", t));
            continue;
        };
        let doc = match dd::document(&http, &cache, &service).await {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{}: {}", t, e));
                continue;
            }
        };
        let schema = match row.api_schema.as_deref() {
            Some(id) => doc.get("schemas").and_then(|s| s.get(id)).map(|s| (id, s)).ok_or_else(|| format!("{}: api_schema `{}` not in the {} document", t, id, service)),
            None => dd::schema_for(&doc, &type_name).map_err(|e| format!("{}: {}", t, e)),
        };
        let (schema_id, schema) = match schema {
            Ok(x) => x,
            Err(e) => {
                skipped.push(e);
                continue;
            }
        };
        let schemas = doc.get("schemas").cloned().unwrap_or(serde_json::Value::Null);
        let tm = crate::align::align(schema, &schemas, &tf.block)?;
        let revision = doc.get("revision").and_then(|r| r.as_str()).unwrap_or("?");
        println!(
            "{:48} {}/{} rev {}: {} exact, {} mapped ({} renamed), {} API-only, {} TF-only",
            t,
            service,
            schema_id,
            revision,
            tm.exact,
            tm.map.len(),
            tm.how.values().filter(|h| *h == "renamed").count(),
            tm.unmatched.len(),
            tm.tf_only.len()
        );
        if verbose {
            for (src, dst) in &tm.map {
                println!("    {:10} {} → {}", tm.how.get(src).map(String::as_str).unwrap_or(""), src, dst);
            }
            for u in &tm.unmatched {
                println!("    API-only   {}", u);
            }
        }
        existing.insert(t.clone(), tm);
        mapped += 1;
    }
    let body = serde_yaml::to_string(&existing)?;
    let header = format!(
        "# Generated by `satz map-types` — the API→Terraform field map per resource type,\n# aligned from the API's Discovery Document and the provider schema in {}.\n# Review rows marked `renamed`; `unmatched` API fields are dropped at import.\n# Re-run after a provider bump. Do not edit by hand: put overrides in import-config.yaml.\n",
        runtime_config.schema_dir
    );
    fsx::write(&out_path, format!("{}{}", header, body))?;
    println!("\nmap-types: {} type(s) mapped → {}", mapped, out_path.display());
    for s in &skipped {
        println!("  skipped: {}", s);
    }
    Ok(())
}

/// The hcl shape: literal resource blocks become Satz resources, the rest is
/// carried verbatim inside `hcl trust` (`--wrap-all`: everything), one estate.
fn import_hcl(src: &str, output: PathBuf, wrap_all: bool, verbose: bool, runtime_config: &ToolConfig) -> Result<(), Box<dyn std::error::Error>> {
    let src_path = Path::new(src);
    let mut files: Vec<PathBuf> = if src_path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(src_path)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("tf"))
            .collect();
        v.sort();
        v
    } else {
        vec![src_path.to_path_buf()]
    };
    if files.is_empty() {
        return Err(format!("{}: no .tf files", src).into());
    }
    files.retain(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.ends_with(".tfstate")));
    let inputs: Vec<satz_hcl::Input> = files
        .iter()
        .map(|f| Ok(satz_hcl::Input { path: f.to_string_lossy().into_owned(), text: fsx::read_to_string(f)? }))
        .collect::<Result<_, std::io::Error>>()?;
    let name = src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("imported_hcl");
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let imported = satz_hcl::import(&inputs, name, wrap_all, &RegistrySchema(Some(&registry)))?;
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write_generated_satz(&final_output, &imported.satz)?;
    println!("Wrote {} — review it, then `satz transpile` and `tofu plan` against the source's state: no changes.", final_output.display());
    println!("{}", satz_hcl::summary(&imported.rows));
    for r in &imported.rows {
        match &r.action {
            satz_hcl::Action::Dropped(why) => println!("  dropped    {}:{} {} — {}", r.file, r.line, r.what, why),
            satz_hcl::Action::Expanded(n) => {
                println!("  expanded   {}:{} {} — `count` over a promoted list: {} resource(s)", r.file, r.line, r.what, n)
            }
            satz_hcl::Action::Wrapped(why) if !wrap_all || verbose => println!("  wrapped    {}:{} {} — {}", r.file, r.line, r.what, why),
            satz_hcl::Action::Promoted(what) => println!("  promoted   {}:{} {} — {}", r.file, r.line, r.what, what),
            satz_hcl::Action::Translated if verbose => println!("  translated {}:{} {}", r.file, r.line, r.what),
            _ => {}
        }
    }
    for n in &imported.notes {
        println!("  note       {}", n);
    }
    Ok(())
}

/// The provider schema, as the HCL importer asks about it. With no schemas on
/// disk nothing is a known type, so every block wraps — loudly, in the report,
/// rather than by mistranslation.
struct RegistrySchema<'a>(Option<&'a ResourceRegistry>);

impl satz_hcl::Schema for RegistrySchema<'_> {
    fn has_type(&self, tf_type: &str) -> bool {
        self.0.is_some_and(|r| r.resources.contains_key(tf_type))
    }

    fn has_attr(&self, tf_type: &str, attr: &str) -> bool {
        self.0.is_some_and(|r| {
            r.resources.get(tf_type).is_some_and(|s| s.1.block.attributes.contains_key(attr))
        })
    }

    fn required_attrs(&self, tf_type: &str) -> Vec<String> {
        self.0
            .and_then(|r| r.resources.get(tf_type))
            .map(|s| s.1.block.attributes.iter().filter(|(_, a)| a.required).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }
}

/// The live shape with `--into`: the delta against what the estate declares.
#[allow(clippy::too_many_arguments)]
async fn import_delta(
    parent: &str,
    estate: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    verbose: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::delta;
    reject_yaml_estate(&estate, "import --into")?;
    println!("import: root {} → into {}", parent, estate.display());

    // 1. what the estate already covers, by live id (adopt's resolution, dry)
    let out = pipeline_b_generate(&estate, tool_config, runtime_config)?;
    let opts = crate::adopt::Options { only: Default::default(), activate: false };
    let mut live = crate::adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = crate::adopt::resolve(&out.manifest, &cfg, &opts, &mut live).await;
    // what an earlier delta import wrote is re-derived now, not "declared"
    let resolutions: Vec<_> = resolutions.into_iter().filter(|r| !delta::from_imported_pack(&r.origin)).collect();
    let declared = delta::declared_from(&resolutions);
    println!("import: {} declared resource(s) resolved to live ids", declared.ids.len());
    if !declared.no_rule.is_empty() {
        println!(
            "import: {} declared resource(s) have no adoption rule and cannot be subtracted from the sweep — they may come back as new: {}",
            declared.no_rule.len(),
            declared.no_rule.join(", ")
        );
    }
    if !declared.blocked.is_empty() {
        let mut msg = format!("import --into: {} declared resource(s) could not be resolved to a live id; the sweep cannot be subtracted, nothing written:", declared.blocked.len());
        for (a, why) in &declared.blocked {
            msg.push_str(&format!("\n  {}: {}", a, why));
        }
        return Err(msg.into());
    }

    // 2. the sweep
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let type_names: std::collections::HashSet<String> = registry.resources.keys().cloned().collect();
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(&registry), on_collision).await?;
    attach_billing_accounts(&mut found.config).await;
    let top = match serde_yaml::to_value(&found.config)? {
        serde_yaml::Value::Mapping(m) => m,
        other => return Err(format!("discovered config is not a mapping: {:?}", other).into()),
    };
    let live_ids = delta::live_ids(&top);

    // 3. subtract
    let d = delta::subtract(top, &declared);

    // 4. packs + `use` lines
    let yaml_dir = Path::new(&runtime_config.yaml_dir);
    let mut estate_text = fsx::read_to_string(&estate)?;
    let estate_before = estate_text.clone();
    let mut written: Vec<String> = Vec::new();
    let header = |what: &str| {
        vec![
            format!("Imported from {} — what the estate did not declare {}.", parent, what),
            "Regenerated on every `satz import --into`; move entries into the estate as you adopt them —".to_string(),
            "the next run subtracts them by live id. `satz transpile`, then `tofu plan` is the check.".to_string(),
        ]
    };
    let is_type = |t: &str| type_names.contains(t);
    let top_name = delta::pack_name(parent, None);
    if d.top.is_empty() {
        // nothing left at the top level: an earlier run's pack goes, with its use
        if yaml_dir.join(&top_name).exists() {
            fsx::remove_file(yaml_dir.join(&top_name))?;
            if let Some(t) = delta::remove_use(&estate_text, &top_name) {
                estate_text = t;
            }
            println!("  removed {} (nothing left to import at the top level)", yaml_dir.join(&top_name).display());
        }
    }
    if !d.top.is_empty() {
        let name = delta::pack_name(parent, None);
        let mut top = d.top.clone();
        condense_document(&mut top, found.organization.as_deref(), &[], &registry);
        let satz = satz_core::migrate::convert_value(&top, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header("at the top level"))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write_generated_satz(yaml_dir.join(&name), &satz)?;
        if let Some(t) = delta::add_use(&estate_text, &name, None)? {
            estate_text = t;
        }
        written.push(name);
    }
    let mut hints: Vec<String> = Vec::new();
    // packs first, then the `use` lines bottom-up so earlier line numbers
    // stay valid (an insert shifts everything below it)
    let mut inserts: Vec<(u32, String)> = Vec::new();
    for (address, children) in &d.under {
        let name = delta::pack_name(parent, Some(address));
        let mut children = children.clone();
        condense_document(&mut children, found.organization.as_deref(), &[], &registry);
        let satz = satz_core::migrate::convert_value(&children, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header(&format!("under {}", address)))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write_generated_satz(yaml_dir.join(&name), &satz)?;
        let origin = declared.containers.values().find(|(a, _)| a == address).and_then(|(_, o)| o.clone());
        match origin {
            Some((file, line)) if Path::new(&file) == estate.as_path() || Path::new(&file).ends_with(&estate) => {
                inserts.push((line, name.clone()));
            }
            Some((file, line)) => hints.push(format!("{} is declared in {}:{} (not the estate) — add `use \"{}\"` inside that block by hand", address, file, line, name)),
            None => hints.push(format!("{} has no declaring line — add `use \"{}\"` inside its block by hand", address, name)),
        }
        written.push(name);
    }
    inserts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (line, name) in inserts {
        if let Some(t) = delta::add_use(&estate_text, &name, Some(line))? {
            estate_text = t;
        }
    }
    // declared containers that have no residue any more: their earlier pack goes
    for (address, _) in declared.containers.values() {
        if d.under.contains_key(address) {
            continue;
        }
        let name = delta::pack_name(parent, Some(address));
        if yaml_dir.join(&name).exists() {
            fsx::remove_file(yaml_dir.join(&name))?;
            if let Some(t) = delta::remove_use(&estate_text, &name) {
                estate_text = t;
            }
            println!("  removed {} (nothing left to import under {})", yaml_dir.join(&name).display(), address);
        }
    }
    fsx::write_edited_satz(&estate, &estate_before, &estate_text)?;

    // 5. report
    println!();
    for name in &written {
        println!("  wrote {} (+ its `use` in {})", yaml_dir.join(name).display(), estate.display());
    }
    for h in &hints {
        println!("  note: {}", h);
    }
    let mut already = d.already.clone();
    already.sort();
    already.dedup();
    println!("\nimport: {} new resource(s) written, {} already declared, {} declared but not live", d.new, already.len(), declared.not_live.len());
    if verbose {
        for (id, address) in &already {
            println!("  already: {} = {}", address, id);
        }
    }
    for a in &declared.not_live {
        println!("  declared, not live (will be created on apply): {}", a);
    }
    let unseen: Vec<&String> = declared.ids.keys().filter(|id| !live_ids.contains(*id)).collect();
    if verbose && !unseen.is_empty() {
        println!("  {} declared id(s) not among the swept assets (types the sweep did not cover, or derived ids)", unseen.len());
    }
    crate::discovery::report_skipped(&found, &filtered, verbose);
    if written.is_empty() {
        println!("import: nothing to add — the estate already declares everything the sweep found.");
    }
    Ok(())
}

/// The Resource Manager asset carries no billing link; without it an imported
/// project plans `billing_account = null` — an unlink. Ask Cloud Billing per
/// project; a project this cannot be read for is named, not guessed.
async fn attach_billing_accounts(config: &mut Config) {
    fn projects_mut(config: &mut Config) -> Vec<&mut crate::config::Project> {
        fn walk<'a>(f: &'a mut crate::config::Folder, out: &mut Vec<&'a mut crate::config::Project>) {
            if let Some(ps) = &mut f.project {
                out.extend(ps.values_mut());
            }
            if let Some(fs) = &mut f.folder {
                for sub in fs.values_mut() {
                    walk(sub, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(ps) = &mut config.project {
            out.extend(ps.values_mut());
        }
        if let Some(fs) = &mut config.folder {
            for f in fs.values_mut() {
                walk(f, &mut out);
            }
        }
        out
    }
    let projects = projects_mut(config);
    if projects.is_empty() {
        return;
    }
    let token = match crate::gcp::access_token().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: billing accounts not read ({}); set `billing_account` on each project by hand", e);
            return;
        }
    };
    let http = reqwest::Client::new();
    for p in projects {
        match crate::gcp::billing::project_billing_account(&http, &token, &p.project_id).await.map_err(String::from) {
            Ok(Some(acct)) => p.billing_account = Some(acct),
            Ok(None) => {}
            Err(e) => eprintln!(
                "warning: billing account of project {} not read ({}) — set `billing_account` by hand or `tofu plan` will unlink it",
                p.project_id,
                e.lines().next().unwrap_or("")
            ),
        }
    }
}

/// The alphabetically first project id in the discovered tree, at any depth.
/// The APIs an organization-scoped read is billed against: a quota project
/// without them makes every org policy, service list and contact read fail
/// with a 403, which is how a live import's `tofu plan` came out as 25 errors
/// on the test organization.
const QUOTA_PROJECT_APIS: [&str; 2] = ["orgpolicy.googleapis.com", "serviceusage.googleapis.com"];

/// The project the providers bill organization-scoped calls to: the first
/// (by id) that enables the APIs those calls need — the day-0 infra project
/// does — else the first project at all, said so. `None` without a project.
fn quota_project(config: &Config) -> Option<String> {
    fn services(p: &crate::config::Project) -> Vec<String> {
        p.project_service
            .iter()
            .flatten()
            .filter_map(|s| match s {
                serde_yaml::Value::String(svc) => Some(svc.clone()),
                serde_yaml::Value::Mapping(m) => m.get("service").and_then(|v| v.as_str()).map(String::from),
                _ => None,
            })
            .collect()
    }
    fn walk_folder(f: &crate::config::Folder, out: &mut Vec<(String, Vec<String>)>) {
        if let Some(ps) = &f.project {
            out.extend(ps.values().map(|p| (p.project_id.clone(), services(p))));
        }
        if let Some(fs) = &f.folder {
            for sub in fs.values() {
                walk_folder(sub, out);
            }
        }
    }
    let mut projects: Vec<(String, Vec<String>)> =
        config.project.iter().flat_map(|ps| ps.values().map(|p| (p.project_id.clone(), services(p)))).collect();
    if let Some(fs) = &config.folder {
        for f in fs.values() {
            walk_folder(f, &mut projects);
        }
    }
    projects.sort();
    let able = projects.iter().find(|(_, svcs)| QUOTA_PROJECT_APIS.iter().all(|api| svcs.iter().any(|s| s == api)));
    match (able, projects.first()) {
        (Some((id, _)), _) => {
            println!("import: providers' quota project {} — it enables {}", id, QUOTA_PROJECT_APIS.join(" and "));
            Some(id.clone())
        }
        (None, Some((id, _))) => {
            println!(
                "import: providers' quota project {} — no project enables {}; organization-scoped reads may fail with 403 until one does (set `billing_project` in `providers` by hand)",
                id,
                QUOTA_PROJECT_APIS.join(" and ")
            );
            Some(id.clone())
        }
        (None, None) => None,
    }
}

/// The first organization number the tree names: an `organizations/<n>`
/// string anywhere, or an `org_id` value. Depth-first, document order.
fn infer_org_id(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => {
            let n = s.strip_prefix("organizations/")?;
            let digits: String = n.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!digits.is_empty() && digits.len() == n.len()).then_some(digits)
        }
        serde_yaml::Value::Mapping(m) => {
            if let Some(serde_yaml::Value::String(id)) = m.get(serde_yaml::Value::String("org_id".into())) {
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                    return Some(id.clone());
                }
            }
            m.values().find_map(infer_org_id)
        }
        serde_yaml::Value::Sequence(s) => s.iter().find_map(infer_org_id),
        _ => None,
    }
}

/// The adopt dry run, computed: the estate's compile, every declared resource
/// resolved against the live organisation, the live client (activation needs it),
/// and what the state already manages. No printing — the CLI and `satz_adopt`
/// share it. The identity is the caller's: the CLI binds it for the process, the
/// MCP tool scopes it to the call.
pub(crate) struct AdoptPlan {
    pub out: PipelineBOut,
    pub resolutions: Vec<crate::adopt::Resolution>,
    pub live: crate::adopt::RealLive,
    /// What the state already manages, read ONCE and used by both halves of the
    /// command. The dry run has to know it: `--execute --import` skips those
    /// addresses, so a table that ranks them as "IMPORT" describes a run that
    /// will not happen.
    ///
    /// Unreadable is a NOTE for the dry run, never a failure — a first adopt has
    /// no state, and refusing to describe the estate because of that would be
    /// refusing the only thing a dry run is for. The import path still fails
    /// fast, because there the imports really would all fail the same way.
    pub state: Result<crate::bootstrap::StateIndex, String>,
}

pub(crate) async fn adopt_plan(
    input_path: &Path,
    only: Vec<String>,
    activate: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<AdoptPlan, Box<dyn std::error::Error>> {
    // Same compile the emitter uses, so the adopted addresses are exactly the
    // ones `apply` will act on.
    let out = pipeline_b_generate(input_path, tool_config, runtime_config)?;
    let rules = load_import_config(None, tool_config, &runtime_config.presets_dir)?.ok_or(
        "adoption rules live in <presets_dir>/import-config.yaml — run `satz get-presets` so it exists",
    )?;
    let opts = crate::adopt::Options { only: only.into_iter().collect(), activate };
    let mut live = crate::adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = crate::adopt::resolve(&out.manifest, &rules, &opts, &mut live).await;
    let state = crate::bootstrap::state_index(&runtime_config.tf_tool, Path::new(&runtime_config.hcl_dir));
    Ok(AdoptPlan { out, resolutions, live, state })
}

/// `satz adopt`: compile, resolve every declared resource against the live
/// org, report, and — only with `--execute` — write the verified ids into the
/// estate or import them into state now.
async fn run_adopt(
    input: &str,
    only: Vec<String>,
    execute: bool,
    import: bool,
    activate: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::adopt::{self, Outcome};
    let input_path = estate_path(PathBuf::from(input), runtime_config);
    reject_yaml_estate(&input_path, "adopt")?;
    configure_estate_impersonation(&input_path, runtime_config)?;
    // a run over every declared resource is the one a notice naming `satz adopt` asks for
    let whole = only.is_empty();
    let AdoptPlan { out, resolutions, mut live, state } =
        adopt_plan(&input_path, only, activate, tool_config, runtime_config).await?;
    let in_state = state.clone().unwrap_or_default();

    println!("\nadopt {} — {} resources declared\n", input_path.display(), out.manifest.resources.len());
    print!("{}", adopt::render_table(&resolutions, &in_state, &out.manifest));
    println!("\n{}", adopt::summary(&resolutions, &in_state));
    if let Err(e) = &state {
        println!(
            "\nnote: the state could not be read ({}), so nothing below is marked as already \
             managed — run `{} init` in {} for the full picture",
            e.lines().next().unwrap_or("(no output)"),
            runtime_config.tf_tool,
            runtime_config.hcl_dir
        );
    }

    // A table with a FAILED / unresolvable / ambiguous / no-rule row did not
    // answer its question: that is an error exit, not a summary count. The
    // table is above; nothing has been changed at this point.
    let unanswered = adopt::unanswered(&resolutions, &in_state);
    if unanswered > 0 {
        return Err(format!(
            "adopt: {} resolution(s) failed, unresolvable, ambiguous or without a rule — see the rows above; nothing was changed",
            unanswered
        )
        .into());
    }

    // One live object with two declarations. A move would not resolve that —
    // it would only change which of the two the next plan wants to create — so
    // the run stops and names both ends. The estate has to drop one first.
    let conflicts = adopt::move_conflicts(&resolutions, &in_state);
    if !conflicts.is_empty() {
        let mut msg =
            String::from("adopt: the estate declares both ends of a state move; nothing was changed:\n");
        for (new_address, old_address) in &conflicts {
            msg.push_str(&format!(
                "  {} is the same live object as {}, which the estate still declares\n",
                new_address, old_address
            ));
        }
        msg.push_str("  drop one of the two declarations, then re-run adopt");
        return Err(msg.into());
    }

    if !execute {
        println!(
            "\ndry run — nothing was changed. Re-run with --execute to write the verified \"import-id\"s into the estate, \
             or --execute --import to run `{} import` now (derived ids are verified by the import itself).",
            runtime_config.tf_tool
        );
        return Ok(());
    }

    if import {
        let hcl_dir = Path::new(&runtime_config.hcl_dir);
        // E04: with no "import-id" in the estate every resolvable resource
        // counts as "to import", and a re-run then issued `tofu import` for
        // addresses the state already manages (17/18 once) — noisy, slow, and
        // each a needless state write. The read above already has them.
        // A FIRST adopt is fine: an initialized empty state lists nothing and
        // errors nothing. An UNREADABLE state (uninitialized dir, changed
        // backend) means every import below would fail the same way — so this
        // fails fast with the fix instead of printing it 117 times. The dry run
        // above only noted it, because describing an estate needs no state.
        let in_state = state.map_err(|e| {
            format!(
                "could not read the state ({}) — the imports would fail the same way; run `{} init` \
                 (or `init -reconfigure` after a backend change) in {} first",
                e.lines().next().unwrap_or("(no output)"),
                runtime_config.tf_tool,
                runtime_config.hcl_dir
            )
        })?;
        // activation posts the DECLARED spec — parameterized managed
        // constraints (allowedContactDomains, allowedPolicyMembers) require
        // their `parameters` and reject a synthesized enforce-only rule
        let declared_specs: std::collections::BTreeMap<String, serde_yaml::Value> = out
            .org_policies
            .iter()
            .filter_map(|(_, body)| {
                let name = body.get("name")?.as_str()?;
                let spec = body.get("spec")?.clone();
                Some((crate::org_policy::constraint_name(name), spec))
            })
            .collect();
        let (mut activated, mut imported, mut failed) = (0usize, 0usize, 0usize);
        let (mut already_managed, mut moved) = (0usize, 0usize);
        for r in &resolutions {
            if in_state.manages(&r.address) {
                println!("  {:60} already managed in the state — skipped", r.address);
                already_managed += 1;
                continue;
            }
            // Before the outcome is read: the outcome says IMPORT, and for a
            // renamed block importing is what puts one live object in the state
            // twice. The object is already managed — only its name changed.
            if let Some(old_address) = adopt::moved_from(r, &in_state) {
                if crate::bootstrap::run_state_mv(
                    &runtime_config.tf_tool,
                    hcl_dir,
                    old_address,
                    &r.address,
                ) {
                    moved += 1;
                } else {
                    failed += 1;
                }
                continue;
            }
            let id = match &r.outcome {
                Outcome::NeedsActivation { id, .. } => {
                    let Some((parent, constraint)) = &r.org_policy else { continue };
                    println!("  {:60} activating (managed, not live)...", r.address);
                    let spec = match declared_specs
                        .get(constraint)
                        .ok_or_else(|| format!("{} declares no spec", constraint))
                        .and_then(crate::org_policy::declared_spec_to_api)
                    {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("  {:60} activation FAILED: {}", r.address, e);
                            failed += 1;
                            continue;
                        }
                    };
                    let client = live.org_policy_client().await?;
                    match client.create_policy(parent, constraint, spec).await {
                        Ok(()) => activated += 1,
                        Err(e) => {
                            eprintln!("  {:60} activation FAILED: {}", r.address, e);
                            failed += 1;
                            continue;
                        }
                    }
                    id
                }
                Outcome::Resolved { id, .. } => id,
                Outcome::OnApply => {
                    println!("  {:60} on apply — skipped (apply creates it)", r.address);
                    continue;
                }
                Outcome::ParentOnApply(why) => {
                    println!("  {:60} on apply — skipped ({})", r.address, why);
                    continue;
                }
                Outcome::AlreadyAdopted(_) | Outcome::Skipped => continue,
                other => {
                    // unanswered rows already ended the run above; this arm
                    // only exists so a new Outcome can never be skipped silently
                    println!("  {:60} skipped ({:?})", r.address, other);
                    continue;
                }
            };
            if crate::bootstrap::run_import(&runtime_config.tf_tool, hcl_dir, &r.address, id) {
                imported += 1;
            } else {
                failed += 1;
            }
        }
        println!(
            "\nadopt: {} activated, {} imported, {} moved, {} already managed (skipped), {} failed.\nnext: `satz transpile {} --plan` — no create for what was imported and no destroy for what was moved.",
            activated, imported, moved, already_managed, failed, input
        );
        if failed > 0 {
            return Err(format!("adopt: {} activation(s)/import(s)/move(s) failed — see above", failed).into());
        }
        // Every declared resource answered and nothing failed: the notices that ask for
        // this run are done, and the estate says so.
        if whole {
            let done: Vec<String> = crate::notices::open(&input_path, runtime_config)?
                .into_iter()
                .filter(|n| n.run.split_whitespace().take(2).eq(["satz", "adopt"]))
                .map(|n| n.param)
                .collect();
            crate::notices::acknowledge(&input_path, &done)?;
            for p in &done {
                println!("adopt: acknowledged the notice {} — bound {} = true in {}", p, p, input_path.display());
            }
        }
    } else {
        let (written, hints) = adopt::write_import_ids(&resolutions, Some(Path::new(&runtime_config.presets_dir)))?;
        for w in &written {
            println!("  wrote {}", w);
        }
        for h in &hints {
            println!("  note: {}", h);
        }
        let pending_activation = resolutions.iter().filter(|r| matches!(r.outcome, Outcome::NeedsActivation { .. })).count();
        if pending_activation > 0 {
            println!("  note: {} managed constraint(s) need activation — that is `--execute --import --activate`, activation cannot be written into the estate", pending_activation);
        }
        println!(
            "\nadopt: {} \"import-id\"(s) written. Run `satz transpile {}` to regenerate imports.tf, then `satz plan`.",
            written.len(),
            input
        );
    }
    Ok(())
}

type ComplianceInputs = (crate::manifest::Manifest, Vec<(String, crate::compliance::Claim)>, Option<String>);

/// The one-line description for each param a question asks about. A pack that
/// bothered to write a prompt has already written the sentence `variables.tf`
/// wants; nothing else in the pipeline had one.
fn question_descriptions(
    fe: &satz_core::pipeline::FrontEnd,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for pq in &fe.questions {
        for q in &pq.questions {
            if q.oneof {
                for o in &q.options {
                    out.entry(o.param.clone()).or_insert_with(|| o.label.clone());
                }
            } else {
                out.entry(q.subject.clone()).or_insert_with(|| q.prompt.clone());
            }
        }
    }
    out
}

fn compliance_inputs(
    input_path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<ComplianceInputs, Box<dyn std::error::Error>> {
    reject_yaml_estate(input_path, "this command")?;
    let out = pipeline_b_generate(input_path, tool_config, runtime_config)?;
    let claims = crate::compliance::claims_from_frontend(&out.claims);
    Ok((out.manifest, claims, out.org_id))
}

/// Append `hcl { … }` bodies verbatim to the generated main.tf, each under a
/// provenance header, and report them: raw HCL deploys but the compliance plane
/// cannot see inside it, so every block warns unless it states a `trust` reason.
/// Every `${…}` reference the estate writes must name a resource it emits.
/// Checked against the EMISSION MANIFEST, not the fold: a project's services
/// and an exploded grant are real emitted addresses that were never fold
/// entities, and referencing one is legitimate.
///
/// Without this a typo compiles and ships. Terraform catches most of them at
/// plan time, one cycle later and pointing at generated HCL instead of the
/// line someone wrote; the one it cannot catch is a typo that happens to name
/// a different real resource.
/// The raw blocks appended to main.tf, verbatim; what they mean for the compliance
/// plane is said by `hcl_findings`.
fn append_hcl_passthrough(mut main_tf: String, blocks: &[satz_core::pipeline::HclPassthrough]) -> String {
    for b in blocks {
        let body = dedent_hcl(&b.body);
        if !main_tf.ends_with('\n') {
            main_tf.push('\n');
        }
        main_tf.push_str(&format!(
            "\n# --- raw HCL passthrough from {}:{} ---\n# Opaque to the compliance plane: no claim covers what is written here.\n",
            b.file, b.line
        ));
        if let Some(reason) = &b.trust {
            main_tf.push_str(&format!("# trusted: {}\n", reason));
        }
        main_tf.push_str(&body);
        if !body.ends_with('\n') {
            main_tf.push('\n');
        }
    }
    main_tf
}

/// Strip the common leading indentation a passthrough body inherited from its
/// position in the .satz file, so the emitted HCL reads like hand-written HCL.
fn dedent_hcl(body: &str) -> String {
    let indent = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let out: Vec<String> = body
        .lines()
        .map(|l| if l.len() >= indent { l[indent..].to_string() } else { l.trim_start().to_string() })
        .collect();
    out.join("\n").trim_matches('\n').to_string()
}

/// Provider source/version maps from tool config — shared by transpile and
/// the stage-B providers emitter.
fn provider_maps(tool_config: &ToolConfig) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut provider_sources = HashMap::new();
    let mut provider_versions = HashMap::new();
    let def_ver = tool_config.provider_version.clone();
    for p in &tool_config.google_providers {
        let (name, ver) = ToolConfig::parse_provider_string_with_default(p, &def_ver);
        let source = if name.contains('/') { name.clone() } else { format!("hashicorp/{}", name) };
        provider_sources.insert(name.clone(), source);
        provider_versions.insert(name, ver);
    }
    for p in &tool_config.aws_providers {
        let (name, ver) = ToolConfig::parse_provider_string_with_default(p, &def_ver);
        let source = if name.contains('/') { name.clone() } else { format!("hashicorp/{}", name) };
        provider_sources.insert(name.clone(), source);
        provider_versions.insert(name, ver);
    }
    for p in &tool_config.azure_providers {
        let (name, ver) = ToolConfig::parse_provider_string_with_default(p, &def_ver);
        let source = if name.contains('/') { name.clone() } else { "hashicorp/azurerm".to_string() };
        provider_sources.insert(name.clone(), source);
        provider_versions.insert(name, ver);
    }
    for p in &tool_config.alibaba_providers {
        let (name, ver) = ToolConfig::parse_provider_string_with_default(p, &def_ver);
        provider_sources.insert(name.clone(), "aliyun/alicloud".to_string());
        provider_versions.insert(name, ver);
    }
    (provider_sources, provider_versions)
}

/// The estate argument as a path, with no compilation. `estate_input` layers the
/// `.gen.yaml` twin build on top of this; commands that read Satz natively want
/// the `.satz` file itself.
/// Resolve an estate argument: absolute stays as given; a relative path that
/// exists from the current directory is taken as given too (the runbooks'
/// long-standing `yaml/X.satz` form, which unconditional yaml_dir-prefixing
/// turned into `yaml/yaml/X.satz → not found`); otherwise it is looked up
/// inside yaml_dir. When both exist, the current-directory file wins and the
/// shadowing is named.
/// Read a `config.toml`, or the defaults when there is none.
///
/// Shared with `satz mcp`, which loads one per estate it opens rather than one
/// per process: estates do not agree about presets, schemas or the provider
/// version, and a server pinned to a single config could serve only the estates
/// that happened to match it.
///
/// The error is the DESCRIBED parse failure, so a caller can print it whole.
pub(crate) fn parse_tool_config(path: &Path) -> Result<ToolConfig, String> {
    if !path.exists() {
        return Ok(ToolConfig {
            yaml_dir: default_yaml_dir(),
            hcl_dir: default_hcl_dir(),
            include_dirs: default_include_dirs(),
            schema_dir: default_schema_dir(),
            presets_dir: default_presets_dir(),
            tf_tool: default_tf_tool(),
            google_providers: default_google_providers(),
            aws_providers: Vec::new(),
            azure_providers: Vec::new(),
            alibaba_providers: Vec::new(),
            provider_version: default_version(),
            auto_explode: default_auto_explode(),
            validation_level: default_validation_level(),
            import_config: None,
            silence: Vec::new(),
            dir: None,
        });
    }
    let content = fsx::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    toml::from_str(&content).map_err(|e| describe_toml_error(path, &content, &e))
}

/// The same config with every relative path resolved against the config's own
/// directory, which is what makes a command runnable from anywhere.
pub(crate) fn resolved_config(tool: &ToolConfig, config_dir: &Path) -> ToolConfig {
    let mut rc = tool.clone();
    rc.dir = Some(config_dir.to_path_buf());
    let at = |d: &str| config_dir.join(d).to_string_lossy().into_owned();
    if Path::new(&rc.yaml_dir).is_relative() {
        rc.yaml_dir = at(&rc.yaml_dir);
    }
    if Path::new(&rc.hcl_dir).is_relative() {
        rc.hcl_dir = at(&rc.hcl_dir);
    }
    if Path::new(&rc.schema_dir).is_relative() {
        rc.schema_dir = at(&rc.schema_dir);
    }
    if Path::new(&rc.presets_dir).is_relative() {
        rc.presets_dir = at(&rc.presets_dir);
    }
    rc.include_dirs = rc
        .include_dirs
        .into_iter()
        .map(|d| if Path::new(&d).is_relative() { at(&d) } else { d })
        .collect();
    rc
}

/// A relative estate path that already names `yaml_dir`, and the bare form it should have
/// been. Paths resolve INSIDE `yaml_dir`, so `estate_path` joins it a second time: naming it
/// yourself writes `yaml/yaml/x.satz`, which every later command misses because they all look
/// in `yaml/`. Returns `None` when the path is absolute, or does not start with the directory.
fn redundant_yaml_dir(estate: &str, yaml_dir: &str) -> Option<String> {
    if std::path::Path::new(estate).is_absolute() {
        return None;
    }
    // by component, not by string: the configured directory reaches here as
    // `./yaml` as often as `yaml`, and a textual prefix misses that
    let parts = |s: &str| -> Vec<String> {
        std::path::Path::new(s)
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(x) => Some(x.to_string_lossy().to_string()),
                _ => None,
            })
            .collect()
    };
    let dir = parts(yaml_dir);
    let est = parts(estate);
    let last = dir.last()?;
    if est.len() < 2 || &est[0] != last {
        return None;
    }
    Some(est[1..].join("/"))
}

pub(crate) fn estate_path(estate: PathBuf, runtime_config: &ToolConfig) -> PathBuf {
    if estate.is_absolute() {
        return estate;
    }
    if estate.exists() {
        let in_yaml_dir = PathBuf::from(&runtime_config.yaml_dir).join(&estate);
        if in_yaml_dir.exists() && crate::fsx::canonicalize(&in_yaml_dir).ok() != crate::fsx::canonicalize(&estate).ok() {
            eprintln!(
                "note: using ./{} (a different {} also exists inside yaml_dir)",
                estate.display(),
                in_yaml_dir.display()
            );
        }
        return estate;
    }
    PathBuf::from(&runtime_config.yaml_dir).join(estate)
}

/// Configure the identity live estate commands run as: on a
/// `deployment_mode = "cloud"` estate, the IaC service account
/// (`{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com` — the
/// emitter's own provider rule), exactly what `tofu` applies with, so the
/// human needs no org-wide read roles. Local mode and `--no-impersonate`
/// stay on the plain ADC. Bootstrap never calls this: on day 0 the SA may
/// not exist yet.
///
/// THE RULE: post-init, anything that reads or writes a customer's estate runs
/// as that estate's service account. The exceptions are deliberate and few —
/// bare `whoami` (it asks about the human; given an estate it binds like
/// everything else), `bootstrap` and `init` (no SA exists yet), and
/// `map-types` (no credentials at all). A live command that calls neither this
/// nor `disable_impersonation` runs as the human by accident, which is what
/// `the_estate_commands_are_the_ones_that_bind` gates.
///
/// Errors, binding nothing, when the estate's identity cannot be derived — see
/// `estate_impersonation_target` — and when the process is already acting as a
/// different estate — see `gcp::configure_impersonation`.
fn configure_estate_impersonation(
    input_path: &Path,
    runtime_config: &ToolConfig,
) -> Result<(), String> {
    crate::gcp::configure_impersonation(estate_impersonation_target(input_path, runtime_config)?)
}

/// WHICH service account an estate's live calls run as — the derivation alone,
/// with nothing bound. `None` is an answer: a local-mode estate impersonates
/// nothing.
///
/// An estate whose params cannot be read, or whose `deployment_mode` the compile
/// refuses, has no answer, and the error says why. Every caller refuses on it: a
/// command that ran on regardless would run as whoever is logged in.
///
/// The CLI binds it for the process; `satz mcp` scopes it to one call, because
/// it works through estates in turn. Both need the same answer, and deriving it
/// in one place is what keeps them from disagreeing.
pub(crate) fn estate_impersonation_target(
    input_path: &Path,
    runtime_config: &ToolConfig,
) -> Result<Option<String>, String> {
    Ok(estate_declaration(input_path, input_path.display().to_string(), runtime_config)?
        .impersonation_target()
        .map(str::to_string))
}

/// What an estate declares about its identity — its `deployment_mode` and its IaC
/// service account — read off its params. `named` is the estate as the operator
/// named it, which is what `whoami` repeats in the `satz migrate` it suggests.
///
/// Refused, naming the estate and the reason, when its params cannot be read or it
/// binds a mode the compile refuses.
pub(crate) fn estate_declaration(
    input_path: &Path,
    named: String,
    runtime_config: &ToolConfig,
) -> Result<crate::gcp::identity::EstateDeclaration, String> {
    let refused = |e: String| format!("{} — satz cannot tell which identity this estate runs as, and runs nothing for it", e);
    // a read or parse error names the file and the line already
    let (src, env) = satz_estate_env(input_path, &runtime_config.include_dirs).map_err(|e| refused(e.to_string()))?;
    crate::gcp::identity::EstateDeclaration::from_env(named, &env).map_err(|e| {
        // at the line the estate binds it on — at none when a pack binds it, as the compile says it
        let at = crate::findings::param_line(&src, "deployment_mode").map(|l| format!(":{}", l)).unwrap_or_default();
        refused(format!("{}{}: {}", input_path.display(), at, e))
    })
}

/// What `satz migrate` changes in an estate file to switch its deployment mode.
struct ModeSwitch {
    /// the mode the estate runs in now
    from: String,
    to: String,
    before: String,
    /// the file with `deployment_mode` bound to `to`; `None` when it runs in `to` already
    after: Option<String>,
}

/// The mode an estate runs in is read as the emitter and `whoami` read it — its
/// `deployment_mode`, `local` when it declares none — and the switch binds the new one in
/// the estate's own `params {}`: the value is replaced where the estate binds it, and the
/// line is added where the mode came from a pack's default or from nowhere. `to` is
/// `mode`, or the other of `local` and `cloud`.
fn mode_switch(
    input_path: &Path,
    runtime_config: &ToolConfig,
    mode: Option<String>,
) -> Result<ModeSwitch, Box<dyn std::error::Error>> {
    let before = fsx::read_to_string(input_path)?;
    let from = estate_declaration(input_path, input_path.display().to_string(), runtime_config)?.mode;
    let to = mode.unwrap_or_else(|| if from == "local" { "cloud" } else { "local" }.to_string());
    if from == to {
        return Ok(ModeSwitch { from, to, before, after: None });
    }
    // the mode the estate would run in is judged by the compile's reader before the file
    // is touched: cloud mode without `svc_iac_account` and `infra_project_name` is refused
    let (_, mut env) = satz_estate_env(input_path, &runtime_config.include_dirs)?;
    env.insert("deployment_mode".to_string(), serde_yaml::Value::String(to.clone()));
    crate::emitter::deployment_mode(&env).map_err(|e| {
        format!("{}: {} — the estate stays in {} mode, and nothing was changed", input_path.display(), e, from)
    })?;
    let after =crate::interview::bind(&before, "deployment_mode", &serde_yaml::Value::String(to.clone()))
        .map_err(|e| format!("{}: deployment_mode cannot be written: {}", input_path.display(), e))?;
    Ok(ModeSwitch { from, to, before, after: Some(after) })
}

/// The parameter table of a `.satz` estate, in the dialect's kebab-case
/// spelling.
///
/// The rename is not cosmetic: `anchor()` in the Satz YAML emitter maps
/// `snake_case` params to `kebab-case` anchor names, so every consumer of this
/// table — `customer-organization-id` lookups, and `build_variables_block`,
/// whose emitted `&anchors` a compiled pack references by name — keys on the
/// kebab form. It stays until those consumers are converted too.
pub(crate) fn satz_estate_params(
    input: &Path,
    include_dirs: &[String],
) -> Result<HashMap<String, serde_yaml::Value>, Box<dyn std::error::Error>> {
    Ok(satz_estate_env(input, include_dirs)?.1.into_iter().map(|(k, v)| (k.replace('_', "-"), v)).collect())
}

/// The source of a `.satz` estate and its parameter table as the compile reads it:
/// snake_case, every `use`d pack's defaults under the estate's own bindings.
fn satz_estate_env(
    input: &Path,
    include_dirs: &[String],
) -> Result<(String, satz_core::pipeline::Env), Box<dyn std::error::Error>> {
    let src = fsx::read_to_string(input)?;
    let loader = satz_loader(input, include_dirs);
    let env = satz_core::pipeline::estate_params(&input.to_string_lossy(), &src, &loader)?;
    Ok((src, env))
}

/// Resolve a `use` path the way the compiler does: beside the using file first,
/// then the configured include dirs.
fn satz_loader(
    input: &Path,
    include_dirs: &[String],
) -> impl Fn(&str) -> Result<String, String> {
    let base_dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    let dirs = include_dirs.to_vec();
    move |p: &str| -> Result<String, String> {
        let mut candidates = vec![base_dir.join(p)];
        candidates.extend(dirs.iter().map(|d| Path::new(d).join(p)));
        for c in candidates {
            if c.exists() {
                return std::fs::read_to_string(&c).map_err(|e| e.to_string());
            }
        }
        Err(format!("use \"{}\": file not found", p))
    }
}

/// The org policies a `.satz` estate declares, read off the folded IR.
///
/// Runs the full generation so the answer is exactly what `transpile` would
/// write — a policy suppressed or lost to a conflict must not show up as
/// desired. `tool_config` only feeds providers.tf, which is discarded here, so
/// the runtime config stands in for it.
pub(crate) fn satz_org_policy_bodies(
    input: &Path,
    runtime_config: &ToolConfig,
) -> Result<Vec<(String, serde_yaml::Value)>, Box<dyn std::error::Error>> {
    Ok(pipeline_b_generate(input, runtime_config, runtime_config)?.org_policies)
}

/// The emitted files as sorted lines, for comparing two compiles of one estate.
/// `prerequisites` is what that compile does with a missing role or API: `merge-presets`,
/// which writes those at its end, passes `Quiet`, so a compile before that step does not
/// refuse on the gap the step is there to close.
pub(crate) fn transpile_sorted_b(
    input: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
    prerequisites: PrerequisiteFindings,
) -> Result<String, Box<dyn std::error::Error>> {
    let out = pipeline_b_compile(input, tool_config, runtime_config, prerequisites, FindingsOutput::Stderr)?;
    fn sorted(s: &str) -> String {
        let mut lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        lines.sort_unstable();
        lines.join("\n")
    }
    Ok([&out.main_tf, &out.imports_tf, &out.variables_tf, &out.tfvars]
        .iter()
        .map(|s| sorted(s))
        .collect::<Vec<_>>()
        .join("\n---\n"))
}




/// Resolve a user-supplied path against the directory that owns its kind — estates against
/// `yaml_dir`, schemas against `schema_dir` — leaving absolute paths untouched.
///
/// Relative paths are never interpreted against the caller's working directory, so a
/// command behaves identically wherever it is run from. `base` must already be resolved
/// from config.toml's directory, i.e. come from `runtime_config`, not `tool_config`.
/// Human-readable output (reports, diffs) deliberately does not use this — those land in
/// the working directory, where the caller is looking.
pub(crate) fn resolve_against(base: &str, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(base).join(path)
    }
}

/// `presets_dir` must come from `runtime_config` so `--config <dir>/config.toml` is honoured.
/// No fallback to pre-presets_dir layouts — if the library is not where the config says,
/// this fails visibly rather than quietly reading from a legacy location.
fn load_import_config(
    path: Option<PathBuf>,
    tool_config: &ToolConfig,
    presets_dir: &str,
) -> Result<Option<ImportConfig>, Box<dyn std::error::Error>> {
    let config_path = if let Some(p) = path {
        resolve_against(presets_dir, p)
    } else if let Some(p_str) = &tool_config.import_config {
        resolve_against(presets_dir, PathBuf::from(p_str))
    } else {
        // `get-presets` writes the presets library to presets_dir (beside config.toml).
        let default = resolve_against(presets_dir, PathBuf::from("import-config.yaml"));
        if default.exists() {
            default
        } else {
            return Ok(None);
        }
    };

    if !config_path.exists() {
         return Err(format!("import configuration file not found at: {}", config_path.display()).into());
    }

    let content = fsx::read_to_string(&config_path)?;
    let mut config: ImportConfig = serde_yaml::from_str(&content)?;
    // the generated field maps ride in a sibling file so the hand-maintained
    // rows (and their comments) are never rewritten by a generator
    let type_map_path = config_path.with_file_name("type-map.yaml");
    if type_map_path.exists() {
        let maps: std::collections::BTreeMap<String, crate::align::TypeMap> =
            serde_yaml::from_str(&fsx::read_to_string(&type_map_path)?)
                .map_err(|e| format!("{}: {}", type_map_path.display(), e))?;
        for (t, tm) in maps {
            if let Some(row) = config.resource_types.get_mut(&t) {
                if !tm.map.is_empty() {
                    row.map = Some(tm.map);
                }
            }
        }
    }
    Ok(Some(config))
}

/// Is this invocation asking for the ROOT help, and in the long form? `Some(true)`
/// for `--help`, `Some(false)` for `-h` and for `satz help`, `None` for everything
/// else — including `satz help transpile` and `satz transpile --help`, which clap
/// answers itself.
///
/// The test is "no argument names a command", and the value of an option is not
/// an argument for this purpose: `satz --config plan --help` asks for the root
/// help about a directory called `plan`, not for `plan`'s. Which options take a
/// value is read off the command rather than listed here. Bare `satz` is not a
/// root-help request at all — it parses, and the `None` command arm prints the
/// same help afterwards.
fn root_help_request() -> Option<bool> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = Cli::command();
    cmd.build();
    let names: std::collections::HashSet<String> = cmd
        .get_subcommands()
        .filter(|s| s.get_name() != "help")
        .flat_map(|s| {
            std::iter::once(s.get_name().to_string())
                .chain(s.get_all_aliases().map(|a| a.to_string()))
        })
        .collect();
    let takes_value: std::collections::HashSet<String> = cmd
        .get_arguments()
        .filter(|a| a.get_num_args().is_none_or(|n| n.takes_values()))
        .filter_map(|a| a.get_long())
        .map(|l| format!("--{l}"))
        .collect();

    let mut skip_next = false;
    for arg in &argv {
        if std::mem::take(&mut skip_next) {
            continue;
        }
        // `--config=x` carries its value; `--config x` eats the token after it
        if takes_value.contains(arg) {
            skip_next = true;
            continue;
        }
        if names.contains(arg) {
            return None;
        }
    }
    if argv.iter().any(|a| a == "--help") {
        return Some(true);
    }
    if argv.iter().any(|a| a == "-h") || argv.first().is_some_and(|a| a == "help") {
        return Some(false);
    }
    None
}

/// One group of `COMMAND_GROUPS`, rendered BY clap so the block matches the rest
/// of the help in styling and wrapping — down to the `[aliases: …]` suffix the
/// org-policy commands carry. The heading is clap's too: a
/// `subcommand_help_heading` over an `{all-args}` template that has no args to
/// print, which is what keeps it bold on a terminal and plain in a pipe.
///
/// The one adjustment to the clones is `display_order`. `write_subcommands`
/// orders by `(display_order, name)` and every command defaults to the same 999,
/// so without it a group would come out alphabetical instead of in the order the
/// table names it.
///
/// Each group aligns to its own widest name rather than to the widest in the
/// whole CLI. The heading already separates the blocks, so the column need not be
/// shared, and not sharing it buys the short groups back the ~20 characters that
/// `export-organizational-policies` would otherwise cost every one of them.
fn print_group_block(heading: &'static str, names: &[&'static str], subcommands: &[clap::Command]) {
    let mut block = clap::Command::new("satz")
        .help_template("{all-args}")
        .subcommand_help_heading(heading)
        .max_term_width(110)
        // no `help` command and no `-h` of its own: this throwaway exists to render
        // one Commands block, and either would add an Options section under it
        .disable_help_subcommand(true)
        .disable_help_flag(true);
    for (order, name) in names.iter().enumerate() {
        let sub = subcommands
            .iter()
            .find(|s| s.get_name() == *name)
            .unwrap_or_else(|| panic!("COMMAND_GROUPS names `{name}`, which is not a satz command"));
        block = block.subcommand(sub.clone().display_order(order));
    }
    let _ = block.print_help();
    println!();
}

/// `satz`, `satz -h`, `satz --help` and `satz help`: the help clap would print,
/// with the one flat list of thirty commands replaced by the groups of
/// `COMMAND_GROUPS`.
///
/// Printed in three parts rather than assembled into one template, because clap
/// 4 can express neither half of this: a subcommand has no help heading to set
/// (only `subcommand_help_heading`, which renames the single `Commands:` block
/// as a whole), and `{options}` in a template deliberately flattens the
/// `Global options` heading away. So head and tail are printed by clap off two
/// throwaway copies and the groups go between them. The tail copy hides every
/// subcommand — that is what makes `{all-args}` skip the flat list while still
/// printing `Options:` and `Global options:` natively — and it is local to this
/// function, so parsing, shell completion and clap's did-you-mean suggestions
/// never see a hidden command.
fn print_root_help(long: bool) {
    let mut canonical = Cli::command();
    canonical.build(); // `help` is generated here, and `COMMAND_GROUPS` lists it
    let subcommands: Vec<clap::Command> = canonical.get_subcommands().cloned().collect();

    // the about and the usage line, off an untouched copy so usage keeps [COMMAND]
    let mut head =
        Cli::command().help_template("{before-help}{about-with-newline}\n{usage-heading} {usage}\n");
    let _ = if long { head.print_long_help() } else { head.print_help() };
    println!();

    for (heading, names) in COMMAND_GROUPS {
        print_group_block(heading, names, &subcommands);
    }

    let mut tail = Cli::command()
        .help_template("{all-args}{after-help}")
        .disable_help_subcommand(true)
        .mut_subcommands(|s| s.hide(true));
    let _ = if long { tail.print_long_help() } else { tail.print_help() };
    println!();
}

/// The CLI as it parses and as a command's help shows it: the global options hidden
/// below the root. clap copies every global option into every command, so
/// `satz transpile --help` repeated the options that belong to `satz` itself under
/// each command. They are listed once, by `satz --help`, and still parse after any
/// command — hiding changes the help and nothing else.
///
/// clap copies a global option into a command only when the command has no option
/// of that id yet, so each command is given a HIDDEN copy before the build. Hiding
/// the copies after the build is not possible: a built command's name lookup is
/// computed once, and re-adding an option moves it.
fn cli_command() -> clap::Command {
    fn hide_below(cmd: clap::Command, globals: &[clap::Arg]) -> clap::Command {
        cmd.mut_subcommands(|mut sub| {
            for global in globals {
                if sub.get_arguments().all(|a| a.get_id() != global.get_id()) {
                    sub = sub.arg(global.clone());
                }
            }
            hide_below(sub, globals)
        })
    }
    let cmd = Cli::command();
    let globals: Vec<clap::Arg> =
        cmd.get_arguments().filter(|a| a.is_global_set()).map(|a| a.clone().hide(true)).collect();
    hide_below(cmd, &globals)
}

/// `satz --verbose` with no command: the grouped root help, then every command's
/// own help, walked in `COMMAND_GROUPS` order so the long form reads like the
/// short one.
fn print_recursive_help(long: bool) {
    print_root_help(long);
    println!();

    let canonical = cli_command();
    let subcommands: Vec<clap::Command> = canonical.get_subcommands().cloned().collect();
    // the rule matches what clap wraps to: the terminal, capped like max_term_width
    let width = terminal_size::terminal_size().map(|(w, _)| w.0 as usize).unwrap_or(100).min(110);

    for name in COMMAND_GROUPS.iter().flat_map(|(_, names)| names.iter()) {
        // `help` would print the very thing we are already inside
        if *name == "help" {
            continue;
        }
        let Some(mut sub) = subcommands.iter().find(|s| s.get_name() == *name).cloned() else {
            continue;
        };
        println!("\n{}", "=".repeat(width));
        println!("COMMAND: {name}");
        println!("{}\n", "=".repeat(width));
        let _ = if long { sub.print_long_help() } else { sub.print_help() };
        println!();
    }
}


use crate::github::{api_error, api_get, API_URL, DOCS_URL, REPO};

/// Fetches latest release from GitHub and returns (latest_version, html_url) if an update is available.
async fn check_update_available(client: &reqwest::Client) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
    let url = format!("{}/{}/releases/latest", API_URL, REPO);
    let response = api_get(client, &url).send().await?;
    if !response.status().is_success() {
        // Surfaced, not swallowed: a 404 here once hid a repo that had no
        // releases at all. The caller prints it and continues with the user's command.
        return Err(api_error("Update check failed", response.status(), response.headers(), "").into());
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
    }
    let release: Release = response.json().await?;
    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let current = env!("CARGO_PKG_VERSION");
    if compare_versions(current, &latest_version)? < 0 {
        Ok(Some((latest_version, release.html_url)))
    } else {
        Ok(None)
    }
}

/// If global settings say so, run a check-only update check and optionally persist last_update_check (daily).
/// Whether a command runs the background update check. Not the update itself, not
/// `init`, not `whoami` — and not the two protocol servers: `mcp` and `lsp` speak
/// JSON-RPC on stdout and are started by a client, so a network round trip to
/// GitHub before the first message is a cost with no reader, and the notice would
/// have nobody to read it but the client's parser.
fn checks_for_updates(cmd: &Commands) -> bool {
    !matches!(
        cmd,
        Commands::SelfUpdate { .. } | Commands::Init { .. } | Commands::Whoami { .. } | Commands::Mcp { .. } | Commands::Lsp
    )
}

async fn maybe_check_for_updates(settings: &mut GlobalSettings) -> Result<(), Box<dyn std::error::Error>> {
    let freq = settings.self_update_frequency.as_str();
    if freq == "never" {
        return Ok(());
    }
    if freq == "daily" {
        if let Some(ref last) = settings.last_update_check {
            let last_ts: u64 = last.parse().unwrap_or(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now.saturating_sub(last_ts) < 86400 {
                return Ok(());
            }
        }
    }
    let client = reqwest::Client::builder()
        .user_agent("satz-update-checker")
        .build()?;
    let update = match check_update_available(&client).await {
        Ok(update) => update,
        Err(e) => {
            eprintln!("⚠️  {} (set self_update_frequency = \"never\" in ~/.config/satz/satz.toml to silence)", e);
            None
        }
    };
    if freq == "daily" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        settings.last_update_check = Some(now.to_string());
        save_global_settings(settings)?;
    }
    if let Some((version, url)) = update {
        // stderr: stdout is a command's output — the JSON of `--format json`, or the
        // protocol of `mcp` and `lsp` — and a notice in front of it breaks every reader.
        eprintln!("⚠️  Update available: {} (current: {}). Run `satz self-update` to install. {}", version, env!("CARGO_PKG_VERSION"), url);
    }
    Ok(())
}

/// The installer verifies the archive it downloads with `sha256sum`, and SKIPS that check —
/// printing one line — when the command is not on PATH, which is the case on macOS before
/// 14 and on a minimal Linux. The installer script itself is verified here against its
/// sidecar; the archive is the installer's business. So where `sha256sum` is missing and
/// `shasum` is present, a shim that runs `shasum -a 256` goes first on the installer's PATH
/// (`Some(PATH)`), and where both are missing the update is refused unless the operator
/// asked to skip checksums. `None`: the PATH as it is.
#[cfg(unix)]
fn installer_checksum_path(
    temp_dir: &Path,
    path: &std::ffi::OsStr,
    skip_checksum: bool,
) -> Result<Option<std::ffi::OsString>, String> {
    use std::os::unix::fs::PermissionsExt;
    let on_path = |name: &str| {
        std::env::split_paths(path).any(|d| {
            std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    };
    if on_path("sha256sum") {
        return Ok(None);
    }
    if on_path("shasum") {
        let bin = temp_dir.join("checksum-bin");
        std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {}", bin.display(), e))?;
        let shim = bin.join("sha256sum");
        fsx::write(&shim, b"#!/bin/sh\nexec shasum -a 256 \"$@\"\n").map_err(|e| e.to_string())?;
        fsx::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        let mut dirs = vec![bin];
        dirs.extend(std::env::split_paths(path));
        return std::env::join_paths(dirs).map(Some).map_err(|e| e.to_string());
    }
    if skip_checksum {
        eprintln!("⚠️  neither sha256sum nor shasum is on PATH: the installer cannot verify the archive (--skip-checksum)");
        return Ok(None);
    }
    Err("neither sha256sum nor shasum is on PATH, so the installer cannot verify the archive it downloads — \
         install one (coreutils provides sha256sum), or re-run with --skip-checksum to update without the check"
        .to_string())
}

// On Windows the update is refused before any download, so the installer's options are
// never read there.
#[cfg_attr(windows, allow(unused_variables))]
async fn run_self_update( open_docs: bool, check_only: bool, skip_checksum: bool) -> Result<(), Box<dyn std::error::Error>> {

    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", current_version);

    let client = reqwest::Client::builder()
        .user_agent("satz-update-checker")
        .build()?;

    let url = format!("{}/{}/releases/latest", API_URL, REPO);
    let response = api_get(&client, &url).send().await?;

    if !response.status().is_success() {
        // `self-update` shares the 60/hour quota with the preset commands, so a
        // fleet-wide sweep can be what actually stopped this. Say which.
        return Err(api_error(
            "Failed to fetch release info",
            response.status(),
            response.headers(),
            "",
        )
        .into());
    }

    // the assets are read by the installer path alone, which is unix-only
    #[derive(Deserialize)]
    #[cfg_attr(windows, allow(dead_code))]
    struct Asset {
        name: String,
        browser_download_url: String,
    }

    #[derive(Deserialize)]
    #[cfg_attr(windows, allow(dead_code))]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        assets: Vec<Asset>,
    }

    let release: Release = response.json().await?;
    let latest_version = release.tag_name.trim_start_matches('v');
    println!("Latest version: {}", latest_version);

    if compare_versions(current_version, latest_version)? < 0 {
        println!("\n⚠️  A new version is available!");
        println!("   Current: {}", current_version);
        println!("   Latest:  {}", latest_version);
        println!("   Release: {}", release.html_url);
        if check_only {
            println!("\nRun `satz self-update` to install.");
            return Ok(());
        }
        // The release's installer is a shell script. Refused before anything is
        // downloaded, rather than after a download and a checksum it then cannot run.
        #[cfg(windows)]
        {
            return Err(format!(
                "self-update installs through a shell script and does not run on Windows — install {} with PowerShell:\n  \
                 powershell -ExecutionPolicy Bypass -c \"irm https://github.com/{}/releases/latest/download/satz-installer.ps1 | iex\"",
                latest_version, REPO
            )
            .into());
        }
        // everything below runs the release's shell installer, which Windows cannot
        #[cfg(unix)]
        {
            println!("\n📥 Installing update...");

            // Installer and sidecar both come from THIS release object, never from
            // `/releases/latest/download/` — a release published between the API
            // call and the download would otherwise pair one release's installer
            // with another's checksum.
            let installer_asset = release.assets.iter()
                .find(|a| a.name == "satz-installer.sh")
                .ok_or_else(|| format!(
                    "Release {} has no satz-installer.sh asset — the release build did not finish. Aborting.",
                    release.html_url
                ))?;

            // Download installer as bytes for checksum verification
            let installer_bytes = client
                .get(&installer_asset.browser_download_url)
                .send()
                .await?
                .error_for_status()
                .map_err(|e| format!("installer download failed: {}", e))?
                .bytes()
                .await?;

            // Checksum verification
            let checksum_asset = release.assets.iter()
                .find(|a| a.name == "satz-installer.sh.sha256");
            match checksum_asset {
                Some(asset) => {
                    let expected_raw = client
                        .get(&asset.browser_download_url)
                        .send()
                        .await?
                        .error_for_status()
                        .map_err(|e| format!("checksum download failed: {}", e))?
                        .text()
                        .await?;
                    let expected = expected_raw.split_whitespace().next().unwrap_or("").to_lowercase();
                    use sha2::{Digest, Sha256};
                    let actual = hex::encode(Sha256::digest(&installer_bytes));
                    if actual != expected {
                        return Err(format!(
                            "Checksum mismatch — installer may have been tampered with.\n\
                             Expected: {}\n\
                             Got:      {}\n\
                             Aborting. Download the release manually from {}",
                            expected, actual, release.html_url
                        ).into());
                    }
                    println!("✅ Checksum verified");
                }
                None if skip_checksum => {
                    eprintln!(
                        "⚠️  No checksum file found in this release. \
                         Proceeding without verification (--skip-checksum)."
                    );
                }
                None => {
                    return Err(
                        "No checksum file (satz-installer.sh.sha256) found in this release.\n\
                         Cannot verify installer integrity. Aborting.\n\
                         If you are confident in the download, re-run with --skip-checksum."
                        .into()
                    );
                }
            }

            // Write to temp file and execute
            // a private, unpredictable directory: on a shared /tmp a pre-created
            // file at a guessable path could be swapped between write and run
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let temp_dir = std::env::temp_dir().join(format!("satz-self-update-{}-{}", std::process::id(), nonce));
            {
                let mut b = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    b.mode(0o700);
                }
                b.create(&temp_dir).map_err(|e| format!("{}: {}", temp_dir.display(), e))?;
            }
            let temp_file = temp_dir.join("satz-installer.sh");
            fsx::write(&temp_file, &installer_bytes)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fsx::set_permissions(&temp_file, std::fs::Permissions::from_mode(0o755))?;

                let path_for_installer = match installer_checksum_path(
                    &temp_dir,
                    &std::env::var_os("PATH").unwrap_or_default(),
                    skip_checksum,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&temp_dir);
                        return Err(e.into());
                    }
                };
                let mut installer = std::process::Command::new("sh");
                installer.arg(&temp_file);
                if let Some(path) = path_for_installer {
                    installer.env("PATH", path);
                }
                let status = installer.status()?;
                let _ = std::fs::remove_dir_all(&temp_dir);

                if status.success() {
                    println!("✅ Update installed successfully!");
                    println!("   Please restart your terminal or run: source ~/.profile");

                    println!("   Documentation: {}", DOCS_URL);
                    if open_docs {
                        open_url(DOCS_URL)?;
                    }
                } else {
                    return Err("Failed to run installer script".into());
                }
            }
        }

    } else {
        println!("✅ You are running the latest version!");
    }

    Ok(())
}



/// `--html-help`: the documentation site, at the section of the invoked
/// command when the README has one (`id="cmd-<name>"`, stamped by
/// `scripts/build-site.py`), else the front page — said, not assumed.
/// `satz fmt`: every `.satz` under the given paths, rewritten in its canonical
/// layout — or, with `--check`, named when it is not. `*.diff.satz` files are
/// unified diffs and are skipped.
fn run_fmt(paths: &[PathBuf], check: bool, stdin: bool) -> Result<(), Box<dyn std::error::Error>> {
    if stdin {
        let mut src = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut src)?;
        print!("{}", satz_core::fmt::format(&src).map_err(|e| format!("fmt: <stdin>: {}", e))?);
        return Ok(());
    }
    if paths.is_empty() {
        return Err("fmt: name the files or directories to format (or --stdin)".into());
    }
    fn collect(p: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
        if p.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
            entries.sort();
            for e in entries {
                collect(&e, out)?;
            }
        } else if p.is_file() {
            let name = p.to_string_lossy();
            if name.ends_with(".satz") && !name.ends_with(".diff.satz") {
                out.push(p.to_path_buf());
            }
        } else {
            return Err(format!("fmt: {}: no such file or directory", p.display()).into());
        }
        Ok(())
    }
    let mut files = Vec::new();
    for p in paths {
        collect(p, &mut files)?;
    }
    if files.is_empty() {
        return Err("fmt: no .satz file under the given paths".into());
    }
    let mut errors = Vec::new();
    let mut changed = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f)?;
        match satz_core::fmt::format(&src) {
            Err(e) => errors.push(format!("{}: {}", f.display(), e)),
            Ok(out) if out == src => {}
            Ok(out) => {
                if !check {
                    fsx::write_verbatim(f, out)?;
                }
                changed.push(f.display().to_string());
            }
        }
    }
    if check {
        for c in &changed {
            println!("{}", c);
        }
    } else {
        for c in &changed {
            println!("fmt: rewrote {}", c);
        }
    }
    for e in &errors {
        eprintln!("{}", e);
    }
    let unchanged = files.len() - changed.len() - errors.len();
    if check {
        if !changed.is_empty() || !errors.is_empty() {
            return Err(format!(
                "fmt --check: {} file(s) not formatted, {} with errors, {} formatted — run `satz fmt` on them",
                changed.len(),
                errors.len(),
                unchanged
            )
            .into());
        }
        println!("fmt --check: OK — {} file(s) formatted", files.len());
    } else {
        println!("fmt: {} file(s) rewritten, {} already formatted", changed.len(), unchanged);
        if !errors.is_empty() {
            return Err(format!("fmt: {} file(s) could not be formatted", errors.len()).into());
        }
    }
    Ok(())
}

fn open_html_help(subcommand: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    const DOCUMENTED: &[&str] = &[
        "init", "bootstrap", "transpile", "migrate", "import", "update-schema", "get-presets", "require",
        "report-compliance", "merge-presets", "check-presets", "self-update", "open-readme", "completion",
        "scan-plan", "generate-migration", "run-actions", "update-prerequisites", "review-pack", "fmt", "lsp",
    ];
    match subcommand {
        Some(cmd) if DOCUMENTED.contains(&cmd) => open_url(&format!("{}#cmd-{}", DOCS_URL, cmd)),
        Some(cmd) => {
            println!("no dedicated section for `{}` in the README yet — opening the command table", cmd);
            open_url(&format!("{}#cli-usage", DOCS_URL))
        }
        None => open_url(DOCS_URL),
    }
}

/// Open a URL in the default browser (never an editor).
fn open_url(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Opening {}", url);
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let status: std::io::Result<std::process::ExitStatus> = Err(std::io::Error::other("no browser opener on this platform"));
    match status {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => Err(format!("could not open {}: the opener exited with {}", url, st).into()),
        Err(e) => Err(format!("could not open {}: {}", url, e).into()),
    }
}


/// Ordering of two dotted numeric versions. A component that is not a number
/// (`0.46.15-rc1`, a malformed tag) is an error — read as 0 it would call a
/// newer release "older" and print "you are on the latest version".
fn compare_versions(v1: &str, v2: &str) -> Result<i32, String> {
    let parse_version = |v: &str| -> Result<Vec<u32>, String> {
        v.split('.')
            .map(|s| s.parse::<u32>().map_err(|_| format!("version `{}` has a non-numeric component `{}`", v, s)))
            .collect()
    };
    let v1_parts = parse_version(v1)?;
    let v2_parts = parse_version(v2)?;
    let max_len = v1_parts.len().max(v2_parts.len());
    for i in 0..max_len {
        let v1_val = v1_parts.get(i).copied().unwrap_or(0);
        let v2_val = v2_parts.get(i).copied().unwrap_or(0);
        if v1_val < v2_val {
            return Ok(-1);
        }
        if v1_val > v2_val {
            return Ok(1);
        }
    }
    Ok(0)
}

/// Turn a TOML parse failure into something actionable: which line, what is on it, and —
/// when `--config` was pointed at a customer YAML — what the caller probably meant.
/// The raw `toml` error carries the whole file in its `Debug` output, which is what the
/// caller would otherwise see.
fn describe_toml_error(path: &Path, content: &str, err: &toml::de::Error) -> String {
    let mut msg = format!("Failed to parse '{}' as TOML: {}", path.display(), err.message());

    if let Some(span) = err.span() {
        let offset = span.start.min(content.len());
        let line_start = content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = content[line_start..]
            .find('\n')
            .map(|i| line_start + i)
            .unwrap_or(content.len());
        msg.push_str(&format!(
            "\n  at line {}, column {}:\n    {}",
            content[..offset].matches('\n').count() + 1,
            offset - line_start + 1,
            content[line_start..line_end].trim_end()
        ));
    }

    let is_yaml = matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("yaml") | Some("yml")
    );
    if is_yaml {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("<config>.yaml");
        msg.push_str(&format!(
            "\n\nThat file is YAML. '--config' expects the tool's own config.toml.\n\
             Pass the customer YAML as the command's argument instead, e.g.:\n    \
             satz bootstrap {name}"
        ));
    }

    msg
}


/// Best-effort detection of the user's shell when none is passed to `completion`.
/// Prefers `$SHELL`; falls back to zsh on macOS (its default login shell) and
/// PowerShell on Windows. Returns an error elsewhere so the user passes one.
fn detect_default_shell() -> Result<String, Box<dyn std::error::Error>> {
    if cfg!(windows) {
        return Ok("powershell".to_string());
    }
    if let Ok(shell_path) = std::env::var("SHELL") {
        if let Some(name) = Path::new(&shell_path).file_name().and_then(|s| s.to_str()) {
            match name {
                "zsh" => return Ok("zsh".to_string()),
                "bash" => return Ok("bash".to_string()),
                "fish" => return Ok("fish".to_string()),
                _ => {}
            }
        }
    }
    if cfg!(target_os = "macos") {
        return Ok("zsh".to_string());
    }
    Err("Could not detect your shell from $SHELL. \
         Pass one explicitly: satz completion <bash|zsh|fish|powershell>"
        .into())
}

fn run_completion(shell_str: &str, install: bool) -> Result<(), Box<dyn std::error::Error>> {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    use std::str::FromStr;

    let shell = Shell::from_str(shell_str)
        .map_err(|_| format!("Unknown shell '{}'. Supported shells: bash, zsh, fish, powershell", shell_str))?;

    let mut cmd = Cli::command();
    let bin_name = "satz";

    if install {
        let (path, post_install_msg) = completion_install_path(shell)?;
        if let Some(parent) = path.parent() {
            fsx::create_dir_all(parent)?;
        }
        let mut file = fsx::create_file(&path)?;
        generate(shell, &mut cmd, bin_name, &mut file);
        println!("Completion script installed to: {}", path.display());
        if let Some(msg) = post_install_msg {
            println!("{}", msg);
        }
    } else {
        generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    }

    Ok(())
}


fn completion_install_path(shell: CompletionShell) -> Result<(PathBuf, Option<String>), Box<dyn std::error::Error>> {
    use clap_complete::Shell;
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    let (path, msg): (PathBuf, Option<String>) = match shell {
        Shell::Bash => (
            PathBuf::from(format!("{}/.local/share/bash-completion/completions/satz", home)),
            Some("Ensure bash-completion is installed and sourced in your ~/.bashrc".to_string()),
        ),
        Shell::Zsh => (
            PathBuf::from(format!("{}/.zsh/completions/_satz", home)),
            Some("Ensure ~/.zsh/completions is in your fpath — add to ~/.zshrc:\n  fpath=(~/.zsh/completions $fpath)\n  autoload -Uz compinit && compinit".to_string()),
        ),
        Shell::Fish => (
            PathBuf::from(format!("{}/.config/fish/completions/satz.fish", home)),
            None,
        ),
        Shell::PowerShell => {
            let userprofile = std::env::var("USERPROFILE").unwrap_or_else(|_| home.clone());
            (
                PathBuf::from(format!(r"{}\Documents\PowerShell\Completions\satz.ps1", userprofile)),
                Some("Add to your $PROFILE:\n  . \"$env:USERPROFILE\\Documents\\PowerShell\\Completions\\satz.ps1\"".to_string()),
            )
        },
        _ => return Err(format!("Unsupported shell: {:?}", shell).into()),
    };
    Ok((path, msg))
}

#[cfg(test)]
mod command_groups {
    //! `COMMAND_GROUPS` is what `satz --help` prints, and nothing about it is
    //! derived: a command lands in a group because the table says so. So the one
    //! thing that can go wrong is the table falling behind the CLI — a new
    //! command that no group names would simply not be printed, and a help page
    //! that quietly omits a command is worse than one that never grouped at all.
    //! These tests make that a build failure instead of a discovery.

    use super::*;
    use std::collections::BTreeSet;

    fn declared() -> Vec<&'static str> {
        COMMAND_GROUPS.iter().flat_map(|(_, names)| names.iter().copied()).collect()
    }

    /// Which identity a command's Google API calls run as.
    ///
    /// This exists because the boundary used to be implicit — five call sites of
    /// `configure_estate_impersonation` — and two commands fell outside it by
    /// accident rather than by decision: `import --into` ran `adopt`'s exact read
    /// path as the human, and every MCP tool did too.
    #[derive(Debug, PartialEq)]
    enum Identity {
        /// Binds the estate's IaC service account, as `tofu` applies with.
        EstateSa,
        /// Deliberately the human's ADC. The reason is the point of the entry.
        Human(&'static str),
        /// satz itself calls no Google API. It may still shell out to `tofu` or
        /// Checkov, or talk to GitHub — neither is a Google credential.
        NoGoogleApi,
        /// `mcp` binds per tool call, from the estate each tool names.
        PerTool,
        /// The human by default, the estate's service account when the command
        /// is GIVEN an estate to answer for. One command, two questions.
        HumanOrEstate(&'static str),
        /// Calls no Google API of its own; a flag that runs `plan` or `apply`
        /// for the operator binds the estate's service account through the same
        /// site those two bind at.
        EstateSaWhenItRuns(&'static str),
    }

    /// EVERY command, classified. A new one fails `every_command_declares_an_identity`
    /// until it is listed here, which is the point: running as the wrong principal is
    /// not visible in output, in tests, or in a diff — only in an audit log, months later.
    const IDENTITIES: &[(&str, Identity)] = &[
        ("export-organizational-policies", Identity::EstateSa),
        ("diff-organizational-policies", Identity::EstateSa),
        ("report-organizational-policies", Identity::EstateSa),
        ("adopt-org-policies", Identity::EstateSa),
        ("report-compliance", Identity::EstateSa),
        ("adopt", Identity::EstateSa),
        // Only `--into` names an estate; plain discovery writes a NEW file and so
        // has no estate to be, exactly like `init`.
        ("import", Identity::EstateSa),
        ("bootstrap", Identity::Human("day 0 — the service account does not exist yet")),
        ("init", Identity::Human("--from-live runs before the estate exists")),
        (
            "whoami",
            Identity::HumanOrEstate(
                "bare, the question IS who the human is; given an estate, who that estate acts as",
            ),
        ),
        ("map-types", Identity::Human("Discovery documents are public — no credential at all")),
        ("mcp", Identity::PerTool),
        ("transpile", Identity::EstateSaWhenItRuns("only --plan/--apply reach an API, through run_tf")),
        ("update-prerequisites", Identity::NoGoogleApi),
        ("review-pack", Identity::NoGoogleApi),
        ("hcl-init", Identity::NoGoogleApi),
        // The API preflight: as the identity the emitted provider impersonates,
        // which is the identity `tofu` is about to act as in the same directory.
        ("plan", Identity::EstateSa),
        ("apply", Identity::EstateSa),
        (
            "migrate",
            Identity::Human("--mode cloud assigns Groups Admin to the IaC service account, which cannot give itself an admin role"),
        ),
        ("scan-plan", Identity::NoGoogleApi),
        ("generate-migration", Identity::NoGoogleApi),
        ("run-actions", Identity::NoGoogleApi),
        ("get-presets", Identity::NoGoogleApi),
        ("merge-presets", Identity::NoGoogleApi),
        ("check-presets", Identity::NoGoogleApi),
        ("doc-packs", Identity::NoGoogleApi),
        ("pack-graph", Identity::NoGoogleApi),
        ("require", Identity::NoGoogleApi),
        ("prowler", Identity::NoGoogleApi),
        ("fmt", Identity::NoGoogleApi),
        ("lsp", Identity::NoGoogleApi),
        ("silence", Identity::NoGoogleApi),
        ("questions", Identity::NoGoogleApi),
        ("interview", Identity::NoGoogleApi),
        ("packs", Identity::NoGoogleApi),
        ("add-pack", Identity::NoGoogleApi),
        ("remove-pack", Identity::NoGoogleApi),
        ("scan", Identity::NoGoogleApi),
        ("triage", Identity::NoGoogleApi),
        ("remediation-plan", Identity::NoGoogleApi),
        ("update-schema", Identity::NoGoogleApi),
        ("self-update", Identity::NoGoogleApi),
        ("completion", Identity::NoGoogleApi),
        ("open-readme", Identity::NoGoogleApi),
        ("help", Identity::NoGoogleApi),
    ];

    /// The rule: post-init, anything that reads or writes a customer's estate runs
    /// as that estate's service account. Every exception is named, with its reason.
    #[test]
    fn every_command_declares_an_identity() {
        let mut cmd = Cli::command();
        cmd.build();
        let cli: BTreeSet<&str> =
            cmd.get_subcommands().filter(|s| !s.is_hide_set()).map(|s| s.get_name()).collect();
        let classified: BTreeSet<&str> = IDENTITIES.iter().map(|(n, _)| *n).collect();

        let missing: Vec<_> = cli.difference(&classified).collect();
        assert!(
            missing.is_empty(),
            "these commands do not say which identity their live calls run as: {missing:?} — \
             add each to IDENTITIES in src/main.rs. If it touches a customer's estate the answer \
             is EstateSa (call configure_estate_impersonation at its arm); if it deliberately \
             stays on the human's ADC, say why."
        );
        let unknown: Vec<_> = classified.difference(&cli).collect();
        assert!(unknown.is_empty(), "IDENTITIES names commands the CLI does not have: {unknown:?}");
    }

    /// Which dispatch site binds which commands. Not one site per command:
    /// `adopt` and `adopt-org-policies` are both served by `run_adopt`, so seven
    /// commands are bound by six sites. Spelling that out is what lets the test
    /// below compare the table against the code instead of guessing a number.
    const BINDING_SITES: &[&[&str]] = &[
        &["export-organizational-policies"],
        &["diff-organizational-policies"],
        &["report-organizational-policies"],
        &["report-compliance"],
        &["adopt", "adopt-org-policies"],
        &["import"],
        &["whoami"],
        // `run_tf`'s API preflight, reached by all three routes into the tool.
        // It binds from the emitted provider rather than from an estate file,
        // because these commands are given a directory and no estate.
        &["plan", "apply", "transpile"],
    ];

    /// The classification is a claim about the code, so check it against the code.
    /// A command that claims the service account and does not bind it runs as the
    /// human — which shows up in no output, no test and no diff, only in an audit
    /// log months later.
    /// `mcp` and `lsp` own stdout as a protocol and are started by a client; the
    /// update itself, `init` and `whoami` never checked. Everything else does, and
    /// hears about it on stderr.
    #[test]
    fn the_protocol_servers_never_check_for_updates() {
        let parse = |args: &[&str]| {
            let mut argv = vec!["satz"];
            argv.extend_from_slice(args);
            Cli::try_parse_from(argv).expect("parses").command.expect("a subcommand")
        };
        for args in [&["mcp"][..], &["lsp"], &["self-update"], &["whoami"]] {
            assert!(!checks_for_updates(&parse(args)), "{args:?} must not check");
        }
        for args in [&["questions", "x.satz", "--format", "text", "--out", "q.txt"][..], &["transpile", "x.satz"]] {
            assert!(checks_for_updates(&parse(args)), "{args:?} checks");
        }
    }

    #[test]
    fn the_estate_commands_are_the_ones_that_bind() {
        let bound: BTreeSet<&str> = BINDING_SITES.iter().flat_map(|s| s.iter().copied()).collect();
        let claimed: BTreeSet<&str> = IDENTITIES
            .iter()
            .filter(|(_, i)| {
                matches!(
                    i,
                    Identity::EstateSa | Identity::HumanOrEstate(_) | Identity::EstateSaWhenItRuns(_)
                )
            })
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            bound, claimed,
            "IDENTITIES and BINDING_SITES disagree about which commands run as the estate's \
             service account"
        );

        // One binding call per site, plus each function's own definition. Two
        // functions, because a command given an estate derives the identity from
        // its params and a command given only the emitted directory reads it out
        // of the artefact `tofu` reads.
        let src = include_str!("main.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        let calls = body.matches("configure_estate_impersonation(").count() - 1
            + body.matches("configure_emitted_impersonation(").count()
            - 1;
        assert_eq!(
            calls,
            BINDING_SITES.len(),
            "{} dispatch sites are declared in BINDING_SITES but the code binds at {} — either a \
             command was added without binding, or a binding was added without saying which \
             commands it serves",
            BINDING_SITES.len(),
            calls
        );
    }

    #[test]
    fn command_groups_cover_the_cli() {
        let mut cmd = Cli::command();
        cmd.build(); // `help` is generated here, and the table lists it too
        // a hidden command is a command the help does not show, so none is exempt
        let cli: BTreeSet<&str> = cmd.get_subcommands().map(|s| s.get_name()).collect();
        let table: BTreeSet<&str> = declared().into_iter().collect();

        let missing: Vec<_> = cli.difference(&table).collect();
        let unknown: Vec<_> = table.difference(&cli).collect();
        assert!(
            missing.is_empty(),
            "these commands exist but no COMMAND_GROUPS group names them, so `satz --help` \
             would not print them: {missing:?} — put each one in a group in src/main.rs"
        );
        assert!(
            unknown.is_empty(),
            "COMMAND_GROUPS names commands the CLI does not have: {unknown:?}"
        );
    }

    /// `satz --help` lists the global options; a command's own help does not repeat
    /// them, at any depth, whichever way the help is asked for.
    #[test]
    fn a_command_s_help_leaves_the_global_options_to_the_root() {
        let mut cmd = cli_command();
        cmd.build();
        let globals: Vec<String> = Cli::command()
            .get_arguments()
            .filter(|a| a.is_global_set())
            .map(|a| a.get_id().to_string())
            .collect();
        assert!(globals.len() >= 5, "found only {globals:?}");
        fn walk(cmd: &mut clap::Command, globals: &[String], checked: &mut usize) {
            for sub in cmd.get_subcommands_mut() {
                let help = sub.render_long_help().to_string();
                assert!(!help.contains("Global options"), "`satz {} --help` repeats the global options", sub.get_name());
                for id in globals {
                    if let Some(arg) = sub.get_arguments().find(|a| a.get_id() == id.as_str()) {
                        assert!(arg.is_hide_set(), "`{id}` shows under `{}`", sub.get_name());
                    }
                }
                *checked += 1;
                walk(sub, globals, checked);
            }
        }
        let mut checked = 0;
        walk(&mut cmd, &globals, &mut checked);
        assert!(checked >= 30, "checked only {checked} commands");
    }

    /// Hiding changes the help and nothing else: a global option after the command
    /// still reaches the root, as it did before.
    #[test]
    fn a_global_option_after_a_command_still_takes_effect() {
        for argv in [
            ["satz", "transpile", "x.satz", "--check", "--config", "estate-dir"],
            ["satz", "--config", "estate-dir", "transpile", "x.satz", "--check"],
        ] {
            let matches = cli_command().try_get_matches_from(argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
            let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches).unwrap();
            assert_eq!(cli.config.as_deref(), Some(Path::new("estate-dir")), "{argv:?}");
        }
        // and a command's --help is still its help, not a missing-argument refusal
        let err = cli_command().try_get_matches_from(["satz", "prowler", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp, "{err}");
    }

    #[test]
    fn no_command_is_in_two_groups() {
        let declared = declared();
        let unique: BTreeSet<&str> = declared.iter().copied().collect();
        assert_eq!(
            declared.len(),
            unique.len(),
            "a command appears in more than one COMMAND_GROUPS group, so the help would list it twice"
        );
    }
}

// Presets ship to users verbatim via `get-presets`, so a malformed one reaches everybody.



#[cfg(test)]
mod satz_vars_parity {
    //! The org-policy commands read an estate's variable table. Until v0.40 they
    //! got it by compiling the estate to a `.gen.yaml` twin and parsing the
    //! `variables:` block back out; they now read it from the fragment pipeline.
    //!
    //! Both routes must agree, because a divergence would be invisible: the
    //! commands would silently address a different org, or resolve a preset
    //! against different values than the ones `transpile` emits. This pins the
    //! two together — including the snake_case -> kebab-case rename, which lives
    //! in the Satz YAML emitter and therefore has to be reproduced by hand on the
    //! pipeline side.
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write fixture");
        p
    }

    #[test]
    fn both_routes_agree_on_the_variable_table() {
        let dir = std::env::temp_dir().join(format!("satz-vars-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        // first-definition-wins: the estate's widget_name must beat the pack's,
        // and the pack's own params must still reach the table.
        let estate = write(
            &dir,
            "estate.satz",
            "estate vars_parity\n\n\
             params {\n\
             \x20 customer_organization_id = \"123456789\"\n\
             \x20 customer_domain = \"example.com\"\n\
             \x20 widget_name = \"overridden-widget\"\n\
             }\n\n\
             use \"pack.satz\"\n\n\
             google_organization_iam_member {\n\
             \x20 use \"grants.satz\"\n\
             }\n",
        );
        write(
            &dir,
            "pack.satz",
            "pack vars_parity_widgets\n\n\
             params {\n\
             \x20 widget_name = \"default-widget\"\n\
             \x20 widget_location = \"europe-west3\"\n\
             \x20 contact_email = \"ops@{customer_domain}\"\n\
             }\n",
        );
        // A grant pack used from inside a resource map. This is the shape that
        // exposed the classification bug: a params-only walk that calls
        // google_organization_iam_member an Entity rejects the whole file.
        write(
            &dir,
            "grants.satz",
            "pack vars_parity_grants\n\n\
             params {\n\
             \x20 admins_group = \"gcp-organization-admins\"\n\
             }\n\n\
             \"group:{admins_group}@{customer_domain}\" = [\n\
             \x20 \"roles/resourcemanager.organizationAdmin\",\n\
             ]\n",
        );

        let via_pipeline = satz_estate_params(&estate, &[]).expect("pipeline route");

        // The facts that matter: kebab keys reach the table, first-definition-wins
        // held, and interpolation was resolved. (This used to also compare against
        // the YAML-twin route; that route is retired with the walk — M5.)
        assert_eq!(
            via_pipeline.get("customer-organization-id").and_then(|v| v.as_str()),
            Some("123456789")
        );
        assert_eq!(via_pipeline.get("widget-name").and_then(|v| v.as_str()), Some("overridden-widget"));
        assert_eq!(via_pipeline.get("widget-location").and_then(|v| v.as_str()), Some("europe-west3"));
        assert_eq!(via_pipeline.get("contact-email").and_then(|v| v.as_str()), Some("ops@example.com"));
        assert_eq!(
            via_pipeline.get("admins-group").and_then(|v| v.as_str()),
            Some("gcp-organization-admins"),
            "a grant pack's params must reach the table — the walk has to classify \
             google_organization_iam_member as a grant map to get that far"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}


#[cfg(test)]
mod corpus {
    //! The corpus: every composition scenario battle-proven in the field,
    //! snapshot-gated (`tests/corpus/<case>/expected.sorted.txt`). This is the
    //! contract any refactor of composition semantics must honor byte-for-byte
    //! on sorted output. Regenerate deliberately with UPDATE_CORPUS=1 and review
    //! the snapshot diff like production code.
    use super::*;
    use std::path::Path;

    /// The corpus schema fixture — a real provider schema trimmed to the types
    /// the fixtures use. The corpus classifies types through THIS, the same way
    /// production does, instead of a hand-written table guessing at what a
    /// resource is.
    pub(super) fn schema_dir() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas").to_string_lossy().into_owned()
    }

    pub(super) fn registry() -> ResourceRegistry {
        ResourceRegistry::load_all(&schema_dir()).expect("corpus schema fixture")
    }

    pub(super) fn sorted_lines(s: &str) -> Vec<String> {
        let mut v: Vec<String> =
            s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
        v.sort_unstable();
        v
    }

    /// Compile `<case>/main.satz` through the fragment pipeline, the way
    /// `transpile` does, and return
    /// `sorted(main.tf) ---tfvars--- sorted(tfvars) ---imports--- sorted(imports.tf)`:
    /// an `"import-id"` is emission too, and one written as an interpolation
    /// must reach `imports.tf` as the literal.
    pub(super) fn run_case(case: &Path) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(case.join("main.satz")).unwrap();
        let reg = registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("main.satz", &src, &resolver, &|p| {
            std::fs::read_to_string(case.join(p))
                .or_else(|_| std::fs::read_to_string(root.join(p)))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{}: front-end failed: {}", name, e));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(
            folded.conflicts().is_empty(),
            "{}: conflicts on a conflict-free case: {:?}",
            name,
            folded.conflicts()
        );
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        // Same as production: without the registry the emitter drops
        // schema-derived detail (it once silently lost every alert policy's
        // notification_channels).
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).unwrap_or_else(|e| panic!("{}: emit failed: {}", name, e));
        format!(
            "{}\n---tfvars---\n{}\n---imports---\n{}",
            sorted_lines(&out.main_tf).join("\n"),
            sorted_lines(&crate::emitter::emit_tfvars(&fe.tfvars)).join("\n"),
            sorted_lines(&out.imports_tf).join("\n")
        )
    }

    /// THE corpus gate: every case's emission must reproduce its snapshot.
    #[test]
    fn every_case_reproduces_its_snapshot() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
        let mut cases: Vec<_> = std::fs::read_dir(&corpus)
            .expect("corpus dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("main.satz").exists())
            .collect();
        cases.sort();
        assert!(!cases.is_empty(), "no corpus case found");
        for case in cases {
            let name = case.file_name().unwrap().to_string_lossy().to_string();
            let got = run_case(&case);
            let expected_path = case.join("expected.sorted.txt");
            if std::env::var("UPDATE_CORPUS").is_ok() {
                std::fs::write(&expected_path, &got).unwrap();
                eprintln!("{}: snapshot regenerated — review the diff", name);
            }
            let expected = std::fs::read_to_string(&expected_path).unwrap();
            assert_eq!(expected, got, "{}: emission diverged from the snapshot", name);
        }
    }
}

#[cfg(test)]
mod placement_gate {
    //! What a `use` places, and what it never emits, through the emitter — the rules
    //! that hold by construction and that nothing pinned: a pack is used at the top level
    //! and its resources land at the organisation, a pack that creates a project names the
    //! folder it is created in, an organisation-level resource hoists to the organisation
    //! from every position an author may write it, a folder or a project written in a
    //! project's body is refused, and a pack's `params`, `question` and `claim` statements
    //! reach the estate and never `main.tf`.
    use super::*;

    const PACK: &str = r#"pack hosting version "1.0"

params {
  host_project_id     = "acme-host-001"
  host_project_folder = ""
  host_is_wanted      = true
}

question host_is_wanted {
  prompt   = "Is the hosting project wanted here?"
  reversal = edit
  blast    = low
}

claim "cis-gcp" "4.0" "2.2" contributes {
  resources = ["google_project.host"]
}

google_project {
  host {
    name            = host_project_id
    project_id      = host_project_id
    folder_id       = host_project_folder
    billing_account = "012345-6789AB-CDEF01"
  }
}
"#;

    /// A pack of CONTENT, no node of its own: one project-scoped resource, one whose
    /// scope is a Resource Manager path, and one that belongs to the organisation
    /// whatever encloses it. `CONTENT_BODY` is the same three maps without the header, so
    /// the same resources can be written by hand where a `use` no longer stands.
    const CONTENT_BODY: &str = r#"google_storage_bucket {
  evidence {
    name                        = "acme-evidence-001"
    location                    = "EU"
    uniform_bucket_level_access = true
  }
}

google_org_policy_policy {
  os_login {
    name = "compute.requireOsLogin"
    spec {
      rules = [
        { enforce = "TRUE" },
      ]
    }
  }
}

google_organization_iam_member {
  "group:gcp-auditors@example.com" = ["roles/viewer"]
}
"#;

    /// The same three maps, project-scoped only: a project's body takes neither the
    /// organisation grant nor anything else that hangs off something above the project.
    const CONTENT_BODY_IN_A_PROJECT: &str = r#"google_storage_bucket {
  evidence {
    name                        = "acme-evidence-001"
    location                    = "EU"
    uniform_bucket_level_access = true
  }
}

google_org_policy_policy {
  os_login {
    name = "compute.requireOsLogin"
    spec {
      rules = [
        { enforce = "TRUE" },
      ]
    }
  }
}
"#;

    fn content_pack() -> String {
        format!("pack content version \"1.0\"\n\n{}", CONTENT_BODY)
    }

    fn load(p: &str) -> Result<String, String> {
        match p {
            "hosting.satz" => Ok(PACK.to_string()),
            "content.satz" => Ok(content_pack()),
            other => Err(format!("no load: {}", other)),
        }
    }

    fn try_emit(estate: &str) -> Result<(String, String, satz_core::pipeline::FrontEnd), String> {
        let reg = corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("main.satz", estate, &resolver, &load)
            .map_err(|e| e.to_string())?;
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty(), "{:?}", folded.conflicts());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).map_err(|e| format!("emit: {}", e))?;
        Ok((out.main_tf, crate::emitter::emit_tfvars(&fe.tfvars), fe))
    }

    fn emit(estate: &str) -> (String, String, satz_core::pipeline::FrontEnd) {
        try_emit(estate).unwrap_or_else(|e| panic!("front end: {}", e))
    }

    /// The one resource block of `main_tf` with this type and label — the label as a
    /// prefix, so a grant found by its hashed label is named by what it grants.
    fn block(main_tf: &str, tf_type: &str, label: &str) -> String {
        let head = format!("\"{}\" \"{}", tf_type, label);
        format!("\n{}", main_tf)
            .split("\nresource ")
            .find(|r| r.starts_with(&head))
            .map(str::to_string)
            .unwrap_or_else(|| panic!("{}.{} is not in main.tf:\n{}", tf_type, label, main_tf))
    }

    const HEAD: &str = "estate t\n\nparams {\n  customer_organization_id = \"123456789012\"\n}\n\n";

    fn in_folder() -> String {
        format!("google_folder {{\n  shared {{\n    display_name = \"Shared\"\n\n{}  }}\n}}\n", CONTENT_BODY)
    }

    fn in_project() -> String {
        format!(
            "google_project {{\n  outer {{\n    name            = \"acme-outer-001\"\n    project_id      = \"acme-outer-001\"\n    \
             billing_account = \"012345-6789AB-CDEF01\"\n\n{}  }}\n}}\n",
            CONTENT_BODY_IN_A_PROJECT
        )
    }

    #[test]
    fn a_pack_is_used_at_the_top_level_and_its_statements_never_reach_main_tf() {
        let (main_tf, tfvars, fe) = emit(&format!("{}{}", HEAD, "use \"hosting.satz\"\n"));
        let host = block(&main_tf, "google_project", "host");
        assert!(host.contains("org_id = \"123456789012\""), "a pack used at the top level creates its project at the organisation:\n{}", host);
        // the statements: in the estate, resolved — and nowhere in the HCL
        assert_eq!(fe.questions.len(), 1);
        assert_eq!(fe.questions[0].questions[0].subject, "host_is_wanted");
        assert_eq!(fe.claims.len(), 1);
        assert!(tfvars.contains("host-project-id = \"acme-host-001\""), "the param is a variable with its value:\n{}", tfvars);
        for word in ["question", "prompt", "Is the hosting project wanted", "reversal", "blast", "params", "claim", "cis-gcp", "host_is_wanted"] {
            assert!(!main_tf.contains(word), "`{}` of the used pack reached main.tf:\n{}", word, main_tf);
        }
    }

    /// A folder's and a project's body hold the estate's own resources (ADR 0046). The
    /// refusal names the line, the node and the edit — the param to bind, because the
    /// folder was what put a pack's project there.
    #[test]
    fn a_use_in_the_body_of_a_folder_or_a_project_is_refused() {
        for (form, tail, node, advice) in [
            (
                "a folder's body",
                "google_folder {\n  shared {\n    display_name = \"Shared\"\n    use \"hosting.satz\"\n  }\n}\n".to_string(),
                "google_folder.shared",
                "bind that param to `google_folder.shared.name`",
            ),
            (
                "a project's body",
                "google_project {\n  outer {\n    name            = \"acme-outer-001\"\n    project_id      = \"acme-outer-001\"\n    \
                 billing_account = \"012345-6789AB-CDEF01\"\n    use \"hosting.satz\"\n  }\n}\n"
                    .to_string(),
                "google_project.outer",
                "names the project itself",
            ),
        ] {
            let Err(err) = try_emit(&format!("{}{}", HEAD, tail)) else {
                panic!("{}: a `use` was accepted there", form);
            };
            assert!(err.contains(&format!("stands in the body of `{}`", node)), "{}: {}", form, err);
            assert!(err.contains("a pack is used at the top level of a file"), "{}: {}", form, err);
            assert!(err.contains("Move the line to the top level"), "{}: {}", form, err);
            assert!(err.contains(advice), "{}: the refusal does not name the edit — {}", form, err);
            assert!(
                err.contains("`logsink_project_folder`") && err.contains("`mdc_mgmt_project_folder`"),
                "{}: the refusal names the two packs whose project the folder placed — {}",
                form,
                err
            );
        }
    }

    /// A pack's project names the folder it is created in, and that param is what an
    /// enclosure used to say. The proof is the project written BY HAND inside the folder:
    /// the two emit the same `google_project` block, so an estate that moves a pack's line
    /// to the top level and binds the param plans nothing.
    #[test]
    fn a_pack_s_project_is_created_in_the_folder_its_param_names() {
        let bare = concat!(
            "estate t\n\nparams {\n",
            "  customer_organization_id = \"123456789012\"\n",
            "  host_project_folder      = \"google_folder.shared.name\"\n",
            "}\n\n",
            "google_folder {\n  shared {\n    display_name = \"Shared\"\n  }\n}\n\n",
            "use \"hosting.satz\"\n"
        );
        let by_hand = format!(
            "{}{}",
            HEAD,
            "google_folder {\n  shared {\n    display_name = \"Shared\"\n    google_project {\n      host {\n        \
             name            = \"acme-host-001\"\n        project_id      = \"acme-host-001\"\n        \
             billing_account = \"012345-6789AB-CDEF01\"\n      }\n    }\n  }\n}\n"
        );
        let (bare_tf, _, _) = emit(bare);
        let (by_hand_tf, _, _) = emit(&by_hand);
        assert_eq!(
            block(&bare_tf, "google_project", "host"),
            block(&by_hand_tf, "google_project", "host"),
            "the param does not reproduce what the enclosure wrote"
        );
        assert!(
            block(&bare_tf, "google_project", "host").contains("folder_id = google_folder.shared.name"),
            "a folder named as a dotted path is a reference, not a quoted string:\n{}",
            bare_tf
        );

        // The default says nothing, so a pack that names no folder creates its project at
        // the organisation.
        let (top_tf, _, _) = emit(&format!("{}{}", HEAD, "use \"hosting.satz\"\n"));
        let host = block(&top_tf, "google_project", "host");
        assert!(host.contains("org_id = \"123456789012\""), "{}", host);
        assert!(!host.contains("folder_id"), "an empty folder_id is not emitted:\n{}", host);
    }

    /// Per SCOPE, where a resource lands: a pack used at the top level reaches the
    /// organisation, and a node's body places what it encloses — including an org policy,
    /// whose parent is the Resource Manager path `projects/<id>` and not the bare id a
    /// project reference gives. An organisation-level resource keeps the organisation
    /// from every position an author may write it in: that hoist is what lets an author
    /// write one beside the project it serves.
    #[test]
    fn a_nodes_body_places_what_it_encloses_and_an_organisation_level_resource_hoists() {
        let forms = [
            (
                "a pack used at the top level",
                "use \"content.satz\"".to_string(),
                None,
                "\"organizations/123456789012\"",
                "\"organizations/123456789012/policies/compute.requireOsLogin\"",
                true,
            ),
            (
                "written in a folder's body",
                in_folder(),
                None,
                "google_folder.shared.name",
                "\"${google_folder.shared.name}/policies/compute.requireOsLogin\"",
                true,
            ),
            (
                "written in a project's body",
                in_project(),
                Some("project = google_project.outer.project_id"),
                "\"projects/${google_project.outer.project_id}\"",
                "\"projects/${google_project.outer.project_id}/policies/compute.requireOsLogin\"",
                false,
            ),
        ];
        for (form, tail, bucket_project, policy_parent, policy_name, grants) in forms {
            let (main_tf, _, _) = emit(&format!("{}{}", HEAD, tail));

            // project-scoped: the project the `use` stands in, or none at all
            let bucket = block(&main_tf, "google_storage_bucket", "evidence");
            match bucket_project {
                Some(p) => assert!(bucket.contains(p), "{}: the bucket is not in the project it stands in:\n{}", form, bucket),
                None => assert!(!bucket.contains("project ="), "{}: the bucket took a project from nowhere:\n{}", form, bucket),
            }

            // a Resource Manager path: organisation, folder or project, in that form
            let policy = block(&main_tf, "google_org_policy_policy", "os_login");
            assert!(
                policy.contains(&format!("parent = {}", policy_parent)),
                "{}: the policy's parent is not the node it stands in:\n{}",
                form,
                policy
            );
            assert!(
                policy.contains(&format!("name = {}", policy_name)),
                "{}: the policy's name is not built from its parent:\n{}",
                form,
                policy
            );

            // organisation-level: the organisation, from every position it may stand in
            if grants {
                let grant = block(&main_tf, "google_organization_iam_member", "iam_group_gcp_auditors_example_com_");
                assert!(grant.contains("org_id = \"123456789012\""), "{}: the organisation grant did not hoist:\n{}", form, grant);
                assert!(grant.contains("provider = google.google"), "{}: the organisation grant took a project's provider:\n{}", form, grant);
            }
        }
    }

    /// A type that belongs above a project, written IN a project's body. It used to be
    /// emitted at the organisation, which reads as "in this project" and is not.
    /// The same type at a file's top level is the hoist the rule above proves, so the
    /// refusal is of the position, never of the type.
    #[test]
    fn a_type_that_belongs_above_a_project_is_refused_in_its_body() {
        for (block, belongs) in [
            ("google_folder {\n      inner {\n        display_name = \"Inner\"\n      }\n    }", "a folder hangs off the organisation or another folder"),
            ("google_project {\n      inner {\n        project_id = \"acme-inner-001\"\n      }\n    }", "a project hangs off the organisation or a folder"),
            ("google_organization_iam_member {\n      \"group:gcp-auditors@example.com\" = [\"roles/viewer\"]\n    }", "it belongs to the organisation"),
            ("google_cloud_identity_group {\n      \"log-admins\" { display_name = \"Log Admins\" }\n    }", "it belongs to the Cloud Identity customer"),
            ("google_billing_account_iam_member {\n      \"group:billing@example.com\" = [\"roles/billing.admin\"]\n    }", "it belongs to the billing account"),
        ] {
            let estate = format!(
                "{}google_project {{\n  outer {{\n    name            = \"acme-outer-001\"\n    project_id      = \"acme-outer-001\"\n    billing_account = \"012345-6789AB-CDEF01\"\n    {}\n  }}\n}}\n",
                HEAD, block
            );
            let Err(err) = try_emit(&estate) else {
                panic!("accepted in a project's body:\n{}", estate);
            };
            assert!(err.contains("stands in the body of a `google_project`"), "{}", err);
            assert!(err.contains(belongs), "{}", err);
        }

        // the same types at the top level of a file, beside the project that file declares
        let estate = format!(
            "{}google_project {{\n  outer {{\n    name            = \"acme-outer-001\"\n    project_id      = \"acme-outer-001\"\n    billing_account = \"012345-6789AB-CDEF01\"\n  }}\n}}\n\ngoogle_organization_iam_member {{\n  \"group:gcp-auditors@example.com\" = [\"roles/viewer\"]\n}}\n",
            HEAD
        );
        let (main_tf, _, _) = emit(&estate);
        let grant = block(&main_tf, "google_organization_iam_member", "iam_group_gcp_auditors_example_com_");
        assert!(grant.contains("org_id = \"123456789012\""), "the grant beside the project did not reach the organisation:\n{}", grant);
    }
}

#[cfg(test)]
mod yaml_estate_gate {
    //! THE gate for the legacy YAML dialect. The dialect is migration input
    //! only (owner, 2026-08-29): nothing transpiles it, `satz import <file>.yaml`
    //! converts it. So what must keep working is the CONVERSION — the fixture
    //! and its `!include` pack become Satz that compiles through the fragment
    //! pipeline and declares every resource the YAML declared.
    //!
    //! Self-contained: no schema registry, no live org, no customer repo.
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/yaml-estate")
    }

    /// Addresses the fixture declares. Written out rather than counted, because
    /// the failure mode being guarded is resources DISAPPEARING — a count is
    /// satisfied by the wrong set, and "it got smaller" was exactly the bug.
    const EXPECTED: &[&str] = &[
        "google_org_policy_policy.compute_requireOsLogin",
        "google_organization_iam_member.",
        "google_folder.infra_folder",
        "google_project.demo_project",
        "google_project_iam_member.",
    ];

    /// The fixture's type table: an explicit ALLOWLIST standing in for the
    /// provider schemas. Not "anything starting with `google_`" — that claims
    /// `google_labels` exists and sends a genuine `labels { … }` attribute
    /// block down the resource path.
    struct FixtureTypes;
    impl FixtureTypes {
        fn known(&self, t: &str) -> bool {
            matches!(
                t,
                "google_org_policy_policy"
                    | "google_organization_iam_member"
                    | "google_project_iam_member"
                    | "google_folder"
                    | "google_project"
            )
        }
    }
    impl satz_core::pipeline::TypeResolver for FixtureTypes {
        fn resolve(&self, key: &str) -> Option<satz_core::pipeline::ResolvedType> {
            if !self.known(key) {
                return None;
            }
            let (class, scope) = satz_core::pipeline::type_facts(key);
            Some(satz_core::pipeline::ResolvedType { tf_type: key.to_string(), class, scope })
        }
    }
    impl satz_core::algebra::TypeTable for FixtureTypes {
        fn merge_class(&self, t: &str) -> satz_core::MergeClass {
            satz_core::pipeline::type_facts(t).0
        }
        fn scope(&self, t: &str) -> satz_core::Scope {
            satz_core::pipeline::type_facts(t).1
        }
    }

    #[test]
    fn a_converted_yaml_estate_compiles_and_declares_every_resource() {
        let src_dir = fixture();
        let tmp = std::env::temp_dir().join(format!("satz-yaml-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let is_type = |t: &str| FixtureTypes.known(t);
        let exists = |p: &str| tmp.join(p).exists();

        for (file, kind, name) in
            [("pack.yaml", "pack", "yaml_gate_pack"), ("main.yaml", "estate", "yaml_gate")]
        {
            let src = std::fs::read_to_string(src_dir.join(file)).unwrap();
            let satz = satz_core::migrate::convert(&src, kind, name)
                .unwrap_or_else(|e| panic!("{} failed to convert: {}", file, e));
            let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
            let satz = satz_core::migrate::retarget_uses(&satz, &exists);
            std::fs::write(tmp.join(file.replace(".yaml", ".satz")), &satz).unwrap();
        }

        let converted = std::fs::read_to_string(tmp.join("main.satz")).unwrap();
        assert!(
            converted.contains("use \"pack.satz\""),
            "the converted estate must point at the converted pack, not the YAML:\n{}",
            converted
        );
        assert!(
            converted.contains("google_org_policy_policy"),
            "shorthand keys must gain the provider prefix:\n{}",
            converted
        );

        let tmp_for_load = tmp.clone();
        let fe = satz_core::pipeline::compile_estate(
            "main.satz",
            &converted,
            &FixtureTypes,
            &|p| std::fs::read_to_string(tmp_for_load.join(p)).map_err(|e| e.to_string()),
        )
        .unwrap_or_else(|e| panic!("converted estate does not compile as Satz: {:?}", e));
        let folded = satz_core::pipeline::fold_fragments(&FixtureTypes, &fe.fragments);
        assert!(folded.conflicts().is_empty(), "conflicts: {:?}", folded.conflicts());
        let ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs: Vec<String> = out.manifest.addresses().into_iter().collect();
        for want in EXPECTED {
            assert!(
                addrs.iter().any(|a| a.starts_with(want)),
                "{} missing after conversion — declared resources must never be \
                 silently dropped.\ngot: {:?}",
                want,
                addrs
            );
        }
        // `labels` is an ATTRIBUTE of the project, not a resource of its own.
        assert!(
            !addrs.iter().any(|a| a.contains("google_labels")),
            "a nested attribute block was emitted as a resource: {:?}",
            addrs
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The Tier-2 spelling: org policies as a SEQUENCE identified by
    /// `constraint:`, at the top level and nested in a project. Every estate
    /// still on the dialect writes them this way, and the converter took none
    /// of them — the sequence printed as a top-level attribute, which does not
    /// compile. Guarded here because the fleet is the only other place it would
    /// have shown up, one estate at a time, years after the fact.
    #[test]
    fn the_tier2_org_policy_list_form_converts_to_addressed_resources() {
        let src = std::fs::read_to_string(fixture().join("tier2.yaml")).unwrap();
        let is_type = |t: &str| FixtureTypes.known(t);
        let satz = satz_core::migrate::convert(&src, "estate", "tier2_gate")
            .unwrap_or_else(|e| panic!("the Tier-2 fixture failed to convert: {}", e));
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);

        // The address is the constraint, dots to dashes — the spelling the
        // preset library uses, so a converted estate and the pack that later
        // replaces it name the same resource.
        for want in [
            "\"iam-disableServiceAccountKeyCreation\"",
            "\"iam-managed-disableServiceAccountKeyUpload\"",
            "\"gcp-resourceLocations\"",
            "\"compute-skipDefaultNetworkCreation\"",
        ] {
            assert!(satz.contains(want), "{} missing from the conversion:\n{}", want, satz);
        }
        // Comments are excluded: the fixture's own header explains this failure
        // mode and would otherwise match the text it warns about.
        let code = |needle: &str| {
            satz.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(needle))
        };
        assert!(
            !code("org_policy_policy ="),
            "the list form printed as an attribute instead of a block:\n{}",
            satz
        );
        // `type: list` is the dialect's own marker; the provider has no such
        // attribute, so an estate carrying it would not validate.
        assert!(!code("type = \"list\""), "the dialect-only `type:` marker reached the estate:\n{}", satz);

        let fe = satz_core::pipeline::compile_estate(
            "tier2.satz",
            &satz,
            &FixtureTypes,
            &|p: &str| Err(format!("the Tier-2 fixture includes nothing, asked for {}", p)),
        )
        .unwrap_or_else(|e| panic!("the converted Tier-2 estate does not compile: {:?}", e));
        let folded = satz_core::pipeline::fold_fragments(&FixtureTypes, &fe.fragments);
        assert!(folded.conflicts().is_empty(), "conflicts: {:?}", folded.conflicts());
        let ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs: Vec<String> = out.manifest.addresses().into_iter().collect();
        for want in [
            "google_org_policy_policy.iam_disableServiceAccountKeyCreation",
            "google_org_policy_policy.iam_managed_disableServiceAccountKeyUpload",
            "google_org_policy_policy.gcp_resourceLocations",
            "google_org_policy_policy.compute_skipDefaultNetworkCreation",
            "google_folder.infra_folder",
            "google_project.infra",
        ] {
            assert!(
                addrs.iter().any(|a| a == want),
                "{} missing after conversion — declared resources must never be \
                 silently dropped.\ngot: {:?}",
                want,
                addrs
            );
        }
    }
}

#[cfg(test)]
mod manifest_gate {
    //! The emission manifest replaced four line scanners over `main.tf`
    //! (`emitted_addresses`, `extract_witness_attrs`, `declared_enforcement`,
    //! `declared_org_policies`). The scanners survive here as oracles: over
    //! every corpus case the manifest must say exactly what they said, so the
    //! compliance plane's verdicts cannot move. The one intended difference is
    //! pinned separately — raw `hcl { … }` passthrough is not in the manifest.
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    fn legacy_addresses(main_tf: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in main_tf.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("resource \"") {
                let mut parts = rest.split('"');
                let tf_type = parts.next().unwrap_or("");
                parts.next();
                let label = parts.next().unwrap_or("");
                if !tf_type.is_empty() && !label.is_empty() {
                    out.insert(format!("{}.{}", tf_type, label));
                }
            }
        }
        out
    }

    fn legacy_witness_attrs(main_tf: &str) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        let mut depth = 0usize;
        for line in main_tf.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("resource \"") {
                let mut parts = rest.split('"');
                let tf_type = parts.next().unwrap_or("");
                parts.next();
                let label = parts.next().unwrap_or("");
                current = Some(format!("{}.{}", tf_type, label));
                out.entry(current.clone().unwrap()).or_default();
                depth = 1;
                continue;
            }
            if current.is_none() {
                continue;
            }
            if t == "}" || t == "}," {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    current = None;
                }
                continue;
            }
            if depth == 1 {
                if let (Some(addr), Some(eq)) = (&current, t.find(" = \"")) {
                    let key = t[..eq].trim().to_string();
                    let val = t[eq + 4..].trim_end_matches('"').to_string();
                    if !key.contains(' ') && !val.contains('"') {
                        out.get_mut(addr).unwrap().insert(key, val);
                    }
                }
            }
            if t.ends_with('{') {
                depth += 1;
            }
        }
        out
    }

    fn legacy_enforcement(main_tf: &str) -> BTreeMap<String, bool> {
        let mut out = BTreeMap::new();
        let mut current: Option<String> = None;
        let mut depth = 0usize;
        let mut found: Vec<bool> = Vec::new();
        for line in main_tf.lines() {
            let t = line.trim();
            if current.is_none() {
                if let Some(rest) = t.strip_prefix(r#"resource "google_org_policy_policy" ""#) {
                    if let Some(label) = rest.split('"').next() {
                        current = Some(format!("google_org_policy_policy.{}", label));
                        depth = t.matches('{').count() - t.matches('}').count();
                        found.clear();
                    }
                }
                continue;
            }
            depth = depth + t.matches('{').count() - t.matches('}').count();
            if let Some(v) = t.strip_prefix("enforce") {
                match v.trim_start_matches([' ', '=']).trim().trim_matches('"').to_ascii_uppercase().as_str() {
                    "TRUE" => found.push(true),
                    "FALSE" => found.push(false),
                    _ => {}
                }
            }
            if depth == 0 {
                if let (Some(addr), [only]) = (current.take(), found.as_slice()) {
                    out.insert(addr, *only);
                }
                found.clear();
            }
        }
        out
    }

    /// (address, constraint, parent, enforce) — the old `declared_org_policies`.
    fn legacy_org_policies(main_tf: &str) -> Vec<(String, String, String, Option<bool>)> {
        let mut out = Vec::new();
        let mut cur: Option<(String, String, String, Vec<bool>)> = None;
        let mut depth = 0usize;
        for line in main_tf.lines() {
            let t = line.trim();
            if cur.is_none() {
                if let Some(rest) = t.strip_prefix(r#"resource "google_org_policy_policy" ""#) {
                    if let Some(label) = rest.split('"').next() {
                        cur = Some((format!("google_org_policy_policy.{}", label), String::new(), String::new(), Vec::new()));
                        depth = t.matches('{').count() - t.matches('}').count();
                    }
                }
                continue;
            }
            depth = depth + t.matches('{').count() - t.matches('}').count();
            if let Some((_, name, parent, enf)) = cur.as_mut() {
                if let Some(v) = t.strip_prefix("name") {
                    let v = v.trim_start_matches([' ', '=']).trim().trim_matches('"');
                    if !v.is_empty() {
                        *name = crate::org_policy::constraint_name(v).to_string();
                    }
                } else if let Some(v) = t.strip_prefix("parent") {
                    let v = v.trim_start_matches([' ', '=']).trim().trim_matches('"');
                    if !v.is_empty() {
                        *parent = v.to_string();
                    }
                } else if let Some(v) = t.strip_prefix("enforce") {
                    match v.trim_start_matches([' ', '=']).trim().trim_matches('"').to_ascii_uppercase().as_str() {
                        "TRUE" => enf.push(true),
                        "FALSE" => enf.push(false),
                        _ => {}
                    }
                }
            }
            if depth == 0 {
                if let Some((address, constraint, parent, enf)) = cur.take() {
                    if !constraint.is_empty() {
                        out.push((address, constraint, parent, if enf.len() == 1 { Some(enf[0]) } else { None }));
                    }
                }
            }
        }
        out
    }

    pub(super) fn emit_case(case: &Path, reg: &crate::ResourceRegistry) -> (crate::emitter::EmitOut, satz_core::pipeline::FrontEnd) {
        let src = std::fs::read_to_string(case.join("main.satz")).unwrap();
        let case_dir = case.to_path_buf();
        let resolver = crate::EstateResolver { registry: reg };
        let fe = satz_core::pipeline::compile_estate("main.satz", &src, &resolver, &|p| {
            std::fs::read_to_string(case_dir.join(p))
                .or_else(|_| std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p)))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{}: front-end failed: {}", case.display(), e));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(reg);
        let out = crate::emitter::emit(&folded, &ctx).unwrap_or_else(|e| panic!("{}: emit failed: {}", case.display(), e));
        (out, fe)
    }

    #[test]
    fn manifest_says_exactly_what_the_text_scanners_said() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
        let reg = super::corpus::registry();
        let mut checked = 0;
        for entry in std::fs::read_dir(&corpus).unwrap().flatten() {
            let case = entry.path();
            if !case.join("main.satz").exists() {
                continue;
            }
            let name = case.file_name().unwrap().to_string_lossy().to_string();
            let (out, _) = emit_case(&case, &reg);
            let m = &out.manifest;
            assert_eq!(m.addresses(), legacy_addresses(&out.main_tf), "{}: addresses", name);
            // Witness attrs: the scanner skipped any value with an escaped quote
            // in it (log filters); the manifest keeps them. Everything the
            // scanner saw the manifest must see identically, and the only
            // extras allowed are exactly those quote-bearing values.
            let got_attrs = m.witness_attrs();
            let legacy_attrs = legacy_witness_attrs(&out.main_tf);
            assert_eq!(
                got_attrs.keys().collect::<Vec<_>>(),
                legacy_attrs.keys().collect::<Vec<_>>(),
                "{}: witness attr addresses",
                name
            );
            let got_refs = m.witness_refs();
            for (addr, legacy) in &legacy_attrs {
                let got = &got_attrs[addr];
                for (k, v) in legacy {
                    // DELIBERATE divergence from the text scanner (R9): a value
                    // that is nothing but one interpolation is a REFERENCE, so
                    // the manifest files it under `refs`. The scanner could only
                    // ever see text, and reading `${…}` as a literal is what made
                    // adopt search live state for a string containing `${`.
                    if v.starts_with("${") && v.ends_with('}') && !v[2..].contains("${") {
                        let want = &v[2..v.len() - 1];
                        assert_eq!(
                            got_refs[addr].get(k).map(String::as_str),
                            Some(want),
                            "{}: {} {} should be a reference, not a literal",
                            name,
                            addr,
                            k
                        );
                        assert!(got.get(k).is_none(), "{}: {} {} must not also be an attr", name, addr, k);
                        continue;
                    }
                    assert_eq!(got.get(k), Some(v), "{}: {} {}", name, addr, k);
                }
                for (k, v) in got {
                    if !legacy.contains_key(k) {
                        assert!(v.contains('"'), "{}: {} {} is new and not a quote-bearing value: {}", name, addr, k, v);
                    }
                }
            }
            assert_eq!(m.declared_enforcement(), legacy_enforcement(&out.main_tf), "{}: enforcement", name);
            // What `adopt` reads for an org policy: address, bare constraint,
            // parent, single enforce — same tuple the old scanner produced.
            let mut got: Vec<_> = m
                .of_type("google_org_policy_policy")
                .filter(|r| r.attrs.get("name").is_some_and(|n| !n.is_empty()))
                .map(|r| {
                    (
                        r.address(),
                        crate::org_policy::constraint_name(r.attrs.get("name").unwrap()),
                        r.attrs.get("parent").cloned().unwrap_or_default(),
                        r.enforce,
                    )
                })
                .collect();
            got.sort();
            let mut want = legacy_org_policies(&out.main_tf);
            want.sort();
            assert_eq!(got, want, "{}: declared org policies", name);
            checked += 1;
        }
        assert!(checked >= 5, "corpus shrank to {} cases", checked);
    }

    /// The contract the scanners could not keep: raw HCL deploys, but no claim
    /// covers it. A resource that exists only inside `hcl { … }` reaches
    /// `main.tf` and never reaches the manifest.
    #[test]
    fn passthrough_is_emitted_but_is_not_a_witness() {
        let reg = super::corpus::registry();
        let tmp = std::env::temp_dir().join("satz-manifest-passthrough");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("main.satz"),
            r#"estate passthrough_case

params {
  customer_organization_id = "123456789012"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_storage_bucket {
  real { name = "real-bucket" location = "EU" }
}

hcl trust "test fixture" {
  resource "google_storage_bucket" "ghost" {
    name = "ghost-bucket"
  }
}
"#,
        )
        .unwrap();
        let (out, fe) = emit_case(&tmp, &reg);
        let main_tf = crate::append_hcl_passthrough(out.main_tf.clone(), &fe.hcl);
        assert!(main_tf.contains(r#"resource "google_storage_bucket" "ghost""#), "passthrough must deploy:\n{}", main_tf);
        let addrs = out.manifest.addresses();
        assert!(addrs.contains("google_storage_bucket.real"), "{:?}", addrs);
        assert!(!addrs.contains("google_storage_bucket.ghost"), "a passthrough resource is not a witness: {:?}", addrs);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod acyclic_gate {
    //! `depends_on` is ordering, and ordering that comes back to where it started
    //! is a plan `tofu` refuses in full — not one broken resource but an estate
    //! that cannot be applied at all. The emitter adds those edges itself
    //! (`order_after_project_services` and the three beside it), so the property
    //! belongs here rather than in the pass that happens to add the last edge:
    //! every case under `tests/corpus/` and `tests/iac/`, graphed over its
    //! references AND its emitted `depends_on`, is acyclic.
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    /// `google_project.infra.project_id` → `google_project.infra`.
    fn address_of(traversal: &str) -> Option<String> {
        let mut parts = traversal.trim().split('.');
        match (parts.next(), parts.next()) {
            (Some(t), Some(l)) if t.starts_with("google_") => Some(format!("{}.{}", t, l)),
            _ => None,
        }
    }

    fn graph(main_tf: &str) -> BTreeMap<String, BTreeSet<String>> {
        let body = hcl::parse(main_tf).expect("emitted HCL parses");
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for b in body.blocks() {
            if b.identifier() != "resource" {
                continue;
            }
            let [t, l] = b.labels() else { continue };
            let from = format!("{}.{}", t.as_str(), l.as_str());
            let to = edges.entry(from).or_default();
            for a in b.body().attributes() {
                match a.expr() {
                    hcl::Expression::Traversal(_) => {
                        if let Ok(rendered) = hcl::format::to_string(a.expr()) {
                            to.extend(address_of(&rendered));
                        }
                    }
                    hcl::Expression::Array(items) => {
                        for i in items {
                            if let (hcl::Expression::Traversal(_), Ok(rendered)) = (i, hcl::format::to_string(i)) {
                                to.extend(address_of(&rendered));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        edges
    }

    /// Iterative depth-first search with a colour map; the panic names the cycle.
    fn find_cycle(edges: &BTreeMap<String, BTreeSet<String>>) -> Option<Vec<String>> {
        let mut done: BTreeSet<&str> = BTreeSet::new();
        for root in edges.keys() {
            let mut path: Vec<&str> = Vec::new();
            let mut on_path: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<(&str, bool)> = vec![(root.as_str(), false)];
            while let Some((node, leaving)) = stack.pop() {
                if leaving {
                    on_path.remove(node);
                    path.pop();
                    done.insert(node);
                    continue;
                }
                if done.contains(node) {
                    continue;
                }
                if !on_path.insert(node) {
                    let start = path.iter().position(|n| *n == node).unwrap_or(0);
                    let mut cycle: Vec<String> = path[start..].iter().map(|n| n.to_string()).collect();
                    cycle.push(node.to_string());
                    return Some(cycle);
                }
                path.push(node);
                stack.push((node, true));
                for next in edges.get(node).into_iter().flatten() {
                    if let Some((k, _)) = edges.get_key_value(next.as_str()) {
                        stack.push((k.as_str(), false));
                    }
                }
            }
        }
        None
    }

    #[test]
    fn no_case_emits_a_dependency_cycle() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reg = super::corpus::registry();
        let mut cases = 0;
        for dir in ["tests/corpus", "tests/iac"] {
            for entry in std::fs::read_dir(root.join(dir)).expect(dir).flatten() {
                let case = entry.path();
                if !case.join("main.satz").exists() {
                    continue;
                }
                let (out, _) = super::manifest_gate::emit_case(&case, &reg);
                if let Some(cycle) = find_cycle(&graph(&out.main_tf)) {
                    panic!("{}: dependency cycle — {}", case.display(), cycle.join(" → "));
                }
                cases += 1;
            }
        }
        assert!(cases >= 10, "the gate saw only {} cases", cases);
    }

    /// The property has to be able to FAIL, or it proves nothing about the search.
    #[test]
    fn the_search_finds_a_cycle_that_is_there() {
        let tf = r#"
resource "google_project" "infra" {
  project_id = "acme-infra-001"
  depends_on = [google_project_service.infra_cloudresourcemanager_googleapis_com]
}
resource "google_project_service" "infra_cloudresourcemanager_googleapis_com" {
  project = google_project.infra.project_id
  service = "cloudresourcemanager.googleapis.com"
}
"#;
        let cycle = find_cycle(&graph(tf)).expect("this graph is a cycle");
        assert!(cycle.len() >= 3, "{:?}", cycle);
    }
}

#[cfg(test)]
mod prerequisites_gate {
    //! Every resource type the library can emit has a row in the prerequisite
    //! table (`src/prerequisites.rs`) — the roles the IaC service account needs for
    //! it AND the API that serves it. The cases under `tests/iac/` together use
    //! every pack, each one unconditionally, so a pack added without a case, or a
    //! pack emitting a type the table does not know, fails here.
    use std::collections::BTreeSet;
    use std::path::Path;

    /// The `use "…"` paths of a case, refusing a `when`: a pack switched off
    /// emits nothing, and a gate over nothing passes.
    fn uses(case: &Path, src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
            let mut rest = line;
            while let Some(i) = rest.find("use \"") {
                let after = &rest[i + 5..];
                let end = after.find('"').unwrap_or_else(|| panic!("{}: unterminated use: {}", case.display(), line));
                let tail = after[end + 1..].trim_start();
                assert!(!tail.starts_with("when"), "{}: `{}` — a gate case uses every pack unconditionally", case.display(), line.trim());
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
        }
        out
    }

    #[test]
    fn every_type_the_library_emits_has_its_prerequisites() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reg = super::corpus::registry();
        let mut used = BTreeSet::new();
        let mut types = BTreeSet::new();
        for entry in std::fs::read_dir(root.join("tests/iac")).expect("tests/iac").flatten() {
            let case = entry.path();
            let src = std::fs::read_to_string(case.join("main.satz"))
                .unwrap_or_else(|e| panic!("{}: {}", case.display(), e));
            used.extend(uses(&case, &src));
            let (out, _) = super::manifest_gate::emit_case(&case, &reg);
            let (_, unknown) = crate::prerequisites::needs(&out.manifest);
            assert!(
                unknown.is_empty(),
                "{}: no role known for {:?} — add the type's row to TYPES in src/prerequisites.rs",
                case.display(),
                unknown
            );
            types.extend(out.manifest.resources.values().map(|r| r.tf_type.clone()));
        }
        // The other half of the row. A type with no API is how a pack ships that
        // enables nothing and fails its first apply on an API nobody named.
        for t in &types {
            let apis = crate::prerequisites::apis_for(t).unwrap_or(&[]);
            assert!(
                !apis.is_empty(),
                "{}: no API known — add the service that serves it to the type's row in src/prerequisites.rs",
                t
            );
            for api in apis {
                assert!(
                    api.ends_with(".googleapis.com"),
                    "{}: {:?} is not an API host — the row names the service to enable, e.g. `logging.googleapis.com`",
                    t,
                    api
                );
            }
        }
        let packs: BTreeSet<String> = crate::doc_packs::packs(&root.join("presets"))
            .expect("the preset library")
            .into_iter()
            .map(|(p, _, _)| format!("presets/{}", crate::fsx::slash(&p)))
            .collect();
        let unused: Vec<&String> = packs.difference(&used).collect();
        assert!(unused.is_empty(), "packs no case under tests/iac/ uses: {:?}", unused);
        assert!(types.len() >= 20, "the gate checked only {} types: {:?}", types.len(), types);

        // The third half of shipping a type: `adopt` can find it live. Found when the
        // SCC notification config 409'd on a re-run and adopt answered "no rule" —
        // a pack that emits a type nobody can adopt has no way back once the object
        // exists, except `tofu import` by hand.
        let cfg: crate::config::ImportConfig =
            serde_yaml::from_str(&std::fs::read_to_string(root.join("presets/import-config.yaml")).expect("import-config.yaml"))
                .expect("import-config.yaml parses");
        let unadoptable: BTreeSet<&str> =
            types.iter().map(String::as_str).filter(|t| !crate::adopt::adoptable(&cfg, t)).collect();
        let excepted: BTreeSet<&str> = NOT_ADOPTABLE_YET.iter().map(|(t, _)| *t).collect();
        let new_gaps: Vec<&&str> = unadoptable.difference(&excepted).collect();
        assert!(
            new_gaps.is_empty(),
            "the library emits types adopt cannot resolve — add `import_id:` or `match_on:` to their rows in presets/import-config.yaml: {:?}",
            new_gaps
        );
        let closed: Vec<&&str> = excepted.difference(&unadoptable).collect();
        assert!(closed.is_empty(), "adopt resolves these now — take them off NOT_ADOPTABLE_YET: {:?}", closed);

        // and every placeholder of a template names something the resource declares: a
        // placeholder the packs never bind would only surface as `unresolvable` on a
        // customer's adopt. A value that is a reference to a folder's or group's live id,
        // or to an attribute known only after apply, legitimately does not render
        // offline — a placeholder with nothing behind it at all is the data bug.
        let mut rendered = 0;
        for entry in std::fs::read_dir(root.join("tests/iac")).expect("tests/iac").flatten() {
            let (out, _) = super::manifest_gate::emit_case(&entry.path(), &reg);
            for r in out.manifest.resources.values() {
                match crate::adopt::render_rule(&cfg, r, &out.manifest) {
                    Some(Ok(id)) => {
                        assert!(!id.contains('{') && !id.trim().is_empty(), "{}: rendered `{}`", r.address(), id);
                        rendered += 1;
                    }
                    Some(Err(why)) if why.contains(&format!("{} has no `", r.address())) => {
                        panic!("{}: its import_id rule names an attribute the resource does not declare — {}", r.address(), why)
                    }
                    _ => {}
                }
            }
        }
        assert!(rendered >= 20, "only {rendered} template ids rendered");
    }

    /// Types the library emits whose live id is assigned by the server, so no template
    /// can derive it from the estate: adopting one needs a live lookup adopt does not do
    /// yet. Each entry leaves the list with the rule that resolves it.
    const NOT_ADOPTABLE_YET: &[(&str, &str)] = &[
        ("google_tags_tag_key", "tagKeys/<number>, assigned on create — needs a lookup by short_name under the parent"),
        ("google_tags_tag_value", "tagValues/<number>, assigned on create — needs a lookup by short_name under the key"),
        ("google_tags_tag_binding", "tagBindings/<url-encoded parent>/tagValues/<number> — the value's number is assigned"),
        ("google_tags_tag_value_iam_member", "tagValues/<number> <role> <member> — the value's number is assigned"),
        ("google_compute_firewall_policy", "locations/global/firewallPolicies/<number>, assigned on create — needs a lookup by short_name"),
        ("google_compute_firewall_policy_association", "…/firewallPolicies/<number>/associations/<name> — the policy's number is assigned"),
        ("google_compute_firewall_policy_rule", "…/firewallPolicies/<number>/rules/<priority> — the policy's number is assigned"),
        ("google_cloudbuild_trigger", "projects/<p>/locations/<l>/triggers/<uuid>, assigned on create — needs a lookup by name"),
    ];
}

#[cfg(test)]
mod variables_gate {
    //! `variables.tf` and `terraform.tfvars` come out of one walk, and `tofu` checks
    //! one against the other before it does anything: a declared type that does not
    //! accept the value beside it refuses the apply, and the operator can fix neither
    //! file. The CIS baseline shipped that way — `cis_sa_key_creation_rules`, a list of
    //! objects, declared `list(string)` — and nothing failed until a customer's first
    //! apply. Here every param of every case, the `tests/iac/` cases using every pack,
    //! meets its declaration.
    use std::path::Path;

    /// Terraform's conversion, for the types `emitter::variable_type` declares: a
    /// string takes any scalar, a bool or a number its own kind or the string that
    /// spells one, a list or a map of strings a collection of scalars, `any` anything.
    fn accepts(ty: &str, v: &serde_yaml::Value) -> bool {
        use serde_yaml::Value;
        let scalar = |v: &Value| matches!(v, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_));
        match ty {
            "any" => true,
            "string" => scalar(v),
            "bool" => matches!(v, Value::Bool(_)) || matches!(v.as_str(), Some("true" | "false")),
            "number" => matches!(v, Value::Number(_)) || v.as_str().is_some_and(|s| s.parse::<f64>().is_ok()),
            "list(string)" => v.as_sequence().is_some_and(|items| items.iter().all(scalar)),
            "map(string)" => v.as_mapping().is_some_and(|m| m.values().all(scalar)),
            other => panic!("no conversion rule for `{other}` — emitter::variable_type declares a type this gate does not know"),
        }
    }

    #[test]
    fn the_model_refuses_what_tofu_refused() {
        let rules: serde_yaml::Value = serde_yaml::from_str("[{enforce: 'TRUE'}]").unwrap();
        assert!(!accepts("list(string)", &rules), "the declaration the CIS baseline shipped with");
        assert!(accepts(crate::emitter::variable_type(&rules), &rules));
        let names: serde_yaml::Value = serde_yaml::from_str("[a, true, 3]").unwrap();
        assert_eq!(crate::emitter::variable_type(&names), "list(string)", "a list of scalars keeps its type");
    }

    /// The attribute half of the same promise: nothing the library emits has a literal
    /// the provider schema's type refuses. The customer apply that found the check —
    /// `notification_emails` a string where a set belongs — came from an answer; this
    /// holds the packs' own defaults to it.
    #[test]
    fn no_case_emits_an_attribute_the_schema_refuses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reg = super::corpus::registry();
        let mut checked = 0;
        for dir in ["tests/iac", "tests/corpus"] {
            for entry in std::fs::read_dir(root.join(dir)).expect("case directory").flatten() {
                let case = entry.path();
                if !case.join("main.satz").exists() {
                    continue;
                }
                let (out, _) = super::manifest_gate::emit_case(&case, &reg);
                assert!(out.wrong_shapes.is_empty(), "{}: {:?}", case.display(), out.wrong_shapes);
                checked += out.manifest.resources.len();
            }
        }
        assert!(checked >= 200, "only {checked} resources checked");
    }

    #[test]
    fn every_param_s_declared_type_accepts_its_value() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reg = super::corpus::registry();
        let mut checked = 0;
        let mut collections = 0;
        for dir in ["tests/iac", "tests/corpus"] {
            for entry in std::fs::read_dir(root.join(dir)).expect("case directory").flatten() {
                let case = entry.path();
                if !case.join("main.satz").exists() {
                    continue;
                }
                let (_, fe) = super::manifest_gate::emit_case(&case, &reg);
                for (name, value) in &fe.tfvars {
                    let ty = crate::emitter::variable_type(value);
                    assert!(
                        accepts(ty, value),
                        "{}: `{}` is declared `{}` and its value is {:?} — tofu refuses the apply",
                        case.display(),
                        name,
                        ty,
                        value
                    );
                    checked += 1;
                    collections += usize::from(value.is_sequence() || value.is_mapping());
                }
            }
        }
        // a gate over scalars alone would not have met the list of objects
        assert!(checked >= 100 && collections >= 10, "checked {checked} params, {collections} collections");
    }
}

#[cfg(test)]
mod reset_replace {
    //! E14's first apply after the 2.7 pass: adopt had moved a legacy twin onto
    //! its `-superseded` address, the plan updated it in place with its old rules
    //! and `reset = true`, and the API refused. `plan` and `apply` replace it.
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_policy_holding_rules_and_declared_reset_is_replaced() {
        let manifest = crate::manifest::Manifest::parse(
            "resource \"google_org_policy_policy\" \"twin_superseded\" {\n  name = \"a\"\n  spec {\n    reset = true\n  }\n}\n\
             resource \"google_org_policy_policy\" \"fresh_reset\" {\n  name = \"b\"\n  spec {\n    reset = true\n  }\n}\n\
             resource \"google_org_policy_policy\" \"enforced\" {\n  name = \"c\"\n  spec {\n    rules {\n      enforce = \"TRUE\"\n    }\n  }\n}\n",
        );
        let state = crate::bootstrap::StateIndex::default()
            .with_rules(&["google_org_policy_policy.twin_superseded", "google_org_policy_policy.enforced"]);
        // not a reset declaration without rules in the state, and not a policy that keeps its rules
        assert_eq!(reset_replacements(&manifest, &state), ["google_org_policy_policy.twin_superseded"]);
    }

    #[test]
    fn a_saved_plan_a_destroy_and_a_refresh_take_no_replace() {
        assert_eq!(replace_args(&args(&[])), Some(Default::default()));
        assert!(replace_args(&args(&["-auto-approve", "-var-file", "x.tfvars", "-parallelism=4"])).is_some());
        assert_eq!(replace_args(&args(&["plan.tfplan"])), None);
        assert_eq!(replace_args(&args(&["-auto-approve", "plan.tfplan"])), None);
        assert_eq!(replace_args(&args(&["-destroy"])), None);
        assert_eq!(replace_args(&args(&["-refresh-only"])), None);
        // what the operator already replaces is not added twice, in either form
        let r = replace_args(&args(&["-replace=google_org_policy_policy.a", "-replace", "google_org_policy_policy.b"])).unwrap();
        assert_eq!(r.into_iter().collect::<Vec<_>>(), ["google_org_policy_policy.a", "google_org_policy_policy.b"]);
    }
}

#[cfg(test)]
mod api_preflight {
    //! What `plan` and `apply` read out of the emitted directory before they
    //! start the tool: the project every call is billed to, the identity to ask
    //! as, and the APIs the estate declares there.
    use super::*;

    const PROVIDERS: &str = r#"
provider "google" {
  alias = "google"
  project = "corp-infra-001"
  billing_project = "corp-infra-001"
  user_project_override = true
  impersonate_service_account = "svc-iac-001@corp-infra-001.iam.gserviceaccount.com"
}

provider "google-beta" {
  alias = "google-beta"
  billing_project = "corp-infra-001"
}

provider "google" {
  alias = "project_logsink"
  project = "corp-log-infra-001"
  billing_project = "corp-log-infra-001"
}
"#;

    fn read(text: &str) -> EmittedProvider {
        default_google_provider(hcl::parse(text).expect("parses").blocks())
    }

    #[test]
    fn the_billed_project_and_the_identity_come_from_the_default_provider() {
        // A per-project alias carries its own project in local mode, so taking
        // whichever block came first would make the answer depend on emission
        // order — and enable a project's APIs on another project.
        let p = read(PROVIDERS);
        assert_eq!(p.billing_project.as_deref(), Some("corp-infra-001"));
        assert_eq!(
            p.impersonate.as_deref(),
            Some("svc-iac-001@corp-infra-001.iam.gserviceaccount.com")
        );
    }

    #[test]
    fn a_local_mode_provider_names_no_identity() {
        let p = read("provider \"google\" {\n  alias = \"google\"\n  billing_project = \"corp-infra-001\"\n}\n");
        assert_eq!(p.billing_project.as_deref(), Some("corp-infra-001"));
        assert_eq!(p.impersonate, None);
        // nothing to preflight where no provider bills centrally
        assert_eq!(read("terraform {\n}\n"), EmittedProvider::default());
    }

    #[test]
    fn the_declared_apis_are_the_ones_on_the_billed_project() {
        // The services of another project are that project's business; the
        // refresh is billed here.
        let manifest = crate::manifest::Manifest::parse(
            "resource \"google_project\" \"infra\" {\n  project_id = \"corp-infra-001\"\n}\n\
             resource \"google_project_service\" \"infra_iam\" {\n  project = google_project.infra.project_id\n  service = \"iam.googleapis.com\"\n}\n\
             resource \"google_project_service\" \"infra_asset\" {\n  project = \"corp-infra-001\"\n  service = \"cloudasset.googleapis.com\"\n}\n\
             resource \"google_project_service\" \"other\" {\n  project = \"corp-log-infra-001\"\n  service = \"logging.googleapis.com\"\n}\n",
        );
        assert_eq!(
            crate::prerequisites::declared_apis(&manifest, "corp-infra-001"),
            ["cloudasset.googleapis.com", "iam.googleapis.com"]
        );
    }

    #[test]
    fn a_refusal_prints_the_command_that_does_it_by_hand() {
        let r = crate::prerequisites::ApiRefusal {
            project: "corp-infra-001".to_string(),
            enable: vec!["cloudasset.googleapis.com".to_string(), "iam.googleapis.com".to_string()],
            why: "satz could not enable them".to_string(),
            detail: "403 Forbidden [PERMISSION_DENIED]".to_string(),
        };
        assert!(
            r.render().contains(
                "gcloud services enable cloudasset.googleapis.com iam.googleapis.com --project corp-infra-001"
            ),
            "{}",
            r.render()
        );
        // the reason the operator can act on comes before the body they cannot
        assert!(r.render().find("gcloud").unwrap() < r.render().find("403").unwrap());
        assert!(r.summary().contains("nothing was planned or applied"));
    }

    #[test]
    fn nothing_to_do_says_so_and_a_change_names_every_api() {
        let done = crate::prerequisites::ApiPreflight {
            project: "corp-infra-001".to_string(),
            declared: vec!["iam.googleapis.com".to_string()],
            enabled: Vec::new(),
        };
        assert_eq!(done.render(), "APIs on corp-infra-001: 1 declared, all enabled\n");
        let changed = crate::prerequisites::ApiPreflight {
            enabled: vec!["iam.googleapis.com".to_string()],
            ..done
        };
        assert!(changed.render().contains("1 declared, 1 off"));
        assert!(changed.render().contains("  enabled iam.googleapis.com\n"));
    }
}

#[cfg(test)]
mod import_id_channels {
    //! `"import-id"` on every kind of emitted resource. The emitter used to
    //! drop it silently for IAM bindings and nested project services, and
    //! memberships had no channel at all.
    use super::*;

    const ESTATE: &str = r#"estate import_channels

params {
  customer_organization_id = "123456789012"
  customer_id = "C0example"
  customer_domain = "example.com"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_organization_iam_member {
  "group:gcp-org-admins@{customer_domain}" = [
    "roles/viewer",
    { role = "roles/browser" "import-id" = "123456789012 roles/browser group:gcp-org-admins@example.com" },
  ]
}

google_project {
  infra {
    "import-id" = "acme-infra-001"
    project_id = "acme-infra-001"
    project_service = [
      "logging.googleapis.com",
      { service = "storage.googleapis.com" "import-id" = "acme-infra-001/storage.googleapis.com" },
    ]
  }
}

google_cloud_identity_group {
  gcp_auditors {
    "import-id" = "groups/00abc"
    member = [
      "user:a@{customer_domain}",
      { id = "user:b@{customer_domain}" "import-id" = "groups/00abc/memberships/111" },
    ]
  }
}
"#;

    fn pipeline_b(reg: &ResourceRegistry) -> crate::emitter::EmitOut {
        let resolver = crate::EstateResolver { registry: reg };
        let fe = satz_core::pipeline::compile_estate("main.satz", ESTATE, &resolver, &|p| Err(format!("no use: {}", p)))
            .expect("front-end");
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(reg);
        crate::emitter::emit(&folded, &ctx).expect("emit")
    }

    #[test]
    fn every_channel_emits_its_import_block() {
        let reg = super::corpus::registry();
        let b = pipeline_b(&reg);

        let binding = crate::emit_shared::iam_member_label("group:gcp-org-admins@example.com", "roles/browser", None, "");
        let membership = crate::emit_shared::membership_resource_label("gcp_auditors", "user:b@example.com");
        for (to, id) in [
            (format!("google_organization_iam_member.{}", binding), "123456789012 roles/browser group:gcp-org-admins@example.com"),
            ("google_project.infra".to_string(), "acme-infra-001"),
            ("google_project_service.infra_storage_googleapis_com".to_string(), "acme-infra-001/storage.googleapis.com"),
            ("google_cloud_identity_group.gcp_auditors".to_string(), "groups/00abc"),
            (format!("google_cloud_identity_group_membership.{}", membership), "groups/00abc/memberships/111"),
        ] {
            assert!(b.imports_tf.contains(&format!("to = {}", to)), "missing import for {}:\n{}", to, b.imports_tf);
            assert!(b.imports_tf.contains(&format!("id = \"{}\"", id)), "missing id {}:\n{}", id, b.imports_tf);
        }
        assert!(!b.main_tf.contains("import-id"), "import-id must never reach a resource body:\n{}", b.main_tf);
        // the unadopted entries still emit as resources
        assert!(b.manifest.addresses().contains("google_project_service.infra_logging_googleapis_com"));
        assert_eq!(b.manifest.of_type("google_cloud_identity_group_membership").count(), 2);
    }

    /// Defect #33, closed: a folder's attributes beyond the fixed set used to
    /// be dropped without a warning.
    #[test]
    fn folder_emits_every_attribute_it_declares() {
        let reg = super::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let src = ESTATE.replace(
            "google_project {",
            "google_folder {\n  shared {\n    display_name = \"Shared\"\n    deletion_protection = false\n    tags = { \"123/env\" = \"prod\" }\n  }\n}\n\ngoogle_project {",
        );
        let fe = satz_core::pipeline::compile_estate("main.satz", &src, &resolver, &|p| Err(format!("no use: {}", p)))
            .expect("front-end");
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        assert!(out.main_tf.contains("deletion_protection = false"), "{}", out.main_tf);
        assert!(out.main_tf.contains("\"123/env\" = \"prod\""), "{}", out.main_tf);
        assert!(out.main_tf.contains("display_name = \"Shared\""), "{}", out.main_tf);
    }

    #[test]
    fn two_different_ids_for_one_binding_refuse() {
        use satz_core::algebra::GrantEdge;
        let mut edges = std::collections::BTreeSet::new();
        for id in ["one", "two"] {
            edges.insert(GrantEdge { member: "user:x@example.com".into(), role: "roles/viewer".into(), condition: String::new(), import_id: id.into() });
        }
        let err = crate::emitter::reconciled_edges(&edges).unwrap_err();
        assert!(err.contains("two different import-ids"), "{}", err);

        let mut merged = std::collections::BTreeSet::new();
        merged.insert(GrantEdge { member: "u".into(), role: "r".into(), condition: String::new(), import_id: String::new() });
        merged.insert(GrantEdge { member: "u".into(), role: "r".into(), condition: String::new(), import_id: "x".into() });
        let got = crate::emitter::reconciled_edges(&merged).unwrap();
        assert_eq!(got.len(), 1, "with and without an id is one binding");
        assert_eq!(got[0].import_id, "x");
    }
}

#[cfg(test)]
mod import_skipped_report {
    //! An import may be partial; it may not be silent about it. Every
    //! resource the state had and the estate does not is named with a reason.
    use crate::discovery::{Discoverer, SkipReason};

    const STATE: &str = r#"{"values":{"root_module":{"resources":[
      {"type":"google_project","name":"infra","values":{"project_id":"acme-infra","name":"Infra"}},
      {"type":"google_storage_bucket","name":"logs","values":{"name":"acme-logs","project":"acme-infra"}},
      {"type":"google_storage_bucket","name":"stray","values":{"name":"acme-stray","project":"not-imported"}},
      {"type":"google_compute_network","name":"vpc","values":{"name":"vpc","project":"acme-infra"}},
      {"type":"google_pubsub_topic","name":"t","values":{"name":"t","project":"acme-infra"}}
    ]}}}"#;

    #[test]
    fn every_left_out_resource_is_named_with_its_reason() {
        let state: serde_json::Value = serde_json::from_str(STATE).unwrap();
        let enabled = ["google_project", "google_storage_bucket"].into_iter().map(String::from).collect();
        let filtered = ["google_compute_network"].into_iter().map(String::from).collect();
        let found = Discoverer::new(state, None, Some(enabled), filtered, Default::default()).discover().unwrap();
        let mut got: Vec<(String, String, SkipReason)> =
            found.skipped.iter().map(|s| (s.tf_type.clone(), s.what.clone(), s.reason.clone())).collect();
        got.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            got,
            vec![
                ("google_compute_network".into(), "vpc".into(), SkipReason::Filtered),
                ("google_pubsub_topic".into(), "t".into(), SkipReason::TypeOff),
                ("google_storage_bucket".into(), "stray".into(), SkipReason::ParentNotFound("project not-imported".into())),
            ]
        );
        // and the one that fit is in the estate
        let infra = &found.config.project.as_ref().unwrap()["infra"];
        assert!(infra.extra.contains_key("google_storage_bucket"), "{:?}", infra.extra.keys().collect::<Vec<_>>());
    }

    /// F5a: a key the provider schema does not know is API vocabulary and
    /// would not plan — dropped, and named.
    #[test]
    fn unknown_attributes_are_dropped_and_reported() {
        let reg = super::corpus::registry();
        let state: serde_json::Value = serde_json::from_str(r#"{"values":{"root_module":{"resources":[
          {"type":"google_project","name":"infra","values":{"project_id":"acme-infra","name":"Infra"}},
          {"type":"google_storage_bucket","name":"logs","values":{"name":"acme-logs","project":"acme-infra","location":"EU",
             "lifecycle":{"rule":[{"action":{"type":"Delete"},"condition":{"age":30}}]},
             "versioning":{"enabled":true}}}
        ]}}}"#).unwrap();
        let enabled = ["google_project", "google_storage_bucket"].into_iter().map(String::from).collect();
        let found = Discoverer::new(state, Some(reg), Some(enabled), Default::default(), Default::default()).discover().unwrap();
        assert_eq!(found.dropped_attrs, vec![("google_storage_bucket".to_string(), "lifecycle".to_string())]);
        let bucket = &found.config.project.as_ref().unwrap()["infra"].extra["google_storage_bucket"];
        let text = serde_yaml::to_string(bucket).unwrap();
        assert!(!text.contains("lifecycle"), "{}", text);
        assert!(text.contains("versioning"), "a known block survives:\n{}", text);
    }

    #[test]
    fn organization_comes_from_the_ancestor_chain() {
        use crate::discovery::organization_from_ancestors;
        let anc = vec!["projects/123".to_string(), "folders/456".to_string(), "organizations/789".to_string()];
        assert_eq!(organization_from_ancestors(&anc).as_deref(), Some("789"));
        assert_eq!(organization_from_ancestors(&Vec::<String>::new()), None);
    }
}

#[cfg(test)]
mod init_template {
    //! The estate `init` writes must compile through the fragment pipeline and
    //! emit exactly the labels `bootstrap` imports by name.

    #[test]
    fn the_generated_estate_compiles_and_carries_bootstraps_labels() {
        let dir = std::env::temp_dir().join(format!("satz-init-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("C0example.satz");
        crate::template::generate_template(&crate::template::tests::args("first.admin", "example.com"), Some(&crate::template::tests::shipped()), &path).unwrap();
        let src = std::fs::read_to_string(&path).unwrap();

        let reg = super::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        // An init estate carries no uncommented pack line: it must compile with no presets
        // fetched, because `satz bootstrap` is the very next command the operator runs.
        let fe = satz_core::pipeline::compile_estate("C0example.satz", &src, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("init template does not compile: {:?}\n{}", e, src));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        for a in [
            "google_folder.infra_folder",
            "google_project.infra",
            "google_storage_bucket.state",
            "google_service_account.provisioner",
            "google_cloud_identity_group.svc_iac_users",
        ] {
            assert!(addrs.contains(a), "bootstrap imports {} by name; got {:?}", a, addrs);
        }
        // The CIS pack claims CIS 5.0 §2.14 (Cloud Asset Inventory enabled) against the
        // service the scaffold enables here, rather than declaring a second
        // `google_project_service` for the same API on the same project. That makes the
        // derived address part of the same contract as the labels above: it is
        // `<project label>_<service, dots to underscores>`, so renaming the project
        // label or dropping the service turns a satisfied control into a broken claim
        // in every estate. Fails here first.
        assert!(
            addrs.contains("google_project_service.infra_cloudasset_googleapis_com"),
            "the CIS pack claims 5.0 §2.14 against this address; got {:?}",
            addrs
        );
        assert!(out.imports_tf.contains("google_storage_bucket.state"), "{}", out.imports_tf);
        // the membership emits the bare email (prefix stripped), the org grants keep it
        assert!(out.main_tf.contains("id = \"first.admin@example.com\""), "{}", out.main_tf);
        assert!(out.main_tf.contains("member = \"group:svc-iac-users@example.com\""), "{}", out.main_tf);
        assert!(out.main_tf.contains("svc-iac-users@example.com"), "{}", out.main_tf);
        assert_eq!(out.manifest.of_type("google_organization_iam_member").count(), 15);
        // named roles, not owner — and exactly what the template's own resource
        // types need, so a fresh estate has nothing to add
        let get = |k: &str| fe.env.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let sa = crate::prerequisites::service_account_of(get).expect("the template names its IaC service account");
        let granted = crate::prerequisites::granted(&out.manifest, &sa);
        assert!(!granted.owner(), "the template grants roles/owner");
        let (needs, unknown) = crate::prerequisites::needs(&out.manifest);
        assert!(unknown.is_empty(), "the template emits types the role table does not know: {:?}", unknown);
        let missing = crate::prerequisites::missing(&needs, &granted);
        assert!(missing.is_empty(), "the template misses roles its own types need: {:?}", crate::prerequisites::describe(&crate::prerequisites::cover(&missing)));
        // and the other half of a prerequisite: the APIs its own types are served
        // by, on the project every call is billed to. A fresh estate that warns on
        // its first transpile is a scaffold that was never finished.
        let infra = get("infra_project_name").expect("the template names its infra project");
        let missing_apis = crate::prerequisites::missing_apis(&out.manifest, &infra);
        assert!(
            missing_apis.is_empty(),
            "the template misses APIs its own types need: {:?}",
            missing_apis.iter().map(|a| a.api.as_str()).collect::<Vec<_>>()
        );
        // the users group may become the IaC service account, and only that one:
        // TokenCreator and serviceAccountUser on the account, not on the org
        assert_eq!(out.manifest.of_type("google_service_account_iam_member").count(), 2);
        assert!(
            !out.manifest
                .of_type("google_organization_iam_member")
                .any(|r| r.attrs.get("role").map(String::as_str) == Some("roles/iam.serviceAccountTokenCreator")),
            "TokenCreator is granted at the organization:\n{}",
            out.main_tf
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod migrate_mode {
    //! `satz migrate` reads an estate's deployment mode as `whoami` and the emitter do —
    //! `local` when the estate declares none — so the migrate `whoami` suggests is one
    //! `migrate` runs.
    use super::*;

    const HEAD: &str = "estate migrating\n\nparams {\n  svc_iac_account    = \"svc-iac-001\"\n  infra_project_name = \"acme-infra-001\"\n";
    const TAIL: &str = "}\n\nterraform {\n  backend {\n    local { path = \"terraform.tfstate\" }\n  }\n}\n";
    const SA: &str = "svc-iac-001@acme-infra-001.iam.gserviceaccount.com";

    /// An estate in its own directory, with `packs` beside it; the config reads nothing else.
    fn estate(case: &str, params: &str, uses: &str, packs: &[(&str, &str)]) -> (PathBuf, ToolConfig) {
        let dir = std::env::temp_dir().join(format!("satz-migrate-mode-{}-{}", std::process::id(), case));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in packs {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let path = dir.join("migrating.satz");
        std::fs::write(&path, format!("{}{}{}{}", HEAD, params, TAIL, uses)).unwrap();
        let mut cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.include_dirs = Vec::new();
        (path, cfg)
    }

    /// What `whoami` reads off the file `migrate` would write.
    fn written(path: &Path, cfg: &ToolConfig, text: &str) -> crate::gcp::identity::EstateDeclaration {
        std::fs::write(path, text).unwrap();
        estate_declaration(path, path.display().to_string(), cfg).unwrap()
    }

    #[test]
    fn an_estate_that_declares_no_mode_is_local_and_the_switch_binds_cloud() {
        let (path, cfg) = estate("none", "", "", &[]);
        let whoami = estate_declaration(&path, path.display().to_string(), &cfg).unwrap();
        let switch = mode_switch(&path, &cfg, None).expect("an estate without the param migrates");
        assert_eq!((switch.from.as_str(), switch.to.as_str()), (whoami.mode.as_str(), "cloud"));
        let after = switch.after.expect("local to cloud changes the file");
        assert!(after.contains("  deployment_mode    = \"cloud\"\n}"), "the binding goes into params:\n{}", after);
        let now = written(&path, &cfg, &after);
        assert_eq!(now.impersonation_target(), Some(SA), "the account whoami named is the one now impersonated");
        // and back: the line is there now, so it is replaced, not added again
        let back = mode_switch(&path, &cfg, Some("local".into())).unwrap().after.unwrap();
        assert_eq!(back.matches("deployment_mode").count(), 1, "{}", back);
        assert_eq!(written(&path, &cfg, &back).mode, "local");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_declared_mode_is_replaced_and_its_comment_kept() {
        let (path, cfg) = estate("declared", "  deployment_mode    = \"local\" // switched by `satz migrate`\n", "", &[]);
        let after = mode_switch(&path, &cfg, Some("cloud".into())).unwrap().after.unwrap();
        assert!(after.contains("  deployment_mode    = \"cloud\" // switched by `satz migrate`\n"), "{}", after);
        assert_eq!(after.matches("deployment_mode").count(), 1, "{}", after);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// `presets/estate-core.satz` binds `deployment_mode = "local"` as a default: the switch
    /// binds the new mode in the estate, whose own params come first.
    #[test]
    fn a_mode_a_pack_defaults_is_overridden_in_the_estate() {
        let core = "// Day-0 params.\npack core version \"1.0\"\n\nparams {\n  deployment_mode = \"local\"\n}\n";
        let (path, cfg) = estate("pack", "", "\nuse \"core.satz\"\n", &[("core.satz", core)]);
        let switch = mode_switch(&path, &cfg, None).unwrap();
        assert_eq!(switch.from, "local");
        let after = switch.after.unwrap();
        assert_eq!(written(&path, &cfg, &after).mode, "cloud", "the pack's default won over the estate:\n{}", after);
        assert_eq!(std::fs::read_to_string(path.parent().unwrap().join("core.satz")).unwrap(), core, "the pack was edited");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The emitter writes a backend for `local` and for `cloud` and for nothing else, so
    /// those are the values `--mode` takes: the help lists them and clap refuses the rest
    /// before the estate is touched.
    #[test]
    fn the_mode_is_local_or_cloud_and_anything_else_is_refused() {
        let mut cmd = Cli::command();
        cmd.build();
        let migrate = cmd.find_subcommand("migrate").expect("migrate is a command");
        let mode = migrate.get_arguments().find(|a| a.get_id() == "mode").expect("migrate takes --mode");
        let values: Vec<String> = mode.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
        assert_eq!(values, ["local", "cloud"]);
        let err = Cli::command()
            .try_get_matches_from(["satz", "migrate", "x.satz", "--mode", "foo"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid value 'foo'"), "{err}");
        assert!(err.contains("local, cloud"), "{err}");
        let parsed = Cli::try_parse_from(["satz", "migrate", "x.satz", "--mode", "cloud"]).expect("parses").command;
        assert!(matches!(&parsed, Some(Commands::Migrate { mode: Some(m), .. }) if m == "cloud"));
    }

    #[test]
    fn the_mode_the_estate_runs_in_already_changes_nothing() {
        let (path, cfg) = estate("same", "", "", &[]);
        let switch = mode_switch(&path, &cfg, Some("local".into())).unwrap();
        assert!(switch.after.is_none(), "an estate that declares no mode is in local mode already");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    const UNDERIVABLE: &str = "satz cannot tell which identity this estate runs as, and runs nothing for it";

    /// Every reader of an estate's identity refuses one it cannot derive, with the same
    /// words: `whoami` (the declaration), the CLI's binding and `satz mcp`'s per-call scope
    /// (the target), and `migrate` (the switch). Reading it as "impersonates nothing" ran
    /// every live command as whoever was logged in.
    fn refused_everywhere(path: &Path, cfg: &ToolConfig, reason: &str) {
        let said = [
            ("whoami", estate_declaration(path, path.display().to_string(), cfg).map(|_| ()).unwrap_err()),
            ("the target", estate_impersonation_target(path, cfg).map(|_| ()).unwrap_err()),
            ("the binding", configure_estate_impersonation(path, cfg).unwrap_err()),
            ("migrate", mode_switch(path, cfg, Some("cloud".into())).map(|_| ()).unwrap_err().to_string()),
        ];
        for (who, e) in said {
            assert!(e.contains(reason), "{who} does not name the reason `{reason}`: {e}");
            assert!(e.contains(UNDERIVABLE), "{who} does not say the identity cannot be derived: {e}");
        }
    }

    /// A mode the compile refuses is refused at the line the estate binds it on.
    #[test]
    fn a_mode_the_compile_refuses_is_refused_by_every_reader() {
        let (path, cfg) = estate("boot", "  deployment_mode    = \"boot\"\n", "", &[]);
        refused_everywhere(&path, &cfg, &format!("{}:6: `deployment_mode = \"boot\"`", path.display()));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Bound by a pack, the mode is refused naming the estate, at no line of it.
    #[test]
    fn a_refused_mode_a_pack_binds_is_refused_naming_the_estate() {
        let core = "// Day-0 params.\npack core version \"1.0\"\n\nparams {\n  deployment_mode = \"boot\"\n}\n";
        let (path, cfg) = estate("boot-pack", "", "\nuse \"core.satz\"\n", &[("core.satz", core)]);
        refused_everywhere(&path, &cfg, &format!("{}: `deployment_mode = \"boot\"`", path.display()));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Params that do not parse have no mode and no account: the parser's error, at its line.
    #[test]
    fn params_that_do_not_parse_are_refused_by_every_reader() {
        let (path, cfg) = estate("unreadable", "  deployment_mode    = \"cloud\n", "", &[]);
        refused_everywhere(&path, &cfg, &format!("{}:6: newline in single-line string", path.display()));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Cloud mode without the account is refused at the line that binds the mode, by
    /// every reader, with the compile's words.
    #[test]
    fn cloud_mode_without_the_account_is_refused_by_every_reader() {
        let (path, cfg) = estate("no-account", "  deployment_mode    = \"cloud\"\n", "", &[]);
        std::fs::write(&path, fsx::read_to_string(&path).unwrap().replace("  svc_iac_account    = \"svc-iac-001\"\n", "")).unwrap();
        refused_everywhere(
            &path,
            &cfg,
            &format!("{}:5: `deployment_mode = \"cloud\"` without a value for `svc_iac_account`: ", path.display()),
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// `migrate --mode cloud` on a local estate that names no account is refused before
    /// the file is touched, naming the params — not written and then refused by the compile.
    #[test]
    fn migrating_to_cloud_without_the_account_is_refused_before_the_file_is_touched() {
        let (path, cfg) = estate("migrate-no-account", "", "", &[]);
        let text = fsx::read_to_string(&path).unwrap().replace("  infra_project_name = \"acme-infra-001\"\n", "");
        std::fs::write(&path, &text).unwrap();
        assert_eq!(estate_declaration(&path, path.display().to_string(), &cfg).unwrap().mode, "local");
        let e = mode_switch(&path, &cfg, Some("cloud".into())).map(|_| ()).unwrap_err().to_string();
        assert!(e.contains("`deployment_mode = \"cloud\"` without a value for `infra_project_name`: "), "{e}");
        assert!(e.contains("the estate stays in local mode, and nothing was changed"), "{e}");
        assert_eq!(fsx::read_to_string(&path).unwrap(), text, "the refused switch edited the estate");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

#[cfg(test)]
mod estate_paths {
    //! An estate path resolves inside `yaml_dir`, so naming the directory yourself
    //! doubles it. Harmless for a file that exists; permanent when one is created.
    use super::*;

    #[test]
    fn a_path_that_names_yaml_dir_is_caught_with_the_bare_form() {
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", "yaml").as_deref(), Some("SKEL.satz"));
        assert_eq!(redundant_yaml_dir("yaml/sub/SKEL.satz", "yaml").as_deref(), Some("sub/SKEL.satz"));
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", "yaml/").as_deref(), Some("SKEL.satz"));
        // how it actually arrives with `--config .`, which is what the guard missed first
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", "./yaml").as_deref(), Some("SKEL.satz"));
        assert_eq!(redundant_yaml_dir("./yaml/SKEL.satz", "./yaml").as_deref(), Some("SKEL.satz"));
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", "/abs/estate/yaml").as_deref(), Some("SKEL.satz"));
        // the bare form, another directory, and a path that IS the directory
        assert_eq!(redundant_yaml_dir("SKEL.satz", "yaml"), None);
        assert_eq!(redundant_yaml_dir("estates/SKEL.satz", "yaml"), None);
        assert_eq!(redundant_yaml_dir("yaml", "yaml"), None);
        // an absolute path is taken as given, and a flat layout has nothing to double
        assert_eq!(redundant_yaml_dir("/abs/yaml/SKEL.satz", "yaml"), None);
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", "."), None);
        assert_eq!(redundant_yaml_dir("yaml/SKEL.satz", ""), None);
    }
}

#[cfg(test)]
mod discover_satz {
    //! `discover-* --satz`: the discovered data must come out as an estate the
    //! fragment pipeline compiles, with the preamble the emitter requires and
    //! every discovered id carried as an import.
    use super::*;

    #[test]
    fn discovered_config_becomes_an_estate_that_compiles_and_imports() {
        let yaml = r#"
folder:
  workloads:
    display_name: Workloads
    import-id: folders/111
    project:
      infra:
        project_id: acme-infra-001
        name: acme-infra-001
        import-id: acme-infra-001
        project_service:
          - service: iam.googleapis.com
            import-id: acme-infra-001/iam.googleapis.com
google_storage_bucket:
  state:
    name: acme-state
    location: EU
    import-id: acme-state
    versioning:
      - enabled: true
google_organization_iam_audit_config:
  all:
    org_id: "123456789012"
    service: allServices
org_policy_policy:
  compute-managed-requireOsLogin:
    import-id: organizations/123456789012/policies/compute.managed.requireOsLogin
    name: compute.managed.requireOsLogin
    spec:
      - rules:
          - enforce: "TRUE"
google_organization_iam_member:
  "group:a@example.com":
    - role: roles/viewer
      import-id: 123456789012 roles/viewer group:a@example.com
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = super::corpus::registry();
        let text = discovered_to_satz(&config, "discovered", None, &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert!(text.contains("customer_organization_id = \"123456789012\""), "{}", text);
        assert!(text.contains("terraform {"), "{}", text);
        assert!(text.contains("google_folder {"), "shorthand keys must be normalised:\n{}", text);
        // the condensed forms: one line per service and per grant edge, a
        // single block as a block, the organization referenced not repeated,
        // a project name equal to its id dropped
        assert!(text.contains("{ service = \"iam.googleapis.com\" \"import-id\" = \"acme-infra-001/iam.googleapis.com\" },"), "{}", text);
        assert!(text.contains("{ role = \"roles/viewer\" \"import-id\" = \"{customer_organization_id} roles/viewer group:a@example.com\" },"), "{}", text);
        assert!(text.contains("spec {\n"), "{}", text);
        assert!(text.contains("{ enforce = \"TRUE\" },"), "{}", text);
        assert!(text.contains("versioning {\n"), "{}", text);
        assert!(text.contains("org_id = customer_organization_id"), "{}", text);
        assert!(text.contains("\"import-id\" = \"organizations/{customer_organization_id}/policies/compute.managed.requireOsLogin\""), "{}", text);
        assert!(!text.contains("name = \"acme-infra-001\""), "{}", text);
        // the file is formatted on the way to disk; the inline forms survive it
        let formatted = satz_core::fmt::format(&text).unwrap();
        assert!(formatted.contains("{ service = \"iam.googleapis.com\" \"import-id\" = \"acme-infra-001/iam.googleapis.com\" },"), "{}", formatted);
        assert!(formatted.contains("{ enforce = \"TRUE\" },"), "{}", formatted);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("discovered estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        for a in [
            "google_folder.workloads",
            "google_project.infra",
            "google_storage_bucket.state",
            "google_organization_iam_audit_config.all",
            "google_org_policy_policy.compute_managed_requireOsLogin",
            "google_project_service.infra_iam_googleapis_com",
        ] {
            assert!(addrs.contains(a), "missing {} in {:?}", a, addrs);
        }
        for id in [
            "folders/111",
            "acme-infra-001",
            "acme-state",
            "acme-infra-001/iam.googleapis.com",
            "organizations/123456789012/policies/compute.managed.requireOsLogin",
            "123456789012 roles/viewer group:a@example.com",
        ] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
        assert!(out.main_tf.contains("organizations/123456789012/policies/compute.managed.requireOsLogin"), "the bare constraint expands:\n{}", out.main_tf);
        assert!(out.main_tf.contains("versioning {"), "{}", out.main_tf);
        assert!(!out.main_tf.contains("import-id"));
    }

    #[test]
    fn a_counted_grant_emits_its_own_address() {
        // one principal, one role, two folders: the map form alone emits one
        // address (its label hashes member + role) and the emitter refuses;
        // with the counter the second edge is a labelled resource of its own
        let yaml = r#"
folder:
  a:
    display_name: A
    import-id: folders/1
    google_folder_iam_member:
      "user:x@example.com":
        - role: roles/resourcemanager.folderAdmin
          import-id: folders/1 roles/resourcemanager.folderAdmin user:x@example.com
  b:
    display_name: B
    import-id: folders/2
    google_folder_iam_member:
      "user:x@example.com":
        - role: roles/resourcemanager.folderAdmin
          import-id: folders/2 roles/resourcemanager.folderAdmin user:x@example.com
"#;
        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = super::corpus::registry();
        let notes = crate::discovery::resolve_grant_collisions(&mut config, crate::discovery::OnCollision::Counter).unwrap();
        assert_eq!(notes.len(), 1, "{:?}", notes);
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert!(text.contains("folderAdmin_x_2 {"), "{}", text);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("counted estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        assert_eq!(addrs.iter().filter(|a| a.starts_with("google_folder_iam_member.")).count(), 2, "{:?}", addrs);
        assert!(addrs.contains("google_folder_iam_member.folderAdmin_x_2"), "{:?}", addrs);
        assert!(out.main_tf.contains("google_folder.b.name"), "the labelled grant inherits its folder from the node:\n{}", out.main_tf);
        for id in ["folders/1 roles/resourcemanager.folderAdmin user:x@example.com", "folders/2 roles/resourcemanager.folderAdmin user:x@example.com"] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
    }

    /// `--on-collision` declares its values as its value parser: the help lists them, clap
    /// refuses anything else before a source is read, and `error` is what an import does
    /// without the flag.
    #[test]
    fn on_collision_is_error_or_counter_and_anything_else_is_refused() {
        use crate::discovery::OnCollision;
        let mut cmd = Cli::command();
        cmd.build();
        let import = cmd.find_subcommand("import").expect("import is a command");
        let arg = import.get_arguments().find(|a| a.get_id() == "on_collision").expect("import takes --on-collision");
        let values: Vec<String> = arg.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
        assert_eq!(values, ["error", "counter"]);
        let err = Cli::command()
            .try_get_matches_from(["satz", "import", "state.json", "--on-collision", "hash"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid value 'hash'"), "{err}");
        assert!(err.contains("error, counter"), "{err}");
        let parsed = |args: &[&str]| match Cli::try_parse_from(args).expect("parses").command {
            Some(Commands::Import { on_collision, .. }) => on_collision,
            _ => panic!("not an import"),
        };
        assert_eq!(parsed(&["satz", "import", "state.json", "--on-collision", "counter"]), OnCollision::Counter);
        assert_eq!(parsed(&["satz", "import", "state.json"]), OnCollision::Error);
    }

    #[test]
    fn pinned_bucket_grants_emit_one_address_per_bucket() {
        // two buckets, one member, one role: two maps pinned to their bucket,
        // two addresses (the pin enters the label), two imports
        let yaml = r#"
project:
  infra:
    project_id: acme-infra-001
    import-id: acme-infra-001
    google_storage_bucket_iam_member:
      - bucket: acme-logs-001
        "group:gcp-auditors@example.com":
          - { role: roles/storage.objectViewer, import-id: "b/acme-logs-001 roles/storage.objectViewer group:gcp-auditors@example.com" }
      - bucket: acme-state
        "group:gcp-auditors@example.com":
          - { role: roles/storage.objectViewer, import-id: "b/acme-state roles/storage.objectViewer group:gcp-auditors@example.com" }
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = super::corpus::registry();
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert_eq!(text.matches("google_storage_bucket_iam_member {").count(), 2, "{}", text);
        assert!(text.contains("bucket = \"acme-logs-001\""), "{}", text);
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("pinned estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        assert_eq!(addrs.iter().filter(|a| a.starts_with("google_storage_bucket_iam_member.")).count(), 2, "{:?}", addrs);
        for id in ["b/acme-logs-001 roles/storage.objectViewer group:gcp-auditors@example.com", "b/acme-state roles/storage.objectViewer group:gcp-auditors@example.com"] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
    }

    #[test]
    fn the_vocabulary_is_bound_in_the_params_and_referenced_in_the_body() {
        // the day-0 params, inferred from what the sweep found, each marked
        // with its rule; every literal they name becomes the reference the
        // library spells, and an interpolated import id still resolves
        let yaml = r#"
organization_iam_member:
  "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com":
    - role: roles/resourcemanager.organizationAdmin
      import-id: 123456789012 roles/resourcemanager.organizationAdmin serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com
  "user:alice@example.com":
    - roles/viewer
folder:
  infra_folder:
    display_name: Infrastructure
    import-id: folders/111
    project:
      infra:
        project_id: acme-infra-001
        billing_account: 01AA-BB-CC
        import-id: acme-infra-001
        google_storage_bucket:
          state:
            name: acme-infra-001-state
            location: EU
            import-id: acme-infra-001-state
            versioning:
              - enabled: true
project:
  logs:
    project_id: acme-logs-001
    import-id: acme-logs-001
"#;
        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = super::corpus::registry();
        let vocab = crate::vocabulary::Vocabulary::infer(&config, Some("123456789012"), None, None);
        vocab.apply_billing(&mut config);
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &vocab).unwrap();
        assert!(text.contains("customer_shortname = \"acme\" // inferred: the leading token of"), "{}", text);
        assert!(text.contains("infra_project_name = \"acme-infra-001\" // inferred:"), "{}", text);
        assert!(text.contains("infra_bucket_name = \"acme-infra-001-state\" // inferred:"), "{}", text);
        assert!(text.contains("billing_account_infra = \"01AA-BB-CC\" // inferred:"), "{}", text);
        assert!(text.contains("customer_domain = \"example.com\" // inferred:"), "{}", text);
        assert!(!text.contains("customer_longname"), "never a placeholder:\n{}", text);
        assert!(text.contains("display_name = infra_folder_name"), "{}", text);
        assert!(text.contains("project_id = infra_project_name"), "{}", text);
        assert!(text.contains("name = infra_bucket_name"), "{}", text);
        assert!(text.contains("\"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\" = ["), "{}", text);
        assert!(text.contains("\"import-id\" = \"{customer_organization_id} roles/resourcemanager.organizationAdmin serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\""), "{}", text);
        assert!(text.contains("project_id = \"{customer_shortname}-logs-001\""), "{}", text);
        assert!(text.contains("billing_account = \"\""), "a project without an account says so:\n{}", text);
        assert!(!text.contains("billing_account = \"01AA-BB-CC\""), "the infra project's account is the param:\n{}", text);
        assert!(text.contains("bucket = infra_bucket_name") && text.contains("prefix = \"hcl/state\""), "the gcs backend beside the local one:\n{}", text);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("vocabulary estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        for id in [
            "123456789012 roles/resourcemanager.organizationAdmin serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com",
            "acme-infra-001",
            "acme-infra-001-state",
            "acme-logs-001",
        ] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
        assert!(out.main_tf.contains("project_id = \"acme-logs-001\""), "{}", out.main_tf);
    }

    #[test]
    fn the_quota_project_is_one_that_enables_the_organization_apis() {
        // alphabetically first is `alpha`, which enables nothing the
        // organization-scoped reads need; `infra` does
        let yaml = r#"
project:
  alpha:
    project_id: alpha-001
    project_service:
      - compute.googleapis.com
folder:
  f:
    display_name: F
    project:
      infra:
        project_id: infra-001
        project_service:
          - { service: orgpolicy.googleapis.com, import-id: infra-001/orgpolicy.googleapis.com }
          - serviceusage.googleapis.com
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(quota_project(&config).as_deref(), Some("infra-001"));
        let none: Config = serde_yaml::from_str("project:\n  alpha:\n    project_id: alpha-001\n").unwrap();
        assert_eq!(quota_project(&none).as_deref(), Some("alpha-001"), "the first project, and the report says why it may fail");
        assert_eq!(quota_project(&Config::default()), None);
    }

    #[test]
    fn org_id_is_inferred_from_references_or_org_id_keys() {
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  parent: organizations/123456789012\n").unwrap();
        assert_eq!(infer_org_id(&v).as_deref(), Some("123456789012"));
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  b:\n    org_id: \"222222222222\"\n").unwrap();
        assert_eq!(infer_org_id(&v).as_deref(), Some("222222222222"));
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  name: organizations/1/policies/x\n").unwrap();
        assert_eq!(infer_org_id(&v), None, "a policy name is not an org reference");
        assert_eq!(satz_output_path("yaml", PathBuf::from("discovered.yaml")), PathBuf::from("yaml/discovered.satz"));
        assert_eq!(satz_output_path("yaml", PathBuf::from("/abs/x.satz")), PathBuf::from("/abs/x.satz"));
    }
}

#[cfg(test)]
mod preset_tests {
    use std::path::{Path, PathBuf};

    fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read presets dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_yaml(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                out.push(path);
            }
        }
    }

    /// Presets are fragments: they reference anchors defined by the config that includes
    /// them, so they cannot be parsed standalone. Predefine every referenced anchor so
    /// the file's own structure is the only thing under test.
    fn anchor_prelude(content: &str) -> String {
        fn scan(content: &str, sigil: char) -> Vec<&str> {
            let mut names = Vec::new();
            // Comment lines are skipped: prose legitimately contains `*` (glob paths) and
            // `&`, and feeding those to the prelude emits anchors YAML cannot scan.
            for line in content.lines().filter(|l| !l.trim_start().starts_with('#')) {
                for (i, _) in line.match_indices(sigil) {
                    let rest = &line[i + 1..];
                    let end = rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                        .unwrap_or(rest.len());
                    // An anchor name must start alphanumerically, so `*.json`-style matches
                    // are not aliases and must not become `&.json` in the prelude.
                    let name = &rest[..end];
                    let starts_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                    if starts_ok && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            names
        }

        // Only supply anchors the file does not define itself. Redefining one it already
        // declares would change what its own aliases resolve to.
        let defined = scan(content, '&');
        let mut out = String::from("_test_anchors:\n");
        for n in scan(content, '*').into_iter().filter(|n| !defined.contains(n)) {
            // Distinct values: presets use aliases as mapping keys, so a shared
            // placeholder would collide them into a duplicate-key error.
            out.push_str(&format!("  {n}: &{n} \"{n}\"\n"));
        }
        out
    }

    #[test]
    fn every_shipped_preset_parses_as_yaml() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        let mut files = Vec::new();
        collect_yaml(&root, &mut files);
        assert!(!files.is_empty(), "no presets found under {}", root.display());

        for path in files {
            let content = std::fs::read_to_string(&path).expect("read preset");
            let doc = format!("{}{}", anchor_prelude(&content), content);
            if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(&doc) {
                panic!("preset {} does not parse: {e}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod checksum_tests {
    /// The self-update path verifies the downloaded installer against the release's
    /// `.sha256` asset (see `run_self_update`). Pin the digest to known vectors so a `sha2` major
    /// upgrade cannot silently change what that comparison computes — a wrong hash here
    /// either blocks every update or, worse, passes something it should not.
    #[test]
    fn sha256_matches_known_vectors() {
        use sha2::{Digest, Sha256};

        assert_eq!(
            hex::encode(Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex::encode(Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

#[cfg(test)]
mod config_error_tests {
    use super::describe_toml_error;
    use std::path::Path;

    #[test]
    fn toml_error_names_the_line_and_the_offending_key() {
        // A customer YAML passed to --config: TOML stops on the first `key:` with no `=`.
        // The raw error says only "key with no value" and embeds the whole file.
        let content = "variables:\n  infra-folder-name: &x \"Infrastructure\"\n";
        let err = toml::from_str::<toml::Value>(content).unwrap_err();
        let msg = describe_toml_error(Path::new("yaml/C0example.yaml"), content, &err);

        assert!(msg.contains("line 1"), "should give the line number: {msg}");
        assert!(msg.contains("variables:"), "should quote the offending line: {msg}");
        assert!(msg.contains("That file is YAML"), "should explain the --config mixup: {msg}");
        assert!(msg.contains("satz bootstrap C0example.yaml"), "should show the fix: {msg}");
    }

    #[test]
    fn genuinely_broken_toml_gets_no_yaml_hint() {
        let content = "yaml_dir = \"yaml\"\nthis line is not toml\n";
        let err = toml::from_str::<toml::Value>(content).unwrap_err();
        let msg = describe_toml_error(Path::new("config.toml"), content, &err);

        assert!(msg.contains("line 2"), "should point at the broken line: {msg}");
        assert!(!msg.contains("That file is YAML"), "no YAML hint for a .toml file: {msg}");
    }
}

#[cfg(test)]
mod path_resolution_tests {
    use super::resolve_against;
    use std::path::PathBuf;

    #[test]
    fn relative_paths_resolve_against_their_kind_directory() {
        // A YAML passed as a flag must land in yaml_dir just like the positional
        // beside it — previously flags resolved against the caller's directory, so
        // `diff C01.yaml --preset CIS.yaml` looked in two different places.
        assert_eq!(
            resolve_against("/proj/yaml", PathBuf::from("CIS-GCP-Foundation-4.0.yaml")),
            PathBuf::from("/proj/yaml/CIS-GCP-Foundation-4.0.yaml")
        );
        assert_eq!(
            resolve_against("/proj/yaml", PathBuf::from("presets/import-config.yaml")),
            PathBuf::from("/proj/yaml/presets/import-config.yaml")
        );
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        assert_eq!(
            resolve_against("/proj/yaml", PathBuf::from("/elsewhere/CIS.yaml")),
            PathBuf::from("/elsewhere/CIS.yaml")
        );
    }

    #[test]
    fn a_relative_base_stays_relative() {
        // yaml_dir is config-dir-prefixed, not canonicalised: `--config ../config.toml`
        // yields `../yaml`, and joining must preserve that.
        assert_eq!(
            resolve_against("../yaml", PathBuf::from("C01.yaml")),
            PathBuf::from("../yaml/C01.yaml")
        );
    }
}

#[cfg(test)]
mod presets_dir_tests {
    use super::*;

    #[test]
    fn presets_dir_defaults_beside_config() {
        // A config.toml without the key gets the library beside config.toml — yaml_dir
        // is reserved for files that are actually used, presets_dir for the copyable set.
        let cfg: ToolConfig = toml::from_str("yaml_dir = \"yaml\"").unwrap();
        assert_eq!(cfg.presets_dir, "presets");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-presets-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MINI_DISCOVERY: &str = "resource_types:\n  google_project:\n    description: p\n    import: true\n";

    #[test]
    fn import_config_resolves_only_in_presets_dir() {
        // Deliberately NO fallback to the pre-presets_dir layout (<yaml_dir>/presets):
        // if the library is not where config.toml says, that should be visible, not
        // silently papered over by reading a legacy location.
        let root = scratch("disc");
        let presets = root.join("presets");
        let yaml_legacy = root.join("yaml").join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::create_dir_all(&yaml_legacy).unwrap();
        let cfg: ToolConfig = toml::from_str("").unwrap();
        let p_dir = presets.to_str().unwrap().to_string();

        // A file in the legacy location alone must NOT be found.
        std::fs::write(yaml_legacy.join("import-config.yaml"), MINI_DISCOVERY).unwrap();
        assert!(load_import_config(None, &cfg, &p_dir).unwrap().is_none());

        // The presets library is the one and only default location.
        std::fs::write(presets.join("import-config.yaml"), MINI_DISCOVERY).unwrap();
        assert!(load_import_config(None, &cfg, &p_dir).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&root);
    }
}


#[cfg(test)]
mod constraint_equivalents {
    //! The rule: where Google replaces a legacy org-policy constraint with a
    //! managed one, a pack runs the REPLACEMENT ALONE and declares the legacy
    //! twin off. Both forms in force means every exemption has to lift two
    //! policies, and for several constraints Google's only exemption path is to
    //! disable the constraint org-wide, grant, and re-enable.
    //!
    //! The pairing is data, not memory: `presets/managed-constraint-equivalents.txt`,
    //! generated from a live organisation by
    //! `scripts/update_constraint_equivalents.py`. Google keeps adding managed
    //! twins, so a rule enforced by re-auditing by hand is a rule enforced when
    //! someone remembers.
    use std::collections::BTreeMap;
    use std::path::Path;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    /// (legacy, managed) pairs, both the generated and the curated ones.
    fn pairs() -> Vec<(String, String)> {
        let path = root().join("presets/managed-constraint-equivalents.txt");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        let mut out = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let mut f = line.split('\t');
            let (legacy, managed) = match (f.next(), f.next()) {
                (Some(l), Some(m)) if !l.is_empty() && !m.is_empty() => (l, m),
                _ => panic!("{}: malformed row: {:?}", path.display(), line),
            };
            let origin = f.next().unwrap_or("");
            assert!(
                origin == "GOOGLE" || origin == "OURS",
                "{}: origin must be GOOGLE or OURS, got {:?} in {:?}",
                path.display(),
                origin,
                line
            );
            if origin == "OURS" {
                assert!(
                    f.next().is_some_and(|n| n.trim().len() > 40),
                    "{}: a pairing Google does not declare needs a note saying why: {:?}",
                    path.display(),
                    legacy
                );
            }
            out.push((legacy.to_string(), managed.to_string()));
        }
        assert!(!out.is_empty(), "the equivalence table is empty");
        out
    }

    /// One emitted org policy: the constraint it targets, and whether the body
    /// switches it OFF (`reset = true`) rather than enforcing it.
    struct Emitted {
        reset: bool,
        origin: String,
    }

    /// Every `google_org_policy_policy` a corpus case emits, by constraint name.
    /// Parsed from the emitted HCL rather than from the pack sources: what the
    /// fold and the params finally produce is what an estate applies.
    fn emitted_policies(case: &Path) -> BTreeMap<String, Emitted> {
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(case.join("main.satz")).unwrap();
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("main.satz", &src, &resolver, &|p| {
            std::fs::read_to_string(case.join(p))
                .or_else(|_| std::fs::read_to_string(root().join(p)))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{}: front-end failed: {}", name, e));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx)
            .unwrap_or_else(|e| panic!("{}: emit failed: {}", name, e));

        let mut found = BTreeMap::new();
        let mut block: Option<Vec<String>> = None;
        for line in out.main_tf.lines() {
            if line.starts_with("resource \"google_org_policy_policy\" ") {
                block = Some(Vec::new());
            } else if line == "}" {
                if let Some(b) = block.take() {
                    let body = b.join("\n");
                    if let Some(c) = body
                        .lines()
                        .find_map(|l| l.trim().strip_prefix("name = \""))
                        .and_then(|l| l.trim_end_matches('"').rsplit("/policies/").next())
                    {
                        found.insert(
                            c.to_string(),
                            Emitted {
                                reset: body.contains("reset = true"),
                                origin: name.clone(),
                            },
                        );
                    }
                }
            } else if let Some(b) = block.as_mut() {
                b.push(line.to_string());
            }
        }
        found
    }

    /// THE gate. Would have caught the pre-2.5 CIS pack, which ran the legacy
    /// protocol-forwarding constraint while its managed twin existed, and left
    /// five other superseded twins undeclared.
    #[test]
    fn no_pack_runs_a_superseded_constraint() {
        let pairs = pairs();
        let corpus = root().join("tests/corpus");
        let mut cases: Vec<_> = std::fs::read_dir(&corpus)
            .expect("corpus dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("main.satz").exists())
            .collect();
        cases.sort();

        let mut policies: BTreeMap<String, Emitted> = BTreeMap::new();
        for case in &cases {
            policies.extend(emitted_policies(case));
        }
        assert!(
            policies.len() > 20,
            "the corpus stopped exercising the org-policy packs — this gate is judging nothing \
             (found {} policies)",
            policies.len()
        );

        let mut problems = Vec::new();
        for (legacy, managed) in &pairs {
            match (policies.get(legacy.as_str()), policies.get(managed.as_str())) {
                // The legacy form is enforced even though a replacement exists.
                (Some(l), _) if !l.reset => problems.push(format!(
                    "{} enforces the legacy `{}`, replaced by `{}`. Emit the managed form and \
                     declare this one `spec {{ reset = true }}`.",
                    l.origin, legacy, managed
                )),
                // The replacement is enforced but nothing says the legacy one is off.
                // Absence is not enough: a legacy policy already set on an organisation
                // is invisible to an apply that does not declare it, so it goes on
                // enforcing beside its twin and has to be deleted by hand.
                (None, Some(m)) if !m.reset => problems.push(format!(
                    "{} enables `{}` without declaring its superseded twin `{}` off. Add a block \
                     with `spec {{ reset = true }}`.",
                    m.origin, managed, legacy
                )),
                _ => {}
            }
        }
        assert!(problems.is_empty(), "\n  - {}\n", problems.join("\n  - "));
    }
}


#[cfg(all(test, unix))]
mod self_update_checksum_tests {
    //! The installer skips its archive check when `sha256sum` is not on PATH, which
    //! was the case on macOS before 14. `self-update` makes the check run wherever
    //! `shasum` exists, and refuses where neither does.
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir_with(name: &str, tools: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("satz-checksum-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (tool, body) in tools {
            let p = d.join(tool);
            fsx::write(&p, body.as_bytes()).unwrap();
            fsx::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
    }

    #[test]
    fn with_sha256sum_the_path_stays_as_it_is() {
        let tools = dir_with("has", &[("sha256sum", "#!/bin/sh\n")]);
        let temp = dir_with("has-temp", &[]);
        assert_eq!(installer_checksum_path(&temp, tools.as_os_str(), false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&tools);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn with_only_shasum_a_shim_runs_it_with_the_installer_s_arguments() {
        let tools = dir_with("shasum", &[("shasum", "#!/bin/sh\necho \"shasum $*\"\n")]);
        let temp = dir_with("shasum-temp", &[]);
        let path = installer_checksum_path(&temp, tools.as_os_str(), false).unwrap().expect("a PATH with the shim");
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, temp.join("checksum-bin"));
        // exactly the installer's call: `sha256sum -b "$_file" | awk '{printf $1}'`
        // `/bin/sh` by path: the PATH under test holds only the two tool directories
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", "sha256sum -b archive.tar.xz"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "shasum -a 256 -b archive.tar.xz");
        let _ = std::fs::remove_dir_all(&tools);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn with_neither_the_update_is_refused_unless_the_check_is_skipped() {
        let empty = dir_with("none", &[]);
        let temp = dir_with("none-temp", &[]);
        let err = installer_checksum_path(&temp, empty.as_os_str(), false).unwrap_err();
        assert!(err.contains("cannot verify the archive") && err.contains("--skip-checksum"), "{err}");
        assert_eq!(installer_checksum_path(&temp, empty.as_os_str(), true).unwrap(), None);
        let _ = std::fs::remove_dir_all(&empty);
        let _ = std::fs::remove_dir_all(&temp);
    }
}

#[cfg(test)]
mod agent_guide_tests {
    /// The agent guide is read by something that will act on it. A syntax error in
    /// an example is not a typo there — it is an instruction to write invalid Satz,
    /// followed by a compile failure the agent has to debug from a doc it trusted.
    ///
    /// This parses every ```satz block in the guide. It is a PARSE gate, not a
    /// compile gate: the fragments deliberately show one construct at a time and
    /// have no estate around them, so schema checks (does this attribute exist on
    /// this type?) are out of reach here. Those are covered where the same
    /// constructs appear in `tests/smoke/yaml/showcase.satz`, which is compiled and
    /// `tofu validate`d by the smoke matrix.
    #[test]
    fn every_example_in_the_agent_guide_parses() {
        let doc = include_str!("../docs/llms.md");
        let mut blocks = Vec::new();
        let mut current: Option<(usize, String)> = None;
        for (n, line) in doc.lines().enumerate() {
            match (&mut current, line.trim_start()) {
                (None, "```satz") => current = Some((n + 2, String::new())),
                (Some(_), "```") => {
                    let (start, body) = current.take().expect("in a block");
                    blocks.push((start, body));
                }
                (Some((_, body)), _) => {
                    body.push_str(line);
                    body.push('\n');
                }
                _ => {}
            }
        }
        assert!(current.is_none(), "an unterminated ```satz block in the guide");
        assert!(
            blocks.len() >= 10,
            "only {} satz examples found — the extractor is looking at the wrong thing",
            blocks.len()
        );
        for (line, body) in &blocks {
            if let Err(e) = satz_core::satz::parse(body) {
                panic!(
                    "docs/llms.md:{}: the example does not parse — {}:{}\n{}",
                    line, e.line, e.msg, body
                );
            }
        }
    }
}

#[cfg(test)]
mod compile_tail_tests {
    //! The checks after the front end, as findings: each at the line it names, at
    //! the severity the validation level gives it. What the CLI prints, the server
    //! shows and MCP returns is this list.
    use super::*;
    use crate::findings::{Kind, Severity};

    const ESTATE: &str = r#"estate tail_case

params {
  customer_organization_id = "123456789012"
  use_budget               = true
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

// use "presets/organization-budget.satz" when use_budget

google_storage_bucket {
  no_location {
    name = "no-location-bucket"
  }
  refers {
    name     = "refers-bucket"
    location = "EU"
    labels   = { folder = "${{google_folder.nope.name}}" }
  }
}

action "step" {
  reason = "a step with no resource"
  run    = "step.sh"
}
"#;

    fn tail(level: &str) -> Tail {
        tail_of(ESTATE, level)
    }

    fn tail_of(src: &str, level: &str) -> Tail {
        let reg = super::corpus::registry();
        let resolver = EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("tail.satz", src, &resolver, &|p| Err(format!("no {}", p)))
            .unwrap_or_else(|e| panic!("front-end failed: {}", e));
        let cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        let graph = crate::pack_graph::Shipped::Graph(crate::template::tests::shipped(), Path::new(env!("CARGO_MANIFEST_DIR")).join("presets"));
        compile_tail(&fe, &resolver, &reg, &cfg, &graph, level, Path::new("tail.satz"), src)
    }

    fn line_of(needle: &str) -> u32 {
        ESTATE.lines().position(|l| l.contains(needle)).map(|i| i as u32 + 1).unwrap()
    }

    #[test]
    fn a_missing_required_attribute_is_a_warning_at_the_declaring_block() {
        let t = tail("warn");
        let f = t.findings.iter().find(|f| f.kind == Kind::MissingRequired).expect("the bucket without a location");
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("google_storage_bucket.no_location") && f.message.contains("location"), "{}", f.message);
        assert_eq!((f.file.as_deref(), f.line), (Some("tail.satz"), Some(line_of("no_location {"))));
    }

    #[test]
    fn the_validation_level_turns_it_into_an_error_or_drops_it() {
        assert_eq!(tail("error").findings.iter().find(|f| f.kind == Kind::MissingRequired).map(|f| f.severity), Some(Severity::Error));
        assert!(tail("none").findings.iter().all(|f| f.kind != Kind::MissingRequired));
        assert!(tail("error").findings.iter().find(|f| f.kind == Kind::MissingRequired).unwrap().group.is_some(), "an error is grouped for the CLI");
    }

    #[test]
    fn a_reference_to_an_unemitted_address_is_an_error_at_its_line() {
        let t = tail("warn");
        let f = t.findings.iter().find(|f| f.kind == Kind::WrittenReference).expect("the folder nobody emits");
        assert_eq!(f.severity, Severity::Error);
        assert!(f.message.contains("google_folder.nope.name") && f.message.contains("no `google_folder` is emitted here at all"), "{}", f.message);
        assert_eq!(f.line, Some(line_of("refers {")), "at the declaring block: a body keeps one site");
    }

    #[test]
    fn a_pack_answered_for_but_commented_out_is_found_at_its_line() {
        let t = tail("warn");
        let f = t.findings.iter().find(|f| f.kind == Kind::UnadoptedPack).expect("the commented budget line");
        assert!(f.message.contains("use_budget") && f.message.contains("still commented out"), "{}", f.message);
        assert_eq!(f.line, Some(line_of("// use \"presets/organization-budget.satz\"")));
        // the command is the finding's `fix`, for this estate, and the sentence does not repeat it
        assert_eq!(f.fix.as_deref(), Some("satz add-pack tail.satz presets/organization-budget.satz"));
        assert!(!f.message.contains("satz add-pack"), "{}", f.message);
    }

    /// Presets without `pack-graph.json`: the checks that read the menu are skipped, and
    /// one note says why — never a crash, never silence.
    #[test]
    fn a_missing_pack_graph_is_one_note() {
        let reg = super::corpus::registry();
        let resolver = EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("tail.satz", ESTATE, &resolver, &|p| Err(format!("no {}", p))).unwrap();
        let cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        let graph = crate::pack_graph::Shipped::Missing(PathBuf::from("presets/pack-graph.json"));
        let t = compile_tail(&fe, &resolver, &reg, &cfg, &graph, "warn", Path::new("tail.satz"), ESTATE);
        let menu: Vec<_> = t.findings.iter().filter(|f| f.kind == Kind::UnadoptedPack).collect();
        assert_eq!(menu.len(), 1, "{menu:?}");
        assert_eq!(menu[0].severity, Severity::Info);
        assert!(menu[0].message.contains("presets/pack-graph.json is not here"), "{}", menu[0].message);
    }

    #[test]
    fn an_action_is_a_warning_at_its_line_and_the_cli_text_is_the_finding() {
        let t = tail("warn");
        let f = t.findings.iter().find(|f| f.kind == Kind::Action).expect("the action");
        assert_eq!((f.severity, f.line), (Severity::Warning, Some(line_of("action \"step\""))));
        // where it stands is the finding's location, said once: the sentence does not repeat it
        assert_eq!(f.file.as_deref(), Some("tail.satz"));
        assert_eq!(f.subject.as_deref(), Some("step"));
        assert!(f.message.starts_with("`satz run-actions` will execute"), "{}", f.message);
        // the tail ran to the end: the emitter's output and the providers are there
        assert!(t.out.is_some() && t.providers_tf.is_some());
    }

    /// A bucket declared outside any project, setting none: a finding at its block, so
    /// the caller decides whether it is printed — a review's compile prints nothing. It
    /// stays a warning at every level: the provider block's project is a valid home.
    #[test]
    fn a_resource_outside_a_project_that_sets_none_is_a_warning_at_its_block() {
        for level in ["warn", "error", "none"] {
            let t = tail(level);
            let f = t
                .findings
                .iter()
                .find(|f| f.kind == Kind::MissingScope && f.message.starts_with("google_storage_bucket.no_location"))
                .unwrap_or_else(|| panic!("level {}: no finding for the unscoped bucket: {:?}", level, t.findings));
            assert_eq!(f.severity, Severity::Warning, "level {}", level);
            assert!(f.message.contains("sets no `project`"), "{}", f.message);
            assert_eq!((f.file.as_deref(), f.line), (Some("tail.satz"), Some(line_of("no_location {"))));
        }
    }

    /// The emitter writes a backend for `local` and `cloud` only: any other mode is an
    /// error at the line binding it, and the compile refuses rather than emitting
    /// `providers.tf` without a backend.
    #[test]
    fn a_deployment_mode_with_no_backend_is_an_error_at_its_line() {
        let src = ESTATE.replacen(
            "  use_budget               = true\n",
            "  use_budget               = true\n  deployment_mode          = \"boot\"\n",
            1,
        );
        let t = tail_of(&src, "warn");
        let f = t.findings.iter().find(|f| f.kind == Kind::DeploymentMode).expect("the mode no backend is emitted for");
        assert_eq!(f.severity, Severity::Error);
        assert!(f.message.contains("\"boot\"") && f.message.contains("\"local\"") && f.message.contains("\"cloud\""), "{}", f.message);
        let line = src.lines().position(|l| l.contains("deployment_mode")).map(|i| i as u32 + 1);
        assert_eq!((f.file.as_deref(), f.line), (Some("tail.satz"), line));
        assert!(t.providers_tf.is_none(), "providers.tf was emitted without a backend");
        let refused = crate::findings::refusal(&t.findings).unwrap_err();
        assert!(refused.contains("deployment_mode = \"boot\""), "{}", refused);
        // the estate binds no mode: local, and the compile goes through
        assert!(tail("warn").findings.iter().all(|f| f.kind != Kind::DeploymentMode));
    }

    /// Cloud mode runs as `{svc_iac_account}@{infra_project_name}`: an estate that binds
    /// it without both is an error at the `deployment_mode` line naming what is missing,
    /// at every validation level — it would otherwise run as whoever is logged in.
    #[test]
    fn cloud_mode_without_the_account_is_an_error_at_the_mode_line() {
        let src = ESTATE.replacen(
            "  use_budget               = true\n",
            "  use_budget               = true\n  infra_project_name       = \"acme-infra-001\"\n  deployment_mode          = \"cloud\"\n",
            1,
        );
        let line = src.lines().position(|l| l.contains("deployment_mode")).map(|i| i as u32 + 1);
        for level in ["warn", "error", "none"] {
            let t = tail_of(&src, level);
            let f = t.findings.iter().find(|f| f.kind == Kind::DeploymentMode).expect("cloud mode without an account");
            assert_eq!(f.severity, Severity::Error, "level {}", level);
            assert!(f.message.starts_with("`deployment_mode = \"cloud\"` without a value for `svc_iac_account`: "), "{}", f.message);
            assert_eq!((f.file.as_deref(), f.line), (Some("tail.satz"), line));
            assert!(t.providers_tf.is_none(), "providers.tf was emitted for an estate that runs as nobody");
        }
        let both = src.replacen("  deployment_mode", "  svc_iac_account          = \"svc-iac-001\"\n  deployment_mode", 1);
        assert!(tail_of(&both, "warn").findings.iter().all(|f| f.kind != Kind::DeploymentMode));
    }
}

#[cfg(test)]
mod probe_quiet_tests {
    //! `whoami`'s probe compiles the estate with its prerequisite findings quiet, since
    //! it tests the permissions live. The quiet is the probe's compile's alone: `satz mcp`
    //! runs the probe for `satz_whoami` and every later compile in the same process.
    use super::*;
    use crate::findings::Kind;

    /// A bucket, and no grant or API for it: the IaC service account lacks the role,
    /// and the infra project does not enable the storage API.
    const ESTATE: &str = r#"estate probe_case

params {
  customer_organization_id = "123456789012"
  svc_iac_account          = "svc-iac-001"
  infra_project_name       = "corp-infra-001"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_storage_bucket {
  logs {
    name     = "probe-logs"
    location = "EU"
  }
}
"#;

    fn estate_at(level: &str) -> (PathBuf, ToolConfig) {
        let dir = std::env::temp_dir().join(format!("satz-probe-quiet-{}-{}", std::process::id(), level));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let estate = dir.join("probe.satz");
        std::fs::write(&estate, ESTATE).unwrap();
        let mut cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.schema_dir = super::corpus::schema_dir();
        cfg.validation_level = level.to_string();
        (estate, cfg)
    }

    fn reports_the_gap(out: &PipelineBOut) -> bool {
        out.findings.iter().any(|f| f.kind == Kind::Prerequisites)
    }

    fn refuses_on_the_gap(r: Result<PipelineBOut, Box<dyn std::error::Error>>) -> bool {
        match r {
            Ok(_) => false,
            Err(e) => crate::findings::refusal_findings(e.as_ref()).iter().any(|f| f.kind == Kind::Prerequisites),
        }
    }

    #[test]
    fn a_compile_after_the_probe_still_reports_the_missing_prerequisite() {
        let (estate, cfg) = estate_at("warn");
        let before = pipeline_b_generate(&estate, &cfg, &cfg).expect("compiles at warn");
        assert!(reports_the_gap(&before), "the fixture lacks a prerequisite: {:?}", before.findings);
        let probe = iac_probe(&estate, &cfg, &cfg).expect("the probe compiles");
        assert!(!probe.needs.is_empty(), "the probe derived no need from the bucket");
        let after = pipeline_b_generate(&estate, &cfg, &cfg).expect("compiles at warn");
        assert!(reports_the_gap(&after), "the probe silenced a later compile: {:?}", after.findings);
        let _ = std::fs::remove_dir_all(estate.parent().unwrap());
    }

    /// At `error` the finding is a refusal, and the probe that is quiet about it must
    /// not take the refusal away from the compile that follows.
    #[test]
    fn at_level_error_the_refusal_survives_the_probe() {
        let (estate, cfg) = estate_at("error");
        assert!(refuses_on_the_gap(pipeline_b_generate(&estate, &cfg, &cfg)), "the gap refuses at error");
        iac_probe(&estate, &cfg, &cfg).expect("the probe's own compile is quiet, so it does not refuse");
        assert!(refuses_on_the_gap(pipeline_b_generate(&estate, &cfg, &cfg)), "the probe removed a refusal");
        let _ = std::fs::remove_dir_all(estate.parent().unwrap());
    }
}

#[cfg(test)]
mod review_quiet_tests {
    //! `review-pack` compiles its scratch estate with nothing printed: the findings are
    //! its report. The silence is that compile's alone — `satz mcp` serves
    //! `satz_review_pack` beside every other call, concurrently, and a compile running
    //! beside a review prints its warnings as it would alone.
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A bucket and no grant for it: at `warn`, the compile prints that the IaC service
    /// account lacks the storage role.
    const ESTATE: &str = r#"estate beside_review

params {
  customer_organization_id = "123456789012"
  svc_iac_account          = "svc-iac-001"
  infra_project_name       = "acme-infra-001"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_storage_bucket {
  logs {
    name     = "acme-beside-review-logs"
    location = "EU"
  }
}
"#;

    const PACK: &str = r#"// A log bucket, reviewed while another estate compiles.
pack beside_compile version "1.0"

google_storage_bucket {
  review_logs {
    name     = "acme-review-logs"
    location = "EU"
  }
}
"#;

    /// Stops the reviewer however the test ends, so a failed assertion does not leave it
    /// reviewing through every test after this one.
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// `transpile --format json` is the object `satz_transpile_check` returns, whether the
    /// compile goes on or refuses — and a refusal by the FRONT END is one too, with its
    /// file and line as fields, never only inside a message. It prints nothing.
    #[test]
    fn the_compile_as_data_is_one_shape_whether_it_compiles_or_is_refused() {
        let dir = std::env::temp_dir().join(format!("satz-compile-summary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.schema_dir = super::corpus::schema_dir();
        cfg.validation_level = "warn".to_string();
        cfg.dir = Some(dir.clone());
        let good = dir.join("good.satz");
        std::fs::write(&good, ESTATE).unwrap();
        crate::findings::take_said();
        let (summary, refused) = compile_summary(&good, "good.satz", None, true, &cfg, &cfg).expect("a verdict");
        assert!(crate::findings::take_said().is_empty(), "the findings are in the object; nothing is printed beside it");
        assert!(!refused && !summary.addresses.is_empty() && summary.written.is_empty(), "{:?}", summary);
        let lacks = summary.findings.iter().find(|f| f.kind == crate::findings::Kind::Prerequisites).expect("the roles it lacks");
        assert_eq!(lacks.fix.as_deref(), Some("satz update-prerequisites good.satz"));
        assert!(!lacks.message.contains("satz update-prerequisites"), "the command moved out of the sentence: {}", lacks.message);
        let json = serde_json::to_value(&summary).unwrap();
        let row = json["findings"].as_array().unwrap().iter().find(|f| f["kind"] == "prerequisites").unwrap();
        assert_eq!(row["fix"], "satz update-prerequisites good.satz", "`fix` is a field beside `message`: {}", row);

        let bad = dir.join("bad.satz");
        std::fs::write(&bad, format!("{}\ngoogle_storage_bucket {{\n  params {{\n    a = 1\n  }}\n}}\n", ESTATE)).unwrap();
        let (summary, refused) = compile_summary(&bad, "bad.satz", None, true, &cfg, &cfg).expect("a refusal is a verdict");
        assert!(refused && summary.addresses.is_empty());
        let f = &summary.findings[0];
        assert_eq!((f.severity, f.kind), (crate::findings::Severity::Error, crate::findings::Kind::FrontEnd));
        assert_eq!(f.file.as_deref(), Some("bad.satz"), "relative to the estate's directory, as every finding's file is");
        assert!(f.line.is_some() && f.message.contains("`params` is a Satz statement"), "{:?}", f);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_compile_beside_a_review_still_prints_its_warnings() {
        let dir = std::env::temp_dir().join(format!("satz-review-beside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let estate = dir.join("beside.satz");
        std::fs::write(&estate, ESTATE).unwrap();
        let pack = dir.join("beside-compile.satz");
        std::fs::write(&pack, PACK).unwrap();
        let mut cfg = parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.schema_dir = super::corpus::schema_dir();
        cfg.validation_level = "warn".to_string();
        let prints_the_warning = || {
            crate::findings::take_said();
            pipeline_b_generate(&estate, &cfg, &cfg).expect("compiles at warn");
            let said = crate::findings::take_said();
            (said.iter().any(|s| s.contains("roles/storage.admin")), said)
        };
        let (alone, said) = prints_the_warning();
        assert!(alone, "the fixture's compile prints no warning even alone: {:?}", said);

        let stop = Stop(Arc::new(AtomicBool::new(false)));
        let reviews = Arc::new(AtomicUsize::new(0));
        let reviewer = {
            let (pack, cfg, stop, reviews) = (pack.clone(), cfg.clone(), stop.0.clone(), reviews.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    crate::findings::take_said();
                    crate::review_pack::review(&pack, None, &cfg, &cfg).expect("the review runs");
                    let said = crate::findings::take_said();
                    assert!(said.is_empty(), "the review printed its compile's findings: {:?}", said);
                    reviews.fetch_add(1, Ordering::Relaxed);
                }
            })
        };
        // Compiles back to back for as long as ten whole reviews take: a quiet that any
        // review could switch for the whole process lands in some of them.
        let mut n = 0usize;
        while (n < 20 || reviews.load(Ordering::Relaxed) < 10) && !reviewer.is_finished() {
            let (printed, said) = prints_the_warning();
            assert!(printed, "compile {} beside a review printed no warning: {:?}", n, said);
            n += 1;
        }
        drop(stop);
        reviewer.join().expect("the reviewer failed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
