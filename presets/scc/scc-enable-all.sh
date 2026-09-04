#!/usr/bin/env bash
# scc-enable-all.sh — turn every Security Command Center service on at the
# organization, and make everything below the organization INHERIT it.
#
# WHY THIS IS A SCRIPT AND NOT A PRESET
# -------------------------------------
# google/google-beta have no binding for securitycentermanagement's
# SecurityCenterService, so SCC module enablement (and tier activation) cannot
# be expressed in Terraform/OpenTofu at all.
#
# It lives beside a preset rather than in scripts/ so that `get-presets` ships
# it: scc-service-enablement.satz, in this directory, binds it as an `action`,
# and a pack whose script did not travel with it would be an action that cannot
# find what it runs.
#
# It calls the securitycentermanagement REST API directly rather than
# `gcloud scc manage`, because the SDK knows only 13 of the 17 services the API
# exposes: ARTIFACT_GUARD, ARTIFACT_ANALYSIS, AGENT_ENGINE_VULN_ASSESSMENT and
# EXTERNAL_EXPOSURE have no gcloud name at all, while the API sets them without
# complaint. gcloud is still used for credentials and for walking the hierarchy.
#
# Everything
# DOWNSTREAM of activation (custom modules, sources + source IAM, notification
# configs, BigQuery exports, mute configs, Security Posture) is codeable and
# belongs in a preset; this file covers only the part that has no resource.
#
# WHAT IT DOES
#   1. org pass         each service -> ENABLED   at organizations/<ID>, except
#                                       the two opt-in ones and the multicloud
#                                       connectors (see OPTIONAL_SERVICES)
#   2. descendant sweep each service -> INHERITED at every folder and project
#                                       under the org, so the org value is the
#                                       single source of truth and no local
#                                       override survives
#   3. optional         each module  -> INHERITED (clears per-module overrides)
#
# It does NOT activate a tier. Enabling a service on an org with no
# Premium/Enterprise subscription fails at the API and no flag here changes
# that — activate the tier first, then run this.
#
# SAFETY: dry run by default. Nothing is written until you pass --apply.

set -euo pipefail

# ---------------------------------------------------------------- defaults --
ORG=""
APPLY=0
DO_ORG=1
DO_DESCENDANTS=1
RESET_MODULES=0
WITH_MULTICLOUD=0
WITH_OPTIONAL=0
SERVICES_OVERRIDE=""
TARGETS_FILE=""
QUOTA_PROJECT=""

# Services gcloud knows about (SDK 580.0.0). Used only as a fallback when the
# org's own service list cannot be read — the live list is authoritative.
FALLBACK_GCP_SERVICES=(
  SECURITY_HEALTH_ANALYTICS
  EVENT_THREAT_DETECTION
  CONTAINER_THREAT_DETECTION
  VM_THREAT_DETECTION
  WEB_SECURITY_SCANNER
  CLOUD_RUN_THREAT_DETECTION
  VM_MANAGER
  GCE_VULNERABILITY_ASSESSMENT
  NOTEBOOK_SECURITY_SCANNER
  AGENT_ENGINE_THREAT_DETECTION
  AGENT_ENGINE_VULN_ASSESSMENT
  EXTERNAL_EXPOSURE
  ARTIFACT_ANALYSIS
  ARTIFACT_GUARD
)
# Multicloud connectors: only meaningful once an AWS/Azure connector exists.
# Skipped unless --with-multicloud, because they fail noisily otherwise.
MULTICLOUD_SERVICES=(
  VM_THREAT_DETECTION_AWS
  EC2_VULNERABILITY_ASSESSMENT
  AZURE_VULNERABILITY_ASSESSMENT
)
# OPTIONAL: everything else on the list is enabled by default, because a detector
# for a workload nobody runs costs nothing and finds nothing — and being ready
# beats hoping someone remembers to switch it on the day the first GKE cluster
# or Cloud Run service appears. These two are different in kind and need saying
# yes to:
#   WEB_SECURITY_SCANNER  actively CRAWLS the customer's web applications. That
#                         is a different consent from passive detection, and not
#                         one to give on a customer's behalf.
#   ARTIFACT_ANALYSIS     billed per image scan, so it is a cost decision rather
#                         than a security one.
OPTIONAL_SERVICES=(
  WEB_SECURITY_SCANNER
  ARTIFACT_ANALYSIS
)
# VM Manager is not a service you switch on HERE. SCC mirrors whether GCE's VM
# Manager is running, and the API answers "Invalid intended_enablement_state" —
# enable VM Manager in Compute and this follows. Still swept to INHERITED, which
# the API does accept.
NOT_ENABLEABLE_SERVICES=(
  VM_MANAGER
)

