# 0018 — editor intelligence comes from the satz binary

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.56.2

## Context

The Zed extension (ADR 0016) gives Satz files colour, an outline and brackets; the
formatter (ADR 0017) gives them a layout. What an editor still lacked was what satz
knows: whether the file compiles, which attributes a resource type has, what a param is
bound to, where a `use` path leads. Every one of those answers exists in the binary —
`satz::parse`, the fragment pipeline behind `transpile --check`, the provider schema
registry, the loader's `use` resolution — and none reached an editor.

The Language Server Protocol is how an editor asks. The decisions were where the server
lives, which parser it uses, when it runs the pipeline, and what a pack file — which has
no estate of its own to compile — gets.

## Considered options

1. **A server in the satz binary, `satz lsp`, on satz's own front end.** The parser and
   pipeline that produce the CLI's errors produce the editor's, with the same message at
   the same line; completion and hover read the schema the estate's `config.toml` points
   at; formatting is `satz fmt`. One binary to install; the server's version is the
   compiler's. Costs: an LSP transport in the binary, and a lexer that had to grow spans.
2. **A server on the tree-sitter grammar** (a separate binary, or Zed's own tree-sitter
   queries for symbols). Positions for free, but no compile, no schema, no `use`
   resolution — everything the editor was missing stays missing — and a second parser
   that can disagree with the compiler.
3. **Diagnostics through the pipeline on every keystroke.** The truest answer, and a
   whole-estate compile — every pack the estate uses, every fragment folded — per
   character typed; over a fleet-sized estate that is a stutter the author blames on the
   editor.

## Decision

Option 1. `src/lsp.rs`, beside `src/mcp.rs`, on `lsp-server` (rust-analyzer's transport:
a synchronous loop over stdio, no second runtime) and `lsp-types`. Zed starts it through
the extension's Rust glue, which does nothing but find `satz` on the PATH and run
`satz lsp`; a `binary.path` in Zed's lsp settings overrides it.

Parse on every change, pipeline on open and save. `satz::parse` is microseconds and its
error is a line; the pipeline runs when the author says the file is ready, the way
`transpile --check` does, and reads the editor's open buffers rather than the disk so an
estate with an unsaved pack still compiles as the author sees it. The pipeline's errors
are attributed to the file they name — the loader records where every `use` path
resolved — and published to that file, so a pack's error shows in the pack.

A pack is compiled through the estates beside it that use it: every `estate` file in the
estate directory whose source names the pack's file. That is what `transpile --check`
would find, and it is the only compile a pack has.

Completion and hover come from the registry `config.toml`'s `schema_dir` holds, loaded
once per estate directory; `AttributeSchema` and `BlockSchema` gained the `description`
the provider ships, which nothing had read before. Where the cursor is — the enclosing
resource type, the nested blocks below it, key or value position — is read from the
token stream with its trivia, the same stream the formatter uses. A resource type's entry
name is the one frame the walk skips; a list of objects is one nested-block level.

## Consequences

- The server knows an estate through its `config.toml`, found by walking up from the
  file. A `.satz` file with no `config.toml` above it gets parse diagnostics only, and so
  does one whose `schema_dir` is empty: `satz update-schema` is the fix, and the server
  says nothing rather than naming every type unknown.
- Diagnostics from the fold — two bodies for one address — are published at every
  contributing line, in every file involved, because the conflict has no single site.
- 2026-09-13, follow-up: the checks after the front end — the emitter, written
  references, missing required attributes, the IaC role table, unadopted packs, the
  providers, actions, passthrough blocks — are one function, `compile_tail`, returning
  structured findings (`src/findings.rs`: severity, kind, file, line, message, group)
  for three readers: the CLI renders them as it always did and refuses on an error,
  the server publishes each as a diagnostic at the line it names (the estate's first
  line when it names none), and `satz_transpile_check` returns them as data, warnings
  included — which MCP had lost entirely. One shape, so the three never disagree. A
  refused compile carries the list too: `CompileRefusal` is the error, rendering as
  the CLI's text and holding the findings behind it, so the MCP refusal hands over a
  `CompileSummary` with nothing emitted and every error at its line.
- The lexer's `Tok`, `Token` and `lex_spanned` are public API of `satz-core` now: two
  consumers, the formatter and the server.
