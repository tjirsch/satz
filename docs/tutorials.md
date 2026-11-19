# satz tutorials

Two walks through satz, each from an empty directory to a running organization, and one
procedure for the two teams that meet when access is delegated to a team, with the
example customer of [`docs/examples.md`](examples.md) — Acme Corp., `acme`,
`example.com` — and the files under [`examples/acme/`](../examples/README.md). Every
command is one you run; every file shown is one satz writes. The reference pages say
what each statement, command and pack does on its own; a tutorial says in which order
they meet.

| tutorial | what you have at the end |
|---|---|
| [An organization from scratch](tutorial-organization.md) | An organization's estate written by `satz init`, bootstrapped, applied as yourself, then run as its estate account; the estate map, the security groups and the CIS baseline on it; and what it publishes to the workload estates beside it. |
| [Workload estates beside the organization](tutorial-projects.md) | Delegated access, and the three presets that provide it: `delegation`, the estate account every delegation has, and the two kinds of delegated resource — which is for whom: `delegated-folders` for a team that creates its own projects in a folder of its own, `delegated-projects` for a team that works in a project the organization creates. Then a customer team with an estate account of its own — an infrastructure project, the estate account, a state bucket, a folder and the roles its work needs — and one Satz template for a Google project that the team runs **the Satz way** (a thin estate binding the template's inputs) or **the HCL way** (the generated `hcl/`, a `terraform.tfvars` and `tofu`), reading the organization's values through its interface and never its state. |
| [Delegated access: the procedure for both sides](tutorial-delegated-access.md) | Delegating access to a folder or a project, as a procedure between the organization's team and a workload's team, by side and by step: what the team asks for, what the organization decides, writes and applies, which interface folders are copied to the team, how the team sets up its workload estate and reads what the transpile says about what it was given — with a diagram of the steps people do and the steps satz does, for the first copy and for every change after it. |


The second tutorial starts where the first ends; the third is followed by two teams, and
needs neither read first. Both assume `satz` and `tofu` on the
path and a `gcloud` login; the values are examples and none of them exists, so replace
them with yours before a plan.
