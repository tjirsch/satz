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
    #[serde(rename = "import-id-comment", skip_serializing_if = "Option::is_none")]
    pub import_id_comment: Option<String>,
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
    #[serde(rename = "import-id-comment", skip_serializing_if = "Option::is_none")]
    pub import_id_comment: Option<String>,
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
pub struct DiscoveryResourceConfig {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DiscoveryConfig {
    pub resource_types: HashMap<String, DiscoveryResourceConfig>,
}
