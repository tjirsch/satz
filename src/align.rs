//! Schema alignment (roadmap F5c): the API's vocabulary against Terraform's,
//! computed once per resource type instead of hand-written.
//!
//! Cloud Asset Inventory hands back each resource in its API shape — the
//! schema of the API's Discovery Document. The provider schema is the
//! Terraform shape. No naming rule relates the two (`lifecycle_rule` is a
//! reserved-word collision, `uniform_bucket_level_access` a flattening
//! chosen per field), but with both schemas in hand the correspondence is an
//! alignment:
//!
//! 1. **exact** — `snake_case(api field)` is the Terraform name at the same
//!    level (the large majority);
//! 2. **flattened** — a Terraform attribute with no same-level source whose
//!    name is an API leaf deeper down, or an API object of that name holding a
//!    single `enabled`/`value` leaf (`iamConfiguration.uniformBucketLevelAccess.enabled`
//!    → `uniform_bucket_level_access`);
//! 3. **renamed** — a Terraform block with no same-level source whose
//!    attribute names overlap an API array-of-objects' properties
//!    (`lifecycle.rule[]` → `lifecycle_rule`);
//! 4. the rest is **unmatched** and reported.
//!
//! The result is data (`presets/type-map.yaml`), applied by discovery before
//! the schema filter, reviewed by a human only where it says unmatched or
//! renamed, and re-derived per provider bump. `tofu plan` remains the check.

use std::collections::BTreeMap;

use crate::schema::BlockSchema;

/// One resource type's alignment.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypeMap {
    /// API path (dotted, camelCase as the API spells it) → Terraform path
    /// (dotted snake_case). Only the deviations — exact matches need no row.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub map: BTreeMap<String, String>,
    /// How each row was found: `flattened` or `renamed` — the ones to review.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub how: BTreeMap<String, String>,
    /// API fields nothing in the Terraform schema corresponds to. Dropped at
    /// import; listed so nothing vanishes unnoticed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched: Vec<String>,
    /// Terraform attributes/blocks the API schema has no source for
    /// (Terraform-only knobs such as `force_destroy`, or computed outputs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tf_only: Vec<String>,
    /// Counts, for the summary line.
    #[serde(default)]
    pub exact: usize,
}

