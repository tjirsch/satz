//! Where workload packs and the preset library come from: the `[[pack_source]]` tables
//! and the `[preset_library]` table of the estate's `config.toml` (ADR 0097), and
//! `satz pack-source add|list|remove`, which edit the first.
//!
//! A pack source has a name, which is what an origin record writes, so a row in
//! `packs_dir/origin.toml` is followed through the `config.toml` beside it by every
//! clone of the repository. The satz repository's own `packs/` is the source `satz`,
//! which is never listed.

use std::path::{Path, PathBuf};

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::settings::ToolConfig;

/// The built-in source: the `packs/` tree of the satz repository.
pub(crate) const BUILTIN: &str = "satz";
/// The branch of the satz repository the built-in source and the preset library are read from.
pub(crate) const BUILTIN_REF: &str = "main";
/// The folder of a GitHub source that holds its packs, unless `path` names another.
pub(crate) const DEFAULT_PATH: &str = "packs";

/// One `[[pack_source]]` table, as config.toml holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackSourceEntry {
    pub name: String,
    /// `owner/repository` on GitHub
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// a branch or tag of `github`; its default branch when absent
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// the folder of `github` that holds the packs; `packs` when absent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// a folder holding packs, relative to config.toml
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// The `[preset_library]` table: the one library that replaces satz's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresetLibraryEntry {
    /// `owner/repository` on GitHub; the library is its `presets/`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// a branch or tag of `github`; its default branch when absent
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// a folder, relative to config.toml; the library is its `presets/`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Where a pack source is, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    Github { repo: String, git_ref: Option<String>, path: String },
    /// the folder as config.toml writes it
    Dir(String),
}

/// A pack source: its name and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Def {
    pub name: String,
    pub location: Location,
}

impl Def {
    pub(crate) fn builtin() -> Def {
        Def {
            name: BUILTIN.to_string(),
            location: Location::Github { repo: crate::github::REPO.to_string(), git_ref: Some(BUILTIN_REF.to_string()), path: DEFAULT_PATH.to_string() },
        }
    }

    pub(crate) fn is_builtin(&self) -> bool {
        self.name == BUILTIN
    }

    /// Where it is, for a reader: `github.com/<owner>/<repo>/<path>` with `@<ref>`, or the folder.
    pub(crate) fn describe(&self) -> String {
        match &self.location {
            Location::Github { repo, git_ref, path } => match git_ref {
                Some(r) => format!("github.com/{repo}/{path}@{r}"),
                None => format!("github.com/{repo}/{path}"),
            },
            Location::Dir(dir) => dir.clone(),
        }
    }
}

/// A source name: a lowercase letter, then lowercase letters, digits and `-`.
pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("`{name}` is no pack source name — a name is a lowercase letter followed by lowercase letters, digits and `-`"));
    }
    Ok(())
}

/// `owner/repository`, each part letters, digits, `-`, `_` and `.`.
fn valid_repo(repo: &str) -> Result<(), String> {
    let part = |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match repo.split_once('/') {
        Some((owner, name)) if part(owner) && part(name) => Ok(()),
        _ => Err(format!("github = \"{repo}\" is no repository — write it as `owner/repository`")),
    }
}

fn valid_ref(git_ref: &str) -> Result<(), String> {
    if git_ref.is_empty() || git_ref.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("ref = \"{git_ref}\" is no branch or tag name"));
    }
    Ok(())
}

/// A folder inside the repository: relative, without `.` or `..` segments.
fn valid_repo_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || path.starts_with('/') || trimmed.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(format!("path = \"{path}\" is no folder of the repository — write it relative to the repository's top, as `packs` or `catalog/packs`"));
    }
    Ok(trimmed.to_string())
}

