//! What a pack asks to be run once it is switched on — its `notice` — and whether the
//! estate has acknowledged it.
//!
//! A notice is open while the estate does not bind its param `true`. Every reader goes
//! through here: the compile's warning, the refusal of `transpile --apply` and
//! `bootstrap`, what `satz interview`, `add-pack` and `merge-presets` print when a switch
//! opens one, and what `satz adopt --execute --import` acknowledges when it has run. The
//! notices are read by the same schema-free walk the questions are, so they are the ones
//! of the files the estate actually uses — a fork's own included — and the report works
//! offline, before `update-schema`.

use std::collections::BTreeSet;
use std::path::Path;

use rmcp::schemars;
use satz_core::pipeline::{acknowledged, estate_questions, Env, PackNotices};

use crate::findings::{Finding, Kind, Severity};
use crate::ToolConfig;

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
    /// `apply`: `transpile --apply` and `bootstrap` refuse while the notice is open
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// the estate binds the param `true`
    pub acknowledged: bool,
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
                before: n.before.clone(),
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
    format!(
        "run `{}`, then bind `{} = true` in the estate's params{}",
        n.run,
        n.param,
        if n.before.as_deref() == Some("apply") { " — apply and bootstrap refuse until then" } else { "" }
    )
}

/// The notices a switch opened, as the CLI prints them: once, when they open.
pub(crate) fn render(rows: &[NoticeRow]) -> String {
    let mut s = String::new();
    for n in rows {
        s.push_str(&format!("\nnotice — {}\n  {}\n  {}\n", n.pack, n.text, then(n)));
    }
    s
}

/// The gate: an estate may not touch an organisation while a `before = apply` notice is
/// open. Names each with what to run.
pub(crate) fn require_acknowledged(input: &Path, runtime: &ToolConfig, action: &str) -> Result<(), String> {
    let held: Vec<NoticeRow> = open(input, runtime)?.into_iter().filter(|n| n.before.as_deref() == Some("apply")).collect();
    if held.is_empty() {
        return Ok(());
    }
    let each: Vec<String> = held.iter().map(|n| format!("{} ({}): {}", n.param, n.pack, then(n))).collect();
    Err(format!("{} refused: {} notice(s) open —\n  {}", action, held.len(), each.join("\n  ")))
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

/// The compile's warning for each open notice, at the estate's `use` line of the pack
/// that declares it — or at the notice itself when another pack uses that one.
pub(crate) fn compile_findings(notices: &[PackNotices], env: &Env, estate: &Path, estate_src: &str) -> Vec<Finding> {
    let open: Vec<NoticeRow> = rows(notices, env).into_iter().filter(|n| !n.acknowledged).collect();
    let header = format!("{} notice(s) open — what a pack asks to be run once it is on:", open.len());
    let scan = crate::packs::scan(estate_src);
    let label = estate.to_string_lossy().into_owned();
    open.iter()
        .map(|n| {
            // the param that acknowledges it is the notice's identity: the same key
            // `satz adopt --import` binds, and what a `[[silence]]` row names
            let f = Finding::new(Severity::Warning, Kind::Notice, format!("`{}`: {} — {}", n.pack, n.text, then(n)))
                .in_group(&header)
                .about(n.param.clone());
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
            before: Some("apply".into()),
            acknowledged,
        }
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
