//! Delta import (`satz import <scope> --into <estate>.satz`): what the live
//! scope has that the estate does not declare.
//!
//! Identity is the LIVE id, never the label — the import names a folder
//! `folder-<n>`, the estate calls it `infra_folder`. The estate's declared
//! resources are resolved to their live ids by the adopt engine (dry, no
//! cloud change); everything the sweep found with one of those ids is
//! subtracted. The remainder is written as packs the estate `use`s: one for
//! the top level, and one per declared CONTAINER (folder/project) that has
//! undeclared children, `use`d from inside that container's block so the
//! fold places the children where they live. The estate is never rewritten
//! beyond those `use` lines; re-running imports only what appeared since.

use std::collections::{BTreeMap, BTreeSet};

use crate::adopt::{Outcome, Resolution};

/// What the estate already covers, by live id.
pub(crate) struct Declared {
    /// live id → declared address
    pub ids: BTreeMap<String, String>,
    /// live id → (address, declaring `label {` line) for folders and projects
    pub containers: BTreeMap<String, (String, Option<(String, u32)>)>,
    /// declared, looked up live, provably absent — `apply` will create these
    pub not_live: Vec<String>,
}

pub(crate) fn declared_from(resolutions: &[Resolution]) -> Declared {
    let mut d = Declared { ids: BTreeMap::new(), containers: BTreeMap::new(), not_live: Vec::new() };
    for r in resolutions {
        let id = match &r.outcome {
            Outcome::AlreadyAdopted(id) | Outcome::Resolved { id, .. } | Outcome::NeedsActivation { id, .. } => id.clone(),
            Outcome::OnApply => {
                d.not_live.push(r.address.clone());
                continue;
            }
            _ => continue,
        };
        if r.tf_type == "google_folder" || r.tf_type == "google_project" {
            d.containers.insert(id.clone(), (r.address.clone(), r.origin.clone()));
        }
        d.ids.insert(id, r.address.clone());
    }
    d
}

/// The remainder of a sweep after subtraction.
#[derive(Default)]
pub(crate) struct Delta {
    /// residue at the top level of the scope
    pub top: serde_yaml::Mapping,
    /// residue under a declared container: declared address → children
    pub under: BTreeMap<String, serde_yaml::Mapping>,
    /// (live id, declared address) of everything subtracted
    pub already: Vec<(String, String)>,
    /// resources kept
    pub new: usize,
}

fn import_id_of(v: &serde_yaml::Value) -> Option<String> {
    v.as_mapping()?.get("import-id")?.as_str().map(String::from)
}

fn grant_id_of(v: &serde_yaml::Value) -> Option<String> {
    import_id_of(v)
}

fn is_container_key(k: &str) -> bool {
    matches!(k, "folder" | "project" | "google_folder" | "google_project")
}

fn is_grant_map_key(k: &str) -> bool {
    k.ends_with("_iam_member") && k != "google_storage_bucket_iam_member" && k != "storage_bucket_iam_member"
}

/// Subtract the declared ids from a discovered document (the `Config` as YAML).
pub(crate) fn subtract(top: serde_yaml::Mapping, declared: &Declared) -> Delta {
    let mut delta = Delta::default();
    let mut top = top;
    prune(&mut top, declared, None, &mut delta);
    delta.top = top;
    delta
}

