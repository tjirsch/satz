//! `satz fmt`: the canonical layout of a Satz file, without changing what it means.
//!
//! The formatter works on the token stream with its trivia — comments and line
//! ends — not on the AST: the AST has no comments and no columns, and a printer
//! over it would have to invent every line break. Here the author's line breaks
//! are the input. What the formatter decides is indentation (two spaces per open
//! brace or bracket that spans lines), the spacing inside a line, `=` alignment
//! over a run of attributes, the commas of a list, and that a construct spanning
//! lines opens at the end of its line and closes on a line of its own. A string
//! and an `hcl { … }` body are verbatim.
//!
//! What `satz fmt` never does is change meaning: `check-presets` compares the
//! canonical form of the parsed file, and that form is the formatter's law —
//! `canonical(parse(format(x))) == canonical(parse(x))` over the whole corpus.

use crate::satz::{lex_spanned, parse, SatzError, Tok, Token};

const INDENT: &str = "  ";

/// Format one Satz source. A file the parser refuses is not formatted: the
/// error is the parser's.
pub fn format(src: &str) -> Result<String, SatzError> {
    // the layout is LF: a CRLF file formats to what its LF twin formats to
    let src = &*crate::satz::lf(src);
    parse(src)?;
    let chars: Vec<char> = src.chars().collect();
    let toks = lex_spanned(src, true)?;
    let multiline = spans_lines(&toks)?;
    let lines = build_lines(&toks, &chars, &multiline);
    let lines = drop_blank_edges(lines);
    Ok(render(&lines))
}

/// Whether the file is already in its canonical layout.
pub fn is_formatted(src: &str) -> Result<bool, SatzError> {
    // line endings are not layout: a formatted file checked out with CRLF is formatted
    Ok(format(src)? == crate::satz::lf(src))
}

// ---------------------------------------------------------------------------
// Pass 1: which openers span lines
// ---------------------------------------------------------------------------

