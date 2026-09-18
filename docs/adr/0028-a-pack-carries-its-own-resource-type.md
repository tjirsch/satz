# 0028 — a pack carries its own resource type, and a moved pack repoints an estate's lines

- **Status:** accepted
- **Date:** 2026-09-17
- **Shipped in:** the release that follows

## Context

A pack has two possible shapes. Either it declares its own resource-type maps and an
estate `use`s it bare at the top level, or it is a bare list of labels and the estate
supplies the type (`google_x { use … }`, or `use … as google_x` written flat). Most of
the library is the first shape. The CIS baseline was the second, alone among the packs
an estate is expected to adopt, and the consequences reached further than its own file:

- `satz` wrote the nested line itself, as fixed template text, because the baseline was
  the one library pack absent from `PACK_LINES`. Nothing else could write it: `interview`
  activates a pack by uncommenting a line with a ` when <gate>` suffix, and the baseline's
  line had no gate; `merge-presets` writes lines from the table; `unadopted_pack_findings`
  reads the same table. satz-studio builds a pack row per gated `use` line, so the
  baseline was the one pack the app never listed.
- Two commands already wrote the bare form the pack could not accept:
  `report-compliance --fix` proposed a `use` line that did not compile, and `review-pack`
  could not assemble a review estate for it at all.
- The baseline lived at the top of `presets/` while the seventeen extensions it gates
  lived in `presets/cis-extensions/`.

The two shapes are also not interchangeable, and the wrong pairing fails silently: as a
map's content every name-less child is read as `(label, body)`, so a self-typed pack has
its type key read as a label and collapses into one resource carrying its contents as
attributes, which the emitter accepts without a word.

## Options

- **(a) Leave the shape, teach satz-studio to list an un-gated line.** One app change;
  leaves the baseline unadoptable by `interview` and `merge-presets`, keeps the two broken
  writers broken, and breaks studio's own invariant that a pack row is a gated line.
- **(b) The pack declares its own type and joins the map as a choice.** One shape for
  every pack an estate adopts, no app change, and the two broken writers become correct.
  It costs an edit in every live estate and a migration for the fleet.
- **(c) (b), and move all eighteen CIS files into `presets/cis/` in the same release.**
  One more path change on top of an edit that was happening anyway.

## Decision

(c), with the decisions of 2026-09-17:

- The baseline wraps its labels in `google_org_policy_policy { … }` and is `use`d bare.
  Nothing emitted changes: both forms resolve to the same addresses and the same manifest,
  which the corpus snapshots prove by reproducing untouched.
- `use_cis_baseline` joins `presets/estate-map.satz` with `recommend = true`. Making it a
  question does not make it optional — the question says the estate exists for these
  thirty policies — but it does make the baseline adoptable, reportable and listable by
  the same machinery as every other pack.
- A self-typed pack used as a resource map's content is REFUSED, in all three forms that
  make it one, naming the pack's own typed map and the line to write instead.
- The CIS files live in `presets/cis/`, the baseline beside the extensions it gates. `cis` is
  the umbrella for every CIS benchmark, not one of them: a Google Workspace Foundation pack
  and a GKE or Kubernetes pack go in beside the GCP Foundation one, and each pack's file name
  carries what it covers (`CIS-GCP-Foundation-4.0.satz`). `presets/cis-foundation/` was
  considered and rejected for that reason — the GCP and Workspace benchmarks are both
  *Foundation* benchmarks, so it would have read correctly for those two while leaving the
  CIS benchmarks that are not Foundation ones needing a sibling folder. `presets/cis-gcp/`
  was rejected the same way: it names one platform. The cost accepted is that `presets/cis/`
  is one character from `presets/ci/`, the verification-runner packs.
- The move is migrated by satz, not by hand: `MOVED_PACKS` in `crates/satz-core/src/pipeline.rs`
  is an old→new table that the compile refuses against by name, and that `merge-presets`
  repoints estate lines from — one table, so the refusal and the migration cannot disagree.
  A directory entry covers the `.local.satz` forks and `.diff.satz` deltas beside a pack.
  Entries are removed once the fleet is past them, as `RENAMED_PARAMS` entries are.

## Consequences

- A `use` of a moved path is refused rather than followed. It has to be: the old file is
  still on disk in every estate that fetched it, so following it would compile a copy
  frozen at the version it had the day the library moved, with nothing to show for it.
- `merge-presets` repoints the lines, carries a fork over and retires the stale pristine
  copies — and it must run BEFORE its own byte-identity transpile, because an
  un-migrated estate no longer compiles.
- The nested `use` form stays in the language and stays documented, with the packs that
  still have that shape: the contacts pack and `showcase-policies.satz`. What changed is
  that the pack's shape decides the form, and each pack states its line in its header.
- One release carries both the reshape and the move on purpose. Reshaping in place would
  have changed the pack's canonical form at its existing path, which the preset
  provenance rule treats as a semantic upstream change: every estate would have been
  auto-forked onto `CIS-GCP-Foundation-4.0.local.satz`. Arriving at a new path instead,
  the reshaped pack is simply a new pristine file.
