//! The HCL input shape of `satz import` (roadmap Phase 3): a directory of
//! `.tf` — hand-written, `gcloud … bulk-export --resource-format=terraform`,
//! `tofu plan -generate-config-out` — becomes a Satz estate.
//!
//! Two tiers, one pass. **Translate**: a `resource` block of a schema-known,
//! non-positional type whose every value is a literal becomes a Satz
//! resource — attributes as written, nested blocks as lists of objects, the
//! label kept (so a wrapped block that references it still resolves).
//! **Wrap**: everything else is carried verbatim as
//! `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
//! written, the compliance plane cannot see into it, the fold cannot compose
//! it — and the report says why. `terraform` and `provider` blocks are
//! dropped with a note: the emitter owns `providers.tf`. Every block is
//! accounted for — an import may be partial, never silent.
//!
//! Positional types — folders, projects, groups, memberships, services, IAM
//! grants — are wrapped for now: their Satz form is their PLACE in the tree,
//! which the flat resource list of a `.tf` does not carry (3.1c).
//!
//! This crate keeps `hcl-rs`/`hcl-edit` out of satz-core.

use std::collections::BTreeMap;

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

/// Types whose Satz form is positional (their place in the tree) or a
/// special arm of the emitter — wrapped until 3.1c recovers nesting.
fn positional(tf_type: &str) -> bool {
    matches!(
        tf_type,
        "google_folder" | "google_project" | "google_project_service" | "google_cloud_identity_group" | "google_cloud_identity_group_membership"
    ) || tf_type.ends_with("_iam_member")
        || tf_type.ends_with("_iam_binding")
        || tf_type.ends_with("_iam_policy")
        || tf_type.ends_with("_iam_audit_config")
}

const META_BLOCKS: &[&str] = &["dynamic", "provisioner", "connection"];
const META_ATTRS: &[&str] = &["count", "for_each", "provider", "depends_on"];

