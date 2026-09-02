#!/usr/bin/env bash
# The smoke matrix: every estate-consuming command, end to end, against the
# fixture estate in tests/smoke/. Offline — no ADC, no org. What it proves is
# that the COMMANDS still run, which the unit tests (engines only) never did:
# two regressions shipped with a green test suite before this existed.
#
#   scripts/smoke.sh            # uses target/release/satz, builds it if missing
#   SATZ=path/to/satz scripts/smoke.sh
#
# `tofu` is optional: with it on PATH the transpiled HCL is also `tofu validate`d
# (the provider is downloaded once; no state, no cloud).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
satz="${SATZ:-$root/target/release/satz}"
if [ ! -x "$satz" ]; then
  (cd "$root" && cargo build --release --quiet)
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
"$satz" --config . require cis-gcp-4.0 showcase.satz > tmp/showcase-require.txt 2>&1 || true
grep -q 'DEVIATION' tmp/showcase-require.txt || fail "the deviates claim did not read as a deviation"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/showcase-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "showcase does not validate"
fi

step "require cis-gcp-4.0 (goal view, offline)"
"$satz" --config . require cis-gcp-4.0 smoke.satz | tee tmp/require.txt
grep -q 'satisfied' tmp/require.txt || fail "require printed no verdict line"

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
grep -q '6 block(s) translated' tmp/import-hcl2.txt || fail "folder, project, service, grants and bucket should translate:\n$(cat tmp/import-hcl2.txt)"
grep -q '^google_folder {' yaml/imported-hcl2.satz || fail "no translated folder in the estate"
grep -q 'customer_organization_id = "123456789012"' yaml/imported-hcl2.satz || fail "the organisation id was not inferred"
"$satz" --config . transpile imported-hcl2.satz --output "$PWD/tmp/imported-hcl2-hcl" 2>&1 | tee tmp/transpile-hcl2.txt
grep -q 'lifecycle_rule {' tmp/imported-hcl2-hcl/main.tf || fail "the translated bucket lost its lifecycle_rule"
grep -q 'folder_id *= *google_folder.workloads.name' tmp/imported-hcl2-hcl/main.tf || fail "the project was not nested under its folder"
grep -q 'resource "google_project_service" "infra_iam_googleapis_com"' tmp/imported-hcl2-hcl/main.tf || fail "the service did not become the project's"
grep -q 'resource "google_organization_iam_member"' tmp/imported-hcl2-hcl/main.tf || fail "the org grant was not emitted"
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

step "adopt, offline dry run must refuse without ADC rather than guess"
if "$satz" --config . adopt smoke.satz --only google_folder >tmp/adopt.txt 2>&1; then
  grep -q 'to import' tmp/adopt.txt || fail "adopt ran but printed no table"
else
  grep -qi 'credential\|token\|auth\|ADC' tmp/adopt.txt || fail "adopt failed for a reason other than credentials:\n$(cat tmp/adopt.txt)"
fi

step "bootstrap --dry-run: offline-safe, the plan prints, the skipped pre-flight is NAMED"
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap smoke.satz --dry-run > tmp/boot-dry.txt 2>&1 \
  || fail "bootstrap --dry-run must exit 0 without credentials:\n$(cat tmp/boot-dry.txt)"
grep -q -- '--- Bootstrap Plan ---' tmp/boot-dry.txt || fail "the plan did not print:\n$(cat tmp/boot-dry.txt)"
grep -q 'pre-flight: SKIPPED' tmp/boot-dry.txt || fail "a pre-flight that did not run must say so, never pass silently:\n$(cat tmp/boot-dry.txt)"

step "the privacy gate judges tokens, not lines, and refuses an unusable range"
# the private-looking address is assembled at runtime so the fixture itself
# never carries a domain the gate would reject
printf 'contact ops@example.com or admin@%s.%s\n' "corp-private-host" "de" > tmp/leak.txt
if bash "$root/scripts/check-names.sh" tmp/leak.txt >tmp/gate.txt 2>&1; then fail "an allowed address on the same line shielded a private one"; fi
grep -q 'corp-private-host' tmp/gate.txt || fail "the gate did not name the private address:\n$(cat tmp/gate.txt)"
if bash "$root/scripts/check-names.sh" tmp/does-not-exist.txt >/dev/null 2>&1; then fail "a missing file passed the gate"; fi
if bash "$root/scripts/check-names.sh" --commits deadbeef..HEAD >tmp/gate2.txt 2>&1; then fail "an unusable commit range passed the gate"; fi

step "documentation site renders (what pages.yml publishes)"
uv run --with markdown "$root/scripts/build-site.py" tmp/site >/dev/null || fail "scripts/build-site.py failed"
for f in index.html docs/satz-language.html presets/index.html; do [ -s "tmp/site/$f" ] || fail "site: $f missing"; done
grep -q 'href="docs/satz-language.html"' tmp/site/index.html || fail "site: README link to the language reference was not rewritten to HTML"

step "corpus + unit tests"
(cd "$root" && cargo test --workspace --quiet 2>&1 | tail -3)

rm -rf hcl tmp yaml/imported-*.satz yaml/discovered*.satz evidence
printf '\nsmoke: every command ran.\n'
