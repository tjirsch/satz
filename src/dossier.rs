//! The remediation dossier: everything about a set of findings that a
//! machine can settle, settled — so that what is left for judgment (grouping
//! into workstreams, priorities in this customer's words, effort, risk
//! acceptance) starts from a triaged, deduplicated, counted picture and not
//! from two raw scanner exports.
//!
//! Pure over the compliance plane's own products: `triage` rows (Prowler
//! FAIL/MANUAL per control, bucketed A–E against the estate's claims),
//! Checkov findings (pointed at the Satz block that declared the resource),
//! the catalog, and the goal view. One item per (control, resource); a
//! resource both scanners flag carries both sources. Deterministic: the same
//! inputs give the same JSON, byte for byte, and its hash names the run.
//!
//! The dossier is written under the estate's `evidence/` directory (git-
//! ignored, gate-rejected) as JSON, CSV and XLSX. The XLSX carries the
//! mechanical columns filled and the `[AI]` columns — the ones a model or a
//! consultant authors — empty, beside a Review column: the findings workbook
//! minus the prose.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::compliance::{Bucket, Catalog, Goal, TriageRow};

/// One finding as the dossier sees it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Item {
    /// Stable id `F-0001`, in dossier order (bucket, severity rank, control, resource).
    pub id: String,
    pub bucket: Bucket,
    pub bucket_title: String,
    /// Prowler severity normalized: Critical / High / Medium / Low / Informational / (empty)
    pub severity: String,
    pub control: String,
    pub control_title: String,
    /// The catalog's own reading of the control.
    pub paraphrase: String,
    pub resource: String,
    pub project: String,
    /// The emitted Satz address that declares this resource, when matched.
    pub declared_address: Option<String>,
    /// `file:line` of the declaring block, when known.
    pub declared_at: Option<String>,
    /// Which scanners flagged it.
    pub sources: Vec<Source>,
    /// Prowler's title / remediation / risk for the check (first source that has them).
    pub scanner_title: String,
    pub scanner_remediation: String,
    pub scanner_risk: String,
    /// The triage plan sentence — the mechanical recommendation.
    pub plan: String,
    /// Packs in the library that would cover the control (bucket A).
    pub providers: Vec<String>,
    /// The estate's declared deviation reasons (bucket C).
    pub deviation_reasons: Vec<String>,
    /// Duties still open on the control.
    pub open_duties: Vec<String>,
    /// What the estate's goal view says about the control.
    pub goal: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "scanner", rename_all = "lowercase")]
pub(crate) enum Source {
    Prowler { check: String, status: String },
    Checkov { check_id: String, check_name: String, guideline: Option<String> },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Dossier {
    pub framework: String,
    pub framework_version: String,
    pub estate: String,
    /// Counts the plan's narrative starts from.
    pub summary: Summary,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub(crate) struct Summary {
    pub items: usize,
    pub by_bucket: BTreeMap<String, usize>,
    pub by_severity: BTreeMap<String, usize>,
    pub by_control: BTreeMap<String, usize>,
    pub by_project: BTreeMap<String, usize>,
    /// Items both scanners flagged.
    pub corroborated: usize,
    /// Bucket-B items the estate declares: `apply` fixes them, no edit needed.
    pub declared_apply_fixes: usize,
}

/// Prowler's severities in rank order (worst first), then anything else.
fn severity_rank(s: &str) -> u8 {
    match s.to_ascii_lowercase().as_str() {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        "informational" | "info" => 4,
        _ => 5,
    }
}

fn normalize_severity(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "critical" => "Critical",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "informational" | "info" => "Informational",
        "" => "",
        _ => return s.to_string(),
    }
    .to_string()
}

/// What the dossier is built from.
pub(crate) struct Inputs<'a> {
    /// The catalog id as the command names it (`cis-gcp-4.0`).
    pub framework: &'a str,
    pub catalog: &'a Catalog,
    pub goals: &'a BTreeMap<String, Goal>,
    pub estate: &'a str,
    pub triage_rows: &'a [TriageRow],
    /// Prowler's title / remediation / risk per check id.
    pub prowler_text: &'a BTreeMap<String, (String, String, String)>,
    /// Checkov findings with their Terraform address; they join Prowler rows
    /// on the DECLARED address (the only identity both sides share).
    pub checkov: &'a [crate::scan::Finding],
    /// `file:line` per emitted address.
    pub declared_at: &'a BTreeMap<String, String>,
}

