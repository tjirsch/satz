# 0070 — an estate publishes a declared interface module of static values and lookups; consumers attach in their own state or contribute through the estate

- **Status:** accepted
- **Date:** 2026-09-25
- **Shipped in:** the release that follows

## Context

satz builds a customer's organisation: the folders, the infrastructure project, the
shared network, the organisation policies. The teams that run workloads in it write
their own HCL, with their own state and in their own repositories, often without satz.
They re-typed the core facts — the organisation id, the domain, the region, the project
ids — and looked folder ids up by hand. There was no output and no data source in what
satz emits: a written reference (`"${{google_folder.x.name}}"`) reached only the estate's
own resources. When a team needed to change shared infrastructure — attach its project to
the shared VPC or to a service perimeter, add a subnet — there was no agreed way, so it
was whoever got there first, in whichever state.

Three terms, for a reader who has not used them:

- An **output** is a value a Terraform module publishes. The root module's outputs are
  what `tofu output` prints; a child module's are what the calling configuration reads
  as `module.<name>.<output>`.
- A **data source** (`data "google_project" "x" { … }`) reads an existing object through
  the provider at plan time. It creates nothing and needs read permission only.
- An **attachment resource** is a provider resource that joins one object to another
  that a different state owns, without owning the other:
  `google_compute_shared_vpc_service_project`,
  `google_access_context_manager_service_perimeter_resource`, every `*_iam_member`.

## Decision

**An estate declares what it publishes with `export "<name>" = <value> [description
"…"]`, and satz generates a relocatable module, `hcl/interface/`, with one output per
export, plus the same outputs in the root module's `outputs.tf`.**

- **Static or looked up, decided per reference.** A value satz knows at compile time — a
  param, a literal, an attribute satz writes, one derived from attributes satz writes (a
  service account's `email`) — is a literal output. An attribute only the cloud knows (a
  folder's `name`, a project's `number`) is a `data` source that reads the resource back
  by the natural key satz writes on it; a key that is itself known only to the cloud
  chains to that resource's lookup. The table is `presets/interface-lookups.yaml`.
- **The module is relocatable.** It references no file outside its directory, takes no
  variable, has no backend and reads no state; it declares its own `required_providers`
  with the version the estate pins. Copied, moved, or sourced by git URL, it works, and
  it runs through the consumer's provider and credentials.
- **Core exports come from satz's own packs.** `estate-core.satz` exports the day-0
  answers; `satz init` writes one export of the infrastructure folder into the estate.
- **Writes to shared infrastructure follow one of two protocols.** *Attach*: the write is
  an attachment resource in the consumer's own state, and satz never declares that
  membership authoritatively. *Contribute*: where no attachment resource exists, or the
  change needs coordination, the change is an entry in the estate (a list param a pack's
  `contributes_<param>` or the operator fills, ADR 0051), applied by satz, and the result
  is exported. Declaring attach points and checking a consumer's HCL against them are the
  next steps; the language and emission here do not depend on them.
- **The change notice is a pack.** `interface-notice.satz` publishes `jsonencode` of the
  root module's `local.satz_interface` as an object in a bucket with a storage
  notification to a Pub/Sub topic; Terraform rewrites the object only when a value
  changes, so a message means an exported value changed.

Decisions the design left open, made here:

- **One syntax.** `description` follows the value as a keyword and a string, the way
  `role "…"` follows `suppress`. A block form (`export "x" { value = … description = … }`)
  would be a second spelling of the same statement.
- **An export that needs a lookup of a type with no row is refused at compile**, naming
  the type and the types the table reads back; a static export of any type needs no row.
  A reference that is not `type.label.attribute` of a `google_*` resource (`${var.x}`,
  `${local.y}`) is refused: the module has neither.
- **The lookup table is compiled into the binary.** What the interface emits is part of
  the emitter, and one binary emits one interface whatever preset directory it reads; a
  table read from `presets_dir` would make two checkouts of one satz emit different
  modules.
- **An object value is refused.** A list of scalars is published; a map would need a
  lookup per field and a type for the output, and no export needs one yet.
- **`hcl/interface/` is satz's.** `transpile` removes and rewrites the directory whole,
  and removes it and `outputs.tf` when the estate exports nothing.
- **One name is one output.** The same name with the same value and description from two
  files is one export; a different one is an error naming both files, the rule the fold
  applies to an address.
- **The notice's service agent comes from a data source in a trusted `hcl` block.** The
  Cloud Storage agent's address carries the project number, known only once the project
  exists; satz declares no `data` source of its own. The notification names its topic
  through the grant, and the object records the notification's id, so the apply orders
  grant → notification → object without `depends_on`, which Satz does not have.

## Options

**`terraform_remote_state` on satz's state.** *Rejected.* Every consumer needs read access
to the state bucket — which holds every attribute of every resource, secrets included —
and the backend configuration, and a consumer breaks when the state moves (`satz migrate
--mode cloud`). It publishes everything rather than what is declared.

**A contract object only (a JSON file in a bucket).** *Rejected as the interface; kept as
the notice.* Every consumer would parse it with `jsondecode(data.google_storage_bucket_object_content…)`,
the values carry no types, and lookups of computed ids would still be the consumer's.

**A module that `source`s satz's `hcl/` directly.** *Rejected.* A module call of the root
module creates its resources a second time in the consumer's state.

**Static values only.** *Rejected.* Folder and project numbers exist only once the
resources do; a consumer would copy them by hand after every recreation, which is the
problem this solves.

**A module with the lookups and no static values.** *Rejected.* A lookup costs an API call
and a read permission per plan for a value satz already knows.

## Consequences

- An estate that exports anything — every estate that uses `estate-core.satz`, every
  estate `satz init` writes — gains `hcl/outputs.tf` and `hcl/interface/`. Its resources
  do not change; its plan shows the new outputs. That is an emission change: a minor
  release.
- `presets/interface-lookups.yaml` is derived from the provider's data source schemas and
  maintained by hand; the rows the showcase uses are validated by the smoke matrix, the
  others by nothing (docs/housekeeping.md).
- A consumer's lookup needs read permission on what it reads; a consumer without it fails
  its own plan, and satz's state is not involved.
- A notice subscriber learns of a change only when the apply goes through the estate's
  state; a change made outside it is drift, which `report-compliance` reports.
