//! What the compile finds after the front end — one list, three readers.
//!
//! The CLI lays each finding out for a reader (`lay_out`) and refuses on an error; the
//! language server turns each finding into a diagnostic at the file and line it names;
//! MCP returns the list as data, warnings included. One shape, so the three never
//! disagree about what satz found, and one layout, so every command that prints a
//! finding prints it the same way.

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
    /// what the front end refuses before anything is folded: a parse error, an unknown
    /// param, an entry that does not belong where it stands
    FrontEnd => "front-end",
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
    /// The title of the group this finding belongs to (`required arguments missing`).
    /// The CLI prints it once above the group, with how many findings stand under it.
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
    /// The sentence: what was found, and why it matters. Never the command that answers
    /// it — that is `fix`.
    pub message: String,
    /// The command that answers this finding, as it is typed: `satz add-pack e.satz
    /// presets/estate-map.satz`. Absent where no one command does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// What this finding says in the words every finding like it uses: `message` with
    /// what is this finding's own — a notice's pack and param — left to the first line,
    /// which carries it as `file:line` and `subject`. The PRODUCER states it; the layout
    /// compares it for equality and never derives it from `message`. Findings of one
    /// group whose `shared`, severity, kind and `fix` are equal stand as one table of
    /// first lines over this text. A finding that stands alone prints `message`.
    ///
    /// For the layout alone, and in no schema: every reader of the JSON gets each
    /// finding whole, in `message`, and a second field holding most of the same
    /// sentence would be two homes for one fact.
    #[serde(skip)]
    pub shared: Option<String>,
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
            fix: None,
            shared: None,
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

    /// The command that answers it, as it is typed.
    pub(crate) fn fix(mut self, command: impl Into<String>) -> Self {
        self.fix = Some(command.into());
        self
    }

    /// What it says in the words every finding like it uses — see `shared`. Written for
    /// a table: the rows above it name what each finding is about.
    pub(crate) fn shared(mut self, text: impl Into<String>) -> Self {
        self.shared = Some(text.into());
        self
    }

    /// What the layout compares to tell whether two findings say the same thing: the
    /// text the producer declared shared, else the whole message — one thing found at
    /// several sites is the same message at each.
    fn said(&self) -> &str {
        self.shared.as_deref().unwrap_or(&self.message)
    }

    /// The command that answers it, for one estate: `<estate>` in it — a pack's notice
    /// writes its command that way — is the estate's file name, so the line is one to
    /// paste.
    pub(crate) fn fix_in(self, command: &str, estate: &std::path::Path) -> Self {
        match estate.file_name() {
            Some(name) => self.fix(command.replace("<estate>", &name.to_string_lossy())),
            None => self.fix(command),
        }
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

/// A compile the findings refused, as an error: it displays as the layout the CLI
/// prints, unwrapped, and carries the findings that produced it for a caller that can
/// show them at their lines. `pipeline_b_generate` returns this boxed, so every `?`
/// call site is unchanged and only the callers that want structure look for it.
///
/// A front-end refusal is one too, with one finding: the parser's error has a file, a
/// line and a message, which is what a finding is.
#[derive(Debug)]
pub(crate) struct CompileRefusal {
    pub findings: Vec<Finding>,
}

impl CompileRefusal {
    /// The front end's refusal as the one error finding it is.
    pub(crate) fn front_end(e: satz_core::pipeline::PipelineError) -> Self {
        CompileRefusal { findings: vec![Finding::new(Severity::Error, Kind::FrontEnd, e.msg).located(e.file, e.line as u32)] }
    }

    /// The first error in one line — `file:line: message` — for a caller that reports a
    /// refused compile as one row of its own report.
    pub(crate) fn brief(&self) -> String {
        let Some(f) = self.findings.iter().find(|f| f.severity == Severity::Error) else { return String::new() };
        let first = f.message.lines().next().unwrap_or_default();
        match location(f) {
            Some(at) => format!("{}: {}", at, first),
            None => first.to_string(),
        }
    }
}

impl std::fmt::Display for CompileRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(lay_out(&self.findings, Shown::Errors, Width::Unwrapped).trim_end())
    }
}
impl std::error::Error for CompileRefusal {}

