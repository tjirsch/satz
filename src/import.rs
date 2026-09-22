//! `satz import` — an existing organisation, an HCL tree or a Terraform state file,
//! read once and written out as Satz.
//!
//! The discovery half lives in `src/discovery.rs`: this module is what the command
//! does with what discovery finds — where the file lands, which parent it hangs
//! under, which billing account and quota project the estate resolves to, and the
//! delta a second run reports against what is already written.

use std::path::{Path, PathBuf};

use crate::config::{Config, ImportConfig};
use crate::fsx;
use crate::schema::ResourceRegistry;
use crate::{pipeline_b_generate, reject_yaml_dialect};
use crate::settings::{ToolConfig};

/// The state shape of `satz import`: `tofu show -json` (a file, or run now).
#[allow(clippy::too_many_arguments)]
pub(crate) fn import_state(
    state_json: Option<PathBuf>,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    customer_shortname: Option<&str>,
    verbose: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let enabled_types = Some(cfg.resource_types.into_iter().filter(|(_, v)| v.import).map(|(k, _)| k).collect());
    println!("Reading infrastructure state...");
    let state_val: serde_json::Value = if let Some(path) = state_json {
        let content = fsx::read_to_string(&path)?;
        serde_json::from_str(&content)?
    } else {
        // Every other spawn sets this. Without it `satz import <state>` reads
        // the state of whatever directory the operator happens to stand in —
        // which is usually none, and the error names neither.
        let out = std::process::Command::new(&tool_config.tf_tool)
            .current_dir(&runtime_config.hcl_dir)
            .arg("show")
            .arg("-json")
            .output()?;
        if !out.status.success() {
            return Err(format!("Failed to run {} show -json: {}", tool_config.tf_tool, String::from_utf8_lossy(&out.stderr)).into());
        }
        serde_json::from_slice(&out.stdout)?
    };
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let discoverer = crate::discovery::Discoverer::new(state_val, Some(registry), enabled_types, filtered, on_collision);
    let mut found = discoverer.discover()?;
    let registry = discoverer.registry.as_ref().ok_or("the registry loaded above is gone")?;
    // the state shape has no ADC: what the data carries, and the flag
    let org = organization_of(&found.config, None);
    let vocab = crate::vocabulary::Vocabulary::infer(&found.config, org.as_deref(), None, customer_shortname);
    vocab.apply_billing(&mut found.config);
    write_imported(&found.config, output, None, registry, &vocab, runtime_config)?;
    crate::discovery::report_skipped(&found, &discoverer.filtered_types, verbose);
    if verbose {
        crate::discovery::Discoverer::print_summary(&found.config);
    }
    Ok(())
}

/// The live shape of `satz import`: one Cloud Asset Inventory sweep under
/// `parent` (`organizations/<n>`, `folders/<n>` or `projects/<id>`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn import_org(
    parent: &str,
    output: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    customer_shortname: Option<&str>,
    verbose: bool,
    generate_unmapped: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("import: root {}", parent);
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let org_hint = cfg.root.as_ref().and_then(|r| r.organization.clone())
        .or_else(|| parent.strip_prefix("organizations/").map(String::from));
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(&registry), on_collision).await?;
    // A folder/project root names no organization; the assets' ancestors do.
    let org_hint = org_hint.or(found.organization.clone());
    attach_billing_accounts(&mut found.config).await;
    // what the ADC states, the way `init` reads it — the sweep's
    // organization is the hint, so an identity that sees many is no ambiguity
    let live = crate::gcp::identity::live_defaults(true, true, org_hint.as_deref()).await?;
    let facts = crate::vocabulary::LiveFacts {
        customer_id: live.customer_id.clone(),
        customer_domain: Some(live.org_display_name.clone().unwrap_or_else(|| live.customer_domain.clone())),
        first_admin: Some(live.first_admin.clone()),
        billing_account: live.billing_account.clone(),
    };
    let org = organization_of(&found.config, org_hint.as_deref());
    let vocab = crate::vocabulary::Vocabulary::infer(&found.config, org.as_deref(), Some(&facts), customer_shortname);
    vocab.apply_billing(&mut found.config);
    let written = write_imported(&found.config, output, org_hint.as_deref(), &registry, &vocab, runtime_config)?;
    crate::discovery::report_skipped(&found, &filtered, verbose);
    if generate_unmapped {
        generate_unmapped_config(&found.skipped, &registry, &written, verbose, tool_config, runtime_config)?;
    }
    Ok(())
}

/// `--generate-unmapped`: hand the resources this sweep could not map to the
/// provider, and read back what it writes.
///
/// The estate above is already on disk — this adds a SECOND file beside it,
/// `<estate>-generated.satz`, because what the provider writes has a different
/// provenance from what satz translated and the operator merges it deliberately.
/// The scratch directory stays: with a refused import id it is what the operator
/// edits, and the two commands it names finish the job by hand.
///
/// It runs as the identity the sweep ran as — the human's Application Default
/// Credentials, which `import` without `--into` binds nothing over (`IDENTITIES`,
/// `src/main.rs`): the child inherits the environment, and the provider block
/// written below impersonates nobody.
fn generate_unmapped_config(
    skipped: &[crate::discovery::Skipped],
    registry: &ResourceRegistry,
    estate: &Path,
    verbose: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::generate_config;
    let plan = generate_config::plan(skipped, &|t| registry.resources.contains_key(t));
    println!(
        "\ngenerate-unmapped: {} unmapped resource(s) the provider can be asked for, {} it cannot:",
        plan.candidates.len(),
        plan.refused.len()
    );
    for (what, why) in &plan.refused {
        println!("  not generated  {} — {}", what, why);
    }
    if plan.candidates.is_empty() {
        println!("  nothing to generate — no child process ran and nothing was written.");
        return Ok(());
    }
    if verbose {
        for c in &plan.candidates {
            println!("  generating     {}.{} = {}", c.tf_type, c.label, c.import_id);
        }
    }
    let providers = providers_for(&plan.candidates, registry, tool_config);
    let stem = estate.file_stem().and_then(|s| s.to_str()).unwrap_or("discovered").to_string();
    let work_dir = estate.with_file_name(format!("{}-generate", stem));
    let generated = generate_config::generate(&work_dir, &plan.candidates, &providers, &mut generate_config::Tofu { tool: &tool_config.tf_tool })
        .map_err(|e| {
            format!(
                "{}\nThe import blocks are in {} — correct the ids the provider refused, then \
                 `{} plan -generate-config-out={}` there and `satz import {}` on what it writes.",
                e,
                work_dir.join(generate_config::IMPORTS_TF).display(),
                tool_config.tf_tool,
                generate_config::GENERATED_TF,
                work_dir.join(generate_config::GENERATED_TF).display(),
            )
        })?;
    println!("\ngenerate-unmapped: {} — reading it back as Satz", generated.display());
    let src = generated.to_str().ok_or_else(|| format!("{}: the generated path is not UTF-8", generated.display()))?;
    import_hcl(src, PathBuf::from(format!("{}-generated.satz", stem)), false, verbose, runtime_config)
}

