//! Which packs an estate uses, and whether that agrees with the pack graph.
//!
//! One module answers it for every reader. `satz packs` and `satz_packs` report it,
//! `satz add-pack` and `satz remove-pack` change it, the interview and `merge-presets`
//! write pack lines through it, and the compile's pack findings are its findings — so
//! what an editor marks, what `transpile --check` prints and what `add-pack` refuses
//! are the same sentences.
//!
//! The inputs are the estate's text and the graph that ships with its presets
//! (`presets/pack-graph.json`, ADR 0031). A gate's value is read from the estate's own
//! `params {}` and, where the estate binds nothing, from the default in the library file
//! that declares it — so the report works on an estate that does not compile, which is
//! when it is needed most.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rmcp::schemars;
use satz_core::pack_graph::{EdgeKind, Node, PackGraph, Role, Source};
use satz_core::satz::{File, Value};

use crate::findings::{Finding, Kind, Severity};
use crate::template::Place;
use crate::ToolConfig;

type BoxErr = Box<dyn std::error::Error>;

// ---------------------------------------------------------------------------
// The estate's `use` lines
// ---------------------------------------------------------------------------

/// One `use` line of the estate, active or commented out.
#[derive(Debug, Clone)]
pub(crate) struct UseLine {
    /// 0-based
    pub index: usize,
    /// the path as the line writes it
    pub written: String,
    pub commented: bool,
    pub alias: Option<String>,
    pub when: Option<String>,
    /// the blocks the line sits in, outermost first: `["google_folder", "infra_folder"]`
    pub blocks: Vec<String>,
    pub indent: String,
    /// a trailing `// …` comment, kept when the line is rewritten
    pub trailing: String,
}

/// The estate's text as the pack logic reads it: every `use` line, and the three places
/// a top-level line can be anchored to when no other pack line is there.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub uses: Vec<UseLine>,
    /// the `presets/estate-core.satz` line, active or commented
    pub core: Option<usize>,
    /// the closing brace of the top-level `params { … }`
    pub params_end: Option<usize>,
    /// the `estate …` line
    pub header: Option<usize>,
}

/// `use "<path>" [as <key>] [when <param>] [// comment]` — `None` for anything else.
fn parse_use(body: &str) -> Option<(String, Option<String>, Option<String>, String)> {
    let rest = body.strip_prefix("use")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    let written = rest[..end].to_string();
    let mut tail = &rest[end + 1..];
    let mut trailing = String::new();
    if let Some(i) = tail.find("//") {
        trailing = tail[i..].trim_end().to_string();
        tail = &tail[..i];
    }
    let words: Vec<&str> = tail.split_whitespace().collect();
    let (mut alias, mut when) = (None, None);
    let mut i = 0;
    while i < words.len() {
        match (words[i], words.get(i + 1)) {
            ("as", Some(k)) => alias = Some(k.to_string()),
            ("when", Some(p)) => when = Some(p.to_string()),
            _ => return None,
        }
        i += 2;
    }
    Some((written, alias, when, trailing))
}

/// Every `use` line of `src`, with the blocks it sits in. Brace counting steps over
/// strings and `//` comments; a commented line is read, never counted.
pub(crate) fn scan(src: &str) -> Scan {
    let mut out = Scan::default();
    let mut stack: Vec<String> = Vec::new();
    let mut in_params = false;
    for (index, raw) in src.lines().enumerate() {
        let t = raw.trim_start();
        let indent = raw[..raw.len() - t.len()].to_string();
        if let Some(body) = t.strip_prefix("//") {
            if let Some((written, alias, when, trailing)) = parse_use(body.trim_start()) {
                if written == CORE && stack.is_empty() {
                    out.core = Some(index);
                }
                out.uses.push(UseLine { index, written, commented: true, alias, when, blocks: stack.clone(), indent, trailing });
            }
            continue;
        }
        if stack.is_empty() && out.header.is_none() && t.starts_with("estate ") {
            out.header = Some(index);
        }
        if let Some((written, alias, when, trailing)) = parse_use(t) {
            if written == CORE && stack.is_empty() {
                out.core = Some(index);
            }
            out.uses.push(UseLine { index, written, commented: false, alias, when, blocks: stack.clone(), indent, trailing });
        }
        // the braces of the code on this line
        let mut seg = String::new();
        let mut chars = t.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    while let Some(s) = chars.next() {
                        match s {
                            '\\' => {
                                chars.next();
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                    seg.push('"');
                }
                '/' if chars.peek() == Some(&'/') => break,
                '{' => {
                    let name = seg.trim().trim_matches('"').to_string();
                    if stack.is_empty() && name == "params" {
                        in_params = true;
                    }
                    stack.push(name);
                    seg.clear();
                }
                '}' => {
                    stack.pop();
                    if stack.is_empty() && in_params {
                        in_params = false;
                        out.params_end = Some(index);
                    }
                    seg.clear();
                }
                _ => seg.push(c),
            }
        }
    }
    out
}

const CORE: &str = "presets/estate-core.satz";

/// The graph node a written path names, and whether it names the node's `.local` fork.
fn node_of<'g>(graph: &'g PackGraph, written: &str) -> Option<(&'g Node, bool)> {
    if let Some(n) = graph.nodes.iter().find(|n| n.path == written) {
        return Some((n, false));
    }
    let base = format!("{}.satz", written.strip_suffix(".local.satz")?);
    graph.nodes.iter().find(|n| n.path == base).map(|n| (n, true))
}

fn segments(block: &str) -> Vec<String> {
    block.split('.').map(str::to_string).collect()
}

/// Whether a line sits where the graph places its node. A line written by hand is the
/// estate's to place.
fn placed_right(n: &Node, l: &UseLine) -> bool {
    if n.by_hand.is_some() {
        return true;
    }
    match crate::template::place(n) {
        Place::Block(b) => l.blocks == segments(b),
        Place::Menu | Place::AfterScaffold => l.blocks.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// The library files the logic reads
// ---------------------------------------------------------------------------

/// The library files the pack logic reads beside the graph: every file that declares a
/// gate (for its default) and every pack that reads another pack's param (for the params
/// it reads it through).
pub(crate) struct Library {
    files: BTreeMap<String, File>,
}

impl Library {
    pub(crate) fn load(graph: &PackGraph, presets_dir: &Path) -> Result<Library, String> {
        let mut needed: BTreeSet<&str> = graph.nodes.iter().filter_map(|n| n.gate_declared_in.as_deref()).collect();
        needed.extend(graph.edges.iter().filter(|e| e.kind == EdgeKind::Data).map(|e| e.from.as_str()));
        let mut files = BTreeMap::new();
        for p in needed {
            let path = presets_dir.join(p.strip_prefix("presets/").unwrap_or(p));
            let text = crate::fsx::read_to_string(&path).map_err(|e| format!("{}: {} — the pack graph names it", path.display(), e))?;
            let file = satz_core::satz::parse(&text).map_err(|e| format!("{}:{}: {}", path.display(), e.line, e.msg))?;
            files.insert(p.to_string(), file);
        }
        Ok(Library { files })
    }
}

// ---------------------------------------------------------------------------
// The estate against the graph
// ---------------------------------------------------------------------------

/// One requirement of a pack: any one of `any_of` meets it.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Requirement {
    /// `requires` (the map declares it), `data` (the pack reads a param the other declares)
    /// or `gate` (the other declares the param the pack is gated on)
    pub kind: &'static str,
    pub any_of: Vec<String>,
    /// the params that make a `data` or `gate` requirement
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// one of `any_of` is on — or, for `data`, the estate binds what the pack would
    /// otherwise read from it
    pub met: bool,
}

pub(crate) struct View<'a> {
    graph: &'a PackGraph,
    lib: &'a Library,
    scan: Scan,
    /// the estate's own `params {}`
    own: BTreeMap<String, Value>,
}

