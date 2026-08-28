//! Compliance plane: catalogs, control claims, and the `require` goal view.
//!
//! Proof-assistant framing, taken literally: a framework catalog is a set of
//! propositions (controls); a preset pack's claims sidecar states lemmas ("including
//! me discharges these controls, witnessed by these resources"); the transpiled
//! estate supplies the witnesses. `satz require <framework> <INPUT>` is the goal
//! view: which obligations are discharged, which are partial (open manual duties),
//! which are unmet — and for unmet ones, which pack in the library would provide
//! them (remediation as tactic suggestions).
//!
//! Deliberate vocabulary: the output never says "compliant". Satisfaction is
//! claims ∧ duties ∧ (later) live verification; this command judges the *declared*
//! estate. The live half (evidence reports) builds on the same data model.
//!
//! Catalogs live in `<presets_dir>/catalogs/<framework>-<version>.yaml` and carry
//! control IDs plus this project's own paraphrases — never framework text (CIS/ISO
//! prose is license-restricted).
//!
//! Claims are language syntax: a `.satz` pack declares them inline and they reach
//! this module through the fragment pipeline's front end, produced by the same
//! compile that produced the `main.tf` whose addresses supply the witnesses. One
//! read, so claims and witnesses can never disagree about which estate they
//! describe. The YAML dialect's `*.claims.yaml` sidecar route survives only for
//! the last un-converted estate and retires with the dialect (M3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

type BoxErr = Box<dyn std::error::Error>;

// ---------------------------------------------------------------------------
// Data model (all plain YAML, all reviewable as diffs)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct Catalog {
    pub catalog: String,
    pub version: String,
    #[serde(default)]
    pub controls: BTreeMap<String, Control>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Control {
    pub title: String,
    #[serde(default)]
    #[allow(dead_code)] // read by the evidence report (Phase 1b)
    pub paraphrase: String,
    #[serde(default = "default_automatability")]
    pub automatability: String,
}

