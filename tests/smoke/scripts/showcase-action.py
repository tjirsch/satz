# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Fixture for a Python `action` — the target of showcase.satz's
`action "showcase-python-step"`, exercised by scripts/smoke.sh.

satz runs a `.py` action with `uv run --script`, so this file runs on every
platform and needs no executable bit. It is written in the shape
docs/language.md 6.13 documents — dry run unless the estate's `execute_args`
passed `--apply`, unknown arguments refused, non-zero on misuse — plus the
echoes the smoke step asserts on: the resolved argument list and the
environment satz promises an action.

It changes nothing anywhere.
"""

import os
import sys
from pathlib import Path

apply = False
org = ""
argv = sys.argv[1:]
while argv:
    arg = argv.pop(0)
    if arg == "--organization":
        org = argv.pop(0)
    elif arg == "--apply":
        apply = True
    else:
        print(f"showcase-python: unknown argument: {arg}", file=sys.stderr)
        raise SystemExit(2)

if not org:
    print("showcase-python: --organization is required", file=sys.stderr)
    raise SystemExit(2)

name = os.environ.get("SATZ_ACTION", "unset")
phase = os.environ.get("SATZ_PHASE", "unset")
mode = os.environ.get("SATZ_MODE", "unset")
print(f"showcase-python: name={name} phase={phase} mode={mode}")
print(f"showcase-python: cwd={Path.cwd().name}")
print(f"showcase-python: target={org}")

if not apply:
    print("showcase-python: DRY RUN — re-run with --execute to write.")
    raise SystemExit(0)
print("showcase-python: WRITE MODE")
