//! The Satz printer: a `serde_yaml` document in, Satz text out.
//!
//! Every import writes through here. The state and live shapes hand it the
//! discovered `Config`, the HCL importer the blocks it translated, the
//! org-policy export the pack it snapshots. Callers build param references,
//! interpolations and rendered param values with `param_ref`,
//! `interpolation`, `interpolated` and `param_value` rather than spelling the
//! printer's internal frames themselves.
//!
//! The printer refuses what it cannot render exactly — a null value, an
//! unknown tag, a non-string key — because a silently wrong estate is worse
//! than an unconverted one.

use std::fmt::Write as _;

#[derive(Debug)]
pub struct MigrateError {
    pub msg: String,
}
impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "migrate: {}", self.msg)
    }
}
impl std::error::Error for MigrateError {}
fn err<T>(msg: impl Into<String>) -> Result<T, MigrateError> {
    Err(MigrateError { msg: msg.into() })
}

// The frame a param reference travels in, so a reference survives a round trip
// through `serde_yaml::Value` without being mistaken for a literal string.
const REF_L: &str = "\u{ab}R:";  // «R:name»
const REF_R: &str = "\u{bb}";

fn ident_from_name(name: &str) -> String {
    name.replace(['-', '.'], "_")
}

/// Escape a literal string chunk for a Satz interpolated string.
fn esc(chunk: &str) -> String {
    chunk
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('{', "{{")
        .replace('}', "}}")
}

fn as_ref_name(s: &str) -> Option<&str> {
    s.strip_prefix(REF_L)?.strip_suffix(REF_R)
}

/// A !format (template, args...) sequence into a Satz interpolation body.
fn format_to_interpolation(vals: &[serde_yaml::Value]) -> Result<String, MigrateError> {
    let template = vals
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| MigrateError { msg: "!format without template".into() })?;
    let mut out = String::new();
    let mut args = vals[1..].iter();
    let tb: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < tb.len() {
        if tb[i] == '{' && tb.get(i + 1) == Some(&'{') {
            out.push_str("{{");
            i += 2;
        } else if tb[i] == '}' && tb.get(i + 1) == Some(&'}') {
            out.push_str("}}");
            i += 2;
        } else if tb[i] == '{' && tb.get(i + 1) == Some(&'}') {
            let arg = args
                .next()
                .ok_or_else(|| MigrateError { msg: format!("!format: more {{}} than args in '{}'", template) })?;
            match arg {
                serde_yaml::Value::String(s) => match as_ref_name(s) {
                    Some(name) => {
                        out.push('{');
                        out.push_str(&ident_from_name(name));
                        out.push('}');
                    }
                    None => out.push_str(&esc(s)),
                },
                other => {
                    let s = serde_yaml::to_string(other).unwrap_or_default();
                    out.push_str(&esc(s.trim()));
                }
            }
            i += 2;
        } else {
            let mut chunk = String::new();
            chunk.push(tb[i]);
            out.push_str(&esc(&chunk));
            i += 1;
        }
    }
    if args.next().is_some() {
        return Err(MigrateError { msg: format!("!format: more args than {{}} in '{}'", template) });
    }
    Ok(out)
}

/// A value that prints on one line: a scalar, or a tagged value that renders
/// to a string (`!format`, `!expr`).
fn is_scalar_like(v: &serde_yaml::Value) -> bool {
    matches!(
        v,
        serde_yaml::Value::String(_) | serde_yaml::Value::Number(_) | serde_yaml::Value::Bool(_) | serde_yaml::Value::Tagged(_)
    )
}

/// A scalar Value into a Satz value expression.
fn scalar_value(v: &serde_yaml::Value) -> Result<String, MigrateError> {
    match v {
        serde_yaml::Value::String(s) => match as_ref_name(s) {
            Some(name) => Ok(ident_from_name(name)),
            None => Ok(format!("\"{}\"", esc(s))),
        },
        serde_yaml::Value::Number(n) => Ok(n.to_string()),
        serde_yaml::Value::Bool(b) => Ok(b.to_string()),
        serde_yaml::Value::Null => err("null values are not convertible — convert by hand"),
        other => err(format!("unexpected scalar {:?}", other)),
    }
}

