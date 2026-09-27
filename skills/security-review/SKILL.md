---
name: security-review
description: >-
  Run a CIS GCP Foundations v5.0 security review for one customer organisation and produce the two
  Security-Toolset deliverables — the CIS-Checkliste (.xlsx, every one of the 93 controls with a
  status from the taxonomy FAIL / LÜCKE / MANUAL / PASS / FALSE POSITIVE / N/A) and the
  Remediation-Plan (.docx, German, satz packs + gcloud). Phase 1 creates the evidence LIVE against the
  customer org (Prowler per service, satz report-compliance + Checkov, triage, require); phase 2 builds
  both documents OFFLINE from that evidence. Use this skill whenever the user mentions a security
  review, security report, Prowler or Checkov run, CIS checklist, Findings-Excel, Remediation-Plan,
  compliance evidence, "Sicherheitsreview", "Checkliste", "Maßnahmenplan" for a customer folder under
  ~/projects/ccc/SHORTCODE/ — even if they only ask for "the Excel" or "the plan", or only for the
  scan, or want an earlier scan's documents rebuilt.
  Do not include the path to Documents like ~/projects/ccc/SHORTCODE/ or others as the contain PII.
---

# Security review (CIS GCP 5.0) for one customer

Two phases, always in this order of trust: **evidence is created live** against the customer's
organisation (read-only), **documents are built offline** from that evidence and nothing else.
The documents can be rebuilt any number of times from a scan folder without touching the cloud;
a scan cannot be faked from documents. Keep the two apart in your head and in the file tree.

Inputs the user gives you: a customer shortcode or folder (`acme`, `~/projects/ccc/acme`). Everything
else is discovered from the customer folder: the central estate repo
`~/projects/ccc/<shortcode>/<shortcode>-C<dirid>/` and the workloads beside it.

## Layout (fixed — other tools and people rely on it)

```
~/projects/ccc/<shortcode>/                  the customer folder (CCC_ROOT overrides ~/projects/ccc)
├── <shortcode>-C<dirid>/                     the CENTRAL estate repo (never renamed)
│   ├── config.toml                           `yaml_dir` names the estate directory; absent = `satz/`
│   ├── satz/ (or yaml/)                      <id>.satz, the estate; requests/<project>.satz, vendored request files
│   ├── hcl/                                  generated root module (report-compliance's Checkov runs here)
│   ├── interfaces/{common,<project>}/<iface>/ generated: README.md, hcl/, satz/interface.satz per interface
│   └── presets/ schemas/ evidence/
├── <workload folder>/  (zero or more)        a PROJECT beside the estate, one of two kinds:
│     satz — config.toml + <yaml_dir>/<project>.satz with a `use "…/interface.satz"` line, its own
│            hcl/, schemas/ and vendor/<project>/ (the interfaces it took from the estate); its own
│            IaC service account and state
│     hcl  — a directory of .tf files reading `module.satz.<export>` (a Terraform-only team), no config.toml
│     anything else beside the repo (documents, pdfs) is not a workload
└── audit/                                    skill-owned
    ├── <domain>/<scan-date>/
    │   ├── raw/                   Prowler OCSF per service + merged, svc-*.log, MANIFEST.md
    │   └── evidence/              satz-report-cis-gcp-5.0.json, cis-gcp-5.0-latest.md,
    │                              satz-triage-cis-gcp-5.0.md, satz-require-cis-gcp-5.0.json,
    │                              consumer-<workload>.txt + checkov-<workload>.json per workload
    ├── tools/scope.yaml           the customer-specific values (one file, reviewed by a human)
    ├── <NN>-<shortcode>-<estate>-CIS-5.0-Checkliste_<date>.xlsx
    └── <NN>-<shortcode>-<estate>-Remediation-Plan_<date>.docx
```

