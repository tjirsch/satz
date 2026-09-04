# satz housekeeping

Most of this repository is code, and code tells you when it is wrong. A handful of
files are **derived from something outside the repository** — Google's provider
schema, Google's constraint catalogue, Google's asset-type list, a CIS benchmark
release — and those go stale in silence. Nothing breaks. The tests stay green. The
answers just quietly describe last month's Google.

The first half of this page lists every one of them: what it is, what refreshes it,
what makes it stale, and what catches it if nobody remembers. The second half is
`scripts/` — one section per script, because a refresh that can be a script is one,
and the rest of what lives there are the gates that keep this page honest.

## At a glance

| file | refreshed by | trigger | what catches staleness |
|---|---|---|---|
| `tests/schemas/google.json` | `scripts/update_schema_fixture.py --add` | the provider pin moves | **nothing** — run `--check` |
| `presets/cai-asset-types.txt` | by hand, from Google's docs | new asset types appear | indirect: unfilled `import-config` rows |
| `presets/import-config.yaml` (`asset_type`) | `scripts/update_import_config.py --cai-types` | the CAI list above changes | smoke: *"every derivable asset_type is filled"* |
| `presets/managed-constraint-equivalents.txt` | `scripts/update_constraint_equivalents.py` | Google ships a new managed twin | `cargo test` catches the *effect*, not the table |
| `presets/docs/*.md` | `satz doc-packs` | any pack changes | smoke: `doc-packs --check` |
| `presets/README.md` (`## Changelog`) | by hand, one row per pack version | a pack version changes | `doc-packs --check`: fails on a version with no row |
| `presets/catalogs/*.yaml` | by hand, from the benchmark | a benchmark release | **nothing** |
| `tests/corpus/*/expected.sorted.txt` | `UPDATE_CORPUS=1 cargo test` | emission changes | `cargo test` (that is the gate) |
| provider version pin | by hand | a provider release | **nothing** |
| crate versions | `cargo update` | routine | `cargo test` after the fact |
| `docs/competitive.md` | a battle review | quarterly, or a phase gate | **nothing** |

Three of those say **nothing**. That is the honest state, and it is why this page
exists rather than a promise that CI has it covered.

## The provider schema fixture

`tests/schemas/google.json` — 26 resource types, cut from the real provider.

The corpus and the smoke estate classify types through this fixture exactly the way
production classifies them through a real schema. It is not a stub: a type missing
from it silently loses schema-derived detail (it once lost every alert policy's
`notification_channels`), and a type whose real schema has moved on means the
snapshots pin yesterday's provider while claiming to pin today's.

```
uv run scripts/update_schema_fixture.py --check
uv run scripts/update_schema_fixture.py --add google_compute_firewall_policy_rule
```

`--check` downloads the pinned provider and reports, per fixture type, whether it is
gone or whether its attribute and block surface has drifted. **Run it when the
provider pin moves.** Nothing else will tell you.

The script never deletes, and the reason is worth keeping: a scan of the sources
gets this wrong in both directions. Three fixture types appear in no `.satz` file at
all because the *emitter* produces them from structural nodes —
`google_folder_iam_member` from a grant map inside a folder,
`google_cloud_identity_group_membership` from members, `google_logging_project_sink`
from a sink. And `google_compute_address` *is* referenced, inside a raw `hcl { }`
block, where no schema is needed. Trimming by what looks unused would delete three
live types and add one dead one.

## The Cloud Asset Inventory type list

`presets/cai-asset-types.txt` — 584 asset types, Google's list, one per line.

`satz import` can only discover a resource type that carries a CAI asset type, so
this list is the ceiling on discovery coverage. `scripts/update_import_config.py
--cai-types` fills the `asset_type` column of `presets/import-config.yaml` from it
(599 rows carry one today) — the column is never filled by hand.

**Refreshing it is manual, and the obvious shortcut does not work.** The source is
Google's asset-types documentation page, which paginates: a plain fetch and a regex
over the HTML yields about 85 of the 584 entries, so a scraper would appear to work
and silently truncate the list to a seventh of it. That is why there is no
`update_cai_asset_types.py` — copy the full list from the page (all sections
expanded), keep the three header comment lines, and re-run
`update_import_config.py --cai-types`.

