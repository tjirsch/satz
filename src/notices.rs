//! What a pack asks to be run once it is switched on — its `notice` — and whether the
//! estate has acknowledged it.
//!
//! A notice is open while the estate does not bind its param `true`. Every reader goes
//! through here: the compile's warning, the refusal of a command that writes to the
//! organisation while an open notice is an `error`, what `satz interview`, `add-pack` and
//! `merge-presets` print when a switch opens one, and what `satz adopt --execute
//! --import` acknowledges when it has run. The
//! notices are read by the same schema-free walk the questions are, so they are the ones
//! of the files the estate actually uses — a fork's own included — and the report works
//! offline, before `update-schema`.

use std::collections::BTreeSet;
use std::path::Path;

use rmcp::schemars;
use satz_core::pipeline::{acknowledged, estate_questions, Env, PackNotices};
use satz_core::satz::Severity as Declared;

use crate::findings::{Finding, Kind, Severity};
use crate::ToolConfig;

/// What the run this finding is produced for does to the organisation. The pack's
/// declared severity is the floor and this is the ceiling: a run that only reads says an
/// open `error` as a warning, a run that writes says it as the error it is and refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Doing {
    /// a compile, a report, a plan — nothing reaches the organisation
    Reading,
    /// this run changes the organisation
    Writing,
}

/// One notice of a pack the estate uses.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct NoticeRow {
    /// the param the estate binds `true` to acknowledge it
    pub param: String,
    /// the file that declares it, as the `use` that reached it names it
    pub pack: String,
    /// what to do and why
    pub text: String,
    /// the command to run
    pub run: String,
    /// what the pack declared: `error` — every command that writes to the organisation
    /// refuses while it is open — `warning`, or `info`
    pub severity: String,
    /// the estate binds the param `true`
    pub acknowledged: bool,
}

impl NoticeRow {
    /// The severity the pack declared, as the language spells it.
    pub(crate) fn declared(&self) -> Declared {
        Declared::parse(&self.severity).unwrap_or_default()
    }
}

pub(crate) fn rows(notices: &[PackNotices], env: &Env) -> Vec<NoticeRow> {
    notices
        .iter()
        .flat_map(|p| {
            p.notices.iter().map(|n| NoticeRow {
                param: n.param.clone(),
                pack: p.file.clone(),
                text: n.text.clone(),
                run: n.run.clone(),
                severity: n.severity.to_string(),
                acknowledged: acknowledged(env, &n.param),
            })
        })
        .collect()
}

/// Every notice of the packs the estate uses, acknowledged or not.
pub(crate) fn estate_notices(input: &Path, runtime: &ToolConfig) -> Result<Vec<NoticeRow>, String> {
    let src = crate::fsx::read_to_string(input).map_err(|e| format!("{}: {}", input.display(), e))?;
    let load = crate::questions::loader(input, runtime);
    let (_, notices, env) =
        estate_questions(&input.display().to_string(), &src, &load).map_err(|e| format!("{}:{}: {}", e.file, e.line, e.msg))?;
    Ok(rows(&notices, &env))
}

/// The notices the estate has not acknowledged.
pub(crate) fn open(input: &Path, runtime: &ToolConfig) -> Result<Vec<NoticeRow>, String> {
    Ok(estate_notices(input, runtime)?.into_iter().filter(|n| !n.acknowledged).collect())
}

/// The notices open `after` that were not open `before`: what a switch just opened, which
/// is what is shown once.
pub(crate) fn opened(before: &[NoticeRow], after: &[NoticeRow]) -> Vec<NoticeRow> {
    let was: BTreeSet<&str> = before.iter().map(|n| n.param.as_str()).collect();
    after.iter().filter(|n| !n.acknowledged && !was.contains(n.param.as_str())).cloned().collect()
}

/// What to do about one notice, in one sentence — the same in the compile's warning, the
/// refusal and what a switch prints.
fn then(n: &NoticeRow) -> String {
    let held = held(n.declared());
    format!(
        "run `{}`, then bind `{} = true` in the estate's params{}",
        n.run,
        n.param,
        if held.is_empty() { String::new() } else { format!(" — {}", held) }
    )
}

