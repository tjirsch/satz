//! The rules a project is held to where it uses a central estate's interface (ADR 0070):
//! `satz check-consumer` on a project's HCL, and the compile of a project estate written in
//! Satz, on its own emitted resources.
//!
//! - an attachment resource whose target is the central estate's and is no attach point
//!   that allows its type;
//! - an authoritative grant (`*_iam_policy`, `*_iam_binding`) or an organisation policy
//!   on a node the central estate manages — the next apply of either side undoes the other's;
//! - a resource the central estate declares too, matched by the natural key the interface
//!   looks it up by (`presets/interface-lookups.yaml`): two states would own one object.
//!
//! Both paths judge through one `Facts`: built from the compiled central estate for
//! `check-consumer`, and read from the generated interface files for a project compile.
//! What a value points at is read two ways: a read of an export — `module.<m>.<output>` of a
//! module sourced from `<interface>/hcl`, or `interface.<export>` in a project estate — and
//! a literal equal to a value the estate publishes or an identity it writes. A value that is
//! neither is the project's own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use satz_core::satz::ManagedFact;
use satz_hcl::{ConsumerBlock, ConsumerValue};

use crate::findings::{Finding, Kind, Severity};
use crate::interface::{self, Interface};
use crate::manifest::{EmittedResource, Manifest};

/// The org-policy types and the argument that names their node.
const ORG_POLICY: &[(&str, &str)] = &[
    ("google_org_policy_policy", "parent"),
    ("google_organization_policy", "org_id"),
    ("google_folder_organization_policy", "folder"),
    ("google_project_organization_policy", "project"),
];

/// The attributes satz writes as a resource's identity: a literal a project writes that
/// equals one of them names that resource.
const IDENTITY: &[&str] = &["project_id", "name", "account_id", "dataset_id", "email", "id"];

/// The form a managed value takes in an interface file. The file travels into every
/// project's folder, and the rules need equality, not the names: a project that writes a
/// value is told it collides, and one that does not learns nothing.
const HASHED: &str = "sha256:";

/// `value` as an interface file carries it.
pub(crate) fn hashed(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{}{}", HASHED, hex::encode(Sha256::digest(value.as_bytes())))
}

/// Whether `mine`, as a project writes it, is what a fact holds — the text, when the facts
/// come from the compiled central estate, or its hash, when they come from a file.
fn same(fact: &str, mine: &str) -> bool {
    if fact.starts_with(HASHED) {
        hashed(mine) == fact
    } else {
        fact == mine
    }
}

/// One export, as far as the rules read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactOutput {
    /// the interface that declares it; `None` for a core export, and for one read from an
    /// interface file, whose names are unique
    pub interface: Option<String>,
    pub name: String,
    pub attach: Vec<String>,
    pub targets: Vec<String>,
    /// the text of a static string value
    pub static_text: Option<String>,
}

/// What a project's resources are judged against: the central estate's exports, the
/// resources it declares that a project can name, and the organisations it manages.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Facts {
    pub estate: String,
    /// `organizations/<id>`, sorted
    pub organizations: Vec<String>,
    pub outputs: Vec<FactOutput>,
    /// by address; the ids and key values as satz writes them, or hashed (`hashed`) when
    /// read from an interface file
    pub managed: Vec<ManagedFact>,
}

