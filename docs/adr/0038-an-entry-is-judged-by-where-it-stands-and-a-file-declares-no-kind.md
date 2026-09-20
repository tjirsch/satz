# 0038 — an entry is judged by where it stands, and a file declares no kind

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** the release that follows (a MINOR under ADR 0010)

## Context

A `use` stands in four positions: the top level of a file, the body of a folder or a
project, `google_folder { … }`, and a resource type map (`google_x { … }`, or
`use … as google_x` written flat). The library has files of two shapes to go with them.
Most packs declare their own resource types and are used bare. Three are bare lists of
labelled bodies — the contacts pack and the two group fragments of the first security
model — and are used inside the map of their type.

Two things held by construction and were written down nowhere. A used file's resources
are placed by where its `use` stands: a `use` in a folder's body is walked as top-level
entries with the folder pushed onto the path. And a used file's `params`, `question`,
`claim`, `notice`, `action` and `hcl` statements are absorbed into estate-wide
accumulators before the walk looks at the position at all, so a bare list with a
`params` block and a `question` is a resource map's content and the map receives the
labelled bodies alone.

What did not hold was that an entry which belongs nowhere is an error. Each walker read
every entry as whatever its position takes:

- `google_x { params { … } }` compiled into a resource labelled `params`.
  `google_folder { params { … } }` made a folder of that name, and `params { … }` in a
  folder's body became an attribute `params = { … }` of the folder, which the provider
  rejects at plan time.
- `google_folder { use "<a pack that declares its own types>" }` was the one `use` path
  that skipped the existing refusal of a typed pack as map content. The pack's type
  keys became folder names: a CIS extension used that way emitted
  `google_folder.google_org_policy_policy` and none of its policies. The logging pack
  was stopped only because its cross-resource references no longer resolved; a pack
  without references was misplaced without a word.
- A file holding statements and no entry (`estate-core.satz`) inside a resource type
  map compiled and contributed its params from there.
- A `suppress` in a used file was parsed and never read: only the estate's own
  suppressions reach the fold.

An earlier draft answered this with pack KINDS: a declared marker per shape (the
existing `content` header word for bare lists, a second word for estate-level files, a
map-pack declaration), checked against the position of the `use`.

## Options

- **(a) Declared kinds.** A pack states what it is, and the compile checks the kind
  against the position. It reads well on the pack's first line. It costs a header
  vocabulary that every pack and every fork has to carry and keep true, a second source
  of truth beside the file's contents — a bare list marked as typed is a new way to be
  wrong — and a migration of every shipped pack and every fork in the fleet. It also
  answers only the used-file half: `google_x { params { … } }` written by hand has no
  header to consult.
- **(b) Judge the contents by the position.** Each position takes its own kinds of
  entry, and ONE check says whether an entry fits — applied to a hand-written entry by
  the walker of its position, and to a used file's entries with the position of its
  `use`. No file declares anything; an estate-level preset inside a resource type map is
  refused because it holds nothing that position takes, without satz knowing that it is
  an estate-level preset.
- **(c) Reshape the library** so that only one shape exists: wrap the three bare lists
  in their resource type and refuse `use` inside a resource type map altogether.
  Simplest rule of all, and it breaks the form most estates use for the contacts pack,
  for a shape whose bodies are valid where they stand and whose params already go where
  they belong.

## Decision

(b). The rules, as decided:

1. A `use` is fine as such, gated or not.
2. Context places what the used file declares.
3. A `params` or `question` in a used file goes to its Satz destination, never to HCL.
4. An element that belongs neither to Satz nor to HCL — the language plus the provider
   schema — is an error, including an estate-level file used inside a resource type.

`crates/satz-core/src/pipeline/position.rs` holds the check: `Position`, `misfit` for
one entry, `used_file_fits` for a used file, and `STATEMENTS` — one row per statement
saying what it does at the top level and what happens to it in a used file. A test holds
that table against `satz::STATEMENT_KEYWORDS`, which is itself derived from the parser's
dispatch, so a statement added to the language without a decision here fails the build.

Two judgments inside (b) were not obvious:

- **Only an identifier key is ever a statement or a type; a quoted key is a name.** A
  resource, folder or project that really is called `params` is written `"params" { … }`
  and emits the same address. This is the rule the typed-pack refusal already used, and
  it is what keeps `action { type = "Delete" }` inside a `lifecycle_rule` out of the
  check: the check looks at the direct children of a position, never into a resource's
  own body, which is the provider's.
- **`hcl` and `suppress` are treated differently, from what each does.** An `hcl` block
  of a used file has a destination that does not depend on the position — the raw
  passthrough, beside the resources — so it is absorbed like `params`, from every
  position. A `suppress` has no destination from a used file at all: suppression is the
  estate's own channel for declining what a pack provides, and only the estate's file is
  read. It is refused in a used file at every position, rather than absorbed, because
  absorbing it would let a pack remove resources of another pack. Written by hand inside
  a block, both were parse errors already; they now say that they are statements.

The `content` header word is removed. It marked one shipped file and decided nothing;
under (b) nothing may decide by it. A word on a header's line is refused, because the
block parser would otherwise read `content params { … }` as a block and lose the params.

## Consequences

- An estate that wrote a refused form is refused until it is edited. No rewriter, no
  migration table and no change to how `merge-presets` edits a line: the compile
  refuses, and `## Breaking changes` in `presets/README.md` says what to edit. That
  section is the one place in the docs that speaks of what satz used to do.
- No pack is reshaped or moved. The contacts pack loses one word and gains a version.
  An estate holding the old copy does not compile, and `merge-presets` — which compiles
  the estate before it changes anything — stops with it. No mechanism is built around
  that: the remedy is `satz get-presets --force`, which overwrites the pristine copies
  of the packs the estate uses, changes to them included, and says so. `--force` reads
  the `use` graph without stopping at a pristine copy that does not parse; every other
  walk still stops there, because there the used set decides what may be overwritten.
- A file of labelled bodies whose labels are all commented out is refused inside a
  resource type map, since it holds no entry. The `use` line is what to comment out.
- A used file is judged before it is walked, so the refusal is located at the `use` —
  the line to edit — and names the entry's line in the used file.
- The schema-free walks (`estate_params`, `estate_questions`) do not apply the check:
  they run before any schema is loaded and cannot tell a type from a label. They follow
  a `use` from every position, which is rule 3.
- Whether a labelled body's ATTRIBUTES are the provider's is not part of this check. The
  emitter reports a missing required attribute and a wrong shape; an attribute the
  schema does not know is still emitted.
- What a project's body places is decided separately: the emitter reads a folder from
  the path of a resource and not a project.
