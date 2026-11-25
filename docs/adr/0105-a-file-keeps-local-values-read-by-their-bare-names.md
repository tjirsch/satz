# 0105 — a file keeps local values, read by their bare names and unique in the estate

- **Status:** accepted
- **Date:** 2026-11-25

## Context

Params share one namespace across the estate and every file it uses (`docs/language.md`
§6.3, rule 3), and since ADR 0104 two used files that declare one param with different
defaults are refused. That turns a silent wrong value into an error, but it leaves a pack
no way to keep a value to itself: every value a pack composes for its own resources — a
bucket name built from the customer's short name, a list it expands with `each` — is a
param, so it collides with another pack's param of the same name, an estate can bind it,
the interview may ask it, and it is written to `variables.tf` and `terraform.tfvars`.
ADR 0104 recorded this as option B, planned before 1.0.

The statement is `locals { name = value }`, in a pack or an estate, and what it declares
is a **local value** — "private" is the `private <type>.<label>` statement (ADR 0070).
Four questions had to be settled.

## Options

**1. How a local value is read.**

- *By its bare name* (`bucket`, `"{bucket}"`, `each buckets by name`, `when keep`), as a
  param is. A pack moves a value from `params` to `locals` without touching a read, and a
  reader does not have to know which of the two a name is. Costs a collision rule: two
  names read the same way have to be kept apart by the declarations, or a read would mean
  one thing in one file and another in the next.
- *As `local.x`*, as Terraform writes it. The read says what it reads and needs no
  collision rule beyond the file. Costs a second reference form in the language, every
  read rewritten on the move from `params`, and a third prefix beside `each.` and
  `interface.` in the reads every tool parses.

**2. How unique a name is.**

- *Unique in the whole estate*: no param, input or other file's local value carries it,
  and a using file that names it in its `params` is refused as a binding. With bare reads
  this is what keeps a local value and a param apart, and it is the rule the walk can
  check where it meets the second of two declarations. Costs a rename when two packs pick
  the same internal name — the cost ADR 0104 already asks of params.
- *Unique in the file only*: a local value shadows a param of the same name inside its
  file. Packs never collide. Costs a read whose meaning depends on the file it stands in,
  and an estate binding a pack's param that a later version of the pack turned into a local
  value would be ignored without a word.

**3. What the HCL holds.**

- *The literal, inlined*, as every param read is (params are literals in `main.tf`, and
  `variables.tf` exists for the reader). A local value then needs nothing from the emitter
  and the HCL of a pack that moves a value from `params` to `locals` is unchanged apart
  from the variable it no longer declares.
- *A Terraform `locals` block*, the read written `local.x`. Closer to hand-written HCL;
  costs a second way the emitter writes a read, a name space in `main.tf` shared with the
  interface's own `locals`, and a plan that reads a value satz already fixed.

**4. What may read a local value.** A param's default reading one would hand a file's own
value to every file that uses it; deferring it keeps a param's default a function of
params alone, which the interview, the presets report and `estate_params` compute without
walking the estate. Reading an input, `each.<field>`, a question, `ask_when`, `use … when`
and an `entries` or `contributes_` list are refused for the same reason: each is read by a
pass that runs on params alone, or decides structure the transpile has to fix.

## Decision

Bare reads, a name unique in the whole estate, the literal inlined; a param's default, an
input, a question, `ask_when`, `use … when`, an `each.<field>` and an `entries` or
`contributes_` list never read or name a local value. Exports, actions, `suppress`, a
contribution's entries and an `entries` statement's `defaults` read the local values of
their own file, resolved after the walk against a per-file table.

The walk keeps the file's resolved local values beside the param namespace for that file
alone: they leave the namespace before a used file is read and return when it is left, so a
used file never sees its user's local values and `terraform.tfvars` holds none. The three
walks that read params and questions without a schema (`collect_params`,
`collect_questions`, the contribution seed) are left separate: none of them reads a local
value, because nothing they evaluate — a param's default, a gate, a question — may read one,
so merging them into one recorded walk would move code without making this change safer.

A local value is part of a pack's canonical form, so a changed one is a pack change that
`check-presets` reports; `doc-presets` does not list local values, and the pack graph
counts a local value's read of another pack's param as a `data` edge. `satz import ./hcl/`
writes a Terraform `locals` entry with a literal value as a local value.

## Consequences

- A top-level `locals { … }` was an unknown resource type before this change, so no estate
  that transpiled changes meaning. A resource labeled `locals`, written unquoted in a type
  map, is now refused as a statement inside a block; it is written quoted.
- The tree-sitter grammar models the statement before the grammar gate
  (`scripts/check-grammar.sh`) passes, and satz-studio's document layer learns it as a node.
- A param's default reading a local value is deferred; allowing it later needs the
  schema-free passes to compute local values too.
- ADR 0104's option B is this record; a namespace per `use … as <name>` (option C) stays
  planned.
