# 0065 — a refused import id costs one resource, not the run

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

`satz import <scope> --generate-unmapped` hands the live resources satz's own mapping
left unmatched to the provider: one `import` block each in a scratch directory,
`tofu init`, `tofu plan -generate-config-out=generated.tf`, and the `.tf` the provider
writes goes back through the hcl import arm into `<base>-generated.satz`
([ADR 0052](0052-the-generate-config-fallback-is-a-flag-on-the-live-import-and-writes-a-second-file.md)).

Measured against a live organisation: 52 import blocks, `Plan: 26 to import`, and the
child exited non-zero. `generate` treated every non-zero exit as the end of the run, so
satz read nothing back and wrote no Satz file — while `generated.tf` sat in the scratch
directory with 34 of the 52 resources in it. The operator was handed two commands to run
by hand instead of a file with 34 resources in it.

Three things went wrong at once, and they are three different questions.

**The ids.** The import id was the asset's relative resource name for every type. That
is the provider's id for most of them and not for all. Eighteen were refused: DNS record
sets carried the managed zone's numeric id where the provider imports by the zone's
NAME, and a `google_compute_instance_settings` carried the trailing `/InstanceSettings`
Cloud Asset appends to a singleton, which the provider's id pattern does not accept. (The
log sinks and log buckets in the same run were a different defect —
`google_logging_billing_account_sink` chosen for a sink under a folder — and
[ADR 0058](0058-the-asset-name-picks-the-terraform-type.md) had already fixed it by
picking the type from the asset name's parent segment.)

**The child's provider block.** It carried `impersonate_service_account` and nothing
else. The estate the same run writes gets `project`, `billing_project` and
`user_project_override` in its own `providers` block, because organization-,
folder- and billing-account-scoped calls name no project of their own and Google needs
one for API enablement and quota. The child was reading the same organisation with no
project to bill.

**The exit code.** `Plan: 26 to import` and a non-zero exit are not a contradiction: the
provider reads each resource in turn, writes the configuration for the ones it could
read, and reports the rest. The exit code says "something was reported", never "nothing
was produced".

## Considered options

For the partial result:

1. **Keep what the provider generated and report the rest per resource**, in the
   provider's own words.
2. **Keep the current all-or-nothing**: any non-zero exit discards the file.
3. **Keep the file only when every candidate was generated for** — which is option 2
   under another name, since the file is complete exactly when the child exits zero.

For the ids:

- **A per-type rule in code** for the types where the provider's id is not the asset's
  relative resource name.
- **A shape check against the table's `import_id` template**, refusing any derived id
  the template contradicts.
- **A generic rule** — strip a trailing segment that repeats the asset type's kind.

## Decision

Option 1, with a per-type rule for the ids and no generic id check.

`generate` returns the file the provider wrote together with what the child said, and
`outcomes` says per candidate which of four things happened, deciding from the FILE and
the message rather than from the exit code:

- **written** — a `resource` block for its address is in `generated.tf` and no
  diagnostic names it;
- **incomplete** — a block is in the file AND a diagnostic names it: generated, not
  usable as it stands, with the provider's words saying what is missing;
- **refused** — no block, and a diagnostic names it;
- **unaccounted for** — no block and no diagnostic. satz cannot say why, and says that.

A diagnostic that names no candidate is printed whole. `imports.tf` stays on disk with
every block in it, and the run repeats the two commands that finish the refused ones by
hand. `init` failing is still the end of the run — no resource was ever read — and so is
a plan that generated no resource at all, whether it wrote no file or an empty one.

The four verdicts are what keeps the rule "never pretend a resource was imported": the
run's closing statement is per resource, derived from what is on disk, and a resource
satz asked for that the provider neither wrote nor mentioned is named as such rather
than counted as either.

The id rules live in `import_id` (`src/generate_config.rs`), one arm per type, each
refusing by name when it cannot build the id from what the sweep holds:

- `google_compute_instance_settings` imports the collection path
  `projects/<p>/zones/<z>/instanceSettings`, without the `/InstanceSettings` Cloud Asset
  appends.
