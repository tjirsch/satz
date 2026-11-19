//! Workload packs: the packs a workload estate takes one at a time into its
//! `packs_dir`, beside the packs it wrote itself (ADR 0083).
//!
//! A workload pack is one `.satz` file with a `pack <id> version "…"` header. Its id
//! is its path under the folder of the pack source it comes from, without the
//! extension. What decides whose a file in `packs_dir` is, is the origin record
//! `origin.toml` there: a pack with a row was fetched, a pack without one is the
//! estate's own and no command here writes it. A row names its pack source, which
//! `config.toml` defines (ADR 0097), so every clone of the repository can follow it.

use std::path::{Path, PathBuf};

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::fsx;
use crate::pack_sources::{Def, Location, BUILTIN};
use crate::settings::ToolConfig;

/// The origin record's file name, inside `packs_dir`.
pub(crate) const RECORD: &str = "origin.toml";

const RECORD_HEADER: &str = "# Written by `satz get-pack` and `satz update-pack`: one row per pack fetched into this folder.\n\
                             # A pack with a row is updated by `satz update-pack` from the pack source it names, which\n\
                             # config.toml defines; a pack without one is this repository's own and is never written.\n\
                             # `satz detach-pack <id>` removes a row.\n\n";

/// One fetched pack, as the origin record holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// the pack's path under its source's folder, without `.satz`
    pub id: String,
    /// the name of the pack source it was fetched from: `satz`, or a `[[pack_source]]` of config.toml
    pub source: String,
    /// for a GitHub source: the repository, `owner/repository`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// for a GitHub source whose packs are not under `packs/`: the folder that holds them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// for a GitHub source: the commit the file was read at
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// for a folder source: the folder, as config.toml writes it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// the version the pack stated when it was fetched
    pub version: String,
    /// SHA-256 of the file as it was fetched
    pub sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    #[serde(default, rename = "pack")]
    packs: Vec<Row>,
}

fn record_path(packs_dir: &Path) -> PathBuf {
    packs_dir.join(RECORD)
}

