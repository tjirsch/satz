//! The shape of a param no file gives a value, read from where it is read (ADR 0108).
//!
//! A param every pack declares `none` carries no value to take a shape from, and the
//! answer to its question still needs one: one address for a set of addresses is a list of
//! one, never a string. The places a pack reads the param say what it is — an attribute
//! whose provider type is a set, an `each` over it, a `when` on it, a text it is
//! interpolated into.
//!
//! The walk that finds those places is the questions walk, and it stays schema-free: it
//! records each read with the path of keys it stands at (`google_x` › `label` › `spec` ›
//! `rules` › element › `parameters`). Only the shapes of the params no file gives a value
//! are then read off those paths, through the provider schema the caller passes, so a
//! command that has no such param, or none read inside a resource, needs no schema.

use super::{perr, PipelineError, TypeResolver};
use crate::satz::{self, Cond, Entry, File, Key, StrPart, Value};
use std::collections::{BTreeMap, BTreeSet};

/// An attribute's type as the provider schema states it.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrType {
    String,
    Number,
    Bool,
    /// a list, a set or a tuple: its elements' type
    List(Box<AttrType>),
    /// a map: its values' type
    Map(Box<AttrType>),
    Object(BTreeMap<String, AttrType>),
    /// `dynamic`: any value, and no verdict
    Dynamic,
}

/// The shape of a param, in the words `satz questions` reports it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    String,
    Number,
    Bool,
    List,
    Map,
}

impl Shape {
    pub fn as_str(self) -> &'static str {
        match self {
            Shape::String => "string",
            Shape::Number => "number",
            Shape::Bool => "bool",
            Shape::List => "list",
            Shape::Map => "map",
        }
    }

    fn of(t: &AttrType) -> Option<Shape> {
        Some(match t {
            AttrType::String => Shape::String,
            AttrType::Number => Shape::Number,
            AttrType::Bool => Shape::Bool,
            AttrType::List(_) => Shape::List,
            AttrType::Map(_) | AttrType::Object(_) => Shape::Map,
            AttrType::Dynamic => return None,
        })
    }
}

/// One step of the path a read stands at, from the top of the estate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seg {
    /// `key { … }` or `key name { … }`; `name` is the body's own `name = "…"` where it is a
    /// plain string — the constraint an organization policy sets
    Map { key: String, named: bool, name: Option<String> },
    /// `key = …`
    Attr(String),
    /// an element of a list value
    Elem,
    /// `key = …` inside an object value
    Key(String),
    /// the body an `each` writes once per entry
    EachBody,
}

#[derive(Debug, Clone)]
enum Site {
    /// the whole value, or a part of it, at this path
    At(Vec<Seg>),
    /// the list an `each` expands
    Each,
    /// a condition: `when`, `use … when`, a question's `ask_when`
    Cond(&'static str),
    /// interpolated into a string
    Text,
}

#[derive(Debug, Clone)]
struct Read {
    param: String,
    file: String,
    line: usize,
    site: Site,
}

/// Every read of every param the questions walk passed, and which params a file other than
/// the estate gives a value.
#[derive(Debug, Default)]
pub struct ParamUses {
    reads: Vec<Read>,
    /// `x = p` in a `params` block: what reads `x` reads `p`
    aliases: Vec<(String, String)>,
    declared_none: BTreeSet<String>,
    valued: BTreeSet<String>,
    /// The named use being walked, with the params it owns: a read of one of them is a read
    /// of `<name>.<param>`, which is the use's own and no flat param.
    named: Option<(String, BTreeSet<String>)>,
}

/// One read whose shape is known, for the message that names it.
struct Evidence {
    shape: Option<Shape>,
    file: String,
    line: usize,
    what: String,
}

impl ParamUses {
    /// Enters a named use (`Some`) or leaves it (`None`).
    pub(crate) fn scope(&mut self, named: Option<(String, BTreeSet<String>)>) {
        self.named = named;
    }

    /// A param's name as the shapes are kept: a param a named use owns is `<name>.<param>`.
    fn name(&self, p: &str) -> String {
        match &self.named {
            Some((n, owned)) if owned.contains(p) => super::named::bound_name(n, p),
            _ => p.to_string(),
        }
    }

