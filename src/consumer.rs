//! `satz check-consumer`: a team's HCL, checked against the estate it reads.
//!
//! A team beside the estate reads `hcl/interfaces/<team>/` and writes to shared
//! infrastructure only through the attach points the estate declares (ADR 0070). This
//! reads the team's `.tf` files offline (`satz_hcl::consumer_blocks`) and holds them to
//! the estate's emission manifest and interface:
//!
//! - an attachment resource whose target is the estate's and is no attach point that
//!   allows its type;
//! - an authoritative grant (`*_iam_policy`, `*_iam_binding`) or an organisation policy
//!   on a node the estate manages — the next apply of either side undoes the other's;
//! - a resource the estate declares too, matched by the natural key the interface looks
//!   it up by (`presets/interface-lookups.yaml`): two states would own one object.
//!
//! What a value points at is read two ways: `module.<name>.<output>` of a module whose
//! `source` ends in `interfaces/<interface>`, and a literal equal to a value the estate
//! publishes or writes. A value that is neither is the team's own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use satz_hcl::{ConsumerBlock, ConsumerValue};

use crate::findings::{Finding, Kind, Severity};
use crate::interface::{self, Interface, Output};
use crate::manifest::{EmittedResource, Manifest};

/// The org-policy types and the argument that names their node.
const ORG_POLICY: &[(&str, &str)] = &[
    ("google_org_policy_policy", "parent"),
    ("google_organization_policy", "org_id"),
    ("google_folder_organization_policy", "folder"),
    ("google_project_organization_policy", "project"),
];

/// Every `.tf` file under `dir`, dot-directories (`.terraform`) left out, in path order.
fn tf_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("{}: no such directory — check-consumer reads a directory of `.tf` files", dir.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {}", d.display(), e))?;
        for e in entries {
            let p = e.map_err(|e| format!("{}: {}", d.display(), e))?.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if p.is_dir() {
                if !name.starts_with('.') {
                    stack.push(p);
                }
            } else if name.ends_with(".tf") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The team's blocks, each located by its file as found under `dir`.
pub(crate) fn read(dir: &Path) -> Result<Vec<ConsumerBlock>, String> {
    let files = tf_files(dir)?;
    if files.is_empty() {
        return Err(format!("{}: holds no `.tf` file", dir.display()));
    }
    let mut inputs = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {}", f.display(), e))?;
        inputs.push(satz_hcl::Input { path: f.display().to_string(), text });
    }
    satz_hcl::consumer_blocks(&inputs)
}

/// What one of the team's values points at in the estate.
struct Hit<'a> {
    /// how the team wrote it: `module.satz.vpc`, or the literal
    written: String,
    /// the export it reads, when it reads one
    output: Option<&'a Output>,
    /// the estate's resources it names
    resources: Vec<String>,
}

struct Check<'a> {
    interface: Option<&'a Interface>,
    manifest: &'a Manifest,
    /// module label → the interface its source is
    modules: BTreeMap<String, String>,
}

impl<'a> Check<'a> {
    fn new(blocks: &[ConsumerBlock], interface: Option<&'a Interface>, manifest: &'a Manifest) -> Self {
        let known = interface.map(|i| i.modules()).unwrap_or_default();
        let mut modules = BTreeMap::new();
        for b in blocks.iter().filter(|b| b.kind == "module") {
            let (Some(label), Some(ConsumerValue::Literal(source))) = (b.labels.first(), b.attrs.get("source")) else { continue };
            let path = source.split('?').next().unwrap_or_default().trim_end_matches('/');
            let Some((_, name)) = path.rsplit_once(&format!("{}/", interface::DIR)) else { continue };
            if known.iter().any(|m| m == name) {
                modules.insert(label.clone(), name.to_string());
            }
        }
        Check { interface, manifest, modules }
    }

