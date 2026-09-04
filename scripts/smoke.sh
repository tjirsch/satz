#!/usr/bin/env bash
# The smoke matrix: every estate-consuming command, end to end, against the
# fixture estate in tests/smoke/. Offline — no ADC, no org. What it proves is
# that the COMMANDS still run, which the unit tests (engines only) never did:
# two regressions shipped with a green test suite before this existed.
#
#   scripts/smoke.sh            # builds target/release/satz, then runs it
#   SATZ=path/to/satz scripts/smoke.sh   # …or run a binary you built yourself
#
# `tofu` is optional: with it on PATH the transpiled HCL is also `tofu validate`d
# (the provider is downloaded once; no state, no cloud).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# Build unconditionally. Building only when the binary was MISSING made this a
# false-verdict generator on every local run after a pull, a merge or an edit:
# the leftover binary from a previous run answered instead of the working tree,
# so a merged fix could read as a regression and — the dangerous direction — a
# broken tree could go green before a push. cargo is a no-op on a warm tree, so
# being unconditional costs nothing. An explicit SATZ means the caller built it
# and owns whether it is current; that path is never rebuilt.
if [ -n "${SATZ:-}" ]; then
  satz="$SATZ"
  [ -x "$satz" ] || { printf 'SATZ=%s is not an executable\n' "$satz" >&2; exit 1; }
else
  satz="$root/target/release/satz"
  (cd "$root" && cargo build --release --quiet --locked)
fi
# no GitHub update check per invocation (CI runners share the unauthenticated quota)
if [ ! -f "$HOME/.config/satz/satz.toml" ]; then
  mkdir -p "$HOME/.config/satz" && printf 'self_update_frequency = "never"\n' > "$HOME/.config/satz/satz.toml"
fi
cd "$root/tests/smoke"
rm -rf hcl tmp yaml/imported-*.satz yaml/discovered*.satz evidence
mkdir -p tmp

step() { printf '\n==> %s\n' "$*"; }
fail() { printf '\nSMOKE FAILED: %s\n' "$*" >&2; exit 1; }

step "transpile"
"$satz" --config . transpile smoke.satz
for f in main.tf providers.tf variables.tf terraform.tfvars; do
  [ -s "hcl/$f" ] || fail "hcl/$f missing or empty"
done
grep -q 'resource "google_org_policy_policy"' hcl/main.tf || fail "the CIS pack's policies are not in main.tf"
grep -q 'resource "google_cloud_identity_group_membership"' hcl/main.tf || fail "the group member was not emitted"
grep -q 'ignore_changes' hcl/main.tf || fail "group lifecycle default missing"

if command -v tofu >/dev/null 2>&1; then
  step "tofu validate"
  (cd hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color)
else
  step "tofu not on PATH — validate skipped"
fi

step "showcase: every language feature in one estate (the reference cites it)"
"$satz" --config . transpile showcase.satz --output "$PWD/tmp/showcase-hcl" >/dev/null
sc=tmp/showcase-hcl/main.tf
grep -q 'vmExternalIpAccess' "$sc" && fail "suppressed policy was emitted"
grep -q 'compute.managed.requireOsLogin' "$sc" || fail "policy from the \`as\` pack missing"
grep -q 'roles/browser' "$sc" && fail "suppressed role was emitted"
grep -q 'roles/iam.securityReviewer' "$sc" || fail "the member's other role vanished with the suppressed one"
grep -q 'audit-objects-only' "$sc" || fail "conditional grant missing"
grep -q 'trusted: reviewed' "$sc" || fail "hcl trust reason missing"
grep -q 'optional-001' "$sc" && fail "a \`when\`=false pack was pulled in"
grep -q 'corp-pack-bucket-001' "$sc" || fail "top-level pack resource missing"
grep -q 'location = "europe-west3"' "$sc" || fail "estate param did not override the pack default"
grep -q 'groups/01abcdef2ghijk3' tmp/showcase-hcl/imports.tf || fail "import-id did not reach imports.tf"
grep -q 'num_newer_versions' "$sc" || fail "list-of-objects lifecycle rules missing"
grep -q 'google_storage_bucket_iam_member' "$sc" || fail "bucket-scoped grant missing"
grep -q 'bucket = "corp-audit-logs-archive"' "$sc" || fail "the member-map form of a bucket-scoped grant did not reach main.tf"
[ "$(grep -c 'resource "google_storage_bucket_iam_member"' "$sc")" = 2 ] || fail "both bucket-scoped grant forms should emit one resource each"
"$satz" --config . require cis-gcp-4.0 showcase.satz > tmp/showcase-require.txt 2>&1 || true
grep -q 'DEVIATION' tmp/showcase-require.txt || fail "the deviates claim did not read as a deviation"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/showcase-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "showcase does not validate"
fi

