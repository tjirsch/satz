# 0080 — a project that switches Compute on gets no `default` network

- **Status:** accepted
- **Date:** 2026-10-21
- **Shipped in:** v0.93.0

## Context

Google creates the `default` network — auto-mode, one subnet per region, firewall rules
admitting SSH, RDP and ICMP from the internet — the moment the Compute Engine API is
switched on in a project, and tests the organisation policy
`compute.skipDefaultNetworkCreation` at that moment and never afterwards. CIS GCP
Foundations 3.1 is "the default network does not exist"; the CIS pack claims it with the
policy, enforced on the organisation, as its one witness.

A customer's new organisation came out of its first apply with a `default` network in the
infrastructure project. The estate used a pack that emits a hierarchical firewall policy,
so `compute.googleapis.com` stood in the infrastructure project's `project_service` list;
the first apply carried that `google_project_service` and the CIS pack's policy with no
edge between them. Three mechanisms let Compute on before the policy, and each had to be
read before the fix was certain:

- Inside the apply, `tofu` creates ten resources at a time; nothing ordered the Compute
  service behind the policy.
- Before the apply, the `plan`/`apply` preflight switches every API a billed project
  declares on imperatively ([ADR 0036](0036-plan-and-apply-enable-the-apis-the-estate-declares.md),
  [ADR 0072](0072-an-api-is-judged-on-the-project-its-provider-bills-to.md)), and
  `bootstrap` does the same for the infrastructure project on day 0. A graph edge inside
  the apply orders neither.
- The emitter never wrote `auto_create_network`, whose provider default is `true`.

The provider's behaviour, read from `resource_google_project.go`: `auto_create_network`
acts on CREATE only — it enables Compute on the new project, deletes the `default`
network and its firewall rules, and treats a 404 as done; it is set to `true` on import
and ignored on update, so `false` on an existing project is one in-place diff that
changes nothing live.

`report-compliance` verified 3.1 by the policy's value alone, so the row read
**verified** while the network existed.

## Considered options

For the project the emitter writes:

- **A · `auto_create_network = false` on every `google_project`.** Every project satz
  creates is clean. Cost: the provider switches Compute on in every project to delete a
  network that, without Compute, never existed — an API the estate did not declare, on
  the audit-archive project among others, and a `google_project_service` the compile
  does not know about.
- **B · `auto_create_network = false` where `project_service` names
  `compute.googleapis.com`, unless the estate states the attribute.** A project without
  Compute has no network, so nothing is said there. Cost: one in-place no-op diff per
  imported project that enables Compute — a plan that moves, so a minor.
- **C · Nothing: the policy and the ordering are enough.** Leaves every estate without
  the CIS pack, and every estate whose policy is suppressed, with the network.

For the order of Compute and the policy:

- **D · A `depends_on` from every Compute `google_project_service` to each policy the
  estate declares enforcing the constraint.** A graph edge, no plan change. Covers the
  apply and nothing before it.
- **E · D, and the preflight and `bootstrap` never switch Compute on imperatively while
  the estate declares the policy.** Simple. Cost: a brownfield import whose Compute
  resources are read through a project with Compute off cannot plan — the preflight
  exists for that read ([ADR 0072](0072-an-api-is-judged-on-the-project-its-provider-bills-to.md)).
- **F · D, and the preflight reads the organisation's effective policy first: Compute
  goes on where the constraint is already enforced, and is left to the apply otherwise,
  with the reason printed.** `bootstrap` defers on day 0 without reading, since nothing
  is live. Cost: one `getEffectivePolicy` read per preflight that finds Compute off, and
  one corner left to the operator: an import whose reads need Compute on a project while
  the policy is not yet live anywhere.
- **G · Switch Compute on and delete the `default` network satz caused.** Needs
  `compute.networks.delete` and `compute.firewalls.delete` on that project, roles the
  identity holds only when the estate emits networks there ([ADR 0009](0009-the-iac-service-account-holds-named-roles-never-owner.md)),
  and a second implementation of what the provider already does on create.

For the compliance plane:

- **H · The policy's value alone**, as before.
- **I · The policy's value and the networks:** beside a `compute.skipDefaultNetworkCreation`
  witness the report reads the organisation's `compute.googleapis.com/Network` assets,
  and a network named `default` in any project fails the witness, naming the projects.

## Decision

B, D with F, and I.

**B.** `emit_project` writes `auto_create_network = false` when the project's
`project_service` names `compute.googleapis.com` and the body does not state the
attribute. The condition is the service list, not the resources inside the project: the
network comes with the API, whatever is built on it.

**D and F.** `order_compute_after_skip_default_network` adds the edge to every policy
`Manifest::enforced_policies_on` returns — `name` ending in `/policies/compute.skipDefaultNetworkCreation`,
one unconditional rule `enforce = "TRUE"`, not `reset` — on the organisation or on a
folder; a policy declared off orders nothing. `prerequisites::compute_deferral` names the
same policy for the preflight, preferring one on the organisation because that is where
its enforcement can be read (`org_policy::effective_enforcement`, the same JSON shape the
compliance plane reads from Cloud Asset). `prerequisites::enable_declared_apis` then
leaves Compute to the apply unless the read says `enforce: true`, and reports it under
`deferred` with the reason; `bootstrap` records Compute as *Deferred to the estate*. A
failed read defers: deferring never creates a network, enabling might.

**I.** `is_skip_default_network_witness` adds `compute.googleapis.com/Network` to the
inventory the witnesses need, and `with_default_networks` turns the witness's verdict
into `Diverged` naming the projects — the row reads **NOT ENFORCED** whether the policy
is on, off or missing. A `deviates` claim on the control is left alone: the networks
prove the deviation rather than judging it.

## Consequences

- A project satz creates with Compute has no `default` network, policy or no policy. The
  provider deletes it on create where the policy did not prevent it.
- On an estate with the CIS pack, the first apply creates the organisation's policies
  before it switches Compute on anywhere; the Compute service waits behind the policy
  chain of its parent ([ADR 0036](0036-plan-and-apply-enable-the-apis-the-estate-declares.md)
  orders the policies on one parent), which is slower and correct.
- An imported project that enables Compute plans one in-place change, once; nothing
  moves live. The Breaking-changes entry for v0.93.0 says so.
- A `default` network that exists is never removed by satz. The compliance row names its
  project; the operator deletes it.
- One corner is the operator's: a plan whose reads need Compute on a project while no
  organisation policy is live yet stops on the read, and the preflight's line says what
  to do.
