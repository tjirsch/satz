# Architecture decision records

One file per decision that was **not obvious**, in [MADR](https://adr.github.io/madr/)
form: the context, the options weighed, what was chosen, and what it costs.

A decision belongs here when reversing it would be expensive, when a reader of the
code would otherwise ask "why on earth" — or when it was a genuine choice between
defensible alternatives. Most changes are none of those and need no record.

## Why they live here rather than in a commit message

A commit message explains one change to whoever reads that commit. An ADR answers
"why is it like this?" months later, when nobody is looking at the commit and the
alternative has started to look attractive again. Several decisions in this repository
were re-proposed after the reasoning was lost; that is the failure this directory
exists to prevent.

## Conventions

- `NNNN-kebab-title.md`, numbered in order, never renumbered. Take the next free
  number from the directory, not from memory: two records were both written as 0005
  on 2026-09-09, and a collision is fixed the day it is made, before anything links
  to the record.
- **Status** is `proposed`, `accepted`, `superseded by ADR-NNNN`, or `rejected`.
  A superseded record is not deleted — the reasoning that was right at the time is
  the thing worth keeping, and the successor should say what changed.
- Options are written with their real trade-offs, including the one that was chosen.
  A record where the alternatives are strawmen documents nothing.
- Nothing here names a customer, an organisation or a person: this repository is
  public, and the privacy gate treats these files like any other.

These pages are **not** published to <https://tjirsch.github.io/satz/>. The site's
navigation is a decided list of pages for people using satz; these are records for
people changing it. `scripts/build-site.py` only reads `docs/*.md`, so a record here
needs no entry in `SITE_DOCS` and implies no navigation decision.

## Records

| | decision | status |
|---|---|---|
| [0001](0001-mcp-identity-is-scoped-to-the-call.md) | the MCP server scopes the identity to each call, not the process | accepted |
| [0002](0002-superseded-org-policies-replace-by-construction.md) | superseded org policies change address so the plan is a replace | accepted |
| [0003](0003-evidence-is-data-the-audit-pack-is-not-satz-s-to-render.md) | evidence is data; the audit pack is the agent's to render | accepted |
| [0004](0004-the-verification-runner-is-a-pack-and-its-pipeline-is-inline.md) | the verification runner is a pack, and its pipeline is inline | accepted |
| [0005](0005-adopt-moves-a-renamed-block-rather-than-importing-it-again.md) | adopt moves a renamed block rather than importing it again | accepted |
| [0006](0006-an-answer-is-a-param-the-estate-binds.md) | an answer is a param the estate binds; nothing may be left unanswered | accepted |
| [0007](0007-the-map-is-a-pack-of-choices-and-the-estate-carries-the-lines.md) | the estate map is a pack of choices; the estate carries the `use` lines | accepted |
| [0008](0008-the-site-renders-markdown-with-githubs-parser.md) | the site renders markdown with GitHub's own parser (cmark-gfm) | accepted |
