#!/usr/bin/env bash
# A stand-in for tofu in the smoke matrix: `show -json` prints the state file named
# by $FAKE_TOFU_STATE, every other call prints the arguments it was given. The
# smoke checks what `satz plan` and `satz apply` hand the tool, without a provider
# or an organisation.
case "$1" in
  show) cat "$FAKE_TOFU_STATE" ;;
  *) echo "fake-tofu $*" ;;
esac
