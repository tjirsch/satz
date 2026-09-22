//! PDF, in the binary.
//!
//! `--format pdf` used to shell out to `pandoc`, which is a Haskell program satz
//! cannot contain — and pandoc alone was not even enough: its PDF output needs a
//! PDF engine behind it, LaTeX by default. So the one format a customer is most
//! likely to be handed was the one format that depended on two tools the operator
//! had to install, and it failed on a machine that had neither.
//!
//! Typst is a typesetting engine written in Rust, so satz compiles the report
//! itself: markdown in, PDF bytes out, no process, no PATH, nothing to install. The
//! fonts are embedded too — `assets/fonts/`, Libertinus Serif for text and DejaVu
//! Sans Mono for code and the status glyphs a compliance table is full of — because
//! a PDF that renders differently on the auditor's machine is not evidence of
//! anything.
//!
//! What is NOT here is a markdown renderer of our own: `markup` turns the markdown
//! satz already produces into Typst markup, and Typst does the typesetting. The
//! conversion is the part that can be wrong, so it is a pure function with tests;
//! the rest is a `World` implementation with one source file in it.

use std::path::Path;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

/// The document preamble: A4, a readable size, and the table shape a compliance
/// report needs. The column widths come from the content (`column_widths`) and the
/// orientation from the whole document (`markup`), so neither is set here.
const PREAMBLE: &str = r#"#set page(paper: "a4", margin: (x: 1.8cm, y: 2cm), numbering: "1")
#set text(font: ("Libertinus Serif", "DejaVu Sans Mono"), size: 9.5pt)
#set par(justify: false, leading: 0.6em)
#show raw: set text(font: "DejaVu Sans Mono", size: 8.5pt)
#show heading: set block(above: 1.2em, below: 0.7em)
#set table(inset: 5pt, stroke: 0.4pt + luma(60%))
#show table.cell.where(y: 0): strong
"#;

/// The faces satz carries: the two families the preamble names, each in the four
/// styles markup can ask for. A heading, a strong span and a table header are bold,
/// an emphasised span is italic, and both at once is bold italic; a code span takes
/// the weight and slant of whatever it sits in, so `` `code` `` inside a heading is
/// bold monospace. A style whose face is absent is typeset in the nearest one that
/// is present, with no diagnostic — so the square is complete rather than trimmed to
/// what today's reports happen to reach.
///
/// The files are the `typst-assets` crate's own, committed under `assets/fonts/`
/// with the licence texts the OFL and the Bitstream licence ask a redistributor to
/// carry. What is left out is New Computer Modern, text and math: 7.3 MB of the
/// 9.7 MB that crate bundles, and nothing a compliance report sets.
const FACES: [(&str, &[u8]); 8] = [
    ("LibertinusSerif-Regular.otf", include_bytes!("../assets/fonts/LibertinusSerif-Regular.otf")),
    ("LibertinusSerif-Bold.otf", include_bytes!("../assets/fonts/LibertinusSerif-Bold.otf")),
    ("LibertinusSerif-Italic.otf", include_bytes!("../assets/fonts/LibertinusSerif-Italic.otf")),
    ("LibertinusSerif-BoldItalic.otf", include_bytes!("../assets/fonts/LibertinusSerif-BoldItalic.otf")),
    ("DejaVuSansMono.ttf", include_bytes!("../assets/fonts/DejaVuSansMono.ttf")),
    ("DejaVuSansMono-Bold.ttf", include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf")),
    ("DejaVuSansMono-Oblique.ttf", include_bytes!("../assets/fonts/DejaVuSansMono-Oblique.ttf")),
    ("DejaVuSansMono-BoldOblique.ttf", include_bytes!("../assets/fonts/DejaVuSansMono-BoldOblique.ttf")),
];

/// One Typst source, compiled with the fonts satz carries.
struct Report {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    source: Source,
}

impl Report {
    fn new(typst_markup: String) -> Self {
        let mut book = FontBook::new();
        let mut fonts = Vec::new();
        for (name, data) in FACES {
            // One face per file, so index 0. A file that is not a font it can read is
            // a broken build, not a report in a substituted face.
            let font = Font::new(Bytes::new(data), 0)
                .unwrap_or_else(|| panic!("the embedded font {} did not parse", name));
            book.push(font.info().clone());
            fonts.push(font);
        }
        let vpath = VirtualPath::new("report.typ").expect("`report.typ` is a valid virtual path");
        let id = RootedPath::new(VirtualRoot::Project, vpath).intern();
        Report {
            library: LazyHash::new(<Library as LibraryExt>::default()),
            book: LazyHash::new(book),
            fonts,
            source: Source::new(id, typst_markup),
        }
    }
}

impl World for Report {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.source.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().get_without_slash().into()))
        }
    }
    /// There is one file and it is the report: a document that reaches for another
    /// is a document satz did not write.
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(id.vpath().get_without_slash().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    /// No date: a report that renders differently tomorrow is not the same report.
    /// What the run happened at is in the report's own text, where satz put it.
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Render markdown as a PDF, and write it. The whole external dependency this
/// replaces is `pandoc` plus whatever PDF engine it was configured with.
pub(crate) fn write(markdown: &str, path: &Path, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = render(markdown)?;
    if crate::out::to_stdout(path, &bytes)? {
        eprintln!("wrote stdout — {}", what);
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            crate::fsx::create_dir_all(dir)?;
        }
    }
    crate::fsx::write(path, &bytes)?;
    eprintln!("wrote {} — {}", path.display(), what);
    Ok(())
}

