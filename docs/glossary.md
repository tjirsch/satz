# Glossary

One word for one thing. These are the words of satz, its documentation, satz-studio and
the texts satz writes, each for the meaning given here. Three rules hold:

1. **A term defined here is used for its thing.**
2. **Where an entry allows a second word, one of the two is used — no third.**
3. **A word defined here is not used for any other meaning.**

The rules hold for prose, command help, messages, labels and comments. A **name in the
format** — a name inside a file, a command or a report — may keep an older word; the last
section lists those. Definitions name a cloud only where the thing is that cloud's own.
The spelling is American English.

## The estates and who works on them

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **estate** | | The written description of one cloud setup — what should exist — in the language Satz. satz transpiles it into HCL code that OpenTofu applies. | every satz command takes one; satz-studio opens one |
| **estate file** | | The `.satz` file that is the estate, beginning with `estate <name>`. Used where the file as such is meant: "copy the estate file to …". | `satz/<name>.satz` |
| **organization** | | A company's whole cloud: the top of the tree that its folders and projects hang under. | `satz init` |
| **customer** | | The company that owns the organization and signs the decisions sheet. Used only where the company is meant, not its cloud. | `--customer-shortname`, the decisions sheet |
| **organization's estate** | | The estate that describes the organization as a whole: its folders, policies, groups and every delegation. One per organization. | `satz init`; satz-studio Create, "An organization" |
| **workload** | | Something a team builds and runs in the cloud: a service, a data platform, an application. It is composed of workload packs and individual definitions. | a workload estate |
| **workload estate** | | An estate that builds one workload inside the resources the organization delegated to it. It runs as the delegation's estate account and reaches nothing else. | `satz init --from …`; satz-studio Create |
| **workload repository** | | The repository that holds a workload estate, with its `config.toml`, its workload packs and its interface folders. | the tutorials |
| **workload folder** | | The folder in the cloud that holds the delegated resources of workloads. Set in the organization's estate file. Never used for anything on disk. | `workload_folder_name` |
| **infrastructure folder** | | The folder in the cloud that holds every infrastructure project: the organization's own and every delegation's. It stands wherever the customer wants it. Set in the organization's estate file. | `infra_folder_name` |
| **team** | | A group of people with a common focus. Qualified only where a text must tell two groups apart: "the organization's team", "a workload's team". | the tutorials |

## Delegated access

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **delegated access** | | The mechanism by which a team gets an estate account of its own, with rights on named resources of the organization and nowhere else, and works from its own repository. | `presets/delegation.satz` |
| **delegation** | | One case of delegated access, with a name (`shop`): one estate account, its infrastructure project, its state bucket, its estate account user group. One entry of the list param `delegations`. | `satz add-entry <estate> delegations …` |
| **delegated resource** | | What a delegation's estate account may work in or on: a folder, a project, and whatever kind the preset library adds. One entry of a list param such as `delegated_folders` or `delegated_projects`. A delegation has any number of them. | `satz add-entry <estate> delegated_projects …` |
| **infrastructure project** | | The project, in the infrastructure folder, that holds an estate's estate account and state bucket: the organization's, created by the bootstrap, or a delegation's. The estate builds nothing else in it. | `<shortname>-infra-001`, `<shortname>-deploy-<name>` |
| **estate account** | account, where only it can be meant | The service account an estate runs as: not a person, no password, no key. "The organization's estate account" and "the delegation's estate account" where both are in a text. | `svc-iac-001`, `svc-deploy-<name>` |
| **estate account user group** | | The group whose members may act as the estate account. Who is a member is decided in the company's directory, not in satz. | `estate_account_user_group` |
| **act as** | impersonate | A signed-in person uses the estate account's rights for one command, because they are a member of its estate account user group. | cloud mode; `--no-impersonate` |

## What one estate gives another

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **interface** | | A named set of values one estate publishes for another to read: ids, names, and what was delegated. | `satz interfaces <estate>`; satz-studio Estate → Interfaces |
| **interface folder** | | The folder satz writes for one interface, with a README, the values in two forms (for Satz and for HCL) and a list of changes. An **outgoing** interface folder is one the estate publishes, under `interfaces/outgoing/<name>/`, written whole by every transpile. An **incoming** one was copied from another estate's repository, under `interfaces/incoming/<name>/`; satz never writes there. | `interfaces/outgoing/`, `interfaces/incoming/` |
| **export** | | One published value of an interface, with a name (`project_id`). | `export "…" = …`; the interface folder's README |
| **named read** | | Reading an export by the name of its interface folder: `${{interface.<folder>.<value>}}`. The short form `${{interface.<value>}}` is for what every used folder agrees on. | a workload estate |
| **list param** | | A param whose value is a list of entries, declared by a preset or workload pack with the `entries` statement, which says what one entry looks like: `delegations`, `delegated_projects`. | `satz add-entry <estate> <list param> …`; satz-studio Estate → Interfaces |
| **entry** | | One element of a list param: one delegation, one delegated resource. | `satz add-entry` |
| **request file** | | A small file in which a workload's team asks for entries of the organization's list params, written as `contributes_<list param> = [ … ]`; the organization's team checks it and adds it. | `satz check-request` |

