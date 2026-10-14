# satz examples

Estates that show one use of satz each, compiled by `scripts/smoke.sh` on every pull
request and every push to `main`.

## Layout

```text
examples/
  acme/                         one organisation
    config.toml
    satz/C0example.satz         the organisation's estate — the central estate
    cost-centre-project/        one example: a project beside the organisation
      config.toml
      satz/workloads.satz
```

`acme/` is the organisation every example below it belongs to: the estate `satz init
--workload-folder-name Workloads` writes, with `presets/estate-core.satz` in use. It
publishes the core exports — `workload_folder`, `customer_domain`,
`customer_shortname`, `default_region`, `organization_id`, `infra_project_id`,
`iac_service_account`, `infra_folder` — as `interfaces/common/core/`.

Each subfolder of `acme/` is one example: a project, an estate with its own config and
state that reads the organisation through the interface file and changes nothing in the
organisation's estate ([projects beside the
estate](../docs/workflows.md#projects-beside-the-estate)).

## The values are examples

Every id, name and domain is Customer A of [`docs/examples.md`](../docs/examples.md):
`acme`, `example.com`, `C0example`, organisation `123456789012`, billing account
`012345-6789AB-CDEF01`, project `acme-infra-001`. None of them exists. Before a plan or
an apply:

- in the organisation's estate, replace the values in `params { … }` of
  `acme/satz/C0example.satz` — or take your own organisation's estate in its place: an
  example reads only the exports named in its header;
- in an example, replace the values its `params { … }` marks as example values. What it
  reads from the organisation arrives through the interface and is not written there.

`config.toml` in both folders points `presets_dir`, `schema_dir` and an include
directory into this repository, so the files compile in place. In a repository of your
own these are the `presets/` and `schemas/` folders beside the estate.

## Running an example

From the repository root, with `satz` on the path:

```bash
# 1. the organisation: writes examples/acme/hcl/ and examples/acme/interfaces/
satz --config examples/acme transpile C0example.satz

# 2. the example takes the organisation's common interfaces whole
mkdir -p examples/acme/cost-centre-project/vendor
cp -R examples/acme/interfaces/common examples/acme/cost-centre-project/vendor/common

# 3. the example: writes examples/acme/cost-centre-project/hcl/
satz --config examples/acme/cost-centre-project transpile workloads.satz
```

`hcl/`, `interfaces/` and `vendor/` are written by these commands and are not tracked.
Step 2 is repeated after every change of the organisation's estate.

## The examples

| example | what it shows |
|---|---|
| [`acme/cost-centre-project`](acme/cost-centre-project/satz/workloads.satz) | Google projects created from a list, one entry each: the project id follows a naming scheme built on the organisation's `customer_shortname`, the parent is the organisation's `workload_folder`, a label carries a four-digit cost centre, and `roles/owner`, `roles/editor` and `roles/viewer` go to three groups named `<project id>-<role>@<customer_domain>`. The groups exist in the directory before the apply — synced from Microsoft Entra ID — and are not declared. A `request` point holds every entry to the naming scheme and the four digits at compile. |

## Adding an example

A new folder under `acme/` with a `config.toml` and its estate in `satz/` is compiled
by the smoke matrix with `interfaces/common/` of the organisation in its `vendor/`. An
example that needs more from the organisation adds the export to
`acme/satz/C0example.satz`. Values come from the table in `docs/examples.md`; `satz fmt
examples` formats the files.
