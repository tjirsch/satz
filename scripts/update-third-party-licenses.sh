#!/usr/bin/env bash
# update-third-party-licenses.sh — rewrite THIRD-PARTY-LICENSES.md, the licence text
# of every crate compiled into satz, from Cargo.lock.
#
#   scripts/update-third-party-licenses.sh           # rewrite the file
#   scripts/update-third-party-licenses.sh --check   # fail when it is stale, print the diff
#
# The file travels in every release archive (`include` in dist-workspace.toml), which
# is what MIT and Apache 2.0 ask of a binary redistribution. `about.toml` holds the
# allow-list of licences and `about.hbs` the layout; a dependency under a licence the
# allow-list does not name fails here, naming the crate.
#
# The `checks` job of .github/workflows/smoke.yml runs `--check` on every pull request
# and every push to main, after clippy and the tests have fetched the crate sources
# this reads.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# cargo-about is a tool, not a dependency — it never enters the binary's crate graph.
# The version is exact on purpose: a different one can read a crate's licence file
# differently, and the generated file would then differ between a machine and CI
# without anything in the repository having changed.
WANT=0.9.2

out=THIRD-PARTY-LICENSES.md
have=$(cargo about --version 2>/dev/null | awk '{print $2}' || true)
if [[ "$have" != "$WANT" ]]; then
  echo "update-third-party-licenses: cargo-about $WANT required${have:+, found $have}" >&2
  echo "  cargo binstall cargo-about@$WANT   # or: cargo install cargo-about --version $WANT --locked" >&2
  exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
# --frozen: --locked (the lockfile is the input and must not move underneath the run)
#           plus --offline, so the crate sources on disk are the only input. Online,
#           cargo-about may pull a licence text out of a crate's git repository
#           instead, and the file would depend on what the network answered. A source
#           that is not unpacked yet: `cargo fetch --locked`.
# --fail:   a crate whose licence cannot be read is an error, never an empty entry
cargo about generate --frozen --fail -c about.toml about.hbs -o "$tmp"

if [[ "${1:-}" == "--check" ]]; then
  if ! diff -u "$out" "$tmp"; then
    echo >&2
    echo "update-third-party-licenses: $out is stale — run scripts/update-third-party-licenses.sh" >&2
    exit 1
  fi
  echo "update-third-party-licenses: OK ($out matches Cargo.lock)"
elif [[ -n "${1:-}" ]]; then
  echo "update-third-party-licenses: unknown argument '$1' (expected --check or nothing)" >&2
  exit 1
else
  mv "$tmp" "$out"
  chmod 0644 "$out"  # mktemp writes 0600
  echo "update-third-party-licenses: wrote $out"
fi
