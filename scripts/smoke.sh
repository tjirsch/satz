#!/usr/bin/env bash
# The smoke matrix: every estate-consuming command, end to end, against the
# fixture estate in tests/smoke/. Offline — no ADC, no org. It proves that the
# COMMANDS run, once each, at their core; the unit tests judge the engines and
# pin the fixed bugs.
#
#   scripts/smoke.sh            # builds target/release/satz, then runs it
#   SATZ=path/to/satz scripts/smoke.sh   # …or run a binary you built yourself
#   SMOKE_SKIP_CARGO_TEST=1 scripts/smoke.sh   # CI: the checks job runs cargo test
#
# `tofu` is optional: with it on PATH the transpiled HCL is also `tofu validate`d
# (no state, no cloud). Providers are downloaded once into TF_PLUGIN_CACHE_DIR.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# Always built, so the working tree answers rather than a leftover binary; cargo is a
# no-op on a warm tree. An explicit SATZ means the caller built it and owns whether it
# is current; that path is never rebuilt.
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
# Every `tofu init` below reads the providers from one cache instead of downloading
# them. Its directories have no lock file, and without the second variable tofu
# downloads again to check the cached copy; the lock files it writes here are scratch.
export TF_PLUGIN_CACHE_DIR="${TF_PLUGIN_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/satz-smoke/tofu-plugins}"
export TF_PLUGIN_CACHE_MAY_BREAK_DEPENDENCY_LOCK_FILE=true
mkdir -p "$TF_PLUGIN_CACHE_DIR"
cd "$root/tests/smoke"
rm -rf hcl tmp yaml/imported-*.satz yaml/identity-*.satz evidence
mkdir -p tmp

step() { printf '\n==> %s\n' "$*"; }
fail() { printf '\nSMOKE FAILED: %s\n' "$*" >&2; exit 1; }

# First, on the clean tree: every Satz file in the repository is in its canonical
# layout. Later steps write .satz files of their own under tests/smoke.
step "fmt --check: every Satz file in the repository is formatted; --stdin round-trips"
"$satz" fmt --check "$root/presets" "$root/tests" > tmp/fmt-check.txt 2>&1 \
  || fail "satz fmt --check: run \`satz fmt presets tests\` and commit:\n$(cat tmp/fmt-check.txt)"
grep -q 'fmt --check: OK' tmp/fmt-check.txt || fail "fmt --check did not report OK"
"$satz" fmt --stdin < yaml/showcase.satz > tmp/fmt-stdin.satz || fail "fmt --stdin failed"
cmp -s yaml/showcase.satz tmp/fmt-stdin.satz || fail "fmt --stdin changed an already formatted file"
printf 'a = "open' | "$satz" fmt --stdin > /dev/null 2> tmp/fmt-err.txt && fail "fmt --stdin accepted an unterminated string"
grep -q 'unterminated string' tmp/fmt-err.txt || fail "fmt did not name the parse error:\n$(cat tmp/fmt-err.txt)"

step "lsp: the language server answers an editor — diagnostics, completion, hover, definition, formatting"
python3 "$root/tests/smoke/lsp_client.py" "$satz" yaml/showcase.satz || fail "satz lsp did not answer as an editor expects"

step "init: the estate satz writes is in the canonical layout"
# init fetches the schema of every provider the config names, unless it is already
# there. Seeding the fixture keeps these steps offline — the fetch itself is
# `tofu providers schema`, which `update-schema` shares.
seed_schemas() {
  mkdir -p "$1/schemas"
  cp "$root/tests/schemas/google.json" "$1/schemas/google.json"
  cp "$root/tests/schemas/google.json" "$1/schemas/google-beta.json"
}
# the presets beside the estate carry the pack graph, which is where the menu comes from
rm -rf tmp/init && mkdir -p tmp/init && seed_schemas tmp/init && ln -s "$root/presets" tmp/init/presets
(cd tmp/init && "$satz" init --customer-id C0example --customer-shortname acme \
  --billing-account-infra 012345-6789AB-CDEF01 --default-region europe-west3 \
  --customer-organization-id 123456789012 --customer-domain example.com \
  --infra-project-name acme-infra-001 --infra-bucket-name acme-infra-state > ../init.txt 2>&1) \
  || fail "satz init failed:\n$(cat tmp/init.txt)"
"$satz" fmt --check tmp/init/satz/C0example.satz || fail "satz init wrote an estate that is not in the canonical layout"
# the estate directory init creates, and names in config.toml, is satz/
grep -qx 'yaml_dir = "satz"' tmp/init/config.toml && [ ! -e tmp/init/yaml ] \
  || fail "satz init must create satz/ and name it in config.toml:\n$(cat tmp/init/config.toml)"
# no --workload-folder-name: the organisation is the workload folder, published, nothing created
grep -q '^export "workload_folder" = "organizations/{customer_organization_id}"' tmp/init/satz/C0example.satz \
  && ! grep -q 'google_folder.workload_folder' tmp/init/satz/C0example.satz \
  || fail "satz init without a workload folder must export the organisation and declare no folder"
# init invents nothing: a value nobody supplied and nothing could derive is EMPTY.
rm -rf tmp/init-bare && mkdir -p tmp/init-bare && seed_schemas tmp/init-bare
(cd tmp/init-bare && GOOGLE_APPLICATION_CREDENTIALS=/nonexistent CLOUDSDK_CONFIG=/nonexistent \
  "$satz" init --customer-id C0bare > ../init-bare.txt 2>&1) \
  || fail "satz init without credentials must still write an estate:\n$(cat tmp/init-bare.txt)"
grep -qE '^  customer_organization_id += ""$' tmp/init-bare/satz/C0bare.satz \
  || fail "an organisation id nobody supplied must be empty, never a placeholder:\n$(grep organization tmp/init-bare/satz/C0bare.satz)"
grep -qE '^  first_admin += ""$' tmp/init-bare/satz/C0bare.satz \
  || fail "an admin nobody supplied must be empty:\n$(grep first_admin tmp/init-bare/satz/C0bare.satz)"
grep -q 'nothing could be derived' tmp/init-bare.txt \
  || fail "init must say that it derived nothing:\n$(cat tmp/init-bare.txt)"
# the pack menu is written commented out: an init estate compiles with no presets fetched
grep -q '// use "presets/estate-map.satz"' tmp/init/satz/C0example.satz \
  || fail "satz init wrote no pack menu — an estate nothing can add a pack to"
# with no presets there is no graph: no menu, and init says which two commands write it
if grep -q '// use "presets/estate-map.satz"' tmp/init-bare/satz/C0bare.satz; then
  fail "an init estate with no pack graph must carry no pack menu"
fi
grep -q 'satz get-presets`, then `satz merge-presets`, write the pack lines' tmp/init-bare.txt \
  || fail "init without a pack graph must say what writes the menu:\n$(cat tmp/init-bare.txt)"
if grep -qE '^use "presets/' tmp/init/satz/C0example.satz; then
  fail "an init estate must compile with no presets fetched — bootstrap is the next command"
fi

step "transpile, and transpile --check: in memory, the yaml/-prefixed form, nothing written"
"$satz" --config . transpile yaml/smoke.satz --check > tmp/check.txt
grep -q 'transpile --check: OK' tmp/check.txt || fail "--check did not report OK:\n$(cat tmp/check.txt)"
[ ! -d hcl ] || fail "--check wrote hcl/"
"$satz" --config . transpile smoke.satz
for f in main.tf providers.tf variables.tf terraform.tfvars; do
  [ -s "hcl/$f" ] || fail "hcl/$f missing or empty"
done
grep -q 'resource "google_org_policy_policy"' hcl/main.tf || fail "the CIS pack's policies are not in main.tf"
grep -q 'resource "google_cloud_identity_group_membership"' hcl/main.tf || fail "the group member was not emitted"
grep -q 'ignore_changes' hcl/main.tf || fail "group lifecycle default missing"
# the provenance line, added at write time: which satz emitted this, and from what
head -1 hcl/main.tf | grep -q 'Generated by satz v.*smoke.satz' \
  || fail "main.tf carries no provenance line naming its estate:\n$(head -1 hcl/main.tf)"

# Every project's provider alias is the estate's provider scoped to that project: its
# own project as the quota project, and the region the estate works in. A literal
# region put regional resources in Frankfurt whatever the estate said, and a central
# quota project had Google test the API on a project the resource is not in.
grep -q 'billing_project = "corp-log-infra-001"' hcl/providers.tf \
  || fail "the log project's alias does not bill to that project:\n$(cat hcl/providers.tf)"
[ "$(grep -c 'region = "europe-west3"' hcl/providers.tf)" = "$(grep -c '^provider "google' hcl/providers.tf)" ] \
  || fail "not every provider block works in the estate's region:\n$(cat hcl/providers.tf)"

if command -v tofu >/dev/null 2>&1; then
  step "tofu validate"
  (cd hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color)
else
  step "tofu not on PATH — validate skipped"
fi

step "showcase: every language feature in one estate (the reference cites it)"
"$satz" --config . transpile showcase.satz --output "$PWD/tmp/showcase-hcl" >/dev/null 2>tmp/showcase-transpile.err \
  || fail "the showcase does not compile:\n$(cat tmp/showcase-transpile.err)"
sc=tmp/showcase-hcl/main.tf
# The reference example puts every resource where it belongs: the pack's bucket is
# declared outside a project and names its own, so the compile has no scope to warn about.
grep -q 'sets no `project`' tmp/showcase-transpile.err \
  && fail "the showcase declares a resource outside a project without naming one:\n$(cat tmp/showcase-transpile.err)"
awk '/^resource "google_storage_bucket" "pack_bucket"/ { inside = 1 } inside && /^}/ { exit }
     inside && /project = "corp-infra-001"/ { found = 1 } END { exit !found }' "$sc" \
  || fail "the pack's bucket does not name its project"
grep -q 'vmExternalIpAccess' "$sc" && fail "suppressed policy was emitted"
grep -q 'compute.managed.requireOsLogin' "$sc" || fail "policy from the \`as\` pack missing"
grep -q 'roles/browser' "$sc" && fail "suppressed role was emitted"
grep -q 'roles/iam.securityReviewer' "$sc" || fail "the member's other role vanished with the suppressed one"
grep -q 'audit-objects-only' "$sc" || fail "conditional grant missing"
grep -q 'trusted: reviewed' "$sc" || fail "hcl trust reason missing"
grep -q 'showcase-action' "$sc" && fail "an action leaked into main.tf — nothing about an action is ever emitted"
grep -q 'showcase-step' "$sc" && fail "an action name leaked into main.tf"
grep -q 'optional-001' "$sc" && fail "a \`when\`=false pack was pulled in"
grep -q 'corp-pack-bucket-001' "$sc" || fail "top-level pack resource missing"
grep -q 'location = "europe-west3"' "$sc" || fail "estate param did not override the pack default"
grep -q 'groups/01abcdef2ghijk3' tmp/showcase-hcl/imports.tf || fail "import-id did not reach imports.tf"
grep -q 'num_newer_versions' "$sc" || fail "list-of-objects lifecycle rules missing"
grep -q 'google_storage_bucket_iam_member' "$sc" || fail "bucket-scoped grant missing"
grep -q 'bucket = "corp-audit-logs-archive"' "$sc" || fail "the member-map form of a bucket-scoped grant did not reach main.tf"
[ "$(grep -c 'resource "google_storage_bucket_iam_member"' "$sc")" = 2 ] || fail "both bucket-scoped grant forms should emit one resource each"
"$satz" --config . require cis-gcp-4.0 showcase.satz --format text --out tmp/showcase-require.txt 2>/dev/null || true
grep -q 'DEVIATION' tmp/showcase-require.txt || fail "the deviates claim did not read as a deviation"
grep -q '0 contradicted claim(s)' tmp/showcase-require.txt || fail "a claim contradicts its own witness in the showcase estate"
# a question reaches variables.tf as a description and nothing else
grep -q 'description = "Short name identifying this customer"' tmp/showcase-hcl/variables.tf \
  || fail "the question's prompt did not become the variable's description"
grep -q 'question' "$sc" && fail "a question must emit nothing into main.tf"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/showcase-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "showcase does not validate"
fi

step "interfaces: the showcase's exports as outputs.tf and one relocatable module per interface"
si=tmp/showcase-hcl/interfaces
for m in core audit archive-team; do
  for f in versions.tf main.tf outputs.tf README.md; do
    [ -f "$si/$m/$f" ] || fail "the showcase exports and interfaces/$m/$f was not written"
  done
done
[ -f tmp/showcase-hcl/outputs.tf ] || fail "the showcase exports and outputs.tf was not written"
grep -q 'infra_folder *= google_folder.infra.name' tmp/showcase-hcl/outputs.tf || fail "the root output does not name the folder:\n$(cat tmp/showcase-hcl/outputs.tf)"
grep -q 'output "archive_team__archive_project_id"' tmp/showcase-hcl/outputs.tf || fail "an interface's root output is not <interface>__<export>:\n$(cat tmp/showcase-hcl/outputs.tf)"
grep -q 'value *= "corp-infra-001"' $si/core/outputs.tf || fail "a written attribute is not a literal output:\n$(cat $si/core/outputs.tf)"
grep -q 'value *= data.google_active_folder.infra.name' $si/archive-team/outputs.tf || fail "a core export is missing from the team's module:\n$(cat $si/archive-team/outputs.tf)"
grep -q 'output "archive_project_number"' $si/core/outputs.tf && fail "the core module carries a team's export"
grep -q 'value *= data.google_project.archive.number' $si/archive-team/outputs.tf || fail "the team's project number is not looked up:\n$(cat $si/archive-team/outputs.tf)"
grep -q 'output "audit_bucket_name"' $si/archive-team/outputs.tf || fail "the team's module lacks the exports of the interface it uses:\n$(cat $si/archive-team/outputs.tf)"
grep -q '^| `audit_bucket_name` | audit |' $si/archive-team/README.md || fail "the README does not name the interface a value comes from:\n$(cat $si/archive-team/README.md)"
[ "$(grep -c 'output "audit__audit_bucket_name"' tmp/showcase-hcl/outputs.tf)" = 1 ] || fail "a used interface's export is not one root output:\n$(cat tmp/showcase-hcl/outputs.tf)"
grep -q '\.\./\|var\.\|terraform_remote_state\|backend' $si/*/*.tf && fail "an interface module reaches outside itself:\n$(cat $si/*/*.tf)"
for m in core audit archive-team; do
  [ "$(sed -n '/^## Exports/,/^## Capabilities/p' $si/$m/README.md | grep -c '^| `')" = "$(grep -c '^output ' $si/$m/outputs.tf)" ] || fail "the $m README does not list every output:\n$(cat $si/$m/README.md)"
done
grep -q 'value *= { "infra" = data.google_active_folder.infra.name }' $si/core/outputs.tf || fail "all google_folder is not a map of the folders keyed by label:\n$(cat $si/core/outputs.tf)"
grep -q 'value *= { "audit_logs" = "corp-audit-logs" }' $si/core/outputs.tf || fail "all google_storage_bucket does not leave out the private bucket:\n$(cat $si/core/outputs.tf)"
grep -q 'keys `infra`' $si/core/README.md || fail "the README does not list the map's keys:\n$(cat $si/core/README.md)"
grep -q 'private' tmp/showcase-hcl/main.tf && fail "private reached main.tf"
grep -q 'resource "google_storage_bucket" "pack_bucket"' tmp/showcase-hcl/main.tf || fail "a private bucket must still be emitted"
cp yaml/showcase.satz tmp/private-export.satz
printf '%s\n' 'export "pack_bucket" = "${{google_storage_bucket.pack_bucket.name}}"' >> tmp/private-export.satz
"$satz" --config . transpile tmp/private-export.satz --check > tmp/private-export.txt 2>&1 && fail "an export of a private resource compiled"
grep -q 'is marked `private = true`' tmp/private-export.txt || fail "the refusal does not name the private mark:\n$(cat tmp/private-export.txt)"
grep -q '^| `archive_project_id` | yes | `google_project_iam_member` |' $si/archive-team/README.md || fail "the README's capability table does not show the attach point:\n$(cat $si/archive-team/README.md)"
if command -v tofu >/dev/null 2>&1; then
  # moved away from the estate, a team's module still initialises and validates on its own
  rm -rf tmp/relocated && mkdir -p tmp/relocated/elsewhere && cp -R "$si/archive-team" tmp/relocated/elsewhere/satz-interface
  (cd tmp/relocated/elsewhere/satz-interface && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) \
    || fail "the archive-team module does not validate away from the estate"
fi
# an estate that exports nothing keeps neither file from an earlier run
cp yaml/showcase.satz tmp/no-exports.satz
python3 - tmp/no-exports.satz <<'PY'
import re, sys
p = sys.argv[1]
s = open(p, encoding="utf-8").read()
s = re.sub(r'(?ms)^interface "[^"]*" \{.*?^\}\n', "", s)
s = re.sub(r'(?m)^export .*\n', "", s)
open(p, "w", encoding="utf-8").write(s)
PY
"$satz" --config . transpile tmp/no-exports.satz --output "$PWD/tmp/showcase-hcl" >/dev/null 2>tmp/no-exports.err \
  || fail "the showcase without exports does not compile:\n$(cat tmp/no-exports.err)"
[ -e tmp/showcase-hcl/outputs.tf ] && fail "outputs.tf survived a transpile of an estate that exports nothing"
[ -e "$si" ] && fail "hcl/interfaces/ survived a transpile of an estate that exports nothing"

step "check-consumer: a team's attachment at an attach point passes, one elsewhere is refused at its line"
"$satz" --config . check-consumer consumer yaml/showcase.satz > tmp/consumer.txt 2>&1 && fail "check-consumer passed an attachment onto an export that takes none:\n$(cat tmp/consumer.txt)"
grep -q 'consumer/main.tf:16' tmp/consumer.txt || fail "the refusal does not name the team's file and line:\n$(cat tmp/consumer.txt)"
grep -q 'no attach point for `google_folder_iam_member`' tmp/consumer.txt || fail "the refusal does not say why:\n$(cat tmp/consumer.txt)"
grep -q 'archive_readers' tmp/consumer.txt && fail "the attachment at an attach point was refused:\n$(cat tmp/consumer.txt)"
mkdir -p tmp/consumer-ok && sed '/^# refused/,$d' consumer/main.tf > tmp/consumer-ok/main.tf
"$satz" --config . check-consumer tmp/consumer-ok yaml/showcase.satz > tmp/consumer-ok.txt 2>&1 || fail "check-consumer refused a team that attaches only at attach points:\n$(cat tmp/consumer-ok.txt)"

step "interface notice: bucket, topic, grant, notification, and the object that holds the interface"
cp yaml/smoke.satz tmp/notice.satz
cat >> tmp/notice.satz <<'SATZ'
use "presets/interface-notice.satz"
SATZ
"$satz" --config . transpile tmp/notice.satz --output "$PWD/tmp/notice-hcl" > tmp/notice.txt 2>&1 || fail "the interface notice pack does not transpile:\n$(cat tmp/notice.txt)"
grep -q 'resource "google_storage_notification" "interface_notice"' tmp/notice-hcl/main.tf || fail "no storage notification"
grep -q 'topic = "${google_pubsub_topic_iam_member.interface_notice_publisher.topic}"' tmp/notice-hcl/main.tf || fail "the notification does not wait for the grant:\n$(grep -A6 'google_storage_notification' tmp/notice-hcl/main.tf)"
grep -q 'content = "${jsonencode(local.satz_interface)}"' tmp/notice-hcl/main.tf || fail "the object does not carry the interface"
grep -q 'value *= "projects/corp-infra-001/topics/corp-satz-interface"' tmp/notice-hcl/interfaces/core/outputs.tf || fail "the topic is not a static export:\n$(cat tmp/notice-hcl/interfaces/core/outputs.tf)"
grep -q 'google_pubsub_subscription' tmp/notice-hcl/interfaces/core/README.md || fail "the README does not show the subscription to write"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/notice-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the interface notice does not validate"
fi

step "run-actions: declared, resolved, and never run without being asked"
# Plan mode is the default and must spawn nothing. The fixture script would print
# a line of its own if it ran, so its absence is the assertion.
"$satz" --config . run-actions showcase.satz > tmp/actions-plan.txt 2>&1 \
  || fail "run-actions (plan) failed"
grep -q '3 action(s) declared' tmp/actions-plan.txt || fail "every action should be collected"
grep -q 'optional-step' tmp/actions-plan.txt && fail "a \`when\`=false pack contributed an action"
grep -q 'from a pack' tmp/actions-plan.txt || fail "the pack-declared action is not reported as coming from a pack"
grep -q 'scripts/showcase-action.sh --organization 123456789012' tmp/actions-plan.txt \
  || fail "the estate's param did not reach the resolved command line"
# The extension decides the launcher: a .py action is spawned through uv, and the
# printed line is the one that runs.
grep -q 'uv run --script scripts/showcase-action.py --organization 123456789012' tmp/actions-plan.txt \
  || fail "a .py action is not shown as \`uv run --script\`"
grep -q -- '--apply' tmp/actions-plan.txt || fail "the --execute form should be shown in the plan"
grep -q 'showcase-action: ' tmp/actions-plan.txt && fail "plan mode spawned the script"
grep -q 'nothing was run' tmp/actions-plan.txt || fail "plan mode did not say it ran nothing"

