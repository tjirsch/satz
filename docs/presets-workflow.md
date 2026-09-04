# Working with presets: `get-presets`, `check-presets`, `merge-presets`

How to tell whether a newer preset exists, what to do about it, and which command
to reach for. Describes **what the tool does today** (v0.44.x).

---

## 1. The mental model

There is **one** `presets/` folder per estate, and the **filename suffix declares
who owns the file**:

| file | owner | what may happen to it |
|---|---|---|
| `X.satz` | upstream | overwritable — this is a pristine copy |
| `X.local.satz` | you | never touched by any command |
| `X.diff.satz` | the tool | the current fork-vs-pristine delta, rewritten each `merge-presets` run |
| `<own>.satz` | you | no upstream counterpart, kept as-is |

Two more facts that decide everything below:

- **Pack versions live inside the file** — `pack CIS_GCP_Foundation_4_0 version "2.1"`.
  Filenames carry only the *framework* version (`CIS-GCP-Foundation-**4.0**`).
- **Upstream is the `presets/` tree on the repo's `main` branch**, fetched over the
  GitHub API. Not a release tag — a push to `main` publishes a preset immediately.

## 2. The three commands, honestly

| command | reads | writes | estate-aware | protects you |
|---|---|---|---|---|
| `get-presets` | upstream | missing + unused files | yes | yes (refuses in-use; `--force` overrides) |
| `check-presets <estate>` | both | nothing | yes | n/a (read-only) |
| `merge-presets` | both | pristine names + forks + diffs | yes | yes |

**`get-presets`** populates the library: it installs what is missing and refreshes
what the estate does **not** use. A pristine pack the estate **does** use is
**refused**, naming the two commands that fit instead — because changing it changes
what the org enforces, and a `tofu plan` would be the first place you noticed.
`--force` overrides, listing each in-use pack as it overwrites it. (Before v0.46.0
it overwrote everything unconditionally with no estate awareness.)

**`check-presets <estate>`** is the read-only report. It walks the estate's `use`
graph, so packs the estate actually includes are tagged `[included]`, and drift in
an included pack exits non-zero — that is the CI gate.

**`merge-presets`** is the safe write path. Its contract is: **a preset your
estate includes never changes silently.** When upstream has moved *semantically*,
it preserves your current content as `X.local.satz`, repoints the estate's `use`
at that fork, proves the repoint by transpile identity, refreshes the pristine
`X.satz`, and writes `X.diff.satz` — the exact delta adopting upstream would make.
Comment and formatting churn upgrades silently instead of forking.

## 3. "A newer preset is available" — what do I do?

### Step 1 — find out, without touching anything

```bash
satz --config <estate-dir> check-presets yaml/<ESTATE>.satz
```

Rate-limited? The GitHub API allows 60 unauthenticated requests an hour and this
command spends about fifteen. A 403 surfaces as a confusing decode error
(`invalid type: map, expected a sequence`) — that is the error body being parsed
as the file listing. Compare against a local checkout instead:

```bash
satz --config <estate-dir> check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

### Step 2 — read the verdict

`check-presets` reports two independent axes, and keeping them apart is the whole
point: the **version line** says whether a newer release exists; the **content
comparison** says whether anyone edited this copy.

- **clean** — identical, or only comments/formatting differ, and the version
  matches upstream.
- **STALE** — the version differs. A newer release exists. Printed with the pair,
  `local v1.5, upstream v2.1`, and with what moved. If the change is comment-only
  it says so and does not fail the gate.
- **EDITED (variables only)** — same version, only scalar defaults differ. The
  report prints the exact lines to lift into your estate's params.
- **EDITED (structural)** — same version, resource bodies or the variable set
  differ. A local edit — or an upstream release that changed without a version
  bump. Review by hand.
- **fork** — an `X.local.*` file. Never an error. If a pristine file reads STALE
  but its `.local` sibling exists, the report says so and tells you to leave the
  pristine copy alone: the estate runs the fork, and that copy is the fork's
  baseline.
- **missing locally** / **local-only** — new upstream preset / your own file.

Drift in an **`[included]`** preset exits non-zero — that is the CI gate, and
since v0.45.0 a stale included pack trips it too.

> **Historical note.** Before v0.45.0 the classifier read only the text ON a
> `variables:` anchor line, so a default written as a **multi-line list** was
> never compared and a pack a whole version behind could report *clean*. Real
> case: estate 1 ran CIS pack v2.0 against upstream v2.1, whose only substantive change
> was a fifth entry in `allowed_policy_member_subjects` — the command printed
> *"13 preset(s) clean, no drift."* Both the comparison and the version reporting
> are fixed; if you are on an older binary, read the in-file `pack … version` line
> yourself.

### Step 3 — decide: is your copy STALE, or EDITED?

This is the decision the tool cannot make for you, and it changes what you run.

```bash
# what release does the local file claim to be?
grep -m1 '^pack' <estate>/presets/CIS-GCP-Foundation-4.0.satz     # -> version "1.5"

# is it byte-identical to that release?
cd ~/projects/satz
git log --format=%H -- presets/CIS-GCP-Foundation-4.0.satz \
  | while read c; do
      v=$(git show $c:presets/CIS-GCP-Foundation-4.0.satz | grep -m1 '^pack')
      echo "$c $v"
    done | head           # find the commit that carried v1.5
