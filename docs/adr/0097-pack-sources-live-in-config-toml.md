# 0097 — pack sources live in config.toml

- **Status:** accepted
- **Date:** 2026-11-20

## Context

A workload pack came from one of two places ([ADR 0083](0083-a-workload-pack-is-fetched-one-at-a-time-and-its-origin-is-recorded.md)):
the `packs/` tree of the satz repository, which was written into the binary, or a folder
typed on each run as `--from <dir>`. The origin record wrote `github:tjirsch/satz` or
`dir` — the kind of source, never the folder, because the path was one machine's. So a
row could not be followed: a colleague who cloned the repository saw `source = "dir"` and
had no way to learn which folder, and a pack from any repository other than satz's could
not be fetched at all. A team that publishes its packs in a repository of its own, private
or public, had no way to name it.

The preset library had the same limit: `get-presets`, `merge-presets` and `check-presets`
read the satz repository, or `--pristine-dir` on each run. An organization that keeps its
own library — satz's presets with its own changes, maintained in a repository of its own —
had to pass the folder on every command.

The formats chosen here are the ones satz 1.0 keeps.

## Options

**Where the named sources are kept**

1. **In the estate's `config.toml`.** The sources travel with the repository: every clone
   reads the same names, and a row of the origin record names a source the repository
   itself defines. Cost: the address of a private repository is visible to everyone who
   can read the estate's repository; a folder is written relative to `config.toml`, so it
   is the same for every clone only when the clones are laid out alike.
2. **In this machine's settings, `~/.config/satz/satz.toml`.** A private address stays on
   the machine. Cost: a row names a source that only the machine that fetched the pack
   knows, which is the problem this record exists to solve; every colleague configures
   the same sources by hand.
3. **Both, the machine's overriding the estate's.** Cost: two places to look, and a row
   whose meaning depends on the machine reading it.

**What replaces the preset library**

1. **One replacement library**, `[preset_library]`, read instead of satz's. Cost: an
   organization that wants satz's packs and its own keeps them together in its library.
2. **Several libraries, mixed per pack.** Cost: two libraries may hold a pack of the same
   path, and the library's own machinery — the estate map, the pack graph, the changelog,
   the catalogs — is one per library; merging graphs is a design of its own.

## Decision

Option 1 for the sources and option 1 for the library.

- **`[[pack_source]]` tables in `config.toml`**: a `name` (`[a-z][a-z0-9-]*`, used once,
  not `satz`) and exactly one of `github = "owner/repository"` — with optional `ref`, a
  branch or tag, and `path`, the folder that holds the packs, `packs` by default — and
  `dir`, a folder relative to `config.toml`. The satz repository's `packs/` on `main` is
  the source `satz`, never listed. A table that breaks a rule stops every command, naming
  the table and what is wrong. `satz pack-source add|list|remove` edits them, keeping the
  rest of the file; no other configuration file holds a source.
- **The commands take `--source <name>`** where they took `--from <dir>`, which is removed.
  `get-pack` reads `satz` without it; `packs` reads every source, or the one named, and
  says per row which source; `update-pack` reads, for each pack, the source its row names.
- **A row of `origin.toml` names its source and where that source was**: for a GitHub
  source the repository (`github`, for `satz` `tjirsch/satz`), the folder of it where that
  is not `packs` (`path`) and the commit the file was read at; for a folder source the
  folder as `config.toml` writes it (`dir`). The row can be followed even if the source's
  definition changes later, and a change is never silent: a row whose source `config.toml`
  does not name is refused with `satz pack-source add`; a row whose repository, folder of
  the repository or folder differs from what `config.toml` now names for its source is
  refused with the two edits that resolve it — write the new place in the row when it holds
  the same packs, or add the old place as a source of its own; the earlier values
  `github:…` and `dir` are refused with the edit that turns them into the new form.
- **A repository `config.toml` names is read with this machine's GitHub sign-in**: the
  token `gh auth token` prints, or `GITHUB_TOKEN` where `gh` has none, sent only to
  GitHub's API. A private repository that answers 404 or 403 without one is refused with
  `gh auth login`. satz stores no token. The satz repository is read without it.
- **One `[preset_library]`**, `github` with optional `ref`, or `dir`; the library is the
  `presets/` of either. All three preset commands read it instead of satz's;
  `--pristine-dir` overrides it for one run. Libraries are not mixed.

## Consequences

- A row of `origin.toml` is followed by every clone of the repository; `update-pack`
  needs no flag. Pointing a source at another repository or folder is an edit of every row
  that names it, made by hand.
- The address of a private pack or library repository is visible to whoever reads the
  estate's repository. Reading the files still needs an account that GitHub lets read it.
- Private reads depend on `gh` being signed in on the machine — the same account the
  operator already uses for the repository — or on `GITHUB_TOKEN`.
- A GitHub source costs one API request per run for its commit and one for a listing.
  The satz repository's preset download costs two where it cost one.
- An existing `origin.toml` is edited once: `source = "github:tjirsch/satz"` becomes
  `source = "satz"` with `github = "tjirsch/satz"` and the commit the file was read at, and `source = "dir"` becomes the
  name of a source `config.toml` lists, with its folder (`presets/CHANGELOG.md`, Breaking
  changes, lists the edit).
- A replacement library is compared and merged by its packs' in-file versions and its
  `pack-graph.json`, as satz's is. The repository gates satz runs on its own library —
  `doc-presets --check`, `preset-graph --check`, the test that holds each pack's version
  to the last release tag — belong to the library's repository and run there.
- satz-studio passes `--source` where it passed `--from`, and reads the source names and
  the `sources` of the pack reports.
- A pack the repository wrote is copied into a folder source by `satz publish-pack`, which
  writes its row as `get-pack --source` does ([ADR 0099](0099-a-pack-the-repository-wrote-is-published-to-a-folder-source.md)).