# --check passes `args` only; --execute appends `execute_args`. The fixture prints
# WRITE MODE only when it sees --apply, so the two runs are told apart by effect.
"$satz" --config . run-actions showcase.satz --check > tmp/actions-check.txt 2>&1 \
  || fail "run-actions --check failed"
grep -q 'showcase-action: name=showcase-step phase=after-apply mode=check' tmp/actions-check.txt \
  || fail "the action did not run, or did not receive its environment"
grep -q 'WRITE MODE' tmp/actions-check.txt && fail "--check must not pass execute_args"
grep -q 'customer_domain=not-exported' tmp/actions-check.txt \
  || fail "a param the estate did not put in args reached the script's environment"
# The Python action ran through uv, in the same directory, with the same environment
# and the same --check contract. Its file carries no executable bit: uv reads it.
grep -q 'showcase-python: name=showcase-python-step phase=after-apply mode=check' tmp/actions-check.txt \
  || fail "the .py action did not run through uv, or did not receive its environment"
grep -q 'showcase-python: DRY RUN' tmp/actions-check.txt || fail "--check must not pass execute_args to a .py action"
[ -x scripts/showcase-action.py ] \
  && fail "the .py fixture is executable — a Python action runs without the bit, and this proves it"

"$satz" --config . run-actions showcase.satz --execute > tmp/actions-exec.txt 2>&1 \
  || fail "run-actions --execute failed"
grep -q 'showcase-action: WRITE MODE' tmp/actions-exec.txt || fail "--execute did not append execute_args"
grep -q 'showcase-python: WRITE MODE' tmp/actions-exec.txt \
  || fail "--execute did not append execute_args to the .py action"

# The switches.
"$satz" --config . run-actions showcase.satz --execute --no-actions > tmp/actions-off.txt 2>&1 \
  || fail "run-actions --no-actions failed"
grep -q 'showcase-action: ' tmp/actions-off.txt && fail "--no-actions still executed something"
grep -q 'nothing was run: --no-actions' tmp/actions-off.txt || fail "--no-actions did not report itself"

"$satz" --config . run-actions showcase.satz --no-pack-actions > tmp/actions-nopack.txt 2>&1 \
  || fail "run-actions --no-pack-actions failed"
grep -q '1 pack-declared action(s) skipped' tmp/actions-nopack.txt || fail "--no-pack-actions skipped nothing"

"$satz" --config . run-actions showcase.satz --only nope > tmp/actions-only.txt 2>&1 \
  && fail "--only with an unknown name should fail"
grep -q 'no action by that name' tmp/actions-only.txt || fail "--only did not name the mistake"

step "questions: what the estate can be asked, and what the answers cost"
"$satz" --config . questions showcase.satz --format text --out tmp/questions.txt 2>/dev/null || fail "satz questions failed"
grep -q 'customer_shortname' tmp/questions.txt || fail "the showcase's question is missing"
grep -q 'one-way' tmp/questions.txt || fail "a recreate-reversal question must be marked as a one-way door"
grep -q 'group_model' tmp/questions.txt || fail "the oneof question is missing"
"$satz" --config . questions showcase.satz --format json --out tmp/questions.json 2>/dev/null || true
python3 - <<'PYEOF' || fail "satz questions --format json did not emit parseable JSON"
import json
d = json.load(open("tmp/questions.json"))
subs = {q["subject"]: q for q in d["questions"]}
assert "customer_shortname" in subs, subs.keys()
assert subs["customer_shortname"]["reversal"] == "recreate", subs["customer_shortname"]
assert d["summary"]["one_way_doors"] >= 1, d["summary"]
m = subs["group_model"]
assert m["kind"] == "oneof" and len(m["options"]) == 2, m
assert sum(1 for o in m["options"] if o["selected"]) == 1, m
assert m.get("required") is True, m
x = subs["optional_extras"]
assert x["kind"] == "oneof" and len(x["options"]) == 1 and not x.get("required"), x
t = subs["team_folder_name"]
assert t["empty"] == "no team folder" and t["state"] == "answered" and t["current"] == "", t
# ANSWERED means the estate's own params bind it — the showcase binds all five
assert all(q["state"] == "answered" for q in d["questions"]), [(q["subject"], q["state"]) for q in d["questions"]]
assert d["summary"]["complete"] is True and d["summary"]["unanswered"] == 0, d["summary"]
PYEOF
grep -q 'satz v' tmp/questions.json && fail "the version banner is on stdout"
"$satz" --config . questions showcase.satz --unanswered --format text --out tmp/questions-open.txt 2>/dev/null || fail "questions --unanswered failed"
if grep -q 'customer_shortname' tmp/questions-open.txt; then fail "an answered question must not be listed under --unanswered"; fi
"$satz" --config . questions showcase.satz --format markdown --out tmp/decisions.md 2>/dev/null || fail "questions --format markdown failed"
grep -q 'All 5 questions are answered' tmp/decisions.md || fail "the decisions sheet must say the showcase is complete"

step "interview: a skeleton, piped answers, derived defaults, the gate, and the decisions sheet"
# The third way to start an estate. `init` takes every answer as a flag; this asks.
# A DAY-0 file: the scaffold and nothing else, so the questions are the estate's own
# eighteen. Piped input: accept the opening offer, type the values nobody can default,
# then Enter for each name that became an offer once its inputs landed.
rm -rf tmp/iv && mkdir -p tmp/iv
printf '%s\n' y C0example 123456789012 example.com acme Acme first.admin 012345-6789AB-CDEF01 '' '' '' '' '' '' '' '' '' '' \
  | "$satz" --config . interview "$PWD/tmp/iv/new.satz" --create > tmp/iv/run.txt 2>&1 \
  || fail "satz interview failed:\n$(cat tmp/iv/run.txt)"
grep -q 'accepted 9 default(s)' tmp/iv/run.txt || fail "a day-0 file offers nine defaults, not the whole library's:\n$(cat tmp/iv/run.txt)"
# accepting `workload_folder_name = ""` writes the section that publishes the organisation
grep -q '^export "workload_folder" = "organizations/{customer_organization_id}"' tmp/iv/new.satz \
  || fail "answering the workload folder must write its export:\n$(tail -5 tmp/iv/new.satz)"
# every pack is commented out, and estate-core is the only `use` that is not
[ "$(grep -c '^use \"presets' tmp/iv/new.satz)" = 1 ] \
  || fail "a day-0 estate uses exactly one pack (estate-core):\n$(grep '^use \"presets' tmp/iv/new.satz)"
grep -q '^// use \"presets/estate-map.satz\"' tmp/iv/new.satz \
  || fail "the map must be written commented — it is what asks which packs the estate has"
grep -q '^// use \"presets/scc/scc-export.satz\" when use_scc_export' tmp/iv/new.satz \
  || fail "every optional pack must be written commented, under its phase"
grep -q '\[acme-infra-001\]' tmp/iv/run.txt || fail "the project id must be OFFERED once the short name is typed — before, it is not a default"
grep -q 'complete — every question is answered' tmp/iv/run.txt || fail "the interview did not end complete:\n$(cat tmp/iv/run.txt)"
grep -q 'would have named this file C0example.satz' tmp/iv/run.txt || fail "the rename hint is missing"
# `+=`: the interview keeps a formatted file formatted, so `=` is aligned across the params block
grep -qE 'customer_shortname += "acme"' tmp/iv/new.satz || fail "the answer was not written into params"
grep -qE 'security_model_s1 += true' tmp/iv/new.satz && fail "a day-0 file does not answer the map's choices — the map is not in it yet"
"$satz" --config . transpile "$PWD/tmp/iv/new.satz" --check > tmp/iv/check.txt 2>&1 || fail "the interviewed estate does not compile:\n$(cat tmp/iv/check.txt)"
"$satz" fmt --check "$PWD/tmp/iv/new.satz" || fail "the interview left the skeleton unformatted — an edit keeps a formatted file formatted"
# THE GATE. An estate with an open question is refused by apply and by bootstrap;
# a dry run warns — looking is how you find out.
"$satz" --config . interview "$PWD/tmp/iv/open.satz" --create < /dev/null > /dev/null 2>&1 || fail "--create with no input must still write the skeleton"
"$satz" fmt --check "$PWD/tmp/iv/open.satz" || fail "the skeleton is not in the canonical layout"
if "$satz" --config . transpile "$PWD/tmp/iv/open.satz" --apply --output "$PWD/tmp/iv/open-hcl" > tmp/iv/apply.txt 2>&1; then
  fail "apply on an unanswered estate was not refused"
fi
grep -q 'apply refused: 18 question(s) unanswered' tmp/iv/apply.txt || fail "the refusal must count the open questions:\n$(cat tmp/iv/apply.txt)"
grep -q 'customer_id (needs a value)' tmp/iv/apply.txt || fail "the refusal must say which need a typed value"
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap "$PWD/tmp/iv/open.satz" > tmp/iv/boot.txt 2>&1; then
  fail "bootstrap on an unanswered estate was not refused"
fi
grep -q 'bootstrap refused' tmp/iv/boot.txt || fail "bootstrap must refuse before it does anything else:\n$(cat tmp/iv/boot.txt)"
sed '/default_zone/d' tmp/iv/new.satz > tmp/iv/almost.satz
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap "$PWD/tmp/iv/almost.satz" --dry-run > tmp/iv/dry.txt 2>&1 \
  || fail "bootstrap --dry-run must warn, not refuse:\n$(cat tmp/iv/dry.txt)"
grep -q 'warning: bootstrap refused: 1 question(s) unanswered — default_zone' tmp/iv/dry.txt || fail "the dry run must warn naming the open question:\n$(cat tmp/iv/dry.txt)"
"$satz" --config . questions "$PWD/tmp/iv/almost.satz" --format markdown --out tmp/iv/decisions.md 2>/dev/null || fail "decisions sheet failed"
grep -q '1 of 18 questions are still open' tmp/iv/decisions.md || fail "the sheet must count what is open:\n$(cat tmp/iv/decisions.md)"
# the workbook a customer fills in and sends back
"$satz" --config . questions "$PWD/tmp/iv/almost.satz" --format xlsx --out "$PWD/tmp/iv/decisions.xlsx" > /dev/null 2>tmp/iv/xlsx.txt \
  || fail "the catalog workbook was not written:\n$(cat tmp/iv/xlsx.txt)"
[ -s tmp/iv/decisions.xlsx ] || fail "the catalog workbook is empty"

# the exclusive choice is checked BEFORE the fold reaches a shared address
sed 's/group_model_split        = false/group_model_split        = true/' yaml/showcase.satz > tmp/twochoice.satz
if "$satz" --config . transpile tmp/twochoice.satz --check >tmp/twochoice.txt 2>&1; then
  fail "two branches of a oneof were both accepted"
fi
grep -q 'group_model_flat and group_model_split are both true' tmp/twochoice.txt \
  || fail "the oneof refusal does not name both branches:\n$(cat tmp/twochoice.txt)"

# a used file is judged by where its `use` stands: a pack that declares its own types, used
# in the folder map, compiled into folders named after those types
{ cat yaml/smoke.satz; printf 'google_folder {\n  use "presets/cis/cmek.satz"\n}\n'; } > tmp/misplaced.satz
if "$satz" --config . transpile tmp/misplaced.satz --check >tmp/misplaced.txt 2>&1; then
  fail "a pack that declares its own types was accepted inside google_folder { … }"
fi
grep -q 'presets/cis/cmek.satz:[0-9]* does not belong there' tmp/misplaced.txt \
  || fail "the refusal does not name the entry in the used file:\n$(cat tmp/misplaced.txt)"

step "require cis-gcp-4.0 (goal view, offline): text, and json where the file carries the answer and the console nothing"
# `require` exits non-zero when a technical control is unmet, and the smoke estate
# leaves controls unmet, so the step asserts on the verdict, not the exit code.
"$satz" --config . require cis-gcp-4.0 smoke.satz --format text --out tmp/require.txt 2>/dev/null || true
grep -q 'satisfied' tmp/require.txt || fail "require printed no verdict line:\n$(cat tmp/require.txt)"
# The artefact is the file `--out` names, and stdout stays EMPTY so
# `--out /dev/stdout | jq` is a clean pipe; the wrote line and the banner are stderr.
"$satz" --config . require cis-gcp-4.0 smoke.satz --format json --out tmp/require.json > tmp/require-stdout.txt 2>tmp/require-stderr.txt || true
[ -s tmp/require-stdout.txt ] && fail "require printed to stdout: $(cat tmp/require-stdout.txt)"
grep -q "wrote tmp/require.json" tmp/require-stderr.txt || fail "the command did not say where it put the report:\n$(cat tmp/require-stderr.txt)"
# `--out -` is stdout on every platform: the report alone, the rest on stderr
"$satz" --config . require cis-gcp-4.0 smoke.satz --format json --out - > tmp/require-dash.json 2>tmp/require-dash-stderr.txt || true
cmp -s tmp/require.json tmp/require-dash.json || fail "--out - did not write the same report to stdout"
grep -q "wrote stdout" tmp/require-dash-stderr.txt || fail "--out - does not say it wrote stdout:\n$(cat tmp/require-dash-stderr.txt)"
[ -e ./- ] && fail "--out - wrote a file named -"
python3 - <<'PY' || fail "require --format json did not write parseable JSON"
import json, sys
d = json.load(open("tmp/require.json"))
assert d["catalog"] == "cis-gcp" and d["version"] == "4.0", d.get("catalog")
assert len(d["controls"]) > 20, len(d["controls"])
s = d["summary"]
# 14 unmet: 2.12 DNS logging and 2.13 CAI, which no pack covers, plus the twelve
# controls the CIS extension fragments cover and this estate does not turn on
assert s["unmet"] == 14, s
assert s["satisfied"] == 18, s
# every row carries a verdict from the closed set
verdicts = {c["verdict"] for c in d["controls"]}
assert verdicts <= {"satisfied","partial","broken","deviation","unmet","organizational","inherited"}, verdicts
PY
grep -q 'satz v' tmp/require.json && fail "the version banner reached the report file"
grep -q 'wrote ' tmp/require.json && fail "a progress line reached the report file"

step "require cis-gcp-5.0: the same pack answers both benchmark versions"
"$satz" --config . require cis-gcp-5.0 smoke.satz --format text --out tmp/require-50.txt 2>/dev/null || true
grep -q 'satisfied' tmp/require-50.txt || fail "require printed no verdict line:\n$(cat tmp/require-50.txt)"
grep -q '0 broken claim' tmp/require-50.txt || fail "a 5.0 claim names a witness the estate does not emit:\n$(grep -i broken tmp/require-50.txt)"
# CIS 5.0 §2.14 is the one control whose witness is the SCAFFOLD's, not a pack's: the
# infrastructure project enables the Cloud Asset API, so the estate satisfies it. The
# address is derived from the `infra` project label, which is the same contract
# `bootstrap` imports by, so this fails the moment either half moves.
grep -q '✓ 2.14' tmp/require-50.txt \
  || fail "5.0 2.14 does not resolve — the scaffold's Cloud Asset service or the infra label moved:\n$(grep '2.14' tmp/require-50.txt)"
grep -q 'google_project_service.infra_cloudasset_googleapis_com' tmp/require-50.txt \
  || fail "2.14 resolved against something other than the scaffold's own service"
# the renumbered controls resolve against the SAME resources as their 4.0 twins
grep -qE '✓ 1.5 .*iam_managed_disableServiceAccountKeyCreation' tmp/require-50.txt || fail "4.0 1.4 -> 5.0 1.5 did not carry over:\n$(grep ' 1.5 ' tmp/require-50.txt)"
grep -qE '✓ 1.6 .*preventPrivilegedBasicRoles' tmp/require-50.txt || fail "4.0 1.5 -> 5.0 1.6 did not carry over"
grep -qE '✓ 3.10 .*compute_requireVpcFlowLogs' tmp/require-50.txt || fail "4.0 3.8 -> 5.0 3.10 did not carry over"
# 1.2 keeps the 4.0 claim's duties, so it is partial rather than satisfied — the
# point is that it resolves at all, against the same policy
grep -qE '◐ 1.2 .*legacy-superseded' tmp/require-50.txt || fail "4.0 1.1 -> 5.0 1.2 did not carry over with its duties"
# 1.1.4 asks whether the org constrains its projects centrally: the whole baseline is the witness
grep -qE '◐ 1.1.4 .*review-baseline' tmp/require-50.txt || fail "the 5.0 §1.1.4 baseline claim is missing:\n$(grep '1.1.4' tmp/require-50.txt)"

step "CIS extensions: opt-in coverage, off by default and on when asked"
# off by default: the baseline must not enforce any of them
grep -q 'compute.requireShieldedVm' hcl/main.tf && fail "an opt-in extension leaked into the baseline"
grep -q 'gcp.restrictNonCmekServices' hcl/main.tf && fail "an opt-in extension leaked into the baseline"
# and on when the estate asks. The three constraint SHAPES differ, and a wrong
# body is a policy that either does nothing or refuses everything — so assert
# each one, not just that something was emitted.
sed -e 's/^params {/params {\n  cis_require_shielded_vm = true\n  cis_cmek_required = true\n  cis_api_key_services = true\n  allowed_api_key_services = ["storage.googleapis.com"]/' yaml/smoke.satz > tmp/ext.satz
cat >> tmp/ext.satz <<'SATZ'
use "presets/cis/shielded-vm.satz" when cis_require_shielded_vm
use "presets/cis/cmek.satz" when cis_cmek_required
use "presets/cis/api-key-services.satz" when cis_api_key_services
SATZ
"$satz" --config . transpile tmp/ext.satz --output "$PWD/tmp/ext-hcl" > tmp/ext.txt 2>&1 || fail "the extensions do not transpile:\n$(cat tmp/ext.txt)"
grep -q 'name = "organizations/123456789012/policies/compute.requireShieldedVm"' tmp/ext-hcl/main.tf || fail "the plain boolean constraint is missing"
grep -q 'parameters = "{\\"allowedServices\\":\[\\"storage.googleapis.com\\"\]}"' tmp/ext-hcl/main.tf || fail "the parameterised constraint did not JSON-encode its parameters:\n$(grep -A3 disableServiceAccountApiKey tmp/ext-hcl/main.tf)"
grep -q 'denied_values' tmp/ext-hcl/main.tf || fail "the CMEK list constraint lost its values"
"$satz" --config . require cis-gcp-4.0 tmp/ext.satz --format text --out tmp/ext-require.txt 2>/dev/null || true
grep -q '0 broken claim' tmp/ext-require.txt || fail "an extension claims a witness it does not emit:\n$(grep -i broken tmp/ext-require.txt)"
grep -qE '✓ 4.8 ' tmp/ext-require.txt || fail "4.8 did not become satisfied with its fragment on"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/ext-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the extensions do not validate"
fi

step "org firewall: the admin ports are closed to the internet and open to the inside"
grep -q 'cis_block_internet_ssh_rdp = true' ../../presets/cis/CIS-GCP-Foundation-4.0.satz \
  || fail "cis_block_internet_ssh_rdp does not default to true"
cp yaml/smoke.satz tmp/fw.satz
cat >> tmp/fw.satz <<'SATZ'
use "presets/cis/internet-ssh-rdp.satz" when cis_block_internet_ssh_rdp
SATZ
"$satz" --config . transpile tmp/fw.satz --output "$PWD/tmp/fw-hcl" > tmp/fw.txt 2>&1 || fail "the admin-port pack does not transpile:\n$(cat tmp/fw.txt)"
grep -q 'resource "google_compute_firewall_policy" "cis_admin_ports"' tmp/fw-hcl/main.tf || fail "no firewall policy"
grep -q 'resource "google_compute_firewall_policy_association" "cis_admin_ports"' tmp/fw-hcl/main.tf \
  || fail "the policy is not attached — a policy that is not associated enforces nothing"
# four rules: a pass and a deny per address family, passes first
for r in cis_admin_ports_listed cis_admin_ports_listed_ipv6 cis_admin_ports_internet_ipv4 cis_admin_ports_internet_ipv6; do
  grep -q "resource \"google_compute_firewall_policy_rule\" \"$r\"" tmp/fw-hcl/main.tf || fail "rule $r is missing"
done
# the private ranges pass, or SSH between two instances in one subnet dies
grep -q '"10.0.0.0/8"' tmp/fw-hcl/main.tf || fail "the private ranges are not passed: internal SSH would be denied by the 0.0.0.0/0 rule"
grep -q '"35.235.240.0/20"' tmp/fw-hcl/main.tf || fail "IAP's range is not passed"
grep -q '"2600:2d00:1:7::/64"' tmp/fw-hcl/main.tf || fail "IAP's IPv6 range is not passed"
# the control is not TCP-only: SSH is also SCTP 22, RDP is also UDP 3389
grep -q 'ip_protocol = "sctp"' tmp/fw-hcl/main.tf || fail "SCTP 22 is not denied, so the SSH control is half-enforced"
grep -q 'ip_protocol = "udp"' tmp/fw-hcl/main.tf || fail "UDP 3389 is not denied, so the RDP control is half-enforced"
# Google forbids logging on goto_next, so exactly the two denies log
[ "$(grep -c 'enable_logging = true' tmp/fw-hcl/main.tf)" = 2 ] \
  || fail "expected logging on the two deny rules only — Google refuses it on goto_next:\n$(grep -c 'enable_logging' tmp/fw-hcl/main.tf)"
"$satz" --config . require cis-gcp-4.0 tmp/fw.satz --format text --out tmp/fwreq.txt 2>/dev/null || true
grep -q '0 broken claim' tmp/fwreq.txt || fail "the 3.6/3.7 claims name a witness the estate does not emit:\n$(grep -i broken tmp/fwreq.txt)"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/fw-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the admin-port policy does not validate"
fi

