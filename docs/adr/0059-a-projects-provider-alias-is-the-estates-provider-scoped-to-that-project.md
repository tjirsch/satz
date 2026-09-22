# 0059 — a project's provider alias is the estate's provider scoped to that project

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

The emitter writes one `provider "google"` per `google_project` the estate declares,
aliased `project_<label>`, and every resource written inside that project's body
carries `provider = google.project_<label>`. Two of its attributes were written by the
compiler rather than by the estate:

```
provider "google" {
  alias                 = project_<label>
  project               = <that project>
  billing_project       = <the infrastructure project>
  user_project_override = true
  region                = "europe-west3"
}
```

`region` was a string literal in `project_provider_block` (`src/emit_shared.rs`). An
estate whose `default_region` is `us-central1` still got `europe-west3` on every alias,
so a regional resource inside a project that wrote no `region` of its own was created
in a region the estate never names. Nothing caught it: `providers.tf` is in no corpus
snapshot, and the corpus snapshots `main.tf`, the tfvars and the imports.

`billing_project` pointed at the infrastructure project, and `user_project_override =
true` sends it as `X-Goog-User-Project` on every call the alias makes. Google then
tests API enablement and quota on THAT project, not on the one the resource lives in.
Measured against a live organisation: a `google_kms_key_ring` in a project that had
`cloudkms.googleapis.com` enabled was refused with a 403 naming the infrastructure
project, where the API was off. The estate could not overrule it — `providers { }` is
keyed by provider name, so a second `google` entry is a repeated key.

That central quota project is a deliberate thing for the DEFAULT provider
([ADR 0036](0036-plan-and-apply-enable-the-apis-the-estate-declares.md),
[ADR 0023](0023-one-command-for-every-prerequisite.md)): an org policy hangs off the
organisation and a budget off the billing account, so the calls that create them have
no project of their own and need one named for quota. The per-project alias is the one
case where a project IS named, and it inherited the central setting from the same
helper, together with `has_billing_project = false` hard-wired at the call site.

## Considered options

1. **Bill the alias to the project it is for.** The alias is the default provider
   scoped to one project; the quota project is that project.
2. **Keep the infrastructure project and let the estate override it**, through a param
   or a per-project attribute.
3. **Keep it as it is** and document that every API a project's resources need must
   also be enabled on the infrastructure project.

For the region: read it from the estate's own `google` provider block, or from the
`default_region` param, or keep a literal and make it a param.

## Decision

Option 1, and the region comes from the estate's `google` provider block.

The per-project alias is the estate's `google` provider scoped to one project and
differs from it in the project alone: `project` and `billing_project` are that project,
`user_project_override` stays `true` so the project is sent as the quota project,
`impersonate_service_account` is the estate's IaC service account as everywhere else,
and `region` is whatever the estate's `google` block names.

The region is read from that block rather than from `default_region` because the block
is where the estate states which region its google provider works in; the scaffold
binds it to `default_region`, and an estate that pins a literal there gets that literal
on its aliases too, instead of two provider blocks disagreeing. An estate whose block
names no region gets aliases that name none: a regional resource that writes no
`region` is then refused by the provider, rather than created somewhere the estate
never said.

Option 2 buys a lever nobody has asked for over a default that is wrong for every
estate, and option 3 keeps satz demanding an API on a project the resource has nothing
to do with.

The prerequisite check is unchanged: it still judges every API the estate's types need
against the infrastructure project's `project_service` list, which is what the default
provider needs. What it does not do is name the resource's own project for a resource
inside a project node — the finding says so in words, and making that check per-project
is its own piece of work (`ApiNeed`, `missing_apis`, `write_apis`, the report's
`infra_project` and `infra_services`, and `bootstrap` all assume one project today).

## Consequences

- A regional resource inside a project node is created in the estate's region. For an
  estate whose `default_region` is not `europe-west3` and that has such a resource
  without its own `region`, the next plan moves it — destroy and create, in the region
  the estate names. That is the fix landing, and `presets/README.md`'s
  `## Breaking changes` carries it.
- A resource in a project whose own APIs are enabled is created without the API also
  being on the infrastructure project. Where an estate relied on the central quota
  project for a project that has the API off, the create now fails naming that project
  — which is the project whose `project_service` list the estate controls.
- `providers.tf` moves for every estate that declares a project; no resource in
  `main.tf` moves, so the corpus snapshots do not change. The alias blocks are gated by
  unit tests instead (`emitter::project_alias_tests`), which is what was missing.
- `configure_google_provider` is no longer called for the alias, so its two hard-wired
  `false` arguments are gone. It still serves the estate's own provider blocks, where
  a declared `billing_project` or `user_project_override` is left alone.
- An estate that wants a project's calls billed centrally says so by writing that
  project's resources at the top level with `project = …`, where the default provider
  serves them.

## Pros and cons of the options

### 1 · The alias bills to its own project *(chosen)*

- **Good:** the project whose API enablement Google tests is the project the resource
  lives in and the project the estate's `project_service` entries enable.
- **Good:** nothing new to bind; the fix reaches every estate on upgrade.
- **Bad:** an estate that had the API only on the infrastructure project now fails
  where it used to work. It fails naming the project it has to enable it on, and that
  project is the one the resource belongs to.

### 2 · Keep it central, with an override

- **Good:** no plan moves for anyone until they ask for it.
- **Bad:** it is a param for a default that is wrong everywhere — the opposite of the
  params-over-forks rule, which is about customisation, not about repairs.
- **Bad:** every estate with a project outside the infrastructure project has to learn
  this and bind it, or keep hitting the 403.

### 3 · Keep it and document it

- **Good:** nothing to build.
- **Bad:** it makes every project's APIs the infrastructure project's business:
  `cloudkms` on the infrastructure project because another project holds a key ring.
- **Bad:** the 403 names a project the operator did not write anywhere near the
  resource, which is how the defect survived to a live rehearsal.

### The region, from the estate's provider block rather than from `default_region`

- **Good:** one source. The alias and the provider it is a copy of can never disagree.
- **Good:** an estate that pins a literal region on its provider gets it everywhere.
- **Bad:** an estate that writes a `providers` block without a region gets aliases
  without one, and a regional resource that writes none is refused by the provider.
  A refusal that names the attribute is better than a resource in Frankfurt.