step "require cis-gcp-4.0 (goal view, offline)"
# `require` exits non-zero when a technical control is unmet — that IS the CI
# gate; the smoke estate leaves 2.12 (DNS logging) and 2.13 (CAI) unmet on
# purpose, so the step asserts on the verdict line, not the exit code
"$satz" --config . require cis-gcp-4.0 smoke.satz > tmp/require.txt 2>&1 || true
cat tmp/require.txt
grep -q 'satisfied' tmp/require.txt || fail "require printed no verdict line"
# 11 unmet: 2.12 DNS logging and 2.13 CAI, which no pack covers, plus the nine
# controls the cis-extensions fragments cover and this estate does not turn on.
# The catalog carries the full CIS surface, so an unclaimed control is visible
# rather than absent — that is what makes the number meaningful.
grep -q '11 unmet' tmp/require.txt || fail "expected 2.12/2.13 plus the nine opt-in extension controls unmet:\n$(tail -3 tmp/require.txt)"

step "require cis-gcp-5.0: the same pack answers both benchmark versions"
"$satz" --config . require cis-gcp-5.0 smoke.satz > tmp/require-50.txt 2>&1 || true
grep -q 'satisfied' tmp/require-50.txt || fail "require printed no verdict line:\n$(cat tmp/require-50.txt)"
grep -q '0 broken claim' tmp/require-50.txt || fail "a 5.0 claim names a witness the estate does not emit:\n$(grep -i broken tmp/require-50.txt)"
# the renumbered controls resolve against the SAME resources as their 4.0 twins
grep -qE '✓ 1.5 .*iam_managed_disableServiceAccountKeyCreation' tmp/require-50.txt || fail "4.0 1.4 -> 5.0 1.5 did not carry over:\n$(grep ' 1.5 ' tmp/require-50.txt)"
grep -qE '✓ 1.6 .*preventPrivilegedBasicRoles' tmp/require-50.txt || fail "4.0 1.5 -> 5.0 1.6 did not carry over"
grep -qE '✓ 3.10 .*compute_requireVpcFlowLogs' tmp/require-50.txt || fail "4.0 3.8 -> 5.0 3.10 did not carry over"
# 1.2 keeps the 4.0 claim's duties, so it is partial rather than satisfied — the
# point is that it resolves at all, against the same policy
grep -qE '◐ 1.2 .*legacy-superseded' tmp/require-50.txt || fail "4.0 1.1 -> 5.0 1.2 did not carry over with its duties"
# 1.1.4 asks whether the org constrains its projects centrally: the whole baseline is the witness
grep -qE '◐ 1.1.4 .*review-baseline' tmp/require-50.txt || fail "the 5.0 §1.1.4 baseline claim is missing:\n$(grep '1.1.4' tmp/require-50.txt)"

step "superseded legacy constraints are declared OFF, never enforced beside their managed twin"
# The defect this prevents: both forms in force, so every exemption has to lift two
# policies. Absence would not prevent it — a legacy policy already set on the org is
# invisible to an apply that does not declare it — so the pack declares each twin reset.
for c in iam.allowedPolicyMemberDomains compute.requireOsLogin \
         essentialcontacts.allowedContactDomains iam.disableServiceAccountKeyCreation \
         iam.disableServiceAccountKeyUpload compute.restrictProtocolForwardingCreationForTypes; do
  python3 - "$c" <<'PY' || fail "the superseded twin is not declared off"
