# 0064 — an empty value the provider schema names is a value

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

An import filters what the source hands it (`filter_values` /
`filter_recursive`, `src/discovery.rs`). Cloud Asset Inventory returns the API's
own document and a Terraform state returns every attribute of the type, most of
them unset, so the filter drops what carries nothing: null, `[]`, `{}` and `""`,
at every level.

`""` is not like the other three. The provider uses it as a VALUE where the API
uses an absent field: `google_pubsub_subscription.expiration_policy.ttl = ""`
means the subscription never expires, and the schema's own description says so —
"If it is set but ttl is "", the resource never expires." The default, when the
block is absent altogether, is Google's 31 days, after which the subscription
deletes itself.

Measured on a live organisation with v0.77.0: a subscription set to never expire
came back from the state shape and from the live shape without its
`expiration_policy`. The state carried `expiration_policy = [{ ttl = "" }]` and
the empty string was dropped, leaving an empty block, which was dropped in turn.
The API states the same fact by leaving the message empty
(`"expirationPolicy": {}`) — proto3 omits an unset field — and an empty mapping
was dropped as carrying nothing.

Applied, either estate restores the 31-day default on a subscription somebody
chose to keep.

## Decision

**A value the provider schema names is kept, empty or not; a value on a key the
schema does not name is dropped as before.**

1. **An empty string on a schema attribute stays** (`filter_recursive`'s final
   pass). The schema is what tells a value from vocabulary: an attribute the
   provider has is a value somebody can set, and the empty string is one of its
   settings. A key neither the attributes nor the block types name is API
   vocabulary the provider does not speak, and an empty one carries nothing.
2. **Null, `[]` and `{}` are still dropped, on any key.** They are how a source
   says nothing. The one exception is stated per type, below.
3. **Where the API states a fact by leaving a block EMPTY and the provider by an
   attribute set to `""`, the pair is data** (`translate_api_values`, beside the
   `isLive` → `with_state` row): `google_pubsub_subscription` at
   `expiration_policy.` stands for `ttl`. That table is per type and per path
   because the correspondence is, and each row is read by a test from the shape
   the source delivers.
4. **Nothing is invented for a block the source does not carry.** A subscription
   that says nothing about expiry says nothing in the estate, and Google's
   default applies — as it did before the import.

## Options

**Keep every empty value.** *Rejected.* The API returns empty strings for fields
that are not set at all, on keys the provider does not have; written into the
estate they are vocabulary errors (`F5`) and noise in a file meant to be read.
The schema is the line between the two and it costs nothing to ask.

**Keep `""` for `google_pubsub_subscription.expiration_policy.ttl` by name.**
*Rejected.* The same shape exists wherever a provider spells an absent API field
as the empty string, and a list of type-and-attribute exceptions is a list
somebody has to add to after each release — quietly wrong until they do. The
schema already knows which keys exist.

**Read the meaning out of the schema description.** *Rejected.* The description
does say it — "If it is set but ttl is "", the resource never expires" — and it
is prose the provider is free to reword. A gate that reads it fails silently the
day it changes.

**Derive `""` for any required string attribute of a block the source carries
empty.** *Rejected as the general rule.* It reads as proto3 semantics (an unset
string field IS `""`) and it invents a value: a block that arrives empty for
another reason gets an attribute nobody set, which the next plan changes. The
translation table states the one case that was measured.

## Consequences

- An imported estate carries `expiration_policy { ttl = "" }` where the
  subscription never expires, from the state and the live shape alike.
- An estate imported before this release is short that block. Nothing rereads it;
  `presets/README.md`'s `## Breaking changes` says what to write.
- Other attributes the provider spells `""` are carried from now on wherever the
  source states them. Where the source does not state them, nothing changes.
- The cost is the estates that gain a line they did not have: a source that
  carries `""` on a schema attribute writes it, which is a value the plan then
  holds to. That is the point of it, and a value nobody wants is one line to
  delete.