/// The bytes, so a caller that is not writing a file can have them too.
pub(crate) fn render(markdown: &str) -> Result<Vec<u8>, String> {
    let world = Report::new(format!("{}\n{}", PREAMBLE, markup(markdown)));
    let document = typst::compile::<PagedDocument>(&world)
        .output
        .map_err(|errors| format!("typesetting the report failed: {}", first_error(&errors)))?;
    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|errors| format!("writing the PDF failed: {}", first_error(&errors)))
}

fn first_error(errors: &[typst::diag::SourceDiagnostic]) -> String {
    errors.first().map(|e| e.message.to_string()).unwrap_or_else(|| "no reason given".to_string())
}

/// Typst reads `#`, `*`, `_`, `` ` ``, `$`, `<`, `@` and the brackets as syntax, so
/// text that carries them has to say so — otherwise a control id like `1.4` is
/// fine but a member string like `serviceAccount:x@y` becomes a label reference and
/// the document stops compiling.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '#' | '*' | '_' | '`' | '$' | '\\' | '<' | '>' | '@' | '[' | ']' | '~' | '"' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A column whose longest cell is at most this many characters is sized to its
/// content: the status glyph a compliance row starts with, a verdict, a short id. It
/// is small enough that `auto` cannot overflow the page, and wide enough to hold a
/// header word like `verdict` over a column of single glyphs.
const AUTO_COLUMN: usize = 12;

/// What a prose column's weight is allowed to be, in characters. The floor keeps a
/// column of short answers from being squeezed to nothing beside a column of
/// sentences; the ceiling keeps one long cell from taking the page, so the widest
/// prose column is at most four and a half times the narrowest.
const WEIGHT_FLOOR: f64 = 10.0;
const WEIGHT_CEILING: f64 = 45.0;

/// A table this wide turns the document. A4 portrait leaves 17.4cm of text, which is
/// about 108 characters at 9.5pt, and each column spends 0.35cm of it on its insets:
/// five columns leave some 19 characters each, which is a word and a half per line.
/// Landscape leaves 26.1cm, about 163 characters, and the same five columns get 30.
const WIDE_TABLE: usize = 5;

