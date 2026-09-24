# satz — working rules

Rust tool (bin `satz`, crate `crates/satz-core`) that compiles estates written in **Satz** — a language whose
resource types and attributes are the Terraform provider's, to the underscore —
to OpenTofu/Terraform HCL, with a compliance plane on top: claims → catalogs →
`require` (goal view, declared) → `report-compliance` (evidence, verified live,
by value). `docs/language.md` is the language reference;
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
  that is not a documented example value (`docs/examples.md`) — `C0…`
  directory ids, 11–13-digit org/project/folder numbers, billing accounts,
  GUIDs (an Entra tenant id) and their dashless 32-hex form (the workload
  identity pool id), project ids in `projects/…`, `project = …` and
  `--project`, e-mail addresses, domains that are neither IANA-reserved nor a
  known vendor host, repository URLs and checkout paths — in files and in
  commit messages;
  it rejects local files (`CLAUDE.local.md`, `*.local.md`, `.claude/`,
  `attestations.yaml`, `evidence/`) if they are ever staged; and it rejects
  any commit whose author or committer is not a GitHub noreply address —
  `$NAMES_IDENT` in the environment allows one more address in a clone, and no
  personal address is written into the script. CI runs it on every PR and
  every push to `main`
  (`.github/workflows/names-gate.yml`), beside the smoke matrix
  (`scripts/smoke.sh`, `.github/workflows/smoke.yml`: every estate-consuming
  command end to end against `tests/smoke/` — a new command gets a step there
  in the same PR — plus a `checks` job: `clippy -D warnings` and
  `cargo test --workspace --locked`); the pre-commit and commit-msg hooks run
  the gate locally — enable once per clone with
  `git config core.hooksPath .githooks`. The gate judges tokens, not lines,
  and an unusable range or a missing file is a failure, never a pass. Examples
  use ONLY the four example customers. If an example needs a value the table
  does not have, add it to the table in the same commit. Identifiers a VENDOR
  publishes and every customer shares — Microsoft's commercial tenant, an
  application id — are allowed by name in `ALLOW_GUID` and listed in that same
  table; a GUID the gate does not know is assumed to identify a customer.
  What the gate cannot see is a NAME: a display name or a company in prose has
  no shape. That is what the local, never-committed denylist
  (`$NAMES_DENYLIST`, a file of one extended regex per line, unset by default)
  is for, and what review is for.