/// The providers the candidates' types come from, at the versions the tool
/// config pins — so the scratch directory reads the same provider the estate is
/// compiled against.
fn providers_for(
    candidates: &[crate::generate_config::Candidate],
    registry: &ResourceRegistry,
    tool_config: &ToolConfig,
) -> Vec<crate::generate_config::Provider> {
    let pinned = tool_config.parsed_providers();
    let names: std::collections::BTreeSet<String> = candidates
        .iter()
        .filter_map(|c| registry.resources.get(&c.tf_type))
        .map(|(provider, _)| provider.rsplit('/').next().unwrap_or(provider).to_string())
        .collect();
    names
        .into_iter()
        .map(|name| {
            let version = pinned
                .iter()
                .find(|(p, _)| crate::schema::derive_registry_source(p).0 == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| tool_config.provider_version.clone());
            let (short, source) = crate::schema::derive_registry_source(&name);
            crate::generate_config::Provider { name: short.to_string(), source, version }
        })
        .collect()
}

pub(crate) fn write_imported(
    config: &Config,
    output: PathBuf,
    org_hint: Option<&str>,
    registry: &ResourceRegistry,
    vocab: &crate::vocabulary::Vocabulary,
    runtime_config: &ToolConfig,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    for line in vocab.report() {
        println!("{}", line);
    }
    let text = discovered_to_satz(config, "discovered", org_hint, registry, vocab)?;
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write_generated_satz(&final_output, &text)?;
    println!("Wrote {} — review it, then `satz transpile` and `tofu plan`.", final_output.display());
    Ok(final_output)
}

pub(crate) fn missing_import_config(presets_dir: &str) -> Box<dyn std::error::Error> {
    format!(
        "import configuration not found. Provide --import-config, or run `satz get-presets` so that '{}/import-config.yaml' exists, or set import_config in config.toml.",
        presets_dir
    )
    .into()
}

/// Which shape a source is, from its form alone. `--from` overrides.
pub(crate) fn detect_import_shape(source: Option<&str>, root: Option<&crate::config::ImportRoot>) -> Result<String, Box<dyn std::error::Error>> {
    let Some(src) = source else {
        return if root.is_some_and(|r| r.organization.is_some() || r.folder.is_some() || r.project.is_some()) {
            Ok("org".into())
        } else {
            Err("nothing to import: give a source (a state file, organizations/<n>, folders/<n>, projects/<id>, a directory of .tf files) or set `root` in the import config".into())
        };
    };
    if src == "-" {
        return Ok("state".into());
    }
    if src.starts_with("organizations/") || src.starts_with("folders/") || src.starts_with("projects/") {
        return Ok("org".into());
    }
    let path = Path::new(src);
    if path.is_dir() {
        return Ok("hcl".into());
    }
    match path.extension().and_then(|e| e.to_str()) {
        // recognised only so the refusal names what the file is
        Some("yaml") | Some("yml") => Ok("yaml".into()),
        Some("tf") => Ok("hcl".into()),
        Some("json") | Some("tfstate") => Ok("state".into()),
        _ => Err(format!("cannot tell what {:?} is — pass --from state|org|hcl", src).into()),
    }
}

/// The CAI scope to import from: the command line's source when given, else
/// the import config's `root` (project > folder > organization). A folder by
/// `path` is resolved live, one segment at a time, never guessed.
pub(crate) async fn resolve_import_parent(
    source: Option<&str>,
    root: Option<&crate::config::ImportRoot>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(s) = source {
        return Ok(s.to_string());
    }
    let root = root.ok_or("no live root: pass organizations/<n>, folders/<n> or projects/<id>, or set `root` in the import config")?;
    if let Some(p) = &root.project {
        return Ok(format!("projects/{}", p.trim_start_matches("projects/")));
    }
    if let Some(f) = &root.folder {
        return match (&f.id, &f.path) {
            (Some(id), None) => Ok(format!("folders/{}", id.trim_start_matches("folders/"))),
            (None, Some(path)) => {
                let org = root.organization.as_deref().ok_or("root.folder.path needs root.organization to start from")?;
                let token = crate::gcp::access_token().await?;
                let http = reqwest::Client::new();
                let name = crate::gcp::resourcemanager::resolve_folder_path(&http, &token, org, path).await?;
                println!("import: folder path {:?} is {}", path, name);
                Ok(name)
            }
            _ => Err("root.folder needs exactly one of `id` or `path`".into()),
        };
    }
    let org = root.organization.as_deref().ok_or("root has neither organization, folder nor project")?;
    Ok(format!("organizations/{}", org.trim_start_matches("organizations/")))
}

pub(crate) fn satz_output_path(yaml_dir: &str, output: PathBuf) -> PathBuf {
    let output = if output.extension().and_then(|e| e.to_str()) == Some("yaml") {
        output.with_extension("satz")
    } else {
        output
    };
    if output.is_absolute() {
        output
    } else {
        PathBuf::from(yaml_dir).join(output)
    }
}

