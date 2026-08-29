//! Preset drift detection: the `check-presets` command.
//!
//! Presets are read-only building blocks — all per-org values belong in the estate's
//! `params { … }` block (overridable defaults, first definition wins). Historically each
//! customer org edited its preset copies instead, which makes preset updates painful.
//! This module compares the local preset library against the pristine upstream one,
//! classifies every drifted file, and for the mechanically fixable class (changed
//! variable defaults) prints the exact override block to paste into the estate.
//!
//! The classification layer is pure and unit-tested; downloading and directory walking
//! are the only IO.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::include_processor::{parse_vars_entry, register_anchors};

type BoxErr = Box<dyn std::error::Error>;

/// The pristine upstream copy to compare against: an explicit `--pristine-dir`,
/// or one download per process.
///
/// All three preset commands come through here, and the download happens ONCE:
/// the fetch is cached for the life of the process, so a second caller inside
/// one run costs no API quota. That quota is 60 requests/hour unauthenticated
/// and shared with `self-update`, which is why every request is worth counting.
async fn pristine_source(pristine_dir: Option<PathBuf>) -> Result<PathBuf, BoxErr> {
    if let Some(dir) = pristine_dir {
        return Ok(dir);
    }
    if let Some(cached) = DOWNLOADED.get() {
        return Ok(cached.clone());
    }
    let tmp = std::env::temp_dir().join(format!("satz-pristine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let n = crate::github::download_presets(&tmp).await?;
    println!("Fetched {n} upstream preset file(s).");
    let _ = DOWNLOADED.set(tmp.clone());
    Ok(tmp)
}

static DOWNLOADED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

// ---------------------------------------------------------------------------
// Pure layer
// ---------------------------------------------------------------------------

/// One entry of a preset's top-level `variables:` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VarEntry {
    pub key: String,
    /// The value text exactly as written (everything after the anchor), trimmed.
    pub value: String,
}

/// A preset file split into its variables block and everything else.
#[derive(Debug)]
pub(crate) struct PresetParts {
    /// anchor name -> entry
    pub vars: BTreeMap<String, VarEntry>,
    /// Non-variables lines, comments and blanks dropped, trailing space trimmed.
    /// Comment-only edits are formatting, not drift.
    pub body: Vec<String>,
}

pub(crate) fn split_preset(text: &str) -> PresetParts {
    let mut vars: BTreeMap<String, VarEntry> = BTreeMap::new();
    let mut body = Vec::new();
    let mut in_vars_block = false;
    // The entry whose value is still being read, with the indent it was declared
    // at. A `variables:` entry may carry its value on the FOLLOWING, more-indented
    // lines — a list, or a block scalar. Keeping only the text on the anchor line
    // made every such default invisible: both sides had an empty value, the items
    // were never compared, and a pack a whole version behind reported "clean".
    // Real case: one estate on CIS v2.0 against upstream v2.1, whose only substantive
    // change was a fifth entry in a list default. See docs/presets-workflow.md.
    let mut open: Option<(String, usize)> = None;

    for line in text.lines() {
        let trimmed = line.trim_start();
        let at_top_level = !line.starts_with(' ') && !trimmed.is_empty() && !trimmed.starts_with('#');
        if at_top_level {
            in_vars_block = trimmed == "variables:";
            open = None;
            if in_vars_block {
                continue;
            }
        }
        if in_vars_block {
            if let Some((indent, key, anchor, rest)) = parse_vars_entry(line) {
                vars.insert(
                    anchor.to_string(),
                    VarEntry { key: key.to_string(), value: rest.trim().to_string() },
                );
                open = Some((anchor.to_string(), indent));
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // A more-indented line continues the open entry's value.
            if let Some((anchor, indent)) = &open {
                if line.len() - trimmed.len() > *indent {
                    if let Some(entry) = vars.get_mut(anchor) {
                        if !entry.value.is_empty() {
                            entry.value.push('\n');
                        }
                        entry.value.push_str(line.trim_end());
                    }
                    continue;
                }
            }
            open = None;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        body.push(line.trim_end().to_string());
    }

    PresetParts { vars, body }
}

/// The version a Satz pack declares in-file: `pack <name> version "2.1"`.
///
/// Filenames carry only the framework version, so this line is the ONLY place a
/// pack states which release it is. It is also the only staleness signal that
/// does not depend on the content comparison being complete, which is why
/// `check-presets` reports it rather than inferring "current" from "clean".
pub(crate) fn pack_version(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim_start).find(|l| l.starts_with("pack "))?;
    let after = &line[line.find(" version ")? + " version ".len()..];
    let rest = after.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// How a local preset relates to its pristine upstream version.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Drift {
    /// Byte-identical, or only comments/blank lines/formatting differ.
    Clean,
    /// Same resources, same variable set — only default values changed.
    /// The pairs are (anchor, local value); these become main-file overrides.
    VariablesOnly(Vec<(String, VarEntry)>),
    /// Resource bodies or the variable set itself differ — not mechanically migratable.
    Structural { summary: String },
}

pub(crate) fn classify(local: &str, pristine: &str) -> Drift {
    if local == pristine {
        return Drift::Clean;
    }
    let l = split_preset(local);
    let p = split_preset(pristine);

    if l.body != p.body {
        let changed = l.body.iter().filter(|line| !p.body.contains(line)).count()
            + p.body.iter().filter(|line| !l.body.contains(line)).count();
        return Drift::Structural {
            summary: format!("{} resource line(s) differ from upstream", changed),
        };
    }

    let l_anchors: BTreeSet<_> = l.vars.keys().cloned().collect();
    let p_anchors: BTreeSet<_> = p.vars.keys().cloned().collect();
    if l_anchors != p_anchors {
        let only_local: Vec<_> = l_anchors.difference(&p_anchors).cloned().collect();
        let only_upstream: Vec<_> = p_anchors.difference(&l_anchors).cloned().collect();
        return Drift::Structural {
            summary: format!(
                "variable set differs (local-only: [{}], upstream-only: [{}])",
                only_local.join(", "),
                only_upstream.join(", ")
            ),
        };
    }

    let changed: Vec<(String, VarEntry)> = l
        .vars
        .iter()
        .filter(|(anchor, entry)| p.vars.get(*anchor).map(|pe| &pe.value) != Some(&entry.value))
        .map(|(anchor, entry)| (anchor.clone(), entry.clone()))
        .collect();

    if changed.is_empty() {
        Drift::Clean // comments/formatting only
    } else {
        Drift::VariablesOnly(changed)
    }
}

/// Anchors a YAML-dialect estate defines itself (its overrides and shared variables).
pub(crate) fn anchors_defined_in_main(main_text: &str) -> BTreeSet<String> {
    let mut seen = std::collections::HashSet::new();
    for line in main_text.lines() {
        register_anchors(line, &mut seen);
    }
    seen.into_iter().collect()
}

// ---------------------------------------------------------------------------
// IO layer
// ---------------------------------------------------------------------------

fn walk_yaml_files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<(), BoxErr> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk_yaml_files(&path, base, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            out.push(path.strip_prefix(base)?.to_path_buf());
        }
    }
    Ok(())
}

/// Every preset SOURCE under `dir`: `.yaml` packs and `.satz` packs alike.
///
/// `walk_yaml_files` only ever saw `.yaml`, which is why `check-presets` silently
/// skipped the entire Satz fleet — a pack it never collected is a pack it never
/// compares, and "nothing compared" printed as "clean".
fn walk_preset_sources(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<(), BoxErr> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk_preset_sources(&path, base, out)?;
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        // `.gen.yaml` / `.gen.claims.yaml` are transpile artifacts, not sources.
        if name.contains(".gen.") {
            continue;
        }
        if name.ends_with(".yaml") || name.ends_with(".satz") {
            out.push(path.strip_prefix(base)?.to_path_buf());
        }
    }
    Ok(())
}