step "dns logging: the one extension that is on by default, and what it claims"
# The baseline already enforces flow logs (compute.requireVpcFlowLogs, claimed for
# 4.0 3.8 / 5.0 3.10), so the smoke estate carries it without asking for anything.
grep -q 'name = "organizations/123456789012/policies/compute.requireVpcFlowLogs"' hcl/main.tf || fail "the baseline lost its flow-log constraint"
# The flag defaults TRUE — the only extension that does — but a flag alone emits
# nothing: the estate carries the `use … when` line (ADR 0007), and the skeleton writes it.
grep -q 'cis_dns_logging            = true' ../../presets/cis/CIS-GCP-Foundation-4.0.satz \
  || fail "cis_dns_logging does not default to true"
cp yaml/smoke.satz tmp/dns.satz
cat >> tmp/dns.satz <<'SATZ'
use "presets/cis/dns-logging.satz" when cis_dns_logging
SATZ
"$satz" --config . transpile tmp/dns.satz --output "$PWD/tmp/dns-hcl" > tmp/dns.txt 2>&1 || fail "the dns-logging fragment does not transpile:\n$(cat tmp/dns.txt)"
# a CUSTOM constraint, because Google publishes no predefined one for DNS logging
grep -q 'resource "google_org_policy_custom_constraint" "cis_dns_logging"' tmp/dns-hcl/main.tf \
  || fail "the DNS custom constraint is missing"
grep -q 'condition = "resource.enableLogging == true"' tmp/dns-hcl/main.tf \
  || fail "the DNS constraint's condition is not the one measured against the live API"
grep -q '"dns.googleapis.com/Policy"' tmp/dns-hcl/main.tf || fail "the DNS constraint names the wrong resource type"
# it CONTRIBUTES, never implements: no org policy can require that a network HAS a policy
grep -q 'contributes' ../../presets/cis/dns-logging.satz || fail "the DNS claim must not be an implements"
"$satz" --config . require cis-gcp-5.0 tmp/dns.satz --format text --out tmp/dnsreq.txt 2>/dev/null || true
grep -q '0 broken claim' tmp/dnsreq.txt || fail "the DNS claim names a witness the estate does not emit:\n$(grep -i broken tmp/dnsreq.txt)"

step "sentinel: federation without a key, and an audit path whose every grant is there"
sed -e 's/^params {/params {\n  sentinel_project_id = infra_project_name\n  sentinel_workload_pool_id = "22222222222222222222222222222222"\n  sentinel_project_number = "123456789012"/' yaml/smoke.satz > tmp/sent.satz
cat >> tmp/sent.satz <<'SATZ'
use "presets/integrations/microsoft-sentinel.satz"
use "presets/integrations/microsoft-sentinel-auditlogs.satz"
SATZ
"$satz" --config . transpile tmp/sent.satz --output "$PWD/tmp/sent-hcl" > tmp/sent.txt 2>&1 || fail "the sentinel packs do not transpile:\n$(cat tmp/sent.txt)"
# the audience is api://<application id>, which is the form Microsoft's own script writes
grep -q '"api://2041288c-b303-4ca0-9076-9612db3beeb2"' tmp/sent-hcl/main.tf \
  || fail "the provider does not carry Sentinel's audience in the api:// form:\n$(grep -A6 sentinel_identity_provider tmp/sent-hcl/main.tf | head -10)"
grep -q 'issuer_uri = "https://sts.windows.net/33e01921-4d64-4f8c-a055-5bdaffd5e33d"' tmp/sent-hcl/main.tf || fail "the provider does not trust Microsoft's commercial tenant"
# the principal set carries the project NUMBER: a pool id alone grants nothing
grep -q 'principalSet://iam.googleapis.com/projects/123456789012/locations/global/workloadIdentityPools/22222222222222222222222222222222/\*' tmp/sent-hcl/main.tf \
  || fail "the workloadIdentityUser binding does not name the pool's principal set"
grep -q 'include_children = true' tmp/sent-hcl/main.tf || fail "the sink must cover every project under the organisation"
# without the publisher grant the sink exists and delivers nothing
grep -q 'role = "roles/pubsub.publisher"' tmp/sent-hcl/main.tf || fail "the sink's writer identity may not publish"
grep -q 'member = "${google_logging_organization_sink.sentinel_auditlogs.writer_identity}"' tmp/sent-hcl/main.tf \
  || fail "the publisher grant does not follow the sink's own writer identity"
# and Sentinel reads ONE subscription, not every subscription in the project
grep -q 'resource "google_pubsub_subscription_iam_member" "sentinel_auditlogs_reader"' tmp/sent-hcl/main.tf || fail "the connector is granted nothing to read"
grep -q 'role = "roles/pubsub.subscriber"' tmp/sent-hcl/main.tf || fail "the connector's read grant is not the subscriber role"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/sent-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the sentinel chain does not validate"
fi

step "sentinel network streams: four sinks, four subscriptions, and no authoritative grant"
sed -e 's/^params {/params {\n  sentinel_project_id = infra_project_name\n  sentinel_workload_pool_id = "22222222222222222222222222222222"\n  sentinel_project_number = "123456789012"/' yaml/smoke.satz > tmp/sentnet.satz
cat >> tmp/sentnet.satz <<'SATZ'
use "presets/integrations/microsoft-sentinel.satz"
use "presets/integrations/microsoft-sentinel-network-logs.satz"
SATZ
"$satz" --config . transpile tmp/sentnet.satz --output "$PWD/tmp/sentnet-hcl" > tmp/sentnet.txt 2>&1 || fail "the sentinel network pack does not transpile:\n$(cat tmp/sentnet.txt)"
# one stream per sink, selected by its own log id — never mixed with the same service's
# audit records, which the audit fragment already carries
for f in 'log_id(\"compute.googleapis.com/vpc_flows\")' 'log_id(\"compute.googleapis.com/firewall\")' 'resource.type=\"dns_query\"' 'log_id(\"compute.googleapis.com/nat_flows\")'; do
  grep -qF "$f" tmp/sentnet-hcl/main.tf || fail "a network stream's filter is missing: $f"
done
for sink in vpc-flow firewall dns nat; do
  grep -q "\"${sink}-logs-organization-sentinel-sink\"" tmp/sentnet-hcl/main.tf || fail "no organisation sink for the ${sink} stream"
done
for sub in vpcflowlogs firewalllogs DNSlogs natlogs; do
  grep -q "\"sentinel-subscription-${sub}\"" tmp/sentnet-hcl/main.tf || fail "no subscription for ${sub} — a stream sharing another's subscription splits its messages"
done
# upstream grants publisher with google_project_iam_binding, which is authoritative:
# the second stream applied would remove the first sink's grant and stop delivery
grep -q 'google_project_iam_binding' tmp/sentnet-hcl/main.tf && fail "an authoritative binding would remove another sink's publisher grant"
[ "$(grep -c 'role = "roles/pubsub.publisher"' tmp/sentnet-hcl/main.tf)" = 4 ] || fail "every sink's writer identity must be able to publish"
[ "$(grep -c 'role = "roles/pubsub.subscriber"' tmp/sentnet-hcl/main.tf)" = 4 ] || fail "the connector must be able to read every stream"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/sentnet-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the sentinel network chain does not validate"
fi

step "scc notifications: the chain is topic + grant + config, and the agent is the organisation's"
sed -e 's/^params {/params {\n  scc_notification_project = infra_project_name/' yaml/smoke.satz > tmp/scc.satz
cat >> tmp/scc.satz <<'SATZ'
use "presets/scc/scc-notifications.satz"
SATZ
"$satz" --config . transpile tmp/scc.satz --output "$PWD/tmp/scc-hcl" > tmp/scc.txt 2>&1 || fail "the scc notification pack does not transpile:\n$(cat tmp/scc.txt)"
grep -q 'resource "google_pubsub_topic" "scc_findings"' tmp/scc-hcl/main.tf || fail "no topic for the findings"
grep -q 'role = "roles/securitycenter.notificationServiceAgent"' tmp/scc-hcl/main.tf || fail "the notification service agent was not granted on the topic"
# the agent is derived from the estate's own organisation id — a hard-coded one
# would publish another organisation's findings nowhere
# the PUBLISHER, which is not the agent SCC activation creates — measured live:
# with `security-center-api` here the config publishes nothing and says nothing
grep -q 'member = "serviceAccount:service-org-123456789012@gcp-sa-scc-notification.iam.gserviceaccount.com"' tmp/scc-hcl/main.tf || fail "the grant does not name the publishing agent with the estate's organisation id:\n$(grep -A2 notificationServiceAgent tmp/scc-hcl/main.tf)"
grep -q 'resource "google_scc_v2_organization_notification_config"' tmp/scc-hcl/main.tf || fail "the v2 notification config is missing"
grep -q 'location = "global"' tmp/scc-hcl/main.tf || fail "the config must sit at the global location"
grep -q 'pubsub_topic = "${google_pubsub_topic.scc_findings.id}"' tmp/scc-hcl/main.tf || fail "the config does not point at the topic this pack creates"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/scc-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the scc notification chain does not validate"
fi

step "scc findings mail: a subscription so nothing is dropped, a mailbox, and an alert"
sed -e 's/^params {/params {\n  scc_notification_project = infra_project_name/' yaml/smoke.satz > tmp/sccm.satz
cat >> tmp/sccm.satz <<'SATZ'
use "presets/scc/scc-notifications.satz"
use "presets/scc/scc-findings-mail.satz"
SATZ
"$satz" --config . transpile tmp/sccm.satz --output "$PWD/tmp/sccm-hcl" > tmp/sccm.txt 2>&1 || fail "the scc findings-mail pack does not transpile:\n$(cat tmp/sccm.txt)"
# a topic with no subscription drops every message: that is what this pack is for
grep -q 'resource "google_pubsub_subscription" "scc_findings"' tmp/sccm-hcl/main.tf || fail "no subscription on the findings topic"
grep -q 'message_retention_duration = "604800s"' tmp/sccm-hcl/main.tf || fail "the subscription must hold a weekend's findings"
# the address is the central alert pack's, by reference — one security mailbox
grep -A6 'resource "google_monitoring_notification_channel" "scc_findings_mail"' tmp/sccm-hcl/main.tf > tmp/sccm-channel.txt || true
grep -q '"email_address" = "gcp-security@example.com"' tmp/sccm-channel.txt \
  || fail "the mailbox does not default to the address the organisation's CIS alerts go to:\n$(grep -A6 'scc_findings_mail' tmp/sccm-hcl/main.tf | head -12)"
grep -q 'resource "google_monitoring_alert_policy" "scc_findings_published"' tmp/sccm-hcl/main.tf || fail "nothing fires when findings arrive"
grep -q '"${google_monitoring_notification_channel.scc_findings_mail.id}"' tmp/sccm-hcl/main.tf \
  || fail "the alert does not reach the mailbox this pack creates"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/sccm-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the scc findings mail chain does not validate"
fi

step "scc findings siem: the connector's own subscription, and the grant without which it reads nothing"
sed -e 's/^params {/params {\n  scc_notification_project = infra_project_name\n  scc_siem_subscriber = "serviceAccount:sentinel-service-account@acme-infra-001.iam.gserviceaccount.com"/' yaml/smoke.satz > tmp/sccs.satz
cat >> tmp/sccs.satz <<'SATZ'
use "presets/scc/scc-notifications.satz"
use "presets/scc/scc-findings-siem.satz"
SATZ
"$satz" --config . transpile tmp/sccs.satz --output "$PWD/tmp/sccs-hcl" > tmp/sccs.txt 2>&1 || fail "the scc findings-siem pack does not transpile:\n$(cat tmp/sccs.txt)"
grep -q 'resource "google_pubsub_subscription" "scc_findings_siem"' tmp/sccs-hcl/main.tf || fail "the connector has no subscription of its own"
# two readers on ONE subscription split the findings, so the mail pack's is not reused
grep -q 'resource "google_pubsub_subscription_iam_member" "scc_findings_siem_reader"' tmp/sccs-hcl/main.tf || fail "nothing grants the connector's identity"
grep -q 'role = "roles/pubsub.subscriber"' tmp/sccs-hcl/main.tf || fail "the connector is granted the wrong role"
grep -q 'member = "serviceAccount:sentinel-service-account@acme-infra-001.iam.gserviceaccount.com"' tmp/sccs-hcl/main.tf \
  || fail "the grant does not name the identity the estate answered with:\n$(grep -A4 scc_findings_siem_reader tmp/sccs-hcl/main.tf | head -8)"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/sccs-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the scc siem chain does not validate"
fi

step "scc export: the API comes first, the dataset keeps its contents, the agent can write"
sed -e 's/^params {/params {\n  scc_export_project = infra_project_name/' yaml/smoke.satz > tmp/scce.satz
cat >> tmp/scce.satz <<'SATZ'
use "presets/scc/scc-export.satz"
SATZ
"$satz" --config . transpile tmp/scce.satz --output "$PWD/tmp/scce-hcl" > tmp/scce.txt 2>&1 || fail "the scc export pack does not transpile:\n$(cat tmp/scce.txt)"
grep -q 'service = "bigquery.googleapis.com"' tmp/scce-hcl/main.tf || fail "the dataset's project does not get the BigQuery API"
# through the service resource, so the API is enabled before the dataset is made
grep -q 'project = "${google_project_service.scc_export_bigquery.project}"' tmp/scce-hcl/main.tf \
  || fail "the dataset does not take its project through the service, so the two race:\n$(grep -A4 google_bigquery_dataset tmp/scce-hcl/main.tf | head -8)"
grep -q 'delete_contents_on_destroy = false' tmp/scce-hcl/main.tf || fail "removing the pack must not delete the finding history"
grep -q 'member = "serviceAccount:service-org-123456789012@gcp-sa-scc-notification.iam.gserviceaccount.com"' tmp/scce-hcl/main.tf \
  || fail "the exporting agent is not granted on the dataset"
grep -q 'resource "google_scc_v2_organization_scc_big_query_export"' tmp/scce-hcl/main.tf || fail "the export itself is missing"
# pinned, because the server assigns it: without it in the config every plan wants to
# null the field and the API refuses the update
grep -q 'name = "organizations/123456789012/locations/global/bigQueryExports/satz-findings"' tmp/scce-hcl/main.tf \
  || fail "the export does not pin the name the server assigns:\n$(grep -A8 'scc_v2_organization_scc_big_query_export' tmp/scce-hcl/main.tf | head -10)"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/scce-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color >/dev/null) || fail "the scc export does not validate"
fi

step "require iso27001-2022 (cross-walk: ISO verdicts folded from the CIS ones)"
"$satz" --config . require iso27001-2022 smoke.satz --format text --out tmp/require-iso.txt 2>/dev/null || true
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
"$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out-dir tmp/plan > tmp/plan.txt 2>&1 || fail "remediation-plan failed:\n$(cat tmp/plan.txt)"
for f in dossier.json findings.csv findings.xlsx meta.json; do [ -s "tmp/plan/$f" ] || fail "remediation-plan: $f missing or empty"; done
grep -q '"declared_address": "google_storage_bucket.state"' tmp/plan/dossier.json || fail "the bucket finding was not joined to its declaring block"
grep -q '^\[Authored\] Recommended fix' <(head -1 tmp/plan/findings.csv | tr ',' '\n') || fail "the CSV lacks the [Authored] columns"
"$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out-dir tmp/plan2 >/dev/null 2>&1
h1=$(grep -o '"dossier_sha256": "[0-9a-f]*"' tmp/plan/meta.json); h2=$(grep -o '"dossier_sha256": "[0-9a-f]*"' tmp/plan2/meta.json)
[ "$h1" = "$h2" ] || fail "the dossier is not deterministic: $h1 vs $h2"
# The round trip: authored values, pinned to the dossier's hash, rendered beside the
# mechanical columns — and the dossier and its hash unchanged by them.
python3 - <<'PYEOF' || fail "could not write the authored fixture"
import json
meta = json.load(open("tmp/plan/meta.json"))
first = json.load(open("tmp/plan/dossier.json"))["items"][0]["id"]
json.dump({"dossier_sha256": meta["dossier_sha256"], "items": {first: {
    "recommended_fix": "Turn on public access prevention for the state bucket",
    "authored_by": "smoke", "authored_at": "2026-09-11T20:00:00Z"}}}, open("tmp/authored.json", "w"))
json.dump({"dossier_sha256": "0" * 64, "items": {}}, open("tmp/authored-stale.json", "w"))
PYEOF
"$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out-dir tmp/plan3 --merge tmp/authored.json > tmp/plan3.txt 2>&1 \
  || fail "remediation-plan --merge failed:\n$(cat tmp/plan3.txt)"
grep -q 'Turn on public access prevention for the state bucket' tmp/plan3/findings.csv || fail "the authored value is not in the CSV"
grep -q 'smoke,2026-09-11T20:00:00Z' tmp/plan3/findings.csv || fail "the CSV does not name who authored the value, and when"
[ -s tmp/plan3/authored.json ] || fail "--merge did not keep authored.json beside the run"
cmp -s tmp/plan/dossier.json tmp/plan3/dossier.json || fail "authoring changed dossier.json — the hash that names the run must not move"
if "$satz" --config . remediation-plan cis-gcp-4.0 smoke.satz --prowler prowler.json --out-dir tmp/plan4 --merge tmp/authored-stale.json > tmp/plan4.txt 2>&1; then
  fail "authored values written against another dossier were merged"
fi
grep -q 'the findings changed' tmp/plan4.txt || fail "the stale-hash refusal does not say why:\n$(cat tmp/plan4.txt)"

step "triage: Prowler FAILs sorted into buckets against the estate's claims"
"$satz" --config . triage cis-gcp-4.0 smoke.satz --prowler prowler.json --format markdown --out tmp/triage.md 2>tmp/triage.err || fail "triage failed:\n$(cat tmp/triage.err)"
grep -q '^## B ·' tmp/triage.md || fail "no bucket headings"
grep -q 'declared as `google_storage_bucket' tmp/triage.md || fail "the bucket finding was not matched to its declaring block:\n$(cat tmp/triage.md)"
# triage writes markdown, so it writes the same document typeset; --out may leave
# the extension off
rm -f tmp/triage.pdf
"$satz" --config . triage cis-gcp-4.0 smoke.satz --prowler prowler.json --format pdf --out tmp/triage 2>tmp/triage-pdf.err \
  || fail "triage --format pdf failed:\n$(cat tmp/triage-pdf.err)"
head -c 5 tmp/triage.pdf | grep -q '%PDF-' || fail "triage --format pdf --out tmp/triage did not write tmp/triage.pdf"
# --fix turns the buckets into the estate edit they imply, inside the report
"$satz" --config . triage cis-gcp-4.0 smoke.satz --prowler prowler.json --fix --format markdown --out tmp/triage-fix.md 2>/dev/null \
  || fail "triage --fix failed:\n$(cat tmp/triage-fix.md)"
grep -q 'proposed estate delta' tmp/triage-fix.md || fail "--fix printed no delta:\n$(cat tmp/triage-fix.md)"

step "report-compliance: the Prowler column, and the envelope says whether live state was read"
"$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --prowler prowler.json --format markdown --out tmp/ev2.md >/dev/null 2>&1 || fail "report-compliance --prowler failed"
grep -q 'FAIL' tmp/ev2.md || fail "the Prowler column is empty"
grep -q 'Prowler 5.42.0' tmp/ev2.md || fail "the report does not name the Prowler version that wrote the export"
# `live` means "the inventory WAS read", never "live was requested", so a caller with
# no stderr (MCP, a pipeline) can tell a blind run from a verified one. CI has no
# credentials, so `--no-live` is the case it can assert.
"$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --format json --out tmp/ev-envelope.json 2>/dev/null || fail "report-compliance --format json failed"
python3 - <<'PYEOF' || fail "the evidence envelope does not describe the live run"
import json
o = json.load(open("tmp/ev-envelope.json"))
for f in ("live", "live_status", "warnings"):
    assert f in o, f"the envelope has no {f}: {sorted(o)}"
assert o["live"] is False, o["live"]
assert o["live_status"] == "skipped", o["live_status"]
assert o["warnings"] == [], o["warnings"]
assert any(r["witnesses"] for r in o["rows"]), "no row carries a witness"
# the estate declares no exemption key: no undeclared-binding section at all
assert o["exemption_bindings"] is None, o["exemption_bindings"]
assert all(r["undeclared_exemptions"] == [] for r in o["rows"])
PYEOF

step "report-compliance with no framework: one section per framework the estate is HELD TO"
# The estate binds `compliance_frameworks = ["cis-gcp-5.0", "iso27001-2022"]`; the packs
# claim CIS 4.0 and 5.0. The two are different facts, and this is the one the customer
# answers to.
"$satz" --config . report-compliance smoke.satz --no-live --format markdown --out tmp/ev-held.md >tmp/ev-held.txt 2>&1 \
  || fail "report-compliance with no framework failed:\n$(cat tmp/ev-held.txt)"
grep -q '^# Evidence report — cis-gcp 5.0' tmp/ev-held.md || fail "no CIS 5.0 section:\n$(head -5 tmp/ev-held.md)"
grep -q '^# Evidence report — iso27001 2022' tmp/ev-held.md || fail "no ISO 27001 section"
grep -q '^---$' tmp/ev-held.md || fail "the sections are not separated"
# The framework the packs claim and the estate is not held to is NOT in the report.
! grep -q '^# Evidence report — cis-gcp 4.0' tmp/ev-held.md \
  || fail "cis-gcp 4.0 is reported, and this estate is not held to it"