import re, sys
name = sys.argv[1]
tf = open("hcl/main.tf").read()
blocks = re.findall(r'resource "google_org_policy_policy" "[^"]+" \{.*?\n\}', tf, re.S)
mine = [b for b in blocks if f'/policies/{name}"' in b]
if len(mine) != 1:
    print(f"expected exactly one block for {name}, found {len(mine)}"); sys.exit(1)
b = mine[0]
if "reset = true" not in b:
    print(f"{name} is declared without reset = true:\n{b}"); sys.exit(1)
if "enforce" in b or "values" in b:
    print(f"{name} still carries an enforcing body:\n{b}"); sys.exit(1)
PY
done
# and the managed replacement IS enforcing, with its parameters
grep -q 'policies/compute.managed.restrictProtocolForwardingCreationForTypes"' hcl/main.tf \
  || fail "the managed protocol-forwarding constraint is missing"
grep -q 'parameters = "{\\"allowedSchemes\\":\[\\"INTERNAL\\"\]}"' hcl/main.tf \
  || fail "the managed protocol-forwarding constraint lost its parameters"

step "cis-extensions: opt-in coverage, off by default and on when asked"
# off by default: the baseline must not enforce any of them
grep -q 'compute.requireShieldedVm' hcl/main.tf && fail "an opt-in extension leaked into the baseline"
grep -q 'gcp.restrictNonCmekServices' hcl/main.tf && fail "an opt-in extension leaked into the baseline"
# and on when the estate asks. The three constraint SHAPES differ, and a wrong
# body is a policy that either does nothing or refuses everything — so assert
# each one, not just that something was emitted.
sed -e 's/^params {/params {\n  cis_require_shielded_vm = true\n  cis_cmek_required = true\n  cis_api_key_services = true\n  allowed_api_key_services = ["storage.googleapis.com"]/' yaml/smoke.satz > tmp/ext.satz
cat >> tmp/ext.satz <<'SATZ'
use "presets/cis-extensions/shielded-vm.satz" when cis_require_shielded_vm
use "presets/cis-extensions/cmek.satz" when cis_cmek_required
use "presets/cis-extensions/api-key-services.satz" when cis_api_key_services
SATZ
"$satz" --config . transpile tmp/ext.satz --output "$PWD/tmp/ext-hcl" > tmp/ext.txt 2>&1 || fail "the extensions do not transpile:\n$(cat tmp/ext.txt)"
grep -q 'name = "organizations/123456789012/policies/compute.requireShieldedVm"' tmp/ext-hcl/main.tf || fail "the plain boolean constraint is missing"
grep -q 'parameters = "{\\"allowedServices\\":\[\\"storage.googleapis.com\\"\]}"' tmp/ext-hcl/main.tf || fail "the parameterised constraint did not JSON-encode its parameters:\n$(grep -A3 disableServiceAccountApiKey tmp/ext-hcl/main.tf)"
grep -q 'denied_values' tmp/ext-hcl/main.tf || fail "the CMEK list constraint lost its values"
"$satz" --config . require cis-gcp-4.0 tmp/ext.satz > tmp/ext-require.txt 2>&1 || true
grep -q '0 broken claim' tmp/ext-require.txt || fail "an extension claims a witness it does not emit:\n$(grep -i broken tmp/ext-require.txt)"
grep -qE '✓ 4.8 ' tmp/ext-require.txt || fail "4.8 did not become satisfied with its fragment on"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/ext-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the extensions do not validate"
fi

