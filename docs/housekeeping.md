# satz housekeeping

A handful of files are **derived from something outside the repository** — Google's
provider schema, Google's constraint catalogue, Google's asset-type list, a CIS
benchmark release. When the source changes, these files go stale without any test
failing.

The first half of this page lists each of them: what refreshes it, what makes it
stale, and what detects it. The second half describes each script: the refreshes and the gates. Every script in
`scripts/` has a section, and so does the one script a pack binds as an `action`
(`presets/scc/scc-enable-all.sh`, which lives under `presets/` because `get-presets`
ships it).

## At a glance

| file | refreshed by | trigger | what catches staleness |
|---|---|---|---|
| `tests/schemas/google.json` | `scripts/update_schema_fixture.py --add` | the provider pin moves | **nothing** — run `--check` |
| `presets/cai-asset-types.txt` | by hand, from Google's docs | new asset types appear | indirect: unfilled `import-config` rows |
| `presets/import-config.yaml` (rows) | `scripts/update_import_config.py --config-file … --schema-dir … --provider-version …` | the provider pin moves | `cargo test`: `provider_version` must equal the pin |
| `presets/import-config.yaml` (`asset_type`) | `scripts/update_import_config.py --config-file … --cai-types <list>`, then `--probe <parent>` | the CAI list above changes; Google changes what ListAssets serves | smoke: *"every derivable asset_type is filled"*; a type ListAssets refuses: the live import aborts naming it |
| `presets/managed-constraint-equivalents.txt` | `scripts/update_constraint_equivalents.py` | Google ships a new managed twin | `cargo test` catches the *effect*, not the table |
| `presets/docs/*.md` | `satz doc-packs` | any pack changes | smoke: `doc-packs --check` |
| `presets/cis-extensions/*-dry-run.satz` | `scripts/build_dry_run_fragments.py` | the enforcing fragment it is derived from changes | smoke: `--check`, which compares byte for byte |
| `presets/README.md` (`## Changelog`) | by hand, one row per pack version | a pack version changes | `doc-packs --check`: fails on a version with no row |
| `presets/catalogs/*.yaml` | by hand, from the benchmark | a benchmark release | **nothing** |
| `src/iac_roles.rs` (the IaC role table) | by hand, from Google's predefined roles | a pack emits a new resource type; Google changes a role | a new type: `cargo test` (`iac_roles_gate`); a changed role: **nothing** — run `scripts/check_iac_roles.py` |
| `tests/corpus/*/expected.sorted.txt` | `UPDATE_CORPUS=1 cargo test` | emission changes | `cargo test` (that is the gate) |
| provider version pin | by hand | a provider release | **nothing** |
| crate versions | `cargo update` | routine | `cargo test` after the fact |
| `docs/competitive.md` | a battle review | quarterly, or a phase gate | **nothing** |

Four have **no** automatic check, and the IaC role table has none for a changed
role: refresh them on their trigger.

## The provider schema fixture

`tests/schemas/google.json` — 33 resource types, cut from the real provider.

The corpus and the smoke estate classify types through this fixture exactly the way
production classifies them through a real schema. A type missing from it loses
schema-derived detail without an error (an alert policy's `notification_channels`,
for example), and a type whose real schema has changed makes the snapshots pin an
older provider than the pin says.

```
uv run scripts/update_schema_fixture.py --check
uv run scripts/update_schema_fixture.py --add google_compute_firewall_policy_rule
```

`--check` downloads the pinned provider and reports, per fixture type, whether it is
gone or whether its attribute and block surface has drifted. **Run it when the
provider pin moves**; no other check reports it.

The script never deletes types, because a scan of the sources misjudges usage in both
directions. Three fixture types appear in no `.satz` file because the *emitter*
produces them from structural nodes —
`google_folder_iam_member` from a grant map inside a folder,
`google_cloud_identity_group_membership` from members, `google_logging_project_sink`
from a sink. And `google_compute_address` *is* referenced, inside a raw `hcl { }`
block, where no schema is needed. Trimming by apparent use would remove three types
that are used.

## The Cloud Asset Inventory type list

`presets/cai-asset-types.txt` — 584 asset types, Google's list, one per line.

`satz import` can only discover a resource type that carries a CAI asset type, so
this list is the ceiling on discovery coverage. `scripts/update_import_config.py
--cai-types` fills the `asset_type` column of `presets/import-config.yaml` from it
(452 of the 1283 rows carry one) — the column is never filled by hand.

