# 0035 — an answer is what the estate binds, not what the library defaults to

- **Status:** accepted
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

ADR-0032 made the compile report a pack whose gate is answered `true` while its `use`
line is commented or absent: the answer is bound and nothing emits it, which is the one
state in which an estate quietly runs less than it says. The check read the gate through
the same resolution every other reader uses — the estate's own `params {}`, else the
default in the library file that declares the gate, while that file is used.

`presets/estate-map.satz` declares six gates `true` by default: `use_cis_baseline`,
`security_model_s1`, `use_audit_logsink`, `use_central_alerts`,
`use_billing_permissions`, `use_essential_contacts`. A skeleton from `satz init` carries
every pack as a commented line — day 0 is the scaffold alone (ADR-0031) — so the moment
the map's own line is uncommented, those six defaults become visible and the compile
reports six findings. At `validation_level = "error"` the estate is refused. Nothing is
wrong with it: it is the state `init` wrote, one step in.

## Considered options

1. **A default is not an answer.** The check reads only the estate's own `params {}`
   binding. The finding appears once the operator says yes — in the interview, through
   `add-pack`, or by hand.
2. **Keep the check on the resolved value and lower it to a note**, so the strict level
   stops refusing a fresh skeleton.
3. **Exempt the map's defaults by name**, keeping pack-declared defaults as answers.

## Decision

Option 1 — which is ADR-0006 ("an answer is a param the estate binds") applied to this
check. An answer is a decision this estate made, and the only place an estate records
a decision is its own `params {}`. That is already what `satz interview` binds, what
`satz add-pack` binds, what `satz merge-presets` binds (ADR-0033) and what `satz packs`
prints in its `answer` column beside the library's `default`. The finding is about a
contradiction the estate contains — it says yes and emits nothing — and a library default
is not the estate saying anything.

The resolved value is unchanged everywhere else: a pack still reads its gate through the
map's default, `satz packs` still shows both columns, and `deploys` is still what decides
whether a pack emits. Only which of the two the finding reads changes.

## Consequences

- A fresh skeleton with the map switched on compiles clean at `validation_level =
  "error"`. So does a fleet estate after `merge-presets`: the check can now only ever
  report fewer packs than before, never more, so no estate that passes today starts being
  refused. That is what the fleet's next `merge-presets` pass means for the strict level.
- A pack the library proposes by default and the estate has not taken is no longer named
  by the compile. `satz packs` is where it is read: the row carries the gate's `default`,
  the `answer` (empty), and `line: commented`. The interview asks the question, and the
  moment the answer is bound the compile reports the gap again.
- The wording follows: the menu comment `satz init` writes, `docs/workflows.md`,
  `docs/interview.md` and `docs/language.md` §6.16 all say the answer is the estate's own
  binding.

## Pros and cons of the options

### 1 — a default is not an answer *(chosen)*

- **Good:** one meaning of "answered" across the interview, `add-pack`, `merge-presets`,
  `satz packs` and the compile. A day-0 estate is clean at the strict level, and the
  check only ever becomes quieter, so no live estate changes verdict against it.
- **Bad:** a default-`true` gate whose line is commented is no longer reported by the
  compile — an operator who expects the library's proposal to be in has to read
  `satz packs` to see that it is not.

### 2 — keep the check and lower it to a note

- **Bad:** a fresh skeleton prints six notes at every compile until each is answered, and
  they are not findings: the estate is in exactly the state `init` wrote. A note nobody
  can clear is how the rest of the findings stop being read.

### 3 — exempt the map's defaults by name

- **Bad:** two kinds of default, a list to maintain, and no rule a reader could state.
