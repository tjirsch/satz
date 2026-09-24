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
//! The import id is the one the mapped route writes as `"import-id"`
//! (`discovery::import_id`, ADR 0068): the asset's relative resource name with the
//! names the sweep read put where Cloud Asset has numbers, rendered through the
//! import-config row's `import_id` template where the row has one. An id that
//! derivation cannot build is refused by name; none is guessed.
//!
//! An unmapped resource that cannot even get an import block is listed with the
//! reason: no Terraform type corresponds to its asset type, or its name is not a
//! Cloud Asset resource name.
//!
//! The child reads the platform the way the estate the run writes reads it: the
//! provider block carries the quota project the estate's own `providers` block
//! gets, `user_project_override` with it, and `impersonate_service_account` when
//! the run is bound to an estate's IaC service account (`--as` / `--into`).
//!
//! A `plan` that exits non-zero having generated SOME of the resources keeps
//! what it generated (ADR 0065): `generated.tf` is the provider's own record of
//! what it could read, and [`outcomes`] says per candidate whether its
//! configuration is in that file, whether the provider reported it, and in the
//! provider's own words.

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
/// purpose, and generating configuration for them would undo the instruction. An
/// `Ambiguous` one is refused with its own reason: several Terraform types claim
/// its asset type, so there is no type to write an import block for.
///
/// `known_type` answers whether the provider schema has that resource type. The
/// live sweep puts the Terraform type in `tf_type` where a row gave it one and
/// the Cloud Asset type where no row did — so the schema is what tells the two
/// apart, and the message says which of the two the reader is looking at.
///
/// `id_of` is the import id of a skipped resource, or why there is none:
/// `Discovered::skipped_import_id`, the derivation every route shares.
pub(crate) fn plan(
    skipped: &[Skipped],
    known_type: &dyn Fn(&str) -> bool,
    id_of: &dyn Fn(&Skipped) -> Result<String, String>,
) -> Plan {
    let mut out = Plan::default();
    let mut taken = BTreeSet::new();
    for s in skipped {
        match &s.reason {
            SkipReason::Unmapped(_) => {}
            // No Terraform type was chosen, so there is no import block to write —
            // but the resource is live and unmanaged, which is what this report is
            // for, so it says so here instead of disappearing between the two.
            SkipReason::Ambiguous(why) => {
                out.refused.push((s.what.clone(), why.clone()));
                continue;
            }
            _ => continue,
        }
        if !known_type(&s.tf_type) {
            let why = if s.tf_type.contains('/') {
                format!("no import-config row names asset type {}, so there is no Terraform type to import as", s.tf_type)
            } else {
                format!("the provider schema does not know the Terraform type {}", s.tf_type)
            };
            out.refused.push((s.what.clone(), why));
            continue;
        }
        let import_id = match id_of(s) {
            Ok(id) => id,
            Err(why) => {
                out.refused.push((s.what.clone(), why));
                continue;
            }
        };
        out.candidates.push(Candidate {
            label: label(&import_id, &mut taken),
            tf_type: s.tf_type.clone(),
            import_id,
            what: s.what.clone(),
        });
    }
    out
}

/// How the child's provider blocks are configured — the same two things the
/// estate the run writes states in its own `providers` block.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildProvider<'a> {
    /// The service account the run is bound to (`--as` / `--into`), `None` on
    /// the caller's own Application Default Credentials.
    pub impersonate: Option<&'a str>,
    /// The project the child bills its calls to and asks for quota on: the one
    /// satz writes into the imported estate's `providers` block
    /// (`import::quota_project`). Organization-, folder- and billing-account-
    /// scoped reads name no project of their own, so without it the provider
    /// has none to send and Google refuses the call.
    pub quota_project: Option<&'a str>,
}

