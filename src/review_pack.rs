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
use crate::settings::ToolConfig;

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

/// How long a reason is. The `text` of a notice that refuses every adopting estate's
/// apply has to say what goes wrong otherwise; anything that fits in fewer characters
/// than this is an instruction, and the operator it stops is left to guess why.
const REASON: usize = 80;

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
    // `/` between components: a `\` in a Satz string is an escape
    s.push_str(&format!("use \"{}\"\n", crate::fsx::slash(pack)));
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
    let pack = crate::fsx::canonicalize(pack).map_err(|e| format!("{}: {}", pack.display(), e))?;
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
            "not formatted — every Satz file in the library is in the canonical layout; the command writes it \
             and never changes meaning"
                .to_string(),
        )
        .fix(format!("satz fmt {}", pack.display()))),
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
                    Severity::Info,
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
    // estate. Everything below reads that estate. The synthetic one lives in a scratch
    // directory this review alone owns, removed on every return below.
    let mut scratch: Option<Scratch> = None;
    let (estate, folded_into) = match against {
        Some(e) => (e.to_path_buf(), e.display().to_string()),
        None => {
            let path = scratch.insert(Scratch::new(&name_of(&pack))?).0.join("review.satz");
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
    let compiled = crate::pipeline_b_compile(
        &estate,
        tool_config,
        runtime_config,
        crate::PrerequisiteFindings::Report,
        crate::FindingsOutput::Silent,
    );
    let out = match compiled {
        Ok(o) => o,
        Err(e) => {
            // a refusal is its first error, where it stands; anything else is what it says
            let why = match crate::findings::as_refusal(e.as_ref()) {
                Some(refusal) => refusal.brief(),
                None => first_line(&e.to_string()),
            };
            f.push(at(&pack, None, Severity::Error, format!("does not compile inside an estate: {}", why)));
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

    // 8. the notices the pack carries: what an operator is told to run once it is on, and
    //    what it holds back. `severity = error` stops every estate that adopts this pack
    //    from applying anything until its operator has run that command and recorded it —
    //    a pack author blocking a customer's apply — so it says why, the way a `deviates`
    //    claim does, and the review says it out loud rather than listing it.
    if let Ok(file) = crate::fsx::read_to_string(&pack).map_err(|e| e.to_string()).and_then(|t| satz_core::satz::parse(&t).map_err(|e| e.msg)) {
        for n in &file.notices {
            let blocks = n.severity == satz_core::satz::Severity::Error;
            f.push(at(
                &pack,
                Some(n.line as u32),
                if blocks { Severity::Warning } else { Severity::Info },
                format!(
                    "notice `{}` [{}]: once the pack is on, `{}` is to run{} — the estate acknowledges it with `{} = true`",
                    n.param,
                    n.severity,
                    n.run,
                    if blocks { ", and every command that writes to the organisation refuses until then" } else { "" },
                    n.param
                ),
            ));
            if blocks && n.text.trim().chars().count() < REASON {
                f.push(at(
                    &pack,
                    Some(n.line as u32),
                    Severity::Error,
                    format!(
                        "notice `{}`: `severity = error` refuses every adopting estate's apply, so its `text` states why \
                         it must wait — a sentence, not a label ({} characters, {} are the bar)",
                        n.param,
                        n.text.trim().chars().count(),
                        REASON
                    ),
                ));
            }
        }
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
                Severity::Info,
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
            Severity::Info,
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
            "adoption rules not checked: <presets_dir>/import-config.yaml is not there — the command writes it".to_string(),
        )
        .fix("satz get-presets")),
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
        Severity::Info,
        "not checked here: the privacy shapes (organisation and project ids, e-mail addresses, domains). \
         `scripts/check-names.sh` in a satz checkout is what rejects them today, and what must become a \
         param before a pack leaves the machine it was written on"
            .to_string(),
    ));

    Ok(Review {
        pack: pack.display().to_string(),
        folded_into,
        emits: mine.iter().map(|r| r.address()).collect(),
        findings: f,
    })
}

/// The directory one review folds its pack in, removed when the review returns, however
/// it returns. `satz mcp` serves reviews concurrently in one process, so the pid and the
/// pack's file stem do not name a review: the number does, one per directory this process
/// has made.
struct Scratch(PathBuf);

impl Scratch {
    fn new(stem: &str) -> Result<Scratch, BoxErr> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("satz-review-{}-{}-{}", std::process::id(), n, stem));
            // `create_dir`, not `create_dir_all`: a directory that is already there belongs
            // to someone else — a process before this one that had the same pid
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Scratch(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("failed to create directory '{}': {}", dir.display(), e).into()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    match (crate::fsx::canonicalize(a), crate::fsx::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.file_name() == b.file_name(),
    }
}

