//! `satz lsp`: the language server behind an editor's Satz support.
//!
//! What an editor gets is what satz knows, from satz's own front end — no second
//! parser, no second schema. Diagnostics come from `satz::parse` on every change
//! and from the whole fragment pipeline (`compile_estate`, the ⊕ fold, the
//! suppressions) on every open and save, the same errors `transpile --check`
//! prints, at the file and line they name. Completion and hover read the
//! provider schema the estate's `config.toml` points at; go-to-definition follows
//! a `use` path the way the compiler resolves it and a param reference to its
//! `params {}` line; formatting is `satz fmt`.
//!
//! A pack has no estate of its own to compile: its pipeline diagnostics come
//! from the estates beside it that `use` it, attributed back to the pack's file.
//!
//! stdout is the protocol. Nothing in this module prints; the registry's loading
//! progress goes to stderr, where Zed's log shows it.

use crate::schema::{BlockSchema, ResourceRegistry};
use crate::{parse_tool_config, resolved_config, EstateResolver, ToolConfig};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, Formatting, GotoDefinition, HoverRequest, Request as _};
use lsp_types::*;
use satz_core::pipeline::{self, PipelineError};
use satz_core::satz::{self, lex_spanned, Entry, File, Tok, Token};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions { include_text: Some(false) })),
            ..Default::default()
        })),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec!["=".into(), " ".into()]),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    // The two-step handshake, so the result carries serverInfo beside the
    // capabilities (the one-call `initialize` wraps its argument as the
    // capabilities alone).
    let (init_id, _init_params) = connection.initialize_start()?;
    let init = serde_json::json!({
        "capabilities": capabilities,
        "serverInfo": { "name": "satz", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(init_id, init)?;
    let mut server = Server::default();
    server.main_loop(&connection)?;
    // The writer thread ends when the last sender is gone: drop the connection
    // before joining, or `exit` never returns.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One `config.toml` and what it points at: the resolved directories and, once
/// something asked for it, the provider schema.
struct Estate {
    dir: PathBuf,
    config: ToolConfig,
    registry: Option<Arc<ResourceRegistry>>,
}

#[derive(Default)]
struct Server {
    /// Open buffers, by path: the text the editor has, saved or not.
    docs: HashMap<PathBuf, String>,
    /// Estates by config directory, loaded on first use.
    estates: HashMap<PathBuf, Estate>,
    /// Files that currently carry pipeline diagnostics, so they can be cleared.
    published: BTreeSet<PathBuf>,
}

impl Server {
    fn main_loop(&mut self, conn: &Connection) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        for msg in &conn.receiver {
            match msg {
                Message::Request(req) => {
                    if conn.handle_shutdown(&req)? {
                        return Ok(());
                    }
                    let resp = self.handle_request(req);
                    conn.sender.send(Message::Response(resp))?;
                }
                Message::Notification(n) => self.handle_notification(conn, n)?,
                Message::Response(_) => {}
            }
        }
        Ok(())
    }

    fn handle_request(&mut self, req: Request) -> Response {
        let id = req.id.clone();
        match req.method.as_str() {
            Completion::METHOD => match serde_json::from_value::<CompletionParams>(req.params) {
                Ok(p) => ok(id, self.completion(&p)),
                Err(e) => invalid(id, e),
            },
            HoverRequest::METHOD => match serde_json::from_value::<HoverParams>(req.params) {
                Ok(p) => ok(id, self.hover(&p)),
                Err(e) => invalid(id, e),
            },
            GotoDefinition::METHOD => match serde_json::from_value::<GotoDefinitionParams>(req.params) {
                Ok(p) => ok(id, self.definition(&p)),
                Err(e) => invalid(id, e),
            },
            Formatting::METHOD => match serde_json::from_value::<DocumentFormattingParams>(req.params) {
                Ok(p) => ok(id, self.formatting(&p)),
                Err(e) => invalid(id, e),
            },
            _ => Response::new_err(id, ErrorCode::MethodNotFound as i32, format!("unsupported: {}", req.method)),
        }
    }

    fn handle_notification(&mut self, conn: &Connection, n: Notification) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        match n.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let p: DidOpenTextDocumentParams = serde_json::from_value(n.params)?;
                if let Some(path) = to_path(&p.text_document.uri) {
                    self.docs.insert(path.clone(), p.text_document.text);
                    self.publish_all(conn, &path, true)?;
                }
            }
            DidChangeTextDocument::METHOD => {
                let p: DidChangeTextDocumentParams = serde_json::from_value(n.params)?;
                if let (Some(path), Some(change)) = (to_path(&p.text_document.uri), p.content_changes.into_iter().last()) {
                    self.docs.insert(path.clone(), change.text);
                    self.publish_all(conn, &path, false)?;
                }
            }
            DidSaveTextDocument::METHOD => {
                let p: DidSaveTextDocumentParams = serde_json::from_value(n.params)?;
                if let Some(path) = to_path(&p.text_document.uri) {
                    self.publish_all(conn, &path, true)?;
                }
            }
            DidCloseTextDocument::METHOD => {
                let p: DidCloseTextDocumentParams = serde_json::from_value(n.params)?;
                if let Some(path) = to_path(&p.text_document.uri) {
                    self.docs.remove(&path);
                    publish(conn, &path, vec![])?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

    /// Parse diagnostics for the file on every change; the pipeline's on open
    /// and save — a whole-estate compile is not a per-keystroke thing.
    fn publish_all(&mut self, conn: &Connection, path: &Path, pipeline: bool) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        let text = self.docs.get(path).cloned().unwrap_or_default();
        let mut by_file: BTreeMap<PathBuf, Vec<Diagnostic>> = BTreeMap::new();
        by_file.entry(path.to_path_buf()).or_default();
        match satz::parse(&text) {
            Err(e) => by_file.get_mut(path).unwrap().push(line_diagnostic(&text, e.line, e.msg, DiagnosticSeverity::ERROR)),
            Ok(file) if pipeline => {
                for (f, d) in self.pipeline_diagnostics(path, &text, &file) {
                    by_file.entry(f).or_default().push(d);
                }
            }
            Ok(_) => {}
        }
        if pipeline {
            // A file that carried pipeline diagnostics last time and has none now
            // gets an empty set, or the editor keeps showing stale ones.
            let stale: Vec<PathBuf> = self.published.iter().filter(|p| !by_file.contains_key(*p)).cloned().collect();
            for p in stale {
                by_file.insert(p, vec![]);
            }
            self.published = by_file.iter().filter(|(_, d)| !d.is_empty()).map(|(p, _)| p.clone()).collect();
        }
        for (f, d) in by_file {
            publish(conn, &f, d)?;
        }
        Ok(())
    }

    /// The compile the CLI's `transpile --check` runs, over the editor's buffers,
    /// for the estate this file is — or, for a pack, for every estate beside it
    /// that uses it.
    fn pipeline_diagnostics(&mut self, path: &Path, text: &str, file: &File) -> Vec<(PathBuf, Diagnostic)> {
        let Some(dir) = find_config_dir(path) else { return vec![] };
        let Some(registry) = self.registry(&dir) else { return vec![] };
        let config = self.estates[&dir].config.clone();
        let roots: Vec<(PathBuf, String)> = if file.is_pack {
            self.estates_using(&dir, &config, path)
        } else {
            vec![(path.to_path_buf(), text.to_string())]
        };
        let mut out = Vec::new();
        for (root, src) in roots {
            out.extend(compile_for_diagnostics(&root, &src, &config, &registry, &self.docs));
        }
        // A pack's own line is what its author is looking at; the estate's other
        // files are not in front of them. Keep every file's diagnostics — the
        // editor shows them where they belong — but never lose the pack's.
        out
    }

    fn registry(&mut self, dir: &Path) -> Option<Arc<ResourceRegistry>> {
        if !self.estates.contains_key(dir) {
            let config = parse_tool_config(&dir.join("config.toml")).ok()?;
            let config = resolved_config(&config, dir);
            self.estates.insert(dir.to_path_buf(), Estate { dir: dir.to_path_buf(), config, registry: None });
        }
        let estate = self.estates.get_mut(dir)?;
        if estate.registry.is_none() {
            let reg = ResourceRegistry::load_all(&estate.config.schema_dir).ok()?;
            if reg.resources.is_empty() {
                // No schema: the pipeline would name every type unknown. Parse
                // diagnostics still work; `satz update-schema` is the fix.
                return None;
            }
            estate.registry = Some(Arc::new(reg));
        }
        let _ = &estate.dir;
        estate.registry.clone()
    }

    /// Estates in the estate directory whose source names this pack's file.
    fn estates_using(&self, dir: &Path, config: &ToolConfig, pack: &Path) -> Vec<(PathBuf, String)> {
        let needle = pack.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = Vec::new();
        let mut dirs = vec![PathBuf::from(&config.yaml_dir), dir.to_path_buf()];
        dirs.dedup();
        for d in dirs {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "satz")).collect();
            paths.sort();
            for p in paths {
                let src = match self.docs.get(&p) {
                    Some(t) => t.clone(),
                    None => match std::fs::read_to_string(&p) {
                        Ok(t) => t,
                        Err(_) => continue,
                    },
                };
                if !src.contains(&needle) {
                    continue;
                }
                if satz::parse(&src).is_ok_and(|f| !f.is_pack && f.estate.is_some()) {
                    out.push((p, src));
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Completion
    // -----------------------------------------------------------------------

    fn completion(&mut self, p: &CompletionParams) -> Option<CompletionResponse> {
        let path = to_path(&p.text_document_position.text_document.uri)?;
        let text = self.docs.get(&path)?.clone();
        let at = char_index(&text, p.text_document_position.position);
        let ctx = context_at(&text, at);
        let registry = find_config_dir(&path).and_then(|d| self.registry(&d));
        let mut items = Vec::new();
        match ctx.position {
            Slot::Value => {
                for (name, value) in self.params_of(&path, &text) {
                    items.push(item(&name, CompletionItemKind::VARIABLE, Some(format!("param = {}", value)), None, "1"));
                }
                items.push(item("true", CompletionItemKind::CONSTANT, None, None, "2"));
                items.push(item("false", CompletionItemKind::CONSTANT, None, None, "2"));
            }
            Slot::Key => {
                let typed = ctx.frames.iter().rev().find_map(|f| {
                    let reg = registry.as_ref()?;
                    reg.resources.get(&f.key).map(|(_, s)| (f, s))
                });
                if let Some((frame, schema)) = typed {
                    if let Some(block) = block_at(&schema.block, &ctx.frames, frame, registry.as_deref()) {
                        for (name, a) in &block.attributes {
                            if a.computed && !a.optional && !a.required {
                                continue; // read-only: nothing to write
                            }
                            let detail = format!("{}{}", type_text(&a.type_), if a.required { " · required" } else { "" });
                            items.push(item(name, CompletionItemKind::PROPERTY, Some(detail), a.description.clone(), "0"));
                        }
                        for (name, b) in &block.block_types {
                            let detail = match b.min_items {
                                Some(n) if n > 0 => "block · required".to_string(),
                                _ => "block".to_string(),
                            };
                            items.push(item(name, CompletionItemKind::STRUCT, Some(detail), b.block.description.clone(), "0"));
                        }
                        items.push(item("use", CompletionItemKind::KEYWORD, Some("include a pack here".into()), None, "3"));
                    }
                    // A resource may nest a resource (a project under a folder).
                    push_types(&mut items, registry.as_deref(), "4");
                } else {
                    match ctx.frames.last().map(|f| f.key.as_str()) {
                        None => {
                            for (k, d) in TOP_LEVEL {
                                items.push(item(k, CompletionItemKind::KEYWORD, Some(d.to_string()), None, "0"));
                            }
                            push_types(&mut items, registry.as_deref(), "1");
                        }
                        Some(k) => {
                            for (block, keys) in BODY_KEYS {
                                if *block == k {
                                    for (key, d) in *keys {
                                        items.push(item(key, CompletionItemKind::PROPERTY, Some(d.to_string()), None, "0"));
                                    }
                                }
                            }
                            if k != "params" && k != "terraform" && k != "providers" {
                                push_types(&mut items, registry.as_deref(), "1");
                            }
                        }
                    }
                }
            }
            Slot::Other => return None,
        }
        Some(CompletionResponse::Array(items))
    }

    /// The params in scope for this file: its own and its packs', with the
    /// resolved values.
    fn params_of(&self, path: &Path, text: &str) -> Vec<(String, String)> {
        let dir = find_config_dir(path);
        let config = dir.as_ref().and_then(|d| self.estates.get(d)).map(|e| e.config.clone());
        let loader = make_loader(path, config.as_ref(), &self.docs, None);
        match pipeline::estate_params(&path.to_string_lossy(), text, &loader) {
            Ok(env) => env.into_iter().map(|(k, v)| (k, yaml_text(&v))).collect(),
            Err(_) => satz::parse(text).map(|f| f.params.iter().map(|(n, _, _)| (n.clone(), "…".into())).collect()).unwrap_or_default(),
        }
    }

    // -----------------------------------------------------------------------
    // Hover
    // -----------------------------------------------------------------------

    fn hover(&mut self, p: &HoverParams) -> Option<Hover> {
        let path = to_path(&p.text_document_position_params.text_document.uri)?;
        let text = self.docs.get(&path)?.clone();
        let at = char_index(&text, p.text_document_position_params.position);
        let toks = lex_spanned(&text, true).ok()?;
        let (i, tok) = toks.iter().enumerate().find(|(_, t)| t.start <= at && at < t.end)?;
        let word = match &tok.tok {
            Tok::Ident(w) => w.clone(),
            Tok::Str(_) => {
                // `use "path"`: where it resolves to.
                if i > 0 && matches!(&toks[i - 1].tok, Tok::Ident(k) if k == "use") {
                    let raw: String = text.chars().skip(tok.start + 1).take(tok.end - tok.start - 2).collect();
                    let resolved = self.resolve_use(&path, &raw);
                    let body = match resolved {
                        Some(p) => format!("`use \"{}\"` → `{}`", raw, p.display()),
                        None => format!("`use \"{}\"` — not found beside the estate or in `include_dirs`", raw),
                    };
                    return Some(markdown(body, tok, &text));
                }
                return None;
            }
            _ => return None,
        };
        let ctx = context_at(&text, tok.start);
        let registry = find_config_dir(&path).and_then(|d| self.registry(&d));
        // A keyword.
        if let Some((_, d)) = TOP_LEVEL.iter().find(|(k, _)| *k == word) {
            if ctx.frames.is_empty() || word == "use" {
                return Some(markdown(format!("**{}** — {}", word, d), tok, &text));
            }
        }
        // A resource type.
        if let Some(reg) = registry.as_ref() {
            if let Some((provider, schema)) = reg.resources.get(&word) {
                let b = &schema.block;
                let mut body = format!(
                    "**{}** — resource type (provider `{}`): {} attributes, {} nested blocks",
                    word,
                    provider,
                    b.attributes.len(),
                    b.block_types.len()
                );
                if let Some(d) = &b.description {
                    body.push_str("\n\n");
                    body.push_str(d);
                }
                return Some(markdown(body, tok, &text));
            }
        }
        match ctx.position {
            Slot::Key => {
                // An attribute or nested block of the enclosing type.
                let reg = registry.as_ref()?;
                let (frame, schema) = ctx.frames.iter().rev().find_map(|f| reg.resources.get(&f.key).map(|(_, s)| (f, s)))?;
                let block = block_at(&schema.block, &ctx.frames, frame, Some(reg))?;
                if let Some(a) = block.attributes.get(&word) {
                    let flags = [(a.required, "required"), (a.optional, "optional"), (a.computed, "computed")]
                        .iter()
                        .filter(|(on, _)| *on)
                        .map(|(_, n)| *n)
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let mut body = format!("**{}** — {} · {}", word, type_text(&a.type_), flags);
                    if let Some(d) = &a.description {
                        body.push_str("\n\n");
                        body.push_str(d);
                    }
                    return Some(markdown(body, tok, &text));
                }
                if let Some(b) = block.block_types.get(&word) {
                    let mut body = format!("**{}** — nested block of `{}`", word, frame.key);
                    if let Some(d) = &b.block.description {
                        body.push_str("\n\n");
                        body.push_str(d);
                    }
                    return Some(markdown(body, tok, &text));
                }
                None
            }
            Slot::Value => {
                if word == "true" || word == "false" {
                    return None;
                }
                let value = self.params_of(&path, &text).into_iter().find(|(n, _)| *n == word)?.1;
                Some(markdown(format!("param **{}** = {}", word, value), tok, &text))
            }
            Slot::Other => None,
        }
    }

    // -----------------------------------------------------------------------
    // Definition
    // -----------------------------------------------------------------------

    fn definition(&mut self, p: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let path = to_path(&p.text_document_position_params.text_document.uri)?;
        let text = self.docs.get(&path)?.clone();
        let at = char_index(&text, p.text_document_position_params.position);
        let toks = lex_spanned(&text, true).ok()?;
        let (i, tok) = toks.iter().enumerate().find(|(_, t)| t.start <= at && at < t.end)?;
        match &tok.tok {
            Tok::Str(_) if i > 0 && matches!(&toks[i - 1].tok, Tok::Ident(k) if k == "use") => {
                let raw: String = text.chars().skip(tok.start + 1).take(tok.end - tok.start - 2).collect();
                let target = self.resolve_use(&path, &raw)?;
                Some(GotoDefinitionResponse::Scalar(Location { uri: to_uri(&target)?, range: Range::default() }))
            }
            Tok::Ident(name) => {
                let ctx = context_at(&text, tok.start);
                if ctx.position != Slot::Value {
                    return None;
                }
                let (file, line) = self.find_param(&path, &text, name, 0)?;
                let uri = to_uri(&file)?;
                let src = self.docs.get(&file).cloned().or_else(|| std::fs::read_to_string(&file).ok())?;
                let range = line_range(&src, line);
                Some(GotoDefinitionResponse::Scalar(Location { uri, range }))
            }
            _ => None,
        }
    }

    /// Where a param is declared: this file, else the packs it uses, in order.
    fn find_param(&self, path: &Path, text: &str, name: &str, depth: usize) -> Option<(PathBuf, usize)> {
        if depth > 32 {
            return None;
        }
        let file = satz::parse(text).ok()?;
        if let Some((_, _, line)) = file.params.iter().find(|(n, _, _)| n == name) {
            return Some((path.to_path_buf(), *line));
        }
        for u in uses_of(&file) {
            let Some(target) = self.resolve_use(path, &u) else { continue };
            let src = match self.docs.get(&target) {
                Some(t) => t.clone(),
                None => std::fs::read_to_string(&target).ok()?,
            };
            if let Some(found) = self.find_param(&target, &src, name, depth + 1) {
                return Some(found);
            }
        }
        None
    }

    /// The compiler's rule: beside the estate file first, then each include dir.
    fn resolve_use(&self, from: &Path, use_path: &str) -> Option<PathBuf> {
        let dir = find_config_dir(from);
        let config = dir.as_ref().and_then(|d| self.estates.get(d)).map(|e| e.config.clone());
        let mut candidates = vec![from.parent()?.join(use_path)];
        if let Some(c) = &config {
            candidates.extend(c.include_dirs.iter().map(|d| Path::new(d).join(use_path)));
        }
        candidates.into_iter().find(|c| c.exists())
    }

    // -----------------------------------------------------------------------
    // Formatting
    // -----------------------------------------------------------------------

    fn formatting(&mut self, p: &DocumentFormattingParams) -> Option<Vec<TextEdit>> {
        let path = to_path(&p.text_document.uri)?;
        let text = self.docs.get(&path)?.clone();
        let out = satz_core::fmt::format(&text).ok()?;
        if out == text {
            return Some(vec![]);
        }
        Some(vec![TextEdit { range: Range { start: Position::new(0, 0), end: end_position(&text) }, new_text: out }])
    }
}

// ---------------------------------------------------------------------------
// The compile, over buffers, to diagnostics
// ---------------------------------------------------------------------------

type Loader = Box<dyn Fn(&str) -> Result<String, String>>;

/// The loader `pipeline_b_generate` uses — beside the estate first, then the
/// include dirs — reading the editor's buffer where one is open, and recording
/// where each `use` path resolved so an error can name the file.
fn make_loader(
    estate: &Path,
    config: Option<&ToolConfig>,
    docs: &HashMap<PathBuf, String>,
    resolved: Option<Rc<RefCell<HashMap<String, PathBuf>>>>,
) -> Loader {
    let base = estate.parent().unwrap_or(Path::new(".")).to_path_buf();
    let include_dirs: Vec<PathBuf> = config.map(|c| c.include_dirs.iter().map(PathBuf::from).collect()).unwrap_or_default();
    let docs = docs.clone();
    Box::new(move |p: &str| {
        let mut candidates = vec![base.join(p)];
        candidates.extend(include_dirs.iter().map(|d| d.join(p)));
        for c in candidates {
            if let Some(text) = docs.get(&c) {
                if let Some(r) = &resolved {
                    r.borrow_mut().insert(p.to_string(), c.clone());
                }
                return Ok(text.clone());
            }
            if c.exists() {
                if let Some(r) = &resolved {
                    r.borrow_mut().insert(p.to_string(), c.clone());
                }
                return std::fs::read_to_string(&c).map_err(|e| e.to_string());
            }
        }
        Err(format!("use \"{}\": file not found", p))
    })
}

fn compile_for_diagnostics(
    root: &Path,
    src: &str,
    config: &ToolConfig,
    registry: &ResourceRegistry,
    docs: &HashMap<PathBuf, String>,
) -> Vec<(PathBuf, Diagnostic)> {
    let resolved = Rc::new(RefCell::new(HashMap::new()));
    let loader = make_loader(root, Some(config), docs, Some(resolved.clone()));
    let resolver = EstateResolver { registry };
    let label = root.to_string_lossy().into_owned();
    let locate = |file: &str| -> PathBuf {
        if file == label {
            return root.to_path_buf();
        }
        resolved.borrow().get(file).cloned().unwrap_or_else(|| root.to_path_buf())
    };
    let text_of = |p: &Path| -> String { docs.get(p).cloned().or_else(|| std::fs::read_to_string(p).ok()).unwrap_or_default() };
    let mut out = Vec::new();
    let mut push_err = |e: PipelineError| {
        let p = locate(&e.file);
        let t = text_of(&p);
        out.push((p, line_diagnostic(&t, e.line, e.msg, DiagnosticSeverity::ERROR)));
    };
    let fe = match pipeline::compile_estate(&label, src, &resolver, &loader) {
        Ok(fe) => fe,
        Err(e) => {
            push_err(e);
            return out;
        }
    };
    // Everything `transpile --check` checks after the front end, as findings: at the
    // file and line each names, or on the estate's first line when it names none.
    let tail = crate::compile_tail(&fe, &resolver, registry, config, &config.validation_level, root, src);
    for finding in tail.findings {
        let severity = match finding.severity {
            crate::findings::Severity::Error => DiagnosticSeverity::ERROR,
            crate::findings::Severity::Warning => DiagnosticSeverity::WARNING,
            crate::findings::Severity::Note => DiagnosticSeverity::INFORMATION,
        };
        let (p, line) = match (&finding.file, finding.line) {
            (Some(file), Some(line)) => (locate(file), line as usize),
            _ => (root.to_path_buf(), 1),
        };
        let t = text_of(&p);
        let message = match &finding.group {
            Some(g) => format!("{}\n{}", g, finding.message),
            None => finding.message.clone(),
        };
        out.push((p, line_diagnostic(&t, line, message, severity)));
    }
    out
}

// ---------------------------------------------------------------------------
// Where the cursor is: the enclosing blocks and the slot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Frame {
    /// The key before the opener: a resource type, an entry name, a block.
    key: String,
    /// `key NAME {`: the second label.
    name: Option<String>,
    /// The opener is `[`.
    list: bool,
    /// An object `{` directly inside a list: no key of its own.
    object: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Slot {
    /// The start of an entry: a key, a block name, a statement.
    Key,
    /// After `=`: a value.
    Value,
    /// Inside a string, after a header, mid-statement — nothing to offer.
    Other,
}

struct Context {
    frames: Vec<Frame>,
    position: Slot,
}

/// Read the tokens before `at` (a char index) and say where that is.
fn context_at(text: &str, at: usize) -> Context {
    let before: String = text.chars().take(at).collect();
    let toks = match lex_spanned(&before, true) {
        Ok(t) => t,
        Err(_) => return Context { frames: vec![], position: Slot::Other },
    };
    let mut frames: Vec<Frame> = Vec::new();
    // The identifiers/strings since the last line end or opener/closer: what
    // will name the next opener.
    let mut pending: Vec<String> = Vec::new();
    let mut after_eq = false;
    for t in &toks {
        match &t.tok {
            Tok::Newline => {
                pending.clear();
                after_eq = false;
            }
            Tok::Comment(_) => {}
            Tok::Ident(w) => {
                if !after_eq {
                    pending.push(w.clone());
                }
            }
            Tok::Str(_) => {
                if !after_eq {
                    pending.push(text.chars().skip(t.start).take(t.end - t.start).collect());
                }
            }
            Tok::Eq => after_eq = true,
            Tok::LBrack => {
                frames.push(Frame { key: pending.first().cloned().unwrap_or_default(), name: None, list: true, object: false });
                pending.clear();
                after_eq = false;
            }
            Tok::LBrace => {
                let in_list = frames.last().is_some_and(|f| f.list);
                frames.push(Frame {
                    key: pending.first().cloned().unwrap_or_default(),
                    name: pending.get(1).cloned(),
                    list: false,
                    object: in_list && pending.is_empty(),
                });
                pending.clear();
                after_eq = false;
            }
            Tok::RBrace | Tok::RBrack => {
                frames.pop();
                pending.clear();
                after_eq = false;
            }
            Tok::Comma => {
                pending.clear();
                after_eq = false;
            }
            Tok::Num(_) | Tok::Hcl(..) => {}
        }
    }
    // What the last significant token was decides the slot.
    let last = toks.iter().rev().find(|t| !matches!(t.tok, Tok::Comment(_)));
    let position = match last.map(|t| &t.tok) {
        None | Some(Tok::Newline) | Some(Tok::LBrace) | Some(Tok::RBrace) | Some(Tok::Comma) => {
            if frames.last().is_some_and(|f| f.list) {
                Slot::Value
            } else {
                Slot::Key
            }
        }
        Some(Tok::Eq) | Some(Tok::LBrack) => Slot::Value,
        Some(Tok::Ident(_)) => {
            // A word being typed: a key if it starts the line, a value after `=`.
            if after_eq {
                Slot::Value
            } else if pending.len() <= 1 {
                if frames.last().is_some_and(|f| f.list) {
                    Slot::Value
                } else {
                    Slot::Key
                }
            } else {
                Slot::Other
            }
        }
        _ => Slot::Other,
    };
    Context { frames, position }
}

/// Walk from the type's frame to the innermost one through the schema's nested
/// blocks: the entry name of a map form is skipped, a list and the object in it
/// are one level, an unknown key ends the walk with nothing.
fn block_at<'a>(root: &'a BlockSchema, frames: &[Frame], type_frame: &Frame, registry: Option<&ResourceRegistry>) -> Option<&'a BlockSchema> {
    let start = frames.iter().position(|f| std::ptr::eq(f, type_frame))?;
    let mut inner = frames[start + 1..].iter().peekable();
    if type_frame.name.is_none() {
        // `google_folder { infra { … } }`: the first inner frame is the name.
        if let Some(f) = inner.peek() {
            if !f.list && !f.object && registry.is_none_or(|r| !r.resources.contains_key(&f.key)) {
                inner.next();
            }
        }
    }
    let mut block = root;
    for f in inner {
        if f.object {
            continue;
        }
        block = &block.block_types.get(&f.key)?.block;
    }
    Some(block)
}

const TOP_LEVEL: &[(&str, &str)] = &[
    ("estate", "the header of an estate: `estate NAME`"),
    ("pack", "the header of a pack: `pack NAME version \"x.y\"`"),
    ("params", "the values this file binds; a pack's are defaults, an estate's are the answers"),
    ("use", "include a pack: `use \"path\" [as TYPE] [when PARAM]`"),
    ("claim", "what a control this file satisfies: `claim FRAMEWORK VERSION CONTROL implements|contributes|deviates { … }`"),
    ("question", "what a customer decides, so a param can be filled: `question [oneof] PARAM { … }`"),
    ("action", "a deployment step with no provider resource: `action \"name\" { reason run args }`"),
    ("offers", "the map only — one pack the library offers: `offers \"presets/…\" { when phase block … }`"),
    ("suppress", "decline what a pack provides: `suppress TYPE \"name\" [role \"…\"]`"),
    ("hcl", "raw HCL passthrough, verbatim and opaque to claims: `hcl [trust \"…\"] { … }`"),
    ("terraform", "the backend block, emitted as providers.tf"),
    ("providers", "the provider blocks, emitted as providers.tf"),
];

const BODY_KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "question",
        &[
            ("prompt", "what to ask, one sentence"),
            ("why", "what getting it wrong costs"),
            ("reversal", "edit | state_surgery | recreate"),
            ("blast", "none | low | high"),
            ("recommend", "the default answer"),
            ("ask_when", "only when this param is truthy"),
            ("required", "true: no default is possible"),
            ("option", "`option PARAM { label = \"…\" why = \"…\" }` in a `question oneof`"),
        ],
    ),
    (
        "action",
        &[
            ("reason", "why this cannot be a resource (mandatory)"),
            ("run", "the script, relative to this file"),
            ("args", "arguments; `{param}` interpolates"),
            ("execute_args", "appended with `run-actions --execute`"),
            ("phase", "\"before-apply\" | \"after-apply\""),
        ],
    ),
    (
        "offers",
        &[
            ("when", "the param the pack's line is gated on"),
            ("phase", "opens a group of lines: what has to be finished before they go in"),
            ("block", "the block the line is written inside, e.g. \"google_folder.infra_folder\""),
            ("after_scaffold", "true: the line goes after the scaffold"),
            ("by_hand", "the line is written by hand; why"),
            ("requires", "packs it needs that its params do not show"),
            ("excludes", "packs it never goes in beside"),
        ],
    ),
];

