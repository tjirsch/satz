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
PACK_PAGES: list[tuple[Path, str]] = []  # derived per-pack pages: rendered, linked from the index, not in the nav
for md in sorted((ROOT / "presets/docs").glob("*.md")):
    PACK_PAGES.append((md, f"presets/docs/{'index' if md.stem == 'README' else md.stem}.html"))

NAV_ORDER = ["satz", "satz-language", "presets", "pack pages", "presets changelog", "presets-workflow", "scripts", "security-toolset-integration"]


def nav_html(current_rel: str) -> str:
    depth = current_rel.count("/")
    up = "../" * depth
    items = []
    by_label = {label: rel for _, rel, label in PAGES}
    if any(rel == "presets/docs/index.html" for _, rel in PACK_PAGES):
        by_label["pack pages"] = "presets/docs/index.html"
    ordered = [l for l in NAV_ORDER if l in by_label] + sorted(l for l in by_label if l not in NAV_ORDER)
    for label in ordered:
        rel = by_label[label]
        cls = ' class="current"' if rel == current_rel else ""
        items.append(f'<a{cls} href="{up}{rel}">{label}</a>')
    items.append('<a href="https://github.com/tjirsch/satz">GitHub</a>')
    return (
        f'<header class="site" data-root="{up}"><nav>'
        + " ".join(items)
        + '</nav><div class="search"><input id="satz-search" type="search" placeholder="Search the docs…" '
        + 'autocomplete="off" spellcheck="false" aria-label="Search the docs">'
        + '<div id="satz-search-results" hidden></div></div></header>\n'
        + f'<script defer src="{up}search-index.js"></script>\n'
    )


NAV_CSS = """
  header.site { position: sticky; top: 0; z-index: 10; background: var(--surface); border-bottom: 1px solid var(--line);
    display: flex; flex-wrap: wrap; align-items: center; gap: .4rem 1.25rem; padding: .55rem 1.5rem; }
  header.site nav { display: flex; flex-wrap: wrap; align-items: center; gap: .3rem 1.1rem; font-size: .95rem; font-weight: 500; }
  header.site nav a { color: var(--ink-2); text-decoration: none; padding: .15rem 0; border-bottom: 2px solid transparent; }
  header.site nav a:hover { color: var(--accent); }
  header.site nav a.current { color: var(--accent); border-bottom-color: var(--accent); }
  header.site .search { position: relative; margin-left: auto; flex: 0 1 22rem; min-width: 14rem; }
  #satz-search { width: 100%; font: inherit; font-size: .95rem; color: var(--ink); background: var(--ground);
    border: 1px solid var(--line); border-radius: .5rem; padding: .4rem .75rem; outline: none; }
  #satz-search:focus { border-color: var(--accent); background: var(--surface); }
  #satz-search-results { position: absolute; right: 0; left: 0; top: calc(100% + .35rem); max-height: 24rem; overflow-y: auto;
    background: var(--surface); border: 1px solid var(--line); border-radius: .5rem; box-shadow: 0 8px 28px rgba(0,0,0,.18); }
  #satz-search-results a { display: block; padding: .5rem .75rem; text-decoration: none; border-bottom: 1px solid var(--line); }
  #satz-search-results a:last-child { border-bottom: none; }
  #satz-search-results a.sel, #satz-search-results a:hover { background: var(--band); }
  #satz-search-results .where { color: var(--muted); font-size: .78rem; }
  #satz-search-results .head { color: var(--ink); font-weight: 600; font-size: .9rem; }
  #satz-search-results .head mark, #satz-search-results .snip mark { background: var(--accent-soft); color: var(--accent); }
  #satz-search-results .snip { color: var(--ink-2); font-size: .82rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #satz-search-results .none { padding: .5rem .75rem; color: var(--muted); font-size: .85rem; }
"""

