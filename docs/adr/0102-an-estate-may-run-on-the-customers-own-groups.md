# 0102 — an estate may run on the customer's own groups

- **Status:** accepted
- **Date:** 2026-11-23

## Context

The map's `security_model` choice was required and had two answers, S1 and S2. Each is a
pack that creates five or six Cloud Identity groups and grants them their organization
roles. A customer whose admin groups and their grants exist already, and stay the
customer's to manage, had no answer: the groups could be renamed to the existing ones and
taken over with `satz adopt`, but then satz owns them — it rewrites their display name and
description, and removing the pack deletes them — and an address had to be
`<name>@<customer domain>`, cut into exactly the model's five or six roles.

Only one pack depended on a model: `billing-account-permissions.satz`, through a declared
`requires` on S1 or S2, although its own question already took any group's full address.

## Options

**How the answer is written**

1. **A named third option, `security_model_own`, that gates no pack.** The interview,
   satz-studio and the decisions sheet the customer signs read "The customer's own groups —
   satz creates no group and grants no admin role". Costs a narrow exception to the rule
   that every map param gates a pack line: an option of a choice may switch no pack, at
   most one per choice, and only beside options that do.
2. **The choice's built-in `none` answer** (the question not `required`). No exception to
   the rule; the answer reads "none" everywhere, and what it means lives only in the
   question's `why`.

**How billing account permissions stays usable**

1. **Its group default reads the model's name**, `{gcp_billing_admins_name}@{customer_domain}`,
   so `preset-graph` derives a `data` edge to both models and the declared `requires` goes.
   A `data` requirement is also met when the estate binds the pack's own param whose
   default reads it, so an estate on its own groups answers `billing_admins_group`.
2. **Drop the requirement altogether.** A grant to a group that does not exist reaches the
   apply before anything says so.

## Decision

Option 1 in both. `estate_map` 5.2 offers `security_model_own`; `billing_account_permissions`
1.3 reads the model's group name. The refusal of a `data` requirement names the pack's
params that the estate may bind instead ("it reads `gcp_billing_admins_name` through
`billing_admins_group`, which the estate may bind instead"), in the transpile, `presets`
and `add-preset` alike.

## Consequences

- An estate on the customer's own groups creates, grants and imports no group; the
  customer's grants are untouched, since every organization-level grant in the library adds
  a member and none sets a role's full member list.
- The domain keeps Project Creator and Billing Account Creator: the estate declares no
  holder of either, so `satz apply` has nobody to leave them with
  ([ADR 0081](0081-the-domain-default-grants-are-removed-by-the-apply-not-by-a-binding.md)).
- An estate on S1 or S2 transpiles exactly as before.
