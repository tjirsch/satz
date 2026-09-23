# 0063 — an import refuses rather than write a value the source does not carry

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

`satz import` turns a discovered `Config` into an estate. Two of the values it
writes are not attributes of any one resource: the organisation number, which a
folder's parent (`organizations/{customer_organization_id}`) and an organisation
grant's `org_id` are written from, and a grant's scope, which its import id
(`<scope> <role> <member>`) begins with.

Both are read off the source. The state shape reads them out of the state file,
the live shape out of the sweep's root and the assets' ancestors. A state file
can carry neither: a state of folders and projects whose top folder hangs under a
folder outside the state names no organisation anywhere, and a `*_iam_member`
whose scope attribute is empty names no scope.

Measured on a live organisation with v0.77.0: a `tofu show -json` state carried
no organisation id. The import printed

```
warning: no organization id found among the discovered resources — add `customer_organization_id` to `params` by hand
```

and wrote the estate anyway. That estate transpiled to `parent = "organizations/"`
on a `google_folder` and `org_id = ""` on a `google_organization_iam_member` —
values the provider cannot use, in a file that looks finished. The same run wrote
an organisation grant whose import id began with a space.

Two of those three holes were not holes at all: the organisation WAS in the state,
as the top-level folder's parent, and the nesting pass dropped the attribute
(correctly — the top level is that parent) without keeping what it said.

## Decision

**An import writes an estate only when it can write every value the estate is
made of, and says what is missing when it cannot.**

1. **The organisation is taken from the source first.** A resource that names it
   (`organizations/<n>`, `org_id`) or a top-level folder's parent
   (`link_folders_to_parents`, `src/discovery.rs`, which now returns it instead of
   dropping it with the attribute). Two organisations among the top-level folders
   end the import naming both: one source is one organisation.
2. **A source that names none refuses the write** (`MISSING_ORGANIZATION`,
   `src/import.rs`). Nothing is written — `discovered_to_satz` fails before the
   file is touched — and the message names the param, what is written from it and
   how to supply it.
3. **`--organization <n>` supplies it, on the state shape only.** A live sweep
   reads the organisation from the root it is given and from the assets'
   ancestors, so the flag is refused there rather than accepted as a second
   source of truth. Where a state names one of its own and the flag says another,
   the import is refused naming both.
4. **A grant whose scope the source does not carry is refused, naming the
   resource and the attributes that carry the scope** — the shape
   `add_resource_to_project` already used for a pinned grant, now used by the
   folder-scoped and organisation-scoped paths too.

## Options

**Keep the warning and write the estate.** *Rejected.* It is the project's
fail-fast rule inverted: a broken value is written as though it were data, in a
file whose header says to review it and whose every other line is right. The
operator learns of it from `tofu plan`, two commands later, or from an apply. A
warning that is followed by writing the file is a warning nobody has to read.

**Write the param with an empty value and let `satz transpile` refuse.** *Rejected.*
It moves the refusal one command later for no gain and leaves an estate on disk
that no command accepts. The import knows what is missing; the transpile only
knows that a param is empty.

**Ask for the organisation interactively.** *Rejected.* `satz import` is run in
scripts and from the MCP server, where there is nobody to ask. A flag is the same
answer, repeatable.

**Read the organisation from the import config's `root:`.** *Rejected.* That key
is the live sweep's root — what to sweep, not what a file already swept belongs
to. Overloading it makes one key mean two things depending on the shape, and a
state import that reads a `root:` meant for a live sweep of another scope would
bind the wrong number silently.

**Take the flag on every shape.** *Rejected.* The live shape already knows the
answer, and the flag would either be ignored where it disagrees — the silent
conflict this ADR exists to remove — or need a reconciliation rule for a fact the
sweep can read for itself.

## Consequences

- A state file satz imported before can be refused now. The remedy is one flag,
  and the refusal names it; `presets/README.md`'s `## Breaking changes` carries
  the entry.
- A state of folders and projects under an organisation imports without the flag,
  where it used to warn: the folder's parent names the organisation and is now
  read.
- An estate written by an earlier import is not reread and not repaired. What it
  holds is named in the breaking-changes entry, for one reading by hand.
- The refusal is the import's, not the emitter's: `satz transpile` still emits
  what an estate declares, and an estate that binds `customer_organization_id` to
  an empty string by hand is that estate's own statement.