/// The refusal this error is, if it is one.
pub(crate) fn as_refusal<'a>(e: &'a (dyn std::error::Error + 'static)) -> Option<&'a CompileRefusal> {
    e.downcast_ref::<CompileRefusal>()
}

/// The findings behind a refusal, if this error is one. Empty for every other
/// failure — a missing schema directory has no line to point at.
pub(crate) fn refusal_findings(e: &(dyn std::error::Error + 'static)) -> Vec<Finding> {
    as_refusal(e).map(|c| c.findings.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The layout: one text form of a finding, for every command that prints one
// ---------------------------------------------------------------------------

/// How far the prose may run before it breaks.
///
/// A terminal has a width and gets prose wrapped to it. Anything else — a pipe, a CI
/// log, a file, an MCP text block — gets every paragraph on one line: what reads such
/// a stream matches substrings, and a break inside the phrase it looks for is a match
/// it misses. The structure is the same either way: the first line, the indented
/// message, the `fix:` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Width {
    Unwrapped,
    Columns(usize),
}

impl Width {
    /// Wider than this and a line of prose is harder to read than a wrapped one.
    const WIDEST: usize = 110;
    /// Narrower than this and the wrapping is worse than the terminal's own.
    const NARROWEST: usize = 40;

    fn of(columns: Option<(terminal_size::Width, terminal_size::Height)>, is_terminal: bool) -> Width {
        match columns {
            Some((w, _)) if is_terminal => Width::Columns((w.0 as usize).clamp(Self::NARROWEST, Self::WIDEST)),
            _ => Width::Unwrapped,
        }
    }

    pub(crate) fn of_stderr() -> Width {
        use std::io::IsTerminal;
        Width::of(terminal_size::terminal_size_of(std::io::stderr()), std::io::stderr().is_terminal())
    }

    pub(crate) fn of_stdout() -> Width {
        use std::io::IsTerminal;
        Width::of(terminal_size::terminal_size_of(std::io::stdout()), std::io::stdout().is_terminal())
    }
}

/// Which findings of a list one call lays out. A silenced finding is in none of them:
/// it stays in the list and in the JSON, and the footer counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    /// what refuses the compile
    Errors,
    /// what does not: the warnings and the notes
    Rest,
    All,
}

impl Shown {
    fn takes(self, f: &Finding) -> bool {
        match self {
            Shown::Errors => f.severity == Severity::Error,
            Shown::Rest => f.severity != Severity::Error,
            Shown::All => true,
        }
    }
}

impl Severity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// `file:line`, or the file alone when the whole file is the subject.
fn location(f: &Finding) -> Option<String> {
    match (&f.file, f.line) {
        (Some(file), Some(line)) => Some(format!("{}:{}", file, line)),
        (Some(file), None) => Some(file.clone()),
        _ => None,
    }
}

/// The indentation of a finding's message and of its `fix:` line under its first line.
const BODY: &str = "    ";

/// The findings as a human reads them. Per finding: a first line of severity, kind,
/// `file:line` and subject, in columns that line up over the whole run; the message
/// under it, indented, wrapped to `width`; and the command that answers it as a last
/// line of its own, `fix: …`. Findings of one group stand together under the group's
/// title, which says how many stand there and how many of the group a silence left out.
/// Findings of a group that say the same thing — a conflict at each file it involves,
/// nine packs asking for one command — are one block: their first lines as a table, then
/// the sentence and the command once (`blocks`). A pipe gets the same blocks; only the
/// wrapping differs.
/// The findings of no group come first, so that what stands under a title is that
/// group's. Every finding, and every title, is a blank line from the one before it.
/// Empty when nothing is shown.
pub(crate) fn lay_out(findings: &[Finding], shown: Shown, width: Width) -> String {
    let printed: Vec<&Finding> = findings.iter().filter(|f| shown.takes(f) && f.silenced.is_none()).collect();
    // The columns are as wide as their widest entry in this RUN, not in this call: a
    // refused compile prints its warnings and then its errors, and they line up.
    let run = || findings.iter().filter(|f| f.silenced.is_none());
    let kind_w = run().map(|f| f.kind.as_str().len()).max().unwrap_or(0);
    let at_w = run().filter_map(location).map(|l| l.chars().count()).max().unwrap_or(0);
    // The findings of no group first, then each group in the order its first finding
    // arrives: whatever stands under a title belongs to it, down to the next title.
    let mut entries: Vec<(Option<&str>, Vec<&Finding>)> = vec![(None, Vec::new())];
    for f in printed {
        let title = f.group.as_deref();
        match entries.iter_mut().find(|(t, _)| *t == title) {
            Some((_, members)) => members.push(f),
            None => entries.push((title, vec![f])),
        }
    }
    let mut out = String::new();
    for (title, members) in entries {
        if let Some(title) = title {
            if !out.is_empty() {
                out.push('\n');
            }
            let silenced = findings.iter().filter(|f| shown.takes(f) && f.silenced.is_some() && f.group.as_deref() == Some(title)).count();
            let count = match silenced {
                0 => format!("{}", members.len()),
                m => format!("{} of {}, {} silenced", members.len(), members.len() + m, m),
            };
            out.push_str(&format!("{} ({})\n", title, count));
        }
        for block in blocks(members) {
            if !out.is_empty() {
                out.push('\n');
            }
            for f in &block {
                let at = location(f).unwrap_or_default();
                // a subject that IS the location — an `hcl` block is named by where it
                // stands — is said once
                let subject = f.subject.as_deref().filter(|s| *s != at).unwrap_or_default();
                let first = format!("{:<7}  {:<kind_w$}  {:<at_w$}  {}", f.severity.as_str(), f.kind.as_str(), at, subject);
                out.push_str(first.trim_end());
                out.push('\n');
            }
            // alone, a finding says its whole message; in a table the rows have said what
            // is each finding's own
            let text = if block.len() == 1 { block[0].message.as_str() } else { block[0].said() };
            for line in text.lines() {
                wrap_into(&mut out, line, width);
            }
            if let Some(fix) = &block[0].fix {
                // never wrapped: a command is pasted
                out.push_str(&format!("{}fix: {}\n", BODY, fix));
            }
        }
    }
    out
}

/// The findings of one group as the blocks they print as. Findings that say the same
/// thing are one block: their first lines as a table, the sentence once, the command
/// once. Same by EQUALITY — of the severity, the kind, the command and what the finding
/// says (`Finding::said`) — so two sentences one word apart are two blocks, and nothing
/// is cut out of a message to make two alike. A block stands where its first finding
/// arrived. One thing found at several sites — a conflict is one finding per site, so an
/// editor marks each — is the plainest case: the same message at every site.
fn blocks<'a>(members: Vec<&'a Finding>) -> Vec<Vec<&'a Finding>> {
    let mut blocks: Vec<Vec<&Finding>> = Vec::new();
    for f in members {
        let key = |f: &'a Finding| (f.severity, f.kind, f.said(), f.fix.as_deref());
        match blocks.iter_mut().find(|b| key(b[0]) == key(f)) {
            Some(block) => block.push(f),
            None => blocks.push(vec![f]),
        }
    }
    blocks
}

