# Satz — language specification

**Version:** v0, as implemented in `crates/satz-core/src/satz.rs` (satz v0.46.41).
This document is derived from the parser, not from intent: where the two
disagree, the parser is right and this file is a bug. Every example below was
compiled on 2026-08-24 and the HCL shown is what came out — or it is lifted from
a shipped pack or a fleet estate, with the file named.

*Satz*: German for both **sentence** and **theorem** — a file is at once a
statement of intent and a provable claim. The two halves of that word are the
two halves of this document: how you *state* an estate, and how the statement
becomes something that can be *proven*.

Written for people who already run infrastructure as code and want to know,
precisely, what they gain and what they give up. Nothing is given up: Satz is
built on HCL, and every hour spent learning the provider carries over.

---

## 1. The layers

satz is four layers, and each one is a strictly stronger statement about the
estate than the one below it. HCL holds the resources. Satz decides which
resources exist and under whose policy. Claims say which control each resource
is *for*. Evidence checks that the control is *actually in force* — live, by
value.

![The four layers: HCL foundation, Satz, controls (declared), evidence (proven)](satz-layers.svg)

| layer | what it holds | what it proves | command |
|---|---|---|---|
| **HCL** | resources, providers, state — the IaC layer's assembly language, run by OpenTofu (preferred) or Terraform, with providers from Google and others | that a resource *exists* as declared | `tofu plan` / `apply` |
| **Satz** | estate + packs, params, composition, suppressions | that the estate is a consistent, conflict-free fold of named, versioned parts — and that every org policy in it is *declared* on purpose | `transpile` |
| **Controls** | claims against a catalog | that each declared control has its witnesses *emitted* — a claim with a missing witness is reported, never silently satisfied | `require` |
| **Evidence** | the goal view joined with the live estate | that each witness is live and — for org policies — *enforcing*, by value; a policy switched off in the console is **NOT ENFORCED**, which outranks DRIFTED | `report-compliance` |

Two things the picture is careful about. First, the controls layer is **declared,
not proven**: `require` judges the estate as written and never says "compliant".
Only the evidence layer looks at the cloud, and even it states check semantics —
"a resource with these properties was verified at this time" — never legal
conformity. Second, the witness arrow points at **emitted HCL addresses**. That is
what ties the layers together: a claim is not prose about a control, it is a list
of Terraform addresses, so the same compile that produces `main.tf` produces the
evidence the claim will be checked against.

"Secure by policy and design" is the Satz layer's contribution, and it is a
property of the language rather than of any pack: org policies are ordinary
resources in versioned packs; a pack the estate includes never changes silently
(a semantic upstream change forks and repoints with proof); declining a control
is a visible one-liner that hard-errors when it goes stale; and there is no
escape hatch the proof layer cannot see, because `hcl { … }` warns on every
transpile until someone signs it.

---

## 2. HCL is the foundation — and you keep all of it

HCL is the language of the IaC layer: the assembly code that OpenTofu (the
preferred tool) or Terraform executes against provider plugins from Google and
others. Satz compiles to it. It does not wrap it, rename it, or hide it.

**There is no Satz provider documentation, on purpose.** The OpenTofu registry
and the HashiCorp registry are the documentation. Resource type names are the
provider's, to the underscore. Attribute names are the provider's, to the
underscore — across six real resource pairs from fleet estates (bucket, org
policy, IAM grants, conditional grant, log metric + alert policy, folder/project
hierarchy) **not one attribute name differs**. What you know about
`google_storage_bucket` from the registry page is what you write.

### 2.1 One resource, both ways

The audit-log bucket from the shipped `organization-audit-logsink` pack, as
written and as emitted for estate 1 (`presets/monitoring/organization-audit-logsink.satz:53`
→ `hcl/main.tf:2363`):

```
google_storage_bucket {
  org_audit_logs {
    project  = "${{google_project.logsink_project.project_id}}"
    name     = logsink_bucket_name
    location = logsink_bucket_location
    storage_class = "NEARLINE"
    uniform_bucket_level_access = true
    public_access_prevention    = "enforced"
    lifecycle_rule = [
      { action { type = "Delete" }
        condition { age = logsink_retention_days } },
    ]
  }
}
```

```hcl
resource "google_storage_bucket" "org_audit_logs" {
  provider = google.google
  project = "${google_project.logsink_project.project_id}"
  name = "acme-organization-audit-bucket"
  location = "europe-west3"
  storage_class = "NEARLINE"
  uniform_bucket_level_access = true
  public_access_prevention = "enforced"

  lifecycle_rule {
    action {
      type = "Delete"
    }

    condition {
      age = 400
    }
  }
}
```

Every attribute — `project`, `name`, `location`, `storage_class`,
`uniform_bucket_level_access`, `public_access_prevention`, `lifecycle_rule`,
`action.type`, `condition.age` — is the provider's name. Where the text differs,
it differs for a reason:

| HCL | Satz | why |
|---|---|---|
| `resource "google_storage_bucket" "org_audit_logs" {` | `google_storage_bucket { org_audit_logs { … } }` | the type is a **map** of resources, so two files can contribute to it and the fold can merge them |
| `provider = google.google` | *(not written)* | derived from where the resource sits — org, folder, or project — never repeated by hand |
| `name = "acme-organization-audit-bucket"` | `name = logsink_bucket_name` | a bare identifier is a **param**; the estate binds it, the pack declares the default |
| `"${google_project…}"` | `"${{google_project…}}"` | `{…}` interpolates params, so a literal brace is doubled |
| `lifecycle_rule { } lifecycle_rule { }` | `lifecycle_rule = [ { … }, { … } ]` | a repeated block is a **list value** — it can be overridden as one thing, and the fold can compare it |
| *(quoted label if it has `-`)* | `"compute-managed-requireOsLogin" {` | labels are identifiers or strings; hyphens become underscores in the address |

### 2.2 The complete list of transformations

Nine syntactic ones — and, below them, the short list of attributes the emitter
*derives* for you. If it is not on either list, Satz did not change it.

1. `resource "T" "L" {` → `T { L { } }`. Label quoted only when it is not an
   identifier; `-` → `_` in the emitted address.
2. `provider = …` is emitted, never written.
3. A bare identifier is a param and is emitted as its resolved **literal** —
   never `var.x` (params also become `variable`s in `variables.tf` with values in
   `terraform.tfvars`, for reference; resources do not point at them).
4. `"{param}"` in a string interpolates; `${{…}}` → `${…}`.
5. A list of objects `x = [ {…}, {…} ]` emits repeated `x { }` blocks. A
   single block `x { }` is unchanged.
6. `labels { k = "v" }` (block syntax) → `labels = { "k" = "v" }` (map attribute).
7. **IAM grants** — the one shape that is not 1:1: `"member" = [roles…]` emits one
   `*_iam_member` resource per (member, role, condition), with a hashed label and
   `role` / `member` / `org_id`-or-`project` synthesized. A type scoped by
   neither the organisation nor its node writes that scope in the map
   (`bucket = …`, `service_account_id = …`), and it namespaces the grant.
8. Hierarchy **is** the parent reference: a `google_folder` inside a folder emits
   `parent = google_folder.<outer>.name`; a `google_project` inside a folder
   emits `folder_id = …`; a top-level folder gets `parent = "organizations/<id>"`.
9. Org policy `name` is written as the bare constraint
   (`compute.managed.requireOsLogin`) and emitted as the full
   `organizations/<id>/policies/<constraint>` — the one attribute whose *value*
   is expanded.

**Derived, never written** (each is a consequence of context, not a rewrite):

- a project without `billing_account` gets `billing_account_infra`; a project
  without `name` gets its `project_id`;
- a `google_cloud_identity_group` label becomes `group_key { id = "<label>@{customer_domain}" }`,
  `parent = "customers/{customer_id}"`, the discussion-forum labels, and
  `lifecycle { ignore_changes = [initial_group_config] }` (merged with a
  declared `lifecycle`); its `member` / `manager` / `owner` lists become
  `google_cloud_identity_group_membership` resources (§6.4);
- `project_service = [ … ]` explodes into one `google_project_service` per
  service; a project gets its own provider alias when it needs one;
- an org policy's structured `parameters { … }` is JSON-encoded into the
  string the provider wants;
- the backend is chosen by `deployment_mode` (§6.8); `"import-id"` becomes an
  `import` block in `imports.tf` (§6.7); param names are kebab-cased in
  `variables.tf` / `terraform.tfvars`.

That is the whole tax. Everything else is registry knowledge, unchanged.

---

## 3. What Satz adds on top

The layers diagram reads bottom-up; this section reads the same way. Each
addition is one line or a few, shown against the shipped CIS pack.

**Include a pack** — 23 controls, 18 org policies, one line:

```
google_org_policy_policy {
  use "presets/CIS-GCP-Foundation-4.0.satz"
}
```

**Tune it without forking** — the pack declares a default, the estate binds it.
estate 3 keeps its contact addresses on a different domain than its organisation, so:

```
params {
  essential_contacts_allowed_domain = "example.net"
}
```

**Decline one control it provides** — no fork, one line, and it fails loudly the
day upstream retires the policy:

```
suppress google_org_policy_policy "compute-managed-requireOsLogin"
```

**Say why, on the record** — so the control reads ⚠ *deviation* in every report
instead of a false ✓ or an oversight-looking ✗:

```
claim "cis-gcp" "4.0" "4.4" deviates {
  resources = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  reason    = "A service here depends on metadata SSH keys; enforcing OS Login breaks it."
  duty_reassess = "Re-assess when that service supports OS Login."
}
```

**Include a bare list as a map's content**, or **conditionally**
(`tests/smoke/yaml/showcase.satz`, lines 48–54):

```
use "showcase-policies.satz" as google_org_policy_policy
use "showcase-optional.satz" when want_optional
```