/// One table checked: a valid name that is not the built-in one, and exactly one of
/// `github` and `dir`, with `ref` and `path` only beside `github`.
pub(crate) fn check_entry(entry: &PackSourceEntry) -> Result<Def, String> {
    valid_name(&entry.name)?;
    if entry.name == BUILTIN {
        return Err(format!("`{BUILTIN}` is the built-in source, the satz repository's packs/, and is not listed — give this source another name"));
    }
    let at = |e: String| format!("[[pack_source]] `{}`: {e}", entry.name);
    let location = match (&entry.github, &entry.dir) {
        (Some(repo), None) => {
            valid_repo(repo).map_err(at)?;
            if let Some(r) = &entry.git_ref {
                valid_ref(r).map_err(at)?;
            }
            let path = match &entry.path {
                Some(p) => valid_repo_path(p).map_err(at)?,
                None => DEFAULT_PATH.to_string(),
            };
            Location::Github { repo: repo.clone(), git_ref: entry.git_ref.clone(), path }
        }
        (None, Some(dir)) => {
            if entry.git_ref.is_some() || entry.path.is_some() {
                return Err(at("`ref` and `path` belong to a `github` source; a `dir` source is the folder itself".to_string()));
            }
            if dir.trim().is_empty() {
                return Err(at("dir = \"\" names no folder".to_string()));
            }
            Location::Dir(dir.clone())
        }
        (Some(_), Some(_)) => return Err(at("names both `github` and `dir` — a source is one of the two".to_string())),
        (None, None) => return Err(at("names neither `github` nor `dir` — a source is a GitHub repository or a folder".to_string())),
    };
    Ok(Def { name: entry.name.clone(), location })
}

/// The preset library that replaces satz's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Library {
    /// the `presets/` of the satz repository
    Satz,
    Github { repo: String, git_ref: Option<String> },
    /// the folder as config.toml writes it; the library is its `presets/`
    Dir(String),
}

fn check_library(entry: &PresetLibraryEntry) -> Result<Library, String> {
    let at = |e: String| format!("[preset_library]: {e}");
    match (&entry.github, &entry.dir) {
        (Some(repo), None) => {
            valid_repo(repo).map_err(at)?;
            if let Some(r) = &entry.git_ref {
                valid_ref(r).map_err(at)?;
            }
            Ok(Library::Github { repo: repo.clone(), git_ref: entry.git_ref.clone() })
        }
        (None, Some(dir)) => {
            if entry.git_ref.is_some() {
                return Err(at("`ref` belongs to a `github` library; a `dir` library is the folder itself".to_string()));
            }
            if dir.trim().is_empty() {
                return Err(at("dir = \"\" names no folder".to_string()));
            }
            Ok(Library::Dir(dir.clone()))
        }
        (Some(_), Some(_)) => Err(at("names both `github` and `dir` — the library is one of the two".to_string())),
        (None, None) => Err(at("names neither `github` nor `dir` — leave the table out to read satz's library".to_string())),
    }
}

/// Every table of the config checked, names unique. config.toml's parse runs it, so
/// a command never starts on a config whose sources are broken.
pub(crate) fn validate(config: &ToolConfig) -> Result<(), String> {
    defs(config)?;
    library(config)?;
    Ok(())
}

/// The configured sources, in the order config.toml lists them, without the built-in one.
pub(crate) fn defs(config: &ToolConfig) -> Result<Vec<Def>, String> {
    let mut out: Vec<Def> = Vec::new();
    for entry in &config.pack_source {
        let def = check_entry(entry)?;
        if out.iter().any(|d| d.name == def.name) {
            return Err(format!("[[pack_source]] `{}` is listed twice — a name is one source", def.name));
        }
        out.push(def);
    }
    Ok(out)
}

/// The built-in source, then every configured one.
pub(crate) fn all(config: &ToolConfig) -> Result<Vec<Def>, String> {
    let mut out = vec![Def::builtin()];
    out.extend(defs(config)?);
    Ok(out)
}

/// The source of that name: the built-in one, or one config.toml lists.
pub(crate) fn find(config: &ToolConfig, name: &str) -> Result<Def, String> {
    all(config)?.into_iter().find(|d| d.name == name).ok_or_else(|| {
        format!("no pack source `{name}` in config.toml — `satz pack-source list` lists the sources, `satz pack-source add {name} --github <owner>/<repository>` or `--dir <folder>` adds it")
    })
}

/// The preset library this estate reads.
pub(crate) fn library(config: &ToolConfig) -> Result<Library, String> {
    match &config.preset_library {
        None => Ok(Library::Satz),
        Some(entry) => check_library(entry),
    }
}

/// A folder as config.toml writes it, resolved against config.toml's own folder.
pub(crate) fn folder(config: &ToolConfig, written: &str) -> PathBuf {
    let p = Path::new(written);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match &config.dir {
        Some(dir) => dir.join(p),
        None => p.to_path_buf(),
    }
}

