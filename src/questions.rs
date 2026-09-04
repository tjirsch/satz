//! `satz questions` — what this estate can be asked, and what the answers cost.
//!
//! A pack declares its params and its claims; it also declares what to ask a
//! customer so those params can be filled. This command joins the two: every
//! question the estate's packs contribute, against the value the estate already
//! carries for it.
//!
//! Read-only, schema-free, offline. An interview happens before anyone has run
//! `update-schema`, so nothing here needs the provider registry.

use std::path::Path;

use satz_core::pipeline::estate_questions;

use crate::ToolConfig;

/// One question, joined with the answer the estate currently carries.
#[derive(Debug, Clone, serde::Serialize)]
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
    /// answered | defaulted | unasked
    ///
    /// `defaulted` is not a state the file records yet — it is what "the estate
    /// carries a value and nobody was asked" looks like from here. Telling those
    /// two apart is the lockfile's job, and the lockfile is a later phase.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<serde_yaml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommend: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionRow>,
    /// the file that declared it — a fork asks its own questions
    pub from: String,
    pub pack: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct OptionRow {
    pub param: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct QuestionsReport {
    pub estate: String,
    pub questions: Vec<QuestionRow>,
    pub summary: QuestionsSummary,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct QuestionsSummary {
    pub total: usize,
    pub answered: usize,
    pub unasked: usize,
    /// questions whose answer is expensive to change — recreate, or high blast
    pub one_way_doors: usize,
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

    let mut rows = Vec::new();
    let mut summary = QuestionsSummary::default();
    for pq in &packs {
        for q in &pq.questions {
            let current = if q.oneof { None } else { env.get(&q.subject).cloned() };
            // "unasked" is the honest word: the estate carries whatever the pack
            // defaulted to, and nobody recorded a decision either way.
            let answered = current.as_ref().map(|v| !is_empty(v)).unwrap_or(false);
            let state = if q.oneof {
                if q.options.iter().any(|o| truthy(env.get(&o.param))) { "answered" } else { "unasked" }
            } else if answered {
                "defaulted"
            } else {
                "unasked"
            };
            let one_way = q.reversal == satz_core::satz::Reversal::Recreate
                || q.blast == satz_core::satz::Blast::High;
            summary.total += 1;
            if state == "unasked" {
                summary.unasked += 1;
            } else {
                summary.answered += 1;
            }
            if one_way {
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

    Ok(QuestionsReport { estate: file_name, questions: rows, summary })
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
        out.push_str("  none: no pack this estate uses declares a question yet.\n");
        return out;
    }
    for q in &r.questions {
        let mark = match q.state {
            "answered" => "✓",
            "defaulted" => "·",
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
            match &q.current {
                Some(v) => format!("now: {}", crate::questions::short(v)),
                None => String::new(),
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
        "\n{} question(s): {} answered, {} unasked; {} expensive to change later.\n",
        s.total, s.answered, s.unasked, s.one_way_doors
    ));
    out
}

pub(crate) fn short(v: &serde_yaml::Value) -> String {
    let s = serde_yaml::to_string(v).unwrap_or_default();
    let s = s.trim().trim_start_matches("- ").to_string();
    if s.len() > 60 { format!("{}…", &s[..57]) } else { s }
}
