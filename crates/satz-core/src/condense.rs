//! The shaping pass every import shape runs before the printer.
//!
//! Discovery hands the printer API-shaped data: a single nested block as a
//! one-element list, the organization number as a literal in every path and
//! import id, a project's `name` repeating its `project_id`. The language
//! reference documents the forms a person writes — `spec { … }`,
//! `"organizations/{customer_organization_id}"`, a project without a
//! redundant `name` — and this pass is where the document takes them, so the
//! live, state and HCL shapes print alike. Schema questions come in as
//! closures: satz-core has no provider schema of its own.

use crate::migrate::{interpolation, param_ref};

/// The param every estate declares for its organization.
pub const ORG_PARAM: &str = "customer_organization_id";

/// What the pass needs to know from outside.
pub struct Shaping<'a> {
    /// The full type name behind a document key (`folder` → `google_folder`,
    /// `google_storage_bucket` → itself), `None` for a key that is not a
    /// resource type (`display_name`, `labels`, `project_service`).
    pub type_of: &'a dyn Fn(&str) -> Option<String>,
    /// Whether the nested block at `path` of a resource type is a single block
    /// — `max_items: 1` or `nesting_mode: single` in the provider schema. The
    /// path is `/`-joined from the resource's body (`spec`, `spec/rules/condition`).
    pub single_block: &'a dyn Fn(&str, &str) -> bool,
    /// The estate's organization number. Every literal naming it becomes the
    /// `customer_organization_id` reference; `None` leaves the literals.
    pub organization: Option<&'a str>,
}

/// Shape a discovered document in place.
pub fn condense(top: &mut serde_yaml::Mapping, s: &Shaping<'_>) {
    shape_container(top, s, true);
    if let Some(org) = s.organization {
        for (_, v) in top.iter_mut() {
            reference_organization(v, org);
        }
    }
}

/// A body that holds resource-type maps beside plain attributes: the top
/// level, a folder, a project.
fn shape_container(body: &mut serde_yaml::Mapping, s: &Shaping<'_>, at_top: bool) {
    for (k, v) in body.iter_mut() {
        let Some(key) = k.as_str() else { continue };
        let Some(tf_type) = (s.type_of)(key) else { continue };
        let Some(by_label) = v.as_mapping_mut() else { continue };
        for (_, entry) in by_label.iter_mut() {
            // a grant map's member line is a list of roles, not a body
            let Some(entry) = entry.as_mapping_mut() else { continue };
            if is_container(&tf_type) {
                shape_container(entry, s, false);
                if tf_type == "google_project" {
                    drop_redundant_project_name(entry);
                }
            } else {
                if tf_type == "google_org_policy_policy" {
                    bare_policy(entry, at_top, s.organization);
                }
                shape_resource(&tf_type, entry, "", s);
            }
        }
    }
}

/// An org policy names its constraint bare (`compute.managed.requireOsLogin`,
/// transformation 9) and inherits `parent` from the enclosing scope. A state
/// carries the full name and the parent as attributes; a top-level policy
/// whose parent is the organization loses it, one under a folder or project
/// keeps a parent that is not that node's (the pass cannot tell) — the live
/// shape never writes one.
fn bare_policy(policy: &mut serde_yaml::Mapping, at_top: bool, organization: Option<&str>) {
    if let Some(serde_yaml::Value::String(name)) = policy.get_mut("name") {
        if let Some((_, constraint)) = name.rsplit_once("/policies/") {
            *name = constraint.to_string();
        }
    }
    if at_top {
        let org_parent = matches!(
            (policy.get("parent"), organization),
            (Some(serde_yaml::Value::String(p)), Some(org)) if p.trim_start_matches("organizations/") == org
        );
        if org_parent {
            policy.remove("parent");
        }
    }
}

fn is_container(tf_type: &str) -> bool {
    matches!(tf_type, "google_folder" | "google_project")
}

/// A project's `name` defaults to its `project_id` at emission; written equal
/// it says nothing.
fn drop_redundant_project_name(project: &mut serde_yaml::Mapping) {
    let same = matches!(
        (project.get("name"), project.get("project_id")),
        (Some(serde_yaml::Value::String(n)), Some(serde_yaml::Value::String(id))) if n == id
    );
    if same {
        project.remove("name");
    }
}

