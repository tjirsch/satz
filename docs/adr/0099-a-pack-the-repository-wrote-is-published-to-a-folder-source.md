# 0099 — a pack the repository wrote is published to a folder source

- **Status:** accepted
- **Date:** 2026-11-20

## Context

A workload pack is developed where it is used: a file in an estate's `packs_dir` with no
row in `origin.toml`, the repository's own ([ADR 0083](0083-a-workload-pack-is-fetched-one-at-a-time-and-its-origin-is-recorded.md)),
`use`d by the estate and transpiled and planned with it. Once it works, other estates are
to fetch it, which they do from a pack source
([ADR 0097](0097-pack-sources-live-in-config-toml.md)). The way there was a hand copy into
the source's folder, after which the estate that wrote the pack held a file `update-pack`
does not look at: a later fix in the source never reached it, and an edit here never
showed as one.

## Options

**Which sources a command writes**

1. **Folder sources only.** The copy is a file write into a folder `config.toml` names.
   Cost: a GitHub source stays a manual pull request.
2. **Folder and GitHub sources.** Cost: satz would push a branch and open a pull request
   with the machine's sign-in — a write to someone else's repository, a review it cannot
   wait for, and a row whose commit does not exist until the pull request merges.

**What the pack must clear before it is copied**

1. **`satz fmt --check` and the estate's transpile.** Cost: a pack that reads private data,
   declares a membership or writes a type without a prerequisite row is shared as it is.
2. **`review-pack`'s bar inside the estate (`--against`).** It holds the two checks above —
   formatted, and the estate transpiles with it and writes resources from it — and the rest
   of the bar a workload pack is judged by. Cost: a pack whose header says nothing the
   index can print, or that writes a type the prerequisite table lacks, is refused until
   that is fixed.
3. **Nothing.** Cost: a pack that does not transpile reaches every estate that fetches it.

**What the estate that wrote the pack holds afterwards**

1. **A fetched pack: its row is written, as `get-pack --source` writes it.** `update-pack`
   compares it with the source from then on; an edit here is reported. Cost: the next
   change is made in the source's folder, or here after `detach-pack` and published again.
2. **Still its own pack.** Cost: the estate that wrote the pack is the one estate whose
   copy no command keeps current.

**A file of that id already in the source's folder**

1. **Identical: nothing copied; different: refused unless `--force`.** Cost: one flag.
2. **Always overwritten.** Cost: a pack of the same id that another estate published is
   replaced unseen.

## Decision

Option 1 for the sources, option 2 for the check, option 1 for what the estate holds and
option 1 for an existing file: `satz publish-pack <id> --source <name>` (MCP
`satz_publish_pack`, class `write`). The name joins `get-pack`, `update-pack` and
`detach-pack`; "publish" says that the pack leaves the repository for others, which `put`
or `push` do not say without a git reading.

## Consequences

- A GitHub source is refused by name, with the pull request as the way to write it.
- `review-pack <file> --against <estate>` is what a pack is published with; its refusal
  lists the errors and names that command.
- After a publish, `satz packs` reports the pack `current` against its source and
  `update-pack` follows it; the source's folder carries the file at the path
  `get-pack --source` reads.
- A pack fetched from somewhere is published only after `detach-pack`, so a row never
  names two sources.
