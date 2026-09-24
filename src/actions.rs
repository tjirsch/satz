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
//!
//! The file's extension decides how it is launched, and that is the whole mechanism —
//! there is no `interpreter` key and no per-OS variant. A `.py` action runs through
//! `uv run --script`, so one file runs on every platform satz ships for; anything else
//! is spawned directly, which on Windows means a `.sh` is refused before the spawn.

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
    let mut tried: Vec<String> = Vec::new();
    let mut candidates = vec![opts.estate_root.join(&rel)];
    candidates.extend(opts.include_dirs.iter().map(|d| d.join(&rel)));
    for c in candidates {
        if c.exists() {
            return absolutize(&c);
        }
        // The estate root is usually also an include dir, so the same candidate
        // comes round twice; a "looked in" list that repeats itself reads like a
        // bug in the search rather than a missing file.
        let shown = lexical_normalize(&c).display().to_string();
        if !tried.contains(&shown) {
            tried.push(shown);
        }
    }
    Err(format!(
        "run = \"{}\" not found. Looked in:\n      {}",
        a.run,
        tried.join("\n      ")
    ))
}

/// What runs a Python action. Spawned by name, so it is found on PATH.
const UV: &str = "uv";

fn is_python(exe: &Path) -> bool {
    exe.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("py"))
}

/// The process satz spawns for one action: the program, and the whole argument vector
/// it is spawned with. Built in one place so the command line that is PRINTED and the
/// process that runs can never disagree.
///
/// A `.py` action becomes `uv run --script <file> <args>`. `--script` is what makes the
/// run reproducible wherever it happens: the file is a standalone script with its own
/// PEP 723 dependencies, resolved by uv, and a `pyproject.toml` that happens to sit in
/// the estate root has no say in it.
fn spawn_command(exe: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    if is_python(exe) {
        let mut argv = vec!["run".to_string(), "--script".to_string(), exe.display().to_string()];
        argv.extend(args.iter().cloned());
        return (PathBuf::from(UV), argv);
    }
    (exe.to_path_buf(), args.to_vec())
}

/// Is a program on PATH? Windows spawns `uv.exe`, unix `uv`, so both names are tried.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| dir.join(program).is_file() || dir.join(format!("{}.exe", program)).is_file())
}

/// What has to be true before an action can be spawned.
///
/// A Python action is READ by uv, so the executable bit says nothing about whether it
/// can run; what has to be there is uv. Everything else is spawned as a program and
/// must be one.
fn check_runnable(path: &Path) -> Result<(), String> {
    runnable(path, on_path(UV))
}

/// [`check_runnable`] with the one fact it reads from the machine handed in, so
/// both branches are judged whatever this machine has installed.
fn runnable(path: &Path, uv_on_path: bool) -> Result<(), String> {
    if is_python(path) {
        return check_uv(path, uv_on_path);
    }
    check_executable(path)
}