/// What an open message of this severity holds back. Empty for the two that hold nothing
/// back: they are said, and the run goes on.
fn held(severity: Declared) -> &'static str {
    match severity {
        Declared::Error => "every command that writes to the organisation refuses until then",
        Declared::Warning | Declared::Info => "",
    }
}

/// What an open message of this severity IS for this run: the pack's severity, capped by
/// what the run does. A run that only reads says an `error` as a warning — the command
/// that closes the message compiles the estate too, and a compile that refused would
/// leave no way to close it.
fn severity(declared: Declared, doing: Doing) -> Severity {
    match (declared, doing) {
        (Declared::Error, Doing::Writing) => Severity::Error,
        (Declared::Error, Doing::Reading) | (Declared::Warning, _) => Severity::Warning,
        (Declared::Info, _) => Severity::Info,
    }
}

/// The notices a switch opened, as the CLI prints them: once, when they open.
pub(crate) fn render(rows: &[NoticeRow]) -> String {
    let mut s = String::new();
    for n in rows {
        s.push_str(&format!("\nnotice — {}\n  {}\n  {}\n", n.pack, n.text, then(n)));
    }
    s
}

/// The gate, for a run that writes to the organisation: every pack-declared message of
/// this estate that is open and declared an `error`, as the error findings it is. Empty
/// while none stands, which is what lets the run go on.
///
/// `src/org_write.rs` decides which runs ask; nothing here knows what a command is.
pub(crate) fn open_errors(input: &Path, runtime: &ToolConfig) -> Result<Vec<Finding>, String> {
    let src = crate::fsx::read_to_string(input).map_err(|e| format!("{}: {}", input.display(), e))?;
    let load = crate::questions::loader(input, runtime);
    let (_, notices, env) =
        estate_questions(&input.display().to_string(), &src, &load).map_err(|e| format!("{}:{}: {}", e.file, e.line, e.msg))?;
    Ok(compile_findings(&notices, &env, input, &src, Doing::Writing)
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .collect())
}

/// Acknowledge `params`: bind each `true` in the estate's params.
pub(crate) fn acknowledge(estate: &Path, params: &[String]) -> Result<(), String> {
    if params.is_empty() {
        return Ok(());
    }
    let before = crate::fsx::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let mut src = before.clone();
    for p in params {
        src = crate::interview::bind(&src, p, &serde_yaml::Value::Bool(true))?;
    }
    crate::fsx::write_edited_satz(estate, &before, &src).map_err(|e| format!("{}: {}", estate.display(), e))
}

