# satz mcp

The estate, over the Model Context Protocol. An MCP client — Claude Code, Cursor, any
other — drives satz. satz calls no model, needs no API key and is deterministic; only
the tools marked `openWorldHint` below reach the network.

```bash
satz mcp                          # read-only, the default
satz mcp --allow read,write       # …and may write files in the estate
satz mcp --allow read,write --self-gated
```

It speaks JSON-RPC on stdio and is started by the client, not by you.

Started by hand, it exits immediately, because the client speaks first:

```
satz mcp: stdin closed before the client said hello — nothing to serve, exiting.
```

To exercise it by hand, pipe a handshake in:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | satz mcp --root /path/to/estates
```

## What an agent may do

Three capability groups, granted independently. Each covers a different kind of
consequence, not a severity level:

| group | what it covers |
|---|---|
| `read` | compile and report. Nothing is written, nothing external runs |
| `write` | writes files inside the estate — `hcl/`, adopted ids, the preset library |
| `exec` | runs an external tool, or changes a live organisation |

`--allow` sets a **ceiling the client cannot raise**. With `--self-gated` the client
may *lower* its own level at runtime through the `satz_restrict` tool and can never
raise it again — so an agent can prove it stayed read-only for a phase of its own work.

At every level **`self-update` is not exposed** (it replaces the binary), and
**every path argument is confined** to the directory the server was started in:
`use "…"` resolves through `include_dirs`, so an estate can pull in other files.

## Resources — what an agent reads before it writes

A client that only speaks MCP has no repository, and the tool schemas do not describe
the language. The server serves the documentation:

| resource | what it is |
|---|---|
| `satz://guide` | [satz for llms](llms.md) — the working subset, the three grant forms, and the order to call the tools in. **Read this before writing a `.satz` file.** |
| `satz://reference` | the complete language reference |
| `satz://presets` | the preset library and its provenance rules |

They are compiled into the binary, so they match the server's version and need no
path. The `instructions` returned at initialize name the guide. A unit test parses
every `satz` block in the guide.

## Tools

| tool | group | what it answers |
|---|---|---|
| `satz_estates` | `read` | which estates this server can open: every `config.toml` under its root, with the estate files beside it |
| `satz_open` | `read` | open one for the session — its `config.toml` and its main `.satz`. Answers with what it resolved, including the identity that estate's live tools will run as |
| `satz_require` | `read` | which controls of a catalog the **declared** estate satisfies, from its packs' claims. Offline |
| `satz_questions` | `read` | every question the estate's packs declare with its state — `answered` when the estate's own params bind it, else `unanswered` with the default the pack offers or `blocking` when none is possible — and `summary.complete`, the gate bootstrap and apply refuse on |
| `satz_interview` | `read` / `write` | the interview: the open questions (or all, with `filter: all`), each with its pack's description and its offer. With `write`: `create` writes the estate first, `answers` writes what the human decided, `accept_defaults` writes every offer — and the report comes back as it now stands. [satz interview](interview.md) |
| `satz_triage` | `read` | a Prowler export's FAILs sorted into buckets A–E against what the estate claims |
| `satz_transpile_check` | `read` | compiles in memory and reports what it *would* emit — writes nothing |
| `satz_check_presets` | `read` | which packs are clean, behind upstream, locally edited, or changed only in the questions they ask |
| `satz_report_compliance` | `read` | the goal view joined with **live** verification through Cloud Asset Inventory, attestations and optional Prowler corroboration |
| `satz_whoami` | `read` | both halves of the identity — the ADC account and the open estate's service account — with the live checks that decide whether the next call works: may this credential become that account, is the quota project reachable, does it hold the permissions the estate's resource types need (`permissions`, each missing one named with its role). The first thing to check when a live call is refused |
| `satz_transpile` | `write` | compiles the estate and writes its OpenTofu HCL into `hcl_dir`, as `satz transpile` does; `written` lists the files |
| `satz_adopt` | `read` / `write` | every declared resource resolved against the **live** organisation, as the estate's service account: per row whether it would be imported, moved in the state, is already managed, or cannot be resolved, what it matched on, and the Satz line that declared it. With `execute` (`write`) the verified ids are written into the estate as `"import-id"`, refused while any row is unanswered or a live object is declared twice. `tofu import`, a state move and activating a managed constraint stay on the command line |
| `satz_iac_roles` | `read` / `write` | the roles the estate's IaC service account needs for the resource types it emits against the roles it grants: the gap, the fewest roles that close it, and the emitted types the role table has no row for. Offline. With `execute` (`write`) the missing roles are written into the estate file and the estate is re-checked — a gap that survives the write restores the file |
| `satz_merge_presets` | `write` | the reconciling update: what was installed, taken as doc/format only, forked to `X.local.satz` with the estate repointed, adopted in place, deferred or refused — as events in the walk's own order, with the counts and `attention`. `adopt` takes upstream in place for the packs named (`all` for every pack merely behind) and the answer carries the emission delta; `report_only` writes nothing |
| `satz_get_presets` | `write` | the upstream library into the open estate's `presets_dir`: missing files installed, identical ones left, changed ones the estate does not use refreshed; a pack the estate uses that upstream changed is refused unless `force`. `pristine_dir` copies from a library under the root instead of downloading. `presets_dir` must be inside the root |
| `satz_remediation_items` | `read` | the remediation dossier's items for an estate and a Prowler export — triaged, deduplicated, joined per (control, resource) — with the `dossier_sha256` authored values must name. The worklist for the `[Authored]` columns; `checkov: true` joins a Checkov run and needs `exec` |
| `satz_remediation_annotate` | `write` | writes authored values (`what_why`, `recommended_fix`, `owner`, `effort`, `phase`, `quick_win`, `risk_acceptance`, and the mandatory `authored_by` and `authored_at`) per item id into `<out>/authored.json`, merged with what is on file, and renders the run there with the `[Authored]` columns filled. Refused when the hash is not the current dossier's, an id is unknown, or an entry names no author |
| `satz_scan_checkov` | `exec` | Checkov over the HCL in `hcl_dir` — the counts, and every failed check with the Satz file and line that declared the resource. Scans what is written: transpile first. Runs `checkov`, else `uvx checkov`, with its output captured |
| `satz_restrict` | — | lowers this session's level; only with `--self-gated` |

