//! Named uses (`docs/language.md` §6.9, ADR 0109): `use "<path>" named <name>` makes one
//! use of a pack under a name, with params of its own, so one pack can be used several times. What this
//! module holds is what the walks share: the estate's bindings `<name>.<param>`, the pack
//! read before its use is walked (the params it owns, what a named use refuses), and the
//! prefix the use's resources are written under.

use super::{perr, PipelineError, GRANT_SCOPE_SEP};
use crate::algebra::{Body, Entity, Fragment, GrantEdge, Rendering};
use crate::satz::{self, Entry, File, Key, StrPart, Value, NAMED_SEP};
use crate::Address;
use std::collections::{BTreeMap, BTreeSet};

/// The estate's bindings of its named uses' params — `<name>.<param> = …` in its `params`
/// — taken out of its flat params: a dotted name is no param of the flat namespace.
#[derive(Debug, Clone, Default)]
pub(crate) struct NamedBindings {
    by_use: BTreeMap<String, Vec<(String, Value, usize)>>,
}

impl NamedBindings {
    /// The dotted params of `file`, removed from it. The parser has refused a dotted param
    /// that names no named use of the file.
    pub(crate) fn take(file: &mut File) -> NamedBindings {
        let mut by_use: BTreeMap<String, Vec<(String, Value, usize)>> = BTreeMap::new();
        file.params.retain(|(name, v, line)| match satz::named_binding(name) {
            Some((n, p)) => {
                by_use.entry(n.to_string()).or_default().push((p.to_string(), v.clone(), *line));
                false
            }
            None => true,
        });
        NamedBindings { by_use }
    }

    pub(crate) fn of(&self, name: &str) -> &[(String, Value, usize)] {
        self.by_use.get(name).map(Vec::as_slice).unwrap_or_default()
    }
}

/// A pack used with `named`, read before the use is walked: the params it and every file it
/// uses declare, at any depth and whatever their gates — every one belongs to the use —
/// with the file and line of the first declaration.
#[derive(Debug, Clone)]
pub(crate) struct NamedPack {
    pub owned: BTreeMap<String, (String, usize)>,
}

/// What a file of a named use may not hold, with why: the first one `file` holds.
fn refused_in_named(file: &File) -> Option<(usize, &'static str, &'static str)> {
    let found: [(Option<usize>, &str, &str); 9] = [
        (file.hcl_blocks.first().map(|h| h.line), "an `hcl` block", "raw HCL is written as it stands, so what it declares cannot take the use's prefix"),
        (file.actions.first().map(|a| a.line), "an `action`", "an action runs once per estate under its name, and two uses would declare one name twice"),
        (file.inputs.first().map(|i| i.line), "an `inputs` block", "an input is a variable of the estate's own, and two uses would declare one variable twice"),
        (file.interfaces.first().map(|i| i.line), "an `interface` block", "an interface is published under its name, and two uses would publish one name twice"),
        (file.interface_file.as_ref().map(|i| i.line), "an interface file", "the values of an interface file are the estate's, read by the flat namespace"),
        (file.exports.first().map(|x| x.line), "an `export`", "an export of a named use is not supported yet: it would be published under the use's prefix"),
        (file.requests.first().map(|r| r.line), "an `entries` statement", "a list param of a named use is not supported yet: entries and contributions to it would be bound per use"),
        (file.notices.first().map(|n| n.line), "a `notice`", "a notice of a named use is not supported yet: its acknowledgement would be bound per use"),
        (file.offers.first().map(|o| o.line), "an `offers` entry", "what the library offers is the map's, which is used once"),
    ];
    found.into_iter().find_map(|(line, what, why)| line.map(|l| (l, what, why)))
}

/// The pack `path` used as `name` at `file_name`:`line`, read whole: its header says
/// `multiple`, no file it uses holds what a named use refuses (`refused_in_named`) or a
/// named use of its own, and the params all of them declare are the use's.
pub(crate) fn named_pack(
    name: &str,
    path: &str,
    file_name: &str,
    line: usize,
    load: &dyn Fn(&str) -> Result<String, String>,
) -> Result<NamedPack, PipelineError> {
    let said = format!("use \"{}\" named {}", path, name);
    let read = |p: &str| -> Result<File, PipelineError> {
        if p.ends_with(".yaml") || p.ends_with(".yml") {
            return perr(file_name, line, format!("use \"{}\": a pack is Satz — satz {} is the last release that converts the pre-Satz YAML dialect", p, crate::LAST_YAML_CONVERTING_RELEASE));
        }
        let src = load(p).map_err(|e| PipelineError { file: file_name.to_string(), line, msg: e })?;
        satz::parse(&src).map_err(|e| PipelineError { file: p.to_string(), line: e.line, msg: e.msg })
    };
    let root = read(path)?;
    if !(root.is_pack && root.multiple) {
        let what = match (&root.estate, root.is_pack) {
            (Some(p), true) => format!("the pack `{}` does not say `multiple` in its header", p),
            _ => format!("{} is no pack", path),
        };
        return perr(
            file_name,
            line,
            format!(
                "{}: {} — the word is its author's promise that the cloud names of what the pack creates come from its params, so that two named uses create two resources and not one resource twice. A named use takes a pack whose header is `pack <name> version \"…\" multiple`; use this one without `named`",
                said, what
            ),
        );
    }
    let mut owned: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<(String, File)> = vec![(path.to_string(), root)];
    while let Some((p, file)) = todo.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        if let Some((l, what, why)) = refused_in_named(&file) {
            return perr(file_name, line, format!("{}: {}:{} holds {}, and a named use takes none — {}. Use the pack without `named`", said, p, l, what, why));
        }
        if let Some((n, inner, l)) = satz::named_uses(&file).first() {
            return perr(file_name, line, format!("{}: {}:{} holds `use \"{}\" named {}` — a named use stands in the estate file, and a pack used under a name holds no named use of its own", said, p, l, inner, n));
        }
        for (param, _, l) in &file.params {
            if satz::contribution_target(param).is_none() {
                owned.entry(param.clone()).or_insert_with(|| (p.clone(), *l));
            }
        }
        for used in satz::use_paths(&file) {
            if !seen.contains(&used) {
                todo.push((used.clone(), read(&used)?));
            }
        }
    }
    Ok(NamedPack { owned })
}

