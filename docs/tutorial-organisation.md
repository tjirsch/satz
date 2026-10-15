# An organisation from scratch

One organisation, from an empty directory to an estate that runs as its own IaC service
account, carries the pack map, the security groups and the CIS baseline, and publishes
what the projects beside it read. The customer is Acme Corp. of
[`docs/examples.md`](examples.md): `acme`, `example.com`, directory `C0example`,
organisation `123456789012`, billing account `012345-6789AB-CDEF01`. None of the values
exists; replace them with yours. The result is the estate under
[`examples/acme/`](../examples/README.md), written by the same command.

What each step does in full is [workflows: from nothing to applied](workflows.md#from-nothing-to-applied);
this page is the order in which you type it.

## What you need

- `satz` and `tofu` on the path, and `gcloud` logged in:

  ```bash
  gcloud auth application-default login
  ```

- On the Google side, the account you logged in with holds **Super Admin** in the
  Workspace or Cloud Identity tenant, **Organization Administrator** on the organisation
  and **Billing Account Administrator** on the billing account. Bootstrap tests the
  permissions it needs before it creates anything and prints the grant commands for what
  is missing.

## 1. Write the estate

In an empty directory — the repository of this organisation from now on — `satz init`
writes the estate from the day-0 values and asks for what they leave out:

```bash
mkdir acme && cd acme
satz init \
  --customer-id C0example \
  --customer-shortname acme \
  --customer-domain example.com \
  --customer-organization-id 123456789012 \
  --billing-account-infra 012345-6789AB-CDEF01 \
  --iac-user admin@example.com \
  --default-region europe-west3 \
  --workload-folder-name Workloads \
  --interview
```

A value you leave out is derived from the login where it can be — the domain and the
first admin from the identity, the organisation and the directory id from
`organizations:search`, the billing account when exactly one is open. `--interview`
fetches the preset library before the file is written, so the estate carries the pack
menu, and then asks the day-0 questions neither the flags nor the login answer: here one,
the customer's full name, `customer_longname` — answer `Acme Corp.`. It ends with every
question answered and names the next command.

It writes:

| file | what |
|---|---|
| `config.toml` | where the estate, the generated HCL, the interfaces, the schemas and the presets live |
| `satz/C0example.satz` | the estate: its `params { }`; `use "presets/estate-core.satz"`, the pack that declares the day-0 params with their questions; the providers, the IaC group and service account, the infrastructure folder with the management project and the state bucket in it, the `Workloads` folder and what the estate publishes (`export "workload_folder"`, `export "infra_folder"`); and the pack menu — one commented `use "presets/…" when <gate>` line per pack of the library, under the phase it can be adopted in |
| `presets/` | the library, with its pack graph |
| `schemas/` | the provider schemas every resource key is checked against, read from the provider through `tofu init` |
| `hcl/` | empty: where the HCL goes |
| `.gitignore` | what is generated and what is private |

Open `satz/C0example.satz`. Two things to read: the `params { }` block, which binds
every day-0 value by name and is the one place a value is changed; and the resources
below it, which are the Terraform provider's resource types to the underscore. The pack
menu is commented: the day-0 scaffold deploys alone, and every pack comes after the
identity it runs as exists.

```bash
satz questions C0example.satz --format text --out -
```

lists each question of the core pack with its answer. Without `--interview`, `init`
writes the core line commented and no menu: `satz get-presets` then `satz merge-presets`
write the menu, and `satz interview` offers to switch the core line on.

Commit the directory. From here every change to the organisation is a change to this
file, and `tofu`'s state is the only other thing that moves.

## 2. Bootstrap the organisation

```bash
satz bootstrap C0example.satz --dry-run
satz bootstrap C0example.satz
```

The dry run prints what the real run would create and runs its pre-flight: the identity
against `first_admin`, the organisation resolved and matched to the directory id, the
permissions tested one by one. The real run creates the day-0 infrastructure — the
infrastructure folder, the management project `acme-infra-001`, its billing link, the
foundation APIs, the state bucket `acme-infra-001-state` — then transpiles, initialises
`tofu` and imports what it made, so the estate manages it from the first plan on. It
creates no service account and grants nothing beyond the roles it may have to give the
caller: the identity layer is declared in the estate and comes with the first apply.

Bootstrap ends by naming the next commands with the estate's own values. They are the
next two sections.

## 3. Apply as yourself

```bash
satz update-prerequisites C0example.satz
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`update-prerequisites` writes into the estate the roles its resource types need on the
IaC service account and the APIs they need on the project the calls are billed to.
`transpile` compiles the estate to `hcl/` — `main.tf`, `providers.tf`, `variables.tf`,
`terraform.tfvars`, `imports.tf`, and `outputs.tf` with what the estate exports — and
`--plan` runs `tofu plan` on it after enabling the APIs the plan would otherwise trip
over. Read the plan: the Cloud Identity group `svc-iac-users`, the service account
`svc-iac-001`, its roles on the organisation and the billing account, the bucket's
settings. `--apply` creates them. This run authenticates as you: the estate is in
`deployment_mode = "local"`, the state is a file in `hcl/`.

## 4. Switch to the service account

```bash
satz migrate C0example.satz --mode cloud
satz whoami C0example.satz
satz plan
```

`migrate` sets `deployment_mode = "cloud"` in the estate, moves the state into the bucket,
re-initialises `tofu` and checks that the service account holds **Groups Admin**, the
Workspace role the group resources need; from here every provider and the backend impersonate
`svc-iac-001@acme-infra-001.iam.gserviceaccount.com`, and so does every live satz
command. `whoami` prints that: `runs as: svc-iac-001@… — impersonated by admin@…`. The
plan must read **No changes** — the estate, the state and the organisation agree — and
that is the condition for the first pack.

## 5. The packs, one at a time

The first line of the menu is the map, and it goes on by its path:

```bash
satz add-pack C0example.satz presets/estate-map.satz
```

The map declares the questions every other pack's line is gated on — which security
group model, which packs on — and `add-pack` says how many it opened and names the
command that asks them:

```bash
satz interview C0example.satz
satz packs C0example.satz --format text --out -
```

Answer the model (`S1`, the five admin groups) and the recommended packs with their
defaults, or run `satz interview C0example.satz --accept-defaults` and read `packs` for
what is on: the pack, its gate's answer, its line, what it needs and what needs it. A
yes switches the pack's line on, so the estate's file is the record of every choice.

Each pack is its own plan and apply, in the order the menu writes them — the security
groups before anything that grants to them:

```bash
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

The CIS baseline is the one to read before applying: its organisation policies land in one
apply. Switch it on and read what it asks for first:

```bash
satz add-pack C0example.satz use_cis_baseline
satz adopt C0example.satz --execute --import
satz transpile C0example.satz --plan
satz transpile C0example.satz --apply
```

`add-pack` binds the gate and activates the line; the pack's notice says why `adopt`
comes first — Google sets some of these policies on every new organisation, and an apply
that creates a policy that exists stops on `409 POLICY_ALREADY_EXISTS`. `adopt` resolves
what is already there, writes its ids into the estate as `"import-id"`, so the plan
imports instead of creating, and binds `cis_baseline_adopted = true`, which every command
that writes to the organisation refuses to run without. A pack that needs a role the service account does not hold
yet is named by the compile; `satz update-prerequisites C0example.satz` writes the grant,
and the apply creates the grant and what needs it.

What the estate claims against the frameworks it is held to, and what it has in fact:

```bash
satz require cis-gcp-5.0 C0example.satz --format text --out -
satz report-compliance cis-gcp-5.0 C0example.satz --format markdown --out evidence/cis-5.0.md
```

`require` judges the declared estate — which controls the packs claim and whether every
witness they name is emitted; `report-compliance` verifies the witnesses live, compares
each organisation policy by value, and writes the report with its append-only evidence
history into `evidence/`, beside the estate and git-ignored.

## 6. What the organisation publishes

Every `transpile` writes `interfaces/` beside `hcl/`:

```bash
satz interfaces C0example.satz --format text --out -
```

`interfaces/common/core/` holds the core exports — `organization_id`,
`customer_domain`, `customer_shortname`, `default_region`, `infra_project_id`,
`iac_service_account` from `estate-core`, `infra_folder` and `workload_folder` from the
estate — as an HCL module and as a Satz file, with a README that names each value, how it
is obtained and what may be attached to it. That folder is what a project beside the
organisation reads, and the next tutorial starts from it:
[projects beside the organisation](tutorial-projects.md).

## Where things stand

| | |
|---|---|
| the estate | `satz/C0example.satz`, the one file every change goes through, in your repository |
| the state | `gs://acme-infra-001-state`, read and written as the IaC service account |
| the identity | `svc-iac-001@acme-infra-001.iam.gserviceaccount.com`, impersonated by the members of `svc-iac-users@example.com` |
| the organisation | the infrastructure folder with its project and bucket, the `Workloads` folder, the five S1 groups and their roles, the CIS baseline's policies |
| what it publishes | `interfaces/common/`, taken whole by every project beside it |
| the next change | edit the file, `satz transpile C0example.satz --plan`, read, `--apply` |

A pack the library gains later is one `satz merge-presets C0example.satz` away, written
into the menu commented; `satz self-update` brings the binary that knows it.
