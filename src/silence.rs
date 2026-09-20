//! "Seen, move on" — said about a finding's IDENTITY, by one of three people.
//!
//! A finding is named by its `kind` and its `subject` (`src/findings.rs`), never by its
//! wording: a message carries paths, line numbers and counts, so a key made of it would
//! be a different key after the next release, and a notice group's header (`10 notice(s)
//! open`) rewords itself when one of the ten is acknowledged.
//!
//! Three tiers name that pair, because the decision belongs to three different people:
//!
//! 1. **the estate** — `[[silence]]` rows in its `config.toml`, each with a mandatory
//!    `reason`, committed and reviewed with the estate, and the only tier that may name
//!    a subject;
//! 2. **the machine** — `[[silence]]` rows in `~/.config/satz/satz.toml`, whole kinds
//!    only, so a silence one operator took for one customer cannot hide a specific
//!    thing at another;
//! 3. **the run** — `--silence <kind>[:<subject>]` and `SATZ_SILENCE`, for a CI
//!    pipeline. It is the whole of what `--no-action-warnings` used to be: one
//!    mechanism, not two.
//!
//! What silencing may never do: an `Error` is never silenced by any tier, so `refusal`
//! never consults this field and cannot be bypassed. A run that asks to silence an
//! error is refused by name — whether a CI switch may DOWNGRADE a named error kind is
//! an open decision, and satz does not answer it by accident.
//!
//! And silencing hides, it never drops: a silenced finding is still produced, still
//! counted, still in `--format json` and in what MCP returns, marked with the tier and
//! the reason. Only the human rendering leaves it out, and it prints one line saying
//! how many it left out and from where.

use std::path::Path;
use std::sync::OnceLock;

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::findings::{Finding, Kind, Severity};
use crate::ToolConfig;

/// Which of the three said so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tier {
    /// the estate's `config.toml`, committed with the estate
    Estate,
    /// this operator's `~/.config/satz/satz.toml`
    Machine,
    /// this run: `--silence`, or `SATZ_SILENCE`
    Run,
}

impl Tier {
    pub(crate) const ALL: &'static [Tier] = &[Tier::Estate, Tier::Machine, Tier::Run];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Tier::Estate => "estate",
            Tier::Machine => "machine",
            Tier::Run => "run",
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the human rendering left out, on the finding itself.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub(crate) struct Silenced {
    /// which tier said so
    pub tier: Tier,
    /// why, as that tier wrote it
    pub reason: String,
}

/// One silence, as a `[[silence]]` table is written.
///
/// `reason` has no default: a row without one is a TOML error naming the file and the
/// table, the way a `deviates` without a reason is a parse error. Unknown keys are
/// refused for the same reason — a misspelt `subjekt` would otherwise silence a whole
/// kind without saying so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    /// the finding kind, as `--format json` spells it: `notice`, `action`, …
    pub kind: Kind,
    /// the one thing of that kind, when only one is meant
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// why it is silenced — a review reads this
    pub reason: String,
}

impl Rule {
    /// `kind` or `kind:subject`, as `--silence` takes it and `satz silence` prints it.
    pub(crate) fn selector(&self) -> String {
        match &self.subject {
            Some(s) => format!("{}:{}", self.kind, s),
            None => self.kind.as_str().to_string(),
        }
    }

    /// Whether this rule names that finding: the kind, and the subject when the rule
    /// names one. Subjects compare whole — there is no pattern, so a silence cannot
    /// grow to cover a finding nobody read.
    pub(crate) fn names(&self, f: &Finding) -> bool {
        self.kind == f.kind && match &self.subject {
            None => true,
            Some(s) => f.subject.as_deref() == Some(s.as_str()),
        }
    }
}

/// Every rule in force for one compile, in tier order: the estate's first, then the
/// machine's, then the run's. The first rule that names a finding is the one recorded.
pub(crate) struct Silences(Vec<(Tier, Rule)>);

impl Silences {
    /// Mark every finding a rule names. An `Error` is never marked: this is the one
    /// place a finding becomes silenced, so the rule holds everywhere at once.
    pub(crate) fn apply(&self, findings: &mut [Finding]) {
        for f in findings.iter_mut().filter(|f| f.severity != Severity::Error) {
            if let Some((tier, rule)) = self.0.iter().find(|(_, r)| r.names(f)) {
                f.silenced = Some(Silenced { tier: *tier, reason: rule.reason.clone() });
            }
        }
    }