/// One line of a message under its finding: indented, and broken at `width` between
/// words. A line the producer indented — a list under a sentence — keeps its indent,
/// and what wraps hangs two further in.
fn wrap_into(out: &mut String, line: &str, width: Width) {
    let text = line.trim_start();
    let lead = &line[..line.len() - text.len()];
    let Width::Columns(columns) = width else {
        out.push_str(&format!("{}{}{}\n", BODY, lead, text));
        return;
    };
    let hang = if lead.is_empty() { String::new() } else { format!("{}  ", lead) };
    let mut current = format!("{}{}", BODY, lead);
    let mut empty = true;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if !empty && current.chars().count() + 1 + word.chars().count() > columns {
            out.push_str(&current);
            out.push('\n');
            current = format!("{}{}", BODY, hang);
            empty = true;
        }
        if !empty {
            current.push(' ');
        }
        current.push_str(word);
        empty = false;
    }
    out.push_str(&current);
    out.push('\n');
}

/// The last line of a run: what was printed, by severity, and what a silence left out,
/// by tier — `1 error, 10 warnings; 3 silenced (2 estate, 1 run) — …`. Nothing when
/// there is nothing to count.
pub(crate) fn footer(findings: &[Finding]) -> Option<String> {
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s && f.silenced.is_none()).count();
    let plural = |n: usize, word: &str| format!("{} {}{}", n, word, if n == 1 { "" } else { "s" });
    let printed: Vec<String> = [(Severity::Error, "error"), (Severity::Warning, "warning"), (Severity::Note, "note")]
        .into_iter()
        .filter_map(|(s, word)| match count(s) {
            0 => None,
            n => Some(plural(n, word)),
        })
        .collect();
    let silenced: Vec<&Finding> = findings.iter().filter(|f| f.silenced.is_some()).collect();
    let by_tier: Vec<String> = crate::silence::Tier::ALL
        .iter()
        .filter_map(|t| {
            let n = silenced.iter().filter(|f| f.silenced.as_ref().is_some_and(|s| s.tier == *t)).count();
            (n > 0).then(|| format!("{} {}", n, t))
        })
        .collect();
    let left_out =
        (!silenced.is_empty()).then(|| format!("{} silenced ({}) — `satz silence list` says by what", silenced.len(), by_tier.join(", ")));
    match (printed.is_empty(), left_out) {
        (true, None) => None,
        (true, Some(s)) => Some(s),
        (false, None) => Some(printed.join(", ")),
        (false, Some(s)) => Some(format!("{}; {}", printed.join(", "), s)),
    }
}

