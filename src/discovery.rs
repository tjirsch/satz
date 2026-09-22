use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};
use serde_json::Value;
use crate::config::{Config, ImportConfig, Folder, Project};
use crate::schema::{ResourceRegistry, ResourceSchema, BlockSchema};
use google_cloud_asset_v1::model::{Asset, ContentType};
use google_cloud_gax::paginator::ItemPaginator;

pub struct Discoverer {
    pub state: Value,
    pub registry: Option<ResourceRegistry>,
    pub enabled_types: Option<HashSet<String>>,
    /// Types the run's `--only` / `--exclude` switched off — reported as
    /// "filtered", not "type off".
    pub filtered_types: HashSet<String>,
    /// What to do with a grant two folders or two projects both hold.
    pub on_collision: OnCollision,
}

/// Asset types per ListAssets request. The quota counts requests
/// ("ListAssets Requests per minute"), so a sweep that asks for one type at a
/// time runs out of it long before `--all` is through; the types travel as
/// query parameters, and a hundred keep the URL near 6 KB.
const ASSET_TYPES_PER_REQUEST: usize = 100;

/// The label the provider stamps on everything it creates when attribution is
/// on. It is the provider's, not the estate's: declared, it is a label nobody
/// chose; left out, the provider puts it back on apply, and a plan that only
/// adds it is not drift.
const ATTRIBUTION_LABEL: &str = "goog-terraform-provisioned";

/// What `satz import` does with a grant edge — one member, one role — that two
/// folders, or two projects, both hold. The map form's emitted label hashes
/// member and role (`iam_member_label`, deliberately not the node, so labels
/// never move), so two such edges emit ONE Terraform address and the transpile
/// refuses. `Error` refuses at import instead, naming the edges and the switch;
/// `Counter` keeps the first edge in the map form and writes each further one
/// as a labelled resource under its own node, with a running number in the
/// label. Import only: a written estate is its author's responsibility.
///
/// The value parser of `satz import --on-collision`: its help lists `error` and
/// `counter`, and clap refuses anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OnCollision {
    #[default]
    Error,
    Counter,
}

/// Why a resource the source had is not in the written estate. An import is
/// allowed to be partial; it is not allowed to be silent about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `import: false` in the import config.
    TypeOff,
    /// Switched off by `--only` / `--exclude` (`only:` / `exclude:`) for this run.
    Filtered,
    /// The source had it, but no import-config row maps it (detail says what
    /// was missing).
    Unmapped(String),
    /// Its project/folder is not in the imported tree, so it has no place.
    ParentNotFound(String),
    /// The platform's, not the estate's: matched a `skip:` pattern on the
    /// import-config row (the built-in `_Default` sink, a service agent's
    /// grant, a Compute default service account). The pattern is named so
    /// the operator can lift it from a copy of the table.
    PlatformOwned(String),
    /// A project Cloud Asset still lists but that is no longer ACTIVE — a
    /// deleted project stays visible for 30 days and nothing can be managed
    /// in it.
    NotActive(String),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::TypeOff => write!(f, "type off (import: false)"),
            SkipReason::Filtered => write!(f, "filtered by --only/--exclude"),
            SkipReason::Unmapped(d) => write!(f, "unmapped: {}", d),
            SkipReason::ParentNotFound(p) => write!(f, "parent not imported: {}", p),
            SkipReason::PlatformOwned(p) => write!(f, "platform-owned: matches skip pattern `{}` on its import-config row", p),
            SkipReason::NotActive(s) => write!(f, "project is {}, not ACTIVE (Cloud Asset lists a deleted project for 30 days)", s),
        }
    }
}

/// The `skip:` pattern on the row that matches `what`, if one does.
fn skip_pattern(res_config: &crate::config::ImportResourceConfig, what: &str) -> Option<String> {
    res_config.skip.iter().flatten().find(|p| crate::config::glob_match(p, what)).cloned()
}

/// The `skip:` pattern a RESOURCE asset matches by what it is called: the
/// last segment of its asset name (`_Default` for a sink), or its `email`,
/// `name` or `displayName` in the asset data — Cloud Asset names a service
/// account by its numeric unique id, and the email that says it is the
/// Compute default account is data.
fn platform_owned(res_config: &crate::config::ImportResourceConfig, asset: &Asset) -> Option<String> {
    res_config.skip.as_ref()?;
    let natural = asset.name.rsplit('/').next().unwrap_or(&asset.name).to_string();
    let mut candidates = vec![natural];
    if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
        for key in ["email", "name", "displayName"] {
            if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
                candidates.push(v.to_string());
            }
        }
    }
    candidates.iter().find_map(|c| skip_pattern(res_config, c))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The Terraform resource type where a row gave the asset one, and the
    /// Cloud Asset type where no row did — the provider schema tells the two
    /// apart (`generate_config::plan`).
    pub tf_type: String,
    /// The live shape names the resource by its Cloud Asset FULL resource name
    /// (`//dns.googleapis.com/projects/p/managedZones/z`), which is what
    /// `asset_resource_name` turns into the provider's import id; the state
    /// shape names it by its Terraform label, which is not one.
    pub what: String,
    pub reason: SkipReason,
}

/// `//logging.googleapis.com/projects/p/sinks/x` → `projects/p/sinks/x`: a Cloud
/// Asset full resource name minus its service, which is the resource name the
/// provider imports by. `None` for anything that is not one.
pub fn asset_resource_name(full_name: &str) -> Option<&str> {
    let path = full_name.strip_prefix("//")?;
    path.split_once('/').map(|(_, p)| p).filter(|p| !p.is_empty())
}

/// What an import produced: the estate, and everything it left out.
pub struct Discovered {
    pub config: Config,
    pub skipped: Vec<Skipped>,
    /// Attributes the source carried and the estate does not, one row each.
    /// Cloud Asset data is API-shaped; a key the Terraform schema lacks would
    /// not plan (roadmap F5). What the schema DOES know and satz still could
    /// not place is a loss, and says so (`DropReason`).
    pub dropped_attrs: Vec<DroppedAttr>,
    /// The organization the assets' ancestors name (live shape only).
    pub organization: Option<String>,
    /// What the import rewrote on the way and says so: one line each.
    pub notes: Vec<String>,
}

/// Why a value the source carried is not in the imported estate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DropReason {
    /// The provider does not speak that name here: API vocabulary with no
    /// Terraform counterpart. Written into HCL it would not plan.
    Vocabulary,
    /// The provider schema names the attribute and the asset data carried a
    /// value for it, and satz did not place it — an apply of the imported
    /// estate resets that attribute on the live resource. The text says what
    /// stopped it.
    NotCarried(String),
}

/// One attribute of one resource that the source had and the estate does not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DroppedAttr {
    pub tf_type: String,
    /// The resource it belongs to, by the name the source gives it.
    pub what: String,
    /// Where it sat, dotted, in the spelling satz got it to.
    pub path: String,
    pub why: DropReason,
}