// ---------------------------------------------------------------------------
// pack-source list
// ---------------------------------------------------------------------------

/// One pack source, as `satz pack-source list --format json` prints it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SourceRow {
    pub name: String,
    /// true for `satz`, the satz repository's `packs/`, which config.toml does not list
    pub builtin: bool,
    /// `owner/repository` of a GitHub source
    pub github: Option<String>,
    /// the branch or tag a GitHub source is read at; none for its default branch
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// the folder of a GitHub source that holds its packs
    pub path: Option<String>,
    /// the folder of a folder source, as config.toml writes it
    pub dir: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SourcesReport {
    pub sources: Vec<SourceRow>,
}

pub(crate) fn list(config: &ToolConfig) -> Result<SourcesReport, String> {
    let sources = all(config)?
        .into_iter()
        .map(|d| {
            let builtin = d.is_builtin();
            match d.location {
                Location::Github { repo, git_ref, path } => SourceRow { name: d.name, builtin, github: Some(repo), git_ref, path: Some(path), dir: None },
                Location::Dir(dir) => SourceRow { name: d.name, builtin, github: None, git_ref: None, path: None, dir: Some(dir) },
            }
        })
        .collect();
    Ok(SourcesReport { sources })
}

pub(crate) fn render_list(r: &SourcesReport) -> String {
    let width = r.sources.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for s in &r.sources {
        let at = match (&s.github, &s.dir) {
            (Some(repo), _) => {
                let path = s.path.as_deref().unwrap_or(DEFAULT_PATH);
                match &s.git_ref {
                    Some(r) => format!("github.com/{repo}/{path} at {r}"),
                    None => format!("github.com/{repo}/{path} at its default branch"),
                }
            }
            (None, Some(dir)) => format!("the folder {dir}"),
            (None, None) => String::new(),
        };
        let builtin = if s.builtin { "  (built in)" } else { "" };
        out.push_str(&format!("{:width$}  {at}{builtin}\n", s.name));
    }
    out
}

// ---------------------------------------------------------------------------
// pack-source add / remove
// ---------------------------------------------------------------------------

/// The estate's `config.toml`, as text that keeps its comments and its layout.
fn document(config_file: &Path) -> Result<toml_edit::DocumentMut, String> {
    if !config_file.exists() {
        return Err(format!("{}: no config.toml — run `satz pack-source` from an estate, or pass --config", config_file.display()));
    }
    let text = crate::fsx::read_to_string(config_file).map_err(|e| format!("{}: {}", config_file.display(), e))?;
    text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{}: not valid TOML ({})", config_file.display(), e))
}

/// Write the edited document, after proving that config.toml reads it.
fn write_document(config_file: &Path, doc: &toml_edit::DocumentMut) -> Result<(), String> {
    let text = doc.to_string();
    let parsed: ToolConfig = toml::from_str(&text).map_err(|e| crate::settings::describe_toml_error(config_file, &text, &e))?;
    validate(&parsed).map_err(|e| format!("{}: {}", config_file.display(), e))?;
    crate::fsx::write(config_file, text).map_err(|e| format!("{}: {}", config_file.display(), e))
}

