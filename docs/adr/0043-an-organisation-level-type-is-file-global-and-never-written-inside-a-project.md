# 0043 — an organisation-level type is file-global and never written inside a project

- **Status:** accepted; ADR 0038 applied to what a project's body takes
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

ADR 0038 made every entry judged by the position it stands in, and listed four positions
a `use` stands in — the top level of a file, the body of a folder or a project, a map of
node names, a resource type map. It left one question unanswered: what the body of a
PROJECT takes.

The emitter reads `folder:` and `project:` out of `Entity::node_path`. A resource whose
type has a `project` attribute and stands in a project's body gets that project; one
whose type has none — a folder, a project, an organisation grant, a Cloud Identity group,
a billing grant — gets nothing from the project and reaches the organisation, the Cloud
Identity customer or the billing account instead. Written at the top level of a file that
is `use`d in a folder's body, that is exactly right and it is what the library relies on:
the audit-logsink pack declares an organisation sink and an organisation audit config
beside the project it creates, and one estate file declares a project together with the
groups that go with it rather than being split in two. `tests/corpus/hoist-two-folders`
and `tests/corpus/billing-nested` pin the same hoist out of a FOLDER's body, where it is
also right — a folder has children, and a group declared under two folders reaches the
organisation once.

Written directly inside `google_project { p { … } }` it is not right. The author wrote
"in this project" and got "at the organisation", with the project's provider alias on the
block, and nothing said so. Two forms of it were worse than silent: `google_folder` and
`google_project` inside a project's body were emitted as children of the ORGANISATION,
a hierarchy edge that does not exist; and an org policy in a project's body was emitted
with `parent = google_project.p.project_id`, the bare id, which is no Resource Manager
path and which no apply could take.

## Decision

**Organisation-level, file-global: accepted. Organisation-level, resource-local:
refused.**

A type that belongs above a project — `google_folder`, `google_project`, and any type
whose scope is `Org`, `Customer` or `Billing` — is written at the TOP LEVEL of a file,
the file that declares a project included, and reaches its own scope from wherever that
file is `use`d. Written as a block directly inside a named project's body it is refused
by the position check (`pipeline/position.rs`, `above_a_project`), with the message
naming what it belongs to and the edit.

Three things this deliberately does NOT do:

- **It does not touch what a used file's entries are judged as.** They are judged at the
  used file's own top level (ADR 0038), which is what makes the hoist file-global; this
  record decides what a project's body takes when a block is written THERE.
- **A folder's body is untouched.** A folder has children; the hoist out of one is a
  corpus-pinned feature, not an accident.
- **Nothing hoists differently.** The refusal is of a position, never of a type, and no
  resource moves in any plan.

**An org policy in a project's body gets `parent = "projects/${…project_id}"`**, with its
`name` built from that path — the one place where "the project places it" was emitted in
a form the API does not take.

## Consequences

- An estate that wrote an organisation grant, a group, a billing grant, a folder or a
  project inside a project's body is refused and edited by hand: the block moves to the
  top level of the same file. Its plan does not change — the resource was already emitted
  at the organisation. `presets/README.md`'s `## Breaking changes` carries the edit
  (ADR 0041).
- The check is decidable without a schema, from `Scope` plus the two node type names.
  What it therefore does NOT catch is a type that is organisation-level only because its
  schema has an `org_id` and no `project` — `google_organization_iam_audit_config`,
  `google_logging_organization_sink`. Written in a project's body those still reach the
  organisation silently. Catching them needs the provider schema at position-check time,
  which the front end does not have; they are the remaining edge, not a second rule.
- `docs/language.md`'s `use` section states the rule with an example that compiles, which
  is where it was missing: the hoist has always been the behaviour and nothing said it.

## Alternatives

- **Place a folder or a project under the project.** Not available: the resource
  hierarchy is organisation → folder → project, and a project has no children in it.
- **Warn instead of refusing.** Rejected: the warning would fire on every compile of an
  estate that cannot be right, which is the pile-up P12 exists to stop.
- **Say nothing and document the hoist.** Rejected for the resource-local case only: an
  author who writes a group inside a project's body has said where it goes, and getting
  something else without a word is the failure this record is about. The file-global case
  needed the documentation and nothing else, and now has it.
