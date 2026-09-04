# Satz for agents

You are reading this because you are driving satz — probably through its MCP server —
and you need to write Satz, not just call tools. This is the working subset, with the
rules that stop you producing something that compiles and is wrong.

[`docs/satz-language.md`](satz-language.md) is the full reference. This page is what to
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
- Overriding a list **replaces** it. There is no concatenation.

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
A repeated *key* inside one body is an error naming both lines — silent last-wins is
not a thing here.

## Grants: three forms, and how to choose

This is where most mistakes happen, so it is worth reading twice.

**1. Member map, scope from position.** For `google_organization_iam_member`,
`google_folder_iam_member`, `google_project_iam_member` — the scope is wherever the
block sits:

```satz
google_project_iam_member {
  "group:gcp-auditors@{customer_domain}" = [ "roles/viewer" ]
}
```

Every key is a member; the value is its list of roles. This is the idiom the packs are
written in, it reads as a grant rather than as a resource, and it is the only form that
**merges across fragments** — two packs granting different roles to the same member at
the same scope fold into one grant.

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
use "presets/CIS-GCP-Foundation-4.0.satz"                      // top level
google_org_policy_policy { use "presets/x.satz" }               // as a map's content
use "presets/x.satz" as google_org_policy_policy                // same, written flat
use "presets/cis-extensions/shielded-vm.satz" when cis_require_shielded_vm
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

If you find yourself wanting to edit a pristine pack: **use a param instead**. A fork
whose whole diff could have been a param is debt. If no param exists for what you need,
say so — do not fork silently.

## Removing something a pack contributes

```satz
suppress google_org_policy_policy "compute-skipDefaultNetworkCreation"
suppress google_organization_iam_member "group:x@example.com" role "roles/viewer"
```

A `suppress` that matches **nothing is a hard error**. That is deliberate: stale
subtractive config must surface rather than silently deploy.

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
- `duty_<id> = "…"` records the human half. A control with an open duty reads *partial*,
  never satisfied, and that is correct.
- Claim what the resources actually do. An overclaim is worse than a gap: it is a
  compliance tool telling a customer they are covered when they are not.

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

A question must be declared in the same file as the param it answers.

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
action "scc" { reason = "…" run = "../scripts/x.sh" args = ["--org", "{customer_organization_id}"] }
```

`hcl { … }` deploys, but the compliance plane **cannot see into it** — it is never a
witness, and it warns on every transpile unless you write `hcl trust "…"`. An `action`
is inert until `satz run-actions`. Both are last resorts; prefer a real resource.

## Working through the MCP server

The loop, and the order matters:

1. **`satz_questions`** — what this customer still has to decide. Start here for
   anything that touches params.
2. **Write or edit the `.satz`.**
3. **`satz_transpile_check`** — compiles in memory, writes nothing. Run it after every
   edit; it is the cheapest feedback there is.
4. **`satz_require <framework>`** — does the estate still discharge what it claims? Run
   it after touching packs, claims or org policies.
5. **`satz_transpile`** (needs the `write` capability) — writes `hcl/`.
6. **`tofu plan` / `apply` — a human does this.** No tool exposes it, deliberately.
7. **`satz_report_compliance`** — the live evidence view, afterwards.

Also available: `satz_check_presets` (is the pack library current, or forked?),
`satz_triage` (sort a Prowler export against what the estate claims), `satz_whoami`
(which identity — check this first when a live call is refused).

A tool your capability level does not permit comes back as an ordinary result marked
`isError`, with a sentence naming the level and what would be needed. That is
recoverable: say what you would need and why, rather than retrying the same call.

## Errors you will meet, and what they mean

| message | what happened |
|---|---|
| unknown key in a resource body | the attribute is not in the provider schema for that type — invented or misspelled |
| the same address declared twice | two files emit one address with different bodies; the fold refuses and names both |
| `use … when X: unknown param` | `when` names a param nobody declares |
| a suppression matched nothing | the thing you are removing is not there — a stale suppression |
| `claim: resources = [...] is required` | a positive claim without witnesses |
| two branches of a choice are true | a `question oneof` has more than one option set |
| a question names no local param | a question must travel with the param it answers |

## Hard rules

1. Never edit `hcl/` — it is generated.
2. Never invent an id, a project number, a directory customer id or a domain. Ask, or
   resolve it with `adopt`.
3. Prefer a param to a fork; prefer a fork to editing a pristine pack.
4. Never claim a control the resources do not actually discharge.
5. Never answer a one-way-door question on the customer's behalf.
6. Run `satz_transpile_check` after every edit, before saying you are done.
7. If satz refuses, read the message — it names the file and the line, and it is
   usually right.
