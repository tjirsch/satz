# 0050 — The privacy shapes are checked twice, and proven equal

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

`scripts/check-names.sh` is this repository's privacy gate. It refuses any token shaped
like private data that is not a documented example value (`docs/examples.md`): directory
ids, 11–13-digit organisation, folder and project numbers, billing accounts, GUIDs and
their dashless form, project ids, e-mail addresses, domains that are neither reserved nor
a known vendor host, repository URLs and checkout paths. CI runs it on every pull request,
the git hooks run it on every commit.

`satz review-pack` judges one pack against the library's bar, and runs where no checkout
of this repository exists — that is what it is for. It checked every rule of the bar but
this one, and said so in a finding. The rule it skipped is the one a pack written outside
the library fails first: a pack written against its author's own organisation carries that
organisation's ids, domains and addresses, and each of them has to become a param before
the pack can go upstream. The author learnt it from the gate of the pull request, which is
the moment `review-pack` exists to come before.

## Decision

**The content rules are implemented twice — the script, and `src/privacy_shapes.rs` in
satz — and one test proves the two equal on one corpus. The allow-lists are one file both
read.**

- The script stays the gate, unchanged in what it accepts. It has to run with no Rust
  toolchain and on a tree that does not compile, which a gate built from satz cannot.
- `src/privacy_shapes.rs` implements the same content rules: the same patterns, the same
  per-token verdicts, the hex-run and GUID stripping of the number rule, ASCII matching as
  the script's `grep -E` under `LC_ALL=C`. It does not implement what only a repository
  has — the committing identity, the files that must never be staged, the local denylist.
- The allow-lists — example values, vendor hosts, vendor-default GUIDs, project
  placeholders — move out of the script into `scripts/check-names-allow.txt`, one entry per
  line as `<list> <ERE>`. The script reads it at start and refuses to run without it; satz
  compiles it in with `include_str!`. Adding an example value is one line in one file.
- `tests/privacy-shapes/` is one fixture corpus: `clean-` files that must pass, `hit-` files
  that must fail, one or more per shape, allowed values beside private ones. A test runs
  the script in `FILE` mode and satz over each and fails on any token one flags and the
  other does not, on a verdict that contradicts the file name, and on a shape no file
  exercises. The fixtures carry their private-looking values split by `%%`, which the test
  removes, so the gate that reads every tracked file finds nothing in them.
- `review-pack` reports each hit as an **error** of a new kind, `private-shape`, at its line,
  naming the token: make it a param, or use the documented example value. It is an error
  rather than a warning because `review-pack`'s verdict is the library's bar, and the bar
  is what a pack goes upstream with; a private pack need not clear it at all.
- The script's directory-id and billing-account rules judged LINES: an allowed value on a
  line hid every other value of that shape on it. They now judge tokens, like every other
  rule and as the script's own documentation says it does. The script's report prints one
  `<file>:<line>: <token>` row per token for every content rule, which is what the test
  reads.

## Options

**Leave the rules in the script only.** *Rejected.* `review-pack` would keep saying it did
not check the rule a pack from outside fails first, and the author would keep learning it
from a pull request.

**Move the rules into satz and make the script call the binary.** *Rejected.* The gate
would need a built satz: a fresh clone, a CI job before the build, a commit on a tree that
does not compile would all lose it. A privacy gate that is skipped when the build is broken
is not a gate.

**Two copies with no proof.** *Rejected.* Two implementations of a regex rule drift the day
someone adds a vendor host to one of them, and the one that drifts is the one nobody runs
on this repository — `review-pack` would pass a pack the gate then refuses.

**Generate one from the other.** *Rejected.* Generating Rust from shell patterns, or shell
from Rust, is a build step and a generator for ten rules that change a few times a year;
the corpus test catches the same drift with no machinery.

## Consequences

- A change to a content rule changes the script, `src/privacy_shapes.rs` and the corpus in
  one change; the test fails on either copy alone.
- The allow-lists have one source. `docs/examples.md` still lists the values for a reader,
  and a value added there is added to `scripts/check-names-allow.txt` in the same commit.
- The Rust copy matches bytes, the script characters in the operator's locale. The two
  can differ on a token directly beside a non-ASCII letter, which the test does not cover:
  it runs the script under `LC_ALL=C`.
- The script checks `--project <id>` in no rule: its pattern finds such a line, but only a
  `projects/…` token is taken out of it. Both copies agree on that; closing it changes
  both.
- The corpus test does not run on Windows, where the checks job has no `bash` the test can
  rely on; the Linux checks job runs it on every pull request.