/// Build the dossier.
pub(crate) fn build(inp: &Inputs<'_>) -> Dossier {
    let Inputs { framework, catalog, goals, estate, triage_rows, prowler_text, checkov, declared_at } = *inp;
    let mut items: Vec<Item> = Vec::new();

    for r in triage_rows {
        let (goal_text, providers, deviation_reasons, open_duties) = describe_goal(goals.get(&r.control));
        let (title, remediation, risk) = prowler_text.get(&r.check).cloned().unwrap_or_default();
        let mut item = Item {
            id: String::new(),
            bucket: r.bucket,
            bucket_title: r.bucket.title().to_string(),
            severity: normalize_severity(&r.severity),
            control: r.control.clone(),
            control_title: r.title.clone(),
            paraphrase: catalog.controls.get(&r.control).map(|c| c.paraphrase.clone()).unwrap_or_default(),
            resource: r.resource.clone(),
            project: r.project.clone(),
            declared_address: r.declared.clone(),
            declared_at: r.declared.as_ref().and_then(|a| declared_at.get(a).cloned()),
            sources: vec![Source::Prowler { check: r.check.clone(), status: r.status.clone() }],
            scanner_title: title,
            scanner_remediation: remediation,
            scanner_risk: risk,
            plan: r.plan.clone(),
            providers,
            deviation_reasons,
            open_duties,
            goal: goal_text,
        };
        // A Checkov finding on the same declared address corroborates it.
        if let Some(addr) = &r.declared {
            for c in checkov.iter().filter(|c| &c.resource == addr) {
                item.sources.push(Source::Checkov {
                    check_id: c.check_id.clone(),
                    check_name: c.check_name.clone(),
                    guideline: c.guideline.clone(),
                });
            }
        }
        items.push(item);
    }

    // Checkov findings on addresses no Prowler row matched: their own items.
    // Checkov judges the PLAN, so these are declared resources with a
    // misconfiguration the estate itself carries — bucket D would lie (they
    // ARE managed); they read as B: declared, and the declaration is what
    // needs the fix.
    let matched: BTreeSet<&String> = items.iter().filter_map(|i| i.declared_address.as_ref()).collect();
    let mut by_addr: BTreeMap<&String, Vec<&crate::scan::Finding>> = BTreeMap::new();
    for c in checkov {
        if !matched.contains(&c.resource) {
            by_addr.entry(&c.resource).or_default().push(c);
        }
    }
    for (addr, cs) in by_addr {
        items.push(Item {
            id: String::new(),
            bucket: Bucket::B,
            bucket_title: Bucket::B.title().to_string(),
            severity: String::new(),
            control: String::new(),
            control_title: String::new(),
            paraphrase: String::new(),
            resource: addr.clone(),
            project: String::new(),
            declared_address: Some(addr.clone()),
            declared_at: declared_at.get(addr).cloned(),
            sources: cs
                .iter()
                .map(|c| Source::Checkov { check_id: c.check_id.clone(), check_name: c.check_name.clone(), guideline: c.guideline.clone() })
                .collect(),
            scanner_title: cs.first().map(|c| c.check_name.clone()).unwrap_or_default(),
            scanner_remediation: String::new(),
            scanner_risk: String::new(),
            plan: "the estate declares this resource with a setting Checkov flags: change the declaration, then apply".to_string(),
            providers: vec![],
            deviation_reasons: vec![],
            open_duties: vec![],
            goal: "declared by the estate (Checkov, plan-time)".to_string(),
        });
    }

    items.sort_by(|a, b| {
        (a.bucket, severity_rank(&a.severity), &a.control, &a.resource).cmp(&(b.bucket, severity_rank(&b.severity), &b.control, &b.resource))
    });
    for (i, item) in items.iter_mut().enumerate() {
        item.id = format!("F-{:04}", i + 1);
    }

    let mut summary = Summary { items: items.len(), ..Default::default() };
    for it in &items {
        *summary.by_bucket.entry(format!("{:?}", it.bucket)).or_default() += 1;
        *summary.by_severity.entry(if it.severity.is_empty() { "(none)".into() } else { it.severity.clone() }).or_default() += 1;
        if !it.control.is_empty() {
            *summary.by_control.entry(it.control.clone()).or_default() += 1;
        }
        if !it.project.is_empty() {
            *summary.by_project.entry(it.project.clone()).or_default() += 1;
        }
        if it.sources.len() > 1 {
            summary.corroborated += 1;
        }
        if it.bucket == Bucket::B && it.declared_address.is_some() && matches!(it.sources.first(), Some(Source::Prowler { .. })) {
            summary.declared_apply_fixes += 1;
        }
    }

    Dossier {
        framework: catalog.catalog.clone(),
        framework_version: catalog.version.clone(),
        estate: estate.to_string(),
        summary,
        items,
    }
    .with_framework_id(framework)
}