**Refreshing it is manual.** Copy the full list from Google's asset-types
documentation page (all sections expanded), keep the four header comment lines,
and re-run `update_import_config.py --cai-types`. Scraping the page does not work:
it paginates, and a plain fetch returns about 85 of the 584 entries.

Staleness shows up indirectly: rows in `import-config.yaml` whose `asset_type` stays
`TODO`/`UNKNOWN` when the type demonstrably has one. The smoke step *"import-config:
every derivable asset_type is filled"* fails when the config is behind the list — it
does **not** notice when the list itself is behind Google.

## The managed/legacy constraint pairing

`presets/managed-constraint-equivalents.txt` — 15 pairs Google declares, 1 this
repository adds.

Where Google replaces a legacy org-policy constraint with a managed one, the packs
run the replacement alone and declare the twin off; see *Superseded legacy
constraints* in the [presets guide](../presets/README.md). The pairing is data,
generated from a live organisation, so a test checks the rule.

```
uv run scripts/update_constraint_equivalents.py
```

Needs ADC and a quota project. Nothing about the organisation reaches the file — the
constraint catalogue is Google's and identical for every customer. The section below
the `CURATED` marker is preserved: it holds the one pair Google does not declare.

The rule is gated offline by
`constraint_equivalents::no_pack_runs_a_superseded_constraint`, which fails when a
pack enforces a superseded constraint or enables a replacement without declaring its
twin off. A twin Google added after the last refresh is not in the table, so the gate
cannot check it: **refresh the table when adding a constraint to a pack**.

## The catalogs

`presets/catalogs/` — `cis-gcp-4.0.yaml`, `cis-gcp-5.0.yaml`, `iso27001-2022.yaml`.

Control ids, titles and paraphrases, transcribed from the published benchmark. There
is no machine-readable source to generate them from, so this is hand work with no
check: a benchmark revision is absent until someone adds it, and reports keep using
the older one without saying so.

Two rules for transcribing:

- **Check renumbered controls, not only new ones.** CIS 5.0 renumbered 1.1→1.2,
  1.4→1.5, 1.5→1.6, 1.16→1.17, 3.8→3.10 while the content stayed the same. A claim
  on the old number compiles and reports the wrong control.
- **Prefer a machine-readable third party over the PDF.** Prowler ships its CIS
  mappings as data; check the numbers against them (Cloud SQL public IP is 6.6 in 4.0
  and 6.7 in 5.0; the API-key constraint is 1.14/1.15).

An unclaimed control reads *unmet*, not absent, so catalog ids can be added before
the packs that implement them.

## The pack surface

- `presets/docs/*.md` (31 pages + the index) — generated by `satz doc-packs`, gated
  by `doc-packs --check` in the smoke matrix and by `cargo test`. Regenerate in the
  same commit as any pack change; CI fails otherwise. A **changelog row** is a pack
  change too: each page carries its pack's history, so editing the table below makes
  a page stale.
- `presets/README.md` `## Changelog` — one row per pack version, by hand.
  `doc-packs` parses it and fails on three things: a pack
  version with no row, a row naming a pack that does not exist, and a table whose
  header or cell count has been reflowed.
- **Pack headers are input to the index.** The index prints the first sentence
  of each header comment, so `doc-packs` refuses one that is empty, a bare URL, a
  leftover `Include …` instruction, or too long for a table cell. A pack whose shape
  does not decide how it is used — a bare list of labels with no claims — must state
  its own `use` line in the header; the error shows the line to add.

## The IaC role table

`src/iac_roles.rs` — per resource type, the permission the IaC service account needs
to manage it and the predefined roles that carry that permission, plus the reads every
estate needs. `satz iac-roles` checks an estate against it and `--execute` writes the
missing roles; `satz whoami <estate>` tests the permissions live. `satz iac-roles
--format json` prints it.

Two triggers make it stale:

- **A pack emits a type the table has no row for.** `iac_roles_gate` in `cargo test`
  compiles the cases under `tests/iac/` — together they use every pack, each one
  unconditionally — and fails on an emitted type without a row, and on a pack no case
  uses. A new pack gets a line in one of those cases; a new type gets its row.
