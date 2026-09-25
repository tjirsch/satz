# satz language

The complete specification of Satz, the language an estate is written in.

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
| **Satz** | estate + packs, params, composition, suppressions | that the estate is a consistent, conflict-free fold of named, versioned parts — and that every org policy in it is *declared* explicitly | `transpile` |
| **Controls** | claims against a catalog | that each declared control has its witnesses *emitted* — a claim with a missing witness is reported as broken, never as satisfied | `require` |
| **Evidence** | the goal view joined with the live estate | that each witness is live and — for org policies — *enforcing*, by value; a policy switched off in the console is **NOT ENFORCED**, which outranks DRIFTED | `report-compliance` |

In the Satz layer, org policies are ordinary resources in versioned packs. A
semantic upstream change to a pack the estate includes forks the pack and repoints
the estate, checked by transpile identity. Declining a control is a `deviates`
claim with its reason. `hcl { … }` warns on every transpile until it is marked
`hcl trust "<reason>"`.

---

## 2. HCL is the foundation

HCL is the language of the IaC layer: the assembly code that OpenTofu (the
preferred tool) or Terraform executes against provider plugins from Google and
others. Satz compiles to it. It does not wrap it, rename it, or hide it.

**Satz has no provider documentation of its own.** The OpenTofu registry
and the HashiCorp registry are the documentation. Resource type names are the
provider's, to the underscore. Attribute names are the provider's, to the
underscore — across six real resource pairs from fleet estates (bucket, org
policy, IAM grants, conditional grant, log metric + alert policy, folder/project
hierarchy) **not one attribute name differs**. What you know about
`google_storage_bucket` from the registry page is what you write.

### 2.1 One resource, both ways

The audit-log bucket from the shipped `organization-audit-logsink` pack, as
written and as emitted:

```
google_storage_bucket {
  org_audit_logs {
    project                     = "${{google_project.logsink_project.project_id}}"
    name                        = logsink_bucket_name
    location                    = logsink_bucket_location
    storage_class               = "NEARLINE"
    uniform_bucket_level_access = true
    public_access_prevention    = "enforced"
    lifecycle_rule = [
      {
        action { type = "Delete" }
        condition { age = logsink_retention_days }
      },
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
`action.type`, `condition.age` — is the provider's name. Where the text differs:

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
  service; each project gets its own provider alias, scoped to it (§6.8);
- an org policy's structured `parameters { … }` is JSON-encoded into the
  string the provider wants;
- the backend is chosen by `deployment_mode` (§6.8); `"import-id"` becomes an
  `import` block in `imports.tf` (§6.7); param names are kebab-cased in
  `variables.tf` / `terraform.tfvars`.

Everything else is the provider's, unchanged.

---

## 3. What Satz adds on top

The layers diagram reads bottom-up; this section reads the same way. Each
addition is one line or a few, shown against the shipped CIS pack.

**Include a pack** — the CIS baseline, one line:

```
use "presets/cis/CIS-GCP-Foundation-4.0.satz" when use_cis_baseline
```

**Tune it without forking** — the pack declares a default, the estate binds it.
An estate whose contact addresses are on a different domain than its organisation
binds:

```
params {
  essential_contacts_allowed_domains = ["@example.net"]
}
```

**Decline one control it provides** — no fork, one line; if upstream removes the
policy, the `suppress` matches nothing and the transpile fails:

```
suppress google_org_policy_policy "compute-managed-requireOsLogin"
```

**Record the reason** — the control then reads ⚠ *deviation* in every report,
not ✓ or ✗:

```
claim "cis-gcp" "4.0" "4.4" deviates {
  resources     = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  reason        = "A service here depends on metadata SSH keys; enforcing OS Login breaks it."
  duty_reassess = "Re-assess when that service supports OS Login."
}
```

**Include a bare list as a map's content**, or **conditionally**
(`tests/smoke/yaml/showcase.satz`):

```
use "showcase-policies.satz" as google_org_policy_policy
use "showcase-optional.satz" when want_optional
```

---

## 4. What makes the layers possible

Each layer exists because a handful of language features make it expressible.

**Params with interpolation** are why a pack can be pristine and still fit a
customer. Outer beats inner; everything globally unique derives from
`customer_shortname`, everything org-scoped from `customer_organization_id`.

**Schema-typed resources** are what the fold relies on. Block keys are matched
exactly against the loaded provider schemas, so the compiler tells a resource
map from a nested attribute block without guessing, and an unknown type — or an
argument a type does not have — is a parse-time error, not a plan-time one.

**`use` and the ⊕ fold** are why an estate is a composition rather than a copy.
Two files defining the *same address differently* is a conflict naming both
locations — composition is something the compiler checks.

**`suppress`** is why customisation stays small: subtractive, one line, and a
hard error when it matches nothing.

**`claim` / `deviates`** are why the controls layer is language, not a sidecar:
read from the same compile, naming emitted addresses.

**`hcl trust`** is why raw HCL is visible: it warns on every transpile until the
block carries a reason.

**Provenance by suffix** is what lets a fleet take pack updates: the filename says
who owns the file, the tooling enforces it, and the diff file is regenerated on
every merge.

---

## 5. How much shorter, measured

Three numbers — the first two measured across the fleet — counting code lines (blank and comment-only lines
removed on both sides):

| what is compared | median | range | typical estate |
|---|---|---|---|
| **the estate file a customer writes** vs the HCL it becomes | **6.3×** | 2.9× – 10.6× | 174 → 1 100 lines, 127 resources |
| all Satz sources incl. packs vs HCL | 1.8× | 1.4× – 3.7× | 618 → 1 100 |
| the CIS pack alone vs its 25 policies' HCL | 0.48× | — | 523 → 252 |

The first row is what is maintained per organisation: a sixth of what runs. The high end (~10×) are the
estates that `use` the most pristine packs; the low end (~3×) are estates with forked or
inlined packs.

The second row explains the first. Packs are *not* short — they carry their
params' reasoning as comments and their control claims and questions in-file,
none of which emits HCL — and the third row is the extreme case: the CIS pack is
about twice as long as the policies it produces, because 164 of its code lines
are claims and 60 are questions.

**What changes** for common tasks:

| to… | HCL | Satz |
|---|---|---|
| decline one control a pack provides | fork the pack | one `suppress` line in the estate |
| override one preset value | copy-edit, or define the anchor above the include in order | bind the param, any order |
| say why a control is not met | nowhere | `claim … deviates { reason = "…" }` |
| know whether a pack changed upstream | diff by hand | `check-presets` — canonical form of the parsed pack |
| keep the compliance story with the code | a sidecar file | claims in the pack, same compile |

---

## 6. Language reference

Two complete estates compile in CI on every pull request and every push to `main`
and are the examples the
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
  own scope), `hcl trust`, questions including a required choice, one that is
  not and one whose empty answer means something, and claims of all three kinds with
  duties and an `interpretation`. `scripts/smoke.sh` transpiles it,
  validates the HCL and checks each feature's effect.

Every snippet in this section is either one of those files or compiles the
same way (`terraform { backend { … } }` is required by the emitter — the
snippets omit it for brevity where they are fragments of a larger estate).

Where HCL is shown, it is the emitted text.

### 6.1 Lexical structure

**Line endings** — a CRLF line ending is a line ending: a file checked out with CRLF
compiles, formats and compares exactly as its LF twin, triple-quoted strings and
`hcl { }` bodies included. Every file satz writes has LF line endings.

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

**`{x}` interpolates a param; `{{` is a literal brace.**

**A reference must name something the estate emits.** Every `${{…}}` is checked
against what was actually emitted — including a project's expanded services and
an exploded grant, which are emitted addresses that were never written as
blocks. A reference naming nothing is a compile error that says where it was
written and lists the labels of that type that do exist:

```
references to resources this estate does not emit (1)

error    written-reference  satz/main.satz:26
    google_service_account_iam_member.onboarding_user writes
    `${google_service_account.onbaording.name}`
      emitted `google_service_account` labels: onboarding

1 error
```

Without the check, Terraform catches most typos one cycle later, pointing at
generated HCL instead of the Satz line, and cannot catch a typo that names a
different real resource. Suppressing a resource that something else references
is the same error: the estate does not emit it.

**A whole-value reference is recorded as a reference.** When a value is *nothing but*
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
header  := ("estate" | "pack") IDENT [ "version" STRING ]
item    := "params" "{" { param } "}"
         | "use" STRING [ "as" IDENT ] [ "when" IDENT ]
         | "claim" STRING STRING STRING COVERAGE "{" { claim-entry } "}"
         | "question" [ "oneof" ] IDENT "{" { question-entry } "}"
         | "suppress" IDENT STRING [ "role" STRING ]
         | "hcl" [ "trust" STRING ] "{" … "}"
         | "action" STRING "{" { action-entry } "}"
         | "offers" STRING "{" { offers-entry } "}"      # the map only
         | "export" STRING "=" value [ "description" STRING ]
         | "interface" STRING "{" { "export" STRING "=" value [ "description" STRING ] } "}"
         | block

block   := KEY [ KEY ] "{" { entry } "}"
entry   := KEY "=" value | block | "use" STRING [ "as" IDENT ] [ "when" IDENT ]
```

An `item` stands at the top level of a file and nowhere else. A `use` is the one
statement that is also an `entry`: it stands in `google_folder { … }` and in a resource
type map (§6.9).

**Header**

```
estate acme                        # a customer estate
pack   monitoring.audit_logsink version "1.1"
```

- `estate` — one customer organisation. Usually one per repo.
- `pack` — a reusable unit. `version` is the pack's own revision, **in-file,
  never in the filename** (framework versions live in claims and are independent
  of it: several pack revisions may implement the same standard).

The header is a name and a version: another word on its line is an error.

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
  log_bucket_name          = audit_bucket_name # bare identifier = param reference
}
```

Resolution rules:

1. A value bound by the *using* document wins over the file's own default
   (outer beats inner — that is what makes a pack configurable).
2. Params may reference each other **regardless of declaration order**; the
   compiler sorts by dependency.
3. The namespace is one document-ordered space: packs see every earlier file's
   params; there is no per-pack scope.
   A pack's **default may therefore reference another pack's param**, and that is
   the right way to wire two packs that share a value — the CIS alert pack
   defaults its project to the audit-logsink pack's `logsink_project_id`, so an
   estate using both sets nothing, and renaming the logsink project moves the
   alerts with it; a repeated literal would keep pointing at the old name. Used
   without the pack that declares the name, the reference stops with
   `unknown param`, and the estate has to bind the value.
4. Overriding a **list replaces it** — Satz has no list concatenation — so an estate
   that adds to a pack's list repeats the entries it keeps. A PACK adds to one instead
   of replacing it, through `contributes_<param>`, below.

#### `contributes_<param>` — a pack's entries in another file's list

A param named `contributes_<param>` is not a param: it is a **contribution**, the entries
a pack adds to the list param `<param>` that another file declares.

```
pack billing_export version "1.0"

params {
  contributes_allowed_policy_member_subjects = [
    "serviceAccount:billing-export-bigquery@system.gserviceaccount.com",
  ]
}
```

What it does:

- The entries are added after whatever `<param>` holds — the estate's binding, else the
  default of the file that declares it. The list stays the estate's; a contribution only
  adds.
- A pack behind a false `use … when` gate contributes nothing, like everything else it
  declares. Switching the pack off takes its entries out.
- An entry the list already holds is not added twice, whoever wrote it.
- The merge happens **before** the compile walks the estate, so where the contributing
  `use` line stands decides nothing.
- It is no value of its own: it is never a `variable`, never in `terraform.tfvars`, and
  no `question` may ask it — what a customer answers is `<param>`.
- Where no file the estate uses declares `<param>`, the entries are dropped. The pack
  graph carries the requirement as a `data` edge, and `satz packs` names it.

What a pack writes it as is checked where it is written: it belongs in a pack, its value
is a list, and a file that declares `<param>` itself writes the entries into that default
instead. A `<param>` that is not a list is refused, naming the contributing file.

`satz packs <estate>` prints each pack's contributions, so every entry in a list has a
pack beside it; a pack's own page under `presets/docs/` lists them under **Contributes**.

What the compiler does with them: every param becomes a typed `variable` in
`variables.tf` (underscores → hyphens) with its resolved value in
`terraform.tfvars`, for anyone reading the HCL — but a resource that references a
param is emitted with the **literal**, never `var.x`. The type follows the value: a
string, a number or a bool is its own type; a list or a map of scalars is
`list(string)` or `map(string)`; a list or a map holding lists or objects is `any`,
because its elements need not share one shape — the CIS baseline's
`cis_sa_key_creation_rules` is a list of rules, and an exemption rule carries a
`condition` the enforcing rule does not.

```hcl
# variables.tf
variable "audit-bucket-name" { type = string }
variable "retention-days"    { type = number }
variable "extra-members"     { type = list(string) }
variable "sa-key-rules"      { type = any }

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
params {
  // essentialcontacts.managed.allowedContactDomains: the DOMAINS whose
  // addresses may be set as essential contacts (each entry `@domain`).
  // Default = the customer's own domain. Some customers keep contacts on a
  // different domain, or on several; the param is a list, so neither case
  // forks the pack.
  essential_contacts_allowed_domains = [
    "@{customer_domain}",
  ]
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

**The keys inside a resource body are the schema's too**, level by level: an
argument or block the provider does not have is a parse error naming the file,
the line and the key. A block's own body is checked the same way
(`spec { … }` of an org policy, `condition { … }` under its rules); what an
attribute carries is a value, so the keys of a `labels` map are the estate's
own. Nine keys are satz's rather than the provider's and are written in any body
they belong to:

| key | what it is |
| --- | --- |
| `"import-id"` | the live id of the resource this block adopts — it becomes an `import` block (§6.7) |
| `lifecycle` | Terraform's meta-argument, emitted as written |
| `provider` | Terraform's meta-argument, emitted as a reference (`google.google`) |
| `project_service` | on `google_project`: the APIs to enable, one `google_project_service` each |
| `org` | on `google_project`: the organisation the project states as its parent |
| `member`, `manager`, `owner` | on `google_cloud_identity_group`: the memberships to create (§6.4) |
| `email` | on `google_cloud_identity_group`: the group address, where the label is not it |

The only bare block keywords are Satz's own: `estate`, `pack`, `params`,
`terraform`, `providers`, `use`, `suppress`, `claim`, `question`, `action`, `notice`, `offers`,
`hcl`. `terraform` and `providers` are blocks and `use` also stands inside a block (§6.9);
the rest are statements, written at the top level of a file, and an error as the key of a
block anywhere else.

**Simple** — one org policy:

```
google_org_policy_policy {
  "compute-managed-requireOsLogin" {
    name   = "compute.managed.requireOsLogin"
    parent = "organizations/{customer_organization_id}"
    spec {
      rules = [{ enforce = "TRUE" }]
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

A policy on a custom constraint names it the same way — `name =
"custom.cisCloudSqlDeletionProtection"` — and when the estate declares that
constraint (`google_org_policy_custom_constraint` with the same `name`), the
emitted policy carries `depends_on` on it, so the apply creates the constraint
first: the API refuses a policy on a constraint that does not exist yet.

**The policies on one parent apply one at a time.** The Org Policy API refuses a write
to a parent's policies while another is in flight (`409 CONCURRENT_POLICY_CHANGES`), and
`tofu` writes ten resources at once. So every `google_org_policy_policy` carries
`depends_on` on the policy before it on the same `parent`, in address order — whichever
pack or file declared it. Policies on different parents, and every other resource, still
apply in parallel; `depends_on` changes no plan.

**Nested blocks** — a bucket with a nested block, a single block, and a
*repeated* block:

```
google_storage_bucket {
  audit_logs {
    name                        = audit_bucket_name
    location                    = "EU"
    uniform_bucket_level_access = true
    versioning { enabled = true }
    retention_policy { retention_period = 34560000 }
    lifecycle_rule = [
      {
        action { type = "Delete" }
        condition { age = 730 }
      },
      {
        action { type = "SetStorageClass" storage_class = "COLDLINE" }
        condition { age = 90 }
      },
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
twice in one body is an error naming both lines. The list form is the only
one. Resource-type maps (`google_…`) may repeat — two
`google_org_policy_policy { … }` groups in one file are one map, folded by address.

`google_cloud_identity_group` is a Satz abstraction that shares a Terraform
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

**A member that names a service account the estate declares waits for it.** When the
member is `serviceAccount:<account_id>@<project>.iam.gserviceaccount.com` of a
`google_service_account` in the same estate, the grant carries
`depends_on = [google_service_account.<label>]`, so the apply creates the account before
the grant and destroys the grant before the account. Without it `tofu` runs both at
once: the API refuses a member that does not exist yet, and an account deleted first
leaves `deleted:serviceAccount:…` bindings behind. A group membership whose member key
names such an account waits for it the same way.

**A member that names a group the estate declares waits for it the same way.** When
the member is `group:<key>@<customer_domain>` of a `google_cloud_identity_group` in the
same estate, the grant carries `depends_on = [google_cloud_identity_group.<label>]`, and
so does a membership whose member key names that group. Without it a fresh organisation
fails on apply: the grants run beside the group creations, the IAM API refuses a member
that does not exist yet, and the first refusal stops the groups still queued from being
created at all.

**A resource waits for the service that enables the API it needs.** Every resource
type satz can emit is served by an API (`google_billing_budget` by
`billingbudgets.googleapis.com`, `google_monitoring_alert_policy` by
`monitoring.googleapis.com`), and the emitted resource carries `depends_on` on each
`google_project_service` in the estate that enables one — bounded to the two projects
that can matter: the project the resource lives in, and the infra project the default
provider bills its calls to. `google_project_service` itself is never ordered behind
another service, and no edge is added into a service block's own dependency closure,
so the project a service is declared on and the folder above it never wait for it.
Without the ordering `tofu` runs both at once and the apply dies on an API that was
about to be switched on. A resource whose API no `google_project_service` enables
gets no edge — the compile reports that instead, because a dependency on nothing is
not a thing.

**The API has to be on the project the call is BILLED to.** The estate's `google`
provider carries `user_project_override = true` with `billing_project =
infra_project_name`, so Google requires the service enabled on the infra project
whatever the resource's own scope is — a budget hangs off the billing account and an
org policy off the organisation, and both still need their API there. A resource
written inside a `google_project { … }` is served by that project's alias, which is
billed to the project itself, so its API has to be on that project. The compile
checks the first half: the APIs the estate's emitted types need, against the
`project_service` list of the infra project.

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
grant, `project` on a map inside a project) is refused.
`google_billing_account_iam_member` is the exception: its `billing_account_id` is estate-wide, pinned once and defaulting to
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

Nesting **is** the parent reference — `parent` and `folder_id` follow from the
block a resource stands in:

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
`google_folder { … }` is a schema type, so it emits a resource;
`labels { … }` is an attribute, so it emits `labels = { … }`. Same syntax; the
schema decides.

A project may **say** its parent instead of standing in it
(`tests/smoke/yaml/showcase.satz`, "a project whose parent is SAID"):

```
params {
  archive_project_folder = "google_folder.infra.name"
}

google_project {
  archive {
    name       = "corp-archive"
    project_id = "{customer_shortname}-archive-001"
    folder_id  = archive_project_folder
  }
}
```

```hcl
resource "google_project" "archive" {
  project_id = "corp-archive-001"
  name = "corp-archive"
  provider = google.google
  billing_account = "012345-6789AB-CDEF01"
  folder_id = google_folder.infra.name
}
```

`folder_id` takes a reference to a folder the estate declares
(`google_folder.<label>.name`) or the id of one that already exists
(`"123456789012"`), and `org_id` takes the organisation's id. A value that is
empty says nothing: the node the block stands in decides, which at the top level
is the organisation. That is how a pack carries its own parent in a param — the
default says nothing, and an estate that names a folder gets the same project
wherever the pack's `use` line stands.

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
        "import-id" = "acme-infra-001"
        project_id  = "acme-infra-001"
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
templates in `presets/import-config.yaml`, IAM grants only when the live IAM
policy of what they are granted on holds them — and `--execute` writes them back: an
`"import-id"` line into a block, the object form into a list entry. A grant the
live policy does not hold is created by `apply` and gets no id. Derived
ids are written too; `tofu plan` verifies each through its import block. It
does not rewrite an entry it cannot find in the source (an interpolated member),
and does not edit a **pristine pack** — packs are upstream-owned, so their
resources come back as hints (`--execute --import`, or fork the pack). A
resolution with more than one live candidate is reported as ambiguous and left
for you to pin.

### 6.8 Estate configuration blocks

`terraform` and `providers` are configuration, not resources. **`terraform` is
required** — an estate without it does not compile (`Missing 'terraform' block`).
A backend may list both `local` and `gcs`; the emitter writes the ONE that
`deployment_mode` selects (`"local"` / `"cloud"`, see the `migrate` command;
an estate with no `deployment_mode` is `"local"`), never both. Any other value is
an error at the line that binds it, and the compile refuses. So is `"cloud"` without a
value for `svc_iac_account` or `infra_project_name`: cloud mode runs as the account the
two name, and the error names the one missing. The mode also decides
which identity the estate's live commands run as, so `whoami`, `migrate` and every
command that runs as the estate refuse the same value with the same reason:

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

**Every `google_project` gets a provider alias.** Beside the blocks the estate
writes, the emitter adds one `provider "google"` per project entity, aliased
`project_<label>`, and every resource written inside that `google_project { … }`
carries `provider = google.project_<label>`. It is the estate's `google`
provider scoped to that project: the same region the estate's `google` block
names — an estate whose block names none gets an alias that names none, and a
regional resource in it writes its own — the same
`impersonate_service_account` in cloud mode, and the project itself as
`project` and as the quota project (`user_project_override = true` with
`billing_project = <that project>`), so the APIs those resources need are the
ones enabled on the project they live in.

### 6.9 `use` — composition

```
use "presets/cis/CIS-GCP-Foundation-4.0.satz" # a pack that carries its own types
google_essential_contacts_contact { use "presets/essential-contacts-organization.satz" } # inside a map
use "presets/essential-contacts-organization.satz" as google_essential_contacts_contact # as: same thing
use "showcase-optional.satz" when want_optional # conditionally
```

A `use` stands at the top level of a file, in `google_folder { … }` or in a resource
type map. The position decides what the used file's entries are read as and what scopes
the resources it declares:

| where the `use` stands | what the used file's entries are | what scopes them |
|---|---|---|
| the top level of a file | resource type maps, `use` lines | the organisation |
| `google_folder { … }` | named folders, `use` lines | the folder the map stands in |
| a resource type map, `google_x { … }` | labelled `google_x` bodies — members, in a grant map — and `use` lines | whatever scopes the map |

`use "…" as google_x` is the third row written flat: the file's entries are the content
of a `google_x` map standing where the `use` stands. It is valid at the top level and in
`google_folder { … }`; inside a resource type map the map's type is the key, and an `as`
naming another type is an error. `google_project { … }` takes projects and no `use`.

**The body of a folder and the body of a project hold the estate's own resources, and
take no `use`.** A pack is used at the top level, where it declares the same resources
whatever else the estate holds; a pack that creates a project names the folder that
project is created in with a param of its own
(`logsink_project_folder`, `mdc_mgmt_project_folder`). A `use` in a node's body is
refused, naming the node and the param to bind:

```
use "presets/monitoring/organization-audit-logsink.satz"` stands in the body of `google_folder.infra_folder`, which holds the estate's own resources — a pack is used at the top level of a file. Move the line to the top level. A pack that creates a project names the folder it is created in with a param of its own — `logsink_project_folder` in `presets/monitoring/organization-audit-logsink.satz`, `mdc_mgmt_project_folder` in `presets/integrations/microsoft-defender-for-cloud.satz` — so bind that param to `google_folder.infra_folder.name` in the estate's `params { … }`. Every other pack emits the same resources wherever its line stands
```

A file declares no kind. It is judged by whether its entries fit the position of its
`use`: a pack that declares its own resource types — the CIS baseline and its
extensions, most of the library — is `use`d at the top level, and a file that is a bare
list of labels, like the contacts pack above or `showcase-policies.satz`, is `use`d
inside the map of its type. An entry that does not fit is an error at the `use` line,
naming the entry's line in the used file:

```
use "presets/cis/cmek.satz" inside `google_folder { … }`: presets/cis/cmek.satz:59 does not belong there — `google_org_policy_policy { … }` opens a map of its own, and directly inside `google_folder { … }` every key is a name — it is read as a folder named `google_org_policy_policy`. A file used inside `google_folder { … }` holds named folders. This one declares its own resource types, so it is written bare, at the top level: `use "presets/cis/cmek.satz"`
```

Each pack states its own line in its header comment, and `presets/docs/` prints it.

**The statements of a used file reach the estate from every position.** Its `params`
join the estate's parameter namespace, where the estate's own binding wins; its
`question`s go to the interview; its `claim`s to the compliance plane; its `notice`s
and `action`s join the estate's; its `hcl` blocks pass through to `main.tf` beside the
resources. None of them is emitted as part of the map the `use` stands in — a bare
list with a `params` block and a `question` is the content of a resource type map, and
the map receives its labelled bodies alone. A file that holds statements and no entry
(`presets/estate-core.satz`, `presets/estate-map.satz`) is `use`d at the top level;
inside `google_folder { … }` or a resource type map it is refused, because it brings
nothing that position takes. `suppress` is read from the estate's own file: a used
file that carries one is refused.

**An organisation-level resource type is written at the top level of a file, and reaches
the organisation from wherever that file is `use`d.** A folder, a project, an
organisation grant, a Cloud Identity group and a billing grant each hang off something
above the project — the organisation, a folder, the Cloud Identity customer, the billing
account — so the position of the `use` does not place them. That is what lets ONE file
declare a project together with the groups that go with it:

```
// team-platform.satz
google_project {
  acme_platform_001 {
    project_id      = "acme-platform-001"
    billing_account = billing_account_infra
  }
}

google_cloud_identity_group {
  "platform-admins" { display_name = "Platform Admins" }
}

google_organization_iam_member {
  "group:platform-admins@example.com" = ["roles/monitoring.viewer"]
}
```

`use "team-platform.satz"` creates the project under the organisation, and the group and
the grant reach the organisation and the Cloud Identity customer. To create the project
in a folder, the file gives it a `folder_id` — a param of the pack, bound by the estate
to `google_folder.<label>.name`. A resource whose type takes a `project` lands in the
project it stands in — an org policy written inside a project's body gets
`parent = "projects/<id>"`.

**The same type written INSIDE a project's body is refused.** A project is the bottom of
the resource hierarchy, so nothing above it is placed by standing in its body; written
there it would reach the organisation anyway, which reads as "in this project" and is
not:

```
`google_organization_iam_member { … }` stands in the body of a `google_project`, and it belongs to the organisation — not to the project. It is written at the top level of a file, the file that declares the project included: one file declares a project together with the organisation-level resources that go with it, and those reach the organisation from wherever that file is `use`d
```

A folder's body takes them, because a folder has children: a group or an organisation
grant written in one reaches the organisation exactly once, however many folders declare
it.

**A statement is written at the top level of a file.** Directly inside a resource type
map, `google_folder { … }`, `google_project { … }`, or the body of a folder or a
project, a block whose key is a statement keyword is an error (a `use` is the one
statement that is also an entry, and stands where §6.9 says):

```
`params` is a Satz statement: it is written at the top level of a file, where it goes to the estate's parameter namespace. Directly inside `google_essential_contacts_contact { … }` it is read as a resource `google_essential_contacts_contact.params`. Move it to the top level of the file; one that really is called `params` is written quoted, `"params" { … }`
```

The same holds for a resource type map written directly inside a map of names
(`google_x { google_y { … } }`, `google_folder { google_x { … } }`): there every key is
a label or a folder name, so the map is refused. Nested blocks of a resource's own
body are the provider's — `action { type = "Delete" }` inside a `lifecycle_rule` is an
attribute block, not the `action` statement.

- **Path** is a plain string, never interpolated. Resolved relative to the using
  file first, then the configured `include_dirs`.
- **`when <param>`** — the file is pulled in only if the param is truthy. A
  skipped file contributes nothing: no resources, no params, no claims, no questions.

A `use` cycle is an error naming the chain; `when` on a param no file declares is an
error, not `false`; an attribute at the top level of a used file is an error.

**In the showcase:** `showcase-pack.satz` declares
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

When upstream renames or removes a resource, every estate that suppresses it
fails on the next transpile, so the stale line is removed rather than kept.

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
  emitted is a **broken claim** (‼), never reported as satisfied.
- An org policy's verdict is its UNCONDITIONAL rule. A conditional rule is an
  exemption — "enforced everywhere except where this tag is bound" — and is reported
  beside the verdict rather than replacing it. A policy with several unconditional
  rules, or a list constraint, yields no verdict at all.
- A claim asserts what its witnesses DO. An `implements` claim naming an org
  policy that carries `enforce = "FALSE"` or `spec { reset = true }` is a
  **contradicted claim** (‼): the witness exists and discharges nothing. A
  `deviates` claim naming a policy that enforces is contradicted the same way —
  it discloses a non-conformance the estate does not have. `contributes` asserts
  nothing about the value, and a policy whose effect has no single answer — a
  list constraint, several rules — is never contradicted.
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
  resources         = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  interpretation    = "OS Login is required, so VM SSH access is governed by IAM rather than metadata keys."
  duty_existing_vms = "Enforcing OS Login can cut existing SSH access patterns; verify before enabling on an org with running VMs."
}
```

Until that duty is attested (§8), the control reads ◐ *partial (open duty)* —
witnesses present, human step outstanding.

**`deviates` — declining a control**

```
claim "cis-gcp" "4.0" "4.4" deviates {
  resources     = ["google_org_policy_policy.compute_managed_requireOsLogin"]
  reason        = "A service here depends on metadata SSH keys; enforcing OS Login breaks it."
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
declare must still be emitted, so deleting the policy outright reports a broken
claim, not a deviation. A deviation renders as ⚠ with its reason, is counted
separately, and does **not** fail the `require` gate. It outranks the claims it contradicts
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

The body is captured verbatim, is **never interpolated**, and bypasses the fold,
so the compliance plane cannot see into it. Every transpile says so:

```
warning  hcl-passthrough  satz/main.satz:14
    raw HCL passthrough (4 lines) emitted verbatim — opaque to the compliance plane; no claim
    can cover it. Add `hcl trust "<reason>" { … }` once reviewed.

info     hcl-passthrough  satz/main.satz:21
    raw HCL passthrough (4 lines) — trusted: reviewed 2026-08-24, provider gap for static IPs

1 warning, 1 info
```

and so does the output:

```hcl
# --- raw HCL passthrough from satz/main.satz:21 ---
# Opaque to the compliance plane: no claim covers what is written here.
# trusted: reviewed 2026-08-24, provider gap for static IPs
resource "google_compute_address" "legacy_trusted" {
  name   = "legacy-ip-2"
  region = "europe-west3"
}
```

The compiler enforces the opacity: the compliance plane reads the *emission manifest*
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
references, which nothing else here can. It also has costs:
`tofu plan` cannot show what the script will do, a failure taints state and has
to be cleaned up by hand, the script runs on whichever machine runs `apply`, and
the block is invisible to the compliance plane like any other passthrough.
Use it only when a step has to happen *between* two resources in one apply;
otherwise declare an `action`, which `run-actions` prints, selects by name and
runs in the operator's terminal.

### 6.13 `action` — a step with no provider resource

Some cloud steps cannot be declared at all. Security Command Center service
enablement is one: provider 7.14.1 has 35
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

The library ships this action as a pack:
`use "presets/scc/scc-service-enablement.satz"` binds it. The declaration above
is that pack's, with its `reason` shortened.

| key | required | meaning |
|---|---|---|
| `reason` | yes | Why this is not a resource. Quoted back in every warning. |
| `run` | yes | The script. Resolved relative to the directory of the file that **declares** it — so a pack that ships a script is self-contained — then against the include dirs, exactly as a `use` path is. Never interpolated. Its extension decides how it is launched: a `.py` runs through `uv run --script`, anything else is spawned as a program. |
| `args` | no | Always passed. `{param}` interpolates; an unknown param is a hard error. |
| `execute_args` | no | Appended **only** under `--execute`. This is where a script's `--apply` lives. |
| `phase` | no | `before-apply` for a prerequisite, `after-apply` for a step that needs what the apply created. Default `after-apply`. It orders the run and selects with `--phase`; `satz apply` does not run actions — see below. |

#### Which phase, and what `phase` does not do

The two values answer one question: does this step have to happen **before** the
estate exists, or **after**?

**`before-apply` — a prerequisite.** The step has to be true before the apply can
succeed, so it cannot wait for it. Enablement is one: the SCC pack above
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
`satz apply` does not run actions and does not check that a `before-apply`
action ran. The value orders the run (`before-apply` first, then declaration
order) and selects with `--phase before-apply|after-apply`; the operator runs
each phase around the apply:

```bash
satz run-actions estate.satz --phase before-apply --execute
satz apply
satz run-actions estate.satz --phase after-apply --execute
```

**Nothing here runs while compiling.** The output of `transpile` depends only on
its sources, so the corpus snapshots, `check-presets` and the auto-fork
transpile-identity check compare like with like, and compiling a cloned estate
runs no script. An action is inert
until [`satz run-actions`](../README.md#run-actions-run-actions) is invoked.

**An action emits nothing and can carry no claim.** It is not in `main.tf`, not
in the emission manifest, and no `claim` can name it — the same opacity `hcl`
has, with execution on top. satz records that the step exists and never says
what it did; nothing about an action reaches `report-compliance`.

Every compile of an estate that declares one says so, and unlike `hcl trust`, a
`reason` does not downgrade the warning, because an action executes a script:

```
warning  action  presets/scc/enable.satz:12  scc-services
    `satz run-actions` will execute enable.sh — a pack declares it
      reason: SCC service enablement has no provider resource (google 7.14.1)

info     action
    --no-pack-actions ignores pack-declared actions, --no-actions disables all execution, `satz
    silence add action --reason "…"` leaves these findings out of the output.

1 warning, 1 info
```

A pack may declare an action, and the warning says when one did. The two
switches exist because `get-presets` downloads packs from a public repository. A
downloaded script arrives without its executable bit, and satz does not set it;
the error names the `chmod +x` to run once the script has been read. A `.py`
action needs no bit, because uv reads the file.

Names are unique across the estate. Two actions answering to one name is a hard
error naming both files, the rule the ⊕ fold already applies to a repeated
address. An action inside a pack that `use … when` switched off is never
collected at all.

#### When an action runs, and how

**Only `satz run-actions` executes one** — not `transpile`, not `plan`, not
`apply`. `phase` orders the run and selects with `--phase`; it does not make
anything happen around an apply.

When `run-actions` does run, in this order:

1. **The estate is compiled first.** If it does not compile, nothing runs: the
   command lines are built from its params.
2. **`{param}` resolves** against the finished namespace. An unknown param is a
   hard error, never an empty argument.
3. **The executable is located** relative to the directory of the file that
   declared it, then against the include dirs — the same search a `use` path
   gets.
4. **Every action is located and checked before any one is spawned** — it exists,
   and it can run: a script satz spawns directly is executable, and `uv` is on PATH
   for a `.py` one. A missing `+x` on the fourth script stops the run before the
   first three change the organisation.
5. **Each is spawned**, in phase order (`before-apply`, then `after-apply`) and
   declaration order within a phase — the estate's own actions first, then
   `use`-visit order.
6. **A non-zero exit stops the run** and satz exits with that code. The
   remaining actions do not run.

#### Writing a script for an action

**Write an action in Python**, unless a shell script is genuinely simpler for the
job. satz ships for Windows as well, and a `.sh` action is refused there before the
spawn — the message names the `bash …` to run it by hand from Git Bash or WSL. A
`.py` action runs on every platform, from one file.

What satz provides to a script:

| | |
|---|---|
| **interpreter** | The extension decides. A `.py` file is spawned as `uv run --script <file> <args>`, so it runs everywhere satz does and declares its own dependencies in PEP 723 inline metadata, which uv resolves. Anything else is spawned as a program and runs under whatever its shebang says — `sh`, `bash`, or a compiled binary. |
| **`uv`** | Required for a `.py` action, and looked for on PATH with every other check before the first action is spawned. Without it the run is refused naming the script; satz does not fall back to a `python` or `python3` on PATH, which is a different interpreter with different packages. |
| **executable bit** | Required for a script satz spawns directly; satz does not set it, and the error names the `chmod +x`. A `.py` action needs none — uv reads the file — so a Python action a pack shipped runs as `get-presets` downloaded it. |
| **working directory** | Always the directory holding `config.toml`, whatever directory the operator invoked satz from. Never assume the caller's cwd. |
| **arguments** | `args`, plus `execute_args` appended under `--execute`. |
| **environment** | Exactly five variables: `SATZ_ACTION` (the name), `SATZ_PHASE`, `SATZ_MODE` (`check` or `execute`), `SATZ_ESTATE` (the estate file), `SATZ_HCL_DIR`. Params are **not** exported — anything a script needs must be named in `args`, so the declaration is the complete record of what the action was told. |
| **exit code** | `0` is success. Anything else stops the run and becomes satz's exit code. |
| **stdout / stderr** | Inherited, so the script's output is the operator's output. satz does not capture, parse or store it. |

Put the form that **reads** in `args` and the flag that **writes** in
`execute_args`: `run-actions --check` then runs the script's own dry run, and only
`--execute` lets it write. Whether the `args` form has side effects is up to the
script; satz cannot see what a script does.

A script written to that contract. `tests/smoke/scripts/showcase-action.py` is
this shape with a few extra echoes the smoke matrix asserts on, and CI runs it on
every pull request and every push to `main`:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
import argparse
import os
import sys

# Dry run unless the estate's execute_args said otherwise. The flag is the
# script's own, not satz's: satz only decides whether to pass it.
p = argparse.ArgumentParser()
p.add_argument("--organization", required=True)
p.add_argument("--apply", action="store_true")
a = p.parse_args()

print(f"action {os.environ['SATZ_ACTION']} ({os.environ['SATZ_MODE']}) on organizations/{a.organization}")
if not a.apply:
    print("DRY RUN — re-run with --execute to write.")
    sys.exit(0)

# … the work. A non-zero exit here stops the whole run.
```

`presets/scc/scc-enable-all.sh` has the same shape — dry run by default,
`--apply` to write, non-zero on any failed call. It lives under `presets/`
rather than `scripts/` because `get-presets` downloads only `presets/**`, and the
action must find its script in the estate's copy of the presets. See
[`docs/housekeeping.md`](housekeeping.md#presetssccscc-enable-allsh--security-command-center-services).

### 6.14 `question` — what to ask, and what the answer costs

A pack declares its params, its claims — and what a human must be asked before
those params can be filled. Metadata: a question emits nothing, never enters the
fold, and never reaches the emission manifest.

```
params {
  customer_shortname         = ""
  default_region             = "europe-west3"
  deploys_regional_resources = true
}

question customer_shortname {
  prompt   = "Short name identifying this customer"
  why      = "Project ids, bucket names and group prefixes derive from it, and those ids are globally unique."
  reversal = recreate
  blast    = none
}

question default_region {
  prompt    = "Which region should regional resources default to?"
  reversal  = state_surgery
  blast     = low
  recommend = "europe-west3"
  ask_when  = deploys_regional_resources // optional: only ask when that param is true
}
```

**Two costs.**
`reversal` is what changing the answer does to the *estate* — `edit`,
`state_surgery` or `recreate`. `blast` is what it does to the *running
organisation* — `none`, `low` or `high`. They are independent: enforcing OS Login
is one boolean to reverse and cuts every existing SSH path. An answer is only
safely deferred when both are low.

`why` is **required** where `reversal = recreate` or `blast = high`, so that when
satz refuses or warns it can quote the pack's own sentence rather than a generic
one — the same rule that makes `reason` mandatory on a `deviates` claim.

**An empty answer that means something.** A question whose param may be `""` says what
`""` means with `empty = "…"`. The empty value is then an answer like any other: `satz
questions` shows it with its meaning, `satz interview` offers it, `--accept-defaults` and
`satz_interview`'s `accept_defaults` bind it, and a bound `""` counts as answered for the
gate. Without `empty`, a `""` the estate binds for a param whose pack default is also `""`
is a value nobody has given, and the question stays open. `empty` belongs to a question
about one param; a choice's "none" is its empty answer.

```
params {
  team_folder_name = ""
}

question team_folder_name {
  prompt   = "Display name of a folder for the team"
  reversal = edit
  blast    = none
  empty    = "no team folder"
}
```

`recommend` is the answer the pack would give. The interview prints it where it
differs from what is on offer; what Enter accepts, and what `--accept-defaults`
binds, stays the `params` default — so a pack can recommend switching on a service
that costs money without any bulk run switching it on.

**A question must be declared in the file that declares its param.** Questions are
absorbed after the `use … when` guard, exactly like claims — so a question that
*gates* a pack cannot live in the gated pack, or it would be invisible until the
answer was already yes. satz refuses it at parse time.

#### An exclusive choice

```
params {
  group_model_flat  = true
  group_model_split = false
}

question oneof group_model {
  prompt   = "Which security-group model does this customer run?"
  why      = "Splitting network authority out later means a new group, membership moves and re-granted roles."
  reversal = state_surgery
  blast    = low
  required = true // exactly one, rather than at most one
  option group_model_flat { label = "Flat — network authority sits with project admins" }
  option group_model_split { label = "Split — a separate network-admins group" why = "For a distinct network team." }
}
```

The options name **existing boolean params**, so an answer set stays a plain param
map and a question never becomes a second way to set a value. satz refuses two
true branches, naming the choice and both params.

A choice that is not `required` has one more answer: none of its options. `satz
interview` offers it as `0) none`, the MCP tool `satz_interview` takes `"none"`, and
either binds every option `false`; while no option is on, `none` is what the choice
offers. So a required choice needs two options and one that is not required needs one
— the option and none are its two answers:

```
params {
  notice_pubsub = false
}

question oneof notice {
  prompt   = "How do the teams hear that a value changed?"
  reversal = edit
  blast    = low
  option notice_pubsub { label = "Pub/Sub — a message on a topic" }
}
```
 `required` is checked at compile and only when the choice applies —
a required choice whose `ask_when` param is false has no missing answer. `satz
questions` never refuses a required choice with no branch set: it reports it as
unanswered and blocking, which is what an interview needs in order to ask it.

Composition follows from that: a choice between two packs is two booleans plus the
`use … when` that already exists.

#### Where they show up

- `satz questions <estate> --format … --out <file>` — every question its packs contribute
  with its state:
  `answered` when the estate's own `params {}` binds the param (accepting a default
  is an answer, written as the default), `unanswered` with the default the pack
  offers or `blocking` when none is possible, `not-applicable` when its `ask_when`
  is false. `--unanswered` is the worklist, `--format markdown` the decisions sheet
  and `--format xlsx` the workbook a customer fills in.
  Offline and schema-free: an interview happens before anyone runs `update-schema`.
- `bootstrap` and `transpile --apply` **refuse while a question is unanswered**;
  `--dry-run` and `--plan` warn. `satz interview` asks and writes the answers —
  [satz interview](interview.md).
- `satz doc-packs` gives each pack a **Questions** section.
- A question's `prompt` becomes the `description` of the generated
  `variables.tf` variable — the pack already wrote the one-line sentence.
- `check-presets` reports a pack whose questions changed as **`questions`**, not as
  drift: what the pack emits is byte-identical, so the estate is not forked, and the
  change — a `recreate → edit` downgrade included — is still listed. Questions are
  canonicalised separately from the body for that reason.

### 6.15 `notice` — the command a pack asks for once it is on

A pack that needs one step taken after it goes into an estate names it here. satz shows
the notice when the pack is switched on, and keeps showing it until the estate binds the
notice's param `true`.

```
pack showcase_pack version "1.0"

params {
  pack_bucket_location = "EU"
  pack_bucket_adopted  = false
}

notice pack_bucket_adopted {
  text     = "The bucket may already exist in the project — creating it again fails. Import what is live first."
  run      = "satz adopt <estate> --execute --import"
  severity = error
}
```

| key | value | meaning |
|---|---|---|
| `text` | a string | what to do and why, as the operator reads it. Required |
| `run` | a string | the command to run. Required |
| `severity` | `error` \| `warning` \| `info` | what an open notice holds back: an `error` refuses every command that writes to the organisation, a `warning` is printed and the run goes on, an `info` waits for nothing. `warning` where the pack declares none |

**The binding is the acknowledgement.** The pack declares the param `false`; the estate
binds it `true` when the command has run, in its own `params {}` — the same record an
answer is (§6.14, [ADR 0006](adr/0006-an-answer-is-a-param-the-estate-binds.md)). Git says
who and when, a second operator and CI read the same state, and there is no local file
beside the estate.

**A notice belongs in a pack, and its param is the notice's alone.** It is declared in
the same file, as `false`, and no `question` asks it — an acknowledgement is not a
customer decision. The param is never emitted: it is in no `variables.tf` and no
`terraform.tfvars`, so binding it moves nothing in the plan. `satz pack-graph` refuses a
library where another pack declares or reads it, or where a notice sits on the day-0 pack
or the map, which nothing switches on.

#### Where they show up

- **When the pack is switched on** — a yes in `satz interview`, `satz add-pack`, or
  `merge-presets` bringing the pack in — the notice is printed once, and the MCP tools
  `satz_interview` and `satz_add_pack` return it in `notices`.
- **Until it is acknowledged** the compile warns at the estate's `use` line of the pack —
  a `notice` finding whose subject is the param and whose `fix:` line is the pack's `run`,
  with the estate's file name where the pack wrote `<estate>` —
  and every command that writes to the organisation — `transpile --apply`, `bootstrap`,
  `run-actions --execute`, `migrate` — refuses while a `severity = error` notice is open.
  `--plan` and `--dry-run` compile, print the finding and go on.
- `satz adopt --execute --import` **acknowledges the notices that name it** when the run
  covers every resource type and nothing failed: it binds their params itself.
- `satz packs` lists each pack's notices with their state, and `satz doc-packs` gives a
  pack a **Notices** section.
- `check-presets` reports a pack whose notices changed as **`questions`**, not as drift:
  a notice emits nothing, so a reworded one never forks an estate.

### 6.16 `offers` — what the library offers an estate

`presets/estate-map.satz` — the map, `pack estate_map` — carries one `offers` entry per
pack in the library. The entry says what an estate's line for that pack looks like and
where it goes; the entries' order is the order the packs can be adopted.

```
offers "presets/essential-contacts-organization.satz" {
  when  = use_essential_contacts
  phase = "once the estate runs as the service account — one contact for Google's notices"
  block = "google_essential_contacts_contact"
}

offers "presets/monitoring/organization-audit-logsink.satz" {
  when  = use_audit_logsink
  phase = """once the estate runs as the service account — the audit archive, which every later
logging pack points at"""
}

offers "presets/cis/cloud-sql-dry-run.satz" {
  when     = cis_cloud_sql_hardening_dry_run
  excludes = ["presets/cis/cloud-sql.satz"]
}
```

| key | value | meaning |
|---|---|---|
| `when` | a param | the gate the pack's line carries (`use "…" when <param>`); every entry but the map's own has one |
| `phase` | a string | opens a group of lines: what has to be finished before they can go in, written as the comment above them |
| `block` | a resource type | the line is written inside that resource type map (`google_essential_contacts_contact`), for a pack that is a bare list of labelled bodies; a value naming a node of the estate is refused |
| `by_hand` | a string | satz writes no line for the pack; the string says how it is used. Takes no `phase` and no `block` |
| `requires` | a list of pack paths | packs this one needs in a way its params do not show |
| `excludes` | a list of pack paths | packs this one never goes in beside |

An entry emits nothing and the compile never reads one. An `offers` entry anywhere but
in `pack estate_map` is refused at parse time.

`satz pack-graph` reads the entries and writes `presets/pack-graph.json`: every library
file as a node, and the edges between them. Most edges are derived from the packs — a
param one pack reads or contributes to and another declares (`data`), a gate a pack
declares for another (`gate`), an `ask_when` (`asks`), the options of one `question oneof`
(`excludes`) — so
`requires` and `excludes` on an entry name only what the packs cannot show, and
`pack-graph` refuses a declared edge it derives. A `data` edge from a READ also puts the
reading pack's line after the declaring pack's; one from a contribution does not, because
a contribution is merged before the compile walks anything. Several `requires` on packs that exclude
one another are one requirement: any of them meets it.

The estate commands read that file from the estate's `presets_dir`, and one pack logic
answers them all (`src/packs.rs`). `init` and `interview --create` write the pack menu
from it. `satz packs` reports every node as the estate has it: the gate's answer and
default, the line — `active`, `ungated` (active without `when <gate>`), `commented`,
`absent`, `forked` (naming the `.local` fork) or `misplaced` (outside the block the graph
places it in) — whether the pack deploys, what it requires and what requires it; a `use`
the graph does not know is `unmanaged`. `satz add-pack` and `satz remove-pack` switch a
pack on or off by its gate or its path. A yes in the interview switches a line on as
`add-pack` does, and `merge-presets` writes the lines the estate lacks from the graph of
its pristine source. An active line of a gated pack that has no `when` is a compile
finding naming the line to write ([workflows](workflows.md#when-a-pack-line-has-no-gate)).
Every line satz writes goes where the graph's order puts it: after
the line of the pack before it in the same place, inside the block the graph names, after
the estate-core line — or, in an estate without one, after its top-level `params` — and at
the end for a pack placed after the scaffold.

A gate's value is the estate's own binding, else the default in the file that declares
it while that file is used. What a pack needs is read from the edges: its `requires` are
one requirement, each `gate` edge one, the map one when the map declares its gate, and
each set of `data` params one. Any pack of a requirement meets it; a `data` requirement is
also met when the estate binds the params the pack reads, or every param of the pack whose
default reads them — the runner grant with `ci_runner_service_account` bound to a runner
that another estate runs. An `asks` edge decides when a question is asked and is no
requirement.

The compile reports, from the same logic: a gate **the estate itself binds** `true` whose
line is commented or absent — skipping a line written `by_hand`, and counting a pack as
used when an `excludes` neighbour on the same gate is. A default in the map or in a pack
is the library's proposal and not this estate's answer, so a skeleton that has answered
nothing compiles clean at every validation level and `satz packs` is where the proposals
are read. Then: an active line of a gated pack without its
`when`, where the gate is declared; a pack that deploys while something it needs is off;
and, as an error before the fold, two packs on two gates that exclude one another both
deploying — a dry-run twin beside its enforcing pack named as the decision it is. A compile that stops
on `unknown param` adds the requirement that is off, inside the refusal itself, so the
file and line it names stay with it and an editor marks that line. With no graph there,
the compile notes it once and skips those checks.

`check-presets` reports a map whose entries changed like one whose questions changed:
the map emits the same, so the estate is not forked, and the change is listed.

### 6.17 `export` and `interface` — what the estate publishes to the HCL beside it

Customer teams write their own HCL beside a satz estate, with their own state and in
their own repositories. `export` names a value they read; `interface` groups the ones
one team reads. Each export is an output of the root module (`hcl/outputs.tf`, read
with `tofu output`) and of the generated modules under `hcl/interfaces/`, which a team
sources from wherever its code lives
([workflows: customer teams beside the estate](workflows.md#customer-teams-beside-the-estate)).

```
export "customer_domain"      = customer_domain description "The customer's primary domain"
export "infra_project_id"     = "${{google_project.infra.project_id}}" description "The infrastructure project"
export "infra_folder"         = "${{google_folder.infra.name}}" description "The infrastructure folder, folders/<number>"
export "infra_project_number" = "${{google_project.infra.number}}"
export "audit_bucket_url"     = "${{google_storage_bucket.audit_logs.url}}"
export "regions"              = [default_region, "europe-west4"]

interface "audit" {
  export "audit_bucket_name" = "${{google_storage_bucket.audit_logs.name}}" description "The audit log bucket"
}

interface "archive-team" {
  use interface "audit"
  export "archive_project_id"     = "${{google_project.archive.project_id}}" description "The team's project"
  export "archive_project_number" = "${{google_project.archive.number}}"
}
```

(`tests/smoke/yaml/showcase.satz`.)

- **An export outside every `interface` block is a core export.** Every interface module
  carries it, so each team's folder is complete on its own, and `hcl/interfaces/core/`
  carries the core exports alone.
- **An export inside `interface "<name>" { … }`** is an output of
  `hcl/interfaces/<name>/` alone — and of the module of every interface that uses it.
  The block holds `export` statements and `use interface` lines, and nothing else.
  The name is the folder: lowercase letters, digits and `-`, starting with a letter;
  `core` is the core module's and is refused. An interface declared in two files is one
  interface, and their exports merge; one file declares it once.
- **`use interface "<name>" [when <param>]`**, or `use interface ["<a>", "<b>"] [when
  <param>]`, inside an interface block puts the exports of the named interfaces into
  this interface's module too, and those of every interface they use in turn. A shared
  set — the network values a pack publishes as `interface "network"` — is declared once,
  and each team whose interface uses it receives it. `when` gates the line as it gates a
  pack line: a false param brings nothing, and a param no file declares is an error. The
  name is an interface some file of the estate declares, never a path, so no tool that
  reads `use "<path>"` lines treats it as a pack. Refused: a name no file declares
  (listing those that are), `core` (every module carries it), a cycle (naming the
  chain), and two exports of one name reaching one module from two interfaces (naming
  both files). An interface of `use interface` lines alone is a module of its own. The
  README's `From` column names the interface each value comes from; in the root
  module each export stays one output, `<interface>__<export>` of the interface that
  declares it.
- **The export's name** is the output's name: lowercase letters, digits and `_`,
  starting with a letter, and no `__`. A consumer reads it as `module.satz.<name>`. An
  interface's export may not take a core export's name: every module carries both, and
  the compile refuses it naming both files.
- **The value** is a param, a literal, a list of them, or a string that carries
  `${{type.label.attribute}}` references to resources the estate emits. An object is
  refused. A reference names something the estate emits, or the compile refuses it at
  the export's line, listing the labels of that type that are emitted; `${{…}}` that is
  not a `type.label.attribute` of a `google_*` resource is refused.
- **`description "…"`** follows the value on the same statement and becomes the
  output's `description` and the README's.

**Known now, or looked up.** Per reference, the compile decides what the value is:

| the attribute | the output in `hcl/interfaces/<name>/` |
|---|---|
| a param or a literal | the literal |
| an attribute satz writes on the resource — a `project_id`, a `name`, a `display_name` | the literal satz writes |
| an attribute derived from ones satz writes — a service account's `email`, a bucket's `url`, a topic's `id` | the literal |
| an attribute only the cloud knows — a folder's `name`, a project's `number`, a network's `self_link` | a `data` source that reads the resource back by what satz writes on it, through the consumer's own provider and credentials |

The derivations and the lookups are the table `presets/interface-lookups.yaml`, compiled
into satz: per resource type, the data source, the keys it is looked up by, the read
permission the lookup needs, and the attributes it yields. A key that is itself known
only to the cloud chains to that resource's lookup: a folder inside a folder is looked up
under its parent's lookup. An export that needs a lookup of a type the table has no row
for is refused, naming the type and the types the table reads back; so is an attribute
the row does not yield, naming the ones it does. A string that embeds a reference is a
template: `"projects/${{google_project.infra.number}}/x"` is `"projects/${data.google_project.infra.number}/x"`
in the module. A module holds the lookups its own outputs read and no other.

**One name is one output.** Exports come from the estate and from every pack it uses; a
pack switched off by its `use … when` exports nothing. The same name in the same
interface — or among the core exports — with the same value and description from two
files is one export, and with a different one it is an error naming both files.
`estate-core.satz` carries the core exports every estate that uses it publishes;
`satz init` writes two core exports into the estate, the infrastructure folder and
`workload_folder`, where the customer's and the teams' folders live
([the library](../presets/README.md#estate-coresatz)).

**What the compile writes.**

- `outputs.tf` in the root module holds every exported value in `local.satz_interface` —
  `{ interface = 1, estate = "<name>", core = { … }, interfaces = { "<name>" = { … } } }` —
  and one output per export that reads it. A core export's output keeps its name; an
  interface's is `<interface>__<export>`, with `-` written `_`
  (`archive_team__archive_project_id`), so two never meet. `presets/interface-notice.satz`
  publishes that local as an object.
- `hcl/interfaces/<name>/` for `core` and for every interface: `versions.tf` (the `google`
  provider and the version the estate pins), `main.tf` (the lookups, when there are any),
  `outputs.tf` and `README.md`, which names the interface and lists every output with
  the interface it comes from — `core`, its own, or one it uses — and how it is obtained. A module takes no variable, has no backend,
  reads no state and names no file outside its directory, so it works copied, moved or
  sourced by git URL.
- `satz transpile` writes `outputs.tf` and `hcl/interfaces/` whole: the folder of an
  interface the estate no longer declares is removed, and so are both when the estate
  exports nothing. The `.tf` files carry the stamp `main.tf` carries.

An export emits no resource and is no witness: it never enters the fold, the emission
manifest or a claim.

### 6.18 Provenance: pristine, fork, ledger

Suffix carries meaning; the tooling enforces it.

| file | meaning |
|---|---|
| `X.satz` | pristine, upstream-owned, always overwritable |
| `X.local.satz` | customer fork, never touched by updates |
| `X.diff.satz` | the current adoption delta (fork vs pristine), rewritten each merge |

- A semantic upstream change to a preset the estate **includes** (the canonical
  form of the parsed pack differs — params or body) auto-forks it and repoints the
  estate. Comment, format and version-line changes upgrade in place.
- Pack versions live **in-file**; filenames carry only framework versions.
  Never `X.local.2.satz`.
- Most customisation is **params**, the rest a `.local` fork. If a fork's whole
  diff could be a param, the param belongs in the pack.

---

## 7. Proof: `require`

`require` says, before anything touches the cloud, which controls the estate
*as written* discharges; a claim whose witness is not emitted reads as broken.

```
satz require cis-gcp-4.0 C0example.satz --config ~/estates/acme --format text --out -
```

```
require cis-gcp 4.0 — goal view for …/acme/satz/C0example.satz

  ◐ 1.1   Corporate login credentials only              — open duties: legacy-superseded, review-allowlist
  ✓ 1.4   Only GCP-managed service account keys         — google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation, google_org_policy_policy.iam_managed_disableServiceAccountKeyUpload
  ✓ 2.2   Sinks for all log entries                     — google_logging_organization_sink.organization_audit_gcs, google_storage_bucket.org_audit_logs, google_storage_bucket_iam_member.org_audit_sink_writer
  ◐ 2.3   Retention on the log bucket                   — no implements claim included
  ✓ 3.1   Default network does not exist                — google_org_policy_policy.compute_skipDefaultNetworkCreation
  ◐ 4.4   OS Login enabled                              — open duties: existing-vms
  ✓ 5.2   Uniform bucket-level access enabled           — google_org_policy_policy.storage_uniformBucketLevelAccess

18 satisfied, 5 partial, 0 deviation(s), 0 unmet, 0 broken claim(s), 0 contradicted claim(s). Goal view judges the DECLARED estate; live verification is the evidence report.
```

(Trimmed to seven of 23 rows.) Every ✓ carries its witnesses — the emitted
addresses the claim named and the compiler found. Rows are the catalog's
controls, string-sorted by id.

On an estate that declines two controls (a fork with `enforce = "FALSE"` and a
`deviates` claim for each), the same command reads:

```
  ⚠ 4.4   OS Login enabled                              — DEVIATION (CIS_GCP_Foundation_4_0) Deliberate: an operational service in this organisation depends on metadata SSH keys, which enforcing OS Login would break. The constraint is declared and managed here, with enforce = FALSE, so the decision is visible in the estate rather than absent from it. open: identify-service, reassess
  ⚠ 4.6   IP forwarding not enabled on instances        — DEVIATION (CIS_GCP_Foundation_4_0) Deliberate: workloads in this organisation require IP forwarding, so the constraint is declared and managed with enforce = FALSE rather than left undeclared. open: identify-workloads, reassess

7 satisfied, 3 partial, 2 deviation(s), 11 unmet, 0 broken claim(s), 0 contradicted claim(s). …
Deviations are disclosed decisions with a stated reason, not gaps — they do not fail this gate.
```

On an estate with no CIS pack, every row says what would provide it:

```
  ✗ 1.4   Only GCP-managed service account keys         — unmet. Provides: CIS_GCP_Foundation_4_0
  ✗ 2.2   Sinks for all log entries                     — unmet (no pack in the library provides it)
```

| glyph | status | meaning (`src/compliance.rs`) |
|---|---|---|
| ✓ | satisfied | ≥1 `implements` claim from an included pack, every witness emitted, no open duties |
| ◐ | partial | witnesses present but duties open, or only `contributes` claims |
| ⚠ | deviation | a declared non-conformance with a stated reason; never counted as a gap |
| ✗ | unmet | no included claim discharges it (none, or only ones that contributed zero witnesses); names the packs in the library that would |
| ‼ | broken claim | an included claim's declared witnesses are not emitted — ranks above unmet. A `deviates` claim whose declared witness vanished reads ‼ too, not ⚠; and ‼ yields to ✓/◐ when another included claim supplied the witnesses |
| ‼ | contradicted claim | the witnesses are all emitted and one of them does the opposite of what the claim says: an `implements` over a policy declared `enforce = "FALSE"` or `reset = true`, or a `deviates` over one that enforces. It outranks every other verdict on that control, including another claim's witnesses |
| ↳ | exempted | printed UNDER a control, not instead of it: a witness policy carries a conditional rule — a tag-conditional exemption — so the control is enforced and named resources are let out. The verdict comes from the policy's unconditional rule |
| ○ | organizational | the catalog marks it as having no IaC witness |

Exit code is **1 when anything is unmet, broken or contradicted**, 0 otherwise;
deviations do not fail it. In CI, an estate that drops a witness a claim depends on fails the
build, and an estate that declines a control with a `deviates` claim does not.

---

## 8. Evidence: `report-compliance`

The goal view joined with the cloud. Every witness of a
satisfied or partial control is looked up through Cloud Asset Inventory **in
its own scope** — a log metric by `projects/<number>/metrics/<name>` from the
`project` it is emitted with, an organization sink under the organization, a
bucket by its global name; a same-named resource in another project never
verifies a witness, and a project-scoped witness emitted without a `project`
reads *unverifiable* with that reason — and for org policies, compared **by
value**, because an inventory lists a switched-off policy like an enforced one.

```
satz report-compliance cis-gcp-4.0 C0example.satz --config ~/estates/acme \
  --format markdown --out evidence/cis-4.0.md
satz report-compliance C0example.satz --config ~/estates/acme \
  --format markdown --out evidence/held-to.md
```

The framework is optional. Named, the report is that catalog's. Left out, it is every
framework the estate is HELD TO — the catalog ids its `compliance_frameworks` param
names ([§8.1](#81-compliance_frameworks--what-the-customer-answers-to)) — one section per
framework in the one file `--out` names, each section the report that framework alone
produces. `--format json` then answers `{frameworks, reports}`, one report per framework,
whether the estate names one or three; named a framework, it answers that one report.
An estate that binds no `compliance_frameworks` is refused with the catalogs it could
name. One evidence record is appended per framework, named as it always is.

The report has seven columns — `Control | Title | Status | Witnesses (declared →
live) | Duties | Prowler | Checkov`; the title cell carries the catalog's own
`paraphrase` of the control under it, the witness cell the `interpretation` the
included claims give of what their resources prove, and an open duty prints
its text beside its id. Three rows, one of each shape (the two tool columns
omitted):

| Control | Status | Witnesses (declared → live) | Duties |
|---|---|---|---|
| 1.4 | **verified** | `google_org_policy_policy.iam_managed_disableServiceAccountKeyCreation` → ✓ `organizations/123456789012/policies/iam.managed.disableServiceAccountKeyCreation` · `…KeyUpload` → ✓ `…KeyUpload` | – |
| 2.1 | **verified** | two org policies → ✓ · `google_organization_iam_audit_config.org_all_services` → ✓ `organizations/123456789012` (the live policy audits `allServices` for every declared log type) | – |
| 2.3 | partial (open duty) | `google_storage_bucket.org_audit_logs` → ✓ `acme-organization-audit-bucket` | open: validate-then-lock — apply the bucket lock after the 30-day validation |

Status precedence, highest first:

1. **NOT ENFORCED** — a witness is live but not doing what the estate declares
   (an org policy's `enforce` differs). Outranks DRIFTED: a missing resource is
   absent from the inventory, while a switched-off one is listed like an enforced
   one.
2. **DRIFTED** — a declared witness is not live.
3. **partial (open duty)** — unattested duties remain.
4. **partial (contributes)** — only `contributes` claims.
5. **unverified (reason)** — no witness of the row could be checked at all
   (no credentials, inventory unavailable, no `project` to scope a witness).
6. **verified\* (n of m)** — some witnesses matched live, the rest have no
   live check for their type (Cloud Asset Inventory serves no witness for it).
7. **declared** — `--no-live`.
8. **verified** — every witness matched live.

(An inventory that was fetched and is empty is not "declared": the
witnesses are then *missing*, and the row reads DRIFTED.)

Plus **deviation (accepted)**, **deviation is STALE** (declared as a deviation,
but the live policy enforces — the fork no longer matches the organisation),
**BROKEN CLAIM**, **CONTRADICTED CLAIM**, **unmet**, and organizational. The verdict of an org policy is
its one unconditional rule; its conditional rules (a tag-conditional exemption,
for one) are listed beside the verdict. A policy with no unconditional rule or more
than one, or a list constraint, yields no verdict, and a policy whose live state
cannot be read reports *unverifiable*, never *verified*.

**Undeclared exemption bindings.** An estate that declares the exemption key —
`google_tags_tag_key.exemption`, from `presets/exemptions/exemption-tag.satz` — gets a
section after the table. `report-compliance` lists every
`cloudresourcemanager.googleapis.com/TagBinding` of the organisation through Cloud Asset
Inventory, keeps those whose value's namespaced name is `<org>/<key short name>/…`, and
subtracts the `google_tags_tag_binding` resources the estate declares on the key's values.
Bindings of any other tag key are not exemptions and are not listed. A live binding
matches a declared one when the value and the target agree; the target is compared as the
estate writes it and with its project id replaced by the project's number, the form Cloud
Asset gives a project. A reference in a declared `parent` resolves through the manifest —
a `google_project` to its `project_id`, an adopted `google_folder` to its `import-id`, a
`google_service_account` to its email; any other reference is not known before an apply,
and a live binding of the same value is listed with that declaration named beside it.

Each binding that remains is listed with its value's namespaced name and its target. A
claimed control whose witness policy has a conditional rule naming the value — a
`${{google_tags_tag_value.<x>.name}}` reference, `matchTag('<org>/<key>', '<value>')`, or
a literal `tagValues/<id>` — is named beside the binding, and its row's witness cell
carries `**undeclared exemption**: <value> bound to <target>`. The row's status stays
what its witnesses make it. `--fail-on undeclared-exemption` fails the run on any binding
the section lists, and on a section that was not checked — `--no-live`, a refused read, no
organisation id — because a gate asked to look that could not look has not passed; `any`
does not include it, so a pipeline names it. A read that is refused
makes the section **NOT CHECKED** with the reason and adds a warning; `--no-live` and an
estate without a customer-organization-id say so in the section. An estate without the
key has no section.

**Attestations** discharge manual duties. `attestations.yaml` beside
`config.toml`, one entry per duty id:

```yaml
validate-then-lock:
  by:   "Jane Doe"
  date: "2026-08-20"
  note: "bucket lock applied after 30-day pipeline validation"
```

An attested duty moves from `open: validate-then-lock` to
`attested: validate-then-lock (Jane Doe, 2026-08-20)` and stops holding the
control at partial.

**Evidence history.** Every run appends `evidence/<framework>-<timestamp>.json`
beside the config; a run in a minute that already has a record takes the next free
name, `…_002.json`, `…_003.json`, so no record is ever replaced and the history listed
by name is in the order the runs wrote it. A record holds `estate`, `framework`, `version`, `live`, `live_status`,
`warnings`, `verified_at`, `estate_commit` (`sha` + `dirty`) — and one row per
control with `control`, `title`, `status`, `responsibility`, `duties`,
`paraphrase`, `interpretation`, `prowler`, `checkov` and `witnesses`, each of
those an OBJECT: `address`, `state` (`verified` · `missing` · `diverged` ·
`unverifiable` · `not-checked`), `live_id`, `detail`, `conditional` (an org
policy's conditional rules, one line each) and `declared_at`
(`file` + `line`), and `undeclared_exemptions` — one `{value, target, policy}` per
undeclared exemption binding that lets one of the row's witness policies out.
`exemption_bindings` is `null` for an estate without the exemption key, and otherwise
`status` (`checked` · `skipped` · `no-organization-id` · `unavailable`), `key`,
`declared`, `live` (the live bindings of the key), `reason` and `undeclared` — each
`value`, `value_id`, `target`, `controls` and `unresolved_declared` — which is `null`
unless the check ran, so a refused read never reads as none found. The report's witness column is markdown; the data carries
none, so an agent can build an audit list from it. `responsibility` is `inherited`, `customer`,
`shared`, `satz-managed` or `unassigned` — the shared-responsibility split as a
derived fact, not a written-up matrix; `unassigned` means nobody has taken the
control yet, which is not the same as `customer`. Each run also writes
the report (`evidence/<framework>-latest.md`, or `--format pdf`; `--format
json` writes only the history entry, no markdown). `--prowler findings.json`
ingests the OCSF export of Prowler 5 (`prowler gcp --output-formats json-ocsf`) as
corroboration and records the Prowler version in `prowler_version`; an export from an
older Prowler is refused with the version it carries, and one that is not one JSON
document with the line, column and byte offset where it breaks — two scans written into
one file are named as the cause. FAIL findings whose check maps to
no control of the framework are counted per check in `prowler_unmapped`. `--checkov`
adds a column from a Checkov run over `hcl_dir`. `--no-live` produces a
declared-only report (statuses read *declared*) but still appends to the
history. `live` records whether the inventory was READ, not whether it was
requested: a run whose read was refused reports `live: false`,
`live_status: "unavailable"` and the reason in `warnings`, and its report header
reads **NOT VERIFIED** instead of naming the service it never reached. The other
outcomes are `verified`, `skipped` (`--no-live`), `no-organization-id` and
`no-witnesses`. The exit code is 0 whatever the verdicts; `--fail-on
not-enforced,drifted` (any status word, or `any`) makes the run fail for CI
after the report is written; `undeclared-exemption` beside them fails it on an exemption
binding the estate does not declare. The report states check semantics: "a resource with
these properties was verified at this time".

### 8.1 `compliance_frameworks` — what the customer answers to

What an estate CLAIMS comes from its packs. What its customer is HELD TO — a contract,
an auditor, a regulator — is a different fact, and the estate states it as a day-0 param
declared by `presets/estate-core.satz`:

```satz
params {
  compliance_frameworks = ["cis-gcp-5.0", "iso27001-2022"]
}
```

The values are catalog ids — the file stems in `<presets_dir>/catalogs/`: `cis-gcp-4.0`,
`cis-gcp-5.0`, `iso27001-2022`. A value naming no catalog is a compile error at the line
that binds it, with the catalogs that exist. The param decides what reports are written
against; it switches no pack on. It is read by `report-compliance` with no framework, by
`satz prowler`, and by a pack like any other param.

---

## 9. Quick reference

| I want to… | Write |
|---|---|
| declare an estate | `estate acme` |
| declare a versioned pack | `pack monitoring.logsink version "1.2"` |
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
| decline a control | `claim … deviates { resources = [...] reason = "…" }` |
| escape into raw HCL | `hcl { … }` — warns unless `hcl trust "…" { … }` |
| declare a step no resource can express | `action "scc" { reason = "…" run = "../scripts/x.sh" args = ["--org", "{customer_organization_id}"] }` |
| run a script inside the apply instead | `hcl trust "…" { resource "terraform_data" … provisioner "local-exec" { … } }` |
| multi-line string | `"""…"""` |
| offer a pack from the map | `offers "presets/x.satz" { when = use_x phase = "…" }` |
| name the command a pack needs once it is on | `notice x_adopted { text = "…" run = "satz adopt <estate> --execute --import" severity = error }` |
| add a pack's entries to another file's list param | `params { contributes_allowed_policy_member_subjects = ["serviceAccount:…"] }` |
| publish a value to the HCL beside the estate | `export "infra_folder" = "${{google_folder.infra.name}}" description "…"` |
| publish values to one team's module alone | `interface "team-a" { export "folder" = "${{google_folder.team_a.name}}" }` |
| comment | `#`, `//`, `/* … */` |

### Commands that consume this language

| command | layer | does |
|---|---|---|
| `transpile <estate>.satz` | Satz → HCL | emit `hcl/`; `--plan` / `--apply` run the tool afterwards, `--scan` runs Checkov, `--print-variables` prints the tfvars; `--format json` prints the compile as data — estate, addresses, files written, findings — and exits 1 on a refusal |
| `require <framework> <estate>.satz --format text\|json --out f` | Controls | goal view — declared estate vs catalog; exit 1 on unmet/broken |
| `report-compliance [<framework>] <estate>.satz --format markdown\|json\|pdf --out f` | Evidence | evidence report, verified against live; `--no-live`, `--prowler`, `--fail-on <statuses>` (exit code as the CI gate). With no framework it reports each one the estate's `compliance_frameworks` names, one section per framework in the one file, and `--format json` answers `{frameworks, reports}`. `pdf` is typeset by satz itself: no tool on PATH and nothing to install, and the same report renders to the same bytes on every machine |
| `questions <estate>.satz --format text\|markdown\|pdf\|json\|xlsx --out f [--unanswered]` | Satz | every question the estate's packs declare, with its state; `markdown` is the decisions sheet and `pdf` the same sheet typeset, `xlsx` the workbook a customer fills in |
| `interview <estate>.satz [--create] [--all] [--accept-defaults]` | Satz | asks the open questions at the terminal and binds each answer as a param; `--create` writes the estate from `estate-core` first |
| `packs <estate>.satz --format text\|markdown\|pdf\|json --out f` | Satz | every pack the pack graph offers as the estate has it: the choice, the line, whether it deploys, what it needs and what needs it, and the compile's pack findings; a `use` the graph does not know is `unmanaged` |
| `add-pack <estate>.satz <gate\|path> [--with-requirements] [--format text\|json]` | Satz | binds the gate true and makes the line active where the graph places it, with the packs that follow its gate; refused, naming them, while a pack it needs is off or one it excludes is on. The edited estate is compiled and restored when it does not compile |
| `remove-pack <estate>.satz <gate\|path> [--cascade] [--format text\|json]` | Satz | binds the gate false and leaves the line; refused, naming them, while a pack that needs it is on (`--cascade` switches those off too) or while its line is not gated on its gate |
| `update-prerequisites [<estate>.satz] [--report-only] [--format text\|json]` | Satz | what the estate's resource types oblige it to declare: the roles the IaC service account needs against what it grants, and the APIs the infrastructure project must enable against what it declares. Writes both into the estate; `--report-only` lists them instead |
| `whoami [<estate>.satz]` | — | the credential satz runs as and, with an estate, what that estate runs as — its service account, impersonated by the credential, in cloud mode; the credential itself in local mode — with the live checks that decide whether the next call works |
| `prowler <estate>.satz [--format text\|json]` | Evidence | prints the Prowler invocation this estate needs — scope, the frameworks its claims name, the OCSF output path — and never runs it |
| `remediation-plan <framework> <estate>.satz --prowler f [--checkov] [--out-dir d] [--merge f]` | Evidence | the remediation dossier: items per control and resource from the triage and the report, written as JSON, CSV and XLSX; `--merge` fills the authored columns from an `authored.json` written against the run's dossier |
| `check-presets <estate>.satz --format text\|json --out f` | Satz | drift of packs vs upstream |
| `merge-presets` | Satz | reconcile pack updates; forks + repoints on semantic change |
| `adopt <estate>.satz [--execute] [--import] [--activate] [--only t,…]` | Satz | resolve live ids of declared resources, write `"import-id"`s or import; `adopt-org-policies` is an alias |
| `plan` / `apply` / `hcl-init` | HCL | run the configured tool (`tf_tool`, OpenTofu by default) in `hcl_dir`; the estate is the one `--config` names and every other argument is the tool's, so a `.satz` file among them is refused |
| `run-actions <estate>.satz [--check\|--execute] [--only n,…] [--phase p]` | Satz | run the estate's declared `action`s (§6.13). Prints and stops by default; `--check` runs each action's own dry-run form, `--execute` the form that writes. Global `--no-actions`, `--no-pack-actions` |
| `import [<source>] [--all] [--only t,…] [--exclude t,…] [--import-config f] [-o <file>] [--into <estate>] [--as <estate>] [--organization <n>] [--wrap-all]` | — | create an estate from what exists (§12): a state file, `organizations/<n>` / `folders/<n>` / `projects/<id>` live, or a directory of `.tf`; `--from` forces the shape; `--into` imports only what the estate does not declare, as packs it `use`s; `--as` reads the scope as that estate's IaC service account; checked by `transpile` + `tofu plan` |
| `triage <framework> <estate>.satz --prowler f --format markdown\|pdf\|json --out f` | Evidence | every Prowler FAIL sorted into buckets A–E (a pack covers it / Satz declares it / declared exception / unmanaged / manual) — the remediation plan's skeleton; `--fix` adds the estate delta the buckets imply to the report (markdown only) |
| `scan [<estate>.satz]` | HCL | Checkov over `hcl_dir`, findings pointed at the Satz line that declared the resource; failed checks exit 1 |
| `review-pack <pack>.satz --format text\|json --out f [--against <estate>.satz]` | Satz | one pack against the library's bar — parses, formatted, a header sentence, a version with its changelog row, no value shaped like private data, no membership, no legacy constraint beside its managed replacement, a prerequisite row for every type it emits, and it compiles. A pack is a fragment, so it is folded into a synthesised estate (the documented example params, the pack's own defaults) unless `--against` names a real one |
| `pack-graph [--presets-dir d] [--check]` | Satz | builds the library's pack graph from the map's `offers` entries and the packs' own param references and `ask_when`, checks it, and writes `<presets_dir>/pack-graph.json`; nothing is written while a check fails, and `--check` fails when the file is behind the library |
| `doc-packs [--out-dir d] [--check]` | Satz | one page per pristine pack derived from the pack file (what it does, the `use` block, params, resources, claims with their catalog titles, duties, version history) + a grouped index with framework coverage; `--check` is the CI gate, and it also refuses an off-catalog claim, a header that says nothing and a pack version with no changelog row |
| `silence list [<estate>.satz]` / `silence add <kind>[:<subject>] --reason "…" [--machine]` / `silence remove <kind>[:<subject>] [--machine]` | — | what an estate or this machine leaves out of its printed output, named by a finding's `kind` and `subject`. `list` with an estate says what each row still silences, or that it is stale. A silenced finding stays in `--format json` and in what MCP returns; an error is never silenced. `--silence <kind>[:<subject>]` and `SATZ_SILENCE` do it for one run |
| `map-types [--only t,…]` | — | derive the API→Terraform field map per type into `presets/type-map.yaml` (from the Discovery Documents and the provider schema) |
| `bootstrap <estate>.satz [--dry-run] [--greenfield]` | Satz | first apply for a new organisation: management project, state bucket, service account |
| `migrate <estate>.satz --mode local\|cloud` | Satz | rewrite `deployment_mode` in the estate's params and move the state |
| `export-organizational-policies <estate>.satz [--output f]`; `diff-` / `report-organizational-policies <estate>.satz --format … --out f [--recursive]` | Evidence | the org-policy specialist tools: snapshot live policies as a pack, diff desired vs live by (parent, constraint), inventory report |

All of them accept `--config <estate-dir-or-config.toml>` and run from anywhere. A command
that produces a report takes `--format`, the rendering, and `--out`, the file it lands in:
one invocation, one artefact, one named path, and nothing on the console but the line on
stderr saying where it went — `--out -` pipes. `update-prerequisites` and `prowler` answer
on the console instead: an exit code and a command line to paste are not documents.
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
params: `a` is declared twice — line 3 and line 7; the second binding would be ignored, so delete the one that is not meant
a second `estate` header (f) — the file is already `e`
use ... as: given twice
`lifecycle_rule` is given twice in this block (first at line 12) — a repeated key would silently last-win; write a list (`lifecycle_rule = [ … ]`) or remove one
block `folder`: unknown resource type. Satz names Terraform types in full — write `google_folder`.
`x` is an attribute at the top level of the file — attributes live inside a resource block
pack header: `content` is not a header word — the header is `pack <name> [version "…"]`; delete `content`
`hcl` is a Satz statement: it is written at the top level of a file, never inside a block — move it out
use "presets/estate-core.satz" inside `google_essential_contacts_contact { … }`: that file holds no entry — only `params`, `question`, which reach the estate from any position. A file used inside `google_essential_contacts_contact { … }` holds labelled `google_essential_contacts_contact` bodies. Write this one at the top level of the estate: `use "presets/estate-core.satz"`
use "x.satz": x.satz:3 is a `suppress`, which is read from the estate alone — in a used file it is never applied. Write it in the estate, or take the resource out of the used file
use … when want_cs: unknown param `want_cs` — a `when` on a param nobody declares would silently drop the pack
use … as google_cloud_identity_group inside `google_org_policy_policy { … }`: the pack is this map's content; move the `use` to the folder or top level to re-key it
use "old-pack.yaml": a pack is Satz — satz v0.71.0 is the last release that converts the pre-Satz YAML dialect
use "x.satz": file not found
cyclic `use`: main.satz → a.satz → b.satz → a.satz
`terraform` is declared twice — one block per estate
google_org_policy_policy.p is declared twice in this file with different bodies (first at line 7)
grant: unknown key `description` in a conditional grant object — the keys are `role`, `condition`, "import-id"
claim: resources = [...] is required (a claim ships its witnesses)
claim … deviates: reason = "…" is required (a deviation is a disclosed decision, and the report carries the reason)
suppress google_org_policy_policy "x" matches nothing — stale suppression (typo or upstream rename)
suppress … role on google_organization_iam_member "group:x@example.com": the address is in conflict (⊥); suppress the whole member or resolve the conflict first
<type>.<label>: 2 disagreeing definitions — a.satz:12, b.satz:40   (under `composition conflicts`, at each site)
`deployment_mode = "boot"`: the mode is "local" (the state in a file) or "cloud" (the state in the gcs bucket), and no backend is emitted for anything else
`deployment_mode = "cloud"` without a value for `svc_iac_account`: cloud mode runs every live call and `tofu` as `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`, so the estate binds both — bind it in `params {}`, or keep `deployment_mode = "local"`
transpile: estate.yaml is written in the pre-Satz YAML dialect, which satz does not read. satz v0.71.0 is the last release that converts it
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
| `action: unexpected entry … (keys are reason, run, args, execute_args, phase)` | An unknown key is refused. |
| `action "x": no interpolation allowed` | The name and `run` are literal; only `args` and `execute_args` interpolate. |
| `action "x": declared twice — a.satz:3 and b.satz:9` | Names are unique across the estate. |
| `run = "x.sh" not found. Looked in: …` | Every place that was tried, in order: the declaring file's directory, then the include dirs. |
| `… is not executable. chmod +x …` | satz does not set the bit; a script that arrived via `get-presets` becomes executable when someone runs the `chmod +x`. |
| `… is a Python action, and satz runs one with uv, which is not on PATH` | Install uv. satz runs no other interpreter for a `.py` action. |

## 11. Known limits

- **Param scoping is document-ordered, not lexical.** Packs see every earlier
  file's params.
- **No list concatenation.** Overriding a list param replaces it. A pack adds to one
  with `contributes_<param>`; an estate repeats the entries it keeps.
- **`use … when` is followed unconditionally when computing which presets an
  estate uses** (`check-presets`), so a conditionally-disabled pack may be
  reported as included, so drift is over-reported rather than missed.
- **No `force` / priority channel.** "Keep my version of one pack resource" is
  a fork; `suppress` + redeclare cannot express it because the
  redeclaration lands at the same address and folds to a conflict.
- **An estate with no resources emits no `main.tf`** — only `providers.tf`,
  `variables.tf` and `terraform.tfvars`.
- **`satz apply` does not run actions.** `run-actions` is a separate verb, and
  `phase` only orders and selects — nothing enforces that a `before-apply`
  action ran before the apply. Coupling the two would change what `plan` means,
  which is the same reason `plan` does not transpile.
- **An action produces no evidence.** Nothing is recorded when one runs, and no
  claim can rest on it: satz cannot tell whether a given step ran against an
  organisation.
- **`suppress` cannot remove an action.** `--no-pack-actions` drops every
  pack-declared one; there is no per-action subtraction.

---

## 12. Importing what exists

`satz import <source>` writes a Satz estate from something that already
exists. The shape is read off the source; the check is always the same —
`satz transpile`, then `tofu plan` against the real state must show no
destroy for what was already managed. Import ids for the live shape are the
asset path; for the others, `satz adopt` resolves them afterwards. A nested value
the API does not return while it holds the provider's default (a subnet's
`log_config.filter_expr`, default `"true"`) plans once as an in-place update to
that default; the first apply writes it and nothing about the resource changes.

| shape | when | what you get | limitations |
|---|---|---|---|
| `import state.json` (or `-` for `tofu show -json` on stdin) | you already run Terraform/OpenTofu and want the estate that reproduces its state | folders/projects nested, grants collapsed to member → roles with one line per edge, services into `project_service` with one line per service, a single nested block as a block, the organization — named by a resource (`organizations/<n>`, `org_id`) or by a top-level folder's parent, else by `--organization <n>` — referenced as `customer_organization_id`, the day-0 params bound from what the state implies (the live row says which rules; the ADC's facts are not read here) with inferred values marked `// inferred:`, every resource with its `"import-id"` from the state id; labels are the state's own (they are its addresses) | only `tofu show -json` output (a raw `.tfstate` is refused); a state that names no organization and no `--organization <n>` is refused and nothing is written; `--organization <n>` against a state that names another organization is refused naming both; a grant whose scope the state does not carry (`org_id`, `billing_account_id`, `folder`, `project`, `bucket`) is refused, naming the resource; a resource without `id` gets no import id; grant conditions are not carried; a row's attribute `exclude`/`map` are not applied to this shape; rows with `import: false` are skipped and listed (`type off`) unless `--all` takes every row; a grant one principal holds on two folders or two projects is refused (the map form emits one address per member and role) unless `--on-collision counter` writes the second and later as labelled resources with a running number |
| `import organizations/<n>` \| `folders/<n>` \| `projects/<id>` (or bare `import` with `root:` in `import-config.yaml`) | brownfield: nothing is in Terraform yet | one Cloud Asset sweep of the scope; every enabled type; folders and projects nested, folders labelled by display name (the number appended only where two share one), grants collapsed to member → roles with one line per edge (a bucket's or a service account's pinned to its scope in the map, one map per scope), services into `project_service` with one line per service, an org policy as its bare constraint with `spec { … }`, a single nested block as a block, the organization referenced as `customer_organization_id` wherever its number was written; what the platform owns — the built-in `_Default`/`_Required` sinks, service agents' grants, the legacy bucket grants, Google-created service accounts, a project that is no longer ACTIVE — skipped and listed under the `skip:` pattern of its import-config row that matched; the providers' quota project is the first project (by id) that enables the Org Policy and Service Usage APIs, and the report names it, or says that none does; the day-0 params (`presets/estate-core.satz`) bound from what the ADC states (`customer_id`, `customer_domain`, `first_admin`, a single open billing account) and what the sweep implies — the service account granted organizationAdmin at the organization gives `svc_iac_account` and `infra_project_name`, that project gives `infra_folder_name`, `infra_bucket_name` (its one versioned bucket) and `billing_account_infra`, the members give `svc_iac_users_group`, the regional resources give `default_region`, the leading token of the project and bucket names gives `customer_shortname` (`--customer-shortname` wins) — an inferred value carrying `// inferred:` with its rule, a value nothing states left out and reported (`customer_longname` always), and every bound literal referenced wherever the body repeats it (`"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com"`); import ids = the one derivation of §12.2 — the asset path with a project number replaced by the project's id and a DNS zone's number by its name, rendered through the row's `import_id` template where it has one (`{project} {name}` for a log metric), a template placeholder nothing fills refusing the resource; required attributes the asset lacks derived (`parent`, `org_id`/`folder`/`project`, `location`/`region` from the asset name, and from the row's `import_id` template matched against the asset path segment for segment — which tells a parent's id from the resource's own, a workload identity provider's `workload_identity_pool_id` from its `workload_identity_pool_provider_id` — else a `*_id` from the path's last segment, a service account's `account_id` from its email); API vocabulary the provider spells differently is renamed per the row's `map:` (a firewall's `allowed[].IPProtocol` → `allow { protocol }`), what the API nests is flattened onto the attribute the provider names — a value whose name is an attribute of the resource, or an object of that name holding a single `enabled`/`value` (`iamConfiguration.uniformBucketLevelAccess.enabled` → `uniform_bucket_level_access`, `iamConfiguration.publicAccessPrevention` → `public_access_prevention`, `billing.requesterPays` → `requester_pays`) —, a fact the API states in other terms is translated (a lifecycle condition's `isLive` → `with_state = "LIVE"` / `"ARCHIVED"`, an empty `expirationPolicy` → `expiration_policy { ttl = "" }`, which is never expires), an empty string on a REQUIRED attribute is a value and is carried (a subscription's `expiration_policy.ttl = ""`) while an empty string on an optional attribute — which the provider reads as unset, and which an enum it validates refuses — and an empty value on a key the schema does not name are dropped, and self-link `region`/`zone` and full-name `name` values are shortened to what the provider takes | only rows with `import: true` AND an `asset_type` are swept (21 enabled by default — the landing-zone types plus VPC network/subnet/firewall, Pub/Sub topic, Secret Manager secret, log metric, Artifact Registry repository, each verified live to plan as import-only; `--all` switches on every row with an `asset_type`; `--only` narrows, never widens; `--exclude` leaves types out; an enabled row with `asset_type: TODO` is an error); IAM conditions are not carried; no Cloud Identity groups (not in Cloud Asset — state shape or `adopt`); a resource whose required attribute cannot be derived is skipped and named; an asset type Cloud Asset Inventory does not serve is found by halving the refused request, left out and named at the end of the run with the rows that asked for it, while any other fetch error aborts the run, and so does a sweep in which EVERY type asked for was refused — that is the scope, however few types were asked; the run prints as whom it sweeps before the first request; a page is 1000 assets; a grant one principal holds on two folders or two projects is refused (the map form emits one address per member and role) unless `--on-collision counter` writes the second and later as labelled resources with a running number; a type no row maps is skipped as `unmapped` — `--generate-unmapped` asks the provider for its configuration instead (§12.2); `force_destroy` is a Terraform-only switch no API reports and is never imported — an estate that wants it declares it; a key the provider does not speak is dropped, counted per type and named with `--verbose`, while an attribute the provider schema DOES name whose value the import could not place is printed per resource with its reason on every run, because an apply would reset it |
| `--into <estate>` | the estate exists; take over what it does not declare yet | packs `imported-<scope>[-<container>].satz` plus `use` lines inserted at the declaring folder/project, and `imported-<scope>-generated.satz` with `--generate-unmapped` (§12.2) | live shape only; the scope must be the estate's `customer_organization_id` or inside it (a folder or project is walked up through Resource Manager as the estate's identity), else refused before the sweep; runs as the estate's IaC service account, or as the caller's credentials for a local-mode estate or under `--no-impersonate`, and says which; if ANY declared resource fails to resolve live (error, ambiguity), nothing is written; the packs, the `use` lines and the removal of a pack nothing is left in are written only after every step that can refuse has run; the packs are regenerated wholesale on every run — hand edits go into the estate, never into an `imported-*` pack; a `use` is inserted automatically only where the container is declared in the estate file itself |
| `--as <estate>` | the estate exists and holds the role, and you want a new file rather than the delta | the same sweep as a bare `import`, read as that estate's IaC service account | live shape only; refused together with `--into`, which names an estate already; refused for an estate that impersonates no service account — a local-mode estate, or any estate under `--no-impersonate` — since the sweep would read as the caller, which a bare `import` does; the scope must be the estate's `customer_organization_id` or inside it, else refused before the sweep; the estate is read for its `svc_iac_account`, `infra_project_name`, `deployment_mode` and `customer_organization_id` and nothing else, and nothing is written into it |
| `import ./hcl/ [--wrap-all]` | hand-written or generated `.tf` (`gcloud beta resource-config bulk-export`, `tofu plan -generate-config-out` — run by hand, or by `import --generate-unmapped`) | `variable`/`locals` promoted to params; schema-known, identifier-labelled `resource` blocks whose values are literals, params or `${…}` references as Satz resources placed under the folder/project they reference; a `count = length(<promoted list>)` whose `count.index` indexes that list expanded into one resource per entry, labelled after the entry; everything else verbatim in `hcl trust "imported from <file>:<line>"`; the report says why per block | `terraform`/`provider` blocks are always dropped (the emitter writes them), `--wrap-all` included — and `--wrap-all` promotes nothing, since a param is a translation; services and grants lose their labels (they become list/map entries); a `project_service` with more than `service` is wrapped; a scope written as an expression satz cannot place is wrapped; the estate gets a local backend and google/google-beta providers to edit, and is bound to the one organisation the configuration names (a literal `organizations/<n>` parent, an `org_id`, an org policy's parent) or `--organization <n>` names — refused, writing nothing, when neither does, when the two disagree, or when the configuration names two; `--wrap-all` names none, so it needs the flag; no import ids (`adopt`); a `${…}` reference is opaque to the compliance plane; a `${…}` from a translated block to a wrapped one, or to an address no block declares, refuses the whole import |

### 12.1 From existing Terraform (`./hcl/`)

Three tiers. A `variable` with a literal `default`, and a `locals` entry with a
literal value, become **params** — params are Satz's variables, so the result
stays re-parameterisable. A `resource` block of a schema-known type becomes a **Satz
resource** when every value is a literal, a promoted param, or a reference to a
managed resource; the folder/project it references (`parent`, `folder_id`,
`project`) decides where it is placed. Every other block is carried **verbatim**
inside `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
written, but the fold cannot compose it and the compliance plane cannot see into
it. The report accounts for every block.

A promoted declaration that a wrapped block still reads is **carried verbatim as
well**, so the wrapped block's `var.x` keeps resolving. A `${…}` reference is
opaque to the compliance plane: a claim cannot reason about it.

A translated resource may reference only another translated one. satz emits no
address for a verbatim block — `hcl trust` is text, and the emission manifest
does not hold it — so a `${…}` that crosses from a translated block to a wrapped
one, or names an address no block in the import declares, is an estate
`satz transpile` refuses with `written-reference`. The import refuses first,
names both sides with the reason the other one is verbatim, and writes nothing:
make the referenced block translatable, import the file that declares it too, or
carry everything verbatim with `--wrap-all`.

| HCL | Becomes |
|---|---|
| `resource` of a schema-known type, identifier label, values that are literals, promoted params or `${…}` references | a Satz resource: attributes as written, repeated nested blocks → a list of objects, `lifecycle` as declared; the label kept for plain resources (services and grants become list/map entries and lose theirs) |
| `variable "x" { default = <literal> }`, `locals { y = <literal> }` | **promoted** to `params { x = … }`; `var.x` becomes a bare param reference, `"a-${var.x}"` the interpolation `"a-{x}"` |
| `variable "x"` with no `default` | named in the header, given no value — `satz transpile` stops with `unknown param 'x'` until it is bound, which is the same gate the source had |
| a reference to a managed resource (`google_project.p.number`), anywhere in a value | carried verbatim as Satz `${{…}}`, which emits back byte-identically |
| a `*_iam_member` whose scope is neither project, folder nor organisation (a service account's, a bucket's) | the **scope-pinned member map** of §6.5, one map per scope value (`bucket = "${{google_storage_bucket.logs.name}}"` beside the members); a grant type whose schema singles out no scope attribute stays a labelled resource |
| a resource whose `project` is a **literal** id of a `google_project` in the input (`gcloud … bulk-export` and `tofu plan -generate-config-out` write literals) | placed under that project, the attribute dropped — the same as a reference; a literal id no project in the input carries stays an attribute at the top level |
| `google_folder` (`parent` = the organisation, or a reference to a folder in the input), `google_project` (`folder_id` a folder reference, or `org_id`), `google_project_service` / `google_project_iam_member` / any project-scoped resource whose `project` references a project in the input, `google_folder_iam_member`, `google_organization_iam_member` | placed: nested under the folder/project they reference; grants become grant-map entries (a `condition` block travels in the object form), services the project's `project_service` list; `customer_organization_id` is inferred from the literals |
| a project whose `folder_id` is a folder *number*, a resource whose parent is wrapped, a scope written as any other expression, groups, memberships, `*_iam_binding` / `_policy` / `_audit_config` (authoritative: they own every binding on their target), billing grants (they hoist to the estate's billing scope), a grant without a resolvable `member`/`role`, a `project_service` carrying more than `service` | wrapped, with the reason (closure by dependency: a child of a wrapped container is wrapped too) |
| `provider = google-beta.google-beta` on a resource | kept as the Satz body key `provider`, which emits back as the reference it was. The estate's own `google.google` is left out — the emitter writes it on every resource that names no other. A folder, a project, a project's service list, a group and a grant map are built from a form with no room for an alias: there an alias that is not the estate's own wraps the block |
| `depends_on = [google_project_service.x]` | dropped, and reported per block (`--verbose`). satz derives a plan's ordering from the estate — a grant on a declared service account or group waits for it, the policies on one parent are chained, a resource waits for the services that enable its APIs. An edge naming a block this import does not carry wraps the block instead: nothing can re-derive it |
| a `resource` using `count`/`for_each`/`dynamic`, a function call, a conditional (`? :`), an arithmetic or `for` expression, a `data.`/`module.` reference, a name no `variable` or `locals` declares, a string holding a literal `${` (written `$${`), a type not in the schema, a label that is not an identifier (letters, digits and `_`), a grant whose `member` is not `<type>:<value>` (a grant map reads a key with no `:` as the scope it pins) | wrapped, with the reason |
| `module`, `data`, `output`, `moved`, `import` | wrapped |
| a `variable` or `locals` whose value is not literal | wrapped, and the names it declares stay Terraform variables |
| `terraform`, `provider` | dropped (one note each), also under `--wrap-all`. A `provider` block's default `project` is carried as **placement**: a resource that named no project of its own lands in that project when the default resolves to one of the imported projects, and the dropped row says so |

**Which row an asset is mapped through.** An asset type may be named by several
import-config rows: `logging.googleapis.com/LogSink` is named by
`google_logging_project_sink`, `google_logging_folder_sink`,
`google_logging_organization_sink` and `google_logging_billing_account_sink`, and
`storage.googleapis.com/Bucket` by `google_storage_bucket` and
`google_storage_bucket_iam_member`. Two things decide, in this order:

1. the **content type**: an asset carrying the resource is mapped through a
   `content_type: RESOURCE` row, one carrying an IAM policy through an
   `IAM_POLICY` row;
2. the **parent**, read off the asset's own name — `projects/…`, `folders/…`,
   `organizations/…`, `billingAccounts/…` — and compared with the parent each
   candidate Terraform type is for, which the provider spells into the type name
   (`google_logging_folder_sink` is for a folder). A type that names no parent
   serves any parent, and is taken only where no type that names one fits.

Where the two leave one row, that row maps the asset. Where they leave several —
the provider has `google_compute_router`, `_router_interface`, `_router_nat` and
`_router_peer` for `compute.googleapis.com/Router`, and none of them names a
parent — the asset is reported `ambiguous` with the types named, and `--only
<type>` picks one. Where they leave none because every enabled row is for another
parent, the asset is reported with that reason. The table's own order decides
nothing.

### 12.2 What the live sweep cannot map (`--generate-unmapped`)

The live sweep fetches the asset types the enabled import-config rows name and
maps each asset through its row. What comes back and cannot be expressed is
skipped and listed as `unmapped`: a required attribute that is not in the asset
data and cannot be derived, an asset whose data holds nothing the provider schema
knows, an asset whose content type or parent no enabled row covers, an asset with
no organisation/folder/project scope at all. (A type no row names is never
fetched, so it is not among them — give it a row.) `satz import <scope>
--generate-unmapped` hands exactly those to the provider:

```bash
satz import organizations/123456789012 --generate-unmapped
```

It writes a scratch directory — `<base>-generate/` — holding
`imports.tf`: the providers at the versions `config.toml` pins, and one

```hcl
import {
  to = google_dns_managed_zone.corp
  id = "projects/acme-infra-001/managedZones/corp"
}
```

per unmapped resource. Then `tofu init` and `tofu plan
-generate-config-out=generated.tf` run there, and `generated.tf` goes through the
hcl shape of §12.1 into `<base>-generated.satz`. The two files stay separate: one
is what satz translated, the other what the provider wrote, and which of it
belongs in the estate is a reading decision — the generated file carries no `use`
line, and joining it is one.

The id is the one the mapped resources' `"import-id"` carries — one derivation
for both, and for `--into`'s subtraction below:

1. the asset's relative resource name, with a project number replaced by the
   project's id and a DNS managed zone's number by the zone's name, both read off
   the Project and ManagedZone assets of the same sweep;
2. `google_compute_instance_settings` without the `/InstanceSettings` its Cloud
   Asset name appends — the provider imports the singleton by its collection path;
3. rendered through the import-config row's `import_id` template where the row
   has one: each `{placeholder}` from the path segment standing where the
   template puts it, else `{project}` from the path, else the resource's own
   attribute of that name.

| type | template | id |
|---|---|---|
| `google_compute_instance_settings` | `projects/{project}/zones/{zone}/instanceSettings` | `projects/<p>/zones/<z>/instanceSettings` |
| `google_dns_managed_zone` | `projects/{project}/managedZones/{name}` | `projects/<p>/managedZones/<zone name>` |
| `google_dns_record_set` | `projects/{project}/managedZones/{managed_zone}/rrsets/{name}/{type}` | `projects/<p>/managedZones/<zone name>/rrsets/<name>/<type>` |
| `google_logging_metric` | `{project} {name}` | refused: the path does not state `name`, and nothing of an unmapped resource's data is carried |

A record set whose managed zone this sweep did not read is refused with that
reason — sweep `dns.googleapis.com/ManagedZone` as well, or correct the id by hand
in `imports.tf`. A template placeholder that neither the path nor the resource
fills refuses the resource, naming the placeholder.

The child's `provider` blocks name `project` and `billing_project`,
`user_project_override = true`, and `impersonate_service_account` where the run is
bound to an estate. The project is the one the estate the resources are read for
bills to: the `billing_project` of the default `google` provider the estate
compiles to under `--into`, and the quota project the run writes into the new
estate's `providers` block otherwise. A run with no such project writes a provider
block that names none.

`<base>` is the file the run is named after, and both files are written beside
it. A plain sweep is named after the estate it writes, `discovered.satz` or the
`-o` path, so its files are `discovered-generate/` and `discovered-generated.satz`.
With `--into` it is the scope's top-level pack,
`imported-organizations-123456789012.satz`, so they are
`imported-organizations-123456789012-generate/` and
`imported-organizations-123456789012-generated.satz`, beside the packs of that
run: two scopes imported into one estate keep their own files, the way their
packs do. A `<base>-generated.satz` that is already there is the operator's to
merge: the run is refused, naming it, before anything is swept. Merge what you
keep into the estate, delete the file, and run again.

Only `unmapped` is generated for. A type switched off (`import: false`, `--only`,
`--exclude`), a platform-owned object and a resource whose parent is outside the
import were left out on instruction, and the fallback does not undo it. An
`ambiguous` resource is listed with its reason among the ones it cannot generate
for: no Terraform type was chosen, so there is no `to =` to write.

What it cannot generate for is listed with the reason, one line each:

```
generate-unmapped: 4 unmapped resource(s) the provider can be asked for, 1 it cannot:
  not generated  //aiplatform.googleapis.com/projects/acme-infra-001/locations/europe-west3/datasets/9 — no import-config
                 row names asset type aiplatform.googleapis.com/Dataset, so there is no Terraform
                 type to import as
```

The provider reads the resources one at a time, writes the configuration for the
ones it could read and reports the rest, so a refused id costs that resource and
not the run. What is on disk decides, never the exit code: every resource satz
asked for gets one of four verdicts, and the ones the provider reported carry its
own words.

```
generate-unmapped: 33 written, 1 written incomplete, 1 refused, 0 unaccounted for:
  written        //dns.googleapis.com/projects/acme-net/managedZones/1234567890 = google_dns_managed_zone.corp
  incomplete     //cloudasset.googleapis.com/organizations/123456789012/feeds/estate = google_cloud_asset_organization_feed.estate
      │ Error: Missing required argument
      │ The argument "billing_project" is required, but no definition was found.
  refused        //compute.googleapis.com/projects/acme-net/zones/europe-west3-b/instanceSettings/InstanceSettings = google_compute_instance_settings.instancesettings
      │ Error: Cannot import non-existent remote object
```

**written** is a `resource` block in `generated.tf` that no diagnostic names;
**incomplete** is a block the provider still reported — generated, and not
applicable as it stands, the message saying what is missing;
**refused** is no block and a diagnostic; **unaccounted for** is no block and no
diagnostic, which satz names rather than counting as either. A diagnostic that
names no resource satz asked for is printed whole. `generated.tf` is read back as
Satz with everything in it, and `imports.tf` stays where it is: correcting the
refused ids there and running the two commands by hand — `tofu plan
-generate-config-out=generated.tf`, then `satz import <that file>` — brings the
rest in. `init` failing, and a plan that generated no resource at all, end the
run.

With `--into` the delta's own subtraction applies to the fallback as well: an
unmapped resource whose import id IS a live id the estate already resolved to is
named and not generated for, because the estate has it — a zone Cloud Asset names
by its number and the estate declares by its name included. The test is equality
of the id; a resource the estate declares under an id of another form stays in
the list. That run is bound to the estate's IaC service account, and the
child reads as it: satz writes `impersonate_service_account` into the provider
block of `imports.tf`. A plain sweep is bound to nothing, and the block
impersonates nobody.

The flag is the live shape's. On a state file it is refused: those resources are
already managed, and their configuration is the `.tf` the state was applied from,
which the hcl shape reads. On the hcl shape it is refused too — that shape is
what READS `-generate-config-out` output.

### 12.3 The pre-Satz YAML dialect

satz reads no YAML estate or pack. `transpile`, `import` and every other command
take `.satz` and refuse a `.yaml` file by name, pointing at satz v0.71.0 — the
last release that converts it:

```
transpile: estate.yaml is written in the pre-Satz YAML dialect, which satz does not read.
satz v0.71.0 is the last release that converts it:

    cargo install --git https://github.com/tjirsch/satz --tag v0.71.0 --locked
    satz import estate.yaml --kind estate            # --kind pack for a pack
    cargo install --git https://github.com/tjirsch/satz --locked
    satz fmt estate.satz
    satz merge-presets --estate estate.satz
```

`satz fmt` puts the conversion in the canonical layout and `merge-presets` brings
it up to the current preset library. The conversion may need edits; `satz
transpile` and a `tofu plan` that shows no destroy for what the estate already
manages is the check.

`presets/import-config.yaml` and the catalogs under `presets/catalogs/` are data
files, not estates. They are YAML and stay YAML.

A brownfield estate never needs the dialect: the state, live and HCL shapes above
write Satz directly, through the same printer.
