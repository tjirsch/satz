#!/usr/bin/env bash
# Parse every Satz file of this checkout with the tree-sitter grammar the Zed
# extension pins (editors/zed/extension.toml) and fail on any parse error. The
# grammar mirrors the parser by hand, so a language change the grammar has not
# followed shows up here — and otherwise only as a mis-highlighted file.
#
#   scripts/check-grammar.sh                                  # the pinned commit, cloned
#   GRAMMAR=../satz-tree-sitter scripts/check-grammar.sh      # a local checkout instead
#
# Needs the tree-sitter CLI (brew install tree-sitter-cli) or node, through
# which npx fetches it. Not run by CI: the grammar repository is private.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
manifest=editors/zed/extension.toml

grammar_field() { # $1 key, read from the [grammars.satz] table
  awk -v key="$1" '
    /^\[grammars\.satz\]/ { s = 1; next }
    /^\[/                 { s = 0 }
    s && $1 == key        { gsub(/"/, "", $3); print $3 }
  ' "$manifest"
}
url=$(grammar_field repository)
rev=$(grammar_field rev)
[[ -n "$url" && -n "$rev" ]] || { echo "check-grammar: no [grammars.satz] repository/rev in $manifest" >&2; exit 1; }

if [[ -n "${GRAMMAR:-}" ]]; then
  dir=$GRAMMAR
  echo "check-grammar: grammar from $dir (pin is $rev)"
else
  dir=$(mktemp -d)
  trap 'rm -rf "$dir"' EXIT
  git clone -q "$url" "$dir"
  git -C "$dir" checkout -q "$rev"
  echo "check-grammar: grammar $url @ $rev"
fi

if command -v tree-sitter >/dev/null; then ts=(tree-sitter); else ts=(npx --yes tree-sitter-cli@0.27.0); fi

files=$(find presets tests -name '*.satz' ! -name '*.diff.satz' | sort | sed "s|^|$root/|")
[[ -n "$files" ]] || { echo "check-grammar: no .satz files found" >&2; exit 1; }

# `parse -q` prints only the files with errors and exits non-zero on any.
cd "$dir"
printf '%s\n' "$files" | xargs "${ts[@]}" parse -q --stat
