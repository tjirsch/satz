# 0074 — a live import writes what the provider reads back, and leaves out what is not live

- **Status:** accepted; extends ADR-0058 (the asset name picks the Terraform type)
- **Date:** 2026-09-30
- **Shipped in:** the release that follows

## Context

A whole-organisation `satz import organizations/<n> --into <estate> --all` on a test
organisation planned "242 to import, 9 to add, 16 to change, 9 to destroy" and 17
errors. Every defect was one of three kinds:

1. **Something that is not there to import was written.** Folders in
   `DELETE_REQUESTED` with their grants and log buckets (11 × 404; only projects were
   checked for their state), resources inside a deleted project handed to
   `--generate-unmapped`, a switched-off service (Cloud Asset still listed it as
   `ENABLED`; the provider refused "Cannot import non-existent remote object"), a
   service account key (the provider has no import for it), a subnet's local route
   (it states `nextHopNetwork`, which no argument of `google_compute_route` sets).
2. **A value was written in the API's shape, not in the one the provider reads
   back.** Log bucket configs named their parent by a bare number where the state an
   import writes holds `folders/<n>` / `projects/<id>` (9 replacements); alert
   policies named notification channels by project number (8 in-place updates); a DNS
   zone's `visibility = "PRIVATE"`; an instance's `metadata { items }`,
   `tags { fingerprint }` and no `boot_disk` / `network_interface`; a disk's type as a
   URL, its image under `sourceImage`, and its `architecture`, which the provider
   leaves empty on import, so the value in the estate forces a replacement.
3. **Two routes disagreed about where a resource hangs.** Cloud Asset names a DNS
   record set under `projects/<number>/locations/global/managedZones/<id>` and its zone
   under `projects/<project id>/managedZones/<id>`; the lookup built the zone's key from
   the record set's prefix and found nothing, so both routes refused every record set.
   A project's network firewall policy and a hierarchical one share
   `compute.googleapis.com/FirewallPolicy`, and ADR-0058's parent rule, which reads
   the parent off the type's name, found neither type naming one.

## Decision

**The import writes a resource only when the provider can import it as it stands,
and writes each value in the form the provider reads back after the import.** What
is left out is listed with its reason; nothing is left out in silence.

- **Not live** (`SkipReason::NotLive`): a folder or project whose state is not
  ACTIVE, everything whose own scope or ancestors include one, a service Cloud Asset
  lists as not `ENABLED`, and a service Service Usage does not report enabled.
  Service Usage is asked once per project (`services:batchGet`) because Cloud Asset
  was measured listing a disabled service as `ENABLED`; a project whose services
  cannot be read is named, and its services are written as Cloud Asset lists them.
- **Platform-managed** (`SkipReason::PlatformManaged`, `PLATFORM_MANAGED` in
  `src/discovery.rs`): a resource whose data carries a field no argument of its type
  sets. One row: a route with `nextHopNetwork`.
- **Not importable** (`importable: false` on the import-config row): `--all` leaves
  the row off and a sweep that has it on leaves it out and names it. One row:
  `google_service_account_key`.
- **The provider's shape**, in `src/discovery.rs`, each a rule with a test that
  reads it from the asset data it comes from: a `{ fingerprint, items }` wrapper is
  the set or map the attribute is; a plural list whose singular is a block of the
  schema is that block; an instance's `disks` are `boot_disk` / `attached_disk`; a
  disk type self-link is its name; the renames a row's `map:` states (a disk's
  `sourceImage` → `image`, `sizeGb` → `size`); lower case where the API writes an
  enum upper case (`google_dns_managed_zone.visibility`); a parent as the resource
  name the provider holds (`PARENT_AS_RESOURCE_NAME`: the four log bucket configs);
  a reference by project number named by the project's id where the sweep read it;
  an attribute the provider does not read back on import and that forces a
  replacement left out and listed (`DropReason::NotReadBack`: a disk's
  `architecture`).
- **The container by its id, not its path**: a DNS zone is found by collection and
  number when the record set's path above it differs, and a template that names no
  location binds against a path without Cloud Asset's `locations/global`.
- **The schema is the next place the provider states a parent** (extends ADR-0058):
  where the asset's parent leaves several rows whose types name none, a type with a
  `project` argument and no parent of its own serves a project, and a type with a
  `parent`, `folder`, `org_id` or `organization` argument and no `project` serves an
  organisation or a folder. Still several is still ambiguous.

## Options

**Filter by Cloud Asset's own state only.** *Rejected for services.* It is what was
measured wrong: a service switched off hours before was still `ENABLED` in the
inventory, and the plan failed on it. Asking Service Usage costs one request per
project, the billing-account lookup beside it already costs one.

**Derive importability from the provider.** *Rejected for now.* The schema JSON does
not say whether a type imports; `tofu plan` does, per resource, which is the
`--generate-unmapped` round trip. A row column is data a person sets once, from the
provider's own refusal, and the default (absent = importable) needs no refresh.

