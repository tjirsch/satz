# 0044 — a project says its parent, and an empty parent says nothing

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

An enclosure holds the estate's own resources; a pack is used at the estate's top level.
A `use` inside a folder's body or a project's body is to be refused. Before that refusal
can land, the packs that depend on that position have to stop depending on it.

An audit compiled all 34 packs of the library twice — once used bare at the top level,
once inside a probe folder's body — and diffed the emitted HCL. Thirty-two are
byte-identical either way: what they declare is organisation-level or project-scoped, and
neither reads the enclosing folder. Exactly two differ, and each by one attribute on one
resource:

| pack | the resource | bare | nested |
|---|---|---|---|
| `presets/monitoring/organization-audit-logsink.satz` | `google_project.logsink_project` | `org_id = "<org>"` | `folder_id = google_folder.<enclosing>.name` |
| `presets/integrations/microsoft-defender-for-cloud.satz` | `google_project.mdc_mgmt` | `org_id = "<org>"` | `folder_id = google_folder.<enclosing>.name` |

Both create a project, and a project's parent is an ordinary attribute of
`google_project` — `org_id` or `folder_id`, one of the two, never both. The emitter filled
it from the node path when the block said nothing. That is the whole of what nesting
bought those two packs, and it is also the whole reason an operator nested them.

A pack cannot state its parent as a literal: the folder is the estate's, its numeric id
exists only after an apply, and its label differs per estate. So the value has to be a
param, and the param has to be able to carry a reference to a folder the estate declares.

## Decision

**A project says its parent, and an empty parent says nothing.**

`google_project` takes `folder_id` or `org_id` as the estate writes it:

- a dotted path is a reference to a node the estate declares —
  `folder_id = "google_folder.shared.name"` is emitted as
  `folder_id = google_folder.shared.name`, the same traversal the emitter writes when it
  fills the attribute from the node path;
- anything else is a literal id — `"123456789012"`, `"folders/123456789012"`;
- an EMPTY value is not a value: the attribute is not emitted, and the node the block
  stands in decides, exactly as if the key were absent.

The two packs each gained a param for it — `logsink_project_folder` and
`mdc_mgmt_project_folder` — defaulting to empty. An estate that uses the pack bare gets
`org_id = "<org>"` as before; an estate whose `use` line stands in a folder's body gets
that folder as before; an estate that sets the param to `google_folder.<label>.name` gets
that folder wherever the `use` line stands, and the emitted HCL is byte-identical to the
nested form — proved on both packs, and at fixture scale by the corpus, where flattening
`tests/corpus/real-packs` moved not one line of `main.tf` and added one row to the
variable table.

## Options

**Refuse an empty parent and make the default the organisation.** The param would then
always state something, which is the shape without a blank convention. Rejected because
it moves a plan: every estate that nests one of the two packs today would have its project
hoisted out of its folder by an upgrade, before the refusal that makes the param necessary
has even shipped. The empty default is what makes this purely additive — the fleet takes
it with no edit, and the edit comes with the refusal.

**A synthetic `parent` key on `google_project`, `"organizations/<id>"` or
`"folders/<id>"`.** One key instead of two, and it matches `google_folder`'s own
`parent`. Rejected: it is a second spelling of an attribute the provider already has, it
still needs a "says nothing" value to stay additive, and it buys nothing over writing the
provider's own `folder_id`.

**A reference written as `"${{google_folder.shared.name}}"`, the library's usual spelling
for a reference.** That emits `folder_id = "${google_folder.shared.name}"` — a plan
identical to the nested form, but not the same text. Rejected because the claim that is
worth having is byte-identity of the emitted HCL: it is checkable, and it is what lets an
operator flatten an estate and see an empty diff rather than read a plan.

## Consequences

- The two packs emit the same project wherever they are used, once the param is set. The
  refusal of a `use` in a folder's body can land without leaving them unable to place
  their project.
- A param holds a reference as a dotted path. It is stringly typed and nothing checks that
  the folder exists at compile time; a name that is wrong reaches `tofu plan` as an
  unknown resource. The same is true of every `${{…}}` reference a pack writes.
- An empty `folder_id` or `org_id` on a `google_project` is not emitted. An estate that
  means "the organisation" and writes `org_id = ""` gets the organisation — which is what
  it means — rather than an attribute the provider refuses.
- Neither param is asked in the interview yet. While a `use` may still stand in a folder's
  body, an answer would not decide where the project lands, and a question whose answer
  can be overruled by the position of a line is a question that lies. It belongs with the
  refusal.