fn default_automatability() -> String {
    "technical".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Claim {
    pub framework: String,
    #[serde(rename = "framework-version", default)]
    pub framework_version: String,
    pub control: String,
    /// `implements` discharges the control; `contributes` is a necessary part.
    pub coverage: String,
    #[serde(default)]
    pub resources: Vec<String>,
    /// Present only on `deviates` claims: why the estate knowingly does not meet
    /// this control.
    #[serde(default)]
    pub reason: String,
    #[serde(rename = "manual-duties", default)]
    pub manual_duties: Vec<ManualDuty>,
    #[serde(default)]
    #[allow(dead_code)] // shown in the evidence report (Phase 1b)
    pub interpretation: String,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ManualDuty {
    pub id: String,
    #[allow(dead_code)]
    pub duty: String,
}

/// One control's verdict in the goal view.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Goal {
    /// ≥1 `implements` claim from an included pack, every witness emitted, no duties.
    Satisfied { witnesses: Vec<String> },
    /// Witnesses present but duties remain open, or only `contributes` claims exist.
    Partial { witnesses: Vec<String>, open_duties: Vec<String>, contributes_only: bool },
    /// A pack was included and claims this control, but declared witnesses are not
    /// in the emitted estate — a broken lemma, worse than unmet.
    ClaimBroken { missing: Vec<String>, pack: String },
    /// An included pack or the estate declares a DELIBERATE non-conformance with
    /// a stated reason. Disclosed as a finding, never counted as a gap: a `.local`
    /// fork exists precisely so a customer can decline a control on purpose, and
    /// the report must say so rather than show a hole indistinguishable from an
    /// oversight. Does not fail the gate.
    Deviation { reasons: Vec<(String, String)>, witnesses: Vec<String>, open_duties: Vec<String> },
    /// No included pack claims it. `providers` = packs in the library that would.
    Unmet { providers: Vec<String> },
    /// The catalog marks it organizational — no IaC witness possible.
    Organizational,
}

// ---------------------------------------------------------------------------
// Pure layer: goal resolution
// ---------------------------------------------------------------------------

/// Resolve every catalog control against the claims of included packs and the
/// emitted resource addresses. `library_claims` = claims of ALL packs in the
/// preset library (for remediation suggestions only); `included_claims` = the
/// claims the estate actually pulled in, carried with the file that declared
/// them rather than looked up by pack name — a `.local` fork shares its
/// pristine twin's pack name but need not share its witnesses.
pub(crate) fn resolve_goals(
    catalog: &Catalog,
    library_claims: &[(String, Claim)], // (pack, claim)
    included_claims: &[(String, Claim)], // (pack, claim) — actually included
    emitted: &BTreeSet<String>,
) -> BTreeMap<String, Goal> {
    let mut goals = BTreeMap::new();

    for (id, control) in &catalog.controls {
        if control.automatability == "organizational" {
            goals.insert(id.clone(), Goal::Organizational);
            continue;
        }

        let relevant: Vec<&(String, Claim)> = library_claims
            .iter()
            .filter(|(_, c)| {
                c.framework == catalog.catalog
                    && (c.framework_version.is_empty() || c.framework_version == catalog.version)
                    && c.control == *id
            })
            .collect();

        let included: Vec<&(String, Claim)> = included_claims
            .iter()
            .filter(|(_, c)| {
                c.framework == catalog.catalog
                    && (c.framework_version.is_empty() || c.framework_version == catalog.version)
                    && c.control == *id
            })
            .collect();

        if included.is_empty() {
            let mut providers: Vec<String> =
                relevant.iter().map(|(pack, _)| pack.clone()).collect();
            providers.sort();
            providers.dedup();
            goals.insert(id.clone(), Goal::Unmet { providers });
            continue;
        }

        // A deliberate deviation decides the control before anything else: it is
        // an explicit statement about THIS control, so it outranks both the
        // positive claims it contradicts (a fork that declares the policy but
        // does not enforce it) and the broken-lemma verdict an estate-level
        // `suppress` would otherwise produce.
        let deviations: Vec<&(String, Claim)> =
            included.iter().copied().filter(|(_, c)| c.coverage == "deviates").collect();
        if !deviations.is_empty() {
            // Witnesses are optional on a deviation, but any it DOES declare must
            // still be emitted — one estate's policy exists with enforce = "FALSE", and
            // if someone later deletes it outright that is a different fact and
            // must resurface rather than stay silently "deviated".
            let mut missing = Vec::new();
            let mut witnesses = Vec::new();
            for (pack, c) in &deviations {
                for r in &c.resources {
                    if emitted.contains(r) {
                        witnesses.push(r.clone());
                    } else {
                        missing.push((r.clone(), (*pack).clone()));
                    }
                }
            }
            if let Some((_, pack)) = missing.first().cloned() {
                let missing: Vec<String> = missing.into_iter().map(|(r, _)| r).collect();
                goals.insert(id.clone(), Goal::ClaimBroken { missing, pack });
                continue;
            }
            let mut open_duties: Vec<String> = deviations
                .iter()
                .flat_map(|(_, c)| c.manual_duties.iter().map(|d| d.id.clone()))
                .collect();
            open_duties.sort();
            open_duties.dedup();
            witnesses.sort();
            witnesses.dedup();
            let reasons =
                deviations.iter().map(|(p, c)| (p.clone(), c.reason.clone())).collect();
            goals.insert(id.clone(), Goal::Deviation { reasons, witnesses, open_duties });
            continue;
        }

        // A claim holds iff every declared witness is actually emitted.
        let mut witnesses = Vec::new();
        let mut open_duties = Vec::new();
        let mut has_implements = false;
        let mut broken: Option<(Vec<String>, String)> = None;

        for (pack, claim) in &included {
            let missing: Vec<String> = claim
                .resources
                .iter()
                .filter(|r| !emitted.contains(*r))
                .cloned()
                .collect();
            if !missing.is_empty() {
                broken = Some((missing, pack.clone()));
                continue;
            }
            witnesses.extend(claim.resources.iter().cloned());
            open_duties.extend(claim.manual_duties.iter().map(|d| d.id.clone()));
            if claim.coverage == "implements" {
                has_implements = true;
            }
        }

        let goal = if witnesses.is_empty() {
            match broken {
                Some((missing, pack)) => Goal::ClaimBroken { missing, pack },
                None => Goal::Unmet { providers: Vec::new() },
            }
        } else if has_implements && open_duties.is_empty() {
            witnesses.sort();
            witnesses.dedup();
            Goal::Satisfied { witnesses }
        } else {
            witnesses.sort();
            witnesses.dedup();
            open_duties.sort();
            open_duties.dedup();
            Goal::Partial { witnesses, open_duties, contributes_only: !has_implements }
        };
        goals.insert(id.clone(), goal);
    }

    goals
}

/// Extract emitted Terraform resource addresses from generated HCL.
pub(crate) fn emitted_addresses(main_tf: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in main_tf.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("resource \"") {
            let mut parts = rest.split('"');
            let tf_type = parts.next().unwrap_or("");
            parts.next(); // space between labels
            let label = parts.next().unwrap_or("");
            if !tf_type.is_empty() && !label.is_empty() {
                out.insert(format!("{}.{}", tf_type, label));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// IO layer: the `require` command
// ---------------------------------------------------------------------------

fn load_catalog(presets_dir: &str, framework: &str) -> Result<Catalog, BoxErr> {
    let path = Path::new(presets_dir).join("catalogs").join(format!("{}.yaml", framework));
    if !path.exists() {
        // Helpful listing of what IS available.
        let dir = Path::new(presets_dir).join("catalogs");
        let mut known = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if let Some(n) = e.path().file_stem().and_then(|s| s.to_str()) {
                    known.push(n.to_string());
                }
            }
        }
        return Err(format!(
            "No catalog '{}' in {} (available: {}). Run `satz get-presets` to refresh.",
            framework,
            dir.display(),
            if known.is_empty() { "none".to_string() } else { known.join(", ") }
        )
        .into());
    }
    let text = crate::fsx::read_to_string(&path)?;
    Ok(serde_yaml::from_str(&text)
        .map_err(|e| format!("catalog {} does not parse: {}", path.display(), e))?)
}

/// Claims of every pack in the library, for remediation suggestions.
///
/// Reads the `.satz` packs directly — the language is the single source, so the
/// library view no longer depends on generated `.gen.claims.yaml` sidecars
/// existing (they only appear after a transpile). `.diff.satz` files are
/// adoption ledgers, not usable packs, and are skipped so a fork's ledger can
/// never masquerade as a provider.
fn load_library_satz_claims(presets_dir: &str) -> Result<Vec<(String, Claim)>, BoxErr> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(presets_dir)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.to_string_lossy().to_string();
            if !name.ends_with(".satz") || name.ends_with(".diff.satz") {
                continue;
            }
            let src = crate::fsx::read_to_string(&path)?;
            // A pack that does not parse is not a compliance error — the
            // transpile reports it. Skip it rather than fail the goal view.
            let Ok(file) = satz_core::satz::parse(&src) else { continue };
            let pack = file.estate.clone().unwrap_or_default();
            for c in &file.claims {
                out.push((pack.clone(), claim_from_decl(c)));
            }
        }
    }
    Ok(out)
}

/// The claims the estate actually pulled in, straight from the front end.
pub(crate) fn claims_from_frontend(
    packs: &[satz_core::pipeline::PackClaims],
) -> Vec<(String, Claim)> {
    packs
        .iter()
        .flat_map(|p| p.claims.iter().map(move |c| (p.pack.clone(), claim_from_decl(c))))
        .collect()
}

fn claim_from_decl(c: &satz_core::satz::ClaimDecl) -> Claim {
    Claim {
        framework: c.framework.clone(),
        framework_version: c.version.clone(),
        control: c.control.clone(),
        coverage: c.coverage.clone(),
        resources: c.resources.clone(),
        reason: c.reason.clone().unwrap_or_default(),
        manual_duties: c
            .duties
            .iter()
            .map(|(id, duty)| ManualDuty { id: id.clone(), duty: duty.clone() })
            .collect(),
        interpretation: c.interpretation.clone().unwrap_or_default(),
    }
}

