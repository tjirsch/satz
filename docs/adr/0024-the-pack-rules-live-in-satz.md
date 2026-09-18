# 0024 — the rules a pack must clear live in satz, as a command

- **Status:** accepted
- **Date:** 2026-09-15
- **Shipped in:** v0.58.1

## Context

A pack is the unit everyone extends satz with, and the bar it has to clear is real:
it parses and is formatted, its header opens with a sentence the index can print, its
version has a changelog row, it declares no membership, it runs no legacy org-policy
constraint beside its managed replacement, every resource type it emits has a row in
the prerequisite table, and it compiles.

Every one of those rules was enforced by a gate INSIDE this repository — a `cargo test`,
`doc-packs --check`, the formatter's corpus test, the privacy script. All of them need a
satz checkout and a Rust toolchain. A pack written anywhere else could not be checked at
all: its author found out by opening a pull request, or never. `check-presets` sounds
like the missing command and is not — it reports drift of installed packs against the
pristine library, a different question.

The desktop app (satz-studio) wants a pack view: findings anchored to `file:line` in the
pack source, in the diagnostics drawer it already has. That view needs the rules. So
does an agent asked to proofread a pack without a checkout.

## Options

1. **The app implements the rules.** It has the UI and the file dialogs, and nothing
   needs to change in satz. Two copies of every rule, and the second copy is always the
   stale one — a rule that disagrees with itself is worse than a rule nobody checks.
2. **Leave them as repository gates and let the pull request be the check.** No work,
   and it is what happens today. It also means the only people who can write a pack with
   confidence are the people who can build satz.
3. **One command in satz, and the app renders what it returns.**

## Decision

Option 3, decided 2026-09-15: what satz can naturally do, satz does — checking and
proofreading a pack included. `satz review-pack <file> --format <fmt> --out <file>`
runs the bar in the order a pack fails it and returns the SAME `Finding` the compile,
`satz lsp` and `satz_transpile_check` already produce, so an editor or an app that reads
that shape needs nothing new. `satz_review_pack` serves it over MCP, read-only. The app
owns no pack rule; satz-studio's own ADR 0007 already refused a copied table.

**A pack is folded into a synthesised estate.** A pack is a fragment: what it emits is
only knowable inside an estate, and demanding one from the author would make the command
useless exactly when it is most needed. satz writes a throwaway estate that binds the
documented example values for the estate's own vocabulary and answers the pack's own
questions with the defaults it declares — which checks the pack the way a customer first
meets it, and names the questions no default can answer as the blocking questions they
are. `--against <estate>` judges it inside a real estate instead.

**The review says what adopting the pack costs.** The roles and the APIs its emitted
types need — the same two halves `update-prerequisites` writes into an estate (ADR 0023).
That is the number an author and an adopter both want, and it falls out of the table.

**The privacy shapes are not in it yet.** A pack written against its author's own
organisation carries project ids, domains and e-mail addresses, and those are what must
become params before it can leave that machine. The check is `scripts/check-names.sh`,
a shell script CI runs; moving its SHAPES into the binary would put the check where the
author who needs it most can run it, and is the obvious next step — but a second
implementation of a rule is exactly what this ADR refuses elsewhere, so it waits for a
decision about which copy is the source. The review says plainly that it did not look.

## Consequences

- A PATCH: a new command and a new read-only tool, nothing existing changes.
- satz-studio's pack view (U14) can start: it is a view over this command's output.
- A new rule for packs is added once, in satz, and every surface gets it — the gate, the
  command, the app, the agent.
- The scratch estate is written under the system temp directory and removed afterwards;
  a pack whose resources are all behind a `when` emits nothing there, and the review says
  so rather than passing silently.
