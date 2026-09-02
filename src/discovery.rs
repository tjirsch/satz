use std::collections::{HashMap, HashSet, BTreeMap};
use serde_json::Value;
use crate::config::{Config, ImportConfig, Folder, Project};
use crate::schema::{ResourceRegistry, ResourceSchema, BlockSchema};
use google_cloud_asset_v1::model::{Asset, ContentType};
use google_cloud_gax::paginator::ItemPaginator;

pub struct Discoverer {
    pub state: Value,
    pub registry: Option<ResourceRegistry>,
    pub enabled_types: Option<HashSet<String>>,
    /// Types the run's `--only` / `only:` switched off — reported as
    /// "filtered", not "type off".
    pub filtered_types: HashSet<String>,
}

/// Why a resource the source had is not in the written estate. An import is
/// allowed to be partial; it is not allowed to be silent about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `import: false` in the import config.
    TypeOff,
    /// Switched off by `--only` / `only:` for this run.
    Filtered,
    /// The source had it, but no import-config row maps it (detail says what
    /// was missing).
    Unmapped(String),
    /// Its project/folder is not in the imported tree, so it has no place.
    ParentNotFound(String),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::TypeOff => write!(f, "type off (import: false)"),
            SkipReason::Filtered => write!(f, "filtered by --only"),
            SkipReason::Unmapped(d) => write!(f, "unmapped: {}", d),
            SkipReason::ParentNotFound(p) => write!(f, "parent not imported: {}", p),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub tf_type: String,
    pub what: String,
    pub reason: SkipReason,
}

/// What an import produced: the estate, and everything it left out.
pub struct Discovered {
    pub config: Config,
    pub skipped: Vec<Skipped>,
    /// Attributes dropped because the provider schema does not know them —
    /// `(tf_type, key path)`. CAI data is API-shaped; a key the Terraform
    /// schema lacks would not plan (roadmap F5).
    pub dropped_attrs: Vec<(String, String)>,
    /// The organization the assets' ancestors name (live shape only).
    pub organization: Option<String>,
}

thread_local! {
    // `filter_values` is called from five places, three of them without a
    // collector in reach; the run is single-threaded, so the dropped-key
    // list is collected here and taken once per import.
    static DROPPED_ATTRS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_dropped(tf_type: &str, key: &str) {
    DROPPED_ATTRS.with(|d| d.borrow_mut().push((tf_type.to_string(), key.to_string())));
}

fn take_dropped() -> Vec<(String, String)> {
    let mut v = DROPPED_ATTRS.with(|d| std::mem::take(&mut *d.borrow_mut()));
    v.sort();
    v.dedup();
    v
}

/// A grant entry carrying its Terraform import id: `{ role, "import-id" }`,
/// the object form the pipeline reads (language reference §6.7).
fn grant_entry(role: &str, import_id: &str) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    m.insert("role".into(), serde_yaml::Value::String(role.to_string()));
    m.insert("import-id".into(), serde_yaml::Value::String(import_id.to_string()));
    serde_yaml::Value::Mapping(m)
}

fn grant_role(v: &serde_yaml::Value) -> Option<&str> {
    v.as_str().or_else(|| v.as_mapping().and_then(|m| m.get("role")).and_then(|r| r.as_str()))
}

fn push_grant(roles: &mut Vec<serde_yaml::Value>, role: &str, import_id: Option<String>) {
    if roles.iter().any(|r| grant_role(r) == Some(role)) {
        return;
    }
    roles.push(match import_id {
        Some(id) => grant_entry(role, &id),
        None => serde_yaml::Value::String(role.to_string()),
    });
}

/// The import id of a grant, from its parent's identity — the same
/// derivation as the `import_id` templates adopt renders:
/// `<parent> <role> <member>` (`b/<bucket>` for bucket grants, the bare
/// number for the organization).
pub fn grant_import_id(tf_type: &str, parent: &str, role: &str, member: &str) -> String {
    let parent = match tf_type {
        "google_storage_bucket_iam_member" => format!("b/{}", parent.trim_start_matches("b/")),
        "google_organization_iam_member" => parent.trim_start_matches("organizations/").to_string(),
        _ => parent.to_string(),
    };
    format!("{} {} {}", parent, role, member)
}

/// The organization an asset's ancestor chain ends in.
pub fn organization_from_ancestors<'a, I: IntoIterator<Item = &'a String>>(ancestors: I) -> Option<String> {
    ancestors
        .into_iter()
        .find_map(|a| a.strip_prefix("organizations/").map(|n| n.to_string()))
}

/// Where a discovered asset lands: the estate under construction and the
/// folder/project maps it is assembled from, plus the id → key index.
struct Sinks<'a> {
    config: &'a mut Config,
    folder_map: &'a mut HashMap<String, Folder>,
    project_map: &'a mut HashMap<String, Project>,
    gcp_id_to_yaml_name: &'a HashMap<String, String>,
}

impl Discoverer {
    pub fn sanitize_yaml_key(s: &str) -> String {
        s.to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .replace(['_', ' ', '.'], "-")
    }
    
    pub fn new(
        state_json: Value,
        registry: Option<ResourceRegistry>,
        enabled_types: Option<HashSet<String>>,
        filtered_types: HashSet<String>,
    ) -> Self {
        Self {
            state: state_json,
            registry,
            enabled_types,
            filtered_types,
        }
    }

    fn is_type_enabled(&self, tf_type: &str) -> bool {
        match &self.enabled_types {
            Some(enabled) => enabled.contains(tf_type),
            None => true,
        }
    }

    pub fn discover(&self) -> Result<Discovered, Box<dyn std::error::Error>> {
        let mut config = Config::default();
        let mut skipped: Vec<Skipped> = Vec::new();
        let mut folder_map: HashMap<String, Folder> = HashMap::new(); 
        let mut project_map: HashMap<String, Project> = HashMap::new(); 
        let mut folder_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut project_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut gcp_id_to_yaml_name: HashMap<String, String> = HashMap::new();
        let mut orphan_resources: Vec<Value> = Vec::new();

        // `tofu show -json` documents carry `values.root_module`; a raw
        // `.tfstate` (or any other JSON) does not, and used to yield an empty
        // estate in silence
        if self.state.get("values").and_then(|v| v.get("root_module")).is_none() {
            return Err("state: not a `tofu show -json` document (no `values.root_module`) — a raw .tfstate? run `tofu show -json > state.json`".into());
        }
        let mut all_resources = Vec::new();
        Self::gather_resources(&self.state["values"]["root_module"], &mut all_resources);

        if !all_resources.is_empty() {
            for res in all_resources {
                let tf_type = res["type"].as_str().unwrap_or("");
                let values = &res["values"];
                let tf_name = res["name"].as_str().unwrap_or("");
                
                if !self.is_type_enabled(tf_type) {
                    let reason = if self.filtered_types.contains(tf_type) { SkipReason::Filtered } else { SkipReason::TypeOff };
                    skipped.push(Skipped { tf_type: tf_type.to_string(), what: tf_name.to_string(), reason });
                    continue;
                }

                match tf_type {
                    "google_folder" => {
                        let display_name = values["display_name"].as_str().unwrap_or(tf_name).to_string();
                        let gcp_id = values["name"].as_str().unwrap_or("").to_string(); 
                        let parent = values["parent"].as_str().unwrap_or("");

                        let yaml_key = if tf_name.is_empty() {
                            format!("folder_{}", gcp_id.replace("folders/", ""))
                        } else {
                            tf_name.to_string()
                        }.replace("/", "_").replace(".", "_").replace("-", "_");

                        gcp_id_to_yaml_name.insert(gcp_id.clone(), yaml_key.clone());

                        folder_map.insert(yaml_key, Folder {
                            display_name,
                            import_id: Some(gcp_id.clone()),
                            ..Default::default()
                        });

                        if !parent.is_empty() {
                            folder_id_to_parent.insert(gcp_id, parent.to_string());
                        }
                    }
                    "google_project" => {
                        let project_id = values["project_id"].as_str().unwrap_or("").to_string();
                        let display_name = values["name"].as_str().map(|s| s.to_string());
                        let folder_id = values["folder_id"].as_str().unwrap_or("");

                        let yaml_key = if tf_name.is_empty() {
                            project_id.clone()
                        } else {
                            tf_name.to_string()
                        }.replace("/", "_").replace(".", "_").replace("-", "_");

                        gcp_id_to_yaml_name.insert(project_id.clone(), yaml_key.clone());

                        project_map.insert(yaml_key, Project {
                            project_id: project_id.clone(),
                            name: display_name,
                            import_id: Some(project_id.clone()),
                            ..Default::default()
                        });

                        if !folder_id.is_empty() {
                            let f_id = if folder_id.starts_with("folders/") {
                                folder_id.to_string()
                            } else {
                                format!("folders/{}", folder_id)
                            };
                            project_id_to_parent.insert(project_id, f_id);
                        }
                    }
                    _ => {
                        orphan_resources.push(res.clone());
                    }
                }
            }
        }
        
        link_projects_to_folders(&project_id_to_parent, &gcp_id_to_yaml_name, &mut project_map, &mut folder_map)?;
        link_folders_to_parents(&folder_id_to_parent, &gcp_id_to_yaml_name, &mut folder_map)?;

        if !folder_map.is_empty() { config.folder = Some(folder_map); }
        if !project_map.is_empty() { config.project = Some(project_map); }

        for res in orphan_resources {
            let tf_type = res["type"].as_str().unwrap_or("");
            let values = &res["values"];
            let tf_name = res["name"].as_str().unwrap_or("");
            let schema = self.registry.as_ref().and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s);

            if let Some(p_id) = values["project"].as_str() {
                let p_yaml = gcp_id_to_yaml_name.get(p_id).map(|s| s.as_str()).unwrap_or(p_id);
                match Self::find_project_mut(&mut config, p_yaml) {
                    Some(project) => self.add_resource_to_project(project, tf_type, tf_name, values, schema)?,
                    None => skipped.push(Skipped {
                        tf_type: tf_type.to_string(),
                        what: tf_name.to_string(),
                        reason: SkipReason::ParentNotFound(format!("project {}", p_id)),
                    }),
                }
            } else if let Some(f_id) = values["folder"].as_str() {
                let f_norm = if f_id.starts_with("folders/") { f_id.to_string() } else { format!("folders/{}", f_id) };
                let f_yaml = gcp_id_to_yaml_name.get(&f_norm).map(|s| s.as_str()).unwrap_or(f_id);
                match Self::find_folder_mut(&mut config, f_yaml) {
                    Some(folder) => self.add_resource_to_folder(folder, tf_type, tf_name, values, schema)?,
                    None => skipped.push(Skipped {
                        tf_type: tf_type.to_string(),
                        what: tf_name.to_string(),
                        reason: SkipReason::ParentNotFound(f_norm),
                    }),
                }
            } else {
                self.add_resource_to_config(&mut config, tf_type, tf_name, values, schema)?;
            }
        }

