# 0036 — `plan` and `apply` enable the APIs the estate declares

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** v0.68.0

## Context

Every emitted provider block carries `user_project_override = true` and
`billing_project = <infrastructure project>`, so Google bills every call the provider
makes — reads included — to that one project and requires the API enabled there,
whatever scope the resource itself lives at.

`tofu apply` refreshes every resource in state before it creates anything. An API that
the estate declares as a `google_project_service` but the project has off therefore
stops the refresh, before the `google_project_service` that would enable it is ever
created. The compiler's ordering of a resource after the service that enables its API
([ADR 0023](0023-one-command-for-every-prerequisite.md)) orders creates inside one
apply; it cannot order the refresh that precedes them.

So an estate that adopts a pack, runs `satz update-prerequisites` and then applies gets
a refused apply: the declaration is in the estate, the API is still off, and the
declaration cannot be reached. The workaround was `gcloud services enable …` by hand, or
a first `tofu apply -target=google_project_service.<label>`.

`bootstrap` already enables the estate's declared services imperatively, for the same
reason, on day 0 — where there is no apply yet. What was missing is the same step on
day 40.

`satz plan` and `satz apply` run the tool in `hcl_dir` and were classified as calling no
Google API of satz's own: the token was `tofu`'s business.

## Considered options

1. **`update-prerequisites` enables the APIs it writes.** It is the command that knows
   the gap.
2. **The compile refuses**, or warns, until every declared API is on — a live check in
   `transpile`.
3. **A preflight in `plan` and `apply`** (and `transpile --plan/--apply`): read the
   declared services for the billed project out of the emitted HCL, ask Service Usage
   which are off, enable those, and stop before the tool where that is refused.

## Decision

Option 3. Before `plan` and `apply` start the tool, satz reads `providers.tf` for the
project the default provider bills to and the service account it impersonates, reads
`main.tf` for the `google_project_service` rows on that project, asks
`services:batchGet` which of them are off, `services:batchEnable`s those and waits for
the operation. It prints one line when there was nothing to do, and one line per API it
switched on.

It runs as the identity the emitted provider impersonates — the estate's IaC service
account, which the estate grants `roles/serviceusage.serviceUsageAdmin` for exactly this
resource type. `plan` and `apply` therefore bind an identity, from the artefact `tofu`
reads rather than from an estate file, because these two commands are given a directory
and no estate.

Where the enable cannot succeed — no permission, the Service Usage API itself off on
the billed project, an org policy — the run stops before `tofu`, prints the APIs and
prints the filled-in line that does it by hand:

```
gcloud services enable <api> <api> --project <infrastructure project>
```

`update-prerequisites` stays an estate edit and enables nothing. It prints the same
line for the APIs it adds, and carries it in its JSON as `enable_missing_apis`, so an
operator or an agent applying with `tofu` directly has it.

`--no-api-preflight` skips the whole preflight, calls nothing and enables nothing.

## Consequences

- An estate that adopted a pack plans and applies in one go, on any day, with no manual
  `gcloud` step and no `-target` first.
- An enable is a live change outside tofu's state. The declared `google_project_service`
  then records an API that is already on; its create is idempotent, so the plan that
  follows shows no change for it.
- `satz plan` now needs a working credential and a reachable Service Usage API where
  `tofu plan` alone needed neither from satz. That is what `--no-api-preflight` is for,
  and `cd hcl && tofu plan` remains.
- `plan` and `apply` move from "satz calls no Google API" to "as the estate's service
  account" in the identity table, and `transpile` follows for `--plan`/`--apply`.
- One `services:batchGet` per `plan` or `apply` — one round trip for an estate with
  twenty declared APIs, plus the token.
- No MCP tool runs `plan` or `apply`, so nothing of this reaches an agent that way; what
  an agent gets is the `gcloud` line in `satz_update_prerequisites`.
- Under ADR 0010 this release is a minor: the same estate can be refused where it used
  to reach the tool.

## Pros and cons of the options

### 1 · `update-prerequisites` enables what it writes

- **Good:** one command, the gap and the fix in the same place.
- **Bad:** it is an estate edit, run in an editor's rhythm and often not run at all —
  a pack picked up by `merge-presets` declares its APIs without it. The apply is where
  the API has to be on, and the apply is what has to hold the guarantee.
- **Bad:** it would make a purely offline command live, and every `--report-only` run a
  credential-holder.
- **Bad:** it still leaves the estate that declared its APIs a release ago and enabled
  them on another machine.

### 2 · The compile refuses until the APIs are on

- **Good:** nothing is enabled behind the operator's back.
- **Bad:** it makes every compile live, including the ones in an editor and in CI.
- **Bad:** it reports work rather than doing work satz is authorised for and the estate
  has already declared.

### 3 · A preflight in `plan` and `apply` *(chosen)*

- **Good:** it holds at the moment it matters, whichever route got there —
  `satz plan`, `satz apply`, `satz transpile --apply`.
- **Good:** it reads the emitted directory, so it acts on what `tofu` is about to run,
  not on a recompile of the estate that may differ from it.
- **Good:** the refusal path is the useful one: the operator gets the exact command
  instead of a provider error a thousand lines into a refresh.
- **Bad:** satz makes a live change before a `plan`, which is a verb that otherwise
  changes nothing. The change is one the estate declares and the next apply would make
  anyway, and every API it switches on is named.
- **Bad:** `plan` gains a credential requirement and a round trip.