# `--format json` asked this way answers the SET, one report per framework.
"$satz" --config . report-compliance smoke.satz --no-live --format json --out tmp/ev-held.json 2>/dev/null \
  || fail "report-compliance with no framework --format json failed"
python3 - <<'PYEOF' || fail "the held-to envelope is not the set"
import json
o = json.load(open("tmp/ev-held.json"))
assert o["frameworks"] == ["cis-gcp-5.0", "iso27001-2022"], o["frameworks"]
got = [r["framework"] + "-" + r["version"] for r in o["reports"]]
assert got == ["cis-gcp-5.0", "iso27001-2022"], got
PYEOF
# A framework the library has no catalog for is refused by the COMPILE, with the list.
sed 's/"cis-gcp-5.0", "iso27001-2022"/"cis-gcp-9.9"/' yaml/smoke.satz > tmp/frameworks-typo.satz
if "$satz" --config . transpile ../tmp/frameworks-typo.satz --check >tmp/typo.txt 2>&1; then
  fail "a compliance_frameworks value naming no catalog compiled"
fi
grep -q 'cis-gcp-9.9' tmp/typo.txt || fail "the refusal does not name the value:\n$(cat tmp/typo.txt)"
grep -q 'cis-gcp-4.0, cis-gcp-5.0, iso27001-2022' tmp/typo.txt \
  || fail "the refusal does not list the catalogs that exist:\n$(cat tmp/typo.txt)"

step "import-config: every derivable asset_type is filled (the CAI list is the source)"
cp "$root/presets/import-config.yaml" tmp/import-config.yaml
uv run --with ruamel.yaml "$root/scripts/update_import_config.py" --config-file tmp/import-config.yaml --cai-types "$root/presets/cai-asset-types.txt" | tee tmp/fill.txt
grep -q '^asset_type filled: 0;' tmp/fill.txt || fail "presets/import-config.yaml is behind presets/cai-asset-types.txt — run the fill and commit it"

step "a tag-conditional exemption keeps the verdict and is reported beside it"
# The CIS baseline's OWN constraint, exempted by rebinding one param — no fork, and the
# claim is the pack's rather than one this step wrote to make the assertion pass.
cp "$root/tests/iac/exemption-tag/main.satz" tmp/exemption.satz
"$satz" --config . require cis-gcp-4.0 ../tmp/exemption.satz --format text --out tmp/exemption.txt 2>/dev/null || true
grep -q 'cis_sa_key_creation_rules' "$root/presets/cis/CIS-GCP-Foundation-4.0.satz" \
  || fail "the baseline does not take the SA-key rules from a param — the exemption would need a fork"
grep -qE '^  . 1\.4 ' tmp/exemption.txt || fail "the exempted control is not in the goal view at all"
grep -q '✓ 1.4' tmp/exemption.txt \
  || fail "a conditional exemption unmade the verdict — the unconditional rule decides"
grep -q '↳ exempted:' tmp/exemption.txt \
  || fail "the exemption is not reported beside the verdict"
grep -q '1 control(s) carry a conditional exemption' tmp/exemption.txt \
  || fail "the exemption is not counted in the summary"

step "dry-run twins are derived from their enforcing fragments, not written beside them"
uv run "$root/scripts/build_dry_run_fragments.py" --check \
  || fail "a dry-run twin is stale — run scripts/build_dry_run_fragments.py and commit"

step "prowler: the invocation this estate needs, printed and never run"
"$satz" --config . prowler smoke.satz > tmp/prowler-plan.txt 2> tmp/prowler-notes.txt \
  || fail "satz prowler failed on the smoke estate"
# stdout is the command and nothing else: it gets pasted into a shell or piped to a
# clipboard, so a heading above it would have to be edited out every time.
[ "$(wc -l < tmp/prowler-plan.txt)" -eq 1 ] \
  || fail "satz prowler printed more than the command line:\n$(cat tmp/prowler-plan.txt)"
grep -q '^prowler gcp ' tmp/prowler-plan.txt \
  || fail "stdout does not start with the invocation:\n$(cat tmp/prowler-plan.txt)"
# what the line cannot say rides on stderr
grep -q '^then: satz report-compliance ' tmp/prowler-notes.txt \
  || fail "the fold-back command is not named on stderr:\n$(cat tmp/prowler-notes.txt)"
grep -q 'prowler gcp --organization-id' tmp/prowler-plan.txt || fail "no invocation printed"
grep -q -- '--compliance cis_4.0_gcp cis_5.0_gcp' tmp/prowler-plan.txt \
  || fail "the frameworks the estate CLAIMS did not reach --compliance"
grep -q -- '--output-formats json-ocsf' tmp/prowler-plan.txt \
  || fail "the only export shape report-compliance reads is not requested"
grep -q 'evidence/prowler/' tmp/prowler-plan.txt || fail "the standard output location is not used"
# Read-only means read-only: nothing is created, least of all the evidence directory.
[ ! -e evidence/prowler ] || fail "satz prowler created something — it prints, it does not run"
"$satz" --config . prowler smoke.satz --format json > tmp/prowler-plan.json 2>/dev/null || true
python3 - <<'PYEOF' || fail "satz prowler --format json did not emit parseable JSON"
import json, pathlib, re
d = json.loads(pathlib.Path("tmp/prowler-plan.json").read_text())
assert d["command"].startswith("prowler gcp "), d["command"]
assert d["output_path"].endswith(".ocsf.json"), d
# Prowler appends to a file that already exists, so each scan's name carries the UTC minute
assert re.fullmatch(r"org-\d{4}-\d\d-\d\dT\d\d-\d\dZ", d["output_filename"]), d["output_filename"]
assert d["output_directory"] == "evidence/prowler/" + d["output_filename"][4:14], d
assert f"--prowler {d['output_path']} " in d["then"], d["then"]
assert d["compliance"] == ["cis_4.0_gcp", "cis_5.0_gcp"], d
assert d["projects"], "no project reached the plan"
PYEOF

step "mcp-config: the block a client reads to start the server, printed and then written"
"$satz" --config . mcp-config smoke.satz > tmp/mcp-config.json 2> tmp/mcp-config-notes.txt \
  || fail "satz mcp-config failed on the smoke estate"
# stdout is the block and nothing else: it is piped into a file or a clipboard.
python3 - <<'PYEOF' || fail "satz mcp-config did not print the .mcp.json shape"
import json, os, pathlib
d = json.loads(pathlib.Path("tmp/mcp-config.json").read_text())
s = d["mcpServers"]["satz"]
assert s["type"] == "stdio", s
assert os.path.isabs(s["command"]), s["command"]
assert s["args"][:2] == ["mcp", "--root"], s["args"]
assert os.path.isabs(s["args"][2]), s["args"]
# the ceiling is written out even when it is the default
assert s["args"][3:] == ["--allow", "read"], s["args"]
PYEOF
grep -q '^then: satz mcp-config ' tmp/mcp-config-notes.txt \
  || fail "the notes do not name the write:\n$(cat tmp/mcp-config-notes.txt)"
[ ! -e .mcp.json ] || fail "satz mcp-config wrote a file — printing is the default"

"$satz" --config . mcp-config smoke.satz --client claude-desktop --allow read,write > tmp/mcp-desktop.json 2>/dev/null \
  || fail "satz mcp-config --client claude-desktop failed"
python3 - <<'PYEOF' || fail "satz mcp-config --client claude-desktop did not print the block Claude Desktop takes"
import json, pathlib
d = json.loads(pathlib.Path("tmp/mcp-desktop.json").read_text())
(key,) = d["mcpServers"]
assert key == "satz-smoke", key
s = d["mcpServers"][key]
assert "type" not in s, s
assert s["args"][3:] == ["--allow", "read,write"], s["args"]
PYEOF

# --write owns one key of the file and leaves every other server alone; a second run
# writes nothing.
mkdir -p tmp/clients
cat > tmp/clients/claude_desktop_config.json <<'JSONEOF'
{"globalShortcut":"Alt+Space","mcpServers":{"filesystem":{"command":"npx","args":["-y","server-filesystem","/tmp"]}}}
JSONEOF
"$satz" --config . mcp-config smoke.satz --client claude-desktop --write --file tmp/clients/claude_desktop_config.json \
  > /dev/null 2> tmp/mcp-write.txt || fail "satz mcp-config --write failed:\n$(cat tmp/mcp-write.txt)"
grep -q '1 other server(s) untouched' tmp/mcp-write.txt \
  || fail "the write does not say what it left alone:\n$(cat tmp/mcp-write.txt)"
"$satz" --config . mcp-config smoke.satz --client claude-desktop --write --file tmp/clients/claude_desktop_config.json \
  > /dev/null 2> tmp/mcp-write2.txt || fail "a second --write failed:\n$(cat tmp/mcp-write2.txt)"
grep -q '^unchanged ' tmp/mcp-write2.txt || fail "--write is not idempotent:\n$(cat tmp/mcp-write2.txt)"
python3 - <<'PYEOF' || fail "--write did not merge into the client's file"
import json, pathlib
d = json.loads(pathlib.Path("tmp/clients/claude_desktop_config.json").read_text())
assert d["globalShortcut"] == "Alt+Space", d
assert d["mcpServers"]["filesystem"]["args"] == ["-y", "server-filesystem", "/tmp"], d
assert d["mcpServers"]["satz-smoke"]["args"][:2] == ["mcp", "--root"], d
PYEOF
# the same estate at another ceiling is a different server: refused, then replaced
if "$satz" --config . mcp-config smoke.satz --client claude-desktop --allow read,write --write \
   --file tmp/clients/claude_desktop_config.json > /dev/null 2> tmp/mcp-refuse.txt; then
  fail "a satz key with other arguments was overwritten without --force"
fi
grep -q -- '--force replaces it' tmp/mcp-refuse.txt || fail "the refusal does not name --force:\n$(cat tmp/mcp-refuse.txt)"
"$satz" --config . mcp-config smoke.satz --client claude-desktop --allow read,write --write --force \
  --file tmp/clients/claude_desktop_config.json > /dev/null 2> tmp/mcp-force.txt \
  || fail "--force did not replace the key:\n$(cat tmp/mcp-force.txt)"
grep -q '^replaced ' tmp/mcp-force.txt || fail "the replacement is not reported:\n$(cat tmp/mcp-force.txt)"
# and the Claude Code half writes .mcp.json into the estate's own directory
"$satz" --config . mcp-config smoke.satz --write --file tmp/clients/.mcp.json > /dev/null 2>&1 \
  || fail "satz mcp-config --write failed for claude-code"
python3 -c "import json;d=json.load(open('tmp/clients/.mcp.json'));assert d['mcpServers']['satz']['type']=='stdio',d" \
  || fail ".mcp.json does not hold the stdio server"

step "review-pack: the library's own bar, as a command, on a good pack and a bad one"
# A pack every gate in this repository already accepts must clear the command too,
# or the command is not the same bar.
"$satz" --config . review-pack "$root/presets/organization-budget.satz" --format text --out tmp/review-good.txt 2>/dev/null \
  || fail "a shipped pack does not clear its own library's bar:\n$(cat tmp/review-good.txt)"
grep -q 'the pack clears the bar' tmp/review-good.txt || fail "the review does not say so:\n$(cat tmp/review-good.txt)"
grep -q 'emits: google_billing_budget' tmp/review-good.txt || fail "the review did not fold the pack into an estate:\n$(cat tmp/review-good.txt)"
# and it says what adopting it costs — the roles and APIs update-prerequisites writes
grep -q 'billingbudgets.googleapis.com' tmp/review-good.txt || fail "the review does not say what the pack costs an estate:\n$(cat tmp/review-good.txt)"

# The rules, each broken on purpose: no header, no version, a membership, unformatted.
mkdir -p tmp/packs
cat > tmp/packs/bad.satz <<'PACKEOF'
google_cloud_identity_group {
  "auditors" {
    display_name = "Auditors"
    parent = "customers/{customer_id}"
    group_key { id = "auditors@{customer_domain}" }
    labels = { "cloudidentity.googleapis.com/groups.discussion_forum" = "" }
  }
}

google_cloud_identity_group_membership {
  "auditors-first-admin" {
    group = "auditors"
    preferred_member_key { id = "{first_admin}@{customer_domain}" }
    roles = [{ name = "MEMBER" }]
  }
}
PACKEOF
if "$satz" --config . review-pack tmp/packs/bad.satz --format text --out tmp/review-bad.txt 2>/dev/null; then
  fail "a pack breaking four rules cleared the bar:\n$(cat tmp/review-bad.txt)"
fi
grep -q 'not formatted' tmp/review-bad.txt || fail "the layout rule did not fire:\n$(cat tmp/review-bad.txt)"
grep -q 'no header comment' tmp/review-bad.txt || fail "the header rule did not fire:\n$(cat tmp/review-bad.txt)"
grep -q 'no `pack <name> version' tmp/review-bad.txt || fail "the version rule did not fire:\n$(cat tmp/review-bad.txt)"
grep -q 'presets define groups, humans grant membership' tmp/review-bad.txt || fail "the membership rule did not fire:\n$(cat tmp/review-bad.txt)"
# the findings are the shape an editor already reads: severity, kind, file, line
"$satz" --config . review-pack tmp/packs/bad.satz --format json --out tmp/review-bad.json 2>/dev/null || true
python3 - <<'PYEOF' || fail "review-pack --format json is not the findings shape"
import json
r = json.load(open("tmp/review-bad.json"))
assert r["folded_into"] == "synthetic", r["folded_into"]
kinds = {f["kind"] for f in r["findings"]}
assert kinds == {"pack"}, kinds
errs = [f for f in r["findings"] if f["severity"] == "error"]
assert len(errs) >= 4, errs
assert any(f.get("line") for f in errs), "no finding is anchored to a line"
assert all(f.get("file", "").endswith("bad.satz") for f in r["findings"]), r["findings"]
PYEOF

step "pack docs are current, claims are on-catalog, every version has a changelog row (satz doc-packs --check)"
"$satz" --config . doc-packs --check || fail "presets/docs is behind the packs — run \`satz doc-packs\` and commit"

step "the pack graph passes its checks and is current (satz pack-graph --check)"
"$satz" pack-graph --presets-dir "$root/presets" --check > tmp/pack-graph.txt 2>&1 \
  || fail "satz pack-graph --check:\n$(cat tmp/pack-graph.txt)"
grep -q 'pack-graph.json current' tmp/pack-graph.txt || fail "pack-graph --check did not report the graph current"

step "report-compliance --format pdf: typeset by satz, with nothing on PATH"
"$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --format pdf --out tmp/evidence.pdf >/dev/null 2>&1 \
  || fail "report-compliance --format pdf failed"
python3 - <<'PYEOF' || fail "what was written is not a PDF"
data = open("tmp/evidence.pdf", "rb").read()
assert data.startswith(b"%PDF-"), data[:16]
assert len(data) > 20_000, f"a whole evidence report in {len(data)} bytes?"
assert b"/Type /Page" in data or b"/Type/Page" in data, "no page objects"
PYEOF

step "check-presets against the repository's own presets (must be clean)"
"$satz" --config . check-presets --pristine-dir "$root/presets" smoke.satz --format text --out /dev/stdout

# and the same verdicts as data — the last reporting command that had no JSON
"$satz" --config . check-presets smoke.satz --pristine-dir "$root/presets" --format json --out tmp/presets.json 2>/dev/null || true
python3 - <<'PYEOF' || fail "check-presets --format json did not write parseable JSON"
import json
d = json.load(open("tmp/presets.json"))
assert d["packs"], "no pack rows"
assert d["summary"]["drift_in_use"] is False, d["summary"]
assert {p["status"] for p in d["packs"]} <= {"clean","stale","edited","fork","local-only","missing-locally"}
PYEOF
grep -q 'satz v' tmp/presets.json && fail "the version banner reached the report file"

step "get-presets installs the library; merge-presets writes the prerequisites an estate lacks, adopts a pack, and forks one with the emission proven"
rm -rf tmp/prq && mkdir -p tmp/prq/yaml tmp/prq/presets tmp/prq/pristine
cp -R "$root/tests/schemas" tmp/prq/schemas
cat > tmp/prq/config.toml <<'CFGEOF'
yaml_dir = "yaml"
hcl_dir = "hcl"
include_dirs = [".", "yaml"]
schema_dir = "schemas"
presets_dir = "presets"
tf_tool = "tofu"
google_providers = ["google", "google-beta"]
provider_version = "7.14.1"
CFGEOF
prq_pack() { # $1 version, $2 extra resources
  printf '// A log bucket, and what later versions add beside it.\npack logs version "%s"\n\ngoogle_storage_bucket {\n  logs {\n    name     = "acme-logs"\n    project  = "acme-infra-001"\n    location = "EU"\n  }\n}\n%b' "$1" "$2"
}
prq_pack 1.0 '' > tmp/prq/pristine/logs.satz
"$satz" --config tmp/prq/config.toml get-presets --pristine-dir tmp/prq/pristine > tmp/prq-get.txt 2>&1 \
  || fail "get-presets failed:\n$(cat tmp/prq-get.txt)"
cmp -s tmp/prq/pristine/logs.satz tmp/prq/presets/logs.satz || fail "get-presets did not install the pack:\n$(cat tmp/prq-get.txt)"
printf 'estate prq\n\nparams {\n  customer_organization_id = "123456789012"\n  billing_account_infra    = "012345-6789AB-CDEF01"\n  svc_iac_account          = "svc-iac-001"\n  infra_project_name       = "acme-infra-001"\n}\n\nterraform {\n  backend {\n    local { path = "terraform.tfstate" }\n  }\n}\n\nproviders {\n  google {\n    alias   = "google"\n    project = "acme-infra-001"\n    region  = "europe-west3"\n  }\n}\n\ngoogle_project {\n  infra {\n    name            = "acme-infra"\n    project_id      = infra_project_name\n    org_id          = customer_organization_id\n    project_service = [\n      "storage.googleapis.com",\n    ]\n  }\n}\n\nuse "presets/logs.satz"\n' > tmp/prq/yaml/prq.satz
(cd tmp/prq && git init -q && git add -A && git -c user.name=smoke -c user.email=smoke@example.com commit -qm estate)
"$satz" --config tmp/prq/config.toml --validation error merge-presets --pristine-dir tmp/prq/pristine > tmp/prq-merge.txt 2>&1 \
  || fail "merge-presets at --validation error stopped on the gap it writes:\n$(cat tmp/prq-merge.txt)"
grep -q 'prerequisite written: roles/storage.admin' tmp/prq-merge.txt \
  || fail "merge-presets did not write the missing role:\n$(cat tmp/prq-merge.txt)"
# The pack's next version adds a topic, whose role and API the estate does not declare.
(cd tmp/prq && git add -A && git -c user.name=smoke -c user.email=smoke@example.com commit -qm prerequisites)
prq_pack 1.1 '\ngoogle_pubsub_topic {\n  logs {\n    name    = "acme-logs"\n    project = "acme-infra-001"\n  }\n}\n' > tmp/prq/pristine/logs.satz
# an adoption asks for a plan, so the run exits 1; the report is the check
"$satz" --config tmp/prq/config.toml --validation error merge-presets --pristine-dir tmp/prq/pristine --adopt logs > tmp/prq-adopt.txt 2>&1 || true
grep -q 'adopted logs.satz in place' tmp/prq-adopt.txt || fail "the pack was not adopted:\n$(cat tmp/prq-adopt.txt)"
grep -q 'prerequisite written: roles/pubsub.editor' tmp/prq-adopt.txt \
  || fail "merge-presets did not write the role the adopted pack needs:\n$(cat tmp/prq-adopt.txt)"
grep -q 'prerequisite written: pubsub.googleapis.com' tmp/prq-adopt.txt \
  || fail "merge-presets did not write the API the adopted pack needs:\n$(cat tmp/prq-adopt.txt)"
"$satz" --config tmp/prq/config.toml --validation error transpile prq.satz --check > tmp/prq-adopted.txt 2>&1 \
  || fail "the estate lacks a prerequisite after the adoption:\n$(cat tmp/prq-adopted.txt)"
# Without --adopt a used pack whose upstream changed what it emits is forked, the estate is
# repointed at the fork, and the run proves the emission did not move.
(cd tmp/prq && git add -A && git -c user.name=smoke -c user.email=smoke@example.com commit -qm adopted)
prq_pack 1.2 '\ngoogle_pubsub_topic {\n  logs {\n    name    = "acme-logs-eu"\n    project = "acme-infra-001"\n  }\n}\n' > tmp/prq/pristine/logs.satz
# a fork asks for a look, so the run exits 1; the report is the check
"$satz" --config tmp/prq/config.toml merge-presets --pristine-dir tmp/prq/pristine > tmp/prq-fork.txt 2>&1 || true
grep -q 'forked logs.satz -> logs.local.satz' tmp/prq-fork.txt || fail "the used pack was not forked:\n$(cat tmp/prq-fork.txt)"
grep -q 'estate edit verified: transpiled output identical' tmp/prq-fork.txt || fail "the repoint was not proven:\n$(cat tmp/prq-fork.txt)"
grep -q '^use "presets/logs.local.satz"$' tmp/prq/yaml/prq.satz || fail "the estate was not repointed:\n$(grep -n '^use' tmp/prq/yaml/prq.satz)"
grep -q 'acme-logs-eu' tmp/prq/presets/logs.local.satz && fail "the fork carries upstream's change instead of what the estate deployed"
[ -f tmp/prq/presets/logs.diff.satz ] || fail "the adoption delta was not written"
(cd tmp/prq && git add -A && git -c user.name=smoke -c user.email=smoke@example.com commit -qm forked)
"$satz" --config tmp/prq/config.toml merge-presets --pristine-dir tmp/prq/pristine > tmp/prq-again.txt 2>&1 \
  || fail "a second merge-presets needs attention:\n$(cat tmp/prq-again.txt)"