impl<'a> View<'a> {
    pub(crate) fn new(graph: &'a PackGraph, lib: &'a Library, src: &str) -> Result<View<'a>, String> {
        let own = satz_core::satz::parse(src)
            .map_err(|e| format!("line {}: {}", e.line, e.msg))?
            .params
            .into_iter()
            .map(|(n, v, _)| (n, v))
            .collect();
        Ok(View { graph, lib, scan: scan(src), own })
    }

    fn node(&self, path: &str) -> Option<&'a Node> {
        self.graph.nodes.iter().find(|n| n.path == path)
    }

    /// The node's lines: the first active one, the first commented one.
    fn lines_of(&self, path: &str) -> (Option<&UseLine>, Option<&UseLine>) {
        let named = |l: &&UseLine| node_of(self.graph, &l.written).is_some_and(|(n, _)| n.path == path);
        let active = self.scan.uses.iter().filter(named).find(|l| !l.commented);
        let commented = self.scan.uses.iter().filter(named).find(|l| l.commented);
        (active, commented)
    }

    /// The value a param has in this estate, where it is a boolean: the estate's own
    /// binding, else the default of the library file that declares it — while that file
    /// is used. `None` when neither says.
    pub(crate) fn value(&self, param: &str) -> Option<bool> {
        self.value_at(param, 0)
    }

    fn value_at(&self, param: &str, depth: u8) -> Option<bool> {
        if depth > 32 {
            return None;
        }
        if let Some(v) = self.own.get(param) {
            return self.eval(v, depth);
        }
        let decl = self.graph.nodes.iter().find(|n| n.gate.as_deref() == Some(param))?.gate_declared_in.as_deref()?;
        if !self.deploys_at(decl, depth + 1) {
            return None;
        }
        let (_, v, _) = self.lib.files.get(decl)?.params.iter().find(|(n, _, _)| n == param)?;
        self.eval(v, depth + 1)
    }

    fn eval(&self, v: &Value, depth: u8) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(*b),
            Value::Ref(r) => self.value_at(r, depth + 1),
            _ => None,
        }
    }

    /// Whether the pack emits: its line is active, and its `when` — if it has one — holds.
    pub(crate) fn deploys(&self, path: &str) -> bool {
        self.deploys_at(path, 0)
    }

    fn deploys_at(&self, path: &str, depth: u8) -> bool {
        let Some(l) = self.lines_of(path).0 else { return false };
        match &l.when {
            None => true,
            Some(w) => self.value_at(w, depth + 1) == Some(true),
        }
    }

    /// The gate of `n` can be answered: the file declaring it is used.
    fn gate_exists(&self, n: &Node) -> bool {
        n.gate_declared_in.as_deref().is_some_and(|d| self.deploys(d))
    }

    /// What `n` needs, grouped: the map's `requires` are one requirement, each pack that
    /// declares its gate one, and the params it reads one per set of params.
    fn groups(&self, n: &Node) -> Vec<(&'static str, Vec<String>, Vec<String>)> {
        let from = |k: EdgeKind| self.graph.edges.iter().filter(move |e| e.from == n.path && e.kind == k);
        let mut out = Vec::new();
        let req: Vec<String> = from(EdgeKind::Requires).map(|e| e.to.clone()).collect();
        if !req.is_empty() {
            out.push(("requires", req, Vec::new()));
        }
        for e in from(EdgeKind::Gate) {
            out.push(("gate", vec![e.to.clone()], e.params.clone()));
        }
        // a gate the map declares: no pack edge says so, and the map has to be used for
        // `when <gate>` to compile at all
        if let (Some(d), Some(g)) = (&n.gate_declared_in, &n.gate) {
            if d != &n.path && !from(EdgeKind::Gate).any(|e| &e.to == d) {
                out.push(("gate", vec![d.clone()], vec![g.clone()]));
            }
        }
        let mut data: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
        for e in from(EdgeKind::Data) {
            let mut p = e.params.clone();
            p.sort();
            data.entry(p).or_default().push(e.to.clone());
        }
        for (params, tos) in data {
            out.push(("data", tos, params));
        }
        out
    }

    /// A `data` requirement the estate meets itself: it binds the params the pack reads,
    /// or every param of the pack whose default reads them — the runner grant's
    /// `ci_runner_service_account` bound to a runner that lives in another estate.
    fn data_bound(&self, n: &Node, params: &[String]) -> bool {
        if params.iter().all(|p| self.own.contains_key(p)) {
            return true;
        }
        let Some(f) = self.lib.files.get(&n.path) else { return false };
        let reads = |r: &crate::doc_packs::Refs| params.iter().any(|p| r.contains_key(p));
        let mut direct = crate::doc_packs::Refs::new();
        f.items.iter().for_each(|e| crate::doc_packs::refs_in_entry(e, &mut direct));
        for a in &f.actions {
            a.args.iter().chain(&a.execute_args).for_each(|p| crate::doc_packs::refs_in_str(p, a.line, &mut direct));
        }
        if reads(&direct) {
            return false;
        }
        let readers: Vec<&str> = f
            .params
            .iter()
            .filter(|(_, v, line)| {
                let mut r = crate::doc_packs::Refs::new();
                crate::doc_packs::refs_in_value(v, *line, &mut r);
                reads(&r)
            })
            .map(|(name, _, _)| name.as_str())
            .collect();
        !readers.is_empty() && readers.iter().all(|r| self.own.contains_key(*r))
    }

    fn requirements(&self, n: &Node, on: &dyn Fn(&str) -> bool) -> Vec<Requirement> {
        self.groups(n)
            .into_iter()
            .map(|(kind, any_of, params)| {
                let met = any_of.iter().any(|p| on(p)) || (kind == "data" && self.data_bound(n, &params));
                Requirement { kind, any_of, params, met }
            })
            .collect()
    }

    /// The sentence for a requirement `n` has and that is off — the same in the compile's
    /// finding, `packs`, and `add-pack`'s refusal.
    fn requirement_text(&self, n: &Node, r: &Requirement) -> String {
        let name = |p: &str| match self.node(p).and_then(|m| m.gate.clone()) {
            Some(g) => format!("`{}` (`{}`)", p, g),
            None => format!("`{}`", p),
        };
        let reads = if r.kind == "data" {
            format!(" (it reads {})", r.params.iter().map(|p| format!("`{}`", p)).collect::<Vec<_>>().join(", "))
        } else {
            String::new()
        };
        match r.any_of.as_slice() {
            [one] => format!("`{}` needs {}, which is off{}", n.path, name(one), reads),
            many => format!(
                "`{}` needs one of {}, and none is on{}",
                n.path,
                many.iter().map(|p| name(p)).collect::<Vec<_>>().join(", "),
                reads
            ),
        }
    }

    fn state(&self, n: &Node) -> (&'static str, Option<&UseLine>) {
        match self.lines_of(&n.path) {
            (Some(l), _) => {
                let fork = node_of(self.graph, &l.written).is_some_and(|(_, f)| f);
                let s = if n.gate.is_some() && l.when.is_none() {
                    "ungated"
                } else if !placed_right(n, l) {
                    "misplaced"
                } else if fork {
                    "forked"
                } else {
                    "active"
                };
                (s, Some(l))
            }
            (None, Some(l)) => ("commented", Some(l)),
            (None, None) => ("absent", None),
        }
    }

    /// A pack an `excludes` neighbour on the same gate stands in for — the S1 model's
    /// two-file spelling for `s1-security-groups` — is used when that neighbour is.
    fn stood_in_for(&self, n: &Node, test: &dyn Fn(&str) -> bool) -> bool {
        self.graph.excluded_by(&n.path).iter().any(|o| o.gate.is_some() && o.gate == n.gate && test(&o.path))
    }

    // ---- findings ------------------------------------------------------------

    /// Two packs on two gates that exclude one another, both on: always an error, found
    /// before the fold, which would name the files instead of the decision. Two packs on
    /// ONE gate that exclude one another are two spellings of the same resources — the S1
    /// model's one-file and two-file forms — and fold as one where they agree.
    fn exclusion_findings(&self, label: &str, src: &str) -> Vec<(String, Finding)> {
        let mut clashes: Vec<(&Node, &Node, bool)> = Vec::new();
        let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
        for e in self.graph.edges.iter().filter(|e| e.kind == EdgeKind::Excludes) {
            let (Some(a), Some(b)) = (self.node(&e.from), self.node(&e.to)) else { continue };
            if a.gate == b.gate || !seen.insert((a.path.as_str(), b.path.as_str())) || !self.deploys(&a.path) || !self.deploys(&b.path) {
                continue;
            }
            // a dry-run twin declares its edge to the enforcing pack
            let dry_run = e.source == Source::Declared;
            clashes.push((a, b, dry_run));
        }
        let dry = clashes.iter().filter(|c| c.2).count();
        let other = clashes.len() - dry;
        let mut out = Vec::new();
        for (a, b, dry_run) in clashes {
            let f = if dry_run {
                let (dry_gate, enforcing) = (a.gate.as_deref().unwrap_or_default(), b.gate.as_deref().unwrap_or_default());
                Finding::new(
                    Severity::Error,
                    Kind::DryRunConflict,
                    format!(
                        "`{}` and `{}` are both true — a dry run REPLACES enforcement while it measures.\n     \
                         Switch one off: `{}` to size the control against this organisation first, `{}` to enforce it now.",
                        enforcing, dry_gate, dry_gate, enforcing
                    ),
                )
                .in_group(format!("{} control(s) asked to be measured and enforced at once:", dry))
                .maybe_at(label.to_string(), crate::findings::param_line(src, dry_gate))
            } else {
                Finding::new(
                    Severity::Error,
                    Kind::ExcludedPacks,
                    format!("`{}` and `{}` exclude one another and both are on — switch one off with `satz remove-pack`", a.path, b.path),
                )
                .in_group(format!("{} pair(s) of packs that exclude one another, both on:", other))
                .maybe_at(label.to_string(), self.lines_of(&a.path).0.map(|l| l.index as u32 + 1))
            };
            out.push((a.path.clone(), f));
        }
        out
    }

    /// The findings about each pack's line and what it needs, at the validation level.
    fn line_findings(&self, label: &str, level: &str) -> Vec<(String, Finding)> {
        let Some(sev) = crate::findings::at_level(level) else { return Vec::new() };
        let at = |l: Option<&UseLine>| l.map(|l| l.index as u32 + 1);
        let mut unadopted: Vec<(String, String, Option<u32>)> = Vec::new();
        let mut ungated: Vec<(String, String, Option<u32>)> = Vec::new();
        let mut needs: Vec<(String, String, Option<u32>)> = Vec::new();
        let on = |p: &str| self.deploys(p);
        for n in &self.graph.nodes {
            let (state, line) = self.state(n);
            // a gate answered yes with no line to switch on
            if let (Some(g), true) = (&n.gate, n.order.is_some() && n.by_hand.is_none()) {
                if self.value(g) == Some(true) && matches!(state, "commented" | "absent") && !self.stood_in_for(n, &|p| self.lines_of(p).0.is_some()) {
                    let msg = if state == "commented" {
                        format!("`{}` is true and `{}` is still commented out — uncomment it, or `satz add-pack` will", g, n.path)
                    } else {
                        format!("`{}` is true and this estate has no line for `{}` — `satz add-pack` writes it where the pack graph places it", g, n.path)
                    };
                    unadopted.push((n.path.clone(), msg, at(line)));
                }
            }
            if state == "ungated" && self.gate_exists(n) {
                let g = n.gate.as_deref().unwrap_or_default();
                ungated.push((
                    n.path.clone(),
                    format!(
                        "`{}` is used without `when {}`, so a no to `{}` does not switch it off — `satz merge-presets` gates it, or write `use \"{}\" when {}`",
                        n.path, g, g, n.path, g
                    ),
                    at(line),
                ));
            }
            if self.deploys(&n.path) {
                for r in self.requirements(n, &on).iter().filter(|r| !r.met) {
                    needs.push((n.path.clone(), format!("{} — `satz add-pack` it first", self.requirement_text(n, r)), at(line)));
                }
            }
        }
        let mut out = Vec::new();
        let mut group = |items: Vec<(String, String, Option<u32>)>, kind: Kind, header: String| {
            for (path, msg, line) in items {
                out.push((path, Finding::new(sev, kind, msg).in_group(&header).maybe_at(label.to_string(), line)));
            }
        };
        let n = unadopted.len();
        group(unadopted, Kind::UnadoptedPack, format!("{} pack(s) this estate asks for but does not use — the answer is bound and nothing emits it:", n));
        let n = ungated.len();
        group(ungated, Kind::UngatedPack, format!("{} pack line(s) without their gate — a no does not switch them off:", n));
        let n = needs.len();
        group(needs, Kind::PackRequirement, format!("{} pack(s) on while a pack they need is off:", n));
        out
    }

    // ---- the report ------------------------------------------------------------

    fn row(&self, n: &Node, findings: &[(String, Finding)]) -> PackRow {
        let on = |p: &str| self.deploys(p);
        let (state, line) = self.state(n);
        let default = match (&n.gate, &n.gate_declared_in) {
            (Some(g), Some(d)) => self
                .lib
                .files
                .get(d)
                .and_then(|f| f.params.iter().find(|(name, _, _)| name == g))
                .map(|(_, v, _)| crate::doc_packs::value_text(v)),
            _ => None,
        };
        let written = line.filter(|l| l.written != n.path).map(|l| l.written.clone());
        let gated_on = line.filter(|l| !l.commented).and_then(|l| l.when.clone()).filter(|w| Some(w) != n.gate.as_ref());
        PackRow {
            path: n.path.clone(),
            role: match n.role {
                Role::Core => "core",
                Role::Map => "map",
                Role::Pack => "pack",
            },
            gate: n.gate.clone(),
            gate_declared_in: n.gate_declared_in.clone(),
            answer: n.gate.as_ref().and_then(|g| self.own.get(g)).map(crate::doc_packs::value_text),
            default,
            value: n.gate.as_deref().and_then(|g| self.value(g)),
            line: state,
            at_line: line.map(|l| l.index + 1),
            written,
            gated_on,
            deploys: self.deploys(&n.path),
            requires: self.requirements(n, &on),
            required_by: self
                .graph
                .nodes
                .iter()
                .filter(|m| self.groups(m).iter().any(|(_, any, _)| any.contains(&n.path)))
                .map(|m| m.path.clone())
                .collect(),
            excludes: self.graph.excluded_by(&n.path).iter().map(|m| m.path.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
            by_hand: n.by_hand.clone(),
            findings: findings.iter().filter(|(p, _)| p == &n.path).map(|(_, f)| f.message.clone()).collect(),
        }
    }

    fn unmanaged(&self) -> Vec<Unmanaged> {
        self.scan
            .uses
            .iter()
            .filter(|l| !l.commented && node_of(self.graph, &l.written).is_none())
            .map(|l| Unmanaged { path: l.written.clone(), at_line: l.index + 1 })
            .collect()
    }
}

