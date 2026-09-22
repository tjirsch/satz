# 0053 — the `-generate-config-out` fallback runs under `--into` as well

- **Status:** accepted; supersedes the `--into` refusal of
  [0052](0052-the-generate-config-fallback-is-a-flag-on-the-live-import-and-writes-a-second-file.md),
  whose other four points stand
- **Date:** 2026-09-22
- **Shipped in:** the release that follows

## Context

[0052](0052-the-generate-config-fallback-is-a-flag-on-the-live-import-and-writes-a-second-file.md)
gave the live import `--generate-unmapped` and refused it with `--into`, on the
grounds that a delta import writes packs of what an estate does not declare and
generated configuration is not one of them.

The delta import runs the same sweep, skips the same resources for the same
reasons and reports them — it always has (`report_skipped` at the end of
`import_delta`, `src/import.rs`). So the operator of an estate that is being
completed sees a list of live resources the estate does not declare AND satz
cannot express, and has to start a second, plain import into a throwaway file to
get at the one mechanism that can express them. That second run sweeps the whole
scope again, as a different principal, and writes an estate nobody wants.

Letting the flag through raises three questions the refusal made moot.

## Decision

**`satz import <scope> --into <estate> --generate-unmapped` is accepted, and
writes the generated configuration into a file named after the run's packs.**

1. **The files are named after the SCOPE, like the packs.** A delta import's
   files are `imported-<scope>[-<container>].satz` (`delta::pack_name`), so the
   fallback's are `imported-<scope>-generate/` and
   `imported-<scope>-generated.satz`, beside them in `yaml_dir`. One estate can be
   filled from several scopes, and each keeps its own files the way its packs do;
   naming them after the ESTATE would have a second scope overwrite the first and
   plan in its directory. The plain sweep is unchanged — it is named after the
   estate it writes, `discovered-generated.satz` — because the rule is the same
   one: the fallback hangs off the name of the file the run is known by
   (`generate_config::output_names`).
2. **The child reads as the estate's service account.** A delta import binds the
   estate's IaC service account for satz's own calls
   (`configure_estate_impersonation`, `src/main.rs`). The `tofu` child inherits
   the environment, not that binding, so satz writes
   `impersonate_service_account` into the provider block of `imports.tf` — the
   same derivation the emitter uses. Without it one command would read the
   platform as two principals, which shows up in no output and no diff, only in an
   audit log. A plain sweep is bound to nothing and the block impersonates nobody,
   as before.
3. **The delta's subtraction applies to the fallback.** An unmapped resource whose
   relative resource name IS a live id the estate already resolved to is named and
   not generated for: the estate has it. The test is equality of the id, the same
   identity the rest of the delta runs on (`delta::undeclared`). Everything else
   is handed to the provider, including a resource whose type the estate declares
   under an id of another form — satz does not decide two ids are the same object
   because they look alike.

The generated file is still a SECOND file and still carries no `use` line (0052,
point 4): what the provider wrote has a different provenance from what satz
translated, and joining it is a reading decision that costs one line.

## Options

**Keep the refusal.** *Rejected.* Its reason was the file the run writes, not the
resources: a delta import's own report already names the unmapped resources, and
the answer it pointed at — run a plain import into a scratch estate — sweeps the
scope twice, as a second principal, for a file that is thrown away.

**Merge the generated resources into the delta's packs.** *Rejected*, for the
reason 0052 rejected merging them into the swept estate: a pack is regenerated
wholesale on every run and is read as "what the sweep found and satz understood".
Generated blocks are neither.

**Run the child on the plain ADC and say so.** *Rejected.* The human's
credentials are usually broader than the estate's service account, so the
fallback would read objects the estate itself cannot, and the generated
configuration would plan as a resource nobody can import. One command, one
identity.

**Subtract by type or by resemblance instead of by id.** *Rejected.* It is the
guess `adopt` and the delta are built to avoid. An unmapped resource whose id the
estate does not carry is generated for, and a duplicate the operator sees while
merging the file is cheaper than a live resource silently left out of both files.

## Consequences

- A delta import whose estate declares an unmapped resource under an id of
  another form generates configuration for it anyway. The generated file is
  separate and unused, so nothing breaks; the operator drops the block while
  merging.
- The scratch directory of a `--into` run holds an impersonating provider block.
  Finishing the run by hand from there — the two commands the error names — reads
  as the service account too, which is what the ids in `imports.tf` were resolved
  against.
- `configure_estate_impersonation` now hands back the identity it bound, because
  one caller has to tell a child process the same answer. Every other caller
  ignores it.
- The delta path cannot be exercised offline any more than the plain one can: the
  smoke matrix asserts that the flag is accepted and that the run reaches the live
  sweep, and the plumbing is unit-tested against an injected runner
  (`src/generate_config.rs`).
