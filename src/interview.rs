//! `satz interview` — a person at a terminal answers what the estate's packs ask.
//!
//! The third way to start an estate. `init` takes every answer as a flag; an agent
//! over MCP asks and writes through `satz_interview`; this asks and writes for a
//! person with neither — one question at a time, the pack's own description when
//! the pack changes, the default in brackets, Enter to accept it. Every answer is
//! written into the estate's `params {}` as it is given, so a run that stops
//! halfway loses nothing: the next run asks what is still open.
//!
//! Line-mode on purpose. It reads stdin, so a piped run drives it end to end in
//! the smoke matrix; a full-screen client can sit on the same report later.
//! satz never calls a model: this presents, the human decides.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::Path;

use crate::questions::{questions_report, short, QuestionRow, QuestionsReport};
use crate::ToolConfig;

/// A value as a Satz literal, as it goes into `params {}`.
pub(crate) fn literal(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::Null => "\"\"".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        serde_yaml::Value::Sequence(items) => {
            format!("[{}]", items.iter().map(literal).collect::<Vec<_>>().join(", "))
        }
        other => format!("\"{}\"", serde_yaml::to_string(other).unwrap_or_default().trim()),
    }
}

/// The estate's top-level `params { … }` block as byte offsets: just after the
/// opening brace, and at the closing one. Brace counting steps over strings and
/// `//` comments, so a `}` inside either does not end the block early.
fn params_block(src: &str) -> Result<(usize, usize), String> {
    let mut at = 0usize;
    let open = loop {
        let rest = &src[at..];
        let Some(rel) = rest.find("params") else {
            return Err("the estate has no `params { }` block — add one; answers are written there".to_string());
        };
        let start = at + rel;
        let line_start = src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let at_line_start = src[line_start..start].trim().is_empty();
        let after = &src[start + "params".len()..];
        let brace = after.trim_start().starts_with('{');
        if at_line_start && brace {
            break start + "params".len() + (after.len() - after.trim_start().len()) + 1;
        }
        at = start + "params".len();
    };
    let bytes = src.as_bytes();
    let mut depth = 1usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((open, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("the estate's `params {` block never closes".to_string())
}

/// Bind `name = value` in the estate's `params {}`: replace an existing binding in
/// place, else append before the closing brace. Text surgery rather than a re-emit,
/// so the comments and the ordering of a hand-edited file survive the interview —
/// the VALUE alone is replaced, so the line keeps its indentation, its `=` column and
/// its trailing comment.
pub(crate) fn bind(src: &str, name: &str, value: &serde_yaml::Value) -> Result<String, String> {
    let (open, close) = params_block(src)?;
    let lit = literal(value);
    let mut out = String::with_capacity(src.len() + 64);
    out.push_str(&src[..open]);
    let mut replaced = false;
    for line in src[open..close].split_inclusive('\n') {
        let trimmed = line.trim_start();
        let binds_it = !replaced
            && trimmed
                .strip_prefix(name)
                .map(|rest| rest.trim_start().starts_with('='))
                .unwrap_or(false);
        if binds_it {
            let (from, to) = value_span(line, name)?;
            out.push_str(&line[..from]);
            out.push_str(&lit);
            out.push_str(&line[to..]);
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    if !replaced {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("  {name} = {lit}\n"));
    }
    out.push_str(&src[close..]);
    Ok(out)
}

/// The value of `name = …` on this line, as a byte range: from the first character
/// after the `=` to the end of the value, with the trailing whitespace and any
/// trailing comment left outside it. A `#` or `//` inside a string is text, not a
/// comment.
///
/// A value that does not finish on its line — an open list or string — is refused
/// rather than half-rewritten: `bind` works a line at a time, and replacing the
/// first line of a multi-line list would leave its tail behind as stray text.
fn value_span(line: &str, name: &str) -> Result<(usize, usize), String> {
    let eq = line[line.find(name).unwrap_or(0)..]
        .find('=')
        .map(|i| line.find(name).unwrap_or(0) + i)
        .ok_or_else(|| format!("{}: the binding has no `=`", name))?;
    let from = line[eq + 1..]
        .find(|c: char| !c.is_whitespace())
        .map(|i| eq + 1 + i)
        .ok_or_else(|| format!("{}: the binding has no value on its line", name))?;

    let bytes = line.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = from;
    let mut end = line.len();
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            match c {
                b'\\' => i += 1,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match c {
                b'"' => in_string = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth -= 1,
                b'#' if depth == 0 => {
                    end = i;
                    break;
                }
                b'/' if depth == 0 && bytes.get(i + 1) == Some(&b'/') => {
                    end = i;
                    break;
                }
                b'\n' => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if in_string || depth != 0 {
        return Err(format!(
            "{}: its value does not finish on one line — write that param by hand, or `satz fmt` it first",
            name
        ));
    }
    let end = line[..end].trim_end().len();
    Ok((from, end))
}

/// Write one answer, given the question it answers. A `oneof` takes an option's
/// param name and sets the siblings false, so the choice stays exclusive by
/// construction. A string with braces is refused: braces interpolate in Satz, and
/// a customer's answer is a value, not a template.
pub(crate) fn answer(src: &str, row: &QuestionRow, value: &serde_yaml::Value) -> Result<String, String> {
    if row.kind == "oneof" {
        let chosen = value
            .as_str()
            .ok_or_else(|| format!("{}: a choice is answered with an option's name", row.subject))?;
        if !row.options.iter().any(|o| o.param == chosen) {
            let names: Vec<&str> = row.options.iter().map(|o| o.param.as_str()).collect();
            return Err(format!("{}: `{}` is not one of its options — {}", row.subject, chosen, names.join(", ")));
        }
        let mut out = src.to_string();
        for o in &row.options {
            let picked = serde_yaml::Value::Bool(o.param == chosen);
            out = bind(&out, &o.param, &picked)?;
            out = uncomment_pack(&out, &o.param, &picked);
        }
        return Ok(out);
    }
    if let Some(s) = value.as_str() {
        if s.contains('{') || s.contains('}') {
            return Err(format!(
                "{}: braces interpolate in a Satz string — if `{}` is what you mean, write that param by hand",
                row.subject, s
            ));
        }
    }
    let out = bind(src, &row.subject, value)?;
    Ok(uncomment_pack(&out, &row.subject, value))
}

/// A pack's `use` line is written commented out, so a day-0 estate applies before any pack
/// exists. Answering its question YES is what puts the pack in the estate — this is where
/// that happens, for the interview and for `satz_interview` alike, since both land here.
///
/// Only ever uncomments. Answering a question `false` leaves the line where it is: `use …
/// when <param>` already emits nothing while the param is false, and silently deleting a
/// pack line from someone's estate is not a thing an answer should do.
pub(crate) fn uncomment_pack(src: &str, gate: &str, value: &serde_yaml::Value) -> String {
    if value.as_bool() != Some(true) {
        return src.to_string();
    }
    let suffix = format!(" when {}", gate);
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        let end = line.trim_end_matches('\n');
        let body = end.trim_start();
        if body.starts_with("// use \"") && end.ends_with(&suffix) {
            out.push_str(&end[..end.len() - body.len()]);
            out.push_str(body.trim_start_matches("// "));
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Read a typed answer in the shape of the value it replaces: a boolean stays a
/// boolean, a number a number, a list a comma-separated list. Anything else is a
/// string — and a param nobody has typed before is a string too.
pub(crate) fn parse_answer(text: &str, like: Option<&serde_yaml::Value>) -> Result<serde_yaml::Value, String> {
    let t = text.trim();
    Ok(match like {
        Some(serde_yaml::Value::Bool(_)) => match t.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" => serde_yaml::Value::Bool(true),
            "false" | "no" | "n" => serde_yaml::Value::Bool(false),
            _ => return Err(format!("`{}`: this one is yes or no", t)),
        },
        Some(serde_yaml::Value::Number(_)) => serde_yaml::from_str::<serde_yaml::Number>(t)
            .map(serde_yaml::Value::Number)
            .map_err(|_| format!("`{}`: this one is a number", t))?,
        Some(serde_yaml::Value::Sequence(_)) => serde_yaml::Value::Sequence(
            t.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| serde_yaml::Value::String(s.to_string()))
                .collect(),
        ),
        _ => serde_yaml::Value::String(t.to_string()),
    })
}

/// Bind a set of answers, each validated against a question the estate asks, and
/// with `accept_defaults` every default the report offers. Returns how many params
/// were written. Nothing is written when any answer is refused.
pub(crate) fn apply(
    estate: &Path,
    runtime: &ToolConfig,
    answers: &BTreeMap<String, serde_yaml::Value>,
    accept_defaults: bool,
) -> Result<usize, String> {
    let report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    let mut src = crate::fsx::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let before = src.clone();
    let mut n = 0;
    for (name, value) in answers {
        let row = report.questions.iter().find(|q| q.subject == *name).ok_or_else(|| {
            format!(
                "{}: no pack this estate uses asks that. An answer names a question's subject — \
                 `satz questions {} --format text --out /dev/stdout` lists them",
                name,
                estate.display()
            )
        })?;
        src = answer(&src, row, value)?;
        n += 1;
    }
    if accept_defaults {
        // Defaults become usable as answers land — a derived name once its input is
        // known — so accepting is judged on the report as it stands after the answers.
        let now = if answers.is_empty() {
            report
        } else {
            crate::fsx::write_edited_satz(estate, &before, &src).map_err(|e| e.to_string())?;
            questions_report(estate, runtime).map_err(|e| e.to_string())?
        };
        for q in now.questions.iter().filter(|q| q.state == "unanswered") {
            if let Some(d) = &q.default {
                src = answer(&src, q, d)?;
                n += 1;
            }
        }
    }
    if n > 0 {
        crate::fsx::write_edited_satz(estate, &before, &src).map_err(|e| e.to_string())?;
    }
    Ok(n)
}

/// The interview itself: ask what is open, write each answer, stop when nothing is
/// open or the input ends. `all` re-asks answered questions too, current answer as
/// the default. `accept_defaults` binds every offered default first and asks only
/// what needs a value; without it the run offers that choice once, up front.
pub(crate) fn run(
    estate: &Path,
    runtime: &ToolConfig,
    all: bool,
    mut accept_defaults: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), String> {
    let w = |out: &mut dyn Write, s: &str| out.write_all(s.as_bytes()).map_err(|e| e.to_string());
    let mut report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    w(out, &format!("\ninterview — {}\n", estate.display()))?;
    if report.questions.is_empty() {
        w(out, "  nothing to ask: no pack this estate uses declares a question.\n")?;
        return Ok(());
    }

    // The offer, once: accept every default now and answer only what needs a value?
    let offerable = report.questions.iter().filter(|q| q.state == "unanswered" && q.default.is_some()).count();
    let typed = report.questions.iter().filter(|q| q.state == "unanswered" && q.blocking).count();
    if !all && !accept_defaults && offerable > 0 {
        w(
            out,
            &format!(
                "\n{} open question(s): {} have a default, {} need a value.\n\
                 Accept all defaults now and answer only those {}? [Y/n] — n goes through every question\n> ",
                offerable + typed,
                offerable,
                typed,
                typed
            ),
        )?;
        out.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return finish(estate, runtime, out);
        }
        accept_defaults = !matches!(line.trim().to_ascii_lowercase().as_str(), "n" | "no");
    }
    if accept_defaults {
        let n = apply(estate, runtime, &BTreeMap::new(), true)?;
        w(out, &format!("  accepted {} default(s).\n", n))?;
        report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    }

    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut last_pack = String::new();
    loop {
        let next = report
            .questions
            .iter()
            .find(|q| !done.contains(&q.subject) && (q.state == "unanswered" || (all && q.state == "answered")));
        let Some(q) = next else { break };
        if q.pack != last_pack {
            w(out, &format!("\n── {} ──", q.pack))?;
            if !q.pack_description.is_empty() {
                w(out, &format!("\n{}", q.pack_description))?;
            }
            w(out, "\n")?;
            last_pack = q.pack.clone();
        }
        w(out, &format!("\n{}", present(q)))?;
        out.flush().map_err(|e| e.to_string())?;

        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            w(out, "\n(end of input)\n")?;
            break;
        }
        let t = line.trim();
        match t {
            "q" | "quit" => break,
            "skip" | "-" => {
                done.insert(q.subject.clone());
                continue;
            }
            "?" => continue,
            _ => {}
        }
        let offered = q.current.as_ref().or(q.default.as_ref());
        let value = if t.is_empty() {
            match offered {
                Some(d) => d.clone(),
                None => {
                    w(out, "  a value is needed here (`skip` leaves it open, `q` stops)\n")?;
                    continue;
                }
            }
        } else if q.kind == "oneof" {
            match choose(q, t) {
                Some(p) => serde_yaml::Value::String(p),
                None => {
                    w(out, &format!("  `{}`: answer with a number from the list\n", t))?;
                    continue;
                }
            }
        } else {
            match parse_answer(t, offered) {
                Ok(v) => v,
                Err(e) => {
                    w(out, &format!("  {}\n", e))?;
                    continue;
                }
            }
        };
        let src = crate::fsx::read_to_string(estate).map_err(|e| e.to_string())?;
        let new_src = match answer(&src, q, &value) {
            Ok(s) => s,
            Err(e) => {
                w(out, &format!("  {}\n", e))?;
                continue;
            }
        };
        crate::fsx::write_edited_satz(estate, &src, &new_src).map_err(|e| e.to_string())?;
        w(out, &format!("  ✓ {} = {}\n", q.subject, literal(&value)))?;
        done.insert(q.subject.clone());
        // Re-read: a derived default may have become usable, a `use … when` may
        // have switched a pack on or off.
        report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    }
    finish(estate, runtime, out)
}

/// One question as the terminal shows it: prompt, why, the cost of changing it
/// later, then the offer — the default in brackets, the options numbered.
fn present(q: &QuestionRow) -> String {
    let mut s = format!("{} — {}\n", q.subject, q.prompt);
    if let Some(why) = &q.why {
        s.push_str(&format!("  {}\n", why));
    }
    let door = if q.reversal == "recreate" || q.blast == "high" { "  ⚠ one-way" } else { "" };
    s.push_str(&format!("  changing it later: {} · blast {}{}\n", q.reversal.replace('_', " "), q.blast, door));
    // What the pack would answer. It is not what Enter accepts — that stays the
    // param default, so a bulk `--accept-defaults` never binds a recommendation
    // nobody read — so it is only worth a line where the two differ.
    if let Some(r) = &q.recommend {
        let r = r.trim_matches('"');
        let offered = q.current.as_ref().or(q.default.as_ref()).map(short);
        if offered.as_deref() != Some(r) {
            s.push_str(&format!("  the pack recommends: {}\n", r));
        }
    }
    if q.kind == "oneof" {
        let picked = q.current.as_ref().or(q.default.as_ref()).and_then(|v| v.as_str().map(str::to_string));
        let mut default_no = None;
        for (i, o) in q.options.iter().enumerate() {
            let is_default = picked.as_deref() == Some(o.param.as_str());
            if is_default {
                default_no = Some(i + 1);
            }
            s.push_str(&format!("  {}) {}{}\n", i + 1, o.label, if is_default { "  (default)" } else { "" }));
            if let Some(why) = &o.why {
                s.push_str(&format!("       {}\n", why));
            }
        }
        match default_no {
            Some(n) => s.push_str(&format!("  [{}] > ", n)),
            None => s.push_str("  > "),
        }
        return s;
    }
    match (q.state, &q.current, &q.default) {
        ("answered", Some(v), _) => s.push_str(&format!("  [{}] > ", short(v))),
        (_, _, Some(d)) => s.push_str(&format!("  [{}] > ", short(d))),
        _ => s.push_str("  no default — a value is needed\n  > "),
    }
    s
}

/// A `oneof` answer: the option's number in the list, or its param name.
fn choose(q: &QuestionRow, text: &str) -> Option<String> {
    if let Ok(n) = text.parse::<usize>() {
        return q.options.get(n.checked_sub(1)?).map(|o| o.param.clone());
    }
    q.options.iter().find(|o| o.param == text).map(|o| o.param.clone())
}

/// Where the estate stands when the interview ends, and what comes next.
fn finish(estate: &Path, runtime: &ToolConfig, out: &mut dyn Write) -> Result<(), String> {
    let report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    let s = &report.summary;
    let mut text = format!(
        "\n{} question(s): {} answered, {} unanswered ({} need a value).\n",
        s.total, s.answered, s.unanswered, s.blocking
    );
    if s.complete {
        text.push_str(&format!(
            "complete — every question is answered.\n  next: satz transpile {} --check, then satz bootstrap {}\n",
            estate.display(),
            estate.display()
        ));
    } else {
        text.push_str(&format!(
            "NOT complete — bootstrap and apply refuse until every question is answered.\n  \
             still open: {}\n  run `satz interview {}` again, or \
             `satz questions {} --format markdown --out decisions.md` for the sheet.\n",
            open_names(&report).join(", "),
            estate.display(),
            estate.display()
        ));
    }
    if let Some(to) = crate::questions::rename_to(estate, &report) {
        text.push_str(&format!(
            "  the estate binds customer_id: `init` would have named this file {} — rename it and its `estate` line.\n",
            to
        ));
    }
    out.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

fn open_names(r: &QuestionsReport) -> Vec<String> {
    r.questions.iter().filter(|q| q.state == "unanswered").map(|q| q.subject.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::Value::String(s.to_string())
    }

    #[test]
    fn bind_appends_into_an_empty_block_and_replaces_in_place() {
        let src = "estate x\n\nparams {\n}\n\nuse \"p.satz\"\n";
        let one = bind(src, "customer_id", &yaml("C0example")).unwrap();
        assert_eq!(one, "estate x\n\nparams {\n  customer_id = \"C0example\"\n}\n\nuse \"p.satz\"\n");
        let two = bind(&one, "customer_id", &yaml("C0other")).unwrap();
        assert_eq!(two, "estate x\n\nparams {\n  customer_id = \"C0other\"\n}\n\nuse \"p.satz\"\n");
        // a longer name that starts the same is not the same binding
        let three = bind(&two, "customer_id_x", &serde_yaml::Value::Bool(true)).unwrap();
        assert!(three.contains("customer_id = \"C0other\"\n  customer_id_x = true\n"), "{}", three);
    }

    #[test]
    fn bind_steps_over_braces_in_strings_and_comments() {
        let src = "params {\n  a = \"}\" // } not the end\n  // } nor this\n}\nother { }\n";
        let out = bind(src, "b", &yaml("v")).unwrap();
        assert_eq!(out, "params {\n  a = \"}\" // } not the end\n  // } nor this\n  b = \"v\"\n}\nother { }\n");
        assert!(bind("estate x\n", "a", &yaml("v")).unwrap_err().contains("no `params { }` block"));
    }

    #[test]
    fn answering_yes_uncomments_that_pack_and_nothing_else() {
        let src = "\
estate e

params {
}

// once the audit archive exists
// use \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel
// use \"presets/integrations/microsoft-sentinel-auditlogs.satz\" when use_sentinel_auditlogs
// use \"presets/organization-budget.satz\" when use_budget

google_folder {
  infra {
    // use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink
  }
}
";
        let yes = serde_yaml::Value::Bool(true);
        let no = serde_yaml::Value::Bool(false);

        // the gate is matched exactly: `use_sentinel` must not drag in `use_sentinel_auditlogs`
        let out = uncomment_pack(src, "use_sentinel", &yes);
        assert!(out.contains("\nuse \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel\n"));
        assert!(
            out.contains("// use \"presets/integrations/microsoft-sentinel-auditlogs.satz\""),
            "a longer param that starts with the same text stays commented:\n{}",
            out
        );

        // answering no changes nothing — a `use … when` already emits nothing, and deleting
        // somebody's pack line is not what an answer does
        assert_eq!(uncomment_pack(src, "use_budget", &no), src);

        // indentation is kept, so a pack inside a block stays inside it
        let out = uncomment_pack(src, "use_audit_logsink", &yes);
        assert!(
            out.contains("    use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink"),
            "the line keeps its four spaces:\n{}",
            out
        );

        // and a value that is not a boolean true leaves the file alone
        assert_eq!(uncomment_pack(src, "use_budget", &serde_yaml::Value::String("yes".into())), src);
    }

    #[test]
    fn literal_escapes_and_types() {
        assert_eq!(literal(&yaml("say \"hi\" \\ back")), r#""say \"hi\" \\ back""#);
        assert_eq!(literal(&serde_yaml::Value::Bool(false)), "false");
        assert_eq!(literal(&serde_yaml::from_str::<serde_yaml::Value>("400").unwrap()), "400");
        assert_eq!(literal(&serde_yaml::from_str::<serde_yaml::Value>("[a, b]").unwrap()), r#"["a", "b"]"#);
    }

    #[test]
    fn parse_answer_follows_the_shape_of_what_it_replaces() {
        let b = serde_yaml::Value::Bool(true);
        assert_eq!(parse_answer("no", Some(&b)).unwrap(), serde_yaml::Value::Bool(false));
        assert!(parse_answer("maybe", Some(&b)).is_err());
        let n = serde_yaml::from_str::<serde_yaml::Value>("30").unwrap();
        assert_eq!(parse_answer("400", Some(&n)).unwrap(), serde_yaml::from_str::<serde_yaml::Value>("400").unwrap());
        let l = serde_yaml::from_str::<serde_yaml::Value>("[]").unwrap();
        assert_eq!(
            parse_answer("in:eu-locations, in:us-locations", Some(&l)).unwrap(),
            serde_yaml::from_str::<serde_yaml::Value>("[\"in:eu-locations\", \"in:us-locations\"]").unwrap()
        );
        assert_eq!(parse_answer("true", None).unwrap(), yaml("true"));
    }

    // ---- a fixture estate with a pack that asks -------------------------------

    fn fixture(name: &str) -> (PathBuf, ToolConfig) {
        let dir = std::env::temp_dir().join(format!("satz-iv-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("asks.satz"),
            r#"// Asks three things and derives a fourth.
//
//   use "asks.satz"
pack asks version "1.0"

params {
  shortname   = ""
  region      = "europe-west3"
  project     = "{shortname}-infra-001"
  zone        = "{region}-a"
  model_a     = true
  model_b     = false
  want_extra  = false
  extra_level = 3
  paid        = false
}

question shortname { prompt = "Short name" why = "Ids derive from it." reversal = recreate blast = high }
question region { prompt = "Region" why = "Regional resources cannot move." reversal = recreate blast = high recommend = "europe-west3" }
question project { prompt = "Project id" why = "A project id is immutable." reversal = recreate blast = high }
question zone { prompt = "Zone" reversal = edit blast = low }
question oneof model {
  prompt = "Which model?" reversal = state_surgery blast = low required = true
  option model_a { label = "A" }
  option model_b { label = "B" }
}
question extra_level { prompt = "Extra level" reversal = edit blast = none ask_when = want_extra }
question paid { prompt = "Switch the paid service on?" why = "It is billed per hour." reversal = edit blast = low recommend = true }
"#,
        )
        .unwrap();
        std::fs::write(dir.join("e.satz"), "estate e\n\nparams {\n}\n\nuse \"asks.satz\"\n").unwrap();
        let mut cfg: ToolConfig = toml::from_str("").unwrap();
        cfg.include_dirs = vec![dir.to_string_lossy().into_owned()];
        (dir.join("e.satz"), cfg)
    }

    #[test]
    fn states_answered_is_bound_by_the_estate_and_defaults_need_their_inputs() {
        let (estate, cfg) = fixture("states");
        let r = questions_report(&estate, &cfg).unwrap();
        let by = |s: &str| r.questions.iter().find(|q| q.subject == s).unwrap().clone();
        assert_eq!(by("shortname").state, "unanswered");
        assert!(by("shortname").blocking, "no default is possible for an empty string");
        assert!(!by("region").blocking);
        assert_eq!(by("region").default, Some(yaml("europe-west3")));
        // derived from an unanswered, undefaultable input: NOT offered — `-infra-001` is not a default
        assert!(by("project").blocking, "{:?}", by("project"));
        // derived from an input whose default is usable: offered, resolved
        assert_eq!(by("zone").default, Some(yaml("europe-west3-a")));
        assert_eq!(by("model").default, Some(yaml("model_a")), "the pack's true option is the offer");
        assert_eq!(by("extra_level").state, "not-applicable");
        assert!(!r.summary.complete);
        assert_eq!((r.summary.total, r.summary.unanswered, r.summary.blocking, r.summary.not_applicable), (6, 6, 2, 1));

        // answer the short name: the derived project id becomes an offer
        apply(&estate, &cfg, &BTreeMap::from([("shortname".to_string(), yaml("acme"))]), false).unwrap();
        let r = questions_report(&estate, &cfg).unwrap();
        let project = r.questions.iter().find(|q| q.subject == "project").unwrap();
        assert_eq!(project.default, Some(yaml("acme-infra-001")));
        assert!(!project.blocking);
        assert_eq!(r.questions.iter().find(|q| q.subject == "shortname").unwrap().state, "answered");

        // accept every default: complete, and the oneof was written as two booleans
        let n = apply(&estate, &cfg, &BTreeMap::new(), true).unwrap();
        assert_eq!(n, 5, "region, project, zone, model, paid — the recommendation is not what a bulk run binds");
        let r = questions_report(&estate, &cfg).unwrap();
        assert!(r.summary.complete, "{:?}", r.summary);
        let src = std::fs::read_to_string(&estate).unwrap();
        assert!(bound(&src, "model_a", "true") && bound(&src, "model_b", "false"), "{}", src);
        assert!(bound(&src, "project", "\"acme-infra-001\""), "{}", src);
        assert!(crate::questions::require_complete(&estate, &cfg, "apply").is_ok());
    }

    fn yaml_num(n: i64) -> serde_yaml::Value {
        serde_yaml::Value::Number(n.into())
    }

    /// A param bound in the file: its line reads `name = value`, at whatever column
    /// the formatter aligned the `=` to.
    fn bound(src: &str, name: &str, value: &str) -> bool {
        src.lines().any(|l| {
            let l = l.trim();
            l.starts_with(name) && l[name.len()..].trim_start().starts_with('=') && l.ends_with(&format!("= {}", value))
        })
    }

    /// An answer replaces the VALUE. What the author wrote around it — the `=` column
    /// of a hand-aligned block, the note at the end of the line — is theirs, and an
    /// interview that eats it is an interview nobody runs twice.
    #[test]
    fn binding_keeps_the_trailing_comment_and_the_alignment() {
        let src = "estate e\n\nparams {\n  audit_retention_days     = 400 # a number\n  customer_shortname       = \"old\"  // typed on day 0\n}\n";
        let out = bind(src, "audit_retention_days", &yaml_num(30)).unwrap();
        assert!(out.contains("  audit_retention_days     = 30 # a number\n"), "{out}");
        let out = bind(&out, "customer_shortname", &yaml("acme")).unwrap();
        assert!(out.contains("  customer_shortname       = \"acme\"  // typed on day 0\n"), "{out}");
    }

    #[test]
    fn a_hash_or_slashes_inside_a_string_are_the_value_not_a_comment() {
        let src = "estate e\n\nparams {\n  logsink_filter = \"log_id(\\\"a#b\\\") // keep\"\n}\n";
        let out = bind(src, "logsink_filter", &yaml("x")).unwrap();
        assert!(out.contains("  logsink_filter = \"x\"\n"), "{out}");
    }

    /// `bind` works a line at a time, so a value that opens a list and closes it three
    /// lines down cannot be replaced by rewriting one line: say so instead of leaving
    /// the tail behind as stray text.
    #[test]
    fn a_value_that_spans_lines_is_refused_not_half_written() {
        let src = "estate e\n\nparams {\n  members = [\n    \"a\",\n  ]\n}\n";
        let e = bind(src, "members", &yaml("x")).unwrap_err();
        assert!(e.contains("does not finish on one line"), "{e}");
    }

    #[test]
    fn an_answer_must_name_a_question_and_a_choice_an_option() {
        let (estate, cfg) = fixture("refuse");
        let e = apply(&estate, &cfg, &BTreeMap::from([("nobody".to_string(), yaml("x"))]), false).unwrap_err();
        assert!(e.contains("no pack this estate uses asks that"), "{}", e);
        let e = apply(&estate, &cfg, &BTreeMap::from([("model".to_string(), yaml("model_c"))]), false).unwrap_err();
        assert!(e.contains("not one of its options"), "{}", e);
        let e = apply(&estate, &cfg, &BTreeMap::from([("shortname".to_string(), yaml("{x}"))]), false).unwrap_err();
        assert!(e.contains("braces interpolate"), "{}", e);
        // nothing was written by the refused calls
        assert_eq!(std::fs::read_to_string(&estate).unwrap(), "estate e\n\nparams {\n}\n\nuse \"asks.satz\"\n");
        let gate = crate::questions::require_complete(&estate, &cfg, "bootstrap").unwrap_err();
        assert!(gate.contains("bootstrap refused") && gate.contains("shortname (needs a value)"), "{}", gate);
    }

    #[test]
    fn a_piped_run_answers_skips_and_stops() {
        let (estate, cfg) = fixture("piped");
        // decline the offer, type the short name, accept region by Enter, skip the
        // project, accept zone, choose model 2, accept the paid question's default,
        // then the input ends
        let mut input = std::io::Cursor::new("n\nacme\n\nskip\n\n2\n\n");
        let mut out = Vec::new();
        run(&estate, &cfg, false, false, &mut input, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("── asks ──"), "{}", text);
        assert!(text.contains("Asks three things and derives a fourth."), "the pack's description opens its section: {}", text);
        assert!(text.contains("✓ shortname = \"acme\""), "{}", text);
        assert!(text.contains("✓ region = \"europe-west3\""), "Enter accepts the default: {}", text);
        // a recommendation is shown where it differs from the offer, and Enter still
        // takes the offer: a pack can recommend a service that costs money without a
        // run binding it by itself
        assert!(text.contains("the pack recommends: true"), "the recommendation is visible: {}", text);
        assert!(!text.contains("recommends: europe-west3"), "a recommendation equal to the offer is noise: {}", text);
        assert!(text.contains("✓ paid = false"), "Enter accepts the default, not the recommendation: {}", text);
        assert!(text.contains("✓ model = \"model_b\""), "{}", text);
        assert!(text.contains("NOT complete") && text.contains("still open: project"), "{}", text);
        let src = std::fs::read_to_string(&estate).unwrap();
        assert!(bound(&src, "model_b", "true") && bound(&src, "model_a", "false"), "{}", src);
        assert!(!src.contains("project ="), "skipped means not written: {}", src);

        // the second run asks only what is open; accepting the offer finishes it
        let mut input = std::io::Cursor::new("y\n");
        let mut out = Vec::new();
        run(&estate, &cfg, false, false, &mut input, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("accepted 1 default(s)") && text.contains("complete — every question is answered"), "{}", text);
    }
}