    /// The rules that name an ERROR among these findings, with what the error says.
    pub(crate) fn naming_errors<'a>(&'a self, findings: &'a [Finding]) -> Vec<(Tier, &'a Rule, &'a Finding)> {
        self.0
            .iter()
            .filter_map(|(t, r)| findings.iter().find(|f| f.severity == Severity::Error && r.names(f)).map(|f| (*t, r, f)))
            .collect()
    }
}

/// The rules the estate's `config.toml` holds.
pub(crate) fn estate_rules(tool: &ToolConfig) -> &[Rule] {
    &tool.silence
}

/// This machine's rules, set once from `~/.config/satz/satz.toml`, and this run's, set
/// once from `--silence` and `SATZ_SILENCE`.
///
/// Process-wide because that is what they are: one settings file and one command line
/// per process, read before any estate is opened. Neither is ever written per call —
/// `satz mcp` compiles for many calls at once, and a list one call changed would
/// silence another call's findings. The run tier is refused for `mcp` and `lsp`
/// outright, so a server never carries one.
static MACHINE: OnceLock<Vec<Rule>> = OnceLock::new();
static RUN: OnceLock<Vec<Rule>> = OnceLock::new();

pub(crate) fn set_machine(rules: Vec<Rule>) {
    let _ = MACHINE.set(rules);
}

pub(crate) fn set_run(rules: Vec<Rule>) {
    let _ = RUN.set(rules);
}

fn machine_rules() -> &'static [Rule] {
    MACHINE.get().map(Vec::as_slice).unwrap_or_default()
}

fn run_rules() -> &'static [Rule] {
    RUN.get().map(Vec::as_slice).unwrap_or_default()
}

/// The three tiers, for one compile of this estate.
pub(crate) fn in_force(tool: &ToolConfig) -> Silences {
    let mut rules: Vec<(Tier, Rule)> = Vec::new();
    rules.extend(estate_rules(tool).iter().map(|r| (Tier::Estate, r.clone())));
    rules.extend(machine_rules().iter().map(|r| (Tier::Machine, r.clone())));
    rules.extend(run_rules().iter().map(|r| (Tier::Run, r.clone())));
    Silences(rules)
}

/// What a run asked to silence that is an error: the run is refused rather than
/// carrying on with a switch that does not do what it says.
pub(crate) fn refuse_run_silence_of_an_error(silences: &Silences, findings: &[Finding]) -> Result<(), String> {
    let named: Vec<String> = silences
        .naming_errors(findings)
        .into_iter()
        .filter(|(t, _, _)| *t == Tier::Run)
        .map(|(_, r, f)| format!("--silence {} names an error: {}", r.selector(), f.message.lines().next().unwrap_or_default()))
        .collect();
    if named.is_empty() {
        return Ok(());
    }
    // One line: a returned error reaches the terminal through the Debug formatter,
    // which turns a newline into a literal \n.
    Err(format!(
        "{} — an error is never silenced. Whether a run may downgrade a named error kind to a warning \
         is an open decision; until it is answered, narrow what the run does (`transpile --check` \
         rather than `bootstrap`) instead of silencing the error.",
        named.join("; ")
    ))
}

// ---------------------------------------------------------------------------
// selectors
// ---------------------------------------------------------------------------

/// `<kind>` or `<kind>:<subject>`. The first colon separates them, so a subject that is
/// itself a `file:line` reads whole.
pub(crate) fn selector(s: &str) -> Result<(Kind, Option<String>), String> {
    let (name, subject) = match s.split_once(':') {
        Some((_, sub)) if sub.trim().is_empty() => {
            return Err(format!("`{}`: nothing after the `:` — write `<kind>` or `<kind>:<subject>`", s))
        }
        Some((k, sub)) => (k, Some(sub.to_string())),
        None => (s, None),
    };
    let kind = Kind::parse(name.trim())
        .ok_or_else(|| format!("`{}` is not a finding kind. The kinds are: {}", name.trim(), Kind::names()))?;
    Ok((kind, subject))
}

