# 0019 — an import skips what the platform owns, and says so

- **Status:** accepted
- **Date:** 2026-09-14
- **Shipped in:** v0.56.11

## Context

A live sweep of an organisation returns everything Cloud Asset Inventory lists, and a
good part of it is Google's, not the customer's: the `_Default` and `_Required` log
sinks that exist on every organisation, folder and project and cannot be deleted; the
grants Google's service agents hold (`service-<n>@gcp-sa-….iam.gserviceaccount.com`,
`<n>@cloudservices.gserviceaccount.com`); the legacy convenience grants a bucket carries
for `projectEditor:`, `projectOwner:` and `projectViewer:`; the Compute Engine and App
Engine default service accounts, whose `account_id` starts with a digit and which the
provider therefore refuses to declare; and a project in `DELETE_REQUESTED`, which Cloud
Asset lists for thirty days after deletion. On the test organisation these were about
a third of the discovered file, and two of them broke the plan outright: the default
service accounts failed validation, and the deleted project — picked as the providers'
quota project — made every organisation-scoped read fail.

An import writes what the customer declares. Declaring the platform's own resources
puts the estate in charge of things it did not create and cannot change, and every
`tofu plan` after that reports them.

## Options

1. **Import everything and let the operator delete.** Honest and complete, and the
   default until now. The file is a third longer than the estate, the plan does not
   pass without hand edits, and the operator learns which lines are Google's by trial.
2. **A hard-coded list in the importer.** Compact, but invisible: an operator who
   wants a skipped resource has to read the source, and a pattern that is wrong for one
   organisation is wrong for all of them until a release.
3. **A `skip:` list on the import-config row, as glob patterns over the resource's
   own name or the grant's member, reported per pattern.** The rule is data beside
   the row it applies to, the report names the pattern that took each group, and the
   override is the same copy of the table `--import-config` already takes.
4. **A flag** (`--skip-platform-owned`) that applies such a list. One more switch to
   document and remember, for a decision the table already expresses per type.

## Decision

Option 3. `presets/import-config.yaml` carries `skip:` on the live-shape rows for
sinks, IAM members, bucket grants and service accounts; `satz import` skips a match
and lists it under `platform-owned, skip pattern <pattern>` with a count, per item
under `--verbose`. A project that is not `ACTIVE` is skipped by its state — that is
not a name pattern but a fact about the asset — and its children are reported as
"parent not imported". A row with a key the schema does not know is refused at load
(`deny_unknown_fields`), so a misspelt `skip:` never passes as an empty one.

The state shape does not apply `skip:`: a state holds only what Terraform already
manages, and a built-in sink in a state is the operator's declaration.

## Consequences

- The discovered estate is the customer's declarations; the plan after an import is
  imports and nothing else.
- What was skipped is one report block away, and one table edit away from being
  imported.
- The pattern list is a judgement about Google's conventions (service agents live in
  Google-owned projects named `gcp-sa-*`, `*-robot`, `*-system`), kept where it can be
  corrected without a release. A customer service account that happens to match a
  pattern is skipped and named; the fix is the table.
