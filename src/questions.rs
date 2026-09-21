//! `satz questions` — what this estate can be asked, and what the answers cost.
//!
//! A pack declares its params and its claims; it also declares what to ask a
//! customer so those params can be filled. This command joins the two: every
//! question the estate's packs contribute, against the value the estate already
//! carries for it.
//!
//! Read-only, schema-free, offline. An interview happens before anyone has run
//! `update-schema`, so nothing here needs the provider registry.

// schemars comes through rmcp: one version in the tree, no second dependency to pin
use rmcp::schemars;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use satz_core::pipeline::{estate_questions, Env};

use crate::settings::ToolConfig;

/// One question, joined with the answer the estate currently carries.
#[derive(Debug, Clone, Default, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct QuestionRow {
    /// the param it answers, or the group name for a choice
    pub subject: String,
    /// param | oneof
    pub kind: &'static str,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// edit | state_surgery | recreate
    pub reversal: &'static str,
    /// none | low | high
    pub blast: &'static str,
    /// answered | unanswered | not-applicable
    ///
    /// ANSWERED means the estate's own `params {}` binds this — to the pack's
    /// default or to anything else. Accepting a default is an answer, and it is
    /// recorded by writing the default in. There is no third state for "a value
    /// is there and nobody looked": that state is what pointed eight alert
    /// policies at a project nothing creates. `not-applicable` is a question
    /// whose `ask_when` param is false; it counts toward nothing.
    pub state: &'static str,
    /// The estate's own value when answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub current: Option<serde_yaml::Value>,
    /// The pack's default when unanswered and one exists — what an interview
    /// OFFERS. Absent when the pack cannot know (an empty string, an empty list).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub default: Option<serde_yaml::Value>,
    /// Unanswered AND no usable default: this cannot be settled by accepting
    /// anything; a value has to be typed before the estate may touch an
    /// organisation.
    pub blocking: bool,
    /// The shape the pack declares for the param — `string`, `number`, `bool`, `list`
    /// or `map` — and so the shape an answer must have. A param declared `[]` has no
    /// default to offer, and without this a client typing one address wrote a string
    /// where the provider wants a set. Absent for a choice, which is answered by an
    /// option's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<&'static str>,
    /// The pack's own description — its header's first paragraph — so an
    /// interview can say what a pack is FOR before asking about its params.
    pub pack_description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommend: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub options: Vec<OptionRow>,
    /// the file that declared it — a fork asks its own questions
    pub from: String,
    pub pack: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct OptionRow {
    pub param: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct QuestionsReport {
    pub estate: String,
    pub questions: Vec<QuestionRow>,
    pub summary: QuestionsSummary,
}

#[derive(Debug, Clone, Default, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct QuestionsSummary {
    pub total: usize,
    pub answered: usize,
    pub unanswered: usize,
    /// `ask_when` false: not asked, not counted
    pub not_applicable: usize,
    /// unanswered with no default to accept — these need a typed value
    pub blocking: usize,
    /// questions whose answer is expensive to change — recreate, or high blast
    pub one_way_doors: usize,
    /// THE GATE: every applicable question is answered. `bootstrap` and
    /// `transpile --apply` refuse while this is false.
    pub complete: bool,
}

/// How the schema-free walks find a `use`d file: beside the estate, then in the include
/// dirs — the compile's order.
pub(crate) fn loader(input: &Path, runtime: &ToolConfig) -> impl Fn(&str) -> Result<String, String> {
    let base = input.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let include_dirs = runtime.include_dirs.clone();
    move |p: &str| -> Result<String, String> {
        let direct = base.join(p);
        if direct.exists() {
            return crate::fsx::read_to_string(&direct).map_err(|e| e.to_string());
        }
        for d in &include_dirs {
            let c = Path::new(d).join(p);
            if c.exists() {
                return crate::fsx::read_to_string(&c).map_err(|e| e.to_string());
            }
        }
        Err(format!("{}: not found", p))
    }
}

