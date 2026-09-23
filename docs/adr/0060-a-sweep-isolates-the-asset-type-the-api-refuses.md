# 0060 — a sweep isolates the asset type the API refuses and finishes without it

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

A live `satz import` asks Cloud Asset Inventory for the asset types its
import-config rows name, a hundred per `ListAssets` request — the quota counts
requests, so one type per request runs out of it long before `--all` is through.

The list of types satz may ask for, `presets/cai-asset-types.txt`, is DERIVED:
`scripts/update_import_config.py --cai-types` writes it from what the API served
the day it was run. Google retires asset types, so the list goes stale by nature,
not by mistake.

When one type in a request is one the API no longer serves, `ListAssets` answers
`INVALID_ARGUMENT` for the WHOLE request and names no type. Every fetch error
then aborted the run with nothing written, on the reasoning that an estate built
from a partial sweep would be short whole types in silence and the next plan
would propose to create them.

Measured on a live organisation with v0.77.0:
`apigee.googleapis.com/SecurityProfileV2` is in the list and the API no longer
serves it, so `satz import --all` wrote nothing at all — no estate, no report,
nothing to look at. The recovery was a manual loop: run, read the error, add one
`--exclude`, run again. The error names the batch, not the type, so the loop is
as long as the number of retired types in the table.

## Decision

**A request the API refuses for an asset type is asked again in halves until the
refusal is down to one type; that type is left out, and the run ends by naming
every type it could not ask for.**

1. **Halve and retry, on `INVALID_ARGUMENT` only** (`sweep_batch`,
   `src/discovery.rs`). A refused request is split in two and each half asked
   again, down to the single type the API does not serve. Isolating one type in a
   hundred costs about fourteen extra requests, against a hundred for asking each
   type on its own.
2. **Nothing is collected from a request that failed.** A request hands back all
   of its pages or none, so the assets of a refused request go with it and a type
   in a retried half is never imported twice.
3. **Anything that is not a refused type still ends the run.** A denied scope is
   answered at the first request, without halving — it is the credential's, not a
   type's, and asking a hundred more times would report a hundred unserved types
   and write an estate missing all of them. A transport error ends the run the
   same way. So does a request refused for EVERY type in it: a scope the API
   cannot read is `INVALID_ARGUMENT` as well, and after the halving it reads as
   the whole table having been retired at once.
4. **The run says what it could not ask for**, every run, never behind
   `--verbose`: each type, the Terraform rows that asked for it, what the API
   answered, and the two ways to stop asking — refresh the table with
   `scripts/update_import_config.py --cai-types`, or leave the rows out with
   `--exclude`. The list is on `Discovered`, so a caller that renders its own
   report has it too.

## Options

**Keep aborting the whole run.** *Rejected.* It is correct about the danger — an
estate short a type plans to create what exists — and wrong about the cost. The
input that triggers it is a stale derived file, which is the normal state of that
file, and the operator's only route back is a manual exclude-and-rerun loop
against an error that does not name the offending type. The danger is answered by
saying what is missing, which the abort never did either.

**Ask the API for every type separately.** *Rejected.* It isolates every failure
with no retry logic at all, and it multiplies the request count by a hundred
against a quota that counts requests. A `--all` sweep of the shipped table would
spend its minute quota before it reached the resources.

**Read the offending type out of the error message.** *Rejected.* The API names
none: the message says an asset type is not supported and stops. A parser for a
message the API is free to reword is a gate that fails silently the day it is
reworded.

**Probe the type list before the sweep.** *Rejected as the answer here.* It is
one request per type against the same quota, before any work is done, to learn
what the sweep learns anyway. `scripts/update_import_config.py --probe` already
does this deliberately, out of band, and the report now points at it.

**Drop the type quietly and write the estate.** *Rejected.* That is the silent
partial sweep the abort existed to prevent. The whole value of continuing is that
the run says what it is short of.

## Consequences

- A stale entry in `presets/cai-asset-types.txt` costs the types it names and
  nothing else. The 99 other types of its batch are imported.
- A refused type is reported, not refused-into-silence: the estate is written,
  and an operator who does not read the report has an estate short that type. The
  report is on stdout with the skipped summary, on every run, and the missing
  rows are named with the flag that acknowledges them.
- A sweep that meets a retired type makes about fourteen more requests than one
  that does not. A sweep that meets none is unchanged.
- The fail-fast rule stands for everything else: the run still ends on a denied
  scope, a broken connection or an API error of any other kind, with
  `fetch_refusal` naming the identity it ran as.
