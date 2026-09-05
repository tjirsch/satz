#!/usr/bin/env bash
#
# fleet-v1.sh — does every estate still compile on the current binary, and does
# it still emit the same infrastructure?
#
#   scripts/fleet-v1.sh [options] [ESTATE...]
#
# V1 is the check that catches an estate which quietly stopped compiling, or one
# whose emitted resource SET moved under a language tightening. Releases are
# small and frequent; an estate is transpiled when someone happens to touch it.
# Between those two facts is the gap this closes, so run it after every release.
#
# Three rules, each of them the reason a naive version of this script is worse
# than nothing:
#
#   * NOTHING IN A CHECKOUT IS TOUCHED. Every estate is copied to a scratch
#     directory and transpiled there. Estate repositories carry work in progress;
#     a verification pass that writes into one costs more than it proves.
#   * THE COMPARE IS BLOCK-LEVEL AND ORDER-INSENSITIVE. `git diff hcl/` reports a
#     block that merely moved, which across many releases buries the finding that
#     matters. Blocks are matched by ADDRESS, so only content counts.
#   * A MISSING CHECKOUT IS NOT A PASS. It is reported UNAVAILABLE and, with
#     --require-all, fails the run. Silence about an estate nobody checked is how
#     a fleet report starts lying.
#
# Two findings, and they are not the same severity:
#
#   BLOCKER   the estate does not transpile, or its ADDRESS SET moved — a
#             resource appeared or disappeared. Nothing ships on top of that.
#   delta     the address set is identical and some block BODY differs. The
#             infrastructure is the same shape; an attribute is rendered
#             differently. Carry it into the estate's next pickup.
#
# Exit: 0 clean · 1 blocker (or --require-all with an unavailable estate) · 2 delta only.

set -euo pipefail

SATZ="${SATZ:-satz}"
roster="${FLEET_ROSTER:-}"
scratch=""
require_all=0
verbose=0
selected=""

usage() {
  cat <<'USAGE'
scripts/fleet-v1.sh [options] [ESTATE...]

ESTATE   a path to an estate checkout (the directory holding config.toml), or a
         roster code such as E01 when --roster is given. With none, every entry
         in the roster is checked.

  --roster FILE   which estates exist and where. Either `code<TAB>path` lines
                  (# comments allowed), or a markdown table whose FIRST cell is
                  the code and whose first backticked absolute-or-~ path is the
                  checkout — so a fleet note you already keep works unchanged.
                  A row without both is prose, and is skipped. Default:
                  $FLEET_ROSTER.
  --scratch DIR   where copies are transpiled (default: a temp dir, removed).
                  Give one to keep the emitted trees for inspection.
  --require-all   an estate with no usable checkout fails the run.
  -v, --verbose   print a unified diff of every changed block.

  SATZ=<path>     which binary to verify with (default: satz on PATH).

Examples:
  scripts/fleet-v1.sh ~/estates/acme
  scripts/fleet-v1.sh --roster ~/fleet.tsv
  scripts/fleet-v1.sh --roster ~/fleet.tsv --require-all E01 E02
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --roster) roster="${2:?--roster needs a file}"; shift 2 ;;
    --scratch) scratch="${2:?--scratch needs a directory}"; shift 2 ;;
    --require-all) require_all=1; shift ;;
    -v|--verbose) verbose=1; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "fleet-v1: unknown option '$1'" >&2; usage >&2; exit 64 ;;
    *) selected="$selected$1"$'\n'; shift ;;
  esac
done

command -v "$SATZ" >/dev/null 2>&1 || { echo "fleet-v1: '$SATZ' is not on PATH" >&2; exit 66; }
command -v rsync >/dev/null 2>&1 || { echo "fleet-v1: rsync is required" >&2; exit 66; }
command -v python3 >/dev/null 2>&1 || { echo "fleet-v1: python3 is required" >&2; exit 66; }

if [ -z "$scratch" ]; then
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/fleet-v1.XXXXXX")"
  trap 'rm -rf "$scratch"' EXIT
else
  mkdir -p "$scratch"
fi

