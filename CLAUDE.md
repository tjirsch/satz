# satz — working rules

Rust tool (bin `satz`, crate `crates/satz-core`) that compiles estates written in **Satz** — a language whose
resource types and attributes are the Terraform provider's, to the underscore —
to OpenTofu/Terraform HCL, with a compliance plane on top: claims → catalogs →
`require` (goal view, declared) → `report-compliance` (evidence, verified live,
by value). `docs/satz-language.md` is the language reference;
`docs/security-toolset-integration.md` is the proposal for the audit loop.

This file holds the rules that apply to every contributor. The maintainer's
private working state (fleet, customers, runbooks, task list) lives OUTSIDE
this repository — in `CLAUDE.local.md`, which is git-ignored and gate-rejected,
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
  (`.github/workflows/names-gate.yml`); the pre-commit hook runs it locally —
  enable once per clone with `git config core.hooksPath .githooks`. Examples
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
- **YAML exists only to be migrated (2026-08-29).** `migrate-to-satz` must keep
  converting old estates and packs for as long as legacy orgs exist; that is
  the whole YAML surface. No new functionality grows a YAML arm, YAML is never
  generated, and a YAML code path that a cleanup breaks is deleted, not
  repaired — the legacy walk, the `.gen.yaml` twin and every YAML command arm
  are gone since v0.46.14. A conversion is reported as NEEDS-REVIEW where it
  cannot be proven; migrated estates may need manual edits (an old
  `!import-include` becomes `use` plus `satz adopt`).
  `tests/corpus/yaml-estate/` is the converter's gate: a YAML fixture through
  `migrate-to-satz` must compile as Satz and emit the expected resource set.
- **`cargo test` does NOT rebuild the debug binary** — `cargo build` before a
  live test, or a stale binary shadows the fix. Same family: an edit to
  `crates/satz-core/` was once not picked up — `touch` the file and confirm
  "Compiling satz-core" before trusting a live run.
- **Corpus (`tests/corpus/`) is snapshot-gated:** `UPDATE_CORPUS=1` + review
  the diff.
- **Docs are derived from the parser, not from intent.** Every example in
  `docs/satz-language.md` compiles; where the doc and the parser disagree, the
  parser is right and the doc is a bug.

## Language state (v0, satz v0.46.4)

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
  rules per type in `presets/discovery-config.yaml` for the rest), never
  guesses (one candidate resolves, many is ambiguous), and `--execute` writes
  verified ids back as `"import-id"` — the only adoption surface in the
  language. `adopt-org-policies` is an alias.
- Compliance plane: `require` is text only and judges the DECLARED estate;
  `report-compliance` verifies witnesses through Cloud Asset Inventory and
  compares org policies by VALUE — a policy that exists but is switched off
  reads NOT ENFORCED, which outranks DRIFTED. `--prowler` currently reads
  Prowler's legacy JSON only (OCSF is not parsed) — see the integration
  proposal.
- Known v0 defects, documented in the language reference: the `google_folder`
  emitter drops every attribute except `display_name` / `parent` / `labels` /
  `lifecycle`; a repeated block key inside one body silently last-wins (write
  repeated blocks as a list of objects).

## Scripts

`docs/scripts.md` describes `scripts/` — the operations that are neither a
command nor a preset (SCC service enablement, the doc build, the privacy gate).
