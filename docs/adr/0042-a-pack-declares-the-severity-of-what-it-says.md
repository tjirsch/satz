# 0042 — a pack declares the severity of what it says, and a command that writes to the organisation refuses on an error

- **Status:** accepted; supersedes the `before = apply` half of ADR 0034
- **Date:** 2026-09-20
- **Shipped in:** the release that follows

## Context

A `notice` said two things at once. It was a rule — a pack names the command an estate has
to run once the pack is on — and it was a behaviour: `transpile --apply` and `bootstrap`
refused while such a notice was open. The behaviour was written into the two commands, at
two call sites in `src/main.rs`, each asking `notices::require_acknowledged` by name. A
third command that writes to a customer's organisation would have had to remember to ask,
and forgetting is invisible: the run succeeds, and what it did is visible only in the
organisation.

The word `before` also carried a phase — `apply`, and no other value was accepted — so the
statement offered one knob that looked like it could say WHEN a message is checked while
in fact it said only whether the message blocks.

Beside that, the compile's own severity ladder said `note` for the lowest rung, a word
that names a KIND of message (a `notice`) rather than how much it holds back. The editor
already mapped it to `INFORMATION`.

## Decision

**A pack declares a `severity` on what it says**, and `severity` replaces `before`:

```satz
notice cis_baseline_adopted {
  text     = "…"
  run      = "satz adopt <estate> --execute --import"
  severity = error        // error | warning | info; warning when it is not declared
}
```

- **`error`** — every command that writes to the organisation refuses while the message is
  open.
- **`warning`** — those commands print it and go on. A pack that declares no severity says
  this.
- **`info`** — the compile says it; nothing waits for it.

**Which commands write to the organisation is one decision, in one place.**
`src/org_write.rs` classifies EVERY command as `Writes(estate)`, `NoEstate(reason)` or
`Reads(reason)`, in a match with no wildcard arm: a command added to the CLI does not
compile until it says which it is, and a `Reads` carries the reason it is one. `main`
calls the gate once, in front of the dispatch. No command's own code names a notice, a
severity or a param.

**The pack sets the floor; the run sets the ceiling.** The same producer makes the
finding for both: a run that only READS says an open `error` as a warning, a run that
WRITES says it as the error it is. The compile of an estate with an open message therefore
succeeds — the command that closes the message compiles that estate too, and a compile
that refused would leave no way to close it — while the apply refuses with the same
finding at the same line. The consequences of an error follow from the machinery that
already existed: no silence tier may hide it (ADR 0037), and a `--silence` naming it is
refused with the reason.

**`Severity::Note` is renamed `Info`** and the word moves everywhere it is published: the
MCP JSON Schema, `docs/llms.md`'s finding table, the last line of a run (`1 info`), the
smoke greps and the doc transcripts. No dual-accept: `"note"` is gone.

**`satz review-pack` judges an error-severity message.** A pack author choosing
`severity = error` blocks the apply of every estate that adopts the pack, so the review
prints that notice as a WARNING rather than listing it, and refuses a `severity = error`
whose `text` is shorter than 80 characters — a reason is a sentence, and the operator the
message stops is otherwise left to guess why.

## Consequences

- A pack carrying `before = apply` is refused with the edit to make, and every CIS
  org-policy pack in the library declares `severity = error` — today's behaviour, now
  said by the pack. This is why the release is a MINOR one, with both entries under
  `## Breaking changes` in `presets/README.md`.
- Two commands that wrote to an organisation and asked nothing now ask: `run-actions
  --execute`, which runs the estate's own deployment steps, and `migrate`, which grants
  the IaC service account Groups Admin. That is the point of a rule over a call site.
- `satz apply` and `satz plan` are handed `hcl_dir` and no estate, so no pack's message is
  reachable from them; the table says so in its entry rather than leaving it unsaid.
  `transpile --apply` is the route that compiles first, and it is gated.
- What `before` could have grown into — "warn at apply time but never block" — is not
  expressible. One knob an author cannot get wrong is worth more than two that interact;
  if a pack ever needs that combination, this is the decision to revisit.
- Whether a CI switch may DOWNGRADE a named error kind is still open, and nothing here
  answers it: the run tier refuses to silence an error exactly as before.

## Pros and cons of the options

### A severity on the message, with the gate over every writing command *(chosen)*

- **Good:** one rule instead of one remembered call per command, and the compiler makes a
  new command state its classification.
- **Good:** the severity is data a pack declares, so `satz packs`, the MCP rows and
  `doc-packs` carry it without any of them knowing what blocks.
- **Bad:** a pack author can block every adopting estate's apply with one word. That is
  what `review-pack`'s warning and the reason bar are for.

### Keep `before = apply` and add the gate to each new command

- **Bad:** the omission is silent, and it is found in an audit log rather than in a test.

### Make the compile itself refuse on an open error-severity message

- **Bad:** `satz adopt` — the command a notice names — compiles the estate, so the estate
  could never be adopted and the message could never be closed. A deadlock by design.

### A second key beside `severity` for the phase (`before`)

- **Bad:** two knobs that interact, and no pack in the library needs the combination.
