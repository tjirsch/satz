#!/usr/bin/env bash
# check-names.sh — the privacy gate. Neutral: it knows no customer, no
# company, no person. It rejects anything SHAPED like private data that is
# not one of the predefined example values (docs/example-customers.md), and
# any commit made under an identity other than the maintainer's or a GitHub
# noreply address.
#
#   scripts/check-names.sh                       # whole tree (CI)
#   scripts/check-names.sh --staged              # staged files + the identity about to commit (pre-commit hook)
#   scripts/check-names.sh --commits A..B        # identities and messages of a commit range (CI)
#   scripts/check-names.sh FILE...               # specific files
#
# Optional LOCAL denylist (never committed): $NAMES_DENYLIST, or
# ~/Documents/thomas01/satz-core-history-rewrite/denylist.txt if present — one
# extended regex per line. CI has none and stays structural.
#
# bash 3.2 compatible (macOS default).
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

# ---- allowlists ---------------------------------------------------------------
# identities that may author or commit: the maintainer, and GitHub's private noreply addresses
ALLOW_IDENT='Thomas\.Jirsch@gmail\.com|[0-9]+\+[A-Za-z0-9-]+@users\.noreply\.github\.com|noreply@github\.com'
# example customers (docs/example-customers.md) + documented legacy placeholders
ALLOW_DIR='C0example|C0bolt002|C0cedar03|C0delta04|C01234567|C0abcd123'
ALLOW_NUM='123456789012|222222222222|333333333333|444444444444|100000000001|200000000002|300000000003|400000000004|123456789|222222222|333333333|444444444'
ALLOW_BILL='012345-6789AB-CDEF01|0B0B0B-0B0B0B-0B0B02|0C0C0C-0C0C0C-0C0C03|0D0D0D-0D0D0D-0D0D04|123456-123456-123456|A12345-B67890-C12345'
# domains: IANA-reserved names, plus the vendors and standards bodies this project genuinely references
ALLOW_DOMAIN='example\.(com|org|net)|[a-z0-9.-]+\.(example|test|invalid|localhost)|[a-z0-9.-]*googleapis\.com|[a-z0-9.-]*gserviceaccount\.com|[a-z0-9.-]*google\.com|[a-z0-9.-]*googleusercontent\.com|[a-z0-9.-]*github\.com|[a-z0-9.-]*githubusercontent\.com|[a-z0-9.-]*github\.io|[a-z0-9.-]*opentofu\.org|[a-z0-9.-]*terraform\.io|[a-z0-9.-]*hashicorp\.com|[a-z0-9.-]*cisecurity\.org|[a-z0-9.-]*w3\.org|[a-z0-9.-]*contributor-covenant\.org|crates\.io|docs\.rs|[a-z0-9.-]*rust-lang\.org|[a-z0-9.-]*rustup\.rs|[a-z0-9.-]*anthropic\.com|[a-z0-9.-]*claude\.ai|[a-z0-9.-]*astral\.sh|[a-z0-9.-]*prowler\.com|[a-z0-9.-]*axo\.dev|[a-z0-9.-]*axodotdev\.github\.io'
ALLOW_MAILDOM="$ALLOW_DOMAIN"

# ---- mode ----------------------------------------------------------------------
mode="${1:-}"; range=""
case "$mode" in
  --staged)  files=$(git diff --cached --name-only --diff-filter=ACMR) ;;
  --commits) range="${2:?usage: --commits A..B}"; files="" ;;
  "")        files=$(git ls-files) ;;
  *)         files="$*" ;;
esac
files=$(printf '%s\n' $files | grep -v -E '^(Cargo\.lock|tests/schemas/.*|.*\.(png|jpg|gif|svg))$' || true)

fail=0
report() { # $1 rule, $2 matching lines — no subshell, the flag must survive
  [[ -n "$2" ]] || return 0
  fail=1; echo "✗ $1"; printf '%s\n' "$2" | cut -c1-160 | sed 's/^/    /'
}
g() { # grep -n ERE over the file list; staged mode reads the index
  [[ -n "$files" ]] || return 0
  if [[ "$mode" == "--staged" ]]; then
    for f in $files; do git show ":$f" 2>/dev/null | grep -n -E "$1" | sed "s|^|$f:|"; done
  else
    grep -n -E "$1" $files 2>/dev/null
  fi
  return 0
}