/// Library view for remediation suggestions: the claims of every `.satz` pack in
/// the library. Generated `.claims.yaml` sidecars are gone — a pack's claims are
/// read from its source.
fn load_library_view(presets_dir: &str) -> Result<Vec<(String, Claim)>, BoxErr> {
    load_library_satz_claims(presets_dir)
}

/// The `require` command. Returns true if any technical control is unmet or a
/// claim is broken (caller exits non-zero — CI gate).
pub(crate) fn run_require(
    framework: &str,
    input: &Path,
    presets_dir: &str,
    included_claims: &[(String, Claim)],
    main_tf: &str,
) -> Result<bool, BoxErr> {
    let catalog = load_catalog(presets_dir, framework)?;
    let library_claims = load_library_view(presets_dir)?;
    let emitted = emitted_addresses(main_tf);
    let goals = resolve_goals(&catalog, &library_claims, included_claims, &emitted);

    println!(
        "\nrequire {} {} — goal view for {}\n",
        catalog.catalog, catalog.version, input.display()
    );

    let (mut sat, mut part, mut unmet, mut broken, mut dev) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for (id, goal) in &goals {
        let title = &catalog.controls[id].title;
        match goal {
            Goal::Satisfied { witnesses } => {
                sat += 1;
                println!("  ✓ {:5} {:45} — {}", id, title, witnesses.join(", "));
            }
            Goal::Partial { open_duties, contributes_only, .. } => {
                part += 1;
                let why = if *contributes_only {
                    "no implements claim included".to_string()
                } else {
                    format!("open duties: {}", open_duties.join(", "))
                };
                println!("  ◐ {:5} {:45} — {}", id, title, why);
            }
            Goal::ClaimBroken { missing, pack } => {
                broken += 1;
                println!(
                    "  ‼ {:5} {:45} — pack {} claims it but witnesses are missing: {}",
                    id, title, pack, missing.join(", ")
                );
            }
            Goal::Deviation { reasons, open_duties, .. } => {
                dev += 1;
                let why = reasons
                    .iter()
                    .map(|(pack, reason)| format!("({}) {}", pack, reason))
                    .collect::<Vec<_>>()
                    .join(" | ");
                let duties = if open_duties.is_empty() {
                    String::new()
                } else {
                    format!(" open: {}", open_duties.join(", "))
                };
                println!("  ⚠ {:5} {:45} — DEVIATION {}{}", id, title, why, duties);
            }
            Goal::Unmet { providers } => {
                unmet += 1;
                if providers.is_empty() {
                    println!("  ✗ {:5} {:45} — unmet (no pack in the library provides it)", id, title);
                } else {
                    println!("  ✗ {:5} {:45} — unmet. Provides: {}", id, title, providers.join(", "));
                }
            }
            Goal::Organizational => {
                println!("  ○ {:5} {:45} — organizational control (no IaC witness)", id, title);
            }
        }
    }

    println!(
        "\n{} satisfied, {} partial, {} deviation(s), {} unmet, {} broken claim(s). \
         Goal view judges the DECLARED estate; live verification is the evidence report.",
        sat, part, dev, unmet, broken
    );
    if dev > 0 {
        println!(
            "Deviations are disclosed decisions with a stated reason, not gaps — they do not fail this gate."
        );
    }
    Ok(unmet > 0 || broken > 0)
}

// ---------------------------------------------------------------------------
// Tests (pure layer only)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        serde_yaml::from_str(
            r#"
catalog: cis-gcp
version: "4.0"
controls:
  "2.1": { title: audit logging, automatability: technical }
  "2.2": { title: org sink, automatability: technical }
  "2.3": { title: retention, automatability: partial }
  "9.9": { title: policy doc, automatability: organizational }