/// Prune one body in place. `project_ctx` is the enclosing project id, for
/// `project_service` entries whose id is `<project>/<service>`.
fn prune(map: &mut serde_yaml::Mapping, declared: &Declared, project_ctx: Option<&str>, delta: &mut Delta) {
    let keys: Vec<serde_yaml::Value> = map.keys().cloned().collect();
    for key in keys {
        let Some(k) = key.as_str().map(String::from) else { continue };
        let Some(value) = map.get_mut(&key) else { continue };
        if is_container_key(&k) {
            if let serde_yaml::Value::Mapping(nodes) = value {
                let labels: Vec<serde_yaml::Value> = nodes.keys().cloned().collect();
                for label in labels {
                    let Some(serde_yaml::Value::Mapping(node)) = nodes.get_mut(&label) else { continue };
                    let id = import_id_of(&serde_yaml::Value::Mapping(node.clone()));
                    let project_id = node.get("project_id").and_then(|v| v.as_str()).map(String::from);
                    let ctx = if k.contains("project") { project_id.as_deref() } else { project_ctx };
                    match id.as_deref().and_then(|id| declared.containers.get(id).map(|c| (id.to_string(), c.clone()))) {
                        Some((id, (address, _))) => {
                            // declared container: keep only its undeclared children,
                            // and those go under the estate's own block
                            prune(node, declared, ctx, delta);
                            let mut children = serde_yaml::Mapping::new();
                            for (ck, cv) in node.iter() {
                                let cks = ck.as_str().unwrap_or("");
                                let is_attr = !matches!(cv, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_))
                                    || matches!(cks, "labels" | "tags" | "lifecycle");
                                if !is_attr && cks != "import-id" {
                                    children.insert(ck.clone(), cv.clone());
                                }
                            }
                            // `project_service = [...]` is an ATTRIBUTE of the project;
                            // a pack cannot contribute attributes to a block it is
                            // `use`d from. Under a declared project the services become
                            // `google_project_service` resources with their ids.
                            if let Some(serde_yaml::Value::Sequence(services)) = children.remove("project_service") {
                                if let Some(pid) = project_id.as_deref() {
                                    let mut map = serde_yaml::Mapping::new();
                                    for s in services {
                                        let svc = s.as_str().map(String::from).or_else(|| s.as_mapping().and_then(|m| m.get("service")).and_then(|v| v.as_str()).map(String::from));
                                        let Some(svc) = svc else { continue };
                                        let mut body = serde_yaml::Mapping::new();
                                        body.insert("import-id".into(), serde_yaml::Value::String(format!("{}/{}", pid, svc)));
                                        body.insert("service".into(), serde_yaml::Value::String(svc.clone()));
                                        map.insert(serde_yaml::Value::String(svc.replace('.', "_")), serde_yaml::Value::Mapping(body));
                                    }
                                    if !map.is_empty() {
                                        children.insert("google_project_service".into(), serde_yaml::Value::Mapping(map));
                                    }
                                }
                            }
                            if !children.is_empty() {
                                delta.under.entry(address.clone()).or_default().extend(children);
                            }
                            delta.already.push((id, address));
                            nodes.remove(&label);
                        }
                        None => {
                            prune(node, declared, ctx, delta);
                            delta.new += 1;
                        }
                    }
                }
                if nodes.is_empty() {
                    map.remove(&key);
                }
            }
            continue;
        }
        if is_grant_map_key(&k) {
            if let serde_yaml::Value::Mapping(members) = value {
                let member_keys: Vec<serde_yaml::Value> = members.keys().cloned().collect();
                for mk in member_keys {
                    if let Some(serde_yaml::Value::Sequence(roles)) = members.get_mut(&mk) {
                        roles.retain(|r| match grant_id_of(r).and_then(|id| declared.ids.get(&id).map(|a| (id, a.clone()))) {
                            Some((id, address)) => {
                                delta.already.push((id, address));
                                false
                            }
                            None => {
                                delta.new += 1;
                                true
                            }
                        });
                        if roles.is_empty() {
                            members.remove(&mk);
                        }
                    }
                }
                if members.is_empty() {
                    map.remove(&key);
                }
            }
            continue;
        }
        if k == "project_service" {
            if let serde_yaml::Value::Sequence(services) = value {
                services.retain(|s| {
                    let svc = s.as_str().map(String::from).or_else(|| s.as_mapping().and_then(|m| m.get("service")).and_then(|v| v.as_str()).map(String::from));
                    let id = match (project_ctx, svc) {
                        (Some(p), Some(svc)) => format!("{}/{}", p, svc),
                        _ => return true,
                    };
                    match declared.ids.get(&id) {
                        Some(address) => {
                            delta.already.push((id, address.clone()));
                            false
                        }
                        None => {
                            delta.new += 1;
                            true
                        }
                    }
                });
                if services.is_empty() {
                    map.remove(&key);
                }
            }
            continue;
        }
        // a resource-type map: label → body with "import-id"
        if let serde_yaml::Value::Mapping(entries) = value {
            let looks_like_type = entries.values().any(|v| import_id_of(v).is_some());
            if looks_like_type {
                let labels: Vec<serde_yaml::Value> = entries.keys().cloned().collect();
                for label in labels {
                    let Some(v) = entries.get(&label) else { continue };
                    match import_id_of(v).and_then(|id| declared.ids.get(&id).map(|a| (id, a.clone()))) {
                        Some((id, address)) => {
                            delta.already.push((id, address));
                            entries.remove(&label);
                        }
                        None => delta.new += 1,
                    }
                }
                if entries.is_empty() {
                    map.remove(&key);
                }
            }
        }
    }
}

