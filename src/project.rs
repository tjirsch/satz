//! A project on satz, end to end. `satz add-project` writes into the central estate the
//! section that onboards one project — its Google project, IaC service account and state
//! bucket, and the `interface "<name>"` that publishes them — the way `init` writes the
//! infra section; the pull request that carries it is the request's review. `satz init
//! --project` writes the project's own estate from exactly those exports.
//!
//! A pack cannot do the first half: a pack is one instance — its params join one
//! estate-wide namespace, `use … as` reads a file as a map, and nothing expands a list
//! into resources — so one project is one generated section, plain Satz the operator owns
//! afterwards (ADR 0070).

use satz_core::satz::InterfaceFile;

/// The exports `add-project` writes and `init --project` reads, by name.
const PROJECT_ID: &str = "project_id";
const IAC_ACCOUNT: &str = "iac_account";
const STATE_BUCKET: &str = "state_bucket";
const REGION: &str = "default_region";

/// A project's name is an interface name and a folder name: `[a-z][a-z0-9-]*`, and not
/// the two folders `interfaces/` reserves.
pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("`{}`: a project's name is an interface name — lowercase letters, digits and `-`, starting with a letter", name));
    }
    if name == satz_core::satz::CORE_INTERFACE || name == satz_core::satz::COMMON_LIBRARY {
        return Err(format!("`{}` is reserved: interfaces/{}/ is satz's", name, name));
    }
    Ok(())
}

/// The name as a Satz label and estate name.
pub(crate) fn label(name: &str) -> String {
    name.replace('-', "_")
}

/// The section that onboards one project into the central estate. `folder` says where the
/// workload folder is: a folder (`google_folder.workload_folder`) or the organisation.
pub(crate) fn section(name: &str, owner_group: &str, folder: bool) -> String {
    const TEXT: &str = r#"// ---- project "@NAME@": its Google project, IaC service account and state bucket, and the
//      interface `@NAME@` that publishes them — written by `satz add-project` -------------------
google_project {
  @LABEL@ {
    name            = "@NAME@"
    project_id      = "{customer_shortname}-@NAME@-001"
    @PARENT@
    billing_account = billing_account_infra
    project_service = [
      "cloudresourcemanager.googleapis.com",
      "iam.googleapis.com",
      "iamcredentials.googleapis.com",
      "serviceusage.googleapis.com",
      "storage.googleapis.com",
    ]
    google_service_account {
      @LABEL@_iac {
        account_id   = "svc-iac-@NAME@"
        display_name = "IaC provisioner of the project @NAME@"
      }
    }
    google_storage_bucket {
      @LABEL@_state {
        name                        = "{customer_shortname}-@NAME@-001-state"
        location                    = default_region
        force_destroy               = true
        public_access_prevention    = "enforced"
        uniform_bucket_level_access = true
        versioning { enabled = true }
      }
    }
    // The project's IaC service account grants itself what its estate's resource types
    // need (`satz update-prerequisites` there): here it takes the project's IAM and its
    // services, and the state bucket. The owner group reads the project.
    google_project_iam_member {
      "serviceAccount:svc-iac-@NAME@@{customer_shortname}-@NAME@-001.iam.gserviceaccount.com" = [
        "roles/resourcemanager.projectIamAdmin",
        "roles/serviceusage.serviceUsageAdmin",
      ]
      "group:@GROUP@" = [
        "roles/viewer",
      ]
    }
    google_storage_bucket_iam_member {
      bucket = "{customer_shortname}-@NAME@-001-state"
      "serviceAccount:svc-iac-@NAME@@{customer_shortname}-@NAME@-001.iam.gserviceaccount.com" = [
        "roles/storage.objectAdmin",
      ]
    }
  }
}

// The owner group may become the project's IaC service account — that account only.
google_service_account_iam_member {
  service_account_id = "${{google_service_account.@LABEL@_iac.name}}"
  "group:@GROUP@" = [
    "roles/iam.serviceAccountTokenCreator",
    "roles/iam.serviceAccountUser",
  ]
}

// What the project's estate reads: `satz init --project @NAME@ --interface
// interfaces/@NAME@/@NAME@/satz/interface.satz` writes it from these four.
interface "@NAME@" {
  export "project_id"     = "${{google_project.@LABEL@.project_id}}" attach ["google_project_iam_member"] description "The project's Google project"
  export "project_number" = "${{google_project.@LABEL@.number}}" description "Its number, looked up"
  export "iac_account"    = "${{google_service_account.@LABEL@_iac.email}}" description "The IaC service account the project's estate runs as"
  export "state_bucket"   = "${{google_storage_bucket.@LABEL@_state.name}}" description "The bucket the project's estate keeps its state in"
}
"#;
    let parent = if folder {
        "folder_id       = \"${{google_folder.workload_folder.name}}\""
    } else {
        "org_id          = customer_organization_id"
    };
    TEXT.replace("@NAME@", name).replace("@LABEL@", &label(name)).replace("@GROUP@", owner_group).replace("@PARENT@", parent)
}

