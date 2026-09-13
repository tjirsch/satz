# 0015 — an exemption is a tag binding, and the control stays on

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.54.0, amended in v0.55.0

## Context

An organisation policy is all-or-nothing per node. To let ONE service account create ONE
key, or to let Security Command Center activate, the policy has to be lowered for a whole
project or folder and raised again afterwards — a lift WINDOW, during which nothing is
enforced for anything under that node, and which nobody remembers to close. Two real
cases forced one: SCC activation, and the domain-restricted-sharing service-agent caveat.

A Resource Manager TAG replaces the window. Unlike a label it is IAM-governed, and an org
policy rule can CONDITION on it, so the policy reads "enforced everywhere except where
this tag value is bound". Google ships this themselves: since 2024 a new organisation gets
`iam.disableServiceAccountKeyCreation` carrying
`resource.matchTagId('tagKeys/…', 'tagValues/…')` against a built-in key, so binding
`not_enforced` to one service account exempts that account and nothing else.

Nothing in satz could express it. The provider schema fixture carried no `google_tags_*`
type at all, so no tag resource could be emitted; the IaC role table had no row for one;
and — worse — the compliance plane could not READ a conditional policy. The manifest
collapsed a policy body to one `enforce` only when it found exactly one, so a policy with
an exemption rule plus an unconditional rule yielded two values and no verdict. Under
[ADR 0013](0013-a-claim-asserts-what-its-witness-does.md) that is `PolicyEffect::Unknown`:
the claim over it stops being checked. A policy would go blind to the compliance plane
the moment it let one resource out, which is the opposite of what an exemption is for.

## Considered options

**Does a policy with N exemptions still read ENFORCED?**

1. **No — any exemption fails the control.** Honest in one narrow sense and it makes the
   mechanism unusable: the one estate that legitimately exempts a legacy connector can
   never report a clean baseline, so it lowers the policy for the folder instead and is
   worse off.
2. **Yes, silently** — the verdict ignores conditions. This is what the code did for LIVE
   policies before the conditional-rule work and it lies: the row says enforced and does
   not say to whom it is not.
3. **Yes, with the exemptions printed beside it**, and a duty on the claim naming who
   reviews them.

**Does the library ship exemptions?**

1. **Ship the ones customers commonly need**, ready to switch on. A library that ships
   convenient exemptions is a library that lowers the baseline by default, and the flag
   to turn one on is easier to find than the reason not to.
2. **Ship only the ABILITY** — the tag vocabulary — and leave every binding to the estate.

## Decision

Option 3 and option 2.

The manifest now reads `enforce` from the `spec`'s UNCONDITIONAL rules alone and records
each conditional rule as one line (`enforce OFF where <title or expression>`). This is the
same rule `live_enforcement` already applied to a policy read back from the organisation,
so the declared and the live side answer the question the same way. `require` prints each
exemption under its control (`↳ exempted: …`) and counts the controls that carry one;
`report-compliance` was already listing the live ones beside the live verdict.

One CIS constraint ships exemptable: `iam.managed.disableServiceAccountKeyCreation`
takes its rules from the baseline param `cis_sa_key_creation_rules`, defaulting to the
plain enforcing rule, so an estate rebinds that one param instead of forking the pack.
It is the only one, because a rules param per constraint would put forty list-of-object
blocks into every estate's `terraform.tfvars` for a case nobody has, and it is THAT one
because it is the constraint organisations actually have to let a single principal out
of — Google ships their own built-in exemption tag for it. Another constraint earns a
param the same way: a real organisation that needed it.

`presets/exemptions/exemption-tag.satz` ships the vocabulary: one organisation tag key
`<shortname>-exemption` with the values `enforced` and `not_enforced`, bound to nothing.
The `_enforced` value exists so a binding can be MOVED back rather than deleted, which
leaves a trace. The binding that exempts a resource, and the condition on the constraint
that honours the tag, are the estate's to write — the pack's header shows the exact shape
of both, and `tests/iac/exemption-tag/` compiles them.

The three tag types joined the schema fixture and the IaC role table. The binding has its
own row (`roles/resourcemanager.tagUser`) rather than sharing the key's
(`roles/resourcemanager.tagAdmin`): an estate may be allowed to define the vocabulary
without being allowed to hand out exemptions with it.

The pack's question carries the thing that usually ends the conversation, and it belongs
in the record too: **does the consumer need a key at all?** A workload inside Google
Cloud, a Cloud Run service, and external CI on GitHub or GitLab can all use Workload
Identity Federation or impersonation and leave the control intact. The tag route is the
exception with a named owner, never the default.

## Consequences

- A conditional org policy is legible to the compliance plane for the first time. Before
  this, adding an exemption silently removed the policy's verdict.
- An exemption is visible in three places: the estate that declares the binding, the goal
  view that prints it under its control, and Cloud Asset Inventory, which answers the
  org-wide question — `gcloud asset search-all-resources --query='tagValues:…'`.
- **CORRECTED 2026-09-13, before this record was a day old.** This consequence first
  read: "the CIS constraints cannot carry a condition without a language change — a param
  interpolates a VALUE, not a structural list, and `rules = {param}` is a parse error."
  The parse error was real and the conclusion drawn from it was wrong. `{param}` is
  OBJECT syntax; a param reference in value position is a BARE identifier
  (`Value::Ref`), and `rules = cis_sa_key_creation_rules` bound to a list of objects
  already emits repeated `rules` blocks, condition blocks included. No language change
  was ever needed, and the amendment above ships the constraint that wanted it.
  The lesson is the cheaper half of the same rule that made this record worth writing:
  a gap claimed against a parser is checked against the parser, in both spellings,
  before it reaches a decision record — a wrong blocker in an ADR is how a language
  change gets proposed for a problem that does not exist.
- An estate that wants a DIFFERENT shipped constraint to honour the tag still has to
  `suppress` the pack's policy and declare its own, which is a fork. The fix is one more
  rules param on that constraint, not a mechanism.
- The library ships no exemption. The two Google-forced cases (SCC activation, the
  domain-restricted-sharing service agent) are already handled where they arise — the
  CIS pack passes the service agents through `allowedMemberSubjects` — so neither needs
  a tag today.