        Ok(Discovered { config, skipped, dropped_attrs: take_dropped(), organization: None })
    }

    pub fn filter_values(tf_type: &str, values: &Value, schema: Option<&ResourceSchema>, add_import_id: bool, exclude: Option<&Vec<String>>, map: Option<&std::collections::BTreeMap<String, String>>) -> serde_yaml::Value {
        let mut yaml_val = serde_yaml::to_value(values).unwrap_or(serde_yaml::Value::Null);
        // API vocabulary → Terraform vocabulary where the names differ (F5c),
        // on the API's own key spelling, before anything else looks at keys
        if let (Some(m), serde_yaml::Value::Mapping(d)) = (map, &mut yaml_val) {
            crate::align::apply_map(d, m);
        }
        let block_schema = schema.map(|s| &s.block);
        
        // Construct Blacklist
        let mut blacklist = vec!["id", "etag", "self_link", "unique_id", "create_time", "update_time", "member", "project", "folder"];
        if tf_type != "google_project" {
            blacklist.push("project_id");
        }
        if tf_type == "google_project_service" {
            blacklist.push("state");
            blacklist.push("name");
            blacklist.push("parent");
        }
        
        let mut full_blacklist: Vec<String> = blacklist.iter().map(|s| s.to_string()).collect();
        if let Some(ex) = exclude {
            full_blacklist.extend(ex.clone());
        }

        Self::filter_recursive(&mut yaml_val, block_schema, &full_blacklist, tf_type, "");

        if let Some(id) = values["id"].as_str() {
            if add_import_id {
                if let serde_yaml::Value::Mapping(map) = yaml_val {
                    let mut new_map = serde_yaml::Mapping::new();
                    new_map.insert(serde_yaml::Value::String("import-id".to_string()), serde_yaml::Value::String(id.to_string()));
                    new_map.extend(map);
                    yaml_val = serde_yaml::Value::Mapping(new_map);
                }
            }
        }

        if tf_type == "google_project_service" {
            if let serde_yaml::Value::Mapping(mut map) = yaml_val {
                if let Some(serde_yaml::Value::String(service)) = map.remove(serde_yaml::Value::String("service".to_string())) {
                    if map.is_empty() {
                        return serde_yaml::Value::String(service);
                    } else {
                        let mut new_map = serde_yaml::Mapping::new();
                        new_map.insert(serde_yaml::Value::String(service), serde_yaml::Value::Mapping(map));
                        return serde_yaml::Value::Mapping(new_map);
                    }
                }
                return serde_yaml::Value::Mapping(map);
            }
        }
        yaml_val
    }


    fn filter_recursive(val: &mut serde_yaml::Value, schema: Option<&BlockSchema>, blacklist: &[String], tf_type: &str, at: &str) {
        if let serde_yaml::Value::Mapping(map) = val {
            if map.keys().any(|k| k.as_str().is_some_and(|k| k.chars().any(|c| c.is_ascii_uppercase()))) {
                let renamed: serde_yaml::Mapping = std::mem::take(map)
                    .into_iter()
                    .map(|(k, v)| match k.as_str() {
                        Some(ks) => (serde_yaml::Value::String(crate::align::snake(ks)), v),
                        None => (k, v),
                    })
                    .collect();
                *map = renamed;
            }
            for key in blacklist {
                map.remove(serde_yaml::Value::String(key.to_string()));
            }

            let label_keys = ["labels", "terraform_labels", "effective_labels"];
            for l_key in label_keys {
                if let Some(serde_yaml::Value::Mapping(labels)) = map.get_mut(serde_yaml::Value::String(l_key.to_string())) {
                    labels.remove(serde_yaml::Value::String("goog-terraform-provisioned".to_string()));
                }
            }

            if let Some(s) = schema {
                map.retain(|k, v| {
                    if let serde_yaml::Value::String(k_str) = k {
                        // A key neither the attributes nor the blocks know is
                        // API vocabulary the provider does not speak (F5):
                        // it would not plan, so it goes — and is reported.
                        if !s.attributes.contains_key(k_str) && !s.block_types.contains_key(k_str) {
                            note_dropped(tf_type, &format!("{}{}", at, k_str));
                            return false;
                        }
                        if let Some(attr) = s.attributes.get(k_str) {
                            if attr.required { return true; }
                            if let Some(default_json) = &attr.default {
                                if let Ok(default_yaml) = serde_yaml::to_value(default_json) {
                                    if v == &default_yaml { return false; }
                                }
                            }
                            if attr.computed && !attr.optional && !attr.required {
                                let keep_computed = ["org_id", "folder_id", "project_id"];
                                if !keep_computed.contains(&k_str.as_str()) { return false; }
                            }
                            if attr.optional && !attr.required
                                && Self::is_absent_value(v) { return false; }
                        }
                        if let Some(block_type) = s.block_types.get(k_str) {
                            if let Some(min) = block_type.min_items {
                                if min > 0 { return true; }
                            }
                        }
                    }
                    true
                });
            }

            for (k, v) in map.iter_mut() {
                let k_str = k.as_str().unwrap_or("");
                // A string-typed attribute holding structured data (org-policy
                // `parameters` is a JSON string in Terraform, an object in
                // the API) is carried as its JSON text.
                if schema.and_then(|s| s.attributes.get(k_str)).is_some_and(|a| a.is_string())
                    && (v.is_mapping() || v.is_sequence())
                {
                    if let Ok(json) = serde_json::to_value(&*v).and_then(|j| serde_json::to_string(&j)) {
                        *v = serde_yaml::Value::String(json);
                    }
                    continue;
                }
                let sub_schema = schema.and_then(|s| s.block_types.get(k_str)).map(|bt| &bt.block);
                Self::filter_recursive(v, sub_schema, blacklist, tf_type, &format!("{}{}.", at, k_str));
            }

            map.retain(|_, v| {
                !Self::is_empty_value(v)
            });
        } else if let serde_yaml::Value::Sequence(seq) = val {
             for item in seq.iter_mut() {
                Self::filter_recursive(item, schema, blacklist, tf_type, at);
            }
            seq.retain(|v| {
                !Self::is_empty_value(v)
            });
        }
    }

    /// Null, "" , [] or {} — carries nothing, dropped from discovered output.
    fn is_empty_value(v: &serde_yaml::Value) -> bool {
        v.is_null()
            || v.as_str().is_some_and(|s| s.is_empty())
            || v.as_sequence().is_some_and(|s| s.is_empty())
            || v.as_mapping().is_some_and(|m| m.is_empty())
    }

    /// Unset, not "zero": `false`, `0`, `""` and `"default"` are values —
    /// where they differ from the provider default they must be written, or
    /// the plan flips them back.
    fn is_absent_value(v: &serde_yaml::Value) -> bool {
        match v {
            serde_yaml::Value::Sequence(seq) => seq.is_empty(),
            serde_yaml::Value::Mapping(m) => m.is_empty(),
            serde_yaml::Value::Null => true,
            _ => false,
        }
    }

    fn find_project_mut<'a>(config: &'a mut Config, project_id: &str) -> Option<&'a mut Project> {
        if let Some(projects) = &mut config.project {
            if let Some(p) = projects.get_mut(project_id) { return Some(p); }
        }
        if let Some(folders) = &mut config.folder {
            for folder in folders.values_mut() {
                if let Some(p) = Self::find_project_in_folder_mut(folder, project_id) { return Some(p); }
            }
        }
        None
    }

    fn find_project_in_folder_mut<'a>(folder: &'a mut Folder, project_id: &str) -> Option<&'a mut Project> {
        if let Some(projects) = &mut folder.project {
            if let Some(p) = projects.get_mut(project_id) { return Some(p); }
        }
        if let Some(folders) = &mut folder.folder {
            for subfolder in folders.values_mut() {
                if let Some(p) = Self::find_project_in_folder_mut(subfolder, project_id) { return Some(p); }
            }
        }
        None
    }

    fn find_folder_mut<'a>(config: &'a mut Config, folder_id: &str) -> Option<&'a mut Folder> {
        if let Some(folders) = &mut config.folder {
            if folders.contains_key(folder_id) { return folders.get_mut(folder_id); }
            for folder in folders.values_mut() {
                if let Some(f) = Self::find_folder_recursive_mut(folder, folder_id) { return Some(f); }
            }
        }
        None
    }

    fn find_folder_recursive_mut<'a>(folder: &'a mut Folder, folder_id: &str) -> Option<&'a mut Folder> {
        if let Some(folders) = &mut folder.folder {
            if folders.contains_key(folder_id) { return folders.get_mut(folder_id); }
            for subfolder in folders.values_mut() {
                if let Some(f) = Self::find_folder_recursive_mut(subfolder, folder_id) { return Some(f); }
            }
        }
        None
    }

    fn add_resource_to_project(&self, p: &mut Project, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) -> Result<(), String> {
        if tf_type.ends_with("_iam_member") {
            let (role, member) = grant_identity(tf_type, tf_name, values)?;
            let parent = if tf_type == "google_storage_bucket_iam_member" {
                values["bucket"].as_str().unwrap_or("").to_string()
            } else {
                p.project_id.clone()
            };
            let id = grant_import_id(tf_type, &parent, &role, &member);
            if !p.extra.contains_key(tf_type) { p.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(members_map)) = p.extra.get_mut(tf_type) {
                let member_key = serde_yaml::Value::String(member);
                if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                    push_grant(roles, &role, Some(id));
                }
            }
            return Ok(());
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None, None);
        if tf_type == "google_project_service" {
            if p.project_service.is_none() { p.project_service = Some(Vec::new()); }
            p.project_service.as_mut().unwrap().push(yaml_val);
        } else {
            if !p.extra.contains_key(tf_type) { p.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(type_map)) = p.extra.get_mut(tf_type) {
                type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
            }
        }
        Ok(())
    }

    fn add_resource_to_folder(&self, f: &mut Folder, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) -> Result<(), String> {
        if tf_type.ends_with("_iam_member") {
            let (role, member) = grant_identity(tf_type, tf_name, values)?;
            let parent = f.import_id.clone().unwrap_or_else(|| values["folder"].as_str().unwrap_or("").to_string());
            let id = grant_import_id(tf_type, &parent, &role, &member);
            if !f.extra.contains_key(tf_type) { f.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(members_map)) = f.extra.get_mut(tf_type) {
                let member_key = serde_yaml::Value::String(member);
                if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                    push_grant(roles, &role, Some(id));
                }
            }
            return Ok(());
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None, None);
        if !f.extra.contains_key(tf_type) { f.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
        if let Some(serde_yaml::Value::Mapping(type_map)) = f.extra.get_mut(tf_type) {
             type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
        }
        Ok(())
    }

    fn add_resource_to_config(&self, c: &mut Config, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) -> Result<(), String> {
        if tf_type.ends_with("_iam_member") {
            let (role, member) = grant_identity(tf_type, tf_name, values)?;
            let parent = ["org_id", "billing_account_id", "folder", "project", "bucket"]
                .iter()
                .find_map(|k| values[*k].as_str().filter(|s| !s.is_empty()))
                .unwrap_or("")
                .to_string();
            let id = grant_import_id(tf_type, &parent, &role, &member);

            if tf_type == "google_organization_iam_member" {
                if c.organization_iam_member.is_none() { c.organization_iam_member = Some(HashMap::new()); }
                if let Some(ref mut members_map) = c.organization_iam_member {
                    let roles = members_map.entry(member).or_insert_with(Vec::new);
                    push_grant(roles, &role, Some(id));
                }
            } else {
                if !c.extra.contains_key(tf_type) { c.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
                if let Some(serde_yaml::Value::Mapping(members_map)) = c.extra.get_mut(tf_type) {
                    let member_key = serde_yaml::Value::String(member);
                    if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                    if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                        push_grant(roles, &role, Some(id));
                    }
                }
            }
            return Ok(());
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None, None);
        if !c.extra.contains_key(tf_type) { c.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
        if let Some(serde_yaml::Value::Mapping(type_map)) = c.extra.get_mut(tf_type) {
            type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
        }
        Ok(())
    }

    fn gather_resources(module: &Value, all: &mut Vec<Value>) {
        if let Some(resources) = module["resources"].as_array() {
            for res in resources { all.push(res.clone()); }
        }
        if let Some(children) = module["child_modules"].as_array() {
            for child in children { Self::gather_resources(child, all); }
        }
    }

    /// (`organization`|`folder`|`project`, id) from the asset name, else from
    /// its ancestors. `None` when neither says — an asset with no scope has no
    /// place in the estate and is skipped with that reason, never filed under
    /// the organization with an empty id.
    fn get_asset_scope(asset: &Asset) -> Option<(String, String)> {
        let name = &asset.name;
        if name.contains("/projects/") {
            let after = name.split("/projects/").last().unwrap_or("");
            let pid = after.split('/').next().unwrap_or(after).to_string();
            return Some(("project".to_string(), pid));
        } else if name.contains("/folders/") {
            let after = name.split("/folders/").last().unwrap_or("");
            let fid = after.split('/').next().unwrap_or(after);
            return Some(("folder".to_string(), format!("folders/{}", fid)));
        } else if name.contains("/organizations/") {
            let after = name.split("/organizations/").last().unwrap_or("");
            let oid = after.split('/').next().unwrap_or(after).to_string();
            return Some(("organization".to_string(), oid));
        }
        for ancestor in &asset.ancestors {
            if let Some(pid) = ancestor.strip_prefix("projects/") {
                return Some(("project".to_string(), pid.to_string()));
            } else if ancestor.starts_with("folders/") {
                return Some(("folder".to_string(), ancestor.to_string()));
            } else if let Some(oid) = ancestor.strip_prefix("organizations/") {
                return Some(("organization".to_string(), oid.to_string()));
            }
        }
        None
    }

    /// `parent` is any Cloud Asset Inventory scope: `organizations/<n>`,
    /// `folders/<n>` or `projects/<id>`.
    pub async fn discover_from_org(
        parent: &str,
        verbose: bool,
        discovery_config: Option<ImportConfig>,
        registry: Option<ResourceRegistry>,
    ) -> Result<Discovered, Box<dyn std::error::Error>> {
        let client = crate::gcp::asset_service().await?;
        
        let mut type_map: BTreeMap<u32, std::collections::BTreeSet<String>> = BTreeMap::new();
        
        // the enabled rows decide what is swept; a row that cannot be swept
        // is an error (TODO asset type, unknown content type) or, for types
        // Cloud Asset Inventory does not carry at all, reported once
        let mut not_inventoried: Vec<&str> = Vec::new();
        if let Some(config) = &discovery_config {
            for (tf_type, resource_config) in &config.resource_types {
                if !resource_config.import { continue; }
                let Some(cat) = resource_config.asset_type.as_deref() else {
                    not_inventoried.push(tf_type);
                    continue;
                };
                if cat.starts_with("TODO") {
                    return Err(format!(
                        "import-config: `{}` has import: true but its asset_type is still {} — \
                         run `scripts/update_import_config.py --cai-types presets/cai-asset-types.txt` or fill it by hand",
                        tf_type, cat
                    ).into());
                }
                let idx = match resource_config.content_type.as_deref().map(|c| c.to_uppercase()).as_deref() {
                    Some("RESOURCE") => 1,
                    Some("IAM_POLICY") => 2,
                    other => {
                        return Err(format!(
                            "import-config: `{}` has content_type {:?}; expected RESOURCE or IAM_POLICY",
                            tf_type, other
                        ).into())
                    }
                };
                type_map.entry(idx).or_default().insert(cat.to_string());
            }
        }
        if !not_inventoried.is_empty() {
            println!(
                "import: {} enabled type(s) are not Cloud Asset Inventory resources and cannot come from the live shape (state shape only): {}",
                not_inventoried.len(),
                not_inventoried.join(", ")
            );
        }

        let mut all_assets = Vec::new();
        let mut stats: HashMap<String, usize> = HashMap::new();
        let mut fetch_errors: Vec<String> = Vec::new();
        let mut unscoped: Vec<(String, String)> = Vec::new();

        for (ctype_int, asset_types) in type_map {
            let ctype = ContentType::from(ctype_int as i32);
            for asset_type in asset_types {
                 let asset_types_vec = vec![asset_type.clone()];
                 
                 let display_type = if asset_type.starts_with("cloudresourcemanager.googleapis.com/") {
                        asset_type.trim_start_matches("cloudresourcemanager.googleapis.com/").to_string()
                    } else if asset_type.starts_with("orgpolicy.googleapis.com/") {
                        asset_type.trim_start_matches("orgpolicy.googleapis.com/").to_string()
                    } else {
                        asset_type.split('/').next_back().unwrap_or(&asset_type).to_string()
                    };
                 
                 println!("Fetching assets for type: {} (Content: {:?})", display_type, ctype);

                 let mut stream = client.list_assets()
                    .set_parent(parent.to_string())
                    .set_asset_types(asset_types_vec)
                    .set_content_type(ctype.clone())
                    .set_page_size(1000)
                    .by_item();
                
                 while let Some(asset_result) = stream.next().await {
                     match asset_result {
                         Ok(asset) => {
                             if verbose { println!("DEBUG: Found asset: {} ({})", asset.name, asset.asset_type); }
                             
                             let Some((scope, _scope_id)) = Self::get_asset_scope(&asset) else {
                                 unscoped.push((asset.asset_type.clone(), asset.name.clone()));
                                 continue;
                             };

                             if let Some(config) = &discovery_config {
                                  for (tf_type, r_config) in &config.resource_types {
                                      if r_config.import && r_config.asset_type.as_deref() == Some(&asset.asset_type) {
                                          // Removed: if verbose || asset.asset_type.contains("Service") { println!("DEBUG: Checking match for {}. tf_type: {}, scope: {}", asset.asset_type, tf_type, scope); }
                                          let is_match = if tf_type.contains("_project_") {
                                              scope == "project"
                                          } else if tf_type.contains("_folder_") {
                                              scope == "folder"
                                          } else if tf_type.contains("_organization_") {
                                              scope == "organization"
                                          } else if tf_type == "google_folder" {
                                              scope == "folder" || asset.asset_type == "cloudresourcemanager.googleapis.com/Folder"
                                          } else if tf_type == "google_project" {
                                              scope == "project" || asset.asset_type == "cloudresourcemanager.googleapis.com/Project"
                                          } else {
                                              true
                                          };
                                          
                                          if is_match {
                                              *stats.entry(tf_type.clone()).or_insert(0) += 1;
                                          }
                                      }
                                  }
                             }
                             all_assets.push(asset);
                         },
                         Err(e) => {
                             eprintln!("Error fetching asset type '{}': {}", asset_type, e);
                             fetch_errors.push(format!("{}: {}", asset_type, e));
                             break;
                         }
                     }
                 }
            }
        }
        
        // Fail fast: an estate built from a partial sweep would be silently
        // missing whole types, and the plan would then propose to create them.
        if !fetch_errors.is_empty() {
            return Err(format!(
                "import aborted — {} asset type(s) could not be fetched, nothing written:\n  {}",
                fetch_errors.len(),
                fetch_errors.join("\n  ")
            )
            .into());
        }

        if stats.is_empty() {
             println!("No assets discovered.");
        } else {
             println!("\n--- Discovery Statistics ---");
             let mut display_stats: Vec<_> = stats.iter().collect();
             display_stats.sort_by_key(|a| a.0);
             let total_label = "Total assets discovered";
             let max_len = display_stats.iter().map(|(n, _)| n.len()).max().unwrap_or(0).max(total_label.len());
             for (name, count) in display_stats {
                 println!("{:<width$}: {}", name, count, width = max_len);
             }
             println!("{:<width$}: {}\n", total_label, all_assets.len(), width = max_len);
        }

        let organization = all_assets.iter().find_map(|a| organization_from_ancestors(&a.ancestors));
        let (config, mut skipped) = Self::construct_config_from_assets(all_assets, registry.as_ref(), discovery_config.as_ref())?;
        for (tf_type, name) in unscoped {
            skipped.push(Skipped {
                tf_type,
                what: name,
                reason: SkipReason::Unmapped("no organization/folder/project scope in the asset name or its ancestors".into()),
            });
        }

        Ok(Discovered { config, skipped, dropped_attrs: take_dropped(), organization })
    }

    fn construct_config_from_assets(
        assets: Vec<Asset>,
        registry: Option<&ResourceRegistry>,
        discovery_config: Option<&ImportConfig>,
    ) -> Result<(Config, Vec<Skipped>), String> {
        let mut config = Config::default();
        let mut skipped: Vec<Skipped> = Vec::new();
        let mut deprecated_seen = HashSet::new();
        let mut folder_map: HashMap<String, Folder> = HashMap::new(); 
        let mut project_map: HashMap<String, Project> = HashMap::new();
        let mut folder_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut project_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut gcp_id_to_yaml_name: HashMap<String, String> = HashMap::new();
        
        let mut asset_type_to_config: HashMap<String, Vec<(String, &crate::config::ImportResourceConfig)>> = HashMap::new();
        if let Some(config) = discovery_config {
             for (tf_type, resource_config) in &config.resource_types {
                 if let Some(cat) = &resource_config.asset_type {
                     asset_type_to_config.entry(cat.clone()).or_default().push((tf_type.clone(), resource_config));
                 }
             }
        }

        // Pass 1: Folders and Projects first, to establish the hierarchy and the
        // id → key map (a Project asset carries both projectId and projectNumber;
        // IAM-policy assets name the project by NUMBER, which is why the map
        // must exist before pass 2 — running the project discovery on an
        // IAM-policy asset used to create a phantom project keyed by number).
        // We only care about RESOURCE content here to get display names and IDs.
        for asset in &assets {
             if asset.resource.is_none() {
                 continue;
             }

             if asset.asset_type != "cloudresourcemanager.googleapis.com/Folder" && 
                asset.asset_type != "cloudresourcemanager.googleapis.com/Project" {
                 continue;
             }

             let configs = if let Some(v) = asset_type_to_config.get(&asset.asset_type) { v } else { continue; };
             let (tf_type, res_config) = if let Some(found) = configs.iter().find(|(t, c)| (t == "google_folder" || t == "google_project") && c.content_type.as_deref() == Some("RESOURCE")) { found } else { continue; };

             if !res_config.import {
                 skipped.push(Skipped { tf_type: tf_type.clone(), what: asset.name.clone(), reason: SkipReason::TypeOff });
                 continue;
             }

             if tf_type == "google_folder" {
                 Self::discover_google_folder(asset, res_config, &mut folder_map, &mut folder_id_to_parent, &mut gcp_id_to_yaml_name);
             } else if tf_type == "google_project" {
                 Self::discover_google_project(asset, res_config, &mut project_map, &mut project_id_to_parent, &mut gcp_id_to_yaml_name);
             }
        }

        // Pass 2: Process all other resources (IAM, Policies, Services, Generic)
        for asset in &assets {
             if (asset.asset_type == "cloudresourcemanager.googleapis.com/Folder" || 
                 asset.asset_type == "cloudresourcemanager.googleapis.com/Project") && asset.resource.is_some() {
                 continue;
             }

             let Some(configs) = asset_type_to_config.get(&asset.asset_type) else {
                 skipped.push(Skipped {
                     tf_type: asset.asset_type.clone(),
                     what: asset.name.clone(),
                     reason: SkipReason::Unmapped(format!("no import-config row has asset_type {}", asset.asset_type)),
                 });
                 continue;
             };

             let Some((scope, scope_id)) = Self::get_asset_scope(asset) else {
                 skipped.push(Skipped {
                     tf_type: asset.asset_type.clone(),
                     what: asset.name.clone(),
                     reason: SkipReason::Unmapped("no organization/folder/project scope in the asset name or its ancestors".into()),
                 });
                 continue;
             };

             let matched_config = configs.iter().find(|(tf_type, c)| {
                 // Skip projects and folders as they are already handled
                 if tf_type == "google_folder" || tf_type == "google_project" {
                     return false;
                 }

                 let type_match = if asset.resource.is_some() { 
                     c.content_type.as_deref() == Some("RESOURCE") 
                 } else { 
                     c.content_type.as_deref() == Some("IAM_POLICY") 
                 };
                 
                 if !type_match { return false; }
                 
                 if !c.import { return false; }
                 
                 if tf_type.contains("_project_") { return scope == "project"; }
                 if tf_type.contains("_folder_") { return scope == "folder"; }
                 if tf_type.contains("_organization_") { return scope == "organization"; }
                 
                 true
             });

             let Some((tf_type, res_config)) = matched_config else {
                 let content = if asset.resource.is_some() { "RESOURCE" } else { "IAM_POLICY" };
                 let reason = if configs.iter().any(|(_, c)| !c.import) {
                     SkipReason::TypeOff
                 } else {
                     SkipReason::Unmapped(format!(
                         "no row for {} with content_type {} at {} scope (rows: {})",
                         asset.asset_type, content, scope,
                         configs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(", ")
                     ))
                 };
                 skipped.push(Skipped { tf_type: asset.asset_type.clone(), what: asset.name.clone(), reason });
                 continue;
             };

             if res_config.deprecated == Some(true) {
                 deprecated_seen.insert(tf_type.to_string());
             }

             if tf_type.contains("organization_policy") || tf_type == "google_org_policy_policy" {
                 Self::discover_organization_policy(tf_type, asset, res_config, registry, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             } else if asset.iam_policy.is_some() {
                 Self::discover_iam_policy(tf_type, asset, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             } else if tf_type == "google_project_service" {
                 Self::discover_google_project_service(tf_type, asset, res_config, registry, &scope_id, &mut project_map, &gcp_id_to_yaml_name);
             } else if let Err(reason) = Self::discover_generic_resource(tf_type, asset, res_config, registry, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name }) {
                 skipped.push(Skipped { tf_type: tf_type.to_string(), what: asset.name.clone(), reason });
             }
        }
        
        link_projects_to_folders(&project_id_to_parent, &gcp_id_to_yaml_name, &mut project_map, &mut folder_map)?;
        link_folders_to_parents(&folder_id_to_parent, &gcp_id_to_yaml_name, &mut folder_map)?;

        if !folder_map.is_empty() { config.folder = Some(folder_map); }
        if !project_map.is_empty() { config.project = Some(project_map); }
        
        for deprecated_type in deprecated_seen {
            eprintln!("Warning: Resource type '{}' is deprecated.", deprecated_type);
        }

        Ok((config, skipped))
    }

    fn discover_google_folder(
        asset: &Asset,
        _res_config: &crate::config::ImportResourceConfig,
        folder_map: &mut HashMap<String, Folder>,
        folder_id_to_parent: &mut HashMap<String, String>,
        gcp_id_to_yaml_name: &mut HashMap<String, String>,
    ) {
         let name = &asset.name;
         let parts: Vec<&str> = name.split("/folders/").collect();
         if parts.len() < 2 { return; }
         let folder_num = parts[1];
         let folder_id = format!("folders/{}", folder_num);

         // Helper for HCL compatibility: keys must start with a letter and be unique.
         // We use "folder-" + id to guarantee this.
         let yaml_key = format!("folder-{}", folder_num);
         
         let display_name = asset.resource.as_ref().and_then(|r| r.data.as_ref())
            .and_then(|d| d.get("displayName").or(d.get("name")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| yaml_key.clone());

         gcp_id_to_yaml_name.insert(folder_id.clone(), yaml_key.clone());
         
         let mut parent_string = None;
         if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
              if let Some(parent_val) = data.get("parent") {
                   parent_string = parent_val.as_str().map(|s| s.to_string());
              }
         }

         let folder = Folder {
             display_name,
             parent: parent_string.clone(),
             import_id: Some(folder_id.clone()),
             ..Default::default()
         };
         folder_map.insert(yaml_key, folder);
         
          if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
               if let Some(parent_val) = data.get("parent") {
                    let parent_str = if let Some(s) = parent_val.as_str() {
                         Some(s.to_string())
                    } else if let Some(obj) = parent_val.as_object() {
                         let type_str = obj.get("type").and_then(|v| v.as_str());
                         let id_str = obj.get("id").and_then(|v| v.as_str());
                         if let (Some(t), Some(id)) = (type_str, id_str) {
                              Some(format!("{}s/{}", t, id))
                         } else { None }
                    } else { None };

                    if let Some(parent) = parent_str {
                        let clean_parent = parent.trim_start_matches("//cloudresourcemanager.googleapis.com/");
                        folder_id_to_parent.insert(folder_id, clean_parent.to_string());
                    }
               }
          }
    }

    fn discover_google_project(
        asset: &Asset,
        res_config: &crate::config::ImportResourceConfig,
        project_map: &mut HashMap<String, Project>,
        project_id_to_parent: &mut HashMap<String, String>,
        gcp_id_to_yaml_name: &mut HashMap<String, String>,
    ) {
         let name = &asset.name; 
         let yaml_key_raw = if let Some(field) = &res_config.derive_yaml_key_from {
              if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
                   data.get(field).and_then(|v| v.as_str()).unwrap_or(name).to_string()
              } else { name.clone() }
         } else { name.clone() };
         let yaml_key = Self::sanitize_yaml_key(&yaml_key_raw);

         let parts: Vec<&str> = name.split("/projects/").collect();
         if parts.len() < 2 { return; }
         let project_id_prefix = parts[1];
         
         let project_id = if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
              data.get("projectId").and_then(|v| v.as_str()).unwrap_or(project_id_prefix).to_string()
         } else { project_id_prefix.to_string() };

         gcp_id_to_yaml_name.insert(project_id.clone(), yaml_key.clone());

         // Fix: Also map the project number (from data) to the yaml key
         // because child resources (like services) often reference the project by number.
         if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
             if let Some(num) = data.get("projectNumber").and_then(|v| v.as_str()) {
                 gcp_id_to_yaml_name.insert(num.to_string(), yaml_key.clone());
             }
         }


         let display_name = asset.resource.as_ref().and_then(|r| r.data.as_ref())
            .and_then(|d| d.get("displayName").or(d.get("name")))
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();

         let mut labels = None;
         let mut tags = None;
         let mut billing_account = None;
         let mut deletion_policy = None;  

         if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
             // Extract Labels
             if let Some(l_map) = data.get("labels").and_then(|v| v.as_object()) {
                 let mut extracted = HashMap::new();
                 for (k, v) in l_map {
                     if let Some(s) = v.as_str() {
                         extracted.insert(k.clone(), s.to_string());
                     }
                 }
                 if !extracted.is_empty() { labels = Some(extracted); }
             }

             // Extract Tags (assuming 'tags' field which is a list of strings)
             if let Some(t_list) = data.get("tags").and_then(|v| v.as_array()) {
                 let extracted: Vec<String> = t_list.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                 if !extracted.is_empty() { tags = Some(extracted); }
             }

             // Extract Billing Account if present
             if let Some(ba) = data.get("billing_account").and_then(|v| v.as_str()) {
                 billing_account = Some(ba.to_string());
             }

             // Extract Deletion Policy
             if let Some(dp) = data.get("deletion_policy").and_then(|v| v.as_str()) {
                 deletion_policy = Some(dp.to_string());
             }
         }

         let project = Project {
             project_id: project_id.clone(),
             name: Some(display_name),
             labels,
             tags,
             billing_account,
             deletion_policy,
             import_id: Some(project_id.clone()),
             ..Default::default()
         };
         project_map.insert(yaml_key, project);
         
          if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
               if let Some(parent_val) = data.get("parent") {
                    let parent_str = if let Some(s) = parent_val.as_str() {
                         Some(s.to_string())
                    } else if let Some(obj) = parent_val.as_object() {
                         let type_str = obj.get("type").and_then(|v| v.as_str());
                         let id_str = obj.get("id").and_then(|v| v.as_str());
                         if let (Some(t), Some(id)) = (type_str, id_str) {
                              Some(format!("{}s/{}", t, id))
                         } else { None }
                    } else { None };

                    if let Some(parent) = parent_str {
                        let clean_parent = parent.trim_start_matches("//cloudresourcemanager.googleapis.com/");
                        project_id_to_parent.insert(project_id, clean_parent.to_string());
                    }
               }
          }
    }

    fn discover_google_project_service(
         tf_type: &str,
         asset: &Asset,
         res_config: &crate::config::ImportResourceConfig,
         registry: Option<&ResourceRegistry>,
         scope_id: &str,
         project_map: &mut HashMap<String, Project>,
         gcp_id_to_yaml_name: &HashMap<String, String>,
    ) {
         // name format: //serviceusage.googleapis.com/projects/my-project/services/storage.googleapis.com
         let service_name = asset.name.split("/services/").last().unwrap_or("").to_string();
         if service_name.is_empty() { return; }

         let resource_val = if let Some(resource) = &asset.resource {
               if let Some(data) = &resource.data {
                   let schema = registry.and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s);
                   let mut data_clone = data.clone();
                   // THIS IS THE FIX: Inject service name since it's missing in asset data
                   data_clone.insert("service".to_string(), serde_json::Value::String(service_name.clone()));

                   let data_val = serde_json::Value::Object(data_clone);
                   Self::filter_values(tf_type, &data_val, schema, false, res_config.exclude.as_ref(), res_config.map.as_ref())
               } else {
                   serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
               }
          } else {
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
          };

          if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
               if let Some(p) = project_map.get_mut(p_yaml) {
                    // The service's import id is `<project>/<service>` (adopt's
                    // rule), in the `{ "<service>": { "import-id": … } }` form the
                    // emitter reads (`filter_values` collapsed a bare service to a
                    // string).
                    let id = serde_yaml::Value::String(format!("{}/{}", p.project_id, service_name));
                    let resource_val = match resource_val {
                        serde_yaml::Value::String(svc) => {
                            let mut attrs = serde_yaml::Mapping::new();
                            attrs.insert("import-id".into(), id);
                            let mut m = serde_yaml::Mapping::new();
                            m.insert(serde_yaml::Value::String(svc), serde_yaml::Value::Mapping(attrs));
                            serde_yaml::Value::Mapping(m)
                        }
                        serde_yaml::Value::Mapping(mut m) => {
                            if let Some((_, serde_yaml::Value::Mapping(attrs))) = m.iter_mut().next() {
                                attrs.insert("import-id".into(), id);
                            }
                            serde_yaml::Value::Mapping(m)
                        }
                        other => other,
                    };
                    if p.project_service.is_none() { p.project_service = Some(Vec::new()); }
                    p.project_service.as_mut().unwrap().push(resource_val);
               }
          }
    }

    fn discover_organization_policy(
         tf_type: &str,
         asset: &Asset,
         res_config: &crate::config::ImportResourceConfig,
         registry: Option<&ResourceRegistry>,
         scope: &str,
         scope_id: &str,
         sinks: Sinks<'_>,
    ) {
         let Sinks { config, folder_map, project_map, gcp_id_to_yaml_name } = sinks;
          let name = &asset.name;
          
          let raw_key = if let Some(field) = &res_config.derive_yaml_key_from {
              if field == "name" {
                   if name.contains("/policies/") {
                        name.split("/policies/").last().unwrap_or(name)
                   } else {
                        name
                   }
              } else {
                   name // Fallback
              }
          } else { name };
          
          let sanitized_key = Self::sanitize_yaml_key(raw_key);
          let mut resource_val = serde_yaml::Mapping::new();
          
          if let Some(reg) = registry {
                if let Some((_, schema)) = reg.find_resource(tf_type) {
                     if let Some(map) = Self::process_organization_policy_family(tf_type, asset, schema, name, scope_id) {
                          resource_val = map;
                     }
                }
          }

          if !resource_val.is_empty() {
                    let import_id_val = resource_val.get(serde_yaml::Value::String("name".to_string())).cloned();

                    if let Some(val) = import_id_val {
                         let old_map = std::mem::replace(&mut resource_val, serde_yaml::Mapping::new());
                         
                         resource_val.insert(serde_yaml::Value::String("import-id".to_string()), val);
                         
                         for (k, v) in old_map {
                              resource_val.insert(k, v);
                         }
                    }
               }
          
          if resource_val.is_empty() { return; }

          let policy_map_val = serde_yaml::Value::Mapping(resource_val);

          if scope == "organization" {
              if tf_type == "google_org_policy_policy" {
                   if config.org_policy_policy.is_none() { config.org_policy_policy = Some(HashMap::new()); }
                   config.org_policy_policy.as_mut().unwrap().insert(sanitized_key.clone(), policy_map_val);
              } else if tf_type == "google_organization_policy" {
                   if config.google_organization_policy.is_none() { config.google_organization_policy = Some(HashMap::new()); }
                   config.google_organization_policy.as_mut().unwrap().insert(sanitized_key.clone(), policy_map_val);
              } else {
                   config.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                   if let Some(serde_yaml::Value::Mapping(m)) = config.extra.get_mut(tf_type) {
                        m.insert(serde_yaml::Value::String(sanitized_key.clone()), policy_map_val);
                   }
              }
          } else if scope == "folder" {
                if let Some(f_yaml) = gcp_id_to_yaml_name.get(scope_id) {
                    if let Some(f) = folder_map.get_mut(f_yaml) {
                        f.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                        if let Some(serde_yaml::Value::Mapping(m)) = f.extra.get_mut(tf_type) {
                            m.insert(serde_yaml::Value::String(sanitized_key.clone()), policy_map_val);
                        }
                    }
                }
          } else if scope == "project" {
                if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
                    if let Some(p) = project_map.get_mut(p_yaml) {
                         p.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                         if let Some(serde_yaml::Value::Mapping(m)) = p.extra.get_mut(tf_type) {
                             m.insert(serde_yaml::Value::String(sanitized_key.clone()), policy_map_val);
                         }
                    }
                }
          }
    }

    fn discover_iam_policy(
         tf_type: &str,
         asset: &Asset,
         scope: &str,
         scope_id: &str,
         sinks: Sinks<'_>,
    ) {
         let Sinks { config, folder_map, project_map, gcp_id_to_yaml_name } = sinks;
         if let Some(iam) = &asset.iam_policy {
             for binding in &iam.bindings {
                 if !binding.members.is_empty() {
                     for member in &binding.members {
                         let role = &binding.role;
                         if scope == "organization" {
                             if tf_type == "google_organization_iam_member" {
                                 if config.organization_iam_member.is_none() { config.organization_iam_member = Some(HashMap::new()); }
                                 if let Some(ref mut members_map) = config.organization_iam_member {
                                     let roles = members_map.entry(member.clone()).or_insert_with(Vec::<serde_yaml::Value>::new);
                                     push_grant(roles, role, Some(grant_import_id(tf_type, scope_id, role, member)));
                                 }
                             }
                         } else if scope == "folder" {
                             if let Some(f_yaml) = gcp_id_to_yaml_name.get(scope_id) {
                                 if let Some(f) = folder_map.get_mut(f_yaml) {
                                      f.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                                      if let Some(serde_yaml::Value::Mapping(members_map)) = f.extra.get_mut(tf_type) {
                                            let member_key = serde_yaml::Value::String(member.clone());
                                            if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                                            if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                                                push_grant(roles, role, Some(grant_import_id(tf_type, scope_id, role, member)));
                                            }
                                      }
                                 }
                             }
                         } else if scope == "project" {
                             if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
                                 if let Some(p) = project_map.get_mut(p_yaml) {
                                      let project_id = p.project_id.clone();
                                      if tf_type == "google_storage_bucket_iam_member" {
                                          let bucket_name = asset.name.split('/').next_back().unwrap_or("unknown-bucket").to_string();
                                          let member_sanitized = member.replace(":", "_").replace("@", "_").replace(".", "_");
                                          let role_sanitized = role.replace("roles/", "").replace(".", "_");
                                          let key = format!("{}-{}-{}", bucket_name, role_sanitized, member_sanitized);
                                          
                                          let mut resource_map = serde_yaml::Mapping::new();
                                          resource_map.insert(serde_yaml::Value::String("bucket".to_string()), serde_yaml::Value::String(bucket_name));
                                          resource_map.insert(serde_yaml::Value::String("member".to_string()), serde_yaml::Value::String(member.clone()));
                                          resource_map.insert(serde_yaml::Value::String("role".to_string()), serde_yaml::Value::String(role.clone()));
                                          resource_map.insert(
                                              serde_yaml::Value::String("import-id".to_string()),
                                              serde_yaml::Value::String(grant_import_id(tf_type, resource_map.get("bucket").and_then(|b| b.as_str()).unwrap_or(""), role, member)),
                                          );
                                          
                                          p.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                                          if let Some(serde_yaml::Value::Mapping(type_map)) = p.extra.get_mut(tf_type) {
                                              type_map.insert(serde_yaml::Value::String(key), serde_yaml::Value::Mapping(resource_map));
                                          }
                                      } else {
                                          p.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                                      if let Some(serde_yaml::Value::Mapping(members_map)) = p.extra.get_mut(tf_type) {
                                            let member_key = serde_yaml::Value::String(member.clone());
                                            if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                                            if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                                                push_grant(roles, role, Some(grant_import_id(tf_type, &project_id, role, member)));
                                            }
                                      }
                                  }
                             }
                        }
                         }
                     }
                 }
             }
         }
    }

    /// A resource that is not a container, a policy or a grant. Returns the
    /// reason when it cannot be expressed — the caller records it; nothing
    /// is dropped in silence.
    fn discover_generic_resource(
         tf_type: &str,
         asset: &Asset,
         res_config: &crate::config::ImportResourceConfig,
         registry: Option<&ResourceRegistry>,
         scope: &str,
         scope_id: &str,
         sinks: Sinks<'_>,
    ) -> Result<(), SkipReason> {
         let Sinks { config, folder_map, project_map, gcp_id_to_yaml_name } = sinks;
          let name = &asset.name;
          let raw_key = if let Some(field) = &res_config.derive_yaml_key_from {
               if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
                    data.get(field).and_then(|v| v.as_str()).unwrap_or(name).to_string()
               } else { name.clone() }
          } else { name.clone() };
          
          let sanitized_key = Self::sanitize_yaml_key(&raw_key.to_string());
          
          let mut resource_val = serde_yaml::Mapping::new();
          
          if let Some(resource) = &asset.resource {
               if let Some(data) = &resource.data {
                   let schema = registry.and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s);
                   let data_val = serde_json::Value::Object(data.clone());
                   if let serde_yaml::Value::Mapping(m) = Self::filter_values(tf_type, &data_val, schema, true, res_config.exclude.as_ref(), res_config.map.as_ref()) {
                        resource_val = m;
                   }
               }
          }
          
          if resource_val.is_empty() {
              return Err(SkipReason::Unmapped("no attribute of the asset data is in the provider schema".into()));
          }
          Self::complete_required(tf_type, asset, registry, &mut resource_val)?;
          // the live shape has no `id` field (that is the state shape's); the
          // asset path IS the resource name the provider imports by —
          // `tofu plan` on the import block validates it
          let id_key = serde_yaml::Value::String("import-id".into());
          let (extra, project_id) = match scope {
              "organization" => (&mut config.extra, None),
              "folder" => {
                  let f = gcp_id_to_yaml_name.get(scope_id).and_then(|f_yaml| folder_map.get_mut(f_yaml));
                  (&mut f.ok_or_else(|| SkipReason::ParentNotFound(scope_id.to_string()))?.extra, None)
              }
              "project" => {
                  let p = gcp_id_to_yaml_name.get(scope_id).and_then(|p_yaml| project_map.get_mut(p_yaml));
                  let p = p.ok_or_else(|| SkipReason::ParentNotFound(scope_id.to_string()))?;
                  let id = p.project_id.clone();
                  (&mut p.extra, Some(id))
              }
              other => return Err(SkipReason::Unmapped(format!("asset scope `{}` has no place in the estate", other))),
          };
          if !resource_val.contains_key(&id_key) {
              // Cloud Asset names project-scoped resources by project NUMBER;
              // imported that way the provider keeps the number as `project`
              // and the declared id then forces a replacement — so the import
              // id names the project by id
              let mut path = Self::asset_path(asset).to_string();
              if let Some(pid) = &project_id {
                  let by_number = format!("projects/{}/", scope_id);
                  if path.starts_with(&by_number) {
                      path = format!("projects/{}/{}", pid, &path[by_number.len()..]);
                  }
              }
              let mut with_id = serde_yaml::Mapping::new();
              with_id.insert(id_key, serde_yaml::Value::String(path));
              with_id.extend(resource_val);
              resource_val = with_id;
          }
          let policy_map_val = serde_yaml::Value::Mapping(resource_val);
          extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
          if let Some(serde_yaml::Value::Mapping(m)) = extra.get_mut(tf_type) {
              m.insert(serde_yaml::Value::String(sanitized_key), policy_map_val);
          }
          Ok(())
    }

    /// `//logging.googleapis.com/projects/p/sinks/x` → `projects/p/sinks/x`
    fn asset_path(asset: &Asset) -> &str {
        let path = asset.name.trim_start_matches("//");
        path.split_once('/').map(|(_, p)| p).unwrap_or(path)
    }

    /// Every attribute the provider REQUIRES must be present, or the block
    /// cannot plan. Cloud Asset Inventory data carries a few of them only
    /// implicitly: `parent` is the asset name minus its own collection and id
    /// (`…/organizations/1/contacts/0` → `organizations/1`); a service
    /// account's `account_id` is the local part of its email. Anything else
    /// still missing is not expressible from the asset — the resource is
    /// skipped with the attribute named.
    fn complete_required(
        tf_type: &str,
        asset: &Asset,
        registry: Option<&ResourceRegistry>,
        values: &mut serde_yaml::Mapping,
    ) -> Result<(), SkipReason> {
        let Some(schema) = registry.and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s) else { return Ok(()) };
        let mut required: Vec<&String> = schema.block.attributes.iter().filter(|(_, a)| a.required).map(|(k, _)| k).collect();
        required.sort();
        let path = Self::asset_path(asset);
        let segs: Vec<&str> = path.split('/').collect();
        // `organizations/1/…` → ("organizations", "1")
        let scope = (segs.len() >= 2).then(|| (segs[0], segs[1]));
        let raw = asset.resource.as_ref().and_then(|r| r.data.as_ref());
        for key in required {
            let k = serde_yaml::Value::String(key.clone());
            if values.contains_key(&k) {
                continue;
            }
            let derived = match key.as_str() {
                "parent" => (segs.len() >= 3).then(|| segs[..segs.len() - 2].join("/")),
                "org_id" => scope.filter(|(c, _)| *c == "organizations").map(|(_, id)| id.to_string()),
                "folder" => scope.filter(|(c, _)| *c == "folders").map(|(_, id)| id.to_string()),
                "project" => scope.filter(|(c, _)| *c == "projects").map(|(_, id)| id.to_string()),
                // the email is `computed` in the schema (filtered above), the
                // account id is its local part
                "account_id" if tf_type == "google_service_account" => raw
                    .and_then(|d| d.get("email"))
                    .and_then(|e| e.as_str())
                    .and_then(|e| e.split_once('@'))
                    .map(|(local, _)| local.to_string()),
                _ => None,
            };
            match derived {
                Some(v) => {
                    values.insert(k, serde_yaml::Value::String(v));
                }
                None => {
                    return Err(SkipReason::Unmapped(format!(
                        "required `{}` is not in the asset data and cannot be derived",
                        key
                    )))
                }
            }
        }
        Ok(())
    }
    

    pub fn print_summary(config: &Config) {
        println!("\n=== Configuration Summary ===");
        
        let mut stats: HashMap<String, usize> = HashMap::new();
        
        
        // Count Org Level
        if let Some(map) = &config.org_policy_policy { *stats.entry("google_org_policy_policy".to_string()).or_insert(0) += map.len(); }
        if let Some(map) = &config.google_organization_policy { *stats.entry("google_organization_policy".to_string()).or_insert(0) += map.len(); }
        if let Some(map) = &config.organization_iam_member { *stats.entry("google_organization_iam_member".to_string()).or_insert(0) += map.len(); }
        for (k, v) in &config.extra {
             if let serde_yaml::Value::Mapping(m) = v {
                 *stats.entry(k.clone()).or_insert(0) += m.len();
             }
        }

        // Count Folders
        if let Some(folders) = &config.folder {
            *stats.entry("google_folder".to_string()).or_insert(0) += folders.len();
            for f in folders.values() {
                Self::count_folder_resources(f, &mut stats);
            }
        }

        // Count Projects
        if let Some(projects) = &config.project {
            *stats.entry("google_project".to_string()).or_insert(0) += projects.len();
            for p in projects.values() {
                Self::count_project_resources(p, &mut stats);
            }
        }

        let mut sorted_stats: Vec<_> = stats.iter().collect();
        sorted_stats.sort_by_key(|a| a.0);
        
        for (k, v) in sorted_stats {
            println!("{:<30}: {}", k, v);
        }
    }

    fn count_folder_resources(f: &Folder, stats: &mut HashMap<String, usize>) {
        for (k, v) in &f.extra {
             if let serde_yaml::Value::Mapping(m) = v {
                 *stats.entry(k.clone()).or_insert(0) += m.len();
             }
        }
        if let Some(children) = &f.folder {
            *stats.entry("google_folder".to_string()).or_insert(0) += children.len();
            for child in children.values() {
                Self::count_folder_resources(child, stats);
            }
        }
        if let Some(projects) = &f.project {
            *stats.entry("google_project".to_string()).or_insert(0) += projects.len();
            for p in projects.values() {
                Self::count_project_resources(p, stats);
            }
        }
    }

    fn count_project_resources(p: &Project, stats: &mut HashMap<String, usize>) {
        for (k, v) in &p.extra {
             if let serde_yaml::Value::Mapping(m) = v {
                 *stats.entry(k.clone()).or_insert(0) += m.len();
             }
        }
        if let Some(services) = &p.project_service {
            *stats.entry("google_project_service".to_string()).or_insert(0) += services.len();
        }
    }

    fn process_organization_policy_family(tf_type: &str, asset: &Asset, schema: &ResourceSchema, name: &str, _scope_id: &str) -> Option<serde_yaml::Mapping> {
         // Derive 'constraint'
         let constraint = if name.contains("/policies/") {
              name.split("/policies/").last().unwrap_or(name)
         } else { name };

         // Extract data to a mutable map to inject missing fields
         let mut data_map = if let Some(r) = &asset.resource {
             if let Some(d) = &r.data {
                 d.clone()
             } else {
                 serde_json::Map::new()
             }
         } else {
             serde_json::Map::new()
         };

         // Parse scope from asset name
         // name format: //orgpolicy.googleapis.com/organizations/123456789012/policies/compute.managed.requireOsLogin
         let parts: Vec<&str> = name.split("/policies/").collect();
         let scope_part = if !parts.is_empty() { parts[0] } else { "" };
         
         if tf_type == "google_org_policy_policy" {
             // For google_org_policy_policy (V2):
             // 'name' argument is the full resource name: organizations/{org_id}/policies/{constraint_name}
             // 'parent' argument is the parent resource: organizations/{org_id}
             
             // Check if 'name' is present, if not inject it from asset name (stripped of service prefix)
             if !data_map.contains_key("name") {
                 // Asset name: //orgpolicy.googleapis.com/organizations/...
                 // We want: organizations/...
                 let relative_name = if let Some(idx) = name.find("organizations/") {
                     &name[idx..]
                 } else if let Some(idx) = name.find("folders/") {
                     &name[idx..]
                 } else if let Some(idx) = name.find("projects/") {
                     &name[idx..]
                 } else {
                     name // Fallback
                 };
                 data_map.insert("name".to_string(), serde_json::Value::String(relative_name.to_string()));
             }
             
             // Inject 'parent' if not present
             if !data_map.contains_key("parent") {
                  let parent = if let Some(idx) = scope_part.find("organizations/") {
                     &scope_part[idx..]
                 } else if let Some(idx) = scope_part.find("folders/") {
                     &scope_part[idx..]
                 } else if let Some(idx) = scope_part.find("projects/") {
                     &scope_part[idx..]
                 } else {
                     "" 
                 };
                 if !parent.is_empty() {
                    data_map.insert("parent".to_string(), serde_json::Value::String(parent.to_string()));
                 }
             }

         } else {
             // Legacy types
             data_map.insert("constraint".to_string(), serde_json::Value::String(constraint.to_string()));

             if tf_type == "google_organization_policy" {
                 if let Some(pos) = scope_part.find("organizations/") {
                     let id = &scope_part[pos+"organizations/".len()..];
                     data_map.insert("org_id".to_string(), serde_json::Value::String(id.to_string()));
                 }
             } else if tf_type == "google_folder_organization_policy" {
                 if let Some(pos) = scope_part.find("folders/") {
                     let id = &scope_part[pos+"folders/".len()..];
                     data_map.insert("folder".to_string(), serde_json::Value::String(id.to_string()));
                 }
             } else if tf_type == "google_project_organization_policy" {
                 if let Some(pos) = scope_part.find("projects/") {
                     let id = &scope_part[pos+"projects/".len()..];
                     data_map.insert("project".to_string(), serde_json::Value::String(id.to_string()));
                 }
             }
         }

         let extracted = schema.block.extract_attributes(&data_map, tf_type, name);
         
         if extracted.is_empty() {
             None
         } else {
             Some(extracted)
         }
    }
}

