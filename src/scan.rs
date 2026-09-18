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
    // The file found, by its full path: on Windows `checkov` is `checkov.exe` or a
    // `checkov.cmd` shim, and a bare name spawns only an `.exe`.
    let found = |bin: &str| {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .flat_map(|d| executable_names(bin).into_iter().map(move |n| d.join(n)))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    };
    if let Some(checkov) = found("checkov") {
        return Ok((checkov, vec![]));
    }
    if let Some(uvx) = found("uvx") {
        return Ok((uvx, vec!["checkov".into()]));
    }
    Err("Checkov not found: install it (`pipx install checkov`) or install uv — `uvx checkov` runs it on demand".into())
}

/// The file names `bin` may have on PATH: itself, and on Windows with each `PATHEXT`
/// extension.
fn executable_names(bin: &str) -> Vec<String> {
    names_for(bin, cfg!(windows).then(|| std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())))
}

fn names_for(bin: &str, pathext: Option<String>) -> Vec<String> {
    let mut names = vec![bin.to_string()];
    if let Some(exts) = pathext {
        names.extend(exts.split(';').filter(|e| !e.is_empty()).map(|e| format!("{}{}", bin, e.to_ascii_lowercase())));
    }
    names
}

/// Run Checkov (terraform framework, JSON) over `hcl_dir`.
pub(crate) fn run(hcl_dir: &Path) -> Result<Report, String> {
    run_with_json(hcl_dir).map(|(report, _)| report)
}

/// Run Checkov over `hcl_dir`: the report, and the JSON it was read from, as Checkov
/// wrote it — the file `read` takes back.
pub(crate) fn run_with_json(hcl_dir: &Path) -> Result<(Report, String), String> {
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
    let report = parse(&text)
        .map_err(|e| format!("could not read Checkov's JSON: {}\n{}", e, String::from_utf8_lossy(&out.stderr).trim()))?;
    Ok((report, text.into_owned()))
}

/// A Checkov JSON report already on disk — what `satz_scan_checkov` writes to its
/// `out`, or `checkov -o json` run by hand. Reading it runs nothing.
pub(crate) fn read(path: &Path) -> Result<Report, String> {
    let text = crate::fsx::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&text).map_err(|e| {
        format!(
            "{}: not a Checkov JSON report ({}) — `satz_scan_checkov` with `out` writes one, \
             as does `checkov -d <hcl_dir> --framework terraform -o json`",
            path.display(),
            e
        )
    })
}

/// Checkov's JSON: one object, or a list of them (one per framework).
pub(crate) fn parse(text: &str) -> Result<Report, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let objs: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(a) => a.iter().collect(),
        o => vec![o],
    };
    if objs.is_empty() {
        return Err("an empty list — no framework's report in it".into());
    }
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
    fn on_windows_checkov_is_looked_for_by_its_pathext_names() {
        assert_eq!(names_for("checkov", None), vec!["checkov"]);
        assert_eq!(names_for("checkov", Some(".EXE;.CMD".into())), vec!["checkov", "checkov.exe", "checkov.cmd"]);
    }

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

    /// The remediation tools read a scan that already ran, so a file that is not
    /// Checkov's JSON is refused by its path, never read as "no findings".
    #[test]
    fn a_report_on_disk_is_read_and_one_that_is_not_checkovs_is_refused_by_its_path() {
        let dir = std::env::temp_dir().join(format!("satz-scan-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("checkov.json");
        std::fs::write(
            &good,
            r#"{"results":{"failed_checks":[{"check_id":"CKV_GCP_62","check_name":"Bucket should log access","resource":"google_storage_bucket.b","file_path":"/main.tf","file_line_range":[10,20]}]},"summary":{"passed":1,"failed":1,"skipped":0,"resource_count":2,"checkov_version":"3.3.15"}}"#,
        )
        .unwrap();
        let r = read(&good).unwrap();
        assert_eq!((r.failed, r.version.as_str(), r.findings[0].check_id.as_str()), (1, "3.3.15", "CKV_GCP_62"));

        let prowler = dir.join("prowler.json");
        for not_checkov in [r#"[{"status_code":"FAIL"}]"#, "[]", ""] {
            std::fs::write(&prowler, not_checkov).unwrap();
            let e = read(&prowler).unwrap_err();
            assert!(e.contains("prowler.json") && e.contains("not a Checkov JSON report"), "{not_checkov:?}: {e}");
        }

        let e = read(&dir.join("absent.json")).unwrap_err();
        assert!(e.contains("absent.json"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