/// A discovered `Config` as a Satz estate that compiles as-is: the local
/// backend the emitter requires, `customer_organization_id` inferred from the
/// resources (every `organizations/<n>` reference or `org_id` names it) and
/// referenced wherever the number was written, the document shaped into the
/// language's own forms (`satz_core::condense`), printed by the same printer
/// the yaml import uses, and shorthand type keys (`folder`, `project`)
/// normalised to provider names.
///
/// Discovery emits plain data — no anchors, tags, includes or nulls — so no
/// dialect pre-pass is needed; that is what makes the direct route possible.
pub(crate) fn discovered_to_satz(
    config: &Config,
    name: &str,
    org_hint: Option<&str>,
    registry: &ResourceRegistry,
    vocab: &crate::vocabulary::Vocabulary,
) -> Result<String, Box<dyn std::error::Error>> {
    let is_type = |t: &str| registry.resources.contains_key(t);
    let mut top = match serde_yaml::to_value(config)? {
        serde_yaml::Value::Mapping(m) => m,
        other => return Err(format!("discovered config is not a mapping: {:?}", other).into()),
    };
    if !top.contains_key(serde_yaml::Value::String("terraform".into())) {
        // the scaffold `init` writes: the local backend for day 0 and, once
        // the state bucket is known, the gcs one `deployment_mode` switches to
        let mut backend: serde_yaml::Value =
            serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n")?;
        if vocab.bindings.iter().any(|b| b.name == "infra_bucket_name") {
            let mut gcs = serde_yaml::Mapping::new();
            gcs.insert("bucket".into(), satz_core::migrate::param_ref("infra_bucket_name"));
            gcs.insert("prefix".into(), serde_yaml::Value::String("hcl/state".into()));
            if let Some(b) = backend.get_mut("backend").and_then(|b| b.as_mapping_mut()) {
                b.insert("gcs".into(), serde_yaml::Value::Mapping(gcs));
            }
        }
        top.insert(serde_yaml::Value::String("terraform".into()), backend);
    }
    // Root-scoped resources (org IAM, org policies, groups) use the root
    // provider alias, which an estate declares in `providers { … }`; without
    // it `tofu plan` says "Provider configuration not present" (live-run F10).
    if !top.contains_key(serde_yaml::Value::String("providers".into())) {
        // Org-scoped APIs (Org Policy, Cloud Identity) need a quota project
        // that enables them, the way the Day-0 template uses the infra
        // project. Review it.
        let billing = quota_project(config);
        let quota = billing
            .as_deref()
            .map(|p| format!("  project: {p}\n  billing_project: {p}\n"))
            .unwrap_or_default();
        let providers: serde_yaml::Value = serde_yaml::from_str(&format!(
            "google:\n  alias: google\n  user_project_override: true\n{quota}google-beta:\n  alias: google-beta\n  user_project_override: true\n{quota}"
        ))?;
        top.insert(serde_yaml::Value::String("providers".into()), providers);
        if billing.is_none() {
            eprintln!("warning: no project among the imported resources — set `billing_project` in the estate's `providers` block by hand (org-scoped APIs need a quota project)");
        }
    }
    let org = organization_of(config, org_hint);
    let mut params = vocab.params();
    if org.is_none() {
        eprintln!("warning: no organization id found among the discovered resources — add `customer_organization_id` to `params` by hand");
    } else if !params.iter().any(|(n, _)| n == satz_core::condense::ORG_PARAM) {
        params.insert(0, (satz_core::condense::ORG_PARAM.to_string(), format!("\"{}\"", org.clone().unwrap_or_default())));
    }
    condense_document(&mut top, org.as_deref(), &vocab.substitutions(), registry);
    let header = vec![
        "Discovered estate — review before use: the hierarchy is as found, folders are labelled".to_string(),
        "by display name, every resource carries its \"import-id\", what the platform owns was".to_string(),
        "skipped and listed, and the params are bound from what the platform states — a value".to_string(),
        "marked `// inferred:` names the rule that chose it. `satz transpile`, then `tofu plan`".to_string(),
        "should show imports and no creates.".to_string(),
    ];
    let satz = satz_core::migrate::convert_value(&top, "estate", name, &params, &header)?;
    Ok(satz_core::migrate::normalize_type_keys(&satz, &is_type))
}

/// The organization a discovered estate belongs to: named by its resources
/// (every `organizations/<n>` reference or `org_id`), else the sweep's hint.
pub(crate) fn organization_of(config: &Config, org_hint: Option<&str>) -> Option<String> {
    let top = serde_yaml::to_value(config).ok()?;
    infer_org_id(&top).or_else(|| org_hint.map(String::from))
}

/// The shaping pass over a discovered document, answered from the provider
/// schema: which document keys are resource types (shorthand included), and
/// which nested blocks occur once.
pub(crate) fn condense_document(
    top: &mut serde_yaml::Mapping,
    organization: Option<&str>,
    substitutions: &[satz_core::condense::Substitution],
    registry: &ResourceRegistry,
) {
    let type_of = |k: &str| -> Option<String> {
        if registry.resources.contains_key(k) {
            return Some(k.to_string());
        }
        let prefixed = format!("google_{}", k);
        registry.resources.contains_key(&prefixed).then_some(prefixed)
    };
    let single_block = |tf_type: &str, path: &str| registry.single_block(tf_type, path);
    satz_core::condense::condense(top, &satz_core::condense::Shaping { type_of: &type_of, single_block: &single_block, organization, substitutions });
}