Staleness shows up indirectly: rows in `import-config.yaml` whose `asset_type` stays
`TODO`/`UNKNOWN` when the type demonstrably has one. The smoke step *"import-config:
every derivable asset_type is filled"* fails when the config is behind the list — it
does **not** notice when the list itself is behind Google.

## The managed/legacy constraint pairing

`presets/managed-constraint-equivalents.txt` — 15 pairs Google declares, 1 we assert.

Where Google replaces a legacy org-policy constraint with a managed one, the packs
run the replacement alone and declare the twin off; see *Superseded legacy
constraints* in the [presets guide](../presets/README.md). The pairing is data
because a rule re-audited by hand is a rule enforced when someone remembers.

```
uv run scripts/update_constraint_equivalents.py
```

Needs ADC and a quota project. Nothing about the organisation reaches the file — the
constraint catalogue is Google's and identical for every customer. The section below
the `CURATED` marker is preserved: it holds pairs Google does not declare at all, and
there is exactly one, the pair that matters most.

The rule is gated offline by
`constraint_equivalents::no_pack_runs_a_superseded_constraint`, which fails when a
pack enforces a superseded constraint or enables a replacement without declaring its
twin off. Note what that does *not* cover: a twin Google added since the last
refresh is simply absent from the table, so the gate cannot know to look for it.
**Refresh the table when adding a constraint to a pack**, which is the moment the
question actually arises.

## The catalogs

`presets/catalogs/` — `cis-gcp-4.0.yaml`, `cis-gcp-5.0.yaml`, `iso27001-2022.yaml`.

Control ids, titles and paraphrases, transcribed from the published benchmark. There
is no machine-readable source to generate them from, so this is hand work, and it is
the item with the least safety net: a benchmark revision simply does not exist here
until someone adds it, and every report keeps answering the older question without
saying so.

Two lessons already paid for, both worth re-reading before transcribing anything:

- **Renumbering is the failure mode, not new controls.** CIS 5.0 renumbered
  1.1→1.2, 1.4→1.5, 1.5→1.6, 1.16→1.17, 3.8→3.10 while the content stayed put. A
  claim pointing at the old number is not a compile error, it is a wrong report.
- **Prefer a machine-readable third party over the PDF.** Prowler ships its CIS
  mappings as data, and checking against them corrected two numbers this project had
  already written down wrong (Cloud SQL public IP is 6.6 in 4.0 and 6.7 in 5.0, not
  6.5/6.7; the API-key constraint is 1.14/1.15, not 1.13).

An unclaimed control is visible as *unmet* rather than absent, so adding ids ahead of
the packs that implement them is safe and is the right order.

## The pack surface

- `presets/docs/*.md` (24 pages + the index) — generated by `satz doc-packs`, gated
  by `doc-packs --check` in the smoke matrix and by `cargo test`. Regenerate in the
  same commit as any pack change; CI fails otherwise. A **changelog row** is a pack
  change too: each page carries its pack's history, so editing the table below makes
  a page stale.
- `presets/README.md` `## Changelog` — one row per pack version, by hand.
  `doc-packs` parses it, so three things fail loudly rather than quietly: a pack
  version with no row, a row naming a pack that does not exist, and a table whose
  header or cell count has been reflowed.
- **Pack headers are an input, not decoration.** The index prints the first sentence
  of each header comment, so `doc-packs` refuses one that is empty, a bare URL, a
  leftover `Include …` instruction, or too long for a table cell. A pack whose shape
  does not decide how it is used — a bare list of labels with no claims — must state
  its own `use` line in the header; the error shows the line to add.

## Versions

- **Provider pin** (`provider_version`, currently 7.14.1, in each estate's
  `config.toml` and in `tests/smoke/config.toml`). A provider release is the trigger.
  Bumping it is cheap; the thing to remember afterwards is
  `update_schema_fixture.py --check`, because the fixture is the only place the tests
  see a schema at all.
- **Crates.** `cargo update` routinely; the tests are the safety net. The project's
  rule is the current version of everything, so a dependency held back needs a reason
  written next to it.

## Recurring reviews

- **Competitive re-audit** — quarterly or at a phase gate, appended as a dated entry
  to `docs/competitive.md`. Keeps the framework inputs, never replaces them.
- **Fleet re-transpile after every satz release** — outside this repository, but it
  belongs on the same list: an estate whose committed HCL no longer matches what the
  current binary emits is a release blocker, not a nuisance.