/// Read the estate and its packs, and join every question with its current value.
pub(crate) fn questions_report(
    input: &Path,
    runtime: &ToolConfig,
) -> Result<QuestionsReport, Box<dyn std::error::Error>> {
    let src = crate::fsx::read_to_string(input)?;
    let load = loader(input, runtime);
    let file_name = input.display().to_string();
    let (packs, _, env) = estate_questions(&file_name, &src, &load)
        .map_err(|e| format!("{}:{}: {}", e.file, e.line, e.msg))?;

    // The estate's OWN bindings. The fold cannot answer "did a human decide
    // this": a pack default and an estate's decision look the same from there.
    // What the estate file itself writes is the record.
    let own: BTreeSet<String> = satz_core::satz::parse(&src)
        .map_err(|e| format!("{}:{}: {}", file_name, e.line, e.msg))?
        .params
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();

    // Each declaring file once: its description (the header's first paragraph)
    // and its raw param declarations — the latter to see what a default is
    // BUILT FROM, which decides whether it may be offered.
    let mut facts: BTreeMap<String, PackFacts> = BTreeMap::new();
    for pq in &packs {
        if facts.contains_key(&pq.file) {
            continue;
        }
        let text = if pq.file == file_name { Ok(src.clone()) } else { load(&pq.file) };
        facts.insert(pq.file.clone(), pack_facts(text, &pq.file));
    }

    let mut rows = Vec::new();
    let mut summary = QuestionsSummary::default();
    for pq in &packs {
        for q in &pq.questions {
            let applicable = q.ask_when.as_ref().map(|gate| truthy(env.get(gate))).unwrap_or(true);
            let declared = facts.get(&pq.file);
            let (state, current, default, blocking) = if !applicable {
                ("not-applicable", None, None, false)
            } else if q.oneof {
                // A choice is answered when the estate itself binds one of its
                // options; the pack's own `= true` is not a decision — but it is
                // the default an interview may offer, by the option's name.
                if q.options.iter().any(|o| own.contains(&o.param)) {
                    ("answered", None, None, false)
                } else {
                    let picked = q
                        .options
                        .iter()
                        .find(|o| truthy(env.get(&o.param)))
                        .map(|o| serde_yaml::Value::String(o.param.clone()));
                    let blocking = picked.is_none();
                    ("unanswered", None, picked, blocking)
                }
            } else if own.contains(&q.subject) && !restates_unknown(&q.subject, declared, &env) {
                ("answered", env.get(&q.subject).cloned(), None, false)
            } else if own.contains(&q.subject) {
                // bound to nothing: open, and the derivation is the offer once its inputs
                // are answered — the estate's `""` hides the pack's default from the fold
                let default = derived_default(&q.subject, declared, &env);
                let blocking = default.is_none();
                ("unanswered", None, default, blocking)
            } else {
                let usable = match declared {
                    Some(f) => default_usable(&q.subject, &f.params, &own, &env, 0),
                    None => env.get(&q.subject).map(|v| !is_empty(v)).unwrap_or(false),
                };
                let default = if usable { env.get(&q.subject).cloned() } else { None };
                ("unanswered", None, default, !usable)
            };
            let shape = if q.oneof { None } else { declared_shape(&q.subject, declared.map(|f| &f.params), &env, 0) };
            let one_way = q.reversal == satz_core::satz::Reversal::Recreate
                || q.blast == satz_core::satz::Blast::High;
            match state {
                "not-applicable" => summary.not_applicable += 1,
                "answered" => {
                    summary.total += 1;
                    summary.answered += 1;
                }
                _ => {
                    summary.total += 1;
                    summary.unanswered += 1;
                    if blocking {
                        summary.blocking += 1;
                    }
                }
            }
            if one_way && state != "not-applicable" {
                summary.one_way_doors += 1;
            }
            rows.push(QuestionRow {
                subject: q.subject.clone(),
                kind: if q.oneof { "oneof" } else { "param" },
                prompt: q.prompt.clone(),
                why: q.why.clone(),
                reversal: q.reversal.as_str(),
                blast: q.blast.as_str(),
                state,
                current,
                default,
                blocking,
                shape,
                pack_description: declared.map(|f| f.description.clone()).unwrap_or_default(),
                recommend: q.recommend.as_ref().map(crate::doc_packs::value_text),
                options: q
                    .options
                    .iter()
                    .map(|o| OptionRow {
                        param: o.param.clone(),
                        label: o.label.clone(),
                        why: o.why.clone(),
                        selected: truthy(env.get(&o.param)),
                    })
                    .collect(),
                from: pq.file.clone(),
                pack: pq.pack.clone(),
            });
        }
    }

    summary.complete = summary.unanswered == 0;
    Ok(QuestionsReport { estate: file_name, questions: rows, summary })
}