/// For every `{` / `[`, whether a line end sits between it and its closer. Such a
/// construct is laid out over lines; one written on a single line stays inline.
fn spans_lines(toks: &[Token]) -> Result<Vec<bool>, SatzError> {
    let mut out = vec![false; toks.len()];
    let mut stack: Vec<(usize, bool)> = Vec::new(); // (opener index, saw a newline)
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::LBrace | Tok::LBrack => stack.push((i, false)),
            Tok::RBrace | Tok::RBrack => {
                let (open, nl) = stack
                    .pop()
                    .ok_or_else(|| SatzError { line: t.line, msg: "unbalanced closing bracket".into() })?;
                out[open] = nl;
            }
            Tok::Newline => {
                for s in stack.iter_mut() {
                    s.1 = true;
                }
            }
            _ => {}
        }
    }
    if let Some((open, _)) = stack.first() {
        return Err(SatzError { line: toks[*open].line, msg: "unbalanced opening bracket".into() });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Pass 2: lines of pieces
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Piece {
    tok: Tok,
    text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Blank,
    /// A line that is only a comment.
    Comment,
    /// `key = value`, the value complete on this line — what `=` alignment acts on.
    Assign,
    Other,
}

struct Line {
    depth: usize,
    kind: Kind,
    /// Rendered, without indent and without the trailing comment.
    text: String,
    key: String,
    value: String,
    comment: Option<String>,
    starts_with_closer: bool,
    ends_with_opener: bool,
}

fn raw(t: &Token, chars: &[char]) -> String {
    chars[t.start..t.end].iter().collect()
}

fn is_value_end(tok: &Tok) -> bool {
    matches!(tok, Tok::Str(_) | Tok::Num(_) | Tok::Ident(_) | Tok::RBrack | Tok::RBrace)
}

/// The next token that is neither a comment nor a line end.
fn next_significant(toks: &[Token], from: usize) -> Option<&Tok> {
    toks[from..].iter().map(|t| &t.tok).find(|t| !matches!(t, Tok::Comment(_) | Tok::Newline))
}

fn build_lines(toks: &[Token], chars: &[char], multiline: &[bool]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Vec<Piece> = Vec::new();
    let mut cur_depth = 0usize;
    let mut open: Vec<usize> = Vec::new(); // every open construct, by opener index
    let mut ml_open: Vec<usize> = Vec::new(); // the ones laid out over lines
    let mut break_pending = false; // a multi-line opener was just written

    let comma = Piece { tok: Tok::Comma, text: ",".into() };

    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::Newline => {
                push_line(&mut lines, std::mem::take(&mut cur), cur_depth);
                break_pending = false;
            }
            Tok::Comment(_) => {
                if cur.is_empty() {
                    cur_depth = ml_open.len();
                }
                cur.push(Piece { tok: t.tok.clone(), text: raw(t, chars).trim_end().to_string() });
            }
            Tok::Comma => {
                // A comma is written where a list item ends (below); one the author
                // wrote is kept only where it is not already there — and an inline
                // list carries none before its `]`.
                let inline_list = open.last().is_some_and(|&o| matches!(toks[o].tok, Tok::LBrack) && !multiline[o]);
                let before_close = matches!(next_significant(toks, i + 1), Some(Tok::RBrack));
                let already = cur.last().is_some_and(|p| matches!(p.tok, Tok::Comma));
                if already || (inline_list && before_close) {
                    continue;
                }
                cur.push(comma.clone());
            }
            Tok::LBrace | Tok::LBrack => {
                if break_pending {
                    push_line(&mut lines, std::mem::take(&mut cur), cur_depth);
                    break_pending = false;
                }
                if cur.is_empty() {
                    cur_depth = ml_open.len();
                }
                cur.push(Piece { tok: t.tok.clone(), text: raw(t, chars) });
                open.push(i);
                if multiline[i] {
                    ml_open.push(i);
                    break_pending = true;
                }
            }
            Tok::RBrace | Tok::RBrack => {
                let opener = open.pop().expect("balanced by spans_lines");
                if multiline[opener] {
                    ml_open.pop();
                    if !cur.is_empty() {
                        push_line(&mut lines, std::mem::take(&mut cur), cur_depth);
                    }
                }
                break_pending = false;
                if cur.is_empty() {
                    cur_depth = ml_open.len();
                }
                cur.push(Piece { tok: t.tok.clone(), text: raw(t, chars) });
                end_of_list_item(toks, i, &open, multiline, &mut cur, &comma);
            }
            Tok::Str(_) | Tok::Num(_) | Tok::Ident(_) | Tok::Hcl(..) => {
                if break_pending {
                    push_line(&mut lines, std::mem::take(&mut cur), cur_depth);
                    break_pending = false;
                }
                if cur.is_empty() {
                    cur_depth = ml_open.len();
                }
                cur.push(Piece { tok: t.tok.clone(), text: raw(t, chars) });
                end_of_list_item(toks, i, &open, multiline, &mut cur, &comma);
            }
            Tok::Eq => {
                if cur.is_empty() {
                    cur_depth = ml_open.len();
                }
                cur.push(Piece { tok: Tok::Eq, text: "=".into() });
            }
        }
    }
    push_line(&mut lines, cur, cur_depth);
    lines
}

/// After a token that ends a list item: write the comma the item needs. Every
/// item of a list laid out over lines carries one, the last included; in an
/// inline list the items are comma-separated and the last carries none.
fn end_of_list_item(toks: &[Token], i: usize, open: &[usize], multiline: &[bool], cur: &mut Vec<Piece>, comma: &Piece) {
    if !is_value_end(&toks[i].tok) {
        return;
    }
    let Some(&o) = open.last() else { return };
    if !matches!(toks[o].tok, Tok::LBrack) {
        return;
    }
    match next_significant(toks, i + 1) {
        Some(Tok::Comma) => {} // the author's comma follows and is kept
        Some(Tok::RBrack) if !multiline[o] => {}
        _ => cur.push(comma.clone()),
    }
}