/// The `columns:` tuple for a table, from the plain text of its cells in row-major
/// order, header row first. A column no wider than [`AUTO_COLUMN`] is sized to its
/// content; the rest share what is left in proportion to the mean length of their
/// filled cells, clamped and rounded to a half.
fn column_widths(columns: usize, cells: &[String]) -> String {
    let columns = columns.max(1);
    let mut by_column: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for (i, cell) in cells.iter().enumerate() {
        // a cell holds line breaks (`<br>` in the compliance report), and what a
        // column has to hold is its longest LINE, not the sum of them
        let widest = cell.lines().map(|line| line.trim().chars().count()).max().unwrap_or(0);
        by_column[i % columns].push(widest);
    }
    let weights: Vec<Option<f64>> = by_column
        .iter()
        .map(|lengths| {
            if lengths.iter().copied().max().unwrap_or(0) <= AUTO_COLUMN {
                return None;
            }
            // empty cells are what a continuation row leaves behind — counting them
            // would say a column is half as wide as its filled cells need
            let filled: Vec<usize> = lengths.iter().copied().filter(|n| *n > 0).collect();
            let mean = filled.iter().sum::<usize>() as f64 / filled.len() as f64;
            Some(mean.clamp(WEIGHT_FLOOR, WEIGHT_CEILING))
        })
        .collect();
    let narrowest = weights.iter().flatten().copied().fold(f64::INFINITY, f64::min);
    weights
        .iter()
        .map(|weight| match weight {
            None => "auto".to_string(),
            Some(w) => {
                let share = (w / narrowest * 2.0).round() / 2.0;
                if share.fract() == 0.0 {
                    format!("{}fr", share as i64)
                } else {
                    format!("{:.1}fr", share)
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The markdown satz writes, as Typst markup. Everything the reports use — headings,
/// emphasis, inline code, links, lists, tables, fenced code, rules, block quotes —
/// and the two HTML tags the compliance report puts inside a cell (`<br>`, `<small>`).
pub(crate) fn markup(markdown: &str) -> String {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = String::new();
    // a table is buffered: Typst takes the cells as arguments, not as rows. Each cell
    // is kept twice — as markup, and as the plain text the column widths are measured
    // from, where a link is its label and `*bold*` is the word inside it.
    let mut table: Option<(usize, Vec<(String, String)>)> = None;
    let mut cell = String::new();
    let mut plain = String::new();
    // set by the widest table in the document: one orientation for all of it
    let mut landscape = false;
    let mut in_cell = false;
    let mut in_code = false;
    let mut list_markers: Vec<Option<u64>> = Vec::new();

    let push = |out: &mut String, cell: &mut String, in_cell: bool, s: &str| {
        if in_cell {
            cell.push_str(s);
        } else {
            out.push_str(s);
        }
    };
    // the text a reader sees in a cell, which is what its column has to be wide enough
    // for: markup the converter adds around it is not part of it
    let measure = |plain: &mut String, in_cell: bool, s: &str| {
        if in_cell {
            plain.push_str(s);
        }
    };

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let depth = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                out.push('\n');
                out.push_str(&"=".repeat(depth));
                out.push(' ');
            }
            Event::End(TagEnd::Heading(_)) => out.push_str("\n\n"),
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => push(&mut out, &mut cell, in_cell, "\n\n"),
            Event::Start(Tag::Strong) => push(&mut out, &mut cell, in_cell, "*"),
            Event::End(TagEnd::Strong) => push(&mut out, &mut cell, in_cell, "*"),
            Event::Start(Tag::Emphasis) => push(&mut out, &mut cell, in_cell, "_"),
            Event::End(TagEnd::Emphasis) => push(&mut out, &mut cell, in_cell, "_"),
            Event::Start(Tag::Strikethrough) => push(&mut out, &mut cell, in_cell, "#strike["),
            Event::End(TagEnd::Strikethrough) => push(&mut out, &mut cell, in_cell, "]"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                push(&mut out, &mut cell, in_cell, &format!("#link(\"{}\")[", dest_url.replace('"', "%22")));
            }
            Event::End(TagEnd::Link) => push(&mut out, &mut cell, in_cell, "]"),
            Event::Start(Tag::List(first)) => {
                list_markers.push(first);
                if list_markers.len() == 1 {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::List(_)) => {
                list_markers.pop();
                if list_markers.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Item) => {
                let indent = "  ".repeat(list_markers.len().saturating_sub(1));
                let marker = if list_markers.last().copied().flatten().is_some() { "+" } else { "-" };
                out.push_str(&format!("{}{} ", indent, marker));
            }
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::Start(Tag::BlockQuote(_)) => out.push_str("#quote(block: true)[\n"),
            Event::End(TagEnd::BlockQuote(_)) => out.push_str("]\n\n"),
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) if !l.is_empty() => l.to_string(),
                    _ => String::new(),
                };
                out.push_str(&format!("\n```{}\n", lang));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push_str("```\n\n");
            }
            // ---- tables: Typst takes the cells as arguments -------------------
            Event::Start(Tag::Table(alignments)) => {
                landscape |= alignments.len() >= WIDE_TABLE;
                table = Some((alignments.len(), Vec::new()));
            }
            Event::End(TagEnd::Table) => {
                if let Some((columns, cells)) = table.take() {
                    let columns = columns.max(1);
                    let measured: Vec<String> = cells.iter().map(|(_, plain)| plain.clone()).collect();
                    out.push_str(&format!("\n#table(\n  columns: ({}),\n", column_widths(columns, &measured)));
                    let mut rest = cells.iter().map(|(markup, _)| markup.trim());
                    // the first row is the header: Typst repeats it on every page the
                    // table spans, and the show rule in the preamble sets it bold
                    let header: Vec<&str> = rest.by_ref().take(columns).collect();
                    if !header.is_empty() {
                        out.push_str("  table.header(\n    repeat: true,\n");
                        for c in header {
                            out.push_str(&format!("    [{}],\n", c));
                        }
                        out.push_str("  ),\n");
                    }
                    for c in rest {
                        out.push_str(&format!("  [{}],\n", c));
                    }
                    out.push_str(")\n\n");
                }
            }
            Event::Start(Tag::TableCell) => {
                in_cell = true;
                cell.clear();
                plain.clear();
            }
            Event::End(TagEnd::TableCell) => {
                in_cell = false;
                if let Some((_, cells)) = table.as_mut() {
                    cells.push((std::mem::take(&mut cell), std::mem::take(&mut plain)));
                }
            }
            Event::Start(Tag::TableHead | Tag::TableRow) | Event::End(TagEnd::TableHead | TagEnd::TableRow) => {}
            // ---- text -----------------------------------------------------------
            Event::Text(t) => {
                let text = if in_code { t.to_string() } else { escape(&t) };
                push(&mut out, &mut cell, in_cell, &text);
                measure(&mut plain, in_cell, &t);
            }
            Event::Code(t) => {
                // a raw span: backticks, with any backtick inside neutralised
                push(&mut out, &mut cell, in_cell, &format!("`{}`", t.replace('`', "'")));
                measure(&mut plain, in_cell, &t);
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                // The compliance report writes `<br>` and `<small>` inside cells, and
                // those are formatting: the break becomes one, the rest goes. Anything
                // else is TEXT the author wrote — `<estate>` in an instruction, say —
                // and is kept, escaped, rather than silently eaten.
                let tag = h.trim().to_ascii_lowercase();
                let formatting = ["<br", "<small", "</small", "<sub", "</sub", "<sup", "</sup", "<b>", "</b>", "<i>", "</i>"];
                if tag.starts_with("<br") {
                    push(&mut out, &mut cell, in_cell, " \\\n");
                    measure(&mut plain, in_cell, "\n");
                } else if !formatting.iter().any(|f| tag.starts_with(f)) {
                    push(&mut out, &mut cell, in_cell, &escape(&h));
                    measure(&mut plain, in_cell, &h);
                }
            }
            Event::SoftBreak => {
                push(&mut out, &mut cell, in_cell, " ");
                measure(&mut plain, in_cell, " ");
            }
            Event::HardBreak => {
                push(&mut out, &mut cell, in_cell, " \\\n");
                measure(&mut plain, in_cell, "\n");
            }
            Event::Rule => out.push_str("\n#line(length: 100%, stroke: 0.4pt + luma(70%))\n\n"),
            Event::TaskListMarker(done) => {
                push(&mut out, &mut cell, in_cell, if done { "\\[x\\] " } else { "\\[ \\] " })
            }
            Event::FootnoteReference(_) | Event::Start(Tag::FootnoteDefinition(_)) | Event::End(TagEnd::FootnoteDefinition) => {}
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => {}
            Event::Start(Tag::MetadataBlock(_)) | Event::End(TagEnd::MetadataBlock(_)) => {}
            Event::Start(Tag::DefinitionList)
            | Event::End(TagEnd::DefinitionList)
            | Event::Start(Tag::DefinitionListTitle)
            | Event::End(TagEnd::DefinitionListTitle)
            | Event::Start(Tag::DefinitionListDefinition)
            | Event::End(TagEnd::DefinitionListDefinition) => {}
            Event::Start(Tag::Superscript | Tag::Subscript) | Event::End(TagEnd::Superscript | TagEnd::Subscript) => {}
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                push(&mut out, &mut cell, in_cell, &escape(&t));
                measure(&mut plain, in_cell, &t);
            }
        }
    }
    // One orientation for the whole document: a report that turns the page around
    // every table reads as two documents interleaved, and the headings between the
    // tables sat on the portrait ones.
    if landscape { format!("#set page(flipped: true)\n{}", out) } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_a_list_and_emphasis_become_typst_markup() {
        let t = markup("# Title\n\nSome **bold** and _thin_ text.\n\n- one\n- two\n");
        assert!(t.contains("= Title"), "{t}");
        assert!(t.contains("*bold*") && t.contains("_thin_"), "{t}");
        assert!(t.contains("- one\n- two\n"), "{t}");
    }

    /// The reports are mostly tables, and Typst takes their cells as arguments
    /// rather than as rows — so this is the shape that has to be right. The first
    /// row is the header: Typst repeats it on every page the table spans.
    #[test]
    fn a_table_becomes_a_typst_table_whose_first_row_is_a_repeating_header() {
        let t = markup("| control | verdict |\n|---|---|\n| 1.4 | satisfied |\n");
        assert!(t.contains("#table(\n  columns: (auto, auto),"), "{t}");
        assert!(t.contains("table.header(\n    repeat: true,\n    [control],\n    [verdict],\n  ),"), "{t}");
        assert!(t.contains("[satisfied],"), "{t}");
        // the header is in the header and nowhere else
        assert_eq!(t.matches("[control],").count(), 1, "{t}");
    }

    /// The complaint this answers: a compliance table's leading column holds status
    /// glyphs and got the same width as the prose beside it, so the prose wrapped for
    /// nothing. A column that cannot be wide is sized to its content; the rest share
    /// what is left in proportion to what they hold.
    #[test]
    fn a_glyph_column_is_sized_to_its_content_and_prose_columns_share_the_rest() {
        let cells: Vec<String> = vec![
            String::new(),
            "decision".into(),
            "why it is asked".into(),
            "✓".into(),
            "x".repeat(20),
            "x".repeat(40),
            "✓".into(),
            "x".repeat(20),
            "x".repeat(40),
        ];
        assert_eq!(column_widths(3, &cells), "auto, 1fr, 2fr");
    }

    /// One cell of an essay must not starve the columns beside it, and a column of
    /// short answers must not be squeezed to a letter a line: the weight is clamped
    /// at both ends, so the widest prose column is 4.5 times the narrowest.
    #[test]
    fn one_long_cell_cannot_take_the_page() {
        let cells: Vec<String> = vec!["name".into(), "note".into(), "x".repeat(15), "x".repeat(200)];
        // unclamped these mean 9.5 and 102 characters, which is eleven to one
        assert_eq!(column_widths(2, &cells), "1fr, 4.5fr");
    }

    /// A continuation row leaves its other cells empty — the compliance sheet writes
    /// one under every question — and an empty cell says nothing about how wide its
    /// column has to be.
    #[test]
    fn an_empty_cell_does_not_shrink_its_column() {
        let filled: Vec<String> = vec!["a".repeat(20), "b".repeat(40)];
        let with_a_continuation: Vec<String> =
            vec!["a".repeat(20), "b".repeat(40), String::new(), String::new()];
        assert_eq!(column_widths(2, &filled), column_widths(2, &with_a_continuation));
    }

    #[test]
    fn a_single_column_and_an_empty_table_are_sized_without_panicking() {
        assert_eq!(column_widths(1, &["ok".to_string()]), "auto");
        assert_eq!(column_widths(1, &["x".repeat(80)]), "1fr");
        assert_eq!(column_widths(3, &[]), "auto, auto, auto");
        assert_eq!(column_widths(0, &[]), "auto");
        let t = markup("| a |\n|---|\n");
        assert!(t.contains("#table(\n  columns: (auto),"), "{t}");
    }

    /// Typst syntax inside report text is text, not syntax: an estate is full of
    /// `serviceAccount:x@y`, `#` and `*`, and each of them would otherwise be a
    /// function call, a label reference or emphasis — and usually a compile error.
    #[test]
    fn typst_syntax_inside_the_text_is_escaped() {
        let t = markup("A member serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com and a #hash and 2 * 3 and a <tag>.\n");
        assert!(t.contains("serviceAccount:svc-iac-001\\@acme-infra-001.iam"), "{t}");
        assert!(t.contains("\\#hash"), "{t}");
        assert!(t.contains("2 \\* 3"), "{t}");
        // `<` opens a label in Typst, and an estate is full of angle brackets
        assert!(t.contains("\\<tag\\>"), "{t}");
    }

    #[test]
    fn a_code_fence_keeps_its_text_verbatim() {
        let t = markup("```bash\nsatz require cis-gcp-4.0 e.satz --format text --out /dev/stdout\n```\n");
        assert!(t.contains("```bash\nsatz require cis-gcp-4.0 e.satz --format text --out /dev/stdout\n```"), "{t}");
    }

    /// `<br>` is how the compliance report puts two lines in one cell.
    #[test]
    fn a_line_break_inside_a_cell_survives_and_other_html_does_not() {
        let t = markup("| witness |\n|---|\n| one<br>two <small>note</small> |\n");
        assert!(t.contains("one \\\ntwo"), "{t}");
        assert!(!t.contains("small"), "{t}");
    }

    /// Five columns of prose do not fit a portrait page: every cell wraps to a word
    /// and a half a line and the report doubles in length. The document turns once —
    /// per table it turned back for every heading between them.
    #[test]
    fn a_wide_table_turns_the_whole_document_once_and_a_narrow_one_leaves_it_upright() {
        let wide = markup(
            "# Decisions\n\n| a | b | c | d | e |\n|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 |\n\n\
             ## more\n\n| a | b | c | d | e |\n|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 |\n",
        );
        assert!(wide.starts_with("#set page(flipped: true)\n"), "{wide}");
        assert_eq!(wide.matches("flipped").count(), 1, "{wide}");
        let narrow = markup("| a | b | c | d |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n");
        assert!(!narrow.contains("flipped"), "{narrow}");
    }

    /// Every style the markup can ask for has a face of its own. Typst answers a
    /// request it has no face for with the nearest one it has and says nothing, so a
    /// dropped face is a report set in the wrong weight or upright where it should
    /// slant — this is what notices.
    #[test]
    fn each_family_carries_all_four_styles() {
        use typst::text::{FontStretch, FontStyle, FontVariant, FontWeight};

        let world = Report::new(String::new());
        for family in ["libertinus serif", "dejavu sans mono"] {
            let mut seen = Vec::new();
            for (style, weight) in [
                (FontStyle::Normal, FontWeight::REGULAR),
                (FontStyle::Normal, FontWeight::BOLD),
                (FontStyle::Italic, FontWeight::REGULAR),
                (FontStyle::Italic, FontWeight::BOLD),
            ] {
                let wanted = FontVariant::new(style, weight, FontStretch::NORMAL);
                let index = world
                    .book
                    .select(family, wanted)
                    .unwrap_or_else(|| panic!("{family} has no face at all"));
                let got = world.fonts[index].info().variant;
                assert_eq!(got.weight, weight, "{family} {style:?} {weight:?} fell back to {got:?}");
                assert_eq!(
                    got.style == FontStyle::Normal,
                    style == FontStyle::Normal,
                    "{family} {style:?} {weight:?} fell back to {got:?}"
                );
                assert!(!seen.contains(&index), "{family} {style:?} {weight:?} shares a face with another style");
                seen.push(index);
            }
        }
    }

    /// The text a compiled page carries, with the face each run was set in, so a
    /// header that is not repeated or not bold is caught where it happens — in the
    /// laid-out document, not in the markup.
    fn words(frame: &typst::layout::Frame, into: &mut Vec<(String, typst::text::FontWeight)>) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Text(text) => {
                    into.push((text.text.to_string(), text.font.info().variant.weight))
                }
                typst::layout::FrameItem::Group(group) => words(&group.frame, into),
                _ => {}
            }
        }
    }

    /// A decisions sheet is one table of five columns and dozens of rows. It is
    /// landscape from the first page to the last, and the column titles stand over
    /// the columns on every one of them — a table that breaks across pages and leaves
    /// its header on page one is a page of unlabelled cells.
    #[test]
    fn a_long_wide_table_is_landscape_throughout_and_repeats_its_header_in_bold() {
        let mut md = String::from(
            "# Decisions\n\n| | decision | your answer | how | changing it later |\n|---|---|---|---|---|\n",
        );
        for i in 0..80 {
            md.push_str(&format!(
                "| ✓ | the question number {i} this estate has to answer before it is applied | `answer-{i}` | \
                 chosen for this estate | the resource is destroyed and made again and the running \
                 organisation feels it |\n"
            ));
        }
        let world = Report::new(format!("{}\n{}", PREAMBLE, markup(&md)));
        let document = typst::compile::<PagedDocument>(&world).output.expect("the decisions sheet compiles");
        assert!(document.pages().len() > 1, "one page: the table has to break for this to prove anything");
        for (n, page) in document.pages().iter().enumerate() {
            let size = page.frame.size();
            assert!(size.x > size.y, "page {} is portrait: {:?}", n + 1, size);
            let mut seen = Vec::new();
            words(&page.frame, &mut seen);
            let header = seen
                .iter()
                .find(|(text, _)| text == "decision")
                .unwrap_or_else(|| panic!("page {} carries no header row", n + 1));
            assert_eq!(header.1, typst::text::FontWeight::BOLD, "the header on page {} is not bold", n + 1);
        }
    }

    /// The whole point: bytes out, with no tool on PATH.
    #[test]
    fn a_report_renders_to_a_pdf_without_anything_installed() {
        let md = "# satz evidence report\n\nCIS GCP 4.0, verified 2026-09-15.\n\n\
                  | control | verdict | witness | framework | scope |\n|---|---|---|---|---|\n\
                  | 1.4 | ✓ satisfied | `google_org_policy_policy.sa_key` | CIS GCP 4.0 | the organisation |\n\
                  | 2.13 | ○ unmet | — | CIS GCP 4.0 | every project |\n\n\
                  - a list item with a `#` and an @ in it\n";
        let bytes = render(md).expect("renders");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF: {:?}", &bytes[..8.min(bytes.len())]);
        assert!(bytes.len() > 2_000, "suspiciously small: {} bytes", bytes.len());
        // deterministic: the same report twice is the same bytes, so a diff of two
        // runs is a diff of the estate
        assert_eq!(bytes, render(md).expect("renders again"));
    }
}