/// The review as a human reads it: what the pack is, what it emits, then the findings
/// in the order they were checked — in the layout every command prints a finding in
/// (`findings::lay_out`) — the count by severity, and the verdict.
pub(crate) fn render(r: &Review, width: crate::findings::Width) -> String {
    let mut out = format!("pack: {}\nfolded into: {}\n", r.pack, r.folded_into);
    out.push_str(&format!("emits: {}\n\n", if r.emits.is_empty() { "nothing".to_string() } else { r.emits.join(", ") }));
    // the first line names the pack in full, so a finding about it names the file alone:
    // the rows stay narrow, and the JSON keeps the path
    let pack_name = Path::new(&r.pack).file_name().map(|n| n.to_string_lossy().into_owned());
    let findings: Vec<Finding> = r
        .findings
        .iter()
        .cloned()
        .map(|mut f| {
            if f.file.as_deref() == Some(r.pack.as_str()) {
                f.file = pack_name.clone().or(f.file);
            }
            f
        })
        .collect();
    out.push_str(&crate::findings::lay_out(&findings, crate::findings::Shown::All, width));
    if let Some(counts) = crate::findings::footer(&findings) {
        out.push_str(&format!("\n{}\n", counts));
    }
    out.push_str(if r.passed() {
        "the pack clears the bar.\n"
    } else {
        "the pack does not clear the bar yet — every error above is a rule the library holds.\n"
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The review prints its findings in the layout every command prints one in — the
    /// pack named by its file in each row, since the first line names it in full — then
    /// the count by severity and the verdict.
    #[test]
    fn the_report_is_the_layout_every_finding_is_printed_in() {
        let pack = Path::new("/tmp/x/my-pack.satz");
        let r = Review {
            pack: pack.display().to_string(),
            folded_into: "synthetic".into(),
            emits: Vec::new(),
            findings: vec![
                at(pack, None, Severity::Error, "not formatted".into()).fix("satz fmt /tmp/x/my-pack.satz"),
                at(pack, Some(3), Severity::Info, "a fact for the log".into()),
            ],
        };
        assert_eq!(
            render(&r, crate::findings::Width::Unwrapped),
            "pack: /tmp/x/my-pack.satz\nfolded into: synthetic\nemits: nothing\n\n\
             error    pack  my-pack.satz\n    not formatted\n    fix: satz fmt /tmp/x/my-pack.satz\n\n\
             info     pack  my-pack.satz:3\n    a fact for the log\n\n\
             1 error, 1 info\n\
             the pack does not clear the bar yet — every error above is a rule the library holds.\n"
        );
    }

    #[test]
    fn the_synthetic_estate_binds_the_documented_examples_and_uses_the_pack() {
        let e = synthetic_estate(Path::new("/tmp/x/my-pack.satz"));
        assert!(e.contains("customer_organization_id = \"123456789012\""), "{}", e);
        assert!(e.contains("use \"/tmp/x/my-pack.satz\""), "{}", e);
        // it is a Satz file satz composes whole, so it must be in the canonical layout
        assert!(satz_core::fmt::is_formatted(&e).unwrap(), "{}", e);
    }

    /// `satz mcp` serves reviews concurrently in one process, and two packs may share a
    /// file stem. Each review folds its own pack into its own estate: neither reads the
    /// other's, and neither finds its estate deleted by the other finishing first.
    #[test]
    fn two_reviews_of_packs_with_one_stem_run_side_by_side() {
        use std::sync::{Arc, Barrier};
        let dir = std::env::temp_dir().join(format!("satz-review-stem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = crate::settings::parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.schema_dir = crate::corpus::schema_dir();
        let packs: Vec<(PathBuf, String)> = ["a", "b"]
            .iter()
            .map(|side| {
                std::fs::create_dir_all(dir.join(side)).unwrap();
                let pack = dir.join(side).join("same-stem.satz");
                let text = format!(
                    "// A log bucket, reviewed beside a pack with the same file name.\n\
                     pack same_stem version \"1.0\"\n\n\
                     google_storage_bucket {{\n  logs_{side} {{\n    name     = \"acme-logs-{side}\"\n    \
                     location = \"EU\"\n  }}\n}}\n"
                );
                std::fs::write(&pack, text).unwrap();
                (pack, format!("google_storage_bucket.logs_{side}"))
            })
            .collect();
        let start = Arc::new(Barrier::new(packs.len()));
        let reviewers: Vec<_> = packs
            .into_iter()
            .map(|(pack, address)| {
                let (cfg, start) = (cfg.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    for n in 0..5 {
                        let r = review(&pack, None, &cfg, &cfg).expect("the review runs");
                        let messages: Vec<&str> = r.findings.iter().map(|f| f.message.as_str()).collect();
                        assert_eq!(r.emits, std::slice::from_ref(&address), "review {} of {}: {:?}", n, pack.display(), messages);
                        assert!(
                            !messages.iter().any(|m| m.starts_with("does not compile")),
                            "review {} of {}: {:?}",
                            n,
                            pack.display(),
                            messages
                        );
                    }
                })
            })
            .collect();
        for reviewer in reviewers {
            reviewer.join().expect("a review read the other's estate or lost its own");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bucket the pack declares outside any project: what the compile says about it is a
    /// finding in the review, at the pack's line, and the review's own compile prints
    /// nothing — the report is the output.
    #[test]
    fn what_the_compile_says_about_the_pack_is_in_the_review_and_nowhere_else() {
        let dir = std::env::temp_dir().join(format!("satz-review-unscoped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::settings::parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        cfg.schema_dir = crate::corpus::schema_dir();
        let pack = dir.join("logs.satz");
        std::fs::write(
            &pack,
            "// A log bucket with no project of its own.\npack logs version \"1.0\"\n\n\
             google_storage_bucket {\n  logs {\n    name     = \"acme-logs\"\n    location = \"EU\"\n  }\n}\n",
        )
        .unwrap();
        crate::findings::take_said();
        let r = review(&pack, None, &cfg, &cfg).expect("the review runs");
        assert!(crate::findings::take_said().is_empty(), "the review's compile printed");
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::MissingScope)
            .unwrap_or_else(|| panic!("the unscoped bucket is not in the review: {:?}", r.findings));
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.starts_with("google_storage_bucket.logs"), "{}", f.message);
        assert_eq!(f.line, Some(5), "at the declaring block");
        let _ = std::fs::remove_dir_all(&dir);
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
