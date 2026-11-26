# 0109 — a named use is a pack used under a name, with params of its own

- **Status:** accepted; supersedes the deferral of pack instances in [ADR 0071](0071-each-expands-a-list-into-labelled-bodies-at-compile-time.md) and its rejection in [ADR 0075](0075-each-writes-an-interface-per-entry-and-onboarding-is-a-pack.md); option C of [ADR 0104](0104-two-packs-declaring-one-param-must-agree.md)
- **Date:** 2026-11-26
- **Shipped in:** the release that follows, in four parts: the language, the walk and the HCL code (this record's first part); the questions and the interview; the commands that read every pack — the presets report, the pack lines, `adopt`, notices and the editor; exports and list params of a named use (the fourth part)

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
   `action`, `inputs`, an `interface` block (an `each` around one included), an `export` and an
   `entries` statement (lifted by the fourth part, below, but `all <type>`), a `notice` (lifted
   by the third part, below), an `offers` entry; a
   `contributes_<param>` that adds to a list param a named use owns. A `contributes_<param>` from a named use to a flat list is
   allowed, its entries resolved inside the use, an entry two uses contribute alike added once.
9. **Gates.** `use … named db when g` reads a flat `g`; a `when` inside the pack reads the
   use's own params, else the flat namespace.

In the first part the commands that read every pack once refused an estate with a named use
rather than misread it: `satz presets`, `add-preset`, `remove-preset`, `satz adopt`, `satz
packs` and the workload pack commands that read the estate. The pack graph refuses a named use
of a preset of the library. The second part taught the questions, the third the commands,
both below; the fourth lifted the refusal of exports and list params.

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

### The third part: the commands that read every pack, and notices

Every command reads an estate with a named use; what each does with the `use … named` line:

1. **The pack lines.** The scanner every pack command shares (`src/preset_lines.rs`) reads
   `named <name>` on a line. `satz presets` and `satz_presets` list a named use as an
   `unmanaged` row of its own, with the new optional field `named`; `satz packs` and
   `satz_packs` give a pack the estate uses under names the new field `named`, the list of
   names, and a read that is a param's value counts as bound by the estate where every use binds
   `<name>.<param>` (and a flat line beside them binds the flat name).
2. **`add-preset`, `remove-preset`** work on an estate with named uses. A line that uses the
   preset they switch under a name is refused, saying so: no preset of the library says
   `multiple`.
3. **`merge-presets`** repoints a named line at a fork as it repoints any line: the path
   changes, `named <name>` and `when` stay.
4. **`satz adopt`** reads the prefixed addresses as written. A flat use turned into a named use
   renames every address its pack writes, and the live object the state holds under the old one
   is a state move (ADR 0005). `--execute` writes no `"import-id"` into a file a named use
   reads: the line serves every use of the pack; the resource is reported with `--execute
   --import` instead, as a pristine pack's is.
5. **Notices** are acknowledged per use. The acknowledgement of a notice of the use's pack is
   the estate's `<name>.<param> = true`; it is no variable (no `<name>--<param>` in
   `terraform.tfvars`), the transpile's message stands at the use's line and names
   `` `<pack>` named <name> ``, the check before a command that writes to the organization
   counts each use's open notice, and `satz adopt --execute --import`, run over every resource,
   binds `<name>.<param> = true` for each use whose notice names `satz adopt`. A notice row's
   `param` is the dotted name and its new optional field `named` the use.
6. **The pack graph** is unchanged per pack. `satz preset-graph` refuses an offered pack whose
   header says `multiple` (check 11): the map offers a pack once, to an estate that uses it once.
7. **`doc-presets`** prints, for a pack that says `multiple`, the named line beside the flat one
   in its "Use it" section, with its params as `web.<param>`.
8. **`review-preset`** holds a pack that says `multiple` to a named use at authoring time: what
   a named use refuses is an error; a param of the estate's own vocabulary declared again, and a
   resource whose identity attributes — what `satz adopt` finds it by: its natural key, the
   placeholders of its `import_id` template, its `match_on` — are all literals, are warnings.