/// `src` with the project's section at its end. Refused, naming the line: an estate that
/// declares `interface "<name>"` already, and one that publishes no `workload_folder`,
/// which says where a project goes.
pub(crate) fn with_project(src: &str, name: &str, owner_group: &str) -> Result<String, String> {
    valid_name(name)?;
    if !owner_group.contains('@') || owner_group.starts_with("group:") {
        return Err(format!("`--owner-group {}`: the group's address, `<name>@<domain>`", owner_group));
    }
    let header = format!("interface \"{}\"", name);
    if let Some((n, _)) = src.lines().enumerate().find(|(_, l)| l.trim_start().starts_with(&header)) {
        return Err(format!("line {} declares `{}` already — one project is one interface", n + 1, header));
    }
    let export = src.lines().find(|l| l.trim_start().starts_with("export \"workload_folder\""));
    let folder = match export {
        Some(l) => l.contains("google_folder.workload_folder."),
        None => {
            return Err(
                "the estate publishes no `workload_folder`, which is where a project goes — `satz init --workload-folder-name <name>` (a folder) or `satz interview` (`workload_folder_name`, `\"\"` for the organisation) writes the section".to_string(),
            )
        }
    };
    let mut out = src.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&section(name, owner_group, folder));
    Ok(out)
}

/// A static string output of the interface, by name.
fn static_output(file: &InterfaceFile, name: &str) -> Option<String> {
    file.outputs.iter().find(|o| o.name == name).and_then(|o| crate::interface::static_text(&o.value))
}

