# Workload estates beside the organization

This tutorial starts where [an organization from scratch](tutorial-organization.md)
ends: Acme's estate runs as its estate account and publishes `interfaces/outgoing/common/`.
A customer team — `shop` — now needs Google projects of its own, in a folder of its own
under `Workloads`, with its own repository, state and pipeline, and it must never run as
the organization's estate account. Two kinds of team do this: one writes Satz, one writes OpenTofu
and nothing else. Both read the organization through its **interface**, and the same
template serves both. The files are [`examples/acme/`](../examples/README.md).

## Delegated access: one estate account, and the delegated resources it works in

**Delegated access** gives a team access to a part of the organization's resources — a
folder, a project — with scoped permissions: an estate account of its own that holds roles
there and nowhere else, and cannot widen them.

A team beside the organization needs three things from the organization's estate: an
estate account its pipelines run as, a bucket its state lives in, and somewhere it may
build. It must hold no more than that — not the organization's estate account, and nothing
that reaches another team. Three presets of the library hand this out. One is the
**base**: the delegation's estate account, which every delegation has. The other two are the **delegated resources**: where
that account works, and they differ in one question: **who creates the Google projects.**

| | [`presets/delegation.satz`](../presets/README.md#delegationsatz) | [`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz) | [`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz) |
|---|---|---|---|
| what it is | the base: one estate account per team | a delegated resource: a delegated folder | a delegated resource: a delegated project |
| the team | every team with delegated access | creates its own projects — one per stage, one per customer — and needs no approval for each | works in a project, or a few, that the organization's team creates and approves |
| the organization creates | per entry of `delegations` an infrastructure project with the estate account and the state bucket in it | per entry of `delegated_folders` a **folder** | per entry of `delegated_projects` a **project**, with no account and no bucket in it |
| the account may | be used by the members of its estate account user group; link a project to the billing account; look folders up | create folders and projects in the folder, set the access of its projects, enable their APIs | set the access of the project and enable its APIs; it creates no project |
| the account may not | reach anything no delegated resource names | set access on any folder, its own included; touch organization policies, tags or billing administration | reach anything outside the project |
| switched on by | `use_delegation` | `use_delegated_folders` | `use_delegated_projects` |

Every entry of a delegated resource names the delegation whose account works in it, `delegation =
"<name>"`. One account works in any number of folders and projects, and the folders need
no common root.

**Every delegation has an infrastructure project of its own**, `acme-deploy-<name>`. The
account and the state bucket stand in it, never in a project the team fills, and every
call the workload estate's plans make is billed to it. So an API a call needs is enabled there, and
the team asks the organization for it: `apis` of the delegation's entry. An API a
resource needs inside a delegated project the team enables there itself.

The intention is the same for both kinds of delegated resource. The organization's estate decides, in one
reviewed file, what an account can reach; the packs create exactly that; and the account
cannot widen it. On a folder the account holds no access control, so it grants itself
nothing there. On a project it holds the project's access control and nothing else, so
what it grants itself stays inside the project.

Each pack also tells the workload estate what it was given: the interfaces carry the roles
and APIs that were delegated, for the workload's team to read. The packs are the ready-made
delegation packs — they stand in the organization's estate, create what the workload is
given and grant its account the roles — and satz checks no role in the workload estate:
`satz update-prerequisites --report-only` lists the roles its resource types need, for the
organization's team, and proposes no grant ([what an estate must
declare](../README.md#what-an-estate-must-declare-update-prerequisites)).

Steps 1 to 5 walk a folder with the team `shop`; step 6 walks projects. The same
ground as a procedure for the two teams, by side — what to ask, what to copy, what to
do next
 — is [delegated access: the procedure for both sides](tutorial-delegated-access.md).

## 1. What the organization publishes, and what a lookup is

```bash
satz interfaces C0example.satz --format text --out -
```

Every `transpile` of the organization writes `interfaces/outgoing/common/core/` — its core
exports in two forms, an HCL module under `hcl/` and a Satz file `satz/interface.satz`,
with a `README.md` that names each value. A value has one of two kinds:

- **A static value** satz knows when it transpiles — `customer_shortname` is `"acme"`,
  `default_region` is `"europe-west3"`, `infra_project_id` is `"acme-infra-001"` — and
  publishes as the literal.
- **A looked-up value** only Google knows — `workload_folder` is `folders/<number>`, and
  the number exists once the folder does. It publishes as a **data lookup**: a `data
  "google_active_folder"` block that finds the folder by its display name under the
  organization. The lookup runs on every plan of the workload estate, through its own
  provider and estate account, so the account that plans needs
  `resourcemanager.folders.list` on the organization; the README names the permission
  beside the value. The organization's state is never read.

A workload estate takes `interfaces/outgoing/common/` whole — a copy, or the organization's repository at
a tag — and takes it again after every change of the organization. The folder's README
carries a content hash, and a transpile that changes a value writes `CHANGES.md` beside
the interface's README with what a workload estate does about it.


## 2. The delegation's account and its delegated folder, once

A team gets an account of its own and a delegated folder of its own from the organization's estate:
the packs [`presets/delegation.satz`](../presets/README.md#delegationsatz) and
[`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz). The
packs are switched on, and each list gets one entry:

```bash
satz add-preset C0example.satz use_delegation
satz add-preset C0example.satz use_delegated_folders
satz add-entry C0example.satz delegations name=shop estate_account_user_group=gcp-shop-admins@example.com
satz add-entry C0example.satz delegated_folders name=shop delegation=shop display_name=Shop
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`add-preset` binds a pack's switch and activates its line. `add-entry` adds one entry to
a list a pack declares: it checks the entry against the declaration and keeps it only
when the estate transpiles. In `C0example.satz` the four commands leave:

```
params {
  use_delegation        = true
  use_delegated_folders = true
  delegations           = [{ name = "shop" estate_account_user_group = "gcp-shop-admins@example.com" }]
  delegated_folders     = [{ name = "shop" delegation = "shop" display_name = "Shop" }]
}
```

The apply creates, for the delegation `shop`:

| what | the delegation's |
|---|---|
| Google project `acme-deploy-shop` | the delegation's infrastructure project: where its calls are billed and its state lives, under the infrastructure folder |
| service account `svc-deploy-shop@acme-deploy-shop.iam.gserviceaccount.com` | what its pipelines run as |
| bucket `acme-deploy-shop-state` | where its states live; the account writes it |
| folder `Shop` under `Workloads` | where it works: every folder and project it creates stands in it |
| roles on the folder `Shop`, and on no other | folder and project creation and deletion, the IAM of its projects, their APIs (`delegated_folders_roles`). No access control on a folder, no organization policy, no tag |
| `roles/billing.user` on the billing account | linking a new project |
| `roles/browser` on the organization | the one organization-level right: the folder lookups of step 1 list a parent's folders |
| group `gcp-shop-admins@example.com` | the estate account user group, created empty in the customer's directory; someone with directory rights adds the team's people to it |
| impersonation for `gcp-shop-admins@example.com` | the group's members may become the account, and nothing else |

A second folder for the same account is a second entry of `delegated_folders` that
names `shop`, under any parent; a delegation that needs more APIs or a further billing
role says so in its entry ([the pack](../presets/README.md#delegationsatz)).

Every transpile of the organization writes the current `interfaces/outgoing/`; the team takes
`delegation-shop/` — its estate account, and what was delegated to it — and `shop/` — its folder —
from there. Everything the team does from here runs as `svc-deploy-shop`.

## 3. The template

[`examples/acme/simple-project/satz/simple-project.satz`](../examples/acme/simple-project/satz/simple-project.satz)
is one Google project as a template. Read it top to bottom:

```
inputs {
  // the application the project serves: lowercase letters, digits and `-`
  application = "shop"
  // the stage: dev, test or prod
  stage       = "dev"
  // the four-digit cost center the project is billed to
  cost_centre = "4711"
}
```

**`inputs`** are what a reader binds — the template's contract. Everything satz checks
reads them as their value (the example until a reader binds one); the HCL reads them as
`var.<name>` ([language §6.3](language.md#inputs--a-templates-contract-the-values-a-reader-binds)).

```
params {
  infra_project_name = "acme-deploy-shop"
  svc_iac_account    = "svc-deploy-shop"
  infra_bucket_name  = "acme-deploy-shop-state"
  billing_account    = "012345-6789AB-CDEF01"
  deployment_mode    = "cloud"
}
```

**`params`** are fixed for every instance: the delegation's account from step 2 — the project
the plan is billed to, the account it runs as, the bucket the state lives in — and the
billing account. `deployment_mode = "cloud"` makes every provider and the backend
impersonate `svc-deploy-shop@acme-deploy-shop.iam.gserviceaccount.com`.

```
use "interfaces/incoming/delegation-shop/delegation-shop/satz/interface.satz"
use "interfaces/incoming/shop/shop/satz/interface.satz"
```

**The interface files**, from the two interface folders the team took: the delegation's
estate account with what was delegated to it, and the folder it works in. A value is read by
the name of its interface folder, `${{interface.<folder>.<value>}}` — the delegated folder
is `${{interface.shop.delegated_folder}}`. Each folder carries the organization's core
values too, with the same content, and a value the organization says once — the short
name, the domain, the region — is read without a folder's name,
`${{interface.customer_shortname}}`.

```
google_project {
  project {
    name            = "{application}-{stage}"
    project_id      = "${{interface.customer_shortname}}-{application}-{stage}"
    folder_id       = "${{interface.shop.delegated_folder}}"
    billing_account = billing_account
    labels          = { cost_centre = cost_centre }
    project_service = ["cloudresourcemanager.googleapis.com"]
    google_project_iam_member {
      owner {
        role   = "roles/owner"
        member = "group:${{interface.customer_shortname}}-{application}-{stage}-owner@${{interface.customer_domain}}"
      }
      …
    }
  }
}
```

**The project**: its id from the organization's short name and the inputs, its parent
the delegated folder, looked up, a label from an input, and three grants to groups named
after the project. The grants are labeled resources (`owner { role member }`) rather
than the member map (`"group:…" = [ … ]`): a group name that reads an input is a value,
and an input is never a key — a key is a Terraform resource name. The groups exist in the
directory before the apply.

## 4. The Satz way

A team that writes Satz keeps the template and binds it from a thin estate. In the
workload repository:

```text
shop-dev/
  config.toml                 include_dirs reaches the template; schemas/ and presets/ beside
  satz/shop_dev.satz          the instance: the inputs bound, the template used
  interfaces/incoming/        outgoing/delegation-shop/ and outgoing/shop/ of the organization, taken whole
  hcl/                        written by satz
```

[`examples/acme/shop-dev/config.toml`](../examples/acme/shop-dev/config.toml) is the
configuration (`include_dirs = [".", "satz", "../simple-project/satz"]`; `satz
update-schema` fetches `schemas/`), and the estate is the whole instance:

```
estate shop_dev

params {
  application = "shop"
  stage       = "dev"
  cost_centre = "4711"
}

use "simple-project.satz"
```

A using document's `params { }` binds a template's inputs as it binds any param. Then:

```bash
mkdir -p interfaces/incoming
cp -R <the organization>/interfaces/outgoing/delegation-shop <the organization>/interfaces/outgoing/shop interfaces/incoming/
satz transpile shop_dev.satz
```

`hcl/` holds:

| file | what |
|---|---|
| `interface.tf` | the organization's values, in two locals: `interface_static` with the ones satz knows, which the providers read, and `interface`, which merges the looked-up ones in — `delegated_folder = data.google_active_folder.shop.name` — followed by the `data` blocks themselves, each with the permission it needs in a comment above it |
| `main.tf` | the project: `project_id = "${local.interface.customer_shortname}-${var.application}-${var.stage}"`, `folder_id = local.interface.delegated_folder`, `"cost_centre" = var.cost_centre`, the three grants. No value of the organization stands in it |
| `providers.tf` | the `google` provider on the delegation's infrastructure project, impersonating the delegation's estate account, its region from `local.interface_static.default_region`; the `gcs` backend on the delegation's state bucket; one alias per project the estate declares |
| `variables.tf` | `variable "application"`, `"stage"`, `"cost_centre"`, each described by the comment above its input. The template's params follow them, as every estate's do, and no resource reads those |
| `terraform.tfvars` | `application = "shop"`, `stage = "dev"`, `cost_centre = "4711"` — the bound values, with the params after them |

Plan and apply as the delegation's estate account — a member of
`gcp-shop-admins@example.com`, signed in with `gcloud`, is who the impersonation runs for:

```bash
satz transpile shop_dev.satz --plan
satz transpile shop_dev.satz --apply
```

The plan reads the delegated folder live (`roles/browser`), creates `acme-shop-dev` in
it (`roles/resourcemanager.projectCreator` on that folder), links the billing account
(`roles/billing.user`) and sets its grants (`roles/resourcemanager.projectIamAdmin`).

satz checks no role here: the roles are the organization's to grant, and `satz
update-prerequisites shop_dev.satz --report-only` lists those the resource types need. A
template that needs more — a budget needs `billingbudgets.googleapis.com` on
`acme-deploy-shop` and `roles/billing.costsManager` — finds both in that list, which the team
gives the organization's team; the delegation's account can grant itself nothing above its
own projects, and what it lacks at the apply is Google's refusal. A
second project is a second thin estate — `shop-test/satz/shop_test.satz` with
`stage = "test"` — or, for a run, `satz transpile shop_dev.satz --input stage=test`.

When the organization changes, the team takes its two folders again, reads `CHANGES.md`
in each for what moved, and transpiles: `interface.tf`
follows, `main.tf` does not change.

The transpile holds the project to the interface's rules: a resource attached to the
organization's objects where no attach point allows it, an authoritative grant or an
organization policy on a node the organization manages, or a resource the organization
declares too, is refused at its line.

## 5. The HCL way

A team that works in OpenTofu alone never runs satz. Someone runs it once — the
organization's team, or the team on a machine that has it — and copies `hcl/` to the team:


```bash
satz --config examples/acme/simple-project transpile simple-project.satz
```

The five files are the ones of step 4, with the example values in `terraform.tfvars`.
The team keeps them beside its other OpenTofu files and works with `tofu`:

```bash
cd hcl
tofu init
tofu plan                                   # shop-dev, as terraform.tfvars binds it
tofu plan -var stage=test -var cost_centre=0815
```

| file | the team |
|---|---|
| `terraform.tfvars` | edits it, or passes `-var` — the three inputs are the variables `main.tf` reads, and the id, the name, the label and the groups follow; the lines below them are the template's params, read by nothing |
| `interface.tf` | takes it again after every change of the organization — or writes the organization's values into it by hand, which is what pointing the same `main.tf` at another organization means |
| `providers.tf` | leaves as it is: it runs as the delegation's account on the delegation's infrastructure project, with the state in the delegation's state bucket |
| `main.tf`, `variables.tf` | leaves as they are; a second project of the same shape is a second directory with its own `terraform.tfvars` |

The organization holds that HCL to the same rules the Satz way is held to, offline,
against a checkout of its estate:

```bash
satz check-consumer <the workload's hcl directory> C0example.satz
```

A value reads the organization when it is `local.interface.<export>` of the
`interface.tf` satz wrote, or `module.satz.<export>` of the interface's HCL module
sourced by a `module` block; `check-consumer` names the file and line of an attachment
no attach point allows, an authoritative grant or an organization policy on the
organization's nodes, or a resource the organization declares too. The organization's
CI runs it against the workload repository.

## 6. The other delegated resource: a project the organization creates

The team `payments` does not create projects. It gets one. Its delegation is the same
base as in step 2 — an infrastructure project with the account and the state bucket —
and its delegated project is an entry of
[`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz):

```bash
satz add-preset C0example.satz use_delegated_projects
satz add-entry C0example.satz delegations name=payments estate_account_user_group=payments-owners@example.com
satz add-entry C0example.satz delegated_projects name=payments delegation=payments
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

In `C0example.satz`:

```
params {
  use_delegated_projects    = true
  delegated_projects_folder = "google_folder.workload_folder.name"
  delegations               = [
    { name = "shop" estate_account_user_group = "gcp-shop-admins@example.com" },
    { name = "payments" estate_account_user_group = "payments-owners@example.com" },
  ]
  delegated_projects        = [{ name = "payments" delegation = "payments" }]
}
```

The apply creates the delegation's infrastructure project `acme-deploy-payments` with the
account `svc-deploy-payments` and the state bucket `acme-deploy-payments-state`, and the
Google project `acme-payments-001` under `Workloads`. The project holds no account and no
bucket. The account holds the project's access control and its services on that project
and nowhere else; the members of `payments-owners@example.com` read the infrastructure
project and may become the account. `interfaces/outgoing/delegation-payments/` publishes
`infrastructure_project_id`, `estate_account`, its id `estate_account_id`, `state_bucket`, and what was delegated beside the
delegated project: `delegated_billing_roles` and `delegated_apis`. `interfaces/outgoing/payments/`
publishes `project_id`, `project_number`, `delegated_project_roles` and
`delegated_budgets`.

The team writes its estate from the two folders, in its own repository:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-payments <the organization>/interfaces/outgoing/payments
```

`init --from` copies both folders into `interfaces/incoming/` and writes `config.toml` and
`satz/payments.satz`: an estate that runs as `svc-deploy-payments`, bills its calls to
`acme-deploy-payments`, keeps its state under `payments/state` in the delegation's bucket
and reads the organization through the two interface files. Its resources name the
project as `project = "${{interface.payments.project_id}}"`.

**What the account lacks on its project, it may grant itself.** A bucket needs
`roles/storage.admin`, which the pack did not delegate; `satz update-prerequisites
--report-only` lists it as `roles/storage.admin on the project — for google_storage_bucket`.
The team writes the grant in its own estate, which the interface allows on `project_id`:

```
google_project_iam_member {
  project = "${{interface.payments.project_id}}"
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/storage.admin",
  ]
}
```

Anything else — a folder, an organization policy — this account does not get. An API a
call needs and the infrastructure project lacks is what to ask the organization for: `apis`
of the delegation's entry.

**One account, several delegated resources.** One account works in any number of delegated folders
and projects, and one estate reads them all. The organization gives the delegation `data`
two projects, `data-lake` and `data-mart`:

```bash
satz add-entry C0example.satz delegations name=data estate_account_user_group=data-owners@example.com
satz add-entry C0example.satz delegated_projects name=data-lake delegation=data
satz add-entry C0example.satz delegated_projects name=data-mart delegation=data
```

```
params {
  delegations        = [
    { name = "shop" estate_account_user_group = "gcp-shop-admins@example.com" },
    { name = "payments" estate_account_user_group = "payments-owners@example.com" },
    { name = "data" estate_account_user_group = "data-owners@example.com" },
  ]
  delegated_projects = [
    { name = "payments" delegation = "payments" },
    { name = "data-lake" delegation = "data" },
    { name = "data-mart" delegation = "data" },
  ]
}
```

On each project, `svc-deploy-data` gets the same two roles. The entry names the account by
its delegation, and one that names a delegation nobody declared is refused by the transpile,
which lists the accounts that exist. An account that works in projects only is a
`delegations` entry without a folder.

The workload estate is written from three interface folders — the account from
`delegation-data`, a project each from `data-lake` and `data-mart`:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-data <the organization>/interfaces/outgoing/data-lake <the organization>/interfaces/outgoing/data-mart
```

This writes `satz/data.satz` with the three `use` lines and, as comments at its end, the
read for each delegated resource. The team adds its resources, each naming its project by the folder:

```
use "interfaces/incoming/delegation-data/delegation-data/satz/interface.satz"
use "interfaces/incoming/data-lake/data-lake/satz/interface.satz"
use "interfaces/incoming/data-mart/data-mart/satz/interface.satz"

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

It runs as `svc-deploy-data` and keeps its state under `data/state` in the delegation's
bucket. In `main.tf` the two projects are `local.interface.data_lake__project_id` and
`local.interface.data_mart__project_id`. `${{interface.project_id}}` is refused in this
estate: both folders carry a `project_id` and the two differ, and the message names the
two reads to choose from.

Exactly one folder is a delegation's own, because an estate runs as one account. `--name`
names the estate, which otherwise takes the delegation's name; two estates of one
delegation keep their states apart in the delegation's bucket.

## What the two ways share

| | the Satz way | the HCL way |
|---|---|---|
| the truth about the organization | the `interface.satz` files under `interfaces/incoming/` | `interface.tf` |
| the values that change per instance | `params { }` of a thin estate, or `--input` | `terraform.tfvars`, or `-var` |
| the estate account | the delegation's estate account, in `params` of the template | the delegation's estate account, in `providers.tf` |
| the state | the delegation's state bucket, `gcs` backend | the same |
| the organization's state | never read | never read |
| held to the interface by | the transpile | `satz check-consumer` |
| after the organization changes | take the interface folders again, transpile | take `interface.tf` again |

One template, one interface, one estate account per team; what differs is who runs the
transpile.

