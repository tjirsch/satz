use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Schema {
    pub provider_schemas: HashMap<String, ProviderSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderSchema {
    pub resource_schemas: HashMap<String, ResourceSchema>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AttributeSchema {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub computed: bool,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// The provider's type expression: `"string"`, `"bool"`, `["list","string"]`,
    /// `["map","string"]` …
    #[serde(rename = "type", default)]
    pub type_: Option<serde_json::Value>,
    /// The provider's own words, shown by the language server on hover.
    #[serde(default)]
    pub description: Option<String>,
}

impl AttributeSchema {
    pub fn is_string(&self) -> bool {
        self.type_.as_ref().and_then(|t| t.as_str()) == Some("string")
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ResourceSchema {
    pub block: BlockSchema,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BlockTypeSchema {
    pub min_items: Option<u64>,
    pub max_items: Option<u64>,
    pub nesting_mode: Option<String>,
    pub block: BlockSchema,
}

impl BlockTypeSchema {
    /// A block that occurs at most once — written `key { … }`, never as a
    /// list of one.
    pub fn is_single(&self) -> bool {
        self.nesting_mode.as_deref() == Some("single") || self.max_items == Some(1)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BlockSchema {
    #[serde(default)]
    pub attributes: HashMap<String, AttributeSchema>,
    #[serde(default)]
    pub block_types: HashMap<String, BlockTypeSchema>,
    #[serde(default)]
    pub description: Option<String>,
}

impl BlockSchema {
    /// The estate's form of one asset's data, and the required attributes the
    /// data did not carry and the estate does not derive — the caller reports
    /// those, because only it knows what the resource is called.
    pub fn extract_attributes(&self, data: &serde_json::Map<String, serde_json::Value>, resource_type: &str) -> (serde_yaml::Mapping, Vec<String>) {
        self.extract(data, resource_type, true)
    }

    /// `scoped` is true for the resource's own attributes and false inside a
    /// nested block: only the resource's own carry the enclosure's values
    /// (`scope_attribute`).
    fn extract(&self, data: &serde_json::Map<String, serde_json::Value>, resource_type: &str, scoped: bool) -> (serde_yaml::Mapping, Vec<String>) {
        let mut map = serde_yaml::Mapping::new();
        let mut missing: Vec<String> = Vec::new();
        
        // 1. Handle Attributes
        for (attr_name, attr_schema) in &self.attributes {
            if attr_name == "id" || attr_name == "etag" || attr_name == "self_link" || 
               attr_name == "create_time" || attr_name == "update_time" {
                continue;
            }

            // Look for value in data using multiple casing strategies if needed
            // The API usually returns camelCase. The schema uses snake_case.
            let camel_name = snake_to_camel(attr_name);
            let val = data.get(attr_name).or_else(|| data.get(&camel_name));

            if let Some(v) = val {
                // Determine if we should include it
                let should_include = if attr_schema.required {
                    true
                } else if attr_schema.optional {
                     // Include if not null/empty? Or just include if present?
                     // Let's assume include if present and not null.
                     !v.is_null()
                } else if attr_schema.computed {
                    // Computed but not required/optional -> generally exclude unless we really want it.
                    // But if it's there, maybe we keep it? 
                    // User rule: "all other with the computed false may be needed if they have a value"
                    // If computed=true and required=false, usually we do NOT write it to config.
                    false 
                } else {
                    false
                };

                if should_include {
                     // A string-typed attribute holding structured data (org-policy
                     // `parameters`: an object in the API, a JSON string in Terraform)
                     // is carried as its JSON text.
                     let yaml_v = if attr_schema.is_string() && (v.is_object() || v.is_array()) {
                         serde_json::to_string(v).ok().map(serde_yaml::Value::String)
                     } else if let (true, Some(b)) = (attr_schema.is_string(), v.as_bool()) {
                         // A boolean in a string-typed attribute (org-policy `enforce`,
                         // `allow_all`, `deny_all`): the provider stores "TRUE"/"FALSE",
                         // so that spelling avoids a plan diff after import.
                         Some(serde_yaml::Value::String(if b { "TRUE" } else { "FALSE" }.into()))
                     } else {
                         serde_yaml::to_value(v).ok()
                     };
                     if let Some(yaml_v) = yaml_v {
                         map.insert(serde_yaml::Value::String(attr_name.clone()), yaml_v);
                     }
                }
            } else if attr_schema.required && !(scoped && scope_attribute(resource_type, attr_name)) {
                // Required, not in the data, and not one the enclosure writes.
                missing.push(attr_name.clone());
            }
        }

        // 2. Handle Nested Blocks
        for (block_name, block_type) in &self.block_types {
            let camel_name = snake_to_camel(block_name);
            let val = data.get(block_name).or_else(|| data.get(&camel_name));
             
            if let Some(v) = val {
                if let Some(arr) = v.as_array() {
                    let mut yaml_arr = Vec::new();
                    for item in arr {
                         if let Some(obj) = item.as_object() {
                             let (sub_map, sub_missing) = block_type.block.extract(obj, resource_type, false);
                             missing.extend(sub_missing.into_iter().map(|a| format!("{}.{}", block_name, a)));
                             if !sub_map.is_empty() {
                                 yaml_arr.push(serde_yaml::Value::Mapping(sub_map));
                             }
                         }
                    }
                    if !yaml_arr.is_empty() {
                        map.insert(serde_yaml::Value::String(block_name.clone()), serde_yaml::Value::Sequence(yaml_arr));
                    }
                } else if let Some(obj) = v.as_object() {
                    // Sometimes blocks are single objects in API but list in TF?
                    // Or standard nested block.
                    let (sub_map, sub_missing) = block_type.block.extract(obj, resource_type, false);
                     missing.extend(sub_missing.into_iter().map(|a| format!("{}.{}", block_name, a)));
                     if !sub_map.is_empty() {
                         // If schema says nice max_items=1 it might be list.
                         // But usually blocks are lists in TF.
                         map.insert(serde_yaml::Value::String(block_name.clone()), 
                             serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(sub_map)]));
                     }
                }
            }
        }

        (map, missing)
    }
}

/// Whether the value of this attribute is where the resource STANDS rather than
/// what it is. The emitter writes each one from the enclosing node — a project,
/// folder or organization body gives `project`/`project_id`, `folder`/`folder_id`
/// and `org_id`/`organization` to every type whose schema names one, and it
/// builds an org policy's `parent` and a folder's `parent` from the same place
/// (`emit_shared::render_resource`). Cloud Asset data therefore never has to
/// carry it, and an import that leaves it out loses nothing: the estate says it
/// by where the resource is written.
pub(crate) fn scope_attribute(tf_type: &str, attr: &str) -> bool {
    matches!(attr, "project" | "project_id" | "folder" | "folder_id" | "org_id" | "organization")
        || (attr == "parent" && matches!(tf_type, "google_org_policy_policy" | "google_folder"))
}

/// `snake_case` → `snakeCase`: the attribute names the Discovery Documents and
/// the live API use, against the provider's underscore form.
pub(crate) fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub struct ResourceRegistry {
    pub resources: HashMap<String, (String, ResourceSchema)>, // resource_name -> (provider_name, schema)
}

impl ResourceRegistry {
    pub fn load_all(directory: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut resources = HashMap::new();
        // A missing directory reads as empty here and is refused below with the rest:
        // satz types every resource from the provider's own schema, so a compile with
        // none is not a compile. It used to load zero types and carry on, and the
        // estate then failed wherever the first resource needed a type — on a fresh
        // organisation that was the IaC users group, three steps into bootstrap.
        let entries = match crate::fsx::read_dir_entries(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        {
            for entry in entries {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    let content = crate::fsx::read_to_string(entry.path())?;
                    let schema: Schema = serde_json::from_str(&content)?;
                    
                    for (prov_name, prov_schema) in schema.provider_schemas {
                        for (res_name, res_schema) in prov_schema.resource_schemas {
                            resources.insert(res_name.clone(), (prov_name.clone(), res_schema));
                        }
                    }
                }
            }
        }
        if resources.is_empty() {
            return Err(format!(
                "no provider schema in '{}' — satz types every resource from the provider's own schema. \
                 `satz update-schema` writes it (it runs `tofu providers schema`, so OpenTofu has to be on PATH)",
                directory
            )
            .into());
        }
        Ok(ResourceRegistry { resources })
    }

    /// Whether the nested block at `path` (`spec`, `spec/rules/condition`) of
    /// a resource type is a single block. Unknown type or path: `false` — the
    /// list form is always valid, the block form only where the schema says.
    pub fn single_block(&self, tf_type: &str, path: &str) -> bool {
        let Some((_, schema)) = self.find_resource(tf_type) else { return false };
        let mut block = &schema.block;
        let mut segments = path.split('/').peekable();
        while let Some(seg) = segments.next() {
            let Some(bt) = block.block_types.get(seg) else { return false };
            if segments.peek().is_none() {
                return bt.is_single();
            }
            block = &bt.block;
        }
        false
    }

    /// The block at `path` inside a resource type — `path` empty is the resource's own
    /// body, each further element a block key one level down. `None` where the registry
    /// has no such type, or the schema no such block.
    pub fn block_at(&self, tf_type: &str, path: &[&str]) -> Option<&BlockSchema> {
        // EXACT lookup: a bare type name is not Satz, and a body
        // is judged against the type the estate actually names.
        let (_, schema) = self.resources.get(tf_type)?;
        let mut block = &schema.block;
        for seg in path {
            block = &block.block_types.get(*seg)?.block;
        }
        Some(block)
    }

    pub fn find_resource(&self, key: &str) -> Option<(&str, &ResourceSchema)> {
        // 1. Try exact match
        if let Some((prov, schema)) = self.resources.get(key) {
            return Some((prov, schema));
        }
        // 2. Try google_ prefix
        let google_key = format!("google_{}", key);
        if let Some((prov, schema)) = self.resources.get(&google_key) {
            return Some((prov, schema));
        }
        None
    }

    pub fn generate_schema(tool: &str, provider: &str, version: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let work_dir = format!(".temp_schema_gen_{}", provider);
        crate::fsx::create_dir_all(&work_dir)?;

        let (name, full_source) = derive_registry_source(provider);

        let main_tf = format!(
            r#"terraform {{
  required_providers {{
    {} = {{
      source = "{}"
      version = "{}"
    }}
  }}
}}
"#,
            name, full_source, version
        );

        crate::fsx::write(format!("{}/main.tf", work_dir), main_tf)?;

        // the provider download is the long part, and the schema dump after it is
        // captured, so without this line the run goes quiet for a minute
        eprintln!(
            "generating the {} schema: `{} init` downloads the provider {} once, then `{} providers schema` reads it …",
            provider, tool, version, tool
        );
        let status = Command::new(tool)
            .arg("init")
            .current_dir(&work_dir)
            .status()?;

        if !status.success() {
            return Err(format!("{} init failed for {}", tool, provider).into());
        }

        let output = Command::new(tool)
            .args(["providers", "schema", "-json"])
            .current_dir(&work_dir)
            .output()?;

        if !output.status.success() {
            return Err(format!("{} providers schema failed for {}", tool, provider).into());
        }

        // Fail loudly if the output does not carry the provider we asked for. A wrong
        // source derivation otherwise writes a plausible-looking file (this is exactly
        // how google-beta.json silently became a copy of the GA schema) and the error
        // only surfaces months later as an "unknown resource type" warning.
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("{} providers schema for {} returned unparseable JSON: {}", tool, provider, e))?;
        if !schema_output_contains_source(&json, &full_source) {
            let delivered: Vec<String> = json
                .get("provider_schemas")
                .and_then(|p| p.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            crate::fsx::remove_dir_all(&work_dir)?;
            return Err(format!(
                "schema fetch for '{}' delivered the wrong provider: requested source '{}', \
                 got {:?}. The name-to-source derivation is probably wrong for this provider.",
                provider, full_source, delivered
            )
            .into());
        }

        crate::fsx::write(output_path, output.stdout)?;
        crate::fsx::remove_dir_all(&work_dir)?;

        Ok(())
    }
}

/// Registry providers that exist in their own right and must never be folded into a
/// sibling by the prefix heuristics: "google-beta" is not an alias of "google", and
/// "azuread"/"awscc" are not aliases of "azurerm"/"aws".
const STANDALONE_PROVIDERS: &[&str] = &["google-beta", "azuread", "awscc", "azapi"];

/// Derive `(local provider name, full registry source)` from a config entry.
///
/// Accepts a bare name (`google`), an alias to be folded onto its base provider
/// (`google-eu` -> `hashicorp/google`), or an already-qualified source
/// (`mycorp/thing`). Exact matches against known standalone providers win over the
/// prefix heuristics, which exist only for aliases.
pub(crate) fn derive_registry_source(provider: &str) -> (&str, String) {
    let parts: Vec<&str> = provider.split('/').collect();
    if parts.len() == 2 {
        return (parts[1], provider.to_string());
    }

    let name = provider;
    let base = if STANDALONE_PROVIDERS.contains(&name) {
        name
    } else if name.starts_with("google") {
        "google"
    } else if name.starts_with("aws") {
        "aws"
    } else if name.starts_with("az") {
        "azurerm"
    } else if name.starts_with("ali") {
        "alicloud"
    } else {
        name
    };
    (name, format!("hashicorp/{}", base))
}

/// True when `tofu providers schema -json` output contains the requested source.
/// Keys are fully qualified (`registry.opentofu.org/hashicorp/google-beta`), so the
/// requested `hashicorp/google-beta` is matched as a path suffix.
pub(crate) fn schema_output_contains_source(json: &serde_json::Value, full_source: &str) -> bool {
    json.get("provider_schemas")
        .and_then(|p| p.as_object())
        .map(|o| {
            o.keys()
                .any(|k| k == full_source || k.ends_with(&format!("/{}", full_source)))
        })
        .unwrap_or(false)
}

// Tests (pure layer only — no tofu, no network).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_is_its_own_registry_entry_not_an_alias() {
        // The bug: starts_with("google") swallowed google-beta, so its throwaway
        // workspace pulled hashicorp/google and google-beta.json became a byte-identical
        // copy of the GA schema — every beta-only resource silently unknown.
        assert_eq!(derive_registry_source("google-beta"), ("google-beta", "hashicorp/google-beta".to_string()));
    }

    #[test]
    fn aliases_fold_onto_their_base_provider() {
        assert_eq!(derive_registry_source("google"), ("google", "hashicorp/google".to_string()));
        assert_eq!(derive_registry_source("google-eu"), ("google-eu", "hashicorp/google".to_string()));
        assert_eq!(derive_registry_source("aws-prod"), ("aws-prod", "hashicorp/aws".to_string()));
        assert_eq!(derive_registry_source("azurerm-x"), ("azurerm-x", "hashicorp/azurerm".to_string()));
    }

    #[test]
    fn sibling_providers_do_not_fold_into_each_other() {
        // Same trap as google-beta, next candidates over.
        assert_eq!(derive_registry_source("azuread"), ("azuread", "hashicorp/azuread".to_string()));
        assert_eq!(derive_registry_source("awscc"), ("awscc", "hashicorp/awscc".to_string()));
    }

    #[test]
    fn qualified_sources_pass_through_with_last_segment_as_name() {
        assert_eq!(derive_registry_source("mycorp/thing"), ("thing", "mycorp/thing".to_string()));
    }

    #[test]
    fn schema_output_validation_matches_suffix_and_rejects_wrong_provider() {
        let json: serde_json::Value = serde_json::json!({
            "provider_schemas": {
                "registry.opentofu.org/hashicorp/google": {}
            }
        });
        assert!(schema_output_contains_source(&json, "hashicorp/google"));
        // The exact silent failure from the bug report: asked for beta, got GA.
        assert!(!schema_output_contains_source(&json, "hashicorp/google-beta"));
        assert!(!schema_output_contains_source(&serde_json::json!({}), "hashicorp/google"));
    }
}


#[cfg(test)]
mod load_all_tests {
    //! A missing schema directory is the designed exception: schemas may be
    //! fetched later, and the registry is empty. A schema file that does not
    //! parse is an error, and every command that loads the registry propagates
    //! it rather than running without one — an emitter with no registry drops
    //! schema-derived detail and says nothing.

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-schema-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A compile with no schema is not a compile. It used to load zero types and carry
    /// on, and the estate failed wherever the first resource needed one — on a fresh
    /// organisation, the IaC users group three steps into bootstrap. Missing or empty,
    /// the directory is refused by name, with the command that fills it.
    #[test]
    fn a_missing_or_empty_schema_directory_is_refused_by_name() {
        let missing = scratch("missing");
        let err = ResourceRegistry::load_all(missing.to_str().unwrap()).err().expect("no schema is an error");
        assert!(err.to_string().contains("satz update-schema"), "{err}");
        assert!(err.to_string().contains(missing.to_str().unwrap()), "{err}");

        let empty = scratch("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let result = ResourceRegistry::load_all(empty.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&empty);
        let err = result.err().expect("an empty schema dir is an error");
        assert!(err.to_string().contains("no provider schema"), "{err}");
    }

    #[test]
    fn a_schema_file_that_does_not_parse_is_an_error() {
        let dir = scratch("malformed");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("google.json"), "{ not json").unwrap();
        let result = ResourceRegistry::load_all(dir.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        let err = result.err().expect("a broken schema file must not load as an empty registry");
        assert!(!err.to_string().is_empty());
    }
}