## Packs

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **pack** | | The technical word: a `.satz` file that starts with `pack`, defines resources and bundles them. Presets and workload packs are packs. Used where either kind, or the file as such, is meant. | the `pack` statement; `satz review-preset`; report fields `packs` |
| **preset** | | A pack of the preset library, kept current for everyone. An organization's estate is built from presets; they are switched on and off per estate. | `presets/`; `satz add-preset`; satz-studio Presets |
| **preset library** | | All presets together, as one release of satz ships them, with the estate map, the changelog and the compliance catalogs. | `satz get-presets`, `satz merge-presets` |
| **workload pack** | pack, where only this kind can be meant | A pack for a workload estate that builds the workload inside what its delegation pack created, as the delegation's estate account: a RAG store, a budget. Taken once from a pack source as a template, then owned by the repository that took it. | `packs/`; `satz get-pack`; satz-studio Packs |
| **delegation pack** | workload delegation pack | A pack for the organization's estate that creates the additional infrastructure a workload pack needs — the project or folder the workload is built in, for example — and grants the delegation's estate account its roles on it. What it creates stays in the organization's state. A workload that needs nothing beyond its delegation has none. | `presets/delegated-folders.satz`, `presets/delegated-projects.satz` |
| **pack source** | | Where a workload pack is fetched from: the satz repository, a pack repository, or a folder. Each has a name, which the origin record writes. | `[[pack_source]]` in `config.toml`; `satz pack-source`; `satz get-pack --source` |
| **estate map** | | The preset that holds the questions which decide which other presets an organization's estate switches on. | `presets/estate-map.satz` |
| **create** | | What a switched-on pack does with the resources it defines: the preset creates a log bucket and a sink. They exist in the cloud once the plan is applied. | `satz presets`, column "Creates" |
| **named use** | | A pack used under a name, with params of its own, so one pack can be used several times: `use "packs/project-budget.satz" named payments`. The estate binds its params as `payments.<param>`; what it creates is written as `payments__<label>`. Its questions are asked once per use, `payments.<param>`, and its notices acknowledged once per use. Only a pack whose header says `multiple` takes one. | `use … named`; `satz transpile`, `satz questions`, `satz interview`, `satz presets`, `satz packs`, `satz adopt` |
| **switch on, switch off** | | To make a preset part of an estate, or take it out; satz writes the line. | `satz add-preset`, `satz remove-preset` |

## Questions and answers

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **question** | | Something a preset or workload pack asks before it can be used: "which region?". | `satz questions`; satz-studio Decisions |
| **answer** | | The value given to a question; stored as a param in the estate file. | `satz interview` |
| **param** | | A named value in the estate file's `params { }` block: `default_region = "europe-west3"`. An answer is stored as one. | satz-studio Estate → Params |
| **no value** | | What a param has when every file that declares it writes `none` and the estate binds nothing: the file reads it and proposes nothing. Not `""` and not `[]`, which are values. A read of it stops the transpile unless a question asks it, when it is blocking. | `satz transpile`; `satz questions` (`blocking`); `satz doc-presets` ("`none` — must be provided") |
| **local value** | | A named value in a file's `locals { }` block, read by its bare name in that file and in no other: never bound by a using file, never a variable, never asked by a question. Not a param, and not `private`, which keeps a resource out of an interface. | the `locals` statement; `satz lsp` hover |
| **decision** | | A question together with its answer — chosen, or taken as offered — and what changing it later costs. What the customer signs on the decisions sheet. | satz-studio Decisions |
| **interview** | | Going through the open questions one by one. | `satz interview` |