/// `role` and `member` of a grant record — both structural: a record without
/// them is corrupt input, never a placeholder in the estate.
fn grant_identity(tf_type: &str, tf_name: &str, values: &Value) -> Result<(String, String), String> {
    let take = |k: &str| {
        values[k]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(String::from)
            .ok_or_else(|| format!("state: {} `{}` has no `{}`", tf_type, tf_name, k))
    };
    Ok((take("role")?, take("member")?))
}

/// Nest each discovered project under its parent folder.
///
/// A project can have a surviving parent edge while its own record was filtered out —
/// e.g. Cloud Asset returned the ancestry but the asset itself was excluded by the
/// discovery config, or the caller lacked read access to it. That is normal partial
/// input, so an unresolvable id is reported and skipped rather than panicking.
///
/// A parent folder that is not in the sweep (`satz import projects/x` under a
/// folder, a folder root whose projects sit in sub-folders the filter dropped)
/// cannot nest the project, so the project keeps it as an explicit `folder_id`
/// — re-parenting it to the organization would make `apply` MOVE the project.
fn link_projects_to_folders(
    project_id_to_parent: &HashMap<String, String>,
    gcp_id_to_yaml_name: &HashMap<String, String>,
    project_map: &mut HashMap<String, Project>,
    folder_map: &mut HashMap<String, Folder>,
) -> Result<(), String> {
    let mut project_ids: Vec<&String> = project_id_to_parent.keys().collect();
    project_ids.sort();
    let mut kept_explicit = Vec::new();
    for p_id in project_ids {
        let f_id = &project_id_to_parent[p_id];
        let Some(p_yaml) = gcp_id_to_yaml_name.get(p_id).cloned() else {
            eprintln!("Warning: skipping project '{}' — it has a parent but no discovered resource record.", p_id);
            continue;
        };
        match gcp_id_to_yaml_name.get(f_id) {
            Some(f_yaml) => {
                let Some(folder) = folder_map.get_mut(f_yaml) else {
                    return Err(format!("import: folder {} ({}) is the parent of project {} but has no record to nest under", f_id, f_yaml, p_id));
                };
                let Some(project) = project_map.remove(&p_yaml) else {
                    return Err(format!("import: project {} ({}) has a parent but no record to move", p_id, p_yaml));
                };
                folder.project.get_or_insert_with(HashMap::new).insert(p_yaml, project);
            }
            None => {
                let Some(project) = project_map.get_mut(&p_yaml) else {
                    return Err(format!("import: project {} ({}) has a parent but no record to annotate", p_id, p_yaml));
                };
                project.extra.insert("folder_id".into(), serde_yaml::Value::String(f_id.trim_start_matches("folders/").to_string()));
                kept_explicit.push(format!("{} → {}", p_id, f_id));
            }
        }
    }
    if !kept_explicit.is_empty() {
        println!(
            "import: {} project(s) sit under a folder outside this sweep; kept as an explicit folder_id: {}",
            kept_explicit.len(),
            kept_explicit.join(", ")
        );
    }
    Ok(())
}