/// The CLI's rendering: the warnings and notes to stderr in the layout, then the
/// verdict `refusal` reaches — `Err` when there is an error, so `?` refuses the compile.
/// The footer closes a compile that goes on; one that is refused gets it from
/// `report_refusal`, under the errors, so it is the last line either way.
///
/// A silenced finding is left out here and nowhere else: it is still in the list this
/// was given, still in `--format json` and still in what MCP returns. What it leaves
/// behind is its count in the footer — how many were silenced, and by which tier — so a
/// silence is visible on every run even when its finding is not.
pub(crate) fn render(findings: &[Finding]) -> Result<(), String> {
    let text = lay_out(findings, Shown::Rest, Width::of_stderr());
    if !text.is_empty() {
        say(text);
    }
    let verdict = refusal(findings);
    if verdict.is_ok() {
        if let Some(line) = footer(findings) {
            say(line);
        }
    }
    verdict
}

/// A refused compile as the CLI's last words: the errors in the layout, then the footer.
pub(crate) fn report_refusal(refusal: &CompileRefusal) {
    let text = lay_out(&refusal.findings, Shown::Errors, Width::of_stderr());
    eprint!("{}", text);
    if let Some(line) = footer(&refusal.findings) {
        eprintln!("\n{}", line);
    }
}

/// One piece of the CLI's rendering, to stderr, followed by a blank line. A test reads
/// back what its own thread printed, which is how it tells a compile that printed its
/// warnings from one that printed nothing.
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