/// The gate. An estate may be transpiled and revised in as many passes as it
/// takes; it may not touch an organisation while a question is open. Names the
/// open ones — the ones that need a typed value first — and both ways to answer.
pub(crate) fn require_complete(input: &Path, runtime: &ToolConfig, action: &str) -> Result<(), String> {
    let r = questions_report(input, runtime).map_err(|e| e.to_string())?;
    if r.summary.complete {
        return Ok(());
    }
    let mut open: Vec<&QuestionRow> = r.questions.iter().filter(|q| q.state == "unanswered").collect();
    open.sort_by_key(|q| !q.blocking);
    let names: Vec<String> = open
        .iter()
        .map(|q| if q.blocking { format!("{} (needs a value)", q.subject) } else { q.subject.clone() })
        .collect();
    Err(format!(
        "{} refused: {} question(s) unanswered — {}. Every question must be answered before the \
         estate touches an organisation. `satz questions {} --unanswered --format text --out -` \
         lists them with their \
         defaults; write the answer (or the default) into the estate's params.",
        action,
        open.len(),
        names.join(", "),
        input.display()
    ))
}

/// The shape of a value, in the words the report uses.
pub(crate) fn value_shape(v: &serde_yaml::Value) -> Option<&'static str> {
    match v {
        serde_yaml::Value::String(_) => Some("string"),
        serde_yaml::Value::Number(_) => Some("number"),
        serde_yaml::Value::Bool(_) => Some("bool"),
        serde_yaml::Value::Sequence(_) => Some("list"),
        serde_yaml::Value::Mapping(_) => Some("map"),
        _ => None,
    }
}

/// The shape a param's own declaration gives it. A declaration that names another param
/// (`x = y`) has that param's shape; one this file does not declare is read off the
/// folded value, where the declaring file already decided it.
fn declared_shape(
    name: &str,
    params: Option<&BTreeMap<String, satz_core::satz::Value>>,
    env: &Env,
    depth: usize,
) -> Option<&'static str> {
    use satz_core::satz::Value;
    match params.and_then(|p| p.get(name)) {
        Some(Value::Str(_)) => Some("string"),
        Some(Value::Num(_)) => Some("number"),
        Some(Value::Bool(_)) => Some("bool"),
        Some(Value::List(_)) => Some("list"),
        Some(Value::Obj(_)) => Some("map"),
        Some(Value::Ref(other)) if depth < 8 => declared_shape(other, params, env, depth + 1),
        Some(Value::Ref(_)) => None,
        None => env.get(name).and_then(value_shape),
    }
}

/// What one declaring file contributes besides its questions.
#[derive(Default)]
struct PackFacts {
    description: String,
    params: BTreeMap<String, satz_core::satz::Value>,
}

