# 0021 — a reporting command takes one format and writes one file

- **Status:** accepted
- **Date:** 2026-09-15
- **Shipped in:** v0.57.0

## Context

`OutFormat` unified what a caller may ASK for: one enum across every reporting command,
so `--format jsom` is refused by name instead of silently rendering markdown. Where the
bytes GO was never unified, and five conventions had grown across eight commands:

| command | format | destination |
|---|---|---|
| `questions` | `--format text\|markdown\|json` | stdout — **plus** a separate `--xlsx <FILE>` |
| `require`, `check-presets` | `--format text\|json` | stdout only |
| `triage` | `--format markdown\|json` | `--report <FILE>` "instead of stdout" |
| `report-compliance`, `report-organizational-policies` | `--format markdown\|json\|pdf` | `--report <FILE>`, with a default path |
| `remediation-plan`, `doc-packs` | — | `--out <DIR>`, several files |

A caller had to know, per command, whether output arrived on stdout, at `--report`, at
`--xlsx` or as three files inside `--out`. `xlsx` was the worst of it: a format wearing
a flag, so it could not be chosen the way every other format is, and it said "also
write" rather than replacing the rendering. `satz questions <estate> --xlsx out.xlsx`
therefore wrote the workbook AND printed the whole text rendering to the console —
two renderings of one catalog from one invocation, one of which nobody asked for.

Beside it sat two smaller versions of the same thing: `diff-organizational-policies`
echoed a console summary in addition to the report it had just written, and
`triage --fix` printed the estate delta to stdout even when the table went to a file.

## Options

1. **Leave it.** Each convention suits its command, and nothing is broken enough to
   move. The cost is per-command memory in the customer-facing surface, and the
   double rendering stays.
2. **Make every destination a flag of its own** — `--xlsx`, `--pdf`, `--report`. The
   flags multiply with the formats, two of them can be passed at once, and "which
   one wins" becomes a rule to remember per command.
3. **One format argument, one destination argument, both required, nothing on the
   console.** Every rendering is a `--format` value, including `xlsx`; every command
   writes exactly one artefact at exactly one path the caller named. Piping is
   explicit: `--format json --out /dev/stdout | jq`.

## Decision

Option 3. A reporting command takes `--format` and `--out`, both required, and writes
one file. The line that says where it went is on stderr, so `--out /dev/stdout` is a
clean pipe. `--report` and `--xlsx` are gone rather than kept as aliases: everyone is
on the current version, and a release IS the migration.

Two commands are not reporting commands and keep the console: `iac-roles`, whose exit
code is the answer and whose text is the diagnosis, and `prowler`, which prints a
command line to paste. `remediation-plan` and `doc-packs` write several files each, so
they take `--out-dir <DIR>` — a directory is a genuinely different shape, and the two
names say which one a command has.

Three consequences follow from "exactly one artefact":

- **No format default.** A default format plus a named file writes the wrong rendering
  into the right path — `--out decisions.xlsx` would have produced text.
- **`--format pdf` fails rather than leaving markdown behind.** It used to keep the
  markdown and print a note, which left an artefact the caller did not ask for under a
  name that says it is something else. (It went through `pandoc` on stdin then; since
  ADR 0025 satz typesets the PDF itself and needs no tool at all.)
- **`triage --fix` writes the delta INTO the report**, and is markdown-only. It is
  prose; a JSON caller reads the rows.

The append-only evidence history `report-compliance` keeps is not a second artefact of
the run: it is the audit trail of a deliberate report, written beside the report and
named in the line that reports it — and still skipped for `--format json`, which is a
caller reading state rather than filing a report.

## Consequences

- A MINOR release: every scripted call of the eight commands needs the two arguments,
  and `--report` / `--xlsx` fail by name.
- The vocabulary is one sentence — a format, a file, one artefact — instead of a table.
- A pipeline says out loud that it is piping, which is the point.
- `iac-roles` and `prowler` are the two exceptions, each with a reason in its own help
  text. Two exceptions with reasons beat five conventions without.
