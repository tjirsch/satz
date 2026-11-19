# Projects beside the organisation

This tutorial starts where [an organisation from scratch](tutorial-organisation.md)
ends: Acme's estate runs as its IaC service account and publishes `interfaces/outgoing/common/`.
A customer team — `shop` — now needs Google projects of its own, in a folder of its own
under `Workloads`, with its own repository, state and pipeline, and it must never run as
the organisation's account. Two kinds of team do this: one writes Satz, one writes OpenTofu
and nothing else. Both read the organisation through its **interface**, and the same
template serves both. The files are [`examples/acme/`](../examples/README.md).

## Delegated access: one identity, and the places it works in

**Delegated access** gives a team access to a part of the organisation's resources — a
folder, a project — with scoped permissions: an account of its own that holds roles there
and nowhere else, and cannot widen them.

A team beside the organisation needs three things from the organisation's estate: an
account its pipelines run as, a place its state lives, and somewhere it may build. It
must hold no more than that — not the organisation's account, and nothing that reaches
another team. Three packs of the library hand this out. One is the **base**: the
identity, which every delegation has. The other two are the **targets**: where that
identity works, and they differ in one question: **who creates the Google projects.**

| | [`presets/delegation.satz`](../presets/README.md#delegationsatz) | [`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz) | [`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz) |
|---|---|---|---|
| what it is | the base: one identity per team | a target: a folder | a target: a Google project |
| the team | every team with delegated access | rolls out its own projects — one per stage, one per customer — and needs no approval for each | works in a project, or a few, that the platform team creates and approves |
| the organisation creates | per entry of `delegations` an infrastructure project with the account and the state bucket in it | per entry of `delegated_folders` a **folder** | per entry of `delegated_projects` a **project**, with no account and no bucket in it |
| the account may | be used by the members of one group; link a project to the billing account; look folders up | create folders and projects in the folder, set the access of its projects, enable their APIs | set the access of the project and enable its APIs; it creates no project |
| the account may not | reach anything no target names | set access on any folder, its own included; touch organisation policies, tags or billing administration | reach anything outside the project |
| switched on by | `use_delegation` | `use_delegated_folders` | `use_delegated_projects` |

Every target entry names the delegation whose account works in it, `delegation =
"<name>"`. One account works in any number of folders and projects, and the folders need
no common root.

**Every delegation has an infrastructure project of its own**, `acme-deploy-<name>`. The
account and the state bucket stand in it, never in a project the team fills, and every
call the team's plans make is billed to it. So an API a call needs is enabled there, and
the team asks the organisation for it: `apis` of the delegation's entry. An API a
resource needs inside a delegated project the team enables there itself.

The intention is the same for both targets. The organisation's estate decides, in one
reviewed file, what an account can reach; the packs create exactly that; and the account
cannot widen it. On a folder the account holds no access control, so it grants itself
nothing there. On a project it holds the project's access control and nothing else, so
what it grants itself stays inside the project.

Each pack also tells the team's estate what it was given. The interfaces carry the roles
and APIs that were delegated, satz judges the team's estate against them at every compile,
and a gap is reported as what to ask the organisation for — never as a grant at the
organisation ([what an estate must
declare](../README.md#what-an-estate-must-declare-update-prerequisites)).

Steps 1 to 5 walk a folder with the team `shop`; step 6 walks projects. The same
ground as a procedure for the two teams, by side — what to ask, what to hand over, what to
do next — is [delegated access: the procedure for both sides](tutorial-delegated-access.md).

## 1. What the organisation publishes, and what a lookup is

```bash
satz interfaces C0example.satz --format text --out -
```

Every `transpile` of the organisation writes `interfaces/outgoing/common/core/` — its core
exports in two forms, an HCL module under `hcl/` and a Satz file `satz/interface.satz`,
with a `README.md` that names each value. A value has one of two kinds:

- **A static value** satz knows when it compiles — `customer_shortname` is `"acme"`,
  `default_region` is `"europe-west3"`, `infra_project_id` is `"acme-infra-001"` — and
  publishes as the literal.
- **A looked-up value** only Google knows — `workload_folder` is `folders/<number>`, and
  the number exists once the folder does. It publishes as a **data lookup**: a `data
  "google_active_folder"` block that finds the folder by its display name under the
  organisation. The lookup runs on every plan of the project, through the project's own
  provider and credentials, so the identity that plans needs
  `resourcemanager.folders.list` on the organisation; the README names the permission
  beside the value. The organisation's state is never read.

A project takes `interfaces/outgoing/common/` whole — a copy, or the organisation's repository at
a tag — and takes it again after every change of the organisation. The folder's README
carries a content hash, and a transpile that changes a value writes `CHANGES.md` beside
the interface's README with what a project does about it.

## 2. The team's identity and its folder, once

A team gets an identity of its own and a folder of its own from the organisation's estate:
the packs [`presets/delegation.satz`](../presets/README.md#delegationsatz) and
[`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz). The
packs are switched on, and each list gets one entry:

```bash
satz add-preset C0example.satz use_delegation
satz add-preset C0example.satz use_delegated_folders
satz add-entry C0example.satz delegations name=shop owner_group=gcp-shop-admins@example.com
satz add-entry C0example.satz delegated_folders name=shop delegation=shop display_name=Shop
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`add-preset` binds a pack's switch and activates its line. `add-entry` adds one entry to
a list a pack declares: it checks the entry against the declaration and keeps it only
when the estate compiles. In `C0example.satz` the four commands leave:

```
params {
  use_delegation            = true
  use_delegated_folders     = true
  delegation_project_folder = "google_folder.infra_folder.name"
  delegations               = [{ name = "shop" owner_group = "gcp-shop-admins@example.com" }]
  delegated_folders         = [{ name = "shop" delegation = "shop" display_name = "Shop" }]
}
```

`delegation_project_folder` is a question, which `satz interview` asks when it is not
bound. The apply creates, for the delegation `shop`:

| what | the team's |
|---|---|
| Google project `acme-deploy-shop` | the delegation's infrastructure project: where its calls are billed and its state lives, under the infrastructure folder |
| service account `svc-deploy-shop@acme-deploy-shop.iam.gserviceaccount.com` | what its pipelines run as |
| bucket `acme-deploy-shop-state` | where its states live; the account writes it |
| folder `Shop` under `Workloads` | where it works: every folder and project it creates stands in it |
| roles on the folder `Shop`, and on no other | folder and project creation and deletion, the IAM of its projects, their APIs (`delegated_folders_roles`). No access control on a folder, no organisation policy, no tag |
| `roles/billing.user` on the billing account | linking a new project |
| `roles/browser` on the organisation | the one organisation-level right: the folder lookups of step 1 list a parent's folders |
| impersonation for `gcp-shop-admins@example.com` | the group's members may become the account, and nothing else |

A second folder for the same account is a second entry of `delegated_folders` that
names `shop`, under any parent; a delegation that needs more APIs or a further billing
role says so in its entry ([the pack](../presets/README.md#delegationsatz)).

Every transpile of the organisation writes the current `interfaces/outgoing/`; the team takes
`delegation-shop/` — its identity, and what was delegated to it — and `shop/` — its folder —
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
  // the four-digit cost centre the project is billed to
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

**`params`** are fixed for every instance: the team's identity from step 2 — the project
the plan is billed to, the account it runs as, the bucket the state lives in — and the
billing account. `deployment_mode = "cloud"` makes every provider and the backend
impersonate `svc-deploy-shop@acme-deploy-shop.iam.gserviceaccount.com`.

```
use "interfaces/incoming/delegation-shop/delegation-shop/satz/interface.satz"
use "interfaces/incoming/shop/shop/satz/interface.satz"
```

**The interface files**, from the two folders the team took, read as
`${{interface.<export>}}`: the delegation's identity with what was delegated to it, and the
folder it works in. Each carries the organisation's core values too.

```
google_project {
  project {
    name            = "{application}-{stage}"
    project_id      = "${{interface.customer_shortname}}-{application}-{stage}"
    folder_id       = "${{interface.delegated_folder}}"
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

**The project**: its id from the organisation's short name and the inputs, its parent
the team's own folder, looked up, a label from an input, and three grants to groups named
after the project. The grants are labelled resources (`owner { role member }`) rather
than the member map (`"group:…" = [ … ]`): a group name that reads an input is a value,
and an input is never a key — a key is a Terraform resource name. The groups exist in the
directory before the apply.

## 4. The Satz way

A team that writes Satz keeps the template and binds it from a thin estate. In the
team's repository:

```text
shop-dev/
  config.toml                 include_dirs reaches the template; schemas/ and presets/ beside
  satz/shop_dev.satz          the instance: the inputs bound, the template used
  interfaces/incoming/        outgoing/delegation-shop/ and outgoing/shop/ of the organisation, taken whole
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
cp -R <the organisation>/interfaces/outgoing/delegation-shop <the organisation>/interfaces/outgoing/shop interfaces/incoming/
satz transpile shop_dev.satz
```

`hcl/` holds:

| file | what |
|---|---|
| `interface.tf` | the organisation's values, in two locals: `interface_static` with the ones satz knows, which the providers read, and `interface`, which merges the looked-up ones in — `delegated_folder = data.google_active_folder.shop.name` — followed by the `data` blocks themselves, each with the permission it needs in a comment above it |
| `main.tf` | the project: `project_id = "${local.interface.customer_shortname}-${var.application}-${var.stage}"`, `folder_id = local.interface.delegated_folder`, `"cost_centre" = var.cost_centre`, the three grants. No value of the organisation stands in it |
| `providers.tf` | the `google` provider on the team's project, impersonating the team's account, its region from `local.interface_static.default_region`; the `gcs` backend on the team's bucket; one alias per project the estate declares |
| `variables.tf` | `variable "application"`, `"stage"`, `"cost_centre"`, each described by the comment above its input. The template's params follow them, as every estate's do, and no resource reads those |
| `terraform.tfvars` | `application = "shop"`, `stage = "dev"`, `cost_centre = "4711"` — the bound values, with the params after them |

Plan and apply as the team's account — a member of `gcp-shop-admins@example.com`,
logged in with `gcloud`, is who the impersonation runs for:

```bash
satz transpile shop_dev.satz --plan
satz transpile shop_dev.satz --apply
```

The plan reads the team's folder live (`roles/browser`), creates `acme-shop-dev` in
it (`roles/resourcemanager.projectCreator` on that folder), links the billing account
(`roles/billing.user`) and sets its grants (`roles/resourcemanager.projectIamAdmin`).

The compile checks exactly that. It reads what was delegated to the account from the
interface `delegation-shop` and judges the estate's resource types against it, so this estate
compiles without a prerequisite warning. A template that needs more — a budget needs
`billingbudgets.googleapis.com` on `acme-deploy-shop` and `roles/billing.costsManager` — is
told what to ask the organisation's estate for, and `--apply` refuses until the API is
delegated: the team's account can grant itself nothing above its own projects. A
second project is a second thin estate — `shop-test/satz/shop_test.satz` with
`stage = "test"` — or, for a run, `satz transpile shop_dev.satz --input stage=test`.

When the organisation changes, the team takes its two folders again, reads `CHANGES.md`
in each for what moved, and transpiles: `interface.tf`
follows, `main.tf` does not change.

The compile holds the project to the interface's rules: a resource attached to the
organisation's objects where no attach point allows it, an authoritative grant or an
organisation policy on a node the organisation manages, or a resource the organisation
declares too, is refused at its line.

## 5. The HCL way

A team that works in OpenTofu alone never runs satz. Someone runs it once — the
platform team, or the team on a machine that has it — and hands over `hcl/`:

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
| `interface.tf` | takes it again after every change of the organisation — or writes the organisation's values into it by hand, which is what pointing the same `main.tf` at another organisation means |
| `providers.tf` | leaves as it is: it runs as the team's account on the team's project, with the state in the team's bucket |
| `main.tf`, `variables.tf` | leaves as they are; a second project of the same shape is a second directory with its own `terraform.tfvars` |

The organisation holds that HCL to the same rules the Satz way is held to, offline,
against a checkout of its estate:

```bash
satz check-consumer <the team's hcl directory> C0example.satz
```

A value reads the organisation when it is `local.interface.<export>` of the
`interface.tf` satz wrote, or `module.satz.<export>` of the interface's HCL module
sourced by a `module` block; `check-consumer` names the file and line of an attachment
no attach point allows, an authoritative grant or an organisation policy on the
organisation's nodes, or a resource the organisation declares too. The organisation's
CI runs it against the team's repository.

## 6. The other target: delegating a project the organisation creates

The team `payments` does not roll out projects. It gets one. Its delegation is the same
base as in step 2 — an infrastructure project with the account and the state bucket —
and its target is an entry of
[`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz):

```bash
satz add-preset C0example.satz use_delegated_projects
satz add-entry C0example.satz delegations name=payments owner_group=payments-owners@example.com
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
    { name = "shop" owner_group = "gcp-shop-admins@example.com" },
    { name = "payments" owner_group = "payments-owners@example.com" },
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
`deploy_project_id`, `iac_account`, `state_bucket`, and what was delegated beside the
target: `delegated_billing_roles` and `delegated_apis`. `interfaces/outgoing/payments/`
publishes `project_id`, `project_number`, `delegated_project_roles` and
`delegated_budgets`.

The team writes its estate from the two folders, in its own repository:

```bash
satz init --from <the organisation>/interfaces/outgoing/delegation-payments <the organisation>/interfaces/outgoing/payments
```

`init --from` copies both folders into `interfaces/incoming/` and writes `config.toml` and
`satz/payments.satz`: an estate that runs as `svc-deploy-payments`, bills its calls to
`acme-deploy-payments`, keeps its state under `payments/state` in the delegation's bucket
and reads the organisation through the two interface files. Its resources name the
project as `project = "${{interface.project_id}}"`.

**What the account lacks on its project, it grants itself.** A bucket needs
`roles/storage.admin`, which the pack did not delegate. The compile says so on the
finding's line — `roles/storage.admin on the project — to grant in this estate` — and the
team writes the grant in its own estate, which the interface allows on `project_id`:

```
google_project_iam_member {
  project = "${{interface.project_id}}"
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/storage.admin",
  ]
}
```

Anything else — a folder, an organisation policy — is reported as `outside the project`:
this account does not get it. An API a call needs and the infrastructure project lacks is
what to ask the organisation for: `apis` of the delegation's entry.

**One account for several projects.** The team `shop` of the steps above has a delegation
already. The organisation gives its account two projects:

```bash
satz add-entry C0example.satz delegated_projects name=shop-backend delegation=shop
satz add-entry C0example.satz delegated_projects name=shop-frontend delegation=shop
```

```
params {
  delegated_projects = [
    { name = "payments" delegation = "payments" },
    { name = "shop-backend" delegation = "shop" },
    { name = "shop-frontend" delegation = "shop" },
  ]
}
```

On each, `svc-deploy-shop` gets the same two roles. The entry names the account by its
delegation, and one that names a delegation nobody declared is refused by the compile,
which lists the accounts that exist. An account that works in projects only is a
`delegations` entry without a folder.

The estate that works in `shop-backend` reads two interfaces — the account from
`delegation-shop`, the project from `shop-backend`:

```bash
satz init --from <the organisation>/interfaces/outgoing/delegation-shop <the organisation>/interfaces/outgoing/shop-backend --name shop-backend
```

`--name` names the estate, which otherwise takes the delegation's name: this writes
`satz/shop_backend.satz`, with the two `use` lines, and the team adds its resources:

```
use "interfaces/incoming/delegation-shop/delegation-shop/satz/interface.satz"
use "interfaces/incoming/shop-backend/shop-backend/satz/interface.satz"

google_storage_bucket {
  assets {
    project  = "${{interface.project_id}}"
    name     = "acme-shop-backend-assets"
    location = "EU"
  }
}
```

It runs as `svc-deploy-shop` and keeps its state under `shop_backend/state` in the
delegation's bucket. One estate works in one project; two projects are two estates that
run as the same account.

## What the two ways share

| | the Satz way | the HCL way |
|---|---|---|
| the truth about the organisation | the two `interface.satz` files under `interfaces/incoming/` | `interface.tf` |
| the values that change per instance | `params { }` of a thin estate, or `--input` | `terraform.tfvars`, or `-var` |
| the identity | the delegation's account, in `params` of the template | the delegation's account, in `providers.tf` |
| the state | the team's bucket, `gcs` backend | the same |
| the organisation's state | never read | never read |
| held to the interface by | the compile | `satz check-consumer` |
| after the organisation changes | take the two interface folders again, transpile | take `interface.tf` again |

One template, one interface, one identity per team; what differs is who runs the
compiler.
