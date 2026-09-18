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
    /// a resource declared outside the project or folder its type is scoped to, which
    /// sets none itself
    MissingScope,
    /// an estate's `deployment_mode` that is neither `local` nor `cloud`
    DeploymentMode,
    /// an emitted attribute whose value the provider refuses by its shape
    AttributeShape,
    Prerequisites,
    /// a pack whose gate is true and whose line is commented out or absent
    UnadoptedPack,
    /// an active line of a gated pack without its `when`: a no does not switch it off
    UngatedPack,
    /// a pack on while a pack it needs is off
    PackRequirement,
    /// two packs that exclude one another, both on
    ExcludedPacks,
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

/// The CLI's rendering: warnings and notes to stderr, each group's header once; then
/// the verdict `refusal` reaches — `Err` when there is an error, so `?` refuses the
/// compile.
pub(crate) fn render(findings: &[Finding]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for f in findings.iter().filter(|f| f.severity != Severity::Error) {
        let tag = if f.severity == Severity::Warning { "warning" } else { "note" };
        match f.group.as_deref() {
            Some(g) if !seen.contains(&g) => {
                seen.push(g);
                say(format!("{}: {}\n  {}", tag, g, f.message));
            }
            Some(_) => say(format!("  {}", f.message)),
            None => say(format!("{}: {}", tag, f.message)),
        }
    }
    refusal(findings)
}

/// One entry of the CLI's rendering, to stderr. A test reads back what its own thread
/// printed, which is how it tells a compile that printed its warnings from one that
/// printed nothing.
fn say(text: String) {
    #[cfg(test)]
    SAID.with(|s| s.borrow_mut().push(text.clone()));
    eprintln!("{}", text);
}

#[cfg(test)]
thread_local! {
    static SAID: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What `render` printed on this thread since the last call.
#[cfg(test)]
pub(crate) fn take_said() -> Vec<String> {
    SAID.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// The verdict `render` reaches, with nothing printed: the errors joined into one
/// message under their headers, `Err` when there is any. A command that reports the
/// findings in its own output compiles with this, so an internal compile does not
/// speak over it.
pub(crate) fn refusal(findings: &[Finding]) -> Result<(), String> {
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

    /// A compile that prints nothing refuses with the same text as one that prints: the
    /// only difference between the two is the warnings on stderr.
    #[test]
    fn the_quiet_verdict_is_the_rendered_one_without_the_printing() {
        let f = vec![
            Finding::new(Severity::Warning, Kind::Action, "an action").located("e.satz", 3),
            Finding::new(Severity::Error, Kind::MissingRequired, "a: the provider requires b").in_group("required arguments missing:"),
            Finding::new(Severity::Error, Kind::Emit, "emit: no"),
        ];
        take_said();
        assert_eq!(refusal(&f), render(&f));
        assert_eq!(take_said(), vec!["warning: an action".to_string()], "render printed the warning, once");
        let _ = refusal(&f);
        assert!(take_said().is_empty(), "refusal printed something");
        assert!(refusal(&f[..1]).is_ok(), "a warning alone does not refuse");
    }
}