## The scripts

Everything in `scripts/` exists because it cannot be a satz command or a
preset. Two kinds live here: **cloud steps with no provider resource** (nothing
in Terraform can express them, so they stay `gcloud`), and **build-time helpers**
that maintain the repo's own data files.

A cloud step still cannot be **authored** in Satz — that is what makes it a
script. It can, since v0.46.69, be **declared and invoked** by an estate: an
[`action`](language.md#613-action-a-step-with-no-provider-resource) names
the step, binds it to a script, and builds its arguments from the estate's own
params, so `satz run-actions` runs it with the organisation id the estate
already knows instead of a human retyping it. Nothing runs at transpile time and
an action carries no claim; the script stays exactly as opaque to the compliance
plane as it is today.

**One of these files does not live in `scripts/`, and the reason is worth
stating.** `get-presets` downloads `presets/**` and nothing else, so a pack that
declares an action must carry its script inside `presets/` or ship an action
that cannot find what it runs. `scc-enable-all.sh` is therefore
`presets/scc/scc-enable-all.sh`, beside the pack that binds it. The rule that
follows: **a script a customer runs belongs under `presets/`; a script only this
repository runs belongs in `scripts/`.** Everything else on this page is the
second kind.

| script | kind | what it does |
|---|---|---|
| `presets/scc/scc-enable-all.sh` | cloud step | enable every SCC service at the org, inherit below. Under `presets/` so `get-presets` ships it and the SCC pack can bind it as an `action` |
| `update_import_config.py` | helper | keep `presets/import-config.yaml` current: new provider types, and `asset_type` filled from Google's Cloud Asset Inventory list |
| `smoke.sh` | gate | every estate-consuming command end to end against `tests/smoke/`; CI runs it on every push and PR |
| `inspect_schema.py` | helper | print one resource type's schema out of a provider schema dump |
| `build-satz-doc.py` | helper | render one `docs/*.md` as a self-contained, theme-aware HTML page (SVGs inlined) |
| `build-site.py` | build | render the documentation site (README, the `docs/*.md` named in `SITE_DOCS`, the presets docs) into `_site/` with a sticky navigation header, a per-page contents column and a client-side search over every page's headings and text (`search-index.js`, no external dependencies; `/` focuses the box). Publishing is explicit: a doc must be listed in `SITE_DOCS` or `SITE_DOCS_EXCLUDED` or the build fails naming it. `.github/workflows/pages.yml` publishes on GitHub Pages on every release tag and on demand |
| `check-names.sh` | gate | refuse any identifier that is not one of the example customers (`docs/examples.md`); judged per TOKEN (an allowed address never shields a private one beside it); CI on every push (`--commits A..B`, an unusable range is a failure, never a pass), `--staged` from the pre-commit hook, `--message FILE` from the commit-msg hook, `FILE…` for one file (missing file = failure) |

## `check-names.sh` — the privacy gate

This repository is public and the fleet it serves is not. The gate refuses any
identifier **shaped** like private data that is not one of the documented example
values in [`docs/examples.md`](examples.md) — directory ids, org/project/folder
numbers, billing accounts, GUIDs and their dashless 32-hex form, project ids,
e-mail addresses, domains that are neither IANA-reserved nor a known vendor host,
repository URLs and checkout paths — in files and in commit messages. It also
refuses the local files that must never be staged (`CLAUDE.local.md`,
`*.local.md`, `.claude/`, `attestations.yaml`, `evidence/`) and any commit whose
author or committer is not the maintainer's identity or a GitHub noreply address.

```bash
scripts/check-names.sh                  # the whole tree (CI)
scripts/check-names.sh --staged         # staged files + the identity about to commit
scripts/check-names.sh --commits A..B   # identities and messages of a commit range
scripts/check-names.sh --message FILE   # one commit message
scripts/check-names.sh FILE...          # specific files
```

It knows no customer, no company and no person: the allowlists are the example
values and the vendor defaults every customer shares, and everything else of that
shape is refused. **It judges tokens, not lines** — an allowed address never
shields a private one beside it — and an unusable commit range or a missing file
is a failure, never a pass.

What it **cannot** see is a NAME. A display name or a company in prose has no
shape, and "Log Admins" and a real customer's project name are the same kind of
string. That is what the local, never-committed denylist (`$NAMES_DENYLIST`) is
for, and what review is for.

Enable the hooks once per clone — they run the gate before a commit and again on
the message:

```bash
git config core.hooksPath .githooks
```

`.github/workflows/names-gate.yml` runs it on every push and pull request.

## `smoke.sh` — the command matrix

The unit tests cover the engines; nothing exercised the *commands* end to end, and
two regressions shipped with a green suite before this existed. `scripts/smoke.sh`
runs, offline, against the fixture estate in `tests/smoke/` (the shipped CIS,
contacts and monitoring packs, a group with a member, org grants, a project with
services and a bucket): `transpile` (then `tofu validate` when `tofu` is on PATH —
provider download only, no state, no cloud), `require`, `check-presets` against the
repository's own presets (must be clean), `import` in its state shape (with the
skipped report and import blocks), hcl shape (`--wrap-all`, then `tofu validate`) and yaml shape (including the refusal that names
a pack still in YAML), `adopt` (a table with ADC, a credentials error without —
never a guess), `scan` when `checkov` or `uvx` is on PATH, then `cargo test`.