impl Facts {
    /// From the compiled central estate. The managed resources are those a project can
    /// name: of a type the lookup table reads back, or the target of an export — never one
    /// marked `private`, which keeps a resource out of everything handed to a project.
    pub fn of_estate(estate: &str, interface: Option<&Interface>, manifest: &Manifest) -> Facts {
        let lookups = interface::lookups();
        let outputs: Vec<FactOutput> = interface
            .map(|i| {
                i.outputs
                    .iter()
                    .map(|o| FactOutput {
                        interface: o.interface.clone(),
                        name: o.name.clone(),
                        attach: o.attach.clone(),
                        targets: o.targets.clone(),
                        static_text: o.static_text(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let targets: BTreeSet<&String> = outputs.iter().flat_map(|o| &o.targets).collect();
        let mut managed = Vec::new();
        for r in manifest.resources.values() {
            let address = r.address();
            if manifest.private.contains(&address) {
                continue;
            }
            let row = lookups.get(&r.tf_type);
            if row.is_none() && !targets.contains(&address) {
                continue;
            }
            let ids: Vec<String> = IDENTITY
                .iter()
                .filter_map(|k| r.attrs.get(*k))
                .filter(|v| !v.is_empty() && !crate::manifest::has_interpolation(v))
                .cloned()
                .fold(Vec::new(), |mut acc, v| {
                    if !acc.contains(&v) {
                        acc.push(v);
                    }
                    acc
                });
            let (mut keys, mut refs) = (BTreeMap::new(), BTreeMap::new());
            for key in row.map(|r| r.keys.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                match (r.attrs.get(&key), r.refs.get(&key)) {
                    (Some(v), _) => {
                        keys.insert(key, v.clone());
                    }
                    (None, Some(reference)) => {
                        refs.insert(key, reference.split('.').take(2).collect::<Vec<_>>().join("."));
                    }
                    (None, None) if key == "project" => {
                        if let Some(p) = manifest.project_of(r) {
                            keys.insert(key, p);
                        }
                    }
                    (None, None) => {}
                }
            }
            managed.push(ManagedFact { address, ids, keys, refs, line: 0 });
        }
        let organizations: BTreeSet<String> = manifest
            .resources
            .values()
            .flat_map(|r| r.attrs.values())
            .filter(|v| v.strip_prefix("organizations/").is_some_and(|id| !id.is_empty() && !id.contains('/') && !id.contains("${")))
            .cloned()
            .collect();
        Facts { estate: estate.to_string(), organizations: organizations.into_iter().collect(), outputs, managed }
    }

    /// From the interface files a project estate uses; the pipeline has already refused
    /// two files that disagree about one name.
    pub fn of_files(files: &[satz_core::pipeline::UsedInterfaceFile]) -> Facts {
        let mut estates: Vec<&str> = Vec::new();
        let mut organizations: BTreeSet<String> = BTreeSet::new();
        let mut outputs: Vec<FactOutput> = Vec::new();
        let mut managed: Vec<ManagedFact> = Vec::new();
        for f in files {
            let i = &f.interface;
            if !estates.contains(&i.estate.as_str()) {
                estates.push(&i.estate);
            }
            organizations.extend(i.organizations.iter().cloned());
            for o in &i.outputs {
                if outputs.iter().any(|x| x.name == o.name) {
                    continue;
                }
                outputs.push(FactOutput {
                    interface: None,
                    name: o.name.clone(),
                    attach: o.attach.clone(),
                    targets: o.targets.clone(),
                    static_text: interface::static_text(&o.value),
                });
            }
            for m in &i.managed {
                if !managed.iter().any(|x| x.address == m.address) {
                    managed.push(m.clone());
                }
            }
        }
        Facts { estate: estates.join(", "), organizations: organizations.into_iter().collect(), outputs, managed }
    }
}

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

/// The project's blocks, each located by its file as found under `dir`.
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

/// What one of the project's values points at in the central estate.
struct Hit<'a> {
    /// how the project wrote it: `module.satz.vpc`, `interface.vpc`, or the literal
    written: String,
    /// the export it reads, when it reads one
    output: Option<&'a FactOutput>,
    /// the central estate's resources it names
    resources: Vec<String>,
}

struct Check<'a> {
    facts: &'a Facts,
    /// a read's prefix (`module.satz`, `interface`) → the exports it reaches, by index
    readers: BTreeMap<String, Vec<usize>>,
}

impl<'a> Check<'a> {
    /// Where a value points in the central estate; empty when it is the project's own.
    fn hits(&self, v: &ConsumerValue) -> Vec<Hit<'a>> {
        let mut out = Vec::new();
        match v {
            ConsumerValue::Reads(reads) => {
                for r in reads {
                    for (prefix, idx) in &self.readers {
                        let Some(rest) = r.strip_prefix(prefix.as_str()).and_then(|x| x.strip_prefix('.')) else { continue };
                        let name = rest.split(['.', '[']).next().unwrap_or_default();
                        if let Some(o) = idx.iter().map(|i| &self.facts.outputs[*i]).find(|o| o.name == name) {
                            out.push(Hit { written: format!("{}.{}", prefix, name), output: Some(o), resources: o.targets.clone() });
                        }
                    }
                }
            }
            ConsumerValue::Literal(l) if !l.is_empty() => {
                for o in self.facts.outputs.iter().filter(|o| o.static_text.as_deref() == Some(l.as_str())) {
                    out.push(Hit { written: format!("\"{}\"", l), output: Some(o), resources: o.targets.clone() });
                }
                let resources: Vec<String> = self.facts.managed.iter().filter(|m| m.ids.iter().any(|i| same(i, l))).map(|m| m.address.clone()).collect();
                if !resources.is_empty() {
                    out.push(Hit { written: format!("\"{}\"", l), output: None, resources });
                }
            }
            ConsumerValue::Literal(_) => {}
        }
        out
    }

    /// Whether the organisation the central estate manages is what a literal names.
    fn is_org(&self, v: &ConsumerValue) -> bool {
        let ConsumerValue::Literal(l) = v else { return false };
        let named = if l.starts_with("organizations/") { l.clone() } else { format!("organizations/{}", l) };
        self.facts.organizations.contains(&named)
    }

    /// The exports that allow `tf_type` on `address`.
    fn allowing(&self, tf_type: &str, address: &str) -> Vec<&'a FactOutput> {
        self.facts.outputs.iter().filter(|o| o.attach.iter().any(|a| a == tf_type) && o.targets.iter().any(|t| t == address)).collect()
    }
}

/// The project's arguments that name what a block joins or covers: its target argument,
/// or for a grant or an org policy every argument but the grant's own.
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

/// `check-consumer`: a project's HCL against the compiled central estate. A module is the
/// interface whose `hcl/` its `source` ends in.
pub(crate) fn check(blocks: &[ConsumerBlock], interface: Option<&Interface>, manifest: &Manifest) -> Vec<Finding> {
    let facts = Facts::of_estate(interface.map(|i| i.estate.as_str()).unwrap_or_default(), interface, manifest);
    let mut readers = BTreeMap::new();
    if let Some(i) = interface {
        let known = i.modules();
        for b in blocks.iter().filter(|b| b.kind == "module") {
            let (Some(label), Some(ConsumerValue::Literal(source))) = (b.labels.first(), b.attrs.get("source")) else { continue };
            let path = source.split('?').next().unwrap_or_default().trim_end_matches('/');
            let Some(dir) = path.strip_suffix(&format!("/{}", interface::HCL_DIR)).or_else(|| (path == interface::HCL_DIR).then_some("")) else { continue };
            let name = dir.rsplit(['/', ':']).next().unwrap_or_default();
            if let Some(m) = known.iter().find(|m| *m == name) {
                let carried: Vec<usize> = i
                    .module_outputs(m)
                    .iter()
                    .filter_map(|o| facts.outputs.iter().position(|f| f.interface == o.interface && f.name == o.name))
                    .collect();
                readers.insert(format!("module.{}", label), carried);
            }
        }
    }
    judge(blocks, &facts, &readers, Kind::Consumer, "the estate")
}

/// A project estate's own resources against the interface files it uses: its emission
/// manifest read as blocks, and a reference its compile replaced by a lookup read as the
/// `interface.<export>` it was written as.
pub(crate) fn check_project(manifest: &Manifest, files: &[satz_core::pipeline::UsedInterfaceFile]) -> Vec<Finding> {
    let facts = Facts::of_files(files);
    // `data.<type>.<label>.<attr>` a looked-up value became → the exports that hold it
    let mut read_as: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        for o in &f.interface.outputs {
            if let serde_yaml::Value::String(s) = &o.value {
                if let Some(inner) = s.strip_prefix("${").and_then(|x| x.strip_suffix('}')).filter(|x| !x.contains("${")) {
                    let names = read_as.entry(inner.trim().to_string()).or_default();
                    if !names.contains(&o.name) {
                        names.push(o.name.clone());
                    }
                }
            }
        }
    }
    let blocks: Vec<ConsumerBlock> = manifest
        .resources
        .values()
        .map(|r: &EmittedResource| {
            let mut attrs: BTreeMap<String, ConsumerValue> = r.attrs.iter().map(|(k, v)| (k.clone(), ConsumerValue::Literal(v.clone()))).collect();
            for (k, v) in &r.refs {
                let reads = match read_as.get(v) {
                    Some(names) => names.iter().map(|n| format!("{}.{}", satz_core::pipeline::INTERFACE_ROOT, n)).collect(),
                    None => vec![v.clone()],
                };
                attrs.insert(k.clone(), ConsumerValue::Reads(reads));
            }
            let (file, line) = r.origin.clone().map(|(f, l)| (f, l as usize)).unwrap_or_default();
            ConsumerBlock { kind: "resource".to_string(), labels: vec![r.tf_type.clone(), r.label.clone()], file, line, attrs }
        })
        .collect();
    let readers = BTreeMap::from([(satz_core::pipeline::INTERFACE_ROOT.to_string(), (0..facts.outputs.len()).collect())]);
    let central = format!("the central estate `{}`", facts.estate);
    judge(&blocks, &facts, &readers, Kind::InterfaceUse, &central)
}

