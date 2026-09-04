# `scripts/` — the operations that are not the compiler

Everything in `scripts/` exists because it cannot be a satz command or a
preset. Two kinds live here: **cloud steps with no provider resource** (nothing
in Terraform can express them, so they stay `gcloud`), and **build-time helpers**
that maintain the repo's own data files.

A cloud step still cannot be **authored** in Satz — that is what makes it a
script. It can, since v0.46.69, be **declared and invoked** by an estate: an
[`action`](satz-language.md#613-action--a-step-with-no-provider-resource) names
the step, binds it to one of these files, and builds its arguments from the
estate's own params, so `satz run-actions` runs it with the organisation id the
estate already knows instead of a human retyping it. Nothing runs at transpile
time and an action carries no claim; the script stays exactly as opaque to the
compliance plane as it is today.

| script | kind | what it does |
|---|---|---|
| `scc-enable-all.sh` | cloud step | enable every SCC service at the org, inherit below |
| `update_import_config.py` | helper | keep `presets/import-config.yaml` current: new provider types, and `asset_type` filled from Google's Cloud Asset Inventory list |
| `smoke.sh` | gate | every estate-consuming command end to end against `tests/smoke/`; CI runs it on every push and PR |
| `inspect_schema.py` | helper | print one resource type's schema out of a provider schema dump |
| `build-satz-doc.py` | helper | render one `docs/*.md` as a self-contained, theme-aware HTML page (SVGs inlined) |
| `build-site.py` | build | render the whole documentation site (README, `docs/*.md`, the presets docs) into `_site/` with a sticky navigation header and a client-side search over every page's headings and text (`search-index.js`, no external dependencies; `/` focuses the box); `.github/workflows/pages.yml` publishes it on GitHub Pages on every release tag and on demand |
| `check-names.sh` | gate | refuse any identifier that is not one of the example customers (`docs/example-customers.md`); judged per TOKEN (an allowed address never shields a private one beside it); CI on every push (`--commits A..B`, an unusable range is a failure, never a pass), `--staged` from the pre-commit hook, `--message FILE` from the commit-msg hook, `FILE…` for one file (missing file = failure) |

---

## `scc-enable-all.sh` — Security Command Center services

Turns every SCC service on at the organization and makes everything below the
organization inherit it.

```bash
scripts/scc-enable-all.sh --organization 123456789012            # dry run
scripts/scc-enable-all.sh --organization 123456789012 --apply    # write
```

Its flag shape is also the shape an `action` binds to — `args` for the form that
reads, `execute_args` for the flag that writes:

```
action "scc-services" {
  reason       = "SCC service enablement has no provider resource (google 7.14.1)"
  run          = "../scripts/scc-enable-all.sh"
  args         = ["--organization", "{customer_organization_id}"]
  execute_args = ["--apply"]
}
```

`satz run-actions <estate>.satz` then prints the resolved command line, `--check`
runs the dry run, and `--execute` adds `--apply`.

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

---

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

Needs `tofu` on PATH; talks to no organisation. See
[housekeeping](housekeeping.md) for the other derived files and their triggers.

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
never a guess), `scan` when `checkov` or `uvx` is on PATH, then `cargo test`.

```bash
scripts/smoke.sh                 # builds target/release/satz if missing
SATZ=~/.cargo/bin/satz scripts/smoke.sh
```

`.github/workflows/smoke.yml` runs it on every push and pull request with OpenTofu
installed. A new command that reads an estate gets a step here in the same PR.
