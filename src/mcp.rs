//! `satz mcp` — the estate, over the Model Context Protocol.
//!
//! WHY A SERVER AND NOT A MODEL CLIENT
//! -----------------------------------
//! satz never calls a model. Judgment — grouping findings into workstreams,
//! wording a remediation for a customer, conducting an interview — happens in
//! whatever agent the operator already trusts, and that agent drives satz
//! through this surface. The binary stays deterministic, keyless and offline
//! by default; nothing here talks to a model, and nothing here needs an API key.
//!
//! WHY THE SDK RATHER THAN ~300 LINES OF JSON-RPC
//! ---------------------------------------------
//! The plan said hand-roll it: satz needs four methods and would never touch
//! the parts of the spec that move. Checking the spec before writing the first
//! line refuted that. The current revision (2026-07-28) negotiates the protocol
//! version PER REQUEST through a `_meta` key, adds a mandatory `server/discover`
//! RPC, and keeps a separate compatibility path for the initialize-based
//! revisions that clients still speak. That is three moving parts to own, in a
//! spec that has revised five times. `rmcp` implements all five revisions, so
//! the churn is someone else's — which was the deciding question the plan named.
//!
//! WHAT MAY BE DONE THROUGH IT
//! ---------------------------
//! Two axes, because "read-only" hides two different risks and a single
//! `--allow-write` is too coarse for a binary that can both write a file and
//! change an organisation:
//!
//!   read   compile and report — nothing leaves the estate, nothing is written
//!   write  writes files in the estate (hcl/, adopted ids, presets)
//!   exec   runs external tools or mutates the cloud
//!
//! `--allow` sets a CEILING the client cannot raise. With `--self-gated` the
//! client may LOWER its own level at runtime and never raise it again, so an
//! agent can prove it stayed read-only for a phase of its own work.
//!
//! `self-update` is not exposed at any level: it replaces the binary.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolResult, ContentBlock, ListResourcesResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};

use crate::ToolConfig;

/// What a tool is allowed to do. Not a severity ladder — three different kinds
/// of consequence, granted independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    /// compiles and reports; writes nothing, runs nothing
    Read,
    /// writes files inside the estate
    Write,
    /// Runs an external tool (Checkov), or changes a live organisation. An exec
    /// tool captures its child's output and gives it no stdin: here stdout is the
    /// protocol and stdin the request stream, so a child inheriting either
    /// corrupts the JSON-RPC session.
    Exec,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Read => "read",
            Group::Write => "write",
            Group::Exec => "exec",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Level {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
}

impl Level {
    /// Parse `read,write` into a level. An unknown group is an error rather than
    /// a silent no-op: a typo in a capability grant must never read as "less".
    pub(crate) fn parse(spec: &str) -> Result<Self, String> {
        let mut l = Level::default();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "read" => l.read = true,
                "write" => l.write = true,
                "exec" => l.exec = true,
                other => {
                    return Err(format!(
                        "unknown capability group '{}' — use read, write or exec (comma-separated)",
                        other
                    ));
                }
            }
        }
        if l == Level::default() {
            return Err("--allow named no group; use read, write or exec".into());
        }
        Ok(l)
    }

    fn allows(self, g: Group) -> bool {
        match g {
            Group::Read => self.read,
            Group::Write => self.write,
            Group::Exec => self.exec,
        }
    }

    fn within(self, ceiling: Self) -> bool {
        (!self.read || ceiling.read) && (!self.write || ceiling.write) && (!self.exec || ceiling.exec)
    }

    fn describe(self) -> String {
        let mut v = Vec::new();
        for (on, name) in [(self.read, "read"), (self.write, "write"), (self.exec, "exec")] {
            if on {
                v.push(name);
            }
        }
        if v.is_empty() { "nothing".into() } else { v.join(",") }
    }
}

/// The estate a session is working on, and everything that follows from it.
///
/// It follows from the estate rather than from the server because estates do not
/// share a `config.toml`: presets, schemas and the provider version are the
/// estate's own. A server pinned to one config could only ever serve estates
/// that agreed with it.
#[derive(Clone)]
struct Open {
    tool: ToolConfig,
    runtime: ToolConfig,
    estate: PathBuf,
    /// The service account this estate's live calls run as — derived exactly as
    /// the CLI and the emitted provider block derive it. `None` for a local-mode
    /// estate, which impersonates nothing.
    sa: Option<String>,
}

struct Ctx {
    /// every config and estate a tool resolves must stay under this directory
    root: PathBuf,
    /// what `satz_open` last opened; nothing until a client opens something
    open: Mutex<Option<Open>>,
}

/// How each CLI command reaches an agent: the MCP tool(s) that serve it, or the
/// reason it is not served. Exposing a command over MCP is a decision, and so is
/// not exposing one — `mcp_parity_is_decided` fails on a command this table does
/// not name, the way `IDENTITIES` fails on a command that declares no identity.
pub(crate) const MCP_PARITY: &[(&str, Parity)] = &[
    // --- served -------------------------------------------------------------
    ("transpile", Parity::Tools(&["satz_transpile", "satz_transpile_check"])),
    ("require", Parity::Tools(&["satz_require"])),
    ("report-compliance", Parity::Tools(&["satz_report_compliance"])),
    ("questions", Parity::Tools(&["satz_questions"])),
    ("interview", Parity::Tools(&["satz_interview"])),
    ("triage", Parity::Tools(&["satz_triage"])),
    ("prowler", Parity::Tools(&["satz_prowler"])),
    ("remediation-plan", Parity::Tools(&["satz_remediation_items", "satz_remediation_annotate"])),
    ("scan", Parity::Tools(&["satz_scan_checkov"])),
    ("check-presets", Parity::Tools(&["satz_check_presets"])),
    ("get-presets", Parity::Tools(&["satz_get_presets"])),
    ("adopt", Parity::Tools(&["satz_adopt"])),
    ("update-prerequisites", Parity::Tools(&["satz_update_prerequisites"])),
    ("review-pack", Parity::Tools(&["satz_review_pack"])),
    ("whoami", Parity::Tools(&["satz_whoami"])),
    ("merge-presets", Parity::Tools(&["satz_merge_presets"])),
    ("fmt", Parity::Tools(&["satz_fmt"])),
    // --- not served ---------------------------------------------------------
    ("lsp", Parity::Off("it is a server for editors, as `mcp` is for agents")),

    ("init", Parity::Off("`satz_interview` creates an estate from the skeleton; init derives from the credentials and runs as the human, before there is an estate")),
    ("bootstrap", Parity::Off("day 0: it creates the folder, project and state bucket as the human, after an interactive pre-flight")),
    ("plan", Parity::Off("it hands stdio to the tool; an agent runs tofu itself")),
    ("apply", Parity::Off("it hands stdio to the tool, approval prompt included")),
    ("hcl-init", Parity::Off("it hands stdio to the tool")),
    ("import", Parity::Off("the live sweep runs for minutes against the platform and rewrites the estate; nothing reports progress over this protocol, and a call that returns after ten silent minutes is a call a client has already given up on")),
    ("adopt-org-policies", Parity::Off("the alias also imports and activates; `satz_adopt` serves the resolution, the writing half stays with the human")),
    ("run-actions", Parity::Off("it runs the estate's deployment steps against the organisation")),
    ("export-organizational-policies", Parity::Off("it writes a preset from a live organisation; `satz_report_compliance` answers what an agent asks of live policy")),
    ("diff-organizational-policies", Parity::Off("the compliance plane compares policies by value over MCP; the specialist diff is a console report")),
    ("report-organizational-policies", Parity::Off("a rendered human report (markdown, PDF)")),
    ("migrate", Parity::Off("a one-off switch of deployment_mode — an estate edit")),
    ("update-schema", Parity::Off("it refreshes the provider schema cache: environment setup, not estate work")),
    ("map-types", Parity::Off("it derives type-map.yaml from the Discovery Documents — a maintainer refresh of shipped data")),
    ("scan-plan", Parity::Off("plan-JSON plumbing for a tofu workflow MCP does not drive")),
    ("generate-migration", Parity::Off("it writes a state-mv script for a human to read and run")),
    ("doc-packs", Parity::Off("it regenerates the pack pages in the repository; `--check` is a repository gate")),
    ("self-update", Parity::Off("it replaces the binary")),
    ("completion", Parity::Off("a shell affordance")),
    ("open-readme", Parity::Off("it opens a browser")),
    ("help", Parity::Off("clap prints it")),
    ("mcp", Parity::Off("this is the server")),
];

/// The satz command each tool stands for, so an agent that knows the CLI can
/// find the tool for what it wants: `transpile -> satz_transpile,
/// satz_transpile_check; require -> satz_require; …`.
fn served_by() -> String {
    let mut rows: Vec<String> = MCP_PARITY
        .iter()
        .filter_map(|(c, p)| match p {
            Parity::Tools(ts) => Some(format!("{} -> {}", c, ts.join(", "))),
            Parity::Off(_) => None,
        })
        .collect();
    rows.sort();
    rows.join("; ")
}

/// The commands an agent cannot run here, each with the reason: an agent that
/// knows what is missing asks for it instead of improvising a way around it
/// (writing HCL by hand because `apply` is absent, say). The terminal's own
/// affordances are left out — nothing an agent would reach for.
fn not_served() -> String {
    let mut rows: Vec<String> = MCP_PARITY
        .iter()
        .filter(|(c, _)| !matches!(*c, "completion" | "open-readme" | "self-update" | "help" | "mcp" | "fmt" | "lsp"))
        .filter_map(|(c, p)| match p {
            Parity::Off(why) => Some(format!("{} ({})", c, why)),
            Parity::Tools(_) => None,
        })
        .collect();
    rows.sort();
    rows.join("; ")
}