/// The run tier: every `--silence` of this run, then `SATZ_SILENCE` (comma-separated,
/// for a pipeline that cannot change the command line).
pub(crate) fn run_tier(flags: &[String], env: Option<&str>) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for s in flags {
        let (kind, subject) = selector(s)?;
        rules.push(Rule { kind, subject, reason: "--silence on the command line".into() });
    }
    for s in env.unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (kind, subject) = selector(s).map_err(|e| format!("SATZ_SILENCE: {}", e))?;
        rules.push(Rule { kind, subject, reason: "SATZ_SILENCE".into() });
    }
    Ok(rules)
}

// ---------------------------------------------------------------------------
// `satz silence`
// ---------------------------------------------------------------------------

/// Where the machine tier lives, said in an error.
fn machine_file() -> String {
    crate::global_settings_path().map(|p| p.display().to_string()).unwrap_or_else(|| "~/.config/satz/satz.toml".into())
}

/// `satz silence list`: both files, every rule, its reason, and — when an estate is
/// named — whether anything it compiles still answers to it.
pub(crate) fn list(estate: Option<&Path>, tool: &ToolConfig, runtime: &ToolConfig, config: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let found: Option<Vec<Finding>> = match estate {
        Some(e) => Some(findings_of(e, tool, runtime)?),
        None => None,
    };
    let mut any = false;
    for (tier, file, rules) in [
        (Tier::Estate, config.display().to_string(), estate_rules(tool).to_vec()),
        (Tier::Machine, machine_file(), machine_rules().to_vec()),
        (Tier::Run, "this run".to_string(), run_rules().to_vec()),
    ] {
        if rules.is_empty() {
            continue;
        }
        any = true;
        println!("\n{} ({}):", tier, file);
        for r in &rules {
            println!("  {}\n    reason: {}", r.selector(), r.reason);
            if let Some(f) = &found {
                println!("    {}", state(r, f));
            }
        }
    }
    if !config.exists() {
        println!("\nno estate: {} does not exist, so only this machine's rows are listed", config.display());
    }
    if !any {
        println!("\nnothing is silenced — `satz silence add <kind>[:<subject>] --reason \"…\"` adds a row");
    } else if found.is_none() {
        println!("\nName an estate (`satz silence list <estate>.satz`) to see which rows still match.");
    }
    Ok(())
}

/// What one rule does to one compile's findings: how many it silences, that it names an
/// error and therefore silences nothing, or that nothing answers to it any more.
fn state(rule: &Rule, findings: &[Finding]) -> String {
    let errors = findings.iter().filter(|f| f.severity == Severity::Error && rule.names(f)).count();
    let n = findings.iter().filter(|f| f.severity != Severity::Error && rule.names(f)).count();
    match (n, errors) {
        (0, 0) => "STALE: nothing this estate finds answers to it — remove it with `satz silence remove`".to_string(),
        (0, e) => format!("silences nothing: it names {} error(s), and an error is never silenced", e),
        (n, 0) => format!("silences {} finding(s)", n),
        (n, e) => format!("silences {} finding(s); {} error(s) it names stay, an error is never silenced", n, e),
    }
}

/// Everything that estate finds, silenced or not — a refused compile included, whose
/// errors are what a stale rule has to be judged against too.
fn findings_of(estate: &Path, tool: &ToolConfig, runtime: &ToolConfig) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
    match crate::pipeline_b_compile(estate, tool, runtime, crate::PrerequisiteFindings::Report, crate::FindingsOutput::Silent) {
        Ok(out) => Ok(out.findings),
        Err(e) => {
            let f = crate::findings::refusal_findings(e.as_ref());
            if f.is_empty() {
                return Err(e);
            }
            Ok(f)
        }
    }
}