"#,
        )
        .unwrap()
    }

    fn claim(pack: &str, control: &str, coverage: &str, resources: &[&str], duties: &[&str]) -> (String, Claim) {
        (
            pack.to_string(),
            Claim {
                framework: "cis-gcp".into(),
                framework_version: "4.0".into(),
                control: control.into(),
                coverage: coverage.into(),
                resources: resources.iter().map(|s| s.to_string()).collect(),
                reason: String::new(),
                manual_duties: duties
                    .iter()
                    .map(|d| ManualDuty { id: d.to_string(), duty: String::new() })
                    .collect(),
                interpretation: String::new(),
            },
        )
    }

    #[test]
    fn goal_view_classifies_all_five_states() {
        let lib = vec![
            claim("logsink", "2.1", "implements", &["a.b"], &[]),
            claim("logsink", "2.3", "contributes", &["a.b"], &["lock-it"]),
            claim("alerts", "2.2", "implements", &["x.y"], &[]),
        ];
        // The estate used the logsink pack only — so only its claims are in.
        let included: Vec<(String, Claim)> =
            lib.iter().filter(|(p, _)| p == "logsink").cloned().collect();
        let emitted: BTreeSet<String> = ["a.b".to_string()].into();

        let goals = resolve_goals(&catalog(), &lib, &included, &emitted);
        assert!(matches!(goals["2.1"], Goal::Satisfied { .. }));
        // 2.3: included, witnesses present, but contributes-only with an open duty
        assert!(matches!(goals["2.3"], Goal::Partial { .. }));
        // 2.2: not included, but the library knows a provider — the tactic suggestion
        match &goals["2.2"] {
            Goal::Unmet { providers } => assert_eq!(providers, &vec!["alerts".to_string()]),
            g => panic!("expected Unmet with provider, got {:?}", g),
        }
        assert!(matches!(goals["9.9"], Goal::Organizational));
    }

    /// The declared side of R7. `enforce` sits two levels down in
    /// `spec { rules { … } }`, which `extract_witness_attrs` deliberately cannot
    /// see, so enforcement needs its own extractor.
    #[test]
    fn declared_enforcement_reads_nested_enforce_per_policy() {
        let tf = r#"
resource "google_org_policy_policy" "on" {
  name = "organizations/1/policies/compute.managed.requireOsLogin"

  spec {
    rules {
      enforce = "TRUE"
    }
  }
}

resource "google_org_policy_policy" "off" {
  name = "organizations/1/policies/compute.managed.vmCanIpForward"

  spec {
    rules {
      enforce = "FALSE"
    }
  }
}

resource "google_org_policy_policy" "listy" {
  name = "organizations/1/policies/iam.allowedPolicyMemberDomains"

  spec {
    rules {
      values {
        allowed_values = [
          "C0example"
        ]
      }
    }
  }
}
"#;
        let d = declared_enforcement(tf);
        assert_eq!(d.get("google_org_policy_policy.on"), Some(&true));
        assert_eq!(d.get("google_org_policy_policy.off"), Some(&false));
        // A list constraint has no single boolean — comparing it would be worse
        // than not comparing it, so it must be absent rather than guessed.
        assert_eq!(d.get("google_org_policy_policy.listy"), None);
        assert_eq!(d.len(), 2);
    }

    /// The live side. Shape verified against the Org Policy API on a real org:
    /// `enforce` is a JSON BOOL live while HCL spells it "TRUE"/"FALSE".
    #[test]
    fn live_enforcement_reads_the_real_api_shape() {
        let on: serde_json::Value = serde_json::from_str(
            r#"{"name":"organizations/1/policies/compute.managed.requireOsLogin",
                "spec":{"rules":[{"enforce":true}],"etag":"x"}}"#,
        )
        .unwrap();
        assert_eq!(live_enforcement(&on), Some(true));

        let off: serde_json::Value =
            serde_json::from_str(r#"{"spec":{"rules":[{"enforce":false}]}}"#).unwrap();
        assert_eq!(live_enforcement(&off), Some(false));

        // legacy list constraint — no enforce field at all
        let listy: serde_json::Value = serde_json::from_str(
            r#"{"spec":{"rules":[{"values":{"allowedValues":["C0example"]}}]}}"#,
        )
        .unwrap();
        assert_eq!(live_enforcement(&listy), None);

        // several rules: ambiguous, so no verdict rather than a wrong one
        let many: serde_json::Value =
            serde_json::from_str(r#"{"spec":{"rules":[{"enforce":true},{"enforce":false}]}}"#).unwrap();
        assert_eq!(live_enforcement(&many), None);

        assert_eq!(live_enforcement(&serde_json::Value::Null), None);
    }

    /// The whole point of R7: a policy switched off in the console still EXISTS,
    /// so an existence-only check reports it verified. Declared and live must be
    /// compared as values.
    #[test]
    fn a_policy_that_exists_but_does_not_enforce_is_not_verified() {
        let tf = r#"
resource "google_org_policy_policy" "os_login" {
  name = "organizations/1/policies/compute.managed.requireOsLogin"

  spec {
    rules {
      enforce = "TRUE"
    }
  }
}
"#;
        let declared = declared_enforcement(tf);
        let live: serde_json::Value =
            serde_json::from_str(r#"{"spec":{"rules":[{"enforce":false}]}}"#).unwrap();
        let want = declared.get("google_org_policy_policy.os_login").copied();
        let got = live_enforcement(&live);
        assert_eq!(want, Some(true));
        assert_eq!(got, Some(false));
        assert_ne!(want, got, "this divergence is what the report must surface");
    }

    /// A fork that deliberately does not enforce a control must report a
    /// DISCLOSED DEVIATION, not a gap. An estate declares
    /// `compute.managed.requireOsLogin` with `enforce = "FALSE"` because a
    /// service needs metadata SSH keys — the resource exists, so the witness is
    /// emitted, and a copied `implements` claim would have read "satisfied".
    #[test]
    fn a_declared_deviation_is_a_finding_not_a_gap() {
        let mut dev = claim("cis_fork", "2.1", "deviates", &["audit.policy"], &["review"]);
        dev.1.reason = "service X needs metadata SSH keys".into();
        let lib = vec![dev.clone()];
        let emitted: BTreeSet<String> = ["audit.policy".to_string()].into();

        let goals = resolve_goals(&catalog(), &lib, &[dev], &emitted);
        match &goals["2.1"] {
            Goal::Deviation { reasons, witnesses, open_duties } => {
                assert_eq!(reasons[0].1, "service X needs metadata SSH keys");
                assert_eq!(witnesses, &vec!["audit.policy".to_string()]);
                assert_eq!(open_duties, &vec!["review".to_string()]);
            }
            g => panic!("expected Deviation, got {:?}", g),
        }
    }

    /// A deviation outranks the positive claim it contradicts — the estate-level
    /// `suppress` case, where the pack still claims the control but the estate
    /// removed the resource and says why.
    #[test]
    fn a_deviation_outranks_a_broken_positive_claim() {
        let packclaim = claim("cis", "2.1", "implements", &["audit.policy"], &[]);
        let mut dev = claim("acme", "2.1", "deviates", &[], &[]);
        dev.1.reason = "suppressed on purpose".into();
        let included = vec![packclaim.clone(), dev];
        let emitted: BTreeSet<String> = BTreeSet::new(); // suppressed → not emitted

        let goals = resolve_goals(&catalog(), &[packclaim], &included, &emitted);
        assert!(
            matches!(goals["2.1"], Goal::Deviation { .. }),
            "a reasoned deviation must not report as a broken claim, got {:?}",
            goals["2.1"]
        );
    }

    /// But a deviation still ships proof for whatever witnesses it DOES declare:
    /// if that estate later deletes the policy outright that is a different fact and
    /// must resurface rather than stay silently "deviated".
    #[test]
    fn a_deviation_whose_declared_witness_vanished_breaks() {
        let mut dev = claim("cis_fork", "2.1", "deviates", &["audit.policy"], &[]);
        dev.1.reason = "not enforcing on purpose".into();
        let goals = resolve_goals(&catalog(), &[dev.clone()], &[dev], &BTreeSet::new());
        assert!(matches!(goals["2.1"], Goal::ClaimBroken { .. }), "got {:?}", goals["2.1"]);
    }

    /// A `.local` fork declares the SAME pack name as its pristine twin but may
    /// claim different witnesses. Resolving inclusion by pack name would let the
    /// pristine claim be attributed to an estate running the fork — reporting a
    /// control satisfied by a resource the estate never emits, or broken when it
    /// is fine. Included claims therefore travel with the file that was used.
    #[test]
    fn a_fork_claims_for_itself_not_for_its_pristine_twin() {
        // Both files declare `pack cis`; the library holds both.
        let pristine = claim("cis", "2.1", "implements", &["audit.pristine"], &[]);
        let fork = claim("cis", "2.1", "implements", &["audit.fork"], &[]);
        let lib = vec![pristine, fork.clone()];
        // The estate used the fork, and emits exactly what the fork claims.
        let included = vec![fork];
        let emitted: BTreeSet<String> = ["audit.fork".to_string()].into();

        let goals = resolve_goals(&catalog(), &lib, &included, &emitted);
        match &goals["2.1"] {
            Goal::Satisfied { witnesses } => assert_eq!(witnesses, &vec!["audit.fork".to_string()]),
            g => panic!("expected Satisfied by the fork's own witness, got {:?}", g),
        }
    }

    /// A pack that claims a control whose witnesses are NOT in the emitted estate is
    /// a broken lemma — reported as such, never silently satisfied or merely unmet.
    #[test]
    fn missing_witnesses_are_a_broken_claim() {
        let lib = vec![claim("logsink", "2.1", "implements", &["gone.away"], &[])];
        let included = lib.clone();
        let emitted: BTreeSet<String> = BTreeSet::new();
        let goals = resolve_goals(&catalog(), &lib, &included, &emitted);
        match &goals["2.1"] {
            Goal::ClaimBroken { missing, pack } => {
                assert_eq!(missing, &vec!["gone.away".to_string()]);
                assert_eq!(pack, "logsink");
            }
            g => panic!("expected ClaimBroken, got {:?}", g),
        }
    }

    #[test]
    fn witness_attrs_ignore_nested_block_shadowing() {
        let hcl = r#"
resource "google_monitoring_alert_policy" "cis_central_2_8_firewall_rule" {
  display_name = "CIS 2.8 — VPC firewall rule changes (org-wide)"
  combiner = "OR"
  conditions {
    display_name = "Firewall rule changed"
    condition_matched_log {
      filter = "x"
    }
  }
}
"#;
        let attrs = extract_witness_attrs(hcl);
        let policy = &attrs["google_monitoring_alert_policy.cis_central_2_8_firewall_rule"];
        assert_eq!(
            policy.get("display_name").map(String::as_str),
            Some("CIS 2.8 — VPC firewall rule changes (org-wide)")
        );
    }

    #[test]
    fn addresses_parse_from_hcl() {
        let hcl = r#"
resource "google_logging_metric" "cis_central_2_5_project_ownership" {
  name = "x"
}
resource "google_storage_bucket" "org_audit_logs" {}
"#;
        let set = emitted_addresses(hcl);
        assert!(set.contains("google_logging_metric.cis_central_2_5_project_ownership"));
        assert!(set.contains("google_storage_bucket.org_audit_logs"));
        assert_eq!(set.len(), 2);
    }
}

// ---------------------------------------------------------------------------
// Evidence report: claims × declared estate × LIVE estate
// ---------------------------------------------------------------------------

/// Live verification result for one witness address.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) enum LiveState {
    /// Found in the live estate (the live identifier that matched).
    Verified(String),
    /// The declared estate emits it, but the live estate does not contain it.
    Missing,
    /// Present live, but not doing what the estate declares — an org policy that
    /// exists with `enforce = "TRUE"` in the config while the live policy has
    /// enforcement OFF. Existence was never the claim; this is the difference
    /// between "a resource is there" and "the control is in force".
    Diverged(String),
    /// No live check exists for this resource type (reason) — never faked.
    Unverifiable(String),
}