/// Nest each discovered folder under its parent folder, deepest first (depth
/// walked over the parent map — never inferred from the id) so a child is
/// always moved before the folder containing it. A folder whose parent is not
/// in the sweep keeps that parent explicitly, for the same reason as above.
fn link_folders_to_parents(
    folder_id_to_parent: &HashMap<String, String>,
    gcp_id_to_yaml_name: &HashMap<String, String>,
    folder_map: &mut HashMap<String, Folder>,
) -> Result<(), String> {
    let depth = |id: &String| -> usize {
        let mut d = 0;
        let mut cur = id;
        while let Some(parent) = folder_id_to_parent.get(cur) {
            d += 1;
            cur = parent;
            if d > 64 {
                break;
            }
        }
        d
    };
    let mut sorted_folder_ids: Vec<&String> = folder_id_to_parent.keys().collect();
    sorted_folder_ids.sort_by_key(|id| (std::cmp::Reverse(depth(id)), (*id).clone()));
    let mut kept_explicit = Vec::new();

    for child_id in sorted_folder_ids {
        let parent_id = &folder_id_to_parent[child_id];
        let Some(child_yaml) = gcp_id_to_yaml_name.get(child_id).cloned() else {
            eprintln!("Warning: skipping folder '{}' — it has a parent but no discovered resource record.", child_id);
            continue;
        };
        if !parent_id.starts_with("folders/") {
            continue; // organization root: the emitter derives it
        }
        match gcp_id_to_yaml_name.get(parent_id) {
            Some(parent_yaml) => {
                let Some(child_folder) = folder_map.remove(&child_yaml) else {
                    return Err(format!("import: folder {} ({}) has a parent but no record to move", child_id, child_yaml));
                };
                let Some(parent_folder) = folder_map.get_mut(parent_yaml) else {
                    return Err(format!(
                        "import: folder {} ({}) is the parent of {} but is not at the top level any more — nesting order is broken",
                        parent_id, parent_yaml, child_id
                    ));
                };
                parent_folder.folder.get_or_insert_with(HashMap::new).insert(child_yaml, child_folder);
            }
            None => {
                let Some(child_folder) = folder_map.get_mut(&child_yaml) else {
                    return Err(format!("import: folder {} ({}) has a parent but no record to annotate", child_id, child_yaml));
                };
                child_folder.parent = Some(parent_id.clone());
                kept_explicit.push(format!("{} → {}", child_id, parent_id));
            }
        }
    }
    if !kept_explicit.is_empty() {
        println!(
            "import: {} folder(s) sit under a folder outside this sweep; kept as an explicit parent: {}",
            kept_explicit.len(),
            kept_explicit.join(", ")
        );
    }
    Ok(())
}

