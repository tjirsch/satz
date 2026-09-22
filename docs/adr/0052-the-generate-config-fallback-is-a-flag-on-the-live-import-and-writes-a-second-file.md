# 0052 — the `-generate-config-out` fallback is a flag on the live import and writes a second file

- **Status:** accepted; the `--into` refusal of point 2 is superseded by
  [0053](0053-the-generate-config-fallback-runs-under-into-as-well.md), which also
  names the files that run writes and the identity its child reads as
- **Date:** 2026-09-22
- **Shipped in:** the release that follows

## Context

A live import is a Cloud Asset Inventory sweep mapped through
`presets/import-config.yaml`: a row names the asset type, satz reads the asset's
data against the provider schema and writes a Satz resource with its
`"import-id"`. What no row names, and what carries nothing the provider schema
knows, is reported as `unmapped` and left out of the estate.

OpenTofu can produce the configuration satz cannot: an `import` block with a `to`
address and an `id`, and `tofu plan -generate-config-out=<file>` writes a
`resource` block for it, read from the live object. satz already reads that output
— the hcl import shape (§12.1 of the language reference) names it as one of its
inputs. The operator half of the loop existed: write the import blocks by hand,
run the plan by hand, feed the file to `satz import`. satz never ran it itself.

Filling that in raises four questions that are not obvious, and this record is the
answer to all four.

## Decision

**A flag on the live import — `satz import <scope> --generate-unmapped` — which
writes a scratch directory, runs `tofu init` and `tofu plan
-generate-config-out`, and hands the result to the hcl import arm, into a SECOND
estate file beside the one the sweep wrote.**

1. **A flag, not a command.** The input is the sweep's own skipped list: a type, a
   live id and the reason it was left out. Nothing outside the run holds it, so a
   separate command would need satz to invent a file format for it, and the
   operator to run two commands where one knows everything.
2. **The live shape only.** `-generate-config-out` reads a live object that no
   state manages. A state file's resources are already managed and their
   configuration is the `.tf` they were applied from, which the hcl shape reads;
   the hcl shape is what reads this output; and `--into` writes packs of what an
   estate does not declare, which generated configuration is not. Each of the
   three is refused by name with that reason.
3. **The id is the asset's relative resource name, and the plan verifies it.**
   That derivation is not new: it is what the mapped resources' `"import-id"`
   already uses (`Discoverer::asset_path`). satz proposes it and the provider
   reads the object — an id that names nothing fails the plan, with the
   provider's own message. satz never resolves an id by guessing, and here it
   resolves none: it hands over the name Cloud Asset gave the object.
4. **A second file, `<estate>-generated.satz`.** What the provider wrote has a
   different provenance from what satz translated — it is whole `resource`
   bodies, placed by the hcl arm and carrying no `"import-id"` — and which of it
   belongs in the estate is a reading decision.
5. **Only `unmapped` is generated for.** A type switched off (`import: false`,
   `--only`, `--exclude`), a platform-owned object and a resource whose parent is
   outside the import were left out on instruction.

The scratch directory is `<estate>-generate/` beside the estate, and it stays
after the run: `imports.tf` is what an operator edits when the provider refused an
id, and the two commands that finish the job by hand are named in the error.

## Options

**A `satz generate-config` command of its own.** *Rejected.* Its input is the
skipped list of an import that has already finished, so either the import writes
that list to a file satz would have to define, version and read back, or the
operator retypes the types and ids. Both duplicate what the import run already
holds in memory.

**Merge the generated resources into the swept estate.** *Rejected.* It hides the
provenance in the one file an operator reads as "what satz understood", and a
generated block is a different thing: unverified by the compliance plane, placed
by the hcl arm's rules, and quite possibly not wanted at all. Two files cost one
`use` line to join and make the choice visible.

**Derive an id per type instead of using the asset name.** *Rejected.* It is the
`import_id` templates of `presets/import-config.yaml` — and the resources this
fallback exists for are exactly the ones no row covers, so there is no template to
render. Writing one per type by hand is the mapping work the fallback is meant to
avoid.

**Drop the import blocks the plan refused and run it again.** *Rejected.* It is
healing a failure satz was told about: the second run would report success while
silently importing less than it was asked to. The run fails with the provider's
output, and `imports.tf` stays for the ids to be corrected.

**Run one plan per resource, so one bad id does not fail the rest.** *Rejected for
now.* It turns one plan into N against the platform, for a failure mode whose fix
— correcting the id in `imports.tf` — is one edit. It becomes worth
reconsidering if a real sweep routinely produces a mixture.

## Consequences

- One bad import id fails the whole generating plan. The error carries the
  provider's own output, the path of `imports.tf` and the two commands that
  continue by hand, so nothing is lost; it is a worse first run than a per-
  resource loop would give.
- The scratch directory runs its own `tofu init`, which downloads the providers
  unless `TF_PLUGIN_CACHE_DIR` is set. It is a fresh import: there is no
  transpiled estate directory to borrow an initialised provider from.
- The child inherits the environment, so it reads live as the identity the sweep
  ran as — the human's Application Default Credentials, since the flag is refused
  with `--into` and the plain live import binds nothing (`IDENTITIES`,
  `src/main.rs`). The provider block satz writes impersonates nobody.
- The fallback cannot be exercised offline: it needs a live object to read. The
  smoke matrix covers the refusals; the plumbing — which resources become import
  blocks, what `imports.tf` says, that a failing child surfaces its output, and
  that nothing runs with nothing to generate — is unit-tested against an injected
  runner (`src/generate_config.rs`).