/// Identify a witness in the live estate: which CAI asset type to list and which
/// HCL attribute carries the matchable identifier.
/// Alert policies and notification channels have server-assigned names, so their
/// identifier is the display_name (read from CAI resource data).
fn live_matcher(tf_type: &str) -> Option<(&'static str, &'static str)> {
    match tf_type {
        "google_logging_organization_sink" => Some(("logging.googleapis.com/LogSink", "name")),
        "google_logging_metric" => Some(("logging.googleapis.com/LogMetric", "name")),
        "google_storage_bucket" => Some(("storage.googleapis.com/Bucket", "name")),
        "google_monitoring_alert_policy" => {
            Some(("monitoring.googleapis.com/AlertPolicy", "display_name"))
        }
        "google_monitoring_notification_channel" => {
            Some(("monitoring.googleapis.com/NotificationChannel", "display_name"))
        }
        // The emitted `name` is `organizations/<org>/policies/<constraint>`, which
        // is exactly the CAI asset name minus its `//service/` prefix.
        "google_org_policy_policy" => Some(("orgpolicy.googleapis.com/Policy", "name")),
        _ => None,
    }
}

/// Extract, per emitted resource address, the simple string attributes of its HCL
/// block (enough to know each witness's live identifier: name / display_name).
pub(crate) fn extract_witness_attrs(
    main_tf: &str,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut depth = 0usize;
    for line in main_tf.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("resource \"") {
            let mut parts = rest.split('"');
            let tf_type = parts.next().unwrap_or("");
            parts.next();
            let label = parts.next().unwrap_or("");
            current = Some(format!("{}.{}", tf_type, label));
            out.entry(current.clone().unwrap()).or_default();
            depth = 1;
            continue;
        }
        if current.is_none() {
            continue;
        }
        if t == "}" || t == "},"{
            depth = depth.saturating_sub(1);
            if depth == 0 {
                current = None;
            }
            continue;
        }
        // Top-level attrs only: a nested block (e.g. an alert policy's conditions)
        // may reuse attr names like display_name and must not shadow the resource's.
        if depth == 1 {
            if let (Some(addr), Some(eq)) = (&current, t.find(" = \"")) {
                let key = t[..eq].trim().to_string();
                let val = t[eq + 4..].trim_end_matches('"').to_string();
                // Only simple one-line string attrs; that's all the matchers need.
                if !key.contains(' ') && !val.contains('"') {
                    out.get_mut(addr).unwrap().insert(key, val);
                }
            }
        }
        if t.ends_with('{') {
            depth += 1;
        }
    }
    out
}

