# 0070 — an estate publishes declared interface modules of static values and lookups, one per team; consumers attach in their own state or contribute through the estate

- **Status:** accepted
- **Date:** 2026-09-25
- **Shipped in:** the release that follows

## Context

satz builds a customer's organisation: the folders, the infrastructure project, the
shared network, the organisation policies. The teams that run workloads in it write
their own HCL, with their own state and in their own repositories, often without satz.
They re-typed the core facts — the organisation id, the domain, the region, the project
ids — and looked folder ids up by hand. There was no output and no data source in what
satz emits: a written reference (`"${{google_folder.x.name}}"`) reached only the estate's
own resources. When a team needed to change shared infrastructure — attach its project to
the shared VPC or to a service perimeter, add a subnet — there was no agreed way, so it
was whoever got there first, in whichever state.

Three terms, for a reader who has not used them:

- An **output** is a value a Terraform module publishes. The root module's outputs are
  what `tofu output` prints; a child module's are what the calling configuration reads
  as `module.<name>.<output>`.
- A **data source** (`data "google_project" "x" { … }`) reads an existing object through
  the provider at plan time. It creates nothing and needs read permission only.
- An **attachment resource** is a provider resource that joins one object to another
  that a different state owns, without owning the other:
  `google_compute_shared_vpc_service_project`,
  `google_access_context_manager_service_perimeter_resource`, every `*_iam_member`.

## Decision

**An estate declares what it publishes with `export "<name>" = <value> [description
"…"]`, groups what one team reads in `interface "<team>" { export … }`, and satz
generates one relocatable module per interface under `hcl/interfaces/`, plus the same
outputs in the root module's `outputs.tf`.**

- **Named interfaces, so each team finds its own code.** An export outside every
  `interface` block is a core export; each `interface` block is one team's.
  `hcl/interfaces/<team>/` carries the team's exports and every core export, so a team's
  folder is complete on its own; `hcl/interfaces/core/` carries the core exports alone.
  The same interface in two files is one interface and its exports merge — a pack can
  add to a team's interface the way it adds resources to a map. `core` is reserved, an
  interface name is a folder name (`[a-z][a-z0-9-]*`), and a team's export may not take a
  core export's name, because both are outputs of the same module.
- **Root outputs never collide.** A core export keeps its name in `outputs.tf`; a team's
  is `<interface>__<export>` with `-` written `_`. No export name holds `__` and no
  interface name holds `_`, so two names cannot meet. `local.satz_interface` nests the
  values the same way: `{ interface = 1, estate, core = {…}, interfaces = { "<team>" = {…} } }`.

- **Static or looked up, decided per reference.** A value satz knows at compile time — a
  param, a literal, an attribute satz writes, one derived from attributes satz writes (a
  service account's `email`) — is a literal output. An attribute only the cloud knows (a
  folder's `name`, a project's `number`) is a `data` source that reads the resource back
  by the natural key satz writes on it; a key that is itself known only to the cloud
  chains to that resource's lookup. The table is `presets/interface-lookups.yaml`.
- **The module is relocatable.** It references no file outside its directory, takes no
  variable, has no backend and reads no state; it declares its own `required_providers`
  with the version the estate pins. Copied, moved, or sourced by git URL, it works, and
  it runs through the consumer's provider and credentials.
- **Core exports come from satz's own packs.** `estate-core.satz` exports the day-0
  answers; `satz init` writes one export of the infrastructure folder into the estate.
- **Writes to shared infrastructure follow one of two protocols.** *Attach*: the write is
  an attachment resource in the consumer's own state, and satz never declares that
  membership authoritatively. *Contribute*: where no attachment resource exists, or the
  change needs coordination, the change is an entry in the estate (a list param a pack's
  `contributes_<param>` or the operator fills, ADR 0051), applied by satz, and the result
  is exported. Declaring attach points and checking a consumer's HCL against them are the
  next steps; the language and emission here do not depend on them.
