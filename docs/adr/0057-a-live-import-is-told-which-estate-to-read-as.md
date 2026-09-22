# 0057 — a live import is told which estate to read as

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

`satz import organizations/<id>` is the headline command of a brownfield
adoption: one Cloud Asset Inventory sweep of an organisation, written out as an
estate. It bound no identity — `IDENTITIES` (`src/main.rs`) classed `import` as
`EstateSa` while the code bound the estate's service account only under `--into`,
and a comment at the dispatch said the bare form stays on the caller's
Application Default Credentials. Table and code disagreed, and the code's answer
was the one that ran.

That answer does not work on the estates satz itself builds. satz prescribes an
IaC service account that holds the named roles and applies everything
(ADR 0009), and `roles/cloudasset.viewer` on the organisation is one of them —
held by that account and by nobody else. So on such an organisation the sweep is
refused:

```
PERMISSION_DENIED … Missing required IAM permission on requested scope
```

while the same sweep through `--into <estate>` returns in seconds. The command
satz's own documentation opens the adoption workflow with could not be run
against the model satz's own documentation prescribes, and the only way through
was to write a delta into an estate that does not exist yet.

The message made it worse: it said an asset type ListAssets refuses is named in
the error and pointed at `--exclude` and
`scripts/update_import_config.py --probe` — the asset-type table, when the cause
was the credential. Two operators can lose an afternoon in that table.

## Decision

**The live import takes its identity from an estate the operator names, and says
so when it is given none.**

1. **`--as <estate>` binds that estate's IaC service account** — the same
   derivation `--into`, `plan`, `apply` and `adopt` use
   (`configure_estate_impersonation`), the account the emitted provider block
   gives `tofu`. It writes a new file, like a bare sweep; the estate is read for
   `svc_iac_account` and `infra_project_name` and nothing else, and nothing is
   written into it.
2. **`--into` keeps binding the same way**, and the two flags are refused
   together: `--into` names the estate already, and two names could differ.
3. **Given neither, the sweep reads as the caller's own credentials.** There is
   no estate to be — the output is a new file — exactly as `init`. `IDENTITIES`
   now says this, as `HumanOrEstate`, and the comment at the dispatch says the
   same; the classification is no longer a claim the code contradicts.
4. **A refused sweep names the credential and the identity it ran as.** The
   message says as whom the requests went out — the service account when one was
   bound, "the caller's own Application Default Credentials" when none was — and,
   for a refusal, that `roles/cloudasset.viewer` is the IaC service account's and
   which flag borrows it. The asset-type advice stays for the errors that are
   about asset types (`fetch_refusal`, `src/discovery.rs`).

## Options

**Derive the estate from `--config`.** *Rejected.* A config directory holds a
`yaml_dir` with any number of estates, and choosing one of them is a guess about
whose organisation is being read and whose service account is being used. satz
does not guess about live identity anywhere else — `adopt` resolves one candidate
or reports ambiguity — and the failure mode here is silent: the sweep would
succeed as the wrong customer's account, which shows up in no output and no diff,
only in an audit log. A config with exactly one estate would work and every other
one would need the flag anyway, so the flag is the mechanism either way.

**Leave the bare form on the caller's credentials and only document it.**
*Rejected as the whole answer, kept as half of it.* It is honest, and it is what
the bare form does now. Alone it leaves the headline command unusable on the
organisations satz sets up, with "grant yourself the role" as the only way
forward — which is the organisation-wide read access the service-account model
exists to avoid.

**Bind the estate's account for the bare form by looking for an estate that
declares this organisation.** *Rejected.* It is the same guess wearing a
justification: several estates may name one organisation id, an estate that names
none would be skipped for reasons the operator cannot see, and the identity would
depend on which files happen to be in `yaml_dir`. An identity that is inferred
from the filesystem is an identity nobody chose.

**Reuse `--into` for both jobs — sweep as the estate, write a new file when an
`--output` is given.** *Rejected.* `--into` means "write the delta into this
estate", and the difference between a delta of packs and a fresh file is the
whole result of the run. One flag with two results, selected by the presence of a
third, is a flag nobody can read at the command line.

## Consequences

- `import` is the second command after `whoami` whose identity depends on what it
  was given, and `IDENTITIES` has one more `HumanOrEstate` row. The binding site
  count is unchanged: one site serves both flags.
- An operator on an organisation satz did not set up, whose own credentials hold
  the role, is unaffected — the bare form is unchanged.
- `--as` is refused on the state and hcl shapes rather than ignored: they read a
  file and call no Google API, so there is nobody to be.
- `--generate-unmapped` on a bare sweep with `--as` writes
  `impersonate_service_account` into the provider block its `tofu` child reads
  with, as `--into` already did (ADR 0053, point 2) — one command, one identity.
- The binding cannot be exercised offline: the smoke matrix asserts the flag is
  accepted and that the run reaches the live sweep, and the derivation is
  unit-tested against a fixture estate (`import_identity`, `src/main.rs`).