    /// The params whose shape is read from their uses: declared `none` and given a value by
    /// no file but the estate itself, whose binding is an answer and no default.
    pub fn without_value(&self) -> BTreeSet<String> {
        self.declared_none.difference(&self.valued).cloned().collect()
    }

    /// Whether a shape this walk derives is read off a resource — whether the caller needs
    /// the provider schema for [`ParamUses::shapes`].
    pub fn needs_types(&self) -> bool {
        self.without_value().iter().any(|p| self.reads_of(p).iter().any(|r| matches!(r.site, Site::At(_))))
    }

    /// Every read of `p`, and of every param declared as `p` (`x = p`).
    fn reads_of(&self, p: &str) -> Vec<&Read> {
        let mut names = vec![p.to_string()];
        let mut i = 0;
        while i < names.len() {
            for (alias, target) in &self.aliases {
                if *target == names[i] && !names.contains(alias) {
                    names.push(alias.clone());
                }
            }
            i += 1;
        }
        self.reads.iter().filter(|r| names.contains(&r.param)).collect()
    }

    /// The shape of every param no file gives a value that is read where a shape is known.
    /// Two reads that disagree — a list in one place, a string in another — are refused,
    /// naming the param and both places. A param read nowhere with a shape has none.
    pub fn shapes(&self, types: &dyn TypeResolver) -> Result<BTreeMap<String, Shape>, PipelineError> {
        let mut out = BTreeMap::new();
        for p in self.without_value() {
            let mut first: Option<Evidence> = None;
            // the scalar shapes agree with a string and with each other, except a number
            // with a bool; the most specific is the shape
            let mut shape: Option<Shape> = None;
            for r in self.reads_of(&p) {
                let Some(e) = evidence(r, types) else { continue };
                match (&first, e.shape) {
                    (None, s) => {
                        shape = s.or(Some(Shape::String));
                        first = Some(e);
                    }
                    (Some(f), s) => {
                        let now = s.unwrap_or(Shape::String);
                        let had = shape.expect("set with the first read");
                        let merged = match (had, now) {
                            (a, b) if a == b => Some(a),
                            (Shape::String, b @ (Shape::Number | Shape::Bool)) => Some(b),
                            (a @ (Shape::Number | Shape::Bool), Shape::String) => Some(a),
                            _ => None,
                        };
                        match merged {
                            Some(m) => shape = Some(m),
                            None => {
                                return perr(
                                    &e.file,
                                    e.line,
                                    format!(
                                        "param `{}` is read as {} and as {} — {}:{} ({}) and {}:{} ({}). A param no file gives a value takes its shape from where it is read, and these two places disagree: read it as one shape in both",
                                        p,
                                        article(f.shape.unwrap_or(Shape::String)),
                                        article(now),
                                        f.file,
                                        f.line,
                                        f.what,
                                        e.file,
                                        e.line,
                                        e.what
                                    ),
                                )
                            }
                        }
                    }
                }
            }
            if let Some(s) = shape {
                out.insert(p, s);
            }
        }
        Ok(out)
    }

    /// A file's `params` block: which params it gives a value, which it declares `none`,
    /// which it declares as another (`x = p`), and the params its defaults interpolate.
    pub(crate) fn note_params(&mut self, file: &File, file_name: &str, is_estate: bool) {
        for (name, v, line) in &file.params {
            if satz::contribution_target(name).is_some() {
                continue;
            }
            match v {
                Value::None => {
                    self.declared_none.insert(self.name(name));
                }
                _ if is_estate => {}
                _ => {
                    self.valued.insert(self.name(name));
                }
            }
            match v {
                Value::Ref(p) => match local(file, p) {
                    Some(Value::Ref(q)) => self.aliases.push((self.name(name), self.name(q))),
                    Some(other) => self.scan_value(other, None, file, file_name, *line, 1),
                    None => self.aliases.push((self.name(name), self.name(p))),
                },
                other => self.scan_value(other, None, file, file_name, *line, 0),
            }
        }
        for q in &file.questions {
            if let Some(gate) = &q.ask_when {
                self.read(gate, file_name, q.line, Site::Cond("a question's `ask_when`"));
            }
        }
    }

    /// `use … when <p>`
    pub(crate) fn note_use_gate(&mut self, p: &str, file_name: &str, line: usize) {
        self.read(p, file_name, line, Site::Cond("a `use … when`"));
    }

