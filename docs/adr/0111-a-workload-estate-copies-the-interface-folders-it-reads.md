# 0111 — a workload estate copies the interface folders it reads

- **Status:** accepted
- **Date:** 2026-11-28
- **Builds on:** [0092](0092-the-interfaces-folder-has-an-outgoing-and-an-incoming-half.md)

## Context

The organization's estate writes one folder per interface under `interfaces/outgoing/`, whole,
on every transpile; a workload estate reads copies of them under `interfaces/incoming/`, where
satz never writes (ADR 0092). The copy was a file copy by hand, after every change of the
organization's estate. A hand copy goes wrong in three ways: a file the new folder no longer
has stays in the old one; a folder the estate reads is forgotten, or one it does not read is
taken; and a copy the workload estate no longer transpiles with is found at the next
transpile, after the old folder is gone. `satz init --from` copies folders once, when it
writes the estate, and not again.

## Options

**Who runs the copy**

1. **The workload estate pulls**: `satz copy-interfaces <workload estate> --from <the
   organization's estate>`, run in the workload repository. The workload estate's `use` lines
   say which folders it needs, its config says where they go, and its transpile is the check.
   Costs: the workload repository needs a checkout of the organization's repository, or the
   folders, on its disk.
2. **The organization's estate pushes**: a command run in the organization's repository that
   writes into each workload repository. It would have to know every workload repository and
   which folders each reads, and it would write into another repository's files.
3. **Point `incoming/` at `outgoing/`** (a link or a path in the config). Considered and not
   built in ADR 0092: the workload estate would read a folder that changes under it, and a
   repository on another disk has nothing to point at.

**Which folders**

1. **Exactly the ones the estate reads**: every `use` path under `interfaces/incoming/`, in
   the estate file and in each file it uses (a template the estate uses, as `shop-dev` uses
   `simple-project.satz`), whether the folder is there yet or not. A folder the organization
   does not publish is refused, naming it and the folders there are.
2. **Every outgoing folder.** Fills `incoming/` with folders the estate does not read, among
   them other workloads' delegations.
3. **Folders named on the command line.** A repetition of the `use` lines, and a way to forget
   one.

**How a folder is written**

1. **Whole**: the new folder replaces the old, a file it lacks is removed, OS and editor files
   (`.DS_Store`, `Thumbs.db`, `*~`, `.*.swp`) are left out. The folder is the organization's,
   as `outgoing/` is: nothing in it is the workload's to keep.
2. **Merge**: files are added and overwritten, none removed. A removed interface or a renamed
   file would survive in the copy.

**When the estate is checked**

1. **Before anything is written**: the estate is transpiled in memory with each folder it
   reads taken from the organization's `outgoing/` (the loader reads the path under
   `incoming/<folder>/` at `outgoing/<folder>/`). A check that fails writes nothing. The
   folders are then written beside their targets and swapped in; a failure before the last
   swap puts every old folder back.
2. **After the copy**: simpler, but a failing check leaves the estate with folders it does not
   transpile with, and the old ones gone.

**Over MCP**

1. **No tool.** The command reads a second repository, outside the estate a session opens, and
   the interface plane has no tools (`interfaces`, `check-request`).
2. **A write tool.** The session's root would have to reach the organization's repository.

## Decision

Pull, exactly the folders the estate reads, whole, checked before anything is written, no MCP
tool. `--from` takes the organization's estate folder, its `config.toml` or its estate file
(whose config is the nearest `config.toml` at or above it); `interfaces/outgoing/` is where
that config's `interfaces_dir` says. A folder under `incoming/` the estate does not read is
reported and left. `--report-only` reports the changes and the check and writes nothing;
`--format json` is the same report as data. The report names per folder the files added,
removed and changed, and for each interface file that changed the values that moved, in the
words of the `CHANGES.md` a transpile writes.

## Consequences

- The copy is one command in the workload repository after every change of the organization's
  estate; the hand copy stays the way for a workload repository that has no checkout of the
  organization's repository on its disk.
- A workload estate never holds a folder it does not transpile with because of the copy; the
  check is the transpile's, so a warning stays a warning.
- A copy-interfaces run that is interrupted leaves `.<folder>.satz-copy` or
  `.<folder>.satz-old` beside the folder; the next run refuses until it is removed.
- `satz init --from` keeps its own copy, made once when it writes the estate.
