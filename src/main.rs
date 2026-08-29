mod config;
mod fsx;
mod schema;
mod transpiler;
mod emit_shared;
mod emitter;
mod manifest;
mod state_migration;
mod discovery;
mod template;
mod adopt;
mod bootstrap;
mod gcp;
mod org_policy;
mod cloud_identity;
mod compliance;
mod presets;
mod github;
mod policy_tree;

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::Shell as CompletionShell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use crate::schema::ResourceRegistry;
use crate::transpiler::Transpiler;
use crate::config::{Config, DiscoveryConfig};

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
    pub discovery_config: Option<String>,
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

mod include_processor;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Project config.toml, or the estate directory containing it. Every path in
    /// the config resolves against the config's own directory, so any command can
    /// be run from anywhere.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Validation level: warn (default), error, or none
    #[arg(long, global = true)]
    validation: Option<String>,

    /// Enable verbose output
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile an estate to HCL (.satz; the legacy .yaml dialect is still accepted)
    Transpile {
        /// Estate file — .satz, or a legacy .yaml (inside yaml_dir if relative).
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
    },
    /// Bootstrap initial Google Cloud infrastructure (Project, Bucket, Service Account)
    Bootstrap {
        /// Estate file, e.g. C0example.satz (inside yaml_dir if relative).
        /// Not the tool config — that is --config
        estate: PathBuf,
        /// Dry run mode (don't create resources)
        #[arg(long)]
        dry_run: bool,
    },
    /// Export the current live Organization Policies to a re-importable YAML preset
    #[command(visible_alias = "export-org-policies")]
    ExportOrganizationalPolicies {
        /// Estate file providing the parameter table, incl. customer-organization-id
        /// (inside yaml_dir if relative). Not the tool config — that is --config
        estate: PathBuf,
        /// Organization id override (numeric or organizations/<id>); else read from config
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Output YAML path (default: <yaml_dir>/<Cxxxx>-orgpolicies.yaml)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Diff a desired Org Policy preset against the live organization state
    #[command(visible_alias = "diff-org-policies")]
    DiffOrganizationalPolicies {
        /// Estate file providing the parameter table
        /// (inside yaml_dir if relative). Not the tool config — that is --config
        estate: PathBuf,
        /// Desired Org Policy preset (e.g. presets/CIS-GCP-Foundation-4.0.satz).
        /// Omit to diff every org_policy_policy the config declares against live.
        #[arg(long)]
        preset: Option<PathBuf>,
        /// Organization id override; else read from config
        #[arg(long)]
        customer_organization_id: Option<String>,
        /// Write the report to this path (else stdout)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Report format: console (default), markdown, json
        #[arg(long, default_value = "console")]
        format: String,
        /// Audit the whole resource hierarchy (org, folders, projects) via Cloud Asset
        /// Inventory, classifying node-level overrides against the baseline.
        /// Needs roles/cloudasset.viewer on the organization
        #[arg(short = 'r', long)]
        recursive: bool,
    },
    /// Produce a human-readable report of Organization Policies with explanatory text
    #[command(visible_alias = "report-org-policies")]
    ReportOrganizationalPolicies {
        /// Estate file providing the parameter table, incl. customer-organization-id
        /// (inside yaml_dir if relative). Not the tool config — that is --config
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
        /// Inventory declared policies across the whole resource hierarchy (org,
        /// folders, projects) via Cloud Asset Inventory. --scope's "available but not
        /// set" section stays org-level. Needs roles/cloudasset.viewer
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
    /// Discover infrastructure and generate YAML config from Terraform state
    DiscoverFromState {
        /// Path to Terraform state JSON file
        #[arg(long)]
        state_json: Option<PathBuf>,
        /// Path to output YAML file
        #[arg(long, default_value = "discovered.yaml")]
        output: PathBuf,
        /// Add import ID to every resource
        #[arg(long)]
        add_import_id: bool,
        /// Add import ID as a comment to every resource
        #[arg(long)]
        add_import_id_as_comment: bool,
        /// Path to discovery configuration YAML file
        #[arg(long)]
        discovery_config: Option<PathBuf>,
    },
    /// Discover infrastructure and generate YAML config from GCP Organization
    DiscoverFromOrganization {
        /// Numeric Organization ID
        #[arg(long)]
        customer_organization_id: String,
        /// Path to output YAML file
        #[arg(long, default_value = "discovered.yaml")]
        output: PathBuf,
        /// Add import ID to every resource
        #[arg(long)]
        add_import_id: bool,
        /// Add import ID as a comment to every resource
        #[arg(long)]
        add_import_id_as_comment: bool,
        /// Path to discovery configuration YAML file
        #[arg(long)]
        discovery_config: Option<PathBuf>,
    },
    /// Migrate state and configuration between local and cloud modes
    Migrate {
        /// Estate file — YAML dialect only: this command rewrites the deployment-mode anchor
        /// (inside yaml_dir if relative)
        input: String,
        /// Target mode (local or cloud)
        #[arg(long)]
        mode: Option<String>,
    },
    /// Check for and install new releases from GitHub
    SelfUpdate {
        /// Do not download README.md after installing
        #[arg(long)]
        no_download_readme: bool,
        /// Do not open README.md after downloading (only applies if download runs)
        #[arg(long)]
        no_open_readme: bool,
        /// Only check if an update is available; do not install or download README
        #[arg(long)]
        check_only: bool,
        /// Skip SHA-256 checksum verification (use only if the release predates sidecar support)
        #[arg(long)]
        skip_checksum: bool,
    },
    /// Fetch the upstream preset library into presets_dir: installs what is
    /// missing and refreshes what the estate does not use. Packs the estate
    /// DOES use are refused (they deploy — use `merge-presets`), unless --force.
    GetPresets {
        /// Overwrite presets the estate uses as well. Lists each one first.
        #[arg(long)]
        force: bool,
        /// Take the library from this directory instead of downloading it
        #[arg(long)]
        pristine_dir: Option<PathBuf>,
    },
    /// Convert a YAML-dialect file (estate or pack) to Satz, then GATE it:
    /// transpile the gate estate before and after and require sorted-identical
    /// output. Only PROVEN conversions are kept.
    MigrateToSatz {
        /// File to convert (estate yaml, or a pack under presets_dir)
        input: PathBuf,
        /// Estate used for the differential gate (defaults to `input` when the
        /// input itself is an estate)
        #[arg(long)]
        gate: Option<PathBuf>,
        /// Declared kind in the emitted file
        #[arg(long, default_value = "pack")]
        kind: String,
        /// Convert a hand-drifted copy of an upstream pack into a fork:
        /// output goes to `<stem>.local.satz` and the original .yaml stays
        /// untouched (forks have no twin duty; upstream deltas flow to the
        /// `.diff.satz` ledger via merge-presets)
        #[arg(long)]
        fork: bool,
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
        /// Adopt upstream IN PLACE for these packs instead of forking them — the
        /// deliberate upgrade. Pass a pack stem (`CIS-GCP-Foundation-4.0`),
        /// repeatable; or `all` for every pack that is merely BEHIND. `all`
        /// never touches a pack that differs at the SAME version — that is an
        /// edit, and it must be named explicitly.
        #[arg(long)]
        adopt: Vec<String>,
    },
    /// (dev) Stage B differential: run pipeline B (satz -> fragments -> fold -> emit)
    /// over an estate and compare against the on-disk hcl/main.tf + terraform.tfvars.
    DiffPipelines {
        /// The estate .satz (relative paths resolve against yaml_dir)
        input: String,
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
        /// Skip live verification (declared-estate report only)
        #[arg(long)]
        no_live: bool,
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
    /// Adopt what already exists: resolve the live ids of the resources this
    /// estate declares (folders by name, groups by email, org policies by
    /// constraint, everything else by its rule in discovery-config.yaml) and
    /// bring them under management. A dry run unless --execute
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
    /// Alias of `adopt --only google_org_policy_policy --activate --execute --import`
    AdoptOrgPolicies {
        /// Estate file (inside yaml_dir if relative)
        input: String,
        /// Show what would be activated and imported, change nothing
        #[arg(long)]
        dry_run: bool,
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
    /// Download and open the latest README from the repository
    OpenReadme,
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
    /// Set (or clear) the preferred editor in global settings
    SetPreferredEditor {
        /// Editor command to use (e.g. "code", "zed", "vim"). Omit to show current value.
        editor: Option<String>,
        /// Remove the preferred_editor setting (fall back to $EDITOR / OS default)
        #[arg(long)]
        clear: bool,
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
    /// Preferred editor command for opening files (e.g. "code", "vim", "nano").
    /// Falls back to $EDITOR env var, then the OS default app.
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_editor: Option<String>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            self_update_frequency: default_self_update_frequency(),
            last_update_check: None,
            preferred_editor: None,
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
fn load_global_settings() -> GlobalSettings {
    let path = match global_settings_path() {
        Some(p) => p,
        None => return GlobalSettings::default(),
    };
    if path.exists() {
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("⚠️  Warning: Could not read {}: {}", path.display(), e);
                return GlobalSettings::default();
            }
        };
        return toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("⚠️  Warning: Could not parse {}: {}", path.display(), e);
            eprintln!("   String values must be quoted, e.g.  preferred_editor = \"zed\"");
            GlobalSettings::default()
        });
    }
    // First run: create directory and write defaults
    let defaults = GlobalSettings::default();
    let _ = save_global_settings(&defaults);
    defaults
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
    let cli = Cli::parse();

    // Load/create global settings on first run (creates ~/.config/satz/satz.toml with defaults)
    let mut global_settings = load_global_settings();

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
                Commands::Transpile { .. } | Commands::ScanPlan { .. } | Commands::GenerateMigration { .. } | Commands::UpdateSchema { .. } | Commands::DiscoverFromState { .. } | Commands::DiscoverFromOrganization { .. } | Commands::Migrate { .. } | Commands::Bootstrap { .. } | Commands::ExportOrganizationalPolicies { .. } | Commands::DiffOrganizationalPolicies { .. } | Commands::ReportOrganizationalPolicies { .. } | Commands::GetPresets { .. } | Commands::CheckPresets { .. } | Commands::Require { .. } | Commands::ReportCompliance { .. } | Commands::Adopt { .. } | Commands::AdoptOrgPolicies { .. } | Commands::DiffPipelines { .. } | Commands::MigrateToSatz { .. } | Commands::MergePresets { .. }
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
                Commands::Init { .. } | Commands::SelfUpdate { .. } | Commands::Completion { .. } | Commands::OpenReadme | Commands::SetPreferredEditor { .. } => {
                    // These commands can proceed without a config file
                    PathBuf::from("config.toml")
                }
            }
        }
    };

    // Optional: check for updates per global settings (skip for SelfUpdate and Init)
    if !matches!(cmd_choice, Commands::SelfUpdate { .. } | Commands::Init { .. } | Commands::SetPreferredEditor { .. }) {
        let _ = maybe_check_for_updates(&mut global_settings).await;
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
            discovery_config: None,
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
        Commands::Transpile { input, output, schema_dir, print_variables } => {
            let _validation_level = cli.validation.unwrap_or(tool_config.validation_level.clone());

            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };
            if let Some(sd) = &schema_dir {
                runtime_config.schema_dir = if Path::new(sd).is_absolute() {
                    sd.to_string_lossy().to_string()
                } else {
                    config_dir.join(sd).to_string_lossy().to_string()
                };
            }
            // BOTH dialects emit HCL. Satz compiles through the fragment
            // pipeline (per-file fragments, the ⊕ fold, emission from Folded);
            // a `.yaml` estate goes through the legacy walk.
            //
            // Keeping the YAML door open is deliberate (owner, 2026-08-23):
            // converting is a decision each repo owner makes when ready, not a
            // toll on using the tool at all. What they give up until they do is
            // real and worth naming rather than discovering — `suppress`,
            // `hcl { … }`, claims and the whole compliance plane are fragment-
            // pipeline features, so the warning below says so once per run.
            let is_satz = input_path.extension().and_then(|e| e.to_str()) == Some("satz");
            let (main_tf, providers_tf, variables_tf, tfvars, imports_tf) = if is_satz {
                let out = pipeline_b_generate(&input_path, &tool_config, &runtime_config)?;
                (out.main_tf, out.providers_tf, out.variables_tf, out.tfvars, out.imports_tf)
            } else {
                eprintln!(
                    "note: {} is a YAML-dialect estate — transpiled through the legacy walk.\n\
                     `suppress`, `hcl {{ … }}`, claims and `require`/`report-compliance` need\n\
                     Satz. Convert when ready; the conversion proves itself:\n\
                     \n    satz migrate-to-satz {} --kind estate\n",
                    input_path.display(),
                    input_path.file_name().unwrap_or_default().to_string_lossy()
                );
                init_resource_merge(&runtime_config.schema_dir);
                let include_paths: Vec<PathBuf> =
                    runtime_config.include_dirs.iter().map(PathBuf::from).collect();
                let (provider_sources, provider_versions) = provider_maps(&tool_config);
                let p = pipeline_a_generate(
                    &input_path,
                    &_validation_level,
                    provider_sources,
                    provider_versions,
                    &include_paths,
                    &runtime_config,
                )?;
                (p.main_tf, p.providers_tf, p.variables_tf, p.tfvars, p.imports_tf)
            };
            if print_variables {
                println!("{}", tfvars);
            }
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
            write_file("main.tf", &main_tf)?;
            write_file("providers.tf", &providers_tf)?;
            write_file("variables.tf", &variables_tf)?;
            write_file("terraform.tfvars", &tfvars)?;
            write_file("imports.tf", &imports_tf)?;
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

            // 4. Generate template YAML if customer_id provided
            if let Some(c_id) = customer_id {
                let yaml_path = PathBuf::from(&runtime_config.yaml_dir).join(format!("{}.yaml", c_id));
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
                    println!("Generated template: {}", yaml_path.display());
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
            crate::state_migration::generate_migration(&m_path, &final_output, &tool_config.tf_tool)?;
            println!("Migration script generated: {}", final_output.display());
            Ok(())
        }
        Commands::DiscoverFromState { state_json, output, add_import_id, add_import_id_as_comment, discovery_config } => {
            let discovery_config_obj = load_discovery_config(discovery_config, &tool_config, &runtime_config.presets_dir)?
                .ok_or_else(|| {
                    let err: Box<dyn std::error::Error> = format!(
                        "Discovery configuration not found. Provide --discovery-config, or run 'satz get-presets' \
                         so that '{}/discovery-config.yaml' exists, or set discovery_config in config.toml.",
                        runtime_config.presets_dir
                    ).into();
                     err
                })?;
            let enabled_types = Some(discovery_config_obj.resource_types.into_iter().filter(|(_,v)| v.import).map(|(k,_)| k).collect());

            println!("Reading infrastructure state...");
            let state_val: serde_json::Value = if let Some(path) = state_json {
                let content = fsx::read_to_string(&path)?;
                serde_json::from_str(&content)?
            } else {
                let output = std::process::Command::new(&tool_config.tf_tool)
                    .arg("show")
                    .arg("-json")
                    .output()?;
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Failed to run {} show -json: {}", tool_config.tf_tool, err).into());
                }
                serde_json::from_slice(&output.stdout)?
            };

            let s_dir = PathBuf::from(&runtime_config.schema_dir);
            let registry = ResourceRegistry::load_all(s_dir.to_str().unwrap_or("schemas")).ok();

            let discoverer = crate::discovery::Discoverer::new(state_val, registry, cli.verbose, add_import_id, add_import_id_as_comment, enabled_types);
            let config = discoverer.discover()?;

            let mut yaml = serde_yaml::to_string(&config)?;

            if add_import_id_as_comment {
                // Post-process to turn import-id-comment fields into actual YAML comments
                let mut lines: Vec<String> = Vec::new();
                for line in yaml.lines() {
                    if line.contains("import-id-comment:") {
                        let parts: Vec<&str> = line.split("import-id-comment:").collect();
                        if parts.len() == 2 {
                            let indent = parts[0];
                            let value = parts[1].trim().trim_matches('"').trim_matches('\'');
                            lines.push(format!("{}# import-id: {}", indent, value));
                            continue;
                        }
                    }
                    lines.push(line.to_string());
                }
                yaml = lines.join("\n") + "\n";
            }

            let final_output = if output.is_absolute() {
                output
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(output)
            };

            if let Some(parent) = final_output.parent() {
                fsx::create_dir_all(parent)?;
            }
            fsx::write(&final_output, yaml)?;
            if cli.verbose {
                crate::discovery::Discoverer::print_summary(&config, Some(discoverer.filtered_count.get()));
            }
            Ok(())
        }
        Commands::DiscoverFromOrganization { customer_organization_id, output, add_import_id, add_import_id_as_comment, discovery_config } => {
            // runtime_config, not tool_config: the sibling DiscoverFromState already
            // uses the resolved directory, so --config was silently ignored here.
            let s_dir = PathBuf::from(&runtime_config.schema_dir);
            let registry = ResourceRegistry::load_all(s_dir.to_str().unwrap_or("schemas"))
                .map_err(|e| format!("Failed to load resource registry from {}: {}", s_dir.display(), e))?;

            let discovery_config_obj = load_discovery_config(discovery_config, &tool_config, &runtime_config.presets_dir)?
                .ok_or_else(|| {
                    let err: Box<dyn std::error::Error> = format!(
                        "Discovery configuration not found. Provide --discovery-config, or run 'satz get-presets' \
                         so that '{}/discovery-config.yaml' exists, or set discovery_config in config.toml.",
                        runtime_config.presets_dir
                    ).into();
                     err
                })?;
            let config = crate::discovery::Discoverer::discover_from_org(&customer_organization_id, cli.verbose, add_import_id, add_import_id_as_comment, Some(discovery_config_obj), Some(registry)).await?;
            let mut yaml = serde_yaml::to_string(&config)?;

            if add_import_id_as_comment {
                // Post-process to turn import-id-comment fields into actual YAML comments
                let mut lines: Vec<String> = Vec::new();
                for line in yaml.lines() {
                    if line.contains("import-id-comment:") {
                        let parts: Vec<&str> = line.split("import-id-comment:").collect();
                        if parts.len() == 2 {
                            let indent = parts[0];
                            let value = parts[1].trim().trim_matches('"').trim_matches('\'');
                            lines.push(format!("{}# import-id: {}", indent, value));
                            continue;
                        }
                    }
                    lines.push(line.to_string());
                }
                yaml = lines.join("\n") + "\n";
            }

            let final_output = if output.is_absolute() {
                output
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(output)
            };

            if let Some(parent) = final_output.parent() {
                fsx::create_dir_all(parent)?;
            }
            fsx::write(&final_output, yaml)?;
            if cli.verbose {
                crate::discovery::Discoverer::print_summary(&config, None);
            }
            Ok(())
        }
        Commands::Bootstrap { estate, dry_run } => {
            init_resource_merge(&runtime_config.schema_dir);
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            crate::bootstrap::bootstrap(
                config_path,
                dry_run,
                runtime_config,
                cli.config.clone(),
                cli.validation.clone(),
                cli.verbose,
            )
            .await?;
            Ok(())
        }
        Commands::ExportOrganizationalPolicies { estate, customer_organization_id, output } => {
            init_resource_merge(&runtime_config.schema_dir);
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            crate::org_policy::export_org_policies(
                config_path,
                customer_organization_id,
                output,
                runtime_config,
            )
            .await?;
            Ok(())
        }
        Commands::DiffOrganizationalPolicies { estate, preset, customer_organization_id, report, format, recursive } => {
            init_resource_merge(&runtime_config.schema_dir);
            // Satz-native: no .gen.yaml twin build. The vars table and the
            // declared policy set both come from the fragment pipeline.
            let config_path = estate_path(estate, &runtime_config);
            // Presets resolve against the presets library (beside config.toml).
            let preset = preset.map(|p| resolve_against(&runtime_config.presets_dir, p));
            crate::org_policy::diff_org_policies(
                config_path,
                preset,
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
            init_resource_merge(&runtime_config.schema_dir);
            // Satz-native: bootstrap needs the variable table, nothing more.
            let config_path = estate_path(estate, &runtime_config);
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
            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };

            if !input_path.exists() {
                return Err(format!("Input file not found: {}", input_path.display()).into());
            }

            let content = fsx::read_to_string(&input_path)?;

            // Detect current mode
            let re_cloud = regex::Regex::new(r"deployment-mode:\s+&deployment-mode\s+cloud").unwrap();
            let current_mode = if re_cloud.is_match(&content) {
                "cloud"
            } else {
                "local"
            };

            let target_mode = match mode {
                Some(m) => m,
                None => if current_mode == "local" { "cloud".to_string() } else { "local".to_string() }
            };

            if current_mode == target_mode {
                println!("Already in {} mode. No changes needed.", target_mode);
                return Ok(());
            }

            println!("Migrating from {} to {} mode...", current_mode, target_mode);

            // Update YAML while preserving formatting and anchors
            let re = regex::Regex::new(r"(?m)^\s*deployment-mode:\s+&deployment-mode\s+\w+.*$").unwrap();
            let new_content = re.replace(&content, format!("  deployment-mode: &deployment-mode {} # switch by command", target_mode)).to_string();
            fsx::write(&input_path, new_content)?;
            println!("Updated YAML: {}", input_path.display());

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
        Commands::SelfUpdate { no_download_readme, no_open_readme, check_only, skip_checksum } => {
            run_self_update(!no_download_readme, !no_open_readme, check_only, skip_checksum, global_settings.preferred_editor.as_deref()).await
        }
        Commands::GetPresets { force, pristine_dir } => {
            crate::presets::run_get_presets(&runtime_config.presets_dir, &runtime_config, force, pristine_dir).await
        }
        Commands::MigrateToSatz { input, gate, kind, fork } => {
            init_resource_merge(&runtime_config.schema_dir);
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
            let satz_path = if fork {
                if kind == "estate" {
                    return Err("--fork applies to packs, not estates".into());
                }
                let stem = src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("converted");
                src_path.with_file_name(format!("{}.local.satz", stem))
            } else {
                src_path.with_extension("satz")
            };

            // Gate estate: explicit, or the input itself for estates.
            let gate_path = match &gate {
                Some(g) => resolve(g),
                None if kind == "estate" => src_path.clone(),
                None => return Err("packs need --gate <estate> for the differential proof".into()),
            };
            let include_paths: Vec<PathBuf> =
                runtime_config.include_dirs.iter().map(PathBuf::from).collect();
            // a .satz gate estate must be compiled to its generated yaml first
            let gate_path = if gate_path.extension().is_some_and(|e| e == "satz") {
                resolve_satz_input(gate_path, &runtime_config.include_dirs)?
            } else {
                gate_path
            };
            let before = transpile_sorted(&gate_path, &include_paths, &runtime_config)?;

            fsx::write(&satz_path, satz.as_bytes())?;
            println!("converted {} -> {}", src_path.display(), satz_path.display());

            // After: for a pack, the gate estate must see the CONVERTED content —
            // rebuild the .yaml twin from the new .satz in place (originals in git).
            let original = fsx::read_to_string(&src_path)?;
            let after = if kind == "estate" {
                let gen = resolve_satz_input(satz_path.clone(), &runtime_config.include_dirs)?;
                transpile_sorted(&gen, &include_paths, &runtime_config)?
            } else {
                let compiled = satz_core::satz::compile(&satz)
                    .map_err(|e| format!("{} in {}", e, satz_path.display()))?;
                let header = twin_header(
                    &satz_path.file_name().unwrap_or_default().to_string_lossy(),
                    &compiled.yaml,
                );
                fsx::write(&src_path, format!("{}{}", header, compiled.yaml).as_bytes())?;
                if let Some(claims) = compiled.claims_yaml {
                    let cp = PathBuf::from(src_path.to_string_lossy().replace(".yaml", ".claims.yaml"));
                    fsx::write(&cp, format!("{}{}", header, claims).as_bytes())?;
                }
                match transpile_sorted(&gate_path, &include_paths, &runtime_config) {
                    Ok(a) => a,
                    Err(e) => {
                        fsx::write(&src_path, original.as_bytes())?; // restore on failure
                        let _ = std::fs::remove_file(&satz_path);
                        return Err(format!("gate transpile failed after conversion (restored): {}", e).into());
                    }
                }
            };

            // The gate above proves "same HCL", but it proves it through the
            // LEGACY walk — which still accepts the YAML shorthand. That made it
            // blind to the output simply not being valid Satz: it reported PROVEN
            // on an estate `transpile` then refused. So the conversion must also
            // compile as Satz, through the pipeline that will actually read it.
            if kind == "estate" {
                if let Err(e) = pipeline_b_generate(&satz_path, &tool_config, &runtime_config) {
                    fsx::write(&src_path, original.as_bytes())?;
                    let _ = std::fs::remove_file(&satz_path);
                    return Err(format!(
                        "conversion produced Satz that does not compile (restored): {}",
                        e
                    )
                    .into());
                }
            }

            if before == after {
                if fork {
                    // fork proven: keep only the .local.satz; the original twin
                    // (and any claims sidecar we wrote) revert to pristine state
                    fsx::write(&src_path, original.as_bytes())?;
                    println!("PROVEN: fork holds — {} written; {} restored (repoint the estate `use` to the fork).",
                        satz_path.display(), src_path.display());
                } else {
                    println!("PROVEN: gate estate output identical — conversion holds.");
                }
                Ok(())
            } else {
                if kind != "estate" {
                    // restore the ORIGINAL twin; keep .satz for inspection
                    fsx::write(&src_path, original.as_bytes())?;
                    let d = tempfile_diff(&before, &after);
                    println!("NEEDS-REVIEW: output differs. First differences:\n{}", d);
                    return Err("conversion not proven — original restored, .satz kept for review".into());
                }
                let d = tempfile_diff(&before, &after);
                println!("NEEDS-REVIEW: output differs. First differences:\n{}", d);
                Err("conversion not proven — .satz kept for review, original untouched".into())
            }
        }
        Commands::MergePresets { pristine_dir, estate, report_only, adopt } => {
            init_resource_merge(&runtime_config.schema_dir);
            let attention = crate::presets::run_merge_presets(
                &runtime_config.presets_dir, pristine_dir, estate, &tool_config, &runtime_config, report_only, &adopt,
            ).await?;
            if attention {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::DiffPipelines { input } => {
            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };
            let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)?;

            let resolver = EstateResolver { registry: &registry };
            let src = fsx::read_to_string(&input_path)?;
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
            let fe = satz_core::pipeline::compile_estate(
                &input_path.to_string_lossy(),
                &src,
                &resolver,
                &loader,
            )?;
            let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
            for c in folded.conflicts() {
                eprintln!("CONFLICT: {}.{} ({} candidates)", c.addr.tf_type, c.addr.label, c.candidates.len());
            }
            let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
            ctx.registry = Some(&registry);
            let b_out = crate::emitter::emit(&folded, &ctx).map_err(|e| format!("emit: {}", e))?;
            // Same as transpile: the raw passthrough is part of what B writes,
            // so leaving it off here reported every `hcl { … }` block as drift.
            let b_main = append_hcl_passthrough(b_out.main_tf, &fe.hcl);
            let b_imports = b_out.imports_tf;
            let b_tfvars = crate::emitter::emit_tfvars(&fe.tfvars);
            let (provider_sources, provider_versions) = provider_maps(&tool_config);
            let b_providers = crate::emitter::emit_providers(&fe.config, &folded, &fe.env, &provider_sources, &provider_versions)
                .map_err(|e| format!("emit_providers: {}", e))?;
            let b_variables = crate::emitter::emit_variables(&fe.tfvars);

            let a_main = fsx::read_to_string(Path::new(&runtime_config.hcl_dir).join("main.tf"))?;
            let a_tfvars = fsx::read_to_string(Path::new(&runtime_config.hcl_dir).join("terraform.tfvars"))?;
            let a_providers = fsx::read_to_string(Path::new(&runtime_config.hcl_dir).join("providers.tf"))?;
            let a_variables = fsx::read_to_string(Path::new(&runtime_config.hcl_dir).join("variables.tf"))?;
            let a_imports = fsx::read_to_string(Path::new(&runtime_config.hcl_dir).join("imports.tf")).unwrap_or_default();
            let lines = |s: &str| -> std::collections::BTreeSet<String> {
                s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect()
            };
            for (name, a, b) in [
                ("main.tf", &a_main, &b_main),
                ("tfvars", &a_tfvars, &b_tfvars),
                ("providers.tf", &a_providers, &b_providers),
                ("variables.tf", &a_variables, &b_variables),
                ("imports.tf", &a_imports, &b_imports),
            ] {
                let (a, b) = (lines(a), lines(b));
                let only_a: Vec<_> = a.difference(&b).collect();
                let only_b: Vec<_> = b.difference(&a).collect();
                println!(
                    "diff-pipelines[{}]: {} matched, {} only in A (walk), {} only in B (fold)",
                    name,
                    a.intersection(&b).count(),
                    only_a.len(),
                    only_b.len()
                );
                if std::env::var("DIFF_DETAIL").is_ok() {
                    for l in only_a { println!("A| {}", l); }
                    for l in only_b { println!("B| {}", l); }
                }
            }
            Ok(())
        }
        Commands::Require { framework, input } => {
            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };
            // This command REPORTS, it does not emit — it needs `main.tf` as a
            // value, never on disk. The stage-B block belongs in `transpile`
            // only; pasted here it once made the command silently regenerate
            // hcl/ and return without a report.
            init_resource_merge(&runtime_config.schema_dir);
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
        Commands::ReportCompliance { framework, input, format, report, prowler, no_live } => {
            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };
            // Reports, never emits — see the note in `require`.
            init_resource_merge(&runtime_config.schema_dir);
            let (manifest, included_claims, org_id) =
                compliance_inputs(&input_path, &tool_config, &runtime_config)?;

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
                no_live,
            )
            .await?;
            Ok(())
        }
        Commands::Adopt { input, only, execute, import, activate } => {
            run_adopt(&input, only, execute, import, activate, &tool_config, &runtime_config).await
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
        Commands::Plan { args } => run_tf(&runtime_config, "plan", &args),
        Commands::Apply { args } => run_tf(&runtime_config, "apply", &args),
        Commands::TfInit { args } => run_tf(&runtime_config, "init", &args),
        Commands::CheckPresets { input, pristine_dir } => {
            let input_path = if Path::new(&input).is_absolute() {
                PathBuf::from(&input)
            } else {
                PathBuf::from(&runtime_config.yaml_dir).join(&input)
            };
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
        Commands::OpenReadme => run_open_readme(global_settings.preferred_editor.as_deref()).await,
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
        Commands::SetPreferredEditor { editor, clear } => {
            if clear {
                global_settings.preferred_editor = None;
                save_global_settings(&global_settings)?;
                println!("✅ preferred_editor cleared (will fall back to $EDITOR / OS default).");
            } else if let Some(e) = editor {
                global_settings.preferred_editor = Some(e.clone());
                save_global_settings(&global_settings)?;
                println!("✅ preferred_editor set to \"{}\".", e);
            } else {
                match &global_settings.preferred_editor {
                    Some(e) => println!("preferred_editor = \"{}\"", e),
                    None => println!("preferred_editor is not set (using $EDITOR / OS default)."),
                }
            }
            Ok(())
        }
    }?;

    Ok(())
}

