# 0058 — the asset's own name picks its Terraform type, and several still fitting is reported

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

`presets/import-config.yaml` maps Terraform types to Cloud Asset types, and the
mapping is many-to-one: `logging.googleapis.com/LogSink` is named by four rows —
`google_logging_project_sink`, `_folder_sink`, `_organization_sink`,
`_billing_account_sink` — `logging.googleapis.com/LogBucket` by four
`*_bucket_config` rows, `storage.googleapis.com/Bucket` by the bucket and its IAM
policy. Thirty-two asset types in the shipped table are claimed by more than one
Terraform type.

The live sweep took the FIRST row that fitted, out of a `HashMap`, with a partial
scope test bolted on: a type containing `_project_`, `_folder_` or
`_organization_` had to match the asset's scope, and any other type matched
anything. `google_logging_billing_account_sink` contains none of the three, so it
matched everything.

Measured on a live organisation with v0.77.0: under `--all`, every log sink of
the organisation — project, folder and organization alike, the estate's own audit
sinks among them — was mapped to `google_logging_billing_account_sink` and then
dropped as `unmapped: required billing_account is not in the asset data`. Every
log bucket went the same way. The default run, where the billing-account rows are
off, mapped the same sinks correctly — so `--all`, which only switches rows ON,
lost resources the default kept. And because the candidates came out of a hash
map, which row won was not even stable between runs.

## Decision

**Which row an asset is mapped through is decided by the asset, in one place
(`row_for_asset`, `src/discovery.rs`), and where the asset does not decide, the
resource is reported rather than guessed.**

1. **The content type first.** An asset carrying the resource is mapped through a
   `content_type: RESOURCE` row, one carrying an IAM policy through an
   `IAM_POLICY` row. This alone resolves the bucket/grant families.
2. **Then the PARENT, read off the asset's own name** — `projects/…`,
   `folders/…`, `organizations/…`, `billingAccounts/…` — compared with the parent
   each candidate type is for, which the provider spells into the type name. A
   type that names no parent serves any, and is taken only where no type that
   names one fits. `billingAccounts/` is a parent Cloud Asset names and the estate
   has no node for: it picks the billing-account type, and placing the resource
   then fails with that reason rather than with a wrong parent.
3. **Several rows still fitting is `SkipReason::Ambiguous`** — reported with the
   rows named and `--only <type>` as the lever, never resolved by order. No row
   fitting because every enabled one is for another parent is reported with that
   reason too.
4. **One selector, three callers.** The discovery statistics, the folder/project
   pass and the resource pass all read it, so the counts printed during the sweep
   are the rows the estate is then built from.

## Options

**Add a `parent:` column to the table and match on it.** *Rejected.* It is 841
rows of data that would have to be filled, kept true by hand and checked by
something, to restate what the provider already spells into every type name —
`google_logging_folder_sink` is for a folder in the only vocabulary this project
has, the provider's own (`CLAUDE.md`: the types are the provider's, to the
underscore). A derived answer that is wrong is a bug in one function; a hand-kept
column that is wrong is a bug in a file nobody re-reads.

**Order the table and document that the first row wins.** *Rejected.* The
selection would then depend on a file's line order, which
`scripts/update_import_config.py` rewrites, and on a map that does not preserve
it. It also answers the wrong question: order says which row is preferred in
general, and the question is which row fits THIS asset.

**Pick a default row per asset type where the parent does not decide.**
*Rejected.* For the families this affects — four types for
`compute.googleapis.com/Router`, two for `run.googleapis.com/Service`, six for
`securitycenter.googleapis.com/BigQueryExport` — the right answer depends on what
the asset is, which the asset type does not say. A default here is the guess
`satz adopt` is built to refuse, and its output is a resource block that plans as
the wrong resource. Reported, the operator sees the live resource, the candidate
types and the flag that picks one; `--generate-unmapped` lists it too.

## Consequences

- `--all` imports log sinks and log buckets under all four parents, and is now a
  superset of the default run for them. A test holds that: nothing the default
  imports may be lost when `--all` switches rows on.
- `--all` no longer imports the families where the provider has several types for
  one asset type and nothing distinguishes them — a Compute router, a Cloud Run
  service, an App Engine version. They were being imported as whichever type the
  hash map offered first, which was right about half the time; they are now
  reported per resource with the candidates named. `--only google_compute_router`
  imports them.
- The rule reads the provider's naming convention, so a provider type that breaks
  it (a parent in the name that is not the asset's parent) would be mis-scoped.
  The convention holds across every family in the shipped table, and a break shows
  up as a reported skip, not as a silently wrong resource.
