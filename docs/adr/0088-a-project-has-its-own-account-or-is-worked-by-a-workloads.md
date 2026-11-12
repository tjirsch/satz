# 0088 — a project has its own account, or is worked by a workload's

- **Status:** accepted
- **Date:** 2026-11-12

## Context

Two packs hand a team an account of its own. `presets/folder-workload.satz` (ADR 0086)
gives an account folders to create projects in. `presets/project-onboarding.satz` created
a project and, inside it, one account for that project alone.

Two things were missing.

- **One account for several projects.** A team with a backend and a frontend project had
  two accounts and two states, or grants written by hand in the organisation's estate.
- **An account that works in folders and in one project elsewhere.** No pack could give a
  folder workload's account a project outside its folders.

The prerequisite check (ADR 0087) also knew nothing of what a project's account was given:
a project's estate was judged at the organization.

## Options

1. **A second list of identities in the project pack**, shaped like `folder_workloads`.
   - Pros: the project pack stands alone.
   - Cons: two packs that create the same kind of account, with the same labels; an
     account that spans folders and projects would still need a link between them.
2. **A project entry names a workload that `folder-workload` declares.** The project
   pack creates accounts only for projects that have their own.
   - Pros: one place where a shared account is declared; an account spans folders and
     projects by the same means; nothing an existing estate holds changes.
   - Cons: a shared account without folders is an entry of a list called
     `folder_workloads`; both packs must be on for such an entry.
3. **Replace the per-project account** by shared accounts throughout.
   - Cons: every project already onboarded would get a new account and lose its state
     bucket, which the pack creates with `force_destroy`. Not built.

## Decision

Option 2, as `presets/project-workload.satz`, which replaces `project-onboarding.satz`;
the old path and params are refused and name the new ones (ADR 0041).

- `project_workload_projects` — `{ name owner_group apis }`: the old `projects` list,
  unchanged in what it deploys: the project, its own account, its state bucket.
- `project_workload_shared_projects` — `{ name workload apis }`: the project, and
  `project_workload_roles` on it for the account of `workload`. The grant is written at
  the top level with its project named, and names the account by reference
  (`google_service_account.deploy_<workload>`): an entry that names a workload nobody
  declares is refused by the compile, and one account on several projects is several
  grants with several addresses. A grant written inside a project node takes its address
  from member and role alone, which is why it is not written there.
- Both kinds publish `interface "<name>"` with `project_id`, `project_number` and
  `delivered_project_roles`; a project with its own account adds `iac_account`,
  `state_bucket` and `delivered_apis`. An estate that works in a shared project reads the
  account from `workload-<name>` of `folder-workload.satz`, whose infrastructure project
  is published as `deploy_project_id` from now on, so the two files do not both say
  `project_id`.
- The prerequisite check reads the delivery from every interface the estate uses. A role
  delivered on the project meets a need on a project. The roles on a workload's folders
  count only for an estate that reads a folder's interface. A role the account lacks on
  its project is reported as the estate's own to grant — it holds the project's IAM —
  and everything else as what to ask the organisation's estate for.
- Two `each` over two different lists may write an interface of the same written name,
  `"{each.name}"`; the parser refused that as one name declared twice.

## Consequences

- One account works in several projects, and a folder workload's account can be given a
  project outside its folders, without a third pack.
- An estate on the old pack renames one path and three params; the plan does not move.
- A param named `projects` is refused for one release line wherever it is declared: left
  in place it would empty the list and the next apply would destroy the projects.
- `update-prerequisites` does not write the grant a project's account gives itself; the
  finding says what to write.
- `satz init --project` writes the estate of a project with its own account. The estate
  of a shared project is written by hand from the two interfaces.
- A project entry and a workload of the same name are two Google projects with one label
  and are refused by the fold.