/// A command's MCP exposure: the tools that serve it, or why none does.
pub(crate) enum Parity {
    Tools(&'static [&'static str]),
    Off(&'static str),
}

/// Tools with no CLI command behind them: the session and capability plumbing
/// MCP needs and a terminal does not.
pub(crate) const MCP_ONLY: &[&str] = &["satz_open", "satz_estates", "satz_restrict"];

#[derive(Clone)]
pub(crate) struct SatzMcp {
    ctx: Arc<Ctx>,
    /// the level in force; `restrict` may only shrink it
    level: Arc<Mutex<Level>>,
    ceiling: Level,
    self_gated: bool,
    /// read by the `#[tool_handler]` expansion, which the lint cannot see
    #[allow(dead_code)]
    tool_router: ToolRouter<SatzMcp>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct EstateArg {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
}


/// Which questions `satz_interview` returns.
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum InterviewFilter {
    /// only what the estate has not decided — the worklist
    #[default]
    Unanswered,
    /// every question with its state — the decisions sheet
    All,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct InterviewArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// `unanswered` (default) or `all`
    #[serde(default)]
    pub filter: InterviewFilter,
    /// Write the estate if it does not exist yet: a skeleton that uses
    /// `presets/estate-core.satz` (the day-0 params with their questions), carries
    /// the security-model choice, and holds the same resources `satz init` writes.
    /// Needs the 'write' capability. An existing file is never touched.
    #[serde(default)]
    pub create: bool,
    /// Answers to write before reporting, keyed by the question's subject: a param's
    /// value, or for a `oneof` the chosen option's param name (its siblings are set
    /// false). Each is checked against a question the estate asks; one refused
    /// answer means nothing is written. Needs 'write'.
    #[serde(default)]
    pub answers: BTreeMap<String, serde_json::Value>,
    /// Also write every default the report offers — the answer a customer gives
    /// when they accept what the pack proposes. Needs 'write'.
    #[serde(default)]
    pub accept_defaults: bool,
}

/// The interview's view: the questions report, filtered, plus what this call did
/// to the file and where the file belongs once the customer id is known.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct InterviewReport {
    /// this call wrote the estate file
    pub created: bool,
    /// how many params this call wrote — answers given plus defaults accepted
    pub written: usize,
    /// The estate binds `customer_id` but the file is not named after it, as
    /// `init` would have named it. `git mv` to this and set the `estate` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_to: Option<String>,
    #[serde(flatten)]
    pub report: crate::questions::QuestionsReport,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct OpenArgs {
    /// The estate's `config.toml`, or the directory holding it
    pub config: String,
    /// The estate's main `.satz` file — absolute, or relative to that config's yaml_dir
    pub estate: String,
}

/// What opening actually resolved. Returned rather than assumed, because every
/// later call depends on it — including which service account the live tools run
/// as, which is the one thing an agent must never be wrong about.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct OpenReport {
    pub config: String,
    pub estate: String,
    /// `cloud` or `local`, as the estate declares it
    pub deployment_mode: Option<String>,
    /// The identity this estate's LIVE tools run as. Null when the estate
    /// impersonates nothing and the calls are the ADC identity itself.
    pub runs_as: Option<String>,
}

/// One estate found under the server's root.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EstateEntry {
    pub config: String,
    pub estate: String,
    pub deployment_mode: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EstatesReport {
    pub root: String,
    pub estates: Vec<EstateEntry>,
}


#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct RequireArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct TriageArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
    /// Prowler 5 OCSF export (`--output-formats json-ocsf`), a path under the server's root
    pub prowler: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct RemediationArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
    /// Prowler 5 OCSF export (`--output-formats json-ocsf`), a path under the server's root
    pub prowler: String,
    /// Also run Checkov over hcl_dir and join its findings — needs the 'exec'
    /// capability. It changes the dossier and its hash: author and render with the
    /// same choice.
    #[serde(default)]
    pub checkov: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct AnnotateArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
    /// Prowler 5 OCSF export, a path under the server's root — the one the items came from
    pub prowler: String,
    /// Whether the items were built with Checkov joined (needs 'exec' when true)
    #[serde(default)]
    pub checkov: bool,
    /// The run directory to write into, a path under the server's root; created when absent
    pub out: String,
    /// The dossier sha256 the values were written against, from `satz_remediation_items`
    pub dossier_sha256: String,
    /// Authored values per item id (`F-0001`). `authored_by` and `authored_at` are
    /// mandatory on every entry.
    pub items: BTreeMap<String, crate::dossier::AuthoredItem>,
}

/// The dossier's items: the worklist for the `[Authored]` columns.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct RemediationItems {
    pub framework: String,
    pub estate: String,
    /// What authored values must name to be accepted.
    pub dossier_sha256: String,
    pub summary: crate::dossier::Summary,
    pub items: Vec<crate::dossier::Item>,
    /// FAIL findings per Prowler check that map to no control of the framework
    pub prowler_unmapped: BTreeMap<String, usize>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct AnnotateReport {
    pub out: String,
    pub dossier_sha256: String,
    /// authored items on file after this call
    pub authored_items: usize,
    pub written: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct AdoptArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Resource types to adopt, e.g. `google_org_policy_policy`; empty means all
    #[serde(default)]
    pub only: Vec<String>,
    /// Write the verified ids into the estate as `"import-id"` — needs 'write'
    #[serde(default)]
    pub execute: bool,
}