/// Value (possibly tagged) into a Satz value expression, block-aware.
fn value_expr(v: &serde_yaml::Value, indent: usize) -> Result<String, MigrateError> {
    match v {
        serde_yaml::Value::Tagged(t) => {
            let tag = t.tag.to_string();
            match tag.trim_start_matches('!') {
                "format" => {
                    let seq = t.value.as_sequence().ok_or_else(|| MigrateError {
                        msg: "!format must be a sequence".into(),
                    })?;
                    Ok(format!("\"{}\"", format_to_interpolation(seq)?))
                }
                "expr" => {
                    let s = t.value.as_str().ok_or_else(|| MigrateError { msg: "!expr needs a string".into() })?;
                    Ok(format!("\"${{{{{}}}}}\"", s))
                }
                "join" => {
                    let seq = t.value.as_sequence().ok_or_else(|| MigrateError { msg: "!join needs a sequence".into() })?;
                    let mut out = String::from("\"");
                    for item in seq {
                        match item {
                            serde_yaml::Value::String(s) => match as_ref_name(s) {
                                Some(name) => {
                                    out.push('{');
                                    out.push_str(&ident_from_name(name));
                                    out.push('}');
                                }
                                None => out.push_str(&esc(s)),
                            },
                            other => out.push_str(&esc(serde_yaml::to_string(other).unwrap_or_default().trim())),
                        }
                    }
                    out.push('"');
                    Ok(out)
                }
                other => err(format!("unknown tag !{} — convert by hand", other)),
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            let pad = " ".repeat(indent + 2);
            let mut out = String::from("[\n");
            for item in seq {
                match item {
                    serde_yaml::Value::Mapping(m) => {
                        // an object of scalars is one line — the form adopt
                        // writes for a grant edge with its import id, and the
                        // library's `rules = [ { enforce = "TRUE" } ]`; the
                        // formatter keeps an inline construct inline
                        if m.values().all(is_scalar_like) {
                            let mut fields = Vec::new();
                            for (k, v) in m {
                                let (key, _) = key_expr(k)?;
                                fields.push(format!("{} = {}", key, value_expr(v, indent + 2)?));
                            }
                            let _ = writeln!(out, "{}{{ {} }},", pad, fields.join(" "));
                            continue;
                        }
                        let _ = writeln!(out, "{}{{", pad);
                        emit_entries(m, &mut out, indent + 4)?;
                        let _ = writeln!(out, "{}}},", pad);
                    }
                    other => {
                        let _ = writeln!(out, "{}{},", pad, value_expr(other, indent + 2)?);
                    }
                }
            }
            let _ = write!(out, "{}]", " ".repeat(indent));
            Ok(out)
        }
        serde_yaml::Value::Mapping(_) => err("nested mapping where scalar expected (handled by caller)"),
        scalar => scalar_value(scalar),
    }
}

fn key_expr(k: &serde_yaml::Value) -> Result<(String, bool), MigrateError> {
    // returns (rendered key, is_identifier)
    match k {
        serde_yaml::Value::String(s) => {
            if let Some(name) = as_ref_name(s) {
                return Ok((format!("\"{{{}}}\"", ident_from_name(name)), false));
            }
            let ident_ok = !s.is_empty()
                && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if ident_ok {
                Ok((s.clone(), true))
            } else {
                Ok((format!("\"{}\"", esc(s)), false))
            }
        }
        serde_yaml::Value::Tagged(t) if t.tag.to_string().trim_start_matches('!') == "format" => {
            let seq = t.value.as_sequence().ok_or_else(|| MigrateError { msg: "!format key".into() })?;
            Ok((format!("\"{}\"", format_to_interpolation(seq)?), false))
        }
        other => err(format!("unsupported key {:?}", other)),
    }
}

/// A grant type carrying a SEQUENCE of maps — one scope-pinned member map per
/// bucket, service account, … (`google_storage_bucket_iam_member { bucket = "a"
/// … } google_storage_bucket_iam_member { bucket = "b" … }`): resource-type maps
/// may repeat, and the document holds one key per type, so the repeats travel
/// as a list and print as one block each.
fn repeated_grant_maps<'a>(k: &serde_yaml::Value, v: &'a serde_yaml::Value) -> Option<Vec<&'a serde_yaml::Mapping>> {
    let key = k.as_str()?;
    if !key.ends_with("_iam_member") {
        return None;
    }
    let seq = v.as_sequence()?;
    if seq.is_empty() {
        return None;
    }
    seq.iter().map(|item| item.as_mapping()).collect()
}

