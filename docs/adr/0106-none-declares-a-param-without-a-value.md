# 0106 — `none` declares a param without a value, and agrees with one default

- **Status:** accepted
- **Date:** 2026-11-25
- **Amends:** [ADR 0104](0104-two-packs-declaring-one-param-must-agree.md) (which declarations agree)
- **Amended by:** [ADR 0108](0108-a-none-param-takes-its-shape-from-where-it-is-read.md) (the `shape` of a `none` param is read from where it is read)

## Context

A pack declares every param it reads (`docs/language.md` §6.3). Satz had no way to say
"declared, and this file proposes no value": the library wrote `""` for it, 32 times, and
`[]` for a list. Both are real values too. `infra_folder_name = ""` bound in an estate means
"no folder"; `allowed_api_key_services = []` means "no service may be targeted". The
question report guessed which was meant from the pack's own declaration
(`restates_unknown`, `derived_default` in `src/questions.rs`), `satz init` wrote `""` for
what it could not derive and the guess had to undo it, and a number or a bool param could
not be left without a value at all.

ADR 0104 made it worse: two used files that declare one param with different defaults are
refused. A pack that only reads `region`, and declares it `""` because something must
provide it, disagreed with the pack that declares `region = "eu"` — although it never meant
a value.

## Options

**A. A keyword `none`** as a param's value: `region = none`. Declared, no value proposed.
It agrees with any one default another file gives, which is then the value; two real
defaults still disagree. A param every declaration leaves `none`, bound by nothing and read
by what the transpile writes, is refused naming its declarations and the read — unless a
question asks it, when it is `blocking` and the gate refuses as for any open question.
`""` and `[]` become ordinary values everywhere. Costs a keyword in the language, a
library migration of the defaults that meant "must be provided", and one representation
the walks carry until a file gives the value. Valid for any type, because it has none.

**B. A keyword `required`.** The same mechanism under another word. It reads wrong where
the declaration is compatible with another pack's default: `region = required` beside
`region = "eu"` says the estate must bind it, when the other pack already provides it. The
word describes one outcome — nobody gave a value — not what the declaration says.

**C. Keep `""` as "no default".** No language change. Keeps the ambiguity with a real empty
value, keeps the guessing in the question report and in `init`, gives a number or a bool no
way to be left open, and keeps ADR 0104 refusing `""` against `"eu"` or teaches it that
`""` is special, which makes `""` a value in one place and no value in the next.

**D. A typed declaration** (`region = none string`, or a type statement per param). Would
give the interview a shape for a param no file gives a value. Costs a type system for
params that nothing else uses: every other param's type is its value's. The shape an
answer needs comes from another file's default, else from the question's `recommend`; a
param with neither is answered as text.

## Decision

A. `none` is a param's value in `params { … }` and nowhere else: refused as an input's
value (an input carries its example value), as a local value's (nothing binds a local
value), as a contribution, and as the name of a param, an input or a local value. Read
anywhere else, `none` is a bare name, and no param carries it.

- **Agreement (amends ADR 0104).** `none` agrees with any default and with `none`. Two real
  defaults that differ are refused as before; `""` and `"eu"` are two real defaults.
- **The value.** The estate's binding wins; else the one default another file gives, from
  the start of the walk — a file walked before the default's reads the default; else none.
- **No value, read.** A read of a param no file gives a value — in a resource, a param's
  default, an export, an action, a suppression, a `when`, an `each`, a gate — stops the
  transpile at the read, naming every declaration. Where a question that applies asks the
  param, the estate is being answered: `satz questions` reports it `blocking`,
  `transpile --plan` warns, `transpile --apply` and `bootstrap` refuse, and the plain
  transpile reads it as the empty value — `""` where it is the whole value, nothing inside
  a text, a false condition, an empty list — which is what it wrote for a blocking `""`
  before. A param with no value is never in `variables.tf` or `terraform.tfvars`.
- **An estate's `none`** is allowed and binds nothing: the packs' default applies, and the
  question stays open. It is what `satz init` writes for a value it could not derive — the
  line is there to edit, and nothing reads it as an answer. Refusing it was the other
  option; it would leave `init` with no way to write that line except `""`, which is an
  answer.
- **Questions.** Answered still means the estate's own `params { }` binds a value. A `none`
  param a question asks is `blocking` unless another file gives a default, which is then
  offered. A pack's `""` or `[]` is a default like any other: offered, and taken by
  `--accept-defaults`. `empty = "…"` says what `""` means; it no longer decides whether
  `""` is an answer.

## Consequences

- The library declares `none` where `""` or `[]` meant "must be provided" and keeps `""`
  where empty is a choice (a folder param whose empty value is the organization, an
  address whose empty value is no channel); `presets/CHANGELOG.md` lists each pack.
- `satz questions` JSON: `default` is absent for a param no file gives a value, and present
  — `""`, `[]` — where a pack's default is empty. `shape` is the shape of another file's
  default or of the question's `recommend` for a `none` param, absent without either.
  ADR 0108 replaces the second half: a param no file gives a value takes the shape of
  where it is read, else of its `recommend`.
- `review-preset` and `doc-presets` refuse a `none` param no question in its file asks; a
  `""` param without a question is a default like any other.
- `satz init` writes `none` for a value it could not derive; `bootstrap --greenfield` fills
  a `none` organization id.
- The grammar reads `none` as an identifier already: no grammar change.