/// `satz silence add`.
pub(crate) fn add(selector_text: &str, reason: &str, machine: bool, config: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let (kind, subject) = selector(selector_text)?;
    if reason.trim().is_empty() {
        return Err("--reason is what a review reads; give one".into());
    }
    let rule = Rule { kind, subject, reason: reason.trim().to_string() };
    if machine {
        if rule.subject.is_some() {
            return Err(format!(
                "{}: this machine silences whole kinds only — `{}` names one subject, which belongs in that \
                 estate's config.toml (`satz silence add {}` without --machine)",
                machine_file(),
                rule.selector(),
                rule.selector()
            )
            .into());
        }
        let mut settings = crate::load_global_settings()?;
        if settings.silence.iter().any(|r| r.kind == rule.kind && r.subject == rule.subject) {
            return Err(format!("{}: `{}` is already silenced there", machine_file(), rule.selector()).into());
        }
        settings.silence.push(rule.clone());
        crate::save_global_settings(&settings)?;
        println!("silenced on this machine: {} — {}\n  {}", rule.selector(), rule.reason, machine_file());
        return Ok(());
    }
    let mut doc = document(config)?;
    let rows = silence_rows(&mut doc);
    if rows.iter().any(|t| row_selector(t) == rule.selector()) {
        return Err(format!("{}: `{}` is already silenced there", config.display(), rule.selector()).into());
    }
    let mut table = toml_edit::Table::new();
    table["kind"] = toml_edit::value(rule.kind.as_str());
    if let Some(s) = &rule.subject {
        table["subject"] = toml_edit::value(s.as_str());
    }
    table["reason"] = toml_edit::value(rule.reason.as_str());
    rows.push(table);
    write_document(config, &doc)?;
    println!("silenced in this estate: {} — {}\n  {}", rule.selector(), rule.reason, config.display());
    Ok(())
}

/// `satz silence remove`.
pub(crate) fn remove(selector_text: &str, machine: bool, config: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let (kind, subject) = selector(selector_text)?;
    let wanted = Rule { kind, subject, reason: String::new() }.selector();
    if machine {
        let mut settings = crate::load_global_settings()?;
        let before = settings.silence.len();
        settings.silence.retain(|r| r.selector() != wanted);
        if settings.silence.len() == before {
            return Err(format!("{}: nothing there silences `{}`", machine_file(), wanted).into());
        }
        crate::save_global_settings(&settings)?;
        println!("no longer silenced on this machine: {}\n  {}", wanted, machine_file());
        return Ok(());
    }
    let mut doc = document(config)?;
    let rows = silence_rows(&mut doc);
    let before = rows.len();
    rows.retain(|t| row_selector(t) != wanted);
    if rows.len() == before {
        return Err(format!("{}: nothing there silences `{}`", config.display(), wanted).into());
    }
    write_document(config, &doc)?;
    println!("no longer silenced in this estate: {}\n  {}", wanted, config.display());
    Ok(())
}

/// The estate's `config.toml`, as text that keeps its comments and its layout: satz
/// adds and removes one table and leaves the rest of the operator's file alone.
fn document(config: &Path) -> Result<toml_edit::DocumentMut, Box<dyn std::error::Error>> {
    if !config.exists() {
        return Err(format!("{}: no config.toml — run `satz silence` from an estate, or pass --config", config.display()).into());
    }
    let text = crate::fsx::read_to_string(config).map_err(|e| format!("{}: {}", config.display(), e))?;
    text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{}: not valid TOML ({})", config.display(), e).into())
}

fn write_document(config: &Path, doc: &toml_edit::DocumentMut) -> Result<(), Box<dyn std::error::Error>> {
    crate::fsx::write(config, doc.to_string())?;
    Ok(())
}