/// What `satz_adopt` found, and with `execute` what it wrote.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct AdoptReport {
    pub estate: String,
    pub declared: usize,
    pub rows: Vec<crate::adopt::AdoptRow>,
    /// The counts line: to import, to move, already managed, …
    pub summary: String,
    /// Rows that did not answer — failed, unresolvable, ambiguous, no rule.
    /// `execute` is refused while any exist.
    pub unanswered: usize,
    /// Live objects the estate declares under two addresses. `execute` is
    /// refused while any exist.
    pub move_conflicts: Vec<MoveConflict>,
    /// The state could not be read, so no row says "already managed".
    pub state_note: Option<String>,
    /// With `execute`: the `"import-id"` lines written into the estate.
    pub written: Vec<String>,
    pub hints: Vec<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct MoveConflict {
    pub address: String,
    /// the address the estate still declares for the same live object
    pub also_declared: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct GetPresetsArgs {
    /// Overwrite packs the estate uses when upstream changed them; without it
    /// they are refused and left alone
    #[serde(default)]
    pub force: bool,
    /// A pristine library under the server's root to copy from instead of downloading
    #[serde(default)]
    pub pristine_dir: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct MergePresetsArgs {
    /// A pristine library under the server's root to compare against instead of
    /// downloading upstream
    #[serde(default)]
    pub pristine_dir: Option<String>,
    /// The estate whose `use` graph decides which packs are protected; the open
    /// estate when omitted
    #[serde(default)]
    pub estate: Option<String>,
    /// Report what would happen and write nothing
    #[serde(default)]
    pub report_only: bool,
    /// Take upstream in place for these pack stems instead of forking them —
    /// `all` for every pack that is merely behind
    #[serde(default)]
    pub adopt: Vec<String>,
}

/// Satz text in, canonical Satz text out. No path and no write: the client holds
/// the file, satz holds the layout.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct FmtArgs {
    /// The Satz source to format — the contents of the file, not its path
    pub text: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct FmtResult {
    /// the same Satz in the canonical layout
    pub formatted: String,
    /// false when the input was already formatted
    pub changed: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct ReviewPackArgs {
    /// The pack file to review, inside the server's root
    pub pack: String,
    /// Judge it inside this estate instead of a synthesised one
    #[serde(default)]
    pub against: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct PrerequisitesArgs {
    /// The estate to check; the open estate when omitted
    #[serde(default)]
    pub estate: Option<String>,
    /// List what is missing and write nothing. The default WRITES the missing
    /// roles and APIs into the estate file and re-checks it.
    #[serde(default)]
    pub report_only: bool,
}

/// What the estate's resource types oblige it to declare — the IaC service
/// account's roles and the infra project's APIs — and what was written into it.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PrerequisitesResult {
    pub report: crate::PrerequisitesReport,
    /// the lines written into the estate; empty under `report_only`, and empty
    /// when nothing was missing
    pub written: Vec<String>,
}

/// What a compile produced. The addresses are the estate's emitted resources —
/// the same set the compliance plane witnesses against.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct CompileSummary {
    pub estate: String,
    pub addresses: Vec<String>,
    /// the files `satz_transpile` wrote; empty for a check
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<String>,
    /// what the compile found and did not refuse on — the warnings and notes the
    /// CLI prints — as data, each at the file and line it names; empty when there is
    /// nothing to say
    pub findings: Vec<crate::findings::Finding>,
}

/// Checkov over the estate's emitted HCL: the counts, and each failed check with
/// the Satz block that declared the resource.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct ScanReport {
    pub estate: String,
    pub hcl_dir: String,
    pub checkov_version: String,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub resource_count: u64,
    pub findings: Vec<ScanFinding>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct ScanFinding {
    pub check_id: String,
    pub check_name: String,
    /// Terraform address, `google_storage_bucket.audit_logs`
    pub resource: String,
    /// the Satz file and line that declared the resource, when the compile knows it
    pub declared_at: Option<String>,
    pub guideline: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct ReportComplianceArgs {
    /// Estate file, e.g. `C0example.satz`. Omit to use the open estate
    #[serde(default)]
    pub estate: Option<String>,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
    /// Prowler export to corroborate with, a path under the server's root
    #[serde(default)]
    pub prowler: Option<String>,
    /// Skip the live Cloud Asset Inventory read and judge the declared estate only
    #[serde(default)]
    pub no_live: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct WhoamiArgs {
    /// Read the ADC file only — no network, no token minted
    #[serde(default)]
    pub offline: bool,
    /// Estate file to answer FOR: a cloud-mode estate reports its IaC service
    /// account, which is the identity its live tools actually run as. Omit to
    /// report the ambient credentials the server itself falls back to.
    #[serde(default)]
    pub estate: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct RestrictArgs {
    /// Groups to keep, comma-separated: read, write, exec
    pub allow: String,
}

/// What an agent has to read before it can WRITE Satz.
///
/// A client that only speaks MCP has no filesystem and no repository — the tool
/// schemas tell it how to CALL satz, and nothing tells it how to write the
/// language the calls are about. These are compiled into the binary so the
/// answer travels with the server: no path to configure, no version to drift.
const GUIDE: &str = include_str!("../docs/llms.md");
const REFERENCE: &str = include_str!("../docs/language.md");
const PRESETS: &str = include_str!("../presets/README.md");

struct Doc {
    uri: &'static str,
    name: &'static str,
    description: &'static str,
    text: &'static str,
    /// Serve `text` only down to this marker. `presets/README.md` closes with the
    /// pack changelog: version history an agent reading "how do I use a pack"
    /// never needs, and a quarter of the resource.
    trim_at: Option<&'static str>,
}

impl Doc {
    fn body(&self) -> &'static str {
        match self.trim_at {
            // `every_trimmed_resource_finds_its_marker` proves every marker is
            // present, so a missing one is a broken build, not a runtime case.
            Some(m) => self.text.split_once(m).expect("the trim marker is asserted by a test").0,
            None => self.text,
        }
    }
}

const DOCS: &[Doc] = &[
    Doc {
        uri: "satz://guide",
        name: "satz for llms",
        description: "How to write Satz: the estate shape, params, hierarchy, the three grant \
                      forms, packs, claims, questions, and the order to call the tools in. Read \
                      this before writing or editing a .satz file.",
        text: GUIDE,
        trim_at: None,
    },
    Doc {
        uri: "satz://reference",
        name: "Satz language reference",
        description: "The complete language reference — every construct, with the errors each \
                      one raises. Consult it when the guide does not cover a case.",
        text: REFERENCE,
        trim_at: None,
    },
    Doc {
        uri: "satz://presets",
        name: "The preset library",
        description: "What each shipped pack contains, how provenance by suffix works \
                      (pristine / .local fork / .diff ledger), and the conventions a pack follows.",
        text: PRESETS,
        trim_at: Some("\n## Changelog\n"),
    },
];

/// A refusal an agent can recover from. The plan's rule: a tool the level does
/// not permit is a tool RESULT with `isError`, never a protocol error — clients
/// retry the former and give up on the latter.
fn refused(msg: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg)])
}

/// A refusal that also hands over what the compile found, so a client can show
/// each error at its own line instead of parsing the text. The text is the one
/// `refused` carries; the structure is a `CompileSummary` with nothing emitted,
/// so it conforms to the schema the tool publishes.
fn refused_with_findings(msg: String, estate: &std::path::Path, e: &(dyn std::error::Error + 'static)) -> CallToolResult {
    let findings = crate::findings::refusal_findings(e);
    let mut result = refused(msg);
    if !findings.is_empty() {
        let summary = CompileSummary {
            estate: estate.display().to_string(),
            addresses: Vec::new(),
            written: Vec::new(),
            findings,
        };
        result.structured_content = serde_json::to_value(summary).ok();
    }
    result
}

/// Every `config.toml` under `root`, depth-limited and blind to the directories
/// that never hold one. A fleet root is somebody's home directory in the worst
/// case; this walk has to stay cheap and finite.
fn find_configs(root: &std::path::Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || out.len() >= 200 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            // hcl/ is output, target/ is a build, evidence/ is a report history:
            // none of them holds an estate, and all of them are large.
            if !matches!(name.as_str(), "hcl" | "target" | "evidence" | "node_modules") {
                dirs.push(path);
            }
        } else if name == "config.toml" {
            out.push(path);
        }
    }
    dirs.sort();
    for d in dirs {
        find_configs(&d, depth - 1, out);
    }
}

/// Whether a `.satz` file is an ESTATE rather than a pack or a fragment. The
/// statement is the definition, so read for it instead of guessing from a name.
fn declares_an_estate(path: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("estate ") || l == "estate"
    })
}

// Every handler returns `Json<T>` on success. The SDK puts that in the result's
// `structuredContent` and publishes T's schema as the tool's `outputSchema`, so a
// client gets a typed value instead of a string it has to parse. Returning the
// report as a text block — what this server first did — threw that away: the JSON
// was there, but nothing said what shape it had.

#[tool_router]
impl SatzMcp {
    pub(crate) fn new(root: PathBuf, ceiling: Level, self_gated: bool) -> Self {
        Self {
            ctx: Arc::new(Ctx { root, open: Mutex::new(None) }),
            level: Arc::new(Mutex::new(ceiling)),
            ceiling,
            self_gated,
            tool_router: Self::tool_router(),
        }
    }

    fn permits(&self, g: Group) -> Result<(), CallToolResult> {
        let level = *self.level.lock().expect("the level lock is never poisoned");
        if level.allows(g) {
            return Ok(());
        }
        Err(refused(format!(
            "this server is running at level '{}' and the tool needs '{}'. \
             Start `satz mcp --allow {}` to grant it.",
            level.describe(),
            g.name(),
            g.name()
        )))
    }

    /// Resolve an estate argument and refuse anything outside the root. Without
    /// this a tool argument is an arbitrary-file read: `use "…"` resolves
    /// through include_dirs, so a path is not just a path.
    /// The estate a call works on, and the configuration it works under.
    ///
    /// `None` means the one that is open — which is the normal case, and the
    /// point of opening: an agent working through a fleet names the estate once.
    /// A name still resolves inside the OPEN estate's config, because a
    /// `config.toml` is what gives a bare name a meaning.
    fn target(&self, name: Option<&str>) -> Result<(Open, PathBuf), CallToolResult> {
        let open = self.opened()?;
        let estate = match name {
            None => open.estate.clone(),
            Some(n) => {
                let p = crate::estate_path(PathBuf::from(n), &open.runtime);
                self.confine(p)?
            }
        };
        Ok((open, estate))
    }

    /// What is open, or the refusal that says how to open something. An agent
    /// reads the message and recovers; that is why it is a tool result.
    fn opened(&self) -> Result<Open, CallToolResult> {
        self.ctx
            .open
            .lock()
            .expect("the open lock is never poisoned")
            .clone()
            .ok_or_else(|| {
                refused(
                    "no estate is open. Call `satz_open` with the estate's config.toml and its \
                     main .satz file first — `satz_estates` lists what is available under this \
                     server's root."
                        .to_string(),
                )
            })
    }

    /// Any other path argument — a Prowler export, a report to read back.
    fn file(&self, name: &str) -> Result<PathBuf, CallToolResult> {
        self.confine(self.ctx.root.join(name))
    }

    /// Bind the identity before any LIVE call, exactly as the CLI does at
    /// dispatch: a cloud-mode estate is read as its own IaC service account,
    /// not as the human who started the server.
    ///
    /// Without this the same code answered differently depending on how it was
    /// reached — `report-compliance` on the command line ran as the service
    /// account, `satz_report_compliance` here ran as the human.
    ///
    /// The identity is SCOPED to the call rather than bound to the process:
    /// this server works through estates in turn, and a process-wide binding
    /// could only ever be right for the first one. Nothing is configured — the
    /// account is derived from the estate, the way the emitted provider block
    /// derives it, and the ADC mints it.
    fn identity_of(open: &Open) -> Option<String> {
        open.sa.clone()
    }

    fn confine(&self, p: PathBuf) -> Result<PathBuf, CallToolResult> {
        let resolved = p
            .canonicalize()
            .map_err(|e| refused(format!("{}: {}", p.display(), e)))?;
        let root = self
            .ctx
            .root
            .canonicalize()
            .map_err(|e| refused(format!("server root {}: {}", self.ctx.root.display(), e)))?;
        if !resolved.starts_with(&root) {
            return Err(refused(format!(
                "{} is outside the server's root ({}) — refused",
                resolved.display(),
                root.display()
            )));
        }
        Ok(resolved)
    }

    /// The manifest and claims every compliance tool starts from.
    fn inputs(
        &self,
        open: &Open,
        estate: &std::path::Path,
    ) -> Result<crate::ComplianceInputs, CallToolResult> {
        crate::compliance_inputs(estate, &open.tool, &open.runtime)
            .map_err(|e| refused(format!("{}: {}", estate.display(), e)))
    }

