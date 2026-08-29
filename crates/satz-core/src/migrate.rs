//! The yaml shape of `satz import`: mechanical conversion of the YAML dialect to Satz.
//!
//! This is the repeatable half of the migration loop: convert, then the caller
//! GATES the result (transpile old vs new, sorted-compare) and only a PROVEN
//! conversion moves on. The converter therefore prefers erroring on anything it
//! does not fully understand over guessing — an unconverted file is fine, a
//! silently wrong one is not.
//!
//! Strategy: serde_yaml resolves anchors/aliases away (losing parameterization),
//! so a textual pre-pass first extracts the `variables:` block (→ params) and
//! substitutes alias tokens and include directives with sentinel strings; the
//! remaining document then parses cleanly and the walker emits Satz, decoding
//! sentinels into param refs, interpolations and `use` statements.
//!
//! Known, printed limitation: interior comments do not survive (the leading
//! header comment block does).

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

// Sentinel frames (printable, never occurring in the dialect):
const REF_L: &str = "\u{ab}R:";  // «R:name»
const REF_R: &str = "\u{bb}";
const USE_L: &str = "\u{ab}U:";  // «U:form|path|cond»
const USE_K: &str = "\u{ab}UK";  // «UK<n>» — synthetic key for Form A in mappings

fn snake(name: &str) -> String {
    name.replace(['-', '.'], "_")
}