impl Dossier {
    fn with_framework_id(mut self, id: &str) -> Self {
        // the catalog id as the command names it (`cis-gcp-4.0`) is what a
        // reader greps for; keep the split fields too
        if !id.is_empty() {
            self.framework = id.to_string();
        }
        self
    }

    /// Canonical JSON: the artifact, and the hash that names the run.
    pub(crate) fn json(&self) -> String {
        serde_json::to_string_pretty(self).expect("dossier serializes")
    }

    pub(crate) fn hash(&self) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(self.json().as_bytes()))
    }
}

fn describe_goal(goal: Option<&Goal>) -> (String, Vec<String>, Vec<String>, Vec<String>) {
    match goal {
        Some(Goal::Satisfied { witnesses }) => (format!("satisfied by {}", witnesses.join(", ")), vec![], vec![], vec![]),
        Some(Goal::Partial { witnesses, open_duties, contributes_only }) => (
            format!("partial{} — witnesses {}", if *contributes_only { " (contributes only)" } else { "" }, witnesses.join(", ")),
            vec![],
            vec![],
            open_duties.clone(),
        ),
        Some(Goal::ClaimBroken { missing, pack }) => (format!("CLAIM BROKEN: {} declares witnesses not emitted: {}", pack, missing.join(", ")), vec![], vec![], vec![]),
        Some(Goal::Deviation { reasons, open_duties, .. }) => (
            "declared deviation".to_string(),
            vec![],
            reasons.iter().map(|(_, r)| r.clone()).collect(),
            open_duties.clone(),
        ),
        Some(Goal::Unmet { providers }) => ("unmet".to_string(), providers.clone(), vec![], vec![]),
        Some(Goal::Organizational) => ("organizational — no IaC witness".to_string(), vec![], vec![], vec![]),
        Some(Goal::Inherited) => ("inherited from the provider — shared responsibility".to_string(), vec![], vec![], vec![]),
        None => ("not in the goal view".to_string(), vec![], vec![], vec![]),
    }
}

// ---------------------------------------------------------------------------
// Renderings: CSV (diffable) and XLSX (the workbook)
// ---------------------------------------------------------------------------

/// Column order shared by CSV and XLSX. `[AI]` columns are authored, empty here.
pub(crate) const COLUMNS: &[&str] = &[
    "Id",
    "Severity",
    "Bucket",
    "Control",
    "Control title",
    "Paraphrase",
    "Resource",
    "Project",
    "Declared address",
    "Declared at",
    "Sources",
    "Scanner title",
    "Scanner remediation",
    "Scanner risk",
    "Plan (mechanical)",
    "Providers",
    "Deviation reasons",
    "Open duties",
    "Goal",
    "[AI] What / why",
    "[AI] Recommended fix",
    "[AI] Owner",
    "[AI] Effort",
    "[AI] Phase",
    "[AI] Quick win",
    "[AI] Risk-acceptance candidate",
    "Review",
    "Reviewer",
    "Reviewed on",
    "Note",
];

/// The first index of an `[AI]` column.
const AI_FROM: usize = 19;
/// The index of the Review column.
const REVIEW_AT: usize = 26;

