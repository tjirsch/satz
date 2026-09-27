#!/usr/bin/env python3
"""Package every skill under skills/<name>/ as skills/<name>.skill, the zip a Claude skill is
uploaded as (Cowork, the desktop app). The folder is the source; the zip is derived from it
and never edited.

    python3 scripts/build-skills.py            # rewrite every skills/<name>.skill
    python3 scripts/build-skills.py --check    # exit 1 naming each zip that is behind its folder

The zip is byte-identical for one folder on every platform: entries are stored (not
compressed, so no zlib version shows), sorted by path, dated 1980-01-01 with the mode
0644 (0755 for scripts/*), and prefixed `<name>/` the way the skill format expects. Excluded:
`__pycache__`, `.pyc`, `.DS_Store`. The smoke matrix runs `--check`, so a skill change that
forgets to rebuild its zip fails CI.
"""

import argparse
import io
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKILLS = ROOT / "skills"
EXCLUDED_NAMES = {"__pycache__", ".DS_Store"}
EXCLUDED_SUFFIXES = {".pyc"}
EPOCH = (1980, 1, 1, 0, 0, 0)


def files_of(folder: Path) -> list[Path]:
    out = []
    for p in sorted(folder.rglob("*")):
        if not p.is_file():
            continue
        if any(part in EXCLUDED_NAMES for part in p.relative_to(folder).parts):
            continue
        if p.suffix in EXCLUDED_SUFFIXES:
            continue
        out.append(p)
    return out


def package(folder: Path) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as z:
        for p in files_of(folder):
            rel = p.relative_to(folder).as_posix()
            info = zipfile.ZipInfo(f"{folder.name}/{rel}", date_time=EPOCH)
            mode = 0o755 if rel.startswith("scripts/") else 0o644
            info.external_attr = (mode & 0xFFFF) << 16
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, p.read_bytes())
    return buf.getvalue()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument(
        "--check",
        action="store_true",
        help="exit 1 naming each zip that is behind its folder",
    )
    a = ap.parse_args()
    folders = (
        sorted(d for d in SKILLS.iterdir() if d.is_dir() and (d / "SKILL.md").is_file())
        if SKILLS.is_dir()
        else []
    )
    if not folders:
        print(
            f"build-skills: no skills/<name>/SKILL.md under {SKILLS}", file=sys.stderr
        )
        return 1
    behind = []
    for folder in folders:
        target = SKILLS / f"{folder.name}.skill"
        want = package(folder)
        if a.check:
            if not target.is_file() or target.read_bytes() != want:
                behind.append(target.relative_to(ROOT).as_posix())
            continue
        target.write_bytes(want)
        print(
            f"wrote {target.relative_to(ROOT)} ({len(files_of(folder))} files, {len(want)} bytes)"
        )
    if a.check:
        if behind:
            print(
                "build-skills: behind its folder — run scripts/build-skills.py and commit: "
                + ", ".join(behind),
                file=sys.stderr,
            )
            return 1
        print(f"build-skills --check: OK ({len(folders)} skill(s) current)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