/// The end of every import: what was left out, and why. Never silent — a
/// partial estate is fine, an unexplained one is not.
pub fn report_skipped(found: &Discovered, filtered_off: &HashSet<String>, verbose: bool) {
    let skipped = &found.skipped;
    if !found.dropped_attrs.is_empty() {
        let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
        for (t, _) in &found.dropped_attrs {
            *by_type.entry(t.as_str()).or_default() += 1;
        }
        println!("import: {} attribute(s) dropped — not in the provider schema (API vocabulary; would not plan):", found.dropped_attrs.len());
        for (t, n) in &by_type {
            println!("  {:5} {}", n, t);
        }
        if verbose {
            for (t, k) in &found.dropped_attrs {
                println!("  - {} .{}", t, k);
            }
        }
    }
    if skipped.is_empty() && filtered_off.is_empty() {
        println!("import: nothing skipped — every resource the source had is in the estate.");
        return;
    }
    let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
    if !filtered_off.is_empty() && !skipped.iter().any(|s| s.reason == SkipReason::Filtered) {
        // live shape: filtered types are never fetched, so they have no
        // per-resource rows — say so at the type level
        by_reason.insert(format!("type(s) filtered by --only, not fetched ({})", {
            let mut v: Vec<&str> = filtered_off.iter().map(String::as_str).collect();
            v.sort();
            v.join(", ")
        }), filtered_off.len());
    }
    for s in skipped {
        let key = match &s.reason {
            SkipReason::TypeOff => "type off (import: false)".to_string(),
            SkipReason::Filtered => "filtered by --only".to_string(),
            SkipReason::Unmapped(_) => "unmapped (no import-config row fits)".to_string(),
            SkipReason::ParentNotFound(_) => "parent not imported".to_string(),
        };
        *by_reason.entry(key).or_default() += 1;
    }
    println!("import: skipped {} resource(s):", skipped.len() + if skipped.iter().any(|s| s.reason == SkipReason::Filtered) { 0 } else { filtered_off.len() });
    for (reason, n) in &by_reason {
        println!("  {:5} {}", n, reason);
    }
    if verbose && !skipped.is_empty() {
        let mut rows: Vec<&Skipped> = skipped.iter().collect();
        rows.sort_by(|a, b| (&a.tf_type, &a.what).cmp(&(&b.tf_type, &b.what)));
        for s in rows {
            println!("  - {} {} — {}", s.tf_type, s.what, s.reason);
        }
    } else {
        println!("  (--verbose lists every one; `import: false` rows and `--only` are the levers)");
    }
}