fn row_of(it: &Item) -> Vec<String> {
    let sources = it
        .sources
        .iter()
        .map(|s| match s {
            Source::Prowler { check, status } => format!("prowler:{} ({})", check, status),
            Source::Checkov { check_id, .. } => format!("checkov:{}", check_id),
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut row = vec![
        it.id.clone(),
        it.severity.clone(),
        format!("{:?}", it.bucket),
        it.control.clone(),
        it.control_title.clone(),
        it.paraphrase.clone(),
        it.resource.clone(),
        it.project.clone(),
        it.declared_address.clone().unwrap_or_default(),
        it.declared_at.clone().unwrap_or_default(),
        sources,
        it.scanner_title.clone(),
        it.scanner_remediation.clone(),
        it.scanner_risk.clone(),
        it.plan.clone(),
        it.providers.join("; "),
        it.deviation_reasons.join("; "),
        it.open_duties.join("; "),
        it.goal.clone(),
    ];
    row.resize(COLUMNS.len(), String::new());
    row
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub(crate) fn csv(d: &Dossier) -> String {
    let mut out = COLUMNS.join(",") + "\n";
    for it in &d.items {
        out.push_str(&row_of(it).iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// The workbook: Findings (mechanical columns filled, `[AI]` columns tinted
/// and empty, Review dropdown), Summary sheets with live formulas over
/// Findings, Provenance.
pub(crate) fn xlsx(d: &Dossier, provenance: &[(String, String)]) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{DataValidation, Format, FormatAlign, Workbook};

    let mut wb = Workbook::new();
    let header = Format::new().set_bold().set_background_color("#D9D9D9").set_align(FormatAlign::Center);
    let ai_header = Format::new().set_bold().set_background_color("#DDEBF7").set_align(FormatAlign::Center);
    let ai_cell = Format::new().set_background_color("#F2F8FD");
    let wrap = Format::new().set_text_wrap();
    let n = d.items.len() as u32;

    // --- Findings -----------------------------------------------------------
    let ws = wb.add_worksheet().set_name("Findings").map_err(|e| e.to_string())?;
    for (c, name) in COLUMNS.iter().enumerate() {
        let f = if (AI_FROM..REVIEW_AT).contains(&c) { &ai_header } else { &header };
        ws.write_string_with_format(0, c as u16, *name, f).map_err(|e| e.to_string())?;
    }
    for (r, it) in d.items.iter().enumerate() {
        for (c, cell) in row_of(it).iter().enumerate() {
            let f = if (AI_FROM..REVIEW_AT).contains(&c) { Some(&ai_cell) } else if c == 5 || c == 12 || c == 14 { Some(&wrap) } else { None };
            match f {
                Some(f) => ws.write_string_with_format(r as u32 + 1, c as u16, cell, f).map_err(|e| e.to_string())?,
                None => ws.write_string(r as u32 + 1, c as u16, cell).map_err(|e| e.to_string())?,
            };
        }
    }
    if n > 0 {
        let review = DataValidation::new().allow_list_strings(&["open", "accepted", "edited", "rejected"]).map_err(|e| e.to_string())?;
        ws.add_data_validation(1, REVIEW_AT as u16, n, REVIEW_AT as u16, &review).map_err(|e| e.to_string())?;
        ws.autofilter(0, 0, n, (COLUMNS.len() - 1) as u16).map_err(|e| e.to_string())?;
    }
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    for (c, w) in [(0, 8), (1, 10), (2, 8), (3, 8), (4, 28), (5, 40), (6, 40), (7, 18), (8, 32), (9, 24), (10, 26), (11, 32), (12, 48), (13, 36), (14, 48), (15, 24), (16, 30), (17, 24), (18, 28)] {
        ws.set_column_width(c, w).map_err(|e| e.to_string())?;
    }
    for c in AI_FROM..COLUMNS.len() {
        ws.set_column_width(c as u16, 22).map_err(|e| e.to_string())?;
    }

    // --- Summaries with live formulas over Findings -------------------------
    let last = n + 1;
    let ws = wb.add_worksheet().set_name("By control").map_err(|e| e.to_string())?;
    for (c, name) in ["Control", "Title", "Findings", "Reviewed (accepted/edited)"].iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *name, &header).map_err(|e| e.to_string())?;
    }
    for (r, (control, _)) in d.summary.by_control.iter().enumerate() {
        let row = r as u32 + 1;
        let title = d.items.iter().find(|i| &i.control == control).map(|i| i.control_title.clone()).unwrap_or_default();
        ws.write_string(row, 0, control).map_err(|e| e.to_string())?;
        ws.write_string(row, 1, &title).map_err(|e| e.to_string())?;
        ws.write_formula(row, 2, format!("=COUNTIF(Findings!$D$2:$D${last},A{})", row + 1).as_str()).map_err(|e| e.to_string())?;
        ws.write_formula(
            row,
            3,
            format!("=COUNTIFS(Findings!$D$2:$D${last},A{},Findings!$AA$2:$AA${last},\"accepted\")+COUNTIFS(Findings!$D$2:$D${last},A{},Findings!$AA$2:$AA${last},\"edited\")", row + 1, row + 1).as_str(),
        )
        .map_err(|e| e.to_string())?;
    }
    ws.set_column_width(1, 36).map_err(|e| e.to_string())?;

    let ws = wb.add_worksheet().set_name("By bucket").map_err(|e| e.to_string())?;
    for (c, name) in ["Bucket", "Meaning", "Findings"].iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *name, &header).map_err(|e| e.to_string())?;
    }
    for (r, b) in [Bucket::A, Bucket::B, Bucket::C, Bucket::D, Bucket::E].iter().enumerate() {
        let row = r as u32 + 1;
        ws.write_string(row, 0, format!("{:?}", b)).map_err(|e| e.to_string())?;
        ws.write_string(row, 1, b.title()).map_err(|e| e.to_string())?;
        ws.write_formula(row, 2, format!("=COUNTIF(Findings!$C$2:$C${last},A{})", row + 1).as_str()).map_err(|e| e.to_string())?;
    }
    ws.set_column_width(1, 60).map_err(|e| e.to_string())?;

    let ws = wb.add_worksheet().set_name("Plan phases").map_err(|e| e.to_string())?;
    for (c, name) in ["Phase", "Workstream", "Goal", "Items (ids)", "Dependencies", "Owner", "Effort", "Notes"].iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *name, &ai_header).map_err(|e| e.to_string())?;
    }

    let ws = wb.add_worksheet().set_name("Provenance").map_err(|e| e.to_string())?;
    for (r, (k, v)) in provenance.iter().enumerate() {
        ws.write_string(r as u32, 0, k).map_err(|e| e.to_string())?;
        ws.write_string(r as u32, 1, v).map_err(|e| e.to_string())?;
    }
    ws.set_column_width(0, 22).map_err(|e| e.to_string())?;
    ws.set_column_width(1, 72).map_err(|e| e.to_string())?;

    wb.save_to_buffer().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::{Control, TriageRow};

    fn catalog() -> Catalog {
        let mut controls = BTreeMap::new();
        controls.insert("5.1".to_string(), Control { title: "Buckets not public".into(), paraphrase: "Public access prevention enforced.".into(), automatability: "technical".into(), evidence: Default::default(), duties: Vec::new() });
        controls.insert("2.11".to_string(), Control { title: "SQL change alerts".into(), paraphrase: "Alert on SQL config changes.".into(), automatability: "technical".into(), evidence: Default::default(), duties: Vec::new() });
        Catalog { catalog: "cis-gcp".into(), version: "4.0".into(), controls }
    }

    fn row(bucket: Bucket, control: &str, severity: &str, resource: &str, declared: Option<&str>) -> TriageRow {
        TriageRow {
            bucket,
            control: control.into(),
            title: catalog().controls.get(control).map(|c| c.title.clone()).unwrap_or_default(),
            check: format!("check_{}", control.replace('.', "_")),
            status: "FAIL".into(),
            severity: severity.into(),
            resource: resource.into(),
            project: "acme-infra-001".into(),
            declared: declared.map(str::to_string),
            plan: "do the thing".into(),
        }
    }

    fn checkov(resource: &str, id: &str) -> crate::scan::Finding {
        crate::scan::Finding { check_id: id.into(), check_name: format!("{} name", id), resource: resource.into(), file: "/main.tf".into(), line: Some(10), guideline: None }
    }

    #[test]
    fn items_are_joined_ordered_counted_and_stable() {
        let cat = catalog();
        let mut goals = BTreeMap::new();
        goals.insert("5.1".to_string(), Goal::Satisfied { witnesses: vec!["google_org_policy_policy.pap".into()] });
        goals.insert("2.11".to_string(), Goal::Unmet { providers: vec!["monitoring/organization-cis-log-alerts-central.satz".into()] });
        let rows = vec![
            row(Bucket::A, "2.11", "Medium", "//sqladmin.googleapis.com/projects/p/instances/db1", None),
            row(Bucket::B, "5.1", "High", "//storage.googleapis.com/acme-state", Some("google_storage_bucket.state")),
        ];
        let mut text = BTreeMap::new();
        text.insert("check_5_1".to_string(), ("Bucket is public".to_string(), "Enable PAP".to_string(), "Data exposure".to_string()));
        let ck = vec![checkov("google_storage_bucket.state", "CKV_GCP_62"), checkov("google_folder.x", "CKV_GCP_1")];
        let mut at = BTreeMap::new();
        at.insert("google_storage_bucket.state".to_string(), "yaml/x.satz:40".to_string());

        let inp = Inputs { framework: "cis-gcp-4.0", catalog: &cat, goals: &goals, estate: "x.satz", triage_rows: &rows, prowler_text: &text, checkov: &ck, declared_at: &at };
        let d = build(&inp);

        // order: bucket first (A before B), ids assigned in that order
        assert_eq!(d.items[0].id, "F-0001");
        assert_eq!(d.items[0].bucket, Bucket::A);
        assert_eq!(d.items[0].providers, vec!["monitoring/organization-cis-log-alerts-central.satz".to_string()]);
        // the bucket-B Prowler row is corroborated by Checkov on the same declared address
        let b = &d.items[1];
        assert_eq!(b.control, "5.1");
        assert_eq!(b.sources.len(), 2, "{:?}", b.sources);
        assert_eq!(b.scanner_remediation, "Enable PAP");
        assert_eq!(b.declared_at.as_deref(), Some("yaml/x.satz:40"));
        // the Checkov-only finding becomes its own bucket-B item
        let only = d.items.iter().find(|i| i.resource == "google_folder.x").expect("checkov-only item");
        assert_eq!(only.bucket, Bucket::B);
        assert!(only.plan.contains("Checkov"));
        // counts
        assert_eq!(d.summary.items, 3);
        assert_eq!(d.summary.corroborated, 1);
        assert_eq!(d.summary.declared_apply_fixes, 1);
        assert_eq!(d.summary.by_bucket["A"], 1);
        assert_eq!(d.summary.by_bucket["B"], 2);
        // deterministic: same inputs, same hash
        let d2 = build(&inp);
        assert_eq!(d.hash(), d2.hash());
        assert_eq!(d.framework, "cis-gcp-4.0");
    }

    #[test]
    fn csv_has_every_column_and_escapes() {
        let cat = catalog();
        let goals = BTreeMap::new();
        let rows = vec![row(Bucket::D, "5.1", "Low", "//storage.googleapis.com/b,with\"quote", None)];
        let (empty_text, empty_at) = (BTreeMap::new(), BTreeMap::new());
        let d = build(&Inputs { framework: "cis-gcp-4.0", catalog: &cat, goals: &goals, estate: "x.satz", triage_rows: &rows, prowler_text: &empty_text, checkov: &[], declared_at: &empty_at });
        let csv = csv(&d);
        let header = csv.lines().next().unwrap();
        assert_eq!(header.split(',').count(), COLUMNS.len());
        assert!(csv.contains("\"//storage.googleapis.com/b,with\"\"quote\""), "{}", csv);
        // every row has exactly the column count (quoted commas do not split)
        assert!(csv.lines().nth(1).unwrap().ends_with(",,,,,,,,,,,"), "AI + review columns are empty: {}", csv);
    }

    #[test]
    fn xlsx_builds_with_the_sheets_and_the_ai_columns_empty() {
        let cat = catalog();
        let goals = BTreeMap::new();
        let rows = vec![row(Bucket::B, "5.1", "High", "//storage.googleapis.com/b", Some("google_storage_bucket.b"))];
        let (empty_text, empty_at) = (BTreeMap::new(), BTreeMap::new());
        let d = build(&Inputs { framework: "cis-gcp-4.0", catalog: &cat, goals: &goals, estate: "x.satz", triage_rows: &rows, prowler_text: &empty_text, checkov: &[], declared_at: &empty_at });
        let bytes = xlsx(&d, &[("dossier".into(), d.hash())]).expect("workbook builds");
        // a valid zip with the five sheets — inspect the package, no reader crate needed
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).expect("xlsx is a zip");
        let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().to_string()).collect();
        for sheet in ["sheet1", "sheet2", "sheet3", "sheet4", "sheet5"] {
            assert!(names.iter().any(|n| n.ends_with(&format!("{}.xml", sheet))), "missing {}: {:?}", sheet, names);
        }
        let mut shared = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/sharedStrings.xml").unwrap(), &mut shared).unwrap();
        assert!(shared.contains("[AI] Recommended fix"), "AI header present");
        assert!(shared.contains("google_storage_bucket.b"), "mechanical cell present");
        let mut sheet1 = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/worksheets/sheet1.xml").unwrap(), &mut sheet1).unwrap();
        assert!(sheet1.contains("dataValidation"), "review dropdown present");
    }
}
