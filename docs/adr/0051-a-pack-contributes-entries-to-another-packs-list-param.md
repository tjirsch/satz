# 0051 — a pack contributes entries to another pack's list param

- **Status:** accepted
- **Date:** 2026-09-22
- **Shipped in:** the release that follows

## Context

The CIS baseline's §1.1 lock, `iam.managed.allowedPolicyMembers`, allows exactly the
principal sets and the individual subjects its `parameters` name. Everything else is
refused at the grant, and a policy an apply DECLARES is authoritative: an entry that is
live but not declared is removed at the next apply.

Two things follow, and both had already happened.

An organisation carried
`serviceAccount:billing-export-bigquery@system.gserviceaccount.com` in that policy,
added by hand when billing export was set up. The estate did not declare it. An apply of
the CIS baseline would have removed it and broken the export — silently, because nothing
in the estate says why the entry is there.

And the Defender pack carried the other half as a comment: "the agentless-scanning
service account lives in a Microsoft project, so it must be in
`allowed_policy_member_subjects` BEFORE any grant to it is applied". A prerequisite a
human reads in a pack header and copies into an estate's params by hand, per estate,
forever. Every later integration has the same problem: Sentinel, a SIEM connector, a
backup product — each needs a principal allowed, and each knows which one.

The param cannot express it. `allowed_policy_member_subjects` is one list, declared by
the CIS baseline, and Satz has no list concatenation: an estate that adds an entry
repeats the five it keeps, and the addition is then the estate's, indistinguishable from
a local exception. Switching Defender off leaves it behind.

## Decision

A pack declares, in its own `params`, a param whose name is `contributes_` followed by
the name of the list param it adds to:

```
pack billing_export version "1.0"

params {
  contributes_allowed_policy_member_subjects = [
    "serviceAccount:billing-export-bigquery@system.gserviceaccount.com",
  ]
}
```

`contributes_<param>` is not a param. It is never a `variable`, never in
`terraform.tfvars`, no `question` may ask it, and `parse` refuses it in an estate, with a
non-list value, in a file that declares `<param>` itself, and with an empty target.

Before the compile walks the estate, one pass over the same `use` graph gathers every
contribution a pack that is actually ON declares, and produces the value each target
param takes: the estate's binding — else the declaring pack's default — followed by the
contributed entries in `use` order, an entry already present not added again. That value
seeds the estate's outermost param environment, where it beats everything, so it is the
only value anything sees. Where no file the estate uses declares the target, the entries
are dropped: there is no policy to be let past, and the pack graph already carries the
requirement (`contributes_x` makes the pack NEED the pack that declares `x`, a derived
`data` edge, which `satz packs` reports and a compile finding names).

`satz packs <estate>` prints each pack's contributions, and a pack's generated page lists
them under **Contributes**, so every entry in a list has the pack that put it there
beside it.

## Options

**A well-known param name prefix, merged in a pass before the compile (chosen).** No new
statement, so the tree-sitter grammar (a separate repository, pinned by commit, and gated
by `scripts/check-grammar.sh`, which parses every `.satz` file of this checkout) needs no
release; `satz fmt` needs no rule; the canonical form that decides whether an estate's
pack copy forks already contains `params`, so a changed contribution forks as a changed
default does; `use … when` gives the on/off behaviour with no code at all. It costs a
reserved prefix — a param may no longer be called `contributes_<something>` — and the
prefix has to be learned, which is what `docs/language.md` §6.3 and the pack pages are
for.

**A `contributes` statement, or a `contributes` keyword inside `params`.** Reads better
and is self-describing. Rejected for its cost: any new syntax is an ERROR node in the
pinned grammar, so the `grammar` CI job fails until the grammar repository has a commit
and `editors/zed/extension.toml` a new pin — two repositories for one feature, with the
satz side red in between. The statement would also have to join
`STATEMENT_KEYWORDS`, the position table, `NEVER_A_TYPE_KEY`, the statement probes and a
canonical product, none of which buys anything a param name does not.

**Merge at the fold instead — a third `MergeClass` that unions list attributes at one
address.** The fold is where two packs meeting at one address is already decided, and
`MergeClass::Grant` proves an additive class works. Rejected: it would make the CIS
policy a resource two packs declare, and every pack contributing a subject would have to
restate the whole `google_org_policy_policy` body, constraint name and parent included,
to reach one field of it. The thing being added is a value, not a resource.

**Leave it to the estate, as today.** The estate repeats the five SCC agents and adds its
own. Rejected: that is the trap this record exists for — the entry has no owner, nothing
removes it when the pack goes, and the operator who has to write it is told so by a
comment in a pack header.

**Refuse a contribution whose target no file declares.** Considered, and it is the rule
this project would normally take: fail fast rather than drop. Rejected because it makes
the Defender pack require the CIS baseline. An estate that onboards Defender without the
baseline has nothing restricting its grants, so the contribution is moot rather than
wrong, and refusing would force a pack into an estate to satisfy a value nobody reads.
The need is not dropped, only the entries: the pack graph's `data` edge names it, and the
compile reports a pack that deploys while something it needs is off.

## Consequences

- An estate that uses no contributing pack compiles byte for byte as before: with no
  contribution, the seed is empty and the environment is built from exactly the call
  today's code makes. Every corpus snapshot is unchanged by this record's change.
- A pack now owns the external principals it needs. The Defender pack's manual
  prerequisite is gone from its header, and `billing-export` is the first pack whose
  whole reason for touching the CIS policy is one line in its own params.
- One extra pass over the `use` graph per compile: the files are parsed twice. Estates are
  small and `estate_params` already walks them separately for other commands.
- That pass runs before the compile's own refusals, so its errors would otherwise replace
  a better message for the same estate (a pack that MOVED, the chain a cyclic `use` is
  reported with). Its error is held back and returned only if the compile finds nothing
  to say.
- `contributes_` is reserved. A param that begins with it and names nothing is refused
  rather than treated as an ordinary param.
- A contribution puts no order on the two `use` lines, unlike a read. Check 6 of
  `satz pack-graph` and the template test that mirrors it skip contributed params.
