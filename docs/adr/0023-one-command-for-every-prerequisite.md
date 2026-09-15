# 0023 — one command for every prerequisite, and the compiler orders the services

- **Status:** accepted
- **Date:** 2026-09-15
- **Shipped in:** v0.58.0
- **Extends:** [0009](0009-iac-service-account-named-roles.md) (the IaC service account holds named roles)

## Context

A resource type obliges the estate to declare two things before it can be applied:
the ROLES the IaC service account needs to create it, and the API that serves it. satz
knew the first and enforced it — `iac-roles` reported the gap, a compile warned about
it, a gate held the table complete. It knew nothing about the second.

The failure that made that visible: a customer apply stopped, repeatedly, because
`tofu` created resources before the APIs they need were on. Two halves behind it.

- `google_project_service` carries no ordering of its own, so inside one apply
  Terraform can create a resource before the service that enables it. `bootstrap`
  dodges the race by enabling a hardcoded eleven imperatively before `tofu` runs — but
  a pack adopted on day 40 has no bootstrap pass.
- Nothing said an API was missing at all. `presets/organization-budget.satz` emits
  `google_billing_budget` and declares no service, so `billingbudgets.googleapis.com`
  was never enabled; `monitoring/organization-cis-log-alerts-central` needs
  `monitoring.googleapis.com` and is a default-on map choice. The error arrives from
  `tofu`, naming an API nobody mentioned, at the end of a plan that read clean.

## Options

1. **Add `satz iac-apis` beside `iac-roles`.** Cheapest, and nothing is renamed. It is
   also the path that ends in five commands for one reason: every future prerequisite
   class earns another verb, and the operator has to know which one to run when.
2. **Fold the check into `merge-presets` only.** It fires exactly when the library
   changes, which is when most gaps appear — but a resource the estate declares itself
   is never covered, and an operator who has not run `merge-presets` has no command.
3. **One command for every prerequisite, which writes.** `update-prerequisites`
   replaces `iac-roles` and covers roles, APIs and whatever comes next, with
   `merge-presets` running the same check at the end of a pickup.

## Decision

Option 3 (Thomas, 2026-09-15).

**One command, and it writes.** `satz update-prerequisites <estate>` derives both
halves from the types the estate emits and writes what is missing into the estate file
— the roles into the IaC service account's grant list, the APIs into the infrastructure
project's `project_service` list. Writing is the point: those declarations have to be
there either way, and a command that only reports leaves the work undone. Both halves
are written before either is verified, and a gap that survives, or an estate that stops
compiling, restores the file — so the estate is never half-edited. `--report-only` is
the escape hatch for an engagement where satz may not grant those roles itself: it
lists the gap, writes nothing, and exits non-zero. `merge-presets` runs the same check
and write at the end of a pickup and reports what it added.

**The compiler orders the services.** Every emitted resource waits for the
`google_project_service` blocks that enable its APIs, bounded to two projects: its own,
and the infrastructure project every provider call is billed to. No pack author writes
a `depends_on`, and nothing can forget one. The rule that keeps the graph acyclic is
the service block's own reference closure, not a list of types — the infra project's
services reference it, and the folder above it is reached through the project, so both
would otherwise wait for a service that waits for them.

**The API is judged on the project the call is BILLED to.** Every provider block
carries `user_project_override` with `billing_project = infra_project_name`, so Google
requires the service enabled on the infrastructure project whatever the resource's own
scope is — a budget hangs off the billing account, an org policy off the organisation,
and both still need their API there. A pack that enables an API on a project of its own
has answered a different question, for its own calls. Satz derives the infrastructure
project's side; the pack owns its own projects' side, and satz does not second-guess a
pack author who forgot (deferred, not denied).

**The knowledge is one compiled-in table with one gate.** `src/prerequisites.rs` carries
one row per resource type with both halves, and the gate that held the roles holds the
APIs. A second test cross-checks every API against the asset type
`presets/import-config.yaml` carries for it — Cloud Asset namespaces each asset by its
service, which makes 58 of the 59 rows checkable against data refreshed from Google.

**An apply refuses; a plan warns.** A missing API reads like a missing role at compile
time — warning by default, error under `--validation error`, silent under `none` — but
`transpile --apply` and `bootstrap` refuse, the same two-speed rule the
unanswered-question gate already uses. An apply that dies halfway on a missing API
leaves a half-built estate, and the fix is one command away.

**`bootstrap` enables what the estate declares.** Its five non-required APIs were a
hardcoded list; they are now the infrastructure project's declared services. A
hardcoded list is missing exactly what a pack brings with it.

## Consequences

- A MINOR release: `iac-roles` is gone, `--execute` with it, and the MCP tool is
  `satz_update_prerequisites` whose default writes and therefore needs `write`.
  A scripted caller that wants the old read-only behaviour passes `--report-only`.
- `Kind::IacRoles` is `Kind::Prerequisites` in the findings an agent or an editor reads.
- Every resource in a project now waits for up to two enablement resources, so an apply
  is more sequential — and a destroy takes the services down after the resources that
  needed them, which is the right direction.
- One name to remember for the whole class, and a place for the next prerequisite to
  land without another verb.
