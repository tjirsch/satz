//! What the compile finds after the front end — one list, three readers.
//!
//! The CLI prints a warning as it always did and refuses on an error; the language
//! server turns each finding into a diagnostic at the file and line it names; MCP
//! returns the list as data, warnings included. One shape, so the three never
//! disagree about what satz found. The texts are the CLI's, verbatim: what the smoke
//! matrix greps for is what an editor shows.

use rmcp::schemars;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    /// The compile refuses; `transpile` writes nothing.
    Error,
    /// The compile goes on; `tofu plan` or a reviewer will not.
    Warning,
    /// For the log: a fact a reviewer wants to see, no fault.
    Note,
}

/// Which check spoke. The CLI's flags silence two of them (`--no-action-warnings`,
/// the `update-prerequisites` command's own report); an agent can group by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    DryRunConflict,
    Conflict,
    Suppression,
    Emit,
    WrittenReference,
    MissingRequired,
    Prerequisites,
    UnadoptedPack,
    Providers,
    Action,
    HclPassthrough,
    /// `review-pack`: a rule the preset library holds, judged on one pack
    Pack,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub(crate) struct Finding {
    pub severity: Severity,
    pub kind: Kind,
    /// The header of the group this finding belongs to, printed once above the group
    /// by the CLI (`required arguments missing:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The file the finding names: a `use` path as the loader saw it, or the estate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 1-based, in that file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub message: String,
}

impl Finding {
    pub(crate) fn new(severity: Severity, kind: Kind, message: impl Into<String>) -> Self {
        Finding { severity, kind, group: None, file: None, line: None, message: message.into() }
    }
    /// The file this finding is about, with no line — the whole file is the subject.
    pub(crate) fn at_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// The line, once the file is set.
    pub(crate) fn at_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    pub(crate) fn located(mut self, file: impl Into<String>, line: u32) -> Self {
        self.file = Some(file.into());
        self.line = Some(line);
        self
    }
    pub(crate) fn maybe_at(mut self, file: impl Into<String>, line: Option<u32>) -> Self {
        if let Some(l) = line {
            self.file = Some(file.into());
            self.line = Some(l);
        }
        self
    }
    pub(crate) fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
}

/// The severity a validation level gives the findings it governs: `warn` → Warning,
/// `error` → Error, `none` → the check is skipped.
pub(crate) fn at_level(level: &str) -> Option<Severity> {
    match level {
        "none" => None,
        "error" => Some(Severity::Error),
        _ => Some(Severity::Warning),
    }
}

/// The line a param is bound on, 1-based: `name = …` at any indentation and
/// alignment. For findings whose check has no position of its own.
pub(crate) fn param_line(src: &str, name: &str) -> Option<u32> {
    src.lines().position(|l| {
        let t = l.trim_start();
        t.starts_with(name) && t[name.len()..].trim_start().starts_with('=')
    })
    .map(|i| i as u32 + 1)
}

/// A compile the findings refused, as an error: it renders exactly what the CLI
/// prints, and carries the findings that produced it for a caller that can show
/// them at their lines. `pipeline_b_generate` returns this boxed, so every `?`
/// call site is unchanged and only the callers that want structure look for it.
#[derive(Debug)]
pub(crate) struct CompileRefusal {
    pub message: String,
    pub findings: Vec<Finding>,
}

impl std::fmt::Display for CompileRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for CompileRefusal {}

/// The findings behind a refusal, if this error is one. Empty for every other
/// failure — a missing schema directory has no line to point at.
pub(crate) fn refusal_findings(e: &(dyn std::error::Error + 'static)) -> Vec<Finding> {
    e.downcast_ref::<CompileRefusal>().map(|c| c.findings.clone()).unwrap_or_default()
}

/// The CLI's rendering: warnings and notes to stderr as they always were, each group's
/// header once; the errors joined into one message under their headers — `Err` when
/// there is any, so `?` refuses the compile.
/// The same verdict `render` reaches, with nothing printed: what a command that
/// reports the findings in its own output needs, so an internal compile does not
/// speak over it.
pub(crate) fn refusal(findings: &[Finding]) -> Result<(), String> {
    let errors: Vec<&Finding> = findings.iter().filter(|f| f.severity == Severity::Error).collect();
    if errors.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    let mut group: Option<&str> = None;
    for f in errors {
        if let Some(g) = f.group.as_deref() {
            if group != Some(g) {
                msg.push_str(g);
                msg.push('\n');
                group = Some(g);
            }
        }
        msg.push_str(&f.message);
        msg.push('\n');
    }
    Err(msg.trim_end().to_string())
}

pub(crate) fn render(findings: &[Finding]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for f in findings.iter().filter(|f| f.severity != Severity::Error) {
        let tag = if f.severity == Severity::Warning { "warning" } else { "note" };
        match f.group.as_deref() {
            Some(g) if !seen.contains(&g) => {
                seen.push(g);
                eprintln!("{}: {}\n  {}", tag, g, f.message);
            }
            Some(_) => eprintln!("  {}", f.message),
            None => eprintln!("{}: {}", tag, f.message),
        }
    }
    let errors: Vec<&Finding> = findings.iter().filter(|f| f.severity == Severity::Error).collect();
    if errors.is_empty() {
        return Ok(());
    }
    let mut out = String::new();
    let mut groups: Vec<&str> = Vec::new();
    for f in errors {
        match f.group.as_deref() {
            Some(g) => {
                if !groups.contains(&g) {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(g);
                    groups.push(g);
                }
                out.push_str("\n  ");
                out.push_str(&f.message);
            }
            None => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&f.message);
            }
        }
    }
    Err(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_governs_severity() {
        assert_eq!(at_level("warn"), Some(Severity::Warning));
        assert_eq!(at_level("error"), Some(Severity::Error));
        assert_eq!(at_level("none"), None);
    }

    #[test]
    fn a_param_line_is_found_whatever_the_alignment() {
        let src = "estate x\nparams {\n  a         = 1\n  svc_iac_account = \"svc\"\n}\n";
        assert_eq!(param_line(src, "svc_iac_account"), Some(4));
        assert_eq!(param_line(src, "svc"), None, "a prefix is not the name");
    }

    #[test]
    fn a_refusal_displays_as_the_cli_text_and_hands_its_findings_over() {
        let findings = vec![Finding::new(Severity::Error, Kind::Emit, "emit: no").located("e.satz", 7)];
        let e: Box<dyn std::error::Error> =
            Box::new(CompileRefusal { message: "emit: no".into(), findings: findings.clone() });
        assert_eq!(e.to_string(), "emit: no");
        assert_eq!(refusal_findings(e.as_ref()).len(), 1);
        let other: Box<dyn std::error::Error> = "no schemas".into();
        assert!(refusal_findings(other.as_ref()).is_empty(), "only a refusal carries findings");
    }

    #[test]
    fn errors_render_under_their_group_once_and_warnings_do_not_refuse() {
        let f = vec![
            Finding::new(Severity::Warning, Kind::Action, "an action").located("e.satz", 3),
            Finding::new(Severity::Error, Kind::MissingRequired, "a: the provider requires b").in_group("required arguments missing:"),
            Finding::new(Severity::Error, Kind::MissingRequired, "c: the provider requires d").in_group("required arguments missing:"),
            Finding::new(Severity::Error, Kind::Emit, "emit: no"),
        ];
        let e = render(&f).unwrap_err();
        assert_eq!(e, "required arguments missing:\n  a: the provider requires b\n  c: the provider requires d\n\nemit: no");
        assert!(render(&f[..1]).is_ok());
    }
}
