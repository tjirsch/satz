# 0067 — an empty string is kept only on a required attribute

- **Status:** accepted; supersedes decision 1 of ADR-0064
- **Date:** 2026-09-24
- **Shipped in:** the release that follows

## Context

ADR-0064 made an import keep `""` on every attribute the provider schema names, so
that `google_pubsub_subscription.expiration_policy.ttl = ""` — never expires —
survives. The reasoning was that the schema tells a value from vocabulary.

The schema does not tell a value from an UNSET attribute. A Terraform state writes
every attribute of a type, and an optional string nobody set is `""` there. Under
ADR-0064 each of them reached the estate. Most are harmless: the provider's
generated resources accept `""` on an optional enum
(`google_compute_subnetwork.ipv6_access_type`, `stack_type`, `purpose`, a bucket's
`rpo` and `public_access_prevention` — `tofu validate` against google 7.14.1
passes). Hand-written ones do not: `google_compute_instance`'s
`network_interface.nic_type = ""` fails validation — `expected
network_interface.0.nic_type to be one of ["GVNIC" "VIRTIO_NET" "IDPF" "MRDMA"
"IRDMA"], got` — so a state import of any compute instance wrote an estate whose
plan fails.

The attribute ADR-0064 was written for is REQUIRED inside its block: `ttl` cannot be
omitted from `expiration_policy`, and the block is what carries the meaning.

## Decision

**An empty string is kept on a required attribute and dropped on an optional
one.** On an optional attribute the provider (SDKv2, which almost every google
resource is written in) reads an unset string as `""`, so leaving it out is the
same setting, and it cannot be refused. On a required attribute the empty string
is the only way to say it, and it stays. Decision 2 (null, `[]`, `{}` dropped on any
key), 3 (the `translate_api_values` rows) and 4 (nothing invented) of ADR-0064
stand.

## Options

**Keep `""` on every attribute the schema names** (ADR-0064). *Rejected now.* It
writes values the provider refuses at plan time, on the most common resource a
state holds.

**A table of attributes whose validation refuses `""`.** *Rejected.* The schema
JSON carries no validation; the table would be read off the provider's Go source
per release and be wrong the day a resource is rewritten. ADR-0064 rejected a
per-type list for the same reason.

**Read enum-ness off the description ("Possible values: …").** *Rejected.* The
generated resources write `Possible values: ["A", "B"]` and accept `""`; the
hand-written `nic_type` writes `Possible values:GVNIC, …` and refuses it. Prose is
no contract.

**Keep `""` only when the block is otherwise set.** *Rejected.* `nic_type` sits in
a `network_interface` that is always set.

## Consequences

- A state or live import writes no `attr = ""` on an optional attribute; the
  imported estate plans as the provider validates it.
- `expiration_policy { ttl = "" }` is still carried from both shapes.
- An attribute implemented in the plugin framework, which does tell `""` from
  null, loses an explicit `""` on import. None is known among the imported types;
  one found is a `translate_api_values`-style row stating that attribute, with the
  test that measured it.
