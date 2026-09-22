//! `tofu plan -generate-config-out`, run by satz for the live resources its own
//! mapping leaves unmatched.
//!
//! A Cloud Asset Inventory sweep finds resources satz cannot express as Satz: no
//! import-config row names the asset type, or nothing in the asset's data is in
//! the provider schema. They are reported as skipped and the estate does not
//! carry them. `satz import <scope> --generate-unmapped` hands exactly those to
//! the provider instead: one `import` block per resource in a scratch directory,
//! `tofu init`, `tofu plan -generate-config-out=generated.tf`, and the `.tf` the
//! provider writes goes back through the HCL import arm — the same arm that
//! reads `-generate-config-out` output an operator produced by hand.
//!
//! The import id is the asset's relative resource name, the derivation the
//! imported resources' own `"import-id"` uses: `tofu plan` on the import block
//! is what verifies it. What the provider refuses, satz reports — the child's
//! own output, the scratch directory it ran in and the `imports.tf` it wrote, so
//! the two commands can be finished by hand from there.
//!
//! An unmapped resource that cannot even get an import block is listed with the
//! reason: no Terraform type corresponds to its asset type, or its name is not a
//! Cloud Asset resource name.
//!
//! The child reads the platform as the identity the sweep read it as: the
//! provider block carries `impersonate_service_account` when the run is bound to
//! an estate's IaC service account (`--into`), and nothing when it is the
//! human's Application Default Credentials.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::discovery::{SkipReason, Skipped};
use crate::fsx;

/// One live resource an `import` block is written for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The Terraform resource type, as the import-config row names it.
    pub tf_type: String,
    /// The label of the address the block imports `to` — unique within the run.
    pub label: String,
    /// The provider's import id: the asset's relative resource name.
    pub import_id: String,
    /// The Cloud Asset full resource name, for the report and the comment above
    /// the block.
    pub what: String,
}

/// What the fallback will do, before anything runs.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub candidates: Vec<Candidate>,
    /// `(what, why)` — an unmapped resource no import block can be written for.
    pub refused: Vec<(String, String)>,
}

/// One entry of the scratch directory's `required_providers`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Provider {
    pub name: String,
    pub source: String,
    pub version: String,
}

/// How the child process runs. A trait so the plumbing around it is tested
/// without OpenTofu: the real one is `Tofu`.
pub(crate) trait Runner {
    /// Run the tool with `args` in `dir`. `Err` carries what the tool said —
    /// nothing is swallowed and nothing is retried.
    fn run(&mut self, dir: &Path, args: &[&str]) -> Result<(), String>;
}

/// The real child: `<tf_tool> <args>` in the scratch directory, inheriting the
/// environment — so it reads live as the same identity the sweep above it did.
pub(crate) struct Tofu<'a> {
    pub tool: &'a str,
}

impl Runner for Tofu<'_> {
    fn run(&mut self, dir: &Path, args: &[&str]) -> Result<(), String> {
        eprintln!("{} {} (in {})", self.tool, args.join(" "), dir.display());
        let out = std::process::Command::new(self.tool)
            .current_dir(dir)
            .args(args)
            .output()
            .map_err(|e| format!("could not run '{}': {}", self.tool, e))?;
        if out.status.success() {
            return Ok(());
        }
        // The provider reports a refused import id on stdout as often as on
        // stderr, and the operator needs the one that carries the address.
        let mut said = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stdout = stdout.trim_end();
        if !stdout.is_empty() {
            if !said.is_empty() {
                said.push('\n');
            }
            said.push_str(stdout);
        }
        Err(format!("`{} {}` failed in {}:\n{}", self.tool, args.join(" "), dir.display(), said))
    }
}

