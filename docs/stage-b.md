# IR core stage B — plan of record

Phase 2, second swap: fragments split per source file, emission from `Folded`,
scope-as-typing replaces walk interception, MERGE_KEY_PREFIX dies with YAML
parsing. Everything lands behind the differential harness; the old pipeline
stays the reference until the last increment flips the default.

## Architecture decision (2026-08-21)

**The stage-B front-end is Satz → Fragments, not YAML → Fragments.** The YAML
dialect composes textually: packs reference anchors their includer defines, so a
pack file cannot even parse standalone — per-file fragments are impossible in
YAML-land without reimplementing anchor scoping. Satz already has real parameter
scoping (pack defaults, estate wins), so the front-end compiles the satz AST
(`satz_core::satz::File`) straight to `Vec<Fragment>` — one per source file —
with resolved params and structural scope. The YAML dialect stays supported on
the old pipeline until it retires (M3); MERGE_KEY_PREFIX and the rename passes
die together with that pipeline's composition step.

Corpus cases gain `main.satz` twins so both pipelines share one source:
- pipeline A: `main.satz` → gen.yaml → include/rename/fold → walk → HCL
- pipeline B: `main.satz` → fragments → `algebra::fold` → emit from `Folded`

The harness asserts **A(satz) == existing snapshot** (proves the satz twin is a
faithful conversion, continuously) and **B == A** (the differential gate),
sorted-byte-identical, with a per-address explained diff on divergence. A
ratchet list names the cases already at parity; unratcheted cases report
coverage without failing the build.

## Increments

- **I0 — harness + skeleton** (started): `src/differential.rs` test mod;
  `crates/satz-core/src/pipeline.rs` (front-end v0: params/uses/folder-scope →
  fragments; emitter v0 for plain resources); `main.satz` twin for
  hoist-two-folders. Ratchet empty; harness green and reporting.
- **I1 — first parity case**: hoist-two-folders ratcheted. Needs: cross-folder
  hoisting as fold idempotence (canonical-equal ⇒ one entity — a law, not a
  walk), grant label hashing, schema default injection, provider assignment,
  tfvars/variables parity.
- **I2 — corpus complete**: override-chain (param priority chain),
  depth-merge (sibling-map union in fragments replaces MERGE_KEY_PREFIX),
  billing-nested, real-packs (whole preset library through B).
- **I3 — estates**: estate 1 + estate 2 estates through B; sorted-identical against the
  production baseline, owner runs `tofu plan` = No changes.
- **I4 — flip**: B becomes the default for `.satz` inputs; delete
  MERGE_KEY_PREFIX, rename_duplicate_resource_keys, merge_renamed_resource_keys
  and the walk-interception paths; scope-as-typing replaces them. YAML inputs
  keep the old pipeline until M3 retires the dialect.
- **I5 — IR-level adoption diffs**: `presets::sem_equal` (and later the
  `.diff.satz` content) compares folded IR instead of compiled YAML twins.

## Parity notes (collected as they are discovered)

- Grant labels: `iam_<member-slug>_<hash>` — hash derived from member+role;
  emitter must reproduce the exact function (find it in transpiler.rs, reuse,
  don't reimplement).
- Defaults: `initial_group_config = "EMPTY"`, group `labels`, `parent =
  customers/<id>` are injected by the walk — must come from one shared place.
- Provider assignment (`provider = google.google`, per-project aliases) is
  config-driven; share the logic, don't duplicate.