step "require iso27001-2022 (cross-walk: ISO verdicts folded from the CIS ones)"
"$satz" --config . require iso27001-2022 smoke.satz > tmp/require-iso.txt 2>&1 || true
grep -q 'satisfied' tmp/require-iso.txt || fail "require printed no verdict line:\n$(cat tmp/require-iso.txt)"
# the fold reaches through: an ISO control with no claim of its own is satisfied
# by the CIS witnesses its evidence names
grep -qE '✓ A\.8\.3 .*google_org_policy_policy' tmp/require-iso.txt || fail "A.8.3 was not satisfied through its CIS evidence:\n$(grep 'A.8.3' tmp/require-iso.txt)"
# a duty named on the control caps it at partial even with the evidence green
grep -q '◐ A.5.3 .*role-matrix-reviewed' tmp/require-iso.txt || fail "a control duty did not cap the verdict:\n$(grep 'A.5.3' tmp/require-iso.txt)"
# 7.x is the provider's under shared responsibility, and never a gap
grep -q '◇ A.7.1 .*inherited from the provider' tmp/require-iso.txt || fail "physical controls must read as inherited"
[ "$(grep -c '◇' tmp/require-iso.txt)" = 14 ] || fail "all fourteen 7.x controls should be inherited"
[ "$(grep -cE '^  [✓◐○◇✗⚠]' tmp/require-iso.txt)" = 93 ] || fail "Annex A has 93 controls; the Statement of Applicability must list every one"

step "remediation-plan: the dossier + workbook, offline and deterministic"
rm -rf tmp/plan tmp/plan2
"$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out tmp/plan > tmp/plan.txt 2>&1 || fail "remediation-plan failed:\n$(cat tmp/plan.txt)"
for f in dossier.json findings.csv findings.xlsx meta.json; do [ -s "tmp/plan/$f" ] || fail "remediation-plan: $f missing or empty"; done
grep -q '"declared_address": "google_storage_bucket.state"' tmp/plan/dossier.json || fail "the bucket finding was not joined to its declaring block"
grep -q '^\[AI\] Recommended fix' <(head -1 tmp/plan/findings.csv | tr ',' '\n') || fail "the CSV lacks the [AI] columns"
"$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out tmp/plan2 >/dev/null 2>&1
h1=$(grep -o '"dossier_sha256": "[0-9a-f]*"' tmp/plan/meta.json); h2=$(grep -o '"dossier_sha256": "[0-9a-f]*"' tmp/plan2/meta.json)
[ "$h1" = "$h2" ] || fail "the dossier is not deterministic: $h1 vs $h2"

step "triage: Prowler FAILs sorted into buckets against the estate's claims"
"$satz" --config . triage cis-gcp-4.0 smoke.satz --prowler prowler.json > tmp/triage.md 2>tmp/triage.err || fail "triage failed:\n$(cat tmp/triage.err)"
grep -q '^## B ·' tmp/triage.md || fail "no bucket headings"
grep -q 'declared as `google_storage_bucket' tmp/triage.md || fail "the bucket finding was not matched to its declaring block:\n$(cat tmp/triage.md)"
"$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --prowler prowler.json --report tmp/ev2.md >/dev/null 2>&1 || fail "report-compliance --prowler failed"
grep -q 'FAIL' tmp/ev2.md || fail "the Prowler column is empty"

step "import-config: every derivable asset_type is filled (the CAI list is the source)"
cp "$root/presets/import-config.yaml" tmp/import-config.yaml
uv run --with ruamel.yaml "$root/scripts/update_import_config.py" --config-file tmp/import-config.yaml --cai-types "$root/presets/cai-asset-types.txt" | tee tmp/fill.txt
grep -q '^asset_type filled: 0;' tmp/fill.txt || fail "presets/import-config.yaml is behind presets/cai-asset-types.txt — run the fill and commit it"

