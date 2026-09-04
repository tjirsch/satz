# Housekeeping — the files that go stale on their own

Most of this repository is code, and code tells you when it is wrong. A handful of
files are **derived from something outside the repository** — Google's provider
schema, Google's constraint catalogue, Google's asset-type list, a CIS benchmark
release — and those go stale in silence. Nothing breaks. The tests stay green. The
answers just quietly describe last month's Google.

This page lists every one of them: what it is, what refreshes it, what makes it
stale, and what catches it if nobody remembers. Where a refresh can be a script it
is one, and the script lives in `scripts/` — see [scripts](scripts.md).

## At a glance

| file | refreshed by | trigger | what catches staleness |
|---|---|---|---|
| `tests/schemas/google.json` | `scripts/update_schema_fixture.py --add` | the provider pin moves | **nothing** — run `--check` |
| `presets/cai-asset-types.txt` | by hand, from Google's docs | new asset types appear | indirect: unfilled `import-config` rows |
| `presets/import-config.yaml` (`asset_type`) | `scripts/update_import_config.py --cai-types` | the CAI list above changes | smoke: *"every derivable asset_type is filled"* |
| `presets/managed-constraint-equivalents.txt` | `scripts/update_constraint_equivalents.py` | Google ships a new managed twin | `cargo test` catches the *effect*, not the table |
| `presets/docs/*.md` | `satz doc-packs` | any pack changes | smoke: `doc-packs --check` |
| `presets/CHANGELOG.md` | by hand, one row per pack version | a pack version changes | smoke: fails on a version with no row |
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

- `presets/docs/*.md` (24 pages) — generated by `satz doc-packs`, gated by
  `doc-packs --check` in the smoke matrix. Regenerate in the same commit as any pack
  change; CI fails otherwise.
- `presets/CHANGELOG.md` — one row per pack version, by hand. The smoke matrix fails
  on a pack version with no row, so this cannot rot quietly.

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

## The shape of the rule

Anything derived gets, in order of preference:

1. a **script** in `scripts/`, so the refresh is reproducible and reviewable;
2. a **gate** that fails offline when the derived file and its consumers disagree;
3. failing both, a **line on this page** saying what goes stale and when.

Silence is the thing to avoid. A file that is merely out of date is a small problem;
a file that is out of date while the tests report success is how a compliance tool
starts lying.
