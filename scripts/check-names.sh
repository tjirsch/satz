#!/usr/bin/env bash
# check-names.sh — the privacy gate. Neutral: it knows no customer, no
# company, no person. It rejects anything SHAPED like private data that is
# not one of the predefined example values (docs/examples.md), and
# any commit made under an identity other than the maintainer's or a GitHub
# noreply address.
#
# The content rules have a second implementation in satz (src/privacy_shapes.rs,
# what `satz review-pack` judges a pack with). Both read their allow-lists from
# scripts/check-names-allow.txt, and a cargo test runs both over the corpus in
# tests/privacy-shapes/ and fails on any token one flags and the other does not.
# A content rule reports one `    <file>:<line>: <token>` row per token under its
# `✗ <rule>` title; that test reads the report by those two forms.
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
# The content allow-lists — example values, vendor hosts, vendor-default GUIDs —
# live in ONE file both readers take them from: this script, and satz itself
# (src/privacy_shapes.rs compiles it in for `satz review-pack`). One entry per
# line, `<list> <ERE>`; a list's entries are joined into one alternation. A list
# with no entry, or a missing file, stops the gate: checking against nothing must
# never read as OK.
ALLOW_FILE=scripts/check-names-allow.txt
allow() { # $1 list name → its entries joined with `|`
  local v
  [[ -f "$ALLOW_FILE" ]] || { echo "check-names: no such file: $ALLOW_FILE" >&2; return 1; }
  v=$(grep -E "^$1 " "$ALLOW_FILE" | cut -d' ' -f2- | paste -sd '|' -)
  [[ -n "$v" ]] || { echo "check-names: $ALLOW_FILE has no '$1' entry" >&2; return 1; }
  printf '%s' "$v"
}
ALLOW_DIR=$(allow dir) || exit 1
ALLOW_NUM=$(allow num) || exit 1
ALLOW_BILL=$(allow bill) || exit 1
# domains: IANA-reserved names, plus the vendors and standards bodies this project references
ALLOW_DOMAIN=$(allow domain) || exit 1
ALLOW_MAILDOM="$ALLOW_DOMAIN"
# GUIDs. Two kinds are legitimate: the four example tenants, and VENDOR DEFAULTS —
# identifiers Microsoft or Google publish and every customer shares. Each vendor
# default is listed in docs/examples.md with what it is; a GUID that is not
# there is assumed to be a customer's Entra tenant or directory object.
ALLOW_GUID=$(allow guid) || exit 1
# the same values without dashes: an Entra tenant id in that form is the workload
# identity POOL id, and identifies the customer just as well
ALLOW_GUID32=$(allow guid32) || exit 1
# project ids: the example customers' projects, the placeholders the docs use, and
# anything still carrying a param or a placeholder ({...}, <...>, UPPER_CASE)
ALLOW_PROJECT=$(allow project) || exit 1


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
# Files no person writes: the lockfile, the provider schema cut, images, the font
# faces satz typesets PDFs with and the two licence texts beside them (copied out of
# the typst-assets crate), and the licence texts cargo-about copies out of the crates
# satz compiles — upstream authors' e-mail addresses and domains, regenerated byte for
# byte by scripts/update-third-party-licenses.sh, which CI re-runs on every pull
# request. A customer's name cannot enter a file nobody edits.
files=$(printf '%s\n' $files | grep -v -E '^(Cargo\.lock|THIRD-PARTY-LICENSES\.md|tests/schemas/.*|assets/fonts/.*|.*\.(png|jpg|gif|svg))$' || true)

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
  "$(g '\bC0[0-9a-z]{7}\b' | tokens '\bC0[0-9a-z]{7}\b' "\b($ALLOW_DIR)\b")"
report "11–13 digit number (org/project/folder id) that is not an example value" \
  "$(g '\b[0-9]{11,13}\b' | sed -E 's/[0-9a-fA-F]{20,}//g; s/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}//g' | tokens '\b[0-9]{11,13}\b' "\b($ALLOW_NUM)\b")"
report "billing account id that is not an example value" \
  "$(g '\b[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}\b' | tokens '\b[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}\b' "($ALLOW_BILL)")"
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
# These two patterns carry double quotes inside single quotes. Written inside
# "$( … )", bash 3.2 — /bin/bash on macOS — misparses that nesting and both
# rules matched nothing; a plain assignment has no enclosing double quotes.
hits=$(g '(projects/[a-z][a-z0-9-]{3,28}[a-z0-9]|project(_id)?[[:space:]]*=[[:space:]]*"[^"]*"|--project[= ][a-z][a-z0-9-]{3,28}[a-z0-9])' \
     | tokens 'projects/[a-z][a-z0-9-]{3,28}[a-z0-9]' "^projects/($ALLOW_PROJECT)$")
report "project id that is not an example value (projects/…, project = …, --project)" "$hits"
hits=$(g 'project(_id)?[[:space:]]*=[[:space:]]*"[a-z][a-z0-9-]{4,28}[a-z0-9]"' \
     | tokens 'project(_id)?[[:space:]]*=[[:space:]]*"[a-z][a-z0-9-]{4,28}[a-z0-9]"' "=[[:space:]]*\"($ALLOW_PROJECT)\"$")
report "project id in an assignment that is not an example value" "$hits"
report "customer repository URL or checkout path" \
  "$(g 'source\.developers\.google\.com|~/projects/(organizations|[a-z]+/[a-z]+-C0)' \
     | tokens 'source\.developers\.google\.com|~/projects/(organizations|[a-z]+/[a-z]+-C0)' '^$')"

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
  echo; echo "check-names: FAILED — private data must not enter this repository; use docs/examples.md values and the maintainer identity"
  exit 1
fi
n=$( [[ -n "$files" ]] && printf '%s\n' $files | wc -l | tr -d ' ' || echo 0 )
echo "check-names: OK (${n} files${range:+, commits $range})"
