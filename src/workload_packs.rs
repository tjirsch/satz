//! Workload packs: the packs a workload's estate takes one at a time into its
//! `packs_dir`, beside the packs it wrote itself (ADR 0083).
//!
//! A workload pack is one `.satz` file with a `pack <id> version "…"` header. Its id
//! is its path under the `packs/` tree it comes from, without the extension. What
//! decides whose a file in `packs_dir` is, is the origin record `origin.toml` there:
//! a pack with a row was fetched, a pack without one is the estate's own and no
//! command here writes it.

use std::path::{Path, PathBuf};

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::fsx;
use crate::settings::ToolConfig;

/// The origin record's file name, inside `packs_dir`.
pub(crate) const RECORD: &str = "origin.toml";

const RECORD_HEADER: &str = "# Written by `satz get-pack` and `satz update-pack`: one row per pack fetched into this folder.\n\
                             # A pack with a row is updated by `satz update-pack`; a pack without one is this\n\
                             # repository's own and is never written. `satz detach-pack <id>` removes a row.\n\n";

/// One fetched pack, as the origin record holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// the pack's path under `packs/`, without `.satz`
    pub id: String,
    /// where it was fetched from: `github:<owner>/<repo>`, or `dir` for a folder
    pub source: String,
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
/// row that repeats an id and a row whose id is no pack id are errors naming the file.
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
    }
    Ok(record.packs)
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
        return Err(format!("`{id}` is no pack id — an id is the pack's path under packs/ without `.satz`: letters, digits, `_`, `-` and `.`, in segments separated by `/`"));
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

/// Where packs are fetched from.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// the `packs/` tree of the satz repository
    Github,
    /// a folder holding packs, `--from`
    Dir(PathBuf),
}

impl Source {
    pub(crate) fn new(from: Option<PathBuf>) -> Result<Self, String> {
        match from {
            None => Ok(Source::Github),
            Some(dir) if dir.is_dir() => Ok(Source::Dir(dir)),
            Some(dir) => Err(format!("--from {}: no such folder", dir.display())),
        }
    }

    /// What a row records as its source. A folder is recorded as `dir`, never as its
    /// path: the path is one machine's.
    fn label(&self) -> String {
        match self {
            Source::Github => format!("github:{}", crate::github::REPO),
            Source::Dir(_) => "dir".to_string(),
        }
    }

    fn describe(&self) -> String {
        match self {
            Source::Github => format!("github.com/{}", crate::github::REPO),
            Source::Dir(dir) => dir.display().to_string(),
        }
    }

    async fn fetch(&self, id: &str) -> Result<Option<String>, String> {
        match self {
            Source::Github => crate::github::fetch_pack(id).await,
            Source::Dir(dir) => {
                let path = pack_file(dir, id);
                if !path.is_file() {
                    return Ok(None);
                }
                fsx::read_to_string(&path).map(Some).map_err(|e| format!("{}: {}", path.display(), e))
            }
        }
    }

