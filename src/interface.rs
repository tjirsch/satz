//! The interface an estate publishes to the HCL customer teams write beside it (ADR 0070).
//!
//! Every `export` becomes one output, twice: in the root module's `outputs.tf`, where the
//! operator reads it with `tofu output`, and in the generated module `hcl/interface/`, which
//! a consumer sources from wherever it keeps its own code. The module is relocatable: it
//! names no file outside itself, takes no variable, has no backend and reads no state.
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

/// The directory under `hcl_dir` the module is written to.
pub(crate) const DIR: &str = "interface";

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
}

/// The lookup table, compiled in: what the interface emits for a type is part of the
/// emitter, and one binary emits one interface whatever preset directory it reads.
pub(crate) fn lookups() -> BTreeMap<String, LookupRow> {
    serde_yaml::from_str(include_str!("../presets/interface-lookups.yaml")).expect("presets/interface-lookups.yaml parses")
}

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
    pub name: String,
    pub description: Option<String>,
    /// the value in the module
    pub module_value: String,
    /// the value in the root module
    pub root_value: String,
    pub how: How,
}

/// The interface of one estate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    /// the estate's name, from its header
    pub estate: String,
    pub outputs: Vec<Output>,
    /// by address
    pub data: BTreeMap<String, DataBlock>,
    pub google_source: String,
    pub google_version: Option<String>,
}

/// An export the compile refuses, where it is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
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
    manifest: &Manifest,
    google_source: &str,
    google_version: Option<&str>,
) -> Result<Interface, Vec<Refusal>> {
    let table = lookups();
    let mut r = Resolver { manifest, table: &table, data: BTreeMap::new(), visiting: Vec::new() };
    let mut outputs = Vec::new();
    let mut refusals = Vec::new();
    for x in exports {
        match r.value(&x.value) {
            Ok((module_value, root_value, parts_all_static, used)) => {
                let how = if parts_all_static {
                    How::Static(module_value.clone())
                } else {
                    let mut all: BTreeSet<String> = BTreeSet::new();
                    for d in &used {
                        r.closure(d, &mut all);
                    }
                    How::Lookup(all.into_iter().collect())
                };
                outputs.push(Output {
                    name: x.name.clone(),
                    description: x.description.clone(),
                    module_value,
                    root_value,
                    how,
                });
            }
            Err(msg) => refusals.push(Refusal { name: x.name.clone(), file: x.file.clone(), line: x.line, msg }),
        }
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    Ok(Interface {
        estate: estate.to_string(),
        outputs,
        data: r.data,
        google_source: google_source.to_string(),
        google_version: google_version.map(str::to_string),
    })
}

struct Resolver<'a> {
    manifest: &'a Manifest,
    table: &'a BTreeMap<String, LookupRow>,
    data: BTreeMap<String, DataBlock>,
    /// the resources whose lookup is being built, for a cycle
    visiting: Vec<String>,
}