fn pack_facts(src: Result<String, String>, rel: &str) -> PackFacts {
    let Ok(src) = src else { return PackFacts::default() };
    // The first paragraph: what the pack is FOR. The rest of the header is how to
    // use it, which the interview is in the middle of doing.
    let description = crate::doc_packs::header(&src, Path::new(rel))
        .ok()
        .map(|h| h.purpose.iter().take_while(|l| !l.is_empty()).cloned().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let params = satz_core::satz::parse(&src)
        .map(|f| f.params.into_iter().map(|(n, v, _)| (n, v)).collect())
        .unwrap_or_default();
    PackFacts { description, params }
}

/// The params a declared value is built from: `{a}` interpolations and bare references.
fn refs(v: &satz_core::satz::Value, out: &mut Vec<String>) {
    use satz_core::satz::{StrPart, Value};
    match v {
        Value::Str(parts) => {
            for p in parts {
                if let StrPart::Param(n) = p {
                    out.push(n.clone());
                }
            }
        }
        Value::Ref(n) => out.push(n.clone()),
        Value::List(items) => items.iter().for_each(|i| refs(i, out)),
        _ => {}
    }
}

/// May an interview OFFER the pack's default for `name`? Not when it is empty,
/// and not when it is built from a param nobody has answered and no default can
/// stand in for: `"{customer_shortname}-infra-001"` with the short name still
/// open folds to `-infra-001`, which is a string and not a default.
fn default_usable(
    name: &str,
    params: &BTreeMap<String, satz_core::satz::Value>,
    own: &BTreeSet<String>,
    env: &Env,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let Some(v) = env.get(name) else { return false };
    if is_empty(v) {
        return false;
    }
    // declared in another file: the fold has resolved it, and that is the answer
    let Some(decl) = params.get(name) else { return true };
    let mut needed = Vec::new();
    refs(decl, &mut needed);
    needed.iter().all(|r| own.contains(r) || default_usable(r, params, own, env, depth + 1))
}

/// `init` names the file after the customer id; an interview starts before that
/// id is known. Once the estate binds it, this is where the file belongs.
pub(crate) fn rename_to(estate: &Path, r: &QuestionsReport) -> Option<String> {
    let row = r.questions.iter().find(|q| q.subject == "customer_id" && q.state == "answered")?;
    let id = row.current.as_ref()?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let stem = estate.file_stem()?.to_str()?;
    (stem != id).then(|| format!("{}.satz", id))
}

/// Whether the estate's own binding of a param is no answer at all. Two shapes are:
///
/// - The pack declares the param `""` because no default is possible — the identity
///   params in `presets/estate-core.satz`, organisation, directory customer, domain,
///   billing account. `satz init` writes `""` for the ones it could not derive, and
///   `bootstrap`'s day-0 gate refuses that value by name, so the question has to
///   agree with the gate or the interview asks nothing while bootstrap refuses.
/// - The pack DERIVES the param from others — `infra_project_name =
///   "{customer_shortname}-infra-001"`. init writes `""` when the shortname is not
///   known yet, and an empty binding overrides the derivation with nothing: a project
///   id cannot be empty. It is open, and `derived_default` offers the derivation.
///
/// A pack default that is a plain non-empty literal is different: `infra_folder_name
/// = ""` against `"Infrastructure"` is a real choice (no folder) and stays answered.
fn restates_unknown(name: &str, declared: Option<&PackFacts>, env: &Env) -> bool {
    let own_empty = env.get(name).map(is_empty).unwrap_or(true);
    let not_a_choice = match declared.and_then(|f| f.params.get(name)) {
        Some(satz_core::satz::Value::Str(parts)) => parts.iter().all(|p| match p {
            satz_core::satz::StrPart::Lit(l) => l.trim().is_empty(),
            satz_core::satz::StrPart::Param(_) => false,
        }) || parts.iter().any(|p| matches!(p, satz_core::satz::StrPart::Param(_))),
        Some(satz_core::satz::Value::Ref(_)) => true,
        Some(_) => false,
        // a pack that declares no default at all has none to restate
        None => true,
    };
    own_empty && not_a_choice
}

/// The pack's derivation of a param, worked out from what the estate already
/// answers: `"{customer_shortname}-infra-001"` with `customer_shortname = "stec"` is
/// `"stec-infra-001"`. `None` while an input is still empty — the question is then
/// blocking, and becomes offerable the moment that input is answered — and for a
/// pack default that derives nothing.
fn derived_default(name: &str, declared: Option<&PackFacts>, env: &Env) -> Option<serde_yaml::Value> {
    let text = |v: &serde_yaml::Value| -> Option<String> {
        match v {
            serde_yaml::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    match declared.and_then(|f| f.params.get(name))? {
        satz_core::satz::Value::Str(parts) if parts.iter().any(|p| matches!(p, satz_core::satz::StrPart::Param(_))) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    satz_core::satz::StrPart::Lit(l) => out.push_str(l),
                    satz_core::satz::StrPart::Param(p) => out.push_str(&text(env.get(p)?)?),
                }
            }
            Some(serde_yaml::Value::String(out))
        }
        satz_core::satz::Value::Ref(p) => env.get(p).filter(|v| !is_empty(v)).cloned(),
        _ => None,
    }
}