git show <that-commit>:presets/CIS-GCP-Foundation-4.0.satz > /tmp/pristine-1.5.satz
diff /tmp/pristine-1.5.satz <estate>/presets/CIS-GCP-Foundation-4.0.satz
```

| result | meaning | what to run |
|---|---|---|
| no diff | **STALE** — nobody edited it, it is simply old | **adopt**: copy the pristine file in (§4) |
| diff | **EDITED** — a real local change | **`merge-presets`** — let it fork and give you `X.diff.satz` |

Getting this wrong in the safe direction is what `merge-presets` does by default:
without a baseline it cannot distinguish the two, so it **forks**. That is right
when you edited the pack, and wrong when the file is merely old — it takes an
estate off the pristine track for nothing.

### Step 4a — adopt (your copy is stale)

```bash
satz --config <estate-dir> merge-presets --adopt CIS-GCP-Foundation-4.0 --report-only
satz --config <estate-dir> merge-presets --adopt CIS-GCP-Foundation-4.0
```

It overwrites the pristine name in place, leaves the estate's `use` alone, and
prints the **emission** delta — which resources appear or disappear, by address.
`--adopt all` does every pack that is merely BEHIND, and refuses one that differs
at the *same* version: that is an edit, and it has to be named. A fork+repoint
needed in the same run is **deferred**, not done silently — the repoint proves
itself by transpile identity, and an adoption legitimately changes the output, so
the two cannot share a run.

`merge-presets` does **not** regenerate `hcl/`. Continue with the normal gates:

```bash
satz --config <estate-dir> transpile yaml/<ESTATE>.satz

cd <estate-dir>
git status --short          # only presets/ + hcl/ should move
git diff hcl/main.tf        # THIS is the real review — the emission delta
satz --config . require cis-gcp-4.0 yaml/<ESTATE>.satz   # verdicts should not surprise you
satz --config . check-presets --pristine-dir ~/projects/satz/presets yaml/<ESTATE>.satz
```

Then read the plan **before** applying:

```bash
cd hcl && tofu plan
```

Adoption is only a no-op when the moved default is one your estate overrides, or
the pack is not `use`d at all. Otherwise expect a real plan and gate it with a
runbook. Nine estates went through exactly this on 2026-08-24; seven produced
`1 to change, 1 to destroy`.

### Step 4b — merge (your copy is edited)

```bash
cd <estate-dir>
git status --short                      # must be clean: auto-repoints refuse a dirty estate
satz --config . merge-presets --report-only   # preview every planned action
satz --config . merge-presets
```

Afterwards you have `X.local.satz` (your content, now the thing the estate uses),
a refreshed pristine `X.satz`, and `X.diff.satz` telling you exactly what adopting
upstream would change. Read the diff; adopt when you are ready by pointing the
estate's `use` back at the pristine name and deleting the fork.

Exit code is non-zero when anything needs attention — a fork was created, a fork's
upstream moved, or a repoint was refused. That is the CI signal.

### Step 4c — the estate runs a fork already

If the estate `use`s `X.local.satz`, **copying pristine over `X.satz` changes
nothing it emits.** The change has to be made in the fork. Do not "refresh" the
pristine sibling either — it is the fork's historical baseline for the eventual
merge, and overwriting it destroys the only record of where the fork branched.

## 4. Rules of thumb

- **Never run `get-presets` on an estate whose packs are in use.** Use it to
  populate a new estate, or to fetch packs that are missing entirely.
- **`check-presets` in CI, `merge-presets` by hand.** The first is a gate, the
  second edits your estate and repoints `use` lines.
- **Read `git diff hcl/main.tf`, not the preset diff.** The preset diff tells you
  what changed upstream; the emission diff tells you what happens to the org.
- **An unused pack is free to refresh** — zero emission delta, and it keeps the
  file from later reading as a customer fork.
- **`check-presets` answers "am I behind?" directly** since v0.45.0 — it prints
  the local and upstream version and a STALE verdict. On an older binary, read
  the in-file `pack … version` line instead.

## 5. When upstream stops answering: the GitHub quota

All three commands read the preset library from GitHub, and GitHub's
unauthenticated REST quota is **60 requests per hour, per IP** — shared with
`satz self-update`. A sweep across a fleet can exhaust it, and then be the reason
your own `self-update` stops working.

Exhaustion says so plainly:

```
GitHub API rate limit reached (60 requests/hour, unauthenticated). Retry in ~48
minutes, set GITHUB_TOKEN, or compare against a local checkout with
`--pristine-dir <checkout>/presets`.
```

Three ways out, cheapest first:

- **`--pristine-dir <checkout>/presets`** — all three commands take it, and it
  makes no network request at all. If you have the tool's repository checked out,
  this is the fastest answer and the one to reach for during a sweep.
- **`export GITHUB_TOKEN=…`** — any token, even one with no scopes, raises the
  quota to 5,000/hour. It is sent only to the API, never to the download host.
- **Wait.** The message says for how long, read from the reset the API reports.

Since v0.46.1 one invocation costs **one** API request (the whole preset subtree
arrives in a single tree response; the files themselves come from a host that is
not rate-limited), so this is far harder to hit than it was.

A 403 that is *not* the quota — a private repo, a bad token — reports as a plain
status instead, because waiting an hour would not fix it.
