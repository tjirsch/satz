#!/usr/bin/env bash
# scc-enable-all.sh — turn every Security Command Center service on at the
# organization, and make everything below the organization INHERIT it.
#
# WHY THIS IS A SCRIPT AND NOT A PRESET
# -------------------------------------
# google/google-beta have no binding for securitycentermanagement's
# SecurityCenterService, so SCC module enablement (and tier activation) cannot
# be expressed in Terraform/OpenTofu at all — see CLAUDE.md #27. Everything
# DOWNSTREAM of activation (custom modules, sources + source IAM, notification
# configs, BigQuery exports, mute configs, Security Posture) is codeable and
# belongs in a preset; this file covers only the part that has no resource.
#
# WHAT IT DOES
#   1. org pass         each service -> ENABLED   at organizations/<ID>
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
ALL_SERVICES=0
SERVICES_OVERRIDE=""
TARGETS_FILE=""

# Services gcloud knows about (SDK 580.0.0). Used only as a fallback when the
# org's own service list cannot be read — the live list is authoritative.
FALLBACK_GCP_SERVICES=(
  security-health-analytics
  event-threat-detection
  container-threat-detection
  vm-threat-detection
  web-security-scanner
  cloud-run-threat-detection
  vm-manager
  gce-vulnerability-assessment
  notebook-security-scanner
  agent-engine-threat-detection
)
# Multicloud connectors: only meaningful once an AWS/Azure connector exists.
# Skipped unless --all-services, because they fail noisily otherwise.
MULTICLOUD_SERVICES=(
  vm-threat-detection-aws
  ec2-vulnerability-assessment
  azure-vulnerability-assessment
)
# Every name `gcloud scc manage services update` accepts (SDK 580.0.0). This is
# NOT the same set the API lists: the API knows services gcloud has no name for
# (ARTIFACT_GUARD, ARTIFACT_ANALYSIS, AGENT_ENGINE_VULN_ASSESSMENT and
# EXTERNAL_EXPOSURE on a live org in 2026-09), and passing one of those turns
# the whole run into a wall of "is not a valid service name". Discovery is
# intersected with this list and the remainder is REPORTED, not attempted.
# VM Manager is not a service you switch on HERE. SCC mirrors whether GCE's VM
# Manager is running, and the API answers ENABLED with
# "Invalid intended_enablement_state" — enable VM Manager in Compute and this
# follows. It is still swept to INHERITED below, which the API does accept.
NOT_ENABLEABLE_SERVICES=(
  vm-manager
)
SETTABLE_SERVICES=(
  "${FALLBACK_GCP_SERVICES[@]}"
  "${MULTICLOUD_SERVICES[@]}"
)

usage() {
  cat <<'USAGE'
Usage: scripts/scc-enable-all.sh --organization ORG_ID [options]

  --organization ID     numeric organization id (required)
  --apply               actually write; without it every call runs with
                        --validate-only and nothing changes
  --services "a b c"    use this service list verbatim instead of discovering it
  --all-services        include the AWS/Azure connector services
  --org-only            enable at the org, skip the descendant sweep
  --descendants-only    only sweep folders/projects to INHERITED
  --reset-modules       also set every MODULE to INHERITED (clears per-module
                        overrides; applies to the org pass and the sweep)
  --targets-file FILE   newline-separated folders/<id> and projects/<id> to
                        sweep, instead of walking the hierarchy
  -h, --help            this text

Examples
  # see what would happen, change nothing
  scripts/scc-enable-all.sh --organization 123456789012

  # do it
  scripts/scc-enable-all.sh --organization 123456789012 --apply
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --organization|--org) ORG="${2:-}"; shift 2 ;;
    --apply)              APPLY=1; shift ;;
    --services)           SERVICES_OVERRIDE="${2:-}"; shift 2 ;;
    --all-services)       ALL_SERVICES=1; shift ;;
    --org-only)           DO_DESCENDANTS=0; shift ;;
    --descendants-only)   DO_ORG=0; shift ;;
    --reset-modules)      RESET_MODULES=1; shift ;;
    --targets-file)       TARGETS_FILE="${2:-}"; shift 2 ;;
    -h|--help)            usage; exit 0 ;;
    *) echo "unknown flag: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$ORG" ]] || { echo "error: --organization is required" >&2; usage >&2; exit 2; }
