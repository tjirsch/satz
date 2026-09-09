# 0005 — an answer is a param the estate binds; nothing may be left unanswered

- **Status:** accepted
- **Date:** 2026-09-09
- **Deciders:** the maintainer

## Context

A pack declares `question` blocks next to the params it needs answered: what to ask,
why, and what changing the answer later costs. The mechanism shipped in v0.46.76. An
interview — an agent asking a customer those questions and writing the answers into
the estate — was planned on top of it, and stalled on one design question: **what is a
deferred answer?**

The design sketch that preceded the language work proposed a lockfile
(`answers.lock.yaml`) recording which questions had been asked, a `derive` step
generating a `.derived.satz` from the answers, and a `check-derived` gate. It also
proposed that a question could be *deferred* — "we'll decide later" — with one-way
doors refusing deferral.

Meanwhile `satz questions` could not say whether a question was answered. It read the
param's value through the fold, where a pack default is indistinguishable from an
estate's decision, and reported `defaulted` for everything with a non-empty value.

## Decision

**An answer is a param the estate's own `params {}` binds.** To the pack's default or
to anything else — accepting a default is an answer, and it is recorded by writing the
default into the estate. A question whose param the estate does not bind is
`unanswered`. There is no third state.

**Every question must be answered before the estate touches an organisation.**
`bootstrap` and `transpile --apply` refuse while any question is unanswered, naming
them; `transpile --plan` and `bootstrap --dry-run` warn. An estate may be transpiled
and revised in as many passes as it takes — the gate is on the irreversible step, not on
looking.

**A question with no usable default blocks until a value is typed.** A directory
customer id, a billing account, a domain — the pack can name the question but cannot
guess the answer. The report carries `blocking` for these so an interview knows which
questions it can offer to settle with one acceptance and which it cannot.

**A question whose `ask_when` param is false is `not-applicable`** and counts toward
nothing. The field existed and was parsed; nothing consulted it.

The `derive` / lockfile / `check-derived` layer is **not built**. The estate file is the
answer record.

## Consequences

- No new syntax. A marker for "the human looked at this and accepted it" would have been
  a second channel for the same fact; the binding is the marker.
- `satz questions --unanswered` is the interview's worklist; `--format markdown` is the
  human's decisions sheet — "these are your decisions, shall we start?".
- The MCP tool `satz_interview` returns the same report filtered, and can create the
  estate file the interview writes into, so an interview may begin before anything
  exists. The agent asks; the human decides; the agent writes the param; the next call
  shows one fewer.
- `presets/estate-core.satz` carries the seventeen params `satz init` writes, each with
  its question, so an interview has something to ask on day 0. `init` itself is
  unchanged: given its parameters it has answered everything, and an estate it writes
  binds every param. Two paths to the same place, both complete.
- Existing estates are untouched: no pack declares a question today, so nothing is
  unanswered anywhere until a pack gains one — and then the fleet sweep shows exactly
  which estates must answer it, which is the migration.
- **Not gated:** bare `satz plan` / `satz apply`. They run the tool in `hcl_dir` and know
  no estate. The gate sits on the two commands that do. Stated here rather than
  papered over.
- **Not built:** renaming the interview's file once the customer id is known. The file
  is created under the name the agent gives it; `git mv` and the `estate` line are one
  edit at the end.

## Pros and cons of the options

### Deferred state, lockfile and a derive step

- **Good:** "we'll decide later" is a real thing a customer says, and it is written down.
- **Good:** the lockfile can record who answered, when — provenance the estate lacks.
- **Bad:** three artefacts for one fact — estate, lockfile, derived file — with the
  derived one generated and never hand-edited, beside an estate that is. Which one is
  the truth is a question every reader would ask.
- **Bad:** a deferred one-way door is a decision nobody made, carried into an apply.
  The maintainer's rule is the opposite: all questions answered, or no organisation.
- **Bad:** `defaulted` — a value present and nobody asked — is exactly the state that
  produced a project pointed at a bucket nothing creates. Making it a first-class state
  legitimises it.

### A marker in the language

- **Good:** explicit; a reader sees `accepted` and knows a human looked.
- **Bad:** two ways to say the same thing — `x = default` versus `x = default accepted`
  — and every consumer has to treat them as equal. A second form of one fact.

### The binding is the answer *(chosen)*

- **Good:** no syntax, no second file, no generated twin. The estate is the record.
- **Good:** the gate is one boolean over one report.
- **Good:** an estate written by `init` with its flags is complete by construction.
- **Bad:** a pack with many params makes an estate that accepts every default long —
  every accepted default is a line. That is the cost of being able to tell "accepted"
  from "never looked", and it is paid in the file rather than in a lockfile.
- **Bad:** it cannot record who accepted, or when. Git can.
