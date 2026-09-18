# 0007 — the map is a pack of choices, and the estate carries the `use` lines

- **Status:** accepted; its "the CIS baseline is not a choice" is superseded by ADR-0028,
  which makes the baseline the map choice `use_cis_baseline`; its "the map declares the
  choices and nothing else" is superseded by ADR-0031, in which the map also carries one
  `offers` entry per pack and the pack graph is written from them
- **Date:** 2026-09-10
- **Deciders:** the maintainer

## Context

Every pack now asks what a customer decides ([ADR 0006](0006-an-answer-is-a-param-the-estate-binds.md)
and the five content PRs after it). What was missing was the step before: **which
packs**. An estate is composed from the library, and that composition is itself a set
of decisions — the security-group model, whether to archive audit logs, whether
Defender is in play. The interview needs a path: the day-0 params, then the choice of
packs, then the questions of every pack the choices switched on.

Two facts constrain where such a map can live. A question that gates a pack cannot
live in the gated pack (questions are absorbed after the `use … when` guard). And a
`use … when` on a param nobody declares is an error by design — a typo must not
silently drop a pack — so the param a `when` names has to be declared by something
the estate always uses.

## Decision

**`presets/estate-map.satz` declares the choices and nothing else**: one boolean per
optional pack, the S1/S2 model as a `question oneof`, each with a `question` whose
`why` is what the pack is for and what turning it off later destroys. **The estate
carries the matching `use … when` lines**, one per choice, in the map's order. The
interview skeleton (`satz interview --create`, `satz_interview {create}`) writes them;
a test asserts the skeleton names every param the map declares and nothing the map
does not.

**The CIS baseline is not a choice.** It is what the estate is for; its seven opt-in
extensions are the baseline pack's own questions, and the skeleton carries their
`when` lines because the baseline's params are always declared.

**Recommended packs default on** (audit archive, central alerts, billing permissions,
essential contact); **optional ones default off** (budget, SCC enablement, security-
audit account, Defender, verification runner). Every choice is a question, so every
choice is bound in the estate before bootstrap, whichever way it went.

## Consequences

- The interview path is the file order: estate-core, estate-map, the baseline, then
  the chosen packs — each `use … when` that holds pulls a pack's questions in.
  Switching a choice re-shapes the rest of the interview; the report re-reads after
  every answer.
- The estate stays the record. A reader sees which packs it uses; a fork repoints its
  own line; `check-presets` and `adopt` work per pack as before.
- The map's list and the skeleton's lines are two places. That is the cost of
  Option B, paid with a test, not with a second mechanism.
- **Hand-wired, by design:** Defender's plan fragments (gated on Defender's own params,
  which are undeclared while Defender is off), the MSP-hosted runner shape (grant here,
  runner in the MSP's estate), the per-project alert pack (the central one covers every
  project). Each is named in the map's header.
- Central alerts need the audit archive: both default on, and an estate that switches
  the archive off must switch the alerts off too, or the compile stops with `unknown
  param 'logsink_project_name'` — the failure the alerts pack's own header calls the
  right one. An `ask_when` on the alerts choice was considered and rejected: it would
  hide the question while leaving the pack on.
- `estate-core` 2.0 loses the model choice to the map, so it is day-0 params only. No
  estate in the fleet uses either pack; both exist for skeletons.

## Pros and cons of the options

### A — the map pack carries the `use … when` lines itself

The estate would say `use "presets/estate-map.satz"` and nothing more.

- **Good:** one line in the estate; a pack added to the library reaches every estate
  through a preset update.
- **Bad:** the estate no longer shows what it uses; a reader has to open the map.
- **Bad:** a fork of one pack (`X.local.satz`) has to repoint a `use` that lives inside
  the map — so it has to fork the map.
- **Bad:** packs whose shape is decided by the estate — inside a folder, under a typed
  key — cannot be placed from a top-level pack.

### B — the map declares the choices; the estate carries the lines *(chosen)*

- **Good:** the estate is explicit and remains the record; forks and per-pack tooling
  are unchanged.
- **Good:** each pack keeps its own `use` shape — the logging packs inside the
  infrastructure folder, the contact under its typed key.
- **Bad:** two places for one list. A test keeps them equal.

### C — no map: each pack asks whether it is wanted

- **Bad:** impossible as stated. Questions are absorbed after the `when` guard, so a
  pack that is off cannot ask; and a `when` on a param the off pack declares is an
  unknown param. The rule "a question that gates a pack cannot live in the gated pack"
  is a consequence of the pipeline, not a preference.
