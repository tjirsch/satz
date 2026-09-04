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
use rmcp::handler::server::wrapper::Parameters;
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

fn ok_json(value: &impl serde::Serialize) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("could not serialise the report: {e}"), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

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
        if name.contains('\0') {
            return Err(refused("the estate name is not a path".into()));
        }
        let p = crate::estate_path(PathBuf::from(name), &self.ctx.runtime);
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

    #[tool(
        name = "satz_require",
        description = "Goal view: which controls of a compliance catalog the DECLARED estate satisfies, \
                       from the claims of the packs it uses. Offline, reads nothing live. Returns JSON."
    )]
    async fn require(&self, Parameters(args): Parameters<RequireArgs>) -> Result<CallToolResult, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(r);
        }
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(r),
        };
        let inputs = crate::compliance_inputs(&estate, &self.ctx.tool, &self.ctx.runtime);
        let (manifest, claims, _org) = match inputs {
            Ok(v) => v,
            Err(e) => return Ok(refused(format!("{}: {}", args.estate, e))),
        };
        match crate::compliance::require_report(
            &args.framework,
            &estate,
            &self.ctx.runtime.presets_dir,
            &claims,
            &manifest,
        ) {
            Ok(report) => ok_json(&report),
            Err(e) => Ok(refused(format!("require {}: {}", args.framework, e))),
        }
    }

    #[tool(
        name = "satz_check_presets",
        description = "Preset drift: which packs in the local library are clean, behind upstream or \
                       locally edited, with the remedy for each. Downloads the pristine library to \
                       compare. Returns JSON."
    )]
    async fn check_presets(&self, Parameters(args): Parameters<EstateArg>) -> Result<CallToolResult, McpError> {
        if let Err(r) = self.permits(Group::Read) {
            return Ok(r);
        }
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(r),
        };
        match crate::presets::check_presets_report(
            &estate,
            &self.ctx.runtime.presets_dir,
            &self.ctx.runtime.include_dirs,
            None,
        )
        .await
        {
            Ok(report) => ok_json(&report),
            Err(e) => Ok(refused(format!("check-presets: {}", e))),
        }
    }

    #[tool(
        name = "satz_transpile",
        description = "Compile the estate to OpenTofu HCL and WRITE it into the configured hcl_dir. \
                       Needs the 'write' capability."
    )]
    async fn transpile(&self, Parameters(args): Parameters<EstateArg>) -> Result<CallToolResult, McpError> {
        if let Err(r) = self.permits(Group::Write) {
            return Ok(r);
        }
        let estate = match self.estate(&args.estate) {
            Ok(p) => p,
            Err(r) => return Ok(r),
        };
        match crate::pipeline_b_generate(&estate, &self.ctx.tool, &self.ctx.runtime) {
            Ok(out) => ok_json(&serde_json::json!({
                "estate": estate.display().to_string(),
                "hcl_dir": self.ctx.runtime.hcl_dir,
                "addresses": out.manifest.addresses().len(),
            })),
            Err(e) => Ok(refused(format!("transpile: {}", e))),
        }
    }

    #[tool(
        name = "satz_restrict",
        description = "Lower this session's capability level for the rest of the connection. It can \
                       only ever shrink — a level cannot be raised back, and never above the ceiling \
                       the server was started with. Available only with --self-gated."
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
    let service = SatzMcp::new(tool, runtime, root, ceiling, self_gated)
        .serve(rmcp::transport::stdio())
        .await?;
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
