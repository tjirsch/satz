# 0049 — `fmt` takes paths and resolves no estate name

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

Every estate-consuming command resolves its estate argument the same way (`estate_path`
in `src/main.rs`): an absolute path as given, a relative path that exists in the working
directory as given, and anything else inside the config's `yaml_dir`. `satz transpile
acme.satz`, `satz adopt acme.satz` and `satz packs acme.satz` all find `yaml/acme.satz`.

`satz fmt` does not. It takes paths, so `satz fmt yaml/acme.satz` works and `satz fmt
acme.satz` fails — with `fmt: acme.satz: no such file or directory`, which says the file
is missing rather than that this command looks somewhere else than the last one did. The
reader is left to work out that the rule they just used does not hold here.

`fmt` is also not shaped like the other estate commands. It takes SEVERAL paths and
DIRECTORIES (`satz fmt presets tests`), it formats the whole tree — packs, library files,
corpus fixtures, most of which lie outside `yaml_dir` — and it rewrites in place.

## Decision

**`fmt` keeps taking paths. The refusal says so, and names the path that works.**

```
fmt: acme.satz: no such file or directory. fmt takes paths, and resolves no estate name
inside ./yaml — ./yaml/acme.satz is there: name it, or a directory to walk
```

The second clause is printed only when the name does exist inside `yaml_dir`; a name
nothing holds says what `fmt` takes and stops. Nothing is resolved on the reader's
behalf.

## Options

**Teach `fmt` the same resolution.** *Rejected.* One rule for every command reads well
until the argument is a directory or a list: `satz fmt presets tests` is two directories,
and "resolve a bare name inside `yaml_dir`" has no meaning for either. It would apply to
the one shape that is a missing relative file — so the rule would hold for some arguments
of this command and not others, which is a subtler inconsistency than the one it fixes.
It also silently redirects a command that WRITES: `satz fmt acme.satz`, with no
`./acme.satz` present, would rewrite `yaml/acme.satz` although the operator named neither
that directory nor that file. For a command that reads, a resolved name is a
convenience; for one that rewrites files in place, the file touched has to be the file
named. And `fmt` runs where there is no `config.toml` at all — it is in the arm of
`run()` that proceeds without one — so the directory it would resolve into is a default
guess rather than the estate's own configuration.

**Leave the error as it is.** *Rejected.* It is true and useless: the file is indeed not
there, and the reader has just used a command where a bare name was enough.

## Consequences

- `satz fmt <estate>.satz` stays a refusal, and the refusal is now the documentation:
  what `fmt` takes, and the path to type.
- The inconsistency stays, deliberately, and this record is why it is not re-proposed.
- `run_fmt` takes `yaml_dir` so it can say where it did not look. That is the whole use
  it makes of the configuration.
- A bare name that exists neither in the working directory nor in `yaml_dir` prints only
  what `fmt` takes — there is no path to suggest, and inventing one would be a guess.
