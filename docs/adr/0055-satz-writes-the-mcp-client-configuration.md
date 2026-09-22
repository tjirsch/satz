# 0055 — satz writes the MCP client configuration, and owns one key of the file

- **Status:** accepted
- **Date:** 2026-09-22
- **Shipped in:** the release that follows

## Context

`satz mcp` is a server an MCP client starts. Starting it needs a small JSON block: the
binary to run, `mcp --root <directory>`, and `--allow <ceiling>`. Until now that block
was a code sample in `docs/mcp.md` that a person copied and filled in by hand.

Three of its values are wrong more often than they are right when a person fills them in:

- **The binary.** `"command": "satz"` is resolved on the client's `PATH`, which is not
  the terminal's. A desktop application on macOS is launched by `launchd` with a minimal
  environment and does not see `~/.local/bin`, so the server never starts and the client
  reports only that it failed.
- **The root.** A relative path resolves against the client's working directory, not the
  one the block was written in. The block has to carry an absolute path, and it has to be
  the directory holding the estate's `config.toml`, its `presets/` and its `schemas/` —
  the boundary `satz mcp` enforces on everything a call opens.
- **The ceiling.** `--allow` defaults to `read`. A block that leaves it out therefore
  works, and nothing in it says whether read-only was a decision or an omission.

A desktop front end (satz-studio) needs the same block, for a button that configures a
client. Building it there would be a second implementation of all three values, in a
codebase that does not parse `--allow` and does not resolve an estate's root — and would
drift the first time a flag moves.

## Decision

**`satz mcp-config <estate>` prints the configuration an MCP client needs, and
`--write` puts it where that client reads it. satz owns exactly one key of that file.**

The command prints — like `prowler`, whose invocation satz composes and never runs. The
block goes to stdout and nothing else does, so it pipes into a file or a clipboard; the
notes go to stderr.

What it decides:

- **`command`** is `std::env::current_exe()`, absolute. The satz that prints the block is
  the satz the client starts. A binary in `target/release` names itself, which is honest:
  it is the binary that ran.
- **`--root`** is the estate's config directory, canonicalised.
- **`--allow`** is always written out, whatever it is, including the default. It is
  parsed by `mcp::Level::parse` and printed by `Level::describe`, so a typo is a refusal
  here exactly as it is at `satz mcp`, and the spelling in the block is the one the
  server prints.

Two clients, two shapes. `--client claude-code` (the default) writes `.mcp.json` — a
`type` of `stdio`, under the key `satz`, one file per project. `--client claude-desktop`
writes the entry that file takes — command and args, no transport key — under
`satz-<estate>`, derived from the estate as the caller named it, because Claude Desktop
keeps every server a person has in one file. `--name` writes another key.

**`--write` merges; it never replaces the file.** satz reads the file, inserts its own
key beside whatever else is there, and writes it back. A satz key already present with
other arguments is printed and refused, and `--force` replaces that key alone. A file
that is not JSON, or whose `mcpServers` is not an object, is refused and left on disk
untouched; `--force` does not cover it.

`IDENTITIES` classifies it `NoGoogleApi`, `MCP_PARITY` does not serve it (below), and
`org_write` classes it with the file-writers.

## Options

**A. Leave it a documentation page.** No code, and it is what exists. It is also what
produced the three failures above, one support question at a time, and a front end still
has to build the block itself.

**B. satz prints it; the client's file stays the person's to edit.** Solves the three
values, no writing at all. It leaves the last, most mechanical step — paste this into
that file without breaking the JSON — to a person, for both clients.

**C. satz prints it, and `--write` replaces the client's file.** Simple to implement and
easy to reason about, and it deletes every other server in a `.mcp.json` or a Claude
Desktop configuration. `--force` would be the only way to add satz to a project that
already has one server, so the escape hatch from the refusal is the data loss.

**D. satz prints it, and `--write` merges satz's own key.** Chosen. The file is a map of
servers and satz owns one entry of it; that is the whole rule, it is the same rule for
both clients, and it makes a second run a no-op. It costs a JSON round-trip of a file
satz did not write: the file comes back pretty-printed with two-space indentation and
each object's keys in sorted order. Every value — commands, arguments, environments,
anything beside `mcpServers` — is carried across unchanged.

**E. satz runs `claude mcp add`.** Shells out to a tool that may not be installed, works
for one client only, and puts satz in the business of driving another program's CLI.

## Consequences

- One place computes the block. satz-studio's button runs this command and renders what
  it prints; it parses no flags and resolves no roots.
- The ceiling is visible in every configuration satz writes. A read-only server says so.
- A client configuration satz wrote names the binary that wrote it. Moving or reinstalling
  satz means running `mcp-config --write --force` again, which is the same as for any
  absolute path in any launcher.
- satz writes into a file another application owns. It is confined to one key and to a
  path the command prints before it writes it, and there is precedent in
  `completion --install`. A file satz cannot parse is never rewritten.
- The Claude Desktop key is derived from the estate's file name. Two estates with the same
  name under two different roots derive the same key; the write refuses on the key that
  is already there, and `--name` is the way past it. satz does not hash the root into the
  key: an unreadable key in a file a person edits by hand is a worse trade than a refusal
  that names the collision.
- No MCP tool serves it. It is what a human runs to reach satz over MCP in the first
  place, and an agent that is already connected has the server it would be asking about.
  It is left out of the `not_served` list the server sends at initialize for the same
  reason `completion` and `open-readme` are: nothing an agent would reach for.
