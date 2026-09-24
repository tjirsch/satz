# 0069 — an import given an estate sweeps only that estate's organisation

- **Status:** accepted
- **Date:** 2026-09-24
- **Shipped in:** the release that follows

## Context

`satz import <scope> --into A.satz` sweeps the scope as A's IaC service account
and writes what A does not declare into packs A `use`s; `--as A.satz` sweeps as
the same account into a new file. Nothing compared the scope with A. `satz import
organizations/<other> --into A.satz` read another organisation — as A's account,
wherever that account can read — and wrote its resources into A, where the next
apply would try to manage them from A's state.

A second gap sat beside it: `--as` naming a local-mode estate, or combined with
`--no-impersonate`, bound no account at all. The sweep ran as the caller's
credentials while the command line named an estate, and a refusal then said "no
estate was named".

## Decision

**A live import given an estate (`--into`, `--as`) sweeps the estate's
organisation or a scope inside it, and nothing else — checked before the sweep.**

- The organisation is the estate's `customer_organization_id`, read off its params.
  An estate that binds none is refused: there is nothing to compare.
- `organizations/<n>` is compared as written. A `folders/<n>` or `projects/<id>`
  is walked up through Resource Manager v3 (`folders.get` / `projects.get`,
  following `parent`) as the identity the run is bound to, to the organisation
  above it. A walk that cannot be read — denied, not found, no organisation above
  it — is a refusal too.
- **`--as` borrows an account or is refused.** A local-mode estate, or any estate
  under `--no-impersonate`, has none to lend; the refusal says so and names the
  bare form, which reads as the caller. `--into` runs as whatever its estate runs
  as — the account, or the caller's credentials for local mode or under
  `--no-impersonate` — and the run prints which before the first request.

## Options

**Check after the sweep, from the assets' ancestors.** *Rejected.* The sweep of
the wrong organisation has then already run, as the estate's account, and every
asset carries the answer only if the sweep returned any.

**Cloud Asset `searchAllResources` for the ancestry.** *Rejected.* It needs
`cloudasset.assets.searchAllResources` on the scope, a second permission for a
question Resource Manager answers with `resourcemanager.folders.get` /
`projects.get`, which the IaC account holds on its organisation.

**Warn and sweep.** *Rejected.* A warning above a sweep that writes another
organisation's resources into an estate is read after the write.

**Refuse `--into` on a local-mode estate as well.** *Rejected.* A local-mode estate
applies as the caller's credentials; its sweep reading as the same credentials is
consistent, and the run says so.

## Consequences

- An import into or as an estate costs one Resource Manager read per level between
  a folder or project scope and the organisation; an organisation scope costs none.
- An estate without `customer_organization_id` can no longer be named by `--into`
  or `--as`; it binds the param first.
- `--as` on a local-mode estate is refused; the bare form is the same sweep.
- The refusal and the start of every sweep name the identity it runs as, with the
  reason when it is the caller's.
