# 0096 — the last renames before 1.0

- **Status:** accepted
- **Date:** 2026-11-19

## Context

With 1.0, the names inside an estate file — params, the fields of a list's entries, the
exports an interface publishes, the statements — are frozen: a later rename costs every
estate an edit, and every interface folder a team copied. The glossary
([ADR 0095](0095-one-word-for-one-thing-the-glossary-and-its-check.md)) fixed the words
satz uses for its things, and kept names in the format out of its rules. A review of the
glossary found five names that contradict its words where a reader meets them every day:

- the entry field `owner_group` of `delegations` and the export `account_group` — both are
  the **estate account user group**;
- the export `iac_account` — the **estate account**;
- the export `deploy_project_id` — the delegation's **infrastructure project**;
- the param `svc_iac_users_group` of `presets/estate-core.satz` — the organization's
  **estate account user group**.

The statement `request <list param> { … }` named one of three uses of what it declares:
the shape of one entry of a list param, which the estate's own entries, `satz add-entry`
and a team's request file are all held to. Read as "request", it seemed to apply to the
request file alone.

A delegation's infrastructure project was created in the folder `delegation_project_folder`
named, the organization by default. The infrastructure folder is where every
infrastructure project belongs (glossary: **infrastructure folder**), and a choice that
has one right answer is a question nobody needs to be asked.

The workload folder could be empty, which made the organization itself the parent of the
delegated resources. Every preset that places a delegated resource then needs a parent
that resolves in both cases, and none does: `google_folder.workload_folder` does not
exist in the second, a pack cannot read the estate's `workload_folder` export, and a
project takes an organization as `org_id`, not as a folder. So `delegated_projects_folder`
defaulted to the organization, and a delegated folder's default parent failed outright
in such an estate.

## Options

1. **Keep the names and explain them in the glossary.** No estate is edited. Cost: the
   format contradicts the words of every text that describes it, permanently; the glossary
   carries a translation table that every reader of an estate file has to know.
2. **Rename the few unambiguous ones before 1.0.** The five names above, the statement,
   the removed folder choice, and a workload folder every organization's estate has. Cost: every existing estate is edited once, every
   interface folder taken from a delegation is copied again, and satz-studio follows the
   renamed report fields.
3. **Rename every name that uses an older word** — `svc_iac_account`, `infra_project_name`,
   `infra_folder_name`, `workload_folder_name`, `deployment_mode`, the project and account
   ids `<shortname>-deploy-<name>`, `svc-deploy-<name>`, the statement `pack`. Cost: the ids
   are resource addresses and names of live resources — renaming them recreates projects
   and accounts — and the bootstrap params are in every script and runbook. The gain over
   option 2 is consistency of words that are not wrong, only older.

## Decision

Option 2:

| before | now |
|---|---|
| entry field `owner_group` of `delegations` | `estate_account_user_group` |
| export `account_group` | `estate_account_user_group` |
| export `iac_account` | `estate_account` |
| export `deploy_project_id` | `infrastructure_project_id` |
| param `svc_iac_users_group` | `estate_account_user_group` |
| statement `request <list param> { … }` | `entries <list param> { … }`, the same body |
| param `delegation_project_folder` | removed: the project is created in `google_folder.infra_folder` |
| `satz init --workload-folder-name` | removed: the workload folder is the answer to `workload_folder_name` |
| `workload_folder_name = ""`, the organization as the workload folder | required: a name, and the section that declares the folder |
| `delegated_projects_folder` default `""`, the organization | `google_folder.workload_folder.name`, like `parent` of `delegated_folders` |

- **Every old name is refused by name, with the new one**, as earlier renames are: the
  param (`RENAMED_PARAMS`, and `REMOVED_PARAMS` for the one with no successor), the entry
  field (`RENAMED_FIELDS`, at the binding's line), the statement (at its line), and a read
  of an old export, which names the new one; a read of a new export from a folder copied
  before the rename says the folder is to be copied again. There is no compatibility path.
- **The request file keeps its name**: `satz check-request`, the prefix `contributes_`
  and "request file" in prose are what a team writes to ask for entries, which is a
  request.
- **The report fields that mirror the exports follow them**: `satz presets --format json`'s
  `delegated_by[]` carries `estate_account`, `estate_account_user_group` and
  `infrastructure_project` where it carried `account`, `account_group` and
  `deploy_project`; `satz interfaces --format json` carries `list_params` where it carried
  `requests`, since each row is one `entries` statement.
- **The workload folder is required.** `workload_folder_name` has no default and its
  question no empty answer; an estate that binds `""`, or publishes the organization as
  `workload_folder`, is refused at the line to edit, naming the question. Only one form of
  the section exists, the folder, so every preset can name it by reference: delegated
  folders and delegated projects default to `google_folder.workload_folder.name`, and an
  entry or the param still names another parent.

## Consequences

- Every existing estate is edited once: `satz get-presets --force`, then the renames in
  the estate file, as `presets/CHANGELOG.md` (Breaking changes, v0.102.0) lists them.
- The workload estates read the new export names, and the interface folders they took
  from a delegation are copied again from the organization's next transpile.
- No resource address changes. A delegation's infrastructure project that stood under the
  organization or another folder moves into the infrastructure folder: `folder_id` changes
  on the `google_project`, which the provider migrates in place.
- satz-studio and every interface folder already copied follow the new names.
- An estate written by `satz init` carries no workload folder until `workload_folder_name`
  is answered; the interview asks it with the other bootstrap questions, among those with
  no possible default.
- An estate whose workload folder was the organization declares one: the folder is
  created or imported, and the delegated projects that stood directly under the
  organization move into it in place. So do the delegated projects of an estate that
  left `delegated_projects_folder` unbound.
