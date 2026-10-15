# Projects beside the organisation

This tutorial starts where [an organisation from scratch](tutorial-organisation.md)
ends: Acme's estate runs as its IaC service account and publishes `interfaces/common/`.
A customer team — `shop` — now needs Google projects of its own under the `Workloads`
folder, with its own repository, state and pipeline, and it must never run as the
organisation's account. Two kinds of team do this: one writes Satz, one writes OpenTofu
and nothing else. Both read the organisation through its **interface**, and the same
template serves both. The files are [`examples/acme/`](../examples/README.md).

## 1. What the organisation publishes, and what a lookup is

```bash
satz interfaces C0example.satz --format text --out -
```

Every `transpile` of the organisation writes `interfaces/common/core/` — its core
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

A project takes `interfaces/common/` whole — a copy, or the organisation's repository at
a tag — and takes it again after every change of the organisation. The folder's README
carries a content hash, and a transpile that changes a value writes `CHANGES.md` beside
the interface's README with what a project does about it.

## 2. The team's identity, once

A team gets an identity of its own from the organisation's estate: the pack
[`presets/workload-deployer.satz`](../presets/README.md#workload-deployersatz). In
`C0example.satz`:

```
params {
  use_workload_deployer   = true
  deployer_project_folder = "google_folder.infra_folder.name"
  workload_deployers = [
    { name = "shop" group = "gcp-project-admins@example.com" },
  ]
}
```

```bash
satz add-pack C0example.satz use_workload_deployer
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`add-pack` activates the pack's line (the `params` above bind what it asks:
`deployer_project_folder` is a question, which `satz interview` asks when it is not
bound). The apply creates, for the team `shop`:

| what | the team's |
|---|---|
| Google project `acme-deploy-shop` | where its plans are billed and its state lives, under the infrastructure folder |
| service account `svc-deploy-shop@acme-deploy-shop.iam.gserviceaccount.com` | what its pipelines run as |
| bucket `acme-deploy-shop-state` | where its states live; the account writes it |
| roles on the `Workloads` folder | folder and project creation, their IAM, their APIs (`deployer_folder_roles`); further parents through `workload_deployer_folders` |
| `roles/billing.user` on the billing account | linking a new project |
| `roles/browser` on the organisation | the one organisation-level right: the folder lookup of step 1 lists the organisation's folders |
| impersonation for `gcp-project-admins@example.com` | the group's members may become the account, and nothing else |

Every transpile of the organisation writes the current `interfaces/`; the team takes
`common/` from there. Everything the team does from here runs as `svc-deploy-shop`.

## 3. The template

[`examples/acme/project-template/satz/template.satz`](../examples/acme/project-template/satz/template.satz)
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
use "vendor/common/core/satz/interface.satz"
```

**The interface file**, from the folder the team took, read as `${{interface.<export>}}`.

```
google_project {
  project {
    name            = "{application}-{stage}"
    project_id      = "${{interface.customer_shortname}}-{application}-{stage}"
    folder_id       = "${{interface.workload_folder}}"
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
the looked-up workload folder, a label from an input, and three grants to groups named
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
  vendor/common/              interfaces/common/ of the organisation, taken whole
  hcl/                        written by satz
```

[`examples/acme/shop-dev/config.toml`](../examples/acme/shop-dev/config.toml) is the
configuration (`include_dirs = [".", "satz", "../project-template/satz"]`; `satz
update-schema` fetches `schemas/`), and the estate is the whole instance:

```
estate shop_dev

params {
  application = "shop"
  stage       = "dev"
  cost_centre = "4711"
}

use "template.satz"
```

A using document's `params { }` binds a template's inputs as it binds any param. Then:

```bash
cp -R <the organisation>/interfaces/common vendor/common
satz transpile shop_dev.satz
```

`hcl/` holds:

| file | what |
|---|---|
| `interface.tf` | the organisation's values, in two locals: `interface_static` with the ones satz knows, which the providers read, and `interface`, which merges the looked-up ones in — `workload_folder = data.google_active_folder.workload_folder.name` — followed by the `data` block itself, with the permission it needs in a comment above it |
| `main.tf` | the project: `project_id = "${local.interface.customer_shortname}-${var.application}-${var.stage}"`, `folder_id = local.interface.workload_folder`, `"cost_centre" = var.cost_centre`, the three grants. No value of the organisation stands in it |
| `providers.tf` | the `google` provider on the team's project, impersonating the team's account, its region from `local.interface_static.default_region`; the `gcs` backend on the team's bucket; one alias per project the estate declares |
| `variables.tf` | `variable "application"`, `"stage"`, `"cost_centre"`, each described by the comment above its input. The template's params follow them, as every estate's do, and no resource reads those |
| `terraform.tfvars` | `application = "shop"`, `stage = "dev"`, `cost_centre = "4711"` — the bound values, with the params after them |

Plan and apply as the team's account — a member of `gcp-project-admins@example.com`,
logged in with `gcloud`, is who the impersonation runs for:

```bash
satz transpile shop_dev.satz --plan
satz transpile shop_dev.satz --apply
```

The plan reads the workload folder live (`roles/browser`), creates `acme-shop-dev` under
it (`roles/resourcemanager.projectCreator` on the folder), links the billing account
(`roles/billing.user`) and sets its grants (`roles/resourcemanager.projectIamAdmin`). A
second project is a second thin estate — `shop-test/satz/shop_test.satz` with
`stage = "test"` — or, for a run, `satz transpile shop_dev.satz --input stage=test`.

When the organisation changes, the team takes `interfaces/common/` again, reads
`CHANGES.md` beside `core/README.md` for what moved, and transpiles: `interface.tf`
follows, `main.tf` does not change.

The compile holds the project to the interface's rules: a resource attached to the
organisation's objects where no attach point allows it, an authoritative grant or an
organisation policy on a node the organisation manages, or a resource the organisation
declares too, is refused at its line.

## 5. The HCL way

A team that works in OpenTofu alone never runs satz. Someone runs it once — the
platform team, or the team on a machine that has it — and hands over `hcl/`:

```bash
satz --config examples/acme/project-template transpile template.satz
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
| `interface.tf` | takes it from the organisation's `interfaces/common/` after every change of the organisation — or writes the organisation's values into it by hand, which is what pointing the same `main.tf` at another organisation means |
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

## What the two ways share

| | the Satz way | the HCL way |
|---|---|---|
| the truth about the organisation | `vendor/common/core/satz/interface.satz` | `interface.tf` |
| the values that change per instance | `params { }` of a thin estate, or `--input` | `terraform.tfvars`, or `-var` |
| the identity | the deployer account, in `params` of the template | the deployer account, in `providers.tf` |
| the state | the team's bucket, `gcs` backend | the same |
| the organisation's state | never read | never read |
| held to the interface by | the compile | `satz check-consumer` |
| after the organisation changes | take `interfaces/common/` again, transpile | take `interface.tf` again |

One template, one interface, one identity per team; what differs is who runs the
compiler.