    /// One entry of a body at `segs`, without descending into a map's body — the walk does
    /// that, with the map's step pushed.
    pub(crate) fn note_entry(&mut self, e: &Entry, segs: &[Seg], file: &File, file_name: &str) {
        match e {
            Entry::Attr { key, value, line } => {
                self.note_key(key, file_name, *line);
                let mut at = segs.to_vec();
                at.push(Seg::Attr(key_text(key)));
                self.scan_value(value, Some(at), file, file_name, *line, 0);
            }
            Entry::Map { key, name, line, .. } => {
                self.note_key(key, file_name, *line);
                if let Some(n) = name {
                    self.note_key(n, file_name, *line);
                }
            }
            Entry::Each { list, line, .. } => match local(file, list) {
                Some(Value::Ref(q)) => self.read(&q.clone(), file_name, *line, Site::Each),
                _ => self.read(list, file_name, *line, Site::Each),
            },
            Entry::When { cond, line, .. } => self.note_cond(cond, file_name, *line),
            Entry::Use { .. } => {}
        }
    }

    fn note_cond(&mut self, cond: &Cond, file_name: &str, line: usize) {
        if !cond.name.starts_with("each.") {
            self.read(&cond.name, file_name, line, Site::Cond("a `when`"));
        }
    }

    fn note_key(&mut self, key: &Key, file_name: &str, line: usize) {
        if let Key::Str(parts) = key {
            self.note_text(parts, file_name, line);
        }
    }

    fn note_text(&mut self, parts: &[StrPart], file_name: &str, line: usize) {
        for part in parts {
            if let StrPart::Param(p) = part {
                if !p.starts_with("each.") {
                    self.read(p, file_name, line, Site::Text);
                }
            }
        }
    }

    /// A value at `at` — `None` where its position has no type (a param's default). A
    /// local value read by its bare name is read where it stands.
    fn scan_value(&mut self, v: &Value, at: Option<Vec<Seg>>, file: &File, file_name: &str, line: usize, depth: usize) {
        match v {
            Value::Ref(name) if name.starts_with("each.") => {}
            Value::Ref(name) => match local(file, name) {
                Some(lv) if depth < 8 => {
                    let lv = lv.clone();
                    self.scan_value(&lv, at, file, file_name, line, depth + 1)
                }
                Some(_) => {}
                None => {
                    if let Some(at) = at {
                        self.read(name, file_name, line, Site::At(at));
                    }
                }
            },
            Value::Str(parts) => {
                for part in parts {
                    if let StrPart::Param(p) = part {
                        match local(file, p) {
                            Some(lv) if depth < 8 => {
                                let lv = lv.clone();
                                self.scan_value(&lv, None, file, file_name, line, depth + 1)
                            }
                            Some(_) => {}
                            None if p.starts_with("each.") => {}
                            None => self.read(p, file_name, line, Site::Text),
                        }
                    }
                }
            }
            Value::List(items) => {
                for i in items {
                    let at = at.clone().map(|mut a| {
                        a.push(Seg::Elem);
                        a
                    });
                    self.scan_value(i, at, file, file_name, line, depth);
                }
            }
            Value::Obj(entries) => {
                for e in entries {
                    match e {
                        Entry::Attr { key, value, line } => {
                            self.note_key(key, file_name, *line);
                            let at = at.clone().map(|mut a| {
                                a.push(Seg::Key(key_text(key)));
                                a
                            });
                            self.scan_value(value, at, file, file_name, *line, depth);
                        }
                        Entry::When { cond, line, .. } => self.note_cond(cond, file_name, *line),
                        _ => {}
                    }
                }
            }
            Value::Num(_) | Value::Bool(_) | Value::None => {}
        }
    }

    fn read(&mut self, param: &str, file_name: &str, line: usize, site: Site) {
        let param = self.name(param);
        self.reads.push(Read { param, file: file_name.to_string(), line, site });
    }
}

/// The step a map's body is walked under: its key, and the plain `name = "…"` it carries.
pub(crate) fn map_seg(key: &Key, name: &Option<Key>, body: &[Entry]) -> Seg {
    let plain = body.iter().find_map(|e| match e {
        Entry::Attr { key: Key::Ident(k), value: Value::Str(parts), .. } if k == "name" => {
            parts.iter().map(|p| match p {
                StrPart::Lit(s) => Some(s.as_str()),
                StrPart::Param(_) => None,
            }).collect::<Option<String>>()
        }
        _ => None,
    });
    Seg::Map { key: key_text(key), named: name.is_some(), name: plain }
}

fn key_text(k: &Key) -> String {
    match k {
        Key::Ident(s) => s.clone(),
        Key::Str(parts) => parts
            .iter()
            .map(|p| match p {
                StrPart::Lit(s) => s.clone(),
                StrPart::Param(n) => format!("{{{}}}", n),
            })
            .collect(),
    }
}

fn local<'f>(file: &'f File, name: &str) -> Option<&'f Value> {
    file.locals.iter().find(|(n, _, _)| n == name).map(|(_, v, _)| v)
}