    #[tool(
        name = "satz_open",
        output_schema = rmcp::handler::server::tool::schema_for_output::<OpenReport>(),
        description = "Open an estate for this session: its `config.toml` and its main `.satz` \
                       file. Every later tool then works on that estate, under that config — its \
                       presets, its schemas, its provider version. Call it again to move to the \
                       next estate; a server serves a fleet, one estate at a time. The answer \
                       states which service account the estate's live tools will run as.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
    ) -> Result<Result<Json<OpenReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let given = PathBuf::from(&args.config);
        let candidate = if given.is_absolute() { given } else { self.ctx.root.join(given) };
        let candidate = match self.confine(candidate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let config = if candidate.is_dir() { candidate.join("config.toml") } else { candidate };
        if !config.is_file() {
            return Ok(Err(refused(format!(
                "{} is not a config.toml — pass the estate's config.toml, or the directory \
                 holding it",
                config.display()
            ))));
        }
        let tool = match crate::parse_tool_config(&config) {
            Ok(c) => c,
            Err(described) => return Ok(Err(refused(described))),
        };
        let dir = config.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let runtime = crate::resolved_config(&tool, &dir);

        let estate = crate::estate_path(PathBuf::from(&args.estate), &runtime);
        let estate = match self.confine(estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        if !estate.is_file() {
            return Ok(Err(refused(format!("no estate file at {}", estate.display()))));
        }

        // Derived here, once, and carried by everything the session does next —
        // never configured, and never asked for. The estate says who it is.
        let sa = crate::estate_impersonation_target(&estate, &runtime);
        let report = OpenReport {
            config: config.display().to_string(),
            estate: estate.display().to_string(),
            deployment_mode: crate::estate_param(&estate, &runtime, "deployment-mode"),
            runs_as: sa.clone(),
        };
        *self.ctx.open.lock().expect("the open lock is never poisoned") =
            Some(Open { tool, runtime, estate, sa });
        Ok(Ok(Json(report)))
    }

    #[tool(
        name = "satz_estates",
        output_schema = rmcp::handler::server::tool::schema_for_output::<EstatesReport>(),
        description = "Which estates this server can open: every `config.toml` under its root, \
                       with the estate files beside it. Read this before `satz_open` rather than \
                       guessing a path.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn estates(&self) -> Result<Result<Json<EstatesReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let mut configs = Vec::new();
        find_configs(&self.ctx.root, 5, &mut configs);
        configs.sort();
        let mut estates = Vec::new();
        for config in configs {
            let Ok(tool) = crate::parse_tool_config(&config) else { continue };
            let dir = config.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
            let runtime = crate::resolved_config(&tool, &dir);
            let Ok(entries) = std::fs::read_dir(&runtime.yaml_dir) else { continue };
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "satz") && declares_an_estate(p))
                .collect();
            found.sort();
            for estate in found {
                estates.push(EstateEntry {
                    config: config.display().to_string(),
                    deployment_mode: crate::estate_param(&estate, &runtime, "deployment-mode"),
                    estate: estate.display().to_string(),
                });
            }
        }
        Ok(Ok(Json(EstatesReport { root: self.ctx.root.display().to_string(), estates })))
    }

    #[tool(
        name = "satz_require",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::compliance::RequireReport>(),
        description = "Goal view: which controls of a compliance catalog the DECLARED estate satisfies, \
                       from the claims of the packs it uses. Offline, reads nothing live.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn require(
        &self,
        Parameters(args): Parameters<RequireArgs>,
    ) -> Result<Result<Json<crate::compliance::RequireReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, _org) = match self.inputs(&open, &estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::compliance::require_report(
            &args.framework,
            &estate,
            &open.runtime.presets_dir,
            &claims,
            &manifest,
        ) {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("require {}: {}", args.framework, e)))),
        }
    }

    #[tool(
        name = "satz_questions",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::questions::QuestionsReport>(),
        description = "What this estate can be asked: every question its packs declare, joined with the \
                       answers its params already carry, why each is asked, and what changing each answer \
                       would cost. This is the catalog's data — an agent renders its own; the CLI's \
                       `--format markdown` and `--format xlsx` write the two a human is handed. Offline and \
                       schema-free.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn questions(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<crate::questions::QuestionsReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::questions::questions_report(&estate, &open.runtime) {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("questions: {}", e)))),
        }
    }

    #[tool(
        name = "satz_interview",
        output_schema = rmcp::handler::server::tool::schema_for_output::<InterviewReport>(),
        description = "Run an interview against an estate: the questions its packs declare that the \
                       estate has not answered yet (or all of them, with `filter: all`), each with the \
                       default the pack offers — or `blocking: true` when no default is possible and a \
                       value must be typed. Answering is writing the param into the estate's `params {}`; \
                       accepting a default is writing the default. `summary.complete` is the gate: \
                       bootstrap and apply refuse until it is true. With `create: true` (needs 'write') \
                       the estate file is written first if it does not exist, so an interview can start \
                       before anything does. Offline and schema-free.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn interview(
        &self,
        Parameters(args): Parameters<InterviewArgs>,
    ) -> Result<Result<Json<InterviewReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let open = match self.opened() {
            Ok(o) => o,
            Err(r) => return Ok(Err(r)),
        };
        let named = match &args.estate {
            None => open.estate.clone(),
            Some(n) => crate::estate_path(PathBuf::from(n), &open.runtime),
        };
        let mut created = false;
        if !named.exists() {
            if !args.create {
                return Ok(Err(refused(format!(
                    "{}: no such estate. Pass `create: true` to start the interview there — the file is \
                     written as a skeleton that uses presets/estate-core.satz, and every question is open.",
                    named.display()
                ))));
            }
            if let Err(r) = self.permits(Group::Write) {
                return Ok(Err(r));
            }
            // The file does not exist yet, so confine its DIRECTORY.
            let dir = named.parent().map(PathBuf::from).unwrap_or_default();
            if let Err(r) = self.confine(dir) {
                return Ok(Err(r));
            }
            let stem = named.file_stem().and_then(|s| s.to_str()).unwrap_or("estate");
            if let Err(e) = crate::fsx::write_generated_satz(&named, &crate::template::skeleton(stem)) {
                return Ok(Err(refused(format!("{}: {}", named.display(), e))));
            }
            created = true;
        }
        let estate = match self.confine(named) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let mut written = 0;
        if !args.answers.is_empty() || args.accept_defaults {
            if let Err(r) = self.permits(Group::Write) {
                return Ok(Err(r));
            }
            let mut answers = BTreeMap::new();
            for (k, v) in &args.answers {
                match serde_yaml::to_value(v) {
                    Ok(y) => answers.insert(k.clone(), y),
                    Err(e) => return Ok(Err(refused(format!("interview: {}: {}", k, e)))),
                };
            }
            written = match crate::interview::apply(&estate, &open.runtime, &answers, args.accept_defaults) {
                Ok(n) => n,
                Err(e) => return Ok(Err(refused(format!("interview: {}", e)))),
            };
        }
        let mut report = match crate::questions::questions_report(&estate, &open.runtime) {
            Ok(r) => r,
            Err(e) => return Ok(Err(refused(format!("interview: {}", e)))),
        };
        let rename_to = crate::questions::rename_to(&estate, &report);
        if args.filter == InterviewFilter::Unanswered {
            // The summary stays whole: it describes the estate, not the filter.
            report.questions.retain(|q| q.state == "unanswered");
        }
        Ok(Ok(Json(InterviewReport { created, written, rename_to, report })))
    }

    #[tool(
        name = "satz_triage",
        output_schema = rmcp::handler::server::tool::schema_for_output::<Vec<crate::compliance::TriageRow>>(),
        description = "Sort a Prowler export's FAILs into buckets A–E against what the estate CLAIMS: \
                       who fixes each finding, and whether a pack already covers it. Offline; the \
                       Prowler JSON is read from a path under the server's root.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn triage(
        &self,
        Parameters(args): Parameters<TriageArgs>,
    ) -> Result<Result<Json<Vec<crate::compliance::TriageRow>>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let prowler = match self.file(&args.prowler) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, _org) = match self.inputs(&open, &estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::compliance::triage_rows(
            &args.framework,
            &open.runtime.presets_dir,
            &claims,
            &manifest,
            &prowler,
        ) {
            Ok(t) => Ok(Ok(Json(t.rows))),
            Err(e) => Ok(Err(refused(format!("triage: {}", e)))),
        }
    }

    #[tool(
        name = "satz_prowler",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::prowler::ProwlerPlan>(),
        description = "The Prowler invocation THIS estate needs: which frameworks, which projects, \
                       which output path — read from what the estate declares. It PRINTS the command; \
                       satz never runs Prowler, and neither does this tool. Run the command yourself, \
                       then feed its export to satz_report_compliance.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn prowler(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<crate::prowler::ProwlerPlan>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, org) = match self.inputs(&open, &estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let today = crate::prowler::today_utc();
        Ok(Ok(Json(crate::prowler::plan(&manifest, &claims, org.as_deref(), &today))))
    }

    #[tool(
        name = "satz_transpile_check",
        output_schema = rmcp::handler::server::tool::schema_for_output::<CompileSummary>(),
        description = "Compile the estate in memory and report what it would emit. Writes nothing — \
                       the gate to run before touching hcl/.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn transpile_check(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<CompileSummary>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::pipeline_b_generate(&estate, &open.tool, &open.runtime) {
            Ok(out) => Ok(Ok(Json(CompileSummary {
                estate: estate.display().to_string(),
                addresses: out.manifest.addresses().into_iter().collect(),
                written: Vec::new(),
                findings: out.findings,
            }))),
            Err(e) => Ok(Err(refused_with_findings(
                format!("transpile --check: {}", e),
                &estate,
                e.as_ref(),
            ))),
        }
    }

    #[tool(
        name = "satz_fmt",
        output_schema = rmcp::handler::server::tool::schema_for_output::<FmtResult>(),
        description = "Format Satz text: the canonical layout every file in the library is in — two-space \
                       indent, `=` aligned over a run, one list item per line. Text in, text out: satz \
                       writes no file, so the client keeps the one it holds. Meaning never changes, and the \
                       formatter proves it. `satz_review_pack` refuses a pack that is not formatted, and \
                       this is what formats it.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn fmt(
        &self,
        Parameters(args): Parameters<FmtArgs>,
    ) -> Result<Result<Json<FmtResult>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        match satz_core::fmt::format(&args.text) {
            Ok(formatted) => {
                let changed = formatted != args.text;
                Ok(Ok(Json(FmtResult { formatted, changed })))
            }
            Err(e) => Ok(Err(refused(format!("fmt: {}", e)))),
        }
    }

    #[tool(
        name = "satz_review_pack",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::review_pack::Review>(),
        description = "Judge one pack against the library's own bar, the way a pull request would: it \
                       parses, it is formatted, its header says what it is, its version has a changelog \
                       row, it declares no membership (presets define groups, humans grant membership), it \
                       runs no legacy org-policy constraint beside its managed replacement, every resource \
                       type it emits has a row in satz's prerequisite table, and it compiles. A pack is a \
                       fragment, so it is folded into an estate to see what it emits — a synthesised one \
                       unless `against` names a real estate. Returns the same findings the compile and the \
                       language server produce, each anchored to file and line. Offline, reads only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn review_pack(
        &self,
        Parameters(args): Parameters<ReviewPackArgs>,
    ) -> Result<Result<Json<crate::review_pack::Review>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        // a pack is a file, not an estate, so the server's root is what bounds it
        let pack = match self.confine(self.ctx.root.join(&args.pack)) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let against = match args.against.as_deref().map(|a| self.confine(self.ctx.root.join(a))) {
            Some(Ok(p)) => Some(p),
            Some(Err(r)) => return Ok(Err(r)),
            None => None,
        };
        let (open, _) = match self.target(None) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::review_pack::review(&pack, against.as_deref(), &open.tool, &open.runtime) {
            Ok(review) => Ok(Ok(Json(review))),
            Err(e) => Ok(Err(refused(format!("review-pack: {}", e)))),
        }
    }

    #[tool(
        name = "satz_check_presets",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::presets::CheckPresetsReport>(),
        description = "Preset drift: which packs in the local library are clean, behind upstream, \
                       locally edited, or changed only in the questions they ask — with the remedy \
                       for each. Downloads the pristine library to compare.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = true)
    )]
    async fn check_presets(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<crate::presets::CheckPresetsReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::presets::check_presets_report(
            &estate,
            &open.runtime.presets_dir,
            &open.runtime.include_dirs,
            None,
        )
        .await
        {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("check-presets: {}", e)))),
        }
    }

    #[tool(
        name = "satz_transpile",
        output_schema = rmcp::handler::server::tool::schema_for_output::<CompileSummary>(),
        description = "Compile the estate to OpenTofu HCL and WRITE it into the configured hcl_dir. \
                       Needs the 'write' capability.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn transpile(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<CompileSummary>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Write) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let out = match crate::pipeline_b_generate(&estate, &open.tool, &open.runtime) {
            Ok(out) => out,
            Err(e) => {
                return Ok(Err(refused_with_findings(format!("transpile: {}", e), &estate, e.as_ref())));
            }
        };
        // The directory, or the part of it that exists, must be inside the root.
        let dir = PathBuf::from(&open.runtime.hcl_dir);
        let existing = dir.ancestors().find(|a| a.exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| dir.clone());
        if let Err(r) = self.confine(existing) {
            return Ok(Err(r));
        }
        let label = estate.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        match crate::write_hcl(&out, &dir, &label) {
            Ok(written) => Ok(Ok(Json(CompileSummary {
                estate: estate.display().to_string(),
                addresses: out.manifest.addresses().into_iter().collect(),
                written: written.iter().map(|p| p.display().to_string()).collect(),
                findings: out.findings.clone(),
            }))),
            Err(e) => Ok(Err(refused(format!("transpile: {}", e)))),
        }
    }

    /// The dossier for a remediation tool: the estate's compile, the Prowler export,
    /// and Checkov when asked for (which needs 'exec').
    async fn remediation(
        &self,
        estate: Option<&str>,
        framework: &str,
        prowler: &str,
        checkov: bool,
    ) -> Result<crate::compliance::RemediationRun, CallToolResult> {
        if checkov {
            self.permits(Group::Exec)?;
        }
        let (open, estate) = self.target(estate)?;
        let prowler = self.file(prowler)?;
        let (manifest, claims, _org) = self.inputs(&open, &estate)?;
        let report = if checkov {
            let dir = self.confine(PathBuf::from(&open.runtime.hcl_dir))?;
            match tokio::task::spawn_blocking(move || crate::scan::run(&dir)).await {
                Ok(Ok(r)) => Some(r),
                Ok(Err(e)) => return Err(refused(format!("checkov: {}", e))),
                Err(e) => return Err(refused(format!("checkov did not finish: {}", e))),
            }
        } else {
            None
        };
        crate::compliance::remediation_run(framework, &open.runtime.presets_dir, &claims, &manifest, &estate, &prowler, report.as_ref())
            .map_err(|e| refused(format!("remediation: {}", e)))
    }

    #[tool(
        name = "satz_remediation_items",
        output_schema = rmcp::handler::server::tool::schema_for_output::<RemediationItems>(),
        description = "The remediation dossier's items for an estate and a Prowler export: every finding \
                       triaged, deduplicated and joined per (control, resource), with the dossier sha256 \
                       authored values must name. The worklist for the [Authored] columns — write them back \
                       with satz_remediation_annotate. Offline; `checkov: true` joins a Checkov run and needs 'exec'.",
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn remediation_items(
        &self,
        Parameters(args): Parameters<RemediationArgs>,
    ) -> Result<Result<Json<RemediationItems>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let run = match self.remediation(args.estate.as_deref(), &args.framework, &args.prowler, args.checkov).await {
            Ok(r) => r,
            Err(r) => return Ok(Err(r)),
        };
        Ok(Ok(Json(RemediationItems {
            framework: run.framework,
            estate: run.estate,
            dossier_sha256: run.hash,
            summary: run.dossier.summary,
            items: run.dossier.items,
            prowler_unmapped: run.prowler_unmapped,
        })))
    }

    #[tool(
        name = "satz_remediation_annotate",
        output_schema = rmcp::handler::server::tool::schema_for_output::<AnnotateReport>(),
        description = "Write authored values for dossier items into <out>/authored.json — merged per item id \
                       with what is on file — and render the run there: dossier.json, findings.csv, \
                       findings.xlsx with the [Authored] columns filled, meta.json. Refused when dossier_sha256 \
                       is not the current dossier's, an id is unknown, or an entry lacks authored_by or \
                       authored_at. Needs the 'write' capability.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn remediation_annotate(
        &self,
        Parameters(args): Parameters<AnnotateArgs>,
    ) -> Result<Result<Json<AnnotateReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Write) {
            return Ok(Err(r));
        }
        let run = match self.remediation(args.estate.as_deref(), &args.framework, &args.prowler, args.checkov).await {
            Ok(r) => r,
            Err(r) => return Ok(Err(r)),
        };
        let out = self.ctx.root.join(&args.out);
        let existing = out.ancestors().find(|a| a.exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| out.clone());
        if let Err(r) = self.confine(existing) {
            return Ok(Err(r));
        }
        let on_file = out.join("authored.json");
        let mut authored = if on_file.is_file() {
            match crate::compliance::read_authored(&on_file) {
                Ok(a) => a,
                Err(e) => return Ok(Err(refused(e.to_string()))),
            }
        } else {
            crate::dossier::Authored::default()
        };
        let new = crate::dossier::Authored { dossier_sha256: args.dossier_sha256, items: args.items };
        if let Err(e) = authored.merge(new) {
            return Ok(Err(refused(format!("{}: {}", on_file.display(), e))));
        }
        if let Err(e) = crate::dossier::check_authored(&run.dossier, &run.hash, &authored) {
            return Ok(Err(refused(e)));
        }
        match crate::compliance::write_remediation(&run, &out, Some(&authored)) {
            Ok(written) => Ok(Ok(Json(AnnotateReport {
                out: out.display().to_string(),
                dossier_sha256: run.hash,
                authored_items: authored.items.len(),
                written: written.iter().map(|p| p.display().to_string()).collect(),
            }))),
            Err(e) => Ok(Err(refused(format!("remediation: {}", e)))),
        }
    }

    #[tool(
        name = "satz_adopt",
        output_schema = rmcp::handler::server::tool::schema_for_output::<AdoptReport>(),
        description = "Resolve every resource the estate declares against the LIVE organisation — natural-key \
                       lookups and the import-config rules — and say per resource whether it would be imported, \
                       moved in the state, is already managed, or cannot be resolved. With `execute` (needs \
                       'write') it writes the verified ids into the estate as \"import-id\". Running `tofu \
                       import`, a state move or activating a managed constraint stays on the command line \
                       (`satz adopt --execute --import`). Runs as the estate's service account.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn adopt(
        &self,
        Parameters(args): Parameters<AdoptArgs>,
    ) -> Result<Result<Json<AdoptReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(if args.execute { Group::Write } else { Group::Read }) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        // The lookups read the live organisation as THIS estate's service account,
        // for the duration of this call only.
        let plan = match crate::gcp::with_identity(
            Self::identity_of(&open),
            crate::adopt_plan(&estate, args.only, false, &open.tool, &open.runtime),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Ok(Err(refused(format!("adopt: {}", e)))),
        };
        let in_state = plan.state.clone().unwrap_or_default();
        let unanswered = crate::adopt::unanswered(&plan.resolutions, &in_state);
        let conflicts = crate::adopt::move_conflicts(&plan.resolutions, &in_state);
        let mut report = AdoptReport {
            estate: estate.display().to_string(),
            declared: plan.out.manifest.resources.len(),
            rows: crate::adopt::rows(&plan.resolutions, &in_state, &plan.out.manifest),
            summary: crate::adopt::summary(&plan.resolutions, &in_state),
            unanswered,
            move_conflicts: conflicts
                .iter()
                .map(|(address, also)| MoveConflict { address: address.clone(), also_declared: also.clone() })
                .collect(),
            state_note: plan.state.as_ref().err().map(|e| {
                format!(
                    "the state could not be read ({}), so nothing is marked as already managed",
                    e.lines().next().unwrap_or("(no output)")
                )
            }),
            written: Vec::new(),
            hints: Vec::new(),
        };
        if args.execute {
            if unanswered > 0 || !conflicts.is_empty() {
                return Ok(Err(refused(format!(
                    "adopt: {} row(s) did not answer and {} live object(s) are declared twice — nothing was written; \
                     run without `execute` to see the rows",
                    unanswered,
                    conflicts.len()
                ))));
            }
            match crate::adopt::write_import_ids(&plan.resolutions, Some(std::path::Path::new(&open.runtime.presets_dir))) {
                Ok((written, hints)) => {
                    report.written = written;
                    report.hints = hints;
                }
                Err(e) => return Ok(Err(refused(format!("adopt: {}", e)))),
            }
        }
        Ok(Ok(Json(report)))
    }

    #[tool(
        name = "satz_merge_presets",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::presets::MergeReport>(),
        description = "Reconcile the estate's preset library with upstream: install what is missing, take doc \
                       and format changes silently, and for a pack the estate USES that changed semantically \
                       fork it to `X.local.satz` and repoint the estate — proving the repoint by transpile \
                       identity. `adopt` takes upstream in place for the packs named (`all` for every pack \
                       merely behind) and reports the emission delta instead. `report_only` writes nothing. \
                       The answer is the run as events in walk order, plus the counts and `attention`, which \
                       is what the command exits non-zero on. Needs the 'write' capability; `report_only` \
                       still needs it, because the walk fetches upstream.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn merge_presets(
        &self,
        Parameters(args): Parameters<MergePresetsArgs>,
    ) -> Result<Result<Json<crate::presets::MergeReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Write) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let presets = PathBuf::from(&open.runtime.presets_dir);
        let existing = presets.ancestors().find(|a| a.exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| presets.clone());
        if let Err(r) = self.confine(existing) {
            return Ok(Err(r));
        }
        let pristine = match args.pristine_dir.as_deref().map(|p| self.file(p)).transpose() {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::presets::run_merge_presets(
            &open.runtime.presets_dir,
            pristine,
            Some(estate),
            &open.tool,
            &open.runtime,
            args.report_only,
            &args.adopt,
        )
        .await
        {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("merge-presets: {}", e)))),
        }
    }

    #[tool(
        name = "satz_get_presets",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::presets::GetPresetsReport>(),
        description = "Fetch the upstream preset library into the open estate's presets_dir: missing files \
                       installed, identical ones left, changed ones the estate does not use refreshed. A pack \
                       the estate USES that upstream changed is refused — merge-presets forks or adopts it — \
                       unless `force`. Needs the 'write' capability.",
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true)
    )]
    async fn get_presets(
        &self,
        Parameters(args): Parameters<GetPresetsArgs>,
    ) -> Result<Result<Json<crate::presets::GetPresetsReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Write) {
            return Ok(Err(r));
        }
        let (open, _estate) = match self.target(None) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let presets = PathBuf::from(&open.runtime.presets_dir);
        let existing = presets.ancestors().find(|a| a.exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| presets.clone());
        if let Err(r) = self.confine(existing) {
            return Ok(Err(r));
        }
        let pristine = match args.pristine_dir.as_deref().map(|p| self.file(p)).transpose() {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::presets::get_presets(&open.runtime.presets_dir, &open.runtime, args.force, pristine).await {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("get-presets: {}", e)))),
        }
    }

    #[tool(
        name = "satz_update_prerequisites",
        output_schema = rmcp::handler::server::tool::schema_for_output::<PrerequisitesResult>(),
        description = "What the estate's own resource types oblige it to declare and it does not: the roles \
                       its IaC service account is missing (`missing`, with `write` the fewest roles that close \
                       it) and the APIs its infrastructure project does not enable (`missing_apis`, each with \
                       the types that need it). `unknown_types` are emitted types the table has no row for. \
                       Offline. It WRITES both into the estate file by default and re-checks — a gap that \
                       survives the write restores the file — so the default needs 'write'; pass `report_only` \
                       to list the gap instead, which needs only 'read'.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_prerequisites(
        &self,
        Parameters(args): Parameters<PrerequisitesArgs>,
    ) -> Result<Result<Json<PrerequisitesResult>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        // the default writes, so the ceiling is checked for the default
        if !args.report_only {
            if let Err(r) = self.permits(Group::Write) {
                return Ok(Err(r));
            }
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let report = match crate::prerequisites_report(&estate, &open.tool, &open.runtime) {
            Ok(r) => r,
            Err(e) => return Ok(Err(refused(format!("update-prerequisites: {}", e)))),
        };
        if args.report_only || (report.missing.is_empty() && report.missing_apis.is_empty()) {
            return Ok(Ok(Json(PrerequisitesResult { report, written: Vec::new() })));
        }
        match crate::prerequisites_write(&estate, &report, &open.tool, &open.runtime) {
            Ok((written, after)) => Ok(Ok(Json(PrerequisitesResult { report: after, written }))),
            Err(e) => Ok(Err(refused(format!("update-prerequisites: {}", e)))),
        }
    }

    #[tool(
        name = "satz_scan_checkov",
        output_schema = rmcp::handler::server::tool::schema_for_output::<ScanReport>(),
        description = "Run Checkov over the estate's emitted HCL (the hcl_dir satz_transpile writes) and return \
                       every failed check with the Satz block that declared the resource. Scans what is written: \
                       transpile first. Needs the 'exec' capability — it runs an external tool (checkov on PATH, \
                       else uvx checkov).",
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn scan_checkov(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<ScanReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Exec) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let dir = match self.confine(PathBuf::from(&open.runtime.hcl_dir)) {
            Ok(d) => d,
            Err(r) => return Ok(Err(r)),
        };
        // The compile says which Satz block declared each resource; the scan is of
        // the files on disk.
        let manifest = match crate::pipeline_b_generate(&estate, &open.tool, &open.runtime) {
            Ok(out) => out.manifest,
            Err(e) => return Ok(Err(refused(format!("scan: {}", e)))),
        };
        let scan_dir = dir.clone();
        let report = match tokio::task::spawn_blocking(move || crate::scan::run(&scan_dir)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Ok(Err(refused(format!("scan: {}", e)))),
            Err(e) => return Ok(Err(refused(format!("scan: Checkov did not finish: {}", e)))),
        };
        let findings = report
            .findings
            .iter()
            .map(|f| ScanFinding {
                check_id: f.check_id.clone(),
                check_name: f.check_name.clone(),
                resource: f.resource.clone(),
                declared_at: manifest
                    .resources
                    .get(&f.resource)
                    .and_then(|r| r.origin.as_ref())
                    .map(|(file, line)| format!("{}:{}", file, line)),
                guideline: f.guideline.clone(),
            })
            .collect();
        Ok(Ok(Json(ScanReport {
            estate: estate.display().to_string(),
            hcl_dir: dir.display().to_string(),
            checkov_version: report.version,
            passed: report.passed,
            failed: report.failed,
            skipped: report.skipped,
            resource_count: report.resource_count,
            findings,
        })))
    }

    #[tool(
        name = "satz_report_compliance",
        output_schema = rmcp::handler::server::tool::schema_for_output::<serde_json::Value>(),
        description = "Evidence report: the goal view joined with LIVE verification through Cloud \
                       Asset Inventory, manual-duty attestations and optional Prowler corroboration. \
                       Reads the organisation with the estate's credentials. Writes nothing — unlike \
                       the command, it does not append to the evidence history, because being ASKED \
                       for state is not a report run. Check `live_status` before trusting the rows: \
                       a run whose inventory could not be read answers `live: false` with the reason \
                       in `warnings`, and every witness reads unverified.",
        annotations(read_only_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn report_compliance(
        &self,
        Parameters(args): Parameters<ReportComplianceArgs>,
    ) -> Result<Result<Json<serde_json::Value>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let (open, estate) = match self.target(args.estate.as_deref()) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let prowler = match args.prowler.as_deref().map(|p| self.file(p)).transpose() {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, org_id) = match self.inputs(&open, &estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        // Everything the report reads — Cloud Asset, the project numbers — mints
        // as THIS estate's service account for the duration of this call, and
        // nothing outside the call is affected.
        match crate::gcp::with_identity(
            Self::identity_of(&open),
            crate::compliance::report_compliance_evidence(
            &args.framework,
            &estate,
            &open.runtime.presets_dir,
            &claims,
            &manifest,
            org_id.as_deref(),
            &self.ctx.root,
            prowler,
            None,
            args.no_live,
        ),
        )
        .await
        {
            Ok((evidence, _md)) => Ok(Ok(Json(evidence))),
            Err(e) => Ok(Err(refused(format!("report-compliance: {}", e)))),
        }
    }

    #[tool(
        name = "satz_whoami",
        output_schema = rmcp::handler::server::tool::schema_for_output::<crate::gcp::identity::WhoamiReport>(),
        description = "BOTH halves of the identity: the Application Default Credentials account \
                       and its file, and the service account the open estate's live tools actually \
                       run as. Online it also CHECKS them — whether this credential may become that \
                       service account, and whether the quota project is reachable — so a refused \
                       live call is explained here rather than guessed at. The first thing to check \
                       when anything live fails.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = true)
    )]
    async fn whoami(
        &self,
        Parameters(args): Parameters<WhoamiArgs>,
    ) -> Result<Result<Json<crate::gcp::identity::WhoamiReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        // "Who am I" has two answers, and which one is wanted depends on whether
        // an estate is in play. With one open, the honest answer is the identity
        // that estate's live tools RUN as — so it is answered inside the same
        // scope they use, not merely described.
        let scoped = match self.ctx.open.lock().expect("the open lock is never poisoned").clone() {
            Some(open) if args.estate.is_none() => {
                let estate = open.estate.clone();
                Some((Self::identity_of(&open), open, estate))
            }
            Some(_) => match self.target(args.estate.as_deref()) {
                Ok((open, estate)) => {
                    Some((crate::estate_impersonation_target(&estate, &open.runtime), open, estate))
                }
                Err(r) => return Ok(Err(r)),
            },
            // Nothing open: the question is about the ambient credentials, which
            // is exactly what `satz whoami` answers with no estate.
            None => None,
        };
        let report = match scoped {
            Some((sa, open, estate)) => {
                // Online, the estate's resource types say which permissions to test;
                // an estate that does not compile still gets its identity answered.
                let probe = if args.offline {
                    None
                } else {
                    crate::iac_probe(&estate, &open.tool, &open.runtime).ok()
                };
                crate::gcp::with_identity(sa, crate::gcp::identity::whoami_report(args.offline, probe)).await
            }
            None => crate::gcp::identity::whoami_report(args.offline, None).await,
        };
        match report {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("whoami: {}", e)))),
        }
    }

    #[tool(
        name = "satz_restrict",
        description = "Lower this session's capability level for the rest of the connection. It can \
                       only ever shrink — never raised back, never above the ceiling the server was \
                       started with. Available only with --self-gated.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn restrict(&self, Parameters(args): Parameters<RestrictArgs>) -> Result<CallToolResult, McpError> {
        if !self.self_gated {
            return Ok(refused(
                "this server was not started with --self-gated; its level is fixed".into(),
            ));
        }
        let wanted = match Level::parse(&args.allow) {
            Ok(l) => l,
            Err(e) => return Ok(refused(e)),
        };
        let mut level = self.level.lock().expect("the level lock is never poisoned");
        if !wanted.within(*level) {
            return Ok(refused(format!(
                "'{}' is wider than the level in force ('{}') — restrict only ever shrinks",
                wanted.describe(),
                level.describe()
            )));
        }
        *level = wanted;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "level is now '{}' (ceiling '{}')",
            wanted.describe(),
            self.ceiling.describe()
        ))]))
    }
}

