# 0016 — editor support is a tree-sitter grammar in its own repository

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.56.1; the repository went public in v0.56.3 for Zed's registry, and the corpus-parse gate joined this repository's CI with it

## Context

Satz files are edited in Zed, which knew nothing about them: every estate and pack was
plain text. Zed defines a language through a [tree-sitter](https://tree-sitter.github.io)
grammar and nothing else — a TextMate grammar is not accepted, and the request for that is
an open discussion without an implementation — so highlighting means a tree-sitter grammar
for Satz, however it is sourced.

The language is small enough for that to be cheap. The whole lexical surface in
`crates/satz-core/src/satz.rs` is ten token kinds (identifier, string, number, `{ } [ ]
= ,` and the raw `hcl` body), there are no operators or expressions, and every keyword is
contextual, which tree-sitter's keyword extraction reproduces. The one construct that is
not context-free is the `hcl { … }` passthrough: a brace-balanced raw body that steps over
strings, comments and heredocs.

What was not obvious is where the grammar lives and how the editor finds it. Zed pins a
grammar by repository URL plus commit and clones it at build time from that URL; it
compiles `src/parser.c` itself, so the generated parser must be committed.

## Considered options

1. **Map `*.satz` onto the HCL language of Zed's `terraform` extension** with a
   `file_types` setting. Five minutes. Comments, strings, numbers, `key = value` and
   nested blocks read correctly and `hcl` bodies perfectly; but `estate`, `use`,
   `suppress`, quoted keys (`"import-id" = …`, IAM member keys), comma-less lists and
   one-line `option x { label = … why = … }` are parse errors, and `{param}`
   interpolation is invisible. A stopgap that mis-reads the constructs that are Satz's own.
2. **A grammar inside the satz repository**, referenced by Zed through an undocumented
   `path` field under `[grammars.satz]`. One repository, one PR per language change — but
   the pin must name a commit that already exists, so the pin would always trail the
   language change by one commit, and the field it depends on has no stability promise.
3. **A grammar in its own repository, the extension in satz.** The tree-sitter
   convention: a grammar repository is what Helix, Neovim and Zed all consume. The
   extension (`editors/zed/`: configuration and queries) pins one commit; a language
   change is a grammar commit first, then a satz PR that bumps the pin and the queries
   together with the parser. Two repositories to keep in step.
4. **Borrow the `terraform` extension's `hcl` grammar** for a language named Satz. Works
   only while that extension is installed, and Zed's publishing rules forbid a language
   that uses a grammar its own extension does not declare.

Within option 3, the raw `hcl` body could be lexed by an external C scanner transcribed
from `scan_hcl_body`, or by a nested rule of braces, string tokens and text tokens with
comments as extras. The scanner handles heredocs; the rule does not. The corpus holds one
`hcl` block and no heredoc.

## Decision

Option 3. The grammar is the repository `satz-tree-sitter` (parser name `satz`, the
generated `src/` committed, MIT). It is private: it carries no privacy gate, so its own
test inputs use example values only, and the corpus it is verified against is this
repository's, cloned by its CI on every push and weekly. The Zed extension is
`editors/zed/` here, pinning a grammar commit. It was installed as a dev extension
while the grammar was private; the grammar is public now and the extension is
submitted to Zed's registry.

The `hcl` body is the nested rule, not a scanner. Its content between the braces is one
node, `hcl_content`, so the extension can hand exactly that text to Zed's HCL grammar as
an injection: Zed injects a captured node's full range, and the braces would otherwise
turn a resource block into an object literal for the HCL parser.

The queries live once, in the extension, in Zed's capture vocabulary. Zed resolves a node
that several patterns capture to the LAST matching pattern (it reads the capture stack
from the top), so `highlights.scm` lists generic patterns first and specific ones after;
a quoted key is `@property`, not `@string`, because the key pattern comes later.

`scripts/check-grammar.sh` parses every `.satz` under `presets/` and `tests/` with the
pinned grammar and fails on any error. It is not in this repository's CI, which holds no
credential for a private repository; the grammar repository's weekly run is the automatic
check, and the script is the pre-PR check for a language change.

## Consequences

- A statement the parser gains is not highlighted until the grammar follows and the pin
  is bumped; until then it reads as an error in the editor and nowhere else. The order
  of work is fixed: grammar commit, then the satz PR with the pin and the parser change.
- The grammar is a second hand-written description of the language. Where it and
  `satz.rs` disagree, the grammar is the bug; the corpus parse is what finds the
  disagreement.
- A heredoc with unbalanced braces inside an `hcl` body mis-parses the rest of that
  block. The external scanner is the remedy when a real estate carries one.
- `*.diff.satz` files are unified diffs with a `.satz` suffix; the extension opens them
  as Satz. A `file_types` setting mapping them to Diff is the per-user answer.
- Publishing in Zed's registry needs the grammar repository public, an `https://` pin, a
  license file inside `editors/zed/`, and weeks of review. The extension's `version`
  is bumped for every later update once that happens.
