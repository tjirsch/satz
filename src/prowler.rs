//! `satz prowler` — the Prowler invocation THIS estate needs, printed, never run.
//!
//! satz does not run Prowler. It stays the thing that joins and judges; the operator or
//! the agent runs the scanner. What was missing was the other half: knowing how to run
//! it for a given estate. A customer with two hundred projects and a claimed CIS 5.0
//! baseline needs different arguments from a single-project estate, and that knowledge
//! lived in somebody's head.
//!
//! So this command reads the estate and writes the command line out. It shells out to
//! nothing, which is what keeps it read-only, keeps it usable on a machine with no
//! Prowler installed, and keeps a human in the loop on a scan that spends API quota in
//! every project of the estate.

// schemars comes through rmcp: one version in the tree, no second dependency to pin
use rmcp::schemars;
use serde::Serialize;
use std::collections::BTreeSet;

use crate::compliance::Claim;
use crate::manifest::Manifest;

/// Where a scan's output goes, decided once so a second scan is comparable with the
/// first and `report-compliance --prowler` has one place to look.
///
/// `evidence/` already exists beside the estate, is already git-ignored and already
/// rejected by the privacy gate, which is exactly right for a file full of a customer's
/// project ids and findings.
pub(crate) const EVIDENCE_DIR: &str = "evidence/prowler";

/// The Prowler compliance framework id for a catalog satz ships, when Prowler has one.
///
/// Only the mappings that are verified against Prowler's own framework list belong here.
/// A claimed framework with no Prowler equivalent is REPORTED as such rather than
/// guessed at: a wrong `--compliance` argument silently scans the wrong control set.
fn prowler_framework(catalog: &str, version: &str) -> Option<&'static str> {
    match (catalog, version) {
        ("cis-gcp", "4.0") => Some("cis_4.0_gcp"),
        ("cis-gcp", "5.0") => Some("cis_5.0_gcp"),
        _ => None,
    }
}

/// What the estate says about how it should be scanned.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub(crate) struct ProwlerPlan {
    /// The organisation the estate declares, when it declares one.
    pub organization_id: Option<String>,
    /// Every project the estate emits, by project id, sorted.
    pub projects: Vec<String>,
    /// Prowler framework ids for the catalogs the estate's claims name.
    pub compliance: Vec<String>,
    /// Catalogs the estate claims that Prowler has no framework for, with why it
    /// matters — those controls are simply not in the scan.
    pub unmapped_frameworks: Vec<String>,
    /// Directory the export belongs in, relative to the estate.
    pub output_directory: String,
    /// File name stem; Prowler appends `.ocsf.json`.
    pub output_filename: String,
    /// The full path `report-compliance --prowler` will read.
    pub output_path: String,
    /// The invocation, ready to paste.
    pub command: String,
    /// What to run afterwards to fold the export back in.
    pub then: String,
}

