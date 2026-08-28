//! Satz — the pack/estate language. v0 front-end.
//!
//! "Satz": German for both *sentence* and *theorem* — a file is simultaneously a
//! statement of intent and a provable claim.
//!
//! v0 compiles Satz to the existing satz YAML dialect, deterministically, and the
//! proven pipeline does the rest. That is a deliberate strangler-fig move: authoring
//! drops the YAML layer now; the runtime drops it later when the typed IR replaces
//! the dialect underneath, with no surface change. What already dissolves at the
//! surface: anchors (params are lexically scoped declarations), the identity-!format
//! wrapper (aliasing is `a = b`), `!format` (string interpolation `"{param}"`),
//! textual override ordering (the emitter places params before uses).
//!
//! # Grammar (v0, line-oriented, brace-blocked)
//!
//! ```text
//! file        := { item }
//! item        := "estate" IDENT
//!              | "params" "{" { param } "}"
//!              | "use" STRING [ "as" IDENT ] [ "when" IDENT ]
//!              | block
//! param       := IDENT "=" value
//! block       := IDENT [ IDENT | STRING ] "{" { entry } "}"
//! entry       := IDENT "=" value            attribute
//!              | IDENT "{" { entry } "}"    nested mapping
//!              | IDENT IDENT "{" ... "}"    map entry: name -> body   (folder x {...})
//!              | IDENT STRING "{" ... "}"   interpolated-key map entry
//!              | STRING "=" value           interpolated key -> value (IAM grants)
//!              | STRING "{" { entry } "}"
//!              | "use" STRING [...]         include inside this mapping
//! value       := STRING | NUMBER | true | false | IDENT (param ref)
//!              | "[" [ value { "," value } [","] ] "]"
//!              | "{" { entry } "}"          object literal (in lists)
//! STRING      := "..." with {param} interpolation ({{ escapes a literal brace)
//!              | """...""" multi-line
//! comment     := "//" to end of line | "#" to end of line
//! ```
//!
//! Identifiers use snake_case; the emitter maps param identifiers to the YAML
//! dialect's kebab-case anchors (`logsink_project_name` <-> `logsink-project-name`),
//! which keeps migrated names identical to the original YAML packs' anchors. Resource
//! attribute names are 1:1 the Terraform provider names — the registry docs are the
//! docs.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(Vec<StrPart>),
    Num(String),
    Bool(bool),
    /// Bare identifier: a reference to a param.
    Ref(String),
    List(Vec<Value>),
    Obj(Vec<Entry>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// `{param}` interpolation.
    Param(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Ident(String),
    /// Interpolated string key.
    Str(Vec<StrPart>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Attr { key: Key, value: Value, line: usize },
    Map { key: Key, name: Option<Key>, body: Vec<Entry>, line: usize },
    Use { path: String, as_key: Option<String>, when: Option<String>, line: usize },
}

/// `suppress <tf_type> "<label>"` — estate-level subtractive override: remove a
/// pack-contributed resource (or, with `role`, one grant edge) after the fold.
/// A suppress that matches nothing is a hard error (stale config must surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Suppression {
    pub tf_type: String,
    /// Resource label / grant member — may interpolate params.
    pub label: Vec<StrPart>,
    /// Grant-edge form: remove only this role for the member.
    pub role: Option<Vec<StrPart>>,
    pub line: usize,
}

/// `hcl { … }` — raw Terraform/HCL passed through verbatim. Rust-`unsafe`-style:
/// it composes and deploys, but the proof layer cannot see inside it, so nothing
/// in here can carry a claim. Warns on every transpile unless the block states a
/// reason with `hcl trust "<why>" { … }`. Never interpolated — params reach raw
/// HCL as ordinary Terraform variables (`var.<param>`).
#[derive(Debug, Clone, PartialEq)]
pub struct HclBlock {
    /// Body between the outer braces, verbatim.
    pub body: String,
    /// `trust "<reason>"` — reviewed on purpose; downgrades the warning to a note.
    pub trust: Option<String>,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct File {
    pub estate: Option<String>,
    /// true when the header keyword was `pack` (estate and pack share the name slot)
    pub is_pack: bool,
    /// `pack <name> content` — forking into `<name>.local.satz` is the EXPECTED
    /// workflow (per-customer content); reporting tone only, mechanics are identical.
    pub content_mode: bool,
    /// `pack <name> version "1.2"` — the pack file's own revision, deliberately kept
    /// OUT of the filename (framework/standard versions live in claims and are
    /// orthogonal: multiple internal revisions may implement the same standard).
    pub version: Option<String>,
    pub params: Vec<(String, Value, usize)>,
    pub items: Vec<Entry>,
    pub claims: Vec<ClaimDecl>,
    pub suppressions: Vec<Suppression>,
    pub hcl_blocks: Vec<HclBlock>,
}

/// A control claim as language syntax:
/// `claim "cis-gcp" "4.0" "2.2" implements { resources = [...] duty "id" = "text" ... }`
/// Compiled to the pack's `.claims.yaml` sidecar — one source, sidecar generated.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimDecl {
    pub framework: String,
    pub version: String,
    pub control: String,
    /// `implements` | `contributes` | `deviates`
    ///
    /// `deviates` is a DELIBERATE, reasoned non-conformance: the estate knowingly
    /// does not meet this control (a policy declared but `enforce = "FALSE"`, a
    /// pristine resource `suppress`ed). It is a disclosed finding, not a gap —
    /// the whole point of a `.local` fork is that the customer had a reason, and
    /// the report must carry that reason instead of showing a hole that looks
    /// like an oversight.
    pub coverage: String,
    pub resources: Vec<String>,
    /// Required for `deviates`, rejected otherwise: why the deviation exists.
    pub reason: Option<String>,
    pub interpretation: Option<String>,
    pub duties: Vec<(String, String)>,
    pub line: usize,
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    /// Raw `hcl { … }` body plus its optional `trust` reason.
    Hcl(String, Option<String>),
    Str(Vec<StrPart>),
    Num(String),
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Eq,
    Comma,
}

#[derive(Debug)]
pub struct SatzError {
    pub line: usize,
    pub msg: String,
}
impl std::fmt::Display for SatzError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "satz: line {}: {}", self.line, self.msg)
    }
}
impl std::error::Error for SatzError {}

fn err<T>(line: usize, msg: impl Into<String>) -> Result<T, SatzError> {
    Err(SatzError { line, msg: msg.into() })
}

