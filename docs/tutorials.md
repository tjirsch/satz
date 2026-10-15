# satz tutorials

Two walks through satz, each from an empty directory to a running organisation, with the
example customer of [`docs/examples.md`](examples.md) — Acme Corp., `acme`,
`example.com` — and the files under [`examples/acme/`](../examples/README.md). Every
command is one you run; every file shown is one satz writes. The reference pages say
what each statement, command and pack does on its own; a tutorial says in which order
they meet.

| tutorial | what you have at the end |
|---|---|
| [An organisation from scratch](tutorial-organisation.md) | An organisation estate written by `satz init`, bootstrapped, applied as yourself, then run as its IaC service account; the pack map, the security groups and the CIS baseline on it; and what it publishes to the projects beside it. |
| [Projects beside the organisation](tutorial-projects.md) | A customer team with an identity of its own — a project, a service account, a state bucket and the roles its work needs — and one Satz template for a Google project that the team runs **the Satz way** (a thin estate binding the template's inputs) or **the HCL way** (the generated `hcl/`, a `terraform.tfvars` and `tofu`), reading the organisation's values through its interface and never its state. |

The second tutorial starts where the first ends. Both assume `satz` and `tofu` on the
path and a `gcloud` login; the values are examples and none of them exists, so replace
them with yours before a plan.
