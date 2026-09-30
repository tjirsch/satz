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