step "pack docs are current (satz doc-packs --check) and every pack version has a changelog line"
"$satz" --config . doc-packs --check || fail "presets/docs is behind the packs — run \`satz doc-packs\` and commit"
for f in $(find "$root/presets" -name '*.satz' ! -name '*.local.satz' ! -name '*.diff.satz'); do
  line=$(grep -m1 -E '^pack ' "$f") || fail "$f has no pack header"
  name=$(printf '%s' "$line" | awk '{print $2}')
  ver=$(printf '%s' "$line" | grep -o 'version "[^"]*"' | cut -d'"' -f2)
  grep -q -E "^\| \`$name\` \| $ver \|" "$root/presets/CHANGELOG.md" || fail "presets/CHANGELOG.md has no row for \`$name\` $ver — add the version bump to the changelog"
done

step "check-presets against the repository's own presets (must be clean)"
"$satz" --config . check-presets --pristine-dir "$root/presets" smoke.satz

step "import, state shape"
"$satz" --config . import state.json -o imported-state.satz --verbose | tee tmp/import-state.txt
grep -q 'skipped' tmp/import-state.txt || fail "the skipped report did not print"
"$satz" --config . transpile imported-state.satz --output "$PWD/tmp/imported-state-hcl"
grep -q 'import {' tmp/imported-state-hcl/imports.tf || fail "state import produced no import blocks"

step "import, yaml shape (the legacy dialect converter)"
cp "$root/tests/corpus/yaml-estate/main.yaml" "$root/tests/corpus/yaml-estate/pack.yaml" tmp/
"$satz" --config . import tmp/pack.yaml --kind pack
"$satz" --config . import tmp/main.yaml --kind estate | tee tmp/import-yaml.txt
grep -q 'CONVERTED' tmp/import-yaml.txt || fail "yaml import did not report CONVERTED"

step "import, yaml shape — the fix is named when a pack is still YAML"
mkdir -p tmp/still && cp "$root/tests/corpus/yaml-estate/main.yaml" "$root/tests/corpus/yaml-estate/pack.yaml" tmp/still/
if "$satz" --config . import tmp/still/main.yaml --kind estate >tmp/still.txt 2>&1; then
  fail "an estate that still uses a YAML pack must be refused"
fi
grep -q 'convert them first' tmp/still.txt || fail "refusal did not name the packs to convert:\n$(cat tmp/still.txt)"

step "import, hcl shape (--wrap-all): every block verbatim, then transpile"
"$satz" --config . import tf --wrap-all -o imported-hcl.satz --verbose | tee tmp/import-hcl.txt
grep -q 'wrapped verbatim' tmp/import-hcl.txt || fail "hcl import printed no summary"
grep -q 'dropped .*provider' tmp/import-hcl.txt || fail "the provider block was not reported as dropped"
"$satz" --config . transpile imported-hcl.satz --output "$PWD/tmp/imported-hcl-hcl" 2>&1 | tee tmp/transpile-hcl.txt
grep -q 'resource "google_storage_bucket" "logs"' tmp/imported-hcl-hcl/main.tf || fail "the wrapped bucket did not reach main.tf"
grep -q 'raw HCL passthrough' tmp/transpile-hcl.txt || fail "passthrough blocks must be announced"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/imported-hcl-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color)
fi

