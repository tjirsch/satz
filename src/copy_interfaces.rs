//! `satz copy-interfaces`: a workload estate takes the interface folders it reads from the
//! organization's estate (ADR 0111). The folders are the ones its `use` lines read under
//! `interfaces/incoming/`; each is copied whole from the organization's
//! `interfaces/outgoing/`, after the estate is transpiled against the new folders, and only
//! when that check passes. Nothing outside `interfaces/incoming/<folder>/` is written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::fsx;
use crate::interface::{INCOMING, OUTGOING, SATZ_DIR, SATZ_FILE};
use crate::settings::ToolConfig;

type BoxErr = Box<dyn std::error::Error>;

/// How deep `use` lines are followed through the files they read.
const MAX_DEPTH: usize = 32;

/// Files an operating system or an editor puts into a folder: never copied, and not
/// reported as a change. `.DS_Store` (macOS Finder), `Thumbs.db` (Windows Explorer), a
/// name ending in `~` (an editor's backup) and `.<name>.swp` (vim's swap file).
pub(crate) fn left_out(name: &str) -> bool {
    name == ".DS_Store" || name == "Thumbs.db" || name.ends_with('~') || (name.starts_with('.') && name.ends_with(".swp"))
}

/// `p` made absolute, with `.` and `..` resolved by its text: a path that does not exist
/// yet compares with one that does. Symbolic links are not followed.
pub(crate) fn lexical(p: &Path) -> std::io::Result<PathBuf> {
    let abs = std::path::absolute(p)?;
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// The interface folders an estate reads under `incoming`, each with the `use` paths that
/// name it. A `use` path resolves as the transpile resolves it — beside the estate, then in
/// each include dir — and a file it reads that is no interface file is followed for the
/// `use` lines it holds. A path under `incoming` names the folder whether it exists or not:
/// the copy is what brings it.
pub(crate) fn folders_read(estate: &Path, include_dirs: &[String], incoming: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    let base = estate.parent().unwrap_or(Path::new(".")).to_path_buf();
    let incoming = lexical(incoming).map_err(|e| format!("{}: {}", incoming.display(), e))?;
    let src = fsx::read_to_string(estate).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    walk(&src, &estate.display().to_string(), &base, include_dirs, &incoming, &mut out, &mut seen, 0)?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    src: &str,
    label: &str,
    base: &Path,
    include_dirs: &[String],
    incoming: &Path,
    out: &mut BTreeMap<String, Vec<String>>,
    seen: &mut BTreeSet<PathBuf>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: `use` nested more than {} deep — cyclic?", label, MAX_DEPTH));
    }
    let parsed = satz_core::satz::parse(src).map_err(|e| format!("{}:{}: {}", label, e.line, e.msg))?;
    let mut paths = Vec::new();
    uses(&parsed.items, &mut paths);
    for (path, line) in paths {
        let mut candidates = vec![base.join(&path)];
        candidates.extend(include_dirs.iter().map(|d| Path::new(d).join(&path)));
        for c in candidates {
            let l = lexical(&c).map_err(|e| format!("{}: {}", c.display(), e))?;
            if let Ok(rest) = l.strip_prefix(incoming) {
                let mut parts = rest.components();
                match (parts.next(), parts.next()) {
                    (Some(Component::Normal(f)), Some(_)) => out.entry(f.to_string_lossy().into_owned()).or_default().push(path.clone()),
                    _ => {
                        return Err(format!(
                            "{}:{}: use \"{}\" reads {} itself, not a file inside one of its interface folders",
                            label,
                            line,
                            path,
                            incoming.display()
                        ))
                    }
                }
                break;
            }
            if c.is_file() {
                if path.ends_with(".satz") && seen.insert(l) {
                    let text = fsx::read_to_string(&c).map_err(|e| e.to_string())?;
                    walk(&text, &path, base, include_dirs, incoming, out, seen, depth + 1)?;
                }
                break;
            }
        }
    }
    Ok(())
}