The central estate's own view of its projects is `satz interfaces <estate>.satz --format json --out -`
(offline): every export with the interface it stands in, every interface, every request point. A
project's Google project id is the static export `project_id` of its interface; `workload_folder`
is a core export. `discover_scope.py` reads it for you.

Scripts live in this skill's `scripts/`; run them with absolute paths (`$SKILL/scripts/...`). Python
scripts need `uv run --with openpyxl --with python-docx --with pyyaml python3 <script>`.

## Phase 0 — preflight (2 minutes, no cloud calls except whoami)

1. `python3 $SKILL/scripts/discover_scope.py <shortcode> --scan-date <YYYY-MM-DD> --write ~/projects/ccc/<sc>/audit/tools/scope.yaml`
   prints org id, domain, infra project, IaC service account, estate file (from `yaml_dir`), audit
   folder, one `workload=<name>:<kind>:<folder>` line per workload beside the repo (`workloads_arg=`
   is the same list on one line, for `satz_evidence.sh --workloads`) and `own_projects=` — the infra
   project, the estate's `google_project` ids and every workload's Google project, read from
   `satz interfaces` when satz is on PATH — and seeds `scope.yaml` from the template if none exists,
   its `workloads:` list included. If it warns that a value is missing, read the estate's
   `params { … }` yourself and fill it in; do not guess an org id. If it warns that `satz interfaces`
   could not run, fill `workloads[].project_id` and the own-project list by hand from the estate.
2. Tools: `prowler --version` (5.x), `satz --version`, `checkov --version`, `gcloud auth list`.
   Missing tools: `uv tool install prowler`, `uv tool install checkov`; satz via its installer.
3. Identity: `cd <estate_repo> && satz --config . whoami <estate>.satz` must print
   `runs as: <iac-sa> — may impersonate` and `quota project … reachable`. If the ADC belongs to
   another customer, STOP and ask the user to switch (`gcloud auth application-default login`);
   never scan one customer with another customer's credentials — the scan would fail late, after
   an hour, and the manifest would name the wrong identity.
4. Say what you are about to do in one line: customer, org id, identity, where the files go.

For an **offline rebuild** of an existing scan folder, only step 1 applies (no whoami, no gcloud);
if `audit/tools/scope.yaml` was written by the earlier per-customer generator (`measures: M1: [...]`,
no `estate_file`/`iac_sa`), rewrite it in the template's format — the generator refuses the old one.

## Phase 1 — evidence, LIVE and read-only

The credentials are the operator's ADC impersonating the estate's IaC service account. Prowler,
report-compliance and triage only read. Nothing in this phase changes the organisation, and this
skill never runs a remediation: the plan is the deliverable, execution is the customer's decision.

1. **Prowler, one run per service with a deadline** (a monolithic org scan freezes on large orgs —
   the Dataproc/Compute clients enumerate every project without a socket timeout; three lost
   runs on a 229-project org taught this):
   ```
   $SKILL/scripts/prowler_scan.sh --org <org_id> --sa <iac_sa> --out <scan-dir>/raw --prefix <sc> \
       --own-projects "<space-separated own project ids>" [--deadline 2700] [--parallel 6]
   ```
   Own projects = the customer's real projects, i.e. everything that is not a generated/fleet
   project: the `own_projects=` line of `discover_scope.py` — infra project, the estate's own
   projects and every workload's Google project — is the list to pass. First time on an org, or
   when the estate exports nothing: run without `--own-projects`, read the own-project list that
   `merge_ocsf.py` prints, then run compute on them (`--services ""` skips the org services):
   `prowler_scan.sh … --services "" --own-projects "…"`. Compute runs only on own projects.
   Run it in the background and poll `svc-*.log` every few minutes (`--verbose` prints progress);
   a service that hits the deadline is reported in the summary and in the manifest — note it as a
   caveat, do not silently accept a missing service. Expect 10–60 minutes.