Each returns the same value the corresponding `--format json` command prints.

**Which commands an agent can run is a decision per command.** `MCP_PARITY`
(`src/mcp.rs`) names every CLI command with the tool that serves it or the reason
none does, and a command in neither column fails `cargo test` — the same way every
command must declare an identity. What is deliberately not served, by class: the commands
that hand stdio to `tofu` (`plan`, `apply`, `hcl-init`), the day-0 ones that run as
the human (`init`, `bootstrap`), the ones that write to an organisation
(`run-actions`, `adopt-org-policies`), the live sweep that rewrites an estate
(`import`), the specialist org-policy tools the compliance plane answers for
(`export-`, `diff-` and `report-organizational-policies`), the maintainer refreshes
of shipped data (`map-types`, `update-schema`, `doc-packs`), the `tofu`-workflow
plumbing (`scan-plan`, `generate-migration`, `migrate`), and the terminal
affordances (`completion`, `open-readme`, `self-update`). The table in `src/mcp.rs`
is the full list, with a reason per command. The tools
the table has no command for — `satz_open`, `satz_estates`, `satz_restrict` — are
the session and capability plumbing a terminal does not need.

A tool the level does not permit returns a tool **result** with `isError`, not a
protocol error, so the agent can continue.

**`satz_report_compliance` reads live and writes nothing.** The CLI command appends
every run to the append-only evidence history, the audit trail of reports someone ran;
the tool does not append. The smoke matrix compares the evidence directory across a
call.

**The rows are data, not a rendered table.** Each witness is an object — address,
live state, the id it matched, and `declared_at`, the `file:line` of the Satz that
declares it. Each control carries a `responsibility` (`inherited` · `customer` ·
`shared` · `satz-managed` · `unassigned`) and the report carries the estate's commit
and whether the tree was dirty. An agent renders the audit list, the spreadsheet or
the remediation commands from these fields. `declared_at` points each witness at the
Satz line that declares it, at the report's commit.

**A report that verified nothing says so.** `live` is whether the inventory was
actually READ, not whether it was asked for; `live_status` says which of the five
outcomes it was — `verified`, `skipped` (`--no-live`), `no-organization-id`,
`no-witnesses`, `unavailable` — and `warnings` carries the reasons, in the words the
command prints to its terminal. The report degrades to unverifiable witnesses rather
than failing the run; `live` and `live_status` tell a caller that cannot read stderr —
an agent, a pipeline reading `--format json` — whether the witnesses were verified.

## Structured output

Every data tool returns its report as **`structuredContent`** and publishes the
report's **`outputSchema`** in `tools/list`. A client gets a typed value it can index,
and knows the shape before it calls.

## Annotations

`--allow` sets what the **server** permits. Tool annotations tell the **client** what
it may run without asking:

