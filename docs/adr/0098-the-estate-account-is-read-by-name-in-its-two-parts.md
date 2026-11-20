# 0098 — the estate account is read by name, in its two parts

- **Status:** accepted
- **Date:** 2026-11-20

## Context

A workload estate runs as its delegation's estate account. satz names that account from
two params, `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`, in text of
its own: the providers' and the backend's `impersonate_service_account`, the project the
providers bill their calls to, the account the prerequisites are judged on, and the
identity every live call binds. The delegation's interface folder publishes the account
whole, `estate_account`, and its project, `infrastructure_project_id`.

`satz init --from` wrote the two params as literals copied out of the folder. A named read
([ADR 0094](0094-a-value-is-read-by-the-name-of-its-interface-folder.md)) was resolved in
resource bodies and in the `terraform` and `providers` blocks, not in a param satz
composes: `infra_project_name = "${{interface.delegation-shop.infrastructure_project_id}}"`
reached the HCL code as `svc-deploy-shop@$${interface.delegation-shop.infrastructure_project_id}…`,
and the prerequisites check judged that text. So an estate that wanted its account to
follow the organization's estate had no form that worked, and the literals went stale
silently when the delegation's folder was copied again after a change.

## Options

1. **Publish the parts and read each by name.** A new export `estate_account_id`, the
   account's id, beside `infrastructure_project_id`; the estate binds
   `svc_iac_account = "${{interface.delegation-<name>.estate_account_id}}"` and
   `infra_project_name = "${{interface.delegation-<name>.infrastructure_project_id}}"`.
   Needs satz to read a param as the value it reads wherever it composes it. Cost: one
   more export; a folder copied from an older transpile lacks it.
2. **One param for the whole address.** The estate binds the account as one value, read
   from `estate_account`. Cost: a third way to name the account beside the two params
   every organization's estate, every live command and satz-studio read; the project the
   calls are billed to is still a second param; every reader of the two params learns to
   prefer the third.
3. **Keep literals.** Cost: the account and the project are copied by hand after every
   change of the delegation, and nothing says when they are stale.

## Decision

- **Option 1.** `presets/delegation.satz` 2.1 exports `estate_account_id`.
- **satz reads a param that holds a named read as the export's value** wherever it reads
  params — the providers, the backend, the prerequisites, `whoami` and the identity a
  command binds. A resource body keeps the read, written as `local.interface.<…>`, as before.
- **A read the organization's estate looks up cannot name the account**: in
  `svc_iac_account` or `infra_project_name` it is refused, naming the param and the value,
  for the reason a provider refuses one — the account is named before any lookup is read.
  In any other param it stays as written and is judged where a resource holds it.
- **`satz init --from` writes `svc_iac_account`, `infra_project_name` and
  `infra_bucket_name` as named reads** of the delegation's folder, and refuses a folder
  without `estate_account_id`, saying to copy it again.

## Consequences

- An estate written by `init --from` follows the organization's estate when the
  delegation's folder is copied again; `CHANGES.md` in the folder says when the account moved.
- The HCL code names the account and its project as literals, and the `providers` block's
  own `project` and `billing_project` as `local.interface_static.<…>`; both are the same
  value.
- A workload estate written before keeps its literals and transpiles unchanged; reading
  the parts by name is an edit the team makes when it wants it.
