# satz interview

How an estate gets its answers when nobody arrives with all of them: the questions the
packs declare, asked one at a time, each answer written into the estate as it is
given, and a gate that keeps the estate off the organisation until nothing is open.

There are three ways to start an estate, and they end at the same file:

| | who answers | how |
|---|---|---|
| `satz init --customer-id … --customer-shortname … --billing-account-infra …` | whoever runs it, all at once | every answer is a flag; the estate binds every param and is complete on arrival |
| `satz interview <estate> [--create]` | a person at a terminal | one question at a time, the default in brackets, Enter accepts it |
| `satz_interview` over MCP | a customer, through an agent | the agent asks in its own words, the human decides, the agent writes the param or passes it as an `answer` |

`init` is for the operator who knows the seventeen values. The other two are for the
conversation that produces them — and for every question a pack adds later, which
`init` has no flag for.

## What a question is, and what an answer is

A pack that needs a param decided declares a `question` beside it — what to ask, why,
and what changing the answer later costs ([the language reference](language.md#questions)
has the syntax). The estate uses the pack; the questions come with it.

**An answer is a param the estate's own `params {}` binds.** To the pack's default or
to anything else. Accepting a default is an answer, and it is recorded by writing the
default into the estate — so a reader of the file can tell "decided" from "never
looked", which the fold alone cannot, because a pack default and an estate's decision
resolve to the same value there.

The states `satz questions` reports:

| state | meaning |
|---|---|
| `answered` | the estate binds the param (for a `oneof`: binds one of its options) |
| `unanswered` | it does not. The row carries the `default` the pack offers, if one is usable |
| `not-applicable` | the question's `ask_when` param is false; not asked, not counted |

and one flag: **`blocking`** — unanswered *and* no usable default. A directory customer
id, a billing account, a domain: the pack can name the question, it cannot guess the
answer. These need a typed value.

A default is usable when it is not empty and not built from something still open.
`infra_project_name` defaults to `"{customer_shortname}-infra-001"`; with the short
name unanswered that is `-infra-001`, which is a string and not a default, so the
project name blocks — until the short name is typed, when it offers `acme-infra-001`.
Ask in the pack's order and the derived names arrive as offers.

## The gate

**Every applicable question must be answered before the estate touches an
organisation.** `bootstrap` and `transpile --apply` refuse while one is open, naming
it; `bootstrap --dry-run` and `transpile --plan` warn instead, because looking is how
you find out. An estate may be transpiled, checked and revised in as many passes as
it takes — the gate is on the irreversible step.

```
bootstrap refused: 3 question(s) unanswered — customer_id (needs a value),
billing_account_infra (needs a value), default_region. Every question must be
answered before the estate touches an organisation. `satz questions C0example.satz
--unanswered` lists them with their defaults; write the answer (or the default) into
the estate's params.
```

`summary.complete` in the JSON report is the same boolean. Bare `satz plan` and
`satz apply` are not gated: they run the tool in `hcl_dir` and know no estate.

## The day-0 questions: `presets/estate-core.satz`

`init` writes seventeen params from its flags. The pack `estate-core` carries the same
seventeen, each with its question, so an interview has something to ask before an
estate exists. Seven have no possible default and block until typed; the rest offer
one — the two derived names once the short name is in. It also asks which
security-group model the customer runs — S1, or S2 with a separate network-admins
group — as two booleans and a `question oneof`, because a question that gates a pack
cannot live in the pack it gates. The estate carries the matching lines:

```satz
use "presets/estate-core.satz"
use "presets/security-group-models/s1-security-groups.satz" when security_model_s1
use "presets/security-group-models/s2-security-groups.satz" when security_model_s2
```

The pack emits nothing. An estate written by `init` does not use it and does not
need to: given its flags, `init` has answered everything.

## At the terminal: `satz interview`

```bash
satz interview yaml/new-customer.satz --create
```

`--create` writes the estate first when it does not exist: an empty `params {}`, the
three `use` lines above, and the same day-0 resources `init` writes — the folder, the
project, the state bucket, the IaC group and service account. Then the interview:

```
17 open question(s): 8 have a default, 9 need a value.
Accept all defaults now and answer only those 9? [Y/n] — n goes through every question
> y
  accepted 8 default(s).

── estate_core ──
The questions every estate has to answer on day 0, with the params they answer.

customer_id — The Google Workspace / Cloud Identity directory customer id (C0…)
  Every group in the estate is created under this customer. There is no changing it afterwards; a different id is a different estate.
  changing it later: recreate · blast high  ⚠ one-way
  no default — a value is needed
  > C0example
  ✓ customer_id = "C0example"
…
customer_shortname — A short name for this customer — lowercase, no spaces
  > acme
  ✓ customer_shortname = "acme"

infra_project_name — The infrastructure project id
  A project id is immutable. Changing it creates a second project and abandons the first, with the state bucket inside it.
  changing it later: recreate · blast high  ⚠ one-way
  [acme-infra-001] >
  ✓ infra_project_name = "acme-infra-001"
```

- The pack's own description opens its section; a question shows its prompt, its
  `why`, and the cost of changing the answer later. A one-way door is marked.
- **Enter** accepts the default in brackets. A `oneof` lists its options numbered,
  the default marked; answer with the number.
- `skip` leaves a question open and moves on; `q` stops. Nothing is lost: every
  answer was written when it was given, and the next run asks what is still open.
- `--accept-defaults` skips the opening offer and accepts them; `--all` re-asks
  answered questions too, with the current answer as the default.
- A typed value takes the shape of what it replaces — a boolean stays a boolean, a
  list is comma-separated. A value with braces is refused: braces interpolate in a
  Satz string, and a customer's answer is a value, not a template.

It ends with where the estate stands, what is still open, and — once `customer_id`
is bound — the name `init` would have given the file, so the interview can start
under any name and the file be renamed at the end (`git mv`, and the `estate` line).

The client is line-mode on purpose: it reads stdin, so a piped run drives it in the
smoke matrix, and satz never calls a model — it presents, the human decides.

## Through an agent: `satz_interview`

The MCP tool returns the same report an interview needs, and can write both ends of
it. Its arguments:

| argument | |
|---|---|
| `estate` | the file; omit for the open estate |
| `filter` | `unanswered` (default) — the worklist; `all` — every question with its state |
| `create` | write the estate first if it does not exist, as `--create` does. Needs `write` |
| `answers` | `{subject: value}` to write before reporting — a param's value, or for a `oneof` the chosen option's param name. Each must name a question the estate asks; one refused answer means nothing is written. Needs `write` |
| `accept_defaults` | also write every default the report offers. Needs `write` |

The loop an agent runs:

1. `satz_interview {create: true}` on a new name → seventeen open questions, seven
   `blocking`, each with `pack_description`, `prompt`, `why`, `reversal`, `blast`,
   and `default` where one is usable.
2. Ask the human, in whatever order and words fit the conversation. Offer the
   defaults as defaults — "the project will be called acme-infra-001 unless you say
   otherwise" — and **never invent an answer to a `blocking` question or a one-way
   door.**
3. Write what was decided: `satz_interview {answers: {customer_shortname: "acme",
   security_model: "security_model_s2"}, accept_defaults: true}`. The response is the
   report as it now stands — fewer open, and the derived defaults resolved.
4. Repeat until `summary.complete`. Then `satz_transpile_check`, and hand over.

An agent with a filesystem may write the params itself instead; `answers` exists so a
client with only MCP can close the loop. Either way the record is the estate file.

## The decisions sheet

Before an organisation is touched, the customer sees what was decided:

```bash
satz questions C0example.satz --format markdown > decisions.md
```

One table per pack, opened by the pack's description: the question, the answer the
estate carries (or the default it would accept, or **needs a value**), and what
changing it later costs. It is the "these are your decisions, shall we start?" page —
and `satz questions <estate> --unanswered` is the same report cut down to what is
still open.

## Writing a pack that can be interviewed

A pack is interviewable when every param a customer must decide has a `question`,
and every question says what it costs to be wrong about. The rules, from the
[language reference](language.md#questions):

- A question lives **in the file that declares its param**. Questions are absorbed
  after the `use … when` guard, so a question that gates a pack cannot live in the
  gated pack — put it in the estate, or in a pack the estate always uses
  (`estate-core` is that pack for the day-0 choices).
- A choice between packs is **two booleans and a `question oneof`**, with the packs
  behind `use … when`. The pack's `= true` option is the default the interview
  offers; the estate's binding is the answer.
- `why` is required wherever satz will refuse or warn — on every `recreate` and every
  `blast = high` — because that is the sentence the interview reads out before a
  one-way door.
- A param whose value the pack **cannot know** defaults to `""`. That is what makes
  it `blocking`, which is what makes the interview insist.
- A param **derived** from another interpolates it: `"{customer_shortname}-infra-001"`.
  The interview offers it once the input is answered, resolved. Declare the input's
  question first, so the derived one arrives as an offer rather than a block.
- `ask_when = <boolean param>` hides a question that only applies on one branch.
- `recommend` is what the interview shows as advice when it differs from the default;
  the `params` value is what applies.

The pack's header comment is its introduction in the interview — the first paragraph
is read to the customer when the pack's questions begin. Write it for them.

## What is not built, and why

- **A deferred state.** "We'll decide later" is not recorded; the question stays open
  and the gate stays closed. A deferred one-way door carried into an apply is a
  decision nobody made — the rule is the opposite.
- **A lockfile or a derived file.** The estate is the answer record; a second artefact
  saying the same thing would be the one readers argue about. Provenance — who
  accepted, when — is git's.
- **A full-screen client.** The line-mode interview is testable from a pipe and covers
  the flow; a richer terminal UI can sit on the same report if the flow turns out to
  want one.

The reasoning is [ADR 0006](adr/0006-an-answer-is-a-param-the-estate-binds.md).
