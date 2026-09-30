# 0075 — `each` writes an interface per entry, and onboarding a project is a pack

- **Status:** accepted
- **Date:** 2026-09-30
- **Shipped in:** the release that follows

## Context

`satz add-project` (ADR 0070) appended to the central estate a generated section per
project: the Google project, its IaC service account and state bucket, the grants, and
`interface "<name>"`. It was a command because a pack could not do it — `each` (ADR 0071)
expands resource bodies, and nothing wrote an `interface` block per entry of a list. The
cost was a second onboarding mechanism beside the request plane: a project asked for a
subnet with an entry in a list and for its own existence with a command someone ran on the
estate, and every onboarded project left a copy of the same fifty lines in the estate.

## Decision

**`each <list> by <field> { interface "…" { … } }` at the top level of a file writes each
interface block once per entry of the list, and `presets/project-onboarding.satz` onboards
the projects of its list param `projects`; `satz add-project` is removed.**

- **One `each`, the statement inside it.** The spelling is the resource form's, at the top
  level, where `each` was refused before, so no file changes meaning. The block holds
  `interface` blocks only; resources keep their `each` inside their type map.
- **Expanded where the file's interfaces are read**, with the param namespace as the walk
  holds it, contributions merged — the same list the resource `each` beside it sees. The
  name reads the entry (`"{each.name}"`, text around it allowed, no param) and is judged as
  a written name once expanded; export values read it through `each_value`, the resource
  form's substitution.
- **An interface an `each` writes is one project's own, a pack's too.** A pack's interface
  is common because a pack is shared by every estate that uses it; one written per entry is
  per project by construction, and a common one would put every project's values into
  every project's folder. `common` in the block still makes it common.
- **Inside `${{…}}` an entry's text is written with `-` as `_`.** A label reaches the HCL
  with `-` as `_` (§2.2), and the reference resolvers compare against emitted labels; a
  project named `data-lake` has the interface `data-lake` and the resource
  `google_project.data_lake`. Only the substituted text changes, and only inside a
  reference, where a `-` could never name anything emitted.
- **The pack writes what `add-project` wrote**: the same resource labels, names, APIs,
  roles and exports. Where `add-project` read the estate's `workload_folder` export to pick the
  parent, the pack takes `project_onboarding_folder`, empty for the organisation — the
  library's convention for a pack's parent (`shared-network`, `billing-export`). A project
  is requested like a subnet: `contributes_projects` in a file `satz check-request` checks.

## Options

**`interface each <list> by <field> "…" { … }`**, the loop on the statement. *Rejected
(2026-09-30):* a second spelling of `each`; the top-level form is the resource
form's.

**Keep `add-project` beside the pack.** *Rejected:* two ways to onboard, one generated text
the operator owns afterwards and one an entry the request plane checks; the command's
`--use-interface` and `--export` are a block of the same name in the estate's own file,
since an interface declared in two files is one.

**Pack instances** (`use "…" as payments { params { … } }`, deferred in ADR 0071).
*Rejected again:* one expansion point covers the case, and instances would need a param
scope per instance in every tool that reads pack lines.

**Normalise `-` in every `${{…}}` reference** rather than in the text an `each` writes.
*Rejected:* it would change how hand-written references are read and reach the emitter's
rewriting; the `each` substitution is the one place a label arrives as data.

**Default the parent to the workload folder.** *Rejected:* an estate whose workload folder
is the organisation declares no `google_folder.workload_folder`, and a pack cannot branch on
the form of an export; the question asks for it.

## Consequences

- `satz add-project` is gone (a `## Breaking changes` entry under v0.90.0). A section it
  wrote is plain Satz and stays; a new project is an entry.
- The owner group's address is no longer checked for `@` before the compile; a malformed
  one fails at apply, as any other member does.
- The grammar (`satz-tree-sitter`) learns the top-level form; the canonical form carries
  `each_interface(name|list|key)`, so pack drift sees it.
- An onboarded project's labels are the entry's name: renaming an entry moves the
  project's resources, as renaming a written label does, and renames its interface.
