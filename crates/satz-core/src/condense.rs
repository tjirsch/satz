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

/// A literal the document repeats and the param that names it: every
/// occurrence bounded by non-identifier characters (or the string's ends)
/// becomes a reference — bare where the whole value is the literal, an
/// interpolation where it sits inside one. The organization number, a
/// customer's domain, the infra project id, a region are all of this kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    pub literal: String,
    pub param: String,
}

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
    /// Further literals to reference, the estate's own vocabulary (a bound
    /// `customer_domain`, `infra_project_name`, …).
    pub substitutions: &'a [Substitution],
}

/// Shape a discovered document in place.
pub fn condense(top: &mut serde_yaml::Mapping, s: &Shaping<'_>) {
    shape_container(top, s, true);
    let mut table: Vec<Substitution> = s.substitutions.to_vec();
    if let Some(org) = s.organization {
        if !org.is_empty() {
            table.push(Substitution { literal: org.to_string(), param: ORG_PARAM.to_string() });
        }
    }
    if !table.is_empty() {
        // longest literal first, so `svc-iac-001-users` is the group and not
        // the account followed by `-users`
        table.sort_by(|a, b| b.literal.len().cmp(&a.literal.len()).then(a.literal.cmp(&b.literal)));
        table.dedup_by(|a, b| a.literal == b.literal);
        reference_params(top, &table);
    }
}

/// Every string in the document, the member keys of grant maps included
/// (a member is `"user:{first_admin}@{customer_domain}"` in a written
/// estate), through the substitution table. Labels — the keys of resource
/// maps and containers — are never rewritten: an address stays literal.
fn reference_params(body: &mut serde_yaml::Mapping, table: &[Substitution]) {
    let entries: Vec<(serde_yaml::Value, serde_yaml::Value)> = std::mem::take(body).into_iter().collect();
    for (k, mut v) in entries {
        reference_in_value(&mut v, table);
        let key = match (&k, &v) {
            // a member line: the key is a principal, the value its roles
            (serde_yaml::Value::String(member), serde_yaml::Value::Sequence(_)) => {
                substitute(member, table).unwrap_or(k)
            }
            _ => k,
        };
        body.insert(key, v);
    }
}

