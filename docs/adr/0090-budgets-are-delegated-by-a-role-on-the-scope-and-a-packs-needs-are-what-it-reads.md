# 0090 — budgets are delegated by a role on the scope, and a pack's needs are what it reads

- **Status:** accepted
- **Date:** 2026-11-12

## Context

Two things came up when a workload pack was rebuilt to fill a project it is given instead
of creating one (delegated access, ADR 0086, 0088, 0089).

**Budgets.** Many delegated workloads want a budget, and their teams ask for one. A budget
is a resource of the billing account, and the roles that manage budgets there —
`roles/billing.costsManager`, `roles/billing.admin` — cover the whole account: every
project's budgets and costs. Delegated access is the opposite of that. Google has a second
path: `billing.resourcebudgets.read` and `billing.resourcebudgets.write`, held on a
project, let a principal with no right on the billing account manage a budget for that
project, which Google scopes to it; `billing.resourceCosts.get` shows that project's
costs. The only predefined roles that carry them are Owner and Editor. All three are
supported in custom roles.

**What a pack needs.** A workload pack that fills a delegated project is of no use in an
estate that was given a folder, and the other way round. Nothing said which is which:
`satz packs` listed a name, a version and a state, and satz-studio offered every pack to
every estate.

## Options

Budgets:

1. **A billing role in the workload's entry** (`billing_roles`, as since ADR 0086).
   - Cons: account-wide. A team sees and changes what is not its own.
2. **The organisation's estate creates each budget** (`presets/organization-budget.satz`).
   - Pros: nothing is delegated.
   - Cons: every budget and every change of one is a request to the organisation.
3. **A custom role with the three project-level permissions, granted on the delegated
   scope to the entries that ask.**
   - Pros: the team manages its own budget and nothing else; one switch per entry.
   - Cons: a custom role to keep; whether the Budget API honours these permissions as the
     console does is not something satz can show without a live organisation.

A pack's needs:

1. **A statement in the pack** (`requires project`).
   - Pros: readable in the file.
   - Cons: a language form, its grammar and its parsers in three repositories, and a pack
     may state one thing and read another.
2. **Derived from what the pack reads.** A pack that reads `${{interface.delegated_folder}}`
   needs a folder; one that reads `${{interface.project_id}}` or
   `${{interface.project_number}}` needs a project.
   - Pros: no new form, no second graph of dependencies, and it cannot be wrong: a pack
     that does not read the value does not need it.
   - Cons: a pack must read the value in its own text — as a param's default, usually —
     for its need to show. A pack written against a literal says nothing.

## Decision

Budgets: option 3. `presets/delegated-budget-role.satz` declares the role once for the
organisation. An entry of `project-workload.satz` or a folder entry of
`folder-workload.satz` that says `budgets = true` is granted it on that project or that
folder; the grant names the role by reference, so an entry that asks while the role's pack
is off is refused by the compile. The interface publishes `delegated_budgets`, and the
prerequisite check takes a budget's need as met by it. The budget itself is a workload
pack, `packs/project-budget.satz`.

A pack's needs: option 2. `satz packs` reports `needs` per pack, and the presets report
says what the estate `works_in` — the organisation, a delegated folder (an interface it
uses publishes `delegated_folder`) or a delegated project. If a pack ever needs a scope it
has no reason to read, this is revisited before any statement is added.

## Consequences

- A team with `budgets = true` creates and changes budgets for its own scope. It holds
  nothing on the billing account.
- `billing_roles` stays for the cases that do need the account.
- Not verified live: a budget created through the Budget API with these permissions alone.
  Google documents the path for the console.
- A pack whose params default to the interface's values shows its need and needs no
  binding in the estate; `packs/workload-project.satz` and `packs/project-budget.satz` are
  written that way.
- A pack that reads the interface does not compile in an estate that uses none, so
  `satz review-pack` folds such a pack into its made-up estate together with a stand-in
  interface that holds every value the pack reads, as example values.
- Two defects fixed with this work: the API preflight of `plan` and `apply` resolves a
  project named through the interface, and a delegated estate that declares an API the
  organisation already switched on is told.
