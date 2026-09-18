# 0025 — the PDF is typeset in the binary, and the binary is three times the size

- **Status:** accepted
- **Date:** 2026-09-15
- **Shipped in:** v0.59.0

## Context

`--format pdf` is how an auditor is handed the evidence report and the org-policy
report. It worked by shelling out to `pandoc` — and pandoc alone is not enough: its
PDF output needs a PDF engine behind it, LaTeX by default. So the format most likely
to leave the building was the one that depended on two tools the operator had to
install, and on a machine with neither it simply failed. It failed on this one.

Until v0.57.0 it failed quietly: the markdown was kept and a note printed, which left
an artefact nobody asked for under a name that says it is something else. That is now
an error, which is honest and still unhelpful.

## Options

1. **Keep pandoc.** Nothing to build. The deliverable keeps depending on a Haskell
   program and a TeX distribution being present and configured.
2. **Drop `--format pdf`.** Markdown and the XLSX workbook are the deliverables;
   whoever needs a PDF makes one. Smallest, and it moves the last step of an audit
   engagement back out of the tool.
3. **Write the PDF directly** with a Rust PDF library. No typesetting engine, so
   pagination, wrapping and tables — which is most of a compliance report — become
   satz's problem. Weeks of layout code to get something worse than pandoc.
4. **Typeset it in the binary with Typst**, a typesetting engine written in Rust and
   usable as a library.

## Decision

Option 4, decided 2026-09-15. `--format pdf` is typeset by satz itself: markdown in,
PDF bytes out, no process, no PATH, nothing to install. The fonts travel with the
binary too (Libertinus Serif, New Computer Modern, DejaVu Sans Mono), because a report
that renders differently on the auditor's machine is not evidence of anything.

**What it costs, measured rather than guessed.** The stripped binary goes from 23 MB to
64 MB and the installer download from 6.7 MB to 20 MB — about three times. Roughly
10 MB of that is the fonts; the rest is the engine. Every `self-update` pulls the
larger artefact. That was weighed against the alternative and accepted: a compliance
tool whose deliverable is a PDF should be able to produce one on any machine, and a
one-time-per-release download on a professional workstation is the cheaper half of the
trade.

**satz writes no Typst of its own beyond a preamble.** `src/pdf.rs` converts the
markdown satz already produces into Typst markup — headings, emphasis, inline code,
links, lists, tables, fences, rules, quotes — and Typst does the typesetting. The
conversion is the part that can be wrong, so it is a pure function with tests; the
rest is a `World` implementation holding one source file and the fonts.

Three details that are decisions rather than mechanics:

- **Text is escaped.** Typst reads `#`, `*`, `_`, `@`, `<`, `[` as syntax, and an
  estate is full of them (`serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com`). Unescaped, a member string
  becomes a label reference and the document stops compiling.
- **A table of five columns or more turns the page.** Seven columns of prose on a
  portrait A4 wraps every cell to four lines and doubles the report; landscape halves
  it. It is the decision a person makes in a word processor.
- **The document has no date of its own.** `World::today` returns `None`, so the same
  report renders to the same bytes twice — a diff of two runs is a diff of the estate,
  not of the clock. When the run happened is in the report's text, where satz put it.

Unknown inline HTML is kept as escaped text rather than dropped: `<br>` and `<small>`
are ours and are formatting, but `<estate>` in an instruction is text somebody wrote,
and silently eating it is data loss in a document meant as evidence.

## Consequences

- `--format pdf` works everywhere, and CI can finally test it: the smoke matrix writes
  a real evidence PDF and asserts it is deterministic, on a runner with no pandoc.
- The binary and every release artefact are about three times larger.
- Typst, Libertinus Serif, New Computer Modern and DejaVu Sans Mono are named in
  `NOTICE`, which Apache-2.0 asks a redistributor to carry (ADR 0022).
- A Typst upgrade is now a satz dependency bump that can change how a report looks.
  The determinism test catches a change in the bytes; what it looks like is reviewed
  by opening one.
