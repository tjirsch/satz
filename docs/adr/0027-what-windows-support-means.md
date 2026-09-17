# 0027 — what Windows support means: a tested build that works or refuses clearly

- **Status:** accepted
- **Date:** 2026-09-17
- **Shipped in:** the release that follows

## Context

satz built and released for macOS and Linux only. satz-studio already runs on Windows and
could not install satz there — its Windows CI compiled satz from its submodule. Every
dependency builds for `x86_64-pc-windows-msvc`, but the code assumed Unix in ways that
failed silently on Windows: the settings file was never read (no `HOME`), so the update
check ran on every command; `--out /dev/stdout` wrote a file named `\dev\stdout.json`;
`review-pack` wrote a `\\?\C:\…` path into a `use` line it could not parse; a CRLF checkout
failed `fmt --check` and `get-presets`; `.sh` actions died with an OS error.

## Options

- **(a) Release binaries and the PowerShell installer only.** One change; ships every
  defect above, and satz's own CI never runs on Windows.
- **(b) (a), plus clippy and the tests on Windows in CI, plus the fixes** — so a Windows
  binary is tested, and where satz cannot do something there it says so. The smoke matrix
  (bash, a fake tofu, python clients) stays on Linux.
- **(c) Full parity** — a Windows smoke matrix, actions that run scripts, an in-place
  `self-update`, a PowerShell `generate-migration`. Roughly doubles the smoke upkeep of
  every new command.

## Decision

(b), with Thomas's decisions on the rest (2026-09-17, the recommended plan):

- x86_64 and ARM64 builds, `installers = ["shell", "powershell"]`, and a `.ps1.sha256`
  sidecar. No MSI (elevation, WiX, nothing self-update or satz-studio could use), no code
  signing yet (SmartScreen is documented).
- A `windows-checks` CI job: clippy `-D warnings` and `cargo test` on windows-latest.
- `.gitattributes` pins LF; a CRLF line ending reads as LF wherever Satz is read.
- `self-update` refuses on Windows before downloading, naming the PowerShell one-liner.
- Settings under `%USERPROFILE%\.config\satz`, where satz-studio writes them.
- `--out -` is stdout on every platform; `/dev/…` is refused on Windows.
- `/` in Satz text and in compared pack paths; the `\\?\` prefix is dropped.
- An action whose `run` is a script is refused before the spawn, naming the bash line.

## Consequences

- Every push runs a Windows job; a change that only fails there is caught on its PR.
- The PowerShell installer verifies nothing it downloads — cargo-dist's template has no
  hash check — so the README shows the manual `Get-FileHash` check against `sha256.sum`.
- What stays unsupported on Windows is refused by name, not approximated: `self-update`,
  script actions, `/dev/…` outputs; `generate-migration` writes bash.