#[tool_handler]
impl ServerHandler for SatzMcp {
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = DOCS
            .iter()
            .map(|d| {
                let mut r = Resource::new(d.uri, d.name.to_string());
                r.description = Some(d.description.to_string());
                r.mime_type = Some("text/markdown".to_string());
                r
            })
            .collect();
        // built from the default so a field added upstream cannot break this
        Ok(ListResourcesResult { resources, ..Default::default() })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        match DOCS.iter().find(|d| d.uri == request.uri) {
            Some(d) => Ok(ReadResourceResult::new(vec![ResourceContents::text(d.body(), d.uri)]).into()),
            None => Err(McpError::resource_not_found(
                format!(
                    "no such resource: {} — this server offers {}",
                    request.uri,
                    DOCS.iter().map(|d| d.uri).collect::<Vec<_>>().join(", ")
                ),
                None,
            )),
        }
    }

    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` and `Implementation` are #[non_exhaustive]: build from the
        // default and assign, so a field added upstream cannot break this.
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().enable_resources().build();
        info.server_info.name = "satz".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(format!(
            "satz compiles an estate written in Satz to OpenTofu HCL and judges it against \
             compliance catalogs. It never calls a model: ask it for facts, and do the judging \
             yourself.\n\n\
             BEFORE WRITING OR EDITING ANY .satz FILE, read the resource `satz://guide`. It is \
             the working subset of the language and the order to call these tools in; without it \
             you will write something that compiles and is wrong. `satz://reference` is the full \
             language reference and `satz://presets` describes the shipped packs.\n\n\
             After every edit, call satz_transpile_check before saying you are done. Never edit \
             the generated hcl/ directory, and never invent an id — resolve it with adopt or ask.\n\n\
             The satz command behind each tool: {}. These have no command behind them — they are \
             this session's own plumbing: {}.\n\n\
             These satz commands are NOT available here, by decision — ask the human to run one \
             rather than working around it: {}.\n\n\
             Capability level in force: '{}'.",
            served_by(),
            MCP_ONLY.join(", "),
            not_served(),
            self.ceiling.describe()
        ));
        info
    }
}

