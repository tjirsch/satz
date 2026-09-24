# 0054 — the S1 security group model ships in one spelling

- **Status:** accepted
- **Date:** 2026-09-22
- **Shipped in:** the release that follows

## Context

The S1 security group model — the five admin groups and their organization-level role
grants — was in the library twice:

- `presets/security-group-models/s1-security-groups.satz` (`pack s1_security_groups`), a
  typed pack with its own `google_cloud_identity_group { … }` and
  `google_organization_iam_member { … }` sections, used at the top level of the estate;
- `presets/security-group-models/s1-group-definitions.satz` (`pack
  s1_group_definitions`) and `presets/security-group-models/s1-group-permissions.satz`
  (`pack s1_group_permissions`), two bare lists `use`d as content INSIDE the resource
  type each one fills.

The two spellings emitted byte-identical HCL from identical params and asked the same
five questions. Which one an estate held was a fact about when it was written, and the
choice carried no consequence anyone could act on — but every reader of the library, of
an estate and of the pack graph had to work out that the three files were two ways to
say one thing, and which of them this estate meant.

Carrying both cost more than the library file it saved. The map needed two `by_hand`
`offers` entries and two `excludes` edges to keep the spellings apart; the billing
grants' `requires` named three files for two models; `satz packs`, `merge-presets` and
`add-pack` each carried a special case so a pack "stood in for" an `excludes` neighbour
on its own gate; and the compile had to treat a pair of mutually excluding packs on ONE
gate as no contradiction, while the same pair on two gates was an error.

## Decision

**The library ships one S1 model, in one file, used at the top level:**

```satz
use "presets/security-group-models/s1-security-groups.satz" when security_model_s1
```

`s1-group-definitions.satz` and `s1-group-permissions.satz` are deleted, with their
`offers` entries, their pages under `presets/docs/`, their changelog rows and the
`tests/iac/s1-split` case. Neither `get-presets` nor `merge-presets` deletes a file the
library dropped, so an estate whose `presets_dir` still holds the two files keeps
compiling them, and `satz check-presets` lists them as `local-only [included]`; an
estate whose `presets_dir` lacks them is refused by name and line — `use
"…/s1-group-definitions.satz": file not found`. `presets/README.md`'s
`## Breaking changes` carries the edit: delete the two nested lines, write the one
top-level line, keep every param as it is, delete the two files from `presets_dir`. No
alias, no shim, no migration: the plan does not move, so the edit is two lines, two
deleted files and a re-transpile.

With the only instance gone, the same-gate alternative-spelling handling goes with it.
Two packs that exclude one another and both deploy are now always the
`ExcludedPacks` error, whatever gates they carry, and neither `merge-presets` nor
`gate_on` has a pack that another pack stands in for.

## Options

**Keep both spellings.** *Rejected.* Nothing was gained by the second one — the library
already proves that a pack can be `use`d under a resource type (the essential-contacts
pack is offered with `block = google_essential_contacts_contact`), so the capability is
exercised without a duplicate of a model. What was lost is the time of everyone who had
to establish that the two were the same, repeatedly.

**Keep the two-file spelling and delete the typed pack.** *Rejected.* The typed pack is
one line in the estate rather than two nested inside resource-type blocks, it is what
`satz init`, `satz interview` and `satz add-pack` write, and it needs no `by_hand` entry
in the map.

**Delete the pair but keep the same-gate exclusion handling for a future pair.**
*Rejected.* Code no data reaches is code no test proves, and the special case is what
made the pair possible to keep. A future pair of spellings on one gate would have to
argue for itself, and would then bring back the handling with a case that exercises it.

## Consequences

- An estate on the two-file spelling keeps compiling while its `presets_dir` holds its
  own copies of the two files, which no library update refreshes or removes; it does not
  compile where they are missing. Either way it is edited, and the two files are deleted
  by hand. The edit is mechanical and the emitted HCL is unchanged, so the estate's
  `tofu plan` after it reports no change.
- `estate_map` 2.2 offers two fewer packs and its `requires` on
  `billing-account-permissions.satz` names the two models.
- ADR 0033's "the two spellings of one security model on one gate are no contradiction"
  no longer describes any pack in the library, and the code that implemented it is gone.
  ADR 0031's `excludes` example is now the dry-run twins alone; `excludes` itself is
  unchanged, and a declared edge between two packs on one gate is still what
  `satz pack-graph` requires before they may share a gate.
