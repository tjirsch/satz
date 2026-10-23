# 0081 — the domain's default grants are removed by the apply, not by a binding

- **Status:** accepted
- **Date:** 2026-10-23
- **Shipped in:** v0.94.0

## Context

A new Google Cloud organisation grants `roles/resourcemanager.projectCreator` and
`roles/billing.creator` to `domain:<customer domain>`: every user in the directory may
create projects and billing accounts, outside any estate. The security-group models grant
the two roles to the org-admins and billing-admins groups, the IaC service account holds
Project Creator through the estate's own grant map, and nothing takes the domain's entries
off. On the organisation this was found on, the domain held both roles months after the
estate went in.

The removal was asked for in the base preset, with no question, safe because the first
admin — the one human the estate names, who bootstraps as themselves — holds the roles
explicitly.

Terraform has no resource that says a member is absent. The provider's forms are additive
(`*_iam_member`: this member holds this role), authoritative per role (`*_iam_binding`:
exactly these members hold this role) and authoritative per node (`*_iam_policy`).

## Considered options

- **A · An authoritative `google_organization_iam_binding` per role in the models**, listing
  the group, the service account and the first admin. Declarative, in state, the domain
  leaves on the first apply. Cost: the binding replaces the role's whole member list —
  every grant a customer made by hand and every holder an organisation that pre-dates
  satz carries go with it, invisibly, since a create shows no "before". Rejected on that
  cost: the fleet runs on organisations satz did not create.
- **B · A removal inside `bootstrap`**, as the human, after self-granting the first admin.
  Runs once, on a new organisation only; an organisation that was bootstrapped before the
  release needs a separate run; nothing is in state, so a hand re-grant goes unnoticed; and
  bootstrap runs before the first apply, when no explicit holder is live.
- **C · A dedicated one-off command.** A step to remember, per organisation.
- **D · A step in the `plan`/`apply` preflight**, where satz already performs live steps
  before `tofu` ([ADR 0036](0036-plan-and-apply-enable-the-apis-the-estate-declares.md),
  [ADR 0080](0080-a-project-that-switches-compute-on-gets-no-default-network.md)): read
  the organisation's IAM policy, and for each of the two roles the estate declares a holder
  of, take the domain's entry off the role's unconditional binding once every declared
  holder is in it. Cost: an IAM write outside the HCL, on every apply that finds one, and a
  second apply after the model goes in before the domain's entries are gone.
- **E · D, writing on `plan` as well.** One run fewer. Cost: a plan that changes IAM.
- **F · A `report-compliance` row instead.** CIS GCP 5.0 has no control for the default
  grants, and a report removes nothing.

## Decision

D, with the explicit grants declared in the models.

**The grants.** `s1-security-groups.satz` and `s2-security-groups.satz` add
`"user:{first_admin}@{customer_domain}"` with the two roles to their
`google_organization_iam_member` map — additive, like every other grant in the library,
two creates in the plan. `first_admin` is a blocking day-0 question.

**The removal.** `remove_default_grants` in the binary runs after `enable_declared_apis`
for `plan` and `apply`, under the same `--no-api-preflight` switch and the same identity
(the impersonated service account in cloud mode, the human in local mode; both hold
`organizations.setIamPolicy`). It reads the organisation and the domain from the emitted
`terraform.tfvars` and the declared holders from `main.tf` (`default_grants::declared_holders`:
every unconditional `google_organization_iam_member` of the two roles). `default_grants::judge`
reads one role of the live policy: the domain does not hold it (nothing to do); the estate
itself declares the domain a holder (kept); the domain holds it under a condition only
(reported, not touched); a declared holder is not in the unconditional binding (left in
place, the holder named — the apply grants it); every declared holder is in it (removable).
`apply` takes every removable role's domain entry off in one `setIamPolicy` under the
policy's etag, re-reading and re-judging on a concurrent change, three attempts, and prints
each removal with the `gcloud organizations add-iam-policy-binding` line that restores it.
`plan` prints `apply would remove …` and writes nothing. A folder-scoped estate, a project
estate and an estate that declares no holder of either role are named and skipped; a policy
that cannot be read fails the command.

The step knows exactly one member — the customer's domain — on exactly two roles. It never
replaces a member list, never touches a conditional binding, never touches a holder the
estate does not name as a default.

## Consequences

- On a new organisation, the first apply with a model grants the first admin and leaves the
  domain in place, saying so; the next apply removes the domain's two entries, saying so.
  On an organisation that pre-dates the release the same two applies follow the upgrade.
  The Breaking-changes entry for v0.94.0 says so.
- A grant a customer made by hand on the organisation is not touched, whatever role it
  grants; the same holds for a domain entry an estate declares in its own grant map.
- `apply` performs an IAM write outside `tofu`'s state. It is idempotent, printed every
  time, and bounded to the one member and the two roles; the restore command is printed
  beside each removal.
- A domain entry somebody restores by hand is removed again on the next apply, with the
  line printed; the estate's grant map is where a wanted exception is declared.
