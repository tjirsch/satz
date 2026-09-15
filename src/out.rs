//! One output vocabulary: what a caller may ask for, and where it goes.
//!
//! A reporting command takes two arguments and no more: `--format`, the rendering,
//! and `--out`, the file it lands in. One invocation produces exactly one artefact
//! at exactly one named path, and the console carries nothing but the line saying
//! where it went — on stderr, so `--format json --out /dev/stdout | jq` is a clean
//! pipe. There is no format default, no second destination flag that means "also
//! write", and no rendering nobody asked for.
//!
//! Two commands answer on the console instead, because what they produce is not a
//! document: `update-prerequisites`, which edits the estate and reports what it
//! wrote, and `prowler`, which
//! prints a command line to paste.

use std::path::Path;

type BoxErr = Box<dyn std::error::Error>;

/// What a caller may ask for. A `String` per command let a typo fall through to the
/// default renderer silently — `--format jsom` printed markdown and exited 0. clap
/// rejects an unknown value by name instead, and each command refuses the formats it
/// cannot produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OutFormat {
    /// human-readable terminal output
    #[value(alias = "console")]
    Text,
    Markdown,
    Json,
    /// the markdown typeset by satz itself — no tool on PATH, nothing to install
    Pdf,
    /// a workbook: the catalog a customer fills in and sends back
    Xlsx,
}

impl OutFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            OutFormat::Text => "text",
            OutFormat::Markdown => "markdown",
            OutFormat::Json => "json",
            OutFormat::Pdf => "pdf",
            OutFormat::Xlsx => "xlsx",
        }
    }

    /// Refuse a format this command cannot produce, naming what it can — a silent
    /// fallback to another renderer is how a caller ends up parsing prose.
    pub(crate) fn require_one_of(self, command: &str, allowed: &[OutFormat]) -> Result<Self, String> {
        if allowed.contains(&self) {
            return Ok(self);
        }
        Err(format!(
            "{}: --format {} is not available here; use {}",
            command,
            self.as_str(),
            allowed.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(" or ")
        ))
    }
}

/// Write the one artefact this invocation produces, and say where it went on
/// stderr. The parent directory is created: a report named into a directory that
/// does not exist yet is a path the caller meant, not a mistake.
pub(crate) fn write_report(path: &Path, bytes: &[u8], what: &str) -> Result<(), BoxErr> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            crate::fsx::create_dir_all(dir)?;
        }
    }
    crate::fsx::write(path, bytes)?;
    eprintln!("wrote {} — {}", path.display(), what);
    Ok(())
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

    #[test]
    fn a_format_a_command_cannot_produce_is_refused_by_name() {
        let err = OutFormat::Pdf
            .require_one_of("require", &[OutFormat::Text, OutFormat::Json])
            .unwrap_err();
        assert!(err.contains("--format pdf is not available here"), "{err}");
        assert!(err.contains("text or json"), "{err}");
    }

    #[test]
    fn an_allowed_format_passes_through() {
        assert_eq!(
            OutFormat::Json.require_one_of("triage", &[OutFormat::Markdown, OutFormat::Json]).unwrap(),
            OutFormat::Json
        );
    }

    /// The workbook is a format like any other since it stopped being a flag: a
    /// command that cannot produce it says so by name instead of ignoring it.
    #[test]
    fn xlsx_is_a_format() {
        assert_eq!(OutFormat::Xlsx.as_str(), "xlsx");
        let err = OutFormat::Xlsx
            .require_one_of("report-compliance", &[OutFormat::Markdown, OutFormat::Json, OutFormat::Pdf])
            .unwrap_err();
        assert!(err.contains("--format xlsx is not available here"), "{err}");
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