fn is_empty(v: &serde_yaml::Value) -> bool {
    match v {
        serde_yaml::Value::Null => true,
        serde_yaml::Value::String(s) => s.trim().is_empty(),
        serde_yaml::Value::Sequence(s) => s.is_empty(),
        _ => false,
    }
}

fn truthy(v: Option<&serde_yaml::Value>) -> bool {
    matches!(v, Some(serde_yaml::Value::Bool(true)))
}

/// Render for a terminal. Takes the report and nothing else.
pub(crate) fn render_questions(r: &QuestionsReport) -> String {
    let mut out = format!("\nquestions — {}\n\n", r.estate);
    if r.questions.is_empty() {
        // `--unanswered` filters the rows and keeps the summary: an empty list
        // after the filter means everything is answered, not that nothing asks.
        if r.summary.total + r.summary.not_applicable > 0 {
            out.push_str(&format!(
                "  0 unanswered of {} ({} answered, {} not applicable).\n",
                r.summary.total, r.summary.answered, r.summary.not_applicable
            ));
        } else {
            out.push_str("  none: no pack this estate uses declares a question.\n");
        }
        return out;
    }
    for q in &r.questions {
        let mark = match q.state {
            "answered" => "✓",
            "not-applicable" => "–",
            _ if q.blocking => "!",
            _ => "?",
        };
        // A one-way door is the whole reason this data exists: cheap to answer
        // now, expensive to have answered wrongly.
        let door = if q.reversal == "recreate" || q.blast == "high" { "  ⚠ one-way" } else { "" };
        out.push_str(&format!("  {} {:32} {}{}\n", mark, q.subject, q.prompt, door));
        out.push_str(&format!(
            "      reversal {:14} blast {:6} {}\n",
            q.reversal,
            q.blast,
            match (&q.current, &q.default) {
                (Some(v), _) => format!("answer: {}", crate::questions::short(v)),
                (None, Some(d)) => format!("default offered: {}", crate::questions::short(d)),
                (None, None) if q.state == "unanswered" => "no default — a value is needed".to_string(),
                _ => String::new(),
            }
        ));
        for o in &q.options {
            out.push_str(&format!(
                "        {} {:28} {}\n",
                if o.selected { "•" } else { " " },
                o.param,
                o.label
            ));
        }
        if let Some(w) = &q.why {
            out.push_str(&format!("      {}\n", w));
        }
    }
    let s = &r.summary;
    out.push_str(&format!(
        "\n{} question(s): {} answered, {} unanswered ({} need a value); {} expensive to change later.\n",
        s.total, s.answered, s.unanswered, s.blocking, s.one_way_doors
    ));
    out.push_str(if s.complete {
        "complete — every question is answered; bootstrap and apply are open.\n"
    } else {
        "NOT complete — bootstrap and apply refuse until every question is answered.\n"
    });
    out
}

