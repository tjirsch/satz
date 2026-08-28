# Proposal — resolving the overlap between the three preset commands

Status: **all five shipped (v0.45.0 / v0.46.0 / v0.46.1).** Written 2026-08-24, after
adopting CIS pack v2.1 across nine estates by hand and hitting every rough edge
below. Companion to [presets-workflow.md](presets-workflow.md), which describes
today's behaviour.

---

## 1. What is actually wrong

The three commands are not really overlapping — they are **one command's job
split across three, with the most-needed mode missing.**

| | classify | fetch | write | estate-aware | protects included packs |
|---|---|---|---|---|---|
| `get-presets` | ✗ | ✓ | ✓ (all, blind) | ✗ | ✗ |
| `check-presets` | ✓ | ✓ | ✗ | ✓ | n/a |
| `merge-presets` | ✓ | ✓ | ✓ (fork-first) | ✓ | ✓ |

Four concrete defects fall out of that shape:

**D1 — `get-presets` is a loaded gun.** It overwrites every pristine-named file
from upstream `main` without looking at what the estate uses. On any estate on
this fleet it would have silently retired a live org policy; the first sign would
have been a `tofu plan` showing a destroy nobody asked for. It is a bootstrap
command wearing no warning label.

**D2 — there is no `adopt` mode.** `merge-presets` deliberately forks an included
pack when upstream moved semantically, because without a baseline it cannot tell
"the customer edited this" from "this is simply old". Both happen constantly, and
the *second* is the common case: on 2026-08-24, **nine of nine** stale packs were
byte-identical to their upstream release. The correct action there is to overwrite
in place — which the tool cannot do, so it was done nine times with `cp`.

**D3 — `check-presets` cannot see a changed list default.** It compares the
compiled canonical twin, and the classifier reads only the text on a
`variables:` anchor line. A default written as a multi-line list has an empty
value on both sides and its items are never compared. estate 1 ran pack v2.0 against
upstream v2.1 — a five-entry list default gaining its fifth entry — and the
command printed *"13 preset(s) clean, no drift."* Silent false green in the one
command whose whole job is to not be silently green.

**D4 — no offline/cached upstream for `get-presets`, and the rate limit is
unreadable.** `check-presets` and `merge-presets` take `--pristine-dir`;
`get-presets` does not. The GitHub API allows 60 unauthenticated requests an hour,
a single command spends ~15, and exhaustion surfaces as
`reqwest::Error { kind: Decode, ... "invalid type: map, expected a sequence" }` —
the 403 body being parsed as a directory listing.

## 2. Proposal

### P1 — `check-presets` reports versions and a staleness verdict  ✅ SHIPPED v0.45.0

The single change with the best ratio of effort to value. Print what is actually
being asked:

```
check-presets: comparing presets/ against upstream

  PACK                             LOCAL   UPSTREAM  STATE
  CIS-GCP-Foundation-4.0.satz      1.5     2.1       STALE [included]   -> adopt
  essential-contacts-organization  1.1     1.1       clean [included]
  s1-group-permissions.local.satz  —       —         fork  [included]   (delta in .diff)
  organization-budget.satz         1.0     1.2       STALE              (not used — free to refresh)
```

`STALE` is a *new, mechanically decidable* verdict, and the one humans actually
need: **the local file is byte-identical to some published upstream release, and
a newer release exists.** It is decidable because upstream's git history is
available — the same lookup done by hand in the workflow doc. Where history is not
reachable, degrade to today's `MODIFIED` rather than guessing.

This distinction — *stale* vs *edited* — is the decision the operator currently
has to make with `git show` and `diff`, and it is the one that determines whether
adopting or merging is correct.

### P2 — `merge-presets --adopt <pack|all>`  ✅ SHIPPED v0.46.0

Make the deliberate upgrade a first-class mode instead of a `cp`:

```bash
satz merge-presets --adopt CIS-GCP-Foundation-4.0 --report-only
satz merge-presets --adopt CIS-GCP-Foundation-4.0
```

Semantics: for the named pristine pack, **overwrite in place and do not fork**,
even though the estate includes it. Then re-transpile and print the emission delta
— resources added/removed and attributes changed — because that, not the preset
diff, is what the operator is deciding about:

```
adopt CIS-GCP-Foundation-4.0: 1.5 -> 2.1
  emission delta:
    - google_org_policy_policy.iam_allowedPolicyMemberDomains   (REMOVED)
    ~ google_org_policy_policy.iam_managed_allowedPolicyMembers (parameters)
  compliance: 7 satisfied / 5 partial / 0 broken  (unchanged)
  next: review `git diff hcl/main.tf`, then `tofu plan`
```

`--adopt all` restricts itself to packs P1 classifies **STALE** and refuses
anything `EDITED`, so the safe default is preserved: a pack you actually edited
still forks unless you name it explicitly.

This is not a new capability — it is the procedure the fleet already follows,
mechanised, with the review step made unskippable.

### P3 — narrow `get-presets`, and give it `--force`  ✅ SHIPPED v0.46.0

`get-presets` should stop being able to change a live org by accident:

- **default**: fetch only files that are **missing locally**, plus refresh
  pristine files the estate does **not** `use`. Report the rest.
