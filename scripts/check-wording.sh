#!/usr/bin/env bash
# check-wording.sh — the glossary's words, and no others. docs/glossary.md says which word
# names what and which alternatives are allowed; scripts/check-wording.txt lists the
# wordings that are not, each with what to say. This reports every line of the tracked
# text that uses one.
#
#   scripts/check-wording.sh            # whole tree (CI)
#   scripts/check-wording.sh --staged   # staged files (pre-commit hook)
#   scripts/check-wording.sh FILE...    # specific files
#
# Read: Markdown, Satz, Rust, shell, Python and TOML sources. Of a Rust source only the
# comments and the string literals of more than one word are read: an identifier or a
# one-word literal is a name in the format, and those keep their words. Not read: the
# decision records before 0093 and the changelogs, which keep the words of their day; the
# glossary, its decision record (0095) and the word list, which have to name the words;
# `presets/docs/`, which satz generates from the presets and their changelog; the
# German-language skill files (`skills/estate-documentation/`,
# `skills/security-review/scripts/build_audit.py`), which follow German usage; and
# third-party trees.
#
# The word list holds two kinds of line. `<extended regex><TAB><what to say>` is a
# wording that is not allowed (case-insensitive). `allow<TAB><extended regex>` exempts a
# line of text that matches it (case-sensitive): for a word with a second, legitimate use
# that a pattern cannot tell apart, named as narrowly as that use.
#
# bash 3.2 compatible (macOS default); needs git, grep and awk.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
list=scripts/check-wording.txt
case "${1:-}" in
  --staged) shift; files=$(git diff --cached --name-only --diff-filter=ACMR) ;;
  "") files=$(git ls-files) ;;
  *) files=$(printf '%s\n' "$@") ;;
esac
files=$(printf '%s\n' "$files" | grep -E '\.(md|satz|rs|sh|py|toml)$' \
  | grep -Ev '^(docs/adr/00[0-8][0-9]-|docs/adr/009[0-2]-|vendor/|target/|.*CHANGELOG\.md$|docs/glossary\.md$|docs/adr/0095-|scripts/check-wording\.(sh|txt)$|presets/docs/|skills/estate-documentation/|skills/security-review/scripts/build_audit\.py$|THIRD-PARTY)' || true)
[ -n "$files" ] || { echo "check-wording: OK (0 files)"; exit 0; }

# The text that is judged, one line per source line as `path:line:text`. A Rust source
# gives its comments and string literals; the code between them is blanked.
view=$(mktemp); allow=$(mktemp); trap 'rm -f "$view" "$allow"' EXIT
printf '%s\n' "$files" | while IFS= read -r f; do
  [ -f "$f" ] || continue
  case "$f" in
    *.rs) awk -v F="$f" '
      # state: 0 code, 1 string, 2 raw string (closed by a quote and `hashes`)
      BEGIN { state = 0 }
      {
        line = $0; out = ""
        while (line != "") {
          if (state == 0) {
            if (match(line, /\/\/|r#*"|"|\047(\\.|[^\\\047])\047/) == 0) break
            tok = substr(line, RSTART, RLENGTH); line = substr(line, RSTART + RLENGTH)
            if (tok == "//") { out = out " " line; line = "" }
            else if (tok == "\"") { state = 1; lit = "" }
            else if (substr(tok, 1, 1) == "r") { state = 2; out = out " | "; hashes = substr(tok, 2, length(tok) - 2) }
          } else if (state == 1) {
            if (match(line, /\\.|"/) == 0) { lit = lit line " "; line = "" }
            else {
              tok = substr(line, RSTART, RLENGTH); lit = lit substr(line, 1, RSTART - 1)
              line = substr(line, RSTART + RLENGTH)
              # a literal of one word is a name — a kind, a field, a key — and not prose
              if (tok == "\"") { state = 0; if (lit ~ /[ \t]/) out = out " | " lit " " } else lit = lit tok
            }
          } else {
            end = index(line, "\"" hashes)
            if (end == 0) { out = out line; line = "" }
            else { out = out substr(line, 1, end - 1) " "; line = substr(line, end + 1 + length(hashes)); state = 0 }
          }
        }
        if (state == 1) { out = out " | " lit; lit = " " }
        printf "%s:%d:%s\n", F, NR, out
      }' "$f" ;;
    *) awk -v F="$f" '{ printf "%s:%d:%s\n", F, NR, $0 }' "$f" ;;
  esac
done > "$view"

# the allowed lines are taken out of the view once
grep -E '^allow	' "$list" | cut -f2- > "$allow"
if [ -s "$allow" ]; then
  grep -Ev -f "$allow" "$view" > "$view.kept" || true
  mv "$view.kept" "$view"
fi

bad=0
while IFS=$'\t' read -r pattern say; do
  case "$pattern" in ''|'#'*|allow) continue ;; esac
  # the pattern is matched against the text, never against the path in front of it
  hits=$(grep -Ei -- "^[^:]*:[0-9]+:.*($pattern)" "$view" 2>/dev/null || true)
  [ -n "$hits" ] || continue
  bad=1
  echo "✗ say: $say"
  printf '%s\n' "$hits" | cut -c1-200 | sed 's/^/    /'
done < "$list"
n=$(printf '%s\n' "$files" | wc -l | tr -d ' ')
if [ "$bad" -eq 0 ]; then echo "check-wording: OK ($n files)"; else echo "check-wording: a wording the glossary does not allow — docs/glossary.md says which word to use"; exit 1; fi
