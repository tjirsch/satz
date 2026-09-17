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
//! fonts are embedded too (Libertinus Serif for text, DejaVu Sans Mono for code and
//! the status glyphs a compliance table is full of), because a PDF that renders
//! differently on the auditor's machine is not evidence of anything.
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
/// report needs — full width, so a wide table wraps inside the page instead of
/// running off it.
const PREAMBLE: &str = r#"#set page(paper: "a4", margin: (x: 1.8cm, y: 2cm), numbering: "1")
#set text(font: ("Libertinus Serif", "DejaVu Sans Mono"), size: 9.5pt)
#set par(justify: false, leading: 0.6em)
#show raw: set text(font: "DejaVu Sans Mono", size: 8.5pt)
#show heading: set block(above: 1.2em, below: 0.7em)
#set table(inset: 5pt, stroke: 0.4pt + luma(60%))
#show table.cell.where(y: 0): strong
"#;

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
        for (font, info) in typst_kit::fonts::embedded() {
            book.push(info);
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

/// The markdown satz writes, as Typst markup. Everything the reports use — headings,
/// emphasis, inline code, links, lists, tables, fenced code, rules, block quotes —
/// and the two HTML tags the compliance report puts inside a cell (`<br>`, `<small>`).
pub(crate) fn markup(markdown: &str) -> String {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = String::new();
    // a table is buffered: Typst takes the cells as arguments, not as rows
    let mut table: Option<(usize, Vec<String>)> = None;
    let mut cell = String::new();
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
            Event::Start(Tag::Table(alignments)) => table = Some((alignments.len(), Vec::new())),
            Event::End(TagEnd::Table) => {
                if let Some((columns, cells)) = table.take() {
                    let columns = columns.max(1);
                    // A compliance table is seven columns of prose. On a portrait page
                    // every cell wraps to four lines and the report doubles in length,
                    // so a wide one turns the page instead — the same decision a person
                    // makes in a word processor, and the reason the PDF is worth having.
                    let wide = columns >= 5;
                    if wide {
                        out.push_str("\n#page(flipped: true)[\n");
                    }
                    out.push_str(&format!("\n#table(\n  columns: ({}),\n", vec!["1fr"; columns].join(", ")));
                    for c in cells {
                        out.push_str(&format!("  [{}],\n", c.trim()));
                    }
                    out.push_str(")\n");
                    out.push_str(if wide { "]\n\n" } else { "\n" });
                }
            }
            Event::Start(Tag::TableCell) => {
                in_cell = true;
                cell.clear();
            }
            Event::End(TagEnd::TableCell) => {
                in_cell = false;
                if let Some((_, cells)) = table.as_mut() {
                    cells.push(std::mem::take(&mut cell));
                }
            }
            Event::Start(Tag::TableHead | Tag::TableRow) | Event::End(TagEnd::TableHead | TagEnd::TableRow) => {}
            // ---- text -----------------------------------------------------------
            Event::Text(t) => {
                let text = if in_code { t.to_string() } else { escape(&t) };
                push(&mut out, &mut cell, in_cell, &text);
            }
            Event::Code(t) => {
                // a raw span: backticks, with any backtick inside neutralised
                push(&mut out, &mut cell, in_cell, &format!("`{}`", t.replace('`', "'")));
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
                } else if !formatting.iter().any(|f| tag.starts_with(f)) {
                    push(&mut out, &mut cell, in_cell, &escape(&h));
                }
            }
            Event::SoftBreak => push(&mut out, &mut cell, in_cell, " "),
            Event::HardBreak => push(&mut out, &mut cell, in_cell, " \\\n"),
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
            Event::InlineMath(t) | Event::DisplayMath(t) => push(&mut out, &mut cell, in_cell, &escape(&t)),
        }
    }
    out
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
    /// rather than as rows — so this is the shape that has to be right.
    #[test]
    fn a_table_becomes_a_typst_table_with_one_cell_per_argument() {
        let t = markup("| control | verdict |\n|---|---|\n| 1.4 | satisfied |\n");
        assert!(t.contains("#table(\n  columns: (1fr, 1fr),"), "{t}");
        assert!(t.contains("[control],"), "{t}");
        assert!(t.contains("[satisfied],"), "{t}");
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

    /// Seven columns of prose do not fit a portrait page: every cell wraps to four
    /// lines and the report doubles in length, so a wide table turns the page.
    #[test]
    fn a_wide_table_turns_the_page_and_a_narrow_one_does_not() {
        let wide = markup("| a | b | c | d | e |\n|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 |\n");
        assert!(wide.contains("#page(flipped: true)["), "{wide}");
        let narrow = markup("| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(!narrow.contains("flipped"), "{narrow}");
    }

    /// The whole point: bytes out, with no tool on PATH.
    #[test]
    fn a_report_renders_to_a_pdf_without_anything_installed() {
        let md = "# satz evidence report\n\nCIS GCP 4.0, verified 2026-09-15.\n\n\
                  | control | verdict | witness |\n|---|---|---|\n\
                  | 1.4 | ✓ satisfied | `google_org_policy_policy.sa_key` |\n\
                  | 2.13 | ○ unmet | — |\n\n\
                  - a list item with a `#` and an @ in it\n";
        let bytes = render(md).expect("renders");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF: {:?}", &bytes[..8.min(bytes.len())]);
        assert!(bytes.len() > 2_000, "suspiciously small: {} bytes", bytes.len());
        // deterministic: the same report twice is the same bytes, so a diff of two
        // runs is a diff of the estate
        assert_eq!(bytes, render(md).expect("renders again"));
    }
}