/// Satz front-end, shared by every command that takes an estate input: a .satz file
/// compiles to its generated .gen.yaml sibling (inspectable, never hand-edited) and
/// the returned path feeds the unchanged YAML pipeline.
/// Stage B generation: satz estate -> fragments -> fold -> emit, schema-driven.
/// Returns every generated file. Used by the transpile flip and diff-pipelines.
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
        /// schemas; the three intrinsic-scope types and the grant classes are the
        /// same facts HoistTable/the auto-explode list encode in the walk.
        struct EstateResolver<'a> {
            registry: &'a ResourceRegistry,
        }
        impl satz_core::pipeline::TypeResolver for EstateResolver<'_> {
            fn resolve(&self, key: &str) -> Option<ResolvedType> {
                use satz_core::{MergeClass, Scope};
                match key {
                    "terraform" | "providers" | "variables" | "include" => return None,
                    "cloud_identity_group" | "google_cloud_identity_group" => {
                        return Some(ResolvedType {
                            tf_type: "google_cloud_identity_group".into(),
                            class: MergeClass::Entity,
                            scope: Scope::Customer,
                        })
                    }
                    _ => {}
                }
                let (_, _schema) = self.registry.find_resource(key)?;
                let tf = if key.starts_with("google_") { key.to_string() } else { format!("google_{}", key) };
                let (class, scope) = if tf == "google_organization_iam_member" {
                    (MergeClass::Grant, Scope::Org)
                } else if tf == "google_billing_account_iam_member" {
                    (MergeClass::Grant, Scope::Billing)
                } else if tf.ends_with("iam_member") {
                    (MergeClass::Grant, Scope::Node)
                } else {
                    (MergeClass::Entity, Scope::Node)
                };
                Some(ResolvedType { tf_type: tf, class, scope })
            }
        }
