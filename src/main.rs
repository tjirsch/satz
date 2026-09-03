mod config;
mod fsx;
mod schema;
mod emit_shared;
mod emitter;
mod manifest;
mod state_migration;
mod discovery;
mod delta;
mod align;
mod scan;
mod template;
mod adopt;
mod bootstrap;
mod preflight;
mod gcp;
mod org_policy;
mod cloud_identity;
mod compliance;
mod dossier;
mod presets;
mod doc_packs;
mod github;
mod policy_tree;

use clap::{Parser, Subcommand, CommandFactory};
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
    #[serde(default)]
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
}

impl ToolConfig {
    pub fn all_providers(&self) -> Vec<String> {
        let mut providers = Vec::new();
        providers.extend(self.google_providers.iter().map(|p| ToolConfig::parse_provider_string(p).0));
        providers.extend(self.aws_providers.iter().map(|p| ToolConfig::parse_provider_string(p).0));
        providers.extend(self.azure_providers.iter().map(|p| ToolConfig::parse_provider_string(p).0));
        providers.extend(self.alibaba_providers.iter().map(|p| ToolConfig::parse_provider_string(p).0));
        providers
    }

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
fn default_version() -> String { "7.12.0".to_string() }
fn default_auto_explode() -> Vec<String> {
    vec![
        "google_project_service".to_string(),
        ".*_iam_member".to_string(),
    ]
}
fn default_validation_level() -> String { "warn".to_string() }


#[derive(Parser)]
#[command(author, version, about, long_about = None, max_term_width = 110)]
struct Cli {
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

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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
        /// Derive the missing values from the Application Default Credentials alone: identity → first admin + domain, organizations:search → org id + directory customer id, billing accounts → the single open account
        ///
        /// Explicit flags always win; nothing is ever guessed
        #[arg(long)]
        from_live: bool,
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
        /// Write the report to this path (else stdout)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Report format: console (default), markdown, json
        #[arg(long, default_value = "console")]
        format: String,
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
        /// Report format: markdown (default), json, pdf (pdf needs pandoc on PATH)
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Output path (default: <yaml_dir>/<Cxxxx>-orgpolicies-report.<ext>)
        #[arg(long)]
        report: Option<PathBuf>,
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
        /// Output file inside yaml_dir (state/live shapes; default discovered.satz)
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Import configuration (default: <presets_dir>/import-config.yaml)
        #[arg(long)]
        import_config: Option<PathBuf>,
        /// yaml shape: estate used to compile a converted pack in context
        #[arg(long)]
        gate: Option<PathBuf>,
        /// yaml shape: declared kind of the converted file
        #[arg(long, default_value = "pack")]
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
    },

