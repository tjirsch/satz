//! The HCL input shape of `satz import` (roadmap Phase 3): a directory of
//! `.tf` — hand-written, `gcloud … bulk-export --resource-format=terraform`,
//! `tofu plan -generate-config-out` — becomes a Satz estate.
//!
//! Two tiers, one pass. **Translate**: a `resource` block of a schema-known
//! type whose values are literals becomes a Satz resource — attributes as
//! written, nested blocks as lists of objects, the label kept (so a wrapped
//! block that references it still resolves). Positional types are placed:
//! a folder whose `parent` is the organisation goes to the top, one whose
//! parent references another folder nests under it; a project nests under
//! the folder its `folder_id` references (or sits at the top when its
//! `org_id` is the organisation); a project's services, IAM grants and
//! project-scoped resources that reference it by `project` move inside it;
//! folder and organisation grants likewise. Those identity references are
//! the ONLY expressions a translated block may contain.
//! **Wrap**: everything else is carried verbatim as
//! `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
//! written, the compliance plane cannot see into it, the fold cannot compose
//! it — and the report says why. A block whose parent is wrapped is wrapped
//! too (closure by dependency). `terraform` and `provider` blocks are
//! dropped with a note: the emitter owns `providers.tf`. Every block is
//! accounted for — an import may be partial, never silent.
//!
//! This crate keeps `hcl-rs`/`hcl-edit` out of satz-core.

use std::collections::BTreeSet;

use hcl::edit::expr::{Expression, ObjectKey};
use hcl::edit::parser::parse_body;
use hcl::edit::structure::{Block, Body, Structure};
use hcl::edit::Span;

/// What happened to one top-level block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file: String,
    pub line: usize,
    /// `resource "google_folder" "x"`, `module "vpc"`, `locals`, …
    pub what: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// a Satz resource now
    Translated,
    /// verbatim inside `hcl trust`, and why
    Wrapped(String),
    /// `terraform` / `provider`: the emitter writes providers.tf itself
    Dropped(String),
}

#[derive(Debug)]
pub struct Imported {
    pub satz: String,
    pub rows: Vec<Row>,
}

/// One input file: its path (as shown in provenance) and text.
pub struct Input {
    pub path: String,
    pub text: String,
}

const META_BLOCKS: &[&str] = &["dynamic", "provisioner", "connection"];
const META_ATTRS: &[&str] = &["count", "for_each", "provider", "depends_on"];

/// Where a translated resource lives.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    /// top level (the organisation)
    Top,
    Folder(String),
    Project(String),
}

/// A resource block after classification.
struct Res {
    tf_type: String,
    label: String,
    file: String,
    line: usize,
    what: String,
    text: String,
    /// literal body (parent/scope attrs removed), when translatable
    body: Option<serde_yaml::Mapping>,
    place: Place,
    /// why it cannot translate on its own
    reason: Option<String>,
}

