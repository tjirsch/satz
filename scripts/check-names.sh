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
#   scripts/check-names.sh --message FILE        # one commit message (commit-msg hook)
#   scripts/check-names.sh FILE...               # specific files
#
# What it CANNOT see: a project's or folder's DISPLAY NAME, or a company name in
# prose. Those have no shape — "Log Admins" and a real customer's project name are
# the same kind of string — so they are the local denylist's job, below.
#
# Optional LOCAL denylist (never committed): $NAMES_DENYLIST, or
# ~/Documents/thomas01/satz-core-history-rewrite/denylist.txt if present — one
# extended regex per line. CI has none and stays structural.
#
# bash 3.2 compatible (macOS default).
set -uo pipefail
orig_pwd="$PWD"
cd "$(git rev-parse --show-toplevel)"

# ---- allowlists ---------------------------------------------------------------
# identities that may author or commit: the maintainer, and GitHub's private noreply addresses
ALLOW_IDENT='Thomas\.Jirsch@gmail\.com|[0-9]+\+[A-Za-z0-9-]+@users\.noreply\.github\.com|noreply@github\.com'
# example customers (docs/example-customers.md) + documented legacy placeholders
ALLOW_DIR='C0example|C0bolt002|C0cedar03|C0delta04|C01234567|C0abcd123'
ALLOW_NUM='123456789012|222222222222|333333333333|444444444444|100000000001|200000000002|300000000003|400000000004|123456789|222222222|333333333|444444444'
ALLOW_BILL='012345-6789AB-CDEF01|0B0B0B-0B0B0B-0B0B02|0C0C0C-0C0C0C-0C0C03|0D0D0D-0D0D0D-0D0D04|123456-123456-123456|A12345-B67890-C12345'
# domains: IANA-reserved names, plus the vendors and standards bodies this project genuinely references
ALLOW_DOMAIN='example\.(com|org|net)|[a-z0-9.-]+\.(example|test|invalid|localhost)|[a-z0-9.-]*googleapis\.com|[a-z0-9.-]*gserviceaccount\.com|[a-z0-9.-]*google\.com|[a-z0-9.-]*googleusercontent\.com|[a-z0-9.-]*github\.com|[a-z0-9.-]*githubusercontent\.com|[a-z0-9.-]*github\.io|[a-z0-9.-]*opentofu\.org|[a-z0-9.-]*terraform\.io|[a-z0-9.-]*hashicorp\.com|[a-z0-9.-]*cisecurity\.org|[a-z0-9.-]*w3\.org|[a-z0-9.-]*contributor-covenant\.org|crates\.io|docs\.rs|[a-z0-9.-]*rust-lang\.org|[a-z0-9.-]*rustup\.rs|[a-z0-9.-]*anthropic\.com|[a-z0-9.-]*claude\.ai|[a-z0-9.-]*astral\.sh|[a-z0-9.-]*prowler\.com|[a-z0-9.-]*axo\.dev|[a-z0-9.-]*axodotdev\.github\.io|[a-z0-9.-]*windows\.net|[a-z0-9.-]*microsoft\.com|[a-z0-9.-]*microsoftonline\.com'
ALLOW_MAILDOM="$ALLOW_DOMAIN"
# GUIDs. Two kinds are legitimate: the four example tenants, and VENDOR DEFAULTS —
# identifiers Microsoft or Google publish and every customer shares. Each vendor
# default is listed in docs/example-customers.md with what it is; a GUID that is not
# there is assumed to be a customer's Entra tenant or directory object.
ALLOW_GUID='11111111-1111-1111-1111-111111111111|22222222-2222-2222-2222-222222222222|33333333-3333-3333-3333-333333333333|44444444-4444-4444-4444-444444444444|00000000-0000-0000-0000-000000000000|33e01921-4d64-4f8c-a055-5bdaffd5e33d|d17a7d74-7e73-4e7d-bd41-8d9525e86cab|6e81e733-9e7f-474a-85f0-385c097f7f52'
# the same values without dashes: an Entra tenant id in that form is the workload
# identity POOL id, and identifies the customer just as well
ALLOW_GUID32='11111111111111111111111111111111|22222222222222222222222222222222|33333333333333333333333333333333|44444444444444444444444444444444|00000000000000000000000000000000|33e019214d644f8ca0555bdaffd5e33d|d17a7d747e734e7dbd418d9525e86cab|6e81e7339e7f474a85f0385c097f7f52'
# project ids: the example customers' projects, the placeholders the docs use, and
# anything still carrying a param or a placeholder ({...}, <...>, UPPER_CASE)
ALLOW_PROJECT='(acme|bolt|cedar|delta|corp)-[a-z0-9-]+|my-project|my-prj|p-one|example-[a-z0-9-]+|[A-Za-z_-]*PROJECT[_-]?ID[A-Za-z_-]*|[^"]*[{<][^"]*'