fn push_line(lines: &mut Vec<Line>, pieces: Vec<Piece>, depth: usize) {
    if pieces.is_empty() {
        // One blank line at most, and none before the first line.
        if lines.last().is_some_and(|l| l.kind != Kind::Blank) {
            lines.push(Line {
                depth: 0,
                kind: Kind::Blank,
                text: String::new(),
                key: String::new(),
                value: String::new(),
                comment: None,
                starts_with_closer: false,
                ends_with_opener: false,
            });
        }
        return;
    }
    let (body, comment) = match pieces.last() {
        Some(p) if matches!(p.tok, Tok::Comment(_)) && pieces.len() > 1 => {
            (&pieces[..pieces.len() - 1], Some(pieces.last().unwrap().text.clone()))
        }
        _ => (&pieces[..], None),
    };
    if body.len() == 1 && matches!(body[0].tok, Tok::Comment(_)) {
        lines.push(Line {
            depth,
            kind: Kind::Comment,
            text: body[0].text.clone(),
            key: String::new(),
            value: String::new(),
            comment: None,
            starts_with_closer: false,
            ends_with_opener: false,
        });
        return;
    }
    // `key = value`, and `export "name" = value [description "…"]`, whose key is the
    // keyword and the name: a run of exports aligns its `=` like a run of attributes.
    let key_len = match body {
        [Piece { tok: Tok::Ident(kw), .. }, Piece { tok: Tok::Str(_), .. }, Piece { tok: Tok::Eq, .. }, rest @ ..]
            if kw == "export" && (is_complete_value(export_value(rest)) || is_all(export_value(rest))) =>
        {
            2
        }
        [Piece { tok: Tok::Ident(_) | Tok::Str(_), .. }, Piece { tok: Tok::Eq, .. }, rest @ ..] if is_complete_value(rest) => 1,
        _ => 0,
    };
    let assign = key_len > 0;
    lines.push(Line {
        depth,
        kind: if assign { Kind::Assign } else { Kind::Other },
        text: join(body),
        key: if assign { join(&body[..key_len]) } else { String::new() },
        value: if assign { join(&body[key_len + 1..]) } else { String::new() },
        comment,
        starts_with_closer: matches!(body[0].tok, Tok::RBrace | Tok::RBrack),
        ends_with_opener: matches!(body.last().map(|p| &p.tok), Some(Tok::LBrace | Tok::LBrack)),
    });
}