/// The Terraform label for a resource, from the last segment of its name, made
/// unique against what earlier candidates already took.
fn label(import_id: &str, taken: &mut BTreeSet<String>) -> String {
    let last = import_id.rsplit('/').next().unwrap_or(import_id);
    let mut base: String = last.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, '_');
    }
    let mut candidate = base.clone();
    let mut n = 2;
    while !taken.insert(candidate.clone()) {
        candidate = format!("{}_{}", base, n);
        n += 1;
    }
    candidate
}

/// Which skipped resources the provider can be asked to generate configuration
/// for, and why each of the others cannot.
///
/// Only `Unmapped` is a candidate: it is the mapping gap this fallback closes. A
/// type switched off (`import: false`, `--only`, `--exclude`), a platform-owned
/// object and a resource whose parent is outside the import were all left out on
/// purpose, and generating configuration for them would undo the instruction.
///
/// `known_type` answers whether the provider schema has that resource type. The
/// live sweep puts the Terraform type in `tf_type` where a row gave it one and
/// the Cloud Asset type where no row did — so the schema is what tells the two
/// apart, and the message says which of the two the reader is looking at.
pub(crate) fn plan(skipped: &[Skipped], known_type: &dyn Fn(&str) -> bool) -> Plan {
    let mut out = Plan::default();
    let mut taken = BTreeSet::new();
    for s in skipped {
        if !matches!(s.reason, SkipReason::Unmapped(_)) {
            continue;
        }
        let Some(import_id) = crate::discovery::asset_resource_name(&s.what) else {
            out.refused.push((
                s.what.clone(),
                format!("`{}` is not a Cloud Asset resource name, so there is no id to import by", s.what),
            ));
            continue;
        };
        if !known_type(&s.tf_type) {
            let why = if s.tf_type.contains('/') {
                format!("no import-config row names asset type {}, so there is no Terraform type to import as", s.tf_type)
            } else {
                format!("the provider schema does not know the Terraform type {}", s.tf_type)
            };
            out.refused.push((s.what.clone(), why));
            continue;
        }
        out.candidates.push(Candidate {
            label: label(import_id, &mut taken),
            tf_type: s.tf_type.clone(),
            import_id: import_id.to_string(),
            what: s.what.clone(),
        });
    }
    out
}

/// The scratch directory's only hand-written file: the providers to download and
/// one `import` block per candidate.
///
/// `impersonate` is the service account the run is bound to, and every provider
/// block carries it — the child then reads each resource as the principal the
/// sweep above it read the organisation as, rather than as whoever is logged in.
pub(crate) fn imports_tf(candidates: &[Candidate], providers: &[Provider], impersonate: Option<&str>) -> String {
    let mut out = String::from(
        "# Written by `satz import --generate-unmapped`: one import block per live resource\n\
         # satz's own mapping left unmatched. `tofu plan -generate-config-out=generated.tf`\n\
         # writes their configuration; `satz import generated.tf` reads it back as Satz.\n\n\
         terraform {\n  required_providers {\n",
    );
    for p in providers {
        out.push_str(&format!(
            "    {} = {{\n      source  = \"{}\"\n      version = \"{}\"\n    }}\n",
            p.name, p.source, p.version
        ));
    }
    out.push_str("  }\n}\n");
    for p in providers {
        match impersonate {
            Some(sa) => out.push_str(&format!(
                "\nprovider \"{}\" {{\n  impersonate_service_account = \"{}\"\n}}\n",
                p.name, sa
            )),
            None => out.push_str(&format!("\nprovider \"{}\" {{}}\n", p.name)),
        }
    }
    for c in candidates {
        out.push_str(&format!(
            "\n# {}\nimport {{\n  to = {}.{}\n  id = \"{}\"\n}}\n",
            c.what, c.tf_type, c.label, c.import_id
        ));
    }
    out
}

