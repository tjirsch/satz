# 0083 — A workload pack is fetched one at a time, and its origin is recorded

- **Status:** accepted
- **Date:** 2026-11-11

## Context

An estate beside the organisation — a workload team's, running as its own account in its
own repository — builds its workload from packs: ones the team wrote, and ones somebody
else published. The preset machinery does not serve it.

- **It moves a whole library.** `get-presets` fetches the `presets/` tree, and what makes
  that tree a library is keyed on `presets_dir`: the estate map's `offers`, the pack
  graph, the catalogs, the changelog, the library groups. A workload pack has none of
  these, and a workload's estate has no use for the organisation's packs.
- **It records no origin.** A file under `presets_dir` is upstream's when upstream holds a
  file at the same path, and "behind" is told from "edited" by the version line. That
  holds for a folder satz owns whole. In a folder that also holds a team's own packs it
  misreads one of them the day upstream publishes a pack of the same name.
- **It forks and repoints.** `merge-presets` turns an edited pack into `X.local.satz` and
  rewrites the `use` lines of the one estate beside the config, proved by an identical
  transpile. One folder of packs shared by several estates — one per customer of a
  service — has no single estate to rewrite.

## Options

**Where a workload's packs live**

1. In `presets_dir`, beside the library. One folder. Cost: every library command walks
   it, and a team's file is one name away from being judged as upstream's.
2. In a folder of their own, `packs_dir`. Cost: a second folder and a second set of
   commands.

**What decides whose a pack is**

1. The path: a pack is fetched when the source holds one of that id. No state. Cost: a
   name decides ownership, and an edit cannot be told from an old version.
2. A record written at fetch time: the id, the source, the version and the SHA-256 of the
   file as fetched. Cost: one more committed file, which must agree with the folder.

**What an update does with a pack edited since it was fetched**

1. Fork it and repoint the estate, as `merge-presets` does. Cost: it needs the estates
   that use the folder, all of them, and a rollback across all of them.
2. Leave it as it is and say so, with its difference to the source's file. Cost: nothing
   merges; the operator chooses between the source's file and their own.

## Decision

**A workload's packs live in `packs_dir`, the origin record decides whose a pack is, and
an update never writes a pack that was edited or that has no row.**

- `packs_dir` is a config key, `packs` beside `config.toml` by default. `use` resolves as
  before, through `include_dirs`, and `get-pack` refuses a `packs_dir` no include folder
  reaches.
- A workload pack is one `.satz` file with a `pack <id> version "…"` header. Its id is
  its path under the source's `packs/` tree without the extension.
- `packs_dir/origin.toml` holds one row per fetched pack. It is parsed strictly: an
  unknown key, a repeated id, and a row whose file is gone are errors that name the file.
- The source is the `packs/` tree of the satz repository on `main`, or a folder,
  `--from <dir>`. A row records the kind of source and never a folder's path, which is
  one machine's. A command reads one source and looks at the rows of that kind.
- `get-pack <id>` fetches one pack and writes its row; `--all` fetches every pack the
  source holds that `packs_dir` lacks. A pack `packs_dir` holds is never written by it.
- `update-pack <id>` overwrites a pack whose file is as fetched when the source holds
  another, leaves an edited pack as it is and exits 1 with its difference, and reports a
  pack the source holds no longer. `--take-upstream` overwrites an edited pack;
  `detach-pack <id>` removes a row, and the pack is the repository's own from then on.
- `packs` lists both kinds with their state and what the source holds beyond them;
  `review-pack` is `review-preset` without the changelog row and the adoption rule.

## Consequences

- A pack the repository wrote is safe from every command by construction: no command
  writes a pack without a row.
- One `packs_dir` serves any number of estates, because no command rewrites an estate.
- An edit to a fetched pack is resolved by hand. Nothing forks, nothing carries a diff
  file, and the suffixes `.local.satz` and `.diff.satz` mean nothing in `packs_dir`.
- A pack is one file: a script an `action` of the pack runs is not fetched with it.
- The source is a branch, so what a fetch returns can change between two runs. The row's
  hash says exactly what was fetched; pinning a fetch to a release and verifying who
  published a pack are not part of this record.
- The library's own tooling — the pack graph, `doc-presets`, the compliance walk over
  library claims — does not see `packs_dir`.
