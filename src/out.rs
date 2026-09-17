//! One output vocabulary: what a caller may ask for, and where it goes.
//!
//! A reporting command takes two arguments and no more: `--format`, the rendering,
//! and `--out`, the file it lands in. One invocation produces exactly one artefact
//! at exactly one named path, and the console carries nothing but the line saying
//! where it went — on stderr, so `--format json --out - | jq` is a clean
//! pipe. There is no format default, no second destination flag that means "also
//! write", and no rendering nobody asked for.
//!
//! Each command declares the formats it writes in one place, the value parser of its
//! `--format` (`formats`): its help lists exactly those, and clap refuses any other
//! naming the same list. A command that writes markdown writes pdf too — the
//! markdown typeset — which a test holds every command to.
//!
//! Two commands answer on the console instead, because what they produce is not a
//! document: `update-prerequisites`, which edits the estate and reports what it
//! wrote, and `prowler`, which
//! prints a command line to paste.

use std::path::{Path, PathBuf};

type BoxErr = Box<dyn std::error::Error>;

/// What a caller may ask for. A `String` per command let a typo fall through to the
/// default renderer silently — `--format jsom` printed markdown and exited 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutFormat {
    Text,
    Markdown,
    Json,
    Pdf,
    Xlsx,
}

impl OutFormat {
    const ALL: [OutFormat; 5] = [OutFormat::Text, OutFormat::Markdown, OutFormat::Json, OutFormat::Pdf, OutFormat::Xlsx];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            OutFormat::Text => "text",
            OutFormat::Markdown => "markdown",
            OutFormat::Json => "json",
            OutFormat::Pdf => "pdf",
            OutFormat::Xlsx => "xlsx",
        }
    }

    /// The extension a file of this format carries, and the ones `--out` may already
    /// end in for it.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            OutFormat::Text => &["txt"],
            OutFormat::Markdown => &["md", "markdown"],
            OutFormat::Json => &["json"],
            OutFormat::Pdf => &["pdf"],
            OutFormat::Xlsx => &["xlsx"],
        }
    }

    fn possible_value(self) -> clap::builder::PossibleValue {
        let value = clap::builder::PossibleValue::new(self.as_str());
        match self {
            OutFormat::Text => value.alias("console").help("human-readable terminal output"),
            OutFormat::Pdf => value.help("the markdown typeset by satz itself — no tool on PATH, nothing to install"),
            OutFormat::Xlsx => value.help("a workbook: the catalog a customer fills in and sends back"),
            OutFormat::Markdown | OutFormat::Json => value,
        }
    }
}

/// The value parser of one command's `--format`: the formats that command writes and
/// no other. The help lists exactly these, and clap refuses anything else naming
/// them — a help line advertising a format the command refuses is how an operator
/// learns the list is wrong, one refusal at a time.
pub(crate) fn formats(allowed: &'static [OutFormat]) -> impl clap::builder::TypedValueParser<Value = OutFormat> {
    use clap::builder::TypedValueParser;
    clap::builder::PossibleValuesParser::new(allowed.iter().map(|f| f.possible_value())).map(|given| {
        // the parser hands back the value as typed, an alias included, and only one it
        // accepted — so exactly one format matches it
        OutFormat::ALL
            .into_iter()
            .find(|f| f.possible_value().matches(&given, false))
            .expect("an accepted value names a format")
    })
}

/// The file an artefact of `format` lands in. `--out` may name it with its extension
/// or without one:
///
/// - a name that ends in the format's extension is used as it is;
/// - a name that ends in ANOTHER format's extension is refused — in
///   `--format pdf --out report.md` one of the two is a mistake, and satz cannot tell
///   which;
/// - any other name gets the format's extension: `--format pdf --out reports/acme`
///   writes `reports/acme.pdf`, and `--out acme.2026-09-16` writes
///   `acme.2026-09-16.pdf`;
/// - a path that exists and is not itself a regular file — `/dev/stdout`, a pipe, a
///   link — is used as it is, since it names a stream or a file chosen elsewhere.
///   The path is judged without following links: `/dev/stdout` is a link, and followed
///   it is whatever stdout is redirected into — a regular file when the caller wrote
///   `> log`, which once turned `--out /dev/stdout` into `/dev/stdout.txt`.
///
/// The line saying where the artefact went names the resulting path.
pub(crate) fn target(out: PathBuf, format: OutFormat) -> Result<PathBuf, String> {
    // `-` is stdout, on every platform
    if out.as_os_str() == "-" {
        return Ok(out);
    }
    // Windows has no `/dev`: the path would be a file named `\dev\stdout.json` on the
    // current drive
    if cfg!(windows) && out.to_string_lossy().starts_with("/dev/") {
        return Err(format!("--out {}: Windows has no /dev — `--out -` writes to stdout", out.display()));
    }
    if std::fs::symlink_metadata(&out).is_ok_and(|m| !m.is_file()) {
        return Ok(out);
    }
    let ext = out.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if let Some(ext) = &ext {
        if format.extensions().contains(&ext.as_str()) {
            return Ok(out);
        }
        if let Some(other) = OutFormat::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str())) {
            return Err(format!(
                "--out {} names a {} file, and --format {} writes {} — name it with .{} or with no extension",
                out.display(),
                other.as_str(),
                format.as_str(),
                format.as_str(),
                format.extensions()[0]
            ));
        }
    }
    let mut name = out.into_os_string();
    name.push(".");
    name.push(format.extensions()[0]);
    Ok(PathBuf::from(name))
}

