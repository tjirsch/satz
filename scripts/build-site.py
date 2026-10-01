#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["cmarkgfm>=2025.10.22"]
# ///
"""Render the documentation site: every Markdown page the repo keeps, as
self-contained themed HTML, into one output directory (GitHub Pages).

    uv run scripts/build-site.py [_site]

Pages: README.md → index.html, docs/*.md → docs/<name>.html,
presets/README.md → presets/index.html. Links between the Markdown files are
rewritten to their HTML twins;
every page gets the same navigation bar. The Markdown stays the source GitHub
shows — one text, two renderings. Rendering itself is `build-satz-doc.py`.
"""

import posixpath
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib

doc = importlib.import_module("build-satz-doc")

ROOT = Path(__file__).resolve().parent.parent
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "_site"

# Which `docs/*.md` the site publishes, and which it deliberately does not.
#
# A glob used to decide this, which meant a page appeared on the public site
# because a file existed. Publishing is a decision now, and so is not publishing:
# a doc must be named in exactly one of these two, or the build fails naming it.
# That way a new doc cannot slip onto the site unnoticed, and cannot be silently
# left off it either.
SITE_DOCS: list[str] = [
    "language",
    "workflows",
    "interview",
    "mcp",
    "examples",
    "housekeeping",
    "competitive",
    "llms",
]

# Not published, and why. These stay in the repository and stay linkable — a link
# to one from a published page is rewritten to GitHub rather than left dead.
SITE_DOCS_EXCLUDED: dict[str, str] = {
    "security-toolset-integration": "proposal under rework; it describes an audit loop that is not what satz does today",
    "fast-delta": "source material for the competitive matrix, which carries the conclusions",
    "stage-b": "how the pipeline was built. The language reference is how it is used, and the migration commands are in `docs/language.md` §12.3",
}

_docs = {md.stem for md in (ROOT / "docs").glob("*.md")}
_unclassified = sorted(_docs - set(SITE_DOCS) - set(SITE_DOCS_EXCLUDED))
_missing = sorted((set(SITE_DOCS) | set(SITE_DOCS_EXCLUDED)) - _docs)
if _unclassified or _missing:
    lines = ["build-site: every docs/*.md must be published or excluded, explicitly."]
    for stem in _unclassified:
        lines.append(f"  docs/{stem}.md is in neither SITE_DOCS nor SITE_DOCS_EXCLUDED")
    for stem in _missing:
        lines.append(f"  docs/{stem}.md is listed but does not exist")
    raise SystemExit("\n".join(lines))

PAGES: list[tuple[Path, str, str]] = []  # (source md, output relative path, nav label)
PAGES.append((ROOT / "README.md", "index.html", "satz"))
for stem in SITE_DOCS:
    PAGES.append((ROOT / "docs" / f"{stem}.md", f"docs/{stem}.html", stem))
PAGES.append((ROOT / "presets/README.md", "presets/index.html", "library"))
PAGES.append((ROOT / "presets/CHANGELOG.md", "presets/changelog.html", "changelog"))
PACK_PAGES: list[
    tuple[Path, str]
] = []  # derived per-pack pages: rendered, linked from the index, not in the nav
for md in sorted((ROOT / "presets/docs").glob("*.md")):
    PACK_PAGES.append(
        (md, f"presets/docs/{'index' if md.stem == 'README' else md.stem}.html")
    )

# The library's groups, in reading order, with the pack pages in each: the side menu
# of every pack page. `presets/library-groups.txt` is the one list (ADR 0076) and
# `satz doc-packs` is its gate; the check here covers only what this script relies
# on — every pack page is in the menu once, and every menu entry is a page.
LIBRARY_PAGE = "presets/index.html"


def library_groups() -> list[tuple[str, list[str]]]:
    groups: list[tuple[str, list[str]]] = []
    path = ROOT / "presets/library-groups.txt"
    for n, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            groups.append((line[1:-1].strip(), []))
        elif not groups:
            raise SystemExit(f"build-site: {path}:{n}: a pack before the first group")
        else:
            groups[-1][1].append(Path(line).stem)
    return groups


