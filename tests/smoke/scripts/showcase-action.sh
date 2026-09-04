#!/usr/bin/env bash
# Fixture for the `action` language feature — the target of showcase.satz's
# `action "showcase-step"`, exercised by scripts/smoke.sh.
#
# It changes nothing anywhere. All it does is echo what satz handed it, which is
# what the smoke step asserts on: the resolved argument list, the environment
# satz promises an action (and only that environment), and the fact that
# `--apply` arrives under `--execute` and not under `--check`.
set -euo pipefail

echo "showcase-action: name=${SATZ_ACTION:-unset} phase=${SATZ_PHASE:-unset} mode=${SATZ_MODE:-unset}"
echo "showcase-action: cwd=$(basename "$PWD")"
echo "showcase-action: args: $*"

for a in "$@"; do
  if [ "$a" = "--apply" ]; then
    echo "showcase-action: WRITE MODE"
  fi
done

# A param the estate did NOT put in `args` must not arrive by the back door:
# satz exports its own five variables and nothing else.
echo "showcase-action: customer_domain=${customer_domain:-not-exported}"
