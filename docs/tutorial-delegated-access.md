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
[Projects beside the organisation](tutorial-projects.md) explains the packs and shows
every file; the [library](../presets/README.md#delegationsatz) lists every field. The
example is Acme Corp. of [`docs/examples.md`](examples.md) and its teams `shop` and
`payments`.

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

## What a delegation is

A **delegation** is one identity for one team, and the places that identity works in. It
has two parts, and each is an entry in a list of the organisation's estate.

| part | list, pack | one entry creates |
|---|---|---|
| the base: the identity | `delegations`, [`presets/delegation.satz`](../presets/README.md#delegationsatz) | an infrastructure project `acme-deploy-<name>`, in it the account `svc-deploy-<name>` and the state bucket `acme-deploy-<name>-state`, and the right of one group to act as the account |
| a target: a folder | `delegated_folders`, [`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz) | a folder the account may create folders and projects in |
| a target: a project | `delegated_projects`, [`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz) | a Google project `acme-<name>-001` the account may fill |

Every target entry names its base: `delegation = "<name>"`. One base has any number of
targets, of either kind.

**Every delegation has an infrastructure project of its own.** The account and the state
bucket stand in it, never in a project the team fills. A delegated project holds no
account, no state bucket and no group: it is worked by the account of the delegation its
entry names. Two things follow for the team:

- **The team's calls are billed to the infrastructure project.** Every call the team's
  plan makes to a Google API is counted against `acme-deploy-<name>`, so the API has to
  be enabled there. The team does not own that project. An API its calls need beyond the
  standard six is asked for (step 1) and written into `apis` of the delegation's entry.
- **An API a resource needs inside a delegated project the team enables there itself**,
  in its own estate: its account holds that project's services.

The three cases of this page:

| the workload team | the organisation writes | what the team gets |
|---|---|---|
| rolls out its own projects, as many as it needs | one entry in `delegations`, one in `delegated_folders` per folder | an account, a state bucket, and one or more folders it may create projects in |
| works in one project it is given | one entry in `delegations`, one in `delegated_projects` | an account, a state bucket, and the project |
| works in several projects with one account | one entry in `delegations`, one in `delegated_projects` per project | an account, a state bucket, and the projects |

A delegation and one of its targets may share a name: the project `payments` of the
delegation `payments`. Two targets may not: a folder and a project of one name would be
one hand-over folder.

## 1. Workload team: ask

Bring this to the conversation. Everything on it ends up in the organisation's estate,
so the clearer it is, the fewer rounds.

| what | example | why it is asked |
|---|---|---|
| a name for the delegation | `shop` | it names the account `svc-deploy-shop`, its infrastructure project and its bucket: lowercase letters, digits and `-`, starting with a letter |
| the group that may act as the account | `gcp-shop-admins@example.com` | its members run the plans; it must exist in the directory |
| the folders it works in, or the projects it needs | one folder `shop` under `Workloads` | the account gets its rights there and nowhere else |
| APIs its calls need beyond the standard six | `pubsub.googleapis.com`, `billingbudgets.googleapis.com` | the calls are billed to the delegation's infrastructure project, and the team cannot enable an API on a project it does not own |
| a budget for its own project or folder | `budgets = true` | the team cannot grant itself anything on the billing account; this gives it budgets for its own scope and nothing else |
| billing roles beyond linking a project | rarely: `roles/billing.costsManager` covers every project's budgets and costs | prefer `budgets = true` above |

The standard six APIs are `cloudbilling`, `cloudresourcemanager`, `iam`, `iamcredentials`,
`serviceusage` and `storage`, each `.googleapis.com`. Every resource type the team's
estate declares calls one API; that API is one of the six, or it is asked for.

If the team does not know the last three yet, it asks without them. Step 7 names exactly
what is missing, and the second diagram below is the round that adds it.

## 2. Organisation team: decide

- **Which targets**, by the table above: folders, projects, or both.
- **Where the folders or projects go.** A folder is created under the workload folder
  unless the entry names another parent. A delegation's folders need no common root.
  Every delegated project is created in one folder, `delegated_projects_folder`.
- **Which group.** The base grants that group the right to act as the account, and to
  read the delegation's infrastructure project. Membership stays in the directory.
- **Whether the extra APIs and billing roles are justified.** They are the only things
  that go beyond the packs' defaults.
- **What should hold inside a folder.** The account may create projects and set their
  access there. Limits inside the folder are organisation policies the organisation's
  estate sets on that folder.

What the account will and will not be able to do is in the packs' reference:
[`delegation`](../presets/README.md#delegationsatz),
[`delegated-folders`](../presets/README.md#delegated-folderssatz),
[`delegated-projects`](../presets/README.md#delegated-projectssatz).

## 3. Organisation team: write the entries

Once per estate, the library copy and the packs' switches:

```bash
satz get-presets
satz add-preset C0example.satz use_delegation
satz add-preset C0example.satz use_delegated_folders      # for folders
satz add-preset C0example.satz use_delegated_projects     # for projects
satz interview C0example.satz
```

`add-preset` binds the switch and makes the pack's `use` line active. It refuses while a
pack it needs is off, and names that pack. The switches open two questions, which
`satz interview` asks: `delegation_project_folder`, the folder the infrastructure
projects are created in, and `delegated_projects_folder`, the folder the delegated
projects are created in.

Then the entries. `satz add-entry <estate> <list> <field>=<value>…` adds one entry to a
list: it checks the fields and their patterns against the pack's declaration, refuses a
name the list already holds, writes the entry into `params { }` of the estate, and keeps
it only when the estate compiles. Below each command is what it writes, for a reader who
edits the file instead. Pick the case.

**A team with folders.** The delegation `shop` and one folder:

```bash
satz add-entry C0example.satz delegations name=shop owner_group=gcp-shop-admins@example.com
satz add-entry C0example.satz delegated_folders name=shop delegation=shop display_name=Shop
```

```
params {
  delegations       = [{ name = "shop" owner_group = "gcp-shop-admins@example.com" }]
  delegated_folders = [{ name = "shop" delegation = "shop" display_name = "Shop" }]
}
```

A second folder for the same account is a second entry of `delegated_folders` that names
`shop`; `parent=google_folder.<label>.name` puts it under another folder the estate
declares.

**A team with one project.** The delegation `payments` and the project `payments`:

```bash
satz add-entry C0example.satz delegations name=payments owner_group=payments-owners@example.com
satz add-entry C0example.satz delegated_projects name=payments delegation=payments
```

```
params {
  delegations        = [{ name = "payments" owner_group = "payments-owners@example.com" }]
  delegated_projects = [{ name = "payments" delegation = "payments" }]
}
```

**One account working in several projects.** One more entry of `delegated_projects` per
project, each naming the same delegation — here `shop`, the delegation of the first case:

```bash
satz add-entry C0example.satz delegated_projects name=shop-backend delegation=shop
satz add-entry C0example.satz delegated_projects name=shop-frontend delegation=shop
```

```
params {
  delegated_projects = [
    { name = "shop-backend" delegation = "shop" },
    { name = "shop-frontend" delegation = "shop" },
  ]
}
```

An entry that names a delegation no entry of `delegations` declares is refused by the
compile, which lists the accounts that exist. A delegation needs no folder: an account
that works in projects only has entries in `delegated_projects` and none in
`delegated_folders`.

**More APIs or billing roles** are fields of the delegation's entry. A list's values are
separated by commas, and a list that is stated is stated whole — the six standard APIs
and the additions:

```bash
satz add-entry C0example.satz delegations name=shop owner_group=gcp-shop-admins@example.com \
  apis=cloudbilling.googleapis.com,cloudresourcemanager.googleapis.com,iam.googleapis.com,iamcredentials.googleapis.com,serviceusage.googleapis.com,storage.googleapis.com,pubsub.googleapis.com \
  billing_roles=roles/billing.user,roles/billing.costsManager
```

```
  delegations = [
    {
      name        = "shop"
      owner_group = "gcp-shop-admins@example.com"
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

## 4. Organisation team: plan, read, apply

```bash
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

Read the plan before the apply. For the delegation `shop` with one folder it shows:

| in the plan | what it is |
|---|---|
| a Google project `acme-deploy-shop`, a service account `svc-deploy-shop`, a bucket `acme-deploy-shop-state` | the delegation's infrastructure project: where the team's calls are billed, what its plans run as, where its state lives |
| a folder `Shop` | where the team builds |
| six role grants on the folder `Shop`, and none on `Workloads` | project and folder creation, project access, APIs — on its own folder only |
| one grant on the billing account, one on the organisation (`roles/browser`) | linking a new project; looking a folder up by its name |
| grants for the group on the account and on the infrastructure project | the members may act as the account, and read its project |

For the delegation `payments` with one project the first, fourth and fifth rows are the
same with `payments` in the names, and the target is:

| in the plan | what it is |
|---|---|
| a Google project `acme-payments-001` | where the team builds; no account and no bucket stand in it |
| two role grants on `acme-payments-001` for `svc-deploy-payments` | the project's access and its services — on that project only |

A grant on a folder the teams share, or a role that sets a folder's access, is not in
that plan. If one is, stop: something other than the packs wrote it.

## 5. Organisation team: hand over

Every transpile writes `interfaces/outgoing/` beside the estate. Hand over these folders, whole:

| case | folders |
|---|---|
| a team with folders | `interfaces/outgoing/delegation-shop/` and one per folder, `interfaces/outgoing/shop/` |
| a team with one project | `interfaces/outgoing/delegation-payments/` and `interfaces/outgoing/payments/` |
| one account working in several projects | `interfaces/outgoing/delegation-shop/` and one per project, `interfaces/outgoing/shop-backend/` and `interfaces/outgoing/shop-frontend/` |

`delegation-<name>/` is the delegation's own folder: the account, the infrastructure
project, the state bucket, the group. Each other folder is one target. A copy is enough;
so is read access to the organisation's repository at a tag.

The two halves of `interfaces/` are the two directions: `outgoing/` is what an estate
publishes, removed and written whole by every transpile; `incoming/` is what it was
given, and no transpile writes there. The organisation's
`interfaces/outgoing/payments/` is the team's `interfaces/incoming/payments/`. The team
needs nothing else of the organisation's: not its estate file, not its library, not its
state.

**The folders explain themselves.** Each has a `README.md` that satz writes for the team
that receives it. In the delegation's own folder, "Delegated access: what you were given"
says which account the team's estate runs as, which group's members may act as it and
what that group holds on the account, which project its calls are billed to, where the
state lives, which billing account a new project is linked to, what the account holds on
the billing account, and which APIs are on. In a target's folder it says which folder or
project to build in and what the account holds there. Under "What to do with it" each
lists the team's next steps, and the first is the command of step 6. Nothing else has to
be written down for the hand-over.

Say one thing with it: the folders are taken again after every change of the
organisation.

## 6. Workload team: set up the estate

In the team's own repository, which is empty, with the folders of step 5 within reach:

```bash
satz init --from <the organisation>/interfaces/outgoing/delegation-shop <the organisation>/interfaces/outgoing/shop
```

`init --from` takes the delegation's own folder and the folder of one target. Exactly one
of the folders is a delegation's own: an estate runs as one account. It copies each
folder into `interfaces/incoming/` unless it stands there already, fetches the provider
schemas, and writes the team's estate under the delegation's name:

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
infrastructure project. The team adds what it builds:

```
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

Everything a team with a folder creates stands under `${{interface.delegated_folder}}`.

**A team with one project** names its two folders the same way:

```bash
satz init --from <the organisation>/interfaces/outgoing/delegation-payments <the organisation>/interfaces/outgoing/payments
```

Its resources name the project as `project = "${{interface.project_id}}"`, and an API a
resource needs inside the project is a `google_project_service` there.

**One estate works in one target.** An account that works in several projects, or in
several folders, has one estate per target, and `--name` names each. They can share a
repository: a folder that stands in `interfaces/incoming/` already is left as it is.

```bash
satz init --from <the organisation>/interfaces/outgoing/delegation-shop <the organisation>/interfaces/outgoing/shop-backend --name shop-backend
satz init --from <the organisation>/interfaces/outgoing/delegation-shop <the organisation>/interfaces/outgoing/shop-frontend --name shop-frontend
```

These write `satz/shop_backend.satz` and `satz/shop_frontend.satz`. Both run as
`svc-deploy-shop`, and each keeps its own state in the delegation's bucket, under
`shop_backend/state` and `shop_frontend/state`.

## 7. Workload team: compile

```bash
satz transpile shop.satz --check
```

The compile judges the estate against what was delegated. It is quiet when everything
the estate needs was handed over. Otherwise each line says who closes it:

| the line says | who acts | what to do |
|---|---|---|
| `… on the delegated folders` | organisation team | ask for the role; it is added to the folder roles, `delegated_folders_roles` |
| `… on the billing account` | organisation team | ask for it in `billing_roles` of the delegation's entry |
| `… API(s) … not enabled on …, the project its calls are billed to` | organisation team | ask for it in `apis` of the delegation's entry: the project is the delegation's infrastructure project |
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
writes no grant for a team's estate. `--apply` is refused while an API is missing.

A note `presets/pack-graph.json is not here` is about the organisation's pack library,
which a team's repository does not hold. It asks nothing of the team.

**Workload packs.** What a team builds can come as a pack: `satz packs --format text --out -`
lists the ones the satz repository holds and says what each needs — a delegated folder or
a delegated project — and `satz get-pack <name>` fetches one. `project-budget` is such a
pack: with `budgets = true` on the project's entry it creates a budget for the delegated
project, after two questions (the amount, the currency) and an address to tell.

```bash
satz get-pack project-budget
```

```
use "packs/project-budget.satz"
```

**A pack may switch on the APIs it adds.** The organisation switched on the standard ones
(the hand-over folder lists them); a pack declares the others in the project itself, and
the account's role on its project allows it. Declaring one the organisation already
switched on is reported: two estates would own one switch.

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
organisation team edits the delegation's entry — an API, a billing role — or adds one
more folder or project with `satz add-entry`, plans, applies, and hands the folders over
again. The team replaces its copies under `interfaces/incoming/` and compiles: the line
is gone. A further target is a further folder to take and a further estate (step 6).

**B. The organisation changed.** A transpile that changes a published value writes
`CHANGES.md` into the interface folder, with what a reader does about it. The team takes
the folders again, reads it, compiles and plans. The organisation team tells the teams, or
switches on the change notice that does
([`presets/interface-notice.satz`](../presets/README.md#interface-noticesatz)).

**Taking a delegation away** is the organisation team's, and it is not an edit like the
others: removing an entry destroys what the pack created for it — for a delegation the
infrastructure project with the account and the state bucket with every state in it, for
a target the project, or the folder once it is empty. The team removes what it built
first.

## The written route

Steps 1 to 3 can be done in writing instead. The workload team puts its entries into a
small file of its own, checks it against the organisation's estate, and sends it as a pull
request to the organisation's repository; merging it is the decision of step 2.

```
pack shop_request version "1.0"

params {
  contributes_delegations       = [{ name = "shop" owner_group = "gcp-shop-admins@example.com" }]
  contributes_delegated_folders = [{ name = "shop" delegation = "shop" }]
}
```

```bash
satz check-request shop_request.satz C0example.satz
```

A project is asked for the same way, as `contributes_delegated_projects`. Nothing takes
effect before the organisation team merges and applies. Steps 4 to 9 are the same.

## Who does what

| | organisation team | workload team |
|---|---|---|
| decides what an account can reach | yes | asks |
| creates the delegation's infrastructure project, its account and bucket, and the folders or projects | yes, with the packs | no |
| names the group that may act as the account | yes | proposes it |
| puts people into that group | whoever administers the directory | whoever administers the directory |
| hands over | the interface folders, after every change; their READMEs are the instructions | — |
| writes the team's estate | no | yes, with `satz init --from` |
| creates projects in a delegated folder | no | yes |
| grants roles on a project the team was given | no | yes, in its own estate |
| adds an API on the infrastructure project, a billing role, a folder, a project, `budgets = true` | yes, on request | asks |
| switches on an API a resource needs, in a project the team was given | no | yes, in its own estate |
| removes a delegation | yes, after the team emptied it | empties it |