LIBRARY_GROUPS = library_groups()
_menu = [stem for _, stems in LIBRARY_GROUPS for stem in stems]
_pack_stems = {src.stem for src, _ in PACK_PAGES} - {"README"}
if sorted(_menu) != sorted(_pack_stems):
    lines = ["build-site: the library menu names every pack page once."]
    for stem in sorted(_pack_stems - set(_menu)):
        lines.append(
            f"  presets/docs/{stem}.md is in no group of presets/library-groups.txt"
        )
    for stem in sorted(set(_menu) - _pack_stems):
        lines.append(f"  {stem} is in presets/library-groups.txt and has no page")
    for stem in sorted({m for m in _menu if _menu.count(m) > 1}):
        lines.append(f"  {stem} is in two groups")
    raise SystemExit("\n".join(lines))

# The menu, in reading order: what satz is, the language it is written in, the
# library that ships with it and its changelog, how you work with it, the machine
# interfaces, then
# the reference shelf. Every page is named here and nothing else is — an unlisted
# page used to be appended alphabetically, which is how the menu drifted into a
# directory listing. It is a failure now, like an unclassified doc above.
NAV_ORDER = [
    "satz",
    "language",
    "library",
    "changelog",
    "workflows",
    "interview",
    "mcp",
    "examples",
    "housekeeping",
    "competitive",
    "llms",
]

_labels = {label for _, _, label in PAGES}
if _labels != set(NAV_ORDER):
    lines = ["build-site: the navigation names every page, in a decided order."]
    for label in sorted(_labels - set(NAV_ORDER)):
        lines.append(f"  page {label!r} is not in NAV_ORDER")
    for label in sorted(set(NAV_ORDER) - _labels):
        lines.append(f"  NAV_ORDER names {label!r}, which is not a page")
    raise SystemExit("\n".join(lines))


def nav_html(current_rel: str) -> str:
    depth = current_rel.count("/")
    up = "../" * depth
    items = []
    by_label = {label: rel for _, rel, label in PAGES}
    for label in NAV_ORDER:
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


TOC_CSS = """
  /* Long pages are the point — the reference is meant to be read straight
     through — so the answer to navigating them is a contents column beside the
     text, not shorter pages. */
  /* The text column is 74ch plus half of the page's width beyond the contents
     column, the gap and 74ch, so the empty space on each side is half what a fixed
     74ch left (9.25rem is half of the contents column and the gap). */
  .page { display: grid; grid-template-columns: 15.5rem minmax(0, calc(50% + 37ch - 9.25rem)); gap: 0 3rem;
    justify-content: center; align-items: start; }
  .page > main { margin: 0; max-width: none; }
  aside.side { position: sticky; top: 4.4rem; margin: 84px 0 0; font-size: .9rem;
    max-height: calc(100vh - 6rem); overflow-y: auto; overscroll-behavior: contain; }
  aside.side details + details { margin-top: 1.4rem; }
  aside.side summary { font-weight: 600; color: var(--ink-2); cursor: pointer; margin-bottom: .6rem;
    list-style: none; }
  aside.side summary::-webkit-details-marker { display: none; }
  aside.side summary::before { content: "▾ "; color: var(--muted); }
  aside.side details:not([open]) summary::before { content: "▸ "; }
  aside.side nav { display: flex; flex-direction: column; border-left: 1px solid var(--line); }
  aside.side a { color: var(--ink-2); text-decoration: none; line-height: 1.35;
    padding: .18rem 0 .18rem .8rem; border-left: 2px solid transparent; margin-left: -1px; }
  aside.side a:hover { color: var(--accent); }
  aside.side a.active, aside.side a.current { color: var(--accent); border-left-color: var(--accent); }
  aside.side a.lvl3 { padding-left: 1.7rem; font-size: .92em; color: var(--muted); }
  aside.side a.lvl3:hover, aside.side a.lvl3.active { color: var(--accent); }
  /* A group of the library folds: its title is the toggle and its entries show
     when it is open. The script opens the group the reader is in; the others
     show their title alone, so the column stays as short as the group list. */
  aside.side details.grp { margin: 0; }
  aside.side nav > details.grp + details.grp { margin-top: 0; }
  aside.side details.grp > summary { font-size: .74rem; font-weight: 600; letter-spacing: .05em;
    text-transform: uppercase; color: var(--muted); padding: .7rem 0 .2rem .8rem; margin: 0; }
  aside.side details.grp > summary:hover { color: var(--accent); }
  aside.side nav > details.grp:first-child > summary { padding-top: .2rem; }
  aside.side details.grp > nav { border-left: 0; margin-left: .4rem; }
  /* Narrow: the side column becomes collapsed blocks above the text (the script
     closes them on load), so a long list never buries the page it describes. */
  @media (max-width: 1180px) {
    .page { grid-template-columns: minmax(0, calc(50% + 37ch)); }
    aside.side { position: static; max-height: none; overflow: visible; margin: 28px 0 0; }
  }
  @media print { aside.side { display: none; } }
"""