| annotation | on |
|---|---|
| `readOnlyHint: true` | `satz_open`, `satz_estates`, `satz_require`, `satz_questions`, `satz_triage`, `satz_transpile_check`, `satz_check_presets`, `satz_report_compliance`, `satz_whoami`, `satz_scan_checkov`, `satz_remediation_items` |
| `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true` | `satz_iac_roles` — reading is free, `execute` writes the estate and is refused below `write`; `satz_transpile` — it writes, but re-running it converges; `satz_remediation_annotate` — the same values written twice leave the same run; `satz_adopt` — reading is free, `execute` writes the estate and is refused below `write`; `satz_interview` — reading is free, `create`/`answers`/`accept_defaults` write the estate and are refused below `write`; `satz_restrict` — it lowers this session's level and nothing else |
| `destructiveHint: true` | `satz_get_presets` — with `force` it overwrites packs the estate uses |
| `openWorldHint: true` (also) | `satz_merge_presets` — without `pristine_dir` it fetches the upstream library |
| `openWorldHint: true` | `satz_check_presets`, `satz_report_compliance`, `satz_whoami`, `satz_scan_checkov`, `satz_adopt`, `satz_get_presets`, `satz_merge_presets` — the ones that can reach the network (`uvx checkov` fetches Checkov) |

Without annotations a client either prompts on every read or runs a write without
asking. The ceiling decides what is *possible*; the annotations decide what runs
without a prompt.

## Claude Code

```json
{
  "mcpServers": {
    "satz": {
      "command": "satz",
      "args": ["mcp", "--root", "/path/to/estates", "--allow", "read"]
    }
  }
}
```

## The MCP SDK

satz serves MCP through [`rmcp`](https://crates.io/crates/rmcp), which implements all
five revisions of the protocol. The current revision (**2026-07-28**) negotiates the
protocol version *per request* through a `_meta` key, adds a mandatory
`server/discover` RPC, and keeps a compatibility path for the initialize-based
revisions clients still speak.

## stdout is the protocol

Everything satz says to a human — the version banner, schema-loader progress, emitter
warnings, the `credentials:` line — goes to **stderr**. Under MCP a stray line on
stdout corrupts the stream, and the client reports nothing useful. The smoke matrix
asserts that every line the server emits parses as JSON-RPC.

The matrix runs without credentials, so it never reaches a **live** tool call. Two
unit tests cover that path: one scans `src/mcp.rs`, the other scans the code a tool
reaches transitively — the token chokepoint `gcp::access_token()` and the announce
path. A new live code path on a tool's route belongs in the second one.

## One server, a fleet, one identity per call

The server holds **no estate** until a client opens one. `satz_open` names a
`config.toml` and a main `.satz`; everything after works on that estate, under that
config — its presets, its schemas, its provider version. Call it again for the next
estate. `satz_estates` lists what is available under the root, so the first call is not
a guess at a path.

The root, given as `satz mcp --root <dir>`, is a **boundary, not a configuration**: every
config and estate a tool resolves must live inside it, and anything outside is refused by
name. It is the only thing the server is started with: estates do not share a
`config.toml`, so each estate's own config comes with `satz_open`, and `satz mcp` takes
no `--config`.

### Which identity, and who decides

A live tool runs as the estate it is working on: for `deployment_mode = "cloud"`, that
estate's IaC service account, exactly as the same command does from the shell. Nothing is
configured and no tool sets it. The ADC authenticates, and satz's first act is to exchange
it for the account the estate itself names — `svc_iac_account` + `infra_project_name`, the
same derivation the emitted provider block uses. `satz_open` reports the result as
`runs_as` so it is stated rather than assumed, and `null` there means the estate
impersonates nothing and the calls are the ADC identity itself.

**The identity is scoped to the call, not bound to the process**, so one server works
through a fleet. It is a scope rather than a mutable global because the server
dispatches requests concurrently: a global changed underneath a call in flight would run
one estate's tools with another estate's credentials. Work started under one estate
finishes under it.

`satz_whoami` answers for the open estate, inside the same scope the other tools use.
With nothing open it answers for the ambient credentials, as `satz whoami` does with no
estate. `--no-impersonate` outranks the scope: every tool then runs as the plain ADC.

### Not available

- **No `tofu` tool.** `satz plan` and `satz apply` inherit stdio — apply's approval prompt
  is interactive — and under MCP stdin and stdout are the protocol; a human runs them.
- **`satz adopt --execute --import` is not exposed** — the `tofu import`, the state
  moves and activating a managed constraint stay with a human; `satz_adopt` writes the
  ids.
- **No progress notifications.** `satz_check_presets` downloads the whole pristine
  library with no feedback to the client.
