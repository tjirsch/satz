#!/usr/bin/env python3
"""Render a docs/*.md file as a self-contained, theme-aware HTML page.

    uv run --with markdown scripts/build-satz-doc.py [MD] [OUT.html] [TITLE]

Defaults: docs/language.md → docs/language.html, title from the
first `# heading`. Any `![…](name.svg)` image whose file sits beside the
markdown is inlined and recoloured through CSS tokens so it follows the
viewer's theme. One source, two renderings: the markdown is what the repo
keeps and GitHub shows; this page is the same text.
"""

import html
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
  /* The last resort, after code_breaks(): a word that still cannot fit a line (a URL,
     two code spans joined by a slash) breaks where it must rather than push the page
     sideways. break-word, not anywhere: it leaves min-content alone, so a table still
     sizes its columns by whole words and scrolls inside its wrapper instead. */
  main { max-width: 74ch; margin: 0 auto; overflow-wrap: break-word; }
  h1 { font-family: "Newsreader", Georgia, "Times New Roman", serif; font-style: italic; font-weight: 400;
       font-size: clamp(40px, 7vw, 64px); line-height: 1.02; letter-spacing: -0.01em; margin: 72px 0 24px; text-wrap: balance; }
  /* A title that is a pack's name wraps at its punctuation (code_breaks); at 1.02 each
     line's code background paints over the underscores of the line above. */
  h1:has(code) { line-height: 1.2; }
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
  /* Table code breaks between its words and never inside one: a flag split after
     its hyphen reads as a hyphenated word. code_breaks() makes every word a span. */
  td code > span { white-space: nowrap; }
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
    svg = re.sub(
        r'<rect x="0" y="0" width="\d+" height="\d+" fill="#FFFFFF"/>\s*', "", svg
    )
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
        return f"<figure>{themed_svg(path)}<figcaption>{alt}</figcaption></figure>"

    return re.sub(r'<p><img alt="([^"]*)" src="([^"]+\.svg)"\s*/?></p>', repl, body)


# A word of inline code longer than this may break before a `/`, `.` or `_`.
# 27 is what fits a third of the 74ch text column in table code after the cell's
# and the code's padding: a longer word cannot share a row with two others, and
# left whole it sets its column's width and starves the rest. Outside tables it is
# the same rule, well inside the ~37 characters of body code a 360 px phone holds.
LONG_TOKEN = 27


def breakable(word: str) -> str:
    """<wbr> before each `/`, `.` or `_` that does not follow a `/`: before the
    punctuation, as a URL is broken, and `//` stays whole. A piece that is still
    longer than LONG_TOKEN (a camelCase constraint name, fifty letters without a
    dot) may also break where a lowercase letter meets a capital."""
    pieces = re.sub(r"(?<=[^/])(?=[/._])", "<wbr>", word).split("<wbr>")
    return "<wbr>".join(
        re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "<wbr>", piece)
        if len(html.unescape(piece)) > LONG_TOKEN
        else piece
        for piece in pieces
    )


def long_word(word: str) -> str:
    return breakable(word) if len(html.unescape(word)) > LONG_TOKEN else word


def code_words(text: str, each) -> str:
    """A <code> element with `each` applied to every run of non-spaces in `text`;
    the spaces stay where they are, and they are where a line may break."""
    return "<code>" + re.sub(r"[^ ]+", lambda m: each(m.group(0)), text) + "</code>"


def code_breaks(body: str) -> str:
    """Where inline code may break, so neither a table column nor a page is pushed
    wide by it. A code block is left alone: it scrolls inside itself.

    - In a table cell every word becomes a span the CSS keeps whole, so a long
      command wraps between its words and never after a flag's hyphen, and a long
      word may also break at its punctuation (LONG_TOKEN).
    - In a heading every word may break at its punctuation. Heading type is two to
      four times body size, so a pack's name alone outruns a phone and, at 64 px,
      the text column.
    - Elsewhere a long word may break at its punctuation.

    A slash that joins two code spans (`A`/`B`) is a break point, as a space
    between them would be.

    Neither <span> nor <wbr> adds a character, so copied code is unchanged.

    A pipe in a table cell is written `\\|`, inside code too. GitHub shows `|`;
    Python-Markdown keeps the backslash in code, so it is dropped there."""

    def cell_word(word: str) -> str:
        return f"<span>{long_word(word)}</span>"

    def inner(region: str, each, unescape_pipe: bool = False) -> str:
        def one(c: "re.Match[str]") -> str:
            text = c.group(1).replace("\\|", "|") if unescape_pipe else c.group(1)
            return code_words(text, each)

        return re.sub(r"<code>(.*?)</code>", one, region, flags=re.S)

    def region(m: "re.Match[str]") -> str:
        if m.group("pre"):
            return m.group(0)
        if m.group("cell"):
            return inner(m.group(0), cell_word, unescape_pipe=True)
        if m.group("head"):
            return inner(m.group(0), breakable)
        return code_words(m.group("code"), long_word)

    body = re.sub(
        r"(?P<pre><pre[^>]*>.*?</pre>)"
        r"|(?P<cell><td[^>]*>.*?</td>)"
        r"|(?P<head><h(?P<lvl>[1-6])[^>]*>.*?</h(?P=lvl)>)"
        r"|<code>(?P<code>.*?)</code>",
        region,
        body,
        flags=re.S,
    )
    return body.replace("</code>/<code>", "</code>/<wbr><code>")


def render(text: str) -> str:
    """Markdown to the page body: the one rendering path, for this script and
    for build-site.py."""
    body = markdown.markdown(
        text, extensions=["tables", "fenced_code", "toc"], output_format="html5"
    )
    body = body.replace("<table>", '<div class="tablewrap"><table>').replace(
        "</table>", "</table></div>"
    )
    return inline_images(code_breaks(body))


def main() -> None:
    text = MD.read_text(encoding="utf-8")
    title = TITLE
    if not title:
        m = re.search(r"^# (.+)$", text, re.M)
        title = m.group(1).strip() if m else MD.stem
    body = render(text)
    OUT.write_text(
        HEAD.format(title=title)
        + "<style>"
        + CSS
        + "</style>\n<main>\n"
        + body
        + "\n</main>\n",
        encoding="utf-8",
    )
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
