# 0073 — a claim carries its measure without satz

- **Status:** accepted
- **Date:** 2026-09-30
- **Shipped in:** the release that follows

## Context

The security-review skill (`skills/security-review/`) writes a remediation plan per
customer: for every control a scan finds open, the measure that closes it — the satz pack
line, the same measure done with gcloud for an operator who does not run satz, and what
goes wrong while it stays open. [ADR 0003](0003-evidence-is-data-the-audit-pack-is-not-satz-s-to-render.md)
assigns the document to the agent and the facts to satz. For this part of the plan the
facts were not in satz. The skill carried its own copy in `assets/measures.yaml`: pack
names, `use` paths, gcloud commands and risk sentences, grouped by hand over the CIS 5.0
controls. That copy had already drifted — its `use` lines named `presets/cis-extensions/…`,
a directory the library no longer has — and nothing could catch it, because the packs
changed in one place and their gcloud equivalents lived in another.

What satz already had: a pack's `claim` names the framework, the control, the coverage
and the witnesses; `require` lists the packs that would provide an unmet control
(`providers`); `report-compliance` carries the included claims' `interpretation`. What it
did not have: the chain pack → control → measure as data, the gcloud route per measure,
and the risk.

Four choices were not obvious.

1. **Where the gcloud route and the risk live.**
2. **What shape they take in the pack.**
3. **Whether they are part of what a pack emits** — the canonical form `merge-presets`
   compares to decide whether an estate's included pack must fork.
4. **Which controls carry the measures in the report.**

## Considered options

For 1:

- **1a · On the claim.** Two new claim attributes, `gcloud` / `gcloud_check`, and `risk`,
  written by the pack author beside `interpretation`.
- **1b · In the catalogs** (`presets/catalogs/*.yaml`), per control.
- **1c · A separate measure file** in `presets/`, keyed by pack and control.
- **1d · Leave it in the skill.**

For 2:

- **2a · A list of commands**, one command per entry, a multi-line command (a heredoc
  writing a policy file) as one entry.
- **2b · One free-text string** per attribute.

For 3:

- **3a · Outside the canonical form**, as a sixth product (`canonical_measures`) beside
  the questions, offers and notices: reported by `check-presets` when a fork's texts differ
  from upstream, never forking an estate.
- **3b · Inside the canonical form**, like `interpretation` and the duties.

For 4:

- **4a · Every claim naming the control**, from the included packs and from the rest of the
  library, marked `included`; a cross-walked control (ISO 27001 reading CIS evidence)
  takes the measures of the controls it reads.
- **4b · The included claims only.**

## Decision

1a, 2a, 3a, 4a.

A `claim` takes three optional attributes: `gcloud = ["…", …]` — the commands that meet the
control the way the claim's resources do; `gcloud_check = ["…", …]` — the commands that
show whether it is met; `risk = "…"` — one sentence, what goes wrong without the measure.
Placeholders are uppercase words (`ORGANIZATION_ID`, `PROJECT_ID`, …), which Satz strings
do not interpolate. An empty list is refused; a claim with no gcloud route leaves the
attribute out.

`satz require --format json`, `satz report-compliance --format json`, `satz_require` and
`satz_report_compliance` carry a `measures` array on every control: per claim the pack, its
`use` path, the claim's framework, version, control and coverage, whether the estate
includes it, its resources, `interpretation`, `gcloud`, `gcloud_check` and `risk` — `null`
where the pack states none. No text rendering changes.

## Consequences

- The skill reads the gcloud route and the risk from the report it already takes, and its
  own catalogue keeps only what is authoring: the German prose, the phases, the grouping
  of controls into plan sections, and the gcloud routes of controls no pack claims.
- A pack change and its gcloud equivalent are reviewed in one diff, and a `use` path in the
  report is the library's own.
- The text is duplicated where a pack claims the same measure for CIS 4.0 and 5.0 — the
  claims are separate statements, and each carries its own copy.
- A command that is wrong is wrong in every report until the pack is fixed. The texts are
  checked against gcloud's own help and, read-only, against a test organisation; the
  commands that change an organisation are not run by the tests.
- Adding or rewording a route never forks an estate's included pack: `merge-presets`
  installs it like a comment change, and `check-presets` names it for a `.local` fork.
- A control no pack claims has no measure in satz. That is the library's gap, and a
  new pack closes it with its claim.

## Pros and cons of the options

### 1a · On the claim *(chosen)*

- **Good:** the claim already names the resources the route reproduces; the gcloud
  commands and the witnesses are written and reviewed together, and they version with the
  pack (`version`, changelog, `doc-packs`).
- **Good:** a `.local` fork that changes the resources can change the route with them.
- **Bad:** the 4.0 and 5.0 claims of one measure each carry the text.

### 1b · In the catalogs

- **Good:** one place per control, also for controls no pack claims.
- **Bad:** the gcloud route depends on HOW a pack meets the control — two packs meeting one
  control differently would share one text.
- **Bad:** the catalogs are derived data with their own refresh (`docs/housekeeping.md`);
  hand-written commands inside them break that.

### 1c · A separate measure file

- **Bad:** the drift the skill's copy showed, moved into satz — a third file to keep equal
  to the pack.

### 1d · Leave it in the skill

- **Bad:** the copy is already stale, and satz cannot tell.

### 2a · A list *(chosen)* / 2b · one string

- A list lets an agent render each command as a step and keeps a heredoc whole; one string
  would make every consumer split text on newlines it cannot tell from a heredoc's.

### 3a · Outside the canonical form *(chosen)*

- **Good:** a better command or a sharper risk reaches every estate on the next
  `merge-presets` without a fork, as a notice's wording does.
- **Bad:** the texts are not part of what `check-presets` calls a structural change, so an
  operator sees a reworded route only as "the claims' gcloud routes or risks differ".

### 3b · Inside the canonical form

- **Bad:** every route added to the library would auto-fork every estate that includes the
  pack, and report it as "resource lines differ" — which is untrue.

### 4a · Every claim naming the control *(chosen)*

- **Good:** an unmet control shows how to meet it, which is what the plan needs, and the
  `providers` list and the measures cannot disagree — both come from the library's claims.
- **Bad:** the report grows by the measure texts; a CIS 5.0 report over the whole library
  carries every route once per control.

### 4b · The included claims only

- **Bad:** the controls a plan is written for — the unmet ones — would carry nothing.
