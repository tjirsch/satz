# 0030 — an estate that names no identity is refused, and `satz mcp` derives it per call

- **Status:** accepted; supersedes ADR-0001's derivation at `satz_open`, keeps its scope per call
- **Date:** 2026-09-18
- **Shipped in:** the release that follows

## Context

After init, a live command runs as the estate's IaC service account, derived from its
params — `deployment_mode` and `svc_iac_account` + `infra_project_name` — exactly as the
emitted provider block derives it. The derivation had two answers: a service account in
cloud mode, and "nothing to impersonate" in local mode, where the calls are the
credentials themselves.

Every failure also came out as the second answer. The derivation read the params with
`.ok()?`, so an estate whose params did not parse impersonated nothing, and a
`deployment_mode` of `"boot"` — which the compile refuses, having no backend for it —
was reported as a mode of its own and impersonated nothing either. A command that had
to run as the service account then ran as whoever was logged in, and said nothing.

`satz mcp` derived the identity once, when `satz_open` opened an estate, and every live
tool ran with that answer — also when the tool named a different estate, and also after
the open estate had been migrated to another mode.

## Considered options

1. **Refuse where the identity cannot be derived; derive it per call under MCP**, from
   the estate the call names or else the open one, as it stands at the call.
2. **Refuse, but keep deriving once at `satz_open`.** A failure refuses the open.
3. **Keep the fallback** and print a warning when the derivation fails.

## Decision

Option 1. The derivation returns an error naming the estate and the reason, and reads
the mode through the emitter's own reader, so the compile, the CLI binding, `whoami`,
`migrate` and the MCP scope accept the same modes and refuse the same ones with the same
words. Every caller refuses on the error: the CLI binding and `whoami <estate>` before
anything is bound, `migrate` before the estate is touched, `satz_open` before anything
is opened, and each live MCP tool before its first call.

## Consequences

- Nothing runs as the login in place of an estate whose params cannot be read.
- A live MCP tool that names another estate runs as that estate, which is what
  `docs/mcp.md` says a live tool does.
- An estate edited after `satz_open` — migrated to cloud mode, say — runs as it stands at
  the next call, as the same command from the shell would.
- Each live MCP call parses the estate's params once more. They are read without the
  provider schema, which makes it a small cost next to the API calls that follow.
- `satz_open`'s `deployment_mode` is the mode the compile reads, `local` when the estate
  declares none, where it was the raw param and null when absent.
- `deployment_mode = "cloud"` without a value for `svc_iac_account` or
  `infra_project_name` is refused the same way, by the same reader: cloud mode runs as the
  account those two name, so without them it names no identity. The compile reports it at
  the `deployment_mode` line, `migrate --mode cloud` refuses before touching the file, and
  `satz_estates` lists every refused estate with its reason instead of a mode.

## Pros and cons of the options

### 1 · Refuse, and derive per call *(chosen)*

- **Good:** one reader, one answer: no surface can accept a mode the compile refuses.
- **Good:** the identity a call runs as is always the one its estate declares now.
- **Bad:** an estate that does not parse cannot be opened under MCP at all, so an agent
  cannot run the offline tools on it either; it gets the reason, and a human fixes the
  file.

### 2 · Refuse, derive once at open

- **Good:** one read per session.
- **Bad:** a tool that names another estate runs as the open one — one estate's lookups
  with another estate's identity, the failure ADR 0001 exists to prevent.
- **Bad:** after a migration the session runs as the old identity until it is opened
  again, and `satz_whoami` had to detect that case and ask for a re-open.

### 3 · Fall back with a warning

- **Good:** nothing that ran before is refused.
- **Bad:** it is the defect with a line on stderr: the calls still run as the login, and
  under MCP stderr is not what the agent reads.
