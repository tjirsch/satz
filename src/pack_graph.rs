//! `satz pack-graph`: the authoring command that checks the library and writes
//! `presets/pack-graph.json`, the graph every estate command reads.
//!
//! The nodes are the library's files. What the map says about each pack — its gate,
//! its phase, its block, its adoption order — comes from the map's `offers`
//! entries. The edges are DERIVED from the packs wherever they show them (a param
//! one pack reads and another declares, a gate a pack declares for another, an
//! `ask_when`, the options of one `question oneof`) and DECLARED on an entry only
//! where they do not. The checks run on the write path as much as under `--check`:
//! a graph that fails one is never written.

use crate::doc_packs;
use crate::template;
use satz_core::pack_graph::{Edge, EdgeKind, Node, Notice, PackGraph, Role, Source};
use satz_core::satz::{File, OffersDecl, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

type BoxErr = Box<dyn std::error::Error>;

/// The file `pack-graph` writes, beside the packs it describes.
pub(crate) const GRAPH_FILE: &str = "pack-graph.json";
const MAP: &str = "estate-map.satz";
const CORE: &str = "estate-core.satz";

/// One failed check: its number in the list the command documents, and what failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) check: u8,
    pub(crate) text: String,
}

fn finding(check: u8, text: String) -> Finding {
    Finding { check, text }
}

/// A file's `notice` statements, as the graph carries them.
fn notices_of(f: &File) -> Vec<Notice> {
    f.notices
        .iter()
        .map(|n| Notice { param: n.param.clone(), text: n.text.clone(), run: n.run.clone(), before: n.before.clone() })
        .collect()
}

/// A library path as a `use` line names it.
fn lib_path(rel: &Path) -> String {
    format!("presets/{}", crate::fsx::slash(rel))
}