```bash
scripts/smoke.sh                 # builds target/release/satz if missing
SATZ=~/.cargo/bin/satz scripts/smoke.sh
```

`.github/workflows/smoke.yml` runs it on every push and pull request with OpenTofu
installed. A new command that reads an estate gets a step here in the same PR.

## `build-site.py` — which docs become pages

Publishing a doc is a decision, and so is not publishing one. `build-site.py`
carries two lists and **fails naming any `docs/*.md` that is in neither**:

- `SITE_DOCS` — published.
- `SITE_DOCS_EXCLUDED` — deliberately not published, each with its reason.

A glob used to decide this, which meant a page appeared on the public site
because a file existed. The gate replaces "someone will notice" with a build
error, in both directions: a new doc cannot slip onto the site unreviewed, and
cannot be silently left off it either. The smoke matrix runs the site build, so
the check is enforced in CI.

**The menu is a third list, gated the same way.** `NAV_ORDER` names every page in
reading order — `satz`, `language`, `presets`, `workflows`, `mcp`, `examples`,
`housekeeping`, `competitive`, `llms` — and the build fails on a page it does not
name, or on a name that is not a page. It used to append an unlisted page
alphabetically, which is how a menu meant as a table of contents drifted into a
directory listing. A page's title is its menu word after `satz` (`# satz
language`, `# satz mcp`; the preset library is `# satz library`), with no trailing
explanation: what the page IS goes in its opening line, where a reader who opened
it will actually see it.

The 24 per-pack pages under `presets/docs/` are rendered and linked from the
preset library, but carry no menu entry of their own — someone looking for a pack
starts at the library.

An excluded doc stays in the repository and stays linkable — a link to one from a
published page is rewritten to its GitHub blob URL rather than left as a `.md`
href that 404s.

Currently excluded, and why:

| doc | why not published |
|---|---|
| `security-toolset-integration.md` | a proposal under rework; it describes an audit loop that is not what satz does today |
| `fast-delta.md` | source material for the competitive matrix, which carries the conclusions |
| `stage-b.md` | how the pipeline was built. The language reference is how it is used, and the migration commands are in the README |
| `interview-design.md` | a design sketch for a layer that is not built; it describes questions satz does not yet ask |

### One page at a time

`build-satz-doc.py` is the renderer `build-site.py` imports: it turns one
`docs/*.md` into a self-contained, theme-aware HTML page, inlining any SVG that
sits beside the markdown and recolouring it through CSS tokens so it follows the
viewer's theme. Run on its own it renders the language reference:

```bash
uv run --with markdown scripts/build-satz-doc.py [MD] [OUT.html] [TITLE]
```

### Navigating a long page

The reference pages are long on purpose — the language reference is meant to be
read straight through — so each page carries a **contents column** built from its
own `h2`/`h3` headings, sticky beside the text, marking the section you are in.
Below 1180px it becomes a collapsed block above the content. It is derived from
the rendered headings, so it cannot drift from the page it describes.

## `update_import_config.py` — keep the type table current

Two passes over `presets/import-config.yaml`, both from data, never by hand;
existing rows are never rewritten and comments survive (`ruamel.yaml`):

- `--schema-dir <dir>` reads every provider schema JSON there and adds a row
  for each resource type the table lacks (`import: false`,
  `asset_type: TODO/UNKNOWN`).
