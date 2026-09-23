# satz for llms

This page is for an agent that drives satz, usually through its MCP server, and
writes Satz. It is the working subset of the language and the rules that keep what
compiles also correct.

[`docs/language.md`](language.md) is the full reference. This page is what to
keep in your head.

## What satz is, and the one rule

Satz is a language for describing a Google Cloud organisation. `satz transpile`
compiles it to OpenTofu HCL; `tofu` applies that. The resource types and attribute
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

A **pack** is the same language with a different header — `pack monitoring.logsink
version "1.2"` — and no `terraform` block. Packs are the reusable half; estates use
them.

## Params

Declared once, referenced two ways:

```satz
params { region = "europe-west3" }

google_storage_bucket {
  b {
    location = region                    // as a value
    name     = "{customer_shortname}-b"  // interpolated into a string
  }
}
```

- `{param}` interpolates inside a string. `{{` and `}}` are literal braces.
- `"${{google_project.infra.project_id}}"` is a **Terraform reference** that survives
  into the HCL — use it to point at another resource in the same estate.
- **First definition wins, outer beats inner.** The estate declares before its packs,
  so an estate param overrides a pack default. That is the customisation channel:
  reach for a param before forking a pack.
- Overriding a list **replaces** it. There is no concatenation for an estate.
- A PACK adds to another file's list param with `contributes_<param> = [ … ]` in its
  own `params`. The entries go after whatever the list holds, each once, they leave
  when the pack is switched off, and they are no variable of their own.

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
        project_service = [ "storage.googleapis.com" ]
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

A repeated *block* is a **list of objects** (`lifecycle_rule = [ { … }, { … } ]`).
A repeated *key* inside one body is an error naming both lines.

## Grants: three forms, and how to choose

**1. Member map, scope from position.** For `google_organization_iam_member`,
`google_folder_iam_member`, `google_project_iam_member` — the scope is wherever the
block sits:

```satz
google_project_iam_member {
  "group:gcp-auditors@{customer_domain}" = [ "roles/viewer" ]
}
```

Every key is a member; the value is its list of roles. The packs use this form, and it
is the only form that **merges across fragments**: two packs granting different roles
to the same member at the same scope fold into one grant.

**2. Member map with its scope pinned.** For a type whose scope is neither the
organisation nor the node it sits in — a bucket, a service account, a KMS key — write
the scope attribute in the map. Every other key is still a member:

```satz
google_storage_bucket_iam_member {
  bucket = "{customer_shortname}-audit-logs-archive"
  "group:gcp-auditors@{customer_domain}" = [ "roles/storage.objectViewer" ]
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

**Choosing:** prefer a member map (1 or 2). Use the labelled form when the scope has to
be a Terraform reference. For a conditional grant, the role becomes an object:

```satz
google_storage_bucket_iam_member {
  bucket = "audit-logs"
  "group:auditors@{customer_domain}" = [
    { role = "roles/storage.objectViewer"
      condition { title = "audit-objects-only" expression = "resource.name.startsWith('objects/audit')" } },
  ]
}
```

**Memberships are not grants.** Presets define groups; humans put people in them. Do
not add `google_cloud_identity_group_membership` to a pack.

## Packs

```satz
use "presets/cis/CIS-GCP-Foundation-4.0.satz"                      // top level
google_org_policy_policy { use "presets/x.satz" }               // as a map's content
use "presets/x.satz" as google_org_policy_policy                // same, written flat
use "presets/cis/shielded-vm.satz" when cis_require_shielded_vm
```

`when` takes a **boolean param that must exist** — an unknown one is an error, never
silently false. An exclusive choice is two booleans and two `use … when` lines; where a
pack declares a `question oneof` over them, satz refuses two true branches by name.

**Provenance by suffix**, and it is enforced:

| file | meaning |
|---|---|
| `X.satz` | pristine, upstream-owned, overwritten by updates |
| `X.local.satz` | a deliberate fork, never touched by updates |
| `X.diff.satz` | the current adoption delta, rewritten on every merge |

To change what a pristine pack does, **use a param**. If no param exists for what you
need, say so instead of forking.

## Removing something a pack contributes

```satz
suppress google_org_policy_policy "compute-skipDefaultNetworkCreation"
suppress google_organization_iam_member "group:x@example.com" role "roles/viewer"
```

A `suppress` that matches **nothing is a hard error**, so a stale suppression is
reported instead of deployed.

## Claims — the compliance plane

A pack says which control it discharges and what witnesses it:

```satz
claim "cis-gcp" "4.0" "1.4" implements {
  resources = [ "google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation" ]
  interpretation = "…what this actually enforces…"
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
params { customer_shortname = "" }

question customer_shortname {
  prompt   = "Short name identifying this customer"
  why      = "Project ids and bucket names derive from it, and those ids are globally unique."
  reversal = recreate          // edit | state_surgery | recreate — cost to the ESTATE
  blast    = none              // none | low | high — cost to the RUNNING organisation
}
```

The two costs are independent. Call `satz_questions` before proposing values: it tells
you which answers are cheap to change and which are one-way doors. **Do not invent an
answer to a one-way door** — ask the human.

A question is **answered when the estate's own `params {}` binds its param** — accepting
the pack's default is an answer, written as the default. Unanswered questions carry the
`default` the pack offers, or `blocking: true` when none is possible (a customer id, a
billing account): those need the human's value. **Every question must be answered before
bootstrap or apply**; `summary.complete` says whether they are.

A question must be declared in the same file as the param it answers.

**A notice is not a question.** A pack can name one command to run once it is switched
on — the CIS org-policy packs name `satz adopt <estate> --execute --import`:

```satz
pack cis_baseline version "2.15"

params { cis_baseline_adopted = false }

notice cis_baseline_adopted {
  text     = "Google sets some of these policies on every new organisation …"
  run      = "satz adopt <estate> --execute --import"
  severity = error
}
```

`satz_interview` and `satz_add_pack` return what their call opened in `notices`. Tell the
human the command and let them run it; when it has run, acknowledge it with
`satz_interview` `answers: {<param>: true}` — never before. Until then the compile warns
and every command that writes to the organisation refuses, because the notice declares
`severity = error`; a notice declaring `warning` or `info` refuses nothing. `satz_packs`
shows every pack's notices with `severity` and `acknowledged`.

**Interviewing a customer** is `satz_interview`: the open questions with their offers;
`create: true` writes a new estate from `presets/estate-core.satz` first; `answers:
{subject: value}` writes what was decided (a `oneof` takes the chosen option's name) and
`accept_defaults: true` writes every offer. Offer defaults as defaults, never invent a
blocking answer, and repeat until `summary.complete`.

**Adding or removing a pack** is `satz_add_pack` / `satz_remove_pack` with the pack's gate
or path — never an edited `use` line. `satz_packs` lists every pack with its answer, its
line, whether it deploys and what it needs. A refusal names the pack that is missing or
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

`hcl { … }` deploys, but the compliance plane **cannot see into it** — it is never a
witness, and it warns on every transpile unless you write `hcl trust "…"`. An `action`
is inert until `satz run-actions`, which launches it by extension: a `.py` script runs
through `uv run --script` on every platform, a `.sh` one does not run on Windows. Write a
new action in Python. Both are last resorts; prefer a real resource.

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
3. **`satz_transpile_check`** — compiles in memory, writes nothing. Run it after every
   edit.
4. **`satz_require <framework>`** — does the estate still discharge what it claims? Run
   it after touching packs, claims or org policies.
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
resource against the live organisation and says per row what it matched on; with
`execute` it writes the verified ids into the estate as `"import-id"`. `rows` is the
worklist, not the table — the rows that ask for something, most urgent first — and
`rows_total`, `rows_omitted` and `note` say what is missing; pass `out` for the whole
report as JSON in a file. The import itself (`satz adopt --execute --import`) is a
human's.

Also available: `satz_check_presets` (is the pack library current, or forked?),
`satz_merge_presets` (bring it up to date: forks a used pack that changed and repoints
the estate, or adopts upstream in place with `adopt`), `satz_update_prerequisites` (the roles the
estate's service account is missing for what it emits and the APIs its infrastructure
project does not enable — it writes both unless `report_only`),
`satz_review_pack` (judge a pack somebody wrote against the library's bar, findings
anchored to file and line — offline and read-only),
`satz_triage` (sort a Prowler export against what the estate claims), `satz_whoami`
(both halves of the identity — the ADC account, and the estate's mode, declared service
account and whether the calls impersonate it — with live checks that the one may become
the other and that the quota project is reachable; check this first when a live call is
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

**Read a finding as fields, not as text.** `satz_transpile_check` — and `satz transpile
<estate> --check --format json` from a shell — return every finding as an object:
`severity` (`error` · `warning` · `info`), `kind`, `subject`, `file`, `line`, `message`
(the sentence) and, where one command answers it, `fix` (that command, as it is typed).
Run `fix`; never cut a command out of `message`. A refused compile returns the same
object with its errors in `findings`, a parse error included (`kind: "front-end"`).

On a terminal the same finding is a block: a first line of severity, kind, `file:line` and
subject, the message indented under it, then `fix: <command>`. A terminal gets the message
wrapped to its width; a pipe gets each paragraph on one line, so a substring of this table
matches what a pipe carries. A group of findings stands under a title ending in its count,
and the last line counts the run: `1 error, 10 warnings; 3 silenced (3 estate) — …`.
Findings of a group that say the same thing — nine packs asking for one command — are one
block, on a terminal and in a pipe: a first line per finding, then the sentence and
`fix: <command>` once. The printed sentence of such a block names no single pack or param;
each finding's whole `message` is in the JSON, and the counts are of findings.

| message | what happened |
|---|---|
| unknown key in a resource body | the attribute is not in the provider schema for that type — invented or misspelled |
| the same address declared twice | two files emit one address with different bodies; the fold refuses and names both |
| `use … when X: unknown param` | `when` names a param nobody declares |
| a suppression matched nothing | the thing you are removing is not there — a stale suppression |
| `claim: resources = [...] is required` | a positive claim without witnesses |
| two branches of a choice are true | a `question oneof` has more than one option set |
| a question names no local param | a question must travel with the param it answers |
| `notices open — what a pack asks to be run once it is on (N)`, kind `notice` | a pack asks for a command to be run now; run the finding's `fix`, then bind its `subject` — the param — `true` |
| the IaC service account … lacks roles, kind `prerequisites` | a resource type the estate emits needs a role the estate does not grant its IaC service account; add the named role to that account's `google_organization_iam_member` list, or run the finding's `fix`, `satz update-prerequisites <estate>` |
| … API(s) this estate's resources need are not enabled on …, kind `prerequisites` | a resource type the estate emits is served by an API no `project_service` entry of the infrastructure project enables; add it to that list, or run the finding's `fix`, `satz update-prerequisites <estate>` |
| `packs on while a pack they need is off (N)`, kind `pack-requirement` | a pack is on and one it needs is off; the `fix` is the `satz add-pack` that switches the needed one on |
| `N silenced (…)` in the last line | the estate or the operator's machine leaves those findings out of the printed output. They are all in `satz_transpile_check`'s `findings`, each carrying `silenced` with the tier and the reason — read them there rather than asking for them to be unsilenced |

## Hard rules

1. Never edit `hcl/` — it is generated.
2. Never invent an id, a project number, a directory customer id or a domain. Ask, or
   resolve it with `adopt`.
3. Prefer a param to a fork; prefer a fork to editing a pristine pack.
4. Never claim a control the resources do not actually discharge.
5. Never answer a one-way-door question on the customer's behalf.
6. Run `satz_transpile_check` after every edit, before saying you are done.
7. If satz refuses, read the message: it names the file and the line.
