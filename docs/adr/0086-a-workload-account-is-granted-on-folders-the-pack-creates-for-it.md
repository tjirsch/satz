# 0086 — a workload's account is granted on folders the pack creates for it

- **Status:** accepted
- **Date:** 2026-11-11

## Context

`presets/workload-deployer.satz` gave a team an account of its own and granted it
`roles/resourcemanager.folderAdmin`, `projectCreator`, `projectIamAdmin` and
`serviceUsageAdmin` on a folder that already existed — by default the workload folder every
team shares. Two things were wrong with that.

- **The grant reached every team.** A role on a folder is inherited by everything under it,
  and `folderAdmin` carries `resourcemanager.folders.setIamPolicy`. Each team's account
  could grant itself any role on the shared folder, and so on every other team's projects.
- **The folder was somebody else's.** The pack granted on a folder it did not create, so
  what a team could reach was decided by where a folder happened to stand, not by the pack.

The estate that runs as such an account also has to be told what it was given: the folder
to create under, and the roles and APIs it can count on, which the prerequisite check reads.

## Options

1. **Keep the grant on the shared folder, without `folderAdmin`.**
   - Pros: no new folders; one line changes.
   - Cons: every team still creates and deletes projects and folders beside every other
     team's, and `projectIamAdmin` on the shared folder still reaches their projects.
2. **The pack creates one folder per entry and grants on that folder alone.** Two lists:
   the identities, and the folders, each naming its identity.
   - Pros: what an account reaches is what the pack created for it; one account can work
     in several folders that share no root; nothing is granted on a folder teams share.
   - Cons: a folder that exists has to be adopted before the pack manages it; an estate
     on the old pack has to be migrated by hand.
3. **One list, the folders nested in each identity's entry.**
   - Pros: one entry says everything about a workload.
   - Cons: `each` expands one list, not a list inside an entry; it would need a new
     language form.
4. **A grant-only form for folders the organisation's estate does not manage.**
   - Cons: it is the old defect with a new name — a grant on a folder whose contents and
     position the pack does not decide. Not built.

## Decision

Option 2, as `presets/folder-workload.satz`, which replaces the old pack; the old path and
param names are refused and name the new ones (ADR 0041).

- `folder_workloads` — `{ name group apis billing_roles }`: the infrastructure project,
  the account, the state bucket; `apis` and `billing_roles` default in the request point.
- `folder_workload_folders` — `{ name workload display_name parent }`: a folder the pack
  creates under `parent` (the workload folder by default, any declared folder or an
  existing folder's id otherwise — ADR 0085), and `folder_workload_roles` on it for the
  account of `workload`. The grant names the account by reference, so an entry that names
  a workload no entry declares is refused by the compile.
- `folder_workload_roles` is folderCreator, folderEditor, projectCreator, projectDeleter,
  projectIamAdmin and serviceUsageAdmin. None carries `resourcemanager.folders.setIamPolicy`,
  an organisation-policy write or a tag permission (checked against Google's role
  reference on the date above).
- **Two interfaces.** `workload-<name>` publishes `project_id`, `iac_account`,
  `state_bucket` and what was delivered — `delivered_folder_roles`,
  `delivered_billing_roles`, `delivered_apis`. `<folder>` publishes `delivered_folder`. A
  workload's estate uses both files.

The packs are named after how far their account reaches: `folder-workload` here, and a
`project-workload` grown out of `project-onboarding.satz` for accounts that work in
projects somebody else creates.

## Consequences

- An account cannot widen its own rights: it sets no IAM on any folder. Inside its
  folders it can make itself owner of the projects it creates; limits there are
  organisation policies the organisation's estate sets on the workload's folder.
- The account can delete its own folder once it is empty (`folderEditor`). The
  organisation's next apply creates it again.
- **Why two interfaces and not one per folder.** An interface cannot take in another
  whose name reads the list entry (`use interface` takes a written name), and an export's
  name cannot read it either. So a folder's interface cannot carry its workload's values,
  and one estate cannot read two folders of its workload: both publish
  `delivered_folder`. A workload with two folders is rolled out by two estates that run
  as the same account. Lifting that is a language change, left out on purpose.
- The export is `delivered_folder`, not `workload_folder`: the organisation's core
  exports already publish `workload_folder`, the shared folder, and every interface
  carries the core exports.
- An estate on the old pack migrates by hand; `presets/CHANGELOG.md` under v0.96.0 has the
  steps and what the plan moves. Projects created under the shared folder stay there.