    /// Migrate state and configuration between local and cloud modes
    Migrate {
        /// Estate file (inside yaml_dir if relative): rewrites `deployment_mode`
        /// in its params
        input: String,
        /// Target mode (local or cloud)
        #[arg(long)]
        mode: Option<String>,
    },
    /// Check for and install new releases from GitHub
    SelfUpdate {
        /// Do not open the documentation site after installing
        #[arg(long)]
        no_open_readme: bool,
        /// Only check if an update is available; do not install or download README
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
    /// Reconciling preset update: install new packs (recording base snapshots),
    /// silently upgrade unmodified ones, emit variable-migration hints for
    /// template drift, and for CONTENT packs (edited in place by design) never
    /// overwrite - write `<pack>.new` plus a three-way merge against the base.
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
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Report file path (default: evidence/<framework>-latest.md beside config)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Prowler native-JSON findings file to ingest as corroboration
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
        /// Prowler export (OCSF or legacy JSON)
        #[arg(long)]
        prowler: PathBuf,
        /// markdown (default) or json
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Write the plan here instead of stdout
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Build the remediation dossier — the findings workbook minus the prose
    ///
    /// Every Prowler FAIL/MANUAL (and Checkov finding) triaged against the
    /// estate's claims, joined per resource, counted, and written under the
    /// estate's evidence/ directory as JSON, CSV and XLSX. The mechanical
    /// columns are filled; the `[AI]` columns and the Review column are the
    /// consultant's (or a later model pass's). Offline, deterministic: the
    /// dossier hash names the run
    RemediationPlan {
        /// Catalog id, e.g. cis-gcp-4.0
        framework: String,
        /// Estate file (.satz, inside yaml_dir if relative)
        input: String,
        /// Prowler export (OCSF or legacy JSON)
        #[arg(long)]
        prowler: PathBuf,
        /// Also run Checkov over hcl_dir and join its findings
        #[arg(long)]
        checkov: bool,
        /// Output directory (default: <config dir>/evidence/plan/<framework>-<timestamp>)
        #[arg(long)]
        out: Option<PathBuf>,
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
        /// Output directory (default: `<presets_dir>/docs`)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Verify instead of write: exit 1 when a page is behind its pack
        #[arg(long)]
        check: bool,
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
    TfInit {
        /// Arguments passed straight to the tool, e.g. `-reconfigure`, `-migrate-state`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Open the documentation site in the browser
    OpenReadme,
    /// Show which identity, credential type and quota project the Application
    /// Default Credentials resolve to
    Whoami {
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

/// User-level settings for satz in ~/.config/satz/satz.toml. Created on first run with defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GlobalSettings {
    /// When to check for updates: "never", "always", "daily". Default "always".
    #[serde(default = "default_self_update_frequency")]
    self_update_frequency: String,
    /// Last time we ran an update check (unix timestamp string). Used for "daily" throttle.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_update_check: Option<String>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            self_update_frequency: default_self_update_frequency(),
            last_update_check: None,
        }
    }
}

fn default_self_update_frequency() -> String {
    "always".to_string()
}

fn global_settings_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("satz").join("satz.toml"))
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
        return toml::from_str(&content).map_err(|e| {
            format!("{}: not valid TOML ({}) — fix the file or delete it to start from defaults", path.display(), e).into()
        });
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("satz v{} (built {})", env!("CARGO_PKG_VERSION"), env!("BUILD_DATE"));
    // parse into matches first: the subcommand NAME is what --html-help needs,
    // and clap only hands it out at this level
    let matches = <Cli as clap::CommandFactory>::command().get_matches();
    let subcommand = matches.subcommand_name().map(|s| s.to_string());
    let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches)?;
    if cli.html_help {
        return open_html_help(subcommand.as_deref());
    }

    // Load/create global settings on first run (creates ~/.config/satz/satz.toml with defaults)
    let mut global_settings = load_global_settings()?;

    let cmd_choice = match cli.command {
        Some(c) => c,
        None => {
            if cli.verbose {
                let mut cmd = Cli::command();
                print_recursive_help(&mut cmd);
            } else {
                let mut cmd = Cli::command();
                let _ = cmd.print_help();
                println!();
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
        } else {
            path.clone()
        }
    } else {
        let default_config = PathBuf::from("config.toml");
        if default_config.exists() {
            default_config
        } else {
            // Config is mandatory for Transpile and other commands that need it
            match cmd_choice {
                Commands::Transpile { .. } | Commands::ScanPlan { .. } | Commands::GenerateMigration { .. } | Commands::UpdateSchema { .. } | Commands::Import { .. } | Commands::Migrate { .. } | Commands::Bootstrap { .. } | Commands::ExportOrganizationalPolicies { .. } | Commands::DiffOrganizationalPolicies { .. } | Commands::ReportOrganizationalPolicies { .. } | Commands::GetPresets { .. } | Commands::CheckPresets { .. } | Commands::Require { .. } | Commands::ReportCompliance { .. } | Commands::Adopt { .. } | Commands::MapTypes { .. } | Commands::Scan { .. } | Commands::DocPacks { .. } | Commands::Triage { .. } | Commands::RemediationPlan { .. } | Commands::AdoptOrgPolicies { .. } | Commands::MergePresets { .. }
                | Commands::Plan { .. } | Commands::Apply { .. } | Commands::TfInit { .. } => {
                    // plan/apply/tf-init hand everything after the subcommand to the
                    // tool verbatim, which also swallows a `--config` written after
                    // those args. "config.toml not found" is baffling then, so name
                    // the actual fix.
                    // Printed rather than returned: `main` renders a returned error
                    // with `Debug`, which escapes the newline into a literal \n.
                    if let Some(hint) = misplaced_config_hint(&cmd_choice) {
                        eprintln!("\n{}\n", hint);
                        return Err("--config came after the pass-through arguments".into());
                    }
                    return Err("Config file 'config.toml' not found in current directory. Please provide it or specify --config <PATH>.".into());
                }
                Commands::Init { .. } | Commands::SelfUpdate { .. } | Commands::Completion { .. } | Commands::OpenReadme | Commands::Whoami { .. } => {
                    // These commands can proceed without a config file
                    PathBuf::from("config.toml")
                }
            }
        }
    };

    // Optional: check for updates per global settings (skip for SelfUpdate and Init)
    if !matches!(cmd_choice, Commands::SelfUpdate { .. } | Commands::Init { .. } | Commands::Whoami { .. }) {
        let _ = maybe_check_for_updates(&mut global_settings).await;
    }

    // --no-impersonate wins over everything: locking the target to None here
    // makes every later per-command configuration a no-op.
    if cli.no_impersonate {
        crate::gcp::configure_impersonation(None);
    }

    let config_dir = config_file_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let tool_config: ToolConfig = if config_file_path.exists() {
        let content = fsx::read_to_string(&config_file_path)?;
        // Printed rather than returned: `main` renders a returned error with `Debug`,
        // which escapes the newlines and inlines the entire file.
        toml::from_str(&content).map_err(|e| {
            eprintln!("\n{}\n", describe_toml_error(&config_file_path, &content, &e));
            format!("could not parse '{}' as TOML", config_file_path.display())
        })?
    } else {
        ToolConfig {
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
        }
    };

    // Create a copy for runtime use with resolved paths
    let mut runtime_config = tool_config.clone();

    // Resolve relative paths in runtime_config relative to the config file directory
    if Path::new(&runtime_config.yaml_dir).is_relative() {
        runtime_config.yaml_dir = config_dir.join(&runtime_config.yaml_dir).to_str().unwrap().to_string();
    }
    if Path::new(&runtime_config.hcl_dir).is_relative() {
        runtime_config.hcl_dir = config_dir.join(&runtime_config.hcl_dir).to_str().unwrap().to_string();
    }
    if Path::new(&runtime_config.schema_dir).is_relative() {
        runtime_config.schema_dir = config_dir.join(&runtime_config.schema_dir).to_str().unwrap().to_string();
    }
    if Path::new(&runtime_config.presets_dir).is_relative() {
        runtime_config.presets_dir = config_dir.join(&runtime_config.presets_dir).to_str().unwrap().to_string();
    }
    runtime_config.include_dirs = runtime_config.include_dirs.into_iter().map(|d| {
        if Path::new(&d).is_relative() {
            config_dir.join(d).to_str().unwrap().to_string()
        } else {
            d
        }
    }).collect();


    match cmd_choice {
        Commands::Transpile { input, output, schema_dir, print_variables, plan, apply, scan, check } => {

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
            let out = pipeline_b_generate(&input_path, &tool_config, &runtime_config)?;
            let (main_tf, providers_tf, variables_tf, tfvars, imports_tf) =
                (&out.main_tf, &out.providers_tf, &out.variables_tf, &out.tfvars, &out.imports_tf);
            if print_variables {
                println!("{}", tfvars);
            }
            if check {
                println!(
                    "transpile --check: OK — {} compiles; nothing was written",
                    input_path.display()
                );
                return Ok(());
            }
            let (main_tf, providers_tf, variables_tf, tfvars, imports_tf) =
                (main_tf.as_str(), providers_tf.as_str(), variables_tf.as_str(), tfvars.as_str(), imports_tf.as_str());
            // --output relocates the emitted HCL; relative to hcl_dir, as before.
            let base_output_path = match &output {
                Some(o) if Path::new(o).is_absolute() => PathBuf::from(o),
                Some(o) => PathBuf::from(&runtime_config.hcl_dir).join(o),
                None => PathBuf::from(&runtime_config.hcl_dir),
            };
            if !base_output_path.exists() {
                fsx::create_dir_all(&base_output_path)?;
            }
            let imports_path = base_output_path.join("imports.tf");
            if imports_path.exists() {
                fsx::remove_file(&imports_path)?;
            }
            let write_file = |filename: &str, content: &str| -> std::io::Result<()> {
                if content.trim().is_empty() {
                    return Ok(());
                }
                let p = base_output_path.join(filename);
                fsx::write(&p, content)?;
                println!("Created {}", p.display());
                Ok(())
            };
            write_file("main.tf", main_tf)?;
            write_file("providers.tf", providers_tf)?;
            write_file("variables.tf", variables_tf)?;
            write_file("terraform.tfvars", tfvars)?;
            write_file("imports.tf", imports_tf)?;
            if plan || apply {
                // one tool: transpile, then the tool, in the estate's hcl dir
                if output.is_some() {
                    return Err("--plan/--apply run in hcl_dir; drop --output".into());
                }
                let hcl_dir = Path::new(&runtime_config.hcl_dir);
                if !hcl_dir.join(".terraform").exists() {
                    run_tf(&runtime_config, "init", &["-input=false".to_string()])?;
                }
                run_tf(&runtime_config, if apply { "apply" } else { "plan" }, &[])?;
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

            // 3b. --from-live: derive the missing values from the ADC alone.
            // Explicit flags always win, and nothing is ever guessed.
            let (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user) =
                if from_live {
                    let live = crate::gcp::identity::live_defaults(
                        customer_organization_id.is_none() || customer_id.is_none(),
                        billing_account_infra.is_none(),
                    )
                    .await?;
                    let customer_domain = customer_domain.or_else(|| Some(live.customer_domain.clone()));
                    let iac_user =
                        iac_user.or_else(|| Some(format!("{}@{}", live.first_admin, live.customer_domain)));
                    let customer_id = customer_id.or_else(|| live.customer_id.clone());
                    // No organization visible = greenfield: the estate is
                    // written with an empty id and bootstrap --greenfield
                    // fills it in.
                    let customer_organization_id =
                        customer_organization_id.or_else(|| Some(live.org_id.clone().unwrap_or_default()));
                    let billing_account_infra = billing_account_infra.or_else(|| live.billing_account.clone());
                    let customer_shortname = match customer_shortname {
                        Some(s) => Some(s),
                        None => Some(crate::gcp::identity::prompt_shortname()?),
                    };
                    if customer_id.is_none() {
                        return Err("no organization (and so no directory customer id) is visible — \
                                    pass --customer-id explicitly alongside --from-live"
                            .into());
                    }
                    if billing_account_infra.as_deref().unwrap_or("").is_empty() {
                        return Err("--from-live could not settle on ONE open billing account — \
                                    pass --billing-account-infra (the visible accounts are listed above)"
                            .into());
                    }
                    if customer_organization_id.as_deref().unwrap_or("").is_empty() {
                        println!(
                            "no organization is visible to these credentials — the estate is written \
                             with an empty customer_organization_id; `satz bootstrap <estate> --greenfield` \
                             materializes the organization and fills it in"
                        );
                    }
                    (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
                } else {
                    (customer_id, customer_shortname, billing_account_infra, customer_organization_id, customer_domain, iac_user)
                };

            // 4. Generate the template estate if customer_id provided
            if let Some(c_id) = customer_id {
                let yaml_path = PathBuf::from(&runtime_config.yaml_dir).join(format!("{}.satz", c_id));
                if !yaml_path.exists() {
                    let domain = customer_domain.clone().unwrap_or_default();
                    let resolved_iac_user = iac_user.unwrap_or_else(|| format!("first.admin@{}", domain));

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

                    let args = crate::template::TemplateArgs {
                        customer_id: c_id.clone(),
                        shortname: customer_shortname.unwrap_or_default(),
                        billing_id: billing_account_infra.unwrap_or_default(),
                        region: default_region.unwrap_or_else(|| "europe-west3".to_string()),
                        org_id: customer_organization_id.unwrap_or_else(|| "123456789012".to_string()),
                        domain: domain.clone(),
                        project_id: infra_project_name.unwrap_or_default(),
                        bucket_id: infra_bucket_name.unwrap_or_default(),
                        first_admin: first_admin.to_string(),
                    };
                    crate::template::generate_template(&args, &yaml_path)?;
                    println!("Generated estate: {} — next: `satz bootstrap {}.satz --dry-run`", yaml_path.display(), c_id);
                } else {
                    println!("Template already exists: {}", yaml_path.display());
                }
            }

            // 4. Fetch Schemas
            let mut all_provs = final_google;
            all_provs.extend(final_aws);
            all_provs.extend(final_azure);
            all_provs.extend(final_alibaba);

            if !all_provs.is_empty() {
                for p in all_provs {
                    println!("Fetching schema for {}...", p);
                    crate::schema::ResourceRegistry::generate_schema(
                        &tool,
                        &p,
                        &runtime_config.provider_version,
                        &format!("{}/{}.json", runtime_config.schema_dir, p)
                    )?;
                }
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
                     ResourceRegistry::generate_schema(&tool, &p_name, &p_ver, out.to_str().unwrap())?;
                 }
            } else {
                 // Use parsed config
                 for (p_name, p_ver) in tool_config.parsed_providers() {
                      // Override if version passed (unlikely for bulk update but possible)
                      let usage_ver = version.clone().unwrap_or(p_ver);
                      let out = PathBuf::from(format!("{}/{}.json", runtime_config.schema_dir, p_name.split('/').next_back().unwrap_or(&p_name)));
                      println!("Updating schema for {} version {} using {}...", p_name, usage_ver, tool);
                      ResourceRegistry::generate_schema(&tool, &p_name, &usage_ver, out.to_str().unwrap())?;
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
        Commands::Import { source, from, only, output, import_config, gate, kind, fork, into, wrap_all } => {
            let cfg_opt = load_import_config(import_config, &tool_config, &runtime_config.presets_dir)?;
            let shape = match from {
                Some(f) => f,
                None => detect_import_shape(source.as_deref(), cfg_opt.as_ref().and_then(|c| c.root.as_ref()))?,
            };
            match shape.as_str() {
                "yaml" => {
                    let src = source.ok_or("the yaml shape needs a file to convert")?;
                    convert_yaml_to_satz(PathBuf::from(src), gate, kind, fork, &tool_config, &runtime_config)
                }
                "hcl" => {
                    let src = source.ok_or("the hcl shape needs a directory of .tf files, or one file")?;
                    let output = output.unwrap_or_else(|| PathBuf::from("imported-hcl.satz"));
                    import_hcl(&src, output, wrap_all, cli.verbose, &runtime_config)
                }
                "state" | "org" => {
                    let mut cfg = cfg_opt.ok_or_else(|| missing_import_config(&runtime_config.presets_dir))?;
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
                    let output = output.unwrap_or_else(|| PathBuf::from("discovered.satz"));
                    if into.is_some() && shape != "org" {
                        return Err("--into applies to the live shape (organizations/…, folders/…, projects/…)".into());
                    }
                    if shape == "state" {
                        let state_json = match source.as_deref() {
                            None | Some("-") => None,
                            Some(p) => Some(PathBuf::from(p)),
                        };
                        import_state(state_json, output, cfg, filtered, cli.verbose, &tool_config, &runtime_config)
                    } else {
                        let parent = resolve_import_parent(source.as_deref(), cfg.root.as_ref()).await?;
                        match into {
                            Some(estate) => import_delta(&parent, estate_path(estate, &runtime_config), cfg, filtered, cli.verbose, &tool_config, &runtime_config).await,
                            None => import_org(&parent, output, cfg, filtered, cli.verbose, &runtime_config).await,
                        }
                    }
                }
                other => Err(format!("unknown import shape {:?} — one of state, org, yaml, hcl", other).into()),
            }
        }
        Commands::Bootstrap { estate, dry_run, greenfield } => {
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            crate::bootstrap::bootstrap(
                config_path,
                dry_run,
                greenfield,
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
            configure_estate_impersonation(&config_path, &runtime_config);
            crate::org_policy::export_org_policies(
                config_path,
                customer_organization_id,
                output,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::DiffOrganizationalPolicies { estate, customer_organization_id, report, format, recursive } => {
            // The params table and the declared policy set both come from the
            // fragment pipeline; the desired set is what the estate emits.
            let config_path = estate_path(estate, &runtime_config);
            configure_estate_impersonation(&config_path, &runtime_config);
            crate::org_policy::diff_org_policies(
                config_path,
                customer_organization_id,
                report,
                format,
                recursive,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::ReportOrganizationalPolicies { estate, customer_organization_id, scope, format, report, recursive } => {
            // Satz-native: bootstrap needs the variable table, nothing more.
            let config_path = estate_path(estate, &runtime_config);
            configure_estate_impersonation(&config_path, &runtime_config);
            crate::org_policy::report_org_policies(
                config_path,
                customer_organization_id,
                scope,
                format,
                report,
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
            let content = fsx::read_to_string(&input_path)?;

            // Detect current mode from the `deployment_mode` param. Absent is an
            // error, not "local": the guard used to be the regex itself, so an
            // estate without the param silently reported "already in local mode".
            let re_mode = regex::Regex::new(r#"(?m)^\s*deployment_mode\s*=\s*"(\w+)""#).unwrap();
            let re_line = regex::Regex::new(r#"(?m)^(\s*)deployment_mode(\s*)=\s*"\w+"[^\n]*$"#).unwrap();
            let current_mode = re_mode
                .captures(&content)
                .map(|c| c[1].to_string())
                .ok_or_else(|| {
                    format!(
                        "{} declares no deployment mode (`deployment_mode = \"local\"` in params) — nothing to migrate",
                        input_path.display()
                    )
                })?;

            let target_mode = match mode {
                Some(m) => m,
                None => if current_mode == "local" { "cloud".to_string() } else { "local".to_string() }
            };

            if current_mode == target_mode {
                println!("Already in {} mode. No changes needed.", target_mode);
                return Ok(());
            }

            println!("Migrating from {} to {} mode...", current_mode, target_mode);

            // Rewrite the one line, preserving its indentation and formatting.
            let new_content = re_line
                .replace(&content, |caps: &regex::Captures| {
                    format!("{}deployment_mode{}= \"{}\" // switched by `satz migrate`", &caps[1], &caps[2], target_mode)
                })
                .to_string();
            fsx::write(&input_path, new_content)?;
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
            Ok(())
        }
        Commands::SelfUpdate { no_open_readme, check_only, skip_checksum } => {
            run_self_update(!no_open_readme, check_only, skip_checksum).await
        }
        Commands::GetPresets { force, pristine_dir } => {
            crate::presets::run_get_presets(&runtime_config.presets_dir, &runtime_config, force, pristine_dir).await
        }
        Commands::MergePresets { pristine_dir, estate, report_only, adopt } => {
            let attention = crate::presets::run_merge_presets(
                &runtime_config.presets_dir, pristine_dir, estate, &tool_config, &runtime_config, report_only, &adopt,
            ).await?;
            if attention {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Require { framework, input } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            // This command REPORTS, it does not emit — it needs `main.tf` as a
            // value, never on disk. The stage-B block belongs in `transpile`
            // only; pasted here it once made the command silently regenerate
            // hcl/ and return without a report.
            let (manifest, included_claims, _org_id) =
                compliance_inputs(&input_path, &tool_config, &runtime_config)?;

            let gaps = crate::compliance::run_require(
                &framework,
                &input_path,
                &runtime_config.presets_dir,
                &included_claims,
                &manifest,
            )?;
            if gaps {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::ReportCompliance { framework, input, format, report, prowler, no_live, checkov, fail_on } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            configure_estate_impersonation(&input_path, &runtime_config);
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
                &format,
                report,
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
        Commands::Triage { framework, input, prowler, format, report } => {
            let input_path = if Path::new(&input).is_absolute() { PathBuf::from(&input) } else { PathBuf::from(&runtime_config.yaml_dir).join(&input) };
            let (manifest, included_claims, _org_id) = compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            crate::compliance::run_triage(&framework, &runtime_config.presets_dir, &included_claims, &manifest, &prowler, &format, report)
        }
        Commands::RemediationPlan { framework, input, prowler, checkov, out } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let (manifest, included_claims, _org_id) = compliance_inputs(&input_path, &tool_config, &runtime_config)?;
            let checkov_report = if checkov { Some(crate::scan::run(Path::new(&runtime_config.hcl_dir))?) } else { None };
            let out = out.unwrap_or_else(|| {
                config_dir.join("evidence").join("plan").join(format!("{}-{}", framework, crate::compliance::chrono_free_timestamp()))
            });
            crate::compliance::run_remediation_dossier(
                &framework,
                &runtime_config.presets_dir,
                &included_claims,
                &manifest,
                &input_path,
                &prowler,
                checkov_report.as_ref(),
                &out,
            )
        }
        Commands::DocPacks { out, check } => {
            let presets = PathBuf::from(&runtime_config.presets_dir);
            let out = out.unwrap_or_else(|| presets.join("docs"));
            crate::doc_packs::run(&presets, &out, check)
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
        Commands::Plan { args } => run_tf(&runtime_config, "plan", &args),
        Commands::Apply { args } => run_tf(&runtime_config, "apply", &args),
        Commands::TfInit { args } => run_tf(&runtime_config, "init", &args),
        Commands::CheckPresets { input, pristine_dir } => {
            let input_path = estate_path(PathBuf::from(&input), &runtime_config);
            let drift = crate::presets::run_check_presets(
                &input_path,
                &runtime_config.presets_dir,
                &runtime_config.include_dirs,
                pristine_dir,
            )
            .await?;
            if drift {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::OpenReadme => open_url(DOCS_URL),
        Commands::Whoami { offline } => crate::gcp::identity::whoami(offline).await,
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

fn pipeline_b_generate(
    input_path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
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
    let fe = satz_core::pipeline::compile_estate(&input_path.to_string_lossy(), &src, &resolver, &loader)?;
    let mut folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
    // Subtractive override channel: estate suppressions apply before conflict
    // reporting (suppressing a conflicted address resolves the conflict).
    satz_core::pipeline::apply_suppressions(&mut folded, &fe.suppressions)?;
    let conflicts = folded.conflicts();
    if !conflicts.is_empty() {
        let mut msg = String::from("composition conflicts:");
        for c in conflicts {
            msg.push_str(&format!("\n  {}.{}: {} disagreeing definitions", c.addr.tf_type, c.addr.label, c.candidates.len()));
            for (_, spans) in &c.candidates {
                for s in spans {
                    msg.push_str(&format!("\n    - {}:{}", s.file, s.line));
                }
            }
        }
        return Err(msg.into());
    }
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
    let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
    ctx.registry = Some(&registry);
    let out = crate::emitter::emit(&folded, &ctx).map_err(|e| format!("emit: {}", e))?;
    let (provider_sources, provider_versions) = provider_maps(tool_config);
    let providers_tf = crate::emitter::emit_providers(&fe.config, &folded, &fe.env, &provider_sources, &provider_versions)
        .map_err(|e| format!("emit_providers: {}", e))?;
    Ok(PipelineBOut {
        main_tf: append_hcl_passthrough(out.main_tf, &fe.hcl),
        manifest: out.manifest,
        providers_tf,
        variables_tf: crate::emitter::emit_variables(&fe.tfvars),
        tfvars: crate::emitter::emit_tfvars(&fe.tfvars),
        imports_tf: out.imports_tf,
        claims: fe.claims,
        org_policies,
        customer_id: ctx.customer_id.clone(),
        // EmitCtx defaults it to the empty string when the estate declares no
        // customer_organization_id; the compliance plane wants None there so it
        // reports "no customer-organization-id" instead of querying org "".
        org_id: Some(ctx.org_id.clone()).filter(|s| !s.is_empty()),
    })
}

/// `plan -x --config <dir>` puts `--config` inside the pass-through args, where
/// clap never sees it. Detect that and print the command that would have worked.
fn misplaced_config_hint(cmd: &Commands) -> Option<String> {
    let (sub, args) = match cmd {
        Commands::Plan { args } => ("plan", args),
        Commands::Apply { args } => ("apply", args),
        Commands::TfInit { args } => ("tf-init", args),
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
/// A thin wrapper on purpose: the point is not to reimplement `plan`/`apply` but
/// to make them location-independent like every other command, so
/// `satz apply --config <estate>` works from anywhere. stdio is inherited, so
/// apply's approval prompt and the usual coloured output behave normally, and the
/// tool's own exit code is propagated — a failed plan must fail the caller.
///
/// It deliberately does NOT transpile first: `hcl/` is generated, but coupling
/// generation to the deploy step would change what `plan` means and hide a diff
/// the operator should see. Transpile, look, then plan.
fn run_tf(
    runtime_config: &ToolConfig,
    subcommand: &str,
    args: &[String],
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
            "'{}' is not initialised — run `satz tf-init` (same --config) first",
            hcl_dir.display()
        )
        .into());
    }
    eprintln!("{} {} (in {})", runtime_config.tf_tool, subcommand, hcl_dir.display());
    let status = std::process::Command::new(&runtime_config.tf_tool)
        .current_dir(hcl_dir)
        .arg(subcommand)
        .args(args)
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

/// satz reads Satz estates. A `.yaml` estate is not an error the user can fix
/// by editing — it is a file in a dialect the tool no longer speaks — so say what
/// to run instead of failing somewhere deep in a YAML scanner.
fn reject_yaml_estate(input: &Path, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    if input.extension().and_then(|e| e.to_str()) == Some("satz") {
        return Ok(());
    }
    // Printed rather than returned: `main` renders a returned error with `Debug`,
    // which escapes the newlines into literal \n.
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
    let satz = satz_core::migrate::convert(&src, &kind, &name)
        .map_err(|e| format!("{} ({})", e, src_path.display()))?;
    // The dialect's implicit `google_` prefix is not Satz, so a verbatim
    // copy of the YAML keys would not compile. Schemas decide.
    let type_registry = ResourceRegistry::load_all(&runtime_config.schema_dir).ok();
    let satz = satz_core::migrate::normalize_type_keys(&satz, &|t: &str| {
        type_registry.as_ref().is_some_and(|r| r.resources.contains_key(t))
    });
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
        // Printed rather than returned: `main` renders a returned error with
        // `Debug`, which escapes the newlines.
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

    fsx::write(&satz_path, satz.as_bytes())?;
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
fn import_state(
    state_json: Option<PathBuf>,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
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
        let out = std::process::Command::new(&tool_config.tf_tool).arg("show").arg("-json").output()?;
        if !out.status.success() {
            return Err(format!("Failed to run {} show -json: {}", tool_config.tf_tool, String::from_utf8_lossy(&out.stderr)).into());
        }
        serde_json::from_slice(&out.stdout)?
    };
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir).ok();
    let type_names: std::collections::HashSet<String> =
        registry.as_ref().map(|r| r.resources.keys().cloned().collect()).unwrap_or_default();
    let discoverer = crate::discovery::Discoverer::new(state_val, registry, enabled_types, filtered);
    let found = discoverer.discover()?;
    write_imported(&found.config, output, None, &|t| type_names.contains(t), runtime_config)?;
    crate::discovery::report_skipped(&found, &discoverer.filtered_types, verbose);
    if verbose {
        crate::discovery::Discoverer::print_summary(&found.config);
    }
    Ok(())
}

/// The live shape of `satz import`: one Cloud Asset Inventory sweep under
/// `parent` (`organizations/<n>`, `folders/<n>` or `projects/<id>`).
async fn import_org(
    parent: &str,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    verbose: bool,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("import: root {}", parent);
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let type_names: std::collections::HashSet<String> = registry.resources.keys().cloned().collect();
    let org_hint = cfg.root.as_ref().and_then(|r| r.organization.clone())
        .or_else(|| parent.strip_prefix("organizations/").map(String::from));
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(registry)).await?;
    // A folder/project root names no organization; the assets' ancestors do.
    let org_hint = org_hint.or(found.organization.clone());
    attach_billing_accounts(&mut found.config).await;
    write_imported(&found.config, output, org_hint.as_deref(), &|t| type_names.contains(t), runtime_config)?;
    crate::discovery::report_skipped(&found, &filtered, verbose);
    Ok(())
}

fn write_imported(
    config: &Config,
    output: PathBuf,
    org_hint: Option<&str>,
    is_type: &dyn Fn(&str) -> bool,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    let text = discovered_to_satz(config, "discovered", org_hint, is_type)?;
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write(&final_output, text)?;
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
/// resources (every `organizations/<n>` reference or `org_id` names it), the
/// data printed by the same printer the yaml import uses, and shorthand
/// type keys (`folder`, `project`) normalised to provider names.
///
/// Discovery emits plain data — no anchors, tags, includes or nulls — so no
/// dialect pre-pass is needed; that is what makes the direct route possible.
fn discovered_to_satz(
    config: &Config,
    name: &str,
    org_hint: Option<&str>,
    is_type: &dyn Fn(&str) -> bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut top = match serde_yaml::to_value(config)? {
        serde_yaml::Value::Mapping(m) => m,
        other => return Err(format!("discovered config is not a mapping: {:?}", other).into()),
    };
    if !top.contains_key(serde_yaml::Value::String("terraform".into())) {
        let backend: serde_yaml::Value =
            serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n")?;
        top.insert(serde_yaml::Value::String("terraform".into()), backend);
    }
    // Root-scoped resources (org IAM, org policies, groups) use the root
    // provider alias, which an estate declares in `providers { … }`; without
    // it `tofu plan` says "Provider configuration not present" (live-run F10).
    if !top.contains_key(serde_yaml::Value::String("providers".into())) {
        // Org-scoped APIs (Org Policy, Cloud Identity) need a quota project:
        // the first project the import found stands in, the way the Day-0
        // template uses the infra project. Review it.
        let billing = first_project_id(config);
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
    let mut params = Vec::new();
    match infer_org_id(&serde_yaml::Value::Mapping(top.clone())).or_else(|| org_hint.map(String::from)) {
        Some(org) => params.push(("customer_organization_id".to_string(), format!("\"{}\"", org))),
        None => eprintln!(
            "warning: no organization id found among the discovered resources — add `customer_organization_id` to `params` by hand"
        ),
    }
    let header = vec![
        "Discovered estate — review before use: hierarchy is as found, names are the".to_string(),
        "Terraform labels, every resource carries its \"import-id\". `satz transpile`, then".to_string(),
        "`tofu plan` should show imports and no creates.".to_string(),
    ];
    let satz = satz_core::migrate::convert_value(&top, "estate", name, &params, &header)?;
    Ok(satz_core::migrate::normalize_type_keys(&satz, is_type))
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
        Err(_) => Default::default(),
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
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir).ok();
    let imported = satz_hcl::import(&inputs, name, wrap_all, &RegistrySchema(registry.as_ref()))?;
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write(&final_output, &imported.satz)?;
    println!("Wrote {} — review it, then `satz transpile` and `tofu plan` against the source's state: no changes.", final_output.display());
    println!("{}", satz_hcl::summary(&imported.rows));
    for r in &imported.rows {
        match &r.action {
            satz_hcl::Action::Dropped(why) => println!("  dropped    {}:{} {} — {}", r.file, r.line, r.what, why),
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
}

/// The live shape with `--into`: the delta against what the estate declares.
async fn import_delta(
    parent: &str,
    estate: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
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
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(registry)).await?;
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
        let satz = satz_core::migrate::convert_value(&d.top, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header("at the top level"))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write(yaml_dir.join(&name), satz)?;
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
        let satz = satz_core::migrate::convert_value(children, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header(&format!("under {}", address)))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write(yaml_dir.join(&name), satz)?;
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
    fsx::write(&estate, estate_text)?;

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
fn first_project_id(config: &Config) -> Option<String> {
    fn walk_folder(f: &crate::config::Folder, out: &mut Vec<String>) {
        if let Some(ps) = &f.project {
            out.extend(ps.values().map(|p| p.project_id.clone()));
        }
        if let Some(fs) = &f.folder {
            for sub in fs.values() {
                walk_folder(sub, out);
            }
        }
    }
    let mut ids: Vec<String> = config.project.iter().flat_map(|ps| ps.values().map(|p| p.project_id.clone())).collect();
    if let Some(fs) = &config.folder {
        for f in fs.values() {
            walk_folder(f, &mut ids);
        }
    }
    ids.sort();
    ids.into_iter().next()
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
    configure_estate_impersonation(&input_path, runtime_config);
    // Same compile the emitter uses, so the adopted addresses are exactly the
    // ones `apply` will act on.
    let out = pipeline_b_generate(&input_path, tool_config, runtime_config)?;
    let rules = load_import_config(None, tool_config, &runtime_config.presets_dir)?.ok_or(
        "adoption rules live in <presets_dir>/import-config.yaml — run `satz get-presets` so it exists",
    )?;
    let opts = adopt::Options { only: only.into_iter().collect(), activate };
    let mut live = adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = adopt::resolve(&out.manifest, &rules, &opts, &mut live).await;

    println!("\nadopt {} — {} resources declared\n", input_path.display(), out.manifest.resources.len());
    print!("{}", adopt::render_table(&resolutions));
    println!("\n{}", adopt::summary(&resolutions));

    // A table with a FAILED / unresolvable / ambiguous / no-rule row did not
    // answer its question: that is an error exit, not a summary count. The
    // table is above; nothing has been changed at this point.
    let unanswered = adopt::unanswered(&resolutions);
    if unanswered > 0 {
        return Err(format!(
            "adopt: {} resolution(s) failed, unresolvable, ambiguous or without a rule — see the rows above; nothing was changed",
            unanswered
        )
        .into());
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
        // each a needless state write. Read the state once and skip those.
        // A FIRST adopt is fine: an initialized empty state lists nothing and
        // errors nothing. An UNREADABLE state (uninitialized dir, changed
        // backend) means every import below would fail the same way — so this
        // fails fast with the fix instead of printing it 117 times.
        let in_state = crate::bootstrap::state_addresses(&runtime_config.tf_tool, hcl_dir)
            .map_err(|e| {
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
        let mut already_managed = 0usize;
        for r in &resolutions {
            if in_state.contains(&r.address) {
                println!("  {:60} already managed in the state — skipped", r.address);
                already_managed += 1;
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
            "\nadopt: {} activated, {} imported, {} already managed (skipped), {} failed. Now run `satz plan` — it should show no create for what was imported.",
            activated, imported, already_managed, failed
        );
        if failed > 0 {
            return Err(format!("adopt: {} activation(s)/import(s) failed — see above", failed).into());
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
fn append_hcl_passthrough(mut main_tf: String, blocks: &[satz_core::pipeline::HclPassthrough]) -> String {
    for b in blocks {
        let body = dedent_hcl(&b.body);
        let lines = body.lines().count();
        match &b.trust {
            Some(reason) => eprintln!(
                "note: raw HCL passthrough at {}:{} ({} lines) — trusted: {}",
                b.file, b.line, lines, reason
            ),
            None => eprintln!(
                "warning: raw HCL passthrough at {}:{} ({} lines) emitted verbatim — opaque to the compliance plane; no claim can cover it. Add `hcl trust \"<reason>\" {{ … }}` once reviewed.",
                b.file, b.line, lines
            ),
        }
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
fn estate_path(estate: PathBuf, runtime_config: &ToolConfig) -> PathBuf {
    if estate.is_absolute() {
        return estate;
    }
    if estate.exists() {
        let in_yaml_dir = PathBuf::from(&runtime_config.yaml_dir).join(&estate);
        if in_yaml_dir.exists() && in_yaml_dir.canonicalize().ok() != estate.canonicalize().ok() {
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
fn configure_estate_impersonation(input_path: &Path, runtime_config: &ToolConfig) {
    let sa = satz_estate_params(input_path, &runtime_config.include_dirs).ok().and_then(|params| {
        let get = |k: &str| params.get(k).and_then(|v| v.as_str()).map(str::to_string);
        if get("deployment-mode").as_deref() != Some("cloud") {
            return None;
        }
        match (get("svc-iac-account"), get("infra-project-name")) {
            (Some(a), Some(p)) if !a.is_empty() && !p.is_empty() => {
                Some(format!("{}@{}.iam.gserviceaccount.com", a, p))
            }
            _ => None,
        }
    });
    crate::gcp::configure_impersonation(sa);
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
    let src = fsx::read_to_string(input)?;
    let loader = satz_loader(input, include_dirs);
    let env = satz_core::pipeline::estate_params(&input.to_string_lossy(), &src, &loader)?;
    Ok(env.into_iter().map(|(k, v)| (k.replace('_', "-"), v)).collect())
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


pub(crate) fn transpile_sorted_b(
    input: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    let out = pipeline_b_generate(input, tool_config, runtime_config)?;
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

    let total_types = config.resource_types.len();
    let enabled_types = config.resource_types.values().filter(|v| v.import).count();
    println!("Loaded {} resource types from import config '{}' ({} enabled for import).", total_types, config_path.display(), enabled_types);

    Ok(Some(config))
}

fn print_recursive_help(cmd: &mut clap::Command) {
    let _ = cmd.print_help();
    println!("\n");

    let mut subcmds: Vec<clap::Command> = cmd.get_subcommands().cloned().collect();
    // Sort to ensure consistent output order
    subcmds.sort_by(|a, b| a.get_name().cmp(b.get_name()));

    for mut subcmd in subcmds {
        // Skip hidden commands and help subcommand to keep output clean
        if subcmd.is_hide_set() || subcmd.get_name() == "help" {
            continue;
        }
        
        // the rule matches what clap wraps to: the terminal, capped like max_term_width
        let width = terminal_size::terminal_size().map(|(w, _)| w.0 as usize).unwrap_or(100).min(110);
        println!("\n{}", "=".repeat(width));
        println!("COMMAND: {}", subcmd.get_name());
        println!("{}\n", "=".repeat(width));
        
        print_recursive_help(&mut subcmd);
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
        let _ = save_global_settings(settings);
    }
    if let Some((version, url)) = update {
        println!("⚠️  Update available: {} (current: {}). Run `satz self-update` to install. {}", version, env!("CARGO_PKG_VERSION"), url);
    }
    Ok(())
}

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

    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }

    #[derive(Deserialize)]
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

            let status = std::process::Command::new("sh")
                .arg(&temp_file)
                .status()?;
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

        #[cfg(windows)]
        {
            return Err("Automatic installation on Windows is not yet supported. Please download and run the installer manually.".into());
        }
    } else {
        println!("✅ You are running the latest version!");
    }

    Ok(())
}



/// `--html-help`: the documentation site, at the section of the invoked
/// command when the README has one (`id="cmd-<name>"`, stamped by
/// `scripts/build-site.py`), else the front page — said, not assumed.
fn open_html_help(subcommand: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    const DOCUMENTED: &[&str] = &[
        "init", "bootstrap", "transpile", "migrate", "import", "update-schema", "get-presets", "require",
        "report-compliance", "merge-presets", "check-presets", "self-update", "open-readme", "completion",
        "scan-plan", "generate-migration",
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
    /// `transpile` does, and return `sorted(main.tf) ---tfvars--- sorted(tfvars)`.
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
            "{}\n---tfvars---\n{}",
            sorted_lines(&out.main_tf).join("\n"),
            sorted_lines(&crate::emitter::emit_tfvars(&fe.tfvars)).join("\n")
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

    fn emit_case(case: &Path, reg: &crate::ResourceRegistry) -> (crate::emitter::EmitOut, satz_core::pipeline::FrontEnd) {
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
            for (addr, legacy) in &legacy_attrs {
                let got = &got_attrs[addr];
                for (k, v) in legacy {
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
        let found = Discoverer::new(state, None, Some(enabled), filtered).discover().unwrap();
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
        let found = Discoverer::new(state, Some(reg), Some(enabled), Default::default()).discover().unwrap();
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
        crate::template::generate_template(&crate::template::tests::args("first.admin", "example.com"), &path).unwrap();
        let src = std::fs::read_to_string(&path).unwrap();

        let reg = super::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
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
        assert!(out.imports_tf.contains("google_storage_bucket.state"), "{}", out.imports_tf);
        // the membership emits the bare email (prefix stripped), the org grants keep it
        assert!(out.main_tf.contains("id = \"first.admin@example.com\""), "{}", out.main_tf);
        assert!(out.main_tf.contains("member = \"group:svc-iac-users@example.com\""), "{}", out.main_tf);
        assert!(out.main_tf.contains("svc-iac-users@example.com"), "{}", out.main_tf);
        assert_eq!(out.manifest.of_type("google_organization_iam_member").count(), 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_rewrites_the_satz_param_and_refuses_an_estate_without_one() {
        let re_mode = regex::Regex::new(r#"(?m)^\s*deployment_mode\s*=\s*"(\w+)""#).unwrap();
        let re_line = regex::Regex::new(r#"(?m)^(\s*)deployment_mode(\s*)=\s*"\w+"[^\n]*$"#).unwrap();
        let src = "params {\n  deployment_mode          = \"local\" // switched by `satz migrate`\n  x = 1\n}\n";
        assert_eq!(&re_mode.captures(src).unwrap()[1], "local");
        let out = re_line.replace(src, |c: &regex::Captures| format!("{}deployment_mode{}= \"cloud\" // switched by `satz migrate`", &c[1], &c[2])).to_string();
        assert_eq!(out, "params {\n  deployment_mode          = \"cloud\" // switched by `satz migrate`\n  x = 1\n}\n");
        assert!(re_mode.captures("params { x = 1 }").is_none());
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
google_storage_bucket:
  state:
    name: acme-state
    location: EU
    import-id: acme-state
google_organization_iam_audit_config:
  all:
    org_id: "123456789012"
    service: allServices
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = super::corpus::registry();
        let text = discovered_to_satz(&config, "discovered", None, &|t| reg.resources.contains_key(t)).unwrap();
        assert!(text.contains("customer_organization_id = \"123456789012\""), "{}", text);
        assert!(text.contains("terraform {"), "{}", text);
        assert!(text.contains("google_folder {"), "shorthand keys must be normalised:\n{}", text);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("discovered estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        for a in ["google_folder.workloads", "google_project.infra", "google_storage_bucket.state", "google_organization_iam_audit_config.all"] {
            assert!(addrs.contains(a), "missing {} in {:?}", a, addrs);
        }
        for id in ["folders/111", "acme-infra-001", "acme-state"] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
        assert!(!out.main_tf.contains("import-id"));
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