grep -q 'forked logs\|estate edit' tmp/prq-again.txt && fail "a second merge-presets edited the estate again:\n$(cat tmp/prq-again.txt)"

step "import, state shape"
"$satz" --config . import state.json -o imported-state.satz --verbose | tee tmp/import-state.txt
grep -q 'skipped' tmp/import-state.txt || fail "the skipped report did not print"
"$satz" --config . transpile imported-state.satz --output "$PWD/tmp/imported-state-hcl"
"$satz" fmt --check yaml/imported-state.satz || fail "import wrote an estate that is not in the canonical layout"
grep -q 'import {' tmp/imported-state-hcl/imports.tf || fail "state import produced no import blocks"
grep -q 'id = "organizations/123456789012/policies/compute.skipDefaultNetworkCreation"' tmp/imported-state-hcl/imports.tf || fail "the interpolated import id did not reach imports.tf as the literal"
"$satz" --config . import state.json --customer-shortname acme -o imported-state-named.satz > /dev/null 2>&1 || fail "import with --customer-shortname failed"
grep -qE '^  customer_shortname += "acme"$' yaml/imported-state-named.satz || fail "--customer-shortname did not win over the inference"

step "import, state shape: a state that names no organization is refused, and --organization names it"
# the same state with its organization taken out: folders and projects only,
# the top folder's parent a folder outside the state
python3 - <<'PY'
import json, pathlib

state = json.loads(pathlib.Path("state.json").read_text())
resources = []
for r in state["values"]["root_module"]["resources"]:
    if r["type"] not in ("google_folder", "google_project"):
        continue
    if r["type"] == "google_folder":
        r["values"]["parent"] = "folders/222222222"
    resources.append(r)
state["values"]["root_module"]["resources"] = resources
pathlib.Path("tmp/state-no-org.json").write_text(json.dumps(state))
PY
if "$satz" --config . import tmp/state-no-org.json --from state -o imported-no-org.satz >tmp/import-no-org.txt 2>&1; then
  fail "a state that names no organization must be refused"
fi
grep -q 'no organization id' tmp/import-no-org.txt || fail "the refusal did not say what is missing:\n$(cat tmp/import-no-org.txt)"
grep -q -- '--organization' tmp/import-no-org.txt || fail "the refusal did not say how to supply it:\n$(cat tmp/import-no-org.txt)"
[ -f yaml/imported-no-org.satz ] && fail "a refused import wrote an estate"
"$satz" --config . import tmp/state-no-org.json --from state --organization 123456789012 -o imported-no-org.satz >/dev/null 2>&1 \
  || fail "the same state with --organization must import"
grep -qE '^  customer_organization_id += "123456789012"$' yaml/imported-no-org.satz \
  || fail "--organization did not reach the estate's params"
# the flag names what the state does not carry, never what it contradicts
if "$satz" --config . import state.json --organization 222222222222 -o imported-conflict.satz >tmp/import-conflict.txt 2>&1; then
  fail "--organization against a state that names another organization must be refused"
fi
grep -q '123456789012' tmp/import-conflict.txt || fail "the refusal did not name the organization the state carries:\n$(cat tmp/import-conflict.txt)"
if "$satz" --config . import organizations/123456789012 --organization 123456789012 >tmp/import-org-flag.txt 2>&1; then
  fail "--organization must be refused on the live shape"
fi
grep -q 'applies to the state and hcl shapes' tmp/import-org-flag.txt \
  || fail "the live refusal did not say which shape --organization belongs to:\n$(cat tmp/import-org-flag.txt)"

step "import --generate-unmapped: the fallback is the live shape's, plain or --into; the other shapes say so"
if "$satz" --config . import state.json --generate-unmapped >tmp/gen-state.txt 2>&1; then
  fail "--generate-unmapped must be refused on the state shape"
fi
grep -q 'applies to the live shape' tmp/gen-state.txt || fail "the state refusal did not say which shape it belongs to:\n$(cat tmp/gen-state.txt)"
grep -q 'hcl shape' tmp/gen-state.txt || fail "the state refusal did not name the shape that reads existing .tf:\n$(cat tmp/gen-state.txt)"
if "$satz" --config . import tf --generate-unmapped >tmp/gen-hcl.txt 2>&1; then
  fail "--generate-unmapped must be refused on the hcl shape"
fi
grep -q -- '-generate-config-out' tmp/gen-hcl.txt || fail "the hcl refusal did not say that this shape already reads that output:\n$(cat tmp/gen-hcl.txt)"
# The delta import takes the flag: offline the run gets as far as the live sweep
# and stops there for want of credentials, which is what this can assert. The
# generated file itself needs a live object to read (§12.2).
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent CLOUDSDK_CONFIG=/nonexistent \
  "$satz" --config . import organizations/123456789012 --into smoke.satz --generate-unmapped >tmp/gen-into.txt 2>&1; then
  fail "a delta import must not succeed without credentials:\n$(cat tmp/gen-into.txt)"
fi
grep -q 'do not go together' tmp/gen-into.txt && fail "--generate-unmapped and --into are refused together again:\n$(cat tmp/gen-into.txt)"
grep -q 'import: root organizations/123456789012 . into' tmp/gen-into.txt \
  || fail "the delta import did not start with --generate-unmapped:\n$(cat tmp/gen-into.txt)"
grep -qi 'credential\|token\|auth\|ADC' tmp/gen-into.txt \
  || fail "the delta import stopped for a reason other than credentials:\n$(cat tmp/gen-into.txt)"
[ -d yaml/imported-state-generate ] && fail "a refused run wrote a scratch directory"
ls yaml | grep -q -- '-generate$' && fail "a run that reached no provider wrote a scratch directory"
ls yaml | grep -q -- '-generated.satz' && fail "a run that reached no provider wrote a generated estate"

step "import --as: the live sweep is told which estate's service account to read as"
# smoke.satz runs in local mode and impersonates nobody: --as would read as the
# caller while naming the estate, so it is refused before anything is swept.
if "$satz" --config . import organizations/123456789012 --as smoke.satz >tmp/import-as-local.txt 2>&1; then
  fail "--as on a local-mode estate must be refused"
fi
grep -q 'runs in local mode' tmp/import-as-local.txt \
  || fail "the --as refusal did not say the estate runs in local mode:\n$(cat tmp/import-as-local.txt)"
# The same estate in cloud mode names an account. Offline the run gets as far as
# the live sweep and stops there for want of credentials, which is what this can
# assert; the binding itself is unit-tested against a fixture estate
# (`import_identity`, src/main.rs).
sed 's/deployment_mode *= *"local"/deployment_mode = "cloud"/' yaml/smoke.satz > yaml/smoke-cloud.satz
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent CLOUDSDK_CONFIG=/nonexistent \
  "$satz" --config . import organizations/123456789012 --as smoke-cloud.satz >tmp/import-as.txt 2>&1; then
  fail "a sweep must not succeed without credentials:\n$(cat tmp/import-as.txt)"
fi
grep -q 'import: root organizations/123456789012' tmp/import-as.txt \
  || fail "--as did not reach the live sweep:\n$(cat tmp/import-as.txt)"
grep -qi 'credential\|token\|auth\|ADC' tmp/import-as.txt \
  || fail "the sweep stopped for a reason other than credentials:\n$(cat tmp/import-as.txt)"
# the scope must be the estate's organisation or inside it
if "$satz" --config . import organizations/222222222222 --as smoke-cloud.satz >tmp/import-as-other.txt 2>&1; then
  fail "--as must refuse a scope outside the estate's organisation"
fi
grep -q 'is bound to organizations/123456789012' tmp/import-as-other.txt \
  || fail "the refusal did not name the estate's organisation:\n$(cat tmp/import-as-other.txt)"
rm -f yaml/smoke-cloud.satz
# --into names the estate already, so the two together are refused, not reconciled
if "$satz" --config . import organizations/123456789012 --as smoke.satz --into smoke.satz >tmp/import-as-into.txt 2>&1; then
  fail "--as and --into must be refused together"
fi
grep -q 'cannot be used with' tmp/import-as-into.txt \
  || fail "the two flags are not refused together:\n$(cat tmp/import-as-into.txt)"
# the shapes that read a file call no Google API, so there is nobody to be
if "$satz" --config . import state.json --as smoke.satz >tmp/import-as-state.txt 2>&1; then
  fail "--as must be refused on the state shape"
fi
grep -q 'applies to the live shape' tmp/import-as-state.txt \
  || fail "the state refusal did not say which shape --as belongs to:\n$(cat tmp/import-as-state.txt)"

step "a YAML estate is refused by name, with the release that reads it"
printf 'variables:\n  a: &a 1\n' > tmp/old-estate.yaml
if "$satz" --config . import tmp/old-estate.yaml >tmp/yaml-refusal.txt 2>&1; then
  fail "satz import must refuse a YAML-dialect file"
fi
grep -q 'pre-Satz YAML dialect' tmp/yaml-refusal.txt || fail "the refusal did not name the dialect:\n$(cat tmp/yaml-refusal.txt)"
grep -q -- '--tag v' tmp/yaml-refusal.txt || fail "the refusal did not name the release that converts:\n$(cat tmp/yaml-refusal.txt)"
if "$satz" --config . transpile tmp/old-estate.yaml >tmp/yaml-transpile.txt 2>&1; then
  fail "transpile must refuse a YAML-dialect estate"
fi
grep -q 'pre-Satz YAML dialect' tmp/yaml-transpile.txt || fail "transpile's refusal did not name the dialect:\n$(cat tmp/yaml-transpile.txt)"

step "import, hcl shape: literal resources become Satz, positional ones wrap; --wrap-all wraps every block"
# --wrap-all translates nothing, so nothing names the organisation but the flag
if "$satz" --config . import tf --wrap-all -o imported-hcl.satz >tmp/import-hcl-no-org.txt 2>&1; then
  fail "--wrap-all without --organization must be refused"
fi
grep -q -- '--organization <n>' tmp/import-hcl-no-org.txt || fail "the refusal did not name the flag:\n$(cat tmp/import-hcl-no-org.txt)"
"$satz" --config . import tf --wrap-all --organization 123456789012 -o imported-hcl.satz --verbose | tee tmp/import-hcl.txt
grep -q 'wrapped verbatim' tmp/import-hcl.txt || fail "hcl import printed no summary"
"$satz" --config . transpile imported-hcl.satz --output "$PWD/tmp/imported-hcl-hcl" 2>&1 | tee tmp/transpile-hcl.txt
grep -q 'resource "google_storage_bucket" "logs"' tmp/imported-hcl-hcl/main.tf || fail "the wrapped bucket did not reach main.tf"
grep -q 'raw HCL passthrough' tmp/transpile-hcl.txt || fail "passthrough blocks must be announced"
"$satz" --config . import tf -o imported-hcl2.satz --verbose | tee tmp/import-hcl2.txt
grep -q '9 block(s) translated' tmp/import-hcl2.txt || fail "folder, project, service, grants and buckets should translate, the count over a list as two:\n$(cat tmp/import-hcl2.txt)"
grep -q '3 promoted to params' tmp/import-hcl2.txt || fail "both variables and the locals block should be promoted, not wrapped:\n$(cat tmp/import-hcl2.txt)"
grep -q 'no longer write the `depends_on`' tmp/import-hcl2.txt || fail "the dropped ordering edges were not reported:\n$(cat tmp/import-hcl2.txt)"
grep -q 'ordering   google_project_iam_member' tmp/import-hcl2.txt || fail "--verbose did not name the dropped edge:\n$(cat tmp/import-hcl2.txt)"
! grep -q 'depends_on' yaml/imported-hcl2.satz || fail "an ordering edge reached the estate; satz derives ordering itself"
"$satz" fmt --check yaml/imported-hcl2.satz || fail "the hcl import wrote a file that is not in the canonical layout"
! grep -q 'count.index' yaml/imported-hcl2.satz || fail "count.index reached the estate"
grep -q '^google_folder {' yaml/imported-hcl2.satz || fail "no translated folder in the estate"
"$satz" --config . transpile imported-hcl2.satz --output "$PWD/tmp/imported-hcl2-hcl" 2>&1 | tee tmp/transpile-hcl2.txt
grep -q 'lifecycle_rule {' tmp/imported-hcl2-hcl/main.tf || fail "the translated bucket lost its lifecycle_rule"
grep -q 'name *= *"corp-logs-001"' tmp/imported-hcl2-hcl/main.tf || fail "the promoted param did not resolve back to the source's literal"
grep -q 'resource "google_organization_iam_member"' tmp/imported-hcl2-hcl/main.tf || fail "the org grant was not emitted"
grep -q 'resource "google_storage_bucket_iam_member" "iam_group_gcp_auditors_example_com_' tmp/imported-hcl2-hcl/main.tf || fail "the pinned bucket grant did not emit"
grep -q 'provider *= *google-beta.google-beta' tmp/imported-hcl2-hcl/main.tf || fail "the translated bucket lost the provider alias it carried"
if command -v tofu >/dev/null 2>&1; then
  (cd tmp/imported-hcl2-hcl && tofu init -backend=false -input=false -no-color >/dev/null && tofu validate -no-color)
fi

step "import, hcl shape: a reference across the translated/verbatim boundary refuses and writes nothing"
if "$satz" --config . import tf-crossing -o imported-crossing.satz >tmp/import-crossing.txt 2>&1; then
  fail "the import must refuse a reference from a translated block to a wrapped one:\n$(cat tmp/import-crossing.txt)"
fi
grep -q 'so nothing was written' tmp/import-crossing.txt || fail "the refusal did not say that nothing was written:\n$(cat tmp/import-crossing.txt)"
grep -q 'references `google_storage_bucket.state`, which stays verbatim' tmp/import-crossing.txt || fail "the refusal did not name both sides:\n$(cat tmp/import-crossing.txt)"
grep -q -- '--wrap-all' tmp/import-crossing.txt || fail "the refusal did not say what to do:\n$(cat tmp/import-crossing.txt)"
[ ! -f yaml/imported-crossing.satz ] || fail "the refused import wrote an estate"
"$satz" --config . import tf-crossing --wrap-all --organization 123456789012 -o imported-crossing.satz >tmp/import-crossing-wrapped.txt 2>&1 \
  || fail "--wrap-all must still carry the same input:\n$(cat tmp/import-crossing-wrapped.txt)"
"$satz" --config . transpile imported-crossing.satz --output "$PWD/tmp/imported-crossing-hcl" >/dev/null 2>&1 \
  || fail "the --wrap-all estate must transpile"

step "scan: Checkov over the transpiled estate, findings pointed at the Satz source"
# Checkov is a stand-in on PATH that prints a report: the scanner is not what this
# judges, and the MCP step below reuses it.
mkdir -p tmp/fake-checkov
python3 - <<'PYEOF'
import json
report = {"check_type": "terraform", "results": {"failed_checks": [
    {"check_id": "CKV_GCP_62", "check_name": "Bucket should log access", "resource": "google_storage_bucket.state",
     "file_path": "/main.tf", "file_line_range": [1, 2], "guideline": None}]},
    "summary": {"passed": 3, "failed": 1, "skipped": 0, "parsing_errors": 0, "resource_count": 4, "checkov_version": "3.2.0"}}
open("tmp/checkov-fixture.json", "w").write(json.dumps(report))
PYEOF
printf '#!/bin/sh\ncat "%s/tmp/checkov-fixture.json"\nexit 1\n' "$PWD" > tmp/fake-checkov/checkov
chmod +x tmp/fake-checkov/checkov
PATH="$PWD/tmp/fake-checkov:$PATH" "$satz" --config . scan smoke.satz > tmp/scan.txt 2>&1 || true
grep -q '^scan: Checkov' tmp/scan.txt || fail "scan printed no summary:\n$(cat tmp/scan.txt)"
grep -q 'declared at' tmp/scan.txt || fail "findings were not pointed at the Satz source:\n$(cat tmp/scan.txt)"
PATH="$PWD/tmp/fake-checkov:$PATH" "$satz" --config . report-compliance cis-gcp-4.0 smoke.satz --no-live --checkov --format markdown --out tmp/evidence.md >/dev/null 2>&1 || fail "report-compliance --checkov failed"
grep -q '| Checkov |' tmp/evidence.md || fail "the evidence report has no Checkov column"

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
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . bootstrap smoke.satz --dry-run --no-default-grants \
  > tmp/boot-nogrants.txt 2>&1 || fail "bootstrap --no-default-grants must be accepted:\n$(cat tmp/boot-nogrants.txt)"

step "the root help is grouped, on every path that prints it, and the globals have their own heading"
# clap cannot group subcommands, so satz renders the root help itself. Four
# invocations reach that renderer and all four must produce the same groups.
for form in "--help" "-h" "help" ""; do
  # shellcheck disable=SC2086
  out=$(COLUMNS=80 "$satz" $form 2>&1)
  # grep reads the variable itself: `printf … | grep -q` fails under pipefail when
  # grep stops at the match while printf is still writing (SIGPIPE)
  for heading in "Estate:" "HCL:" "Presets:" "Policies:" "Compliance and audit:" "Tool:"; do
    grep -qx "$heading" <<<"$out" \
      || fail "\`satz ${form:-<no args>}\` has no '$heading' section — the grouped root help did not render"
  done
  grep -qx 'Global options:' <<<"$out" \
    || fail "\`satz ${form:-<no args>}\`: the globals lost their own heading"
done
# a command lands under its group, not just anywhere in the output
COLUMNS=80 "$satz" --help > tmp/help-root-80.txt 2>&1
awk '/^Policies:/{f=1;next} /^[A-Z].*:$/{f=0} f' tmp/help-root-80.txt > tmp/help-root-policies.txt
grep -q 'adopt-org-policies' tmp/help-root-policies.txt || fail "adopt-org-policies is not listed under Policies"
awk '/^HCL:/{f=1;next} /^[A-Z].*:$/{f=0} f' tmp/help-root-80.txt > tmp/help-root-hcl.txt
grep -q 'hcl-init' tmp/help-root-hcl.txt || fail "hcl-init is not listed under HCL"

step "update-prerequisites: the estate declares the roles and APIs its own types need, and the write closes a gap"
"$satz" --config . update-prerequisites smoke.satz > tmp/prereq.txt 2>&1 || fail "the smoke estate misses a prerequisite:\n$(cat tmp/prereq.txt)"
grep -q '^missing: none' tmp/prereq.txt || fail "update-prerequisites did not report the roles complete:\n$(cat tmp/prereq.txt)"
grep -q '^missing APIs: none' tmp/prereq.txt || fail "update-prerequisites did not report the APIs complete:\n$(cat tmp/prereq.txt)"
# a complete estate is not edited by a run that finds nothing
cmp -s yaml/smoke.satz "$root/tests/smoke/yaml/smoke.satz" || fail "update-prerequisites edited a complete estate"
# A gap in BOTH halves: the storage role and the monitoring API go, and the
# estate still has a bucket and alert policies.
grep -v '"roles/storage.admin",' yaml/smoke.satz | grep -v '"monitoring.googleapis.com",' > tmp/prereq-gap.satz
cp tmp/prereq-gap.satz tmp/prereq-gap-before.satz
"$satz" --config . transpile tmp/prereq-gap.satz --check > tmp/prereq-warn.txt 2>&1 || fail "a role gap failed the compile at the default level:\n$(cat tmp/prereq-warn.txt)"
grep -q 'roles/storage.admin at the organization — for google_storage_bucket' tmp/prereq-warn.txt \
  || fail "the warning does not name the role and the type:\n$(cat tmp/prereq-warn.txt)"
grep -q 'monitoring.googleapis.com — needed by google_monitoring_alert_policy' tmp/prereq-warn.txt \
  || fail "the warning does not name the API and the type:\n$(cat tmp/prereq-warn.txt)"
if "$satz" --config . --validation error transpile tmp/prereq-gap.satz --check > tmp/prereq-err.txt 2>&1; then
  fail "--validation error compiled an estate with a role gap"
fi
grep -q 'roles/storage.admin' tmp/prereq-err.txt || fail "the refusal does not name the role:\n$(cat tmp/prereq-err.txt)"
"$satz" --config . --validation none transpile tmp/prereq-gap.satz --check > tmp/prereq-none.txt 2>&1
if grep -q 'lacks roles' tmp/prereq-none.txt; then fail "--validation none still checked roles"; fi
if "$satz" --config . update-prerequisites tmp/prereq-gap.satz --report-only > tmp/prereq-dry.txt 2>&1; then
  fail "--report-only exited 0 on a gap:\n$(cat tmp/prereq-dry.txt)"
fi
# one role and one API, counted together: a prerequisite is a prerequisite
grep -q '2 prerequisite(s) missing' tmp/prereq-dry.txt || fail "--report-only does not count both halves of the gap:\n$(cat tmp/prereq-dry.txt)"
# declaring an API does not switch it on: the report prints the line that does
grep -q 'gcloud services enable monitoring.googleapis.com --project corp-infra-001' tmp/prereq-dry.txt \
  || fail "the report does not print the command that enables the API it would declare:\n$(cat tmp/prereq-dry.txt)"
