# 0031 — the map offers every pack, and the pack graph ships with the presets

- **Status:** accepted; supersedes the part of ADR-0007 that has the map declare "the
  choices and nothing else"
- **Date:** 2026-09-19
- **Shipped in:** the release that follows

## Context

Which packs an estate can take, which question switches each one on, and which pack
needs which other one was spread over four places:

- `presets/estate-map.satz` — the choices, their questions, and six `ask_when` edges
  that hide a question while its parent is off;
- `PACK_LINES` in `src/template.rs` — path, gate, phase and block for 39 rows, compiled
  into the binary, with the dependencies between packs only as prose in the phase text;
- the estate's own `use … when` lines;
- the packs themselves — param defaults that read another pack's params
  (`organization-cis-log-alerts-central.satz`, `scc-findings-mail.satz`,
  `microsoft-sentinel.satz`) and `// NEEDS` prose in headers.

That has three costs. `PACK_LINES` is in the binary while the packs come from upstream
`main` (`get-presets`, `merge-presets`), so a map choice newer than the installed binary
gets no line and no finding. Six library packs had no row and nothing said they were
exempt. And a dependency stated only in prose cannot be checked: a consumer's line was
written before its provider's once (the Sentinel ordering), and satz-studio, which
derives its pack tree from `ask_when` alone, misses five real edges — central alerts →
archive, findings mail → central alerts, Sentinel → archive, billing → security groups,
runner grant → runner.

Every consumer — the CLI interview, the MCP tools, satz-studio — should run the same
logic against one graph, and satz-studio should derive nothing.

## Considered options

1. **The map carries one `offers` entry per pack; edges are derived from the packs'
   param references and `ask_when`, and declared on an entry only where derivation
   cannot see them; the graph ships as `presets/pack-graph.json`.**
2. **A `requires` statement in each pack.** Each pack says what it needs.
3. **Derived only.** No declarations; the graph is whatever the packs show.
4. **The graph in the binary**, generated at build time instead of shipped.

## Decision

Option 1. `offers "<path>" { when … phase … block … }` is a statement only the map
(`pack estate_map`) may carry; the parser refuses it anywhere else. The entry's position
is the adoption order. `requires` and `excludes` on an entry name only the edges the packs
cannot show: the billing grants reach the model's billing-admins group by a literal
address, the S1 model's two-file spelling replaces the one-file spelling under the same
gate, and a dry-run twin declares the same policies as its enforcing pack. `satz
pack-graph` refuses a declared edge it derives, so a declaration never outlives the
reason it was needed.

`satz pack-graph` builds the graph and runs its checks on the write path as much as
under `--check`: every library file is a node; every pack has a gate declared once;
paths are unique and a gate is shared only by packs with a declared edge between them;
no cycle; the provider of a `data` or `gate` edge is asked wherever its consumer is;
a consumer's line comes after its provider's; every block a line is placed in exists;
no declared edge duplicates a derived one. `presets/pack-graph.json` is a generated file
beside the packs, gated like `presets/docs/` by `--check` in the smoke matrix and in
`cargo test`, and `get-presets` and `merge-presets` carry it as an artefact.

The graph's types live in `satz-core`, so satz and satz-studio read the same structs.

The runner grant gets its own gate, `use_verification_runner_grant`, following
`use_verification_runner` by reference: in the MSP-hosted shape the runner and its grant
live in different estates, so one gate cannot describe both. The per-project alert pack
becomes the choice `use_project_cis_log_alerts`, off by default.

## Consequences

- The map stops being "the choices and nothing else": it is the library's menu, and a
  new pack cannot reach the library without an entry — `pack-graph` names the file.
- Until the estate writers read the graph, `PACK_LINES` and the entries say the same
  thing twice; a test holds them equal, and the follow-up deletes `PACK_LINES`. After
  that, the lines an estate gets come from the graph that arrived with its packs, so a
  pack newer than the binary gets its line.
- An entry emits nothing, so `check-presets` reports a changed entry the way it reports
  a changed question, and `merge-presets` updates the map without forking it. The
  entries are compared through their own canonical form (`canonical_offers`), not the
  one that decides a fork.
- `requires` edges from one pack to packs that exclude one another are one requirement
  that any of them meets. `data` edges from one pack to several providers of the same
  param are alternatives in the same way.
- A `data` edge is a compile failure only while the reading param takes its default: the
  runner grant compiles without the runner when the estate binds
  `ci_runner_service_account` itself.
- Every check needs the skeleton's blocks, which are the binary's; a block the graph
  names that the binary's scaffold lacks is a failed check, not a panic at `init`.

## Pros and cons of the options

### 1 — `offers` in the map, derived edges, declared exceptions *(chosen)*

- **Good:** each fact has one home: the gate and the order in the map, a param
  dependency in the pack that reads it.
- **Good:** a declaration is the exception, visible in one file, and refused once the
  packs show the edge.
- **Bad:** a grammar-visible statement that only one file may use.

### 2 — a `requires` statement in each pack

- **Bad:** it duplicates what a param reference already shows, and drifts from it.
- **Bad:** a pack would name the map's gates, so a pack could not be used outside the map.

### 3 — derived only

- **Good:** nothing to declare.
- **Bad:** blind to four real edges — the billing grants' group, the S1 spelling, the
  dry-run twins, and any requirement a literal carries.

### 4 — the graph in the binary

- **Bad:** it brings back the skew this record removes: the packs come from upstream
  `main`, the binary is whatever release is installed, and a pack newer than the binary
  would have no node.