/// The body a key opens as a `{ … }` block, or `None` when the value is an
/// attribute. A mapping is a block; every other value is an attribute.
fn as_block(v: &serde_yaml::Value) -> Option<&serde_yaml::Mapping> {
    match v {
        serde_yaml::Value::Mapping(child) => Some(child),
        _ => None,
    }
}

fn emit_entries(m: &serde_yaml::Mapping, out: &mut String, indent: usize) -> Result<(), MigrateError> {
    let pad = " ".repeat(indent);
    for (k, v) in m {
        let (key, _) = key_expr(k)?;
        if let Some(maps) = repeated_grant_maps(k, v) {
            for child in maps {
                let _ = writeln!(out, "{}{} {{", pad, key);
                emit_entries(child, out, indent + 2)?;
                let _ = writeln!(out, "{}}}", pad);
            }
            continue;
        }
        match as_block(v) {
            Some(child) => {
                let _ = writeln!(out, "{}{} {{", pad, key);
                emit_entries(child, out, indent + 2)?;
                let _ = writeln!(out, "{}}}", pad);
            }
            None => {
                let _ = writeln!(out, "{}{} = {}", pad, key, value_expr(v, indent)?);
            }
        }
    }
    Ok(())
}

/// Keys that look like a shorthand type name but are not: Satz's own block
/// keywords, and the one attribute (`project_service`) a project body carries
/// as a list.
///
/// `folder` and `project` are deliberately NOT here. They are structural in
/// Satz, but structure is not a reason to invent a bare keyword — they are real
/// Terraform types (`google_folder`, `google_project`) and Satz names every type
/// in full, so the short spelling gets rewritten like any other.
const NEVER_A_TYPE_KEY: &[&str] = &[
    "params",
    "terraform",
    "providers",
    "backend",
    "hcl",
    "claim",
    "question",
    "action",
    "offers",
    "notice",
    "export",
    "interface",
    "suppress",
    "project_service",
];