9. **The editor** completes `<name>.<param>` in the estate's `params`, and hover and
   go-to-definition on `web.vm_zone` reach the pack's declaration. A bare name follows the flat
   uses only.
10. **The manifest** carries the use that wrote each resource, from its `<name>__` prefix:
    `satz scan`, `satz_scan_checkov` and `satz_adopt` show it beside the pack line, which
    serves every use, as the new optional field `named`.

### The fourth part: exports and list params

1. **Exports.** A core export of a file of the use is the output `<name>__<export>` — a dot is
   no character of a Terraform output's name, and `__` is how the use already names what it
   writes. Its value is resolved inside the use, as the use's resources are: a param reads the
   use's value, a local value the file's own, and a `${type.label…}` reference that names a
   resource of the use is rewritten to the prefixed address. It is then a core export like any
   other: `outputs.tf`, every interface folder (`satz/interface.satz`, the module, the README's
   table), `satz interfaces` and the comparison of an interface's files between transpiles. An
   `interface` block, an `each` around one, and `export … = all <type>` stay refused.
2. **Lists.** A list param of the use's pack is the use's own, like every param it declares: the
   estate binds `<name>.<list>`, an `each` over it expands per use under the use's prefix, its
   `defaults` are filled per use, and the pack's `entries` statement holds that list, named
   `<name>.<list>` in its messages. `satz add-entry <estate> <name>.<list> …` writes the entry
   into the estate's `<name>.<list> = [ … ]`, checked against the use's `entries` statement as an
   entry of a flat list is.
3. **Reports.** A list of a named use is reported as the estate binds it: the `param` of a
   `list_params` row of `satz interfaces` is `<name>.<list>`, and the new optional field `named`
   names the use. No field is removed or renamed.
4. **Contributions stay refused.** A `contributes_<list>` from another pack, and from a request
   file, adds to a list of the flat namespace by its name; one that names a list only a named use
   declares is refused, naming `<name>.<list>` and `satz add-entry`. `contributes_<name>.<list>`
   is refused by the parser in any file.

What a named use refuses, the limits this record keeps: `hcl`, an `action`, `inputs`, an
`interface` block (an `each` around one included) or an interface file, an `export … = all
<type>`, an `offers` entry; an `entries` statement on a list the use does not own; a
`contributes_<list>` to a list of a named use, from another pack, a request file or a file of the
use itself; a core export of a use and an interface's export that are one output; a named use
in a pack or outside the estate file; and a named use of a preset of the library.

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
- **A notice in a named use** was refused by the first and second parts: a notice is closed by
  running its command — in the library an `adopt --execute --import`, which binds the
  acknowledgement — and `satz adopt` refused an estate with a named use. The third part lifts
  it, per use (above): a whole `adopt --execute --import` covers every use, so it binds each
  use's acknowledgement.
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
  interview and `satz presets` in the smoke run, which refused a named use until the third
  part; it now uses `tests/smoke/yaml/showcase-bucket.satz` twice, each use acknowledging the
  pack's notice for itself.