/// Replace `*alias` tokens (value or key position) with sentinel strings so the
/// document parses without anchors. Boundary-aware; leaves comments alone (they
/// are dropped by the YAML parse anyway).
fn substitute_aliases(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let b: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            i += 1;
            continue;
        }
        let boundary_before = i == 0
            || matches!(b[i - 1], ' ' | '[' | ',' | ':' | '(' | '{');
        if c == '*' && !in_str && boundary_before {
            let mut j = i + 1;
            let mut name = String::new();
            while j < b.len()
                && (b[j].is_ascii_alphanumeric() || b[j] == '-' || b[j] == '_' || b[j] == '.')
            {
                name.push(b[j]);
                j += 1;
            }
            if !name.is_empty() {
                out.push('"');
                out.push_str(REF_L);
                out.push_str(&name);
                out.push_str(REF_R);
                out.push('"');
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Pre-pass over the raw text. Returns (yaml-for-parse, header_comment, params, uses_at_top).
struct PrePassed {
    yaml: String,
    header: Vec<String>,
    /// (snake name, satz value expression)
    params: Vec<(String, String)>,
    /// The source used `!import-include`, the dialect's transpile-time live
    /// import. Converted as a plain include; the operator runs `satz adopt`.
    import_include_seen: bool,
}

fn pre_pass(src: &str) -> Result<PrePassed, MigrateError> {
    let lines: Vec<&str> = src.lines().collect();
    let mut header = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with('#') {
            header.push(t.trim_start_matches('#').trim_start().to_string());
            i += 1;
        } else if t.is_empty() && header.is_empty() {
            i += 1;
        } else {
            break;
        }
    }

    let mut out: Vec<String> = Vec::new();
    let mut params: Vec<(String, String)> = Vec::new();
    let mut in_vars = false;
    let mut use_n = 0usize;
    let mut import_include_seen = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // top-level variables block extraction
        if indent == 0 && trimmed == "variables:" {
            in_vars = true;
            i += 1;
            continue;
        }
        if in_vars {
            if !line.starts_with(' ') && !trimmed.is_empty() && !trimmed.starts_with('#') {
                in_vars = false; // fall through to normal handling of this line
            } else {
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    i += 1;
                    continue;
                }
                // entry: key: &anchor VALUE   (VALUE may be inline !format or block !format)
                let Some(colon) = trimmed.find(':') else {
                    return err(format!("variables: unparseable line: {}", line));
                };
                let key = trimmed[..colon].trim().to_string();
                let after = trimmed[colon + 1..].trim();
                let Some(rest) = after.strip_prefix('&') else {
                    return err(format!("variables entry without anchor: {}", line));
                };
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                    .unwrap_or(rest.len());
                let anchor = &rest[..end];
                if anchor != key {
                    return err(format!(
                        "variables entry where anchor '{}' differs from key '{}' — convert by hand",
                        anchor, key
                    ));
                }
                let mut value_text = rest[end..].trim().to_string();
                if value_text == "!format" || value_text.is_empty() {
                    // block form: consume following "- item" lines (deeper indent)
                    let tag_block = value_text == "!format";
                    let mut items = Vec::new();
                    let mut j = i + 1;
                    while j < lines.len() {
                        let lt = lines[j].trim_start();
                        let li = lines[j].len() - lt.len();
                        if li > indent && lt.starts_with("- ") {
                            items.push(lt[2..].trim().to_string());
                            j += 1;
                        } else if lt.is_empty() {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    if !tag_block || items.is_empty() {
                        return err(format!("variables entry with empty value: {}", line));
                    }
                    value_text = format!("!format [{}]", items.join(", "));
                    i = j - 1;
                }
                let satz_value = convert_scalar_text(&substitute_aliases(&value_text))?;
                params.push((snake(&key), satz_value));
                i += 1;
                continue;
            }
        }

        // include directives (either form), preserved positionally via sentinels
        let subst = substitute_aliases(line);
        let strimmed = subst.trim_start();
        let sindent = subst.len() - strimmed.len();
        let mk_use = |form: &str, path: &str, cond: Option<&str>, extra: &str| {
            format!(
                "{}\"{}{}»\": \"{}{}|{}|{}»\"",
                " ".repeat(sindent),
                USE_K,
                extra,
                USE_L,
                form,
                path,
                cond.unwrap_or("")
            )
        };
        // `!import-include` was the transpile-time live import of the dialect;
        // Satz has no such tag — the file is `use`d like any other and the
        // adoption happens through `satz adopt` afterwards. Convert it as a
        // plain include and tell the operator.
        let subst = if strimmed.contains("!import-include ") && !strimmed.starts_with('#') {
            import_include_seen = true;
            subst.replace("!import-include ", "!include ")
        } else {
            subst
        };
        let strimmed = subst.trim_start();
        if !strimmed.starts_with('#') {
            if let Some(rest) = strimmed.strip_prefix("!include-if ") {
                let mut parts = rest.trim().splitn(2, ' ');
                let cond = parts.next().unwrap_or("").trim_start_matches(['*', '&']);
                let path = parts.next().unwrap_or("").trim();
                out.push(mk_use("A", path, Some(cond), &use_n.to_string()));
                use_n += 1;
                i += 1;
                continue;
            }
            if let Some(rest) = strimmed.strip_prefix("!include ") {
                out.push(mk_use("A", rest.trim(), None, &use_n.to_string()));
                use_n += 1;
                i += 1;
                continue;
            }
            if let Some(colon) = strimmed.find(": !include") {
                let key = strimmed[..colon].trim().to_string();
                let after = &strimmed[colon + 2..];
                let (path, cond) = if let Some(rest) = after.strip_prefix("!include-if ") {
                    let mut parts = rest.trim().splitn(2, ' ');
                    let c = parts.next().unwrap_or("").trim_start_matches(['*', '&']).to_string();
                    (parts.next().unwrap_or("").trim().to_string(), c)
                } else if let Some(rest) = after.strip_prefix("!include ") {
                    (rest.trim().to_string(), String::new())
                } else {
                    (String::new(), String::new())
                };
                if !path.is_empty() {
                    out.push(format!(
                        "{}{}: \"{}B|{}|{}»\"",
                        " ".repeat(sindent), key, USE_L, path, cond
                    ));
                    i += 1;
                    continue;
                }
            }
        }
        out.push(subst);
        i += 1;
    }

    // Preserve the source's final newline: a literal block (`|`) as the file's last
    // content keeps its trailing \n only if the text still ends with one — losing it
    // silently changes block-scalar chomping semantics (found by one estate's conversion gate).
    let mut yaml = out.join("\n");
    if src.ends_with('\n') {
        yaml.push('\n');
    }
    Ok(PrePassed { yaml, header, params, import_include_seen })
}

/// Convert a scalar value's TEXT (from the variables pre-pass) into a Satz value
/// expression: quoted string, number, bool, param ref, or interpolation.
fn convert_scalar_text(text: &str) -> Result<String, MigrateError> {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("!format ") {
        let inner = rest.trim();
        let inner = inner
            .strip_prefix('[')
            .and_then(|x| x.strip_suffix(']'))
            .ok_or_else(|| MigrateError { msg: format!("unsupported !format: {}", t) })?;
        let args = split_top_commas(inner);
        let vals: Vec<serde_yaml::Value> = args
            .iter()
            .map(|a| serde_yaml::from_str(a.trim()).map_err(|e| MigrateError {
                msg: format!("!format arg '{}': {}", a, e),
            }))
            .collect::<Result<_, _>>()?;
        return Ok(format!("\"{}\"", format_to_interpolation(&vals)?));
    }
    if let Some(rest) = t.strip_prefix('*') {
        return Ok(snake(rest));
    }
    let v: serde_yaml::Value =
        serde_yaml::from_str(t).map_err(|e| MigrateError { msg: format!("value '{}': {}", t, e) })?;
    scalar_value(&v)
}

fn split_top_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut in_str = false;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            '[' | '{' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' if !in_str => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_str && depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
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
                        out.push_str(&snake(name));
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
    Ok(out)
}