step "import, hcl shape (translate): literal resources become Satz, positional ones wrap"
"$satz" --config . import tf -o imported-hcl2.satz --verbose | tee tmp/import-hcl2.txt
grep -q '7 block(s) translated' tmp/import-hcl2.txt || fail "folder, project, service, grants and buckets should translate:\n$(cat tmp/import-hcl2.txt)"
grep -q '2 promoted to params' tmp/import-hcl2.txt || fail "the variable and the locals block should be promoted, not wrapped:\n$(cat tmp/import-hcl2.txt)"
grep -q 'promoted .*locals' tmp/import-hcl2.txt || fail "the locals block was not reported as promoted"
grep -q '^google_folder {' yaml/imported-hcl2.satz || fail "no translated folder in the estate"
grep -q 'customer_organization_id = "123456789012"' yaml/imported-hcl2.satz || fail "the organisation id was not inferred"
grep -q '^  env = "prod"' yaml/imported-hcl2.satz || fail "the local did not become a param"
grep -q '^  bucket_suffix = "001"' yaml/imported-hcl2.satz || fail "the variable default did not become a param"
"$satz" --config . transpile imported-hcl2.satz --output "$PWD/tmp/imported-hcl2-hcl" 2>&1 | tee tmp/transpile-hcl2.txt
grep -q 'lifecycle_rule {' tmp/imported-hcl2-hcl/main.tf || fail "the translated bucket lost its lifecycle_rule"
grep -q 'name *= *"corp-logs-001"' tmp/imported-hcl2-hcl/main.tf || fail "the promoted param did not resolve back to the source's literal"
grep -q 'folder_id *= *google_folder.workloads.name' tmp/imported-hcl2-hcl/main.tf || fail "the project was not nested under its folder"
grep -q 'resource "google_project_service" "infra_iam_googleapis_com"' tmp/imported-hcl2-hcl/main.tf || fail "the service did not become the project's"
grep -q 'resource "google_organization_iam_member"' tmp/imported-hcl2-hcl/main.tf || fail "the org grant was not emitted"
grep -q 'resource "google_storage_bucket_iam_member" "logs_reader"' tmp/imported-hcl2-hcl/main.tf || fail "a bucket grant must translate as a labelled resource, not a member map"
grep -q 'bucket *= *"\?\${google_storage_bucket.logs.name}' tmp/imported-hcl2-hcl/main.tf || fail "the verbatim \${...} reference did not survive the round trip"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/imported-hcl2-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color)
fi

if command -v checkov >/dev/null 2>&1 || command -v uvx >/dev/null 2>&1; then
  step "scan: Checkov over the transpiled estate, findings pointed at the Satz source"
  "$satz" --config . transpile smoke.satz >/dev/null
  "$satz" --config . scan smoke.satz > tmp/scan.txt 2>&1 || true
  grep -q '^scan: Checkov' tmp/scan.txt || fail "scan printed no summary:\n$(cat tmp/scan.txt)"
  grep -q 'declared at' tmp/scan.txt || fail "findings were not pointed at the Satz source:\n$(cat tmp/scan.txt)"
  "$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --checkov --report tmp/evidence.md >/dev/null 2>&1 || fail "report-compliance --checkov failed"
  grep -q '| Checkov |' tmp/evidence.md || fail "the evidence report has no Checkov column"
else
  step "neither checkov nor uvx on PATH — scan skipped"
fi

step "adopt, offline dry run must refuse without ADC rather than guess (folders AND projects)"
for t in google_folder google_project; do
  if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . adopt smoke.satz --only $t >tmp/adopt.txt 2>&1; then
    fail "adopt --only $t must not succeed without credentials — a project is a live existence check, never a derived id:\n$(cat tmp/adopt.txt)"
  fi
  grep -qi 'credential\|token\|auth\|ADC' tmp/adopt.txt || fail "adopt --only $t failed for a reason other than credentials:\n$(cat tmp/adopt.txt)"
done

step "bootstrap --dry-run: offline-safe, the plan prints, the skipped pre-flight is NAMED"
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap smoke.satz --dry-run > tmp/boot-dry.txt 2>&1 \
  || fail "bootstrap --dry-run must exit 0 without credentials:\n$(cat tmp/boot-dry.txt)"
grep -q -- '--- Bootstrap Plan ---' tmp/boot-dry.txt || fail "the plan did not print:\n$(cat tmp/boot-dry.txt)"
grep -q 'pre-flight: SKIPPED' tmp/boot-dry.txt || fail "a pre-flight that did not run must say so, never pass silently:\n$(cat tmp/boot-dry.txt)"

step "help fits the terminal: no line wider than the width, globals under their own heading"
# clap reads the tty width; there is none in CI, so COLUMNS pins it
long=$(COLUMNS=80 "$satz" adopt --help 2>&1 | awk 'length > 80' | head -3)
[ -z "$long" ] || fail "adopt --help at 80 columns has lines wider than 80:\n$long"
COLUMNS=80 "$satz" adopt -h 2>&1 | grep -q '^Global options:' || fail "the global options are not under their own heading"
COLUMNS=80 "$satz" import -h 2>&1 | grep -q "see more with '--help'" || fail "-h did not become the short form (no summary/details split?)"