/// The estate's bindings of `name`'s params checked against the params the pack owns: a
/// binding of a param the pack does not declare is refused, naming its params.
pub(crate) fn check_bindings(name: &str, path: &str, pack: &NamedPack, bindings: &NamedBindings, file_name: &str) -> Result<(), PipelineError> {
    for (p, _, line) in bindings.of(name) {
        if !pack.owned.contains_key(p) {
            let params: Vec<String> = pack.owned.keys().map(|k| format!("`{}`", k)).collect();
            return perr(
                file_name,
                *line,
                format!(
                    "params: `{}.{}` binds the param `{}` of the named use `{}`, and {} declares no param `{}` — its params are {}",
                    name,
                    p,
                    p,
                    name,
                    path,
                    p,
                    if params.is_empty() { "none".to_string() } else { params.join(", ") }
                ),
            );
        }
    }
    Ok(())
}

/// The name a param of a named use is shown under: `web.vm_zone`, as the estate binds it.
pub fn bound_name(name: &str, param: &str) -> String {
    format!("{}.{}", name, param)
}

/// The name a param of a named use is written under in the HCL code: `web__vm_zone`,
/// which `variables.tf` and `terraform.tfvars` write as `web--vm-zone`.
pub(crate) fn variable_name(name: &str, param: &str) -> String {
    format!("{}{}{}", name, NAMED_SEP, param)
}

/// Every param a body reads by its bare name or inside a string, at any depth.
pub(crate) fn reads_of_entries(entries: &[Entry], out: &mut BTreeSet<String>) {
    for e in entries {
        match e {
            Entry::Attr { key, value, .. } => {
                reads_of_key(key, out);
                reads_of_value(value, out);
            }
            Entry::Map { key, name, body, .. } => {
                reads_of_key(key, out);
                if let Some(n) = name {
                    reads_of_key(n, out);
                }
                reads_of_entries(body, out);
            }
            Entry::When { cond, entry, .. } => {
                out.insert(cond.name.clone());
                reads_of_entries(std::slice::from_ref(entry.as_ref()), out);
            }
            Entry::Each { list, body, .. } => {
                out.insert(list.clone());
                reads_of_entries(body, out);
            }
            Entry::Use { .. } => {}
        }
    }
}

fn reads_of_key(k: &Key, out: &mut BTreeSet<String>) {
    if let Key::Str(parts) = k {
        reads_of_parts(parts, out);
    }
}

fn reads_of_parts(parts: &[StrPart], out: &mut BTreeSet<String>) {
    for p in parts {
        if let StrPart::Param(n) = p {
            out.insert(n.clone());
        }
    }
}

pub(crate) fn reads_of_value(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Ref(n) => {
            out.insert(n.clone());
        }
        Value::Str(parts) => reads_of_parts(parts, out),
        Value::List(items) => items.iter().for_each(|i| reads_of_value(i, out)),
        Value::Obj(entries) => reads_of_entries(entries, out),
        Value::Num(_) | Value::Bool(_) | Value::None => {}
    }
}

/// What gives a named use's resources their prefix: every resource it declares — an entity,
/// a folder, a project, a labelled grant resource — is written as `<name>__<label>`, every
/// reference to one of them is rewritten, and the folder and project steps of a structural
/// path follow. A grant of a grant map keeps its address — its identity is the member, the
/// role and the scope, and the scope follows the node it names.
pub(crate) struct Prefix {
    name: String,
    /// `(type, label as written in HCL)` → the label prefixed, as written in HCL
    refs: BTreeMap<(String, String), String>,
    /// `folder:<label>` / `project:<label>` → the step prefixed
    steps: BTreeMap<String, String>,
}

/// Whether the walk wrote this address as a resource of its own: what a named use prefixes.
pub(crate) fn declares(addr: &Address, e: &Entity) -> bool {
    matches!(e.body, Body::Attrs(_)) && !addr.tf_type.starts_with("__")
}