- **Which files an `"import-id"` is not written into** (third part). Every file a named use
  reads — the pack the line names and every file it uses — and for every resource whose
  declaring line is in one, a resource of a flat use of the same pack included: an id written
  into a line every use reads would be every use's. Refused with the hint to `--execute
  --import`, never guessed per use.
- **A named use of a preset of the library** stays refused by the pack graph (third part), its
  `.local` fork included: the graph reads each preset once, the map offers it once, and an
  offered pack that says `multiple` is a check `preset-graph` fails. `add-preset` and
  `remove-preset` refuse a named line of the preset they switch rather than switch the flat
  line beside it.
- **`satz presets` reports no notice of an unmanaged pack**, named or not (third part): the
  graph carries the notices it reports. A named use's notice reaches the operator through the
  transpile's message at the use's line and the check before a command that writes.
- **`bound_by_estate` of a pack used under names** (third part) holds only where every use
  binds the param, and a flat line beside them binds the flat name: one use that does not makes
  the read.
- **The `named` field's form** (third part). One per row where a row is one line — `unmanaged`,
  a notice, a manifest resource — and a list where a row is one pack the estate may use several
  times (`satz packs`). Absent, or empty, for a flat use, so a reader of the flat report reads it
  unchanged.
- **A contribution to a list of a named use** (fourth part). `contributes_event_topics` names a
  list by its flat name, and every use of the pack has a list of that name: a contribution to
  one of them would be a guess. `contributes_web.event_topics` would make a pack, or a workload
  estate's request file, name a use of the estate it is vendored into, which a pack written for
  any estate cannot know. Refused both ways; the estate binds `web.event_topics` or runs
  `satz add-entry`, and a workload estate asks the estate's team. A file of the use that
  contributes to the use's own list is refused too: the list's value is the estate's binding,
  else the pack's default, and nothing else adds to it.
- **A request to a list of a named use** (fourth part). `satz check-request` refuses a
  `contributes_<list>` whose list is a named use's, naming `<name>.<list>`; the interface README's
  *What you may request* lists the flat lists alone, so no workload estate is offered a request
  that the check refuses.
- **An `entries` statement in a named use on a flat list** (fourth part). Refused: every use
  would declare the one flat list once more, and `resolve_requests` refuses a list declared
  twice. The statement stands beside the list, in the file that declares it.
- **`all <type>` in a named use** (fourth part). It publishes every resource of the type the
  estate writes; whether a use's map would hold the use's resources, the estate's, or every
  use's is not the use's to decide. Refused; a use exports each of its resources by reference.
- **A use and an interface writing one output** (fourth part). A use's core export `bucket` is
  `web__bucket`, and so is the export `bucket` of an interface `web` (or `web` written `-`).
  Refused at the use's line, naming both, rather than one output silently winning.
- **The property test and the estate's vocabulary** (fourth part). With exports lifted,
  `presets/estate-core.satz` takes a named use, and the property "a named use writes what the
  flat use writes under its prefix" does not hold for it: it declares `customer_shortname` and
  the rest of the estate's own params, which a named use owns and does not read from the
  estate. The test leaves out a pack that declares a param the estate binds — what
  `review-preset` warns about — rather than bind the use's copies to the estate's values.
- **The export's value** is resolved when its file is walked, inside the use, against the use's
  namespace — every param of the use is in it from the start and what the use reads of the flat
  namespace is fixed at the `use` line — where a flat export is resolved after the whole walk.
- **What review-preset calls a core param and an identity** (third part). A core param is one of
  the estate's own vocabulary the review binds to its example values (`presets/estate-core.satz`);
  an identity is what `satz adopt` finds a resource by. A resource that writes none of its
  identity attributes is not judged: what the provider derives is not the pack's to say.

## Consequences

- `docs/language.md` §6.2, §6.3 rule 3, §6.9 (`named`), §10 and §11; the glossary's
  **named use** and the names `named` and `multiple`; `docs/llms.md`.
- `packs/project-budget.satz` 1.2 is the first pack that says `multiple`.
- The grammar (`satz-tree-sitter`) reads `named NAME` on a `use` and `multiple` in a pack's
  header; the canonical form carries both only where written, so no pack's form moves.
- A property test holds every pack of the library and the corpus that a named use takes to
  the flat use's HCL under the prefix; `tests/corpus/named-uses/` is the snapshot.
- `tests/smoke/yaml/showcase.satz` uses `showcase-bucket.satz` twice; the smoke run reads it
  with every command and asserts the presets report, the packs report, the editor and the
  per-use notice.
- The new optional JSON fields, none removed or renamed: `named` on an `unmanaged` row of the
  presets report, on a notice row (whose `param` is dotted for a named use), on an adopt row and
  a scan finding; `named`, a list, on a row of the packs report; `named` on a `list_params` row
  of `satz interfaces`, whose `param` is dotted for a named use.
- `tests/smoke/yaml/showcase-bucket.satz` exports `bucket` and declares the list
  `bucket_archives`; the smoke run asserts `reports__bucket` and `scratch__bucket` in
  `outputs.tf` and every interface folder, adds an entry with `satz add-entry <estate>
  reports.bucket_archives`, reads it back from `satz interfaces` and refuses a request to it.
