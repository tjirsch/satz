//! The interface an estate publishes to the projects beside it (ADR 0070). A project is an
//! estate that depends on parts of another estate's interface: its own repository or
//! folder, config, state and pipeline, written in HCL or in Satz.
//!
//! Every `export` becomes an output of the root module's `outputs.tf`, where the operator
//! reads it with `tofu output`, and a value of the generated interfaces under
//! `interfaces/`, beside `hcl/`. An export outside every `interface` block is a core
//! export, carried by every interface. Each interface has two forms side by side:
//! `<name>/hcl/`, a relocatable module (it names no file outside itself, takes no variable,
//! has no backend and reads no state), and `<name>/satz/interface.satz`, a file a project
//! estate written in Satz `use`s.
//!
//! `interfaces/common/` holds the library alone — `core`, every interface a pack declares
//! and every one marked `common`. `interfaces/<project>/` holds one project's own
//! interface and the whole library beside it, so a project takes its folder whole.
//!
//! Per `${type.label.attr}` reference, the compile decides what the value is:
//!
//! - **static** — satz writes the attribute itself (`project_id`, `display_name`), or it is
//!   derived from attributes satz writes (a service account's `email`): a literal output;
//! - **a lookup** — only the cloud knows it (a folder's `name`, a project's `number`): a
//!   `data` source, keyed by what satz writes (`presets/interface-lookups.yaml`), which the
//!   consumer's plan reads through the consumer's own provider and credentials. A key that
//!   is itself a lookup chains to that resource's data source.
//!
//! The root module needs neither: there the reference names the resource.

use crate::manifest::{EmittedResource, Manifest};
use std::collections::{BTreeMap, BTreeSet};

/// The module that carries the core exports alone.
pub(crate) const CORE: &str = satz_core::satz::CORE_INTERFACE;

/// The folder under `interfaces_dir` that holds the library alone.
pub(crate) const COMMON: &str = satz_core::satz::COMMON_LIBRARY;

/// Inside one interface's folder: the HCL module, and the directory of the Satz file.
pub(crate) const HCL_DIR: &str = "hcl";
pub(crate) const SATZ_DIR: &str = "satz";
pub(crate) const SATZ_FILE: &str = "interface.satz";

/// The local in the root `outputs.tf` that holds every exported value: the outputs read it,
/// and a pack reads it whole (`interface-notice` publishes `jsonencode` of it).
pub(crate) const LOCAL: &str = "satz_interface";

/// One row of `presets/interface-lookups.yaml`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LookupRow {
    pub data_source: String,
    pub permission: String,
    pub keys: BTreeMap<String, String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub derived: BTreeMap<String, String>,
    /// what `all <type>` gives per resource
    pub all: String,
}

/// The lookup table, compiled in: what the interface emits for a type is part of the
/// emitter, and one binary emits one interface whatever preset directory it reads.
pub(crate) fn lookups() -> BTreeMap<String, LookupRow> {
    serde_yaml::from_str(include_str!("../presets/interface-lookups.yaml")).expect("presets/interface-lookups.yaml parses")
}

/// One row of `presets/attach-points.yaml`: an attachment type an export may allow.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachRow {
    /// the attachment's argument that names the shared object; `None` for `*_iam_member`,
    /// whose node is every argument but the grant's own
    #[serde(default)]
    pub target: Option<String>,
    pub central: Vec<Central>,
}

/// What in the estate conflicts with an attachment on the exported object.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Central {
    /// `*` stands for the member type's prefix
    #[serde(rename = "type")]
    pub tf_type: String,
    #[serde(default)]
    pub sets: Option<String>,
    #[serde(default)]
    pub ignore: Option<String>,
    #[serde(default)]
    pub same_node: bool,
}

/// The attach-point table, compiled in for the reason the lookup table is.
pub(crate) fn attach_points() -> BTreeMap<String, AttachRow> {
    serde_yaml::from_str(include_str!("../presets/attach-points.yaml")).expect("presets/attach-points.yaml parses")
}

/// The member-grant row's key, and the suffix every member-grant type ends in.
const MEMBER: &str = "*_iam_member";
const MEMBER_SUFFIX: &str = "_iam_member";

/// The row an attachment type falls under, with the prefix `*` stands for (a member
/// grant's `google_project` in `google_project_iam_member`; empty for an exact row).
pub(crate) fn attach_row<'t>(table: &'t BTreeMap<String, AttachRow>, tf_type: &str) -> Option<(&'t AttachRow, String)> {
    if let Some(r) = table.get(tf_type) {
        return Some((r, String::new()));
    }
    let prefix = tf_type.strip_suffix(MEMBER_SUFFIX).filter(|p| !p.is_empty())?;
    table.get(MEMBER).map(|r| (r, prefix.to_string()))
}

/// The arguments of a grant that are the grant itself; every other one names the node.
pub(crate) const GRANT_ARGS: &[&str] = &["role", "member", "members", "condition", "policy_data", "etag", "provider", "depends_on", "lifecycle", "count", "for_each"];

/// One piece of an output value: text known now, or an HCL expression the plan evaluates.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Lit(String),
    Expr(String),
}

/// One `data` block of the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataBlock {
    pub data_source: String,
    pub label: String,
    /// the resource it reads back (`google_folder.infra`)
    pub resource: String,
    /// argument → HCL expression
    pub args: Vec<(String, String)>,
    /// argument → the same as text, `${…}` where it reads another lookup: the form a
    /// project's own emission writes
    pub texts: Vec<(String, String)>,
    pub permission: String,
    /// the other data blocks its keys read
    deps: BTreeSet<String>,
}

impl DataBlock {
    pub fn address(&self) -> String {
        format!("data.{}.{}", self.data_source, self.label)
    }
}

/// How an output obtains its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum How {
    /// known at compile time: the HCL literal
    Static(String),
    /// read by the consumer's plan: the data blocks it needs, by address
    Lookup(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Output {
    /// the interface it belongs to; `None` for a core export, which every module carries
    pub interface: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// the value in the module
    pub module_value: String,
    /// the value in the root module
    pub root_value: String,
    /// the value a Satz project reads as `${{interface.<name>}}`: a literal, or text over
    /// `${data.<type>.<label>.<attr>}` where it is looked up
    pub value: serde_yaml::Value,
    pub how: How,
    /// the attachment types a project may create against it
    pub attach: Vec<String>,
    /// the emitted resources its value reads, by address; for `all <type>`, every one
    /// the map holds
    pub targets: Vec<String>,
    /// `all <type>`: the type the map is of, keyed by the labels in `targets`
    pub all: Option<String>,
}

/// The interface of one estate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    /// the estate's name, from its header
    pub estate: String,
    pub outputs: Vec<Output>,
    /// every declared interface, by name, with the interfaces its module also carries
    /// (`use interface`, transitively)
    pub uses: BTreeMap<String, Vec<String>>,
    /// the declared interfaces in the library: marked `common`, or declared in a pack
    pub common: BTreeSet<String>,
    /// by address
    pub data: BTreeMap<String, DataBlock>,
    pub google_source: String,
    pub google_version: Option<String>,
}

/// An export the compile refuses, where it is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    /// the interface the export stands in; `None` for a core export
    pub interface: Option<String>,
    pub name: String,
    pub file: String,
    pub line: usize,
    pub msg: String,
}

/// Build the interface from the resolved exports and what the emitter emitted. Every
/// export is judged; the refusals are all returned, each at its declaration.
pub(crate) fn build(
    estate: &str,
    exports: &[satz_core::pipeline::ResolvedExport],
    interfaces: &[satz_core::pipeline::ResolvedInterface],
    manifest: &Manifest,
    google_source: &str,
    google_version: Option<&str>,
) -> Result<Interface, Vec<Refusal>> {
    let table = lookups();
    let attach_table = attach_points();
    let mut r = Resolver { manifest, table: &table, data: BTreeMap::new(), visiting: Vec::new() };
    let mut outputs = Vec::new();
    let mut refusals = Vec::new();
    for x in exports {
        let resolved = match &x.all {
            Some(t) => r.all(t),
            None => r.value(&x.value).and_then(|v| {
                // a resource marked `private` is published by no export
                match addresses_in(&x.value).into_iter().find(|a| manifest.private.contains(a)) {
                    Some(a) => Err(format!("`{}` is marked `private = true`, which keeps it out of every export — remove the mark, or export something else", a)),
                    None => Ok(v),
                }
            }),
        };
        match resolved {
            Ok((module_value, root_value, parts_all_static, used, value)) => {
                let how = if parts_all_static {
                    How::Static(module_value.clone())
                } else {
                    let mut all: BTreeSet<String> = BTreeSet::new();
                    for d in &used {
                        r.closure(d, &mut all);
                    }
                    How::Lookup(all.into_iter().collect())
                };
                let targets = match &x.all {
                    Some(t) => r.members(t).iter().map(|m| m.address()).collect(),
                    None => addresses_in(&x.value),
                };
                for msg in attach_refusals(&x.attach, &targets, manifest, &attach_table) {
                    refusals.push(Refusal { interface: x.interface.clone(), name: x.name.clone(), file: x.file.clone(), line: x.line, msg });
                }
                outputs.push(Output {
                    interface: x.interface.clone(),
                    name: x.name.clone(),
                    description: x.description.clone(),
                    module_value,
                    root_value,
                    value,
                    how,
                    attach: x.attach.clone(),
                    targets,
                    all: x.all.clone(),
                });
            }
            Err(msg) => refusals.push(Refusal { interface: x.interface.clone(), name: x.name.clone(), file: x.file.clone(), line: x.line, msg }),
        }
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    // the core outputs first, then each interface's, each in declaration order
    outputs.sort_by(|a, b| a.interface.cmp(&b.interface));
    Ok(Interface {
        estate: estate.to_string(),
        outputs,
        uses: interfaces.iter().map(|i| (i.name.clone(), i.uses.clone())).collect(),
        common: interfaces.iter().filter(|i| i.common).map(|i| i.name.clone()).collect(),
        data: r.data,
        google_source: google_source.to_string(),
        google_version: google_version.map(str::to_string),
    })
}

/// (module HCL, root HCL, whether it is known now, the data blocks it reads directly, the
/// value a Satz project reads)
type Resolved = (String, String, bool, Vec<String>, serde_yaml::Value);

struct Resolver<'a> {
    manifest: &'a Manifest,
    table: &'a BTreeMap<String, LookupRow>,
    data: BTreeMap<String, DataBlock>,
    /// the resources whose lookup is being built, for a cycle
    visiting: Vec<String>,
}

