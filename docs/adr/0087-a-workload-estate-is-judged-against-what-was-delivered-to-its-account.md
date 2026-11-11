# 0087 — a workload's estate is judged against what was delivered to its account

- **Status:** accepted
- **Date:** 2026-11-11

## Context

Every compile checks that the account an estate runs as holds the roles its resource
types need, and that the APIs serving them are enabled on the project their calls are
billed to. The check knew one kind of estate: an organisation's. Roles were read from
grants at the organization and on the billing account, every gap was offered as a grant
at the organization, and `update-prerequisites` wrote it.

For an estate beside the organisation's, run as an account `presets/folder-workload.satz`
created (ADR 0086), all three were wrong.

- The account holds its roles on its own folders, so every need read as missing: the
  first such estate was told it lacked thirteen roles "at the organization".
- The fix offered was a grant at the organization, written into an estate whose account
  could not apply it — and which, applied by someone who could, would have undone the
  pack's whole point.
- The project the estate's calls are billed to is declared by the organisation's estate.
  The workload's estate cannot declare an API on it, and `apply` refuses while an API is
  not declared: the estate could not be applied at all.

## Options

1. **Skip the check for an estate that reads an interface.**
   - Pros: nothing to build.
   - Cons: the apply then fails half-way on the first resource the account may not
     create, which is what the check exists to prevent.
2. **Teach the role table folder scope and read the organisation's estate.**
   - Pros: exact.
   - Cons: a workload's repository does not hold the organisation's estate, and must
     not need to.
3. **The pack publishes what it delivered, and the check reads it.** By convention
   name, in the interface the workload's estate already vendors.
   - Pros: the same file that hands over the folder says what the account can do in it;
     no second source; the answer to a gap is a request to the estate that can close it.
   - Cons: a contract by name between a pack and the binary; an interface written by
     hand can claim a delivery nobody made.
4. **A new field of the interface file** (`delivery { … }`).
   - Cons: an interface file with a field an older binary does not know is refused by
     every older binary; exports are read by all of them.

## Decision

Option 3. An estate is a workload's when an interface file it uses exports
`delivered_folder_roles`; `delivered_billing_roles` and `delivered_apis` come from the
same file. For such an estate:

- a need on a folder or a project is met by a delivered folder role, or by a role the
  estate grants its own account on a folder or project it declares;
- a need on a project is met as well when `roles/resourcemanager.projectCreator` was
  delivered: Google makes a project's creator its owner, and the estate's projects are
  the ones it creates;
- a need on the billing account is met by a delivered billing role;
- an API on the default provider's billing project is enabled when it was delivered;
  one on a project the estate declares is judged as before;
- the reads `import`, `adopt` and `report-compliance` need are not asked for.

What is not delivered is a finding at the validation level that says what to ask the
organisation's estate for. It carries no `fix`. `update-prerequisites` writes the APIs of
the estate's own projects and nothing else, refuses a grant at the organization or on
the billing account for such an estate whoever asks, and exits non-zero while anything is
undelivered. `apply` keeps refusing on a missing API. `whoami` tests the billing account
live and says the rest is judged against the delivery: the folders' ids exist only once a
plan has looked them up.

An estate on an interface without these exports is judged at the organization as before,
with a note saying so.

A pack's on/off param that the estate binds itself now counts as answered when the pack
that asks it is not used: the example organisation uses `folder-workload.satz` without the
estate map and was told the map was off.

## Consequences

- The check stays, and it is now true for a workload: a gap names the list and the pack
  where it is closed.
- A workload's estate cannot widen its own account by running a satz command.
- The delivery is what the organisation's estate declares, not what is live: an
  organisation that has not applied yet has delivered nothing, and the plan says so.
- Owning a project is assumed for projects the estate creates. A resource in a project
  somebody else created needs a role there that this check does not see.
- `presets/project-onboarding.satz` publishes no delivery yet; its projects are judged
  as before until it does.
