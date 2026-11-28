# Delegated access: the procedure for both sides

**Delegated access** gives a team access to a part of the organization's resources — a
project, a folder — with an estate account of its own that holds roles there and nowhere
else, and cannot widen them. The team builds its workload from its own repository, with its
own state.

Two teams meet here. The **organization's team** owns the organization's estate and its
state, and may change them. The **workload's team** owns a workload repository, the
workload estate in it and that estate's state, and never runs as the organization's estate
account. This page is the procedure between them, step by step and by side: what each team
does, with the satz command and, where the app has one, the way through satz-studio.

Sections 1 to 7 are the route for a workload with needs of its own: it brings a pair of
packs. Section 8 is the simpler route for a workload that needs a plain project or folder
with the standard roles, which the preset library's ready-made delegation packs give.
[Workload estates beside the organization](tutorial-projects.md) explains the language
underneath; the [library](../presets/README.md#delegationsatz) lists every field. The
example is Acme Corp. of [`docs/examples.md`](examples.md) and a product catalog, the
workload `catalog`, used once under the name `books`.

## 1. The two sides and what passes between them

![The two sides of a delegated workload: the workload repository with the pack pair, the workload estate, its incoming interface folders and the estate account it runs as; the organization's repository with the fetched delegation pack, the named use in the organization's estate, its outgoing interface folders and the delegation; between them the delegation pack crossing through a pack source and the interface folders crossing back, copied whole; below, which state holds which resource in the cloud](delegated-workload-sides.svg)

| | the organization's estate | the workload estate |
|---|---|---|
| belongs to | the organization's team | the workload's team |
| stands in | the organization's repository | the workload repository |
| runs as | the organization's estate account, `svc-iac-001` | the delegation's estate account, `svc-deploy-books` |
| keeps its state in | the organization's state bucket | the delegation's state bucket, `acme-deploy-books-state`, under `books/state` |
| uses | the presets, and the **delegation pack** `catalog-delegation.satz` | the **workload pack** `catalog-workload.satz` |

**The delegation** is what joins them. It is one entry of the list param `delegations` of
[`presets/delegation.satz`](../presets/README.md#delegationsatz), in the organization's
estate, and creates the delegation's infrastructure project `acme-deploy-books`, in it the
estate account `svc-deploy-books` and the state bucket `acme-deploy-books-state`, and the
estate account user group, in the customer's directory, with its right to act as the estate
account. Its interface folder `delegation-books` carries all four to the workload's team.

**The base and the delegation pack divide the work.** What every delegation needs — the
estate account, its infrastructure project, its state bucket, its basic grants and the estate
account user group — is created by `presets/delegation.satz`. What one workload needs — the
resources it is given and the estate account's roles on them — is in the workload's delegation
pack, which the organization's team reviews. The organization's team applies the
organization's estate as the organization's estate account, `svc-iac-001`, and is not in the
delegation's group; it never acts as `svc-deploy-books`.

**The pack pair** is the workload's, written by the workload's team and named after the
workload:

- **The delegation pack**, `<workload>-delegation.satz`, stands in the organization's
  estate. It creates what the workload is given — here a project — grants the delegation's
  estate account its roles there and nowhere else, and adds its own entry to `delegations`.
  Its header says `multiple`, so the organization's estate uses it once per instance of the
  workload, each a [named use](language.md#named--one-pack-used-several-times):
  `use "packs/catalog-delegation.satz" named books`. Its interface `project` is then the
  interface folder `books-project`.
- **The workload pack**, `<workload>-workload.satz`, stands in the workload estate. It reads
  what was given with named reads, `${{interface.books-project.project_id}}`, and builds
  inside it as the delegation's estate account.

**What crosses** between the teams is two things. The delegation pack goes from the
workload's team to the organization's team, through a **pack source** (section 3). The
interface folders go back from the organization's team to the workload's team, copied whole
(section 5). No key, no password and no state crosses: the interface folders hold names and
what was granted, and the right to act as the estate account comes from the estate account
user group, which the organization's estate creates and whose members are decided in the
directory, not in satz.

**Whoever creates a resource keeps it in its state.** The project `acme-books-001`, the
estate account's grants on it and the delegation are created by the organization's estate
and stay in the organization's state, which creates, changes and deletes them. The bucket
and the topic the workload pack creates inside the project are the workload's state. No
resource is in both, and the workload estate imports nothing the organization created
([ADR 0101](adr/0101-a-delegation-is-two-packs-and-a-workload-estate-has-no-rights-check.md)).

The procedure, in order:

![A swimlane of the procedure, the organization's team on the left and the workload's team on the right, each step numbered by its section: the workload's team writes the pack pair, finds the roles and makes the delegation pack reachable; the delegation pack crosses to the organization's team, which fetches it, reviews it, writes the named use, plans and applies, and copies the interface folders; the folders cross back, and the workload's team takes them in, signs in, plans and applies as the delegation's estate account. A dashed arrow from the last step back to finding the roles is section 7: a role is missing, so a new version of the delegation pack](delegated-workload-sequence.svg)

A dashed box is a step people do: a review, a copy, a sign-in. A solid box is a step done
with satz, in the terminal or in satz-studio. The two crossings are the pack source and the
copy of the interface folders.

## 2. The workload's team writes the pack pair

**What to do:** write both packs in the workload repository's `packs/`, develop the workload
estate against stand-in interface folders, list the roles its estate account needs and put
them into the delegation pack, and review both packs.

```text
catalog/                            the workload repository
  packs/
    catalog-delegation.satz         the delegation pack, for the organization's estate
    catalog-workload.satz           the workload pack, for the workload estate
  config.toml
  satz/books.satz                   the workload estate
  interfaces/incoming/              the interface folders it reads
  test-organization/                a stand-in organization's estate
    config.toml
    satz/C0example.satz
```

**The delegation pack.** One named use is one catalog. It declares its values as params
with questions, so the organization's team answers them per use; adds the catalog's
delegation to the flat list `delegations` through `contributes_delegations`, so deleting
its `use` line removes the delegation with it; creates the project; grants the delegation's
estate account — `google_service_account.deploy_<name>`, written by
`presets/delegation.satz` — its roles on that project; and publishes the interface
`project`:

```
// The catalog's delegation pack. In the organization's estate it creates the project the
// catalog is built in, grants the delegation's estate account its roles there and adds the
// catalog's delegation to the organization's list `delegations`. The catalog's workload
// estate builds inside the project with the workload pack `catalog-workload.satz`.
//
//   use "packs/catalog-delegation.satz" named <name>
//
// One named use is one catalog; the estate binds `<name>.catalog_name` and
// `<name>.catalog_estate_account_user_group`.

pack catalog_delegation version "1.0" multiple

params {
  // The catalog's name: its project is `{customer_shortname}-<name>-001`, its delegation `<name>`.
  catalog_name                      = none
  // The group whose members may act as the delegation's estate account, which the
  // delegation creates.
  catalog_estate_account_user_group = none

  // The catalog's delegation, in the organization's list `delegations`.
  contributes_delegations = [
    {
      name                      = catalog_name
      estate_account_user_group = catalog_estate_account_user_group
      apis = [
        "cloudbilling.googleapis.com",
        "cloudresourcemanager.googleapis.com",
        "iam.googleapis.com",
        "iamcredentials.googleapis.com",
        "serviceusage.googleapis.com",
        "storage.googleapis.com",
        "pubsub.googleapis.com",
      ]
    },
  ]
}

question catalog_name {
  prompt   = "The catalog's name: lowercase letters and digits; its project is <customer_shortname>-<name>-001"
  why      = "The project's id is fixed at creation: another name is another project."
  reversal = recreate
  blast    = high
}

question catalog_estate_account_user_group {
  prompt   = "The address of the group whose members may act as the delegation's estate account"
  why      = "The delegation creates the group at this address; its members plan and apply the catalog's workload estate. Another address is another group."
  reversal = recreate
  blast    = low
}

google_project {
  catalog {
    name            = "catalog-{catalog_name}"
    project_id      = "{customer_shortname}-{catalog_name}-001"
    folder_id       = "google_folder.workload_folder.name"
    billing_account = billing_account_infra
    project_service = [
      "cloudresourcemanager.googleapis.com",
      "serviceusage.googleapis.com",
      "storage.googleapis.com",
    ]
  }
}

// The roles the catalog's resource types need, as `satz update-prerequisites --report-only`
// lists them in the workload estate: on the project and nowhere else.
google_project_iam_member {
  project = "${{google_project.catalog.project_id}}"
  "serviceAccount:${{google_service_account.deploy_{catalog_name}.email}}" = [
    "roles/serviceusage.serviceUsageAdmin",
    "roles/storage.admin",
    "roles/pubsub.editor",
  ]
}

interface "project" {
  export "project_id"              = "${{google_project.catalog.project_id}}" description "The project the catalog is built in"
  export "delegated_project_roles" = ["roles/serviceusage.serviceUsageAdmin", "roles/storage.admin", "roles/pubsub.editor"] description "What the delegation's estate account holds on the project"
}
```

The delegation's `apis` are those of its infrastructure project, which the workload
estate's calls are billed to: the six standard ones of `presets/delegation.satz` and every
API the workload's resource types call, here `pubsub.googleapis.com`.

**The workload pack.** It reads the project through a param whose default is the short
read, because a pack cannot know the name of the interface folder it is given; the workload
estate binds the param to its named read:

```
// The catalog's workload pack. It builds one product catalog inside the project the
// catalog's delegation pack (`catalog-delegation.satz`) creates in the organization's
// estate, as the delegation's estate account: the catalog's bucket and its topic.
//
//   use "packs/catalog-workload.satz"
//
// The workload estate binds `catalog_project_id` to its named read,
// `${{interface.<use>-project.project_id}}`.

pack catalog_workload version "1.0"

params {
  // The project the catalog is built in, as the delegation pack's interface folder publishes it.
  catalog_project_id = "${{interface.project_id}}"
}

google_project_service {
  catalog_pubsub {
    project = catalog_project_id
    service = "pubsub.googleapis.com"
  }
}

google_storage_bucket {
  catalog_items {
    project                     = catalog_project_id
    name                        = "{catalog_project_id}-items"
    location                    = default_region
    uniform_bucket_level_access = true
  }
}

google_pubsub_topic {
  catalog_changes {
    project = catalog_project_id
    name    = "catalog-changes"
  }
}
```

**Stand-in interface folders.** The workload estate is developed before the organization
has applied anything. `test-organization/` is an organization's estate of example values —
`satz init` and `satz get-presets` write one, as in [an organization from
scratch](tutorial-organization.md) — with `presets/delegation.satz` switched on (`satz
add-preset C0example.satz use_delegation`) and the line and values the organization's team
will write (section 4). Its `config.toml` names the repository's packs folder, `packs_dir =
"../packs"`, and the repository's top in `include_dirs`, `include_dirs = [".", "satz",
".."]`, so `use "packs/catalog-delegation.satz"` reaches the pack. Its transpile
writes the two interface folders, and `satz init --from` writes the workload estate from
them:

```bash
satz --config test-organization transpile C0example.satz
satz init --from test-organization/interfaces/outgoing/delegation-books test-organization/interfaces/outgoing/books-project
```

`init --from` copies both folders into `interfaces/incoming/` and writes `satz/books.satz`
(section 6 shows it). The team adds its workload pack and binds the param:

```
params {
  // …the lines init --from wrote…
  catalog_project_id = "${{interface.books-project.project_id}}"
}

use "packs/catalog-workload.satz"
```

The folder names come from the organization's estate: `delegation-<name>` from the
delegation's name, `<use>-<interface>` from the name of the use and the pack's interface.
The two teams agree on both names before the organization's team writes the line, so the
named reads find their folders.

**The roles.** satz checks no role in a workload estate: the estate account's roles are the
organization's to grant, through the delegation pack
([ADR 0101](adr/0101-a-delegation-is-two-packs-and-a-workload-estate-has-no-rights-check.md)).
`satz update-prerequisites --report-only` lists the roles the workload estate's resource
types need, as information:

```bash
satz transpile books.satz --check
satz update-prerequisites books.satz --report-only
```

```
estate account: svc-deploy-books@acme-deploy-books.iam.gserviceaccount.com
a workload estate: its account's roles are the organization's to grant, through the delegation pack that created what this estate builds in, and are not checked here
needed roles, for the organization's team:
  roles/pubsub.editor on the project — for google_pubsub_topic
  roles/serviceusage.serviceUsageAdmin on the project — for google_project_service
  roles/storage.admin on the project — for google_storage_bucket
needed APIs on acme-deploy-books, for the organization's team — its estate enables them there:
  pubsub.googleapis.com — for google_pubsub_topic
  serviceusage.googleapis.com — for google_project_service
  storage.googleapis.com — for google_storage_bucket
```

Each role goes into the delegation pack's `google_project_iam_member`, and each API into its
delegation's `apis`; the export `delegated_project_roles` repeats the roles, so the
interface folder's README tells the workload's team what the account holds. In
satz-studio, with the workload estate open, the Overview's **Needed roles** shows the same
list, and **Copy** puts it on the clipboard as text.

**The review.** `satz review-pack` holds a pack to the bar of the preset library: it parses,
it is formatted, its header says what it is, it carries no value shaped like private data,
every type it writes has a prerequisite row, and it transpiles. The workload pack is judged
inside the workload estate, the delegation pack inside the stand-in organization's estate:
without `--against`, the review's estate has no delegation for the pack's grant to name.

```bash
satz review-pack packs/catalog-workload.satz --against satz/books.satz --format text --out -
satz --config test-organization review-pack packs/catalog-delegation.satz --against test-organization/satz/C0example.satz --format text --out -
```

## 3. Making the delegation pack reachable: a pack source

**What to do:** put the delegation pack where the organization's team can fetch it with
`satz get-pack`. A **pack source** is a folder or a GitHub repository, with a name, declared
as a `[[pack_source]]` table in the `config.toml` of the estate that fetches from it
([workflows](workflows.md#the-pack-sources)). Three places serve:

| the pack source | what the organization's team declares | what it costs |
|---|---|---|
| **the workload repository's `packs/` folder**, as a folder source | `satz pack-source add catalog --dir ../catalog/packs` | the organization's team needs a checkout of the workload repository on disk, at a path its `config.toml` reaches; fits teams of one company that share their repositories |
| **a GitHub repository**: the workload repository itself, or a pack repository that holds the packs of several workloads | `satz pack-source add catalog --github acme/catalog [--ref v1] [--path packs]` | nothing on disk; satz reads the branch or tag at the commit it points to when the command runs. A private repository is read with this machine's GitHub sign-in (`gh auth token`, or `GITHUB_TOKEN`) |
| **another folder source the pack is stored in**, with `satz publish-pack`: a shared pack repository many teams store into, or a folder of the customer's own | `satz pack-source add shared --dir <that folder>` | one more step for the workload's team, and the pack in the workload repository becomes a fetched one (below) |

`publish-pack` copies one of the repository's own packs into a folder source, after the pack
clears `review-pack`'s bar inside an estate that uses it — for the delegation pack, the
stand-in organization's estate:

```bash
satz --config test-organization pack-source add shared --dir ../../shared-packs
satz --config test-organization publish-pack catalog-delegation --source shared
```

In satz-studio, with the stand-in organization's estate open: Packs, **Store in a pack
source…** on the pack's row. A GitHub source is written by a pull request to its
repository, never by `publish-pack`. From then on the pack has a row in `origin.toml`, as
if it were fetched from `shared`: the next change is made in the source's folder, or here
after `satz detach-pack catalog-delegation` and published again
([workflows](workflows.md#publishing-a-pack-to-a-folder-source)).

**What satz records.** `get-pack` and `publish-pack` write one row per pack into
`origin.toml` beside the packs: its id, the name of the pack source, for a GitHub source the
repository (`github`), its folder where that is not `packs` (`path`) and the commit the file
was read at, for a folder source the folder relative to the folder that holds `origin.toml`
(`folder`), the version the pack states and the SHA-256 of the file:

```toml
[[pack]]
id = "catalog-delegation"
source = "catalog"
folder = "../../catalog/packs"
version = "1.0"
sha256 = "…"
```

Every estate that shares one packs folder shares its `origin.toml`, so every one of them
declares each source a row names, under the same name, pointing at the same repository or
folder ([one packs folder, several estates](workflows.md#one-packs-folder-several-estates)).
In the workload repository above, `config.toml` and `test-organization/config.toml` both
declare `shared`, each with the path that reaches it from there.

## 4. The organization's team takes it in

**What to do:** add the pack source, fetch the pack, review it, switch the base on, write the
named use and its values, plan, read, apply.

**The source and the fetch.** Once per source, then once per pack:

```bash
satz pack-source add catalog --dir ../catalog/packs      # or --github acme/catalog
satz get-pack catalog-delegation --source catalog
```

```
installed catalog-delegation 1.0 from `catalog` (../catalog/packs) → packs/catalog-delegation.satz
    use "packs/catalog-delegation.satz" named <name>
```

`get-pack` writes the pack into `packs_dir` (`packs/` beside `config.toml` unless the
config names another) and its row into `origin.toml`, and prints the line that uses it: for
a pack whose header says `multiple`, the named line, with `<name>` for the estate to fill.
It is refused when `packs_dir` already holds a pack of that id, and when `packs_dir` is under
none of the config's `include_dirs`.

In satz-studio: Packs, the **Pack sources** card, **Manage**, **Add a source…** — a name,
then **GitHub repository** with the repository, an optional branch or tag and folder, or
**Folder** with **Choose a folder…** — and **Add**. Choosing the source in the same dialog
lists its packs against the open estate; **Fetch** on the pack's row runs `get-pack`.

**The review.** The delegation pack is what the organization's team signs for: every
resource it creates and every role it grants is created and granted by the organization's
estate account, in the organization's state. Read it as a change to the organization — the
project, where it is created, the roles and that each stands on the project alone, the
APIs of the delegation — and judge it inside the estate:

```bash
satz review-pack packs/catalog-delegation.satz --against satz/C0example.satz --format text --out -
```

**The base.** The pack's grants name the delegation's estate account, which
`presets/delegation.satz` writes, so that preset is switched on once per estate:

```bash
satz add-preset C0example.satz use_delegation
```

In satz-studio: Presets, `delegation` switched on.

**The named use and its values.** One line per instance, with the name the two teams agreed
on, and the use's values bound as `<use>.<param>` in the estate's `params`:

```
params {
  // …
  use_delegation                          = true
  books.catalog_name                      = "books"
  books.catalog_estate_account_user_group = "catalog-books-admins@example.com"
}

use "presets/delegation.satz" when use_delegation

use "packs/catalog-delegation.satz" named books
```

The values may instead be answered: `satz interview C0example.satz` asks each question of
the pack once per use, as `books.catalog_name`, under the heading
`books — packs/catalog-delegation.satz`, and writes the answer. In satz-studio the same
questions are in Decisions, under that heading; the `use … named` line is written in the
estate file itself, since **Add to the estate** on a pack's row writes a `use` line without
a name.

**Plan, read, apply.**

```bash
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

For the use `books` the plan adds:

| in the plan | what it is |
|---|---|
| `google_project.books__catalog`, the project `acme-books-001`, and its three services | the project the catalog is built in; every address of the use carries its name, `books__` |
| three `google_project_iam_member` grants on `acme-books-001` for `svc-deploy-books` | the roles from section 2, on that project only |
| the project `acme-deploy-books`, the service account `svc-deploy-books`, the bucket `acme-deploy-books-state`, the delegation's seven APIs | the delegation `books`, which the pack's `contributes_delegations` added |
| a grant of `roles/serviceusage.serviceUsageConsumer` for `svc-deploy-books` on `acme-deploy-books` | the workload estate bills every call to its infrastructure project, and Google checks that the account may use that project's services before any call |
| the group `catalog-books-admins@example.com`, `google_cloud_identity_group.deploy_books_users` | the estate account user group, created empty in the customer's directory |
| grants for `catalog-books-admins@example.com` on the estate account and on `acme-deploy-books` | its members may act as the estate account, and read its infrastructure project; each waits for the group |
| one grant on the billing account, one on the organization (`roles/browser`) | linking a project, and looking a folder up by its name |

Creating the group needs the Groups Admin role in the Google admin console for the
organization's estate account, which `satz update-prerequisites` names; an estate the `init`
template wrote holds it already, and an estate on the customer's own groups assigns it once,
in the admin console. A group that exists in the directory already — created by
hand for an earlier version of the delegation — is imported once, before the apply, so the
plan does not create it a second time:

```bash
satz adopt C0example.satz --only google_cloud_identity_group --execute --import
```

A grant the pack does not state, or a role on a folder the teams share, is not in the plan.
If one is, stop: something other than the pack wrote it. In satz-studio: Apply, then plan
and apply in its list of steps.

**After the apply**, someone with directory rights adds the workload team's people to
`catalog-books-admins@example.com` in the Google admin console (Directory, Groups). The
organization's estate creates the group empty and never manages its members.

**A second instance** is a second named use, with values of its own, and writes its own
project, delegation and interface folders:

```
use "packs/catalog-delegation.satz" named music
```

**Deleting the `use` line** removes everything that instance has on the organization's side
— the project, the grants and the delegation with its infrastructure project, estate
account, estate account user group and state bucket — at the next apply (section 7).

## 5. The interface folders cross back

**What to do:** the workload's team takes the two folders the transpile wrote, whole, with
`satz copy-interfaces`.

Every transpile writes `interfaces/outgoing/` beside the estate, whole:

| folder | what it holds |
|---|---|
| `interfaces/outgoing/delegation-books/` | the delegation: `infrastructure_project_id`, `estate_account`, `estate_account_id`, `state_bucket`, `estate_account_user_group`, `billing_account`, `delegated_apis`, `delegated_billing_roles` |
| `interfaces/outgoing/books-project/` | the use's interface: `project_id`, `delegated_project_roles` |

The organization's `interfaces/outgoing/books-project/` is the workload repository's
`interfaces/incoming/books-project/`. Once the workload estate exists (section 6), its team
takes the folders in the workload repository, with a checkout of the organization's
repository on the same disk:

```bash
satz copy-interfaces books.satz --from <the organization>
```

It copies exactly the folders the estate's `use` lines read, each whole — a file the new
folder lacks is removed, `.DS_Store` and editor files are left out — and only after the
estate transpiled with the new folders; a check that fails copies nothing. It reports per
folder the files and the values that changed; `--report-only` reports and writes nothing.
A workload repository without the organization's repository on its disk takes the folders by
hand — the organization's team sends them, or opens a pull request in the workload
repository — each copied whole over the folder of the same name in `interfaces/incoming/`. The workload's team needs nothing else of the
organization's: not its estate file, not its library, not its state. In satz-studio, the
organization's estate's Estate → Interfaces lists every interface it publishes, with its
exports.

**The folders explain themselves.** Each has a `README.md` that satz writes for the team that
receives it. Its section "For a workload's team" is a table of each export of the folder's
interface with its value — the literal, or "looked up" for one the workload estate's plan
reads — and its description, as the pack declares them; "What to do with it" lists the next
steps, the first being the command of section 6. After a transpile that changed an export,
`CHANGES.md` beside it says what moved and what a reader does about it.

## 6. The workload's team builds

**What to do:** take the folders in, use the workload pack, sign in as a member of the
estate account user group, plan, read and apply.

**The folders.** The copies replace the stand-ins under `interfaces/incoming/`; the estate
reads them by the same names. A workload repository that has no estate yet writes one from
them, in its own directory:

```bash
satz init --from <the organization>/interfaces/outgoing/delegation-books <the organization>/interfaces/outgoing/books-project
```

In satz-studio: the Start screen, Create, **A workload, from interface folders**, **Add
folders…** for the two folders, then **Create**.

`init --from` copies each folder into `interfaces/incoming/` unless it stands there already,
and writes `config.toml` and `satz/books.satz`. Exactly one of the folders is a
delegation's own: an estate runs as one estate account. The estate reads its estate account
and its state bucket from the delegation's folder:

```
estate books

params {
  // The delegation's estate account, `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`,
  // and its state bucket, read from the folder `delegation-books`: the estate runs as the account,
  // its calls are billed to the infrastructure project and its state is kept in the bucket.
  // A copy of the folder after a change of the organization's estate changes them here too.
  infra_project_name = "${{interface.delegation-books.infrastructure_project_id}}"
  infra_bucket_name  = "${{interface.delegation-books.state_bucket}}"
  svc_iac_account    = "${{interface.delegation-books.estate_account_id}}"
  default_region     = "europe-west3"
  default_zone       = "europe-west3-a"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
  catalog_project_id = "${{interface.books-project.project_id}}"
}

use "interfaces/incoming/delegation-books/delegation-books/satz/interface.satz"
use "interfaces/incoming/books-project/books-project/satz/interface.satz"

// …the gcs backend under books/state and the providers, as init --from writes them…

use "packs/catalog-workload.satz"
```

With `deployment_mode = "cloud"` every provider and the state backend act as
`svc-deploy-books@acme-deploy-books.iam.gserviceaccount.com`, the state is kept in
`acme-deploy-books-state` under `books/state`, and every call is billed to
`acme-deploy-books`.

**The workload pack** is the repository's own, in `packs/`. A workload pack another team
wrote is fetched the same way the organization fetched the delegation pack: `satz
pack-source add`, then `satz get-pack <id> --source <name>` (satz-studio: Packs, **Fetch**).

**Sign in.** Whoever runs the plan is a member of the estate account user group — added in
the directory, by whoever administers it — and signed in:

```bash
gcloud auth application-default login
satz whoami books.satz
```

`whoami` says which identity the run uses. In satz-studio: Apply, whose first card says who
is signed in and as which account the estate runs.

**Plan, read, apply.**

```bash
satz transpile books.satz --plan
satz transpile books.satz --apply
```

The plan switches `pubsub.googleapis.com` on in `acme-books-001` and creates the bucket and
the topic there; nothing else. A role the account lacks is Google's refusal at the apply,
naming the permission (section 7).

Anything outside satz's resources is the organization's, granted by hand: a role in the
directory, such as the Groups Admin role a workload that creates Google groups needs, is
assigned in the Google admin console by a directory administrator.

## 7. Afterwards

**A new version of the delegation pack.** The workload's team changes the pack, raises its
`version` and makes it reachable again — a commit in its repository for a folder or GitHub
source, `publish-pack` for a folder it stores into. The organization's team brings it in,
reviews the difference, plans and applies:

```bash
satz update-pack catalog-delegation --report-only
satz update-pack catalog-delegation
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`update-pack` overwrites a pack unchanged since it was fetched and updates its row; a pack
edited in the organization's repository is left as it is and reported with its difference.
In satz-studio: Packs, choose the source, **Update** on the pack's row. When the apply
changed an export, the workload's team takes the folders again, `satz copy-interfaces
books.satz --from <the organization>` (section 5), which says what moved.

**A new version of the workload pack** concerns the workload's team alone: it changes the
pack, or `satz update-pack <id>` brings a fetched one up to date, then plan and apply in the
workload estate.

**A role is missing** when the apply is refused naming a permission, or when
`update-prerequisites --report-only` lists a role the README's `delegated_project_roles`
does not. The role goes into the delegation pack, as a new version, never into a grant of the
workload estate: the estate account holds no right to grant itself one beyond what the
delegation pack allows.

**Removing a workload.** The workload's team removes first: it deletes the workload pack's
`use` line, plans and applies, which deletes the bucket, the topic and the API from its
state. Then the organization's team deletes the `use … named books` line and its
`books.<param>` values, plans and applies, which removes the project, the grants and the
delegation — its infrastructure project, estate account, estate account user group and
state bucket with every state in it. A Google project is protected against deletion unless its body says `deletion_policy
= "DELETE"`; [tear the estate down](workflows.md#tear-the-estate-down) is how that is
declared and applied before the removal.

## 8. The simpler route: the library's delegation packs

A workload that needs a plain project or folder with the standard roles needs no pack pair:
the preset library's ready-made delegation packs give it.
[`presets/delegated-projects.satz`](../presets/README.md#delegated-projectssatz) creates a
project the account works in;
[`presets/delegated-folders.satz`](../presets/README.md#delegated-folderssatz) creates a
folder the account creates projects in. Each entry of their lists names the delegation whose
account works in it, an entry of `delegations`.

| the workload's team | the organization writes | what the team gets |
|---|---|---|
| works in one project it is given | one entry in `delegations`, one in `delegated_projects` | an estate account, a state bucket, and the project `acme-<name>-001` |
| works in several projects with one account | one entry in `delegations`, one in `delegated_projects` per project | an estate account, a state bucket, and the projects |
| creates its own projects | one entry in `delegations`, one in `delegated_folders` per folder | an estate account, a state bucket, and one or more folders it may create projects in |

**What the workload's team asks for:** a name for the delegation (`payments`: lowercase
letters, digits and `-`, starting with a letter); the address of its estate account user
group, which the delegation creates; the folders or projects; the APIs its calls need beyond the standard
six (`cloudbilling`, `cloudresourcemanager`, `iam`, `iamcredentials`, `serviceusage`,
`storage`, each `.googleapis.com`); and `budgets = true` for a budget on its own project or
folder.

**The organization's team** switches the packs on once and adds the entries:

```bash
satz add-preset C0example.satz use_delegation
satz add-preset C0example.satz use_delegated_projects     # for projects
satz add-preset C0example.satz use_delegated_folders      # for folders
satz interview C0example.satz
satz add-entry C0example.satz delegations name=payments estate_account_user_group=payments-owners@example.com
satz add-entry C0example.satz delegated_projects name=payments delegation=payments
```

```
params {
  delegations        = [{ name = "payments" estate_account_user_group = "payments-owners@example.com" }]
  delegated_projects = [{ name = "payments" delegation = "payments" }]
}
```

`add-entry` checks the fields against the pack's `entries` statement, refuses a name the
list already holds, writes the entry into `params { }` and keeps it only when the estate
transpiles. More APIs and billing roles are fields of the delegation's entry, a list stated
whole: `apis=cloudbilling.googleapis.com,…,pubsub.googleapis.com`. A budget is
`budgets=true` on the entry, after `satz add-preset C0example.satz use_delegated_budgets`.
In satz-studio: Presets to switch the packs on, Estate → Interfaces to add an entry —
**Add** on the list, a form of its declared fields. Then plan, read, apply, as in section 4.

The transpile writes `interfaces/outgoing/delegation-payments/` and
`interfaces/outgoing/payments/`, which are copied to the workload's team as in section 5;
the workload's team writes its estate with `satz init --from` on both and builds as in
section 6, reading `${{interface.payments.project_id}}`. A folder's interface gives
`${{interface.<folder>.delegated_folder}}` to create projects under.

**The written route.** The entries can be asked for in writing: the workload's team puts
them into a small request file, checks it against a checkout of the organization's estate,
and sends it as a pull request to the organization's repository; merging it is the
organization's decision.

```
pack payments_request version "1.0"

params {
  contributes_delegations        = [{ name = "payments" estate_account_user_group = "payments-owners@example.com" }]
  contributes_delegated_projects = [{ name = "payments" delegation = "payments" }]
}
```

```bash
satz check-request payments_request.satz C0example.satz
```

[Workload estates beside the organization](tutorial-projects.md) walks this route with every
file.

## 9. Who does what

| | organization's team | workload's team |
|---|---|---|
| writes the delegation pack and the workload pack | reviews the delegation pack | yes |
| finds the roles the estate account needs | grants them, through the delegation pack | lists them with `update-prerequisites --report-only` |
| makes the delegation pack reachable | declares the pack source | puts the pack there: its `packs/`, a GitHub repository, or `publish-pack` |
| writes the `use … named` line and the use's values | yes | agrees on the names |
| creates the project, the grants and the delegation | yes, in its state | no |
| names the estate account user group | yes, as the use's value | proposes it |
| creates that group | yes, in its state, empty | no |
| puts people into that group | whoever administers the directory | whoever administers the directory |
| takes the interface folders | sends them where the workload repository has no checkout of the organization's | `satz copy-interfaces`, after every change |
| writes the workload estate | no | yes, with `satz init --from` |
| creates the workload's resources inside what it was given | no | yes, in its state |
| grants a role outside satz, such as Groups Admin | yes, by hand | asks |
| removes a workload | deletes the `use` line, after the team emptied it | empties it first |
