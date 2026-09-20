//! The pack graph: which packs the library offers, which gate switches each one on,
//! and which pack needs which other one — as `satz pack-graph` writes it to
//! `presets/pack-graph.json` and every consumer reads it.
//!
//! The types live here, in the core, so that satz and satz-studio deserialize the
//! same structs from the one shipped file and neither derives the graph itself.
//! Building it — reading the map's `offers` entries and deriving the edges from the
//! packs — is `satz pack-graph`'s job, not this module's.

use serde::{Deserialize, Serialize};

/// The whole library as a graph: every pack file a node, in adoption order for the
/// offered packs, and the edges between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl PackGraph {
    /// The packs whose line satz writes into an estate, in adoption order: every
    /// offered node except those whose line is written by hand.
    pub fn lines(&self) -> Vec<&Node> {
        let mut out: Vec<&Node> = self.nodes.iter().filter(|n| n.order.is_some() && n.by_hand.is_none()).collect();
        out.sort_by_key(|n| n.order);
        out
    }

    /// The nodes an `excludes` edge joins to `path`, in either direction.
    pub fn excluded_by(&self, path: &str) -> Vec<&Node> {
        self.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Excludes)
            .filter_map(|e| {
                if e.from == path {
                    Some(e.to.as_str())
                } else if e.to == path {
                    Some(e.from.as_str())
                } else {
                    None
                }
            })
            .filter_map(|p| self.nodes.iter().find(|n| n.path == p))
            .collect()
    }
}

/// What a library file is to an estate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// `estate-core.satz`: the day-0 params, the one pack every estate starts with
    Core,
    /// `estate-map.satz`: the choices and the `offers` entries
    Map,
    /// a pack the map offers
    Pack,
}

/// One library file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// as a `use` line names it: `presets/…`
    pub path: String,
    /// the file's own `pack … version`
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
    pub role: Role,
    /// the param the pack's line is gated on
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gate: Option<String>,
    /// the library file that declares the gate
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gate_declared_in: Option<String>,
    /// the gate this node's gate defaults to, by reference (`use_sentinel_auditlogs =
    /// use_sentinel`): answering the other one answers this one unless it is bound
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub follows: Option<String>,
    /// the adoption order: the position of the node's `offers` entry in the map
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub order: Option<usize>,
    /// the phase comment that opens this node's group of lines
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub phase: Option<String>,
    /// the block of the estate the line is written inside
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub block: Option<String>,
    /// the line goes after the scaffold rather than in the menu above it
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub after_scaffold: bool,
    /// the line is written by hand, never by satz; why
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub by_hand: Option<String>,
    /// where the node's `offers` entry is: `presets/estate-map.satz:<line>` (`at` in
    /// the file)
    #[serde(rename = "at", skip_serializing_if = "Option::is_none", default)]
    pub location: Option<String>,
    /// what the pack asks to be run once it is switched on (its `notice` statements)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub notices: Vec<Notice>,
}

/// One `notice` of a pack: shown when the pack is switched on, open until the estate
/// binds `param` true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub param: String,
    pub text: String,
    pub run: String,
    /// `error` · `warning` · `info`: what a command that writes to the organisation
    /// does while the notice is open — refuse, print and go on, nothing
    pub severity: String,
}

/// What an edge says about its two ends. `from` is always the node that has the
/// need — the consumer, the child, the one asked later — and `to` the other one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// `from` needs `to` in a way its params do not show (declared only). Several
    /// `requires` edges from one node to packs that exclude one another are one
    /// requirement: any one of them meets it.
    Requires,
    /// `from` reads a param `to` declares (`params` names them); the compile stops
    /// with `unknown param` while `to` is off, unless the estate binds the reading
    /// param itself. A param several packs declare gives one edge to each; packs
    /// that exclude one another are alternatives.
    Data,
    /// `to` declares the param `from` is gated on — the CIS extensions and the
    /// baseline
    Gate,
    /// `from`'s question is asked only while `to`'s gate holds (`ask_when`):
    /// question visibility, nothing more
    Asks,
    /// the two never go in together: options of one `question oneof`, or a dry-run
    /// twin and its enforcing pack
    Excludes,
}

/// Whether the map declared an edge or `satz pack-graph` derived it from the packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Declared,
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub source: Source,
    /// the params that make the edge: those read (`data`), the gate (`gate`), the
    /// `ask_when` param (`asks`); empty for a declared edge
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub params: Vec<String>,
    /// where the edge is written or read: `presets/<file>:<line>` (`at` in the file)
    #[serde(rename = "at")]
    pub location: String,
}
