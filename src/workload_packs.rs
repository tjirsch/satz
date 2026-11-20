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

/// A pack's file: its id's `/`-separated segments joined as path parts, so the
/// path is written in the platform's own form.
fn pack_file(packs_dir: &Path, id: &str) -> PathBuf {
    let mut file = packs_dir.to_path_buf();
    let mut segments: Vec<&str> = id.split('/').collect();
    let last = segments.pop().unwrap_or_default();
    file.extend(segments);
    file.push(format!("{last}.satz"));
    file
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
                let written = written.clone();
                return Source::open_dir(def, &written, runtime);
            }
        };
        Ok(Source { def, place })
    }

    /// A folder source ready to read: its folder found. Nothing to wait for.
    fn open_dir(def: Def, written: &str, runtime: &ToolConfig) -> Result<Source, String> {
        let folder = crate::pack_sources::folder(runtime, written);
        if !folder.is_dir() {
            return Err(format!(
                "pack source `{}`: dir = \"{written}\" is no folder ({}) — correct it in config.toml, or remove the source with `satz pack-source remove {}`",
                def.name,
                folder.display(),
                def.name
            ));
        }
        Ok(Source { def, place: Place::Dir { written: written.to_string(), folder } })
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
    /// fetched and unchanged since, and its source was not read: `--source` names another
    /// one, or `--local` reads none
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
    /// the interface values the pack reads that the estate does not give, each with why —
    /// the transpile refuses them once the estate uses the pack
    pub unmet: Vec<String>,
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
/// listed as present. Each pack's reads are matched against `estate`; one the estate does
/// not give is reported, and the pack is fetched all the same.
pub(crate) async fn get(runtime: &ToolConfig, source: Option<&str>, which: &Which, estate: &Estate) -> Result<GetReport, String> {
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
        let unmet = unmet(&matched(declarations_of(&text, &format!("{} `{id}`", source.describe()))?, estate));
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
        report.installed.push(Installed { id, version, path: file.display().to_string(), use_line, unmet });
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
        for reason in &p.unmet {
            out.push_str(&format!("    warning: the estate does not give a value the pack reads — {reason}\n"));
        }
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

/// One param a pack declares, with the value it takes when the estate binds none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct ParamDecl {
    pub name: String,
    /// the value as the pack writes it, in Satz (`"europe-west3"`, `0.5`, `["a", "b"]`,
    /// `"${{interface.project_id}}"`); none where it writes `""`, a value nobody has given yet
    pub default: Option<String>,
    /// the pack asks for it: a `question` names it, or an option of a `oneof` question does
    pub question: bool,
    /// the question's prompt, where one asks for it
    pub prompt: Option<String>,
    /// the question's `why`, what getting the answer wrong costs; none where no question
    /// asks for the param or the question has no `why`
    pub why: Option<String>,
}

/// One `entries <list param> { … }` of a pack: what a team may add to a list param.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct ListParamDecl {
    pub param: String,
    /// the field that names an entry
    pub key: String,
    /// every field an entry may carry, `key` among them
    pub fields: Vec<String>,
    /// per field, the value an entry that does not carry it takes, in Satz
    pub defaults: std::collections::BTreeMap<String, String>,
}

/// One export of a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct ExportDecl {
    /// the `interface "<name>"` block it stands in, the name as written — an interface an
    /// `each` writes is named by its pattern, `ks-{each.id}`; none for a core export
    pub interface: Option<String>,
    pub name: String,
    /// the export's `description "…"`; none where it has none
    pub description: Option<String>,
}

/// One value a pack reads from the interface folders the estate uses,
/// `${{interface.<folder>.<value>}}` or `${{interface.<value>}}`, and whether the estate
/// gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct ReadDecl {
    /// the interface folder a named read names; none for the bare form
    pub folder: Option<String>,
    pub value: String,
    /// the param whose value the read is; none where a resource or an export reads it
    pub param: Option<String>,
    /// the estate binds `param`, so the pack's value is not read and nothing is matched
    pub bound_by_estate: bool,
    /// the used interface folders that give the value: the one a named read names, every
    /// folder that carries a bare one; empty when the estate does not give it
    pub met_by: Vec<String>,
    /// why the estate does not give it, in the words the transpile refuses the read with
    pub unmet_reason: Option<String>,
    /// the description of the export that gives the value, from the first folder of
    /// `met_by`; none where the estate does not give it or the export has none
    pub description: Option<String>,
}

/// What a pack declares, read from its parse: its params, its list params, its exports and
/// the interface values it reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct Declarations {
    pub params: Vec<ParamDecl>,
    pub list_params: Vec<ListParamDecl>,
    pub exports: Vec<ExportDecl>,
    pub reads: Vec<ReadDecl>,
}

/// The estate a pack is matched against: the params its own file binds and the interface
/// files it uses, as its transpile loads them.
pub(crate) struct Estate {
    pub path: PathBuf,
    pub bound: std::collections::BTreeSet<String>,
    pub interfaces: Vec<satz_core::pipeline::UsedInterfaceFile>,
}

/// One interface folder the estate uses: its name, the organization's estate that wrote it
/// and the values it exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct InterfaceFolder {
    /// the folder's name, the one a named read writes: `${{interface.<name>.<value>}}`
    pub name: String,
    /// the name of the organization's estate that wrote it
    pub estate: String,
    /// the values it exports, as it writes them
    pub exports: Vec<FolderExport>,
}

/// One value an interface folder exports: its name and its description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub(crate) struct FolderExport {
    pub name: String,
    /// the export's description; none where it has none
    pub description: Option<String>,
}

impl Estate {
    pub(crate) fn folders(&self) -> Vec<InterfaceFolder> {
        self.interfaces
            .iter()
            .map(|f| InterfaceFolder {
                name: f.interface.name.clone(),
                estate: f.interface.estate.clone(),
                exports: f.interface.outputs.iter().map(|o| FolderExport { name: o.name.clone(), description: o.description.clone() }).collect(),
            })
            .collect()
    }
}