/// Import every file. `wrap_all` skips translation; `is_type` answers
/// whether a Terraform type is in the provider schema.
pub fn import(inputs: &[Input], name: &str, wrap_all: bool, is_type: &dyn Fn(&str) -> bool) -> Result<Imported, String> {
    let mut resources: Vec<Res> = Vec::new();
    let mut wrapped = String::new();
    let mut rows = Vec::new();
    let mut org_ids: BTreeSet<String> = BTreeSet::new();

    for input in inputs {
        let body = parse_body(&input.text).map_err(|e| format!("{}: {}", input.path, e))?;
        for s in body.iter() {
            let Some(span) = s.span() else {
                return Err(format!("{}: a structure without a span (not parsed from text?)", input.path));
            };
            let text = input.text[span.clone()].trim_end().to_string();
            let line = input.text[..span.start].matches('\n').count() + 1;
            let what = describe(s);
            let wrap_now = |reason: String, wrapped: &mut String, rows: &mut Vec<Row>| {
                wrapped.push_str(&wrap_block(&input.path, line, &text));
                rows.push(Row { file: input.path.clone(), line, what: what.clone(), action: Action::Wrapped(reason) });
            };
            let Some(block) = s.as_block() else {
                wrap_now("a top-level attribute".into(), &mut wrapped, &mut rows);
                continue;
            };
            let ident = block.ident.to_string();
            if ident == "terraform" || ident == "provider" {
                rows.push(Row {
                    file: input.path.clone(),
                    line,
                    what,
                    action: Action::Dropped("the emitter writes providers.tf from the estate's `terraform` and `providers` blocks".into()),
                });
                continue;
            }
            if wrap_all {
                wrap_now("--wrap-all".into(), &mut wrapped, &mut rows);
                continue;
            }
            if ident != "resource" {
                wrap_now(format!("`{}` blocks stay verbatim", ident), &mut wrapped, &mut rows);
                continue;
            }
            let (tf_type, label) = match block.labels.as_slice() {
                [t, l] => (t.as_str().to_string(), l.as_str().to_string()),
                _ => {
                    wrap_now("a resource block needs exactly two labels".into(), &mut wrapped, &mut rows);
                    continue;
                }
            };
            let classified = classify(&tf_type, &label, block, is_type, &mut org_ids);
            resources.push(Res {
                tf_type,
                label,
                file: input.path.clone(),
                line,
                what,
                text,
                body: classified.body,
                place: classified.place,
                reason: classified.reason,
            });
        }
    }

    // closure by dependency: a resource under a wrapped container is wrapped
    let mut wrapped_idx: BTreeSet<usize> = resources.iter().enumerate().filter(|(_, r)| r.reason.is_some()).map(|(i, _)| i).collect();
    loop {
        let mut changed = false;
        for i in 0..resources.len() {
            if wrapped_idx.contains(&i) {
                continue;
            }
            let parent = match &resources[i].place {
                Place::Top => None,
                Place::Folder(l) => resources.iter().position(|r| r.tf_type == "google_folder" && &r.label == l),
                Place::Project(l) => resources.iter().position(|r| r.tf_type == "google_project" && &r.label == l),
            };
            match (&resources[i].place, parent) {
                (Place::Top, _) => {}
                (place, None) => {
                    resources[i].reason = Some(format!("its parent {:?} is not among the imported resources", place_name(place)));
                    wrapped_idx.insert(i);
                    changed = true;
                }
                (_, Some(p)) if wrapped_idx.contains(&p) => {
                    resources[i].reason = Some(format!("its parent {} is wrapped", resources[p].what));
                    wrapped_idx.insert(i);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    // wrapped resources, in source order
    for (i, r) in resources.iter().enumerate() {
        if wrapped_idx.contains(&i) {
            wrapped.push_str(&wrap_block(&r.file, r.line, &r.text));
            rows.push(Row { file: r.file.clone(), line: r.line, what: r.what.clone(), action: Action::Wrapped(r.reason.clone().unwrap_or_default()) });
        } else {
            rows.push(Row { file: r.file.clone(), line: r.line, what: r.what.clone(), action: Action::Translated });
        }
    }
    rows.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));

    // the tree
    let translated: Vec<&Res> = resources.iter().enumerate().filter(|(i, _)| !wrapped_idx.contains(i)).map(|(_, r)| r).collect();
    let mut top = serde_yaml::Mapping::new();
    top.insert("terraform".into(), serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n").map_err(|e| e.to_string())?);
    top.insert(
        "providers".into(),
        serde_yaml::from_str("google:\n  alias: google\ngoogle-beta:\n  alias: google-beta\n").map_err(|e| e.to_string())?,
    );
    for r in &translated {
        if r.place == Place::Top {
            place_into(&mut top, r, &translated);
        }
    }

    let params: Vec<(String, String)> = match org_ids.iter().next() {
        Some(org) => vec![("customer_organization_id".to_string(), format!("\"{}\"", org))],
        None => Vec::new(),
    };
    let mut header = vec![
        "Imported from existing Terraform. Resource blocks with literal values are Satz".to_string(),
        "resources, placed by the folder/project they reference; everything else is carried".to_string(),
        "verbatim inside `hcl trust` (it deploys as written; the compliance plane cannot see".to_string(),
        "into it) — the import report says why. `satz transpile`, then `tofu plan` against the".to_string(),
        "source's state must show no changes; `satz adopt` resolves the import ids afterwards.".to_string(),
    ];
    if params.is_empty() {
        header.push("No organisation id was found among the literals — add `customer_organization_id` to `params` by hand.".to_string());
    }
    let mut satz = satz_core::migrate::convert_value(&top, "estate", &sanitize_name(name), &params, &header).map_err(|e| e.to_string())?;
    if !satz.ends_with("\n\n") {
        satz.push('\n');
    }
    satz.push_str(&wrapped);
    Ok(Imported { satz, rows })
}

fn place_name(p: &Place) -> String {
    match p {
        Place::Top => "the organisation".into(),
        Place::Folder(l) => format!("google_folder.{}", l),
        Place::Project(l) => format!("google_project.{}", l),
    }
}

/// Put `r` into `into` (a folder body, a project body, or the top level),
/// with its own children placed under it.
fn place_into(into: &mut serde_yaml::Mapping, r: &Res, all: &[&Res]) {
    let mut body = r.body.clone().unwrap_or_default();
    let key = |s: &str| serde_yaml::Value::String(s.to_string());
    match r.tf_type.as_str() {
        "google_folder" => {
            for c in all.iter().filter(|c| c.place == Place::Folder(r.label.clone())) {
                place_into(&mut body, c, all);
            }
            let m = into.entry(key("google_folder")).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
        "google_project" => {
            for c in all.iter().filter(|c| c.place == Place::Project(r.label.clone())) {
                place_into(&mut body, c, all);
            }
            let m = into.entry(key("google_project")).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
        "google_project_service" => {
            // `classify` guarantees a literal `service`
            let svc = body.get("service").and_then(|v| v.as_str()).map(String::from).expect("classified project_service has a service");
            let list = into.entry(key("project_service")).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
            if let serde_yaml::Value::Sequence(seq) = list {
                seq.push(serde_yaml::Value::String(svc));
            }
        }
        t if t.ends_with("_iam_member") => {
            // `classify` guarantees literal `member` and `role`; a `condition`
            // rides along in the object form — dropping it would widen the grant
            let member = body.get("member").and_then(|v| v.as_str()).map(String::from).expect("classified iam member has a member");
            let role = body.get("role").and_then(|v| v.as_str()).map(String::from).expect("classified iam member has a role");
            let entry = match body.get("condition") {
                Some(cond) => {
                    let mut o = serde_yaml::Mapping::new();
                    o.insert(key("role"), serde_yaml::Value::String(role));
                    // an HCL block is a list of one object; the grant form takes the object
                    let cond = match cond {
                        serde_yaml::Value::Sequence(items) if items.len() == 1 => items[0].clone(),
                        other => other.clone(),
                    };
                    o.insert(key("condition"), cond);
                    serde_yaml::Value::Mapping(o)
                }
                None => serde_yaml::Value::String(role),
            };
            let m = into.entry(key(t)).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                let roles = m.entry(key(&member)).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
                if let serde_yaml::Value::Sequence(seq) = roles {
                    if !seq.contains(&entry) {
                        seq.push(entry);
                    }
                }
            }
        }
        t => {
            let m = into.entry(key(t)).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
    }
}

struct Classified {
    body: Option<serde_yaml::Mapping>,
    place: Place,
    reason: Option<String>,
}

fn wrapped(reason: String) -> Classified {
    Classified { body: None, place: Place::Top, reason: Some(reason) }
}

/// Classify one resource block: translatable (with its body and place) or
/// the reason it is not.
fn classify(tf_type: &str, label: &str, block: &Block, is_type: &dyn Fn(&str) -> bool, org_ids: &mut BTreeSet<String>) -> Classified {
    if !is_type(tf_type) {
        return wrapped(format!("`{}` is not in the provider schema", tf_type));
    }
    if !is_identifier(label) {
        return wrapped(format!("label `{}` is not a Satz identifier (a rename would break references)", label));
    }
    if matches!(tf_type, "google_cloud_identity_group" | "google_cloud_identity_group_membership")
        || tf_type.ends_with("_iam_binding")
        || tf_type.ends_with("_iam_policy")
        || tf_type.ends_with("_iam_audit_config")
        || tf_type == "google_storage_bucket_iam_member"
        || tf_type == "google_billing_account_iam_member"
    {
        return wrapped(format!("`{}` has a special Satz form not derived from HCL yet", tf_type));
    }
    // the attributes Satz's special forms are keyed by must be literal strings
    let literal_str = |k: &str| {
        block.body.iter().find_map(|s| match s {
            Structure::Attribute(a) if a.key.to_string() == k => literal(&a.value).and_then(|v| v.as_str().map(String::from)),
            _ => None,
        })
    };
    if tf_type.ends_with("_iam_member") {
        for k in ["member", "role"] {
            if literal_str(k).is_none() {
                return wrapped(format!("`{}` is not a literal string", k));
            }
        }
        // anything beyond member/role/condition and the scope attribute is not
        // part of Satz's grant form — say so rather than drop it
        for s in block.body.iter() {
            let k = match s {
                Structure::Attribute(a) => a.key.to_string(),
                Structure::Block(b) => b.ident.to_string(),
            };
            if !matches!(k.as_str(), "member" | "role" | "condition") && !is_scope_attr(tf_type, &k) {
                return wrapped(format!("`{}` has no place in a Satz grant", k));
            }
        }
    }
    if tf_type == "google_project_service" && literal_str("service").is_none() {
        return wrapped("`service` is not a literal string".into());
    }
    // meta-arguments and non-literal values
    for s in block.body.iter() {
        match s {
            Structure::Attribute(a) => {
                let k = a.key.to_string();
                if META_ATTRS.contains(&k.as_str()) {
                    return wrapped(format!("uses `{}`", k));
                }
                if is_scope_attr(tf_type, &k) {
                    continue; // judged below
                }
                if literal(&a.value).is_none() {
                    return wrapped(format!("`{}` is an expression, not a literal", k));
                }
            }
            Structure::Block(b) => {
                let id = b.ident.to_string();
                if META_BLOCKS.contains(&id.as_str()) {
                    return wrapped(format!("uses `{}`", id));
                }
                if id == "lifecycle" {
                    if lifecycle_body(&b.body).is_none() {
                        return wrapped("a `lifecycle` block satz cannot express".into());
                    }
                    continue;
                }
                if !b.labels.is_empty() {
                    return wrapped(format!("nested block `{}` carries labels", id));
                }
                if literal_body(&b.body).is_none() {
                    return wrapped(format!("nested block `{}` holds an expression", id));
                }
            }
        }
    }
    let mut body = literal_body_skipping(&block.body, |k| is_scope_attr(tf_type, k)).expect("checked literal");
    // the scope attribute decides the place
    let scope = scope_attr_value(tf_type, block);
    let place = match tf_type {
        "google_folder" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            None => return wrapped("a folder needs a literal `parent`".into()),
            Some(other) => return wrapped(format!("`parent` = {} cannot be placed", other.describe())),
        },
        "google_project" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            None => Place::Top,
            Some(other) => return wrapped(format!("`folder_id`/`org_id` = {} cannot be placed (a folder number is not a folder in this input)", other.describe())),
        },
        "google_organization_iam_member" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            _ => return wrapped("an organisation grant needs a literal `org_id`".into()),
        },
        "google_folder_iam_member" => match scope {
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            _ => return wrapped("a folder grant must reference a folder in this input".into()),
        },
        "google_project_iam_member" | "google_project_service" => match scope {
            Some(ScopeRef::Project(p)) => Place::Project(p),
            _ => return wrapped(format!("`{}` must reference a project in this input", tf_type)),
        },
        _ => match scope {
            Some(ScopeRef::Project(p)) => Place::Project(p),
            Some(ScopeRef::Literal(v)) => {
                // a project-scoped resource naming its project by id stays where
                // it is, with the literal — Satz is position-independent there
                body.insert("project".into(), serde_yaml::Value::String(v));
                Place::Top
            }
            _ => Place::Top,
        },
    };
    if tf_type == "google_org_policy_policy" {
        if let Some(p) = body.get("parent").and_then(|v| v.as_str()).and_then(|p| p.strip_prefix("organizations/")) {
            org_ids.insert(p.to_string());
        }
    }
    Classified { body: Some(body), place, reason: None }
}

enum ScopeRef {
    Org(String),
    Folder(String),
    Project(String),
    Literal(String),
    Other(String),
}

impl ScopeRef {
    fn describe(&self) -> String {
        match self {
            ScopeRef::Org(o) => format!("organizations/{}", o),
            ScopeRef::Folder(f) => format!("google_folder.{}", f),
            ScopeRef::Project(p) => format!("google_project.{}", p),
            ScopeRef::Literal(v) | ScopeRef::Other(v) => v.clone(),
        }
    }
}

/// The attribute that carries a resource's place.
fn is_scope_attr(tf_type: &str, key: &str) -> bool {
    match tf_type {
        "google_folder" => key == "parent",
        "google_project" => key == "folder_id" || key == "org_id",
        "google_organization_iam_member" => key == "org_id",
        "google_folder_iam_member" => key == "folder",
        "google_org_policy_policy" => false,
        _ => key == "project",
    }
}

/// The scope attribute's meaning: the organisation, a folder or project of
/// this input (by identity reference), a literal, or something else.
fn scope_attr_value(tf_type: &str, block: &Block) -> Option<ScopeRef> {
    for s in block.body.iter() {
        let Structure::Attribute(a) = s else { continue };
        let k = a.key.to_string();
        if !is_scope_attr(tf_type, &k) {
            continue;
        }
        return Some(match &a.value {
            Expression::String(s) => {
                let v = s.value().to_string();
                if let Some(o) = v.strip_prefix("organizations/") {
                    ScopeRef::Org(o.to_string())
                } else if k == "org_id" && v.chars().all(|c| c.is_ascii_digit()) {
                    ScopeRef::Org(v)
                } else if v.starts_with("folders/") || (k == "folder_id" && v.chars().all(|c| c.is_ascii_digit())) {
                    ScopeRef::Other(v)
                } else {
                    ScopeRef::Literal(v)
                }
            }
            Expression::Traversal(_) => {
                let text = a.value.to_string().trim().to_string();
                let parts: Vec<&str> = text.split('.').collect();
                match parts.as_slice() {
                    ["google_folder", l, "name" | "id" | "folder_id"] => ScopeRef::Folder((*l).to_string()),
                    ["google_project", l, "project_id" | "id" | "name"] => ScopeRef::Project((*l).to_string()),
                    _ => ScopeRef::Other(text),
                }
            }
            e => ScopeRef::Other(e.to_string().trim().to_string()),
        });
    }
    None
}

/// A body whose values are all literals, as a mapping: attributes as
/// written, nested blocks as lists of objects (repeated blocks append),
/// `lifecycle` as a mapping.
fn literal_body(body: &Body) -> Option<serde_yaml::Mapping> {
    literal_body_skipping(body, |_| false)
}

fn literal_body_skipping(body: &Body, skip: impl Fn(&str) -> bool) -> Option<serde_yaml::Mapping> {
    let mut m = serde_yaml::Mapping::new();
    for s in body.iter() {
        match s {
            Structure::Attribute(a) => {
                let k = a.key.to_string();
                if skip(&k) {
                    continue;
                }
                m.insert(serde_yaml::Value::String(k), literal(&a.value)?);
            }
            Structure::Block(b) => {
                let id = b.ident.to_string();
                if id == "lifecycle" {
                    m.insert("lifecycle".into(), serde_yaml::Value::Mapping(lifecycle_body(&b.body)?));
                    continue;
                }
                if !b.labels.is_empty() {
                    return None;
                }
                let inner = serde_yaml::Value::Mapping(literal_body(&b.body)?);
                let key = serde_yaml::Value::String(id);
                match m.get_mut(&key) {
                    Some(serde_yaml::Value::Sequence(seq)) => seq.push(inner),
                    _ => {
                        m.insert(key, serde_yaml::Value::Sequence(vec![inner]));
                    }
                }
            }
        }
    }
    Some(m)
}

/// `lifecycle { ignore_changes = [a, b] prevent_destroy = true }` — the
/// list items are bare traversals in HCL and strings in Satz.
fn lifecycle_body(body: &Body) -> Option<serde_yaml::Mapping> {
    let mut m = serde_yaml::Mapping::new();
    for s in body.iter() {
        let a = s.as_attribute()?;
        let k = a.key.to_string();
        let v = match (k.as_str(), &a.value) {
            ("ignore_changes" | "replace_triggered_by", Expression::Array(arr)) => serde_yaml::Value::Sequence(
                arr.iter().map(|e| serde_yaml::Value::String(e.to_string().trim().to_string())).collect(),
            ),
            ("ignore_changes", Expression::Variable(v)) => serde_yaml::Value::String(v.to_string()),
            (_, e) => literal(e)?,
        };
        m.insert(serde_yaml::Value::String(k), v);
    }
    Some(m)
}

/// A literal expression as a YAML value; `None` for anything that needs
/// evaluation (templates, references, functions, operators).
fn literal(e: &Expression) -> Option<serde_yaml::Value> {
    Some(match e {
        Expression::Null(_) => serde_yaml::Value::Null,
        Expression::Bool(b) => serde_yaml::Value::Bool(*b.value()),
        Expression::Number(n) => match (n.value().as_i64(), n.value().as_f64()) {
            (Some(i), _) => serde_yaml::Value::Number(i.into()),
            (None, Some(f)) => serde_yaml::Value::Number(f.into()),
            _ => return None,
        },
        Expression::String(s) => serde_yaml::Value::String(s.value().to_string()),
        Expression::Array(a) => serde_yaml::Value::Sequence(a.iter().map(literal).collect::<Option<Vec<_>>>()?),
        Expression::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, v) in o.iter() {
                let key = match k {
                    ObjectKey::Ident(i) => i.to_string(),
                    ObjectKey::Expression(Expression::String(s)) => s.value().to_string(),
                    _ => return None,
                };
                m.insert(serde_yaml::Value::String(key), literal(v.expr())?);
            }
            serde_yaml::Value::Mapping(m)
        }
        _ => return None,
    })
}

