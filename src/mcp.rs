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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolResult, ContentBlock, ProtocolVersion, ServerCapabilities, ServerInfo};
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
    /// Runs an external tool, or changes a live organisation. Grantable today
    /// (`--allow exec`) but claimed by no tool yet, deliberately: an exec tool
    /// must CAPTURE its child's output. `tofu` and Checkov inherit stdio from
    /// the CLI, and here stdout is the protocol — a child writing to it is a
    /// corrupt JSON-RPC stream, not interleaved logs. That plumbing comes with
    /// the first exec tool rather than being hurried in beside the transport.
    #[allow(dead_code)]
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

struct Ctx {
    tool: ToolConfig,
    runtime: ToolConfig,
    /// every estate path a tool resolves must stay under this directory
    root: PathBuf,
}

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
    /// Estate file, e.g. `C0example.satz` (resolved inside the configured yaml_dir)
    pub estate: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct RequireArgs {
    /// Estate file, e.g. `C0example.satz`
    pub estate: String,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct TriageArgs {
    /// Estate file, e.g. `C0example.satz`
    pub estate: String,
    /// Catalog id, e.g. `cis-gcp-4.0`
    pub framework: String,
    /// Prowler export (OCSF or legacy JSON), a path under the server's root
    pub prowler: String,
}

/// What a compile produced. The addresses are the estate's emitted resources —
/// the same set the compliance plane witnesses against.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct CompileSummary {
    pub estate: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct ReportComplianceArgs {
    /// Estate file, e.g. `C0example.satz`
    pub estate: String,
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
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct RestrictArgs {
    /// Groups to keep, comma-separated: read, write, exec
    pub allow: String,
}

/// A refusal an agent can recover from. The plan's rule: a tool the level does
/// not permit is a tool RESULT with `isError`, never a protocol error — clients
/// retry the former and give up on the latter.
fn refused(msg: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg)])
}

// Every handler returns `Json<T>` on success. The SDK puts that in the result's
// `structuredContent` and publishes T's schema as the tool's `outputSchema`, so a
// client gets a typed value instead of a string it has to parse. Returning the
// report as a text block — what this server first did — threw that away: the JSON
// was there, but nothing said what shape it had.

