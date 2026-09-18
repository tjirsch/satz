# 0020 — a discovered estate binds the library's vocabulary, and says how

- **Status:** accepted
- **Date:** 2026-09-14
- **Shipped in:** v0.56.12

## Context

`satz init` writes sixteen params — the day-0 vocabulary of `presets/estate-core.satz`:
the organisation, the customer's domain and short name, the infra folder, project and
bucket, the IaC service account and its users group, the billing account, the region.
Every pack speaks it: a pack's grant is `"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com"`,
a pack's bucket is `"{customer_shortname}-organization-audit-logs"`. An estate
`satz import` wrote bound one of the sixteen, `customer_organization_id`, and carried
every other value as a literal, a hundred times over. Such an estate does not line up
with a written one and no pack drops in: the operator lifts the literals back into
params by hand, guessing which ones the library means.

The question was which literals become params and under which names. Two rules were
on the table: the attribute's own name (`location`, `org_id` — mechanical, no guessing,
and an estate whose params are named after attributes accepts no pack), or the
library's convention, recognised by what the values are. The library's convention was
chosen for the whole vocabulary, heuristics included, on 2026-09-14.

## Options

1. **Bind only what is certain** — the organisation id, and a billing account every
   project shares. Nothing guessed; the estate still carries the domain, the short name,
   the infra project and the service account as literals, and the packs still do not
   drop in.
2. **Bind the vocabulary by derivation and inference, each visible.** A value the
   platform states is bound as a fact; a value a rule over the discovered data chooses
   is bound with the rule beside it; a value nothing can say is left out and reported.
3. **Ask.** `init --from-live` prompts for the short name; an import could prompt for
   every value the platform does not state. Sixteen questions on every import, most of
   them answerable from the data.

## Decision

Option 2. Three classes, each visible in the file:

- **derived** — a fact the platform states, bound without comment: the ADC's identity
  gives `first_admin` and the domain, `organizations:search` gives `customer_id` and the
  organisation's display name (its primary domain), `billingAccounts.list` a single
  open account; the sweep's root gives the organisation. The live shape reads them
  through the same derivation `init --from-live` uses; the state shape has no ADC and
  binds none of them.
- **inferred** — a rule over the discovered data, bound with `// inferred: <rule and
  evidence>` on the line and one report line: the one service account granted
  `roles/resourcemanager.organizationAdmin` at the organisation gives `svc_iac_account`
  and `infra_project_name`; that project gives `infra_folder_name` (the folder holding
  it), `infra_bucket_name` (its one versioned bucket) and `billing_account_infra`; a
  group named `<account>-users` among the members gives `svc_iac_users_group`; the
  region most regional resources name gives `default_region` and `default_zone`; the
  leading token most project and bucket names share gives `customer_shortname`, unless
  `--customer-shortname` says — the one value no platform fact carries, which is why
  `init --from-live` prompts for it.
- **not derivable** — absent from `params`, one report line naming how to bind it.
  `customer_longname` always; a rule with two candidates names both and binds nothing
  downstream of it.

Every bound literal is then referenced wherever the document repeats it, member keys
included, the way the library spells it — the longest literal first, at identifier
boundaries only, a bare reference where a whole value is the literal. A project whose
billing account is the bound one drops the attribute; one without any keeps
`billing_account = ""`, or the emitter's fallback would attach the bound account on
apply.

## Consequences

- A discovered estate's `params` block reads like `init`'s, and a pack `use`d into it
  resolves its references to the same values the sweep found.
- An inference can be wrong; it is never silent. The file says which rule chose a
  value and on what evidence, and the report repeats it, so the review reads the notes
  rather than the whole file.
- The rules are judgements about the library's own conventions (the IaC account holds
  organizationAdmin, the state bucket is versioned, the short name leads the names);
  an organisation built another way gets fewer bindings and more "not derivable" lines,
  never a wrong-but-quiet one.
- `live_defaults` takes the sweep's organisation as a hint, so an identity that sees
  many organisations is no longer an error on import.
