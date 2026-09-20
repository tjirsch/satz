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
//! dialect's kebab-case anchors (`logsink_bucket_name` <-> `logsink-bucket-name`),
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

/// `action "<name>" { … }` — a deployment step that has no provider resource.
///
/// Satz never runs one while compiling: an action is inert until `satz run-actions`
/// is invoked. It emits nothing, never enters the fold or the emission manifest, and
/// can carry no claim — the same opacity `hcl { … }` has, with execution on top. That
/// is why `reason` is mandatory and why the warning an action raises cannot be
/// downgraded the way `hcl trust` downgrades its own: HCL only deploys, an action runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionDecl {
    /// Unique across the estate; a duplicate is an error naming both files.
    pub name: String,
    /// Why this step is not a resource. Quoted back in every warning.
    pub reason: String,
    /// The executable, resolved relative to the directory of the DECLARING file, so a
    /// pack that ships a script is self-contained. Never interpolated: a path assembled
    /// from params is a path no reader can check against the warning.
    pub run: String,
    /// Always passed. `{param}` interpolates.
    pub args: Vec<Vec<StrPart>>,
    /// Appended only under `--execute` — where a script's `--apply` lives.
    pub execute_args: Vec<Vec<StrPart>>,
    /// `before-apply` | `after-apply`. Advisory: it orders the run and selects with
    /// `--phase`, but nothing chains `satz apply`, so satz enforces no relationship.
    pub phase: String,
    pub line: usize,
}

/// What changing an answer costs the ESTATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reversal {
    /// one line, one apply
    Edit,
    /// a rename plus `state mv`, and whatever the old name is wired into
    StateSurgery,
    /// destroy and recreate — globally unique ids, mail addresses, buckets
    Recreate,
}

/// What changing an answer costs the RUNNING organisation. Orthogonal to
/// `Reversal`, and conflating the two is the mistake this design exists to
/// avoid: enforcing OS Login is one boolean to reverse (edit) and cuts every
/// existing SSH path (high).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blast {
    None,
    Low,
    High,
}

impl Reversal {
    pub fn as_str(self) -> &'static str {
        match self {
            Reversal::Edit => "edit",
            Reversal::StateSurgery => "state_surgery",
            Reversal::Recreate => "recreate",
        }
    }
    fn parse(id: &str) -> Option<Self> {
        match id {
            "edit" => Some(Reversal::Edit),
            "state_surgery" => Some(Reversal::StateSurgery),
            "recreate" => Some(Reversal::Recreate),
            _ => None,
        }
    }
}

impl Blast {
    pub fn as_str(self) -> &'static str {
        match self {
            Blast::None => "none",
            Blast::Low => "low",
            Blast::High => "high",
        }
    }
    fn parse(id: &str) -> Option<Self> {
        match id {
            "none" => Some(Blast::None),
            "low" => Some(Blast::Low),
            "high" => Some(Blast::High),
            _ => None,
        }
    }
}

/// One branch of a `question oneof`. Names an existing boolean param, so an
/// answer set stays a plain param map and a question never becomes a second
/// channel for setting values.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionOption {
    pub param: String,
    pub label: String,
    pub why: Option<String>,
    pub line: usize,
}

/// `question <param> { … }` — what to ask a customer so a param can be filled,
/// and what getting it wrong costs.
///
/// Declared beside `params` and `claims`, in the file that owns the param,
/// because a question that gates a pack cannot live in the gated pack: questions
/// are absorbed after the `use … when` guard, so it would be invisible until the
/// answer is already yes.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionDecl {
    /// the param this answers — or, for `oneof`, the group's own name
    pub subject: String,
    pub oneof: bool,
    pub prompt: String,
    /// Required where satz will refuse or warn, so it can quote the pack's own
    /// sentence instead of a generic one.
    pub why: Option<String>,
    pub reversal: Reversal,
    pub blast: Blast,
    /// What the interview OFFERS. The `params` default is what applies when
    /// nobody asked — one default, one recommendation, never two channels.
    pub recommend: Option<Value>,
    /// Only ask this when another param is truthy.
    pub ask_when: Option<String>,
    /// `oneof` only: exactly one option must be true, rather than at most one.
    pub required: bool,
    pub options: Vec<QuestionOption>,
    pub line: usize,
}

/// `offers "<pack path>" { … }` — one pack the library offers an estate, in the map.
///
/// The map is the one file every estate uses to decide which packs it takes, so it
/// is where a pack's gate, its phase, its place in the estate and its adoption order
/// (the entry's position in the file) are written. What a pack needs from another
/// pack is mostly visible in the packs themselves — a param default that reads
/// another pack's param — and `satz pack-graph` derives that; `requires` and
/// `excludes` declare only what the packs cannot show. Emits nothing: the compile
/// never reads an entry, so it is inert in an estate's fold.
#[derive(Debug, Clone, PartialEq)]
pub struct OffersDecl {
    /// the pack, as a `use` line names it (`presets/…`)
    pub path: String,
    /// the param the pack's line is gated on; only the map's own entry has none
    pub when: Option<String>,
    /// opens a group of lines: what has to be finished before they can go in
    pub phase: Option<String>,
    /// the block the line is written inside (`google_folder.infra_folder`)
    pub block: Option<String>,
    /// the line goes after the scaffold rather than in the menu above it
    pub after_scaffold: bool,
    /// the line is written by hand, never by satz; the reason why
    pub by_hand: Option<String>,
    pub requires: Vec<String>,
    pub excludes: Vec<String>,
    pub line: usize,
}

/// The one pack that may carry `offers` entries.
pub const MAP_PACK: &str = "estate_map";