2. **Merge + manifest**:
   `python3 $SKILL/scripts/merge_ocsf.py --raw <scan-dir>/raw --prefix <sc> [--fleet-regex '<re>']`
   → `<sc>-prowler-merged.ocsf.json`, `MANIFEST.md` (files, sha256, per-service outcome, own
   vs fleet projects, caveats). The merge takes every `*.ocsf.json` in `raw/`, so re-run it after
   any Nachscan (a later compute run, a re-run of a service that hit the deadline) — the merged file
   is the one input of phase 2 and must contain everything. Read the FAIL-per-check table it prints: it is the first sanity
   check (e.g. 226 × `iam_cloud_asset_inventory_enabled` means no logging foundation, not 226 problems).
3. **satz evidence** (report-compliance live + Checkov over `hcl/`, triage, require, then every
   workload):
   ```
   $SKILL/scripts/satz_evidence.sh --repo <estate_repo> --estate <estate>.satz \
       --ocsf <scan-dir>/raw/<sc>-prowler-merged.ocsf.json --out <scan-dir>/evidence \
       --workloads "<the workloads_arg= line of discover_scope.py>"
   ```
   Checkov is installed via `uv tool install checkov`; satz runs it itself with `--checkov` over the
   estate's `hcl/`. Per workload, offline and after the estate's own evidence: a satz project is
   compiled (`satz --config <folder> transpile <project>.satz --check`) and its generated `hcl/` is
   held to the interface (`satz --config <repo> check-consumer <folder>/hcl <estate>`); an HCL
   project is held to it directly (`check-consumer <folder> <estate>`); Checkov runs over each
   project's HCL (`checkov -d … -o json` → `checkov-<workload>.json`). Every step lands in
   `consumer-<workload>.txt` with its exit code (4 = skipped, the reason on the line before — a
   satz project that has never been transpiled has no `hcl/`, and its compile is then the only
   check). A refused consumer check is the project team's measure, not the estate's.
4. Fill `scope.yaml` from what you saw: `fleet_regex`/`fleet_label` (a noun without the count)
   when the org has generated projects with an identical picture (aggregate them — 212 rows saying the same thing hide the
   17 that matter), `scan_caveats` from the manifest, `luecke` for automated controls whose
   foundation is missing, `false_positives` only with a proof command in the text,
   `estate_status` in one sentence. Read `references/format.md` for what each key does.

## Phase 2 — documents, OFFLINE

```
uv run --with openpyxl --with python-docx --with pyyaml python3 $SKILL/scripts/build_audit.py \
    --scope <audit>/tools/scope.yaml \
    --ocsf <scan-dir>/raw/<sc>-prowler-merged.ocsf.json \
    --satz-report <scan-dir>/evidence/satz-report-cis-gcp-5.0.json \
    --satz-require <scan-dir>/evidence/satz-require-cis-gcp-5.0.json \
    --out-dir <audit>
uv run --with openpyxl --with python-docx python3 $SKILL/scripts/verify_outputs.py \
    --xlsx <audit>/CIS-5.0-Checkliste_<sc>_<date>.xlsx --docx <audit>/Remediation-Plan_<sc>_CIS-5.0_<date>.docx --customer <sc>
```

`--satz-report` and `--satz-require` are optional: without the report the Bewertung column has no
`[satz]` seed and the plan cannot say which controls are verified or drifted; without the require
file the "Pack stellt es bereit" hint is missing. Say so in the hand-over if either is absent. The
workloads' evidence (`consumer-<workload>.txt`, `checkov-<workload>.json`) is read from the folder
of `--satz-report`, or from `--evidence-dir`; a workload in `scope.yaml` without its file reads
"kein Nachweis" in both documents and becomes an open point.