/// `satz pack-source add`: one more `[[pack_source]]` table. A folder source must be a
/// folder; a GitHub source is not contacted.
pub(crate) fn add(config_file: &Path, config: &ToolConfig, entry: PackSourceEntry) -> Result<Def, String> {
    let def = check_entry(&entry)?;
    if defs(config)?.iter().any(|d| d.name == def.name) {
        return Err(format!("{}: the pack source `{}` is there already — `satz pack-source remove {}` first to change it", config_file.display(), def.name, def.name));
    }
    if let Location::Dir(dir) = &def.location {
        let at = folder(config, dir);
        if !at.is_dir() {
            return Err(format!("--dir {dir}: no such folder ({}) — a folder is written relative to config.toml", at.display()));
        }
    }
    let mut doc = document(config_file)?;
    if !doc.contains_key("pack_source") {
        doc["pack_source"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let rows = doc["pack_source"]
        .as_array_of_tables_mut()
        .ok_or_else(|| format!("{}: `pack_source` is not a list of [[pack_source]] tables", config_file.display()))?;
    let mut table = toml_edit::Table::new();
    table["name"] = toml_edit::value(entry.name.as_str());
    for (key, value) in [("github", &entry.github), ("ref", &entry.git_ref), ("path", &entry.path), ("dir", &entry.dir)] {
        if let Some(v) = value {
            table[key] = toml_edit::value(v.as_str());
        }
    }
    rows.push(table);
    write_document(config_file, &doc)?;
    Ok(def)
}

/// `satz pack-source remove`: the table goes. Refused while an origin record names the
/// source, since those rows could not be followed any more.
pub(crate) fn remove(config_file: &Path, config: &ToolConfig, name: &str) -> Result<(), String> {
    valid_name(name)?;
    if name == BUILTIN {
        return Err(format!("`{BUILTIN}` is the built-in source, which config.toml does not list — nothing to remove"));
    }
    if !defs(config)?.iter().any(|d| d.name == name) {
        return Err(format!("{}: no pack source `{name}` — `satz pack-source list` lists them", config_file.display()));
    }
    let using = crate::workload_packs::recorded_from(config, name)?;
    if !using.is_empty() {
        return Err(format!(
            "the origin record of {} names `{name}` for {} — remove those rows with `satz detach-pack <id>` first, or keep the source",
            config.packs_dir,
            using.join(", ")
        ));
    }
    let mut doc = document(config_file)?;
    let rows = doc["pack_source"]
        .as_array_of_tables_mut()
        .ok_or_else(|| format!("{}: `pack_source` is not a list of [[pack_source]] tables", config_file.display()))?;
    rows.retain(|t| t.get("name").and_then(|v| v.as_str()) != Some(name));
    if rows.is_empty() {
        doc.remove("pack_source");
    }
    write_document(config_file, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ToolConfig, String> {
        let config: ToolConfig = toml::from_str(text).map_err(|e| e.to_string())?;
        validate(&config)?;
        Ok(config)
    }

    #[test]
    fn a_source_is_a_named_repository_or_folder() {
        let config = parse(
            "yaml_dir = \"satz\"\n\n[[pack_source]]\nname = \"official\"\ngithub = \"acme/packs\"\nref = \"v2\"\npath = \"catalog/packs/\"\n\n\
             [[pack_source]]\nname = \"shared\"\ndir = \"../shared-packs\"\n",
        )
        .unwrap();
        let defs = all(&config).unwrap();
        assert_eq!(defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["satz", "official", "shared"]);
        assert_eq!(defs[1].location, Location::Github { repo: "acme/packs".into(), git_ref: Some("v2".into()), path: "catalog/packs".into() });
        assert_eq!(defs[2].location, Location::Dir("../shared-packs".into()));
        // `path` defaults to `packs`, `ref` to the default branch
        let plain = parse("[[pack_source]]\nname = \"a\"\ngithub = \"acme/packs\"\n").unwrap();
        assert_eq!(defs_of(&plain)[0].location, Location::Github { repo: "acme/packs".into(), git_ref: None, path: "packs".into() });
        assert_eq!(find(&plain, "satz").unwrap(), Def::builtin());
        assert!(find(&plain, "other").unwrap_err().contains("satz pack-source add other"));
    }

    fn defs_of(c: &ToolConfig) -> Vec<Def> {
        defs(c).unwrap()
    }

    #[test]
    fn a_broken_source_is_refused_by_what_is_wrong() {
        let refused = |text: &str, says: &str| {
            let err = parse(text).unwrap_err();
            assert!(err.contains(says), "{text}\n→ {err}");
        };
        refused("[[pack_source]]\nname = \"a\"\ndir = \"x\"\n[[pack_source]]\nname = \"a\"\ndir = \"y\"\n", "listed twice");
        refused("[[pack_source]]\nname = \"a\"\ngithub = \"acme/packs\"\ndir = \"x\"\n", "both `github` and `dir`");
        refused("[[pack_source]]\nname = \"a\"\n", "neither `github` nor `dir`");
        refused("[[pack_source]]\nname = \"Official\"\ndir = \"x\"\n", "no pack source name");
        refused("[[pack_source]]\nname = \"1st\"\ndir = \"x\"\n", "no pack source name");
        refused("[[pack_source]]\nname = \"a_b\"\ndir = \"x\"\n", "no pack source name");
        refused("[[pack_source]]\nname = \"satz\"\ndir = \"x\"\n", "built-in source");
        refused("[[pack_source]]\nname = \"a\"\ngithub = \"acme\"\n", "owner/repository");
        refused("[[pack_source]]\nname = \"a\"\ndir = \"x\"\nref = \"main\"\n", "belong to a `github` source");
        refused("[[pack_source]]\nname = \"a\"\ngithub = \"acme/packs\"\npath = \"../up\"\n", "no folder of the repository");
        refused("[[pack_source]]\nname = \"a\"\ndir = \"x\"\nbranch = \"main\"\n", "unknown field `branch`");
        refused("[preset_library]\ngithub = \"acme/lib\"\ndir = \"x\"\n", "[preset_library]: names both");
        refused("[preset_library]\n", "[preset_library]: names neither");
        refused("[preset_library]\ndir = \"x\"\nref = \"v1\"\n", "belongs to a `github` library");
    }

    #[test]
    fn the_preset_library_is_satzs_unless_the_table_names_another() {
        assert_eq!(library(&parse("yaml_dir = \"satz\"\n").unwrap()).unwrap(), Library::Satz);
        assert_eq!(library(&parse("[preset_library]\ngithub = \"acme/lib\"\nref = \"v1\"\n").unwrap()).unwrap(), Library::Github { repo: "acme/lib".into(), git_ref: Some("v1".into()) });
        assert_eq!(library(&parse("[preset_library]\ndir = \"../lib\"\n").unwrap()).unwrap(), Library::Dir("../lib".into()));
    }

    #[test]
    fn add_list_and_remove_keep_the_rest_of_config_toml() {
        let dir = std::env::temp_dir().join(format!("satz-pack-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("up")).unwrap();
        let file = dir.join("config.toml");
        let original = "# the estate's settings\nyaml_dir = \"satz\"  # where the estate is\npacks_dir = \"packs\"\n\n[[silence]]\nkind = \"action\"\nreason = \"seen\"\n";
        std::fs::write(&file, original).unwrap();
        let load = || {
            let c = crate::settings::parse_tool_config(&file).unwrap();
            crate::settings::resolved_config(&c, &dir)
        };

        let entry = |name: &str, github: Option<&str>, dir: Option<&str>| PackSourceEntry { name: name.into(), github: github.map(Into::into), git_ref: None, path: None, dir: dir.map(Into::into) };
        add(&file, &load(), entry("local", None, Some("up"))).unwrap();
        add(&file, &load(), PackSourceEntry { git_ref: Some("v1".into()), ..entry("official", Some("acme/packs"), None) }).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# the estate's settings\nyaml_dir = \"satz\"  # where the estate is\n"), "{text}");
        assert!(text.contains("[[silence]]\nkind = \"action\""), "{text}");
        assert!(text.contains("[[pack_source]]\nname = \"local\"\ndir = \"up\"\n"), "{text}");
        assert!(text.contains("name = \"official\"\ngithub = \"acme/packs\"\nref = \"v1\"\n"), "{text}");

        let listed = list(&load()).unwrap();
        assert_eq!(listed.sources.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["satz", "local", "official"]);
        assert!(listed.sources[0].builtin && !listed.sources[1].builtin);

        assert!(add(&file, &load(), entry("local", None, Some("up"))).unwrap_err().contains("there already"));
        assert!(add(&file, &load(), entry("gone", None, Some("nowhere"))).unwrap_err().contains("no such folder"));
        assert!(remove(&file, &load(), "satz").unwrap_err().contains("built-in"));
        assert!(remove(&file, &load(), "other").unwrap_err().contains("no pack source `other`"));

        // a source an origin record names stays
        std::fs::create_dir_all(dir.join("packs")).unwrap();
        std::fs::write(dir.join("packs/demo.satz"), "pack demo version \"1.0\"\n").unwrap();
        std::fs::write(
            dir.join("packs/origin.toml"),
            "[[pack]]\nid = \"demo\"\nsource = \"local\"\ndir = \"up\"\nversion = \"1.0\"\nsha256 = \"x\"\n",
        )
        .unwrap();
        let err = remove(&file, &load(), "local").unwrap_err();
        assert!(err.contains("demo") && err.contains("detach-pack"), "{err}");
        std::fs::remove_file(dir.join("packs/origin.toml")).unwrap();

        remove(&file, &load(), "local").unwrap();
        remove(&file, &load(), "official").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim_end(), original.trim_end());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
