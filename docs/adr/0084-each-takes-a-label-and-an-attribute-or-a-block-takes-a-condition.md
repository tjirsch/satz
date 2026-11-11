# 0084 — `each` takes a label, and an attribute or a block takes a condition

- **Status:** accepted
- **Date:** 2026-11-11

## Context

Two things a pack could not say without writing a body twice.

- **Several resources of one type per entry of a list.** `each <list> by <field>` labels
  each body with the entry's field, so two `each` over one list in one type map write the
  same labels and the fold refuses the second. A project's body takes resources under
  written labels (`"{each.name}_iac" { … }`), which is how the deployer and the
  onboarding packs write an account and a bucket per entry; a type with no node to nest
  in — a Cloud Identity group, a topic — has no such place. The way out was a second
  field: `each <list> by viewer_group`, which makes the resource's address follow a name
  the customer configures, and moves the address when the name is changed.
- **One attribute or one block that is there for some and not for others.** `use … when`
  gates a pack and `each … when <field>` a whole body. A body that differs in one block
  was written twice: `organization-budget.satz` carried the budget once with its
  notification rule and once without, under `when email` and `when not email`. A pack that
  takes a customer-managed key only where the customer has one had no form at all short
  of two packs.

Neither is a value: both decide what the estate holds, so both are settled when the
estate compiles, as `each` and `use … when` are.

## Options

**The label**

1. `each <list> by <field> as "<label>"`: the label is a string that reads the entry.
   Cost: one more clause on `each`.
2. A written label inside the body of an `each` in any type map, as a project's body
   takes one. Cost: an `each` body would be a resource body in one map and a map of
   labelled bodies in another, decided by nothing the line says.
3. Nothing: label by another field. Cost: the address follows a configured name.

**The condition**

1. `key = value when <param>` and `key [name] when <param> { … }`, with `not`, and inside
   an `each` also `when each.<field>`. Cost: a keyword in a new position, and every
   reader of a body has to meet a body the walk has already settled.
2. An empty value means "leave it out". Cost: satz would guess; an attribute set to `""`
   on purpose and one left out are different plans.
3. A conditional expression in the value (`a ? b : null`). Cost: a value that decides
   structure, which is what satz keeps out of values, and the first of a family of
   operators.
4. General functions over lists and strings. Cost: the same, and the tools that read an
   estate without evaluating it — adoption, the claims, the interfaces, the reviews —
   would have to evaluate it.

## Decision

**Option 1 of each.**

- `each <list> by <field> as "<label>" [when [not] <field>] { … }`. The label reads the
  entry (`{each.x}`) and text, never a param; it is a label like any other — a letter,
  then letters, digits, `_` and `-` — and two entries may not make the same one. The
  field after `by` still names the entry in every message.
- `… when [not] <name>` on an attribute and on a block, a labelled resource in its type
  map included. The name is a param, or `each.<field>` inside an `each`. What carries a
  value is what does for `use … when`: not `false`, not `""`, not an empty list or
  object. A param nobody declares is an error and an input is refused, as for a gate.
- The walk settles every condition where it expands `each`, before anything reads the
  body, so the fold, the schema check, the manifest and the emitter see an ordinary body.
  A condition inside an object that is a value, and one at the top level of a file, are
  refused.
- The canonical form of a pack carries `as` and a condition only where they are written.

`organization-budget.satz` 2.1 writes each budget once, with its notification rule under
`when each.email`. Its emission is the one 2.0 had.

## Consequences

- An address can stay fixed while the name a customer configures changes: the label
  reads the entry's key, the name another field.
- A key written twice in one body stays an error under a condition too, so an attribute
  with one of two values is two params, not two lines.
- A condition tests one name for carrying a value. There is no comparison, no `and` and
  no `or`.
- The tree-sitter grammar and the tools built on it read the two forms once they are in
  the grammar; until then a file that uses them is an error node there.
- An estate that uses `organization-budget.satz` sees a changed pack at its next
  `merge-presets`, which forks it unless `--adopt organization-budget` takes it in
  place; the adoption moves no plan.
