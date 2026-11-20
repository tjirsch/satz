//! A workload estate on satz, end to end. An entry of `delegations` in
//! `presets/delegation.satz` gives the organization's estate the delegation's account — an
//! infrastructure project, the account, its state bucket — and the interface
//! `delegation-<name>` that publishes them; a preset for a delegated resource (`presets/delegated-folders.satz`,
//! `presets/delegated-projects.satz`) publishes what that account works in, one interface per
//! entry. `satz init --from` takes those interface folders and writes the workload estate
//! from them.

use satz_core::satz::InterfaceFile;

/// The exports `presets/delegation.satz` publishes for a delegation and `init --from` reads.
const INFRASTRUCTURE_PROJECT: &str = "infrastructure_project_id";
const ESTATE_ACCOUNT: &str = "estate_account";
const ESTATE_ACCOUNT_ID: &str = "estate_account_id";
const STATE_BUCKET: &str = "state_bucket";
const REGION: &str = "default_region";

/// The prefix of a delegation's own interface, `delegation-<name>`.
pub(crate) const BASE_PREFIX: &str = "delegation-";

/// An estate's name is a folder name and a label: `[a-z][a-z0-9-]*`, and not the two
/// folders `interfaces/outgoing/` reserves.
pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("`{}`: a name is an interface name — lowercase letters, digits and `-`, starting with a letter", name));
    }
    if name == satz_core::satz::CORE_INTERFACE || name == satz_core::satz::COMMON_LIBRARY {
        return Err(format!("`{}` is reserved: interfaces/outgoing/{}/ is satz's", name, name));
    }
    Ok(())
}

/// The name as a Satz label and estate name.
pub(crate) fn label(name: &str) -> String {
    name.replace('-', "_")
}

/// A static string output of the interface, by name.
fn static_output(file: &InterfaceFile, name: &str) -> Option<String> {
    file.outputs.iter().find(|o| o.name == name).and_then(|o| crate::interface::static_text(&o.value))
}

/// Whether the interface carries a delegation's account: the three exports of the base.
fn is_base(file: &InterfaceFile) -> bool {
    [INFRASTRUCTURE_PROJECT, ESTATE_ACCOUNT, STATE_BUCKET].iter().all(|e| static_output(file, e).is_some())
}

/// The name an estate written from these interfaces takes when none is given: the
/// delegation's, without the prefix of its interface.
pub(crate) fn default_name(files: &[(String, InterfaceFile)]) -> Option<String> {
    let base = files.iter().find(|(_, f)| is_base(f))?;
    Some(base.1.name.strip_prefix(BASE_PREFIX).unwrap_or(&base.1.name).to_string())
}

