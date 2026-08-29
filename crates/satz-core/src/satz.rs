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
/// Read by the compliance plane straight from the source through the front end.
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

/// The `.satz` files a parsed file `use`s, as written (any depth).
pub fn use_paths(file: &File) -> Vec<String> {
    let mut out = Vec::new();
    collect_satz_deps(&file.items, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Canonical form
//
// What `check-presets` / `merge-presets` compare: the parsed file printed
// deterministically, without comments, formatting or line numbers — so two
// files that MEAN the same thing print the same. The pack `version` is metadata
// and deliberately not part of it: a version bump with no content change must
// read as a comment-only upgrade, which is what the staleness check reports
// separately. This replaced the YAML twin as the canonical form (M5).
// ---------------------------------------------------------------------------

/// A file in canonical form, split the way drift classification needs it:
/// params (name → canonical value) and everything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonical {
    pub params: Vec<(String, String)>,
    pub body: String,
}

/// Canonical text of the whole file (params + body).
pub fn canonical(file: &File) -> String {
    let c = canonical_parts(file);
    let mut s = String::new();
    for (n, v) in &c.params {
        s.push_str(n);
        s.push('=');
        s.push_str(v);
        s.push('\n');
    }
    s.push_str(&c.body);
    s
}

pub fn canonical_parts(file: &File) -> Canonical {
    let params = file.params.iter().map(|(n, v, _)| (n.clone(), canon_value(v))).collect();
    let mut body = String::new();
    match (&file.estate, file.is_pack) {
        (Some(n), true) => {
            body.push_str("pack ");
            body.push_str(n);
            if file.content_mode {
                body.push_str(" content");
            }
            body.push('\n');
        }
        (Some(n), false) => {
            body.push_str("estate ");
            body.push_str(n);
            body.push('\n');
        }
        (None, _) => {}
    }
    for e in &file.items {
        canon_entry(e, &mut body);
        body.push('\n');
    }
    for c in &file.claims {
        body.push_str(&format!(
            "claim({}|{}|{}|{}|[{}]|{}|{}|[{}])\n",
            c.framework,
            c.version,
            c.control,
            c.coverage,
            c.resources.join(","),
            c.reason.as_deref().unwrap_or(""),
            c.interpretation.as_deref().unwrap_or(""),
            c.duties.iter().map(|(a, b)| format!("{}={}", a, b)).collect::<Vec<_>>().join(",")
        ));
    }
    for s in &file.suppressions {
        body.push_str(&format!(
            "suppress({}|{}|{})\n",
            s.tf_type,
            canon_str(&s.label),
            s.role.as_ref().map(|r| canon_str(r)).unwrap_or_default()
        ));
    }
    for h in &file.hcl_blocks {
        body.push_str(&format!("hcl({}){{{}}}\n", h.trust.as_deref().unwrap_or(""), h.body.trim()));
    }
    Canonical { params, body }
}

fn canon_str(parts: &[StrPart]) -> String {
    let mut s = String::from("\"");
    for p in parts {
        match p {
            StrPart::Lit(l) => {
                for ch in l.chars() {
                    match ch {
                        '{' => s.push_str("{{"),
                        '}' => s.push_str("}}"),
                        '"' => s.push_str("\\\""),
                        '\\' => s.push_str("\\\\"),
                        '\n' => s.push_str("\\n"),
                        c => s.push(c),
                    }
                }
            }
            StrPart::Param(p) => {
                s.push('{');
                s.push_str(p);
                s.push('}');
            }
        }
    }
    s.push('"');
    s
}

fn canon_key(k: &Key) -> String {
    match k {
        Key::Ident(i) => i.clone(),
        Key::Str(parts) => canon_str(parts),
    }
}

fn canon_value(v: &Value) -> String {
    match v {
        Value::Str(parts) => canon_str(parts),
        Value::Num(n) => n.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Ref(r) => r.clone(),
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(canon_value).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Obj(entries) => {
            let mut s = String::from("{");
            for e in entries {
                canon_entry(e, &mut s);
                s.push(';');
            }
            s.push('}');
            s
        }
    }
}

fn canon_entry(e: &Entry, out: &mut String) {
    match e {
        Entry::Attr { key, value, .. } => {
            out.push_str(&canon_key(key));
            out.push('=');
            out.push_str(&canon_value(value));
        }
        Entry::Map { key, name, body, .. } => {
            out.push_str(&canon_key(key));
            if let Some(n) = name {
                out.push(' ');
                out.push_str(&canon_key(n));
            }
            out.push('{');
            for b in body {
                canon_entry(b, out);
                out.push(';');
            }
            out.push('}');
        }
        Entry::Use { path, as_key, when, .. } => {
            out.push_str(&format!(
                "use({}|{}|{})",
                path,
                as_key.as_deref().unwrap_or(""),
                when.as_deref().unwrap_or("")
            ));
        }
    }
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

    /// Claims are language syntax read straight from the parsed file — no
    /// sidecar is generated any more; the front end carries them to the
    /// compliance plane. Witnesses mandatory.
    #[test]
    fn claims_are_parsed_from_the_source() {
        let f = parse(
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
        assert_eq!(f.claims.len(), 1, "{:?}", f.claims);
        let c = &f.claims[0];
        assert_eq!((c.framework.as_str(), c.version.as_str(), c.control.as_str()), ("cis-gcp", "4.0", "2.2"));
        assert_eq!(c.resources, vec!["google_logging_organization_sink.archive".to_string()]);
        assert_eq!(c.duties.len(), 1, "{:?}", c.duties);
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

    /// The canonical form is what drift classification compares: comments,
    /// formatting and the pack version must not move it; a param default or a
    /// resource body must.
    #[test]
    fn canonical_ignores_churn_and_version_but_sees_meaning() {
        let base = "pack demo version \"1.0\"\nparams {\n  bucket_name = \"demo-audit\"\n}\ngoogle_storage_bucket {\n  b { name = bucket_name location = \"EU\" }\n}\n";
        let churn = "// note\npack demo   version \"1.1\"\n\nparams {\n\n  bucket_name = \"demo-audit\" // default\n}\n\ngoogle_storage_bucket {\n  b {\n    name     = bucket_name\n    location = \"EU\"\n  }\n}\n";
        let a = canonical_parts(&parse(base).unwrap());
        let b = canonical_parts(&parse(churn).unwrap());
        assert_eq!(a, b, "comment/format/version churn must be canonical-equal");

        let param = base.replace("\"demo-audit\"", "\"customer-audit\"");
        let c = canonical_parts(&parse(&param).unwrap());
        assert_eq!(a.body, c.body, "a default change is not a body change");
        assert_ne!(a.params, c.params);
        assert_eq!(c.params[0], ("bucket_name".to_string(), "\"customer-audit\"".to_string()));

        let body = base.replace("\"EU\"", "\"US\"");
        let d = canonical_parts(&parse(&body).unwrap());
        assert_eq!(a.params, d.params);
        assert_ne!(a.body, d.body, "a resource change is a body change");

        let deps = use_paths(&parse("estate e\nuse \"a.satz\"\ngoogle_folder { f { use \"b.satz\" } }\n").unwrap());
        assert_eq!(deps, vec!["a.satz".to_string(), "b.satz".to_string()]);
    }
}