fn wrap_block(path: &str, line: usize, text: &str) -> String {
    format!("hcl trust \"imported from {}:{}\" {{\n{}\n}}\n\n", path, line, indent(text))
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_lowercase())
        && chars.all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn describe(s: &Structure) -> String {
    match s {
        Structure::Attribute(a) => format!("{} = …", a.key),
        Structure::Block(b) => {
            let labels: Vec<String> = b.labels.iter().map(|l| format!("\"{}\"", l.as_str())).collect();
            if labels.is_empty() {
                b.ident.to_string()
            } else {
                format!("{} {}", b.ident, labels.join(" "))
            }
        }
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("  {}", l) }).collect::<Vec<_>>().join("\n")
}

fn sanitize_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        "imported_hcl".to_string()
    } else {
        out
    }
}

/// Human summary of the rows.
pub fn summary(rows: &[Row]) -> String {
    let translated = rows.iter().filter(|r| r.action == Action::Translated).count();
    let wrapped = rows.iter().filter(|r| matches!(r.action, Action::Wrapped(_))).count();
    let dropped = rows.len() - translated - wrapped;
    format!("import: {} block(s) translated to Satz, {} wrapped verbatim, {} dropped (terraform/provider)", translated, wrapped, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TF: &str = r#"terraform {
  required_providers {
    google = { source = "hashicorp/google" }
  }
}

provider "google" {
  project = "acme-infra-001"
}

resource "google_folder" "workloads" {
  display_name = "Workloads"
  parent       = "organizations/123456789012"
}

resource "google_folder" "team" {
  display_name = "Team"
  parent       = google_folder.workloads.name
}

resource "google_project" "infra" {
  name       = "Infra"
  project_id = "acme-infra-001"
  folder_id  = google_folder.team.name
}

resource "google_project" "orphan" {
  name       = "Orphan"
  project_id = "acme-orphan-001"
  folder_id  = "folders/999"
}

resource "google_project_service" "infra_iam" {
  project = google_project.infra.project_id
  service = "iam.googleapis.com"
}

resource "google_project_iam_member" "infra_viewer" {
  project = google_project.infra.project_id
  role    = "roles/viewer"
  member  = "group:auditors@example.com"
}

resource "google_folder_iam_member" "team_viewer" {
  folder = google_folder.team.name
  role   = "roles/viewer"
  member = "group:team@example.com"
}

resource "google_organization_iam_member" "org_admin" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.organizationAdmin"
  member = "group:admins@example.com"
}

