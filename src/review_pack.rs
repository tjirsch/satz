//! `satz review-pack` — the library's quality bar, as a command.
//!
//! A pack is the unit everyone extends satz with, and until this existed the bar it
//! had to clear was enforced entirely by gates inside this repository: a `cargo test`
//! over the prerequisite table, `doc-packs --check`, the formatter's corpus test, the
//! changelog row the index refuses to be without, the managed/legacy pairing. Every
//! one of them needs a satz checkout and a Rust toolchain, so a pack written anywhere
//! else could not be checked at all — its author found out by opening a pull request,
//! or never.
//!
//! The checks are the ones that already exist; what is new is that they are reachable
//! from outside the repository, in the order a pack fails them, as the same `Finding`
//! the compile, `satz lsp` and `satz_transpile_check` produce. Nothing renders them
//! specially: an editor already knows that shape.
//!
//! A pack is a fragment, so to learn what it emits satz folds it into an estate. It
//! synthesises one — the documented example values for the estate's own params, the
//! pack's own declared defaults for its questions — which checks the pack the way a
//! customer first meets it. `--against <estate>` judges it inside a real estate
//! instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rmcp::schemars;

use crate::findings::{Finding, Kind, Severity};
use crate::ToolConfig;

type BoxErr = Box<dyn std::error::Error>;

/// The estate params a pack may read that no pack declares — the estate's own
/// vocabulary (`presets/estate-core.satz`). Bound to the documented example values,
/// so a pack that reads one compiles and a pack that reads something else says so.
const EXAMPLE_PARAMS: &[(&str, &str)] = &[
    ("customer_id", "C0example"),
    ("customer_organization_id", "123456789012"),
    ("customer_domain", "example.com"),
    ("customer_longname", "Example Corp"),
    ("customer_shortname", "acme"),
    ("first_admin", "first.admin"),
    ("billing_account_infra", "012345-6789AB-CDEF01"),
    ("infra_folder_name", "Infrastructure"),
    ("infra_project_name", "acme-infra-001"),
    ("infra_bucket_name", "acme-infra-001-state"),
    ("svc_iac_account", "svc-iac-001"),
    ("svc_iac_users_group", "svc-iac-users"),
    ("deployment_engine", "tofu"),
    ("deployment_mode", "local"),
    ("default_region", "europe-west3"),
    ("default_zone", "europe-west3-a"),
];

/// The estate satz writes to fold the pack into: the example params, then the pack.
pub(crate) fn synthetic_estate(pack: &Path) -> String {
    let mut s = String::from(
        "// Written by `satz review-pack` to fold one pack into an estate. The params are\n\
         // the documented example values; the pack's own questions are answered with the\n\
         // defaults it declares.\n\nestate review\n\nparams {\n",
    );
    for (k, v) in EXAMPLE_PARAMS {
        s.push_str(&format!("  {:<24} = \"{}\"\n", k, v));
    }
    s.push_str("}\n\n");
    // the emitter writes providers for a real estate, so the estate it is given has
    // to be one: a local backend, which is what a fresh estate carries before its
    // state moves to a bucket
    s.push_str("terraform {\n  backend {\n    local { path = \"terraform.tfstate\" }\n  }\n}\n\n");
    s.push_str(&format!("use \"{}\"\n", pack.display()));
    s
}

/// What the review found, and what it could not look at.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Review {
    /// the pack as named on the command line
    pub pack: String,
    /// the estate it was folded into: `synthetic`, or the path `--against` named
    pub folded_into: String,
    /// the resource addresses the pack contributes to that estate
    pub emits: Vec<String>,
    pub findings: Vec<Finding>,
}

impl Review {
    /// A review passes when nothing it found refuses. Warnings are the author's to
    /// weigh; an error is the bar.
    pub(crate) fn passed(&self) -> bool {
        !self.findings.iter().any(|f| f.severity == Severity::Error)
    }
}

