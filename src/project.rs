//! A team's estate on satz, end to end. An entry of `delegations` in
//! `presets/delegation.satz` gives the central estate the team's identity — an
//! infrastructure project, the account, its state bucket — and the interface
//! `delegation-<name>` that publishes them; a target preset (`presets/delegated-folders.satz`,
//! `presets/delegated-projects.satz`) publishes what that account works in, one interface per
//! entry. `satz init --from` takes those hand-over folders and writes the team's own estate
//! from them.

use satz_core::satz::InterfaceFile;

/// The exports `presets/delegation.satz` publishes for a delegation and `init --from` reads.
const DEPLOY_PROJECT: &str = "deploy_project_id";
const IAC_ACCOUNT: &str = "iac_account";
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

/// Whether the interface carries a delegation's identity: the three exports of the base.
fn is_base(file: &InterfaceFile) -> bool {
    [DEPLOY_PROJECT, IAC_ACCOUNT, STATE_BUCKET].iter().all(|e| static_output(file, e).is_some())
}

/// The name an estate written from these interfaces takes when none is given: the
/// delegation's, without the prefix of its interface.
pub(crate) fn default_name(files: &[(String, InterfaceFile)]) -> Option<String> {
    let base = files.iter().find(|(_, f)| is_base(f))?;
    Some(base.1.name.strip_prefix(BASE_PREFIX).unwrap_or(&base.1.name).to_string())
}

/// A workload estate, from the interface folders it was given: `files` is each folder's
/// interface with the path the estate `use`s. Exactly one of them is a delegation's own —
/// the identity: the estate runs as its account, in its infrastructure project, with its
/// state in its bucket under the estate's name, so several estates of one delegation share
/// the bucket. Every other is a place that account works in, any number of them: a value is
/// read by the name of its folder. Refused when no interface, or more than one, carries an
/// identity, or when none carries the region.
pub(crate) fn estate(name: &str, files: &[(String, InterfaceFile)]) -> Result<String, String> {
    valid_name(name)?;
    let listed = || files.iter().map(|(_, f)| format!("`{}`", f.name)).collect::<Vec<_>>().join(", ");
    let bases: Vec<&InterfaceFile> = files.iter().map(|(_, f)| f).filter(|f| is_base(f)).collect();
    let base = match bases.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "none of the interfaces {} carries a delegation's identity — the static exports `{}`, `{}` and `{}`, which an entry of `delegations` in `presets/delegation.satz` publishes as the interface `{}<name>`: take that folder too",
                listed(),
                DEPLOY_PROJECT,
                IAC_ACCOUNT,
                STATE_BUCKET,
                BASE_PREFIX
            ));
        }
        many => {
            return Err(format!(
                "{} of the interfaces {} carry a delegation's identity ({}): an estate runs as one account — give the folders of one delegation",
                many.len(),
                listed(),
                many.iter().map(|f| format!("`{}`", f.name)).collect::<Vec<_>>().join(", ")
            ));
        }
    };
    let read = |export: &str| static_output(base, export).expect("a base carries the three exports");
    let region = files.iter().find_map(|(_, f)| static_output(f, REGION)).ok_or_else(|| {
        format!(
            "none of the interfaces {} publishes a static `{}` — `presets/estate-core.satz` exports it: switch its `use` line on in the central estate",
            listed(),
            REGION
        )
    })?;
    let account = read(IAC_ACCOUNT);
    let account_id = account.split('@').next().unwrap_or(&account).to_string();
    let uses: String = files.iter().map(|(path, _)| format!("use \"{}\"\n", path)).collect();
    let reads: String = files
        .iter()
        .map(|(_, f)| f)
        .filter(|f| !is_base(f))
        .map(|f| {
            if f.outputs.iter().any(|o| o.name == crate::prerequisites::DELEGATED_FOLDER) {
                format!("//   under the folder `{0}`:  folder_id = \"${{{{interface.{0}.{1}}}}}\"\n", f.name, crate::prerequisites::DELEGATED_FOLDER)
            } else {
                format!("//   in the project `{0}`:    project   = \"${{{{interface.{0}.project_id}}}}\"\n", f.name)
            }
        })
        .collect();
    const TEXT: &str = r#"// Generated by `satz init --from` — the estate `@NAME@` of the delegation `@BASE@`: it
