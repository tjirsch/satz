# 0072 — an API is judged on the project its resource's provider bills to

- **Status:** accepted
- **Date:** 2026-09-30
- **Shipped in:** the release that follows

## Context

[ADR 0059](0059-a-projects-provider-alias-is-the-estates-provider-scoped-to-that-project.md)
made each per-project provider alias bill to its own project: a resource written inside a
`google_project` node carries `provider = google.project_<label>`, whose
`billing_project` is that project, and Google tests API enablement there. The
prerequisite plane stayed single-project. `prerequisites::apis`, `missing_apis`,
`declared_apis` and `write_apis`, the report's `infra_project` and `infra_services`, the
`plan`/`apply` preflight and `bootstrap` all judged every API against the
`project_service` list of `infra_project_name`.

So an estate whose project node holds a bucket, with `storage.googleapis.com` on the
infrastructure project and not on the bucket's project, passed every check and failed the
apply with a 403 naming the bucket's project; and one with the API on the bucket's project
alone was told to enable it on the infrastructure project, where nothing needs it. The
finding said so in words; the check did not.

Four choices were not obvious.

1. **Where the check learns which project a resource is billed to.** From the node the
   resource stands in (inside `google_project` → that project, otherwise
   `infra_project_name`), or from the emitted `providers.tf`, reading the resource's
   `provider` reference and that provider's `billing_project`.
2. **What the `plan`/`apply` preflight does on a project node's project** that this very
   run creates: Service Usage cannot read or enable anything on a project that does not
   exist, and the preflight refuses on any error.
3. **What happens to a resource whose provider names no billing project** — the default
   provider of an estate that binds no `infra_project_name`, or a provider the estate
   declares without one.
4. **The shape of `update-prerequisites --format json`**, which carried one project
   (`infra_project`) and one `gcloud` line (`enable_missing_apis`).

## Considered options

For 1:

- **1a · The emitted `providers.tf`.** A `Billing` map, provider reference → literal
  `billing_project`, parsed from the same text `tofu` reads; each resource is judged on
  the entry its `provider` names.
- **1b · The tree position.** Inside a project node → that project's id, else
  `infra_project_name`.

For 2:

- **2a · Check a project node's project only when it exists**: when the state holds its
  `google_project`, or an `import` block adopts it; otherwise say so in one line and go on.
- **2b · Check every billed project**, and let a project the run creates fail the
  preflight.
- **2c · Ask Resource Manager whether the project exists.**

For 3:

- **3a · A note naming the provider and the types it serves**, at `info`.
- **3b · Judge them on the resource's own project**, the one Google falls back to.
- **3c · An error.**

For 4:

- **4a · One record per API and project** (`apis[].project`), one `gcloud` line per
  project (`enable_missing_apis` a list), the default provider's project as
  `default_billing_project`, and `infra_project`/`infra_services` kept for what
  `bootstrap` creates and enables on day 0.
- **4b · Keep the one-project fields and add a per-project map beside them.**

## Decision

1a, 2a, 3a and 4a.

**1a.** The emitter decides the provider of every resource (`alias_for`) and the provider
blocks decide the billing project (`configure_google_provider`, `project_provider_block`).
Deriving the same answer again from the tree is a second implementation of both, and it
is already wrong in one place: a project's own `project_service` entries are emitted with
the provider AROUND the project, not the project's alias, so they are billed to the
infrastructure project and need `serviceusage.googleapis.com` there, not on the project.
A declared `billing_project` on the estate's own `google` block, a `google-beta` provider,
a pack whose project node the emitter writes with the default provider — the file is
right about all of them because it is what runs. The compile emits `providers.tf` before
the check, and the preflight reads the one in `hcl_dir`, so both halves read one source.

**2a.** The preflight exists because `tofu` refreshes every resource in the state before
it creates anything, and a refresh against an API that is off stops the run. A project
this run creates has nothing in the state to refresh: its `google_project` is created
first and its `project_service` entries after it, which the emitter's ordering (ADR 0023)
already handles. An adopted project exists and is refreshed through its imports, so an
`import` block counts as existence. The state is read only when some project node's
project declares an API, with `tofu show -json`, the same read `reset_replacements_for`
does; a state that cannot be read refuses the run. The default provider's project is
always checked, since `bootstrap` creates it before any apply. 2c was rejected: Resource
Manager answers 403 both for a project that does not exist and for one the caller may not
see, and a preflight that guesses between them is worse than one that reads the state.