/// Write the one artefact this invocation produces, and say where it went on
/// stderr. The parent directory is created: a report named into a directory that
/// does not exist yet is a path the caller meant, not a mistake.
pub(crate) fn write_report(path: &Path, bytes: &[u8], what: &str) -> Result<(), BoxErr> {
    if to_stdout(path, bytes)? {
        eprintln!("wrote stdout — {}", what);
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            crate::fsx::create_dir_all(dir)?;
        }
    }
    crate::fsx::write(path, bytes)?;
    eprintln!("wrote {} — {}", path.display(), what);
    Ok(())
}

/// `--out -`: the bytes go to stdout. `Ok(false)` for any other path.
pub(crate) fn to_stdout(path: &Path, bytes: &[u8]) -> Result<bool, BoxErr> {
    if path.as_os_str() != "-" {
        return Ok(false);
    }
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)?;
    out.flush()?;
    Ok(true)
}

/// A PDF from markdown, typeset by satz itself (`src/pdf.rs`). It used to shell out
/// to `pandoc`, which needed a PDF engine behind it in turn, so the format a customer
/// is most likely to be handed was the one that failed on a machine with neither.
pub(crate) fn pdf_from_markdown(markdown: &str, path: &Path, what: &str) -> Result<(), BoxErr> {
    crate::pdf::write(markdown, path, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Every `--format` of every command: its name and the values it takes.
    fn format_args() -> Vec<(String, Vec<String>)> {
        let mut cmd = crate::Cli::command();
        cmd.build();
        cmd.get_subcommands()
            .filter_map(|sub| {
                let arg = sub.get_arguments().find(|a| a.get_id() == "format")?;
                let values = arg.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
                Some((sub.get_name().to_string(), values))
            })
            .collect()
    }

    #[test]
    fn a_command_that_writes_markdown_writes_pdf() {
        let args = format_args();
        assert!(args.len() >= 10, "found only {} commands with --format: {:?}", args.len(), args);
        for (command, values) in &args {
            assert!(!values.is_empty(), "{command}: --format lists no values");
            if values.iter().any(|v| v == "markdown") {
                assert!(values.iter().any(|v| v == "pdf"), "{command} writes markdown but not pdf: {values:?}");
            }
        }
    }

    /// The help and the refusal read one list. `prowler` was the case found: its help
    /// listed all five formats while it wrote two.
    #[test]
    fn the_help_lists_only_what_a_command_writes_and_the_rest_is_refused_by_name() {
        let (_, values) = format_args().into_iter().find(|(c, _)| c == "prowler").expect("prowler takes --format");
        assert_eq!(values, ["text", "json"]);
        let err = crate::Cli::command()
            .try_get_matches_from(["satz", "prowler", "x.satz", "--format", "pdf"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid value 'pdf'"), "{err}");
        assert!(err.contains("text, json"), "{err}");
    }

    #[test]
    fn an_alias_parses_to_its_format() {
        let m = crate::Cli::command()
            .try_get_matches_from(["satz", "require", "cis-gcp-4.0", "x.satz", "--format", "console", "--out", "r"])
            .unwrap();
        let (_, sub) = m.subcommand().unwrap();
        assert_eq!(sub.get_one::<OutFormat>("format"), Some(&OutFormat::Text));
    }

    #[test]
    fn the_extension_may_be_left_off_and_a_contradicting_one_is_refused() {
        let t = |out: &str, f| target(PathBuf::from(out), f);
        assert_eq!(t("reports/acme", OutFormat::Pdf).unwrap(), PathBuf::from("reports/acme.pdf"));
        assert_eq!(t("reports/acme.pdf", OutFormat::Pdf).unwrap(), PathBuf::from("reports/acme.pdf"));
        assert_eq!(t("acme.PDF", OutFormat::Pdf).unwrap(), PathBuf::from("acme.PDF"));
        assert_eq!(t("notes.markdown", OutFormat::Markdown).unwrap(), PathBuf::from("notes.markdown"));
        assert_eq!(t("q", OutFormat::Text).unwrap(), PathBuf::from("q.txt"));
        // a dot inside a name is not an extension satz knows, so the format's is added
        assert_eq!(t("acme.2026-09-16", OutFormat::Xlsx).unwrap(), PathBuf::from("acme.2026-09-16.xlsx"));
        let err = t("report.md", OutFormat::Pdf).unwrap_err();
        assert!(err.contains("names a markdown file") && err.contains("--format pdf"), "{err}");
        assert!(err.contains(".pdf or with no extension"), "{err}");
    }

    #[test]
    fn a_dash_is_stdout() {
        assert_eq!(target(PathBuf::from("-"), OutFormat::Json).unwrap(), PathBuf::from("-"));
        assert_eq!(target(PathBuf::from("-"), OutFormat::Pdf).unwrap(), PathBuf::from("-"));
    }

    #[cfg(unix)]
    #[test]
    fn a_stream_is_written_as_named() {
        assert_eq!(target(PathBuf::from("/dev/stdout"), OutFormat::Json).unwrap(), PathBuf::from("/dev/stdout"));
        assert_eq!(target(PathBuf::from("/dev/null"), OutFormat::Pdf).unwrap(), PathBuf::from("/dev/null"));
    }

    /// `/dev/stdout` redirected into a file is a link to a regular file: the link, not
    /// the file behind it, decides.
    #[cfg(unix)]
    #[test]
    fn a_link_to_a_regular_file_is_written_as_named() {
        let dir = std::env::temp_dir().join(format!("satz-out-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("log");
        std::fs::write(&file, b"").unwrap();
        let link = dir.join("stdout");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(target(link.clone(), OutFormat::Text).unwrap(), link);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_artefact_lands_at_the_named_path_and_the_directory_is_made() {
        let dir = std::env::temp_dir().join(format!("satz-out-{}", std::process::id()));
        let path = dir.join("nested").join("report.md");
        write_report(&path, b"# report\n", "1 row").unwrap();
        assert_eq!(crate::fsx::read_to_string(&path).unwrap(), "# report\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
