# 0101 — a delegation is two packs, and a workload estate has no prerequisites check for roles

- **Status:** accepted; amended by [ADR 0110](0110-the-delegation-base-creates-the-estate-account-user-group.md) — what every delegation needs, its estate account user group included, is created by the base `presets/delegation.satz`; what one workload needs is in its delegation pack
- **Date:** 2026-11-21

## Context

satz checked a workload estate's roles against what the organization's estate delegated
([ADR 0087](0087-a-workload-estate-is-judged-against-what-was-delivered-to-its-account.md)).
It knew what was delegated only by five export names of the delegation presets:
`delegated_folder_roles`, `delegated_project_roles`, `delegated_billing_roles`,
`delegated_apis` and `delegated_budgets`. The README of an interface folder and the hints of
`satz init --from` picked rows and reads by the same names (`delegated_folder`,
`project_id`, `estate_account`, …). So every delegation had to be one of the shapes the
presets publish, and a workload with needs of its own — a product catalog whose account needs
other roles on a project the organization creates for it — had either to fit those export
names or be checked against nothing it was given.

A delegation has two sides. The organization's estate creates what a workload is given — a
folder, a project — and grants the delegation's estate account its roles on it; the
parameters are agreed in a change process outside satz, and the pack's author states the
intent the organization's team reviews. The workload estate reads what was given from the
interface folder — static values as text, plan-time values looked up from Google by its
own plan — and builds inside it.

## Options

1. **One pack with two halves.** A single pack declares what the organization creates and
   what the workload builds, and each estate takes its half. Cost: the language needs a way
   to say which half a block belongs to; a pack the organization reviews also carries the
   workload's resources, so every change of the workload is a change the organization's
   team reviews; satz still has to know which exports carry the grant to check it.
2. **Two packs in two estates.** A delegation pack in the organization's estate creates
   what is given and grants the roles; a workload pack in the workload estate builds inside
   it. Whoever creates a resource keeps it in its state. The workload estate checks no role.
   Cost: a missing role is found at the apply, as Google's refusal, not at the transpile;
   the workload's team learns what it needs from a list it gives the organization's team.
3. **One estate with two packs and one state.** Both packs in the organization's estate.
   Cost: the workload's account would need to write the organization's state, which is
   read access to everything the organization declares, and the organization's team
   applies every workload change.

## Decision

Option 2.

- **A delegation is two packs in two estates.** The delegation pack stands in the
  organization's estate: it creates what one kind of workload is given and grants the
  delegation's estate account its roles on exactly that. `presets/delegated-folders.satz`
  and `presets/delegated-projects.satz` are the ready-made delegation packs. The workload
  pack stands in the workload estate and builds inside what the delegation pack created,
  as the delegation's estate account. A workload with needs of its own ships its own pair.
- **Whoever creates a resource keeps it in its state.** What the delegation pack creates,
  and its grants, live in the organization's state; creating, deleting and granting them
  stay there. A resource the delegation pack created is never imported into the workload's
  state: one resource in two states is changed and deleted by both.
- **A workload estate has no prerequisites check for roles.** An estate that uses an
  interface file gets no role finding and no `update-prerequisites` refusal; `satz
  update-prerequisites` lists, as information for the organization's team, the roles its
  resource types need by where each is granted (`needed_roles`: project, folder,
  organization, billing account) and the APIs they need on the delegation's infrastructure
  project (`needed_apis`). A role that is missing is Google's refusal at the apply. The API
  check of the projects the workload estate declares itself stays. The organization's
  estate and the bootstrap keep the full check.
- **satz names no export.** The README of an interface folder lists each of the
  interface's own exports with its value — the literal, or "looked up" — and its
  description; `satz init --from` hints a parent from the resource type an export's
  `targets` name (`google_folder` → `folder_id`, `google_project` → `project`). The
  delegation's account is still recognized by its three exports
  `infrastructure_project_id`, `estate_account` and `state_bucket`: it is what the estate
  runs as, not something it builds in, and has no resource type of its own to be
  recognized by.
- **The interface folder is unchanged.** It already gives exactly what is meant for the
  workload; reading the organization's state directly would give the workload's account
  read access to the whole of it.

## Consequences

- The report of `update-prerequisites` drops `delegation`, `request` and
  `not_delegated_apis`, and `granted.own`; it gains `workload`, `needed_roles` and
  `needed_apis`. For a workload estate `missing` and `write` are empty.
- The finding "the account … was not delegated roles …", the finding of an API switched on
  twice, the refusal of an interface written before the exports were renamed (`delivered_…`)
  and the refusal of `apply` for an API missing on the delegation's infrastructure project
  are gone.
- `whoami <estate>` on a workload estate tests the billing account live and leaves the
  folders and projects untested, as before; the skipped line points at the list.
- The `delegated_*` exports stay: they tell the workload's team what the account holds,
  and the team compares them with the list.
- satz-studio's Overview shows the list for a workload estate instead of missing roles.