Fourteen organisations run that same pristine pack today. The differences
between them are params and, on three of them, a `.local` fork; everything else
is byte-identical to upstream and `check-presets` proves it.

---

## 4. What makes the layers possible

Each layer exists because a handful of language features make it expressible.

**Params with interpolation** are why a pack can be pristine and still fit a
customer. Outer beats inner; everything globally unique derives from
`customer_shortname`, everything org-scoped from `customer_organization_id`.

**Schema-typed resources** are why the fold is safe. Block keys are matched
exactly against the loaded provider schemas, so the compiler tells a resource
map from a nested attribute block without guessing, and an unknown type is a
parse-time error, not a plan-time one.

**`use` and the ⊕ fold** are why an estate is a composition rather than a copy.
Two files defining the *same address differently* is a conflict naming both
locations — composition is something the compiler checks.

**`suppress`** is why customisation stays small: subtractive, one line, and a
hard error when it matches nothing.

**`claim` / `deviates`** are why the controls layer is language, not a sidecar:
read from the same compile, naming emitted addresses.

**`hcl trust`** is why the escape hatch is visible: raw HCL warns on every
transpile until someone writes down why.

**Provenance by suffix** is why updates are safe on a fleet: the filename says
who owns the file, the tooling enforces it, and the diff file is regenerated on
every merge.

---

## 5. How much shorter, measured

Three numbers, all from the fleet on 2026-08-24, counting code lines (blank and
comment-only lines removed on both sides):

| what is compared | median | range | typical estate |
|---|---|---|---|
| **the estate file a customer writes** vs the HCL it becomes | **6.3×** | 2.9× – 10.6× | estate 8: 174 → 1 100 lines, 127 resources |
| all Satz sources incl. packs vs HCL | 1.8× | 1.4× – 3.7× | estate 8: 618 → 1 100 |
| the CIS pack alone vs its 18 policies' HCL | 0.66× | — | 298 → 197 |

The first row is the honest form of "much shorter": what you maintain per
organisation is a sixth of what runs. The high end (estate 1, estate 2 at ~10×) are the
estates that `use` the most pristine packs; the low end (estate 15, estate 5 at ~3×) are
estates with forked or inlined packs.

The second row explains the first. Packs are *not* short — they carry their
params' reasoning as comments and their control claims in-file, none of which
emits HCL — and the third row is the extreme case: the CIS pack is longer than
the policies it produces, because 66 of its lines are claims and 40 are the
reasoning behind two lists. That is the trade: the source is where the
compliance story lives, so the source is bigger than the output would suggest.

Where line count is not the point, **what you have to touch** is:

| to… | HCL / the YAML dialect | Satz |
|---|---|---|
| decline one control a pack provides | fork the pack | one `suppress` line in the estate |
| override one preset value | copy-edit, or define the anchor above the include in order | bind the param, any order |
| say why a control is not met | nowhere | `claim … deviates { reason = "…" }` |
| know whether a pack changed upstream | diff by hand | `check-presets` — canonical form of the parsed pack |
| keep the compliance story with the code | a sidecar file | claims in the pack, same compile |

---

## 6. Language reference

Two complete estates compile in CI on every push and are the examples the
sections below cite instead of carrying loose snippets:

- **the smallest complete estate** — `tests/corpus/override-chain/main.satz`
  (18 lines: header, `params`, `terraform`, one `use`, one `use … when` with a
  declared-false switch) with `pack.satz` and `optional.satz` beside it;
  snapshot-gated (`tests/corpus/override-chain/expected.sorted.txt`).
- **the showcase** — `tests/smoke/yaml/showcase.satz` with
  `showcase-pack.satz`, `showcase-policies.satz`, `showcase-optional.satz`:
  comments of all three kinds, params of every shape, `terraform` +
  `providers`, `use` in all three positions (`as`, `when`), two `suppress`
  forms, a group with a member and an `"import-id"`, grants incl. a
  conditional one, folder → project → bucket nesting with a list-of-objects
  block, a bucket-scoped grant in both forms (labelled and member map with its
  own scope), `hcl trust`, and claims of all three
  kinds with duties and an `interpretation`. `scripts/smoke.sh` transpiles it,
  validates the HCL and checks each feature's effect.

Every snippet in this section is either one of those files or compiles the
same way (`terraform { backend { … } }` is required by the emitter — the
snippets omit it for brevity where they are fragments of a larger estate).

Every example in this section compiled on 2026-08-24 against the full
google/google-beta 7.12.0 schema; where HCL is shown, it is the emitted text.

### 6.1 Lexical structure

**Comments**

```
# hash to end of line
// slashes to end of line
/* block comment,
   may span lines */
```

**Identifiers** — `[A-Za-z_][A-Za-z0-9_.]*`, conventionally `snake_case`; the
dot is for dotted pack names (`pack monitoring.audit_logsink`). Used for param
names, block keywords, resource types, map keys and param references.

**Numbers and booleans** — bare literals (`400`, `1.5`) and `true` / `false`.

**Strings** — single-line, double-quoted; multi-line, triple-quoted (the only
form that may contain a raw newline; `{param}` interpolates in both forms —
the doubled-brace escape is under **Interpolation** below):

```
"europe-west3"

"""
first line
second line
"""
```

**Escapes** (single-line only): `\n`, `\"`, `\\`. Any other escape is an error.

**Interpolation** — `{param_name}` inside any string splices a param's value.
The name must be `[A-Za-z0-9_]+` and terminated by `}`.

```
parent = "organizations/{customer_organization_id}"
email  = "essential-contacts-all@{customer_domain}"
```

**Literal braces are doubled**, because Terraform's `${…}` and JSON policy
parameters both contain braces:

```
parameters = "{{\"allowedDomains\" : [\"@{customer_domain}\"]}}"
             ↑↑ literal { }                      ↑ interpolated param

value = "${{google_project.x.project_id}}"   # a literal Terraform reference
```

Rule of thumb: **`{x}` is Satz, `{{` is a brace you want to survive.**

**A reference must name something the estate emits.** Every `${{…}}` is checked
against what was actually emitted — including a project's expanded services and
an exploded grant, which are emitted addresses that were never written as
blocks. A reference naming nothing is a compile error that says where it was
written and lists the labels of that type that do exist:

```
references to resources this estate does not emit:
  yaml/main.satz:26: google_service_account_iam_member writes `${google_service_account.onbaording.name}`
    emitted `google_service_account` labels: onboarding
```

Without the check a typo compiles: Terraform catches most of them one cycle
later, pointing at generated HCL instead of the line someone wrote, and the one
it cannot catch is a typo that happens to name a different real resource.
Suppressing a resource something else references is the same error, which is
the point — the estate no longer emits it.

**A reference is understood, not just emitted.** When a value is *nothing but*
one reference — `service_account_id = "${{google_service_account.onb.name}}"` —
the emission manifest records it as a reference, so `satz adopt` follows it to
the resource it names and `report-compliance` scopes a witness through it. When
the reference is *embedded* in a longer string —
`"principalSet://…/projects/${{google_project.mgmt.number}}/…"` — it is not a
reference to a resource but a string that mentions one, and its value is only
known after apply. Those are reported as unresolvable, naming the reference,
rather than matched against live state as if the `${…}` text were literal.

### 6.2 File structure

```ebnf
file    := [ header ] { item }
header  := ("estate" | "pack") IDENT { "content" | "version" STRING }
item    := "params" "{" { param } "}"
         | "use" STRING [ "as" IDENT ] [ "when" IDENT ]
         | "claim" STRING STRING STRING COVERAGE "{" { claim-entry } "}"
         | "suppress" IDENT STRING [ "role" STRING ]
         | "hcl" [ "trust" STRING ] "{" … "}"
         | "action" STRING "{" { action-entry } "}"
         | block
```

**Header**

```
estate acme                        # a customer estate
pack   monitoring.audit_logsink version "1.1"
pack   essential_contacts_organization version "1.1" content
```

- `estate` — one customer organisation. Usually one per repo.
- `pack` — a reusable unit. `version` is the pack's own revision, deliberately
  **in-file, never in the filename** (framework versions live in claims and are
  orthogonal: several pack revisions may implement the same standard).
- `content` — marks a pack that is *expected* to be edited per customer; forking
  it to `<name>.local.satz` is the normal workflow. Reporting tone only.

The header is optional in fragment files that are only ever `use`d.

### 6.3 Params

Params are **declarations with defaults**:

```
params {
  customer_organization_id = "123456789012"
  customer_shortname       = "acme"
  retention_days           = 400
  versioning_enabled       = true
  extra_members            = []
  audit_bucket_name        = "{customer_shortname}-audit-001"
  log_bucket_name          = audit_bucket_name       # bare identifier = param reference
}
```

Resolution rules:

1. A value bound by the *using* document wins over the file's own default
   (outer beats inner — that is what makes a pack configurable).
2. Params may reference each other **regardless of declaration order**; the
   compiler sorts by dependency.
3. The namespace is one document-ordered space: packs see every earlier file's
   params. (True lexical pack scoping is a deliberate future change, not v0.)
4. Overriding a **list replaces it** — v0 has no concatenation — so an estate
   that adds to a pack's list repeats the entries it keeps.

What the compiler does with them: every param becomes a typed `variable` in
`variables.tf` (underscores → hyphens; `[]` is typed `list(string)`) with its
resolved value in `terraform.tfvars`, for anyone reading the HCL — but a
resource that references a param is emitted with the **literal**, never `var.x`:

```hcl
# variables.tf
variable "audit-bucket-name" { type = string }
variable "retention-days"    { type = number }
variable "extra-members"     { type = list(string) }

# terraform.tfvars
audit-bucket-name = "acme-audit-001"
log-bucket-name   = "acme-audit-001"      # the alias resolved fully

# main.tf — the resource carries the value
resource "google_storage_bucket" "probe" {
  provider = google.google
  name = "acme-audit-001"
  location = "EU"
}
```