/// Every `use` path of a file, with its line, in the blocks too.
fn uses(items: &[satz_core::satz::Entry], out: &mut Vec<(String, usize)>) {
    for e in items {
        match e {
            satz_core::satz::Entry::Use { path, line, .. } => out.push((path.clone(), *line)),
            satz_core::satz::Entry::Map { body, .. } => uses(body, out),
            _ => {}
        }
    }
}

/// Where the organization's estate keeps what it publishes.
pub(crate) struct Source {
    pub config: PathBuf,
    pub outgoing: PathBuf,
}

/// The organization's estate `--from` names — its folder, its `config.toml`, or its estate
/// file, whose config is the nearest `config.toml` at or above it — and the outgoing half of
/// the `interfaces_dir` that config names.
pub(crate) fn source(from: &Path) -> Result<Source, String> {
    const WHAT: &str = "name the organization's estate folder (the one that holds its config.toml), its config.toml or its estate file";
    let config = if from.is_dir() {
        let c = from.join("config.toml");
        if !c.is_file() {
            return Err(format!("--from {}: no config.toml in it — {}", from.display(), WHAT));
        }
        c
    } else if from.is_file() {
        match from.extension().and_then(|e| e.to_str()) {
            Some("toml") => from.to_path_buf(),
            Some("satz") => {
                let at = lexical(from).map_err(|e| format!("--from {}: {}", from.display(), e))?;
                at.ancestors()
                    .skip(1)
                    .map(|d| d.join("config.toml"))
                    .find(|c| c.is_file())
                    .ok_or_else(|| format!("--from {}: no config.toml at or above it — {}", from.display(), WHAT))?
            }
            _ => return Err(format!("--from {}: neither a config.toml nor an estate file — {}", from.display(), WHAT)),
        }
    } else {
        return Err(format!("--from {}: no such file or folder — {}", from.display(), WHAT));
    };
    let tool = crate::settings::parse_tool_config(&config).map_err(|e| format!("{}: {}", config.display(), e))?;
    let dir = config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let rc = crate::settings::resolved_config(&tool, &dir);
    let outgoing = Path::new(&rc.interfaces_dir).join(OUTGOING);
    if !outgoing.is_dir() {
        return Err(format!(
            "{} has no {} — `satz transpile` of the organization's estate writes it, one folder per interface it publishes",
            config.display(),
            outgoing.display()
        ));
    }
    Ok(Source { config, outgoing })
}

/// The folders directly under `dir`, by name, those starting with a dot left out.
fn folders_in(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> = fsx::read_dir_entries(dir)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    Ok(out)
}

/// Every file under `root` by its path relative to it, `/`-separated, with its bytes; and
/// the files left out (`left_out`).
type Tree = BTreeMap<String, Vec<u8>>;

fn tree(root: &Path) -> Result<(Tree, Vec<String>), String> {
    fn go(root: &Path, at: &Path, files: &mut Tree, skipped: &mut Vec<String>) -> Result<(), String> {
        let mut entries = fsx::read_dir_entries(at).map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = fsx::slash(p.strip_prefix(root).map_err(|e| e.to_string())?);
            if left_out(&e.file_name().to_string_lossy()) {
                skipped.push(rel);
            } else if p.is_dir() {
                go(root, &p, files, skipped)?;
            } else {
                let bytes = std::fs::read(&p).map_err(|e| format!("{}: {}", p.display(), e))?;
                files.insert(rel, bytes);
            }
        }
        Ok(())
    }
    let (mut files, mut skipped) = (Tree::new(), Vec::new());
    go(root, root, &mut files, &mut skipped)?;
    Ok((files, skipped))
}

/// One folder as the copy finds it.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    /// not under `interfaces/incoming/` yet
    New,
    Changed,
    Unchanged,
}

