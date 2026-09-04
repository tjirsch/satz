#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Maintain tests/schemas/google.json — the trimmed provider schema the tests judge by.

The corpus and the smoke estate classify resource types through this fixture, the
same way production classifies them through a real provider schema. So the fixture
is not decoration: a type missing from it silently loses schema-derived detail (it
once lost every alert policy's notification_channels), and a type whose real schema
has MOVED ON since the fixture was cut means the snapshots pin yesterday's provider.

    uv run scripts/update_schema_fixture.py --check
    uv run scripts/update_schema_fixture.py --add google_compute_firewall_policy_rule

`--check` downloads the pinned provider and reports, per fixture type, whether it
still exists and whether its attribute or block set has drifted. Run it whenever the
provider pin moves — that is the trigger; nothing else notices.

`--add` inserts types from the real schema, sorted, leaving everything else byte for
byte. It never removes: what looks unreferenced usually is not. Three types in the
fixture appear in no `.satz` source at all because the EMITTER produces them from
structural nodes (`google_folder_iam_member` from a grant map inside a folder,
`google_cloud_identity_group_membership` from members, `google_logging_project_sink`
from a sink), and one type that IS referenced needs no schema because it sits inside
a raw `hcl { }` block. A scan of the sources would get both wrong, which is why
deletion is not offered.

Needs `tofu` on PATH. Nothing here talks to an organisation.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

FIXTURE = Path("tests/schemas/google.json")
CONFIG = Path("tests/smoke/config.toml")
PROVIDER_KEY = "registry.opentofu.org/hashicorp/google"


def pinned_version(explicit: str | None) -> str:
    if explicit:
        return explicit
    for line in CONFIG.read_text().splitlines():
        if line.startswith("provider_version"):
            return line.split("=", 1)[1].strip().strip('"')
    sys.exit(f"no provider_version in {CONFIG}; pass --provider-version")


def real_schema(version: str) -> dict:
    """The full provider schema, from a throwaway init in a temp directory."""
    with tempfile.TemporaryDirectory() as td:
        d = Path(td)
        (d / "main.tf").write_text(
            "terraform {\n"
            '  required_providers {\n    google = {\n      source  = "hashicorp/google"\n'
            f'      version = "{version}"\n    }}\n  }}\n}}\n'
        )
        init = subprocess.run(
            ["tofu", "init", "-backend=false", "-input=false", "-no-color"],
            cwd=d,
            capture_output=True,
            text=True,
        )
        if init.returncode != 0:
            sys.exit(f"tofu init failed:\n{init.stdout[-2000:]}{init.stderr[-2000:]}")
        out = subprocess.run(
            ["tofu", "providers", "schema", "-json"],
            cwd=d,
            capture_output=True,
            text=True,
        )
        if out.returncode != 0:
            sys.exit(f"tofu providers schema failed:\n{out.stderr[-2000:]}")
        schemas = json.loads(out.stdout)["provider_schemas"]
    for key, val in schemas.items():
        if key.endswith("hashicorp/google"):
            return val["resource_schemas"]
    sys.exit(f"the google provider is not in the schema output: {list(schemas)}")


def surface(block: dict) -> tuple[set[str], set[str]]:
    """The names a fixture type is actually consulted for: attributes and blocks."""
    b = block.get("block", {})
    return set(b.get("attributes", {})), set(b.get("block_types", {}))


def load_fixture() -> tuple[dict, dict]:
    doc = json.loads(FIXTURE.read_text())
    return doc, doc["provider_schemas"][PROVIDER_KEY]["resource_schemas"]


def write_fixture(doc: dict, types: dict) -> None:
    doc["provider_schemas"][PROVIDER_KEY]["resource_schemas"] = dict(
        sorted(types.items())
    )
    FIXTURE.write_text(json.dumps(doc, indent=2, sort_keys=False) + "\n")


def cmd_check(version: str) -> int:
    _, fixture_types = load_fixture()
    real = real_schema(version)
    gone, drifted = [], []
    for name, block in sorted(fixture_types.items()):
        if name not in real:
            gone.append(name)
            continue
        f_attrs, f_blocks = surface(block)
        r_attrs, r_blocks = surface(real[name])
        added = (r_attrs - f_attrs) | {f"{b} (block)" for b in r_blocks - f_blocks}
        removed = (f_attrs - r_attrs) | {f"{b} (block)" for b in f_blocks - r_blocks}
        if added or removed:
            drifted.append((name, sorted(added), sorted(removed)))

    print(f"fixture: {len(fixture_types)} type(s) against provider {version}")
    for name in gone:
        print(f"  GONE     {name} — no longer in the provider")
    for name, added, removed in drifted:
        print(f"  DRIFTED  {name}")
        if added:
            print(f"             + {', '.join(added)}")
        if removed:
            print(f"             - {', '.join(removed)}")
    if not gone and not drifted:
        print("  every fixture type matches the provider exactly")
        return 0
    print(
        "\nDrift is not automatically a bug: the fixture only has to carry what the tests\n"
        "consult. It IS the signal to re-cut a type when a test starts depending on an\n"
        "attribute the fixture predates — `--add <type>` overwrites it from the provider."
    )
    return 1


def cmd_add(version: str, wanted: list[str]) -> int:
    doc, fixture_types = load_fixture()
    real = real_schema(version)
    missing = [t for t in wanted if t not in real]
    if missing:
        sys.exit(f"not in provider {version}: {', '.join(missing)}")
    added, refreshed = [], []
    for t in wanted:
        (refreshed if t in fixture_types else added).append(t)
        fixture_types[t] = real[t]
    write_fixture(doc, fixture_types)
    if added:
        print(f"added:     {', '.join(sorted(added))}")
    if refreshed:
        print(f"refreshed: {', '.join(sorted(refreshed))}")
    print(f"{FIXTURE}: {len(fixture_types)} type(s)")
    print("Now run `UPDATE_CORPUS=1 cargo test corpus` and review the snapshot diff.")
    return 0


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--check",
        action="store_true",
        help="report fixture types that are gone or whose schema has drifted",
    )
    ap.add_argument(
        "--add", nargs="+", metavar="TYPE", help="insert or re-cut these types"
    )
    ap.add_argument(
        "--provider-version", help=f"default: the provider_version pinned in {CONFIG}"
    )
    args = ap.parse_args()
    if not args.check and not args.add:
        ap.error("nothing to do: pass --check or --add")
    if not FIXTURE.exists():
        sys.exit(f"{FIXTURE} not found — run from the repository root")

    version = pinned_version(args.provider_version)
    rc = 0
    if args.add:
        rc |= cmd_add(version, args.add)
    if args.check:
        rc |= cmd_check(version)
    sys.exit(rc)


if __name__ == "__main__":
    main()
