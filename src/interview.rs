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
use crate::settings::ToolConfig;
use satz_core::pack_graph::PackGraph;
use satz_core::satz::{lex_spanned, Tok, Token};

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

/// The estate's `params { … }` blocks and the bindings in them, read with the
/// language's own lexer: a comment in any of its forms, a string, a raw `hcl { … }`
/// body and a second binding on one line are what the lexer says they are, never what
/// a brace count guesses. A brace count that guessed wrong wrote a second `x = …`
/// beside the one already there, and satz refuses that estate on the next read.
#[derive(Default)]
struct Params {
    /// Per top-level block, as byte offsets: just after its `{`, and at its `}`.
    blocks: Vec<(usize, usize)>,
    /// Every binding in those blocks: its name, and the byte range of its VALUE.
    bound: Vec<(String, usize, usize)>,
}

impl Params {
    /// The byte range of the value `name` is bound to, in whichever block binds it —
    /// a second `params { }` block counts, which is how a subject that looked unbound
    /// was bound a second time.
    fn value_of(&self, name: &str) -> Option<(usize, usize)> {
        self.bound.iter().find(|(n, _, _)| n == name).map(|(_, from, to)| (*from, *to))
    }
}

/// The byte offset of every character, plus the end: the lexer spans characters and
/// the splices below cut bytes.
fn byte_offsets(src: &str) -> Vec<usize> {
    src.char_indices().map(|(i, _)| i).chain(std::iter::once(src.len())).collect()
}

/// Every top-level `params { … }` of `src`. A nested one — a body key, a block inside
/// a resource — is not the estate's: only depth 0 counts.
fn params_of(src: &str) -> Result<Params, String> {
    let toks = lex_spanned(src, false).map_err(|e| e.to_string())?;
    let off = byte_offsets(src);
    let mut out = Params::default();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i].tok {
            Tok::Ident(id) if id == "params" && depth == 0 && matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::LBrace)) => {
                i = read_block(&toks, i + 2, off[toks[i + 1].end], &off, &mut out)?;
            }
            Tok::LBrace | Tok::LBrack => {
                depth += 1;
                i += 1;
            }
            Tok::RBrace | Tok::RBrack => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(out)
}

/// One block, from the token after its `{`: every `name = value` in it and the
/// block's own span, and the token index just past its `}`.
fn read_block(toks: &[Token], mut i: usize, open: usize, off: &[usize], out: &mut Params) -> Result<usize, String> {
    loop {
        let Some(t) = toks.get(i) else {
            return Err("the estate's `params {` block never closes".to_string());
        };
        if t.tok == Tok::RBrace {
            out.blocks.push((open, off[t.start]));
            return Ok(i + 1);
        }
        let Tok::Ident(name) = &t.tok else {
            return Err(format!("params: line {}: expected a name or `}}`", t.line));
        };
        if toks.get(i + 1).map(|t| &t.tok) != Some(&Tok::Eq) {
            return Err(format!("params: line {}: `{}` is not followed by `=`", t.line, name));
        }
        let end = end_of_value(toks, i + 2)?;
        out.bound.push((name.clone(), off[toks[i + 2].start], off[toks[end - 1].end]));
        i = end;
    }
}

/// The token index just past the value that starts at `at`: one token, or a list or
/// map to its matching bracket — however many lines it takes.
fn end_of_value(toks: &[Token], at: usize) -> Result<usize, String> {
    let Some(first) = toks.get(at) else {
        return Err("params: a binding without a value".to_string());
    };
    if !matches!(first.tok, Tok::LBrack | Tok::LBrace) {
        return Ok(at + 1);
    }
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate().skip(at) {
        match t.tok {
            Tok::LBrack | Tok::LBrace => depth += 1,
            Tok::RBrack | Tok::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Ok(k + 1);
                }
            }
            _ => {}
        }
    }
    Err(format!("params: line {}: the value opens a list or a map that never closes", first.line))
}

/// The literal `name` is bound to in the estate's params, exactly as it is written
/// there, or `None` when nothing binds it.
pub(crate) fn bound_literal(src: &str, name: &str) -> Result<Option<String>, String> {
    Ok(params_of(src)?.value_of(name).map(|(from, to)| src[from..to].to_string()))
}