/// Raw-capture the body of `hcl { … }`: everything between the outer braces,
/// verbatim. Brace counting steps over quoted strings, comments and heredocs so
/// ordinary Terraform bodies survive unharmed. Returns (body, next_index, line).
fn scan_hcl_body(b: &[char], open_brace: usize, open_line: usize) -> Result<(String, usize, usize), SatzError> {
    let mut i = open_brace + 1;
    let mut line = open_line;
    let mut depth = 1usize;
    let mut out = String::new();
    while i < b.len() {
        match b[i] {
            '\n' => {
                line += 1;
                out.push('\n');
                i += 1;
            }
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    out.push(b[i]);
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    out.push(b[i]);
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                out.push_str("/*");
                i += 2;
                while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push_str("*/");
                    i += 2;
                }
            }
            '"' => {
                out.push('"');
                i += 1;
                while i < b.len() {
                    if b[i] == '\\' {
                        out.push('\\');
                        if let Some(&n) = b.get(i + 1) {
                            out.push(n);
                            if n == '\n' {
                                line += 1;
                            }
                        }
                        i += 2;
                        continue;
                    }
                    let ch = b[i];
                    out.push(ch);
                    i += 1;
                    if ch == '"' {
                        break;
                    }
                    if ch == '\n' {
                        line += 1;
                    }
                }
            }
            '<' if b.get(i + 1) == Some(&'<') => {
                out.push_str("<<");
                i += 2;
                if b.get(i) == Some(&'-') {
                    out.push('-');
                    i += 1;
                }
                let mut tag = String::new();
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                    tag.push(b[i]);
                    out.push(b[i]);
                    i += 1;
                }
                if tag.is_empty() {
                    continue;
                }
                while i < b.len() && b[i] != '\n' {
                    out.push(b[i]);
                    i += 1;
                }
                loop {
                    if i >= b.len() {
                        return err(line, format!("unterminated heredoc <<{} inside hcl block", tag));
                    }
                    out.push('\n');
                    line += 1;
                    i += 1;
                    let start = i;
                    while i < b.len() && b[i] != '\n' {
                        i += 1;
                    }
                    let text: String = b[start..i].iter().collect();
                    out.push_str(&text);
                    if text.trim() == tag {
                        break;
                    }
                }
            }
            '{' => {
                depth += 1;
                out.push('{');
                i += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((out, i + 1, line));
                }
                out.push('}');
                i += 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    err(open_line, "unterminated hcl { … } block")
}

/// A lexed `hcl { … }` block plus where the lexer resumes.
struct LexedHcl {
    body: String,
    trust: Option<String>,
    next: usize,
    line: usize,
}

/// After the `hcl` keyword: optional `trust "<reason>"`, then the raw body.
/// Returns None (consuming nothing) when `hcl` is not followed by a block, so the
/// word stays usable as an ordinary identifier.
fn try_lex_hcl(b: &[char], after_kw: usize, line: usize) -> Result<Option<LexedHcl>, SatzError> {
    let mut i = after_kw;
    let mut ln = line;
    let skip_ws = |i: &mut usize, ln: &mut usize| {
        while *i < b.len() {
            match b[*i] {
                '\n' => {
                    *ln += 1;
                    *i += 1;
                }
                ' ' | '\t' | '\r' => *i += 1,
                '#' => {
                    while *i < b.len() && b[*i] != '\n' {
                        *i += 1;
                    }
                }
                '/' if b.get(*i + 1) == Some(&'/') => {
                    while *i < b.len() && b[*i] != '\n' {
                        *i += 1;
                    }
                }
                _ => break,
            }
        }
    };
    skip_ws(&mut i, &mut ln);

    let mut trust = None;
    if b[i..].starts_with(&['t', 'r', 'u', 's', 't']) {
        let after = i + 5;
        let boundary = b.get(after).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != '_');
        if boundary {
            i = after;
            skip_ws(&mut i, &mut ln);
            if b.get(i) != Some(&'"') {
                return err(ln, "hcl trust: expected a quoted reason, e.g. hcl trust \"reviewed 2026-08\" { … }");
            }
            i += 1;
            let mut reason = String::new();
            while i < b.len() && b[i] != '"' {
                if b[i] == '\n' {
                    return err(ln, "hcl trust: newline in reason string");
                }
                reason.push(b[i]);
                i += 1;
            }
            if i >= b.len() {
                return err(ln, "hcl trust: unterminated reason string");
            }
            i += 1;
            trust = Some(reason);
            skip_ws(&mut i, &mut ln);
        }
    }

    if b.get(i) != Some(&'{') {
        // Not a passthrough block (and `trust` was not consumed unless we matched it).
        if trust.is_some() {
            return err(ln, "hcl trust \"…\": expected '{' to open the block");
        }
        return Ok(None);
    }
    let (body, next, end_line) = scan_hcl_body(b, i, ln)?;
    Ok(Some(LexedHcl { body, trust, next, line: end_line }))
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, SatzError> {
    let mut toks = Vec::new();
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '{' => {
                toks.push((Tok::LBrace, line));
                i += 1;
            }
            '}' => {
                toks.push((Tok::RBrace, line));
                i += 1;
            }
            '[' => {
                toks.push((Tok::LBrack, line));
                i += 1;
            }
            ']' => {
                toks.push((Tok::RBrack, line));
                i += 1;
            }
            '=' => {
                toks.push((Tok::Eq, line));
                i += 1;
            }
            ',' => {
                toks.push((Tok::Comma, line));
                i += 1;
            }
            '"' => {
                // Triple-quoted multi-line or normal string; both interpolate {param}.
                let triple = b.get(i + 1) == Some(&'"') && b.get(i + 2) == Some(&'"');
                let start_line = line;
                i += if triple { 3 } else { 1 };
                let mut parts = Vec::new();
                let mut lit = String::new();
                loop {
                    if i >= b.len() {
                        return err(start_line, "unterminated string");
                    }
                    let done = if triple {
                        b[i] == '"' && b.get(i + 1) == Some(&'"') && b.get(i + 2) == Some(&'"')
                    } else {
                        b[i] == '"'
                    };
                    if done {
                        i += if triple { 3 } else { 1 };
                        break;
                    }
                    match b[i] {
                        '\n' => {
                            line += 1;
                            if !triple {
                                return err(start_line, "newline in single-line string (use \"\"\" for multi-line)");
                            }
                            lit.push('\n');
                            i += 1;
                        }
                        '{' if b.get(i + 1) == Some(&'{') => {
                            lit.push('{');
                            i += 2;
                        }
                        '}' if b.get(i + 1) == Some(&'}') => {
                            lit.push('}');
                            i += 2;
                        }
                        '{' => {
                            // interpolation
                            if !lit.is_empty() {
                                parts.push(StrPart::Lit(std::mem::take(&mut lit)));
                            }
                            i += 1;
                            let mut name = String::new();
                            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                                name.push(b[i]);
                                i += 1;
                            }
                            if b.get(i) != Some(&'}') {
                                return err(line, format!("unterminated interpolation '{{{}'", name));
                            }
                            if name.is_empty() {
                                return err(line, "empty interpolation {} (use {{}} for a literal brace)");
                            }
                            i += 1;
                            parts.push(StrPart::Param(name));
                        }
                        '\\' if !triple => {
                            // minimal escapes in single-line strings
                            match b.get(i + 1) {
                                Some('n') => lit.push('\n'),
                                Some('"') => lit.push('"'),
                                Some('\\') => lit.push('\\'),
                                other => return err(line, format!("unknown escape \\{:?}", other)),
                            }
                            i += 2;
                        }
                        ch => {
                            lit.push(ch);
                            i += 1;
                        }
                    }
                }
                if !lit.is_empty() || parts.is_empty() {
                    parts.push(StrPart::Lit(lit));
                }
                toks.push((Tok::Str(parts), start_line));
            }
            c if c.is_ascii_digit() || (c == '-' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let mut n = String::new();
                n.push(c);
                i += 1;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                    n.push(b[i]);
                    i += 1;
                }
                toks.push((Tok::Num(n), line));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut id = String::new();
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_' || b[i] == '.') {
                    id.push(b[i]);
                    i += 1;
                }
                let _ = c;
                if id == "hcl" {
                    if let Some(h) = try_lex_hcl(&b, i, line)? {
                        toks.push((Tok::Hcl(h.body, h.trust), line));
                        i = h.next;
                        line = h.line;
                        continue;
                    }
                }
                toks.push((Tok::Ident(id), line));
            }
            other => return err(line, format!("unexpected character '{}'", other)),
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------------------