resource "google_storage_bucket" "logs" {
  name     = "acme-logs-001"
  location = "EU"
  project  = google_project.infra.project_id
  lifecycle_rule {
    action { type = "Delete" }
    condition { age = 30 }
  }
}

resource "google_storage_bucket" "elsewhere" {
  name     = "acme-elsewhere"
  location = "EU"
  project  = "some-other-project"
}

resource "google_storage_bucket" "orphaned" {
  name     = "acme-orphaned"
  location = "EU"
  project  = google_project.orphan.project_id
}

resource "google_org_policy_policy" "skip_default" {
  name   = "organizations/123456789012/policies/compute.skipDefaultNetworkCreation"
  parent = "organizations/123456789012"
  spec {
    rules { enforce = "TRUE" }
  }
}

module "vpc" {
  source = "./modules/vpc"
}
"#;

    fn known(t: &str) -> bool {
        matches!(
            t,
            "google_folder"
                | "google_project"
                | "google_project_service"
                | "google_project_iam_member"
                | "google_folder_iam_member"
                | "google_organization_iam_member"
                | "google_storage_bucket"
                | "google_org_policy_policy"
        )
    }

    #[test]
    fn resources_are_placed_by_the_folder_and_project_they_reference() {
        let imported = import(&[Input { path: "main.tf".into(), text: TF.into() }], "acme", false, &known).unwrap();
        let s = &imported.satz;
        assert!(s.contains("customer_organization_id = \"123456789012\""), "{}", s);
        // team nests under workloads, infra under team, the bucket/service/grant under infra
        let i_workloads = s.find("  workloads {").expect("workloads");
        let i_team = s.find("    team {").expect("team nested");
        let i_infra = s.find("      infra {").expect("infra nested under team");
        assert!(i_workloads < i_team && i_team < i_infra, "{}", s);
        let c: String = s.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(c.contains("project_service=[\"iam.googleapis.com\",]"), "{}", s);
        assert!(c.contains("google_project_iam_member{\"group:auditors@example.com\"=[\"roles/viewer\",]}"), "{}", s);
        assert!(c.contains("google_folder_iam_member{\"group:team@example.com\"=[\"roles/viewer\",]}"), "{}", s);
        assert!(c.contains("google_organization_iam_member{\"group:admins@example.com\"=[\"roles/resourcemanager.organizationAdmin\",]}"), "{}", s);
        assert!(c.contains("google_storage_bucket{logs{name=\"acme-logs-001\"location=\"EU\"lifecycle_rule=[{action=[{type=\"Delete\"},]condition=[{age=30},]},]}}"), "bucket inside the project:\n{}", s);
        assert!(!c.contains("project=\"acme-infra-001\""), "the project reference became placement, not an attribute:\n{}", s);
        assert!(c.contains("google_storage_bucket{elsewhere{name=\"acme-elsewhere\"location=\"EU\"project=\"some-other-project\"}}"), "a literal project stays explicit at the top:\n{}", s);
        assert!(c.contains("google_org_policy_policy{skip_default{"), "{}", s);
        // wrapped, with reasons
        let by_what: std::collections::BTreeMap<&str, &Action> = imported.rows.iter().map(|r| (r.what.as_str(), &r.action)).collect();
        assert!(matches!(by_what["resource \"google_project\" \"orphan\""], Action::Wrapped(r) if r.contains("folder number")), "{:?}", by_what["resource \"google_project\" \"orphan\""]);
        assert!(matches!(by_what["resource \"google_storage_bucket\" \"orphaned\""], Action::Wrapped(r) if r.contains("is wrapped")), "closure by dependency: {:?}", by_what["resource \"google_storage_bucket\" \"orphaned\""]);
        assert!(matches!(by_what["module \"vpc\""], Action::Wrapped(_)));
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 10);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn wrap_all_wraps_everything_that_is_not_dropped() {
        let imported = import(&[Input { path: "main.tf".into(), text: TF.into() }], "acme", true, &known).unwrap();
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 0);
        assert_eq!(imported.rows.iter().filter(|r| matches!(r.action, Action::Wrapped(_))).count(), 13);
        satz_core::satz::parse(&imported.satz).unwrap();
    }

    #[test]
    fn a_syntax_error_names_the_file() {
        let e = import(&[Input { path: "bad.tf".into(), text: "resource \"x\" {".into() }], "e", true, &known).unwrap_err();
        assert!(e.starts_with("bad.tf: "), "{}", e);
    }

    /// The review found a conditional binding imported as an unconditional one,
    /// reported Translated — a silent privilege widening.
    #[test]
    fn a_conditional_grant_keeps_its_condition_and_a_partial_one_is_wrapped() {
        let tf = r#"
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_project_iam_member" "cond" {
  project = google_project.p.project_id
  role    = "roles/viewer"
  member  = "group:auditors@example.com"
  condition {
    title      = "office-hours"
    expression = "request.time.getHours(\"Europe/Berlin\") >= 8"
  }
}
resource "google_project_iam_member" "no_member" {
  project = google_project.p.project_id
  role    = "roles/viewer"
}
"#;
        let imported = import(&[Input { path: "main.tf".into(), text: tf.into() }], "acme", false, &known).unwrap();
        let c: String = imported.satz.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(c.contains("\"group:auditors@example.com\"=[{role=\"roles/viewer\"condition{title=\"office-hours\""), "{:?}
{}", imported.rows, imported.satz);
        let row = imported.rows.iter().find(|r| r.what.contains("no_member")).unwrap();
        assert!(matches!(&row.action, Action::Wrapped(r) if r.contains("`member`")), "{:?}", row.action);
    }
}
