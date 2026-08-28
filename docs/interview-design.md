# Interview layer — design sketch (Phase 3)

Status: **sketch for review, nothing built.** The roadmap says design before the
question format, for a reason recorded by the owner:

> answers are often "we'll decide later" — and that is the normal case, not an
> edge case

Everything below follows from taking that literally.

## 1. The problem, stated precisely

An interview that produces an estate is not a questionnaire. A questionnaire
assumes answers exist. Here the good questions — "do you have a naming concept?",
"who owns billing?", "which domain do contacts live on?" — are exactly the ones a
customer defers, and we cannot hold up a rollout waiting for them.

So the layer has to answer a harder question than "what did they say":

**What is safe to decide without them, and what does that decision cost to undo?**

Get that wrong in the cheap direction and you block on trivia. Get it wrong in
the expensive direction and six months later a rename is a state-mv plus a
mail-flip across every group in the org.

## 2. Two costs, not one

The design's load-bearing distinction. Every answer has two independent costs,
and conflating them is the mistake to avoid:

**Reversal cost — what does changing the answer do to the estate?**

| | example | cost |
|---|---|---|
| edit | org-wide `roles/viewer` for the security group | one line, one apply |
| state surgery | group naming scheme | rename + `state mv` + mail flips |
| recreate | `customer_shortname` → project and bucket ids | destroy/recreate, ids are globally unique |

**Blast radius — what does changing the answer do to the running org?**

`compute.managed.requireOsLogin` is a single boolean. Reversal cost: trivial. Blast
radius: it cuts existing SSH access patterns, which is exactly why estate 5 does not
enforce it. Cheap to change, expensive to have changed.

These are orthogonal. A question is only safely deferrable when **both** are low.

## 3. The rule that falls out

> A question may be deferred only if its default is cheap to reverse **and** cheap
> to have wrong. A one-way door must be answered before `derive` will emit.

That is enforceable, not advisory. `derive` refuses on a deferred one-way door and
names the cost. It is the whole point of the layer: the tool knows which questions
are load-bearing, so the human does not have to remember.

Deferral therefore becomes a **first-class answer**, recorded as such — not an
absent key. Three states, always distinguishable afterwards:

- `answered` — the customer decided
- `defaulted` — we chose, they did not object (and may not know)
- `deferred` — explicitly "later", with the safe default in force meanwhile

The difference between `defaulted` and `deferred` matters at review time: the
first is a decision nobody examined, the second is a decision consciously
postponed. Today both are invisible.

## 4. Questions are language syntax, next to the pack

Owner constraint: question modules ship beside the packs they configure, so
content growth is additive. Taken one step further — **questions belong IN the
pack**, exactly as claims do.

Claims already work this way and it has paid off twice (R5 read them straight from
the front end; R6 compared them from source). A pack that declares its own
questions has one source of truth and cannot drift from a sidecar.

A question answers a **param** — the customization channel that already exists.
Sketch:

```
params {
  customer_shortname = ""            // no default: must be answered
  enforce_os_login   = "TRUE"
}

question customer_shortname {
  prompt   = "Short name identifying this customer"
  why      = "Derives project ids, bucket names and group prefixes."
  reversal = recreate     // ids are globally unique; changing this recreates them
  blast    = none
}

question enforce_os_login {
  prompt   = "Enforce OS Login org-wide?"
  why      = "CIS 4.4. IAM governs SSH instead of metadata keys."
  reversal = edit
  blast    = high         // cuts existing SSH access patterns on running VMs
  defer_to = "TRUE"       // safe default while deferred
}
```

`reversal` ∈ `edit | state-surgery | recreate`; `blast` ∈ `none | low | high`.
`derive` refuses a deferred question whose `reversal` is `recreate` or whose
`blast` is `high`, and says why.

**Open:** whether `blast: high` should block deferral or merely warn loudly.
Enforcing OS Login by default is the *compliant* choice and the *disruptive* one.
Leaning: warn, require an explicit acknowledgement, record it as `defaulted` —
because refusing to emit would make the CIS pack unusable without a full
interview, which defeats the purpose.

## 5. What `derive` emits

Answers → a generated estate, then the normal pipeline. No new engine.

Real estates are not wholly derivable — estate 1 has `vertex-hub.satz` and
`projects-user-folders.satz`, hand-written and customer-specific. So `derive` must
not own the whole file. Mirror the split that already works for presets:

```
yaml/C0xxxx.derived.satz     # generated, never hand-edited
yaml/C0xxxx.satz             # use "C0xxxx.derived.satz" + hand-written extras
```

Regeneration overwrites only the first. Same discipline as pristine vs `.local`,
and the same failure mode is prevented: a regeneration can never silently eat
hand-written work.

## 6. The lockfile

`answers.lock.yaml` records, per question: the answer, its state
(answered/defaulted/deferred), the pack **and pack version** the question came
from, and when. That buys three things:

- **`check-derived` as a CI gate** — re-derive, fold, compare the IR. Not text:
  the folded IR, so formatting churn cannot fail the build. The machinery exists.
- **Deferral follow-up** — when a deferred answer finally arrives, the tool can say
  what re-derives and what it costs, from the `reversal`/`blast` it already knows.
- **Upgrade honesty** — a pack that adds a question makes every lockfile that
  predates it visibly incomplete, instead of silently defaulting.

## 7. The fourteen estates that already exist

Phase 3 must not orphan them, and they are the best test data available: fourteen
real answer-sets, already known-good.

Proposed direction: `check-derived` runs backwards too. Given an existing estate,
assert that a derived estate from a candidate answer-set folds to the same IR. If
it does, the interview can reproduce reality — which is a far stronger validation
than any synthetic fixture. Where it does not, the gap names a missing question.

That is also the honest way to discover the question set: **do not invent it —
read it off the fleet.** Every estate param override that exists today is
evidence that a question was needed. The `.local` forks are evidence of questions
the params could not express.

## 8. Deliberately out of scope

- **LLM-assisted exploration.** The roadmap has it as post-stabilisation and that
  is right. The deterministic core must exist and be trustworthy first; the agent
  proposes, the deterministic layer disposes.
- **A UI.** Answers are a file. A file is reviewable, diffable and CI-gateable;
  a wizard is none of those.

## 9. Open questions for the owner

1. **`blast: high` — block or warn?** (§4). My lean is warn + acknowledge.
2. **Is `reversal` per-question or derivable?** A param feeding a globally-unique
   name is always `recreate` — the tool may be able to infer it from where the
   param is used rather than trusting a hand-written annotation that can rot.
3. **Where do cross-pack questions live?** `customer_shortname` is not owned by
   any one pack. A `core` question module, or the estate itself declaring them?
4. **How many questions is the real target?** The roadmap says 6 for Cloud Cockpit
   core. Reading them off the fleet (§7) will produce a number — worth checking
   whether it is 6, 16, or 60 before committing to the format.
