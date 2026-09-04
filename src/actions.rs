//! `satz run-actions` — running the deployment steps that have no provider resource.
//!
//! An action is the deployment's `unsafe` block. Satz resolves its arguments from the
//! estate's own parameters and runs it on demand; it claims nothing about what the
//! script did, records nothing, and no action can ever be a witness. Everything the
//! compliance plane says is still said about declared resources only.
//!
//! Three modes, and the difference between them is which arguments are passed:
//!
//! * **plan** (default) — resolve and print. Nothing is spawned.
//! * `--check` — spawn with `args` only, the action's own dry-run form. Whether that
//!   form is side-effect-free is the ACTION's contract, not satz's: satz cannot know
//!   what a script does and does not pretend to.
//! * `--execute` — spawn with `args` + `execute_args`, the form that writes.
//!
//! The executable is located the way a `use`d file is located — the declaring file's
//! directory, then the configured include dirs — so a pack that ships a script is
//! self-contained and an estate's own action reads relative to the estate file.

use satz_core::pipeline::ResolvedAction;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Resolve and print; spawn nothing.
    Plan,
    /// Spawn with `args` only.
    Check,
    /// Spawn with `args` + `execute_args`.
    Execute,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Check => "check",
            Mode::Execute => "execute",
        }
    }
}

pub(crate) struct RunOptions<'a> {
    pub mode: Mode,
    /// `--only a,b` — run just these names. `None` means all.
    pub only: Option<Vec<String>>,
    /// `--phase before-apply|after-apply`. `None` means both.
    pub phase: Option<String>,
    /// Global `--no-actions`: nothing is spawned, whatever the mode.
    pub no_actions: bool,
    /// Global `--no-pack-actions`: only the estate's own actions are considered.
    pub no_pack_actions: bool,
    /// The directory holding `config.toml` — the working directory for every action.
    pub estate_root: &'a Path,
    /// Where `use` paths are searched, in order, after the declaring file's directory.
    pub include_dirs: &'a [PathBuf],
    pub estate_file: &'a Path,
    pub hcl_dir: &'a Path,
}

/// Lexical cleanup, for display and for spawning: `yaml/../scripts/x.sh` names the
/// same file as `scripts/x.sh`, and only one of the two is readable in a warning
/// about something that is about to be executed. Purely textual — no symlink
/// resolution and no touching the filesystem, so what is printed is what was written.
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(c),
            },
            other => out.push(other),
        }
    }
    if out.is_empty() {
        return PathBuf::from(".");
    }
    out.iter().collect()
}

/// Make a path absolute without touching the filesystem.
///
/// An action is spawned with `current_dir` set to the estate root, and a RELATIVE
/// program path combined with `current_dir` is explicitly unspecified across
/// platforms in std — it may resolve against the parent's cwd or the child's. Satz
/// resolves it itself so there is exactly one answer, on every platform, and so the
/// path in the warning is the path that runs.
fn absolutize(p: &Path) -> Result<PathBuf, String> {
    if p.is_absolute() {
        return Ok(lexical_normalize(p));
    }
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the working directory: {}", e))?;
    Ok(lexical_normalize(&cwd.join(p)))
}

/// Shorten an absolute path for printing when it sits under the estate root. The
/// full path is still what gets executed; this only keeps the listing readable.
fn display_path(exe: &Path, estate_root: &Path) -> String {
    match absolutize(estate_root) {
        Ok(root) => match exe.strip_prefix(&root) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => exe.display().to_string(),
        },
        Err(_) => exe.display().to_string(),
    }
}

/// Where an action's executable actually is.
///
/// `ResolvedAction::file` is the path as the front end knew it: an absolute or
/// cwd-relative path for the estate itself, and the literal `use` path for a pack.
/// Joining `run` onto that file's directory therefore yields the same relative shape
/// the loader used, which is then tried against the same roots the loader tries. A
/// pack found under an include dir finds its script beside itself.
fn locate(a: &ResolvedAction, opts: &RunOptions) -> Result<PathBuf, String> {
    if Path::new(&a.run).is_absolute() {
        let p = PathBuf::from(&a.run);
        return if p.exists() {
            absolutize(&p)
        } else {
            Err(format!("run = \"{}\" does not exist", a.run))
        };
    }
    let rel = Path::new(&a.file).parent().unwrap_or(Path::new("")).join(&a.run);
    if rel.is_absolute() {
        return if rel.exists() {
            absolutize(&rel)
        } else {
            Err(format!("run = \"{}\" resolves to {}, which does not exist", a.run, rel.display()))
        };
    }
    let mut tried = Vec::new();
    let mut candidates = vec![opts.estate_root.join(&rel)];
    candidates.extend(opts.include_dirs.iter().map(|d| d.join(&rel)));
    for c in candidates {
        if c.exists() {
            return absolutize(&c);
        }
        tried.push(lexical_normalize(&c).display().to_string());
    }
    Err(format!(
        "run = \"{}\" not found. Looked in:\n      {}",
        a.run,
        tried.join("\n      ")
    ))
}