/// The project's own estate, from the interface `add-project` published for it: it runs as
/// the project's IaC service account, keeps its state in the project's bucket, and reads the
/// central estate through `use_path`. Refused when the interface lacks one of the four
/// exports `add-project` writes.
pub(crate) fn estate(name: &str, file: &InterfaceFile, use_path: &str) -> Result<String, String> {
    valid_name(name)?;
    let read = |export: &str| -> Result<String, String> {
        static_output(file, export).ok_or_else(|| {
            let writer = if export == REGION {
                "`presets/estate-core.satz` exports it: switch its `use` line on in the central estate".to_string()
            } else {
                format!("`satz add-project {}` writes the section that does", name)
            };
            format!(
                "the interface `{}` of the estate `{}` publishes no static `{}` — {}; it publishes: {}",
                file.name,
                file.estate,
                export,
                writer,
                file.outputs.iter().map(|o| format!("`{}`", o.name)).collect::<Vec<_>>().join(", ")
            )
        })
    };
    let project_id = read(PROJECT_ID)?;
    let account = read(IAC_ACCOUNT)?;
    let bucket = read(STATE_BUCKET)?;
    let region = read(REGION)?;
    let account_id = account.split('@').next().unwrap_or(&account).to_string();
    const TEXT: &str = r#"// Generated by `satz init --project` — the estate of the project `@NAME@`: it reads the
// central estate `@CENTRAL@` through the interface file below and runs as the project's
// own IaC service account. Every value below is a param: change it here.

estate @LABEL@

params {
  infra_project_name = "@PROJECT@" // the project this estate runs in, and keeps its state in
  infra_bucket_name  = "@BUCKET@"
  svc_iac_account    = "@ACCOUNT@"
  default_region     = "@REGION@"
  default_zone       = "@REGION@-a"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
}

// The central estate's interface: a value is read as `${{interface.<export>}}`; the
// README beside the file lists the values and what may be attached to, and `CHANGES.md`
// there, after a change of the central estate, what to do about it.
use "@USE@"

terraform {
  backend {
    local { path = "terraform.tfstate" }
    gcs {
      bucket = infra_bucket_name
      prefix = "hcl/state"
    }
  }
}

providers {
  "google" {
    project               = infra_project_name
    region                = default_region
    alias                 = "google"
    user_project_override = true
    billing_project       = infra_project_name
  }
  "google-beta" {
    project               = infra_project_name
    region                = default_region
    alias                 = "google-beta"
    user_project_override = true
    billing_project       = infra_project_name
  }
}

// The project's own resources, in its Google project: `project = "${{interface.project_id}}"`.
"#;
    Ok(TEXT
        .replace("@NAME@", name)
        .replace("@LABEL@", &label(name))
        .replace("@CENTRAL@", &file.estate)
        .replace("@PROJECT@", &project_id)
        .replace("@BUCKET@", &bucket)
        .replace("@ACCOUNT@", &account_id)
        .replace("@REGION@", &region)
        .replace("@USE@", use_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const GROUP: &str = "payments-owners@example.com";

    /// The central estate `init` writes, with or without a workload folder. `tag` keeps two
    /// tests' scratch directories apart: the runner interleaves them.
    fn central(tag: &str, folder: Option<&str>) -> String {
        let dir = std::env::temp_dir().join(format!("satz-project-{}-{}-{}", std::process::id(), tag, folder.is_some()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("C0example.satz");
        let mut args = crate::template::tests::args("first.admin", "example.com");
        args.workload_folder_name = folder.map(str::to_string);
        crate::template::generate_template(&args, None, &path).unwrap();
        let src = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        src
    }

    fn compile(src: &str, loader: &dyn Fn(&str) -> Result<String, String>) -> (satz_core::pipeline::FrontEnd, crate::manifest::Manifest, crate::interface::Interface) {
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("e.satz", src, &resolver, loader).unwrap_or_else(|e| panic!("front-end: {}\n{}", e, src));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty(), "{:?}", folded.conflicts());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).unwrap();
        let i = crate::interface::build("e", &fe.exports, &fe.interfaces, &out.manifest, "hashicorp/google", None)
            .unwrap_or_else(|r| panic!("an export refused: {:?}", r));
        (fe, out.manifest, i)
    }

    /// The section compiles in the estate `init` writes, under the organisation and under a
    /// workload folder, and publishes the four exports the project's estate is written from.
    #[test]
    fn the_section_compiles_and_publishes_what_the_project_needs() {
        for folder in [None, Some("Workloads")] {
            let src = with_project(&central("section", folder), "payments", GROUP).unwrap();
            assert_eq!(satz_core::fmt::format(&src).unwrap(), src, "the section is not in the canonical layout");
            let (_, manifest, i) = compile(&src, &|p| Err(format!("no {}", p)));
            let project = manifest.resources.values().find(|r| r.address() == "google_project.payments").expect("the project");
            match folder {
                None => assert_eq!(project.attrs.get("org_id").map(String::as_str), Some("123456789012"), "{:?}", project.attrs),
                Some(_) => assert_eq!(project.refs.get("folder_id").map(String::as_str), Some("google_folder.workload_folder.name"), "{:?}", project.refs),
            }
            assert!(i.projects().contains(&"payments".to_string()), "{:?}", i.projects());
            let outputs: Vec<(&str, bool)> = i.module_outputs("payments").iter().map(|o| (o.name.as_str(), o.static_text().is_some())).collect();
            for (name, is_static) in [("project_id", true), ("project_number", false), ("iac_account", true), ("state_bucket", true)] {
                assert!(outputs.contains(&(name, is_static)), "{} static={} — {:?}", name, is_static, outputs);
            }
            let attach = i.module_outputs("payments").into_iter().find(|o| o.name == "project_id").unwrap();
            assert_eq!(attach.attach, ["google_project_iam_member"]);
            assert_eq!(i.module_outputs("payments").into_iter().find(|o| o.name == "iac_account").unwrap().static_text().as_deref(), Some("svc-iac-payments@acme-payments-001.iam.gserviceaccount.com"));
            // the grants: the account on its project and its bucket, the group on the account
            assert!(manifest.resources.values().any(|r| r.tf_type == "google_project_iam_member" && r.attrs.get("role").map(String::as_str) == Some("roles/resourcemanager.projectIamAdmin")));
            assert!(manifest.resources.values().any(|r| r.tf_type == "google_storage_bucket_iam_member" && r.attrs.get("bucket").map(String::as_str) == Some("acme-payments-001-state")));
            assert!(manifest.resources.values().any(|r| r.tf_type == "google_service_account_iam_member" && r.attrs.get("member").map(String::as_str) == Some(&format!("group:{}", GROUP))));
        }
    }

    #[test]
    fn a_project_is_added_once_and_only_where_the_estate_says_where_projects_go() {
        let src = central("once", None);
        let once = with_project(&src, "payments", GROUP).unwrap();
        let twice = with_project(&once, "payments", GROUP).unwrap_err();
        assert!(twice.contains("declares `interface \"payments\"` already"), "{}", twice);
        let none = src.lines().filter(|l| !l.contains("export \"workload_folder\"")).collect::<Vec<_>>().join("\n");
        assert!(with_project(&none, "payments", GROUP).unwrap_err().contains("publishes no `workload_folder`"));
        assert!(with_project(&src, "Payments", GROUP).unwrap_err().contains("lowercase"));
        assert!(with_project(&src, "core", GROUP).unwrap_err().contains("reserved"));
        assert!(with_project(&src, "payments", "group:x@example.com").unwrap_err().contains("the group's address"));
        assert!(with_project(&src, "payments", "owners").unwrap_err().contains("the group's address"));
    }

    /// `init --project` writes an estate from the four exports, in the canonical layout,
    /// that compiles as the project's own IaC service account.
    #[test]
    fn the_project_estate_runs_as_the_projects_account_and_reads_the_interface() {
        // the interface the central estate published, as the file a project takes; the
        // region is estate-core's export, which the estate `init` writes switches on later
        let src = format!("{}\nexport \"default_region\" = default_region\n", with_project(&central("estate", None), "payments", GROUP).unwrap());
        let (_, manifest, i) = compile(&src, &|p| Err(format!("no {}", p)));
        let facts = crate::consumer::Facts::of_estate("e", Some(&i), &manifest);
        let text = satz_core::fmt::format(&i.satz_file("payments", &facts, "0.0.0", "e.satz")).unwrap();
        let file = satz_core::satz::parse(&text).unwrap().interface_file.unwrap();
        let estate = estate("payments", &file, "vendor/payments/payments/satz/interface.satz").unwrap();
        assert_eq!(satz_core::fmt::format(&estate).unwrap(), estate, "the project estate is not in the canonical layout");
        assert!(estate.contains("estate payments\n") && estate.contains("svc_iac_account    = \"svc-iac-payments\"") && estate.contains("infra_bucket_name  = \"acme-payments-001-state\""), "{}", estate);
        // compiled through the tail: cloud mode impersonates the project's account
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("payments.satz", &estate, &resolver, &|p| if p.ends_with("interface.satz") { Ok(text.clone()) } else { Err(format!("no {}", p)) }).unwrap();
        let cfg = crate::parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        let graph = crate::pack_graph::Shipped::Missing(std::path::PathBuf::from("presets/pack-graph.json"));
        let t = crate::compile_tail(&fe, &resolver, &reg, &cfg, &graph, "presets", "warn", Path::new("payments.satz"), "payments.satz", &estate);
        let errors: Vec<&str> = t.findings.iter().filter(|f| f.severity == crate::findings::Severity::Error).map(|f| f.message.as_str()).collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let providers = t.providers_tf.expect("providers.tf");
        assert!(providers.contains("impersonate_service_account = \"svc-iac-payments@acme-payments-001.iam.gserviceaccount.com\""), "{}", providers);
        // an interface without the exports is refused by name
        let bare = satz_core::satz::parse("interface \"x\"\n\ncentral {\n  estate = \"e\"\n  organizations = []\n}\n\noutput \"project_id\" {\n  value = \"p\"\n}\n").unwrap().interface_file.unwrap();
        let err = super::estate("payments", &bare, "x").unwrap_err();
        assert!(err.contains("publishes no static `iac_account`") && err.contains("`project_id`"), "{}", err);
        let no_region = satz_core::satz::parse(&text.replace("output \"default_region\"", "output \"region_gone\"")).unwrap().interface_file.unwrap();
        assert!(super::estate("payments", &no_region, "x").unwrap_err().contains("estate-core"));
    }
}
