//! `satz add-entry`: one entry added to a list a pack declares with `entries`. The
//! declaration says what an entry is — its key, its fields, a pattern per field, the value a
//! field takes when left out — and that is all this writer knows: it adds a delegation, a
//! delegated project, a subnet or a budget alike, and a pack that declares a new kind of
//! entry needs nothing here (ADR 0093). The entry is written into the estate's own
//! `params {}`, text surgery as every other writer of a param, and the estate is transpiled
//! before the file is kept.

use std::path::Path;

use rmcp::schemars;
use satz_core::pipeline::ResolvedRequest;

use crate::ToolConfig;

/// What `add-entry` did.
#[derive(Debug, serde::Serialize, schemars::JsonSchema, PartialEq)]
pub(crate) struct EntryAdded {
    pub estate: String,
    /// the list param the entry was added to
    pub list: String,
    /// the value of the entry's key field
    pub key: String,
    /// the entry as it stands in the file
    pub entry: String,
    /// how many entries the list holds now, contributions included
    pub entries: usize,
}

/// One `field=value` as typed: the declaration decides what kind of value it is.
fn typed(request: &ResolvedRequest, field: &str, text: &str) -> Result<serde_yaml::Value, String> {
    let like = request.defaults.iter().find(|(f, _)| f == field).map(|(_, v)| v);
    match like {
        Some(serde_yaml::Value::Sequence(_)) => Ok(serde_yaml::Value::Sequence(
            text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| serde_yaml::Value::String(s.to_string())).collect(),
        )),
        Some(serde_yaml::Value::Bool(_)) => match text {
            "true" => Ok(serde_yaml::Value::Bool(true)),
            "false" => Ok(serde_yaml::Value::Bool(false)),
            other => Err(format!("`{}` is `true` or `false` — got `{}`", field, other)),
        },
        Some(serde_yaml::Value::Number(_)) => text
            .parse::<i64>()
            .map(|n| serde_yaml::Value::Number(n.into()))
            .map_err(|_| format!("`{}` is a number — got `{}`", field, text)),
        _ => Ok(serde_yaml::Value::String(text.to_string())),
    }
}

/// An entry as Satz writes one: `{ name = "payments" delegation = "payments" budgets = true }`, the
/// fields in the order the declaration lists them.
pub(crate) fn render(request: &ResolvedRequest, fields: &[(String, serde_yaml::Value)]) -> String {
    let mut parts = Vec::new();
    for f in &request.fields {
        if let Some((_, v)) = fields.iter().find(|(name, _)| name == f) {
            parts.push(format!("{} = {}", f, crate::interview::literal(v)));
        }
    }
    format!("{{ {} }}", parts.join(" "))
}

/// The list's literal with `entry` added as its last one. A list on one line stays on one
/// line; a list with an entry per line gets a line.
pub(crate) fn appended(list: &str, entry: &str) -> Result<String, String> {
    let close = list.rfind(']').ok_or_else(|| format!("the value `{}` is no list", list.trim()))?;
    if !list.trim_start().starts_with('[') {
        return Err(format!("the value `{}` is no list", list.trim()));
    }
    let (head, tail) = list.split_at(close);
    let body = head.trim_end();
    if body.ends_with('[') {
        return Ok(format!("{}{}{}", body, entry, tail));
    }
    if head.contains('\n') {
        // one entry per line: the new one takes the indentation of the last
        let indent: String = body.lines().last().unwrap_or_default().chars().take_while(|c| c.is_whitespace()).collect();
        let comma = if body.ends_with(',') { "" } else { "," };
        let closing: String = head[body.len()..].to_string();
        return Ok(format!("{}{}\n{}{},{}{}", body, comma, indent, entry, closing, tail));
    }
    let comma = if body.ends_with(',') { "" } else { "," };
    Ok(format!("{}{} {}{}", body, comma, entry, tail))
}

