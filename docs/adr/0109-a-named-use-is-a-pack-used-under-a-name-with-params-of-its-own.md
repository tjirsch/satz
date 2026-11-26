# 0109 — a named use is a pack used under a name, with params of its own

- **Status:** accepted; supersedes the deferral of pack instances in [ADR 0071](0071-each-expands-a-list-into-labelled-bodies-at-compile-time.md) and its rejection in [ADR 0075](0075-each-writes-an-interface-per-entry-and-onboarding-is-a-pack.md); option C of [ADR 0104](0104-two-packs-declaring-one-param-must-agree.md)
- **Date:** 2026-11-26
- **Shipped in:** the release that follows, in four parts: the language, the walk and the HCL code (this record's first part); the questions and the interview; the presets report, the pack lines, `adopt` and the editor; exports and list params of a named use

## Context

A pack is used once. Its params join one estate-wide namespace (`docs/language.md` §6.3,
rule 3), and the same pack used twice is the same resources. ADR 0071 deferred pack
instances because `each` covered the lists that had come up — projects, subnets, folders —
and ADR 0075 rejected them again for the same reason. What `each` does not cover is a pack
that is a unit of its own: a machine with its disk, its project and its grants; a budget
with its channel and its API. An estate that needs two of those today forks the pack, or
writes the second copy by hand, and keeps two files in step.

ADR 0104 made two packs that declare one param differently an error and named the way out
for a pack used several times: a namespace per use (its option C). This record is that
option.

## Decision

`use "<path>" named <name> [when <param>]`, at the top level of the estate file, uses a pack
whose header says `multiple` under a name, with params of its own:

```
use "packs/vm.satz" named web
use "packs/vm.satz" named db when want_db
params {
  web.vm_zone = "europe-west3-a"
  db.vm_zone  = "europe-west3-b"
}
```

1. **The keyword `named`**, after the path, beside `as` and `when`; only at the top level of
   the estate file. `use … as google_x` and a plain `use` keep their meaning exactly.
2. **Opt-in by the pack.** A pack says `multiple` as the last word of its header (`pack vm
   version "1.2" multiple`), its author's promise that the cloud names of what it creates come
   from its params. A named use of a pack without it is refused, saying so. A flat use of a
   `multiple` pack is allowed. Before, any word after the version was an error, so no file
   changes meaning.
3. **Bindings in the estate's `params`**, dotted: `web.vm_zone = …`. A dotted param is refused
   unless its first part names a named use at the top level of the same file, and a binding
   of a param the pack does not declare (`web.typo`) is refused, naming the pack's params. A
   dotted name in `inputs` or `locals`, and a question on one, are refused. Before, `web.x`
   was a flat param named `web.x`: no `.satz` file of the repository, and no Satz example of
   its documentation, writes one (checked over every tracked file).
4. **What a named use reads.** Every param the pack declares — and every param a file it uses
   declares, at any depth, whatever its gate — belongs to the use: its value is the estate's
   `<name>.<param>`, else the pack's default resolved inside the use. Every other name is read
   from the estate's flat namespace as it stands at the `use` line, read-only, so a pack
   written for one use works unchanged. A flat binding of the same name never reaches the use.
   Local values are the use's own. ADR 0104's refusal is judged among the files of one use and
   among the flat files, never between them: a use's `vm_zone` and a flat pack's `vm_zone` are
   two params. `none` (ADR 0106) has no value per use — the message names `web.vm_zone`, the
   name the estate binds — and the shape of such a param (ADR 0108) is read per use. Files the
   pack uses belong to the same use.
5. **Addresses.** Every resource the use declares — a resource of a type map, a folder, a
   project, a labeled grant resource — is written as `<name>__<label>`
   (`google_compute_instance.web__vm`). Every `${type.label…}` inside the use that names a
   resource of the use, and a whole text that is such a dotted path (a folder's `parent`, a
   project's `folder_id`), is rewritten; a reference to a resource outside the use stays.
   Claim witnesses follow. A grant of a grant map is **not** prefixed: its identity is the
   member, the role and the scope, identical edges merge by union, and two HCL resources for
   one binding would each remove the other's on destroy. A grant in a folder or a project of
   the use follows that node, and its HCL label hashes the node's path where a flat one hashes
   none — the label a flat grant has always had does not move. Cloud names — a machine's name,
   a bucket's, a project id — are never touched: they come from the use's params.
6. **One resource, one writer.** Two resources with one body — written by two named uses, or
   by a named use and the estate — are refused, naming both and the params of the pack the
   resource reads, under each use (`bind web.vm_name, db.vm_name differently`). Where the
   `import_id` template of `presets/import-config.yaml` renders one id for two such resources
   from what the estate writes, they are refused too. A use name that is a param, an input or
   a local value of the estate, that holds `__`, or that is not `[a-z][a-z0-9_]*`, is refused;
   so is a param, an input or a resource label of the estate beginning `<name>__` where a named
   use `<name>` exists.
7. **Variables.** A param of the use is `<name>__<param>`, written through the `_`→`-` rule:
   `variable "web--vm-zone"` and `web--vm-zone = …` in `terraform.tfvars`, described by the
   param's question.
8. **What a named use refuses in this part**, each at the `use` line with the file and line of
   the statement and the reason: a named use outside the estate file or inside another; `hcl`,
   `action`, `inputs`, an `interface` block (an `each` around one included), an `export`, an
   `entries` statement, a `notice`, an `offers` entry; a `contributes_<param>` that adds to a
   list param a named use owns. A `contributes_<param>` from a named use to a flat list is
   allowed, its entries resolved inside the use, an entry two uses contribute alike added once.
9. **Gates.** `use … named db when g` reads a flat `g`; a `when` inside the pack reads the
   use's own params, else the flat namespace.

The commands that read every pack once refuse an estate with a named use rather than misread
it, naming the line: `satz presets`, `add-preset`, `remove-preset`, `satz adopt`, `satz packs`
and the workload pack commands that read the estate. The pack graph refuses a named use of a
preset of the library. The editor's completion and hover show the flat namespace. The third
part of this change teaches them named uses; the fourth lifts the refusal of exports and list
params. The second part taught the questions, below.

### The second part: questions, the interview and the gate

`satz questions`, the interview and `satz_interview`, and the check of open questions before
`transpile --apply` and `bootstrap` read a named use. Each question of the use's pack is asked
once per use:

1. **One row per use.** The row's `subject` is the name the estate binds, `web.vm_zone`; a
   choice's is `web.<choice>` and its options' `param` are `web.<option>`. The JSON report gains
   two optional fields, absent for a flat use: `named` (`web`) and `param` (`vm_zone`, the
   pack's own name). No field is removed or renamed, so a reader of the flat report reads it
   unchanged.
2. **Answered** means the estate's `params` binds `web.<param>`. The default, the shape (ADR
   0108) and `ask_when` are read inside the use: the schema-free walk keeps each use's
   namespace as `<name>.<param>` beside the flat one. A param declared `none` blocks per use.
3. **Written** by the one writer of a param (`bind`): `web.vm_zone = …`, aligned like any other
   binding. `--accept-defaults` binds each use's offers.
4. **Grouped by use.** The terminal interview and the decisions sheet head a use's questions
   `web — packs/vm.satz`; the workbook has a `use` column, the last, empty for a flat use.
5. **The gate** counts each use's open questions and names them `web.<param>`.

The internal name of a use's param in the HCL code, `web__vm_zone`, stays the transpile's: the
questions the transpile carries for a variable's description are renamed so, the questions the
report carries are renamed to the dotted name. Neither is shown where the other is.

## Options

**The keyword: `named` / `instance` / overloading `as`.** `as` already means "the file is the
content of a map of this type"; `use "vm.satz" as web` would read as a type named `web` and
make one keyword decide between two meanings by whether a schema knows the word. `instance`
names a thing satz does not otherwise talk about and is not in the glossary. `named` says what
the line adds — a name — and reads as English. *Chosen: `named`.*

**Where the bindings live: the estate's `params` / a block on the line.** `use "vm.satz" named
web { params { … } }`, the form ADR 0071 sketched, puts a second binding syntax beside the
estate's `params` and a block on a line every reader of pack lines parses by text. Dotted names
in `params` keep one place where an estate binds values, one place `satz interview` will write
answers, and the `params` reader of every tool. Costs: a dot is no longer free in a param's
name. *Chosen: dotted names in `params`.*

**What a use reads: read-through / explicit passing.** Passing every value a pack reads from
another pack (`customer_organization_id`, `default_region`, `logsink_project_id`) into each use
would make every library pack unusable as it stands and double every binding. Read-through
keeps packs unchanged; a param the pack declares is the use's, everything else is the estate's,
read-only. Costs: a pack that reads a name another pack declares depends on that pack being
used flat. *Chosen: read-through.*

**Addresses: an automatic prefix / labels the author writes.** Labels written per use
(`{use}_vm`) would need a new interpolation in labels and an author who never forgets one; an
unprefixed label in a pack used twice is the same address twice, which the fold would merge or
refuse. The prefix is mechanical, applies to every resource, and is undone by removing it —
which is what the property test checks. Costs: addresses an author did not write, and a
reference rewrite. *Chosen: the automatic prefix.*

**Opt-in: a header word / any pack.** The prefix makes two addresses; only the pack knows
whether the cloud names behind them differ per use. A pack whose bucket name is a literal,
used twice, plans two buckets of one name and fails at apply — or, worse, a project service
twice, which a destroy of either switches off. The identity refusal catches identical bodies,
not a literal name inside otherwise different ones. `multiple` is the author's statement that
the names come from params. Costs: a pack must be edited and versioned to be used twice.
*Chosen: the header word.*

**The v1 limits.** Exports, list params, actions, raw HCL, inputs and interfaces each need a
rule of their own for what a second use means — a second export of one name, a second variable
of one name, a second step of one name. Refusing them now keeps every part of this change
small and loses nothing an estate had. Notices are refused too: their acknowledgement is a
param, and who acknowledges which use is the interview's to decide. *Chosen: refuse, each
liftable.*

## Questions the decision above did not settle, settled here by refusing

- **A pack's files and `multiple`.** Only the pack the line names must say `multiple`; the
  files it uses belong to the use without it.
- **Which params a use owns.** Every param any file of the pack declares, read whole before
  the walk, whatever its gate — not only the files the gates admit — so the set does not
  depend on the use's own answers.
- **A named use and the estate writing one body.** Refused like two named uses: one body is
  one resource whoever writes it.
- **A use name ending in `_`.** Refused: `web_` + `__vm` would read as `web___vm`.
- **A labeled grant resource** (`role` and `member` in a labeled body) has an address of its
  own and is prefixed like an entity; a grant of a grant map is not.
- **A grant inside a node of a named use** keeps its edges' identity, and its HCL label hashes
  the node's path, so the grants two uses write on their own nodes are two resources.
- **An `import_id` check without `presets/import-config.yaml`** in the presets directory has
  no template to render and judges nothing; identical bodies are refused without it.
- **A notice in a named use** is refused (the decision named no rule for it). The second part
  keeps the refusal: a notice is closed by running its command — in the library an `adopt
  --execute --import`, which binds the acknowledgement — and `satz adopt` refuses an estate
  with a named use until the third part. A notice per use that no command can close would be
  a gate that only a hand edit opens, and the transpile's warnings, the refusal of a command
  that writes to the organization and the report of `satz presets` read the notice's param
  flat. The third part decides it with `adopt`.
- **A choice's name in a named use** is `web.<choice>`, though a choice's name is no param:
  the subject is what an answer is keyed by, and two uses of one choice need two keys.
- **The workbook's `use` column** is the last, after `param`, so no column a reader finds by
  position moves.
- **A yes to a gate of a named use** binds `web.<gate>` and switches no pack line: the pack
  graph knows no gate of a named use, and the pack's own `use … when` lines read the use's
  params.
- **The workload pack's changelog.** `presets/CHANGELOG.md`'s table is the library's;
  `project_budget` 1.2 is recorded under `### Unreleased`, as its 1.1 was.
- **The showcase.** `tests/smoke/yaml/showcase.satz` is read by `satz questions`, the
  interview and `satz presets` in the smoke run, which refuse a named use in this part; the
  example lives in `tests/corpus/named-uses/` and in the smoke's workload estate with the
  budget pack, and joins the showcase when those commands read named uses.

## Consequences

- `docs/language.md` §6.2, §6.3 rule 3, §6.9 (`named`), §10 and §11; the glossary's
  **named use** and the names `named` and `multiple`; `docs/llms.md`.
- `packs/project-budget.satz` 1.2 is the first pack that says `multiple`.
- The grammar (`satz-tree-sitter`) reads `named NAME` on a `use` and `multiple` in a pack's
  header; the canonical form carries both only where written, so no pack's form moves.
- A property test holds every pack of the library and the corpus that a named use takes to
  the flat use's HCL under the prefix; `tests/corpus/named-uses/` is the snapshot.
- Until the third part ships, an estate with a named use transpiles, plans, applies and is
  interviewed, and is refused by the commands named above.
