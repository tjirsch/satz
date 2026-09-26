# 0071 — `each` expands a list param into labelled bodies at compile time

- **Status:** accepted
- **Date:** 2026-09-26
- **Shipped in:** the release that follows

## Context

A pack is one instance: its params join one estate-wide namespace, and nothing in Satz
turned a list into resources. That shaped three designs in one day. Onboarding a project
became a section `satz add-project` generates instead of a pack taking a list of
projects (ADR 0070); a request for subnets needed a second request form, because a
subnet is its own resource and a contributed list entry could not become one; pack
instances were deferred. Each workaround was a mechanism of its own.

## Decision

**`each <list param> by <field> { … }`, inside a resource type map, is expanded by the
compile into one labelled body per entry of the list** — the label the entry's `<field>`,
`{each.x}` in a string or a key the field's text, a bare `each.x` its value.

- **At compile time, never as `for_each`.** satz knows every list once params and
  contributions are resolved, so it writes N ordinary bodies before the fold
  (`Walk::expand_each`, `crates/satz-core/src/pipeline.rs`). The emitted HCL is what
  hand-written bodies give: plain addresses, no `for_each` in `main.tf`, so plan, import,
  `adopt`, claims, attach points, `all … under`, `check-consumer` and the interface read
  them unchanged.
- **Keyed by a field, not by position.** Reordering the list moves nothing; renaming an
  entry's key moves its resource, as renaming a written label does.
- **The list is the one the compile sees, contributions merged** (ADR 0051), so an entry
  a pack or a team contributes expands like the estate's own.
- **Where it stands:** inside a resource type map — a type's, `google_folder { … }`,
  `google_project { … }`, at the top level or in a node's body. Not in a grant map, whose
  entries are members; not in a resource body; not at the top level of a file.
- **One level.** An `each` inside an `each` body is refused; nested resources in the body
  are expanded with it, a per-entry label written as a quoted key (`"{each.name}_iac"`).
- **A dot in an interpolation name** (`{each.name}`) is lexed; before, `{a.b}` was an
  error, so no file changes meaning.

## Options

**HCL `for_each` in the emission.** *Rejected.* Every consumer of the emission manifest —
adoption by natural key, claim witnesses, the interface's lookups, attach points,
`check-consumer` — reads addresses of single resources; `for_each` addresses
(`google_x.y["k"]`) would need a second path through each of them, and the plan would
depend on a value only Terraform resolves.

**Pack instances** (`use "x.satz" as payments { params { … } }`). *Deferred.* A second
param scope per instance, labels prefixed per instance, and every tool that reads pack
lines (the interview, the pack graph, `merge-presets`) learning instances. `each` covers
the cases that arose — a list of projects, subnets, folders — with one expansion point.

**Two request forms** (entries in one resource, and whole resources a request file may
declare). *Superseded* before it was built: with `each`, a request is always entries
contributed to a list, whatever the entries become.

## Consequences

- `each` on `interface` and `export` statements is not built: onboarding a project stays
  `satz add-project` until an interface can be written per entry.
- The grammar (`satz-tree-sitter`), the Zed extension and satz-studio's tree learn the
  entry; the canonical form carries it, so pack drift sees it.
- An entry's label is part of what the estate publishes when an `all` export reaches its
  type: renaming it is a breaking change for the projects that read it.