/// Add one entry to `list` in the estate. Refused, with nothing written: a list no pack of
/// the estate declares with `entries`, a field the declaration does not list, a value its
/// pattern does not match, an entry without its key, a key the list already holds, and an
/// estate that does not transpile with the entry in it.
pub(crate) fn add(estate: &Path, tool: &ToolConfig, runtime: &ToolConfig, list: &str, pairs: &[(String, String)]) -> Result<EntryAdded, String> {
    let compiled = crate::pipeline_b_compile(estate, tool, runtime, crate::PrerequisiteFindings::Report, crate::FindingsOutput::Silent).map_err(|e| e.to_string())?;
    let request = compiled.requests.iter().find(|r| r.param == list).ok_or_else(|| {
        let known: Vec<String> = compiled.requests.iter().map(|r| format!("`{}`", r.param)).collect();
        format!(
            "`{}` is no list a pack of this estate declares with `entries` — {}",
            list,
            if known.is_empty() { "it declares none: switch on the pack that takes the entry first".to_string() } else { format!("it declares {}", known.join(", ")) }
        )
    })?;
    let mut fields: Vec<(String, serde_yaml::Value)> = Vec::new();
    for (field, text) in pairs {
        if !request.fields.contains(field) {
            return Err(format!(
                "`{}` is no field of an entry of `{}` — its fields are {}",
                field,
                list,
                request.fields.iter().map(|f| format!("`{}`", f)).collect::<Vec<_>>().join(", ")
            ));
        }
        if fields.iter().any(|(f, _)| f == field) {
            return Err(format!("`{}` is given twice", field));
        }
        fields.push((field.clone(), typed(request, field, text)?));
    }
    let key = match fields.iter().find(|(f, _)| *f == request.key) {
        Some((_, serde_yaml::Value::String(k))) if !k.is_empty() => k.clone(),
        _ => return Err(format!("an entry of `{}` is named by its `{}` — give `{}=<value>`", list, request.key, request.key)),
    };
    if request.entries.iter().any(|e| e.get(request.key.as_str()).and_then(|v| v.as_str()) == Some(key.as_str())) {
        return Err(format!("`{}` holds an entry `{}` already — an entry is changed in the estate file", list, key));
    }
    let entry = render(request, &fields);
    // the declaration's own check: the patterns, as a request file is judged
    let as_request = format!("pack entry version \"1\"\n\nparams {{\n  contributes_{} = [\n    {},\n  ]\n}}\n", list, entry);
    let found = satz_core::pipeline::check_request_file(&as_request, &compiled.requests, &compiled.env);
    if let Some((_, msg)) = found.first() {
        return Err(msg.clone());
    }
    let before = crate::fsx::read_to_string(estate).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let current = crate::interview::bound_literal(&before, list)?.unwrap_or_else(|| "[]".to_string());
    let after = crate::interview::bind_literal(&before, list, &appended(&current, &entry)?)?;
    crate::fsx::write_edited_satz(estate, &before, &after).map_err(|e| e.to_string())?;
    let proven = crate::pipeline_b_compile(estate, tool, runtime, crate::PrerequisiteFindings::Report, crate::FindingsOutput::Silent);
    let entries = match proven {
        Ok(c) => c.requests.iter().find(|r| r.param == list).map(|r| r.entries.len()).unwrap_or_default(),
        Err(e) => {
            return Err(match crate::fsx::write_verbatim(estate, &before) {
                Ok(()) => format!("the entry leaves an estate satz refuses: {} — nothing is written, {} is as it was", e, estate.display()),
                Err(w) => format!("the entry leaves an estate satz refuses: {} — and restoring {} failed: {}", e, estate.display(), w),
            });
        }
    };
    Ok(EntryAdded { estate: estate.display().to_string(), list: list.to_string(), key, entry, entries })
}

/// `field=value` as the command line gives it.
pub(crate) fn pair(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((f, v)) if !f.trim().is_empty() => Ok((f.trim().to_string(), v.to_string())),
        _ => Err(format!("`{}`: a field is given as `<field>=<value>`", text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ResolvedRequest {
        ResolvedRequest {
            param: "delegated_projects".into(),
            key: "name".into(),
            fields: vec!["name".into(), "delegation".into(), "apis".into(), "budgets".into()],
            patterns: vec![("name".into(), "[a-z][a-z0-9-]*".into())],
            defaults: vec![
                ("apis".into(), serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("iam.googleapis.com".into())])),
                ("budgets".into(), serde_yaml::Value::Bool(false)),
            ],
            field_defaults: Vec::new(),
            description: None,
            entries: Vec::new(),
            file: "p.satz".into(),
            line: 1,
        }
    }

    /// A value is what the declaration's default says it is: a list, a yes or no, a text.
    #[test]
    fn a_value_takes_the_kind_the_declaration_gives_its_field() {
        let r = request();
        assert_eq!(typed(&r, "budgets", "true").unwrap(), serde_yaml::Value::Bool(true));
        assert!(typed(&r, "budgets", "yes").unwrap_err().contains("`true` or `false`"));
        assert_eq!(
            typed(&r, "apis", "run.googleapis.com, iam.googleapis.com").unwrap(),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("run.googleapis.com".into()), serde_yaml::Value::String("iam.googleapis.com".into())])
        );
        assert_eq!(typed(&r, "name", "true").unwrap(), serde_yaml::Value::String("true".into()));
    }

    /// The entry is written in the declaration's order, whatever order it was typed in.
    #[test]
    fn an_entry_is_rendered_in_the_declarations_order() {
        let r = request();
        let fields = vec![
            ("budgets".to_string(), serde_yaml::Value::Bool(true)),
            ("name".to_string(), serde_yaml::Value::String("rag".into())),
            ("delegation".to_string(), serde_yaml::Value::String("rag".into())),
        ];
        assert_eq!(render(&r, &fields), "{ name = \"rag\" delegation = \"rag\" budgets = true }");
    }

    /// An empty list, a list on one line and a list with an entry per line each take the
    /// entry in their own layout.
    #[test]
    fn the_entry_joins_the_list_in_the_lists_own_layout() {
        let e = "{ name = \"b\" }";
        assert_eq!(appended("[]", e).unwrap(), "[{ name = \"b\" }]");
        assert_eq!(appended("[{ name = \"a\" }]", e).unwrap(), "[{ name = \"a\" }, { name = \"b\" }]");
        assert_eq!(appended("[\n    { name = \"a\" },\n  ]", e).unwrap(), "[\n    { name = \"a\" },\n    { name = \"b\" },\n  ]");
        assert_eq!(appended("[\n    { name = \"a\" }\n  ]", e).unwrap(), "[\n    { name = \"a\" },\n    { name = \"b\" },\n  ]");
        assert!(appended("\"text\"", e).is_err());
    }

    #[test]
    fn a_field_is_given_with_an_equals_sign() {
        assert_eq!(pair("name=rag").unwrap(), ("name".to_string(), "rag".to_string()));
        assert_eq!(pair("parent=folders/1=2").unwrap(), ("parent".to_string(), "folders/1=2".to_string()));
        assert!(pair("name").is_err());
    }
}