/// Give a shorthand resource key its provider prefix (`folder` →
/// `google_folder`).
///
/// A discovered document names its containers the way the `Config` shape spells
/// them, without the prefix; Satz names every Terraform type in full, so a
/// verbatim key would not compile. Operates on the printer's own output, whose
/// formatting is known: a block opener is `<indent><ident> {` on its own line.
/// `is_type` answers from the provider schemas — a key is only rewritten when
/// the prefixed form is a real type and the bare form is not, so `labels { … }`
/// is left alone.
pub fn normalize_type_keys(satz: &str, is_type: &dyn Fn(&str) -> bool) -> String {
    fn full(ident: &str, is_type: &dyn Fn(&str) -> bool) -> Option<String> {
        if NEVER_A_TYPE_KEY.contains(&ident) || is_type(ident) {
            return None;
        }
        let prefixed = format!("google_{}", ident);
        is_type(&prefixed).then_some(prefixed)
    }

    let mut out = String::with_capacity(satz.len());
    for line in satz.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trailing = &line[body.len()..];
        let indent_len = body.len() - body.trim_start().len();
        let (indent, rest) = body.split_at(indent_len);

        // `<ident> {`
        if let Some(ident) = rest.strip_suffix('{').map(str::trim_end) {
            let is_ident = !ident.is_empty()
                && ident.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if is_ident {
                if let Some(f) = full(ident, is_type) {
                    out.push_str(indent);
                    out.push_str(&f);
                    out.push_str(" {");
                    out.push_str(trailing);
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    out
}

/// A value that prints as an interpolated Satz string: `template` with one
/// `{}` per name in `params`, each becoming a `{param}` reference — for
/// callers building documents for `convert_value` that must reference a
/// param rather than carry a literal (an exported pack's `parent`).
pub fn interpolated(template: &str, params: &[&str]) -> serde_yaml::Value {
    let mut seq = vec![serde_yaml::Value::String(template.to_string())];
    for p in params {
        seq.push(serde_yaml::Value::String(format!("{}{}{}", REF_L, p, REF_R)));
    }
    serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
        tag: serde_yaml::value::Tag::new("format"),
        value: serde_yaml::Value::Sequence(seq),
    }))
}

/// The Satz param name for an HCL identifier: `-` and `.` become `_`, the same
/// normalisation every other printed reference gets.
pub fn param_name(name: &str) -> String {
    ident_from_name(name)
}

/// A value that prints as a **bare** param reference (`role_id = IAMRoleID`).
/// Bare is not cosmetic: it preserves the param's TYPE, so a list or bool param
/// can be referenced as one, which `"{name}"` could not do. Callers building
/// documents for `convert_value` use this instead of writing the sentinel frame
/// themselves.
pub fn param_ref(name: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(format!("{}{}{}", REF_L, name, REF_R))
}

/// A value that prints as an interpolated Satz string, built from literal
/// chunks and `param_ref`s in order.
///
/// Every part travels as an ARGUMENT and the template is nothing but `{}`
/// placeholders. That is deliberate: `format_to_interpolation` escapes an
/// argument with `esc` (total — it doubles `{` and `}`), while its template
/// path reads `{{`, `}}` and `{}` as syntax. Text whose braces are not under
/// the caller's control is therefore only safe in the argument position.
pub fn interpolation(parts: Vec<serde_yaml::Value>) -> serde_yaml::Value {
    let mut seq = vec![serde_yaml::Value::String("{}".repeat(parts.len()))];
    seq.extend(parts);
    serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
        tag: serde_yaml::value::Tag::new("format"),
        value: serde_yaml::Value::Sequence(seq),
    }))
}

/// One value rendered as the text a `params` entry takes in `convert_value` —
/// the printer's own renderer, at the indentation a param sits at, so callers
/// never re-implement it.
pub fn param_value(v: &serde_yaml::Value) -> Result<String, MigrateError> {
    value_expr(v, 2)
}

