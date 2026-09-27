#!/usr/bin/env bash
# Read-only Prowler scan of one GCP organisation, ONE RUN PER SERVICE with a deadline.
#
#   prowler_scan.sh --org <org-id> --sa <iac-sa-email> --out <raw-dir> --prefix <shortcode> \
#                   [--own-projects "p1 p2 ..."] [--deadline 2700] [--parallel 6] [--services "..."]
#
# --own-projects: the customer's real projects — the infra project, the estate's own projects and
# every workload's Google project (the `own_projects=` line discover_scope.py prints holds all of
# them; merge_ocsf.py's own-project list completes it after a first run). Compute (CIS §4) is
# scanned only on these.
#
# Why per service: a monolithic `prowler gcp --organization-id` freezes on organisations with many
# projects (the Dataproc and Compute clients enumerate every project without a socket timeout —
# three lost runs on a 229-project org). One run per service with a kill deadline always yields
# usable OCSF; a service that hits the deadline is reported, not silently missing.
# Why compute only on own projects: the compute client is the slowest; generated/fleet projects
# show the same picture in every control and are aggregated anyway.
# Credentials: gcloud ADC of the operator, impersonating the estate's IaC SA (read-only use).
set -u
ORG=""; SA=""; OUT=""; PREFIX="scan"; OWN=""; DEADLINE=2700; PAR=6
SERVICES="apikeys artifacts bigquery cloudfunction cloudsql cloudstorage dataproc dns gcr gemini gke iam kms logging secretmanager"
while [ $# -gt 0 ]; do case "$1" in
  --org) ORG=$2; shift 2;; --sa) SA=$2; shift 2;; --out) OUT=$2; shift 2;; --prefix) PREFIX=$2; shift 2;;
  --own-projects) OWN=$2; shift 2;; --deadline) DEADLINE=$2; shift 2;; --parallel) PAR=$2; shift 2;;
  --services) SERVICES=$2; shift 2;; *) echo "unknown arg $1" >&2; exit 2;; esac; done
[ -n "$ORG" ] && [ -n "$SA" ] && [ -n "$OUT" ] || { echo "need --org --sa --out" >&2; exit 2; }
mkdir -p "$OUT"; command -v prowler >/dev/null || { echo "prowler not on PATH (uv tool install prowler)" >&2; exit 3; }

run_one() {  # name, then prowler args
  local name=$1; shift
  local log="$OUT/svc-$name.log"
  ( prowler gcp --organization-id "$ORG" --impersonate-service-account "$SA" --verbose "$@" \
      -M json-ocsf -o "$OUT" -F "$PREFIX-ocsf-$name" --no-banner --ignore-exit-code > "$log" 2>&1 &
    pid=$!
    ( sleep "$DEADLINE"; kill -9 $pid 2>/dev/null && echo "DEADLINE: killed after ${DEADLINE}s" >> "$log" ) & wd=$!
    wait $pid; echo "exit: $?" >> "$log"; kill $wd 2>/dev/null )
}

echo "org $ORG as $SA → $OUT (deadline ${DEADLINE}s, $PAR parallel)"
# fast services in one run, the slow ones one each
FAST=""; SLOW=""
for s in $SERVICES; do case $s in apikeys|artifacts|bigquery|cloudfunction|cloudsql|cloudstorage) FAST="$FAST $s";; *) SLOW="$SLOW $s";; esac; done
jobs_running() { jobs -rp | wc -l | tr -d ' '; }
[ -n "$FAST" ] && { run_one fast -s $FAST & }
for s in $SLOW; do
  while [ "$(jobs_running)" -ge "$PAR" ]; do sleep 15; done
  run_one "$s" -s "$s" &
done
wait
if [ -n "$OWN" ]; then
  echo "compute on own projects: $OWN"
  log="$OUT/svc-compute.log"
  prowler gcp --project-ids $OWN --impersonate-service-account "$SA" --verbose -s compute \
    -M json-ocsf -o "$OUT" -F "$PREFIX-ocsf-compute" --no-banner --ignore-exit-code > "$log" 2>&1
  echo "exit: $?" >> "$log"
else
  echo "NOTE: no --own-projects given — compute (CIS §4) not scanned; pass the own project ids (discover_scope.py: own_projects=, workload projects included) to scan it" >&2
fi
echo "--- summary"
for l in "$OUT"/svc-*.log; do n=$(basename "$l" .log); printf "%-22s %s\n" "$n" "$(grep -h 'exit:\|DEADLINE' "$l" | tr '\n' ' ')"; done
ls -1 "$OUT"/*.ocsf.json 2>/dev/null | sed 's|.*/||'
