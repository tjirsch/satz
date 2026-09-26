//! What satz reads before it reads an estate: the estate's `config.toml` (`ToolConfig`,
//! parsed and then resolved against the config's own directory) and this machine's
//! `~/.config/satz/satz.toml` (`GlobalSettings` — the update frequency and the machine
//! tier of `satz silence`).

use crate::fsx;
use crate::{Commands, SilenceSub};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolConfig {
    #[serde(default = "default_yaml_dir")]
    pub yaml_dir: String,
    #[serde(default = "default_hcl_dir")]
    pub hcl_dir: String,
    /// Where the estate's interfaces are written for the projects beside it: `common/`,
    /// and one folder per project. satz rewrites it whole on every transpile.
    #[serde(default = "default_interfaces_dir")]
    pub interfaces_dir: String,
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
    pub(crate) google_providers: Vec<String>,
    #[serde(default)]
    pub(crate) aws_providers: Vec<String>,
    #[serde(default)]
    pub(crate) azure_providers: Vec<String>,
    #[serde(default)]
    pub(crate) alibaba_providers: Vec<String>,
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
}

/// The directory the estate's own `.satz` files live in. The key keeps its name; the
/// directory `init` creates, and an omitted key reads, is `satz/`.
fn default_yaml_dir() -> String { "satz".to_string() }

fn default_hcl_dir() -> String { "hcl".to_string() }

fn default_interfaces_dir() -> String { "interfaces".to_string() }

/// Includes are searched relative to the including file first, then these directories
/// (resolved from config.toml's own directory). `"."` must be present so an `!include`
/// of a file sitting next to config.toml resolves; `init` has always written it, and a
/// hand-written config.toml that omits the key used to silently lose it.
fn default_include_dirs() -> Vec<String> { vec![".".to_string(), default_yaml_dir()] }

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

/// User-level settings for satz in ~/.config/satz/satz.toml. Created on first run with defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GlobalSettings {
    /// When to check for updates: "never", "always", "daily". Default "always".
    #[serde(default = "default_self_update_frequency")]
    pub(crate) self_update_frequency: String,
    /// Last time we ran an update check (unix timestamp string). Used for "daily" throttle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_update_check: Option<String>,
    /// What this operator has said "seen, move on" to on every estate they open:
    /// `[[silence]]` tables naming a whole `kind`, never a subject. Managed by
    /// `satz silence add --machine`.
    ///
    /// Last in the struct because TOML writes tables after scalars.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) silence: Vec<crate::silence::Rule>,
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

pub(crate) fn global_settings_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".config").join("satz").join("satz.toml"))
}

/// The user's home: `HOME`, and on Windows `USERPROFILE` first — native Windows sets no
/// `HOME`, so satz read no settings there and checked for updates on every command.
/// `%USERPROFILE%\.config\satz\satz.toml` is where satz-studio writes them too.
pub(crate) fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    var.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The global settings (`~/.config/satz/satz.toml`), created with defaults
/// when absent. A file that exists but cannot be read or parsed is an error —
/// "defaults" would be a silent reset that the next save writes back.
pub(crate) fn load_global_settings() -> Result<GlobalSettings, Box<dyn std::error::Error>> {
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

pub(crate) fn save_global_settings(settings: &GlobalSettings) -> Result<(), Box<dyn std::error::Error>> {
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
            interfaces_dir: default_interfaces_dir(),
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
    if Path::new(&rc.interfaces_dir).is_relative() {
        rc.interfaces_dir = at(&rc.interfaces_dir);
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

/// Turn a TOML parse failure into something actionable: which line, what is on it, and —
/// when `--config` was pointed at a customer YAML — what the caller probably meant.
/// The raw `toml` error carries the whole file in its `Debug` output, which is what the
/// caller would otherwise see.
pub(crate) fn describe_toml_error(path: &Path, content: &str, err: &toml::de::Error) -> String {
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
    use crate::resolve_against;
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

/// Which `config.toml` this run reads, and whether it may run without one.
///
/// `--config` takes the file or the estate DIRECTORY that holds it — every path inside the
/// config resolves against the config's own directory, so either form makes the command
/// location-independent. Without `--config`, `./config.toml` is it. The match below decides
/// what happens when neither exists, and it has no wildcard arm on purpose: a new command
/// does not compile until it says whether it can run outside an estate.
pub(crate) fn config_file_path(
    config: Option<&PathBuf>,
    cmd_choice: &Commands,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = if let Some(path) = config {
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
                Commands::Transpile { .. } | Commands::ScanPlan { .. } | Commands::GenerateMigration { .. } | Commands::UpdateSchema { .. } | Commands::Import { .. } | Commands::Migrate { .. } | Commands::Bootstrap { .. } | Commands::ExportOrganizationalPolicies { .. } | Commands::DiffOrganizationalPolicies { .. } | Commands::ReportOrganizationalPolicies { .. } | Commands::GetPresets { .. } | Commands::CheckPresets { .. } | Commands::Require { .. } | Commands::ReportCompliance { .. } | Commands::Adopt { .. } | Commands::MapTypes { .. } | Commands::Scan { .. } | Commands::DocPacks { .. } | Commands::Triage { .. } | Commands::RemediationPlan { .. } | Commands::AdoptOrgPolicies { .. } | Commands::MergePresets { .. } | Commands::RunActions { .. } | Commands::Questions { .. } | Commands::Interview { .. } | Commands::Prowler { .. } | Commands::McpConfig { .. }
                | Commands::Packs { .. } | Commands::AddPack { .. } | Commands::RemovePack { .. } | Commands::CheckConsumer { .. }
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
                    if let Some(hint) = crate::misplaced_config_hint(cmd_choice) {
                        return Err(format!("--config came after the pass-through arguments.\n\n{}", hint).into());
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
    Ok(path)
}
