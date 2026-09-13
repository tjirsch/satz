# 0017 — the formatter keeps the author's line breaks

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.56.2

## Context

Satz had no formatter. The corpus is hand-formatted to one convention — two-space
indentation, `=` aligned over a run of attributes, a trailing comma on every item of a
list laid out over lines — and every pack and estate kept it by care, not by a tool.
An editor with a language server wants a formatting provider, and a repository with
seventy Satz files wants a gate that says whether they are in shape.

What made the design a decision is the parser. `satz::parse` builds an AST that carries
no comments and no columns, only line numbers; `canonical_parts` prints that AST as a
comparison key for `check-presets`, dropping comments and layout on purpose. Neither
is a printer a formatter could be built on without first deciding what a formatter is
allowed to change.

## Considered options

1. **A printer over the AST**, after teaching the parser to keep comments. Every line
   break, every blank line and every inline block becomes the printer's decision, so
   the output is fully canonical — and every file in the corpus is rewritten to the
   printer's taste, including the single-line `option x { label = "…" }` and
   `local { path = "…" }` forms the corpus uses deliberately. The parser gains a
   second concern (positions and trivia) that the compiler never needs.
2. **A formatter over the token stream with its trivia.** The lexer gains char spans
   and, on request, comment and line-end tokens; the parser sees exactly what it saw
   before. The author's line breaks are the input: a construct written on one line
   stays on one line, one written over lines opens at the end of its line and closes
   on a line of its own. What the formatter decides is indentation, spacing inside a
   line, `=` alignment over a run of attributes, the commas of a list, and blank lines
   (at most one, none against a brace). Strings and `hcl { … }` bodies are verbatim.
3. **A formatter over the tree-sitter grammar** (ADR 0016), where comments are nodes
   already. A second parser in the satz binary, a git dependency on a private
   repository for `cargo build --locked`, and a formatter that agrees with the editor
   grammar rather than with the compiler.

## Decision

Option 2: `crates/satz-core/src/fmt.rs`, served by `satz fmt` and, later, by the
language server as its formatting provider. The formatter's law is the one
`check-presets` already applies: `canonical(parse(format(x))) == canonical(parse(x))`,
tested over every Satz file in the repository together with idempotency, and every
file in the repository is formatted (`cargo test` and `satz fmt --check` in the smoke
matrix both say so).

Alignment is per run: consecutive attributes at one depth, a comment line inside the
run transparent, a blank line or anything else ending it, the widest key setting the
column — the way `terraform fmt` does it. The corpus had a second convention beside
it, an over-long key overflowing with one space while its neighbours kept a narrower
column; the run rule replaces it, because a formatter that reproduces an author's
judgement about which key is too long is one nobody can predict.

A file the parser refuses is not formatted; the error is the parser's.

## Consequences

- Fifty-four of seventy-four corpus files changed in the PR that shipped this, all
  layout: the compiler's view of every one is identical, which the canonical-form test
  proves rather than asserts. A reformatted pristine pack is not drift for an estate:
  `merge-presets` compares canonical forms and upgrades comment and format churn in
  place.
- The lexer now carries spans and trivia. `Tok::Comment` and `Tok::Newline` exist for
  the formatter alone; `parse` never receives them, which `lex` guarantees by
  construction rather than by filtering.
- A single line carrying two entries (`a = 1 b = 2` inside a body laid out over lines)
  stays one line: the formatter does not split entries. The corpus has none, and the
  rule that would (which keyword continues a statement, which starts the next) is
  parser knowledge the formatter would have to duplicate.
- Heredocs and everything else inside an `hcl { … }` body are the author's; the body
  is HCL, and `tofu fmt` is its formatter.