/// The catalog as a workbook — the format a customer can fill in and send back.
///
/// One row per question, grouped by pack in the order the interview asks them. The
/// "your answer" column is the customer's to edit; "needs an answer" says which rows are
/// still waiting. `why` and the cost of a later change travel with each row, because a
/// decision read six months later without its reason is not a decision anybody can defend.
pub(crate) fn xlsx(r: &QuestionsReport) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Format, FormatAlign, Workbook};

    const COLUMNS: [&str; 8] =
        ["pack", "decision", "your answer", "needs an answer", "how it was set", "why it is asked", "changing it later", "param"];

    let mut wb = Workbook::new();
    let header = Format::new().set_bold().set_background_color("#D9D9D9").set_align(FormatAlign::Center);
    let theirs = Format::new().set_background_color("#FFF7E6");
    let wrap = Format::new().set_text_wrap();

    let ws = wb.add_worksheet().set_name("Decisions").map_err(|e| e.to_string())?;
    for (c, name) in COLUMNS.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *name, &header).map_err(|e| e.to_string())?;
    }
    for (i, q) in r.questions.iter().enumerate() {
        let row = i as u32 + 1;
        let answer = match (&q.current, &q.default) {
            (Some(v), _) => short(v),
            (None, Some(d)) => short(d),
            _ => String::new(),
        };
        let cells = [
            q.pack.clone(),
            q.prompt.clone(),
            answer,
            if q.state == "answered" { "no".into() } else if q.blocking { "yes — and it has no default".into() } else { "yes".into() },
            how_answered(q).to_string(),
            q.why.clone().unwrap_or_default(),
            cost_in_words(q),
            q.subject.clone(),
        ];
        for (c, cell) in cells.iter().enumerate() {
            let f = match c {
                2 => Some(&theirs),
                1 | 5 | 6 => Some(&wrap),
                _ => None,
            };
            match f {
                Some(f) => ws.write_string_with_format(row, c as u16, cell, f).map_err(|e| e.to_string())?,
                None => ws.write_string(row, c as u16, cell).map_err(|e| e.to_string())?,
            };
        }
    }
    let n = r.questions.len() as u32;
    if n > 0 {
        ws.autofilter(0, 0, n, (COLUMNS.len() - 1) as u16).map_err(|e| e.to_string())?;
    }
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    for (c, w) in [(0u16, 28.0), (1, 52.0), (2, 28.0), (3, 22.0), (4, 24.0), (5, 64.0), (6, 44.0), (7, 30.0)] {
        ws.set_column_width(c, w).map_err(|e| e.to_string())?;
    }
    wb.save_to_buffer().map_err(|e| e.to_string())
}

/// Whether a bound answer was CHOSEN or merely took what the pack offered.
///
/// satz cannot read intent — a customer may pick the default deliberately — so this says
/// what it can prove: the value differs from the pack's default, or it does not. That is
/// the useful half anyway. It is where a customer decided something for themselves.
pub(crate) fn how_answered(q: &QuestionRow) -> &'static str {
    match (q.state, &q.current, &q.default) {
        ("answered", Some(c), Some(d)) if c == d => "same as the pack's default",
        ("answered", Some(_), _) => "chosen for this estate",
        ("answered", None, _) => "answered",
        ("not-applicable", _, _) => "not applicable here",
        _ => "still open",
    }
}

/// What changing this answer later costs, as a sentence rather than two enum names.
pub(crate) fn cost_in_words(q: &QuestionRow) -> String {
    let what = match q.reversal {
        "recreate" => "the resource is destroyed and made again",
        "state_surgery" => "the estate's state has to be edited by hand",
        _ => "an edit to the estate",
    };
    let who = match q.blast {
        "high" => "and the running organisation feels it",
        "low" => "with little effect on what is running",
        _ => "and nothing running notices",
    };
    format!("{} {}", what, who)
}

