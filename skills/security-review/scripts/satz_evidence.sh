#!/usr/bin/env bash
# satz side of the evidence: report-compliance (live, read-only, + Checkov on hcl/), triage, require —
# then every workload beside the estate: its compile, its consumer check, Checkov over its HCL.
#   satz_evidence.sh --repo <estate-repo> --estate <file.satz> --ocsf <merged.ocsf.json> --out <evidence-dir> \
#                    [--workloads "<name>:<kind>:<folder> ..."] [--framework cis-gcp-5.0] [--no-live]
# Runs as the estate's IaC service account (satz binds it from the estate; operator ADC must be
# allowed to impersonate — check `satz whoami` first). report-compliance only READS (Cloud Asset
# Inventory); Checkov runs locally over the transpiled HCL.
# --workloads takes the `workloads_arg=` line discover_scope.py prints (kind `satz` or `hcl`, the
# folder relative to the customer folder — the parent of --repo). Per workload, offline:
#   satz  → `satz --config <folder> transpile <name>.satz --check`, then
#           `satz --config <repo> check-consumer <folder>/<hcl_dir> <estate>` over its generated HCL
#   hcl   → `satz --config <repo> check-consumer <folder> <estate>`
#   both  → `checkov -d <hcl> --framework terraform -o json` → checkov-<name>.json
# Everything lands in consumer-<name>.txt, one `== step` block each, closed by `exit: <code>`
# (4 = skipped, with the reason on the line before). build_audit.py reads these files.
set -u
REPO=""; ESTATE=""; OCSF=""; OUT=""; FW="cis-gcp-5.0"; LIVE=""; WORKLOADS=""
while [ $# -gt 0 ]; do case "$1" in
  --repo) REPO=$2; shift 2;; --estate) ESTATE=$2; shift 2;; --ocsf) OCSF=$2; shift 2;; --out) OUT=$2; shift 2;;
  --workloads) WORKLOADS=$2; shift 2;;
  --framework) FW=$2; shift 2;; --no-live) LIVE="--no-live"; shift;; *) echo "unknown arg $1" >&2; exit 2;; esac; done
[ -n "$REPO" ] && [ -n "$ESTATE" ] && [ -n "$OCSF" ] && [ -n "$OUT" ] || { echo "need --repo --estate --ocsf --out" >&2; exit 2; }
mkdir -p "$OUT"; OUT=$(cd "$OUT" && pwd); cd "$REPO" || exit 3
REPO=$(pwd); REPO_NAME=$(basename "$REPO"); CUST=$(dirname "$REPO")
echo "== whoami"; satz --config . whoami "$ESTATE" 2>&1 | grep -v "^Loaded"
echo "== report-compliance $FW (+prowler, +checkov)"
# ADR 0021 (satz 0.57+): every format writes exactly one file named by --out; nothing on stdout.
satz --config . report-compliance "$FW" "$ESTATE" --prowler "$OCSF" --checkov $LIVE --format json --out "$OUT/satz-report-$FW.json" 2>"$OUT/satz-report-$FW.stderr"
grep -v "^Loaded" "$OUT/satz-report-$FW.stderr" | tail -3
satz --config . report-compliance "$FW" "$ESTATE" --prowler "$OCSF" $LIVE --format markdown --out "$OUT/$FW-latest.md" >/dev/null 2>&1
echo "== triage"
satz --config . triage "$FW" "$ESTATE" --prowler "$OCSF" --format markdown --out "$OUT/satz-triage-$FW.md" 2>/dev/null
grep -E "^## " "$OUT/satz-triage-$FW.md"
echo "== require (goal view)"
satz --config . require "$FW" "$ESTATE" --format json --out "$OUT/satz-require-$FW.json" 2>/dev/null
python3 - "$OUT/satz-require-$FW.json" "$OUT/satz-report-$FW.json" <<'PY'
import json, sys, collections
r = json.load(open(sys.argv[1])); print("require:", collections.Counter(c["verdict"] for c in r["controls"]))
rep = json.load(open(sys.argv[2])); print("report:", collections.Counter(x["status"] for x in rep["rows"]), "live:", rep.get("live_status"))
PY

# ---- workloads: offline, from the customer folder so every path in a finding is <folder>/…, never absolute
step() { # file, title — opens a block
  printf '== %s\n' "$2" >> "$1"
}
done_step() { # file, code — closes it
  printf 'exit: %s\n' "$2" >> "$1"
}
cd "$CUST" || exit 3
for w in $WORKLOADS; do
  name=${w%%:*}; rest=${w#*:}; kind=${rest%%:*}; folder=${rest#*:}
  f="$OUT/consumer-$name.txt"; : > "$f"
  echo "== workload $name ($kind, $folder)"
  if [ ! -d "$folder" ]; then
    step "$f" "workload $folder"; echo "workload folder not found under the customer folder" >> "$f"; done_step "$f" 2
    tail -2 "$f"; continue
  fi
  case "$kind" in
    satz)
      hcl=$(grep -E '^\s*hcl_dir\s*=' "$folder/config.toml" 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/'); hcl=${hcl:-hcl}
      step "$f" "transpile --check ($folder/$name.satz)"
      (cd "$folder" && satz --config . transpile "$name.satz" --check) >> "$f" 2>&1; done_step "$f" $?
      check_dir="$folder/$hcl";;
    hcl) check_dir="$folder";;
    *) step "$f" "workload $name"; echo "unknown kind '$kind' (satz or hcl)" >> "$f"; done_step "$f" 2; tail -2 "$f"; continue;;
  esac
  step "$f" "check-consumer $check_dir"
  if [ -d "$check_dir" ]; then
    satz --config "$REPO_NAME" check-consumer "$check_dir" "$ESTATE" >> "$f" 2>&1; done_step "$f" $?
  else
    echo "$check_dir is not there — the project has not been transpiled; check-consumer skipped" >> "$f"; done_step "$f" 4
  fi
  step "$f" "checkov $check_dir -> checkov-$name.json"
  if [ ! -d "$check_dir" ]; then
    echo "$check_dir is not there; Checkov skipped" >> "$f"; done_step "$f" 4
  elif command -v checkov >/dev/null; then
    checkov -d "$check_dir" --framework terraform -o json --quiet > "$OUT/checkov-$name.json" 2>>"$f"; code=$?
    python3 - "$OUT/checkov-$name.json" >> "$f" <<'PY'
import json, sys
reports = json.load(open(sys.argv[1])); reports = reports if isinstance(reports, list) else [reports]
s = {k: sum((r.get("summary") or {}).get(k, 0) for r in reports) for k in ("passed", "failed", "skipped")}
print("checkov:", s)
PY
    done_step "$f" $code
  else
    echo "checkov not on PATH (uv tool install checkov); not run" >> "$f"; done_step "$f" 4
  fi
  grep -E "^(== |exit: |check-consumer: |transpile --check: |checkov: )" "$f"
done
ls -1 "$OUT"