step "transpile --check: compiles in memory, accepts the yaml/-prefixed form, writes nothing"
rm -rf hcl
"$satz" --config . transpile yaml/smoke.satz --check > tmp/check.txt
grep -q 'transpile --check: OK' tmp/check.txt || fail "--check did not report OK:\n$(cat tmp/check.txt)"
[ ! -d hcl ] || fail "--check wrote hcl/"
"$satz" --config . transpile smoke.satz >/dev/null   # restore hcl/ for the later steps

step "a reference to a resource the estate does not emit is refused, naming the labels that exist"
cat > tmp/badref.satz <<'EOF'
estate badref

params {
  customer_organization_id = "123456789012"
  billing_account_infra    = "012345-6789AB-CDEF01"
}

terraform { backend { local { path = "terraform.tfstate" } } }
providers { google { alias = "google" } }

google_storage_bucket {
  logs {
    name     = "corp-logs-001"
    location = "EU"
  }
}

google_storage_bucket_iam_member {
  bucket = "${{google_storage_bucket.lugs.name}}"
  "group:gcp-auditors@example.com" = [ "roles/storage.objectViewer" ]
}
EOF
"$satz" --config . transpile tmp/badref.satz --check > tmp/badref.txt 2>&1 && fail "a typo'd reference must not compile:\n$(cat tmp/badref.txt)"
grep -q 'does not emit' tmp/badref.txt || fail "the reference check did not fire:\n$(cat tmp/badref.txt)"
grep -q 'google_storage_bucket.lugs.name' tmp/badref.txt || fail "the error does not name the bad reference"
grep -q 'emitted `google_storage_bucket` labels: logs' tmp/badref.txt || fail "the error does not name the labels that do exist:\n$(cat tmp/badref.txt)"

step "generate-migration: the script cds into hcl_dir, paces, retries 429s and summarizes"
printf 'google_project.old: google_project.new\n' > tmp/mapping.yaml
"$satz" --config . generate-migration tmp/mapping.yaml --output tmp/migrate.sh
grep -q 'cd "$HCL_DIR"' tmp/migrate.sh || fail "the script does not cd into hcl_dir"
grep -q "mv_state 'google_project.old' 'google_project.new'" tmp/migrate.sh || fail "the move is missing"
grep -q 'sleep 1' tmp/migrate.sh || fail "no pacing between moves"
grep -q "grep -q '429'" tmp/migrate.sh || fail "no 429 retry"
grep -q 'state moves:' tmp/migrate.sh || fail "no summary line"

step "bootstrap on a greenfield estate: the empty org id gets the greenfield guidance, not a bare error"
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap greenfield.satz --dry-run > tmp/boot-green.txt 2>&1; then
  fail "an empty customer_organization_id without --greenfield must fail:\n$(cat tmp/boot-green.txt)"
fi
grep -q -- '--greenfield' tmp/boot-green.txt || fail "the failure did not explain --greenfield:\n$(cat tmp/boot-green.txt)"
grep -q 'init --from-live' tmp/boot-green.txt || fail "the failure did not name init --from-live:\n$(cat tmp/boot-green.txt)"

step "whoami: refuses without credentials naming the fix; reads an impersonated-SA ADC offline"
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" whoami --offline > tmp/who.txt 2>&1; then
  fail "whoami --offline must fail without an ADC file:\n$(cat tmp/who.txt)"
fi
grep -q 'application-default login' tmp/who.txt || fail "whoami failure did not name the fix:\n$(cat tmp/who.txt)"
printf '{"type":"impersonated_service_account","service_account_impersonation_url":"https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/svc-iac@acme-infra-001.iam.gserviceaccount.com:generateAccessToken","quota_project_id":"acme-infra-001"}\n' > tmp/adc.json
GOOGLE_APPLICATION_CREDENTIALS="$PWD/tmp/adc.json" "$satz" whoami --offline > tmp/who2.txt 2>&1 \
  || fail "whoami --offline failed on a valid impersonated-SA ADC:\n$(cat tmp/who2.txt)"
