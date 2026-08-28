use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct PlanJson {
    resource_changes: Option<Vec<ResourceChange>>,
}

#[derive(Debug, Deserialize)]
struct ResourceChange {
    address: String,
    #[serde(rename = "type")]
    resource_type: String,
    change: Change,
}

#[derive(Debug, Deserialize)]
struct Change {
    actions: Vec<String>,
    before: Option<Value>,
    after: Option<Value>,
}

pub fn scan_plan(plan_path: &Path) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let content = crate::fsx::read_to_string(plan_path)?;
    let plan: PlanJson = serde_json::from_str(&content)?;
    
    let changes = plan.resource_changes.unwrap_or_default();
    
    let mut deletions = Vec::new();
    let mut creations = Vec::new();
    
    for rc in &changes {
        if rc.change.actions.contains(&"delete".to_string()) && rc.change.actions.len() == 1 {
            deletions.push(rc);
        } else if rc.change.actions.contains(&"create".to_string()) && rc.change.actions.len() == 1 {
            creations.push(rc);
        }
    }
    
    let mut mapping = HashMap::new();
    let mut matched_creations = std::collections::HashSet::new();
    
    for del in &deletions {
        let best_match = creations.iter()
            .enumerate()
            .filter(|(idx, _)| !matched_creations.contains(idx))
            .filter(|(_, cre)| cre.resource_type == del.resource_type)
            .find(|(_, cre)| {
                if let (Some(before), Some(after)) = (&del.change.before, &cre.change.after) {
                    is_match(&del.resource_type, before, after)
                } else {
                    false
                }
            });
            
        if let Some((idx, cre)) = best_match {
            mapping.insert(del.address.clone(), cre.address.clone());
            matched_creations.insert(idx);
        } else {
            // Fallback: match by normalized address if attribute matching failed
            let del_norm = normalize_address(&del.address);
            let best_addr_match = creations.iter()
                .enumerate()
                .filter(|(idx, _)| !matched_creations.contains(idx))
                .filter(|(_, cre)| cre.resource_type == del.resource_type)
                .find(|(_, cre)| {
                    let cre_norm = normalize_address(&cre.address);
                    del_norm == cre_norm || del_norm.contains(&cre_norm) || cre_norm.contains(&del_norm)
                });
            
            if let Some((idx, cre)) = best_addr_match {
                mapping.insert(del.address.clone(), cre.address.clone());
                matched_creations.insert(idx);
            }
        }
    }

    if mapping.is_empty() {
        println!("No matches found. Deletions: {}, Creations: {}", deletions.len(), creations.len());
        if !deletions.is_empty() && !creations.is_empty() {
            println!("Example deletion: {} ({})", deletions[0].address, deletions[0].resource_type);
            println!("Example creation: {} ({})", creations[0].address, creations[0].resource_type);
        }
    } else {
        println!("Found {} resource renames out of {} deleted and {} created resources", mapping.len(), deletions.len(), creations.len());
    }
    
    Ok(mapping)
}

fn is_match(res_type: &str, before: &Value, after: &Value) -> bool {
    let keys_to_check = match res_type {
        "google_project" => vec!["project_id", "folder_id", "org_id", "name"],
        "google_folder" => vec!["display_name", "parent"],
        "google_project_service" => vec!["project", "service"],
        "google_service_account" => vec!["account_id", "project"],
        "google_storage_bucket" => vec!["name", "project"],
        "google_storage_bucket_iam_member" => vec!["bucket", "member", "role"],
        "google_bigquery_dataset" => vec!["dataset_id", "project"],
        "google_bigquery_data_transfer_config" => vec!["display_name", "data_source_id", "project"],
        "google_pubsub_subscription" => vec!["name", "topic", "project"],
        "google_pubsub_topic" => vec!["name", "project"],
        "google_cloud_identity_group" => vec!["display_name", "description"],
        "google_org_policy_policy" => vec!["name", "parent"],
        t if t.contains("iam_member") || t.contains("iam_binding") => {
            vec!["member", "role", "project", "folder", "org_id", "members", "bucket"]
        },
        _ => vec!["name", "id", "project_id", "bucket", "dataset_id"],
    };
    
    // Special handling for group_key in cloud identity groups
    if res_type == "google_cloud_identity_group" {
        if let (Some(b_gk), Some(a_gk)) = (before.get("group_key"), after.get("group_key")) {
            if let (Some(b_seq), Some(a_seq)) = (b_gk.as_array(), a_gk.as_array()) {
                if !b_seq.is_empty() && !a_seq.is_empty() {
                    let b_id = b_seq[0].get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let a_id = a_seq[0].get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if b_id != a_id && !b_id.is_empty() && !a_id.is_empty() {
                        return false;
                    }
                }
            }
        }
    }

    for key in keys_to_check {
        let b_val = before.get(key);
        let a_val = after.get(key);
        
        match (b_val, a_val) {
            (Some(b), Some(a)) => {
                let b_str = val_to_string(b).to_lowercase();
                let a_str = val_to_string(a).to_lowercase();
                if b_str != a_str && !fuzzy_match(&b_str, &a_str) {
                    return false;
                }
            }
            // Relaxed: only fail if both exist and don't match, or if one exists and is significantly different
            // (e.g. not null or empty). Actually, let's just ignore if one is missing for now, 
            // as defaults can cause mismatches in the plan JSON.
            _ => {}
        }
    }
    true
}

