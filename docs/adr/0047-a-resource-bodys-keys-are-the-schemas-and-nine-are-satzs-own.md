# 0047 — a resource body's keys are the schema's, and nine are satz's own

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

The README states the rule the language is sold on: "every resource key and block key is
checked against the schema at parse time — an unknown key is an error, not a guess". The
first half was true and the second was not. What the front end checked was the key that
names a TYPE: `google_storage_buckets { … }` is refused with "unknown resource type". The
keys INSIDE a body were checked by nothing at all.

So this compiled, on every type:

```
google_project {
  archive {
    project_id = "corp-archive-001"
    parent     = "organizations/123456789012"
  }
}
```

and `main.tf` got `parent = "organizations/123456789012"` in the resource block — an
argument `google_project` does not have (its parent is `folder_id` or `org_id`; `parent`
is the Resource Manager path, which belongs to an org policy). The first thing to object
was `tofu validate`, and only if someone ran it; `satz transpile` reported success.

The defect was found on `google_project`, whose block `src/emitter.rs::emit_project`
builds by hand. The emitter's generic path, `emit_shared::single_resource_block`, reads
the schema — but only to tell an attribute from a nested block and to pick the narrowest
scope attribute to inherit. It refuses nothing either, and an estate that writes
`parent = "nonsense"` on a bucket got it in `main.tf` too.

The emitter is also the wrong place for the rule: by then a body is a resolved mapping
with no file and no line, so the best a check there could say is which address is wrong,
not where it is written.

## Decision

**Every key of a resource body is checked against the provider schema in the front end,
level by level, and a key the schema does not name is a parse error.**

The `TypeResolver` the walk already consults for "is this a resource type" gains
`body_keys(tf_type, path)`: the keys the schema names in one body — `path` empty is the
resource's own body, each further element a block key one level down. The estate's
resolver answers from the loaded registry (`ResourceRegistry::block_at`); a test table
with no schema behind it answers `None`, and a level with no schema gets no verdict.

The walk checks a body where it already has the entries, with their lines: the named
resources of a resource map, and what is left of a folder's or a project's body once the
nested resource types are routed to the children. A key that opens a block is followed one
level down; a key that carries a value is a leaf, because what an attribute carries is
data — the keys of a `labels` map are the estate's own, and so is the content of an org
policy's `parameters`.

The refusal names the file, the line, the type and the key:

```
main.satz:18: google_project: unknown key `parent` — the provider schema names no such
argument or block here
```

**Nine keys are satz's rather than the provider's**, and they are a closed list
(`satz_body_key`): `"import-id"`, `lifecycle` and `provider` in any body; a project's
`project_service` and `org`; a group's `member`, `manager`, `owner` and `email`. Each is
read by the emitter and turned into something — an `import` block, a
`google_project_service`, a membership. A feature that adds another key adds it there, in
the same change.

`depends_on` is deliberately not on the list: satz derives the ordering a plan needs
itself, and nothing has ever read a `depends_on` out of a body.

With the rule in the front end, `single_resource_block`'s `google_project` branch is
deleted. It had been dead since the emitter began dispatching every project to
`emit_project`, and it was the second, staler implementation of a project's parent and
billing account.

## Options

**Narrow the README's promise to what was true.** One sentence, no code. Rejected: the
promise is why the resource types and attributes are the provider's to the underscore.
Without it a typo is a silent deletion or an invalid plan, and the language's whole claim
to being typed rests on a check that is not run.

**Check in the emitter, beside `missing_required` and `wrong_shapes`.** Those two already
walk the emitted block against the registry and report findings. Rejected: by then there
is no line to name, and a finding is a warning — an argument the provider does not have
is not a warning, it is an estate that cannot apply. The two that are there report on what
is MISSING or mis-shaped in a block satz built, which is an emitter question; what an
author may write is a language question.

**Check only `google_project`, where the defect was found.** Rejected: the hole is in
every type, and a project-specific check would have been a third implementation of a rule
that belongs in one place.

**Check only the top level of a body, not nested blocks.** Cheaper and enough for the
reported defect. Rejected: `spec { bogus = true }` on an org policy is the same mistake
one level down, and the schema answers for a nested block exactly as it does for the body.

## Consequences

- An estate whose provider schema is older than the attribute it writes is refused until
  `satz update-schema` runs. That is the cost of typing against a schema fixture, and it
  is the same cost the type check has always had; the refusal names the key, so the cause
  is readable.
- A key written inside a LIST of objects — `lifecycle_rule = [ { action { … } } ]` — is
  not checked: the list is a value, and its objects never become entries with lines. The
  schema check reaches what is written as a block.
- The nine satz keys are a list in `crates/satz-core/src/pipeline.rs` and nowhere else. A
  feature that gives a type a synthetic key and forgets the list gets a refusal on its own
  fixture, which is the intended failure mode.
- Nothing in the library, the scaffold or the fixtures carried an unknown key: the whole
  corpus, all 34 packs and the smoke estate compile unchanged, and no snapshot moved. The
  release is a MINOR because an estate outside the repository may hold one.