/// Every finding about the project's blocks, in the order the blocks stand.
fn judge(blocks: &[ConsumerBlock], facts: &Facts, readers: &BTreeMap<String, Vec<usize>>, kind: Kind, central: &str) -> Vec<Finding> {
    let c = Check { facts, readers: readers.clone() };
    let attach_table = interface::attach_points();
    let lookups = interface::lookups();
    let mut out = Vec::new();
    let err = |b: &ConsumerBlock, msg: String| {
        let f = Finding::new(Severity::Error, kind, msg).about(b.labels.join("."));
        if b.file.is_empty() {
            f
        } else {
            f.located(b.file.clone(), b.line as u32)
        }
    };
    for b in blocks.iter().filter(|b| b.kind == "resource") {
        let [tf_type, label] = b.labels.as_slice() else { continue };
        let address = format!("{}.{}", tf_type, label);

        // an attachment: onto the central estate only at an attach point that allows its type
        if let Some((row, _)) = interface::attach_row(&attach_table, tf_type) {
            for (arg, v) in node_args(b, row.target.as_deref()) {
                for h in c.hits(v) {
                    let allowed = match h.output {
                        Some(o) => o.attach.iter().any(|a| a == tf_type),
                        None => h.resources.iter().any(|r| !c.allowing(tf_type, r).is_empty()),
                    };
                    if !allowed {
                        let offered: Vec<String> = facts.outputs.iter().filter(|o| o.attach.iter().any(|a| a == tf_type)).map(|o| format!("`{}`", o.name)).collect();
                        out.push(err(
                            b,
                            format!(
                                "`{}` attaches to {} through `{}`, which is no attach point for `{}` — {} owns it and its next apply may undo the change. {}",
                                address,
                                h.written,
                                arg,
                                tf_type,
                                central,
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

        // an authoritative grant or an org policy on a node the central estate manages
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
                            "`{}` {} through `{} = {}`, a node {} manages — its next apply and yours undo each other. {}",
                            address,
                            what,
                            arg,
                            w,
                            central,
                            if authoritative { "Grant with the matching `*_iam_member` at an attach point, or ask for the grant in the estate" } else { "Ask for the policy in the estate" }
                        ),
                    ));
                } else if c.is_org(v) {
                    out.push(err(
                        b,
                        format!(
                            "`{}` {} through `{}`: the organisation {} manages — its next apply and yours undo each other. Ask for the change in the estate",
                            address, what, arg, central
                        ),
                    ));
                }
            }
        }

        // a resource the central estate declares too, by the natural key the interface reads it by
        if let Some(row) = lookups.get(tf_type.as_str()) {
            for m in facts.managed.iter().filter(|m| m.address.split_once('.').is_some_and(|(t, _)| t == tf_type)) {
                let same = row.keys.keys().all(|key| match (b.attrs.get(key), m.keys.get(key), m.refs.get(key)) {
                    (Some(ConsumerValue::Literal(mine)), Some(theirs), _) => same(theirs, mine),
                    (Some(v), None, Some(target)) => c.hits(v).iter().any(|h| h.resources.contains(target)),
                    (Some(v @ ConsumerValue::Reads(_)), Some(theirs), _) => {
                        c.hits(v).iter().any(|h| h.output.and_then(|o| o.static_text.as_deref()).is_some_and(|s| same(theirs, s)))
                    }
                    _ => false,
                });
                if same {
                    out.push(err(
                        b,
                        format!(
                            "`{}` is the object {} declares as `{}` — the same {} — and two states would own it. Read it from the interface instead of declaring it",
                            address,
                            central,
                            m.address,
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
            under: None,
            file: "e.satz".into(),
            line: 1,
        };
        let exports = [
            x(None, "infra_folder", "${google_folder.infra.name}", &[]),
            x(Some("payments"), "project_id", "${google_project.team.project_id}", &["google_project_iam_member"]),
        ];
        let interfaces = [ResolvedInterface { name: "payments".into(), common: false, uses: Vec::new(), file: "e.satz".into(), line: 1 }];
        (interface::build("e", &exports, &interfaces, &manifest, "hashicorp/google", None).unwrap(), manifest)
    }

    fn findings(tf: &str) -> Vec<String> {
        let (i, m) = estate();
        let blocks = satz_hcl::consumer_blocks(&[satz_hcl::Input { path: "project.tf".into(), text: tf.into() }]).unwrap();
        check(&blocks, Some(&i), &m).into_iter().map(|f| format!("{}:{} {}", f.file.unwrap_or_default(), f.line.unwrap_or_default(), f.message)).collect()
    }

    const MODULE: &str = "module \"satz\" {\n  source = \"git::https://example.com/estate.git//interfaces/payments/payments/hcl?ref=abc\"\n}\n";

    #[test]
    fn an_attachment_at_an_attach_point_passes_and_elsewhere_is_refused() {
        let ok = format!("{}resource \"google_project_iam_member\" \"a\" {{\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  member  = \"group:g@example.com\"\n}}\n", MODULE);
        assert!(findings(&ok).is_empty(), "{:?}", findings(&ok));
        // the same project written as its literal id is the same attach point
        assert!(findings(&ok.replace("module.satz.project_id", "\"corp-team-001\"")).is_empty());
        let bad = format!("{}resource \"google_folder_iam_member\" \"b\" {{\n  folder = module.satz.infra_folder\n  role   = \"roles/viewer\"\n  member = \"group:g@example.com\"\n}}\n", MODULE);
        let f = findings(&bad);
        assert_eq!(f.len(), 1, "{:?}", f);
        assert!(f[0].starts_with("project.tf:4 ") && f[0].contains("module.satz.infra_folder") && f[0].contains("no attach point for `google_folder_iam_member`"), "{}", f[0]);
        // the project's own folder is the project's business
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
        assert!(findings(&under).is_empty(), "a new folder under the estate's is the project's: {:?}", findings(&under));
    }

    /// The same project, written in HCL and written as a Satz estate's emission, meets the
    /// same rules with the same verdict: nothing at an attach point, and each refusal on
    /// the same resource.
    #[test]
    fn each_rule_fires_identically_on_a_project_estate_and_on_hcl() {
        let (i, m) = estate();
        let facts = Facts::of_estate("e", Some(&i), &m);
        let text = satz_core::fmt::format(&i.satz_file("payments", &facts, "0.0.0", "e.satz")).unwrap();
        let file = satz_core::satz::parse(&text).unwrap().interface_file.unwrap();
        let used = [satz_core::pipeline::UsedInterfaceFile { file: "vendor/payments/satz/interface.satz".into(), interface: file }];
        // (HCL a project writes, the resource a project estate emits for the same intent)
        let cases: [(&str, &str, bool); 5] = [
            (
                "resource \"google_project_iam_member\" \"a\" {\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  member  = \"group:g@example.com\"\n}\n",
                "resource \"google_project_iam_member\" \"a\" {\n  project = \"corp-team-001\"\n  role    = \"roles/viewer\"\n  member  = \"group:g@example.com\"\n}\n",
                false,
            ),
            (
                "resource \"google_folder_iam_member\" \"b\" {\n  folder = module.satz.infra_folder\n  role   = \"roles/viewer\"\n  member = \"group:g@example.com\"\n}\n",
                "resource \"google_folder_iam_member\" \"b\" {\n  folder = \"${data.google_active_folder.infra.name}\"\n  role   = \"roles/viewer\"\n  member = \"group:g@example.com\"\n}\n",
                true,
            ),
            (
                "resource \"google_project_iam_binding\" \"c\" {\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  members = []\n}\n",
                "resource \"google_project_iam_binding\" \"c\" {\n  project = \"corp-team-001\"\n  role    = \"roles/viewer\"\n  members = []\n}\n",
                true,
            ),
            (
                "resource \"google_org_policy_policy\" \"d\" {\n  name   = \"organizations/123456789012/policies/x\"\n  parent = \"organizations/123456789012\"\n}\n",
                "resource \"google_org_policy_policy\" \"d\" {\n  name   = \"organizations/123456789012/policies/x\"\n  parent = \"organizations/123456789012\"\n}\n",
                true,
            ),
            (
                "resource \"google_folder\" \"e\" {\n  display_name = \"Infrastructure\"\n  parent       = \"organizations/123456789012\"\n}\n",
                "resource \"google_folder\" \"e\" {\n  display_name = \"Infrastructure\"\n  parent       = \"organizations/123456789012\"\n}\n",
                true,
            ),
        ];
        for (hcl, emitted, refused) in cases {
            let blocks = satz_hcl::consumer_blocks(&[satz_hcl::Input { path: "p.tf".into(), text: format!("{}{}", MODULE, hcl) }]).unwrap();
            let from_hcl: Vec<(Option<String>, Kind)> = check(&blocks, Some(&i), &m).into_iter().map(|f| (f.subject, f.kind)).collect();
            let from_satz: Vec<(Option<String>, Kind)> = check_project(&Manifest::parse(emitted), &used).into_iter().map(|f| (f.subject, f.kind)).collect();
            assert_eq!(from_hcl.len(), usize::from(refused), "HCL: {}: {:?}", hcl, from_hcl);
            assert_eq!(from_satz.len(), from_hcl.len(), "Satz: {}: {:?}", emitted, from_satz);
            for ((a, ka), (b, kb)) in from_hcl.iter().zip(&from_satz) {
                assert_eq!(a, b, "the same resource");
                assert_eq!((*ka, *kb), (Kind::Consumer, Kind::InterfaceUse));
            }
        }
    }
}