#[cfg(test)]
mod nesting_tests {
    //! Folder nesting used to order by id STRING LENGTH and re-insert a child at
    //! the top level when its parent was already nested — a folder three levels
    //! deep was emitted under the organization, and `apply` would have moved it.
    use super::*;

    fn folder(name: &str) -> Folder {
        Folder { import_id: None, display_name: name.into(), parent: None, folder: None, project: None, extra: HashMap::new() }
    }

    #[test]
    fn folders_nest_by_depth_whatever_their_ids_look_like() {
        // A (short id) → B (long id) → C (medium id)
        let parents: HashMap<String, String> = [
            ("folders/1", "organizations/1"),
            ("folders/2000000002", "folders/1"),
            ("folders/30000003", "folders/2000000002"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let names: HashMap<String, String> = [
            ("folders/1", "a"),
            ("folders/2000000002", "b"),
            ("folders/30000003", "c"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let mut map: HashMap<String, Folder> = ["a", "b", "c"].into_iter().map(|n| (n.to_string(), folder(n))).collect();
        link_folders_to_parents(&parents, &names, &mut map).unwrap();
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["a"]);
        let b = &map["a"].folder.as_ref().unwrap()["b"];
        assert!(b.folder.as_ref().unwrap().contains_key("c"), "c nests under b under a");
    }

    #[test]
    fn a_parent_outside_the_sweep_stays_explicit() {
        let parents: HashMap<String, String> = [("folders/30000003", "folders/999999999")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let names: HashMap<String, String> = [("folders/30000003", "c")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut map: HashMap<String, Folder> = [("c".to_string(), folder("c"))].into_iter().collect();
        link_folders_to_parents(&parents, &names, &mut map).unwrap();
        assert_eq!(map["c"].parent.as_deref(), Some("folders/999999999"));

        let pparents: HashMap<String, String> = [("projects/p1", "folders/999999999")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let pnames: HashMap<String, String> = [("projects/p1", "p1")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut projects: HashMap<String, Project> = [("p1".to_string(), Project {
            import_id: None, name: None, project_id: "p1".into(), billing_account: None, labels: None, tags: None,
            deletion_policy: None, project_service: None, extra: HashMap::new(),
        })].into_iter().collect();
        let mut folders: HashMap<String, Folder> = HashMap::new();
        link_projects_to_folders(&pparents, &pnames, &mut projects, &mut folders).unwrap();
        assert_eq!(projects["p1"].extra.get("folder_id").and_then(|v| v.as_str()), Some("999999999"));
    }

    #[test]
    fn a_grant_without_role_is_corrupt_input_not_a_placeholder() {
        let v: serde_json::Value = serde_json::json!({"member": "user:a@example.com"});
        let err = grant_identity("google_project_iam_member", "x", &v).unwrap_err();
        assert!(err.contains("has no `role`"), "{}", err);
    }
}
