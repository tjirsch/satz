# 0032 — one pack logic reads the estate against the shipped graph

- **Status:** accepted
- **Date:** 2026-09-19
- **Shipped in:** the release that follows

## Context

ADR-0031 made `presets/pack-graph.json` the one source of which packs exist and what
they need. Four pieces of code still answered "which packs does this estate use, and is
that consistent" each on their own: the interview uncommented a line by matching the text
` when <gate>` at its end, `merge-presets` skipped any path written anywhere in the file
and appended the rest at the end, the compile matched lines by path, and satz-studio
derived its own tree. They disagreed — an old estate whose runner grant is gated on the
runner's gate had its grant uncommented by the runner's yes — and none of them could say
whether a pack's requirements were met, because none of them read the edges.

Three commands were decided on 2026-09-18: `satz packs` reports, `satz add-pack` and
`satz remove-pack` change answer and line together, and `remove-pack` leaves the line.
What that decision left open is how the logic reads an estate.

## Decisions

**One module, `src/packs.rs`, and every reader goes through it.** The report, both
switches, the interview's yes, `merge-presets`' missing lines and the compile's pack
findings call it, so the sentence `add-pack` refuses with is the sentence
`transpile --check` prints.

**A gate's value is read from the text, not from the compile.** The estate's own binding,
else the default in the library file that declares the gate while that file's line is
active, following a reference (`use_sentinel_auditlogs = use_sentinel`). Options:

- *the compile's resolved parameters* — authoritative, but a compile that stops on
  `unknown param` has none, and that is the estate `packs` has to explain;
- *read from the text* (chosen) — the same answer for every boolean gate, available on an
  estate that does not compile; it cannot follow a gate bound to a string or to a param no
  library file declares, and reports that gate's value as unknown.

**What a pack needs is read from the edge kinds, and only `requires`, `gate` and `data`
are requirements.** `requires` edges of one pack are one requirement, any of them meets it
(ADR-0031). A `gate` edge is one, and so is the map when the map declares the gate — the
line's `when` does not compile without it. A `data` requirement is met by its provider, or
by the estate binding the params the pack reads, or every param of the pack whose default
reads them: the runner grant with `ci_runner_service_account` bound to a runner another
estate runs compiles and is right, so a hard `data` rule would refuse the MSP-hosted shape.
An `asks` edge only hides a question.

**A line goes where the graph's order puts it.** After the line of the pack before it in
the same place (the menu, after the scaffold, or the same block), else before the one after
it, else where that place starts: inside its block, after the estate-core line — or, in an
estate without one, after the top-level `params` — or at the end for a pack placed after
the scaffold. Appending at the end, which `merge-presets` did, put the map line below the
folder whose lines it gates in an imported estate, and uncommenting one gave
`unknown param`.

**The interview's yes switches the line on as `add-pack` does**: uncommented and gated on
the pack's own gate, or written where the graph places it when the estate has no line. The
line is found by its path (or its `.local` fork), never by the text after `when`.

**`add-pack` refuses while a pack it excludes is on**, the other option of a choice
included, and `--with-requirements` switches on only a requirement the graph names one pack
for; where several can meet it — the billing grants need a security model — it refuses and
names them, because that is a customer's choice.

**`remove-pack` refuses a pack whose line is not gated on its gate.** Binding the gate
false leaves an ungated line deploying; the refusal names the line and the `when` to write,
which is the gating migration's edit and not this command's.

## Consequences

- The compile gains three findings at the validation level — an active line of a gated
  pack without its `when` (only where the gate is declared), a pack that deploys while
  something it needs is off, and two packs on two gates that exclude one another both
  deploying (an error). Two packs on one gate that exclude one another — the S1 model's two
  spellings — are no error: they fold as one where they agree, and an estate on the test
  organisation carries both. An estate that adopted its packs as plain `use` lines and
  uses the map now shows the first on each of them until the gating migration runs.
- The interview without a `pack-graph.json` binds answers and switches no line, and says
  so once; before, it matched the `when` text.
- The report is the rows satz-studio's Packs view reads; the app's own derivation can go.
- A notice (the next P10 item) has no statement yet, so neither the report nor `add-pack`
  carries notices.
