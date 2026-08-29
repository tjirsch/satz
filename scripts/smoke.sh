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

step "require cis-gcp-4.0 (goal view, offline)"
"$satz" --config . require cis-gcp-4.0 smoke.satz | tee tmp/require.txt
grep -q 'satisfied' tmp/require.txt || fail "require printed no verdict line"

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

step "adopt, offline dry run must refuse without ADC rather than guess"
if "$satz" --config . adopt smoke.satz --only google_folder >tmp/adopt.txt 2>&1; then
  grep -q 'to import' tmp/adopt.txt || fail "adopt ran but printed no table"
else
  grep -qi 'credential\|token\|auth\|ADC' tmp/adopt.txt || fail "adopt failed for a reason other than credentials:\n$(cat tmp/adopt.txt)"
fi

step "corpus + unit tests"
(cd "$root" && cargo test --quiet 2>&1 | tail -3)

rm -rf hcl tmp yaml/imported-*.satz yaml/discovered*.satz evidence
printf '\nsmoke: every command ran.\n'