/// Bind `name = value` in the estate's `params {}`: replace the binding it already
/// has, in place, and append one only when nothing binds the name. Every writer of a
/// param lands here — `init`, the interview and `satz_interview`, `add-pack` and
/// `remove-pack`, the notice `adopt --execute --import` acknowledges, `migrate`'s
/// deployment mode, the greenfield write-back — so no two of them can leave a subject
/// bound twice, which satz refuses to compile.
///
/// Text surgery rather than a re-emit, so the comments and the ordering of a
/// hand-edited file survive the write — the VALUE alone is replaced, and the line
/// keeps its indentation and its trailing comment. When the binding is APPENDED, the
/// `params` block is re-aligned by the formatter's own rule, because a new
/// `name = value` would otherwise break the `=` column of every binding around it; a
/// replaced value changes no width and touches nothing but itself. Everything outside
/// the block stays as the author laid it out.
pub(crate) fn bind(src: &str, name: &str, value: &serde_yaml::Value) -> Result<String, String> {
    let params = params_of(src)?;
    let lit = literal(value);
    if let Some((from, to)) = params.value_of(name) {
        return Ok(format!("{}{}{}", &src[..from], lit, &src[to..]));
    }
    let (_, close) = *params
        .blocks
        .first()
        .ok_or_else(|| "the estate has no `params { }` block — add one; answers are written there".to_string())?;
    let mut out = String::with_capacity(src.len() + 64);
    out.push_str(&src[..close]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("  {name} = {lit}\n"));
    out.push_str(&src[close..]);
    Ok(align_params(&out))
}

/// The `params` block as `satz fmt` lays it out, spliced back into text whose rest is
/// left alone. `write_edited_satz` re-formats only a file that was canonical before
/// the edit, so a block a person already touched by hand — a line deleted, which
/// leaves the others padded for a name that is gone — kept its broken columns and
/// gained unaligned ones. Text the formatter cannot parse is returned as it came:
/// the writer refuses it next, and says why.
fn align_params(text: &str) -> String {
    let Ok(formatted) = satz_core::fmt::format(text) else { return text.to_string() };
    let first = |s: &str| params_of(s).ok().and_then(|p| p.blocks.first().copied());
    match (first(text), first(&formatted)) {
        (Some((open, close)), Some((fopen, fclose))) => {
            format!("{}{}{}", &text[..open], &formatted[fopen..fclose], &text[close..])
        }
        _ => text.to_string(),
    }
}

/// Write one answer, given the question it answers. A `oneof` takes an option's
/// param name and sets the siblings false, so the choice stays exclusive by
/// construction. A string with braces is refused: braces interpolate in Satz, and
/// a customer's answer is a value, not a template.
///
/// `graph` is the pack graph of the estate's presets: a yes to a pack's gate switches
/// that pack's line on through it. Without one, the answer is bound and no line moves.
pub(crate) fn answer(
    src: &str,
    row: &QuestionRow,
    value: &serde_yaml::Value,
    graph: Option<&PackGraph>,
) -> Result<String, String> {
    if row.kind == "oneof" {
        let chosen = value
            .as_str()
            .ok_or_else(|| format!("{}: a choice is answered with an option's name", row.subject))?;
        let none = chosen == crate::questions::NO_BRANCH && !row.required;
        if !none && !row.options.iter().any(|o| o.param == chosen) {
            let mut names: Vec<&str> = row.options.iter().map(|o| o.param.as_str()).collect();
            if !row.required {
                names.push(crate::questions::NO_BRANCH);
            }
            return Err(format!("{}: `{}` is not one of its options — {}", row.subject, chosen, names.join(", ")));
        }
        let mut out = src.to_string();
        for o in &row.options {
            let picked = serde_yaml::Value::Bool(o.param == chosen);
            out = bind(&out, &o.param, &picked)?;
            out = pack_lines(&out, &o.param, &picked, graph)?;
        }
        return Ok(out);
    }
    check_shape(row, value)?;
    if let Some(s) = value.as_str() {
        if s.contains('{') || s.contains('}') {
            return Err(format!(
                "{}: braces interpolate in a Satz string — if `{}` is what you mean, write that param by hand",
                row.subject, s
            ));
        }
    }
    let out = bind(src, &row.subject, value)?;
    let out = workload_folder(&out, row, value)?;
    pack_lines(&out, &row.subject, value, graph)
}

/// The day-0 scaffold's one answer-shaped section: answering estate-core's
/// `workload_folder_name` writes the section that publishes `workload_folder` — the
/// organisation for "", the folder for a name — as `satz init` does from its flag, so an
/// estate an interview started ends where an init estate does. A section of the other
/// form is refused by [`crate::template::with_workload_folder`], never rewritten.
fn workload_folder(src: &str, row: &QuestionRow, value: &serde_yaml::Value) -> Result<String, String> {
    match value.as_str() {
        Some(name) if row.subject == crate::template::WORKLOAD_FOLDER_NAME && row.pack == "estate_core" => {
            crate::template::with_workload_folder(src, !name.trim().is_empty())
        }
        _ => Ok(src.to_string()),
    }
}