/// `notice <param> { text run before }` — what to run once the pack is switched on.
///
/// A pack that needs one step after it goes in — the CIS org-policy packs need `satz
/// adopt`, because Google sets some of their policies on every new organisation and the
/// first apply stops on `409` for each — names that step here. satz shows the notice when
/// the pack is switched on and until the estate acknowledges it by binding `<param> =
/// true`; with `before = apply`, `transpile --apply` and `bootstrap` refuse while it is
/// open. The param is the pack's own, declared `false` in the same file, and read by
/// nothing: it is an acknowledgement, not configuration, so it is never emitted.
#[derive(Debug, Clone, PartialEq)]
pub struct NoticeDecl {
    /// the param the estate binds `true` to acknowledge the notice
    pub param: String,
    /// what to do and why, as the operator reads it
    pub text: String,
    /// the command to run
    pub run: String,
    /// `apply`: `transpile --apply` and `bootstrap` refuse while the notice is open
    pub before: Option<String>,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct File {
    pub estate: Option<String>,
    /// true when the header keyword was `pack` (estate and pack share the name slot)
    pub is_pack: bool,
    /// `pack <name> version "1.2"` — the pack file's own revision, deliberately kept
    /// OUT of the filename (framework/standard versions live in claims and are
    /// orthogonal: multiple internal revisions may implement the same standard).
    pub version: Option<String>,
    pub params: Vec<(String, Value, usize)>,
    pub items: Vec<Entry>,
    pub claims: Vec<ClaimDecl>,
    pub suppressions: Vec<Suppression>,
    pub hcl_blocks: Vec<HclBlock>,
    pub actions: Vec<ActionDecl>,
    pub questions: Vec<QuestionDecl>,
    /// `offers` entries, in file order — only the map (`pack estate_map`) has any
    pub offers: Vec<OffersDecl>,
    /// `notice` statements — only a pack has any
    pub notices: Vec<NoticeDecl>,
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
pub enum Tok {
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
    /// Trivia, lexed only for the formatter: a comment, verbatim, and a line end.
    Comment(String),
    Newline,
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

/// A string that must not interpolate. Used where a value has to be readable in a
/// warning exactly as it is written in the file.
fn lit_str(parts: &[StrPart], line: usize, what: &str) -> Result<String, SatzError> {
    match parts {
        [StrPart::Lit(v)] => Ok(v.clone()),
        _ => err(line, format!("{}: no interpolation allowed", what)),
    }
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

/// A token with where it sits in the source: `line` (1-based) and the char range
/// `start..end` into `src.chars()`. Trivia — comments and newlines — is lexed only
/// on request, for the formatter and the language server, never for the parser.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, SatzError> {
    Ok(lex_spanned(src, false)?.into_iter().map(|t| (t.tok, t.line)).collect())
}

pub fn lex_spanned(src: &str, trivia: bool) -> Result<Vec<Token>, SatzError> {
    let mut toks = Vec::new();
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let start_line = line;
        let tok = match c {
            '\n' => {
                line += 1;
                i += 1;
                if !trivia {
                    continue;
                }
                Tok::Newline
            }
            ' ' | '\t' | '\r' => {
                i += 1;
                continue;
            }
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                if !trivia {
                    continue;
                }
                Tok::Comment(b[start..i].iter().collect())
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                loop {
                    if i >= b.len() {
                        return Err(SatzError { line: start_line, msg: "unterminated block comment".into() });
                    }
                    if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                        i += 2;
                        break;
                    }
                    if b[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if !trivia {
                    continue;
                }
                Tok::Comment(b[start..i].iter().collect())
            }
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                if !trivia {
                    continue;
                }
                Tok::Comment(b[start..i].iter().collect())
            }
            '{' => {
                i += 1;
                Tok::LBrace
            }
            '}' => {
                i += 1;
                Tok::RBrace
            }
            '[' => {
                i += 1;
                Tok::LBrack
            }
            ']' => {
                i += 1;
                Tok::RBrack
            }
            '=' => {
                i += 1;
                Tok::Eq
            }
            ',' => {
                i += 1;
                Tok::Comma
            }
            '"' => {
                // Triple-quoted multi-line or normal string; both interpolate {param}.
                let triple = b.get(i + 1) == Some(&'"') && b.get(i + 2) == Some(&'"');
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
                Tok::Str(parts)
            }
            c if c.is_ascii_digit() || (c == '-' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let mut n = String::new();
                n.push(c);
                i += 1;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                    n.push(b[i]);
                    i += 1;
                }
                let dots = n.matches('.').count();
                if dots > 1 || n.ends_with('.') {
                    return Err(SatzError { line, msg: format!("malformed number `{}`", n) });
                }
                Tok::Num(n)
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut id = String::new();
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_' || b[i] == '.') {
                    id.push(b[i]);
                    i += 1;
                }
                if id == "hcl" {
                    if let Some(h) = try_lex_hcl(&b, i, line)? {
                        i = h.next;
                        line = h.line;
                        toks.push(Token { tok: Tok::Hcl(h.body, h.trust), line: start_line, start, end: i });
                        continue;
                    }
                }
                Tok::Ident(id)
            }
            other => return err(line, format!("unexpected character '{}'", other)),
        };
        toks.push(Token { tok, line: start_line, start, end: i });
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
        // (key, name) → first line. A key that repeats inside ONE body used to
        // last-win silently (`lifecycle_rule { A } lifecycle_rule { B }` emitted
        // only B); it is an error naming both lines now. Resource-type maps
        // (`google_…`) may repeat — two `google_org_policy_policy { … }` groups
        // in one file are the same map, folded by address.
        let mut seen: Vec<(String, Option<String>, usize)> = Vec::new();
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
                            note_key(&mut seen, &key, None, line)?;
                            out.push(Entry::Attr { key, value, line });
                        }
                        Some(Tok::LBrace) => {
                            self.next();
                            let body = self.entries()?;
                            note_key(&mut seen, &key, None, line)?;
                            out.push(Entry::Map { key, name: None, body, line });
                        }
                        Some(Tok::Ident(_)) | Some(Tok::Str(_)) => {
                            let name = match self.next().unwrap() {
                                Tok::Ident(id) => Key::Ident(id),
                                Tok::Str(s) => Key::Str(s),
                                _ => unreachable!(),
                            };
                            if let (Key::Ident(k), false) = (&key, matches!(self.peek(), Some(Tok::LBrace))) {
                                if STATEMENT_KEYWORDS.contains(&k.as_str()) {
                                    return err(line, statement_in_a_block(k));
                                }
                            }
                            self.expect(Tok::LBrace, "'{' after map entry name")?;
                            let body = self.entries()?;
                            note_key(&mut seen, &key, Some(&name), line)?;
                            out.push(Entry::Map { key, name: Some(name), body, line });
                        }
                        other => {
                            return err(line, format!("after key: expected '=', '{{' or a name, found {:?}", other))
                        }
                    }
                }
                Some(Tok::Hcl(..)) => return err(line, statement_in_a_block("hcl")),
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
                Entry::Map { key: Key::Ident(k), name: Some(Key::Str(_)), .. } if k == "duty" => {
                    // the block form `duty "id" { text = "..." }` is refused in favour of the attribute
                    return err(line, "duty: write it as an attribute, `duty_<id> = \"text\"`");
                }
                Entry::Attr { key: Key::Str(_), .. } => {
                    return err(line, "claim: unexpected string key (a duty is `duty_<id> = \"text\"`)")
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k.starts_with("duty_") || k == "duty" => {
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

    fn question_stmt(&mut self, line: usize) -> Result<QuestionDecl, SatzError> {
        let oneof = matches!(self.peek(), Some(Tok::Ident(id)) if id == "oneof");
        if oneof {
            self.next();
        }
        let subject = match self.next() {
            Some(Tok::Ident(id)) => id,
            other => {
                return err(
                    line,
                    format!(
                        "question: expected the {} name, found {:?}",
                        if oneof { "group" } else { "param" },
                        other
                    ),
                );
            }
        };
        self.expect(Tok::LBrace, "'{' after the question name")?;
        let body = self.entries()?;

        let mut prompt = None;
        let mut why = None;
        let mut reversal = None;
        let mut blast = None;
        let mut recommend = None;
        let mut ask_when = None;
        let mut required = false;
        let mut options: Vec<QuestionOption> = Vec::new();

        for e in body {
            match e {
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "prompt" => {
                    prompt = Some(lit_str(&parts, l, "question: prompt")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "why" => {
                    why = Some(lit_str(&parts, l, "question: why")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Ref(id), line: l } if k == "reversal" => {
                    reversal = Some(Reversal::parse(&id).ok_or_else(|| {
                        SatzError { line: l, msg: format!(
                            "question {}: reversal = {} is not one of edit | state_surgery | recreate",
                            subject, id) }
                    })?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Ref(id), line: l } if k == "blast" => {
                    blast = Some(Blast::parse(&id).ok_or_else(|| {
                        SatzError { line: l, msg: format!(
                            "question {}: blast = {} is not one of none | low | high", subject, id) }
                    })?);
                }
                Entry::Attr { key: Key::Ident(k), value: v, .. } if k == "recommend" => {
                    recommend = Some(v);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Ref(id), .. } if k == "ask_when" => {
                    ask_when = Some(id);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Bool(b), .. } if k == "required" => {
                    required = b;
                }
                Entry::Map { key: Key::Ident(k), name: Some(Key::Ident(param)), body, line: l }
                    if k == "option" =>
                {
                    let mut label = None;
                    let mut owhy = None;
                    for oe in body {
                        match oe {
                            Entry::Attr { key: Key::Ident(ok), value: Value::Str(parts), line: ol }
                                if ok == "label" =>
                            {
                                label = Some(lit_str(&parts, ol, "question option: label")?);
                            }
                            Entry::Attr { key: Key::Ident(ok), value: Value::Str(parts), line: ol }
                                if ok == "why" =>
                            {
                                owhy = Some(lit_str(&parts, ol, "question option: why")?);
                            }
                            other => {
                                return err(l, format!(
                                    "question option {}: unexpected entry {:?} (only label and why)",
                                    param, other));
                            }
                        }
                    }
                    let label = label.ok_or_else(|| SatzError {
                        line: l,
                        msg: format!("question option {}: label = \"…\" is required", param),
                    })?;
                    if let Some(first) = options.iter().find(|o| o.param == param) {
                        return err(l, format!(
                            "question oneof {}: option {} appears twice (line {} and line {})",
                            subject, param, first.line, l));
                    }
                    options.push(QuestionOption { param, label, why: owhy, line: l });
                }
                other => {
                    return err(line, format!(
                        "question {}: unexpected entry {:?} — the keys are prompt, why, reversal, \
                         blast, recommend, ask_when{}",
                        subject, other,
                        if oneof { ", required and `option <param> { … }`" } else { "" }));
                }
            }
        }

        let prompt = prompt.ok_or_else(|| SatzError {
            line,
            msg: format!("question {}: prompt = \"…\" is required — it is what a human is asked", subject),
        })?;
        let reversal = reversal.ok_or_else(|| SatzError {
            line,
            msg: format!(
                "question {}: reversal = edit | state_surgery | recreate is required (what changing \
                 the answer costs the estate)", subject),
        })?;
        let blast = blast.ok_or_else(|| SatzError {
            line,
            msg: format!(
                "question {}: blast = none | low | high is required (what changing the answer costs \
                 the running organisation — orthogonal to reversal)", subject),
        })?;
        // Where satz will refuse or warn, it must be able to quote the pack's own
        // sentence. Same rule as `claim … deviates` requiring a reason.
        if why.is_none() && (reversal == Reversal::Recreate || blast == Blast::High) {
            return err(line, format!(
                "question {}: why = \"…\" is required when reversal is recreate or blast is high — \
                 that is the sentence satz quotes when it refuses to derive on a deferred answer",
                subject));
        }
        if oneof {
            if options.len() < 2 {
                return err(line, format!(
                    "question oneof {}: needs at least two `option <param> {{ … }}` branches (found {})",
                    subject, options.len()));
            }
        } else {
            if !options.is_empty() {
                return err(line, format!(
                    "question {}: `option` belongs to a `question oneof <group>`", subject));
            }
            if required {
                return err(line, format!(
                    "question {}: `required` belongs to a `question oneof <group>`", subject));
            }
        }

        Ok(QuestionDecl {
            subject, oneof, prompt, why, reversal, blast, recommend, ask_when, required, options, line,
        })
    }

    fn action_stmt(&mut self, line: usize) -> Result<ActionDecl, SatzError> {
        let name = match self.next() {
            Some(Tok::Str(parts)) => lit_str(&parts, line, "action: the name")?,
            other => return err(line, format!("action: expected a quoted name, found {:?}", other)),
        };
        if name.is_empty() {
            return err(line, "action: the name is empty (it is how `run-actions --only` selects it)");
        }
        self.expect(Tok::LBrace, "'{' after the action name")?;
        let body = self.entries()?;

        let mut reason = None;
        let mut run = None;
        let mut args = Vec::new();
        let mut execute_args = Vec::new();
        let mut phase = None;

        // A list of plain-or-interpolated strings; anything else is refused rather
        // than stringified, because an argument list is what gets executed.
        let arg_list = |items: Vec<Value>, what: &str| -> Result<Vec<Vec<StrPart>>, SatzError> {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Value::Str(parts) => out.push(parts),
                    other => {
                        return err(line, format!("action {}: expected strings, found {:?}", what, other))
                    }
                }
            }
            Ok(out)
        };

        for e in body {
            match e {
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "reason" => {
                    reason = Some(lit_str(&parts, line, "action reason")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "run" => {
                    run = Some(lit_str(&parts, line, "action run")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "phase" => {
                    phase = Some(lit_str(&parts, line, "action phase")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::List(items), .. } if k == "args" => {
                    args = arg_list(items, "args")?;
                }
                Entry::Attr { key: Key::Ident(k), value: Value::List(items), .. } if k == "execute_args" => {
                    execute_args = arg_list(items, "execute_args")?;
                }
                other => {
                    return err(
                        line,
                        format!(
                            "action: unexpected entry {:?} (keys are reason, run, args, execute_args, phase)",
                            other
                        ),
                    )
                }
            }
        }

        let Some(reason) = reason else {
            return err(line, format!("action \"{}\": reason = \"…\" is required (it is what the execution warning quotes)", name));
        };
        let Some(run) = run else {
            return err(line, format!("action \"{}\": run = \"…\" is required (the executable to run)", name));
        };
        if run.is_empty() {
            return err(line, format!("action \"{}\": run = \"\" is empty", name));
        }
        let phase = phase.unwrap_or_else(|| "after-apply".to_string());
        if phase != "before-apply" && phase != "after-apply" {
            return err(line, format!("action \"{}\": phase = \"{}\" — expected \"before-apply\" or \"after-apply\"", name, phase));
        }
        Ok(ActionDecl { name, reason, run, args, execute_args, phase, line })
    }

    fn notice_stmt(&mut self, line: usize) -> Result<NoticeDecl, SatzError> {
        let param = match self.next() {
            Some(Tok::Ident(id)) => id,
            other => return err(line, format!("notice: expected the param that acknowledges it, found {:?}", other)),
        };
        self.expect(Tok::LBrace, "'{' after the notice's param")?;
        let body = self.entries()?;
        let (mut text, mut run, mut before) = (None, None, None);
        for e in body {
            match e {
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "text" => {
                    text = Some(lit_str(&parts, l, "notice: text")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "run" => {
                    run = Some(lit_str(&parts, l, "notice: run")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Ref(id), line: l } if k == "before" => {
                    if id != "apply" {
                        return err(l, format!("notice {}: before = {} — the one step a notice holds back is `apply`", param, id));
                    }
                    before = Some(id);
                }
                other => {
                    return err(
                        line,
                        format!("notice {}: unexpected entry {:?} — the keys are text, run and before = apply", param, other),
                    )
                }
            }
        }
        let text = match text {
            Some(t) if !t.trim().is_empty() => t,
            _ => return err(line, format!("notice {}: text = \"…\" is required — it is what the operator reads", param)),
        };
        let run = match run {
            Some(r) if !r.trim().is_empty() => r,
            _ => return err(line, format!("notice {}: run = \"…\" is required — the command the notice names", param)),
        };
        Ok(NoticeDecl { param, text, run, before, line })
    }

    fn offers_stmt(&mut self, line: usize) -> Result<OffersDecl, SatzError> {
        let path = match self.next() {
            Some(Tok::Str(parts)) => lit_str(&parts, line, "offers: the pack path")?,
            other => return err(line, format!("offers: expected the pack's path as a quoted string, found {:?}", other)),
        };
        if !path.ends_with(".satz") {
            return err(line, format!("offers \"{}\": the path names a pack, a `.satz` file", path));
        }
        self.expect(Tok::LBrace, "'{' after the offered pack's path")?;
        let body = self.entries()?;
        let mut o = OffersDecl {
            path,
            when: None,
            phase: None,
            block: None,
            after_scaffold: false,
            by_hand: None,
            requires: Vec::new(),
            excludes: Vec::new(),
            line,
        };
        let paths = |items: Vec<Value>, key: &str, path: &str, l: usize| -> Result<Vec<String>, SatzError> {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Value::Str(parts) => out.push(lit_str(&parts, l, key)?),
                    other => {
                        return err(l, format!("offers \"{}\": {} lists pack paths as strings, found {:?}", path, key, other))
                    }
                }
            }
            Ok(out)
        };
        for e in body {
            match e {
                Entry::Attr { key: Key::Ident(k), value: Value::Ref(p), .. } if k == "when" => o.when = Some(p),
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "phase" => {
                    o.phase = Some(lit_str(&parts, l, "offers: phase")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "block" => {
                    o.block = Some(lit_str(&parts, l, "offers: block")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Bool(b), .. } if k == "after_scaffold" => {
                    o.after_scaffold = b;
                }
                Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), line: l } if k == "by_hand" => {
                    o.by_hand = Some(lit_str(&parts, l, "offers: by_hand")?);
                }
                Entry::Attr { key: Key::Ident(k), value: Value::List(items), line: l } if k == "requires" => {
                    o.requires = paths(items, "requires", &o.path, l)?;
                }
                Entry::Attr { key: Key::Ident(k), value: Value::List(items), line: l } if k == "excludes" => {
                    o.excludes = paths(items, "excludes", &o.path, l)?;
                }
                other => {
                    return err(
                        line,
                        format!(
                            "offers \"{}\": unexpected entry {:?} — the keys are when = PARAM, phase, block, \
                             after_scaffold, by_hand and requires / excludes = [\"<pack path>\", …]",
                            o.path, other
                        ),
                    )
                }
            }
        }
        if o.block.is_some() && o.after_scaffold {
            return err(line, format!("offers \"{}\": a line sits in a block or after the scaffold, not both", o.path));
        }
        if o.by_hand.is_some() && (o.block.is_some() || o.after_scaffold || o.phase.is_some()) {
            return err(
                line,
                format!("offers \"{}\": a `by_hand` pack has no line satz writes, so it takes no phase, block or after_scaffold", o.path),
            );
        }
        Ok(o)
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
                    if as_key.is_some() {
                        return err(line, "use ... as: given twice");
                    }
                    match self.next() {
                        Some(Tok::Ident(k)) => as_key = Some(k),
                        other => return err(line, format!("use ... as: expected identifier, found {:?}", other)),
                    }
                }
                Some(Tok::Ident(id)) if id == "when" => {
                    self.next();
                    if when.is_some() {
                        return err(line, "use ... when: given twice");
                    }
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

/// The text of a key for the duplicate check: an identifier as is, a string
/// key by its literal parts with `{…}` for an interpolation.
fn key_text(k: &Key) -> String {
    match k {
        Key::Ident(s) => s.clone(),
        Key::Str(parts) => parts
            .iter()
            .map(|p| match p {
                StrPart::Lit(s) => s.clone(),
                StrPart::Param(r) => format!("{{{}}}", r),
            })
            .collect(),
    }
}

/// Record a body entry's key; a repeat of a non-resource key is an error.
fn note_key(seen: &mut Vec<(String, Option<String>, usize)>, key: &Key, name: Option<&Key>, line: usize) -> Result<(), SatzError> {
    let k = key_text(key);
    if k.starts_with("google_") {
        return Ok(());
    }
    let n = name.map(key_text);
    if let Some((_, _, first)) = seen.iter().find(|(sk, sn, _)| *sk == k && *sn == n) {
        let what = match &n {
            Some(n) => format!("`{} {}`", k, n),
            None => format!("`{}`", k),
        };
        return err(line, format!("{} is given twice in this block (first at line {}) — a repeated key would silently last-win; write a list (`{} = [ … ]`) or remove one", what, first, k));
    }
    seen.push((k, n, line));
    Ok(())
}

/// Satz text as the language reads it: a CRLF line ending is a line ending. A file
/// checked out on Windows compiles to exactly what its LF twin does — without this, a
/// `\r` survived inside `"""` strings and `hcl { }` bodies and moved the emission.
pub fn lf(src: &str) -> std::borrow::Cow<'_, str> {
    if src.contains('\r') {
        std::borrow::Cow::Owned(src.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

/// A statement the block parser meets inside `{ … }` in a shape it cannot read as an
/// entry — `hcl { … }`, which the lexer hands over whole, and `suppress <type> "…"` or
/// `claim "…" "…" "…"`, whose third word is no `{`. The shapes it CAN read as an entry
/// (`params { … }`, `question x { … }`) are refused by the walk, which knows the position.
fn statement_in_a_block(keyword: &str) -> String {
    format!("`{}` is a Satz statement: it is written at the top level of a file, never inside a block — move it out", keyword)
}

/// Every keyword that opens a top-level statement in Satz, sorted.
///
/// The tree-sitter grammar mirrors the parser by hand, and a keyword it has no rule for
/// parses as a resource block rather than an error — a green parse is no proof that the
/// tree means what satz means. `scripts/check-grammar.sh` reads this list and fails by
/// name on a keyword the grammar's `node-types.json` does not declare, so a new statement
/// cannot reach a release ungrammared.
///
/// It is derived, not remembered: `statement_keywords_are_the_parser_s_own_dispatch`
/// below reads the dispatch in `parse` out of this file's source and fails when the two
/// differ.
pub const STATEMENT_KEYWORDS: &[&str] =
    &["action", "claim", "estate", "hcl", "notice", "offers", "pack", "params", "question", "suppress", "use"];

pub fn parse(src: &str) -> Result<File, SatzError> {
    let src = lf(src);
    let toks = lex(&src)?;
    let mut p = P { toks, i: 0 };
    let mut file = File::default();
    loop {
        let line = p.line();
        // ---- statement dispatch: one arm per STATEMENT_KEYWORDS entry ----------------
        match p.peek() {
            None => break,
            Some(Tok::Ident(id)) if id == "estate" || id == "pack" => {
                let is_pack = id == "pack";
                let keyword = if is_pack { "pack" } else { "estate" };
                p.next();
                match p.next() {
                    Some(Tok::Ident(name)) => {
                        if let Some(first) = &file.estate {
                            return err(line, format!("a second `{}` header ({}) — the file is already `{}`", keyword, name, first));
                        }
                        file.estate = Some(name);
                        file.is_pack = is_pack;
                    }
                    other => return err(line, format!("estate: expected a name, found {:?}", other)),
                }
                if matches!(p.peek(), Some(Tok::Ident(m)) if m == "version") {
                    p.next();
                    match p.next() {
                        Some(Tok::Str(parts)) => match parts.as_slice() {
                            [StrPart::Lit(v)] => file.version = Some(v.clone()),
                            _ => return err(line, "version: plain string required"),
                        },
                        other => return err(line, format!("version: expected string, found {:?}", other)),
                    }
                }
                // The header is a name and a version. A word after it on the same line
                // would otherwise open the next block under a key nobody wrote.
                if let Some(Tok::Ident(m)) = p.peek() {
                    if p.line() == line {
                        return err(
                            line,
                            format!("{} header: `{}` is not a header word — the header is `{} <name> [version \"…\"]`; delete `{}`", keyword, m, keyword, m),
                        );
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
                            if file.params.iter().any(|(n, _, _)| n == &name) {
                                return err(line, format!("params: `{}` is declared twice — the second binding would be ignored", name));
                            }
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
            // Before the generic `IDENT IDENT "{"` arm below, which would other-
            // wise swallow `question foo { … }` as a resource map and fail later
            // in the walk with an error about a type nobody wrote.
            Some(Tok::Ident(id)) if id == "question" => {
                p.next();
                let q = p.question_stmt(line)?;
                if let Some(first) = file.questions.iter().find(|x| x.subject == q.subject) {
                    return err(q.line, format!(
                        "question {}: declared twice in this file (line {} and line {})",
                        q.subject, first.line, q.line));
                }
                file.questions.push(q);
            }
            Some(Tok::Ident(id)) if id == "action" => {
                p.next();
                let a = p.action_stmt(line)?;
                if let Some(first) = file.actions.iter().find(|x| x.name == a.name) {
                    return err(
                        a.line,
                        format!(
                            "action \"{}\": declared twice in this file (line {} and line {})",
                            a.name, first.line, a.line
                        ),
                    );
                }
                file.actions.push(a);
            }
            // Before the generic arm, which would read `notice p { … }` as a resource
            // map named `notice`.
            Some(Tok::Ident(id)) if id == "notice" => {
                p.next();
                let n = p.notice_stmt(line)?;
                if let Some(first) = file.notices.iter().find(|x| x.param == n.param) {
                    return err(
                        n.line,
                        format!("notice {}: declared twice in this file (line {} and line {})", n.param, first.line, n.line),
                    );
                }
                file.notices.push(n);
            }
            // Before the generic arm, which would read `offers "p" { … }` as a
            // resource map named `offers`.
            Some(Tok::Ident(id)) if id == "offers" => {
                p.next();
                let o = p.offers_stmt(line)?;
                file.offers.push(o);
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
        // ---- end of the statement dispatch ------------------------------------------
    }

    // A question must live in the file that declares its param. Questions are
    // absorbed AFTER the `use … when` guard, so a question gating a pack would be
    // invisible until the answer was already yes — the chicken-and-egg is made
    // unrepresentable here rather than documented. (The repo already avoided it by
    // accident: `cis_require_shielded_vm` is declared in the CIS pack, not in the
    // extension it gates.)
    // What the library offers is the map's to say: an estate or another pack that
    // offered packs would be a second menu nobody reads.
    if let Some(o) = file.offers.first() {
        if !(file.is_pack && file.estate.as_deref() == Some(MAP_PACK)) {
            return err(
                o.line,
                format!(
                    "offers \"{}\": an `offers` entry belongs in the map (`pack {}`, presets/estate-map.satz) — \
                     it says what the library offers every estate",
                    o.path, MAP_PACK
                ),
            );
        }
    }

    let declared: std::collections::BTreeSet<&str> =
        file.params.iter().map(|(n, _, _)| n.as_str()).collect();
    for q in &file.questions {
        if q.oneof {
            for o in &q.options {
                if !declared.contains(o.param.as_str()) {
                    return err(o.line, format!(
                        "question oneof {}: option {} names no param declared in this file — a \
                         question travels with the param it answers",
                        q.subject, o.param));
                }
            }
        } else if !declared.contains(q.subject.as_str()) {
            return err(q.line, format!(
                "question {}: no param of that name is declared in this file — a question travels \
                 with the param it answers, because a question that gates a pack cannot live in the \
                 gated pack",
                q.subject));
        }
        if let Some(w) = &q.ask_when {
            if !declared.contains(w.as_str()) {
                return err(q.line, format!(
                    "question {}: ask_when names {}, which this file does not declare",
                    q.subject, w));
            }
        }
    }
    // A notice is shown when its pack is switched on, so it lives in a pack; the estate
    // acknowledges it by binding the pack's param, which the pack declares `false`, so an
    // estate that never looked has it open. A question on the same param would make an
    // acknowledgement a customer decision, which it is not.
    for n in &file.notices {
        if !file.is_pack {
            return err(n.line, format!("notice {}: a notice belongs in a pack — it is shown when the pack is switched on", n.param));
        }
        match file.params.iter().find(|(name, _, _)| name == &n.param) {
            Some((_, Value::Bool(false), _)) => {}
            Some((_, _, l)) => {
                return err(
                    *l,
                    format!("notice {}: the param is declared `false` — the estate acknowledges the notice by binding it true", n.param),
                )
            }
            None => {
                return err(
                    n.line,
                    format!("notice {}: no param of that name is declared in this file — declare `{} = false` in its params", n.param, n.param),
                )
            }
        }
        if file.questions.iter().any(|q| q.subject == n.param || q.options.iter().any(|o| o.param == n.param)) {
            return err(n.line, format!("notice {}: a question asks this param — an acknowledgement is no customer decision", n.param));
        }
    }
    Ok(file)
}

/// Params in dependency order (stable Kahn topological sort): a param may reference
/// any other param regardless of declaration order — the emitter linearizes so YAML's
/// backward-only aliases always resolve. Cycles fall back to source order; the
/// pipeline then reports the first unresolvable reference as an unknown param.
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

/// The questions of a file, canonically — a THIRD product beside `canonical_parts`,
/// deliberately not folded into `canonical.body`.
///
/// `canonical` means "what this file emits". A pack whose questions changed emits
/// byte-identical HCL, so putting them in the body would auto-fork every estate that
/// uses the pack over a prompt typo, and report it as "N resource line(s) differ" —
/// which is a lie. Leaving them out of everything would be worse: a
/// `reversal: recreate → edit` downgrade is a governance fact and must not ship
/// silently. So it is reported loudly and separately, and never forks.
pub fn canonical_questions(file: &File) -> String {
    let mut out = String::new();
    let mut qs: Vec<&QuestionDecl> = file.questions.iter().collect();
    qs.sort_by(|a, b| a.subject.cmp(&b.subject));
    for q in qs {
        out.push_str(&format!(
            "question({}|{}|{}|{}|{}|{}|{}|{}|[{}])\n",
            if q.oneof { "oneof" } else { "param" },
            q.subject,
            q.prompt,
            q.why.as_deref().unwrap_or(""),
            q.reversal.as_str(),
            q.blast.as_str(),
            q.recommend.as_ref().map(canon_value).unwrap_or_default(),
            q.ask_when.as_deref().unwrap_or(""),
            q.options
                .iter()
                .map(|o| format!("{}={}", o.param, o.label))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    out
}

/// The `offers` entries of a file, canonically, in file order (the order is the
/// adoption order, so it is meaning). A FOURTH product, beside the questions and
/// for the same reason: an entry emits nothing, so a changed entry is reported and
/// never forks the map the way an emission change would.
pub fn canonical_offers(file: &File) -> String {
    let mut out = String::new();
    for o in &file.offers {
        out.push_str(&format!(
            "offers({}|{}|{}|{}|{}|{}|[{}]|[{}])\n",
            o.path,
            o.when.as_deref().unwrap_or(""),
            o.phase.as_deref().unwrap_or(""),
            o.block.as_deref().unwrap_or(""),
            o.after_scaffold,
            o.by_hand.as_deref().unwrap_or(""),
            o.requires.join(","),
            o.excludes.join(",")
        ));
    }
    out
}

/// The notices of a file, canonically, by param. A FIFTH product, for the reason the
/// questions are one: a notice emits nothing, so a changed wording is reported and never
/// forks an estate that uses the pack.
pub fn canonical_notices(file: &File) -> String {
    let mut ns: Vec<&NoticeDecl> = file.notices.iter().collect();
    ns.sort_by(|a, b| a.param.cmp(&b.param));
    ns.iter().map(|n| format!("notice({}|{}|{}|{})\n", n.param, n.text, n.run, n.before.as_deref().unwrap_or(""))).collect()
}

pub fn canonical_parts(file: &File) -> Canonical {
    // A notice's param is an acknowledgement and never emitted, so it belongs to the
    // notice's canonical form: a pack gaining a notice does not change what it emits.
    let params = file
        .params
        .iter()
        .filter(|(n, _, _)| !file.notices.iter().any(|x| &x.param == n))
        .map(|(n, v, _)| (n.clone(), canon_value(v)))
        .collect();
    let mut body = String::new();
    match (&file.estate, file.is_pack) {
        (Some(n), true) => {
            body.push_str("pack ");
            body.push_str(n);
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
    for a in &file.actions {
        body.push_str(&format!(
            "action({}|{}|{}|[{}]|[{}]|{})\n",
            a.name,
            a.reason,
            a.run,
            a.args.iter().map(|p| canon_str(p)).collect::<Vec<_>>().join(","),
            a.execute_args.iter().map(|p| canon_str(p)).collect::<Vec<_>>().join(","),
            a.phase
        ));
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
    // ---- `question` -------------------------------------------------------

    fn q(body: &str) -> String {
        format!("pack demo version \"1.0\"\n\nparams {{\n  region = \"eu\"\n  a = true\n  b = false\n}}\n\n{}\n", body)
    }

    #[test]
    fn a_question_parses_with_its_two_costs() {
        let f = parse(&q("question region {\n  prompt = \"Which region?\"\n  reversal = state_surgery\n  blast = low\n  recommend = \"europe-west3\"\n}")).expect("parse");
        assert_eq!(f.questions.len(), 1);
        let x = &f.questions[0];
        assert_eq!(x.subject, "region");
        assert!(!x.oneof);
        assert_eq!(x.reversal, Reversal::StateSurgery);
        assert_eq!(x.blast, Blast::Low);
        assert!(x.recommend.is_some());
    }

    /// A question that GATES a pack cannot live in the gated pack — questions are
    /// absorbed after the `use … when` guard, so it would be invisible until the
    /// answer was already yes. Making it a parse error makes that unrepresentable.
    #[test]
    fn a_question_must_travel_with_its_param() {
        let e = parse(&q("question nowhere {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n}")).unwrap_err();
        assert!(e.msg.contains("nowhere"), "{}", e.msg);
        assert!(e.msg.contains("travels with the param it answers"), "{}", e.msg);
    }

    /// Where satz will refuse or warn it must quote the pack's OWN sentence, not a
    /// generic one — the same rule that makes `reason` mandatory on a deviation.
    #[test]
    fn a_one_way_door_must_say_why() {
        let e = parse(&q("question region {\n  prompt = \"?\"\n  reversal = recreate\n  blast = none\n}")).unwrap_err();
        assert!(e.msg.contains("why"), "{}", e.msg);
        let e = parse(&q("question region {\n  prompt = \"?\"\n  reversal = edit\n  blast = high\n}")).unwrap_err();
        assert!(e.msg.contains("why"), "{}", e.msg);
        // and it parses once the sentence is there
        parse(&q("question region {\n  prompt = \"?\"\n  why = \"ids are global\"\n  reversal = recreate\n  blast = none\n}")).expect("parse");
    }

    #[test]
    fn the_cost_vocabularies_are_closed() {
        for (rev, blast) in [("rename", "none"), ("edit", "medium")] {
            let e = parse(&q(&format!(
                "question region {{\n  prompt = \"?\"\n  reversal = {}\n  blast = {}\n}}",
                rev, blast
            )))
            .unwrap_err();
            // the error names the vocabulary rather than saying "invalid"
            assert!(e.msg.contains("is not one of"), "{}/{}: {}", rev, blast, e.msg);
        }
    }

    #[test]
    fn an_unknown_key_is_refused_naming_the_keys() {
        let e = parse(&q("question region {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n  urgency = \"high\"\n}")).unwrap_err();
        assert!(e.msg.contains("prompt, why, reversal"), "{}", e.msg);
    }

    #[test]
    fn prompt_reversal_and_blast_are_all_required() {
        for missing in [
            "question region {\n  reversal = edit\n  blast = none\n}",
            "question region {\n  prompt = \"?\"\n  blast = none\n}",
            "question region {\n  prompt = \"?\"\n  reversal = edit\n}",
        ] {
            assert!(parse(&q(missing)).is_err(), "{}", missing);
        }
    }

    #[test]
    fn a_question_declared_twice_names_both_lines() {
        let e = parse(&q("question region {\n  prompt = \"a\"\n  reversal = edit\n  blast = none\n}\n\nquestion region {\n  prompt = \"b\"\n  reversal = edit\n  blast = none\n}")).unwrap_err();
        assert!(e.msg.contains("declared twice"), "{}", e.msg);
    }

    #[test]
    fn a_oneof_needs_two_branches_and_they_must_be_declared_params() {
        let one = "question oneof m {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n  option a { label = \"A\" }\n}";
        let e = parse(&q(one)).unwrap_err();
        assert!(e.msg.contains("at least two"), "{}", e.msg);

        let undeclared = "question oneof m {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n  option a { label = \"A\" }\n  option zz { label = \"Z\" }\n}";
        let e = parse(&q(undeclared)).unwrap_err();
        assert!(e.msg.contains("zz"), "{}", e.msg);

        let good = "question oneof m {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n  required = true\n  option a { label = \"A\" }\n  option b { label = \"B\" why = \"for the other case\" }\n}";
        let f = parse(&q(good)).expect("parse");
        assert!(f.questions[0].oneof);
        assert!(f.questions[0].required);
        assert_eq!(f.questions[0].options.len(), 2);
    }

    #[test]
    fn option_belongs_to_a_oneof_only() {
        let e = parse(&q("question region {\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n  option a { label = \"A\" }\n}")).unwrap_err();
        assert!(e.msg.contains("`option` belongs to"), "{}", e.msg);
    }

    /// Before this statement existed, `question foo { … }` fell into the generic
    /// `IDENT IDENT "{"` arm, became a resource map, and failed later in the walk
    /// with an error about a type nobody wrote.
    #[test]
    fn a_malformed_question_reports_a_question_error() {
        let e = parse(&q("question region {\n  prompt = \"?\"\n}")).unwrap_err();
        assert!(e.msg.starts_with("question region"), "{}", e.msg);
    }

    /// Questions are a THIRD canonical product. In `canonical.body` a prompt typo
    /// would auto-fork every estate using the pack and report it as a resource
    /// difference; out of everything, a `recreate → edit` downgrade would ship
    /// silently.
    #[test]
    fn questions_are_canonical_separately_from_the_body() {
        let a = parse(&q("question region {\n  prompt = \"A\"\n  reversal = edit\n  blast = none\n}")).unwrap();
        let b = parse(&q("question region {\n  prompt = \"B\"\n  reversal = edit\n  blast = none\n}")).unwrap();
        assert_eq!(canonical(&a), canonical(&b), "a prompt change must not read as an emission change");
        assert_ne!(
            canonical_questions(&a),
            canonical_questions(&b),
            "but it must be visible somewhere"
        );
    }

    #[test]
    fn long_string_param_with_escaped_quotes_round_trips() {
        let src = "pack demo version \"1.1\"\n\nparams {\n  logsink_filter = \"log_id(\\\"cloudaudit.googleapis.com/activity\\\") OR log_id(\\\"cloudaudit.googleapis.com/policy\\\")\"\n}\n\ngoogle_logging_organization_sink {\n  s {\n    filter = logsink_filter\n  }\n}\n";
        let f = parse(src).expect("parse");
        let (_, v, _) = f.params.iter().find(|(n, _, _)| n == "logsink_filter").expect("param");
        // the escaped quotes survive as quotes in the param value
        let s = format!("{:?}", v);
        assert!(s.contains("log_id(\\\"cloudaudit.googleapis.com/activity\\\")"), "{}", s);
    }

    #[test]
    fn pack_header_takes_a_version_and_no_other_word() {
        let f = parse("pack demo version \"2.1\"\n\nparams {\n  a = \"1\"\n}\n").unwrap();
        assert_eq!(f.version.as_deref(), Some("2.1"));
        assert_eq!(f.params.len(), 1);
        // A word on the header's line would open the next block under a key nobody
        // wrote: `content params { … }` parses, and the params are gone.
        for src in [
            "pack demo version \"1.0\" content\n\nparams {\n  a = \"1\"\n}\n",
            "pack demo content version \"1.0\"\n\nparams {\n  a = \"1\"\n}\n",
        ] {
            let e = parse(src).expect_err(src);
            assert_eq!(e.line, 1, "{}", src);
            assert!(e.msg.contains("`content` is not a header word") && e.msg.contains("delete `content`"), "{}: {}", src, e.msg);
        }
        // a label on its own line is a label, whatever it is called
        let f = parse("pack demo version \"1.0\"\ncontent {\n  a = \"1\"\n}\n").unwrap();
        assert_eq!(f.items.len(), 1);
    }

    #[test]
    fn a_statement_inside_a_block_is_named_as_one() {
        for (src, kw) in [
            ("estate e\ngoogle_x {\n  hcl {\n    # raw\n  }\n}\n", "hcl"),
            ("estate e\ngoogle_x {\n  suppress google_y \"z\"\n}\n", "suppress"),
            ("estate e\ngoogle_x {\n  claim \"f\" \"1\" \"1.1\" implements {\n  }\n}\n", "claim"),
        ] {
            let e = parse(src).expect_err(src);
            assert_eq!(e.line, 3, "{}", src);
            assert!(e.msg.contains(&format!("`{}` is a Satz statement", kw)), "{}: {}", src, e.msg);
        }
    }

    use super::*;


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


    /// An action is the deployment's `unsafe` block: satz will execute it, so the
    /// declaration has to carry everything a reader needs to judge it before it runs.
    #[test]
    fn an_action_is_parsed_with_its_two_argument_lists() {
        let f = parse(
            r#"
estate demo
action "scc-services" {
  reason       = "SCC service enablement has no provider resource"
  run          = "scc-enable-all.sh"
  args         = ["--organization", "{customer_organization_id}"]
  execute_args = ["--apply"]
  phase        = "before-apply"
}
"#,
        )
        .unwrap();
        assert_eq!(f.actions.len(), 1, "{:?}", f.actions);
        let a = &f.actions[0];
        assert_eq!(a.name, "scc-services");
        assert_eq!(a.run, "scc-enable-all.sh");
        assert_eq!(a.phase, "before-apply");
        assert_eq!(a.args.len(), 2);
        // The second argument interpolates; the first is literal.
        assert_eq!(a.args[0], vec![StrPart::Lit("--organization".to_string())]);
        assert_eq!(a.args[1], vec![StrPart::Param("customer_organization_id".to_string())]);
        assert_eq!(a.execute_args, vec![vec![StrPart::Lit("--apply".to_string())]]);
    }

    #[test]
    fn an_action_defaults_to_after_apply_and_rejects_any_other_phase() {
        let f = parse("action \"a\" {\n  reason = \"r\"\n  run = \"x.sh\"\n}\n").unwrap();
        assert_eq!(f.actions[0].phase, "after-apply");
        let e = parse("action \"a\" {\n  reason = \"r\"\n  run = \"x.sh\"\n  phase = \"whenever\"\n}\n")
            .unwrap_err();
        assert!(e.msg.contains("before-apply"), "{}", e.msg);
    }

    /// Both are mandatory: `run` because there is nothing to do without it, `reason`
    /// because it is what the execution warning quotes back to whoever is about to
    /// run the thing.
    #[test]
    fn an_action_without_a_reason_or_a_run_is_refused() {
        let e = parse("action \"a\" {\n  run = \"x.sh\"\n}\n").unwrap_err();
        assert!(e.msg.contains("reason"), "{}", e.msg);
        let e = parse("action \"a\" {\n  reason = \"r\"\n}\n").unwrap_err();
        assert!(e.msg.contains("run"), "{}", e.msg);
    }

    #[test]
    fn an_action_name_and_run_path_may_not_interpolate() {
        let e = parse("action \"{x}\" {\n  reason = \"r\"\n  run = \"x.sh\"\n}\n").unwrap_err();
        assert!(e.msg.contains("no interpolation"), "{}", e.msg);
        let e = parse("action \"a\" {\n  reason = \"r\"\n  run = \"{x}.sh\"\n}\n").unwrap_err();
        assert!(e.msg.contains("no interpolation"), "{}", e.msg);
    }

    #[test]
    fn two_actions_of_one_name_in_one_file_name_both_lines() {
        let e = parse(
            "action \"a\" {\n  reason = \"r\"\n  run = \"x.sh\"\n}\naction \"a\" {\n  reason = \"r\"\n  run = \"y.sh\"\n}\n",
        )
        .unwrap_err();
        assert!(e.msg.contains("declared twice"), "{}", e.msg);
    }

    #[test]
    fn an_unknown_action_key_is_refused_rather_than_ignored() {
        let e = parse("action \"a\" {\n  reason = \"r\"\n  run = \"x.sh\"\n  timeout = \"30\"\n}\n")
            .unwrap_err();
        assert!(e.msg.contains("reason, run, args, execute_args, phase"), "{}", e.msg);
    }

    /// `check-presets` compares packs by canonical form. An action that did not
    /// appear there would let a pack grow — or lose — an executable step while the
    /// drift check called the two versions identical.
    #[test]
    fn the_canonical_form_carries_the_action() {
        let with = parse("pack p\naction \"a\" {\n  reason = \"r\"\n  run = \"x.sh\"\n}\n").unwrap();
        let without = parse("pack p\n").unwrap();
        let changed = parse("pack p\naction \"a\" {\n  reason = \"r\"\n  run = \"y.sh\"\n}\n").unwrap();
        assert_ne!(canonical(&with), canonical(&without));
        assert_ne!(canonical(&with), canonical(&changed));
    }

    #[test]
    fn claim_without_witnesses_is_rejected() {
        let e = parse("claim \"cis-gcp\" \"4.0\" \"2.2\" implements { interpretation = \"x\" }")
            .unwrap_err();
        assert!(e.msg.contains("witnesses"), "{e}");
    }

    #[test]
    fn satz_pack_uses_are_reported() {
        let f = parse("use \"packs/logsink.satz\"\nuse \"plain.yaml\"\n").unwrap();
        assert_eq!(use_paths(&f), vec!["packs/logsink.satz".to_string()]);
    }

    #[test]
    fn errors_carry_line_numbers() {
        let e = parse("params {\n  broken =\n}").unwrap_err();
        assert_eq!(e.line, 3, "{e}"); // value missing, found '}' on line 3
        let e = parse("x = ").unwrap_err();
        assert!(e.line >= 1);
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
    fn hcl_blocks_are_collected() {
        let f = parse("estate e\nhcl {\n  output \"x\" { value = 1 }\n}\n").unwrap();
        assert_eq!(f.hcl_blocks.len(), 1);
        assert!(parse("estate e\n").unwrap().hcl_blocks.is_empty());
    }

}

#[cfg(test)]
mod empty_collection_tests {
    use super::*;

    /// An empty list param is an empty list — it used to vanish into null on
    /// the way through the (now gone) YAML twin.
    #[test]
    fn empty_list_param_stays_an_empty_list() {
        let src = "pack p\n\nparams {\n  subjects = []\n}\n\n\
                   google_org_policy_policy {\n  x {\n    name = \"c\"\n    members = subjects\n  }\n}\n";
        let f = parse(src).expect("parse");
        let c = canonical(&f);
        assert!(c.contains("subjects=[]"), "canonical must show the empty list:\n{}", c);
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

/// One minimal file per statement keyword, each carrying that statement. Shared by the
/// tests that hold a per-statement table against `STATEMENT_KEYWORDS`.
#[cfg(test)]
pub(crate) fn statement_probe(kw: &str) -> String {
    match kw {
        "estate" => "estate e\n".into(),
        "pack" => "pack p version \"1.0\"\n".into(),
        "params" => "estate e\nparams { a = \"1\" }\n".into(),
        "use" => "estate e\nuse \"p.satz\"\n".into(),
        "hcl" => "estate e\nhcl {\n  # raw\n}\n".into(),
        "claim" => "estate e\nclaim \"f\" \"1\" \"1.1\" implements {\n  resources = [\"google_x.y\"]\n}\n".into(),
        "question" => {
            "pack p version \"1.0\"\nparams { a = false }\nquestion a {\n  prompt   = \"?\"\n  reversal = edit\n  blast    = none\n}\n".into()
        }
        "action" => "estate e\naction \"a\" {\n  reason = \"no provider resource does it\"\n  run    = \"a.sh\"\n}\n".into(),
        "notice" => "pack p version \"1.0\"\nparams { a = false }\nnotice a {\n  text = \"t\"\n  run = \"satz adopt\"\n  before = apply\n}\n".into(),
        "offers" => "pack estate_map\noffers \"presets/a.satz\" {\n  when = use_a\n}\n".into(),
        "suppress" => "estate e\nsuppress google_x \"y\"\n".into(),
        other => panic!("no probe for the statement `{}` — add one", other),
    }
}

/// `STATEMENT_KEYWORDS` against the parser it is taken from. The list leaves this crate:
/// `scripts/check-grammar.sh` fails on a keyword the tree-sitter grammar declares no node
/// for, so a statement missing here ships ungrammared — which is what `offers` did.
#[cfg(test)]
mod statement_set_tests {
    use super::*;

    /// The keywords the dispatch in `parse` matches on, read out of this file's own
    /// source between the two markers: every `id == "…"` guard, plus `hcl`, whose block
    /// the lexer hands over as one token. The dispatch binds the keyword as `id` and
    /// nothing else in it does — the header's `version` binds `m`, because it opens no
    /// statement.
    fn dispatched_keywords() -> std::collections::BTreeSet<String> {
        const SRC: &str = include_str!("satz.rs");
        // spelled in halves so this function's own source is not the first match
        let begin = concat!("// ---- statement ", "dispatch: one arm per STATEMENT_KEYWORDS entry");
        let end = concat!("// ---- end of the statement ", "dispatch ---");
        let (_, rest) = SRC.split_once(begin).expect("the dispatch in `parse` carries its begin marker");
        let (body, _) = rest.split_once(end).expect("the dispatch in `parse` carries its end marker");
        let mut out = std::collections::BTreeSet::new();
        if body.contains("Tok::Hcl(") {
            out.insert("hcl".to_string());
        }
        let mut rest = body;
        while let Some(at) = rest.find("id == \"") {
            rest = &rest[at + "id == \"".len()..];
            let Some(close) = rest.find('"') else { break };
            out.insert(rest[..close].to_string());
            rest = &rest[close..];
        }
        out
    }

    /// `STATEMENT_KEYWORDS` is what `parse` dispatches on, and nothing else.
    #[test]
    fn statement_keywords_are_the_parser_s_own_dispatch() {
        let derived = dispatched_keywords();
        let listed: std::collections::BTreeSet<String> = STATEMENT_KEYWORDS.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            derived, listed,
            "STATEMENT_KEYWORDS and the dispatch in `parse` disagree — add the keyword to the list \
             (and a rule for it to the tree-sitter grammar, which scripts/check-grammar.sh checks)"
        );
        let mut sorted: Vec<&&str> = STATEMENT_KEYWORDS.iter().collect();
        sorted.sort();
        assert_eq!(sorted, STATEMENT_KEYWORDS.iter().collect::<Vec<_>>(), "STATEMENT_KEYWORDS is sorted");
    }

    /// Every listed keyword is a statement to the parser, not a resource block: the shape
    /// that hid `offers` from the grammar gate hides a stale entry from this list.
    #[test]
    fn every_statement_keyword_parses_as_a_statement_not_a_block() {
        for kw in STATEMENT_KEYWORDS {
            let src = statement_probe(kw);
            let f = parse(&src).unwrap_or_else(|e| panic!("`{}` probe: {}:{}", kw, e.line, e.msg));
            let as_block = f.items.iter().any(|e| match e {
                Entry::Map { key: Key::Ident(k), .. } | Entry::Attr { key: Key::Ident(k), .. } => k == kw,
                _ => false,
            });
            assert!(!as_block, "`{}` parsed as a resource block, so it is no statement", kw);
        }
    }
}

#[cfg(test)]
mod review_2026_08_29_tests {
    use super::*;

    #[test]
    fn duplicate_params_and_headers_are_errors() {
        assert!(parse("estate e\nparams { a = \"1\" a = \"2\" }\n").unwrap_err().msg.contains("declared twice"));
        assert!(parse("estate e\nestate f\n").unwrap_err().msg.contains("second `estate` header"));
        assert!(parse("estate e\nuse \"p\" as x as y\n").unwrap_err().msg.contains("given twice"));
    }

    #[test]
    fn malformed_numbers_are_errors_and_block_comments_lex() {
        assert!(parse("estate e\ngoogle_x { a { v = 1.2.3 } }\n").unwrap_err().msg.contains("malformed number"));
        assert!(parse("estate e\ngoogle_x { a { v = 1. } }\n").unwrap_err().msg.contains("malformed number"));
        let f = parse("estate e\n/* a block\n   comment */\ngoogle_x { a { v = 1 } }\n").unwrap();
        assert_eq!(f.items.len(), 1);
        assert!(parse("estate e\n/* never closed\n").unwrap_err().msg.contains("unterminated"));
    }

    #[test]
    fn a_repeated_key_in_one_body_is_an_error_but_resource_maps_may_repeat() {
        let e = parse("estate e\ngoogle_storage_bucket { b { name = \"x\" lifecycle_rule { action { type = \"Delete\" } } lifecycle_rule { action { type = \"Delete\" } } } }\n").unwrap_err();
        assert!(e.msg.contains("`lifecycle_rule` is given twice") && e.msg.contains("first at line 2"), "{}", e.msg);
        let e = parse("estate e\ngoogle_storage_bucket { b { name = \"x\"\n name = \"y\" } }\n").unwrap_err();
        assert!(e.msg.contains("`name` is given twice"), "{}", e.msg);
        parse("estate e\ngoogle_org_policy_policy { a { name = \"a\" } }\ngoogle_org_policy_policy { b { name = \"b\" } }\n").expect("two groups of one resource type are one map");
        parse("estate e\ngoogle_folder { a { display_name = \"A\" } b { display_name = \"B\" } }\n").expect("different labels");
    }

    // ---- `offers` ---------------------------------------------------------

    #[test]
    fn the_map_offers_packs_in_file_order_and_nothing_else_may() {
        let map = "pack estate_map version \"1.0\"\n\nparams {\n  use_a = true\n}\n\noffers \"presets/a.satz\" {\n  when     = use_a\n  phase    = \"\"\"first\nsecond\"\"\"\n  block    = \"google_folder.infra_folder\"\n  excludes = [\"presets/b.satz\"]\n}\n\noffers \"presets/b.satz\" {\n  when    = use_a\n  by_hand = \"why\"\n}\n";
        let f = parse(map).unwrap();
        assert_eq!(f.offers.len(), 2);
        let a = &f.offers[0];
        assert_eq!((a.path.as_str(), a.when.as_deref(), a.phase.as_deref()), ("presets/a.satz", Some("use_a"), Some("first\nsecond")));
        assert_eq!(a.block.as_deref(), Some("google_folder.infra_folder"));
        assert_eq!(a.excludes, vec!["presets/b.satz".to_string()]);
        assert_eq!(f.offers[1].by_hand.as_deref(), Some("why"));
        assert!(f.items.is_empty(), "an entry is no resource map");
        // the order is the adoption order: swapping two entries is a change
        let swapped = map.replacen("presets/a.satz\" {", "presets/c.satz\" {", 1);
        assert_ne!(canonical_offers(&f), canonical_offers(&parse(&swapped).unwrap()));
        assert_eq!(canonical(&f), canonical(&parse(&swapped).unwrap()), "an entry emits nothing");
        // anywhere but the map it is refused
        let e = parse("pack other version \"1.0\"\n\noffers \"presets/a.satz\" {\n  when = x\n}\n").unwrap_err();
        assert!(e.msg.contains("belongs in the map"), "{}", e.msg);
        let e = parse("estate e\n\noffers \"presets/a.satz\" {\n  when = x\n}\n").unwrap_err();
        assert!(e.msg.contains("belongs in the map"), "{}", e.msg);
        // an unknown key, and a line in two places
        let e = parse("pack estate_map\n\noffers \"presets/a.satz\" {\n  gate = x\n}\n").unwrap_err();
        assert!(e.msg.contains("the keys are when"), "{}", e.msg);
        let e = parse("pack estate_map\n\noffers \"presets/a.satz\" {\n  block = \"b\"\n  after_scaffold = true\n}\n").unwrap_err();
        assert!(e.msg.contains("not both"), "{}", e.msg);
    }

    // ---- `notice` ---------------------------------------------------------

    #[test]
    fn a_notice_travels_with_its_false_param_and_emits_nothing() {
        let pack = "pack p version \"1.0\"\n\nparams {\n  p_adopted = false\n}\n\nnotice p_adopted {\n  text   = \"Adopt what exists.\"\n  run    = \"satz adopt <estate> --execute --import\"\n  before = apply\n}\n";
        let f = parse(pack).unwrap();
        assert_eq!(f.notices.len(), 1);
        let n = &f.notices[0];
        assert_eq!((n.param.as_str(), n.before.as_deref()), ("p_adopted", Some("apply")));
        assert!(f.items.is_empty(), "a notice is no resource map");
        // the notice and its param are outside what the pack emits
        let bare = parse("pack p version \"1.0\"\n").unwrap();
        assert_eq!(canonical(&f), canonical(&bare), "a notice emits nothing");
        let reworded = parse(&pack.replace("Adopt what exists.", "Adopt it.")).unwrap();
        assert_ne!(canonical_notices(&f), canonical_notices(&reworded));
        // the param: declared here, `false`, asked by no question
        let e = parse("pack p\n\nnotice x {\n  text = \"t\"\n  run = \"r\"\n}\n").unwrap_err();
        assert!(e.msg.contains("no param of that name"), "{}", e.msg);
        let e = parse(&pack.replace("p_adopted = false", "p_adopted = true")).unwrap_err();
        assert!(e.msg.contains("declared `false`"), "{}", e.msg);
        let asked = format!("{}\nquestion p_adopted {{\n  prompt = \"?\"\n  reversal = edit\n  blast = none\n}}\n", pack);
        assert!(parse(&asked).unwrap_err().msg.contains("no customer decision"));
        // only in a pack, once, with its two texts and only `before = apply`
        let e = parse(&pack.replace("pack p version \"1.0\"", "estate e")).unwrap_err();
        assert!(e.msg.contains("belongs in a pack"), "{}", e.msg);
        let twice = format!("{}\nnotice p_adopted {{\n  text = \"t\"\n  run = \"r\"\n}}\n", pack);
        assert!(parse(&twice).unwrap_err().msg.contains("declared twice"));
        assert!(parse(&pack.replace("  run    = \"satz adopt <estate> --execute --import\"\n", "")).unwrap_err().msg.contains("run = "));
        assert!(parse(&pack.replace("before = apply", "before = plan")).unwrap_err().msg.contains("`apply`"));
        assert!(parse(&pack.replace("before = apply", "when = apply")).unwrap_err().msg.contains("the keys are"));
    }
}