- **Google changes a predefined role** — a permission renamed, or moved out of the role
  the table names. Nothing in the repository sees it. `scripts/check_iac_roles.py`
  reads every role the table names from the IAM API and fails when an entry's
  permission is in none of its roles: **run it when adding a row, and on each provider
  pin move.**

## Versions

- **Provider pin** (`provider_version`, currently 7.14.1, in each estate's
  `config.toml` and in `tests/smoke/config.toml`). A provider release is the trigger.
  After bumping it, run `update_schema_fixture.py --check`: the fixture is the only
  schema the tests see.
- **Crates.** `cargo update` routinely, then `cargo test`. Every dependency is on its
  current version; one held back carries its reason next to it.

## Recurring reviews

- **Competitive re-audit** — quarterly or at a phase gate, appended as a dated entry
  to `docs/competitive.md`. Keeps the framework inputs, never replaces them.
- **Fleet re-transpile after every satz release** — the estates live outside this
  repository; the check is in it: [`fleet-v1.sh`](#fleet-v1sh--every-estate-on-the-current-binary).
  An estate whose emitted HCL differs from what the current binary produces blocks the
  release. Run it after each release, so a difference points at that release.

## The scripts

Everything in `scripts/` exists because it cannot be a satz command or a
preset. Two kinds live here: **cloud steps with no provider resource** (nothing
in Terraform can express them, so they stay `gcloud`), and **build-time helpers**
that maintain the repo's own data files.

A cloud step cannot be **written** in Satz, so it is a script. An estate can
**declare and invoke** it: an
[`action`](language.md#613-action--a-step-with-no-provider-resource) names
the step, binds it to a script, and builds its arguments from the estate's own
params, so `satz run-actions` runs it with the organisation id the estate
already has. Nothing runs at transpile time, and an action carries no claim: the
compliance plane cannot see into the script.

**One of these files lives under `presets/`.** `get-presets` downloads
`presets/**` and nothing else, so a pack that
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
| `fleet-v1.sh` | gate | every estate you operate, re-transpiled on the current binary and compared block by block against what it emitted before. Not run by CI — CI has no estates. Run it after every release |
| `update_constraint_equivalents.py` | helper | refresh `presets/managed-constraint-equivalents.txt` — which managed constraint replaces which legacy one — from a live organisation's `ListConstraints` |
| `update_schema_fixture.py` | helper | keep `tests/schemas/google.json` in step with the types the packs emit; `--check` names what is missing |
| `inspect_schema.py` | helper | print one resource type's schema out of a provider schema dump |
| `build_dry_run_fragments.py` | helper | generate each CIS extension's dry-run twin from the enforcing fragment: `spec` becomes `dry_run_spec`, every claim is dropped. `--check` fails on a stale twin |
| `check_iac_roles.py` | gate | hold the IaC role table (`src/iac_roles.rs`) against Google's predefined role definitions; needs ADC, not run by CI |
| `build-satz-doc.py` | helper | render one `docs/*.md` as a self-contained, theme-aware HTML page (SVGs inlined) |
| `build-site.py` | build | render the documentation site (README, the `docs/*.md` named in `SITE_DOCS`, the presets docs) into `_site/` with a sticky navigation header, a per-page contents column and a client-side search over every page's headings and text (`search-index.js`, no external dependencies; `/` focuses the box). Publishing is explicit: a doc must be listed in `SITE_DOCS` or `SITE_DOCS_EXCLUDED` or the build fails naming it. `.github/workflows/pages.yml` publishes on GitHub Pages on every release tag and on demand |
| `check-names.sh` | gate | refuse any identifier that is not one of the example customers (`docs/examples.md`); judged per TOKEN (an allowed address never shields a private one beside it); CI on every push (`--commits A..B`, an unusable range is a failure, never a pass), `--staged` from the pre-commit hook, `--message FILE` from the commit-msg hook, `FILE…` for one file (missing file = failure) |

## `check-names.sh` — the privacy gate

This repository is public; the estates it serves are private. The gate refuses any
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
is a failure, never a pass. It runs under bash 3.2 (`/bin/bash` on macOS) as well
as bash 5; the smoke matrix runs its identifier check under both.

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

The unit tests cover the engines; `scripts/smoke.sh` covers the *commands*, end to
end. It runs, offline, against the fixture estate in `tests/smoke/` (the shipped CIS,
contacts and monitoring packs, a group with a member, org grants, a project with
services and a bucket): `transpile` (then `tofu validate` when `tofu` is on PATH —
provider download only, no state, no cloud), `require`, `check-presets` against the
repository's own presets (must be clean), `import` in its state shape (with the
skipped report and import blocks), hcl shape (`--wrap-all`, then `tofu validate`)
and yaml shape (including the refusal that names a pack still in YAML), `adopt` (a
table with ADC, a credentials error without), `scan` when `checkov` or `uvx` is on
PATH, then `cargo test`.

```bash
scripts/smoke.sh                 # builds target/release/satz first
SATZ=~/.cargo/bin/satz scripts/smoke.sh   # uses that binary, never rebuilt
```

`.github/workflows/smoke.yml` runs it on every push and pull request with OpenTofu
installed. A new command that reads an estate gets a step here in the same PR.

## `fleet-v1.sh` — every estate on the current binary

The script answers: **does every estate you operate still compile on the current
binary, and does it emit the same infrastructure?** The corpus tests the compiler
against fixtures, the smoke matrix tests the commands against one estate, and
`tofu plan` tests one estate against its own organisation with that estate's
credentials; none of them re-transpiles the fleet.

```bash
scripts/fleet-v1.sh ~/estates/acme            # one estate
scripts/fleet-v1.sh --roster ~/fleet.tsv      # the fleet
scripts/fleet-v1.sh --roster ~/fleet.tsv -v   # ... with the diffs
```

### The roster

Which estates exist, and where, is **not in this repository**; it is operator
state. The script takes it as an
argument (or `$FLEET_ROSTER`) in either of two shapes:

```
# code<TAB>path, comments allowed
E01     ~/estates/acme
E02     ~/estates/bolt
```

or a markdown table, so a fleet note you already keep works unchanged — the code
is the **first** cell and the checkout is the first backticked absolute-or-`~`
path, which may carry a note after it:

```
| E01 | Customer A | `~/estates/acme` (also elsewhere) | … |
```

A row without both is skipped as prose. The parser is strict: a lenient one would
read ordinary prose and unrelated tables as estates, report each as UNAVAILABLE,
and hide the estates that really were not checked.

Bare paths on the command line need no roster at all.

### Reading the result

| outcome | what it means | what to do |
|---|---|---|
| **clean** | the address set is identical and no block body moved | nothing |
| **delta** | same address set, some block body differs | not a release blocker. Carry it into the estate's next pickup and commit the re-transpiled `hcl/` |
| **BLOCKER** | the estate does not transpile, or its **address set moved** — a resource appeared or disappeared | stop the release until the change is explained |
| **UNAVAILABLE** | no checkout, no `config.toml`, no `hcl/` to compare against — or no provider schema, which estate repos gitignore as a derived cache, so a fresh clone has none | run it where the checkouts are, or `satz update-schema` in that one. `--require-all` turns this into a failure |

Exit codes follow: `0` clean, `1` blocker (or `--require-all` with an unchecked
estate), `2` delta only.

**A hand-written `.tf` in `hcl/` is listed, not compared.** The comparison set is what
satz *emits*; a file this emission did not produce is listed with `!` and its blocks
are left out of the count. An estate may keep one: a write-only secret cannot come
from the estate, so the `variable` block declaring it lives in `hcl/` beside the
generated files. Compared, its blocks would read as deletions. The script cannot tell
a hand-written file from one an older satz emitted, so it names the file; check a
named file that is not hand-written.

**A subdirectory in `hcl/` is the same thing one level up.** satz emits flat files into
`hcl/` and never below it, so a `modules/` or `landing-zones/` tree an estate keeps
there is its own, and the `main.tf` inside is not the emitted `main.tf` that shares the
name. The walk is flat: a subdirectory is listed with `!` and nothing in it is
compared. Walked, such a tree would read as deletions on an estate whose emission
has not changed at all.

**Why a moved address blocks.** A body delta renders the same resources differently.
A moved address means the next plan creates or destroys a resource.

### When the delta is the state backend

A delta in the `terraform` block's backend is a changed state backend, and `tofu`
refuses the next command until it is re-initialised, once per estate:

```bash
cd hcl/ && tofu init -reconfigure
```

Local-mode estates need nothing, and `satz migrate --mode cloud` re-initialises
on its own.

### What it does not do

- **It never writes into a checkout.** Every estate is copied to a scratch
  directory and transpiled there, because estate repositories carry work in
  progress. Pass `--scratch DIR` to keep the emitted trees.
- **It does not diff text.** `git diff hcl/` also reports a block that only moved.
  Blocks are matched by **address**, so content counts and order does not.
- **It does not treat a missing checkout as a pass**; it reports it as UNAVAILABLE.
- **It reads no cloud.** It compares what satz *emits*. What the organisation has is
  `report-compliance`, with that estate's credentials.

## `build-site.py` — which docs become pages

`build-site.py` publishes the docs it names in one list and skips those in another,
and **fails naming any `docs/*.md` that is in neither**:

- `SITE_DOCS` — published.
- `SITE_DOCS_EXCLUDED` — deliberately not published, each with its reason.

The gate is a build error in both directions: a new doc cannot slip onto the site
unreviewed, and cannot be silently left off it either. The smoke matrix runs the
site build, so the check is enforced in CI.

**The menu is a third list, gated the same way.** `NAV_ORDER` names every page in
reading order — `satz`, `language`, `library`, `workflows`, `interview`, `mcp`,
`examples`, `housekeeping`, `competitive`, `llms` — and the build fails on a page
it does not name, or on a name that is not a page, so the menu stays a table of
contents rather than a directory listing. A page's title is its menu word after
`satz` (`# satz language`, `# satz library`), with no trailing explanation; what the
page is goes in its opening line. The browser tab shows that title as plain text,
taken from the rendered heading, and a page without a `# ` title fails the build.

The 24 per-pack pages under `presets/docs/` are rendered and linked from the
preset library, but carry no menu entry of their own — someone looking for a pack
starts at the library.

An excluded doc stays in the repository and stays linkable — a link to one from a
published page is rewritten to its GitHub blob URL rather than left as a `.md`
href that 404s. So is a link to any other repository file the site does not
publish, an ADR under `docs/adr/` for one; a link to a file that does not exist
fails the build, naming the page and the link.

Excluded, and why:

| doc | why not published |
|---|---|
| `security-toolset-integration.md` | a proposal under rework; it describes an audit loop satz does not implement |
| `fast-delta.md` | source material for the competitive matrix, which carries the conclusions |
| `stage-b.md` | how the pipeline was built. The language reference is how it is used, and the migration commands are in the README |

### One page at a time

`build-satz-doc.py` is the renderer `build-site.py` imports. It turns one
`docs/*.md` into a self-contained, theme-aware HTML page, inlining any SVG that sits
beside the markdown and recolouring it through CSS tokens so it follows the viewer's
theme. It parses with cmark-gfm, GitHub's own parser
([ADR 0008](adr/0008-the-site-renders-markdown-with-githubs-parser.md)), so a page
shows what GitHub shows and every heading carries the anchor GitHub gives it. Both
scripts declare that dependency themselves, so a bare `uv run` runs them. A link to an
anchor its target page does not carry fails the site build.

Inline code never pushes a page or a column wide. In a table cell it breaks between
its words and never inside one, so a long command cannot claim its column's whole
width; in running text a word of up to 27 characters is kept whole as well, so
`--help` never ends a line as `--`. A word too long to share a table row with two
others, in a cell or in running text, may also break before a `/`, `.` or `_`, and a
piece of it that is still that long where a lowercase letter meets a capital. In a
heading every word may break before a `/`, `.` or `_`, because a pack's name at
heading size does not fit a phone line. Whatever still cannot fit a line breaks where
it must; a code block scrolls inside itself instead.

Run on its own, it renders the language reference:

```bash
uv run scripts/build-satz-doc.py [MD] [OUT.html] [TITLE]
```

### Navigating a long page

Each page carries a **contents column** built from its own `h2`/`h3` headings, sticky
beside the text, marking the section in view. Below 1180px it becomes a collapsed
block above the content. It is built from the rendered headings.

## `update_import_config.py` — keep the type table current

Three passes over `presets/import-config.yaml`, all from data, never by hand;
comments and row order survive (`ruamel.yaml`):

- `--schema-dir <dir> --provider-version <v>` makes the rows the provider's
  resource types: it reads every provider schema JSON there (google and
  google-beta), adds a row for each type the table lacks (`import: false`,
  `asset_type: TODO/UNKNOWN`), removes the rows no schema knows, and records `v`
  as the table's `provider_version`. `cargo test` fails when that differs from
  the pin in `tests/smoke/config.toml`. The schemas come from
  `tofu providers schema -json > <dir>/schemas.json`, run in a directory
  initialized with both providers at the pinned version.
- `--cai-types presets/cai-asset-types.txt` resolves the `TODO/UNKNOWN` rows:
  the Cloud Asset Inventory name is derived from the Terraform type
  (`google_dns_managed_zone` → `dns.googleapis.com/ManagedZone`, with an alias
  table for the services whose provider name is not their API host) and kept
  ONLY when it is in Google's published list — `cai-asset-types.txt` is that
  list, dated in its header. Rows that are not Cloud Asset resources at all
  (IAM members/bindings, org-policy v1 shapes, provider constructs, and
  `google_billing_budget`, which `adopt` resolves through the Billing API) lose
  the `asset_type` key: known, not unknown. What stays `TODO/UNKNOWN` is printed
  with what was tried. The smoke matrix runs this pass and fails when the
  table is behind the list.
- `--probe <parent>` asks Cloud Asset Inventory for one page of every named
  row's asset type under `<parent>` (`organizations/<n>`, `folders/<n>`,
  `projects/<id>`), with the Application Default Credentials through gcloud and
  80 requests a minute (`--probe-rate`), below the ListAssets quota. Google's
  list names types ListAssets refuses at the row's `content_type` — the Access
  Context Manager, Cloud Identity, Cloud SQL user and database types among
  them; the refusal names the type and points at Google's supported-asset-types
  page, while a type the scope merely has none of answers with an empty page.
  A refused row loses its `asset_type`, gets a comment naming the refusal, and
  is state shape only. Any other error ends the run. It needs a live
  organization, so no gate runs it; run it after `--cai-types`.