/// The verdict `render` reaches, with nothing printed: the errors in the layout,
/// unwrapped, `Err` when there is any. A command that reports the findings in its own
/// output compiles with this, so an internal compile does not speak over it.
///
/// It never reads `silenced`, and it never has to: `Silences::apply` skips an `Error`,
/// so no tier can reach the verdict.
pub(crate) fn refusal(findings: &[Finding]) -> Result<(), String> {
    match lay_out(findings, Shown::Errors, Width::Unwrapped) {
        text if text.is_empty() => Ok(()),
        text => Err(text.trim_end().to_string()),
    }
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

    fn notice(subject: &str) -> Finding {
        Finding::new(Severity::Warning, Kind::Notice, format!("the notice of {}", subject))
            .in_group("notices open")
            .about(subject)
            .located("e.satz", 4)
            .fix("satz adopt e.satz --execute --import")
    }

    /// The layout, whole: lone findings first, a group under its title with its count,
    /// the columns lined up over the run, the message indented, the fix last.
    #[test]
    fn a_finding_is_a_first_line_its_message_and_its_fix() {
        let f = vec![
            notice("a"),
            Finding::new(Severity::Note, Kind::HclPassthrough, "raw HCL passthrough (3 lines) — trusted: reviewed")
                .about("p.satz:12")
                .located("p.satz", 12),
            notice("b"),
        ];
        assert_eq!(
            lay_out(&f, Shown::All, Width::Unwrapped),
            "note     hcl-passthrough  p.satz:12\n\
             \x20   raw HCL passthrough (3 lines) — trusted: reviewed\n\
             \n\
             notices open (2)\n\
             \n\
             warning  notice           e.satz:4   a\n\
             \x20   the notice of a\n\
             \x20   fix: satz adopt e.satz --execute --import\n\
             \n\
             warning  notice           e.satz:4   b\n\
             \x20   the notice of b\n\
             \x20   fix: satz adopt e.satz --execute --import\n"
        );
        assert_eq!(footer(&f).as_deref(), Some("2 warnings, 1 note"));
        assert!(lay_out(&f, Shown::Errors, Width::Unwrapped).is_empty(), "there is no error to lay out");
    }

    /// A conflict is one finding per site, so an editor marks every file; the CLI says the
    /// sentence once, under the sites.
    #[test]
    fn the_same_sentence_at_several_sites_is_said_once() {
        let at = |file: &str, line| Finding::new(Severity::Error, Kind::Conflict, "x.y: 2 disagreeing definitions").in_group("composition conflicts").located(file, line);
        let f = vec![at("a.satz", 12), at("b.satz", 40)];
        assert_eq!(
            lay_out(&f, Shown::Errors, Width::Unwrapped),
            "composition conflicts (2)\n\n\
             error    conflict  a.satz:12\n\
             error    conflict  b.satz:40\n\
             \x20   x.y: 2 disagreeing definitions\n"
        );
    }

    /// A pack's notice as its producer builds it: the message names the pack and the
    /// param, and `shared` is what is left when the first line has said both.
    fn pack_notice(pack: &str, line: u32, text: &str) -> Finding {
        let param = format!("{}_adopted", pack);
        Finding::new(Severity::Warning, Kind::Notice, format!("`presets/{}.satz`: {}\nOnce it has run, bind `{} = true`.", pack, text, param))
            .shared(format!("{}\nOnce it has run, bind each param named above `true`.", text))
            .in_group("notices open")
            .about(param)
            .located("e.satz", line)
            .fix("satz adopt e.satz --execute --import")
    }

    /// Findings of one group whose shared sentence and command are EQUAL are one block:
    /// a row each, then the sentence and the command once — into a pipe as on a terminal.
    /// The block stands where its first finding arrived, and a finding that arrives
    /// between two of its rows does not split it.
    #[test]
    fn findings_that_say_the_same_thing_share_one_table() {
        let f = vec![
            pack_notice("a", 4, "Google may hold the policy."),
            pack_notice("baseline", 5, "Google sets the policy."),
            pack_notice("b", 6, "Google may hold the policy."),
            pack_notice("c", 7, "Google may hold the policy."),
        ];
        assert_eq!(
            lay_out(&f, Shown::All, Width::Unwrapped),
            "notices open (4)\n\
             \n\
             warning  notice  e.satz:4  a_adopted\n\
             warning  notice  e.satz:6  b_adopted\n\
             warning  notice  e.satz:7  c_adopted\n\
             \x20   Google may hold the policy.\n\
             \x20   Once it has run, bind each param named above `true`.\n\
             \x20   fix: satz adopt e.satz --execute --import\n\
             \n\
             warning  notice  e.satz:5  baseline_adopted\n\
             \x20   `presets/baseline.satz`: Google sets the policy.\n\
             \x20   Once it has run, bind `baseline_adopted = true`.\n\
             \x20   fix: satz adopt e.satz --execute --import\n"
        );
        assert_eq!(footer(&f).as_deref(), Some("4 warnings"), "a row is a finding, and the count is of findings");
        // the JSON is where it was: every finding whole, and the shared sentence in no field
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json[2]["message"], "`presets/b.satz`: Google may hold the policy.\nOnce it has run, bind `b_adopted = true`.");
        assert!(json[2].get("shared").is_none(), "{}", json[2]);
    }

    /// One row is no table: a finding that shares its sentence with nothing printed
    /// beside it says its whole message, as a finding with no shared sentence does.
    #[test]
    fn a_finding_alone_in_its_block_prints_its_whole_message() {
        let f = vec![pack_notice("a", 4, "Google may hold the policy.")];
        assert_eq!(
            lay_out(&f, Shown::All, Width::Unwrapped),
            "notices open (1)\n\
             \n\
             warning  notice  e.satz:4  a_adopted\n\
             \x20   `presets/a.satz`: Google may hold the policy.\n\
             \x20   Once it has run, bind `a_adopted = true`.\n\
             \x20   fix: satz adopt e.satz --execute --import\n"
        );
    }

    /// Equality is the bar. One word apart is two sentences, a different command is two
    /// answers, another group is another title: each stays a block of its own.
    #[test]
    fn a_sentence_one_word_apart_is_a_block_of_its_own() {
        let f = vec![
            pack_notice("a", 4, "Google may hold the policy."),
            pack_notice("b", 5, "Google may hold a policy."),
            pack_notice("c", 6, "Google may hold the policy.").fix("satz adopt e.satz --execute"),
            pack_notice("d", 7, "Google may hold the policy.").in_group("other notices"),
        ];
        let text = lay_out(&f, Shown::All, Width::Unwrapped);
        assert_eq!(text.matches("    fix: ").count(), 4, "four blocks, each with its command:\n{}", text);
        assert!(!text.contains("named above"), "no two of them share a table:\n{}", text);
        for pack in ["a", "b", "c", "d"] {
            assert!(text.contains(&format!("`presets/{}.satz`: ", pack)), "{} says its whole message:\n{}", pack, text);
        }
    }

    /// A silenced finding is no row: the table holds what is printed, the title counts
    /// it exactly, and a table a silence leaves one row of is a finding on its own.
    #[test]
    fn a_silenced_finding_is_no_row_and_the_title_counts_what_stands_under_it() {
        use crate::silence::{Silenced, Tier};
        let mut f: Vec<Finding> = (0..10u32).map(|i| pack_notice(&format!("p{}", i), 4 + i, "Google may hold the policy.")).collect();
        for i in [1, 4, 8] {
            f[i].silenced = Some(Silenced { tier: Tier::Estate, reason: "adopted".into() });
        }
        let text = lay_out(&f, Shown::All, Width::Unwrapped);
        assert!(text.starts_with("notices open (7 of 10, 3 silenced)\n\n"), "{}", text);
        assert_eq!(text.matches("warning  notice  ").count(), 7, "{}", text);
        assert!(!text.contains("p1_adopted") && !text.contains("p4_adopted") && !text.contains("p8_adopted"), "{}", text);
        assert_eq!(text.matches("    fix: ").count(), 1, "one table, one command:\n{}", text);
        assert_eq!(footer(&f).as_deref(), Some("7 warnings; 3 silenced (3 estate) — `satz silence list` says by what"));
        for x in f.iter_mut().skip(1) {
            x.silenced = Some(Silenced { tier: Tier::Estate, reason: "adopted".into() });
        }
        let text = lay_out(&f, Shown::All, Width::Unwrapped);
        assert!(text.starts_with("notices open (1 of 10, 9 silenced)\n\n"), "{}", text);
        assert!(text.contains("`presets/p0.satz`: Google may hold the policy."), "one row left, so its whole message:\n{}", text);
    }

    /// Prose breaks between words at the width; a line the producer indented keeps its
    /// indent and hangs what wraps; a command is never broken, because it is pasted.
    #[test]
    fn prose_wraps_to_the_width_and_a_fix_never_does() {
        let f = vec![Finding::new(Severity::Warning, Kind::Prerequisites, "one two three four five six seven\n  eight nine ten eleven twelve thirteen")
            .fix("satz update-prerequisites an-estate-with-a-long-name.satz --report-only")];
        assert_eq!(
            lay_out(&f, Shown::All, Width::Columns(24)),
            "warning  prerequisites\n\
             \x20   one two three four\n\
             \x20   five six seven\n\
             \x20     eight nine ten\n\
             \x20       eleven twelve\n\
             \x20       thirteen\n\
             \x20   fix: satz update-prerequisites an-estate-with-a-long-name.satz --report-only\n"
        );
        // no terminal, no width: a pipe gets each paragraph whole, so a substring matches
        assert!(lay_out(&f, Shown::All, Width::Unwrapped).contains("one two three four five six seven\n"));
    }

    /// A silence hides, it never drops: the finding is still in the list the caller
    /// holds — and so in `--format json` and in what MCP returns — while the printing
    /// leaves it out, the group's title counts what is left under it, and the footer
    /// says how many were left out, and by which tier.
    #[test]
    fn a_silenced_finding_is_not_printed_but_is_still_there_and_counted() {
        use crate::silence::{Silenced, Tier};
        let mut f = vec![notice("a"), notice("b"), notice("c"), Finding::new(Severity::Warning, Kind::Action, "an action").about("x")];
        f[0].silenced = Some(Silenced { tier: Tier::Estate, reason: "adopted".into() });
        f[1].silenced = Some(Silenced { tier: Tier::Run, reason: "--silence on the command line".into() });
        take_said();
        render(&f).expect("warnings do not refuse");
        let said = take_said();
        assert_eq!(said.len(), 2, "the layout, then the footer: {:?}", said);
        assert!(said[0].contains("notices open (1 of 3, 2 silenced)\n"), "the title counts what stands under it: {}", said[0]);
        assert!(said[0].contains("  c\n") && !said[0].contains("the notice of a"), "{}", said[0]);
        assert_eq!(said[1], "2 warnings; 2 silenced (1 estate, 1 run) — `satz silence list` says by what");
        assert_eq!(f.len(), 4, "nothing was dropped");
        assert_eq!(footer(&f[3..]).as_deref(), Some("1 warning"), "nothing silenced, nothing said about it");
        // a group silenced whole leaves no title behind
        f[2].silenced = Some(Silenced { tier: Tier::Machine, reason: "seen".into() });
        assert!(!lay_out(&f, Shown::All, Width::Unwrapped).contains("notices open"));
        assert!(footer(&[]).is_none());
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
    fn a_refusal_displays_as_the_layout_and_hands_its_findings_over() {
        let findings = vec![
            Finding::new(Severity::Warning, Kind::Action, "an action"),
            Finding::new(Severity::Error, Kind::Emit, "emit: no\nand why").located("e.satz", 7),
        ];
        let e: Box<dyn std::error::Error> = Box::new(CompileRefusal { findings });
        assert_eq!(e.to_string(), "error    emit    e.satz:7\n    emit: no\n    and why", "the errors alone, and no trailing newline");
        assert_eq!(refusal_findings(e.as_ref()).len(), 2);
        assert_eq!(as_refusal(e.as_ref()).unwrap().brief(), "e.satz:7: emit: no");
        let other: Box<dyn std::error::Error> = "no schemas".into();
        assert!(refusal_findings(other.as_ref()).is_empty(), "only a refusal carries findings");
    }

    /// The parser's refusal is a refusal like any other: one error finding, at its line.
    #[test]
    fn a_front_end_error_is_one_finding_at_its_line() {
        let e = satz_core::pipeline::PipelineError { file: "e.satz".into(), line: 8, msg: "unknown param `x`".into() };
        let r = CompileRefusal::front_end(e);
        assert_eq!(r.findings.len(), 1);
        let f = &r.findings[0];
        assert_eq!((f.severity, f.kind, f.file.as_deref(), f.line), (Severity::Error, Kind::FrontEnd, Some("e.satz"), Some(8)));
        assert_eq!(r.to_string(), "error    front-end  e.satz:8\n    unknown param `x`");
    }

    #[test]
    fn errors_render_under_their_group_once_and_warnings_do_not_refuse() {
        let f = vec![
            Finding::new(Severity::Warning, Kind::Action, "an action").located("e.satz", 3),
            Finding::new(Severity::Error, Kind::MissingRequired, "a: the provider requires b").in_group("required arguments missing"),
            Finding::new(Severity::Error, Kind::MissingRequired, "c: the provider requires d").in_group("required arguments missing"),
            Finding::new(Severity::Error, Kind::Emit, "emit: no"),
        ];
        let e = render(&f).unwrap_err();
        assert_eq!(
            e,
            "error    emit\n    emit: no\n\n\
             required arguments missing (2)\n\n\
             error    missing-required\n    a: the provider requires b\n\n\
             error    missing-required\n    c: the provider requires d"
        );
        assert!(render(&f[..1]).is_ok());
    }

    /// A compile that prints nothing refuses with the same text as one that prints: the
    /// only difference between the two is the warnings on stderr.
    #[test]
    fn the_quiet_verdict_is_the_rendered_one_without_the_printing() {
        let f = vec![
            Finding::new(Severity::Warning, Kind::Action, "an action").located("e.satz", 3),
            Finding::new(Severity::Error, Kind::MissingRequired, "a: the provider requires b").in_group("required arguments missing"),
            Finding::new(Severity::Error, Kind::Emit, "emit: no"),
        ];
        take_said();
        assert_eq!(refusal(&f), render(&f));
        assert_eq!(
            take_said(),
            vec!["warning  action            e.satz:3\n    an action\n".to_string()],
            "render printed the warning, once, and left the footer to whoever reports the refusal"
        );
        let _ = refusal(&f);
        assert!(take_said().is_empty(), "refusal printed something");
        assert!(refusal(&f[..1]).is_ok(), "a warning alone does not refuse");
    }
}