/// The scratch directory's only hand-written file: the providers to download and
/// one `import` block per candidate.
///
/// Every provider block is the estate's own: the quota project as `project` and
/// `billing_project` with `user_project_override`, so the child reads each
/// resource the way the estate satz just wrote reads it, and the impersonation
/// so it reads as the principal the sweep above it read the organisation as
/// rather than as whoever is logged in.
pub(crate) fn imports_tf(candidates: &[Candidate], providers: &[Provider], child: ChildProvider<'_>) -> String {
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
        let mut body = String::new();
        if let Some(project) = child.quota_project {
            body.push_str(&format!(
                "  project                     = \"{p}\"\n  billing_project             = \"{p}\"\n  user_project_override       = true\n",
                p = project
            ));
        }
        if let Some(sa) = child.impersonate {
            body.push_str(&format!("  impersonate_service_account = \"{}\"\n", sa));
        }
        if body.is_empty() {
            out.push_str(&format!("\nprovider \"{}\" {{}}\n", p.name));
        } else {
            out.push_str(&format!("\nprovider \"{}\" {{\n{}}}\n", p.name, body));
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
/// `(scratch directory, the Satz file)`: `<base>-generate/` and
/// `<base>-generated.satz`, both beside the base. One base, one pair of paths, so
/// a second scope imported into the same estate neither overwrites the first nor
/// plans in its directory.
pub(crate) fn output_names(base: &Path) -> (PathBuf, PathBuf) {
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("discovered");
    (base.with_file_name(format!("{}-generate", stem)), base.with_file_name(format!("{}-generated.satz", stem)))
}

/// Refused when the Satz file a `--generate-unmapped` run writes is already there:
/// it is the operator's to merge and edit, and a second run would replace it.
pub(crate) fn refuse_existing_output(base: &Path) -> Result<(), String> {
    let (_, file) = output_names(base);
    if file.exists() {
        return Err(format!(
            "{} exists — an earlier --generate-unmapped run wrote it, and this run would replace it. \
             Merge what you keep of it into the estate, delete it, and run again. Nothing was swept.",
            file.display()
        ));
    }
    Ok(())
}

/// The file `tofu plan -generate-config-out` writes inside the scratch directory.
pub(crate) const GENERATED_TF: &str = "generated.tf";
/// The file satz writes there.
pub(crate) const IMPORTS_TF: &str = "imports.tf";

/// What the child left behind.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Generated {
    /// The file the provider wrote, with the resources it could read.
    pub file: PathBuf,
    /// Its text, read once — [`outcomes`] judges against it.
    pub text: String,
    /// What the child said when it exited non-zero, `None` when it exited 0.
    /// The resources it names are refused; [`outcomes`] ties them to candidates.
    pub said: Option<String>,
}

/// Write the scratch directory, run `init` and the generating `plan`, and hand
/// back the file the provider wrote.
///
/// A `plan` that exits non-zero is not the end of the run (ADR 0065). The
/// provider reads the resources one by one and writes what it could read, so a
/// non-zero exit with a `generated.tf` beside it means SOME resources were
/// generated and the rest were reported; both halves are carried back and
/// [`outcomes`] says which resource is which. A non-zero exit with no file at
/// all, and a zero exit with no file, are failures — the fallback produced
/// nothing, and saying otherwise would leave the operator believing the estate
/// is complete. `init` failing is a failure too: no resource was ever read.
pub(crate) fn generate(
    work_dir: &Path,
    candidates: &[Candidate],
    providers: &[Provider],
    child: ChildProvider<'_>,
    runner: &mut dyn Runner,
) -> Result<Generated, String> {
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
    fsx::write(&imports, imports_tf(candidates, providers, child)).map_err(|e| format!("{}: {}", imports.display(), e))?;
    eprintln!(
        "generate-unmapped: {} import block(s) in {} — `init` downloads the provider once, then \
         `plan -generate-config-out` reads each resource …",
        candidates.len(),
        imports.display()
    );
    runner.run(work_dir, &["init", "-input=false"])?;
    let said = runner.run(work_dir, &["plan", "-input=false", &format!("-generate-config-out={}", GENERATED_TF)]).err();
    let text = match fsx::read_to_string(&generated) {
        Ok(t) if !generated_addresses(&t).is_empty() => t,
        // No file, or a file with no resource in it: the fallback produced
        // nothing, whatever the exit code said.
        _ => {
            return Err(match said {
                Some(e) => e,
                None => format!(
                    "the plan succeeded but wrote no resource into {} in {} — nothing was generated",
                    GENERATED_TF,
                    work_dir.display()
                ),
            })
        }
    };
    Ok(Generated { file: generated, text, said })
}

/// What became of one candidate once the child had run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Its configuration is in the generated file and the child said nothing
    /// against it.
    Written,
    /// Its configuration is in the generated file AND the child reported the
    /// address: generated, and not usable as it stands. The provider's words say
    /// what is missing.
    Incomplete(String),
    /// Nothing was generated for it, in the provider's words.
    Refused(String),
    /// Nothing was generated for it and nothing the child said names it. The
    /// resource is not in the estate and satz cannot say why.
    Unaccounted,
}