What the generator does, so you can judge its output: every one of the 93 CIS 5.0 controls gets a
row; status from the taxonomy (FALSE POSITIVE from scope → FAIL if any FAIL → PASS → MANUAL if the
catalog says Manual → LÜCKE if the foundation is missing → N/A); Befund from Prowler's
`status_detail`, own projects individually, fleet collapsed; Prüfweg = the CIS AuditProcedure with
the real org id/project; Bewertung seeded from satz (`verified` with witness address, DRIFTED,
partial, unmet, plus the pack `satz require` says would provide it); Maßnahme = the plan section.
Measures come from `assets/measures.yaml`, a catalogue that covers all 93 controls: a measure is
emitted when one of its controls is FAIL/LÜCKE/FALSE POSITIVE (review measures: MANUAL), numbered in
phase order Sofort → Kurzfristig → Nach Freigabe → Laufend → CISO-Review, with satz pack lines AND
a gcloud alternative, prerequisites, and the proof command. MANUAL controls get a review table,
N/A controls a Wiedervorlage table naming the pack that pre-empts them. `scope.workloads` gets one
section in the plan and one sheet in the workbook, "Workloads": per project its name, kind, Google
project and the outcome of its consumer check (bestanden, or the refusals as satz printed them) and
its Checkov counts — the workload is named by its name, never by its folder, so `verify_outputs.py`
stays clean of paths.

Then read the plan yourself (`textutil -convert txt -stdout <docx> | less`) and the FAIL rows of
the workbook. You are the reviewer the generator does not have: check that the Befund of each
FAIL row names real resources, that the measure texts fit this customer (a measure's `why` is
generic — `scope.measures.override` lets you sharpen a title, aufwand or risiko; `extra` adds a
customer-specific measure; `skip` removes one), and that the Reihenfolge makes sense for this
org. Rebuild after every scope change — it takes seconds, and the documents must stay reproducible
from scope + evidence, never hand-edited.

## Hand-over

Report: the two file paths, the status counts (one line), the measures list with phases, the
caveats from the manifest, and what the reviewer must decide (Offene Punkte). The documents are
drafts for the CISO review, not findings of record — say so. Do not paste customer resource
names into chat beyond what the user needs to judge the result.

## Rules that are not negotiable

- **Live phase reads; nothing remediates.** No `tofu apply`, no `gcloud … update`, no
  `satz adopt --execute` during a review, even if the fix is one line. The plan's commands are for
  the customer's operator.
- **One customer, one identity, one scan folder.** Never mix files of two customers; the verify
  script fails on another customer's name for this reason. Other customers' documents are format
  references only (`references/format.md` has everything you need without opening them).
- **Every control, every time.** A control the scan did not report is not "fine" — it is MANUAL,
  LÜCKE or N/A with a reason. Prowler 5 emits no MANUAL rows and its CIS 5.0 map lacks 2.13
  (`check_overrides` in scope handles it).
- **German documents, English identifiers.** Prose in German as the reference documents; check
  ids, resource names, commands verbatim.
- **No local paths in a deliverable.** Paths expose the operator's user name and the customer
  folder layout (PII). Commands in the plan use the repo's folder name and relative paths
  (`cd <sc>-C<dirid>`, `OUT=audit/<domain>/…`); `verify_outputs.py` fails on `/Users/`, `~/`,
  `$HOME/` or `projects/ccc/` anywhere in either document.
- **Reproducible.** Documents are generated from scope + evidence. A change of wording goes into
  `scope.yaml` or `assets/measures.yaml` (if it is true for every customer), never into the .docx.

## When something is off

- Prowler stalls without progress in a service log for 15 minutes → let the deadline kill it; the
  service becomes a caveat (Dataproc is the usual one; check whether its API is even enabled:
  `gcloud services list --enabled --project=<p> | grep dataproc`).
- `satz report-compliance` refuses the identity → the ADC is not allowed to impersonate the IaC SA;
  the user has to fix membership in `svc-iac-users`, you cannot.
- Dossier titles in satz's own `remediation-plan` all identical → known satz defect with Prowler 5
  OCSF field names; this skill reads the OCSF directly and does not need that command.
- More lessons: `references/lessons.md`.
