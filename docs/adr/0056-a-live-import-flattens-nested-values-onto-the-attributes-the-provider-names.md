# 0056 — a live import flattens nested values onto the attributes the provider names

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

Two Cloud Storage buckets created with `uniform_bucket_level_access = true` and
`public_access_prevention = "enforced"` were swept back by `satz import <scope>` without
either attribute, and without the lifecycle rule's `with_state`. The estate compiled, the
plan looked clean, and an apply of it would have switched uniform bucket-level access off
and public access prevention back to inherited on two live buckets. For a tool whose
output is compliance evidence that is the worst loss there is: the report said "25
attributes dropped — not in the provider schema", and the operator reads that as the API
talking about itself.

The cause is the shape of the two schemas. Cloud Asset Inventory hands back the API's own
representation, the Cloud Storage JSON API's `Bucket`:

```json
"iamConfiguration": {
  "uniformBucketLevelAccess": {"enabled": true, "lockedTime": "…"},
  "publicAccessPrevention": "enforced"
},
"billing": {"requesterPays": true},
"lifecycle": {"rule": [{"action": {…}, "condition": {"age": 365, "isLive": false}}]}
```

The provider flattens all of it: `uniform_bucket_level_access`, `public_access_prevention`
and `requester_pays` are attributes of `google_storage_bucket`, and `with_state` is an
attribute of `lifecycle_rule.condition`. `iamConfiguration` and `billing` are names the
provider schema does not carry, so the schema filter in `src/discovery.rs` dropped each
container whole — with everything under it.

satz had an answer for this and it was not reachable: `satz map-types` (`src/align.rs`)
derives the API→Terraform map from the API's Discovery Document, including exactly these
flattenings, into `type-map.yaml`. That file is generated per estate, needs the network,
and is in no estate by default. The shipped `import-config.yaml` carries two hand-written
`map:` entries in 4800 rows. So the default import — the one every brownfield customer
runs first — had no flattening at all.

`isLive` is worse than a missing map row: the GCS API states the fact as a boolean and the
provider as an enum (`LIVE` / `ARCHIVED` / `ANY`). The two fields share no name, so no
alignment of names, derived or hand-written, can ever produce it.

## Decision

**The import flattens nested values onto the attributes the provider names, reading the
data against the provider schema, and never picks between two that disagree.**

Three parts, all in `src/discovery.rs`:

1. **Flattening (`flatten_nested`).** For a key the provider schema does not name whose
   value is an object, every scalar or list leaf under it is a candidate for the attribute
   of this block whose name it carries — `publicAccessPrevention` → `public_access_prevention` —
   or, where the leaf is the `enabled` / `value` of an object with at most two fields, for
   the attribute the OBJECT is named after — `uniformBucketLevelAccess.enabled` →
   `uniform_bucket_level_access`. The candidate must fit the attribute's type, and an
   attribute that is a pure output takes nothing. These are the two rules `align` applies
   to the Discovery Document, applied to the data instead, so they need no generated file
   and no network.
2. **Translation (`translate_api_values`).** Where the API states a fact in other terms
   than the provider, the pair is written out, keyed by type and path. One row today:
   a bucket lifecycle condition's `is_live` is `with_state = "LIVE"` / `"ARCHIVED"`.
3. **Two classes of loss, reported apart.** `DropReason::Vocabulary` is a name the
   provider does not speak — counted per type, named with `--verbose`, as before.
   `DropReason::NotCarried` is an attribute the provider schema DOES name whose value the
   import could not place; it is printed per resource, with the attribute and the reason,
   on every run. Each row names the resource it came from, which the old
   `(type, key)` pair did not.

**Two fields claiming one attribute with different values carry neither**, and both are
named. Same for a nested field contradicting a value the data already states at the
provider's own spelling: the same-level value stands and the other is reported. satz
resolves an ambiguity by reporting it, the way `adopt` does with two candidate ids.

`force_destroy` has no counterpart in any API. It is Terraform's own switch, it can be
read from nothing, and no import will ever carry it; that is written in the README,
`docs/language.md` and `docs/workflows.md` rather than left looking like a loss.

The generated `type-map.yaml` now MERGES into the `map:` rows of `import-config.yaml`
instead of replacing them, with the hand-maintained rows winning. Replacing them meant a
`satz map-types` run silently undid a correction someone made in the table — the bucket's
`lifecycle.rule` → `lifecycle_rule` row among them.

## Options

**Ship `type-map.yaml` in the repository.** The generator exists; run it once, commit the
result. Rejected: it is derived from thousands of Discovery Documents, it goes stale with
every API change, and it would still not produce `with_state`. Worse, it hides the
question — an estate on a newer API silently loses what the committed file does not know,
and the failure is invisible.

**Hand-write `map:` rows for the types that lose attributes.** The fix for the reported
bucket is three lines of YAML. Rejected: it fixes the buckets somebody looked at. The
defect is in every type whose API nests what the provider flattens, and a table
maintained by whoever notices a loss is a table that documents the losses already
suffered.

**Make an unknown key an error.** The strictest reading of "never silently heal": refuse
the import until a human maps the key. Rejected: most of an API's vocabulary genuinely has
no Terraform counterpart (`kind`, `selfLink`, `etag`, `satisfiesPZS`), so every import
would refuse, and the operator would learn to pass the flag that switches it off.

**Report only, carry nothing.** Print what the schema names and the import dropped, and
let a person write it. Rejected as the whole answer: it is the right report and the wrong
default. The values are there, the provider takes them, and an estate that has to be
hand-patched after every sweep is not an import.

## Consequences

- An imported estate now carries attributes the previous release left out, so the same
  sweep produces a different file, and a plan over it changes where the flattening applies
  — which is the point: the plan no longer proposes to undo the hardening. The release is
  a MINOR by ADR 0010 (an input, run through the new binary, plans differently); it
  refuses nothing that compiled before.
- The flattening is a correspondence by name and type, not a guess about meaning. Where
  two names collide it carries nothing and says so, so a wrong carry needs an API that
  spells an unrelated field exactly as the provider spells one of the resource's
  attributes, at a matching type.
- Reported paths are per resource and per attribute, so the counts in the dropped-attribute
  summary are higher than before: what used to count one container (`iamConfiguration`)
  now counts the leaves under it that nothing took.
- `translate_api_values` is a table of one row. A second pair goes in it with a test that
  reads it from the asset data it comes from; the alternative — a general value-semantics
  layer — would be a large machine for a handful of pairs.
- The unit tests read `tests/assets/storage-bucket.json`, a Cloud Asset asset in the API's
  own shape. The live path cannot be smoke-tested offline; the fixture is where the real
  shape is recorded, and `nothing_the_schema_names_is_dropped_in_silence` walks it against
  the provider schema so a future filter change cannot quietly start dropping again.