/// `satz map-types`: align every selected row's API schema against the
/// provider schema and write `type-map.yaml` beside the import config.
pub(crate) async fn map_types(cfg: ImportConfig, only: Vec<String>, verbose: bool, runtime_config: &ToolConfig) -> Result<(), Box<dyn std::error::Error>> {
    use crate::gcp::discovery_doc as dd;
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let cache = dd::cache_dir(&runtime_config.presets_dir);
    let http = reqwest::Client::new();
    let mut rows: Vec<(&String, &crate::config::ImportResourceConfig)> = cfg
        .resource_types
        .iter()
        .filter(|(t, r)| if only.is_empty() { r.import } else { only.iter().any(|o| crate::config::glob_match(o, t)) })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    if rows.is_empty() {
        return Err("no rows selected — `import: true` rows, or --only <types>".into());
    }
    let out_path = Path::new(&runtime_config.presets_dir).join("type-map.yaml");
    let mut existing: std::collections::BTreeMap<String, crate::align::TypeMap> = match fsx::read_to_string(&out_path) {
        Ok(t) => serde_yaml::from_str(&t)?,
        // no map yet: the first run writes it
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(format!("{}: {}", out_path.display(), e).into()),
    };
    let (mut mapped, mut skipped) = (0usize, Vec::new());
    for (t, row) in rows {
        if row.content_type.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("IAM_POLICY")) {
            continue; // an IAM binding has no resource schema of its own — its asset is the parent's policy
        }
        let Some(asset_type) = row.asset_type.as_deref().filter(|a| !a.starts_with("TODO")) else {
            skipped.push(format!("{}: no asset_type", t));
            continue;
        };
        let Some((service, type_name)) = dd::split_asset_type(asset_type) else {
            skipped.push(format!("{}: asset_type {} is not <service>.googleapis.com/<Type>", t, asset_type));
            continue;
        };
        let Some((_, tf)) = registry.find_resource(t) else {
            skipped.push(format!("{}: not in the provider schema", t));
            continue;
        };
        let doc = match dd::document(&http, &cache, &service).await {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{}: {}", t, e));
                continue;
            }
        };
        let schema = match row.api_schema.as_deref() {
            Some(id) => doc.get("schemas").and_then(|s| s.get(id)).map(|s| (id, s)).ok_or_else(|| format!("{}: api_schema `{}` not in the {} document", t, id, service)),
            None => dd::schema_for(&doc, &type_name).map_err(|e| format!("{}: {}", t, e)),
        };
        let (schema_id, schema) = match schema {
            Ok(x) => x,
            Err(e) => {
                skipped.push(e);
                continue;
            }
        };
        let schemas = doc.get("schemas").cloned().unwrap_or(serde_json::Value::Null);
        let tm = crate::align::align(schema, &schemas, &tf.block)?;
        let revision = doc.get("revision").and_then(|r| r.as_str()).unwrap_or("?");
        println!(
            "{:48} {}/{} rev {}: {} exact, {} mapped ({} renamed), {} API-only, {} TF-only",
            t,
            service,
            schema_id,
            revision,
            tm.exact,
            tm.map.len(),
            tm.how.values().filter(|h| *h == "renamed").count(),
            tm.unmatched.len(),
            tm.tf_only.len()
        );
        if verbose {
            for (src, dst) in &tm.map {
                println!("    {:10} {} → {}", tm.how.get(src).map(String::as_str).unwrap_or(""), src, dst);
            }
            for u in &tm.unmatched {
                println!("    API-only   {}", u);
            }
        }
        existing.insert(t.clone(), tm);
        mapped += 1;
    }
    let body = serde_yaml::to_string(&existing)?;
    let header = format!(
        "# Generated by `satz map-types` — the API→Terraform field map per resource type,\n# aligned from the API's Discovery Document and the provider schema in {}.\n# Review rows marked `renamed`; `unmatched` API fields are dropped at import.\n# Re-run after a provider bump. Do not edit by hand: put overrides in import-config.yaml.\n",
        runtime_config.schema_dir
    );
    fsx::write(&out_path, format!("{}{}", header, body))?;
    println!("\nmap-types: {} type(s) mapped → {}", mapped, out_path.display());
    for s in &skipped {
        println!("  skipped: {}", s);
    }
    Ok(())
}

/// The hcl shape: literal resource blocks become Satz resources, the rest is
/// carried verbatim inside `hcl trust` (`--wrap-all`: everything), one estate.
pub(crate) fn import_hcl(src: &str, output: PathBuf, wrap_all: bool, verbose: bool, runtime_config: &ToolConfig) -> Result<(), Box<dyn std::error::Error>> {
    let src_path = Path::new(src);
    let mut files: Vec<PathBuf> = if src_path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(src_path)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("tf"))
            .collect();
        v.sort();
        v
    } else {
        vec![src_path.to_path_buf()]
    };
    if files.is_empty() {
        return Err(format!("{}: no .tf files", src).into());
    }
    files.retain(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.ends_with(".tfstate")));
    let inputs: Vec<satz_hcl::Input> = files
        .iter()
        .map(|f| Ok(satz_hcl::Input { path: f.to_string_lossy().into_owned(), text: fsx::read_to_string(f)? }))
        .collect::<Result<_, std::io::Error>>()?;
    let name = src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("imported_hcl");
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let imported = satz_hcl::import(&inputs, name, wrap_all, &RegistrySchema(Some(&registry)))?;
    let final_output = satz_output_path(&runtime_config.yaml_dir, output);
    if let Some(parent) = final_output.parent() {
        fsx::create_dir_all(parent)?;
    }
    fsx::write_generated_satz(&final_output, &imported.satz)?;
    println!("Wrote {} — review it, then `satz transpile` and `tofu plan` against the source's state: no changes.", final_output.display());
    println!("{}", satz_hcl::summary(&imported.rows));
    for r in &imported.rows {
        match &r.action {
            satz_hcl::Action::Dropped(why) => println!("  dropped    {}:{} {} — {}", r.file, r.line, r.what, why),
            satz_hcl::Action::Expanded(n) => {
                println!("  expanded   {}:{} {} — `count` over a promoted list: {} resource(s)", r.file, r.line, r.what, n)
            }
            satz_hcl::Action::Wrapped(why) if !wrap_all || verbose => println!("  wrapped    {}:{} {} — {}", r.file, r.line, r.what, why),
            satz_hcl::Action::Promoted(what) => println!("  promoted   {}:{} {} — {}", r.file, r.line, r.what, what),
            satz_hcl::Action::Translated if verbose => println!("  translated {}:{} {}", r.file, r.line, r.what),
            _ => {}
        }
    }
    for n in &imported.notes {
        println!("  note       {}", n);
    }
    Ok(())
}

/// The provider schema, as the HCL importer asks about it. With no schemas on
/// disk nothing is a known type, so every block wraps — loudly, in the report,
/// rather than by mistranslation.
pub(crate) struct RegistrySchema<'a>(Option<&'a ResourceRegistry>);

impl satz_hcl::Schema for RegistrySchema<'_> {
    fn has_type(&self, tf_type: &str) -> bool {
        self.0.is_some_and(|r| r.resources.contains_key(tf_type))
    }

    fn has_attr(&self, tf_type: &str, attr: &str) -> bool {
        self.0.is_some_and(|r| {
            r.resources.get(tf_type).is_some_and(|s| s.1.block.attributes.contains_key(attr))
        })
    }

    fn required_attrs(&self, tf_type: &str) -> Vec<String> {
        self.0
            .and_then(|r| r.resources.get(tf_type))
            .map(|s| s.1.block.attributes.iter().filter(|(_, a)| a.required).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }
}