TOC_JS = r"""
/* The side column: closed on narrow screens, and marking the section the
   reader is actually in. No dependencies — the site ships no third-party JS. */
(function () {
  var side = document.querySelector("aside.side");
  if (!side) return;
  var narrow = window.matchMedia("(max-width: 1180px)");
  if (narrow.matches) Array.prototype.forEach.call(side.querySelectorAll("details"), function (d) { d.open = false; });
  var toc = side.querySelector("details.toc");
  if (!toc) return;
  var links = [], heads = [];
  Array.prototype.forEach.call(toc.querySelectorAll("a[href^='#']"), function (a) {
    var el = document.getElementById(decodeURIComponent(a.getAttribute("href").slice(1)));
    if (el) { links.push(a); heads.push(el); }
  });
  if (!heads.length) return;
  var tops = [], active = null, queued = false;
  function measure() {
    tops = heads.map(function (el) { return el.getBoundingClientRect().top + window.scrollY; });
  }
  function update() {
    queued = false;
    /* the heading the reader has most recently passed, allowing for the sticky header */
    var y = window.scrollY + 140, i = 0;
    for (var k = 0; k < tops.length; k++) { if (tops[k] <= y) i = k; else break; }
    var a = links[i];
    if (a === active) return;
    if (active) active.classList.remove("active");
    active = a;
    a.classList.add("active");
    /* the group the reader is in is the one that is open */
    var g = a.closest("details.grp");
    if (g && !g.open) {
      Array.prototype.forEach.call(toc.querySelectorAll("details.grp[open]"), function (d) { d.open = false; });
      g.open = true;
    }
    /* keep the mark visible in a long side column, without scrolling the page:
       only the aside's own scrollTop is touched */
    if (!narrow.matches && toc.open && side.scrollHeight > side.clientHeight) {
      var r = a.getBoundingClientRect(), t = side.getBoundingClientRect();
      if (r.top < t.top) side.scrollTop -= (t.top - r.top) + 8;
      else if (r.bottom > t.bottom) side.scrollTop += (r.bottom - t.bottom) + 8;
    }
  }
  function onScroll() { if (!queued) { queued = true; requestAnimationFrame(update); } }
  measure();
  update();
  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", function () { measure(); update(); }, { passive: true });
  /* fonts and inlined SVGs change the offsets after first paint */
  window.addEventListener("load", function () { measure(); update(); });
})();
"""

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


def index_entries(title: str, rel: str, body: str) -> list[dict]:
    """One entry per page plus one per h1–h3: heading, anchor and a short
    excerpt of the text that follows — what the search box looks through."""
    entries = []
    parts = re.split(r"(<h[123][^>]*>.*?</h[123]>)", body, flags=re.S)
    lead = doc.plain_text(parts[0])[:180]
    entries.append({"t": title, "p": rel, "a": "", "h": title, "x": lead})
    for i in range(1, len(parts), 2):
        m = re.match(
            r"<h([123])[^>]*?id=\"([^\"]+)\"[^>]*>(.*?)</h\1>", parts[i], flags=re.S
        )
        if not m:
            continue
        heading = doc.plain_text(m.group(3))
        follow = doc.plain_text(parts[i + 1] if i + 1 < len(parts) else "")[:180]
        entries.append(
            {"t": title, "p": rel, "a": m.group(2), "h": heading, "x": follow}
        )
    return entries