/// Live inventory relevant to the witnesses: for each needed CAI asset type, the set
/// of live identifiers (terminal name segment, plus displayName when present).
/// Live assets by type, keyed by every identifier they can be matched on, with
/// the asset's resource data attached.
///
/// The data used to be discarded, which capped live verification at "does a
/// resource with this identifier exist". For an org policy that is not the
/// control: a policy with enforcement OFF exists just as much as one with it on.
type Inventory = BTreeMap<String, BTreeMap<String, serde_json::Value>>;

async fn live_inventory(org_id: &str, asset_types: &BTreeSet<String>) -> Result<Inventory, BoxErr> {
    use google_cloud_asset_v1::client::AssetService;
    use google_cloud_asset_v1::model::ContentType;
    let client = AssetService::builder().build().await?;
    let mut out: Inventory = BTreeMap::new();
    for at in asset_types {
        let mut ids: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        use google_cloud_gax::paginator::ItemPaginator as _;
        let mut stream = client
            .list_assets()
            .set_parent(format!("organizations/{}", org_id))
            .set_asset_types(vec![at.clone()])
            .set_content_type(ContentType::Resource)
            .set_page_size(1000)
            .by_item();
        while let Some(asset) = stream.next().await {
            let asset: google_cloud_asset_v1::model::Asset = asset?;
            let data = asset
                .resource
                .as_ref()
                .and_then(|r| r.data.as_ref())
                .and_then(|d| serde_json::to_value(d).ok())
                .unwrap_or(serde_json::Value::Null);
            if let Some(seg) = asset.name.rsplit('/').next() {
                ids.insert(seg.to_string(), data.clone());
            }
            // CAI names are `//<service>/<path>`; the path is what an org policy's
            // emitted `name` attribute holds verbatim.
            if let Some(path) = asset.name.strip_prefix("//").and_then(|r| r.split_once('/')) {
                ids.insert(path.1.to_string(), data.clone());
            }
            // display_name lives in the resource data (alert policies, channels)
            if let Some(dn) = data.get("displayName").and_then(|d| d.as_str()) {
                ids.insert(dn.to_string(), data.clone());
            }
        }
        out.insert(at.clone(), ids);
    }
    Ok(out)
}

/// The single `enforce` value declared for an org-policy resource, if it declares
/// exactly one.
///
/// Deliberately conservative: a policy with several rules, or none, or one that
/// sets `values`/`allowed_values` instead (the legacy list constraints) has no
/// single boolean to compare, and a wrong comparison here would be worse than no
/// comparison at all. `extract_witness_attrs` cannot supply this — it stops at the
/// resource's top level so nested blocks can't shadow it, and `enforce` lives two
/// levels down in `spec { rules { … } }`.
pub(crate) fn declared_enforcement(main_tf: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut depth = 0usize;
    let mut found: Vec<bool> = Vec::new();
    for line in main_tf.lines() {
        let t = line.trim();
        if current.is_none() {
            if let Some(rest) = t.strip_prefix(r#"resource "google_org_policy_policy" ""#) {
                if let Some(label) = rest.split('\"').next() {
                    current = Some(format!("google_org_policy_policy.{}", label));
                    depth = t.matches('{').count() - t.matches('}').count();
                    found.clear();
                }
            }
            continue;
        }
        depth = depth + t.matches('{').count() - t.matches('}').count();
        if let Some(v) = t.strip_prefix("enforce") {
            let v = v.trim_start_matches([' ', '=']).trim().trim_matches('\"');
            match v.to_ascii_uppercase().as_str() {
                "TRUE" => found.push(true),
                "FALSE" => found.push(false),
                _ => {}
            }
        }
        if depth == 0 {
            if let (Some(addr), [only]) = (current.take(), found.as_slice()) {
                out.insert(addr, *only);
            }
            found.clear();
        }
    }
    out
}

/// The single `enforce` value the LIVE policy carries, if it carries exactly one.
/// Shape (verified against the Org Policy API): `spec.rules[].enforce` as a JSON
/// bool — note the live form is a boolean while HCL spells it "TRUE"/"FALSE".
pub(crate) fn live_enforcement(data: &serde_json::Value) -> Option<bool> {
    let rules = data.get("spec")?.get("rules")?.as_array()?;
    let mut found: Vec<bool> = rules.iter().filter_map(|r| r.get("enforce")?.as_bool()).collect();
    match found.len() {
        1 => found.pop(),
        _ => None,
    }
}

/// Attestations for manual duties, recorded by the customer:
/// `attestations.yaml` beside the tool config — duty-id -> {by, date, note}.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct Attestations {
    #[serde(flatten)]
    pub duties: BTreeMap<String, Attestation>,
}
#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub(crate) struct Attestation {
    pub by: String,
    pub date: String,
    #[serde(default)]
    pub note: String,
}