impl satz_core::algebra::TypeTable for EstateResolver<'_> {
    fn merge_class(&self, t: &str) -> satz_core::MergeClass {
        satz_core::pipeline::type_facts(t).0
    }
    fn scope(&self, t: &str) -> satz_core::Scope {
        satz_core::pipeline::type_facts(t).1
    }
}

    fn pipeline_b_generate(
    input_path: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<PipelineBOut, Box<dyn std::error::Error>> {
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)?;

    /// Schema-driven resolver: tf-type facts come from the loaded provider
    /// schemas; the intrinsic scopes and grant classes are the same facts
    /// HoistTable / the auto-explode list encode in the walk.
    struct EstateResolver<'a> {
        registry: &'a ResourceRegistry,
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
        "\n{}: {} is a YAML-dialect estate, and this command needs the fragment\n\
         pipeline (claims and the compliance plane are Satz-only). `transpile`\n\
         still accepts YAML.\n\
         Convert once — the conversion proves itself by transpiling before and\n\
         after and requiring identical output:\n\n    satz migrate-to-satz {} --kind estate\n",
        what,
        input.display(),
        input.file_name().unwrap_or_default().to_string_lossy()
    );
    Err("YAML-dialect estate: convert it with `migrate-to-satz` first".into())
}

/// The two facts `require` and `report-compliance` need: the emitted `main.tf`
/// as a value, and the claims the estate actually pulled in.
///
/// `.satz` estates run the fragment pipeline — same compile as `transpile`, so
/// the witnesses the goal view matches against are exactly the ones that would
/// be written to disk. The emission manifest, not the rendered text, is what
/// the compliance plane reads: a witness inside a raw `hcl { … }` block is
/// therefore not a witness, as documented.
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
    let input_path = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        PathBuf::from(&runtime_config.yaml_dir).join(input)
    };
    reject_yaml_estate(&input_path, "adopt")?;
    init_resource_merge(&runtime_config.schema_dir);
    // Same compile the emitter uses, so the adopted addresses are exactly the
    // ones `apply` will act on.
    let out = pipeline_b_generate(&input_path, tool_config, runtime_config)?;
    let rules = load_discovery_config(None, tool_config, &runtime_config.presets_dir)?.ok_or(
        "adoption rules live in <presets_dir>/discovery-config.yaml — run `satz get-presets` so it exists",
    )?;
    let opts = adopt::Options { only: only.into_iter().collect(), activate };
    let mut live = adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = adopt::resolve(&out.manifest, &rules, &opts, &mut live).await;

    println!("\nadopt {} — {} resources declared\n", input_path.display(), out.manifest.resources.len());
    print!("{}", adopt::render_table(&resolutions));
    println!("\n{}", adopt::summary(&resolutions));

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
        let (mut activated, mut imported, mut failed) = (0usize, 0usize, 0usize);
        for r in &resolutions {
            let id = match &r.outcome {
                Outcome::NeedsActivation { id, enforce } => {
                    let Some((parent, constraint)) = &r.org_policy else { continue };
                    println!("  {:60} activating (managed, not live)...", r.address);
                    let spec = serde_json::json!({ "rules": [{ "enforce": enforce.unwrap_or(true) }] });
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
                _ => continue,
            };
            if crate::bootstrap::run_import(&runtime_config.tf_tool, hcl_dir, &r.address, id) {
                imported += 1;
            } else {
                failed += 1;
            }
        }
        println!("\nadopt: {} activated, {} imported, {} failed. Now run `satz plan` — it should show no create for what was imported.", activated, imported, failed);
    } else {
        let (written, hints) = adopt::write_import_ids(&resolutions)?;
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
fn estate_path(estate: PathBuf, runtime_config: &ToolConfig) -> PathBuf {
    if estate.is_absolute() {
        estate
    } else {
        PathBuf::from(&runtime_config.yaml_dir).join(estate)
    }
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

pub(crate) fn resolve_satz_input(input_path: PathBuf, include_dirs: &[String]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if input_path.extension().and_then(|e| e.to_str()) != Some("satz") {
        return Ok(input_path);
    }
    // Compile the whole .satz tree: the estate plus every `use`d .satz pack,
    // recursively. Each file gets a .gen.yaml sibling (packs also a .claims.yaml
    // when they declare claims); the emitted includes point at the siblings, so
    // resolution and first-definition-wins work exactly as for YAML packs.
    let mut queue = vec![input_path.clone()];
    let mut done: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    while let Some(path) = queue.pop() {
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !done.insert(canon) {
            continue;
        }
        let src = fsx::read_to_string(&path)?;
        let compiled = satz_core::satz::compile(&src)
            .map_err(|e| format!("{} in {}", e, path.display()))?;
        if compiled.has_suppressions {
            return Err(format!(
                "{}: `suppress` requires the fragment pipeline — this command still runs the YAML path, which cannot honor suppressions",
                path.display()
            )
            .into());
        }
        if compiled.has_hcl {
            return Err(format!(
                "{}: `hcl {{ … }}` requires the fragment pipeline — this command still runs the YAML path, which cannot carry raw HCL",
                path.display()
            )
            .into());
        }
        let gen_path = path.with_extension("gen.yaml");
        fsx::write(&gen_path, compiled.yaml.as_bytes())?;
        println!("satz: compiled {} -> {}", path.display(), gen_path.display());
        // No claims sidecar: since R5 the compliance plane reads claims from the
        // `.satz` source through the front end. Writing one meant a read-only
        // command like `report-organizational-policies` left an untracked file in
        // a customer repo — and estate `.gitignore`s cover `*.gen.yaml`, which
        // does not match `*.gen.claims.yaml`.
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        for dep in compiled.satz_deps {
            // Same search order as includes: relative to the using file, then the
            // configured include dirs — so packs resolve identically in both worlds.
            let mut candidates = vec![parent.join(&dep)];
            candidates.extend(include_dirs.iter().map(|d| Path::new(d).join(&dep)));
            match candidates.into_iter().find(|c| c.exists()) {
                Some(dep_path) => queue.push(dep_path),
                None => {
                    return Err(format!(
                        "use \"{}\" in {}: file not found (searched beside the file and include_dirs)",
                        dep,
                        path.display()
                    )
                    .into())
                }
            }
        }
    }
    Ok(input_path.with_extension("gen.yaml"))
}

/// Header for a generated pack twin. Content packs get the customer-facing
/// wording: their local copy is MEANT to be edited; updates come via merge-presets.
pub(crate) fn twin_header(src_name: &str, yaml: &str) -> String {
    if yaml.contains("# satz-mode: content") {
        format!(
            "# GENERATED scaffold from {} \u{2014} customers EDIT THEIR COPY in place;\n# updates arrive via `satz merge-presets` (three-way against your base).\n",
            src_name
        )
    } else {
        format!(
            "# GENERATED by `satz build-packs` from {} \u{2014} do not edit; edit the .satz source.\n",
            src_name
        )
    }
}

/// Transpile an estate through the standard pipeline and return sorted
/// main.tf + tfvars — the comparison form every differential gate uses.
/// Emit a YAML-dialect estate through the legacy walk.
///
/// The dialect is convert-ONLY no longer: an owner decision (2026-08-23) keeps
/// `transpile` open to `.yaml` so nobody is forced through `migrate-to-satz`
/// before they are ready. Satz remains the direction of travel and the only
/// dialect the fragment pipeline — and therefore `suppress`, `hcl { … }` and
/// the compliance plane — can see.
///
/// `validation_level` and the provider maps are parameters because the gate in
/// `transpile_sorted` deliberately passes different ones: it compares main.tf
/// and tfvars only, so provider metadata would be noise in a proof.
fn pipeline_a_generate(
    input: &Path,
    validation_level: &str,
    provider_sources: HashMap<String, String>,
    provider_versions: HashMap<String, String>,
    include_paths: &[PathBuf],
    runtime_config: &ToolConfig,
) -> Result<crate::transpiler::GeneratedProject, Box<dyn std::error::Error>> {
    let (text, _) = include_processor::process_includes_with_ops(input, include_paths)?;
    let raw: serde_yaml::Value = serde_yaml::from_str(&text).inspect_err(|e| {
        print_yaml_error_context(&text, e);
    })?;
    let raw = merge_renamed_resource_keys(raw)?;
    let variables = extract_variables(&resolve_yaml_custom_tags(raw.clone()));
    let resolved = resolve_yaml_custom_tags(merge_variables(raw));
    let config: Config = serde_yaml::from_value(resolved)?;
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir).ok();
    let t = Transpiler::new(
        &config,
        registry,
        runtime_config.auto_explode.clone(),
        validation_level.to_string(),
        variables,
        provider_sources,
        provider_versions,
    );
    t.transpile()
}

pub(crate) fn transpile_sorted(
    input: &Path,
    include_paths: &[PathBuf],
    runtime_config: &ToolConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    let project = pipeline_a_generate(
        input,
        "none",
        HashMap::new(),
        HashMap::new(),
        include_paths,
        runtime_config,
    )?;
    let mut lines: Vec<&str> = project.main_tf.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.sort_unstable();
    let mut tv: Vec<&str> = project.tfvars.lines().collect();
    tv.sort_unstable();
    Ok(format!("{}\n---\n{}", lines.join("\n"), tv.join("\n")))
}

/// The same comparison form as `transpile_sorted`, but produced by the fragment
/// pipeline — no YAML round trip, so `suppress` and `hcl { … }` are honored and
/// the gate sees exactly what `transpile` would write.
///
/// Wider than the legacy form on purpose: a preset repoint can move an
/// `import-id` or a variable default, and those belong in an identity proof.
/// The gate's comparison form for EITHER dialect: Satz through the fragment
/// pipeline, YAML through the legacy walk.
///
/// Both arms are load-bearing. YAML input stays supported (owner decision,
/// 2026-08-23: three estates are still unconverted), so a gate that only speaks
/// Satz is a gate that refuses half the estates it is supposed to protect.
pub(crate) fn transpile_sorted_for(
    input: &Path,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    if input.extension().and_then(|e| e.to_str()) == Some("satz") {
        return transpile_sorted_b(input, tool_config, runtime_config);
    }
    let include_paths: Vec<PathBuf> =
        runtime_config.include_dirs.iter().map(PathBuf::from).collect();
    transpile_sorted(input, &include_paths, runtime_config)
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

fn tempfile_diff(before: &str, after: &str) -> String {
    let b: std::collections::BTreeSet<&str> = before.lines().collect();
    let a: std::collections::BTreeSet<&str> = after.lines().collect();
    let mut out = String::new();
    for l in b.difference(&a).take(5) {
        out.push_str(&format!("  - {}\n", l));
    }
    for l in a.difference(&b).take(5) {
        out.push_str(&format!("  + {}\n", l));
    }
    out
}

/// Load the provider schemas' resource names and install them for cross-file merging
/// (see include_processor::set_resource_types). Missing schema dir => empty set =>
/// merging off, strict duplicate-key errors — deterministic, no heuristics.
pub(crate) fn init_resource_merge(schema_dir: &str) {
    let names = ResourceRegistry::load_all(schema_dir)
        .map(|r| r.resources.keys().cloned().collect::<std::collections::HashSet<_>>())
        .unwrap_or_default();
    include_processor::set_resource_types(names);
}

/// Fold `_satz_merge_<n>_<key>` top-level maps (colliding resource keys renamed
/// during include expansion) back into `<key>`, id by id: distinct ids union; the same
/// id with deep-equal content collapses with a note (a repeated definition means "this
/// resource should exist"); the same id with different content is an error.
pub(crate) fn merge_renamed_resource_keys(value: serde_yaml::Value) -> Result<serde_yaml::Value, Box<dyn std::error::Error>> {
    let serde_yaml::Value::Mapping(map) = value else {
        return Ok(value);
    };
    let mut out = serde_yaml::Mapping::new();
    let mut errors: Vec<String> = Vec::new();

    for (k, v) in map {
        // Depth first: collisions inside a folder/project block are renamed at that
        // level, so the fold must reach them there.
        let v = match v {
            serde_yaml::Value::Mapping(_) => merge_renamed_resource_keys(v)?,
            other => other,
        };
        let Some(orig_key) = k
            .as_str()
            .and_then(|ks| ks.strip_prefix(include_processor::MERGE_KEY_PREFIX))
            // strip the running index: "0_google_logging_organization_sink"
            .and_then(|rest| rest.split_once('_').map(|(_, key)| key.to_string()))
        else {
            out.insert(k, v);
            continue;
        };

        let target_key = serde_yaml::Value::String(orig_key.clone());
        let serde_yaml::Value::Mapping(additions) = v else {
            errors.push(format!("'{}' is declared twice but not as a mapping both times", orig_key));
            continue;
        };
        let target = out
            .entry(target_key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let serde_yaml::Value::Mapping(existing) = target else {
            errors.push(format!("'{}' is declared twice but not as a mapping both times", orig_key));
            continue;
        };
        for (id, body) in additions {
            match existing.get(&id) {
                None => {
                    existing.insert(id, body);
                }
                Some(prev) if *prev == body => {
                    println!(
                        "note: {}.{} is defined in more than one included file with identical content — merged",
                        orig_key,
                        id.as_str().unwrap_or("?")
                    );
                }
                Some(_) => {
                    errors.push(format!(
                        "{}.{} is defined in more than one included file with different content",
                        orig_key,
                        id.as_str().unwrap_or("?")
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(serde_yaml::Value::Mapping(out))
    } else {
        Err(format!("Conflicting merged resource entries:\n  - {}", errors.join("\n  - ")).into())
    }
}

pub(crate) fn extract_variables(value: &serde_yaml::Value) -> HashMap<String, serde_yaml::Value> {
    let mut vars = HashMap::new();
    collect_variables_recursive(value, &mut vars);
    vars
}

fn is_variables_key(k: &serde_yaml::Value) -> bool {
    k.as_str().is_some_and(|s| {
        s == "variables" || s.starts_with(include_processor::INCLUDE_VARS_PREFIX)
    })
}

fn extract_mapping_vars(variables: &serde_yaml::Mapping, vars: &mut HashMap<String, serde_yaml::Value>) {
    for (k, v) in variables {
        if let serde_yaml::Value::String(k_str) = k {
            vars.insert(k_str.clone(), v.clone());
        }
    }
}

fn collect_variables_recursive(value: &serde_yaml::Value, vars: &mut HashMap<String, serde_yaml::Value>) {
    if let serde_yaml::Value::Mapping(map) = value {
        // Recurse into non-variable children first (lowest priority)
        for (k, v) in map {
            if !is_variables_key(k) {
                collect_variables_recursive(v, vars);
            }
        }
        // Apply renamed include vars (medium priority — overwritten by direct variables:)
        for (k, v) in map {
            if k.as_str().is_some_and(|s| s.starts_with(include_processor::INCLUDE_VARS_PREFIX)) {
                if let serde_yaml::Value::Mapping(variables) = v {
                    extract_mapping_vars(variables, vars);
                }
            }
        }
        // Apply direct variables: block last (highest priority at this level)
        if let Some(serde_yaml::Value::Mapping(variables)) = map.get("variables") {
            extract_mapping_vars(variables, vars);
        }
    } else if let serde_yaml::Value::Sequence(seq) = value {
        for item in seq {
            collect_variables_recursive(item, vars);
        }
    }
}

fn strip_variables_recursive(value: serde_yaml::Value) -> serde_yaml::Value {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let cleaned: serde_yaml::Mapping = map
                .into_iter()
                .filter_map(|(k, v)| {
                    if is_variables_key(&k) {
                        None
                    } else {
                        Some((k, strip_variables_recursive(v)))
                    }
                })
                .collect();
            serde_yaml::Value::Mapping(cleaned)
        }
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.into_iter().map(strip_variables_recursive).collect())
        }
        other => other,
    }
}

pub(crate) fn merge_variables(value: serde_yaml::Value) -> serde_yaml::Value {
    // Collect top-level variables before stripping so they can be promoted to root
    let top_level_vars = if let serde_yaml::Value::Mapping(ref map) = value {
        map.get("variables").and_then(|v| {
            if let serde_yaml::Value::Mapping(m) = v { Some(m.clone()) } else { None }
        })
    } else {
        None
    };

    let value = strip_variables_recursive(value);

    if let serde_yaml::Value::Mapping(mut map) = value {
        if let Some(variables) = top_level_vars {
            for (k, v) in variables {
                if !map.contains_key(&k) {
                    map.insert(k, v);
                }
            }
        }
        serde_yaml::Value::Mapping(map)
    } else {
        value
    }
}

pub(crate) fn resolve_yaml_custom_tags(value: serde_yaml::Value) -> serde_yaml::Value {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let mut new_map = serde_yaml::Mapping::new();
            for (k, v) in map {
                let processed_k = resolve_yaml_custom_tags(k);
                let key_str = processed_k.as_str().unwrap_or("").to_string();
                let mut processed_v = resolve_yaml_custom_tags(v);

                // Coerce known string fields if they are numbers
                if matches!(key_str.as_str(), "customer-organization-id" | "infra-bucket-name" | "project_id" | "org_id" | "folder_id") {
                    if let serde_yaml::Value::Number(n) = processed_v {
                        processed_v = serde_yaml::Value::String(n.to_string());
                    }
                }

                new_map.insert(processed_k, processed_v);
            }
            serde_yaml::Value::Mapping(new_map)
        }
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.into_iter().map(resolve_yaml_custom_tags).collect())
        }
        serde_yaml::Value::Tagged(tagged) => {
            if tagged.tag == "!expr" {
                // Resolve to an interpolation string: `!expr a.b.c` -> "${a.b.c}".
                // The transpiler's string_to_hcl_expr renders that as a template, which
                // in HCL is semantically the bare expression — Terraform tracks the
                // dependency, which is the whole point of !expr over !format.
                //
                // Passing the Tagged value through (as this arm used to) broke the
                // typed Config deserialization whenever !expr appeared in key position
                // ("untagged and internally tagged enums do not support enum input").
                let inner = resolve_yaml_custom_tags(tagged.value);
                if let serde_yaml::Value::String(s) = inner {
                    let t = s.trim();
                    // Already-interpolated input stays as-is; wrapping would nest ${}.
                    let out = if t.contains("${") { t.to_string() } else { format!("${{{}}}", t) };
                    return serde_yaml::Value::String(out);
                }
                // Non-string payloads keep the tag and fail downstream, as before.
                return serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
                    tag: tagged.tag,
                    value: inner,
                }));
            }
            if tagged.tag == "!join" {
                if let serde_yaml::Value::Sequence(items) = tagged.value {
                    let mut result = String::new();
                    for item in items {
                        let inner = resolve_yaml_custom_tags(item);
                        match inner {
                            serde_yaml::Value::String(s) => result.push_str(&s),
                            serde_yaml::Value::Number(n) => result.push_str(&n.to_string()),
                            serde_yaml::Value::Bool(b) => result.push_str(&b.to_string()),
                            _ => {}
                        }
                    }
                    return serde_yaml::Value::String(result);
                } else {
                    let inner = resolve_yaml_custom_tags(tagged.value);
                    return match inner {
                        serde_yaml::Value::String(s) => serde_yaml::Value::String(s),
                        serde_yaml::Value::Number(n) => serde_yaml::Value::String(n.to_string()),
                        _ => serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
                            tag: tagged.tag,
                            value: inner,
                        }))
                    };
                }
            } else if tagged.tag == "!format" {
                if let serde_yaml::Value::Sequence(items) = tagged.value {
                    if items.is_empty() { return serde_yaml::Value::Null; }
                    let fmt_v = resolve_yaml_custom_tags(items[0].clone());
                    let mut fmt = match fmt_v {
                        serde_yaml::Value::String(s) => s,
                        _ => return serde_yaml::Value::Null,
                    };
                    for i in 1..items.len() {
                        let arg = resolve_yaml_custom_tags(items[i].clone());
                        let arg_str = match arg {
                            serde_yaml::Value::String(s) => s,
                            serde_yaml::Value::Number(n) => n.to_string(),
                            serde_yaml::Value::Bool(b) => b.to_string(),
                            _ => "".to_string(),
                        };
                        fmt = fmt.replacen("{}", &arg_str, 1);
                    }
                    return serde_yaml::Value::String(fmt);
                }
            }
            serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
                tag: tagged.tag,
                value: resolve_yaml_custom_tags(tagged.value),
            }))
        }
        _ => value,
    }
}