## From estate to cloud

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **transpile** | | satz reads an estate, checks it, and writes its HCL code. | `satz transpile <estate>` |
| **HCL code** | generated HCL code, transpiled HCL code | What a transpile writes into `hcl/`: files in the HCL format, for OpenTofu and compatible with Terraform. Never edited by hand. | `hcl/` |
| **OpenTofu** | | The separate open-source tool that reads the HCL code and makes the changes in the cloud; satz starts it. Named only where the tool is meant. Terraform runs the same code and is named where it is the one meant. | `--tf-tool` |
| **plan** | | The list of what an apply would create, change or delete. Nothing changes. | `satz transpile --plan` |
| **apply** | | To carry out the plan in the cloud. | `satz transpile --apply`; satz-studio Apply |
| **state** | | OpenTofu's record of what it created, so that the next plan knows what exists. | `deployment_mode` |
| **state bucket** | | The storage bucket the state is kept in, in the estate's infrastructure project. | `<shortname>-infra-001-state` |
| **local mode, cloud mode** | | Where the state is kept. Local: on this computer, as during the bootstrap, before the state bucket exists; the estate runs as the signed-in person. Cloud: in the state bucket; the estate runs as its estate account, which the signed-in person acts as. | `deployment_mode` |
| **bootstrap** | | The one-time first setup of an organization, done by a person with their own rights: the infrastructure project, the billing link, the state bucket, the estate account. Everything after runs as the estate account. | `satz bootstrap`; satz-studio Apply |
| **prerequisites** | | The roles the estate account needs and the services that must be switched on before an estate can apply. satz works them out from the estate and says which are missing. | `satz update-prerequisites` |
| **signed in** | | A person has logged in on this computer so that satz may call the cloud in their name. | `satz whoami`; satz-studio Apply |
| **message** | | A problem satz reports about an estate, with its place and what to do. Its severity is an error, a warning or a note. | every command; satz-studio Messages |
| **finding** | | What a security tool reports about a resource: a check it ran and its result. Prowler and Checkov report findings; satz reads them, it raises none itself. | `satz scan`, `report-compliance --prowler`, `triage`, the remediation dossier; satz-studio Security |
| **notice** | | A note a preset or workload pack attaches for whoever switches it on: what to do before or after. | `satz presets`; satz-studio Overview |

## The documents

| Term | Also allowed | What it is | Where it shows |
|---|---|---|---|
| **documents** | | The files an estate produces for people — the decisions sheet, the workbook, the evidence report, the estate documentation — each made again after a change. | satz-studio Documents |
| **decisions sheet** | | The document that lists every decision of an estate. Before anything is applied the customer signs it; afterwards it is the record. | `satz questions --format markdown` |
| **workbook** | | The decisions as a spreadsheet the customer fills in and sends back. | `satz questions --format xlsx` |
| **evidence report** | | The report that compares an estate with what is live in the organization, for a compliance catalog. | the compliance commands |
| **estate documentation** | | A short description of the organization as Word documents, written from the estate. | satz-studio Documents |

## Words not used

| Not | Say |
|---|---|
| central estate, org estate, organization estate | organization's estate |
| team's estate, team estate, project estate, a project on satz | workload estate |
| team's repository; workload folder for anything on disk | workload repository |
| Workloads folder | workload folder |
| target, place, scope (of a delegation) | delegated resource |
| onboarding a project, delivery | delegated access |
| deploy project, infra project, seed project | infrastructure project |
| IaC account, deployer, identity | estate account |
| account group, owner group, IaC users group, deployers group | estate account user group |
| become the account | act as, impersonate |
| hand-over folder, handover, vendor folder | interface folder |
| hand over | copy |
| output (for an export), published value | export |
| request point | list param |
| component, building block, service pack | workload pack, or preset |
| pack map | estate map |
| compile, emit, generate, translate | transpile |
| generated code, OpenTofu code, Terraform code, root module | HCL code |
| deploy, roll out | apply; for what a pack does: create |
| builds, deploys (for what a pack does) | creates |
| day 0 | bootstrap |
| rights check, permission check, pre-flight | prerequisites |
| logged in, credentials, ADC, Application Default Credentials | signed in, sign-in |
| diagnostic | message |
| sign-off page, sign-off sheet, handover record, decisions catalog | decisions sheet |
| organisation and other British spellings | American spelling |

`scripts/check-wording.sh` reports every line that uses one of these, as far as a pattern
can find it; the patterns are `scripts/check-wording.txt`. Rule 3 is for review.

## Names in the format

These names stay as they are: `satz transpile`, `hcl/`, `hcl_dir`, `satz bootstrap`,
`--no-impersonate`, `--iac-user`, `satz check-consumer`, `workload_folder`,
`workload_folder_name`, `infra_folder_name`, the project ids `<shortname>-infra-001` and
`<shortname>-deploy-<name>`, the account ids `svc-iac-001` and `svc-deploy-<name>`,
`deployment_mode`, the statements `pack` and `locals`, the keyword `none` of a param with no value,
the keyword `named` of a `use` and the word `multiple` of a pack's header, the prefix `contributes_` of a request file, `satz get-pack`, `satz packs`,
the export `account_group_roles` of a delegation's interface, and the report fields `packs` and `pack`.
