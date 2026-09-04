#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Refresh presets/managed-constraint-equivalents.txt from a live organisation.

Google publishes, per org-policy constraint, the constraint that replaces it
(`equivalentConstraint`). That pairing is the whole input to the rule the packs
follow — run the managed replacement ALONE, never both forms — so it belongs in
the repository as data, generated, rather than in someone's memory of the last
manual audit.

    uv run scripts/update_constraint_equivalents.py            # auto-detect org + quota project
    uv run scripts/update_constraint_equivalents.py --org 123456789012

Needs Application Default Credentials and a quota project: the OrgPolicy API
refuses bare ADC. Nothing about the organisation is written to the file — only
constraint names, which are Google's and identical for every customer.

The file has two sections. The GENERATED one is rewritten wholesale on every run
and must never be hand-edited. The CURATED one below it is preserved untouched:
it holds pairs Google does not declare, which exist — `iam.allowedPolicyMemberDomains`
and `iam.managed.allowedPolicyMembers` replace each other in practice and share
no `equivalentConstraint` in either direction.
"""

import argparse
import json
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_OUT = Path("presets/managed-constraint-equivalents.txt")
GENERATED_MARKER = "# --- GENERATED, do not edit: rewritten by scripts/update_constraint_equivalents.py"
CURATED_MARKER = "# --- CURATED, maintained by hand: pairs Google does not declare"

HEADER = f"""# Managed org-policy constraints and the legacy constraints they replace.
#
# One pair per line, tab-separated: <legacy>\t<managed>\t<origin>\t<note>
#
# The packs run the managed replacement ALONE and declare the legacy twin off
# (`spec {{ reset = true }}`) — see "Superseded legacy constraints" in
# presets/README.md for why both forms in force is a defect. The test
# `constraint_equivalents::no_pack_runs_a_superseded_constraint` in src/main.rs
# reads this file and fails the build when a pack breaks that rule in either
# direction.
#
# Most managed constraints are NOT in here: they were born managed and have no
# legacy form, so there is nothing to switch off.
#
{GENERATED_MARKER}
"""

CURATED_HEADER = f"""
{CURATED_MARKER}
# Each line needs a note saying why we assert a pairing the API does not.
"""

CURATED_SEED = (
    "iam.allowedPolicyMemberDomains\tiam.managed.allowedPolicyMembers\tOURS\t"
    "Different names, no equivalentConstraint in either direction, but they enforce the same "
    "control: Domain Restricted Sharing. The legacy one cannot name exceptions for specific "
    "principals, so Google's own remedy for granting a service agent is to disable it org-wide, "
    "grant, and re-enable — running both turns every service activation into a window with the "
    "control off. CIS pack v2.0 dropped it, v2.5 declares it reset.\n"
)


def sh(*args: str) -> str:
    return subprocess.run(
        args, capture_output=True, text=True, check=False
    ).stdout.strip()


def fetch_constraints(org: str, token: str, quota_project: str) -> list[dict]:
    url = f"https://orgpolicy.googleapis.com/v2/organizations/{org}/constraints?pageSize=1000"
    out: list[dict] = []
    while url:
        req = urllib.request.Request(
            url,
            headers={
                "Authorization": f"Bearer {token}",
                "x-goog-user-project": quota_project,
            },
        )
        try:
            with urllib.request.urlopen(req) as resp:
                page = json.load(resp)
        except urllib.error.HTTPError as e:
            sys.exit(f"OrgPolicy API {e.code}: {e.read().decode()[:400]}")
        out.extend(page.get("constraints", []))
        tok = page.get("nextPageToken")
        url = f"{url.split('&pageToken=')[0]}&pageToken={tok}" if tok else None
    return out


def pairs_from(constraints: list[dict]) -> list[tuple[str, str]]:
    """Every declared equivalence, read in BOTH directions.

    The declaration is asymmetric in Google's data — far more managed constraints
    name their legacy twin than the other way round — so reading one side only
    finds a fraction of the pairs.
    """
    short = {c["name"].split("/")[-1]: c for c in constraints}
    found: set[tuple[str, str]] = set()
    for name, c in short.items():
        eq = c.get("equivalentConstraint", "").split("/")[-1]
        if not eq:
            continue
        a, b = (name, eq) if ".managed." in eq else (eq, name)
        if ".managed." not in b or ".managed." in a:
            continue  # not a legacy/managed pair; skip rather than guess
        found.add((a, b))
    return sorted(found)


def existing_curated(path: Path) -> str:
    if not path.exists():
        return CURATED_HEADER + CURATED_SEED
    text = path.read_text()
    idx = text.find(CURATED_MARKER)
    if idx == -1:
        return CURATED_HEADER + CURATED_SEED
    return "\n" + text[idx:].rstrip("\n") + "\n"


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--org",
        dest="organization",
        help="organisation id; default: the first one gcloud lists",
    )
    ap.add_argument(
        "--quota-project",
        help="billing/quota project; default: the active gcloud project",
    )
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()

    org = (
        args.organization
        or sh("gcloud", "organizations", "list", "--format=value(name)").split("\n")[0]
    )
    if not org:
        sys.exit("no organisation found: pass --org, or log in with gcloud")
    org = org.rsplit("/", 1)[-1]
    quota = args.quota_project or sh("gcloud", "config", "get-value", "project")
    if not quota:
        sys.exit(
            "no quota project: pass --quota-project, or set one with gcloud config set project"
        )
    token = sh("gcloud", "auth", "application-default", "print-access-token")
    if not token:
        sys.exit("no ADC token: run `gcloud auth application-default login`")

    constraints = fetch_constraints(org, token, quota)
    if not constraints:
        sys.exit("the API returned no constraints — refusing to write an empty table")
    pairs = pairs_from(constraints)
    managed = sum(1 for c in constraints if ".managed." in c["name"].split("/")[-1])

    body = "".join(f"{legacy}\t{mgd}\tGOOGLE\n" for legacy, mgd in pairs)
    args.out.write_text(HEADER + body + existing_curated(args.out))

    print(
        f"{args.out}: {len(pairs)} declared pair(s) from {len(constraints)} constraints "
        f"({managed} managed, {managed - len(pairs)} of them with no legacy form)"
    )


if __name__ == "__main__":
    main()
