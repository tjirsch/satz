# 0077 — A pack's tier is its default, its id is its file name, and the library page holds no history

- **Status:** accepted
- **Date:** 2026-10-01

## Context

The preset library — 48 packs under `presets/`, the library page `presets/README.md`
and one generated page per pack under `presets/docs/` — had an order (ADR 0076) but no
written design. Four things were decided by whoever last touched a pack:

- **Which packs an estate gets.** Nothing said what every estate has, what is opt-in and
  what is a vendor integration. The estate map (`presets/estate-map.satz`) already
  decides it, in its params: each pack's gate defaults to `true`, `false`, or another
  gate, under comments that read "recommended: on unless the customer says otherwise" and
  "optional: off until the customer asks".
- **What a pack is called.** `pack <id>` followed four spellings: a folder prefix
  (`cis_extensions.cmek`, `monitoring.organization_audit_logsink`, `ci.verification_runner`,
  `integrations.microsoft_sentinel`, `exemptions.exemption_tag`), no prefix (`scc_export`,
  `s1_security_groups`, `project_cis_log_alerts`), and a framework version
  (`CIS_GCP_Foundation_4_0`). Eighteen of 48 ids equal their file name; 30 do not. Nothing
  in satz reads the prefix: the parser takes any identifier, and the id's readers are the
  version line, the changelog key, the report text and the canonical form.
- **What the page is for.** `presets/README.md` carried the conventions, prose per pack
  that the generated pages also derive, and 1,200 lines of history — `## Breaking changes`
  per release and the `## Changelog` table — 38 % of the page every reader scrolled past.
  The MCP resource `satz://presets` served the page trimmed at the changelog for that
  reason.
- **How the page is navigated.** The side column listed every group's packs flat: 48
  entries on a pack page, more on the library page.

The contents of the library — which packs it lacks — are planned in their own roadmap
note and are not this record's subject; that note needed the rules above to name a folder,
a default and an id for each pack it proposes.

## Decision

**A pack's tier is derived from the map, its id is its file name, every pack clears the
same gates, the library page holds the conventions and the generated pages hold the
facts, and the history has a page of its own.**

- **Tiers are derived, never declared.** `estate-core` has no `offers` entry — every
  estate starts with it; `estate-map` is the map. Every other pack has exactly one gate
  (pack-graph check 2), and the gate's default in the map is the tier: `true` is on unless
  the customer declines, `false` is opt-in, a reference to another gate follows that
  choice. A vendor integration is an opt-in pack under `presets/integrations/`, and
  pack-graph check 10 refuses one whose gate defaults to `true`. No `tier` field exists
  in `offers` or in `library-groups.txt`.
- **The id is the file name.** `pack <id>` is the file stem with `-` written `_`, no
  folder prefix, unique in the library: `cis/cmek.satz` is `pack cmek`. `satz doc-packs`
  and `satz review-pack` refuse any other id. The 30 ids that predate the rule stand in
  `LEGACY_PACK_IDS` (`src/doc_packs.rs`), a table that refuses a row whose pack now
  conforms or is gone, so it empties as the renames land and cannot grow.
- **Folders.** A pack lives in the folder of its family; a new family is a new kebab-case
  folder plus a line in `presets/library-groups.txt`. `presets/cis/` holds CIS packs only
  (ADR 0028); organisation-policy packs from other guidance go in `presets/hardening/`.
- **What every pack must carry** is one list, and each item is a gate: it parses, is
  formatted, opens with a sentence the index prints and states its own `use` line, declares
  `pack <id> version` with a changelog row, carries no private shape, compiles in an
  estate, declares no membership, runs no legacy constraint beside its managed twin, has a
  prerequisite row per emitted type, a `tests/iac/` line, an `offers` entry and one gate,
  notices only on a gated pack, claims with `risk` and `gcloud_check`, a current dry-run
  twin and a current page, and a line in the groups file. Two gates join with this record:
  the id rule, and **a string param that defaults to `""` — a value the operator must
  supply — has a `question` in the same file.**
- **The library page and the pack pages.** `presets/README.md` holds the conventions,
  one introduction per group and the artefacts that are not packs. A generated page holds
  everything derivable from the pack plus its notes region. The history — `## Breaking
  changes` and `## Changelog` — is `presets/CHANGELOG.md`, the site's `changelog` page
  after `library`; `satz get-presets` ships it beside the packs, so the refusal of a moved
  pack still points at a file the operator has.
- **The side menu folds by group.** Each group of the side column is a `<details>` whose
  packs are shown only when it is open; the group the reader is in is open, the others
  are closed.

## Options

**A declared tier** — a `tier = …` field in each `offers` entry, or a tier marker in
`library-groups.txt`. *Rejected:* it restates what the map's default already says, and
the two would drift; ADR 0031 rejected a second home for the map's facts on the same
ground.

**Folder prefix plus file name** (`cis.cmek`, `monitoring.organization_audit_logsink`).
*Rejected:* the folder already says where a pack lives and the groups file says its
topic, so the prefix is a third statement of grouping; and `review-pack` sees a pack
outside the library by its file name only, so the rule could not be checked where ADR 0024
needs it most.

**No naming rule.** *Rejected:* the index, the changelog and every compliance report
would keep printing four spellings side by side, and a new pack would pick a fifth.

**Rename the 30 ids now.** *Rejected for this release:* `canonical_parts`
(`crates/satz-core/src/satz.rs`) writes `pack <id>` into the body the drift comparison
reads, so an id change forks the pack in every estate that uses it. The emitter writes no
id, so the header does not belong in "what this file emits"; taking it out, then renaming,
and moving the baseline's path in the same minor release is a roadmap item.

**The history stays on the library page**, with the menu folding it away. *Rejected:*
the page would stay 38 % history on GitHub, where the menu does not exist, and the rule
that docs say what satz does now would keep its one exception on the page a customer
reads first.

**Fold the menu in the browser only**, leaving the HTML flat. *Rejected:* the HTML is what
a reader without the script gets, and `<details>` folds without one.

## Consequences

- A new pack is named by its file; `doc-packs` and `review-pack` say so when it is not,
  and a `""` default without a question is refused the same way.
- `LEGACY_PACK_IDS` holds 30 rows. The rename release takes the header out of the
  canonical form, renames the ids as silent upgrades (a version bump each, the old
  changelog rows rewritten to the new id), moves
  `presets/cis/CIS-GCP-Foundation-4.0.satz` to `presets/cis/cis.satz` through
  `MOVED_PACKS`, and empties the table — one minor release, one `## Breaking changes`
  entry, one `use` line per estate on the baseline.
- `presets/CHANGELOG.md` is where a version bump adds its row and a refusing release its
  entry; `doc-packs` reads it there. `satz://presets` serves the whole library page, untrimmed.
- The prose still written per pack on the library page migrates into the pack pages'
  notes regions one group at a time; until then both exist.
- The claim-measure rule (`risk` and `gcloud_check`) is held by `cargo test` only; it
  joins `review-pack` when that command next changes.
