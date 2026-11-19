# 0094 — a value is read by the name of its interface folder, and one estate works in several places

- **Status:** accepted
- **Date:** 2026-11-19

## Context

A workload estate reads what the organization's estate delegated through interface
folders: one for the delegation — the account, its project, its state bucket — and one
for every folder or project that account works in
([ADR 0093](0093-delegation-is-a-base-and-its-targets-and-entries-are-added-by-one-writer.md)).
A value was read by its name alone, `${{interface.project_id}}`.

Every delegated project publishes a `project_id`, every delegated folder a
`delegated_folder`. Read by the name alone, two of them in one estate are one name with
two values, so an estate could read one delegated resource only, and `satz init --from` refused a
second. The delegation itself never had that limit: one account may work in any number
of delegated resources. A workload layered over three projects and applied by one account from one
estate is an ordinary case, and it needed three estates.

The same name-alone read hid a second problem in the single case: a pack that reads
"the" project relies on there being exactly one. Whatever is delegated carries a name
that says what it is for — a project `ledger`, later an instance — and the name is the
stable way to say which one is meant.

An export could not be built from a value the estate read, so an estate that
publishes addresses inside the project it was given had to be told that project's id a
second time, written out.

## Options

1. **Keep one delegated resource per estate.** No change to the language. Cost: one estate per
   project for a workload that is one thing; shared values copied between them.
2. **Rename the values per delegated resource** (`ledger_project_id`). Cost: every pack that reads
   a project would have to know its name; the prerequisites check and the reports, which read
   the fixed names, would have to guess.
3. **Read a value by the name of its interface folder**,
   `${{interface.<folder>.<value>}}`, the names inside a folder unchanged.

## Decision

- **Option 3.** `${{interface.<folder>.<value>}}` is the value `<value>` of the used
  interface folder named `<folder>` — the name in the first line of its
  `satz/interface.satz`. It is the form satz writes and documents, also for an estate
  with one delegated resource.
- **The bare form `${{interface.<value>}}` stays for what is said once**: a value every
  used folder that carries it agrees on, such as the organization's region. A value two
  folders carry differently has no bare name; the read is refused, naming the folders
  and the two named reads. A workload pack, which cannot know a folder's name, defaults
  to the bare form and is bound to a named read in an estate with several delegated resources.
- **One estate works in any number of delegated resources** of one delegation. `satz init
  --from` takes every folder and writes, per delegated resource, the line that reads it. An estate still runs
  as one account: exactly one folder is a delegation's own.
- **An export may be built from a value that was read.**
- **In the generated HCL code** a named read is `local.interface.<folder>__<value>`, the
  folder's `-` written `_`; two reads that would be written alike are refused.
- **Two used interface files of one interface name are refused**: the name is how a
  value is told apart.
- **The prerequisites check does not judge which delegated resource a resource stands in.** With
  several, a role counts as delegated when it was delegated on every one of its kind,
  and budgets when every folder that speaks of them says yes.

## Consequences

- The prerequisites check is stricter than the grants for an estate whose delegated resources were
  given different roles: a role held on two of three projects does not count. Judging each
  resource against the delegated resource it names is the step that would lift this.
- An estate written before reads its one delegated resource by the bare name and transpiles unchanged.
- A workload pack's default (`${{interface.project_id}}`) is refused in an estate with
  two delegated projects until the estate binds the pack's param; the message names the
  reads to choose from.
- The names of the values inside a folder stay fixed, so the reports and the prerequisites
  check read on unchanged.