/// The pack file name for a scope: `imported-organizations-123.satz`,
/// `imported-folders-456-infra_folder.satz`.
pub(crate) fn pack_name(scope: &str, container: Option<&str>) -> String {
    let slug: String = scope.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    match container {
        Some(c) => format!("imported-{}-{}.satz", slug, c.replace(['.', '-'], "_")),
        None => format!("imported-{}.satz", slug),
    }
}

/// Add `use "<pack>"` to the estate: at the end of the file for the top-level
/// pack, right after the container's `label {` line for a container pack.
/// Idempotent — a line already present is left alone.
pub(crate) fn add_use(estate_text: &str, pack: &str, after_line: Option<u32>) -> Result<Option<String>, String> {
    let use_line = format!("use \"{}\"", pack);
    if estate_text.lines().any(|l| l.trim() == use_line) {
        return Ok(None);
    }
    let mut lines: Vec<String> = estate_text.lines().map(String::from).collect();
    match after_line {
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(use_line);
        }
        Some(n) => {
            let idx = n as usize - 1;
            let decl = lines.get(idx).ok_or_else(|| format!("line {} is past the end of the estate", n))?;
            if !decl.trim_end().ends_with('{') {
                return Err(format!("line {} does not open a block: {}", n, decl.trim()));
            }
            let indent: String = decl.chars().take_while(|c| c.is_whitespace()).collect();
            lines.insert(idx + 1, format!("{}  {}", indent, use_line));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(Some(out))
}

/// Is this resolution's declaring file one of the packs a delta import
/// wrote? Those are re-derived on every run, so their content must not
/// count as "declared" — or nothing moved out of a pack would ever be
/// subtracted, and an emptied pack would never go away.
pub(crate) fn from_imported_pack(origin: &Option<(String, u32)>) -> bool {
    origin
        .as_ref()
        .and_then(|(f, _)| std::path::Path::new(f).file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("imported-")))
        .unwrap_or(false)
}

/// Drop the `use "<pack>"` line from the estate, wherever it sits.
pub(crate) fn remove_use(estate_text: &str, pack: &str) -> Option<String> {
    let use_line = format!("use \"{}\"", pack);
    if !estate_text.lines().any(|l| l.trim() == use_line) {
        return None;
    }
    let mut out: String = estate_text.lines().filter(|l| l.trim() != use_line).collect::<Vec<_>>().join("\n");
    out.push('\n');
    Some(out)
}