usage() {
  cat <<'USAGE'
Usage: presets/scc/scc-enable-all.sh --organization ORG_ID [options]

  --organization ID     numeric organization id (required)
  --apply               actually write; without it every call carries
                        validateOnly and nothing changes
  --services "a b c"    use this service list verbatim instead of discovering it
  --with-optional       also enable WEB_SECURITY_SCANNER (which actively crawls
                        the customer's web apps) and ARTIFACT_ANALYSIS (billed
                        per image scan). Everything else is on by default.
  --with-multicloud     include the AWS/Azure connector services
  --org-only            enable at the org, skip the descendant sweep
  --descendants-only    only sweep folders/projects to INHERITED
  --reset-modules       also set every MODULE to INHERITED (clears per-module
                        overrides; applies to the org pass and the sweep)
  --targets-file FILE   newline-separated folders/<id> and projects/<id> to
                        sweep, instead of walking the hierarchy
  --quota-project ID    project the API bills the call to; defaults to the
                        active gcloud project. securitycentermanagement
                        refuses a call without one.
  -h, --help            this text

Examples
  # see what would happen, change nothing
  presets/scc/scc-enable-all.sh --organization 123456789012

  # do it
  presets/scc/scc-enable-all.sh --organization 123456789012 --apply

  # or let the estate supply the organisation id: the pack beside this file
  # binds this script as an `action`, so satz builds the command line from
  # the estate's own params
  satz run-actions <estate>.satz              # print it, run nothing
  satz run-actions <estate>.satz --check      # the dry run above
  satz run-actions <estate>.satz --execute    # adds --apply
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --organization|--org) ORG="${2:-}"; shift 2 ;;
    --apply)              APPLY=1; shift ;;
    --services)           SERVICES_OVERRIDE="${2:-}"; shift 2 ;;
    --with-multicloud)    WITH_MULTICLOUD=1; shift ;;
    --with-optional)      WITH_OPTIONAL=1; shift ;;
    --org-only)           DO_DESCENDANTS=0; shift ;;
    --descendants-only)   DO_ORG=0; shift ;;
    --reset-modules)      RESET_MODULES=1; shift ;;
    --targets-file)       TARGETS_FILE="${2:-}"; shift 2 ;;
    --quota-project)      QUOTA_PROJECT="${2:-}"; shift 2 ;;
    -h|--help)            usage; exit 0 ;;
    *) echo "unknown flag: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$ORG" ]] || { echo "error: --organization is required" >&2; usage >&2; exit 2; }
ORG="${ORG#organizations/}"
[[ "$ORG" =~ ^[0-9]+$ ]] || { echo "error: organization id must be numeric, got '$ORG'" >&2; exit 2; }

# ------------------------------------------------------------- the API ------
# Calls go to securitycentermanagement REST, not to `gcloud scc manage`. The SDK
# knows only 13 of the 17 services this API exposes — ARTIFACT_GUARD,
# ARTIFACT_ANALYSIS, AGENT_ENGINE_VULN_ASSESSMENT and EXTERNAL_EXPOSURE have no
# gcloud name and are unreachable through it, though the API sets them happily.
# Going straight to the API also removes a translation step that was itself a
# bug: the API says SECURITY_HEALTH_ANALYTICS, the CLI wanted
# security-health-analytics, and discovery fed one to the other.
API="https://securitycentermanagement.googleapis.com/v1"

api_token() {
  gcloud auth application-default print-access-token 2>/dev/null \
    || gcloud auth print-access-token 2>/dev/null
}

