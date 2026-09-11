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

use crate::ToolConfig;

/// One question, joined with the answer the estate currently carries.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
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
    /// The pack's own description — its header's first paragraph — so an
    /// interview can say what a pack is FOR before asking about its params.
    pub pack_description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommend: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
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

/// Read the estate and its packs, and join every question with its current value.
pub(crate) fn questions_report(
    input: &Path,
    runtime: &ToolConfig,
) -> Result<QuestionsReport, Box<dyn std::error::Error>> {
    let src = crate::fsx::read_to_string(input)?;
    let base = input.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let include_dirs = runtime.include_dirs.clone();
    let load = move |p: &str| -> Result<String, String> {
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
    };

    let file_name = input.display().to_string();
    let (packs, env) = estate_questions(&file_name, &src, &load)
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
            } else if own.contains(&q.subject) {
                ("answered", env.get(&q.subject).cloned(), None, false)
            } else {
                let usable = match declared {
                    Some(f) => default_usable(&q.subject, &f.params, &own, &env, 0),
                    None => env.get(&q.subject).map(|v| !is_empty(v)).unwrap_or(false),
                };
                let default = if usable { env.get(&q.subject).cloned() } else { None };
                ("unanswered", None, default, !usable)
            };
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
                pack_description: declared.map(|f| f.description.clone()).unwrap_or_default(),
                recommend: q.recommend.as_ref().map(|v| format!("{:?}", v)),
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
         estate touches an organisation. `satz questions {} --unanswered` lists them with their \
         defaults; write the answer (or the default) into the estate's params.",
        action,
        open.len(),
        names.join(", "),
        input.display()
    ))
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

/// The decisions sheet — "these are your decisions, shall we start?" — for a
/// human to read before an organisation is touched. Grouped by pack, each pack
/// introduced by its own description, each question with the answer the estate
/// carries or the default it would accept. Markdown, because it is handed over.
pub(crate) fn render_decisions(r: &QuestionsReport) -> String {
    let mut out = format!("# Decisions — {}\n\n", r.estate);
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
        out.push_str("| | decision | your answer | changing it later |\n|---|---|---|---|\n");
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
            let cost = format!("{} · blast {}", q.reversal.replace('_', " "), q.blast);
            out.push_str(&format!("| {} | {} | {} | {} |\n", mark, q.prompt, answer, cost));
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