/// Everything the sweep found, by live id — for the "declared but not live"
/// cross-check.
pub(crate) fn live_ids(top: &serde_yaml::Mapping) -> BTreeSet<String> {
    fn walk(v: &serde_yaml::Value, out: &mut BTreeSet<String>) {
        match v {
            serde_yaml::Value::Mapping(m) => {
                if let Some(id) = m.get("import-id").and_then(|v| v.as_str()) {
                    out.insert(id.to_string());
                }
                for x in m.values() {
                    walk(x, out);
                }
            }
            serde_yaml::Value::Sequence(s) => {
                for x in s {
                    walk(x, out);
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(&serde_yaml::Value::Mapping(top.clone()), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y(s: &str) -> serde_yaml::Mapping {
        serde_yaml::from_str(s).unwrap()
    }

    fn declared(ids: &[(&str, &str)], containers: &[(&str, &str, u32)]) -> Declared {
        let mut d = Declared { ids: BTreeMap::new(), containers: BTreeMap::new(), not_live: vec![] };
        for (id, a) in ids {
            d.ids.insert(id.to_string(), a.to_string());
        }
        for (id, a, line) in containers {
            d.ids.insert(id.to_string(), a.to_string());
            d.containers.insert(id.to_string(), (a.to_string(), Some(("main.satz".into(), *line))));
        }
        d
    }

    #[test]
    fn declared_things_are_subtracted_by_id_not_label() {
        let top = y(r#"
org_policy_policy:
  compute-x: { import-id: "organizations/1/policies/compute.x", name: compute.x }
  compute-y: { import-id: "organizations/1/policies/compute.y", name: compute.y }
organization_iam_member:
  "group:a@example.com":
    - { role: roles/viewer, import-id: "1 roles/viewer group:a@example.com" }
    - { role: roles/browser, import-id: "1 roles/browser group:a@example.com" }
folder:
  folder-368:
    import-id: folders/368
    display_name: Infra
    project:
      p1:
        import-id: p1
        project_id: p1
        project_service: [a.googleapis.com, b.googleapis.com]
        google_storage_bucket:
          bkt: { import-id: bkt, name: bkt }
      p2:
        import-id: p2
        project_id: p2
"#);
        let d = declared(
            &[
                ("organizations/1/policies/compute.x", "google_org_policy_policy.x"),
                ("1 roles/viewer group:a@example.com", "google_organization_iam_member.g_a"),
                ("p1/a.googleapis.com", "google_project_service.p1_a"),
            ],
            &[("folders/368", "google_folder.infra_folder", 10), ("p1", "google_project.infra", 14)],
        );
        let delta = subtract(top, &d);
        let top = serde_yaml::to_string(&delta.top).unwrap();
        assert!(top.contains("compute-y") && !top.contains("compute-x"), "{}", top);
        assert!(top.contains("roles/browser") && !top.contains("roles/viewer"), "{}", top);
        assert!(!top.contains("folder-368"), "declared folder is not re-declared:\n{}", top);
        // residue under the declared folder: p2; under the declared project: b.googleapis.com and the bucket
        let under_folder = serde_yaml::to_string(&delta.under["google_folder.infra_folder"]).unwrap();
        assert!(under_folder.contains("p2") && !under_folder.contains("p1:"), "{}", under_folder);
        let under_project = serde_yaml::to_string(&delta.under["google_project.infra"]).unwrap();
        assert!(under_project.contains("google_project_service") && under_project.contains("p1/b.googleapis.com") && !under_project.contains("a.googleapis.com"), "{}", under_project);
        assert!(under_project.contains("bkt"), "{}", under_project);
        assert_eq!(delta.already.len(), 5);
    }

    #[test]
    fn use_lines_are_inserted_once_and_at_the_right_place() {
        let estate = "estate e\n\ngoogle_folder {\n  infra_folder {\n    display_name = \"Infra\"\n  }\n}\n";
        let with_top = add_use(estate, "imported-org.satz", None).unwrap().unwrap();
        assert!(with_top.ends_with("}\n\nuse \"imported-org.satz\"\n"), "{}", with_top);
        assert!(add_use(&with_top, "imported-org.satz", None).unwrap().is_none(), "idempotent");
        let with_inner = add_use(&with_top, "imported-org-infra_folder.satz", Some(4)).unwrap().unwrap();
        assert!(with_inner.contains("  infra_folder {\n    use \"imported-org-infra_folder.satz\"\n"), "{}", with_inner);
        assert!(add_use(&with_inner, "x.satz", Some(5)).is_err(), "not a block line");
    }

    #[test]
    fn use_lines_come_out_again_and_imported_packs_are_recognised() {
        let estate = "estate e\n\ngoogle_folder {\n  infra_folder {\n    use \"imported-org-x.satz\"\n  }\n}\n\nuse \"imported-org.satz\"\n";
        let without_top = remove_use(estate, "imported-org.satz").unwrap();
        assert!(!without_top.contains("imported-org.satz\""), "{}", without_top);
        assert!(without_top.contains("imported-org-x.satz"), "{}", without_top);
        assert!(remove_use(&without_top, "imported-org.satz").is_none(), "idempotent");
        assert!(from_imported_pack(&Some(("./yaml/imported-organizations-1.satz".into(), 3))));
        assert!(!from_imported_pack(&Some(("./yaml/C0example.satz".into(), 3))));
        assert!(!from_imported_pack(&None));
    }

    #[test]
    fn pack_names_are_stable_slugs() {
        assert_eq!(pack_name("organizations/123", None), "imported-organizations-123.satz");
        assert_eq!(pack_name("folders/4", Some("google_folder.infra-x")), "imported-folders-4-google_folder_infra_x.satz");
    }
}