/// A pack's `use` line is written commented out, so a day-0 estate applies before any pack
/// exists. Answering its question YES is what puts the pack in the estate: the line of
/// every pack on that gate is switched on as `satz add-pack` switches it — uncommented, or
/// written where the pack graph places it — for the interview and for `satz_interview`
/// alike, since both land here.
///
/// Only a yes moves a line. Answering `false` leaves the line where it is: `use … when
/// <param>` already emits nothing while the param is false, and deleting a pack line from
/// someone's estate is not a thing an answer should do.
fn pack_lines(src: &str, gate: &str, value: &serde_yaml::Value, graph: Option<&PackGraph>) -> Result<String, String> {
    match (value.as_bool(), graph) {
        (Some(true), Some(g)) => Ok(crate::packs::gate_on(src, g, gate)?.0),
        _ => Ok(src.to_string()),
    }
}

/// Read a typed answer in the shape the pack declares for the param — or, where the
/// report names none, the shape of the value it replaces: a boolean stays a boolean, a
/// number a number, a list a comma-separated list, one entry a one-element list. With no
/// shape at all it is a string. The declared shape comes first because a param declared
/// `[]` offers nothing to replace, and one address typed there was written as a string.
pub(crate) fn parse_answer(text: &str, like: Option<&serde_yaml::Value>, shape: Option<&str>) -> Result<serde_yaml::Value, String> {
    let t = text.trim();
    Ok(match shape.or_else(|| like.and_then(crate::questions::value_shape)) {
        Some("bool") => match t.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" => serde_yaml::Value::Bool(true),
            "false" | "no" | "n" => serde_yaml::Value::Bool(false),
            _ => return Err(format!("`{}`: this one is yes or no", t)),
        },
        Some("number") => serde_yaml::from_str::<serde_yaml::Number>(t)
            .map(serde_yaml::Value::Number)
            .map_err(|_| format!("`{}`: this one is a number", t))?,
        Some("list") => serde_yaml::Value::Sequence(
            t.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| serde_yaml::Value::String(s.to_string()))
                .collect(),
        ),
        Some("map") => return Err(format!("`{}`: this one is a map — write it into the estate's params by hand", t)),
        _ => serde_yaml::Value::String(t.to_string()),
    })
}

/// Refuse an answer whose shape contradicts the one the pack declares: a string where
/// the pack declares a list, `"true"` where it declares a bool. The provider refuses
/// that value at `tofu apply`, naming an attribute in generated HCL; here the question
/// names the param. A number or a bool for a string is a string Terraform converts, and
/// passes.
pub(crate) fn check_shape(row: &QuestionRow, value: &serde_yaml::Value) -> Result<(), String> {
    let Some(declared) = row.shape else { return Ok(()) };
    let given = crate::questions::value_shape(value);
    let fits = match declared {
        "string" => matches!(given, Some("string" | "number" | "bool")),
        other => given == Some(other),
    };
    if fits {
        return Ok(());
    }
    let hint = match declared {
        "list" => " — answer with a list, even of one: [\"security@example.com\"]",
        "bool" => " — answer true or false, unquoted",
        "number" => " — answer with a number, unquoted",
        _ => "",
    };
    Err(format!(
        "{}: the pack declares a {}, and this answer is a {}{}",
        row.subject,
        declared,
        given.unwrap_or("null"),
        hint
    ))
}

/// The gates an answer says yes to: the chosen option of a choice, or the param itself
/// when it is answered `true`. A choice answered "none" says yes to nothing.
fn gates_said_yes(row: &QuestionRow, value: &serde_yaml::Value) -> Vec<String> {
    if row.kind == "oneof" {
        return value
            .as_str()
            .filter(|s| *s != crate::questions::NO_BRANCH)
            .map(|s| vec![s.to_string()])
            .unwrap_or_default();
    }
    match value.as_bool() {
        Some(true) => vec![row.subject.clone()],
        _ => Vec::new(),
    }
}

/// Refuse a yes to a pack whose requirements are off in `src` — as `satz add-pack`
/// refuses the same switch — before anything is written.
fn refuse_unmet(
    graph: Option<&PackGraph>,
    runtime: &ToolConfig,
    src: &str,
    row: &QuestionRow,
    value: &serde_yaml::Value,
) -> Result<(), String> {
    let Some(g) = graph else { return Ok(()) };
    for gate in gates_said_yes(row, value) {
        let unmet = crate::packs::unmet_for_gate(g, Path::new(&runtime.presets_dir), src, &gate)?;
        if !unmet.is_empty() {
            return Err(format!("{} = yes: {}", gate, unmet.join("; ")));
        }
    }
    Ok(())
}

