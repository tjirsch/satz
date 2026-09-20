# 0039 — a finding is laid out by one renderer, and the command that answers it is a field

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

The compile's findings were data for two of their three readers and one unbroken line for
the third. `findings::render` printed `warning: <the whole message>`; the location was in
front of nothing and inside the prose only where a producer had written it there; a pack's
notice arrived as one line of about 300 characters holding the pack path, two sentences,
a command and a param. An estate using the CIS packs printed eleven of them.

Three more things were wrong in the same place:

- **A refused compile was printed by Rust's `Debug` formatter.** `main` returned
  `Result<(), Box<dyn Error>>`, so every error reached the terminal as `Error: "…"` with
  its quotes and newlines escaped, and a refused compile as
  `Error: CompileRefusal { message: "…", findings: [Finding { … }, …] }`. Eleven call
  sites worked around it by printing their detail themselves and returning a short error.
- **A front-end refusal was not a refusal.** A parse error, an unknown param or an entry
  that does not belong where it stands is a `PipelineError`; only a `CompileRefusal`
  carried findings, so `satz_transpile_check` answered a parse error with
  `structuredContent` absent and the location only inside the sentence.
- **A group's count was baked into its title** (`10 notice(s) open`), so a partly silenced
  group printed a title that overcounted (ADR 0037's last consequence).

`satz review-pack` had a second layout of its own (`severity: file:line: message`),
`satz packs` a third (the bare message), and `transpile --check` had no `--format`, so one
run could not be had as text and as data from the CLI.

## Decision

**One renderer, `findings::lay_out`, prints a finding for every command that prints one.**
Per finding: a first line of severity, `kind`, `file:line` and `subject`, in columns as
wide as their widest entry in the run; the message under it, indented four columns; and
the command that answers it as a last line of its own, `fix: …`. Findings of a group
stand under the group's title; findings of no group come first, so whatever stands under
a title belongs to it. Every finding and every title is a blank line from the one before.
One thing found at several sites — a conflict is one finding per site, so an editor marks
every file — is one first line per site over the one message. The run ends in one line:
the count by severity, then what was silenced by tier.

**`Finding` gains `fix: Option<String>`, and the command MOVES there.** `message` is the
sentence, `fix` is the command as it is typed — `<estate>` in a pack's `run` replaced by
the estate's file name — and it is in `fix` alone. A finding no ONE command answers (two
packs that exclude one another; a requirement several packs could meet) has no `fix` and
says so in its sentence. The location leaves the prose where the finding carries it.

**A group's title carries no count.** `group` is `notices open — what a pack asks to be
run once it is on`; the renderer appends `(10)`, or `(7 of 10, 3 silenced)`.

**The prose is wrapped for a terminal and for nothing else.** A terminal's width, at most
110 columns. A pipe, a file, a CI log and an MCP text block get each paragraph on one
line: what reads those matches substrings — the smoke matrix, a script, an agent working
from `docs/llms.md` — and a break inside the phrase it looks for is a match it misses.
The structure is the same either way. A `fix:` line is never wrapped: it is pasted.

**There is no colour.** The severity word in a fixed first column is the anchor; colour
would add a terminal-detection matrix (`NO_COLOR`, dumb terminals, CI) for no information
the word does not carry.

**A front-end refusal is a `CompileRefusal` with one finding,** of the new kind
`front-end`, at the parser's file and line. One refusal, one shape: MCP returns it as
`structuredContent`, `--format json` prints it, the CLI lays it out.

**`main` prints an error by `Display`.** A refused compile prints its errors in the layout
under the warnings the compile already printed, then the closing count; anything else is
`error: <what it says>`.

**`transpile --format json`** prints the `CompileSummary` that `satz_transpile_check` and
`satz_transpile` return — on a refusal too, with exit code 1 — and the compile prints
nothing beside it.

**The language server mirrors what a diagnostic can hold:** the range is the location, the
editor draws the severity, `code` is the kind, and the message is the group's title, the
sentence, and `fix: <command>` as its last line.

## Options considered

1. **Keep the command in `message` and repeat it in `fix`.** No client that reads
   `message` loses anything. Rejected: two homes for one fact is the dual-accept path this
   project does not keep, and the layout would print the command twice.
2. **Cut the command off the end of `message` by pattern when printing.** No schema change.
   Rejected: it holds until a producer words its sentence differently, and MCP and the
   editor would still get the unsplit line.
3. **Wrap at a fixed width when stderr is not a terminal.** A CI log would read like a
   terminal. Rejected for the reason above: a wrapped pipe breaks every substring match on
   it, silently, and a log viewer soft-wraps a long line anyway.
4. **Keep the count in `group` and let the renderer rewrite the leading number.** The JSON
   value would not move. Rejected: it is the pattern-matching of option 2 on a different
   string, and the count in the JSON is wrong for the same reason it was wrong on screen —
   it counts what was produced, and a reader of the JSON can count.
5. **Report a front-end error under an existing kind** (`emit`, `conflict`). No enum
   widening. Rejected: a kind says which check spoke, and none of them did.
6. **Colour on a terminal.** Considered and left out; it can be added without moving
   anything else.

## Consequences

- The printed wording moved, and with it every contract on it, in the same change: the
  smoke matrix's greps, `docs/llms.md`'s substring table, the transcripts in
  `docs/language.md` and `README.md`. A program that parsed the old `warning: …` line, or
  the `Debug` dump of a refusal, reads `--format json` instead.
- The JSON schema WIDENED: `fix` is a new optional field and `front-end` a new value of
  `kind`. No field was renamed or removed. The VALUES of `group` and `message` changed —
  a title without its count, a sentence without its command and its location.
- An `hcl` block's `subject` is spelled relative to the estate's directory by every
  reader. It was the path as the caller spelled it, so a `[[silence]]` row written from
  one reader's output did not answer to another's; a row that named the old spelling reads
  `STALE` in `satz silence list` and is written again.
- The eleven call sites that print their detail and return a short error still do; they no
  longer have to. Each can return its whole message when it is next touched.
- Column widths are per run, so the same finding sits at a different column in two runs. A
  reader that needs a position reads the JSON.