/// Import every file. `wrap_all` skips translation; `is_type` answers
/// whether a Terraform type is in the provider schema.
pub fn import(inputs: &[Input], name: &str, wrap_all: bool, is_type: &dyn Fn(&str) -> bool) -> Result<Imported, String> {
    let mut translated: BTreeMap<String, serde_yaml::Mapping> = BTreeMap::new();
    let mut wrapped = String::new();
    let mut rows = Vec::new();
    for input in inputs {
        let body = parse_body(&input.text).map_err(|e| format!("{}: {}", input.path, e))?;
        for s in body.iter() {
            let Some(span) = s.span() else {
                return Err(format!("{}: a structure without a span (not parsed from text?)", input.path));
            };
            let text = input.text[span.clone()].trim_end();
            let line = input.text[..span.start].matches('\n').count() + 1;
            let what = describe(s);
            let mut wrap = |reason: String, rows: &mut Vec<Row>| {
                wrapped.push_str(&format!("hcl trust \"imported from {}:{}\" {{\n{}\n}}\n\n", input.path, line, indent(text)));
                rows.push(Row { file: input.path.clone(), line, what: what.clone(), action: Action::Wrapped(reason) });
            };
            let Some(block) = s.as_block() else {
                wrap("a top-level attribute".into(), &mut rows);
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
                wrap("--wrap-all".into(), &mut rows);
                continue;
            }
            if ident != "resource" {
                wrap(format!("`{}` blocks stay verbatim", ident), &mut rows);
                continue;
            }
            let (tf_type, label) = match block.labels.as_slice() {
                [t, l] => (t.as_str().to_string(), l.as_str().to_string()),
                _ => {
                    wrap("a resource block needs exactly two labels".into(), &mut rows);
                    continue;
                }
            };
            match translate_reason(&tf_type, &label, block, is_type) {
                Some(reason) => wrap(reason, &mut rows),
                None => {
                    let body = literal_body(&block.body).expect("checked literal");
                    translated.entry(tf_type).or_default().insert(serde_yaml::Value::String(label), serde_yaml::Value::Mapping(body));
                    rows.push(Row { file: input.path.clone(), line, what, action: Action::Translated });
                }
            }
        }
    }

    let mut top = serde_yaml::Mapping::new();
    top.insert("terraform".into(), serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n").map_err(|e| e.to_string())?);
    top.insert(
        "providers".into(),
        serde_yaml::from_str("google:\n  alias: google\ngoogle-beta:\n  alias: google-beta\n").map_err(|e| e.to_string())?,
    );
    for (t, entries) in translated {
        top.insert(serde_yaml::Value::String(t), serde_yaml::Value::Mapping(entries));
    }
    let header = vec![
        "Imported from existing Terraform. Resource blocks with literal values are Satz".to_string(),
        "resources; everything else is carried verbatim inside `hcl trust` (it deploys as".to_string(),
        "written; the compliance plane cannot see into it) — the import report says why.".to_string(),
        "`satz transpile`, then `tofu plan` against the source's state must show no changes;".to_string(),
        "`satz adopt` resolves the import ids afterwards.".to_string(),
    ];
    let mut satz = satz_core::migrate::convert_value(&top, "estate", &sanitize_name(name), &[], &header).map_err(|e| e.to_string())?;
    if !satz.ends_with("\n\n") {
        satz.push('\n');
    }
    satz.push_str(&wrapped);
    Ok(Imported { satz, rows })
}

/// Why this resource block cannot be translated — `None` when it can.
fn translate_reason(tf_type: &str, label: &str, block: &Block, is_type: &dyn Fn(&str) -> bool) -> Option<String> {
    if !is_type(tf_type) {
        return Some(format!("`{}` is not in the provider schema", tf_type));
    }
    if positional(tf_type) {
        return Some(format!("`{}` is positional in Satz (its place in the tree) — translated with nesting in 3.1c", tf_type));
    }
    if !is_identifier(label) {
        return Some(format!("label `{}` is not a Satz identifier (a rename would break references)", label));
    }
    for s in block.body.iter() {
        match s {
            Structure::Attribute(a) => {
                let k = a.key.to_string();
                if META_ATTRS.contains(&k.as_str()) {
                    return Some(format!("uses `{}`", k));
                }
                if literal(&a.value).is_none() {
                    return Some(format!("`{}` is an expression, not a literal", k));
                }
            }
            Structure::Block(b) => {
                let id = b.ident.to_string();
                if META_BLOCKS.contains(&id.as_str()) {
                    return Some(format!("uses `{}`", id));
                }
                if id == "lifecycle" {
                    if lifecycle_body(&b.body).is_none() {
                        return Some("a `lifecycle` block satz cannot express".into());
                    }
                    continue;
                }
                if !b.labels.is_empty() {
                    return Some(format!("nested block `{}` carries labels", id));
                }
                if literal_body(&b.body).is_none() {
                    return Some(format!("nested block `{}` holds an expression", id));
                }
            }
        }
    }
    None
}

/// A body whose values are all literals, as a mapping: attributes as
/// written, nested blocks as lists of objects (repeated blocks append),
/// `lifecycle` as a mapping.
fn literal_body(body: &Body) -> Option<serde_yaml::Mapping> {
    let mut m = serde_yaml::Mapping::new();
    for s in body.iter() {
        match s {
            Structure::Attribute(a) => {
                m.insert(serde_yaml::Value::String(a.key.to_string()), literal(&a.value)?);
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

resource "google_storage_bucket" "logs" {
  name     = "acme-logs-001"
  location = "EU"
  project  = "acme-infra-001"
  labels   = { env = "prod" }

  uniform_bucket_level_access = true

  lifecycle_rule {
    action {
      type = "Delete"
    }
    condition {
      age = 30
    }
  }
  lifecycle_rule {
    action {
      type = "SetStorageClass"
      storage_class = "NEARLINE"
    }
    condition {
      age = 7
    }
  }
  lifecycle {
    ignore_changes = [labels]
  }
}

resource "google_storage_bucket" "derived" {
  name     = "${google_storage_bucket.logs.name}-copy"
  location = google_storage_bucket.logs.location
}

resource "google_storage_bucket" "with-dash" {
  name = "x"
}

resource "google_widget" "w" {
  name = "w"
}

module "vpc" {
  source = "./modules/vpc"
}
"#;

    fn known(t: &str) -> bool {
        matches!(t, "google_folder" | "google_storage_bucket")
    }

    #[test]
    fn literal_resources_translate_and_the_rest_is_wrapped_with_a_reason() {
        let imported = import(&[Input { path: "main.tf".into(), text: TF.into() }], "acme", false, &known).unwrap();
        let s = &imported.satz;
        assert!(s.contains("estate acme\n"), "{}", s);
        assert!(s.contains("google_storage_bucket {\n  logs {\n"), "{}", s);
        assert!(s.contains("uniform_bucket_level_access = true"), "{}", s);
        assert!(s.contains("lifecycle_rule = [\n"), "repeated blocks become a list:\n{}", s);
        assert!(s.contains("storage_class = \"NEARLINE\""), "{}", s);
        assert!(s.contains("ignore_changes = [\n      \"labels\",") || s.contains("ignore_changes = [\"labels\"]") || s.contains("ignore_changes = [\n        \"labels\","), "{}", s);
        assert!(s.contains("hcl trust \"imported from main.tf:11\" {\n  resource \"google_folder\""), "the folder is positional → wrapped:\n{}", s);
        assert!(s.contains("hcl trust \"imported from main.tf:46\""), "the derived bucket references another → wrapped:\n{}", s);
        let by_what: BTreeMap<&str, &Action> = imported.rows.iter().map(|r| (r.what.as_str(), &r.action)).collect();
        assert_eq!(by_what["resource \"google_storage_bucket\" \"logs\""], &Action::Translated);
        assert!(matches!(by_what["resource \"google_folder\" \"workloads\""], Action::Wrapped(r) if r.contains("positional")));
        assert!(matches!(by_what["resource \"google_storage_bucket\" \"derived\""], Action::Wrapped(r) if r.contains("expression")));
        assert!(matches!(by_what["resource \"google_storage_bucket\" \"with-dash\""], Action::Wrapped(r) if r.contains("identifier")));
        assert!(matches!(by_what["resource \"google_widget\" \"w\""], Action::Wrapped(r) if r.contains("provider schema")));
        assert!(matches!(by_what["module \"vpc\""], Action::Wrapped(r) if r.contains("verbatim")));
        assert!(matches!(by_what["terraform"], Action::Dropped(_)));
        assert_eq!(summary(&imported.rows), "import: 1 block(s) translated to Satz, 5 wrapped verbatim, 2 dropped (terraform/provider)");
        // the estate parses
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn wrap_all_wraps_everything_that_is_not_dropped() {
        let imported = import(&[Input { path: "main.tf".into(), text: TF.into() }], "acme", true, &known).unwrap();
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 0);
        assert_eq!(imported.rows.iter().filter(|r| matches!(r.action, Action::Wrapped(_))).count(), 6);
        satz_core::satz::parse(&imported.satz).unwrap();
    }

    #[test]
    fn a_syntax_error_names_the_file() {
        let e = import(&[Input { path: "bad.tf".into(), text: "resource \"x\" {".into() }], "e", true, &known).unwrap_err();
        assert!(e.starts_with("bad.tf: "), "{}", e);
    }
}