# api_patch <resource-path> <query> <json-body> — echoes the response body and
# returns non-zero on a non-2xx answer, so the caller reports the API's own
# message rather than a generic failure.
api_patch() {
  local path="$1" query="$2" body="$3" out code
  out=$(curl -sS -w $'\n%{http_code}' -X PATCH \
          -H "Authorization: Bearer $TOKEN" \
          -H "x-goog-user-project: $QUOTA_PROJECT" \
          -H "Content-Type: application/json" \
          -d "$body" "$API/$path?$query" 2>&1) || { printf '%s' "$out"; return 1; }
  code=${out##*$'\n'}
  out=${out%$'\n'*}
  printf '%s' "$out"
  [[ "$code" =~ ^2 ]]
}

command -v gcloud >/dev/null || { echo "error: gcloud not on PATH" >&2; exit 2; }
command -v jq     >/dev/null || { echo "error: jq not on PATH" >&2; exit 2; }
command -v curl   >/dev/null || { echo "error: curl not on PATH" >&2; exit 2; }

# gcloud is still how we authenticate and how the hierarchy is walked; the SCC
# calls themselves go to the REST API (see "the API" below).
TOKEN=$(api_token)
[[ -n "$TOKEN" ]] || {
  echo "error: no access token — run 'gcloud auth application-default login'" >&2; exit 2; }
QUOTA_PROJECT="${QUOTA_PROJECT:-$(gcloud config get-value project 2>/dev/null)}"
[[ -n "$QUOTA_PROJECT" ]] || {
  echo "error: no quota project — pass --quota-project, or 'gcloud config set project'" >&2
  echo "       securitycentermanagement refuses a call without one." >&2; exit 2; }

# read_lines VAR_NAME  — mapfile replacement; bash 3.2 has no mapfile.
read_into() {
  local __name="$1" __line
  eval "$__name=()"
  while IFS= read -r __line; do
    [[ -n "$__line" ]] || continue
    eval "$__name+=(\"\$__line\")"
  done
}

# ------------------------------------------------------------- reporting ----
OK=0; FAILED=0
declare -a FAILURES=()

say()  { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }

# Turn a gcloud error into a sentence the operator can act on. These are the
# three failures this command actually produces in the field.
diagnose() {
  local err="$1"
  case "$err" in
    *PERMISSION_DENIED*|*"does not have permission"*)
      say "        -> caller lacks securitycentermanagement.securityCenterServices.update"
      say "           (roles/securitycenter.admin at the org, or the settings admin role)" ;;
    *constraint*|*allowedPolicyMemberDomains*|*allowedPolicyMembers*)
      say "        -> blocked by the CIS §1.1 domain/principal locks: enabling a"
      say "           service provisions a new SCC service agent and Google's"
      say "           auto-grant is refused. See the SCC section of presets/README.md" ;;
    *"not supported"*|*NOT_FOUND*|*"is not enabled"*|*subscription*|*tier*)
      say "        -> service unavailable at this org's SCC tier, or its API is off."
      say "           Tier activation is NOT scriptable here — activate, then re-run." ;;
  esac
}

# set_state <parent> <service> <ENABLED|INHERITED>
set_state() {
  local parent="$1" service="$2" state="$3" err q
  q="updateMask=intendedEnablementState"
  (( APPLY )) || q="$q&validateOnly=true"
  if err=$(api_patch "$parent/locations/global/securityCenterServices/$service" \
             "$q" "{\"intendedEnablementState\":\"$state\"}"); then
    say "    ok    $service -> $state"
    OK=$((OK+1))
  else
    say "    FAIL  $service -> $state"
    say "        ${err//$'\n'/$'\n'        }"
    diagnose "$err"
    FAILED=$((FAILED+1))
    FAILURES+=("$parent $service $state")
  fi
}

# reset_modules <parent> <service> — every module of the service to INHERITED,
# which is how a module says "whatever my parent says".
reset_modules() {
  local parent="$1" service="$2" res modules body err count q
  res="$parent/locations/global/securityCenterServices/$service"
  modules=$(curl -sS -H "Authorization: Bearer $TOKEN" \
              -H "x-goog-user-project: $QUOTA_PROJECT" "$API/$res" 2>/dev/null \
            | jq -r '(.modules // {}) | keys[]' 2>/dev/null || true)
  if [[ -z "$modules" ]]; then
    say "    --    $service: no modules reported"
    return
  fi
  count=$(printf '%s\n' "$modules" | grep -c . || true)
  body=$(printf '%s\n' "$modules" \
         | jq -R . | jq -s '{modules: (map({(.): {intendedEnablementState: "INHERITED"}}) | add)}')
  q="updateMask=modules"
  (( APPLY )) || q="$q&validateOnly=true"
  if err=$(api_patch "$res" "$q" "$body"); then
    say "    ok    $service: $count module(s) -> INHERITED"
    OK=$((OK+1))
  else
    say "    FAIL  $service: modules -> INHERITED"
    say "        ${err//$'\n'/$'\n'        }"
    diagnose "$err"
    FAILED=$((FAILED+1))
    FAILURES+=("$parent $service modules")
  fi
}

# ------------------------------------------------------- service discovery --
discover_services() {
  if [[ -n "$SERVICES_OVERRIDE" ]]; then
    printf '%s\n' $SERVICES_OVERRIDE
    return
  fi
  local live
  live=$(curl -sS -H "Authorization: Bearer $TOKEN" \
           -H "x-goog-user-project: $QUOTA_PROJECT" \
           "$API/organizations/$ORG/locations/global/securityCenterServices" 2>/dev/null \
         | jq -r '.securityCenterServices[]?.name // empty | split("/") | last' 2>/dev/null || true)
  if [[ -n "$live" ]]; then
    printf '%s\n' "$live"
    return
  fi
  say "note: could not read the org's service list; using the built-in list" >&2
  printf '%s\n' "${FALLBACK_GCP_SERVICES[@]}"
  if (( WITH_MULTICLOUD )); then printf '%s\n' "${MULTICLOUD_SERVICES[@]}"; fi
}

