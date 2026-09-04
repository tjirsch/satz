# Example customers — the only identifiers allowed in docs, examples and tests

This repository is public. Real customer names, domains, Google Workspace
directory ids (`C0…`), GCP organisation, folder and project numbers, billing
account ids and repository URLs must never appear in a tracked file. Instead,
**every example uses one of the four customers below, with exactly these
values.** `scripts/check-names.sh` enforces it: any `C0…` id, any 11–13-digit
number, any billing-account id, any GUID, any 32-hex string, any project id, any
e-mail or `.de` domain that is not on this page fails the commit and the CI run.

All domains are IANA-reserved (`example.com/net/org`, the `.example` TLD), so
nothing here can resolve. All numbers are visibly synthetic.

| | Customer A | Customer B | Customer C | Customer D |
|---|---|---|---|---|
| full name | Acme Corp. | Bolt Industries GmbH | Cedar Logistics AG | Delta Clinics |
| `customer_shortname` | `acme` | `bolt` | `cedar` | `delta` |
| `customer_domain` | `example.com` | `example.org` | `example.net` | `delta.example` |
| contacts domain (if different) | `example.net` | – | – | `mail.delta.example` |
| directory id (`customer_id`) | `C0example` | `C0bolt002` | `C0cedar03` | `C0delta04` |
| organisation id | `123456789012` | `222222222222` | `333333333333` | `444444444444` |
| billing account | `012345-6789AB-CDEF01` | `0B0B0B-0B0B0B-0B0B02` | `0C0C0C-0C0C0C-0C0C03` | `0D0D0D-0D0D0D-0D0D04` |
| infra project | `acme-infra-001` | `bolt-infra-001` | `cedar-infra-001` | `delta-infra-001` |
| log project | `acme-log-001` | `bolt-log-001` | `cedar-log-001` | `delta-log-001` |
| audit bucket | `acme-organization-audit-bucket` | `bolt-organization-audit-bucket` | `cedar-organization-audit-bucket` | `delta-organization-audit-bucket` |
| folder id (for `import-id` examples) | `123456789` | `222222222` | `333333333` | `444444444` |
| project number | `100000000001` | `200000000002` | `300000000003` | `400000000004` |
| estate file | `yaml/C0example.satz` | `yaml/C0bolt002.satz` | `yaml/C0cedar03.satz` | `yaml/C0delta04.satz` |
| repo path in prose | `~/estates/acme` | `~/estates/bolt` | `~/estates/cedar` | `~/estates/delta` |
| Microsoft Entra tenant id | `11111111-1111-1111-1111-111111111111` | `22222222-2222-2222-2222-222222222222` | `33333333-3333-3333-3333-333333333333` | `44444444-4444-4444-4444-444444444444` |
| workload identity pool id (the tenant id without dashes) | `11111111111111111111111111111111` | `22222222222222222222222222222222` | `33333333333333333333333333333333` | `44444444444444444444444444444444` |

## Vendor default identifiers

Some GUIDs are not anybody's secret: Microsoft and Google publish them, and every
customer's estate carries the same value. Those are allowed by name, listed here and
in `ALLOW_GUID` in the gate.

| identifier | what it is |
|---|---|
| `33e01921-4d64-4f8c-a055-5bdaffd5e33d` | **Microsoft's commercial-cloud tenant.** The `sts.windows.net/<tenant>` issuer every Defender for Cloud and Sentinel connector federates from — published in Microsoft's connector documentation for AWS and GCP alike, identical for every customer. It is *not* the customer's tenant, which is why it is allowed. |
| `d17a7d74-7e73-4e7d-bd41-8d9525e86cab` | Defender for Cloud's auto-provisioner application id, the `api://…` audience of the auto-provisioner OIDC provider. |
| `6e81e733-9e7f-474a-85f0-385c097f7f52` | Defender for Cloud's CSPM application id, the audience of the CSPM provider. |

**Adding one:** confirm the vendor publishes it and that it does not vary per
customer — a tenant id that appears in the customer's generated script is theirs, not
the vendor's, and belongs in a param. Then add it to `ALLOW_GUID` and to this table
in the same commit. A GUID the gate does not know is assumed to identify a customer.

## Legacy fixture placeholders

`corp-infra-001` and `corp-log-infra-001` are project ids in the smoke estate and the
corpus fixtures, from before this page existed. They are fictional and allowed, but
they are not a fifth customer: **new examples use the four above.** The same applies
to the directory ids `C01234567` and `C0abcd123`.

## What the gate cannot see

Shapes are enforceable; names are not. A project's or folder's DISPLAY name, a
company name in prose, a person's name in a comment — `"Log Admins"` and a real
customer's project name are the same kind of string, and no pattern separates them.
Two things cover that gap:

- **Review.** The rule is that no customer, company or person is named in a tracked
  file. The gate cannot check it; the author can.
- **The local denylist.** `$NAMES_DENYLIST` (or
  `~/Documents/thomas01/satz-core-history-rewrite/denylist.txt`) holds one extended
  regex per line — real customer names, project names, internal words. It is never
  committed, so CI stays structural while the pre-commit hook on the maintainer's
  machine knows the actual words to refuse. Anything that is a name rather than a
  shape belongs there.

**Which one to use.** Customer A for any single-estate example — it is the
one the existing docs already use. B, C, D only when an example needs a
second, third or fourth organisation (cross-org grants, a fleet table, a
partner directory). Customer A's separate contacts domain and Customer D's
subdomain exist so that "the org and the contacts live on different domains"
can be shown without inventing a value.

**Fleet estates in working-state files** (CLAUDE.md, ROADMAP.md, apply
worklists) use opaque codenames and never a domain, id or
path beyond `~/estates/E0n`. The mapping to real customers lives outside the
repository.

**Legacy placeholders, allowed but not to be spread further:** `C01234567`
(the README's original example directory id, also in older tests),
`C0abcd123` (one test), `A12345-B67890-C12345` and `123456-123456-123456`
(billing placeholders in README and a preset). New text uses the table.

**Domains.** Only IANA-reserved names (`example.com/org/net`, and the
`.example`, `.test`, `.invalid`, `.localhost` TLDs) and the vendor hosts the
project genuinely references (`googleapis.com`, `gserviceaccount.com`,
`github.com`, `opentofu.org`, `terraform.io`, `cisecurity.org`, and Microsoft's
`windows.net`, `microsoft.com`, `microsoftonline.com` — a workload-identity
federation example cannot avoid naming the issuer it federates, and
`sts.windows.net` is Microsoft's, not a customer's) may appear anywhere. Any
other domain fails the gate — including plausible-looking ones: the obvious
"fictional" company domains are real, registered businesses.

**Commit identity.** Every commit's author and committer must be the
maintainer's address or a GitHub noreply address
(`<id>+<user>@users.noreply.github.com` — enable "keep my email address
private" in GitHub settings). Employer or customer addresses are rejected by
the pre-commit hook and by CI on every push and pull request. This is not
about one person: no contributor's affiliation belongs in a public history.

**Other allowed identities:** `noreply@anthropic.com` (Claude's co-author
trailer), `*.iam.gserviceaccount.com` service accounts built from the values
above (e.g. `svc-iac@acme-infra-001.iam.gserviceaccount.com`), and
Google-owned agents of the form `service-org-<org-id>@…gserviceaccount.com`
with an org id from the table.

If an example genuinely needs a value this page does not define, add the
value here first, in the same commit.