- **The change notice is a pack.** `interface-notice.satz` publishes `jsonencode` of the
  root module's `local.satz_interface` as an object in a bucket with a storage
  notification to a Pub/Sub topic; Terraform rewrites the object only when a value
  changes, so a message means an exported value changed.

Decisions the design left open, made here:

- **One syntax.** `description` follows the value as a keyword and a string, the way
  `role "…"` follows `suppress`. A block form (`export "x" { value = … description = … }`)
  would be a second spelling of the same statement.
- **An export that needs a lookup of a type with no row is refused at compile**, naming
  the type and the types the table reads back; a static export of any type needs no row.
  A reference that is not `type.label.attribute` of a `google_*` resource (`${var.x}`,
  `${local.y}`) is refused: the module has neither.
- **The lookup table is compiled into the binary.** What the interface emits is part of
  the emitter, and one binary emits one interface whatever preset directory it reads; a
  table read from `presets_dir` would make two checkouts of one satz emit different
  modules.
- **An object value is refused.** A list of scalars is published; a map would need a
  lookup per field and a type for the output, and no export needs one yet.
- **`hcl/interfaces/` is satz's.** `transpile` removes and rewrites the directory whole,
  so the folder of an interface the estate no longer declares goes, and it removes the
  directory and `outputs.tf` when the estate exports nothing. A module holds only the
  lookups its own outputs read.
- **The notice object stays `interface = 1`.** Nesting the values per interface changed
  its shape before any release carried the flat one, so there is no earlier shape to tell
  apart.
- **One name is one output.** The same name with the same value and description from two
  files is one export; a different one is an error naming both files, the rule the fold
  applies to an address.
- **The notice's service agent comes from a data source in a trusted `hcl` block.** The
  Cloud Storage agent's address carries the project number, known only once the project
  exists; satz declares no `data` source of its own. The notification names its topic
  through the grant, and the object records the notification's id, so the apply orders
  grant → notification → object without `depends_on`, which Satz does not have.

## Options

**`terraform_remote_state` on satz's state.** *Rejected.* Every consumer needs read access
to the state bucket — which holds every attribute of every resource, secrets included —
and the backend configuration, and a consumer breaks when the state moves (`satz migrate
--mode cloud`). It publishes everything rather than what is declared.

**A contract object only (a JSON file in a bucket).** *Rejected as the interface; kept as
the notice.* Every consumer would parse it with `jsondecode(data.google_storage_bucket_object_content…)`,
the values carry no types, and lookups of computed ids would still be the consumer's.

**A module that `source`s satz's `hcl/` directly.** *Rejected.* A module call of the root
module creates its resources a second time in the consumer's state.

**Static values only.** *Rejected.* Folder and project numbers exist only once the
resources do; a consumer would copy them by hand after every recreation, which is the
problem this solves.

**A module with the lookups and no static values.** *Rejected.* A lookup costs an API call
and a read permission per plan for a value satz already knows.

**One module for every team.** *Rejected.* Every team reads every other team's values
and every lookup, needs read permission on all of them for its plan to run, and finds
nothing that says which values are its own.

**A per-export audience list** (`export "x" = … for ["team-a", "team-b"]`). *Rejected;
the block was chosen.* A list on each export spreads one team's interface over every file
that exports something for it, so what a team reads is known only by collecting the lists.
A block names the interface once, reads as the team's contract, and merges across files
like a resource map. A value two teams read is a core export, or it is written in both
blocks.

## Amendment — the notice is a choice, and the workload folder is a core export

**The change notice is a selector.** The map asks how the teams hear of a changed export
as `question oneof interface_notice`, not required, with one option per delivery form;
`interface_notice_pubsub` gates `interface-notice.satz`, and the boolean
`use_interface_notice` it replaces is refused by name (`RENAMED_PARAMS`). A further form —
a webhook, a push of the interface to a customer repository — joins as an option and a
pack beside it, and the estate answers which one it takes without a boolean per form
that nothing stops from being on twice. A choice that is not required may have one
option, because "none" is its second answer; `satz interview` offers it as `0) none` and
`satz_interview` takes `"none"`.

