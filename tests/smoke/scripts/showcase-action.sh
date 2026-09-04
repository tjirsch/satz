#!/usr/bin/env bash
# Fixture for the `action` language feature — the target of showcase.satz's
# `action "showcase-step"` and showcase-pack.satz's `action "pack-step"`,
# exercised by scripts/smoke.sh.
#
# It is written in the shape docs/satz-language.md §6.13 documents — dry run
# unless the estate's `execute_args` passed `--apply`, unknown arguments refused,
# non-zero on misuse — plus the echoes the smoke step asserts on: the resolved
# argument list, the environment satz promises an action, and the fact that a
# param the estate did NOT put in `args` never arrives by the back door.
#
# It changes nothing anywhere.
set -euo pipefail

apply=0
org=""
pack=""
while [ $# -gt 0 ]; do
  case "$1" in
    --organization) org="$2"; shift 2 ;;
    --pack)         pack="$2"; shift 2 ;;
    --apply)        apply=1; shift ;;
    *) echo "showcase-action: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$org" ] || [ -n "$pack" ] || {
  echo "showcase-action: one of --organization / --pack is required" >&2
  exit 2
}

echo "showcase-action: name=${SATZ_ACTION:-unset} phase=${SATZ_PHASE:-unset} mode=${SATZ_MODE:-unset}"
echo "showcase-action: cwd=$(basename "$PWD")"
echo "showcase-action: target=${org:-$pack}"

# A param the estate did not name in `args` must not reach the environment:
# satz exports its own five variables and nothing else.
echo "showcase-action: customer_domain=${customer_domain:-not-exported}"

if [ "$apply" = 0 ]; then
  echo "showcase-action: DRY RUN — re-run with --execute to write."
  exit 0
fi
echo "showcase-action: WRITE MODE"
