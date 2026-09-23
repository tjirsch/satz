# 0061 — satz writes no deletion default, and a teardown is an estate edit

- **Status:** accepted
- **Date:** 2026-09-23
- **Shipped in:** the release that follows

## Context

The Google provider protects the two nodes an estate builds its tree from:
`google_project.deletion_policy` defaults to `PREVENT` and
`google_folder.deletion_protection` defaults to `true`. A `tofu destroy` of an
estate that declares neither is refused — once for the project, and again for the
folder after the project is dealt with, because each refusal stops the destroy
before the next one is reached.

satz emits both as ordinary declared attributes: an estate that writes
`deletion_policy = "DELETE"` or `deletion_protection = false` gets it in the HCL,
and an estate that writes neither gets neither, so the provider's own default
stands.

Measured on a live organisation with v0.77.0, tearing a test estate down took:
destroy (the project is refused) → add `deletion_policy = "DELETE"` → apply that
one attribute → destroy (the folder is refused) → add
`deletion_protection = false` → apply that one attribute → destroy. Seven steps,
none of them written down anywhere.

## Decision

**satz carries no deletion default of its own. The provider's protection stands
until the estate says otherwise, and the teardown sequence is documented where an
operator tearing an estate down looks — `docs/workflows.md`, with the commands in
order.**

A test holds both halves: an estate that declares neither attribute emits
neither, and an estate that declares them emits them as written.

## Options

**Emit `deletion_policy = "DELETE"` and `deletion_protection = false` by
default.** *Rejected.* It would make `tofu destroy` work on the first try and
make every satz estate destroyable by a command nobody meant to run. What is
behind those two attributes is a customer's production project with everything in
it and a folder with everything under it, and a project delete is not undone
after thirty days. satz exists to manage those organisations; weakening the one
provider default that stands between a mistyped `-target` and an irreversible
delete buys seven documented steps in a sandbox and sells the protection of every
production estate. The teardown is also rare and deliberate, while the protection
is wanted on every apply.

**Emit the protective values explicitly — `deletion_policy = "PREVENT"`,
`deletion_protection = true`.** *Rejected.* It changes nothing about what the
provider does and puts an attribute nobody chose into every estate's HCL, which is
the same objection as the provider's attribution label: declared, it reads as a
decision the estate made. It would also have to be suppressible, which is a param
for a value that already is the default.

**A `satz destroy` command that makes the edits, applies them and destroys.**
*Rejected.* It is a command whose whole purpose is to disarm the protection, one
mistyped `--config` from the wrong organisation, and it would have to be refused
or guarded in exactly the cases where it is dangerous. The estate edit is two
lines an operator writes deliberately, reviews in a plan, and can see in git.

**A param, `allow_destroy`, that switches both attributes.** *Rejected.* It is a
new variable for something the language already expresses per node, and a param
is inherited by every node under it — which is the opposite of what a teardown
wants, where the operator destroys a sandbox folder and not the estate around it.

## Consequences

- `tofu destroy` on a satz estate that declares neither attribute is refused,
  twice, and the workflow page says why and what to write.
- The teardown costs two estate edits and two targeted applies before the
  destroy. Both edits are in git, which is where a "why is this project
  destroyable" question is answered.
- A customer estate whose project someone tries to delete by accident is
  protected by the provider, as it is for any other Terraform configuration.