**The workload folder is a core export.** Where the customer's and the teams' folders live
is the parent every team's folder takes, so every interface carries it as
`workload_folder`. It works like the infrastructure folder, except that it may be empty:
one param, `workload_folder_name`, with a question. `""`, the default, is the
organisation itself, and nothing is created for it. Every customer has one, so every
estate that uses `estate-core.satz` answers the question; the one-line edit per estate
was accepted. The parent is where the folder block stands in the estate: at the top
level it is the organisation, and inside another folder's block — a top-level folder
named after the organisation — it is that folder, which the interface lookup follows.
Thomas chose this shape.

- **A question may say what an empty answer means** (`empty = "…"`). The question model
  read `""` as "not known yet", so an empty answer could not be recorded; with `empty`,
  `""` is offered, accepted and counted, and a question without it keeps the old reading.
  It is a question attribute rather than a special case for one param, so any pack can
  declare it.
- *A boolean `workload_folder` with a question, and the name asked only when it was
  `true`* was built first, to get around the empty-answer reading. Rejected: two params for
  one fact, and the parent is already said by where the block stands.
- *One param with no question* was built second, so nothing would block. Rejected: every
  customer has a workload folder, so it is a day-0 answer like the infrastructure folder,
  and an unasked param is one nobody decides.
- *The export in `estate-core.satz`* would need an export whose form depends on a param
  — a static value in one case, a declared folder and its lookup in the other — and Satz
  has no conditional export. So the estate carries the section, as it carries
  `infra_folder`: `satz init` writes it from `--workload-folder-name`, and answering the
  question in an interview writes it through the same function. An answer whose form
  differs from the section already there is refused rather than rewritten, because
  turning a folder into the organisation, or the reverse, moves every folder the teams
  created under it.
- The compile refuses a name and a section that disagree (finding kind
  `workload-folder`): a name with no `export "workload_folder"` or with the organisation's,
  and an empty name with a folder's. An empty name with no section compiles — it
  publishes nothing false — so an estate adds the section when it wants the export.

## Amendment — an interface uses another

A pack can declare a coherent set — `interface "network" { export … }` — that several
teams read. A team's module carried the core exports and its own, so a team that needed
the network values had to repeat each export in its own block.

