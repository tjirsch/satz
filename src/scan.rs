//! `satz scan` (roadmap Phase 6): Checkov over the emitted HCL, findings
//! pointed back at the Satz source through the emission manifest. A finding
//! on an emitted resource is evidence input for the compliance plane later;
//! today it is a report — and a gate: failed checks exit non-zero.
//!
//! Checkov is not bundled: `checkov` on PATH, else `uvx checkov` (uv runs it
//! on demand), else a clear error naming both.

use std::path::Path;
use std::process::Command;

use crate::manifest::Manifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    pub check_id: String,
    pub check_name: String,
    /// Terraform address, `google_folder.x`
    pub resource: String,
    pub file: String,
    pub line: Option<u64>,
    pub guideline: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub resource_count: u64,
    pub version: String,
    pub findings: Vec<Finding>,
}

/// How to invoke Checkov on this machine.
fn runner() -> Result<(String, Vec<String>), String> {
    let on_path = |bin: &str| {
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
            .unwrap_or(false)
    };
    if on_path("checkov") {
        return Ok(("checkov".into(), vec![]));
    }
    if on_path("uvx") {
        return Ok(("uvx".into(), vec!["checkov".into()]));
    }
    Err("Checkov not found: install it (`pipx install checkov`) or install uv — `uvx checkov` runs it on demand".into())
}

/// Run Checkov (terraform framework, JSON) over `hcl_dir`.
pub(crate) fn run(hcl_dir: &Path) -> Result<Report, String> {
    if !hcl_dir.is_dir() {
        return Err(format!("hcl dir '{}' does not exist — run `transpile` first", hcl_dir.display()));
    }
    let (bin, pre) = runner()?;
    let out = Command::new(&bin)
        .args(&pre)
        .args(["-d", &hcl_dir.to_string_lossy(), "--framework", "terraform", "-o", "json", "--quiet"])
        .output()
        .map_err(|e| format!("{}: {}", bin, e))?;
    // Checkov exits 1 when checks fail; the JSON is the result either way
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return Err(format!("{} produced no output: {}", bin, String::from_utf8_lossy(&out.stderr).trim()));
    }
    parse(&text).map_err(|e| format!("could not read Checkov's JSON: {}\n{}", e, String::from_utf8_lossy(&out.stderr).trim()))
}

/// Checkov's JSON: one object, or a list of them (one per framework).
pub(crate) fn parse(text: &str) -> Result<Report, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let objs: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(a) => a.iter().collect(),
        o => vec![o],
    };
    let mut r = Report::default();
    for o in objs {
        let s = o.get("summary").ok_or("no `summary`")?;
        let n = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        r.passed += n("passed");
        r.failed += n("failed");
        r.skipped += n("skipped");
        r.resource_count += n("resource_count");
        if let Some(ver) = s.get("checkov_version").and_then(|x| x.as_str()) {
            r.version = ver.to_string();
        }
        if let Some(fails) = o.get("results").and_then(|x| x.get("failed_checks")).and_then(|x| x.as_array()) {
            for f in fails {
                let g = |k: &str| f.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                r.findings.push(Finding {
                    check_id: g("check_id"),
                    check_name: g("check_name"),
                    resource: g("resource"),
                    file: g("file_path").trim_start_matches('/').to_string(),
                    line: f.get("file_line_range").and_then(|x| x.as_array()).and_then(|a| a.first()).and_then(|x| x.as_u64()),
                    guideline: f.get("guideline").and_then(|x| x.as_str()).map(String::from),
                });
            }
        }
    }
    r.findings.sort_by(|a, b| (&a.resource, &a.check_id).cmp(&(&b.resource, &b.check_id)));
    Ok(r)
}

/// The report, each finding pointed at the Satz block that declared the
/// resource when the manifest knows it.
pub(crate) fn render(r: &Report, manifest: Option<&Manifest>) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "scan: Checkov {} over {} resource(s) — {} passed, {} failed, {} skipped\n",
        r.version, r.resource_count, r.passed, r.failed, r.skipped
    ));
    let mut last = String::new();
    for f in &r.findings {
        if f.resource != last {
            let origin = manifest
                .and_then(|m| m.resources.get(&f.resource))
                .and_then(|res| res.origin.as_ref())
                .map(|(file, line)| format!("  (declared at {}:{})", file, line))
                .unwrap_or_default();
            out.push_str(&format!("\n  {}{}\n", f.resource, origin));
            last = f.resource.clone();
        }
        let at = match f.line {
            Some(l) => format!("{}:{}", f.file, l),
            None => f.file.clone(),
        };
        out.push_str(&format!("    {:12} {} — {}", f.check_id, at, f.check_name));
        if let Some(g) = &f.guideline {
            out.push_str(&format!("\n                 {}", g));
        }
        out.push('\n');
    }
    if r.failed == 0 {
        out.push_str("scan: no failed checks.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkov_json_parses_in_both_shapes_and_findings_sort_by_resource() {
        let one = r#"{"check_type":"terraform","results":{"failed_checks":[
            {"check_id":"CKV_GCP_62","check_name":"Bucket should log access","resource":"google_storage_bucket.b","file_path":"/main.tf","file_line_range":[10,20],"guideline":"https://example.com/g"},
            {"check_id":"CKV_GCP_45","check_name":"No impersonation roles at org level","resource":"google_organization_iam_member.a","file_path":"/main.tf","file_line_range":[3,5],"guideline":null}
        ]},"summary":{"passed":20,"failed":2,"skipped":0,"parsing_errors":0,"resource_count":58,"checkov_version":"3.3.15"}}"#;
        let r = parse(one).unwrap();
        assert_eq!((r.passed, r.failed, r.resource_count, r.version.as_str()), (20, 2, 58, "3.3.15"));
        assert_eq!(r.findings[0].resource, "google_organization_iam_member.a");
        assert_eq!(r.findings[1].line, Some(10));
        assert_eq!(r.findings[1].file, "main.tf");
        let list = format!("[{}]", one);
        assert_eq!(parse(&list).unwrap(), r);
        let text = render(&r, None);
        assert!(text.contains("2 failed"), "{}", text);
        assert!(text.contains("CKV_GCP_62   main.tf:10 — Bucket should log access"), "{}", text);
        assert!(text.contains("https://example.com/g"), "{}", text);
    }
}
