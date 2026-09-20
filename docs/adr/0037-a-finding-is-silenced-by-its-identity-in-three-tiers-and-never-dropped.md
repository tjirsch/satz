# 0037 — a finding is silenced by its identity, in three tiers, and is never dropped from the machine stream

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

A compile reports what it finds as findings, and an estate that uses the CIS packs
produces eleven of them on every compile: ten open notices, one pack requirement, all at
one line, all saying what was read once and acted on. Nothing in satz could say "seen,
move on" except the two acknowledgements built into the language — `hcl trust "<reason>"`
and a notice's param — and neither covers a warning an operator has simply decided to
live with.

A key was needed before a mute could exist, and the finding had none:

- **The wording cannot be it.** A message carries paths, line numbers and counts. A
  notice group's header reads `10 notice(s) open`, so acknowledging one rewords the other
  nine, and a hash of the message would be resurrected by every rewording, every path
  change and every release.
- **`file` was not even stable between callers.** The same finding carried a path
  relative to the terminal's directory from the CLI and an absolute one through MCP.
- What every producer DID have, and threw away, was the thing the finding is about: the
  pack path in `packs.rs`, the notice's param (already the acknowledgement key), the
  action's name, the `hcl` block's location.

Two ad-hoc silencers already existed — `--no-action-warnings` and a caller that reports
the prerequisites in its own output — each with its own mechanism, neither reusable.

## Decision

**A finding's identity is `kind` + `subject`.** `kind` already existed and is published in
the MCP output schema; `subject: Option<String>` is new and is filled at the four
producers that have it. `file` is normalised to the estate's own directory everywhere, so
the CLI, the editor and an agent name one path.

**Three tiers name that pair, because the decision belongs to three different people:**

| tier | where | what it may name |
|---|---|---|
| estate | `[[silence]]` in the estate's `config.toml` | a kind, or one subject of a kind |
| machine | `[[silence]]` in `~/.config/satz/satz.toml` | a whole kind only |
| run | `--silence <kind>[:<subject>]`, `SATZ_SILENCE` | a kind, or one subject of a kind |

Every row carries a mandatory `reason`, as a `deviates` does: a row without one is a TOML
error naming the file and the line, and an unknown `kind` or key is the same. The estate
tier is versioned with the estate, so a review sees who silenced what and why, and
reversing it is an edit. The machine tier takes no subject, so a mute one operator took
for one customer cannot hide a specific thing at another.

**The run tier replaces `--no-action-warnings`,** which is gone: one mechanism, not two.
It is refused for `satz mcp` and `satz lsp` — both live for many estates and many calls in
one process, and a silence given once on their command line would hold for all of them.

**A silenced finding is never dropped.** `Finding` gains `silenced: { tier, reason }`; the
list, `--format json` and what MCP returns keep it. Only the printed output leaves it out,
and a run that silenced anything ends with one line — `11 finding(s) silenced (11 estate)`
— so a silence is visible on every run even when its finding is not.

**An error is never silenced by any tier.** The marking happens in one place and skips
`Error`, so `refusal` never consults the field and cannot be bypassed. A `--silence` that
names an error refuses the run by name.

**The editor and an agent read the estate and machine tiers, not the run tier.** An estate
that is clean in the terminal is clean in the editor; what a CI run hid, an agent still
sees, and sees marked.

**`satz silence list|add|remove` manages the two written tiers.** `add` and `remove` edit
the estate's `config.toml` through a format-preserving TOML document, so the operator's
comments and layout survive. `list` with an estate compiles it and says per row how many
findings it silences — or `STALE`, when nothing answers to it any more.

## Options considered

1. **A hash of the message.** No new field, works for every producer. Rejected: every
   rewording, path change and count change mints a new key, so a mute lasts until the next
   release and an estate accumulates dead rows nobody can read.
2. **A `suppress`-style statement in the language.** Silencing would live in the estate
   text beside the thing silenced. Rejected: a finding is not a resource, the estate would
   carry statements about satz's own output, and the machine and run tiers have no file to
   live in.
3. **One tier only (the estate).** Simplest. Rejected: an operator's standing preference
   would have to be written into every customer's repository, and a CI pipeline cannot
   edit an estate to get past a warning.
4. **Drop a silenced finding outright.** Cheapest rendering, and what `--no-action-warnings`
   did. Rejected: a silence would then be invisible to a reviewer, an agent and a report,
   and a wrong one could not be found.
5. **Let a run downgrade a named error kind to a warning** (the CI case that motivates the
   run tier). Not decided here: it stays open, and until it is answered a `--silence` that
   names an error refuses the run rather than half-doing it.

## Consequences

- `--no-action-warnings` is gone. An input that stopped being read makes this a MINOR
  release (ADR-0010).
- A finding's identity is now part of what satz publishes: renaming a `Kind`, or changing
  what a producer puts in `subject`, breaks every `[[silence]]` row that named it. The
  kinds and their names come from one macro, so a new kind cannot be missing from the
  list `satz silence` reads.
- Subjects compare whole. There is no pattern and no wildcard, so a silence cannot grow to
  cover a finding nobody read; the cost is one row per `hcl` block and one per notice
  where a whole kind is too broad.
- A `[[silence]]` row an estate outgrows is dead weight until someone runs
  `satz silence list <estate>`. Nothing fails on a stale row — the compile is where noise
  was being removed, so the report lives in the command that is about silences.
- The group headers still carry the count they were produced with, so a partly silenced
  group prints a header that overcounts. The readable layout that replaces those headers
  is the other half of this work.