/// The live shape with `--into`: the delta against what the estate declares.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn import_delta(
    parent: &str,
    estate: PathBuf,
    cfg: ImportConfig,
    filtered: std::collections::HashSet<String>,
    on_collision: crate::discovery::OnCollision,
    verbose: bool,
    tool_config: &ToolConfig,
    runtime_config: &ToolConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::delta;
    reject_yaml_dialect(&estate, "import --into")?;
    println!("import: root {} → into {}", parent, estate.display());

    // 1. what the estate already covers, by live id (adopt's resolution, dry)
    let out = pipeline_b_generate(&estate, tool_config, runtime_config)?;
    let opts = crate::adopt::Options { only: Default::default(), activate: false };
    let mut live = crate::adopt::RealLive::new(&out.customer_id).await?;
    let resolutions = crate::adopt::resolve(&out.manifest, &cfg, &opts, &mut live).await;
    // what an earlier delta import wrote is re-derived now, not "declared"
    let resolutions: Vec<_> = resolutions.into_iter().filter(|r| !delta::from_imported_pack(&r.origin)).collect();
    let declared = delta::declared_from(&resolutions);
    println!("import: {} declared resource(s) resolved to live ids", declared.ids.len());
    if !declared.no_rule.is_empty() {
        println!(
            "import: {} declared resource(s) have no adoption rule and cannot be subtracted from the sweep — they may come back as new: {}",
            declared.no_rule.len(),
            declared.no_rule.join(", ")
        );
    }
    if !declared.blocked.is_empty() {
        let mut msg = format!("import --into: {} declared resource(s) could not be resolved to a live id; the sweep cannot be subtracted, nothing written:", declared.blocked.len());
        for (a, why) in &declared.blocked {
            msg.push_str(&format!("\n  {}: {}", a, why));
        }
        return Err(msg.into());
    }

    // 2. the sweep
    let registry = ResourceRegistry::load_all(&runtime_config.schema_dir)
        .map_err(|e| format!("Failed to load resource registry from {}: {}", runtime_config.schema_dir, e))?;
    let type_names: std::collections::HashSet<String> = registry.resources.keys().cloned().collect();
    let mut found = crate::discovery::Discoverer::discover_from_org(parent, verbose, Some(cfg), Some(&registry), on_collision).await?;
    attach_billing_accounts(&mut found.config).await;
    let top = match serde_yaml::to_value(&found.config)? {
        serde_yaml::Value::Mapping(m) => m,
        other => return Err(format!("discovered config is not a mapping: {:?}", other).into()),
    };
    let live_ids = delta::live_ids(&top);

    // 3. subtract
    let d = delta::subtract(top, &declared);

    // 4. packs + `use` lines
    let yaml_dir = Path::new(&runtime_config.yaml_dir);
    let mut estate_text = fsx::read_to_string(&estate)?;
    let estate_before = estate_text.clone();
    let mut written: Vec<String> = Vec::new();
    let header = |what: &str| {
        vec![
            format!("Imported from {} — what the estate did not declare {}.", parent, what),
            "Regenerated on every `satz import --into`; move entries into the estate as you adopt them —".to_string(),
            "the next run subtracts them by live id. `satz transpile`, then `tofu plan` is the check.".to_string(),
        ]
    };
    let is_type = |t: &str| type_names.contains(t);
    let top_name = delta::pack_name(parent, None);
    if d.top.is_empty() {
        // nothing left at the top level: an earlier run's pack goes, with its use
        if yaml_dir.join(&top_name).exists() {
            fsx::remove_file(yaml_dir.join(&top_name))?;
            if let Some(t) = delta::remove_use(&estate_text, &top_name) {
                estate_text = t;
            }
            println!("  removed {} (nothing left to import at the top level)", yaml_dir.join(&top_name).display());
        }
    }
    if !d.top.is_empty() {
        let name = delta::pack_name(parent, None);
        let mut top = d.top.clone();
        condense_document(&mut top, found.organization.as_deref(), &[], &registry);
        let satz = satz_core::migrate::convert_value(&top, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header("at the top level"))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write_generated_satz(yaml_dir.join(&name), &satz)?;
        if let Some(t) = delta::add_use(&estate_text, &name, None)? {
            estate_text = t;
        }
        written.push(name);
    }
    let mut hints: Vec<String> = Vec::new();
    // packs first, then the `use` lines bottom-up so earlier line numbers
    // stay valid (an insert shifts everything below it)
    let mut inserts: Vec<(u32, String)> = Vec::new();
    for (address, children) in &d.under {
        let name = delta::pack_name(parent, Some(address));
        let mut children = children.clone();
        condense_document(&mut children, found.organization.as_deref(), &[], &registry);
        let satz = satz_core::migrate::convert_value(&children, "pack", &name.trim_end_matches(".satz").replace('-', "_"), &[], &header(&format!("under {}", address)))?;
        let satz = satz_core::migrate::normalize_type_keys(&satz, &is_type);
        fsx::write_generated_satz(yaml_dir.join(&name), &satz)?;
        let origin = declared.containers.values().find(|(a, _)| a == address).and_then(|(_, o)| o.clone());
        match origin {
            Some((file, line)) if Path::new(&file) == estate.as_path() || Path::new(&file).ends_with(&estate) => {
                inserts.push((line, name.clone()));
            }
            Some((file, line)) => hints.push(format!("{} is declared in {}:{} (not the estate) — add `use \"{}\"` inside that block by hand", address, file, line, name)),
            None => hints.push(format!("{} has no declaring line — add `use \"{}\"` inside its block by hand", address, name)),
        }
        written.push(name);
    }
    inserts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (line, name) in inserts {
        if let Some(t) = delta::add_use(&estate_text, &name, Some(line))? {
            estate_text = t;
        }
    }
    // declared containers that have no residue any more: their earlier pack goes
    for (address, _) in declared.containers.values() {
        if d.under.contains_key(address) {
            continue;
        }
        let name = delta::pack_name(parent, Some(address));
        if yaml_dir.join(&name).exists() {
            fsx::remove_file(yaml_dir.join(&name))?;
            if let Some(t) = delta::remove_use(&estate_text, &name) {
                estate_text = t;
            }
            println!("  removed {} (nothing left to import under {})", yaml_dir.join(&name).display(), address);
        }
    }
    fsx::write_edited_satz(&estate, &estate_before, &estate_text)?;

    // 5. report
    println!();
    for name in &written {
        println!("  wrote {} (+ its `use` in {})", yaml_dir.join(name).display(), estate.display());
    }
    for h in &hints {
        println!("  note: {}", h);
    }
    let mut already = d.already.clone();
    already.sort();
    already.dedup();
    println!("\nimport: {} new resource(s) written, {} already declared, {} declared but not live", d.new, already.len(), declared.not_live.len());
    if verbose {
        for (id, address) in &already {
            println!("  already: {} = {}", address, id);
        }
    }
    for a in &declared.not_live {
        println!("  declared, not live (will be created on apply): {}", a);
    }
    let unseen: Vec<&String> = declared.ids.keys().filter(|id| !live_ids.contains(*id)).collect();
    if verbose && !unseen.is_empty() {
        println!("  {} declared id(s) not among the swept assets (types the sweep did not cover, or derived ids)", unseen.len());
    }
    crate::discovery::report_skipped(&found, &filtered, verbose);
    if written.is_empty() {
        println!("import: nothing to add — the estate already declares everything the sweep found.");
    }
    Ok(())
}