fn val_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "".to_string(),
        _ => v.to_string(),
    }
}

fn fuzzy_match(s1: &str, s2: &str) -> bool {
    if s1 == s2 { return true; }
    
    // Handle cases like "projects/foo" match "foo"
    // Also handle "organizations/123/policies/foo" match "foo"
    let prefixes = ["projects/", "folders/", "organizations/", "locations/", "groups/", "b/"];
    let clean_s1 = s1;
    let clean_s2 = s2;
    fn strip_prefixes<'a>(mut s: &'a str, prefixes: &[&str]) -> &'a str {
        let mut changed = true;
        while changed {
            changed = false;
            for p in prefixes {
                if s.starts_with(p) {
                    s = &s[p.len()..];
                    changed = true;
                    break;
                }
            }
            // Also strip "/policies/" if it appears in the middle
            if let Some(pos) = s.find("/policies/") {
                s = &s[pos + 10..];
                changed = true;
            }
            // Also strip numbers if they are between slashes
            // (simplistic approach: just keep the last part)
            if let Some(pos) = s.rfind('/') {
                s = &s[pos + 1..];
                changed = true;
            }
        }
        s
    }

    let s1_clean = strip_prefixes(clean_s1, &prefixes);
    let s2_clean = strip_prefixes(clean_s2, &prefixes);
    
    s1_clean == s2_clean
}

fn normalize_address(addr: &str) -> String {
    // Strip resource type prefix if present
    let parts: Vec<&str> = addr.split('.').collect();
    let name = if parts.len() > 1 { parts[1] } else { parts[0] };
    
    let mut normalized = name.to_lowercase();
    
    // Strip common stack prefixes
    if normalized.starts_with("ci-") {
        normalized = normalized[3..].to_string();
    }
    
    // Replace known resource type fragments from old names with underscore
    // This handles "storage-bucket-iam-member" vs "google_storage_bucket_iam_member"
    // and also standardizes separators
    normalized = normalized
        .replace("storage-bucket-iam-member-", "")
        .replace("project-iam-member-", "")
        .replace("organization-iam-member-", "")
        .replace("folder-iam-member-", "")
        .replace("project-service-", "")
        .replace('-', "_");
    
    // Strip trailing hex hash if it's 16 chars long (e.g. _93083ca01ba6149a)
    if normalized.len() > 17 && normalized.as_bytes()[normalized.len()-17] == b'_' {
        let potential_hash = &normalized[normalized.len()-16..];
        if potential_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            normalized.truncate(normalized.len() - 17);
        }
    }
        
    normalized
}

pub fn generate_migration(mapping_path: &Path, output_path: &Path, tf_tool: &str) -> Result<(), Box<dyn std::error::Error>> {
    let content = crate::fsx::read_to_string(mapping_path)?;
    let mapping: HashMap<String, String> = serde_yaml::from_str(&content)?;
    
    let mut script = String::new();
    script.push_str("#!/bin/bash\n");
    script.push_str("#set -e\n\n");
    script.push_str(&format!("TF_TOOL=\"{}\"\n\n", tf_tool));
    
    script.push_str("if ! command -v \"$TF_TOOL\" &> /dev/null; then\n");
    script.push_str("    echo \"Error: $TF_TOOL could not be found\"\n");
    script.push_str("    exit 1\n");
    script.push_str("fi\n\n");
    
    let mut items: Vec<_> = mapping.into_iter().collect();
    items.sort();
    
    for (old, new) in items {
        script.push_str(&format!("\"$TF_TOOL\" state mv '{}' '{}'\n", old, new));
    }
    
    crate::fsx::write(output_path, script)?;
    Ok(())
}