impl Prefix {
    pub(crate) fn new(name: &str, frags: &[Fragment]) -> Prefix {
        let mut refs = BTreeMap::new();
        let mut steps = BTreeMap::new();
        for (addr, e) in frags.iter().flat_map(|f| f.entities.iter()) {
            if !declares(addr, e) {
                continue;
            }
            let hcl = addr.label.replace('-', "_");
            refs.insert((addr.tf_type.clone(), hcl.clone()), format!("{}{}{}", name, NAMED_SEP, hcl));
            let step = match addr.tf_type.as_str() {
                "google_folder" => Some("folder:"),
                "google_project" => Some("project:"),
                _ => None,
            };
            if let Some(s) = step {
                steps.insert(format!("{}{}", s, addr.label), format!("{}{}{}{}", s, name, NAMED_SEP, addr.label));
            }
        }
        Prefix { name: name.to_string(), refs, steps }
    }

    /// The fragments with the prefix given.
    pub(crate) fn apply(&self, frags: Vec<Fragment>) -> Vec<Fragment> {
        frags
            .into_iter()
            .map(|f| {
                let mut out = Fragment::default();
                for (addr, e) in f.entities {
                    let label = if declares(&addr, &e) { format!("{}{}{}", self.name, NAMED_SEP, addr.label) } else { self.grant_label(&addr.label) };
                    let addr = Address { tf_type: addr.tf_type, label };
                    let body = match e.body {
                        Body::Attrs(v) => Body::Attrs(self.value(&v)),
                        Body::Grant(edges) => Body::Grant(
                            edges
                                .into_iter()
                                .map(|g| GrantEdge {
                                    member: self.text(&g.member),
                                    role: self.text(&g.role),
                                    condition: self.text(&g.condition),
                                    import_id: g.import_id,
                                    rendered_member: self.text(&g.rendered_member),
                                    rendered_role: self.text(&g.rendered_role),
                                })
                                .collect(),
                        ),
                    };
                    let rendering = e.rendering.map(|r| Rendering { attrs: r.attrs.map(|a| self.value(&a)), scope: r.scope.map(|s| self.text(&s)) });
                    let node_path = e.node_path.iter().map(|s| self.step(s)).collect();
                    out.entities.insert(addr.clone(), Entity { addr, scope: e.scope, body, provenance: e.provenance, node_path, rendering });
                }
                out
            })
            .collect()
    }

    fn step(&self, s: &str) -> String {
        self.steps.get(s).cloned().unwrap_or_else(|| s.to_string())
    }

    /// A grant's label: its structural path or scope pin, the separator, the member.
    fn grant_label(&self, label: &str) -> String {
        match label.split_once(GRANT_SCOPE_SEP) {
            Some((scope, member)) => {
                let scope = if scope.starts_with("folder:") || scope.starts_with("project:") {
                    scope.split('/').map(|s| self.step(s)).collect::<Vec<_>>().join("/")
                } else {
                    match scope.split_once('=') {
                        Some((attr, value)) => format!("{}={}", attr, self.text(value)),
                        None => scope.to_string(),
                    }
                };
                format!("{}{}{}", scope, GRANT_SCOPE_SEP, self.text(member))
            }
            None => self.text(label),
        }
    }

    fn value(&self, v: &serde_yaml::Value) -> serde_yaml::Value {
        match v {
            serde_yaml::Value::String(s) => serde_yaml::Value::String(self.text(s)),
            serde_yaml::Value::Sequence(items) => serde_yaml::Value::Sequence(items.iter().map(|i| self.value(i)).collect()),
            serde_yaml::Value::Mapping(m) => serde_yaml::Value::Mapping(m.iter().map(|(k, v)| (self.value(k), self.value(v))).collect()),
            serde_yaml::Value::Tagged(t) => serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue { tag: t.tag.clone(), value: self.value(&t.value) })),
            other => other.clone(),
        }
    }

    /// A text with every reference to a resource of the use rewritten: inside `${…}`, and a
    /// whole text that is a dotted path (`google_folder.f.name`, the form a folder's parent
    /// and a project's `folder_id` take).
    pub(crate) fn text(&self, s: &str) -> String {
        if !s.contains("${") {
            return self.traversal(s).unwrap_or_else(|| s.to_string());
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            let end = rest.find('}').unwrap_or(rest.len());
            let inner = &rest[..end];
            let trimmed = inner.trim_start();
            out.push_str(&inner[..inner.len() - trimmed.len()]);
            out.push_str(&self.traversal(trimmed).unwrap_or_else(|| trimmed.to_string()));
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }

    /// `<type>.<label>…` with the label prefixed, where it names a resource of the use.
    fn traversal(&self, t: &str) -> Option<String> {
        if !t.starts_with("google_") {
            return None;
        }
        let (ty, rest) = t.split_once('.')?;
        let label: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        if label.is_empty() || !ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let tail = &rest[label.len()..];
        if !tail.is_empty() && !tail.starts_with('.') && !tail.starts_with('[') {
            return None;
        }
        let new = self.refs.get(&(ty.to_string(), label.replace('-', "_")))?;
        Some(format!("{}.{}{}", ty, new, tail))
    }

    /// A claim's witness, `<type>.<label>`, prefixed where the use declares it.
    pub(crate) fn witness(&self, w: &str) -> String {
        self.traversal(w).unwrap_or_else(|| w.to_string())
    }
}

