# 0085 — a top-level folder says its parent by reference or by id

- **Status:** accepted
- **Date:** 2026-11-11

## Context

A folder's parent follows from the block it stands in. A top-level folder could also
state `parent`, and the value was written as a quoted string whatever it said. That
served one case: a partial import, where the parent is the id of a folder outside the
estate (`"folders/123456789012"`).

A pack that creates folders has no block to stand in. It is used at the top level
(ADR 0046), and the folder its folders belong under is the estate's: named in a param,
or different for each entry of a list. Written as `parent = "google_folder.workloads.name"`,
the generated code carried that text as a string, which Google refuses as a parent and
which gives OpenTofu no dependency on the folder it names.

A project already says its parent this way: `folder_id` reads a dotted path as a
reference and anything else as an id (ADR 0045).

## Options

1. **Read a top-level folder's `parent` as a project's `folder_id` is read.** A dotted
   path without `/` is a reference, `${{…}}` is a reference, `folders/<id>` and
   `organizations/<id>` are ids.
   - Pros: one rule for both node types; no new syntax; an estate that states an id
     gets the code it got before.
   - Cons: the form of the text decides, so a typo in a reference is found by
     `tofu validate`, not by satz.
2. **A second attribute for the reference** (`parent_folder = workloads`).
   - Pros: the label can be checked against the estate's folders at compile time.
   - Cons: two attributes for one fact, a second rule beside ADR 0045, and a list entry
     that may name either kind needs both fields.
3. **Let a pack nest into a folder the estate names** (a block whose label is a param).
   - Cons: a label that is a param is a new language form; an entry that names an id of
     an existing folder still needs `parent`.

## Decision

Option 1. A top-level `google_folder` states `parent` as:

- `google_folder.<label>.name` or `"${{google_folder.<label>.name}}"` — a reference, written
  bare, so OpenTofu orders the two folders;
- `folders/<id>` or `organizations/<id>` — the id, written as it is;
- nothing — the organisation.

A nested folder that also states `parent` is refused, as before.

## Consequences

- A pack creates a folder under a folder the estate names, and one list can place its
  folders under unrelated parents: one declared here, one that already exists.
- A reference to a folder the estate does not declare is reported by `tofu validate`.
- No existing estate changes: an id contains a `/` and is written as before.
