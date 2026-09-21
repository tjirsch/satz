#!/usr/bin/env python3
"""Refresh presets/cai-asset-types.txt from Google's published list of Cloud Asset Inventory types.

The page names each type as `service.googleapis.com/Kind`, broken for line wrapping with `<wbr>`
tags inside the name, so the names are read from the page's text with every tag removed. A type
the file carries that the page no longer names is not dropped: the run refuses and lists it,
because a type Google stopped publishing is a decision for whoever runs this, not for the script.
Delete such a line by hand after checking it, then run again.

    uv run scripts/update_cai_asset_types.py            # rewrite the file
    uv run scripts/update_cai_asset_types.py --check    # exit 1 when the file is behind the page

Then fill the import config from the new list:
`uv run --with ruamel.yaml scripts/update_import_config.py --config-file presets/import-config.yaml
--cai-types presets/cai-asset-types.txt`.
"""

import argparse
import datetime
import html
import re
import sys
import urllib.request
from pathlib import Path

URL = "https://docs.cloud.google.com/asset-inventory/docs/asset-types"
FILE = Path(__file__).resolve().parent.parent / "presets" / "cai-asset-types.txt"
NAME = re.compile(r"([a-z0-9-]+(?:\.[a-z0-9-]+)*\.googleapis\.com/[A-Z][A-Za-z0-9]*)")
HEADER = [
    "# Cloud Asset Inventory asset types (RESOURCE content type), one per line.",
    "# Source: {url} ({date}).",
    "# Read by scripts/update_import_config.py --cai-types: an import-config row gets",
    "# its asset_type only when the name derived from its Terraform type is in here.",
]


def published(page: str) -> list[str]:
    """Every asset type the page names, from its text with the markup removed."""
    text = html.unescape(re.sub(r"<[^>]+>", "", page))
    return sorted(set(NAME.findall(text)))


def carried(path: Path) -> list[str]:
    return [l.strip() for l in path.read_text().splitlines() if l.strip() and not l.startswith("#")]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true", help="exit 1 when the file is behind the page; write nothing")
    ap.add_argument("--url", default=URL)
    args = ap.parse_args()

    with urllib.request.urlopen(args.url, timeout=60) as r:
        page = r.read().decode("utf-8")
    new = published(page)
    old = carried(FILE)
    if len(new) < len(old) // 2:
        print(f"the page names {len(new)} types and the file carries {len(old)}: the page did not "
              f"read as the list — nothing written", file=sys.stderr)
        return 2
    gone = sorted(set(old) - set(new))
    if gone:
        print(f"{len(gone)} type(s) the file carries are no longer on the page — check each, delete "
              f"its line by hand if Google dropped it, then run again:", file=sys.stderr)
        for g in gone:
            print(f"  {g}", file=sys.stderr)
        return 1
    added = sorted(set(new) - set(old))
    if args.check:
        if added:
            print(f"presets/cai-asset-types.txt is behind the page by {len(added)} type(s):")
            for a in added:
                print(f"  {a}")
            return 1
        print(f"presets/cai-asset-types.txt carries all {len(new)} types the page names")
        return 0
    header = [h.format(url=args.url, date=datetime.date.today().isoformat()) for h in HEADER]
    FILE.write_text("\n".join(header) + "\n" + "\n".join(new) + "\n")
    print(f"{len(old)} -> {len(new)} types ({len(added)} added)")
    for a in added:
        print(f"  + {a}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