is_optional() {
  local s="$1" k
  for k in "${OPTIONAL_SERVICES[@]}"; do
    [[ "$s" == "$k" ]] && return 0
  done
  return 1
}

is_not_enableable() {
  local s="$1" k
  for k in "${NOT_ENABLEABLE_SERVICES[@]}"; do
    [[ "$s" == "$k" ]] && return 0
  done
  return 1
}

is_multicloud() {
  local s="$1" m
  for m in "${MULTICLOUD_SERVICES[@]}"; do
    [[ "$s" == "$m" ]] && return 0
  done
  return 1
}

# --------------------------------------------------- hierarchy discovery ----
# Emits folders/<id> and projects/<id> for everything under the org.
walk_folders() {
  local parent_flag="$1" parent_val="$2" f
  while IFS= read -r f; do
    [[ -n "$f" ]] || continue
    printf 'folders/%s\n' "$f"
    walk_folders --folder "$f"
  done < <(gcloud resource-manager folders list "$parent_flag=$parent_val" \
             --format="value(name)" 2>/dev/null | sed 's|^folders/||')
}

list_projects_under() {
  local parent_id="$1"
  gcloud projects list \
    --filter="parent.id=$parent_id AND lifecycleState=ACTIVE" \
    --format="value(projectId)" 2>/dev/null | sed 's|^|projects/|'
}

discover_targets() {
  if [[ -n "$TARGETS_FILE" ]]; then
    grep -v '^[[:space:]]*\(#\|$\)' "$TARGETS_FILE"
    return
  fi
  local folders f
  folders=$(walk_folders --organization "$ORG")
  [[ -n "$folders" ]] && printf '%s\n' "$folders"
  list_projects_under "$ORG"
  while IFS= read -r f; do
    [[ -n "$f" ]] || continue
    list_projects_under "${f#folders/}"
  done <<<"$folders"
}

# ------------------------------------------------------------------ main ----
read_into SERVICES < <(discover_services)
(( ${#SERVICES[@]} )) || { echo "error: no SCC services to act on" >&2; exit 1; }

say "organization : organizations/$ORG"
if (( APPLY )); then
  say "mode         : APPLY — writes"
else
  say "mode         : DRY RUN — validateOnly, nothing is written"
fi
say "services     : ${SERVICES[*]}"

if (( DO_ORG )); then
  step "1. organizations/$ORG — every service ENABLED"
  for s in "${SERVICES[@]}"; do
    if is_multicloud "$s" && (( ! WITH_MULTICLOUD )); then
      say "    skip  $s (multicloud connector; pass --with-multicloud to include)"
      continue
    fi
    if is_optional "$s" && (( ! WITH_OPTIONAL )); then
      say "    skip  $s (opt-in: active scanning or per-scan cost; --with-optional)"
      continue
    fi
    if is_not_enableable "$s"; then
      say "    skip  $s (not enableable here; SCC mirrors the underlying service)"
      continue
    fi
    set_state "organizations/$ORG" "$s" ENABLED
    if (( RESET_MODULES )); then reset_modules "organizations/$ORG" "$s"; fi
  done
fi

if (( DO_DESCENDANTS )); then
  step "2. folders and projects — every service INHERITED"
  read_into TARGETS < <(discover_targets)
  if (( ! ${#TARGETS[@]} )); then
    say "  no folders or projects found under the org (or listing was denied)"
  else
    say "  ${#TARGETS[@]} target(s)"
    for t in "${TARGETS[@]}"; do
      say "  $t"
      for s in "${SERVICES[@]}"; do
        if is_multicloud "$s" && (( ! WITH_MULTICLOUD )); then continue; fi
        if is_optional "$s" && (( ! WITH_OPTIONAL )); then continue; fi
        set_state "$t" "$s" INHERITED
        if (( RESET_MODULES )); then reset_modules "$t" "$s"; fi
      done
    done
  fi
fi

step "summary"
say "  ok: $OK   failed: $FAILED"
if (( FAILED )); then
  say "  failed calls:"
  printf '    %s\n' "${FAILURES[@]}"
fi
(( APPLY )) || say "  DRY RUN — re-run with --apply to write."
(( FAILED == 0 ))