/// The compile's pack findings, in two halves: the exclusions, which stop the compile
/// before the fold, and the rest. A library file the graph names that does not read is
/// one warning, never a stopped compile.
pub(crate) fn compile_findings(graph: &PackGraph, presets_dir: &Path, label: &str, src: &str, level: &str) -> (Vec<Finding>, Vec<Finding>) {
    let lib = match Library::load(graph, presets_dir) {
        Ok(l) => l,
        Err(e) => {
            return (Vec::new(), vec![Finding::new(Severity::Warning, Kind::UnadoptedPack, format!("{} — no pack line is checked against the pack graph", e))]);
        }
    };
    let view = match View::new(graph, &lib, src) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), vec![Finding::new(Severity::Warning, Kind::UnadoptedPack, format!("{}: {}", label, e))]),
    };
    let strip = |v: Vec<(String, Finding)>| v.into_iter().map(|(_, f)| f).collect();
    (strip(view.exclusion_findings(label, src)), strip(view.line_findings(label, level)))
}

/// What the pack graph says about an estate whose front end failed: the requirements
/// that are off, which is usually why a param is unknown. Empty when it has nothing to add.
pub(crate) fn front_end_hints(graph: &PackGraph, presets_dir: &Path, label: &str, src: &str, level: &str) -> Vec<String> {
    let Ok(lib) = Library::load(graph, presets_dir) else { return Vec::new() };
    let Ok(view) = View::new(graph, &lib, src) else { return Vec::new() };
    view.line_findings(label, level)
        .into_iter()
        .chain(view.exclusion_findings(label, src))
        .filter(|(_, f)| matches!(f.kind, Kind::PackRequirement | Kind::ExcludedPacks | Kind::DryRunConflict))
        .map(|(_, f)| f.message)
        .collect()
}

// ---------------------------------------------------------------------------
// `satz packs`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PackRow {
    /// as a `use` line names it: `presets/…`
    pub path: String,
    /// `core`, `map` or `pack`
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_declared_in: Option<String>,
    /// the estate's own binding of the gate, as written
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// the gate's default in the file that declares it, as written
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// what the gate is in this estate: the answer, else the default while the declaring
    /// file is used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<bool>,
    /// `active`, `ungated` (active without `when <gate>`), `commented`, `absent`, `forked`
    /// (active, naming the `.local` fork) or `misplaced` (active outside the block the
    /// graph places it in)
    pub line: &'static str,
    /// the line's number, 1-based
    #[serde(rename = "at", skip_serializing_if = "Option::is_none")]
    pub at_line: Option<usize>,
    /// the path the line names when it is not the pack's own — its fork
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<String>,
    /// the param an active line is gated on when it is not the pack's gate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gated_on: Option<String>,
    /// the pack emits: its line is active and its `when` holds
    pub deploys: bool,
    pub requires: Vec<Requirement>,
    /// the packs with a requirement this one meets
    pub required_by: Vec<String>,
    pub excludes: Vec<String>,
    /// the line is written by hand, never by satz; why
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_hand: Option<String>,
    /// the compile's findings about this pack
    pub findings: Vec<String>,
}

/// A `use` of a file the pack graph does not know.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Unmanaged {
    pub path: String,
    #[serde(rename = "at")]
    pub at_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PacksReport {
    pub estate: String,
    /// why the report has no packs: the presets carry no pack graph
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// every node of the graph, in the graph's order
    pub packs: Vec<PackRow>,
    pub unmanaged: Vec<Unmanaged>,
    /// the compile's pack findings, as `transpile --check` prints them
    pub findings: Vec<Finding>,
}

/// The graph an estate command works from: an error when the presets carry none.
fn graph_for(runtime: &ToolConfig) -> Result<(PackGraph, PathBuf), BoxErr> {
    let dir = PathBuf::from(&runtime.presets_dir);
    match crate::pack_graph::read(&dir)? {
        Some(g) => Ok((g, dir)),
        None => Err(no_graph(&dir).into()),
    }
}

fn no_graph(dir: &Path) -> String {
    format!(
        "{} is not here, so no pack is known to this estate — `satz get-presets` fetches it",
        dir.join(crate::pack_graph::GRAPH_FILE).display()
    )
}

/// `satz packs <estate>`: every pack the graph offers, as this estate has it.
pub(crate) fn report(estate: &Path, runtime: &ToolConfig) -> Result<PacksReport, BoxErr> {
    let src = crate::fsx::read_to_string(estate)?;
    let label = estate.display().to_string();
    let dir = PathBuf::from(&runtime.presets_dir);
    let Some(graph) = crate::pack_graph::read(&dir)? else {
        // every `use` is one the graph does not know, because there is none
        let s = scan(&src);
        return Ok(PacksReport {
            estate: label,
            note: Some(no_graph(&dir)),
            packs: Vec::new(),
            unmanaged: s.uses.iter().filter(|l| !l.commented).map(|l| Unmanaged { path: l.written.clone(), at_line: l.index + 1 }).collect(),
            findings: Vec::new(),
        });
    };
    let lib = Library::load(&graph, &dir)?;
    let view = View::new(&graph, &lib, &src).map_err(|e| format!("{}: {}", label, e))?;
    let mut found = view.exclusion_findings(&label, &src);
    found.extend(view.line_findings(&label, &runtime.validation_level));
    let mut nodes: Vec<&Node> = graph.nodes.iter().collect();
    nodes.sort_by_key(|n| (n.order.map(|o| o + 1).unwrap_or(0), n.path.clone()));
    Ok(PacksReport {
        estate: label,
        note: None,
        packs: nodes.iter().map(|n| view.row(n, &found)).collect(),
        unmanaged: view.unmanaged(),
        findings: found.into_iter().map(|(_, f)| f).collect(),
    })
}

