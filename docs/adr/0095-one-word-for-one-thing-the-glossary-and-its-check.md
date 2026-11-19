# 0095 — one word for one thing: the glossary and its check

- **Status:** accepted
- **Date:** 2026-11-19

## Context

One thing had several names, and one name stood for several things. The folder an
estate publishes for another was an interface folder in one page, a hand-over folder in
the next and a vendor folder in a third; "hand-over" also named the documents a customer
receives. "Pack" and "component" competed for what a workload is built from, a delegated
resource was a target, a place or a scope, and the service account an estate runs as was
the IaC account, the deployer and the identity. The same text said compile, emit and
generate for what satz does to an estate, deploy and roll out for what OpenTofu does with
the result, and mixed British and American spelling.

A reader who meets two words assumes two things. The texts satz writes — command help,
messages, the README of an interface folder — reach people who never read the docs, and
satz-studio shows them as labels. Every new text copied whichever word its author had
read last.

## Options

1. **A glossary alone.** One page that defines each term and lists the words not used.
   Cost: nothing holds a new text to it; the old words come back with the next change
   written from memory.
2. **A glossary plus a check of the disallowed wordings a pattern can find.**
   `scripts/check-wording.sh` reports every line of the tracked text that uses a word the
   glossary lists as not used, run by the hooks and in CI. Cost: a word with a second,
   legitimate use needs a narrow `allow` line, and a word used for another meaning — the
   third rule — is beyond a pattern and stays with review.
3. **Renaming every name in the format too** — flags, params, export and field names,
   JSON report fields, file names, command names — so that no old word survives anywhere.
   Cost: every estate, every interface folder already copied, every satz-studio release
   and every script that reads a report breaks at once, for words a reader of the prose
   does not see.

## Decision

- **Option 2.** `docs/glossary.md` defines the terms of satz, its documentation,
  satz-studio and the texts satz writes. Three rules hold for prose, command help,
  messages, labels and comments:
  1. a term defined there is used for its thing;
  2. where an entry allows a second word, one of the two is used — no third;
  3. a word defined there is not used for any other meaning.
- **American spelling** throughout.
- **The definitions are cloud-neutral.** A definition names a cloud only where the thing
  is that cloud's own.
- **Names in the format keep their words.** A rename of a name in the format is a
  language or report change of its own, with its own record; the glossary lists the names
  that stay.
- **`scripts/check-wording.sh`** reads `scripts/check-wording.txt` — one pattern per
  wording the glossary's table "Words not used" lists, and `allow` lines for a second
  use a pattern cannot tell apart — over Markdown, Satz, Rust (comments and string
  literals of more than one word), shell, Python and TOML. The pre-commit hook runs it on
  the staged files and `.github/workflows/names-gate.yml` on the whole tree.

## Consequences

- A new text is held to the glossary before it lands; a new term starts as a glossary
  entry and, where an old word competes, a pattern.
- Rule 3 is review's: whether "project" means a cloud project or a workload estate is a
  question of meaning, not of spelling.
- Older decision records and the changelogs keep the words of their day; the check does
  not read them.
- The German-language skill files follow German usage and are not read.
