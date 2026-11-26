# satz llms

This page is for an agent that drives satz, usually through its MCP server, and
writes Satz. It is the working subset of the language and the rules that keep what
transpiles also correct.

[`docs/language.md`](language.md) is the full reference. This page is what to
keep in your head.

## What satz is, and the one rule

Satz is a language for describing a Google Cloud organization. `satz transpile`
transpiles it to OpenTofu HCL; `tofu` applies that. The resource types and attribute
names **are the Terraform provider's**, to the underscore — if you know
`google_storage_bucket`, you know the block.

**You edit `.satz`. You never edit `hcl/`.** It is regenerated on every transpile and
your edit disappears at the next one. If something cannot be expressed in Satz, say so
— do not route around it by writing HCL into the output directory.

Satz **refuses rather than guesses**. An unknown block key is a parse error, not an
ignored line; a `use … when` on a param nobody declared is an error, not `false`. When
it refuses, the message names the file and the line. Treat a refusal as information.

## The shape of an estate

```satz
estate acme

params {
  customer_organization_id = "123456789012"
  customer_domain          = "example.com"
  customer_shortname       = "acme"
  default_region           = "europe-west3"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_folder {
  infra {
    display_name = "Infrastructure"
  }
}
```

`inputs { … }` declares a template's contract: params a reader binds — the using estate's `params { }`, or `satz transpile <template> --input <name>=<value>` — which every check reads as their value and the HCL reads as `var.<name>` (`variables.tf` declares them, `terraform.tfvars` carries the bound value). An input decides no structure: not a label, a grant's member or scope, an `each` list, a `when` gate, an export, a question or satz's own keys ([language §6.3](language.md#inputs--a-templates-contract-the-values-a-reader-binds)).

A **pack** is the same language with a different header — `pack monitoring.logsink
version "1.2"` — and no `terraform` block. Packs are the reusable half; estates use
them.

## Params

Declared once, referenced two ways:

```satz
params { region = "europe-west3" }

google_storage_bucket {
  b {
    location = region                   // as a value
    name     = "{customer_shortname}-b" // interpolated into a string
  }
}
```

- `{param}` interpolates inside a string. `{{` and `}}` are literal braces.
- `"${{google_project.infra.project_id}}"` is a **Terraform reference** that survives
  into the HCL — use it to point at another resource in the same estate.
- **First definition wins, outer beats inner.** The estate declares before its packs,
  so an estate param overrides a pack default. That is the customization channel:
  reach for a param before forking a pack.
- **One name, one meaning.** Params share one namespace across the estate and every
  file it uses. Two packs may declare the same param only with the same default (and the
  same question): that is a shared param. Different defaults are
  refused, and a binding in the estate does not lift that — `""` and `"eu"` are two
  defaults. A pack that reads a param and proposes no value declares it `none`
  (`region = none`): that agrees with any one default another file gives, which is then the
  value. A param that belongs to one
  pack carries the pack's name as its prefix (`vm_zone`); a shared one carries none.
- Overriding a list **replaces** it. There is no concatenation for an estate. Entries from
  outside arrive one way: a workload estate's request file, a pack of `contributes_<param>` the
  estate `use`s (see *Interfaces* below).
- A PACK adds to another file's list param with `contributes_<param> = [ … ]` in its
  own `params`. The entries go after whatever the list holds, each once, they leave
  when the pack is switched off, and they are no variable of their own.
- **A value only the file reads is a local value**, `locals { bucket = "{customer_shortname}-logs" }`,
  read by its bare name like a param, in a pack or an estate. No other file sees it,
  nothing binds it, no question asks it, and it is no variable — every read is written as
  its value. Its name is unique in the whole estate: a param, an input or another file's
  local value of that name is refused. A param's default, a `question`, `ask_when` and
  `use … when` never read one ([language §6.3](language.md#locals--values-a-file-keeps-to-itself)).

## Hierarchy is nesting

Where a resource sits *is* its scope. There is no `parent = …` to get wrong:

```satz
google_folder {
  infra {
    display_name = "Infrastructure"
    google_project {
      infra {
        project_id      = infra_project_name
        billing_account = billing_account_infra
        project_service = ["storage.googleapis.com"]
        google_storage_bucket {
          audit_logs {
            name     = "{customer_shortname}-audit-logs"
            location = "EU"
          }
        }
      }
    }
  }
}
```

A project whose `project_service` names `compute.googleapis.com` is written with
`auto_create_network = false`, so it gets no `default` network; state the attribute
to decide otherwise.

A repeated *block* is a **list of objects** (`lifecycle_rule = [ { … }, { … } ]`).
A repeated *key* inside one body is an error naming both lines.

## Grants: three forms, and how to choose

**1. Member map, scope from position.** For `google_organization_iam_member`,
`google_folder_iam_member`, `google_project_iam_member` — the scope is wherever the
block sits:

```satz
google_project_iam_member {
  "group:gcp-auditors@{customer_domain}" = ["roles/viewer"]
}
```

Every key is a member; the value is its list of roles. The packs use this form, and it
is the only form that **merges across fragments**: two packs granting different roles
to the same member at the same scope fold into one grant.

**2. Member map with its scope pinned.** For a type whose scope is neither the
organization nor the node it sits in — a bucket, a service account, a KMS key — write
the scope attribute in the map. Every other key is still a member:

```satz
google_storage_bucket_iam_member {
  bucket                                 = "{customer_shortname}-audit-logs-archive"
  "group:gcp-auditors@{customer_domain}" = ["roles/storage.objectViewer"]
}
```

The scope namespaces the grant, so a second map for a second bucket is a second grant
even with the same member and role.

**3. Labelled resource.** One member, one role, its scope an ordinary attribute:

```satz
google_storage_bucket_iam_member {
  auditors_read {
    bucket = "${{google_storage_bucket.audit_logs.name}}"
    role   = "roles/storage.objectViewer"
    member = "group:gcp-auditors@{customer_domain}"
  }
}
```

**Choosing:** prefer a member map (1 or 2). Use the labeled form when the scope has to
be a Terraform reference. For a conditional grant, the role becomes an object:

```satz
google_storage_bucket_iam_member {
  bucket = "audit-logs"
  "group:auditors@{customer_domain}" = [
    {
      role = "roles/storage.objectViewer"
      condition { title = "audit-objects-only" expression = "resource.name.startsWith('objects/audit')" }
    },
  ]
}
```

**Memberships are not grants.** Presets define groups; humans put people in them. Do
not add `google_cloud_identity_group_membership` to a pack.

## Packs

```satz
use "presets/cis/CIS-GCP-Foundation-4.0.satz" // top level
google_org_policy_policy { use "presets/x.satz" } // as a map's content
use "presets/x.satz" as google_org_policy_policy // same, written flat
use "presets/cis/shielded-vm.satz" when cis_require_shielded_vm
use "packs/project-budget.satz" named payments // under a name, with params of its own
```

`when` takes a **boolean param that must exist** — an unknown one is an error, never
false by default. An exclusive choice is two booleans and two `use … when` lines; where a
pack declares a `question oneof` over them, satz refuses two true branches by name.

**A pack used several times is a named use**, and only a pack whose header ends in
`multiple` takes one. Bind its params in the estate's `params` as `<name>.<param>`
(`payments.budget_amount = "300"`); every other name it reads comes from the estate. Its
resources are written as `<name>__<label>` and its params as the variables `<name>--<param>`;
its grants are not prefixed. Bind each use's names differently: two uses that write one
resource are refused, naming the params to bind. A named use stands in the estate file only
and takes no `hcl`, `action`, `inputs`, `interface`, `export`, `entries` or `notice`;
`satz_questions`, `satz_interview`, `satz_presets`, `satz_add_preset`, `satz_remove_preset`,
`satz_adopt` and `satz_packs` refuse an estate with one — say so rather than working around it
([language §6.9](language.md#named--one-pack-used-several-times)).

**Provenance by suffix**, and it is enforced:

| file | meaning |
|---|---|
| `X.satz` | pristine, upstream-owned, overwritten by updates |
| `X.local.satz` | a deliberate fork, never touched by updates |
| `X.diff.satz` | the current adoption delta, rewritten on every merge |

Nothing else belongs in `presets/`. A file the library dropped (its `dropped-files.txt`
lists it) is removed by `merge-presets`, or forked when the estate uses it; any other
file there is named in a warning, and moving it out is the operator's call.

To change what a pristine pack does, **use a param**. If no param exists for what you
need, say so instead of forking.

**A workload pack is not a preset.** A preset is a pack of the library under `presets/`,
switched on with `satz_add_preset`. A workload pack is one file in the estate's
`packs_dir`, used by an estate beside the organization, taken from a pack source — `satz`,
the satz repository, or a repository or folder the estate's `config.toml` names, which
`satz_pack_sources` lists: `satz_packs` lists the packs with their state and source, and
what each declares — its params with their defaults and questions (`prompt`, `why`), its
list params, its exports with their descriptions — and the interface values it reads, each
with the folders of the estate that give it (`met_by`) and that export's `description`, or
why none does (`unmet_reason`); a pack fits the estate when nothing is unmet.
`interface_folders` lists the estate's interface folders with the organization's estate
that wrote each and the values it exports, each with its `name` and `description`; `local: true` reads no pack source and
lists only `packs_dir`. `satz_get_pack` fetches one (`source` names where from) and records its origin in
`packs_dir/origin.toml`,
`satz_update_pack` brings a fetched one up to date and leaves an edited one as it is
(`report_only` first), `satz_detach_pack` makes one the repository's own,
`satz_publish_pack` copies one of the repository's own into a folder source and records it
as fetched from there, and `satz_review_pack` judges one. A pack with no row in `origin.toml` is the repository's own:
no tool writes it, and you edit it as you edit the estate. A source is added with `satz
pack-source add`, which the human runs; a refusal that names a source `config.toml` lacks, or
declares elsewhere than a row of `origin.toml` records, is theirs to resolve.

## Removing something a pack contributes

```satz
suppress google_org_policy_policy "compute-skipDefaultNetworkCreation"
suppress google_organization_iam_member "group:x@example.com" role "roles/viewer"
```

A `suppress` that matches **nothing is a hard error**, so a stale suppression is
reported instead of applied.

## Claims — the compliance plane

A pack says which control it discharges and what witnesses it:

```satz
claim "cis-gcp" "4.0" "1.4" implements {
  resources        = ["google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation"]
  interpretation   = "…what this actually enforces…"
  duty_rotate_keys = "A human must rotate the remaining keys quarterly."
}
```

- `implements` discharges the control; `contributes` helps; `deviates` declines it and
  **requires `reason`**.
- A positive claim must ship its witnesses — `resources = [...]` is mandatory.
- `duty_<id> = "…"` records the human half. A control with an open duty reads
  *partial*, not satisfied.
- Claim only what the resources do: an overclaim reports a control as covered when it
  is not.

## Questions — what to ask before a param can be filled

```satz
params { customer_shortname = none } // declared, no value proposed

question customer_shortname {
  prompt   = "Short name identifying this customer"
  why      = "Project ids and bucket names derive from it, and those ids are globally unique."
  reversal = recreate // edit | state_surgery | recreate — cost to the ESTATE
  blast    = none     // none | low | high — cost to the RUNNING organization
}
```

The two costs are independent. Call `satz_questions` before proposing values: it tells
you which answers are cheap to change and which are one-way doors. **Do not invent an
answer to a one-way door** — ask the human.

A question is **answered when the estate's own `params {}` binds its param** — accepting
the pack's default is an answer, written as the default. Unanswered questions carry the
`default` a pack offers, or `blocking: true` when no file gives the param a value — it is
declared `none` (a customer id, a billing account): those need the human's value. `""` and
`[]` are values, offered and bound like any other; an estate's `p = none` binds nothing.
A param declared `none` that nothing binds and nothing asks stops the transpile at its
read, naming every file that declares it — bind it in the estate's `params {}`. Each
question's `shape` is the shape to answer in: a param no file gives a value takes it from
where the packs read it (a set attribute is a `list`), so answer a `list` with a list —
one string is written as a list of one. **Every question must be answered before
bootstrap or apply**; `summary.complete` says whether they are.

A question must be declared in the same file as the param it answers.

**A notice is not a question.** A pack can name one command to run once it is switched
on — the CIS org-policy packs name `satz adopt <estate> --execute --import`:

```satz
pack CIS_GCP_Foundation_4_0 version "2.17"

params { cis_baseline_adopted = false }

notice cis_baseline_adopted {
  text     = "Google sets some of these policies on every new organization …"
  run      = "satz adopt <estate> --execute --import"
  severity = error
}
```

`satz_interview` and `satz_add_preset` return what their call opened in `notices`. Tell the
human the command and let them run it; when it has run, acknowledge it with
`satz_interview` `answers: {<param>: true}` — never before. Until then the transpile warns
and every command that writes to the organization refuses, because the notice declares
`severity = error`; a notice declaring `warning` or `info` refuses nothing. `satz_presets`
shows every pack's notices with `severity` and `acknowledged`.

**Interviewing a customer** is `satz_interview`: the open questions with their offers;
`create: true` writes a new estate from `presets/estate-core.satz` first; `answers:
{subject: value}` writes what was decided (a `oneof` takes the chosen option's name) and
`accept_defaults: true` writes every offer. Offer defaults as defaults, never invent a
blocking answer, and repeat until `summary.complete`.

**Adding or removing a pack** is `satz_add_preset` / `satz_remove_preset` with the pack's gate
or path — never an edited `use` line. `satz_presets` lists every pack with its answer, its
line, whether it creates anything and what it needs. A refusal names the pack that is missing or
still depends on it: ask the human before passing `with_requirements` or `cascade`.

## Adopting what already exists

```satz
google_folder { infra { "import-id" = "folders/123456789" display_name = "Infrastructure" } }
```

**Never invent an id.** Use `satz adopt` (a dry run by default) to resolve live ids: it
looks them up and refuses when a lookup is ambiguous. A wrong `import-id` adopts the
wrong object.

## The escape hatches, and what they cost

```satz
hcl trust "reviewed: the provider has no resource for X" { resource "…" "…" { } }
action "scc" { reason = "…" run = "../scripts/x.py" args = ["--org", "{customer_organization_id}"] }
```

`hcl { … }` is applied, but the compliance plane **cannot see into it** — it is never a
witness, and it warns on every transpile unless you write `hcl trust "…"`. An `action`
is inert until `satz run-actions`, which launches it by extension: a `.py` script runs
through `uv run --script` on every platform, a `.sh` one does not run on Windows. Write a
new action in Python. Both are last resorts; prefer a real resource.

## Interfaces — what the workload estates beside an estate read, and what they may ask for

An estate publishes values with `export "<name>" = <value> [attach ["<type>", …]]
[description "…"]` — outside every block a core export, inside `interface "<name>"
[common] { … }` one workload estate's; `use interface "<other>"` composes; `export … = all <type>
[under <type>.<label>]` publishes a map keyed by label; `private = true` in a body or
`private <type>.<label>` keeps a resource out. `satz transpile` writes `interfaces/outgoing/` beside
`hcl/`: `common/` and one folder per workload estate, each interface as an HCL module and as
`satz/interface.satz`, which another estate `use`s. It reads a value by the name of its
interface folder, `"${{interface.<folder>.<value>}}"` — `<folder>` is the interface's name,
the folder's name under `interfaces/incoming/`: `"${{interface.payments.project_id}}"`.
Write this form always. `"${{interface.<value>}}"` reads only a value every used folder
carries with the same content (`default_region`, `customer_domain`); two folders that
carry it differently refuse it. One estate uses one delegation's folder and any number of
delegated folders and projects. A read stands in a resource body, in `terraform` and
`providers`, or as the value of an `export`; inside `hcl { }` it is refused (kind
`interface-use`).
`CHANGES.md` beside an interface's README says what the last transpile changed for it.

`each <list param> by <field> { … }` inside a resource type map writes one labeled body per
entry of the list; `entries <list param> { key fields patterns defaults description }` declares what one entry
of that list looks like, and so what a workload estate may add to it, and a workload estate's request file — a pack of `contributes_<param>` entries
— is checked by `satz check-request` and vendored into the estate by pull request.

`each <list> by <field> as "{each.<field>}_<what>" { … }` labels each body with the string,
so two `each` over one list write two resources of one type per entry, under labels that
stay fixed. `key = value when <param>` and `block when <param> { … }` write the attribute or
the block only where the param carries a value — `when not` for the opposite, and inside an
`each` also `when each.<field>`. A param nobody declares is an error, never `false`.

Over MCP, `satz_check_consumer` checks a workload estate's HCL against the interface it reads.
`satz interfaces` and `satz check-request` have no tool: ask a human for them. Access is
delegated to a team by an entry of `delegations` in `presets/delegation.satz` — its estate
account, state bucket and estate account user group — and
 one entry per delegated resource that names it: `delegated_folders`
(`presets/delegated-folders.satz`) or `delegated_projects`
(`presets/delegated-projects.satz`). `satz_add_entry` adds an entry to any list a pack
declares with `entries` — `{ list, fields }`, checked against the declaration — or a
reviewed pull request does; a request file is checked in the workload repository's pipeline.


## Working through the MCP server

The loop, in order:

0. **`satz_estates`, then `satz_open`.** The server holds no estate until you open
   one — it is started with a root directory, not a config. `satz_estates` lists every
   `config.toml` under that root with the estate files beside it; `satz_open` names one
   config and one `.satz`, and every later call works on that estate under its own
   config. Each listed estate carries its `deployment_mode`, or `refused` with the reason
   `satz_open` would give. Its answer includes `runs_as`: the service account the estate's live tools
   run as. Call it again to move to the next estate; nothing else changes. An estate
   whose params do not parse, whose `deployment_mode` is neither `local` nor `cloud`, or
   whose cloud mode has no `svc_iac_account` or `infra_project_name`, is refused, naming the file and the reason — no tool can say which identity it runs
   as, so the file is fixed first.
1. **`satz_questions`** — what this customer still has to decide. Start here for
   anything that touches params. For a new estate, `satz_interview {create: true}`
   writes the file and returns the open questions; pass what the human decides back
   as `answers` until `summary.complete`.
2. **Write or edit the `.satz`.**
3. **`satz_transpile_check`** — transpiles in memory, writes nothing. Run it after every
   edit.
4. **`satz_require <framework>`** — does the estate still discharge what it claims? Run
   it after touching packs, claims or organization policies.
5. **`satz_transpile`** (needs the `write` capability) — writes `hcl/`.
   `satz_scan_checkov` (needs `exec`) then runs Checkov over it and names the Satz block
   behind each failed check; with `out` (needs `write` too) it also writes Checkov's
   report to that path, for the remediation tools.
6. **`tofu plan` / `apply` — a human runs these.** No tool exposes them.
7. **`satz_report_compliance`** — the live evidence view, afterwards.

**Authoring the remediation plan.** `satz_remediation_items` returns the dossier's items
and its `dossier_sha256`; write the judgment per item — what and why in the customer's
words, the fix, owner, effort, phase, quick win, risk acceptance — and hand it to
`satz_remediation_annotate` with `authored_by` naming you and the tool, and
`authored_at`. It renders the workbook. For the Checkov column, pass both tools the
same `checkov` — the report `satz_scan_checkov` wrote to its `out`; they read it and run
nothing. Never author against a hash you did not get from `satz_remediation_items` for
the same Prowler export and Checkov report.

**Adopting what exists.** Never invent an id: `satz_adopt` resolves every declared
resource against the live organization and says per row what it matched on; with
`execute` it writes the verified ids into the estate as `"import-id"`. `rows` is the
worklist, not the table — the rows that ask for something, most urgent first — and
`rows_total`, `rows_omitted` and `note` say what is missing; pass `out` for the whole
report as JSON in a file. The import itself (`satz adopt --execute --import`) is a
human's.

Also available: `satz_check_presets` (is the pack library current, or forked?),
`satz_merge_presets` (bring it up to date: forks a used pack that changed and repoints
the estate, or adopts upstream in place with `adopt`), `satz_update_prerequisites` (the roles the
estate's service account is missing for what it writes and the APIs its infrastructure
project does not enable — it writes both unless `report_only`),
`satz_review_preset` (judge a pack somebody wrote against the library's bar, messages
anchored to file and line — offline and read-only),
`satz_triage` (sort a Prowler export against what the estate claims), `satz_fmt` (the
canonical layout of a file), `satz_get_presets` (fetch the pack library), `satz_prowler`
(the Prowler command line this estate needs), `satz_whoami`
(both halves of the identity — the signed-in account, and the estate's mode, declared
estate account and whether the calls impersonate it — with live checks that the one may
act as the other and that the quota project is reachable; check this first when a live call is
refused).

**`satz_report_compliance` returns data, not a table.** Read `live_status` before
trusting the rows: `verified` means the inventory was read; `unavailable` means it was
not, and `warnings` says why. Each row carries `responsibility` (`inherited` · `customer`
· `shared` · `satz-managed` · `unassigned`) and each witness is an object — `address`,
`state`, `live_id`, and `declared_at`, the `file:line` of the Satz that declares it.
Render the audit list, spreadsheet or remediation commands from these fields; satz does
not render them.

A tool your capability level does not permit comes back as an ordinary result marked
`isError`, with a sentence naming the level and what would be needed. That is
recoverable: say what you would need and why, rather than retrying the same call.

## Errors you will meet, and what they mean

**Read a message as fields, not as text.** `satz_transpile_check` — and `satz transpile
<estate> --check --format json` from a shell — return every message as an object:
`severity` (`error` · `warning` · `info`), `kind`, `subject`, `file`, `line`, `message`
(the sentence) and, where one command answers it, `fix` (that command, as it is typed).
Run `fix`; never cut a command out of `message`. A refused transpile returns the same
object with its errors in `findings`, a parse error included (`kind: "front-end"`).

On a terminal the same message is a block: a first line of severity, kind, `file:line` and
subject, the message indented under it, then `fix: <command>`. A terminal gets the message
wrapped to its width; a pipe gets each paragraph on one line, so a substring of this table
matches what a pipe carries. A group of messages stands under a title ending in its count,
and the last line counts the run: `1 error, 10 warnings; 3 silenced (3 estate) — …`.
Messages of a group that say the same thing — nine packs asking for one command — are one
block, on a terminal and in a pipe: a first line per message, then the sentence and
`fix: <command>` once. The printed sentence of such a block names no single pack or param;
each message's whole `message` is in the JSON, and the counts are of messages.

| message | what happened |
|---|---|
| unknown key in a resource body | the attribute is not in the provider schema for that type — invented or misspelled |
| the same address declared twice | two files write one address with different bodies; the fold refuses and names both |
| `use … when X: unknown param` | `when` names a param nobody declares |
| ``use "…" named x: the pack … does not say `multiple` in its header`` | a named use takes only a pack whose header ends in `multiple`; use it without `named`, or ask for a pack that says it |
| ``the named uses `a` and `b` write the same resource`` | two named uses (or a named use and the estate) write one body; bind the params the message names differently in each use |
| `named uses are not supported by … yet` | the tool reads every pack once and refuses an estate with a named use; `satz_transpile_check` reads it |
| a suppression matched nothing | the thing you are removing is not there — a stale suppression |
| `claim: resources = [...] is required` | a positive claim without witnesses |
| two branches of a choice are true | a `question oneof` has more than one option set |
| a question names no local param | a question must travel with the param it answers |
| `notices open — what a pack asks to be run once it is on (N)`, kind `notice` | a pack asks for a command to be run now; run the message's `fix`, then bind its `subject` — the param — `true` |
| the estate account … lacks roles, kind `prerequisites` | a resource type the estate writes needs a role the estate does not grant its estate account; add the named role to that account's `google_organization_iam_member` list, or run the message's `fix`, `satz update-prerequisites <estate>` |
| … API(s) this estate's resources need are not enabled on the project their calls are billed to …, kind `prerequisites` | a resource type the estate writes is served by an API no `project_service` entry enables on the project its provider bills to — the project node it stands in, or the infrastructure project for everything else; add it to that project's list, or run the message's `fix`, `satz update-prerequisites <estate>` |
| `packs on while a pack they need is off (N)`, kind `pack-requirement` | a pack is on and one it needs is off; the `fix` is the `satz add-preset` that switches the needed one on |
| `"${{interface.x}}"`: no used interface folder has a value `x`, kind `interface-use` | no interface file the estate `use`s has a value `x`; the message lists what they have |
| `"${{interface.x}}"`: the interface folders `a`, `b` each have a `x` and they differ, kind `interface-use` | two used folders carry the value with different content; name the folder, `"${{interface.a.x}}"` or `"${{interface.b.x}}"` — the message writes both |
| `"${{interface.f.x}}"`: no used interface folder is named `f`, or the folder `f` has no value `x`, kind `interface-use` | the folder's name is the interface's name, the folder under `interfaces/incoming/`; the message lists the folders the estate uses, or the values the folder has |
| the `hcl` block writes `${interface.<export>}`, kind `interface-use` | a passthrough cannot read the interface; write what needs the value as a resource, where satz replaces it |
| `entries <param>: entry … has <field>, which is no field of this list` | a request file carries a field the list does not declare; the message lists its fields |
| `entries <param>: entry … has <field> = "…", which the list's pattern … does not match` | the field's value is not of the form the list's `patterns` sets — `estate_account_user_group` of `delegations` is an address `<name>@<domain>`, and `display_name` of `delegated_projects`, the entry's name when left out, is 4 to 30 characters; write the value in that form, or give the entry a `display_name` that is |
| `N silenced (…)` in the last line | the estate or the operator's machine leaves those messages out of the printed output. They are all in `satz_transpile_check`'s `findings`, each carrying `silenced` with the tier and the reason — read them there rather than asking for them to be unsilenced |

## Hard rules

1. Never edit `hcl/` or `interfaces/outgoing/` — both are generated.
2. Never invent an id, a project number, a directory customer id or a domain. Ask, or
   resolve it with `adopt`.
3. Prefer a param to a fork; prefer a fork to editing a pristine pack.
4. Never claim a control the resources do not actually discharge.
5. Never answer a one-way-door question on the customer's behalf.
6. Run `satz_transpile_check` after every edit, before saying you are done.
7. If satz refuses, read the message: it names the file and the line.