fn at(pack: &Path, line: Option<u32>, severity: Severity, message: String) -> Finding {
    let f = Finding::new(severity, Kind::Pack, message).at_file(pack.display().to_string());
    match line {
        Some(l) => f.at_line(l),
        None => f,
    }
}

/// The line a string first appears on, 1-based — so a finding points at the thing it
/// names rather than at the top of the file.
fn line_of(src: &str, needle: &str) -> Option<u32> {
    src.lines().position(|l| l.contains(needle)).map(|i| i as u32 + 1)
}

/// `pack <name> version "x"` — the version a pack carries in-file, which is what the
/// library's changelog and the fork/repoint machinery compare.
fn declared_version(src: &str) -> Option<(String, String)> {
    for line in src.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("pack ") else { continue };
        let mut parts = rest.split_whitespace();
        let name = parts.next()?.trim_matches('"').to_string();
        let version = rest.split("version").nth(1)?.trim().trim_matches('"').to_string();
        return Some((name, version));
    }
    None
}

/// The whole review, in the order a pack fails it.
pub(crate) fn review(
    pack: &Path,
    against: Option<&Path>,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<Review, BoxErr> {
    let pack = pack.canonicalize().map_err(|e| format!("{}: {}", pack.display(), e))?;
    let src = crate::fsx::read_to_string(&pack)?;
    let mut f: Vec<Finding> = Vec::new();

    // 1. it parses. Nothing below means anything if this fails, so the review stops.
    if let Err(e) = satz_core::satz::parse(&src) {
        f.push(at(&pack, None, Severity::Error, format!("does not parse: {}", e)));
        return Ok(Review {
            pack: pack.display().to_string(),
            folded_into: String::new(),
            emits: Vec::new(),
            findings: f,
        });
    }

    // 2. the layout is the library's (ADR 0017). `satz fmt <file>` is the whole fix.
    match satz_core::fmt::is_formatted(&src) {
        Ok(true) => {}
        Ok(false) => f.push(at(
            &pack,
            None,
            Severity::Error,
            "not formatted — every Satz file in the library is in the canonical layout; \
             `satz fmt <file>` writes it and never changes meaning"
                .to_string(),
        )),
        Err(e) => f.push(at(&pack, None, Severity::Warning, format!("could not be formatted: {}", e))),
    }

    // 3. the header says what the pack IS: the index prints that sentence, and a pack
    //    the index cannot introduce is a pack nobody finds.
    let rel = pack.file_name().map(PathBuf::from).unwrap_or_else(|| pack.to_path_buf());
    match crate::doc_packs::header(&src, &rel).and_then(|h| crate::doc_packs::summary(&h, &rel)) {
        Ok(_) => {}
        // the index's own message, minus the `presets/<file>:` it prefixes for a pack
        // inside the library — this finding already carries the file it is about
        Err(e) => {
            let text = e.to_string();
            let message = text.split_once(": ").map(|(_, rest)| rest.to_string()).unwrap_or(text);
            f.push(at(&pack, Some(1), Severity::Error, message));
        }
    }

    // 4. the version in-file, and its row in the library's changelog. A pack outside a
    //    library has no changelog to be in, and is told so rather than failed.
    match declared_version(&src) {
        None => f.push(at(
            &pack,
            line_of(&src, "pack "),
            Severity::Error,
            "no `pack <name> version \"…\"` statement — the version lives in the file, and \
             every update compares it"
                .to_string(),
        )),
        Some((name, version)) => {
            let readme = Path::new(&runtime_config.presets_dir).join("README.md");
            match std::fs::read_to_string(&readme) {
                Ok(text) => {
                    let row = text
                        .lines()
                        .any(|l| l.starts_with('|') && l.contains(&name) && l.contains(&format!("| {} |", version)));
                    if !row {
                        f.push(at(
                            &pack,
                            line_of(&src, "pack "),
                            Severity::Error,
                            format!(
                                "version {} has no row in {}'s `## Changelog` — a version nobody can read the \
                                 history of is a version nobody can adopt",
                                version,
                                readme.display()
                            ),
                        ));
                    }
                }
                Err(_) => f.push(at(
                    &pack,
                    None,
                    Severity::Note,
                    format!(
                        "no `{}` to check the changelog row against — a pack contributed upstream needs one \
                         row per version there",
                        readme.display()
                    ),
                )),
            }
        }
    }

    // The fold: a pack is a fragment, so what it emits is only knowable inside an
    // estate. Everything below reads that estate.
    let scratch = std::env::temp_dir().join(format!("satz-review-{}-{}", std::process::id(), name_of(&pack)));
    crate::fsx::create_dir_all(&scratch)?;
    let (estate, folded_into) = match against {
        Some(e) => (e.to_path_buf(), e.display().to_string()),
        None => {
            let path = scratch.join("review.satz");
            crate::fsx::write_generated_satz(&path, &synthetic_estate(&pack))?;
            (path, "synthetic".to_string())
        }
    };
    // the pack's own questions, answered with the defaults it declares — the way a
    // customer first meets it. A question with no possible default is named below.
    if against.is_none() {
        if let Err(e) = crate::interview::apply(&estate, runtime_config, &BTreeMap::new(), true) {
            f.push(at(&pack, None, Severity::Warning, format!("its questions could not all be answered: {}", e)));
        }
    }

    // 5. it compiles, and 9. everything the compile itself checks — raw HCL, actions,
    //    required arguments — arrives as the findings the CLI and the editor already
    //    read. The compile is internal, so it prints nothing: this report is the output.
    crate::FINDINGS_QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
    let compiled = crate::pipeline_b_generate(&estate, tool_config, runtime_config);
    crate::FINDINGS_QUIET.store(false, std::sync::atomic::Ordering::Relaxed);
    let out = match compiled {
        Ok(o) => o,
        Err(e) => {
            f.push(at(
                &pack,
                None,
                Severity::Error,
                format!("does not compile inside an estate: {}", first_line(&e.to_string())),
            ));
            let _ = std::fs::remove_dir_all(&scratch);
            return Ok(Review { pack: pack.display().to_string(), folded_into, emits: Vec::new(), findings: f });
        }
    };

    // what THIS pack contributes: the resources whose declaring file is the pack
    let mine: Vec<&crate::manifest::EmittedResource> = out
        .manifest
        .resources
        .values()
        .filter(|r| r.origin.as_ref().is_some_and(|(file, _)| same_file(file, &pack)))
        .collect();
    if mine.is_empty() {
        f.push(at(
            &pack,
            None,
            match against {
                Some(_) => Severity::Error,
                None => Severity::Warning,
            },
            match against {
                Some(e) => format!("{} emits nothing from this pack — does that estate `use` it?", e.display()),
                None => "it emits no resource in a fresh estate — a pack whose resources are all behind a \
                         `when` is checked with `--against <estate>`"
                    .to_string(),
            },
        ));
    }

    // 6. blocking questions: a question with no possible default is what a customer
    //    must type, and naming them is half of what a pack author is reviewing.
    if let Ok(report) = crate::questions::questions_report(&estate, runtime_config) {
        let blocking: Vec<&str> =
            report.questions.iter().filter(|q| q.blocking && q.state == "unanswered").map(|q| q.subject.as_str()).collect();
        if !blocking.is_empty() {
            f.push(at(
                &pack,
                None,
                Severity::Note,
                format!(
                    "{} question(s) a customer must answer, no default is possible: {}",
                    blocking.len(),
                    blocking.join(", ")
                ),
            ));
        }
    }

    // 7. memberships stay OUT of presets: a pack defines groups, humans grant
    //    membership. This is the one rule a well-meaning pack breaks most easily.
    for r in mine.iter().filter(|r| r.tf_type == "google_cloud_identity_group_membership") {
        f.push(at(
            &pack,
            r.origin.as_ref().map(|(_, l)| *l),
            Severity::Error,
            format!(
                "declares the membership {} — presets define groups, humans grant membership: who is in a \
                 group is the customer's decision and never the library's",
                r.address()
            ),
        ));
    }

    // 8. one form of a constraint, never two: a legacy constraint a managed one
    //    replaces is declared OFF (`spec { reset = true }`), never enforced beside it.
    let pairs = crate::presets::constraint_pairs(Path::new(&runtime_config.presets_dir));
    for r in mine.iter().filter(|r| r.tf_type == "google_org_policy_policy") {
        let Some(name) = r.attrs.get("name") else { continue };
        let constraint = name.rsplit('/').next().unwrap_or(name);
        let Some(managed) = pairs.get(constraint) else { continue };
        let off = r.nested.get("spec.reset").is_some_and(|v| v == "true");
        if !off {
            f.push(at(
                &pack,
                r.origin.as_ref().map(|(_, l)| *l),
                Severity::Error,
                format!(
                    "enforces the legacy constraint {}, which Google replaced with {} — run the replacement \
                     alone and declare this one `spec {{ reset = true }}`, or an organisation ends up with \
                     both set from two places",
                    constraint, managed
                ),
            ));
        }
    }

    // 9. what this pack obliges an estate to declare, from the types it emits: the
    //    roles its IaC service account needs and the APIs its infra project must
    //    enable. `satz update-prerequisites` writes exactly these into a customer's
    //    estate — saying them here is what tells the author what adopting it costs.
    let mut roles: std::collections::BTreeSet<&str> = Default::default();
    let mut apis: std::collections::BTreeSet<&str> = Default::default();
    let mut unknown: Vec<String> = Vec::new();
    for t in mine.iter().map(|r| r.tf_type.as_str()).collect::<std::collections::BTreeSet<_>>() {
        match crate::prerequisites::entries_for(t) {
            Some(es) => roles.extend(es.iter().filter_map(|e| e.roles.first().copied())),
            None => unknown.push(t.to_string()),
        }
        apis.extend(crate::prerequisites::apis_for(t).unwrap_or(&[]).iter().copied());
    }
    if !unknown.is_empty() {
        f.push(at(
            &pack,
            None,
            Severity::Error,
            format!(
                "emits {} — no row in satz's prerequisite table, so neither the role its IaC service account \
                 needs nor the API that serves it can be checked. A type the library emits carries a row \
                 (src/prerequisites.rs); until it does, an estate using this pack fails its apply on a \
                 permission or an API nobody named",
                unknown.join(", ")
            ),
        ));
    }
    if !roles.is_empty() || !apis.is_empty() {
        f.push(at(
            &pack,
            None,
            Severity::Note,
            format!(
                "adopting it costs an estate: {} and {}. `satz update-prerequisites <estate>` writes both",
                if roles.is_empty() { "no new role".to_string() } else { roles.into_iter().collect::<Vec<_>>().join(", ") },
                if apis.is_empty() { "no new API".to_string() } else { apis.into_iter().collect::<Vec<_>>().join(", ") }
            ),
        ));
    }

    // 10. whether `adopt` can find what this pack creates, once it exists for any
    //     reason — a console click, a partial apply, a re-run after a failure. A type
    //     with no adoption rule leaves `tofu import` by hand as the only way back: the
    //     SCC notification config 409'd on a re-run and adopt answered "no rule".
    match crate::load_import_config(None, tool_config, &runtime_config.presets_dir) {
        Ok(Some(rules)) => {
            let unadoptable: Vec<&str> = mine
                .iter()
                .map(|r| r.tf_type.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter(|t| !crate::adopt::adoptable(&rules, t))
                .collect();
            if !unadoptable.is_empty() {
                f.push(at(
                    &pack,
                    None,
                    Severity::Warning,
                    format!(
                        "emits {} — `satz adopt` has no rule for {}, so an object that already exists cannot be \
                         brought under management except by `tofu import` by hand. Add `import_id:` (the id \
                         follows from what the pack declares) or `match_on:` (a live lookup) to the row in \
                         import-config.yaml",
                        unadoptable.join(", "),
                        if unadoptable.len() == 1 { "it" } else { "them" }
                    ),
                ));
            }
        }
        Ok(None) => f.push(at(
            &pack,
            None,
            Severity::Warning,
            "adoption rules not checked: <presets_dir>/import-config.yaml is not there — `satz get-presets` writes it".to_string(),
        )),
        Err(e) => f.push(at(&pack, None, Severity::Error, format!("adoption rules not checked: {}", e))),
    }

    // What the compile said that names this pack's own file — raw HCL it carries, an
    // action it declares. The estate-wide findings are about the scratch estate.
    for finding in out.findings.iter().filter(|x| x.file.as_deref().is_some_and(|file| same_file(file, &pack))) {
        f.push(finding.clone());
    }

    // The privacy shapes are not part of this command yet: a pack written against the
    // author's own organisation carries ids, domains and addresses that must become
    // params before it can leave their machine, and that gate is still the shell
    // script in the repository.
    f.push(at(
        &pack,
        None,
        Severity::Note,
        "not checked here: the privacy shapes (organisation and project ids, e-mail addresses, domains). \
         `scripts/check-names.sh` in a satz checkout is what rejects them today, and what must become a \
         param before a pack leaves the machine it was written on"
            .to_string(),
    ));

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(Review {
        pack: pack.display().to_string(),
        folded_into,
        emits: mine.iter().map(|r| r.address()).collect(),
        findings: f,
    })
}

fn name_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "pack".to_string())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

/// Two paths naming one file, whatever the shape they arrived in: a `use` path as the
/// loader saw it, or the absolute path the command line named.
fn same_file(a: &str, b: &Path) -> bool {
    let a = Path::new(a);
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.file_name() == b.file_name(),
    }
}