/// Which preset files does this estate actually use, relative to `presets_dir`?
///
/// A `.satz` estate declares `use "presets/…"`; the YAML dialect uses `!include`.
/// The old code only knew the second, so for a Satz estate the "included" set came
/// back EMPTY and every drift was reported against nothing.
///
/// Note: `use … when <param>` is followed unconditionally here. That can mark a
/// conditionally-disabled pack as included — deliberately the safe direction, since
/// over-reporting drift is recoverable and under-reporting it is exactly the bug
/// this function exists to fix.
fn used_preset_files(
    input: &Path,
    presets_dir: &str,
    include_dirs: &[String],
) -> Result<BTreeSet<PathBuf>, BoxErr> {
    let canon_presets = std::fs::canonicalize(presets_dir)
        .unwrap_or_else(|_| PathBuf::from(presets_dir));
    let mut used = BTreeSet::new();

    if input.extension().and_then(|e| e.to_str()) != Some("satz") {
        let include_paths: Vec<PathBuf> = include_dirs.iter().map(PathBuf::from).collect();
        let (_text, bindings) = crate::include_processor::process_includes_with_ops(input, &include_paths)?;
        for b in &bindings {
            if let Ok(c) = std::fs::canonicalize(&b.path) {
                if let Ok(rel) = c.strip_prefix(&canon_presets) {
                    used.insert(rel.to_path_buf());
                }
            }
        }
        return Ok(used);
    }

    // Walk the `use` graph the same way the compiler resolves it: relative to the
    // using file first, then the configured include dirs.
    let mut queue = vec![input.to_path_buf()];
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    while let Some(path) = queue.pop() {
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(canon.clone()) {
            continue;
        }
        if let Ok(rel) = canon.strip_prefix(&canon_presets) {
            used.insert(rel.to_path_buf());
        }
        let Ok(src) = crate::fsx::read_to_string(&path) else { continue };
        let Ok(file) = satz_core::satz::parse(&src) else { continue };
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        for dep in satz_core::satz::use_paths(&file) {
            let mut candidates = vec![parent.join(&dep)];
            candidates.extend(include_dirs.iter().map(|d| Path::new(d).join(&dep)));
            if let Some(found) = candidates.into_iter().find(|c| c.exists()) {
                queue.push(found);
            }
        }
    }
    Ok(used)
}

/// Semantic drift between two preset sources, in their canonical form.
///
/// `.satz` packs are compared as parsed: `satz::canonical_parts` prints the AST
/// without comments, formatting or the pack version, split into params
/// (the "variables" half) and everything else (the "structural" half) —
/// exactly the distinction the report needs. This is the SAME canonical
/// comparison `merge-presets` makes, so the two commands cannot disagree about
/// whether a pack drifted.
fn classify_source(rel: &Path, local: &str, pristine: &str) -> Drift {
    let is_satz = rel.extension().and_then(|e| e.to_str()) == Some("satz");
    if !is_satz {
        return classify(local, pristine);
    }
    if local == pristine {
        return Drift::Clean;
    }
    let (l, p) = match (satz_core::satz::parse(local), satz_core::satz::parse(pristine)) {
        (Ok(l), Ok(p)) => (satz_core::satz::canonical_parts(&l), satz_core::satz::canonical_parts(&p)),
        // A pack that no longer parses is drift the operator must see, not a skip.
        _ => return Drift::Structural { summary: "differs from upstream and does not parse".into() },
    };
    if l.body != p.body {
        let lb: BTreeSet<&str> = l.body.lines().collect();
        let pb: BTreeSet<&str> = p.body.lines().collect();
        let changed = lb.symmetric_difference(&pb).count();
        return Drift::Structural { summary: format!("{} resource line(s) differ from upstream", changed) };
    }
    let l_names: BTreeSet<&str> = l.params.iter().map(|(n, _)| n.as_str()).collect();
    let p_names: BTreeSet<&str> = p.params.iter().map(|(n, _)| n.as_str()).collect();
    if l_names != p_names {
        let only_local: Vec<_> = l_names.difference(&p_names).cloned().collect();
        let only_upstream: Vec<_> = p_names.difference(&l_names).cloned().collect();
        return Drift::Structural {
            summary: format!(
                "param set differs (local-only: [{}], upstream-only: [{}])",
                only_local.join(", "),
                only_upstream.join(", ")
            ),
        };
    }
    let pristine_vals: std::collections::BTreeMap<&str, &str> =
        p.params.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    let changed: Vec<(String, VarEntry)> = l
        .params
        .iter()
        .filter(|(n, v)| pristine_vals.get(n.as_str()) != Some(&v.as_str()))
        .map(|(n, v)| (n.clone(), VarEntry { key: n.clone(), value: v.clone() }))
        .collect();
    if changed.is_empty() {
        Drift::Clean
    } else {
        Drift::VariablesOnly(changed)
    }
}

/// The `X.local.<ext>` name beside a pristine `X.<ext>`.
fn fork_sibling(rel: &Path) -> PathBuf {
    let name = rel.file_name().unwrap_or_default().to_string_lossy();
    match name.rsplit_once('.') {
        Some((stem, ext)) => rel.with_file_name(format!("{stem}.local.{ext}")),
        None => rel.to_path_buf(),
    }
}

/// Render a variable's value for the report. A default written as a list spans
/// several lines; printing it inline turns the remedy into an unreadable blob.
fn indented(value: &str) -> String {
    if value.contains('\n') {
        let mut out = String::new();
        for line in value.lines() {
            out.push('\n');
            out.push_str("        ");
            out.push_str(line.trim());
        }
        out
    } else {
        format!(" {value}")
    }
}

/// The per-file detail of a drift verdict, shared by the STALE and EDITED arms
/// so a version gap and a local edit are described the same way — only the
/// headline and the remedy differ.
fn print_drift_detail(drift: &Drift, rel: &Path, main_anchors: &BTreeSet<String>, stale: bool) {
    match drift {
        Drift::Clean => {}
        // A STALE pack's local values are the OLD defaults. Telling the operator
        // to pin them in the estate would freeze exactly what they are trying to
        // move off, so name what moved and stop there — the remedy is adoption.
        Drift::VariablesOnly(changed) if stale => {
            let names: Vec<&str> = changed.iter().map(|(a, _)| a.as_str()).collect();
            println!("    default(s) changed upstream: {}", names.join(", "));
        }
        Drift::VariablesOnly(changed) => {
            let mut to_add = Vec::new();
            for (anchor, entry) in changed {
                if main_anchors.contains(anchor) {
                    println!(
                        "    {} ={}\n      — already overridden in the main file; local edit is redundant",
                        anchor,
                        indented(&entry.value)
                    );
                } else {
                    to_add.push((anchor, entry));
                }
            }
            if !to_add.is_empty() {
                // Same remedy either way — pin the value in the estate so the pack
                // stays pristine — but the two dialects spell it differently, and
                // the anchors a Satz reader sees here are the compiled twin's, not
                // the names in their source.
                let satz = rel.extension().and_then(|e| e.to_str()) == Some("satz");
                if satz {
                    println!("    override in the estate's `params {{ … }}` block, then restore the preset with `get-presets`:");
                    for (anchor, entry) in to_add {
                        println!("      {} ={}", anchor.replace('-', "_"), indented(&entry.value));
                    }
                } else {
                    println!("    add to the main file's `variables:` block (before the include), then restore the preset with `get-presets`:");
                    for (anchor, entry) in to_add {
                        println!("      {}: &{}{}", entry.key, anchor, indented(&entry.value));
                    }
                }
            }
        }
        Drift::Structural { summary } => {
            println!("    {summary} — not mechanically migratable, review by hand");
        }
    }
}