pub(crate) fn render_text(r: &PacksReport) -> String {
    let mut s = format!("packs — {}\n", r.estate);
    if let Some(n) = &r.note {
        s.push_str(&format!("  {}\n", n));
    }
    for p in &r.packs {
        let choice = match (&p.gate, p.value) {
            (Some(g), v) => format!(
                "  {} = {}{}",
                g,
                v.map(|b| b.to_string()).unwrap_or_else(|| "—".into()),
                match (&p.answer, &p.default) {
                    (Some(_), _) => " (answered)".to_string(),
                    (None, Some(d)) => format!(" (default {})", d),
                    (None, None) => String::new(),
                }
            ),
            (None, _) => String::new(),
        };
        s.push_str(&format!(
            "\n{} {:<9} {}{}\n",
            if p.deploys { "✓" } else { "·" },
            p.line,
            p.path,
            p.at_line.map(|a| format!(":{}", a)).unwrap_or_default()
        ));
        if !choice.is_empty() {
            s.push_str(&format!("   {}\n", choice.trim_start()));
        }
        for q in &p.requires {
            s.push_str(&format!("    needs {}{}\n", q.any_of.join(" | "), if q.met { "" } else { "  — OFF" }));
        }
        // the map is needed by every pack it gates; the text says so once, not forty times
        if !p.required_by.is_empty() && p.role == "pack" {
            s.push_str(&format!("    needed by {}\n", p.required_by.join(", ")));
        }
        if let Some(h) = &p.by_hand {
            s.push_str(&format!("    written by hand: {}\n", h));
        }
    }
    if !r.unmanaged.is_empty() {
        s.push_str("\nunmanaged — used here, unknown to the pack graph:\n");
        for u in &r.unmanaged {
            s.push_str(&format!("  {} (line {})\n", u.path, u.at_line));
        }
    }
    if !r.findings.is_empty() {
        s.push_str("\nfindings:\n");
        for f in &r.findings {
            s.push_str(&format!("  {}\n", f.message));
        }
    }
    s
}

pub(crate) fn render_markdown(r: &PacksReport) -> String {
    let cell = |t: &str| t.replace('|', "\\|").replace('\n', " ");
    let mut s = format!("# Packs — {}\n\n", cell(&r.estate));
    if let Some(n) = &r.note {
        s.push_str(&format!("{}\n\n", n));
    }
    if !r.packs.is_empty() {
        s.push_str("| Pack | Gate | Answer | Default | Line | Deploys | Needs | Needed by |\n|---|---|---|---|---|---|---|---|\n");
        for p in &r.packs {
            let needs: Vec<String> =
                p.requires.iter().map(|q| format!("{}{}", q.any_of.join(" or "), if q.met { "" } else { " (off)" })).collect();
            s.push_str(&format!(
                "| `{}` | {} | {} | {} | {}{} | {} | {} | {} |\n",
                p.path,
                p.gate.as_deref().map(|g| format!("`{}`", g)).unwrap_or_default(),
                cell(p.answer.as_deref().unwrap_or("")),
                cell(p.default.as_deref().unwrap_or("")),
                p.line,
                p.at_line.map(|a| format!(" (line {})", a)).unwrap_or_default(),
                if p.deploys { "yes" } else { "no" },
                cell(&needs.join("; ")),
                cell(&p.required_by.join(", ")),
            ));
        }
    }
    if !r.unmanaged.is_empty() {
        s.push_str("\n## Unmanaged\n\n");
        for u in &r.unmanaged {
            s.push_str(&format!("- `{}` (line {})\n", u.path, u.at_line));
        }
    }
    if !r.findings.is_empty() {
        s.push_str("\n## Findings\n\n");
        for f in &r.findings {
            s.push_str(&format!("- {}\n", cell(&f.message)));
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Writing lines
// ---------------------------------------------------------------------------

/// A pack whose line has no place in the estate: the graph puts it in a block the
/// estate does not have, and a folder is the estate's structure, never invented.
#[derive(Debug)]
pub(crate) struct Unplaced {
    pub block: String,
}

fn lines_vec(src: &str) -> Vec<&str> {
    src.split_inclusive('\n').collect()
}

/// `text` (whole lines, each ending in `\n`) inserted before line `at`, 0-based; at the
/// end when `at` is past the last line.
fn insert_before(src: &str, at: usize, text: &str) -> String {
    let lines = lines_vec(src);
    let mut out = String::with_capacity(src.len() + text.len() + 1);
    for l in &lines[..at.min(lines.len())] {
        out.push_str(l);
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(text);
    for l in &lines[at.min(lines.len())..] {
        out.push_str(l);
    }
    out
}

fn replace_line(src: &str, at: usize, line: &str) -> String {
    let mut out = String::with_capacity(src.len() + line.len());
    for (i, l) in lines_vec(src).into_iter().enumerate() {
        if i == at {
            out.push_str(line);
            if l.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(l);
        }
    }
    out
}

fn active_line(path: &str, gate: Option<&str>) -> String {
    match gate {
        None => format!("use \"{}\"", path),
        Some(g) => format!("use \"{}\" when {}", path, g),
    }
}

/// Where `n`'s line goes in `src`, by the graph's order: after the line of the pack
/// before it in the same place, else before the one after it, else where the place
/// starts — inside its block, after the estate-core line (or, in an estate without one,
/// after the top-level `params`), or at the end for a pack placed after the scaffold.
/// Returns the new text and the line's number, 1-based.
pub(crate) fn place_line(src: &str, graph: &PackGraph, n: &Node, commented: bool) -> Result<(String, usize), Unplaced> {
    let s = scan(src);
    let text = if commented {
        crate::template::pack_line(&n.path, n.gate.as_deref())
    } else {
        active_line(&n.path, n.gate.as_deref())
    };
    let phase = n.phase.as_deref().filter(|p| !p.trim().is_empty());
    let place = crate::template::place(n);
    let order = n.order.unwrap_or(usize::MAX);
    let neighbours: Vec<(usize, &UseLine)> = s
        .uses
        .iter()
        .filter_map(|l| {
            let (m, _) = node_of(graph, &l.written)?;
            let o = m.order.filter(|_| m.by_hand.is_none() && m.path != n.path)?;
            (crate::template::place(m) == place && placed_right(m, l)).then_some((o, l))
        })
        .collect();
    let prev = neighbours.iter().filter(|(o, _)| *o < order).max_by_key(|(o, l)| (*o, l.index));
    let next = neighbours.iter().filter(|(o, _)| *o > order).min_by_key(|(o, l)| (*o, l.index));
    let block = |indent: &str, lead: bool, tail: bool| {
        let mut b = String::new();
        if lead && phase.is_some() {
            b.push('\n');
        }
        if let Some(p) = phase {
            b.push_str(&crate::template::phase_comment(p, indent));
        }
        b.push_str(indent);
        b.push_str(&text);
        b.push('\n');
        if tail && phase.is_some() {
            b.push('\n');
        }
        b
    };
    let out = if let Some((_, p)) = prev {
        insert_before(src, p.index + 1, &block(&p.indent, true, false))
    } else if let Some((_, nx)) = next {
        // above the comment lines that head the next line
        let lines = lines_vec(src);
        let mut at = nx.index;
        while at > 0 {
            let t = lines[at - 1].trim_start();
            if t.starts_with("//") && parse_use(t.trim_start_matches('/').trim_start()).is_none() {
                at -= 1;
            } else {
                break;
            }
        }
        insert_before(src, at, &block(&nx.indent, false, true))
    } else {
        match place {
            Place::Block(b) => match crate::template::insert_into_block(src, b, &text, phase.unwrap_or("")) {
                Some(o) => o,
                None => match crate::template::block_stub(b, &text, phase.unwrap_or("")) {
                    Some(stub) => {
                        let mut o = src.to_string();
                        if !o.ends_with('\n') {
                            o.push('\n');
                        }
                        o.push('\n');
                        o.push_str(&stub);
                        o
                    }
                    None => return Err(Unplaced { block: b.to_string() }),
                },
            },
            Place::Menu => {
                let at = s.core.or(s.params_end).or(s.header).map(|i| i + 1).unwrap_or(0);
                let mut b = String::from("\n");
                b.push_str(&block("", false, false));
                insert_before(src, at, &b)
            }
            Place::AfterScaffold => {
                let mut o = src.to_string();
                if !o.ends_with('\n') {
                    o.push('\n');
                }
                o.push('\n');
                o.push_str(&block("", false, false));
                o
            }
        }
    };
    let at = scan(&out)
        .uses
        .iter()
        .find(|l| l.written == n.path && l.commented == commented)
        .map(|l| l.index + 1)
        .unwrap_or(0);
    Ok((out, at))
}

/// What a switch did to one line.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct LineEdit {
    pub path: String,
    /// 1-based, in the file as written
    #[serde(rename = "at")]
    pub at_line: usize,
    /// `uncommented` or `written`
    pub edit: &'static str,
}

/// Make `n`'s line active: nothing when it is, uncomment it — gated on its own gate —
/// when it is commented out, write it where the graph places it when it is absent.
pub(crate) fn line_on(src: &str, graph: &PackGraph, n: &Node) -> Result<(String, Option<LineEdit>), String> {
    let s = scan(src);
    let named = |l: &&UseLine| node_of(graph, &l.written).is_some_and(|(m, _)| m.path == n.path);
    if s.uses.iter().filter(named).any(|l| !l.commented) {
        return Ok((src.to_string(), None));
    }
    if let Some(c) = s.uses.iter().filter(named).find(|l| l.commented) {
        let mut line = format!("{}use \"{}\"", c.indent, c.written);
        if let Some(a) = &c.alias {
            line.push_str(&format!(" as {}", a));
        }
        if let Some(w) = n.gate.as_ref().or(c.when.as_ref()) {
            line.push_str(&format!(" when {}", w));
        }
        if !c.trailing.is_empty() {
            line.push_str(&format!("  {}", c.trailing));
        }
        let out = replace_line(src, c.index, &line);
        return Ok((out, Some(LineEdit { path: n.path.clone(), at_line: c.index + 1, edit: "uncommented" })));
    }
    match place_line(src, graph, n, false) {
        Ok((out, at)) => Ok((out, Some(LineEdit { path: n.path.clone(), at_line: at, edit: "written" }))),
        Err(u) => Err(format!("`{}` belongs in a `{}` block and this estate has none — add the block, then run it again", n.path, u.block)),
    }
}

/// The lines a gate answered yes switches on: every pack on that gate whose line satz
/// writes. The interview's yes goes through here.
pub(crate) fn gate_on(src: &str, graph: &PackGraph, gate: &str) -> Result<(String, Vec<LineEdit>), String> {
    let mut out = src.to_string();
    let mut edits = Vec::new();
    for n in graph.lines().into_iter().filter(|n| n.gate.as_deref() == Some(gate)) {
        // the other spelling of this pack is in use: its line stays as it is
        let s = scan(&out);
        let has = |p: &str| s.uses.iter().any(|l| node_of(graph, &l.written).is_some_and(|(m, _)| m.path == p));
        if graph.excluded_by(&n.path).iter().any(|o| o.gate.as_deref() == Some(gate) && has(&o.path)) {
            continue;
        }
        let (next, edit) = line_on(&out, graph, n)?;
        out = next;
        edits.extend(edit);
    }
    Ok((out, edits))
}

// ---------------------------------------------------------------------------
// The gating migration: `when <gate>` on every ungated line
// ---------------------------------------------------------------------------

/// One line the gating migration gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatedLine {
    pub path: String,
    /// 1-based
    pub at_line: usize,
    /// the line as it reads after the edit, trimmed
    pub text: String,
}

/// One gate the gating migration binds, and what the estate said before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateBinding {
    pub param: String,
    pub value: bool,
    /// the estate's own binding as written, or where the value came from
    pub was: String,
    /// an explicit binding to the other value was overwritten: the estate answered no
    /// and deployed the pack anyway
    pub overwrote: bool,
    pub why: String,
}

/// What the gating migration does to one estate.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gating {
    pub lines: Vec<GatedLine>,
    pub bound: Vec<GateBinding>,
    /// the ungated lines it leaves, each with the reason
    pub left: Vec<String>,
}