- `--cai-types presets/cai-asset-types.txt` resolves the `TODO/UNKNOWN` rows:
  the Cloud Asset Inventory name is derived from the Terraform type
  (`google_dns_managed_zone` → `dns.googleapis.com/ManagedZone`, with an alias
  table for the services whose provider name is not their API host) and kept
  ONLY when it is in Google's published list — `cai-asset-types.txt` is that
  list, dated in its header. Rows that are not Cloud Asset resources at all
  (IAM members/bindings, org-policy v1 shapes, provider constructs) lose the
  `asset_type` key: known, not unknown. What stays `TODO/UNKNOWN` is printed
  with what was tried. The smoke matrix runs this pass and fails when the
  table is behind the list.

```bash
uv run --with ruamel.yaml scripts/update_import_config.py \
  --config-file presets/import-config.yaml \
  --schema-dir <dir-of-schema-json> \
  --cai-types presets/cai-asset-types.txt
```

To refresh the list itself: the page
<https://docs.cloud.google.com/asset-inventory/docs/asset-types> renders its
table client-side, so copy the rendered text and keep one
`service.googleapis.com/Kind` per line; update the date in the header.

At the live shape an enabled row with `asset_type: TODO/UNKNOWN` (or an
unknown `content_type`) is a hard error; an enabled row with no `asset_type`
(a type Cloud Asset does not carry — Cloud Identity groups, say) is reported
once as "state shape only" and skipped there.

## `update_constraint_equivalents.py` — which managed constraint replaces which

Google publishes, per org-policy constraint, the constraint that replaces it
(`equivalentConstraint`). The packs act on that pairing — run the managed replacement
alone, declare the legacy twin off — so it lives in the repository as generated data:

```
uv run scripts/update_constraint_equivalents.py            # auto-detect org + quota project
uv run scripts/update_constraint_equivalents.py --org 123456789012
```

Writes `presets/managed-constraint-equivalents.txt`. Needs ADC **and** a quota project —
the OrgPolicy API refuses bare ADC. Nothing about the organisation reaches the file: the
constraint catalogue is Google's and identical for every customer.

Two things the script does that a one-liner would not:

- **Reads the equivalence in both directions.** The declaration is asymmetric — far more
  managed constraints name their legacy twin than the reverse — so reading one side finds
  a fraction of the pairs.
- **Preserves the curated section.** Below the `CURATED` marker sit pairs Google does not
  declare at all, with a note each saying why we assert them. Today that is
  `iam.allowedPolicyMemberDomains` ↔ `iam.managed.allowedPolicyMembers`: different names,
  no declared equivalence in either direction, same control. The generated section above
  the marker is rewritten wholesale and must never be hand-edited.

The table is enforced offline by `constraint_equivalents::no_pack_runs_a_superseded_constraint`
(`src/main.rs`), which compiles every corpus case and fails when a pack enforces a legacy
constraint that has a replacement, or enables a replacement without declaring its twin off.
So refreshing the table is a deliberate act needing credentials, while the rule it encodes
is checked on every `cargo test` with none.

## `update_schema_fixture.py` — keep the test schema honest

`tests/schemas/google.json` is a real provider schema trimmed to the types the
fixtures use, and the corpus classifies types through it the way production classifies
them through the real thing. A type missing from it silently loses schema-derived
detail; a type whose real schema has moved on means the snapshots pin yesterday's
provider.

```
uv run scripts/update_schema_fixture.py --check
uv run scripts/update_schema_fixture.py --add google_compute_firewall_policy_rule
```

`--check` downloads the pinned provider (from `provider_version` in
`tests/smoke/config.toml`) and reports each fixture type as GONE or DRIFTED, listing
the attributes and blocks that appeared or vanished. **Run it when the provider pin
moves** — nothing else notices. `--add` inserts or re-cuts types from the real schema
and leaves the rest byte for byte; regenerate the corpus afterwards.

It never deletes, deliberately. Three fixture types appear in no `.satz` source
because the emitter produces them from structural nodes, and one type that is
referenced needs no schema because it sits in a raw `hcl { }` block — trimming by
what looks unused would remove three live types and keep a dead one.

Needs `tofu` on PATH; talks to no organisation.

## `inspect_schema.py` — look at one type