/// A field of the API schema, flattened with its dotted path.
#[derive(Debug, Clone)]
struct ApiField {
    path: String,
    /// last segment
    name: String,
    /// `object` with properties, `array<object>`, or a scalar/other
    kind: ApiKind,
    /// property names of an object / array-of-objects
    props: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ApiKind {
    Object,
    ArrayOfObjects,
    Leaf,
}

/// Every field of a Discovery schema, depth-first, paths dotted. `$ref`s are
/// followed through `schemas` (one level, to keep cycles out).
fn api_fields(schema: &serde_json::Value, schemas: &serde_json::Value, prefix: &str, depth: usize, out: &mut Vec<ApiField>) {
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else { return };
    for (name, prop) in props {
        let path = if prefix.is_empty() { name.clone() } else { format!("{}.{}", prefix, name) };
        let resolved = resolve_ref(prop, schemas);
        let item = resolved.get("items").map(|i| resolve_ref(i, schemas));
        let (kind, sub) = match (resolved.get("type").and_then(|t| t.as_str()), &item) {
            (Some("object"), _) if resolved.get("properties").is_some() => (ApiKind::Object, Some(resolved.clone())),
            (Some("array"), Some(it)) if it.get("properties").is_some() => (ApiKind::ArrayOfObjects, Some(it.clone())),
            _ => (ApiKind::Leaf, None),
        };
        let props: Vec<String> = sub
            .as_ref()
            .and_then(|s| s.get("properties"))
            .and_then(|p| p.as_object())
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        out.push(ApiField { path: path.clone(), name: name.clone(), kind: kind.clone(), props });
        if let Some(s) = sub {
            if depth < 4 {
                api_fields(&s, schemas, &path, depth + 1, out);
            }
        }
    }
}

fn resolve_ref(v: &serde_json::Value, schemas: &serde_json::Value) -> serde_json::Value {
    match v.get("$ref").and_then(|r| r.as_str()) {
        Some(r) => schemas.get(r).cloned().unwrap_or_else(|| v.clone()),
        None => v.clone(),
    }
}

pub(crate) fn snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Align one API schema (a Discovery `schemas.<Name>` object, with the
/// document's `schemas` for `$ref`s) against one Terraform block.
pub fn align(api_schema: &serde_json::Value, schemas: &serde_json::Value, tf: &BlockSchema) -> TypeMap {
    let mut fields = Vec::new();
    api_fields(api_schema, schemas, "", 0, &mut fields);
    let mut tm = TypeMap::default();
    let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    align_block(&fields, tf, "", "", &mut tm, &mut used);
    // API top-level fields nothing consumed (and none of their descendants either)
    for f in &fields {
        if f.path.contains('.') {
            continue;
        }
        let consumed = used.iter().any(|u| u == &f.path || u.starts_with(&format!("{}.", f.path)));
        if !consumed {
            tm.unmatched.push(f.path.clone());
        }
    }
    tm
}

/// Align the Terraform block at `tf_prefix` against the API subtree at
/// `api_prefix`.
fn align_block(
    fields: &[ApiField],
    tf: &BlockSchema,
    tf_prefix: &str,
    api_prefix: &str,
    tm: &mut TypeMap,
    used: &mut std::collections::BTreeSet<String>,
) {
    let same_level = |f: &ApiField| -> bool {
        match f.path.rsplit_once('.') {
            Some((p, _)) => p == api_prefix,
            None => api_prefix.is_empty(),
        }
    };
    let tf_path = |name: &str| if tf_prefix.is_empty() { name.to_string() } else { format!("{}.{}", tf_prefix, name) };

    // attributes
    let mut names: Vec<&String> = tf.attributes.keys().collect();
    names.sort();
    for name in names {
        let attr = &tf.attributes[name];
        if let Some(f) = fields.iter().find(|f| same_level(f) && snake(&f.name) == *name) {
            used.insert(f.path.clone());
            tm.exact += 1;
            continue;
        }
        if attr.computed && !attr.optional && !attr.required {
            continue; // an output; the API side is irrelevant
        }
        // flattened: a deeper leaf of that name, or an object of that name
        // wrapping a single enabled/value leaf — under this API subtree only
        let under = |f: &ApiField| api_prefix.is_empty() || f.path.starts_with(&format!("{}.", api_prefix));
        let leaf = fields.iter().filter(|f| under(f) && f.kind == ApiKind::Leaf && snake(&f.name) == *name).collect::<Vec<_>>();
        let wrapper = fields
            .iter()
            .filter(|f| under(f) && f.kind == ApiKind::Object && snake(&f.name) == *name && f.props.len() <= 2)
            .filter_map(|f| {
                f.props
                    .iter()
                    .find(|p| matches!(p.as_str(), "enabled" | "value"))
                    .map(|p| format!("{}.{}", f.path, p))
            })
            .collect::<Vec<_>>();
        let candidates: Vec<String> = leaf.iter().map(|f| f.path.clone()).chain(wrapper).collect();
        match candidates.as_slice() {
            [one] => {
                tm.map.insert(one.clone(), tf_path(name));
                tm.how.insert(one.clone(), "flattened".into());
                used.insert(one.clone());
            }
            _ => tm.tf_only.push(tf_path(name)),
        }
    }

    // blocks
    let mut bnames: Vec<&String> = tf.block_types.keys().collect();
    bnames.sort();
    for name in bnames {
        if name == "timeouts" {
            continue;
        }
        let bt = &tf.block_types[name];
        if let Some(f) = fields.iter().find(|f| same_level(f) && snake(&f.name) == *name && f.kind != ApiKind::Leaf) {
            used.insert(f.path.clone());
            tm.exact += 1;
            align_block(fields, &bt.block, &tf_path(name), &f.path, tm, used);
            continue;
        }
        // renamed: an array-of-objects (or object) under this subtree whose
        // properties overlap the block's names — the best unique overlap wins
        let block_names: std::collections::BTreeSet<String> =
            bt.block.attributes.keys().chain(bt.block.block_types.keys()).cloned().collect();
        let under = |f: &ApiField| api_prefix.is_empty() || f.path.starts_with(&format!("{}.", api_prefix));
        let mut scored: Vec<(usize, &ApiField)> = fields
            .iter()
            .filter(|f| under(f) && f.kind != ApiKind::Leaf && !used.contains(&f.path))
            .map(|f| (f.props.iter().filter(|p| block_names.contains(&snake(p))).count(), f))
            .filter(|(n, f)| *n > 0 && *n * 2 >= block_names.len().min(f.props.len()).max(1))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        match scored.as_slice() {
            [(n, f), rest @ ..] if rest.first().map(|(m, _)| m < n).unwrap_or(true) => {
                tm.map.insert(f.path.clone(), tf_path(name));
                tm.how.insert(f.path.clone(), "renamed".into());
                used.insert(f.path.clone());
                align_block(fields, &bt.block, &tf_path(name), &f.path, tm, used);
            }
            _ => tm.tf_only.push(tf_path(name)),
        }
    }
}

/// Apply a map to API-shaped data: every source path present is moved to
/// its Terraform path (dotted; created as nested mappings). Done before
/// snake-casing, on the API's own key spelling.
pub fn apply_map(data: &mut serde_yaml::Mapping, map: &BTreeMap<String, String>) {
    // longest source paths first so a moved parent does not hide its children
    let mut rows: Vec<(&String, &String)> = map.iter().collect();
    rows.sort_by(|a, b| b.0.matches('.').count().cmp(&a.0.matches('.').count()));
    for (src, dst) in rows {
        if let Some(v) = take_path(data, src) {
            put_path(data, dst, v);
        }
    }
    prune_empty(data);
}

fn take_path(data: &mut serde_yaml::Mapping, path: &str) -> Option<serde_yaml::Value> {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let key = serde_yaml::Value::String(head.to_string());
    match rest {
        None => data.remove(&key),
        Some(r) => match data.get_mut(&key) {
            Some(serde_yaml::Value::Mapping(m)) => take_path(m, r),
            _ => None,
        },
    }
}

fn put_path(data: &mut serde_yaml::Mapping, path: &str, v: serde_yaml::Value) {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let key = serde_yaml::Value::String(head.to_string());
    match rest {
        None => {
            data.insert(key, v);
        }
        Some(r) => {
            let entry = data.entry(key).or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            if let serde_yaml::Value::Mapping(m) = entry {
                put_path(m, r, v);
            }
        }
    }
}

fn prune_empty(data: &mut serde_yaml::Mapping) {
    let keys: Vec<serde_yaml::Value> = data.keys().cloned().collect();
    for k in keys {
        if let Some(serde_yaml::Value::Mapping(m)) = data.get_mut(&k) {
            prune_empty(m);
            if m.is_empty() {
                data.remove(&k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tf() -> BlockSchema {
        serde_json::from_value(serde_json::json!({
            "attributes": {
                "name": {"type": "string", "required": true},
                "storage_class": {"type": "string", "optional": true},
                "uniform_bucket_level_access": {"type": "bool", "optional": true, "computed": true},
                "public_access_prevention": {"type": "string", "optional": true, "computed": true},
                "requester_pays": {"type": "bool", "optional": true},
                "force_destroy": {"type": "bool", "optional": true},
                "self_link": {"type": "string", "computed": true}
            },
            "block_types": {
                "versioning": {"nesting_mode": "list", "block": {"attributes": {"enabled": {"type": "bool", "required": true}}}},
                "lifecycle_rule": {"nesting_mode": "list", "block": {"block_types": {
                    "action": {"nesting_mode": "set", "block": {"attributes": {"type": {"type": "string", "required": true}, "storage_class": {"type": "string", "optional": true}}}},
                    "condition": {"nesting_mode": "set", "block": {"attributes": {"age": {"type": "number", "optional": true}}}}
                }}},
                "timeouts": {"nesting_mode": "single", "block": {"attributes": {"create": {"type": "string", "optional": true}}}}
            }
        }))
        .unwrap()
    }

    fn api() -> serde_json::Value {
        serde_json::json!({
            "Bucket": {"type": "object", "properties": {
                "name": {"type": "string"},
                "storageClass": {"type": "string"},
                "kind": {"type": "string"},
                "selfLink": {"type": "string"},
                "billing": {"type": "object", "properties": {"requesterPays": {"type": "boolean"}}},
                "iamConfiguration": {"type": "object", "properties": {
                    "uniformBucketLevelAccess": {"type": "object", "properties": {"enabled": {"type": "boolean"}, "lockedTime": {"type": "string"}}},
                    "publicAccessPrevention": {"type": "string"}
                }},
                "versioning": {"type": "object", "properties": {"enabled": {"type": "boolean"}}},
                "lifecycle": {"type": "object", "properties": {
                    "rule": {"type": "array", "items": {"type": "object", "properties": {
                        "action": {"type": "object", "properties": {"type": {"type": "string"}, "storageClass": {"type": "string"}}},
                        "condition": {"type": "object", "properties": {"age": {"type": "integer"}}}
                    }}}
                }},
                "acl": {"type": "array", "items": {"$ref": "BucketAccessControl"}}
            }},
            "BucketAccessControl": {"type": "object", "properties": {"entity": {"type": "string"}}}
        })
    }

    #[test]
    fn bucket_aligns_into_exact_flattened_renamed_and_unmatched() {
        let schemas = api();
        let tm = align(&schemas["Bucket"], &schemas, &tf());
        assert_eq!(tm.map.get("iamConfiguration.uniformBucketLevelAccess.enabled").map(String::as_str), Some("uniform_bucket_level_access"));
        assert_eq!(tm.map.get("iamConfiguration.publicAccessPrevention").map(String::as_str), Some("public_access_prevention"));
        assert_eq!(tm.map.get("billing.requesterPays").map(String::as_str), Some("requester_pays"));
        assert_eq!(tm.map.get("lifecycle.rule").map(String::as_str), Some("lifecycle_rule"));
        assert_eq!(tm.how.get("lifecycle.rule").map(String::as_str), Some("renamed"));
        assert_eq!(tm.how.get("billing.requesterPays").map(String::as_str), Some("flattened"));
        assert_eq!(tm.unmatched, vec!["acl".to_string(), "kind".to_string()]);
        assert_eq!(tm.tf_only, vec!["force_destroy".to_string()]);
        // name, storage_class, versioning(+enabled), lifecycle_rule's action/condition (+ their attrs), self_link skipped as output
        assert!(tm.exact >= 8, "{}", tm.exact);
    }

    #[test]
    fn apply_moves_values_and_prunes_the_emptied_wrappers() {
        let schemas = api();
        let tm = align(&schemas["Bucket"], &schemas, &tf());
        let mut data: serde_yaml::Mapping = serde_yaml::from_str(
            "name: b\niamConfiguration:\n  uniformBucketLevelAccess:\n    enabled: true\n    lockedTime: x\n  publicAccessPrevention: enforced\nbilling:\n  requesterPays: false\nlifecycle:\n  rule:\n    - action: {type: Delete}\n      condition: {age: 30}\n",
        )
        .unwrap();
        apply_map(&mut data, &tm.map);
        let out = serde_yaml::to_string(&data).unwrap();
        assert!(out.contains("uniform_bucket_level_access: true"), "{}", out);
        assert!(out.contains("public_access_prevention: enforced"), "{}", out);
        assert!(out.contains("requester_pays: false"), "{}", out);
        assert!(out.contains("lifecycle_rule:\n- action:"), "{}", out);
        assert!(!out.contains("billing:"), "emptied wrapper pruned:\n{}", out);
        assert!(out.contains("lockedTime: x"), "an unmapped sibling stays where it was (the schema filter drops it later):\n{}", out);
    }
}