/// `raw` with ` when <gate>` after the `use` clause, before a trailing comment.
fn with_when(raw: &str, trailing: &str, gate: &str) -> String {
    let (code, rest) = match (trailing.is_empty(), raw.rfind(trailing)) {
        (false, Some(i)) => raw.split_at(i),
        _ => (raw, ""),
    };
    let body = code.trim_end();
    let gap = &code[body.len()..];
    if rest.is_empty() {
        format!("{} when {}", body, gate)
    } else {
        format!("{} when {}{}{}", body, gate, if gap.is_empty() { "  " } else { gap }, rest)
    }
}

fn contradiction(view: &View, src: &str) -> Option<String> {
    let clashes = view.exclusion_findings("", src);
    (!clashes.is_empty()).then(|| {
        format!(
            "no line is gated while packs that exclude one another both deploy — binding both gates true would answer one choice two ways:\n  {}",
            clashes.iter().map(|(_, f)| f.message.replace('\n', " ")).collect::<Vec<_>>().join("\n  ")
        )
    })
}

/// The refusal [`gate_ungated`] would give `src`, checked before anything is written;
/// `None` too for an estate that does not parse, which the compile reports.
pub(crate) fn gating_contradiction(graph: &PackGraph, lib: &Library, src: &str) -> Option<String> {
    contradiction(&View::new(graph, lib, src).ok()?, src)
}

/// The gating migration `merge-presets` runs: every ACTIVE `use` of a graph node — or of
/// its fork — at any depth, that has a gate and no `when`, gets ` when <gate>`, where the
/// file declaring the gate is used and `declares` confirms the copy the estate uses has
/// it. Each such gate the estate does not already bind true is bound `true`, because the
/// line deployed: the emission stays what it was — a default that is already true is
/// bound too, since the file declaring it may be used below the line. A commented line is
/// never touched.
///
/// A gate that another one follows (`use_sentinel_auditlogs = use_sentinel`) and that
/// the estate leaves unbound is bound to the value it had, so switching its leader on
/// switches nothing else on; the other options of a choice held true are bound false.
///
/// It refuses while two packs that exclude one another both deploy: gating both would
/// answer a choice two ways at once.
pub(crate) fn gate_ungated(
    graph: &PackGraph,
    lib: &Library,
    src: &str,
    declares: &dyn Fn(&str, &str) -> Result<bool, String>,
) -> Result<(String, Gating), String> {
    let view = View::new(graph, lib, src)?;
    if let Some(c) = contradiction(&view, src) {
        return Err(c);
    }
    let mut g = Gating::default();
    let mut out = src.to_string();
    let mut gates: Vec<&str> = Vec::new();
    for l in view.scan.uses.iter().filter(|l| !l.commented && l.when.is_none()) {
        let Some((n, _)) = node_of(graph, &l.written) else { continue };
        let Some(gate) = n.gate.as_deref() else { continue };
        if !view.gate_exists(n) {
            continue;
        }
        let decl = n.gate_declared_in.as_deref().unwrap_or_default();
        let written = view.lines_of(decl).0.map(|d| d.written.clone()).unwrap_or_else(|| decl.to_string());
        if !declares(&written, gate)? {
            g.left.push(format!(
                "line {}: `{}` stays ungated — `{}`, which this estate uses, does not declare `{}`",
                l.index + 1,
                l.written,
                written,
                gate
            ));
            continue;
        }
        let raw = lines_vec(&out)[l.index].trim_end_matches(['\n', '\r']).to_string();
        let line = with_when(&raw, &l.trailing, gate);
        out = replace_line(&out, l.index, &line);
        g.lines.push(GatedLine { path: n.path.clone(), at_line: l.index + 1, text: line.trim().to_string() });
        if !gates.contains(&gate) {
            gates.push(gate);
        }
    }
    let written_as = |p: &str| view.own.get(p).map(crate::doc_packs::value_text);
    let was = |p: &str| match (written_as(p), view.value(p)) {
        (Some(w), _) => w,
        (None, Some(v)) => format!("unbound, default {}", v),
        (None, None) => "unbound".to_string(),
    };
    // bound in the estate even where the default is already true: a `when` is checked where
    // the walk meets it, and an older estate often uses the map below the lines it gates
    for gate in &gates {
        if view.own.contains_key(*gate) && view.value(gate) == Some(true) {
            continue;
        }
        g.bound.push(GateBinding {
            param: gate.to_string(),
            value: true,
            was: was(gate),
            overwrote: view.own.get(*gate) == Some(&Value::Bool(false)),
            why: "its line deployed".to_string(),
        });
    }
    let on: Vec<String> = g.bound.iter().map(|b| b.param.clone()).collect();
    // a follower of a gate bound true here keeps the value it had
    for m in &graph.nodes {
        let (Some(f), Some(mg)) = (&m.follows, &m.gate) else { continue };
        if !on.contains(f)
            || view.own.contains_key(mg)
            || view.value(mg) == Some(true)
            || gates.contains(&mg.as_str())
            || g.bound.iter().any(|b| &b.param == mg)
        {
            continue;
        }
        g.bound.push(GateBinding {
            param: mg.clone(),
            value: false,
            was: was(mg),
            overwrote: false,
            why: format!("it follows `{}`, bound true here, and stays as it was", f),
        });
    }
    // the other options of a choice bound true here
    for e in graph.edges.iter().filter(|e| e.kind == EdgeKind::Excludes && e.source == Source::Derived) {
        for (a, b) in [(&e.from, &e.to), (&e.to, &e.from)] {
            let (Some(ga), Some(gb)) = (view.node(a).and_then(|n| n.gate.clone()), view.node(b).and_then(|n| n.gate.clone())) else { continue };
            if ga == gb || !on.contains(&ga) || view.value(&gb) != Some(true) || g.bound.iter().any(|x| x.param == gb) {
                continue;
            }
            g.bound.push(GateBinding {
                param: gb.clone(),
                value: false,
                was: was(&gb),
                overwrote: view.own.get(&gb) == Some(&Value::Bool(true)),
                why: format!("it is the other option of `{}`, bound true here, and its pack does not deploy", ga),
            });
        }
    }
    for b in &g.bound {
        out = crate::interview::bind(&out, &b.param, &serde_yaml::Value::Bool(b.value))?;
    }
    Ok((out, g))
}

// ---------------------------------------------------------------------------
// `satz add-pack` and `satz remove-pack`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Bound {
    pub param: String,
    pub value: bool,
}

/// What `add-pack` or `remove-pack` changed.
#[derive(Debug, Clone, Default, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct PackChange {
    pub estate: String,
    /// `add` or `remove`
    pub action: &'static str,
    /// the packs switched, requirements and dependents included
    pub switched: Vec<String>,
    pub bound: Vec<Bound>,
    pub lines: Vec<LineEdit>,
    /// what the switch left as it is, and why
    pub left: Vec<String>,
    /// the questions the switch opened: the estate has to answer them now
    pub opened: Vec<String>,
}

/// The nodes `arg` names: one pack by path (`presets/…`, the path without `presets/`,
/// or its fork), or every pack on a gate.
fn resolve<'g>(graph: &'g PackGraph, arg: &str) -> Result<Vec<&'g Node>, String> {
    let as_path = if arg.starts_with("presets/") { arg.to_string() } else { format!("presets/{}", arg) };
    if let Some((n, _)) = node_of(graph, arg).or_else(|| node_of(graph, &as_path)) {
        return Ok(vec![n]);
    }
    let on_gate: Vec<&Node> = graph.nodes.iter().filter(|n| n.gate.as_deref() == Some(arg)).collect();
    if on_gate.is_empty() {
        return Err(format!("`{}` is neither a pack the pack graph offers nor a gate — `satz packs <estate>` lists both", arg));
    }
    Ok(on_gate)
}