/// Tolerant Prowler-JSON ingest: per catalog control, PASS/FAIL counts from findings
/// whose compliance mapping references this framework. Unknown shapes are skipped —
/// corroboration must never fail the report.
pub(crate) fn ingest_prowler(
    raw: &serde_json::Value,
    catalog_name: &str,
    catalog_version: &str,
) -> BTreeMap<String, (usize, usize)> {
    let mut out: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let Some(findings) = raw.as_array() else { return out };
    let fw_needle = format!(
        "{}_{}",
        catalog_name.replace("-gcp", ""),
        catalog_version
    ); // "cis_4.0" matches prowler's "cis_4.0_gcp"
    for f in findings {
        let status = f
            .get("status")
            .or_else(|| f.get("Status"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let Some(compliance) = f.get("compliance").or_else(|| f.get("Compliance")) else {
            continue;
        };
        let Some(map) = compliance.as_object() else { continue };
        for (fw, controls) in map {
            if !fw.to_lowercase().contains(&fw_needle) {
                continue;
            }
            if let Some(list) = controls.as_array() {
                for c in list.iter().filter_map(|c| c.as_str()) {
                    let entry = out.entry(c.to_string()).or_default();
                    match status {
                        "PASS" => entry.0 += 1,
                        "FAIL" => entry.1 += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// The `report-compliance` command: the evidence run. Joins goal view × live
/// inventory × attestations × optional Prowler corroboration into an
/// auditor-shaped report, and appends the run to the evidence history.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_report_compliance(
    framework: &str,
    input: &Path,
    presets_dir: &str,
    included_claims: &[(String, Claim)],
    main_tf: &str,
    org_id: Option<&str>,
    config_dir: &Path,
    format: &str,
    report_path: Option<PathBuf>,
    prowler_path: Option<PathBuf>,
    no_live: bool,
) -> Result<(), BoxErr> {
    let catalog = load_catalog(presets_dir, framework)?;
    let library_claims = load_library_view(presets_dir)?;
    let emitted = emitted_addresses(main_tf);
    let goals = resolve_goals(&catalog, &library_claims, included_claims, &emitted);
    let attrs = extract_witness_attrs(main_tf);

    // ---- live verification (degrades to Unverifiable, never fails the report) ----
    let verified_at = chrono_free_timestamp();
    let mut live: BTreeMap<String, LiveState> = BTreeMap::new();
    let declared = declared_enforcement(main_tf);
    if !no_live {
        // Which asset types do the satisfied/partial witnesses need?
        let mut needed: BTreeSet<String> = BTreeSet::new();
        for goal in goals.values() {
            let ws = match goal {
                Goal::Satisfied { witnesses } | Goal::Partial { witnesses, .. } => witnesses,
                // A deviation states that a control is deliberately NOT met. That
                // is a claim about the live estate too: if the policy turns out to
                // be enforcing after all, the deviation is stale and the report
                // should say so rather than repeat last year's reason.
                Goal::Deviation { witnesses, .. } => witnesses,
                _ => continue,
            };
            for w in ws {
                if let Some(tf_type) = w.split('.').next() {
                    if let Some((at, _)) = live_matcher(tf_type) {
                        needed.insert(at.to_string());
                    }
                }
            }
        }
        let inventory = match org_id {
            Some(org) if !needed.is_empty() => match live_inventory(org, &needed).await {
                Ok(inv) => Some(inv),
                Err(e) => {
                    eprintln!("warning: live inventory unavailable ({}); report marks witnesses unverifiable", e);
                    None
                }
            },
            _ => None,
        };
        for goal in goals.values() {
            let ws = match goal {
                Goal::Satisfied { witnesses } | Goal::Partial { witnesses, .. } => witnesses,
                // A deviation states that a control is deliberately NOT met. That
                // is a claim about the live estate too: if the policy turns out to
                // be enforcing after all, the deviation is stale and the report
                // should say so rather than repeat last year's reason.
                Goal::Deviation { witnesses, .. } => witnesses,
                _ => continue,
            };
            for w in ws {
                let tf_type = w.split('.').next().unwrap_or("");
                let state = match live_matcher(tf_type) {
                    None => LiveState::Unverifiable(format!(
                        "no live check for {} yet (org IAM auditConfig etc. — roadmap)",
                        tf_type
                    )),
                    Some((at, attr)) => {
                        let id = attrs.get(w).and_then(|a| a.get(attr)).cloned();
                        match (&inventory, id) {
                            (None, _) => LiveState::Unverifiable(
                                if org_id.is_none() { "no customer-organization-id".into() }
                                else { "live inventory unavailable".into() },
                            ),
                            (Some(_), None) => LiveState::Unverifiable(format!(
                                "identifier attribute '{}' not extractable from HCL", attr
                            )),
                            (Some(inv), Some(id)) => match inv.get(at).and_then(|ids| ids.get(&id)) {
                                None => LiveState::Missing,
                                Some(data) => {
                                    // Existence is not the control. For an org
                                    // policy, compare what the estate DECLARES
                                    // against what the live policy actually does.
                                    match (declared.get(w).copied(), live_enforcement(data)) {
                                        (Some(want), Some(got)) if want != got => {
                                            LiveState::Diverged(format!(
                                                "declared enforce = {}, live policy has enforcement {}",
                                                if want { "TRUE" } else { "FALSE" },
                                                if got { "ON" } else { "OFF" }
                                            ))
                                        }
                                        (Some(_), Some(_)) => LiveState::Verified(id),
                                        // We declare an enforcement value but could
                                        // not read the live one. Reporting "verified"
                                        // here would be the exact dishonesty this
                                        // check exists to remove — existence is not
                                        // the control. Say we could not check.
                                        (Some(_), None) => LiveState::Unverifiable(
                                            "policy exists, but its live enforcement could not be read"
                                                .into(),
                                        ),
                                        // Nothing enforcement-shaped was declared
                                        // (list constraints, non-policy types):
                                        // existence IS the whole claim.
                                        (None, _) => LiveState::Verified(id),
                                    }
                                }
                            },
                        }
                    }
                };
                live.insert(w.clone(), state);
            }
        }
    }

    // ---- attestations + prowler ----
    let attestations: Attestations = {
        let p = config_dir.join("attestations.yaml");
        if p.exists() {
            serde_yaml::from_str(&crate::fsx::read_to_string(&p)?)
                .map_err(|e| format!("attestations.yaml does not parse: {}", e))?
        } else {
            Attestations::default()
        }
    };
    let prowler: BTreeMap<String, (usize, usize)> = match prowler_path {
        Some(p) => {
            let raw: serde_json::Value = serde_json::from_str(&crate::fsx::read_to_string(&p)?)
                .map_err(|e| format!("prowler json does not parse: {}", e))?;
            ingest_prowler(&raw, &catalog.catalog, &catalog.version)
        }
        None => BTreeMap::new(),
    };

    // ---- render ----
    let mut md = String::new();
    md.push_str(&format!(
        "# Evidence report — {} {}\n\nEstate: `{}` · run: {} · live verification: {}\n\n\
         > This report states **check semantics** (\"a resource with these properties was \
         verified at this time\"), never legal conformity. Satisfaction = claims ∧ manual \
         duties ∧ attestations.\n\n\
         | Control | Title | Status | Witnesses (declared → live) | Duties | Prowler |\n\
         |---|---|---|---|---|---|\n",
        catalog.catalog,
        catalog.version,
        input.display(),
        verified_at,
        if no_live { "off (--no-live)" } else { "Cloud Asset Inventory" },
    ));

    let mut json_rows = Vec::new();
    for (id, goal) in &goals {
        let control = &catalog.controls[id];
        let (status, witness_cell, duty_cell) = match goal {
            // A disclosed deviation is the one status an auditor most needs to
            // read, so it carries its reason into the report rather than being
            // flattened into "not met".
            Goal::Deviation { reasons, witnesses, open_duties } => {
                let why = reasons
                    .iter()
                    .map(|(pack, reason)| format!("{} — {}", pack, reason))
                    .collect::<Vec<_>>()
                    .join("<br>");
                let duties = if open_duties.is_empty() {
                    "–".to_string()
                } else {
                    open_duties.iter().map(|d| format!("open: {}", d)).collect::<Vec<_>>().join("; ")
                };
                let mut stale = false;
                let wcells: Vec<String> = witnesses
                    .iter()
                    .map(|w| match live.get(w) {
                        Some(LiveState::Diverged(d)) => {
                            stale = true;
                            format!("`{}` → **{}**", w, d)
                        }
                        Some(LiveState::Missing) => format!("`{}` → ✗ not live", w),
                        _ => format!("`{}` (declared, not enforcing)", w),
                    })
                    .collect();
                let cell = if wcells.is_empty() { why.clone() } else { format!("{}<br>{}", wcells.join("<br>"), why) };
                let status = if stale {
                    // The estate says "we deliberately do not meet this" while the
                    // live policy meets it. Not a failure — but the deviation and
                    // its reason are out of date and should be retired.
                    "**deviation is STALE**".to_string()
                } else {
                    "**deviation (accepted)**".to_string()
                };
                (status, cell, duties)
            }
            Goal::Satisfied { witnesses } | Goal::Partial { witnesses, .. } => {
                let mut wcells = Vec::new();
                let mut any_missing = false;
                let mut any_unver = false;
                let mut any_diverged = false;
                for w in witnesses {
                    match live.get(w) {
                        Some(LiveState::Verified(idn)) => wcells.push(format!("`{}` → ✓ `{}`", w, idn)),
                        Some(LiveState::Missing) => { any_missing = true; wcells.push(format!("`{}` → **✗ not live**", w)); }
                        Some(LiveState::Diverged(d)) => { any_diverged = true; wcells.push(format!("`{}` → **✗ {}**", w, d)); }
                        Some(LiveState::Unverifiable(r)) => { any_unver = true; wcells.push(format!("`{}` → – ({})", w, r)); }
                        None => wcells.push(format!("`{}` (declared)", w)),
                    }
                }
                let (open, attested): (Vec<_>, Vec<_>) = match goal {
                    Goal::Partial { open_duties, .. } => open_duties
                        .iter()
                        .partition(|d| !attestations.duties.contains_key(*d)),
                    _ => (Vec::new(), Vec::new()),
                };
                // NOT ENFORCED outranks DRIFTED: a resource that is missing is
                // visibly absent, but one that is present and switched off looks
                // healthy in every inventory. It is the more dangerous verdict.
                let status = if any_diverged { "**NOT ENFORCED**".to_string() }
                    else if any_missing { "**DRIFTED**".to_string() }
                    else if !open.is_empty() { "partial (open duty)".to_string() }
                    else if matches!(goal, Goal::Partial { contributes_only: true, .. }) { "partial (contributes)".to_string() }
                    else if any_unver { "verified*".to_string() }
                    else if no_live || live.is_empty() { "declared".to_string() }
                    else { "**verified**".to_string() };
                let duties = open.iter().map(|d| format!("open: {}", d))
                    .chain(attested.iter().map(|d| {
                        let a = &attestations.duties[*d];
                        format!("attested: {} ({}, {})", d, a.by, a.date)
                    }))
                    .collect::<Vec<_>>().join("; ");
                (status, wcells.join("<br>"), if duties.is_empty() { "–".into() } else { duties })
            }
            Goal::ClaimBroken { missing, pack } => (
                "**BROKEN CLAIM**".into(),
                format!("pack `{}` declares missing witnesses: {}", pack, missing.join(", ")),
                "–".into(),
            ),
            Goal::Unmet { providers } => (
                "**unmet**".into(),
                if providers.is_empty() { "no providing pack in library".into() }
                else { format!("Provides: {}", providers.join(", ")) },
                "–".into(),
            ),
            Goal::Organizational => ("organizational".into(), "no IaC witness".into(), "–".into()),
        };
        let prowler_cell = prowler
            .get(id)
            .map(|(p, f)| format!("{} PASS / {} FAIL", p, f))
            .unwrap_or_else(|| "–".into());
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            id, control.title, status, witness_cell, duty_cell, prowler_cell
        ));
        json_rows.push(serde_json::json!({
            "control": id, "title": control.title, "status": status.replace("**",""),
            "witnesses": witness_cell.replace("**","").replace('`',""),
            "duties": duty_cell, "prowler": prowler_cell,
        }));
    }

    let evidence = serde_json::json!({
        "framework": catalog.catalog, "version": catalog.version,
        "estate": input.display().to_string(), "verified_at": verified_at,
        "live": !no_live, "rows": json_rows,
    });

    // append-only history
    let hist_dir = config_dir.join("evidence");
    crate::fsx::create_dir_all(&hist_dir)?;
    let hist = hist_dir.join(format!("{}-{}.json", framework, verified_at.replace(':', "-")));
    crate::fsx::write(&hist, serde_json::to_string_pretty(&evidence)?.as_bytes())?;

    let out_path = report_path.unwrap_or_else(|| hist_dir.join(format!("{}-latest.md", framework)));
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&evidence)?),
        _ => {
            crate::fsx::write(&out_path, md.as_bytes())?;
            println!("Wrote evidence report to {} (history: {})", out_path.display(), hist.display());
            if format == "pdf" {
                crate::org_policy::try_pandoc_pdf(&out_path);
            }
        }
    }
    Ok(())
}

/// UTC timestamp without a chrono dependency (SystemTime → ISO-8601, minute precision).
pub(crate) fn chrono_free_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // days since epoch → civil date (Howard Hinnant's algorithm)
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m) = (rem / 3600, (rem % 3600) / 60);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}Z", y, mth, d, h, m)
}