/// The review as a human reads it: what the pack is, what it emits, then the findings
/// in the order they were checked.
pub(crate) fn render(r: &Review) -> String {
    let mut out = format!("pack: {}\nfolded into: {}\n", r.pack, r.folded_into);
    out.push_str(&format!("emits: {}\n", if r.emits.is_empty() { "nothing".to_string() } else { r.emits.join(", ") }));
    let count = |s: Severity| r.findings.iter().filter(|f| f.severity == s).count();
    out.push_str(&format!(
        "{} error(s), {} warning(s), {} note(s)\n\n",
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Note)
    ));
    for f in &r.findings {
        let label = match f.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        let at = match (&f.file, f.line) {
            (Some(file), Some(line)) => format!("{}:{}: ", file, line),
            (Some(file), None) => format!("{}: ", file),
            _ => String::new(),
        };
        out.push_str(&format!("{}: {}{}\n", label, at, f.message));
    }
    out.push_str(if r.passed() {
        "\nthe pack clears the bar.\n"
    } else {
        "\nthe pack does not clear the bar yet — every error above is a rule the library holds.\n"
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthetic_estate_binds_the_documented_examples_and_uses_the_pack() {
        let e = synthetic_estate(Path::new("/tmp/x/my-pack.satz"));
        assert!(e.contains("customer_organization_id = \"123456789012\""), "{}", e);
        assert!(e.contains("use \"/tmp/x/my-pack.satz\""), "{}", e);
        // it is a Satz file satz composes whole, so it must be in the canonical layout
        assert!(satz_core::fmt::is_formatted(&e).unwrap(), "{}", e);
    }

    #[test]
    fn the_version_is_read_from_the_pack_statement() {
        assert_eq!(
            declared_version("// a pack\n\npack organization_budget version \"1.0\"\n"),
            Some(("organization_budget".to_string(), "1.0".to_string()))
        );
        assert_eq!(declared_version("// no pack statement\n"), None);
    }
}