/// Where a run's fallback works and what it writes, both hung off the name of
/// the file the run is known by — the estate a plain sweep wrote, the scope's
/// top-level pack under `--into`.
///
/// `(scratch directory, the Satz file name)`: `<base>-generate/` beside the base,
/// and `<base>-generated.satz`, which lands where every other file of the run
/// does. One base, one pair of names, so a second scope imported into the same
/// estate neither overwrites the first nor plans in its directory.
pub(crate) fn output_names(base: &Path) -> (PathBuf, String) {
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("discovered");
    (base.with_file_name(format!("{}-generate", stem)), format!("{}-generated.satz", stem))
}

/// The file `tofu plan -generate-config-out` writes inside the scratch directory.
pub(crate) const GENERATED_TF: &str = "generated.tf";
/// The file satz writes there.
pub(crate) const IMPORTS_TF: &str = "imports.tf";

/// Write the scratch directory, run `init` and the generating `plan`, and hand
/// back the file the provider wrote.
///
/// Every step that fails fails the command: a fallback that reports success on a
/// plan it never got would leave the operator believing the estate is complete.
pub(crate) fn generate(
    work_dir: &Path,
    candidates: &[Candidate],
    providers: &[Provider],
    impersonate: Option<&str>,
    runner: &mut dyn Runner,
) -> Result<PathBuf, String> {
    if candidates.is_empty() {
        return Err("no resource to generate configuration for".to_string());
    }
    if providers.is_empty() {
        return Err("no provider to generate with — the provider schema names none for these types".to_string());
    }
    fsx::create_dir_all(work_dir).map_err(|e| format!("{}: {}", work_dir.display(), e))?;
    let generated = work_dir.join(GENERATED_TF);
    // OpenTofu refuses to overwrite the target of -generate-config-out, so a
    // second run in the same directory would fail on the first run's output.
    if generated.exists() {
        fsx::remove_file(&generated).map_err(|e| format!("{}: {}", generated.display(), e))?;
    }
    let imports = work_dir.join(IMPORTS_TF);
    fsx::write(&imports, imports_tf(candidates, providers, impersonate)).map_err(|e| format!("{}: {}", imports.display(), e))?;
    eprintln!(
        "generate-unmapped: {} import block(s) in {} — `init` downloads the provider once, then \
         `plan -generate-config-out` reads each resource …",
        candidates.len(),
        imports.display()
    );
    runner.run(work_dir, &["init", "-input=false"])?;
    runner.run(work_dir, &["plan", "-input=false", &format!("-generate-config-out={}", GENERATED_TF)])?;
    if !generated.exists() {
        return Err(format!(
            "the plan succeeded but wrote no {} in {} — nothing was generated",
            GENERATED_TF,
            work_dir.display()
        ));
    }
    Ok(generated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn skipped(tf_type: &str, what: &str, reason: SkipReason) -> Skipped {
        Skipped { tf_type: tf_type.into(), what: what.into(), reason }
    }

    fn google() -> Vec<Provider> {
        vec![Provider { name: "google".into(), source: "hashicorp/google".into(), version: "7.14.1".into() }]
    }

    /// A runner that records what it was asked to run and writes the file the
    /// provider would write — or fails at the step it is told to fail at.
    struct FakeTofu {
        calls: RefCell<Vec<String>>,
        dir: PathBuf,
        fail_at: Option<&'static str>,
        write_output: bool,
    }

    impl FakeTofu {
        fn new(dir: &Path) -> Self {
            Self { calls: RefCell::new(Vec::new()), dir: dir.to_path_buf(), fail_at: None, write_output: true }
        }
    }

    impl Runner for FakeTofu {
        fn run(&mut self, dir: &Path, args: &[&str]) -> Result<(), String> {
            assert_eq!(dir, self.dir, "the child runs in the scratch directory");
            self.calls.borrow_mut().push(args.join(" "));
            if let Some(step) = self.fail_at {
                if args[0] == step {
                    return Err(format!(
                        "`tofu {}` failed in {}:\nError: Cannot import non-existent remote object",
                        args.join(" "),
                        dir.display()
                    ));
                }
            }
            if args[0] == "plan" && self.write_output {
                std::fs::write(dir.join(GENERATED_TF), "resource \"google_dns_managed_zone\" \"corp\" {}\n").unwrap();
            }
            Ok(())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-genconfig-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// An unmapped type the provider schema knows gets an import block; the
    /// block's id is the asset's relative resource name.
    #[test]
    fn an_unmapped_type_the_schema_knows_becomes_an_import_block() {
        let skipped = [skipped(
            "google_dns_managed_zone",
            "//dns.googleapis.com/projects/acme-net/managedZones/corp",
            SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into()),
        )];
        let plan = plan(&skipped, &|t| t == "google_dns_managed_zone");
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.candidates,
            vec![Candidate {
                tf_type: "google_dns_managed_zone".into(),
                label: "corp".into(),
                import_id: "projects/acme-net/managedZones/corp".into(),
                what: "//dns.googleapis.com/projects/acme-net/managedZones/corp".into(),
            }]
        );
        let tf = imports_tf(&plan.candidates, &google(), None);
        assert!(tf.contains("to = google_dns_managed_zone.corp"), "{tf}");
        assert!(tf.contains("id = \"projects/acme-net/managedZones/corp\""), "{tf}");
        assert!(tf.contains("version = \"7.14.1\""), "{tf}");
        assert!(tf.contains("provider \"google\" {}"), "the plain ADC impersonates nobody:\n{tf}");
    }

    /// Bound to an estate (`--into`), the child reads as that estate's IaC
    /// service account: the provider block carries the impersonation, so one
    /// command reads the platform as one principal.
    #[test]
    fn a_bound_run_writes_the_impersonation_into_the_provider_block() {
        let plan = plan(
            &[skipped(
                "google_dns_managed_zone",
                "//dns.googleapis.com/projects/acme-net/managedZones/corp",
                SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into()),
            )],
            &|_| true,
        );
        let tf = imports_tf(&plan.candidates, &google(), Some("svc-iac-001@acme-infra-001.iam.gserviceaccount.com"));
        assert!(
            tf.contains("impersonate_service_account = \"svc-iac-001@acme-infra-001.iam.gserviceaccount.com\""),
            "{tf}"
        );
    }

    /// A type neither matched nor generatable is listed, with the reason. It is
    /// never silently dropped and never guessed at.
    #[test]
    fn a_type_with_no_terraform_name_is_listed_as_refused() {
        let skipped = [
            skipped(
                "aiplatform.googleapis.com/Dataset",
                "//aiplatform.googleapis.com/projects/1/locations/eu/datasets/9",
                SkipReason::Unmapped("no import-config row has asset_type aiplatform.googleapis.com/Dataset".into()),
            ),
            // the state shape names a resource by its Terraform label, which is
            // no live id: there is nothing to import by
            skipped("google_compute_network", "vpc", SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into())),
            skipped("google_pubsub_topic", "t", SkipReason::TypeOff),
            skipped("google_storage_bucket", "b", SkipReason::Filtered),
            skipped("google_logging_metric", "m", SkipReason::PlatformOwned("_Default".into())),
        ];
        let plan = plan(&skipped, &|t| t.starts_with("google_"));
        assert!(plan.candidates.is_empty(), "{:?}", plan.candidates);
        assert_eq!(plan.refused.len(), 2, "only the unmapped ones are this fallback's business: {:?}", plan.refused);
        assert!(plan.refused[0].1.contains("no import-config row names asset type aiplatform.googleapis.com/Dataset"), "{:?}", plan.refused);
        assert!(plan.refused[1].1.contains("is not a Cloud Asset resource name"), "{:?}", plan.refused);
    }

    /// Both shapes hang their files off the name of the file the run is known
    /// by: the estate a plain sweep wrote, the scope's top-level pack with
    /// `--into`. Two scopes into one estate therefore never share either.
    #[test]
    fn the_scratch_directory_and_the_generated_file_are_named_after_the_run() {
        let (dir, file) = output_names(Path::new("yaml/discovered.satz"));
        assert_eq!(dir, PathBuf::from("yaml/discovered-generate"));
        assert_eq!(file, "discovered-generated.satz");
        let (dir, file) = output_names(Path::new("yaml/imported-organizations-123456789012.satz"));
        assert_eq!(dir, PathBuf::from("yaml/imported-organizations-123456789012-generate"));
        assert_eq!(file, "imported-organizations-123456789012-generated.satz");
    }

    /// Two resources whose names end in the same segment get two addresses.
    #[test]
    fn labels_are_unique_within_a_run() {
        let reason = || SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into());
        let skipped = [
            skipped("google_dns_managed_zone", "//dns.googleapis.com/projects/a/managedZones/corp", reason()),
            skipped("google_dns_managed_zone", "//dns.googleapis.com/projects/b/managedZones/corp", reason()),
        ];
        let plan = plan(&skipped, &|_| true);
        let labels: Vec<&str> = plan.candidates.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["corp", "corp_2"]);
    }

    /// The happy path: the scratch directory is written, `init` and the
    /// generating `plan` run in it, and the provider's file comes back.
    #[test]
    fn the_child_runs_init_then_the_generating_plan() {
        let dir = scratch("happy");
        let plan_ = plan(
            &[skipped(
                "google_dns_managed_zone",
                "//dns.googleapis.com/projects/acme-net/managedZones/corp",
                SkipReason::Unmapped("x".into()),
            )],
            &|_| true,
        );
        let mut fake = FakeTofu::new(&dir);
        let out = generate(&dir, &plan_.candidates, &google(), None, &mut fake).expect("generated");
        assert_eq!(out, dir.join(GENERATED_TF));
        assert_eq!(
            *fake.calls.borrow(),
            vec!["init -input=false".to_string(), "plan -input=false -generate-config-out=generated.tf".to_string()]
        );
        let written = std::fs::read_to_string(dir.join(IMPORTS_TF)).unwrap();
        assert!(written.contains("import {"), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The child failing surfaces what the child said, and generates nothing.
    #[test]
    fn a_failing_child_surfaces_its_output() {
        let dir = scratch("failing");
        let plan_ = plan(
            &[skipped("google_dns_managed_zone", "//dns.googleapis.com/projects/a/managedZones/corp", SkipReason::Unmapped("x".into()))],
            &|_| true,
        );
        let mut fake = FakeTofu::new(&dir);
        fake.fail_at = Some("plan");
        let said = generate(&dir, &plan_.candidates, &google(), None, &mut fake).unwrap_err();
        assert!(said.contains("Cannot import non-existent remote object"), "{said}");
        assert!(!dir.join(GENERATED_TF).exists(), "nothing was generated");
        // the import blocks stay on disk: they are what the operator edits
        assert!(dir.join(IMPORTS_TF).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plan that exits 0 and writes nothing is a failure, not an empty success.
    #[test]
    fn a_plan_that_writes_nothing_is_an_error() {
        let dir = scratch("silent");
        let plan_ = plan(
            &[skipped("google_dns_managed_zone", "//dns.googleapis.com/projects/a/managedZones/corp", SkipReason::Unmapped("x".into()))],
            &|_| true,
        );
        let mut fake = FakeTofu::new(&dir);
        fake.write_output = false;
        let said = generate(&dir, &plan_.candidates, &google(), None, &mut fake).unwrap_err();
        assert!(said.contains("wrote no generated.tf"), "{said}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With nothing to generate, no child runs and no directory is written.
    #[test]
    fn nothing_to_generate_runs_nothing() {
        let dir = scratch("empty");
        let mut fake = FakeTofu::new(&dir);
        let said = generate(&dir, &[], &google(), None, &mut fake).unwrap_err();
        assert!(said.contains("no resource to generate configuration for"), "{said}");
        assert!(fake.calls.borrow().is_empty());
        assert!(!dir.exists(), "no scratch directory without a candidate");
    }
}