/// The `check-presets` command. Returns true if any *included* preset drifted
/// (the caller turns that into a non-zero exit code for CI use).
pub(crate) async fn run_check_presets(
    input: &Path,
    presets_dir: &str,
    include_dirs: &[String],
    pristine_dir: Option<PathBuf>,
) -> Result<bool, BoxErr> {
    let local_base = PathBuf::from(presets_dir);
    if !local_base.is_dir() {
        return Err(format!(
            "presets directory {} not found — run `satz get-presets` first",
            local_base.display()
        )
        .into());
    }

    // Pristine copy: an explicit directory, or one download for the process.
    let pristine_base = pristine_source(pristine_dir).await?;

    // Which presets does the estate actually use? Satz `use` graph or YAML
    // `!include` manifest, depending on the dialect.
    let included = used_preset_files(input, presets_dir, include_dirs)?;

    let main_text = crate::fsx::read_to_string(input)?;
    let main_anchors = anchors_defined_in_main(&main_text);

    let mut local_files = Vec::new();
    walk_preset_sources(&local_base, &local_base, &mut local_files)?;
    let mut pristine_files = Vec::new();
    walk_preset_sources(&pristine_base, &pristine_base, &mut pristine_files)?;
    let local_set: BTreeSet<_> = local_files.into_iter().collect();
    let pristine_set: BTreeSet<_> = pristine_files.into_iter().collect();

    let mut drift_in_use = false;
    let mut clean = 0usize;
    let mut stale = 0usize;

    println!("\ncheck-presets: comparing {} against upstream\n", local_base.display());

    for rel in local_set.union(&pristine_set) {
        // bookkeeping and generated artifacts are not presets
        let fname_s = rel.file_name().unwrap_or_default().to_string_lossy().to_string();
        if rel.starts_with(".base") || fname_s.contains(".gen.") {
            continue;
        }
        // A `.yaml` whose `.satz` sibling exists upstream is that pack's generated
        // twin, not a preset of its own. `merge-presets` already treats it that
        // way; without the same rule here a Satz estate is told that every twin it
        // correctly does not have is a "new upstream preset".
        if fname_s.ends_with(".yaml")
            && !fname_s.ends_with(".claims.yaml")
            && pristine_set.contains(&rel.with_extension("satz"))
        {
            continue;
        }
        if fname_s.ends_with(".claims.yaml") {
            continue; // legacy sidecar (pre-v0.38) still lying in an estate — never a source
        }
        let in_use = included.contains(rel);
        let tag = if in_use { " [included]" } else { "" };

        match (local_set.contains(rel), pristine_set.contains(rel)) {
            (true, false) => {
                let name = rel.file_name().unwrap_or_default().to_string_lossy();
                if name.contains(".local.") {
                    let stem = name.split(".local.").next().unwrap_or_default();
                    println!("  fork{}: {} (deliberate fork of {} — updates to the upstream file accumulate in {}.diff.satz)", tag, rel.display(), stem, stem);
                } else if name.ends_with(".diff.satz") {
                    // ledger files are artifacts of merge-presets, not presets
                } else {
                    println!("  local-only{}: {} (not an upstream preset — kept as-is)", tag, rel.display());
                }
            }
            (false, true) => {
                println!("  missing locally: {} (new upstream preset — `get-presets` fetches it)", rel.display());
            }
            (true, true) => {
                let local_text = crate::fsx::read_to_string(local_base.join(rel))?;
                let pristine_text = crate::fsx::read_to_string(pristine_base.join(rel))?;
                let drift = classify_source(rel, &local_text, &pristine_text);
                // Two independent axes, and conflating them is what made the old
                // report unhelpful: the VERSION line says whether a newer release
                // exists, the content comparison says whether anyone edited this
                // copy. Behind + differing = simply old (adopt). Same version +
                // differing = a local edit (merge, so the edit is preserved).
                let lv = pack_version(&local_text);
                let uv = pack_version(&pristine_text);
                let behind = match (&lv, &uv) {
                    (Some(l), Some(u)) => l != u,
                    _ => false,
                };
                let vtag = match (&lv, &uv) {
                    (Some(l), Some(u)) if l != u => format!(" — local v{l}, upstream v{u}"),
                    (Some(l), _) => format!(" — v{l}"),
                    _ => String::new(),
                };
                match (&drift, behind) {
                    (Drift::Clean, false) => clean += 1,
                    // Version moved, semantics did not: comment or formatting
                    // churn upstream. Nothing to decide, nothing to fail.
                    (Drift::Clean, true) => {
                        stale += 1;
                        println!(
                            "  STALE{}: {}{} — no semantic change; adopting costs nothing",
                            tag,
                            rel.display(),
                            vtag
                        );
                    }
                    (_, true) => {
                        stale += 1;
                        if in_use {
                            drift_in_use = true;
                        }
                        println!("  STALE{}: {}{}", tag, rel.display(), vtag);
                        print_drift_detail(&drift, rel, &main_anchors, true);
                        // A pristine file whose `.local` fork exists is not a
                        // candidate for adoption: the estate runs the fork, so
                        // overwriting this copy changes nothing it emits AND
                        // destroys the branch point the eventual merge needs.
                        if local_set.contains(&fork_sibling(rel)) {
                            println!("    the estate runs {} — this pristine copy is that fork's", fork_sibling(rel).display());
                            println!("    baseline. Leave it; `merge-presets` refreshes it and rewrites the .diff.");
                        } else {
                            println!("    a newer release exists — the differences above are the version gap.");
                            println!("    Adopt it (copy the pristine file in), or `merge-presets` if this copy may");
                            println!("    ALSO have been edited. See docs/presets-workflow.md.");
                        }
                    }
                    (Drift::VariablesOnly(_) | Drift::Structural { .. }, false) => {
                        if in_use {
                            drift_in_use = true;
                        }
                        let kind = match &drift {
                            Drift::VariablesOnly(_) => "variables only",
                            _ => "structural",
                        };
                        println!("  EDITED ({kind}){}: {}{}", tag, rel.display(), vtag);
                        print_drift_detail(&drift, rel, &main_anchors, false);
                        if lv.is_some() && lv == uv {
                            println!("    same version as upstream, different content — a local edit (or an");
                            println!("    upstream release that changed without a version bump).");
                        }
                    }
                }
            }
            (false, false) => unreachable!(),
        }
    }

    println!("\n{clean} preset(s) clean, {stale} behind upstream.");
    if stale > 0 {
        println!("`clean` means unedited, not current — the version line is the staleness signal.");
    }
    if drift_in_use {
        println!("Drift detected in included preset(s) — exit code 1.");
    } else {
        println!("No drift in included presets.");
    }
    Ok(drift_in_use)
}

// ---------------------------------------------------------------------------
// get-presets: populate and refresh, without ever changing a live org by accident
// ---------------------------------------------------------------------------