grep -q 'svc-iac@acme-infra-001' tmp/who2.txt || fail "impersonation target not shown:\n$(cat tmp/who2.txt)"
grep -q 'impersonated service account' tmp/who2.txt || fail "credential type not shown:\n$(cat tmp/who2.txt)"
grep -q 'quota project acme-infra-001' tmp/who2.txt || fail "quota project not shown:\n$(cat tmp/who2.txt)"

step "the privacy gate judges tokens, not lines, and refuses an unusable range"
# the private-looking address is assembled at runtime so the fixture itself
# never carries a domain the gate would reject
printf 'contact ops@example.com or admin@%s.%s\n' "corp-private-host" "de" > tmp/leak.txt
if bash "$root/scripts/check-names.sh" tmp/leak.txt >tmp/gate.txt 2>&1; then fail "an allowed address on the same line shielded a private one"; fi
grep -q 'corp-private-host' tmp/gate.txt || fail "the gate did not name the private address:\n$(cat tmp/gate.txt)"
if bash "$root/scripts/check-names.sh" tmp/does-not-exist.txt >/dev/null 2>&1; then fail "a missing file passed the gate"; fi
if bash "$root/scripts/check-names.sh" --commits deadbeef..HEAD >tmp/gate2.txt 2>&1; then fail "an unusable commit range passed the gate"; fi

step "the privacy gate catches customer identifiers and lets vendor defaults through"
# Assembled at runtime so the fixture itself never carries a value the gate rejects.
pid="$(printf '%s-prod-infra-01' "kunde")"
{
  printf 'tenant = "%s-3d8e-4a56-9b1f-2c4d6e8a0b3c"\n' "7f9c2b41"
  printf 'pool   = "%s3d8e4a569b1f2c4d6e8a0b3c"\n' "7f9c2b41"
  printf 'project = "%s"\n' "$pid"
  printf 'path: projects/%s\n' "$pid"
} > tmp/ids.txt
if bash "$root/scripts/check-names.sh" tmp/ids.txt >tmp/ids-out.txt 2>&1; then fail "customer identifiers passed the gate:\n$(cat tmp/ids-out.txt)"; fi
for want in 'GUID' '32 hex' "projects/$pid" "project = \"$pid\""; do
  grep -q -- "$want" tmp/ids-out.txt || fail "the gate did not report $want:\n$(cat tmp/ids-out.txt)"
done
# and the values that must NOT be rejected: a vendor default, an example customer's
# project, and a value too short to be a project id at all
{
  printf 'issuer = "https://sts.windows.net/33e01921-4d64-4f8c-a055-5bdaffd5e33d"\n'
  printf 'audience = "api://d17a7d74-7e73-4e7d-bd41-8d9525e86cab"\n'
  printf 'project = "acme-infra-001"\n'
  printf 'project = "p"\n'
} > tmp/ok.txt
bash "$root/scripts/check-names.sh" tmp/ok.txt >tmp/ok-out.txt 2>&1 \
  || fail "the gate rejected a vendor default or a documented example:\n$(cat tmp/ok-out.txt)"

step "documentation site renders (what pages.yml publishes)"
uv run --with markdown "$root/scripts/build-site.py" tmp/site >/dev/null || fail "scripts/build-site.py failed"
for f in index.html docs/satz-language.html presets/index.html; do [ -s "tmp/site/$f" ] || fail "site: $f missing"; done
grep -q 'href="docs/satz-language.html"' tmp/site/index.html || fail "site: README link to the language reference was not rewritten to HTML"

step "corpus + unit tests"
(cd "$root" && cargo test --workspace --quiet 2>&1 | tail -3)

rm -rf hcl tmp yaml/imported-*.satz yaml/discovered*.satz evidence
printf '\nsmoke: every command ran.\n'
