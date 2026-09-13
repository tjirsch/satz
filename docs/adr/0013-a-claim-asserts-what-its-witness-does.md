# 0013 — a claim asserts what its witness does, not that it exists

- **Status:** accepted
- **Date:** 2026-09-13
- **Shipped in:** v0.54.0

## Context

`claim "cis-gcp" "4.0" "3.1" implements { resources = [...] }` means "this control is
discharged, and here is the proof". Until now `require` checked one thing about that
proof: that every address in `resources` appears in the emission manifest. Existence
was the whole test.

An org policy is not discharged by existing. `spec { rules = [{ enforce = "FALSE" }] }`
emits a `google_org_policy_policy` that does nothing, and so does
`spec { reset = true }` — the form the library itself uses to declare a superseded
legacy twin OFF. Either one satisfied the claim. An estate could read

    ✓ 3.1   Default network does not exist   — google_org_policy_policy.compute_skipDefaultNetworkCreation

with that policy switched off in the same file. The goal view is what a customer reads
before an audit, and it said the control was met.

The route into that state is ordinary: a `.local` fork of a pack (the supported way to
customise past what params allow) edits a policy body and leaves the claim, which lives
in the same file, untouched. Nothing in the tree objected. `report-compliance` would
eventually contradict it — it compares org policies by value against the live
organisation and reports NOT ENFORCED — but only after an apply, against a real org,
with credentials. The declared estate had already passed.

The mirror case is a `deviates` claim over a policy that enforces. A deviation is a
disclosed non-conformance with a written reason, and an auditor reads it as accepted
risk. Over an enforcing policy it discloses something that is not there. Less dangerous
than a false `implements`, and just as false.

## Considered options

1. **A new claim syntax that spells the assertion out** — `implements {
   resources = [...], asserts = { "google_org_policy_policy.p": { enforce: true } } }`.
   Explicit, and every claim in the library would have to carry it. 80-odd claims would
   restate what their coverage word already says, and a claim written without it would
   be back to the existing behaviour — an opt-in check nobody opts into.
2. **A transpile-time refusal.** Catch it in `pipeline_b_generate` beside
   `report_iac_roles`, so a contradicted claim fails the compile. Earliest possible, and
   wrong in one common case: an estate MID-EDIT — a policy being switched off before
   its claim is rewritten — could not compile at all, and neither could an estate that
   deliberately ships a policy off under a `contributes` claim while another pack
   enforces it.
3. **The coverage word IS the assertion, checked in the goal view.** `implements` means
   the witnesses enforce; `deviates` means they do not; `contributes` asserts nothing
   about the value. No new syntax, no per-claim opt-in, and every claim in the library
   is covered the day it ships.

## Decision

Option 3. `require` reads the effect of every emitted `google_org_policy_policy` from
the manifest and judges each claim against its own coverage word. A claim whose
witnesses contradict it resolves to `Goal::ClaimContradicted`, rendered `‼ CONTRADICTED
CLAIM`, counted in the summary and failing the exit code beside unmet and broken.

Where the effect has no single answer, no verdict is given. The manifest collapses a
policy body to one `enforce` only when exactly one is declared; a list constraint
(`allowed_values` / `denied_values`) leaves nothing behind, and a multi-rule policy
yields nothing. All of those read `Unknown` and are never contradicted — the manifest's
own rule, that no verdict beats a wrong one, applies here unchanged.

The verdict outranks everything else on its control, including witnesses another
included claim supplied. A control the estate claims and switches off is the one thing
the reader must not be able to miss, and averaging it against a second claim's evidence
is how it would be missed.

## Consequences

- Existence is no longer sufficient proof for an org-policy witness. A pack that ships
  a policy off under an `implements` claim now fails its own gate — which is the point,
  and which is why the shipped library reports zero contradicted claims: its
  `reset = true` twins are declared under `-superseded` addresses that no claim names.
- `satz require` can now fail an estate that used to pass, so the release is a MINOR
  under [ADR 0010](0010-the-minor-version-marks-an-upgrade-that-brings-work.md).
- Only `google_org_policy_policy` is judged. For every other witness type — a sink, a
  bucket, a custom constraint — existence IS the effect, which the witness check already
  tests.
- A tag-conditional exemption (a rule with a `condition`) adds a second rule to a
  policy, so the manifest sees more than one `enforce` and the effect reads `Unknown`.
  That is safe but blunt: the claim stops being checked exactly where the estate got
  more interesting. Making conditional rules legible to the goal view means giving the
  manifest rule-level structure, which is its own decision and is not taken here.