/// The decisions sheet — "these are your decisions, shall we start?" — and the catalog a
/// customer keeps afterwards. Grouped by pack, each pack introduced by its own
/// description, each question with the answer the estate carries or the default it would
/// accept, WHY it is asked at all, and what changing it later costs. Markdown, because it
/// is handed over.
pub(crate) fn render_decisions(r: &QuestionsReport) -> String {
    let mut out = format!(
        "# Decisions — {}\n\nEvery decision this estate rests on: what was asked, what it is set to, \
         whether that was chosen or taken as offered, and what changing it later costs. The italic \
         line under each is why the question exists at all.\n\n",
        r.estate
    );
    let s = &r.summary;
    if s.complete {
        out.push_str(&format!(
            "**All {} questions are answered.** Nothing below is undecided; this estate may be bootstrapped.\n\n",
            s.total
        ));
    } else {
        out.push_str(&format!(
            "**{} of {} questions are still open, {} of them without a default to accept.** Bootstrap and \
             apply refuse until every one is answered.\n\n",
            s.unanswered, s.total, s.blocking
        ));
    }
    let mut packs: Vec<&str> = r.questions.iter().map(|q| q.pack.as_str()).collect();
    packs.dedup();
    for pack in packs {
        let rows: Vec<&QuestionRow> = r.questions.iter().filter(|q| q.pack == pack).collect();
        out.push_str(&format!("## {}\n\n", pack));
        if let Some(d) = rows.first().map(|q| q.pack_description.as_str()).filter(|d| !d.is_empty()) {
            out.push_str(&format!("{}\n\n", d));
        }
        out.push_str("| | decision | your answer | how | changing it later |\n|---|---|---|---|---|\n");
        for q in rows {
            let mark = match q.state {
                "answered" => "✓",
                "not-applicable" => "–",
                _ if q.blocking => "**!**",
                _ => "?",
            };
            let answer = match (q.state, &q.current, &q.default) {
                ("not-applicable", _, _) => "not applicable".to_string(),
                (_, Some(v), _) => format!("`{}`", short(v)),
                (_, None, Some(d)) if q.kind == "oneof" => {
                    let label = q
                        .options
                        .iter()
                        .find(|o| d.as_str() == Some(o.param.as_str()))
                        .map(|o| o.label.clone())
                        .unwrap_or_else(|| short(d));
                    format!("default: {} — accept, or choose another", label)
                }
                (_, None, Some(d)) => format!("default `{}` — accept, or change", short(d)),
                _ if q.kind == "oneof" => {
                    let opts: Vec<String> = q.options.iter().map(|o| o.label.clone()).collect();
                    format!("**choose:** {}", opts.join(" / "))
                }
                _ => "**needs a value**".to_string(),
            };
            let cost = cost_in_words(q);
            out.push_str(&format!("| {} | {} | {} | {} | {} |\n", mark, q.prompt, answer, how_answered(q), cost));
            if let Some(why) = q.why.as_deref().filter(|w| !w.is_empty()) {
                out.push_str(&format!("| | *{}* | | | |\n", why.replace('|', "\\|")));
            }
        }
        out.push('\n');
    }
    out
}