/// Write `after` over `before` only when the estate still reads with it — its params
/// resolve and every line stands where it may — and restore `before` when it does not:
/// an answer that leaves an estate satz refuses is refused, with the file as it was (the
/// rule `satz add-pack` follows, ADR 0023). The questions report is the proof rather than
/// the whole compile, because it needs no provider schema: an estate is answered on day
/// 0, before `satz update-schema` has run.
fn write_proven(estate: &Path, runtime: &ToolConfig, before: &str, after: &str) -> Result<(), String> {
    crate::fsx::write_edited_satz(estate, before, after).map_err(|e| e.to_string())?;
    match questions_report(estate, runtime) {
        Ok(_) => Ok(()),
        Err(e) => Err(restored(estate, before, &format!("the answer leaves an estate satz refuses: {}", e))),
    }
}

/// `why`, with the estate written back to `before`: said once it is back, and said
/// louder when it could not be put back.
fn restored(estate: &Path, before: &str, why: &str) -> String {
    match crate::fsx::write_verbatim(estate, before) {
        Ok(()) => format!("{} — nothing is written, {} is as it was", why, estate.display()),
        Err(w) => format!("{} — and restoring {} failed: {}", why, estate.display(), w),
    }
}

/// Bind a set of answers, each validated against a question the estate asks, and
/// with `accept_defaults` every default the report offers. Returns how many params
/// were written. Nothing is written when any answer is refused, when a yes switches on
/// a pack whose requirements are off, or when the result does not compile.
pub(crate) fn apply(
    estate: &Path,
    runtime: &ToolConfig,
    answers: &BTreeMap<String, serde_yaml::Value>,
    accept_defaults: bool,
) -> Result<usize, String> {
    let before = crate::fsx::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    match apply_to(estate, runtime, answers, accept_defaults, &before) {
        Ok(n) => Ok(n),
        // `accept_defaults` writes once in the middle to read the report the answers
        // leave; a refusal after that puts the file back
        Err(e) => match crate::fsx::read_to_string(estate) {
            Ok(now) if now != before => Err(restored(estate, &before, &e)),
            _ => Err(e),
        },
    }
}