/// The `[[silence]]` array of tables, created empty when the file has none.
fn silence_rows(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::ArrayOfTables {
    if !doc.contains_key("silence") {
        doc["silence"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    doc["silence"].as_array_of_tables_mut().expect("`silence` is the array of tables this writes")
}

fn row_selector(t: &toml_edit::Table) -> String {
    let s = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    match t.get("subject").and_then(|v| v.as_str()) {
        Some(sub) => format!("{}:{}", s("kind"), sub),
        None => s("kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Kind, Severity};

    fn finding(kind: Kind, subject: Option<&str>, severity: Severity) -> Finding {
        let f = Finding::new(severity, kind, "a message with 10 counted things in it");
        match subject {
            Some(s) => f.about(s),
            None => f,
        }
    }

    fn rule(kind: Kind, subject: Option<&str>) -> Rule {
        Rule { kind, subject: subject.map(str::to_string), reason: "seen".into() }
    }

    #[test]
    fn a_selector_splits_at_the_first_colon_so_a_file_line_subject_survives() {
        assert_eq!(selector("notice").unwrap(), (Kind::Notice, None));
        assert_eq!(
            selector("hcl-passthrough:yaml/e.satz:12").unwrap(),
            (Kind::HclPassthrough, Some("yaml/e.satz:12".to_string()))
        );
        assert!(selector("notices").unwrap_err().contains("is not a finding kind"));
        assert!(selector("notice:").unwrap_err().contains("nothing after the `:`"));
    }

    #[test]
    fn a_rule_names_a_whole_kind_or_one_subject() {
        let kindwide = rule(Kind::Notice, None);
        let one = rule(Kind::Notice, Some("cis_baseline_adopted"));
        let mine = finding(Kind::Notice, Some("cis_baseline_adopted"), Severity::Warning);
        let other = finding(Kind::Notice, Some("cis_cmek_required_adopted"), Severity::Warning);
        assert!(kindwide.names(&mine) && kindwide.names(&other));
        assert!(one.names(&mine) && !one.names(&other));
        assert!(!one.names(&finding(Kind::Action, Some("cis_baseline_adopted"), Severity::Warning)));
    }

    #[test]
    fn an_error_is_never_silenced_and_the_run_that_asked_is_refused() {
        let silences = Silences(vec![(Tier::Run, rule(Kind::Emit, None)), (Tier::Estate, rule(Kind::Notice, None))]);
        let mut findings = vec![
            finding(Kind::Emit, None, Severity::Error),
            finding(Kind::Notice, Some("p"), Severity::Warning),
        ];
        silences.apply(&mut findings);
        assert!(findings[0].silenced.is_none(), "an error is never marked");
        assert_eq!(findings[1].silenced.as_ref().unwrap().tier, Tier::Estate);
        let e = refuse_run_silence_of_an_error(&silences, &findings).unwrap_err();
        assert!(e.contains("--silence emit names an error"), "{}", e);
        assert!(e.contains("open decision"), "{}", e);
        assert!(!e.contains('\n'), "a returned error reaches the terminal through Debug: {}", e);
    }

    /// The estate's rows do not refuse a run: the error prints in full and refuses on
    /// its own, and `satz silence list` says the row silences nothing.
    #[test]
    fn a_committed_row_that_names_an_error_refuses_nothing_and_reads_as_silencing_nothing() {
        let silences = Silences(vec![(Tier::Estate, rule(Kind::Emit, None))]);
        let findings = vec![finding(Kind::Emit, None, Severity::Error)];
        assert!(refuse_run_silence_of_an_error(&silences, &findings).is_ok());
        assert!(state(&rule(Kind::Emit, None), &findings).contains("an error is never silenced"));
    }

    #[test]
    fn a_rule_nothing_answers_to_reads_stale() {
        let findings = vec![finding(Kind::Notice, Some("a"), Severity::Warning)];
        assert!(state(&rule(Kind::Notice, Some("gone")), &findings).starts_with("STALE"));
        assert_eq!(state(&rule(Kind::Notice, Some("a")), &findings), "silences 1 finding(s)");
    }

    #[test]
    fn the_run_tier_reads_flags_and_the_environment() {
        let rules = run_tier(&["action".to_string()], Some("notice:p , hcl-passthrough")).unwrap();
        assert_eq!(rules.iter().map(Rule::selector).collect::<Vec<_>>(), ["action", "notice:p", "hcl-passthrough"]);
        assert_eq!(rules[1].reason, "SATZ_SILENCE");
        assert!(run_tier(&[], Some("nope")).unwrap_err().starts_with("SATZ_SILENCE:"));
    }

    /// A row missing its reason is a TOML error, not a row that silences without one.
    #[test]
    fn a_row_without_a_reason_does_not_parse() {
        let e = toml::from_str::<Rule>("kind = \"notice\"").unwrap_err().to_string();
        assert!(e.contains("reason"), "{}", e);
        let e = toml::from_str::<Rule>("kind = \"notice\"\nsubjekt = \"p\"\nreason = \"r\"").unwrap_err().to_string();
        assert!(e.contains("subjekt"), "{}", e);
        let e = toml::from_str::<Rule>("kind = \"notices\"\nreason = \"r\"").unwrap_err().to_string();
        assert!(e.contains("notices"), "{}", e);
    }
}