- **Release flow:** work commits on `main`; `cargo release patch|minor
  --execute --no-confirm` bumps, tags and pushes; the tag triggers cargo-dist.
  `.github/workflows/release.yml` is generated from `dist-workspace.toml`: change
  that file and run `dist generate` with the version its `cargo-dist-version`
  names, never edit the workflow. The `release-workflow` job of `smoke.yml` runs
  `dist generate --check` on every PR and every push to `main` and fails on a
  hand-edited or un-regenerated `release.yml`, printing the diff.
  **What a release keeps (2026-09-23):** `prune-releases.yml` deletes every release but
  the five newest, tags included. A release satz still NAMES survives that — the workflow
  reads `satz_core::LAST_YAML_CONVERTING_RELEASE` out of the source at the commit being
  released and keeps it however old it is, because a refusal that points at a 404 is a
  dead end. A new constant that names a release adds a line there in the same change.
  **Minor or patch (2026-09-11, ADR 0010):** a release is a MINOR when the same
  estate or input, run through the new binary, needs an edit, is refused, or
  plans differently — a language change, a removed or renamed command or flag,
  an input format no longer read, an emission change that moves a plan.
  Everything else is a PATCH. The minor number is how an operator sees that an
  upgrade brings work; it grants no time to postpone that work — everyone stays
  on the current version. Release without asking when a discussed solution is releasable (tests +
  clippy green, docs updated). After the release, stop — no polling of the
  GitHub API (unauthenticated: 60 req/h, shared with users' `self-update`).
- **satz owns no credential.** No OAuth client, no token on disk, no per-estate
  credential store: the Application Default Credentials are gcloud's and satz
  only reads them. What satz owns is the IDENTITY — post-init, anything that
  reads or writes a customer's estate runs as that estate's IaC service
  account, derived from `svc_iac_account` + `infra_project_name` exactly as the
  emitter derives the provider's `impersonate_service_account`. The exceptions
  are bare `whoami` and bare `import` (given an estate — `whoami <estate>`,
  `import --into`/`--as` — they bind like everything else), `bootstrap`, `init`,
  `migrate` (`--mode cloud` assigns Groups Admin, which the account cannot give
  itself) and `map-types`, each named with its reason in `IDENTITIES` (`src/main.rs`), which a test forces every new
  command to join. One command, one identity: the CLI binds it for the process
  and a second, different binding is refused, never ignored. `satz mcp` is
  long-lived and works through estates in turn, so it SCOPES the identity to each
  call instead — same rule, stated per call. It must be a scope rather than a
  mutable global because the server dispatches concurrently, and a target that
  changed under a call in flight is how one customer's tools run as another's
  service account.
- **What an agent may run is decided per command.** `MCP_PARITY` (`src/mcp.rs`)
  names every CLI command with the MCP tool that serves it or the reason none
  does; `cargo test` fails on a command in neither column, on a registered tool
  the table does not name, and on a tool `docs/mcp.md` does not list. The server
  sends both halves to the client at initialize — the command behind each tool,
  and what it will not run with the reason — so an agent asks for `apply` instead
  of writing HCL by hand. A new tool gets a step in the smoke matrix in the same
  PR: its `tools/list` assertion is an exact set.
- **Presets, provenance by suffix:** `X.satz` pristine, upstream-owned, always
  overwritable / `X.local.satz` the user's fork, never touched by updates /
  `X.diff.satz` the current adoption delta, rewritten on every merge. A preset
  an estate INCLUDES never changes silently: a semantic upstream change
  (compiled canonical form differs) auto-forks and repoints the estate with a
  transpile-identity proof; comment/format churn upgrades silently. Pack
  versions live IN-FILE; filenames carry only framework versions. Never
  `.local.<n>.satz`.
- **One form of a constraint, never two.** Where Google replaces a legacy
  org-policy constraint with a managed one, a pack runs the replacement ALONE
  and declares the legacy twin off (`spec { reset = true }`) — absence is not
  enough, because a policy already set on an organisation is invisible to an
  apply that does not declare it. The pairing is data
  (`presets/managed-constraint-equivalents.txt`, refreshed from a live org by
  `scripts/update_constraint_equivalents.py`, never by hand) and the rule is a
  test that compiles every corpus case against it.
- **Memberships stay OUT of presets** — presets define groups, humans grant
  membership.
- **The IaC service account holds named roles, never `roles/owner`** (ADR 0009). A
  pack that emits a resource type the prerequisite table in `src/prerequisites.rs`
  has no row for — the roles it needs AND the API that serves it — adds the row in the
  same PR, verified with `scripts/check_prerequisites.py`; a new pack
  gets a line in one of the `tests/iac/` cases. `prerequisites_gate` fails on either
  omission.
- **80% of customisation via params, the rest via `.local` forks** — no
  variable explosion. Names that must be globally unique derive from
  `customer_shortname`.
- **Everyone is on the CURRENT version.** A release IS the migration: a
  breaking language change ships together with the estate edits that satisfy
  it. No deprecation periods, no dual-accept paths for old binaries.
- **satz reads no YAML estate (ADR 0048).** A `.yaml` estate or pack is the
  pre-Satz YAML dialect, and no command reads it: every entry point refuses it by
  name and points at `satz_core::LAST_YAML_CONVERTING_RELEASE`, the last release
  that converts, followed by `satz fmt` and `merge-presets` on the current binary.
  That constant is the one place the release is written; a refusal that names a
  version literal is a bug. YAML is never parsed as a language and never
  generated, and a YAML code path a cleanup breaks is deleted, not repaired. What
  survives is the PRINTER (`crates/satz-core/src/migrate.rs`): every import shape,
  the HCL importer and the org-policy export write Satz through `convert_value`,
  so nothing in that file may be cut without checking its callers.
  `presets/import-config.yaml` and `presets/catalogs/*.yaml` are data, not
  estates — they are YAML and stay YAML.
- **`cargo test` does NOT rebuild the debug binary** — `cargo build` before a
  live test, or a stale binary shadows the fix. Same family: an edit to
  `crates/satz-core/` was once not picked up — `touch` the file and confirm
  "Compiling satz-core" before trusting a live run. `scripts/smoke.sh` always
  builds the binary it judges; pass `SATZ=` only when you built the binary
  yourself and want that one.
- **Corpus (`tests/corpus/`) is snapshot-gated:** `UPDATE_CORPUS=1` + review
  the diff.
- **Every Satz file in the repository is formatted (2026-09-13, ADR 0017).** `satz fmt`
  is the layout — two-space indent, `=` aligned over a run, list commas — and both
  `cargo test` (`crates/satz-core/tests/fmt_corpus.rs`) and the smoke matrix fail on a
  file that is not. Run `satz fmt presets tests` before committing a `.satz` change;
  the formatter never changes meaning, and the same test proves it. Satz text has
  three ways to disk, all in `src/fsx.rs`: `write_generated_satz` (composed whole,
  formatted), `write_edited_satz` (a splice; the author's layout stays, a formatted
  file stays formatted), `write_verbatim` (bytes satz copies). `fsx::write` refuses a
  `.satz` path and `clippy.toml` disallows `std::fs::write` outside that module.
- **Docs ship with the change, in two renderings.** A change to the language,
  a command, a flag or a pack updates `README.md`, `docs/*.md` and
  `presets/README.md` in the same PR (the language reference cites
  `tests/smoke/yaml/showcase.satz` — a new feature gets its example there
  first). The HTML site <https://tjirsch.github.io/satz/> is rendered from
  those files by `scripts/build-site.py` — from the ones it PUBLISHES: every
  `docs/*.md` must be named in that script's `SITE_DOCS` or in
  `SITE_DOCS_EXCLUDED` (with its reason), and the build fails naming any doc in
  neither, so a page reaching the public site is a decision and so is a page not
  reaching it. The MENU is the same kind of decision: `NAV_ORDER` names every
  page in reading order — `satz`, `language`, `library`, `workflows`,
  `interview`, `mcp`, `examples`, `housekeeping`, `competitive`, `llms` — and a
  page it does not name fails the build rather than being appended
  alphabetically. A page title is that menu word after `satz` (`# satz
  language`, `# satz library`) and carries no trailing explanation — what the
  page IS belongs in its opening line. Published by
  `.github/workflows/pages.yml` on every release tag; nothing is written for
  the site separately, and a doc that is not in the repo is not documentation.
  Packs: `presets/docs/` is generated by `satz doc-packs` — a change to a pack
  regenerates it (`--check` is a smoke gate AND a unit test), and a version bump
  adds a row to `presets/README.md` under `## Changelog`, which `doc-packs` reads
  and refuses to be without. Each page also carries the pack's own history, so a
  changelog edit makes a page stale. The index prints the first sentence of each
  pack's header comment and refuses one that says nothing; a pack the shape does
  not decide how to `use` states its own line in that header.
  The site parses with cmark-gfm, GitHub's own parser (ADR 0008), so it shows
  what GitHub shows — lists, fences and heading anchors included; a link to an
  anchor that does not exist fails the build. Run it locally before a docs PR:
  `uv run scripts/build-site.py _site` (the script declares its dependency) and
  open `_site/index.html`. `satz open-readme` and the post-install step open
  that site.
- **A decision that was not obvious gets an ADR.** `docs/adr/`, MADR form, one
  file per decision, numbered and never renumbered: the context, the options with
  their real trade-offs, what was chosen and what it costs. The bar is "would
  reversing this be expensive, or would a reader ask why on earth" — most changes
  are neither. A commit message explains one change to whoever reads that commit;
  an ADR answers the same question months later, when the alternative has started
  to look attractive again, which is how decisions here got re-proposed after the
  reasoning was lost. A superseded record stays, with its status changed and its
  successor named. These pages are not on the site: `build-site.py` reads
  `docs/*.md` only, so a record implies no navigation decision.
- **Docs are derived from the parser, not from intent.** Every example in
  `docs/language.md` compiles; where the doc and the parser disagree, the
  parser is right and the doc is a bug.
- **Docs say what satz does, now (2026-09-11).** Facts in the present
  tense: what a command, statement or pack does and what it refuses. No history —
  versions, dates, "used to", "since vX", "no longer", how a rule came about;
  that belongs to the changelogs, the ADRs and git. No interpretation and no
  warnings or disclaimers around the facts. A reason stays when it is a present
  fact ("so one apply authenticates as one principal"), not a story. A pitfall
  is stated as what to do to make it work and what does not work, with the
  reason — never under a label ("the quota-project trap"), and without
  rhetoric ("on purpose", "deliberately", "silently fails", "the point").
  The single exception is `## Breaking changes` in `presets/README.md`: per
  release, newest first, what satz now refuses that it compiled before and the
  edit that satisfies it, written so an operator can act with no context. A
  change that refuses a form an estate may hold adds its entry there in the
  same PR.
- **`Satz` is the language; `satz` is everything else** — the binary, the
  project, the repository. "written in Satz", "a Satz estate", "a Satz
  abstraction"; "satz compiles", "a satz command", "the satz repository". Page
  titles are the one exception: they stay lowercase (`# satz language`), being
  the site's identity line rather than prose.

## Language state (v0)

- The fragment pipeline parses Satz directly: per-file fragments, the ⊕ fold
  (same address, different body = hard error naming both files), schema-typed
  resources (an unknown block key is a parse-time error, and so is a key inside a
  body that the schema does not name — the nine satz reads itself are a closed
  list in `satz_body_key`, ADR 0047), `use` / `use … as` /
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
  reads NOT ENFORCED, which outranks DRIFTED. `--prowler` reads the OCSF export
  of Prowler 5 only — check id from `metadata.event_code`, project from
  `cloud.account.uid`, the version from `metadata.product.version`, which is
  checked (an older export is refused by its version); a FAIL on a verified
  witness makes the row CONTESTED (integration proposal I2).
- A key repeated inside one body is a parse error naming both lines (repeated
  blocks are a list of objects, resource-type maps may repeat).

## Scripts and housekeeping

`docs/housekeeping.md` is one page for two halves of the same subject. Its second
half describes `scripts/` one script at a time — the operations that are neither a
command nor a preset (SCC service enablement, the doc build, the privacy gate).

Its first half lists every file DERIVED from something outside this
repository — the provider schema fixture, the CAI asset-type list, the
managed/legacy constraint pairing, the catalogs, the version pins, the `NOTICE`
that attributes them — with what refreshes each, what triggers it, and what
catches it when nobody remembers. Six of them have no automatic check at all,
which is the reason the page exists. Anything derived gets a script, else a gate, else a line on that page:
a file that is out of date while the tests report success is how a compliance
tool starts lying.