/// The origin record of `packs_dir`: no file is no row. A file that does not parse, a
/// row that repeats an id, a row whose id is no pack id and a row in a form this
/// release does not write are errors naming the file.
fn read_record(packs_dir: &Path) -> Result<Vec<Row>, String> {
    let path = record_path(packs_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let record: Record = toml::from_str(&text).map_err(|e| format!("{}: not a valid origin record — {}", path.display(), e.message()))?;
    let mut seen = std::collections::BTreeSet::new();
    for row in &record.packs {
        valid_id(&row.id).map_err(|e| format!("{}: {}", path.display(), e))?;
        if !seen.insert(row.id.as_str()) {
            return Err(format!("{}: the pack `{}` has two rows", path.display(), row.id));
        }
        check_row(row).map_err(|e| format!("{}: {}", path.display(), e))?;
    }
    Ok(record.packs)
}

/// A row in the form this release writes: a source name, and a repository with a commit
/// or a folder. The two values an earlier release wrote, which named neither, are refused
/// with the edit.
fn check_row(row: &Row) -> Result<(), String> {
    let id = &row.id;
    if row.source == "dir" {
        return Err(format!(
            "the pack `{id}` records `source = \"dir\"`, which names no folder — add the folder it was fetched from as a pack source (`satz pack-source add <name> --dir <folder>`), then write `source = \"<name>\"` and `dir = \"<folder>\"`, the folder as config.toml writes it, in its row"
        ));
    }
    if row.source.starts_with("github") {
        return Err(format!(
            "the pack `{id}` records `source = \"{}\"`, which names no commit — write `source = \"{BUILTIN}\"`, `github = \"{}\"` and `commit = \"<sha>\"` in its row, the commit of the satz repository the file was read at: of the commits `git log --format=%H -- packs/{id}.satz` lists in a satz checkout, the one for which `git show <commit>:packs/{id}.satz | shasum -a 256` prints the row's sha256",
            row.source,
            crate::github::REPO
        ));
    }
    crate::pack_sources::valid_name(&row.source).map_err(|e| format!("the pack `{id}`: {e}"))?;
    let github = row.github.is_some() || row.path.is_some() || row.commit.is_some();
    match (github, &row.dir) {
        (true, Some(_)) => Err(format!("the pack `{id}` records both a repository and `dir` — a GitHub source records `github` and `commit`, a folder source `dir`")),
        (false, None) => Err(format!("the pack `{id}` records neither a repository nor `dir` — a GitHub source records `github` and the `commit` the file was read at, a folder source `dir`")),
        (false, Some(_)) => Ok(()),
        (true, None) => match (&row.github, &row.commit) {
            (None, _) => Err(format!("the pack `{id}` records a commit and no `github` — write the repository it was read from, `github = \"<owner>/<repository>\"`, in its row")),
            (Some(_), None) => Err(format!("the pack `{id}` records `github` and no `commit` — a GitHub source records the commit the file was read at")),
            (Some(_), Some(c)) if c.len() == 40 && c.chars().all(|x| x.is_ascii_hexdigit()) => Ok(()),
            (Some(_), Some(c)) => Err(format!("the pack `{id}` records commit = \"{c}\", which is no commit — a commit is 40 hexadecimal digits")),
        },
    }
}

/// Each row's source as config.toml defines it now, and the row in the form that source
/// writes: the same repository and folder of it for a GitHub source, the same folder for a
/// folder.
fn check_rows(runtime: &ToolConfig, rows: &[Row]) -> Result<(), String> {
    let record = record_path(Path::new(&runtime.packs_dir));
    for row in rows {
        let def = crate::pack_sources::find(runtime, &row.source).map_err(|_| {
            format!(
                "{}: the pack `{}` was fetched from the pack source `{}`, which config.toml does not name — add it with `satz pack-source add {} --github <owner>/<repository>` or `--dir <folder>`, or remove the row with `satz detach-pack {}`",
                record.display(),
                row.id,
                row.source,
                row.source,
                row.id
            )
        })?;
        follows(&def, row).map_err(|e| format!("{}: {e}", record.display()))?;
    }
    Ok(())
}

fn follows(def: &Def, row: &Row) -> Result<(), String> {
    let (id, name) = (&row.id, &def.name);
    match (&def.location, &row.dir) {
        (Location::Github { repo, path, .. }, None) => {
            let recorded = (row.github.as_deref().unwrap_or_default(), row.path.as_deref().unwrap_or(crate::pack_sources::DEFAULT_PATH));
            if recorded == (repo.as_str(), path.as_str()) {
                return Ok(());
            }
            let (was, now) = (format!("github.com/{}/{}", recorded.0, recorded.1), format!("github.com/{repo}/{path}"));
            let edit = if path == crate::pack_sources::DEFAULT_PATH {
                format!("github = \"{repo}\" and no `path`")
            } else {
                format!("github = \"{repo}\" and path = \"{path}\"")
            };
            Err(format!(
                "the pack `{id}` was fetched from {was} as `{name}`, and config.toml names {now} for it — if {now} holds the same packs, write {edit} in the row; if not, add {was} as a pack source of its own and name that source in the row"
            ))
        }
        (Location::Github { .. }, Some(_)) => Err(format!("the pack `{id}` records a folder, and `{name}` is a GitHub repository in config.toml — a row of a GitHub source records `github` and `commit`")),
        (Location::Dir(_), None) => Err(format!("the pack `{id}` records a repository, and `{name}` is a folder in config.toml — a row of a folder source records `dir`")),
        (Location::Dir(written), Some(dir)) if dir == written => Ok(()),
        (Location::Dir(written), Some(dir)) => Err(format!(
            "the pack `{id}` was fetched from the folder `{dir}` of `{name}`, and config.toml names `{written}` for it — if `{written}` holds the same packs, write dir = \"{written}\" in the row; if not, add `{dir}` as a pack source of its own and name that source in the row"
        )),
    }
}

/// The ids of the packs the origin record says came from the source `name`.
pub(crate) fn recorded_from(runtime: &ToolConfig, name: &str) -> Result<Vec<String>, String> {
    Ok(read_record(Path::new(&runtime.packs_dir))?.into_iter().filter(|r| r.source == name).map(|r| r.id).collect())
}

/// Write the record, rows in id order. A record without rows is no file.
fn write_record(packs_dir: &Path, mut rows: Vec<Row>) -> Result<(), String> {
    let path = record_path(packs_dir);
    if rows.is_empty() {
        if path.exists() {
            fsx::remove_file(&path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    let body = toml::to_string(&Record { packs: rows }).map_err(|e| format!("{}: {}", path.display(), e))?;
    fsx::create_dir_all(packs_dir).map_err(|e| e.to_string())?;
    fsx::write(&path, format!("{RECORD_HEADER}{body}")).map_err(|e| e.to_string())
}

/// A pack id: path segments of letters, digits, `_`, `-` and `.`, separated by `/`,
/// none of them `.` or `..`, and no `.satz` at the end.
pub(crate) fn valid_id(id: &str) -> Result<(), String> {
    let segment_ok = |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if id.ends_with(".satz") {
        return Err(format!("`{id}` is a file name — a pack is named by its id, `{}`", id.trim_end_matches(".satz")));
    }
    if id.is_empty() || !id.split('/').all(segment_ok) {
        return Err(format!("`{id}` is no pack id — an id is the pack's path under its source's folder without `.satz`: letters, digits, `_`, `-` and `.`, in segments separated by `/`"));
    }
    Ok(())
}

fn pack_file(packs_dir: &Path, id: &str) -> PathBuf {
    packs_dir.join(format!("{id}.satz"))
}

fn sha256(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// A pack source this run reads: a GitHub repository at the commit its branch or tag
/// points at now, or a folder.
pub(crate) struct Source {
    def: Def,
    place: Place,
}

enum Place {
    Github { repo: crate::github::Repo, path: String, commit: String, http: reqwest::Client },
    Dir { written: String, folder: PathBuf },
}

/// A pack source as a report names what was read.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub(crate) struct SourceRead {
    /// the source's name: `satz`, or a `[[pack_source]]` of config.toml
    pub name: String,
    /// where it is: `github.com/<owner>/<repository>/<folder>`, with `@<ref>` where config.toml names one, or the folder as config.toml writes it
    pub location: String,
    /// for a GitHub source: the commit read
    pub commit: Option<String>,
}

impl Source {
    /// The source ready to read: a GitHub source's branch or tag resolved to its commit
    /// (one API request), a folder source's folder found.
    pub(crate) async fn open(def: Def, runtime: &ToolConfig) -> Result<Source, String> {
        let place = match &def.location {
            Location::Github { repo, git_ref, path } => {
                let repo = if def.is_builtin() { crate::github::Repo::satz() } else { crate::github::Repo::configured(repo, git_ref.as_deref()) };
                let http = crate::github::client("satz-packs").map_err(|e| e.to_string())?;
                let commit = repo.commit(&http, crate::github::PACKS_HINT).await.map_err(|e| format!("pack source `{}`: {e}", def.name))?;
                Place::Github { repo, path: path.clone(), commit, http }
            }
            Location::Dir(written) => {
                let folder = crate::pack_sources::folder(runtime, written);
                if !folder.is_dir() {
                    return Err(format!(
                        "pack source `{}`: dir = \"{written}\" is no folder ({}) — correct it in config.toml, or remove the source with `satz pack-source remove {}`",
                        def.name,
                        folder.display(),
                        def.name
                    ));
                }
                Place::Dir { written: written.clone(), folder }
            }
        };
        Ok(Source { def, place })
    }

    pub(crate) fn name(&self) -> &str {
        &self.def.name
    }

    fn describe(&self) -> String {
        match &self.place {
            Place::Github { commit, .. } => format!("`{}` ({} at {})", self.def.name, self.def.describe(), &commit[..12]),
            Place::Dir { written, .. } => format!("`{}` ({written})", self.def.name),
        }
    }

    pub(crate) fn read(&self) -> SourceRead {
        SourceRead {
            name: self.def.name.clone(),
            location: self.def.describe(),
            commit: match &self.place {
                Place::Github { commit, .. } => Some(commit.clone()),
                Place::Dir { .. } => None,
            },
        }
    }

    /// A row for a pack this source just gave: its name, and its repository, the folder of
    /// it where that is not `packs`, and the commit — or its folder.
    fn row(&self, id: &str, version: &str, text: &str) -> Row {
        let mut row = Row { id: id.to_string(), source: self.def.name.clone(), github: None, path: None, commit: None, dir: None, version: version.to_string(), sha256: sha256(text) };
        match &self.place {
            Place::Github { repo, path, commit, .. } => {
                row.github = Some(repo.name.clone());
                row.path = (path != crate::pack_sources::DEFAULT_PATH).then(|| path.clone());
                row.commit = Some(commit.clone());
            }
            Place::Dir { written, .. } => row.dir = Some(written.clone()),
        }
        row
    }

    async fn fetch(&self, id: &str) -> Result<Option<String>, String> {
        match &self.place {
            Place::Github { repo, path, commit, http } => {
                let Some(bytes) = repo.file(http, commit, &format!("{path}/{id}.satz")).await? else { return Ok(None) };
                String::from_utf8(bytes).map(Some).map_err(|_| format!("{}: {path}/{id}.satz is not UTF-8 text", self.describe()))
            }
            Place::Dir { folder, .. } => {
                let path = pack_file(folder, id);
                if !path.is_file() {
                    return Ok(None);
                }
                fsx::read_to_string(&path).map(Some).map_err(|e| format!("{}: {}", path.display(), e))
            }
        }
    }

    async fn list(&self) -> Result<Vec<String>, String> {
        match &self.place {
            Place::Github { repo, path, commit, http } => {
                let under = format!("{path}/");
                let mut ids: Vec<String> = repo
                    .files(http, commit, path, crate::github::PACKS_HINT)
                    .await?
                    .into_iter()
                    .filter_map(|f| f.strip_prefix(&under).and_then(|p| p.strip_suffix(".satz")).map(str::to_string))
                    .collect();
                ids.sort();
                Ok(ids)
            }
            Place::Dir { folder, .. } => local_ids(folder),
        }
    }
}

/// The id of every `.satz` file under `dir`, sorted. A folder that does not exist
/// holds none.
fn local_ids(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "satz") {
                let rel = path.strip_prefix(dir).map_err(|e| e.to_string())?.with_extension("");
                ids.push(fsx::slash(&rel));
            }
        }
    }
    ids.sort();
    Ok(ids)
}

/// The sources a run reads, each opened once, in the order they were first needed.
#[derive(Default)]
struct Opened(Vec<Source>);

impl Opened {
    async fn get(&mut self, runtime: &ToolConfig, name: &str) -> Result<&Source, String> {
        if let Some(at) = self.0.iter().position(|s| s.name() == name) {
            return Ok(&self.0[at]);
        }
        let source = Source::open(crate::pack_sources::find(runtime, name)?, runtime).await?;
        self.0.push(source);
        Ok(self.0.last().expect("just pushed"))
    }

    fn read(&self) -> Vec<SourceRead> {
        self.0.iter().map(Source::read).collect()
    }
}

/// What the source says about one pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Upstream<'a> {
    /// the source was not asked: `--source` names another one than the row
    NotAsked,
    /// the source holds no such pack
    Missing,
    Found(&'a str),
}

/// How a pack in `packs_dir` stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    /// no row: the repository's own pack, which no command writes
    Own,
    /// fetched, unchanged since, and the source holds the same file
    Current,
    /// fetched, unchanged since, and the source holds another file
    Behind,
    /// fetched and changed since: never overwritten without `--take-upstream`
    Edited,
    /// fetched, and the source holds it no longer
    Gone,
    /// fetched from another source than the one `--source` names
    NotChecked,
    /// the source holds it and `packs_dir` does not
    Available,
}

