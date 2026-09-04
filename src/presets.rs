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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};


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

/// One changed param default: its name and the canonical value text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VarEntry {
    pub value: String,
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

// ---------------------------------------------------------------------------
// IO layer
// ---------------------------------------------------------------------------

/// Every pack SOURCE under `dir`: the `.satz` files. (`.yaml` files in a presets
/// dir — catalogs, `import-config.yaml` — are data refreshed as artifacts,
/// not packs to classify; the YAML pack dialect is gone.)
fn walk_preset_sources(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<(), BoxErr> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk_preset_sources(&path, base, out)?;
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.ends_with(".satz") {
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
        return Err(format!("{}: not a Satz estate — YAML estates are converted with `satz import <file>.yaml`, not checked", input.display()).into());
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
        // a file on the `use` graph that cannot be read or parsed leaves the
        // "used" set incomplete — and an incomplete set disarms the guard
        // (drift would read [unused], merge would overwrite a used pack)
        let src = crate::fsx::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let file = satz_core::satz::parse(&src).map_err(|e| format!("{}:{}: {}", path.display(), e.line, e.msg))?;
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
fn classify_source(local: &str, pristine: &str) -> Drift {
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
        .map(|(n, v)| (n.clone(), VarEntry { value: v.clone() }))
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
/// One default whose value differs from upstream.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ChangedDefault {
    pub name: String,
    pub value: String,
    /// The estate already pins this one, so the local edit changes nothing.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub overridden_in_estate: bool,
}

/// One preset file's verdict, as data. Every sentence the renderer prints is
/// derived from these fields — it reaches nothing else, so the text cannot say
/// anything the JSON omits.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PresetRow {
    pub file: String,
    /// in the estate's `use` graph
    pub included: bool,
    /// clean | stale | edited | fork | local-only | missing-locally
    pub status: &'static str,
    /// for `edited`: "variables only" | "structural"
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub local_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub upstream_version: Option<String>,
    /// for a `.local` fork: the pristine stem it forked from
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fork_of: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub changed_defaults: Vec<ChangedDefault>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structural_summary: Option<String>,
    /// a stale pristine copy whose `.local` fork exists: leave it, it is the
    /// branch point the eventual merge needs
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub baseline_for_fork: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub same_version_different_content: bool,
    /// stale, but the compiled form is identical — comment or format churn
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub semantics_unchanged: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct CheckPresetsSummary {
    pub clean: usize,
    pub stale: usize,
    pub drift_in_use: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct CheckPresetsReport {
    pub presets_dir: String,
    pub packs: Vec<PresetRow>,
    pub summary: CheckPresetsSummary,
}

/// The per-file detail of a drift verdict, shared by the STALE and EDITED arms
/// so a version gap and a local edit are described the same way — only the
/// headline and the remedy differ.
fn render_drift_detail(row: &PresetRow, out: &mut String) {
    if let Some(summary) = &row.structural_summary {
        out.push_str(&format!("    {summary} — not mechanically migratable, review by hand\n"));
        return;
    }
    if row.changed_defaults.is_empty() {
        return;
    }
    // A STALE pack's local values are the OLD defaults. Telling the operator
    // to pin them in the estate would freeze exactly what they are trying to
    // move off, so name what moved and stop there — the remedy is adoption.
    if row.status == "stale" {
        let names: Vec<&str> = row.changed_defaults.iter().map(|c| c.name.as_str()).collect();
        out.push_str(&format!("    default(s) changed upstream: {}\n", names.join(", ")));
        return;
    }
    let mut to_add = Vec::new();
    for c in &row.changed_defaults {
        if c.overridden_in_estate {
            out.push_str(&format!(
                "    {} ={}\n      — already overridden in the estate's params; the local edit is redundant\n",
                c.name,
                indented(&c.value)
            ));
        } else {
            to_add.push(c);
        }
    }
    if !to_add.is_empty() {
        // Pin the value in the estate so the pack stays pristine.
        out.push_str("    override in the estate's `params { … }` block, then restore the preset with `get-presets`:\n");
        for c in to_add {
            out.push_str(&format!("      {} ={}\n", c.name, indented(&c.value)));
        }
    }
}

/// Render the drift report for a terminal. Takes the report and NOTHING else.
pub(crate) fn render_check_presets(r: &CheckPresetsReport) -> String {
    let mut out = format!("\ncheck-presets: comparing {} against upstream\n\n", r.presets_dir);
    for row in &r.packs {
        let tag = if row.included { " [included]" } else { "" };
        let vtag = match (&row.local_version, &row.upstream_version) {
            (Some(l), Some(u)) if l != u => format!(" — local v{l}, upstream v{u}"),
            (Some(l), _) => format!(" — v{l}"),
            _ => String::new(),
        };
        match row.status {
            // a clean, current preset is counted, not printed
            "clean" => {}
            "fork" => {
                let stem = row.fork_of.clone().unwrap_or_default();
                out.push_str(&format!(
                    "  fork{}: {} (deliberate fork of {} — updates to the upstream file accumulate in {}.diff.satz)\n",
                    tag, row.file, stem, stem
                ));
            }
            "local-only" => {
                out.push_str(&format!("  local-only{}: {} (not an upstream preset — kept as-is)\n", tag, row.file));
            }
            "missing-locally" => {
                out.push_str(&format!(
                    "  missing locally: {} (new upstream preset — `get-presets` fetches it)\n",
                    row.file
                ));
            }
            "stale" if row.semantics_unchanged => {
                out.push_str(&format!(
                    "  STALE{}: {}{} — no semantic change; adopting costs nothing\n",
                    tag, row.file, vtag
                ));
            }
            "stale" => {
                out.push_str(&format!("  STALE{}: {}{}\n", tag, row.file, vtag));
                render_drift_detail(row, &mut out);
                if row.baseline_for_fork {
                    let fork = fork_sibling(std::path::Path::new(&row.file));
                    out.push_str(&format!("    the estate runs {} — this pristine copy is that fork's\n", fork.display()));
                    out.push_str("    baseline. Leave it; `merge-presets` refreshes it and rewrites the .diff.\n");
                } else {
                    out.push_str("    a newer release exists — the differences above are the version gap.\n");
                    out.push_str("    Adopt it (copy the pristine file in), or `merge-presets` if this copy may\n");
                    out.push_str("    ALSO have been edited. See docs/presets-workflow.md.\n");
                }
            }
            _ => {
                out.push_str(&format!(
                    "  EDITED ({}){}: {}{}\n",
                    row.kind.unwrap_or("structural"),
                    tag,
                    row.file,
                    vtag
                ));
                render_drift_detail(row, &mut out);
                if row.same_version_different_content {
                    out.push_str("    same version as upstream, different content — a local edit (or an\n");
                    out.push_str("    upstream release that changed without a version bump).\n");
                }
            }
        }
    }

    out.push_str(&format!(
        "\n{} preset(s) clean, {} behind upstream.\n",
        r.summary.clean, r.summary.stale
    ));
    if r.summary.stale > 0 {
        out.push_str("`clean` means unedited, not current — the version line is the staleness signal.\n");
    }
    if r.summary.drift_in_use {
        out.push_str("Drift detected in included preset(s) — exit code 1.\n");
    } else {
        out.push_str("No drift in included presets.\n");
    }
    out
}

/// Compare the local preset library against pristine upstream. Downloads the
/// pristine copy, then computes — no printing, so the same verdicts serve the
/// terminal, `--format json` and (later) an MCP tool.
pub(crate) async fn check_presets_report(
    input: &Path,
    presets_dir: &str,
    include_dirs: &[String],
    pristine_dir: Option<PathBuf>,
) -> Result<CheckPresetsReport, BoxErr> {
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

    // Which packs does the estate actually use? Its `use` graph.
    let included = used_preset_files(input, presets_dir, include_dirs)?;

    // The params the estate itself declares — an edited pack default that the
    // estate already overrides is redundant, not a remedy to print.
    let estate_params: BTreeSet<String> = satz_core::satz::parse(&crate::fsx::read_to_string(input)?)
        .map(|f| f.params.iter().map(|(n, _, _)| n.clone()).collect())
        .unwrap_or_default();

    let mut local_files = Vec::new();
    walk_preset_sources(&local_base, &local_base, &mut local_files)?;
    let mut pristine_files = Vec::new();
    walk_preset_sources(&pristine_base, &pristine_base, &mut pristine_files)?;
    let local_set: BTreeSet<_> = local_files.into_iter().collect();
    let pristine_set: BTreeSet<_> = pristine_files.into_iter().collect();

    let mut summary = CheckPresetsSummary::default();
    let mut packs = Vec::new();

    for rel in local_set.union(&pristine_set) {
        // bookkeeping is not a preset
        if rel.starts_with(".base") {
            continue;
        }
        let in_use = included.contains(rel);
        let mut row = PresetRow {
            file: rel.display().to_string(),
            included: in_use,
            status: "clean",
            kind: None,
            local_version: None,
            upstream_version: None,
            fork_of: None,
            changed_defaults: Vec::new(),
            structural_summary: None,
            baseline_for_fork: false,
            same_version_different_content: false,
            semantics_unchanged: false,
        };

        match (local_set.contains(rel), pristine_set.contains(rel)) {
            (true, false) => {
                let name = rel.file_name().unwrap_or_default().to_string_lossy();
                if name.contains(".local.") {
                    row.status = "fork";
                    row.fork_of = Some(name.split(".local.").next().unwrap_or_default().to_string());
                } else if name.ends_with(".diff.satz") {
                    // ledger files are artifacts of merge-presets, not presets
                    continue;
                } else {
                    row.status = "local-only";
                }
            }
            (false, true) => row.status = "missing-locally",
            (true, true) => {
                let local_text = crate::fsx::read_to_string(local_base.join(rel))?;
                let pristine_text = crate::fsx::read_to_string(pristine_base.join(rel))?;
                let drift = classify_source(&local_text, &pristine_text);
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
                row.local_version = lv.clone();
                row.upstream_version = uv.clone();
                match &drift {
                    Drift::VariablesOnly(changed) => {
                        row.changed_defaults = changed
                            .iter()
                            .map(|(name, entry)| ChangedDefault {
                                name: name.clone(),
                                value: entry.value.clone(),
                                overridden_in_estate: estate_params.contains(name),
                            })
                            .collect();
                    }
                    Drift::Structural { summary } => row.structural_summary = Some(summary.clone()),
                    Drift::Clean => {}
                }
                match (&drift, behind) {
                    (Drift::Clean, false) => {
                        summary.clean += 1;
                        row.status = "clean";
                    }
                    // Version moved, semantics did not: comment or formatting
                    // churn upstream. Nothing to decide, nothing to fail.
                    (Drift::Clean, true) => {
                        summary.stale += 1;
                        row.status = "stale";
                        row.semantics_unchanged = true;
                    }
                    (_, true) => {
                        summary.stale += 1;
                        row.status = "stale";
                        if in_use {
                            summary.drift_in_use = true;
                        }
                        // A pristine file whose `.local` fork exists is not a
                        // candidate for adoption: the estate runs the fork, so
                        // overwriting this copy changes nothing it emits AND
                        // destroys the branch point the eventual merge needs.
                        row.baseline_for_fork = local_set.contains(&fork_sibling(rel));
                    }
                    (Drift::VariablesOnly(_) | Drift::Structural { .. }, false) => {
                        if in_use {
                            summary.drift_in_use = true;
                        }
                        row.status = "edited";
                        row.kind = Some(match &drift {
                            Drift::VariablesOnly(_) => "variables only",
                            _ => "structural",
                        });
                        row.same_version_different_content = lv.is_some() && lv == uv;
                    }
                }
            }
            (false, false) => unreachable!(),
        }
        packs.push(row);
    }

    Ok(CheckPresetsReport {
        presets_dir: local_base.display().to_string(),
        packs,
        summary,
    })
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
            // the library is more than packs: docs, the import config and
            // catalogs (.yaml), the CAI asset-type list (.txt)
            if name.ends_with(".md") || name.ends_with(".yaml") || name.ends_with(".txt") {
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
        (Some(est), false) => Some(crate::transpile_sorted_b(est, tool_config, runtime_config)?),
        _ => None,
    };
    let estate_dirty = match estate.as_deref() {
        Some(e) => is_git_dirty(e)?,
        None => false,
    };

    // ---- upstream inventory --------------------------------------------------
    let mut upstream_files = Vec::new();
    let mut stack = vec![pristine.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue; }
            let name = p.to_string_lossy();
            // a fork or a delta in the UPSTREAM tree would make pack_stem
            // collapse it onto the user's own `.local.satz` and overwrite it
            if name.ends_with(".local.satz") || name.ends_with(".diff.satz") {
                return Err(format!("merge-presets: {} is a local fork/delta inside the pristine dir — upstream carries pristine packs only", p.display()).into());
            }
            if name.ends_with(".satz") || name.ends_with(".md") || name.ends_with(".yaml") || name.ends_with(".txt") {
                upstream_files.push(p.strip_prefix(&pristine)?.to_path_buf());
            }
        }
    }
    upstream_files.sort();
    upstream_files.dedup();
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
        // docs and data (catalogs, import-config) are artifacts: upstream
        // owns them, nothing to fork
        let is_artifact = fname.ends_with(".md") || fname.ends_with(".yaml");
        if is_artifact {
            if report_only { println!("  would update artifact {}", rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            artifacts += 1;
            continue;
        }

        // semantic comparison in the canonical form (the parsed AST, printed
        // without comments/formatting/version — same as check-presets)
        let sem_equal = match (satz_core::satz::parse(&lo), satz_core::satz::parse(&up)) {
            (Ok(a), Ok(b)) => satz_core::satz::canonical(&a) == satz_core::satz::canonical(&b),
            _ => false,
        };
        if sem_equal {
            if report_only { println!("  would update {} (doc/format only)", rel.display()); continue; }
            crate::fsx::write(&lo_path, up.as_bytes())?;
            doc_only += 1;
            continue;
        }

        // version hygiene cross-check (packs carry in-file versions)
        let (v_lo, v_up) = (satz_version(&lo), satz_version(&up));
        if v_lo.is_some() && v_lo == v_up {
            println!("  WARNING {}: content changed semantically but the pack version did not — upstream release-hygiene bug", rel.display());
            needs_attention = true;
        }

        let stem = pack_stem(rel).unwrap_or_else(|| rel.clone());
        let fork_rel = PathBuf::from(format!("{}.local.satz", stem.display()));
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
            let after = crate::transpile_sorted_b(&est, tool_config, runtime_config)?;
            println!("\n  emission delta after adoption:");
            print!("{}", emission_delta(&before, &after));
            println!("  hcl/ on disk is NOT regenerated by this command — run `satz transpile`,");
            println!("  read `git diff hcl/main.tf`, then `tofu plan` before applying.");
        }
    }

    // the self-verifying estate edit: output must be byte-identical
    if estate_edited {
        let est = estate.clone().unwrap();
        let rollback = |journal: &Vec<(PathBuf, String)>, created: &Vec<PathBuf>| -> Result<(), BoxErr> {
            for p in created { let _ = std::fs::remove_file(p); }
            for (p, content) in journal.iter().rev() { crate::fsx::write(p, content.as_bytes())?; }
            Ok(())
        };
        // a repoint that does not even transpile is rolled back the same way
        // as one that transpiles differently — the estate is never left edited
        let after = match crate::transpile_sorted_b(&est, tool_config, runtime_config) {
            Ok(a) => a,
            Err(e) => {
                rollback(&journal, &created)?;
                return Err(format!("merge-presets: the repointed estate does not transpile ({}) — rolled back everything", e).into());
            }
        };
        if baseline.as_deref() != Some(after.as_str()) {
            rollback(&journal, &created)?;
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
    let mut found = Vec::new();
    for e in std::fs::read_dir(yaml_dir).ok()?.flatten() {
        let p = e.path();
        let name = p.to_string_lossy().to_string();
        if name.contains(".local.") || p.extension().and_then(|x| x.to_str()) != Some("satz") {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&p) {
            if let Ok(f) = satz_core::satz::parse(&src) {
                if f.estate.is_some() && !f.is_pack {
                    found.push(p);
                }
            }
        }
    }
    if found.len() == 1 { found.pop() } else { None }
}

/// rel path -> pack stem: strip the `.satz` suffix and a `.local` marker.
fn pack_stem(rel: &Path) -> Option<PathBuf> {
    let s = rel.to_string_lossy();
    let stem = s.strip_suffix(".satz")?;
    let stem = stem.strip_suffix(".local").unwrap_or(stem);
    Some(PathBuf::from(stem))
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

/// "clean" is an answer git gave, never a fallback: outside a repository or
/// without git the estate edit has no undo, so the question is an error.
fn is_git_dirty(path: &Path) -> Result<bool, String> {
    let dir = path.parent().ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    let out = std::process::Command::new("git")
        .arg("-C").arg(dir)
        .args(["status", "--porcelain", "--"])
        .arg(path)
        .output()
        .map_err(|e| format!("git status on {}: {} — merge-presets edits the estate and needs git for the undo", path.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "git status on {} failed: {} — merge-presets edits the estate and needs a repository for the undo",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(!out.stdout.is_empty())
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
                if candidates.iter().any(|c| {
                    std::fs::canonicalize(c).map(|cc| cc == canon_target).unwrap_or(false)
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
            // orphaned delta: the fork was adopted/deleted -> the diff goes too
            if let Some(stem_name) = name.strip_suffix(".diff.satz") {
                let has_fork = p.with_file_name(format!("{}.local.satz", stem_name)).exists();
                if !has_fork {
                    let _ = std::fs::remove_file(&p);
                    println!("  removed orphaned {} (fork adopted)", p.display());
                }
                continue;
            }
            let Some(stem_name) = name.strip_suffix(".local.satz") else { continue };
            let pristine = p.with_file_name(format!("{}.satz", stem_name));
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
mod tests {
    use super::{
        ChangedDefault, CheckPresetsReport, CheckPresetsSummary, PresetRow, render_check_presets,
    };

    fn prow(file: &str, status: &'static str) -> PresetRow {
        PresetRow {
            file: file.into(),
            included: true,
            status,
            kind: None,
            local_version: None,
            upstream_version: None,
            fork_of: None,
            changed_defaults: Vec::new(),
            structural_summary: None,
            baseline_for_fork: false,
            same_version_different_content: false,
            semantics_unchanged: false,
        }
    }

    fn report(packs: Vec<PresetRow>, summary: CheckPresetsSummary) -> CheckPresetsReport {
        CheckPresetsReport { presets_dir: "presets".into(), packs, summary }
    }

    /// The estate's smoke fixture is all-clean, so these arms would otherwise
    /// only ever run against a customer's library. One line per verdict, pinned.
    #[test]
    fn every_verdict_renders_its_own_shape() {
        let mut stale = prow("a.satz", "stale");
        stale.local_version = Some("1.0".into());
        stale.upstream_version = Some("1.1".into());
        stale.changed_defaults = vec![ChangedDefault {
            name: "region".into(),
            value: "\"eu\"".into(),
            overridden_in_estate: false,
        }];

        let mut churn = prow("b.satz", "stale");
        churn.local_version = Some("2.0".into());
        churn.upstream_version = Some("2.1".into());
        churn.semantics_unchanged = true;

        let mut baseline = prow("c.satz", "stale");
        baseline.baseline_for_fork = true;

        let mut edited = prow("d.satz", "edited");
        edited.kind = Some("variables only");
        edited.local_version = Some("1.0".into());
        edited.upstream_version = Some("1.0".into());
        edited.same_version_different_content = true;
        edited.changed_defaults = vec![
            ChangedDefault { name: "kept".into(), value: "\"x\"".into(), overridden_in_estate: false },
            ChangedDefault { name: "pinned".into(), value: "\"y\"".into(), overridden_in_estate: true },
        ];

        let mut structural = prow("e.satz", "edited");
        structural.kind = Some("structural");
        structural.structural_summary = Some("2 resource(s) differ".into());

        let mut fork = prow("f.local.satz", "fork");
        fork.fork_of = Some("f".into());

        let out = render_check_presets(&report(
            vec![
                prow("clean.satz", "clean"),
                stale,
                churn,
                baseline,
                edited,
                structural,
                fork,
                prow("g.satz", "local-only"),
                prow("h.satz", "missing-locally"),
            ],
            CheckPresetsSummary { clean: 1, stale: 3, drift_in_use: true },
        ));

        // a clean, current preset is counted and NOT printed
        assert!(!out.contains("clean.satz"), "{out}");
        assert!(out.contains("STALE [included]: a.satz — local v1.0, upstream v1.1"), "{out}");
        // a stale pack's local values are the OLD defaults: name what moved, do not
        // tell the operator to pin them
        assert!(out.contains("default(s) changed upstream: region"), "{out}");
        assert!(!out.contains("override in the estate's `params { … }` block, then restore the preset with `get-presets`:\n      region"), "{out}");
        assert!(out.contains("no semantic change; adopting costs nothing"), "{out}");
        assert!(out.contains("this pristine copy is that fork's"), "{out}");
        assert!(out.contains("EDITED (variables only) [included]: d.satz — v1.0"), "{out}");
        assert!(out.contains("already overridden in the estate's params"), "{out}");
        assert!(out.contains("same version as upstream, different content"), "{out}");
        assert!(out.contains("2 resource(s) differ — not mechanically migratable"), "{out}");
        assert!(out.contains("fork [included]: f.local.satz (deliberate fork of f"), "{out}");
        assert!(out.contains("local-only [included]: g.satz"), "{out}");
        assert!(out.contains("missing locally: h.satz"), "{out}");
        assert!(out.contains("1 preset(s) clean, 3 behind upstream."), "{out}");
        assert!(out.contains("Drift detected in included preset(s) — exit code 1."), "{out}");
    }

    /// Drift outside the estate's `use` graph is reported but must not fail CI —
    /// a customer's library carries packs they do not run.
    #[test]
    fn drift_outside_the_use_graph_does_not_fail() {
        let out = render_check_presets(&report(
            vec![prow("x.satz", "edited")],
            CheckPresetsSummary { clean: 0, stale: 0, drift_in_use: false },
        ));
        assert!(out.contains("No drift in included presets."), "{out}");
        assert!(!out.contains("exit code 1"), "{out}");
    }

    use super::*;

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
        let drifted = PACK.replace(r#"enforce = "TRUE""#, r#"enforce = "FALSE""#);
        assert_ne!(drifted, PACK, "the fixture must actually change");
        match classify_source(&drifted, PACK) {
            Drift::Structural { .. } => {}
            other => panic!("expected Structural drift for a satz pack, got {:?}", other),
        }
    }

    /// A pack `params` default is the "variables" half of the canonical form,
    /// so the variables-only remedy (pin it in the estate, keep the pack
    /// pristine) applies to Satz packs — under the param's own name.
    #[test]
    fn satz_param_drift_is_variables_only() {
        let drifted = PACK.replace(r#""demo-audit""#, r#""customer-audit""#);
        match classify_source(&drifted, PACK) {
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
        let bumped = PACK.replacen("version \"", "version \"9", 1);
        assert_ne!(bumped, PACK);
        assert_eq!(classify_source(&bumped, PACK), Drift::Clean);
    }

    /// Comment and formatting churn upgrades silently — same rule `merge-presets`
    /// applies, so the two commands agree.
    #[test]
    fn satz_comment_churn_is_clean() {
        let commented = format!("// a local note
{}", PACK);
        assert_eq!(classify_source(&commented, PACK), Drift::Clean);
    }

}