#[tool_router]
impl SatzMcp {
    pub(crate) fn new(
        tool: ToolConfig,
        runtime: ToolConfig,
        root: PathBuf,
        ceiling: Level,
        self_gated: bool,
    ) -> Self {
        Self {
            ctx: Arc::new(Ctx { tool, runtime, root }),
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
    fn estate(&self, name: &str) -> Result<PathBuf, CallToolResult> {
        let p = crate::estate_path(PathBuf::from(name), &self.ctx.runtime);
        self.confine(p)
    }

    /// Any other path argument — a Prowler export, a report to read back.
    fn file(&self, name: &str) -> Result<PathBuf, CallToolResult> {
        self.confine(self.ctx.root.join(name))
    }

    fn confine(&self, p: PathBuf) -> Result<PathBuf, CallToolResult> {
        let resolved = p
            .canonicalize()
            .map_err(|e| refused(format!("{}: {}", p.display(), e)))?;
        let root = self.ctx.root.canonicalize().unwrap_or_else(|_| self.ctx.root.clone());
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
    fn inputs(&self, estate: &std::path::Path) -> Result<crate::ComplianceInputs, CallToolResult> {
        crate::compliance_inputs(estate, &self.ctx.tool, &self.ctx.runtime)
            .map_err(|e| refused(format!("{}: {}", estate.display(), e)))
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
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, _org) = match self.inputs(&estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::compliance::require_report(
            &args.framework,
            &estate,
            &self.ctx.runtime.presets_dir,
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
                       answers its params already carry, and what changing each answer would cost. \
                       Offline and schema-free.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn questions(
        &self,
        Parameters(args): Parameters<EstateArg>,
    ) -> Result<Result<Json<crate::questions::QuestionsReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::questions::questions_report(&estate, &self.ctx.runtime) {
            Ok(report) => Ok(Ok(Json(report))),
            Err(e) => Ok(Err(refused(format!("questions: {}", e)))),
        }
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
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let prowler = match self.file(&args.prowler) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, _org) = match self.inputs(&estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::compliance::triage_rows(
            &args.framework,
            &self.ctx.runtime.presets_dir,
            &claims,
            &manifest,
            &prowler,
        ) {
            Ok((_catalog, rows)) => Ok(Ok(Json(rows))),
            Err(e) => Ok(Err(refused(format!("triage: {}", e)))),
        }
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
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::pipeline_b_generate(&estate, &self.ctx.tool, &self.ctx.runtime) {
            Ok(out) => Ok(Ok(Json(CompileSummary {
                estate: estate.display().to_string(),
                addresses: out.manifest.addresses().into_iter().collect(),
            }))),
            Err(e) => Ok(Err(refused(format!("transpile --check: {}", e)))),
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
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::presets::check_presets_report(
            &estate,
            &self.ctx.runtime.presets_dir,
            &self.ctx.runtime.include_dirs,
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
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        match crate::pipeline_b_generate(&estate, &self.ctx.tool, &self.ctx.runtime) {
            Ok(out) => Ok(Ok(Json(CompileSummary {
                estate: estate.display().to_string(),
                addresses: out.manifest.addresses().into_iter().collect(),
            }))),
            Err(e) => Ok(Err(refused(format!("transpile: {}", e)))),
        }
    }

    #[tool(
        name = "satz_report_compliance",
        output_schema = rmcp::handler::server::tool::schema_for_output::<serde_json::Value>(),
        description = "Evidence report: the goal view joined with LIVE verification through Cloud \
                       Asset Inventory, manual-duty attestations and optional Prowler corroboration. \
                       Reads the organisation with the estate's credentials. Writes nothing — unlike \
                       the command, it does not append to the evidence history, because being ASKED \
                       for state is not a report run.",
        annotations(read_only_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn report_compliance(
        &self,
        Parameters(args): Parameters<ReportComplianceArgs>,
    ) -> Result<Result<Json<serde_json::Value>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let prowler = match args.prowler.as_deref().map(|p| self.file(p)).transpose() {
            Ok(p) => p,
            Err(r) => return Ok(Err(r)),
        };
        let (manifest, claims, org_id) = match self.inputs(&estate) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        match crate::compliance::report_compliance_evidence(
            &args.framework,
            &estate,
            &self.ctx.runtime.presets_dir,
            &claims,
            &manifest,
            org_id.as_deref(),
            &self.ctx.root,
            prowler,
            None,
            args.no_live,
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
        description = "Which identity, credential type and quota project the Application Default \
                       Credentials resolve to. The first thing to check when a live call is refused.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = true)
    )]
    async fn whoami(
        &self,
        Parameters(args): Parameters<WhoamiArgs>,
    ) -> Result<Result<Json<crate::gcp::identity::WhoamiReport>, CallToolResult>, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(Err(r));
        }
        match crate::gcp::identity::whoami_report(args.offline).await {
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
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` and `Implementation` are #[non_exhaustive]: build from the
        // default and assign, so a field added upstream cannot break this.
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info.name = "satz".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(format!(
            "satz compiles an estate written in Satz to OpenTofu HCL and judges it against \
             compliance catalogs. It never calls a model: ask it for facts, and do the judging \
             yourself. Capability level in force: '{}'.",
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
    tool: ToolConfig,
    runtime: ToolConfig,
    root: PathBuf,
    ceiling: Level,
    self_gated: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!(
        "satz mcp: serving on stdio at level '{}'{}",
        ceiling.describe(),
        if self_gated { ", self-gated" } else { "" }
    );
    let service = match SatzMcp::new(tool, runtime, root, ceiling, self_gated)
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
    use super::{Group, Level};

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

    #[test]
    fn describe_names_every_granted_group() {
        assert_eq!(Level::parse("read,write,exec").unwrap().describe(), "read,write,exec");
        assert_eq!(Level::default().describe(), "nothing");
    }
}