/// The state of one pack from its row, its local text and what the source says. A row
/// without a local file is the caller's error, not a state.
pub(crate) fn classify(row: Option<&Row>, local: Option<&str>, upstream: Upstream<'_>) -> Option<State> {
    match (row, local) {
        (None, Some(_)) => Some(State::Own),
        (None, None) => matches!(upstream, Upstream::Found(_)).then_some(State::Available),
        (Some(_), None) => None,
        (Some(row), Some(local)) => Some(if sha256(local) != row.sha256 {
            State::Edited
        } else {
            match upstream {
                Upstream::NotAsked => State::NotChecked,
                Upstream::Missing => State::Gone,
                Upstream::Found(u) if sha256(u) == row.sha256 => State::Current,
                Upstream::Found(_) => State::Behind,
            }
        }),
    }
}

/// The local text of a recorded pack. A row whose file is gone is an error: the
/// record and the folder disagree, and nothing here decides which is right.
fn recorded_text(packs_dir: &Path, row: &Row) -> Result<String, String> {
    let path = pack_file(packs_dir, &row.id);
    if !path.is_file() {
        return Err(format!(
            "{} records the pack `{}`, and {} is not there — restore the file, or remove the row with `satz detach-pack {}`",
            record_path(packs_dir).display(),
            row.id,
            path.display(),
            row.id
        ));
    }
    fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))
}

/// The version a fetched file states. A file without one is no pack.
fn version_of(id: &str, source: &Source, text: &str) -> Result<String, String> {
    crate::presets::pack_version(text)
        .ok_or_else(|| format!("{}: the file for `{id}` carries no `pack <id> version \"…\"` statement, so it is no pack", source.describe()))
}

/// A path as the file system resolves it, for a folder that may not exist yet: the
/// longest part that exists is canonicalized and the rest is appended, so a
/// `packs_dir` no pack was fetched into yet compares with the folders that are there.
fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut rest = Vec::new();
    let mut at = absolute.as_path();
    loop {
        if let Ok(real) = fsx::canonicalize(at) {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return absolute,
        }
    }
}

