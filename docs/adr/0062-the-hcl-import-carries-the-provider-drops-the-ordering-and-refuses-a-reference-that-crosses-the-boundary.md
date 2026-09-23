# 0062 — the hcl import carries the provider, drops the ordering, and refuses a reference that crosses the boundary

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

The hcl shape of `satz import` (`crates/satz-hcl`) sorted every input block into
three tiers: promoted to a param, translated to a Satz resource, or carried
verbatim inside `hcl trust`. A block using `count`, `for_each`, `provider` or
`depends_on` was disqualified from translation — the four were one list of
meta-arguments, `META_ATTRS`.

Two of those four are what satz itself writes. The emitter puts `provider =
google.<alias>` on every resource it emits and derives a `depends_on` wherever the
plan needs ordering that no reference already gives it (`order_after_service_accounts`,
`order_after_groups`, `order_after_custom_constraints`, `order_org_policies_per_parent`,
`order_after_project_services`, all in `src/emitter.rs`). Measured on the smoke
estate, whose `main.tf` carries 90 resources: **0 translated, 90 wrapped** — 44 on
`uses provider`, 43 on a label the import did not accept, 3 on a Satz form of their
own. An import of satz's own emission produced a passthrough estate: it deploys, and
`require` and `report-compliance` see nothing in it.

Worse, the split was not safe where it did happen. A translated resource that
referenced a wrapped one wrote `${google_kms_key_ring.ring.id}` into the estate,
and satz emits no address for an `hcl trust` block — it is text, and
`src/manifest.rs` does not hold it. `satz transpile` refused the estate the import
had just written (`written-reference … no google_kms_key_ring is emitted here at
all`), while the import itself only printed a note: *"verbatim `${…}` reference(s)
leave this import and must resolve in the target module"*.

## Decision

**Judge each meta-argument by whether Satz can say it, and never write an estate
`satz transpile` refuses.**

1. **`provider` translates.** It is already one of the nine keys a Satz resource
   body carries that no provider schema names (`satz_body_key`,
   `crates/satz-core/src/pipeline.rs`, ADR 0047), and `emit_shared.rs` renders the
   string back as the reference it was. The alias that is the estate's own —
   `google.google`, which `base_estate()` declares and the emitter writes on every
   resource that names no other — is left out of the body, so the round trip is the
   same bytes rather than the same attribute moved down the block. A folder, a
   project, a project's service list, a group and every grant written as a member
   map are built from a form of their own that takes the alias from the estate and
   reads none from the body: there an alias that is not the estate's own wraps the
   block, naming it.

2. **`depends_on` is dropped, per edge, and reported.** Satz has no `depends_on`
   and gains none: ordering is derived from the estate, which is why a pack author
   never writes one. The edge is dropped only when every address it names is a block
   this import carries — then the emitter has the same estate to derive from, and
   `depends_on` is not in a plan diff, so `tofu plan` against the source's state
   still shows no changes. An edge naming something the import does not hold wraps
   the block instead: nothing can re-derive it. Every dropped edge is in the report
   (`--verbose`), with a one-line note on every run.

3. **A `${…}` that crosses the boundary refuses the import.** A translated resource
   may reference only another translated one. The import names both sides — the
   referring block with its file and line, the referenced address, and the reason
   the other side is verbatim — and writes nothing.

4. **A label is letters, digits and `_`.** The import required all-lowercase, which
   rejects `compute_managed_requireOsLogin` and every hashed grant label satz itself
   emits. A Satz label emits as itself with `-` replaced by `_`, so a label already
   spelled that way survives the round trip unchanged.

5. **A grant map's member must be a member.** `satz_core::pipeline` reads a key
   with no `:` as the scope the map pins (`is_scope_attr_key`), so a `member` that
   is a bare `${…}` reference would become a scope attribute and the front end
   would refuse the estate. Such a grant wraps.

## Options considered

**For the crossing reference — refuse, or wrap the referring block too?** The
importer already closes the other direction: a verbatim block that references a
translated resource whose emitted label is derived wraps that resource. Mirroring
it would always produce a working estate and never refuse. It was rejected: the
cascade is silent, and it takes coverage away exactly where the reviewer believes
they have it — a resource wrapped because of a neighbour is a resource `require`
and `report-compliance` no longer see, with no line in the report saying which
neighbour. And it cannot save the other case anyway: a reference to an address no
input declares dangles in `main.tf` whether the referring block is wrapped or not.
One rule for both, stated once, is worth more than a rescue that half works. The
refusal carries the three ways forward, and `--wrap-all` is the deliberate,
whole-import version of the cascade.

**For `depends_on` — drop it, or add it to the language?** A tenth body key would
carry the edge exactly and lose nothing. It was rejected: it re-proposes a decision
the language already made (the emitter derives ordering, so a hand-written edge
would be a second source of truth for it), and a `depends_on` in Satz is a
foreign Terraform address in a language whose addresses come from where a resource
stands — an estate could be edited into an edge that names a label the emission no
longer writes.

**For `provider` on a grant map — accept only the estate's own alias, or wrap every
grant carrying one?** Wrapping every one would leave satz's own grants, which all
carry `provider = google.google`, untranslatable forever. The coupling this
introduces — the importer knowing what `base_estate()` declares — is one constant,
`DEFAULT_PROVIDER`, and a test (`base_estate_declares_the_default_provider`)
that fails if the two ever disagree.

## Consequences

Measured on the same 90-resource emission, feeding `main.tf` back through
`satz import`: **83 translated, 7 wrapped, 0 dropped**, and the estate transpiles.
The re-emission holds the same 90 resource addresses, and no resource body differs
except where the import placed a resource inside a project — there it takes that
project's provider alias and writes its `project` as a reference rather than the
literal, which is satz's own convention for a resource that stands in a project.

The seven that stay verbatim each have a reason that is not about a
meta-argument: a billing grant (it hoists to the estate's billing scope), a group
and its membership (a derived Satz form), an audit config (authoritative), two
grants whose member is a `${…}` reference, and a bucket whose `project` is written
as a quoted `"${google_project.x.project_id}"` — a scope the placement rule reads
only as a bare traversal.

What is lost: of the 119 ordering edges the source carried, the re-emission derives
51. The 68 it does not are the "wait for the API that enables you" edges, which
`order_after_project_services` bounds to the resource's own project and the infra
project the provider calls are billed to — and an imported estate carries no
`infra_project_name`, so the org-level policies get no edge to the services on it.
That changes no plan; it can change the order of a first apply on an estate whose
APIs are not yet on, which an adopted estate's are.

A `.tf` directory that imported before and has a reference across the boundary now
refuses (`presets/README.md`, `## Breaking changes`). This is a minor release by
ADR 0010: the same input is refused.