# ---- mode ----------------------------------------------------------------------
mode="${1:-}"; range=""; msgfile=""
case "$mode" in
  --staged)  files=$(git diff --cached --name-only --diff-filter=ACMR) ;;
  --commits) range="${2:?usage: --commits A..B}"; files="" ;;
  --message) msgfile="${2:?usage: --message FILE}"; files="" ;;
  "")        files=$(git ls-files) ;;
  *)
    # explicit files: resolved against the caller's directory (we cd to the
    # repository root above), and a file that does not exist is an error —
    # checking nothing must never read as OK
    files=""
    for f in "$@"; do
      case "$f" in /*) abs="$f" ;; *) abs="$orig_pwd/$f" ;; esac
      [[ -f "$abs" ]] || { echo "check-names: no such file: $f"; exit 1; }
      abs="$(cd "$(dirname "$abs")" && pwd)/$(basename "$abs")"
      files="$files${abs#"$PWD"/}"$'\n'  # inside the repository: relative to its root
    done ;;
esac
# an unusable range must FAIL, not pass with nothing checked
if [[ -n "$range" ]]; then
  for r in "${range%%..*}" "${range##*..}"; do
    git rev-parse --verify --quiet "$r^{commit}" >/dev/null || { echo "check-names: unusable commit range '$range' ($r does not resolve)"; exit 1; }
  done
fi
files=$(printf '%s\n' $files | grep -v -E '^(Cargo\.lock|tests/schemas/.*|.*\.(png|jpg|gif|svg))$' || true)

fail=0
report() { # $1 rule, $2 matching lines — no subshell, the flag must survive
  [[ -n "$2" ]] || return 0
  fail=1; echo "✗ $1"; printf '%s\n' "$2" | cut -c1-160 | sed 's/^/    /'
}
g() { # grep -Hn ERE over the file list; staged mode reads the index
  [[ -n "$files" ]] || return 0
  if [[ "$mode" == "--staged" ]]; then
    for f in $files; do git show ":$f" 2>/dev/null | grep -n -E "$1" | sed "s|^|$f:|"; done
  else
    grep -H -n -E "$1" $files 2>/dev/null
  fi
  return 0
}
# tokens PATTERN ALLOW-ERE: from `file:line:content` lines on stdin, print
# `file:line: token` for every token matching PATTERN that does NOT match the
# allowlist. Per TOKEN — one allowed address on a line never shields another.
tokens() {
  local pat="$1" allow="$2" l pre
  while IFS= read -r l; do
    pre="${l%%:*}:$(printf '%s' "$l" | cut -d: -f2)"
    printf '%s' "$l" | cut -d: -f3- | grep -o -E "$pat" | grep -i -v -E "$allow" | sed "s|^|$pre: |"
  done
  return 0
}

# ---- 0. identity: who is committing -------------------------------------------
if [[ "$mode" == "--staged" ]]; then
  a=$(git var GIT_AUTHOR_IDENT | sed 's/.*<\(.*\)>.*/\1/'); c=$(git var GIT_COMMITTER_IDENT | sed 's/.*<\(.*\)>.*/\1/')
  bad=""; for e in "$a" "$c"; do echo "$e" | grep -q -E "^($ALLOW_IDENT)$" || bad="$bad$e"$'\n'; done
  report "commit identity is not the maintainer or a GitHub noreply address (set: git config user.email …)" "$bad"
