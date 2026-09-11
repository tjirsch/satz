# 0010 — the minor version marks an upgrade that brings work

- **Status:** accepted
- **Date:** 2026-09-11
- **Shipped in:** v0.47.0

## Context

The public repository started at 0.46.1 on 2026-08-28. All 109 releases after it were
patches, v0.46.2 to v0.46.110, and some of them changed what an existing estate or
input needs:

- #38 made the front-end refuse what it had silently dropped — a `use … when` on an
  undeclared param, a duplicate param — so estates that compiled before needed edits;
- #103–#105 gave the emitted state backend the service account's identity, so `tofu`
  refused the next command until `init -reconfigure`;
- #139 refused Prowler 4 exports, which the release before had read.

satz keeps everyone on the current version. A release is the migration: a breaking
change ships together with the estate edits that satisfy it, with no deprecation
period. That rule stays. What it left open is how an operator sees, before upgrading,
whether an upgrade brings work. Under patches only, 0.46.95 → 0.46.110 looks the same
whether nothing changed for the estate or three edits are owed.

Before 1.0, semantic versioning puts breaking changes in the minor number.

## Considered options

1. **Patches only**, as before.
2. **A minor when an upgrade brings work:** the same estate or input, run through the
   new binary, needs an edit, is refused, or plans differently. Everything else is a
   patch.
3. **A minor per roadmap milestone** — the fleet re-verified, the interview shipped.
4. **1.0 now, then full semantic versioning.**

## Decision

Option 2. The rule is in `CLAUDE.md` (release flow) and in the README's *Releasing*
section. The categories that make a release a minor are:

- a language change;
- a removed or renamed command or flag;
- an input format no longer read;
- an emission change that moves a plan.

0.47.0 is cut with this rule itself. By the rule that release is a patch; the line
opens with the release that defines it.

## Consequences

- An operator reads a minor as "read the release notes before the pickup", and a
  patch as "upgrade". Fleet pickups can be keyed on the minor number.
- Every release takes a judgement from its author. The failure mode is a breaking
  change released as a patch. The four categories make that judgement checkable in
  review, from the diff alone.
- In an active phase the minor number moves often. That is the information the rule
  is meant to carry, not noise.
- Nothing about migration changes: no deprecation periods, no dual-accept paths. A
  minor says there is work; it gives no time to postpone it.

## Pros and cons of the options

### 1 · Patches only

- **Good:** no judgement per release.
- **Bad:** the number says nothing about the upgrade, which is the problem this
  record exists for.

### 2 · A minor when an upgrade brings work *(chosen)*

- **Good:** the number answers the one question an operator has before upgrading.
- **Good:** it follows the pre-1.0 semantic-versioning reading, so it means what
  tooling and readers already expect.
- **Bad:** the author has to judge each release, and a wrong patch is not caught by
  any test.

### 3 · A minor per milestone

- **Good:** the number marks progress.
- **Bad:** it tells an operator nothing about the upgrade. A milestone release can be
  fully compatible, and a breaking change between two milestones hides in a patch.

### 4 · 1.0 and full semantic versioning

- **Good:** the most widely understood contract.
- **Bad:** 1.0 promises a stable language and command surface. satz does not give
  that promise: it changes the language when needed and ships the migration with the
  change. Declaring 1.0 is a separate decision about maturity, not about release
  numbering.
