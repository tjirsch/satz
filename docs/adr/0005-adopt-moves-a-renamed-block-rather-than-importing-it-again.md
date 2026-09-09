# 5. Adopt moves a renamed block rather than importing it again

- Status: accepted
- Date: 2026-09-09

## Context

A pack that renames a resource block changes the name the estate gives an object. It
does not change the object in the cloud. The state, however, is keyed by the old name.

`satz adopt` asked one question of the state: *is this address managed?* For a renamed
block the answer is no, so adopt resolved the object live and imported it — under the
new address, while the old address still held it. One live object, in the state twice.
The next `plan` then proposed to destroy the old address, and destroying an org policy
address deletes the policy for real.

This is not hypothetical. It happened on the first CIS 2.1 → 2.6 upgrade, where the
pack renamed six superseded blocks. The repair was manual: `tofu state rm` on each old
address, then a `-replace` apply. Eight estates are due the same upgrade, six renames
each, so the manual repair does not scale and the failure mode is destructive.

The state can answer the better question — *is this live object managed, under any
name?* — because `show -json` carries each resource's type and live id beside its
address. What was missing was asking it.

## Options

**Ask about the address only, and let the operator notice.** What we had. It costs
nothing to build and it is what caused the incident: by the time the duplicate is
visible it is in the plan as a destroy, alongside legitimate destroys, in a diff that
an operator upgrading a pack expects to be noisy.

**Detect the duplicate and refuse, printing the `state mv` to run.** Safe, and it keeps
every state mutation in the operator's hands. But `--execute --import` already mutates
the state — that is what an import is — so refusing here draws the line in a strange
place: adopt may write to the state when the write is an import, but not when it is a
move, though the move is the *less* destructive of the two. It also leaves the eight
pending upgrades as manual work, which is the problem.

**Detect the duplicate and move it.** Chosen. `--execute --import` runs
`tofu state mv <old> <new>` for the row and reports it as a move; the dry run says
`MOVE` and names the address it would move from, so the operator sees the decision
before it is taken, exactly as they see a pending import.

## Decision

Adopt indexes the state by `(resource type, live id)` as well as by address. A
resolution whose live object is already managed under a *different* address is a
**move**, not an import.

Three constraints keep the inference honest:

- **Exact match only.** Both the type and the live id must match exactly. A near-match
  is not evidence of sameness, and the cost of a wrong move is moving a resource that
  merely looks like the one being adopted. A missed match is merely the old behaviour.
- **Data sources are never matched.** They are read, not managed; they can be neither
  imported nor moved, so indexing them could only produce a false match.
- **Declaring both ends is an error.** If the estate still declares the old address,
  one live object has two declarations. A move does not resolve that — it changes which
  of the two the next plan wants to create — so adopt stops and names the pair.

## Consequences

Adopt now performs a state mutation that is not an import, under `--execute --import`.
That is the cost: the flag's name says "import" and one of the things it may now do is
a move. The alternative was a flag per state operation, which describes the mechanism
instead of the intent — the intent is "bring the state in line with what the estate
declares", and both operations serve it.

Reading the state moved from `state list` to `show -json`. That is a larger response
and needs the providers installed, which `init` does anyway; the same call already
backs `satz import <state>`.

The pack-rename upgrade becomes ordinary: adopt, then plan, with no manual
`tofu state rm` and no window in which a plan proposes to delete a live policy.
