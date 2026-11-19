# 0093 — delegation is a base and its targets, entries are added by one general writer, and a team's estate is written from the folders it takes

- **Status:** accepted
- **Date:** 2026-11-19

This record also holds the concept it rests on — presets and packs, how a delegated
workload comes about, and the layout of the two repositories — because the three were
settled together and each explains the others.

## The concept

### Two kinds of pack

A pack is a `.satz` file an estate `use`s. Two kinds are kept apart by where they come
from and how they change, not by what they contain (ADR 0082, ADR 0083):

```text
                 PRESET                                 PACK (workload pack)
  what it is     one library, interwoven:               one file, standing alone:
                 a map, a graph of who needs whom,      a template for a workload or a
                 compliance catalogs, a changelog       part of one
  why it changes organisation policies and the cloud    a team adapts it to its workload
                 provider change, for everyone
  source         one: the library of the satz           several: the satz repository, a
                 release                                pack repository, a folder
  taken with     satz get-presets (the whole library)   satz get-pack <id> (one file)
  kept current   satz check-presets / merge-presets:    rarely: a pack edited here is the
                 compared, merged, forked when edited   repository's own from then on
  switched       satz add-preset / remove-preset        a `use` line
  lives in       presets/                               packs/
  who uses it    an organisation's estate               a team's estate; an organisation's
                                                        may use one too
```

Both are optional, both have dependencies, and the questions of both are answered the
same way (`satz questions`, `satz interview`).

### Three ways an estate comes about

```text
  1  AN ORGANISATION            2  A DELEGATION, in the          3  A WORKLOAD, by the team
                                   organisation's estate

  satz init                     satz add-preset … use_delegation   satz init --from <folders>
  satz add-preset …             satz add-entry … delegations       satz get-pack <id>
  satz interview                satz add-entry … delegated_…       satz interview
  satz bootstrap                satz transpile --plan / --apply    satz transpile --plan / --apply
  satz transpile --plan/--apply         │
                                        │ writes interfaces/outgoing/
                                        ▼
                               delegation-<name>/   <target>/   ──── copied ───▶  interfaces/incoming/
```

1 and 2 are done by whoever runs the organisation; 3 by the team. The two meet in one
place: the hand-over folders.

### What a delegation is

```text
  THE BASE  — presets/delegation.satz, one entry of `delegations`
  ┌───────────────────────────────────────────────────────────────────────┐
  │  infrastructure project  <shortname>-deploy-<name>                      │
  │    account               svc-deploy-<name>   ← the team's estates run as │
  │    state bucket          <shortname>-deploy-<name>-state                 │
  │    APIs                  what the team's calls need (billed here)        │
  │  group                   its members may act as the account              │
  │  billing account         roles/billing.user, or what the entry says      │
  └───────────────────────────────────────────────────────────────────────┘
        │ account works in                         │ account works in
        ▼                                          ▼
  A TARGET — presets/delegated-folders.satz   A TARGET — presets/delegated-projects.satz
  one entry of `delegated_folders`            one entry of `delegated_projects`
  a folder; the account creates folders       a Google project; the account holds its
  and projects under it and holds no          IAM and services, and nothing beside it
  access control on it

  Each target entry names its base: `delegation = "<name>"`. One base, any number of
  targets of any kind. A new kind of target — a project with a network it may not change,
  an account on another cloud — is a new preset beside these; the base does not change.
```

### The two repositories

```text
  THE ORGANISATION'S                         A TEAM'S
  <customer>/                                <workload>/
    config.toml                                config.toml
    satz/<estate>.satz    the estate           satz/<workload>.satz     the estate
    presets/              the library          packs/                   its packs, and origin.toml
    packs/                packs, if any        hcl/                     generated
    schemas/              provider schemas     schemas/
    hcl/                  generated            interfaces/
    interfaces/                                  incoming/              what it was given: never
      outgoing/           written whole on         delegation-<name>/   written by satz
        common/           every transpile          <target>/
        delegation-<name>/   ── copied ──▶       outgoing/              what it publishes in turn
        <target>/                                                       (often nothing)
      incoming/           (an organisation
                          is given nothing)
```

