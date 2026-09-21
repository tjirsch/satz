# 0048 — satz reads no YAML estate

- **Status:** accepted
- **Date:** 2026-09-21
- **Shipped in:** the release that follows

## Context

Before Satz existed, an estate was a YAML document with custom tags: `variables:` with
`&anchor` / `*alias` for parameters, `!include` and `!include-if` for composition,
`!format` and `!join` for string building, `!expr` for a raw Terraform reference,
`!import-include` for a transpile-time live import, and resource keys written without the
`google_` prefix. `satz import <file>.yaml` converted such a file to Satz: a textual
pre-pass lifted the `variables:` block and replaced aliases and include directives with
sentinel strings, the remainder parsed as plain YAML, and a walker printed Satz, decoding
the sentinels back into param references, interpolations and `use` statements.

That converter was about 700 lines of production code plus its unit tests, a corpus
fixture (`tests/corpus/yaml-estate/`), a gate module in `src/main.rs` that compiled the
fixture end to end, and two steps in the smoke matrix. It also constrained the rest of
the tree: the printer carried dialect-only branches (the Tier-2 org-policy list form, the
null-valued conditional-role key, the `use`-sentinel decoding), and every language change
had to keep a document in a dead dialect compiling.

Nobody needs it any more. Every estate on the roster is Satz. The two that are still YAML
are out of scope by design and will not be converted by anyone.

The 2026-08-29 rule said the dialect "must keep converting old estates and packs for as
long as legacy orgs exist". The clause is what is being reversed here; the rest of that
rule — YAML is never generated, no new functionality grows a YAML arm — stands.

## Decision

**satz reads no YAML estate or pack. The converter is deleted; the printer it was built
on stays.**

Deleted: `pre_pass`, `substitute_aliases`, `convert_scalar_text`, `split_top_commas`,
`convert`, `inline_sequence_includes`, `retarget_uses`, `list_form` and
`normalise_conditional_binding` in `crates/satz-core/src/migrate.rs`;
`convert_yaml_to_satz` and the `--kind` / `--gate` / `--fork` flags in `src/main.rs`; the
`yaml` value of `--from`; the corpus fixture, the gate module, the dialect's unit tests
and its smoke steps.

Kept: everything in `migrate.rs` that prints Satz — `convert_value`, `emit_entries`,
`value_expr`, `key_expr`, `scalar_value`, `format_to_interpolation`,
`repeated_grant_maps`, `normalize_type_keys` and the four helpers a caller builds
references with (`param_ref`, `param_name`, `param_value`, `interpolation`,
`interpolated`). The state shape, the live shape, `--into`, the HCL importer and
`export-organizational-policies` all write Satz through it; it is the printer, not the
converter, and cutting into it breaks imports that have nothing to do with the dialect.

A `.yaml` estate or pack meeting any entry point is refused by name, with the last
release that converts it, the two commands that bring the conversion up to date, and the
check:

```
transpile: estate.yaml is written in the pre-Satz YAML dialect, which satz does not read.
satz v0.71.0 is the last release that converts it:

    cargo install --git https://github.com/tjirsch/satz --tag v0.71.0 --locked
    satz import estate.yaml --kind estate            # --kind pack for a pack
    cargo install --git https://github.com/tjirsch/satz --locked
    satz fmt estate.satz
    satz merge-presets --estate estate.satz
```

The release is written once, in `satz_core::LAST_YAML_CONVERTING_RELEASE`, so the CLI's
refusal and the compiler's `use "x.yaml"` error cannot drift apart.

`presets/import-config.yaml` and `presets/catalogs/*.yaml` are data files that configure
satz, not estates written in a language. They are YAML and stay YAML.

## Options

**Delete it; the escape hatch is the last release that converts.** *Chosen.* Removes the
code, its gate and the constraint it put on every language change. Costs: converting a
dialect file means installing an older binary first, and that binary will not understand
a pack written after it.

**Keep it.** No install step for a conversion nobody has scheduled. Costs: the lines
stay, the printer keeps its dialect branches, and every language change keeps proving
that a dead dialect still compiles. Rejected — the upkeep is paid continuously for a
capability that is used never.

**A second binary, `satz-migrate`.** The converter moves out of `satz` and keeps
shipping. Costs: a second release artifact, a second installer, a second version to
reason about, and a weaker gate than the one being removed — it would carry a copy of the
printer or depend on `satz-core`, which puts the constraint back. Rejected; this shape
was rejected once before for the YAML walk, for the same reasons.

**An unshipped workspace crate.** The converter lives in the repository, builds in CI,
and is run with `cargo run -p satz-migrate`. Costs: the same upkeep as keeping it, with
a weaker gate and a build nobody exercises. Rejected.

## Consequences

- An operator holding a dialect file installs `v0.71.0`, converts, reinstalls the current
  release, then runs `satz fmt` and `satz merge-presets`. The refusal prints that
  sequence, so it needs no documentation lookup.
- `migrate.rs` is the printer and says so. A future cut to it is a decision about every
  import shape, not about a dead dialect.
- An estate `use`ing a `.yaml` pack still gets a parse error naming the file — the
  compiler's own refusal, which now names the release rather than a command that no
  longer exists.
- The release is a MINOR under [ADR 0010](0010-the-minor-version-marks-an-upgrade-that-brings-work.md): an input format the
  binary no longer reads.
- Reversing this means restoring the converter from git and re-teaching it whatever the
  language gained since. That is the cost being accepted.