/// The Resource Manager asset carries no billing link; without it an imported
/// project plans `billing_account = null` — an unlink. Ask Cloud Billing per
/// project; a project this cannot be read for is named, not guessed.
pub(crate) async fn attach_billing_accounts(config: &mut Config) {
    fn projects_mut(config: &mut Config) -> Vec<&mut crate::config::Project> {
        fn walk<'a>(f: &'a mut crate::config::Folder, out: &mut Vec<&'a mut crate::config::Project>) {
            if let Some(ps) = &mut f.project {
                out.extend(ps.values_mut());
            }
            if let Some(fs) = &mut f.folder {
                for sub in fs.values_mut() {
                    walk(sub, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(ps) = &mut config.project {
            out.extend(ps.values_mut());
        }
        if let Some(fs) = &mut config.folder {
            for f in fs.values_mut() {
                walk(f, &mut out);
            }
        }
        out
    }
    let projects = projects_mut(config);
    if projects.is_empty() {
        return;
    }
    let token = match crate::gcp::access_token().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: billing accounts not read ({}); set `billing_account` on each project by hand", e);
            return;
        }
    };
    let http = reqwest::Client::new();
    for p in projects {
        match crate::gcp::billing::project_billing_account(&http, &token, &p.project_id).await.map_err(String::from) {
            Ok(Some(acct)) => p.billing_account = Some(acct),
            Ok(None) => {}
            Err(e) => eprintln!(
                "warning: billing account of project {} not read ({}) — set `billing_account` by hand or `tofu plan` will unlink it",
                p.project_id,
                e.lines().next().unwrap_or("")
            ),
        }
    }
}

/// The alphabetically first project id in the discovered tree, at any depth.
/// The APIs an organization-scoped read is billed against: a quota project
/// without them makes every org policy, service list and contact read fail
/// with a 403, which is how a live import's `tofu plan` came out as 25 errors
/// on the test organization.
const QUOTA_PROJECT_APIS: [&str; 2] = ["orgpolicy.googleapis.com", "serviceusage.googleapis.com"];

/// The project the providers bill organization-scoped calls to: the first
/// (by id) that enables the APIs those calls need — the day-0 infra project
/// does — else the first project at all, said so. `None` without a project.
pub(crate) fn quota_project(config: &Config) -> Option<String> {
    fn services(p: &crate::config::Project) -> Vec<String> {
        p.project_service
            .iter()
            .flatten()
            .filter_map(|s| match s {
                serde_yaml::Value::String(svc) => Some(svc.clone()),
                serde_yaml::Value::Mapping(m) => m.get("service").and_then(|v| v.as_str()).map(String::from),
                _ => None,
            })
            .collect()
    }
    fn walk_folder(f: &crate::config::Folder, out: &mut Vec<(String, Vec<String>)>) {
        if let Some(ps) = &f.project {
            out.extend(ps.values().map(|p| (p.project_id.clone(), services(p))));
        }
        if let Some(fs) = &f.folder {
            for sub in fs.values() {
                walk_folder(sub, out);
            }
        }
    }
    let mut projects: Vec<(String, Vec<String>)> =
        config.project.iter().flat_map(|ps| ps.values().map(|p| (p.project_id.clone(), services(p)))).collect();
    if let Some(fs) = &config.folder {
        for f in fs.values() {
            walk_folder(f, &mut projects);
        }
    }
    projects.sort();
    let able = projects.iter().find(|(_, svcs)| QUOTA_PROJECT_APIS.iter().all(|api| svcs.iter().any(|s| s == api)));
    match (able, projects.first()) {
        (Some((id, _)), _) => {
            println!("import: providers' quota project {} — it enables {}", id, QUOTA_PROJECT_APIS.join(" and "));
            Some(id.clone())
        }
        (None, Some((id, _))) => {
            println!(
                "import: providers' quota project {} — no project enables {}; organization-scoped reads may fail with 403 until one does (set `billing_project` in `providers` by hand)",
                id,
                QUOTA_PROJECT_APIS.join(" and ")
            );
            Some(id.clone())
        }
        (None, None) => None,
    }
}

/// The first organization number the tree names: an `organizations/<n>`
/// string anywhere, or an `org_id` value. Depth-first, document order.
pub(crate) fn infer_org_id(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => {
            let n = s.strip_prefix("organizations/")?;
            let digits: String = n.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!digits.is_empty() && digits.len() == n.len()).then_some(digits)
        }
        serde_yaml::Value::Mapping(m) => {
            if let Some(serde_yaml::Value::String(id)) = m.get(serde_yaml::Value::String("org_id".into())) {
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                    return Some(id.clone());
                }
            }
            m.values().find_map(infer_org_id)
        }
        serde_yaml::Value::Sequence(s) => s.iter().find_map(infer_org_id),
        _ => None,
    }
}

#[cfg(test)]
mod discover_satz {
    //! `discover-* --satz`: the discovered data must come out as an estate the
    //! fragment pipeline compiles, with the preamble the emitter requires and
    //! every discovered id carried as an import.
    use super::*;
    use crate::Commands;
    use clap::{CommandFactory, Parser};