A team's estate needs the hand-over folders and nothing else of the organisation's: not
its estate file, not its library, not its state.

## Context

Delegated access was two presets. `folder-workload` already had two lists — the
workloads (an account each) and the folders an account works in. `project-workload`
mixed both into one entry (a project with its own account inside it) and had a second
list for a project worked by a workload's account. So "an identity" and "a place it
works in" were modelled three ways, and a new kind of place had no pattern to follow.

Adding an entry to one of those lists was editing the estate file. `satz_interview`
refuses such a value, and the command that once wrote one, `add-project`, went with
ADR 0075 because it wrote resources the binary knew.

`satz init --project --interface` wrote a team's estate for one of the three shapes
only, copied nothing, and told the others to write theirs by hand.

Nothing machine-readable said who delegated: the organisation, the name of its estate,
the account. It was prose in the hand-over folder's README.

## Options

**The library**

1. Keep the two presets. Cost: three shapes for one idea.
2. **One base preset, one preset per kind of target.** Cost: a rename of every list, and
   a project loses the account inside it — every delegation has an infrastructure
   project of its own.
3. A base built into the binary (`satz delegate`) with targets as presets. Cost: the
   shape of an identity is then the binary's, and changes with every variant and every
   other cloud.

**The writer**

1. A command per kind of entry. Cost: the binary learns every list.
2. **One command for any list a pack declares with `request`.** The declaration already
   names the key, the fields, a pattern per field and the defaults.
3. Leave it to editing. Cost: no standard route, on the command line or in an app.

## Decision

- **`presets/delegation.satz`** — list `delegations`, entry `{ name owner_group apis
  billing_roles }` — is the base: the infrastructure project, the account, the state
  bucket, the group. Its interface is `delegation-<name>`.
- **`presets/delegated-folders.satz`** — list `delegated_folders`, entry `{ name
  delegation display_name parent budgets }` — and **`presets/delegated-projects.satz`**
  — list `delegated_projects`, entry `{ name delegation apis budgets }` — are the
  targets. An entry names its base by `delegation`; an entry that names none that exists
  is refused by the compile. A target's interface is `<name>`.
- `presets/folder-workload.satz` and `presets/project-workload.satz` are gone; their
  paths and every param of theirs are refused by name with the replacement.
- **The team's calls are billed to the delegation's infrastructure project.** An API a
  call needs is asked for in the delegation's `apis`; an API a resource needs in a
  delegated project the team enables there itself.
- **`satz add-entry <estate> <list> <field>=<value>…`** (tool `satz_add_entry`) adds one
  entry to a declared list: every field declared, each pattern matched, the key new;
  written into the estate's params and kept only when the estate compiles. `satz
  interfaces` reports, per list, its entries and the defaults of its fields.
- **`satz init --from <folder>…`** writes a team's estate from the hand-over folders it
  is given: each is copied to `interfaces/incoming/`, unless it stands there already;
  exactly one must be a delegation's own, the others are the places the estate works
  in; the estate runs as the delegation's account and keeps its state under its own
  name in the delegation's bucket. `--project` and `--interface`
  are gone.
- **`satz presets`** reports `delegated_by`, per used interface file: the central
  estate's name, the organisations it manages, and the identity and place the file
  carries.

## Consequences

- One more project per delegation than a project with its own account had. In exchange
  one account works in any number of places, and nothing a team runs as lives in a
  project it also fills.
- An estate that held `project_workload_projects` gets a new account and state bucket
  for each; its state has to move. No estate known holds state in that shape.
- A delegation and one of its targets may share a name — a project `payments` of the
  delegation `payments`. Two targets may not: a folder and a project of one name would
  be one interface.
- An estate reads the values of several places by the name of each interface folder
  ([ADR 0094](0094-a-value-is-read-by-the-name-of-its-interface-folder.md)).
- `add-entry` adds; it does not change or remove an entry. Those stay edits of the file.
- A hand-over folder is still copied by hand or by `init --from`; nothing checks that a
  copy is still what the organisation publishes.