# ---- 0. identity: who is committing -------------------------------------------
if [[ "$mode" == "--staged" ]]; then
  a=$(git var GIT_AUTHOR_IDENT | sed 's/.*<\(.*\)>.*/\1/'); c=$(git var GIT_COMMITTER_IDENT | sed 's/.*<\(.*\)>.*/\1/')
  bad=""; for e in "$a" "$c"; do echo "$e" | grep -q -E "^($ALLOW_IDENT)$" || bad="$bad$e"$'\n'; done
  report "commit identity is not the maintainer or a GitHub noreply address (set: git config user.email …)" "$bad"
elif [[ -n "$range" ]]; then
  bad=$(git log --format='%h %ae %ce' "$range" | awk -v re="^($ALLOW_IDENT)$" '{ if ($2 !~ re || $3 !~ re) print }')
  report "commit identity in $range is not the maintainer or a GitHub noreply address" "$bad"
  # commit messages in the range go through the same content rules as files
  msgs=$(git log --format='%h %B' "$range")
  report "directory id (C0…) in a commit message"      "$(printf '%s\n' "$msgs" | grep -E '\bC0[0-9a-z]{7}\b' | grep -v -E "\b($ALLOW_DIR)\b")"
  report "11–13 digit number in a commit message"       "$(printf '%s\n' "$msgs" | grep -E '\b[0-9]{11,13}\b' | grep -v -E "\b($ALLOW_NUM)\b")"
  report "e-mail outside allowed domains in a message"  "$(printf '%s\n' "$msgs" | grep -o -E '[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' | grep -i -v -E "@($ALLOW_MAILDOM)\b")"
fi

# ---- 1–6. content rules --------------------------------------------------------
report "directory id (C0…) that is not an example value" \
  "$(g '\bC0[0-9a-z]{7}\b' | grep -v -E "\b($ALLOW_DIR)\b")"
report "11–13 digit number (org/project/folder id) that is not an example value" \
  "$(g '\b[0-9]{11,13}\b' | grep -v -E "\b($ALLOW_NUM)\b" | grep -v -E '[0-9a-f]{20,}')"
report "billing account id that is not an example value" \
  "$(g '\b[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}\b' | grep -v -E "($ALLOW_BILL)")"
report "e-mail address outside reserved/vendor domains (placeholders like <customer-domain> are fine)" \
  "$(g '[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' | grep -v -E '@<' | grep -i -v -E "@($ALLOW_MAILDOM)\b")"
report "domain that is neither IANA-reserved nor a known vendor host (a real company's domain?)" \
  "$(g '\b[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|org|net|io|dev|de|eu|ch|at|uk|us|fr|it|nl|cloud|app|ai|co)\b' \
     | grep -o -E '^[^:]+:[0-9]+:.*' | grep -i -v -E "\b($ALLOW_DOMAIN)\b" )"
report "customer repository URL or checkout path" \
  "$(g 'source\.developers\.google\.com|~/projects/(organizations|[a-z]+/[a-z]+-C0)')"

# ---- 6b. local / private files must never be tracked or staged ------------------
if [[ -n "$files" ]]; then
  report "local file that must not be committed (CLAUDE.local.md, *.local.md, .claude/, attestations.yaml, evidence/)" \
    "$(printf '%s\n' $files | grep -E '(^|/)(CLAUDE\.local\.md|[^/]+\.local\.md|\.claude/.*|attestations\.yaml|evidence/.*)$' || true)"
fi

# ---- 7. optional local denylist (never committed) ------------------------------
DENY="${NAMES_DENYLIST:-$HOME/Documents/thomas01/satz-core-history-rewrite/denylist.txt}"
if [[ -f "$DENY" && -n "$files" ]]; then
  pat=$(grep -v -E '^[[:space:]]*(#|$)' "$DENY" | paste -sd '|' -)
  [[ -z "$pat" ]] || report "local denylist match" "$(g "$pat" | grep -i -E "$pat")"
fi

if (( fail )); then
  echo; echo "check-names: FAILED — private data must not enter this repository; use docs/example-customers.md values and the maintainer identity"
  exit 1
fi
n=$( [[ -n "$files" ]] && printf '%s\n' $files | wc -l | tr -d ' ' || echo 0 )
echo "check-names: OK (${n} files${range:+, commits $range})"
