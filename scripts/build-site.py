#!/usr/bin/env python3
"""Render the documentation site: every Markdown page the repo keeps, as
self-contained themed HTML, into one output directory (GitHub Pages).

    uv run --with markdown scripts/build-site.py [_site]

Pages: README.md → index.html, docs/*.md → docs/<name>.html,
presets/README.md → presets/index.html (and presets/CHANGELOG.md when it
exists). Links between the Markdown files are rewritten to their HTML twins;
every page gets the same navigation bar. The Markdown stays the source GitHub
shows — one text, two renderings. Rendering itself is `build-satz-doc.py`.
"""
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib

doc = importlib.import_module("build-satz-doc")

ROOT = Path(__file__).resolve().parent.parent
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "_site"

PAGES: list[tuple[Path, str, str]] = []  # (source md, output relative path, nav label)
PAGES.append((ROOT / "README.md", "index.html", "satz"))
for md in sorted((ROOT / "docs").glob("*.md")):
    PAGES.append((md, f"docs/{md.stem}.html", md.stem))
PAGES.append((ROOT / "presets/README.md", "presets/index.html", "presets"))
if (ROOT / "presets/CHANGELOG.md").exists():
    PAGES.append((ROOT / "presets/CHANGELOG.md", "presets/changelog.html", "presets changelog"))

NAV_ORDER = ["satz", "satz-language", "presets", "presets changelog", "presets-workflow", "scripts", "security-toolset-integration"]


def nav_html(current_rel: str) -> str:
    depth = current_rel.count("/")
    up = "../" * depth
    items = []
    by_label = {label: rel for _, rel, label in PAGES}
    ordered = [l for l in NAV_ORDER if l in by_label] + sorted(l for l in by_label if l not in NAV_ORDER)
    for label in ordered:
        rel = by_label[label]
        cls = ' class="current"' if rel == current_rel else ""
        items.append(f'<a{cls} href="{up}{rel}">{label}</a>')
    return '<nav class="site">' + " ".join(items) + '<a href="https://github.com/tjirsch/satz">GitHub</a></nav>\n'


NAV_CSS = """
  nav.site { max-width: 62rem; margin: 0 auto; padding: .6rem 1.5rem 0; font-size: .9rem; display: flex; flex-wrap: wrap; gap: .3rem 1rem; }
  nav.site a { color: var(--ink-2); text-decoration: none; border-bottom: 1px solid transparent; }
  nav.site a:hover, nav.site a.current { color: var(--accent); border-bottom-color: var(--accent); }
"""


def command_anchors(body: str) -> str:
    """A heading that names a command in backticks — `### Transpile (`transpile`)` —
    gets a stable `id="cmd-transpile"` so `satz <cmd> --html-help` can open it.
    The rendered heading keeps its own generated id as a nested anchor."""

    def repl(m: "re.Match[str]") -> str:
        level, attrs, inner = m.group(1), m.group(2), m.group(3)
        cmds = re.findall(r"<code>([a-z][a-z0-9-]*)</code>", inner)
        if not cmds:
            return m.group(0)
        return f'<h{level}{attrs} id="cmd-{cmds[0]}">{inner}</h{level}>'

    return re.sub(r'<h([23])((?:\s+(?!id=)[a-z-]+="[^"]*")*)(?:\s+id="[^"]*")?>(.*?\(<code>[a-z][a-z0-9-]*</code>\).*?)</h\1>', repl, body)


def rewrite_links(body: str, src_rel: Path) -> str:
    """`docs/x.md` / `../README.md` / `#anchor` links → the rendered twins."""
    targets = {str(src.relative_to(ROOT)): rel for src, rel, _ in PAGES}

    def repl(m: "re.Match[str]") -> str:
        href = m.group(1)
        if href.startswith(("http://", "https://", "#", "mailto:")):
            return m.group(0)
        path, _, frag = href.partition("#")
        if not path.endswith(".md"):
            return m.group(0)
        target = (src_rel.parent / path).resolve().relative_to(ROOT.resolve()) if not path.startswith("/") else Path(path.lstrip("/"))
        html = targets.get(str(target))
        if not html:
            return m.group(0)
        here = Path(targets[str(src_rel)]).parent
        rel = Path(*([".."] * len(here.parts))) / html if here.parts else Path(html)
        return f'href="{rel.as_posix()}{"#" + frag if frag else ""}"'

    return re.sub(r'href="([^"]+)"', repl, body)


def main() -> None:
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    for src, rel, _label in PAGES:
        text = src.read_text(encoding="utf-8")
        m = re.search(r"^# (.+)$", text, re.M)
        title = m.group(1).strip() if m else src.stem
        doc.MD = src  # the renderer inlines SVGs relative to the source
        body = doc.markdown.markdown(text, extensions=["tables", "fenced_code", "toc"], output_format="html5")
        body = body.replace("<table>", '<div class="tablewrap"><table>').replace("</table>", "</table></div>")
        body = doc.inline_images(body)
        body = rewrite_links(body, src.relative_to(ROOT))
        body = command_anchors(body)
        out = OUT / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(
            doc.HEAD.format(title=title) + "<style>" + doc.CSS + NAV_CSS + "</style>\n" + nav_html(rel) + "<main>\n" + body + "\n</main>\n",
            encoding="utf-8",
        )
        print(f"wrote {out.relative_to(OUT)} ({out.stat().st_size} bytes)")
    (OUT / ".nojekyll").write_text("")


if __name__ == "__main__":
    main()