**A column per type for the parent's form, or the lower-case enums, or the fields
not read back.** *Rejected as columns.* Each is a handful of rows measured against a
live plan; as code tables beside `translate_api_values` they are read by the same
tests that measured them, and `scripts/update_import_config.py`, which rewrites the
table, cannot drop them.

**Read enums and parent forms off the schema's description text.** *Rejected*, as
in ADR-0067: prose is no contract.

**Leave the instance's metadata out**, since Cloud Asset states the effective,
case-normalised metadata (`"true"` for `"TRUE"`, a project-level key merged in).
*Rejected.* Left out, the plan removes every key; carried, the plan updates to values
the instance already takes as the same. The difference is named here, not hidden.

## Consequences

- On the test organisation the same sweep plans imports only, apart from the
  provider's own attribution label, the labels the provider does not read back into
  `labels` on import (a Pub/Sub topic's), the effective instance metadata above and a
  notification channel label Cloud Asset reports and the API read does not; and a
  403 on log buckets in projects whose Logging API is off, which ADR-0059's
  per-project billing alias meets and which is a decision of its own.
- Every PR that finds another such value adds a row to one of these tables, with the
  test that measured it.
- A resource whose data states no name is keyed by its asset name's last segment,
  a singleton by what it is the singleton of, instead of by the whole asset name.

## Amendment — an API a project has off, and the project the reads bill to

- **Date:** 2026-09-30

### Context

After the decision above, the same sweep on the test organisation planned 233 imports and
4 errors: the `_Default` and `_Required` log bucket configs of two projects whose Logging
API is off. They are written inside their project's node, so the project's own provider
alias serves them and bills their calls to that project
([ADR 0059](0059-a-projects-provider-alias-is-the-estates-provider-scoped-to-that-project.md));
Google refuses the read there. The import had no rule for an API that is off where the
resources it writes need it, and it billed its own reads to whatever quota project the
caller's credentials named.

### Decision

An API used by resources inside a workload project is switched on in that project and
used through that project's own provider. Everything not tied to one project goes
through the infrastructure project, which also switches the APIs on.

- **The import's own reads.** Given an estate (`--into`, `--as`), every read — the
  sweep, Service Usage, Cloud Billing, Resource Manager, the lookups `--into` resolves
  the estate with — is billed to the estate's `infra_project_name`
  (`gcp::bill_reads_to`, which `resolve_quota_project` answers first). Once the scope
  is checked against the estate's organisation, and before the sweep, any of the APIs those reads call (`import::READ_APIS`) that is off there is
  switched on; the infrastructure project is satz's own. The import switches nothing on
  in a workload project. Without an estate the reads stay on the caller's quota project.
- **An API a project has off.** The Service Usage request that confirms a project's
  services also asks for every API the resources written inside the project need
  (`prerequisites::apis_for` of each type in the project's node, its own
  `project_service` entries excepted, which the provider around the project serves).
  One that is off is added to the project's `project_service` list with the import id
  it has once it is on (`<project>/<api>`) and named in the report. Under `--into`, an
  API the estate declares on that project already is left to the estate.
- **Switching it on.** The `plan`/`apply` preflight
  ([ADR 0072](0072-an-api-is-judged-on-the-project-its-provider-bills-to.md)) checks an
  adopted project — its `google_project` has an `import` block — and switches the
  declared API on before `tofu` starts, with the Service Usage calls billed to the
  infrastructure project. The plan then imports the service with the rest.
- **The collision default is unchanged.** A grant one principal holds on two nodes
  still stops the import and names `--on-collision counter`.

### Options

**Write the added API without an import id.** *Rejected.* The plan would show a create
per added service — a no-op against a service the preflight has just switched on, but
an "add" in a plan that is otherwise imports only, and the next `--into` would find
the service live under an id the estate does not carry. With the id, the plan imports
it. The cost: a `tofu plan` run directly, without `satz plan`, fails on that import
until the API is on — and it failed on the resources' refresh before, so it needs the
`gcloud services enable` line either way.

**Switch the API on during the import.** *Rejected.* The import reads; changing a
workload project is an apply-time act, and `satz plan` already does it as the estate's
identity, where the estate declares it.

**Leave the resources out, as not live.** *Rejected.* They are live; only the API that
reads them is off, and the estate's own list is where an API is switched on.

**Derive the needed APIs from the emitted `providers.tf`**, as ADR 0072's check does.
*Not needed here.* The import writes the tree whose position decides the provider, and
the compile after it runs ADR 0072's check against `providers.tf`, which finds a gap
this rule missed.

**Keep the reads on the caller's quota project.** *Rejected.* That project is a
setting of the person's `gcloud`, which may be in another organisation or have the
APIs off; the infrastructure project is the estate's own and already the one its
providers bill to.

### Consequences

- On the test organisation the same sweep plans 240 imports, 8 in-place changes (the
  ones listed above) and no error; three APIs were added — Logging on two projects and
  Resource Manager on one, whose project grants need it.
- A run given an estate may switch an API on in the estate's infrastructure project,
  and says so.
- `serviceusage::service_states`, `batch_enable` and `billing::project_billing_account`
  take the project their call bills to.
