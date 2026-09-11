# 0009 — the IaC service account holds named roles, derived from the resource types

- **Status:** accepted
- **Date:** 2026-09-11
- **Shipped in:** v0.46.110

## Context

`satz init` wrote an estate whose IaC service account held `roles/owner` at the
organization. The reason given was reach: the account has to change folders and
projects that existed before the estate, or were created by hand, and are adopted into
it later.

Reach does not need owner. A role granted at the organization is inherited by every
folder and project under it, existing and future, hand-made or not. What owner adds
is breadth: every permission of every service, none of them named. An apply can then
change anything in any project, and a reviewer reading the estate learns only that the
account can do everything.

Three facts narrow the alternatives:

- **Owner cannot be granted temporarily.** IAM conditions and Privileged Access
  Manager both exclude the legacy basic roles (owner, editor, viewer). PAM grants
  predefined or custom roles, for at most seven days.
- **The account keeps `roles/resourcemanager.organizationAdmin`,** because the packs
  grant organization-level roles to groups. That role carries
  `resourcemanager.organizations.setIamPolicy`, so the account can always grant itself
  any role. Named roles narrow what an ordinary apply uses and name each permission;
  they are not a boundary on what the account could do.
- **Which roles an estate needs follows from what it emits.** The emission manifest
  lists every resource type; each type needs a small set of permissions, each carried
  by one or a few predefined roles.

## Considered options

1. **Keep owner** and record the deviation from CIS 1.5 in the estate.
2. **Named roles in the estate, checked at every compile and written by
   `satz iac-roles --execute`,** from a type→role table in satz's source.
3. **satz emits the grants itself** from the manifest, with no line in the estate.
4. **Each pack carries the grants its resources need.**
5. **Option 2, with `organizationAdmin` gated through PAM** — requested per apply,
   granted for the apply's duration.

Two smaller choices sit inside option 2:

- **Where the table lives:** `src/iac_roles.rs`, or a data file under `presets/`
  that `get-presets` ships.
- **Print or write:** the compile prints the grant lines to paste, or a command
  writes them into the estate.

## Decision

Option 2. The table is a static list in `src/iac_roles.rs`, and the command writes.

- `satz iac-roles <estate>` compares the roles the estate — packs included — grants
  the IaC service account with what the emitted types need, and exits non-zero on a
  gap. `--execute` adds the missing roles to the account's existing grant list, or
  appends a block, then compiles again and restores the file if a gap remains.
- Every compile runs the same check at `validation_level`: `warn` names the roles and
  the command, `error` refuses, `none` skips. `roles/owner` at the organization counts
  as covering every organization and project need.
- `satz whoami <estate>` tests the permissions live, with the credential the estate's
  live commands run as.
- The template's account holds the reads (`viewer`, `browser`,
  `iam.securityReviewer`, `cloudasset.viewer`, `serviceUsageConsumer`), the roles its
  own types need, and `roles/billing.admin` on the billing account.

## Consequences

- Every role the account holds is a line in the estate file, and an estate diff shows
  a new one. The estate stays the record of every grant.
- A pack that brings a new resource type brings a compile warning naming the role,
  and one command writes it.
- The table is maintained. A pack emitting a type without a row fails `cargo test`
  (`iac_roles_gate`, over cases in `tests/iac/` that use every pack). A role Google
  changes is caught only by `scripts/check_iac_roles.py`, which needs credentials and
  is run by hand — `docs/housekeeping.md` lists it.
- The apply that adds a role and the resources that need it runs them in one pass; a
  resource created before the grant takes effect fails and succeeds on the next apply.
- Google makes a project's creator its owner, so the account is owner of every project
  it creates and can delete those; a project it did not create needs
  `roles/resourcemanager.projectDeleter` for its deletion, which the table does not
  grant.
- `organizationAdmin` remains, so the account can still grant itself anything. Named
  roles do not meet CIS 1.5 either: its audit flags a service account holding owner,
  editor or any role whose name contains `Admin`. What changes is that each is named.
- Estates written before this keep `roles/owner` until each is migrated. Owner counts
  as covered, so nothing breaks in the meantime. Migrating one is removing the
  `roles/owner` line and running `satz iac-roles <estate> --execute`, which then writes
  the named roles.

## Verification

On a test organization, 2026-09-11: a service account holding only the 16 roles
`satz iac-roles --execute` wrote for a probe estate created, and then destroyed, a
folder with an IAM grant and a folder-level org policy, a project with an IAM grant,
an organization custom role, and — inside an existing project it had not created —
a workload identity pool, a bucket, a log metric and a service account with an IAM
grant. It moved the project between folder and organization in place. `satz whoami`
tested 20 permissions as that account and named the one withheld on purpose, the
billing link, which the provider's own pre-check then asked for by the same
permission. `scripts/check_iac_roles.py` held all 51 table entries against Google's
34 role definitions.

## Pros and cons of the options

### 1 · Keep owner

- **Good:** an apply never fails for a missing permission; nothing to maintain.
- **Good:** reach is total, including services no pack uses yet.
- **Bad:** every permission of every service, none named; a reviewer cannot tell from
  the estate what the account touches.
- **Bad:** the broadest role there is, as a standing grant on the identity every apply
  runs as.
- **Bad:** it cannot be made temporary — conditions and PAM exclude it.

### 2 · Named roles, checked and written *(chosen)*

- **Good:** every role is in the estate file; reach through organization inheritance
  is the same as owner's.
- **Good:** the gap is found at compile time, before an apply, and written by a command
  rather than by hand — the grant list's key is `{param}`-interpolated, which is easy
  to get wrong when pasting.
- **Good:** `whoami` tests the result live, so a grant still propagating or held
  elsewhere shows before the apply.
- **Bad:** a table to keep current, half of it (Google's role changes) without an
  automatic check.
- **Bad:** a new role can meet propagation on the apply that grants it.
- **Bad:** not a security boundary while `organizationAdmin` stays.

### 3 · satz emits the grants

- **Good:** never stale, no estate edit, nothing to forget.
- **Bad:** grants appear in the plan that are in no estate file; the estate stops being
  the record of what the account holds, and a reviewer of the estate diff never sees a
  role change.
- **Bad:** an estate that wants a different role than the table's choice has to
  `suppress` a grant it never wrote.

### 4 · Grants carried by the packs

- **Good:** the need sits beside the resource that has it.
- **Bad:** the estate's own resources — the template's folder, project and bucket —
  need the same mechanism, so there would be two.
- **Bad:** what the account holds is spread over every pack the estate uses; a
  reviewer reads each one to know.
- **Bad:** each pack author maps types to roles again, with no gate holding the copies
  to Google's definitions — the table does it once, and the script checks it.

### 5 · PAM-gated `organizationAdmin`

- **Good:** the power to re-grant exists only during an approved window.
- **Bad:** every apply becomes a request, an approver and a wait; the entitlement
  itself has to be managed, and a scheduled or CI apply needs a requester that can
  approve itself or a human in the loop.
- **Not rejected:** it builds on option 2 and can follow it; the table is the
  precondition, because PAM grants named roles only.

### Table in source, not in `presets/`

- **Good:** versioned with the binary that reads it; the gate is a unit test.
- **Good:** no new YAML — the dialect exists only to be migrated.
- **Bad:** a new row needs a satz release, where a preset file would reach estates
  through `get-presets`. Rows change when packs change, and packs ship in releases too.

### Write, not print

- **Good:** the written grant is checked again before the command returns; the file is
  restored on a gap.
- **Bad:** satz edits the estate file. The edit is bounded: one list, or one appended
  block, with a comment naming the command.
