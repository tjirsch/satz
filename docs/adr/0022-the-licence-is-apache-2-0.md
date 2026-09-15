# 0022 — the licence is Apache 2.0

- **Status:** accepted
- **Date:** 2026-09-15
- **Deciders:** the maintainer

## Context

satz was MIT from its first commit — the licence a Rust template offers, chosen before
the project had users. What satz does now changes the question it has to answer: it
compiles a customer's cloud estate, runs as that estate's IaC service account, applies
org policies across an organisation, and produces the evidence an auditor reads. Its
users are enterprises, and it reaches them through a legal review that reads the
licence before anyone runs the binary.

MIT is silent on patents. It grants copyright permission and says nothing about
whether a contributor's patents travel with the code, and a reviewer cannot treat
silence as a grant — an unresolved question is a finding, and a finding is an
escalation. MIT is also silent on the terms an inbound contribution arrives under,
which is why MIT projects bolt a contributor agreement on the side. This repository
papered over that gap with one sentence in `CONTRIBUTING.md` asserting terms the
licence does not contain; a sentence in a markdown file is not a grant.

Relicensing is possible right now and will not stay possible. Every commit in the
history was authored and committed by the maintainer, there are no outside
contributors and no forks, and nothing is published to crates.io (`publish = false`
in each crate manifest and in `release.toml`), so the copyright holder can relicense
alone. The first outside commit closes that window.

## Considered options

1. **Stay on MIT.** Shortest text, widest compatibility, no work.
2. **`MIT OR Apache-2.0`** — the Rust ecosystem's usual dual licence, which keeps
   maximum compatibility and lets the downstream user choose the terms.
3. **Apache 2.0 alone** — the express patent grant, the contribution terms in §5, and
   a `NOTICE` file that travels with redistribution.

## Decision

Option 3. `LICENSE` is the canonical Apache 2.0 text verbatim, appendix included, and
`NOTICE` at the root carries `Copyright 2026 Thomas Jirsch` beside the standard
licence paragraph and names the third-party material this repository ships. Every
manifest reads `license = "Apache-2.0"`; `editors/zed/LICENSE` is the same text,
because Zed's registry requires a licence file inside the extension directory.
`CONTRIBUTING.md` points at §5 instead of asserting terms of its own.

The dual licence is rejected for the reason it exists. `MIT OR Apache-2.0` is the
convention for library crates on crates.io, where it lets a crate be linked into
GPLv2 code that Apache 2.0's patent-termination clause would otherwise exclude; satz
is a binary nobody links, published to no registry. What the dual form adds here is a
choice, and the choice on offer is the arm with no patent grant and no `NOTICE` —
the version that gives a legal review nothing to approve. Moving to gain a guarantee
and then making it optional gains nothing.

## Consequences

- **`NOTICE` now travels with redistribution** (§4(d)), and it is only as true as the
  last change to the tree. Material that arrives from outside the project gets its
  entry in the same change; nothing checks that, so it is a row in
  `docs/housekeeping.md` beside the other derived files.
- **The text is 202 lines where MIT was 21.** Someone looking up the terms reads a
  document rather than a paragraph. That cost is paid to an audience that reads
  licences professionally, which is the audience satz has.
- **The licence now states what MIT left open:** an express patent grant from every
  contributor, terminating for anyone who brings a patent claim against the work, and
  the inbound contribution terms in §5 — so there is no contributor agreement to
  draft, sign or chase, and the project asks for nothing beyond the pull request.
- **The privacy gate learned `apache.org`.** The licence text carries
  `http://www.apache.org/licenses/LICENSE-2.0`, and `scripts/check-names.sh` rejects
  every domain that is not on its allowlist; without the entry the gate fails on
  `LICENSE` itself.
- **Changing the licence again is another relicensing act**, and cheap only while the
  maintainer is the sole copyright holder. The first contribution that lands under
  Apache 2.0 makes the next change a matter of asking whoever wrote it.

## Pros and cons of the options

### 1 · Stay on MIT

- **Good:** 21 lines, read in a minute, and compatible with everything including
  GPLv2.
- **Good:** no work, and no `NOTICE` file to keep true.
- **Bad:** silent on patents. A reviewer cannot tell whether use is covered, and
  "unresolved" is the answer that costs a deployment weeks.
- **Bad:** silent on inbound contributions, so the terms had to be asserted in
  `CONTRIBUTING.md` — outside the licence, where an assertion carries no weight.

### 2 · `MIT OR Apache-2.0`

- **Good:** the Rust ecosystem's convention, and the widest compatibility of the
  three: the MIT arm reaches GPLv2 code that Apache 2.0 cannot.
- **Good:** nothing downstream is ever blocked on the licence, because the user picks
  the arm they can live with.
- **Bad:** the choice is the defect. A user who takes MIT takes no patent grant and
  carries no `NOTICE`, so the guarantees exist only for whoever opts into them — and
  the reason for moving was to stop the question being open.
- **Bad:** it buys a compatibility satz has no consumer for — the convention serves
  crates being linked, and satz is a binary with `publish = false` everywhere — at the
  price of two licences to state in every manifest and every conversation about what
  satz is under.

### 3 · Apache 2.0 alone *(chosen)*

- **Good:** §3 is an express patent grant from every contributor — the clause a legal
  review is looking for — and it terminates for anyone who brings a patent claim
  against the work. MIT has neither the grant nor the defence.
- **Good:** §5 states the contribution terms, so no separate agreement is needed.
- **Good:** the licence enterprise legal review approves without discussion, which is
  what a tool pointed at a customer's own organisation needs it to be.
- **Bad:** long, and a `NOTICE` file that every redistributor must carry and this
  project must keep accurate by hand.
- **Bad:** incompatible with GPLv2, though compatible with GPLv3. Nothing wants to
  link satz into GPLv2 code today, and a binary is not linked at all, but the door is
  shut.
