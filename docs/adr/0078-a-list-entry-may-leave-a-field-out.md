# 0078 — A list entry may leave a field out: request defaults and `each … when [not]`

- **Status:** accepted
- **Date:** 2026-10-02
- **Shipped in:** the release that follows

## Context

`each` (ADR 0071) writes one body per entry of a list param, and a body reads an entry's
fields. Every field the body read had to be in every entry, and every entry got every
body. A pack whose entries differ — a project with a budget and one without, a budget with
its own recipient and one without — could not be written: an optional field was refused at
the `each` line, and a resource for some entries only could not be expressed. Requesters
filled every field with a placeholder, or the pack forked. The budget pack
(`presets/organization-budget.satz`) was the first to need both: a channel only for the
budgets that name an address, and a budget that names the channel or none.

## Decision

**A request point's `defaults = { <field> = <value> }` fills a field an entry leaves out,
and `each <list> by <field> when <field>` / `when not <field>` writes the body only for the
entries that carry the field, or only for the others.**

- **Defaults live on the request point**, the declaration of the list's shape (`fields`,
  `patterns`). They are written into the list in the param namespace when the request point
  is read, before the file's items are walked, so an `each`, the patterns, the interface and
  `terraform.tfvars` read one filled list. A default may read a param; the key has none.
- **A project's request file is checked as written** and filled with the same defaults
  before its entries are compared with the estate's, so an entry already vendored is no
  collision.
- **`when` reads one field**: absent, `""`, an empty list or object, and `false` do not
  count. `when not` is its complement, so two `each` over one list write every entry once.
- **The canonical form** carries `when` and `defaults` only where written, so no pack that
  does not use them forks in an estate.
- **An empty `"import-id"` adopts nothing**: the emitter writes no import block for it, so
  a list entry can carry `import_id = ""` by default and an existing resource is adopted by
  giving its id.

## Options considered

- **Defaults on the `each`** (`each xs by name defaults { … }`): two `each` over one list
  could then disagree on what an entry holds, and the request check and tfvars would see
  another list than the bodies. Rejected.
- **A condition language** (`when a and not b`, comparisons): no pack needs more than one
  field; one field and its complement cover the budget and onboarding cases. Rejected until
  a pack needs it.
- **An optional field read as empty** (`each.x` of a missing field is `""`): hides a typo
  in a field name, which the `each` line refuses today. Rejected.
- **A channel always written, to the billing admins' group by default**: mails the
  administrators twice, once as IAM recipients and once through the channel. Rejected.

## Consequences

- The grammar (satz-tree-sitter `7e30532`) reads `when [not] <field>` after `by`; `not` is a
  keyword there only, and a field named `not` is written `when not {` in satz's parser.
- `docs/language.md` §6.4 and §6.17 state both; the showcase writes a subscription for the
  one topic that names one, and `billing` takes the default retention.
- A pack may now take projects' budgets, contacts or APIs as optional fields — the
  project-onboarding entry of the library roadmap is unblocked.
