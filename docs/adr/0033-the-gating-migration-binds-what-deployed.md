# 0033 — the gating migration binds what deployed

- **Status:** superseded by
  [ADR 0041](0041-satz-carries-no-code-that-rewrites-an-estate-for-a-breaking-change.md):
  `merge-presets` gates no line and binds no gate. What this record found about the edit —
  bind the gate `true` where the line deployed, a default included, keep a follower at its
  value, bind the other option of a choice false — is the hand edit in `presets/README.md`
  under `## Breaking changes`
- **Date:** 2026-09-19
- **Shipped in:** the release that follows

## Context

ADR-0031 gave every pack but `estate-core` and the map a gate, and ADR-0032 made the
compile report an active line of a gated pack written without `when <gate>`. Estates
older than the map adopted their packs as plain `use` lines. Their answers switch
nothing: a gate answered `false` beside an ungated line leaves the pack deploying, and
the next apply after someone "fixes" the answer by hand, or after a tool that trusts the
answer writes the line gated, destroys what the estate runs. The test organisation's
estate had nine such lines, four of their gates answered `false` and one unanswered.

`merge-presets` is the pickup every estate runs, so the migration belongs there. What it
does with the answer was decided on 2026-09-18: bind the gate `true` where the line was
active. This record keeps the reasons and the three consequences the implementation
found.

## Considered options

1. **Gate the line and bind the gate `true` where the line was active; report every
   flip, loudly where an explicit `false` is overwritten.**
2. **Refuse and list every line for a hand edit.**
3. **Gate the line and keep the answer**, `false` included.

## Decision

Option 1. The emission does not move, so the pickup stays a no-op for `tofu plan`, and the
answer now says what the estate deploys; a customer who meant the `false` runs
`remove-pack`, which the report names.

- **A default that is already true is bound too.** A `when` is checked where the compile
  meets the line, and an estate older than the map often uses the map below the folder
  and resource-map lines it gates; the gate is then unknown at that line. Binding it in
  the estate's `params` makes it known everywhere.
- **What follows a bound gate keeps its value.** `use_sentinel_auditlogs = use_sentinel`
  would switch on with its leader; an unbound follower is bound to the value it had. The
  other option of a choice held true is bound false.
- **The proof admits the tfvars lines of the bound gates.** Every param is a variable
  with a value in `terraform.tfvars`, so a flipped answer changes that file. The
  transpile-identity check that guards a fork repoint requires `main.tf`, `imports.tf`
  and `variables.tf` identical, `terraform.tfvars` identical but for the bound gates, and
  no emitted resource reading `var.<gate>`; anything else rolls the run back.
- **A contradiction is refused before anything is written:** two packs that exclude one
  another on two gates, both deploying. Binding both gates true would answer one choice
  two ways. The two spellings of one security model on one gate are no contradiction and
  are gated together.
- **A commented line is never touched**, and a line whose gate the used copy of the
  declaring file lacks — an old fork of the map — is named and left, which needs
  attention.

## Consequences

- An estate that answered `false` beside a deploying line exits `merge-presets` non-zero
  once, with `ANSWER CHANGED` per gate; the second run finds nothing.
- `terraform.tfvars` changes in the pickup commit for each flipped answer; the plan does
  not.
- A CIS baseline fork still keyed inside a `google_org_policy_policy { … }` block is
  gated where it is and reads `misplaced` in `satz packs`; lifting it is a hand edit.

## Pros and cons of the options

### 1 — bind `true` where the line deployed *(chosen)*

- **Good:** nothing is destroyed and nobody edits every estate's lines by hand.
- **Bad:** it overwrites an answer the customer gave; the report is the only place that
  says so.

### 2 — refuse and list

- **Bad:** every fleet pickup stops until someone edits each line.

### 3 — keep the answer

- **Bad:** the next apply destroys what the estate deploys.