- **refuses** to overwrite a pristine pack the estate includes, and says why:
  ```
  refusing to overwrite CIS-GCP-Foundation-4.0.satz — the estate uses it and
  upstream moved 1.5 -> 2.1. Use `merge-presets` (forks, safe) or
  `merge-presets --adopt CIS-GCP-Foundation-4.0` (upgrades in place), or pass
  --force to overwrite anyway.
  ```
- **`--force`**: do what it does today — overwrite everything — but print the
  included packs it is about to change first.

That is where `--force` belongs. It should *not* go on `merge-presets`: there,
"force" would mean "skip the fork protection", which is exactly what `--adopt`
expresses better and more narrowly.

### P4 — fix the classification blind spot (D3)  ✅ SHIPPED v0.45.0

`split_preset` currently keeps only the text on an anchor line and discards
everything else inside the `variables:` block. It should attach the entry's
**full value**, including the indented block that follows it, so a list default is
compared item by item. Small and self-contained; the risk is that estates which
currently read "clean" would start reporting `MODIFIED (variables only)` — which
is the point, and the fleet is currently on pristine v2.1 everywhere, so this is
the cheapest moment to change it.

### ~~P5 — `--pristine-dir` on `get-presets`, and cache the fetch~~  ✅ SHIPPED v0.46.1

`--pristine-dir` landed in v0.46.0; the rest in v0.46.1.

An exhausted quota used to reach the user as
`reqwest::Error { kind: Decode … "invalid type: map, expected a sequence" }` —
GitHub answers a 403 with a JSON *object*, the caller was deserializing a
*sequence*, and so the deserializer's complaint arrived instead of the limit that
actually stopped the command. Every API response is now status-checked before it
is parsed, and renders as:

```
GitHub API rate limit reached (60 requests/hour, unauthenticated). Retry in ~48
minutes, set GITHUB_TOKEN, or compare against a local checkout with
`--pristine-dir <checkout>/presets`.
```

`x-ratelimit-remaining: 0` is what distinguishes this from the other 403s (a
private repo, a bad token), which keep a plain status message — telling that user
to wait an hour would send them nowhere. `GITHUB_TOKEN` is now actually sent on
API requests, so the advice is true rather than merely encouraging; it is never
attached to blob downloads, which go to a different host.

**The bigger win is that the sweep costs less.** The download walked the
`contents` API one request per directory — five per invocation, so twelve estates
exhausted the hourly quota on directory listings alone. One `git/trees?recursive=1`
request now covers the whole subtree (verified live: 14 files, byte-identical, one
API call), with the old walk kept as the fallback for a truncated tree response —
half a preset library looks exactly like an upstream that deleted packs, which
would read as drift on every estate. Blobs come from raw.githubusercontent.com,
which is not part of the quota at all.

All three commands now take their pristine copy from one helper that downloads at
most once per process.

Worth noting: `self-update` shares that same quota, so a preset sweep across a
fleet can lock users out of updating — which is why its 403 gets the same message.

## 3. What this does not change

- **Provenance by suffix stays exactly as it is.** `.local` is still the fork
  declaration, `.diff` is still the current adoption delta, versions still live
  in-file. Nothing here touches the layout doctrine.
- **`merge-presets` keeps forking by default.** P2 adds a named, deliberate
  escape; it does not weaken the standing guarantee that an included preset never
  changes silently.
- **No new command.** Three verbs stay three verbs; they just stop having a hole
  between them.

## 4. Order

1. ~~**P4** (blind-spot fix)~~ — **done, v0.45.0.** `split_preset` now attaches an
   entry's full value, including the indented block under it. Two tests guard it,
   both verified to fail when the regression is reintroduced.
2. ~~**P1** (version + STALE verdict)~~ — **done, v0.45.0.** The report separates
   the two axes: STALE when the version differs, EDITED when it does not. A
   pristine file whose `.local` sibling exists is exempted from the adopt advice,
   because the estate runs the fork and that copy is its baseline.
3. ~~**P3** (`get-presets` narrowing + `--force`)~~ — **done, v0.46.0.** Missing
   files install, unused files refresh, and a pack the estate USES is refused with
   the two commands that actually fit. `--force` overwrites and names each in-use
   pack as it goes.
4. ~~**P2** (`--adopt`)~~ — **done, v0.46.0.** `--adopt <stem>` overwrites in place
   and leaves the estate's `use` alone; `--adopt all` restricts itself to packs
   merely BEHIND and refuses one that differs at the same version, since that is
   an edit nobody named. Adoption and auto-forking cannot share a run: the
   repoint proves itself by transpile identity and an adoption legitimately
   changes the output, so a fork in an `--adopt` run is DEFERRED rather than
   silently unproven. The run prints the EMISSION delta, not the preset diff.
5. ~~**P5** (quota)~~ — **done, v0.46.1.** `--pristine-dir` landed on `get-presets`
   with P3 (without it, P3 was untestable while the quota was exhausted); the
   readable 403, `GITHUB_TOKEN`, the one-request tree download and the per-process
   fetch cache followed. Four tests guard the message, including the two 403s that
   are NOT the quota.
