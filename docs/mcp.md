# `satz mcp` — the estate, over the Model Context Protocol

satz never calls a model. This is the other direction: an agent you already trust —
Claude Code, Cursor, your own — drives satz, and satz stays deterministic, keyless and
offline by default. Nothing in this surface talks to a model and nothing needs an API
key.

```bash
satz mcp                          # read-only, the default
satz mcp --allow read,write       # …and may write files in the estate
satz mcp --allow read,write --self-gated
```

It speaks JSON-RPC on stdio and is started by the client, not by you.

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

## Tools

| tool | group | what it answers |
|---|---|---|
| `satz_require` | `read` | which controls of a catalog the **declared** estate satisfies, from its packs' claims. Offline |
| `satz_check_presets` | `read` | which packs are clean, behind upstream or locally edited, and the remedy for each |
| `satz_transpile` | `write` | compiles the estate to OpenTofu HCL in `hcl_dir` |
| `satz_restrict` | — | lowers this session's level; only with `--self-gated` |

Each returns the same JSON the corresponding `--format json` command prints, so a
tool result and a CLI answer are the same value.

A tool the level does not permit comes back as a tool **result** with `isError`, not a
protocol error — an agent recovers from the first and gives up on the second.

## Claude Code

```json
{
  "mcpServers": {
    "satz": {
      "command": "satz",
      "args": ["--config", "/path/to/estate", "mcp", "--allow", "read"]
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
warnings — goes to **stderr**, and has since the reporting commands learned `--format
json`. Under MCP that is not a convenience: a stray line on stdout is a corrupt stream,
and the client reports nothing useful rather than reporting an error. The smoke matrix
asserts that every line the server emits parses as JSON-RPC.

This is also why no `exec` tool exists yet, though the group is grantable. `tofu` and
Checkov inherit stdio from the CLI; an exec tool has to **capture** its child's output
first, and that plumbing ships with the first such tool rather than being hurried in
beside the transport.