// reads the central estate `@CENTRAL@` through the interface files below and runs as the
// delegation's own account. Every value below is a param: change it here.

estate @LABEL@

params {
  infra_project_name = "@PROJECT@" // the project this estate's calls are billed to, and its state is kept in
  infra_bucket_name  = "@BUCKET@"
  svc_iac_account    = "@ACCOUNT@"
  default_region     = "@REGION@"
  default_zone       = "@REGION@-a"
  deployment_engine  = "tofu"
  deployment_mode    = "cloud"
}

// What the organisation's estate delegated, one interface folder each. A value is read by
// the folder's name, `${{interface.<folder>.<value>}}`; the README beside each file lists
// the values and what may be attached to, and `CHANGES.md` there, after a change of the
// organisation's estate, what to do about it.
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
        .replace("@PROJECT@", &read(DEPLOY_PROJECT))
        .replace("@BUCKET@", &read(STATE_BUCKET))
        .replace("@ACCOUNT@", &account_id)
        .replace("@REGION@", &region)
        .replace("@USES@", &uses)
        .replace("@READS@", &reads))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The estate `satz init` writes for the example organisation, with the delegation
    /// packs and the entries given.
    fn central(tag: &str, folder: Option<&str>, delegations: &str, projects: &str) -> String {
        let dir = std::env::temp_dir().join(format!("satz-project-{}-{}-{}", std::process::id(), tag, folder.is_some()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("C0example.satz");
        let mut args = crate::template::tests::args("first.admin", "example.com");
        args.workload_folder_name = folder.map(str::to_string);
        crate::template::generate_template(&args, None, &path).unwrap();
        let src = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let parent = if folder.is_some() { "  delegated_projects_folder = \"google_folder.workload_folder.name\"\n" } else { "" };
        let src = src.replacen("params {\n", &format!("params {{\n  delegations = [{}]\n  delegated_projects = [{}]\n{}", delegations, projects, parent), 1);
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

    const DELEGATIONS: &str = "{ name = \"payments\" owner_group = \"payments-owners@example.com\" }, { name = \"data\" owner_group = \"lake-owners@example.com\" }";
    const PROJECTS: &str = "{ name = \"payments\" delegation = \"payments\" }, { name = \"data-lake\" delegation = \"data\" }, { name = \"data-mart\" delegation = \"data\" }";

    /// An owner group that is no address, and a name that is no interface name, are
    /// refused at compile — naming the entry and the value — rather than at apply.
    #[test]
    fn a_malformed_entry_is_refused_at_compile() {
        let reg = crate::corpus::registry();
        let resolver = crate::EstateResolver { registry: &reg };
        for (entry, says) in [
            ("{ name = \"payments\" owner_group = \"payments-owners\" }", "entry `payments` has `owner_group` = \"payments-owners\""),
            ("{ name = \"payments\" owner_group = \"group:payments-owners@example.com\" }", "entry `payments` has `owner_group` = \"group:payments-owners@example.com\""),
            ("{ name = \"payments\" owner_group = \"payments-owners@example.com \" }", "entry `payments` has `owner_group`"),
            ("{ name = \"Payments\" owner_group = \"payments-owners@example.com\" }", "entry `Payments`"),
        ] {
            let src = central(&format!("bad-{}", says.len()), None, entry, "");
            let e = satz_core::pipeline::compile_estate("e.satz", &src, &resolver, &presets).err().unwrap_or_else(|| panic!("{} compiled", entry));
            assert!(e.msg.contains(says), "{}: {}", entry, e.msg);
        }
    }

    /// A delegation is an identity with its own interface; a delegated project is a
    /// Google project under the organisation or the workload folder with the delegation's
    /// account on it, and its own interface. One account works in several projects, and a
    /// project may carry its delegation's name.
    #[test]
    fn a_delegation_is_an_identity_and_each_project_names_the_one_that_works_in_it() {
        for folder in [None, Some("Workloads")] {
            let (manifest, i) = compile(&central("pack", folder, DELEGATIONS, PROJECTS), &presets);
            let address = |a: &str| manifest.resources.values().find(|r| r.address() == a).unwrap_or_else(|| panic!("no {}", a));
            // the delegation's project and the delegated project of the same name stand apart
            address("google_project.deploy_payments");
            let project = address("google_project.data_lake");
            match folder {
                None => assert_eq!(project.attrs.get("org_id").map(String::as_str), Some("123456789012"), "{:?}", project.attrs),
                Some(_) => assert_eq!(project.refs.get("folder_id").map(String::as_str), Some("google_folder.workload_folder.name"), "{:?}", project.refs),
            }
            for (name, exports) in [
                ("delegation-payments", vec![("deploy_project_id", true), ("iac_account", true), ("state_bucket", true), ("account_group", true)]),
                ("payments", vec![("project_id", true), ("project_number", false)]),
                ("data-lake", vec![("project_id", true), ("project_number", false)]),
            ] {
                assert!(i.projects().contains(&name.to_string()), "{} — {:?}", name, i.projects());
                let outputs: Vec<(&str, bool)> = i.module_outputs(name).iter().map(|o| (o.name.as_str(), o.static_text().is_some())).collect();
                for want in exports {
                    assert!(outputs.contains(&want), "{}: {:?} — {:?}", name, want, outputs);
                }
            }
            assert!(!i.module_outputs("payments").iter().any(|o| o.name == "iac_account"), "a project carries an account of its own");
            assert_eq!(i.module_outputs("payments").into_iter().find(|o| o.name == "project_id").unwrap().attach, ["google_project_iam_member"]);
            assert_eq!(
                i.module_outputs("delegation-data").into_iter().find(|o| o.name == "iac_account").unwrap().static_text().as_deref(),
                Some("svc-deploy-data@acme-deploy-data.iam.gserviceaccount.com")
            );
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
        let (manifest, i) = compile(&central("none", None, "", ""), &presets);
        assert!(!manifest.resources.values().any(|r| r.tf_type == "google_project" && r.address().contains("deploy_")));
        assert!(i.projects().is_empty(), "{:?}", i.projects());
    }

    /// `init --from` writes an estate from the delegation's interface and those of what
    /// it works in, in the canonical layout, that compiles as the delegation's account.
    #[test]
    fn the_teams_estate_runs_as_the_delegations_account_and_reads_every_interface() {
        // the interfaces the central estate published, as the files a team takes; the
        // region is estate-core's export, which the estate `init` writes switches on later
        let src = format!("{}\nexport \"default_region\" = default_region\n", central("estate", None, DELEGATIONS, PROJECTS));
        let (manifest, i) = compile(&src, &presets);
        let facts = crate::consumer::Facts::of_estate("e", Some(&i), &manifest);
        let taken = |name: &str| {
            let text = satz_core::fmt::format(&i.satz_file(name, &facts, "0.0.0", "e.satz")).unwrap();
            let file = satz_core::satz::parse(&text).unwrap().interface_file.unwrap();
            (format!("interfaces/incoming/{0}/{0}/satz/interface.satz", name), text, file)
        };
        let (base, project) = (taken("delegation-data"), taken("data-lake"));
        let files = vec![(base.0.clone(), base.2.clone()), (project.0.clone(), project.2.clone())];
        assert_eq!(default_name(&files).as_deref(), Some("data"));
        let estate = estate("data-lake", &files).unwrap();
        assert_eq!(satz_core::fmt::format(&estate).unwrap(), estate, "the estate is not in the canonical layout");
        for want in [
            "estate data_lake\n",
            "svc_iac_account    = \"svc-deploy-data\"",
            "infra_project_name = \"acme-deploy-data\"",
            "infra_bucket_name  = \"acme-deploy-data-state\"",
            "prefix = \"data_lake/state\"",
            "use \"interfaces/incoming/delegation-data/delegation-data/satz/interface.satz\"",
            "use \"interfaces/incoming/data-lake/data-lake/satz/interface.satz\"",
        ] {
            assert!(estate.contains(want), "{} — {}", want, estate);
        }
        // compiled through the tail: cloud mode impersonates the delegation's account
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

        // a target alone carries no identity; two delegations are one too many
        let alone = super::estate("data-lake", &[(project.0.clone(), project.2.clone())]).unwrap_err();
        assert!(alone.contains("carries a delegation's identity") && alone.contains("`data-lake`") && alone.contains("presets/delegation.satz"), "{}", alone);
        let other = taken("delegation-payments");
        let two = super::estate("x", &[(base.0.clone(), base.2.clone()), (other.0.clone(), other.2.clone())]).unwrap_err();
        assert!(two.contains("an estate runs as one account"), "{}", two);
        // one account, several places: each is read by the name of its interface folder,
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