impl Resolver<'_> {
    /// Every resource of `tf_type` the estate emits and does not mark `private`, by label.
    fn members<'s>(&'s self, tf_type: &'s str) -> Vec<&'s EmittedResource> {
        let mut out: Vec<&EmittedResource> = self.manifest.of_type(tf_type).filter(|r| !self.manifest.private.contains(&r.address())).collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    /// `all <type>`: a map keyed by resource label, each value the attribute the lookup
    /// table's `all` names — static where satz knows it, a lookup where the cloud does.
    fn all(&mut self, tf_type: &str) -> Result<Resolved, String> {
        let attr = match self.table.get(tf_type) {
            Some(row) => row.all.clone(),
            None => {
                return Err(format!(
                    "`all {}`: satz has no lookup for `{}`, so it cannot say what each one is — presets/interface-lookups.yaml names the types `all` takes: {}",
                    tf_type,
                    tf_type,
                    self.table.keys().cloned().collect::<Vec<_>>().join(", ")
                ))
            }
        };
        let labels: Vec<String> = self.members(tf_type).iter().map(|r| r.label.clone()).collect();
        let (mut module, mut root, mut known, mut used) = (Vec::new(), Vec::new(), true, Vec::new());
        let mut value = serde_yaml::Mapping::new();
        for label in labels {
            let parts = self.attribute(tf_type, &label, &attr)?;
            value.insert(serde_yaml::Value::String(label.clone()), serde_yaml::Value::String(template_text(&parts)));
            for p in &parts {
                if let Part::Expr(e) = p {
                    used.extend(data_address_in(e));
                }
            }
            known &= parts.iter().all(|p| matches!(p, Part::Lit(_)));
            module.push(format!("{} = {}", hcl_string(&label), render_parts(&parts)));
            root.push(format!("{} = {}.{}.{}", hcl_string(&label), tf_type, label, attr));
        }
        let map = |entries: Vec<String>| if entries.is_empty() { "{}".to_string() } else { format!("{{ {} }}", entries.join(", ")) };
        Ok((map(module), map(root), known, used, serde_yaml::Value::Mapping(value)))
    }

    /// One export value: (module HCL, root HCL, whether it is known now, the data blocks it
    /// reads directly, the value a Satz project reads).
    fn value(&mut self, v: &serde_yaml::Value) -> Result<Resolved, String> {
        use serde_yaml::Value;
        match v {
            Value::String(s) => {
                let parts = self.text(s)?;
                let used = parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Expr(e) => Some(data_address_in(e)),
                        Part::Lit(_) => None,
                    })
                    .flatten()
                    .collect();
                let known = parts.iter().all(|p| matches!(p, Part::Lit(_)));
                Ok((render_parts(&parts), root_text(s), known, used, Value::String(template_text(&parts))))
            }
            Value::Number(n) => Ok((n.to_string(), n.to_string(), true, Vec::new(), v.clone())),
            Value::Bool(b) => Ok((b.to_string(), b.to_string(), true, Vec::new(), v.clone())),
            Value::Sequence(items) => {
                let mut module = Vec::new();
                let mut root = Vec::new();
                let mut known = true;
                let mut used = Vec::new();
                let mut value = Vec::new();
                for i in items {
                    if matches!(i, Value::Sequence(_) | Value::Mapping(_)) {
                        return Err("a list export holds strings, numbers or bools — a nested list or an object is no output value here".to_string());
                    }
                    let (m, r, k, u, pv) = self.value(i)?;
                    module.push(m);
                    root.push(r);
                    known &= k;
                    used.extend(u);
                    value.push(pv);
                }
                Ok((format!("[{}]", module.join(", ")), format!("[{}]", root.join(", ")), known, used, Value::Sequence(value)))
            }
            Value::Mapping(_) => Err("the value is an object — an export is a string, a number, a bool or a list of them".to_string()),
            Value::Null | Value::Tagged(_) => Err("the value is empty — an export publishes a value".to_string()),
        }
    }

    /// A string, cut at its `${…}` references, each resolved.
    fn text(&mut self, s: &str) -> Result<Vec<Part>, String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(i) = rest.find("${") {
            if i > 0 {
                out.push(Part::Lit(rest[..i].to_string()));
            }
            let after = &rest[i + 2..];
            let end = after.find('}').ok_or_else(|| format!("`{}` opens a `${{` that no `}}` closes", s))?;
            let inner = after[..end].trim();
            out.extend(self.reference(inner)?);
            rest = &after[end + 1..];
        }
        if !rest.is_empty() {
            out.push(Part::Lit(rest.to_string()));
        }
        Ok(merge_lits(out))
    }

    /// `type.label.attr`, the only expression an export reads.
    fn reference(&mut self, inner: &str) -> Result<Vec<Part>, String> {
        let bad = || {
            format!(
                "`${{{}}}` is no reference to what the estate emits — an export reads `${{{{type.label.attribute}}}}` of a resource this estate declares",
                inner
            )
        };
        let parts: Vec<&str> = inner.split('.').collect();
        let [tf_type, label, attr] = parts.as_slice() else { return Err(bad()) };
        let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !tf_type.starts_with("google_") || !ident(tf_type) || !ident(label) || !ident(attr) {
            return Err(bad());
        }
        self.attribute(tf_type, label, attr)
    }

    fn resource(&self, tf_type: &str, label: &str) -> Result<&EmittedResource, String> {
        let address = format!("{}.{}", tf_type, label);
        self.manifest.resources.get(&address).ok_or_else(|| {
            let mut same: Vec<&str> = self.manifest.of_type(tf_type).map(|r| r.label.as_str()).collect();
            same.sort();
            if same.is_empty() {
                format!("`{}` names nothing this estate emits — no `{}` is emitted here at all", address, tf_type)
            } else {
                format!("`{}` names nothing this estate emits — emitted `{}` labels: {}", address, tf_type, same.join(", "))
            }
        })
    }

    /// An attribute of an emitted resource: what satz writes, else what derives from it,
    /// else a lookup.
    fn attribute(&mut self, tf_type: &str, label: &str, attr: &str) -> Result<Vec<Part>, String> {
        if let Some(parts) = self.written(tf_type, label, attr)? {
            return Ok(parts);
        }
        if let Some(parts) = self.derived(tf_type, label, attr)? {
            return Ok(parts);
        }
        let row = self.table.get(tf_type).ok_or_else(|| {
            format!(
                "`{}.{}.{}` is known only once the resource exists, and satz has no lookup for `{}` — presets/interface-lookups.yaml names the types it can read back: {}",
                tf_type,
                label,
                attr,
                tf_type,
                self.table.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let Some(expr) = row.attributes.get(attr) else {
            let mut offered: Vec<&String> = row.attributes.keys().chain(row.derived.keys()).collect();
            offered.sort();
            return Err(format!(
                "`{}.{}.{}` is neither written by satz nor read back by `{}` — the attributes an export of `{}` reads: {}",
                tf_type,
                label,
                attr,
                row.data_source,
                tf_type,
                offered.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ));
        };
        let address = self.lookup(tf_type, label)?;
        Ok(vec![Part::Expr(expr.replace("{data}", &address))])
    }

    /// What satz writes on the resource, resolved: a literal, a template, or a reference.
    fn written(&mut self, tf_type: &str, label: &str, attr: &str) -> Result<Option<Vec<Part>>, String> {
        let r = self.resource(tf_type, label)?;
        if let Some(v) = r.attrs.get(attr).cloned() {
            return self.text(&v).map(Some);
        }
        if let Some(t) = r.refs.get(attr).cloned() {
            return self.reference(&t).map(Some);
        }
        Ok(None)
    }

    /// An attribute the table derives from written ones, when every one it reads is known.
    fn derived(&mut self, tf_type: &str, label: &str, attr: &str) -> Result<Option<Vec<Part>>, String> {
        let Some(template) = self.table.get(tf_type).and_then(|row| row.derived.get(attr)).cloned() else { return Ok(None) };
        let mut out = String::new();
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let close = rest[open..].find('}').map(|c| open + c).expect("a derived template closes every `{`");
            let name = &rest[open + 1..close];
            match self.known(tf_type, label, name)? {
                Some(v) => out.push_str(&v),
                None => return Ok(None),
            }
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        Ok(Some(vec![Part::Lit(out)]))
    }

    /// An attribute satz writes that is known now, or `None`. `project` falls back to the
    /// project the resource belongs to.
    fn known(&mut self, tf_type: &str, label: &str, attr: &str) -> Result<Option<String>, String> {
        let parts = match self.written(tf_type, label, attr)? {
            Some(p) => p,
            None if attr == "project" => {
                let r = self.resource(tf_type, label)?;
                match self.manifest.project_of(r) {
                    Some(p) => vec![Part::Lit(p)],
                    None => return Ok(None),
                }
            }
            None => return Ok(None),
        };
        Ok(match parts.as_slice() {
            [] => Some(String::new()),
            [Part::Lit(s)] => Some(s.clone()),
            _ => None,
        })
    }

    /// The data block that reads the resource back, built once; its address.
    fn lookup(&mut self, tf_type: &str, label: &str) -> Result<String, String> {
        let resource = format!("{}.{}", tf_type, label);
        let row = self.table.get(tf_type).cloned().expect("the caller found the row");
        let address = format!("data.{}.{}", row.data_source, label);
        if self.data.contains_key(&address) {
            return Ok(address);
        }
        if self.visiting.contains(&resource) {
            return Err(format!("`{}` is looked up by a key that needs its own lookup: {} → {}", resource, self.visiting.join(" → "), resource));
        }
        self.visiting.push(resource.clone());
        let mut args = Vec::new();
        let mut texts = Vec::new();
        let mut deps = BTreeSet::new();
        for (attr, arg) in &row.keys {
            let parts = match self.written(tf_type, label, attr)? {
                Some(p) => p,
                None => match self.known(tf_type, label, attr)? {
                    Some(v) => vec![Part::Lit(v)],
                    None => {
                        self.visiting.pop();
                        return Err(format!(
                            "`{}` is looked up by its `{}`, and satz writes none on it — write `{}` on the resource, or export an attribute satz writes",
                            resource, attr, attr
                        ));
                    }
                },
            };
            for p in &parts {
                if let Part::Expr(e) = p {
                    deps.extend(data_address_in(e));
                }
            }
            args.push((arg.clone(), render_parts(&parts)));
            texts.push((arg.clone(), template_text(&parts)));
        }
        self.visiting.pop();
        let block = DataBlock {
            data_source: row.data_source.clone(),
            label: label.to_string(),
            resource,
            args,
            texts,
            permission: row.permission.clone(),
            deps,
        };
        self.data.insert(address.clone(), block);
        Ok(address)
    }

    /// A data block and every one its keys read.
    fn closure(&self, address: &str, out: &mut BTreeSet<String>) {
        if !out.insert(address.to_string()) {
            return;
        }
        if let Some(d) = self.data.get(address) {
            for dep in &d.deps {
                self.closure(dep, out);
            }
        }
    }
}

/// The emitted resources an export's value reads: every `${type.label.…}` in it.
fn addresses_in(v: &serde_yaml::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        let mut rest = s;
        while let Some(i) = rest.find("${") {
            let after = &rest[i + 2..];
            let end = after.find('}').unwrap_or(after.len());
            let parts: Vec<&str> = after[..end].trim().split('.').collect();
            if let [t, l, ..] = parts.as_slice() {
                let a = format!("{}.{}", t, l);
                if !out.contains(&a) {
                    out.push(a);
                }
            }
            rest = &after[end..];
        }
    };
    match v {
        serde_yaml::Value::String(s) => add(s),
        serde_yaml::Value::Sequence(items) => items.iter().filter_map(|i| i.as_str()).for_each(&mut add),
        _ => {}
    }
    out
}

/// What an export's attach points refuse: an attachment type the table does not know,
/// and — per resource the export reads — a membership the estate writes itself, a
/// `lifecycle` that does not leave the attached members alone, or an authoritative
/// resource on the same node.
fn attach_refusals(attach: &[String], targets: &[String], manifest: &Manifest, table: &BTreeMap<String, AttachRow>) -> Vec<String> {
    let mut out = Vec::new();
    for t in attach {
        let Some((row, prefix)) = attach_row(table, t) else {
            out.push(format!(
                "attach \"{}\": no attachment type satz knows — presets/attach-points.yaml names {}",
                t,
                table.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
            continue;
        };
        for address in targets {
            let Some(r) = manifest.resources.get(address) else { continue };
            for c in &row.central {
                let central_type = c.tf_type.replace('*', &prefix);
                if c.same_node {
                    for other in manifest.of_type(&central_type) {
                        if on_node(other, r) {
                            out.push(format!(
                                "attach \"{}\": `{}` lets a project add its own members to `{}`, and the estate declares `{}`, which sets that node's members whole — the estate's next apply would remove every member a team adds. Grant with `{}` in the estate instead",
                                t,
                                t,
                                address,
                                other.address(),
                                t
                            ));
                        }
                    }
                    continue;
                }
                if central_type != r.tf_type {
                    continue;
                }
                if let Some(key) = &c.sets {
                    if r.set.contains_key(key) {
                        out.push(format!(
                            "attach \"{}\": projects attach to `{}` in their own state, and the estate sets its `{}` — the list the attachments add to. Remove `{}` from `{}`",
                            t, address, key, key, address
                        ));
                    }
                }
                if let Some(ignore) = &c.ignore {
                    let held = r.set.get("lifecycle.ignore_changes").is_some_and(|l| l.replace(' ', "").contains(ignore.as_str()));
                    if !held {
                        out.push(format!(
                            "attach \"{}\": projects attach to `{}` in their own state, and its `lifecycle` does not ignore `{}` — the estate's next apply would remove what they attached. Write `lifecycle {{ ignore_changes = [{}] }}` on `{}`",
                            t, address, ignore, ignore, address
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Whether a grant `g` stands on the node `r`: an argument of the grant's node reads `r`,
/// or carries a value `r` writes.
fn on_node(g: &EmittedResource, r: &EmittedResource) -> bool {
    let prefix = format!("{}.", r.address());
    let node = |k: &String| !GRANT_ARGS.contains(&k.as_str());
    g.refs.iter().filter(|(k, _)| node(k)).any(|(_, v)| v.starts_with(&prefix))
        || g.attrs.iter().filter(|(k, v)| node(k) && !v.is_empty()).any(|(_, v)| r.attrs.values().any(|w| w == v))
}

/// Adjacent literal parts as one.
fn merge_lits(parts: Vec<Part>) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    for p in parts {
        match (out.last_mut(), p) {
            (Some(Part::Lit(a)), Part::Lit(b)) => a.push_str(&b),
            (_, p) => out.push(p),
        }
    }
    out
}

/// The `data.<type>.<label>` an expression reads, if it reads one.
fn data_address_in(expr: &str) -> Option<String> {
    let at = expr.find("data.")?;
    let rest = &expr[at..];
    let mut dots = 0;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        if c == '.' {
            dots += 1;
            if dots == 3 {
                end = i;
                break;
            }
        } else if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            end = i;
            break;
        }
    }
    Some(rest[..end].to_string())
}

/// Text as an HCL string literal: quotes, backslashes and template openers escaped.
pub(crate) fn hcl_string(s: &str) -> String {
    let mut out = String::from("\"");
    out.push_str(&escape(s));
    out.push('"');
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace("${", "$${").replace("%{", "%%{")
}

/// The parts as one HCL expression: a literal, a bare expression, or a template.
fn render_parts(parts: &[Part]) -> String {
    match parts {
        [] => "\"\"".to_string(),
        [Part::Lit(s)] => hcl_string(s),
        [Part::Expr(e)] => e.clone(),
        _ => {
            let mut out = String::from("\"");
            for p in parts {
                match p {
                    Part::Lit(s) => out.push_str(&escape(s)),
                    Part::Expr(e) => {
                        out.push_str("${");
                        out.push_str(e);
                        out.push('}');
                    }
                }
            }
            out.push('"');
            out
        }
    }
}

/// The parts as the text a Satz project's string holds: a literal with its template openers
/// escaped (`$${`, `%%{`), an expression as `${…}`. The emitter writes such text as an HCL
/// template, so it reads what `render_parts` reads.
fn template_text(parts: &[Part]) -> String {
    let mut out = String::new();
    for p in parts {
        match p {
            Part::Lit(s) => out.push_str(&s.replace("${", "$${").replace("%{", "%%{")),
            Part::Expr(e) => {
                out.push_str("${");
                out.push_str(e);
                out.push('}');
            }
        }
    }
    out
}

/// The export's text in the root module, where its references name the resources: a
/// whole-value reference as the bare traversal, anything else as the template it is.
fn root_text(s: &str) -> String {
    if let Some(inner) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        if !inner.contains("${") && !inner.contains('}') {
            return inner.trim().to_string();
        }
    }
    let mut out = String::from("\"");
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&escape(&rest[..i]));
        let after = &rest[i..];
        let end = after.find('}').map(|e| e + 1).unwrap_or(after.len());
        out.push_str(&after[..end]);
        rest = &after[end..];
    }
    out.push_str(&escape(rest));
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// The stamp every generated `.tf` file opens with, as `main.tf` does.
pub(crate) fn stamp(version: &str, estate_path: &str) -> String {
    format!("# Generated by satz v{} — do not edit; re-emit from {}.\n\n", version, estate_path)
}

/// An output block's body: its description, when it has one, and its value, aligned.
fn output_body(d: &Option<String>, value: &str) -> String {
    match d {
        Some(d) => format!("  description = {}\n  value       = {}\n", hcl_string(d), value),
        None => format!("  value = {}\n", value),
    }
}

impl Output {
    /// Its name among the root module's outputs: a core export keeps its name, an
    /// interface's is `<interface>__<export>` with `-` as `_`. No export name holds `__`
    /// and no interface name holds `_`, so two never meet.
    pub fn root_name(&self) -> String {
        match &self.interface {
            Some(i) => format!("{}__{}", i.replace('-', "_"), self.name),
            None => self.name.clone(),
        }
    }

    /// The text of a static string output, as a project would write it literally.
    pub fn static_text(&self) -> Option<String> {
        static_text(&self.value)
    }

    /// Where the root module's local holds it.
    fn local_path(&self) -> String {
        match &self.interface {
            Some(i) => format!("local.{}.interfaces[{}].{}", LOCAL, hcl_string(i), self.name),
            None => format!("local.{}.core.{}", LOCAL, self.name),
        }
    }
}

impl Interface {
    /// The interfaces written under `interfaces/`: `core`, then every declared interface
    /// — one that holds only `use interface` lines included.
    pub fn modules(&self) -> Vec<String> {
        let mut named: BTreeSet<&str> = self.outputs.iter().filter_map(|o| o.interface.as_deref()).collect();
        named.extend(self.uses.keys().map(String::as_str));
        std::iter::once(CORE.to_string()).chain(named.into_iter().map(str::to_string)).collect()
    }

    /// The library: `core`, then every common interface.
    pub fn library(&self) -> Vec<String> {
        std::iter::once(CORE.to_string()).chain(self.common.iter().cloned()).collect()
    }

    /// The projects: every declared interface that is not common, each a folder of its own
    /// under `interfaces/`.
    pub fn projects(&self) -> Vec<String> {
        self.modules().into_iter().filter(|m| m != CORE && !self.common.contains(m)).collect()
    }

    /// The interfaces one module carries besides the core: its own, then each it uses.
    fn carried<'a>(&'a self, module: &'a str) -> Vec<&'a str> {
        if module == CORE {
            return Vec::new();
        }
        std::iter::once(module).chain(self.uses.get(module).into_iter().flatten().map(String::as_str)).collect()
    }

    /// The outputs of one module: the core exports, then the interface's own, then those
    /// of every interface it uses.
    pub fn module_outputs(&self, module: &str) -> Vec<&Output> {
        let mut out: Vec<&Output> = self.outputs.iter().filter(|o| o.interface.is_none()).collect();
        for i in self.carried(module) {
            out.extend(self.outputs.iter().filter(|o| o.interface.as_deref() == Some(i)));
        }
        out
    }

    /// The root module's `outputs.tf`: the values in one local — the core ones, and one map
    /// per interface — and one output per export that reads it.
    pub fn root_outputs_tf(&self) -> String {
        fn values(out: &mut String, outputs: &[&Output], pad: &str) {
            let w = outputs.iter().map(|o| o.name.len()).max().unwrap_or(0);
            for o in outputs {
                out.push_str(&format!("{}{:<w$} = {}\n", pad, o.name, o.root_value, w = w));
            }
        }
        let mut out = String::new();
        out.push_str("locals {\n");
        out.push_str(&format!("  {} = {{\n", LOCAL));
        out.push_str("    interface = 1\n");
        out.push_str(&format!("    estate    = {}\n", hcl_string(&self.estate)));
        out.push_str("    core = {\n");
        let core: Vec<&Output> = self.outputs.iter().filter(|o| o.interface.is_none()).collect();
        values(&mut out, &core, "      ");
        out.push_str("    }\n");
        out.push_str("    interfaces = {\n");
        for m in self.modules().iter().filter(|m| *m != CORE) {
            out.push_str(&format!("      {} = {{\n", hcl_string(m)));
            let own: Vec<&Output> = self.outputs.iter().filter(|o| o.interface.as_deref() == Some(m.as_str())).collect();
            values(&mut out, &own, "        ");
            out.push_str("      }\n");
        }
        out.push_str("    }\n  }\n}\n");
        for o in &self.outputs {
            out.push_str(&format!("\noutput \"{}\" {{\n", o.root_name()));
            out.push_str(&output_body(&o.description, &o.local_path()));
            out.push_str("}\n");
        }
        out
    }

    /// A module's `versions.tf`: the provider it needs, and nothing that configures it —
    /// the consumer's own provider is the one the lookups run through.
    fn module_versions_tf(&self) -> String {
        let mut out = String::from("terraform {\n  required_providers {\n    google = {\n");
        out.push_str(&format!("      source  = {}\n", hcl_string(&self.google_source)));
        if let Some(v) = &self.google_version {
            out.push_str(&format!("      version = {}\n", hcl_string(v)));
        }
        out.push_str("    }\n  }\n}\n");
        out
    }

    /// The data blocks one module's outputs read, by address.
    fn data_of(&self, module: &str) -> Vec<&DataBlock> {
        let mut used: BTreeSet<&String> = BTreeSet::new();
        for o in self.module_outputs(module) {
            if let How::Lookup(addrs) = &o.how {
                used.extend(addrs);
            }
        }
        used.into_iter().filter_map(|a| self.data.get(a)).collect()
    }

    /// A module's `main.tf`: its lookups, or nothing when every output is static.
    fn module_main_tf(&self, module: &str) -> String {
        let mut out = String::new();
        for d in self.data_of(module) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("# reads back {}; needs {}\n", d.resource, d.permission));
            out.push_str(&format!("data \"{}\" \"{}\" {{\n", d.data_source, d.label));
            let w = d.args.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
            for (k, v) in &d.args {
                out.push_str(&format!("  {:<w$} = {}\n", k, v, w = w));
            }
            out.push_str("}\n");
        }
        out
    }

    /// A module's `outputs.tf`.
    fn module_outputs_tf(&self, module: &str) -> String {
        let mut out = String::new();
        for o in self.module_outputs(module) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("output \"{}\" {{\n", o.name));
            out.push_str(&output_body(&o.description, &o.module_value));
            out.push_str("}\n");
        }
        out
    }

    /// The `.tf` files of `<module>/hcl/`, by name, without their stamps. A file
    /// with nothing in it is not written.
    pub fn module_files(&self, module: &str) -> Vec<(&'static str, String)> {
        let mut files = vec![("versions.tf", self.module_versions_tf())];
        for (name, content) in [("main.tf", self.module_main_tf(module)), ("outputs.tf", self.module_outputs_tf(module))] {
            if !content.is_empty() {
                files.push((name, content));
            }
        }
        files
    }

    /// `<module>/README.md`, beside the two forms, which travels with them.
    pub fn readme(&self, module: &str, version: &str, estate_path: &str, notice: Option<&Notice>) -> String {
        let outputs = self.module_outputs(module);
        let mut s = String::new();
        s.push_str(&format!("# The `{}` interface of estate `{}`\n\n", module, self.estate));
        s.push_str(&format!(
            "Generated by satz v{} from `{}`. Do not edit: every `satz transpile` of the estate writes `interfaces/` whole.\n\n",
            version, estate_path
        ));
        s.push_str("It publishes values the estate exports to a project: an estate with its own repository or\n");
        s.push_str("folder, config, state and pipeline, written in HCL or in Satz. It has two forms with the same\n");
        s.push_str("values: `hcl/` is a module that references no file outside itself, takes no input variable and\n");
        s.push_str("reads no state — copy it, move it, or source it by git URL — and `satz/interface.satz` is a file\n");
        s.push_str("a project estate `use`s. When the last transpile changed a value here, `CHANGES.md` beside this\n");
        s.push_str("file says what a project that reads it does about it.\n\n");
        if module == CORE {
            s.push_str("It holds the core values alone: the ones every interface of this estate carries.\n\n");
        } else {
            s.push_str(&format!(
                "It holds the values of the interface `{}` and the core values, which every interface of this\n\
                 estate carries; the table says which is which.\n\n",
                module
            ));
            let used = &self.carried(module)[1..];
            if !used.is_empty() {
                s.push_str(&format!(
                    "It also carries the values of the interfaces it uses: {}. The column `From` names the\n\
                     interface each value comes from.\n\n",
                    used.iter().map(|u| format!("`{}`", u)).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        s.push_str("## How to use it\n\n");
        s.push_str("From HCL, source the module:\n\n");
        s.push_str(&format!("```hcl\nmodule \"satz\" {{\n  source = \"<path or git URL>/{}/{}\"\n}}\n", module, HCL_DIR));
        match outputs.first() {
            Some(o) => s.push_str(&format!("\n# module.satz.{}\n```\n\n", o.name)),
            None => s.push_str("```\n\n"),
        }
        s.push_str(&format!(
            "It needs the `google` provider ({}{}) in the calling configuration. A lookup runs through that\n\
             provider, with the caller's credentials, and needs the read permission its row names.\n\n",
            self.google_source,
            self.google_version.as_deref().map(|v| format!(" {}", v)).unwrap_or_default()
        ));
        s.push_str("From a project estate written in Satz, use the file at the top level and name a value by its\n");
        s.push_str("export:\n\n");
        s.push_str(&format!("```\nuse \"<path>/{}/{}/{}\"\n", module, SATZ_DIR, SATZ_FILE));
        match outputs.first() {
            Some(o) => s.push_str(&format!("\n// \"${{{{interface.{}}}}}\"\n```\n\n", o.name)),
            None => s.push_str("```\n\n"),
        }
        s.push_str("A static value becomes its literal. A looked-up value becomes the same `data` source the module\n");
        s.push_str("reads, written once into the project's root module. The project's compile holds its own resources\n");
        s.push_str("to the capabilities below.\n\n");
        s.push_str("## Exports\n\n");
        if outputs.is_empty() {
            s.push_str("None: the estate exports no core value.\n");
        } else {
            s.push_str("| Output | From | Description | How it is obtained |\n|---|---|---|---|\n");
        }
        for o in &outputs {
            let how = match &o.how {
                How::Static(v) => format!("static: `{}`", table_cell(v)),
                How::Lookup(addrs) => {
                    let parts: Vec<String> = addrs
                        .iter()
                        .filter_map(|a| self.data.get(a))
                        .map(|d| {
                            format!(
                                "`{}` by {} (needs {})",
                                d.address(),
                                d.args.iter().map(|(k, _)| format!("`{}`", k)).collect::<Vec<_>>().join(", "),
                                d.permission
                            )
                        })
                        .collect();
                    format!("lookup: {}", parts.join("; "))
                }
            };
            let how = match &o.all {
                Some(t) => {
                    let keys: Vec<String> = o.targets.iter().filter_map(|a| a.split_once('.')).map(|(_, l)| format!("`{}`", l)).collect();
                    format!(
                        "a map of every `{}`, keyed by label — {}; {}",
                        t,
                        if keys.is_empty() { "none is emitted".to_string() } else { format!("keys {}", keys.join(", ")) },
                        how
                    )
                }
                None => how,
            };
            s.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                o.name,
                o.interface.as_deref().unwrap_or(CORE),
                o.description.as_deref().map(table_cell).unwrap_or_default(),
                how
            ));
        }
        s.push_str("\n## Capabilities\n\n");
        if outputs.iter().all(|o| o.attach.is_empty()) {
            s.push_str("Every value here is read. None is an attach point: a write to shared infrastructure goes into\n");
            s.push_str("this estate as a contribution, through the estate's own change management.\n\n");
        } else {
            s.push_str("Every value is read. An attach point also takes the attachment resources named beside it, in your\n");
            s.push_str("own state: they add your object to the shared one, and the estate leaves what you add alone.\n");
            s.push_str("Any other write to shared infrastructure goes into this estate as a contribution. `satz\n");
            s.push_str("check-consumer <your directory>` checks your HCL against this table; a project estate's own\n");
            s.push_str("compile checks its resources against it.\n\n");
            s.push_str("| Output | Read | Attach |\n|---|---|---|\n");
            for o in &outputs {
                let attach = if o.attach.is_empty() {
                    "—".to_string()
                } else {
                    o.attach.iter().map(|t| format!("`{}`", t)).collect::<Vec<_>>().join(", ")
                };
                s.push_str(&format!("| `{}` | yes | {} |\n", o.name, attach));
            }
            s.push('\n');
        }
        s.push_str("## Change notice\n\n");
        match notice {
            Some(n) => {
                s.push_str(&format!(
                    "Every apply that changes an exported value rewrites `{}`, and Cloud Storage publishes one\n\
                     message to the topic `{}`. An apply that changes nothing publishes nothing. The object holds\n\
                     the values as JSON (`interface`, `estate`, `core`, and `interfaces` with one map per\n\
                     interface); the message names the object.\n\n\
                     Subscribe in your own state:\n\n\
                     ```hcl\n\
                     resource \"google_pubsub_subscription\" \"satz_interface\" {{\n  \
                       name    = \"<project>-satz-interface\"\n  \
                       project = \"<your project>\"\n  \
                       topic   = module.satz.{}\n\
                     }}\n\
                     ```\n",
                    n.object, n.topic, n.topic_output
                ));
            }
            None => s.push_str("No change notice is set up for this estate.\n"),
        }
        s
    }
}

impl Interface {
    /// `<module>/satz/interface.satz`: the same values as the module, as data a project
    /// estate `use`s — each value in the form `${{interface.<name>}}` stands for, the
    /// lookups those read, and the facts its compile holds the project's resources to.
    pub fn satz_file(&self, module: &str, facts: &crate::consumer::Facts, version: &str, estate_path: &str) -> String {
        use satz_core::satz::quote;
        let list = |items: &[String]| format!("[{}]", items.iter().map(|i| quote(i)).collect::<Vec<_>>().join(", "));
        let mut s = String::new();
        s.push_str(&format!(
            "// Generated by satz v{} from `{}` (estate `{}`) — do not edit: every\n\
             // `satz transpile` of that estate writes it whole. A project estate reads it with\n\
             // `use \"<path>/{}/{}/{}\"` and names a value `${{{{interface.<export>}}}}`.\n\
             interface {}\n\n",
            version,
            estate_path,
            self.estate,
            module,
            SATZ_DIR,
            SATZ_FILE,
            quote(module)
        ));
        s.push_str(&format!("central {{\n  estate = {}\n  organizations = {}\n}}\n", quote(&self.estate), list(&facts.organizations)));
        for o in self.module_outputs(module) {
            s.push_str(&format!("\noutput {} {{\n  value = {}\n", quote(&o.name), satz_value(&o.value, "  ")));
            if !o.attach.is_empty() {
                s.push_str(&format!("  attach = {}\n", list(&o.attach)));
            }
            if !o.targets.is_empty() {
                s.push_str(&format!("  targets = {}\n", list(&o.targets)));
            }
            if let Some(d) = &o.description {
                s.push_str(&format!("  description = {}\n", quote(d)));
            }
            s.push_str("}\n");
        }
        for d in self.data_of(module) {
            s.push_str(&format!(
                "\nlookup {} {{\n  reads = {}\n  permission = {}\n  arguments {{\n",
                quote(&d.address()),
                quote(&d.resource),
                quote(&d.permission)
            ));
            for (k, v) in &d.texts {
                s.push_str(&format!("    {} = {}\n", k, quote(v)));
            }
            s.push_str("  }\n}\n");
        }
        for m in &facts.managed {
            s.push_str(&format!("\nmanaged {} {{\n", quote(&m.address)));
            // the ids and key values hashed: the file travels into every project's folder,
            // and the duplicate rule needs equality, not the central estate's inventory
            if !m.ids.is_empty() {
                s.push_str(&format!("  ids = {}\n", list(&m.ids.iter().map(|i| crate::consumer::hashed(i)).collect::<Vec<_>>())));
            }
            for (block, entries) in [("keys", &m.keys), ("refs", &m.refs)] {
                if entries.is_empty() {
                    continue;
                }
                s.push_str(&format!("  {} {{\n", block));
                for (k, v) in entries {
                    let v = if block == "keys" { crate::consumer::hashed(v) } else { v.clone() };
                    s.push_str(&format!("    {} = {}\n", k, quote(&v)));
                }
                s.push_str("  }\n");
            }
            s.push_str("}\n");
        }
        s
    }

    /// `interfaces/<folder>/README.md`: what the folder holds, where it came from, and the
    /// content hash of every other file in it. `folder` is `common` or a project.
    pub fn index_readme(&self, folder: &str, version: &str, estate_path: &str, hash: &str) -> String {
        let mut s = String::new();
        if folder == COMMON {
            s.push_str(&format!("# The common interfaces of estate `{}`\n\n", self.estate));
        } else {
            s.push_str(&format!("# The interfaces of estate `{}` for the project `{}`\n\n", self.estate, folder));
        }
        s.push_str(&format!(
            "Generated by satz v{} from `{}`. Do not edit: every `satz transpile` of the estate writes this\n\
             folder whole.\n\n\
             Content hash: `sha256:{}`\n\n\
             The hash is SHA-256 over every other file of this folder: per file in path order, its path\n\
             relative to the folder, a NUL byte, its length as 8 little-endian bytes, and its bytes. Two\n\
             copies of the folder are the same when their hashes are.\n\n",
            version, estate_path, hash
        ));
        if folder == COMMON {
            s.push_str("It holds the library alone — the interfaces every project's folder carries — for a project that\n");
            s.push_str("reads no interface of its own. Take the folder whole.\n\n");
        } else {
            s.push_str("A project is an estate that depends on parts of this estate's interface, with its own repository or\n");
            s.push_str("folder, config, state and pipeline. Take the folder whole: it holds the project's own interface and\n");
            s.push_str("the library every project's folder carries.\n\n");
        }
        s.push_str("Every interface has the same two forms: `<name>/hcl/` is a module a configuration written in HCL\n");
        s.push_str("sources, and `<name>/satz/interface.satz` is a file a project estate written in Satz `use`s, reading a\n");
        s.push_str("value as `\"${{interface.<export>}}\"`. `<name>/README.md` lists the values and what may be done with them,\n");
        s.push_str("and `<name>/CHANGES.md` is there when the last transpile changed a value: the todo for a project that reads it.\n\n");
        s.push_str("| Interface | What it holds |\n|---|---|\n");
        if folder != COMMON {
            let used = &self.carried(folder)[1..];
            s.push_str(&format!(
                "| `{}` | the project's own: its exports and the core ones{} |\n",
                folder,
                if used.is_empty() { String::new() } else { format!(", and those of {}", used.iter().map(|u| format!("`{}`", u)).collect::<Vec<_>>().join(", ")) }
            ));
        }
        for m in self.library() {
            let what = if m == CORE { "the core exports alone" } else { "a common interface, and the core exports" };
            s.push_str(&format!("| `{}` | {} |\n", m, what));
        }
        s
    }
}

/// A value as Satz text: a string quoted, a list inline, a map one entry per line.
fn satz_value(v: &serde_yaml::Value, pad: &str) -> String {
    use satz_core::satz::quote;
    match v {
        serde_yaml::Value::String(s) => quote(s),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Sequence(items) => format!("[{}]", items.iter().map(|i| satz_value(i, pad)).collect::<Vec<_>>().join(", ")),
        serde_yaml::Value::Mapping(m) if m.is_empty() => "{}".to_string(),
        serde_yaml::Value::Mapping(m) => {
            let mut out = String::from("{\n");
            for (k, val) in m {
                let key = quote(k.as_str().expect("a map export is keyed by resource label"));
                out.push_str(&format!("{}  {} = {}\n", pad, key, satz_value(val, &format!("{}  ", pad))));
            }
            out.push_str(pad);
            out.push('}');
            out
        }
        serde_yaml::Value::Null | serde_yaml::Value::Tagged(_) => unreachable!("`build` refuses an empty export"),
    }
}

/// The content hash of one folder: SHA-256 over each file in path order — its path
/// relative to the folder, a NUL byte, its length as 8 little-endian bytes, its bytes.
pub(crate) fn content_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (path, bytes) in files {
        h.update(path.as_bytes());
        h.update([0u8]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    hex::encode(h.finalize())
}

/// What the change notice publishes, when the estate uses `interface-notice`.
pub(crate) struct Notice {
    /// the object URL (`gs://…/interface.json`)
    pub object: String,
    /// the topic id
    pub topic: String,
    /// the output a consumer reads the topic from
    pub topic_output: String,
}

/// The text of a string value that holds no template — a value a project could write as a
/// literal. One definition for both forms of the interface, so a literal matches the same
/// export whichever form the project reads.
pub(crate) fn static_text(v: &serde_yaml::Value) -> Option<String> {
    let serde_yaml::Value::String(s) = v else { return None };
    (!s.contains("${") && !s.contains("%{")).then(|| s.clone())
}

/// The change notice, read off the core outputs: `interface_topic` and `interface_object`
/// are what the pack exports, and every module carries them.
pub(crate) fn notice(interface: &Interface) -> Option<Notice> {
    let value = |name: &str| {
        interface.outputs.iter().find(|o| o.interface.is_none() && o.name == name).and_then(|o| match &o.how {
            How::Static(v) => Some(v.trim_matches('"').to_string()),
            How::Lookup(_) => None,
        })
    };
    Some(Notice { object: value("interface_object")?, topic: value("interface_topic")?, topic_output: "interface_topic".to_string() })
}

/// Text as one Markdown table cell: no pipe ends it, no angle bracket opens a tag.
fn table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('<', "&lt;").replace('>', "&gt;").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use satz_core::pipeline::ResolvedExport;

    const MAIN_TF: &str = r#"
resource "google_folder" "infra" {
  display_name = "Infrastructure"
  parent       = "organizations/123456789012"
}
resource "google_folder" "team" {
  display_name = "Team"
  parent       = google_folder.infra.name
}
resource "google_project" "infra" {
  project_id = "corp-infra-001"
  folder_id  = google_folder.infra.name
}
resource "google_service_account" "provisioner" {
  project    = google_project.infra.project_id
  account_id = "svc-iac-001"
}
resource "google_cloud_identity_group" "g" {
  display_name = "G"
}
"#;

    fn export(name: &str, value: &str) -> ResolvedExport {
        ResolvedExport { interface: None, name: name.into(), value: serde_yaml::Value::String(value.into()), description: None, attach: Vec::new(), all: None, file: "e.satz".into(), line: 3 }
    }

    fn build_one(value: &str) -> Result<Interface, Vec<Refusal>> {
        build("e", &[export("x", value)], &[], &Manifest::parse(MAIN_TF), "hashicorp/google", Some("7.14.1"))
    }

    #[test]
    fn a_param_or_a_written_attribute_is_a_literal_output() {
        let i = build_one("corp").unwrap();
        assert_eq!(i.outputs[0].how, How::Static("\"corp\"".into()));
        let i = build_one("${google_project.infra.project_id}").unwrap();
        assert_eq!(i.outputs[0].module_value, "\"corp-infra-001\"");
        assert_eq!(i.outputs[0].root_value, "google_project.infra.project_id");
        assert!(i.data.is_empty());
    }

    #[test]
    fn a_derived_attribute_is_known_from_what_satz_writes() {
        let i = build_one("${google_service_account.provisioner.email}").unwrap();
        assert_eq!(i.outputs[0].module_value, "\"svc-iac-001@corp-infra-001.iam.gserviceaccount.com\"");
    }

    #[test]
    fn a_computed_attribute_is_a_data_source_and_the_lookups_chain() {
        let i = build_one("${google_folder.team.name}").unwrap();
        assert_eq!(i.outputs[0].module_value, "data.google_active_folder.team.name");
        let team = &i.data["data.google_active_folder.team"];
        assert_eq!(team.args, vec![("display_name".into(), "\"Team\"".into()), ("parent".into(), "data.google_active_folder.infra.name".into())]);
        let infra = &i.data["data.google_active_folder.infra"];
        assert_eq!(infra.args[1], ("parent".into(), "\"organizations/123456789012\"".into()));
        assert_eq!(i.outputs[0].how, How::Lookup(vec!["data.google_active_folder.infra".into(), "data.google_active_folder.team".into()]));
        let main = i.module_main_tf(CORE);
        assert!(main.contains("data \"google_active_folder\" \"team\" {"), "{}", main);
    }

    #[test]
    fn an_embedded_reference_is_a_template() {
        let i = build_one("projects/${google_project.infra.number}/x").unwrap();
        assert_eq!(i.outputs[0].module_value, "\"projects/${data.google_project.infra.number}/x\"");
        assert_eq!(i.outputs[0].root_value, "\"projects/${google_project.infra.number}/x\"");
    }

    #[test]
    fn an_absent_address_an_unreadable_type_and_an_unread_attribute_are_refused() {
        let e = build_one("${google_folder.nope.name}").unwrap_err();
        assert!(e[0].msg.contains("names nothing this estate emits") && e[0].msg.contains("infra, team"), "{}", e[0].msg);
        let e = build_one("${google_cloud_identity_group.g.name}").unwrap_err();
        assert!(e[0].msg.contains("no lookup for `google_cloud_identity_group`"), "{}", e[0].msg);
        let e = build_one("${google_folder.infra.create_time}").unwrap_err();
        assert!(e[0].msg.contains("the attributes an export of `google_folder` reads: folder_id, id, name"), "{}", e[0].msg);
        let e = build_one("${var.x}").unwrap_err();
        assert!(e[0].msg.contains("is no reference to what the estate emits"), "{}", e[0].msg);
        assert_eq!((e[0].file.as_str(), e[0].line), ("e.satz", 3));
    }

    fn in_interface(i: &str, name: &str, value: &str) -> ResolvedExport {
        ResolvedExport { interface: Some(i.into()), ..export(name, value) }
    }

    /// Per module: the README has one row per output and `outputs.tf` one output per export
    /// — the core ones in every module, an interface's own in its module alone.
    #[test]
    fn every_module_carries_the_core_exports_and_its_own_and_the_readme_lists_them() {
        let exports = [
            export("org", "123"),
            in_interface("team-a", "folder", "${google_folder.team.name}"),
            in_interface("team-b", "project", "${google_project.infra.number}"),
        ];
        let i = build("e", &exports, &[], &Manifest::parse(MAIN_TF), "hashicorp/google", Some("7.14.1")).unwrap();
        assert_eq!(i.modules(), ["core", "team-a", "team-b"]);
        for (module, want) in [("core", vec!["org"]), ("team-a", vec!["org", "folder"]), ("team-b", vec!["org", "project"])] {
            let readme = i.readme(module, "0.0.0", "satz/e.satz", None);
            let rows: Vec<&str> = readme.lines().filter(|l| l.starts_with("| `")).collect();
            let files = i.module_files(module);
            let outputs_tf = &files.iter().find(|(n, _)| *n == "outputs.tf").unwrap().1;
            let outputs: Vec<&str> = outputs_tf.lines().filter(|l| l.starts_with("output ")).collect();
            assert_eq!(rows.len(), want.len(), "{}: {}", module, readme);
            assert_eq!(outputs.len(), want.len(), "{}: {}", module, outputs_tf);
            for name in &want {
                assert!(rows.iter().any(|r| r.starts_with(&format!("| `{}` |", name))), "{}: {}", module, readme);
                assert!(outputs.contains(&format!("output \"{}\" {{", name).as_str()));
            }
            assert!(readme.contains(&format!("# The `{}` interface of estate `e`", module)));
            assert!(readme.contains("| `org` | core |"), "{}", readme);
            assert!(readme.contains("No change notice is set up"));
        }
        // a module reads only the lookups its own outputs need
        let main_a = &i.module_files("team-a").iter().find(|(n, _)| *n == "main.tf").unwrap().1.clone();
        assert!(main_a.contains("google_active_folder") && !main_a.contains("data \"google_project\""), "{}", main_a);
        assert!(!i.module_files("core").iter().any(|(n, _)| *n == "main.tf"), "core reads nothing");

        let root = i.root_outputs_tf();
        assert!(root.contains("output \"org\""), "{}", root);
        assert!(root.contains("output \"team_a__folder\"") && root.contains("local.satz_interface.interfaces[\"team-a\"].folder"), "{}", root);
        assert!(root.contains("\"team-b\" = {"), "{}", root);
    }

    /// A project's interface carries the exports of every interface it uses, the README names
    /// the interface each value comes from, and the root module still has each export once.
    #[test]
    fn a_module_carries_the_interfaces_it_uses_and_says_where_each_value_is_from() {
        use satz_core::pipeline::ResolvedInterface;
        let exports = [export("org", "123"), in_interface("network", "vpc", "v"), in_interface("team-a", "own", "1")];
        let ri = |name: &str, uses: &[&str]| ResolvedInterface { name: name.into(), common: false, uses: uses.iter().map(|u| u.to_string()).collect(), file: "e.satz".into(), line: 1 };
        let interfaces = [ri("network", &[]), ri("team-a", &["network"]), ri("team-b", &["network"])];
        let i = build("e", &exports, &interfaces, &Manifest::parse(MAIN_TF), "hashicorp/google", None).unwrap();
        assert_eq!(i.modules(), ["core", "network", "team-a", "team-b"], "an interface of uses alone is a module");
        for (module, want) in [("team-a", vec!["org", "own", "vpc"]), ("team-b", vec!["org", "vpc"]), ("network", vec!["org", "vpc"])] {
            let files = i.module_files(module);
            let outputs_tf = &files.iter().find(|(n, _)| *n == "outputs.tf").unwrap().1;
            let got: Vec<&str> = outputs_tf.lines().filter_map(|l| l.strip_prefix("output \"")).map(|l| l.trim_end_matches("\" {")).collect();
            assert_eq!(got, want, "{}", module);
        }
        let readme = i.readme("team-a", "0.0.0", "satz/e.satz", None);
        assert!(readme.contains("| `vpc` | network |") && readme.contains("| `own` | team-a |"), "{}", readme);
        assert!(readme.contains("the interfaces it uses: `network`"), "{}", readme);
        let root = i.root_outputs_tf();
        assert_eq!(root.matches("output \"network__vpc\"").count(), 1, "{}", root);
        assert!(!root.contains("team_a__vpc"), "{}", root);
    }

    /// An attach point names a type the table knows, and the estate may not write the
    /// membership it opens: a perimeter's `status.resources`, a perimeter whose lifecycle
    /// does not ignore them, an authoritative grant on the node a member grant attaches to.
    #[test]
    fn an_attach_point_refuses_the_estate_s_own_authoritative_membership() {
        const TF: &str = r#"
resource "google_project" "team" {
  project_id = "corp-team-001"
}
resource "google_access_context_manager_service_perimeter" "open" {
  name   = "accessPolicies/1/servicePerimeters/open"
  title  = "open"
  parent = "accessPolicies/1"
  lifecycle {
    ignore_changes = [status[0].resources]
  }
}
resource "google_access_context_manager_service_perimeter" "closed" {
  name   = "accessPolicies/1/servicePerimeters/closed"
  title  = "closed"
  parent = "accessPolicies/1"
  status {
    resources = ["projects/1"]
  }
}
resource "google_project_iam_binding" "team_viewers" {
  project = google_project.team.project_id
  role    = "roles/viewer"
  members = []
}
"#;
        let with = |value: &str, attach: &[&str]| ResolvedExport { attach: attach.iter().map(|a| a.to_string()).collect(), ..export("x", value) };
        let run = |x: ResolvedExport| build("e", &[x], &[], &Manifest::parse(TF), "hashicorp/google", None);
        let perimeter = "google_access_context_manager_service_perimeter_resource";
        let i = run(with("${google_access_context_manager_service_perimeter.open.name}", &[perimeter])).unwrap();
        assert_eq!(i.outputs[0].attach, [perimeter]);
        assert_eq!(i.outputs[0].targets, ["google_access_context_manager_service_perimeter.open"]);
        let readme = i.readme(CORE, "0.0.0", "satz/e.satz", None);
        assert!(readme.contains(&format!("| `x` | yes | `{}` |", perimeter)), "{}", readme);

        let e = run(with("${google_access_context_manager_service_perimeter.closed.name}", &[perimeter])).unwrap_err();
        let msgs: Vec<&str> = e.iter().map(|r| r.msg.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("the estate sets its `status.resources`")), "{:?}", msgs);
        assert!(msgs.iter().any(|m| m.contains("ignore_changes = [status[0].resources]")), "{:?}", msgs);

        let e = run(with("${google_project.team.project_id}", &["google_project_iam_member"])).unwrap_err();
        assert!(e[0].msg.contains("`google_project_iam_binding.team_viewers`"), "{}", e[0].msg);
        assert!(run(with("${google_project.team.project_id}", &["google_folder_iam_member"])).is_ok(), "another node's grant type is no conflict");

        let e = run(with("${google_project.team.project_id}", &["google_compute_instance"])).unwrap_err();
        assert!(e[0].msg.contains("no attachment type satz knows") && e[0].msg.contains("*_iam_member"), "{}", e[0].msg);
        assert!(run(with("${google_project.team.project_id}", &["google_compute_shared_vpc_service_project"])).is_ok());
    }

    /// `all <type>` is a map keyed by label, each value what the row's `all` names —
    /// looked up where the cloud knows it — without the resources marked private; an
    /// export naming a private resource and a type with no row are refused.
    #[test]
    fn an_all_export_maps_every_resource_of_a_type_but_the_private_ones() {
        let mut m = Manifest::parse(MAIN_TF);
        let all = |t: &str| ResolvedExport { all: Some(t.into()), value: serde_yaml::Value::Null, ..export("x", "") };
        let i = build("e", &[all("google_folder"), all("google_project")], &[], &m, "hashicorp/google", None).unwrap();
        let folders = &i.outputs[0];
        assert_eq!(folders.module_value, "{ \"infra\" = data.google_active_folder.infra.name, \"team\" = data.google_active_folder.team.name }");
        assert_eq!(folders.root_value, "{ \"infra\" = google_folder.infra.name, \"team\" = google_folder.team.name }");
        assert!(matches!(folders.how, How::Lookup(_)));
        assert_eq!(folders.targets, ["google_folder.infra", "google_folder.team"]);
        assert_eq!(i.outputs[1].how, How::Static("{ \"infra\" = \"corp-infra-001\" }".into()));
        let readme = i.readme(CORE, "0.0.0", "satz/e.satz", None);
        assert!(readme.contains("a map of every `google_folder`, keyed by label — keys `infra`, `team`; lookup:"), "{}", readme);

        m.private.insert("google_folder.team".into());
        let i = build("e", &[all("google_folder")], &[], &m, "hashicorp/google", None).unwrap();
        assert_eq!(i.outputs[0].targets, ["google_folder.infra"], "a private folder is left out");
        let e = build("e", &[export("x", "${google_folder.team.name}")], &[], &m, "hashicorp/google", None).unwrap_err();
        assert!(e[0].msg.contains("`google_folder.team` is marked `private = true`"), "{}", e[0].msg);
        let e = build("e", &[all("google_cloud_identity_group")], &[], &m, "hashicorp/google", None).unwrap_err();
        assert!(e[0].msg.contains("`all google_cloud_identity_group`: satz has no lookup"), "{}", e[0].msg);
        let none = build("e", &[all("google_pubsub_topic")], &[], &m, "hashicorp/google", None).unwrap();
        assert_eq!(none.outputs[0].how, How::Static("{}".into()), "no resource of the type is an empty map");
    }

    #[test]
    fn every_attach_row_is_an_attachment_with_a_target() {
        for (t, row) in attach_points() {
            assert!(t == MEMBER || row.target.is_some(), "{}: an attachment names what it joins", t);
            for c in &row.central {
                assert!(c.same_node || c.sets.is_some() || c.ignore.is_some(), "{}: `{}` says nothing", t, c.tf_type);
            }
        }
    }

    fn ri(name: &str, common: bool, uses: &[&str]) -> satz_core::pipeline::ResolvedInterface {
        satz_core::pipeline::ResolvedInterface { name: name.into(), common, uses: uses.iter().map(|u| u.to_string()).collect(), file: "e.satz".into(), line: 1 }
    }

    /// `interfaces/` is satz's: `common/` holds the library, each project's folder its own
    /// interface and the whole library — never another project's — each interface in both
    /// forms; a transpile removes a project the estate no longer declares, `hcl/interfaces/`,
    /// and the directory and `outputs.tf` when the estate exports nothing.
    #[test]
    fn the_interfaces_directory_holds_the_library_and_one_folder_per_project() {
        let dir = std::env::temp_dir().join(format!("satz-interfaces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (hcl, out) = (dir.join("hcl"), dir.join("interfaces"));
        std::fs::create_dir_all(hcl.join("interfaces/old")).unwrap();
        let manifest = Manifest::parse(MAIN_TF);
        let two = [export("org", "123"), in_interface("net", "vpc", "v"), in_interface("pay", "a", "1"), in_interface("ship", "b", "2")];
        let interfaces = [ri("net", true, &[]), ri("pay", false, &["net"]), ri("ship", false, &[])];
        let i = build("e", &two, &interfaces, &manifest, "hashicorp/google", None).unwrap();
        assert_eq!(i.library(), ["core", "net"]);
        assert_eq!(i.projects(), ["pay", "ship"]);
        crate::write_interface(Some(&i), &manifest, &hcl, &out, "e.satz").unwrap();
        let listing = |d: &std::path::Path| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
            v.sort();
            v
        };
        assert_eq!(listing(&out), ["common", "pay", "ship"]);
        assert_eq!(listing(&out.join("common")), ["README.md", "core", "net"]);
        assert_eq!(listing(&out.join("pay")), ["README.md", "core", "net", "pay"], "a project carries its own and the library, never another project's");
        assert_eq!(listing(&out.join("ship")), ["README.md", "core", "net", "ship"]);
        for m in ["core", "net", "pay"] {
            for f in ["README.md", "hcl/versions.tf", "hcl/outputs.tf", "satz/interface.satz"] {
                assert!(out.join("pay").join(m).join(f).exists(), "pay/{}/{} missing", m, f);
            }
        }
        let index = std::fs::read_to_string(out.join("pay/README.md")).unwrap();
        assert!(index.contains("Content hash: `sha256:") && index.contains("| `pay` |") && index.contains("| `net` |"), "{}", index);
        assert!(hcl.join("outputs.tf").exists());
        assert!(!hcl.join("interfaces").exists(), "hcl/interfaces/ survived");

        // the first write has nothing to diff against
        assert!(!out.join("pay/pay/CHANGES.md").exists() && !out.join("common/core/CHANGES.md").exists());
        let hash_of = |folder: &str| std::fs::read_to_string(out.join(folder).join("README.md")).unwrap().lines().find(|l| l.starts_with("Content hash:")).unwrap().to_string();

        let one = [export("org", "123"), in_interface("pay", "a", "1")];
        let i = build("e", &one, &[ri("pay", false, &[])], &manifest, "hashicorp/google", None).unwrap();
        crate::write_interface(Some(&i), &manifest, &hcl, &out, "e.satz").unwrap();
        assert_eq!(listing(&out), ["common", "pay"], "the folder of a removed project survived");
        // `pay` no longer uses `net`, so its module lost `vpc`: a CHANGES.md beside its README,
        // and none for `core`, which did not change
        let changes = std::fs::read_to_string(out.join("pay/pay/CHANGES.md")).expect("pay changed");
        assert!(changes.contains("# What changed in `pay`") && changes.contains("- [ ] `vpc` is gone"), "{}", changes);
        assert!(!out.join("pay/core/CHANGES.md").exists() && !out.join("common/core/CHANGES.md").exists());
        let hashed = hash_of("pay");
        // the same estate again: nothing changed, no file, and the hash never saw the file
        crate::write_interface(Some(&i), &manifest, &hcl, &out, "e.satz").unwrap();
        assert!(!out.join("pay/pay/CHANGES.md").exists(), "a transpile that changed nothing wrote a CHANGES.md");
        assert_eq!(hash_of("pay"), hashed, "the content hash depends on the previous state");

        crate::write_interface(None, &manifest, &hcl, &out, "e.satz").unwrap();
        assert!(!out.exists() && !hcl.join("outputs.tf").exists(), "an estate that exports nothing keeps no interface");
        let refused = crate::write_interface(Some(&i), &manifest, &hcl, &dir, "e.satz").unwrap_err();
        assert!(refused.to_string().contains("holds hcl_dir"), "{}", refused);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Satz form carries what the HCL form does, as data: generated, parsed back, every
    /// value, lookup, attach point and managed fact is there.
    #[test]
    fn the_interface_file_round_trips_every_value_lookup_attach_point_and_managed_fact() {
        let manifest = Manifest::parse(MAIN_TF);
        let with_attach = ResolvedExport { attach: vec!["google_folder_iam_member".into()], ..in_interface("pay", "team_folder", "${google_folder.team.name}") };
        let exports = [
            export("org", "123"),
            export("regions", ""),
            with_attach,
            in_interface("pay", "infra_project", "${google_project.infra.project_id}"),
            ResolvedExport { all: Some("google_folder".into()), value: serde_yaml::Value::Null, ..in_interface("pay", "folders", "") },
        ];
        let mut exports = exports.to_vec();
        exports[1].value = serde_yaml::Value::Sequence(vec!["europe-west3".into(), serde_yaml::Value::Number(2.into())]);
        let i = build("e", &exports, &[ri("pay", false, &[])], &manifest, "hashicorp/google", Some("7.14.1")).unwrap();
        let facts = crate::consumer::Facts::of_estate("e", Some(&i), &manifest);
        let text = i.satz_file("pay", &facts, "0.0.0", "satz/e.satz");
        let formatted = satz_core::fmt::format(&text).unwrap_or_else(|e| panic!("{}: {}\n{}", e.line, e.msg, text));
        let file = satz_core::satz::parse(&formatted).unwrap().interface_file.expect("an interface file");
        assert_eq!(file.name, "pay");
        assert_eq!(file.estate, "e");
        assert_eq!(file.organizations, ["organizations/123456789012"]);
        let names: Vec<String> = file.outputs.iter().map(|o| o.name.clone()).collect();
        assert_eq!(names, ["org", "regions", "team_folder", "infra_project", "folders"]);
        for (o, x) in file.outputs.iter().zip(i.module_outputs("pay")) {
            assert_eq!(o.value, x.value, "{}", o.name);
            assert_eq!(o.attach, x.attach, "{}", o.name);
            assert_eq!(o.targets, x.targets, "{}", o.name);
            assert_eq!(o.description, x.description, "{}", o.name);
        }
        assert_eq!(file.outputs[2].value, serde_yaml::Value::String("${data.google_active_folder.team.name}".into()));
        assert_eq!(file.outputs[3].value, serde_yaml::Value::String("corp-infra-001".into()));
        let lookups: Vec<(&str, &str)> = file.lookups.iter().map(|l| (l.address.as_str(), l.reads.as_str())).collect();
        assert_eq!(lookups, [("data.google_active_folder.infra", "google_folder.infra"), ("data.google_active_folder.team", "google_folder.team")]);
        assert_eq!(file.lookups[1].arguments, [("display_name".to_string(), "Team".to_string()), ("parent".to_string(), "${data.google_active_folder.infra.name}".to_string())]);
        // the managed facts travel hashed: the same addresses, refs and keys, no value in clear
        let lineless: Vec<satz_core::satz::ManagedFact> = file.managed.iter().map(|m| satz_core::satz::ManagedFact { line: 0, ..m.clone() }).collect();
        let hashed = |m: &satz_core::satz::ManagedFact| satz_core::satz::ManagedFact {
            line: 0,
            ids: m.ids.iter().map(|i| crate::consumer::hashed(i)).collect(),
            keys: m.keys.iter().map(|(k, v)| (k.clone(), crate::consumer::hashed(v))).collect(),
            ..m.clone()
        };
        assert_eq!(lineless, facts.managed.iter().map(hashed).collect::<Vec<_>>());
        let project = file.managed.iter().find(|m| m.address == "google_project.infra").expect("the project is managed");
        assert_eq!(project.ids, [crate::consumer::hashed("corp-infra-001")]);
        assert!(project.ids[0].starts_with("sha256:") && !project.ids[0].contains("corp"), "{}", project.ids[0]);
        let team = file.managed.iter().find(|m| m.address == "google_folder.team").unwrap();
        assert_eq!(team.refs.get("parent").map(String::as_str), Some("google_folder.infra"));
        // read back, the facts are the estate's
        let used = [satz_core::pipeline::UsedInterfaceFile { file: "pay.satz".into(), interface: file }];
        let back = crate::consumer::Facts::of_files(&used);
        // an interface file names no declaring interface: its names are unique
        let carried: Vec<crate::consumer::FactOutput> =
            facts.outputs.iter().filter(|o| names.contains(&o.name)).map(|o| crate::consumer::FactOutput { interface: None, ..o.clone() }).collect();
        assert_eq!(back.outputs, carried);
        assert_eq!(back.organizations, facts.organizations);
    }

    /// The content hash changes when, and only when, a file changes.
    #[test]
    fn the_content_hash_follows_the_files_and_nothing_else() {
        let a: BTreeMap<String, Vec<u8>> = [("x/README.md".to_string(), b"one".to_vec()), ("x/hcl/outputs.tf".to_string(), b"two".to_vec())].into();
        let same = a.clone();
        assert_eq!(content_hash(&a), content_hash(&same));
        let mut changed = a.clone();
        changed.insert("x/hcl/outputs.tf".into(), b"twO".to_vec());
        assert_ne!(content_hash(&a), content_hash(&changed));
        let mut renamed = BTreeMap::new();
        renamed.insert("x/README.md".to_string(), b"one".to_vec());
        renamed.insert("x/hcl/output.tf".to_string(), b"two".to_vec());
        assert_ne!(content_hash(&a), content_hash(&renamed), "a renamed file is a change");
        // bytes cannot move across a file boundary unnoticed
        let split: BTreeMap<String, Vec<u8>> = [("a".to_string(), b"bc".to_vec())].into();
        let moved: BTreeMap<String, Vec<u8>> = [("ab".to_string(), b"c".to_vec())].into();
        assert_ne!(content_hash(&split), content_hash(&moved));
    }

    #[test]
    fn every_lookup_row_reads_back_through_its_own_data_source() {
        let table = lookups();
        let mut seen = BTreeSet::new();
        for (t, row) in &table {
            assert!(seen.insert(row.data_source.clone()), "two rows read back through `{}`", row.data_source);
            assert!(!row.keys.is_empty(), "{}: a lookup needs a key", t);
            assert!(row.attributes.contains_key(&row.all) || row.derived.contains_key(&row.all) || row.keys.contains_key(&row.all), "{}: `all = {}` is an attribute the row neither yields nor derives", t, row.all);
            for expr in row.attributes.values() {
                assert!(expr.contains("{data}"), "{}: `{}` reads nothing of the data source", t, expr);
            }
        }
    }
}
