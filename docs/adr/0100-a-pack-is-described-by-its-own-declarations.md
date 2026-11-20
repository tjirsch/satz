# 0100 — a pack is described by its own declarations

- **Status:** accepted
- **Date:** 2026-11-20

## Context

`satz packs` said of each workload pack what it needs of the estate that uses it: a
delegated folder, a delegated project, or neither
([ADR 0091](0091-budgets-are-delegated-by-a-role-on-the-scope-and-a-packs-needs-are-what-it-reads.md)).
The need was derived from a text scan for three value names, `delegated_folder`,
`project_id` and `project_number`. So the tooling knew two kinds of delegated resource.

The language knows none. Since [ADR 0093](0093-delegation-is-a-base-and-its-targets-and-entries-are-added-by-one-writer.md)
a delegation is a base and one pack per kind of delegated resource, and since
[ADR 0094](0094-a-value-is-read-by-the-name-of-its-interface-folder.md) a value is read by
the name of its interface folder. The delegated resource may be one VM, a bucket or anything
an interface folder exports, and a pack that works on it reads that folder's values. The
two kinds described none of those packs, and said nothing about what a pack takes and
gives: the params an estate answers, the lists a team adds to, the exports. A person or an
editor that chooses or writes a pack needs exactly that, and needs to know whether the
estate gives what the pack reads.

## Options

1. **Keep the two kinds and add more** (a VM, a bucket). Cost: every new kind of
   delegation is a change to satz; a pack that reads two kinds or an unusual value is
   still described wrongly.
2. **Report what the pack declares, from its parse, and match its reads against the
   estate's interface folders** by the transpile's own rule. Cost: `satz packs` and
   `get-pack` need an estate — the one in `yaml_dir`, or `--estate` — and the estate's
   front end has to load; the report is longer.
3. **Report the declarations without matching.** Cost: whether a pack fits the estate is
   known only at the transpile, after the pack is fetched and used.

## Decision

Option 2.

- **Each row of `satz packs`** carries `params` (`name`, `default` in Satz or `null` for
  `""`, `question`, `prompt`), `list_params` (each `entries` statement: `param`, `key`,
  `fields`, `defaults`), `exports` (`interface` or `null` for a core export, `name`) and
  `reads`, every `${{interface.<folder>.<value>}}` and `${{interface.<value>}}` the pack
  writes in a param, a resource or an export. They are read from the parse, so a comment
  reads nothing.
- **A read is matched by the transpile's rule** (`interface_read` in the front end): a
  named read by the used folder of that name when it exports the value, a bare read by
  every used folder that exports it when they agree. `met_by` lists those folders;
  `unmet_reason` is the transpile's refusal when there are none. A read in a param the
  estate binds is not made, and is reported as `bound_by_estate`.
- **The interface folders are the ones the estate's transpile loads** — its front end, not
  a second reader. `satz packs` and `get-pack` take `--estate`, the one estate in
  `yaml_dir` without it, and refuse when the front end refuses the estate.
- **`get-pack` warns** per read the estate does not give and fetches the pack all the
  same; `review-pack --against` already refuses such a read through the transpile.
- **`needs` is removed**, with no field in its place.

## Consequences

- A pack is judged by what it reads, whatever was delegated: a pack that reads
  `${{interface.delegation-vm.instance_name}}` fits an estate whose folder `delegation-vm`
  exports `instance_name`, and nothing else is asked of the estate.
- A pack a source holds and `packs_dir` does not is matched too, so the list says before
  the fetch whether it fits.
- `satz packs` and `get-pack` no longer run without an estate; a repository whose
  `yaml_dir` holds several names one with `--estate`.
- A pack whose bare read two folders carry differently is reported unmet until the estate
  binds the param that holds it, which is the edit ADR 0094 asks for.
- `review-pack` without `--against` gives a pack that names folders one stand-in folder
  per name, so a pack written with named reads clears the bar it is reviewed against.
