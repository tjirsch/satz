//! What the compile finds after the front end — one list, three readers.
//!
//! The CLI prints a warning as it always did and refuses on an error; the language
//! server turns each finding into a diagnostic at the file and line it names; MCP
//! returns the list as data, warnings included. One shape, so the three never
//! disagree about what satz found. The texts are the CLI's, verbatim: what the smoke
//! matrix greps for is what an editor shows.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::silence::Silenced;

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

/// The enum, the name each variant is silenced by, and the list of them all, from one
/// declaration: a kind the list does not know could not be named in a `[[silence]]`
/// row, so the macro fills the list rather than a second table that can fall behind.
macro_rules! kinds {
    ($($(#[$attr:meta])* $variant:ident => $name:literal),+ $(,)?) => {
        /// Which check spoke, and half of a finding's identity: a `[[silence]]` row and
        /// `--silence` name a kind, never a wording. The other half is `subject`. The
        /// `update-prerequisites` command's own report silences its kind for its own
        /// compile; an agent can group by it.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(rename_all = "kebab-case")]
        pub(crate) enum Kind { $($(#[$attr])* $variant),+ }

        impl Kind {
            /// Every kind there is, in declaration order: what `satz silence` offers and
            /// what a selector is read against.
            pub(crate) const ALL: &'static [Kind] = &[$(Kind::$variant),+];

            /// The name a silence names it by — the kebab-case form the JSON carries.
            pub(crate) fn as_str(self) -> &'static str {
                match self { $(Kind::$variant => $name),+ }
            }
        }
    };
}

kinds! {
    DryRunConflict => "dry-run-conflict",
    Conflict => "conflict",
    Suppression => "suppression",
    Emit => "emit",
    WrittenReference => "written-reference",
    MissingRequired => "missing-required",
    /// a resource declared outside the project or folder its type is scoped to, which
    /// sets none itself
    MissingScope => "missing-scope",
    /// an estate's `deployment_mode` that is neither `local` nor `cloud`
    DeploymentMode => "deployment-mode",
    /// an emitted attribute whose value the provider refuses by its shape
    AttributeShape => "attribute-shape",
    Prerequisites => "prerequisites",
    /// a pack whose gate is true and whose line is commented out or absent
    UnadoptedPack => "unadopted-pack",
    /// an active line of a gated pack without its `when`: a no does not switch it off
    UngatedPack => "ungated-pack",
    /// a pack on while a pack it needs is off
    PackRequirement => "pack-requirement",
    /// two packs that exclude one another, both on
    ExcludedPacks => "excluded-packs",
    /// a pack's notice the estate has not acknowledged: the command it names has to run
    Notice => "notice",
    Providers => "providers",
    Action => "action",
    HclPassthrough => "hcl-passthrough",
    /// `review-pack`: a rule the preset library holds, judged on one pack
    Pack => "pack",
}

impl Kind {
    /// The kind of that name, or nothing: a selector naming no kind is refused, never
    /// read as a subject.
    pub(crate) fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.iter().copied().find(|k| k.as_str() == name)
    }

    /// Every name, for an error that has to say what is on offer.
    pub(crate) fn names() -> String {
        Kind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
    /// What this finding is ABOUT, and with `kind` its identity: the pack a pack
    /// finding judges, the param a notice is acknowledged by, the action's name, the
    /// `hcl` block's `file:line`. A `[[silence]]` row names this pair; the message,
    /// which carries counts and paths, names nothing stable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub message: String,
    /// Set when a tier silenced this finding. The finding stays in the list and in the
    /// JSON either way — only the human rendering leaves it out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silenced: Option<Silenced>,
}

impl Finding {
    pub(crate) fn new(severity: Severity, kind: Kind, message: impl Into<String>) -> Self {
        Finding {
            severity,
            kind,
            group: None,
            file: None,
            line: None,
            subject: None,
            message: message.into(),
            silenced: None,
        }
    }

    /// What this finding is about: the second half of its identity.
    pub(crate) fn about(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
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
///
/// A silenced finding is left out here and nowhere else: it is still in the list this
/// was given, still in `--format json` and still in what MCP returns. What it leaves
/// behind is the summary line — how many were silenced, and by which tier — so a
/// silence is visible on every run even when its finding is not.
pub(crate) fn render(findings: &[Finding]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for f in findings.iter().filter(|f| f.severity != Severity::Error && f.silenced.is_none()) {
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
    if let Some(line) = silenced_summary(findings) {
        say(line);
    }
    refusal(findings)
}

/// `3 finding(s) silenced (2 estate, 1 run)` — nothing when none was.
pub(crate) fn silenced_summary(findings: &[Finding]) -> Option<String> {
    let silenced: Vec<&Finding> = findings.iter().filter(|f| f.silenced.is_some()).collect();
    if silenced.is_empty() {
        return None;
    }
    let by_tier: Vec<String> = crate::silence::Tier::ALL
        .iter()
        .filter_map(|t| {
            let n = silenced.iter().filter(|f| f.silenced.as_ref().is_some_and(|s| s.tier == *t)).count();
            (n > 0).then(|| format!("{} {}", n, t))
        })
        .collect();
    Some(format!("{} finding(s) silenced ({}) — `satz silence list` says by what", silenced.len(), by_tier.join(", ")))
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
///
/// It never reads `silenced`, and it never has to: `Silences::apply` skips an `Error`,
/// so no tier can reach the verdict.
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

    /// What a `[[silence]]` row writes is what `--format json` prints: one spelling, so
    /// an operator reads a kind out of a report and pastes it into a rule.
    #[test]
    fn every_kind_is_named_the_same_way_in_the_json_and_in_a_silence() {
        for k in Kind::ALL {
            assert_eq!(serde_json::to_string(k).unwrap(), format!("\"{}\"", k.as_str()));
            assert_eq!(Kind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(Kind::parse("no-such-kind"), None);
        assert!(Kind::names().contains("hcl-passthrough"));
    }

    /// A silence hides, it never drops: the finding is still in the list the caller
    /// holds — and so in `--format json` and in what MCP returns — while the printing
    /// leaves it out and says how many it left out, and by which tier.
    #[test]
    fn a_silenced_finding_is_not_printed_but_is_still_there_and_counted() {
        use crate::silence::{Silenced, Tier};
        let mut f = vec![
            Finding::new(Severity::Warning, Kind::Notice, "notice one").in_group("2 notice(s) open:").about("a"),
            Finding::new(Severity::Warning, Kind::Notice, "notice two").in_group("2 notice(s) open:").about("b"),
            Finding::new(Severity::Warning, Kind::Action, "an action").about("x"),
        ];
        f[0].silenced = Some(Silenced { tier: Tier::Estate, reason: "adopted".into() });
        f[1].silenced = Some(Silenced { tier: Tier::Run, reason: "--silence on the command line".into() });
        take_said();
        render(&f).expect("warnings do not refuse");
        let said = take_said();
        assert_eq!(said.len(), 2, "the group printed nothing and the action printed once: {:?}", said);
        assert_eq!(said[0], "warning: an action");
        assert_eq!(said[1], "2 finding(s) silenced (1 estate, 1 run) — `satz silence list` says by what");
        assert_eq!(f.len(), 3, "nothing was dropped");
        assert!(silenced_summary(&f[2..]).is_none(), "nothing silenced, nothing said");
    }

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
