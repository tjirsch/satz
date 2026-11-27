# 0110 — the delegation base creates the estate account user group

- **Status:** accepted
- **Date:** 2026-11-27
- **Amends:** [0101](0101-a-delegation-is-two-packs-and-a-workload-estate-has-no-rights-check.md)

## Context

`presets/delegation.satz` created, per delegation, the infrastructure project, the estate
account, the state bucket and the grants of the estate account user group on the account and
the project. The group itself was named by address in the entry and had to exist in the
customer's directory before the apply: someone with directory rights created it by hand, and
an address that did not exist failed the apply at the first grant. A delegation therefore
needed a step outside satz, by a person who is often neither of the two teams.

ADR 0101 split a delegation into a delegation pack in the organization's estate and a
workload pack in the workload estate, but did not say which of the organization-side
resources belong to the common base and which to each workload's delegation pack.

## Options

**Where the group is created**

1. **In the base, per entry of `delegations`.** Every delegation has exactly one estate
   account user group, whatever is delegated; the organization's estate account creates it.
   Costs: the organization's estate account needs the Groups Admin role in the Google admin
   console — which every estate the `init` template wrote holds already, since that template
   creates the organization's own estate account user group; an estate whose group exists
   imports it once with `satz adopt … --execute --import`.
2. **In each workload's delegation pack.** The group is then written once per kind of
   workload, by authors outside the organization's team, and the ready-made delegation packs
   of the library would each need it too.
3. **Outside satz, as before.** No Groups Admin needed; the hand step and its failure mode
   stay.

**Who is in the group**

1. **An optional entry field `estate_account_user_group_members`**, a list of e-mail
   addresses, empty by default; each becomes a membership. The members are values the
   estate binds — the pack names none — so who is in a group stays the customer's
   decision. A member added in the directory and not listed stays, unmanaged.
2. **No member field; members are added in the directory.** Keeps every membership out of
   the library's packs, as the S1 and S2 models do. The hand step returns for every member.

**The group's address**

1. **Required, as before.** An `entries` default is a value, a param or one field of the
   entry (ADR 0107); an address built from the entry's name and `customer_domain` would need
   a template default, a language change, and the library has no naming convention for a
   per-delegation group to derive one from.
2. **A derived default**, such as `<shortname>-deploy-<name>-users@<domain>`. Costs the
   language change above.

**The organization's estate account and the group**

1. **No membership.** The S1 and S2 models make the estate account the groups' owner; an
   owner is a member, and a member of this group may act as the delegation's estate account.
   The organization's estate account creates the group with Groups Admin and needs no
   membership.
2. **Owner, as in the models.** Gives the organization's estate account the delegation's
   rights for no purpose.

## Decision

Option 1 in all four. The rule behind the first: **what every delegation needs is created by
the base** — the estate account, its infrastructure project, its state bucket, its basic
grants and the estate account user group, whose only right is to act as the estate account and
read its project. **What one workload needs is in that workload's delegation pack** — the
resources it is given and the estate account's roles on them — which the organization's team
reviews. The organization's team applies the organization's estate as the organization's
estate account; no member of that team needs to be in a delegation's group.

`delegation` 2.2 writes, per entry, `google_cloud_identity_group.deploy_<name>_users` at the
entry's `estate_account_user_group`, with one membership per member listed. The group's grants
still name it by address; the emitter gives every grant on a group the estate declares a
`depends_on` on it, so the apply creates the group first.

Two groups with one address are refused at the transpile: a delegation that names a group the
estate already declares — its own estate account user group, a group of S1 or S2 — would
otherwise create it twice, and Google refuses the second.

## Consequences

- The plan of an estate with delegations adds one group per delegation and one membership
  per listed member. An existing group is imported once:
  `satz adopt <estate> --only google_cloud_identity_group,google_cloud_identity_group_membership --execute --import`.
- `satz update-prerequisites` names Groups Admin for an estate that uses the base, through
  the row of `google_cloud_identity_group` it already had. An estate on the customer's own
  groups (ADR 0102) that uses delegations now needs that role as well.
- The library's rule "memberships stay out of presets" holds in its sense — the library
  names no member — but a pack now writes memberships the estate lists; the rule's wording in
  the repository's working rules is the maintainer's to restate.
- Removing a delegation's entry deletes its group, with its memberships, at the next apply.
- The address stays required; a derived default waits for a template default in `entries`.