/// The `use` path that reaches a pack of `packs_dir` from the estate: its path under
/// the first of `include_dirs` that holds it. `use` resolves against those folders,
/// so a `packs_dir` under none of them holds packs no estate can use.
pub(crate) fn use_path(runtime: &ToolConfig, id: &str) -> Result<String, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let file = resolved(packs_dir).join(format!("{id}.satz"));
    runtime
        .include_dirs
        .iter()
        .filter_map(|d| file.strip_prefix(resolved(Path::new(d))).ok().map(fsx::slash))
        .min_by_key(String::len)
        .ok_or_else(|| {
            format!(
                "packs_dir {} is under none of the config's include_dirs ({}), so no `use` line reaches a pack in it — add it, or a folder above it, to `include_dirs` in config.toml",
                packs_dir.display(),
                runtime.include_dirs.join(", ")
            )
        })
}

// ---------------------------------------------------------------------------
// get-pack
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct Installed {
    pub id: String,
    pub version: String,
    /// the file written
    pub path: String,
    /// the line an estate reads the pack with
    pub use_line: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct GetReport {
    /// the name of the pack source the packs came from
    pub source: String,
    /// where that source is: `github.com/<owner>/<repository>/<folder>`, with `@<ref>` where config.toml names one, or the folder as config.toml writes it
    pub location: String,
    /// for a GitHub source: the commit the packs were read at
    pub commit: Option<String>,
    pub installed: Vec<Installed>,
    /// with `all`: packs the source holds that `packs_dir` already has, left as they are
    pub present: Vec<String>,
}

/// Which packs a command is about.
#[derive(Debug, Clone)]
pub(crate) enum Which {
    One(String),
    All,
}

impl Which {
    /// One id or `--all`, exactly one of the two.
    pub(crate) fn new(id: Option<String>, all: bool, command: &str) -> Result<Self, String> {
        match (id, all) {
            (Some(id), false) => {
                valid_id(&id)?;
                Ok(Which::One(id))
            }
            (None, true) => Ok(Which::All),
            (Some(_), true) => Err(format!("{command}: name one pack or pass --all, not both")),
            (None, false) => Err(format!("{command}: name the pack, or pass --all — `satz packs` lists them")),
        }
    }
}

/// Fetch one pack, or every pack the source holds that `packs_dir` lacks, from the
/// source `source` names — `satz` without one — and record where each came from. A pack
/// `packs_dir` already holds is never written: one named is refused, and with `all` it is
/// listed as present.
pub(crate) async fn get(runtime: &ToolConfig, source: Option<&str>, which: &Which) -> Result<GetReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    check_rows(runtime, &rows)?;
    let def = crate::pack_sources::find(runtime, source.unwrap_or(BUILTIN))?;

    if let Which::One(id) = which {
        let file = pack_file(packs_dir, id);
        if file.exists() {
            let how = if rows.iter().any(|r| &r.id == id) {
                format!("it was fetched before — `satz update-pack {id}` brings it up to date")
            } else {
                "it has no recorded origin, so it is this repository's own pack and is not overwritten".to_string()
            };
            return Err(format!("{} is already there: {how}", file.display()));
        }
    }
    let source = Source::open(def, runtime).await?;
    let read = source.read();
    let mut report = GetReport { source: read.name, location: read.location, commit: read.commit, installed: Vec::new(), present: Vec::new() };

    let ids = match which {
        Which::One(id) => vec![id.clone()],
        Which::All => {
            let mut wanted = Vec::new();
            for id in source.list().await? {
                if pack_file(packs_dir, &id).exists() {
                    report.present.push(id);
                } else {
                    wanted.push(id);
                }
            }
            wanted
        }
    };

    for id in ids {
        valid_id(&id)?;
        let use_line = format!("use \"{}\"", use_path(runtime, &id)?);
        let Some(text) = source.fetch(&id).await? else {
            return Err(format!("{} holds no pack `{id}` — `satz packs --source {}` lists the packs it holds", source.describe(), source.name()));
        };
        let version = version_of(&id, &source, &text)?;
        let file = pack_file(packs_dir, &id);
        if let Some(parent) = file.parent() {
            fsx::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fsx::write_verbatim(&file, text.as_bytes()).map_err(|e| e.to_string())?;
        rows.retain(|r| r.id != id);
        rows.push(source.row(&id, &version, &text));
        // the record follows every file as it lands: a later pack that fails leaves
        // the earlier ones recorded
        write_record(packs_dir, rows.clone())?;
        report.installed.push(Installed { id, version, path: file.display().to_string(), use_line });
    }
    Ok(report)
}

pub(crate) fn render_get(r: &GetReport) -> String {
    let from = match &r.commit {
        Some(c) => format!("`{}` ({} at {})", r.source, r.location, &c[..12.min(c.len())]),
        None => format!("`{}` ({})", r.source, r.location),
    };
    let mut out = String::new();
    for p in &r.installed {
        out.push_str(&format!("installed {} {} from {from} → {}\n    {}\n", p.id, p.version, p.path, p.use_line));
    }
    if !r.present.is_empty() {
        out.push_str(&format!("already there, left as they are: {}\n", r.present.join(", ")));
    }
    if r.installed.is_empty() {
        out.push_str(&format!("nothing to install: every pack of {from} is already there\n"));
    }
    out
}

// ---------------------------------------------------------------------------
// update-pack
// ---------------------------------------------------------------------------

/// What an update did with one recorded pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Action {
    /// the source holds the same file
    Current,
    /// overwritten with the source's file
    Updated,
    /// `report_only`: the source holds another file, and an update would write it
    WouldUpdate,
    /// changed here since it was fetched: left as it is
    Edited,
    /// `take_upstream`: changed here, and overwritten with the source's file
    TookUpstream,
    /// the source holds it no longer: left as it is
    Gone,
    /// fetched from another source than the one `--source` names: not looked at
    NotChecked,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct UpdateRow {
    pub id: String,
    /// the name of the pack source its row records
    pub source: String,
    pub action: Action,
    /// the version the local file states
    pub version: String,
    /// the version the source's file states, where the source was read and holds it
    pub upstream_version: Option<String>,
    /// for an edited pack: the local file against the source's, as a unified diff
    pub diff: Option<String>,
    /// what to do about it, where something is to be done
    pub note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct UpdateReport {
    /// the pack sources read, each once
    pub sources: Vec<SourceRead>,
    pub packs: Vec<UpdateRow>,
    /// a pack is edited or gone: the command exits 1
    pub attention: bool,
}

/// Bring one recorded pack, or every recorded pack, up to date with the source its row
/// names — only the packs of `only`, where it names one. A pack unchanged since it was
/// fetched is overwritten when the source holds another file; a pack changed here is
/// left alone unless `take_upstream`; a pack without a row is the repository's own and
/// is not looked at.
pub(crate) async fn update(runtime: &ToolConfig, only: Option<&str>, which: &Which, report_only: bool, take_upstream: bool) -> Result<UpdateReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    check_rows(runtime, &rows)?;
    if let Some(name) = only {
        crate::pack_sources::find(runtime, name)?;
    }

    let targets: Vec<Row> = match which {
        Which::One(id) => {
            let Some(row) = rows.iter().find(|r| &r.id == id) else {
                let own = pack_file(packs_dir, id).exists();
                return Err(if own {
                    format!("`{id}` has no recorded origin: it is this repository's own pack, and no update writes it")
                } else {
                    format!("`{id}` is not in {} — `satz get-pack {id}` fetches it", packs_dir.display())
                });
            };
            if let Some(name) = only.filter(|n| *n != row.source) {
                return Err(format!("`{id}` was fetched from `{}`, and this run reads only `{name}` — leave `--source` out, or pass `--source {}`", row.source, row.source));
            }
            vec![row.clone()]
        }
        Which::All => rows.clone(),
    };

    let mut opened = Opened::default();
    let mut packs = Vec::new();
    let mut attention = false;
    for row in targets {
        let local = recorded_text(packs_dir, &row)?;
        let version = crate::presets::pack_version(&local).unwrap_or_else(|| row.version.clone());
        let mut out = UpdateRow { id: row.id.clone(), source: row.source.clone(), action: Action::Current, version, upstream_version: None, diff: None, note: None };
        if let Some(name) = only.filter(|n| *n != row.source) {
            out.action = Action::NotChecked;
            out.note = Some(format!("fetched from `{}`, and this run reads only `{name}` — leave `--source` out to read the source of every pack", row.source));
            if sha256(&local) != row.sha256 {
                out.action = Action::Edited;
                out.note = Some(format!("changed here since it was fetched; it came from `{}`, which this run does not read", row.source));
                attention = true;
            }
            packs.push(out);
            continue;
        }
        let source = opened.get(runtime, &row.source).await?;
        let fetched = source.fetch(&row.id).await?;
        let upstream = match &fetched {
            None => Upstream::Missing,
            Some(text) => Upstream::Found(text),
        };
        let upstream_version = match upstream {
            Upstream::Found(text) => Some(version_of(&row.id, source, text)?),
            _ => None,
        };
        out.upstream_version = upstream_version.clone();
        let state = classify(Some(&row), Some(&local), upstream).expect("a recorded pack with a local file has a state");

        // the one write: the source's file over the local one, and the row after it
        let take = |text: &str, rows: &mut Vec<Row>| -> Result<(), String> {
            fsx::write_verbatim(pack_file(packs_dir, &row.id), text.as_bytes()).map_err(|e| e.to_string())?;
            let fresh = source.row(&row.id, upstream_version.as_deref().unwrap_or_default(), text);
            for r in rows.iter_mut().filter(|r| r.id == row.id) {
                *r = fresh.clone();
            }
            write_record(packs_dir, rows.clone())
        };

        match (state, upstream) {
            (State::Current, _) => {}
            (State::Gone, _) => {
                out.action = Action::Gone;
                out.note = Some(format!("{} holds it no longer — keep it as this repository's own with `satz detach-pack {}`", source.describe(), row.id));
                attention = true;
            }
            (State::Behind, Upstream::Found(text)) => {
                if report_only {
                    out.action = Action::WouldUpdate;
                } else {
                    take(text, &mut rows)?;
                    out.action = Action::Updated;
                }
            }
            (State::Edited, Upstream::Found(text)) if take_upstream && !report_only => {
                take(text, &mut rows)?;
                out.action = Action::TookUpstream;
            }
            (State::Edited, found) => {
                out.action = Action::Edited;
                out.note = Some(match found {
                    Upstream::Found(text) => {
                        if text != local {
                            out.diff = Some(crate::presets::text_diff(&local, text));
                        }
                        format!(
                            "changed here since it was fetched, so it is left as it is — `satz update-pack {} --take-upstream` overwrites it with the source's file, `satz detach-pack {}` keeps it as this repository's own",
                            row.id, row.id
                        )
                    }
                    _ => format!("changed here since it was fetched, and {} holds it no longer — `satz detach-pack {}` keeps it as this repository's own", source.describe(), row.id),
                });
                attention = true;
            }
            (state, upstream) => unreachable!("{state:?} with {upstream:?}"),
        }
        packs.push(out);
    }
    Ok(UpdateReport { sources: opened.read(), packs, attention })
}

pub(crate) fn render_update(r: &UpdateReport) -> String {
    if r.packs.is_empty() {
        return "no pack here has a recorded origin: nothing to update\n".to_string();
    }
    let mut out = String::new();
    for p in &r.packs {
        let arrow = match &p.upstream_version {
            Some(v) if v != &p.version => format!("{} → {}", p.version, v),
            _ => p.version.clone(),
        };
        let what = match p.action {
            Action::Current => "current",
            Action::Updated => "updated",
            Action::WouldUpdate => "behind — an update would write it",
            Action::Edited => "edited — left as it is",
            Action::TookUpstream => "edited — overwritten with the source's file",
            Action::Gone => "gone from the source — left as it is",
            Action::NotChecked => "not checked",
        };
        out.push_str(&format!("{}  {}  from {}  {}\n", p.id, arrow, p.source, what));
        if let Some(note) = &p.note {
            out.push_str(&format!("    {note}\n"));
        }
        if let Some(diff) = &p.diff {
            out.push_str("    this file (-) against the source's (+):\n");
            for line in diff.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// packs
// ---------------------------------------------------------------------------

/// What a pack needs of the estate that uses it: a place its account was delegated access
/// to. It is not declared. It is what the pack READS from the interface the estate uses —
/// a pack that reads the delegated folder is of no use in an estate that was given none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Need {
    /// it reads `${{interface.delegated_folder}}`
    Folder,
    /// it reads `${{interface.project_id}}` or `${{interface.project_number}}`
    Project,
}

/// The interface values a pack reads, by export name: every `${{interface.<name>}}` outside
/// its comments.
pub(crate) fn interface_reads(text: &str) -> std::collections::BTreeSet<String> {
    const OPEN: &str = "${{interface.";
    let mut out = std::collections::BTreeSet::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
        let mut rest = line;
        while let Some(i) = rest.find(OPEN) {
            rest = &rest[i + OPEN.len()..];
            // `<value>` or `<folder>.<value>`: the value is what a need is read from
            let key: String = rest.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')).collect();
            let name = key.rsplit('.').next().unwrap_or_default();
            if !name.is_empty() && rest[key.len()..].starts_with("}}") {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// The needs of a pack, from its text: the interface values it reads outside comments.
pub(crate) fn needs(text: &str) -> Vec<Need> {
    let read = interface_reads(text);
    let reads = |export: &str| read.contains(export);
    let mut out = Vec::new();
    if reads(crate::prerequisites::DELEGATED_FOLDER) {
        out.push(Need::Folder);
    }
    if reads("project_id") || reads("project_number") {
        out.push(Need::Project);
    }
    out
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct PackRow {
    pub id: String,
    pub state: State,
    /// what the pack needs of the estate that uses it, derived from what it reads: empty
    /// for a pack that reads neither a delegated folder nor a delegated project
    pub needs: Vec<Need>,
    /// the version the local file states; none for a pack that is only available
    pub version: Option<String>,
    /// the version the source's file states, where the source holds the pack
    pub upstream_version: Option<String>,
    /// the name of the pack source: the one a fetched pack's row records, or the one that
    /// holds an available pack; none for the repository's own
    pub source: Option<String>,
    /// for a pack fetched from a GitHub source: the commit its row records
    pub commit: Option<String>,
    /// the line an estate reads the pack with; none for a pack that is only available
    pub use_line: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct PacksReport {
    pub packs_dir: String,
    /// the pack sources read: every one, or the one `--source` names
    pub sources: Vec<SourceRead>,
    pub packs: Vec<PackRow>,
}

/// Every pack in `packs_dir` with its state, and every pack a source holds that
/// `packs_dir` does not — one row per source that holds it. Reads every pack source,
/// the built-in one first, or only the one `only` names.
pub(crate) async fn list(runtime: &ToolConfig, only: Option<&str>) -> Result<PacksReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let rows = read_record(packs_dir)?;
    for row in &rows {
        recorded_text(packs_dir, row)?;
    }
    check_rows(runtime, &rows)?;
    let defs = match only {
        Some(name) => vec![crate::pack_sources::find(runtime, name)?],
        None => crate::pack_sources::all(runtime)?,
    };
    let mut opened = Opened::default();
    for def in &defs {
        opened.get(runtime, &def.name).await?;
    }
    let sources = &opened.0;
    let local = local_ids(packs_dir)?;

    let mut packs = Vec::new();
    for id in &local {
        let path = pack_file(packs_dir, id);
        let text = fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let row = rows.iter().find(|r| &r.id == id);
        let fetched = match row.and_then(|r| sources.iter().find(|s| s.name() == r.source)) {
            Some(source) => Some(source.fetch(id).await?),
            None => None,
        };
        let upstream = match &fetched {
            None => Upstream::NotAsked,
            Some(None) => Upstream::Missing,
            Some(Some(t)) => Upstream::Found(t),
        };
        let state = classify(row, Some(&text), upstream).expect("a local pack has a state");
        packs.push(PackRow {
            id: id.clone(),
            state,
            needs: needs(&text),
            version: crate::presets::pack_version(&text),
            upstream_version: match upstream {
                Upstream::Found(t) => crate::presets::pack_version(t),
                _ => None,
            },
            source: row.map(|r| r.source.clone()),
            commit: row.and_then(|r| r.commit.clone()),
            use_line: use_path(runtime, id).ok().map(|p| format!("use \"{p}\"")),
        });
    }
    for source in sources {
        for id in source.list().await?.into_iter().filter(|id| !local.contains(id)) {
            let text = source.fetch(&id).await?;
            packs.push(PackRow {
                id,
                state: State::Available,
                needs: text.as_deref().map(needs).unwrap_or_default(),
                version: None,
                upstream_version: text.as_deref().and_then(crate::presets::pack_version),
                source: Some(source.name().to_string()),
                commit: None,
                use_line: None,
            });
        }
    }
    // the built-in source first among rows of one id, then the order config.toml lists them in
    let rank = |name: &Option<String>| name.as_ref().and_then(|n| defs.iter().position(|d| &d.name == n)).unwrap_or(0);
    packs.sort_by(|a, b| a.id.cmp(&b.id).then(rank(&a.source).cmp(&rank(&b.source))));
    Ok(PacksReport { packs_dir: packs_dir.display().to_string(), sources: opened.read(), packs })
}

pub(crate) fn render_list(r: &PacksReport) -> String {
    let against = r
        .sources
        .iter()
        .map(|s| match &s.commit {
            Some(c) => format!("{} ({} at {})", s.name, s.location, &c[..12.min(c.len())]),
            None => format!("{} ({})", s.name, s.location),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!("packs in {} against {against}\n", r.packs_dir);
    if r.packs.is_empty() {
        out.push_str("  none here, and the sources hold none\n");
        return out;
    }
    let width = r.packs.iter().map(|p| p.id.len()).max().unwrap_or(0);
    let source_width = r.packs.iter().map(|p| p.source.as_deref().map_or(1, str::len)).max().unwrap_or(0);
    for p in &r.packs {
        let state = match p.state {
            State::Own => "own",
            State::Current => "current",
            State::Behind => "behind",
            State::Edited => "edited",
            State::Gone => "gone from the source",
            State::NotChecked => "not checked (another source)",
            State::Available => "available",
        };
        let version = match (&p.version, &p.upstream_version) {
            (Some(v), Some(u)) if v != u => format!("{v} → {u}"),
            (Some(v), _) => v.clone(),
            (None, Some(u)) => u.clone(),
            (None, None) => "-".to_string(),
        };
        let needs = match p.needs.as_slice() {
            [] => String::new(),
            n => format!(
                "  — needs {}",
                n.iter().map(|x| if *x == Need::Folder { "a delegated folder" } else { "a delegated project" }).collect::<Vec<_>>().join(" and ")
            ),
        };
        let source = p.source.as_deref().unwrap_or("-");
        out.push_str(&format!("  {:width$}  {:source_width$}  {:10}  {}{}\n", p.id, source, version, state, needs));
    }
    out
}

// ---------------------------------------------------------------------------
// detach-pack
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct Detached {
    pub id: String,
    /// the name of the pack source it had been fetched from
    pub source: String,
}

/// Remove a pack's row: the file stays, and from here on it is the repository's own.
pub(crate) fn detach(runtime: &ToolConfig, id: &str) -> Result<Detached, String> {
    valid_id(id)?;
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    let Some(at) = rows.iter().position(|r| r.id == id) else {
        return Err(format!("`{id}` has no recorded origin in {} — nothing to detach", record_path(packs_dir).display()));
    };
    let row = rows.remove(at);
    write_record(packs_dir, rows)?;
    Ok(Detached { id: row.id, source: row.source })
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "pack demo version \"1.0\"\n";
    const V2: &str = "pack demo version \"1.1\"\n";

    fn row(text: &str) -> Row {
        Row { id: "demo".into(), source: "local".into(), github: None, path: None, commit: None, dir: Some("up".into()), version: "1.0".into(), sha256: sha256(text) }
    }

    #[test]
    fn a_packs_needs_are_what_it_reads_from_the_interface() {
        let project = "pack p version \"1.0\"\nparams {\n  p = \"${{interface.project_id}}\"\n}\n";
        let folder = "pack f version \"1.0\"\nparams {\n  parent = \"${{interface.delegated_folder}}\"\n}\n";
        let number = "pack n version \"1.0\"\ngoogle_billing_budget {\n  b {\n    x = \"projects/${{interface.project_number}}\"\n  }\n}\n";
        // a comment that names a value reads nothing, and a pack that reads neither needs neither
        let neither = "// `${{interface.delegated_folder}}` is what another pack reads\npack o version \"1.0\"\nparams {\n  region = \"${{interface.default_region}}\"\n}\n";
        assert_eq!(needs(project), [Need::Project]);
        assert_eq!(needs(folder), [Need::Folder]);
        assert_eq!(needs(number), [Need::Project]);
        assert!(needs(neither).is_empty());
        assert_eq!(needs(&format!("{}{}", folder, project)), [Need::Folder, Need::Project]);
        // the two packs the repository ships say what they are for
        let shipped = |name: &str| std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("packs").join(name)).unwrap();
        assert_eq!(needs(&shipped("workload-project.satz")), [Need::Folder]);
        assert_eq!(needs(&shipped("project-budget.satz")), [Need::Project]);
    }

    #[test]
    fn a_pack_without_a_row_is_own_whatever_the_source_holds() {
        assert_eq!(classify(None, Some(V1), Upstream::Found(V2)), Some(State::Own));
        assert_eq!(classify(None, Some(V1), Upstream::Missing), Some(State::Own));
        assert_eq!(classify(None, Some(V1), Upstream::NotAsked), Some(State::Own));
    }

    #[test]
    fn an_unchanged_pack_follows_the_source() {
        let r = row(V1);
        assert_eq!(classify(Some(&r), Some(V1), Upstream::Found(V1)), Some(State::Current));
        assert_eq!(classify(Some(&r), Some(V1), Upstream::Found(V2)), Some(State::Behind));
        assert_eq!(classify(Some(&r), Some(V1), Upstream::Missing), Some(State::Gone));
        assert_eq!(classify(Some(&r), Some(V1), Upstream::NotAsked), Some(State::NotChecked));
    }

    #[test]
    fn a_changed_pack_is_edited_whatever_the_source_holds() {
        let r = row(V1);
        let mine = "pack demo version \"1.0\"\n// mine\n";
        for upstream in [Upstream::Found(V1), Upstream::Found(V2), Upstream::Found(mine), Upstream::Missing, Upstream::NotAsked] {
            assert_eq!(classify(Some(&r), Some(mine), upstream), Some(State::Edited), "{upstream:?}");
        }
    }

    #[test]
    fn what_only_the_source_holds_is_available_and_a_row_without_a_file_has_no_state() {
        assert_eq!(classify(None, None, Upstream::Found(V1)), Some(State::Available));
        assert_eq!(classify(None, None, Upstream::Missing), None);
        assert_eq!(classify(Some(&row(V1)), None, Upstream::Found(V1)), None);
    }

    #[test]
    fn an_id_is_a_path_without_the_extension() {
        for ok in ["demo", "rag/rag-store", "a_b/c.d-e"] {
            assert!(valid_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "demo.satz", "../demo", "a//b", "/demo", "a/./b", "a b"] {
            assert!(valid_id(bad).is_err(), "{bad}");
        }
    }

    /// A row is followed through config.toml: its source must be named there, and in the
    /// form that source writes — a commit for a repository, the same folder for a folder.
    #[test]
    fn a_row_follows_the_source_config_toml_names() {
        let config: ToolConfig = toml::from_str("[[pack_source]]\nname = \"local\"\ndir = \"up\"\n\n[[pack_source]]\nname = \"official\"\ngithub = \"acme/packs\"\n").unwrap();
        let commit = Some("0123456789abcdef0123456789abcdef01234567".to_string());
        check_rows(&config, &[row(V1)]).unwrap();
        let official = Row { source: "official".into(), github: Some("acme/packs".into()), commit: commit.clone(), dir: None, ..row(V1) };
        check_rows(&config, std::slice::from_ref(&official)).unwrap();
        check_rows(&config, &[Row { source: "satz".into(), github: Some("tjirsch/satz".into()), commit: commit.clone(), dir: None, ..row(V1) }]).unwrap();
        // a row whose repository or folder of it is not the one config.toml names is refused with the edit
        let err = check_rows(&config, &[Row { github: Some("acme/old".into()), ..official.clone() }]).unwrap_err();
        assert!(err.contains("fetched from github.com/acme/old/packs as `official`, and config.toml names github.com/acme/packs/packs") && err.contains("write github = \"acme/packs\" and no `path`"), "{err}");
        let err = check_rows(&config, &[Row { path: Some("catalog".into()), ..official.clone() }]).unwrap_err();
        assert!(err.contains("github.com/acme/packs/catalog"), "{err}");
        let moved: ToolConfig = toml::from_str("[[pack_source]]\nname = \"official\"\ngithub = \"acme/packs\"\npath = \"catalog\"\n").unwrap();
        let err = check_rows(&moved, std::slice::from_ref(&official)).unwrap_err();
        assert!(err.contains("write github = \"acme/packs\" and path = \"catalog\""), "{err}");
        check_rows(&moved, &[Row { path: Some("catalog".into()), ..official.clone() }]).unwrap();

        let err = check_rows(&config, &[Row { source: "other".into(), ..row(V1) }]).unwrap_err();
        assert!(err.contains("`other`, which config.toml does not name") && err.contains("satz pack-source add other"), "{err}");
        let err = check_rows(&config, &[Row { dir: Some("../moved".into()), ..row(V1) }]).unwrap_err();
        assert!(err.contains("folder `../moved` of `local`, and config.toml names `up`"), "{err}");
        let err = check_rows(&config, &[Row { source: "official".into(), ..row(V1) }]).unwrap_err();
        assert!(err.contains("is a GitHub repository"), "{err}");
        let err = check_rows(&config, &[Row { github: Some("acme/packs".into()), commit, dir: None, ..row(V1) }]).unwrap_err();
        assert!(err.contains("is a folder"), "{err}");
    }

    #[test]
    fn the_record_refuses_what_it_does_not_know() {
        let dir = std::env::temp_dir().join(format!("satz-origin-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_record(&dir).unwrap(), Vec::new(), "no file is no row");

        write_record(&dir, vec![row(V1)]).unwrap();
        assert_eq!(read_record(&dir).unwrap(), vec![row(V1)]);

        let path = record_path(&dir);
        std::fs::write(&path, "[[pack]]\nid = \"demo\"\nsource = \"dir\"\nversion = \"1.0\"\nsha256 = \"x\"\nextra = 1\n").unwrap();
        let err = read_record(&dir).unwrap_err();
        assert!(err.contains("origin.toml") && err.contains("not a valid origin record"), "{err}");

        std::fs::write(&path, "[[pack]]\nid = \"demo\"\nsource = \"local\"\ndir = \"up\"\nversion = \"1.0\"\nsha256 = \"x\"\n".repeat(2)).unwrap();
        assert!(read_record(&dir).unwrap_err().contains("has two rows"));

        // the new form: a GitHub source records its commit, a folder source its folder
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let github = Row { source: "satz".into(), github: Some("tjirsch/satz".into()), commit: Some(commit.into()), dir: None, ..row(V1) };
        write_record(&dir, vec![github.clone()]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(&format!("source = \"satz\"\ngithub = \"tjirsch/satz\"\ncommit = \"{commit}\"\n")), "{text}");
        assert_eq!(read_record(&dir).unwrap(), vec![github]);

        let refused = |body: &str, says: &str| {
            std::fs::write(&path, format!("[[pack]]\nid = \"demo\"\n{body}version = \"1.0\"\nsha256 = \"x\"\n")).unwrap();
            let err = read_record(&dir).unwrap_err();
            assert!(err.contains("origin.toml") && err.contains(says), "{body}\n→ {err}");
        };
        // the two values an earlier release wrote are refused with the edit
        refused("source = \"github:tjirsch/satz\"\n", "write `source = \"satz\"`, `github = \"tjirsch/satz\"` and `commit = \"<sha>\"`");
        refused("source = \"dir\"\n", "satz pack-source add <name> --dir <folder>");
        refused("source = \"Local\"\ndir = \"up\"\n", "no pack source name");
        refused("source = \"local\"\n", "neither a repository nor `dir`");
        refused("source = \"local\"\ndir = \"up\"\ngithub = \"acme/packs\"\ncommit = \"0123456789abcdef0123456789abcdef01234567\"\n", "both a repository and `dir`");
        refused("source = \"satz\"\ngithub = \"tjirsch/satz\"\ncommit = \"main\"\n", "no commit");
        refused("source = \"satz\"\ncommit = \"0123456789abcdef0123456789abcdef01234567\"\n", "no `github`");
        refused("source = \"satz\"\ngithub = \"tjirsch/satz\"\n", "no `commit`");

        write_record(&dir, Vec::new()).unwrap();
        assert!(!path.exists(), "a record without rows is no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
