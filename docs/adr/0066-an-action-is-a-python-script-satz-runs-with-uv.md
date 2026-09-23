# 0066 — an action is a Python script, and satz runs it with uv

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

An `action` is the one deployment step satz does not compile: a script bound to an
estate's params, printed by `satz run-actions` and spawned by `--check` or
`--execute` (`src/actions.rs`). Until now satz spawned the file as a program and the
shebang decided the interpreter, which makes a shell script the obvious thing to
write — the library's own action, `presets/scc/scc-enable-all.sh`, is one.

satz ships a Windows build. Windows runs an executable, not a script, so a `.sh`
action is refused there before the spawn, with a message naming the `bash …` to run
it by hand from Git Bash or WSL. An operator on Windows therefore cannot run an
action at all, and the packs that will bind more of them — service enablement,
per-project settings, anything the provider has no resource for — would ship a step
that half the supported platforms cannot take.

The repository already has a Python toolchain: every script under `scripts/` is run
with `uv run`, and the ones with dependencies declare them in PEP 723 inline
metadata (`scripts/build-site.py` is the pattern). The smoke matrix installs uv.

## Decision

**A pack's action is written as a Python script, unless Python would be notably
more complex than a shell script for that job. satz launches an action by its
file extension: a `.py` file is spawned as `uv run --script <file> <args>`, and
anything else is spawned as a program, exactly as before.**

The extension is the whole mechanism. There is no `interpreter` key, no per-OS
variant and no new syntax — `run = "seed-settings.py"` is the declaration.

`--script` is part of the decision: it runs the file as a standalone script with
its own PEP 723 dependencies, so a `pyproject.toml` that happens to sit in the
estate root — the working directory of every action — has no say in what the script
imports.

Two consequences, both tested (`src/actions.rs`, `scripts/smoke.sh`):

- **`uv` is required for a `.py` action**, looked for on PATH with every other
  check before the first action is spawned, and its absence refuses the run naming
  the script. satz never falls back to a `python` or `python3` on PATH: that is a
  different interpreter with a different set of packages, so the script that was
  tested would not be the script that ran.
- **A `.py` action needs no executable bit.** uv reads the file, so a Python action
  a pack ships runs as `get-presets` downloaded it, without the `chmod +x` a
  directly spawned script still needs.

The Windows refusal for a directly spawned script stays as it is, with one sentence
added: a `.py` action runs through uv on every platform.

## Options

**Ship every action twice — one `sh`, one PowerShell.** *Rejected.* It doubles the
writing and the testing of every step, and nothing can tell whether the two halves
still agree: they are two programs with one name, and the one that is never run on
the maintainer's machine is the one that rots.

**Spawn `python3` when it is on PATH, uv otherwise.** *Rejected.* Which
interpreter ran, with which packages, would depend on the machine. A step that
changes a customer's organisation is the last place for that, and a fallback like
this fails at the moment the dependency is missing, inside the script, rather than
before anything was spawned.

**Keep the shebang as the mechanism and let a Python action use
`#!/usr/bin/env -S uv run --script`.** *Rejected.* It works on unix only — Windows
reads no shebang — which is the problem being solved. It also needs the executable
bit on a file a pack downloaded.

**An `interpreter` key in the `action` block.** *Rejected.* It is a second place to
say what the file already says, and it would have to be kept in step with the
extension for the error messages to be truthful. The estate gains nothing it cannot
express by naming the file.

## Consequences

An action now depends on a tool that is not satz. `uv` has to be installed wherever
actions run, including CI; the refusal names it, and the install is one command.
That is the price of one script per step instead of two.

`presets/scc/scc-enable-all.sh` stays a shell script for now — converting it is a
separate change, judged against the same rule.