/// Per candidate, in the candidates' own order, what became of it — plus the
/// blocks of the child's output that name no candidate, which are printed whole
/// rather than dropped.
///
/// `generated` is the text of the file the provider wrote and `said` what it
/// printed when it exited non-zero. The two together, never the exit code
/// alone, decide: a resource counts as written because a `resource` block for
/// its address is in the file, and as refused because the child named that
/// address.
pub(crate) fn outcomes(candidates: &[Candidate], generated: &str, said: Option<&str>) -> (Vec<Outcome>, Vec<String>) {
    let written = generated_addresses(generated);
    let blocks = said.map(error_blocks).unwrap_or_default();
    let mut claimed = vec![false; blocks.len()];
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        let address = format!("{}.{}", c.tf_type, c.label);
        let mut reported: Vec<&str> = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            if names_resource(b, &c.tf_type, &c.label) {
                claimed[i] = true;
                reported.push(b);
            }
        }
        out.push(match (written.contains(&address), reported.is_empty()) {
            (true, true) => Outcome::Written,
            (true, false) => Outcome::Incomplete(reported.join("\n")),
            (false, false) => Outcome::Refused(reported.join("\n")),
            (false, true) => Outcome::Unaccounted,
        });
    }
    let rest = blocks.into_iter().zip(claimed).filter(|(_, c)| !c).map(|(b, _)| b).collect();
    (out, rest)
}

/// The addresses the provider wrote a `resource` block for. Generated
/// configuration puts each block's header at column zero, one per line.
fn generated_addresses(generated: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in generated.lines() {
        let Some(rest) = line.strip_prefix("resource \"") else { continue };
        let Some((tf_type, rest)) = rest.split_once('"') else { continue };
        let Some((_, rest)) = rest.split_once('"') else { continue };
        let Some((label, _)) = rest.split_once('"') else { continue };
        out.insert(format!("{}.{}", tf_type, label));
    }
    out
}

/// The child's output cut into the blocks it prints one per problem: a block
/// starts at a line whose text begins `Error:` — after the box-drawing prefix
/// OpenTofu frames its diagnostics with — and runs to the next one.
fn error_blocks(said: &str) -> Vec<String> {
    let starts_block = |line: &str| line.trim_start_matches(['│', '╷', '╵', ' ', '\t']).starts_with("Error:");
    let mut blocks: Vec<String> = Vec::new();
    for line in said.lines() {
        if starts_block(line) {
            blocks.push(String::new());
        }
        if let Some(current) = blocks.last_mut() {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line.trim_end());
        }
    }
    blocks.into_iter().map(|b| b.trim_end().to_string()).filter(|b| !b.is_empty()).collect()
}

/// Whether a diagnostic names this resource. The tool writes it two ways: as
/// the address (`with google_dns_record_set.www,`) when it is about the import,
/// and as the block header (`in resource "google_dns_record_set" "www":`) when
/// it is about the configuration that was generated for it.
fn names_resource(block: &str, tf_type: &str, label: &str) -> bool {
    block.contains(&format!("\"{}\" \"{}\"", tf_type, label)) || names_address(block, &format!("{}.{}", tf_type, label))
}

