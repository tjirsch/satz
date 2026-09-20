# 0040 — findings that say the same thing share one block, and the producer says what is shared

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

ADR 0039 made a finding readable: a first line, the message under it, `fix:` last. It did
nothing about volume. `tests/corpus/cis-packs` has the CIS baseline and nine of its
extensions on; each pack carries a `notice`, and ten open notices printed ten blocks — on a
terminal 100 columns wide, 111 lines, of which about 90 are the same paragraph nine times
and nearly the same a tenth. What the run has to say is "nine packs ask for one command,
and so does the baseline".

The ten messages are not equal. Each opens with its pack's path and closes with a sentence
that names its param (``bind `cis_cmek_required_adopted = true` …``). Between those, nine
packs wrote the same text byte for byte; the baseline wrote its own, a few words apart.
The layout already printed one message under several first lines where the messages were
EQUAL and adjacent — a composition conflict is one finding per site — so the form of a
collapsed block existed. What did not exist was a way to know that two messages which are
not equal say the same thing.

## Decision

**A finding's producer states the sentence it shares, and the layout compares that for
equality.** `Finding` gains `shared: Option<String>`: what the finding says in the words
every finding like it uses, with what is its own left to the first line, which carries it
already as `file:line` and `subject`. For a notice that is the pack's `text` as the pack
wrote it, and the closing sentence in the plural — ``bind each param named above `true` …``
— in place of the pack's path in front and the param inside. `message` is what it was.

**Findings of one group are one block when their severity, kind, `fix` and what they say
are equal** — what they say being `shared` where the producer stated it, else `message`. A
block is the members' first lines as a table, in the columns of the run, then the sentence
once and `fix:` once. A block stands where its first member arrived, and a finding that
arrives between two members does not split it. A block of one prints its `message`, exactly
as before: there is no table of one row. The conflict's rows over one message are this rule
with no `shared` stated.

**Equal is the bar.** The baseline's text is a few words from the extensions', so the corpus
case prints one block of one and one table of nine. That is the correct result, and no
pack's prose is reworded to make it a table of ten.

**`shared` is not serialized and is in no schema.** `--format json`, `structuredContent`
and the published `outputSchema` are byte for byte what they were: every finding its own
object with its whole `message`. The language server is not collapsed either — a diagnostic
is one per location — and reads `message`.

**The collapse applies into a pipe as on a terminal.** ADR 0039's rule is that the structure
is the same both ways and only the wrapping differs; a block is structure. Every substring
the smoke matrix and `docs/llms.md` match on — a group's title with its count, a first
line, a `fix:` line, the closing count — is printed by a table as by a lone block.

**The counts are of findings.** The group's title, `(7 of 10, 3 silenced)`, and the last
line count rows, never blocks. A silenced finding is no row, and a table a silence leaves
one row of is a lone finding with its whole message.

## Options considered

1. **Compare messages fuzzily** — a similarity threshold, or equality after dropping
   backticked tokens. Needs nothing from a producer. Rejected: a compliance tool that
   decides two warnings are "about the same" will one day fold away the one that differed
   in the word that mattered, and nobody can say from the output which words were dropped.
2. **Cut the per-instance parts out of `message` by pattern** — strip a leading
   `` `path`: `` and the param in the last sentence. Rejected for the reason ADR 0039
   rejected it for the command: it holds until a producer words a sentence differently, and
   the renderer would hold knowledge of one producer's wording.
3. **Build `message` from typed segments** — shared and per-instance — and let the layout
   print the shared ones. One source for both texts. Rejected: the notice's closing
   sentence has its param in the middle, so the collapsed form needs a sentence of its own
   whatever the split ("each param named above"); segments would still need a plural
   wording per per-instance segment, and every producer would pay for a structure one of
   them uses. Two texts from the same parts, side by side in one `format!` pair with a
   test on both, is the smaller thing.
4. **Publish `shared` in the JSON.** An agent could group without comparing. Rejected: it
   is most of `message` a second time — the two-homes-for-one-fact ADR 0039 refused for
   the command — and a reader of the JSON groups by `kind` and `fix`, which it has. It can
   be published later without moving anything; unpublishing could not.
5. **Reword the baseline's notice to the extensions' text** so the table has ten rows.
   Rejected: the baseline's text says something the extensions' does not — Google sets some
   of ITS policies on every new organisation — and a layout is no reason to make a pack say
   less.
6. **Collapse on a terminal only.** A pipe would keep every full message. Rejected: the
   structure would differ by where the output goes, which ADR 0039 ruled out, and a CI log
   is where ninety repeated lines cost a reader most. What a pipe's reader needs whole is
   in `--format json`.
7. **A header row over the table.** Rejected: the columns are those of every first line in
   the run, which have none, and a header would be a line per table for four words a
   reader has seen on the block above.

## Consequences

- The corpus case prints 38 lines on a 100-column terminal where it printed 111, and 29
  into a pipe where it printed 61.
- The printed sentence of a table names no single pack or param. The row does — the pack by
  its `use` line, or by its own path where another pack uses it — and the JSON has the
  whole message. `docs/llms.md` says so for an agent reading a pipe.
- Two findings with equal messages and an equal command that were not adjacent are now one
  block; they were two. Nothing matched on their order.
- A producer joins by stating `shared`; none is forced to, and only `notices` does. A
  producer that states a `shared` text which leaves out something the first line does not
  carry hides it from the printed output of a table — the JSON still has it. That is the
  producer's contract, and its test asserts both texts.
- `Finding` has a field the JSON does not: a reader of the struct sees `shared`, a reader
  of the schema does not. The field's doc comment says why.