def command_anchors(body: str) -> str:
    """A heading that names a command in backticks — `### Transpile (`transpile`)` —
    gets a stable `id="cmd-transpile"` so `satz <cmd> --html-help` can open it.
    The rendered heading keeps its own generated id as a nested anchor."""

    def repl(m: "re.Match[str]") -> str:
        level, attrs, slug, inner = m.group(1), m.group(2), m.group(3), m.group(4)
        cmds = re.findall(r"<code>([a-z][a-z0-9-]*)</code>", inner)
        if not cmds:
            return m.group(0)
        keep = f'<a id="{slug}"></a>' if slug else ""
        return f'<h{level}{attrs} id="cmd-{cmds[0]}">{keep}{inner}</h{level}>'

    return re.sub(
        r'<h([23])((?:\s+(?!id=)[a-z-]+="[^"]*")*)(?:\s+id="([^"]*)")?>(.*?\(<code>[a-z][a-z0-9-]*</code>\).*?)</h\1>',
        repl,
        body,
    )


GITHUB_BLOB = "https://github.com/tjirsch/satz/blob/main/"


TOC_MIN_HEADINGS = 3


def toc_html(body: str, grouped: bool = False) -> str:
    """ "On this page" — the h2/h3 headings of the rendered body, as a sidebar.

    Built AFTER `command_anchors` has run, so the hrefs are the ids the document
    actually carries. h4 and deeper are left out: a table of contents that lists
    every paragraph is another long page to navigate. A `grouped` page — the
    library, whose h2 headings are its groups — shows one level more: the h2 as a
    group header, its h3 and h4 as the entries under it.
    """
    deepest = "4" if grouped else "3"
    heads = re.findall(
        rf'<h([2-{deepest}])[^>]*?\bid="([^"]+)"[^>]*>(.*?)</h\1>', body, flags=re.S
    )
    if len(heads) < TOC_MIN_HEADINGS:
        return ""
    items = []
    if grouped:
        # each h2 is a group that folds; its h3/h4 are the entries inside, and the
        # script opens the group the reader is in. An h2 with nothing under it is a
        # plain entry.
        sections: list[tuple[str, str, list[str]]] = []
        cls = {"3": "lvl2", "4": "lvl3"}
        for level, anchor, inner in heads:
            label = html_escape(doc.plain_text(inner))
            if level == "2":
                sections.append((anchor, label, []))
            elif sections:
                sections[-1][2].append(f'<a class="{cls[level]}" href="#{anchor}">{label}</a>')
        for anchor, label, entries in sections:
            if entries:
                items.append(fold(label, entries, False))
            else:
                items.append(f'<a class="lvl2" href="#{anchor}">{label}</a>')
    else:
        cls = {"2": "lvl2", "3": "lvl3"}
        for level, anchor, inner in heads:
            label = html_escape(doc.plain_text(inner))
            items.append(f'<a class="{cls[level]}" href="#{anchor}">{label}</a>')
    return (
        '<details class="toc" open><summary>On this page</summary><nav>'
        + "".join(items)
        + "</nav></details>\n"
    )


def library_html(current_rel: str) -> str:
    """A pack page's library menu: every pack page under its group header, in
    the order of `presets/library-groups.txt`, the current one marked."""
    items = ['<a href="index.html">All packs</a>']
    for title, stems in LIBRARY_GROUPS:
        entries = []
        here = False
        for stem in stems:
            current = current_rel == f"presets/docs/{stem}.html"
            here = here or current
            cls = ' class="current"' if current else ""
            entries.append(f'<a{cls} href="{stem}.html">{stem}</a>')
        items.append(fold(html_escape(title), entries, here))
    return (
        '<details class="lib" open><summary>Library</summary><nav>'
        + "".join(items)
        + "</nav></details>\n"
    )


def fold(title: str, entries: list[str], is_open: bool) -> str:
    """One group of the side column: its title is the toggle, its entries show when
    it is open. Without the script a reader still opens it by hand."""
    state = " open" if is_open else ""
    return (
        f'<details class="grp"{state}><summary>{title}</summary><nav>'
        + "".join(entries)
        + "</nav></details>"
    )


def side_html(body: str, rel: str, is_pack: bool) -> str:
    """The side column: the page's own contents, and on a pack page the library."""
    parts = toc_html(body, grouped=rel == LIBRARY_PAGE)
    if is_pack:
        parts += library_html(rel)
    return f'<aside class="side">\n{parts}</aside>\n' if parts else ""