/// Whether a diagnostic names exactly this address. `google_dns_record_set.www`
/// is a prefix of `google_dns_record_set.www_2`, so the character after the
/// match has to end the identifier.
fn names_address(block: &str, address: &str) -> bool {
    let mut from = 0;
    while let Some(at) = block[from..].find(address) {
        let end = from + at + address.len();
        let after = block[end..].chars().next();
        if !matches!(after, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn skipped(tf_type: &str, what: &str, reason: SkipReason) -> Skipped {
        Skipped { tf_type: tf_type.into(), what: what.into(), reason }
    }

    fn unmapped(tf_type: &str, what: &str) -> Skipped {
        skipped(tf_type, what, SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into()))
    }

    fn google() -> Vec<Provider> {
        vec![Provider { name: "google".into(), source: "hashicorp/google".into(), version: "7.14.1".into() }]
    }

    fn no_names() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// The id of a skipped resource the way the run derives it
    /// (`Discovered::skipped_import_id`): the shipped rows' templates over the names
    /// the sweep read.
    fn ids(names: BTreeMap<String, String>) -> impl Fn(&Skipped) -> Result<String, String> {
        let cfg: crate::config::ImportConfig =
            serde_yaml::from_str(include_str!("../presets/import-config.yaml")).expect("the shipped table parses");
        move |s: &Skipped| {
            crate::discovery::import_id(
                &s.tf_type,
                &s.what,
                cfg.resource_types.get(&s.tf_type).and_then(|r| r.import_id.as_deref()),
                &serde_yaml::Mapping::new(),
                &names,
            )
        }
    }

    /// The sweep's reading of one DNS managed zone: Cloud Asset names it by the
    /// number, its data states the name.
    fn zone_names() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "//dns.googleapis.com/projects/acme-net/managedZones/1234567890".to_string(),
            "corp".to_string(),
        )])
    }

    /// A runner that records what it was asked to run and writes the file the
    /// provider would write — or fails at the step it is told to fail at.
    struct FakeTofu {
        calls: RefCell<Vec<String>>,
        dir: PathBuf,
        fail_at: Option<&'static str>,
        /// What the failing step says, when it is not the default refusal.
        says: Option<String>,
        write_output: bool,
        /// What `plan` writes as `generated.tf`.
        output: String,
    }

    impl FakeTofu {
        fn new(dir: &Path) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                dir: dir.to_path_buf(),
                fail_at: None,
                says: None,
                write_output: true,
                output: "resource \"google_dns_managed_zone\" \"corp\" {}\n".to_string(),
            }
        }
    }

    impl Runner for FakeTofu {
        fn run(&mut self, dir: &Path, args: &[&str]) -> Result<(), String> {
            assert_eq!(dir, self.dir, "the child runs in the scratch directory");
            self.calls.borrow_mut().push(args.join(" "));
            let failing = self.fail_at == Some(args[0]);
            // the real child writes what it could read before it reports the rest
            if args[0] == "plan" && self.write_output {
                std::fs::write(dir.join(GENERATED_TF), &self.output).unwrap();
            }
            if failing {
                return Err(self.says.clone().unwrap_or_else(|| {
                    format!(
                        "`tofu {}` failed in {}:\nError: Cannot import non-existent remote object",
                        args.join(" "),
                        dir.display()
                    )
                }));
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
        let skipped = [unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890")];
        let plan = plan(&skipped, &|t| t == "google_dns_managed_zone", &ids(zone_names()));
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.candidates,
            vec![Candidate {
                tf_type: "google_dns_managed_zone".into(),
                label: "corp".into(),
                import_id: "projects/acme-net/managedZones/corp".into(),
                what: "//dns.googleapis.com/projects/acme-net/managedZones/1234567890".into(),
            }]
        );
        let tf = imports_tf(&plan.candidates, &google(), ChildProvider::default());
        assert!(tf.contains("to = google_dns_managed_zone.corp"), "{tf}");
        assert!(tf.contains("id = \"projects/acme-net/managedZones/corp\""), "{tf}");
        assert!(tf.contains("version = \"7.14.1\""), "{tf}");
        assert!(tf.contains("provider \"google\" {}"), "with neither an identity nor a quota project the block is empty:\n{tf}");
    }

    /// Bound to an estate (`--as` / `--into`), the child reads as that estate's
    /// IaC service account and bills where the estate bills: one command reads
    /// the platform as one principal, against one quota project.
    #[test]
    fn the_child_provider_block_is_the_estates_own() {
        let plan = plan(&[unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890")], &|_| true, &ids(zone_names()));
        let tf = imports_tf(
            &plan.candidates,
            &google(),
            ChildProvider {
                impersonate: Some("svc-iac-001@acme-infra-001.iam.gserviceaccount.com"),
                quota_project: Some("acme-infra-001"),
            },
        );
        assert!(tf.contains("impersonate_service_account = \"svc-iac-001@acme-infra-001.iam.gserviceaccount.com\""), "{tf}");
        assert!(tf.contains("project                     = \"acme-infra-001\""), "{tf}");
        assert!(tf.contains("billing_project             = \"acme-infra-001\""), "{tf}");
        assert!(tf.contains("user_project_override       = true"), "{tf}");
    }

    /// A run that found no project has no quota project to name, and the block
    /// says so by carrying none rather than by naming one satz made up.
    #[test]
    fn without_a_quota_project_the_block_names_none() {
        let plan = plan(&[unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890")], &|_| true, &ids(zone_names()));
        let tf = imports_tf(&plan.candidates, &google(), ChildProvider { impersonate: Some("svc@acme-infra-001.iam.gserviceaccount.com"), quota_project: None });
        assert!(!tf.contains("billing_project"), "{tf}");
        assert!(tf.contains("impersonate_service_account"), "{tf}");
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
            unmapped("google_compute_network", "vpc"),
            skipped("google_pubsub_topic", "t", SkipReason::TypeOff),
            skipped("google_storage_bucket", "b", SkipReason::Filtered),
            skipped("google_logging_metric", "m", SkipReason::PlatformOwned("_Default".into())),
        ];
        let plan = plan(&skipped, &|t| t.starts_with("google_"), &ids(no_names()));
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
        assert_eq!(file, PathBuf::from("yaml/discovered-generated.satz"));
        let (dir, file) = output_names(Path::new("yaml/imported-organizations-123456789012.satz"));
        assert_eq!(dir, PathBuf::from("yaml/imported-organizations-123456789012-generate"));
        assert_eq!(file, PathBuf::from("yaml/imported-organizations-123456789012-generated.satz"));
        // `-o` outside yaml_dir: both land beside the file the run wrote
        let (dir, file) = output_names(Path::new("/srv/estates/acme.satz"));
        assert_eq!(dir, PathBuf::from("/srv/estates/acme-generate"));
        assert_eq!(file, PathBuf::from("/srv/estates/acme-generated.satz"));
    }

    /// A `-generated.satz` an earlier run wrote is the operator's: a second run is
    /// refused, naming it, rather than replacing what may have been edited.
    #[test]
    fn a_generated_file_already_there_is_refused_by_name() {
        let dir = scratch("existing");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("discovered.satz");
        refuse_existing_output(&base).expect("nothing there yet");
        std::fs::write(dir.join("discovered-generated.satz"), "estate edited {}\n").unwrap();
        let why = refuse_existing_output(&base).unwrap_err();
        assert!(why.contains(&dir.join("discovered-generated.satz").display().to_string()), "{why}");
        assert!(why.contains("Nothing was swept"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two resources whose names end in the same segment get two addresses.
    #[test]
    fn labels_are_unique_within_a_run() {
        let skipped = [
            unmapped("google_dns_policy", "//dns.googleapis.com/projects/a/policies/corp"),
            unmapped("google_dns_policy", "//dns.googleapis.com/projects/b/policies/corp"),
        ];
        let plan = plan(&skipped, &|_| true, &ids(no_names()));
        let labels: Vec<&str> = plan.candidates.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["corp", "corp_2"]);
    }

    /// The happy path: the scratch directory is written, `init` and the
    /// generating `plan` run in it, and the provider's file comes back with
    /// nothing said against it.
    #[test]
    fn the_child_runs_init_then_the_generating_plan() {
        let dir = scratch("happy");
        let plan_ = plan(&[unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890")], &|_| true, &ids(zone_names()));
        let mut fake = FakeTofu::new(&dir);
        let out = generate(&dir, &plan_.candidates, &google(), ChildProvider::default(), &mut fake).expect("generated");
        assert_eq!(out.file, dir.join(GENERATED_TF));
        assert_eq!(out.said, None);
        assert_eq!(
            *fake.calls.borrow(),
            vec!["init -input=false".to_string(), "plan -input=false -generate-config-out=generated.tf".to_string()]
        );
        let written = std::fs::read_to_string(dir.join(IMPORTS_TF)).unwrap();
        assert!(written.contains("import {"), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The child refusing some ids keeps what it generated for the rest: the
    /// file comes back with what the child said beside it, so the run reads 1
    /// resource back instead of throwing it away over the other 2.
    #[test]
    fn a_child_that_refused_some_ids_keeps_what_it_generated() {
        let dir = scratch("partial");
        let plan_ = plan(
            &[
                unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890"),
                unmapped("google_compute_instance_settings", "//compute.googleapis.com/projects/acme-net/zones/europe-west3-b/instanceSettings/InstanceSettings"),
                unmapped("google_cloud_asset_organization_feed", "//cloudasset.googleapis.com/organizations/123456789012/feeds/estate"),
            ],
            &|_| true,
            &ids(zone_names()),
        );
        assert_eq!(plan_.candidates.len(), 3);
        let mut fake = FakeTofu::new(&dir);
        fake.fail_at = Some("plan");
        fake.output = "resource \"google_dns_managed_zone\" \"corp\" {}\nresource \"google_cloud_asset_organization_feed\" \"estate\" {}\n".to_string();
        fake.says = Some(
            "╷\n│ Error: Cannot import non-existent remote object\n│ \n│   with google_compute_instance_settings.instancesettings,\n│   on imports.tf line 20:\n│   20: import {\n╵\n╷\n│ Error: Missing required argument\n│ \n│   on generated.tf line 3, in resource \"google_cloud_asset_organization_feed\" \"estate\":\n│    3: resource \"google_cloud_asset_organization_feed\" \"estate\" {\n│ \n│ The argument \"billing_project\" is required, but no definition was found.\n╵"
                .to_string(),
        );
        let out = generate(&dir, &plan_.candidates, &google(), ChildProvider::default(), &mut fake).expect("the generated file comes back");
        assert_eq!(out.file, dir.join(GENERATED_TF));
        let (outcomes, rest) = outcomes(&plan_.candidates, &out.text, out.said.as_deref());
        assert_eq!(outcomes[0], Outcome::Written, "the zone was read and written");
        let Outcome::Refused(why) = &outcomes[1] else { panic!("{:?}", outcomes[1]) };
        assert!(why.contains("Cannot import non-existent remote object"), "{why}");
        let Outcome::Incomplete(why) = &outcomes[2] else { panic!("{:?}", outcomes[2]) };
        assert!(why.contains("The argument \"billing_project\" is required"), "{why}");
        assert!(rest.is_empty(), "every block named a resource satz asked for: {rest:?}");
        // the import blocks stay on disk: they are what the operator edits
        assert!(dir.join(IMPORTS_TF).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A candidate the child neither generated for nor mentioned is said to be
    /// unaccounted for. The run never reports a resource as imported on the
    /// strength of having asked for it.
    #[test]
    fn a_candidate_the_child_never_mentions_is_unaccounted_for() {
        let candidates = plan(
            &[
                unmapped("google_dns_managed_zone", "//dns.googleapis.com/projects/acme-net/managedZones/1234567890"),
                unmapped("google_pubsub_topic", "//pubsub.googleapis.com/projects/acme-net/topics/events"),
            ],
            &|_| true,
            &ids(zone_names()),
        )
        .candidates;
        let (outcomes, rest) =
            outcomes(&candidates, "resource \"google_dns_managed_zone\" \"corp\" {}\n", Some("╷\n│ Error: something went wrong\n╵"));
        assert_eq!(outcomes, vec![Outcome::Written, Outcome::Unaccounted]);
        assert_eq!(rest.len(), 1, "a block naming no candidate is kept whole: {rest:?}");
        assert!(rest[0].contains("something went wrong"));
    }

    /// A label that is another label's prefix is not confused with it: the
    /// diagnostic about `corp_2` says nothing about `corp`.
    #[test]
    fn an_address_that_prefixes_another_is_not_confused_with_it() {
        let candidates = plan(
            &[
                unmapped("google_dns_policy", "//dns.googleapis.com/projects/a/policies/corp"),
                unmapped("google_dns_policy", "//dns.googleapis.com/projects/b/policies/corp"),
            ],
            &|_| true,
            &ids(no_names()),
        )
        .candidates;
        let said = "╷\n│ Error: Cannot import non-existent remote object\n│ \n│   with google_dns_policy.corp_2,\n╵";
        let (outcomes, _) = outcomes(&candidates, "", Some(said));
        assert_eq!(outcomes[0], Outcome::Unaccounted, "corp is not what the diagnostic names");
        assert!(matches!(outcomes[1], Outcome::Refused(_)), "{:?}", outcomes[1]);
    }

    /// A child that failed and generated nothing is a failure: there is nothing
    /// to read back, and the operator gets the child's own output.
    #[test]
    fn a_failing_child_that_generated_nothing_surfaces_its_output() {
        let dir = scratch("failing");
        let plan_ = plan(&[unmapped("google_dns_policy", "//dns.googleapis.com/projects/a/policies/corp")], &|_| true, &ids(no_names()));
        let mut fake = FakeTofu::new(&dir);
        fake.fail_at = Some("plan");
        fake.write_output = false;
        let said = generate(&dir, &plan_.candidates, &google(), ChildProvider::default(), &mut fake).unwrap_err();
        assert!(said.contains("Cannot import non-existent remote object"), "{said}");
        assert!(!dir.join(GENERATED_TF).exists(), "nothing was generated");
        // the import blocks stay on disk: they are what the operator edits
        assert!(dir.join(IMPORTS_TF).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `init` failing is the end of the run: no resource was ever read, so
    /// there is no partial result to keep.
    #[test]
    fn a_failing_init_is_the_end_of_the_run() {
        let dir = scratch("init");
        let plan_ = plan(&[unmapped("google_dns_policy", "//dns.googleapis.com/projects/a/policies/corp")], &|_| true, &ids(no_names()));
        let mut fake = FakeTofu::new(&dir);
        fake.fail_at = Some("init");
        let said = generate(&dir, &plan_.candidates, &google(), ChildProvider::default(), &mut fake).unwrap_err();
        assert!(said.contains("Cannot import non-existent remote object"), "{said}");
        assert_eq!(*fake.calls.borrow(), vec!["init -input=false".to_string()], "no plan runs after a failed init");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plan that exits 0 and generates nothing is a failure, not an empty
    /// success — whether it wrote no file or a file with no resource in it.
    #[test]
    fn a_plan_that_generates_nothing_is_an_error() {
        for (name, write_output) in [("silent", false), ("empty-file", true)] {
            let dir = scratch(name);
            let plan_ = plan(&[unmapped("google_dns_policy", "//dns.googleapis.com/projects/a/policies/corp")], &|_| true, &ids(no_names()));
            let mut fake = FakeTofu::new(&dir);
            fake.write_output = write_output;
            fake.output = "# __generated__ by OpenTofu\n".to_string();
            let said = generate(&dir, &plan_.candidates, &google(), ChildProvider::default(), &mut fake).unwrap_err();
            assert!(said.contains("wrote no resource into generated.tf"), "{name}: {said}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// With nothing to generate, no child runs and no directory is written.
    #[test]
    fn nothing_to_generate_runs_nothing() {
        let dir = scratch("empty");
        let mut fake = FakeTofu::new(&dir);
        let said = generate(&dir, &[], &google(), ChildProvider::default(), &mut fake).unwrap_err();
        assert!(said.contains("no resource to generate configuration for"), "{said}");
        assert!(fake.calls.borrow().is_empty());
        assert!(!dir.exists(), "no scratch directory without a candidate");
    }
}
