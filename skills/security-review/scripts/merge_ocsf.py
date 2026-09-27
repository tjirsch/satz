#!/usr/bin/env python3
"""Merge per-service Prowler OCSF files into one, dedupe by finding uid, write MANIFEST.md.

    python3 merge_ocsf.py --raw <raw-dir> --prefix <shortcode> [--fleet-regex '<re>'] [--extra-caveat '...']

Writes <raw>/<prefix>-prowler-merged.ocsf.json and <raw>/MANIFEST.md (files, counts, sha256,
services that hit the deadline, project split own/fleet). Prints the own-project list and the
per-check FAIL counts so the caller can sanity-check the scan before the offline phase.
"""

import argparse
import collections
import hashlib
import json
import re
import sys
from pathlib import Path


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()[:16] + "…"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--raw", required=True)
    ap.add_argument("--prefix", required=True)
    ap.add_argument("--fleet-regex", default="")
    ap.add_argument("--extra-caveat", action="append", default=[])
    a = ap.parse_args()
    raw = Path(a.raw).expanduser()
    merged_name = f"{a.prefix}-prowler-merged.ocsf.json"
    parts = sorted(p for p in raw.glob("*.ocsf.json") if p.name != merged_name)
    if not parts:
        sys.exit(f"no *.ocsf.json in {raw}")
    seen, out, rows = set(), [], []
    for p in parts:
        data = json.loads(p.read_text() or "[]")
        n = 0
        for f in data:
            uid = (
                f.get("finding_info", {}).get("uid")
                or json.dumps(f, sort_keys=True)[:200]
            )
            if uid in seen:
                continue
            seen.add(uid)
            out.append(f)
            n += 1
        rows.append((p.name, len(data), n, sha(p)))
    merged = raw / merged_name
    merged.write_text(json.dumps(out, ensure_ascii=False))
    # logs: which services ended how
    svc = []
    for log in sorted(raw.glob("svc-*.log")):
        t = log.read_text(errors="replace")
        state = (
            "DEADLINE"
            if "DEADLINE" in t
            else ("exit " + (re.findall(r"exit: (\d+)", t) or ["?"])[-1])
        )
        svc.append((log.stem[4:], state))
    fleet = re.compile(a.fleet_regex) if a.fleet_regex else None
    projects = sorted(
        {(f.get("cloud") or {}).get("account", {}).get("uid", "") for f in out} - {""}
    )
    own = [p for p in projects if not (fleet and fleet.match(p))]
    fl = [p for p in projects if fleet and fleet.match(p)]
    fails = collections.Counter(
        (f.get("metadata") or {}).get("event_code", "?")
        for f in out
        if f.get("status_code") == "FAIL"
    )
    lines = [
        f"# Rohdaten {a.prefix} — Prowler OCSF, Scan-Ordner {raw.parent.name}",
        "",
        "Prowler read-only, ein Lauf je Service (siehe svc-*.log). Dedupliziert nach finding_info.uid.",
        "",
        "| Datei | Findings | neu im Merge | sha256 |",
        "|---|---|---|---|",
    ]
    lines += [f"| {n} | {c} | {u} | {s} |" for n, c, u, s in rows]
    lines += [
        f"| **{merged_name}** | **{len(out)}** | | {sha(merged)} |",
        "",
        "## Services",
        "",
        "| Service | Ende |",
        "|---|---|",
    ] + [f"| {n} | {s} |" for n, s in svc]
    dead = [n for n, s in svc if s == "DEADLINE"]
    lines += [
        "",
        "## Scope",
        "",
        f"- Projekte mit Findings: {len(projects)} ({len(own)} eigene, {len(fl)} Flotte)",
        f"- eigene Projekte: {', '.join(own)}",
    ]
    caveats = list(a.extra_caveat)
    if dead:
        caveats.append(
            f"Services mit Deadline-Abbruch (kein Ergebnis, Kontrollen → N/A oder LÜCKE im Review): {', '.join(dead)}"
        )
    if not any("compute" in n for n, _ in svc):
        caveats.append(
            "Compute (CIS §4) nicht gescannt — kein --own-projects übergeben"
        )
    if caveats:
        lines += ["", "## Werkzeuggrenzen", ""] + [f"- {c}" for c in caveats]
    (raw / "MANIFEST.md").write_text("\n".join(lines) + "\n")
    print(f"merged {len(out)} findings from {len(parts)} files → {merged}")
    print(f"own_projects={' '.join(own)}")
    print(f"fleet_projects={len(fl)}")
    print("FAIL per check:")
    [print(f"  {n:5d} {c}") for c, n in fails.most_common()]
    if dead:
        print(f"DEADLINE: {', '.join(dead)}")


if __name__ == "__main__":
    main()