/// The legacy YAML dialect spells a conditional IAM binding as a null-valued role
/// key with a sibling `condition:`:
///
/// ```yaml
/// - roles/storage.objectViewer:
///   condition: { title: …, expression: … }
/// ```
///
/// Satz says the same thing explicitly — `{ role = "…", condition = { … } }` —
/// which both pipelines accept. Rewrite the legacy shape on the way through;
/// anything else passes untouched.
fn normalise_conditional_binding(m: &serde_yaml::Mapping) -> serde_yaml::Mapping {
    let has_condition = m.contains_key(serde_yaml::Value::String("condition".into()));
    if !has_condition || m.contains_key(serde_yaml::Value::String("role".into())) {
        return m.clone();
    }
    let role = m.iter().find_map(|(k, v)| match (k.as_str(), v) {
        (Some(k), serde_yaml::Value::Null) if k != "condition" && k != "import-id" => Some(k.to_string()),
        _ => None,
    });
    let Some(role) = role else { return m.clone() };
    let mut out = serde_yaml::Mapping::new();
    out.insert(serde_yaml::Value::String("role".into()), serde_yaml::Value::String(role.clone()));
    for (k, v) in m {
        if k.as_str() == Some(role.as_str()) {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    out
}

/// A scalar Value into a Satz value expression.
fn scalar_value(v: &serde_yaml::Value) -> Result<String, MigrateError> {
    match v {
        serde_yaml::Value::String(s) => match as_ref_name(s) {
            Some(name) => Ok(snake(name)),
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
                                    out.push_str(&snake(name));
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
                        let m = normalise_conditional_binding(m);
                        let _ = writeln!(out, "{}{{", pad);
                        emit_entries(&m, &mut out, indent + 4)?;
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
                return Ok((format!("\"{{{}}}\"", snake(name)), false));
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

fn is_use_sentinel(v: &serde_yaml::Value) -> Option<(String, String, Option<String>)> {
    let s = v.as_str()?;
    let inner = s.strip_prefix(USE_L)?.strip_suffix('»')?;
    let mut parts = inner.splitn(3, '|');
    let form = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let cond = parts.next().filter(|c| !c.is_empty()).map(snake);
    Some((form, path, cond))
}

fn emit_use(out: &mut String, indent: usize, path: &str, cond: &Option<String>, as_key: Option<&str>) {
    let pad = " ".repeat(indent);
    let _ = write!(out, "{}use \"{}\"", pad, path);
    if let Some(k) = as_key {
        let _ = write!(out, " as {}", k);
    }
    if let Some(c) = cond {
        let _ = write!(out, " when {}", c);
    }
    out.push('\n');
}

fn emit_entries(m: &serde_yaml::Mapping, out: &mut String, indent: usize) -> Result<(), MigrateError> {
    let pad = " ".repeat(indent);
    for (k, v) in m {
        // Form A use sentinel: key "\x01U<n>", value carries the payload
        if let Some(ks) = k.as_str() {
            if ks.starts_with(USE_K) {
                if let Some((_, path, cond)) = is_use_sentinel(v) {
                    emit_use(out, indent, &path, &cond, None);
                    continue;
                }
            }
        }
        // Form B use sentinel: real key, sentinel value
        if let Some((form, path, cond)) = is_use_sentinel(v) {
            if form == "B" {
                let (key, is_ident) = key_expr(k)?;
                if !is_ident {
                    return err(format!("use ... as with non-identifier key {}", key));
                }
                emit_use(out, indent, &path, &cond, Some(&key));
                continue;
            }
        }
        let (key, _) = key_expr(k)?;
        match v {
            serde_yaml::Value::Mapping(child) => {
                let _ = writeln!(out, "{}{} {{", pad, key);
                emit_entries(child, out, indent + 2)?;
                let _ = writeln!(out, "{}}}", pad);
            }
            other => {
                let _ = writeln!(out, "{}{} = {}", pad, key, value_expr(other, indent)?);
            }
        }
    }
    Ok(())
}

/// Convert one YAML-dialect file to Satz. `kind_keyword` is "pack" or "estate";
/// `name` becomes the declared name.
/// Keys that look like a shorthand type name but are not: Satz's own block
/// keywords, and the one attribute (`project_service`) a project body carries
/// as a list.
///
/// `folder` and `project` are deliberately NOT here. They are structural in
/// Satz, but structure is not a reason to invent a bare keyword — they are
/// real Terraform types (`google_folder`, `google_project`) and Satz names
/// every type in full, so the dialect's short spelling gets rewritten like any
/// other.
const NEVER_A_TYPE_KEY: &[&str] = &[
    "params",
    "terraform",
    "providers",
    "backend",
    "hcl",
    "claim",
    "suppress",
    "project_service",
];

/// Rewrite YAML-dialect shorthand resource keys into full Terraform type names.
///
/// The dialect lets a key drop the provider prefix; Satz does not (v0.41.0), so
/// a conversion that copied keys verbatim produced a file `transpile` refuses —
/// and the converter's own gate could not see it, because that gate runs the
/// LEGACY walk, which still accepts the shorthand. It reported PROVEN on an
/// estate that would not compile.
///
/// Operates on the converter's own output, whose formatting is known: a block
/// opener is `<indent><ident> {` on its own line. `is_type` answers from the
/// provider schemas — a key is only rewritten when the prefixed form is a real
/// type and the bare form is not, so `labels { … }` is left alone.
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

        // `use "…" as <ident>`
        if let Some(as_pos) = rest.find("\" as ") {
            let (head, tail) = rest.split_at(as_pos + "\" as ".len());
            let ident = tail.trim();
            if rest.starts_with("use \"") && !ident.is_empty() {
                if let Some(f) = full(ident, is_type) {
                    out.push_str(indent);
                    out.push_str(head);
                    out.push_str(&f);
                    out.push_str(trailing);
                    continue;
                }
            }
        }
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

/// Repoint `use "X.yaml"` at `X.satz` wherever that sibling exists.
///
/// A converted estate inherits its `!include` targets verbatim, which point at
/// the YAML packs — or, once those are converted, at the twins the pack
/// conversion leaves behind for the still-YAML estate to keep working. Either
/// way the result is a Satz estate that `use`s YAML, and the fragment pipeline
/// cannot load a `.yaml` pack. The legacy walk could, which is why this went
/// unnoticed: the conversion gate ran pipeline A and was perfectly happy.
///
/// `exists` is supplied by the caller so this stays free of filesystem access;
/// it answers whether a use-path resolves, the same way the compiler resolves it.
pub fn retarget_uses(satz: &str, exists: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(satz.len());
    for line in satz.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("use \"") {
            if let Some(open) = line.find('"') {
                if let Some(close) = line[open + 1..].find('"') {
                    let path = &line[open + 1..open + 1 + close];
                    if let Some(stem) = path.strip_suffix(".yaml") {
                        let satz_path = format!("{}.satz", stem);
                        if exists(&satz_path) {
                            out.push_str(&line[..open + 1]);
                            out.push_str(&satz_path);
                            out.push_str(&line[open + 1 + close..]);
                            continue;
                        }
                    }
                }
            }
        }
        out.push_str(line);
    }
    out
}

pub fn convert(src: &str, kind_keyword: &str, name: &str) -> Result<String, MigrateError> {
    let pre = pre_pass(src)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&pre.yaml)
        .map_err(|e| MigrateError { msg: format!("pre-passed document does not parse: {}", e) })?;
    // A variables-only pack leaves an empty document after extraction — legal:
    // it compiles to a params-only Satz file.
    let top = match doc {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        _ => return err("top level is not a mapping"),
    };
    let mut header = pre.header.clone();
    header.push(String::new());
    header.push("Converted by `satz import` — interior comments were not carried.".to_string());
    if pre.import_include_seen {
        header.push("NEEDS ADOPTION: the source used `!import-include` (a transpile-time live import).".to_string());
        header.push("It is a plain `use` here; run `satz adopt <estate> --execute` after converting to import what already exists.".to_string());
    }
    convert_value(&top, kind_keyword, name, &pre.params, &header)
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

/// Print an already-parsed document as Satz: the printer behind `convert`,
/// for callers that hold plain data rather than dialect text — discovery
/// hands it a `Config` with no anchors, tags or includes. `params` are
/// `(name, already-rendered value)` pairs; `header` lines become leading
/// `//` comments (an empty line stays a blank comment line).
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
    let _ = writeln!(out, "{} {}\n", kind_keyword, snake(name));

    if !params.is_empty() {
        out.push_str("params {\n");
        for (name, value) in params {
            let _ = writeln!(out, "  {} = {}", name, value);
        }
        out.push_str("}\n\n");
    }

    let param_names: std::collections::HashSet<&str> =
        params.iter().map(|(n, _)| n.as_str()).collect();

    for (k, v) in top {
        // Form A use at top level
        if let Some(ks) = k.as_str() {
            if ks.starts_with(USE_K) {
                if let Some((_, path, cond)) = is_use_sentinel(v) {
                    emit_use(&mut out, 0, &path, &cond, None);
                    continue;
                }
            }
        }
        if let Some((form, path, cond)) = is_use_sentinel(v) {
            if form == "B" {
                let (key, is_ident) = key_expr(k)?;
                if !is_ident {
                    return err(format!("use ... as with non-identifier key {}", key));
                }
                emit_use(&mut out, 0, &path, &cond, Some(&key));
                continue;
            }
        }
        // Drop redundant top-level self-references (`x: *x`) — merge_variables
        // promotes params to the root anyway.
        if let (Some(ks), serde_yaml::Value::String(vs)) = (k.as_str(), v) {
            if let Some(refname) = as_ref_name(vs) {
                if snake(refname) == snake(ks) && param_names.contains(snake(ks).as_str()) {
                    continue;
                }
            }
        }
        let (key, is_ident) = key_expr(k)?;
        match v {
            serde_yaml::Value::Mapping(child) => {
                let _ = writeln!(out, "{} {{", key);
                emit_entries(child, &mut out, 2)?;
                out.push_str("}\n\n");
            }
            other => {
                let _ = is_ident;
                // fragment packs: top-level entry with a value (IAM member -> roles list,
                // scalar attrs) — legal Satz since top-level `key = value` landed
                let _ = writeln!(out, "{} = {}\n", key, value_expr(other, 0)?);
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repro_sva_logging_block() {
        // literal block as the FILE'S LAST content — trailing newline must survive
        let y = "variables:\n  sink-name: &sink-name \"s\"\n  org-id: &org-id \"1\"\nsec:\n  *sink-name:\n    org_id: *org-id\n    include_children: True\n    filter: |\n      lineA \"q\"\n      lineB:\"z\"\n";
        let s = convert(y, "pack", "t").unwrap();
        assert!(s.contains("lineA \\\"q\\\"\\nlineB:\\\"z\\\"\\n\""), "TRAILING LOST:\n{s}");
    }

    #[test]
    fn import_include_converts_to_use_with_an_adoption_note() {
        let y = "org_policy_policy: !import-include presets/cis.yaml\n!import-include presets/groups.yaml\n";
        let s = convert(y, "estate", "t").unwrap();
        assert!(s.contains("use \"presets/cis.yaml\" as org_policy_policy"), "{s}");
        assert!(s.contains("use \"presets/groups.yaml\""), "{s}");
        // the directive is gone from the body; only the note mentions it
        assert!(!s.lines().any(|l| !l.starts_with("//") && l.contains("import-include")), "{s}");
        assert!(s.contains("// NEEDS ADOPTION") && s.contains("satz adopt"), "{s}");
        // a source without the tag carries no note
        let s2 = convert("!include a.yaml\n", "estate", "t").unwrap();
        assert!(!s2.contains("NEEDS ADOPTION"), "{s2}");
    }

    #[test]
    fn variables_become_params_and_selfrefs_drop() {
        let y = "variables:\n  a-b: &a-b \"x\"\n  n: &n 400\na-b: *a-b\nsection:\n  item:\n    k: *a-b\n";
        let s = convert(y, "pack", "t").unwrap();
        assert!(s.contains("a_b = \"x\""), "{s}");
        assert!(s.contains("n = 400"), "{s}");
        assert!(!s.contains("\na_b ="), "self-ref dropped: {s}");
        assert!(s.contains("k = a_b"), "{s}");
    }

    #[test]
    fn format_inline_and_block_to_interpolation() {
        let y = "variables:\n  p: &p !format [\"{}-x\", *customer-shortname]\n  q: &q !format\n    - \"a{}b\"\n    - *p\nsection:\n  i:\n    d: !format [\"projects/{}/b\", *p]\n";
        let s = convert(y, "pack", "t").unwrap();
        assert!(s.contains("p = \"{customer_shortname}-x\""), "{s}");
        assert!(s.contains("q = \"a{p}b\""), "{s}");
        assert!(s.contains("d = \"projects/{p}/b\""), "{s}");
    }

    #[test]
    fn alias_keys_and_format_keys() {
        let y = "variables:\n  g: &g \"grp\"\nsection:\n  *g:\n    display_name: \"D\"\n  !format [\"group:{}\", *g]: [\"roles/x\"]\n";
        let s = convert(y, "pack", "t").unwrap();
        assert!(s.contains("\"{g}\" {"), "{s}");
        assert!(s.contains("\"group:{g}\" = ["), "{s}");
    }

    #[test]
    fn includes_both_forms_and_conditions() {
        let y = "key: !include a.yaml\n!include b.yaml\n!include-if cond-x c.yaml\nfolder:\n  f:\n    !include d.yaml\n";
        let s = convert(y, "estate", "t").unwrap();
        assert!(s.contains("use \"a.yaml\" as key"), "{s}");
        assert!(s.contains("\nuse \"b.yaml\"\n"), "{s}");
        assert!(s.contains("use \"c.yaml\" when cond_x"), "{s}");
        assert!(s.contains("    use \"d.yaml\""), "{s}");
    }

    #[test]
    fn literal_blocks_and_tf_refs_escape() {
        let y = "section:\n  i:\n    d: |\n      line1\n      line2\n    m: \"${a.b.c}\"\n";
        let s = convert(y, "pack", "t").unwrap();
        assert!(s.contains("d = \"line1\\nline2\\n\""), "{s}");
        assert!(s.contains("m = \"${{a.b.c}}\""), "{s}");
    }

    #[test]
    fn converted_output_parses_as_satz() {
        // The gate in miniature: converted output must parse as Satz (the full
        // compile is the yaml_estate_gate in the binary).
        let y = "variables:\n  v: &v \"x\"\nsection:\n  a:\n    k: *v\n    n: [1, 2]\n";
        let s = convert(y, "pack", "t").unwrap();
        let f = crate::satz::parse(&s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
        assert!(f.params.iter().any(|(n, _, _)| n == "v"), "{}", s);
        assert!(s.contains("k = v"), "{}", s);
    }
}

#[cfg(test)]
mod conversion_output_tests {
    //! The converter's own gate runs the LEGACY walk, which still accepts the
    //! YAML dialect's shorthand and can read `.yaml` packs. So it reported
    //! PROVEN on output that `transpile` refused. These pin the two rewrites
    //! that make a conversion produce valid Satz rather than plausible Satz.
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
    /// are real Terraform types, so the dialect's short spelling is rewritten
    /// like any other. Satz has no keyword resource types at all — the schema
    /// is the only authority on what a type is called.
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

    #[test]
    fn uses_are_repointed_at_converted_packs() {
        let src = "use \"cis.yaml\"\nuse \"not-converted.yaml\"\nuse \"other.satz\"\n";
        let out = retarget_uses(src, &|p| p == "cis.satz");
        assert!(out.contains("use \"cis.satz\""), "{}", out);
        assert!(out.contains("use \"not-converted.yaml\""), "{}", out);
        assert!(out.contains("use \"other.satz\""), "{}", out);
    }

    #[test]
    fn repointing_keeps_the_as_and_when_clauses() {
        let out = retarget_uses("  use \"cis.yaml\" as google_org_policy_policy\n", &|p| p == "cis.satz");
        assert_eq!(out, "  use \"cis.satz\" as google_org_policy_policy\n");
        let out = retarget_uses("use \"cis.yaml\" when flag\n", &|p| p == "cis.satz");
        assert_eq!(out, "use \"cis.satz\" when flag\n");
    }
}