impl Resolver<'_> {
    /// One export value: (module HCL, root HCL, whether it is known now, the data blocks it
    /// reads directly).
    fn value(&mut self, v: &serde_yaml::Value) -> Result<(String, String, bool, Vec<String>), String> {
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
                Ok((render_parts(&parts), root_text(s), known, used))
            }
            Value::Number(n) => Ok((n.to_string(), n.to_string(), true, Vec::new())),
            Value::Bool(b) => Ok((b.to_string(), b.to_string(), true, Vec::new())),
            Value::Sequence(items) => {
                let mut module = Vec::new();
                let mut root = Vec::new();
                let mut known = true;
                let mut used = Vec::new();
                for i in items {
                    if matches!(i, Value::Sequence(_) | Value::Mapping(_)) {
                        return Err("a list export holds strings, numbers or bools — a nested list or an object is no output value here".to_string());
                    }
                    let (m, r, k, u) = self.value(i)?;
                    module.push(m);
                    root.push(r);
                    known &= k;
                    used.extend(u);
                }
                Ok((format!("[{}]", module.join(", ")), format!("[{}]", root.join(", ")), known, used))
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
        }
        self.visiting.pop();
        let block = DataBlock {
            data_source: row.data_source.clone(),
            label: label.to_string(),
            resource,
            args,
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

impl Interface {
    /// The root module's `outputs.tf`: the values in one local, one output per export.
    pub fn root_outputs_tf(&self) -> String {
        let mut out = String::new();
        out.push_str("locals {\n");
        out.push_str(&format!("  {} = {{\n", LOCAL));
        out.push_str("    interface = 1\n");
        out.push_str(&format!("    estate    = {}\n", hcl_string(&self.estate)));
        out.push_str("    values = {\n");
        let w = self.outputs.iter().map(|o| o.name.len()).max().unwrap_or(0);
        for o in &self.outputs {
            out.push_str(&format!("      {:<w$} = {}\n", o.name, o.root_value, w = w));
        }
        out.push_str("    }\n  }\n}\n");
        for o in &self.outputs {
            out.push_str(&format!("\noutput \"{}\" {{\n", o.name));
            out.push_str(&output_body(&o.description, &format!("local.{}.values.{}", LOCAL, o.name)));
            out.push_str("}\n");
        }
        out
    }

    /// The module's `versions.tf`: the provider it needs, and nothing that configures it —
    /// the consumer's own provider is the one the lookups run through.
    pub fn module_versions_tf(&self) -> String {
        let mut out = String::from("terraform {\n  required_providers {\n    google = {\n");
        out.push_str(&format!("      source  = {}\n", hcl_string(&self.google_source)));
        if let Some(v) = &self.google_version {
            out.push_str(&format!("      version = {}\n", hcl_string(v)));
        }
        out.push_str("    }\n  }\n}\n");
        out
    }

    /// The module's `main.tf`: the lookups, or nothing when every export is static.
    pub fn module_main_tf(&self) -> String {
        let mut out = String::new();
        for d in self.data.values() {
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

    /// The module's `outputs.tf`.
    pub fn module_outputs_tf(&self) -> String {
        let mut out = String::new();
        for o in &self.outputs {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("output \"{}\" {{\n", o.name));
            out.push_str(&output_body(&o.description, &o.module_value));
            out.push_str("}\n");
        }
        out
    }

    /// The files of `hcl/interface/`, by name, without their stamps.
    pub fn module_files(&self) -> Vec<(&'static str, String)> {
        let mut files = vec![("versions.tf", self.module_versions_tf())];
        let main = self.module_main_tf();
        if !main.is_empty() {
            files.push(("main.tf", main));
        }
        files.push(("outputs.tf", self.module_outputs_tf()));
        files
    }

    /// `hcl/interface/README.md`, which travels with the module.
    pub fn readme(&self, version: &str, estate_path: &str, notice: Option<&Notice>) -> String {
        let mut s = String::new();
        s.push_str(&format!("# The interface of estate `{}`\n\n", self.estate));
        s.push_str(&format!(
            "Generated by satz v{} from `{}`. Do not edit: every `satz transpile` writes this directory whole.\n\n",
            version, estate_path
        ));
        s.push_str("This module publishes the values the estate exports to HCL that is not part of it — a team's own\n");
        s.push_str("code, with its own state, in its own repository. It references no file outside this directory, takes\n");
        s.push_str("no input variable and reads no state: copy it, move it, or source it by git URL.\n\n");
        s.push_str("## How to use it\n\n");
        s.push_str("```hcl\nmodule \"satz\" {\n  source = \"<path or git URL>/interface\"\n}\n\n");
        match self.outputs.first() {
            Some(o) => s.push_str(&format!("# module.satz.{}\n```\n\n", o.name)),
            None => s.push_str("```\n\n"),
        }
        s.push_str(&format!(
            "It needs the `google` provider ({}{}) in the calling configuration. A lookup runs through that\n\
             provider, with the caller's credentials, and needs the read permission its row names.\n\n",
            self.google_source,
            self.google_version.as_deref().map(|v| format!(" {}", v)).unwrap_or_default()
        ));
        s.push_str("## Exports\n\n");
        s.push_str("| Output | Description | How it is obtained |\n|---|---|---|\n");
        for o in &self.outputs {
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
            s.push_str(&format!(
                "| `{}` | {} | {} |\n",
                o.name,
                o.description.as_deref().map(table_cell).unwrap_or_default(),
                how
            ));
        }
        s.push_str("\n## Attach points\n\n");
        s.push_str("None are declared. A write to shared infrastructure goes into this estate as a contribution.\n\n");
        s.push_str("## Change notice\n\n");
        match notice {
            Some(n) => {
                s.push_str(&format!(
                    "Every apply that changes an exported value rewrites `{}`, and Cloud Storage publishes one\n\
                     message to the topic `{}`. An apply that changes nothing publishes nothing. The object holds\n\
                     the values as JSON (`interface`, `estate`, `values`); the message names the object.\n\n\
                     Subscribe in your own state:\n\n\
                     ```hcl\n\
                     resource \"google_pubsub_subscription\" \"satz_interface\" {{\n  \
                       name    = \"<team>-satz-interface\"\n  \
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

/// What the change notice publishes, when the estate uses `interface-notice`.
pub(crate) struct Notice {
    /// the object URL (`gs://…/interface.json`)
    pub object: String,
    /// the topic id
    pub topic: String,
    /// the output a consumer reads the topic from
    pub topic_output: String,
}

/// The change notice, read off the interface's own outputs: `interface_topic` and
/// `interface_object` are what the pack exports.
pub(crate) fn notice(interface: &Interface) -> Option<Notice> {
    let value = |name: &str| {
        interface.outputs.iter().find(|o| o.name == name).and_then(|o| match &o.how {
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
        ResolvedExport { name: name.into(), value: serde_yaml::Value::String(value.into()), description: None, file: "e.satz".into(), line: 3 }
    }

    fn build_one(value: &str) -> Result<Interface, Vec<Refusal>> {
        build("e", &[export("x", value)], &Manifest::parse(MAIN_TF), "hashicorp/google", Some("7.14.1"))
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
        let main = i.module_main_tf();
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

    #[test]
    fn the_readme_has_one_row_per_output_and_the_module_one_output_per_export() {
        let exports = [export("org", "123"), export("folder", "${google_folder.team.name}")];
        let i = build("e", &exports, &Manifest::parse(MAIN_TF), "hashicorp/google", Some("7.14.1")).unwrap();
        let readme = i.readme("0.0.0", "satz/e.satz", None);
        let rows: Vec<&str> = readme.lines().filter(|l| l.starts_with("| `")).collect();
        let module = i.module_outputs_tf();
        let outputs: Vec<&str> = module.lines().filter(|l| l.starts_with("output ")).collect();
        assert_eq!(rows.len(), exports.len());
        assert_eq!(outputs.len(), exports.len());
        for x in &exports {
            assert!(rows.iter().any(|r| r.starts_with(&format!("| `{}` |", x.name))));
            assert!(outputs.contains(&format!("output \"{}\" {{", x.name).as_str()));
        }
        assert!(readme.contains("No change notice is set up"));
    }

    #[test]
    fn every_lookup_row_reads_back_through_its_own_data_source() {
        let table = lookups();
        let mut seen = BTreeSet::new();
        for (t, row) in &table {
            assert!(seen.insert(row.data_source.clone()), "two rows read back through `{}`", row.data_source);
            assert!(!row.keys.is_empty(), "{}: a lookup needs a key", t);
            for expr in row.attributes.values() {
                assert!(expr.contains("{data}"), "{}: `{}` reads nothing of the data source", t, expr);
            }
        }
    }
}
