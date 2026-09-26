//! Which runs change the customer's ORGANISATION, and the one gate that stands in front
//! of them.
//!
//! A pack says what has to happen before its resources are applied, and how much it holds
//! back (`severity` on a `notice`, `src/notices.rs`). Whether a run is held back is not
//! the pack's business and not the command's either: it is one rule, here. `of` answers
//! it for EVERY command — the match is exhaustive, so a command added to the CLI does not
//! compile until it says which it is, and a reason is required for saying no. `refuse`
//! then asks the estate's packs and refuses while one of their messages is an open error.
//!
//! No command's own arm mentions a notice, a severity or a param. A command says what it
//! does to the organisation; this decides what that means.

use std::path::PathBuf;

use crate::findings::CompileRefusal;
use crate::{estate_path, Commands};
use crate::settings::{ToolConfig};

/// What a run does to the organisation it is pointed at.
#[derive(Debug, PartialEq)]
pub(crate) enum OrgWrite {
    /// It changes that organisation, and this estate's packs speak for it.
    Writes(PathBuf),
    /// It changes an organisation but is handed no estate, so no pack's message is
    /// reachable from here. The reason is the entry.
    NoEstate(&'static str),
    /// It changes no organisation: it reads, or it writes files. The reason is the entry.
    Reads(&'static str),
}

/// Every command, classified. A new one fails to compile until it is here — running
/// against a customer's organisation while a pack says the estate is not ready is a
/// wrong apply, and a wrong apply is visible in the organisation, not in a diff.
pub(crate) fn of(command: &Commands, runtime: &ToolConfig) -> OrgWrite {
    use OrgWrite::{NoEstate, Reads, Writes};
    let estate = |input: &str| Writes(estate_path(PathBuf::from(input), runtime));
    match command {
        // the two that create and change what the estate declares
        Commands::Transpile { input, apply: true, .. } => estate(input),
        Commands::Transpile { .. } => Reads("--check and --plan compile and propose; only --apply writes"),
        Commands::Bootstrap { estate: e, dry_run: false, .. } => Writes(estate_path(e.clone(), runtime)),
        Commands::Bootstrap { .. } => Reads("--dry-run prints the plan and creates nothing"),
        // `--mode cloud` grants the IaC service account Groups Admin on the organisation;
        // `--mode local` moves the state home and grants nothing. Without the flag the
        // target is the other of the two, which only the estate says: a run that may
        // write is one.
        Commands::Migrate { mode: Some(m), .. } if m == "local" => Reads("--mode local moves the state home and grants nothing"),
        Commands::Migrate { input, .. } => estate(input),
        // the estate's own deployment steps, run against the organisation by `--execute`
        Commands::RunActions { input, execute: true, .. } => estate(input),
        Commands::RunActions { .. } => Reads("without --execute it prints, or runs each action's dry-run form"),
        // `tofu apply` in hcl_dir is handed a directory, and the emitted HCL does not name
        // the estate that wrote it: the estate's messages are not reachable from here.
        // `transpile --apply` is the route that compiles first, and it is gated.
        Commands::Apply { .. } => NoEstate("it runs the tool in hcl_dir, which names no estate"),
        Commands::Plan { .. } | Commands::HclInit { .. } => Reads("the tool proposes, or initialises the backend"),
        // `adopt` is the command a notice asks for: it writes the tofu state and the
        // estate file, and `--activate` turns constraints on so they can be imported —
        // the step that closes a message, never the apply a message guards
        Commands::Adopt { .. } | Commands::AdoptOrgPolicies { .. } => {
            Reads("it writes the state and the estate; --activate turns constraints on so they can be imported")
        }
        // the live readers: Cloud Asset Inventory, the Policy API, Service Usage
        Commands::ExportOrganizationalPolicies { .. }
        | Commands::DiffOrganizationalPolicies { .. }
        | Commands::ReportOrganizationalPolicies { .. }
        | Commands::ReportCompliance { .. }
        | Commands::Whoami { .. }
        | Commands::MapTypes { .. } => Reads("it reads the organisation and writes nothing to it"),
        // the file-writers: an estate, a preset, a report, this machine's config
        Commands::Init { .. }
        | Commands::Import { .. }
        | Commands::UpdatePrerequisites { .. }
        | Commands::Interview { .. }
        | Commands::AddPack { .. }
        | Commands::AddProject { .. }
        | Commands::RemovePack { .. }
        | Commands::MergePresets { .. }
        | Commands::GetPresets { .. }
        | Commands::CheckPresets { .. }
        | Commands::DocPacks { .. }
        | Commands::PackGraph { .. }
        | Commands::ReviewPack { .. }
        | Commands::Packs { .. }
        | Commands::CheckConsumer { .. }
        | Commands::Questions { .. }
        | Commands::Require { .. }
        | Commands::Triage { .. }
        | Commands::RemediationPlan { .. }
        | Commands::Prowler { .. }
        | Commands::McpConfig { .. }
        | Commands::Scan { .. }
        | Commands::ScanPlan { .. }
        | Commands::GenerateMigration { .. }
        | Commands::UpdateSchema { .. }
        | Commands::Fmt { .. }
        | Commands::Silence { .. }
        | Commands::SelfUpdate { .. }
        | Commands::Completion { .. }
        | Commands::OpenReadme => Reads("it reads, and what it writes is a file"),
        // the two servers dispatch per call; each tool arrives as the command it serves
        Commands::Mcp { .. } | Commands::Lsp => Reads("a server serves many estates and is bound per call"),
    }
}

/// The gate. A run that writes to an estate's organisation is refused while a message
/// that estate's packs declare is open and an `error`; the refusal is the findings
/// themselves, so it prints where every other finding prints and at the line it names.
///
/// An estate that cannot be read is no pass: the caller is about to change an
/// organisation on the strength of that file.
pub(crate) fn refuse(command: &Commands, tool_config: &ToolConfig, runtime: &ToolConfig) -> Result<(), Box<dyn std::error::Error>> {
    let OrgWrite::Writes(estate) = of(command, runtime) else { return Ok(()) };
    // one spelling of the file, the compile's: a `[[silence]]` row written from a
    // compile's JSON answers to the same finding here
    let mut findings: Vec<_> = crate::notices::open_errors(&estate, runtime)?
        .into_iter()
        .map(|f| crate::estate_relative_file(f, runtime.dir.as_deref()))
        .collect();
    if findings.is_empty() {
        return Ok(());
    }
    let silences = crate::silence::in_force(tool_config);
    // an error is silenced by no tier, and a run that asks to silence one is told so
    silences.apply(&mut findings);
    crate::silence::refuse_run_silence_of_an_error(&silences, &findings)?;
    Err(Box::new(CompileRefusal { findings }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a run is classified as, for the command line that starts it.
    fn of_args(args: &[&str]) -> OrgWrite {
        let cfg: ToolConfig = toml::from_str("yaml_dir = \"yaml\"").unwrap();
        let mut argv = vec!["satz"];
        argv.extend_from_slice(args);
        let cli = <crate::Cli as clap::Parser>::try_parse_from(argv).expect("parses");
        of(&cli.command.expect("a subcommand"), &cfg)
    }

    /// The classification, on the commands the rule exists for: the two that were gated
    /// by hand, the two that write and never asked, and the command a notice names —
    /// which must never be held back by the message it closes.
    #[test]
    fn what_a_run_does_to_the_organisation_is_read_from_its_command_line() {
        let writes = |args: &[&str]| matches!(of_args(args), OrgWrite::Writes(_));
        assert!(writes(&["transpile", "e.satz", "--apply"]));
        assert!(!writes(&["transpile", "e.satz", "--plan"]));
        assert!(!writes(&["transpile", "e.satz", "--check"]));
        assert!(writes(&["bootstrap", "e.satz"]));
        assert!(!writes(&["bootstrap", "e.satz", "--dry-run"]));
        assert!(writes(&["run-actions", "e.satz", "--execute"]));
        assert!(!writes(&["run-actions", "e.satz", "--check"]));
        assert!(writes(&["migrate", "e.satz", "--mode", "cloud"]));
        assert!(!writes(&["migrate", "e.satz", "--mode", "local"]));
        assert!(writes(&["migrate", "e.satz"]), "the target is the estate's to say, so a run that may write is one");
        assert!(!writes(&["adopt", "e.satz", "--execute", "--import"]), "the command a notice names is never gated by it");
        assert!(!writes(&["report-compliance", "cis-gcp", "e.satz", "--format", "json", "--out", "r.json"]));
        assert_eq!(of_args(&["transpile", "e.satz", "--apply"]), OrgWrite::Writes(PathBuf::from("yaml/e.satz")));
        assert!(matches!(of_args(&["apply"]), OrgWrite::NoEstate(_)), "it is handed a directory, and says so");
    }

    /// The classification is only worth what its reasons are worth: a `Reads` with an
    /// empty reason is a command nobody thought about. And no arm may answer for a
    /// command it was not asked about — the match is exhaustive, which is what makes a
    /// new command fail to compile until it is classified, so this test reads the source
    /// for the wildcard that would defeat it.
    #[test]
    fn every_command_says_what_it_does_to_the_organisation() {
        let src = include_str!("org_write.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        assert!(
            !body.contains("_ =>"),
            "org_write::of answers a command with a wildcard arm — then a new command \
             writes to a customer's organisation without ever saying so"
        );
        for reason in ["Reads(\"\")", "NoEstate(\"\")"] {
            assert!(!body.contains(reason), "a command classified with no reason: {}", reason);
        }
    }
}
