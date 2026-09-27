#!/usr/bin/env python3
"""Verify the two deliverables before handing them over.

    uv run --with openpyxl --with python-docx python3 verify_outputs.py --xlsx <file> --docx <file> --customer <shortcode> [--deny name ...]

Checks: 93 control rows, statuses from the taxonomy, counts consistent, every FAIL/LÜCKE row has a measure Mn,
every MANUAL row has PFLICHT, no unresolved {placeholder}, the docx has every Mn the workbook references,
and no other customer's name appears (siblings under ~/projects/ccc plus --deny words). Exit 1 on any failure.
"""

import argparse
import os
import re
import sys
from pathlib import Path
from openpyxl import load_workbook
from docx import Document

TAX = ["FAIL", "LÜCKE", "MANUAL", "PASS", "FALSE POSITIVE", "N/A (KEINE RESSOURCEN)"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--xlsx", required=True)
    ap.add_argument("--docx", required=True)
    ap.add_argument("--customer", required=True)
    ap.add_argument("--deny", nargs="*", default=[])
    ap.add_argument("--controls", type=int, default=93)
    a = ap.parse_args()
    errs, warns = [], []
    wb = load_workbook(a.xlsx)
    ws = wb["CIS 5.0 Checkliste"]
    rows = [
        r
        for r in ws.iter_rows(min_row=2, values_only=True)
        if r[0] and re.match(r"^\d+(\.\d+)+$", str(r[0]))
    ]
    if len(rows) != a.controls:
        errs.append(f"checklist has {len(rows)} control rows, expected {a.controls}")
    bad = [r[0] for r in rows if r[5] not in TAX]
    if bad:
        errs.append(f"status outside taxonomy: {bad}")
    nomeasure = [
        r[0]
        for r in rows
        if r[5] in ("FAIL", "LÜCKE") and not (r[11] and re.search(r"M\d+", str(r[11])))
    ]
    if nomeasure:
        errs.append(f"FAIL/LÜCKE without measure: {nomeasure}")
    nopflicht = [r[0] for r in rows if r[5] == "MANUAL" and r[8] != "PFLICHT"]
    if nopflicht:
        errs.append(f"MANUAL without PFLICHT: {nopflicht}")
    emptybef = [r[0] for r in rows if not r[6]]
    if emptybef:
        errs.append(f"empty Befund: {emptybef}")
    ph = {
        m
        for r in rows
        for c in r
        if isinstance(c, str)
        for m in re.findall(r"\{[a-z_]+\}", c)
    }
    if ph:
        errs.append(f"unresolved placeholders in workbook: {sorted(ph)}")
    refs = {m for r in rows if r[11] for m in re.findall(r"M\d+", str(r[11]))}
    doc = Document(a.docx)
    text = (
        "\n".join(p.text for p in doc.paragraphs)
        + "\n"
        + "\n".join(c.text for t in doc.tables for row in t.rows for c in row.cells)
    )
    heads = {
        m
        for p in doc.paragraphs
        if p.style.name.startswith("Heading")
        for m in re.findall(r"\bM\d+\b", p.text)
    }
    missing = sorted(refs - heads, key=lambda x: int(x[1:]))
    if missing:
        errs.append(f"workbook references measures without a docx section: {missing}")
    ph = set(re.findall(r"\{[a-z_]+\}", text))
    if ph:
        errs.append(f"unresolved placeholders in docx: {sorted(ph)}")
    for must in (
        "Ausgangslage",
        "Reihenfolge",
        "Nachweis",
        "Offene Punkte",
        "Manuelle Kontrollen",
    ):
        if must not in text:
            errs.append(f"docx lacks section '{must}'")
    # other customers' names: sibling folders under the ccc root
    ccc = Path(os.environ.get("CCC_ROOT", Path.home() / "projects" / "ccc"))
    deny = set(a.deny)
    if ccc.is_dir():
        deny |= {
            d.name
            for d in ccc.iterdir()
            if d.is_dir() and d.name != a.customer and len(d.name) >= 3
        }
    deny -= {a.customer}
    blob = (text + "\n" + "\n".join(str(c) for r in rows for c in r if c)).lower()
    hits = sorted(
        n
        for n in deny
        if re.search(r"(?<![a-z0-9])" + re.escape(n.lower()) + r"(?![a-z0-9])", blob)
    )
    if hits:
        errs.append(f"other customer names present: {hits}")
    # local filesystem paths expose user names and the customer folder layout (PII) — never in a deliverable
    paths = sorted(
        set(
            re.findall(
                r"(?:/Users/|/home/|[A-Za-z]:\\\\Users\\\\|~/|\$HOME/|projects/ccc/)[^\s\"')]*",
                text + "\n" + blob,
            )
        )
    )
    if paths:
        errs.append(f"local paths present (PII): {paths[:8]}")
    ov = wb["Übersicht"]
    if not any(
        isinstance(c.value, str) and c.value.startswith("=COUNTIF")
        for row in ov.iter_rows()
        for c in row
    ):
        errs.append("Übersicht lacks COUNTIF formulas")
    print(
        f"rows {len(rows)}; statuses {dict((s, sum(1 for r in rows if r[5] == s)) for s in TAX)}; measures referenced {len(refs)}, in docx {len(heads)}"
    )
    for w in warns:
        print("WARN", w)
    for e in errs:
        print("FAIL", e)
    print("verify:", "OK" if not errs else f"{len(errs)} problem(s)")
    sys.exit(1 if errs else 0)


if __name__ == "__main__":
    main()