fn apply_to(
    estate: &Path,
    runtime: &ToolConfig,
    answers: &BTreeMap<String, serde_yaml::Value>,
    accept_defaults: bool,
    before: &str,
) -> Result<usize, String> {
    let report = questions_report(estate, runtime).map_err(|e| e.to_string())?;
    let graph = crate::pack_graph::read(Path::new(&runtime.presets_dir)).map_err(|e| e.to_string())?;
    let mut src = before.to_string();
    let notices = crate::notices::estate_notices(estate, runtime)?;
    let mut n = 0;
    for (name, value) in answers {
        // a notice's param is acknowledged with `true`, and with nothing else
        if let Some(notice) = notices.iter().find(|x| x.param == *name) {
            if value != &serde_yaml::Value::Bool(true) {
                return Err(format!("{}: acknowledges the notice of {} — the one answer is true", name, notice.pack));
            }
            src = bind(&src, name, value)?;
            n += 1;
            continue;
        }
        let row = report.questions.iter().find(|q| q.subject == *name).ok_or_else(|| {
            format!(
                "{}: no pack this estate uses asks that or names it in a notice. An answer names a question's \
                 subject — `satz questions {} --format text --out -` lists them",
                name,
                estate.display()
            )
        })?;
        refuse_unmet(graph.as_ref(), runtime, &src, row, value)?;
        src = answer(&src, row, value, graph.as_ref())?;
        n += 1;
    }
    if accept_defaults {
        // Defaults become usable as answers land — a derived name once its input is
        // known — so accepting is judged on the report as it stands after the answers.
        let now = if answers.is_empty() {
            report
        } else {
            crate::fsx::write_edited_satz(estate, before, &src).map_err(|e| e.to_string())?;
            questions_report(estate, runtime).map_err(|e| e.to_string())?
        };
        for q in now.questions.iter().filter(|q| q.state == "unanswered") {
            if let Some(d) = &q.default {
                refuse_unmet(graph.as_ref(), runtime, &src, q, d)?;
                src = answer(&src, q, d, graph.as_ref())?;
                n += 1;
            }
        }
    }
    if n > 0 {
        write_proven(estate, runtime, before, &src)?;
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
    let presets_dir = Path::new(&runtime.presets_dir);
    let graph = crate::pack_graph::read(presets_dir).map_err(|e| e.to_string())?;
    w(out, &format!("\ninterview — {}\n", estate.display()))?;
    if graph.is_none() {
        w(
            out,
            &format!(
                "  {} is not here: answers are written, and no pack line is switched on — `satz get-presets` fetches it\n",
                presets_dir.join(crate::pack_graph::GRAPH_FILE).display()
            ),
        )?;
    }
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
    // A yes that switches a pack on opens its notice; it is shown once, when it opens.
    // So is what it adds to another pack's list params.
    let mut open_notices = crate::notices::open(estate, runtime)?;
    let mut contributions = crate::packs::contributions(estate, runtime)?;
    let mut switch_opened = |out: &mut dyn Write| -> Result<(), String> {
        let now = crate::notices::open(estate, runtime)?;
        w(out, &crate::notices::render(&crate::notices::opened(&open_notices, &now)))?;
        open_notices = now;
        let now = crate::packs::contributions(estate, runtime)?;
        w(out, &crate::packs::render_contributions(&crate::packs::contributed(&contributions, &now)))?;
        contributions = now;
        Ok(())
    };
    if accept_defaults {
        let n = apply(estate, runtime, &BTreeMap::new(), true)?;
        w(out, &format!("  accepted {} default(s).\n", n))?;
        switch_opened(out)?;
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
            match parse_answer(t, offered, q.shape) {
                Ok(v) => v,
                Err(e) => {
                    w(out, &format!("  {}\n", e))?;
                    continue;
                }
            }
        };
        let src = crate::fsx::read_to_string(estate).map_err(|e| e.to_string())?;
        let new_src = match refuse_unmet(graph.as_ref(), runtime, &src, q, &value).and_then(|()| answer(&src, q, &value, graph.as_ref())) {
            Ok(s) => s,
            Err(e) => {
                w(out, &format!("  {}\n", e))?;
                continue;
            }
        };
        if let Err(e) = write_proven(estate, runtime, &src, &new_src) {
            w(out, &format!("  {}\n", e))?;
            continue;
        }
        w(out, &format!("  ✓ {} = {}\n", q.subject, literal(&value)))?;
        switch_opened(out)?;
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
        if !q.required {
            let is_default = picked.as_deref() == Some(crate::questions::NO_BRANCH);
            if is_default {
                default_no = Some(0);
            }
            s.push_str(&format!("  0) none{}\n", if is_default { "  (default)" } else { "" }));
        }
        match default_no {
            Some(n) => s.push_str(&format!("  [{}] > ", n)),
            None => s.push_str("  > "),
        }
        return s;
    }
    match (q.state, &q.current, &q.default) {
        ("answered", Some(v), _) => s.push_str(&format!("  [{}] > ", q.shown(v))),
        (_, _, Some(d)) => s.push_str(&format!("  [{}] > ", q.shown(d))),
        _ => s.push_str("  no default — a value is needed\n  > "),
    }
    s
}

