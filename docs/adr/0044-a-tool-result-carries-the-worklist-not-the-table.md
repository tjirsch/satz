# 0044 — a tool result carries the worklist, not the table

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

An MCP client caps what it will read from a tool. Claude Code cuts a result longer than
`MAX_MCP_OUTPUT_TOKENS` — 25,000 tokens unless the variable is set — and what it hands
the model afterwards is no longer JSON; some clients write the whole result to a file
instead, which helps only a model that has a tool to open one.

Measured with `scripts/mcp-probe.py` against a live organisation and against the smoke
fixture:

| tool | bytes | ~tokens |
|---|---|---|
| `satz_adopt`, resolve only, live | 173,613 | 43,400 |
| `satz_report_compliance`, live, CIS GCP 5.0 | 76,481 | 19,100 |
| `satz_report_compliance`, the smoke fixture | 69,934 | 17,483 |
| `satz_merge_presets`, the next largest | 31,187 | 7,796 |
| every other tool | under 18,000 | under 4,400 |

Half of each number is the text block: MCP asks a tool that returns `structuredContent`
to repeat the same JSON as text, for a client that reads only text, so every row costs
twice its own JSON.

The two tools at the top are the two whose reports carry a row per resource.
`satz_adopt` returns one row per declared resource — what it resolved to, what it matched
on, the Satz line that declares it — and most of those rows say that there is nothing to
do: already managed, already adopted, or apply creates it. `satz_report_compliance`
returns a control per catalog entry with its witnesses, and every one of them is
evidence.

Against that stands the rule that a tool returns the value the corresponding
`--format json` command prints, so an agent and an operator read one thing. `satz adopt`
has no `--format`: it prints a table to a terminal, which has no limit.

## Considered options

1. **Leave the results whole and raise the client's limit.**
2. **Drop the rows that ask for nothing from `satz_adopt`, and keep the rest whole.**
3. **Drop the rows that ask for nothing AND cap the rest, with the counts, a note and
   an `out` argument that writes the whole report to a file.**
4. **Refuse a result over a size satz picks, naming `out` as the way through.**
5. **Leave the text block off a large result.**

## Decision

Option 3 for `satz_adopt`, and option 1 for `satz_report_compliance`.

**`satz_adopt`'s `rows` is the worklist.** Each row carries an `action` —
`unresolved` · `move` · `import` · `none` — derived from the outcome, not from the
verdict's wording. The result keeps the rows that are not `none`, in that order of
urgency, at most `ROWS_IN_RESULT` (50, around 9,000 tokens with the text block) of them.

**What it leaves out is a field of the report.** `rows_total` is the table's length,
`rows_omitted` is what is not in `rows`, and `note` states both in a sentence and names
the two ways to the rest: `out`, and `satz adopt <estate>` in a terminal. `written` and
`hints`, which `execute` fills with a line per resource, are capped the same way beside
`written_total` and `hints_total`. A result that quietly returned a subset would be worse
than one that is too big: an oversized result at least breaks visibly.

**`out` is a path under the server's root and needs `write`**, as `satz_scan_checkov`'s
does, and is judged before the organisation is read. The file holds the report with every
row and every line, and the result then names it.

**`satz_report_compliance` is unchanged.** An evidence report that omits evidence is a
different document, and it is the value `satz report-compliance --format json` writes. It
is the one result that grows without a bound satz sets, and where an estate outgrows the
client, the client's limit is what moves.

**The parity rule gains one stated exception**, in `docs/mcp.md` beside the tool table
and in the structured-output section: `satz_adopt`'s result is the worklist and says so.
The text block still equals `structuredContent` — the trimming is in the report, not in
one of its two renderings.

**`scripts/mcp-probe.py` fails the run on a result over `--limit-tokens`** (25,000 by
default), so the size is a check rather than a number in a table nobody reads.

## Consequences

- An agent calling `satz_adopt` on an estate of any size gets a result it can parse. The
  one it gets back is smaller than the table, which it is told in the same object.
- The full table over MCP costs a `write` grant. A read-only server can resolve an estate
  and see its worklist, and cannot have the table written for it; the operator runs
  `satz adopt` for that.
- `satz adopt` on the command line is untouched — same table, same counts, no `--format`.
- The MCP output schema of `satz_adopt` changes: five fields added, none removed. A
  client reading `rows` as the whole table now reads a worklist, which is the point.
- `satz_report_compliance` stays the one result that can pass a client's limit on a big
  enough estate. When it does, the decision to revisit is this one, with option 3's
  shape available: an `out` that writes the report and a result that carries the header
  and the counts. The cost of doing that now is `readOnlyHint: false` on every call of
  the tool an agent calls most.

## Pros and cons of the options

### Drop the rows that ask for nothing, cap the rest, name the file *(chosen)*

- **Good:** the result is bounded by a number in the code, whatever the estate holds.
- **Good:** what is missing and where it is are in the object, so an omission cannot be
  mistaken for an empty estate.
- **Bad:** two shapes of one report exist — the file's and the result's — and a reader
  has to know which one is in front of them. The `note` is there to say so.

### Leave the results whole and raise the client's limit

- **Good:** one shape, no exception to the parity rule.
- **Bad:** it is not satz's to raise. Every client that has not been configured cuts
  `satz_adopt` mid-JSON, and the tool is unusable on precisely the estates it is for.

### Drop the rows that ask for nothing and keep the rest

- **Bad:** it does not bound anything. A first adoption is an estate where *every* row is
  an import, which is the run that produced the 173,613-byte result.

### Refuse a result over a size satz picks

- **Good:** nothing is ever cut, and the refusal says what to do.
- **Bad:** satz would be guessing at the client's limit and refusing calls a client with
  a larger one handles fine. MCP negotiates no such number.

### Leave the text block off a large result

- **Good:** halves every result at a stroke.
- **Bad:** it is the copy MCP asks for, and a client that reads only text would get
  nothing at all from the largest tools.