    /// Where a value points in the estate; empty when it is the team's own.
    fn hits(&self, v: &ConsumerValue) -> Vec<Hit<'a>> {
        let mut out = Vec::new();
        match v {
            ConsumerValue::Reads(reads) => {
                for r in reads {
                    let parts: Vec<&str> = r.split('.').collect();
                    let ["module", m, name, ..] = parts.as_slice() else { continue };
                    let (Some(module), Some(i)) = (self.modules.get(*m), self.interface) else { continue };
                    if let Some(o) = i.module_outputs(module).into_iter().find(|o| o.name == *name) {
                        out.push(Hit { written: format!("module.{}.{}", m, name), output: Some(o), resources: o.targets.clone() });
                    }
                }
            }
            ConsumerValue::Literal(l) if !l.is_empty() => {
                if let Some(i) = self.interface {
                    for o in i.outputs.iter().filter(|o| o.static_text().as_deref() == Some(l.as_str())) {
                        out.push(Hit { written: format!("\"{}\"", l), output: Some(o), resources: o.targets.clone() });
                    }
                }
                let resources: Vec<String> = self.manifest.resources.values().filter(|r| identifies(r, l)).map(|r| r.address()).collect();
                if !resources.is_empty() {
                    out.push(Hit { written: format!("\"{}\"", l), output: None, resources });
                }
            }
            ConsumerValue::Literal(_) => {}
        }
        out
    }

    /// Whether the organisation the estate manages is what a literal names.
    fn is_org(&self, v: &ConsumerValue) -> bool {
        let ConsumerValue::Literal(l) = v else { return false };
        let named = if l.starts_with("organizations/") { l.clone() } else { format!("organizations/{}", l) };
        self.manifest.resources.values().any(|r| r.attrs.values().any(|a| *a == named))
    }

    /// The exports that allow `tf_type` on `address`.
    fn allowing(&self, tf_type: &str, address: &str) -> Vec<&'a Output> {
        self.interface
            .map(|i| i.outputs.iter().filter(|o| o.attach.iter().any(|a| a == tf_type) && o.targets.iter().any(|t| t == address)).collect())
            .unwrap_or_default()
    }
}

/// Whether an estate resource carries `value` as the identity satz writes on it.
fn identifies(r: &EmittedResource, value: &str) -> bool {
    ["project_id", "name", "account_id", "dataset_id", "email", "id"].iter().any(|k| r.attrs.get(*k).is_some_and(|a| a == value))
}

/// The team's arguments that name what a block joins or covers: its target argument, or
/// for a grant or an org policy every argument but the grant's own.
fn node_args<'b>(b: &'b ConsumerBlock, target: Option<&str>) -> Vec<(&'b str, &'b ConsumerValue)> {
    b.attrs
        .iter()
        .filter(|(k, _)| match target {
            Some(t) => k.as_str() == t,
            None => !interface::GRANT_ARGS.contains(&k.as_str()),
        })
        .map(|(k, v)| (k.as_str(), v))
        .collect()
}