A pack's params are its contract, and the comment travels with the default.
From the shipped CIS pack:

```
pack CIS_GCP_Foundation_4_0 version "2.1"

params {
  // essentialcontacts.managed.allowedContactDomains: the domain whose addresses
  // may be set as essential contacts. Default = the customer's own domain. Some
  // customers keep their contact addresses on a DIFFERENT domain than the org
  // (estate 3: org example.com, contacts @example.net) — set it here rather
  // than forking the pack.
  essential_contacts_allowed_domain = customer_domain
}
```

### 6.4 Blocks, entries and resources

```ebnf
block   := KEY [ NAME ] "{" { entry } "}"
entry   := KEY "=" value                  attribute
         | KEY "{" { entry } "}"          nested mapping
         | KEY NAME "{" { entry } "}"     named map entry
         | "use" STRING [ … ]             include inside this mapping
KEY     := IDENT | STRING                 (a string key may interpolate)
NAME    := IDENT | STRING
value   := STRING | NUMBER | true | false | IDENT
         | "[" [ value { "," value } [ "," ] ] "]"
         | "{" { entry } "}"
```

List items may be separated by commas **or** newlines; a trailing comma is
allowed.

**The resource types are the ones the provider schemas declare**, matched
exactly: `google_org_policy_policy`, `google_folder`, `google_project`. A key
the loaded schemas do not know is a hard error. That applies to types nested
inside a `google_project { … }` or `google_folder { … }` body as well, which
are otherwise indistinguishable from a nested attribute block such as
`labels { … }` — the schema is what tells the two apart.

The only bare block keywords are Satz's own: `estate`, `pack`, `params`,
`terraform`, `providers`, `use`, `suppress`, `claim`, `hcl`.

**Simple** — one org policy:

```
google_org_policy_policy {
  "compute-managed-requireOsLogin" {
    name   = "compute.managed.requireOsLogin"
    parent = "organizations/{customer_organization_id}"
    spec {
      rules = [ { enforce = "TRUE" } ]
    }
  }
}
```

```hcl
resource "google_org_policy_policy" "compute_managed_requireOsLogin" {
  provider = google.google
  name = "organizations/123456789012/policies/compute.managed.requireOsLogin"
  parent = "organizations/123456789012"

  spec {
    rules {
      enforce = "TRUE"
    }
  }
}
```

The label has a hyphen, so it is quoted; the address has an underscore, and
that address — `google_org_policy_policy.compute_managed_requireOsLogin` — is
what a claim names.

**Proving** — a bucket with a nested block, a single block, and a *repeated*
block:

```
google_storage_bucket {
  audit_logs {
    name     = audit_bucket_name
    location = "EU"
    uniform_bucket_level_access = true
    versioning       { enabled = true }
    retention_policy { retention_period = 34560000 }
    lifecycle_rule = [
      {
        action    { type = "Delete" }
        condition { age = 730 }
      },
      {
        action    { type = "SetStorageClass" storage_class = "COLDLINE" }
        condition { age = 90 }
      }
    ]
  }
}
```

```hcl
resource "google_storage_bucket" "audit_logs" {
  provider = google.google
  name = "acme-audit-001"
  location = "EU"
  uniform_bucket_level_access = true

  versioning {
    enabled = true
  }

  retention_policy {
    retention_period = 34560000
  }

  lifecycle_rule {
    action {
      type = "Delete"
    }

    condition {
      age = 730
    }
  }

  lifecycle_rule {
    action {
      type = "SetStorageClass"
      storage_class = "COLDLINE"
    }

    condition {
      age = 90
    }
  }
}
```

**A repeated block is a list of objects.** `lifecycle_rule { … }` written
twice in one body is an error naming both lines (a repeat used to keep only
the last one, silently). The list form is the only one. Resource-type maps
(`google_…`) may repeat — two `google_org_policy_policy { … }` groups in one
file are one map, folded by address.

`google_cloud_identity_group` is a satz abstraction that shares a Terraform
type name: it is expanded into a group resource (deriving `group_key`, `parent =
customers/<id>` and the discussion_forum/security labels) rather than passed
through.
Every group block also carries `lifecycle { ignore_changes = [initial_group_config] }`
(merged with a lifecycle you declare): `initial_group_config` is create-only and a
live group does not report it, so without this an *adopted* group would plan as
"must be replaced" — destroyed and recreated with its memberships.

### 6.5 IAM grants

Grant resources take a **member → roles** shape. The member is the key, usually
interpolated; a role is a string, or an object when it carries a condition:

```
google_organization_iam_member {
  "group:gcp-org-admins@{customer_domain}" = [
    "roles/resourcemanager.organizationAdmin",
    "roles/iam.organizationRoleAdmin",
  ]
  "group:gcp-auditors@{customer_domain}" = [
    {
      role = "roles/storage.objectViewer"
      condition {
        title      = "audit-objects-only"
        expression = "resource.name.startsWith(\"projects/_/buckets/x/objects/y\")"
      }
    },
  ]
}
```

```hcl
resource "google_organization_iam_member" "iam_group_gcp_auditors_example_com_96115764c9f71ca9" {
  role = "roles/storage.objectViewer"
  member = "group:gcp-auditors@example.com"
  org_id = "123456789012"

  condition {
    title = "audit-objects-only"
    expression = "resource.name.startsWith(\"projects/_/buckets/x/objects/y\")"
  }

  provider = google.google
}

resource "google_organization_iam_member" "iam_group_gcp_org_admins_example_com_3fb0828564be6711" {
  role = "roles/iam.organizationRoleAdmin"
  member = "group:gcp-org-admins@example.com"
  org_id = "123456789012"
  provider = google.google
}

resource "google_organization_iam_member" "iam_group_gcp_org_admins_example_com_21d2774f810ee5df" {
  role = "roles/resourcemanager.organizationAdmin"
  member = "group:gcp-org-admins@example.com"
  org_id = "123456789012"
  provider = google.google
}
```

One resource per (member, role, condition). `org_id` comes from
`customer_organization_id`; inside a `google_project { … }` it is `project =
google_project.<label>.project_id` and a per-project provider alias instead.
The condition is hashed into the address, so the same role to the same member
under two conditions is two resources. Emission order is by address, not by
source order.

**Every `*_iam_member` type takes the member map.** The organisation's scope
comes from `customer_organization_id`, a project's or folder's from the node the
map is written in, and every other type writes its scope in the map — one key
that cannot be a member, because a member is always `<type>:<value>` (or one of
the two reserved forms `allUsers` and `allAuthenticatedUsers`):

```
google_storage_bucket_iam_member {
  bucket = "${{google_storage_bucket.audit_logs.name}}"
  "group:gcp-auditors@{customer_domain}" = [
    "roles/storage.objectViewer",
  ]
}

google_service_account_iam_member {
  service_account_id = "${{google_service_account.onboarding.name}}"
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/iam.workloadIdentityUser",
  ]
}
```

The scope **namespaces the grant**, so a second map for a second bucket is a
second grant even when the member and role are identical; and it enters the
emitted label, so the two never collide. One scope per map — repeat the map for
the next one. Order inside the map does not matter: the scope is read before the
members. A misspelt scope attribute is a compile error naming the type's real
one, and a scope on a type that already has one (`org_id` on an organisation
grant, `project` on a map inside a project) is refused rather than silently
ignored. `google_billing_account_iam_member` is the exception that proves the
rule: its `billing_account_id` is estate-wide, pinned once and defaulting to
`billing_account_infra`, not per-map.

The same grant as a **labelled resource** is equally valid, and is the form to
use for a single edge or when the scope differs per member:

```
google_storage_bucket_iam_member {
  audit_viewer {
    bucket = "${{google_storage_bucket.audit_logs.name}}"
    role   = "roles/storage.objectViewer"
    member = "group:gcp-auditors@{customer_domain}"
  }
}
```

Memberships stay **out of packs**: packs define groups, humans grant membership.

### 6.6 Folders and hierarchy

Nesting **is** the parent reference — `parent` and `folder_id` are never written:

```
google_folder {
  workloads {
    display_name = "Workloads"
    google_folder {
      team_alpha {
        display_name = "Team Alpha"
        google_project {
          alpha_prod {
            name       = "{customer_shortname}-alpha-prod"
            project_id = "{customer_shortname}-alpha-prod-001"
            labels { env = "prod" }
          }
        }
      }
    }
  }
}
```

```hcl
resource "google_folder" "workloads" {
  display_name = "Workloads"
  parent = "organizations/123456789012"
  provider = google.google
}

resource "google_folder" "team_alpha" {
  display_name = "Team Alpha"
  parent = google_folder.workloads.name
  provider = google.google
}

resource "google_project" "alpha_prod" {
  project_id = "acme-alpha-prod-001"
  name = "acme-alpha-prod"
  provider = google.google
  folder_id = google_folder.team_alpha.name
  labels = {
    "env" = "prod"
  }
}
```

Read the two `{ }` bodies inside `alpha_prod` against each other:
`google_folder { … }` is a schema type and became a resource;
`labels { … }` is an attribute and became `labels = { … }`. Same syntax; the
schema decided.

### 6.7 Adoption of existing resources

`"import-id"` records the live id so the tool adopts rather than creates. It
is the only adoption surface in the language — the *result* of resolving a
live resource, declarative and visible in `tofu plan` — and it is honoured on
every resource the compiler emits:

