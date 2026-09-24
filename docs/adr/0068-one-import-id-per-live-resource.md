# 0068 — one import id per live resource, whichever route writes it

- **Status:** accepted; replaces the per-type id rules of ADR-0065
- **Date:** 2026-09-24
- **Shipped in:** the release that follows

## Context

A live import derives a provider import id for a resource in three places:

1. **the mapped route** (`discover_generic_resource`) wrote `"import-id"`: the
   row's `import_id` template rendered from the resource's values, falling back
   to the asset path with the project number replaced by the id;
2. **`--generate-unmapped`** (`generate_config::import_id`) wrote the `import`
   block: the raw asset path, plus three hard-coded rules (instance settings, DNS
   zone, DNS record set) — ignoring the row's template and the project rewrite;
3. **`--into`'s subtraction** (`delta::undeclared`) compared the raw asset path
   with the ids the estate declares.

They disagreed. A DNS zone the estate declares by name was generated a second
time under `--into --generate-unmapped`, because (3) compared the path Cloud Asset
names by the zone's NUMBER. A mapped record set got the numeric path from (1),
which the provider does not import by. An unmapped log metric got
`projects/<n>/metrics/<name>` from (2) where the provider imports `{project} {name}`.

## Decision

**One function, `discovery::import_id(tf_type, full_name, template, values,
asset_names) -> Result<id>`, is the id on every route.**

1. The relative resource name, with each segment Cloud Asset names by a number
   and the provider by a name rewritten from the sweep's own reading: a DNS
   managed zone's number → its `name`, a project number → its `projectId`
   (`NAMED_BY_DATA`, a table of asset type, collection and data key). A zone the
   sweep did not read is a refusal; a project it did not read keeps its number,
   which the provider imports by as well.
2. `google_compute_instance_settings` drops the `/InstanceSettings` Cloud Asset
   appends — the one rewrite the template language cannot state, so it lives in
   the function.
3. The row's `import_id` template, where the row has one, IS the id: each
   placeholder from the path segment standing where the template puts it, else
   `{project}` from the path, else the resource's own attribute of that name. A
   placeholder nothing fills refuses the resource, naming it.

The DNS and instance-settings ids move into `presets/import-config.yaml` as
templates (`google_dns_record_set`, `google_compute_instance_settings`), where
`satz adopt` reads them too. A skipped resource reaches the function through
`Discovered::skipped_import_id` — its asset name, its row's template (the sweep
keeps `import_templates`) and no values, since nothing of its data was carried.
A grant's id reads the same table: `grant_import_id` writes the scope the way the
row's template writes it (`b/{bucket}`), instead of hard-coding the `b/` prefix and
an organisation-prefix strip.

## Options

**Three derivations kept, a test holding them equal.** *Rejected.* The test would
have to enumerate the types; the disagreement was found by review, not by a test,
because nobody knew there were three.

**Template-less: the asset path everywhere, per-type rules for the exceptions.**
*Rejected.* The template is the provider's own import format and `adopt` already
renders it; a second encoding of the same fact per type is what drifted.

**Fall back to the asset path when a template does not render** (what route 1
did). *Rejected.* A path the row says the provider does not import by is a guess;
the resource is refused with the placeholder named, and `--generate-unmapped`
lists it.

## Consequences

- `--into --generate-unmapped` no longer generates a resource the estate declares
  by the name Cloud Asset gives a number to.
- A mapped resource whose row's template cannot be rendered from its data is
  skipped as unmapped, naming the placeholder, where it used to be written with
  the asset path.
- An unmapped resource whose template needs an attribute the path does not state
  (`{project} {name}`) is refused by `--generate-unmapped` rather than asked for
  under an id the provider refuses.
- A new singleton or a new number-for-name collection is a template or a
  `NAMED_BY_DATA` row, not a new branch in three places.
