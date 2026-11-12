# Delegated access: the procedure for both sides

**Delegated access** gives a team access to a part of the organisation's resources — a
folder, a project — with scoped permissions: an account of its own that holds roles there
and nowhere else, and cannot widen them.

Two teams meet here. The **organisation team** owns the organisation's estate and may
change it. The **workload team** builds something beside it — in its own repository, with
its own state — and must never run as the organisation's account. This page is the
process between them, step by step and by side: what the workload team asks for, what the
organisation team decides and creates, what is handed over, and what each does next.

It is a procedure to follow, not a walk through the language.
[Projects beside the organisation](tutorial-projects.md) explains the two packs and shows
every file; the [library](../presets/README.md#folder-workloadsatz) lists every field. The
example is Acme Corp. of [`docs/examples.md`](examples.md) and its team `shop`.

## The process

![Two lanes, the organisation team and the workload team, and nine steps: the workload team asks; the organisation team decides, writes the entries, plans and applies, and hands over the interface folders; the workload team sets up its estate, compiles, joins the group and logs in, then plans and applies as its own account. A dashed arrow leads from the compile back to the decision: something is not delegated, ask for it](delegated-access-first.svg)

A dashed box is a step people do: a conversation, a decision, a hand-over. A solid box is
a step done with satz. The numbers are the sections below.

Three things are true throughout:

- **Nothing secret changes hands.** No key, no password. What is handed over is a few
  folders of generated files that hold names and what was granted.
- **The right to act as the account comes from a group.** The organisation's estate names
  the group; who is in it is decided in the directory, not in satz.
- **The account cannot widen what it was given.** What it lacks, the team asks for.

## Which pack

Access is delegated by one of two packs. The organisation team picks by one question: who creates the Google
projects?

| the workload team | pack | what it gets |
|---|---|---|
| rolls out its own projects, as many as it needs | `presets/folder-workload.satz` | an account, a state bucket, and one or more folders it may create projects in |
| works in a project it is given | `presets/project-workload.satz` | the project, with an account and a state bucket of its own |
| has an account already, and needs one more project | `presets/project-workload.satz`, the shared list | the project; its existing account gets the rights on it |

## 1. Workload team: ask

Bring this to the conversation. Everything on it ends up in the organisation's estate,
so the clearer it is, the fewer rounds.

| what | example | why it is asked |
|---|---|---|
| a name for the workload | `shop` | it names the account `svc-deploy-shop`, its project and its bucket: lowercase letters, digits and `-` |
| the group that may act as the account | `gcp-shop-admins@example.com` | its members run the plans; it must exist in the directory |
| the folders it works in, or the projects it needs | one folder `shop` under `Workloads` | the account gets its rights there and nowhere else |
| APIs beyond the standard ones, on the project its calls are billed to | `billingbudgets.googleapis.com` | the team cannot enable an API on a project it does not own |
| billing roles beyond linking a project | `roles/billing.costsManager`, for budgets | the team cannot grant itself anything on the billing account |

If the team does not know the last two yet, it asks without them. Step 7 names exactly
what is missing, and the second diagram below is the round that adds it.

## 2. Organisation team: decide

- **Which pack**, by the table above.
- **Where the folders or projects go.** A folder is created under the workload folder
  unless the entry names another parent. A workload's folders need no common root.
- **Which group.** The pack grants that group the right to act as the account, and to
  read the account's project. Membership stays in the directory.
- **Whether the extra APIs and billing roles are justified.** They are the only things
  that go beyond the pack's defaults.
- **What should hold inside the folder.** The account may create projects and set their
  access there. Limits inside the folder are organisation policies the organisation's
  estate sets on that folder.

What the account will and will not be able to do is in the pack's reference:
[`folder-workload`](../presets/README.md#folder-workloadsatz),
[`project-workload`](../presets/README.md#project-workloadsatz).

## 3. Organisation team: write the entries

Once per estate, the pack's library copy and its switch:

```bash
satz get-presets
satz add-preset C0example.satz use_folder_workload     # or use_project_workload
```

Then the entries, in `params { }` of the organisation's estate. Pick the case.

**A team that rolls out its own projects** (`folder-workload`):

```
params {
  use_folder_workload            = true
  folder_workload_project_folder = "google_folder.infra_folder.name"
  folder_workloads               = [{ name = "shop" group = "gcp-shop-admins@example.com" }]
  folder_workload_folders        = [{ name = "shop" workload = "shop" display_name = "Shop" }]
}
```

A second folder for the same account is a second entry that names `shop`. More APIs or
billing roles are fields of the workload's entry, and a list that is stated is stated
whole — the six standard APIs and the additions:

```
  folder_workloads = [
    {
      name  = "shop"
      group = "gcp-shop-admins@example.com"
      apis = [
        "cloudbilling.googleapis.com",
        "cloudresourcemanager.googleapis.com",
        "iam.googleapis.com",
        "iamcredentials.googleapis.com",
        "serviceusage.googleapis.com",
        "storage.googleapis.com",
        "billingbudgets.googleapis.com",
      ]
      billing_roles = ["roles/billing.user", "roles/billing.costsManager"]
    },
  ]
```

**A team that works in a project it is given** (`project-workload`):

```
params {
  use_project_workload      = true
  project_workload_folder   = "google_folder.workload_folder.name"
  project_workload_projects = [{ name = "payments" owner_group = "payments-owners@example.com" }]
}
```

**A team that has an account and needs a project** (`project-workload`, the shared list;
`shop` is the workload of the first case):

```
params {
  project_workload_shared_projects = [{ name = "shop-backend" workload = "shop" }]
}
```

## 4. Organisation team: plan, read, apply

```bash
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

Read the plan before the apply. For the workload `shop` with one folder it shows:

| in the plan | what it is |
|---|---|
| a Google project `acme-deploy-shop`, a service account `svc-deploy-shop`, a bucket `acme-deploy-shop-state` | where the team's plans are billed, what they run as, where its state lives |
| a folder `Shop` | where the team builds |
| six role grants on the folder `Shop`, and none on `Workloads` | project and folder creation, project access, APIs — on its own folder only |
| one grant on the billing account, one on the organisation (`roles/browser`) | linking a new project; looking a folder up by its name |
| grants for the group on the account and on its project | the members may act as the account, and read its project |

A grant on a folder the teams share, or a role that sets a folder's access, is not in
that plan. If one is, stop: something other than the pack wrote it.

## 5. Organisation team: hand over

Every transpile writes `interfaces/` beside the estate. Hand over these folders, whole:

| case | folders |
|---|---|
| a team with folders | `interfaces/workload-shop/` and one per folder, `interfaces/shop/` |
| a project with its own account | `interfaces/payments/` |
| a project for an existing account | `interfaces/shop-backend/`; the team has `interfaces/workload-shop/` already |

A copy is enough; so is read access to the organisation's repository at a tag.

**The folders explain themselves.** Each has a `README.md` that satz writes for the team
that receives it. Under "Delegated access: what you were given" it says which account the team's estate
runs as, which group's members may act as it and what that group holds on the account,
where the state lives, which billing account a new project is linked to, what the account
holds on its folders, its project and the billing account, and which APIs are on. Under
"What to do with it" it lists the team's next steps with the values filled in. Nothing
else has to be written down for the hand-over.

Say one thing with it: the folders are taken again after every change of the
organisation.

## 6. Workload team: set up the estate

In the team's own repository:

```text
shop/
  config.toml
  satz/shop.satz
  vendor/workload-shop/       from the organisation, whole
  vendor/shop/                from the organisation, whole
```

`config.toml`:

```toml
yaml_dir = "satz"
hcl_dir = "hcl"
include_dirs = [".", "satz"]
schema_dir = "schemas"
tf_tool = "tofu"
google_providers = ["google", "google-beta"]
provider_version = "7.14.1"
auto_explode = ["google_project_service", ".*_iam_member"]
validation_level = "warn"
```

```bash
satz update-schema        # fetches the provider schemas into schemas/, once
```

`satz/shop.satz`. The three names at the top are written out in the README of
`vendor/workload-shop/`, under "What to do with it":

```
estate shop

params {
  infra_project_name = "acme-deploy-shop"
  svc_iac_account    = "svc-deploy-shop"
  infra_bucket_name  = "acme-deploy-shop-state"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
}

// what the organisation handed over: the account with what was delegated, and the folder
use "vendor/workload-shop/workload-shop/satz/interface.satz"
use "vendor/shop/shop/satz/interface.satz"

terraform {
  backend {
    gcs {
      bucket = infra_bucket_name
      prefix = "shop/state"
    }
  }
}

providers {
  "google" {
    alias                 = "google"
    project               = infra_project_name
    region                = "${{interface.default_region}}"
    user_project_override = true
    billing_project       = infra_project_name
  }
}

google_project {
  shop_dev {
    name            = "shop-dev"
    project_id      = "${{interface.customer_shortname}}-shop-dev"
    folder_id       = "${{interface.delegated_folder}}"
    billing_account = "${{interface.billing_account}}"
    project_service = ["cloudresourcemanager.googleapis.com"]
  }
}
```

Everything the team creates stands under `${{interface.delegated_folder}}`. One estate
works in one folder: a second folder of the same workload is a second estate that runs as
the same account.

A team with **a project of its own account** does not write this by hand:

```bash
satz init --project payments --interface vendor/payments/payments/satz/interface.satz
```

Its resources name the project as `project = "${{interface.project_id}}"`. A team with
**a project for an existing account** uses both `use` lines, of `workload-shop` and of the
project's folder, and the same `project = …`.

## 7. Workload team: compile

```bash
satz transpile shop.satz --check
```

The compile judges the estate against what was delegated. It is quiet when everything
the estate needs was handed over. Otherwise each line says who closes it:

| the line says | who acts | what to do |
|---|---|---|
| `… on the workload's folders` | organisation team | ask for the role; it is added to the pack's folder roles |
| `… on the billing account` | organisation team | ask for it in `billing_roles` of the workload's entry |
| `… API(s) … not enabled on …, the project its calls are billed to` | organisation team | ask for it in `apis` of the entry that created that project |
| `… on the project — to grant in this estate` | workload team | write the grant in this estate (below) |
| `… outside the project` | nobody: this account does not get it | build it another way, or ask for a folder |

The one gap a team closes itself is a role on a project it was given, because its account
holds that project's access control:

```
google_project_iam_member {
  project = "${{interface.project_id}}"
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/storage.admin",
  ]
}
```

`satz update-prerequisites shop.satz --report-only` prints the same as a list, with what
was delegated. Nothing at the organisation is ever proposed, and `satz update-prerequisites`
writes no grant for a workload's estate. `--apply` is refused while an API is missing.

A note `presets/pack-graph.json is not here` is about the organisation's pack library,
which a workload's repository does not hold. It asks nothing of the team.

## 8. Workload team: be in the group, and log in

Whoever runs the plan is a member of the group from step 1 — added in the directory, by
whoever administers it — and logged in:

```bash
gcloud auth application-default login
satz whoami shop.satz
```

`whoami` says which identity the run uses. With `deployment_mode = "cloud"` every provider
and the state backend act as `svc-deploy-shop`.

A workload that creates Google groups needs one more thing no satz file grants: the Groups
Admin role for its account, assigned in the Google admin console by a directory
administrator.

## 9. Workload team: plan, read, apply

```bash
satz transpile shop.satz --plan
satz transpile shop.satz --apply
```

The plan looks the folder up live, creates `acme-shop-dev` in it and links the billing
account. The team's account is the owner of every project it creates.

## Afterwards

![Two lanes and two cases. A, the team needs more: the compile names the gap, the team asks for it, the organisation team edits the entry, plans and applies and hands the folders over again, the team takes them again and compiles. B, the organisation changed: an apply changes a published value and satz writes CHANGES.md into the interface folder, the organisation team tells the teams or lets the change notice do it, the team takes the folders again, reads CHANGES.md, compiles and plans](delegated-access-later.svg)

**A. The team needs more.** The compile says what, and which list takes it (step 7). The
organisation team edits the workload's entry — an API, a billing role, one more folder or
project — plans, applies, and hands the same folders over again. The team replaces its
copies and compiles: the line is gone.

**B. The organisation changed.** A transpile that changes a published value writes
`CHANGES.md` into the interface folder, with what a reader does about it. The team takes
the folders again, reads it, compiles and plans. The organisation team tells the teams, or
switches on the change notice that does
([`presets/interface-notice.satz`](../presets/README.md#interface-noticesatz)).

**Taking a workload away** is the organisation team's, and it is not an edit like the
others: removing an entry destroys what the pack created for it — the account, the state
bucket with every state in it, and a folder once it is empty. The team removes what it
built first.

## The written route

Steps 1 to 3 can be done in writing instead. The workload team puts its entries into a
small file of its own, checks it against the organisation's estate, and sends it as a pull
request to the organisation's repository; merging it is the decision of step 2.

```
pack shop_request version "1.0"

params {
  contributes_folder_workloads        = [{ name = "shop" group = "gcp-shop-admins@example.com" }]
  contributes_folder_workload_folders = [{ name = "shop" workload = "shop" }]
}
```

```bash
satz check-request shop_request.satz C0example.satz
```

Nothing takes effect before the organisation team merges and applies. Steps 4 to 9 are
the same.

## Who does what

| | organisation team | workload team |
|---|---|---|
| decides what an account can reach | yes | asks |
| creates the account, its bucket, its folders or project | yes, with the pack | no |
| names the group that may act as the account | yes | proposes it |
| puts people into that group | whoever administers the directory | whoever administers the directory |
| hands over | the interface folders, after every change; their READMEs are the instructions | — |
| creates projects in a workload's folder | no | yes |
| grants roles on a project the team was given | no | yes, in its own estate |
| adds an API, a billing role, a folder | yes, on request | asks |
| removes a workload | yes, after the team emptied it | empties it |