def html_escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def rewrite_links(body: str, src_rel: Path) -> str:
    """`docs/x.md` / `../README.md` / `#anchor` links → the rendered twins.

    A link to a repository file the site does not publish (an excluded doc, an
    ADR) is sent to GitHub rather than left as a `.md` href that 404s: the file
    is still there, it is simply not a page. A link to a file that does not exist,
    or that leaves the repository, fails the build naming it. Paths resolve
    against the repository, never the working directory: smoke builds from
    `tests/smoke`.
    """
    targets = {str(src.relative_to(ROOT)): rel for src, rel, _ in PAGES}
    targets.update({str(src.relative_to(ROOT)): rel for src, rel in PACK_PAGES})
    broken: list[str] = []

    def repl(m: "re.Match[str]") -> str:
        href = m.group(1)
        if href.startswith(("http://", "https://", "#", "mailto:")):
            return m.group(0)
        path, _, frag = href.partition("#")
        if not path.endswith(".md"):
            return m.group(0)
        anchor = f"#{frag}" if frag else ""
        if path.startswith("/"):
            target = Path(path.lstrip("/"))
        else:
            resolved = (ROOT / src_rel.parent / path).resolve()
            if not resolved.is_relative_to(ROOT.resolve()):
                broken.append(f"  {href} leaves the repository")
                return m.group(0)
            target = resolved.relative_to(ROOT.resolve())
        page = targets.get(str(target))
        if page is None:
            if not (ROOT / target).is_file():
                broken.append(f"  {href} names {target}, which does not exist")
                return m.group(0)
            return f'href="{GITHUB_BLOB}{target.as_posix()}{anchor}"'
        here = Path(targets[str(src_rel)]).parent
        rel = Path(*([".."] * len(here.parts))) / page if here.parts else Path(page)
        return f'href="{rel.as_posix()}{anchor}"'

    body = re.sub(r'href="([^"]+)"', repl, body)
    if broken:
        raise SystemExit(
            f"build-site: {src_rel} links to nothing:\n" + "\n".join(broken)
        )
    return body


def dead_anchors(pages: dict[str, str]) -> list[str]:
    """Every `page.html#anchor` link between pages of the site whose target page
    carries no such id. Heading ids are GitHub's (ADR 0008), so an anchor that
    lands here lands on GitHub too, and one that does not is dead in both."""
    ids = {rel: set(re.findall(r'\bid="([^"]+)"', page)) for rel, page in pages.items()}
    dead = []
    for rel, page in pages.items():
        here = posixpath.dirname(rel)
        for target, frag in re.findall(r'href="([^"#:]*)#([^"]+)"', page):
            to = posixpath.normpath(posixpath.join(here, target)) if target else rel
            if to in ids and frag not in ids[to]:
                dead.append(f"  {rel}: #{frag} is not an anchor of {to}")
    return dead


def main() -> None:
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    index: list[dict] = []
    pages: dict[str, str] = {}
    pack_rels = {r for s, r in PACK_PAGES if s.stem != "README"}
    for src, rel, _label in PAGES + [(s, r, "") for s, r in PACK_PAGES]:
        doc.MD = src  # the renderer inlines SVGs relative to the source
        body = doc.render(src.read_text(encoding="utf-8"))
        title = doc.page_title(body, src.relative_to(ROOT))
        body = rewrite_links(body, src.relative_to(ROOT))
        body = command_anchors(body)
        index.extend(index_entries(title, rel, body))
        pages[rel] = (
            doc.head(title)
            + "<style>"
            + doc.CSS
            + NAV_CSS
            + TOC_CSS
            + "</style>\n"
            + nav_html(rel)
            + '<div class="page">\n'
            + side_html(body, rel, rel in pack_rels)
            + "<main>\n"
            + body
            + "\n</main>\n</div>\n"
        )
    dead = dead_anchors(pages)
    if dead:
        raise SystemExit(
            "build-site: links to anchors that do not exist:\n" + "\n".join(dead)
        )
    for rel, page in pages.items():
        out = OUT / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(page, encoding="utf-8")
        print(f"wrote {out.relative_to(OUT)} ({out.stat().st_size} bytes)")
    import json

    (OUT / "search-index.js").write_text(
        "window.SATZ_INDEX="
        + json.dumps(index, ensure_ascii=False)
        + ";\n"
        + SEARCH_JS
        + TOC_JS,
        encoding="utf-8",
    )
    print(
        f"wrote search-index.js ({(OUT / 'search-index.js').stat().st_size} bytes, {len(index)} entries)"
    )
    (OUT / ".nojekyll").write_text("")


if __name__ == "__main__":
    main()