ORG="${ORG#organizations/}"
[[ "$ORG" =~ ^[0-9]+$ ]] || { echo "error: organization id must be numeric, got '$ORG'" >&2; exit 2; }

command -v gcloud >/dev/null || { echo "error: gcloud not on PATH" >&2; exit 2; }
command -v jq     >/dev/null || { echo "error: jq not on PATH" >&2; exit 2; }

# bash 3.2 (the macOS default) errors on "${arr[@]}" for an empty array under
# `set -u`, so this is expanded through the ${arr[@]+…} guard at both use sites.
VALIDATE=()
(( APPLY )) || VALIDATE=(--validate-only)

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
  local parent="$1" service="$2" state="$3" err
  if err=$(gcloud scc manage services update "$service" \
             --parent="$parent" --enablement-state="$state" \
             ${VALIDATE[@]+"${VALIDATE[@]}"} --format=none 2>&1); then
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
  local parent="$1" service="$2" modules cfg err count m
  modules=$(gcloud scc manage services describe "$service" --parent="$parent" \
              --format=json 2>/dev/null \
            | jq -r '(.modules // {}) | keys[]' 2>/dev/null || true)
  if [[ -z "$modules" ]]; then
    say "    --    $service: no modules reported"
    return
  fi

  cfg=$(mktemp)
  count=0
  while IFS= read -r m; do
    [[ -n "$m" ]] || continue
    printf '%s:\n  intended_enablement_state: INHERITED\n' "$m" >>"$cfg"
    count=$((count+1))
  done <<<"$modules"

  if err=$(gcloud scc manage services update "$service" \
             --parent="$parent" --module-config-file="$cfg" \
             ${VALIDATE[@]+"${VALIDATE[@]}"} --format=none 2>&1); then
    say "    ok    $service: $count module(s) -> INHERITED"
    OK=$((OK+1))
  else
    say "    FAIL  $service: modules -> INHERITED"
    say "        ${err//$'\n'/$'\n'        }"
    diagnose "$err"
    FAILED=$((FAILED+1))
    FAILURES+=("$parent $service modules")
  fi
  rm -f "$cfg"
}

# ------------------------------------------------------- service discovery --
discover_services() {
  if [[ -n "$SERVICES_OVERRIDE" ]]; then
    printf '%s\n' $SERVICES_OVERRIDE
    return
  fi
  local live
  # The API answers SECURITY_HEALTH_ANALYTICS; the command takes
  # security-health-analytics. Feeding the API's spelling to gcloud verbatim
  # failed EVERY call on the first live run — the whole point of discovery,
  # inverted. Lowercase and hyphenate here, then keep only what gcloud accepts.
  live=$(gcloud scc manage services list --parent="organizations/$ORG" \
           --format=json 2>/dev/null \
         | jq -r '.[]?.name // empty | split("/") | last | ascii_downcase | gsub("_"; "-")' 2>/dev/null || true)
  if [[ -n "$live" ]]; then
    local keep="" skip="" svc
    for svc in $live; do
      if is_settable "$svc"; then keep="$keep$svc"$'\n'; else skip="$skip $svc"; fi
    done
    [[ -z "$skip" ]] || say "note: the org lists services this gcloud cannot set, skipping:$skip" >&2
    if [[ -n "$keep" ]]; then printf '%s' "$keep"; return; fi
    say "note: none of the discovered services is settable; using the built-in list" >&2
  fi
  say "note: could not read the org's service list; using the built-in list" >&2
  printf '%s\n' "${FALLBACK_GCP_SERVICES[@]}"
  if (( ALL_SERVICES )); then printf '%s\n' "${MULTICLOUD_SERVICES[@]}"; fi
}

is_settable() {
  local s="$1" k
  for k in "${SETTABLE_SERVICES[@]}"; do
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
  say "mode         : DRY RUN — --validate-only, nothing is written"
fi
say "services     : ${SERVICES[*]}"

if (( DO_ORG )); then
  step "1. organizations/$ORG — every service ENABLED"
  for s in "${SERVICES[@]}"; do
    if is_multicloud "$s" && (( ! ALL_SERVICES )); then
      say "    skip  $s (multicloud connector; pass --all-services to include)"
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
        if is_multicloud "$s" && (( ! ALL_SERVICES )); then continue; fi
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