Prints one resource type's schema out of a `terraform providers schema -json`
dump, and lists near-miss key names when the type is not found. Edit the `target`
variable to change which type it reports.

```bash
python3 scripts/inspect_schema.py <schema.json>
```

## `presets/scc/scc-enable-all.sh` — Security Command Center services

Turns every SCC service on at the organization and makes everything below the
organization inherit it.

```bash
presets/scc/scc-enable-all.sh --organization 123456789012            # dry run
presets/scc/scc-enable-all.sh --organization 123456789012 --apply    # write
```

**Or let the estate supply the organisation id.** `presets/scc/scc-service-enablement.satz`
is a pack whose entire content is an `action` binding this script — no resources,
because enablement is precisely the part that has none:

```
use "presets/scc/scc-service-enablement.satz"
```

```bash
satz run-actions estate.satz              # print the resolved command line, run nothing
satz run-actions estate.satz --check      # the dry run above
satz run-actions estate.satz --execute    # adds --apply
```

The script's flag shape is what makes that binding clean: `args` carries the form
that reads, `execute_args` the one flag that writes. The pack declares
`phase = "before-apply"`, because enablement is a prerequisite for the SCC
resources a later pack will declare.

### What is on by default, and what you have to ask for

Everything except three things. A detector for a workload nobody runs costs
nothing and finds nothing, so enabling Container Threat Detection before the first
GKE cluster exists is free — and it beats hoping somebody remembers to switch it
on the day the cluster appears. Being ready is the default.

Two services are different in kind, and one group is meaningless without a
connector:

| flag | what it adds | why it is not the default |
|---|---|---|
| `--with-optional` | `WEB_SECURITY_SCANNER` | it actively **crawls** the customer's web applications — a different consent from passive detection, and not one to give on their behalf |
| `--with-optional` | `ARTIFACT_ANALYSIS` | billed per image scan: a cost decision rather than a security one |
| `--with-multicloud` | the AWS/Azure connector services | they fail noisily until an AWS or Azure connector exists |

Without the flag those services are **left alone entirely** — not enabled at the
org, and not swept to `INHERITED` on descendants either. Sweeping them would mean
"take the org's value", and the org has no value for something we deliberately did
not decide, so the sweep would quietly disable a scanner somebody turned on for one
project on purpose.

### Why it is a script

google/google-beta carry **no binding** for
`securitycentermanagement.googleapis.com`'s `SecurityCenterService`. Turning
Security Health Analytics, Event Threat Detection, Container Threat Detection,
VM Threat Detection, Web Security Scanner or DSPM on or off therefore cannot be
written as a resource in any language satz compiles, and neither can tier
activation. Re-checked against **7.14.1**: the provider ships 35 `google_scc_*` /
`google_securityposture_*` types and none of them is service enablement or a
tier — the `google_scc_management_*` ones are custom modules only.

Everything **downstream** of activation is codeable and belongs in a preset:
custom modules (SHA + ETD), sources and source IAM, notification configs,
BigQuery exports, mute configs, and
`google_securityposture_posture(_deployment)`. Use the **v2** notification
resources: the v1 API answers `This API is no longer available. Please use API
V2` on a live org, so `google_scc_notification_config` is a dead end and
`google_scc_v2_organization_notification_config` is the one to write.

### It calls the API, not `gcloud scc manage`

The SDK knows **13** of the **17** services the API exposes. `ARTIFACT_GUARD`,
`ARTIFACT_ANALYSIS`, `AGENT_ENGINE_VULN_ASSESSMENT` and `EXTERNAL_EXPOSURE` have
no gcloud name at all — the CLI answers "is not a valid service name" — while the
API sets them without complaint. Four services silently out of reach is reason
enough on its own; going straight to the API also deletes a translation step that
had been a bug (the API says `SECURITY_HEALTH_ANALYTICS`, the CLI wanted
`security-health-analytics`, and discovery fed one to the other, so *every*
discovered run failed completely). gcloud still supplies the credentials and
walks the hierarchy.

The API needs a quota project — `--quota-project`, defaulting to the active
gcloud project — and refuses the call without one. `--apply` is the difference
between `validateOnly` and a write.

### What the live runs found (2026-09-04)

Until that day this script had only ever run against a gcloud test double, and
the double accepted everything. Against a real organization, on the gcloud path,
7 of 33 calls failed; on the API path 38 of 41 succeed, and the three that fail
are the organization's own state. What the runs turned up:

- **Four services were unreachable through the CLI** — the reason for the move
  above. All four now enable, verified end to end.
- **VM Manager cannot be enabled at all here.** The API answers `Invalid
  intended_enablement_state. ENABLED is not a valid enablement state`: SCC mirrors
  whether GCE's VM Manager is running. It is skipped in the org pass with a note
  and still swept to `INHERITED`, which the API does accept.
- **Security Health Analytics answers `FAILED_PRECONDITION`** at every level on an
  org where it is disabled and the subscription does not carry it. That is not a
  script bug and is left to surface with the API's own message rather than a
  guess at the cause.
- **`external-exposure` sat in the built-in fallback list** in a spelling nothing
  accepted, so the fallback path had been broken too, and silently. The list is
  now in the API's own names, which is what discovery returns.

### What it does

Two passes:

1. **org pass** — each service → `ENABLED` at `organizations/<id>`.
2. **descendant sweep** — each service → `INHERITED` at every folder and every
   project under the org.

The second pass is the point of the exercise. A descendant carrying its own
`ENABLED`/`DISABLED` keeps overriding the organization, so enabling at the org
alone does not make the org authoritative — it only makes it *first*. Sweeping
the hierarchy to `INHERITED` leaves exactly one place where the state is decided.
(`INHERITED` is only a legal state below the org, which is why the two passes set
different values.)

With `--reset-modules` each service's individual **modules** are set to
`INHERITED` as well, clearing per-module overrides on top of the per-service ones.

### Flags

| flag | effect |
|---|---|
| `--organization ID` | numeric org id (required); `organizations/123` is accepted too |
| `--apply` | actually write — without it every call carries `validateOnly` |
| `--services "a b c"` | use this service list verbatim instead of discovering it |
| `--with-optional` | also enable Web Security Scanner and Artifact Analysis |
| `--with-multicloud` | include the AWS/Azure connector services |
| `--quota-project ID` | project the API bills the call to; defaults to the active gcloud project |
| `--org-only` | enable at the org, skip the sweep |
| `--descendants-only` | only sweep folders/projects to `INHERITED` |
| `--reset-modules` | also set every module to `INHERITED` |
| `--targets-file FILE` | explicit `folders/<id>` / `projects/<id>` list instead of hierarchy discovery |

### Three things worth knowing before running it

**It is a dry run by default.** Every call carries `validateOnly` until
`--apply` is passed. This runs against customer orgs;
the safe direction is the default one.

**The service list is read from the org, not hardcoded.** It comes from the API's
own `securityCenterServices` listing, so a service Google adds later is picked up
without touching the script. The built-in list of 14 is only a fallback for when
that call cannot be made, and the AWS/Azure connectors in it are skipped unless
`--with-multicloud`, because they fail noisily on an org with no such connector.

**Activate the tier first.** The script cannot do it — there is no scriptable
surface for tier activation either — and enabling a service on an org without a
Premium/Enterprise subscription simply fails at the API.

### Failures it explains rather than echoes

Three failures are classified in the output instead of being left as a raw API
error:

- **`PERMISSION_DENIED`** — the caller lacks
  `securitycentermanagement.securityCenterServices.update` (`roles/securitycenter.admin`
  at the org, or the settings admin role).
- **service unavailable** — the org's SCC tier does not carry that service, or
  its API is off.
- **blocked by a constraint** — the CIS §1.1 locks. This is the one this fleet
  actually hits: each newly enabled service provisions another SCC service agent,
  and domain-restricted sharing refuses Google's auto-grant to it. The procedure
  for the lift window is in the SCC section of `presets/README.md`; the pack
  param is `allowed_policy_member_subjects`.

Exit status is non-zero if any call failed, so it can gate a runbook step.

### Requirements

`gcloud` and `jq` on `PATH`, and a shell — it is written for bash 3.2, the macOS
default, so no `mapfile` and no unguarded empty-array expansion.

## The shape of the rule

Anything derived gets, in order of preference:

1. a **script** in `scripts/`, so the refresh is reproducible and reviewable;
2. a **gate** that fails offline when the derived file and its consumers disagree;
3. failing both, a **line on this page** saying what goes stale and when.

Silence is the thing to avoid. A file that is merely out of date is a small problem;
a file that is out of date while the tests report success is how a compliance tool
starts lying.

