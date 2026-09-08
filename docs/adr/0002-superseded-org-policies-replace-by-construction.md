# 0002 — superseded org policies change address so the plan is a replace

- **Status:** accepted
- **Date:** 2026-09-08
- **Shipped in:** v0.46.92, CIS pack v2.6

## Context

Where Google replaces a legacy org-policy constraint with a managed one, the CIS
baseline runs the replacement alone and declares the legacy twin off with
`spec { reset = true }`. Absence is not enough: a legacy policy already set on an
organisation is invisible to an apply that does not declare it, and goes on enforcing.

Pack v2.5 kept the legacy blocks at their existing addresses, on the reasoning that an
estate upgrading would then see an in-place update rather than a destroy and a create.

**That update is impossible.** The provider PATCHes the rules it still holds in state
together with `reset`, and the API refuses the pair:

```
400 Cannot set PolicyRules if reset is true
```

So every estate upgrading needed `tofu apply -replace=google_org_policy_policy.<addr>`
by hand, once per legacy policy that existed live with rules — and the pack's own
comment said the opposite of what happened.

## Considered options

1. **Keep the addresses, document the `-replace` list** in the upgrade runbook.
2. **Give the superseded blocks a distinct address**, so the plan is a destroy and a
   create by construction.

## Decision

Option 2. The six superseded blocks carry a `-superseded` suffix as of pack v2.6. The
policy **name** is unchanged, so this is one org policy being reset, not two.

## Consequences

- The replace is in the plan, where an operator reviews it, instead of in a runbook
  they have to remember.
- **One-time fleet cost:** an estate on 2.4 or 2.5 sees one destroy + create per
  legacy policy that exists live with rules. An estate that already performed the
  manual `-replace` sees it once more, and then never again.
- Between the destroy and the create, that constraint is briefly absent. The managed
  replacement enforces the same control throughout, which is what makes the window
  uneventful — and it is why this record would read differently for a constraint with
  no managed twin.
- The wrong comment in the pack is corrected, and the reasoning now sits next to the
  blocks it explains.

## Pros and cons of the options

### 1 · Keep the addresses, document `-replace`

- **Good:** no address churn; estates already on v2.5 see nothing new.
- **Good:** no code change at all.
- **Bad:** the fix lives in prose, so it works only for the operator who read it.
  Everyone else meets a failed apply and an API error naming neither cause nor cure.
- **Bad:** it leaves the tool emitting a plan that cannot succeed, which is the thing
  a transpiler exists to prevent.
- **Bad:** it conflicts with the project's rule that a release IS the migration —
  breaking changes ship with the edits that satisfy them, not with instructions.

### 2 · Distinct address *(chosen)*

- **Good:** the plan is correct by construction; nothing to remember, nothing to
  document as a manual step.
- **Good:** matches "a release IS the migration".
- **Good:** the emitted plan shows exactly what will happen, in the place an operator
  already looks.
- **Bad:** an address change, so v2.5 estates carry one replace they would otherwise
  have skipped — including two that had already done it by hand.
- **Bad:** `-superseded` is now load-bearing in the address, so renaming it later is
  itself a replace. That is the same cost as this decision, paid again.
