# 0008 — the site renders markdown with GitHub's own parser

- **Status:** proposed
- **Date:** 2026-09-11
- **Deciders:** the maintainer

## Context

The documentation is one text with two renderings: GitHub shows the markdown, and
<https://tjirsch.github.io/satz/> is the same text rendered by `scripts/build-site.py`.
The site renders with Python-Markdown (3.10), which is not CommonMark and not GitHub
Flavored Markdown. The docs are written for GitHub, where they are also read, so every
rule where the two parsers disagree is a place where the site shows something the
author never saw.

To measure the gap, every published page (39) was rendered with cmark-gfm, the parser
GitHub runs, and compared with the site, as of 2026-09-11.

- **Lists that follow a line of text.** CommonMark lets a list interrupt a paragraph;
  Python-Markdown needs a blank line first. 29 lists in the README start right under a
  line such as `**Parameters:**`. On the site each of them is a run-on paragraph with
  literal dashes, which covers most of the CLI reference on the front page. The README
  has 155 list items on GitHub and 30 on the site. Three pack pages lose their lists
  the same way.
- **Nested lists** indented two spaces nest on GitHub and flatten on the site: 4 in the
  README.
- **A fenced block inside a list item or a quote** renders as inline code with a stray
  `bash` on the site. That affects four blocks: the README installer note and three in
  `docs/workflows.md`. Python-Markdown's fences run before block parsing, so they never
  see an indented or quoted fence.
- **Heading anchors.** 94 of 498 headings get a different id on the site than on
  GitHub. For example, the site collapses `—` and backticks where GitHub keeps a double
  hyphen. There are 47 `#fragment` links into published pages. One works only on
  GitHub, and seven work only on the site: five were written against the site's slugs,
  and two point at the site's generated `#cmd-` anchors.
- **Smaller.** Bare URLs and addresses that GitHub links stay plain text. And one
  paragraph of the library page broke on GitHub but not on the site: a line began
  with `<param>`, and `param` is one of the tag names that CommonMark reads as the
  start of a raw HTML block. The site hid a doc bug that GitHub showed. It is fixed
  in the same change as this record.

## Options

### A. Keep Python-Markdown and write for it

Add blank lines before the 29 lists, indent nested lists four spaces, move the four
fences out of their lists, give the `toc` extension a GitHub-like `slugify`, and add a
gate for each rule.

- **Pro:** no new dependency, and the builder's HTML post-processing stays as it is.
- **Con:** authors write for GitHub, and nothing stops the next list written the GitHub
  way. Each rule needs its own gate, and a gate for "a list follows a paragraph" is a
  markdown parser of its own. Nested fences stay impossible without
  `pymdownx.superfences`, which is a second dependency that still wants four-space
  indentation.

### B. markdown-it-py (4.2.0) with mdit-py-plugins (0.6.1)

markdown-it-py is CommonMark-compliant: lists interrupt paragraphs, nesting follows the
marker width, and fences work anywhere. Its GFM-like preset adds tables and
strikethrough, and the anchors plugin takes a custom slug function.

- **Pro:** pure Python, current releases, and a plugin ecosystem.
- **Con:** it is not GitHub's parser, so edge cases can still differ. Autolinks need
  `linkify-it-py`, and GitHub's tag filter and some autolink rules are its own.

### C. cmarkgfm (2025.10.22)

cmarkgfm is the Python binding to cmark-gfm, the parser GitHub runs. It supports tables,
autolinks, strikethrough and task lists, and it parses like GitHub by construction.

- **Pro:** the site parses what GitHub parses, so the two renderings agree without a
  rule list. It needs no content edits: the 29 lists, the nested lists and the four
  fences render correctly as written.
- **Con:** it is a C extension. It ships wheels for macOS arm64, Linux x86_64 and
  aarch64, and Windows; anything else builds from source. It releases roughly
  quarterly. It emits no heading ids, so the builder needs GitHub's slugger, about
  fifteen lines, which it needs anyway for the anchors to agree.

## Decision

Proposed: **C**. The requirement is that the site shows the text GitHub shows, and
running GitHub's parser is the only option where that holds by construction rather than
by a growing list of rules and gates.

## Consequences, if accepted

- **The builder's post-processing follows the new HTML.** `inline_images` matches
  `<img alt=… src=…>`, and cmark-gfm writes `src` first. Fences come out as `<pre
  lang="…">`. `toc_html`, `index_entries` and `command_anchors` read heading ids, which
  the new slugger supplies. The `\|` unescape in `code_breaks` becomes dead code, since
  GFM unescapes it itself, and is deleted.
- **94 heading anchors change** to GitHub's. Outside links to those headings, such as
  bookmarks, break once. The builder's `#cmd-` anchors, which `satz <cmd> --html-help`
  opens, are unaffected. The seven links written against the site's slugs are
  rewritten, after which all 47 resolve in both renderings.
- **The build command changes.** It moves from `uv run --with markdown` to cmarkgfm in
  `pages.yml`, `scripts/smoke.sh`, `CLAUDE.md` and `docs/housekeeping.md`. The
  alternative is PEP 723 inline metadata in the two scripts, so that a bare `uv run
  scripts/build-site.py` resolves its own dependency.
- **Every page's HTML changes.** The switch is reviewed with a structural before/after
  comparison of all pages, the same one that produced the numbers above.
- **Unaffected:** the content, the site's look, the search, the privacy gate and the
  published URLs.