elif [[ -n "$range" ]]; then
  # Not via `awk -v`: it processes backslash escapes, so `\+` in the noreply
  # pattern lost its literal `+` and every GitHub squash-merge author
  # (`<id>+<user>@users.noreply.github.com`) was rejected.
  bad=$(git log --format='%h %ae %ce' "$range" | while read -r h a c; do
    for e in "$a" "$c"; do
      echo "$e" | grep -q -E "^($ALLOW_IDENT)$" || { echo "$h $a $c"; break; }
    done
  done)
  report "commit identity in $range is not the maintainer or a GitHub noreply address" "$bad"
  # commit messages in the range go through the same content rules as files
  msgs=$(git log --format='%h %B' "$range")
fi
[[ -z "$msgfile" ]] || msgs=$(cat "$msgfile")
if [[ -n "$range" || -n "$msgfile" ]]; then
  report "directory id (C0…) in a commit message"      "$(printf '%s\n' "$msgs" | grep -o -E '\bC0[0-9a-z]{7}\b' | grep -v -E "\b($ALLOW_DIR)\b")"
  report "11–13 digit number in a commit message"       "$(printf '%s\n' "$msgs" | sed -E 's/[0-9a-fA-F]{20,}//g; s/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}//g' | grep -o -E '\b[0-9]{11,13}\b' | grep -v -E "\b($ALLOW_NUM)\b")"
  report "e-mail outside allowed domains in a message"  "$(printf '%s\n' "$msgs" | grep -o -E '[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' | grep -i -v -E "@($ALLOW_MAILDOM)\b")"
fi

# ---- 1–6. content rules --------------------------------------------------------
report "directory id (C0…) that is not an example value" \
  "$(g '\bC0[0-9a-z]{7}\b' | grep -v -E "\b($ALLOW_DIR)\b")"
report "11–13 digit number (org/project/folder id) that is not an example value" \
  "$(g '\b[0-9]{11,13}\b' | sed -E 's/[0-9a-fA-F]{20,}//g; s/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}//g' | tokens '\b[0-9]{11,13}\b' "\b($ALLOW_NUM)\b")"
report "billing account id that is not an example value" \
  "$(g '\b[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}\b' | grep -v -E "($ALLOW_BILL)")"
report "e-mail address outside reserved/vendor domains (placeholders like <customer-domain> are fine)" \
  "$(g '[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' | tokens '[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' "@($ALLOW_MAILDOM)\b")"
report "domain that is neither IANA-reserved nor a known vendor host (a real company's domain?)" \
  "$(g '\b[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|org|net|io|dev|de|eu|ch|at|uk|us|fr|it|nl|cloud|app|ai|co)\b' \
     | tokens '\b[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|org|net|io|dev|de|eu|ch|at|uk|us|fr|it|nl|cloud|app|ai|co)\b' "^($ALLOW_DOMAIN)$")"
report "GUID that is neither an example value nor a documented vendor default (an Entra tenant id?)" \
  "$(g '\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b' \
     | tokens '\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b' "^($ALLOW_GUID)$")"
report "32 hex characters — an Entra tenant id without dashes is the workload identity pool id" \
  "$(g '\b[0-9a-fA-F]{32}\b' | tokens '\b[0-9a-fA-F]{32}\b' "^($ALLOW_GUID32)$")"
report "project id that is not an example value (projects/…, project = …, --project)" \
  "$(g '(projects/[a-z][a-z0-9-]{3,28}[a-z0-9]|project(_id)?[[:space:]]*=[[:space:]]*"[^"]*"|--project[= ][a-z][a-z0-9-]{3,28}[a-z0-9])' \
     | tokens 'projects/[a-z][a-z0-9-]{3,28}[a-z0-9]' "^projects/($ALLOW_PROJECT)$")"
report "project id in an assignment that is not an example value" \
  "$(g 'project(_id)?[[:space:]]*=[[:space:]]*"[a-z][a-z0-9-]{4,28}[a-z0-9]"' \
     | tokens 'project(_id)?[[:space:]]*=[[:space:]]*"[a-z][a-z0-9-]{4,28}[a-z0-9]"' "=[[:space:]]*\"($ALLOW_PROJECT)\"$")"
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