/// Every finding about the team's blocks, in the order the blocks stand.
pub(crate) fn check(blocks: &[ConsumerBlock], interface: Option<&Interface>, manifest: &Manifest) -> Vec<Finding> {
    let c = Check::new(blocks, interface, manifest);
    let attach_table = interface::attach_points();
    let lookups = interface::lookups();
    let mut out = Vec::new();
    let err = |b: &ConsumerBlock, msg: String| {
        Finding::new(Severity::Error, Kind::Consumer, msg).about(b.labels.join(".")).located(b.file.clone(), b.line as u32)
    };
    for b in blocks.iter().filter(|b| b.kind == "resource") {
        let [tf_type, label] = b.labels.as_slice() else { continue };
        let address = format!("{}.{}", tf_type, label);

        // an attachment: onto the estate only at an attach point that allows its type
        if let Some((row, _)) = interface::attach_row(&attach_table, tf_type) {
            for (arg, v) in node_args(b, row.target.as_deref()) {
                for h in c.hits(v) {
                    let allowed = match h.output {
                        Some(o) => o.attach.iter().any(|a| a == tf_type),
                        None => h.resources.iter().any(|r| !c.allowing(tf_type, r).is_empty()),
                    };
                    if !allowed {
                        let offered: Vec<String> = interface
                            .map(|i| i.outputs.iter().filter(|o| o.attach.iter().any(|a| a == tf_type)).map(|o| format!("`{}`", o.name)).collect())
                            .unwrap_or_default();
                        out.push(err(
                            b,
                            format!(
                                "`{}` attaches to {} through `{}`, which is no attach point for `{}` — the estate owns it and its next apply may undo the change. {}",
                                address,
                                h.written,
                                arg,
                                tf_type,
                                if offered.is_empty() {
                                    format!("The estate allows `{}` on no export; ask for the change in the estate", tf_type)
                                } else {
                                    format!("The exports that allow it: {}", offered.join(", "))
                                }
                            ),
                        ));
                    }
                }
            }
        }

        // an authoritative grant or an org policy on a node the estate manages
        let authoritative = tf_type.ends_with("_iam_policy") || tf_type.ends_with("_iam_binding");
        let policy = ORG_POLICY.iter().find(|(t, _)| t == tf_type).map(|(_, a)| *a);
        if authoritative || policy.is_some() {
            for (arg, v) in node_args(b, policy) {
                let managed: Vec<String> = c.hits(v).into_iter().filter(|h| !h.resources.is_empty() || h.output.is_some()).map(|h| h.written).collect();
                let what = if authoritative { "sets every member of a role on it" } else { "sets an organisation policy on it" };
                if let Some(w) = managed.first() {
                    out.push(err(
                        b,
                        format!(
                            "`{}` {} through `{} = {}`, a node the estate manages — the estate's next apply and yours undo each other. {}",
                            address,
                            what,
                            arg,
                            w,
                            if authoritative { "Grant with the matching `*_iam_member` at an attach point, or ask for the grant in the estate" } else { "Ask for the policy in the estate" }
                        ),
                    ));
                } else if c.is_org(v) {
                    out.push(err(
                        b,
                        format!(
                            "`{}` {} through `{}`: the organisation the estate manages — the estate's next apply and yours undo each other. Ask for the change in the estate",
                            address, what, arg
                        ),
                    ));
                }
            }
        }

        // a resource the estate declares too, by the natural key the interface reads it by
        if let Some(row) = lookups.get(tf_type.as_str()) {
            for r in manifest.of_type(tf_type) {
                let same = row.keys.keys().all(|key| match (b.attrs.get(key), r.attrs.get(key), r.refs.get(key)) {
                    (Some(ConsumerValue::Literal(mine)), Some(theirs), _) => mine == theirs,
                    (Some(ConsumerValue::Literal(mine)), None, None) if key == "project" => manifest.project_of(r).as_deref() == Some(mine.as_str()),
                    (Some(v), _, Some(reference)) => {
                        let target = reference.split('.').take(2).collect::<Vec<_>>().join(".");
                        c.hits(v).iter().any(|h| h.resources.contains(&target))
                    }
                    (Some(v @ ConsumerValue::Reads(_)), Some(theirs), _) => {
                        c.hits(v).iter().any(|h| h.output.and_then(|o| o.static_text()).as_deref() == Some(theirs.as_str()))
                    }
                    _ => false,
                });
                if same {
                    out.push(err(
                        b,
                        format!(
                            "`{}` is the object the estate declares as `{}` — the same {} — and two states would own it. Read it from the interface instead of declaring it",
                            address,
                            r.address(),
                            row.keys.keys().map(|k| format!("`{}`", k)).collect::<Vec<_>>().join(" and ")
                        ),
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use satz_core::pipeline::{ResolvedExport, ResolvedInterface};

    const ESTATE_TF: &str = r#"
resource "google_folder" "infra" {
  display_name = "Infrastructure"
  parent       = "organizations/123456789012"
}
resource "google_project" "team" {
  project_id = "corp-team-001"
  folder_id  = google_folder.infra.name
}
"#;

    fn estate() -> (Interface, Manifest) {
        let manifest = Manifest::parse(ESTATE_TF);
        let x = |i: Option<&str>, name: &str, value: &str, attach: &[&str]| ResolvedExport {
            interface: i.map(str::to_string),
            name: name.into(),
            value: serde_yaml::Value::String(value.into()),
            description: None,
            attach: attach.iter().map(|a| a.to_string()).collect(),
            all: None,
            file: "e.satz".into(),
            line: 1,
        };
        let exports = [
            x(None, "infra_folder", "${google_folder.infra.name}", &[]),
            x(Some("team-a"), "project_id", "${google_project.team.project_id}", &["google_project_iam_member"]),
        ];
        let interfaces = [ResolvedInterface { name: "team-a".into(), uses: Vec::new(), file: "e.satz".into(), line: 1 }];
        (interface::build("e", &exports, &interfaces, &manifest, "hashicorp/google", None).unwrap(), manifest)
    }

    fn findings(tf: &str) -> Vec<String> {
        let (i, m) = estate();
        let blocks = satz_hcl::consumer_blocks(&[satz_hcl::Input { path: "team.tf".into(), text: tf.into() }]).unwrap();
        check(&blocks, Some(&i), &m).into_iter().map(|f| format!("{}:{} {}", f.file.unwrap_or_default(), f.line.unwrap_or_default(), f.message)).collect()
    }

    const MODULE: &str = "module \"satz\" {\n  source = \"git::https://example.com/estate.git//hcl/interfaces/team-a?ref=abc\"\n}\n";

    #[test]
    fn an_attachment_at_an_attach_point_passes_and_elsewhere_is_refused() {
        let ok = format!("{}resource \"google_project_iam_member\" \"a\" {{\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  member  = \"group:g@example.com\"\n}}\n", MODULE);
        assert!(findings(&ok).is_empty(), "{:?}", findings(&ok));
        // the same project written as its literal id is the same attach point
        assert!(findings(&ok.replace("module.satz.project_id", "\"corp-team-001\"")).is_empty());
        let bad = format!("{}resource \"google_folder_iam_member\" \"b\" {{\n  folder = module.satz.infra_folder\n  role   = \"roles/viewer\"\n  member = \"group:g@example.com\"\n}}\n", MODULE);
        let f = findings(&bad);
        assert_eq!(f.len(), 1, "{:?}", f);
        assert!(f[0].starts_with("team.tf:4 ") && f[0].contains("module.satz.infra_folder") && f[0].contains("no attach point for `google_folder_iam_member`"), "{}", f[0]);
        // the team's own folder is the team's business
        assert!(findings("resource \"google_folder_iam_member\" \"c\" {\n  folder = google_folder.mine.name\n  role = \"r\"\n  member = \"m\"\n}\n").is_empty());
    }

    #[test]
    fn an_authoritative_grant_or_a_policy_on_a_managed_node_is_refused() {
        let binding = format!("{}resource \"google_project_iam_binding\" \"b\" {{\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  members = []\n}}\n", MODULE);
        assert!(findings(&binding)[0].contains("sets every member of a role on it through `project = module.satz.project_id`"), "{:?}", findings(&binding));
        let policy = "resource \"google_org_policy_policy\" \"p\" {\n  name   = \"organizations/123456789012/policies/x\"\n  parent = \"organizations/123456789012\"\n}\n";
        assert!(findings(policy)[0].contains("the organisation the estate manages"), "{:?}", findings(policy));
        let own = "resource \"google_project_iam_binding\" \"b\" {\n  project = google_project.mine.project_id\n  role = \"r\"\n  members = []\n}\n";
        assert!(findings(own).is_empty());
    }

    #[test]
    fn a_resource_the_estate_declares_too_is_refused() {
        let dup = "resource \"google_project\" \"p\" {\n  project_id = \"corp-team-001\"\n  name       = \"x\"\n}\n";
        let f = findings(dup);
        assert!(f[0].contains("`google_project.team`") && f[0].contains("the same `project_id`"), "{:?}", f);
        let folder = format!("{}resource \"google_folder\" \"f\" {{\n  display_name = \"Infrastructure\"\n  parent       = \"organizations/123456789012\"\n}}\n", MODULE);
        assert!(findings(&folder)[0].contains("`google_folder.infra`"), "{:?}", findings(&folder));
        let under = format!("{}resource \"google_folder\" \"f\" {{\n  display_name = \"Team\"\n  parent       = module.satz.infra_folder\n}}\n", MODULE);
        assert!(findings(&under).is_empty(), "a new folder under the estate's is the team's: {:?}", findings(&under));
    }
}
