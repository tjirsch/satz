# Lessons from real scans (keep adding)

## Prowler 5.x on GCP

- **Monolithic org scans freeze** after cloudstorage on organisations with hundreds of projects:
  the Dataproc and Compute clients enumerate every project with no socket timeout. Three runs of
  45+ minutes were lost that way. `prowler_scan.sh` runs one service per process with a kill
  deadline; Dataproc is the usual casualty and is N/A when its API is enabled nowhere.
- Progress: `--verbose` prints per-check lines into `svc-<service>.log`; a log without a new line
  for 15 minutes is a stall, not slowness.
- The Mac must not sleep during a scan (`caffeinate -i` around the run, or power settings).
- OCSF fields that matter: `metadata.event_code` (check id), `cloud.account.uid` (project id),
  `unmapped.compliance["CIS-5.0"]` (control ids), `status_code`, `status_detail` (the Befund),
  `resources[0].name|uid`, `severity`, `remediation.desc`, `finding_info.uid` (dedupe key).
- Prowler 5 emits no MANUAL rows; the 22 manual controls come from the catalog
  (`<prowler>/compliance/gcp/cis_5.0_gcp.json`, 93 requirements).
- Prowler's CIS 5.0 map does not carry `logging_sink_created` → 2.13; `check_overrides` fixes it.
- Prowler credits org-level alerting to child projects (`get_projects_covered_by_aggregated_metric`),
  so the central alerts pack is one measure for every project.
- Generated/fleet projects (identical picture in every control) are aggregated via
  `fleet_regex`; compute is scanned only on own projects.

## satz

- `report-compliance --prowler <ocsf>` ingests Prowler ≥ 4 OCSF; its own `remediation-plan`
  dossier reads Prowler-4 field names (`unmapped.check_id`, `cloud.project.uid`) and shows wrong
  titles/projects with Prowler 5 — read the OCSF directly (build_audit.py does).
- `require --format json` gives the goal view: verdict per control and `providers` (packs that
  would provide an unmet control). Every control in it and in `report-compliance --format json`
  carries `measures`: per claiming pack its `use` path, whether the estate includes it, the gcloud
  commands that meet (`gcloud`) and check (`gcloud_check`) the control without satz, and `risk`
  (what goes wrong without it). build_audit.py renders the plan's satz `use` lines, the gcloud
  alternative, the Prüfkommandos and "Risiko ohne Maßnahme" from them; `assets/measures.yaml`
  adds only what satz does not state.
- Identity: satz binds the estate's IaC SA; the ADC must be allowed to impersonate it. `satz whoami
  <estate>` before anything live. Switching customers = new ADC.
- Which pack covers which control is satz's data (`measures[].pack` / `use`), not a list here.
  Legacy→reset policy changes need `tofu apply -replace`.

## Documents

- Findings ≠ problems: 3777 findings on one org were eight measures. Say that in Ausgangslage.
- Never let another customer's name into a document; the verify script checks sibling folder
  names under `~/projects/ccc`, and the reference documents of other customers are not to be
  opened for content.
- 2.1 becomes a FALSE POSITIVE only after the org audit config is live (project view inherits);
  before that it is a real FAIL.
- Bucket lock (2.4) is a one-way door → "nach Freigabe", never in the Sofort phase.