struct P {
    toks: Vec<(Tok, usize)>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|(t, _)| t)
    }
    fn line(&self) -> usize {
        self.toks.get(self.i).map(|(_, l)| *l).unwrap_or_else(|| {
            self.toks.last().map(|(_, l)| *l).unwrap_or(1)
        })
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t.map(|(t, _)| t)
    }
    fn expect(&mut self, want: Tok, what: &str) -> Result<(), SatzError> {
        let line = self.line();
        match self.next() {
            Some(t) if t == want => Ok(()),
            other => err(line, format!("expected {} but found {:?}", what, other)),
        }
    }

    fn value(&mut self) -> Result<Value, SatzError> {
        let line = self.line();
        match self.next() {
            Some(Tok::Str(parts)) => Ok(Value::Str(parts)),
            Some(Tok::Num(n)) => Ok(Value::Num(n)),
            Some(Tok::Ident(id)) if id == "true" => Ok(Value::Bool(true)),
            Some(Tok::Ident(id)) if id == "false" => Ok(Value::Bool(false)),
            Some(Tok::Ident(id)) => Ok(Value::Ref(id)),
            Some(Tok::LBrack) => {
                let mut items = Vec::new();
                loop {
                    if self.peek() == Some(&Tok::RBrack) {
                        self.next();
                        break;
                    }
                    items.push(self.value()?);
                    match self.peek() {
                        Some(Tok::Comma) => {
                            self.next();
                        }
                        Some(Tok::RBrack) => {}
                        _ => {} // newline-separated is fine: no comma required
                    }
                }
                Ok(Value::List(items))
            }
            Some(Tok::LBrace) => {
                let body = self.entries()?;
                Ok(Value::Obj(body))
            }
            other => err(line, format!("expected a value, found {:?}", other)),
        }
    }

    /// Entries until the matching `}` (consumed).
    fn entries(&mut self) -> Result<Vec<Entry>, SatzError> {
        let mut out = Vec::new();
        loop {
            let line = self.line();
            match self.peek() {
                None => return err(line, "unexpected end of file: missing '}'"),
                Some(Tok::RBrace) => {
                    self.next();
                    return Ok(out);
                }
                Some(Tok::Ident(id)) if id == "use" => {
                    self.next();
                    out.push(self.use_stmt(line)?);
                }
                Some(Tok::Ident(_)) | Some(Tok::Str(_)) => {
                    let key = match self.next().unwrap() {
                        Tok::Ident(id) => Key::Ident(id),
                        Tok::Str(s) => Key::Str(s),
                        _ => unreachable!(),
                    };
                    match self.peek() {
                        Some(Tok::Eq) => {
                            self.next();
                            let value = self.value()?;
                            out.push(Entry::Attr { key, value, line });
                        }
                        Some(Tok::LBrace) => {
                            self.next();
                            let body = self.entries()?;
                            out.push(Entry::Map { key, name: None, body, line });
                        }
                        Some(Tok::Ident(_)) | Some(Tok::Str(_)) => {
                            let name = match self.next().unwrap() {
                                Tok::Ident(id) => Key::Ident(id),
                                Tok::Str(s) => Key::Str(s),
                                _ => unreachable!(),
                            };
                            self.expect(Tok::LBrace, "'{' after map entry name")?;
                            let body = self.entries()?;
                            out.push(Entry::Map { key, name: Some(name), body, line });
                        }
                        other => {
                            return err(line, format!("after key: expected '=', '{{' or a name, found {:?}", other))
                        }
                    }
                }
                Some(other) => return err(line, format!("unexpected {:?} in block", other)),
            }
        }
    }

    fn claim_stmt(&mut self, line: usize) -> Result<ClaimDecl, SatzError> {
        let take_str = |what: &str, p: &mut P| -> Result<String, SatzError> {
            match p.next() {
                Some(Tok::Str(parts)) => match parts.as_slice() {
                    [StrPart::Lit(v)] => Ok(v.clone()),
                    _ => err(line, format!("claim {}: no interpolation allowed", what)),
                },
                other => err(line, format!("claim: expected {} string, found {:?}", what, other)),
            }
        };
        let framework = take_str("framework", self)?;
        let version = take_str("version", self)?;
        let control = take_str("control", self)?;
        let coverage = match self.next() {
            Some(Tok::Ident(c)) if c == "implements" || c == "contributes" || c == "deviates" => c,
            other => {
                return err(line, format!("claim: expected implements|contributes|deviates, found {:?}", other))
            }
        };
        self.expect(Tok::LBrace, "'{' after claim header")?;
        let body = self.entries()?;
        let mut decl = ClaimDecl {
            framework, version, control, coverage,
            resources: Vec::new(), reason: None, interpretation: None, duties: Vec::new(), line,
        };
        for e in body {
            match e {
                Entry::Attr { key: Key::Ident(k), value: Value::List(items), .. } if k == "resources" => {
                    for it in items {
                        match it {
                            Value::Str(parts) => match parts.as_slice() {
                                [StrPart::Lit(r)] => decl.resources.push(r.clone()),
                                _ => return err(line, "claim resources: no interpolation allowed (addresses are static)"),
                            },
                            _ => return err(line, "claim resources: expected strings"),
                        }
                    }
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "reason" => {
                    let lit: String = parts.iter().map(|p| match p {
                        StrPart::Lit(s) => s.as_str(), _ => "",
                    }).collect();
                    decl.reason = Some(lit);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "interpretation" => {
                    let lit: String = parts.iter().map(|p| match p {
                        StrPart::Lit(s) => s.as_str(), _ => "",
                    }).collect();
                    decl.interpretation = Some(lit);
                }
                Entry::Map { key: Key::Ident(k), name: Some(Key::Str(idp)), body, .. } if k == "duty" => {
                    // rare form duty "id" { text = "..." } — accept but prefer attr form
                    let _ = (idp, body);
                    return err(line, "duty: use `duty \"id\" = \"text\"`");
                }
                Entry::Attr { key: Key::Str(_), .. } => {
                    return err(line, "claim: unexpected string key (did you mean `duty \"id\" = \"text\"`? prefix with duty)")
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k.starts_with("duty_") || k == "duty" => {
                    let _ = l;
                    let text: String = parts.iter().map(|p| match p { StrPart::Lit(s) => s.as_str(), _ => "" }).collect();
                    decl.duties.push((k.trim_start_matches("duty_").replace('_', "-"), text));
                }
                other => return err(line, format!("claim: unexpected entry {:?}", other)),
            }
        }
        if decl.coverage == "deviates" {
            if decl.reason.is_none() {
                return err(line, "claim … deviates: reason = \"…\" is required (a deviation is a disclosed decision, and the report carries the reason)");
            }
        } else {
            if decl.reason.is_some() {
                return err(line, "claim: reason = \"…\" belongs to a `deviates` claim; use interpretation for the others");
            }
            // A positive claim must ship its witnesses. A deviation need not: the
            // resource may be present-but-not-enforcing (witnessed) OR absent
            // because the estate suppressed it (nothing to witness).
            if decl.resources.is_empty() {
                return err(line, "claim: resources = [...] is required (a claim ships its witnesses)");
            }
        }
        Ok(decl)
    }

    fn use_stmt(&mut self, line: usize) -> Result<Entry, SatzError> {
        let path = match self.next() {
            Some(Tok::Str(parts)) => match parts.as_slice() {
                [StrPart::Lit(p)] => p.clone(),
                _ => return err(line, "use: path must be a plain string (no interpolation)"),
            },
            other => return err(line, format!("use: expected a path string, found {:?}", other)),
        };
        let mut as_key = None;
        let mut when = None;
        loop {
            match self.peek() {
                Some(Tok::Ident(id)) if id == "as" => {
                    self.next();
                    match self.next() {
                        Some(Tok::Ident(k)) => as_key = Some(k),
                        other => return err(line, format!("use ... as: expected identifier, found {:?}", other)),
                    }
                }
                Some(Tok::Ident(id)) if id == "when" => {
                    self.next();
                    match self.next() {
                        Some(Tok::Ident(p)) => when = Some(p),
                        other => return err(line, format!("use ... when: expected param name, found {:?}", other)),
                    }
                }
                _ => break,
            }
        }
        Ok(Entry::Use { path, as_key, when, line })
    }
}

pub fn parse(src: &str) -> Result<File, SatzError> {
    let toks = lex(src)?;
    let mut p = P { toks, i: 0 };
    let mut file = File::default();
    loop {
        let line = p.line();
        match p.peek() {
            None => break,
            Some(Tok::Ident(id)) if id == "estate" || id == "pack" => {
                let is_pack = id == "pack";
                p.next();
                match p.next() {
                    Some(Tok::Ident(name)) => {
                        file.estate = Some(name);
                        file.is_pack = is_pack;
                    }
                    other => return err(line, format!("estate: expected a name, found {:?}", other)),
                }
                loop {
                    match p.peek() {
                        Some(Tok::Ident(m)) if m == "content" => {
                            p.next();
                            file.content_mode = true;
                        }
                        Some(Tok::Ident(m)) if m == "version" => {
                            p.next();
                            match p.next() {
                                Some(Tok::Str(parts)) => match parts.as_slice() {
                                    [StrPart::Lit(v)] => file.version = Some(v.clone()),
                                    _ => return err(line, "version: plain string required"),
                                },
                                other => return err(line, format!("version: expected string, found {:?}", other)),
                            }
                        }
                        _ => break,
                    }
                }
            }
            Some(Tok::Ident(id)) if id == "params" => {
                p.next();
                p.expect(Tok::LBrace, "'{' after params")?;
                loop {
                    let line = p.line();
                    match p.peek() {
                        Some(Tok::RBrace) => {
                            p.next();
                            break;
                        }
                        Some(Tok::Ident(_)) => {
                            let name = match p.next().unwrap() {
                                Tok::Ident(n) => n,
                                _ => unreachable!(),
                            };
                            p.expect(Tok::Eq, "'=' in param")?;
                            let v = p.value()?;
                            file.params.push((name, v, line));
                        }
                        other => return err(line, format!("params: expected name or '}}', found {:?}", other)),
                    }
                }
            }
            Some(Tok::Ident(id)) if id == "use" => {
                p.next();
                let u = p.use_stmt(line)?;
                file.items.push(u);
            }
            Some(Tok::Hcl(..)) => {
                let (body, trust) = match p.next().unwrap() {
                    Tok::Hcl(b, t) => (b, t),
                    _ => unreachable!(),
                };
                file.hcl_blocks.push(HclBlock { body, trust, line });
            }
            Some(Tok::Ident(id)) if id == "claim" => {
                p.next();
                file.claims.push(p.claim_stmt(line)?);
            }
            Some(Tok::Ident(id)) if id == "suppress" => {
                p.next();
                let tf_type = match p.next() {
                    Some(Tok::Ident(t)) => t,
                    other => return err(line, format!("suppress: expected a resource type, found {:?}", other)),
                };
                let label = match p.next() {
                    Some(Tok::Str(parts)) => parts,
                    other => return err(line, format!("suppress: expected a quoted name, found {:?}", other)),
                };
                let role = match p.peek() {
                    Some(Tok::Ident(r)) if r == "role" => {
                        p.next();
                        match p.next() {
                            Some(Tok::Str(parts)) => Some(parts),
                            other => return err(line, format!("suppress … role: expected a quoted role, found {:?}", other)),
                        }
                    }
                    _ => None,
                };
                file.suppressions.push(Suppression { tf_type, label, role, line });
            }
            // Top-level items: identifier-keyed blocks, and — for fragment packs whose
            // top level is an entry map (CIS constraints, alias-named groups, IAM
            // member grants) — string/interpolated keys with a block or `= value`.
            Some(Tok::Ident(_)) | Some(Tok::Str(_)) => {
                let key = match p.next().unwrap() {
                    Tok::Ident(id) => Key::Ident(id),
                    Tok::Str(parts) => Key::Str(parts),
                    _ => unreachable!(),
                };
                match p.peek() {
                    Some(Tok::LBrace) => {
                        p.next();
                        let body = p.entries()?;
                        file.items.push(Entry::Map { key, name: None, body, line });
                    }
                    Some(Tok::Eq) => {
                        p.next();
                        let value = p.value()?;
                        file.items.push(Entry::Attr { key, value, line });
                    }
                    Some(Tok::Ident(_)) | Some(Tok::Str(_)) => {
                        let name = match p.next().unwrap() {
                            Tok::Ident(id) => Key::Ident(id),
                            Tok::Str(s) => Key::Str(s),
                            _ => unreachable!(),
                        };
                        p.expect(Tok::LBrace, "'{' after block name")?;
                        let body = p.entries()?;
                        file.items.push(Entry::Map { key, name: Some(name), body, line });
                    }
                    other => return err(line, format!("top-level: expected '{{', '=' or name after key, found {:?}", other)),
                }
            }
            Some(other) => return err(line, format!("unexpected {:?} at top level", other)),
        }
    }
    Ok(file)
}

// ---------------------------------------------------------------------------
// Emitter: Satz AST -> the existing YAML dialect
// ---------------------------------------------------------------------------

/// snake_case param identifier -> the dialect's kebab-case anchor name.
fn anchor(name: &str) -> String {
    name.replace('_', "-")
}

fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

fn emit_value(v: &Value, out: &mut String, indent: usize) {
    match v {
        Value::Str(parts) => emit_str(parts, out),
        Value::Num(n) => out.push_str(n),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Ref(id) => {
            out.push('*');
            out.push_str(&anchor(id));
        }
        Value::List(items) if items.is_empty() => {
            // A declared-but-empty list is a VALUE ("no entries"), not an absent
            // key. Emitting nothing made the anchor resolve to null, so a param
            // like `allowed_policy_member_subjects = []` reached the dialect as
            // null and vanished from the resulting spec — which made
            // diff-organizational-policies report a false difference against a
            // live policy that already matched.
            out.push_str("[]");
        }
        Value::Obj(body) if body.is_empty() => out.push_str("{}"),
        Value::List(items) => {
            for item in items {
                out.push('\n');
                out.push_str(&" ".repeat(indent + 2));
                out.push_str("- ");
                match item {
                    Value::Obj(body) => {
                        // object in list: entries indented under the dash
                        let mut sub = String::new();
                        emit_entries(body, &mut sub, indent + 4);
                        // first line goes right after "- "
                        let sub = sub.trim_start_matches('\n');
                        let mut lines = sub.lines();
                        if let Some(first) = lines.next() {
                            out.push_str(first.trim_start());
                        }
                        for l in lines {
                            out.push('\n');
                            out.push_str(l);
                        }
                    }
                    other => emit_value(other, out, indent + 2),
                }
            }
        }
        Value::Obj(body) => {
            emit_entries(body, out, indent + 2);
        }
    }
}

fn emit_str(parts: &[StrPart], out: &mut String) {
    let has_interp = parts.iter().any(|p| matches!(p, StrPart::Param(_)));
    if !has_interp {
        let lit: String = parts
            .iter()
            .map(|p| match p {
                StrPart::Lit(s) => s.as_str(),
                _ => unreachable!(),
            })
            .collect();
        out.push_str(&yaml_quote(&lit));
        return;
    }
    // Single pure param interpolation "{x}" -> plain alias (identity), else !format.
    if let [StrPart::Param(p)] = parts {
        out.push_str(&format!("!format [\"{{}}\", *{}]", anchor(p)));
        return;
    }
    let mut template = String::new();
    let mut args = Vec::new();
    for p in parts {
        match p {
            StrPart::Lit(s) => {
                // The dialect's !format knows ONLY `{}` as a placeholder — lone braces
                // are literals and there is no escape syntax. A literal `{}` pair in
                // the text is represented as a placeholder fed a literal-"{}" argument.
                let mut rest = s.as_str();
                while let Some(idx) = rest.find("{}") {
                    template.push_str(&rest[..idx]);
                    template.push_str("{}");
                    args.push("\"{}\"".to_string());
                    rest = &rest[idx + 2..];
                }
                template.push_str(rest);
            }
            StrPart::Param(name) => {
                template.push_str("{}");
                args.push(format!("*{}", anchor(name)));
            }
        }
    }
    out.push_str(&format!("!format [{}, {}]", yaml_quote(&template), args.join(", ")));
}

fn emit_key(k: &Key, out: &mut String) {
    match k {
        Key::Ident(id) => out.push_str(id),
        Key::Str(parts) => {
            let mut s = String::new();
            emit_str(parts, &mut s);
            // A !format key must be written as a complex key? The dialect accepts
            // tagged scalars as keys directly (the existing presets do it).
            out.push_str(&s);
        }
    }
}

fn emit_entries(entries: &[Entry], out: &mut String, indent: usize) {
    let pad = " ".repeat(indent);
    for e in entries {
        match e {
            Entry::Attr { key, value, .. } => {
                out.push('\n');
                out.push_str(&pad);
                emit_key(key, out);
                out.push_str(": ");
                match value {
                    Value::List(_) | Value::Obj(_) => {
                        // block-style: colon then indented content
                        let mut sub = String::new();
                        emit_value(value, &mut sub, indent);
                        out.push_str(sub.trim_end());
                    }
                    scalar => emit_value(scalar, out, indent),
                }
            }
            Entry::Map { key, name, body, .. } => {
                out.push('\n');
                out.push_str(&pad);
                emit_key(key, out);
                out.push(':');
                match name {
                    // An EMPTY block must round-trip as an empty mapping, not as a
                    // null: `auto {}` means "automatic replication", and `auto:`
                    // (null) is a different thing that the walk drops entirely.
                    None if body.is_empty() => out.push_str(" {}"),
                    None => emit_entries(body, out, indent + 2),
                    Some(n) => {
                        out.push('\n');
                        out.push_str(&" ".repeat(indent + 2));
                        emit_key(n, out);
                        out.push(':');
                        if body.is_empty() {
                            out.push_str(" {}");
                        } else {
                            emit_entries(body, out, indent + 4);
                        }
                    }
                }
            }
            Entry::Use { path, as_key, when, .. } => {
                out.push('\n');
                out.push_str(&pad);
                // A .satz pack is compiled to its .gen.yaml sibling by the driver; the
                // emitted include points there so the include machinery (and
                // first-definition-wins over the pack's params) works unchanged.
                let inc_path = if path.ends_with(".satz") {
                    format!("{}.gen.yaml", path.trim_end_matches(".satz"))
                } else {
                    path.clone()
                };
                let directive = match when {
                    Some(p) => format!("!include-if {} {}", anchor(p), inc_path),
                    None => format!("!include {}", inc_path),
                };
                match as_key {
                    Some(k) => {
                        out.push_str(k);
                        out.push_str(": ");
                        out.push_str(&directive);
                    }
                    None => out.push_str(&directive),
                }
            }
        }
    }
}


/// Params in dependency order (stable Kahn topological sort): a param may reference
/// any other param regardless of declaration order — the emitter linearizes so YAML's
/// backward-only aliases always resolve. Cycles fall back to source order and are
/// caught by the YAML parse with its line context.
pub(crate) fn sort_params_by_deps(
    params: &[(String, Value, usize)],
) -> Vec<&(String, Value, usize)> {
    fn deps_of(v: &Value, names: &std::collections::HashSet<&str>, out: &mut Vec<String>) {
        match v {
            Value::Ref(r) if names.contains(r.as_str()) => out.push(r.clone()),
            Value::Str(parts) => {
                for p in parts {
                    if let StrPart::Param(n) = p {
                        if names.contains(n.as_str()) {
                            out.push(n.clone());
                        }
                    }
                }
            }
            Value::List(items) => items.iter().for_each(|i| deps_of(i, names, out)),
            Value::Obj(entries) => {
                for e in entries {
                    if let Entry::Attr { value, .. } = e {
                        deps_of(value, names, out);
                    }
                }
            }
            _ => {}
        }
    }
    let names: std::collections::HashSet<&str> =
        params.iter().map(|(n, _, _)| n.as_str()).collect();
    let mut emitted: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut out: Vec<&(String, Value, usize)> = Vec::new();
    let mut remaining: Vec<&(String, Value, usize)> = params.iter().collect();
    while !remaining.is_empty() {
        let before = out.len();
        remaining.retain(|p| {
            let mut d = Vec::new();
            deps_of(&p.1, &names, &mut d);
            if d.iter().all(|n| emitted.contains(n.as_str()) || n == &p.0) {
                emitted.insert(p.0.as_str());
                out.push(p);
                false
            } else {
                true
            }
        });
        if out.len() == before {
            // cycle: keep source order for the rest, let YAML report it with context
            out.append(&mut remaining);
        }
    }
    out
}

/// Compile a parsed Satz file to the YAML dialect. Params become the `variables:`
/// block (anchors in emission order, so derived params may reference earlier ones);
/// items follow in source order.
pub fn compile_to_yaml(file: &File) -> String {
    let mut out = String::new();
    if let Some(name) = &file.estate {
        let _ = writeln!(out, "# generated by satz from estate '{}' — do not edit", name);
        if file.content_mode {
            let _ = writeln!(out, "# satz-mode: content");
        }
        if let Some(v) = &file.version {
            let _ = writeln!(out, "# satz-pack-version: {}", v);
        }
    }
    if !file.params.is_empty() {
        out.push_str("variables:");
        for (name, v, _) in sort_params_by_deps(&file.params) {
            out.push('\n');
            out.push_str("  ");
            let a = anchor(name);
            out.push_str(&a);
            out.push_str(": &");
            out.push_str(&a);
            out.push(' ');
            // An anchored definition whose value is another param must not emit the
            // (illegal) anchor-on-alias form — the identity !format wrapper carries it.
            match v {
                Value::Ref(other) => {
                    out.push_str(&format!("!format [\"{{}}\", *{}]", anchor(other)))
                }
                other => emit_value(other, &mut out, 2),
            }
        }
        out.push('\n');
    }
    // Merge same-key top-level Maps (two `folder` blocks are one folder: mapping).
    // Emission is in first-occurrence order; bodies concatenate.
    let mut order: Vec<&Key> = Vec::new();
    let mut merged: Vec<(usize, Vec<&Entry>)> = Vec::new();
    let mut uses: Vec<&Entry> = Vec::new();
    for item in &file.items {
        match item {
            Entry::Use { .. } => uses.push(item),
            Entry::Map { key, .. } => {
                let idx = order.iter().position(|k| *k == key);
                match idx {
                    Some(i) => merged[i].1.push(item),
                    None => {
                        order.push(key);
                        merged.push((order.len() - 1, vec![item]));
                    }
                }
            }
            Entry::Attr { .. } => {
                // top-level scalar attrs (customer-organization-id etc.)
                uses.push(item);
            }
        }
    }
    // top-level attrs and root-level uses first (they define/include at root)…
    for u in &uses {
        emit_entries(std::slice::from_ref(*u), &mut out, 0);
    }
    // …then the merged maps.
    for (oi, items) in &merged {
        let key = order[*oi];
        out.push('\n');
        emit_key(key, &mut out);
        out.push(':');
        for item in items {
            if let Entry::Map { name, body, .. } = item {
                match name {
                    None => emit_entries(body, &mut out, 2),
                    Some(n) => {
                        out.push('\n');
                        out.push_str("  ");
                        emit_key(n, &mut out);
                        out.push(':');
                        emit_entries(body, &mut out, 4);
                    }
                }
            }
        }
    }
    out.push('\n');
    out
}

/// Result of compiling one .satz file.
pub struct Compiled {
    /// True when the source declares `suppress` statements — the YAML dialect
    /// cannot express them, so the legacy pipeline must refuse the file.
    pub has_suppressions: bool,
    /// True when the source declares `hcl { … }` passthrough blocks — the YAML
    /// dialect cannot carry raw HCL, so the legacy pipeline must refuse the file.
    pub has_hcl: bool,
    pub yaml: String,
    /// `use`-referenced .satz paths (as written) that the driver must compile too.
    pub satz_deps: Vec<String>,
    /// Generated claims sidecar (YAML), if the file declares claims.
    pub claims_yaml: Option<String>,
}

fn collect_satz_deps(entries: &[Entry], out: &mut Vec<String>) {
    for e in entries {
        match e {
            Entry::Use { path, .. } if path.ends_with(".satz") => out.push(path.clone()),
            Entry::Map { body, .. } => collect_satz_deps(body, out),
            _ => {}
        }
    }
}

fn emit_claims_sidecar(file: &File) -> Option<String> {
    if file.claims.is_empty() {
        return None;
    }
    let pack = file.estate.clone().unwrap_or_default();
    let mut y = String::new();
    let _ = writeln!(y, "# generated by satz from pack '{}' — do not edit", pack);
    let _ = writeln!(y, "pack: {}", pack);
    let _ = writeln!(y, "version: \"{}\"", file.version.as_deref().unwrap_or("1.0"));
    y.push_str("claims:\n");
    for c in &file.claims {
        let _ = writeln!(y, "  - framework: {}", c.framework);
        let _ = writeln!(y, "    framework-version: \"{}\"", c.version);
        let _ = writeln!(y, "    control: \"{}\"", c.control);
        let _ = writeln!(y, "    coverage: {}", c.coverage);
        if let Some(reason) = &c.reason {
            let _ = writeln!(y, "    reason: {}", yaml_quote(reason));
        }
        y.push_str("    resources:\n");
        for r in &c.resources {
            let _ = writeln!(y, "      - {}", r);
        }
        if !c.duties.is_empty() {
            y.push_str("    manual-duties:\n");
            for (id, text) in &c.duties {
                let _ = writeln!(y, "      - id: {}", id);
                let _ = writeln!(y, "        duty: {}", yaml_quote(text));
            }
        }
        if let Some(interp) = &c.interpretation {
            let _ = writeln!(y, "    interpretation: {}", yaml_quote(interp));
        }
    }
    Some(y)
}

/// Compile a source file: YAML dialect text, .satz deps to compile next, and the
/// claims sidecar when the file declares claims.
pub fn compile(src: &str) -> Result<Compiled, SatzError> {
    let file = parse(src)?;
    let mut satz_deps = Vec::new();
    collect_satz_deps(&file.items, &mut satz_deps);
    Ok(Compiled {
        has_suppressions: !file.suppressions.is_empty(),
        has_hcl: !file.hcl_blocks.is_empty(),
        yaml: compile_to_yaml(&file),
        satz_deps,
        claims_yaml: emit_claims_sidecar(&file),
    })
}

/// Parse + compile in one step (yaml only).
pub fn satz_to_yaml(src: &str) -> Result<String, SatzError> {
    Ok(compile(src)?.yaml)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn long_string_param_with_escaped_quotes_round_trips() {
        let src = "pack demo version \"1.1\"\n\nparams {\n  logsink_filter = \"log_id(\\\"cloudaudit.googleapis.com/activity\\\") OR log_id(\\\"cloudaudit.googleapis.com/policy\\\")\"\n}\n\ngoogle_logging_organization_sink {\n  s {\n    filter = logsink_filter\n  }\n}\n";
        let out = compile(src).expect("compile");
        let doc: serde_yaml::Value = serde_yaml::from_str(&out.yaml).expect("twin parses");
        let filter = doc["google_logging_organization_sink"]["s"]["filter"].clone();
        // alias resolves to the anchored param value, quotes intact
        let s = serde_yaml::to_string(&filter).unwrap();
        assert!(s.contains("log_id(\"cloudaudit.googleapis.com/activity\")"), "{}", s);
    }

    #[test]
    fn pack_header_version_and_content_any_order() {
        for src in [
            "pack demo version \"1.0\" content\n\nvariables {\n  a = \"1\"\n}\n",
            "pack demo content version \"1.0\"\n\nvariables {\n  a = \"1\"\n}\n",
        ] {
            let f = parse(src).expect(src);
            assert_eq!(f.version.as_deref(), Some("1.0"), "{}", src);
            assert!(f.content_mode, "{}", src);
        }
        let f = parse("pack demo version \"2.1\"\n").unwrap();
        assert_eq!(f.version.as_deref(), Some("2.1"));
        assert!(!f.content_mode);
    }

    use super::*;

    #[test]
    fn params_become_variables_with_kebab_anchors() {
        let y = satz_to_yaml(
            r#"
estate demo
params {
  customer_shortname = "abc"
  logsink_bucket_name = "{customer_shortname}-audit-logs"
}
"#,
        )
        .unwrap();
        assert!(y.contains("customer-shortname: &customer-shortname \"abc\""), "{y}");
        assert!(
            y.contains("logsink-bucket-name: &logsink-bucket-name !format [\"{}-audit-logs\", *customer-shortname]"),
            "{y}"
        );
    }

    /// `a = b` between params is the aliasing case that YAML forbids as `&a *b`;
    /// the identity wrapper carries it — and forward references are legal because
    /// the emitter linearizes params by dependency, not by declaration order.
    #[test]
    fn ref_params_use_identity_wrapper_and_forward_refs_resolve() {
        let y = satz_to_yaml(
            r#"
params {
  alerts_project = logsink_project
  logsink_project = "abc-logs-001"
}
"#,
        )
        .unwrap();
        assert!(
            y.contains("alerts-project: &alerts-project !format [\"{}\", *logsink-project]"),
            "{y}"
        );
        // dependency order: logsink-project must be defined first
        let li = y.find("logsink-project: &").unwrap();
        let ai = y.find("alerts-project: &").unwrap();
        assert!(li < ai, "dependency must precede dependent:\n{y}");
        // and the whole document must parse with the alias resolving
        let v: serde_yaml::Value = serde_yaml::from_str(&y).expect(&y);
        assert_eq!(v["alerts-project"].as_str(), None); // variables block only
    }

    #[test]
    fn use_forms_compile_to_include_directives() {
        let y = satz_to_yaml(
            r#"
params { logsink_project_name = "p" }
use "presets/a.yaml"
use "presets/b.yaml" as org_policy_policy
use "presets/c.yaml" when logsink_project_name
"#,
        )
        .unwrap();
        assert!(y.contains("\n!include presets/a.yaml"), "{y}");
        assert!(y.contains("org_policy_policy: !include presets/b.yaml"), "{y}");
        assert!(y.contains("!include-if logsink-project-name presets/c.yaml"), "{y}");
    }

    #[test]
    fn blocks_nest_and_use_inside_folder_works() {
        let y = satz_to_yaml(
            r#"
folder logging_folder {
  display_name = "logging"
  use "presets/monitoring/organization-audit-logsink-1.0.yaml"
}
"#,
        )
        .unwrap();
        assert!(y.contains("folder:"), "{y}");
        assert!(y.contains("\n  logging_folder:"), "{y}");
        assert!(y.contains("\n    display_name: \"logging\""), "{y}");
        assert!(y.contains("\n    !include presets/monitoring/organization-audit-logsink-1.0.yaml"), "{y}");
    }

    #[test]
    fn interpolated_keys_and_role_lists() {
        let y = satz_to_yaml(
            r#"
params {
  svc_iac_account = "svc-iac-001"
  infra_project_name = "acme-infra-001"
}
google_organization_iam_member {
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/owner",
    "roles/billing.user",
  ]
}
"#,
        )
        .unwrap();
        assert!(
            y.contains("!format [\"serviceAccount:{}@{}.iam.gserviceaccount.com\", *svc-iac-account, *infra-project-name]:"),
            "{y}"
        );
        assert!(y.contains("- \"roles/owner\""), "{y}");
    }

    #[test]
    fn two_folder_blocks_merge_into_one_mapping() {
        let y = satz_to_yaml(
            r#"
folder a { display_name = "A" }
folder b { display_name = "B" }
"#,
        )
        .unwrap();
        assert_eq!(y.matches("\nfolder:").count(), 1, "{y}");
        assert!(y.contains("\n  a:"), "{y}");
        assert!(y.contains("\n  b:"), "{y}");
    }

    #[test]
    fn object_lists_emit_block_sequences() {
        let y = satz_to_yaml(
            r#"
google_storage_bucket {
  state {
    lifecycle_rule = [
      { action { type = "Delete" } condition { age = 400 } },
    ]
  }
}
"#,
        )
        .unwrap();
        assert!(y.contains("lifecycle_rule:"), "{y}");
        assert!(y.contains("- action:"), "{y}");
        assert!(y.contains("age: 400"), "{y}");
        // and the whole thing must be valid YAML
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&y).is_ok(), "{y}");
    }

    /// Claims are language syntax compiled to the sidecar the compliance loader
    /// already consumes — one source, sidecar generated, witnesses mandatory.
    #[test]
    fn claims_compile_to_the_sidecar() {
        let c = compile(
            r#"
pack demo.pack
claim "cis-gcp" "4.0" "2.2" implements {
  resources = ["google_logging_organization_sink.archive"]
  duty_lock_it = "lock the bucket"
  interpretation = "why this counts"
}
"#,
        )
        .unwrap();
        let y = c.claims_yaml.expect("sidecar generated");
        assert!(y.contains("pack: demo.pack"), "{y}");
        assert!(y.contains("control: \"2.2\""), "{y}");
        assert!(y.contains("- google_logging_organization_sink.archive"), "{y}");
        assert!(y.contains("- id: lock-it"), "{y}");
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&y).is_ok(), "{y}");
    }

    #[test]
    fn claim_without_witnesses_is_rejected() {
        let e = parse("claim \"cis-gcp\" \"4.0\" \"2.2\" implements { interpretation = \"x\" }")
            .unwrap_err();
        assert!(e.msg.contains("witnesses"), "{e}");
    }

    #[test]
    fn satz_pack_uses_are_rewritten_and_reported() {
        let c = compile("use \"packs/logsink.satz\"\nuse \"plain.yaml\"\n").unwrap();
        assert!(c.yaml.contains("!include packs/logsink.gen.yaml"), "{}", c.yaml);
        assert!(c.yaml.contains("!include plain.yaml"), "{}", c.yaml);
        assert_eq!(c.satz_deps, vec!["packs/logsink.satz".to_string()]);
    }

    #[test]
    fn errors_carry_line_numbers() {
        let e = parse("params {\n  broken =\n}").unwrap_err();
        assert_eq!(e.line, 3, "{e}"); // value missing, found '}' on line 3
        let e = parse("x = ").unwrap_err();
        assert!(e.line >= 1);
    }

    #[test]
    fn generated_yaml_parses_and_resolves_params() {
        let y = satz_to_yaml(
            r#"
estate t
params {
  region = "europe-west3"
}
terraform {
  backend {
    local { path = "terraform.tfstate" }
  }
}
google_storage_bucket {
  b {
    location = region
  }
}
"#,
        )
        .unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&y).expect(&y);
        // alias resolves through the variables block
        assert_eq!(
            v["google_storage_bucket"]["b"]["location"].as_str(),
            Some("europe-west3"),
            "{y}"
        );
    }

    #[test]
    fn hcl_passthrough_captures_body_verbatim() {
        let f = parse(concat!(
            "estate e\n",
            "hcl {\n",
            "  # brace in a comment }\n",
            "  resource \"google_storage_bucket\" \"raw\" {\n",
            "    name = \"a-}-b\"\n",
            "    lifecycle {\n",
            "      prevent_destroy = true\n",
            "    }\n",
            "    doc = <<-EOT\n",
            "      { still inside }\n",
            "    EOT\n",
            "  }\n",
            "}\n",
        ))
        .unwrap();
        assert_eq!(f.hcl_blocks.len(), 1);
        let b = &f.hcl_blocks[0];
        assert!(b.trust.is_none());
        // Every brace-bearing construct survived: the block did not end early.
        assert!(b.body.contains("prevent_destroy = true"), "{}", b.body);
        assert!(b.body.contains("{ still inside }"), "{}", b.body);
        assert!(b.body.contains("name = \"a-}-b\""), "{}", b.body);
        // …and nothing after the block leaked into it.
        assert!(!b.body.contains("estate"), "{}", b.body);
    }

    #[test]
    fn hcl_trust_carries_its_reason_and_blocks_still_follow() {
        let f = parse(concat!(
            "estate e\n",
            "hcl trust \"reviewed 2026-08 by TJ\" {\n",
            "  output \"x\" { value = 1 }\n",
            "}\n",
            "google_storage_bucket {\n",
            "  b { location = \"EU\" }\n",
            "}\n",
        ))
        .unwrap();
        assert_eq!(f.hcl_blocks.len(), 1);
        assert_eq!(f.hcl_blocks[0].trust.as_deref(), Some("reviewed 2026-08 by TJ"));
        // parsing resumed correctly after the raw block
        assert_eq!(f.items.len(), 1);
    }

    #[test]
    fn hcl_stays_usable_as_an_ordinary_identifier() {
        // `hcl` only opens a passthrough when a block (or `trust`) follows it.
        let f = parse(concat!(
            "estate e\n",
            "params {\n",
            "  hcl = \"not a block\"\n",
            "}\n",
        ))
        .unwrap();
        assert!(f.hcl_blocks.is_empty());
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.params[0].0, "hcl");
    }

    #[test]
    fn unterminated_hcl_block_is_an_error() {
        let e = parse("estate e\nhcl {\n  resource \"x\" \"y\" {\n").unwrap_err();
        assert!(e.msg.contains("unterminated hcl"), "{}", e.msg);
    }

    #[test]
    fn compiled_flags_hcl_for_the_yaml_path_guard() {
        let c = compile("estate e\nhcl {\n  output \"x\" { value = 1 }\n}\n").unwrap();
        assert!(c.has_hcl);
        let c = compile("estate e\n").unwrap();
        assert!(!c.has_hcl);
    }

}

#[cfg(test)]
mod empty_collection_tests {
    use super::*;

    /// An empty list must survive the YAML round trip as `[]`. It used to emit
    /// nothing, leaving `key: &anchor` with no value — which YAML reads as null,
    /// so every consumer of the twin saw the key as absent.
    #[test]
    fn empty_list_param_emits_a_list_not_null() {
        let src = "pack p\n\nparams {\n  subjects = []\n}\n\n\
                   google_org_policy_policy {\n  x {\n    name = \"c\"\n    members = subjects\n  }\n}\n";
        let c = compile(src).expect("compile");
        assert!(
            c.yaml.contains("subjects: &subjects []"),
            "empty list must emit []:\n{}",
            c.yaml
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&c.yaml).expect("twin must parse");
        let vars = parsed.get("variables").expect("variables block");
        assert_eq!(
            vars.get("subjects").and_then(|v| v.as_sequence()).map(|s| s.len()),
            Some(0),
            "must read back as an empty sequence, not null"
        );
    }
}
