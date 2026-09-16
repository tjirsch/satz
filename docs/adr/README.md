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

| | what it decides | status |
|---|---|---|
| [0001](0001-mcp-identity-is-scoped-to-the-call.md) | the MCP server scopes the identity to each call, not the process | accepted |
| [0002](0002-superseded-org-policies-replace-by-construction.md) | superseded org policies change address so the plan is a replace | accepted |
| [0003](0003-evidence-is-data-the-audit-pack-is-not-satz-s-to-render.md) | evidence is data; the audit pack is the agent's to render | accepted |
| [0004](0004-the-verification-runner-is-a-pack-and-its-pipeline-is-inline.md) | the verification runner is a pack, and its pipeline is inline | accepted |
| [0005](0005-adopt-moves-a-renamed-block-rather-than-importing-it-again.md) | adopt moves a renamed block rather than importing it again | accepted |
| [0006](0006-an-answer-is-a-param-the-estate-binds.md) | an answer is a param the estate binds; nothing may be left unanswered | accepted |
| [0007](0007-the-map-is-a-pack-of-choices-and-the-estate-carries-the-lines.md) | the estate map is a pack of choices; the estate carries the `use` lines | accepted |
| [0008](0008-the-site-renders-markdown-with-githubs-parser.md) | the site renders markdown with GitHub's own parser (cmark-gfm) | accepted |
| [0009](0009-iac-service-account-named-roles.md) | the IaC service account holds named roles, derived from the resource types; `update-prerequisites` writes them | accepted |
| [0010](0010-the-minor-version-marks-an-upgrade-that-brings-work.md) | the minor version marks an upgrade that brings estate work; everything else is a patch | accepted |
| [0011](0011-plan-and-apply-replace-a-policy-switching-to-reset.md) | `satz plan` and `satz apply` replace an org policy the state holds with rules and the estate declares reset | accepted |
| [0012](0012-the-admin-port-policy-ships-on-and-passes-the-private-ranges.md) | the admin-port policy ships on and passes the private ranges | accepted |
| [0013](0013-a-claim-asserts-what-its-witness-does.md) | a claim asserts what its witness DOES; an org policy that is switched off discharges nothing | accepted |
| [0014](0014-a-dry-run-is-a-generated-twin-and-claims-nothing.md) | a dry-run policy is a GENERATED twin of its enforcing fragment and carries no claim | accepted |
| [0015](0015-an-exemption-is-a-tag-and-the-control-stays-on.md) | an exemption is a tag binding; the control stays enforced and the exemption is reported beside it | accepted |
| [0016](0016-editor-support-is-a-tree-sitter-grammar-in-its-own-repository.md) | editor support is a tree-sitter grammar in its own repository; the Zed extension lives here and pins one commit of it | accepted |
| [0017](0017-the-formatter-keeps-the-authors-line-breaks.md) | `satz fmt` works on the token stream and keeps the author's line breaks; alignment is per run, meaning is proven by the canonical form | accepted |
| [0018](0018-editor-intelligence-comes-from-the-satz-binary.md) | `satz lsp` serves editors from satz's own parser, pipeline and schema; parse per change, pipeline per save; a pack is compiled through the estates that use it | accepted |
| [0019](0019-an-import-skips-what-the-platform-owns-and-says-so.md) | a live import skips what the platform owns — built-in sinks, service agents' grants, legacy bucket grants, default service accounts, deleted projects — as `skip:` patterns on the import-config row, reported per pattern | accepted |
| [0020](0020-a-discovered-estate-binds-the-library-s-vocabulary.md) | a discovered estate binds the day-0 vocabulary: derived facts without comment, inferred values with their rule beside them, what nothing states left out and reported; every bound literal referenced the library's way | accepted |
| [0021](0021-one-format-one-file-one-artefact.md) | a reporting command takes `--format` and `--out`, both required, and writes exactly one artefact at exactly one named path; `xlsx` is a format, `--report` and `--xlsx` are gone, and `update-prerequisites` and `prowler` keep the console | accepted |
| [0022](0022-the-licence-is-apache-2-0.md) | satz is licensed under Apache 2.0 alone — the express patent grant and the contribution terms in §5, with a `NOTICE` file that travels with redistribution | accepted |
| [0023](0023-one-command-for-every-prerequisite.md) | one command for every prerequisite a resource type implies — `update-prerequisites` writes the missing roles AND APIs into the estate, `merge-presets` runs it at the end of a pickup, and the compiler orders every resource after the service that enables its API | accepted |
| [0024](0024-the-pack-rules-live-in-satz.md) | the rules a pack must clear live in satz as `review-pack`, not in the app that renders them; a pack is folded into a synthesised estate to see what it emits, and the review says what adopting it costs | accepted |
| [0025](0025-the-pdf-is-typeset-in-the-binary.md) | `--format pdf` is typeset by Typst compiled into satz, with its fonts, instead of shelling out to pandoc and a LaTeX engine — accepted at three times the binary size, because a deliverable that needs two installed tools is not one | accepted |
| [0026](0026-a-command-declares-its-formats-once.md) | a command declares its formats once, as the value parser of `--format`, so the help lists exactly what it writes; a command that writes markdown writes pdf; `--out` may leave the extension off; global options are listed by `satz --help` alone | accepted |
