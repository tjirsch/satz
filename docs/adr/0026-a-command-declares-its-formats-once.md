# 0026 — a command declares its formats once, markdown brings pdf, and `--out` may leave the extension off

- **Status:** accepted
- **Date:** 2026-09-16
- **Shipped in:** v0.60.0

## Context

ADR 0021 gave every reporting command one vocabulary, `--format` and `--out`, and each
command narrowed the shared `OutFormat` at run time with `require_one_of`. The help was
generated from the enum, so every command's `--help` listed all five formats: `prowler`
advertised `pdf` and `xlsx` while it wrote two, and `report-compliance` said "markdown,
json or pdf" in its own sentence with all five listed two lines below. The refusal was
correct; the help an operator reads first was not, and ten commands each carried a
different real set behind one advertised set.

Three commands wrote markdown and no pdf — `questions` (the decisions sheet),
`triage` and `diff-organizational-policies` — although pdf is only the markdown typeset
(ADR 0025) and the form in which a document is handed to someone.

`--out` had to name the file exactly, so `--format pdf --out evidence/cis` wrote a file
called `cis` with no extension, and `--format pdf --out cis.md` wrote a PDF named `.md`.

The help had a second fault of the same kind: clap copies every global option into
every command, so each command's help repeated the eight options that belong to `satz`
itself, under the heading `Global options`.

## Options

**Where a command's formats are declared.**

- *Keep `require_one_of` and generate the help sentence from the same constant.* The
  "Possible values" block clap prints from the enum would still list all five.
- *A value parser per command* (`out::formats(&[…])` as the `--format` value parser).
  One declaration drives both the help and the refusal; the refusal becomes clap's
  (`invalid value 'pdf' … [possible values: text, json]`) instead of satz's own sentence.

**What `--out` without the format's extension does.**

- *Write the name as given.* What satz did: the file carries no extension, and a
  contradicting one goes through.
- *Refuse a name without the extension.* Strict, and exactly the friction the change is
  meant to remove.
- *Add the format's extension; refuse a name that ends in another format's extension;
  write any path that exists and is not a regular file as given.* A name with a dot that
  is not a format's extension (`acme.2026-09-16`) gets the extension added. `/dev/stdout`
  and a pipe keep working. `--format pdf --out cis.md` is refused, because one of the two
  is a mistake and satz cannot tell which.

**How the global options leave a command's help.** Hiding them after clap has built the
command breaks parsing — a built command's name lookup is computed once, and re-adding
an option moves it, so `satz prowler --help` became a missing-argument error. Giving
each command a hidden copy BEFORE the build works: clap copies a global option only into
a command that has none of that id, the copy parses exactly as the original, and values
still reach the root.

## Decision

- Each command declares its formats once, as the value parser of its `--format`. The
  help lists exactly those; clap refuses the rest naming them. `require_one_of` is gone.
- A command that writes markdown writes pdf. A test walks every command's `--format` and
  fails on markdown without pdf.
- `--out` may leave the extension off: the format's extension is added, a contradicting
  format extension is refused, and a stream is written as named. The `wrote …` line on
  stderr names the path written.
- Global options are listed once, by `satz --help`, and hidden from every command's own
  help while still parsing after any command.

## Consequences

- A caller that named `--out report` and read `report` back now finds `report.json` —
  the same invocation writes a different path, which makes this a minor release
  (ADR 0010). satz-studio names its reports with the format's extension and is unaffected.
- A caller that paired a format with another format's extension is refused where it used
  to succeed.
- The refusal text of a wrong format is clap's, not satz's: `--format pdf is not
  available here; use text or json` became `invalid value 'pdf' for '--format <FORMAT>'
  … [possible values: text, json]`. Both name the format and the alternatives.
- The global options no longer appear in `satz <command> --help`; `satz --help` and the
  README list them.
