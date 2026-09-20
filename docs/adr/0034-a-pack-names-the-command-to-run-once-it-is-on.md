# 0034 — a pack names the command to run once it is on, and the estate acknowledges it by binding a param

- **Status:** accepted; its `before = apply` half is superseded by ADR 0042, which replaces the key with `severity`
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

Switching the CIS org-policy packs on brings one step with them that nothing in the
language could say: `satz adopt` has to run before the first apply. Google sets several
of those policies on every new organisation, so an apply that creates a policy which
already exists stops on `409 POLICY_ALREADY_EXISTS` — one resource at a time. The run
that prevents it writes the live ids into the state; it is forgotten because nothing
asks for it.

The mechanisms that existed do not fit:

- an `action` EXECUTES through `run-actions`, and warns on every compile whether or not
  anything is outstanding;
- `update-prerequisites` writes its own fix, and there is nothing here to write;
- a `question` is a customer decision — an operator's task in the decisions sheet would
  be answered by the wrong person, and it would count toward the completeness gate;
- keys on the gating question would only reach map-gated packs, and the CIS extensions
  are gated by the baseline.

Whether the step has been taken cannot be judged offline either: `adopt` refuses to write
`"import-id"` into a pristine pack, so the proof of an adoption lives in the tofu state,
not in the estate.

## Decision

**A pack carries a `notice <param> { text run before }`.** satz shows it when the pack is
switched on — a yes in the interview, `satz add-pack`, `merge-presets` bringing the pack
in — and keeps warning at the estate's `use` line until the estate acknowledges it.

**The acknowledgement is the estate binding `<param> = true`**, in its own `params {}`,
exactly as an answer is (ADR-0006). Git records who and when, a second operator and CI
read the same state, and there is no state file beside the estate — one the privacy gate
would reject in a customer's repository anyway.

**`before = apply` holds back the two commands that change an organisation**:
`transpile --apply` and `bootstrap` refuse while such a notice is open, `--plan` and
`--dry-run` warn. The same two-speed rule as the questions gate (ADR-0006) and the
prerequisites (ADR-0023). *(Superseded by ADR 0042: the notice declares
`severity = error` instead, and the refusal covers every command that writes to the
organisation.)*

**`satz adopt --execute --import` acknowledges the notices that name it** when the run
covers every resource type and finishes with nothing unresolved: it binds their params
itself, so the operator who followed the notice is not asked to record it twice.

**The param is the notice's alone and is never emitted.** The pack declares it `false`,
no question asks it, no other library file declares or reads it, and it is in no
`variables.tf` and no `terraform.tfvars` — `satz pack-graph`'s check 9 refuses a library
that breaks any of that. A notice also has its own canonical form, beside the questions'
and the offers' (ADR-0031), so a pack that gains or rewords one is reported by
`check-presets` and never forks an estate.

**One notice per org-policy pack** — the baseline and each extension, dry-run twins
included — because each is adopted when it is switched on.

## Consequences

- An estate that deploys a CIS org-policy pack has one new param to bind after its next
  `merge-presets`, and its `apply` refuses until it does. That is the migration, and it
  is why the release is a MINOR one.
- Binding the param changes nothing in the emitted HCL, so the pickup stays a no-op for
  `tofu plan`.
- The interview gained no question and the decisions sheet no row: an operator task and
  a customer decision stay apart.
- A notice is shown once per switch. An estate that never acknowledges it is reminded by
  every compile and refused by every apply, which is the point; `satz packs` says which
  packs are waiting.
- satz-studio shows the same notices as a dialog, from `satz_interview` and
  `satz_add_pack`, and acknowledges through satz. It derives nothing of its own.

## Pros and cons of the options

### A `notice` statement acknowledged by binding its param *(chosen)*

- **Good:** the record is in the estate, in git, visible to every other reader.
- **Good:** it mirrors `question` — one more thing a pack declares beside its params,
  with the same acknowledgement mechanics and none of its meaning.
- **Bad:** a new statement in the language, and one more param in the estate per pack.

### An attribute on `action`

- **Bad:** `run-actions` would try to execute it, and the compile warns about every
  action whether or not anything is outstanding.

### A `question` kind

- **Bad:** it pollutes the decisions sheet, which a customer signs, with an operator's
  task, and the completeness gate with something no answer settles.

### Keys on the gating question

- **Bad:** only a map-gated pack could carry one; the CIS extensions are gated by the
  baseline and would be left out.
