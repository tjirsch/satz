# `scripts/` — the operations that are not the compiler

Everything in `scripts/` exists because it cannot be a satz command or a
preset. Two kinds live here: **cloud steps with no provider resource** (nothing
in Terraform can express them, so they stay `gcloud`), and **build-time helpers**
that maintain the repo's own data files.

| script | kind | what it does |
|---|---|---|
| `scc-enable-all.sh` | cloud step | enable every SCC service at the org, inherit below |
| `update_import_config.py` | helper | add newly-seen provider types to `presets/import-config.yaml` |
| `smoke.sh` | gate | every estate-consuming command end to end against `tests/smoke/`; CI runs it on every push and PR |
| `inspect_schema.py` | helper | print one resource type's schema out of a provider schema dump |
| `build-satz-doc.py` | helper | render `docs/satz-language.md` as a self-contained HTML page with the layers diagram inlined |
| `check-names.sh` | gate | refuse any identifier that is not one of the example customers (`docs/example-customers.md`); CI on every push, `--staged` from the pre-commit hook |

---

## `scc-enable-all.sh` — Security Command Center services

Turns every SCC service on at the organization and makes everything below the
organization inherit it.

```bash
scripts/scc-enable-all.sh --organization 123456789012            # dry run
scripts/scc-enable-all.sh --organization 123456789012 --apply    # write
```

### Why it is a script

google/google-beta 7.12.0 carry **no binding** for
`securitycentermanagement.googleapis.com`'s `SecurityCenterService`. Turning
Security Health Analytics, Event Threat Detection, Container Threat Detection,
VM Threat Detection, Web Security Scanner or DSPM on or off therefore cannot be
written as a resource in any language satz compiles, and neither can tier
activation.

Everything **downstream** of activation is codeable and belongs in a preset:
custom modules (SHA + ETD), sources and source IAM, notification configs (v1 and
v2), BigQuery exports, mute configs, and
`google_securityposture_posture(_deployment)`. This script covers only the part
that has no resource to compile.

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
| `--apply` | actually write — without it every call carries `--validate-only` |
| `--services "a b c"` | use this service list verbatim instead of discovering it |
| `--all-services` | include the AWS/Azure connector services |
| `--org-only` | enable at the org, skip the sweep |
| `--descendants-only` | only sweep folders/projects to `INHERITED` |
| `--reset-modules` | also set every module to `INHERITED` |
| `--targets-file FILE` | explicit `folders/<id>` / `projects/<id>` list instead of hierarchy discovery |

### Three things worth knowing before running it

**It is a dry run by default.** Every `gcloud` call goes out with
`--validate-only` until `--apply` is passed. This runs against customer orgs;
the safe direction is the default one.

**The service list is read from the org, not hardcoded.** It comes from
`gcloud scc manage services list --parent=organizations/<id>`, so a service
Google adds later is picked up without touching the script. The built-in list of
14 is only a fallback for when that call cannot be made, and the AWS/Azure
connectors in it are skipped unless `--all-services`, because they fail noisily
on an org with no such connector.

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

---

## `update_import_config.py` — keep the type table current

Reads every provider schema JSON in a directory and adds any resource type that
`presets/import-config.yaml` does not yet list, as an auto-generated entry
with `asset_type: TODO/UNKNOWN` for a human to fill in. Existing entries are
never rewritten, and comments survive (it uses `ruamel.yaml`).

```bash
uv run --with ruamel.yaml scripts/update_import_config.py \
  --schema-dir <dir-of-schema-json> \
  --config-file presets/import-config.yaml
```

## `inspect_schema.py` — look at one type

Prints one resource type's schema out of a `terraform providers schema -json`
dump, and lists near-miss key names when the type is not found. Edit the `target`
variable to change which type it reports.

```bash
python3 scripts/inspect_schema.py <schema.json>
```

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
never a guess), then `cargo test`.

```bash
scripts/smoke.sh                 # builds target/release/satz if missing
SATZ=~/.cargo/bin/satz scripts/smoke.sh
```

`.github/workflows/smoke.yml` runs it on every push and pull request with OpenTofu
installed. A new command that reads an estate gets a step here in the same PR.