**`use interface "<name>" [when <param>]`, and the list form `use interface ["network",
"dns"]`, inside an `interface` block puts the named interfaces' exports into the team's
module.** It reaches through a chain (a used interface's own `use interface` lines count),
`when` gates it as it gates a pack line, and a param no file declares is an error. Refused:
a name no file of the estate declares, `core`, the interface itself, a cycle (naming the
chain), and two exports of one name reaching one module from two interfaces (naming both
files). An interface of `use interface` lines alone is a module. The README's `From`
column names the interface each value comes from. The root module keeps one output per
export — `<interface>__<export>` of the interface that declares it — because the root
module is not what a team reads, and a second output of one value would be two names for
it in `tofu output` and in the notice object.

- **The verb is `use`, not `include`** (Thomas). One verb brings something in, and
  `include` is the YAML dialect's word (`!include`).
- **`interface` qualifies the line; `use "<path>"` stays unqualified** (Thomas). The
  argument here is a NAME, not a path, so the word says which kind of thing is meant. A
  file declares no kind — what it is follows from its contents — so `use preset` /
  `use module` would be a second source of truth needing a mismatch refusal, packs do not
  fall into clean kinds (`estate-core.satz` holds params, questions and exports), and
  every `use` line of every estate and pack would need a hand edit. A reader who wants to
  know what a line brings in reads `satz packs` or the language server's hover.
- **No tool that reads pack lines sees it.** The line stands only inside an `interface`
  block and names no path: the `use "<path>"` scanner of `satz packs`, `add-pack` and the
  interview (`src/packs.rs`), `merge-presets`' repoint and walk (`src/presets.rs`), the
  pack graph, `doc-packs` and the language server each have a test that it is not read as
  a pack line.
- **A tag per resource (`group "network"`) was not built.** It spreads an interface's
  definition across the estate. It would be reconsidered for a set that crosses packs and
  resource types and cannot be named as an interface.

**An interface change that breaks its consumers is a breaking change** (Thomas). A pack
version that removes or renames an export, or changes its value's shape, gets a
`## Breaking changes` entry in `presets/README.md` and makes the release a minor one
(ADR 0010), like any other refusal. Handing a new or changed interface to the teams is the
build pipeline's job, not satz's.

## Amendment — attach points, and checking a team's HCL against them

The *Attach* protocol above had no declaration: nothing told a team which of the estate's
objects it may attach to, and nothing stopped the estate from writing the membership a team
attached to — its next apply would remove the team's attachment.

**`export "<name>" = <value> attach ["<resource type>", …] [description "…"]` declares an
attach point.** The capability is per export, not per interface (discussed with Thomas):
one team's interface mixes reading `org_id`, attaching to the host VPC and requesting a
firewall rule, so a read/write qualifier on the interface would say nothing true. Each
team README shows a capability table: every export is read, an attach point also takes the
types it names. A *request* column — the list param a central pack takes for the teams'
contributions — joins the table when a pack declares one; none does yet, so the table
has no such column.

- **Spelling: a keyword clause after the value, like `description`**, in either order and
  each once. The roadmap sketched `{ attach = [ … ] }`, a block after the value; the
  export statement already carries a trailing keyword and a value (`description "…"`),
  and a block would be a second shape for the same kind of thing on one statement.
- **The types and their conflicts are data**, `presets/attach-points.yaml`, compiled into
  satz for the reason the lookup table is. Per type: the argument that names the shared
  object, and the estate's authoritative form of the membership — an attribute it must not
  set, the entry its `lifecycle { ignore_changes }` must hold, or a type it must not
  declare on the same node. Seeded with the shared-VPC service project, the perimeter
  resource, the NCC spoke and `*_iam_member`. A type the table has no row for is refused.
- **The perimeter needs both halves.** Not setting `status.resources` is not enough: the
  provider treats an unset list as empty and removes every attached project, so the
  perimeter must also ignore `status[0].resources`. The compile demands the
  `lifecycle` line rather than writing it, because the emitter adds nothing an estate did
  not declare except the documented derivations.
- **`satz check-consumer <dir> [<estate>]`** reads the team's `.tf` files with
  `crates/satz-hcl` and holds them to the compiled estate, offline: an attachment off an
  attach point, an authoritative grant or an org policy on a node the estate manages, a
  resource the estate declares too (by the lookup table's keys). A value reads the estate
  when it is `module.<m>.<output>` of a module sourced from `interfaces/<interface>`, or a
  literal equal to a value the estate publishes or writes; anything else is the team's own,
  and the check says nothing about it. Without an estate argument it takes the one estate
  in `yaml_dir`, and refuses naming them when there are several.
- **It is an MCP tool, `satz_check_consumer`, read-only.** It reads files under the
  server's root and compiles in memory, like `satz_transpile_check`; an agent writing a
  team's HCL is the reader most likely to attach where it should not.

**The design rule for central packs** — stub or central-only, decided per resource and by
security as much as by the provider — is in `presets/README.md` ("A pack that publishes an
interface").

## Amendment — every resource of one kind as a map, and `private`

A team that creates projects under the estate's folders needed one export per folder, and
a folder the estate added later reached no team until someone wrote its export.

**`export "<name>" = all <resource type>` publishes every resource of the type as one map
output, keyed by satz's resource label** (Thomas: "if that changes it should be for a
reason"). Labels are predictable and every README lists them; a display name can hold
anything and changes for cosmetic reasons, and a numeric id is unknown at compile time.
Each value is the attribute the lookup table's new `all` column names for the type —
`name` for a folder, `project_id` for a project, `email` for a service account, `id` for a
network, subnetwork, topic or dataset, `name` for a bucket or a tag — static where satz
writes it, a lookup where the cloud knows it, exactly as a plain export of that attribute
would be. A type the table has no row for is refused; a type the estate emits none of is
an empty map, because a pack may export a kind an estate does not declare yet.

- **`all` is a keyword only before a word on its own line.** `export "x" = all` with
  nothing after it on the line reads the param `all`, so no estate's param changes
  meaning.
- **A renamed label is a breaking change for the teams**, like a renamed export: the pack
  version's changelog row names the old and the new label, and the release is a minor
  one (the standing rule above).
- **The cost** is a lookup per element that only the cloud knows, in every team's plan.

**`private = true` in a resource's body keeps it out of the interface**: `all` skips it,
and an export whose value names it is refused. It is a satz body key (`satz_body_key`,
ADR 0047 — the list is now ten), stripped by the emitter before any rule reads the body
and recorded in the emission manifest; `true` or `false`, anything else refused.

- **A body key, not a keyword before the label.** Every other thing satz says about one
  resource — `"import-id"`, `lifecycle`, `provider` — is a key in its body, and a keyword
  before the label would be read as a named entry (`key name { … }`) by the parser.
- **No "exportable" marker.** Nothing leaves the estate unless an `export` names it, so
  the opt-out is the only marker needed.
- **The scaffold marks what no team should read**: `satz init` writes `private = true` on
  the state bucket and on the IaC service account. `iac_service_account` stays a core
  export of `estate-core.satz`: it is a param template, not a reference to the resource,
  and a team needs the address to grant the estate access to what it creates.

## Amendment — projects, `interfaces/`, and the Satz form

The interfaces served consumers that write HCL. A consumer that writes its own Satz estate
had nothing to read: nothing in `hcl/interfaces/<name>/` describes the interface as data —
the attach points, static-versus-lookup and the managed nodes were README prose — a
written reference reached only `google_*` roots, and `check-consumer` needed the central
estate compiled beside it.

**Terms.** A **project** is an estate that depends on parts of another estate's
interface; it may be maintained by a different team and has its own repository or folder,
config, state and pipeline. The estate it reads is the **central estate**. In docs and
messages "project" alone always means such an estate; a GCP project is "Google project"
or `google_project`. The documentation states the rule once, in the language reference
where the term is introduced, and "team" gave way to "project" wherever it named a
consumer.

**Decisions** (the plan approved on 2026-09-26, and the details it left open):

1. **Layout: `hcl/` stays the root module; `interfaces_dir` (default `interfaces`, a
   `config.toml` key `satz init` writes) is generated beside it.** `interfaces/common/`
   holds the library alone; `interfaces/<project>/` holds the project's own interface and
   every common one, so a project takes one folder whole and never picks common files
   again after an update. Each interface is `README.md`, `hcl/` (the module, unchanged)
   and `satz/interface.satz`. A common interface is `core`, every interface a pack
   declares, and one marked `interface "<name>" common { … }`; no other project's
   interface is ever copied into a project's folder. `hcl/interfaces/` is removed on the
   next transpile, and `common` joins `core` as a reserved interface name.
2. **A project's own interface keeps the merge**: it carries the core exports and those of
   every interface it uses, so a project can work from that one module or file alone.
3. **Nothing generated goes into `presets/`.** The generated files hold one estate's
   values and lookup keys; `doc-packs` and `pack-graph` refuse an interface file in the
   library by name.
4. **The Satz form is a file kind of its own**, `interface "<name>"` alone on its line as
   the header, stamped in a comment with the satz version and the estate it came from.
   *Chosen here:* the body is data in the parser's generic block form — `central { estate
   organizations }`, `output "<name>" { value attach targets description }`, `lookup
   "data.<type>.<label>" { reads permission arguments { … } }`, `managed "<address>" { ids
   keys { … } refs { … } }` — and the parser reads it into `satz::InterfaceFile`, refusing
   anything else at its line. Only the header is new syntax, so the formatter, the
   tree-sitter grammar and the language server read the body as they read any block; a
   dedicated statement per kind (`lookup`, `managed` as keywords) was rejected as four new
   keywords for data no one writes by hand, and reusing `export` statements was rejected
   because a project estate's own `export`s and the central estate's values would share
   one statement with two meanings. A value is written in the form a project reads it — a
   literal, or text over `${data.<type>.<label>.<attr>}` — so `${{interface.x}}` is one
   substitution, not an evaluation.
5. **A project reads `${{interface.<export>}}`** after `use "<path>/interface.satz"` at the
   top level. A whole-value reference becomes the value (a list stays a list); an embedded
   one its text. A lookup's `data` block, with those its arguments read, is emitted once
   into the project's `main.tf` through the provider its top-level resources carry. *Chosen
   here:* one export name from two used files is one value when both carry the same, and
   an error naming both files when they differ — the rule one name in two files follows
   everywhere in Satz. The plan's "an error" for any clash would have refused every
   project that uses its own interface beside a common one, since both carry the core
   exports.
6. **The rules `check-consumer` runs hold at the project's compile.** `src/consumer.rs`
   judges through one `Facts` value — the exports with their attach points, targets and
   static text; the resources a project can name, with their identities and natural keys;
   the organisations — built from the compiled central estate for `check-consumer`, and
   read from the interface files for a project compile. A violation is the finding kind
   `interface-use`. *Chosen here:* the managed resources are those of a type the lookup
   table reads back and those an export names, never one marked `private`; `check-consumer`
   reads the same set, so the two paths judge the same facts. The cost: `check-consumer`
   no longer recognises a literal that names a resource of a type outside the lookup
   table (a secret, a key ring) as the central estate's, and a private resource is outside
   both checks.
7. **Request is unchanged**: a contribution is a pack the central estate `use`s (ADR 0051);
   fetching it from the project's repository is the pipeline's job.

*Chosen here too:*

- **The content hash** is SHA-256 over every file of the folder but its index README, in
  path order: the path relative to the folder, a NUL byte, the length as 8 little-endian
  bytes, the bytes. The length keeps bytes from moving across a file boundary unnoticed.
  The files carry the satz version in their stamps, so a new satz changes the hash.
- **`--output` places the root module only**; the interfaces go to `interfaces_dir`
  either way, and `interfaces_dir` may not hold `hcl_dir`, because satz removes it whole.
- **A common interface uses only common interfaces.** It travels into every project's
  folder, and a project's interface it used would travel with it.
- **The managed facts are in every interface file of the estate, hashed.** The duplicate
  rule needs every resource the central estate declares that a project can name, and the
  file travels into every project's folder — so with the values in clear, one project's
  file named every other project's Google project id, folder name and service account.
  Decided with Thomas (2026-09-26): the ids and key values are `sha256:<hex>` of the
  value, and a project's compile hashes its own literals to compare (`same` in
  `src/consumer.rs`); `check-consumer` against a compiled estate compares in clear, and
  one function reads both. A name is not a secret — the point is that the file carries no
  inventory; a project that writes a value is told it collides, and one that does not
  learns nothing. The refs stay addresses, which name a label and no id. A `private`
  resource is absent from the file, so a project that re-declares the state bucket is not
  caught; accepted, since its name is not published, and it fails at apply.
- **A `${interface.<export>}` inside an `hcl { }` block is refused at the block.** The
  passthrough is appended after emission and nothing in it is replaced, so the reference
  reached Terraform as an unknown object; the refusal names the block and says to write
  what needs the value as a resource. Re-export — a project publishing what it read to
  projects of its own — is not designed here; an `export` of an interface value is refused
  by the reference rule already.
- **The lookups keep the central estate's labels in the project's `main.tf`.** A
  project's own `data` block of the same address, which only an `hcl { }` block can
  declare, is a duplicate Terraform refuses at validate — loud, and rare enough that a
  prefix on every label was not worth its three places.
- `satz packs` lists an interface file under `interfaces`, not as an unmanaged pack; the
  language server compiles no interface file on its own.

**Options not taken.** *Keeping `hcl/interfaces/` and adding the Satz file beside each
module* was rejected: the root module's directory would keep holding directories that are
not part of it, and a project would still pick the common modules one by one. *A Satz
project reading the central estate's source* (compiling it, as `check-consumer` does) was
rejected: a project has its own repository and pipeline, and the central estate's source
is not in it. *Referencing values as `module.satz.x` in Satz* was rejected: Satz has no
modules, and the reference root names what it reads.

**Consequences.** A project written in HCL edits its module `source` once (a
`## Breaking changes` entry under v0.85.0, a minor release); the plans of the estate and
of its projects do not move. satz-studio's catch-up — the `interfaces_dir` default, the
file kind and `common` in its tree — follows its satz pin.

## Amendment — `CHANGES.md`: what the transpile changed, from the previous file on disk

A project pins an interface by a commit of the central estate's repository, and between
two commits the module may change because the satz binary moved, `get-presets` pulled a
newer pack, or the estate was edited; nothing in the folder said what changed for that
project. The question the project has is "what changed in my interface, and what do I do
about it" — not which stream caused it.

**Before `satz transpile` rewrites `interfaces/`, it reads the previous `interface.satz`
of every interface; after writing, it diffs old against new and writes
`<interface>/CHANGES.md` where something changed** (`src/interface_changes.rs`): a todo in
both spellings for what a project must do — a rename (an output gone and one added that
name the same resources in the same shape; nothing else is a rename), an output gone, a
map key lost, an attach point dropped, a value now looked up with the permission its plan
needs, a shape that changed — then what else changed as information. A description edit
is no change. The file is outside the content hash, which is the interface's content and
not the state it replaced.

- **No contract version, no provenance per output.** A shape hash and the pack and
  version that declared each output were designed first and struck: the net effect is
  enough, and `targets` already tells a rename from a removal. The operator sees pack
  versions in `satz packs`.
- **No accumulation, no date.** One transpile is one step, and the estate's history of
  the file is the guide across several. An accumulated file would diverge between two
  operators transpiling from different prior states, and a date would move the corpus.
- **No `check-consumer` finding for a read of an output that is gone.** `tofu plan`
  names the missing attribute, and `CHANGES.md` names the replacement; a Satz project is
  refused at compile already.
- **No `interface-diff` command over two git refs.** Two `interface.satz` files are
  Satz, and `git diff` reads them; the transpile's own diff is where the previous state
  is at hand for free.
- **A previous file that does not parse refuses the transpile**, naming the file: satz
  wrote it and rewrites the directory whole, so a hand edit or an older satz's file is
  removed, not read around.

## Consequences

- An estate that exports anything — every estate that uses `estate-core.satz`, every
  estate `satz init` writes — gains `hcl/outputs.tf` and `hcl/interfaces/core/`. Its resources
  do not change; its plan shows the new outputs. That is an emission change: a minor
  release.
- `presets/interface-lookups.yaml` is derived from the provider's data source schemas and
  maintained by hand; the rows the showcase uses are validated by the smoke matrix, the
  others by nothing (docs/housekeeping.md).
- A consumer's lookup needs read permission on what it reads; a consumer without it fails
  its own plan, and satz's state is not involved.
- A notice subscriber learns of a change only when the apply goes through the estate's
  state; a change made outside it is drift, which `report-compliance` reports.
