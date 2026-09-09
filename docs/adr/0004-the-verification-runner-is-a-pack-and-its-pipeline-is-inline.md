# 0004 — the verification runner is a pack, and its pipeline is inline

- **Status:** accepted
- **Date:** 2026-09-09

## Context

Continuous verification — compile on every push, compare the live organisation
nightly — was planned as a "GitHub Actions template per estate". Two facts changed
the shape:

1. The fleet's estate repositories live in Cloud Source Repositories, not GitHub.
   The native runner there is Cloud Build, and a Cloud Build trigger runs as a
   service account directly, so the workload-identity plumbing a GitHub runner
   needs does not arise.
2. Every piece of runner infrastructure — trigger, scheduler, service account, IAM
   — is a `google_*` resource. The thing that sets up the pipeline can therefore be
   the thing that sets up everything else: a pack.

The intended commercial shape is MSP-hosted: the runner in the MSP's project,
watching each customer's repository, acting as that customer's IaC service account
through one IAM grant. That shape decides two design questions below.

## Decisions

**The runner is two packs, not one.** `ci/verification-runner.satz` (trigger,
scheduler, runner account, its project roles) and `ci/verification-runner-grant.satz`
(one binding on the estate's IaC service account). In the customer-hosted shape an
estate uses both and wires nothing; in the MSP-hosted shape the runner pack lives in
the MSP's estate and the grant pack in the customer's, with the runner's email as its
one param. The split follows ownership: the two resources belong to two parties.

**The build steps are inline in the trigger, not a `cloudbuild.yaml` in the watched
repository.** Whoever controls the build file controls what runs as the runner's
identity. In the MSP-hosted shape that file would sit in a repository the MSP does not
own, so a customer commit could redirect the runner. Inline, the pipeline is defined by
whoever applies the pack — the party whose service account it is.

**The runner installs satz at build time from the release, rather than running a
published container image.** Zero new release infrastructure; `ci_satz_release`
defaults to `latest`, which is what "continuous" means. An image is the natural next
step if build time or GitHub rate limits become the constraint.

**v1 reports through the exit code and the build log, and writes nothing back.**
Committing evidence into the watched repository needs write access to a repository
the runner may not own, and is a decision for whoever adopts the runner.

**The nightly trigger uses a concrete branch, the check trigger a regex.** Cloud
Scheduler starts the nightly build through `triggers:run`, which resolves the
template's branch; a push trigger matches a regex against the pushed branch. Same
resource type, two different meanings of `branch_name`.

## Consequences

- One `use` line per estate sets up continuous verification. No YAML is hand-written
  anywhere.
- A language tightening that breaks an estate fails a PR in that repo on the day it
  lands — the systematic form of what the 2026-09 fleet sweep did by hand.
- **A Cloud Source Repositories trigger can only watch a repository in its own
  project.** The MSP-hosted runner must therefore live in the repository project.
  That is a constraint of the platform, not a choice, and it is stated in the pack
  header.
- The nightly run has the same two read dependencies `report-compliance` has from a
  workstation — Cloud Asset Inventory enabled, an asset-viewer role at organization
  level — and fails for the same reasons. `unverified` is not in the default
  `--fail-on` set for exactly that reason.
- GitHub Actions is not built. The pack's GCP half (a workload-identity pool and
  provider) would be a third small pack; the workflow file would live in GitHub and
  cannot be a resource. Deferred until a customer's repositories are on GitHub.

## Pros and cons of the options

### A per-estate `cloudbuild.yaml` in each repository

- **Good:** the pipeline is visible where the estate is; anyone can read it.
- **Good:** Cloud Build's own conventional shape.
- **Bad:** in the MSP-hosted shape it hands control of the runner's identity to whoever
  can commit to the customer repository.
- **Bad:** N copies of one file, drifting.

### Inline build steps in the trigger, from a pack *(chosen)*

- **Good:** control of the pipeline follows ownership of the service account.
- **Good:** one definition, applied like any other resource, versioned with the pack.
- **Bad:** the steps are shell inside HCL inside Satz — three levels of quoting, and a
  change to the pipeline is a pack version bump rather than a file edit.
- **Bad:** a reader looking for "the CI config" in the estate repo finds nothing; the
  header has to say where it is.

### A published container image

- **Good:** faster builds, no dependence on GitHub at build time.
- **Bad:** new release infrastructure to keep current, and a second artefact whose
  version can drift from the binary's. Deferred, not rejected.