/// An export's value without the `description "…"` and `attach [ … ]` that may follow it,
/// in either order.
fn export_value(rest: &[Piece]) -> &[Piece] {
    let mut value = rest;
    loop {
        match value {
            [v @ .., Piece { tok: Tok::Ident(d), .. }, Piece { tok: Tok::Str(_), .. }] if d == "description" => value = v,
            [.., Piece { tok: Tok::RBrack, .. }] => {
                // the `[` that opens the trailing list, and the keyword before it
                let mut depth = 0usize;
                let open = value.iter().rposition(|x| {
                    match x.tok {
                        Tok::RBrack => depth += 1,
                        Tok::LBrack => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                match open {
                    Some(o) if o > 0 && matches!(&value[o - 1].tok, Tok::Ident(a) if a == "attach") => value = &value[..o - 1],
                    _ => return value,
                }
            }
            _ => return value,
        }
    }
}

/// `all <type> [under <address>]`, an export's value that is every resource of a type.
fn is_all(p: &[Piece]) -> bool {
    match p {
        [Piece { tok: Tok::Ident(a), .. }, Piece { tok: Tok::Ident(_), .. }] => a == "all",
        [Piece { tok: Tok::Ident(a), .. }, Piece { tok: Tok::Ident(_), .. }, Piece { tok: Tok::Ident(u), .. }, Piece { tok: Tok::Ident(_), .. }] => a == "all" && u == "under",
        _ => false,
    }
}

/// One value, whole on this line: a scalar, or a bracket group that closes here.
fn is_complete_value(p: &[Piece]) -> bool {
    match p.len() {
        0 => false,
        1 => matches!(p[0].tok, Tok::Str(_) | Tok::Num(_) | Tok::Ident(_)),
        _ => {
            if !matches!(p[0].tok, Tok::LBrack | Tok::LBrace) {
                return false;
            }
            let mut depth = 0usize;
            for (k, x) in p.iter().enumerate() {
                match x.tok {
                    Tok::LBrack | Tok::LBrace => depth += 1,
                    Tok::RBrack | Tok::RBrace => {
                        depth -= 1;
                        if depth == 0 {
                            return k == p.len() - 1;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
    }
}

/// The spacing inside a line: `key = value`, `[a, b]`, `{ a = 1 }`, `{}`.
fn join(p: &[Piece]) -> String {
    let mut out = String::new();
    for (k, x) in p.iter().enumerate() {
        if k > 0 {
            out.push_str(sep(&p[k - 1].tok, &x.tok));
        }
        out.push_str(&x.text);
    }
    out
}

fn sep(prev: &Tok, next: &Tok) -> &'static str {
    match (prev, next) {
        (Tok::LBrack, _) | (_, Tok::RBrack) | (_, Tok::Comma) | (Tok::LBrace, Tok::RBrace) => "",
        _ => " ",
    }
}

// ---------------------------------------------------------------------------
// Pass 3: blank lines, alignment, output
// ---------------------------------------------------------------------------

/// No blank line right after an opener, right before a closer, or at the end.
fn drop_blank_edges(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for l in lines {
        if l.kind == Kind::Blank && out.last().is_some_and(|p| p.ends_with_opener) {
            continue;
        }
        if l.starts_with_closer && out.last().is_some_and(|p| p.kind == Kind::Blank) {
            out.pop();
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.kind == Kind::Blank) {
        out.pop();
    }
    out
}

fn width(s: &str) -> usize {
    s.chars().count()
}

fn render(lines: &[Line]) -> String {
    let mut texts: Vec<String> = lines.iter().map(|l| l.text.clone()).collect();

    // `=` alignment over a run of attributes at one depth. A comment line inside
    // the run is transparent; a blank line or anything else ends it.
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != Kind::Assign {
            i += 1;
            continue;
        }
        let depth = lines[i].depth;
        let mut j = i;
        while j < lines.len()
            && lines[j].depth == depth
            && matches!(lines[j].kind, Kind::Assign | Kind::Comment)
        {
            j += 1;
        }
        let run: Vec<usize> = (i..j).filter(|&k| lines[k].kind == Kind::Assign).collect();
        let key_w = run.iter().map(|&k| width(&lines[k].key)).max().unwrap_or(0);
        for &k in &run {
            texts[k] = format!("{:<key_w$} = {}", lines[k].key, lines[k].value);
        }
        // Trailing comments in the run line up too, one space past the widest
        // commented line.
        let commented: Vec<usize> = run.iter().copied().filter(|&k| lines[k].comment.is_some()).collect();
        let text_w = commented.iter().map(|&k| width(&texts[k])).max().unwrap_or(0);
        for &k in &commented {
            let pad = text_w - width(&texts[k]);
            texts[k] = format!("{}{} {}", texts[k], " ".repeat(pad), lines[k].comment.as_ref().unwrap());
        }
        i = j;
    }

    let mut out = String::new();
    for (k, l) in lines.iter().enumerate() {
        if l.kind == Kind::Blank {
            out.push('\n');
            continue;
        }
        out.push_str(&INDENT.repeat(l.depth));
        out.push_str(&texts[k]);
        if l.kind != Kind::Assign {
            if let Some(c) = &l.comment {
                out.push(' ');
                out.push_str(c);
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satz::{canonical, parse};

    fn fmt(s: &str) -> String {
        format(s).unwrap()
    }

    #[test]
    fn indentation_and_spacing_are_normalised() {
        let src = "google_folder{\ninfra   {\n      display_name=\"Infrastructure\"\n}\n}\n";
        assert_eq!(fmt(src), "google_folder {\n  infra {\n    display_name = \"Infrastructure\"\n  }\n}\n");
    }

    #[test]
    fn equals_align_over_a_run_and_a_blank_line_ends_it() {
        let src = "params {\n  a = 1\n  long_name = 2\n\n  b = 3\n}\n";
        assert_eq!(fmt(src), "params {\n  a         = 1\n  long_name = 2\n\n  b = 3\n}\n");
    }

    #[test]
    fn a_comment_line_is_transparent_and_trailing_comments_line_up() {
        let src = "params {\n  a = 1 # one\n  // between\n  long_name = \"two\" # two\n  c = 3\n}\n";
        assert_eq!(
            fmt(src),
            "params {\n  a         = 1     # one\n  // between\n  long_name = \"two\" # two\n  c         = 3\n}\n"
        );
    }

    #[test]
    fn lists_over_lines_get_their_commas_and_inline_lists_lose_the_last() {
        let src = "a = [\n  \"x\"\n  \"y\",\n]\nb = [ \"x\" \"y\", ]\nc = []\n";
        assert_eq!(fmt(src), "a = [\n  \"x\",\n  \"y\",\n]\nb = [\"x\", \"y\"]\nc = []\n");
    }

    #[test]
    fn an_inline_block_stays_inline_and_a_split_one_opens_and_closes_on_its_own_lines() {
        let src = "x {\n  local { path = \"t\" }\n  rule = [ { action { type = \"Delete\" }\n    condition { age = 3 } }, ]\n}\n";
        assert_eq!(
            fmt(src),
            "x {\n  local { path = \"t\" }\n  rule = [\n    {\n      action { type = \"Delete\" }\n      condition { age = 3 }\n    },\n  ]\n}\n"
        );
    }

    #[test]
    fn blank_lines_collapse_and_never_touch_a_brace() {
        let src = "\n\nestate x\n\n\n\nparams {\n\n  a = 1\n\n}\n\n\n";
        assert_eq!(fmt(src), "estate x\n\nparams {\n  a = 1\n}\n");
    }

    #[test]
    fn hcl_bodies_and_strings_are_verbatim() {
        let src = "hcl trust \"r\" {\n    resource \"a\" \"b\" {\n\tx = 1 }\n}\ns = \"\"\"\n  keep   this\n\"\"\"\n";
        assert_eq!(fmt(src), src);
    }

    #[test]
    fn a_run_of_exports_aligns_its_equals_and_a_description_follows_the_value() {
        let src = "export \"org_id\" = customer_organization_id\nexport  \"infra_folder\"   =  \"${{google_folder.infra.name}}\"  description \"The folder\"\nexport \"regions\" = [\n\"a\"\n]\n";
        assert_eq!(
            fmt(src),
            "export \"org_id\"       = customer_organization_id\nexport \"infra_folder\" = \"${{google_folder.infra.name}}\" description \"The folder\"\nexport \"regions\" = [\n  \"a\",\n]\n"
        );
    }

    #[test]
    fn a_run_of_exports_with_attach_points_aligns_like_any_other() {
        let src = "export \"a\" = \"1\" attach [\"google_x\"] description \"d\"\nexport \"long_name\" = [\"x\"] description \"d\" attach [ \"google_y\" , \"google_z\" ]\nexport \"b\" = [\"y\"]\n";
        assert_eq!(
            fmt(src),
            "export \"a\"         = \"1\" attach [\"google_x\"] description \"d\"\nexport \"long_name\" = [\"x\"] description \"d\" attach [\"google_y\", \"google_z\"]\nexport \"b\"         = [\"y\"]\n"
        );
    }

    #[test]
    fn an_all_export_aligns_in_a_run() {
        assert_eq!(
            fmt("export \"folders\" = all  google_folder description \"d\"\nexport \"a\" = \"1\"\n"),
            "export \"folders\" = all google_folder description \"d\"\nexport \"a\"       = \"1\"\n"
        );
    }

    #[test]
    fn an_interface_block_indents_its_exports_and_aligns_them() {
        let src = "interface \"team-a\" {\nexport \"a\" = \"1\"\n    export \"long_name\"=2 description \"d\"\n}\n";
        assert_eq!(
            fmt(src),
            "interface \"team-a\" {\n  export \"a\"         = \"1\"\n  export \"long_name\" = 2 description \"d\"\n}\n"
        );
    }

    #[test]
    fn a_use_interface_line_keeps_its_list_inline_and_its_gate() {
        let src = "interface \"team-a\" {\n    use  interface \"network\"   when want\nuse interface [ \"dns\" , \"logs\" ]\n  export \"a\" = \"1\"\n}\n";
        assert_eq!(
            fmt(src),
            "interface \"team-a\" {\n  use interface \"network\" when want\n  use interface [\"dns\", \"logs\"]\n  export \"a\" = \"1\"\n}\n"
        );
    }

    #[test]
    fn a_file_the_parser_refuses_is_not_formatted() {
        assert!(format("a = \"open\n").is_err());
    }

    #[test]
    fn formatting_is_idempotent_and_preserves_meaning_on_the_showcase() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/smoke/yaml/showcase.satz")).unwrap();
        let once = fmt(&src);
        assert_eq!(fmt(&once), once, "not idempotent");
        assert_eq!(canonical(&parse(&once).unwrap()), canonical(&parse(&src).unwrap()), "meaning changed");
    }
}
