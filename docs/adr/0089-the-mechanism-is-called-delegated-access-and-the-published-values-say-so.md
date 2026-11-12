# 0089 — the mechanism is called delegated access, and the published values say so

- **Status:** accepted
- **Date:** 2026-11-12

## Context

Two packs give a team limited rights in an organisation: `presets/folder-workload.satz`
(ADR 0086) and `presets/project-workload.satz` (ADR 0088). The texts around them had no
one word for what they do. The project pack was "onboarding a project", which reads as
moving an existing project into the organisation; that is not what it does. Elsewhere it
was "a team's own account", "handing over", and "delivered" for what the account was
granted — the last one also in the names of the values the packs publish
(`delivered_folder_roles` and four more), which the prerequisite check reads (ADR 0087).

Several words for one thing is how the same mechanism gets explained three times and
understood as three mechanisms.

## Options

1. **Replace "onboarding" where it misleads, and leave the rest.**
   - Pros: a handful of sentences.
   - Cons: the mechanism still has no name; "delivered" stays in prose and in names.
2. **One term in the texts, the published names as they are.**
   - Pros: no estate is refused.
   - Cons: the prose says "delegated" and the names say `delivered_`, for as long as the
     names live — which is as long as the packs.
3. **One term in the texts and in the published names.**
   - Pros: what a team reads in the documentation is what it reads in its interface and
     in a finding.
   - Cons: a breaking change for every estate that vendored such an interface. At the
     time that is one.
4. **Rename the packs too** (`folder-delegation`, `project-delegation`).
   - Cons: a third rename of the same packs within two days, for names that already say
     what the account reaches. Not done.

## Decision

Option 3.

- The mechanism is **delegated access**: it gives a team access to a part of the
  organisation's resources — a folder, a project — with scoped permissions: an account of
  its own that holds roles there and nowhere else, and cannot widen them. Folders and
  projects are today's scopes; the term does not depend on them.
- It is the lead term wherever the mechanism is introduced: the packs' headers, the
  questions the estate map asks, the references, the tutorials, the README satz writes
  into an interface folder, the findings of the prerequisite check.
- The published values are `delegated_folder`, `delegated_folder_roles`,
  `delegated_project_roles`, `delegated_billing_roles` and `delegated_apis`. An interface
  file that still exports a `delivered_…` name is refused where it is used, naming the
  file: it is not read under the old name.
- Words that stay: **hand over** for passing the generated folders to the team; the
  packs' names and their lists; Microsoft's "onboarding" for Defender and Sentinel; the
  day-0 onboarding of a new customer organisation, which is what `satz bootstrap` does.

## Consequences

- A team's estate that vendored an interface before v0.98.0 takes the folder again and
  renames one reference; `presets/CHANGELOG.md` has the steps. No plan moves.
- The JSON report of `update-prerequisites` names its fields `delegation` and
  `not_delegated_apis`.
- Earlier records (ADR 0086 to 0088) keep the words they were written in.
