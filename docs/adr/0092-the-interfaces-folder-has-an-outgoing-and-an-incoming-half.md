# 0092 — the interfaces folder has an outgoing and an incoming half

- **Status:** accepted
- **Date:** 2026-11-12

## Context

An estate publishes values to the estates beside it as interface folders, and reads the
folders others published to it. The two had different names. What an estate published
was written to `interfaces/`; a copy of what it was given stood wherever its repository
put it, by convention `vendor/` — a programmer's word that says nothing about what the
folder is, for the same folder that is called an interface where it is written.

`interfaces/` could not hold both: satz owned the directory, removed it whole before
every transpile and wrote it again, so that the folder of an interface the estate no
longer declares does not survive. A copy placed there was gone after the next compile.

## Options

1. **Keep two names**, and document `vendor/`. Cost: one thing under two words, and the
   word on the receiving side is jargon.
2. **A second name in the vocabulary of delegated access**, `delegated/`. Cost: an
   interface that delegates nothing — values one estate publishes for a pipeline — does
   not belong under it, so a third folder follows.
3. **One folder with two halves**, `interfaces/outgoing/` and `interfaces/incoming/`.
   satz owns the outgoing half and never writes the incoming one. Cost: every path to a
   published interface gains a segment, in every estate that publishes one and in
   whatever reads it.

## Decision

Option 3.

- `interfaces_dir` (default `interfaces`) holds `outgoing/` and `incoming/` and nothing
  else.
- A transpile removes `outgoing/` and writes it whole: `common/`, and one folder per
  project. It goes when the estate exports nothing, and the directory with it when
  nothing was given either.
- `incoming/` is the estate's: the copies of other estates' `outgoing/<project>/`
  folders, which its `use` lines read. satz reads there and never writes.
- Anything else directly under `interfaces_dir` is refused by name, and nothing is
  removed: a folder an older satz wrote there is to be deleted, a copy of another
  estate's interface moved to `incoming/`. Names that start with a dot are left alone.
- The hand-over is a file copy: the publishing estate's `interfaces/outgoing/<project>/`
  becomes the reading estate's `interfaces/incoming/<project>/`. Pointing one estate's
  incoming half at another's outgoing half was considered and not built.
- The README satz writes into an interface folder names `interfaces/incoming/` as the
  place to take the folder to, and the `use` line that reads it.

## Consequences

- Breaking for an estate that publishes an interface: its `interfaces/` holds folders of
  the old layout, and the next transpile refuses them until they are deleted
  (`presets/CHANGELOG.md`). Whatever reads those folders — another estate's copy, a
  module source in hand-written HCL — follows the new path.
- A reader of a repository sees the direction of every interface folder from its path.
- satz does not check that a `use` line reads from `incoming/`; an estate may still read
  an interface file from any path.