- `google_dns_managed_zone` and `google_dns_record_set` import by the zone's name. Cloud
  Asset names the zone by a number, and the zone asset's own data states the name, so
  the sweep keeps it (`Discovered::asset_names`, filled for the asset types of
  `NAMED_BY_DATA`) and both ids read it. A record set whose zone this sweep did not read
  is refused naming the asset type to sweep — the number is never sent to find out.

The child's provider block is the estate's own: the quota project
`import::quota_project_choice` picks — the first project of the sweep that enables the
Org Policy and Service Usage APIs, which is the project satz writes into the estate's
`providers` block — as `project` and `billing_project`, with `user_project_override`,
plus the impersonation when the run is bound to an estate. This is
[ADR 0059](0059-a-projects-provider-alias-is-the-estates-provider-scoped-to-that-project.md)'s
rule applied where it belongs: the default provider bills to the infrastructure project,
and the child IS the default provider of the estate the run just wrote.

## Consequences

- A run in which the provider refuses some ids now writes `<base>-generated.satz` with
  what it could read, and the operator reads one report to see what is in it. Before, it
  wrote nothing and pointed at the scratch directory.
- The generated file may hold a resource the provider reported — the **incomplete**
  verdict. `google_cloud_asset_organization_feed` is the example the rehearsal produced:
  `billing_project` is a REQUIRED argument on that resource that the Cloud Asset API does
  not return, so `-generate-config-out` omits it and the generated block does not
  validate. No provider block can supply it: it is the resource's argument, not the
  provider's. satz names the resource with the provider's message and the operator adds
  the attribute; `satz transpile` and `tofu plan` refuse it in the same words until they
  do.
- A wrong id now costs one line of report instead of the run, which is why the shape
  check against the table's `import_id` template was rejected. That check cannot judge a
  placeholder segment — `{parent}` and `{key_ring}` span several — so it would pass the
  DNS ids it was meant to catch while refusing correct `google_org_policy_policy` ids.
  With the blast radius down to one resource it would cost more than it saves.
  `template_segments` (`src/discovery.rs`,
  [ADR 0063](0063-an-import-refuses-rather-than-write-a-value-the-source-does-not-carry.md))
  matches a template against an asset path the same way and stops at the same wall: a
  template it cannot describe segment for segment binds nothing. It answers the other
  half of the question — which segment a MAPPED resource's required attribute comes from
  — and a resource that reaches `--generate-unmapped` is by definition one no row mapped,
  so there is nothing for it to bind.
- The generic "strip a trailing segment that repeats the kind" rule was rejected for the
  same reason in the other direction: it would silently truncate the id of any resource
  whose last segment happens to equal its kind, which is guessing.
- A new singleton or number-named type needs an arm in `import_id`. Until it gets one,
  its id is the relative resource name, the provider refuses it, and the report names it
  with the provider's reason — a defect that costs a line, not a run.
- `Discovered` carries `asset_names`. The state shape fills it empty: a state file names
  every resource by its Terraform address, so there is no Cloud Asset name to key by.

## Pros and cons of the options

### 1 · Keep what was generated, report the rest per resource *(chosen)*

- **Good:** the operator ends with the resources the provider could read and a
  per-resource account of the ones it could not, in the provider's own words.
- **Good:** the verdicts come from the file and the diagnostics, so they stay true when
  the child's exit code says only "something happened".
- **Bad:** the generated file can contain a block that does not validate. It is named as
  incomplete and the message says why, rather than being dropped along with the 33 good
  ones beside it.

### 2 · All-or-nothing on the exit code

- **Good:** one rule, no parsing of the child's output.
- **Bad:** measured: 34 usable configurations thrown away because 18 ids were wrong. The
  operator then runs the same two commands by hand and keeps exactly those 34.
- **Bad:** it makes the exit code mean "nothing was produced", which for a per-resource
  read is not what it means.

### 3 · Keep the file only when it is complete

- **Bad:** it is option 2 — the file is complete exactly when the child exits zero.

### A shape check of every derived id against the table's `import_id` template

- **Good:** it would gate a type nobody has looked at, rather than only the three the
  rehearsal named.
- **Bad:** a template placeholder can span several path segments, so the check either
  refuses correct ids or matches anything. It would not have caught the DNS ids.
- **Bad:** a false refusal costs a resource that would have imported, and with the
  partial result in place a wrong id costs one report line.
