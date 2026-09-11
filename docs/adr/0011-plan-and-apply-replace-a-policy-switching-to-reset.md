# 0011 — `plan` and `apply` replace a policy switching to reset

- **Status:** accepted
- **Date:** 2026-09-11
- **Shipped in:** v0.48.0

## Context

Switching an org policy from rules to `spec { reset = true }` cannot be an in-place
update. The provider sends the rules it holds in state together with `reset`, and the
API refuses the pair: `400 Cannot set PolicyRules if reset is true`.

[ADR 0002](0002-superseded-org-policies-replace-by-construction.md) made the switch a
replace by construction: the CIS pack declares each legacy twin under a new
`-superseded` address, so tofu plans a destroy of the old address and a create of the
new one. [ADR 0005](0005-adopt-moves-a-renamed-block-rather-than-importing-it-again.md)
then made `adopt` move a renamed block's state instead of importing it again — without
the move, the plan destroys the live object under its old name. For a legacy twin the
two decisions collide. The move carries the old rules onto the `-superseded` address,
tofu sees an in-place update again, and the apply is refused. This happened on an
estate's first apply after the 2.7 pass; the runbooks answered it with `tofu apply
-replace=…` by hand, and nine more estates had the same step ahead.

`satz plan` and `satz apply` were deliberately thin: they run the tool in `hcl_dir`,
pass the arguments through and propagate the exit code.

## Considered options

1. **A runbook step:** `tofu apply -replace=<address>` by hand for every moved twin.
2. **`adopt` does not move a twin declared reset**, and leaves it to the plan's
   destroy + create.
3. **`satz plan` and `satz apply` add `-replace`** for each org policy the state holds
   with rules while the estate declares it reset, and say so; `adopt` names the case
   on its MOVE row.

## Decision

Option 3. The condition is read from the state (`tofu show -json`) and the emitted
`main.tf`, and only when `main.tf` declares a reset policy. Nothing is added to an apply
of a saved plan, to `-destroy` or `-refresh-only`, or for an address the arguments
already replace. `plan` adds it as well as `apply`, so the plan reviewed is the one
applied.

## Consequences

- The 2.7 pass needs no manual replace; `satz plan` shows the replace with a note
  naming the policy.
- `satz plan` and `tofu plan` differ for these policies: `tofu plan` run directly
  still shows the in-place update that the apply cannot make.
- An estate with a reset policy pays one state read per `plan` or `apply`.
- The wrapper is no longer purely pass-through; this is its one addition, and it is
  named in the command table and in the `run_tf` comment.
- Under ADR 0010 this release is a minor: the same estate plans differently through
  `satz plan`.

## Pros and cons of the options

### 1 · A runbook step

- **Good:** the wrapper stays pass-through.
- **Bad:** the knowledge lives outside the tool, and the first plain apply after an
  upgrade fails on it — as it did.

### 2 · No move for reset twins

- **Good:** the plan's destroy + create is the replace ADR 0002 intended, with no
  change to the wrapper.
- **Bad:** it covers only the path through `adopt`. A policy the estate switches to
  reset at an unchanged address, or a twin moved by hand with `tofu state mv`, still
  meets the refused update.
- **Bad:** adopt needs a new outcome for "in state elsewhere, left for the plan to
  replace", beside MOVE and IMPORT — one more rule in the place where a wrong answer
  deletes a live object.

### 3 · `plan` and `apply` replace *(chosen)*

- **Good:** correct whichever way the state got there — moved by adopt, imported by
  hand, or an estate that switched a policy to reset itself.
- **Good:** visible: a note per policy, and the plan shows the replace before the
  apply makes it.
- **Bad:** satz and plain `tofu` plan these policies differently.
- **Bad:** an extra state read, and argument parsing to tell a saved plan from a flag's
  value.
