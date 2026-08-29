//! The HCL input shape of `satz import` (roadmap Phase 3): a directory of
//! `.tf` — hand-written, `gcloud … bulk-export --resource-format=terraform`,
//! `tofu plan -generate-config-out` — becomes a Satz estate.
//!
//! Two tiers, one pass. **Wrap**: a top-level block is carried verbatim as
//! `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
//! written, the compliance plane cannot see into it, and the fold cannot
//! compose it. **Translate** (3.1b, not here yet): a `resource` block of a
//! schema-known type whose values are literals becomes Satz. `terraform` and
//! `provider` blocks are dropped with a note: the emitter owns
//! `providers.tf`. Every block is accounted for in the report — an import may
//! be partial, never silent.
//!
//! This crate keeps `hcl-rs`/`hcl-edit` out of satz-core.

use hcl::edit::parser::parse_body;
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
    Wrapped,
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

/// Wrap every block of every file. `name` is the estate name.
pub fn wrap_all(inputs: &[Input], name: &str) -> Result<Imported, String> {
    let mut out = String::new();
    out.push_str("// Imported from existing Terraform — every block is carried verbatim inside\n");
    out.push_str("// `hcl trust` (it deploys as written; the compliance plane cannot see into it).\n");
    out.push_str("// Move blocks into Satz resources as you adopt them. `satz transpile`, then\n");
    out.push_str("// `tofu plan` against the source's state must show no changes.\n\n");
    out.push_str(&format!("estate {}\n\n", sanitize_name(name)));
    out.push_str("terraform {\n  backend {\n    local { path = \"terraform.tfstate\" }\n  }\n}\n\n");
    out.push_str("providers {\n  google {\n    alias = \"google\"\n  }\n  \"google-beta\" {\n    alias = \"google-beta\"\n  }\n}\n\n");
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
            let ident = s.as_block().map(|b| b.ident.to_string()).unwrap_or_default();
            if ident == "terraform" || ident == "provider" {
                rows.push(Row {
                    file: input.path.clone(),
                    line,
                    what,
                    action: Action::Dropped("the emitter writes providers.tf from the estate's `terraform` and `providers` blocks".into()),
                });
                continue;
            }
            out.push_str(&format!("hcl trust \"imported from {}:{}\" {{\n{}\n}}\n\n", input.path, line, indent(text)));
            rows.push(Row { file: input.path.clone(), line, what, action: Action::Wrapped });
        }
    }
    Ok(Imported { satz: out, rows })
}

fn describe(s: &hcl::edit::structure::Structure) -> String {
    match s {
        hcl::edit::structure::Structure::Attribute(a) => format!("{} = …", a.key),
        hcl::edit::structure::Structure::Block(b) => {
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
    let wrapped = rows.iter().filter(|r| r.action == Action::Wrapped).count();
    let dropped = rows.len() - wrapped;
    format!("import: {} block(s) wrapped verbatim, {} dropped (terraform/provider)", wrapped, dropped)
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

module "vpc" {
  source = "./modules/vpc"
  name   = "shared"
}

locals {
  env = "prod"
}
"#;

    #[test]
    fn every_block_is_wrapped_or_dropped_and_accounted_for() {
        let imported = wrap_all(&[Input { path: "main.tf".into(), text: TF.into() }], "acme").unwrap();
        let s = &imported.satz;
        assert!(s.starts_with("// Imported from existing Terraform"), "{}", s);
        assert!(s.contains("estate acme\n"), "{}", s);
        assert!(s.contains("hcl trust \"imported from main.tf:11\" {\n  resource \"google_folder\" \"workloads\" {"), "{}", s);
        assert!(s.contains("hcl trust \"imported from main.tf:16\" {\n  module \"vpc\" {"), "{}", s);
        assert!(s.contains("hcl trust \"imported from main.tf:21\" {\n  locals {"), "{}", s);
        assert!(!s.contains("required_providers"), "terraform block dropped:\n{}", s);
        assert!(!s.contains("provider \"google\""), "provider block dropped:\n{}", s);
        let whats: Vec<&str> = imported.rows.iter().map(|r| r.what.as_str()).collect();
        assert_eq!(whats, vec!["terraform", "provider \"google\"", "resource \"google_folder\" \"workloads\"", "module \"vpc\"", "locals"]);
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Wrapped).count(), 3);
        assert_eq!(summary(&imported.rows), "import: 3 block(s) wrapped verbatim, 2 dropped (terraform/provider)");
    }

    #[test]
    fn a_syntax_error_names_the_file() {
        let e = wrap_all(&[Input { path: "bad.tf".into(), text: "resource \"x\" {".into() }], "e").unwrap_err();
        assert!(e.starts_with("bad.tf: "), "{}", e);
    }
}