cmp -s tmp/prereq-gap.satz tmp/prereq-gap-before.satz || fail "--report-only edited the estate"
"$satz" --config . update-prerequisites tmp/prereq-gap.satz > tmp/prereq-write.txt 2>&1 || fail "the write failed:\n$(cat tmp/prereq-write.txt)"
grep -q 'wrote roles/storage.admin in google_organization_iam_member' tmp/prereq-write.txt \
  || fail "update-prerequisites did not write the role:\n$(cat tmp/prereq-write.txt)"
"$satz" fmt --check tmp/prereq-gap.satz || fail "the write left a formatted estate unformatted"
[ "$(grep -c '^google_organization_iam_member {' tmp/prereq-gap.satz)" = 1 ] || fail "update-prerequisites added a second grant block"
"$satz" --config . update-prerequisites tmp/prereq-gap.satz --report-only > /dev/null 2>&1 || fail "a gap survived the write"
"$satz" update-prerequisites --format json > tmp/prereq-table.json
python3 - <<'PYEOF' || fail "update-prerequisites --format json did not print the table"
import json
t = json.load(open("tmp/prereq-table.json"))
assert t["read"], "no read entries"
row = t["types"]["google_project"]
# both halves of a prerequisite, per type: what it takes to be allowed, and what
# has to be switched on
assert any(e["roles"] == ["roles/resourcemanager.projectCreator"] for e in row["roles"]), row["roles"]
assert "cloudresourcemanager.googleapis.com" in row["apis"], row["apis"]
assert t["types"]["google_billing_budget"]["apis"] == ["billingbudgets.googleapis.com"], t["types"]["google_billing_budget"]
PYEOF

step "plan/apply replace an org policy the state holds with rules and the estate declares reset"
# After adopt moves a legacy twin onto its -superseded address, the state holds its
# rules under a declaration that says reset; updating that in place is refused by
# the API. A stand-in tool shows what satz hands to tofu.
mkdir -p tmp/reset/hcl/.terraform
cat > tmp/reset/config.toml <<EOF
yaml_dir = "."
hcl_dir = "hcl"
tf_tool = "$root/tests/smoke/scripts/fake-tofu.sh"
EOF
cat > tmp/reset/hcl/main.tf <<'EOF'
resource "google_org_policy_policy" "twin_superseded" {
  name   = "organizations/123456789012/policies/compute.vmCanIpForward"
  parent = "organizations/123456789012"
  spec {
    reset = true
  }
}
resource "google_org_policy_policy" "kept" {
  name   = "organizations/123456789012/policies/compute.managed.vmCanIpForward"
  parent = "organizations/123456789012"
  spec {
    rules {
      enforce = "TRUE"
    }
  }
}
EOF
cat > tmp/reset/state.json <<'EOF'
{"values": {"root_module": {"resources": [
  {"address": "google_org_policy_policy.twin_superseded", "mode": "managed", "type": "google_org_policy_policy",
   "values": {"id": "organizations/123456789012/policies/compute.vmCanIpForward", "spec": [{"reset": false, "rules": [{"enforce": "TRUE"}]}]}},
  {"address": "google_org_policy_policy.kept", "mode": "managed", "type": "google_org_policy_policy",
   "values": {"id": "organizations/123456789012/policies/compute.managed.vmCanIpForward", "spec": [{"reset": false, "rules": [{"enforce": "TRUE"}]}]}}
]}}}
EOF
export FAKE_TOFU_STATE="$PWD/tmp/reset/state.json"
"$satz" --config tmp/reset/config.toml apply -auto-approve > tmp/reset/apply.txt 2>&1 || fail "satz apply failed:\n$(cat tmp/reset/apply.txt)"
grep -q '^fake-tofu apply -auto-approve -replace=google_org_policy_policy.twin_superseded$' tmp/reset/apply.txt \
  || fail "apply did not replace the twin:\n$(cat tmp/reset/apply.txt)"
grep -q 'google_org_policy_policy.twin_superseded — the state holds it with rules' tmp/reset/apply.txt \
  || fail "apply did not say why it replaces:\n$(cat tmp/reset/apply.txt)"
if grep -q 'replace=google_org_policy_policy.kept' tmp/reset/apply.txt; then fail "a policy that keeps its rules was replaced"; fi
"$satz" --config tmp/reset/config.toml plan > tmp/reset/plan.txt 2>&1 || fail "satz plan failed:\n$(cat tmp/reset/plan.txt)"
grep -q '^fake-tofu plan -replace=google_org_policy_policy.twin_superseded$' tmp/reset/plan.txt \
  || fail "plan does not show the replace apply will make:\n$(cat tmp/reset/plan.txt)"
unset FAKE_TOFU_STATE

step "hcl-init: runs the tool's init in hcl_dir, and an estate among the arguments is refused with the command that works"
"$satz" --config tmp/reset/config.toml hcl-init -reconfigure > tmp/reset/init.txt 2>&1 \
  || fail "satz hcl-init failed:\n$(cat tmp/reset/init.txt)"
grep -q '^fake-tofu init -reconfigure$' tmp/reset/init.txt \
  || fail "hcl-init did not run the tool's init with its arguments:\n$(cat tmp/reset/init.txt)"
# the estate is not an argument for the tool: written like every other satz
# command it would reach `tofu init` as a positional and be refused there
if "$satz" --config tmp/reset/config.toml hcl-init smoke.satz > tmp/reset/init-estate.txt 2>&1; then
  fail "hcl-init handed a Satz estate to the tool:\n$(cat tmp/reset/init-estate.txt)"
fi
grep -q 'is a Satz estate' tmp/reset/init-estate.txt \
  || fail "hcl-init did not say what is wrong with the estate argument:\n$(cat tmp/reset/init-estate.txt)"
grep -q 'try: satz hcl-init' tmp/reset/init-estate.txt \
  || fail "the refusal does not name the command that works:\n$(cat tmp/reset/init-estate.txt)"

step "whoami: refuses without credentials naming the fix; reads an impersonated-SA ADC offline; answers for an estate"
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" whoami --offline > tmp/who.txt 2>&1; then
  fail "whoami --offline must fail without an ADC file:\n$(cat tmp/who.txt)"
fi
grep -q 'application-default login' tmp/who.txt || fail "whoami failure did not name the fix:\n$(cat tmp/who.txt)"
printf '{"type":"impersonated_service_account","service_account_impersonation_url":"https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/svc-iac@acme-infra-001.iam.gserviceaccount.com:generateAccessToken","quota_project_id":"acme-infra-001"}\n' > tmp/adc.json
GOOGLE_APPLICATION_CREDENTIALS="$PWD/tmp/adc.json" "$satz" whoami --offline > tmp/who2.txt 2>&1 \
  || fail "whoami --offline failed on a valid impersonated-SA ADC:\n$(cat tmp/who2.txt)"
grep -q 'svc-iac@acme-infra-001' tmp/who2.txt || fail "impersonation target not shown:\n$(cat tmp/who2.txt)"
grep -q 'impersonated service account' tmp/who2.txt || fail "credential type not shown:\n$(cat tmp/who2.txt)"
grep -q 'quota project: acme-infra-001' tmp/who2.txt || fail "quota project not shown:\n$(cat tmp/who2.txt)"
# BOTH halves, always: the credential, and the account the runs become
grep -q '^runs as: *svc-iac@acme-infra-001.* — no estate given' tmp/who2.txt \
  || fail "without an estate, the runs-as line must name the credential and say no estate was given:\n$(cat tmp/who2.txt)"
grep -q 'not checked (--offline)' tmp/who2.txt \
  || fail "--offline must say the live checks were not made, never imply they passed:\n$(cat tmp/who2.txt)"
# Given an estate, it answers who the estate acts as, from the estate file alone.
sed -e 's/^\(  deployment_mode *= *\)"local"/\1"cloud"/' yaml/smoke.satz > tmp/who-cloud.satz
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . whoami tmp/who-cloud.satz --offline > tmp/who3.txt 2>&1 \
  || fail "whoami <estate> failed without credentials, but it reads the estate file only:\n$(cat tmp/who3.txt)"
grep -q '^runs as: *svc-iac-001@corp-infra-001.* — impersonated by ' tmp/who3.txt \
  || fail "whoami <estate> did not name the estate's service account and who impersonates it:\n$(cat tmp/who3.txt)"

step "an estate whose identity cannot be derived is refused by migrate and every command that binds it"
# Cloud mode without `svc_iac_account` names no account to run as: each command
# refuses, naming the estate and the reason, and nothing runs as the login instead.
# (bound empty rather than removed: the estate's grants reference the param)
sed -e 's/^\(  deployment_mode *= *\)"local"/\1"cloud"/' -e 's/^\(  svc_iac_account *= *\)"svc-iac-001"/\1""/' \
  yaml/smoke.satz > tmp/mode-noaccount.satz
grep -q '^  svc_iac_account *= *""$' tmp/mode-noaccount.satz || fail "the no-account fixture still names an account"
cp tmp/mode-noaccount.satz tmp/mode-noaccount.before
# (the refusal reaches stderr in its Debug form, quotes escaped: the pattern takes both)
reason='mode-noaccount\.satz:[0-9]+: `deployment_mode = \\?"cloud\\?"` without a value for `svc_iac_account`'
identity_refused() {  # <output> <what was run>
  grep -Eq "$reason" "$1" || fail "$2 does not name the estate and the reason:\n$(cat "$1")"
  grep -q 'satz cannot tell which identity this estate runs as, and runs nothing for it' "$1" \
    || fail "$2 does not say that the identity cannot be derived:\n$(cat "$1")"
}
if "$satz" --config . migrate tmp/mode-noaccount.satz --mode cloud > tmp/id-migrate.txt 2>&1; then
  fail "migrate switched an estate with no identity:\n$(cat tmp/id-migrate.txt)"
fi
identity_refused tmp/id-migrate.txt "migrate"
cmp -s tmp/mode-noaccount.satz tmp/mode-noaccount.before || fail "the refused migrate edited the estate"
# report-compliance and adopt bind the estate's service account before they read anything
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . report-compliance cis-gcp-4.0 tmp/mode-noaccount.satz --no-live \
    --format json --out tmp/id-report.json > tmp/id-report.txt 2>&1; then
  fail "report-compliance ran for an estate with no identity:\n$(cat tmp/id-report.txt)"
fi
identity_refused tmp/id-report.txt "report-compliance"
[ -e tmp/id-report.json ] && fail "the refused report-compliance wrote a report"
if GOOGLE_APPLICATION_CREDENTIALS=/nonexistent "$satz" --config . adopt tmp/mode-noaccount.satz > tmp/id-adopt.txt 2>&1; then
  fail "adopt ran for an estate with no identity:\n$(cat tmp/id-adopt.txt)"
fi
identity_refused tmp/id-adopt.txt "adopt"

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

step "satz mcp: a client that never says hello is not an error"
# A request driver: send one, wait for its reply, send the next. A batch written
# straight into stdin is dispatched CONCURRENTLY by the server, so `satz_open`
# would race the calls that depend on it — and a racing gate is worse than none.
cat > tmp/mcp-drive.py <<'PYEOF'
import json
import subprocess
import sys

proc = subprocess.Popen(
    sys.argv[1:], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL, text=True, bufsize=1,
)
seen = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()
    if "id" not in req:
        continue
    while True:
        resp = proc.stdout.readline()
        if not resp:
            sys.exit("the server closed the stream")
        seen.append(resp)
        try:
            if json.loads(resp).get("id") == req["id"]:
                break
        except json.JSONDecodeError:
            # Kept in the output on purpose: stdout IS the protocol, and the
            # caller asserts that every line parses.
            continue
proc.stdin.close()
proc.wait(timeout=120)
sys.stdout.write("".join(seen))
PYEOF

# The first thing anyone does to check the command is run it by hand. Closed stdin
# means the client hung up before `initialize`, which is not a failure — exiting
# non-zero there teaches an operator to distrust a server that is working.
if ! printf '' | "$satz" mcp --root . >tmp/mcp-eof.txt 2>&1; then
  fail "a closed stdin made satz mcp exit non-zero:\n$(cat tmp/mcp-eof.txt)"
fi
grep -q 'before the client said hello' tmp/mcp-eof.txt \
  || fail "the EOF message does not explain itself:\n$(cat tmp/mcp-eof.txt)"

step "satz mcp: a real handshake, a real tool call, and the capability gate"
# stdout IS the protocol here, so the assertion is that EVERY line parses as
# JSON-RPC — the version banner or an emitter warning would each be a
# corrupt stream rather than cosmetic noise.
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"satz_estates","arguments":{}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":9,"method":"resources/list","params":{}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":10,"method":"resources/read","params":{"uri":"satz://guide"}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_require","arguments":{"estate":"smoke.satz","framework":"cis-gcp-4.0"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"satz_questions","arguments":{"estate":"showcase.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"satz_interview","arguments":{"estate":"showcase.satz","filter":"all"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"satz_interview","arguments":{"estate":"tmp/none.satz","create":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"satz_triage","arguments":{"estate":"smoke.satz","framework":"cis-gcp-4.0","prowler":"prowler.json"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"satz_report_compliance","arguments":{"estate":"smoke.satz","framework":"cis-gcp-4.0","no_live":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"satz_transpile","arguments":{"estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"satz_require","arguments":{"estate":"../../../README.md","framework":"cis-gcp-4.0"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"satz_scan_checkov","arguments":{"estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"satz_update_prerequisites","arguments":{"estate":"smoke.satz","report_only":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"satz_transpile_check","arguments":{"estate":"showcase.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"satz_transpile_check","arguments":{"estate":"tmp/refuse.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"satz_check_consumer","arguments":{"estate":"showcase.satz","dir":"consumer"}}}'
} > tmp/mcp-in.jsonl
# Satz text carries newlines and quotes, which do not survive a shell-quoted JSON
# line: python writes this one.
python3 - >> tmp/mcp-in.jsonl <<'PYEOF'
import json
text = 'estate e\n\nparams {\n  a  =  "1"\n}\n'
print(json.dumps({"jsonrpc": "2.0", "id": 20, "method": "tools/call",
                  "params": {"name": "satz_fmt", "arguments": {"text": text}}}))
PYEOF
# An estate that cannot compile: it writes a reference to a folder nobody emits.
# The refusal must carry that finding at its line, not only the sentence.
sed 's|name *= *"{customer_shortname}-audit-logs"|name = "${{google_folder.nope.name}}"|' yaml/showcase.satz > tmp/refuse.satz
grep -q 'google_folder.nope.name' tmp/refuse.satz || fail "the refusing fixture was not written"
python3 tmp/mcp-drive.py "$satz" mcp --root . < tmp/mcp-in.jsonl > tmp/mcp.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the MCP handshake did not behave"
import json
lines = [l for l in open("tmp/mcp.jsonl") if l.strip()]
assert lines, "the server answered nothing"
msgs = {}
for l in lines:
    d = json.loads(l)          # every line must parse: stdout is the protocol
    if "id" in d:
        msgs[d["id"]] = d

assert msgs[1]["result"]["serverInfo"]["name"] == "satz", msgs[1]

# A client that only speaks MCP has no repository to read. The language guide has to
# travel with the server, and the instructions have to point at it — otherwise an
# agent knows how to CALL satz and not how to write the language the calls are about.
assert "resources" in msgs[1]["result"]["capabilities"], msgs[1]["result"]["capabilities"]
assert "satz://guide" in msgs[1]["result"].get("instructions", ""), "the instructions do not send the agent to the guide"
# An agent must also learn what it CANNOT do here, or it improvises around it —
# writing HCL by hand because `apply` is absent. MCP_PARITY renders both halves.
instructions = msgs[1]["result"]["instructions"]
assert "transpile -> satz_transpile" in instructions, instructions
assert "apply (it hands stdio to the tool" in instructions, instructions
uris = {r["uri"] for r in msgs[9]["result"]["resources"]}
assert {"satz://guide", "satz://reference", "satz://presets"} <= uris, uris
guide = msgs[10]["result"]["contents"][0]["text"]
assert guide.startswith("# satz llms"), guide[:80]
assert "Never edit `hcl/`" in guide, "the guide lost its hard rules"
tools = {t["name"]: t for t in msgs[2]["result"]["tools"]}
# EXACTLY these: a tool that ships without a step in this matrix is exercised by
# nothing, and `cargo test` only holds the list against MCP_PARITY and the docs
assert set(tools) == {"satz_require", "satz_check_presets", "satz_questions", "satz_interview", "satz_triage",
                      "satz_prowler",
                      "satz_transpile_check", "satz_transpile", "satz_report_compliance",
                      "satz_whoami", "satz_open", "satz_estates", "satz_scan_checkov",
                      "satz_remediation_items", "satz_remediation_annotate", "satz_adopt", "satz_get_presets",
                      "satz_update_prerequisites", "satz_merge_presets", "satz_restrict", "satz_review_pack", "satz_fmt",
                      "satz_packs", "satz_add_pack", "satz_remove_pack", "satz_check_consumer"}, sorted(tools)

# The server holds no estate until a client opens one, so it has to be able to
# say which ones it could open — otherwise the first call is a guess at a path.
found = msgs[11]["result"]["structuredContent"]
assert any(e["estate"].endswith("smoke.satz") for e in found["estates"]), found
assert any(e["deployment_mode"] == "local" for e in found["estates"]), found
# satz_fmt: text in, canonical text out, and satz wrote no file
fmt = msgs[20]["result"]["structuredContent"]
assert fmt["changed"] is True, fmt
assert fmt["formatted"] == 'estate e\n\nparams {\n  a = "1"\n}\n', fmt["formatted"]

opened = msgs[12]["result"]["structuredContent"]
assert opened["estate"].endswith("smoke.satz"), opened
assert opened["deployment_mode"] == "local", opened
# A local-mode estate impersonates nothing: the calls ARE the ADC identity.
assert opened["runs_as"] is None, opened

# Every data tool publishes an OUTPUT SCHEMA and is ANNOTATED. The annotations are
# the client's half of the safety model: the server's --allow ceiling says what is
# permitted, readOnlyHint says what an agent may run without stopping to ask.
for name in ("satz_require", "satz_questions", "satz_interview", "satz_triage", "satz_prowler", "satz_check_presets",
             "satz_transpile_check", "satz_transpile", "satz_report_compliance",
             "satz_whoami", "satz_scan_checkov", "satz_remediation_items", "satz_remediation_annotate",
             "satz_adopt", "satz_get_presets", "satz_update_prerequisites", "satz_merge_presets",
             "satz_packs", "satz_add_pack", "satz_remove_pack", "satz_check_consumer"):
    assert tools[name].get("outputSchema"), f"{name} publishes no output schema"
    ann = tools[name].get("annotations") or {}
    assert "readOnlyHint" in ann, f"{name} carries no annotations: {ann}"
assert tools["satz_require"]["annotations"]["readOnlyHint"] is True
assert tools["satz_transpile"]["annotations"]["readOnlyHint"] is False
assert tools["satz_packs"]["annotations"]["readOnlyHint"] is True
assert tools["satz_check_consumer"]["annotations"]["readOnlyHint"] is True
# satz_check_consumer: the team's one attachment off an attach point, at its line
consumer = msgs[19]["result"]["structuredContent"]
assert consumer["resources"] == 2, consumer
assert [(f["kind"], f["file"].endswith("consumer/main.tf"), f["line"]) for f in consumer["findings"]] == [("consumer", True, 16)], consumer
assert tools["satz_add_pack"]["annotations"]["readOnlyHint"] is False

# the role gap an agent asks about: the smoke estate grants what it emits, and the
# answer names the service account it judged
roles = msgs[16]["result"]["structuredContent"]["report"]
assert roles["service_account"].startswith("serviceAccount:") or "@" in roles["service_account"], roles
assert roles["missing"] == [], f"the smoke estate lacks roles it emits types for: {roles['missing']}"
assert roles["needs"], "no role need was derived at all"
assert msgs[16]["result"]["structuredContent"]["written"] == [], "a read-level call wrote grants"
# What the compile warned about reaches an agent as data — the showcase declares an
# action and a trusted passthrough, so the check returns both, each at its line.
chk = msgs[17]["result"]["structuredContent"]
assert chk["addresses"], chk
kinds = {f["kind"] for f in chk["findings"]}
assert {"action", "hcl-passthrough"} <= kinds, chk["findings"]
# a finding with no site (the pack-actions note) carries no line at all
assert all(f.get("line") for f in chk["findings"] if f["kind"] in ("action", "hcl-passthrough") and f["severity"] != "info"), chk["findings"]
# A REFUSED check is an error result that still carries its findings, each at its
# line — a client shows them where they are instead of parsing the sentence.
bad = msgs[18]["result"]
assert bad.get("isError"), bad
assert "does not emit" in bad["content"][0]["text"], bad["content"]
ref = [f for f in bad["structuredContent"]["findings"] if f["kind"] == "written-reference"]
assert ref and ref[0]["severity"] == "error" and ref[0]["line"], bad["structuredContent"]
assert bad["structuredContent"]["addresses"] == [], "a refused compile emitted nothing"

# a granted tool returns the report as STRUCTURED content, not a string to parse
rep = msgs[3]["result"]["structuredContent"]
assert rep["summary"]["unmet"] == 14, rep["summary"]
q = msgs[6]["result"]["structuredContent"]
assert q["summary"]["one_way_doors"] >= 1, q["summary"]
# the interview at read level: it reports, and it will not create
iv = msgs[13]["result"]["structuredContent"]
assert iv["created"] is False and iv["written"] == 0 and iv["summary"]["complete"] is True, iv
assert len(iv["questions"]) == 5, "filter: all returns every question"
create = msgs[14]["result"]
assert create["isError"] is True and "needs 'write'" in create["content"][0]["text"], create
rows = msgs[7]["result"]["structuredContent"]["rows"]
assert rows and {"bucket", "control"} <= set(rows[0]), rows[:1]
ev = msgs[8]["result"]["structuredContent"]
assert ev["rows"], "the evidence report came back empty"
assert ev["live"] is False, ev["live"]

# a tool outside the granted level is refused as a tool RESULT, not a protocol
# error — agents recover from the first and give up on the second
assert msgs[4]["result"]["isError"] is True, msgs[4]
assert "needs 'write'" in msgs[4]["result"]["content"][0]["text"], msgs[4]

# and a path that escapes the server's root is refused by name
assert msgs[5]["result"]["isError"] is True, msgs[5]
assert "outside the server's root" in msgs[5]["result"]["content"][0]["text"], msgs[5]

# Checkov is an external tool: the exec group, refused at read, and not read-only —
# a client runs a read-only tool without asking
assert tools["satz_scan_checkov"]["annotations"]["readOnlyHint"] is False
assert msgs[15]["result"]["isError"] is True and "needs 'exec'" in msgs[15]["result"]["content"][0]["text"], msgs[15]

PYEOF

step "satz mcp: satz_transpile writes the HCL it compiled, at the write level"
rm -f hcl/main.tf
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_transpile","arguments":{}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"satz_remediation_items","arguments":{"framework":"cis-gcp-4.0","prowler":"prowler.json"}}}'
} > tmp/mcp-write-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/mcp-write-in.jsonl > tmp/mcp-write.jsonl 2>/dev/null || true
[ -s hcl/main.tf ] || fail "satz_transpile at the write level wrote no main.tf:\n$(cat tmp/mcp-write.jsonl)"
head -1 hcl/main.tf | grep -q 'Generated by satz v.*smoke.satz' || fail "the MCP-written main.tf carries no provenance line:\n$(head -1 hcl/main.tf)"
python3 - <<'PYEOF' || fail "satz_transpile did not report what it wrote"
import json
msgs = {d["id"]: d for d in (json.loads(l) for l in open("tmp/mcp-write.jsonl") if l.strip()) if "id" in d}
r = msgs[3]["result"]["structuredContent"]
assert any(w.endswith("main.tf") for w in r["written"]), r
assert r["addresses"], r
items = msgs[4]["result"]["structuredContent"]
assert items["items"] and len(items["dossier_sha256"]) == 64, items
# hand the worklist to the next session: the item id and the hash authored values must name
open("tmp/mcp-items.json", "w").write(json.dumps({"id": items["items"][0]["id"], "hash": items["dossier_sha256"]}))
PYEOF
python3 - <<'PYEOF' || fail "could not build the annotate request"
import json
w = json.load(open("tmp/mcp-items.json"))
call = {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "satz_remediation_annotate", "arguments": {
    "framework": "cis-gcp-4.0", "prowler": "prowler.json", "out": "tmp/mcp-plan", "dossier_sha256": w["hash"],
    "items": {w["id"]: {"what_why": "written through MCP", "authored_by": "smoke via satz mcp", "authored_at": "2026-09-11T20:00:00Z"}}}}}
lines = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "1"}}},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "satz_open", "arguments": {"config": ".", "estate": "smoke.satz"}}},
    call,
]
open("tmp/mcp-annotate-in.jsonl", "w").write("\n".join(json.dumps(l) for l in lines) + "\n")
PYEOF
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/mcp-annotate-in.jsonl > tmp/mcp-annotate.jsonl 2>/dev/null || true
grep -q 'written through MCP' tmp/mcp-plan/findings.csv 2>/dev/null || fail "satz_remediation_annotate did not render the authored value:\n$(cat tmp/mcp-annotate.jsonl)"
grep -q '"authored_by": "smoke via satz mcp"' tmp/mcp-plan/authored.json || fail "authored.json does not name the author"

