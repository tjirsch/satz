# satz mcp

The estate, over the Model Context Protocol. satz never calls a model. This is the other direction: an agent you already trust —
Claude Code, Cursor, your own — drives satz, and satz stays deterministic, keyless and
offline by default. Nothing in this surface talks to a model and nothing needs an API
key.

```bash
satz mcp                          # read-only, the default
satz mcp --allow read,write       # …and may write files in the estate
satz mcp --allow read,write --self-gated
```

It speaks JSON-RPC on stdio and is started by the client, not by you.

Running it by hand is the first thing anyone tries, and it exits immediately with

```
satz mcp: stdin closed before the client said hello — nothing to serve, exiting.
```

That is correct behaviour, not a failure: the client speaks first. To exercise it
yourself, pipe a real handshake in:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | satz mcp --root /path/to/estates
```

## What an agent may do

Three capability groups, granted independently. They are **not** a severity ladder —
they are three different kinds of consequence:

| group | what it covers |
|---|---|
| `read` | compile and report. Nothing is written, nothing external runs |
| `write` | writes files inside the estate — `hcl/`, adopted ids, the preset library |
| `exec` | runs an external tool, or changes a live organisation |

`--allow` sets a **ceiling the client cannot raise**. With `--self-gated` the client
may *lower* its own level at runtime through the `satz_restrict` tool and can never
raise it again — so an agent can prove it stayed read-only for a phase of its own work.

Two things hold at every level: **`self-update` is never exposed** (it replaces the
binary), and **every path argument is confined** to the directory the server was
started in. That second one is not paranoia — `use "…"` resolves through
`include_dirs`, so an estate path is not merely a file to read.

## Resources — what an agent reads before it writes

A client that only speaks MCP has no repository. The tool schemas say how to *call*
satz; nothing in them says how to write the language the calls are about. So the server
serves the documentation itself:

| resource | what it is |
|---|---|
| `satz://guide` | [satz for llms](llms.md) — the working subset, the three grant forms, and the order to call the tools in. **Read this before writing a `.satz` file.** |
| `satz://reference` | the complete language reference |
| `satz://presets` | the preset library and its provenance rules |

They are compiled into the binary, so the answer travels with the server: no path to
configure, no copy to drift. The `instructions` returned at initialize point at the
guide by name, so an agent is told where to look before it is asked to write anything.

The guide's examples are gated: a unit test parses every `satz` block in it. That gate
caught a broken example on its first run — a doc an agent will act on cannot afford
one.

## Tools

| tool | group | what it answers |
|---|---|---|
| `satz_estates` | `read` | which estates this server can open: every `config.toml` under its root, with the estate files beside it |
| `satz_open` | `read` | open one for the session — its `config.toml` and its main `.satz`. Answers with what it resolved, including the identity that estate's live tools will run as |
| `satz_require` | `read` | which controls of a catalog the **declared** estate satisfies, from its packs' claims. Offline |
| `satz_questions` | `read` | every question the estate's packs declare, joined with the answers its params carry, and what changing each costs |
| `satz_triage` | `read` | a Prowler export's FAILs sorted into buckets A–E against what the estate claims |
| `satz_transpile_check` | `read` | compiles in memory and reports what it *would* emit — writes nothing |
| `satz_check_presets` | `read` | which packs are clean, behind upstream, locally edited, or changed only in the questions they ask |
| `satz_report_compliance` | `read` | the goal view joined with **live** verification through Cloud Asset Inventory, attestations and optional Prowler corroboration |
| `satz_whoami` | `read` | which identity, credential type and quota project the ADC resolve to — the first thing to check when a live call is refused |
| `satz_transpile` | `write` | compiles the estate to OpenTofu HCL in `hcl_dir` |
| `satz_restrict` | — | lowers this session's level; only with `--self-gated` |

Each returns the same value the corresponding `--format json` command prints, so a
tool result and a CLI answer are the same thing.

A tool the level does not permit comes back as a tool **result** with `isError`, not a
protocol error — an agent recovers from the first and gives up on the second.

**`satz_report_compliance` reads live and writes nothing.** The command appends every
run to an append-only evidence history — that history is the audit trail of a
*deliberate* report. Being asked for current state is not that, so the tool does not
append, and the smoke matrix compares the evidence directory across the call to prove
it.

**A report that verified nothing says so.** `live` is whether the inventory was
actually READ, not whether it was asked for; `live_status` says which of the five
outcomes it was — `verified`, `skipped` (`--no-live`), `no-organization-id`,
`no-witnesses`, `unavailable` — and `warnings` carries the reasons, in the words the
command prints to its terminal. The report degrades to unverifiable witnesses rather
than failing the run, so a caller that cannot read stderr — an agent here, a pipeline
reading `--format json` — would otherwise have no way to tell a blind run from a
verified one.

## Structured output, and why it matters

Every data tool returns its report as **`structuredContent`** and publishes the
report's **`outputSchema`** in `tools/list`. A client gets a typed value it can index,
and knows the shape before it calls.

This server's first version returned the same JSON as a *text block*. The data was
there and nothing said what shape it had, so every caller had to parse a string and
guess — which is the failure the structured-output part of the protocol exists to
prevent.

