# satz competitive

How satz compares with other frameworks for a Google Cloud organisation foundation.
Detailed technical diffs are in their own audit docs
([fast-delta.md](fast-delta.md)); this page holds the landscape, the differences
and the review log. Every audit adds a row, and its raw findings stay in the
repository.

## Landscape (as of 2026-08-22)

| Contender | What it is | CIS/OSCAL mapping | Brownfield | Multi-customer | Evidence plane |
|---|---|---|---|---|---|
| **Fabric FAST** (Google) | Greenfield landing-zone, YAML factories (`0-org-setup` + stages) | none | no (greenfield, prefix-named) | no — one repo per org, no upstream merge machinery | none (observability factory ships empty) |
| **GCP Hardening Toolkit** (Google, 2025-12) | Gemini CLI LLM agent over CAI/SCC exports + blueprint library | none (HIPAA/SOC2/PCI bundles) | yes (remediation blueprints, no import into managed estate) | no | none |
| **compliance.tf** | Paid CIS-enforced TF module library | CIS, module-level | no | n/a | none |
| **OSCAL ecosystem** (iac2oscal, GRC tools) | Mapping examples & documentation tooling | OSCAL, docs-side | n/a | n/a | GRC reporting, not provisioning |
| **satz** | CIS-mapped org foundation compiled from Satz estates | claims → CIS 4.0/5.0 catalogs | yes — import into an ongoing managed estate | yes — presets + merge-presets/fork/ledger | `require` goal view + `report-compliance` live witnesses |

## Differences

1. **Deterministic derivation.** Interview → derive → folded-IR check can be
   re-run, so an auditor can replay the derivation. A Gemini transcript (GHT)
   cannot be replayed. satz calls no model; an agent uses it through MCP.
2. **Multi-customer maintenance.** N customers on FAST are N diverged copies of
   the FAST repository, with no upstream/fork/ledger tooling, no `merge-presets`
   and no proof that an upgrade preserves behaviour. satz keeps provenance per
   pack — `X.satz` pristine, `X.local.satz` fork, `X.diff.satz` ledger, the
   version in the file — and `merge-presets` upgrades an estate with a
   transpile-identity proof.
3. **Exceptions.** An exception FAST does not ship (a policy carve-out, a
   per-customer deviation) is an edit to a stage in a private copy, and later
   upstream changes do not merge into it. satz has params, `suppress`, `.local`
   forks with an adoption ledger, and `deviates` claims.
4. **Compliance content.** satz ships the §2.1 data-access audit config (FAST: a
   literal `# TODO`) and the §2.4–2.12 alert stack, verified live (FAST ships it
   commented out), with claims on every emitted resource.
5. **Brownfield import.** Import ids and import blocks bring existing resources
   under the estate. GHT generates fix blueprints and leaves no managed estate.

## Watch list

- **GHT**: Google, Gemini-based, free, aimed at brownfield organisations. Track
  whether it gains CIS mapping, import, or multi-org support.
- **FAST**: its factory/context-interpolation design is close to satz's param
  model. Track whether it adds fork/upgrade tooling.

## Battle-review log

Each re-audit of the landscape appends a dated entry here.

- **2026-08-22** — initial audit (FAST delta + kill-check, docs/fast-delta.md).
  Verdict: no kill; category validated by Google's GHT entry; differentiators
  sharpened (determinism, multi-customer maintenance, escape hatches, evidence
  plane). Actions: Phase 6 rollout integration on roadmap, tag-conditional
  policy-lift feature on roadmap, LLM targeted-exploration step on roadmap.