/// A value as a human reads it: a string is itself (YAML would quote `"123456789012"`
/// to keep it a string, which is not a distinction a customer is deciding on).
pub(crate) fn short(v: &serde_yaml::Value) -> String {
    let s = match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().trim_start_matches("- ").to_string(),
    };
    if s.chars().count() > 60 { format!("{}…", s.chars().take(57).collect::<String>()) } else { s }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    /// The rule that made `init --interview` ask nothing: an estate that binds an
    /// identity param to `""` has restated the pack's own "no default possible", and
    /// the question is still open — the day-0 gate refuses the same value by name.
    /// A param whose pack default is NOT empty is different: `""` there is a choice.
    /// Found on a fresh estate: the shortname was answered in the interview, after
    /// init had written `""` for the project and bucket derived from it, and neither
    /// was asked. A derived param bound to nothing is open, and the derivation is the
    /// offer once its input is answered.
    #[test]
    fn a_derived_param_bound_to_nothing_is_offered_its_derivation() {
        let declared = PackFacts {
            description: String::new(),
            params: BTreeMap::from([(
                "infra_project_name".to_string(),
                satz_core::satz::Value::Str(vec![
                    satz_core::satz::StrPart::Param("customer_shortname".into()),
                    satz_core::satz::StrPart::Lit("-infra-001".into()),
                ]),
            )]),
        };
        let mut env: Env = BTreeMap::from([
            ("infra_project_name".to_string(), serde_yaml::Value::String(String::new())),
            ("customer_shortname".to_string(), serde_yaml::Value::String(String::new())),
        ]);
        assert!(restates_unknown("infra_project_name", Some(&declared), &env), "an empty project id is no answer");
        // the input is not answered yet: nothing to offer, so the question blocks
        assert_eq!(derived_default("infra_project_name", Some(&declared), &env), None);
        // once it is, the derivation is the offer
        env.insert("customer_shortname".into(), serde_yaml::Value::String("stec".into()));
        assert_eq!(
            derived_default("infra_project_name", Some(&declared), &env),
            Some(serde_yaml::Value::String("stec-infra-001".into()))
        );
    }

    #[test]
    fn an_empty_binding_of_a_param_nobody_can_default_is_still_open() {
        let str_lit = |s: &str| satz_core::satz::Value::Str(vec![satz_core::satz::StrPart::Lit(s.to_string())]);
        let declared = PackFacts {
            description: String::new(),
            params: BTreeMap::from([
                ("customer_organization_id".to_string(), str_lit("")),
                ("infra_folder_name".to_string(), str_lit("Infrastructure")),
            ]),
        };
        let env: Env = BTreeMap::from([
            ("customer_organization_id".to_string(), serde_yaml::Value::String(String::new())),
            ("infra_folder_name".to_string(), serde_yaml::Value::String(String::new())),
            ("customer_domain".to_string(), serde_yaml::Value::String("example.com".into())),
        ]);
        // "" for an undefaultable param: the pack's unknown, restated — open
        assert!(restates_unknown("customer_organization_id", Some(&declared), &env));
        // no derivation to offer for it
        assert_eq!(derived_default("customer_organization_id", Some(&declared), &env), None);
        // "" where the pack offers a default: a real choice (no folder) — answered
        assert!(!restates_unknown("infra_folder_name", Some(&declared), &env));
        // a value is a value
        assert!(!restates_unknown("customer_domain", Some(&declared), &env));
    }

    #[test]
    fn the_catalog_workbook_carries_the_reason_and_marks_what_is_open() {
        let mut r = report(QuestionsSummary { total: 2, answered: 1, unanswered: 1, blocking: 1, ..Default::default() });
        r.questions = vec![
            QuestionRow {
                subject: "chosen_one".into(),
                kind: "param",
                prompt: "Which region?".into(),
                why: Some("Regional resources cannot move.".into()),
                reversal: "recreate",
                blast: "high",
                state: "answered",
                current: Some(serde_yaml::Value::String("europe-west4".into())),
                default: Some(serde_yaml::Value::String("europe-west3".into())),
                blocking: false,
                pack: "estate_core".into(),
                pack_description: "day 0".into(),
                ..Default::default()
            },
            QuestionRow {
                subject: "still_open".into(),
                kind: "param",
                prompt: "Which mailbox?".into(),
                why: Some("Nobody reads a wrong address.".into()),
                reversal: "edit",
                blast: "low",
                state: "unanswered",
                current: None,
                default: None,
                blocking: true,
                pack: "alerts".into(),
                pack_description: "alerting".into(),
                ..Default::default()
            },
        ];

        // an answer that differs from the pack's default is where a customer decided something
        assert_eq!(how_answered(&r.questions[0]), "chosen for this estate");
        assert_eq!(how_answered(&r.questions[1]), "still open");
        assert!(cost_in_words(&r.questions[0]).contains("destroyed and made again"));

        let bytes = xlsx(&r).expect("the workbook builds");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("xlsx is a zip");
        let mut strings = String::new();
        {
            use std::io::Read;
            let mut f = zip.by_name("xl/sharedStrings.xml").expect("shared strings");
            f.read_to_string(&mut strings).unwrap();
        }
        assert!(strings.contains("why it is asked"), "the reason is a column, not a footnote");
        assert!(strings.contains("Regional resources cannot move."), "each row carries its own why");
        assert!(strings.contains("yes — and it has no default"), "a question with no default is marked as needing one");
        assert!(strings.contains("chosen for this estate"));
    }

    fn report(summary: QuestionsSummary) -> QuestionsReport {
        QuestionsReport { estate: "e.satz".into(), questions: Vec::new(), summary }
    }

    #[test]
    fn an_empty_filtered_list_says_all_answered_not_that_nothing_asks() {
        let all_answered = render_questions(&report(QuestionsSummary {
            total: 5,
            answered: 5,
            not_applicable: 1,
            complete: true,
            ..Default::default()
        }));
        assert!(all_answered.contains("0 unanswered of 5 (5 answered, 1 not applicable)"), "{all_answered}");
        let nothing_asks = render_questions(&report(QuestionsSummary::default()));
        assert!(nothing_asks.contains("no pack this estate uses declares a question"), "{nothing_asks}");
    }
}