/// Print a document as Satz. `params` are `(name, already-rendered value)`
/// pairs; `header` lines become leading `//` comments (an empty line stays a
/// blank comment line).
pub fn convert_value(
    top: &serde_yaml::Mapping,
    kind_keyword: &str,
    name: &str,
    params: &[(String, String)],
    header: &[String],
) -> Result<String, MigrateError> {
    let mut out = String::new();
    for h in header {
        if h.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "// {}", h);
        }
    }
    if !header.is_empty() {
        out.push('\n');
    }
    let _ = writeln!(out, "{} {}\n", kind_keyword, ident_from_name(name));

    if !params.is_empty() {
        out.push_str("params {\n");
        for (name, value) in params {
            let _ = writeln!(out, "  {} = {}", name, value);
        }
        out.push_str("}\n\n");
    }

    for (k, v) in top {
        let (key, is_ident) = key_expr(k)?;
        if let Some(maps) = repeated_grant_maps(k, v) {
            for child in maps {
                let _ = writeln!(out, "{} {{", key);
                emit_entries(child, &mut out, 2)?;
                out.push_str("}\n\n");
            }
            continue;
        }
        match as_block(v) {
            Some(child) => {
                let _ = writeln!(out, "{} {{", key);
                emit_entries(child, &mut out, 2)?;
                out.push_str("}\n\n");
            }
            None => {
                let _ = is_ident;
                // fragment packs: top-level entry with a value (IAM member -> roles list,
                // scalar attrs) — legal Satz since top-level `key = value` landed
                let _ = writeln!(out, "{} = {}\n", key, value_expr(v, 0)?);
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three helpers callers build their param references with. A bare ref
    /// must stay bare (that is what preserves a list's type), and a literal
    /// chunk must be escaped even when it contains the interpolation syntax —
    /// which is exactly why the chunk travels as an argument, not a template.
    #[test]
    fn param_helpers_render_bare_refs_and_escape_literal_chunks() {
        assert_eq!(param_name("a-b.c"), "a_b_c");

        let mut top = serde_yaml::Mapping::new();
        let mut inner = serde_yaml::Mapping::new();
        inner.insert("bare".into(), param_ref("a-b"));
        inner.insert(
            "mixed".into(),
            interpolation(vec![
                serde_yaml::Value::String("x{y} ${z.w} ".to_string()),
                param_ref("p"),
            ]),
        );
        let mut t = serde_yaml::Mapping::new();
        t.insert("i".into(), serde_yaml::Value::Mapping(inner));
        top.insert("section".into(), serde_yaml::Value::Mapping(t));

        let s = convert_value(&top, "pack", "t", &[], &[]).unwrap();
        // bare: prints as an identifier, not a quoted "{a_b}"
        assert!(s.contains("bare = a_b"), "{s}");
        // the chunk's own braces are doubled, and the param becomes {p}
        assert!(s.contains(r#"mixed = "x{{y}} ${{z.w}} {p}""#), "{s}");

        // a list renders at a param's indentation
        let list = serde_yaml::Value::Sequence(vec!["a".into(), "b".into()]);
        assert_eq!(param_value(&list).unwrap(), "[\n    \"a\",\n    \"b\",\n  ]");
    }

    /// `interpolated` names the template itself, so its `{}` placeholders are
    /// syntax and the params fill them in order — what the org-policy export
    /// writes a pack's `parent` with.
    #[test]
    fn an_interpolated_template_fills_its_placeholders_in_order() {
        let mut inner = serde_yaml::Mapping::new();
        inner.insert("parent".into(), interpolated("organizations/{}", &["customer_organization_id"]));
        let mut t = serde_yaml::Mapping::new();
        t.insert("p".into(), serde_yaml::Value::Mapping(inner));
        let mut top = serde_yaml::Mapping::new();
        top.insert("google_org_policy_policy".into(), serde_yaml::Value::Mapping(t));
        let s = convert_value(&top, "pack", "t", &[], &[]).unwrap();
        assert!(s.contains(r#"parent = "organizations/{customer_organization_id}""#), "{s}");
    }

    /// What the printer writes must parse as Satz. Every import shape rides on
    /// this.
    #[test]
    fn printed_output_parses_as_satz() {
        let mut body = serde_yaml::Mapping::new();
        body.insert("display_name".into(), "f".into());
        let mut block = serde_yaml::Mapping::new();
        block.insert("a".into(), serde_yaml::Value::Mapping(body));
        let mut top = serde_yaml::Mapping::new();
        top.insert("google_folder".into(), serde_yaml::Value::Mapping(block));
        let s = convert_value(&top, "pack", "t", &[("v".to_string(), "\"x\"".to_string())], &[]).unwrap();
        let f = crate::satz::parse(&s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
        assert!(f.params.iter().any(|(n, _, _)| n == "v"), "{}", s);
    }

    /// A literal Terraform reference keeps its braces doubled, and a multi-line
    /// string travels as one escaped Satz string.
    #[test]
    fn literal_blocks_and_tf_refs_escape() {
        let mut body = serde_yaml::Mapping::new();
        body.insert("d".into(), "line1\nline2\n".into());
        body.insert("m".into(), "${a.b.c}".into());
        let mut block = serde_yaml::Mapping::new();
        block.insert("i".into(), serde_yaml::Value::Mapping(body));
        let mut top = serde_yaml::Mapping::new();
        top.insert("section".into(), serde_yaml::Value::Mapping(block));
        let s = convert_value(&top, "pack", "t", &[], &[]).unwrap();
        assert!(s.contains("d = \"line1\\nline2\\n\""), "{s}");
        assert!(s.contains("m = \"${{a.b.c}}\""), "{s}");
    }

    /// A null value has no Satz spelling, so the printer refuses rather than
    /// inventing one.
    #[test]
    fn a_null_value_is_refused() {
        let mut body = serde_yaml::Mapping::new();
        body.insert("k".into(), serde_yaml::Value::Null);
        let mut block = serde_yaml::Mapping::new();
        block.insert("i".into(), serde_yaml::Value::Mapping(body));
        let mut top = serde_yaml::Mapping::new();
        top.insert("section".into(), serde_yaml::Value::Mapping(block));
        let e = convert_value(&top, "pack", "t", &[], &[]).unwrap_err();
        assert!(e.msg.contains("null values"), "{}", e.msg);
    }
}

#[cfg(test)]
mod type_key_tests {
    //! A discovered document names its containers without the provider prefix.
    //! Satz names every type in full, so the printer's output is normalised
    //! against the schemas before it is written.
    use super::*;

    fn types(t: &str) -> bool {
        matches!(
            t,
            "google_org_policy_policy"
                | "google_project_iam_member"
                | "google_folder"
                | "google_project"
                | "google_billing_budget"
        )
    }

    #[test]
    fn shorthand_keys_gain_the_provider_prefix() {
        let out = normalize_type_keys("org_policy_policy {\n  p {\n  }\n}\n", &types);
        assert!(out.starts_with("google_org_policy_policy {"), "{}", out);
    }

    #[test]
    fn nested_shorthand_gains_it_too() {
        let src = "project {\n  p1 {\n    project_iam_member {\n    }\n  }\n}\n";
        let out = normalize_type_keys(src, &types);
        assert!(out.contains("    google_project_iam_member {"), "{}", out);
    }

    /// Structure is not a reason to keep a bare keyword: `folder` and `project`
    /// are real Terraform types, so the short spelling is rewritten like any
    /// other. Satz has no keyword resource types at all — the schema is the only
    /// authority on what a type is called.
    #[test]
    fn structural_types_are_rewritten_too() {
        let out = normalize_type_keys("folder {\n  a {\n  }\n}\nproject {\n  b {\n  }\n}\n", &types);
        assert!(out.starts_with("google_folder {"), "{}", out);
        assert!(out.contains("\ngoogle_project {"), "{}", out);
    }

    /// Satz's own block keywords are a different thing entirely, and no
    /// `google_` type shadows them.
    #[test]
    fn satz_block_keywords_are_left_alone() {
        let out = normalize_type_keys("params {\n}\nterraform {\n}\nproviders {\n}\n", &types);
        assert!(out.starts_with("params {"), "{}", out);
        assert!(out.contains("\nterraform {"), "{}", out);
        assert!(out.contains("\nproviders {"), "{}", out);
    }

    /// A nested attribute block no `google_` type shadows must be left alone.
    #[test]
    fn attribute_blocks_are_left_alone() {
        let out = normalize_type_keys("project_iam_member {\n  labels {\n  }\n}\n", &types);
        assert!(out.contains("  labels {"), "{}", out);
    }
}