/// The estate written, compiled, and restored when the compile refuses: an edit that
/// leaves the estate broken is never left behind (ADR 0023's rule).
fn write_proven(estate: &Path, before: &str, after: &str, tool: &ToolConfig, runtime: &ToolConfig) -> Result<(), BoxErr> {
    crate::fsx::write_edited_satz(estate, before, after)?;
    if let Err(e) = crate::pipeline_b_compile(estate, tool, runtime, crate::PrerequisiteFindings::Quiet, crate::FindingsOutput::Silent) {
        return Err(match crate::fsx::write_verbatim(estate, before) {
            Ok(()) => format!("the edited estate does not compile — {} restored:\n{}", estate.display(), e).into(),
            Err(w) => format!("the edited estate does not compile ({}) — and restoring {} failed: {}", e, estate.display(), w).into(),
        });
    }
    Ok(())
}

/// `satz add-pack <estate> <gate|path>`.
pub(crate) fn add(estate: &Path, tool: &ToolConfig, runtime: &ToolConfig, arg: &str, with_requirements: bool) -> Result<PackChange, BoxErr> {
    let (graph, dir) = graph_for(runtime)?;
    let lib = Library::load(&graph, &dir)?;
    let before = crate::fsx::read_to_string(estate)?;
    let view = View::new(&graph, &lib, &before).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let mut targets = resolve(&graph, arg)?;
    for n in &targets {
        match n.role {
            Role::Core => return Err(format!("`{}` is the day-0 pack every estate starts with — it is not switched", n.path).into()),
            Role::Map | Role::Pack => {}
        }
    }
    if targets.len() > 1 {
        targets.retain(|n| n.by_hand.is_none());
    }
    if let [n] = targets.as_slice() {
        if let Some(why) = &n.by_hand {
            return Err(format!("`{}`'s line is written by hand, never by satz: {}", n.path, why).into());
        }
    }

    // the plan: requirements first, then the packs asked for, then what follows them
    let mut plan: Vec<&Node> = Vec::new();
    let mut refusals: Vec<String> = Vec::new();
    fn close<'g>(
        view: &View<'g>,
        n: &'g Node,
        with: bool,
        plan: &mut Vec<&'g Node>,
        refusals: &mut Vec<String>,
        depth: u8,
    ) {
        if plan.iter().any(|p| p.path == n.path) || depth > 16 {
            return;
        }
        let planned: Vec<String> = plan.iter().map(|p| p.path.clone()).collect();
        let on = |p: &str| view.deploys(p) || planned.iter().any(|q| q == p) || p == n.path;
        for r in view.requirements(n, &on).into_iter().filter(|r| !r.met) {
            match (with, r.any_of.as_slice()) {
                (true, [one]) => match view.node(one) {
                    Some(m) => close(view, m, with, plan, refusals, depth + 1),
                    None => refusals.push(view.requirement_text(n, &r)),
                },
                (true, _) => refusals.push(format!("{} — which one is a decision: add-pack it first", view.requirement_text(n, &r))),
                (false, _) => refusals.push(format!("{} — `satz add-pack` it first", view.requirement_text(n, &r))),
            }
        }
        plan.push(n);
    }
    for n in &targets {
        close(&view, n, with_requirements, &mut plan, &mut refusals, 0);
    }
    // the packs whose gate follows one being switched on come with it, unless the estate
    // decided them itself
    let mut left = Vec::new();
    let mut children: Vec<&Node> = Vec::new();
    for n in graph.lines() {
        let Some(f) = &n.follows else { continue };
        if !plan.iter().any(|p| p.gate.as_ref() == Some(f)) || plan.iter().any(|p| p.path == n.path) {
            continue;
        }
        let g = n.gate.as_deref().unwrap_or_default();
        if view.own.contains_key(g) {
            left.push(format!("`{}` stays as `{}` binds it — it follows `{}` only while unbound", n.path, g, f));
        } else {
            children.push(n);
        }
    }
    for c in children {
        close(&view, c, with_requirements, &mut plan, &mut refusals, 0);
    }
    for n in &plan {
        for x in graph.excluded_by(&n.path) {
            if view.deploys(&x.path) && !plan.iter().any(|p| p.path == x.path) {
                refusals.push(format!("`{}` excludes `{}`, which is on — `satz remove-pack` it first", n.path, x.path));
            }
        }
    }
    if !refusals.is_empty() {
        refusals.dedup();
        let hint = if with_requirements { "" } else { "\n  `--with-requirements` switches a requirement on too, where there is one to choose" };
        return Err(format!("add-pack {}: refused, nothing written:\n  {}{}", arg, refusals.join("\n  "), hint).into());
    }

    // the questions as they stand, for the choices and for what opens
    let rows = crate::questions::questions_report(estate, runtime)?;
    let asked: BTreeSet<String> = rows.questions.iter().filter(|q| q.state != "not-applicable").map(|q| q.subject.clone()).collect();
    let mut change = PackChange { estate: estate.display().to_string(), action: "add", left, ..PackChange::default() };
    let mut src = before.clone();
    let yes = serde_yaml::Value::Bool(true);
    for n in &plan {
        let following = n.follows.as_ref().is_some_and(|f| plan.iter().any(|p| p.gate.as_ref() == Some(f)));
        if let (Some(g), false) = (&n.gate, following) {
            if view.own.get(g) != Some(&Value::Bool(true)) && !change.bound.iter().any(|b| &b.param == g) {
                let oneof = rows.questions.iter().find(|q| q.kind == "oneof" && q.options.iter().any(|o| &o.param == g));
                src = match oneof {
                    Some(row) => crate::interview::answer(&src, row, &serde_yaml::Value::String(g.clone()), None)?,
                    None => crate::interview::bind(&src, g, &yes)?,
                };
                change.bound.push(Bound { param: g.clone(), value: true });
                if let Some(row) = oneof {
                    for o in row.options.iter().filter(|o| &o.param != g) {
                        change.bound.push(Bound { param: o.param.clone(), value: false });
                    }
                }
            }
        }
        let (next, edit) = line_on(&src, &graph, n)?;
        src = next;
        change.lines.extend(edit);
        change.switched.push(n.path.clone());
    }
    if src == before {
        change.left.push("already on — nothing to write".to_string());
        return Ok(change);
    }
    write_proven(estate, &before, &src, tool, runtime)?;
    // the line numbers as written: a later line moves an earlier edit
    let s = scan(&crate::fsx::read_to_string(estate)?);
    for e in &mut change.lines {
        if let Some(l) = s.uses.iter().find(|l| !l.commented && node_of(&graph, &l.written).is_some_and(|(m, _)| m.path == e.path)) {
            e.at_line = l.index + 1;
        }
    }
    let after = crate::questions::questions_report(estate, runtime)?;
    change.opened = after.questions.iter().filter(|q| q.state == "unanswered" && !asked.contains(&q.subject)).map(|q| q.subject.clone()).collect();
    Ok(change)
}

/// `satz remove-pack <estate> <gate|path>`: the gate bound false, the line left as it
/// is — a gated line with a false gate deploys nothing.
pub(crate) fn remove(estate: &Path, tool: &ToolConfig, runtime: &ToolConfig, arg: &str, cascade: bool) -> Result<PackChange, BoxErr> {
    let (graph, dir) = graph_for(runtime)?;
    let lib = Library::load(&graph, &dir)?;
    let before = crate::fsx::read_to_string(estate)?;
    let view = View::new(&graph, &lib, &before).map_err(|e| format!("{}: {}", estate.display(), e))?;
    let named = resolve(&graph, arg)?;
    let mut off: Vec<&Node> = Vec::new();
    for n in named {
        let Some(g) = &n.gate else {
            return Err(format!("`{}` has no gate, so nothing switches it off", n.path).into());
        };
        // one gate switches every pack on it
        for m in graph.nodes.iter().filter(|m| m.gate.as_ref() == Some(g)) {
            if !off.iter().any(|o| o.path == m.path) {
                off.push(m);
            }
        }
    }
    let followers = |off: &Vec<&Node>| -> Vec<&Node> {
        graph
            .nodes
            .iter()
            .filter(|m| {
                m.follows.as_ref().is_some_and(|f| off.iter().any(|o| o.gate.as_ref() == Some(f)))
                    && !m.gate.as_ref().is_some_and(|g| view.own.contains_key(g))
                    && !off.iter().any(|o| o.path == m.path)
            })
            .collect()
    };
    let mut refusals = Vec::new();
    loop {
        let more = followers(&off);
        off.extend(more);
        let gone: Vec<String> = off.iter().map(|o| o.path.clone()).collect();
        let on = |p: &str| view.deploys(p) && !gone.iter().any(|g| g == p);
        let mut dependents: Vec<&Node> = Vec::new();
        for m in graph.nodes.iter().filter(|m| on(&m.path)) {
            for r in view.requirements(m, &on).into_iter().filter(|r| !r.met) {
                let Some(t) = r.any_of.iter().find(|p| gone.contains(p)) else { continue };
                if cascade {
                    if !dependents.iter().any(|d| d.path == m.path) {
                        dependents.push(m);
                    }
                } else {
                    let tg = view.node(t).and_then(|n| n.gate.clone()).map(|g| format!(" (`{}`)", g)).unwrap_or_default();
                    refusals.push(format!("`{}` needs `{}`{}, which this would switch off — `--cascade` switches `{}` off too", m.path, t, tg, m.path));
                }
            }
        }
        if dependents.is_empty() {
            break;
        }
        for d in dependents {
            if d.gate.is_none() {
                refusals.push(format!("`{}` needs what this switches off and has no gate to switch it off with", d.path));
            } else {
                off.push(d);
            }
        }
        if !refusals.is_empty() {
            break;
        }
    }
    // a false gate stops a pack only through a line gated on it
    for n in &off {
        let (Some(l), Some(g)) = (view.lines_of(&n.path).0, &n.gate) else { continue };
        match &l.when {
            None => refusals.push(format!(
                "`{}` is used without `when {}` at line {}, so binding `{}` false does not switch it off — `satz merge-presets` gates it, or write `use \"{}\" when {}` there first",
                n.path, g, l.index + 1, g, l.written, g
            )),
            Some(w) if w != g && !off.iter().any(|o| o.gate.as_ref() == Some(w)) => refusals.push(format!(
                "`{}` is gated on `{}`, not `{}`, at line {}, so binding `{}` false does not switch it off — write `when {}` there first",
                n.path, w, g, l.index + 1, g, g
            )),
            Some(_) => {}
        }
    }
    if !refusals.is_empty() {
        refusals.dedup();
        return Err(format!("remove-pack {}: refused, nothing written:\n  {}", arg, refusals.join("\n  ")).into());
    }
    let mut change = PackChange { estate: estate.display().to_string(), action: "remove", ..PackChange::default() };
    let mut src = before.clone();
    let no = serde_yaml::Value::Bool(false);
    let gates: Vec<&str> = off.iter().filter_map(|n| n.gate.as_deref()).collect();
    for n in &off {
        change.switched.push(n.path.clone());
        let Some(g) = &n.gate else { continue };
        // a gate that follows one bound false here goes with it, bound or not
        let follows_off = n.follows.as_deref().is_some_and(|f| gates.contains(&f)) && !view.own.contains_key(g);
        if follows_off || change.bound.iter().any(|b| &b.param == g) {
            continue;
        }
        if view.own.get(g) == Some(&Value::Bool(false)) {
            continue;
        }
        src = crate::interview::bind(&src, g, &no)?;
        change.bound.push(Bound { param: g.clone(), value: false });
    }
    if src == before {
        change.left.push("already off — nothing to write".to_string());
        return Ok(change);
    }
    write_proven(estate, &before, &src, tool, runtime)?;
    Ok(change)
}

