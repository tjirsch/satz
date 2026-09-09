# 0003 — evidence is data; the audit pack is not satz's to render

- **Status:** accepted
- **Date:** 2026-09-09
- **Shipped in:** v0.46.94

## Context

The plan carried a `report-compliance --audit-pack` command: a dated bundle for an
auditor — per-control verdicts, witness values, corroboration, the estate commit, a
shared-responsibility matrix, the deviation register.

satz's architecture says the opposite of building that. The tool states facts and never
calls a model; an agent asks for the facts over MCP and does the judging and the
authoring. `remediation-plan` already works that way — satz renders the mechanical half
and leaves the authored columns empty.

Measured against that, most of an "audit pack" is authoring: a cover page, a prose
matrix, a document shaped for a human reader, in whatever format the engagement wants.
Building it into satz would duplicate the agent, freeze one output shape, and put satz
in the business of formatting documents.

Meanwhile the data an agent would build it from was not fit for the purpose. The
evidence JSON's `witnesses` field was the **rendered markdown column**:

```
"witnesses": "google_org_policy_policy.iam_managed_allowedPolicyMembers → – (live
              inventory unavailable)<br><small>Domain Restricted Sharing, …</small>"
```

One field served a markdown table and an API response, and the API side lost: a consumer
had to parse presentation back out of the data.

## Considered options

1. **Build `--audit-pack`** as specified — satz renders the bundle.
2. **Add `--format audit`** to `report-compliance` — same content, one file, less surface.
3. **Make the evidence data complete and structured**, and let the agent author.

## Decision

Option 3. No new command and no new flag. Instead:

- witnesses become objects — address, live state, matched id, detail, and the
  `file:line` of the Satz that declares them;
- each control carries a `responsibility` — `inherited`, `customer`, `shared`,
  `satz-managed` or `unassigned`, derived from its goal;
- the report carries the estate's commit and whether the tree was dirty.

`satz_report_compliance` already returns this JSON, so the MCP surface gained all of it
without a new tool.

## Consequences

- An agent pulls one call and writes the Excel, the audit list or the remediation
  commands in whatever shape the engagement wants.
- The report's markdown column keeps its markup; the data no longer carries any. The
  rendered string is not duplicated into the JSON — one form of one fact.
- `declared_at` makes the claim no cloud-native compliance dashboard can make: not
  "the organisation has this policy" but "here is the code that put it there, at this
  commit".
- `unassigned` is a deliberate fifth value. An unmet control is nobody's yet; calling it
  the customer's would assign work no one agreed to.
- What is NOT solved: nothing renders an audit pack today. That is the point — but it
  means the first agent to build one is also the test of whether the data is complete.

## Pros and cons of the options

### 1 · satz renders the audit pack

- **Good:** one command produces something you can hand over; no agent required.
- **Good:** the format is version-controlled and reviewable like the rest of satz.
- **Bad:** it is authoring, which the architecture assigns to the agent — and satz would
  then have two ways to answer the same question, one of them frozen.
- **Bad:** auditors want different shapes. A rendered bundle serves the first engagement
  and is wrong for the second.
- **Bad:** it would have been built on the same unstructured data, so the defect above
  would have survived inside it.

### 2 · `--format audit`

- **Good:** cheapest of the three; no directory, no new command.
- **Bad:** same objection as 1, smaller. Still satz choosing the document's shape.

### 3 · Complete the data *(chosen)*

- **Good:** less code in satz, and it fixes a defect that was already live rather than
  adding a renderer that would need the same fix later.
- **Good:** the output format becomes whatever was asked for.
- **Good:** every consumer benefits — the MCP tool, `--format json`, the archived
  evidence history.
- **Bad:** there is no artefact to demonstrate. "The data is now sufficient" is harder to
  show than a generated bundle, and the claim stays unproven until an agent uses it.
- **Bad:** the markdown renderer and the fact builder walk the goal's witnesses
  separately. They cannot disagree about WHICH addresses — both read the same vector
  off the same goal — but they can drift in how each is described, and only the
  markdown side is read by a human who would notice.
