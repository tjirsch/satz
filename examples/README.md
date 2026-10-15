# satz examples

Estates that show one use of satz each, compiled by `scripts/smoke.sh` on every pull
request and every push to `main`.

## Layout

```text
examples/
  acme/                         one organisation
    config.toml
    satz/C0example.satz         the organisation's estate — the central estate
    simple-project/             one example: one Google project as a template, used two ways
      config.toml
      satz/simple-project.satz
    shop-dev/                   the template's instance the Satz way: a thin estate
      config.toml
      satz/shop_dev.satz
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
mkdir -p examples/acme/simple-project/vendor
cp -R examples/acme/interfaces/common examples/acme/simple-project/vendor/common

# 3. the example: writes examples/acme/simple-project/hcl/
satz --config examples/acme/simple-project transpile simple-project.satz
```

`hcl/`, `interfaces/` and `vendor/` are written by these commands and are not tracked.
Step 2 is repeated after every change of the organisation's estate. `shop-dev` reaches
the template through its `include_dirs` and needs its own `vendor/` copy. What the
compile writes for a project estate — `interface.tf`, the `local.interface.<export>`
references, the variables of the inputs — is [language
§6.17](../docs/language.md#617-export-and-interface--what-the-estate-publishes-to-the-projects-beside-it)
and [§6.3](../docs/language.md#inputs--a-templates-contract-the-values-a-reader-binds);
the two tutorials walk the same files step by step ([an organisation from
scratch](../docs/tutorial-organisation.md), [projects beside the
organisation](../docs/tutorial-projects.md)).

## The examples

| example | what it shows |
|---|---|
| [`acme/simple-project`](acme/simple-project/satz/simple-project.satz) | One Google project as a **template**: `inputs { application stage cost_centre }` are the values a reader binds, everything else is fixed — the id by the organisation's `customer_shortname` and the inputs, the parent the organisation's `workload_folder`, a four-digit cost centre as a label, and `roles/owner`, `roles/editor` and `roles/viewer` granted to three groups named after the project, which exist in the directory (synced from Microsoft Entra ID) before the apply. Compiled as it stands, its `hcl/` is the HCL team's form — `variables.tf` declares the three inputs, `terraform.tfvars` carries the example values, `main.tf` reads `var.<name>`, `interface.tf` holds the organisation's values — so a team edits `terraform.tfvars` or passes `-var` and runs `tofu` without satz. `satz transpile simple-project.satz --input stage=test` binds one for a run. |
| [`acme/shop-dev`](acme/shop-dev/satz/shop_dev.satz) | The same template the **Satz** way: a thin estate that binds the inputs in its `params { }` and `use`s the template, compiling to the same `main.tf` with the bound values in `terraform.tfvars`. One template, two kinds of team. |

## Adding an example

A new folder under `acme/` with a `config.toml` and its estate in `satz/` is compiled
by the smoke matrix with `interfaces/common/` of the organisation in its `vendor/`. An
example that needs more from the organisation adds the export to
`acme/satz/C0example.satz`. Values come from the table in `docs/examples.md`; `satz fmt
examples` formats the files.