/// One line of what changed in an interface's values: the `CHANGES.md` a transpile writes,
/// computed from the two copies of `satz/interface.satz`.
#[derive(Debug, Serialize)]
pub(crate) struct ValueChange {
    pub interface: String,
    /// a reader of the value has something to do
    pub todo: bool,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct FolderReport {
    pub folder: String,
    pub state: State,
    /// files in the new folder
    pub files: usize,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub values: Vec<ValueChange>,
    /// files of the organization's folder that are not copied (`left_out`)
    pub left_out: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Check {
    /// the estate transpiles with the new folders
    pub transpiles: bool,
    pub findings: Vec<crate::findings::Finding>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CopyReport {
    pub estate: String,
    /// the organization's `interfaces/outgoing/`
    pub from: String,
    pub incoming: String,
    pub folders: Vec<FolderReport>,
    /// folders under `interfaces/incoming/` the estate reads no file of: reported, left as they are
    pub unread: Vec<String>,
    pub check: Check,
    /// the folders were written: false with `--report-only`, when the check fails and when
    /// every folder is the same as the organization's
    pub written: bool,
}

/// What `new` changes against `old`, file by file and in the values of each interface the
/// two have in common.
fn compare(folder: &str, old: Option<&Tree>, new: &Tree, left: Vec<String>) -> Result<FolderReport, String> {
    let Some(old) = old else {
        return Ok(FolderReport { folder: folder.to_string(), state: State::New, files: new.len(), added: new.keys().cloned().collect(), removed: Vec::new(), changed: Vec::new(), values: Vec::new(), left_out: left });
    };
    let added: Vec<String> = new.keys().filter(|k| !old.contains_key(*k)).cloned().collect();
    let removed: Vec<String> = old.keys().filter(|k| !new.contains_key(*k)).cloned().collect();
    let changed: Vec<String> = new.iter().filter(|(k, v)| old.get(*k).is_some_and(|o| o != *v)).map(|(k, _)| k.clone()).collect();
    let suffix = format!("/{}/{}", SATZ_DIR, SATZ_FILE);
    let mut values = Vec::new();
    for key in &changed {
        let Some(interface) = key.strip_suffix(&suffix).filter(|i| !i.contains('/')) else { continue };
        let new_file = interface_file(&new[key]).map_err(|e| format!("{}/{}: {} — the organization's folder does not read", folder, key, e))?;
        match interface_file(&old[key]) {
            Ok(old_file) => values.extend(crate::interface_changes::changes(&old_file, &new_file).into_iter().map(|c| ValueChange { interface: interface.to_string(), todo: c.todo, text: c.text })),
            Err(e) => values.push(ValueChange { interface: interface.to_string(), todo: false, text: format!("the previous copy does not read ({}), so its values are not compared", e) }),
        }
    }
    let state = if added.is_empty() && removed.is_empty() && changed.is_empty() { State::Unchanged } else { State::Changed };
    Ok(FolderReport { folder: folder.to_string(), state, files: new.len(), added, removed, changed, values, left_out: left })
}

fn interface_file(bytes: &[u8]) -> Result<satz_core::satz::InterfaceFile, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    satz_core::satz::parse(text).map_err(|e| format!("line {}: {}", e.line, e.msg))?.interface_file.ok_or_else(|| "no interface file".to_string())
}

/// `satz copy-interfaces <estate> --from <the organization's estate>`.
pub(crate) fn run(estate: &Path, from: &Path, report_only: bool, tool: &ToolConfig, rc: &ToolConfig) -> Result<CopyReport, BoxErr> {
    let incoming = Path::new(&rc.interfaces_dir).join(INCOMING);
    let read = folders_read(estate, &rc.include_dirs, &incoming)?;
    if read.is_empty() {
        return Err(format!(
            "{} reads no interface folder under {}, so there is nothing to copy — a workload estate reads one with \
             `use \"interfaces/incoming/<folder>/<interface>/satz/interface.satz\"`",
            estate.display(),
            incoming.display()
        )
        .into());
    }
    let src = source(from)?;
    let own = lexical(&Path::new(&rc.interfaces_dir).join(OUTGOING))?;
    if lexical(&src.outgoing)? == own {
        return Err(format!("--from {}: that is this estate's own config, and an estate copies its interface folders from the organization's estate", src.config.display()).into());
    }
    let available = folders_in(&src.outgoing)?;
    let missing: Vec<&String> = read.keys().filter(|f| !available.contains(f)).collect();
    if !missing.is_empty() {
        return Err(format!(
            "{} reads interface folders that {} does not hold: {} ({}). It holds {}. Nothing was copied",
            estate.display(),
            src.outgoing.display(),
            missing.iter().map(|f| format!("`{}`", f)).collect::<Vec<_>>().join(", "),
            missing.iter().map(|f| format!("`{}`", read[*f].join("`, `"))).collect::<Vec<_>>().join("; "),
            if available.is_empty() { "none".to_string() } else { available.iter().map(|f| format!("`{}`", f)).collect::<Vec<_>>().join(", ") }
        )
        .into());
    }
    let mut folders = Vec::new();
    let mut trees = Vec::new();
    for f in read.keys() {
        let (new, left) = tree(&src.outgoing.join(f))?;
        let target = incoming.join(f);
        let old = if target.is_dir() {
            Some(tree(&target)?.0)
        } else if target.exists() {
            return Err(format!("{} is no folder: an interface folder is copied there", target.display()).into());
        } else {
            None
        };
        folders.push(compare(f, old.as_ref(), &new, left)?);
        trees.push(new);
    }
    let mut unread: Vec<String> = Vec::new();
    if incoming.is_dir() {
        for e in fsx::read_dir_entries(&incoming)? {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with('.') && !read.contains_key(&n) {
                unread.push(n);
            }
        }
        unread.sort();
    }
    // the check, before anything is written: the estate transpiled with each folder it reads
    // taken from the organization's outgoing half
    let redirect = read.keys().map(|f| Ok((lexical(&incoming.join(f))?, src.outgoing.join(f)))).collect::<std::io::Result<Vec<_>>>()?;
    let (transpiles, findings) = crate::check_redirected(estate, tool, rc, redirect)?;
    let mut report = CopyReport {
        estate: estate.display().to_string(),
        from: src.outgoing.display().to_string(),
        incoming: incoming.display().to_string(),
        folders,
        unread,
        check: Check { transpiles, findings },
        written: false,
    };
    if !transpiles || report_only {
        return Ok(report);
    }
    let to_write: Vec<(&str, &Tree)> = report.folders.iter().zip(&trees).filter(|(f, _)| f.state != State::Unchanged).map(|(f, t)| (f.folder.as_str(), t)).collect();
    if to_write.is_empty() {
        return Ok(report);
    }
    swap(&incoming, &to_write)?;
    report.written = true;
    Ok(report)
}

/// Beside `interfaces/incoming/<folder>/`: where the new copy is written, and where the old
/// one stands while the two are swapped.
fn staged(incoming: &Path, folder: &str) -> PathBuf {
    incoming.join(format!(".{}.satz-copy", folder))
}
fn aside(incoming: &Path, folder: &str) -> PathBuf {
    incoming.join(format!(".{}.satz-old", folder))
}

fn write_tree(root: &Path, files: &Tree) -> std::io::Result<()> {
    fsx::create_dir_all(root)?;
    for (rel, bytes) in files {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fsx::create_dir_all(parent)?;
        }
        fsx::write_verbatim(&p, bytes)?;
    }
    Ok(())
}

/// Write every folder beside its target, then swap each in: the old one moved aside, the new
/// one renamed over, the old ones removed once all are in. A failure before the last swap
/// puts every old folder back and removes the new copies.
fn swap(incoming: &Path, folders: &[(&str, &Tree)]) -> Result<(), BoxErr> {
    for (f, _) in folders {
        for p in [staged(incoming, f), aside(incoming, f)] {
            if std::fs::symlink_metadata(&p).is_ok() {
                return Err(format!(
                    "{} is there: a copy-interfaces run that did not finish left it. Compare it with {}, remove it, and run again; nothing was copied",
                    p.display(),
                    incoming.join(f).display()
                )
                .into());
            }
        }
    }
    fsx::create_dir_all(incoming)?;
    let remove_staged = |upto: usize| -> Vec<String> {
        folders[..upto].iter().filter_map(|(f, _)| fsx::remove_dir_all(staged(incoming, f)).err().map(|e| e.to_string())).collect()
    };
    for (i, (f, files)) in folders.iter().enumerate() {
        if let Err(e) = write_tree(&staged(incoming, f), files) {
            let left = remove_staged(i + 1);
            return Err(failed(format!("{} — nothing was copied", e), Vec::new(), left));
        }
    }
    // (folder, whether an old copy was moved aside)
    let mut swapped: Vec<(&str, bool)> = Vec::new();
    for (f, _) in folders {
        let (target, new, old) = (incoming.join(f), staged(incoming, f), aside(incoming, f));
        let had = target.exists();
        if had {
            if let Err(e) = std::fs::rename(&target, &old) {
                let (put_back, left) = undo(incoming, &swapped);
                let mut left = left;
                left.extend(remove_staged(folders.len()));
                return Err(failed(format!("moving {} aside: {} — nothing was copied", target.display(), e), put_back, left));
            }
        }
        if let Err(e) = std::fs::rename(&new, &target) {
            let mut current = swapped.clone();
            if had {
                current.push((f, true));
            }
            // the folder that failed has no new copy in place: only its old one goes back
            let (put_back, mut left) = undo_with(incoming, &current, Some(f));
            left.extend(remove_staged(folders.len()));
            return Err(failed(format!("moving the new copy to {}: {} — nothing was copied", target.display(), e), put_back, left));
        }
        swapped.push((f, had));
    }
    let left: Vec<String> = swapped
        .iter()
        .filter(|(_, had)| *had)
        .filter_map(|(f, _)| fsx::remove_dir_all(aside(incoming, f)).err().map(|e| format!("{} — the folders are copied; remove it", e)))
        .collect();
    if !left.is_empty() {
        return Err(left.join("\n").into());
    }
    Ok(())
}

fn undo(incoming: &Path, swapped: &[(&str, bool)]) -> (Vec<String>, Vec<String>) {
    undo_with(incoming, swapped, None)
}

/// Put back what `swapped` moved, the last first: the new copy goes back beside the folder
/// (removed with the staged copies), the old one returns. `only_old` names a folder whose new
/// copy never moved in.
fn undo_with(incoming: &Path, swapped: &[(&str, bool)], only_old: Option<&str>) -> (Vec<String>, Vec<String>) {
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for (f, had) in swapped.iter().rev() {
        let target = incoming.join(f);
        if only_old != Some(*f) {
            if let Err(e) = std::fs::rename(&target, staged(incoming, f)) {
                failed.push(format!("{}: {}", target.display(), e));
                continue;
            }
        }
        if *had {
            match std::fs::rename(aside(incoming, f), &target) {
                Ok(()) => done.push(format!("put back {}", target.display())),
                Err(e) => failed.push(format!("{}: {}", aside(incoming, f).display(), e)),
            }
        }
    }
    (done, failed)
}

fn failed(msg: String, done: Vec<String>, left: Vec<String>) -> BoxErr {
    let mut s = msg;
    if !done.is_empty() {
        s.push_str(&format!("\ncopy-interfaces {}", done.join(", ")));
    }
    if !left.is_empty() {
        s.push_str(&format!("\ncopy-interfaces could not undo: {}", left.join("; ")));
    }
    s.into()
}

/// The report as a person reads it.
pub(crate) fn render_text(r: &CopyReport) -> String {
    let mut s = format!("copy-interfaces — {} reads {} interface folder(s) from {}\n", r.estate, r.folders.len(), r.from);
    for f in &r.folders {
        match f.state {
            State::New => s.push_str(&format!("  {}: new — {} file(s)\n", f.folder, f.files)),
            State::Unchanged => s.push_str(&format!("  {}: unchanged\n", f.folder)),
            State::Changed => {
                s.push_str(&format!("  {}: changed — {} added, {} removed, {} changed\n", f.folder, f.added.len(), f.removed.len(), f.changed.len()));
                for (what, files) in [("added  ", &f.added), ("removed", &f.removed), ("changed", &f.changed)] {
                    for p in files {
                        s.push_str(&format!("    {} {}\n", what, p));
                    }
                }
            }
        }
        for v in &f.values {
            s.push_str(&format!("    {} `{}`: {}\n", if v.todo { "to do " } else { "value " }, v.interface, v.text));
        }
        for p in &f.left_out {
            s.push_str(&format!("    left out {}\n", p));
        }
    }
    for u in &r.unread {
        s.push_str(&format!("  warning: {}/{} is read by no `use` line of the estate; it is left as it is\n", r.incoming, u));
    }
    if r.check.transpiles {
        let warnings = r.check.findings.iter().filter(|f| f.severity == crate::findings::Severity::Warning).count();
        s.push_str(&format!("check: {} transpiles with these folders", r.estate));
        if warnings > 0 {
            s.push_str(&format!(", with {} warning(s) — `satz transpile --check` prints them", warnings));
        }
        s.push('\n');
    } else {
        s.push_str(&format!("check: {} does not transpile with these folders\n", r.estate));
    }
    let changed: Vec<&str> = r.folders.iter().filter(|f| f.state != State::Unchanged).map(|f| f.folder.as_str()).collect();
    if r.written {
        s.push_str(&format!("copied {} folder(s) into {}: {}\n", changed.len(), r.incoming, changed.join(", ")));
    } else if !r.check.transpiles {
        s.push_str(&format!("nothing was copied: {} is as it was\n", r.incoming));
    } else if changed.is_empty() {
        s.push_str("nothing to copy: every folder is the same as the organization's\n");
    } else {
        s.push_str(&format!("--report-only: nothing was written; {} folder(s) would be copied: {}\n", changed.len(), changed.join(", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("satz-copy-interfaces-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// The folders come from every `use` line under `interfaces/incoming/`, in a block and in a
    /// file the estate uses too, whether the folder is there yet or not; a pack outside it is
    /// no folder.
    #[test]
    fn the_folders_are_the_ones_the_use_lines_read_under_incoming() {
        let d = scratch("discover");
        put(&d.join("packs/team.satz"), "pack team\n\nuse \"interfaces/incoming/shared/shared/satz/interface.satz\"\n");
        put(&d.join("interfaces/incoming/delegation-books/delegation-books/satz/interface.satz"), "interface \"delegation-books\"\n");
        put(
            &d.join("satz/books.satz"),
            "estate books\n\n\
             use \"interfaces/incoming/delegation-books/delegation-books/satz/interface.satz\"\n\
             use \"interfaces/incoming/books-project/books-project/satz/interface.satz\"\n\
             use \"interfaces/incoming/books-project/core/satz/interface.satz\"\n\
             use \"packs/team.satz\"\n",
        );
        let include = vec![d.display().to_string(), d.join("satz").display().to_string()];
        let found = folders_read(&d.join("satz/books.satz"), &include, &d.join("interfaces/incoming")).unwrap();
        assert_eq!(found.keys().map(String::as_str).collect::<Vec<_>>(), ["books-project", "delegation-books", "shared"]);
        assert_eq!(found["books-project"].len(), 2, "{:?}", found);
        assert_eq!(found["shared"], ["interfaces/incoming/shared/shared/satz/interface.satz"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_estate_that_reads_no_folder_finds_none_and_a_file_directly_in_incoming_is_refused() {
        let d = scratch("none");
        put(&d.join("satz/org.satz"), "estate org\n\nuse \"presets/estate-core.satz\"\n");
        let include = vec![d.display().to_string()];
        assert!(folders_read(&d.join("satz/org.satz"), &include, &d.join("interfaces/incoming")).unwrap().is_empty());
        put(&d.join("satz/odd.satz"), "estate odd\n\nuse \"interfaces/incoming/loose.satz\"\n");
        let e = folders_read(&d.join("satz/odd.satz"), &include, &d.join("interfaces/incoming")).unwrap_err();
        assert!(e.contains("loose.satz") && e.contains("not a file inside one of its interface folders"), "{}", e);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn os_and_editor_files_are_left_out() {
        for n in [".DS_Store", "Thumbs.db", "README.md~", ".interface.satz.swp"] {
            assert!(left_out(n), "{}", n);
        }
        for n in ["README.md", "interface.satz", ".gitkeep", "CHANGES.md"] {
            assert!(!left_out(n), "{}", n);
        }
    }

    /// A file the new copy lacks is removed, one it adds is added, and a changed interface
    /// file says which values moved.
    #[test]
    fn the_comparison_names_each_file_and_each_value_that_changed() {
        let iface = |v: &str| format!("interface \"pay\"\n\ncentral {{\n  estate = \"e\"\n  organizations = [\"organizations/123456789012\"]\n}}\n\noutput \"project_id\" {{\n  value = \"{}\"\n}}\n", v).into_bytes();
        let old: Tree = [("pay/satz/interface.satz".to_string(), iface("acme-pay-001")), ("stale.txt".to_string(), b"x".to_vec())].into();
        let new: Tree = [("pay/satz/interface.satz".to_string(), iface("acme-pay-002")), ("pay/CHANGES.md".to_string(), b"y".to_vec())].into();
        let r = compare("pay", Some(&old), &new, vec![".DS_Store".into()]).unwrap();
        assert_eq!(r.state, State::Changed);
        assert_eq!(r.added, ["pay/CHANGES.md"]);
        assert_eq!(r.removed, ["stale.txt"]);
        assert_eq!(r.changed, ["pay/satz/interface.satz"]);
        assert_eq!(r.values.len(), 1);
        assert!(r.values[0].text.contains("`project_id` has a new value"), "{:?}", r.values);
        assert_eq!(compare("pay", Some(&new), &new, Vec::new()).unwrap().state, State::Unchanged);
        assert_eq!(compare("pay", None, &new, Vec::new()).unwrap().state, State::New);
    }

    #[test]
    fn the_source_is_found_from_the_folder_the_config_or_the_estate_file() {
        let d = scratch("source");
        put(&d.join("config.toml"), "yaml_dir = \"satz\"\ninterfaces_dir = \"published\"\n");
        put(&d.join("satz/org.satz"), "estate org\n");
        let e = source(&d).err().expect("no outgoing half yet");
        assert!(e.contains("satz transpile"), "{}", e);
        std::fs::create_dir_all(d.join("published/outgoing/pay")).unwrap();
        for from in [d.clone(), d.join("config.toml"), d.join("satz/org.satz")] {
            let s = source(&from).unwrap_or_else(|e| panic!("{}: {}", from.display(), e));
            assert!(s.outgoing.ends_with("published/outgoing"), "{}", s.outgoing.display());
        }
        let e = source(&d.join("satz")).err().expect("a folder without a config");
        assert!(e.contains("no config.toml in it"), "{}", e);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two folders swapped in: the old ones gone, the staged names gone, a stray name a run
    /// left refuses before anything is written.
    #[test]
    fn the_swap_replaces_each_folder_whole_and_a_leftover_refuses() {
        let d = scratch("swap");
        let incoming = d.join("incoming");
        put(&incoming.join("a/old.txt"), "old");
        let a: Tree = [("new.txt".to_string(), b"new".to_vec())].into();
        let b: Tree = [("b/x.txt".to_string(), b"x".to_vec())].into();
        swap(&incoming, &[("a", &a), ("b", &b)]).unwrap();
        assert!(!incoming.join("a/old.txt").exists() && incoming.join("a/new.txt").is_file() && incoming.join("b/b/x.txt").is_file());
        assert_eq!(folders_in(&incoming).unwrap(), ["a", "b"]);
        std::fs::create_dir_all(staged(&incoming, "a")).unwrap();
        let e = swap(&incoming, &[("a", &b)]).unwrap_err().to_string();
        assert!(e.contains(".a.satz-copy") && e.contains("nothing was copied"), "{}", e);
        assert!(incoming.join("a/new.txt").is_file(), "a refused swap changed the folder");
        let _ = std::fs::remove_dir_all(&d);
    }
}
