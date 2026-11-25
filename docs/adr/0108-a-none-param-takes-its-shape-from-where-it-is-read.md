# 0108 — A param no file gives a value takes its shape from where it is read

- **Status:** accepted
- **Date:** 2026-11-25
- **Amends:** [ADR 0106](0106-none-declares-a-param-without-a-value.md) (the `shape` of a `none` param)

## Context

`satz questions` reports a `shape` for each param question — `string`, `number`, `bool`,
`list` or `map` — and the interview, `satz_interview` and satz-studio type an answer in it:
one address for a list is a list of one. ADR 0106 took the shape of a `none` param from the
default another file gives it, else from the question's `recommend`, else none.

The library declares `access_approval_notification_emails` (`cis_extensions.access_approval`)
and `allowed_api_key_services` (`cis_extensions.api_key_services` and its dry run) `none`,
where they were `[]`. Neither has a default anywhere or a `recommend`, so neither had a shape:
an answer was typed as text, one address was written as `"security@example.com"`, the
transpile accepted it, and the apply failed because the provider wants a set. The pack says
what the param is in the one place it reads it — `notification_emails`, a set of strings in
the provider's schema; `allowedServices`, a list parameter of the managed constraint.

## Options

**A. Read the shape from where the param is read** (chosen). A param every declaration
leaves `none` takes the shape of its reads: an attribute's type in the provider schema (a
list or a set is a list, a map or an object a map), the list an `each` expands, a condition
(`when`, `use … when`, `ask_when`) a bool, a string it is interpolated into a string. A
param declared as another (`x = p`) and a local value read by its bare name carry their
reads to the param they name. Nothing to remember for an author, and the shape cannot
disagree with what the transpile writes, because it is read off the same place. Costs: the
questions report needs the provider schema whenever such a param is read inside a resource;
the JSON of a managed constraint's `parameters` is one string to the provider, so the
parameters' own types become data (`presets/managed-constraint-parameters.txt`, written
with the constraint pairing by `scripts/update_constraint_equivalents.py`); and two reads
that disagree must be refused, which is a new refusal.

**B. A `shape = …` field on the question.** Explicit, schema-free, one line per question.
Every author has to remember it for every `none` param, a forgotten one is the bug this
record answers, and a written one can disagree with the attribute the pack reads — a second
statement of a fact the pack already makes, which nothing would check.

**C. Back to `[]` for the two params.** No code. The question then offers an empty list as
its default, `--accept-defaults` takes it, and `allowed_api_key_services = []` forbids every
service-account API key — an answer nobody gave. That is what ADR 0106 removed.

**D. A typed declaration** (`p = none list`). Rejected in ADR 0106 for the reason B is: a
type system for params that states what the reads already state.

## Decision

A. For a param no file but the estate gives a value — every declaration in a used file is
`none` — the shape is read from where it is read:

- **An attribute** of a resource: the attribute's type in the provider schema. A list, set
  or tuple is `list`, a map or an object `map`, `string`, `number` and `bool` themselves. A
  read as an element of a list value is the element's type, a read as a field of an object
  value the field's type. `dynamic`, a key the schema does not name, a grant's roles and a
  configuration block (`providers`, `terraform`) say nothing.
- **A managed constraint's `parameters`** (`google_org_policy_policy`, a rule under `spec` or
  `dry_run_spec`): the parameter's type in `presets/managed-constraint-parameters.txt`,
  found by the policy's `name`. A test holds the table to the library: every parameter a
  pack writes has a row, and a default a pack gives there has the row's shape.
- **`each <p> by …`**: `list`. **`when <p>`, `use … when <p>`, `ask_when = <p>`**: `bool`.
  **`"…{p}…"`**: `string`.
- **`x = p`** in a `params` block and a local value `l = p` read by its bare name: the reads
  of `x` and `l` are reads of `p`.
- **Agreement.** `string` agrees with `number` and `bool`, and the more specific one is the
  shape — a number interpolated into a name is still a number. Any other two shapes
  disagree, and the transpile and `satz questions` refuse the param, naming both places:
  `param \`p\` is read as a list and as a string — a.satz:7 (\`emails\` of
  \`google_thing\`) and a.satz:10 (inside a string)`.
- **Read nowhere with a shape:** the question's `recommend`, else no shape — a string, as
  before.
- **A param a file gives a default** keeps the shape of that default. A default that
  disagrees with its reads is not checked here.
- **The estate's binding** is an answer, not a default: it neither gives the shape nor
  stops the param's reads from giving it.

The questions walk stays schema-free: it records each read with the path of keys it stands
at, and only the shapes of the params no file gives a value are read off those paths. `satz
questions`, the interview and `satz_interview` load the schema from the estate's
`schema_dir` only when such a param is read inside a resource; a missing schema is then
the transpile's own refusal (`no provider schema in '…' — … \`satz update-schema\` writes
it`), never a fallback to text. The transpile, which has the schema loaded, runs the same
walk and refuses two reads that disagree.

An answer given through `satz_interview` as one string where the shape is a list is
written as a list of one, as the terminal interview reads one typed address.

## Consequences

- `access_approval_notification_emails` and `allowed_api_key_services` are reported as
  `list`; one address answered for either is written as `["security@example.com"]`.
- An estate whose packs read a `none` param inside a resource needs its provider schema for
  `satz questions` and the interview — estate-core's own `none` params are read in
  resources, so in practice every estate does. `satz init` fetches the schema before its
  interview starts; an interview through `satz_interview` `create` in a directory without
  one is refused until `satz update-schema` has run.
- The transpile runs the schema-free questions walk once more per estate to find the reads.
- `presets/managed-constraint-parameters.txt` is a derived file (`docs/housekeeping.md`);
  a managed constraint a pack starts to configure needs its row before the test passes.
