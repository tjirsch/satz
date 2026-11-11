# 0082 — The library commands say `preset`, and `pack` is left for workload packs

- **Status:** accepted
- **Date:** 2026-11-11

## Context

A pack is a Satz file with a `pack <id> version "…"` header. Every file of the preset
library under `presets/` is one, and six commands that act on that library carried `pack`
in their name: `packs`, `add-pack`, `remove-pack`, `review-pack`, `doc-packs` and
`pack-graph`, with the MCP tools `satz_packs`, `satz_add_pack`, `satz_remove_pack` and
`satz_review_pack`. Three more already said `preset`: `get-presets`, `check-presets` and
`merge-presets`.

Two kinds of pack exist in practice:

- **A preset** is a pack of the library. The library is one interwoven set: the estate
  map offers each pack behind a gate, `pack-graph.json` orders them and holds the edges
  between them, the compliance catalogs point at their claims, the changelog and the
  library groups name each one. A preset configures an organisation, and its maintenance
  — pristine, fork, diff — follows from being part of that set.
- **A workload pack** is a pack a workload team uses in an estate of its own, beside the
  organisation: one building block for one workload, with no gate in the map, no node in
  the graph and no claim in a catalog. It stands alone the way an example does.

Commands for workload packs — fetching one, updating one — need names. `get-pack` beside
`add-pack` would read as two halves of one thing, while one downloads a workload pack and
the other switches on a preset of the library.

## Options

1. **Keep every name and add `get-pack`, `update-pack`.** No break. Cost: `add-pack` and
   `get-pack`, `packs` and `get-presets` stand side by side with different subjects, and
   the documentation carries the difference forever.
2. **Rename the library commands to `preset`.** Cost: a breaking rename of six commands
   and four MCP tools, in one minor release, with every script, runbook and client that
   calls them edited. The vocabulary is then one rule: a command that says `preset` acts
   on the library under `presets_dir`, a command that says `pack` acts on one workload
   pack.
3. **Call workload packs something else** (`template`). No break. Cost: a template is
   already a Satz file whose `inputs` are its contract (ADR 0079), which most workload
   packs are not.

## Decision

Option 2.

| before | now |
|---|---|
| `satz packs` | `satz presets` |
| `satz add-pack` | `satz add-preset` |
| `satz remove-pack` | `satz remove-preset` |
| `satz review-pack` | `satz review-preset` |
| `satz doc-packs` | `satz doc-presets` |
| `satz pack-graph` | `satz preset-graph` |
| `satz_packs`, `satz_add_pack`, `satz_remove_pack`, `satz_review_pack` | `satz_presets`, `satz_add_preset`, `satz_remove_preset`, `satz_review_preset` |

Unchanged: the language keyword `pack`, the folder `presets/`, the file
`presets/pack-graph.json` and the fields of every report (`packs`, `pack`), which name
packs in the language's sense; the flag `--no-pack-actions`; the finding kinds
`pack-requirement` and `unadopted-pack`.

There is no alias for an old name: everyone is on the current version, and a release is
the migration.

## Consequences

- The release that carries this is a minor (ADR 0010): a script that calls an old name
  is refused by the argument parser, and an MCP client that calls an old tool gets "unknown
  tool". `presets/CHANGELOG.md` → *Breaking changes* names the edit.
- A client that pins satz — an editor integration, a pipeline — moves its calls in the
  same step in which it moves its pin.
- The names `packs`, `review-pack`, `get-pack` and `update-pack` are free for the
  workload-pack commands.
- Records before this one keep the names of their day.