/// Fetch the upstream library into `presets_dir`.
///
/// This used to overwrite every pristine-named file unconditionally, with no
/// idea what the estate uses. On a live estate that silently retires whatever
/// upstream retired, and the first sign is a destroy in a `tofu plan` nobody
/// asked for. It is a BOOTSTRAP command, so it now behaves like one:
///
/// - missing locally -> installed
/// - identical -> skipped
/// - differs, NOT used by the estate -> refreshed (git history keeps it if tracked)
/// - differs, USED by the estate -> REFUSED, with the two commands that are
///   actually appropriate; `--force` overrides, after listing what it will change
///
/// `X.local.*` files have no upstream counterpart, so nothing here can touch them.
pub(crate) async fn run_get_presets(
    presets_dir: &str,
    runtime_config: &crate::ToolConfig,
    force: bool,
    pristine_dir: Option<PathBuf>,
) -> Result<(), BoxErr> {
    let local_base = PathBuf::from(presets_dir);
    crate::fsx::create_dir_all(&local_base)?;

    // `--pristine-dir` is the same escape its two siblings already had, and the
    // only way to work while the unauthenticated GitHub quota (60/hour, shared
    // with `self-update`) is exhausted.
    let tmp = pristine_source(pristine_dir).await?;

    // Which pack stems does the estate actually use? No estate (a fresh repo) =
    // nothing to protect, which is exactly the bootstrap case.
    let estate = find_estate(&runtime_config.yaml_dir);
    let mut used_stems: BTreeSet<PathBuf> = BTreeSet::new();
    match &estate {
        Some(est) => {
            for rel in used_preset_files(est, presets_dir, &runtime_config.include_dirs)? {
                if let Some(stem) = pack_stem(&rel) {
                    used_stems.insert(stem);
                }
            }
        }
        None => println!("note: no estate found in '{}' — nothing to protect, refreshing everything", runtime_config.yaml_dir),
    }

    let mut files = Vec::new();
    walk_preset_sources(&tmp, &tmp, &mut files)?;
    let mut extra = Vec::new();
    let mut stack = vec![tmp.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue; }
            let name = p.to_string_lossy();
            if name.ends_with(".md") {
                extra.push(p.strip_prefix(&tmp)?.to_path_buf());
            }
        }
    }
    files.extend(extra);
    files.sort();
    files.dedup();

    let (mut installed, mut current, mut refreshed, mut refused) = (0usize, 0usize, 0usize, 0usize);
    for rel in &files {
        let up = crate::fsx::read_to_string(tmp.join(rel))?;
        let lo_path = local_base.join(rel);
        if !lo_path.exists() {
            if let Some(parent) = lo_path.parent() { crate::fsx::create_dir_all(parent)?; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            installed += 1;
            continue;
        }
        let lo = crate::fsx::read_to_string(&lo_path)?;
        if lo == up { current += 1; continue; }

        let stem = pack_stem(rel).unwrap_or_else(|| rel.clone());
        let in_use = used_stems.contains(&stem);
        if in_use && !force {
            let (v_lo, v_up) = (pack_version(&lo), pack_version(&up));
            println!("  REFUSED {}: the estate uses it and upstream moved {}", rel.display(), version_arrow(&v_lo, &v_up));
            println!("    `merge-presets` (forks it, keeps your content) or");
            println!("    `merge-presets --adopt {}` (upgrades it in place), or --force to overwrite anyway.", stem.file_name().unwrap_or_default().to_string_lossy());
            refused += 1;
            continue;
        }
        if in_use {
            let (v_lo, v_up) = (pack_version(&lo), pack_version(&up));
            println!("  --force overwrote IN-USE {} ({}) — re-transpile and read the plan", rel.display(), version_arrow(&v_lo, &v_up));
        }
        crate::fsx::write(&lo_path, up.as_bytes())?;
        refreshed += 1;
    }
    println!("\nget-presets: {installed} installed, {current} already current, {refreshed} refreshed, {refused} refused.");
    if refused > 0 {
        println!("Refused files are packs this estate deploys — changing them changes the org.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// merge-presets: the reconciling update flow (fork + ledger, base-aware)
// ---------------------------------------------------------------------------

/// The reconciling update — provenance by suffix, no snapshots:
///
/// Pristine names belong to upstream and are always overwritable. A preset that
/// is USED by the estate never changes silently: if upstream moved semantically,
/// the current content is preserved as `X.local.satz`, the estate's `use` is
/// repointed to it (verified by transpile identity — guaranteed, the fork IS the
/// old content), and `X.diff.satz` is written with the CURRENT adoption delta
/// (refreshed on every update, not an accumulating ledger — git is the history).
/// Adoption = point the estate back at the pristine name and delete fork + diff.
///
/// Semantic change = the compiled canonical YAML differs (comments/formatting
/// don't fork anything). Version fields cross-check: same version + different
/// semantics warns (upstream release-hygiene bug).
#[allow(clippy::too_many_lines)]
pub(crate) async fn run_merge_presets(
    presets_dir: &str,
    pristine_dir: Option<PathBuf>,
    estate_arg: Option<PathBuf>,
    tool_config: &crate::ToolConfig,
    runtime_config: &crate::ToolConfig,
    report_only: bool,
    adopt: &[String],
) -> Result<bool, BoxErr> {
    let local_base = PathBuf::from(presets_dir);
    crate::fsx::create_dir_all(&local_base)?;
    // Adoption and auto-forking cannot share a run. The fork+repoint proves
    // itself by transpile identity, and an adoption legitimately CHANGES the
    // transpiled output — running both would make that proof meaningless (or,
    // worse, roll a good repoint back). One run, one kind of operation.
    let adopting = !adopt.is_empty();
    let pristine = pristine_source(pristine_dir).await?;

    // .base/ is retired: pristine names are upstream-owned, forks carry the local
    // truth, diffs carry the adoption delta — no third copy needed.
    let old_base = local_base.join(".base");
    if old_base.exists() && !report_only {
        std::fs::remove_dir_all(&old_base)?;
        println!("note: removed obsolete {} (snapshots retired — pristine names are upstream-owned)", old_base.display());
    }

    // ---- estate context: which pack stems are actually included -------------
    let estate = match estate_arg {
        Some(e) => Some(e),
        None => find_estate(&runtime_config.yaml_dir),
    };
    let mut used_stems: BTreeSet<PathBuf> = BTreeSet::new();
    if let Some(est) = &estate {
        // The same `use`-graph walk `check-presets` makes — no include manifest,
        // so a satz estate is read as satz rather than through a generated twin.
        for rel in used_preset_files(est, presets_dir, &runtime_config.include_dirs)? {
            if let Some(stem) = pack_stem(&rel) {
                used_stems.insert(stem);
            }
        }
    } else {
        println!("note: no estate found in '{}' — used-preset protection inactive; changed presets are reported, not forked", runtime_config.yaml_dir);
    }

    // baseline for the self-verifying estate edit
    let baseline = match (&estate, report_only) {
        (Some(est), false) => Some(crate::transpile_sorted_for(est, tool_config, runtime_config)?),
        _ => None,
    };
    let estate_dirty = estate.as_deref().map(is_git_dirty).unwrap_or(false);

    // ---- upstream inventory --------------------------------------------------
    let mut upstream_files = Vec::new();
    walk_yaml_files(&pristine, &pristine, &mut upstream_files)?;
    let mut stack = vec![pristine.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue; }
            let name = p.to_string_lossy();
            if name.ends_with(".satz") || name.ends_with(".md") {
                upstream_files.push(p.strip_prefix(&pristine)?.to_path_buf());
            }
        }
    }
    upstream_files.sort();
    upstream_files.dedup();
    let upstream_set: BTreeSet<PathBuf> = upstream_files.iter().cloned().collect();

    let (mut installed, mut current, mut doc_only, mut artifacts, mut unused_over, mut forked, mut refreshed, mut refused) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut adopted = 0usize;
    let mut deferred = 0usize;
    let mut needs_attention = false;
    // rollback journal for the estate edit: (path, previous content) + created files
    let mut journal: Vec<(PathBuf, String)> = Vec::new();
    let mut created: Vec<PathBuf> = Vec::new();
    let mut estate_edited = false;

    for rel in &upstream_files {
        if rel.starts_with(".base") { continue; }
        let up_path = pristine.join(rel);
        let lo_path = local_base.join(rel);
        let up = crate::fsx::read_to_string(&up_path)?;

        if !lo_path.exists() {
            if report_only { println!("  would install {}", rel.display()); continue; }
            if let Some(parent) = lo_path.parent() { crate::fsx::create_dir_all(parent)?; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            installed += 1;
            continue;
        }
        let lo = crate::fsx::read_to_string(&lo_path)?;
        if lo == up { current += 1; continue; }

        let fname = rel.file_name().unwrap_or_default().to_string_lossy().to_string();
        let is_satz = fname.ends_with(".satz");
        // twins of satz packs and docs are generated/derived artifacts
        let is_artifact = fname.ends_with(".md") // (docs)
            || (fname.ends_with(".yaml") && upstream_set.contains(&rel.with_extension("satz")));
        if is_artifact {
            if report_only { println!("  would update artifact {}", rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            artifacts += 1;
            continue;
        }

        // semantic comparison in the canonical form (the parsed AST, printed
        // without comments/formatting/version — same as check-presets)
        let sem_equal = if is_satz {
            match (satz_core::satz::parse(&lo), satz_core::satz::parse(&up)) {
                (Ok(a), Ok(b)) => satz_core::satz::canonical(&a) == satz_core::satz::canonical(&b),
                _ => false,
            }
        } else {
            normalized_yaml(&lo) == normalized_yaml(&up)
        };
        if sem_equal {
            if report_only { println!("  would update {} (doc/format only)", rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            doc_only += 1;
            continue;
        }

        // version hygiene cross-check (satz packs carry in-file versions)
        let (v_lo, v_up) = if is_satz {
            (satz_version(&lo), satz_version(&up))
        } else { (None, None) };
        if is_satz && v_lo.is_some() && v_lo == v_up {
            println!("  WARNING {}: content changed semantically but the pack version did not — upstream release-hygiene bug", rel.display());
            needs_attention = true;
        }

        let stem = pack_stem(rel).unwrap_or_else(|| rel.clone());
        let fork_ext = if is_satz { "satz" } else { "yaml" };
        let fork_rel = PathBuf::from(format!("{}.local.{}", stem.display(), fork_ext));
        let fork_path = local_base.join(&fork_rel);
        let diff_path = local_base.join(format!("{}.diff.satz", stem.display()));
        let used = used_stems.contains(&stem);

        if fork_path.exists() {
            // existing fork: pristine tracks upstream, diff refreshed below
            if report_only { println!("  would update {} (fork {} present)", rel.display(), fork_rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            refreshed += 1;
            println!("  fork {}: upstream moved {} — review {}", stem.display(),
                version_arrow(&v_lo, &v_up), diff_path.display());
            needs_attention = true;
            continue;
        }

        if !used {
            if report_only { println!("  would overwrite unused {} (differs; git history keeps it if tracked)", rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            unused_over += 1;
            println!("  overwrote unused {} (differed; git history keeps it if tracked)", rel.display());
            continue;
        }

        // USED + semantically changed. `--adopt` turns this into the deliberate
        // in-place upgrade: overwrite the pristine name, leave the estate's `use`
        // alone, and show the EMISSION delta — which is what the operator is
        // actually deciding about, not the preset diff.
        let stem_name = stem.file_name().unwrap_or_default().to_string_lossy().to_string();
        let behind = v_lo.is_some() && v_up.is_some() && v_lo != v_up;
        let choice = adopt_choice(adopt, &stem_name, &stem, behind);
        if choice == AdoptChoice::SkipEdited {
            println!("  --adopt all SKIPPED {}: same version as upstream but different content — that is a local EDIT, not staleness. Name it explicitly to overwrite it.", rel.display());
            needs_attention = true;
            continue;
        }
        if choice == AdoptChoice::Adopt {
            if report_only {
                println!("  would adopt {} in place ({})", rel.display(), version_arrow(&v_lo, &v_up));
                adopted += 1;
                needs_attention = true;
                continue;
            }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            adopted += 1;
            needs_attention = true;
            println!("  adopted {} in place ({}) — the estate keeps using the pristine name", rel.display(), version_arrow(&v_lo, &v_up));
            continue;
        }
        if adopting {
            println!("  DEFERRED {}: needs a fork+repoint, which cannot share a run with --adopt (the repoint proves itself by transpile identity). Re-run `merge-presets` without --adopt.", rel.display());
            deferred += 1;
            needs_attention = true;
            continue;
        }

        // USED + semantically changed + no fork -> auto-fork + repoint
        if report_only {
            println!("  would fork {} -> {} and repoint the estate (upstream {})",
                rel.display(), fork_rel.display(), version_arrow(&v_lo, &v_up));
            needs_attention = true;
            continue;
        }
        if estate_dirty {
            println!("  REFUSED {}: estate file has uncommitted changes — commit/stash it so the repoint stays an isolated edit (pack left untouched)", rel.display());
            refused += 1;
            needs_attention = true;
            continue; // pristine NOT updated either: the estate still deploys the old content
        }
        let est = estate.clone().expect("used_stems nonempty implies estate");
        let est_text = crate::fsx::read_to_string(&est)?;
        match rewrite_estate_uses(&est_text, &est, &runtime_config.include_dirs, &lo_path, &fork_rel) {
            Some(new_text) => {
                if journal.iter().all(|(p, _)| p != &est) {
                    journal.push((est.clone(), est_text.clone()));
                }
                crate::fsx::write(&fork_path, lo.as_bytes())?;
                created.push(fork_path.clone());
                journal.push((lo_path.clone(), lo.clone()));
                crate::fsx::write(&lo_path, up.as_bytes())?;
                crate::fsx::write(&est, new_text.as_bytes())?;
                estate_edited = true;
                forked += 1;
                println!("  forked {} -> {} (upstream {}); estate repointed — adoption delta in {}",
                    rel.display(), fork_rel.display(), version_arrow(&v_lo, &v_up), diff_path.display());
                needs_attention = true;
            }
            None => {
                println!("  REFUSED {}: could not locate the estate `use` for this pack (used via another pack?) — fork it by hand", rel.display());
                refused += 1;
                needs_attention = true;
            }
        }
    }

    // refresh every fork's adoption delta (idempotent)
    if !report_only {
        refresh_adoption_diffs(&local_base)?;
    }

    // Adoption changes what the estate emits — that is the point, so there is no
    // identity check to make. Show the delta instead, in the terms that matter.
    if adopted > 0 && !report_only {
        if let (Some(est), Some(before)) = (estate.clone(), baseline.clone()) {
            let after = crate::transpile_sorted_for(&est, tool_config, runtime_config)?;
            println!("\n  emission delta after adoption:");
            print!("{}", emission_delta(&before, &after));
            println!("  hcl/ on disk is NOT regenerated by this command — run `satz transpile`,");
            println!("  read `git diff hcl/main.tf`, then `tofu plan` before applying.");
        }
    }

    // the self-verifying estate edit: output must be byte-identical
    if estate_edited {
        let est = estate.clone().unwrap();
        let after = crate::transpile_sorted_for(&est, tool_config, runtime_config)?;
        if baseline.as_deref() != Some(after.as_str()) {
            for p in &created { let _ = std::fs::remove_file(p); }
            for (p, content) in journal.iter().rev() { crate::fsx::write(p, content.as_bytes())?; }
            return Err("merge-presets: estate repoint changed the transpiled output — rolled back everything (this should be impossible; please report)".into());
        }
        println!("  estate repoint verified: transpiled output identical.");
    }

    println!(
        "\nmerge-presets: {installed} installed, {current} current, {artifacts} artifacts updated, {doc_only} doc-only, {unused_over} unused overwritten, {adopted} adopted in place, {forked} forked+repointed, {refreshed} fork diffs refreshed, {deferred} deferred, {refused} refused."
    );
    Ok(needs_attention)
}

/// What `--adopt` says about one pack.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdoptChoice {
    /// Overwrite the pristine name in place, leave the estate's `use` alone.
    Adopt,
    /// `--adopt all` met a pack that differs at the SAME version. That is an
    /// EDIT, not staleness, and blanket-overwriting it would throw away a
    /// customer's change without anyone naming it. Blanket means "everything
    /// merely behind", never "everything".
    SkipEdited,
    /// Not selected — the normal fork path applies.
    No,
}

pub(crate) fn adopt_choice(adopt: &[String], stem_name: &str, stem: &Path, behind: bool) -> AdoptChoice {
    let named = adopt.iter().any(|a| a == stem_name || Path::new(a) == stem);
    if named {
        return AdoptChoice::Adopt;
    }
    if adopt.iter().any(|a| a == "all") {
        return if behind { AdoptChoice::Adopt } else { AdoptChoice::SkipEdited };
    }
    AdoptChoice::No
}

/// Summarise what an adoption does to the emitted HCL, from two *sorted*
/// transpiler outputs. Sorted text cannot attribute a changed attribute to its
/// resource, so resources are named and everything else is counted — enough to
/// tell "one policy disappears" from "one attribute moves", which is the whole
/// question before a `tofu plan`.
fn emission_delta(before: &str, after: &str) -> String {
    fn addresses(s: &str) -> BTreeSet<String> {
        s.lines()
            .filter_map(|l| {
                let t = l.trim();
                let rest = t.strip_prefix("resource ")?;
                Some(rest.trim_end_matches(" {").replace('"', "").replace(' ', "."))
            })
            .collect()
    }
    let (a, b) = (addresses(before), addresses(after));
    let mut out = String::new();
    for gone in a.difference(&b) {
        out.push_str(&format!("    - {gone}  (REMOVED)\n"));
    }
    for new in b.difference(&a) {
        out.push_str(&format!("    + {new}  (added)\n"));
    }
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let changed = before_lines.iter().filter(|l| !after_lines.contains(*l)).count()
        + after_lines.iter().filter(|l| !before_lines.contains(*l)).count();
    let resource_lines = a.symmetric_difference(&b).count();
    let attr = changed.saturating_sub(resource_lines);
    if attr > 0 {
        out.push_str(&format!(
            "    ~ {attr} other line(s) differ — attribute changes, plus the bodies of any resource named above\n"
        ));
    }
    if out.is_empty() {
        out.push_str("    (none — the estate overrides everything that moved)\n");
    }
    out
}

/// The single estate file: a `.satz` in yaml_dir whose header declares `estate`.
fn find_estate(yaml_dir: &str) -> Option<PathBuf> {
    let mut satz = Vec::new();
    let mut yaml = Vec::new();
    for e in std::fs::read_dir(yaml_dir).ok()?.flatten() {
        let p = e.path();
        let name = p.to_string_lossy().to_string();
        if name.contains(".local.") || name.contains(".gen.") {
            continue;
        }
        match p.extension().and_then(|x| x.to_str()) {
            Some("satz") => {
                if let Ok(src) = std::fs::read_to_string(&p) {
                    if let Ok(f) = satz_core::satz::parse(&src) {
                        if f.estate.is_some() && !f.is_pack {
                            satz.push(p);
                        }
                    }
                }
            }
            // A YAML estate declares the deployment itself; a YAML pack never
            // does. `terraform:` at the top level is therefore the discriminator.
            Some("yaml") | Some("yml") => {
                if let Ok(src) = std::fs::read_to_string(&p) {
                    if src.lines().any(|l| l.trim_end() == "terraform:") {
                        yaml.push(p);
                    }
                }
            }
            _ => {}
        }
    }
    // Satz wins outright: falling back only when there is no satz estate keeps
    // this change incapable of altering what a satz repo resolves to, including
    // one with a stale hand-written `.yaml` estate still lying around.
    if satz.len() == 1 {
        return satz.pop();
    }
    if satz.is_empty() && yaml.len() == 1 {
        return yaml.pop();
    }
    None
}

/// rel path -> pack stem: strip generated/derived suffixes and a `.local` marker.
fn pack_stem(rel: &Path) -> Option<PathBuf> {
    let s = rel.to_string_lossy();
    let stem = s
        .strip_suffix(".gen.claims.yaml")
        .or_else(|| s.strip_suffix(".claims.yaml"))
        .or_else(|| s.strip_suffix(".gen.yaml"))
        .or_else(|| s.strip_suffix(".yaml"))
        .or_else(|| s.strip_suffix(".satz"))?;
    let stem = stem.strip_suffix(".local").unwrap_or(stem);
    Some(PathBuf::from(stem))
}

fn normalized_yaml(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn satz_version(src: &str) -> Option<String> {
    satz_core::satz::parse(src).ok().and_then(|f| f.version)
}

fn version_arrow(a: &Option<String>, b: &Option<String>) -> String {
    match (a, b) {
        (Some(x), Some(y)) => format!("{} -> {}", x, y),
        _ => "content changed".to_string(),
    }
}

fn is_git_dirty(path: &Path) -> bool {
    let Some(dir) = path.parent() else { return false };
    std::process::Command::new("git")
        .arg("-C").arg(dir)
        .args(["status", "--porcelain", "--"])
        .arg(path)
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Repoint every estate `use "..."` that resolves to `target` at `fork_rel`
/// (written with the same path prefix style the estate already uses).
fn rewrite_estate_uses(
    text: &str,
    estate: &Path,
    include_dirs: &[String],
    target: &Path,
    fork_rel: &Path,
) -> Option<String> {
    let est_dir = estate.parent().unwrap_or(Path::new("."));
    let canon_target = std::fs::canonicalize(target).ok()?;
    let fork_name = fork_rel.file_name()?.to_string_lossy().to_string();
    let mut out = String::with_capacity(text.len());
    let mut hit = false;
    for line in text.lines() {
        let mut newline = line.to_string();
        if let Some(a) = line.find("use \"") {
            if let Some(b) = line[a + 5..].find('"') {
                let written = &line[a + 5..a + 5 + b];
                let mut candidates = vec![est_dir.join(written)];
                candidates.extend(include_dirs.iter().map(|d| Path::new(d).join(written)));
                // a written `X.yaml` may resolve to the twin of the target .satz
                let twin_target = canon_target.with_extension("yaml");
                if candidates.iter().any(|c| {
                    std::fs::canonicalize(c)
                        .map(|cc| cc == canon_target || cc == twin_target)
                        .unwrap_or(false)
                }) {
                    let new_written = match written.rfind('/') {
                        Some(i) => format!("{}/{}", &written[..i], fork_name),
                        None => fork_name.clone(),
                    };
                    newline = line.replace(written, &new_written);
                    hit = true;
                }
            }
        }
        out.push_str(&newline);
        out.push('\n');
    }
    if hit { Some(out) } else { None }
}

/// Every `X.local.*` gets `X.diff.satz` = the CURRENT adoption delta against the
/// pristine file. Idempotent; rewritten (not appended) on every run.
fn refresh_adoption_diffs(local_base: &Path) -> Result<(), BoxErr> {
    let mut stack = vec![local_base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == ".base") { continue; }
                stack.push(p);
                continue;
            }
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            // orphaned generated twins of an adopted fork
            if let Some(stem_name) = name.strip_suffix(".local.gen.yaml").or_else(|| name.strip_suffix(".local.gen.claims.yaml")) {
                if !p.with_file_name(format!("{}.local.satz", stem_name)).exists() {
                    let _ = std::fs::remove_file(&p);
                }
                continue;
            }
            // orphaned delta: the fork was adopted/deleted -> the diff goes too
            if let Some(stem_name) = name.strip_suffix(".diff.satz") {
                let has_fork = p.with_file_name(format!("{}.local.satz", stem_name)).exists()
                    || p.with_file_name(format!("{}.local.yaml", stem_name)).exists();
                if !has_fork {
                    let _ = std::fs::remove_file(&p);
                    println!("  removed orphaned {} (fork adopted)", p.display());
                }
                continue;
            }
            let Some(stem_name) = name.strip_suffix(".local.satz").or_else(|| name.strip_suffix(".local.yaml")) else { continue };
            let ext = if name.ends_with(".satz") { "satz" } else { "yaml" };
            let pristine = p.with_file_name(format!("{}.{}", stem_name, ext));
            if !pristine.exists() { continue; }
            let fork_text = crate::fsx::read_to_string(&p)?;
            let pris_text = crate::fsx::read_to_string(&pristine)?;
            let (v_l, v_u) = (satz_version(&fork_text), satz_version(&pris_text));
            let diff_path = p.with_file_name(format!("{}.diff.satz", stem_name));
            let body = text_diff(&fork_text, &pris_text);
            let content = format!(
                "// {}: adoption delta, fork {} (generated by merge-presets — what adopting\n// the pristine preset would change; refreshed on every update, never edited by hand)\n\n{}",
                stem_name, version_arrow(&v_l, &v_u), body
            );
            let prev = diff_path.exists().then(|| crate::fsx::read_to_string(&diff_path)).transpose()?;
            if prev.as_deref() != Some(content.as_str()) {
                crate::fsx::write(&diff_path, content.as_bytes())?;
            }
        }
    }
    Ok(())
}

/// Unified diff via git (battle-tested); falls back to a full old/new dump.
fn text_diff(old: &str, new: &str) -> String {
    let tmp = std::env::temp_dir();
    let (fo, fn_) = (tmp.join(format!("mp_old_{}", std::process::id())), tmp.join(format!("mp_new_{}", std::process::id())));
    if std::fs::write(&fo, old).is_ok() && std::fs::write(&fn_, new).is_ok() {
        if let Ok(out) = std::process::Command::new("git")
            .args(["diff", "--no-index", "--no-color", "--unified=3"])
            .arg(&fo).arg(&fn_)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            let body: String = text.lines()
                .skip_while(|l| !l.starts_with("@@"))
                .map(|l| format!("{}\n", l))
                .collect();
            if !body.is_empty() { return body; }
        }
    }
    format!("// --- local fork ---\n{}\n// --- pristine ---\n{}\n", old, new)
}

// ---------------------------------------------------------------------------
// Tests (pure layer only — no network, no filesystem)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod find_estate_tests {
    //! YAML input stays supported (owner, 2026-08-23), so estate discovery has
    //! to see both dialects — but a satz repo is FULL of `.gen.yaml` twins that
    //! each carry a top-level `terraform:`, and counting one of those as a second
    //! estate would return None and silently switch used-preset protection off.
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("satz-find-estate-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    const YAML_ESTATE: &str = "variables:\n  a: &a \"x\"\nterraform:\n  backend:\n    local:\n      path: \"t.tfstate\"\n";
    const SATZ_ESTATE: &str = "estate demo\n\nparams {\n  a = \"x\"\n}\n";

    #[test]
    fn finds_a_yaml_estate_when_there_is_no_satz_one() {
        let d = dir("yaml-only");
        std::fs::write(d.join("C01.yaml"), YAML_ESTATE).unwrap();
        // a YAML pack: resources but no `terraform:` — must not be mistaken for one
        std::fs::write(d.join("pack.yaml"), "google_storage_bucket:\n  b:\n    name: n\n").unwrap();
        assert_eq!(find_estate(d.to_str().unwrap()), Some(d.join("C01.yaml")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn generated_twins_never_count_as_estates() {
        let d = dir("twins");
        std::fs::write(d.join("C01.satz"), SATZ_ESTATE).unwrap();
        // what a satz repo actually looks like after a transpile
        std::fs::write(d.join("C01.gen.yaml"), YAML_ESTATE).unwrap();
        std::fs::write(d.join("other.gen.yaml"), YAML_ESTATE).unwrap();
        assert_eq!(
            find_estate(d.to_str().unwrap()),
            Some(d.join("C01.satz")),
            "twins must not turn a satz repo into an ambiguous one"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn satz_wins_over_a_stale_hand_written_yaml_estate() {
        let d = dir("both");
        std::fs::write(d.join("C01.satz"), SATZ_ESTATE).unwrap();
        std::fs::write(d.join("C01-old.yaml"), YAML_ESTATE).unwrap();
        assert_eq!(find_estate(d.to_str().unwrap()), Some(d.join("C01.satz")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn two_estates_of_the_same_dialect_stay_ambiguous() {
        let d = dir("ambiguous");
        std::fs::write(d.join("a.yaml"), YAML_ESTATE).unwrap();
        std::fs::write(d.join("b.yaml"), YAML_ESTATE).unwrap();
        assert_eq!(find_estate(d.to_str().unwrap()), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRISTINE: &str = "\
# header comment
variables:
  logsink-project-name: &logsink-project-name \"log-infra-001\"
  logsink-retention-days: &logsink-retention-days 400

google_storage_bucket:
  org_audit_logs:
    name: *logsink-bucket-name
";

    const PACK: &str = r#"pack demo version "1.0"

params {
  bucket_name = "demo-audit"
}

"policy" {
  name = "compute.managed.requireOsLogin"
  spec {
    rules = [
      {
        enforce = "TRUE"
      },
    ]
  }
}
"#;

    /// P2. `--adopt all` means "everything merely BEHIND", never "everything".
    /// A pack that differs at the SAME version is an edit somebody made, and
    /// blanket-overwriting it would discard that without anyone naming it.
    #[test]
    fn adopt_all_upgrades_stale_packs_but_never_edited_ones() {
        let all = vec!["all".to_string()];
        let stem = Path::new("CIS-GCP-Foundation-4.0");
        assert_eq!(adopt_choice(&all, "CIS-GCP-Foundation-4.0", stem, true), AdoptChoice::Adopt);
        assert_eq!(adopt_choice(&all, "CIS-GCP-Foundation-4.0", stem, false), AdoptChoice::SkipEdited);
    }

    /// Naming a pack is the deliberate act, so it wins even for an edited pack —
    /// that is how a customer discards their own fork on purpose.
    #[test]
    fn naming_a_pack_adopts_it_even_when_it_was_edited() {
        let named = vec!["CIS-GCP-Foundation-4.0".to_string()];
        let stem = Path::new("CIS-GCP-Foundation-4.0");
        assert_eq!(adopt_choice(&named, "CIS-GCP-Foundation-4.0", stem, false), AdoptChoice::Adopt);
        assert_eq!(adopt_choice(&named, "essential-contacts", Path::new("essential-contacts"), true), AdoptChoice::No);
        assert_eq!(adopt_choice(&[], "CIS-GCP-Foundation-4.0", stem, true), AdoptChoice::No);
    }

    /// The delta an operator reads before `tofu plan`: which resources appear or
    /// disappear, by address, and how much else moved.
    #[test]
    fn emission_delta_names_resources_and_counts_the_rest() {
        let before = "resource \"google_org_policy_policy\" \"legacy\" {\n  enforce = \"TRUE\"\n";
        let after = "  enforce = \"FALSE\"\n";
        let d = emission_delta(before, after);
        assert!(d.contains("google_org_policy_policy.legacy"), "{d}");
        assert!(d.contains("REMOVED"), "{d}");
        assert!(d.contains("other line(s) differ"), "{d}");
        assert!(emission_delta("x\n", "x\n").contains("none"));
    }

    /// P4. A default written as a LIST lives on the lines BELOW its anchor.
    /// `split_preset` used to keep only the text on the anchor line, so both
    /// sides had an empty value, the items were never compared, and a pack a
    /// whole version behind reported "clean". This is the real case above: CIS v2.0 vs
    /// upstream v2.1, whose only substantive change was a fifth list entry.
    #[test]
    fn a_changed_list_default_is_drift_not_clean() {
        let old = "\
variables:
  subjects: &subjects
    - \"a\"
    - \"b\"

google_org_policy_policy:
  p:
    name: x
";
        let new = old.replace("    - \"b\"", "    - \"b\"\n    - \"c\"");
        assert_ne!(classify(&new, old), Drift::Clean, "a list default gaining an entry is drift");
        match classify(&new, old) {
            Drift::VariablesOnly(changed) => {
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].0, "subjects");
            }
            other => panic!("expected VariablesOnly, got {other:?}"),
        }
    }

    /// The same walk must not turn indentation or comments inside the variables
    /// block into drift — comment churn upgrades silently, by design.
    #[test]
    fn comments_inside_the_variables_block_are_not_drift() {
        let a = "\
variables:
  subjects: &subjects
    - \"a\"

google_x:
  y:
    name: n
";
        let b = "\
variables:
  # explaining the list
  subjects: &subjects
    - \"a\"

google_x:
  y:
    name: n
";
        assert_eq!(classify(a, b), Drift::Clean);
    }

    /// A multi-line value must not leak into the resource body — that would make
    /// every list default read as a structural difference instead.
    #[test]
    fn list_items_do_not_leak_into_the_body() {
        let parts = split_preset(
            "\
variables:
  subjects: &subjects
    - \"a\"
    - \"b\"

google_x:
  y:
    name: n
",
        );
        assert!(parts.body.iter().all(|l| !l.contains("\"a\"")), "body: {:?}", parts.body);
        assert_eq!(parts.vars["subjects"].value.lines().count(), 2);
    }

    /// P1. The version line is the staleness signal — the only one that does not
    /// depend on the content comparison being complete.
    #[test]
    fn pack_version_is_read_from_the_pack_line() {
        assert_eq!(pack_version(PACK).as_deref(), Some("1.0"));
        assert_eq!(pack_version("pack demo\n").as_deref(), None);
        assert_eq!(pack_version("variables:\n  a: &a 1\n").as_deref(), None);
        assert_eq!(
            pack_version("// leading comment\npack CIS_X version \"2.1\"\n").as_deref(),
            Some("2.1")
        );
    }

    /// The R6 regression. `check-presets` compared only `.yaml`, so a `.satz`
    /// pack was never collected, never compared, and "nothing compared" printed
    /// as "clean" — it reported no drift for an estate whose CIS pack had stopped
    /// enforcing OS Login. Classification of a pack goes through its canonical
    /// compiled form, the same comparison `merge-presets` makes.
    #[test]
    fn satz_pack_drift_is_detected_structurally() {
        let rel = Path::new("CIS.satz");
        let drifted = PACK.replace(r#"enforce = "TRUE""#, r#"enforce = "FALSE""#);
        assert_ne!(drifted, PACK, "the fixture must actually change");
        match classify_source(rel, &drifted, PACK) {
            Drift::Structural { .. } => {}
            other => panic!("expected Structural drift for a satz pack, got {:?}", other),
        }
    }

    /// A pack `params` default is the "variables" half of the canonical form,
    /// so the variables-only remedy (pin it in the estate, keep the pack
    /// pristine) applies to Satz packs — under the param's own name.
    #[test]
    fn satz_param_drift_is_variables_only() {
        let rel = Path::new("CIS.satz");
        let drifted = PACK.replace(r#""demo-audit""#, r#""customer-audit""#);
        match classify_source(rel, &drifted, PACK) {
            Drift::VariablesOnly(changed) => {
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].0, "bucket_name");
                assert_eq!(changed[0].1.value, "\"customer-audit\"");
            }
            other => panic!("expected VariablesOnly, got {:?}", other),
        }
    }

    /// A version bump with no content change is not drift — the staleness
    /// check reports the version separately, and `merge-presets` upgrades it
    /// silently as comment-only.
    #[test]
    fn satz_version_bump_alone_is_clean() {
        let rel = Path::new("CIS.satz");
        let bumped = PACK.replacen("version \"", "version \"9", 1);
        assert_ne!(bumped, PACK);
        assert_eq!(classify_source(rel, &bumped, PACK), Drift::Clean);
    }

    /// Comment and formatting churn upgrades silently — same rule `merge-presets`
    /// applies, so the two commands agree.
    #[test]
    fn satz_comment_churn_is_clean() {
        let rel = Path::new("CIS.satz");
        let commented = format!("// a local note
{}", PACK);
        assert_eq!(classify_source(rel, &commented, PACK), Drift::Clean);
    }

    #[test]
    fn identical_and_comment_only_edits_are_clean() {
        assert_eq!(classify(PRISTINE, PRISTINE), Drift::Clean);
        let commented = PRISTINE.replace("# header comment", "# customer note\n# more notes");
        assert_eq!(classify(&commented, PRISTINE), Drift::Clean);
    }

    #[test]
    fn changed_default_values_are_variables_only_with_local_values() {
        let local = PRISTINE.replace("\"log-infra-001\"", "\"acme-infra-001\"");
        match classify(&local, PRISTINE) {
            Drift::VariablesOnly(changed) => {
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].0, "logsink-project-name");
                assert_eq!(changed[0].1.value, "\"acme-infra-001\"");
                assert_eq!(changed[0].1.key, "logsink-project-name");
            }
            other => panic!("expected VariablesOnly, got {:?}", other),
        }
    }

    #[test]
    fn body_edits_are_structural_even_if_variables_also_changed() {
        let local = PRISTINE
            .replace("400", "900")
            .replace("org_audit_logs:", "my_audit_logs:");
        match classify(&local, PRISTINE) {
            Drift::Structural { summary } => assert!(summary.contains("resource line"), "{summary}"),
            other => panic!("expected Structural, got {:?}", other),
        }
    }

    #[test]
    fn added_or_removed_variables_are_structural() {
        let local = PRISTINE.replace(
            "  logsink-retention-days: &logsink-retention-days 400",
            "  logsink-retention-days: &logsink-retention-days 400\n  my-own-var: &my-own-var \"x\"",
        );
        match classify(&local, PRISTINE) {
            Drift::Structural { summary } => assert!(summary.contains("my-own-var"), "{summary}"),
            other => panic!("expected Structural, got {:?}", other),
        }
    }

    #[test]
    fn main_anchor_detection_sees_variables_and_ignores_comments() {
        let main = "variables:\n  a: &logsink-project-name \"x\"\n# not &commented-anchor\n";
        let anchors = anchors_defined_in_main(main);
        assert!(anchors.contains("logsink-project-name"));
        assert!(!anchors.contains("commented-anchor"));
    }
}