**3a.** A provider without `billing_project` and `user_project_override` bills to
whatever Google picks — the resource's project for most APIs, the credential's quota
project for some — and satz does not know which. Judging on a guess (3b) is the silent
fallback this check exists to replace. It is not the estate's error (3c): a local-mode
estate with no infrastructure project is valid. The note says which provider, which
types, and what to bind.

**4a.** The old fields named a single project that the report was no longer about. Keeping
them beside a new map (4b) leaves two answers to one question in the output, and an MCP
client reading the old field would read the wrong one. `infra_project` and
`infra_services` keep a meaning that is still true — the project `bootstrap` creates and
the services it enables before the first apply — and `bootstrap` still reads them.

`update-prerequisites` writes each missing API into the `project_service` list of the
project it is missing on, in the estate file. A project declared elsewhere — in a pack —
is named with the APIs to add, and nothing is written, the roles included: the estate is
never left half-edited, as before.

## Consequences

- An estate with a resource inside a project node whose API is not in that project's
  `project_service` list is warned at every compile and by `transpile --plan`, and refused
  by `bootstrap` and `transpile --apply`, where it compiled and then failed live. The release is
  a minor one, and `presets/README.md`'s `## Breaking changes` carries the edit.
- The reverse — an API enabled only where it is used — no longer demands a second
  entry on the infrastructure project.
- `update-prerequisites --format json` changes shape: `apis[]` and `missing_apis[]` carry
  `project`, `enable_missing_apis` is a list of lines, `default_billing_project` and
  `unbilled` are new. The MCP tool returns the same object.
- The `plan`/`apply` preflight prints one line per billed project, and may run
  `tofu show -json` once more before the plan.
- The check trusts `providers.tf`. A resource whose provider reference is written by hand
  in an `hcl { … }` passthrough is not in the manifest and not checked, as before.

## Pros and cons of the options

### 1a · Read the billing project from `providers.tf` *(chosen)*

- **Good:** one source with `tofu`; covers every provider the estate or the emitter writes.
- **Good:** the compile check and the preflight read the same map.
- **Bad:** the check depends on `providers.tf` being emitted; when it is not, the compile
  already carries the error that stopped it and the API half says it did not run.

### 1b · Derive it from the tree

- **Good:** needs nothing but the manifest.
- **Bad:** a second copy of the emitter's provider choice, wrong for a project's own
  services and blind to a declared `billing_project`.

### 2a · Check a project node's project only when the state or an import holds it *(chosen)*

- **Good:** a run that creates a project is not refused for the project not existing yet.
- **Bad:** one more `tofu show -json` when a project node declares APIs.

### 2b · Check every billed project

- **Good:** nothing to read.
- **Bad:** every greenfield plan with a project node is refused.

### 2c · Ask Resource Manager

- **Bad:** a 403 means "absent" and "hidden" alike.

### 3a · A note for an unbilled provider *(chosen)*

- **Good:** says what was not checked and how to have it checked.
- **Bad:** a local-mode estate gets an `info` line on every compile.

### 3b · Fall back to the resource's own project

- **Bad:** a judgement on a guess about what Google bills.

### 4a · Per-project records, one line per project *(chosen)*

- **Good:** one answer per question; `bootstrap` keeps its two fields.
- **Bad:** a consumer of the old `enable_missing_apis` string breaks.

### 4b · Keep the old fields beside a map

- **Bad:** two answers, one of them about a project the check no longer judges on.

## Amendment — the preflight switches APIs on through the infrastructure project

- **Date:** 2026-09-30

The preflight's Service Usage calls — which APIs are on, and switching the off ones on —
are billed to the default provider's project, the infrastructure project, for every
billed project: `x-goog-user-project` names it. Before, they carried no quota project, and
Google billed them to whatever project it took for the credential, so the answer
depended on the credential. A workload project
whose own `serviceusage.googleapis.com` is off has its APIs switched on all the same.
An estate whose default provider names no `billing_project` sends none, as before.

This is what [ADR 0074](0074-a-live-import-writes-what-the-provider-reads-back-and-leaves-out-what-is-not-live.md)'s
amendment relies on: a live import adds to an adopted project's `project_service` every
API its imported resources need and the project has off, and the preflight — which
checks a project an `import` block adopts (2a) — switches it on before the plan.
