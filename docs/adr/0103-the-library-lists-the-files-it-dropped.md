# 0103 — the library lists the files it dropped, and merge-presets removes them

- **Status:** accepted
- **Date:** 2026-11-25

## Context

An estate's `presets/` folder is satz's: it holds the preset library's files, the
estate's forks `X.local.satz` and their adoption deltas `X.diff.satz`. When a release
drops a file from the library — `presets/project-workload.satz` and
`presets/folder-workload.satz` went with ADR 0093, the CIS extensions moved with the
`cis/` folder — `merge-presets` installed and updated what the library carries and never
looked at what it no longer carries. The old file stayed in every estate's folder,
beside a commented `use` line for it in the estate file, and a reader took both for
something live.

Removing such a file needs one fact `merge-presets` did not have: whether a file the
library does not carry is one it dropped, or one somebody put into the folder. The
downloaded library says only what it carries today.

## Options

**How satz tells a dropped file from a foreign one**

1. **The library lists what it dropped**, `presets/dropped-files.txt`, one path under
   `presets/` per line, read from the library `merge-presets` merges. Ships with the
   files, so a `[preset_library]` repository or folder states its own; a test against the
   last release tag catches a removal that forgot its line. Costs a list kept by hand,
   and a library folder of somebody else's has to carry it, empty if it never dropped a
   file.
2. **The estate records what `get-presets` installed.** Exact for files installed from
   then on, but every existing estate has no record, and filling one in for them is the
   guess the rule against silent healing forbids.
3. **Every file the library does not carry is dropped.** No list, and a customer's own
   file in the folder is deleted without anyone naming it.
4. **The library's git history.** Exact, and one GitHub request per file or a clone per
   run against an unauthenticated quota of 60 requests an hour; a library folder may
   have no history at all.

**What happens to a dropped pack the estate uses**

1. **Forked like a changed pack**: the content preserved as `X.local.satz`, the estate
   repointed and checked by transpile identity, the pristine copy removed. The estate
   keeps what it creates, and the fork says by its name that it is the estate's own.
2. **Refused until the estate stops using it.** Nothing is written, and the estate keeps
   transpiling a pristine-named copy nothing updates.

## Decision

Option 1 for both. `presets/dropped-files.txt` is the list; `merge-presets` and
`check-presets` refuse a library without it and a line that names a file the library
carries or a path outside the folder. `merge-presets` removes a dropped file the estate
does not use and a folder that leaves empty, forks one it uses, removes the
`X.diff.satz` of a dropped pack and keeps its `X.local.satz`, saying the original is
gone. A file that is none of these — not the library's, a fork, a delta or dropped — is
never deleted: both commands warn about it by path. `check-presets` reports a used
dropped pack as drift in an included pack.

A pack the library moved (the paths in `MOVED_PACKS`) is still refused at its old path
by the transpile, so `merge-presets` cannot run on an estate that uses it; once the
`use` names the new path, the old copy is an unused dropped file and goes.

## Consequences

- A change that removes or moves a file under `presets/` adds its old path to the list
  in the same commit; `cargo test` fails otherwise.
- A `[preset_library]` library or `--pristine-dir` folder without the list is refused by
  `merge-presets` and `check-presets`, naming the file; `get-presets` does not read it.
- The JSON of `check-presets` has no `local-only` status: a file the library does not
  carry is `fork`, `dropped` or `foreign`. The merge report gains the actions
  `dropped-removed`, `dropped-forked` and `fork-kept`, with their counts.
- The commented `use` line of a removed pack stays in the estate file; it names a file
  that is no longer there.