fn article(s: Shape) -> String {
    match s {
        Shape::List => "a list".into(),
        Shape::Map => "a map".into(),
        Shape::Bool => "a bool".into(),
        Shape::Number => "a number".into(),
        Shape::String => "a string".into(),
    }
}

/// What one read says about its param's shape, or `None` where it says nothing.
fn evidence(r: &Read, types: &dyn TypeResolver) -> Option<Evidence> {
    let (shape, what) = match &r.site {
        Site::Each => (Some(Shape::List), "the list an `each` expands".to_string()),
        Site::Cond(what) => (Some(Shape::Bool), what.to_string()),
        Site::Text => (None, "inside a string".to_string()),
        Site::At(segs) => {
            let (t, what) = type_at(segs, types)?;
            (Some(Shape::of(&t)?), what)
        }
    };
    Some(Evidence { shape, file: r.file.clone(), line: r.line, what })
}

/// Where a path stands, read step by step against the schema.
enum State {
    Top,
    /// inside a resource type map: its entries are labelled bodies
    Type(String),
    /// inside a resource body, at a block path
    Body { tf_type: String, path: Vec<String>, constraint: Option<String> },
    /// a block written as a value (`rules = [{ … }]`)
    Block { tf_type: String, path: Vec<String>, constraint: Option<String> },
    /// an organization policy rule's `parameters`, JSON the constraint defines
    Parameters(Option<String>),
    Typed(AttrType),
}

/// The type of the attribute a path ends in, with the words that name it — `None` where
/// the schema has no verdict: an unknown type, a key it does not name, a grant's roles, a
/// configuration block.
fn type_at(segs: &[Seg], types: &dyn TypeResolver) -> Option<(AttrType, String)> {
    let mut state = State::Top;
    let mut named: Vec<String> = Vec::new();
    let mut tf: Option<String> = None;
    for seg in segs {
        state = match (state, seg) {
            (State::Top | State::Body { .. }, Seg::Map { key, named: is_named, name }) if types.resolve(key).is_some() => {
                tf = Some(key.clone());
                named.clear();
                if *is_named {
                    State::Body { tf_type: key.clone(), path: Vec::new(), constraint: name.clone() }
                } else {
                    State::Type(key.clone())
                }
            }
            (State::Top, _) => State::Top,
            (State::Type(t), Seg::Map { name, .. }) => State::Body { tf_type: t, path: Vec::new(), constraint: name.clone() },
            (State::Type(t), Seg::EachBody) => State::Body { tf_type: t, path: Vec::new(), constraint: None },
            (State::Type(_), _) => return None,
            (State::Body { tf_type, mut path, constraint }, Seg::Map { key, .. }) => {
                path.push(key.clone());
                named.push(key.clone());
                State::Body { tf_type, path, constraint }
            }
            (State::Body { tf_type, path, constraint } | State::Block { tf_type, path, constraint }, Seg::Attr(k) | Seg::Key(k)) => {
                named.push(k.clone());
                attr_state(types, tf_type, path, constraint, k)?
            }
            (s @ State::Block { .. }, Seg::Elem) => s,
            (State::Parameters(Some(c)), Seg::Key(k)) => {
                named.push(k.clone());
                State::Typed(types.constraint_parameter(&c, k)?)
            }
            (State::Typed(AttrType::List(t)), Seg::Elem) => State::Typed(*t),
            (State::Typed(AttrType::Map(t)), Seg::Key(k)) => {
                named.push(k.clone());
                State::Typed(*t)
            }
            (State::Typed(AttrType::Object(mut fields)), Seg::Key(k)) => {
                named.push(k.clone());
                State::Typed(fields.remove(k)?)
            }
            _ => return None,
        };
    }
    match state {
        State::Typed(t) => Some((t, format!("`{}` of `{}`", named.join("."), tf.unwrap_or_default()))),
        _ => None,
    }
}

