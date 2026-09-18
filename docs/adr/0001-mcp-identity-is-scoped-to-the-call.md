# 0001 — the MCP server scopes the identity to each call

- **Status:** accepted; deriving the identity once at `satz_open` is superseded by
  ADR-0030, which derives it per call from the estate the call works on
- **Date:** 2026-09-05
- **Shipped in:** v0.46.90

## Context

After init, everything satz does against a customer's estate runs as that estate's
IaC service account, derived from the estate's own params exactly as the emitted
provider block derives it. The Application Default Credentials authenticate; satz's
first act is to exchange them for that account.

`satz mcp` is long-lived, and working through estates in turn — one after another,
not in parallel — is ordinary operator work. The identity was a process-wide
`OnceLock`: the first live call bound it, and a second estate needing a different
service account was refused. That refusal was correct for what the code could then
guarantee, but it meant re-registering the server and restarting the client for every
estate.

The constraint that makes this hard: **the server dispatches requests concurrently.**
That is not theoretical — the smoke matrix demonstrated two tool calls racing, and
clients batch calls in a single turn.

## Considered options

1. **Keep one identity per process, restart between estates.**
2. **A mutable global, reset by a "switch estate" tool.**
3. **Scope the identity to each call.**

## Decision

Option 3. `satz_open` names an estate; the identity is derived from it and scoped to
each call with a `tokio::task_local`, which binds the value to the *future* rather
than the thread, so it holds across every await inside a call and cannot leak into a
call running beside it. Nothing in this tree spawns a task, so nothing escapes the
scope.

## Consequences

- One server serves a fleet; `satz_open` moves to the next estate with no restart.
- The fifteen `access_token` call sites are untouched — the scope is read at the
  chokepoint they already share, so this was ~100 lines rather than the ~450 that
  threading the identity as an explicit parameter would have cost.
- `--no-impersonate` is checked before the scope, so an operator's explicit request
  for the plain ADC still outranks everything.
- The CLI keeps the process-wide binding, where one command means one estate and the
  guarantee is free.

## Pros and cons of the options

### 1 · One identity per process

- **Good:** trivially safe; the guarantee is a property of the process.
- **Good:** no new concepts.
- **Bad:** re-registering a server and restarting the client per estate, for work
  that is routine.
- **Bad:** it made a normal workflow feel like a limitation of the tool, which is how
  operators end up running things outside it.

### 2 · Mutable global with a switch tool

- **Good:** small change; matches the mental model of "point it at an estate".
- **Bad:** **unsafe under concurrent dispatch.** A call in flight on estate A would
  finish under estate B's identity if the switch landed between its start and its
  API calls — one customer's data read with another's credentials. This is the exact
  defect a previous version had, where a silently dropped binding meant estate B's
  tools ran as estate A's service account every time.
- **Bad:** the danger is invisible in output, in tests and in a diff. It shows up in
  an audit log, months later.

### 3 · Scope per call *(chosen)*

- **Good:** correct under concurrency by construction — work started under one estate
  finishes under it, whatever else the server is doing.
- **Good:** no switch tool is needed at all, because every data tool already names an
  estate.
- **Good:** the same rule as the CLI's, stated per call instead of per process.
- **Bad:** a task-local is a less obvious mechanism than a parameter, so the comment
  explaining why it is not a global has to carry its weight.
- **Bad:** it would silently stop protecting anything if this tree ever spawned a
  task; that assumption is stated where the scope is defined.