SEARCH_JS = """
(function () {
  var input = document.getElementById('satz-search');
  var box = document.getElementById('satz-search-results');
  if (!input || !box || !window.SATZ_INDEX) return;
  var root = (document.querySelector('header.site') || {}).getAttribute
    ? (document.querySelector('header.site').getAttribute('data-root') || '') : '';
  var sel = -1, hits = [];
  function esc(s) { return s.replace(/[&<>"]/g, function (c) { return ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'})[c]; }); }
  function mark(s, terms) {
    var e = esc(s);
    terms.forEach(function (t) {
      if (!t) return;
      e = e.replace(new RegExp('(' + t.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&') + ')', 'ig'), '<mark>$1</mark>');
    });
    return e;
  }
  function search(q) {
    var terms = q.toLowerCase().split(/\\s+/).filter(Boolean);
    if (!terms.length) return [];
    return window.SATZ_INDEX.map(function (e) {
      var h = e.h.toLowerCase(), x = e.x.toLowerCase(), t = e.t.toLowerCase();
      var ok = terms.every(function (w) { return h.indexOf(w) >= 0 || x.indexOf(w) >= 0 || t.indexOf(w) >= 0; });
      if (!ok) return null;
      var score = terms.reduce(function (s, w) {
        if (h.indexOf(w) === 0) return s + 3;
        if (h.indexOf(w) >= 0) return s + 2;
        return s + 1;
      }, e.a ? 0 : 1);
      return { e: e, score: score };
    }).filter(Boolean).sort(function (a, b) { return b.score - a.score; }).slice(0, 12).map(function (r) { return r.e; });
  }
  function render(q) {
    hits = search(q); sel = -1;
    if (!q.trim()) { box.hidden = true; box.innerHTML = ''; return; }
    var terms = q.toLowerCase().split(/\\s+/).filter(Boolean);
    box.innerHTML = hits.length
      ? hits.map(function (e) {
          return '<a href="' + root + e.p + (e.a ? '#' + e.a : '') + '">'
            + '<div class="where">' + esc(e.t) + '</div>'
            + '<div class="head">' + mark(e.h, terms) + '</div>'
            + (e.x ? '<div class="snip">' + mark(e.x, terms) + '</div>' : '')
            + '</a>';
        }).join('')
      : '<div class="none">Nothing found.</div>';
    box.hidden = false;
  }
  function move(d) {
    var links = box.querySelectorAll('a');
    if (!links.length) return;
    sel = (sel + d + links.length) % links.length;
    links.forEach(function (l, i) { l.classList.toggle('sel', i === sel); });
    links[sel].scrollIntoView({ block: 'nearest' });
  }
  input.addEventListener('input', function () { render(input.value); });
  input.addEventListener('keydown', function (ev) {
    if (ev.key === 'ArrowDown') { ev.preventDefault(); move(1); }
    else if (ev.key === 'ArrowUp') { ev.preventDefault(); move(-1); }
    else if (ev.key === 'Enter') {
      var links = box.querySelectorAll('a');
      var l = links[sel >= 0 ? sel : 0];
      if (l) window.location.href = l.getAttribute('href');
    } else if (ev.key === 'Escape') { box.hidden = true; input.blur(); }
  });
  document.addEventListener('click', function (ev) {
    if (!box.contains(ev.target) && ev.target !== input) box.hidden = true;
  });
  document.addEventListener('keydown', function (ev) {
    if (ev.key === '/' && document.activeElement !== input) { ev.preventDefault(); input.focus(); }
  });
})();
"""


def strip_tags(html: str) -> str:
    return re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", html)).strip()


def index_entries(title: str, rel: str, body: str) -> list[dict]:
    """One entry per page plus one per h1–h3: heading, anchor and a short
    excerpt of the text that follows — what the search box looks through."""
    entries = []
    parts = re.split(r"(<h[123][^>]*>.*?</h[123]>)", body, flags=re.S)
    lead = strip_tags(parts[0])[:180]
    entries.append({"t": title, "p": rel, "a": "", "h": title, "x": lead})
    for i in range(1, len(parts), 2):
        m = re.match(r"<h([123])[^>]*?id=\"([^\"]+)\"[^>]*>(.*?)</h\1>", parts[i], flags=re.S)
        if not m:
            continue
        heading = strip_tags(m.group(3))
        follow = strip_tags(parts[i + 1] if i + 1 < len(parts) else "")[:180]
        entries.append({"t": title, "p": rel, "a": m.group(2), "h": heading, "x": follow})
    return entries



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
    targets.update({str(src.relative_to(ROOT)): rel for src, rel in PACK_PAGES})

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
    index: list[dict] = []
    for src, rel, _label in PAGES + [(s, r, "") for s, r in PACK_PAGES]:
        text = src.read_text(encoding="utf-8")
        m = re.search(r"^# (.+)$", text, re.M)
        title = m.group(1).strip() if m else src.stem
        doc.MD = src  # the renderer inlines SVGs relative to the source
        body = doc.markdown.markdown(text, extensions=["tables", "fenced_code", "toc"], output_format="html5")
        body = body.replace("<table>", '<div class="tablewrap"><table>').replace("</table>", "</table></div>")
        body = doc.inline_images(body)
        body = rewrite_links(body, src.relative_to(ROOT))
        body = command_anchors(body)
        index.extend(index_entries(title, rel, body))
        out = OUT / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(
            doc.HEAD.format(title=title) + "<style>" + doc.CSS + NAV_CSS + "</style>\n" + nav_html(rel) + "<main>\n" + body + "\n</main>\n",
            encoding="utf-8",
        )
        print(f"wrote {out.relative_to(OUT)} ({out.stat().st_size} bytes)")
    import json

    (OUT / "search-index.js").write_text("window.SATZ_INDEX=" + json.dumps(index, ensure_ascii=False) + ";\n" + SEARCH_JS, encoding="utf-8")
    print(f"wrote search-index.js ({(OUT / 'search-index.js').stat().st_size} bytes, {len(index)} entries)")
    (OUT / ".nojekyll").write_text("")


if __name__ == "__main__":
    main()