/// Serve on stdio until the client disconnects.
///
/// stdout IS the protocol here. Everything satz says to a human — the version
/// banner, schema-loader progress, emitter warnings — already goes to stderr for
/// exactly this reason; a stray line on stdout is a corrupt stream, not noise.
pub(crate) async fn serve(
    root: PathBuf,
    ceiling: Level,
    self_gated: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!(
        "satz mcp: serving on stdio at level '{}'{}",
        ceiling.describe(),
        if self_gated { ", self-gated" } else { "" }
    );
    let service = match SatzMcp::new(root, ceiling, self_gated)
        .serve(rmcp::transport::stdio())
        .await
    {
        Ok(s) => s,
        // A client that hangs up before `initialize` has not failed at anything.
        // It is what `echo "" | satz mcp` does, and what a client does when it
        // decides not to start us after all. Saying `Error: ConnectionClosed`
        // and exiting non-zero teaches an operator to distrust a server that is
        // working — the first thing anyone does to check this command is run it
        // by hand with nothing on stdin.
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => {
            eprintln!(
                "satz mcp: stdin closed before the client said hello — nothing to serve, exiting."
            );
            eprintln!(
                "          This is what running it by hand does. A client starts it and speaks first; \
                 to try it yourself, pipe an `initialize` request in."
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Group, Level, DOCS};

    /// A typo in a capability grant must be an error. Read as "less" it would
    /// silently disable half a pipeline; read as "more" it would grant what
    /// nobody asked for.
    #[test]
    fn an_unknown_group_is_refused() {
        assert!(Level::parse("read,wrote").is_err());
        assert!(Level::parse("").is_err(), "granting nothing is a mistake, not a level");
        assert_eq!(
            Level::parse("read, exec").unwrap(),
            Level { read: true, write: false, exec: true }
        );
    }

    #[test]
    fn a_level_permits_only_what_it_names() {
        let l = Level::parse("read").unwrap();
        assert!(l.allows(Group::Read));
        assert!(!l.allows(Group::Write));
        assert!(!l.allows(Group::Exec));
    }

    /// The whole point of --self-gated: an agent may tie its own hands and can
    /// never untie them, nor reach past the ceiling it was started with.
    #[test]
    fn restrict_only_ever_shrinks() {
        let ceiling = Level::parse("read,write").unwrap();
        let narrower = Level::parse("read").unwrap();
        assert!(narrower.within(ceiling));
        assert!(!ceiling.within(narrower), "a level may not grow back");
        let exec = Level::parse("exec").unwrap();
        assert!(!exec.within(ceiling), "a level may not reach past the ceiling");
    }

    /// stdout is the protocol. A `println!` anywhere on a tool's path corrupts the
    /// JSON-RPC stream, and the client reports nothing useful rather than an error —
    /// so this module may not write to stdout at all. The smoke matrix checks the
    /// server's actual output; this catches the mistake at the source, in `cargo test`.
    #[test]
    fn this_module_never_writes_to_stdout() {
        let whole = include_str!("mcp.rs");
        // the test module names the forbidden macros in its own assertions
        let src = whole.split("#[cfg(test)]").next().unwrap_or(whole);
        for (n, line) in src.lines().enumerate() {
            // `eprintln!` contains `println!` as a substring — stderr is fine
            let code = line
                .split("//")
                .next()
                .unwrap_or("")
                .replace("eprintln!", "")
                .replace("eprint!", "");
            assert!(
                !code.contains("println!(") && !code.contains("print!("),
                "src/mcp.rs:{}: stdout is the protocol here — use eprintln!: {}",
                n + 1,
                line.trim()
            );
        }
    }

    /// The module test above is not enough on its own, and one real bug proved it:
    /// `identity::announce` printed the credential line to stdout from behind
    /// `gcp::access_token()`, so the FIRST live tool call corrupted the stream
    /// without a single `println!` appearing in this file.
    ///
    /// The smoke matrix cannot catch that — it has no credentials, so it never
    /// reaches a live tool call. So the code a tool reaches transitively is
    /// gated here instead: the token chokepoint, and the announce path behind it.
    /// (`identity.rs` as a whole is NOT covered — `whoami` prints its answer to
    /// stdout, which is correct for a CLI command.)
    #[test]
    fn the_token_path_never_writes_to_stdout() {
        let mut regions: Vec<(&str, &str)> = vec![("src/gcp/mod.rs", include_str!("gcp/mod.rs"))];

        // Just the three announce functions, not the whole file.
        let identity = include_str!("gcp/identity.rs");
        let start = identity
            .find("async fn announce_info")
            .expect("announce_info moved — re-point this gate");
        let end = identity
            .find("pub(crate) fn mark_announced")
            .expect("mark_announced moved — re-point this gate");
        assert!(start < end, "the announce path is no longer one contiguous region");
        regions.push(("src/gcp/identity.rs (announce path)", &identity[start..end]));

        // `satz_check_presets` downloads the pristine library and compares; the
        // download counted itself on stdout once, which corrupted the stream.
        regions.push(("src/github.rs", include_str!("github.rs")));
        let presets = include_str!("presets.rs");
        let start = presets
            .find("async fn pristine_source")
            .expect("pristine_source moved — re-point this gate");
        let end = presets
            .find("pub(crate) async fn check_presets_report")
            .and_then(|at| presets[at..].find("\n}\n").map(|e| at + e))
            .expect("check_presets_report moved — re-point this gate");
        regions.push(("src/presets.rs (the check-presets path)", &presets[start..end]));

        // `satz_get_presets` and `satz_adopt` reach these in full.
        let start = presets.find("pub(crate) async fn get_presets").expect("get_presets moved — re-point this gate");
        let end = presets.find("/// What `get-presets` did to the library.").expect("GetPresetsReport moved — re-point this gate");
        regions.push(("src/presets.rs (get_presets)", &presets[start..end]));
        let adopt = include_str!("adopt.rs");
        regions.push(("src/adopt.rs", adopt.split("#[cfg(test)]").next().unwrap_or(adopt)));
        let main = include_str!("main.rs");
        let start = main.find("pub(crate) async fn adopt_plan").expect("adopt_plan moved — re-point this gate");
        let end = main[start..].find("\n}\n").map(|e| start + e).expect("adopt_plan has no end");
        regions.push(("src/main.rs (adopt_plan)", &main[start..end]));

        for (what, src) in regions {
            for line in src.lines() {
                let code = line
                    .split("//")
                    .next()
                    .unwrap_or("")
                    .replace("eprintln!", "")
                    .replace("eprint!", "");
                assert!(
                    !code.contains("println!(") && !code.contains("print!("),
                    "{}: a tool reaches this code, and stdout is the protocol — \
                     write to stderr or take a sink: {}",
                    what,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn describe_names_every_granted_group() {
        assert_eq!(Level::parse("read,write,exec").unwrap().describe(), "read,write,exec");
        assert_eq!(Level::default().describe(), "nothing");
    }

    /// `Doc::body` slices at the marker and panics if it is gone. A heading
    /// renamed in the Markdown would otherwise turn a served resource into a
    /// panic on the first read — or, worse, a silent no-op if the trim ever
    /// grew a fallback.
    #[test]
    fn every_trimmed_resource_finds_its_marker() {
        for d in DOCS {
            let Some(m) = d.trim_at else { continue };
            assert!(
                d.text.contains(m),
                "{}: the trim marker {:?} is gone from the source document",
                d.uri,
                m
            );
            assert!(d.body().len() < d.text.len(), "{}: the trim removed nothing", d.uri);
        }
    }
}

#[cfg(test)]
mod parity_tests {
    //! An agent's reach over satz is a decision per command, and the three
    //! places that record it — this table, the registered tools, and the
    //! documented tool list — must agree.
    use super::*;
    use clap::CommandFactory;
    use std::collections::BTreeSet;

    fn table_tools() -> BTreeSet<&'static str> {
        MCP_PARITY
            .iter()
            .flat_map(|(_, p)| match p {
                Parity::Tools(ts) => ts.to_vec(),
                Parity::Off(_) => Vec::new(),
            })
            .chain(MCP_ONLY.iter().copied())
            .collect()
    }

    fn registered() -> BTreeSet<String> {
        SatzMcp::tool_router().list_all().into_iter().map(|t| t.name.to_string()).collect()
    }

    /// A reason is prose, and prose goes stale: `init`'s said `--from-live` for a
    /// release after that flag was retired, and nothing noticed. Any `--flag` a
    /// reason names has to be a flag the CLI still has.
    #[test]
    fn a_reason_that_names_a_flag_names_one_that_exists() {
        use clap::CommandFactory;
        let cli = crate::Cli::command();
        let mut flags: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut collect = |c: &clap::Command| {
            for a in c.get_arguments() {
                if let Some(l) = a.get_long() {
                    flags.insert(format!("--{}", l));
                }
                for l in a.get_all_aliases().unwrap_or_default() {
                    flags.insert(format!("--{}", l));
                }
            }
        };
        collect(&cli);
        for sub in cli.get_subcommands() {
            collect(sub);
        }
        for (command, parity) in super::MCP_PARITY {
            let Parity::Off(reason) = parity else { continue };
            for word in reason.split_whitespace() {
                let token: String =
                    word.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                if !token.starts_with("--") || token.len() < 4 {
                    continue;
                }
                assert!(
                    flags.contains(&token),
                    "MCP_PARITY: the reason for `{}` names {}, which is not a flag satz has — \
                     the reason has outlived the thing it described",
                    command,
                    token
                );
            }
        }
    }

    #[test]
    fn mcp_parity_is_decided() {
        let mut cmd = crate::Cli::command();
        cmd.build(); // `help` is generated here
        let cli: BTreeSet<&str> = cmd.get_subcommands().filter(|c| !c.is_hide_set()).map(|c| c.get_name()).collect();
        let table: BTreeSet<&str> = MCP_PARITY.iter().map(|(c, _)| *c).collect();
        let undecided: Vec<_> = cli.difference(&table).collect();
        let unknown: Vec<_> = table.difference(&cli).collect();
        assert!(
            undecided.is_empty(),
            "these commands are in no MCP_PARITY row, so whether an agent can run them is undecided: \
             {undecided:?} — give each one a tool or the reason it has none (src/mcp.rs)"
        );
        assert!(unknown.is_empty(), "MCP_PARITY names commands the CLI does not have: {unknown:?}");
        assert_eq!(MCP_PARITY.len(), table.len(), "a command has two MCP_PARITY rows");
    }

    #[test]
    fn every_tool_is_named_by_the_table() {
        let registered = registered();
        let table = table_tools();
        let missing: Vec<_> = registered.iter().filter(|t| !table.contains(t.as_str())).collect();
        let stale: Vec<_> = table.iter().filter(|t| !registered.contains(**t)).collect();
        assert!(
            missing.is_empty(),
            "these tools are served but no MCP_PARITY row (or MCP_ONLY) names them: {missing:?} — \
             a tool an agent can call must say which command it serves"
        );
        assert!(stale.is_empty(), "MCP_PARITY names tools that are not registered: {stale:?}");
    }

    /// The instructions are where an agent learns what it cannot do here; a
    /// table nothing renders would drift from the server it describes.
    #[test]
    fn the_instructions_name_the_tools_and_what_is_missing() {
        let served = served_by();
        assert!(served.contains("transpile -> satz_transpile, satz_transpile_check"), "{served}");
        assert!(served.contains("update-prerequisites -> satz_update_prerequisites"), "{served}");
        let off = not_served();
        assert!(off.contains("apply (it hands stdio to the tool"), "{off}");
        assert!(off.contains("bootstrap (day 0"), "{off}");
        assert!(!off.contains("completion"), "the shell affordances are noise here: {off}");
        for (c, p) in MCP_PARITY {
            if let Parity::Off(why) = p {
                assert!(!why.is_empty(), "{c} is not served and says no reason");
                assert!(why.starts_with(|ch: char| ch.is_lowercase() || ch == '`'), "{c}: {why}");
            }
        }
    }

    /// docs/mcp.md is the tool list a client's author reads. Nothing compared it
    /// to the server, so a tool could ship undocumented, or a removed one could
    /// stay on the page.
    #[test]
    fn the_docs_name_every_tool() {
        let doc = include_str!("../docs/mcp.md");
        let documented: BTreeSet<String> = doc
            .lines()
            .filter(|l| l.trim_start().starts_with("| `satz_"))
            .filter_map(|l| l.split('`').nth(1).map(str::to_string))
            .collect();
        let registered = registered();
        let undocumented: Vec<_> = registered.difference(&documented).collect();
        let gone: Vec<_> = documented.difference(&registered).collect();
        assert!(
            undocumented.is_empty(),
            "these tools are served and docs/mcp.md's table does not list them: {undocumented:?}"
        );
        assert!(gone.is_empty(), "docs/mcp.md lists tools the server does not serve: {gone:?}");
    }
}
