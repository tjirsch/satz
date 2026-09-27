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
  would provide an unmet control) — that is where "Pack stellt es bereit" comes from.
- Identity: satz binds the estate's IaC SA; the ADC must be allowed to impersonate it. `satz whoami
  <estate>` before anything live. Switching customers = new ADC.
- Packs that map to measures: `monitoring.organization_audit_logsink` (2.1/2.3/2.4),
  `monitoring.organization_cis_log_alerts_central` (2.5–2.12), `essential_contacts_organization`
  (1.17), `cis_extensions.*` (1.15, 2.4, 4.3, 4.8, 4.11, 6.5/6.7, 7.2/7.3/8.1),
  `CIS_GCP_Foundation_4_0` (org policies: 1.1.4, 1.2, 1.5, 1.6, 3.1, 3.10, 4.4–4.6, 4.9, 5.1, 5.2),
  `sa_security_audit` (audit identity). Legacy→reset policy changes need `tofu apply -replace`.

## Documents

- Findings ≠ problems: 3777 findings on one org were eight measures. Say that in Ausgangslage.
- Never let another customer's name into a document; the verify script checks sibling folder
  names under `~/projects/ccc`, and the reference documents of other customers are not to be
  opened for content.
- 2.1 becomes a FALSE POSITIVE only after the org audit config is live (project view inherits);
  before that it is a real FAIL.
- Bucket lock (2.4) is a one-way door → "nach Freigabe", never in the Sofort phase.