step "satz mcp: Checkov runs in satz_scan_checkov, and the read-only remediation tools read its report"
# A client runs a read-only tool without asking, so satz_remediation_items runs
# nothing: satz_scan_checkov (exec, and write for `out`) runs Checkov and writes its
# report, and the worklist reads that file. Checkov is the scan step's stand-in.
rm -f tmp/checkov-report.json
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_scan_checkov","arguments":{"out":"tmp/checkov-report.json"}}}'
} > tmp/mcp-scan-in.jsonl
PATH="$PWD/tmp/fake-checkov:$PATH" python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write,exec < tmp/mcp-scan-in.jsonl > tmp/mcp-scan.jsonl 2>/dev/null || true
cmp -s tmp/checkov-fixture.json tmp/checkov-report.json \
  || fail "satz_scan_checkov with out did not write Checkov's report as Checkov wrote it:\n$(cat tmp/mcp-scan.jsonl)"
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"satz_remediation_items","arguments":{"framework":"cis-gcp-4.0","prowler":"prowler.json","checkov":"tmp/checkov-report.json"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"satz_remediation_items","arguments":{"framework":"cis-gcp-4.0","prowler":"prowler.json","checkov":true}}}'
} > tmp/mcp-ck-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . < tmp/mcp-ck-in.jsonl > tmp/mcp-ck.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the Checkov report did not travel from satz_scan_checkov to satz_remediation_items"
import json
scan = {d["id"]: d for d in (json.loads(l) for l in open("tmp/mcp-scan.jsonl") if l.strip()) if "id" in d}
s = scan[3]["result"]["structuredContent"]
assert s["written"].endswith("checkov-report.json") and s["checkov_version"] == "3.2.0", s
assert s["findings"][0]["declared_at"], "the finding was not pointed at the Satz source"
msgs = {d["id"]: d for d in (json.loads(l) for l in open("tmp/mcp-ck.jsonl") if l.strip()) if "id" in d}
tools = {t["name"]: t for t in msgs[3]["result"]["tools"]}
assert tools["satz_remediation_items"]["annotations"]["readOnlyHint"] is True
assert tools["satz_scan_checkov"]["annotations"]["readOnlyHint"] is False
# at the read level: the report is read, and its finding joins the dossier
items = msgs[4]["result"]["structuredContent"]["items"]
assert any(src.get("scanner") == "checkov" and src["check_id"] == "CKV_GCP_62" for i in items for src in i["sources"]), items[:2]
# a switch is not a report: refused, never read as "no Checkov"
bad = msgs[5]
assert "error" in bad or bad["result"].get("isError"), bad
PYEOF

step "satz mcp: one server works through estates in turn, each as its own identity"
# The identity a live tool runs as is invisible in its output, so it is asserted
# here. Two cloud-mode estates with DIFFERENT service accounts, opened in turn in
# ONE server: each must answer as its own, and re-opening the first must come back
# to the first. Nothing is configured — the account is derived from the estate.
# Offline throughout: whoami reports the target without minting a token.
for c in acme bolt; do
  cat > "yaml/identity-$c.satz" <<EOF
estate identity_$c

params {
  customer_organization_id = "123456789012"
  customer_id = "C0TEST"
  customer_domain = "example.com"
  customer_shortname = "$c"
  infra_project_name = "$c-infra-001"
  svc_iac_account = "svc-iac-001"
  deployment_engine = "tofu"
  deployment_mode = "cloud"
}
EOF
done
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"satz_require","arguments":{"framework":"cis-gcp-4.0"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"identity-acme.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"satz_whoami","arguments":{"offline":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"identity-bolt.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"satz_whoami","arguments":{"offline":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":24,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"identity-acme.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":25,"method":"tools/call","params":{"name":"satz_whoami","arguments":{"offline":true}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":26,"method":"tools/call","params":{"name":"satz_whoami","arguments":{"offline":true,"estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":29,"method":"tools/call","params":{"name":"satz_adopt","arguments":{"estate":"tmp/mode-noaccount.satz"}}}'
} > tmp/mcp-id-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . < tmp/mcp-id-in.jsonl > tmp/mcp-id.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the identity did not follow the open estate"
import json
msgs = {}
for l in open("tmp/mcp-id.jsonl"):
    if l.strip():
        d = json.loads(l)      # stdout is still the protocol
        if "id" in d:
            msgs[d["id"]] = d

# Before anything is open there is no estate to be, and the refusal has to say
# how to fix that rather than merely failing.
first = msgs[19]["result"]
assert first.get("isError"), first
assert "satz_open" in first["content"][0]["text"], first

for open_id, who_id, want in ((20, 21, "acme"), (22, 23, "bolt"), (24, 25, "acme")):
    opened = msgs[open_id]["result"]
    assert not opened.get("isError"), f"opening {want} was refused: {opened}"
    sa = f"svc-iac-001@{want}-infra-001.iam.gserviceaccount.com"
    assert opened["structuredContent"]["runs_as"] == sa, opened
    assert opened["structuredContent"]["deployment_mode"] == "cloud", opened

    who = msgs[who_id]["result"]
    assert not who.get("isError"), f"whoami after opening {want} was refused: {who}"
    # BOTH halves: the estate's service account is what the tools RUN as, and the
    # ADC is the credential that becomes it. Reporting only one is what this
    # report shape exists to prevent.
    reported = who["structuredContent"]
    assert reported["estate"]["service_account"] == sa, reported
    assert "adc" in reported and "kind" in reported["adc"], reported
    # Offline: the checks were not made, and must not be reported as passed.
    assert reported["estate"]["may_impersonate"] is None, reported
    assert reported["estate"]["impersonated"] is True, reported
    assert reported["estate"]["deployment_mode"] == "cloud", reported

# A local-mode estate named while a cloud-mode one is open: answered for the one
# named, and the data carries what the terminal line says — its mode, the account it
# declares, and that nothing impersonates it.
local = msgs[26]["result"]
assert not local.get("isError"), f"whoami for a named local-mode estate was refused: {local}"
est = local["structuredContent"]["estate"]
assert est["deployment_mode"] == "local", est
assert est["impersonated"] is False, est
assert est["service_account"] == "svc-iac-001@corp-infra-001.iam.gserviceaccount.com", est

# A live tool naming an estate whose identity cannot be derived is refused with the
# reason; nothing runs as the credentials instead.
r = msgs[29]["result"]
assert r.get("isError"), f"adopt on an estate with no identity was not refused: {r}"
text = r["content"][0]["text"]
assert "without a value for `svc_iac_account`" in text, text
assert "satz cannot tell which identity this estate runs as, and runs nothing for it" in text, text
PYEOF

step "satz mcp: the interview loop closes without a filesystem — create, answer, accept, complete"
# An agent that only speaks MCP cannot edit the estate itself. `answers` and
# `accept_defaults` are how it writes what the human decided; the report it gets
# back is the estate as it now stands.
rm -f tmp/iv/agent.satz
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_interview\",\"arguments\":{\"estate\":\"$PWD/tmp/iv/agent.satz\",\"create\":true}}}"
  printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_interview\",\"arguments\":{\"estate\":\"$PWD/tmp/iv/agent.satz\",\"answers\":{\"customer_id\":\"C0example\",\"customer_organization_id\":\"123456789012\",\"customer_domain\":\"example.com\",\"customer_shortname\":\"acme\",\"customer_longname\":\"Acme\",\"first_admin\":\"first.admin\",\"billing_account_infra\":\"012345-6789AB-CDEF01\"},\"accept_defaults\":true}}}"
  printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_interview\",\"arguments\":{\"estate\":\"$PWD/tmp/iv/agent.satz\",\"answers\":{\"nobody\":\"x\"}}}}"
} > tmp/mcp-iv-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/mcp-iv-in.jsonl > tmp/mcp-iv.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the MCP interview loop did not close"
import json
msgs = {}
for l in open("tmp/mcp-iv.jsonl"):
    if l.strip():
        d = json.loads(l)
        if "id" in d:
            msgs[d["id"]] = d
a = msgs[3]["result"]["structuredContent"]
assert a["created"] is True, a
# `create` over MCP writes the same DAY-0 file the CLI does: the estate's own eighteen,
# and every pack commented out under its phase. Nine of the eighteen need a typed value
# until their inputs land; the map's choices are not asked at all, because the map is not
# in yet.
assert (a["summary"]["unanswered"], a["summary"]["blocking"]) == (18, 9), a["summary"]
assert len(a["questions"]) == 18 and all(q["state"] == "unanswered" for q in a["questions"]), "the default filter is the worklist"
assert "use_scc_notifications" not in {q["subject"] for q in a["questions"]}, "no pack choice is asked on a day-0 file"
assert "security_model" not in {q["subject"] for q in a["questions"]}, "the map is commented out, so its choices are not asked"
by = {q["subject"]: q for q in a["questions"]}
assert by["infra_project_name"]["blocking"] is True, "a name derived from an unanswered input is not a default"
assert by["default_zone"]["default"] == "europe-west3-a", by["default_zone"]
assert "day 0" in by["customer_id"]["pack_description"], by["customer_id"]["pack_description"]
b = msgs[4]["result"]["structuredContent"]
# 7 answers, then every remaining default: the estate's own eighteen questions
assert b["written"] == 18 and b["summary"]["complete"] is True, b["summary"]
assert "security_model" not in str(b), "a day-0 file has no map, so no choice is answered here"
assert b["rename_to"] == "C0example.satz", b
assert b["questions"] == [], "nothing is open once every answer landed"
r = msgs[5]["result"]
assert r["isError"] is True and "no pack this estate uses asks that" in r["content"][0]["text"], r
PYEOF
grep -qE '^  security_model_s[12] += ' tmp/iv/agent.satz && fail "a day-0 file has no map, so no choice is bound in it"
grep -q '^export "workload_folder" = "organizations/{customer_organization_id}"' tmp/iv/agent.satz \
  || fail "satz_interview must write the workload folder's section as the CLI interview does"
# A SECOND ROUND, once the map is in: an agent answers the exclusive choice, and the tool
# writes it as two booleans AND uncomments that model's pack line. Same code as the CLI —
# `satz_interview` calls `interview::apply` — so this is the parity the table promises.
sed -i.bak 's|^// use "presets/estate-map.satz"|use "presets/estate-map.satz"|' tmp/iv/agent.satz
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}' \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_interview\",\"arguments\":{\"estate\":\"$PWD/tmp/iv/agent.satz\",\"answers\":{\"security_model\":\"security_model_s2\"},\"accept_defaults\":true}}}" \
  > tmp/mcp-iv2-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/mcp-iv2-in.jsonl > tmp/mcp-iv2.jsonl 2>/dev/null || true
"$satz" fmt --check "$PWD/tmp/iv/agent.satz" || fail "the MCP interview left a formatted estate unformatted"
grep -qE 'security_model_s2 += true' tmp/iv/agent.satz || fail "the oneof answer was not written by the MCP tool"
grep -qE 'security_model_s1 += false' tmp/iv/agent.satz || fail "the oneof must set the siblings false"
grep -q '^use "presets/security-group-models/s2-security-groups.satz" when security_model_s2' tmp/iv/agent.satz \
  || fail "answering the choice must uncomment that model's pack line:\n$(grep 'security-group-models' tmp/iv/agent.satz)"
grep -q '^// use "presets/security-group-models/s1-security-groups.satz"' tmp/iv/agent.satz \
  || fail "the model that was not chosen keeps its line commented"
"$satz" --config . transpile "$PWD/tmp/iv/agent.satz" --check > /dev/null 2>&1 || fail "the agent-interviewed estate does not compile"

step "packs, add-pack and remove-pack: one pack logic over the shipped graph, the same in the CLI, the compile and MCP"
# The interviewed day-0 estate: the map is commented, so every pack line is inert.
rm -rf tmp/pk && mkdir -p tmp/pk && cp tmp/iv/new.satz tmp/pk/e.satz
"$satz" --config . add-pack "$PWD/tmp/pk/e.satz" presets/estate-map.satz > tmp/pk/map.txt 2>&1 || fail "add-pack of the map failed:\n$(cat tmp/pk/map.txt)"
grep -q '^use "presets/estate-map.satz"' tmp/pk/e.satz || fail "add-pack did not uncomment the map's line"
# a pack whose provider is off is refused, naming the provider's gate
if "$satz" --config . add-pack "$PWD/tmp/pk/e.satz" use_central_alerts > tmp/pk/refused.txt 2>&1; then
  fail "add-pack of the central alerts with the logsink off was not refused"
fi
grep -q 'organization-audit-logsink.satz` (`use_audit_logsink`), which is off' tmp/pk/refused.txt \
  || fail "the refusal must name the logsink's gate:\n$(cat tmp/pk/refused.txt)"
"$satz" --config . add-pack "$PWD/tmp/pk/e.satz" use_central_alerts --with-requirements > tmp/pk/add.txt 2>&1 \
  || fail "add-pack --with-requirements failed:\n$(cat tmp/pk/add.txt)"
grep -qE '^use "presets/monitoring/organization-audit-logsink.satz" when use_audit_logsink' tmp/pk/e.satz \
  || fail "the requirement's line was not switched on at the top level:\n$(grep -n logsink tmp/pk/e.satz)"
grep -q 'cis_central_email' tmp/pk/add.txt || fail "add-pack must name the questions the pack opened:\n$(cat tmp/pk/add.txt)"
"$satz" fmt --check "$PWD/tmp/pk/e.satz" || fail "add-pack left a formatted estate unformatted"
# a pack others need is refused, naming them; --cascade switches them off too
if "$satz" --config . remove-pack "$PWD/tmp/pk/e.satz" use_audit_logsink > tmp/pk/rm.txt 2>&1; then
  fail "remove-pack of the logsink with the central alerts on was not refused"
fi
grep -q 'organization-cis-log-alerts-central.satz` needs' tmp/pk/rm.txt || fail "the refusal must name the dependent:\n$(cat tmp/pk/rm.txt)"
# the compile names the same requirement when the estate is edited by hand
sed 's/^\(  use_audit_logsink *= \)true/\1false/' tmp/pk/e.satz > tmp/pk/off.satz
if "$satz" --config . transpile "$PWD/tmp/pk/off.satz" --check > tmp/pk/off.txt 2>&1; then
  fail "the estate with the logsink off under the central alerts compiled"
fi
grep -q 'organization-audit-logsink.satz` (`use_audit_logsink`), which is off' tmp/pk/off.txt \
  || fail "transpile --check must print the pack graph's sentence:\n$(cat tmp/pk/off.txt)"
"$satz" --config . packs "$PWD/tmp/pk/e.satz" --format json --out tmp/pk/packs.json > /dev/null 2>&1 || fail "satz packs failed"
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}' \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_packs\",\"arguments\":{\"estate\":\"$PWD/tmp/pk/e.satz\"}}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_remove_pack\",\"arguments\":{\"estate\":\"$PWD/tmp/pk/e.satz\",\"pack\":\"use_audit_logsink\",\"cascade\":true}}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_add_pack\",\"arguments\":{\"estate\":\"$PWD/tmp/pk/e.satz\",\"pack\":\"use_audit_logsink\"}}}" \
  > tmp/pk/mcp-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/pk/mcp-in.jsonl > tmp/pk/mcp.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the pack tools over MCP disagree with the CLI"
import json
msgs = {d["id"]: d for d in (json.loads(l) for l in open("tmp/pk/mcp.jsonl") if l.strip()) if "id" in d}
cli = json.load(open("tmp/pk/packs.json"))
served = msgs[3]["result"]["structuredContent"]
assert served["packs"] == cli["packs"], "satz_packs and `satz packs --format json` report different rows"
row = {p["path"]: p for p in served["packs"]}
assert row["presets/monitoring/organization-cis-log-alerts-central.satz"]["deploys"] is True
assert row["presets/monitoring/organization-cis-log-alerts-central.satz"]["requires"][0]["met"] is True
removed = msgs[4]["result"]["structuredContent"]
assert {b["param"] for b in removed["bound"]} == {"use_audit_logsink", "use_central_alerts"}, removed
added = msgs[5]["result"]["structuredContent"]
assert added["bound"] == [{"param": "use_audit_logsink", "value": True}], added
PYEOF

step "a pack's notice: named when the pack goes on, open until the estate binds its param, and apply refuses"
# The CIS baseline asks for `satz adopt` before the first apply — Google sets some of its
# policies on every new organisation, and creating one that exists stops the apply.
"$satz" --config . add-pack "$PWD/tmp/pk/e.satz" use_cis_baseline > tmp/pk/notice.txt 2>&1 \
  || fail "add-pack of the CIS baseline failed:\n$(cat tmp/pk/notice.txt)"
grep -q 'notice — presets/cis/CIS-GCP-Foundation-4.0.satz' tmp/pk/notice.txt \
  || fail "switching the pack on must name its notice:\n$(cat tmp/pk/notice.txt)"
grep -q 'bind `cis_baseline_adopted = true`' tmp/pk/notice.txt || fail "the notice must say how it is acknowledged"
"$satz" --config . transpile "$PWD/tmp/pk/e.satz" --check > tmp/pk/notice-check.txt 2>&1 \
  || fail "the estate with an open notice must compile:\n$(cat tmp/pk/notice-check.txt)"