thread_local! {
    // `filter_values` is called from five places, three of them without a
    // collector in reach; the run is single-threaded, so the dropped-key
    // list is collected here and taken once per import.
    static DROPPED_ATTRS: std::cell::RefCell<Vec<DroppedAttr>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_dropped(tf_type: &str, what: &str, path: &str, why: DropReason) {
    DROPPED_ATTRS.with(|d| {
        d.borrow_mut().push(DroppedAttr {
            tf_type: tf_type.to_string(),
            what: what.to_string(),
            path: path.to_string(),
            why,
        })
    });
}

fn take_dropped() -> Vec<DroppedAttr> {
    let mut v = DROPPED_ATTRS.with(|d| std::mem::take(&mut *d.borrow_mut()));
    v.sort();
    v.dedup();
    v
}

/// Where the API states a fact in other terms than the provider — not another
/// name for the same value, another value. A Cloud Storage lifecycle condition
/// carries `isLive` as a boolean; the provider's `with_state` is `LIVE` /
/// `ARCHIVED`. No name alignment can see this (the two fields share no name),
/// so the pairs stand here, keyed by the type and the path they sit at, and a
/// test reads each from the asset data it comes from.
fn translate_api_values(map: &mut serde_yaml::Mapping, tf_type: &str, at: &str) {
    let rows: &[(&str, &str, &str, &str)] = &[
        // (tf type, path, API key, provider key)
        ("google_storage_bucket", "lifecycle_rule.condition.", "is_live", "with_state"),
    ];
    for (t, path, from, to) in rows {
        if *t != tf_type || *path != at {
            continue;
        }
        let Some(v) = map.remove(serde_yaml::Value::String((*from).to_string())) else { continue };
        let Some(live) = v.as_bool() else { continue };
        map.insert(
            serde_yaml::Value::String((*to).to_string()),
            serde_yaml::Value::String(if live { "LIVE" } else { "ARCHIVED" }.to_string()),
        );
    }
}

/// Does this block take `name` as an attribute, with a value of that shape?
/// An output (computed, neither optional nor required) takes nothing.
fn attribute_takes(block: &BlockSchema, name: &str, v: &serde_yaml::Value) -> bool {
    let Some(a) = block.attributes.get(name) else { return false };
    if a.computed && !a.optional && !a.required {
        return false;
    }
    match a.type_.as_ref() {
        Some(t) if t.is_string() => match t.as_str().unwrap_or("") {
            "string" => v.is_string(),
            "bool" => v.is_bool(),
            "number" => v.is_number(),
            _ => false,
        },
        // `["list","string"]`, `["map","string"]`, `["set","string"]`
        Some(t) if t.is_array() => v.is_sequence() || v.is_mapping(),
        _ => false,
    }
}

/// One scalar or list the API nested: its dotted path in provider spelling,
/// its own name, how many fields sit beside it in its object, and its value.
struct Leaf {
    path: String,
    name: String,
    siblings: usize,
    value: serde_yaml::Value,
}

/// Every scalar or list leaf under `v`, snake_cased on the way. A list is a
/// leaf: what sits inside one is a block's business, not a flattening's.
fn leaves(prefix: &str, v: &serde_yaml::Value, depth: usize, siblings: usize, out: &mut Vec<Leaf>) {
    match v {
        serde_yaml::Value::Mapping(m) if depth < 4 => {
            for (k, v) in m {
                let Some(k) = k.as_str() else { continue };
                let name = crate::align::snake(k);
                let path = if prefix.is_empty() { name } else { format!("{}.{}", prefix, name) };
                leaves(&path, v, depth + 1, m.len(), out);
            }
        }
        _ => {
            let name = prefix.rsplit('.').next().unwrap_or(prefix).to_string();
            out.push(Leaf { path: prefix.to_string(), name, siblings, value: v.clone() });
        }
    }
}

/// A key the provider schema does not name can still CARRY attributes it does:
/// the API nests what Terraform flattens. `iamConfiguration.uniformBucketLevelAccess.enabled`
/// IS the provider's `uniform_bucket_level_access` and `billing.requesterPays`
/// its `requester_pays`; dropped as unknown vocabulary, an apply of the
/// imported estate switches uniform bucket-level access back off.
///
/// The correspondence is the one `align` derives from the API's Discovery
/// Document, read off the data instead, so it needs no generated map: a leaf
/// whose snake_case name is an attribute of this block, or an object of that
/// name holding a single `enabled` / `value`. One candidate carries. Two
/// disagreeing ones carry nothing and say so — satz does not pick.
fn flatten_nested(map: &mut serde_yaml::Mapping, block: &BlockSchema, tf_type: &str, what: &str, at: &str) {
    let unknown: Vec<String> = map
        .iter()
        .filter_map(|(k, v)| k.as_str().filter(|_| v.is_mapping()).map(String::from))
        .filter(|k| !block.attributes.contains_key(k) && !block.block_types.contains_key(k))
        .collect();
    // every nested container at once: two of them claiming one attribute is
    // the same ambiguity as two fields of one container claiming it
    let mut found: Vec<Leaf> = Vec::new();
    for key in &unknown {
        let Some(nested) = map.get(serde_yaml::Value::String(key.clone())).cloned() else { continue };
        let mut under = Vec::new();
        leaves(&format!("{}{}", at, key), &nested, 0, 0, &mut under);
        found.append(&mut under);
    }
    // candidate per leaf: its own name, or — for the `{ enabled: … }` /
    // `{ value: … }` wrapper — the name of the object holding it
    let mut carried: BTreeMap<String, Vec<&Leaf>> = BTreeMap::new();
    for leaf in &found {
        let wrapper = match leaf.path.rsplit_once('.') {
            Some((head, last)) if matches!(last, "enabled" | "value") && leaf.siblings <= 2 => {
                head.rsplit('.').next().map(String::from)
            }
            _ => None,
        };
        for candidate in [Some(leaf.name.clone()), wrapper].into_iter().flatten() {
            if attribute_takes(block, &candidate, &leaf.value) {
                carried.entry(candidate).or_default().push(leaf);
            }
        }
    }
    let mut placed: HashSet<&str> = HashSet::new();
    for (attr, sources) in &carried {
        // the same value from two spellings of one API field (a bucket's
        // `bucketPolicyOnly` mirrors `uniformBucketLevelAccess`) is one value
        let agreed = sources.iter().all(|l| l.value == sources[0].value);
        // the provider's own spelling, at this level, is the value; a nested
        // field that says something else is named, never silently overruled
        let present = map.get(serde_yaml::Value::String(attr.clone())).cloned();
        let why = match (&present, agreed) {
            (Some(v), _) if sources.iter().all(|l| l.value == *v) => None,
            (Some(_), _) => Some(format!(
                "`{}{}` is set at this level and the asset data says otherwise here",
                at, attr
            )),
            (None, false) => Some(format!(
                "`{}{}` is claimed by {} fields of the asset data that disagree: {}",
                at,
                attr,
                sources.len(),
                sources.iter().map(|l| l.path.as_str()).collect::<Vec<_>>().join(", ")
            )),
            (None, true) => None,
        };
        match why {
            Some(why) => {
                for l in sources {
                    note_dropped(tf_type, what, &l.path, DropReason::NotCarried(why.clone()));
                }
            }
            None => {
                if present.is_none() {
                    map.insert(serde_yaml::Value::String(attr.clone()), sources[0].value.clone());
                }
            }
        }
        sources.iter().for_each(|l| {
            placed.insert(l.path.as_str());
        });
    }
    for leaf in &found {
        if !placed.contains(leaf.path.as_str()) {
            note_dropped(tf_type, what, &leaf.path, DropReason::Vocabulary);
        }
    }
    for key in unknown {
        map.remove(serde_yaml::Value::String(key));
    }
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

/// A grant on a scope the map form names in the map (`bucket = …`, language
/// reference §6.5): one map per scope value under the type key, kept as a
/// list of maps because a document holds one key per type. The member's
/// roles join the map for that scope; a new scope opens a new map.
fn push_pinned_grant(
    extra: &mut HashMap<String, serde_yaml::Value>,
    tf_type: &str,
    pin: &str,
    scope_value: &str,
    member: &str,
    role: &str,
    import_id: Option<String>,
) {
    let maps = extra.entry(tf_type.to_string()).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
    let serde_yaml::Value::Sequence(maps) = maps else { return };
    let pin_key = serde_yaml::Value::String(pin.to_string());
    let at = maps.iter().position(|m| m.as_mapping().and_then(|m| m.get(&pin_key)).and_then(|v| v.as_str()) == Some(scope_value));
    let map = match at {
        Some(i) => &mut maps[i],
        None => {
            let mut m = serde_yaml::Mapping::new();
            m.insert(pin_key.clone(), serde_yaml::Value::String(scope_value.to_string()));
            maps.push(serde_yaml::Value::Mapping(m));
            maps.last_mut().expect("just pushed")
        }
    };
    let Some(map) = map.as_mapping_mut() else { return };
    let member_key = serde_yaml::Value::String(member.to_string());
    if !map.contains_key(&member_key) {
        map.insert(member_key.clone(), serde_yaml::Value::Sequence(Vec::new()));
    }
    if let Some(serde_yaml::Value::Sequence(roles)) = map.get_mut(&member_key) {
        push_grant(roles, role, import_id);
    }
}

/// The attribute that names a grant type's scope in the map form, from the
/// provider schema — `bucket`, `service_account_id` — or `None` for a type
/// scoped by the organization, the billing account or the node it is written
/// in, and for one the schema does not single out.
fn pin_attr(registry: Option<&ResourceRegistry>, tf_type: &str) -> Option<String> {
    let schema = registry?.find_resource(tf_type)?.1;
    let required: Vec<&str> = schema.block.attributes.iter().filter(|(_, a)| a.required).map(|(k, _)| k.as_str()).collect();
    match satz_core::pipeline::grant_form(tf_type, &required) {
        satz_core::pipeline::GrantForm::Pinned(attr) => Some(attr),
        _ => None,
    }
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
    pub fn sanitize_asset_key(s: &str) -> String {
        s.to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .replace(['_', ' ', '.'], "-")
    }
    
    pub fn new(
        state_json: Value,
        registry: Option<ResourceRegistry>,
        enabled_types: Option<HashSet<String>>,
        filtered_types: HashSet<String>,
        on_collision: OnCollision,
    ) -> Self {
        Self {
            state: state_json,
            registry,
            enabled_types,
            filtered_types,
            on_collision,
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
                let tf_type = res["type"]
                    .as_str()
                    .ok_or_else(|| format!("state: resource {} has no `type`", res["address"].as_str().unwrap_or("(no address)")))?;
                let values = &res["values"];
                let tf_name = res["name"].as_str().ok_or_else(|| format!("state: a {} has no `name`", tf_type))?;
                
                if !self.is_type_enabled(tf_type) {
                    let reason = if self.filtered_types.contains(tf_type) { SkipReason::Filtered } else { SkipReason::TypeOff };
                    skipped.push(Skipped { tf_type: tf_type.to_string(), what: tf_name.to_string(), reason });
                    continue;
                }

                match tf_type {
                    "google_folder" => {
                        let display_name = values["display_name"].as_str().unwrap_or(tf_name).to_string();
                        let gcp_id = values["name"]
                            .as_str()
                            .ok_or_else(|| format!("state: google_folder {} has no `name`", tf_name))?
                            .to_string(); 
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
                        let project_id = values["project_id"]
                            .as_str()
                            .ok_or_else(|| format!("state: google_project {} has no `project_id`", tf_name))?
                            .to_string();
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
            let tf_type = res["type"]
                .as_str()
                .ok_or_else(|| format!("state: resource {} has no `type`", res["address"].as_str().unwrap_or("(no address)")))?;
            let values = &res["values"];
            let tf_name = res["name"].as_str().ok_or_else(|| format!("state: a {} has no `name`", tf_type))?;
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
        qualify_duplicate_keys(&mut config);
        let notes = resolve_grant_collisions(&mut config, self.on_collision)?;

        Ok(Discovered { config, skipped, dropped_attrs: take_dropped(), organization: None, notes })
    }

    /// `what` names the resource in the report: a value the source carried and
    /// the estate does not is named with the resource it was carried on.
    pub fn filter_values(tf_type: &str, what: &str, values: &Value, schema: Option<&ResourceSchema>, add_import_id: bool, exclude: Option<&Vec<String>>, map: Option<&std::collections::BTreeMap<String, String>>) -> serde_yaml::Value {
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

        Self::filter_recursive(&mut yaml_val, block_schema, &full_blacklist, tf_type, what, "");

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
            // a `project_service` entry: the bare service, or the documented
            // object form `{ service = "…" "import-id" = "…" }` (language
            // reference §6.7) when it carries more — service first
            if let serde_yaml::Value::Mapping(mut map) = yaml_val {
                if let Some(serde_yaml::Value::String(service)) = map.remove(serde_yaml::Value::String("service".to_string())) {
                    if map.is_empty() {
                        return serde_yaml::Value::String(service);
                    } else {
                        let mut new_map = serde_yaml::Mapping::new();
                        new_map.insert("service".into(), serde_yaml::Value::String(service));
                        new_map.extend(map);
                        return serde_yaml::Value::Mapping(new_map);
                    }
                }
                return serde_yaml::Value::Mapping(map);
            }
        }
        yaml_val
    }


    fn filter_recursive(val: &mut serde_yaml::Value, schema: Option<&BlockSchema>, blacklist: &[String], tf_type: &str, what: &str, at: &str) {
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
            translate_api_values(map, tf_type, at);
            for key in blacklist {
                map.remove(serde_yaml::Value::String(key.to_string()));
            }

            let label_keys = ["labels", "terraform_labels", "effective_labels"];
            for l_key in label_keys {
                if let Some(serde_yaml::Value::Mapping(labels)) = map.get_mut(serde_yaml::Value::String(l_key.to_string())) {
                    labels.remove(serde_yaml::Value::String(ATTRIBUTION_LABEL.to_string()));
                }
            }

            if let Some(s) = schema {
                flatten_nested(map, s, tf_type, what, at);
                map.retain(|k, v| {
                    if let serde_yaml::Value::String(k_str) = k {
                        // A key neither the attributes nor the blocks know is
                        // API vocabulary the provider does not speak (F5):
                        // it would not plan, so it goes — and is reported.
                        if !s.attributes.contains_key(k_str) && !s.block_types.contains_key(k_str) {
                            note_dropped(tf_type, what, &format!("{}{}", at, k_str), DropReason::Vocabulary);
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
                // The API speaks in self-links and full resource names where the
                // provider takes the short form: a subnet's `region` arrives as
                // `https://www.googleapis.com/compute/v1/projects/p/regions/europe-west3`
                // (provider: `europe-west3`), a topic's `name` as
                // `projects/p/topics/t` (provider: `t`). Left as is, the plan
                // forces a replacement on import. Only `region`/`zone`/`name`
                // are shortened — a `network`/`subnetwork` reference is a valid
                // provider value as a self-link and stays.
                if let serde_yaml::Value::String(s) = v {
                    let shorten = match k_str {
                        "region" | "zone" => s.starts_with("https://") || s.contains('/'),
                        "name" => at.is_empty() && s.starts_with("projects/") && s.matches('/').count() >= 3,
                        _ => false,
                    };
                    if shorten {
                        if let Some(last) = s.rsplit('/').next() {
                            *v = serde_yaml::Value::String(last.to_string());
                        }
                        continue;
                    }
                }
                let sub_schema = schema.and_then(|s| s.block_types.get(k_str)).map(|bt| &bt.block);
                Self::filter_recursive(v, sub_schema, blacklist, tf_type, what, &format!("{}{}.", at, k_str));
            }

            map.retain(|_, v| {
                !Self::is_empty_value(v)
            });
        } else if let serde_yaml::Value::Sequence(seq) = val {
             for item in seq.iter_mut() {
                Self::filter_recursive(item, schema, blacklist, tf_type, what, at);
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
            // a scope the map names in the map (`bucket = …`): one map per scope
            if let Some(pin) = pin_attr(self.registry.as_ref(), tf_type) {
                let Some(scope_value) = values[pin.as_str()].as_str().filter(|v| !v.is_empty()) else {
                    return Err(format!("state: {} `{}` has no `{}`", tf_type, tf_name, pin));
                };
                let id = grant_import_id(tf_type, scope_value, &role, &member);
                push_pinned_grant(&mut p.extra, tf_type, &pin, scope_value, &member, &role, Some(id));
                return Ok(());
            }
            let parent = p.project_id.clone();
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
        let yaml_val = Self::filter_values(tf_type, tf_name, values, schema, true, None, None);
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
        let yaml_val = Self::filter_values(tf_type, tf_name, values, schema, true, None, None);
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
        let yaml_val = Self::filter_values(tf_type, tf_name, values, schema, true, None, None);
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
    /// `folders/<n>` or `projects/<id>`. The enabled types are fetched
    /// `ASSET_TYPES_PER_REQUEST` at a time.
    pub async fn discover_from_org(
        parent: &str,
        verbose: bool,
        discovery_config: Option<ImportConfig>,
        registry: Option<&ResourceRegistry>,
        on_collision: OnCollision,
    ) -> Result<Discovered, Box<dyn std::error::Error>> {
        use google_cloud_gax::options::RequestOptionsBuilder;
        let client = crate::gcp::asset_service().await?;
        let quota_project = crate::org_policy::resolve_quota_project();

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
            let asset_types: Vec<String> = asset_types.into_iter().collect();
            for batch in asset_types.chunks(ASSET_TYPES_PER_REQUEST) {
                 println!("Fetching assets: {} type(s) (Content: {:?})", batch.len(), ctype);
                 if verbose {
                     for t in batch { println!("  {}", t); }
                 }
                 let what = match batch {
                     [one] => one.clone(),
                     _ => format!("{} type(s) {} … {}", batch.len(), batch[0], batch[batch.len() - 1]),
                 };

                 // Same quota project every other Cloud Asset sweep sends; without
                 // it a credential with no default quota project is refused.
                 let mut builder = client.list_assets()
                    .set_parent(parent.to_string())
                    .set_asset_types(batch.to_vec())
                    .set_content_type(ctype.clone())
                    .set_page_size(1000);
                 if let Some(qp) = &quota_project {
                     builder = builder.with_quota_project(qp);
                 }
                 let mut stream = builder.by_item();
                
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
                             eprintln!("Error fetching {}: {}", what, e);
                             fetch_errors.push(format!("{}: {}", what, e));
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
                "import aborted — {} request(s) failed, nothing written:\n  {}\n\
                 An asset type ListAssets refuses is named in the message: leave its row out with \
                 --exclude, and correct the table with scripts/update_import_config.py --probe.",
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
        let (mut config, mut skipped) = Self::construct_config_from_assets(all_assets, registry, discovery_config.as_ref())?;
        qualify_duplicate_keys(&mut config);
        let notes = resolve_grant_collisions(&mut config, on_collision)?;
        for (tf_type, name) in unscoped {
            skipped.push(Skipped {
                tf_type,
                what: name,
                reason: SkipReason::Unmapped("no organization/folder/project scope in the asset name or its ancestors".into()),
            });
        }

        Ok(Discovered { config, skipped, dropped_attrs: take_dropped(), organization, notes })
    }

    fn construct_config_from_assets(
        assets: Vec<Asset>,
        registry: Option<&ResourceRegistry>,
        discovery_config: Option<&ImportConfig>,
    ) -> Result<(Config, Vec<Skipped>), String> {
        let mut config = Config::default();
        let mut skipped: Vec<Skipped> = Vec::new();
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
                 if let Err(reason) = Self::discover_google_project(asset, res_config, &mut project_map, &mut project_id_to_parent, &mut gcp_id_to_yaml_name) {
                     skipped.push(Skipped { tf_type: tf_type.clone(), what: asset.name.clone(), reason });
                 }
             }
        }
        label_folders_by_display_name(&mut folder_map, &mut gcp_id_to_yaml_name);

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

             if tf_type == "google_org_policy_policy" {
                 Self::discover_organization_policy(tf_type, asset, res_config, registry, &scope, &scope_id, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
             } else if asset.iam_policy.is_some() {
                 Self::discover_iam_policy(tf_type, asset, res_config, registry, &scope, &scope_id, &mut skipped, Sinks { config: &mut config, folder_map: &mut folder_map, project_map: &mut project_map, gcp_id_to_yaml_name: &gcp_id_to_yaml_name });
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

    /// A project that is no longer ACTIVE is skipped with its state: Cloud
    /// Asset lists a deleted project for 30 days, nothing in it can be
    /// managed, and one chosen as the providers' quota project fails every
    /// organization-scoped read.
    fn discover_google_project(
        asset: &Asset,
        res_config: &crate::config::ImportResourceConfig,
        project_map: &mut HashMap<String, Project>,
        project_id_to_parent: &mut HashMap<String, String>,
        gcp_id_to_yaml_name: &mut HashMap<String, String>,
    ) -> Result<(), SkipReason> {
         let name = &asset.name;
         if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
             let state = data.get("lifecycleState").or_else(|| data.get("state")).and_then(|v| v.as_str());
             if let Some(state) = state.filter(|s| *s != "ACTIVE") {
                 return Err(SkipReason::NotActive(state.to_string()));
             }
         }
         let yaml_key_raw = if let Some(field) = &res_config.derive_yaml_key_from {
              if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
                   data.get(field).and_then(|v| v.as_str()).unwrap_or(name).to_string()
              } else { name.clone() }
         } else { name.clone() };
         let yaml_key = Self::sanitize_asset_key(&yaml_key_raw);

         let parts: Vec<&str> = name.split("/projects/").collect();
         if parts.len() < 2 { return Ok(()); }
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
             if let Some(l_map) = data.get("labels").and_then(|v| v.as_object()) {
                 labels = declared_labels(l_map);
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
          Ok(())
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
                   Self::filter_values(tf_type, &service_name, &data_val, schema, false, res_config.exclude.as_ref(), res_config.map.as_ref())
               } else {
                   serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
               }
          } else {
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
          };

          if let Some(p_yaml) = gcp_id_to_yaml_name.get(scope_id) {
               if let Some(p) = project_map.get_mut(p_yaml) {
                    // The service's import id is `<project>/<service>` (adopt's
                    // rule), in the documented object form
                    // `{ service = "…" "import-id" = "…" }` (language reference
                    // §6.7), which the printer writes on one line;
                    // `filter_values` gave a bare service as a string and one
                    // with more attributes as `{ service, … }`.
                    let id = serde_yaml::Value::String(format!("{}/{}", p.project_id, service_name));
                    let mut entry = serde_yaml::Mapping::new();
                    entry.insert("service".into(), serde_yaml::Value::String(service_name.clone()));
                    entry.insert("import-id".into(), id);
                    if let serde_yaml::Value::Mapping(m) = resource_val {
                        for (k, v) in m {
                            if k.as_str() != Some("service") {
                                entry.insert(k, v);
                            }
                        }
                    }
                    let resource_val = serde_yaml::Value::Mapping(entry);
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
          
          // The constraint is the policy's identity. Its address is the
          // library's spelling — dots to dashes, case kept
          // (`compute-managed-vmExternalIpAccess`) — so a pack that later
          // carries the same policy replaces this block by address.
          let constraint = name.rsplit("/policies/").next().unwrap_or(name);
          let sanitized_key = if res_config.derive_yaml_key_from.as_deref() == Some("name") && name.contains("/policies/") {
              constraint.replace('.', "-")
          } else {
              Self::sanitize_asset_key(name)
          };
          let mut resource_val = serde_yaml::Mapping::new();

          if let Some(reg) = registry {
                if let Some((_, schema)) = reg.find_resource(tf_type) {
                     if let Some(map) = Self::process_organization_policy_family(tf_type, asset, schema, name, scope_id) {
                          resource_val = map;
                     }
                }
          }

          if !resource_val.is_empty() {
                    // the import id is the full resource name; the block's
                    // `name` is the bare constraint the emitter expands
                    // (transformation 9), and `parent` is the enclosing scope
                    let import_id = name.find("organizations/").or_else(|| name.find("folders/")).or_else(|| name.find("projects/")).map(|i| name[i..].to_string());
                    let old_map = std::mem::replace(&mut resource_val, serde_yaml::Mapping::new());
                    if let Some(id) = import_id {
                        resource_val.insert(serde_yaml::Value::String("import-id".to_string()), serde_yaml::Value::String(id));
                    }
                    for (k, v) in old_map {
                        if k.as_str() == Some("parent") {
                            continue;
                        }
                        resource_val.insert(k, v);
                    }
               }
          
          if resource_val.is_empty() { return; }

          let policy_map_val = serde_yaml::Value::Mapping(resource_val);

          if scope == "organization" {
              if tf_type == "google_org_policy_policy" {
                   if config.org_policy_policy.is_none() { config.org_policy_policy = Some(HashMap::new()); }
                   config.org_policy_policy.as_mut().unwrap().insert(sanitized_key.clone(), policy_map_val);
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

    #[allow(clippy::too_many_arguments)]
    fn discover_iam_policy(
         tf_type: &str,
         asset: &Asset,
         res_config: &crate::config::ImportResourceConfig,
         registry: Option<&ResourceRegistry>,
         scope: &str,
         scope_id: &str,
         skipped: &mut Vec<Skipped>,
         sinks: Sinks<'_>,
    ) {
         let Sinks { config, folder_map, project_map, gcp_id_to_yaml_name } = sinks;
         // a grant on a folder/project that is not in the tree (skipped, or
         // outside the sweep) has no place — say so, per policy
         if scope != "organization" && !gcp_id_to_yaml_name.contains_key(scope_id) {
             skipped.push(Skipped { tf_type: tf_type.to_string(), what: asset.name.clone(), reason: SkipReason::ParentNotFound(scope_id.to_string()) });
             return;
         }
         if let Some(iam) = &asset.iam_policy {
             for binding in &iam.bindings {
                 if !binding.members.is_empty() {
                     for member in &binding.members {
                         let role = &binding.role;
                         if let Some(pattern) = skip_pattern(res_config, member) {
                             skipped.push(Skipped {
                                 tf_type: tf_type.to_string(),
                                 what: format!("{} {} on {}", member, role, scope_id),
                                 reason: SkipReason::PlatformOwned(pattern),
                             });
                             continue;
                         }
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
                                      if let Some(pin) = pin_attr(registry, tf_type) {
                                          // a bucket's grant names the bucket; any other
                                          // pinned type names its scope by the asset path
                                          let scope_value = if tf_type == "google_storage_bucket_iam_member" {
                                              asset.name.split('/').next_back().unwrap_or("unknown-bucket").to_string()
                                          } else {
                                              Self::asset_path(asset).to_string()
                                          };
                                          let id = grant_import_id(tf_type, &scope_value, role, member);
                                          push_pinned_grant(&mut p.extra, tf_type, &pin, &scope_value, member, role, Some(id));
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
          // the platform's own: the built-in sinks, a default service account
          if let Some(pattern) = platform_owned(res_config, asset) {
              return Err(SkipReason::PlatformOwned(pattern));
          }
          let raw_key = if let Some(field) = &res_config.derive_yaml_key_from {
               if let Some(data) = asset.resource.as_ref().and_then(|r| r.data.as_ref()) {
                    data.get(field).and_then(|v| v.as_str()).unwrap_or(name).to_string()
               } else { name.clone() }
          } else { name.clone() };
          
          let sanitized_key = Self::sanitize_asset_key(&raw_key.to_string());
          
          let mut resource_val = serde_yaml::Mapping::new();
          
          if let Some(resource) = &asset.resource {
               if let Some(data) = &resource.data {
                   let schema = registry.and_then(|r| r.find_resource(tf_type)).map(|(_, s)| s);
                   let data_val = serde_json::Value::Object(data.clone());
                   // `add_import_id: false` — the state shape's `id` is the
                   // provider's import id, but Cloud Asset data carries the API's
                   // own `id` (compute: a bare number) which the provider does
                   // NOT import by; the asset path below is the import id here
                   if let serde_yaml::Value::Mapping(m) = Self::filter_values(tf_type, &raw_key, &data_val, schema, false, res_config.exclude.as_ref(), res_config.map.as_ref()) {
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
          {
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
              // The row's `import_id` template is the provider's own import
              // format and wins where it renders (`{project} {name}` for a log
              // metric — not a path at all); the asset path is the fallback
              // for rows without one.
              let import_id = res_config
                  .import_id
                  .as_deref()
                  .and_then(|t| Self::render_import_template(t, &resource_val, project_id.as_deref()))
                  .unwrap_or(path);
              let mut with_id = serde_yaml::Mapping::new();
              with_id.insert(id_key, serde_yaml::Value::String(import_id));
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

    /// Render an `import_id` template from the resource's own values:
    /// `{project}` is the containing project id, every other `{key}` the
    /// attribute of that name. `None` when a placeholder has no value — the
    /// caller falls back to the asset path rather than emitting a half id.
    fn render_import_template(template: &str, values: &serde_yaml::Mapping, project_id: Option<&str>) -> Option<String> {
        let mut out = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let end = rest[start..].find('}')?;
            let key = &rest[start + 1..start + end];
            let v = if key == "project" {
                project_id.map(str::to_string)
            } else {
                values.get(serde_yaml::Value::String(key.to_string())).and_then(|v| match v {
                    serde_yaml::Value::String(s) => Some(s.clone()),
                    serde_yaml::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            }?;
            out.push_str(&v);
            rest = &rest[start + end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// `//logging.googleapis.com/projects/p/sinks/x` → `projects/p/sinks/x`
    fn asset_path(asset: &Asset) -> &str {
        asset_resource_name(&asset.name).unwrap_or_else(|| asset.name.trim_start_matches("//"))
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
                // Regional/located resources carry their location in the asset
                // name: `…/locations/<location>/…`, `…/regions/<region>/…`.
                "location" => segs.iter().position(|s| *s == "locations").and_then(|i| segs.get(i + 1)).map(|s| s.to_string()),
                "region" => segs.iter().position(|s| *s == "regions").and_then(|i| segs.get(i + 1)).map(|s| s.to_string()),
                // A resource's own user-chosen id is the last segment of its
                // asset name: `…/secrets/<secret_id>`,
                // `…/repositories/<repository_id>`. Only the `*_id` shape —
                // `name` is usually in the data already, and where it is not
                // it may be computed (a full resource name), not user-chosen.
                k if k.ends_with("_id") && segs.len() >= 2 => segs.last().map(|s| s.to_string()),
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
             
             // `name` is the bare constraint (`compute.managed.requireOsLogin`):
             // the emitter expands it to the full resource name under the
             // enclosing scope (language reference, transformation 9), and
             // derives `parent` from that scope, so neither is written
             let constraint = name.rsplit("/policies/").next().unwrap_or(name);
             data_map.insert("name".to_string(), serde_json::Value::String(constraint.to_string()));
             data_map.remove("parent");
             let _ = scope_part;

         } else {
             // the caller dispatches on the type, and this is the only org-policy
             // type the importer reads
             unreachable!("process_organization_policy_family called for {}", tf_type);
         }

         let extracted = schema.block.extract_attributes(&data_map, tf_type, name);
         
         if extracted.is_empty() {
             None
         } else {
             Some(extracted)
         }
    }
}

/// The label a discovered folder is written under: its display name as an
/// identifier (`Infrastructure` → `infrastructure`, `satz-preflight scratch`
/// → `satz_preflight_scratch`), the folder number appended only where two
/// folders share a display name. Discovery keys folders `folder-<n>` while
/// the sweep runs (unique by construction, and what every pass-2 lookup
/// resolves by), so the renaming is one pass over both maps once every
/// folder is known. The `"import-id"` still carries `folders/<n>`.
fn label_folders_by_display_name(folder_map: &mut HashMap<String, Folder>, gcp_id_to_yaml_name: &mut HashMap<String, String>) {
    fn identifier(display_name: &str) -> String {
        let mut out: String = display_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        while out.contains("__") {
            out = out.replace("__", "_");
        }
        let out = out.trim_matches('_').to_string();
        if out.is_empty() || out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            format!("folder_{}", out)
        } else {
            out
        }
    }
    let mut keys: Vec<String> = folder_map.keys().cloned().collect();
    keys.sort();
    let mut by_label: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in &keys {
        by_label.entry(identifier(&folder_map[key].display_name)).or_default().push(key.clone());
    }
    let mut renames: Vec<(String, String)> = Vec::new();
    for (label, olds) in by_label {
        let unique = olds.len() == 1;
        for old in olds {
            let new = if unique {
                label.clone()
            } else {
                format!("{}_{}", label, old.trim_start_matches("folder-"))
            };
            renames.push((old, new));
        }
    }
    for (old, new) in renames {
        if old == new {
            continue;
        }
        if let Some(f) = folder_map.remove(&old) {
            folder_map.insert(new.clone(), f);
        }
        for v in gcp_id_to_yaml_name.values_mut() {
            if *v == old {
                *v = new.clone();
            }
        }
    }
}

/// A project's labels as the estate declares them: the string-valued ones,
/// minus the provider's attribution label, which `filter_recursive` strips
/// from every other type for the same reason.
fn declared_labels(labels: &serde_json::Map<String, serde_json::Value>) -> Option<HashMap<String, String>> {
    let extracted: HashMap<String, String> = labels
        .iter()
        .filter(|(k, _)| k.as_str() != ATTRIBUTION_LABEL)
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();
    if extracted.is_empty() { None } else { Some(extracted) }
}

/// A type whose map is keyed by MEMBER (`"user:…" = [roles]`) rather than by
/// address — satz-core's own rule, so the two never drift. Those keys are
/// never qualified: a member is the same principal in every container.
fn is_grant_map(tf_type: &str) -> bool {
    satz_core::pipeline::type_facts(tf_type).0 == satz_core::MergeClass::Grant
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
///
/// A project whose parent IS the organization stays at the top level with no
/// parent attribute: the emitter derives `org_id` there. It used to fall into
/// the "outside the sweep" arm and come out as `folder_id = "organizations/…"`,
/// which the provider refuses.
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
        if !f_id.starts_with("folders/") {
            continue; // organization root: the emitter derives it
        }
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
/// in the sweep keeps that parent explicitly, for the same reason as above. A
/// folder whose parent is the organization carries none: the top level IS that
/// parent (language reference §6.6) and the emitter derives it.
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
            // organization root: the emitter derives it, so the parent the
            // asset was discovered with must not be written beside it
            if let Some(root) = folder_map.get_mut(&child_yaml) {
                root.parent = None;
            }
            continue;
        }
        match gcp_id_to_yaml_name.get(parent_id) {
            Some(parent_yaml) => {
                let Some(mut child_folder) = folder_map.remove(&child_yaml) else {
                    return Err(format!("import: folder {} ({}) has a parent but no record to move", child_id, child_yaml));
                };
                // the nesting IS the parent; declared beside it, the emitter
                // refuses the folder
                child_folder.parent = None;
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

/// A Satz address is `<type>.<key>` across the whole estate, and two projects
/// hold resources of the same name all the time (every project has a
/// `_Default` log sink). Where one key is used by a type more than once, the
/// copies inside a folder or project take that container's name as a prefix;
/// the one at the organisation keeps the plain key. Without this the fold
/// refuses the imported estate, naming both lines.
///
/// A grant map is keyed by member, not by address, and is left alone: one
/// principal granted on two folders used to come out as
/// `"folder-<n>-user:x@y"`, a member that exists nowhere, while its import id
/// still named the real one — the plan imported the binding and then replaced it.
fn qualify_duplicate_keys(config: &mut Config) {
    fn count(extra: &HashMap<String, serde_yaml::Value>, seen: &mut BTreeMap<String, BTreeMap<String, usize>>) {
        for (tf_type, val) in extra {
            if is_grant_map(tf_type) {
                continue;
            }
            if let serde_yaml::Value::Mapping(m) = val {
                for k in m.keys().filter_map(|k| k.as_str()) {
                    *seen.entry(tf_type.clone()).or_default().entry(k.to_string()).or_default() += 1;
                }
            }
        }
    }
    fn count_folder(f: &Folder, seen: &mut BTreeMap<String, BTreeMap<String, usize>>) {
        count(&f.extra, seen);
        for sub in f.folder.iter().flat_map(|m| m.values()) {
            count_folder(sub, seen);
        }
        for p in f.project.iter().flat_map(|m| m.values()) {
            count(&p.extra, seen);
        }
    }
    fn qualify(
        extra: &mut HashMap<String, serde_yaml::Value>,
        container: &str,
        seen: &BTreeMap<String, BTreeMap<String, usize>>,
        taken: &mut BTreeMap<String, BTreeSet<String>>,
    ) {
        for (tf_type, val) in extra.iter_mut() {
            let Some(dupes) = seen.get(tf_type) else { continue };
            let serde_yaml::Value::Mapping(m) = val else { continue };
            let keys: Vec<String> = m.keys().filter_map(|k| k.as_str()).map(str::to_string).collect();
            for key in keys {
                if dupes.get(&key).copied().unwrap_or(0) < 2 {
                    continue;
                }
                let used = taken.entry(tf_type.clone()).or_default();
                let mut name = format!("{}-{}", container, key);
                let mut n = 2;
                while used.contains(&name) || dupes.contains_key(&name) {
                    name = format!("{}-{}-{}", container, key, n);
                    n += 1;
                }
                used.insert(name.clone());
                if let Some(v) = m.remove(serde_yaml::Value::String(key)) {
                    m.insert(serde_yaml::Value::String(name), v);
                }
            }
        }
    }
    fn qualify_folder(
        f: &mut Folder,
        label: &str,
        seen: &BTreeMap<String, BTreeMap<String, usize>>,
        taken: &mut BTreeMap<String, BTreeSet<String>>,
    ) {
        qualify(&mut f.extra, label, seen, taken);
        for (name, sub) in f.folder.iter_mut().flat_map(|m| m.iter_mut()) {
            let name = name.clone();
            qualify_folder(sub, &name, seen, taken);
        }
        for (name, p) in f.project.iter_mut().flat_map(|m| m.iter_mut()) {
            let name = name.clone();
            qualify(&mut p.extra, &name, seen, taken);
        }
    }

    let mut seen: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    count(&config.extra, &mut seen);
    for f in config.folder.iter().flat_map(|m| m.values()) {
        count_folder(f, &mut seen);
    }
    for p in config.project.iter().flat_map(|m| m.values()) {
        count(&p.extra, &mut seen);
    }
    if !seen.values().any(|keys| keys.values().any(|n| *n > 1)) {
        return;
    }
    let mut taken: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, f) in config.folder.iter_mut().flat_map(|m| m.iter_mut()) {
        let name = name.clone();
        qualify_folder(f, &name, &seen, &mut taken);
    }
    for (name, p) in config.project.iter_mut().flat_map(|m| m.iter_mut()) {
        let name = name.clone();
        qualify(&mut p.extra, &name, &seen, &mut taken);
    }
}

/// Visit every folder and project of a discovered estate in one fixed order —
/// folders before projects, siblings by key, a folder before its children —
/// with the node's path and its resources. The order is what makes "the first
/// node keeps the map form" reproducible from one run to the next.
fn visit_nodes(config: &mut Config, f: &mut dyn FnMut(&str, &mut HashMap<String, serde_yaml::Value>)) {
    fn visit_folder(path: &str, fo: &mut Folder, f: &mut dyn FnMut(&str, &mut HashMap<String, serde_yaml::Value>)) {
        f(path, &mut fo.extra);
        if let Some(subs) = fo.folder.as_mut() {
            let mut keys: Vec<String> = subs.keys().cloned().collect();
            keys.sort();
            for k in keys {
                visit_folder(&format!("{}/{}", path, k), subs.get_mut(&k).expect("key from keys()"), f);
            }
        }
        if let Some(projects) = fo.project.as_mut() {
            let mut keys: Vec<String> = projects.keys().cloned().collect();
            keys.sort();
            for k in keys {
                f(&format!("{}/{}", path, k), &mut projects.get_mut(&k).expect("key from keys()").extra);
            }
        }
    }
    if let Some(folders) = config.folder.as_mut() {
        let mut keys: Vec<String> = folders.keys().cloned().collect();
        keys.sort();
        for k in keys {
            visit_folder(&k, folders.get_mut(&k).expect("key from keys()"), f);
        }
    }
    if let Some(projects) = config.project.as_mut() {
        let mut keys: Vec<String> = projects.keys().cloned().collect();
        keys.sort();
        for k in keys {
            f(&k, &mut projects.get_mut(&k).expect("key from keys()").extra);
        }
    }
}

fn identifier_from(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// The readable label a counted grant is written under, before its number:
/// the role's last segment and the member's local part
/// (`folderAdmin_alice` for `roles/resourcemanager.folderAdmin` and
/// `user:alice@example.com`).
fn grant_label_base(role: &str, member: &str) -> String {
    let role_short = role.rsplit(['/', '.']).next().unwrap_or(role);
    let local = member.split_once(':').map(|(_, v)| v).unwrap_or(member);
    let local = local.split('@').next().unwrap_or(local);
    format!("{}_{}", identifier_from(role_short), identifier_from(local))
}

fn grant_entry_import_id(v: &serde_yaml::Value) -> Option<String> {
    v.as_mapping().and_then(|m| m.get("import-id")).and_then(|id| id.as_str()).map(String::from)
}

/// Grant edges that two nodes of the same kind both hold, resolved per
/// [`OnCollision`]. Returns one note per edge rewritten; `Error` returns the
/// full list as the error.
pub fn resolve_grant_collisions(config: &mut Config, mode: OnCollision) -> Result<Vec<String>, String> {
    type Edge = (String, String, String); // (type, member, role)
    // pass 1: every edge with the nodes holding it, in visiting order, and
    // every label already taken per type (a labelled grant is an address)
    let mut sites: BTreeMap<Edge, Vec<String>> = BTreeMap::new();
    let mut taken: BTreeSet<(String, String)> = BTreeSet::new();
    visit_nodes(config, &mut |node, extra| {
        for (tf_type, val) in extra.iter() {
            if !is_grant_map(tf_type) {
                continue;
            }
            let Some(m) = val.as_mapping() else { continue };
            for (k, v) in m {
                let Some(key) = k.as_str() else { continue };
                match v {
                    serde_yaml::Value::Sequence(roles) => {
                        for r in roles {
                            if let Some(role) = grant_role(r) {
                                sites.entry((tf_type.clone(), key.to_string(), role.to_string())).or_default().push(node.to_string());
                            }
                        }
                    }
                    _ => {
                        taken.insert((tf_type.clone(), key.to_string()));
                    }
                }
            }
        }
    });
    let colliding: BTreeMap<Edge, Vec<String>> = sites.into_iter().filter(|(_, nodes)| nodes.len() > 1).collect();
    if colliding.is_empty() {
        return Ok(Vec::new());
    }
    if mode == OnCollision::Error {
        let mut msg = format!(
            "import: {} grant(s) held on more than one folder/project would emit one address each — the map form's label is member + role:\n",
            colliding.len()
        );
        for ((tf_type, member, role), nodes) in &colliding {
            msg.push_str(&format!("  {} {} {} on {}\n", tf_type, member, role, nodes.join(", ")));
        }
        msg.push_str(
            "`--on-collision counter` keeps the first in the map form and writes the others as labelled resources with a running number; or edit the estate by hand",
        );
        return Err(msg);
    }
    // pass 2: every node but the first gives the edge up to a labelled resource
    let first_of: BTreeMap<Edge, String> = colliding.into_iter().map(|(e, nodes)| (e, nodes[0].clone())).collect();
    let mut notes = Vec::new();
    visit_nodes(config, &mut |node, extra| {
        for (tf_type, val) in extra.iter_mut() {
            if !is_grant_map(tf_type) {
                continue;
            }
            let Some(m) = val.as_mapping_mut() else { continue };
            let mut inserts: Vec<(String, serde_yaml::Mapping)> = Vec::new();
            let mut emptied: Vec<serde_yaml::Value> = Vec::new();
            for (k, v) in m.iter_mut() {
                let (Some(member), serde_yaml::Value::Sequence(roles)) = (k.as_str().map(str::to_string), v) else { continue };
                let mut moved: Vec<(String, Option<String>)> = Vec::new();
                roles.retain(|r| {
                    let Some(role) = grant_role(r) else { return true };
                    match first_of.get(&(tf_type.clone(), member.clone(), role.to_string())) {
                        Some(first) if first != node => {
                            moved.push((role.to_string(), grant_entry_import_id(r)));
                            false
                        }
                        _ => true,
                    }
                });
                for (role, id) in moved {
                    let base = grant_label_base(&role, &member);
                    let mut n = 2;
                    let mut label = format!("{}_{}", base, n);
                    while !taken.insert((tf_type.clone(), label.clone())) {
                        n += 1;
                        label = format!("{}_{}", base, n);
                    }
                    let first = &first_of[&(tf_type.clone(), member.clone(), role.clone())];
                    notes.push(format!(
                        "import: {} {} on {} is written as {}.{} — {} holds the same grant, and the map form has one address per member and role",
                        member, role, node, tf_type, label, first
                    ));
                    let mut body = serde_yaml::Mapping::new();
                    if let Some(id) = id {
                        body.insert("import-id".into(), serde_yaml::Value::String(id));
                    }
                    body.insert("role".into(), serde_yaml::Value::String(role));
                    body.insert("member".into(), serde_yaml::Value::String(member.clone()));
                    inserts.push((label, body));
                }
                if roles.is_empty() {
                    emptied.push(k.clone());
                }
            }
            for k in emptied {
                m.remove(k);
            }
            for (label, body) in inserts {
                m.insert(serde_yaml::Value::String(label), serde_yaml::Value::Mapping(body));
            }
        }
    });
    Ok(notes)
}

/// The Terraform type of a parent a skipped resource named: `project <id>` and
/// `projects/<id>` are projects, `folders/<n>` a folder. An organisation is never
/// filtered out, so it has none.
fn parent_type_of(parent: &str) -> Option<&'static str> {
    if parent.starts_with("project ") || parent.starts_with("projects/") {
        Some("google_project")
    } else if parent.starts_with("folders/") {
        Some("google_folder")
    } else {
        None
    }
}

/// Resources dropped because their parent is not in the estate, where the parent's
/// TYPE is one `--only`/`--exclude` filtered out: `(child type, parent type) → count`.
fn parents_left_out<'a>(skipped: &'a [Skipped], filtered_off: &HashSet<String>) -> BTreeMap<(&'a str, &'static str), usize> {
    let filtered = |t: &str| {
        filtered_off.contains(t) || skipped.iter().any(|s| s.reason == SkipReason::Filtered && s.tf_type == t)
    };
    let mut out = BTreeMap::new();
    for s in skipped {
        if let SkipReason::ParentNotFound(parent) = &s.reason {
            if let Some(parent_type) = parent_type_of(parent).filter(|t| filtered(t)) {
                *out.entry((s.tf_type.as_str(), parent_type)).or_default() += 1;
            }
        }
    }
    out
}

/// The end of every import: what was left out, and why. Never silent — a
/// partial estate is fine, an unexplained one is not.
pub fn report_skipped(found: &Discovered, filtered_off: &HashSet<String>, verbose: bool) {
    for n in &found.notes {
        println!("{}", n);
    }
    let skipped = &found.skipped;
    // An attribute the provider schema HAS, whose value the asset data carried
    // and satz could not place, is a loss: apply the estate and the live
    // resource loses that setting. It is named in full, every run, per
    // resource — never a count behind a flag.
    let (lost, vocabulary): (Vec<&DroppedAttr>, Vec<&DroppedAttr>) =
        found.dropped_attrs.iter().partition(|d| matches!(d.why, DropReason::NotCarried(_)));
    if !lost.is_empty() {
        println!(
            "import: {} attribute(s) the provider schema names are NOT in the estate — an apply would reset them on the live resource:",
            lost.len()
        );
        for d in &lost {
            let DropReason::NotCarried(why) = &d.why else { continue };
            println!("  - {} {} .{} — {}", d.tf_type, d.what, d.path, why);
        }
    }
    if !vocabulary.is_empty() {
        let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
        for d in &vocabulary {
            *by_type.entry(d.tf_type.as_str()).or_default() += 1;
        }
        println!("import: {} attribute(s) dropped — not in the provider schema (API vocabulary; would not plan):", vocabulary.len());
        for (t, n) in &by_type {
            println!("  {:5} {}", n, t);
        }
        if verbose {
            for d in &vocabulary {
                println!("  - {} {} .{}", d.tf_type, d.what, d.path);
            }
        }
    }
    if skipped.is_empty() && filtered_off.is_empty() {
        println!("import: nothing skipped — every resource the source had is in the estate.");
        return;
    }
    // A resource needs its parent in the estate. When `--only`/`--exclude` left the
    // parent's type out, every child is dropped and the estate can come out empty:
    // name the type to add, in the normal output — a targeted import that yields
    // nothing, with the reason only behind `--verbose`, is the defect this answers.
    for ((child, parent), n) in parents_left_out(skipped, filtered_off) {
        println!("import: {} {} need {}, which --only/--exclude left out — add {} to --only", n, child, parent, parent);
    }

    let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
    if !filtered_off.is_empty() && !skipped.iter().any(|s| s.reason == SkipReason::Filtered) {
        // live shape: filtered types are never fetched, so they have no
        // per-resource rows — say so at the type level, as a count: the names of
        // hundreds of types on one line buried everything around them
        by_reason.insert("type(s) filtered by --only/--exclude, not fetched".to_string(), filtered_off.len());
    }
    for s in skipped {
        let key = match &s.reason {
            SkipReason::TypeOff => "type off (import: false)".to_string(),
            SkipReason::Filtered => "filtered by --only/--exclude".to_string(),
            SkipReason::Unmapped(_) => "unmapped (no import-config row fits)".to_string(),
            SkipReason::ParentNotFound(_) => "parent not imported".to_string(),
            // one line per pattern: the operator sees what each one took
            SkipReason::PlatformOwned(p) => format!("platform-owned, skip pattern `{}`", p),
            SkipReason::NotActive(s) => format!("project not ACTIVE ({})", s),
        };
        *by_reason.entry(key).or_default() += 1;
    }
    println!("import: skipped {} resource(s):", skipped.len() + if skipped.iter().any(|s| s.reason == SkipReason::Filtered) { 0 } else { filtered_off.len() });
    for (reason, n) in &by_reason {
        println!("  {:5} {}", n, reason);
    }
    if verbose && !filtered_off.is_empty() {
        let mut types: Vec<&str> = filtered_off.iter().map(String::as_str).collect();
        types.sort();
        println!("  filtered type(s): {}", types.join(", "));
    }
    if verbose && !skipped.is_empty() {
        let mut rows: Vec<&Skipped> = skipped.iter().collect();
        rows.sort_by(|a, b| (&a.tf_type, &a.what).cmp(&(&b.tf_type, &b.what)));
        for s in rows {
            println!("  - {} {} — {}", s.tf_type, s.what, s.reason);
        }
    } else {
        println!("  (--verbose lists every one; `import: false` rows, `--all`, `--only` and `--exclude` are the levers)");
    }
}

#[cfg(test)]
mod skipped_tests {
    use super::*;

    /// Found importing four types from a live organisation: `--only
    /// google_monitoring_alert_policy,…` fetched them all and dropped every one, because
    /// `--only` had left out `google_project`, and the estate came out empty.
    #[test]
    fn a_child_dropped_for_a_filtered_parent_names_the_type_to_add() {
        let skip = |t: &str, reason: SkipReason| Skipped { tf_type: t.into(), what: "x".into(), reason };
        let skipped = vec![
            skip("google_monitoring_alert_policy", SkipReason::ParentNotFound("project acme-infra-001".into())),
            skip("google_monitoring_alert_policy", SkipReason::ParentNotFound("projects/acme-infra-001".into())),
            skip("google_bigquery_dataset", SkipReason::ParentNotFound("project acme-infra-001".into())),
            skip("google_folder_iam_member", SkipReason::ParentNotFound("folders/111".into())),
        ];
        let filtered: HashSet<String> = ["google_project".to_string()].into();
        let left_out = parents_left_out(&skipped, &filtered);
        assert_eq!(left_out.get(&("google_monitoring_alert_policy", "google_project")), Some(&2));
        assert_eq!(left_out.get(&("google_bigquery_dataset", "google_project")), Some(&1));
        // a folder the filter did not touch is some other reason, not this one
        assert_eq!(left_out.get(&("google_folder_iam_member", "google_folder")), None);
    }
}

#[cfg(test)]
mod key_tests {
    //! Two projects hold a resource of the same name — every project has a
    //! `_Default` log sink — and a Satz address is `<type>.<key>` across the
    //! estate, so the imported keys must not collide.
    use super::*;

    fn project(id: &str, sink: &str) -> Project {
        let mut sinks = serde_yaml::Mapping::new();
        sinks.insert(serde_yaml::Value::String(sink.into()), serde_yaml::Value::String("body".into()));
        let mut extra = HashMap::new();
        extra.insert("google_logging_project_sink".to_string(), serde_yaml::Value::Mapping(sinks));
        Project { project_id: id.into(), extra, ..Default::default() }
    }

    fn keys(config: &Config, project: &str) -> Vec<String> {
        let p = &config.project.as_ref().unwrap()[project];
        let serde_yaml::Value::Mapping(m) = &p.extra["google_logging_project_sink"] else { panic!() };
        m.keys().filter_map(|k| k.as_str()).map(str::to_string).collect()
    }

    #[test]
    fn a_key_two_projects_share_takes_the_project_as_prefix() {
        let mut config = Config { project: Some(HashMap::from([
            ("alpha".to_string(), project("alpha", "-default")),
            ("beta".to_string(), project("beta", "-default")),
        ])), ..Default::default() };
        qualify_duplicate_keys(&mut config);
        assert_eq!(keys(&config, "alpha"), ["alpha--default"]);
        assert_eq!(keys(&config, "beta"), ["beta--default"]);
    }

    #[test]
    fn grant_map_members_are_never_qualified() {
        // a member is the same principal in every container; the key is not
        // an address, and a prefixed one is a member that exists nowhere
        fn grants(id: &str) -> Project {
            let mut members = serde_yaml::Mapping::new();
            members.insert(
                serde_yaml::Value::String("user:a@example.com".into()),
                serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("roles/viewer".into())]),
            );
            let mut extra = HashMap::new();
            extra.insert("google_project_iam_member".to_string(), serde_yaml::Value::Mapping(members));
            Project { project_id: id.into(), extra, ..Default::default() }
        }
        let mut config = Config { project: Some(HashMap::from([
            ("alpha".to_string(), grants("alpha")),
            ("beta".to_string(), grants("beta")),
        ])), ..Default::default() };
        qualify_duplicate_keys(&mut config);
        for p in ["alpha", "beta"] {
            let serde_yaml::Value::Mapping(m) = &config.project.as_ref().unwrap()[p].extra["google_project_iam_member"] else { panic!() };
            let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
            assert_eq!(keys, ["user:a@example.com"], "{}", p);
        }
    }

    #[test]
    fn the_attribution_label_is_not_a_declared_label() {
        let raw: serde_json::Value = serde_json::json!({"env": "prod", "goog-terraform-provisioned": "true"});
        let labels = declared_labels(raw.as_object().unwrap()).unwrap();
        assert_eq!(labels.get("env").map(String::as_str), Some("prod"));
        assert!(!labels.contains_key("goog-terraform-provisioned"));
        let only: serde_json::Value = serde_json::json!({"goog-terraform-provisioned": "true"});
        assert_eq!(declared_labels(only.as_object().unwrap()), None, "nothing left is no labels block at all");
    }

    #[test]
    fn a_key_only_one_project_has_stays_as_it_is() {
        let mut config = Config { project: Some(HashMap::from([
            ("alpha".to_string(), project("alpha", "-default")),
            ("beta".to_string(), project("beta", "audit")),
        ])), ..Default::default() };
        qualify_duplicate_keys(&mut config);
        assert_eq!(keys(&config, "alpha"), ["-default"]);
        assert_eq!(keys(&config, "beta"), ["audit"]);
    }
}

#[cfg(test)]
mod shape_tests {
    //! The forms the live shape writes: folders labelled by display name,
    //! the platform's own resources skipped under the pattern that matched.
    use super::*;

    #[test]
    fn folders_are_labelled_by_display_name_and_numbered_only_on_collision() {
        let mut folders: HashMap<String, Folder> = HashMap::from([
            ("folder-1".to_string(), Folder { display_name: "Infrastructure".into(), ..Default::default() }),
            ("folder-2".to_string(), Folder { display_name: "Infrastructure".into(), ..Default::default() }),
            ("folder-3".to_string(), Folder { display_name: "satz-preflight scratch".into(), ..Default::default() }),
            ("folder-4".to_string(), Folder { display_name: "2024 archive".into(), ..Default::default() }),
        ]);
        let mut names: HashMap<String, String> = HashMap::from([
            ("folders/1".to_string(), "folder-1".to_string()),
            ("folders/2".to_string(), "folder-2".to_string()),
            ("folders/3".to_string(), "folder-3".to_string()),
            ("folders/4".to_string(), "folder-4".to_string()),
        ]);
        label_folders_by_display_name(&mut folders, &mut names);
        let mut keys: Vec<&String> = folders.keys().collect();
        keys.sort();
        assert_eq!(keys, ["folder_2024_archive", "infrastructure_1", "infrastructure_2", "satz_preflight_scratch"]);
        assert_eq!(names["folders/3"], "satz_preflight_scratch", "every pass-2 lookup follows the rename");
        assert_eq!(names["folders/1"], "infrastructure_1");
        assert_eq!(folders["infrastructure_2"].display_name, "Infrastructure");
    }

    #[test]
    fn a_bucket_grant_is_a_pinned_map_per_bucket() {
        // two buckets, one member, one role: two maps under one key, each
        // pinned to its bucket, never a labelled `<bucket>-<role>-<member>`
        let mut extra = HashMap::new();
        for bucket in ["a", "b", "a"] {
            push_pinned_grant(&mut extra, "google_storage_bucket_iam_member", "bucket", bucket, "group:x@example.com", "roles/storage.objectViewer", Some(format!("b/{} roles/storage.objectViewer group:x@example.com", bucket)));
        }
        push_pinned_grant(&mut extra, "google_storage_bucket_iam_member", "bucket", "a", "group:x@example.com", "roles/storage.admin", None);
        let serde_yaml::Value::Sequence(maps) = &extra["google_storage_bucket_iam_member"] else { panic!("a list of maps") };
        assert_eq!(maps.len(), 2, "{:?}", maps);
        let a = maps[0].as_mapping().unwrap();
        assert_eq!(a.get("bucket").unwrap().as_str(), Some("a"));
        let roles = a.get("group:x@example.com").unwrap().as_sequence().unwrap();
        assert_eq!(roles.len(), 2, "the second grant on the same bucket joins the map, the third is a repeat: {:?}", roles);
        assert_eq!(maps[1].as_mapping().unwrap().get("bucket").unwrap().as_str(), Some("b"));
    }

    #[test]
    fn a_skip_pattern_names_what_it_matched_and_the_row_refuses_a_typo() {
        let row: crate::config::ImportResourceConfig = serde_yaml::from_str(
            "description: x\nimport: true\nskip: [\"_Default\", \"serviceAccount:service-*@gcp-sa-*.iam.gserviceaccount.com\"]\n",
        )
        .unwrap();
        assert_eq!(skip_pattern(&row, "_Default").as_deref(), Some("_Default"));
        assert_eq!(
            skip_pattern(&row, "serviceAccount:service-123@gcp-sa-ktd.iam.gserviceaccount.com").as_deref(),
            Some("serviceAccount:service-*@gcp-sa-*.iam.gserviceaccount.com")
        );
        assert_eq!(skip_pattern(&row, "serviceAccount:svc-iac-001@acme-infra.iam.gserviceaccount.com"), None);
        assert_eq!(skip_pattern(&row, "audit-sink"), None);
        let typo: Result<crate::config::ImportResourceConfig, _> = serde_yaml::from_str("description: x\nimport: true\nskp: []\n");
        assert!(typo.is_err(), "an unknown key on a row is refused, never ignored");
        assert!(SkipReason::PlatformOwned("_Default".into()).to_string().contains("skip pattern `_Default`"));
        assert!(SkipReason::NotActive("DELETE_REQUESTED".into()).to_string().contains("DELETE_REQUESTED, not ACTIVE"));
    }
}

#[cfg(test)]
mod collision_tests {
    //! One principal granted one role on two folders: the map form's emitted
    //! label hashes member + role and not the node, so the two edges would emit
    //! one address. The import says so, or numbers the second on request.
    use super::*;

    fn folder_with_grant(name: &str, member: &str, roles: Vec<serde_yaml::Value>) -> Folder {
        let mut members = serde_yaml::Mapping::new();
        members.insert(serde_yaml::Value::String(member.into()), serde_yaml::Value::Sequence(roles));
        let mut extra = HashMap::new();
        extra.insert("google_folder_iam_member".to_string(), serde_yaml::Value::Mapping(members));
        Folder { display_name: name.into(), extra, ..Default::default() }
    }

    fn two_folders() -> Config {
        Config {
            folder: Some(HashMap::from([
                ("a".to_string(), folder_with_grant("a", "user:x@example.com", vec![
                    grant_entry("roles/resourcemanager.folderAdmin", "folders/1 roles/resourcemanager.folderAdmin user:x@example.com"),
                    grant_entry("roles/browser", "folders/1 roles/browser user:x@example.com"),
                ])),
                ("b".to_string(), folder_with_grant("b", "user:x@example.com", vec![
                    grant_entry("roles/resourcemanager.folderAdmin", "folders/2 roles/resourcemanager.folderAdmin user:x@example.com"),
                ])),
            ])),
            ..Default::default()
        }
    }

    fn grants<'a>(config: &'a Config, folder: &str) -> &'a serde_yaml::Mapping {
        config.folder.as_ref().unwrap()[folder].extra["google_folder_iam_member"].as_mapping().unwrap()
    }

    #[test]
    fn the_same_grant_on_two_folders_is_refused_by_default() {
        let mut config = two_folders();
        let err = resolve_grant_collisions(&mut config, OnCollision::Error).unwrap_err();
        assert!(err.contains("google_folder_iam_member user:x@example.com roles/resourcemanager.folderAdmin on a, b"), "{}", err);
        assert!(err.contains("--on-collision counter"), "{}", err);
        assert!(!err.contains("roles/browser"), "a role only one folder holds is no collision:\n{}", err);
    }

    #[test]
    fn with_counter_the_second_folder_gets_a_labelled_grant() {
        let mut config = two_folders();
        let notes = resolve_grant_collisions(&mut config, OnCollision::Counter).unwrap();
        assert_eq!(notes.len(), 1, "{:?}", notes);
        assert!(notes[0].contains("google_folder_iam_member.folderAdmin_x_2"), "{}", notes[0]);
        assert!(notes[0].contains("a holds the same grant"), "{}", notes[0]);
        // the first node keeps both roles in the map form
        let a = grants(&config, "a");
        assert_eq!(a.get("user:x@example.com").unwrap().as_sequence().unwrap().len(), 2);
        // the second: the member line held nothing else and is gone; the edge
        // is a labelled resource carrying its own import id
        let b = grants(&config, "b");
        assert!(b.get("user:x@example.com").is_none(), "{:?}", b);
        let labelled = b.get("folderAdmin_x_2").unwrap().as_mapping().unwrap();
        assert_eq!(labelled.get("role").unwrap().as_str(), Some("roles/resourcemanager.folderAdmin"));
        assert_eq!(labelled.get("member").unwrap().as_str(), Some("user:x@example.com"));
        assert_eq!(labelled.get("import-id").unwrap().as_str(), Some("folders/2 roles/resourcemanager.folderAdmin user:x@example.com"));
    }

    #[test]
    fn a_third_folder_takes_the_next_number() {
        let mut config = two_folders();
        config.folder.as_mut().unwrap().insert("c".to_string(), folder_with_grant("c", "user:x@example.com", vec![
            grant_entry("roles/resourcemanager.folderAdmin", "folders/3 roles/resourcemanager.folderAdmin user:x@example.com"),
        ]));
        let notes = resolve_grant_collisions(&mut config, OnCollision::Counter).unwrap();
        assert_eq!(notes.len(), 2, "{:?}", notes);
        assert!(grants(&config, "b").get("folderAdmin_x_2").is_some());
        assert!(grants(&config, "c").get("folderAdmin_x_3").is_some());
    }

    #[test]
    fn a_folder_grant_and_a_project_grant_never_collide() {
        // different types emit different addresses whatever the member and role
        let mut config = two_folders();
        let mut members = serde_yaml::Mapping::new();
        members.insert(
            serde_yaml::Value::String("user:x@example.com".into()),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("roles/browser".into())]),
        );
        let mut extra = HashMap::new();
        extra.insert("google_project_iam_member".to_string(), serde_yaml::Value::Mapping(members));
        config.project = Some(HashMap::from([("p".to_string(), Project { project_id: "p".into(), extra, ..Default::default() })]));
        config.folder.as_mut().unwrap().remove("b");
        assert_eq!(resolve_grant_collisions(&mut config, OnCollision::Error).unwrap(), Vec::<String>::new());
        assert_eq!(grants(&config, "a").get("user:x@example.com").unwrap().as_sequence().unwrap().len(), 2, "nothing moved");
    }

    #[test]
    fn labels_are_readable_identifiers() {
        assert_eq!(grant_label_base("roles/resourcemanager.folderAdmin", "user:alice.b@example.com"), "folderAdmin_alice_b");
        assert_eq!(grant_label_base("organizations/1/roles/myRole", "serviceAccount:svc@p.iam.gserviceaccount.com"), "myRole_svc");
        assert_eq!(grant_label_base("roles/viewer", "allUsers"), "viewer_allUsers");
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

    fn folder_with_parent(name: &str, parent: &str) -> Folder {
        Folder { parent: Some(parent.into()), ..folder(name) }
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
    fn a_nested_folder_drops_the_parent_it_was_discovered_with() {
        // the nesting is the parent; declared beside it the emitter refuses the
        // folder, so a live import of nested folders would not compile
        let parents: HashMap<String, String> =
            [("folders/2", "folders/1")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let names: HashMap<String, String> =
            [("folders/1", "a"), ("folders/2", "b")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut map: HashMap<String, Folder> = [
            ("a".to_string(), folder("a")),
            ("b".to_string(), folder_with_parent("b", "folders/1")),
        ]
        .into_iter()
        .collect();
        link_folders_to_parents(&parents, &names, &mut map).unwrap();
        assert_eq!(map["a"].folder.as_ref().unwrap()["b"].parent, None);
    }

    #[test]
    fn a_project_under_the_organisation_is_top_level_without_folder_id() {
        // it used to take the "outside the sweep" arm and come out as
        // `folder_id = "organizations/…"`, which the provider refuses
        let pparents: HashMap<String, String> = [("projects/p1", "organizations/1")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let pnames: HashMap<String, String> = [("projects/p1", "p1")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut projects: HashMap<String, Project> =
            [("p1".to_string(), Project { project_id: "p1".into(), ..Default::default() })].into_iter().collect();
        let mut folders: HashMap<String, Folder> = HashMap::new();
        link_projects_to_folders(&pparents, &pnames, &mut projects, &mut folders).unwrap();
        assert!(projects.contains_key("p1"), "the project stays at the top level");
        assert_eq!(projects["p1"].extra.get("folder_id"), None, "no parent attribute: the emitter derives org_id");
    }

    #[test]
    fn a_top_level_folder_carries_no_parent() {
        // the top level IS the organization parent; written beside it the
        // estate repeats the organization id the emitter derives
        let parents: HashMap<String, String> =
            [("folders/1", "organizations/1")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let names: HashMap<String, String> = [("folders/1", "a")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let mut map: HashMap<String, Folder> =
            [("a".to_string(), folder_with_parent("a", "organizations/1"))].into_iter().collect();
        link_folders_to_parents(&parents, &names, &mut map).unwrap();
        assert_eq!(map["a"].parent, None);
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


#[cfg(test)]
mod state_document_tests {
    //! A `tofu show -json` entry without `type` or `name` is a broken document,
    //! not a resource of type "" that the operator turned off.
    use super::*;

    #[test]
    fn a_resource_without_a_type_is_refused() {
        let state = serde_json::json!({
            "values": { "root_module": { "resources": [
                { "address": "google_folder.x", "name": "x", "values": {} }
            ] } }
        });
        let err = Discoverer::new(state, None, None, HashSet::new(), OnCollision::default()).discover().err().expect("no type is not a type");
        let msg = err.to_string();
        assert!(msg.contains("google_folder.x") && msg.contains("`type`"), "{msg}");
    }
}

#[cfg(test)]
mod asset_attributes {
    //! What a Cloud Asset sweep hands back against what the estate must carry.
    //! The fixture is a `storage.googleapis.com/Bucket` asset in the API's own
    //! shape (`tests/assets/storage-bucket.json`), read through the shipped
    //! import-config row and the provider schema fixture.
    use super::*;
    use std::path::Path;

    fn repo(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn asset_data() -> Value {
        let text = std::fs::read_to_string(repo("tests/assets/storage-bucket.json")).expect("bucket asset fixture");
        let asset: Value = serde_json::from_str(&text).expect("bucket asset fixture parses");
        asset["resource"]["data"].clone()
    }

    fn registry() -> ResourceRegistry {
        ResourceRegistry::load_all(&repo("tests/schemas").to_string_lossy()).expect("provider schema fixture")
    }

    /// The row `satz import` uses for a bucket, from the shipped table.
    fn bucket_row() -> crate::config::ImportResourceConfig {
        let text = std::fs::read_to_string(repo("presets/import-config.yaml")).expect("import-config");
        let config: ImportConfig = serde_yaml::from_str(&text).expect("import-config parses");
        config.resource_types["google_storage_bucket"].clone()
    }

    fn import_bucket(data: &Value) -> (serde_yaml::Value, Vec<DroppedAttr>) {
        let _ = take_dropped();
        let reg = registry();
        let schema = reg.find_resource("google_storage_bucket").map(|(_, s)| s);
        let row = bucket_row();
        let out = Discoverer::filter_values(
            "google_storage_bucket",
            "acme-organization-audit-bucket",
            data,
            schema,
            false,
            row.exclude.as_ref(),
            row.map.as_ref(),
        );
        (out, take_dropped())
    }

    fn losses(dropped: &[DroppedAttr]) -> Vec<String> {
        dropped
            .iter()
            .filter_map(|d| match &d.why {
                DropReason::NotCarried(why) => Some(format!("{} .{} — {}", d.what, d.path, why)),
                DropReason::Vocabulary => None,
            })
            .collect()
    }

    /// Two hardened buckets came back from a live sweep without their
    /// hardening: the API nests what the provider flattens, and the nesting
    /// was dropped as unknown vocabulary. An apply of that estate switches
    /// uniform bucket-level access off and public access prevention back to
    /// inherited.
    #[test]
    fn a_hardened_bucket_is_imported_hardened() {
        let (out, dropped) = import_bucket(&asset_data());
        let text = serde_yaml::to_string(&out).expect("yaml");
        assert_eq!(out["uniform_bucket_level_access"], serde_yaml::Value::Bool(true), "{text}");
        assert_eq!(out["public_access_prevention"].as_str(), Some("enforced"), "{text}");
        assert_eq!(out["requester_pays"], serde_yaml::Value::Bool(true), "{text}");
        // `isLive: false` is the provider's `with_state = "ARCHIVED"`
        assert_eq!(out["lifecycle_rule"][0]["condition"]["with_state"].as_str(), Some("ARCHIVED"), "{text}");
        assert_eq!(out["lifecycle_rule"][0]["condition"]["age"].as_u64(), Some(365), "{text}");
        assert!(out["encryption"]["default_kms_key_name"].as_str().is_some(), "{text}");
        assert!(losses(&dropped).is_empty(), "{:?}", losses(&dropped));
        // the API's own vocabulary is still dropped, and named per resource
        let vocab: Vec<&str> = dropped.iter().map(|d| d.path.as_str()).collect();
        assert!(vocab.contains(&"iam_configuration.bucket_policy_only.enabled"), "{:?}", vocab);
        assert!(vocab.contains(&"kind"), "{:?}", vocab);
        assert!(dropped.iter().all(|d| d.what == "acme-organization-audit-bucket"), "{:?}", dropped);
    }

    /// The guard: for every value the asset data carries under a key the
    /// provider schema does not name, where the schema DOES name an attribute
    /// that takes it, the estate carries it — or the report says it does not.
    /// Silence is the defect.
    #[test]
    fn nothing_the_schema_names_is_dropped_in_silence() {
        let data = asset_data();
        let (out, dropped) = import_bucket(&data);
        let reg = registry();
        let block = &reg.find_resource("google_storage_bucket").expect("bucket schema").1.block;
        let reported: HashSet<&str> = dropped
            .iter()
            .filter(|d| matches!(d.why, DropReason::NotCarried(_)))
            .map(|d| d.path.as_str())
            .collect();
        let data: serde_yaml::Value = serde_yaml::to_value(&data).expect("yaml");
        let serde_yaml::Value::Mapping(top) = &data else { panic!("the asset data is an object") };
        for (key, value) in top {
            let key = crate::align::snake(key.as_str().unwrap_or(""));
            if block.block_types.contains_key(&key) || block.attributes.contains_key(&key) {
                continue; // a block or an attribute of its own — not a flattening
            }
            let mut found = Vec::new();
            leaves("", value, 0, 0, &mut found);
            for leaf in &found {
                let wrapper = match leaf.path.rsplit_once('.') {
                    Some((head, last)) if matches!(last, "enabled" | "value") && leaf.siblings <= 2 => {
                        head.rsplit('.').next().map(String::from)
                    }
                    _ => None,
                };
                for candidate in [Some(leaf.name.clone()), wrapper].into_iter().flatten() {
                    if !attribute_takes(block, &candidate, &leaf.value) {
                        continue;
                    }
                    let path = format!("{}.{}", key, leaf.path);
                    assert!(
                        out.get(candidate.as_str()).is_some() || reported.contains(path.as_str()),
                        "{} carries `{}` and neither the estate nor the report has it:\n{}",
                        path,
                        candidate,
                        serde_yaml::to_string(&out).unwrap_or_default()
                    );
                }
            }
        }
    }

    /// Two fields of the asset data claiming one attribute, with different
    /// values: satz carries neither and names both. Picking one would be a
    /// guess about which of them the live resource is in.
    #[test]
    fn two_fields_claiming_one_attribute_carry_neither_and_say_so() {
        let data: Value = serde_json::json!({
            "name": "acme-organization-audit-bucket",
            "location": "EU",
            "iamConfiguration": {"publicAccessPrevention": "enforced"},
            "legacyConfiguration": {"publicAccessPrevention": "inherited"}
        });
        let (out, dropped) = import_bucket(&data);
        assert!(out.get("public_access_prevention").is_none(), "{:?}", out);
        let lost = losses(&dropped);
        assert_eq!(lost.len(), 2, "{:?}", dropped);
        assert!(lost.iter().all(|l| l.contains("public_access_prevention") && l.contains("disagree")), "{:?}", lost);
    }
}