/// One named use the walk passed: its name, the pack, the `use` line, and every address it
/// wrote with the params of the pack the resource there reads — what names the param to
/// bind differently when two uses write one resource.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedUse {
    pub name: String,
    pub path: String,
    pub file: String,
    pub line: usize,
    pub addresses: BTreeMap<Address, BTreeSet<String>>,
    /// The use's params, as `terraform.tfvars` writes them: `web__vm_zone`
    pub params: Vec<String>,
}

/// One writer of a resource body: the named use (`None`: the estate), the address, and the
/// file and line that declare it.
type Writer = (Option<usize>, Address, String);

/// Two resources with one body — written by two named uses, or by a named use and the estate
/// — are one resource in the cloud under two addresses, which an apply creates twice and a
/// destroy of either removes. Refused, naming both and the params to bind differently.
pub(crate) fn refuse_identical(uses: &[NamedUse], frags: &[Fragment], estate_file: &str) -> Result<(), PipelineError> {
    if uses.is_empty() {
        return Ok(());
    }
    let owner_of = |addr: &Address| uses.iter().position(|u| u.addresses.contains_key(addr));
    // (type, body) → its writers
    let mut by_body: BTreeMap<(String, String), Vec<Writer>> = BTreeMap::new();
    for (addr, e) in frags.iter().flat_map(|f| f.entities.iter()) {
        if !declares(addr, e) {
            continue;
        }
        let Body::Attrs(v) = &e.body else { continue };
        let Ok(text) = serde_yaml::to_string(v) else { continue };
        let site = e.provenance.first().map(|s| format!("{}:{}", s.file, s.line)).unwrap_or_default();
        let writers = by_body.entry((addr.tf_type.clone(), text)).or_default();
        if !writers.iter().any(|(_, a, _)| a == addr) {
            writers.push((owner_of(addr), addr.clone(), site));
        }
    }
    for writers in by_body.values() {
        for (i, (o1, a1, s1)) in writers.iter().enumerate() {
            let Some((o2, a2, s2)) = writers[i + 1..].iter().find(|(o2, _, _)| o2 != o1 && (o1.is_some() || o2.is_some())) else { continue };
            let (u1, u2) = (o1.map(|i| &uses[i]), o2.map(|i| &uses[i]));
            let mut reads: BTreeSet<String> = BTreeSet::new();
            for (u, a) in [(u1, a1), (u2, a2)] {
                if let Some(r) = u.and_then(|u| u.addresses.get(a)) {
                    reads.extend(r.iter().cloned());
                }
            }
            let bind = if reads.is_empty() {
                "It reads none of the pack's params, so every use of the pack writes it alike: a pack that writes it is used once".to_string()
            } else {
                let named: Vec<&NamedUse> = [u1, u2].into_iter().flatten().collect();
                let names: Vec<String> = reads
                    .iter()
                    .flat_map(|p| named.iter().map(move |u| format!("`{}`", bound_name(&u.name, p))))
                    .collect();
                format!("Bind {} differently", names.join(", "))
            };
            let line = u2.or(u1).map(|u| u.line).unwrap_or(0);
            let who = match (u1, u2) {
                (Some(a), Some(b)) => format!("the named uses `{}` and `{}` write", a.name, b.name),
                (Some(a), None) | (None, Some(a)) => format!("the named use `{}` and the estate write", a.name),
                (None, None) => unreachable!("one writer is a named use"),
            };
            return perr(
                estate_file,
                line,
                format!(
                    "{} the same resource — `{}.{}` ({}) and `{}.{}` ({}) have one body, which is one resource in the cloud under two addresses: an apply creates it twice and a destroy of either removes it. {}",
                    who, a1.tf_type, a1.label, s1, a2.tf_type, a2.label, s2, bind
                ),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Named uses (ADR 0109): a pack used under a name, with params of its own, its resources
    //! prefixed, and every form a named use refuses with its message.
    use super::super::*;
    use crate::algebra::Body;

    struct Table;
    impl TypeResolver for Table {
        fn body_keys(&self, _tf_type: &str, _path: &[&str]) -> Option<BodyKeys> {
            None
        }
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            let (class, scope) = match key {
                "google_storage_bucket" => (MergeClass::Entity, Scope::Node),
                "google_organization_iam_member" => (MergeClass::Grant, Scope::Org),
                "google_project_iam_member" => (MergeClass::Grant, Scope::Node),
                _ => return None,
            };
            Some(ResolvedType { tf_type: key.into(), class, scope })
        }
    }

    fn compile_with(src: &str, files: &[(&str, &str)]) -> Result<FrontEnd, PipelineError> {
        let load = |p: &str| files.iter().find(|(n, _)| *n == p).map(|(_, s)| s.to_string()).ok_or_else(|| format!("no such file {}", p));
        compile_estate("main.satz", src, &Table, &load)
    }

    fn refused(src: &str, files: &[(&str, &str)]) -> PipelineError {
        match compile_with(src, files) {
            Ok(_) => panic!("must be refused:\n{}", src),
            Err(e) => e,
        }
    }

    fn says(e: &PipelineError, what: &str) {
        assert!(e.msg.contains(what), "the message must say `{}`:\n{}:{}: {}", what, e.file, e.line, e.msg);
    }

    /// Every entity the estate writes with its body, by address.
    fn bodies(fe: &FrontEnd) -> BTreeMap<String, serde_yaml::Value> {
        fe.fragments
            .iter()
            .flat_map(|f| f.entities.values())
            .filter_map(|e| match &e.body {
                Body::Attrs(v) => Some((format!("{}.{}", e.addr.tf_type, e.addr.label), v.clone())),
                Body::Grant(_) => None,
            })
            .collect()
    }

    fn attr(v: &serde_yaml::Value, k: &str) -> String {
        v.get(k).map(|x| match x {
            serde_yaml::Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
        }).unwrap_or_default()
    }

    const VM: &str = r#"pack vm version "1.0" multiple

params {
  vm_name = "acme-vm"
  vm_zone = "europe-west3-a"
  vm_size = "small"
  vm_logs = false
  contributes_members = [
    "group:vm@example.com",
    "user:{vm_name}@example.com",
  ]
}

locals {
  vm_bucket = "{vm_name}-data"
}

question vm_zone {
  prompt   = "The zone of the machine"
  reversal = edit
  blast    = none
}

google_storage_bucket {
  data {
    name     = vm_bucket
    location = vm_zone
    labels   = { size = vm_size }
  }
  logs when vm_logs {
    name     = "{vm_name}-logs"
    location = "${{google_storage_bucket.data.location}}"
    labels   = { network = "${{google_storage_bucket.shared.name}}" }
  }
}

google_organization_iam_member {
  "group:vm@example.com" = ["roles/viewer"]
}

claim "cis-gcp" "4.0" "5.1" contributes {
  resources = ["google_storage_bucket.data"]
}
"#;

    fn estate(body: &str) -> String {
        format!("estate t\n\nparams {{\n  customer_organization_id = \"123456789012\"\n  members = [\"user:owner@example.com\"]\n{}}}\n", body)
    }

    #[test]
    fn a_named_use_has_params_of_its_own() {
        let src = estate("  vm_zone = \"flat-zone\"\n  want_db = true\n  web.vm_name = \"acme-web\"\n  web.vm_logs = true\n  db.vm_name = \"acme-db\"\n  db.vm_zone = \"europe-west3-b\"\n")
            + "\nuse \"vm.satz\" named web\nuse \"vm.satz\" named db when want_db\nuse \"vm.satz\"\n\ngoogle_storage_bucket {\n  shared {\n    name = \"acme-shared\"\n  }\n}\n";
        let fe = compile_with(&src, &[("vm.satz", VM)]).unwrap_or_else(|e| panic!("{}:{}: {}", e.file, e.line, e.msg));
        let b = bodies(&fe);
        assert_eq!(attr(&b["google_storage_bucket.web__data"], "name"), "acme-web-data", "a local value is the use's own");
        assert_eq!(attr(&b["google_storage_bucket.web__data"], "location"), "europe-west3-a", "a flat binding of the same name never reaches the use");
        assert_eq!(attr(&b["google_storage_bucket.db__data"], "location"), "europe-west3-b");
        assert_eq!(attr(&b["google_storage_bucket.data"], "location"), "flat-zone", "the flat use reads the flat binding");
        assert_eq!(attr(&b["google_storage_bucket.web__logs"], "location"), "${google_storage_bucket.web__data.location}", "a reference to the use's own resource is prefixed");
        let labels = &b["google_storage_bucket.web__logs"]["labels"];
        assert_eq!(attr(labels, "network"), "${google_storage_bucket.shared.name}", "a reference outside the use stays");
        assert!(!b.contains_key("google_storage_bucket.db__logs") && !b.contains_key("google_storage_bucket.logs"), "a `when` inside the pack reads the use's own param");
        // one grant, written by all three, not prefixed
        let grants: Vec<&Entity> = fe.fragments.iter().flat_map(|f| f.entities.values()).filter(|e| e.addr.tf_type == "google_organization_iam_member").collect();
        assert!(grants.iter().all(|g| g.addr.label == "group:vm@example.com"), "{:?}", grants.iter().map(|g| &g.addr.label).collect::<Vec<_>>());
        // the uses' params, as the HCL code names them
        assert_eq!(fe.tfvars["web__vm_name"], serde_yaml::Value::from("acme-web"));
        assert_eq!(fe.tfvars["web__vm_zone"], serde_yaml::Value::from("europe-west3-a"));
        assert_eq!(fe.tfvars["db__vm_zone"], serde_yaml::Value::from("europe-west3-b"));
        assert_eq!(fe.tfvars["vm_zone"], serde_yaml::Value::from("flat-zone"));
        assert!(!fe.tfvars.keys().any(|k| k.contains('.')), "a binding is no flat param: {:?}", fe.tfvars.keys());
        // contributions of every use reach the flat list, the shared entry once
        let members: Vec<&str> = fe.tfvars["members"].as_sequence().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(members, ["user:owner@example.com", "group:vm@example.com", "user:acme-web@example.com", "user:acme-db@example.com", "user:acme-vm@example.com"]);
        // the use's question names its param as the HCL code does
        let subjects: Vec<&str> = fe.questions.iter().flat_map(|q| q.questions.iter()).map(|q| q.subject.as_str()).collect();
        assert_eq!(subjects, ["web__vm_zone", "db__vm_zone", "vm_zone"]);
        // a claim's witness follows the address
        let witnesses: Vec<&str> = fe.claims.iter().flat_map(|c| c.claims.iter()).flat_map(|c| c.resources.iter()).map(String::as_str).collect();
        assert_eq!(witnesses, ["google_storage_bucket.web__data", "google_storage_bucket.db__data", "google_storage_bucket.data"]);
        assert_eq!(fe.named.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["web", "db"]);
    }

    #[test]
    fn a_gate_on_a_named_use_reads_the_flat_namespace() {
        let src = estate("  want_db = false\n  db.vm_name = \"acme-db\"\n") + "\nuse \"vm.satz\" named db when want_db\n";
        let fe = compile_with(&src, &[("vm.satz", VM)]).unwrap();
        assert!(bodies(&fe).is_empty() && fe.named.is_empty() && !fe.tfvars.contains_key("db__vm_name"));
    }

    #[test]
    fn a_param_declared_none_has_no_value_per_use() {
        let pack = "pack p version \"1.0\" multiple\n\nparams {\n  p_name = none\n}\n\ngoogle_storage_bucket {\n  b {\n    name = p_name\n  }\n}\n";
        let src = estate("  a.p_name = \"acme-a\"\n") + "\nuse \"p.satz\" named a\nuse \"p.satz\" named b\n";
        let e = refused(&src, &[("p.satz", pack)]);
        assert_eq!(e.file, "p.satz");
        says(&e, "param `b.p_name` has no value");
        let src = estate("  a.p_name = \"acme-a\"\n  b.p_name = \"acme-b\"\n") + "\nuse \"p.satz\" named a\nuse \"p.satz\" named b\n";
        let fe = compile_with(&src, &[("p.satz", pack)]).unwrap();
        assert_eq!(attr(&bodies(&fe)["google_storage_bucket.b__b"], "name"), "acme-b");
    }

    /// ADR 0108: a param no file gives a value takes its shape from where it is read — a
    /// named use's from the use's reads, a flat one's from the flat reads.
    #[test]
    fn a_param_s_shape_is_read_per_use() {
        let named = "pack p version \"1.0\" multiple\n\nparams {\n  p_x = none\n}\n\ngoogle_storage_bucket {\n  b {\n    name = \"{p_x}\"\n  }\n}\n";
        let flat = "pack q version \"1.0\"\n\nparams {\n  p_x = none\n}\n\ngoogle_storage_bucket {\n  each p_x by name {\n    name = \"q-{each.name}\"\n  }\n}\n";
        let src = estate("  p_x = [{ name = \"a\" }]\n  web.p_x = \"w\"\n") + "\nuse \"q.satz\"\nuse \"p.satz\" named web\n";
        let fe = compile_with(&src, &[("p.satz", named), ("q.satz", flat)]).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
        let b = bodies(&fe);
        assert_eq!(attr(&b["google_storage_bucket.web__b"], "name"), "w");
        assert_eq!(attr(&b["google_storage_bucket.a"], "name"), "q-a");
        // the shapes, each under its own name
        let load = |p: &str| match p {
            "p.satz" => Ok(named.to_string()),
            "q.satz" => Ok(flat.to_string()),
            _ => Err(p.to_string()),
        };
        let (_, _, _, uses) = questions_walk(satz::parse(&src).unwrap(), "main.satz", &load).unwrap();
        let shapes = uses.shapes(&Table).unwrap();
        assert_eq!(shapes.get("web.p_x"), Some(&Shape::String), "{:?}", shapes);
        assert_eq!(shapes.get("p_x"), Some(&Shape::List), "{:?}", shapes);
    }

    #[test]
    fn a_use_and_a_flat_pack_declaring_one_name_differently_are_two_params() {
        let other = "pack o version \"1.0\"\n\nparams {\n  vm_zone = \"elsewhere\"\n}\n";
        let src = estate("  web.vm_name = \"acme-web\"\n") + "\nuse \"o.satz\"\nuse \"vm.satz\" named web\n";
        let fe = compile_with(&src, &[("vm.satz", VM), ("o.satz", other)]).unwrap();
        assert_eq!(attr(&bodies(&fe)["google_storage_bucket.web__data"], "location"), "europe-west3-a");
        assert_eq!(fe.tfvars["vm_zone"], serde_yaml::Value::from("elsewhere"));
    }

    #[test]
    fn two_resources_with_one_body_are_refused_naming_the_params_to_bind() {
        let src = estate("") + "\nuse \"vm.satz\" named web\nuse \"vm.satz\" named db\n";
        let e = refused(&src, &[("vm.satz", VM)]);
        says(&e, "the named uses `web` and `db` write the same resource");
        says(&e, "`google_storage_bucket.web__data`");
        says(&e, "Bind `web.vm_name`, `db.vm_name`");
        assert_eq!((e.file.as_str(), e.line), ("main.satz", 9));
        // a named use and the flat use
        let src = estate("") + "\nuse \"vm.satz\" named web\nuse \"vm.satz\"\n";
        says(&refused(&src, &[("vm.satz", VM)]), "the named use `web` and the estate write the same resource");
    }

    #[test]
    fn a_pack_must_say_multiple() {
        let single = VM.replace(" multiple", "");
        let e = refused(&(estate("") + "\nuse \"vm.satz\" named web\n"), &[("vm.satz", &single)]);
        says(&e, "use \"vm.satz\" named web: the pack `vm` does not say `multiple` in its header");
        says(&e, "the cloud names of what the pack creates come from its params");
        // a flat use of a `multiple` pack is allowed
        assert!(compile_with(&(estate("") + "\nuse \"vm.satz\"\n"), &[("vm.satz", VM)]).is_ok());
    }

    #[test]
    fn a_binding_names_a_param_the_pack_declares() {
        let e = refused(&(estate("  web.vm_nmae = \"x\"\n") + "\nuse \"vm.satz\" named web\n"), &[("vm.satz", VM)]);
        says(&e, "`web.vm_nmae` binds the param `vm_nmae` of the named use `web`, and vm.satz declares no param `vm_nmae` — its params are `vm_logs`, `vm_name`, `vm_size`, `vm_zone`");
        assert_eq!(e.line, 6);
    }

    #[test]
    fn what_a_named_use_refuses_in_its_pack() {
        let cases: [(&str, &str); 7] = [
            ("hcl {\n  locals {}\n}\n", "holds an `hcl` block"),
            ("action \"a\" {\n  reason = \"r\"\n  run    = \"x.sh\"\n  phase  = \"after-apply\"\n}\n", "holds an `action`"),
            ("inputs {\n  stage = \"dev\"\n}\n", "holds an `inputs` block"),
            ("interface \"i\" {\n  export \"x\" = vm_name\n}\n", "holds an `interface` block"),
            ("export \"x\" = vm_name\n", "holds an `export`"),
            ("entries vm_list {\n  key    = \"name\"\n  fields = [\"name\"]\n}\n", "holds an `entries` statement"),
            ("params {\n  vm_ack = false\n}\n\nnotice vm_ack {\n  text = \"t\"\n  run  = \"x\"\n}\n", "holds a `notice`"),
        ];
        for (extra, what) in cases {
            let pack = format!("{}\n{}", VM, extra);
            let pack = if extra.starts_with("entries") { pack.replace("  vm_logs = false\n", "  vm_logs = false\n  vm_list = []\n") } else { pack };
            let src = estate("  web.vm_name = \"acme-web\"\n") + "\nuse \"vm.satz\" named web\n";
            let e = match compile_with(&src, &[("vm.satz", &pack)]) {
                Ok(_) => panic!("{} must be refused", what),
                Err(e) => e,
            };
            says(&e, what);
            says(&e, "and a named use takes none");
            assert_eq!(e.file, "main.satz", "{}", e.msg);
        }
    }

    #[test]
    fn a_named_use_stands_in_the_estate_file_alone() {
        let inner = "use \"vm.satz\" named web\n";
        let e = refused(&(estate("") + "\nuse \"inner.satz\"\n"), &[("inner.satz", inner), ("vm.satz", VM)]);
        says(&e, "a named use stands at the top level of the estate file");
        let wraps = "pack w version \"1.0\" multiple\n\nuse \"inner.satz\"\n";
        let e = refused(&(estate("") + "\nuse \"w.satz\" named web\n"), &[("w.satz", wraps), ("inner.satz", inner), ("vm.satz", VM)]);
        says(&e, "holds `use \"vm.satz\" named web` — a named use stands in the estate file");
    }

    #[test]
    fn the_name_of_a_use_is_nothing_else_of_the_estate() {
        let e = refused(&(estate("  web = \"x\"\n  web.vm_name = \"acme-web\"\n") + "\nuse \"vm.satz\" named web\n"), &[("vm.satz", VM)]);
        says(&e, "`web` is also a param of the estate");
        let e = refused(&(estate("  web__x = \"x\"\n  web.vm_name = \"acme-web\"\n") + "\nuse \"vm.satz\" named web\n"), &[("vm.satz", VM)]);
        says(&e, "the param `web__x`");
        says(&e, "begins with `web__`");
        let own = "\ngoogle_storage_bucket {\n  web__own {\n    name = \"acme-own\"\n  }\n}\n";
        let e = refused(&(estate("  web.vm_name = \"acme-web\"\n") + "\nuse \"vm.satz\" named web\n" + own), &[("vm.satz", VM)]);
        says(&e, "`google_storage_bucket.web__own`");
        // without a use of that name, the same label is the estate's own
        assert!(compile_with(&(estate("") + own), &[]).is_ok());
    }

    #[test]
    fn a_contribution_to_a_list_of_a_named_use_is_refused() {
        let pack = "pack p version \"1.0\" multiple\n\nparams {\n  p_list = []\n  p_name = \"acme-p\"\n}\n\ngoogle_storage_bucket {\n  b {\n    name = p_name\n  }\n}\n";
        let adds = "pack a version \"1.0\"\n\nparams {\n  contributes_p_list = [\"x\"]\n}\n";
        let e = refused(&(estate("") + "\nuse \"p.satz\" named web\nuse \"a.satz\"\n"), &[("p.satz", pack), ("a.satz", adds)]);
        says(&e, "`p_list` is a list param of the named use `web`");
        says(&e, "not supported yet");
    }

    #[test]
    fn the_questions_of_the_estate_are_asked_once_per_named_use() {
        let src = estate("  web.vm_zone = \"europe-west3-b\"\n") + "\nuse \"vm.satz\" named web\nuse \"vm.satz\" named db\n";
        let load = |p: &str| if p == "vm.satz" { Ok(VM.to_string()) } else { Err(p.to_string()) };
        let (asked, _, env, _) = estate_questions_and_uses("main.satz", &src, &load).unwrap_or_else(|e| panic!("{}:{}: {}", e.file, e.line, e.msg));
        let subjects: Vec<(Option<&str>, &str)> = asked.iter().flat_map(|pq| pq.questions.iter().map(move |q| (pq.named.as_deref(), q.subject.as_str()))).collect();
        assert_eq!(subjects, [(Some("web"), "web.vm_zone"), (Some("db"), "db.vm_zone")]);
        assert!(asked.iter().all(|pq| pq.named_params.contains("vm_zone") && pq.named_params.contains("vm_name")));
        // each use's params, resolved inside the use, under the names the estate binds
        assert_eq!(env.get("web.vm_zone"), Some(&serde_yaml::Value::String("europe-west3-b".into())));
        assert_eq!(env.get("db.vm_zone"), Some(&serde_yaml::Value::String("europe-west3-a".into())));
        assert_eq!(env.get("db.vm_name"), Some(&serde_yaml::Value::String("acme-vm".into())));
        assert!(!env.contains_key("vm_zone"), "a param of a named use is no flat param");
        assert!(!estate_params("main.satz", &src, &load).unwrap().contains_key("vm_name"));
    }

    #[test]
    fn the_parser_refuses_what_a_named_use_cannot_be() {
        let cases: [(&str, &str); 9] = [
            ("estate e\nuse \"vm.satz\" as google_storage_bucket named web\n", "write one of the two"),
            ("estate e\ngoogle_storage_bucket {\n  use \"vm.satz\" named web\n}\n", "a named use stands at the top level of the estate file"),
            ("pack p\nuse \"vm.satz\" named web\n", "and this is a pack"),
            ("estate e\nuse \"vm.satz\" named Web\n", "a use's name is a lowercase letter"),
            ("estate e\nuse \"vm.satz\" named a__b\n", "holds no `__`"),
            ("estate e\nuse \"vm.satz\" named web\nuse \"vm.satz\" named web\n", "the name is given twice"),
            ("estate e\nparams {\n  web.vm_zone = \"x\"\n}\n", "no `use … named web` stands at the top level of this file — this file has none"),
            ("estate e multiple\n", "`multiple` is a word of a pack's header"),
            ("estate e\nlocals {\n  web.x = \"x\"\n}\nuse \"vm.satz\" named web\n", "a local value's name holds no dot"),
        ];
        for (src, what) in cases {
            match satz::parse(src) {
                Ok(_) => panic!("must be refused:\n{}", src),
                Err(e) => assert!(e.msg.contains(what), "`{}` must say `{}`: {}", src, what, e.msg),
            }
        }
        let f = satz::parse("pack p version \"1.0\" multiple\n").unwrap();
        assert!(f.multiple && f.is_pack);
        let f = satz::parse("pack p multiple\n").unwrap();
        assert!(f.multiple && f.version.is_none());
        assert!(satz::parse("pack p version \"1.0\" multiple twice\n").unwrap_err().msg.contains("[multiple]"));
        // the canonical form carries the word and the name, and nothing where neither is written
        let plain = satz::canonical(&satz::parse("pack p\nuse \"a.satz\" when x\n").unwrap());
        assert!(!plain.contains("multiple") && !plain.contains("named"), "{}", plain);
        let named = satz::canonical(&satz::parse("estate e\nuse \"a.satz\" named web when x\n").unwrap());
        assert!(named.contains("use(a.satz||x|named:web)"), "{}", named);
    }
}