/// The graph of the library `all` (every pack, as `doc_packs::packs` reads them),
/// and every check it fails.
pub(crate) fn build(all: &[(PathBuf, File, String)]) -> Result<(PackGraph, Vec<Finding>), BoxErr> {
    let mut findings = Vec::new();
    let by_path: BTreeMap<String, &File> = all.iter().map(|(rel, f, _)| (lib_path(rel), f)).collect();
    let map_path = format!("presets/{}", MAP);
    let core_path = format!("presets/{}", CORE);
    let map = *by_path
        .get(&map_path)
        .ok_or_else(|| format!("pack-graph: the library has no {} — the map is where the offered packs are written", map_path))?;
    let at_map = |line: usize| format!("{}:{}", map_path, line);

    // every param, and where it is declared
    let mut declared: BTreeMap<&str, Vec<(&str, usize, &Value)>> = BTreeMap::new();
    for (path, f) in &by_path {
        for (n, v, line) in &f.params {
            declared.entry(n.as_str()).or_default().push((path.as_str(), *line, v));
        }
    }

    // ---- nodes -------------------------------------------------------------
    let mut nodes: Vec<Node> = Vec::new();
    if let Some(core) = by_path.get(&core_path) {
        nodes.push(Node {
            path: core_path.clone(),
            version: core.version.clone(),
            role: Role::Core,
            gate: None,
            gate_declared_in: None,
            follows: None,
            order: None,
            phase: None,
            block: None,
            after_scaffold: false,
            by_hand: None,
            location: None,
            notices: notices_of(core),
        });
    }
    let own = map.offers.iter().position(|o| o.path == map_path);
    if own.is_none() {
        nodes.push(Node {
            path: map_path.clone(),
            version: map.version.clone(),
            role: Role::Map,
            gate: None,
            gate_declared_in: None,
            follows: None,
            order: None,
            phase: None,
            block: None,
            after_scaffold: false,
            by_hand: None,
            location: None,
            notices: notices_of(map),
        });
    }
    let mut entries: Vec<&OffersDecl> = Vec::new();
    for (order, o) in map.offers.iter().enumerate() {
        // check 3: a path is offered once
        if let Some(first) = entries.iter().find(|e| e.path == o.path) {
            findings.push(finding(
                3,
                format!("{}: `{}` is offered twice (line {} and line {})", map_path, o.path, first.line, o.line),
            ));
            continue;
        }
        let Some(file) = by_path.get(&o.path) else {
            findings.push(finding(1, format!("{}: offers `{}`, which is not in the library", at_map(o.line), o.path)));
            continue;
        };
        let role = if o.path == map_path {
            Role::Map
        } else if o.path == core_path {
            findings.push(finding(1, format!("{}: the day-0 pack is no offer — every estate starts with it", at_map(o.line))));
            continue;
        } else {
            Role::Pack
        };
        // check 2: every pack has a gate, and the map none
        let (mut gate_declared_in, mut follows) = (None, None);
        match (&o.when, role) {
            (Some(g), Role::Map) => findings.push(finding(
                2,
                format!("{}: the map is gated on `{}` — the map declares the gates, so nothing can switch it off", at_map(o.line), g),
            )),
            (None, Role::Pack) => findings.push(finding(
                2,
                format!("{}: `{}` has no `when` — every offered pack is gated, so that a no never deploys", at_map(o.line), o.path),
            )),
            (Some(g), _) => match declared.get(g.as_str()).map(|d| d.as_slice()).unwrap_or_default() {
                [] => findings.push(finding(
                    2,
                    format!("{}: `{}` is gated on `{}`, which no library file declares", at_map(o.line), o.path, g),
                )),
                [(path, _, value)] => {
                    gate_declared_in = Some(path.to_string());
                    if let Value::Ref(other) = value {
                        follows = Some(other.clone());
                    }
                }
                many => findings.push(finding(
                    2,
                    format!(
                        "{}: the gate `{}` of `{}` is declared {} times ({}) — a gate is declared once",
                        at_map(o.line),
                        g,
                        o.path,
                        many.len(),
                        many.iter().map(|(p, l, _)| format!("{}:{}", p, l)).collect::<Vec<_>>().join(", ")
                    ),
                )),
            },
            (None, _) => {}
        }
        entries.push(o);
        nodes.push(Node {
            path: o.path.clone(),
            version: file.version.clone(),
            role,
            gate: o.when.clone(),
            gate_declared_in,
            follows,
            order: Some(order),
            phase: o.phase.clone(),
            block: o.block.clone(),
            after_scaffold: o.after_scaffold,
            by_hand: o.by_hand.clone(),
            location: Some(at_map(o.line)),
            notices: notices_of(file),
        });
    }
    // check 1: every library file is a node
    for path in by_path.keys() {
        if !nodes.iter().any(|n| &n.path == path) {
            findings.push(finding(
                1,
                format!("`{}` is in the library and the map offers it nowhere — give it an `offers` entry in {}", path, map_path),
            ));
        }
    }
    // a follows that names no gate is a plain default, not a following
    let gates: BTreeSet<String> = nodes.iter().filter_map(|n| n.gate.clone()).collect();
    for n in &mut nodes {
        if n.follows.as_ref().is_some_and(|f| !gates.contains(f)) {
            n.follows = None;
        }
    }

    // ---- edges -------------------------------------------------------------
    let is_pack = |p: &str| nodes.iter().any(|n| n.path == p && n.role == Role::Pack);
    let gated = |g: &str| -> Vec<String> {
        nodes.iter().filter(|n| n.gate.as_deref() == Some(g)).map(|n| n.path.clone()).collect()
    };
    let mut edges: Vec<Edge> = Vec::new();

    // declared, from the entries
    for o in &entries {
        for (kind, targets) in [(EdgeKind::Requires, &o.requires), (EdgeKind::Excludes, &o.excludes)] {
            for t in targets {
                if !nodes.iter().any(|n| &n.path == t) {
                    findings.push(finding(1, format!("{}: `{}` names `{}`, which the map does not offer", at_map(o.line), o.path, t)));
                    continue;
                }
                edges.push(Edge {
                    from: o.path.clone(),
                    to: t.clone(),
                    kind,
                    source: Source::Declared,
                    params: Vec::new(),
                    location: at_map(o.line),
                });
            }
        }
    }

    // gate: a pack declares the param another pack is gated on
    for n in nodes.iter().filter(|n| n.role == Role::Pack) {
        if let (Some(g), Some(by)) = (&n.gate, &n.gate_declared_in) {
            if is_pack(by) && by != &n.path {
                let line = declared[g.as_str()][0].1;
                edges.push(Edge {
                    from: n.path.clone(),
                    to: by.clone(),
                    kind: EdgeKind::Gate,
                    source: Source::Derived,
                    params: vec![g.clone()],
                    location: format!("{}:{}", by, line),
                });
            }
        }
    }

    // asks and excludes, from the questions
    for (path, f) in &by_path {
        for q in &f.questions {
            let subjects: Vec<&str> =
                if q.oneof { q.options.iter().map(|o| o.param.as_str()).collect() } else { vec![q.subject.as_str()] };
            if let Some(w) = &q.ask_when {
                for s in &subjects {
                    for child in gated(s) {
                        for parent in gated(w) {
                            if child != parent {
                                edges.push(Edge {
                                    from: child.clone(),
                                    to: parent,
                                    kind: EdgeKind::Asks,
                                    source: Source::Derived,
                                    params: vec![w.clone()],
                                    location: format!("{}:{}", path, q.line),
                                });
                            }
                        }
                    }
                }
            }
            if q.oneof {
                for (i, a) in q.options.iter().enumerate() {
                    for b in &q.options[i + 1..] {
                        for x in gated(&a.param) {
                            for y in gated(&b.param) {
                                edges.push(Edge {
                                    from: x.clone(),
                                    to: y,
                                    kind: EdgeKind::Excludes,
                                    source: Source::Derived,
                                    params: vec![a.param.clone(), b.param.clone()],
                                    location: format!("{}:{}", path, q.line),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // data: a param one pack reads and another declares — never to a pack it
    // excludes, which is an alternative to it, not a provider
    let excluded = |a: &str, b: &str, edges: &[Edge]| {
        edges.iter().any(|e| e.kind == EdgeKind::Excludes && ((e.from == a && e.to == b) || (e.from == b && e.to == a)))
    };
    let mut data: BTreeMap<(String, String), (Vec<String>, usize)> = BTreeMap::new();
    for n in nodes.iter().filter(|n| n.role == Role::Pack) {
        let file = by_path[&n.path];
        for (param, line) in doc_packs::needs_at(file) {
            for (provider, _, _) in declared.get(param.as_str()).map(|d| d.as_slice()).unwrap_or_default() {
                if *provider == n.path || !is_pack(provider) || excluded(&n.path, provider, &edges) {
                    continue;
                }
                let e = data.entry((n.path.clone(), provider.to_string())).or_insert((Vec::new(), line));
                e.0.push(param.clone());
                e.1 = e.1.min(line);
            }
        }
    }
    for ((from, to), (params, line)) in data {
        let location = format!("{}:{}", from, line);
        edges.push(Edge { from, to, kind: EdgeKind::Data, source: Source::Derived, params, location });
    }

    // one order for the file: by the consuming node, then kind, then the other end
    let index: BTreeMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (n.path.as_str(), i)).collect();
    edges.sort_by(|a, b| {
        (index[a.from.as_str()], a.kind, index[a.to.as_str()], a.source).cmp(&(index[b.from.as_str()], b.kind, index[b.to.as_str()], b.source))
    });
    let graph = PackGraph { nodes, edges };
    findings.extend(check_notices(&graph, &by_path, &declared));
    findings.extend(check(&graph));
    Ok((graph, findings))
}

/// Check 9: a notice sits on a gated pack the map offers — it is shown when that pack is
/// switched on — and its param is the notice's alone: declared by that pack only, asked
/// by no question, no gate, and read by no file, because an acknowledgement is never
/// emitted and binding it must change nothing a pack builds.
fn check_notices(g: &PackGraph, by_path: &BTreeMap<String, &File>, declared: &BTreeMap<&str, Vec<(&str, usize, &Value)>>) -> Vec<Finding> {
    let mut out = Vec::new();
    // every param a library file reads, with the first file and line that reads it
    let mut reads: BTreeMap<String, String> = BTreeMap::new();
    for (path, f) in by_path {
        let mut r = doc_packs::Refs::new();
        f.items.iter().for_each(|e| doc_packs::refs_in_entry(e, &mut r));
        for (_, v, line) in &f.params {
            doc_packs::refs_in_value(v, *line, &mut r);
        }
        for a in &f.actions {
            a.args.iter().chain(&a.execute_args).for_each(|p| doc_packs::refs_in_str(p, a.line, &mut r));
        }
        for (param, line) in r {
            reads.entry(param).or_insert_with(|| format!("{}:{}", path, line));
        }
    }
    for n in &g.nodes {
        for x in &n.notices {
            let at = format!("{}: the notice `{}`", n.path, x.param);
            if n.role != Role::Pack || n.gate.is_none() {
                out.push(finding(9, format!("{} — a notice is shown when a pack is switched on, so only a gated pack the map offers carries one", at)));
            }
            let elsewhere: Vec<String> = declared
                .get(x.param.as_str())
                .map(|d| d.iter().filter(|(p, _, _)| *p != n.path).map(|(p, l, _)| format!("{}:{}", p, l)).collect())
                .unwrap_or_default();
            if !elsewhere.is_empty() {
                out.push(finding(9, format!("{} — its param is declared again at {}; it belongs to the notice alone", at, elsewhere.join(", "))));
            }
            let asked = by_path.iter().find(|(_, f)| {
                f.questions.iter().any(|q| q.subject == x.param || q.options.iter().any(|o| o.param == x.param))
            });
            if let Some((p, _)) = asked {
                out.push(finding(9, format!("{} — {} asks its param as a question; an acknowledgement is no customer decision", at, p)));
            }
            if g.nodes.iter().any(|m| m.gate.as_deref() == Some(x.param.as_str())) {
                out.push(finding(9, format!("{} — its param is a pack's gate", at)));
            }
            if let Some(site) = reads.get(&x.param) {
                out.push(finding(9, format!("{} — {} reads its param; an acknowledgement is never emitted, so nothing may read it", at, site)));
            }
        }
    }
    out
}

/// Checks 3 to 8 over a built graph (1 and 2 are found while building it).
fn check(g: &PackGraph) -> Vec<Finding> {
    let mut out = Vec::new();
    let declared_between = |a: &str, b: &str| {
        g.edges.iter().any(|e| {
            e.source == Source::Declared
                && matches!(e.kind, EdgeKind::Requires | EdgeKind::Excludes)
                && ((e.from == a && e.to == b) || (e.from == b && e.to == a))
        })
    };

    // 3. a gate is shared only by a declared bundle: every pack on a shared gate has
    // a declared edge to another pack on it
    let mut by_gate: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for n in g.nodes.iter().filter(|n| n.role == Role::Pack) {
        if let Some(gate) = &n.gate {
            by_gate.entry(gate.as_str()).or_default().push(n.path.as_str());
        }
    }
    for (gate, packs) in &by_gate {
        if packs.len() < 2 {
            continue;
        }
        for p in packs {
            if !packs.iter().any(|q| q != p && declared_between(p, q)) {
                out.push(finding(
                    3,
                    format!(
                        "`{}` shares the gate `{}` with {} and declares no edge to them — a gate is shared only by a declared bundle (`requires` / `excludes` on the entries)",
                        p,
                        gate,
                        packs.iter().filter(|q| *q != p).map(|q| format!("`{}`", q)).collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
        }
    }

    // 4. no cycle through what a pack needs or where it is asked
    let needs: BTreeMap<&str, Vec<&str>> = {
        let mut m: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for e in g.edges.iter().filter(|e| e.kind != EdgeKind::Excludes) {
            m.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
        m
    };
    if let Some(cycle) = find_cycle(&g.nodes, &needs) {
        out.push(finding(4, format!("the graph has a cycle: {}", cycle.join(" → "))));
    }

    // 5. the provider of every data or gate edge is an ancestor of its consumer
    out.extend(provider_is_ancestor(g));

    // 7. every block a line is placed in exists in the estate satz writes
    let skeleton = template::bare_skeleton();
    let mut place: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for n in g.nodes.iter().filter(|n| n.order.is_some() && n.by_hand.is_none()) {
        let order = n.order.unwrap_or_default();
        let section = match (&n.block, n.after_scaffold) {
            (Some(b), _) => match template::insert_into_block(&skeleton, b, "// pack-graph marker", "") {
                Some(text) => text.lines().position(|l| l.trim() == "// pack-graph marker").unwrap_or_default(),
                None => {
                    out.push(finding(
                        7,
                        format!("{}: `{}` is placed in `{}`, a block the estate satz writes does not have", n.location.clone().unwrap_or_default(), n.path, b),
                    ));
                    continue;
                }
            },
            (None, true) => usize::MAX,
            (None, false) => 0,
        };
        place.insert(n.path.as_str(), (section, order));
    }

    // 6. a consumer's line comes after its provider's line — at least one provider's,
    // where several packs provide the same param or meet the same requirement
    let map_line = g.nodes.iter().find(|n| n.role == Role::Map).and_then(|m| place.get(m.path.as_str()).copied());
    let mut wants: BTreeMap<(&str, String), Vec<&str>> = BTreeMap::new();
    for e in &g.edges {
        match e.kind {
            EdgeKind::Data => {
                for p in &e.params {
                    wants.entry((e.from.as_str(), format!("reads `{}`", p))).or_default().push(e.to.as_str());
                }
            }
            EdgeKind::Gate => wants.entry((e.from.as_str(), format!("is gated on `{}`", e.params.join(", ")))).or_default().push(e.to.as_str()),
            EdgeKind::Requires => wants.entry((e.from.as_str(), "requires".to_string())).or_default().push(e.to.as_str()),
            _ => {}
        }
    }
    for ((consumer, why), providers) in &wants {
        let Some(at) = place.get(consumer) else { continue };
        let placed: Vec<(&str, (usize, usize))> = providers.iter().filter_map(|p| place.get(p).map(|q| (*p, *q))).collect();
        if !placed.is_empty() && !placed.iter().any(|(_, q)| q < at) {
            out.push(finding(
                6,
                format!(
                    "`{}` {} from {}, and its line comes before theirs — answering it yes stops the compile with `unknown param`",
                    consumer,
                    why,
                    placed.iter().map(|(p, _)| format!("`{}`", p)).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    if let Some(m) = map_line {
        for n in g.nodes.iter().filter(|n| n.gate_declared_in.as_deref() == g.nodes.iter().find(|x| x.role == Role::Map).map(|x| x.path.as_str())) {
            if place.get(n.path.as_str()).is_some_and(|at| *at < m) {
                out.push(finding(6, format!("`{}` is gated on the map's `{}`, and its line comes before the map's", n.path, n.gate.clone().unwrap_or_default())));
            }
        }
    }

    // 8. no declared edge duplicates a derived one
    for d in g.edges.iter().filter(|e| e.source == Source::Declared) {
        let dup = g.edges.iter().find(|e| {
            e.source == Source::Derived
                && match d.kind {
                    EdgeKind::Requires => {
                        matches!(e.kind, EdgeKind::Data | EdgeKind::Gate | EdgeKind::Requires) && e.from == d.from && e.to == d.to
                    }
                    EdgeKind::Excludes => {
                        e.kind == EdgeKind::Excludes && ((e.from == d.from && e.to == d.to) || (e.from == d.to && e.to == d.from))
                    }
                    _ => false,
                }
        });
        if let Some(e) = dup {
            out.push(finding(
                8,
                format!(
                    "{}: `{}` declares `{}` on `{}`, which the packs already show (a {} edge at {}) — remove the declaration",
                    d.location,
                    d.from,
                    kind(d.kind),
                    d.to,
                    kind(e.kind),
                    e.location
                ),
            ));
        }
    }
    out
}

/// A cycle in `needs`, as the path that closes it.
fn find_cycle(nodes: &[Node], needs: &BTreeMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    fn visit<'a>(
        n: &'a str,
        needs: &BTreeMap<&'a str, Vec<&'a str>>,
        state: &mut BTreeMap<&'a str, u8>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match state.get(n) {
            Some(2) => return None,
            Some(1) => {
                let from = stack.iter().position(|s| *s == n).unwrap_or_default();
                let mut cycle: Vec<String> = stack[from..].iter().map(|s| s.to_string()).collect();
                cycle.push(n.to_string());
                return Some(cycle);
            }
            _ => {}
        }
        state.insert(n, 1);
        stack.push(n);
        for m in needs.get(n).map(|v| v.as_slice()).unwrap_or_default() {
            if let Some(c) = visit(m, needs, state, stack) {
                return Some(c);
            }
        }
        stack.pop();
        state.insert(n, 2);
        None
    }
    let mut state = BTreeMap::new();
    for n in nodes {
        if let Some(c) = visit(n.path.as_str(), needs, &mut state, &mut Vec::new()) {
            return Some(c);
        }
    }
    None
}

/// An edge kind as the file spells it.
fn kind(k: EdgeKind) -> &'static str {
    match k {
        EdgeKind::Requires => "requires",
        EdgeKind::Data => "data",
        EdgeKind::Gate => "gate",
        EdgeKind::Asks => "asks",
        EdgeKind::Excludes => "excludes",
    }
}

/// Check 5: the provider of every `data` or `gate` edge is an ancestor of its
/// consumer in the interview — every question the provider's is asked under
/// (`asks`, transitively) is one the consumer's is asked under too. A consumer asked
/// where its provider cannot be would take a yes that stops the compile, since the
/// hidden provider stays off.
fn provider_is_ancestor(g: &PackGraph) -> Vec<Finding> {
    let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in g.edges.iter().filter(|e| e.kind == EdgeKind::Asks) {
        parents.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    let asked_under = |n: &str| -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![n.to_string()];
        while let Some(x) = todo.pop() {
            for p in parents.get(x.as_str()).map(|v| v.as_slice()).unwrap_or_default() {
                if seen.insert(p.to_string()) {
                    todo.push(p.to_string());
                }
            }
        }
        seen
    };
    let mut out = Vec::new();
    for e in g.edges.iter().filter(|e| matches!(e.kind, EdgeKind::Data | EdgeKind::Gate)) {
        let consumer = asked_under(&e.from);
        let missing: Vec<String> = asked_under(&e.to).into_iter().filter(|a| !consumer.contains(a) && a != &e.from).collect();
        if !missing.is_empty() {
            out.push(finding(
                5,
                format!(
                    "`{}` needs `{}` ({} edge at {}), whose question is asked only under {} — `{}`'s is asked without, so it can be answered yes while its provider cannot",
                    e.from,
                    e.to,
                    kind(e.kind),
                    e.location,
                    missing.iter().map(|m| format!("`{}`", m)).collect::<Vec<_>>().join(", "),
                    e.from
                ),
            ));
        }
    }
    out
}

/// The pack graph an estate's presets arrived with, as the compile reads it: the menu-
/// dependent checks run over `Graph`, and the other two are one finding each.
pub(crate) enum Shipped {
    /// the graph, and the presets folder it came from — the library files the pack logic
    /// reads beside it
    Graph(PackGraph, PathBuf),
    /// `<presets_dir>/pack-graph.json` is not there
    Missing(PathBuf),
    /// it is there and does not read as a graph; why
    Unreadable(String),
}

/// `<presets_dir>/pack-graph.json`: `None` when there is none, an error when it does not
/// read as a pack graph.
pub(crate) fn read(presets_dir: &Path) -> Result<Option<PackGraph>, BoxErr> {
    let path = presets_dir.join(GRAPH_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {}", path.display(), e).into()),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{}: not a pack graph this satz reads ({}) — `satz self-update`, then `satz get-presets`", path.display(), e).into())
}

/// [`read`], for the compile: never an error, so the menu never stops a compile.
pub(crate) fn shipped(presets_dir: &Path) -> Shipped {
    match read(presets_dir) {
        Ok(Some(g)) => Shipped::Graph(g, presets_dir.to_path_buf()),
        Ok(None) => Shipped::Missing(presets_dir.join(GRAPH_FILE)),
        Err(e) => Shipped::Unreadable(e.to_string()),
    }
}

/// [`read`], for a command that WRITES pack lines (`init`, `interview --create`,
/// `merge-presets`): a graph that places a pack in a block this binary's scaffold does not
/// have is refused here, before anything is written.
pub(crate) fn for_writing(presets_dir: &Path) -> Result<Option<PackGraph>, BoxErr> {
    let Some(g) = read(presets_dir)? else { return Ok(None) };
    let skeleton = template::bare_skeleton();
    for n in g.lines() {
        if let template::Place::Block(b) = template::place(n) {
            if template::insert_into_block(&skeleton, b, "// pack-graph marker", "").is_none() {
                return Err(format!("{}: {}", presets_dir.join(GRAPH_FILE).display(), template::unknown_block(&n.path, b)).into());
            }
        }
    }
    Ok(Some(g))
}

/// What a command that writes an estate says when the presets carry no graph: the file is
/// written without pack lines, and these two commands write them.
pub(crate) fn no_menu_note(presets_dir: &Path) -> String {
    format!(
        "no pack menu written: {} is not here — `satz get-presets`, then `satz merge-presets`, write the pack lines",
        presets_dir.join(GRAPH_FILE).display()
    )
}

/// Build, check and write (or with `check`, compare) `<presets_dir>/pack-graph.json`.
pub(crate) fn run(presets_dir: &Path, check: bool) -> Result<(), BoxErr> {
    let all = doc_packs::packs(presets_dir)?;
    let (graph, findings) = build(&all)?;
    if !findings.is_empty() {
        let mut lines: Vec<String> = findings.iter().map(|f| format!("check {}: {}", f.check, f.text)).collect();
        lines.sort();
        return Err(format!("pack-graph: {} finding(s) in the library — nothing written:\n  {}", lines.len(), lines.join("\n  ")).into());
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&graph)?);
    let out = presets_dir.join(GRAPH_FILE);
    let current = std::fs::read_to_string(&out).ok();
    let declared = graph.edges.iter().filter(|e| e.source == Source::Declared).count();
    let summary = format!("{} node(s), {} edge(s), {} of them declared", graph.nodes.len(), graph.edges.len(), declared);
    if current.as_deref() == Some(text.as_str()) {
        println!("pack-graph: {} current — {}", out.display(), summary);
        return Ok(());
    }
    if check {
        return Err(format!("pack-graph: {} is behind the library — run `satz pack-graph` and commit ({})", out.display(), summary).into());
    }
    crate::fsx::write(&out, &text)?;
    println!("pack-graph: wrote {} — {}", out.display(), summary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library in memory: (path under presets/, source).
    fn lib(files: &[(&str, &str)]) -> Vec<(PathBuf, File, String)> {
        files
            .iter()
            .map(|(rel, src)| {
                let f = satz_core::satz::parse(src).unwrap_or_else(|e| panic!("{}:{}: {}", rel, e.line, e.msg));
                (PathBuf::from(rel), f, src.to_string())
            })
            .collect()
    }

    /// The map of a test library: `params` declares the gates, `rest` the entries.
    fn map(params: &str, rest: &str) -> String {
        format!("pack estate_map version \"1.0\"\n\nparams {{\n{}\n}}\n\n{}\n", params, rest)
    }

    /// The checks a library fails, each once.
    fn checks(files: &[(&str, &str)]) -> Vec<u8> {
        let (_, found) = build(&lib(files)).unwrap();
        let mut n: Vec<u8> = found.iter().map(|f| f.check).collect();
        n.sort();
        n.dedup();
        n
    }

    const A: &str = "pack a version \"1.0\"\n\nparams {\n  a_value = \"x\"\n}\n";
    const B_READS_A: &str = "pack b version \"1.0\"\n\nparams {\n  b_value = \"{a_value}\"\n}\n";

    #[test]
    fn the_shipped_pack_graph_is_current() {
        let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        run(&presets, true).expect("presets/pack-graph.json is behind the library — run `satz pack-graph` and commit");
    }

    #[test]
    fn a_clean_library_derives_its_edges_and_passes() {
        let m = map(
            "  use_a = true\n  use_b = use_a",
            "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/b.satz\" {\n  when = use_b\n}\n",
        );
        let (g, found) = build(&lib(&[("estate-map.satz", &m), ("a.satz", A), ("b.satz", B_READS_A)])).unwrap();
        assert!(found.is_empty(), "{:?}", found);
        let e = g.edges.iter().find(|e| e.kind == EdgeKind::Data).expect("b reads a's param");
        assert_eq!((e.from.as_str(), e.to.as_str()), ("presets/b.satz", "presets/a.satz"));
        assert_eq!(e.params, vec!["a_value".to_string()]);
        assert_eq!(e.location, "presets/b.satz:4");
        let b = g.nodes.iter().find(|n| n.path == "presets/b.satz").unwrap();
        assert_eq!(b.follows.as_deref(), Some("use_a"), "a gate defaulting to another gate follows it");
        assert_eq!(b.gate_declared_in.as_deref(), Some("presets/estate-map.satz"));
    }

    #[test]
    fn check_1_a_pack_the_map_does_not_offer() {
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A), ("b.satz", B_READS_A)]), vec![1]);
    }

    #[test]
    fn check_2_a_pack_without_a_gate_and_a_gate_declared_twice() {
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A)]), vec![2]);
        let twice = "pack a version \"1.0\"\n\nparams {\n  use_a = true\n}\n";
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", twice)]), vec![2]);
    }

    #[test]
    fn check_3_a_gate_two_packs_share_without_a_declared_edge() {
        let c = "pack c version \"1.0\"\n";
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/c.satz\" {\n  when = use_a\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A), ("c.satz", c)]), vec![3]);
        // declared, the two are a bundle
        let m = map(
            "  use_a = true",
            "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/c.satz\" {\n  when     = use_a\n  requires = [\"presets/a.satz\"]\n}\n",
        );
        assert!(checks(&[("estate-map.satz", &m), ("a.satz", A), ("c.satz", c)]).is_empty());
    }

    #[test]
    fn check_4_two_packs_reading_each_other() {
        let a = "pack a version \"1.0\"\n\nparams {\n  a_value = \"{b_value}\"\n}\n";
        let m = map(
            "  use_a = true\n  use_b = true",
            "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/b.satz\" {\n  when = use_b\n}\n",
        );
        assert!(checks(&[("estate-map.satz", &m), ("a.satz", a), ("b.satz", B_READS_A)]).contains(&4));
    }

    #[test]
    fn check_5_a_consumer_asked_where_its_provider_is_not() {
        let m = format!(
            "{}\nquestion use_a {{\n  prompt   = \"a?\"\n  reversal = edit\n  blast    = low\n  ask_when = use_x\n}}\n",
            map(
                "  use_x = false\n  use_a = false\n  use_b = false",
                "offers \"presets/x.satz\" {\n  when = use_x\n}\n\noffers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/b.satz\" {\n  when = use_b\n}\n",
            )
        );
        let x = "pack x version \"1.0\"\n";
        assert_eq!(checks(&[("estate-map.satz", &m), ("x.satz", x), ("a.satz", A), ("b.satz", B_READS_A)]), vec![5]);
    }

    #[test]
    fn check_6_a_provider_placed_after_its_consumer() {
        // the Sentinel shape: a line in the menu reading a param of a pack inside the folder
        let m = map(
            "  use_a = true\n  use_b = true",
            "offers \"presets/b.satz\" {\n  when = use_b\n}\n\noffers \"presets/a.satz\" {\n  when  = use_a\n  block = \"google_folder.infra_folder\"\n}\n",
        );
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A), ("b.satz", B_READS_A)]), vec![6]);
    }

    #[test]
    fn check_7_a_block_the_estate_does_not_have() {
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n  when  = use_a\n  block = \"google_folder.nowhere\"\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A)]), vec![7]);
    }

    #[test]
    fn check_8_a_declared_edge_the_packs_already_show() {
        let m = map(
            "  use_a = true\n  use_b = true",
            "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/b.satz\" {\n  when     = use_b\n  requires = [\"presets/a.satz\"]\n}\n",
        );
        assert_eq!(checks(&[("estate-map.satz", &m), ("a.satz", A), ("b.satz", B_READS_A)]), vec![8]);
    }

    #[test]
    fn check_9_a_notice_on_a_gated_pack_whose_param_nothing_else_touches() {
        let a = "pack a version \"1.0\"\n\nparams {\n  a_adopted = false\n}\n\nnotice a_adopted {\n  text = \"t\"\n  run  = \"satz adopt <estate> --execute --import\"\n}\n";
        let m = map("  use_a = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n");
        let (g, found) = build(&lib(&[("estate-map.satz", &m), ("a.satz", a)])).unwrap();
        assert!(found.is_empty(), "{:?}", found);
        let node = g.nodes.iter().find(|n| n.path == "presets/a.satz").unwrap();
        assert_eq!(node.notices.len(), 1, "the graph carries the notice for every reader");
        // read by another pack, and declared again there
        let b = "pack b version \"1.0\"\n\nparams {\n  b_value = \"{a_adopted}\"\n}\n";
        let m2 = map("  use_a = true\n  use_b = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/b.satz\" {\n  when = use_b\n}\n");
        assert_eq!(checks(&[("estate-map.satz", &m2), ("a.satz", a), ("b.satz", b)]), vec![9]);
        let c = "pack c version \"1.0\"\n\nparams {\n  a_adopted = false\n}\n";
        let m3 = map("  use_a = true\n  use_c = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n\noffers \"presets/c.satz\" {\n  when = use_c\n}\n");
        assert!(checks(&[("estate-map.satz", &m3), ("a.satz", a), ("c.satz", c)]).contains(&9));
        // on the day-0 pack, which nothing switches on
        let core = "pack estate_core version \"1.0\"\n\nparams {\n  x_adopted = false\n}\n\nnotice x_adopted {\n  text = \"t\"\n  run  = \"r\"\n}\n";
        assert_eq!(checks(&[("estate-map.satz", &map("  use_a = true", "offers \"presets/a.satz\" {\n  when = use_a\n}\n")), ("a.satz", A), ("estate-core.satz", core)]), vec![9]);
    }
}
