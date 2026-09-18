# 0029 — satz is released for four targets: no Intel macOS, no ARM64 Windows

- **Status:** accepted; supersedes the ARM64 half of ADR 0027
- **Date:** 2026-09-18
- **Shipped in:** the release that follows

## Context

A release built six targets: macOS, Linux and Windows, each on x86_64 and on ARM64.
Upgrading cargo-dist from 0.30.3 to 0.33.0 ran `dist init`, which asks for the target
list again, and the question became whether all six are worth carrying.

Two of them had no user asking for them and no test beyond CI compiling them:

- **Intel macOS** (`x86_64-apple-darwin`). Apple stopped selling Intel Macs in 2023.
  satz-studio, the desktop app that drives satz, already ships for Apple silicon alone
  (its own ADR 0016), so an Intel Mac could run satz but not the app built around it.
- **ARM64 Windows** (`aarch64-pc-windows-msvc`), added with ADR 0027. It needed its own
  runner: cargo-dist's default for the target, cargo-xwin in a Linux container, does not
  compile satz (an old rustc in the container, then clang-cl rejecting the GNU-syntax
  ARM64 assembly of `psm` under typst's `stacker`), so `dist-workspace.toml` carried a
  `github-custom-runners` entry for GitHub's `windows-11-arm`.

What happens on a machine with no build matters for the choice. cargo-dist's installers
pick the archive for the machine's own architecture and stop when there is none: the
PowerShell installer throws "could not find binaries for this platform" on ARM64
Windows rather than falling back to the x64 zip, which Windows 11 could run through its
own emulation. And `prune-releases.yml` keeps only the five newest releases, so an old
binary for a dropped target does not stay downloadable either.

## Options

- **(a) Keep all six.** Every machine that ran satz keeps getting it, and `self-update`
  keeps working everywhere. Costs two build jobs per release, one of them on a custom
  runner held in config, and two platforms answered for with nothing but a compile.
- **(b) Drop Intel macOS only.** Matches satz-studio. Keeps the ARM64 Windows build,
  which is what a Windows 11 virtual machine on an Apple-silicon Mac runs natively.
- **(c) Four targets: Apple-silicon macOS, Linux on x86_64 and ARM64, Windows on
  x86_64.** One build job per platform a user is known to have, no custom runner.

## Decision

(c), decided 2026-09-18.

## Consequences

- An Intel Mac or an ARM64 Windows machine gets no binary: the one-line installers stop
  with an error, and `self-update` on an Intel Mac finds nothing to install. satz builds
  from source there: `cargo install --git https://github.com/tjirsch/satz --locked`.
  The README says so beside each installer.
- The `windows-11-arm` custom runner is gone from `dist-workspace.toml`; the ARM64 half of
  ADR 0027's consequences no longer applies. The rest of ADR 0027 — what Windows support
  means, on x86_64 — stands.
- A release publishes four archives instead of six, and moves four builds' worth of
  transport artifacts instead of six.
- Released as a MINOR, which stretches ADR 0010: that rule is written about estates, and
  no estate changes here. The minor number is still the one signal an operator reads for
  "this upgrade brings work", and for anyone on the two dropped platforms it brings the
  most work there is: there is nothing to upgrade to.
- Adding a target back is one line in `targets` plus `dist generate` — and, for ARM64
  Windows, the custom runner, for the reasons in ADR 0027.
