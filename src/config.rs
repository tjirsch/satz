use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Config {
    // Configuration Blocks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terraform: Option<serde_yaml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<HashMap<String, serde_yaml::Value>>,

    // Organization Level Resources (First in output)
    #[serde(alias = "google_org_policy_policy", skip_serializing_if = "Option::is_none")]
    pub org_policy_policy: Option<HashMap<String, serde_yaml::Value>>,
    #[serde(alias = "google_organization_policy", skip_serializing_if = "Option::is_none")]
    pub google_organization_policy: Option<HashMap<String, serde_yaml::Value>>,
    #[serde(alias = "google_organization_iam_member", skip_serializing_if = "Option::is_none")]
    pub organization_iam_member: Option<HashMap<String, Vec<serde_yaml::Value>>>,
    #[serde(alias = "google_billing_account_iam_member", skip_serializing_if = "Option::is_none")]
    pub billing_account_iam_member: Option<serde_yaml::Value>,

    // Hierarchical Resources
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "google_folder")]
    pub folder: Option<HashMap<String, Folder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "google_project")]
    pub project: Option<HashMap<String, Project>>,

    // Catch-all for other top level fields
    #[serde(flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Folder {
    #[serde(rename = "import-id", skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    // Recursive folder structure
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "google_folder")]
    pub folder: Option<HashMap<String, Folder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "google_project")]
    pub project: Option<HashMap<String, Project>>,

    // Catch-all for other resources in folder
    #[serde(flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Project {
    #[serde(rename = "import-id", skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion_policy: Option<String>,

    // Project specific explicit fields (lists)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_service: Option<Vec<serde_yaml::Value>>,

    // Catch-all for other resources in project
    #[serde(flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImportResourceConfig {
    pub description: String,
    pub import: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derive_yaml_key_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    /// Adoption rule for a type whose Terraform import id is user-chosen: a
    /// template over the emitted resource's attributes and resolved
    /// references, e.g. `projects/{project}/serviceAccounts/{account_id}@…`.
    /// Rendered offline, no API call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    /// Adoption rule for a type whose id GCP assigns: the attributes to match
    /// a live asset on (under the resolved parent), the live `name` being the
    /// import id. Needs `asset_type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_on: Option<Vec<String>>,
    /// `managed`: a missing constraint must be activated before import
    /// (org policies only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activate: Option<String>,
}

/// Where a live import starts. `organization` is required for the live shape;
/// `folder` (by number, or by display-name path from the org) and `project`
/// narrow it. Exactly one of `folder.id` / `folder.path`.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ImportRoot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<FolderRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct FolderRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `import-config.yaml`: what `satz import` reads. YAML on purpose — it is
/// data that configures an import, not an estate. `root` and `only` are the
/// repeatable form of the command line (`satz import <source> --only …`),
/// which overrides them when given.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImportConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<ImportRoot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    pub resource_types: HashMap<String, ImportResourceConfig>,
}

impl ImportConfig {
    /// Restrict the import to the types matching `globs` (`*` wildcard);
    /// every other type has `import` switched off. Returns the names that
    /// were on and are now off, so the run can say what was filtered.
    pub fn apply_only(&mut self, globs: &[String]) -> Vec<String> {
        let mut off = Vec::new();
        for (name, rc) in self.resource_types.iter_mut() {
            if rc.import && !globs.iter().any(|g| glob_match(g, name)) {
                rc.import = false;
                off.push(name.clone());
            }
        }
        off.sort();
        off
    }
}

/// `*`-only glob: `google_*_iam_member` matches every IAM member type.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }
    let mut rest = s;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if !part.is_empty() {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod glob_tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("google_folder", "google_folder"));
        assert!(!glob_match("google_folder", "google_folder_iam_member"));
        assert!(glob_match("google_*_iam_member", "google_folder_iam_member"));
        assert!(glob_match("google_*", "google_project"));
        assert!(!glob_match("google_*_iam_member", "google_project"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn only_switches_off_everything_else_and_names_it() {
        let mut cfg: ImportConfig = serde_yaml::from_str(
            "resource_types:\n  google_folder: {description: f, import: true}\n  google_project: {description: p, import: true}\n  google_x: {description: x, import: false}\n",
        )
        .unwrap();
        let off = cfg.apply_only(&["google_folder".to_string()]);
        assert_eq!(off, vec!["google_project".to_string()]);
        assert!(cfg.resource_types["google_folder"].import);
        assert!(!cfg.resource_types["google_project"].import);
    }
}