```bash
uv run --with ruamel.yaml scripts/update_import_config.py \
  --config-file presets/import-config.yaml \
  --schema-dir <dir-of-schema-json> --provider-version <pinned version> \
  --cai-types presets/cai-asset-types.txt
uv run --with ruamel.yaml scripts/update_import_config.py \
  --config-file presets/import-config.yaml --probe organizations/<n>
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

The script:

- **Reads the equivalence in both directions.** The declaration is asymmetric — far more
  managed constraints name their legacy twin than the reverse — so reading one side finds
  a fraction of the pairs.
- **Preserves the curated section.** Below the `CURATED` marker sit pairs Google does not
  declare, each with a note saying why it is asserted. The one such pair is
  `iam.allowedPolicyMemberDomains` ↔ `iam.managed.allowedPolicyMembers`: different names,
  no declared equivalence in either direction, the same control. The generated section
  above the marker is rewritten wholesale; do not edit it by hand.

The table is enforced offline by `constraint_equivalents::no_pack_runs_a_superseded_constraint`
(`src/main.rs`), which compiles every corpus case and fails when a pack enforces a legacy
constraint that has a replacement, or enables a replacement without declaring its twin off.
Refreshing the table needs credentials; the rule is checked on every `cargo test`
without any.

## `update_schema_fixture.py` — keep the test schema current

`tests/schemas/google.json` is a real provider schema trimmed to the types the
fixtures use, and the corpus classifies types through it as production does through
the full schema. A type missing from it loses schema-derived detail without an error;
a type whose real schema has changed makes the snapshots pin an older provider than
the pin says.

```
uv run scripts/update_schema_fixture.py --check
uv run scripts/update_schema_fixture.py --add google_compute_firewall_policy_rule
```

`--check` downloads the pinned provider (from `provider_version` in
`tests/smoke/config.toml`) and reports each fixture type as GONE or DRIFTED, listing
the attributes and blocks that appeared or vanished. **Run it when the provider pin
moves**; no other check does. `--add` inserts or re-cuts types from the real schema
and leaves the rest byte for byte; regenerate the corpus afterwards.

It never deletes: three fixture types appear in no `.satz` source because the
emitter produces them from structural nodes, and one referenced type needs no schema
because it sits in a raw `hcl { }` block, so usage in the sources does not show which
types are needed.

Needs `tofu` on PATH; talks to no organisation.

## `build_dry_run_fragments.py` — the dry-run twins

An org policy can carry `dry_run_spec` instead of `spec`. Google evaluates every rule,
writes a violation to the audit log for each action it WOULD have blocked, and blocks
nothing — so a breaking control is sized against a live organisation before it is
enforced.

The twin must measure exactly the policy that is later enforced, or its numbers answer a
question nobody asked. So it is derived, not written:

```bash
uv run scripts/build_dry_run_fragments.py            # write the twins
uv run scripts/build_dry_run_fragments.py --check    # fail if any is stale
```

`DRY_RUNNABLE` at the top of the script is the list, with the param that gates each
twin. A fragment reaches it when its constraint supports a dry run; five extensions do
not, each for a stated reason in that table — the legacy constraints (Shielded VM, both
CMEK list constraints) have no dry-run form, Access Approval is not an org policy, and
the two on-by-default extensions have nothing to size.

Three things the derivation does:

- **`spec` becomes `dry_run_spec`** — except a `spec { reset = true }`, which declares a
  superseded legacy twin OFF. That is not a control being enforced and there is nothing
  to size about it, so it stays as it is.
- **Every `claim` is dropped.** A dry run discharges nothing while it measures, so
  `require` reports the control unmet, which is the truth. A claim over a dry-run policy
  would be contradicted by its own witness ([ADR 0013](adr/0013-a-claim-asserts-what-its-witness-does.md)).
- **The pack name gains `_dry_run`** and its version tracks the fragment it came from.

Read the violations in Cloud Logging:

```
protoPayload.metadata."@type"="type.googleapis.com/google.cloud.audit.OrgPolicyViolationInfo"
```

Promote by switching the dry-run param off and the enforcing param on. Both on at once is
refused, naming both params and what each choice means.

## `check_iac_roles.py` — the role table against Google's roles

The IaC role table names, per resource type, a permission and the predefined roles
that carry it. The script reads the table from `satz iac-roles --format json` (of this
checkout through `cargo run`, or of an installed binary with `--satz`), fetches each
role it names from the IAM API (`roles.get`), and fails when none of an entry's roles
carries the entry's permission: a typo in the table, a permission Google renamed, or a
role Google narrowed.

```bash
uv run scripts/check_iac_roles.py                 # the table of this checkout
uv run scripts/check_iac_roles.py --satz satz     # the table of an installed binary
```

Needs Application Default Credentials; predefined roles are Google's and the same for
every organisation, so any credential that may call the IAM API will do. Workspace
entries are not IAM roles and are skipped. Run it when adding a row and on each
provider pin move; CI has no credentials to run it.

## `build-satz-doc.py` — one page, self-contained

`build-site.py` renders the whole site; this renders ONE `docs/*.md` as a single
theme-aware HTML file with its SVGs inlined, for sending a page to someone who will
open it from disk rather than from the site:

```bash
uv run scripts/build-satz-doc.py docs/language.md out/language.html ["Page title"]
```

Both paths are optional: with none it renders `docs/language.md` beside itself.
Nothing gates it — the page it writes is not published, and the site build does not
read its output.

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
is a pack whose only content is an `action` binding this script, since service
enablement has no provider resource:

```
use "presets/scc/scc-service-enablement.satz"
```

```bash
satz run-actions estate.satz              # print the resolved command line, run nothing
satz run-actions estate.satz --check      # the dry run above
satz run-actions estate.satz --execute    # adds --apply
```

`args` carries the reading form of the command, `execute_args` the one flag that
writes. The pack declares
`phase = "before-apply"`, because enablement is a prerequisite for the SCC
resources a later pack will declare.

### Defaults

Every service is enabled except the ones below. A detector for a workload that does
not exist yet costs nothing, so Container Threat Detection is on before the first GKE
cluster exists.

These are opt-in:

| flag | what it adds | why it is not the default |
|---|---|---|
| `--with-optional` | `WEB_SECURITY_SCANNER` | it actively **crawls** the customer's web applications, which needs the customer's consent |
| `--with-optional` | `ARTIFACT_ANALYSIS` | billed per image scan |
| `--with-multicloud` | the AWS/Azure connector services | they fail until an AWS or Azure connector exists |

Without the flag those services are **left alone**: not enabled at the org, and not
swept to `INHERITED` below it. Sweeping them would make descendants take the org's
value, which is unset, and switch off a scanner enabled on one project.

### Why it is a script

google/google-beta carry **no binding** for
`securitycentermanagement.googleapis.com`'s `SecurityCenterService`. Turning
Security Health Analytics, Event Threat Detection, Container Threat Detection,
VM Threat Detection, Web Security Scanner or DSPM on or off therefore cannot be
written as a resource in any language satz compiles, and neither can tier
activation. Provider **7.14.1** ships 35 `google_scc_*` / `google_securityposture_*`
types and none of them is service enablement or a tier — the
`google_scc_management_*` ones are custom modules only.

Everything **downstream** of activation is codeable and belongs in a preset:
custom modules (SHA + ETD), sources and source IAM, notification configs,
BigQuery exports, mute configs, and
`google_securityposture_posture(_deployment)`. Use the **v2** notification
resources: the v1 API answers `This API is no longer available. Please use API
V2` on a live org, so `google_scc_notification_config` does not work;
`google_scc_v2_organization_notification_config` is the one to write.

### It calls the API, not `gcloud scc manage`

The SDK knows **13** of the **17** services the API exposes. `ARTIFACT_GUARD`,
`ARTIFACT_ANALYSIS`, `AGENT_ENGINE_VULN_ASSESSMENT` and `EXTERNAL_EXPOSURE` have
no gcloud name at all — the CLI answers "is not a valid service name" — while the
API sets them. The script therefore calls the API, which also needs no translation
between its names and the CLI's (`SECURITY_HEALTH_ANALYTICS` against
`security-health-analytics`); discovery returns the API's form. gcloud supplies the
credentials and walks the hierarchy.

The API needs a quota project — `--quota-project`, defaulting to the active
gcloud project — and refuses the call without one. `--apply` is the difference
between `validateOnly` and a write.

### What a live organization answers

Against a real organization, 38 of 41 calls succeed; the three that fail are the
organization's own state:

- **The four services the CLI cannot name enable through the API**, verified end
  to end.
- **VM Manager cannot be enabled at all here.** The API answers `Invalid
  intended_enablement_state. ENABLED is not a valid enablement state`: SCC mirrors
  whether GCE's VM Manager is running. It is skipped in the org pass with a note
  and still swept to `INHERITED`, which the API does accept.
- **Security Health Analytics answers `FAILED_PRECONDITION`** at every level on an
  org where it is disabled and the subscription does not carry it. The script passes
  the API's message through.
- **The built-in fallback list is in the API's own names**, which is what discovery
  returns; a CLI spelling there would break the fallback path silently.

### What it does

Two passes:

1. **org pass** — each service → `ENABLED` at `organizations/<id>`.
2. **descendant sweep** — each service → `INHERITED` at every folder and every
   project under the org.

The second pass makes the organization the one place the state is set: a descendant
with its own `ENABLED`/`DISABLED` overrides the organization, so enabling at the org
alone does not reach it. (`INHERITED` is only a legal state below the org, which is
why the two passes set different values.)

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

### Before running it

**It is a dry run by default.** Every call carries `validateOnly` until `--apply` is
passed.

**The service list is read from the org, not hardcoded.** It comes from the API's
own `securityCenterServices` listing, so a service Google adds later is picked up
without touching the script. The built-in list of 14 is only a fallback for when
that call cannot be made, and the AWS/Azure connectors in it are skipped unless
`--with-multicloud`, because they fail on an org with no such connector.

**Activate the SCC tier first.** Tier activation has no API the script could call,
and enabling a service on an org on the Standard tier fails
at the API.

### Failures it classifies

Three failures are classified in the output instead of being left as a raw API
error:

- **`PERMISSION_DENIED`** — the caller lacks
  `securitycentermanagement.securityCenterServices.update` (`roles/securitycenter.admin`
  at the org, or the settings admin role).
- **service unavailable** — the org's SCC tier does not carry that service, or
  its API is off.
- **blocked by a constraint** — the CIS §1.1 lock refused Google's role grant to
  a service agent that `allowed_policy_member_subjects` does not name. The pack's
  default names SCC's five agents; the SCC section of `presets/README.md` lists
  them and says how to allow another.

Exit status is non-zero if any call failed, so it can gate a runbook step.

### Requirements

`gcloud` and `jq` on `PATH`, and a shell — it is written for bash 3.2, the macOS
default, so no `mapfile` and no unguarded empty-array expansion.

## The shape of the rule

Anything derived gets, in order of preference:

1. a **script** in `scripts/`, so the refresh is reproducible and reviewable;
2. a **gate** that fails offline when the derived file and its consumers disagree;
3. failing both, a **line on this page** saying what goes stale and when.

A derived file with none of the three can go stale while every test passes.