pub(crate) fn render_change(c: &PackChange) -> String {
    let mut s = format!("{}-pack — {}\n", c.action, c.estate);
    for b in &c.bound {
        s.push_str(&format!("  bound {} = {}\n", b.param, b.value));
    }
    for l in &c.lines {
        s.push_str(&format!("  {} line {}: {}\n", l.edit, l.at_line, l.path));
    }
    for l in &c.left {
        s.push_str(&format!("  {}\n", l));
    }
    if !c.opened.is_empty() {
        s.push_str(&format!(
            "  {} question(s) opened: {}\n  next: satz interview {}\n",
            c.opened.len(),
            c.opened.join(", "),
            c.estate
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> PackGraph {
        crate::template::tests::shipped()
    }

    fn lib(g: &PackGraph) -> Library {
        Library::load(g, &Path::new(env!("CARGO_MANIFEST_DIR")).join("presets")).expect("the shipped library loads")
    }

    fn messages(src: &str) -> Vec<String> {
        let g = graph();
        let (a, b) = compile_findings(&g, &Path::new(env!("CARGO_MANIFEST_DIR")).join("presets"), "e.satz", src, "warn");
        a.into_iter().chain(b).map(|f| f.message).collect()
    }

    const HEAD: &str = "estate e\n\nparams {\n  x = 1\n}\n\nuse \"presets/estate-core.satz\"\nuse \"presets/estate-map.satz\"\n";

    #[test]
    fn the_scanner_reads_lines_blocks_and_anchors() {
        let src = "estate e\n\nparams {\n  a = \"{b}\"\n}\n\n// use \"presets/estate-core.satz\"\ngoogle_folder {\n  infra_folder {\n    display_name = \"x { y\"\n    use \"presets/p.satz\" when g  // note\n  }\n}\n";
        let s = scan(src);
        assert_eq!((s.header, s.params_end, s.core), (Some(0), Some(4), Some(6)));
        let l = s.uses.iter().find(|l| l.written == "presets/p.satz").unwrap();
        assert_eq!(l.blocks, vec!["google_folder".to_string(), "infra_folder".to_string()]);
        assert_eq!((l.when.as_deref(), l.trailing.as_str(), l.commented), (Some("g"), "// note", false));
    }

    #[test]
    fn a_gate_reads_the_estate_then_the_default_of_a_used_declaring_file() {
        let g = graph();
        let l = lib(&g);
        let with_map = format!("{}use \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel\n", HEAD.replace("x = 1", "use_sentinel = true"));
        let v = View::new(&g, &l, &with_map).unwrap();
        assert_eq!(v.value("use_sentinel"), Some(true));
        // follows: the log fragment's gate defaults to use_sentinel by reference
        assert_eq!(v.value("use_sentinel_auditlogs"), Some(true));
        let no_map = "estate e\n\nparams {\n  x = 1\n}\n";
        assert_eq!(View::new(&g, &l, no_map).unwrap().value("use_sentinel"), None, "the map is not used, so its default says nothing");
    }

    #[test]
    fn a_pack_answered_for_but_commented_out_or_absent_is_found() {
        let src = format!("{}// use \"presets/organization-budget.satz\" when use_budget\n", HEAD.replace("x = 1", "use_budget = true"));
        let m = messages(&src);
        assert!(m.iter().any(|m| m.contains("use_budget") && m.contains("still commented out")), "{m:?}");
        let src = HEAD.replace("x = 1", "use_budget = true");
        assert!(messages(&src).iter().any(|m| m.contains("has no line for `presets/organization-budget.satz`")));
        // the fork is the pack
        let src = format!("{}use \"presets/organization-budget.local.satz\" when use_budget\n", HEAD.replace("x = 1", "use_budget = true"));
        assert!(messages(&src).iter().all(|m| !m.contains("organization-budget")), "{:?}", messages(&src));
    }

    #[test]
    fn packs_that_exclude_one_another_under_one_gate_are_alternatives() {
        let head = HEAD.replace("x = 1", "security_model_s1 = true\n  security_model_s2 = false");
        let split = format!(
            "{}google_cloud_identity_group {{\n  use \"presets/security-group-models/s1-group-definitions.satz\" when security_model_s1\n}}\n",
            head
        );
        assert!(messages(&split).iter().all(|m| !m.contains("s1-security-groups")), "{:?}", messages(&split));
        let neither = messages(&head);
        assert!(neither.iter().any(|m| m.contains("s1-security-groups.satz")), "{neither:?}");
    }

    #[test]
    fn an_ungated_line_is_found_only_where_its_gate_exists() {
        let src = format!("{}use \"presets/organization-budget.satz\"\n", HEAD.replace("x = 1", "use_budget = false"));
        assert!(messages(&src).iter().any(|m| m.contains("without `when use_budget`")), "{:?}", messages(&src));
        let no_map = "estate e\n\nparams {\n  x = 1\n}\n\nuse \"presets/organization-budget.satz\"\n";
        assert!(messages(no_map).iter().all(|m| !m.contains("without `when")), "without the map a no is not possible");
    }

    #[test]
    fn a_requirement_that_is_off_is_found_and_the_estate_can_meet_a_data_one_itself() {
        let billing = format!(
            "{}use \"presets/billing-account-permissions.satz\" when use_billing_permissions\n",
            HEAD.replace("x = 1", "use_billing_permissions = true")
        );
        let m = messages(&billing);
        assert!(m.iter().any(|m| m.contains("billing-account-permissions.satz` needs one of") && m.contains("none is on")), "{m:?}");
        // the runner grant, with the runner in another estate
        let g = graph();
        let l = lib(&g);
        let grant = "presets/ci/verification-runner-grant.satz";
        let src = format!(
            "{}use \"{}\" when use_verification_runner_grant\n",
            HEAD.replace("x = 1", "use_verification_runner_grant = true\n  ci_runner_service_account = \"r@example.com\""),
            grant
        );
        let v = View::new(&g, &l, &src).unwrap();
        let n = v.node(grant).unwrap();
        assert!(v.requirements(n, &|p| v.deploys(p)).iter().all(|r| r.met), "a bound reader meets the data requirement");
        let src = format!("{}use \"{}\" when use_verification_runner_grant\n", HEAD.replace("x = 1", "use_verification_runner_grant = true"), grant);
        let v = View::new(&g, &l, &src).unwrap();
        assert!(v.requirements(v.node(grant).unwrap(), &|p| v.deploys(p)).iter().any(|r| !r.met && r.kind == "data"));
    }

    #[test]
    fn a_dry_run_and_its_enforcing_pack_both_on_is_the_dry_run_error() {
        let src = format!(
            "{}use \"presets/cis/CIS-GCP-Foundation-4.0.satz\" when use_cis_baseline\nuse \"presets/cis/cloud-sql.satz\" when cis_cloud_sql_hardening\nuse \"presets/cis/cloud-sql-dry-run.satz\" when cis_cloud_sql_hardening_dry_run\n",
            HEAD.replace("x = 1", "use_cis_baseline = true\n  cis_cloud_sql_hardening = true\n  cis_cloud_sql_hardening_dry_run = true")
        );
        let g = graph();
        let (stop, _) = compile_findings(&g, &Path::new(env!("CARGO_MANIFEST_DIR")).join("presets"), "e.satz", &src, "warn");
        assert_eq!(stop.len(), 1, "{stop:?}");
        assert_eq!(stop[0].kind, Kind::DryRunConflict);
        assert!(stop[0].message.contains("REPLACES enforcement"), "{}", stop[0].message);
        // the S1 model's two spellings on one gate fold as one: no error
        let both = format!(
            "{}use \"presets/security-group-models/s1-security-groups.satz\" when security_model_s1\ngoogle_cloud_identity_group {{\n  use \"presets/security-group-models/s1-group-definitions.satz\" when security_model_s1\n}}\n",
            HEAD.replace("x = 1", "security_model_s1 = true\n  security_model_s2 = false")
        );
        let (stop, _) = compile_findings(&g, &Path::new(env!("CARGO_MANIFEST_DIR")).join("presets"), "e.satz", &both, "warn");
        assert!(stop.is_empty(), "{stop:?}");
    }

    #[test]
    fn a_line_goes_where_the_graph_orders_it() {
        let g = graph();
        let node = |p: &str| g.nodes.iter().find(|n| n.path == p).unwrap();
        // an imported estate: no estate-core line, a folder block, no pack line at all
        let src = "estate e\n\nparams {\n  x = 1\n}\n\ngoogle_folder {\n  infra_folder {\n    display_name = \"Infra\"\n  }\n}\n";
        let (with_map, at) = place_line(src, &g, node("presets/estate-map.satz"), false).map_err(|u| u.block).unwrap();
        let map_at = with_map.lines().position(|l| l.contains("estate-map.satz")).unwrap();
        let folder_at = with_map.lines().position(|l| l.starts_with("google_folder")).unwrap();
        assert!(map_at < folder_at, "the map goes above the blocks whose lines it gates:\n{with_map}");
        assert_eq!(at, map_at + 1);
        // the alerts after the logsink in the folder, whichever is written first
        let (a, _) = place_line(&with_map, &g, node("presets/monitoring/organization-cis-log-alerts-central.satz"), false).map_err(|u| u.block).unwrap();
        let (b, _) = place_line(&a, &g, node("presets/monitoring/organization-audit-logsink.satz"), false).map_err(|u| u.block).unwrap();
        let sink = b.lines().position(|l| l.contains("organization-audit-logsink")).unwrap();
        let alerts = b.lines().position(|l| l.contains("organization-cis-log-alerts-central")).unwrap();
        assert!(sink < alerts, "{b}");
        let s = scan(&b);
        assert!(s.uses.iter().all(|l| l.written.contains("estate-map") || l.blocks == vec!["google_folder", "infra_folder"]), "{b}");
        // a pack placed after the scaffold lands at the end, below the folder
        let (c, _) = place_line(&b, &g, node("presets/integrations/microsoft-sentinel.satz"), true).map_err(|u| u.block).unwrap();
        assert!(c.trim_end().ends_with("// use \"presets/integrations/microsoft-sentinel.satz\" when use_sentinel"), "{c}");
        // no folder: reported, never invented
        let bare = "estate e\n\nparams {\n  x = 1\n}\n";
        assert_eq!(place_line(bare, &g, node("presets/monitoring/organization-audit-logsink.satz"), false).unwrap_err().block, "google_folder.infra_folder");
    }

    fn bound(src: &str, param: &str) -> Option<bool> {
        match satz_core::satz::parse(src).ok()?.params.into_iter().find(|(n, _, _)| n == param)?.1 {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    fn gate(src: &str) -> Result<(String, Gating), String> {
        let g = graph();
        let l = lib(&g);
        gate_ungated(&g, &l, src, &|_, _| Ok(true))
    }

    #[test]
    fn the_gating_migration_gates_every_ungated_line_and_binds_what_deployed() {
        let src = format!(
            "{}google_essential_contacts_contact {{\n  use \"presets/essential-contacts-organization.satz\"\n}}\n\ngoogle_folder {{\n  infra_folder {{\n    display_name = \"Infra\"\n    use \"presets/monitoring/organization-audit-logsink.satz\"  // the archive\n    use \"presets/monitoring/organization-cis-log-alerts-central.satz\" when use_central_alerts\n  }}\n}}\n\nuse \"presets/scc/scc-findings-mail.satz\"\n// use \"presets/organization-budget.satz\"\n",
            HEAD.replace("x = 1", "use_audit_logsink = false\n  use_essential_contacts = true")
        );
        let (out, g) = gate(&src).unwrap();
        // in a resource map, in a folder with its trailing comment, at the top level
        assert!(out.contains("\n  use \"presets/essential-contacts-organization.satz\" when use_essential_contacts\n"), "{out}");
        assert!(out.contains("\n    use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink  // the archive\n"), "{out}");
        assert!(out.contains("\nuse \"presets/scc/scc-findings-mail.satz\" when use_scc_findings_mail\n"), "{out}");
        // a commented line keeps its `//` and its text; a gated one is left alone
        assert!(out.contains("\n// use \"presets/organization-budget.satz\"\n"), "{out}");
        assert_eq!(g.lines.len(), 3, "{:?}", g.lines);
        // the explicit no is overwritten and says so; the unanswered one says its default;
        // the one already true is not bound again
        let b = |p: &str| g.bound.iter().find(|b| b.param == p).cloned();
        let sink = b("use_audit_logsink").unwrap();
        assert!(sink.value && sink.overwrote && sink.was == "false", "{sink:?}");
        let mail = b("use_scc_findings_mail").unwrap();
        assert!(mail.value && !mail.overwrote && mail.was == "unbound, default false", "{mail:?}");
        assert!(b("use_essential_contacts").is_none());
        // a default that is already true is bound too: the map may be used below the line
        let late_map = "estate e\n\nparams {\n  x = 1\n}\n\nuse \"presets/estate-core.satz\"\nuse \"presets/cis/CIS-GCP-Foundation-4.0.satz\"\nuse \"presets/estate-map.satz\"\n";
        let (late, lg) = gate(late_map).unwrap();
        assert_eq!(bound(&late, "use_cis_baseline"), Some(true), "{late}");
        assert_eq!(lg.bound[0].was, "unbound, default true");
        assert_eq!((bound(&out, "use_audit_logsink"), bound(&out, "use_scc_findings_mail")), (Some(true), Some(true)), "{out}");
        // no line of a gated pack is left ungated, and a second run changes nothing
        let v = View::new(&graph(), &lib(&graph()), &out).map(|v| graph().nodes.iter().filter(|n| v.state(n).0 == "ungated").count()).unwrap();
        assert_eq!(v, 0);
        let (again, g2) = gate(&out).unwrap();
        assert_eq!(again, out);
        assert!(g2.lines.is_empty() && g2.bound.is_empty());
    }

    #[test]
    fn the_gating_migration_keeps_what_follows_a_gate_it_binds_and_the_other_option_of_a_choice() {
        let src = format!(
            "{}use \"presets/integrations/microsoft-sentinel.satz\"\n// use \"presets/integrations/microsoft-sentinel-auditlogs.satz\" when use_sentinel_auditlogs\nuse \"presets/security-group-models/s2-security-groups.satz\"\n// use \"presets/security-group-models/s1-security-groups.satz\" when security_model_s1\n",
            HEAD.replace("x = 1", "use_sentinel = false")
        );
        let (out, g) = gate(&src).unwrap();
        let b = |p: &str| g.bound.iter().find(|b| b.param == p).cloned().unwrap();
        assert!(b("use_sentinel").value);
        let follower = b("use_sentinel_auditlogs");
        assert!(!follower.value && follower.why.contains("follows `use_sentinel`"), "{follower:?}");
        assert!(b("security_model_s2").value);
        let other = b("security_model_s1");
        assert!(!other.value && other.was == "unbound, default true", "{other:?}");
        assert_eq!((bound(&out, "use_sentinel_auditlogs"), bound(&out, "security_model_s1")), (Some(false), Some(false)), "{out}");
    }

    #[test]
    fn the_gating_migration_refuses_both_options_of_a_choice_and_leaves_a_gate_the_used_copy_lacks() {
        let both = format!(
            "{}use \"presets/security-group-models/s1-security-groups.satz\"\nuse \"presets/security-group-models/s2-security-groups.satz\"\n",
            HEAD
        );
        let e = gate(&both).unwrap_err();
        assert!(e.contains("s1-security-groups.satz") && e.contains("s2-security-groups.satz"), "{e}");
        // the S1 model's two spellings on one gate are not a contradiction: both are gated
        let spellings = format!(
            "{}use \"presets/security-group-models/s1-security-groups.satz\"\ngoogle_cloud_identity_group {{\n  use \"presets/security-group-models/s1-group-definitions.satz\"\n}}\n",
            HEAD
        );
        let (_, g) = gate(&spellings).unwrap();
        assert_eq!(g.lines.len(), 2, "{:?}", g.lines);
        // the map copy the estate uses does not declare the gate: the line stays, reported
        let g0 = graph();
        let l = lib(&g0);
        let src = format!("{}use \"presets/organization-budget.satz\"\n", HEAD);
        let (out, g) = gate_ungated(&g0, &l, &src, &|_, gate| Ok(gate != "use_budget")).unwrap();
        assert_eq!(out, src);
        assert!(g.lines.is_empty() && g.left.len() == 1 && g.left[0].contains("does not declare `use_budget`"), "{:?}", g.left);
        // without the map no gate exists, so nothing is gated
        let no_map = "estate e\n\nparams {\n  x = 1\n}\n\nuse \"presets/organization-budget.satz\"\n";
        let (out, g) = gate(no_map).unwrap();
        assert_eq!(out, no_map);
        assert!(g.lines.is_empty());
    }

    #[test]
    fn a_yes_uncomments_its_line_by_path_and_gates_it_on_its_own_gate() {
        let g = graph();
        let src = "  // use \"presets/ci/verification-runner-grant.satz\" when use_verification_runner  // hosted\n";
        let (out, edits) = gate_on(src, &g, "use_verification_runner_grant").unwrap();
        assert_eq!(out, "  use \"presets/ci/verification-runner-grant.satz\" when use_verification_runner_grant  // hosted\n");
        assert_eq!(edits.len(), 1);
        // the runner's own yes does not touch the grant's line
        let (same, none) = gate_on(src, &g, "use_verification_runner").unwrap();
        assert!(none.iter().all(|e| e.path != "presets/ci/verification-runner-grant.satz"));
        assert!(same.contains("// use \"presets/ci/verification-runner-grant.satz\""));
    }
}