/// Read the estate and work out how Prowler should be pointed at it.
///
/// `--organization-id` narrows the scan to one organisation; the project list narrows it
/// further and is what makes a two-hundred-project estate's scan finish. Both come from
/// what the estate ACTUALLY declares, which is the whole reason this is a command rather
/// than a documentation page.
pub(crate) fn plan(
    manifest: &Manifest,
    claims: &[(String, Claim)],
    org_id: Option<&str>,
    today: &str,
) -> ProwlerPlan {
    let projects: Vec<String> = manifest
        .resources
        .values()
        .filter(|r| r.tf_type == "google_project")
        .filter_map(|r| r.attrs.get("project_id").cloned())
        .filter(|p| !p.contains('{') && !p.contains("${"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut compliance: Vec<String> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (_, c) in claims {
        if !seen.insert((c.framework.clone(), c.framework_version.clone())) {
            continue;
        }
        match prowler_framework(&c.framework, &c.framework_version) {
            Some(f) => compliance.push(f.to_string()),
            None => unmapped.push(format!("{} {}", c.framework, c.framework_version)),
        }
    }
    compliance.sort();
    compliance.dedup();
    unmapped.sort();
    unmapped.dedup();

    // The name carries the scope and the date, so two scans of one estate never
    // overwrite each other and a file on its own says what produced it.
    let scope = org_id.map(|_| "org").unwrap_or("projects");
    let stem = format!("{}-{}", scope, today);
    let dir = format!("{}/{}", EVIDENCE_DIR, today);

    let mut argv: Vec<String> = vec!["prowler".into(), "gcp".into()];
    if let Some(org) = org_id {
        argv.push("--organization-id".into());
        argv.push(org.to_string());
    }
    if !projects.is_empty() {
        argv.push("--project-ids".into());
        argv.extend(projects.iter().cloned());
    }
    if !compliance.is_empty() {
        argv.push("--compliance".into());
        argv.extend(compliance.iter().cloned());
    }
    argv.push("--output-formats".into());
    argv.push("json-ocsf".into());
    argv.push("--output-directory".into());
    argv.push(dir.clone());
    argv.push("--output-filename".into());
    argv.push(stem.clone());

    let output_path = format!("{}/{}.ocsf.json", dir, stem);
    ProwlerPlan {
        organization_id: org_id.map(str::to_string),
        projects,
        compliance,
        unmapped_frameworks: unmapped,
        output_directory: dir,
        output_filename: stem,
        then: format!(
            "satz report-compliance <framework> <estate> --prowler {} --format markdown --out <file>",
            output_path
        ),
        output_path,
        command: shell_join(&argv),
    }
}

/// Join an argv into a line that can be pasted into a shell unchanged.
///
/// Quoted only where it has to be: a project id or an organisation number never needs
/// it, and a line full of unnecessary quotes is a line people retype by hand.
fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty() && a.chars().all(|c| c.is_alphanumeric() || "-_./:=+,@".contains(c)) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The plan for a terminal: the command first, because that is what the reader came for.
pub(crate) fn render(p: &ProwlerPlan) -> String {
    let mut out = String::from("\nRun Prowler against this estate:\n\n");
    out.push_str(&format!("  {}\n\n", p.command));
    out.push_str("Then fold the export back in:\n\n");
    out.push_str(&format!("  {}\n\n", p.then));

    out.push_str("Why these arguments:\n");
    match &p.organization_id {
        Some(o) => out.push_str(&format!(
            "  --organization-id {}   the organisation this estate declares\n",
            o
        )),
        None => out.push_str(
            "  (no --organization-id)   this estate declares no organisation; Prowler will scan what the credential reaches\n",
        ),
    }
    if p.projects.is_empty() {
        out.push_str(
            "  (no --project-ids)   this estate emits no project with a literal id — a project id built from a param cannot be resolved here\n",
        );
    } else {
        out.push_str(&format!(
            "  --project-ids   the {} project(s) this estate emits, so the scan does not walk the whole organisation\n",
            p.projects.len()
        ));
    }
    if p.compliance.is_empty() {
        out.push_str(
            "  (no --compliance)   this estate claims no framework Prowler has; every check runs\n",
        );
    } else {
        out.push_str(&format!(
            "  --compliance {}   the framework(s) this estate CLAIMS — this also FILTERS which checks run\n",
            p.compliance.join(" ")
        ));
    }
    out.push_str("  --output-formats json-ocsf   the only shape `report-compliance --prowler` reads\n");
    out.push_str(&format!(
        "  --output-directory / --output-filename   makes the path deterministic: {}\n",
        p.output_path
    ));

    if !p.unmapped_frameworks.is_empty() {
        out.push_str(&format!(
            "\nClaimed here, and Prowler has no framework for it: {}.\nThose controls are not in this scan; `require` and `report-compliance` still judge them.\n",
            p.unmapped_frameworks.join(", ")
        ));
    }

    out.push_str(
        "\nsatz does not run Prowler: the scan spends API quota in every project above, so it is yours to start.\nThe credential is yours too — Prowler reads as whoever is logged in, not as the estate's service account.\n",
    );
    out
}

/// Today, UTC, as `YYYY-MM-DD` — the directory a scan's output goes in.
///
/// UTC rather than local time so two people in two time zones scanning the same estate
/// on the same day write into one directory rather than two that look like two scans.
pub(crate) fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_date(secs.div_euclid(86_400))
}

/// Civil date from a day number since the Unix epoch (Howard Hinnant's algorithm).
/// No date crate for one format string.
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::EmittedResource;
    use std::collections::BTreeMap;

    fn project(label: &str, id: &str) -> (String, EmittedResource) {
        (
            format!("google_project.{label}"),
            EmittedResource {
                tf_type: "google_project".into(),
                label: label.into(),
                attrs: BTreeMap::from([("project_id".to_string(), id.to_string())]),
                refs: Default::default(),
                nested: Default::default(),
                nested_all: Default::default(),
                enforce: None,
                reset: false,
                dry_run: false,
                conditional: Vec::new(),
                import_id: None,
                origin: None,
            },
        )
    }

    fn claim(framework: &str, version: &str) -> (String, Claim) {
        (
            "pack".to_string(),
            Claim {
                framework: framework.into(),
                framework_version: version.into(),
                control: "1.1".into(),
                coverage: "implements".into(),
                resources: vec!["google_org_policy_policy.p".into()],
                reason: String::new(),
                manual_duties: Vec::new(),
                interpretation: String::new(),
            },
        )
    }

    #[test]
    fn the_command_carries_the_org_the_projects_and_the_claimed_frameworks() {
        let mut m = Manifest::default();
        m.resources.extend([project("b", "acme-log-001"), project("a", "acme-infra-001")]);
        let claims = vec![claim("cis-gcp", "4.0"), claim("cis-gcp", "5.0"), claim("cis-gcp", "4.0")];

        let p = plan(&m, &claims, Some("123456789012"), "2026-09-13");

        // Projects sorted and de-duplicated, so two runs produce the same line.
        assert_eq!(p.projects, vec!["acme-infra-001", "acme-log-001"]);
        assert_eq!(p.compliance, vec!["cis_4.0_gcp", "cis_5.0_gcp"]);
        assert!(p.unmapped_frameworks.is_empty());
        assert_eq!(
            p.command,
            "prowler gcp --organization-id 123456789012 --project-ids acme-infra-001 acme-log-001 \
             --compliance cis_4.0_gcp cis_5.0_gcp --output-formats json-ocsf \
             --output-directory evidence/prowler/2026-09-13 --output-filename org-2026-09-13"
        );
        assert_eq!(p.output_path, "evidence/prowler/2026-09-13/org-2026-09-13.ocsf.json");
        assert!(p.then.contains(&p.output_path));
    }

    #[test]
    fn a_framework_prowler_does_not_have_is_named_not_guessed() {
        // A wrong --compliance argument silently scans the wrong control set, so an
        // unmapped catalog is reported rather than mapped to something that looks close.
        let p = plan(&Manifest::default(), &[claim("iso27001", "2022")], None, "2026-09-13");
        assert!(p.compliance.is_empty());
        assert_eq!(p.unmapped_frameworks, vec!["iso27001 2022"]);
        assert!(!p.command.contains("--compliance"));
        let text = render(&p);
        assert!(text.contains("Prowler has no framework for it: iso27001 2022"), "{text}");
        assert!(text.contains("every check runs"), "{text}");
    }

    #[test]
    fn a_project_id_that_is_still_a_param_is_left_out() {
        // `--project-ids acme-{customer_shortname}-001` would scan nothing. Better to
        // omit the flag and say so than to emit a line that fails in the terminal.
        let mut m = Manifest::default();
        m.resources.extend([project("a", "{customer_shortname}-infra-001")]);
        let p = plan(&m, &[], None, "2026-09-13");
        assert!(p.projects.is_empty());
        assert!(!p.command.contains("--project-ids"));
        assert!(render(&p).contains("cannot be resolved here"));
    }

    #[test]
    fn an_argument_that_needs_quoting_gets_it() {
        assert_eq!(shell_join(&["a".into(), "b c".into()]), "a 'b c'");
        assert_eq!(shell_join(&["it's".into()]), r"'it'\''s'");
        assert_eq!(shell_join(&["acme-infra-001".into()]), "acme-infra-001");
    }
}