    #[test]
    fn discovered_config_becomes_an_estate_that_compiles_and_imports() {
        let yaml = r#"
folder:
  workloads:
    display_name: Workloads
    import-id: folders/111
    project:
      infra:
        project_id: acme-infra-001
        name: acme-infra-001
        import-id: acme-infra-001
        project_service:
          - service: iam.googleapis.com
            import-id: acme-infra-001/iam.googleapis.com
google_storage_bucket:
  state:
    name: acme-state
    location: EU
    import-id: acme-state
    versioning:
      - enabled: true
google_organization_iam_audit_config:
  all:
    org_id: "123456789012"
    service: allServices
org_policy_policy:
  compute-managed-requireOsLogin:
    import-id: organizations/123456789012/policies/compute.managed.requireOsLogin
    name: compute.managed.requireOsLogin
    spec:
      - rules:
          - enforce: "TRUE"
google_organization_iam_member:
  "group:a@example.com":
    - role: roles/viewer
      import-id: 123456789012 roles/viewer group:a@example.com
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = crate::corpus::registry();
        let text = discovered_to_satz(&config, "discovered", None, &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert!(text.contains("customer_organization_id = \"123456789012\""), "{}", text);
        assert!(text.contains("terraform {"), "{}", text);
        assert!(text.contains("google_folder {"), "shorthand keys must be normalised:\n{}", text);
        // the condensed forms: one line per service and per grant edge, a
        // single block as a block, the organization referenced not repeated,
        // a project name equal to its id dropped
        assert!(text.contains("{ service = \"iam.googleapis.com\" \"import-id\" = \"acme-infra-001/iam.googleapis.com\" },"), "{}", text);
        assert!(text.contains("{ role = \"roles/viewer\" \"import-id\" = \"{customer_organization_id} roles/viewer group:a@example.com\" },"), "{}", text);
        assert!(text.contains("spec {\n"), "{}", text);
        assert!(text.contains("{ enforce = \"TRUE\" },"), "{}", text);
        assert!(text.contains("versioning {\n"), "{}", text);
        assert!(text.contains("org_id = customer_organization_id"), "{}", text);
        assert!(text.contains("\"import-id\" = \"organizations/{customer_organization_id}/policies/compute.managed.requireOsLogin\""), "{}", text);
        assert!(!text.contains("name = \"acme-infra-001\""), "{}", text);
        // the file is formatted on the way to disk; the inline forms survive it
        let formatted = satz_core::fmt::format(&text).unwrap();
        assert!(formatted.contains("{ service = \"iam.googleapis.com\" \"import-id\" = \"acme-infra-001/iam.googleapis.com\" },"), "{}", formatted);
        assert!(formatted.contains("{ enforce = \"TRUE\" },"), "{}", formatted);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("discovered estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        assert!(folded.conflicts().is_empty());
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        for a in [
            "google_folder.workloads",
            "google_project.infra",
            "google_storage_bucket.state",
            "google_organization_iam_audit_config.all",
            "google_org_policy_policy.compute_managed_requireOsLogin",
            "google_project_service.infra_iam_googleapis_com",
        ] {
            assert!(addrs.contains(a), "missing {} in {:?}", a, addrs);
        }
        for id in [
            "folders/111",
            "acme-infra-001",
            "acme-state",
            "acme-infra-001/iam.googleapis.com",
            "organizations/123456789012/policies/compute.managed.requireOsLogin",
            "123456789012 roles/viewer group:a@example.com",
        ] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
        assert!(out.main_tf.contains("organizations/123456789012/policies/compute.managed.requireOsLogin"), "the bare constraint expands:\n{}", out.main_tf);
        assert!(out.main_tf.contains("versioning {"), "{}", out.main_tf);
        assert!(!out.main_tf.contains("import-id"));
    }

    #[test]
    fn a_counted_grant_emits_its_own_address() {
        // one principal, one role, two folders: the map form alone emits one
        // address (its label hashes member + role) and the emitter refuses;
        // with the counter the second edge is a labelled resource of its own
        let yaml = r#"
folder:
  a:
    display_name: A
    import-id: folders/1
    google_folder_iam_member:
      "user:x@example.com":
        - role: roles/resourcemanager.folderAdmin
          import-id: folders/1 roles/resourcemanager.folderAdmin user:x@example.com
  b:
    display_name: B
    import-id: folders/2
    google_folder_iam_member:
      "user:x@example.com":
        - role: roles/resourcemanager.folderAdmin
          import-id: folders/2 roles/resourcemanager.folderAdmin user:x@example.com
"#;
        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = crate::corpus::registry();
        let notes = crate::discovery::resolve_grant_collisions(&mut config, crate::discovery::OnCollision::Counter).unwrap();
        assert_eq!(notes.len(), 1, "{:?}", notes);
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert!(text.contains("folderAdmin_x_2 {"), "{}", text);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("counted estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        assert_eq!(addrs.iter().filter(|a| a.starts_with("google_folder_iam_member.")).count(), 2, "{:?}", addrs);
        assert!(addrs.contains("google_folder_iam_member.folderAdmin_x_2"), "{:?}", addrs);
        assert!(out.main_tf.contains("google_folder.b.name"), "the labelled grant inherits its folder from the node:\n{}", out.main_tf);
        for id in ["folders/1 roles/resourcemanager.folderAdmin user:x@example.com", "folders/2 roles/resourcemanager.folderAdmin user:x@example.com"] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
    }

    /// `--on-collision` declares its values as its value parser: the help lists them, clap
    /// refuses anything else before a source is read, and `error` is what an import does
    /// without the flag.
    #[test]
    fn on_collision_is_error_or_counter_and_anything_else_is_refused() {
        use crate::discovery::OnCollision;
        let mut cmd = crate::Cli::command();
        cmd.build();
        let import = cmd.find_subcommand("import").expect("import is a command");
        let arg = import.get_arguments().find(|a| a.get_id() == "on_collision").expect("import takes --on-collision");
        let values: Vec<String> = arg.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
        assert_eq!(values, ["error", "counter"]);
        let err = crate::Cli::command()
            .try_get_matches_from(["satz", "import", "state.json", "--on-collision", "hash"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid value 'hash'"), "{err}");
        assert!(err.contains("error, counter"), "{err}");
        let parsed = |args: &[&str]| match crate::Cli::try_parse_from(args).expect("parses").command {
            Some(Commands::Import { on_collision, .. }) => on_collision,
            _ => panic!("not an import"),
        };
        assert_eq!(parsed(&["satz", "import", "state.json", "--on-collision", "counter"]), OnCollision::Counter);
        assert_eq!(parsed(&["satz", "import", "state.json"]), OnCollision::Error);
    }

    #[test]
    fn pinned_bucket_grants_emit_one_address_per_bucket() {
        // two buckets, one member, one role: two maps pinned to their bucket,
        // two addresses (the pin enters the label), two imports
        let yaml = r#"
project:
  infra:
    project_id: acme-infra-001
    import-id: acme-infra-001
    google_storage_bucket_iam_member:
      - bucket: acme-logs-001
        "group:gcp-auditors@example.com":
          - { role: roles/storage.objectViewer, import-id: "b/acme-logs-001 roles/storage.objectViewer group:gcp-auditors@example.com" }
      - bucket: acme-state
        "group:gcp-auditors@example.com":
          - { role: roles/storage.objectViewer, import-id: "b/acme-state roles/storage.objectViewer group:gcp-auditors@example.com" }
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = crate::corpus::registry();
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &crate::vocabulary::Vocabulary::default()).unwrap();
        assert_eq!(text.matches("google_storage_bucket_iam_member {").count(), 2, "{}", text);
        assert!(text.contains("bucket = \"acme-logs-001\""), "{}", text);
        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("pinned estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        let addrs = out.manifest.addresses();
        assert_eq!(addrs.iter().filter(|a| a.starts_with("google_storage_bucket_iam_member.")).count(), 2, "{:?}", addrs);
        for id in ["b/acme-logs-001 roles/storage.objectViewer group:gcp-auditors@example.com", "b/acme-state roles/storage.objectViewer group:gcp-auditors@example.com"] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
    }

    #[test]
    fn the_vocabulary_is_bound_in_the_params_and_referenced_in_the_body() {
        // the day-0 params, inferred from what the sweep found, each marked
        // with its rule; every literal they name becomes the reference the
        // library spells, and an interpolated import id still resolves
        let yaml = r#"
organization_iam_member:
  "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com":
    - role: roles/resourcemanager.organizationAdmin
      import-id: 123456789012 roles/resourcemanager.organizationAdmin serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com
  "user:alice@example.com":
    - roles/viewer
folder:
  infra_folder:
    display_name: Infrastructure
    import-id: folders/111
    project:
      infra:
        project_id: acme-infra-001
        billing_account: 01AA-BB-CC
        import-id: acme-infra-001
        google_storage_bucket:
          state:
            name: acme-infra-001-state
            location: EU
            import-id: acme-infra-001-state
            versioning:
              - enabled: true
project:
  logs:
    project_id: acme-logs-001
    import-id: acme-logs-001
"#;
        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let reg = crate::corpus::registry();
        let vocab = crate::vocabulary::Vocabulary::infer(&config, Some("123456789012"), None, None);
        vocab.apply_billing(&mut config);
        let text = discovered_to_satz(&config, "discovered", Some("123456789012"), &reg, &vocab).unwrap();
        assert!(text.contains("customer_shortname = \"acme\" // inferred: the leading token of"), "{}", text);
        assert!(text.contains("infra_project_name = \"acme-infra-001\" // inferred:"), "{}", text);
        assert!(text.contains("infra_bucket_name = \"acme-infra-001-state\" // inferred:"), "{}", text);
        assert!(text.contains("billing_account_infra = \"01AA-BB-CC\" // inferred:"), "{}", text);
        assert!(text.contains("customer_domain = \"example.com\" // inferred:"), "{}", text);
        assert!(!text.contains("customer_longname"), "never a placeholder:\n{}", text);
        assert!(text.contains("display_name = infra_folder_name"), "{}", text);
        assert!(text.contains("project_id = infra_project_name"), "{}", text);
        assert!(text.contains("name = infra_bucket_name"), "{}", text);
        assert!(text.contains("\"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\" = ["), "{}", text);
        assert!(text.contains("\"import-id\" = \"{customer_organization_id} roles/resourcemanager.organizationAdmin serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\""), "{}", text);
        assert!(text.contains("project_id = \"{customer_shortname}-logs-001\""), "{}", text);
        assert!(text.contains("billing_account = \"\""), "a project without an account says so:\n{}", text);
        assert!(!text.contains("billing_account = \"01AA-BB-CC\""), "the infra project's account is the param:\n{}", text);
        assert!(text.contains("bucket = infra_bucket_name") && text.contains("prefix = \"hcl/state\""), "the gcs backend beside the local one:\n{}", text);

        let resolver = crate::EstateResolver { registry: &reg };
        let fe = satz_core::pipeline::compile_estate("discovered.satz", &text, &resolver, &|p| Err(format!("no use: {}", p)))
            .unwrap_or_else(|e| panic!("vocabulary estate does not compile: {:?}\n{}", e, text));
        let folded = satz_core::pipeline::fold_fragments(&resolver, &fe.fragments);
        let mut ctx = crate::emitter::EmitCtx::from_env(&fe.env);
        ctx.registry = Some(&reg);
        let out = crate::emitter::emit(&folded, &ctx).expect("emit");
        for id in [
            "123456789012 roles/resourcemanager.organizationAdmin serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com",
            "acme-infra-001",
            "acme-infra-001-state",
            "acme-logs-001",
        ] {
            assert!(out.imports_tf.contains(&format!("id = \"{}\"", id)), "missing import {}:\n{}", id, out.imports_tf);
        }
        assert!(out.main_tf.contains("project_id = \"acme-logs-001\""), "{}", out.main_tf);
    }

    #[test]
    fn the_quota_project_is_one_that_enables_the_organization_apis() {
        // alphabetically first is `alpha`, which enables nothing the
        // organization-scoped reads need; `infra` does
        let yaml = r#"
project:
  alpha:
    project_id: alpha-001
    project_service:
      - compute.googleapis.com
folder:
  f:
    display_name: F
    project:
      infra:
        project_id: infra-001
        project_service:
          - { service: orgpolicy.googleapis.com, import-id: infra-001/orgpolicy.googleapis.com }
          - serviceusage.googleapis.com
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(quota_project(&config).as_deref(), Some("infra-001"));
        let none: Config = serde_yaml::from_str("project:\n  alpha:\n    project_id: alpha-001\n").unwrap();
        assert_eq!(quota_project(&none).as_deref(), Some("alpha-001"), "the first project, and the report says why it may fail");
        assert_eq!(quota_project(&Config::default()), None);
    }

    #[test]
    fn org_id_is_inferred_from_references_or_org_id_keys() {
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  parent: organizations/123456789012\n").unwrap();
        assert_eq!(infer_org_id(&v).as_deref(), Some("123456789012"));
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  b:\n    org_id: \"222222222222\"\n").unwrap();
        assert_eq!(infer_org_id(&v).as_deref(), Some("222222222222"));
        let v: serde_yaml::Value = serde_yaml::from_str("a:\n  name: organizations/1/policies/x\n").unwrap();
        assert_eq!(infer_org_id(&v), None, "a policy name is not an org reference");
        assert_eq!(satz_output_path("yaml", PathBuf::from("discovered.yaml")), PathBuf::from("yaml/discovered.satz"));
        assert_eq!(satz_output_path("yaml", PathBuf::from("/abs/x.satz")), PathBuf::from("/abs/x.satz"));
    }
}