    async fn list(&self) -> Result<Vec<String>, String> {
        match self {
            Source::Github => crate::github::list_packs().await,
            Source::Dir(dir) => local_ids(dir),
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

/// What the source says about one pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Upstream<'a> {
    /// the source was not asked: the row names another kind of source than this run's
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
    /// fetched from another kind of source than this run reads
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
    /// where the packs came from
    pub source: String,
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

/// Fetch one pack, or every pack the source holds that `packs_dir` lacks, and record
/// where each came from. A pack `packs_dir` already holds is never written: one named
/// is refused, and with `all` it is listed as present.
pub(crate) async fn get(runtime: &ToolConfig, source: &Source, which: &Which) -> Result<GetReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    let mut report = GetReport { source: source.describe(), installed: Vec::new(), present: Vec::new() };

    let ids = match which {
        Which::One(id) => {
            let file = pack_file(packs_dir, id);
            if file.exists() {
                let how = if rows.iter().any(|r| &r.id == id) {
                    format!("it was fetched before — `satz update-pack {id}` brings it up to date")
                } else {
                    "it has no recorded origin, so it is this repository's own pack and is not overwritten".to_string()
                };
                return Err(format!("{} is already there: {how}", file.display()));
            }
            vec![id.clone()]
        }
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
            return Err(format!("{} holds no pack `{id}` — `satz packs` lists the packs it holds", source.describe()));
        };
        let version = version_of(&id, source, &text)?;
        let file = pack_file(packs_dir, &id);
        if let Some(parent) = file.parent() {
            fsx::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fsx::write_verbatim(&file, text.as_bytes()).map_err(|e| e.to_string())?;
        rows.retain(|r| r.id != id);
        rows.push(Row { id: id.clone(), source: source.label(), version: version.clone(), sha256: sha256(&text) });
        // the record follows every file as it lands: a later pack that fails leaves
        // the earlier ones recorded
        write_record(packs_dir, rows.clone())?;
        report.installed.push(Installed { id, version, path: file.display().to_string(), use_line });
    }
    Ok(report)
}

pub(crate) fn render_get(r: &GetReport) -> String {
    let mut out = String::new();
    for p in &r.installed {
        out.push_str(&format!("installed {} {} from {} → {}\n    {}\n", p.id, p.version, r.source, p.path, p.use_line));
    }
    if !r.present.is_empty() {
        out.push_str(&format!("already there, left as they are: {}\n", r.present.join(", ")));
    }
    if r.installed.is_empty() {
        out.push_str(&format!("nothing to install: every pack of {} is already there\n", r.source));
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
    /// recorded from another kind of source than this run reads: not looked at
    NotChecked,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct UpdateRow {
    pub id: String,
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
    pub source: String,
    pub packs: Vec<UpdateRow>,
    /// a pack is edited or gone: the command exits 1
    pub attention: bool,
}

/// Bring one recorded pack, or every recorded pack, up to date with the source. A pack
/// unchanged since it was fetched is overwritten when the source holds another file; a
/// pack changed here is left alone unless `take_upstream`; a pack without a row is the
/// repository's own and is not looked at.
pub(crate) async fn update(runtime: &ToolConfig, source: &Source, which: &Which, report_only: bool, take_upstream: bool) -> Result<UpdateReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let mut rows = read_record(packs_dir)?;
    let label = source.label();

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
            if row.source != label {
                return Err(format!(
                    "`{id}` was fetched from `{}`, and this run reads `{label}` — {}",
                    row.source,
                    if row.source == "dir" { "pass the folder with `--from <dir>`" } else { "leave `--from` out" }
                ));
            }
            vec![row.clone()]
        }
        Which::All => rows.clone(),
    };

    let mut report = UpdateReport { source: source.describe(), packs: Vec::new(), attention: false };
    for row in targets {
        let local = recorded_text(packs_dir, &row)?;
        let version = crate::presets::pack_version(&local).unwrap_or_else(|| row.version.clone());
        let fetched = if row.source == label { Some(source.fetch(&row.id).await?) } else { None };
        let upstream = match &fetched {
            None => Upstream::NotAsked,
            Some(None) => Upstream::Missing,
            Some(Some(text)) => Upstream::Found(text),
        };
        let upstream_version = match upstream {
            Upstream::Found(text) => Some(version_of(&row.id, source, text)?),
            _ => None,
        };
        let state = classify(Some(&row), Some(&local), upstream).expect("a recorded pack with a local file has a state");
        let mut out = UpdateRow { id: row.id.clone(), action: Action::Current, version, upstream_version: upstream_version.clone(), diff: None, note: None };

        // the one write: the source's file over the local one, and the row after it
        let take = |text: &str, rows: &mut Vec<Row>| -> Result<(), String> {
            fsx::write_verbatim(pack_file(packs_dir, &row.id), text.as_bytes()).map_err(|e| e.to_string())?;
            for r in rows.iter_mut().filter(|r| r.id == row.id) {
                r.version = upstream_version.clone().unwrap_or_default();
                r.sha256 = sha256(text);
            }
            write_record(packs_dir, rows.clone())
        };

        match (state, upstream) {
            (State::Current, _) => {}
            (State::NotChecked, _) => {
                out.action = Action::NotChecked;
                out.note = Some(format!(
                    "fetched from `{}`, and this run reads `{label}` — {}",
                    row.source,
                    if row.source == "dir" { "pass the folder with `--from <dir>`" } else { "run without `--from`" }
                ));
            }
            (State::Gone, _) => {
                out.action = Action::Gone;
                out.note = Some(format!("{} holds it no longer — keep it as this repository's own with `satz detach-pack {}`", source.describe(), row.id));
                report.attention = true;
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
                    Upstream::Missing => format!("changed here since it was fetched, and {} holds it no longer — `satz detach-pack {}` keeps it as this repository's own", source.describe(), row.id),
                    Upstream::NotAsked => format!("changed here since it was fetched; it came from `{}`, which this run does not read", row.source),
                });
                report.attention = true;
            }
            (state, upstream) => unreachable!("{state:?} with {upstream:?}"),
        }
        report.packs.push(out);
    }
    Ok(report)
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
        out.push_str(&format!("{}  {}  {}\n", p.id, arrow, what));
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

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct PackRow {
    pub id: String,
    pub state: State,
    /// the version the local file states; none for a pack that is only available
    pub version: Option<String>,
    /// the version the source's file states, where the source holds the pack
    pub upstream_version: Option<String>,
    /// where a fetched pack came from
    pub source: Option<String>,
    /// the line an estate reads the pack with; none for a pack that is only available
    pub use_line: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct PacksReport {
    pub packs_dir: String,
    pub source: String,
    pub packs: Vec<PackRow>,
}

/// Every pack in `packs_dir` with its state, and every pack the source holds that
/// `packs_dir` does not.
pub(crate) async fn list(runtime: &ToolConfig, source: &Source) -> Result<PacksReport, String> {
    let packs_dir = Path::new(&runtime.packs_dir);
    let rows = read_record(packs_dir)?;
    for row in &rows {
        recorded_text(packs_dir, row)?;
    }
    let label = source.label();
    let local = local_ids(packs_dir)?;
    let upstream_ids = source.list().await?;

    let mut packs = Vec::new();
    for id in &local {
        let path = pack_file(packs_dir, id);
        let text = fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let row = rows.iter().find(|r| &r.id == id);
        let fetched = match row {
            Some(r) if r.source == label => Some(source.fetch(id).await?),
            _ => None,
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
            version: crate::presets::pack_version(&text),
            upstream_version: match upstream {
                Upstream::Found(t) => crate::presets::pack_version(t),
                _ => None,
            },
            source: row.map(|r| r.source.clone()),
            use_line: use_path(runtime, id).ok().map(|p| format!("use \"{p}\"")),
        });
    }
    for id in upstream_ids.iter().filter(|id| !local.contains(id)) {
        let text = source.fetch(id).await?;
        packs.push(PackRow {
            id: id.clone(),
            state: State::Available,
            version: None,
            upstream_version: text.as_deref().and_then(crate::presets::pack_version),
            source: Some(label.clone()),
            use_line: None,
        });
    }
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(PacksReport { packs_dir: packs_dir.display().to_string(), source: source.describe(), packs })
}

pub(crate) fn render_list(r: &PacksReport) -> String {
    let mut out = format!("packs in {} against {}\n", r.packs_dir, r.source);
    if r.packs.is_empty() {
        out.push_str("  none here, and the source holds none\n");
        return out;
    }
    let width = r.packs.iter().map(|p| p.id.len()).max().unwrap_or(0);
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
        out.push_str(&format!("  {:width$}  {:10}  {}\n", p.id, version, state));
    }
    out
}

// ---------------------------------------------------------------------------
// detach-pack
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct Detached {
    pub id: String,
    /// where it had been fetched from
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
        Row { id: "demo".into(), source: "dir".into(), version: "1.0".into(), sha256: sha256(text) }
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

        std::fs::write(&path, "[[pack]]\nid = \"demo\"\nsource = \"dir\"\nversion = \"1.0\"\nsha256 = \"x\"\n".repeat(2)).unwrap();
        assert!(read_record(&dir).unwrap_err().contains("has two rows"));

        write_record(&dir, Vec::new()).unwrap();
        assert!(!path.exists(), "a record without rows is no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
