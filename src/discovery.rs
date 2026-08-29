use std::collections::{HashMap, HashSet, BTreeMap};
use serde_json::Value;
use crate::config::{Config, ImportConfig, Folder, Project};
use crate::schema::{ResourceRegistry, ResourceSchema, BlockSchema};
use google_cloud_asset_v1::client::AssetService;
use google_cloud_asset_v1::model::{Asset, ContentType};
use google_cloud_gax::paginator::ItemPaginator;

pub struct Discoverer {
    pub state: Value,
    pub registry: Option<ResourceRegistry>,
    pub filtered_count: std::cell::Cell<usize>,
    pub _verbose: bool, // Renamed to silence warning
    pub enabled_types: Option<HashSet<String>>,
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
        verbose: bool,
        enabled_types: Option<HashSet<String>>,
    ) -> Self {
        Self {
            state: state_json,
            registry,
            filtered_count: std::cell::Cell::new(0),
            _verbose: verbose,
            enabled_types,
        }
    }

    fn is_type_enabled(&self, tf_type: &str) -> bool {
        match &self.enabled_types {
            Some(enabled) => enabled.contains(tf_type),
            None => true,
        }
    }

    pub fn discover(&self) -> Result<Config, Box<dyn std::error::Error>> {
        let mut config = Config::default();
        let mut folder_map: HashMap<String, Folder> = HashMap::new(); 
        let mut project_map: HashMap<String, Project> = HashMap::new(); 
        let mut folder_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut project_id_to_parent: HashMap<String, String> = HashMap::new();
        let mut gcp_id_to_yaml_name: HashMap<String, String> = HashMap::new();
        let mut orphan_resources: Vec<Value> = Vec::new();

        let mut all_resources = Vec::new();
        Self::gather_resources(&self.state["values"]["root_module"], &mut all_resources);

        if !all_resources.is_empty() {
            for res in all_resources {
                let tf_type = res["type"].as_str().unwrap_or("");
                let values = &res["values"];
                let tf_name = res["name"].as_str().unwrap_or("");
                
                if !self.is_type_enabled(tf_type) {
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
        
        link_projects_to_folders(&project_id_to_parent, &gcp_id_to_yaml_name, &mut project_map, &mut folder_map);
        link_folders_to_parents(&folder_id_to_parent, &gcp_id_to_yaml_name, &mut folder_map);

        if !folder_map.is_empty() { config.folder = Some(folder_map); }
        if !project_map.is_empty() { config.project = Some(project_map); }

        for res in orphan_resources {
            let tf_type = res["type"].as_str().unwrap_or("");
            let values = &res["values"];
            let tf_name = res["name"].as_str().unwrap_or("");
            let schema = self.registry.as_ref().and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s);

            if let Some(p_id) = values["project"].as_str() {
                let p_yaml = gcp_id_to_yaml_name.get(p_id).map(|s| s.as_str()).unwrap_or(p_id);
                if let Some(project) = Self::find_project_mut(&mut config, p_yaml) {
                    self.add_resource_to_project(project, tf_type, tf_name, values, schema);
                }
            } else if let Some(f_id) = values["folder"].as_str() {
                let f_norm = if f_id.starts_with("folders/") { f_id.to_string() } else { format!("folders/{}", f_id) };
                let f_yaml = gcp_id_to_yaml_name.get(&f_norm).map(|s| s.as_str()).unwrap_or(f_id);
                if let Some(folder) = Self::find_folder_mut(&mut config, f_yaml) {
                    self.add_resource_to_folder(folder, tf_type, tf_name, values, schema);
                }
            } else {
                self.add_resource_to_config(&mut config, tf_type, tf_name, values, schema);
            }
        }

        Ok(config)
    }

    pub fn filter_values(tf_type: &str, values: &Value, schema: Option<&ResourceSchema>, add_import_id: bool, exclude: Option<&Vec<String>>) -> serde_yaml::Value {
        let mut yaml_val = serde_yaml::to_value(values).unwrap_or(serde_yaml::Value::Null);
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

        Self::filter_recursive(&mut yaml_val, block_schema, &full_blacklist);

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

    fn filter_recursive(val: &mut serde_yaml::Value, schema: Option<&BlockSchema>, blacklist: &[String]) {
        if let serde_yaml::Value::Mapping(map) = val {
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
                        if let Some(attr) = s.attributes.get(k_str) {
                            if attr.required { return true; }
                            if let Some(default_json) = &attr.default {
                                if let Ok(default_yaml) = serde_yaml::to_value(default_json) {
                                    if v == &default_yaml { return false; }
                                }
                            }
                            if attr.computed && !attr.optional && !attr.required {
                                let keep_computed = ["project_number", "org_id", "folder_id", "project_id"];
                                if !keep_computed.contains(&k_str.as_str()) { return false; }
                            }
                            if attr.optional && !attr.required
                                && Self::is_zero_value(v) { return false; }
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
                let sub_schema = schema.and_then(|s| s.block_types.get(k_str)).map(|bt| &bt.block);
                Self::filter_recursive(v, sub_schema, blacklist);
            }

            map.retain(|_, v| {
                !Self::is_empty_value(v)
            });
        } else if let serde_yaml::Value::Sequence(seq) = val {
             for item in seq.iter_mut() {
                Self::filter_recursive(item, schema, blacklist);
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

    fn is_zero_value(v: &serde_yaml::Value) -> bool {
        match v {
            serde_yaml::Value::Bool(false) => true,
            serde_yaml::Value::String(st) => st.is_empty() || st == "default" || st == "STANDARD",
            serde_yaml::Value::Number(n) => n.as_f64() == Some(0.0) || n.as_i64() == Some(0) || n.as_u64() == Some(0),
            serde_yaml::Value::Sequence(seq) if seq.is_empty() => true,
            serde_yaml::Value::Mapping(m) if m.is_empty() => true,
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

    fn add_resource_to_project(&self, p: &mut Project, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) {
        if tf_type.ends_with("_iam_member") {
            let role = values["role"].as_str().unwrap_or("unknown_role").to_string();
            let member = values["member"].as_str().unwrap_or("unknown_member").to_string();
            if !p.extra.contains_key(tf_type) { p.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(members_map)) = p.extra.get_mut(tf_type) {
                let member_key = serde_yaml::Value::String(member);
                if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                    let role_val = serde_yaml::Value::String(role);
                    if !roles.contains(&role_val) { roles.push(role_val); }
                }
            }
            return;
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None);
        if tf_type == "google_project_service" {
            if p.project_service.is_none() { p.project_service = Some(Vec::new()); }
            p.project_service.as_mut().unwrap().push(yaml_val);
        } else {
            if !p.extra.contains_key(tf_type) { p.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(type_map)) = p.extra.get_mut(tf_type) {
                type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
            }
        }
    }

    fn add_resource_to_folder(&self, f: &mut Folder, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) {
        if tf_type.ends_with("_iam_member") {
            let role = values["role"].as_str().unwrap_or("unknown_role").to_string();
            let member = values["member"].as_str().unwrap_or("unknown_member").to_string();
            if !f.extra.contains_key(tf_type) { f.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
            if let Some(serde_yaml::Value::Mapping(members_map)) = f.extra.get_mut(tf_type) {
                let member_key = serde_yaml::Value::String(member);
                if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                    let role_val = serde_yaml::Value::String(role);
                    if !roles.contains(&role_val) { roles.push(role_val); }
                }
            }
            return;
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None);
        if !f.extra.contains_key(tf_type) { f.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
        if let Some(serde_yaml::Value::Mapping(type_map)) = f.extra.get_mut(tf_type) {
             type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
        }
    }

    fn add_resource_to_config(&self, c: &mut Config, tf_type: &str, tf_name: &str, values: &Value, schema: Option<&ResourceSchema>) {
        if tf_type.ends_with("_iam_member") {
            let role = values["role"].as_str().unwrap_or("unknown_role").to_string();
            let member = values["member"].as_str().unwrap_or("unknown_member").to_string();

            if tf_type == "google_organization_iam_member" {
                if c.organization_iam_member.is_none() { c.organization_iam_member = Some(HashMap::new()); }
                if let Some(ref mut members_map) = c.organization_iam_member {
                    let roles = members_map.entry(member).or_insert_with(Vec::new);
                    let role_val = serde_yaml::Value::String(role);
                    if !roles.contains(&role_val) { roles.push(role_val); }
                }
            } else {
                if !c.extra.contains_key(tf_type) { c.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
                if let Some(serde_yaml::Value::Mapping(members_map)) = c.extra.get_mut(tf_type) {
                    let member_key = serde_yaml::Value::String(member);
                    if !members_map.contains_key(&member_key) { members_map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new())); }
                    if let Some(serde_yaml::Value::Sequence(roles)) = members_map.get_mut(&member_key) {
                        let role_val = serde_yaml::Value::String(role);
                        if !roles.contains(&role_val) { roles.push(role_val); }
                    }
                }
            }
            return;
        }
        let yaml_val = Self::filter_values(tf_type, values, schema, true, None);
        if !c.extra.contains_key(tf_type) { c.extra.insert(tf_type.to_string(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new())); }
        if let Some(serde_yaml::Value::Mapping(type_map)) = c.extra.get_mut(tf_type) {
            type_map.insert(serde_yaml::Value::String(tf_name.to_string()), yaml_val);
        }
    }

    fn gather_resources(module: &Value, all: &mut Vec<Value>) {
        if let Some(resources) = module["resources"].as_array() {
            for res in resources { all.push(res.clone()); }
        }
        if let Some(children) = module["child_modules"].as_array() {
            for child in children { Self::gather_resources(child, all); }
        }
    }

    fn get_asset_scope(asset: &Asset) -> (String, String) {
        let name = &asset.name;
        if name.contains("/projects/") {
            let after = name.split("/projects/").last().unwrap_or("");
            let pid = after.split('/').next().unwrap_or(after).to_string();
            return ("project".to_string(), pid);
        } else if name.contains("/folders/") {
            let after = name.split("/folders/").last().unwrap_or("");
            let fid = after.split('/').next().unwrap_or(after);
            let full_fid = format!("folders/{}", fid);
            return ("folder".to_string(), full_fid);
        } else if name.contains("/organizations/") {
            let after = name.split("/organizations/").last().unwrap_or("");
            let oid = after.split('/').next().unwrap_or(after).to_string();
            return ("organization".to_string(), oid);
        }

        // Fallback: Check ancestors
        if !asset.ancestors.is_empty() {
             for ancestor in &asset.ancestors {
                 if ancestor.starts_with("projects/") {
                     let pid = ancestor.trim_start_matches("projects/");
                     return ("project".to_string(), pid.to_string());
                 } else if ancestor.starts_with("folders/") {
                     return ("folder".to_string(), ancestor.to_string());
                 } else if ancestor.starts_with("organizations/") {
                     let oid = ancestor.trim_start_matches("organizations/");
                     return ("organization".to_string(), oid.to_string());
                 }
             }
        }
        
        ("organization".to_string(), "".to_string())
    }

    /// `parent` is any Cloud Asset Inventory scope: `organizations/<n>`,
    /// `folders/<n>` or `projects/<id>`.
    pub async fn discover_from_org(
        parent: &str,
        verbose: bool,
        discovery_config: Option<ImportConfig>,
        registry: Option<ResourceRegistry>,
    ) -> Result<Config, Box<dyn std::error::Error>> {
        
        let client = AssetService::builder().build().await?;
        
        let mut type_map: BTreeMap<u32, std::collections::BTreeSet<String>> = BTreeMap::new();
        
        if let Some(config) = &discovery_config {
            for resource_config in config.resource_types.values() {
                if !resource_config.import { continue; }
                
                if let (Some(cat), Some(ct)) = (&resource_config.asset_type, &resource_config.content_type) {
                     let ctype_enum = match ct.to_uppercase().as_str() {
                         "IAM_POLICY" => ContentType::IamPolicy,
                         "RESOURCE" | "RESOURCES" => ContentType::Resource,
                         _ => ContentType::Unspecified,
                     };
                     
                     let idx = match ctype_enum {
                         ContentType::Resource => 1,
                         ContentType::IamPolicy => 2,
                         _ => 0,
                     };
                     
                     if idx > 0 {
                         type_map.entry(idx).or_default().insert(cat.clone());
                     }
                }
            }
        }

        let mut all_assets = Vec::new();
        let mut stats: HashMap<String, usize> = HashMap::new();

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
                             
                             let (scope, _scope_id) = Self::get_asset_scope(&asset);

                             if let Some(config) = &discovery_config {
                                  for (tf_type, r_config) in &config.resource_types {
                                      if r_config.asset_type.as_deref() == Some(&asset.asset_type) {
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
                         }
                     }
                 }
            }
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

        let config = Self::construct_config_from_assets(all_assets, verbose, registry.as_ref(), discovery_config.as_ref());

        Ok(config)
    }

    fn construct_config_from_assets(
        assets: Vec<Asset>, 
        _verbose: bool,
        registry: Option<&ResourceRegistry>,
        discovery_config: Option<&ImportConfig>,
    ) -> Config {
        let mut config = Config::default();
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

        // Pass 1: Discover Folders and Projects first to build the hierarchy/map
        for asset in &assets {
             if (asset.asset_type == "cloudresourcemanager.googleapis.com/Folder" || 
                 asset.asset_type == "cloudresourcemanager.googleapis.com/Project") && asset.resource.is_some() {
                 continue;
             }

             let configs = if let Some(v) = asset_type_to_config.get(&asset.asset_type) { v } else { continue; };
             let (tf_type, res_config) = if let Some(found) = configs.iter().find(|(t, c)| (t == "google_folder" || t == "google_project") && c.content_type.as_deref() == Some("RESOURCE")) { found } else { continue; };

             if tf_type == "google_folder" {
                 Self::discover_google_folder(asset, res_config, &mut folder_map, &mut folder_id_to_parent, &mut gcp_id_to_yaml_name);
             } else if tf_type == "google_project" {
                 Self::discover_google_project(asset, res_config, &mut project_map, &mut project_id_to_parent, &mut gcp_id_to_yaml_name);
             }
        }
        
        // Pass 2: Discover other resources
        // Pass 1: Process Folders and Projects first to establish hierarchy and ID mappings
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

             if !res_config.import { continue; }

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

             let configs = if let Some(v) = asset_type_to_config.get(&asset.asset_type) { v } else { continue; };
             
             let (scope, scope_id) = Self::get_asset_scope(asset);

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

             let (tf_type, res_config) = if let Some(found) = matched_config { found } else { continue; };

             if !res_config.import { continue; }

             if res_config.deprecated == Some(true) {
                 deprecated_seen.insert(tf_type.to_string());
             }

             if tf_type.contains("organization_policy") || tf_type == "google_org_policy_policy" {
                 Self::discover_organization_policy(tf_type, asset, res_config, registry, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             } else if asset.iam_policy.is_some() {
                 Self::discover_iam_policy(tf_type, asset, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             } else if tf_type == "google_project_service" {
                 Self::discover_google_project_service(tf_type, asset, res_config, registry, &scope_id, &mut project_map, &gcp_id_to_yaml_name);
             } else {
                 Self::discover_generic_resource(tf_type, asset, res_config, registry, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             }
        }
        
        link_projects_to_folders(&project_id_to_parent, &gcp_id_to_yaml_name, &mut project_map, &mut folder_map);
        link_folders_to_parents(&folder_id_to_parent, &gcp_id_to_yaml_name, &mut folder_map);

        if !folder_map.is_empty() { config.folder = Some(folder_map); }
        if !project_map.is_empty() { config.project = Some(project_map); }
        
        for deprecated_type in deprecated_seen {
            eprintln!("Warning: Resource type '{}' is deprecated.", deprecated_type);
        }

        config
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
                   Self::filter_values(tf_type, &data_val, schema, false, res_config.exclude.as_ref())
               } else {
                   serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
               }
          } else {
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
          };

          if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
               if let Some(p) = project_map.get_mut(p_yaml) {
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
                                     let role_val = serde_yaml::Value::String(role.clone());
                                     if !roles.contains(&role_val) { roles.push(role_val); }
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
                                                let role_val = serde_yaml::Value::String(role.clone());
                                                if !roles.contains(&role_val) { roles.push(role_val); }
                                            }
                                      }
                                 }
                             }
                         } else if scope == "project" {
                             if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
                                 if let Some(p) = project_map.get_mut(p_yaml) {
                                      if tf_type == "google_storage_bucket_iam_member" {
                                          let bucket_name = asset.name.split('/').next_back().unwrap_or("unknown-bucket").to_string();
                                          let member_sanitized = member.replace(":", "_").replace("@", "_").replace(".", "_");
                                          let role_sanitized = role.replace("roles/", "").replace(".", "_");
                                          let key = format!("{}-{}-{}", bucket_name, role_sanitized, member_sanitized);
                                          
                                          let mut resource_map = serde_yaml::Mapping::new();
                                          resource_map.insert(serde_yaml::Value::String("bucket".to_string()), serde_yaml::Value::String(bucket_name));
                                          resource_map.insert(serde_yaml::Value::String("member".to_string()), serde_yaml::Value::String(member.clone()));
                                          resource_map.insert(serde_yaml::Value::String("role".to_string()), serde_yaml::Value::String(role.clone()));
                                          
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
                                                let role_val = serde_yaml::Value::String(role.clone());
                                                if !roles.contains(&role_val) { roles.push(role_val); }
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

    fn discover_generic_resource(
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
                   if let serde_yaml::Value::Mapping(m) = Self::filter_values(tf_type, &data_val, schema, true, res_config.exclude.as_ref()) {
                        resource_val = m;
                   }
               }
          }
          
          if resource_val.is_empty() { return; }
          let policy_map_val = serde_yaml::Value::Mapping(resource_val);

          if scope == "organization" {
               config.extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
               if let Some(serde_yaml::Value::Mapping(m)) = config.extra.get_mut(tf_type) {
                    m.insert(serde_yaml::Value::String(sanitized_key.clone()), policy_map_val);
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
    

    pub fn print_summary(config: &Config, filtered_count: Option<usize>) {
        println!("\n=== Configuration Summary ===");
        if let Some(count) = filtered_count {
            println!("Filtered resources: {}", count);
        }
        
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

/// Nest each discovered project under its parent folder.
///
/// A project can have a surviving parent edge while its own record was filtered out —
/// e.g. Cloud Asset returned the ancestry but the asset itself was excluded by the
/// discovery config, or the caller lacked read access to it. That is normal partial
/// input, so an unresolvable id is reported and skipped rather than panicking.
fn link_projects_to_folders(
    project_id_to_parent: &HashMap<String, String>,
    gcp_id_to_yaml_name: &HashMap<String, String>,
    project_map: &mut HashMap<String, Project>,
    folder_map: &mut HashMap<String, Folder>,
) {
    let project_ids: Vec<String> = project_id_to_parent.keys().cloned().collect();
    for p_id in project_ids {
        let f_id = &project_id_to_parent[&p_id];
        let Some(p_yaml) = gcp_id_to_yaml_name.get(&p_id).cloned() else {
            eprintln!("Warning: skipping project '{}' — it has a parent but no discovered resource record.", p_id);
            continue;
        };

        if let Some(f_yaml) = gcp_id_to_yaml_name.get(f_id) {
            if let (Some(project), Some(folder)) = (project_map.remove(&p_yaml), folder_map.get_mut(f_yaml)) {
                folder.project.get_or_insert_with(HashMap::new).insert(p_yaml, project);
            }
        }
    }
}

/// Nest each discovered folder under its parent folder, deepest first so a child is
/// always moved before the folder containing it. Unresolvable ids are skipped as above.
fn link_folders_to_parents(
    folder_id_to_parent: &HashMap<String, String>,
    gcp_id_to_yaml_name: &HashMap<String, String>,
    folder_map: &mut HashMap<String, Folder>,
) {
    let mut sorted_folder_ids: Vec<String> = folder_id_to_parent.keys().cloned().collect();
    sorted_folder_ids.sort_by_key(|id| std::cmp::Reverse(id.len()));

    for child_id in sorted_folder_ids {
        let parent_id = &folder_id_to_parent[&child_id];
        let Some(child_yaml) = gcp_id_to_yaml_name.get(&child_id).cloned() else {
            eprintln!("Warning: skipping folder '{}' — it has a parent but no discovered resource record.", child_id);
            continue;
        };

        if let Some(parent_yaml) = gcp_id_to_yaml_name.get(parent_id) {
            if let Some(child_folder) = folder_map.remove(&child_yaml) {
                if let Some(parent_folder) = folder_map.get_mut(parent_yaml) {
                    parent_folder.folder.get_or_insert_with(HashMap::new).insert(child_yaml, child_folder);
                } else {
                    folder_map.insert(child_yaml, child_folder);
                }
            }
        }
    }
}