fn attr_state(types: &dyn TypeResolver, tf_type: String, path: Vec<String>, constraint: Option<String>, k: &str) -> Option<State> {
    if tf_type == "google_org_policy_policy" && k == "parameters" && path.last().map(String::as_str) == Some("rules") {
        return Some(State::Parameters(constraint));
    }
    let borrowed: Vec<&str> = path.iter().map(String::as_str).collect();
    if let Some(t) = types.attribute_type(&tf_type, &borrowed, k) {
        return Some(State::Typed(t));
    }
    let keys = types.body_keys(&tf_type, &borrowed)?;
    if keys.blocks.contains(k) {
        let mut path = path;
        path.push(k.to_string());
        return Some(State::Block { tf_type, path, constraint });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{AttrType, Shape};
    use crate::pipeline::{estate_questions_and_uses, type_facts, BodyKeys, ResolvedType, TypeResolver};
    use std::collections::BTreeMap;

    /// One resource type with a set, a string and a number; one with a rule block whose
    /// `parameters` a constraint defines.
    struct Types;
    impl TypeResolver for Types {
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            matches!(key, "google_thing" | "google_org_policy_policy").then(|| {
                let (class, scope) = type_facts(key);
                ResolvedType { tf_type: key.to_string(), class, scope }
            })
        }
        fn body_keys(&self, tf_type: &str, path: &[&str]) -> Option<BodyKeys> {
            let mut k = BodyKeys::default();
            match (tf_type, path) {
                ("google_org_policy_policy", []) => {
                    k.attributes.insert("name".into());
                    k.blocks.insert("spec".into());
                }
                ("google_org_policy_policy", ["spec"]) => {
                    k.blocks.insert("rules".into());
                }
                _ => return None,
            }
            Some(k)
        }
        fn attribute_type(&self, tf_type: &str, path: &[&str], key: &str) -> Option<AttrType> {
            match (tf_type, path, key) {
                ("google_thing", [], "emails") => Some(AttrType::List(Box::new(AttrType::String))),
                ("google_thing", [], "name") => Some(AttrType::String),
                ("google_thing", [], "days") => Some(AttrType::Number),
                ("google_thing", [], "labels") => Some(AttrType::Map(Box::new(AttrType::String))),
                ("google_org_policy_policy", [], "name") => Some(AttrType::String),
                _ => None,
            }
        }
        fn constraint_parameter(&self, constraint: &str, parameter: &str) -> Option<AttrType> {
            (constraint == "iam.managed.c" && parameter == "allowed").then(|| AttrType::List(Box::new(AttrType::String)))
        }
    }

    const PACK_HEAD: &str = "pack p version \"1\"\n";

    fn shapes(pack: &str) -> Result<BTreeMap<String, Shape>, String> {
        let pack = format!("{}{}", PACK_HEAD, pack);
        let load = move |p: &str| if p == "p.satz" { Ok(pack.clone()) } else { Err(format!("no {}", p)) };
        let (_, _, _, uses) = estate_questions_and_uses("e.satz", "estate e\n\nuse \"p.satz\"\n", &load).map_err(|e| e.msg)?;
        uses.shapes(&Types).map_err(|e| format!("{}:{}: {}", e.file, e.line, e.msg))
    }

    #[test]
    fn an_attribute_gives_its_provider_type() {
        let s = shapes("params {\n  to = none\n  n = none\n  d = none\n  l = none\n}\ngoogle_thing {\n  a {\n    emails = to\n    name   = n\n    days   = d\n    labels = l\n  }\n}\n").unwrap();
        assert_eq!(s.get("to"), Some(&Shape::List));
        assert_eq!(s.get("n"), Some(&Shape::String));
        assert_eq!(s.get("d"), Some(&Shape::Number));
        assert_eq!(s.get("l"), Some(&Shape::Map), "a map or an object is a map");
    }

    #[test]
    fn an_element_of_a_list_attribute_is_its_element_type() {
        let s = shapes("params {\n  one = none\n}\ngoogle_thing {\n  a {\n    emails = [one, \"b@example.com\"]\n  }\n}\n").unwrap();
        assert_eq!(s.get("one"), Some(&Shape::String));
    }

    #[test]
    fn a_constraint_parameter_gives_the_type_the_constraint_defines() {
        let s = shapes("params {\n  allowed = none\n}\ngoogle_org_policy_policy {\n  \"c\" {\n    name = \"iam.managed.c\"\n    spec {\n      rules = [\n        {\n          parameters = {\n            allowed = allowed\n          }\n        },\n      ]\n    }\n  }\n}\n").unwrap();
        assert_eq!(s.get("allowed"), Some(&Shape::List));
    }

    #[test]
    fn each_when_and_a_string_give_a_list_a_bool_and_a_string() {
        let s = shapes("params {\n  items = none\n  on = none\n  prefix = none\n}\ngoogle_thing {\n  each items by key {\n    name = \"{prefix}-{each.key}\"\n  }\n  b when on {\n    name = \"b\"\n  }\n}\n").unwrap();
        assert_eq!(s.get("items"), Some(&Shape::List));
        assert_eq!(s.get("on"), Some(&Shape::Bool));
        assert_eq!(s.get("prefix"), Some(&Shape::String));
    }

    #[test]
    fn a_param_read_through_another_or_a_local_value_has_its_shape() {
        let s = shapes("params {\n  to = none\n  also = to\n}\nlocals {\n  via = to\n}\ngoogle_thing {\n  a {\n    emails = also\n  }\n  b {\n    emails = via\n  }\n}\n").unwrap();
        assert_eq!(s.get("to"), Some(&Shape::List));
    }

    #[test]
    fn a_string_and_a_number_agree_and_the_number_is_the_shape() {
        let s = shapes("params {\n  d = none\n}\ngoogle_thing {\n  a {\n    days = d\n    name = \"keep-{d}\"\n  }\n}\n").unwrap();
        assert_eq!(s.get("d"), Some(&Shape::Number));
    }

    #[test]
    fn two_places_that_disagree_are_refused_naming_both() {
        let e = shapes("params {\n  to = none\n}\ngoogle_thing {\n  a {\n    emails = to\n  }\n  b {\n    name = \"{to}\"\n  }\n}\n").unwrap_err();
        assert!(e.contains("param `to` is read as a list and as a string"), "{}", e);
        assert!(e.contains("`emails` of `google_thing`") && e.contains("inside a string"), "{}", e);
        assert!(e.contains("p.satz:7") && e.contains("p.satz:10"), "both places, by line: {}", e);
    }

    #[test]
    fn a_param_read_nowhere_or_given_a_value_has_no_shape_from_use() {
        let s = shapes("params {\n  unused = none\n  given = []\n}\ngoogle_thing {\n  a {\n    name = given\n  }\n}\n").unwrap();
        assert_eq!(s.get("unused"), None, "read nowhere: no shape");
        assert_eq!(s.get("given"), None, "a default gives the shape, not its use");
    }

    #[test]
    fn the_estate_binding_is_an_answer_and_the_shape_still_comes_from_use() {
        let pack = format!("{}params {{\n  to = none\n}}\ngoogle_thing {{\n  a {{\n    emails = to\n  }}\n}}\n", PACK_HEAD);
        let load = move |p: &str| if p == "p.satz" { Ok(pack.clone()) } else { Err(format!("no {}", p)) };
        let (_, _, _, uses) = estate_questions_and_uses("e.satz", "estate e\n\nparams {\n  to = \"a@example.com\"\n}\n\nuse \"p.satz\"\n", &load).unwrap();
        assert_eq!(uses.shapes(&Types).unwrap().get("to"), Some(&Shape::List));
    }

    #[test]
    fn the_transpile_refuses_two_places_that_disagree() {
        let pack = format!("{}params {{\n  to = none\n}}\nquestion to {{ prompt = \"To\" reversal = edit blast = none }}\ngoogle_thing {{\n  a {{\n    emails = to\n  }}\n  b {{\n    name = \"{{to}}\"\n  }}\n}}\n", PACK_HEAD);
        let load = move |p: &str| if p == "p.satz" { Ok(pack.clone()) } else { Err(format!("no {}", p)) };
        let e = crate::pipeline::compile_estate("e.satz", "estate e\n\nuse \"p.satz\"\n", &Types, &load).err().expect("refused");
        assert!(e.msg.contains("is read as a list and as a string"), "{}", e.msg);
    }
}
