# 0014 — a dry run is a generated twin, and it claims nothing

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.55.0

## Context

Nine CIS controls ship as opt-in extensions because enforcing them blind breaks running
workloads: Confidential Computing is limited to particular machine families, CMEK needs
keys and grants to exist first, Cloud SQL hardening cuts public-IP connectivity. So the
customer is asked a question — "do you want this?" — with nothing to answer it from.
"Will this break us?" had no answer short of trying it in production.

Google's org policy answers it. A policy carrying `dry_run_spec` instead of `spec` is
evaluated on every action: Google writes a violation to the audit log for each one it
WOULD have blocked, and blocks none of them. The count is the organisation's own answer.

The emitter already supported it — `src/emit_shared.rs` renders `spec` and
`dry_run_spec` identically — and no pack used it. What was missing was the convention:
how an estate asks for a dry run, and what a dry-run policy means to the compliance
plane.

## Considered options

**How an estate asks for one:**

1. **A tri-state param per control** — `off | dry-run | on`. Closest to how the decision
   actually feels, and it needs a param type the language does not have: params are
   bound values and `use … when` tests truthiness. A language change for one feature.
2. **A hand-written sibling fragment** beside each extension. No language change, and it
   duplicates every policy body — so an edit to `cloud-sql.satz` that misses
   `cloud-sql-dry-run.satz` leaves a dry run measuring a policy that is not the one that
   will be enforced. That failure is silent and produces numbers that look right.
3. **A GENERATED sibling fragment**, derived from the enforcing one, with a `--check`
   gate. Same shape as option 2 with drift made impossible, and it matches what this
   repository already does with everything derived: a script, else a gate, else a line
   on `housekeeping.md`.

**What a dry-run policy means to `require`:**

1. **It satisfies the claim** — it is the same control, after all. This is what the code
   did before this change, by accident: `collect_enforce` walked the whole resource body,
   found the `enforce = "TRUE"` inside `dry_run_spec`, and reported the policy as
   enforcing. An estate could measure a control and report it as met.
2. **It is a deviation** — the estate declares it does not meet the control. Wrong word:
   a deviation is a decision not to comply, and a dry run is the opposite, the step
   before complying.
3. **It claims nothing.** The twin carries no claim, the control reports unmet, and the
   dry-run policy is instrumentation rather than evidence.

## Decision

Option 3 in both cases.

`scripts/build_dry_run_fragments.py` derives `<x>-dry-run.satz` from `<x>.satz`: `spec`
becomes `dry_run_spec`, every `claim` is dropped, the pack name gains `_dry_run` and its
version tracks the source. `--check` is a smoke gate. A `spec { reset = true }` is left
alone — that declares a superseded legacy twin OFF, which is not a control being enforced
and has nothing to size.

Six extensions get a twin. Five do not, and the table in the script says why for each:
the legacy constraints have no dry-run form, Access Approval is not an org policy, and
the two on-by-default extensions have nothing to size before enforcing.

The manifest now reads `enforce` from `spec` alone and records `dry_run` separately, so a
dry-run-only policy is `PolicyEffect::Inert` under
[ADR 0013](0013-a-claim-asserts-what-its-witness-does.md). That is what makes the
convention self-enforcing: a claim over a dry-run policy is reported as contradicted, so
a twin that carried one would fail its own gate.

Asking for a dry run and enforcement of the same control at once is an error, not a
warning. There is no reading of "measure it and block it" the estate could have meant.
The check runs before the fold — which would otherwise refuse the same thing as two
disagreeing definitions of one address, naming files rather than the decision behind
them — and the fold's own message gained a line naming the pair, for an estate that uses
both fragments without their gates.

## Consequences

- Every breaking control that CAN be measured now has a way to be measured, and the
  question `satz interview` asks about it has an answer that is not a guess.
- The twins are generated: editing one by hand is reverted by the next run and caught by
  the gate in between. The enforcing fragment is the only place to make a change.
- **Reading the violations is not in satz.** They land in Cloud Logging, and satz has no
  Logging API client — live verification goes through Cloud Asset Inventory. The
  fragment header and the library page carry the log filter; a `report-compliance` column
  that counts them is the remaining half of this work.
- A dry run shows as an unmet control in `require`. That is the honest reading, and it
  also means a dry run cannot be left running quietly as if it were compliance: the gate
  fails while it runs, which is the same pressure that gets it promoted.
- `dry_run_spec` was never read by `live_enforcement` either, so a dry-run policy reports
  *unverifiable* in a live report rather than enforced. Correct, and blunt — telling a
  reader "this is measuring, and here is the count" needs the Logging half above.