/// A finding for each open notice, at the estate's `use` line of the pack that declares
/// it — or at the notice itself when another pack uses that one — at the severity the
/// pack declared, capped by what this run does (`Doing`).
pub(crate) fn compile_findings(notices: &[PackNotices], env: &Env, estate: &Path, estate_src: &str, doing: Doing) -> Vec<Finding> {
    let open: Vec<NoticeRow> = rows(notices, env).into_iter().filter(|n| !n.acknowledged).collect();
    let header = "notices open — what a pack asks to be run once it is on";
    let scan = crate::packs::scan(estate_src);
    let label = estate.to_string_lossy().into_owned();
    open.iter()
        .map(|n| {
            // the param that acknowledges it is the notice's identity: the same key
            // `satz adopt --import` binds, and what a `[[silence]]` row names
            // the command is the finding's `fix`; the sentence says what is left to do
            // once it has run
            let held = match held(n.declared()) {
                "" => String::new(),
                clause => format!("; {}", clause),
            };
            let f = Finding::new(
                severity(n.declared(), doing),
                Kind::Notice,
                format!("`{}`: {}\nOnce the command has run, bind `{} = true` in the estate's params{}.", n.pack, n.text, n.param, held),
            )
            // what is this notice's own is its pack and its param, and the first line
            // carries both — the `use` line and the subject. What is left is the pack's
            // text as the pack wrote it: packs that wrote the same text share a table,
            // and a pack that worded it differently stands alone.
            .shared(format!("{}\nOnce the command has run, bind each param named above `true` in the estate's params{}.", n.text, held))
            .in_group(header)
            .about(n.param.clone())
            .fix_in(&n.run, estate);
            match scan.uses.iter().find(|l| !l.commented && l.written == n.pack) {
                Some(l) => f.located(label.clone(), l.index as u32 + 1),
                None => {
                    let line = notices
                        .iter()
                        .filter(|p| p.file == n.pack)
                        .flat_map(|p| &p.notices)
                        .find(|x| x.param == n.param)
                        .map(|x| x.line as u32);
                    f.maybe_at(n.pack.clone(), line)
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(param: &str, acknowledged: bool) -> NoticeRow {
        NoticeRow {
            param: param.into(),
            pack: "presets/p.satz".into(),
            text: "t".into(),
            run: "satz adopt <estate> --execute --import".into(),
            severity: "error".into(),
            acknowledged,
        }
    }

    fn decl(param: &str, severity: Declared) -> Vec<PackNotices> {
        vec![PackNotices {
            pack: "p".into(),
            file: "presets/p.satz".into(),
            notices: vec![satz_core::satz::NoticeDecl {
                param: param.into(),
                text: "Adopt what is live.".into(),
                run: "satz adopt <estate> --execute --import".into(),
                severity,
                line: 7,
            }],
        }]
    }

    /// The compile's finding for an open notice: the pack's command is its `fix`, with the
    /// estate's file name where the pack wrote `<estate>`, and the sentence carries the
    /// text and the acknowledgement, never the command.
    #[test]
    fn an_open_notice_is_a_finding_whose_fix_is_the_packs_command() {
        let notices = decl("p_adopted", Declared::Error);
        let src = "estate e\nuse \"presets/p.satz\"\n";
        let f = compile_findings(&notices, &Env::new(), Path::new("yaml/e.satz"), src, Doing::Reading);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].fix.as_deref(), Some("satz adopt e.satz --execute --import"));
        assert_eq!(
            f[0].message,
            "`presets/p.satz`: Adopt what is live.\nOnce the command has run, bind `p_adopted = true` in the estate's params; every command that writes to the organisation refuses until then."
        );
        // what it shares with every pack that wrote the same text: the pack and the param
        // are out of it, because the first line of a table row says both
        assert_eq!(
            f[0].shared.as_deref(),
            Some("Adopt what is live.\nOnce the command has run, bind each param named above `true` in the estate's params; every command that writes to the organisation refuses until then.")
        );
        assert_eq!((f[0].subject.as_deref(), f[0].file.as_deref(), f[0].line), (Some("p_adopted"), Some("yaml/e.satz"), Some(2)));
        assert_eq!(f[0].group.as_deref(), Some("notices open — what a pack asks to be run once it is on"), "a title, with no count in it");
    }

    /// The pack sets the floor and the run sets the ceiling: an `error` is an error only
    /// where the run writes to the organisation, a `warning` never is, and an `info` waits
    /// for nothing.
    #[test]
    fn what_a_message_is_depends_on_the_pack_and_on_what_the_run_does() {
        let src = "estate e\nuse \"presets/p.satz\"\n";
        let at = |declared, doing| {
            compile_findings(&decl("p_adopted", declared), &Env::new(), Path::new("yaml/e.satz"), src, doing)[0].severity
        };
        assert_eq!(at(Declared::Error, Doing::Writing), Severity::Error, "a run that writes refuses");
        assert_eq!(at(Declared::Error, Doing::Reading), Severity::Warning, "a compile says it and goes on");
        assert_eq!(at(Declared::Warning, Doing::Writing), Severity::Warning, "the apply prints it and goes on");
        assert_eq!(at(Declared::Info, Doing::Writing), Severity::Info, "nothing waits for it");
        // and only an error names what it holds back
        let says = |declared| {
            compile_findings(&decl("p_adopted", declared), &Env::new(), Path::new("yaml/e.satz"), src, Doing::Reading)[0]
                .message
                .clone()
        };
        assert!(says(Declared::Error).contains("refuses until then"));
        assert!(!says(Declared::Warning).contains("refuses"), "{}", says(Declared::Warning));
    }

    #[test]
    fn a_switch_shows_what_it_opened_and_nothing_else() {
        let before = vec![row("a", false)];
        let after = vec![row("a", false), row("b", false), row("c", true)];
        let got: Vec<String> = opened(&before, &after).into_iter().map(|n| n.param).collect();
        assert_eq!(got, vec!["b".to_string()], "`a` was open already and `c` is acknowledged");
        assert!(render(&opened(&before, &after)).contains("bind `b = true`"));
    }
}
