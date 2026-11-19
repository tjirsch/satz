# Delegated access: the procedure for both sides

**Delegated access** gives a team access to a part of the organization's resources — a
folder, a project — with scoped permissions: an estate account of its own that holds roles
there and nowhere else, and cannot widen them.

Two teams meet here. The **organization's team** owns the organization's estate and may
change it. The **workload's team** builds something beside it — in its own repository, with
its own state — and must never run as the organization's estate account. This page is the
process between them, step by step and by side: what the workload's team asks for, what the
organization's team decides and creates, what is copied to the workload's team, and what
each does next.

It is a procedure to follow, not a walk through the language.
[Workload estates beside the organization](tutorial-projects.md) explains the packs and shows
every file; the [library](../presets/README.md#delegationsatz) lists every field. The
example is Acme Corp. of [`docs/examples.md`](examples.md) and its teams `shop` and
`payments`.

## The process

![Two lanes, the organization's team and the workload's team, and nine steps: the workload's team asks; the organization's team decides, writes the entries, plans and applies, and copies the interface folders to the workload's team; the workload's team sets up its workload estate, transpiles, joins the estate account user group and signs in, then plans and applies as its own estate account. A dashed arrow leads from the transpile back to the decision: something is not delegated, ask for it](delegated-access-first.svg)

A dashed box is a step people do: a conversation, a decision, copying folders. A solid box is
a step done with satz. The numbers are the sections below.

Three things are true throughout:

- **Nothing secret changes hands.** No key, no password. What is copied is a few
  interface folders that hold names and what was granted.
- **The right to act as the estate account comes from a group.** The organization's estate
  names the estate account user group; who is in it is decided in the directory, not in satz.
- **The account cannot widen what it was given.** What it lacks, the team asks for.

## What a delegation is

A **delegation** is one account for one team, and the delegated resources that account
works in. It has two parts, and each is an entry in a list of the organization's estate.

| part | list, pack | one entry creates |
|---|---|---|
| the base: the estate account | `delegations`, [`presets/delegation.satz`](../presets/README.md#delegationsatz) | an infrastructure project `acme-deploy-<name>`, in it the estate account `svc-deploy-<name>` and the state bucket `acme-deploy-<name>-state`, and the right of its estate account user group to act as the estate account |

| a delegated resource: a delegated folder | `delegated_folders`, [`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz) | a folder the account may create folders and projects in |
| a delegated resource: a delegated project | `delegated_projects`, [`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz) | a Google project `acme-<name>-001` the account may fill |

Every entry of a delegated resource names its base: `delegation = "<name>"`. One base has
any number of delegated resources, of either kind.

**Every delegation has an infrastructure project of its own.** The account and the state
bucket stand in it, never in a project the team fills. A delegated project holds no
account, no state bucket and no group: it is worked by the account of the delegation its
entry names. Two things follow for the team:

- **The account's calls are billed to the delegation's infrastructure project.** Every call
  the workload estate's plan makes to a Google API is counted against `acme-deploy-<name>`, so the API has to
  be enabled there. The team does not own that project. An API its calls need beyond the
  standard six is asked for (step 1) and written into `apis` of the delegation's entry.
- **An API a resource needs inside a delegated project the team enables there itself**,
  in its own estate: its account holds that project's services.

The three cases of this page:

| the workload's team | the organization writes | what the team gets |
|---|---|---|
| creates its own projects, as many as it needs | one entry in `delegations`, one in `delegated_folders` per folder | an estate account, a state bucket, and one or more folders it may create projects in |
| works in one project it is given | one entry in `delegations`, one in `delegated_projects` | an account, a state bucket, and the project |
| works in several projects with one account | one entry in `delegations`, one in `delegated_projects` per project | an account, a state bucket, and the projects |

A delegation and one of its delegated resources may share a name: the project `payments`
of the delegation `payments`. Two delegated resources may not: a folder and a project of one name would be
one interface folder.

## 1. Workload's team: ask

Bring this to the conversation. Everything on it ends up in the organization's estate,
so the clearer it is, the fewer rounds.

| what | example | why it is asked |
|---|---|---|
| a name for the delegation | `shop` | it names the account `svc-deploy-shop`, its infrastructure project and its bucket: lowercase letters, digits and `-`, starting with a letter |
| the estate account user group, whose members may act as the estate account | `gcp-shop-admins@example.com` | its members run the plans; it must exist in the directory |
| the folders it works in, or the projects it needs | one folder `shop` under `Workloads` | the account gets its rights there and nowhere else |
| APIs its calls need beyond the standard six | `pubsub.googleapis.com`, `billingbudgets.googleapis.com` | the calls are billed to the delegation's infrastructure project, and the team cannot enable an API on a project it does not own |
| a budget for its own project or folder | `budgets = true` | the team cannot grant itself anything on the billing account; this gives it budgets for its own scope and nothing else |
| billing roles beyond linking a project | rarely: `roles/billing.costsManager` covers every project's budgets and costs | prefer `budgets = true` above |

The standard six APIs are `cloudbilling`, `cloudresourcemanager`, `iam`, `iamcredentials`,
`serviceusage` and `storage`, each `.googleapis.com`. Every resource type the workload
estate declares calls one API; that API is one of the six, or it is asked for.

If the team does not know the last three yet, it asks without them. Step 7 names exactly
what is missing, and the second diagram below is the round that adds it.

## 2. Organization's team: decide

- **Which delegated resources**, by the table above: folders, projects, or both.
- **Where the folders or projects go.** A folder is created under the workload folder
  unless the entry names another parent. A delegation's folders need no common root.
  Every delegated project is created in one folder, `delegated_projects_folder`: the
  workload folder unless the estate names another.
- **Which group.** The base grants that group the right to act as the account, and to
  read the delegation's infrastructure project. Membership stays in the directory.
- **Whether the extra APIs and billing roles are justified.** They are the only things
  that go beyond the packs' defaults.
- **What should hold inside a folder.** The account may create projects and set their
  access there. Limits inside the folder are organization policies the organization's
  estate sets on that folder.

What the account will and will not be able to do is in the packs' reference:
[`delegation`](../presets/README.md#delegationsatz),
[`delegated-folders`](../presets/README.md#delegated-folderssatz),
[`delegated-projects`](../presets/README.md#delegated-projectssatz).

## 3. Organization's team: write the entries

Once per estate, the library copy and the packs' switches:

```bash
satz get-presets
satz add-preset C0example.satz use_delegation
satz add-preset C0example.satz use_delegated_folders      # for folders
satz add-preset C0example.satz use_delegated_projects     # for projects
satz interview C0example.satz
```

`add-preset` binds the switch and makes the pack's `use` line active. It refuses while a
pack it needs is off, and names that pack. The infrastructure projects are created in the
infrastructure folder. The projects switch opens one question, which `satz interview`
asks: `delegated_projects_folder`, the folder the delegated projects are created in, the
workload folder unless another is named.

Then the entries. `satz add-entry <estate> <list> <field>=<value>…` adds one entry to a
list: it checks the fields and their patterns against the pack's declaration, refuses a
name the list already holds, writes the entry into `params { }` of the estate, and keeps
it only when the estate transpiles. Below each command is what it writes, for a reader who
edits the file instead. Pick the case.

**A team with folders.** The delegation `shop` and one folder:

```bash
satz add-entry C0example.satz delegations name=shop estate_account_user_group=gcp-shop-admins@example.com
satz add-entry C0example.satz delegated_folders name=shop delegation=shop display_name=Shop
```

```
params {
  delegations       = [{ name = "shop" estate_account_user_group = "gcp-shop-admins@example.com" }]
  delegated_folders = [{ name = "shop" delegation = "shop" display_name = "Shop" }]
}
```

A second folder for the same account is a second entry of `delegated_folders` that names
`shop`; `parent=google_folder.<label>.name` puts it under another folder the estate
declares.

**A team with one project.** The delegation `payments` and the project `payments`:

```bash
satz add-entry C0example.satz delegations name=payments estate_account_user_group=payments-owners@example.com
satz add-entry C0example.satz delegated_projects name=payments delegation=payments
```

```
params {
  delegations        = [{ name = "payments" estate_account_user_group = "payments-owners@example.com" }]
  delegated_projects = [{ name = "payments" delegation = "payments" }]
}
```

**One account working in several projects.** One entry of `delegated_projects` per
project, each naming the same delegation — here the delegation `data` with the projects
`data-lake` and `data-mart`:

```bash
satz add-entry C0example.satz delegations name=data estate_account_user_group=data-owners@example.com
satz add-entry C0example.satz delegated_projects name=data-lake delegation=data
satz add-entry C0example.satz delegated_projects name=data-mart delegation=data
```

```
params {
  delegations        = [{ name = "data" estate_account_user_group = "data-owners@example.com" }]
  delegated_projects = [
    { name = "data-lake" delegation = "data" },
    { name = "data-mart" delegation = "data" },
  ]
}
```

An entry that names a delegation no entry of `delegations` declares is refused by the
transpile, which lists the accounts that exist. A delegation needs no folder: an account
that works in projects only has entries in `delegated_projects` and none in
`delegated_folders`.

**More APIs or billing roles** are fields of the delegation's entry. A list's values are
separated by commas, and a list that is stated is stated whole — the six standard APIs
and the additions:

```bash
satz add-entry C0example.satz delegations name=shop estate_account_user_group=gcp-shop-admins@example.com \
  apis=cloudbilling.googleapis.com,cloudresourcemanager.googleapis.com,iam.googleapis.com,iamcredentials.googleapis.com,serviceusage.googleapis.com,storage.googleapis.com,pubsub.googleapis.com \
  billing_roles=roles/billing.user,roles/billing.costsManager
```

```
  delegations = [
    {
      name        = "shop"
      estate_account_user_group = "gcp-shop-admins@example.com"
      apis = [
        "cloudbilling.googleapis.com",
        "cloudresourcemanager.googleapis.com",
        "iam.googleapis.com",
        "iamcredentials.googleapis.com",
        "serviceusage.googleapis.com",
        "storage.googleapis.com",
        "pubsub.googleapis.com",
      ]
      billing_roles = ["roles/billing.user", "roles/billing.costsManager"]
    },
  ]
```

**A budget** for a folder or a project is `budgets=true` on its entry. The estate
switches the role for it on first, once
([the role](../presets/README.md#delegated-budget-rolesatz)); without it the entry is
refused:

```bash
satz add-preset C0example.satz use_delegated_budgets
satz add-entry C0example.satz delegated_projects name=payments delegation=payments budgets=true
```

`add-entry` adds an entry. Changing one — an API more on a delegation that exists — or
removing one is an edit of the estate file.

## 4. Organization's team: plan, read, apply

```bash
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

Read the plan before the apply. For the delegation `shop` with one folder it shows:

| in the plan | what it is |
|---|---|
| a Google project `acme-deploy-shop`, a service account `svc-deploy-shop`, a bucket `acme-deploy-shop-state` | the delegation's infrastructure project: where the account's calls are billed, what the workload estate's plans run as, where its state lives |
| a folder `Shop` | where the team builds |
| six role grants on the folder `Shop`, and none on `Workloads` | project and folder creation, project access, APIs — on its own folder only |
| one grant on the billing account, one on the organization (`roles/browser`) | linking a new project; looking a folder up by its name |
| grants for the estate account user group on the estate account and on the infrastructure project | the members may act as the estate account, and read its project |


For the delegation `payments` with one project the first, fourth and fifth rows are the
same with `payments` in the names, and the delegated project is:

| in the plan | what it is |
|---|---|
| a Google project `acme-payments-001` | where the team builds; no account and no bucket stand in it |
| two role grants on `acme-payments-001` for `svc-deploy-payments` | the project's access and its services — on that project only |

A grant on a folder the teams share, or a role that sets a folder's access, is not in
that plan. If one is, stop: something other than the packs wrote it.

## 5. Organization's team: copy the interface folders

Every transpile writes `interfaces/outgoing/` beside the estate. Copy these folders, whole, to
the workload's team:

| case | folders |
|---|---|
| a team with folders | `interfaces/outgoing/delegation-shop/` and one per folder, `interfaces/outgoing/shop/` |
| a team with one project | `interfaces/outgoing/delegation-payments/` and `interfaces/outgoing/payments/` |
| one account working in several projects | `interfaces/outgoing/delegation-data/` and one per project, `interfaces/outgoing/data-lake/` and `interfaces/outgoing/data-mart/` |

`delegation-<name>/` is the delegation's own folder: the estate account, the infrastructure
project, the state bucket, the estate account user group. Each other folder is one delegated resource. A copy is enough;
so is read access to the organization's repository at a tag.

The two halves of `interfaces/` are the two directions: `outgoing/` is what an estate
publishes, removed and written whole by every transpile; `incoming/` is what it was
given, and no transpile writes there. The organization's
`interfaces/outgoing/payments/` is the workload repository's
`interfaces/incoming/payments/`. The workload's team needs nothing else of the organization's: not its estate file, not its library, not its
state.

**The folders explain themselves.** Each has a `README.md` that satz writes for the team
that receives it. In the delegation's own folder, "Delegated access: what you were given"
says which estate account the workload estate runs as, which estate account user group's
members may act as it and what that group holds on the account, which infrastructure project its calls are billed to, where the
state lives, which billing account a new project is linked to, what the account holds on
the billing account, and which APIs are on. In a delegated resource's folder it says which delegated
folder or delegated project to build in and what the account holds there. Under "What to do with it" each
lists the team's next steps, and the first is the command of step 6. Nothing else has to
be written down to copy the folders.


Say one thing with it: the folders are taken again after every change of the
organization.

## 6. Workload's team: set up the estate

In the workload repository, which is empty, with the folders of step 5 within reach:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-shop <the organization>/interfaces/outgoing/shop
```

`init --from` takes the delegation's own interface folder and the interface folder of
every delegated resource the estate works in. Exactly one of the folders is a delegation's own: an
estate runs as one account. It copies each folder into `interfaces/incoming/` unless it
stands there already, fetches the provider schemas, and writes the workload estate under
the delegation's name:

```text
shop/
  config.toml
  satz/shop.satz
  interfaces/incoming/delegation-shop/   the delegation's own folder, whole
  interfaces/incoming/shop/              the delegated folder's, whole
  schemas/
  hcl/                                   written by satz transpile
```

`satz/shop.satz` runs as the delegation's account, bills its calls to the delegation's
infrastructure project, and keeps its state under `shop/state` in the delegation's
bucket:

```
estate shop

params {
  infra_project_name = "acme-deploy-shop"
  infra_bucket_name  = "acme-deploy-shop-state"
  svc_iac_account    = "svc-deploy-shop"
  default_region     = "europe-west3"
  default_zone       = "europe-west3-a"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
}

use "interfaces/incoming/delegation-shop/delegation-shop/satz/interface.satz"
use "interfaces/incoming/shop/shop/satz/interface.satz"
```

Below these it writes the `gcs` backend on the bucket and the providers on the
infrastructure project, and at the end, as a comment, the read for each delegated resource: `folder_id
= "${{interface.shop.delegated_folder}}"`. The team adds what it builds:

```
google_project {
  shop_dev {
    name            = "shop-dev"
    project_id      = "${{interface.customer_shortname}}-shop-dev"
    folder_id       = "${{interface.shop.delegated_folder}}"
    billing_account = "${{interface.delegation-shop.billing_account}}"
    project_service = ["cloudresourcemanager.googleapis.com"]
  }
}
```

**A value is read by the name of its interface folder**,
`${{interface.<folder>.<value>}}`: the delegated folder from `shop`, the billing account
from `delegation-shop`. The name is the folder's name under `interfaces/incoming/`.
`customer_shortname` is written without a folder's name because the organization says it
once: every interface folder carries it with the same content, and such a value — the
short name, the domain, the default region — is read as `${{interface.<value>}}`
([language §6.17](language.md#617-export-and-interface--what-the-estate-publishes-to-the-workload-estates-beside-it)).

Everything a team with a folder creates stands under
`${{interface.shop.delegated_folder}}`.

**A team with one project** names its two folders the same way:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-payments <the organization>/interfaces/outgoing/payments
```

Its resources name the project as `project = "${{interface.payments.project_id}}"`, and
an API a resource needs inside the project is a `google_project_service` there.

**One account, several delegated resources.** One estate works in any number of delegated
folders and projects: `init --from` takes the delegation's own interface folder and one
per delegated resource.
The delegation `data` with the projects `data-lake` and `data-mart`:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-data <the organization>/interfaces/outgoing/data-lake <the organization>/interfaces/outgoing/data-mart
```

This writes `satz/data.satz` with three `use` lines. It runs as `svc-deploy-data` and
keeps its state under `data/state` in the delegation's bucket. Its last lines are the
read for each delegated resource:

```
use "interfaces/incoming/delegation-data/delegation-data/satz/interface.satz"
use "interfaces/incoming/data-lake/data-lake/satz/interface.satz"
use "interfaces/incoming/data-mart/data-mart/satz/interface.satz"

// The estate's own resources. A value is read by the name of its interface folder:
//   in the delegated project `data-lake`:    project   = "${{interface.data-lake.project_id}}"
//   in the delegated project `data-mart`:    project   = "${{interface.data-mart.project_id}}"
```

Each resource says which project it stands in by the folder's name:

```
google_storage_bucket {
  raw {
    project  = "${{interface.data-lake.project_id}}"
    name     = "${{interface.customer_shortname}}-data-lake-raw"
    location = "${{interface.default_region}}"
  }
}

google_project_service "mart_bigquery" {
  project = "${{interface.data-mart.project_id}}"
  service = "bigquery.googleapis.com"
}
```

Both folders carry a `project_id`, and the two differ, so `${{interface.project_id}}` is
refused here: the message names `data-lake` and `data-mart` and the two reads to choose
from. `default_region` is the same in all three folders and keeps its short form.

Exactly one of the folders is a delegation's own. Work for a second account is a second
estate, written by its own `init --from`; `--name` names an estate, which otherwise takes
the delegation's name, and two estates of one delegation keep their states apart in the
delegation's bucket, each under its own name.

## 7. Workload's team: transpile

```bash
satz transpile shop.satz --check
```

The transpile judges the estate against what was delegated. It is quiet when everything
the estate needs was delegated. Otherwise each line says who closes it:

| the line says | who acts | what to do |
|---|---|---|
| `… on the delegated folders` | organization's team | ask for the role; it is added to the folder roles, `delegated_folders_roles` |
| `… on the billing account` | organization's team | ask for it in `billing_roles` of the delegation's entry |
| `… API(s) … not enabled on …, the project its calls are billed to` | organization's team | ask for it in `apis` of the delegation's entry: the project is the delegation's infrastructure project |
| `… on the project — to grant in this estate` | workload's team | write the grant in this estate (below) |
| `… outside the project` | nobody: this account does not get it | build it another way, or ask for a folder |

The one gap a team closes itself is a role on a project it was given, because its account
holds that project's access control:

```
google_project_iam_member {
  project = "${{interface.payments.project_id}}"
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/storage.admin",
  ]
}
```

**An estate with several delegated resources.** The transpile does not judge which of them
a resource stands in. A role counts as delegated when it was delegated on every one of its kind —
on every delegated project the estate uses, on every delegated folder — and budgets count
as delegated when every interface folder that speaks of budgets says yes. A role the
account holds on one project and lacks on another is reported.

`satz update-prerequisites shop.satz --report-only` prints the same as a list, with what
was delegated. Nothing at the organization is ever proposed, and `satz update-prerequisites`
writes no grant for a workload estate. `--apply` is refused while an API is missing.

A note `presets/pack-graph.json is not here` is about the organization's preset library,
which a workload repository does not hold. It asks nothing of the team.

**Workload packs.** What a team builds can come as a pack: `satz packs --format text --out -`
lists the ones its pack sources hold (the satz repository, and each one `config.toml`
names) and says what each needs — a delegated folder or a delegated project — and `satz
get-pack <name>` fetches one. `project-budget` is such a
pack: with `budgets = true` on the project's entry it creates a budget for the delegated
project, after two questions (the amount, the currency) and an address to tell.

```bash
satz get-pack project-budget
```

```
use "packs/project-budget.satz"
```

**A pack may switch on the APIs it adds.** The organization switched on the standard ones
(the interface folder lists them); a pack declares the others in the project itself, and
the account's role on its project allows it. Declaring one the organization already
switched on is reported: two estates would own one switch.

## 8. Workload's team: be in the group, and sign in

Whoever runs the plan is a member of the estate account user group from step 1 — added in
the directory, by whoever administers it — and signed in:

```bash
gcloud auth application-default login
satz whoami shop.satz
```

`whoami` says which identity the run uses. With `deployment_mode = "cloud"` every provider
and the state backend act as `svc-deploy-shop`.

A workload that creates Google groups needs one more thing no satz file grants: the Groups
Admin role for its account, assigned in the Google admin console by a directory
administrator.

## 9. Workload's team: plan, read, apply

```bash
satz transpile shop.satz --plan
satz transpile shop.satz --apply
```

The plan looks the folder up live, creates `acme-shop-dev` in it and links the billing
account. The delegation's account is the owner of every project it creates.

## Afterwards

![Two lanes and two cases. A, the team needs more: the transpile names the gap, the team asks for it, the organization's team edits the entry, plans and applies and copies the folders to the team again, the team takes them again and transpiles. B, the organization changed: an apply changes an export and satz writes CHANGES.md into the interface folder, the organization's team tells the teams or lets the change notice do it, the team takes the folders again, reads CHANGES.md, transpiles and plans](delegated-access-later.svg)

**A. The team needs more.** The transpile says what, and which list takes it (step 7). The
organization's team edits the delegation's entry — an API, a billing role — or adds one
more folder or project with `satz add-entry`, plans, applies, and hands the folders over
again. The team replaces its copies under `interfaces/incoming/` and transpiles: the line
is gone. A further delegated resource is a further interface folder to take and a further estate (step 6).

**B. The organization changed.** A transpile that changes an export writes

`CHANGES.md` into the interface folder, with what a reader does about it. The team takes
the folders again, reads it, transpiles and plans. The organization's team tells the teams, or
switches on the change notice that does
([`presets/interface-notice.satz`](../presets/README.md#interface-noticesatz)).

**Taking a delegation away** is for the organization's team, and it is not an edit like the
others: removing an entry destroys what the pack created for it — for a delegation the
infrastructure project with the account and the state bucket with every state in it, for
a delegated resource the project, or the folder once it is empty. The team removes what it built
first.

## The written route

Steps 1 to 3 can be done in writing instead. The workload's team puts its entries into a
small file of its own, checks it against the organization's estate, and sends it as a pull
request to the organization's repository; merging it is the decision of step 2.

```
pack shop_request version "1.0"

params {
  contributes_delegations       = [{ name = "shop" estate_account_user_group = "gcp-shop-admins@example.com" }]
  contributes_delegated_folders = [{ name = "shop" delegation = "shop" }]
}
```

```bash
satz check-request shop_request.satz C0example.satz
```

A project is asked for the same way, as `contributes_delegated_projects`. Nothing takes
effect before the organization's team merges and applies. Steps 4 to 9 are the same.

## Who does what

| | organization's team | workload's team |
|---|---|---|
| decides what an estate account can reach | yes | asks |
| creates the delegation's infrastructure project, its estate account and bucket, and the folders or projects | yes, with the packs | no |
| names the estate account user group | yes | proposes it |
| puts people into that group | whoever administers the directory | whoever administers the directory |
| copies to the workload's team | the interface folders,
 after every change; their READMEs are the instructions | — |
| writes the workload estate | no | yes, with `satz init --from` |
| creates projects in a delegated folder | no | yes |
| grants roles on a project the team was given | no | yes, in its own estate |
| adds an API on the infrastructure project, a billing role, a folder, a project, `budgets = true` | yes, on request | asks |
| switches on an API a resource needs, in a project the team was given | no | yes, in its own estate |
| removes a delegation | yes, after the team emptied it | empties it |
