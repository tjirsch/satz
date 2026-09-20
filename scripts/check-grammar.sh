#!/usr/bin/env bash
# Hold the tree-sitter grammar the Zed extension pins (editors/zed/extension.toml)
# against satz's own parser, in two checks:
#
#   1. Every statement keyword the parser dispatches on — `STATEMENT_KEYWORDS` in
#      crates/satz-core/src/satz.rs, which a unit test derives from the dispatch
#      itself — is a node or token the grammar declares in src/node-types.json.
#      A keyword the grammar has no rule for parses as a resource block, so the
#      parse below stays green while every tool over the grammar reads the file
#      differently from satz.
#   2. Every Satz file of this checkout parses with no ERROR or MISSING node.
#
#   scripts/check-grammar.sh                                  # the pinned commit, cloned
#   GRAMMAR=../satz-tree-sitter scripts/check-grammar.sh      # a local checkout instead
#
# Needs the tree-sitter CLI (brew install tree-sitter-cli) or node, through
# which npx fetches it. CI runs it: .github/workflows/smoke.yml, job `grammar`.
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

# 1. the two statement sets
node_types=$dir/src/node-types.json
[[ -f $node_types ]] || { echo "check-grammar: $node_types is not there — the grammar ships it beside src/parser.c" >&2; exit 1; }
python3 - "$root/crates/satz-core/src/satz.rs" "$node_types" <<'PY'
import json, re, sys

parser_src, node_types = sys.argv[1], sys.argv[2]
text = open(parser_src, encoding="utf-8").read()
m = re.search(r"pub const STATEMENT_KEYWORDS: &\[&str\] =\s*&\[(.*?)\];", text, re.S)
if not m:
    sys.exit(f"check-grammar: no STATEMENT_KEYWORDS in {parser_src} — the gate reads the parser's statement set from it")
keywords = re.findall(r'"([^"]+)"', m.group(1))
if not keywords:
    sys.exit(f"check-grammar: STATEMENT_KEYWORDS in {parser_src} is empty")

declared = {n["type"] for n in json.load(open(node_types, encoding="utf-8"))}
missing = [k for k in keywords if k not in declared]
if missing:
    sys.exit(
        "check-grammar: the grammar models no node for %d statement(s) satz parses: %s\n"
        "  A keyword the grammar has no rule for parses as a resource block, so the parse "
        "below would stay green.\n"
        "  Add the rule in the grammar repository, then bump `rev` in editors/zed/extension.toml."
        % (len(missing), ", ".join(missing))
    )
print(f"check-grammar: {len(keywords)} statement(s) satz parses, all modelled by the grammar")
PY

# 2. every Satz file of this checkout

files=$(find presets tests -name '*.satz' ! -name '*.diff.satz' | sort | sed "s|^|$root/|")
[[ -n "$files" ]] || { echo "check-grammar: no .satz files found" >&2; exit 1; }

# `parse -q` prints only the files with errors and exits non-zero on any.
cd "$dir"
printf '%s\n' "$files" | xargs "${ts[@]}" parse -q --stat