fn push_types(items: &mut Vec<CompletionItem>, registry: Option<&ResourceRegistry>, sort: &str) {
    if let Some(reg) = registry {
        let mut names: Vec<&String> = reg.resources.keys().collect();
        names.sort();
        for name in names {
            let (provider, s) = &reg.resources[name];
            items.push(item(
                name,
                CompletionItemKind::CLASS,
                Some(format!("resource type · {}", provider)),
                s.block.description.clone(),
                sort,
            ));
        }
    }
}

fn item(label: &str, kind: CompletionItemKind, detail: Option<String>, doc: Option<String>, sort: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail,
        documentation: doc.map(|d| Documentation::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value: d })),
        sort_text: Some(format!("{}{}", sort, label)),
        ..Default::default()
    }
}

fn type_text(t: &Option<serde_json::Value>) -> String {
    match t {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a.iter().map(type_text_v).collect::<Vec<_>>().join(" of "),
        Some(other) => other.to_string(),
        None => "block".into(),
    }
}

fn type_text_v(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(type_text_v).collect::<Vec<_>>().join(" of "),
        serde_json::Value::Object(o) => format!("object{{{}}}", o.keys().cloned().collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}

fn yaml_text(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => format!("\"{}\"", s),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

fn uses_of(file: &File) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(entries: &[Entry], out: &mut Vec<String>) {
        for e in entries {
            match e {
                Entry::Use { path, .. } => out.push(path.clone()),
                Entry::Map { body, .. } => walk(body, out),
                Entry::Attr { .. } => {}
            }
        }
    }
    walk(&file.items, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Files, positions, messages
// ---------------------------------------------------------------------------

fn find_config_dir(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?.to_path_buf();
    loop {
        if dir.join("config.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn to_path(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

fn to_uri(path: &Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    Uri::from_str(url.as_str()).ok()
}

/// An LSP position (line, UTF-16 unit) to a char index into the text.
fn char_index(text: &str, pos: Position) -> usize {
    let mut idx = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n == pos.line as usize {
            let mut units = 0u32;
            for (k, ch) in line.chars().enumerate() {
                if units >= pos.character {
                    return idx + k;
                }
                units += ch.len_utf16() as u32;
            }
            return idx + line.chars().count();
        }
        idx += line.chars().count() + 1;
    }
    text.chars().count()
}

fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

fn end_position(text: &str) -> Position {
    let lines: Vec<&str> = text.split('\n').collect();
    Position::new((lines.len() - 1) as u32, utf16_len(lines[lines.len() - 1]))
}

/// The whole of a 1-based line.
fn line_range(text: &str, line: usize) -> Range {
    let l = line.saturating_sub(1) as u32;
    let len = text.split('\n').nth(l as usize).map(utf16_len).unwrap_or(0);
    Range { start: Position::new(l, 0), end: Position::new(l, len) }
}

fn line_diagnostic(text: &str, line: usize, msg: String, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic {
        range: line_range(text, line),
        severity: Some(severity),
        source: Some("satz".into()),
        message: msg,
        ..Default::default()
    }
}

fn markdown(body: String, tok: &Token, text: &str) -> Hover {
    let start = char_to_position(text, tok.start);
    let end = char_to_position(text, tok.end);
    Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: body }),
        range: Some(Range { start, end }),
    }
}

fn char_to_position(text: &str, idx: usize) -> Position {
    let mut line = 0u32;
    let mut units = 0u32;
    for (k, ch) in text.chars().enumerate() {
        if k == idx {
            break;
        }
        if ch == '\n' {
            line += 1;
            units = 0;
        } else {
            units += ch.len_utf16() as u32;
        }
    }
    Position::new(line, units)
}

fn publish(conn: &Connection, path: &Path, diagnostics: Vec<Diagnostic>) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let Some(uri) = to_uri(path) else { return Ok(()) };
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    conn.sender.send(Message::Notification(Notification::new(PublishDiagnostics::METHOD.to_string(), params)))?;
    Ok(())
}

fn ok<T: serde::Serialize>(id: RequestId, value: T) -> Response {
    Response::new_ok(id, value)
}

fn invalid(id: RequestId, e: serde_json::Error) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stdout is the protocol: nothing in this module may print to it.
    #[test]
    fn this_module_never_writes_to_stdout() {
        let src = include_str!("lsp.rs");
        let body = src.split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains("println!") && !body.contains("print!("), "lsp.rs prints to stdout");
    }

    #[test]
    fn the_cursor_context_names_the_enclosing_type_and_the_slot() {
        let text = "google_folder {\n  infra {\n    display_name = \"x\"\n    google_project {\n      infra {\n        ";
        let ctx = context_at(text, text.chars().count());
        assert_eq!(ctx.position, Slot::Key);
        let keys: Vec<&str> = ctx.frames.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["google_folder", "infra", "google_project", "infra"]);
    }

    #[test]
    fn after_an_equals_sign_the_slot_is_a_value_and_inside_a_list_too() {
        let text = "params {\n  a = ";
        assert_eq!(context_at(text, text.chars().count()).position, Slot::Value);
        let text = "x {\n  member = [\n    ";
        let ctx = context_at(text, text.chars().count());
        assert_eq!(ctx.position, Slot::Value);
        assert!(ctx.frames.last().unwrap().list);
    }

    #[test]
    fn a_list_of_objects_is_one_schema_level() {
        let text = "google_storage_bucket {\n  b {\n    lifecycle_rule = [\n      {\n        condition {\n          ";
        let ctx = context_at(text, text.chars().count());
        let keys: Vec<(String, bool, bool)> = ctx.frames.iter().map(|f| (f.key.clone(), f.list, f.object)).collect();
        assert_eq!(
            keys,
            [
                ("google_storage_bucket".into(), false, false),
                ("b".into(), false, false),
                ("lifecycle_rule".into(), true, false),
                ("".into(), false, true),
                ("condition".into(), false, false),
            ]
        );
    }

    #[test]
    fn positions_round_trip_through_utf16() {
        let text = "a = \"é\"\nb = 1\n";
        let idx = char_index(text, Position::new(1, 0));
        assert_eq!(text.chars().nth(idx), Some('b'));
        assert_eq!(char_to_position(text, idx), Position::new(1, 0));
        assert_eq!(end_position(text), Position::new(2, 0));
    }
}