/// A workload estate, from the interface folders it was given: `files` is each folder's
/// interface with the path the estate `use`s. Exactly one of them is a delegation's own —
/// the account: the estate runs as it, in its infrastructure project, with its
/// state in its bucket under the estate's name, so several estates of one delegation share
/// the bucket. Every other is a delegated resource that account works in, any number of
/// them: a value is read by the name of its folder. Refused when no interface, or more than
/// one, carries an account, or when none carries the region.
pub(crate) fn estate(name: &str, files: &[(String, InterfaceFile)]) -> Result<String, String> {
    valid_name(name)?;
    let listed = || files.iter().map(|(_, f)| format!("`{}`", f.name)).collect::<Vec<_>>().join(", ");
    let bases: Vec<&InterfaceFile> = files.iter().map(|(_, f)| f).filter(|f| is_base(f)).collect();
    let base = match bases.as_slice() {
        [one] => *one,
        [] => {
            // a folder copied before the exports were renamed carries the old names
            let stale: Vec<String> = files
                .iter()
                .filter(|(_, f)| satz_core::pipeline::RENAMED_EXPORTS.iter().any(|(old, _)| static_output(f, old).is_some()))
                .map(|(_, f)| format!("`{}`", f.name))
                .collect();
            return Err(format!(
                "none of the interfaces {} carries a delegation's account — the static exports `{}`, `{}` and `{}`, which an entry of `delegations` in `presets/delegation.satz` publishes as the interface `{}<name>`: take that folder too{}",
                listed(),
                INFRASTRUCTURE_PROJECT,
                ESTATE_ACCOUNT,
                STATE_BUCKET,
                BASE_PREFIX,
                if stale.is_empty() {
                    String::new()
                } else {
                    format!(
                        ". {} carries the names before the rename (`iac_account`, `deploy_project_id`) — it was copied from an older transpile of the organization's estate; copy it again",
                        stale.join(", ")
                    )
                }
            ));
        }
        many => {
            return Err(format!(
                "{} of the interfaces {} carry a delegation's account ({}): an estate runs as one account — give the folders of one delegation",
                many.len(),
                listed(),
                many.iter().map(|f| format!("`{}`", f.name)).collect::<Vec<_>>().join(", ")
            ));
        }
    };
    let region = files.iter().find_map(|(_, f)| static_output(f, REGION)).ok_or_else(|| {
        format!(
            "none of the interfaces {} publishes a static `{}` — `presets/estate-core.satz` exports it: switch its `use` line on in the organization's estate",
            listed(),
            REGION
        )
    })?;
    // the estate account's id is what `svc_iac_account` reads; a folder without it was
    // copied from a transpile of the organization's estate that did not publish it yet
    if static_output(base, ESTATE_ACCOUNT_ID).is_none() {
        return Err(format!(
            "the interface `{}` publishes no static `{}`, the estate account's id that `svc_iac_account` reads — it was copied from an older transpile of the organization's estate; copy it again",
            base.name, ESTATE_ACCOUNT_ID
        ));
    }
    let uses: String = files.iter().map(|(path, _)| format!("use \"{}\"\n", path)).collect();
    let reads: String = files
        .iter()
        .map(|(_, f)| f)
        .filter(|f| !is_base(f))
        .map(|f| {
            if f.outputs.iter().any(|o| o.name == crate::prerequisites::DELEGATED_FOLDER) {
                format!("//   under the delegated folder `{0}`:  folder_id = \"${{{{interface.{0}.{1}}}}}\"\n", f.name, crate::prerequisites::DELEGATED_FOLDER)
            } else {
                format!("//   in the delegated project `{0}`:    project   = \"${{{{interface.{0}.project_id}}}}\"\n", f.name)
            }
        })
        .collect();
    const TEXT: &str = r#"// Generated by `satz init --from` — the workload estate `@NAME@` of the delegation `@BASE@`:
// it reads the organization's estate `@CENTRAL@` through the interface files below and runs as
// the delegation's account. Every value below is a param: change it here.

estate @LABEL@

params {
  // The delegation's estate account, `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`,
  // and its state bucket, read from the folder `@BASE@`: the estate runs as the account,
  // its calls are billed to the infrastructure project and its state is kept in the bucket.
  // A copy of the folder after a change of the organization's estate changes them here too.
  infra_project_name = "${{interface.@BASE@.@PROJECT_EXPORT@}}"
  infra_bucket_name  = "${{interface.@BASE@.@BUCKET_EXPORT@}}"
  svc_iac_account    = "${{interface.@BASE@.@ACCOUNT_EXPORT@}}"
  default_region     = "@REGION@"
  default_zone       = "@REGION@-a"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
}

// What the organization's estate delegated, one interface folder each. A value is read by
// the folder's name, `${{interface.<folder>.<value>}}`; the README beside each file lists
// the values and what may be attached to, and `CHANGES.md` there, after a change of the
// organization's estate, what to do about it.
@USES@
terraform {
  backend {
    local { path = "terraform.tfstate" }
    gcs {
      bucket = infra_bucket_name
      prefix = "@LABEL@/state"
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

// The estate's own resources. A value is read by the name of its interface folder:
@READS@"#;
    Ok(TEXT
        .replace("@NAME@", name)
        .replace("@LABEL@", &label(name))
        .replace("@BASE@", &base.name)
        .replace("@CENTRAL@", &base.estate)
        .replace("@PROJECT_EXPORT@", INFRASTRUCTURE_PROJECT)
        .replace("@BUCKET_EXPORT@", STATE_BUCKET)
        .replace("@ACCOUNT_EXPORT@", ESTATE_ACCOUNT_ID)
        .replace("@REGION@", &region)
        .replace("@USES@", &uses)
        .replace("@READS@", &reads))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    /// The estate `satz init` writes for the example organization, with its workload folder
    /// answered, the delegation packs, the entries given and `extra` params.
    fn central(tag: &str, delegations: &str, projects: &str, extra: &str) -> String {
        // one directory per call: tests that take the same tag run at once
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("satz-project-{}-{}-{}", std::process::id(), tag, call));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("C0example.satz");
        let args = crate::template::tests::args("first.admin", "example.com");
        crate::template::generate_template(&args, None, &path).unwrap();
        let src = crate::template::tests::answered_workload_folder(&std::fs::read_to_string(&path).unwrap(), "Workloads");
        let _ = std::fs::remove_dir_all(&dir);
        let src = src.replacen("params {\n", &format!("params {{\n  delegations = [{}]\n  delegated_projects = [{}]\n{}", delegations, projects, extra), 1);
        format!("{}\nuse \"presets/delegation.satz\"\nuse \"presets/delegated-projects.satz\"\n", src)
    }

    /// The library's packs from the repository, every other path refused.
    fn presets(p: &str) -> Result<String, String> {
        match p {
            "presets/delegation.satz" | "presets/delegated-projects.satz" => std::fs::read_to_string(p).map_err(|e| e.to_string()),
            other => Err(format!("no {}", other)),
        }
    }

    fn compile(src: &str, loader: &dyn Fn(&str) -> Result<String, String>) -> (crate::manifest::Manifest, crate::interface::Interface) {
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
        (out.manifest, i)
    }

    const DELEGATIONS: &str = "{ name = \"payments\" estate_account_user_group = \"payments-owners@example.com\" }, { name = \"data\" estate_account_user_group = \"lake-owners@example.com\" }";
    const PROJECTS: &str = "{ name = \"payments\" delegation = \"payments\" }, { name = \"data-lake\" delegation = \"data\" }, { name = \"data-mart\" delegation = \"data\" }";

    /// An estate account user group that is no address, and a name that is no interface name, are
    /// refused at transpile — naming the entry and the value — rather than at apply.
    #[test]
    fn a_malformed_entry_is_refused_at_compile() {
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        for (entry, says) in [
            ("{ name = \"payments\" estate_account_user_group = \"payments-owners\" }", "entry `payments` has `estate_account_user_group` = \"payments-owners\""),
            ("{ name = \"payments\" estate_account_user_group = \"group:payments-owners@example.com\" }", "entry `payments` has `estate_account_user_group` = \"group:payments-owners@example.com\""),
            ("{ name = \"payments\" estate_account_user_group = \"payments-owners@example.com \" }", "entry `payments` has `estate_account_user_group`"),
            ("{ name = \"Payments\" estate_account_user_group = \"payments-owners@example.com\" }", "entry `Payments`"),
        ] {
            let src = central(&format!("bad-{}", says.len()), entry, "", "");
            let e = satz_core::pipeline::compile_estate("e.satz", &src, &resolver, &presets).err().unwrap_or_else(|| panic!("{} transpiled", entry));
            assert!(e.msg.contains(says), "{}: {}", entry, e.msg);
        }
    }

    /// A delegation is an identity with its own interface; a delegated project is a
    /// Google project in the workload folder, or another parent the estate names, with the delegation's
    /// account on it, and its own interface. One account works in several projects, and a
    /// project may carry its delegation's name.
    #[test]
    fn a_delegation_is_an_identity_and_each_project_names_the_one_that_works_in_it() {
        // the projects stand in the workload folder unless the estate names another parent
        for (tag, extra, parent) in [
            ("pack", "", "google_folder.workload_folder.name"),
            ("pack-other", "  delegated_projects_folder = \"google_folder.infra_folder.name\"\n", "google_folder.infra_folder.name"),
        ] {
            let (manifest, i) = compile(&central(tag, DELEGATIONS, PROJECTS, extra), &presets);
            let address = |a: &str| manifest.resources.values().find(|r| r.address() == a).unwrap_or_else(|| panic!("no {}", a));
            // the delegation's project and the delegated project of the same name stand apart
            address("google_project.deploy_payments");
            let project = address("google_project.data_lake");
            assert_eq!(project.refs.get("folder_id").map(String::as_str), Some(parent), "{:?}", project.refs);
            // a delegation's infrastructure project stands in the infrastructure folder
            let deploy = address("google_project.deploy_payments");
            assert_eq!(deploy.refs.get("folder_id").map(String::as_str), Some("google_folder.infra_folder.name"), "{:?}", deploy.refs);
            for (name, exports) in [
                ("delegation-payments", vec![("infrastructure_project_id", true), ("estate_account", true), ("estate_account_id", true), ("state_bucket", true), ("estate_account_user_group", true)]),
                ("payments", vec![("project_id", true), ("project_number", false)]),
                ("data-lake", vec![("project_id", true), ("project_number", false)]),
            ] {
                assert!(i.projects().contains(&name.to_string()), "{} — {:?}", name, i.projects());
                let outputs: Vec<(&str, bool)> = i.module_outputs(name).iter().map(|o| (o.name.as_str(), o.static_text().is_some())).collect();
                for want in exports {
                    assert!(outputs.contains(&want), "{}: {:?} — {:?}", name, want, outputs);
                }
            }
            assert!(!i.module_outputs("payments").iter().any(|o| o.name == "estate_account"), "a project carries an account of its own");
            assert_eq!(i.module_outputs("payments").into_iter().find(|o| o.name == "project_id").unwrap().attach, ["google_project_iam_member"]);
            assert_eq!(
                i.module_outputs("delegation-data").into_iter().find(|o| o.name == "estate_account").unwrap().static_text().as_deref(),
                Some("svc-deploy-data@acme-deploy-data.iam.gserviceaccount.com")
            );
            // the folder's README names the reads a workload estate binds the account with
            let readme = i.readme("delegation-data", "0.0.0", "satz/e.satz", None);
            for want in [
                "`svc_iac_account = \"${{interface.delegation-data.estate_account_id}}\"`",
                "`infra_project_name = \"${{interface.delegation-data.infrastructure_project_id}}\"`",
                "`infra_bucket_name = \"${{interface.delegation-data.state_bucket}}\"`",
            ] {
                assert!(readme.contains(want), "{} — {}", want, readme);
            }
            // the one account of `data` holds the project roles on both of its projects
            let grants = manifest
                .resources
                .values()
                .filter(|r| r.tf_type == "google_project_iam_member" && r.attrs.get("role").map(String::as_str) == Some("roles/resourcemanager.projectIamAdmin"))
                .count();
            assert_eq!(grants, 3, "one grant per delegated project");
            assert!(manifest.resources.values().any(|r| r.tf_type == "google_storage_bucket_iam_member" && r.attrs.get("bucket").map(String::as_str) == Some("acme-deploy-payments-state")));
            assert!(manifest.resources.values().any(|r| r.tf_type == "google_service_account_iam_member" && r.attrs.get("member").map(String::as_str) == Some("group:lake-owners@example.com")));
        }
        // no entry, no project and no interface
        let (manifest, i) = compile(&central("none", "", "", ""), &presets);
        assert!(!manifest.resources.values().any(|r| r.tf_type == "google_project" && r.address().contains("deploy_")));
        assert!(i.projects().is_empty(), "{:?}", i.projects());
    }

    /// The interface folders the organization's estate publishes for `delegations` and
    /// `delegated_projects`, as a workload repository takes them: `taken(name)` is the path
    /// the estate `use`s, the file's text and the file parsed.
    pub(crate) fn published() -> impl Fn(&str) -> (String, String, InterfaceFile) {
        // the region is estate-core's export, which the estate `init` writes switches on later
        let src = format!("{}\nexport \"default_region\" = default_region\n", central("estate", DELEGATIONS, PROJECTS, ""));
        let (manifest, i) = compile(&src, &presets);
        let facts = crate::consumer::Facts::of_estate("e", Some(&i), &manifest);
        move |name: &str| {
            let text = satz_core::fmt::format(&i.satz_file(name, &facts, "0.0.0", "e.satz")).unwrap();
            let file = satz_core::satz::parse(&text).unwrap().interface_file.unwrap();
            (format!("interfaces/incoming/{0}/{0}/satz/interface.satz", name), text, file)
        }
    }

    /// The workload estate `src` through the whole transpile, reading the folders `taken`.
    fn tail(src: &str, taken: &[&(String, String, InterfaceFile)]) -> crate::Tail {
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let load = |p: &str| taken.iter().find(|t| t.0 == p).map(|t| t.1.clone()).ok_or_else(|| format!("no {}", p));
        let fe = satz_core::pipeline::compile_estate("data_lake.satz", src, &resolver, &load).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        let cfg = crate::parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        let graph = crate::pack_graph::Shipped::Missing(std::path::PathBuf::from("presets/pack-graph.json"));
        crate::compile_tail(&fe, &resolver, &reg, &cfg, &graph, "presets", "warn", Path::new("data_lake.satz"), "data_lake.satz", src)
    }

    /// A workload estate uses no preset, so the preset library's pack graph missing beside
    /// it is no finding; an organization's estate gets one note
    /// (`a_missing_pack_graph_is_one_note` in `src/main.rs`).
    #[test]
    fn a_workload_estate_is_not_sent_for_the_preset_library() {
        let taken = published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let written = estate("data-lake", &[(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone())]).unwrap();
        let t = tail(&written, &[&base, &project]);
        let menu: Vec<&str> = t.findings.iter().filter(|f| f.kind == crate::findings::Kind::UnadoptedPack).map(|f| f.message.as_str()).collect();
        assert!(menu.is_empty(), "{:?}", menu);
        assert!(t.providers_tf.is_some(), "the transpile ran to the end");
    }

    /// A param satz composes into text of its own — the account the estate runs as,
    /// `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`, and the project its
    /// calls are billed to — may read an interface folder by name: the providers, the backend
    /// and the prerequisite check name the export's value. A value the organization's estate
    /// looks up is refused there, naming the param and the value.
    #[test]
    fn a_named_read_in_a_composed_param_is_the_exports_value() {
        let taken = published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let files = vec![(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone())];
        let written = estate("data-lake", &files).unwrap();
        let src = written
            .lines()
            .map(|l| match l.split_whitespace().next() {
                Some("infra_project_name") => "  infra_project_name = \"${{interface.delegation-data.infrastructure_project_id}}\"".to_string(),
                Some("svc_iac_account") => "  svc_iac_account    = \"svc-deploy-data\"".to_string(),
                _ => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(src.contains("${{interface.delegation-data.infrastructure_project_id}}"), "{}", src);
        let t = tail(&src, &[&base, &project]);
        let errors: Vec<&str> = t.findings.iter().filter(|f| f.severity == crate::findings::Severity::Error).map(|f| f.message.as_str()).collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let unresolved: Vec<&str> = t.findings.iter().map(|f| f.message.as_str()).filter(|m| m.contains("interface.")).collect();
        assert!(unresolved.is_empty(), "a finding names the read, not its value: {:?}", unresolved);
        let providers = t.providers_tf.expect("providers.tf");
        assert!(!providers.contains("interface.delegation"), "{}", providers);
        assert!(providers.contains("impersonate_service_account = \"svc-deploy-data@acme-deploy-data.iam.gserviceaccount.com\""), "{}", providers);
        assert!(providers.contains("billing_project = local.interface_static.delegation_data__infrastructure_project_id"), "{}", providers);

        // the identity every live call binds reads the same params
        let load = |p: &str| [&base, &project].iter().find(|t| t.0 == p).map(|t| t.1.clone()).ok_or_else(|| format!("no {}", p));
        let env = satz_core::pipeline::estate_params("data_lake.satz", &src, &load).unwrap();
        let declared = crate::gcp::identity::EstateDeclaration::from_env("data_lake.satz".into(), &env).unwrap();
        assert_eq!(declared.impersonation_target(), Some("svc-deploy-data@acme-deploy-data.iam.gserviceaccount.com"));

        // a value the organization's estate looks up cannot name the account
        let looked_up = src.replace(
            "\"${{interface.delegation-data.infrastructure_project_id}}\"",
            "\"${{interface.data-lake.project_number}}\"",
        );
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let e = satz_core::pipeline::compile_estate("data_lake.satz", &looked_up, &resolver, &load).err().expect("a looked-up value names the account");
        assert!(e.msg.contains("`infra_project_name`") && e.msg.contains("interface.data-lake.project_number") && e.msg.contains("looks up"), "{}", e.msg);
        assert_eq!(e.line, crate::findings::param_line(&looked_up, "infra_project_name").unwrap() as usize, "{}", e);
        let e = satz_core::pipeline::estate_params("data_lake.satz", &looked_up, &load).expect_err("the identity is refused the same");
        assert!(e.msg.contains("`infra_project_name`") && e.msg.contains("looks up"), "{}", e.msg);
    }

    /// `terraform.tfvars` and `variables.tf` hold each param as the HCL code reads it: a
    /// named read of a value the interface folder publishes is the export's value, a read of
    /// a value the organization's estate looks up has no literal before the plan and leaves
    /// the param out of both files, and a read that does not resolve is refused at its param.
    #[test]
    fn tfvars_binds_a_named_read_as_the_exports_value() {
        let taken = published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let files = vec![(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone())];
        let written = estate("data-lake", &files).unwrap();
        let with = |extra: &str| {
            written
                .lines()
                .map(|l| match l.split_whitespace().next() {
                    Some("infra_project_name") => format!("  infra_project_name = \"${{{{interface.delegation-data.infrastructure_project_id}}}}\"\n{}", extra),
                    _ => l.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let src = with("  lake_project_id = \"${{interface.data-lake.project_id}}\"\n  lake_project_number = \"${{interface.data-lake.project_number}}\"");
        assert!(src.contains("lake_project_number = \"${{interface.data-lake.project_number}}\""), "{}", src);
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let load = |p: &str| [&base, &project].iter().find(|t| t.0 == p).map(|t| t.1.clone()).ok_or_else(|| format!("no {}", p));
        let fe = satz_core::pipeline::compile_estate("data_lake.satz", &src, &resolver, &load).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        let tfvars = crate::emitter::emit_tfvars(&fe.tfvars, &fe.inputs);
        let variables = crate::emitter::emit_variables(&fe.tfvars, &Default::default(), &fe.inputs);
        assert!(!tfvars.contains("interface."), "no read is written as text:\n{}", tfvars);
        assert!(tfvars.contains("infra-project-name = \"acme-deploy-data\"\n"), "{}", tfvars);
        assert!(tfvars.contains("lake-project-id = \"acme-data-lake-001\"\n"), "{}", tfvars);
        assert!(variables.contains("variable \"lake-project-id\""), "{}", variables);
        assert!(!tfvars.contains("lake-project-number"), "a looked-up value has no literal:\n{}", tfvars);
        assert!(!variables.contains("lake-project-number"), "and no variable without one:\n{}", variables);

        let unresolved = with("  lake_project_id = \"${{interface.data-lake.nope}}\"");
        let e = satz_core::pipeline::compile_estate("data_lake.satz", &unresolved, &resolver, &load).err().expect("an unresolved read is refused");
        assert!(e.msg.contains("`lake_project_id`") && e.msg.contains("no value `nope`"), "{}", e.msg);
        assert_eq!(e.line, crate::findings::param_line(&unresolved, "lake_project_id").unwrap() as usize, "{}", e);

        // an input is the variable the resources read: a static read is its value, a looked-up one is refused
        let input = |read: &str| format!("{}\ninputs {{\n  lake = \"${{{{interface.data-lake.{}}}}}\"\n}}\n", with(""), read);
        let fe = satz_core::pipeline::compile_estate("data_lake.satz", &input("project_id"), &resolver, &load).unwrap_or_else(|e| panic!("{}", e));
        let tfvars = crate::emitter::emit_tfvars(&fe.tfvars, &fe.inputs);
        assert!(tfvars.contains("lake = \"acme-data-lake-001\"\n"), "{}", tfvars);
        let e = satz_core::pipeline::compile_estate("data_lake.satz", &input("project_number"), &resolver, &load).err().expect("a looked-up input is refused");
        assert!(e.msg.contains("input `lake`") && e.msg.contains("looks up"), "{}", e.msg);
    }

    /// `init --from` writes an estate from the delegation's interface and those of what
    /// it works in, in the canonical layout, that transpiles as the delegation's account.
    #[test]
    fn the_teams_estate_runs_as_the_delegations_account_and_reads_every_interface() {
        let taken = published();
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let files = vec![(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone())];
        assert_eq!(default_name(&files).as_deref(), Some("data"));
        // a folder copied before the exports were renamed is named, with what to do
        let stale_text = base.1.replace("output \"estate_account\"", "output \"iac_account\"").replace("output \"infrastructure_project_id\"", "output \"deploy_project_id\"");
        let stale = satz_core::satz::parse(&stale_text).unwrap().interface_file.unwrap();
        let err = estate("data-lake", &[(base.0.clone(), stale), (project.0.clone(), project.2.clone())]).unwrap_err();
        assert!(err.contains("`delegation-data` carries the names before the rename") && err.contains("copy it again"), "{}", err);
        // a folder copied before the estate account's id was published is named too
        let no_id_text = base.1.replace("output \"estate_account_id\"", "output \"gone\"");
        let no_id = satz_core::satz::parse(&no_id_text).unwrap().interface_file.unwrap();
        let err = estate("data-lake", &[(base.0.clone(), no_id), (project.0.clone(), project.2.clone())]).unwrap_err();
        assert!(err.contains("`delegation-data` publishes no static `estate_account_id`") && err.contains("copy it again"), "{}", err);
        let estate = estate("data-lake", &files).unwrap();
        assert_eq!(satz_core::fmt::format(&estate).unwrap(), estate, "the estate is not in the canonical layout");
        for want in [
            "estate data_lake\n",
            "svc_iac_account    = \"${{interface.delegation-data.estate_account_id}}\"",
            "infra_project_name = \"${{interface.delegation-data.infrastructure_project_id}}\"",
            "infra_bucket_name  = \"${{interface.delegation-data.state_bucket}}\"",
            "prefix = \"data_lake/state\"",
            "use \"interfaces/incoming/delegation-data/delegation-data/satz/interface.satz\"",
            "use \"interfaces/incoming/data-lake/data-lake/satz/interface.satz\"",
        ] {
            assert!(estate.contains(want), "{} — {}", want, estate);
        }
        // transpiled through the tail: cloud mode impersonates the delegation's account
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        let load = |p: &str| {
            if p == base.0 {
                Ok(base.1.clone())
            } else if p == project.0 {
                Ok(project.1.clone())
            } else {
                Err(format!("no {}", p))
            }
        };
        let fe = satz_core::pipeline::compile_estate("data_lake.satz", &estate, &resolver, &load).unwrap();
        let cfg = crate::parse_tool_config(Path::new("/nonexistent/config.toml")).unwrap();
        let graph = crate::pack_graph::Shipped::Missing(std::path::PathBuf::from("presets/pack-graph.json"));
        let t = crate::compile_tail(&fe, &resolver, &reg, &cfg, &graph, "presets", "warn", Path::new("data_lake.satz"), "data_lake.satz", &estate);
        let errors: Vec<&str> = t.findings.iter().filter(|f| f.severity == crate::findings::Severity::Error).map(|f| f.message.as_str()).collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let providers = t.providers_tf.expect("providers.tf");
        assert!(providers.contains("impersonate_service_account = \"svc-deploy-data@acme-deploy-data.iam.gserviceaccount.com\""), "{}", providers);

        // a delegated resource alone carries no account; two delegations are one too many
        let alone = super::estate("data-lake", &[(project.0.clone(), project.2.clone())]).unwrap_err();
        assert!(alone.contains("carries a delegation's account") && alone.contains("`data-lake`") && alone.contains("presets/delegation.satz"), "{}", alone);
        let other = taken("delegation-payments");
        let two = super::estate("x", &[(base.0.clone(), base.2.clone()), (other.0.clone(), other.2.clone())]).unwrap_err();
        assert!(two.contains("an estate runs as one account"), "{}", two);
        // one account, several delegated resources: each is read by the name of its interface folder,
        // and the estate may pass on what it read
        let second = taken("data-mart");
        let both = super::estate("data", &[(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone()), (second.0.clone(), second.2.clone())]).unwrap();
        for want in ["project   = \"${{interface.data-lake.project_id}}\"", "project   = \"${{interface.data-mart.project_id}}\""] {
            assert!(both.contains(want), "{} — {}", want, both);
        }
        let load = |p: &str| [&base, &project, &second].iter().find(|t| t.0 == p).map(|t| t.1.clone()).ok_or_else(|| format!("no {}", p));
        let compile = |body: &str| {
            let src = format!("{}\n{}", both, body);
            let fe = satz_core::pipeline::compile_estate("data.satz", &src, &resolver, &load).unwrap_or_else(|e| panic!("{}", e));
            crate::compile_tail(&fe, &resolver, &reg, &cfg, &graph, "presets", "warn", Path::new("data.satz"), "data.satz", &src)
        };
        const SERVICE: &str = "google_project_service \"@L@\" {\n  project = \"${{interface.@R@}}\"\n  service = \"storage.googleapis.com\"\n}\n";
        let named = compile(&format!(
            "{}{}export \"lake_project\" = \"${{{{interface.data-lake.project_id}}}}\" description \"The lake's project\"\n",
            SERVICE.replace("@L@", "lake").replace("@R@", "data-lake.project_id"),
            SERVICE.replace("@L@", "mart").replace("@R@", "data-mart.project_id")
        ));
        let errors: Vec<&str> = named.findings.iter().filter(|f| f.severity == crate::findings::Severity::Error).map(|f| f.message.as_str()).collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let out = named.out.expect("emitted");
        assert!(out.main_tf.contains("local.interface.data_lake__project_id") && out.main_tf.contains("local.interface.data_mart__project_id"), "{}", out.main_tf);
        assert!(out.interface_tf.contains("data_lake__project_id") && out.interface_tf.contains("\"acme-data-mart-001\""), "{}", out.interface_tf);
        let published = named.interface.expect("the estate publishes the lake's project");
        assert!(format!("{:?}", published).contains("acme-data-lake-001"), "{:?}", published);
        // by its bare name the project is two different ones, and the message names both
        let bare = compile(&SERVICE.replace("@L@", "lake").replace("@R@", "project_id"));
        let said: Vec<&str> = bare.findings.iter().filter(|f| f.severity == crate::findings::Severity::Error).map(|f| f.message.as_str()).collect();
        assert!(
            said.iter().any(|m| m.contains("they differ") && m.contains("${{interface.data-lake.project_id}}") && m.contains("${{interface.data-mart.project_id}}")),
            "{:?}",
            said
        );
        let unknown = compile(&SERVICE.replace("@L@", "lake").replace("@R@", "data-pond.project_id"));
        assert!(unknown.findings.iter().any(|f| f.message.contains("no used interface folder is named `data-pond`") && f.message.contains("`data-lake`")));
        let no_region_text = base.1.replace("output \"default_region\"", "output \"region_gone\"");
        let no_region = satz_core::satz::parse(&no_region_text).unwrap().interface_file.unwrap();
        assert!(super::estate("data", &[(base.0.clone(), no_region)]).unwrap_err().contains("estate-core"));
    }
}
