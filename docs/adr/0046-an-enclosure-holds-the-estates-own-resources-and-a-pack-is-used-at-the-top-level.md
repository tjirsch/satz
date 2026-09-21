# 0046 — an enclosure holds the estate's own resources, and a pack is used at the top level

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows (a MINOR under ADR 0010)

## Context

A `use` stood in four positions: the top level of a file, the body of a folder or a
project, `google_folder { … }`, and a resource type map. The body of a folder or a
project was the one that placed what the used file declared: the walk pushed the node
onto the path, so a pack's resources were scoped by where the line happened to sit. That
was the estate's way of saying "these resources belong in this folder".

Most of the placement it bought was imaginary. An audit of all 34 packs of the library,
each compiled bare and again inside a probe folder's body, found 32 byte-identical either
way. Their resources are either organisation-level — an org policy, a group, an
organisation grant, all of which hoist — or project-level, naming their project
themselves. Exactly two packs differed, each by one attribute on one resource: the
`google_project` they create took `folder_id = google_folder.<enclosing>.name` nested and
`org_id = "<org>"` bare. Those two took a param for it instead (ADR 0045), which reads the
folder from the estate and emits the same HCL either way.

What the position cost was real and recurring. Nearly every placement defect of the past
week came from a pack standing in an enclosure:

- `google_folder { use "<a pack that declares its own types>" }` compiled into folders
  named after the pack's resource types and emitted none of the pack's resources
  (ADR 0038 closed that one).
- The two monitoring packs' graph entry, header comment and test fixtures each claimed a
  different position for the same line, because nothing in the file said where it stood.
- An answer in the interview could not decide where a pack's project landed while the
  position of a line could overrule it, so the two folder params shipped without a
  question (ADR 0045): a question whose answer a line's position can overrule is a
  question that lies.
- The whole placement machinery of the pack graph — `block` as a dotted node path,
  `after_scaffold`, the checks and refusals that guarded them — existed to write a pack's
  line inside the scaffold and nowhere else.

## Options

- **(a) Keep it, and document the position per pack.** Each pack's header already states
  its own `use` line; make that the source of truth and check it. Costs a per-pack claim
  that nothing verifies against what the pack emits, and leaves the graph machinery, the
  interview's overrulable answer and the folder-name failure mode all in place. It also
  keeps two ways to say one thing — the line's position and the pack's folder param — with
  no rule for which wins.
- **(b) An enclosure holds the estate's own resources; a pack is used at the top level.**
  One rule, checked in one place: a `use` in the body of a folder or a project is refused,
  naming the node and the edit. A pack that needs a folder names it with a param, which an
  operator can read, an interview can ask and a report can print. Costs a breaking change:
  every estate with a nested pack line is refused until the line moves, and two of them
  bind a param as well.
- **(c) Remove nesting entirely** — flat files, every resource naming its parent
  explicitly, no `google_folder { x { google_project { … } } }`. That removes the same
  failure mode and more, and it removes what the nesting is FOR: the estate's own
  hierarchy, written once, with the parent references derived rather than repeated. The
  scaffold alone would gain a parent reference on the management project, the state bucket
  and the service account, each of which is currently read off the tree. Rejected: the
  convenience is the language's main idea and it has no defect.

## Decision

(b). The rule, stated once:

> The body of a folder and the body of a project hold the estate's own resources. A pack
> is used at the top level of a file.

`crates/satz-core/src/pipeline/position.rs` holds the check, beside the other position
refusals: `misfit` refuses an `Entry::Use` at `Position::NodeBody`, and the position now
carries the node's label so the refusal can name it. The message says what stands there,
what to do — move the line to the top level — and, for the two packs that create a
project, the param to bind and the reference to bind it to.

What nesting keeps, because neither has the defect:

- **A resource written directly in a node's body** is placed by that body. `google_folder
  { infra_folder { google_project { infra { … } } } }` is unchanged, and so is an org
  policy in a project's body.
- **An organisation-level type at a used FILE's top level** still reaches its own scope
  (ADR 0043). That is what lets one file declare a project together with the groups and
  grants that go with it.
- **A `use` in `google_folder { … }`** — a file of named folders — and **`use … as
  <type>`**, the flat spelling of a resource type map's content. `google_folder { … }` is
  a map of names, not an enclosure.

## Consequences

- An estate that nests a pack is refused until it is edited. No rewriter and no migration
  code (ADR 0041): the refusal names the edit and `## Breaking changes` in
  `presets/README.md` carries it, including which param the two project-creating packs
  need and that only the text of the line moves — a commented line stays commented.
- The pack graph's placement collapses to two kinds: the menu at the top level, and the
  resource type map a bare list of labelled bodies is written inside. `after_scaffold` is
  gone — it existed only so a line could come after a pack standing inside the scaffold —
  and `block` is restricted to a single resource type, a dotted node path being refused at
  parse time. With it go `Place::AfterScaffold`, `packs_after_scaffold`, the dotted-path
  walk in `insert_into_block` (now `insert_into_map`), `block_stub`'s refusal to invent a
  folder, `template::unknown_block`, `packs::Unplaced`, `PackLines.unplaced`,
  `pack_graph::for_writing` and pack-graph check 7. A graph can no longer place a line
  somewhere the estate does not have, so nothing has to report that it could not.
- `satz init` binds `logsink_project_folder = "google_folder.infra_folder.name"` in the
  estate it writes, because it writes that folder and used to write the pack's line inside
  it. Without that binding a new estate's archive project would be created under the
  organisation instead. `main.tf` for a fresh estate is byte-identical to what the release
  before emitted, with the pack off and with it on; the estate carries one more variable,
  which is what binding a param looks like.
- The interview asks for both folders. `logsink_project_folder` and
  `mdc_mgmt_project_folder` gained a question each, which ADR 0045 left out on purpose
  while a line's position could overrule the answer.
- A pack's header comment states one `use` line and not two, and `presets/docs/` prints
  that line. Where a pack's resources land is now readable from the pack's params rather
  than from the estate's indentation.