```
google_folder {
  workloads {
    "import-id"  = "folders/123456789"
    display_name = "workloads"
    google_project {
      infra {
        "import-id"     = "acme-infra-001"
        project_id      = "acme-infra-001"
        project_service = [
          "logging.googleapis.com",
          { service = "storage.googleapis.com" "import-id" = "acme-infra-001/storage.googleapis.com" },
        ]
      }
    }
  }
}

google_organization_iam_member {
  "group:gcp-org-admins@{customer_domain}" = [
    "roles/viewer",
    { role = "roles/browser" "import-id" = "123456789012 roles/browser group:gcp-org-admins@example.com" },
  ]
}

google_cloud_identity_group {
  gcp_auditors {
    "import-id" = "groups/00abc"
    member = [
      "user:a@{customer_domain}",
      { id = "user:b@{customer_domain}" "import-id" = "groups/00abc/memberships/111" },
    ]
  }
}
```

Every `"import-id"` becomes one `import { to = <address> id = "…" }` block in
`hcl/imports.tf`, addressed exactly as the resource is emitted (hashed labels
for bindings and memberships included), and is stripped from the resource
body. Where the resource is an *entry* rather than a block — a role in a
grant list, a service in `project_service`, a member of a group — the entry
takes its object form and carries the id there. An IAM binding declared with
and without an id (a pack's grant that the estate adopts) is one resource;
two different ids for the same binding is an error.

You rarely write these by hand: `satz adopt <estate>` resolves the live ids of
everything the estate declares — folders by display name under their parent,
groups by email, org policies by constraint (activating managed constraints
with `--activate`), any other GCP-assigned id through Cloud Asset Inventory
under the resource's own scope on the row's `match_on` attributes (contacts by
email, alert policies by display name), user-chosen ids from the `import_id`
templates in `presets/import-config.yaml` — and `--execute` writes them back: an
`"import-id"` line into a block, the object form into a list entry. Derived
ids are written too; `tofu plan` verifies each through its import block. Two
things it will not do: rewrite an entry it cannot find in the source (an
interpolated member), and edit a **pristine pack** — packs are upstream-owned,
so their resources come back as hints (`--execute --import`, or fork the pack).
A resolution with more than one live candidate is reported as ambiguous and
left for you to pin; nothing is ever guessed.

### 6.8 Estate configuration blocks

`terraform` and `providers` are configuration, not resources. **`terraform` is
required** — an estate without it does not compile (`Missing 'terraform' block`).
A backend may list both `local` and `gcs`; the emitter writes the ONE that
`deployment_mode` selects (`"local"` / `"cloud"`, see the `migrate` command),
never both:

```
terraform {
  backend {
    local { path = "terraform.tfstate" }
    gcs {
      bucket = infra_bucket_name
      prefix = "hcl/state"
    }
  }
}

providers {
  "google" {
    project               = infra_project_name
    region                = default_region
    alias                 = "google"
    user_project_override = true
    billing_project       = infra_project_name
  }
}
```

### 6.9 `use` — composition

```
google_org_policy_policy { use "presets/CIS-GCP-Foundation-4.0.satz" }   # inside a map
use "presets/CIS-GCP-Foundation-4.0.satz" as google_org_policy_policy    # as: same thing
use "showcase-optional.satz" when want_optional                          # conditionally
```

(The CIS pack is a bare list of policy labels, so it needs the map — bare at
the top level it is `unknown resource type`; a pack whose entries are typed
maps, like `showcase-pack.satz`, is `use`d bare.)

- **Path** is a plain string, never interpolated. Resolved relative to the using
  file first, then the configured `include_dirs`.
- **`as <key>`** — the pack's top-level entries become the *content* of a
  resource map keyed by `<key>`. For a pack that is a bare list of resources.
- **`when <param>`** — the pack is pulled in only if the param is truthy. A
  skipped pack contributes nothing: no resources, no params, no claims.

`use` is valid at three places, and all three behave identically with respect
to params, claims and `hcl` blocks: top level; inside a `google_folder { … }`
block (where `as` is honoured too — the pack becomes that resource map, scoped
to the folder); and inside a resource map, **the most common form in real
estates** (there the map's type IS the key: an `as` naming another type is an
error). A `use` cycle is an error naming the chain; `when` on a param no file
declares is an error, not `false`; a top-level attribute in a file `use`d
without `as` is an error.

**Proving** — the showcase does exactly this. `showcase-pack.satz` declares
`pack_bucket_location = "EU"` and a bucket that uses it; the estate binds
`pack_bucket_location = "europe-west3"` and `use`s the pack bare, so the bucket
emits with `europe-west3` (remove the estate's param and it emits `"EU"`).
`showcase-policies.satz` is a bare list of three policy labels, keyed by
`use "showcase-policies.satz" as google_org_policy_policy`; one of the three is
then removed by `suppress` (§6.10) and one is the subject of a `deviates`
claim (§6.11). `showcase-optional.satz` sits behind `when want_optional`,
declared `false`, and contributes nothing — the smoke run asserts its bucket
is absent.

Composition is a **fold**: two files may contribute to the same resource map
and the results merge. Two files defining the *same address differently* is a
conflict and a hard error naming both source locations.

### 6.10 `suppress` — the subtractive channel

An estate declines something a pack provides, without forking the pack:

```
suppress google_org_policy_policy "compute-managed-requireOsLogin"
suppress google_organization_iam_member "group:sec@{customer_domain}" role "roles/browser"
```

- Type is the **full** Terraform type name; the label may interpolate.
- `role "<role>"` narrows the suppression to one edge of a grant instead of the
  whole member.
- A grant inside a folder or project is addressed by its node: a bare member
  (`"group:x@…"`) suppresses that member on EVERY node that grants it; a
  node-qualified label (`"shared/prod::group:x@…"`, the folder/project labels
  joined by `/`) suppresses it on that one node. A grant that writes its own
  scope is addressed the same way, by that scope:
  `suppress google_storage_bucket_iam_member "bucket=acme-audit::group:x@…"`.
- `role` on an address that is in conflict (⊥) is an error: suppress the whole
  member, or resolve the conflict.

Against the estate above, the first line removes the policy from `main.tf` and
nothing else changes. Suppressions apply before conflict detection, so
suppressing a conflicted address resolves the conflict.

A suppression that matches nothing is a **hard error**:

```
suppress google_org_policy_policy "compute-managed-noSuchPolicy" matches nothing — stale suppression (typo or upstream rename)
```

A stale suppression silently doing nothing is exactly the failure this channel
exists to prevent, and it has earned its keep once already: when the CIS pack
retired the legacy §1.1 constraint in v2.0, every estate still carrying a
`suppress` for it failed on the next transpile instead of keeping a dead line
forever.

### 6.11 `claim` — the compliance plane

Claims are language syntax, read directly from the source by `require` and
`report-compliance`. They leave no trace in `main.tf`.

```
claim "cis-gcp" "4.0" "1.4" implements {
  resources = [
    "google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation",
    "google_org_policy_policy.iam_managed_disableServiceAccountKeyUpload",
  ]
  interpretation       = "Service account keys cannot be created or uploaded."
  duty_rotate_existing = "Existing user-managed keys must be removed by hand."
}
```

```ebnf
claim FRAMEWORK VERSION CONTROL COVERAGE "{" { claim-entry } "}"
COVERAGE    := "implements" | "contributes" | "deviates"
claim-entry := "resources" "=" "[" { STRING } "]"
             | "interpretation" "=" STRING
             | "reason" "=" STRING            (deviates only, required)
             | "duty_" IDENT "=" STRING
```

- The three header strings are plain — **no interpolation**; control ids are
  static.
- `resources` are emitted Terraform addresses. A claim whose witnesses are not
  emitted is a **broken claim** (‼), reported loudly, never silently satisfied.
- `duty_<name>` records a manual duty; underscores become hyphens in reports
  (`duty_validate_then_lock` → `validate-then-lock`).
- `implements` discharges the control; `contributes` is a necessary part.

The catalog the claim is judged against is data
(`presets/catalogs/cis-gcp-4.0.yaml`): control ids, this project's own
paraphrases, and an `automatability` (`technical` / `partial` /
`organizational`) — never framework prose, which is licence-restricted.

A duty is where the claim records what code cannot do. From the shipped pack:

```
claim "cis-gcp" "4.0" "4.4" implements {
  resources = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  interpretation    = "OS Login is required, so VM SSH access is governed by IAM rather than metadata keys."
  duty_existing_vms = "Enforcing OS Login can cut existing SSH access patterns; verify before enabling on an org with running VMs."
}
```

Until that duty is attested (§8), the control reads ◐ *partial (open duty)* —
witnesses present, human step outstanding.

**`deviates` — declining a control on purpose**

```
claim "cis-gcp" "4.0" "4.4" deviates {
  resources = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  reason    = "A service here depends on metadata SSH keys; enforcing OS Login breaks it."
  duty_reassess = "Re-assess when that service supports OS Login."
}
```

`reason` is **mandatory** on a deviation and **rejected** on the other kinds.
Leave it out and the compile stops:

```
claim … deviates: reason = "…" is required (a deviation is a disclosed decision, and the report carries the reason)
```

Witnesses are optional here — the resource may be present-but-not-enforcing, or
absent because the estate suppressed it — but any witness the claim *does*
declare must still be emitted, so deleting the policy outright resurfaces as a
broken claim rather than staying silently "deviated". A deviation renders as ⚠
with its reason, is counted separately, and does **not** fail the `require`
gate: a disclosed decision, not a gap. It outranks the claims it contradicts
and can be declared by a pack fork or by the estate itself.

### 6.12 `hcl` — raw passthrough

```
hcl {
  resource "google_compute_address" "legacy" {
    name   = "legacy-ip"
    region = "europe-west3"
  }
}

hcl trust "reviewed 2026-08-24, provider gap for static IPs" {
  resource "google_compute_address" "legacy_trusted" {
    name   = "legacy-ip-2"
    region = "europe-west3"
  }
}
```

The body is captured verbatim, is **never interpolated**, and bypasses the fold
entirely — which is what "opaque to the proof layer" means. Every transpile says
so:

```
warning: raw HCL passthrough at yaml/main.satz:14 (4 lines) emitted verbatim — opaque to the compliance plane; no claim can cover it. Add `hcl trust "<reason>" { … }` once reviewed.
note: raw HCL passthrough at yaml/main.satz:21 (4 lines) — trusted: reviewed 2026-08-24, provider gap for static IPs
```

and so does the output:

```hcl
# --- raw HCL passthrough from yaml/main.satz:21 ---
# Opaque to the compliance plane: no claim covers what is written here.
# trusted: reviewed 2026-08-24, provider gap for static IPs
resource "google_compute_address" "legacy_trusted" {
  name   = "legacy-ip-2"
  region = "europe-west3"
}
```

Escape hatch by design, visible by design. "Opaque" is a property the compiler
enforces, not a convention: the compliance plane reads the *emission manifest*
— the resources the compiler itself built — never the rendered `main.tf`, and
the passthrough is appended to the text after emission. A resource that exists
only inside `hcl { … }` therefore deploys but is **not a witness**; a claim
that names it reports **‼ broken claim**, exactly as if the resource were
missing.

#### Running a script from inside the apply

Because the body is verbatim Terraform, the passthrough is also the way to put a
script *in the dependency graph* — the one thing [`action`](#613-action--a-step-with-no-provider-resource)
cannot do:

```
hcl trust "SCC enablement has no provider resource (google 7.14.1)" {
  resource "terraform_data" "scc_services" {
    triggers_replace = [var.customer_organization_id]
    provisioner "local-exec" {
      command = "${path.module}/../presets/scc/scc-enable-all.sh --organization ${var.customer_organization_id} --apply"
    }
  }
}
```

This orders the script against satz-emitted resources through ordinary HCL
references, which nothing else here can. It also costs more than it looks:
`tofu plan` cannot show what the script will do, a failure taints state and has
to be cleaned up by hand, the script runs on whichever machine runs `apply`, and
the block is invisible to the compliance plane like any other passthrough.
Reach for it only when a step genuinely has to happen *between* two resources in
one apply; otherwise declare an `action`, which is legible, selectable, and runs
where you can watch it.

### 6.13 `action` — a step with no provider resource

Some cloud steps cannot be declared at all. Security Command Center service
enablement is the standing example: as of provider 7.14.1 there are 35
`google_scc_*` / `google_securityposture_*` types and **none** of them is service
enablement or tier activation, so no language satz compiles can express it. The
step is still part of the deployment.

`action` names such a step, binds it to a script, and builds its arguments from
the estate's own parameters:

```
action "scc-services" {
  reason       = "SCC service enablement has no provider resource (google 7.14.1)"
  run          = "scc-enable-all.sh"
  args         = ["--organization", "{customer_organization_id}"]
  execute_args = ["--apply"]
  phase        = "before-apply"
}
```

That exact case ships as a pack, so an estate does not have to write it at all —
`use "presets/scc/scc-service-enablement.satz"` is the whole binding, and the
declaration above is that pack's, with its `reason` shortened to fit here.

| key | required | meaning |
|---|---|---|
| `reason` | yes | Why this is not a resource. Quoted back in every warning. |
| `run` | yes | The executable. Resolved relative to the directory of the file that **declares** it — so a pack that ships a script is self-contained — then against the include dirs, exactly as a `use` path is. Never interpolated. |
| `args` | no | Always passed. `{param}` interpolates; an unknown param is a hard error. |
| `execute_args` | no | Appended **only** under `--execute`. This is where a script's `--apply` lives. |
| `phase` | no | `before-apply` for a prerequisite, `after-apply` for a step that needs what the apply created. Default `after-apply`. Advisory: it orders the run and selects with `--phase`, but nothing chains `satz apply` — see below. |

#### Which phase, and what `phase` does not do

The two values answer one question: does this step have to happen **before** the
estate exists, or **after**?

**`before-apply` — a prerequisite.** The step has to be true before the apply can
succeed, so it cannot wait for it. Enablement is the archetype: the SCC pack above
turns the services on ahead of the resources that depend on them.

**`after-apply` — the step needs what the apply created.** It cannot run earlier,
because the thing it operates on does not exist yet. A per-project setting with no
provider binding is the plain case — it cannot be applied to a project the apply
has not created:

```
action "seed-project-settings" {
  reason       = "the setting has no provider resource, and the project it targets is created by this estate"
  run          = "../scripts/seed-settings.sh"
  args         = ["--project", "{infra_project_name}"]
  execute_args = ["--apply"]
  phase        = "after-apply"
}
```

`after-apply` is the default, so the second form needs no `phase` line at all;
write it when it helps a reader, and write `before-apply` whenever the step is a
prerequisite.

**What `phase` does not do:** it does not run anything around `satz apply`.
`satz apply` neither knows nor mentions actions, so nothing enforces that a
`before-apply` action actually ran before the apply. The value orders the run
(`before-apply` first, then declaration order) and selects with
`--phase before-apply|after-apply`; sequencing the two against the apply is the
operator's job:

```bash
satz run-actions estate.satz --phase before-apply --execute
satz apply
satz run-actions estate.satz --phase after-apply --execute
```

**Nothing here runs while compiling.** `transpile` is a pure function of its
sources and stays one: that is what lets the corpus snapshots, `check-presets`
and the auto-fork transpile-identity proof mean what they say, and it is why
cloning an estate and compiling it cannot execute anything. An action is inert
until [`satz run-actions`](#run-actions) is invoked.

**An action emits nothing and can carry no claim.** It is not in `main.tf`, not
in the emission manifest, and no `claim` can name it — the same opacity `hcl`
has, with execution on top. Satz records that the step exists and never says
what it did; nothing about an action reaches `report-compliance`.

Every compile of an estate that declares one says so, and unlike `hcl trust`, a
`reason` does not downgrade the warning — HCL only deploys, an action runs:

```
warning: action "scc-services" declared in presets/scc/enable.satz:12 (from a pack) — `satz run-actions` will execute enable.sh
  reason: SCC service enablement has no provider resource (google 7.14.1)
note: --no-pack-actions ignores pack-declared actions, --no-actions disables all execution, --no-action-warnings silences this.
```

A pack may declare an action, and the warning says when one did. That is
deliberate — the pack is where the knowledge that a step is needed lives — and
it is why the three switches exist: `get-presets` downloads packs from a public
repository. A downloaded script also arrives without its executable bit, and
satz will not set it; the error names the `chmod +x` and leaves the decision to
whoever read the file.

Names are unique across the estate. Two actions answering to one name is a hard
error naming both files, the rule the ⊕ fold already applies to a repeated
address. An action inside a pack that `use … when` switched off is never
collected at all.

#### When an action runs, and how

**Never on its own.** `satz run-actions` is the only thing that executes one:
not `transpile`, not `plan`, not `apply` — `satz apply` does not know actions
exist. `phase` orders the run and selects with `--phase`; it does not make
anything happen around an apply.

When `run-actions` does run, in this order:

1. **The estate is compiled first.** If it does not compile, nothing runs — an
   estate whose parameters cannot be trusted is not one to build a command line
   from.
2. **`{param}` resolves** against the finished namespace. An unknown param is a
   hard error, never an empty argument.
3. **The executable is located** relative to the directory of the file that
   declared it, then against the include dirs — the same search a `use` path
   gets.
4. **Every action is located and vetted before any one is spawned** (it exists,
   it is executable). A run must not die half way on the fourth script's missing
   `+x` when the first three already changed the organisation.
5. **Each is spawned**, in phase order (`before-apply`, then `after-apply`) and
   declaration order within a phase — the estate's own actions first, then
   `use`-visit order.
6. **A non-zero exit stops the run** and satz exits with that code. The
   remaining actions do not run: a failed step is not a reason to keep changing
   the organisation.

#### Writing a script for an action

The contract is small, and satz guarantees all of it:

| | |
|---|---|
| **interpreter** | Whatever the file's shebang says. Satz executes the file; it does not know or care whether the target is `sh`, `bash`, Python or a compiled binary. A Python script with dependencies can use `#!/usr/bin/env -S uv run --script` and PEP 723 inline metadata. |
| **executable bit** | Required. Satz never sets it — the error names the `chmod +x`. |
| **working directory** | Always the directory holding `config.toml`, whatever directory the operator invoked satz from. Never assume the caller's cwd. |
| **arguments** | `args`, plus `execute_args` appended under `--execute`. |
| **environment** | Exactly five variables, and **nothing else**: `SATZ_ACTION` (the name), `SATZ_PHASE`, `SATZ_MODE` (`check` or `execute`), `SATZ_ESTATE` (the estate file), `SATZ_HCL_DIR`. Params are **not** exported — anything a script needs must be named in `args`, so the declaration is the complete record of what the action was told. |
| **exit code** | `0` is success. Anything else stops the run and becomes satz's exit code. |
| **stdout / stderr** | Inherited, so the script's output is the operator's output. Satz does not capture, parse or store it. |

The two-list split is what makes a script safe to point at: put the form that
**reads** in `args` and the flag that **writes** in `execute_args`, so
`run-actions --check` exercises the script's own dry run and only `--execute`
lets it write. Whether the `args` form really is side-effect-free is the
script's business — satz cannot know what a script does and does not pretend to.

A script written to that contract. `tests/smoke/scripts/showcase-action.sh` is
this shape with a few extra echoes the smoke matrix asserts on, and CI runs it on
every push:

```bash
#!/usr/bin/env bash
set -euo pipefail

# Dry run unless the estate's execute_args said otherwise. The flag is the
# script's own, not satz's: satz only decides whether to pass it.
apply=0
org=""
while [ $# -gt 0 ]; do
  case "$1" in
    --organization) org="$2"; shift 2 ;;
    --apply)        apply=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$org" ] || { echo "--organization is required" >&2; exit 2; }

echo "action ${SATZ_ACTION} (${SATZ_MODE}) on organizations/${org}"
if [ "$apply" = 0 ]; then
  echo "DRY RUN — re-run with --execute to write."
  exit 0
fi

# … the work. A non-zero exit here stops the whole run.
```

`presets/scc/scc-enable-all.sh` in this repository is the real one, and it
already has that shape — dry run by default, `--apply` to write, non-zero on any
failed call. It lives under `presets/` rather than `scripts/` because
`get-presets` ships only `presets/**`, and a pack whose script did not travel
with it would declare an action that cannot find what it runs. See
[`docs/scripts.md`](scripts.md).

### 6.14 Provenance: pristine, fork, ledger

Suffix carries meaning; the tooling enforces it.

| file | meaning |
|---|---|
| `X.satz` | pristine, upstream-owned, always overwritable |
| `X.local.satz` | customer fork, never touched by updates |
| `X.diff.satz` | the current adoption delta (fork vs pristine), rewritten each merge |

- A preset the estate **includes** never changes silently. A semantic upstream
  change (the canonical form of the parsed pack differs — params or body)
  auto-forks and repoints the estate. Comment, format and version-line churn
  upgrades silently.
- Pack versions live **in-file**; filenames carry only framework versions.
  Never `X.local.2.satz`.
- 80% of customisation should be **params**; the rest a `.local` fork. A fork
  whose entire diff could have been a param is upstream debt.

---

## 7. Proof: `require`

**What you gain:** a gate, before anything touches the cloud, that says which
controls the estate *as written* discharges — and refuses to be fooled by a
claim whose witness is not there.

```
satz require cis-gcp-4.0 C0example.satz --config ~/estates/acme
```

```
require cis-gcp 4.0 — goal view for …/acme/yaml/C0example.satz

  ◐ 1.1   Corporate login credentials only              — open duties: legacy-superseded, review-allowlist
  ✓ 1.4   Only GCP-managed service account keys         — google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation, google_org_policy_policy.iam_managed_disableServiceAccountKeyUpload
  ✓ 2.2   Sinks for all log entries                     — google_logging_organization_sink.organization_audit_gcs, google_storage_bucket.org_audit_logs, google_storage_bucket_iam_member.org_audit_sink_writer
  ◐ 2.3   Retention on the log bucket                   — no implements claim included
  ✓ 3.1   Default network does not exist                — google_org_policy_policy.compute_skipDefaultNetworkCreation
  ◐ 4.4   OS Login enabled                              — open duties: existing-vms
  ✓ 5.2   Uniform bucket-level access enabled           — google_org_policy_policy.storage_uniformBucketLevelAccess

18 satisfied, 5 partial, 0 deviation(s), 0 unmet, 0 broken claim(s). Goal view judges the DECLARED estate; live verification is the evidence report.
```

(Trimmed to seven of 23 rows.) Every ✓ carries its witnesses — the emitted
addresses the claim named and the compiler found. Rows are the catalog's
controls, string-sorted by id.

On an estate that declines two controls (estate 5, a fork with `enforce = "FALSE"`
and a `deviates` claim for each), the same command reads:

```
  ⚠ 4.4   OS Login enabled                              — DEVIATION (CIS_GCP_Foundation_4_0) Deliberate: an operational service in this organisation depends on metadata SSH keys, which enforcing OS Login would break. The constraint is declared and managed here, with enforce = FALSE, so the decision is visible in the estate rather than absent from it. open: identify-service, reassess
  ⚠ 4.6   IP forwarding not enabled on instances        — DEVIATION (CIS_GCP_Foundation_4_0) Deliberate: workloads in this organisation require IP forwarding, so the constraint is declared and managed with enforce = FALSE rather than left undeclared. open: identify-workloads, reassess

7 satisfied, 3 partial, 2 deviation(s), 11 unmet, 0 broken claim(s). …
Deviations are disclosed decisions with a stated reason, not gaps — they do not fail this gate.
```

And on an estate with no CIS pack at all (estate 7), every row says what would
provide it:

```
  ✗ 1.4   Only GCP-managed service account keys         — unmet. Provides: CIS_GCP_Foundation_4_0
  ✗ 2.2   Sinks for all log entries                     — unmet (no pack in the library provides it)
```

| glyph | status | meaning (`src/compliance.rs`) |
|---|---|---|
| ✓ | satisfied | ≥1 `implements` claim from an included pack, every witness emitted, no open duties |
| ◐ | partial | witnesses present but duties open, or only `contributes` claims |
| ⚠ | deviation | a deliberate non-conformance with a stated reason — disclosed, never counted as a gap |
| ✗ | unmet | no included claim discharges it (none, or only ones that contributed zero witnesses); names the packs in the library that would |
| ‼ | broken claim | an included claim's declared witnesses are not emitted — worse than unmet. A `deviates` claim whose declared witness vanished reads ‼ too, not ⚠; and ‼ yields to ✓/◐ when another included claim supplied the witnesses |
| ○ | organizational | the catalog marks it as having no IaC witness |

Exit code is **1 when anything is unmet or broken**, 0 otherwise. Deviations do
not fail it. That is the CI gate: an estate that drops a witness a claim
depends on fails the build, and an estate that declines a control on the
record does not.

---

## 8. Evidence: `report-compliance`

**What you gain:** the goal view joined with the cloud. Every witness of a
satisfied or partial control is looked up through Cloud Asset Inventory **in
its own scope** — a log metric by `projects/<number>/metrics/<name>` from the
`project` it is emitted with, an organization sink under the organization, a
bucket by its global name; a same-named resource in another project never
verifies a witness, and a project-scoped witness emitted without a `project`
reads *unverifiable* with that reason — and for org policies, compared **by
value**, because a policy that exists and is switched off looks healthy in
every inventory.

```
satz report-compliance cis-gcp-4.0 C0example.satz --config ~/estates/acme
```

The report has seven columns — `Control | Title | Status | Witnesses (declared →
live) | Duties | Prowler | Checkov`; the title cell carries the catalog's own
`paraphrase` of the control under it, the witness cell the `interpretation` the
included claims give of what their resources prove, and an open duty prints
its text beside its id. Three rows from estate 1's report, one of each shape
(the two tool columns omitted):

| Control | Status | Witnesses (declared → live) | Duties |
|---|---|---|---|
| 1.4 | **verified** | `google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation` → ✓ `organizations/123456789012/policies/iam.managed.disableServiceAccountKeyCreation` · `…KeyUpload` → ✓ `…KeyUpload` | – |
| 2.1 | verified* (2 of 3) | two org policies → ✓ · `google_organization_iam_audit_config.org_all_services` → – (no live check for this type yet) | – |
| 2.3 | partial (open duty) | `google_storage_bucket.org_audit_logs` → ✓ `acme-organization-audit-bucket` | open: validate-then-lock — apply the bucket lock after the 30-day validation |

Status precedence, highest first:

1. **NOT ENFORCED** — a witness is live but not doing what the estate declares
   (an org policy's `enforce` differs). Outranks DRIFTED on purpose: a missing
   resource is visibly absent; a present-but-off one is invisible.
2. **DRIFTED** — a declared witness is not live.
3. **partial (open duty)** — unattested duties remain.
4. **partial (contributes)** — only `contributes` claims.
5. **unverified (reason)** — no witness of the row could be checked at all
   (no credentials, inventory unavailable, no `project` to scope a witness).
   Never spelled "verified".
6. **verified\* (n of m)** — some witnesses matched live, the rest have no
   live check for their type yet. Stated, never faked.
7. **declared** — `--no-live`.
8. **verified** — every witness matched live.

(An inventory that was fetched and is simply empty is not "declared": the
witnesses are then *missing*, and the row reads DRIFTED.)

Plus **deviation (accepted)**, **deviation is STALE** (declared as a deviation,
but the live policy actually enforces — the fork is behind reality), **BROKEN
CLAIM**, **unmet**, and organizational. The comparison refuses to guess: a
policy with several rules, with none, or a list constraint yields no verdict
rather than a wrong one, and a policy whose live state cannot be read reports
*unverifiable*, never *verified*.

**Attestations** discharge manual duties. `attestations.yaml` beside
`config.toml`, one entry per duty id:

```yaml
validate-then-lock:
  by:   "Jane Doe"
  date: "2026-08-20"
  note: "bucket lock applied after 30-day pipeline validation"
```

An attested duty moves from `open: validate-then-lock` to
`attested: validate-then-lock (Jane Doe, 2026-08-20)` and no longer holds the
control at partial. No estate in the fleet has one yet.

**Evidence history.** Every run appends `evidence/<framework>-<timestamp>.json`
beside the config — `estate`, `framework`, `version`, `live`, `verified_at`,
and one row per control with `control`, `title`, `status`, `witnesses`,
`duties`, `paraphrase`, `interpretation`, `prowler`, `checkov` — and writes
the report (`evidence/<framework>-latest.md`, or `--format pdf`; `--format
json` writes only the history entry, no markdown). `--prowler findings.json`
ingests a Prowler export (OCSF or legacy JSON) as corroboration; `--checkov`
adds a column from a Checkov run over `hcl_dir`. `--no-live` produces a
declared-only report (statuses read *declared*) but still appends to the
history. The exit code is 0 whatever the verdicts; `--fail-on
not-enforced,drifted` (any status word, or `any`) makes the run fail for CI
after the report is written. The report states check
semantics — "a resource with these properties was verified at this time" —
never legal conformity.

---

## 9. Quick reference

| I want to… | Write |
|---|---|
| declare an estate | `estate acme` |
| declare a versioned pack | `pack monitoring.logsink version "1.2"` |
| mark a pack as per-customer content | `pack contacts version "1.1" content` |
| declare a tunable | `params { region = "europe-west3" }` |
| reference a param in a string | `"organizations/{customer_organization_id}"` |
| reference a param as a value | `bucket = infra_bucket_name` |
| write a literal brace | `"{{"` / `"}}"` |
| write a Terraform reference | `"${{google_project.x.project_id}}"` |
| include a pack | `use "presets/x.satz"` |
| include as a resource map's content | `use "presets/x.satz" as google_org_policy_policy` |
| include conditionally | `use "presets/x.satz" when want_x` |
| declare a resource | `type { "label" { attr = … } }` |
| repeat a nested block | `lifecycle_rule = [ { … }, { … } ]` |
| grant roles | `google_organization_iam_member { "group:x@{domain}" = ["roles/viewer"] }` |
| grant on a bucket, service account, … | the same map plus its scope: `google_storage_bucket_iam_member { bucket = "b" "group:x@{domain}" = ["roles/storage.objectViewer"] }` |
| grant conditionally | role becomes `{ role = "…" condition { title = … expression = … } }` |
| adopt an existing resource | `"import-id" = "folders/123"` |
| drop one pack resource | `suppress google_org_policy_policy "label"` |
| drop one grant edge | `suppress google_organization_iam_member "member" role "roles/x"` |
| claim a control | `claim "cis-gcp" "4.0" "1.4" implements { resources = [...] }` |
| record a manual duty | `duty_lock_bucket = "…"` inside a claim |
| decline a control on purpose | `claim … deviates { resources = [...] reason = "…" }` |
| escape into raw HCL | `hcl { … }` — warns unless `hcl trust "…" { … }` |
| declare a step no resource can express | `action "scc" { reason = "…" run = "../scripts/x.sh" args = ["--org", "{customer_organization_id}"] }` |
| run a script inside the apply instead | `hcl trust "…" { resource "terraform_data" … provisioner "local-exec" { … } }` |
| multi-line string | `"""…"""` |
| comment | `#`, `//`, `/* … */` |

### Commands that consume this language

| command | layer | does |
|---|---|---|
| `transpile <estate>.satz` | Satz → HCL | emit `hcl/`; `--plan` / `--apply` run the tool afterwards, `--scan` runs Checkov, `--print-variables` prints the tfvars |
| `require <framework> <estate>.satz` | Controls | goal view — declared estate vs catalog; exit 1 on unmet/broken |
| `report-compliance <framework> <estate>.satz` | Evidence | evidence report, verified against live; `--no-live`, `--prowler`, `--format pdf`, `--fail-on <statuses>` (exit code as the CI gate) |
| `check-presets <estate>.satz` | Satz | drift of packs vs upstream |
| `merge-presets` | Satz | reconcile pack updates; forks + repoints on semantic change |
| `adopt <estate>.satz [--execute] [--import] [--activate] [--only t,…]` | Satz | resolve live ids of declared resources, write `"import-id"`s or import; `adopt-org-policies` is an alias |
| `plan` / `apply` / `hcl-init` | HCL | run the configured tool (`tf_tool`, OpenTofu by default) in `hcl_dir` |
| `run-actions <estate>.satz [--check\|--execute] [--only n,…] [--phase p]` | Satz | run the estate's declared `action`s (§6.13). Prints and stops by default; `--check` runs each action's own dry-run form, `--execute` the form that writes. Global `--no-actions`, `--no-pack-actions`, `--no-action-warnings` |
| `import [<source>] [--only t,…] [--import-config f] [-o <file>] [--into <estate>] [--wrap-all] [--kind estate\|pack] [--gate <estate>] [--fork]` | — | create an estate from what exists (§12): a state file, `organizations/<n>` / `folders/<n>` / `projects/<id>` live, a directory of `.tf`, or a legacy `.yaml` file; `--from` forces the shape; `--into` imports only what the estate does not declare, as packs it `use`s; checked by `transpile` + `tofu plan` |
| `triage <framework> <estate>.satz --prowler f` | Evidence | every Prowler FAIL sorted into buckets A–E (a pack covers it / Satz declares it / declared exception / unmanaged / manual) — the remediation plan's skeleton |
| `scan [<estate>.satz]` | HCL | Checkov over `hcl_dir`, findings pointed at the Satz line that declared the resource; failed checks exit 1 |
| `doc-packs [--out d] [--check]` | Satz | one page per pristine pack derived from the pack file (purpose, params, resources, claims, duties) + index; `--check` is the CI gate |
| `map-types [--only t,…]` | — | derive the API→Terraform field map per type into `presets/type-map.yaml` (from the Discovery Documents and the provider schema) |
| `bootstrap <estate>.satz [--dry-run]` | Satz | first apply for a new organisation: management project, state bucket, service account |
| `migrate <estate>.satz --mode local\|cloud` | Satz | rewrite `deployment_mode` in the estate's params and move the state |
| `export-` / `diff-` / `report-organizational-policies <estate>.satz [--recursive]` | Evidence | the org-policy specialist tools: snapshot live policies as a pack, diff desired vs live by (parent, constraint), inventory report |

All of them accept `--config <estate-dir-or-config.toml>` and run from anywhere.
The estate file is a positional argument, relative to `yaml_dir`.

---

## 10. Errors

Every error carries the file and line and, where a fix exists, names it.
Verbatim:

```
unterminated interpolation '{custome
empty interpolation {} (use {{}} for a literal brace)
newline in single-line string (use """ for multi-line)
unterminated block comment
malformed number `1.2.3`
unknown param 'no_such_param'
params: `a` is declared twice — the second binding would be ignored
a second `estate` header (f) — the file is already `e`
use ... as: given twice
`lifecycle_rule` is given twice in this block (first at line 12) — a repeated key would silently last-win; write a list (`lifecycle_rule = [ … ]`) or remove one
block `folder`: unknown resource type. Satz names Terraform types in full — write `google_folder`. (Leaving the provider prefix off is a YAML-dialect shorthand; it is not Satz.)
`x` is an attribute at the top level of the file — attributes live inside a resource block
use … when want_cs: unknown param `want_cs` — a `when` on a param nobody declares would silently drop the pack
use … as google_cloud_identity_group inside `google_org_policy_policy { … }`: the pack is this map's content; move the `use` to the folder or top level to re-key it
use "old-pack.yaml": packs are Satz — convert it first: `satz import old-pack.yaml --kind pack`
use "x.satz": file not found
cyclic `use`: main.satz → a.satz → b.satz → a.satz
`terraform` is declared twice — one block per estate
google_org_policy_policy.p is declared twice in this file with different bodies (first at line 7)
grant: unknown key `description` in a conditional grant object — the keys are `role`, `condition`, "import-id"
claim: resources = [...] is required (a claim ships its witnesses)
claim … deviates: reason = "…" is required (a deviation is a disclosed decision, and the report carries the reason)
suppress google_org_policy_policy "x" matches nothing — stale suppression (typo or upstream rename)
suppress … role on google_organization_iam_member "group:x@example.com": the address is in conflict (⊥); suppress the whole member or resolve the conflict first
composition conflicts: <type>.<label>: 2 disagreeing definitions
  - a.satz:12
  - b.satz:40
transpile: `estate.yaml` is the legacy YAML dialect — convert it: `satz import estate.yaml --kind estate`
```

The same address declared twice in one file with the SAME body is idempotent;
across files it is the fold's conflict above.

---

### Actions

| message | cause |
|---|---|
| `action "x": reason = "…" is required` | An action must say why the step is not a resource; it is what the warning quotes. |
| `action "x": run = "…" is required` | Nothing to run. |
| `action "x": phase = "…" — expected "before-apply" or "after-apply"` | Those are the two phases. |
| `action: unexpected entry … (keys are reason, run, args, execute_args, phase)` | An unknown key is refused, never ignored. |
| `action "x": no interpolation allowed` | The name and `run` are literal; only `args` and `execute_args` interpolate. |
| `action "x": declared twice — a.satz:3 and b.satz:9` | Names are unique across the estate. |
| `run = "x.sh" not found. Looked in: …` | Every place that was tried, in order: the declaring file's directory, then the include dirs. |
| `… is not executable. chmod +x …` | Satz never sets the bit — a script that arrived via `get-presets` becomes executable when a human decides it should. |

## 11. Known v0 limits

- **Param scoping is document-ordered, not lexical.** Packs see every earlier
  file's params. True lexical scoping is a future semantics change.
- **No list concatenation.** Overriding a list param replaces it.
- **`use … when` is followed unconditionally when computing which presets an
  estate uses** (`check-presets`), so a conditionally-disabled pack may be
  reported as included. Over-reporting drift is the safe direction.
- **No `force` / priority channel.** "Keep my version of one pack resource" is
  a fork today; `suppress` + redeclare cannot express it because the
  redeclaration lands at the same address and folds to a conflict.
- **An estate with no resources emits no `main.tf`** — only `providers.tf`,
  `variables.tf` and `terraform.tfvars`.
- **`satz apply` does not run actions.** `run-actions` is a separate verb, and
  `phase` only orders and selects — nothing enforces that a `before-apply`
  action ran before the apply. Coupling the two would change what `plan` means,
  which is the same reason `plan` does not transpile.
- **An action produces no evidence.** Nothing is recorded when one runs, and no
  claim can rest on it: satz still cannot tell you whether a given step was ever
  run against an organisation. Where that matters the pattern to copy is
  `--prowler <FILE>` — satz ingests a file, it never trusts a process.
- **`suppress` cannot remove an action.** `--no-pack-actions` drops every
  pack-declared one; there is no per-action subtraction.

---

## 12. Importing what exists

`satz import <source>` writes a Satz estate from something that already
exists. The shape is read off the source; the check is always the same —
`satz transpile`, then `tofu plan` against the real state must show no
destroy for what was already managed. Import ids for the live shape are the
asset path; for the others, `satz adopt` resolves them afterwards.

| shape | when | what you get | limitations |
|---|---|---|---|
| `import state.json` (or `-` for `tofu show -json` on stdin) | you already run Terraform/OpenTofu and want the estate that reproduces its state | folders/projects nested, grants collapsed to member → roles, services into `project_service`, every resource with its `"import-id"` from the state id | only `tofu show -json` output (a raw `.tfstate` is refused); a resource without `id` gets no import id; grant conditions are not carried; the import-config's `exclude`/`map` are not applied to this shape; rows with `import: false` are skipped and listed (`type off`) |
| `import organizations/<n>` \| `folders/<n>` \| `projects/<id>` (or bare `import` with `root:` in `import-config.yaml`) | brownfield: nothing is in Terraform yet | one Cloud Asset sweep of the scope; every enabled type; import ids = the row's `import_id` template rendered from the resource (`{project} {name}` for a log metric), else the asset path with the project named by ID; required attributes the asset lacks derived (`parent`, `org_id`/`folder`/`project`, `location`/`region` from the asset name, a `*_id` from its last segment — `secret_id`, `repository_id` —, a service account's `account_id`); API vocabulary the provider spells differently is renamed per the row's `map:` (a firewall's `allowed[].IPProtocol` → `allow { protocol }`), and self-link `region`/`zone` and full-name `name` values are shortened to what the provider takes | only rows with `import: true` AND an `asset_type` are swept (21 enabled by default — the landing-zone types plus VPC network/subnet/firewall, Pub/Sub topic, Secret Manager secret, log metric, Artifact Registry repository, each verified live to plan as import-only; `--only` narrows, never widens; an enabled row with `asset_type: TODO` is an error); IAM conditions are not carried; no Cloud Identity groups (not in Cloud Asset — state shape or `adopt`); a resource whose required attribute cannot be derived is skipped and named; one fetch error aborts the run; a page is 1000 assets |
| `--into <estate>` | the estate exists; take over what it does not declare yet | packs `imported-<scope>[-<container>].satz` plus `use` lines inserted at the declaring folder/project | live shape only; if ANY declared resource fails to resolve live (error, ambiguity), nothing is written; the packs are regenerated wholesale on every run — hand edits go into the estate, never into an `imported-*` pack; a `use` is inserted automatically only where the container is declared in the estate file itself |
| `import ./hcl/ [--wrap-all]` | hand-written or generated `.tf` (`gcloud beta resource-config bulk-export`, `tofu plan -generate-config-out`) | `variable`/`locals` promoted to params; schema-known, identifier-labelled `resource` blocks whose values are literals, params or `${…}` references as Satz resources placed under the folder/project they reference; everything else verbatim in `hcl trust "imported from <file>:<line>"`; the report says why per block | `terraform`/`provider` blocks are always dropped (the emitter writes them), `--wrap-all` included — and `--wrap-all` promotes nothing, since a param is a translation; services and grants lose their labels (they become list/map entries); a `project_service` with more than `service` is wrapped; a scope written as an expression satz cannot place is wrapped; the estate gets a local backend and google/google-beta providers to edit; no import ids (`adopt`); a `${…}` reference is opaque to the compliance plane |
| `import <file>.yaml --kind estate\|pack [--gate <estate>] [--fork]` | the legacy YAML dialect (until the last org is moved) | `<stem>.satz` beside the source, proven by compiling it (`CONVERTED … N resources emitted`) | refuses while a `use` still points at a YAML pack (each named — convert packs first); a result that does not compile is deleted and reported; `--fork` writes `<stem>.local.satz` (packs only); a pack without `--gate` is only parsed; interior comments are not carried; a `!include` in a `!format` value is refused (inline it) |

### 12.1 From existing Terraform (`./hcl/`)

Three tiers. A `variable` with a literal `default`, and a `locals` entry with a
literal value, become **params** — params *are* Satz's variables, so this is the
language's own model rather than a rewrite, and the result stays
re-parameterisable. A `resource` block of a schema-known type becomes a **Satz
resource** when every value is a literal, a promoted param, or a reference to a
managed resource; the folder/project it references (`parent`, `folder_id`,
`project`) decides where it is placed. Every other block is carried **verbatim**
inside `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
written, but the fold cannot compose it and the compliance plane cannot see into
it. Every block is accounted for; nothing vanishes.

Two things the report is careful about. A promoted declaration that a wrapped
block still reads is **carried verbatim as well**, so the wrapped block's
`var.x` keeps resolving. And *translated* is not *proven*: a `${…}` reference is
opaque to the compliance plane, so a claim cannot reason about it.

| HCL | Becomes |
|---|---|
| `resource` of a schema-known type, identifier label, values that are literals, promoted params or `${…}` references | a Satz resource: attributes as written, repeated nested blocks → a list of objects, `lifecycle` as declared; the label kept for plain resources (services and grants become list/map entries and lose theirs) |
| `variable "x" { default = <literal> }`, `locals { y = <literal> }` | **promoted** to `params { x = … }`; `var.x` becomes a bare param reference, `"a-${var.x}"` the interpolation `"a-{x}"` |
| `variable "x"` with no `default` | named in the header, given no value — `satz transpile` stops with `unknown param 'x'` until it is bound, which is the same gate the source had |
| a reference to a managed resource (`google_project.p.number`), anywhere in a value | carried verbatim as Satz `${{…}}`, which emits back byte-identically |
| a `*_iam_member` whose scope is neither project, folder nor organisation (a service account's, a bucket's) | a **labelled resource** with its scope attribute (`service_account_id = …`, `bucket = …`) — the member map has no room for one |
| `google_folder` (`parent` = the organisation, or a reference to a folder in the input), `google_project` (`folder_id` a folder reference, or `org_id`), `google_project_service` / `google_project_iam_member` / any project-scoped resource whose `project` references a project in the input, `google_folder_iam_member`, `google_organization_iam_member` | placed: nested under the folder/project they reference; grants become grant-map entries (a `condition` block travels in the object form), services the project's `project_service` list; `customer_organization_id` is inferred from the literals |
| a project whose `folder_id` is a folder *number*, a resource whose parent is wrapped, a scope written as any other expression, groups, memberships, `*_iam_binding` / `_policy` / `_audit_config` (authoritative: they own every binding on their target), billing grants (they hoist to the estate's billing scope), a grant without a resolvable `member`/`role`, a `project_service` carrying more than `service` | wrapped, with the reason (closure by dependency: a child of a wrapped container is wrapped too) |
| a `resource` using `count`/`for_each`/`dynamic`/`provider`/`depends_on`, a function call, a conditional (`? :`), an arithmetic or `for` expression, a `data.`/`module.` reference, a name no `variable` or `locals` declares, a string holding a literal `${` (written `$${`), a type not in the schema, a label that is not an identifier | wrapped, with the reason |
| `module`, `data`, `output`, `moved`, `import` | wrapped |
| a `variable` or `locals` whose value is not literal | wrapped, and the names it declares stay Terraform variables |
| `terraform`, `provider` | dropped (one note each), also under `--wrap-all`. A `provider` block's default `project` is carried as **placement**: a resource that named no project of its own lands in that project when the default resolves to one of the imported projects, and the dropped row says so |

### 12.2 From the YAML dialect

satz's first surface language was a YAML dialect with custom tags. Since
v0.46.14 nothing reads it but `satz import <file>.yaml`: `transpile` and every
other command take `.satz` and refuse a `.yaml` estate with a pointer to the
converter. (Brownfield estates never need the dialect: the state and live
shapes above write a Satz estate directly, through the same printer.)

**What changed at the surface:**

| YAML dialect | Satz |
|---|---|
| `org_policy_policy:` (implicit `google_`) | `google_org_policy_policy { … }` — the schema's type name |
| `&anchor` / `*anchor` | params are declarations; a bare identifier references one |
| `!format ["organizations/{}", *customer-organization-id]` | `"organizations/{customer_organization_id}"` |
| identity-`!format` aliasing | `a = b` |
| override an anchor *above* the `!include`, in textual order | bind the param; the compiler sorts by dependency |
| `!include x.yaml` / `!include-if anchor x.yaml` | `use "x.satz"` / `use "x.satz" when param` |
| `!expr google_x.y.z` | `"${{google_x.y.z}}"` — doubled braces |
| `<pack>.claims.yaml` sidecar | `claim … { … }` in the pack |
| `kebab-case` anchors | `snake_case` identifiers (the emitter maps `logsink_project_name` ↔ `logsink-project-name`) |

Side by side, one org policy:

```yaml
iam-managed-disableServiceAccountKeyCreation:
  name: iam.managed.disableServiceAccountKeyCreation
  parent: !format ["organizations/{}", *customer-organization-id]
  spec:
    rules:
      - enforce: "TRUE"
```

```
"iam-managed-disableServiceAccountKeyCreation" {
  name   = "iam.managed.disableServiceAccountKeyCreation"
  parent = "organizations/{customer_organization_id}"
  spec {
    rules = [ { enforce = "TRUE" } ]
  }
}
```

**How a conversion is checked.** `satz import <file>.yaml` converts a file and
compiles the result through the fragment pipeline — the pipeline that will
actually read it — and prints the emitted resource set (`CONVERTED: … N
resources emitted`). An estate is gated on itself; a pack is gated on the
`.satz` estate you pass with `--gate`, or only parsed when there is none. A
result that does not compile is deleted and reported; a conversion that cannot
be checked in context says `NEEDS-REVIEW`; the last word is `satz transpile`
and a `tofu plan` that shows no destroy for what the old estate managed. An
estate that used the dialect's `!import-include` converts to a plain `use`
with a `NEEDS ADOPTION` note: run `satz adopt` afterwards.

**Packs first.** An estate whose `use` still points at `x.yaml` is refused by
the converter and by the compiler alike (`use "x.yaml": packs are Satz —
convert it first: satz import x.yaml --kind pack`).

**Why the dialect still parses at all:** to be migrated. That is the whole of
its support (owner decision, 2026-08-29): YAML is never transpiled or generated
by new functionality, and a YAML code path a cleanup breaks is removed rather
than repaired. A migrated estate may need a manual edit or two.