/// Provider-schema sync, previously driven by the YAML estate's `providers:`
/// block. The Satz path resolves providers through the fragment pipeline and does
/// not call this; `update-schema` remains the explicit door.
#[allow(dead_code)]
fn sync_schemas(tool_config: &mut ToolConfig, runtime_config: &ToolConfig, provider_names: &[String], config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut updated = false;
    let all_known = tool_config.all_providers(); // Just names

    for p in provider_names {
        // Categorize if not already known
        let (p_name, _) = ToolConfig::parse_provider_string(p);
        
        if !all_known.contains(&p_name) {
             // Add purely as name for now, or assume default version if added dynamically
            if p_name.starts_with("google") {
                if !tool_config.google_providers.iter().any(|existing| ToolConfig::parse_provider_string(existing).0 == p_name) {
                    tool_config.google_providers.push(p.to_string());
                    updated = true;
                }
            } else if p_name.starts_with("aws") {
                if !tool_config.aws_providers.iter().any(|existing| ToolConfig::parse_provider_string(existing).0 == p_name) {
                    tool_config.aws_providers.push(p.to_string());
                    updated = true;
                }
            } else if p_name.starts_with("az") {
                if !tool_config.azure_providers.iter().any(|existing| ToolConfig::parse_provider_string(existing).0 == p_name) {
                    tool_config.azure_providers.push(p.to_string());
                    updated = true;
                }
            } else if p_name.starts_with("ali")
                 && !tool_config.alibaba_providers.iter().any(|existing| ToolConfig::parse_provider_string(existing).0 == p_name) {
                    tool_config.alibaba_providers.push(p.to_string());
                    updated = true;
                }
        }

        // Generate schema if file missing
        // For schema generation, we need the version.
        // If it's a new provider just added, it uses the global default or whatever is in the string.
        // We need to resolve the version from the tool_config (which might have been just updated)
        
        let (p_name_resolved, p_ver_resolved) = tool_config.parsed_providers().into_iter().find(|(n,_)| n == &p_name)
             .unwrap_or_else(|| ToolConfig::parse_provider_string_with_default(p, &tool_config.provider_version));

        let out_name = p_name_resolved.split('/').next_back().unwrap_or(&p_name_resolved);
        let schema_path = PathBuf::from(&runtime_config.schema_dir).join(format!("{}.json", out_name));
        if !schema_path.exists() {
            // Ensure schema directory exists
            fsx::create_dir_all(&runtime_config.schema_dir)?;

            println!("Generating schema for provider: {} version {}...", p_name_resolved, p_ver_resolved);
            ResourceRegistry::generate_schema(&runtime_config.tf_tool, &p_name_resolved, &p_ver_resolved, schema_path.to_str().unwrap())?;
            updated = true;
        }
    }

    if updated {
        tool_config.save(config_path)?;
        println!("Updated config.toml and schemas.");
    }

    Ok(())
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
fn load_discovery_config(
    path: Option<PathBuf>,
    tool_config: &ToolConfig,
    presets_dir: &str,
) -> Result<Option<DiscoveryConfig>, Box<dyn std::error::Error>> {
    let config_path = if let Some(p) = path {
        resolve_against(presets_dir, p)
    } else if let Some(p_str) = &tool_config.discovery_config {
        resolve_against(presets_dir, PathBuf::from(p_str))
    } else {
        // `get-presets` writes the presets library to presets_dir (beside config.toml).
        let default = resolve_against(presets_dir, PathBuf::from("discovery-config.yaml"));
        if default.exists() {
            default
        } else {
            return Ok(None);
        }
    };

    if !config_path.exists() {
         return Err(format!("Discovery configuration file not found at: {}", config_path.display()).into());
    }

    let content = fsx::read_to_string(&config_path)?;
    let config: DiscoveryConfig = serde_yaml::from_str(&content)?;

    let total_types = config.resource_types.len();
    let enabled_types = config.resource_types.values().filter(|v| v.import).count();
    println!("Loaded {} resource types from discovery config file '{}' ({} enabled for import).", total_types, config_path.display(), enabled_types);

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
        
        println!("\n{:=<80}", "");
        println!("COMMAND: {}", subcmd.get_name());
        println!("{:=<80}\n", "");
        
        print_recursive_help(&mut subcmd);
    }
}


