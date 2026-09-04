#!/usr/bin/env python3
"""Render a docs/*.md file as a self-contained, theme-aware HTML page.

    uv run --with markdown scripts/build-satz-doc.py [MD] [OUT.html] [TITLE]

Defaults: docs/language.md → docs/language.html, title from the
first `# heading`. Any `![…](name.svg)` image whose file sits beside the
markdown is inlined and recoloured through CSS tokens so it follows the
viewer's theme. One source, two renderings: the markdown is what the repo
keeps and GitHub shows; this page is the same text.
"""
import re
import sys
from pathlib import Path

import markdown

ROOT = Path(__file__).resolve().parent.parent
MD = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs" / "language.md"
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else MD.with_suffix(".html")
TITLE = sys.argv[3] if len(sys.argv) > 3 else None

CSS = """
  :root {
    --ground:#F5F7F8; --surface:#FFFFFF; --ink:#1B2430; --ink-2:#4E5A66; --muted:#7A8792;
    --line:#D5DBE0; --code-bg:#EDF0F2; --band:#EEF1F3; --accent:#0F766E; --accent-soft:#D7ECE8;
  }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme="light"]) {
      --ground:#12171C; --surface:#171D23; --ink:#E6EAEE; --ink-2:#B4BDC5; --muted:#8A959F;
      --line:#2A333C; --code-bg:#1A2129; --band:#1B2229; --accent:#3CC1B0; --accent-soft:#163B37;
    }
  }
  :root[data-theme="dark"] {
    --ground:#12171C; --surface:#171D23; --ink:#E6EAEE; --ink-2:#B4BDC5; --muted:#8A959F;
    --line:#2A333C; --code-bg:#1A2129; --band:#1B2229; --accent:#3CC1B0; --accent-soft:#163B37;
  }
  html { background: var(--ground); }
  body { background: var(--ground); color: var(--ink); font-family: "IBM Plex Sans","Helvetica Neue",Arial,sans-serif;
         font-size: 16px; line-height: 1.6; margin: 0; padding: 0 20px 96px; }
  main { max-width: 74ch; margin: 0 auto; }
  h1 { font-family: "Newsreader", Georgia, "Times New Roman", serif; font-style: italic; font-weight: 400;
       font-size: clamp(40px, 7vw, 64px); line-height: 1.02; letter-spacing: -0.01em; margin: 72px 0 24px; text-wrap: balance; }
  h2 { font-weight: 600; font-size: 26px; line-height: 1.2; margin: 72px 0 16px; text-wrap: balance; }
  h3 { font-weight: 600; font-size: 18.5px; margin: 40px 0 10px; text-wrap: balance; }
  p { margin: 0 0 16px; }
  ul, ol { padding-left: 22px; margin: 0 0 16px; }
  li { margin-bottom: 6px; }
  a { color: var(--accent); }
  a:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  hr { border: 0; border-top: 1px solid var(--line); margin: 56px 0; }
  code, pre { font-family: "IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace; font-size: 0.9em; }
  code { background: var(--code-bg); padding: 1px 5px; border-radius: 3px; }
  pre { background: var(--code-bg); border: 1px solid var(--line); border-radius: 4px; padding: 14px 16px;
        overflow-x: auto; line-height: 1.5; margin: 0 0 18px; font-size: 13.5px; }
  pre code { background: none; padding: 0; font-size: inherit; }
  .tablewrap { overflow-x: auto; margin: 0 0 24px; }
  table { border-collapse: collapse; width: 100%; font-size: 14.5px; }
  th, td { text-align: left; vertical-align: top; padding: 9px 12px 9px 0; border-bottom: 1px solid var(--line); }
  th { font-weight: 600; font-size: 12.5px; letter-spacing: 0.04em; text-transform: uppercase; color: var(--muted); }
  td code { white-space: nowrap; }
  figure { margin: 24px 0 40px; }
  figure svg { display: block; width: 100%; height: auto; color: var(--ink); }
  figcaption { font-size: 14px; color: var(--ink-2); margin: 14px 0 0; }
  .lede { font-size: 18px; color: var(--ink-2); }
  @media (prefers-reduced-motion: no-preference) { html { scroll-behavior: smooth; } }
"""

HEAD = """<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:ital,wght@0,400;0,500;1,400&family=IBM+Plex+Sans:ital,wght@0,400;0,500;0,600;1,400&family=Newsreader:ital,opsz,wght@1,6..72,400&display=swap">
"""


def themed_svg(path: Path) -> str:
    svg = path.read_text(encoding="utf-8")
    # drop the baked white background and the fixed size; keep the viewBox
    svg = re.sub(r'<rect x="0" y="0" width="\d+" height="\d+" fill="#FFFFFF"/>\s*', "", svg)
    svg = re.sub(r' width="\d+" height="\d+"', "", svg, count=1)
    svg = svg.replace("#1B2430", "currentColor").replace("#0F766E", "var(--accent)")
    svg = svg.replace("#EEF1F3", "var(--band)").replace("#FFFFFF", "var(--surface)")
    return svg


def inline_images(body: str) -> str:
    def repl(m: "re.Match[str]") -> str:
        alt, src = m.group(1), m.group(2)
        path = MD.parent / src
        if not path.exists():
            return m.group(0)
        return f'<figure>{themed_svg(path)}<figcaption>{alt}</figcaption></figure>'
    return re.sub(r'<p><img alt="([^"]*)" src="([^"]+\.svg)"\s*/?></p>', repl, body)


def main() -> None:
    text = MD.read_text(encoding="utf-8")
    title = TITLE
    if not title:
        m = re.search(r"^# (.+)$", text, re.M)
        title = m.group(1).strip() if m else MD.stem
    body = markdown.markdown(text, extensions=["tables", "fenced_code", "toc"], output_format="html5")
    body = body.replace("<table>", '<div class="tablewrap"><table>').replace("</table>", "</table></div>")
    body = inline_images(body)
    OUT.write_text(HEAD.format(title=title) + "<style>" + CSS + "</style>\n<main>\n" + body + "\n</main>\n",
                   encoding="utf-8")
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
