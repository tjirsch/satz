#!/usr/bin/env bash
# Hold a published release to the files cargo-dist planned for it, and fail naming each
# one it does not carry.
#
#   scripts/check-release-assets.sh <tag> <plan.json> [<owner/repo>]
#
# <plan.json> is cargo-dist's manifest for the release: the `plan` input every
# post-announce job receives in the Release workflow (`.github/workflows/release.yml`), or
# `dist plan --output-format=json` locally. The expected files are every artifact its
# `releases[].artifacts` names — the archives per target, their `.sha256`, the installers,
# `sha256.sum`, the source tarball and its `.sha256` — so they follow the targets and
# installers of `dist-workspace.toml` with nothing written twice. One file is added:
# `dist-manifest.json`, which the `host` job of the Release workflow writes and attaches
# beside the artifacts but no manifest lists as one.
#
# It only reads: in the Release workflow, the job that runs it first attaches the
# `dist-manifest.json` the run built when the host job did not
# (.github/workflows/check-release-assets.yml).
#
# The installers' `.sha256` sidecars are attached by `attach-checksum.yml`, a post-announce
# job that runs beside this one; that job fails by itself when it cannot attach them.
#
# Needs gh and jq. The repository is <owner/repo>, else $GITHUB_REPOSITORY.
# CI runs it after every release: .github/workflows/check-release-assets.yml.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <tag> <plan.json> [<owner/repo>]" >&2; exit 2; }
tag=$1
plan=$2
repo=${3:-${GITHUB_REPOSITORY:-}}
[ -n "$repo" ] || { echo "check-release-assets: no repository — pass <owner/repo> or set GITHUB_REPOSITORY" >&2; exit 2; }
[ -s "$plan" ] || { echo "check-release-assets: the plan $plan is missing or empty" >&2; exit 2; }

planned=$(jq -r '.releases[].artifacts[]' "$plan")
[ -n "$planned" ] || { echo "check-release-assets: the plan $plan names no artifact — it is no cargo-dist manifest" >&2; exit 2; }
expected=$(printf '%s\ndist-manifest.json\n' "$planned" | sort -u)
attached=$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name' | sort -u)

missing=$(comm -23 <(printf '%s\n' "$expected") <(printf '%s\n' "$attached"))
count=$(printf '%s\n' "$expected" | wc -l | tr -d ' ')
if [ -n "$missing" ]; then
  echo "check-release-assets: the release $tag of $repo lacks $(printf '%s\n' "$missing" | wc -l | tr -d ' ') of the $count files planned for it:" >&2
  printf '  %s\n' $missing >&2
  exit 1
fi
echo "check-release-assets: the release $tag of $repo carries all $count files planned for it"
