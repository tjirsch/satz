# Competitive reference — promote & defend

Standing reference for positioning Cloud Cockpit against other frameworks.
Detailed technical diffs live in their own audit docs (currently
[fast-delta.md](fast-delta.md)); this file holds the landscape, the arguments,
and the battle-review log. Keep ALL framework inputs — every audit adds a row
and its raw findings stay in the repo.

## Landscape (as of 2026-08-22)

| Contender | What it is | CIS/OSCAL mapping | Brownfield | Multi-customer | Evidence plane |
|---|---|---|---|---|---|
| **Fabric FAST** (Google) | Greenfield landing-zone, YAML factories (`0-org-setup` + stages) | none | no (greenfield, prefix-named) | no — one repo per org, no upstream merge machinery | none (observability factory ships empty) |
| **GCP Hardening Toolkit** (Google, 2025-12) | Gemini CLI LLM agent over CAI/SCC exports + blueprint library | none (HIPAA/SOC2/PCI bundles) | yes (remediation blueprints, no import into managed estate) | no | none |
| **compliance.tf** | Paid CIS-enforced TF module library | CIS, module-level | no | n/a | none |
| **OSCAL ecosystem** (iac2oscal, GRC tools) | Mapping examples & documentation tooling | OSCAL, docs-side | n/a | n/a | GRC reporting, not provisioning |
| **Cloud Cockpit (us)** | CIS-mapped org foundation compiled from Satz estates | claims → CIS 4.0/5.0 catalogs | yes — import into an ongoing managed estate | yes — presets + merge-presets/fork/ledger | `require` goal view + `report-compliance` live witnesses |

## Core arguments

1. **Determinism is auditable.** Interview → derive → folded-IR check is
   replayable; an auditor can re-run the derivation. A Gemini transcript (GHT)
   cannot be replayed or certified. LLM assistance belongs *on top of* a
   deterministic core, not in place of one (see roadmap: post-stabilization
   targeted exploration).
2. **Multi-customer maintenance is the moat** (owner, 2026-08-22). Running N
   customers on FAST means N diverged copies of the FAST repo; diffing preset
   drift across them is a nightmare — there is no upstream/fork/ledger
   machinery, no `merge-presets`, no transpile-identity proof that an upgrade
   is behavior-preserving. Our provenance model (`X.satz` pristine /
   `X.local.satz` fork / `X.diff.satz` ledger, versions in-file) exists
   precisely for fleet-wide customer-driven individual configs.
3. **Escape hatches are designed, not improvised** (owner, 2026-08-22). If you
   need an exception FAST does not ship (a policy carve-out, a per-customer
   deviation), you are editing a stage in your private copy — unmergeable mess.
   We have graded channels: params (80%), `suppress`, `.local` forks with
   adoption ledger, and (roadmap) tag+condition-based temporary policy lifts.
4. **Compliance content, shipped working.** §2.1 data-access audit config
   (FAST: literal `# TODO`), §2.4–2.12 alert stack live-proven (FAST ships it
   commented out), claims on every emitted resource.
5. **Brownfield means import, not just remediation.** Import-ids/import blocks
   bring existing resources under the estate; GHT generates fix-blueprints but
   leaves no coherent managed estate behind.

## Watch list

- **GHT** is the competitor to watch: Google + Gemini + free + brownfield
  framing. Its existence validates the category. Track whether it gains CIS
  mapping, import, or multi-org support.
- FAST factory/context-interpolation design converging on our param model —
  watch for FAST growing fork/upgrade tooling.

## Battle-review log

Recurring step (see ROADMAP): re-audit the landscape, challenge our strategy
and abilities against it, append a dated entry here.

- **2026-08-22** — initial audit (FAST delta + kill-check, docs/fast-delta.md).
  Verdict: no kill; category validated by Google's GHT entry; differentiators
  sharpened (determinism, multi-customer maintenance, escape hatches, evidence
  plane). Actions: Phase 6 rollout integration on roadmap, tag-conditional
  policy-lift feature on roadmap, LLM targeted-exploration step on roadmap.
