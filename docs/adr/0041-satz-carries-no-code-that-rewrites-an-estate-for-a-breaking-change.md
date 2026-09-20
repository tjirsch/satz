# 0041 — satz carries no code that rewrites an estate for a breaking change

- **Status:** accepted; supersedes the migration half of ADR 0028 and the decision of ADR 0033
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

Two breaking changes each shipped with code that converted an estate from the old form to
the new one, run by the pickup every estate makes (`get-presets`, `merge-presets`):

- ADR 0028 moved the CIS packs into `presets/cis/` and reshaped the baseline. The refusal
  of an old path came with `migrate_moved_packs`: it repointed `use` lines, moved forks and
  deltas, retired pristine copies, and lifted the baseline's line out of its
  `google_org_policy_policy { … }` block — that last part written against one file of one
  release.
- ADR 0033 made `merge-presets` write ` when <gate>` on every ungated line of a gated pack
  and bind the gates, with a second form of the transpile-identity proof that let the
  tfvars lines of the bound gates move.

Together that was about 650 lines of production code and 220 of tests, a smoke step, a
field in a published MCP output schema, and a widened proof. All of it serves the one
release that introduced each change. Once the estates that held the old form are past it,
the code converts nothing, and it stays: it is read, kept compiling, kept green and
reasoned about with every later change to `merge-presets` — the proof's exception for
bound gates sat in the path that guards every fork repoint. Each new breaking change would
have added its own converter beside these.

The rule that everyone is on the current version already says an upgrade brings work. The
question was who does that work: the compiler, by code that knows every past form, or the
operator, from a refusal that names the edit.

## Considered options

1. **Keep a converter per breaking change, and remove each once the estates are past it.**
2. **A breaking change ships as a refusal naming the fix, plus an entry in
   `presets/README.md` under `## Breaking changes`; satz rewrites no estate for it.**
3. **A separate `satz upgrade` command that holds the converters**, out of `merge-presets`.

## Decision

Option 2. A breaking change is the operator's to fix, not the compiler's. satz carries no
code that rewrites, repoints or migrates an estate so that an old form becomes the new
one.

**Deleted:** `repoint_moved_uses`, `unwrap_cis_baseline`, `migrate_moved_packs` with its
calls in `get-presets` and `merge-presets`, and `GetPresetsReport.migrated` — a field of
`satz_get_presets`' published output schema; the gating migration (`gate_ungated`,
`gating_contradiction`, `plan_gating`, `gating_notes` and their types) and the allowance
in the transpile-identity proof for bound gates, which is `before == after` again; the
retirement of the `.base/` snapshot directory and the three places that skipped it; every
test of those and the smoke step for the gating.

**Kept, and why each:**

- **The refusals.** `MOVED_PACKS` with `moved_pack`, and `RENAMED_PARAMS` with
  `renamed_param`, are errors naming the fix, which is this rule exactly. An entry costs a
  table row and leaves with its release line. `moved_pack`'s message says what to edit and
  points at `## Breaking changes`; `MovedPack` carries that sentence (`edit`) in place of
  the `reshaped` flag only the converter read.
- **The `ungated-pack` finding and `remove-pack`'s refusal** (ADR 0032). They are standing
  checks of the estate against the graph. Both now state the line to write and the gate to
  bind; the finding has no `fix`, since a `fix` is one runnable command or nothing
  (ADR 0039).
- **The standing update machinery.** Drift classification, the auto-fork and repoint of a
  pack whose upstream changed, the `.diff.satz` delta, the transpile-identity proof with
  its journal and rollback, `--adopt`, `check-presets`. This serves EVERY upstream change
  to a pack, in every release, and is the promise that a preset an estate includes never
  changes silently. The test for staying is that: code that serves any change stays, code
  that serves one past change goes.
- **`adopt_pack_lines` and `template::with_menu`.** They write the commented line for a
  pack the library gained after the estate was written. No breaking change is involved;
  without the line a new pack's question binds an answer nothing emits.
- **The YAML converter** (`satz import <file>.yaml`) is a separate decision and is not
  touched here.

## Consequences

- An estate holding an old form does not compile after the upgrade, and `merge-presets`
  stops on the same error before it writes anything. The operator, or their agent, makes
  the hand edit `## Breaking changes` describes. Every entry there has to be actionable
  with no context, because nothing else does the work.
- The hand edit is unproven by satz. The converter proved the gating by transpile
  identity; a hand edit is proven by whoever makes it, by comparing the generated HCL and
  reading `tofu plan`. For a fleet, the maintainer's private runbook drives an agent
  through the edit per estate and proves the result by comparing the emitted resources.
- A careless hand edit can do what the converter was written to prevent — a baseline line
  that comes back commented takes thirty organisation policies off at the next apply. The
  refusal and the entry both say so; nothing enforces it.
- Removing `migrated` from `GetPresetsReport` is a change to a published MCP output schema,
  and `get-presets` and `merge-presets` stop doing something estates relied on: the release
  is a MINOR under ADR 0010.
- `src/presets.rs` is about 560 lines shorter and the proof that guards a fork repoint has
  one form.

## Pros and cons of the options

### 1 — a converter per breaking change

- **Good:** a fleet pickup is one command per estate, and the converter's edit is proven.
- **Bad:** every converter is code for one release that outlives it; "remove it once the
  fleet is past it" needs someone to know when that is, for estates nobody here sees.
- **Bad:** converters accrete inside `merge-presets`, the command whose proof has to stay
  simple enough to trust.

### 2 — a refusal and a documented edit *(chosen)*

- **Good:** the cost of a breaking change is a message and a paragraph, so it is paid once
  and the code base carries no history.
- **Good:** the operator sees every change made to their estate, because they made it.
- **Bad:** the work moves to every operator, and a hand edit can be wrong in ways the
  converter could not be.

### 3 — a separate upgrade command

- **Good:** `merge-presets` stays clean.
- **Bad:** the converters still exist and still outlive their release; only their address
  changes.