/// `uv` is what runs a Python action, and there is no second way: a `python` or
/// `python3` on PATH is a different interpreter with a different set of packages, and
/// running one instead would mean the script that was tested is not the script that
/// ran.
fn check_uv(script: &Path, uv_on_path: bool) -> Result<(), String> {
    if uv_on_path {
        return Ok(());
    }
    Err(format!(
        "{} is a Python action, and satz runs one with `uv`, which is not on PATH.\n      \
         Install uv — `brew install uv`, `pipx install uv`, or the installer uv's own documentation names — and run this again.\n      \
         satz does not fall back to `python` or `python3`: that is a different interpreter with different packages.",
        script.display()
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

/// Windows runs an executable, not a script: a `.sh` spawned directly fails with an OS
/// error that names neither. Refused before the spawn, naming how to run it by hand.
#[cfg(not(unix))]
fn check_executable(path: &Path) -> Result<(), String> {
    windows_runnable(path)
}

#[cfg_attr(unix, allow(dead_code))]
fn windows_runnable(path: &Path) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("exe" | "cmd" | "bat" | "com")) {
        return Ok(());
    }
    Err(format!(
        "{} is not a Windows executable, and satz runs an action's `run` directly — run it in a shell that can, e.g. `bash {}` from Git Bash or WSL. A `.py` action runs through `uv` on every platform",
        path.display(),
        path.display()
    ))
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

/// The command line as the operator reads it — the same program and arguments that are
/// spawned, with the script's path shortened where it sits under the estate root, so
/// what is printed can be pasted back into a shell.
fn command_line(exe: &Path, args: &[String], estate_root: &Path) -> String {
    let shown = PathBuf::from(display_path(exe, estate_root));
    let (program, argv) = spawn_command(&shown, args);
    let mut out = shell_quote(&program.display().to_string());
    for a in &argv {
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
            check_runnable(&exe)
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
        let (program, argv) = spawn_command(exe, args);
        let status = std::process::Command::new(&program)
            .args(&argv)
            .current_dir(opts.estate_root)
            .env("SATZ_ACTION", &a.name)
            .env("SATZ_PHASE", &a.phase)
            .env("SATZ_MODE", opts.mode.as_str())
            .env("SATZ_ESTATE", opts.estate_file)
            .env("SATZ_HCL_DIR", opts.hcl_dir)
            .status()
            .map_err(|e| {
                format!(
                    "action \"{}\": could not run {}: {}",
                    a.name,
                    command_line(exe, args, opts.estate_root),
                    e
                )
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_python_action_is_spawned_through_uv_and_everything_else_directly() {
        let args = vec!["--organization".to_string(), "123456789012".to_string()];

        let (program, argv) = spawn_command(Path::new("/estate/scripts/seed.py"), &args);
        assert_eq!(program, PathBuf::from("uv"));
        assert_eq!(
            argv,
            vec!["run", "--script", "/estate/scripts/seed.py", "--organization", "123456789012"]
        );

        let (program, argv) = spawn_command(Path::new("/estate/scripts/seed.sh"), &args);
        assert_eq!(program, PathBuf::from("/estate/scripts/seed.sh"));
        assert_eq!(argv, args);

        // --check is the same construction with `args` only: whatever the launcher,
        // the execute_args are not in the vector.
        let (_, argv) = spawn_command(Path::new("/estate/scripts/seed.py"), &args[..1]);
        assert_eq!(argv, vec!["run", "--script", "/estate/scripts/seed.py", "--organization"]);
    }

    #[test]
    fn the_printed_command_line_is_the_one_that_is_spawned() {
        // Built through `absolutize`, so the root is absolute in the platform's own
        // terms: on Windows a leading `/` is a root without a drive, which
        // `Path::is_absolute` rejects, and a unix-shaped literal would make this test
        // assert the unshortened fallback instead of the shortening.
        let root = absolutize(Path::new("estate")).unwrap();
        let py = root.join("scripts").join("seed.py");
        let sh = root.join("scripts").join("seed.sh");
        let args = vec!["--organization".to_string(), "123456789012".to_string()];

        // The path is printed in the platform's own shape, and `shell_quote` wraps a
        // Windows one because a backslash is not a plain character.
        #[cfg(unix)]
        let (py_shown, sh_shown) = ("scripts/seed.py", "scripts/seed.sh");
        #[cfg(windows)]
        let (py_shown, sh_shown) = (r"'scripts\seed.py'", r"'scripts\seed.sh'");

        assert_eq!(
            command_line(&py, &args, &root),
            format!("uv run --script {py_shown} --organization 123456789012")
        );
        assert_eq!(
            command_line(&sh, &args, &root),
            format!("{sh_shown} --organization 123456789012")
        );

        // …and what is spawned is that same file, at its full path: the printed line
        // differs from the spawned vector in the shortening and nothing else.
        let (program, argv) = spawn_command(&py, &args);
        assert_eq!(program, PathBuf::from(UV));
        let want: Vec<String> = vec![
            "run".to_string(),
            "--script".to_string(),
            py.display().to_string(),
            "--organization".to_string(),
            "123456789012".to_string(),
        ];
        assert_eq!(argv, want);
        assert_eq!(spawn_command(&sh, &args), (sh.clone(), args));
    }

    #[test]
    fn without_uv_a_python_action_is_refused_instead_of_run_by_some_other_python() {
        let err = check_uv(Path::new("scripts/seed.py"), false).unwrap_err();
        assert!(err.contains("scripts/seed.py"), "{err}");
        assert!(err.contains("`uv`"), "{err}");
        assert!(err.contains("not on PATH"), "{err}");
        assert!(err.contains("python3"), "{err}");
        assert!(check_uv(Path::new("scripts/seed.py"), true).is_ok());
    }

    #[test]
    fn a_python_action_needs_no_executable_bit_and_a_shell_script_does() {
        let dir = std::env::temp_dir().join(format!("satz-actions-runnable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("seed.py");
        std::fs::write(&script, "print('hi')\n").unwrap();
        // 0644, as a `get-presets` download arrives: uv reads the file, so the mode bit
        // decides nothing — uv on PATH is the whole condition, both ways.
        runnable(&script, true).expect("a .py action with uv on PATH runs without its executable bit");
        let err = runnable(&script, false).unwrap_err();
        assert!(err.contains("`uv`") && err.contains("seed.py"), "{err}");

        // a shell script is spawned as a program: without the bit it is refused, and
        // uv changes nothing about that
        let sh = dir.join("seed.sh");
        std::fs::write(&sh, "#!/bin/sh\n").unwrap();
        for uv in [true, false] {
            let err = runnable(&sh, uv).unwrap_err();
            assert!(err.contains("seed.sh"), "uv on PATH: {uv}: {err}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
            runnable(&sh, false).expect("an executable shell script runs, uv or not");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn on_windows_a_script_is_refused_and_an_executable_is_not() {
        let err = windows_runnable(Path::new("presets/scc/scc-enable-all.sh")).unwrap_err();
        assert!(err.contains("not a Windows executable") && err.contains("bash presets/scc/scc-enable-all.sh"), "{err}");
        assert!(windows_runnable(Path::new("tools/enable.EXE")).is_ok());
        assert!(windows_runnable(Path::new("tools/enable.cmd")).is_ok());
    }

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
