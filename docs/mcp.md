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
| `satz_transpile` | `write` | compiles the estate to OpenTofu HCL in `hcl_dir` |
| `satz_restrict` | — | lowers this session's level; only with `--self-gated` |

Each returns the same value the corresponding `--format json` command prints.

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
| `readOnlyHint: true` | `satz_require`, `satz_questions`, `satz_triage`, `satz_transpile_check`, `satz_check_presets`, `satz_report_compliance`, `satz_whoami` |
| `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true` | `satz_transpile` — it writes, but re-running it converges; `satz_interview` — reading is free, `create`/`answers`/`accept_defaults` write the estate and are refused below `write` |
| `openWorldHint: true` | `satz_check_presets`, `satz_report_compliance`, `satz_whoami` — the three that reach the network |

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

- **No `exec` tool**, though the group is grantable: `tofu` and Checkov inherit stdio
  from the CLI, and an exec tool has to **capture** its child's output first.
- **`adopt`, `merge-presets` and `get-presets` are not exposed.** Each prints from
  inside its own walk, which under MCP would write to stdout, the protocol stream; they
  need a compute/render split first.
- **No progress notifications.** `satz_check_presets` downloads the whole pristine
  library with no feedback to the client.