/// A `oneof` answer: the option's number in the list, or its param name — or, for a choice
/// that is not required, `0` or `none`.
fn choose(q: &QuestionRow, text: &str) -> Option<String> {
    if !q.required && (text == "0" || text == crate::questions::NO_BRANCH) {
        return Some(crate::questions::NO_BRANCH.to_string());
    }
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

    /// Found on a live estate: the `= ""` lines were deleted by hand so the interview
    /// would ask them, which left the rest padded for a name that was gone; the
    /// answers were appended unaligned, and the columns never came back.
    #[test]
    fn an_answer_appended_to_a_hand_edited_block_leaves_it_aligned() {
        let src = "estate e\n\nparams {\n  customer_id              = \"C0example\"\n  customer_shortname       = \"acme\"\n}\n\n\
                   google_folder {\n  infra_folder {\n    display_name   =   \"Infrastructure\"\n  }\n}\n";
        let out = bind(src, "customer_organization_id", &serde_yaml::Value::String("123456789012".into())).unwrap();
        assert!(
            out.contains(
                "params {\n  customer_id              = \"C0example\"\n  customer_shortname       = \"acme\"\n  customer_organization_id = \"123456789012\"\n}"
            ),
            "{out}"
        );
        // outside the block the author's layout is untouched, odd spacing included
        assert!(out.contains("    display_name   =   \"Infrastructure\""), "{out}");
    }
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
        // and an appended binding joins the `=` column of the one already there
        assert!(three.contains("customer_id   = \"C0other\"\n  customer_id_x = true\n"), "{}", three);
    }

    #[test]
    fn bind_steps_over_braces_in_strings_and_comments() {
        let src = "params {\n  a = \"}\" // } not the end\n  // } nor this\n}\nother { }\n";
        let out = bind(src, "b", &yaml("v")).unwrap();
        assert_eq!(out, "params {\n  a = \"}\" // } not the end\n  // } nor this\n  b = \"v\"\n}\nother { }\n");
        assert!(bind("estate x\n", "a", &yaml("v")).unwrap_err().contains("no `params { }` block"));
    }

    /// Found on a live estate: `cis_baseline_adopted = true` stood twice in one
    /// `params {}` — a writer had appended beside a binding that was already there,
    /// and satz refuses to compile what it wrote. Whatever the shape of the block, a
    /// subject that is bound is REPLACED.
    #[test]
    fn a_subject_that_is_already_bound_is_replaced_never_appended_beside() {
        let once = |src: &str| {
            let out = bind(src, "cis_baseline_adopted", &serde_yaml::Value::Bool(true)).unwrap();
            assert_eq!(out.matches("cis_baseline_adopted").count(), 1, "bound twice:\n{out}");
            assert!(out.contains("cis_baseline_adopted = true"), "{out}");
        };
        // a `}` in a `#` comment and in a block comment ends no block
        once("estate e\n\nparams {\n  # not the end }\n  cis_baseline_adopted = false\n}\n");
        once("estate e\n\nparams {\n  /* nor this } */\n  cis_baseline_adopted = false\n}\n");
        // two bindings on one line are two bindings
        once("estate e\n\nparams {\n  other = 1 cis_baseline_adopted = false\n}\n");
        // a second block binds it, and the first is where an append would have landed
        once("estate e\n\nparams {\n  other = 1\n}\n\nparams {\n  cis_baseline_adopted = false\n}\n");
        // a raw HCL body is opaque: what it says is not the estate's params
        once("estate e\n\nhcl trust \"reviewed\" {\n  locals {\n    params { other = 1 }\n  }\n}\n\nparams {\n  cis_baseline_adopted = false\n}\n");
    }

    /// A binding that is commented out binds nothing — the answer is appended, once,
    /// and the comment is left where its author put it.
    #[test]
    fn a_commented_out_binding_is_not_a_binding() {
        let src = "estate e\n\nparams {\n  // cis_baseline_adopted = true\n}\n";
        let out = bind(src, "cis_baseline_adopted", &serde_yaml::Value::Bool(true)).unwrap();
        assert_eq!(out, "estate e\n\nparams {\n  // cis_baseline_adopted = true\n  cis_baseline_adopted = true\n}\n");
        // and the second write replaces the first: one live binding, whatever the run
        let again = bind(&out, "cis_baseline_adopted", &serde_yaml::Value::Bool(true)).unwrap();
        assert_eq!(again, out);
    }

    /// The replace touches the value and nothing else: the rest of the file is the
    /// same bytes it was.
    #[test]
    fn a_replaced_value_is_the_only_byte_that_moves() {
        let src = "estate e\n\nparams {\n  a                    = 1\n  cis_baseline_adopted = false // day 0\n  z                    = \"x\"\n}\n\ngoogle_folder {\n  f {\n    display_name =  \"F\"\n  }\n}\n";
        let out = bind(src, "cis_baseline_adopted", &serde_yaml::Value::Bool(true)).unwrap();
        assert_eq!(out, src.replace("= false // day 0", "= true // day 0"));
    }

    #[test]
    fn answering_yes_switches_on_that_pack_and_nothing_else() {
        let src = "\
estate e

params {
}

// once the audit archive exists
// use \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel
// use \"presets/integrations/microsoft-sentinel-auditlogs.satz\" when use_sentinel_auditlogs
// use \"presets/organization-budget.satz\" when use_budget

google_folder {
  infra_folder {
    // use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink
  }
}
";
        let graph = crate::template::tests::shipped();
        let g = Some(&graph);
        let yes = serde_yaml::Value::Bool(true);
        let no = serde_yaml::Value::Bool(false);

        // the pack is matched by path: `use_sentinel` does not drag in the log fragment
        let out = pack_lines(src, "use_sentinel", &yes, g).unwrap();
        assert!(out.contains("\nuse \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel\n"));
        assert!(out.contains("// use \"presets/integrations/microsoft-sentinel-auditlogs.satz\""), "{}", out);

        // answering no changes nothing — a `use … when` already emits nothing, and deleting
        // somebody's pack line is not what an answer does
        assert_eq!(pack_lines(src, "use_budget", &no, g).unwrap(), src);

        // a pack is used at the top level: a commented line left inside a folder by an
        // estate written before that is refused, naming the move, and nothing changes
        let e = pack_lines(src, "use_audit_logsink", &yes, g).unwrap_err();
        assert!(e.contains("commented inside `google_folder.infra_folder`") && e.contains("move the commented line"), "{e}");

        // a value that is not a boolean true, or no graph, leaves the file alone
        assert_eq!(pack_lines(src, "use_budget", &serde_yaml::Value::String("yes".into()), g).unwrap(), src);
        assert_eq!(pack_lines(src, "use_budget", &yes, None).unwrap(), src);
    }

    fn choice(required: bool) -> QuestionRow {
        QuestionRow {
            subject: "notice".into(),
            kind: "oneof",
            required,
            options: vec![crate::questions::OptionRow { param: "notice_pubsub".into(), label: "Pub/Sub".into(), why: None, selected: false }],
            ..Default::default()
        }
    }

    /// A choice that is not required is answered "none" — every option `false` — and says
    /// yes to no pack; a required choice has no such answer.
    #[test]
    fn a_choice_that_is_not_required_is_answered_none() {
        let src = "estate e\n\nparams {\n}\n";
        let none = serde_yaml::Value::String(crate::questions::NO_BRANCH.into());
        let out = answer(src, &choice(false), &none, None).unwrap();
        assert!(out.contains("notice_pubsub = false"), "{}", out);
        assert!(gates_said_yes(&choice(false), &none).is_empty());
        let err = answer(src, &choice(true), &none, None).unwrap_err();
        assert!(err.contains("not one of its options — notice_pubsub"), "{}", err);
        assert!(!err.contains(", none"), "a required choice offers no none: {}", err);
        let err = answer(src, &choice(false), &serde_yaml::Value::String("webhook".into()), None).unwrap_err();
        assert!(err.contains("notice_pubsub, none"), "{}", err);
        assert_eq!(choose(&choice(false), "0").as_deref(), Some("none"));
        assert_eq!(choose(&choice(true), "0"), None);
        assert!(present(&choice(false)).contains("  0) none\n"), "{}", present(&choice(false)));
    }

    /// Answering estate-core's `workload_folder_name` writes the section that publishes
    /// the workload folder, as `satz init` does; the same subject from another pack writes
    /// nothing.
    #[test]
    fn answering_the_workload_folder_writes_its_section() {
        let src = crate::template::skeleton("x", None);
        let row = |pack: &str| QuestionRow { subject: "workload_folder_name".into(), kind: "param", pack: pack.into(), ..Default::default() };
        let s = |v: &str| serde_yaml::Value::String(v.into());
        let org = answer(&src, &row("estate_core"), &s(""), None).unwrap();
        assert!(org.contains("workload_folder_name = \"\""), "{}", org);
        assert!(org.contains("export \"workload_folder\" = \"organizations/{customer_organization_id}\""), "{}", org);
        assert!(!org.contains("google_folder.workload_folder"), "{}", org);
        let folder = answer(&src, &row("estate_core"), &s("Workloads"), None).unwrap();
        assert!(folder.contains("export \"workload_folder\" = \"${{google_folder.workload_folder.name}}\""), "{}", folder);
        let err = answer(&folder, &row("estate_core"), &s(""), None).unwrap_err();
        assert!(err.contains("publishes the workload folder as the folder"), "{}", err);
        let other = answer(&src, &row("someone_else"), &s("Workloads"), None).unwrap();
        assert!(!other.contains("export \"workload_folder\""), "{}", other);
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
        assert_eq!(parse_answer("no", Some(&b), None).unwrap(), serde_yaml::Value::Bool(false));
        assert!(parse_answer("maybe", Some(&b), None).is_err());
        let n = serde_yaml::from_str::<serde_yaml::Value>("30").unwrap();
        assert_eq!(parse_answer("400", Some(&n), None).unwrap(), serde_yaml::from_str::<serde_yaml::Value>("400").unwrap());
        let l = serde_yaml::from_str::<serde_yaml::Value>("[]").unwrap();
        assert_eq!(
            parse_answer("in:eu-locations, in:us-locations", Some(&l), None).unwrap(),
            serde_yaml::from_str::<serde_yaml::Value>("[\"in:eu-locations\", \"in:us-locations\"]").unwrap()
        );
        assert_eq!(parse_answer("true", None, None).unwrap(), yaml("true"));
    }

    /// Found on a customer estate: a param declared `[]` offers no default, and one
    /// address typed at the prompt was written as a string — `tofu apply` then refused
    /// "set of string required, but have string". The declared shape decides, with
    /// nothing offered as well.
    #[test]
    fn with_nothing_offered_the_declared_shape_decides() {
        let one = parse_answer("security@example.com", None, Some("list")).unwrap();
        assert_eq!(one, serde_yaml::from_str::<serde_yaml::Value>("[\"security@example.com\"]").unwrap());
        assert_eq!(parse_answer("yes", None, Some("bool")).unwrap(), serde_yaml::Value::Bool(true));
        assert!(parse_answer("{ a = 1 }", None, Some("map")).is_err());
        // the declared shape outranks a wrongly shaped value already in the estate
        assert_eq!(
            parse_answer("a@example.com", Some(&yaml("old@example.com")), Some("list")).unwrap(),
            serde_yaml::from_str::<serde_yaml::Value>("[\"a@example.com\"]").unwrap()
        );
    }

    #[test]
    fn an_answer_that_contradicts_the_declared_shape_is_refused_by_name() {
        let row = |shape: &'static str| QuestionRow {
            subject: "access_approval_notification_emails".into(),
            kind: "param",
            prompt: String::new(),
            why: None,
            reversal: "edit",
            blast: "low",
            state: "unanswered",
            current: None,
            default: None,
            blocking: true,
            shape: Some(shape),
            pack_description: String::new(),
            recommend: None,
            options: vec![],
            required: false,
            empty: None,
            from: String::new(),
            pack: String::new(),
        };
        let err = check_shape(&row("list"), &yaml("security@example.com")).unwrap_err();
        assert!(err.contains("declares a list") && err.contains("[\"security@example.com\"]"), "{err}");
        // a gate answered with the string "true" never switched its pack on
        assert!(check_shape(&row("bool"), &yaml("true")).unwrap_err().contains("unquoted"));
        assert!(check_shape(&row("list"), &serde_yaml::from_str("[a]").unwrap()).is_ok());
        assert!(check_shape(&row("string"), &serde_yaml::from_str("400").unwrap()).is_ok());
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

    /// `empty = "…"` makes "" an answer: offered while unbound, accepted by
    /// `--accept-defaults`, and counted once bound. A question without it keeps "" as a
    /// value nobody has given.
    #[test]
    fn a_question_that_says_what_empty_means_takes_empty_as_an_answer() {
        let dir = std::env::temp_dir().join(format!("satz-iv-{}-empty", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("asks.satz"),
            "pack asks version \"1.0\"\n\nparams {\n  folder = \"\"\n  label  = \"\"\n}\n\nquestion folder { prompt = \"Folder\" reversal = edit blast = none empty = \"the organisation\" }\nquestion label { prompt = \"Label\" reversal = edit blast = none }\n",
        )
        .unwrap();
        let estate = dir.join("e.satz");
        std::fs::write(&estate, "estate e\n\nparams {\n}\n\nuse \"asks.satz\"\n").unwrap();
        let mut cfg: ToolConfig = toml::from_str("").unwrap();
        cfg.include_dirs = vec![dir.to_string_lossy().into_owned()];
        let r = questions_report(&estate, &cfg).unwrap();
        let by = |r: &QuestionsReport, s: &str| r.questions.iter().find(|q| q.subject == s).unwrap().clone();
        assert_eq!(by(&r, "folder").default, Some(yaml("")), "\"\" is offered");
        assert!(!by(&r, "folder").blocking);
        assert!(present(&by(&r, "folder")).contains("[\"\" (the organisation)]"), "{}", present(&by(&r, "folder")));
        assert!(by(&r, "label").blocking, "without `empty`, \"\" is no default");
        // accepting the defaults binds "" for the question that means something by it
        assert_eq!(apply(&estate, &cfg, &BTreeMap::new(), true).unwrap(), 1);
        let src = std::fs::read_to_string(&estate).unwrap();
        assert!(bound(&src, "folder", "\"\""), "{}", src);
        let r = questions_report(&estate, &cfg).unwrap();
        assert_eq!(by(&r, "folder").state, "answered");
        // and bound "" by hand: answered with `empty`, still open without it
        std::fs::write(&estate, "estate e\n\nparams {\n  folder = \"\"\n  label  = \"\"\n}\n\nuse \"asks.satz\"\n").unwrap();
        let r = questions_report(&estate, &cfg).unwrap();
        assert_eq!(by(&r, "folder").state, "answered");
        assert_eq!(by(&r, "label").state, "unanswered");
        let _ = std::fs::remove_dir_all(&dir);
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

    /// A value that opens a list and closes it three lines down is one value, and the
    /// whole of it is what an answer replaces — never its first line, with the tail
    /// left behind as stray text.
    #[test]
    fn a_value_that_spans_lines_is_replaced_whole() {
        let src = "estate e\n\nparams {\n  members = [\n    \"a\",\n  ]\n  other   = 1\n}\n";
        let out = bind(src, "members", &serde_yaml::Value::Sequence(vec![yaml("b")])).unwrap();
        assert_eq!(out, "estate e\n\nparams {\n  members = [\"b\"]\n  other   = 1\n}\n");
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