## Annotations — the client's half of the safety model

`--allow` is the **server's** answer to "what is permitted". Tool annotations are the
**client's** answer to "what may I run without stopping to ask":

| annotation | on |
|---|---|
| `readOnlyHint: true` | `satz_require`, `satz_questions`, `satz_triage`, `satz_transpile_check`, `satz_check_presets`, `satz_report_compliance`, `satz_whoami` |
| `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true` | `satz_transpile` — it writes, but re-running it converges |
| `openWorldHint: true` | `satz_check_presets`, `satz_report_compliance`, `satz_whoami` — the three that reach the network |

Without them a client either prompts on every read (friction that makes the server
tiresome) or auto-runs a write (wrong). Both halves are needed: the ceiling decides
what is *possible*, the annotation decides what is *unremarkable*.

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

## Why the SDK and not a hand-rolled shim

The plan for this said hand-roll ~300 lines: satz needs four methods and would never
touch the parts of the spec that move. Reading the spec before writing the first line
refuted it. The current revision (**2026-07-28**) negotiates the protocol version *per
request* through a `_meta` key, adds a mandatory `server/discover` RPC, and keeps a
separate compatibility path for the initialize-based revisions clients still speak.
That is three moving parts to own, in a spec that has revised five times.
[`rmcp`](https://crates.io/crates/rmcp) implements all five, so the churn is not ours —
which was the deciding question, rather than which option was less code.

## stdout is the protocol

Everything satz says to a human — the version banner, schema-loader progress, emitter
warnings, the `credentials:` line — goes to **stderr**, and has since the reporting
commands learned `--format json`. Under MCP that is not a convenience: a stray line on
stdout is a corrupt stream, and the client reports nothing useful rather than reporting
an error. The smoke matrix asserts that every line the server emits parses as JSON-RPC.

That check has a blind spot worth knowing about. The matrix runs without credentials, so
it never reaches a **live** tool call — and the first real bug here was exactly there:
the credential line was printed from behind `gcp::access_token()`, in a different module,
so the first live call corrupted the stream without a single `println!` in `src/mcp.rs`.
Two unit tests cover what the matrix cannot: one scans this module, the other scans the
code a tool reaches transitively — the token chokepoint and the announce path. A new
live code path on a tool's route belongs in the second one.

## One server, a fleet, one identity per call

The server holds **no estate** until a client opens one. `satz_open` names a
`config.toml` and a main `.satz`; everything after works on that estate, under that
config — its presets, its schemas, its provider version. Call it again for the next
estate. `satz_estates` lists what is available under the root, so the first call is not
a guess at a path.

The root, given as `satz mcp --root <dir>`, is a **boundary, not a configuration**: every
config and estate a tool resolves must live inside it, and anything outside is refused by
name. It is the only thing the server is started with.

That shape exists because estates do not share a `config.toml`. A server pinned to one
could serve only the estates that happened to agree with it — which is why `--config` is
gone from this command.

### Which identity, and who decides

A live tool runs as the estate it is working on: for `deployment_mode = "cloud"`, that
estate's IaC service account, exactly as the same command does from the shell. Nothing is
configured and no tool sets it. The ADC authenticates, and satz's first act is to exchange
it for the account the estate itself names — `svc_iac_account` + `infra_project_name`, the
same derivation the emitted provider block uses. `satz_open` reports the result as
`runs_as` so it is stated rather than assumed, and `null` there means the estate
impersonates nothing and the calls are the ADC identity itself.

**The identity is scoped to the call, not bound to the process.** That is the difference
that lets one server work through a fleet. It also has to be per call rather than a
mutable global, because the server dispatches requests concurrently — the smoke matrix
demonstrated two calls racing — so a global that changed underneath a call in flight would
hand one estate's tools another estate's credentials, at random, across customers. A
scope cannot do that: work started under one estate finishes under it.

Two earlier versions of this were wrong in instructive ways. The first bound nothing, so
`satz_report_compliance` read the organisation as the human while `satz report-compliance`
read it as the service account — same code, two principals, no visible difference in the
output. The second bound the **process** on the first live call and refused any estate
needing a different account, which was safe but meant restarting the server for every
estate; working through a fleet is ordinary, so that refusal was the wrong trade.

`satz_whoami` answers for whatever is open, inside the same scope its neighbours use, so
it describes the session an agent is about to get rather than a different one. With
nothing open it answers for the ambient credentials — the question `satz whoami` answers
with no estate. `--no-impersonate` still outranks everything: the operator asked for the
plain ADC, and a per-call scope is no more entitled to override that than a process
binding was.

### Still missing

- **No `exec` tool**, though the group is grantable. `tofu` and
Checkov inherit stdio from the CLI; an exec tool has to **capture** its child's output
first, and that plumbing ships with the first such tool rather than being hurried in
beside the transport.
- **`adopt`, `merge-presets` and `get-presets` are not exposed yet.** Each still prints
  from inside its own walk, so exposing one would put its output on stdout — which here
  is the protocol. They need the same compute/render split the reporting commands went
  through first. That is the work, not the tool definition.
- **No progress notifications.** `satz_check_presets` downloads the whole pristine
  library with no feedback to the client.
