# 0079 — A Satz file is a template for HCL teams: the organisation's values are a local, inputs become variables, everything else stays a literal

- **Status:** accepted
- **Date:** 2026-10-15
- **Shipped in:** `interface.tf` and the rendering pass in the release that follows; `inputs` in the minor after it

## Context

An organisation estate publishes values to the projects beside it through interfaces
(ADR 0070). A project estate written in Satz reads them as `${{interface.<export>}}`, and
its emitted `main.tf` carried them as literals — the organisation's domain, short name and
region spelled out in every resource, the lookups appended as `data` blocks. Every param of
the estate became a `variable` in `variables.tf` with its value in `terraform.tfvars`, for
reading only: no resource read `var.x`. So the HCL a transpile wrote was a one-off. A team
that works in OpenTofu alone could not reuse it with other values without re-running satz,
and could not point it at another organisation without re-running satz either.

Customer teams roll out many Google projects and folders under the workload folder, mostly
in OpenTofu, some in Satz. Thomas asked (2026-10-15) for one Satz template that serves both:
reused by a Satz team through params, and by an HCL team through `terraform.tfvars`, with
the organisation's values taken from the interface and kept out of the template's text.

Every satz check reads the *emitted* blocks: the manifest is built from them
(`src/emitter.rs`, `Manifest::from_blocks`), `check-consumer` and a project's own compile
map `data.*` references back to exports, `project_of` needs a literal project for the
prerequisite check, `depends_on` is derived from a manifest of the blocks, and the
prerequisite check reads `providers.tf` as text. A rendering that replaced a value with a
reference in the one body the fold holds would blind all of them.

## Decision

**A project estate's emitted HCL reads the organisation's values from `interface.tf`, and a
template's inputs from `terraform.tfvars`; every other value stays the literal the fold
judged.**

- **`interface.tf`**, beside `main.tf`, holds two locals — `interface_static` with every
  value the estate read that satz knows, and `interface`, which merges the looked-up ones
  in as the bare addresses of their `data` blocks — and then the `data` blocks those values
  need. `main.tf` references `local.interface.<export>`, `providers.tf`
  `local.interface_static.<export>`: bare where the whole value was read, inside a template
  where text held it. The split is forced by OpenTofu's graph: a provider is configured
  before any data source is read, the lookups run through that provider, and a provider
  that read a local holding one of them is a cycle (found on the first live plan). A copy
  of the file written for another organisation re-targets the project. A Cloud Identity
  group writes the literal: its address derives from its body.
- **The fold holds one body per entity, judged; the emission has a rendering beside it.**
  An entity that read an interface value carries a *rendering twin* of its body — the same
  shape, strings differing only where a value was spliced, a whole value as an
  `!expr`-tagged expression — and a grant's edge carries the rendered member and role (like
  `import_id`, outside the binding's identity, reconciled by the emitter). The emitter
  builds the blocks **twice**: a real pass from the bodies, which feeds the manifest, the
  findings and `imports.tf`, and, where any entity has a rendering, a rendering pass whose
  blocks become the text — aligned to the real pass's addresses and order and given its
  `depends_on`. Every decision a builder makes (a label, a parent, a scope, what is a
  block) reads the body in both passes. The two passes building different addresses is
  refused, not written. `providers.tf` is built twice the same way; the prerequisite
  check and the plan preflight read the written file through the names `interface.tf`
  and `terraform.tfvars` give its references.
- **`inputs { … }`** (the minor after this release) is the designation of a template's
  contract: declared like `params`, same namespace, scalars or lists of scalars. An input
  is a param for everything that judges — bound by a using document's `params { }` or by
  `--input name=value` on the command line — and the emitter writes `var.<input>` through
  the same twin, with the default as the example value in `terraform.tfvars`. An input
  cannot decide structure: an `each` list, a `when` gate, a label, a grant's member or
  scope, an export, a backend and an `hcl` passthrough refuse one at its line.
- **The interface module's provider pin is `>= <the estate's version>`**, not the exact
  version: an exact pin held every consumer to the estate's provider upgrades.
- **`check-consumer` reads `local.interface.<export>`** as a read of the estate, like a
  module's output: HCL a team keeps beside a generated `interface.tf` is judged.

## Options considered

- **Every param as a variable.** No designation, every scalar param `var.x`. Rejected: a
  param that is also a gate or a label part has to be refused or silently excluded, a rule
  nobody sees; and the transpile-identity proof (ADR 0033) relies on no resource reading a
  gate's variable.
- **Marker text in the judged values** — substitute `${var.x}` / `${local.interface.x}`
  into the one body and let everything read it. Rejected: `satz interfaces`, the manifest,
  `adopt`'s natural keys, the billed-project check and `report-compliance` would read
  markers in place of ids.
- **A side map keyed by (address, attribute path)** instead of a twin body. Rejected: the
  path would be threaded through every builder and `render_value`/`render_block` call
  (about forty sites) and the grant edges and scope pins would still need their own case.
- **HCL `for_each` over a list input.** Rejected again (ADR 0071): every consumer of the
  manifest, lookups, attach points and `check-consumer` read single-resource addresses.
- **A post-apply values snapshot** — `tofu output -json` of the organisation after each
  apply, written as tfvars so no team needs a lookup permission. Not built: the lookups
  stay live, read with the team's own credentials, and the deployer identity a team runs
  as carries the read right (`roles/browser` on the organisation) instead; a snapshot
  would be a second source of truth with its own staleness.
- **`terraform_remote_state`.** ADR 0070's rejection stands: it needs the state bucket and
  publishes everything.

## Consequences

- A project estate's `hcl/` gains one file; `main.tf` and `providers.tf` change text where
  a value came from the interface, and plan identically. The corpus snapshot of the project
  case carries an `interface.tf` section.
- The emitter costs twice on a project estate that read a value, and nothing on any other.
- A decision that reads text in a builder and is missed shows up as the two passes
  disagreeing — refused at the compile, never a wrong file.
- A group's values from the interface stay literals in `main.tf`; a project that exports
  an interface value onward keeps `${interface.x}` in its export, which the central
  estate's rule refuses (exports are never substituted).