/// A value as a pack writes it, in Satz.
fn satz_text(v: &satz_core::satz::Value) -> String {
    use satz_core::satz::{Entry, Key, StrPart, Value};
    fn string(parts: &[StrPart]) -> String {
        let mut s = String::from("\"");
        for p in parts {
            match p {
                StrPart::Param(r) => s.push_str(&format!("{{{r}}}")),
                StrPart::Lit(t) => {
                    let q = satz_core::satz::quote(t);
                    s.push_str(&q[1..q.len() - 1]);
                }
            }
        }
        s.push('"');
        s
    }
    fn key(k: &Key) -> String {
        match k {
            Key::Ident(i) => i.clone(),
            Key::Str(parts) => string(parts),
        }
    }
    match v {
        Value::Str(parts) => string(parts),
        Value::Num(n) => n.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Ref(r) => r.clone(),
        Value::List(items) => format!("[{}]", items.iter().map(satz_text).collect::<Vec<_>>().join(", ")),
        Value::Obj(entries) => {
            let inner: Vec<String> = entries
                .iter()
                .map(|e| match e {
                    Entry::Attr { key: k, value, .. } => format!("{} = {}", key(k), satz_text(value)),
                    other => format!("{other:?}"),
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
    }
}

/// `""`: a param declared without a value.
fn is_unset(v: &satz_core::satz::Value) -> bool {
    use satz_core::satz::{StrPart, Value};
    matches!(v, Value::Str(parts) if parts.iter().all(|p| matches!(p, StrPart::Lit(t) if t.is_empty())))
}

/// Every text in a value, keys included, `{param}` parts written as such.
fn texts_of_value(v: &satz_core::satz::Value, out: &mut Vec<String>) {
    use satz_core::satz::{StrPart, Value};
    match v {
        Value::Str(parts) => out.push(parts.iter().map(|p| match p {
            StrPart::Lit(t) => t.clone(),
            StrPart::Param(r) => format!("{{{r}}}"),
        }).collect()),
        Value::List(items) => items.iter().for_each(|i| texts_of_value(i, out)),
        Value::Obj(entries) => entries.iter().for_each(|e| texts_of_entry(e, out)),
        Value::Num(_) | Value::Bool(_) | Value::Ref(_) => {}
    }
}

fn texts_of_entry(e: &satz_core::satz::Entry, out: &mut Vec<String>) {
    use satz_core::satz::{Entry, Key, Value};
    let key = |k: &Key, out: &mut Vec<String>| {
        if let Key::Str(parts) = k {
            texts_of_value(&Value::Str(parts.clone()), out);
        }
    };
    match e {
        Entry::Attr { key: k, value, .. } => {
            key(k, out);
            texts_of_value(value, out);
        }
        Entry::Map { key: k, name, body, .. } => {
            key(k, out);
            if let Some(n) = name {
                key(n, out);
            }
            body.iter().for_each(|b| texts_of_entry(b, out));
        }
        Entry::When { entry, .. } => texts_of_entry(entry, out),
        Entry::Each { body, .. } => body.iter().for_each(|b| texts_of_entry(b, out)),
        Entry::Use { .. } => {}
    }
}

/// The interface reads in one text: `(folder, value)` of every `${interface.<value>}` and
/// `${interface.<folder>.<value>}` — `${{…}}` as a pack writes it, its braces unescaped
/// by the parser.
fn reads_in(text: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("${") {
        rest = &rest[i + 2..];
        let Some(end) = rest.find('}') else { break };
        let inner = rest[..end].trim();
        rest = &rest[end + 1..];
        let Some(key) = inner.strip_prefix(satz_core::pipeline::INTERFACE_ROOT).and_then(|k| k.strip_prefix('.')) else { continue };
        match key.rsplit_once('.') {
            Some((folder, value)) => out.push((Some(folder.to_string()), value.to_string())),
            None => out.push((None, key.to_string())),
        }
    }
    out
}

/// What a pack declares, from its parse: the params with their defaults and questions,
/// the list params, the exports and every interface value its params, resources and
/// exports read — a comment reads nothing. The reads are not matched; `matched` does that.
pub(crate) fn declarations(file: &satz_core::satz::File) -> Declarations {
    let asked = |name: &str| file.questions.iter().find(|q| (!q.oneof && q.subject == name) || q.options.iter().any(|o| o.param == name));
    let params = file
        .params
        .iter()
        .filter(|(name, _, _)| satz_core::satz::contribution_target(name).is_none())
        .map(|(name, v, _)| ParamDecl {
            name: name.clone(),
            default: (!is_unset(v)).then(|| satz_text(v)),
            question: asked(name).is_some(),
            prompt: asked(name).map(|q| q.prompt.clone()),
            why: asked(name).and_then(|q| q.why.clone()),
        })
        .collect();
    let list_params = file
        .requests
        .iter()
        .map(|r| ListParamDecl { param: r.param.clone(), key: r.key.clone(), fields: r.fields.clone(), defaults: r.defaults.iter().map(|(f, v, _)| (f.clone(), satz_text(v))).collect() })
        .collect();
    let mut exports: Vec<ExportDecl> = file.exports.iter().map(|e| ExportDecl { interface: None, name: e.name.clone(), description: e.description.clone() }).collect();
    for i in &file.interfaces {
        exports.extend(i.exports.iter().map(|e| ExportDecl { interface: Some(i.name.clone()), name: e.name.clone(), description: e.description.clone() }));
    }

    // the texts a read can stand in, each with the param it is the value of
    let mut texts: Vec<(Option<String>, String)> = Vec::new();
    let mut push = |param: Option<&str>, found: Vec<String>| texts.extend(found.into_iter().map(|t| (param.map(str::to_string), t)));
    for (name, v, _) in &file.params {
        let mut t = Vec::new();
        texts_of_value(v, &mut t);
        push(Some(name), t);
    }
    for i in &file.inputs {
        let mut t = Vec::new();
        texts_of_value(&i.value, &mut t);
        push(Some(&i.name), t);
    }
    let mut t = Vec::new();
    file.items.iter().for_each(|e| texts_of_entry(e, &mut t));
    for r in &file.requests {
        r.defaults.iter().for_each(|(_, v, _)| texts_of_value(v, &mut t));
    }
    for e in file.exports.iter().chain(file.interfaces.iter().flat_map(|i| &i.exports)) {
        if let satz_core::satz::ExportValue::Value(v) = &e.value {
            texts_of_value(v, &mut t);
        }
    }
    push(None, t);
    let mut reads: Vec<ReadDecl> = Vec::new();
    for (param, text) in texts {
        for (folder, value) in reads_in(&text) {
            let read = ReadDecl { folder, value, param: param.clone(), bound_by_estate: false, met_by: Vec::new(), unmet_reason: None, description: None };
            if !reads.contains(&read) {
                reads.push(read);
            }
        }
    }
    Declarations { params, list_params, exports, reads }
}

/// The declarations of a pack's text. A text that does not parse is an error naming `what`.
pub(crate) fn declarations_of(text: &str, what: &str) -> Result<Declarations, String> {
    let file = satz_core::satz::parse(text).map_err(|e| format!("{what}: line {}: {}", e.line, e.msg))?;
    Ok(declarations(&file))
}

/// Each read matched against the estate, by the transpile's own rule (ADR 0094): a named
/// read is given by the used interface folder of that name when it carries the value, a
/// bare one by every used folder that carries it when they agree. A read that is the value
/// of a param the estate binds is not made.
pub(crate) fn matched(mut d: Declarations, estate: &Estate) -> Declarations {
    for r in &mut d.reads {
        if r.param.as_ref().is_some_and(|p| estate.bound.contains(p)) {
            r.bound_by_estate = true;
            continue;
        }
        let key = match &r.folder {
            Some(f) => format!("{f}.{}", r.value),
            None => r.value.clone(),
        };
        match satz_core::pipeline::interface_read(&key, &estate.interfaces) {
            Ok((folders, export)) => {
                r.met_by = folders.into_iter().map(str::to_string).collect();
                r.description = export.description.clone();
            }
            Err(e) => r.unmet_reason = Some(e),
        }
    }
    d
}

/// The reads of a pack the estate does not give, one line each.
pub(crate) fn unmet(d: &Declarations) -> Vec<String> {
    d.reads.iter().filter_map(|r| r.unmet_reason.clone()).collect()
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct PackRow {
    pub id: String,
    pub state: State,
    /// what the pack declares — params, list params, exports — and the interface values it
    /// reads, each matched against the estate
    #[serde(flatten)]
    pub declarations: Declarations,
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
    /// the estate the reads are matched against
    pub estate: String,
    /// the interface folders that estate uses, each with the organization's estate that wrote
    /// it and the values it exports
    pub interface_folders: Vec<InterfaceFolder>,
    /// the pack sources read: every one, or the one `--source` names; none with `--local`
    pub sources: Vec<SourceRead>,
    pub packs: Vec<PackRow>,
}

/// Every pack in `packs_dir` with its state, and every pack a source holds that
/// `packs_dir` does not — one row per source that holds it. Reads every pack source,
/// the built-in one first, or only the one `only` names; `local` reads none and lists
/// only `packs_dir`, each fetched pack `edited` or `not-checked` by its recorded sha256.
/// Each row carries what the pack declares, its reads matched against `estate`.
pub(crate) async fn list(runtime: &ToolConfig, only: Option<&str>, local: bool, estate: &Estate) -> Result<PacksReport, String> {
    if local && only.is_some() {
        return Err("--local reads no pack source and --source names one to read — give one of the two".to_string());
    }
    let packs_dir = Path::new(&runtime.packs_dir);
    let rows = read_record(packs_dir)?;
    for row in &rows {
        recorded_text(packs_dir, row)?;
    }
    check_rows(runtime, &rows)?;
    let defs = match only {
        _ if local => Vec::new(),
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
            declarations: matched(declarations_of(&text, &path.display().to_string())?, estate),
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
            let Some(text) = source.fetch(&id).await? else {
                return Err(format!("{} lists the pack `{id}` and holds no file for it", source.describe()));
            };
            let declarations = matched(declarations_of(&text, &format!("{} `{id}`", source.describe()))?, estate);
            packs.push(PackRow {
                id,
                state: State::Available,
                declarations,
                version: None,
                upstream_version: crate::presets::pack_version(&text),
                source: Some(source.name().to_string()),
                commit: None,
                use_line: None,
            });
        }
    }
    // the built-in source first among rows of one id, then the order config.toml lists them in
    let rank = |name: &Option<String>| name.as_ref().and_then(|n| defs.iter().position(|d| &d.name == n)).unwrap_or(0);
    packs.sort_by(|a, b| a.id.cmp(&b.id).then(rank(&a.source).cmp(&rank(&b.source))));
    Ok(PacksReport { packs_dir: packs_dir.display().to_string(), estate: estate.path.display().to_string(), interface_folders: estate.folders(), sources: opened.read(), packs })
}

pub(crate) fn render_list(r: &PacksReport) -> String {
    let against = match r.sources.as_slice() {
        [] => "no pack source read".to_string(),
        sources => format!(
            "against {}",
            sources
                .iter()
                .map(|s| match &s.commit {
                    Some(c) => format!("{} ({} at {})", s.name, s.location, &c[..12.min(c.len())]),
                    None => format!("{} ({})", s.name, s.location),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut out = match r.interface_folders.as_slice() {
        [] => format!("packs in {}, {against}\nreads matched against {}, which uses no interface folder\n", r.packs_dir, r.estate),
        folders => {
            let mut out = format!("packs in {}, {against}\nreads matched against {}, which uses the interface folders\n", r.packs_dir, r.estate);
            for f in folders {
                let exports = if f.exports.is_empty() { "nothing".to_string() } else { f.exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ") };
                out.push_str(&format!("  {}, written by {}, exports {exports}\n", f.name, f.estate));
            }
            out
        }
    };
    if r.packs.is_empty() {
        out.push_str(if r.sources.is_empty() { "  none here\n" } else { "  none here, and the sources hold none\n" });
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
            State::NotChecked => "not checked (source not read)",
            State::Available => "available",
        };
        let version = match (&p.version, &p.upstream_version) {
            (Some(v), Some(u)) if v != u => format!("{v} → {u}"),
            (Some(v), _) => v.clone(),
            (None, Some(u)) => u.clone(),
            (None, None) => "-".to_string(),
        };
        let source = p.source.as_deref().unwrap_or("-");
        out.push_str(&format!("  {:width$}  {:source_width$}  {:10}  {}\n", p.id, source, version, state));
        out.push_str(&render_declarations(&p.declarations, "      "));
    }
    out
}

/// A pack's declarations, compact: one line per kind it declares, and one per read the
/// estate does not give.
pub(crate) fn render_declarations(d: &Declarations, indent: &str) -> String {
    let mut out = String::new();
    let mut line = |label: &str, items: Vec<String>| {
        if !items.is_empty() {
            out.push_str(&format!("{indent}{label}: {}\n", items.join(", ")));
        }
    };
    line("params", d.params.iter().map(|p| if p.question { format!("{} (asked)", p.name) } else { p.name.clone() }).collect());
    line("list params", d.list_params.iter().map(|l| format!("{} by {}", l.param, l.key)).collect());
    line(
        "exports",
        d.exports
            .iter()
            .map(|e| match &e.interface {
                Some(i) => format!("{} in interface \"{i}\"", e.name),
                None => e.name.clone(),
            })
            .collect(),
    );
    line(
        "reads",
        d.reads
            .iter()
            .map(|r| {
                let read = match &r.folder {
                    Some(f) => format!("{f}.{}", r.value),
                    None => r.value.clone(),
                };
                match (&r.met_by[..], &r.param) {
                    (_, Some(p)) if r.bound_by_estate => format!("{read} (the estate binds {p})"),
                    ([], _) => format!("{read} (not given)"),
                    (by, _) => format!("{read} from {}", by.join(" and ")),
                }
            })
            .collect(),
    );
    for reason in unmet(d) {
        out.push_str(&format!("{indent}not given: {reason}\n"));
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

// ---------------------------------------------------------------------------
// publish-pack
// ---------------------------------------------------------------------------

/// What `publish-pack` did with the file in the source's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PublishAction {
    /// the folder held no file of that id: the pack was copied there
    Copied,
    /// the folder held the same file: nothing was copied
    Identical,
    /// `force`: the folder held another file, and the pack was copied over it
    Overwritten,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct Published {
    pub id: String,
    /// the version the pack states
    pub version: String,
    /// the name of the folder source it was copied to
    pub source: String,
    /// that source's folder, as config.toml writes it
    pub location: String,
    /// the file in the source's folder, where `get-pack --source <source>` reads it
    pub path: String,
    pub action: PublishAction,
    /// the estate the pack was checked in
    pub estate: String,
}

/// Copy one of the repository's own packs into a folder source, and record it as fetched
/// from there, so `update-pack` compares it with the source from then on. Refused for a
/// GitHub source, for a pack fetched from somewhere (a row in the origin record), for a
/// source file of that id that differs unless `force`, and for a pack that does not clear
/// `review-pack`'s bar inside `estate` — formatted, and the estate transpiles with it.
pub(crate) fn publish(tool: &ToolConfig, runtime: &ToolConfig, id: &str, source: &str, estate: &Path, force: bool) -> Result<Published, String> {
    valid_id(id)?;
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    check_rows(runtime, &rows)?;
    let def = crate::pack_sources::find(runtime, source)?;
    let written = match &def.location {
        Location::Dir(written) => written.clone(),
        Location::Github { .. } => {
            return Err(format!(
                "`{}` is the GitHub source {} — a GitHub source is written by a pull request to its repository; publish-pack writes only a folder source",
                def.name,
                def.describe()
            ))
        }
    };
    let local = pack_file(packs_dir, id);
    if !local.is_file() {
        return Err(format!("`{id}` is not in {} — publish-pack copies a pack of packs_dir, named by its path there without `.satz`", packs_dir.display()));
    }
    if let Some(row) = rows.iter().find(|r| r.id == id) {
        return Err(format!("the pack `{id}` came from `{}`; detach it first (`satz detach-pack {id}`) to publish it as your own", row.source));
    }
    let text = fsx::read_to_string(&local).map_err(|e| format!("{}: {}", local.display(), e))?;
    let version = crate::presets::pack_version(&text).ok_or_else(|| format!("{}: no `pack <id> version \"…\"` statement, so it is no pack", local.display()))?;

    let source = Source::open_dir(def, &written, runtime)?;
    let target = pack_file(&crate::pack_sources::folder(runtime, &written), id);
    let action = if !target.exists() {
        PublishAction::Copied
    } else {
        let there = fsx::read_to_string(&target).map_err(|e| format!("{}: {}", target.display(), e))?;
        match (there == text, force) {
            (true, _) => PublishAction::Identical,
            (false, true) => PublishAction::Overwritten,
            (false, false) => {
                return Err(format!(
                    "{} holds another `{id}` than {} — compare the two files; `--force` copies this one over it",
                    target.display(),
                    local.display()
                ))
            }
        }
    };

    if !estate.is_file() {
        return Err(format!("no estate file at {} — the pack is checked in the estate it was tested in", estate.display()));
    }
    // the bar a pack is published with: review-pack's, inside the estate it was tested in
    let review = crate::review_pack::review(&local, Some(estate), crate::review_pack::Bar::Workload, tool, runtime).map_err(|e| format!("{}: {}", local.display(), e))?;
    if !review.passed() {
        let errors: Vec<String> = review
            .findings
            .iter()
            .filter(|f| f.severity == crate::findings::Severity::Error)
            .map(|f| match f.line {
                Some(line) => format!("  line {line}: {}", f.message),
                None => format!("  {}", f.message),
            })
            .collect();
        return Err(format!(
            "{} does not clear review-pack's bar inside {}, so it is not published:\n{}\n`satz review-pack {} --against {} --format text --out -` lists every finding",
            local.display(),
            estate.display(),
            errors.join("\n"),
            local.display(),
            estate.display()
        ));
    }

    if action != PublishAction::Identical {
        if let Some(parent) = target.parent() {
            fsx::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fsx::write_verbatim(&target, text.as_bytes()).map_err(|e| e.to_string())?;
    }
    rows.push(source.row(id, &version, &text));
    write_record(packs_dir, rows)?;
    Ok(Published {
        id: id.to_string(),
        version,
        source: source.def.name.clone(),
        location: written,
        path: target.display().to_string(),
        action,
        estate: estate.display().to_string(),
    })
}

pub(crate) fn render_publish(p: &Published) -> String {
    let what = match p.action {
        PublishAction::Copied => format!("published {} {} to `{}` ({}) → {}", p.id, p.version, p.source, p.location, p.path),
        PublishAction::Identical => format!("published {} {}: `{}` ({}) holds the same file, nothing copied → {}", p.id, p.version, p.source, p.location, p.path),
        PublishAction::Overwritten => format!("published {} {} to `{}` ({}), over the file that was there → {}", p.id, p.version, p.source, p.location, p.path),
    };
    format!("{what}\n    checked in {}; packs_dir/origin.toml records it as fetched from `{}`, so `satz update-pack {}` follows the source\n", p.estate, p.source, p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "pack demo version \"1.0\"\n";
    const V2: &str = "pack demo version \"1.1\"\n";

    fn row(text: &str) -> Row {
        Row { id: "demo".into(), source: "local".into(), github: None, path: None, commit: None, dir: Some("up".into()), version: "1.0".into(), sha256: sha256(text) }
    }

    const DECLARING: &str = "// `${{interface.delegated_folder}}` in a comment is read by nobody\n\
        pack declaring version \"1.0\"\n\n\
        params {\n\
          region  = \"europe-west3\"\n\
          project = \"${{interface.project_id}}\"\n\
          owner   = \"\"\n\
          sizes   = [1, 2]\n\
          stores  = []\n\
        }\n\n\
        question owner {\n\
          prompt   = \"The group that owns it\"\n\
          why      = \"Its members are the ones paged\"\n\
          reversal = edit\n\
          blast    = low\n\
        }\n\n\
        entries stores {\n\
          key    = \"name\"\n\
          fields = [\"name\", \"tier\"]\n\
          defaults = { tier = \"standard\" }\n\
        }\n\n\
        google_compute_instance {\n\
          vm {\n\
            name    = \"${{interface.delegation-vm.instance_name}}\"\n\
            project = project\n\
          }\n\
        }\n\n\
        export \"vm_name\" = \"${{google_compute_instance.vm.name}}\" description \"The VM's name\"\n\n\
        interface \"vm-users\" {\n\
          export \"vm_zone\" = \"${{interface.zone}}\"\n\
        }\n";

    /// A parameter of the estate a pack is matched against: interface folders by name, each
    /// with its values.
    fn estate(bound: &[&str], folders: &[(&str, &[(&str, &str)])]) -> Estate {
        let folders: Vec<DescribedFolder> = folders.iter().map(|(name, values)| (*name, values.iter().map(|(v, value)| (*v, *value, None)).collect())).collect();
        described_estate(bound, &folders)
    }

    /// An interface folder by name, each value with its description or none.
    type DescribedFolder<'a> = (&'a str, Vec<(&'a str, &'a str, Option<&'a str>)>);

    /// `estate`, each value with the description its folder gives it, or none.
    fn described_estate(bound: &[&str], folders: &[DescribedFolder]) -> Estate {
        let interfaces = folders
            .iter()
            .map(|(name, values)| {
                let mut text = format!("interface \"{name}\"\n\ncentral {{\n  estate        = \"acme\"\n  organizations = [\"organizations/123456789012\"]\n}}\n");
                for (v, value, description) in values {
                    let description = description.map(|d| format!("  description = \"{d}\"\n")).unwrap_or_default();
                    text.push_str(&format!("\noutput \"{v}\" {{\n  value = \"{value}\"\n{description}}}\n"));
                }
                let interface = satz_core::satz::parse(&text).unwrap().interface_file.unwrap();
                satz_core::pipeline::UsedInterfaceFile { file: format!("interfaces/{name}/satz/interface.satz"), interface }
            })
            .collect();
        Estate { path: PathBuf::from("team.satz"), bound: bound.iter().map(|b| b.to_string()).collect(), interfaces }
    }

    fn read<'a>(d: &'a Declarations, value: &str) -> &'a ReadDecl {
        d.reads.iter().find(|r| r.value == value).unwrap_or_else(|| panic!("no read of {value}: {:?}", d.reads))
    }

    /// Params with and without a value and with a question, a list param, a core export
    /// and an interface export, and the reads in a param, a resource and an export — a
    /// comment reads nothing.
    #[test]
    fn a_pack_is_described_by_what_it_declares() {
        let d = declarations_of(DECLARING, "declaring.satz").unwrap();
        let param = |n: &str| d.params.iter().find(|p| p.name == n).unwrap_or_else(|| panic!("no param {n}"));
        assert_eq!(d.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["region", "project", "owner", "sizes", "stores"]);
        assert_eq!(param("region").default.as_deref(), Some("\"europe-west3\""));
        assert_eq!(param("project").default.as_deref(), Some("\"${{interface.project_id}}\""));
        assert_eq!(param("sizes").default.as_deref(), Some("[1, 2]"));
        assert_eq!(param("owner").default, None, "a param written \"\" has no default");
        assert!(param("owner").question && param("owner").prompt.as_deref() == Some("The group that owns it"));
        assert_eq!(param("owner").why.as_deref(), Some("Its members are the ones paged"));
        assert!(!param("region").question && param("region").prompt.is_none() && param("region").why.is_none());

        assert_eq!(d.list_params.len(), 1);
        let l = &d.list_params[0];
        assert_eq!((l.param.as_str(), l.key.as_str(), l.fields.clone()), ("stores", "name", vec!["name".to_string(), "tier".to_string()]));
        assert_eq!(l.defaults.get("tier").map(String::as_str), Some("\"standard\""));

        assert_eq!(
            d.exports,
            [
                ExportDecl { interface: None, name: "vm_name".into(), description: Some("The VM's name".into()) },
                ExportDecl { interface: Some("vm-users".into()), name: "vm_zone".into(), description: None }
            ]
        );

        assert_eq!(
            d.reads.iter().map(|r| (r.folder.as_deref(), r.value.as_str(), r.param.as_deref())).collect::<Vec<_>>(),
            [(None, "project_id", Some("project")), (Some("delegation-vm"), "instance_name", None), (None, "zone", None)]
        );
        assert!(!d.reads.iter().any(|r| r.value == "delegated_folder"), "a comment was read");
    }

    /// A delegation of one VM: its folder exports the instance's name, and a
    /// pack that reads it is met — the estate needs no folder and no project of its own.
    #[test]
    fn a_read_is_met_by_the_folder_that_gives_it() {
        let vm_only = estate(&[], &[("delegation-vm", &[("instance_name", "vm-1"), ("zone", "europe-west3-a"), ("project_id", "acme-vm-001")])]);
        let d = matched(declarations_of(DECLARING, "declaring.satz").unwrap(), &vm_only);
        assert!(unmet(&d).is_empty(), "{:?}", d.reads);
        assert_eq!(read(&d, "instance_name").met_by, ["delegation-vm"]);
        // the bare form: every folder that carries it, when they agree
        let two = estate(&[], &[("delegation-vm", &[("instance_name", "vm-1"), ("zone", "z"), ("project_id", "p")]), ("vm", &[("zone", "z")])]);
        let d = matched(declarations_of(DECLARING, "declaring.satz").unwrap(), &two);
        assert_eq!(read(&d, "zone").met_by, ["delegation-vm", "vm"]);
    }

    /// A read carries the description of the export that gives it: the first folder's where
    /// several give it, none where the export has none or the estate does not give the value.
    #[test]
    fn a_read_carries_the_description_of_the_export_that_gives_it() {
        let d = matched(
            declarations_of(DECLARING, "declaring.satz").unwrap(),
            &described_estate(
                &[],
                &[
                    ("delegation-vm", vec![("instance_name", "vm-1", Some("The VM's name")), ("zone", "z", Some("The VM's zone"))]),
                    ("vm", vec![("zone", "z", Some("The zone of the VM"))]),
                ],
            ),
        );
        assert_eq!(read(&d, "instance_name").description.as_deref(), Some("The VM's name"));
        assert_eq!((read(&d, "zone").met_by.clone(), read(&d, "zone").description.as_deref()), (vec!["delegation-vm".to_string(), "vm".to_string()], Some("The VM's zone")));
        let r = read(&d, "project_id");
        assert!(r.met_by.is_empty() && r.unmet_reason.is_some() && r.description.is_none(), "{r:?}");
        // met by an export without a description
        let d = matched(declarations_of(DECLARING, "declaring.satz").unwrap(), &estate(&[], &[("delegation-vm", &[("instance_name", "vm-1")])]));
        let r = read(&d, "instance_name");
        assert!(r.met_by == ["delegation-vm"] && r.description.is_none(), "{r:?}");
    }

    #[test]
    fn a_read_the_estate_does_not_give_says_why() {
        let decl = || declarations_of(DECLARING, "declaring.satz").unwrap();
        // no used folder of the name
        let d = matched(decl(), &estate(&[], &[("vm", &[("instance_name", "vm-1"), ("zone", "z"), ("project_id", "p")])]));
        let r = read(&d, "instance_name");
        assert!(r.met_by.is_empty() && r.unmet_reason.as_deref().is_some_and(|m| m.contains("no used interface folder is named `delegation-vm`")), "{r:?}");
        // the folder, without the value
        let d = matched(decl(), &estate(&[], &[("delegation-vm", &[("zone", "z"), ("project_id", "p")])]));
        let r = read(&d, "instance_name");
        assert!(r.unmet_reason.as_deref().is_some_and(|m| m.contains("the interface folder `delegation-vm` has no value `instance_name`")), "{r:?}");
        // the bare form, carried differently by two folders: name the folder
        let d = matched(
            decl(),
            &estate(&[], &[("delegation-vm", &[("instance_name", "vm-1"), ("zone", "a"), ("project_id", "p")]), ("other", &[("zone", "b")])]),
        );
        let r = read(&d, "zone");
        assert!(r.met_by.is_empty() && r.unmet_reason.as_deref().is_some_and(|m| m.contains("each have a `zone` and they differ — name the folder")), "{r:?}");
        // no interface folder at all
        let d = matched(decl(), &estate(&[], &[]));
        assert_eq!(unmet(&d).len(), 3, "{:?}", d.reads);
        // a param the estate binds is not read from the interface
        let d = matched(decl(), &estate(&["project"], &[("delegation-vm", &[("instance_name", "vm-1"), ("zone", "z")])]));
        let r = read(&d, "project_id");
        assert!(r.bound_by_estate && r.met_by.is_empty() && r.unmet_reason.is_none(), "{r:?}");
        assert!(unmet(&d).is_empty(), "{:?}", d.reads);
        let text = render_declarations(&d, "  ");
        assert!(text.contains("  params: region, project, owner (asked), sizes, stores\n") && text.contains("project_id (the estate binds project)"), "{text}");
    }

    /// The packs the repository ships: the budget reads the project it counts by the bare
    /// form, through params an estate with several delegated projects binds.
    #[test]
    fn the_shipped_packs_say_what_they_read() {
        let shipped = |name: &str| std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("packs").join(name)).unwrap();
        let d = declarations_of(&shipped("project-budget.satz"), "project-budget.satz").unwrap();
        assert_eq!(
            d.reads.iter().map(|r| (r.value.as_str(), r.param.as_deref())).collect::<Vec<_>>(),
            [("project_id", Some("budget_project_id")), ("project_number", Some("budget_project_number")), ("billing_account", Some("budget_billing_account"))]
        );
        let d = declarations_of(&shipped("workload-project.satz"), "workload-project.satz").unwrap();
        assert_eq!(d.reads.iter().map(|r| r.value.as_str()).collect::<Vec<_>>(), ["delegated_folder"]);
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

    /// An estate whose own pack `team/logs` it uses, a folder source `shared`, a GitHub
    /// source `remote`, and the configuration that reads them.
    fn publishing_estate(name: &str) -> (PathBuf, ToolConfig, PathBuf) {
        let dir = std::env::temp_dir().join(format!("satz-publish-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("satz")).unwrap();
        std::fs::create_dir_all(dir.join("packs/team")).unwrap();
        std::fs::create_dir_all(dir.join("shared")).unwrap();
        let config = dir.join("config.toml");
        std::fs::write(
            &config,
            "yaml_dir = \"satz\"\npacks_dir = \"packs\"\ninclude_dirs = [\".\", \"satz\"]\n\n\
             [[pack_source]]\nname = \"shared\"\ndir = \"shared\"\n\n[[pack_source]]\nname = \"remote\"\ngithub = \"acme/packs\"\n",
        )
        .unwrap();
        let mut cfg = crate::settings::resolved_config(&crate::settings::parse_tool_config(&config).unwrap(), &dir);
        cfg.schema_dir = crate::corpus::schema_dir();
        std::fs::write(dir.join("packs/team/logs.satz"), LOGS).unwrap();
        let estate = dir.join("satz/team.satz");
        std::fs::write(
            &estate,
            "estate team\n\nparams {\n  customer_organization_id = \"123456789012\"\n}\n\n\
             terraform {\n  backend {\n    local { path = \"terraform.tfstate\" }\n  }\n}\n\nuse \"packs/team/logs.satz\"\n",
        )
        .unwrap();
        (dir, cfg, estate)
    }

    const LOGS: &str = "// A log bucket for the team.\npack logs version \"1.0\"\n\n\
                        google_storage_bucket {\n  logs {\n    name     = \"acme-logs\"\n    location = \"EU\"\n  }\n}\n";

    /// The pack lands where `get-pack --source shared` reads it, and the row is the one that
    /// fetch would write, so the estate follows the source from then on. Published again
    /// over the same file, nothing is copied.
    #[test]
    fn a_published_pack_is_copied_to_the_folder_and_recorded_as_fetched_from_it() {
        let (dir, cfg, estate) = publishing_estate("ok");
        let published = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap();
        assert_eq!(published.action, PublishAction::Copied);
        assert_eq!((published.version.as_str(), published.location.as_str()), ("1.0", "shared"));
        assert_eq!(std::fs::read_to_string(dir.join("shared/team/logs.satz")).unwrap(), LOGS);
        let rows = read_record(&dir.join("packs")).unwrap();
        let want = Row { id: "team/logs".into(), source: "shared".into(), github: None, path: None, commit: None, dir: Some("shared".into()), version: "1.0".into(), sha256: sha256(LOGS) };
        assert_eq!(rows, vec![want]);
        check_rows(&cfg, &rows).unwrap();
        assert_eq!(classify(rows.first(), Some(LOGS), Upstream::Found(LOGS)), Some(State::Current));

        // the row is there now: publishing a fetched pack is refused until it is detached
        let err = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap_err();
        assert!(err.contains("the pack `team/logs` came from `shared`; detach it first (`satz detach-pack team/logs`)"), "{err}");
        detach(&cfg, "team/logs").unwrap();
        let again = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap();
        assert_eq!(again.action, PublishAction::Identical);
        assert_eq!(read_record(&dir.join("packs")).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An id's segments are path parts, and a dot inside an id stays part of its name.
    #[test]
    fn a_pack_file_is_its_ids_segments_with_satz_appended() {
        let base = Path::new("packs");
        assert_eq!(pack_file(base, "team/logs"), base.join("team").join("logs.satz"));
        assert_eq!(pack_file(base, "budget.v2"), base.join("budget.v2.satz"));
    }

    #[test]
    fn a_different_file_in_the_folder_is_refused_unless_forced() {
        let (dir, cfg, estate) = publishing_estate("force");
        std::fs::create_dir_all(dir.join("shared/team")).unwrap();
        let theirs = LOGS.replace("1.0", "0.9");
        std::fs::write(dir.join("shared/team/logs.satz"), &theirs).unwrap();
        let err = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap_err();
        let there = dir.join("shared").join("team").join("logs.satz");
        let here = dir.join("packs").join("team").join("logs.satz");
        assert!(err.contains(&format!("{} holds another `team/logs` than {}", there.display(), here.display())) && err.contains("--force"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("shared/team/logs.satz")).unwrap(), theirs, "a refusal wrote the file");
        assert!(read_record(&dir.join("packs")).unwrap().is_empty(), "a refusal wrote a row");
        let forced = publish(&cfg, &cfg, "team/logs", "shared", &estate, true).unwrap();
        assert_eq!(forced.action, PublishAction::Overwritten);
        assert_eq!(std::fs::read_to_string(dir.join("shared/team/logs.satz")).unwrap(), LOGS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_writes_only_a_folder_source_it_knows() {
        let (dir, cfg, estate) = publishing_estate("sources");
        for github in ["remote", "satz"] {
            let err = publish(&cfg, &cfg, "team/logs", github, &estate, false).unwrap_err();
            assert!(err.contains("a GitHub source is written by a pull request to its repository; publish-pack writes only a folder source"), "{github}: {err}");
        }
        let err = publish(&cfg, &cfg, "team/logs", "nowhere", &estate, false).unwrap_err();
        assert!(err.contains("no pack source `nowhere`"), "{err}");
        let err = publish(&cfg, &cfg, "team/none", "shared", &estate, false).unwrap_err();
        assert!(err.contains("`team/none` is not in"), "{err}");
        assert!(!dir.join("shared/team").exists() && read_record(&dir.join("packs")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The estate does not transpile with the pack, or the pack is not formatted: nothing is
    /// copied and no row is written, and the refusal says why.
    #[test]
    fn a_pack_that_does_not_clear_the_bar_is_not_published() {
        let (dir, cfg, estate) = publishing_estate("bar");
        let broken = LOGS.replace("location = \"EU\"", "shade    = \"EU\"");
        std::fs::write(dir.join("packs/team/logs.satz"), &broken).unwrap();
        let err = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap_err();
        assert!(err.contains("does not clear review-pack's bar") && err.contains("does not transpile"), "{err}");
        let unformatted = LOGS.replace("name     =", "name =");
        std::fs::write(dir.join("packs/team/logs.satz"), &unformatted).unwrap();
        let err = publish(&cfg, &cfg, "team/logs", "shared", &estate, false).unwrap_err();
        assert!(err.contains("not formatted") && err.contains("satz review-pack"), "{err}");
        assert!(!dir.join("shared/team/logs.satz").exists(), "a refused pack was copied");
        assert!(read_record(&dir.join("packs")).unwrap().is_empty(), "a refused pack was recorded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--local` lists `packs_dir` alone: the repository's own pack `own`, a fetched one whose
    /// file still has its recorded sha256 `not-checked`, a changed one `edited` — with the
    /// row's source and commit — and reads no source: the two configured here cannot be read,
    /// a folder that is not there and a repository nobody has, and the listing succeeds.
    #[tokio::test]
    async fn local_lists_packs_dir_and_reads_no_source() {
        let (dir, mut cfg, _) = publishing_estate("local");
        let config = "[[pack_source]]\nname = \"gone\"\ndir = \"no-such-folder\"\n\n[[pack_source]]\nname = \"remote\"\ngithub = \"acme/no-such-repository\"\n";
        cfg.pack_source = toml::from_str::<ToolConfig>(config).unwrap().pack_source;
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let fetched = |id: &str, text: &str| {
            std::fs::write(dir.join(format!("packs/team/{id}.satz")), text).unwrap();
            Row { id: format!("team/{id}"), source: "remote".into(), github: Some("acme/no-such-repository".into()), path: None, commit: Some(commit.into()), dir: None, version: "1.0".into(), sha256: sha256(text) }
        };
        let kept = fetched("kept", &DECLARING.replace("declaring", "kept"));
        let changed = Row { source: "gone".into(), github: None, commit: None, dir: Some("no-such-folder".into()), ..fetched("changed", &LOGS.replace("logs", "changed")) };
        std::fs::write(dir.join("packs/team/changed.satz"), LOGS.replace("logs", "changed") + "// changed here\n").unwrap();
        write_record(&dir.join("packs"), vec![kept, changed]).unwrap();
        let against = estate(&[], &[("delegation-vm", &[("instance_name", "vm-1")]), ("vm-users", &[("zone", "europe-west3-a"), ("project_id", "acme-vm")])]);

        let report = list(&cfg, None, true, &against).await.unwrap();
        assert!(report.sources.is_empty(), "{:?}", report.sources);
        let states: Vec<(&str, State, Option<&str>, Option<&str>)> = report.packs.iter().map(|p| (p.id.as_str(), p.state, p.source.as_deref(), p.commit.as_deref())).collect();
        assert_eq!(
            states,
            [("team/changed", State::Edited, Some("gone"), None), ("team/kept", State::NotChecked, Some("remote"), Some(commit)), ("team/logs", State::Own, None, None)]
        );
        let kept = report.packs.iter().find(|p| p.id == "team/kept").unwrap();
        assert_eq!(kept.upstream_version, None);
        assert_eq!(kept.declarations.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["region", "project", "owner", "sizes", "stores"]);
        assert_eq!(read(&kept.declarations, "instance_name").met_by, ["delegation-vm"]);
        assert!(render_list(&report).contains("no pack source read"), "{}", render_list(&report));

        // the same configuration read for real fails: the source is not there
        let err = list(&cfg, Some("gone"), false, &against).await.unwrap_err();
        assert!(err.contains("no-such-folder"), "{err}");
        let err = list(&cfg, Some("gone"), true, &against).await.unwrap_err();
        assert!(err.contains("--local reads no pack source and --source names one"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The report lists each interface folder the estate uses by name, with the organization's
    /// estate that wrote it and the names of the values it exports.
    #[tokio::test]
    async fn the_report_lists_the_interface_folders_with_their_exports() {
        let (dir, cfg, _) = publishing_estate("folders");
        let against = described_estate(&[], &[("delegation-vm", vec![("instance_name", "vm-1", Some("The VM's name")), ("zone", "z", None)]), ("vm-users", vec![])]);
        let report = list(&cfg, None, true, &against).await.unwrap();
        let export = |name: &str, description: Option<&str>| FolderExport { name: name.into(), description: description.map(str::to_string) };
        let want = vec![
            InterfaceFolder { name: "delegation-vm".into(), estate: "acme".into(), exports: vec![export("instance_name", Some("The VM's name")), export("zone", None)] },
            InterfaceFolder { name: "vm-users".into(), estate: "acme".into(), exports: Vec::new() },
        ];
        assert_eq!(report.interface_folders, want);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["interface_folders"][0],
            serde_json::json!({ "name": "delegation-vm", "estate": "acme", "exports": [
                { "name": "instance_name", "description": "The VM's name" },
                { "name": "zone", "description": null }
            ] })
        );
        assert!(render_list(&report).contains("delegation-vm, written by acme, exports instance_name, zone"), "{}", render_list(&report));
        let _ = std::fs::remove_dir_all(&dir);
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