version="$("$SATZ" --version 2>/dev/null </dev/null | tail -1)"
printf 'fleet-v1 · %s · scratch %s\n\n' "$version" "$scratch"

# ---------------------------------------------------------------------------
# The roster: one `code<TAB>path` line per estate. A bare path on the command
# line is its own entry, so the script is usable with no roster at all.
# ---------------------------------------------------------------------------
list="$scratch/roster.tsv"
: > "$list"
printf '%s' "$selected" | while IFS= read -r item; do
  [ -n "$item" ] || continue
  case "$item" in
    */*|.|..) printf '%s\t%s\n' "$(basename "$item")" "$item" >> "$list" ;;
  esac
done

if [ -n "$roster" ]; then
  [ -f "$roster" ] || { echo "fleet-v1: roster not found: $roster" >&2; exit 66; }
  codes="$(printf '%s' "$selected" | grep -v '/' || true)"
  python3 - "$roster" "$codes" >> "$list" <<'PY'
import os
import re
import sys

# A roster is read out of a document a human maintains, so the parser has to be
# STRICT rather than forgiving. A loose one finds "estates" in prose — two words
# on a line, a backticked path in an unrelated table — and every one of them is
# then reported UNAVAILABLE, which buries the estates that really were not
# checked under noise. Both shapes below require a code AND a path that looks
# like a path; anything else is not a roster line.
CODE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,15}\Z")
PATH = re.compile(r"(~|\.{0,2})/\S*\Z")

path_file, codes = sys.argv[1], sys.argv[2].split()
wanted = set(codes)
home = os.path.expanduser("~")
seen = set()

for line in open(path_file, encoding="utf-8"):
    line = line.rstrip("\n")
    if not line.strip() or line.lstrip().startswith("#"):
        continue
    if line.lstrip().startswith("|"):
        # A markdown table row. The code is the FIRST cell — not any cell that
        # happens to look like one — and the checkout is the first backticked
        # path. A row without both is prose in a table, and is skipped.
        cells = [c.strip() for c in line.split("|")]
        if len(cells) < 3:
            continue
        code = cells[1]
        path = ""
        for c in cells[2:]:
            # The cell may carry a note after the path ("`~/a/b` (also `~/c`)"),
            # so match the LEADING backticked token rather than the whole cell —
            # while still requiring it to be shaped like a path, which is what
            # keeps prose out.
            m = re.match(r"`(\S+)`", c)
            if m and PATH.fullmatch(m.group(1)):
                path = m.group(1)
                break
    else:
        parts = line.split("\t") if "\t" in line else line.split()
        if len(parts) != 2:
            continue
        code, path = parts[0].strip(), parts[1].strip()
    if not CODE.fullmatch(code) or not PATH.fullmatch(path):
        continue
    if wanted and code not in wanted:
        continue
    if code in seen:
        continue
    seen.add(code)
    print(f"{code}\t{path.replace('~', home, 1)}")
PY
fi

if [ ! -s "$list" ]; then
  echo "fleet-v1: nothing to check — pass an estate path, or --roster FILE" >&2
  exit 64
fi

total=0; clean=0
unavailable=""; blockers=""; deltas=""

# ---------------------------------------------------------------------------
while IFS="$(printf '\t')" read -r code repo; do
  [ -n "$code" ] || continue
  total=$((total + 1))
  printf '== %s\n' "$code"

  skip=""
  if [ -z "$repo" ] || [ ! -d "$repo" ]; then
    skip="no checkout at ${repo:-<no path>}"
  elif [ ! -f "$repo/config.toml" ]; then
    skip="no config.toml in $repo"
  elif [ ! -d "$repo/hcl" ]; then
    skip="no emitted hcl/ to compare against"
  fi
  if [ -n "$skip" ]; then
    printf '   UNAVAILABLE — %s\n\n' "$skip"
    unavailable="$unavailable $code"
    continue
  fi

  # A dirty hcl/ means the baseline is the working tree rather than the commit.
  # That is usable, but the reader has to be told which one they are seeing.
  if git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
    dirty="$(git -C "$repo" status --porcelain -- hcl 2>/dev/null | wc -l | tr -d ' ')"
    if [ "$dirty" != "0" ]; then
      printf '   note: hcl/ has %s uncommitted change(s) — comparing against the working tree\n' "$dirty"
    fi
  fi

  work="$scratch/$code"
  rm -rf "$work"; mkdir -p "$work"
  rsync -a --exclude '.git/' --exclude 'hcl/' "$repo"/ "$work"/ </dev/null

  # A config path that escapes the repository would resolve to a different tree
  # in scratch, so the run would verify something else and say it was the estate.
  if grep -Eq '^(include_dirs|presets_dir|schema_dir|yaml_dir)[^=]*=.*(\.\./|"/)' "$work/config.toml"; then
    printf '   UNAVAILABLE — config.toml points outside the repository; the scratch copy would not be faithful\n\n'
    unavailable="$unavailable $code"
    continue
  fi

  yaml_dir="$(sed -n 's/^yaml_dir *= *"\(.*\)"/\1/p' "$work/config.toml" | head -1)"
  yaml_dir="${yaml_dir:-yaml}"
  estates="$(grep -lE '^estate[[:space:]]' "$work/$yaml_dir"/*.satz 2>/dev/null || true)"
  if [ -z "$estates" ]; then
    printf '   UNAVAILABLE — no file declaring an estate in %s/\n\n' "$yaml_dir"
    unavailable="$unavailable $code"
    continue
  fi

  failed=0; delta=0
  for est in $estates; do
    name="$(basename "$est")"
    if ! "$SATZ" --config "$work" transpile "$name" > "$work/.transpile.log" 2>&1 </dev/null; then
      printf '   BLOCKER — %s does not transpile\n' "$name"
      sed -n '1,12p' "$work/.transpile.log" | sed 's/^/      /'
      failed=1
      continue
    fi

    out="$(python3 - "$repo/hcl" "$work/hcl" "$verbose" <<'PY'
import difflib
import pathlib
import sys


def address(header: str) -> str:
    toks = [t.strip('"') for t in header.replace("{", " ").split()]
    if not toks:
        return "<anonymous>"
    if toks[0] == "resource" and len(toks) >= 3:
        return f"{toks[1]}.{toks[2]}"
    if toks[0] == "data" and len(toks) >= 3:
        return f"data.{toks[1]}.{toks[2]}"
    return ".".join(toks)


def normalise(body: str) -> str:
    keep = []
    for raw in body.splitlines():
        line = raw.rstrip()
        if not line.strip() or line.strip().startswith(("#", "//")):
            continue
        keep.append(line)
    return "\n".join(keep)


def blocks(root: pathlib.Path) -> dict[str, str]:
    """Top-level HCL blocks, keyed by address.

    Braces inside strings, comments and heredocs are not structure. Counting
    them would mis-split a file and report differences that are an artefact of
    this script rather than of the estate.
    """
    found: dict[str, list[str]] = {}
    for path in sorted(root.rglob("*.tf")):
        text = path.read_text(encoding="utf-8", errors="replace")
        depth = start = 0
        i, n = 0, len(text)
        in_str = in_line = in_blk = False
        heredoc = None
        while i < n:
            c = text[i]
            nxt = text[i + 1] if i + 1 < n else ""
            if heredoc is not None:
                j = text.find("\n", i)
                j = n if j < 0 else j
                if text[i:j].strip() == heredoc:
                    heredoc = None
                i = j + 1
                continue
            if in_line:
                if c == "\n":
                    in_line = False
                i += 1
                continue
            if in_blk:
                if c == "*" and nxt == "/":
                    in_blk = False
                    i += 2
                    continue
                i += 1
                continue
            if in_str:
                if c == "\\":
                    i += 2
                    continue
                if c == '"':
                    in_str = False
                i += 1
                continue
            if c == "#" or (c == "/" and nxt == "/"):
                in_line = True
                i += 1
                continue
            if c == "/" and nxt == "*":
                in_blk = True
                i += 2
                continue
            if c == '"':
                in_str = True
                i += 1
                continue
            if c == "<" and nxt == "<":
                k = i + 2
                if k < n and text[k] == "-":
                    k += 1
                m = k
                while m < n and (text[m].isalnum() or text[m] == "_"):
                    m += 1
                if m > k:
                    heredoc = text[k:m]
                    i = m
                    continue
            if c == "{":
                if depth == 0:
                    start = i
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    before = text[:start].strip()
                    header = before.splitlines()[-1] if before else ""
                    found.setdefault(address(header), []).append(normalise(text[start + 1 : i]))
                    text = text[i + 1 :]
                    n, i = len(text), -1
            i += 1
    # A header may legitimately repeat — two `provider "google"` blocks differing
    # only by alias. Index those by sorted body so an unchanged pair still
    # matches; a changed one shows as one added and one removed, which is
    # honest rather than silently paired.
    out: dict[str, str] = {}
    for key, bodies in found.items():
        if len(bodies) == 1:
            out[key] = bodies[0]
        else:
            for idx, body in enumerate(sorted(bodies)):
                out[f"{key}~{idx}"] = body
    return out


old_root, new_root = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
verbose = sys.argv[3] == "1"
old, new = blocks(old_root), blocks(new_root)

added = sorted(set(new) - set(old))
removed = sorted(set(old) - set(new))
changed = sorted(a for a in set(old) & set(new) if old[a] != new[a])

print(f"ADDR {len(added)} {len(removed)}")
print(f"BODY {len(changed)}")
for a in added:
    print(f"  + {a}")
for a in removed:
    print(f"  - {a}")
for a in changed:
    print(f"  ~ {a}")
    if verbose:
        for line in difflib.unified_diff(
            old[a].splitlines(), new[a].splitlines(), "emitted-before", "emitted-now", lineterm="", n=1
        ):
            print(f"      {line}")
PY
)"

    n_added="$(printf '%s' "$out" | awk '/^ADDR/{print $2}')"
    n_removed="$(printf '%s' "$out" | awk '/^ADDR/{print $3}')"
    n_body="$(printf '%s' "$out" | awk '/^BODY/{print $2}')"
    detail="$(printf '%s' "$out" | grep -E '^  ' || true)"

    if [ "$n_added" != "0" ] || [ "$n_removed" != "0" ]; then
      printf '   BLOCKER — %s: address set moved (+%s / -%s); %s body delta(s)\n' \
        "$name" "$n_added" "$n_removed" "$n_body"
      printf '%s\n' "$detail" | sed 's/^/   /'
      failed=1
    elif [ "$n_body" != "0" ]; then
      printf '   delta — %s: address set identical, %s block(s) differ in body\n' "$name" "$n_body"
      printf '%s\n' "$detail" | sed 's/^/   /'
      delta=1
    else
      printf '   clean — %s: address set identical, no body delta\n' "$name"
    fi
  done

  if [ "$failed" = "1" ]; then blockers="$blockers $code"
  elif [ "$delta" = "1" ]; then deltas="$deltas $code"
  else clean=$((clean + 1)); fi
  printf '\n'
done < "$list"

# ---------------------------------------------------------------------------
n_delta=$(printf '%s' "$deltas" | wc -w | tr -d ' ')
n_block=$(printf '%s' "$blockers" | wc -w | tr -d ' ')
n_unavail=$(printf '%s' "$unavailable" | wc -w | tr -d ' ')

printf -- '---\n'
printf 'checked %s · clean %s · delta %s · BLOCKER %s · unavailable %s\n' \
  "$total" "$clean" "$n_delta" "$n_block" "$n_unavail"
if [ "$n_delta" != "0" ]; then printf 'delta:      %s\n' "$deltas"; fi
if [ "$n_block" != "0" ]; then printf 'BLOCKER:    %s\n' "$blockers"; fi
if [ "$n_unavail" != "0" ]; then printf 'unavailable:%s\n' "$unavailable"; fi

if [ "$n_block" != "0" ]; then
  exit 1
fi
if [ "$require_all" = "1" ] && [ "$n_unavail" != "0" ]; then
  printf 'fleet-v1: --require-all, and %s estate(s) were never checked\n' "$n_unavail"
  exit 1
fi
if [ "$n_delta" != "0" ]; then
  exit 2
fi
exit 0
