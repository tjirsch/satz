# 0104 — two packs declaring one param must agree, and a pack's own param carries its name

- **Status:** accepted
- **Date:** 2026-11-25

## Context

Params share one namespace across the estate and every file it uses (`docs/language.md`
§6.3, rule 3): the estate declares first, then each used file in document order, and the
first definition wins. The namespace is flat on purpose — a pack's default may read
another pack's param, which is how the CIS alert pack follows the audit-logsink pack's
project without the estate binding anything — and several library packs declare the same
param with the same default, which is how they share a value.

Two packs that declare the same name with DIFFERENT defaults broke that silently. An
estate that used pack `a` (`par01 = "from-a"`) and pack `b` (`par01 = "from-b"`) got one
variable, `par01 = "from-a"`, and `b`'s resources read `a`'s value. Nothing said so. An
estate binding `par01` hid it further: the binding picks one value for both packs, and
one of them still means something else by the name.

## Options

**A. Refuse disagreeing declarations, and name a pack's own param after the pack.**
Two used files that declare one param must give it the same default, and the same
question if both ask one; otherwise the transpile stops, naming both files, their lines
and both defaults. An estate binding does not lift it. A file that uses another, directly
or through others, binds that file's param (rule 1) and is no second declaration. The
convention that keeps packs apart: a param that belongs to one pack carries the pack's
name as its prefix (`vm_zone`), a shared one carries none. Costs a rename where two packs
collide, and a convention the author has to follow; it changes nothing for an estate
whose packs already agree. Buys a loud error where there was a wrong value.

**B. Private `locals` in a pack.** A pack declares values only it reads, invisible to the
estate and to other packs, so a pack's internals cannot collide at all. Costs a new
statement, a scope rule in the walk and in every reader of params (interview, presets
report, the document layer of satz-studio), and the decision of what a `local` may read.
It removes the collisions of internals, not of a pack's contract: a value the estate
answers still has to be a param.

**C. A namespace per `use … as <name>`.** A pack used with a name gets its params under
that name (`<name>.par01`); a plain `use` keeps the flat meaning. Two instances of one
pack become possible. Costs the namespaced reference syntax, a second way to bind a param,
and a migration for every reader that lists params by bare name.

## Decision

A, now: the refusal and the prefix convention. It is the smallest change that turns the
silent wrong value into an error, and it keeps every rule an estate relies on — the flat
namespace, a pack reading another pack's param, rule 1's binding.

B and C are planned before 1.0, as separate changes with their own records. A remains
true with either: two files that declare one name in one namespace still have to agree.

## Consequences

- `docs/language.md` §6.3 rule 3 states the refusal and the convention; the errors list in
  §10 quotes both messages.
- The comparison is of the declarations as written, line numbers aside: `"{customer_shortname}-x"`
  in two packs agrees, the same value written two ways does not.
- The preset library holds two pairs that disagree, neither usable together: the
  security-group models S1 and S2 (`gcp_project_admins_roles`), which the map's
  `security_model` choice admits one at a time, and the organization's budget preset and
  the project-budget workload pack (`budget_amount`, `budget_currency`), which belong to
  two different estates. A rename of either pair under the convention is a breaking change
  of its own and is not part of this one.
