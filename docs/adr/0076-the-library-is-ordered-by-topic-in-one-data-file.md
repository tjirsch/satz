# 0076 — The library is ordered by topic, in one data file

- **Status:** accepted
- **Date:** 2026-09-30

## Context

The preset library had three orders and none of them was a reader's. The index of pack
pages (`presets/docs/README.md`, written by `satz doc-packs`) grouped packs by directory —
the root first, then `ci/`, `cis/`, `exemptions/` … alphabetically — so the estate map sat
between the billing packs and the budget, and continuous verification came before the
CIS baseline. The library page (`presets/README.md`) had its pack sections in the order
they were written. The map's `offers` entries (`presets/estate-map.satz`) list the packs in
ADOPTION order, with a `phase` text per entry: that is when to switch a pack on, a
different question from where to read about it, and it stays as it is.

A reader wants topics: the map and the core, security groups, billing, the CIS baseline
and its extensions, monitoring, networking, projects and interfaces, security operations,
continuous verification. The pack files cannot move to express that — every estate's
`use` line names the path — so the order has to be recorded beside them.

## Decision

**`presets/library-groups.txt` names the groups in reading order and every pristine pack
in exactly one of them, in the order an operator meets them.** `[<group>]` opens a group;
each line after it is a pack path under `presets/`.

- `satz doc-packs` reads it on the write path and under `--check`, so `cargo test` and the
  smoke matrix hold it: a pack in no group or in two, a line naming no pack, an empty group
  — all reported in one pass.
- The index prints one section per group, in the file's order.
- `presets/README.md` carries a `## <group>` heading per group, in the same order, with the
  pack sections under it; `doc-packs` fails on a missing or misplaced heading. The page
  is still written by hand — the gate holds only its skeleton to the file.
- `scripts/build-site.py` reads the same file for the side column of every pack page (the
  whole library under group headers, the page marked) and shows the library page's group
  headings as group headers in its contents column.

## Options

**A line in each pack's header comment** (`// library group: Networking`). The group
would travel with the pack, and a new pack could not forget it. *Rejected:* the header is
the page's Purpose text and the index's summary, so the line would have to be stripped
from both; it gives no order between groups or within one without a second field; and
changing a group would touch a pack file — a pack page goes stale, and a pack whose file
changes carries a changelog row. Regrouping 48 packs would have meant 48 pack edits.

**Group by directory, moving the files.** *Rejected:* every estate's `use` line names the
path, and a move is a breaking change for every estate, for an ordering question.

**The order in `build-site.py` only.** *Rejected:* the index and the library page on
GitHub would keep the old order, and nothing would fail on a new pack left out of the menu.

**YAML** (`library-groups.yaml`). *Rejected:* the content is two levels of names; a
plain list is what `managed-constraint-equivalents.txt` beside it already is, and
reading it needs no schema.

## Consequences

- A new pack adds one line to `library-groups.txt` in the same PR; `doc-packs` names it
  until it does.
- A group title is also a heading, so its anchor (`#cis-baseline-and-extensions`) is what
  links into the index and the library page use. Renaming a group moves that anchor, and
  the site build fails on every link to the old one.
- The index lost the directory anchors (`#cis`, `#scc`, …); the one link that used them
  was repointed.