grep -q 'notices open — what a pack asks to be run once it is on (1)' tmp/pk/notice-check.txt || fail "the compile must warn while a notice is open:\n$(cat tmp/pk/notice-check.txt)"
# the layout: the first line, the sentence under it, the command as a last line of its own
grep -q '^warning  notice  *tmp/pk/e.satz:[0-9]*  cis_baseline_adopted$' tmp/pk/notice-check.txt || fail "a finding opens with severity, kind, file:line and subject:\n$(cat tmp/pk/notice-check.txt)"
# the estate is named as a command takes it — this one is given as an absolute path, and
# that is what the line has to carry: the file name alone would name yaml/e.satz, another file
grep -q "^    fix: satz adopt $PWD/tmp/pk/e.satz --execute --import$" tmp/pk/notice-check.txt || fail "the command is the finding's last line, naming the estate as a command takes it:\n$(cat tmp/pk/notice-check.txt)"
# the acknowledgement is an answer: `satz_interview` takes it, and `satz_packs` reports both states
"$satz" --config . packs "$PWD/tmp/pk/e.satz" --format json --out tmp/pk/notice-packs.json > /dev/null 2>&1 || fail "satz packs failed"
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}' \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_interview\",\"arguments\":{\"estate\":\"$PWD/tmp/pk/e.satz\",\"answers\":{\"cis_baseline_adopted\":true}}}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"satz_packs\",\"arguments\":{\"estate\":\"$PWD/tmp/pk/e.satz\"}}}" \
  > tmp/pk/notice-in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/pk/notice-in.jsonl > tmp/pk/notice-mcp.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the notice is not carried through satz packs and satz_interview"
import json
cli = {p["path"]: p for p in json.load(open("tmp/pk/notice-packs.json"))["packs"]}
n = cli["presets/cis/CIS-GCP-Foundation-4.0.satz"]["notices"]
assert len(n) == 1 and n[0]["param"] == "cis_baseline_adopted", n
assert n[0]["acknowledged"] is False and n[0]["severity"] == "error", n
assert n[0]["run"].startswith("satz adopt"), n
msgs = {d["id"]: d for d in (json.loads(l) for l in open("tmp/pk/notice-mcp.jsonl") if l.strip()) if "id" in d}
interview = msgs[3]["result"]["structuredContent"]
assert interview["written"] == 1, interview
assert interview["notices"] == [], "answering opened no pack, so it opened no notice"
after = {p["path"]: p for p in msgs[4]["result"]["structuredContent"]["packs"]}
assert after["presets/cis/CIS-GCP-Foundation-4.0.satz"]["notices"][0]["acknowledged"] is True
PYEOF
"$satz" --config . transpile "$PWD/tmp/pk/e.satz" --check > tmp/pk/notice-done.txt 2>&1 \
  || fail "the acknowledged estate does not compile:\n$(cat tmp/pk/notice-done.txt)"
grep -q 'notices open' tmp/pk/notice-done.txt && fail "the acknowledged notice still warns:\n$(cat tmp/pk/notice-done.txt)"
# and the gate: an estate whose questions are all answered is still refused while a notice is open
rm -rf tmp/nt && mkdir -p tmp/nt
sed 's/pack_bucket_adopted      = true/pack_bucket_adopted      = false/' yaml/showcase.satz > tmp/nt/open-notice.satz
# it compiles — the command that closes the notice compiles the estate too — and the
# acknowledgement reaches no emitted file
"$satz" --config . transpile "$PWD/tmp/nt/open-notice.satz" --output "$PWD/tmp/nt/hcl" > tmp/nt/transpile.txt 2>&1 \
  || fail "an estate with an open notice must compile:\n$(cat tmp/nt/transpile.txt)"
grep -q 'adopted' tmp/nt/hcl/variables.tf tmp/nt/hcl/terraform.tfvars && fail "an acknowledgement is never emitted"
# the pack declares `severity = error`, so the run that writes to the organisation refuses
if "$satz" --config . transpile "$PWD/tmp/nt/open-notice.satz" --apply --output "$PWD/tmp/nt/hcl" > tmp/nt/apply.txt 2>&1; then
  fail "apply with an open notice was not refused"
fi
grep -qE '^error +notice +.*pack_bucket_adopted$' tmp/nt/apply.txt \
  || fail "the refusal is the finding, at its line and by its subject:\n$(cat tmp/nt/apply.txt)"
grep -q 'every command that writes to the organisation refuses until then' tmp/nt/apply.txt \
  || fail "the refusal must say what holds the run back:\n$(cat tmp/nt/apply.txt)"
# and no tier may silence it away
if "$satz" --config . --silence notice transpile "$PWD/tmp/nt/open-notice.satz" --apply --output "$PWD/tmp/nt/hcl" > tmp/nt/silenced.txt 2>&1; then
  fail "--silence of an error-severity notice was not refused"
fi
grep -q 'an error is never silenced' tmp/nt/silenced.txt || fail "silencing an error must be refused by name:\n$(cat tmp/nt/silenced.txt)"

step "silence: a finding is left out of the printed output by its kind and subject, stays in the machine stream, and a rule nothing answers to reads stale"
# The shape this exists for: the CIS extensions all on at once produce eleven findings
# on every compile — ten open notices and the baseline's requirement — all saying what
# has already been read once.
rm -rf tmp/sil && mkdir -p tmp/sil
printf 'yaml_dir = "."\nhcl_dir = "hcl"\ninclude_dirs = [".", "../../../.."]\nschema_dir = "../../../schemas"\npresets_dir = "../../../../presets"\ntf_tool = "tofu"\ngoogle_providers = ["google", "google-beta"]\nprovider_version = "7.14.1"\n' > tmp/sil/config.toml
cp "$root/tests/corpus/cis-packs/main.satz" tmp/sil/cis.satz
"$satz" --config tmp/sil transpile cis.satz --check > tmp/sil/before.txt 2>&1 \
  || fail "the corpus estate must compile:\n$(cat tmp/sil/before.txt)"
grep -q 'notices open — what a pack asks to be run once it is on (10)' tmp/sil/before.txt || fail "the ten notices are what this silences:\n$(cat tmp/sil/before.txt)"
grep -q 'packs on while a pack they need is off (1)' tmp/sil/before.txt || fail "the eleventh finding is missing"
grep -q '^11 warnings, 2 infos$' tmp/sil/before.txt || fail "the run closes with its count by severity:\n$(cat tmp/sil/before.txt)"
# the two infos: this estate binds neither param the prerequisite check is judged on,
# and a check that cannot run says so rather than nothing
grep -q 'the APIs this estate.s resources need were not checked' tmp/sil/before.txt \
  || fail "the API half of the prerequisite check said nothing at all:\n$(cat tmp/sil/before.txt)"
grep -q 'the roles this estate.s resource types need were not checked' tmp/sil/before.txt \
  || fail "the role half of the prerequisite check said nothing at all:\n$(cat tmp/sil/before.txt)"
# two rows, written by satz into the estate's own config.toml
"$satz" --config tmp/sil silence add notice --reason "satz adopt --execute --import has run" > tmp/sil/add.txt 2>&1 \
  || fail "silence add failed:\n$(cat tmp/sil/add.txt)"
"$satz" --config tmp/sil silence add "pack-requirement:presets/cis/CIS-GCP-Foundation-4.0.satz" \
  --reason "this estate compiles the baseline without the map on purpose" >> tmp/sil/add.txt 2>&1 \
  || fail "silence add with a subject failed:\n$(cat tmp/sil/add.txt)"
grep -q 'kind = "notice"' tmp/sil/config.toml || fail "the row was not written into config.toml:\n$(cat tmp/sil/config.toml)"
"$satz" --config tmp/sil transpile cis.satz --check > tmp/sil/after.txt 2>&1 \
  || fail "the estate must still compile:\n$(cat tmp/sil/after.txt)"
grep -q 'notices open' tmp/sil/after.txt && fail "a silenced finding was printed:\n$(cat tmp/sil/after.txt)"
grep -q '11 silenced (11 estate)' tmp/sil/after.txt \
  || fail "every run says how many findings it left out, and from which tier:\n$(cat tmp/sil/after.txt)"
# …and an agent is handed all eleven, each marked
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":"tmp/sil","estate":"cis.satz"}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_transpile_check","arguments":{}}}' \
  > tmp/sil/in.jsonl
python3 tmp/mcp-drive.py "$satz" mcp --root . < tmp/sil/in.jsonl > tmp/sil/mcp.jsonl 2>/dev/null || true
python3 - <<'PYEOF' || fail "the machine stream lost what the terminal left out"
import json
msgs = {d["id"]: d for d in (json.loads(l) for l in open("tmp/sil/mcp.jsonl") if l.strip()) if "id" in d}
f = msgs[3]["result"]["structuredContent"]["findings"]
silenced = [x for x in f if "silenced" in x]
assert len(silenced) == 11, f"the JSON kept {len(silenced)} of eleven silenced findings"
assert {x["silenced"]["tier"] for x in silenced} == {"estate"}, silenced
assert all(x["subject"] for x in silenced), "every one of these findings names what it is about"
# one path, whoever asks: the CLI and MCP spell the estate the same way
assert {x["file"] for x in silenced} == {"cis.satz"}, {x["file"] for x in silenced}
# what was not silenced is what nothing silences: the two prerequisite notes this
# estate earns by binding neither param the check is judged on
rest = [x for x in f if "silenced" not in x]
assert all(x["kind"] == "prerequisites" for x in rest), rest
PYEOF
# a row the estate outgrew says so
"$satz" --config tmp/sil silence add "notice:no_such_param" --reason "a pack that is gone" > /dev/null 2>&1 \
  || fail "silence add of a subject nothing matches must still be written"
"$satz" --config tmp/sil silence list cis.satz > tmp/sil/list.txt 2>&1 || fail "silence list failed:\n$(cat tmp/sil/list.txt)"
grep -q 'silences 10 finding(s)' tmp/sil/list.txt || fail "list must say what each row silences:\n$(cat tmp/sil/list.txt)"
grep -q 'STALE' tmp/sil/list.txt || fail "a row nothing answers to must read stale:\n$(cat tmp/sil/list.txt)"
"$satz" --config tmp/sil silence remove "notice:no_such_param" > /dev/null 2>&1 || fail "silence remove failed"
"$satz" --config tmp/sil silence list cis.satz | grep -q 'STALE' && fail "the stale row was not removed"
# an error is never silenced, and the run that asked for it is refused by name
if "$satz" --config tmp/sil --validation error --silence pack-requirement transpile cis.satz --check > tmp/sil/err.txt 2>&1; then
  fail "a --silence naming an error was accepted"
fi
grep -q 'an error is never silenced' tmp/sil/err.txt || fail "the refusal must say why:\n$(cat tmp/sil/err.txt)"
# and the run tier is one run's: a server serving many estates never carries one
if "$satz" --silence action mcp --root . > tmp/sil/srv.txt 2>&1; then
  fail "satz mcp accepted --silence"
fi
grep -q 'one run' tmp/sil/srv.txt || fail "the refusal must say why:\n$(cat tmp/sil/srv.txt)"

step "satz mcp: adopt refuses without credentials; get-presets stays inside the root and fills a library"
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"smoke.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_adopt","arguments":{"only":["google_folder"]}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"satz_get_presets","arguments":{}}}'
} > tmp/mcp-adopt-in.jsonl
GOOGLE_APPLICATION_CREDENTIALS=/nonexistent python3 tmp/mcp-drive.py "$satz" mcp --root . --allow read,write < tmp/mcp-adopt-in.jsonl > tmp/mcp-adopt.jsonl 2>/dev/null || true
rm -rf tmp/gp && mkdir -p tmp/gp/yaml && cp -R "$root/presets" tmp/gp/pristine
cat > tmp/gp/config.toml <<'EOF'
yaml_dir = "yaml"
hcl_dir = "hcl"
include_dirs = [".", "yaml"]
presets_dir = "presets"
tf_tool = "tofu"
EOF
printf '%s\n' 'estate gp' '' 'params {' '  customer_organization_id = "123456789012"' '}' '' 'terraform {' '  backend {' '    local { path = "terraform.tfstate" }' '  }' '}' > tmp/gp/yaml/gp.satz
{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"satz_open","arguments":{"config":".","estate":"gp.satz"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"satz_get_presets","arguments":{"pristine_dir":"pristine"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"satz_get_presets","arguments":{"pristine_dir":"pristine"}}}'
  printf '%s\n' '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"satz_merge_presets","arguments":{"pristine_dir":"pristine","report_only":true}}}'
} > tmp/mcp-gp-in.jsonl
(cd tmp/gp && python3 ../mcp-drive.py "$satz" mcp --root . --allow read,write < ../mcp-gp-in.jsonl > ../mcp-gp.jsonl 2>/dev/null) || true
python3 - <<'PYEOF' || fail "satz_adopt / satz_get_presets did not behave"
import json
def read(path):
    out = {}
    for l in open(path):
        if l.strip():
            d = json.loads(l)          # every line must parse: stdout is the protocol
            if "id" in d:
                out[d["id"]] = d
    return out
a = read("tmp/mcp-adopt.jsonl")
adopt = a[3]["result"]
assert adopt["isError"] is True, adopt
assert any(w in adopt["content"][0]["text"].lower() for w in ("credential", "token", "adc", "auth")), adopt
gp_outside = a[4]["result"]
assert gp_outside["isError"] is True and "outside the server's root" in gp_outside["content"][0]["text"], gp_outside
g = read("tmp/mcp-gp.jsonl")
first = g[3]["result"]["structuredContent"]
# a pack that binds a script is an action that cannot run without it: the install
# carries the .sh, executable
import os
script = "tmp/gp/presets/scc/scc-enable-all.sh"
assert os.path.exists(script), f"get-presets did not install the pack's script: {sorted(os.listdir('tmp/gp/presets/scc'))}"
assert os.access(script, os.X_OK), "the installed script is not executable — the action would refuse to run it"
assert first["installed"] and not first["refused"], first
second = g[4]["result"]["structuredContent"]
assert not second["installed"] and not second["refreshed"] and second["current"] == len(first["installed"]), second
# the same library through merge-presets: everything current, nothing to do, and the
# report is the walk as events — not a string a client would have to parse
merged = g[5]["result"]["structuredContent"]
assert merged["report_only"] is True, merged
assert merged["counts"]["current"] == second["current"], merged["counts"]
assert merged["attention"] is False, merged
# every event is structured — packs, and the notes the run has to add (an estate
# with no IaC service account cannot be prerequisite-checked, and says so)
assert all(e["kind"] in ("pack", "note") for e in merged["events"]), [e["kind"] for e in merged["events"]][:5]
PYEOF
[ -s tmp/gp/presets/cis/CIS-GCP-Foundation-4.0.satz ] || fail "satz_get_presets did not install the library"

step "fleet-v1: clean, body delta, moved address set, and an estate nobody checked"
# V1 is the only check that catches an estate which quietly stopped compiling or
# whose emitted resource SET moved. Its own four outcomes are worth a gate,
# because the one that matters most — a checkout that was never looked at — is
# the one a naive script reports as success.
mkdir -p tmp/fleet/estate/yaml tmp/fleet/estate/presets
cp -R "$root/tests/schemas" tmp/fleet/estate/schemas
cat > tmp/fleet/estate/config.toml <<'EOF'
yaml_dir = "yaml"
hcl_dir = "hcl"
include_dirs = [".", "yaml"]
schema_dir = "schemas"
presets_dir = "presets"
tf_tool = "tofu"
google_providers = ["google", "google-beta"]
provider_version = "7.14.1"
EOF
cat > tmp/fleet/estate/yaml/fixture.satz <<'EOF'
estate fleet_fixture

params {
  customer_organization_id = "123456789012"
  customer_shortname = "acme"
  infra_project_name = "acme-infra-001"
  deployment_engine = "tofu"
  deployment_mode = "local"
}

terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}

google_folder {
  first { display_name = "First" }
  second { display_name = "Second" }
}
EOF
# The baseline: what the estate emitted "before".
"$satz" --config tmp/fleet/estate transpile fixture.satz >/dev/null 2>&1 \
  || fail "the fleet-v1 fixture does not transpile"
cp -R tmp/fleet/estate/hcl tmp/fleet/baseline

run_v1() {  # run_v1 <expected-exit> <label> [args...]
  local want="$1" label="$2"; shift 2
  local rc=0
  SATZ="$satz" bash "$root/scripts/fleet-v1.sh" "$@" > tmp/fleet/out.txt 2>&1 || rc=$?
  [ "$rc" = "$want" ] || fail "fleet-v1 $label: expected exit $want, got $rc:\n$(cat tmp/fleet/out.txt)"
}

# 1 · nothing changed
run_v1 0 "clean" tmp/fleet/estate
grep -q 'no body delta' tmp/fleet/out.txt || fail "fleet-v1 did not report a clean estate:\n$(cat tmp/fleet/out.txt)"

# 2 · a block body differs — the estate is the same shape, an attribute is not
sed -i.bak 's/display_name = "First"/display_name = "Moved"/' tmp/fleet/estate/hcl/main.tf
run_v1 2 "body delta" tmp/fleet/estate
grep -q 'address set identical' tmp/fleet/out.txt || fail "a body delta was not reported as one:\n$(cat tmp/fleet/out.txt)"
grep -q 'google_folder.first' tmp/fleet/out.txt || fail "the changed block was not named:\n$(cat tmp/fleet/out.txt)"
#     ... and --verbose shows WHAT differs, under the block it names.
run_v1 2 "body delta, verbose" -v tmp/fleet/estate
grep -q '^ *+ *display_name = "First"' tmp/fleet/out.txt \
  || fail "--verbose did not print the block diff:\n$(cat tmp/fleet/out.txt)"

# 3 · the address set moved — a resource the baseline never had. BLOCKER.
rm -rf tmp/fleet/estate/hcl
cp -R tmp/fleet/baseline tmp/fleet/estate/hcl
python3 - <<'PYEOF'
import pathlib
p = pathlib.Path("tmp/fleet/estate/hcl/main.tf")
text = p.read_text()
start = text.index('resource "google_folder" "second"')
end = text.index("\n}\n", start) + 3
p.write_text(text[:start] + text[end:])
PYEOF
run_v1 1 "moved address set" tmp/fleet/estate
grep -q 'address set moved' tmp/fleet/out.txt || fail "a new address was not reported as a blocker:\n$(cat tmp/fleet/out.txt)"
grep -q '+ google_folder.second' tmp/fleet/out.txt || fail "the added address was not named:\n$(cat tmp/fleet/out.txt)"

# 4 · an estate nobody checked is not a pass. The roster is read in the markdown
#     table form a fleet note already uses, so no second list has to be kept.
rm -rf tmp/fleet/estate/hcl
cp -R tmp/fleet/baseline tmp/fleet/estate/hcl
cat > tmp/fleet/roster.md <<EOF
| code | repo | path |
|---|---|---|
| E01 | fixture | \`$PWD/tmp/fleet/estate\` |
| E02 | gone | \`$PWD/tmp/fleet/not-here\` |
EOF
run_v1 0 "unavailable is not a failure by default" --roster tmp/fleet/roster.md
grep -q 'UNAVAILABLE' tmp/fleet/out.txt || fail "a missing checkout was not reported:\n$(cat tmp/fleet/out.txt)"
run_v1 1 "--require-all" --roster tmp/fleet/roster.md --require-all
grep -q 'never checked' tmp/fleet/out.txt || fail "--require-all did not fail on an unchecked estate:\n$(cat tmp/fleet/out.txt)"

step "documentation site renders (what pages.yml publishes)"
uv run "$root/scripts/build-site.py" tmp/site >/dev/null || fail "scripts/build-site.py failed"
for f in index.html docs/language.html presets/index.html; do [ -s "tmp/site/$f" ] || fail "site: $f missing"; done
# The menu always carries href="docs/language.html", so the rewrite is judged by a link
# only the README's text has: one into a section of the language reference.
grep -q 'href="docs/language.html#' tmp/site/index.html || fail "site: README's links into the language reference were not rewritten to HTML"
if grep -rqE 'href="[^":#]+\.md(#[^"]*)?"' tmp/site; then
  fail "site: a relative .md link survived the build, and it 404s on the site:\n$(grep -rhoE 'href="[^":#]+\.md(#[^"]*)?"' tmp/site | sort -u | head -5)"
fi
grep -q 'blob/main/docs/adr/0006-' tmp/site/docs/interview.html \
  || fail "site: a link to an ADR, which the site does not publish, does not go to GitHub"
# `satz <cmd> --html-help` opens the front page at #cmd-<cmd>, else at #cli-usage: the
# binary's list is the contract, read from the source rather than copied here.
documented="$(sed -n '/const DOCUMENTED: &\[&str\] = &\[/,/\];/p' "$root/src/main.rs" | grep -o '"[a-z-]*"' | tr -d '"')"
[ -n "$documented" ] || fail "site: could not read the --html-help command list from src/main.rs"
for cmd in $documented; do
  grep -q "id=\"cmd-$cmd\"" tmp/site/index.html || fail "site: --html-help opens #cmd-$cmd, which the front page does not carry"
done
grep -q 'id="cli-usage"' tmp/site/index.html || fail "site: --html-help falls back to #cli-usage, which the front page does not carry"

# CI sets SMOKE_SKIP_CARGO_TEST=1: its `checks` job runs the same tests on the same commit.
if [ -n "${SMOKE_SKIP_CARGO_TEST:-}" ]; then
  step "corpus + unit tests skipped (SMOKE_SKIP_CARGO_TEST)"
else
  step "corpus + unit tests"
  (cd "$root" && cargo test --workspace --locked --quiet 2>&1 | tail -3)
fi

rm -rf hcl tmp yaml/imported-*.satz yaml/identity-*.satz evidence
printf '\nsmoke: every command ran.\n'