fn reference_in_value(v: &mut serde_yaml::Value, table: &[Substitution]) {
    match v {
        serde_yaml::Value::String(s) => {
            if let Some(r) = substitute(s, table) {
                *v = r;
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items.iter_mut() {
                reference_in_value(item, table);
            }
        }
        serde_yaml::Value::Mapping(m) => reference_params(m, table),
        _ => {}
    }
}

fn is_boundary(c: Option<char>) -> bool {
    !c.is_some_and(|c| c.is_ascii_alphanumeric())
}

/// The value `s` becomes with the table applied, `None` when nothing in it is
/// a table literal. One left-to-right pass; at each position the longest
/// literal that fits with boundaries on both sides wins, and a substituted
/// span is never rescanned.
pub fn substitute(s: &str, table: &[Substitution]) -> Option<serde_yaml::Value> {
    if let Some(sub) = table.iter().find(|sub| sub.literal == s) {
        return Some(param_ref(&sub.param));
    }
    let mut parts: Vec<serde_yaml::Value> = Vec::new();
    let mut literal = String::new();
    let mut i = 0;
    let mut hit = false;
    while i < s.len() {
        let before = s[..i].chars().next_back();
        let found = table.iter().find(|sub| {
            !sub.literal.is_empty()
                && s[i..].starts_with(sub.literal.as_str())
                && is_boundary(before)
                && is_boundary(s[i + sub.literal.len()..].chars().next())
        });
        match found {
            Some(sub) => {
                if !literal.is_empty() {
                    parts.push(serde_yaml::Value::String(std::mem::take(&mut literal)));
                }
                parts.push(param_ref(&sub.param));
                i += sub.literal.len();
                hit = true;
            }
            None => {
                let ch_len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                literal.push_str(&s[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    if !hit {
        return None;
    }
    if !literal.is_empty() {
        parts.push(serde_yaml::Value::String(literal));
    }
    Some(interpolation(parts))
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

    fn shaping<'a>(organization: Option<&'a str>, substitutions: &'a [Substitution]) -> Shaping<'a> {
        Shaping { type_of: &type_of, single_block: &single_block, organization, substitutions }
    }

    fn sub(literal: &str, param: &str) -> Substitution {
        Substitution { literal: literal.into(), param: param.into() }
    }

    #[test]
    fn a_single_block_prints_as_a_block_and_a_repeated_one_stays_a_list() {
        let mut top = doc(
            "org_policy_policy:\n  x:\n    name: compute.requireOsLogin\n    spec:\n      - rules:\n          - enforce: 'TRUE'\n",
        );
        condense(&mut top, &shaping(None, &[]));
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
        condense(&mut top, &shaping(Some("123456789012"), &[]));
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
        condense(&mut top, &shaping(None, &[]));
        let text = print(&top);
        assert!(text.contains("versioning {\n"), "{}", text);
        assert!(!text.contains("name = \"p\""), "a name equal to the project id is dropped:\n{}", text);
        assert!(text.contains("project_id = \"p\""), "{}", text);
    }

    #[test]
    fn a_project_name_that_differs_stays() {
        let mut top = doc("project:\n  p:\n    project_id: p\n    name: Production\n");
        condense(&mut top, &shaping(None, &[]));
        assert!(print(&top).contains("name = \"Production\""));
    }

    #[test]
    fn the_organization_number_becomes_the_param_reference_everywhere() {
        let mut top = doc(
            "google_logging_organization_sink:\n  s:\n    org_id: '123456789012'\n    import-id: organizations/123456789012/sinks/s\n    destination: logging.googleapis.com/organizations/123456789012/locations/global/buckets/_Default\ngoogle_organization_iam_member:\n  'group:a@example.com':\n    - role: roles/viewer\n      import-id: 123456789012 roles/viewer group:a@example.com\ngoogle_essential_contacts_contact:\n  c:\n    parent: organizations/123456789012\n    email: a@example.com\n",
        );
        condense(&mut top, &shaping(Some("123456789012"), &[]));
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
    fn a_literal_is_referenced_only_between_boundaries_and_the_longest_wins() {
        let table = [
            sub("svc-iac-001-users", "svc_iac_users_group"),
            sub("acme-infra-001", "infra_project_name"),
            sub("europe-west3-a", "default_zone"),
            sub("europe-west3", "default_region"),
            sub("svc-iac-001", "svc_iac_account"),
            sub("example.com", "customer_domain"),
            sub("alice", "first_admin"),
            sub("acme", "customer_shortname"),
            sub("1234", ORG_PARAM),
        ];
        let text = |s: &str| convert_value(&doc(&format!("google_x:\n  y:\n    v: '{}'\n", s)), "estate", "t", &[], &[]).unwrap();
        let mut top = doc("google_x:\n  y:\n    v: 'serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com'\n");
        condense(&mut top, &shaping(None, &table));
        assert!(print(&top).contains("v = \"serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com\""), "{}", print(&top));
        let mut top = doc("google_x:\n  y:\n    v: 'group:svc-iac-001-users@example.com'\n");
        condense(&mut top, &shaping(None, &table));
        assert!(print(&top).contains("v = \"group:{svc_iac_users_group}@{customer_domain}\""), "the longest literal wins:\n{}", print(&top));
        let mut top = doc("google_x:\n  y:\n    v: acme-log-001\n    w: acmecorp-log\n    z: europe-west3-a\n");
        condense(&mut top, &shaping(None, &table));
        let out = print(&top);
        assert!(out.contains("v = \"{customer_shortname}-log-001\""), "{}", out);
        assert!(out.contains("w = \"acmecorp-log\""), "no boundary, no reference:\n{}", out);
        assert!(out.contains("z = default_zone\n"), "a whole value is a bare reference:\n{}", out);
        assert!(text("organizations/12345/x").contains("organizations/12345/x"), "a longer number is another organization");
        let mut top = doc("google_x:\n  y:\n    v: organizations/12345/x\n");
        condense(&mut top, &shaping(Some("1234"), &[]));
        assert!(print(&top).contains("v = \"organizations/12345/x\""), "{}", print(&top));
    }

    #[test]
    fn a_member_key_is_rewritten_and_a_label_is_not() {
        let table = [sub("example.com", "customer_domain"), sub("alice", "first_admin")];
        let mut top = doc(
            "google_organization_iam_member:\n  'user:alice@example.com':\n    - roles/viewer\ngoogle_storage_bucket:\n  alice:\n    name: alice-bucket-001\n",
        );
        condense(&mut top, &shaping(None, &table));
        let out = print(&top);
        assert!(out.contains("\"user:{first_admin}@{customer_domain}\" = ["), "{}", out);
        assert!(out.contains("\n  alice {\n"), "a label stays literal:\n{}", out);
        assert!(out.contains("name = \"{first_admin}-bucket-001\""), "{}", out);
    }
}