use crate::github::{api_error, api_get, API_URL, REPO};

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
    if compare_versions(current, &latest_version) < 0 {
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

async fn run_self_update(download_readme: bool, open_readme: bool, check_only: bool, skip_checksum: bool, preferred_editor: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {

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

    if compare_versions(current_version, latest_version) < 0 {
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
        let installer_bytes = client.get(&installer_asset.browser_download_url).send().await?.bytes().await?;

        // Checksum verification
        let checksum_asset = release.assets.iter()
            .find(|a| a.name == "satz-installer.sh.sha256");
        match checksum_asset {
            Some(asset) => {
                let expected_raw = client.get(&asset.browser_download_url)
                    .send().await?.text().await?;
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
        let temp_file = std::env::temp_dir().join(format!("satz-installer-{}.sh", std::process::id()));
        fsx::write(&temp_file, &installer_bytes)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fsx::set_permissions(&temp_file, std::fs::Permissions::from_mode(0o755))?;

            let status = std::process::Command::new("sh")
                .arg(&temp_file)
                .status()?;
            let _ = std::fs::remove_file(&temp_file);

            if status.success() {
                println!("✅ Update installed successfully!");
                println!("   Please restart your terminal or run: source ~/.profile");

                if download_readme {
                    match download_and_open_readme(&client, REPO, latest_version, open_readme, preferred_editor).await {
                        Ok(Some(path)) => println!("README: {}", path.display()),
                        Ok(None) => {}
                        Err(e) => eprintln!("⚠️  Warning: Could not download README: {}", e),
                    }
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

async fn download_and_open_readme(
    client: &reqwest::Client,
    repo: &str,
    version: &str,
    open_after_download: bool,
    preferred_editor: Option<&str>,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let download_dir = get_download_dir()?;
    let readme_path = download_dir.join(format!("satz-{}-README.md", version));
    let readme_url = format!("https://raw.githubusercontent.com/{}/main/README.md", repo);
    println!("\n📄 Downloading README to '{}'...", readme_path.display());
    let readme_content = client.get(&readme_url).send().await?.text().await?;
    fsx::write(&readme_path, &readme_content)?;
    if open_after_download {
        open_file(&readme_path, preferred_editor)?;
    }
    Ok(Some(readme_path))
}

fn get_download_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home).join("Downloads"))
    }
    
    #[cfg(target_os = "linux")]
    {
        // Try XDG_DOWNLOAD_DIR first, fallback to ~/Downloads
        if let Ok(dir) = std::env::var("XDG_DOWNLOAD_DIR") {
            Ok(PathBuf::from(dir))
        } else {
            let home = std::env::var("HOME")?;
            Ok(PathBuf::from(home).join("Downloads"))
        }
    }
    
    #[cfg(target_os = "windows")]
    {
        use std::env;
        let user_profile = env::var("USERPROFILE")?;
        Ok(PathBuf::from(user_profile).join("Downloads"))
    }
    
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("Unsupported platform for download directory".into())
    }
}

fn open_file(path: &Path, preferred_editor: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let path_str = path.to_str()
        .ok_or_else(|| format!("File path {:?} contains non-UTF-8 characters", path))?;

    let editor_env = std::env::var("EDITOR").ok();
    let editor = preferred_editor.or(editor_env.as_deref());

    if let Some(editor) = editor {
        println!("   Opening '{}' with '{}'...", path_str, editor);
        // Try direct invocation first — works when the editor binary is in PATH
        let result = std::process::Command::new(editor).arg(path).status();
        match result {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // On macOS, fall back to `open -a <editor> <file>` so GUI apps
                // (like Zed, VS Code) can be found by app-bundle name even when
                // their CLI wrapper is not on the system PATH.
                #[cfg(target_os = "macos")]
                {
                    let open_result = std::process::Command::new("open")
                        .args(["-a", editor, path_str])
                        .status();
                    if open_result.map(|s| s.success()).unwrap_or(false) {
                        return Ok(());
                    }
                }
                return Err(format!(
                    "Editor '{}' not found — is it installed and on your PATH?\n\
                     Hint: set preferred_editor to the full path in ~/.config/satz/satz.toml\n\
                     e.g.  preferred_editor = \"/usr/local/bin/zed\"",
                    editor
                ).into());
            }
            Err(e) => return Err(format!("Failed to launch editor '{}': {}", editor, e).into()),
        }
    }

    // No editor configured — use OS default
    #[cfg(target_os = "macos")]
    {
        println!("   Opening '{}' with system default app...", path_str);
        std::process::Command::new("open")
            .arg(path_str)
            .status()
            .map_err(|e| format!("Failed to open '{}' with 'open': {}", path_str, e))?;
    }
    #[cfg(target_os = "linux")]
    {
        println!("   Opening '{}' with xdg-open...", path_str);
        if std::process::Command::new("xdg-open").arg(path_str).status().is_err() {
            return Err(format!(
                "Could not open '{}': xdg-open failed and neither preferred_editor nor $EDITOR is set",
                path_str
            ).into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        println!("   Opening '{}' with system default app...", path_str);
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path_str])
            .status()
            .map_err(|e| format!("Failed to open '{}': {}", path_str, e))?;
    }
    Ok(())
}

fn compare_versions(v1: &str, v2: &str) -> i32 {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|s| s.parse::<u32>().unwrap_or(0))
            .collect()
    };
    
    let v1_parts = parse_version(v1);
    let v2_parts = parse_version(v2);
    
    let max_len = v1_parts.len().max(v2_parts.len());
    
    for i in 0..max_len {
        let v1_val = v1_parts.get(i).copied().unwrap_or(0);
        let v2_val = v2_parts.get(i).copied().unwrap_or(0);
        
        if v1_val < v2_val {
            return -1;
        } else if v1_val > v2_val {
            return 1;
        }
    }
    
    0
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

pub(crate) fn print_yaml_error_context(content: &str, err: &serde_yaml::Error) {
    // serde_yaml reports duplicate-key errors at the enclosing mapping's start (line 1
    // for top-level keys), which points nowhere useful. Locate the actual occurrences
    // in the merged document and attribute each to the file it came from.
    let msg = err.to_string();
    if let Some(key) = msg
        .split_once("duplicate entry with key \"")
        .and_then(|(_, r)| r.split('\"').next())
    {
        let occurrences = find_key_occurrences(content, key);
        if occurrences.len() >= 2 {
            eprintln!("\nDuplicate key '{}' — defined at:", key);
            for (line_no, source) in &occurrences {
                match source {
                    Some(src) => eprintln!("  - line {} (from include: {})", line_no, src),
                    None => eprintln!("  - line {} (main file)", line_no),
                }
            }
            eprintln!(
                "\nTwo files (or two places in one file) declare '{}' at the same level. \
                 YAML forbids duplicate keys. Resource-type keys merge automatically \
                 across files when provider schemas are present (run `satz \
                 update-schema` if they are not); hoisted-scope types may also be \
                 declared per fragment inside distinct folder/project blocks. See \
                 'Hoisted scopes' in the README.\n",
                key
            );
            return;
        }
    }

    if let Some(location) = err.location() {
        // serde_yaml reports 1-based lines, but has been seen to report 0; saturating
        // keeps a malformed location from panicking inside the error reporter itself.
        let line_idx = location.line().saturating_sub(1);
        let lines: Vec<&str> = content.lines().collect();

        if line_idx < lines.len() {
            // Scan backward from the error line to find the nearest satz:source: annotation
            let source_file = lines[..=line_idx]
                .iter()
                .rev()
                .find_map(|l| l.trim().strip_prefix("# satz:source: "));

            if let Some(src) = source_file {
                eprintln!("\nError in included file: {}", src);
            }

            eprintln!("\nError context (line {}):", line_idx + 1);
            eprintln!("--------------------------------------------------");

            let start = usize::max(0, line_idx.saturating_sub(2));
            let end = usize::min(lines.len() - 1, line_idx + 2);

            for i in start..=end {
                let marker = if i == line_idx { ">>" } else { "  " };
                eprintln!("{} {:4} | {}", marker, i + 1, lines[i]);
            }
            eprintln!("--------------------------------------------------\n");
        }
    }
}

/// All lines in the merged document where `key:` opens a mapping entry, each attributed
/// to the include file it came from (`None` = the main file). Source attribution tracks
/// the `# satz:source:` / `# satz:source-end:` markers as a stack, since includes
/// nest.
fn find_key_occurrences(content: &str, key: &str) -> Vec<(usize, Option<String>)> {
    let mut source_stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(src) = trimmed.strip_prefix("# satz:source: ") {
            source_stack.push(src.to_string());
            continue;
        }
        if trimmed.strip_prefix("# satz:source-end: ").is_some() {
            source_stack.pop();
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        let rest = line.trim_start();
        if let Some(after) = rest.strip_prefix(key) {
            if after.starts_with(':') {
                out.push((idx + 1, source_stack.last().cloned()));
            }
        }
    }
    out
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

async fn run_open_readme(preferred_editor: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent("satz-open-readme")
        .build()?;
    match download_and_open_readme(&client, REPO, "latest", true, preferred_editor).await {
        Ok(Some(path)) => println!("README saved to: {}", path.display()),
        Ok(None) => {}
        Err(e) => return Err(e),
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
mod resource_merge_tests {
    use super::*;

    fn fold(yaml: &str) -> Result<serde_yaml::Value, Box<dyn std::error::Error>> {
        merge_renamed_resource_keys(serde_yaml::from_str(yaml).unwrap())
    }

    /// The user-facing semantics: merging steps INTO the colliding key and unions the
    /// ids; the rename is only how the document survives the YAML parser.
    #[test]
    fn distinct_ids_union_under_the_original_key() {
        let v = fold(
            "google_logging_organization_sink:\n  archive:\n    name: a\n\
             _satz_merge_0_google_logging_organization_sink:\n  metrics:\n    name: b\n",
        )
        .unwrap();
        let sinks = v.get("google_logging_organization_sink").unwrap().as_mapping().unwrap();
        assert_eq!(sinks.len(), 2);
        assert!(sinks.contains_key(serde_yaml::Value::String("archive".into())));
        assert!(sinks.contains_key(serde_yaml::Value::String("metrics".into())));
        assert!(v.as_mapping().unwrap().len() == 1, "renamed key must not survive");
    }

    #[test]
    fn identical_id_collapses_and_different_content_errors() {
        let v = fold(
            "s:\n  a:\n    name: x\n_satz_merge_0_s:\n  a:\n    name: x\n",
        )
        .unwrap();
        assert_eq!(v.get("s").unwrap().as_mapping().unwrap().len(), 1);

        let err = fold("s:\n  a:\n    name: x\n_satz_merge_0_s:\n  a:\n    name: y\n")
            .expect_err("conflicting id must error");
        assert!(err.to_string().contains("s.a"), "{err}");
    }

    /// The real-world case that forced depth-awareness: two presets included inside
    /// the SAME folder block both declare the sink key one level down. The rename must
    /// fire at that level and the fold must recurse to reach it.
    #[test]
    fn colliding_keys_inside_a_folder_block_merge() {
        include_processor::set_resource_types(
            ["google_logging_organization_sink".to_string()].into_iter().collect(),
        );
        let dir = std::env::temp_dir().join(format!("satz-merge-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.yaml"), "google_logging_organization_sink:\n  archive:\n    name: a\n").unwrap();
        std::fs::write(dir.join("b.yaml"), "google_logging_organization_sink:\n  metrics:\n    name: b\n").unwrap();
        std::fs::write(
            dir.join("main.yaml"),
            "folder:\n  logging_folder:\n    display_name: L\n    !include a.yaml\n    !include b.yaml\n",
        )
        .unwrap();

        let (text, _) = include_processor::process_includes_with_ops(&dir.join("main.yaml"), &[]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let raw: serde_yaml::Value = serde_yaml::from_str(&text).expect("renamed doc parses");
        let folded = merge_renamed_resource_keys(raw).unwrap();
        let sinks = folded["folder"]["logging_folder"]["google_logging_organization_sink"]
            .as_mapping()
            .expect("merged under the folder");
        assert_eq!(sinks.len(), 2, "both sinks under one key inside the folder");
    }

    /// End-to-end through the include processor: two files, both declaring the sink
    /// key at top level, merge into one map. Relies on the process-wide resource-type
    /// set; harmless for other tests since no other fixture has duplicate keys.
    #[test]
    fn two_includes_with_the_same_resource_key_merge() {
        include_processor::set_resource_types(
            ["google_logging_organization_sink".to_string()].into_iter().collect(),
        );
        let dir = std::env::temp_dir().join(format!("satz-merge-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.yaml"), "google_logging_organization_sink:\n  archive:\n    name: a\n").unwrap();
        std::fs::write(dir.join("b.yaml"), "google_logging_organization_sink:\n  metrics:\n    name: b\n").unwrap();
        std::fs::write(dir.join("main.yaml"), "!include a.yaml\n!include b.yaml\n").unwrap();

        let (text, _) = include_processor::process_includes_with_ops(&dir.join("main.yaml"), &[]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let raw: serde_yaml::Value = serde_yaml::from_str(&text).expect("renamed doc parses");
        let folded = merge_renamed_resource_keys(raw).unwrap();
        let sinks = folded.get("google_logging_organization_sink").unwrap().as_mapping().unwrap();
        assert_eq!(sinks.len(), 2, "both sinks under one key");
    }
}

#[cfg(test)]
mod variable_table_tests {
    use super::*;

    /// Derived variables must reach the table resolved: extracted raw, they were
    /// emitted as `variable` blocks with no tfvars value, and tofu prompts
    /// interactively for every declared-but-unset variable on apply.
    #[test]
    fn derived_variables_resolve_before_extraction() {
        let yaml = r#"
variables:
  customer-prefix: &customer-prefix "acme"
  bucket-name: &bucket-name !format ["{}-audit-logs", *customer-prefix]
  bucket-url: &bucket-url !format ["gs://{}", *bucket-name]
"#;
        let raw: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let vars = extract_variables(&resolve_yaml_custom_tags(raw));
        assert_eq!(vars.get("bucket-name").and_then(|v| v.as_str()), Some("acme-audit-logs"));
        assert_eq!(vars.get("bucket-url").and_then(|v| v.as_str()), Some("gs://acme-audit-logs"));
    }
}

#[cfg(test)]
mod yaml_transpile_tests {
    //! `transpile` accepts BOTH dialects (owner, 2026-08-23), so the YAML arm is
    //! a supported path and needs a gate of its own. It had none, which is how a
    //! v0.40.0 change silently routed YAML estates into the Satz parser.
    use super::*;

    #[test]
    fn a_yaml_estate_emits_hcl_through_the_legacy_walk() {
        let dir = std::env::temp_dir().join(format!("satz-yaml-transpile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let estate = dir.join("estate.yaml");
        std::fs::write(
            &estate,
            "variables:\n  \
             customer-organization-id: &customer-organization-id \"123456789\"\n  \
             bucket-name: &bucket-name \"demo-bucket\"\n\
             terraform:\n  backend:\n    local:\n      path: \"terraform.tfstate\"\n\
             google_storage_bucket:\n  demo:\n    name: *bucket-name\n    location: \"EU\"\n",
        )
        .expect("write estate");

        // Every field carries a serde default, so empty TOML is the config a
        // repo with no config.toml would get.
        let cfg: ToolConfig = toml::from_str("").expect("default config");
        let project = pipeline_a_generate(&estate, "none", HashMap::new(), HashMap::new(), &[], &cfg)
            .expect("YAML estate must transpile");
        assert!(
            project.main_tf.contains(r#"resource "google_storage_bucket" "demo""#),
            "expected the bucket resource, got:\n{}",
            project.main_tf
        );
        assert!(
            project.main_tf.contains(r#""demo-bucket""#),
            "the anchor must resolve, got:\n{}",
            project.main_tf
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

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
        let gen = resolve_satz_input(estate.clone(), &[]).expect("compile twin");
        let via_yaml = crate::org_policy::resolve_config_vars(&gen, &[]).expect("yaml route");

        let sorted = |m: &HashMap<String, serde_yaml::Value>| {
            let mut v: Vec<(String, String)> = m
                .iter()
                .map(|(k, val)| (k.clone(), serde_yaml::to_string(val).unwrap_or_default()))
                .collect();
            v.sort();
            v
        };
        assert_eq!(sorted(&via_pipeline), sorted(&via_yaml), "variable tables diverged");

        // The facts the assertion is worth nothing without: kebab keys reach the
        // table, first-definition-wins held, and interpolation was resolved.
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
mod yaml_estate_gate {
    //! THE gate for the YAML dialect. It is a supported input (owner rule), and
    //! until this existed nothing ran a YAML estate end to end — which is how
    //! two regressions shipped in one day with the Satz fleet fully green:
    //! v0.40.0 sent every estate to the fragment pipeline, so `merge-presets`
    //! died on a Satz parse error against a `.yaml` file; v0.41.0 had
    //! `migrate-to-satz` emit Satz that would not compile while its own gate
    //! reported PROVEN, because that gate runs the legacy walk.
    //!
    //! Self-contained: no schema registry, no live org, no customer repo. It
    //! replaces wsw as the thing to test against.
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/yaml-estate")
    }

    /// Addresses the estate declares. Written out rather than counted, because
    /// the failure mode being guarded is resources DISAPPEARING — a count is
    /// satisfied by the wrong set, and "it got smaller" was exactly the bug.
    const EXPECTED: &[&str] = &[
        "google_org_policy_policy.compute_requireOsLogin",
        "google_organization_iam_member.",
        "google_folder.infra_folder",
        "google_project.demo_project",
        "google_project_iam_member.",
    ];

    fn addresses(main_tf: &str) -> Vec<String> {
        main_tf
            .lines()
            .filter_map(|l| l.trim().strip_prefix("resource \""))
            .map(|r| {
                let mut it = r.split('"').filter(|p| !p.trim().is_empty() && *p != " ");
                let ty = it.next().unwrap_or_default();
                let label = it.next().unwrap_or_default();
                format!("{}.{}", ty, label)
            })
            .collect()
    }

    fn pipeline_a(dir: &Path) -> String {
        crate::corpus::run_case(dir, &[dir.to_path_buf()])
    }

    /// The fixture's type table: an explicit ALLOWLIST, standing in for the
    /// provider schemas.
    ///
    /// Not "anything starting with `google_`" — that shortcut (which
    /// `CorpusTable` still takes) claims `google_labels` exists, which sends a
    /// genuine `labels { … }` attribute block down the resource path and fails
    /// a conversion the real registry handles fine. Nested attribute blocks and
    /// resource maps are the same syntax; only a schema separates them, so a
    /// stub that guesses is a stub that lies.
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
    fn a_yaml_estate_emits_every_resource_it_declares() {
        let out = pipeline_a(&fixture());
        let addrs = addresses(&out);
        for want in EXPECTED {
            assert!(
                addrs.iter().any(|a| a.starts_with(want)),
                "{} missing from the emitted HCL — declared resources must never \
                 be silently dropped.\ngot: {:?}",
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
    }

    /// The v0.41.0 regression, end to end: convert the estate AND its pack, then
    /// compile the result through the FRAGMENT pipeline — the one that will
    /// actually read it — and require the same resources as the legacy walk.
    /// The conversion gate alone cannot catch this: it runs pipeline A on both
    /// sides, and pipeline A accepts everything the converter might get wrong.
    #[test]
    fn a_converted_estate_compiles_as_satz_and_emits_the_same_resources() {
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
        let folded =
            satz_core::pipeline::fold_fragments(&FixtureTypes, &fe.fragments);
        assert!(folded.conflicts().is_empty(), "conflicts: {:?}", folded.conflicts());
        let ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        let b = crate::emitter::emit(&folded, &ctx).expect("emit").main_tf;

        let mut a_addrs = addresses(&pipeline_a(&src_dir));
        let mut b_addrs = addresses(&b);
        a_addrs.sort();
        b_addrs.sort();
        assert_eq!(
            a_addrs, b_addrs,
            "the converted estate emits a different resource set than the YAML it came from"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod corpus {
    //! The differential corpus: every composition scenario battle-proven in the
    //! field, snapshot-gated. This is the contract the satz-core swap (and any
    //! future refactor of composition semantics) must honor byte-for-byte on
    //! sorted output. Regenerate deliberately with UPDATE_CORPUS=1 and review the
    //! snapshot diff like production code.
    use super::*;
    use std::path::{Path, PathBuf};

    /// The corpus schema fixture — a real provider schema trimmed to the types
    /// the fixtures use. Both pipelines classify types through THIS, the same
    /// way production does, instead of a hand-written table guessing at what a
    /// resource is. Without it pipeline A has no registry at all and files every
    /// nested `google_*` block away as an attribute of its parent.
    pub(super) fn schema_dir() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas").to_string_lossy().into_owned()
    }

    pub(super) fn registry() -> ResourceRegistry {
        ResourceRegistry::load_all(&schema_dir()).expect("corpus schema fixture")
    }

    pub(super) fn run_case(case_dir: &Path, include_paths: &[PathBuf]) -> String {
        include_processor::set_resource_types(
            ["google_logging_organization_sink".to_string()].into_iter().collect(),
        );
        let main = case_dir.join("main.yaml");
        let (text, _) =
            include_processor::process_includes_with_ops(&main, include_paths).expect("expand");
        let raw: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse");
        let raw = merge_renamed_resource_keys(raw).expect("fold");
        let variables = extract_variables(&resolve_yaml_custom_tags(raw.clone()));
        let resolved = resolve_yaml_custom_tags(merge_variables(raw));
        let config: Config = serde_yaml::from_value(resolved).expect("config");
        let t = Transpiler::new(
            &config,
            Some(registry()),
            vec!["google_project_service".to_string(), ".*_iam_member".to_string()],
            "none".to_string(),
            variables,
            HashMap::new(),
            HashMap::new(),
        );
        let project = t.transpile().expect("transpile");
        let mut lines: Vec<&str> =
            project.main_tf.lines().filter(|l| !l.trim().is_empty()).collect();
        lines.sort_unstable();
        format!("{}\n---tfvars---\n{}", lines.join("\n"), {
            let mut v: Vec<&str> = project.tfvars.lines().collect();
            v.sort_unstable();
            v.join("\n")
        })
    }

}

#[cfg(test)]
mod differential {
    //! THE corpus gate (M3 step 6). For every case: (1) compile `main.satz` and
    //! run pipeline A over the result, which must reproduce the recorded
    //! snapshot, and (2) run pipeline B over the same source. PARITY cases must
    //! match byte-identically on sorted lines, so B is gated against the snapshot
    //! transitively; the rest report their distance without failing the build —
    //! the ratchet only ever tightens.
    //!
    //! Pipeline A is here as the CONVERTER's cross-check, not as the product
    //! path: satz emits from B. It generates its own YAML from the Satz source
    //! in a temp dir, so it depends on no checked-in twins — which is what let the
    //! `.yaml` twins be deleted.
    
    use std::path::{Path, PathBuf};

    fn sorted_lines(s: &str) -> Vec<String> {
        let mut v: Vec<String> =
            s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
        v.sort_unstable();
        v
    }

    /// Cases pipeline B already reproduces byte-identically (sorted). Ratchet:
    /// additions only — removing an entry means a parity regression.
    const PARITY: &[&str] = &["billing-nested", "depth-merge", "hoist-two-folders", "override-chain", "real-packs"];

    /// The corpus resolves types through the SAME schema-backed resolver
    /// production uses, over the trimmed provider schema in `tests/schemas/`.
    ///
    /// It used to be a hand-written `CorpusTable` answering "is this a type?"
    /// with "does it start with `google_`" — which claims `google_labels` exists
    /// and sends a genuine attribute block down the resource path. The truth
    /// about type names lives in the schema; a second, guessing opinion is how
    /// two pipelines drift apart without anyone noticing.
    fn corpus_resolver(registry: &crate::ResourceRegistry) -> crate::EstateResolver<'_> {
        crate::EstateResolver { registry }
    }

    #[test]
    fn satz_twins_gate_and_pipeline_b_distance() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let corpus = root.join("tests/corpus");
        let mut cases: Vec<PathBuf> = std::fs::read_dir(&corpus)
            .expect("corpus dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("main.satz").exists())
            .collect();
        cases.sort();
        assert!(!cases.is_empty(), "no corpus case has a main.satz twin yet");
        for case in cases {
            let name = case.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(case.join("main.satz")).unwrap();
            let compiled = satz_core::satz::compile(&src)
                .unwrap_or_else(|e| panic!("{}: satz compile failed: {}", name, e));

            // GATE: pipeline A over the twin == the snapshot of the YAML case.
            // The twin's .satz packs compile to .gen.yaml siblings in the temp
            // dir, exactly like resolve_satz_input does for real estates.
            let tmp = std::env::temp_dir().join(format!("satz-diff-{}", name));
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).unwrap();
            std::fs::write(tmp.join("main.yaml"), &compiled.yaml).unwrap();
            let mut queue = compiled.satz_deps.clone();
            let mut done = std::collections::HashSet::new();
            while let Some(dep) = queue.pop() {
                if !done.insert(dep.clone()) {
                    continue;
                }
                let dep_src = std::fs::read_to_string(case.join(&dep))
                    .or_else(|_| std::fs::read_to_string(root.join(&dep)))
                    .unwrap_or_else(|e| panic!("{}: dep {} unreadable: {}", name, dep, e));
                let dep_compiled = satz_core::satz::compile(&dep_src)
                    .unwrap_or_else(|e| panic!("{}: dep {} failed: {}", name, dep, e));
                let gen = dep.trim_end_matches(".satz").to_string() + ".gen.yaml";
                let gen_path = tmp.join(&gen);
                if let Some(parent) = gen_path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&gen_path, &dep_compiled.yaml).unwrap();
                queue.extend(dep_compiled.satz_deps);
            }
            let include_paths = vec![tmp.clone(), root.to_path_buf()];
            let reg = super::corpus::registry();
            let a = super::corpus::run_case(&tmp, &include_paths);
            // The module doc promises UPDATE_CORPUS=1 regeneration; this assertion
            // never implemented it, so the only way to move a snapshot was to
            // hand-transcribe it out of a panic message.
            let expected_path = case.join("expected.sorted.txt");
            if std::env::var("UPDATE_CORPUS").is_ok() {
                std::fs::write(&expected_path, &a).unwrap();
                eprintln!("{}: snapshot regenerated — review the diff", name);
            }
            let expected = std::fs::read_to_string(&expected_path).unwrap();
            assert_eq!(
                expected, a,
                "{}: pipeline A over main.satz diverged from the snapshot — the twin is not a faithful conversion",
                name
            );

            // DIFF: pipeline B over the same source.
            let case_dir = case.clone();
            let fe = satz_core::pipeline::compile_estate(
                "main.satz",
                &src,
                &corpus_resolver(&reg),
                &|p| {
                    std::fs::read_to_string(case_dir.join(p))
                        .or_else(|_| std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p)))
                        .map_err(|e| e.to_string())
                },
            )
            .unwrap_or_else(|e| panic!("{}: pipeline B front-end failed: {}", name, e));
            let folded = satz_core::pipeline::fold_fragments(&corpus_resolver(&reg), &fe.fragments);
            assert!(
                folded.conflicts().is_empty(),
                "{}: pipeline B raised conflicts on a conflict-free case: {:?}",
                name,
                folded.conflicts()
            );
            let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
            // Same as production: without the registry the emitter drops
            // schema-derived detail (it silently lost every alert policy's
            // notification_channels), and the corpus then "proved" parity
            // between two equally impoverished outputs.
            ctx.registry = Some(&reg);
            let b = match crate::emitter::emit(&folded, &ctx) {
                Ok(out) => {
                    format!("{}\n---tfvars---\n{}", out.main_tf, crate::emitter::emit_tfvars(&fe.tfvars))
                }
                Err(e) => {
                    assert!(
                        !PARITY.contains(&name.as_str()),
                        "{}: PARITY case failed to emit: {}",
                        name,
                        e
                    );
                    eprintln!("differential[{}]: emitter incomplete: {}", name, e);
                    continue;
                }
            };

            let av = sorted_lines(&a);
            let bv = sorted_lines(&b);
            if PARITY.contains(&name.as_str()) {
                assert_eq!(av, bv, "{}: PARITY case regressed", name);
            } else {
                let aset: std::collections::BTreeSet<&String> = av.iter().collect();
                let bset: std::collections::BTreeSet<&String> = bv.iter().collect();
                let matched = aset.intersection(&bset).count();
                eprintln!(
                    "differential[{}]: {} lines matched, {} only in A (walk), {} only in B (fold) — not yet ratcheted",
                    name,
                    matched,
                    aset.difference(&bset).count(),
                    bset.difference(&aset).count()
                );
                if std::env::var("DIFF_DETAIL").is_ok() {
                    for l in aset.difference(&bset) {
                        eprintln!("A| {}", l);
                    }
                    for l in bset.difference(&aset) {
                        eprintln!("B| {}", l);
                    }
                }
            }
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
    //! `"import-id"` on every kind of emitted resource, both pipelines. Pipeline
    //! B used to drop it silently for IAM bindings and nested project services,
    //! and neither pipeline had a channel for memberships. Parity is the gate:
    //! the walk over the compiled twin must produce the same import blocks.
    use super::*;
    use std::path::Path;

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

    fn sorted_lines(s: &str) -> Vec<String> {
        let mut v: Vec<String> = s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect();
        v.sort_unstable();
        v
    }

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

    fn pipeline_a(tmp: &Path, reg: ResourceRegistry) -> crate::transpiler::GeneratedProject {
        let compiled = satz_core::satz::compile(ESTATE).expect("compile twin");
        std::fs::write(tmp.join("main.yaml"), &compiled.yaml).unwrap();
        include_processor::set_resource_types(Default::default());
        let (text, _) = include_processor::process_includes_with_ops(&tmp.join("main.yaml"), &[tmp.to_path_buf()]).expect("expand");
        let raw: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse");
        let raw = merge_renamed_resource_keys(raw).expect("fold");
        let variables = extract_variables(&resolve_yaml_custom_tags(raw.clone()));
        let resolved = resolve_yaml_custom_tags(merge_variables(raw));
        let config: Config = serde_yaml::from_value(resolved).expect("config");
        let t = Transpiler::new(
            &config,
            Some(reg),
            vec!["google_project_service".to_string(), ".*_iam_member".to_string()],
            "none".to_string(),
            variables,
            HashMap::new(),
            HashMap::new(),
        );
        t.transpile().expect("transpile")
    }

    #[test]
    fn every_channel_emits_its_import_block_and_both_pipelines_agree() {
        let reg = super::corpus::registry();
        let b = pipeline_b(&reg);

        let binding = crate::emit_shared::iam_member_label("group:gcp-org-admins@example.com", "roles/browser", None);
        let membership = crate::transpiler::membership_resource_label("gcp_auditors", "user:b@example.com");
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

        let tmp = std::env::temp_dir().join("satz-import-id-channels");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let a = pipeline_a(&tmp, super::corpus::registry());
        assert_eq!(sorted_lines(&a.imports_tf), sorted_lines(&b.imports_tf), "the walk and the emitter disagree on import blocks");
        let _ = std::fs::remove_dir_all(&tmp);
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
            resolve_against("/proj/yaml", PathBuf::from("presets/discovery-config.yaml")),
            PathBuf::from("/proj/yaml/presets/discovery-config.yaml")
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
    fn discovery_config_resolves_only_in_presets_dir() {
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
        std::fs::write(yaml_legacy.join("discovery-config.yaml"), MINI_DISCOVERY).unwrap();
        assert!(load_discovery_config(None, &cfg, &p_dir).unwrap().is_none());

        // The presets library is the one and only default location.
        std::fs::write(presets.join("discovery-config.yaml"), MINI_DISCOVERY).unwrap();
        assert!(load_discovery_config(None, &cfg, &p_dir).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&root);
    }
}