/// One resource body: a one-element list of one mapping under a single block
/// type becomes the block itself, at every depth.
fn shape_resource(tf_type: &str, body: &mut serde_yaml::Mapping, path: &str, s: &Shaping<'_>) {
    for (k, v) in body.iter_mut() {
        let Some(key) = k.as_str() else { continue };
        let at = if path.is_empty() { key.to_string() } else { format!("{}/{}", path, key) };
        let single = matches!(v, serde_yaml::Value::Sequence(items)
            if items.len() == 1 && items[0].is_mapping() && (s.single_block)(tf_type, &at));
        if single {
            let serde_yaml::Value::Sequence(mut items) = std::mem::take(v) else { unreachable!("matched a sequence") };
            *v = items.remove(0);
        }
        match v {
            serde_yaml::Value::Sequence(items) => {
                for item in items.iter_mut() {
                    if let Some(m) = item.as_mapping_mut() {
                        shape_resource(tf_type, m, &at, s);
                    }
                }
            }
            serde_yaml::Value::Mapping(m) => shape_resource(tf_type, m, &at, s),
            _ => {}
        }
    }
}

/// Every string naming the organization, anywhere in the value, becomes the
/// param reference — bare where the string IS the number (`org_id`), an
/// interpolation where it sits in a path (`organizations/<n>/policies/x`) or
/// leads an organization grant's import id (`<n> roles/x member`). Keys are
/// never touched: a member is a principal, not a reference.
fn reference_organization(v: &mut serde_yaml::Value, org: &str) {
    match v {
        serde_yaml::Value::String(s) => {
            if let Some(r) = organization_reference(s, org) {
                *v = r;
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items.iter_mut() {
                reference_organization(item, org);
            }
        }
        serde_yaml::Value::Mapping(m) => {
            for (_, item) in m.iter_mut() {
                reference_organization(item, org);
            }
        }
        _ => {}
    }
}

fn organization_reference(s: &str, org: &str) -> Option<serde_yaml::Value> {
    if org.is_empty() {
        return None;
    }
    if s == org {
        return Some(param_ref(ORG_PARAM));
    }
    let needle = format!("organizations/{}", org);
    if s.contains(&needle) {
        let chunks: Vec<&str> = s.split(needle.as_str()).collect();
        // a longer number with this one as its prefix is another organization
        if chunks[1..].iter().any(|c| c.chars().next().is_some_and(|ch| ch.is_ascii_digit())) {
            return None;
        }
        let mut parts = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                parts.push(serde_yaml::Value::String("organizations/".into()));
                parts.push(param_ref(ORG_PARAM));
            }
            if !chunk.is_empty() {
                parts.push(serde_yaml::Value::String((*chunk).to_string()));
            }
        }
        return Some(interpolation(parts));
    }
    if let Some(rest) = s.strip_prefix(org) {
        if rest.starts_with(' ') {
            return Some(interpolation(vec![param_ref(ORG_PARAM), serde_yaml::Value::String(rest.to_string())]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::convert_value;

    fn doc(yaml: &str) -> serde_yaml::Mapping {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn type_of(k: &str) -> Option<String> {
        match k {
            "folder" => Some("google_folder".into()),
            "project" => Some("google_project".into()),
            "org_policy_policy" => Some("google_org_policy_policy".into()),
            k if k.starts_with("google_") => Some(k.to_string()),
            _ => None,
        }
    }

    fn single_block(tf_type: &str, path: &str) -> bool {
        matches!((tf_type, path), ("google_org_policy_policy", "spec") | ("google_storage_bucket", "versioning"))
    }

    fn print(top: &serde_yaml::Mapping) -> String {
        convert_value(top, "estate", "t", &[], &[]).unwrap()
    }

    #[test]
    fn a_single_block_prints_as_a_block_and_a_repeated_one_stays_a_list() {
        let mut top = doc(
            "org_policy_policy:\n  x:\n    name: compute.requireOsLogin\n    spec:\n      - rules:\n          - enforce: 'TRUE'\n",
        );
        condense(&mut top, &Shaping { type_of: &type_of, single_block: &single_block, organization: None });
        let text = print(&top);
        assert!(text.contains("    spec {\n"), "{}", text);
        assert!(text.contains("rules = [\n"), "rules is a repeated block type:\n{}", text);
        assert!(!text.contains("spec = ["), "{}", text);
    }

    #[test]
    fn a_state_shaped_policy_becomes_the_bare_constraint_without_its_organization_parent() {
        let mut top = doc(
            "org_policy_policy:\n  x:\n    import-id: organizations/123456789012/policies/compute.requireOsLogin\n    name: organizations/123456789012/policies/compute.requireOsLogin\n    parent: organizations/123456789012\n    spec:\n      - rules:\n          - enforce: 'TRUE'\nfolder:\n  f:\n    display_name: F\n    org_policy_policy:\n      y:\n        name: folders/1/policies/compute.requireOsLogin\n        parent: folders/1\n",
        );
        condense(&mut top, &Shaping { type_of: &type_of, single_block: &single_block, organization: Some("123456789012") });
        let text = print(&top);
        assert!(text.contains("name = \"compute.requireOsLogin\"\n"), "{}", text);
        assert!(!text.contains("parent = \"organizations/"), "a top-level policy's organization parent is derived:\n{}", text);
        assert!(text.contains("parent = \"folders/1\""), "a nested policy's parent stays — the pass cannot prove it is the node's:\n{}", text);
        assert!(text.contains("\"import-id\" = \"organizations/{customer_organization_id}/policies/compute.requireOsLogin\""), "{}", text);
    }

    #[test]
    fn a_single_block_is_found_inside_a_folder_and_a_project() {
        let mut top = doc(
            "folder:\n  f:\n    display_name: F\n    project:\n      p:\n        project_id: p\n        name: p\n        google_storage_bucket:\n          b:\n            name: b\n            versioning:\n              - enabled: true\n",
        );
        condense(&mut top, &Shaping { type_of: &type_of, single_block: &single_block, organization: None });
        let text = print(&top);
        assert!(text.contains("versioning {\n"), "{}", text);
        assert!(!text.contains("name = \"p\""), "a name equal to the project id is dropped:\n{}", text);
        assert!(text.contains("project_id = \"p\""), "{}", text);
    }

    #[test]
    fn a_project_name_that_differs_stays() {
        let mut top = doc("project:\n  p:\n    project_id: p\n    name: Production\n");
        condense(&mut top, &Shaping { type_of: &type_of, single_block: &single_block, organization: None });
        assert!(print(&top).contains("name = \"Production\""));
    }

    #[test]
    fn the_organization_number_becomes_the_param_reference_everywhere() {
        let mut top = doc(
            "google_logging_organization_sink:\n  s:\n    org_id: '123456789012'\n    import-id: organizations/123456789012/sinks/s\n    destination: logging.googleapis.com/organizations/123456789012/locations/global/buckets/_Default\ngoogle_organization_iam_member:\n  'group:a@example.com':\n    - role: roles/viewer\n      import-id: 123456789012 roles/viewer group:a@example.com\ngoogle_essential_contacts_contact:\n  c:\n    parent: organizations/123456789012\n    email: a@example.com\n",
        );
        condense(&mut top, &Shaping { type_of: &type_of, single_block: &single_block, organization: Some("123456789012") });
        let text = print(&top);
        assert!(text.contains("org_id = customer_organization_id\n"), "bare where the string is the number:\n{}", text);
        assert!(text.contains("\"import-id\" = \"organizations/{customer_organization_id}/sinks/s\""), "{}", text);
        assert!(text.contains("logging.googleapis.com/organizations/{customer_organization_id}/locations/global/buckets/_Default"), "{}", text);
        assert!(text.contains("\"import-id\" = \"{customer_organization_id} roles/viewer group:a@example.com\""), "{}", text);
        assert!(text.contains("parent = \"organizations/{customer_organization_id}\""), "{}", text);
        assert!(text.contains("\"group:a@example.com\" = ["), "a member key is never rewritten:\n{}", text);
        assert!(!text.contains("123456789012"), "{}", text);
    }

    #[test]
    fn another_organization_with_this_number_as_a_prefix_is_left_alone() {
        assert_eq!(organization_reference("organizations/12345/x", "1234"), None);
        assert_eq!(organization_reference("serviceAccount:service-org-123456789012@gcp-sa-x.iam.gserviceaccount.com", "123456789012"), None);
        assert_eq!(organization_reference("x", ""), None);
    }
}