/// A file satz is about to execute must already be executable.
///
/// It is not chmod-ed here on purpose. `get-presets` downloads preset blobs over HTTP
/// and cannot carry a mode bit, so a pack-shipped script arrives at 0644 — and making
/// a file downloaded from upstream executable should stay a deliberate act by the
/// person who read it, not a side effect of running satz.
#[cfg(unix)]
fn check_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "{} is not executable.\n      chmod +x {}\n      (satz does not set the bit itself: for a script that came from `get-presets`, \
             making it executable is a decision, not a formality)",
            path.display(),
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Render one argument for display so a printed command line can be pasted back into
/// a shell unchanged.
fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn command_line(exe: &Path, args: &[String], estate_root: &Path) -> String {
    let mut out = shell_quote(&display_path(exe, estate_root));
    for a in args {
        out.push(' ');
        out.push_str(&shell_quote(a));
    }
    out
}

/// Phase first, then declaration order — the order `FrontEnd::actions` already carries
/// (the estate's own, then `use`-visit order).
fn phase_rank(phase: &str) -> u8 {
    match phase {
        "before-apply" => 0,
        _ => 1,
    }
}

pub(crate) fn run(actions: &[ResolvedAction], opts: &RunOptions) -> Result<(), String> {
    if actions.is_empty() {
        println!("no actions declared by this estate.");
        return Ok(());
    }

    let mut selected: Vec<&ResolvedAction> = Vec::new();
    let mut skipped_pack = 0usize;
    let mut skipped_filter = 0usize;
    for a in actions {
        if opts.no_pack_actions && a.from_pack {
            skipped_pack += 1;
            continue;
        }
        if let Some(only) = &opts.only {
            if !only.iter().any(|n| n == &a.name) {
                skipped_filter += 1;
                continue;
            }
        }
        if let Some(p) = &opts.phase {
            if &a.phase != p {
                skipped_filter += 1;
                continue;
            }
        }
        selected.push(a);
    }
    // A stable sort keeps declaration order inside a phase.
    selected.sort_by_key(|a| phase_rank(&a.phase));

    if let Some(only) = &opts.only {
        for n in only {
            if !actions.iter().any(|a| &a.name == n) {
                return Err(format!(
                    "--only \"{}\": no action by that name. Declared: {}",
                    n,
                    actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }

    // Locate and vet every action BEFORE spawning any of them: a run that dies
    // half-way because the fourth script was not executable has already changed the
    // organisation with the first three.
    let mut plan: Vec<(&ResolvedAction, PathBuf, Vec<String>)> = Vec::new();
    for a in &selected {
        let exe = locate(a, opts).map_err(|e| format!("action \"{}\" ({}:{}): {}", a.name, a.file, a.line, e))?;
        if opts.mode != Mode::Plan && !opts.no_actions {
            check_executable(&exe)
                .map_err(|e| format!("action \"{}\" ({}:{}): {}", a.name, a.file, a.line, e))?;
        }
        let mut args = a.args.clone();
        if opts.mode == Mode::Execute {
            args.extend(a.execute_args.iter().cloned());
        }
        plan.push((a, exe, args));
    }

    println!(
        "{} action(s) declared, {} selected — mode: {}",
        actions.len(),
        selected.len(),
        if opts.no_actions { "disabled (--no-actions)" } else { opts.mode.as_str() }
    );
    if skipped_pack > 0 {
        println!("  {} pack-declared action(s) skipped (--no-pack-actions)", skipped_pack);
    }
    if skipped_filter > 0 {
        println!("  {} action(s) filtered out", skipped_filter);
    }

    for (a, exe, args) in &plan {
        println!();
        println!("action \"{}\"  [{}]", a.name, a.phase);
        println!(
            "  declared in  {}:{}{}",
            a.file,
            a.line,
            if a.from_pack { "  (from a pack)" } else { "" }
        );
        println!("  reason       {}", a.reason);
        println!("  runs         {}", command_line(exe, args, opts.estate_root));
        if opts.mode != Mode::Execute && !a.execute_args.is_empty() {
            let mut full = a.args.clone();
            full.extend(a.execute_args.iter().cloned());
            println!("  --execute    {}", command_line(exe, &full, opts.estate_root));
        }
    }

    if opts.no_actions {
        println!();
        println!("nothing was run: --no-actions is set.");
        return Ok(());
    }
    if opts.mode == Mode::Plan {
        println!();
        println!(
            "nothing was run. `--check` runs each action's own dry-run form; `--execute` runs the form that writes."
        );
        return Ok(());
    }

    for (a, exe, args) in &plan {
        println!();
        println!("==> {} ({})", a.name, command_line(exe, args, opts.estate_root));
        let status = std::process::Command::new(exe)
            .args(args)
            .current_dir(opts.estate_root)
            .env("SATZ_ACTION", &a.name)
            .env("SATZ_PHASE", &a.phase)
            .env("SATZ_MODE", opts.mode.as_str())
            .env("SATZ_ESTATE", opts.estate_file)
            .env("SATZ_HCL_DIR", opts.hcl_dir)
            .status()
            .map_err(|e| format!("action \"{}\": could not run {}: {}", a.name, exe.display(), e))?;
        match status.code() {
            Some(0) => {}
            // Propagate rather than wrap. An action's exit code is its own contract
            // with whoever is reading it, and the remaining actions do not run: a
            // failed step is not a reason to keep changing the organisation.
            Some(code) => {
                eprintln!("action \"{}\" failed (exit {}) — stopping, {} action(s) not run", a.name, code, plan.len() - 1);
                std::process::exit(code);
            }
            None => return Err(format!("action \"{}\" was killed by a signal", a.name)),
        }
    }
    println!();
    println!("{} action(s) ran.", plan.len());
    Ok(())
}

/// The warning every estate-compiling command prints, mirroring the raw-HCL one.
///
/// Unlike `hcl trust`, a `reason` does not downgrade this to a note: HCL only deploys,
/// an action executes. The declaring file is on the line because the difference
/// between "my estate declares this" and "a pack I downloaded declares this" is the
/// whole of the trust story.
pub(crate) fn warn(actions: &[ResolvedAction]) {
    for a in actions {
        eprintln!(
            "warning: action \"{}\" declared in {}:{}{} — `satz run-actions` will execute {}\n  reason: {}",
            a.name,
            a.file,
            a.line,
            if a.from_pack { " (from a pack)" } else { "" },
            a.run,
            a.reason
        );
    }
    if actions.iter().any(|a| a.from_pack) {
        eprintln!(
            "note: --no-pack-actions ignores pack-declared actions, --no-actions disables all execution, \
             --no-action-warnings silences this."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, file: &str, run: &str, from_pack: bool) -> ResolvedAction {
        ResolvedAction {
            file: file.to_string(),
            name: name.to_string(),
            reason: "no provider resource".to_string(),
            run: run.to_string(),
            args: vec![],
            execute_args: vec![],
            phase: "after-apply".to_string(),
            from_pack,
            line: 1,
        }
    }

    #[test]
    fn quoting_keeps_a_plain_argument_plain_and_wraps_the_rest() {
        assert_eq!(shell_quote("--organization"), "--organization");
        assert_eq!(shell_quote("organizations/123"), "organizations/123");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_pack_script_is_looked_for_beside_its_pack_not_beside_the_estate() {
        let dir = std::env::temp_dir().join(format!("satz-actions-{}", std::process::id()));
        let pack_dir = dir.join("presets").join("scc");
        std::fs::create_dir_all(&pack_dir).unwrap();
        let script = pack_dir.join("enable.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();

        let a = action("scc", "presets/scc/pack.satz", "enable.sh", true);
        let opts = RunOptions {
            mode: Mode::Plan,
            only: None,
            phase: None,
            no_actions: false,
            no_pack_actions: false,
            estate_root: &dir,
            include_dirs: &[],
            estate_file: Path::new("yaml/main.satz"),
            hcl_dir: Path::new("hcl"),
        };
        assert_eq!(locate(&a, &opts).unwrap(), dir.join("presets/scc/enable.sh"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_script_names_every_place_that_was_looked_in() {
        let a = action("nope", "yaml/main.satz", "missing.sh", false);
        let opts = RunOptions {
            mode: Mode::Plan,
            only: None,
            phase: None,
            no_actions: false,
            no_pack_actions: false,
            estate_root: Path::new("/nonexistent-satz-root"),
            include_dirs: &[],
            estate_file: Path::new("yaml/main.satz"),
            hcl_dir: Path::new("hcl"),
        };
        let e = locate(&a, &opts).unwrap_err();
        assert!(e.contains("missing.sh"), "{}", e);
        assert!(e.contains("Looked in"), "{}", e);
    }

    #[test]
    fn before_apply_sorts_ahead_of_after_apply() {
        assert!(phase_rank("before-apply") < phase_rank("after-apply"));
    }
}
