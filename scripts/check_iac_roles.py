#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Check satz's IaC role table against Google's predefined role definitions.

The table (`src/iac_roles.rs`, printed by `satz iac-roles --format json`) names,
per resource type, a permission and the predefined roles that carry it. This reads
each named role from the IAM API and fails when none of an entry's roles carries
its permission — a typo in the table, a permission Google renamed, or a role Google
narrowed.

    uv run scripts/check_iac_roles.py                 # the table of this checkout (cargo run)
    uv run scripts/check_iac_roles.py --satz satz     # the table of an installed binary

Needs Application Default Credentials (`gcloud auth application-default login`);
predefined roles are Google's and the same for every organisation, so any
credential that can call the IAM API will do. Workspace entries are not IAM roles
and are skipped.
"""

import argparse
import json
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def table(satz: str | None) -> dict:
    cmd = (
        [satz, "iac-roles", "--format", "json"]
        if satz
        else ["cargo", "run", "--quiet", "--", "iac-roles", "--format", "json"]
    )
    try:
        out = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"could not read the table ({' '.join(cmd)}): {e}")
    return json.loads(out.stdout)


def adc_token() -> str:
    try:
        out = subprocess.run(
            ["gcloud", "auth", "application-default", "print-access-token"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"no ADC token ({e}) — run `gcloud auth application-default login`")
    return out.stdout.strip()


def role_permissions(role: str, token: str) -> set[str]:
    req = urllib.request.Request(
        f"https://iam.googleapis.com/v1/{role}",
        headers={"Authorization": f"Bearer {token}"},
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return set(json.load(resp).get("includedPermissions", []))
    except urllib.error.HTTPError as e:
        sys.exit(f"{role}: the IAM API answered {e.code} {e.reason}")


def entries(t: dict) -> list[tuple[str, dict]]:
    """(where, entry) for every ROLE entry, `read` and each type.

    A type's row carries both halves of its prerequisite — `roles` and `apis`.
    Only the roles are checkable against Google's IAM API, which is what this
    script does; the API half is cross-checked against the import table's asset
    types by a unit test in src/prerequisites.rs.
    """
    out = [("read", e) for e in t.get("read", [])]
    for tf_type, row in t.get("types", {}).items():
        out.extend((tf_type, e) for e in row["roles"])
    return out


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--satz", help="a satz binary to read the table from (default: cargo run)"
    )
    args = parser.parse_args()

    checked = [
        (where, e)
        for where, e in entries(table(args.satz))
        if e.get("scope") != "workspace"
    ]
    if not checked:
        sys.exit("the table has no entry to check")
    token = adc_token()
    cache: dict[str, set[str]] = {}
    failures = []
    for where, e in checked:
        perm = e["permission"]
        carrying = []
        for role in e["roles"]:
            if role not in cache:
                cache[role] = role_permissions(role, token)
            if perm in cache[role]:
                carrying.append(role)
        if not carrying:
            failures.append(f"{where}: {perm} is in none of {', '.join(e['roles'])}")

    if failures:
        print("\n".join(failures))
        sys.exit(
            f"{len(failures)} of {len(checked)} entries do not hold against Google's role definitions"
        )
    print(
        f"{len(checked)} entries, {len(cache)} roles: every entry's permission is in one of its roles"
    )


if __name__ == "__main__":
    main()
