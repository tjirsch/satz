# satz — working rules

Rust tool (bin `satz`, crate `crates/satz-core`) that compiles estates written in **Satz** — a language whose
resource types and attributes are the Terraform provider's, to the underscore —
to OpenTofu/Terraform HCL, with a compliance plane on top: claims → catalogs →
`require` (goal view, declared) → `report-compliance` (evidence, verified live,
by value). `docs/satz-language.md` is the language reference;
`docs/security-toolset-integration.md` is the proposal for the audit loop.

This file holds the rules that apply to every contributor. The maintainer's
private working state (fleet, customers, runbooks, task list) lives OUTSIDE
this repository — in `CLAUDE.local.md`, which is git-ignored and gate-rejected
and is a SYMLINK into the maintainer's private notes so every machine reads
the same file (`ln -sfn <private-notes>/satz-private/CLAUDE.local.md
CLAUDE.local.md` in the checkout; the exact command is in that file's header),
and in the maintainer's notes. Nothing in this file names a customer.

## Rules

- **Privacy gate, every commit (2026-08-28).** This repository is public. No
  customer, company or person is named here, and the gate does not need to
  know any: `scripts/check-names.sh` rejects anything SHAPED like private data
  that is not a documented example value (`docs/example-customers.md`) — `C0…`
  directory ids, 11–13-digit org/project/folder numbers, billing accounts,
  e-mail addresses, domains that are neither IANA-reserved nor a known vendor
  host, repository URLs and checkout paths — in files and in commit messages;
  it rejects local files (`CLAUDE.local.md`, `*.local.md`, `.claude/`,
  `attestations.yaml`, `evidence/`) if they are ever staged; and it rejects
  any commit whose author or committer is not the maintainer's private
  identity or a GitHub noreply address. CI runs it on every push and PR
  (`.github/workflows/names-gate.yml`), beside the smoke matrix
  (`scripts/smoke.sh`, `.github/workflows/smoke.yml`: every estate-consuming
  command end to end against `tests/smoke/` — a new command gets a step there
  in the same PR — plus a `checks` job: `clippy -D warnings` and
  `cargo test --workspace --locked`); the pre-commit and commit-msg hooks run
  the gate locally — enable once per clone with
  `git config core.hooksPath .githooks`. The gate judges tokens, not lines,
  and an unusable range or a missing file is a failure, never a pass. Examples
  use ONLY the four example customers. If an example needs a value the table
  does not have, add it to the table in the same commit.
- **Release flow:** work commits on `main`; `cargo release patch|minor
  --execute --no-confirm` bumps, tags and pushes; the tag triggers cargo-dist.
  Release without asking when a discussed solution is releasable (tests +
  clippy green, docs updated). After the release, stop — no polling of the
  GitHub API (unauthenticated: 60 req/h, shared with users' `self-update`).
- **Presets, provenance by suffix:** `X.satz` pristine, upstream-owned, always
  overwritable / `X.local.satz` the user's fork, never touched by updates /
  `X.diff.satz` the current adoption delta, rewritten on every merge. A preset
  an estate INCLUDES never changes silently: a semantic upstream change
  (compiled canonical form differs) auto-forks and repoints the estate with a
  transpile-identity proof; comment/format churn upgrades silently. Pack
  versions live IN-FILE; filenames carry only framework versions. Never
  `.local.<n>.satz`.
- **Memberships stay OUT of presets** — presets define groups, humans grant
  membership.
- **80% of customisation via params, the rest via `.local` forks** — no
  variable explosion. Names that must be globally unique derive from
  `customer_shortname`.
- **Everyone is on the CURRENT version.** A release IS the migration: a
  breaking language change ships together with the estate edits that satisfy
  it. No deprecation periods, no dual-accept paths for old binaries.
- **YAML exists only to be migrated (2026-08-29).** `satz import <file>.yaml` must keep
  converting old estates and packs for as long as legacy orgs exist; that is the whole YAML surface. No new functionality grows a YAML arm, YAML is never
  generated, and a YAML code path that a cleanup breaks is deleted, not
  repaired — the legacy walk, the `.gen.yaml` twin and every YAML command arm
  are gone since v0.46.14. A conversion is reported as NEEDS-REVIEW where it
  cannot be proven; migrated estates may need manual edits (an old
  `!import-include` becomes `use` plus `satz adopt`).
  `tests/corpus/yaml-estate/` is the converter's gate: a YAML fixture through
  the converter must compile as Satz and emit the expected resource set.
- **`cargo test` does NOT rebuild the debug binary** — `cargo build` before a
  live test, or a stale binary shadows the fix. Same family: an edit to
  `crates/satz-core/` was once not picked up — `touch` the file and confirm
  "Compiling satz-core" before trusting a live run.
- **Corpus (`tests/corpus/`) is snapshot-gated:** `UPDATE_CORPUS=1` + review
  the diff.
- **Docs ship with the change, in two renderings.** A change to the language,
  a command, a flag or a pack updates `README.md`, `docs/*.md` and
  `presets/README.md` in the same PR (the language reference cites
  `tests/smoke/yaml/showcase.satz` — a new feature gets its example there
  first). The HTML site <https://tjirsch.github.io/satz/> is rendered from
  exactly those files by `scripts/build-site.py` and published by
  `.github/workflows/pages.yml` on every release tag; nothing is written for
  the site separately, and a doc that is not in the repo is not documentation.
  Packs: `presets/docs/` is generated by `satz doc-packs` — a change to a pack
  regenerates it (`--check` is a smoke gate), and a version bump adds a row to
  `presets/CHANGELOG.md` (the smoke matrix fails without one).
  Run it locally before a docs PR: `uv run --with markdown
  scripts/build-site.py _site` and open `_site/index.html`. `satz open-readme`
  and the post-install step open that site.
- **Docs are derived from the parser, not from intent.** Every example in
  `docs/satz-language.md` compiles; where the doc and the parser disagree, the
  parser is right and the doc is a bug.

## Language state (v0, satz v0.46.48)

- The fragment pipeline parses Satz directly: per-file fragments, the ⊕ fold
  (same address, different body = hard error naming both files), schema-typed
  resources (an unknown block key is a parse-time error), `use` / `use … as` /
  `use … when`, `suppress` (subtractive; a suppression that matches nothing is
  a hard error), `claim` with `implements` / `contributes` / `deviates`
  (`reason` mandatory on a deviation), `hcl { … }` raw passthrough that warns
  on every transpile unless `hcl trust "…"` and is never a witness (the
  compliance plane and `adopt-org-policies` read the emission manifest,
  `src/manifest.rs`, not `main.tf` text).
- Adoption: `satz adopt` resolves live ids over the emission manifest (natural-key
  lookups for folders/groups/memberships/org policies, `import_id`/`match_on`
  rules per type in `presets/import-config.yaml` for the rest — its
  `asset_type` column is filled from `presets/cai-asset-types.txt` by
  `scripts/update_import_config.py --cai-types`, never by hand; the smoke
  matrix fails when the table is behind the list), never
  guesses (one candidate resolves, many is ambiguous), and `--execute` writes
  verified ids back as `"import-id"` — the only adoption surface in the
  language. `adopt-org-policies` is an alias.
- Compliance plane: `require` is text only and judges the DECLARED estate;
  `report-compliance` verifies witnesses through Cloud Asset Inventory and
  compares org policies by VALUE — a policy that exists but is switched off
  reads NOT ENFORCED, which outranks DRIFTED. `--prowler` reads Prowler's OCSF
  output (≥ 4) and the legacy JSON; a FAIL on a verified witness makes the row
  CONTESTED (integration proposal I2).
- A key repeated inside one body is a parse error naming both lines (was the
  last known v0 silent-last-wins defect; repeated blocks are a list of
  objects, resource-type maps may repeat).

## Scripts

`docs/scripts.md` describes `scripts/` — the operations that are neither a
command nor a preset (SCC service enablement, the doc build, the privacy gate).
