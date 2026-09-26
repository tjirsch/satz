//! An entry is judged by where it stands (ADR 0038).
//!
//! The walk has four positions, and each takes its own kinds of entry. There is no
//! declared kind of file: a `use`d file is judged by whether its entries fit the
//! position its `use` stands in, which is the same judgment a hand-written entry gets.
//! What fits nowhere — it is neither Satz at that position nor anything the provider
//! has there — is an error naming the line, never a label, a folder name or an
//! attribute made up to have somewhere to put it.
//!
//! | position | what stands there |
//! |---|---|
//! | the top level of a file | statements, resource type maps, `use` |
//! | the body of a folder or a project | the node's attributes, resource type maps |
//! | `google_folder { … }`, `google_project { … }` | named nodes; in the folder map, `use` of a file of named nodes |
//! | `google_x { … }` | labelled bodies (members, in a grant map); `use` of a file of those |
//!
//! A folder's or a project's body holds the estate's own resources, and a pack is used at
//! the top level of the estate (ADR 0046): a `use` there is refused.
//!
//! A statement is written at the top level of a file and nowhere else. The statements of
//! a `use`d file go where `STATEMENTS` says, whatever the position of the `use`.

use super::{unknown_type_msg, PipelineError, TypeResolver};
use crate::satz::{self, Entry, Key, StrPart};

/// Where an entry stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Position<'a> {
    /// The top level of a file: the estate's own, and a `use`d file's when its `use`
    /// stands at the top level or in a node's body.
    File,
    /// The body of one folder or one project, with the node's label — the refusal of a
    /// `use` there names the folder to bind a pack's folder param to.
    NodeBody { node: &'static str, label: &'a str },
    /// `google_folder { … }` / `google_project { … }`: every key names a node.
    NodeMap { node: &'static str },
    /// `google_x { … }`: every key is a label, or a member where the type is a grant.
    ResourceMap { tf_type: &'a str, grant: bool },
}

impl Position<'_> {
    /// The position as the operator wrote it.
    fn written(&self) -> String {
        match self {
            Position::File => "the top level of a file".to_string(),
            Position::NodeBody { node, .. } => format!("the body of a `{}`", node),
            Position::NodeMap { node } => format!("`{} {{ … }}`", node),
            Position::ResourceMap { tf_type, .. } => format!("`{} {{ … }}`", tf_type),
        }
    }

    /// What an identifier key `k` is read as here.
    fn reads(&self, k: &str) -> String {
        match self {
            Position::File => format!("a resource type `{}`", k),
            Position::NodeBody { node, .. } => format!("an attribute block `{}` of the `{}`, which the provider does not have", k, node),
            Position::NodeMap { node } => format!("a {} named `{}`", node.trim_start_matches("google_"), k),
            Position::ResourceMap { tf_type, .. } => format!("a resource `{}.{}`", tf_type, k),
        }
    }

    /// Where a `use` in this position stands, and what a file used there holds. A `use`
    /// never stands in a node's body, so that position never reaches this.
    fn stands_and_takes(&self) -> (String, String) {
        match self {
            Position::File | Position::NodeBody { .. } => ("at the top level".to_string(), "resource type maps".to_string()),
            Position::NodeMap { node } => (format!("inside {}", self.written()), format!("named {}s", node.trim_start_matches("google_"))),
            Position::ResourceMap { tf_type, .. } => (format!("inside {}", self.written()), format!("labelled `{}` bodies", tf_type)),
        }
    }
}

/// What happens to a statement of a `use`d file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Used {
    /// It reaches the estate from any position: `enter_use` absorbs it before the walk
    /// looks at where the `use` stands.
    Absorbed,
    /// It names the file and goes nowhere.
    Header,
    /// It is an entry, judged with the rest.
    Entry,
    /// Only the estate's own is read, so a `use`d file's would be dropped.
    EstateOnly,
}

/// One row per statement of the language: what it does at the top level of a file — the
/// phrase completes "where it …" — and what happens to it in a `use`d file.
///
/// `every_statement_has_a_row_and_is_found_in_a_file` holds this against
/// `satz::STATEMENT_KEYWORDS`, which is derived from the parser's own dispatch, so a new
/// statement cannot reach a release without a decision here.
pub(super) const STATEMENTS: &[(&str, &str, Used)] = &[
    ("action", "joins the estate's actions", Used::Absorbed),
    ("claim", "goes to the compliance plane", Used::Absorbed),
    ("estate", "names the file", Used::Header),
    ("export", "joins the estate's interface, the outputs customer HCL reads", Used::Absorbed),
    ("hcl", "passes through to main.tf beside the resources", Used::Absorbed),
    ("interface", "joins one project's interface, interfaces/<name>/, or — as a header — names a generated interface file", Used::Absorbed),
    ("notice", "joins the estate's notices", Used::Absorbed),
    ("offers", "goes to the pack graph", Used::Absorbed),
    ("pack", "names the file", Used::Header),
    ("params", "goes to the estate's parameter namespace", Used::Absorbed),
    ("question", "goes to the interview", Used::Absorbed),
    ("suppress", "removes a resource from the estate's fold", Used::EstateOnly),
    ("use", "pulls in another file", Used::Entry),
];

fn statement(k: &str) -> Option<&'static (&'static str, &'static str, Used)> {
    STATEMENTS.iter().find(|(kw, _, _)| *kw == k)
}

/// The statements a parsed file carries, by keyword, with the line of the first where
/// the parser keeps one.
pub(super) fn statements_in(file: &satz::File) -> Vec<(&'static str, Option<usize>)> {
    let header = |pack: bool| (file.estate.is_some() && file.is_pack == pack).then_some(None);
    let found: [(&'static str, Option<Option<usize>>); 13] = [
        ("action", file.actions.first().map(|a| Some(a.line))),
        ("claim", file.claims.first().map(|c| Some(c.line))),
        ("estate", header(false)),
        ("export", file.exports.first().map(|x| Some(x.line))),
        ("hcl", file.hcl_blocks.first().map(|h| Some(h.line))),
        ("interface", file.interfaces.first().map(|i| Some(i.line)).or(file.interface_file.as_ref().map(|i| Some(i.line)))),
        ("notice", file.notices.first().map(|n| Some(n.line))),
        ("offers", file.offers.first().map(|o| Some(o.line))),
        ("pack", header(true)),
        ("params", file.params.first().map(|(_, _, line)| Some(*line))),
        ("question", file.questions.first().map(|q| Some(q.line))),
        ("suppress", file.suppressions.first().map(|s| Some(s.line))),
        (
            "use",
            file.items.iter().find_map(|e| match e {
                Entry::Use { line, .. } => Some(Some(*line)),
                _ => None,
            }),
        ),
    ];
    found.into_iter().filter_map(|(kw, at)| at.map(|line| (kw, line))).collect()
}

/// An entry that does not belong where it stands: what it is, and what to do about it
/// where the entry is hand-written. A `use`d file's misfit gets its own advice — the
/// line to edit there is the `use`, not the entry.
#[derive(Debug)]
pub(super) struct Misfit {
    pub line: usize,
    pub what: String,
    pub fix: String,
}

impl Misfit {
    pub(super) fn message(&self) -> String {
        if self.fix.is_empty() {
            self.what.clone()
        } else {
            format!("{}. {}", self.what, self.fix)
        }
    }
}

/// What a type belongs to when it is not the node it is written in — `None` for a type
/// that lives where it stands.
///
/// A project is the bottom of the resource hierarchy, so nothing above it is placed by
/// standing in its body: a folder and a project hang off the organisation or off a
/// folder, an organisation grant off the organisation, a group off the Cloud Identity
/// customer, a billing grant off the billing account. Written there they would reach the
/// organisation anyway, which reads as "in this project" and is not. The same types are
/// written at the TOP LEVEL of a file — the one that declares the project included — and
/// hoist from wherever that file is `use`d.
fn above_a_project(k: &str, types: &dyn TypeResolver) -> Option<String> {
    match k {
        "google_folder" => return Some("a folder hangs off the organisation or another folder".to_string()),
        "google_project" => return Some("a project hangs off the organisation or a folder".to_string()),
        _ => {}
    }
    match types.resolve(k)?.scope {
        crate::Scope::Org => Some("it belongs to the organisation".to_string()),
        crate::Scope::Customer => Some("it belongs to the Cloud Identity customer".to_string()),
        crate::Scope::Billing => Some("it belongs to the billing account".to_string()),
        crate::Scope::Node => None,
    }
}

/// Whether a key names something that opens a map at the top level of a file.
fn opens_a_map(k: &str, types: &dyn TypeResolver) -> bool {
    matches!(k, "google_folder" | "google_project" | "terraform" | "providers") || types.resolve(k).is_some()
}

/// The key as a plain word, when it is one: an identifier, or a quoted string with no
/// interpolation in it.
fn plain(key: &Key) -> Option<&str> {
    match key {
        Key::Ident(k) => Some(k),
        Key::Str(parts) => match parts.as_slice() {
            [StrPart::Lit(k)] => Some(k),
            _ => None,
        },
    }
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Ident(k) => k.clone(),
        Key::Str(_) => plain(key).map(|k| format!("\"{}\"", k)).unwrap_or_else(|| "\"…\"".to_string()),
    }
}

/// What a line moved out of a node's body has to carry with it. Every pack emits the same
/// resources wherever its line stands, bar the two that create a project: those name the
/// folder the project is created in with a param of their own, and the param is what has
/// to be bound once the line no longer stands in the folder.
fn relocation_advice(node: &str, label: &str) -> String {
    let packs = "`logsink_project_folder` in `presets/monitoring/organization-audit-logsink.satz`, \
                 `mdc_mgmt_project_folder` in `presets/integrations/microsoft-defender-for-cloud.satz`";
    match node {
        "google_folder" => format!(
            "A pack that creates a project names the folder it is created in with a param of its own — {packs} — \
             so bind that param to `{node}.{label}.name` in the estate's `params {{ … }}`. Every other pack emits \
             the same resources wherever its line stands",
            packs = packs,
            node = node,
            label = label
        ),
        _ => format!(
            "A resource of the pack that belongs in this project names the project itself; a pack that creates \
             a project names the folder it is created in with a param of its own ({packs})",
            packs = packs
        ),
    }
}

/// `Some` when `entry` does not belong at `pos`. Only an IDENTIFIER key is ever a
/// statement or, inside a map of names, a type: a quoted key is a name, which is how a
/// resource that really is called `params` is written.
pub(super) fn misfit(pos: Position, entry: &Entry, types: &dyn TypeResolver) -> Option<Misfit> {
    match entry {
        Entry::Use { path, line, .. } => match pos {
            Position::NodeMap { node: "google_project" } => Some(Misfit {
                line: *line,
                what: "`use` directly inside `google_project { … }`, where every entry is a project".to_string(),
                fix: "A `use` stands at the top level of a file, in `google_folder { … }` or in a resource type map".to_string(),
            }),
            Position::NodeBody { node, label } => Some(Misfit {
                line: *line,
                what: format!(
                    "`use \"{path}\"` stands in the body of `{node}.{label}`, which holds the estate's own resources — a pack is used at the top level of a file",
                    path = path,
                    node = node,
                    label = label
                ),
                fix: format!("Move the line to the top level. {}", relocation_advice(node, label)),
            }),
            _ => None,
        },
        Entry::Attr { key, line, .. } => match pos {
            Position::NodeBody { .. } | Position::ResourceMap { grant: true, .. } => None,
            Position::File => Some(Misfit {
                line: *line,
                what: format!("`{:?}` is an attribute at the top level of the file — attributes live inside a resource block", key),
                fix: String::new(),
            }),
            Position::NodeMap { .. } | Position::ResourceMap { .. } => Some(Misfit {
                line: *line,
                what: format!("`{} = …` is an attribute directly inside {}, where every entry is a name with its body", key_text(key), pos.written()),
                fix: "An attribute lives inside one of those bodies".to_string(),
            }),
        },
        Entry::Map { key, name, line, .. } => {
            if let Key::Ident(k) = key {
                if let Some((_, does, _)) = statement(k) {
                    let quoted = match pos {
                        Position::NodeMap { .. } | Position::ResourceMap { .. } => {
                            format!("; one that really is called `{k}` is written quoted, `\"{k}\" {{ … }}`", k = k)
                        }
                        Position::File | Position::NodeBody { .. } => String::new(),
                    };
                    return Some(Misfit {
                        line: *line,
                        what: format!(
                            "`{}` is a Satz statement: it is written at the top level of a file, where it {}. Directly inside {} it is read as {}",
                            k,
                            does,
                            pos.written(),
                            pos.reads(k)
                        ),
                        fix: format!("Move it to the top level of the file{}", quoted),
                    });
                }
            }
            match pos {
                Position::File => match (plain(key), key) {
                    (Some(k), _) if opens_a_map(k, types) => None,
                    (Some(k), Key::Ident(_)) => Some(Misfit { line: *line, what: unknown_type_msg(types, k, "block"), fix: String::new() }),
                    _ => Some(Misfit {
                        line: *line,
                        what: format!("`{} {{ … }}` is a label with its body, and the top level of a file takes resource type maps", key_text(key)),
                        fix: "A label stands inside the map of its type: `google_x { \"…\" { … } }`".to_string(),
                    }),
                },
                Position::NodeBody { node: "google_project", .. } => {
                    plain(key).and_then(|k| above_a_project(k, types)).map(|what| Misfit {
                        line: *line,
                        what: format!(
                            "`{} {{ … }}` stands in the body of a `google_project`, and {} — not to the project",
                            key_text(key), what
                        ),
                        fix: "It is written at the top level of a file, the file that declares the project included: \
                              one file declares a project together with the organisation-level resources that go with it, \
                              and those reach the organisation from wherever that file is `use`d"
                            .to_string(),
                    })
                }
                Position::NodeBody { .. } => None,
                Position::NodeMap { .. } | Position::ResourceMap { .. } => match (key, name) {
                    (Key::Ident(k), None) if opens_a_map(k, types) => Some(Misfit {
                        line: *line,
                        what: format!(
                            "`{} {{ … }}` opens a map of its own, and directly inside {} every key is a name — it is read as {}",
                            k,
                            pos.written(),
                            pos.reads(k)
                        ),
                        fix: "It stands at the top level of a file, or in the body of a folder or a project".to_string(),
                    }),
                    (_, Some(_)) if matches!(pos, Position::ResourceMap { .. }) => Some(Misfit {
                        line: *line,
                        what: format!("`{} <name> {{ … }}` directly inside {}, where every entry is one label with its body", key_text(key), pos.written()),
                        fix: "Write `<label> { … }`".to_string(),
                    }),
                    _ => None,
                },
            }
        }
    }
}

/// A `use`d file against the position its `use` stands in. The error is located at the
/// `use` — that is the line to edit — and names the entry in the used file that does not
/// fit, or says that the file brings nothing the position takes.
pub(super) fn used_file_fits(
    pos: Position,
    file: &satz::File,
    use_path: &str,
    using_file: &str,
    use_line: usize,
    types: &dyn TypeResolver,
) -> Result<(), PipelineError> {
    let refuse = |msg: String| Err(PipelineError { file: using_file.to_string(), line: use_line, msg });
    let (stands, takes) = pos.stands_and_takes();
    if let Some(i) = &file.interface_file {
        if pos != Position::File {
            return refuse(format!(
                "use \"{}\" {}: it is the interface file of `{}`, which is used at the top level of a file, without `as` — its values are read as `${{{{interface.<export>}}}}` wherever they are needed",
                use_path, stands, i.name
            ));
        }
    }
    let carried = statements_in(file);
    for (kw, at) in &carried {
        if statement(kw).is_some_and(|(_, _, used)| *used == Used::EstateOnly) {
            return refuse(format!(
                "use \"{path}\": {path}:{at} is a `{kw}`, which is read from the estate alone — in a used file it is never applied. Write it in the estate, or take the resource out of the used file",
                path = use_path,
                at = at.unwrap_or(0),
                kw = kw
            ));
        }
    }
    for e in &file.items {
        if let Some(m) = misfit(pos, e, types) {
            return refuse(format!(
                "use \"{path}\" {stands}: {path}:{at} does not belong there — {what}. A file used {stands} holds {takes}. {advice}",
                path = use_path,
                stands = stands,
                at = m.line,
                what = m.what,
                takes = takes,
                advice = advice(pos, file, use_path, types)
            ));
        }
    }
    if !matches!(pos, Position::File | Position::NodeBody { .. }) && file.items.is_empty() {
        let held: Vec<String> = carried
            .iter()
            .filter(|(kw, _)| statement(kw).is_some_and(|(_, _, used)| *used == Used::Absorbed))
            .map(|(kw, _)| format!("`{}`", kw))
            .collect();
        return refuse(format!(
            "use \"{path}\" {stands}: that file holds {held}. A file used {stands} holds {takes}. Write this one at the top level of the estate: `use \"{path}\"`",
            path = use_path,
            stands = stands,
            held = if held.is_empty() {
                "no entry".to_string()
            } else {
                format!("no entry — only {}, which reach the estate from any position", held.join(", "))
            },
            takes = takes
        ));
    }
    Ok(())
}

/// The line to write instead, read off the used file's own shape.
fn advice(pos: Position, file: &satz::File, use_path: &str, types: &dyn TypeResolver) -> String {
    let typed = file.items.iter().any(|e| matches!(e, Entry::Map { key: Key::Ident(k), name: None, .. } if opens_a_map(k, types)));
    match pos {
        Position::NodeMap { .. } | Position::ResourceMap { .. } if typed => format!(
            "This one declares its own resource types, so it is written bare, at the top level or in the body of a folder or a project: `use \"{}\"`",
            use_path
        ),
        Position::NodeMap { .. } | Position::ResourceMap { .. } => "Move the `use` to where the file's entries belong".to_string(),
        Position::File | Position::NodeBody { .. } => format!(
            "A file that is a list of labelled bodies is used inside the map of their type: `<type> {{ use \"{path}\" }}`, or `use \"{path}\" as <type>`",
            path = use_path
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{compile_estate, estate_questions, type_facts, FrontEnd, ResolvedType};
    use super::*;

    /// The rule cannot lag the language: every statement the parser dispatches on has a
    /// row here, no row names anything else, and a file carrying the statement is seen to
    /// carry it. `STATEMENT_KEYWORDS` is itself derived from the dispatch in `parse`.
    #[test]
    fn every_statement_has_a_row_and_is_found_in_a_file() {
        let rows: Vec<&str> = STATEMENTS.iter().map(|(kw, _, _)| *kw).collect();
        assert_eq!(
            rows,
            satz::STATEMENT_KEYWORDS,
            "position::STATEMENTS and satz::STATEMENT_KEYWORDS disagree — a statement needs a row saying what it does \
             at the top level of a file and what happens to it in a `use`d file"
        );
        for kw in satz::STATEMENT_KEYWORDS {
            let file = satz::parse(&satz::statement_probe(kw)).unwrap_or_else(|e| panic!("`{}` probe: {}:{}", kw, e.line, e.msg));
            assert!(
                statements_in(&file).iter().any(|(found, _)| found == kw),
                "`statements_in` does not see `{}` in a file that carries it — add the field the parser fills for it",
                kw
            );
        }
    }

    struct Types;
    impl TypeResolver for Types {
        // No schema behind this table: no verdict on a body's keys.
        fn body_keys(&self, _tf_type: &str, _path: &[&str]) -> Option<crate::pipeline::BodyKeys> {
            None
        }
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            const KNOWN: [&str; 4] =
                ["google_storage_bucket", "google_org_policy_policy", "google_essential_contacts_contact", "google_organization_iam_member"];
            KNOWN.contains(&key).then(|| {
                let (class, scope) = type_facts(key);
                ResolvedType { tf_type: key.to_string(), class, scope }
            })
        }
    }

    /// A pack that declares its own types, with everything a pack may carry beside them.
    const TYPED: &str = r#"pack logging version "1.0"

params {
  archive_bucket = "acme-archive-001"
  want_archive   = true
}

question want_archive {
  prompt   = "Keep an archive?"
  reversal = edit
  blast    = low
}

claim "cis-gcp" "4.0" "2.2" implements {
  resources = ["google_storage_bucket.archive"]
}

google_storage_bucket {
  archive {
    name = archive_bucket
  }
}
"#;

    /// A bare list: labelled bodies with no type of their own, and a param and its question.
    const LIST: &str = r#"pack contacts version "1.0"

params {
  contact_email = "ops@example.com"
}

question contact_email {
  prompt   = "Who is told?"
  reversal = edit
  blast    = low
}

all {
  email = contact_email
}
"#;

    /// An estate-level file: statements and no entry.
    const STATEMENTS_ONLY: &str = r#"pack core version "1.0"

params {
  customer_shortname = ""
}

question customer_shortname {
  prompt   = "A short name"
  reversal = edit
  blast    = low
}
"#;

    const WITH_SUPPRESS: &str = "pack sup version \"1.0\"\n\nsuppress google_storage_bucket \"archive\"\n";

    fn load(p: &str) -> Result<String, String> {
        match p {
            "typed.satz" => Ok(TYPED.to_string()),
            "list.satz" => Ok(LIST.to_string()),
            "core.satz" => Ok(STATEMENTS_ONLY.to_string()),
            "sup.satz" => Ok(WITH_SUPPRESS.to_string()),
            other => Err(format!("no load: {}", other)),
        }
    }

    fn compile(estate: &str) -> Result<FrontEnd, PipelineError> {
        compile_estate("t.satz", estate, &Types, &load)
    }

    fn refused(form: &str, estate: &str) -> PipelineError {
        match compile(estate) {
            Ok(_) => panic!("{}: must be refused", form),
            Err(e) => e,
        }
    }

    /// Rule 2, as ADR 0046 leaves it: a pack is used at the top level of a file, where it
    /// declares its resources under nothing. A `use` in the body of a folder or a project
    /// is refused, and the refusal names the node and the edit.
    #[test]
    fn a_pack_is_used_at_the_top_level_and_a_use_in_a_nodes_body_is_refused() {
        let fe = compile("estate t\n\nuse \"typed.satz\"\n").expect("a pack is used at the top level");
        let buckets: Vec<_> =
            fe.fragments.iter().flat_map(|f| f.entities.values()).filter(|e| e.addr.tf_type == "google_storage_bucket").collect();
        assert_eq!(buckets.len(), 1, "the pack's bucket is declared once");
        assert_eq!(buckets[0].addr.label, "archive");
        assert!(buckets[0].node_path.is_empty(), "a pack used at the top level stands under no node");

        for (form, estate, node, advice) in [
            (
                "in a folder's body",
                "estate t\n\ngoogle_folder {\n  shared {\n    display_name = \"Shared\"\n    use \"typed.satz\"\n  }\n}\n",
                "google_folder.shared",
                "bind that param to `google_folder.shared.name`",
            ),
            (
                "in a project's body",
                "estate t\n\ngoogle_project {\n  \"acme-host-001\" {\n    name = \"acme-host-001\"\n    use \"typed.satz\"\n  }\n}\n",
                "google_project.acme-host-001",
                "names the project itself",
            ),
        ] {
            let err = refused(form, estate);
            assert_eq!(err.file, "t.satz", "{}", form);
            assert!(err.msg.contains(&format!("stands in the body of `{}`", node)), "{}: {}", form, err.msg);
            assert!(err.msg.contains("a pack is used at the top level of a file"), "{}: {}", form, err.msg);
            assert!(err.msg.contains("Move the line to the top level"), "{}: {}", form, err.msg);
            assert!(err.msg.contains(advice), "{}: the refusal does not name the edit — {}", form, err.msg);
        }
    }

    /// Rule 3: a used file's statements reach the estate from every position its `use`
    /// may take, and none of them becomes an entity. The questions report's own walk
    /// (`estate_questions`, schema-free) sees the same questions.
    #[test]
    fn a_used_file_s_statements_reach_the_estate_from_every_position() {
        let forms = [
            ("at the top level", "estate t\n\nuse \"typed.satz\"\n", "logging", "want_archive", "archive_bucket"),
            ("in a resource type map", "estate t\n\ngoogle_essential_contacts_contact {\n  use \"list.satz\"\n}\n", "contacts", "contact_email", "contact_email"),
            ("as a resource type", "estate t\n\nuse \"list.satz\" as google_essential_contacts_contact\n", "contacts", "contact_email", "contact_email"),
            (
                "as a resource type, in the folder map",
                "estate t\n\ngoogle_folder {\n  use \"list.satz\" as google_essential_contacts_contact\n}\n",
                "contacts",
                "contact_email",
                "contact_email",
            ),
        ];
        for (form, estate, pack, question, param) in forms {
            let fe = compile(estate).unwrap_or_else(|e| panic!("{}: {}", form, e));
            assert_eq!(fe.questions.len(), 1, "{}: the pack's questions reach the front end", form);
            assert_eq!((fe.questions[0].pack.as_str(), fe.questions[0].questions[0].subject.as_str()), (pack, question), "{}", form);
            assert!(fe.tfvars.contains_key(param), "{}: the pack's param is in the estate's namespace", form);
            if pack == "logging" {
                assert_eq!(fe.claims.len(), 1, "{}: and so is its claim", form);
            }
            for e in fe.fragments.iter().flat_map(|f| f.entities.values()) {
                for word in ["params", "question", "claim", "pack", question] {
                    assert_ne!(e.addr.label, word, "{}: a statement became the entity {}.{}", form, e.addr.tf_type, e.addr.label);
                }
                if let crate::algebra::Body::Attrs(serde_yaml::Value::Mapping(m)) = &e.body {
                    for word in ["params", "question", "prompt", "reversal", "blast"] {
                        assert!(!m.contains_key(word), "{}: `{}` reached the body of {}.{}", form, word, e.addr.tf_type, e.addr.label);
                    }
                }
            }
            let (asked, _, _) = estate_questions("t.satz", estate, &load).unwrap_or_else(|e| panic!("{}: {}", form, e));
            assert_eq!(asked.len(), 1, "{}: the questions report sees the pack", form);
            assert_eq!(asked[0].questions[0].subject, question, "{}", form);
        }
    }

    /// A bare list is a resource type map's content, gated or not.
    #[test]
    fn a_bare_list_is_the_content_of_the_map_it_is_used_in() {
        for (form, estate, emitted) in [
            ("inside the map", "estate t\n\ngoogle_essential_contacts_contact {\n  use \"list.satz\"\n}\n", true),
            ("gated on", "estate t\n\nparams { want = true }\n\ngoogle_essential_contacts_contact {\n  use \"list.satz\" when want\n}\n", true),
            ("gated off", "estate t\n\nparams { want = false }\n\ngoogle_essential_contacts_contact {\n  use \"list.satz\" when want\n}\n", false),
            ("as", "estate t\n\nuse \"list.satz\" as google_essential_contacts_contact\n", true),
        ] {
            let fe = compile(estate).unwrap_or_else(|e| panic!("{}: {}", form, e));
            let contacts: Vec<_> = fe.fragments.iter().flat_map(|f| f.entities.keys()).map(|a| format!("{}.{}", a.tf_type, a.label)).collect();
            let expected: Vec<String> = if emitted { vec!["google_essential_contacts_contact.all".into()] } else { Vec::new() };
            assert_eq!(contacts, expected, "{}", form);
        }
    }

    /// Rule 4, hand-written: a statement stands at the top level of a file. Anywhere else
    /// it used to be read as whatever the position takes — a resource labelled `params`,
    /// a folder called `question`, an attribute block the provider does not have.
    #[test]
    fn a_statement_written_inside_a_block_is_refused_at_every_position() {
        let forms = [
            ("params in a resource type map", "estate t\n\ngoogle_storage_bucket {\n  params {\n    name = \"x\"\n  }\n}\n", "params", "a resource `google_storage_bucket.params`"),
            ("notice in a grant map", "estate t\n\ngoogle_organization_iam_member {\n  notice {\n  }\n}\n", "notice", "a resource `google_organization_iam_member.notice`"),
            ("params in the folder map", "estate t\n\ngoogle_folder {\n  params {\n    a = 1\n  }\n}\n", "params", "a folder named `params`"),
            ("question in the folder map", "estate t\n\ngoogle_folder {\n  question a {\n  }\n}\n", "question", "a folder named `question`"),
            ("action in the project map", "estate t\n\ngoogle_project {\n  action {\n  }\n}\n", "action", "a project named `action`"),
            ("params in a folder's body", "estate t\n\ngoogle_folder {\n  shared {\n    params {\n      a = 1\n    }\n  }\n}\n", "params", "an attribute block `params` of the `google_folder`"),
            ("claim in a project's body", "estate t\n\ngoogle_project {\n  p {\n    claim {\n    }\n  }\n}\n", "claim", "an attribute block `claim` of the `google_project`"),
        ];
        for (form, estate, kw, read_as) in forms {
            let err = refused(form, estate);
            assert_eq!(err.file, "t.satz", "{}", form);
            assert!(err.msg.contains(&format!("`{}` is a Satz statement", kw)), "{}: {}", form, err.msg);
            assert!(err.msg.contains(read_as), "{}: the refusal says what it would have become — got {}", form, err.msg);
            assert!(err.msg.contains("Move it to the top level of the file"), "{}: {}", form, err.msg);
        }
        let err = refused("question in a resource type map", "estate t\n\ngoogle_storage_bucket {\n  question a {\n  }\n}\n");
        assert_eq!(err.line, 4);
        // a quoted key is a label, whatever it spells — and a block of a resource's own
        // body is the provider's, which is where `action { … }` of a lifecycle rule lives
        let fe = compile("estate t\n\ngoogle_storage_bucket {\n  \"params\" {\n    lifecycle_rule {\n      action { type = \"Delete\" }\n    }\n  }\n}\n").unwrap();
        assert!(fe.fragments.iter().flat_map(|f| f.entities.keys()).any(|a| a.label == "params"));
    }

    /// Rule 4, hand-written: inside a map of names every key is a name, so a resource
    /// type map written there became a resource — or a folder — called after the type.
    #[test]
    fn a_map_written_inside_a_map_of_names_is_refused() {
        for (form, estate, read_as) in [
            ("a type map in a type map", "estate t\n\ngoogle_org_policy_policy {\n  google_storage_bucket {\n  }\n}\n", "a resource `google_org_policy_policy.google_storage_bucket`"),
            ("a type map in the folder map", "estate t\n\ngoogle_folder {\n  google_storage_bucket {\n  }\n}\n", "a folder named `google_storage_bucket`"),
            ("the project map in the folder map", "estate t\n\ngoogle_folder {\n  google_project {\n  }\n}\n", "a folder named `google_project`"),
            ("a type map in the project map", "estate t\n\ngoogle_project {\n  google_storage_bucket {\n  }\n}\n", "a project named `google_storage_bucket`"),
        ] {
            let err = refused(form, estate);
            assert_eq!(err.line, 4, "{}", form);
            assert!(err.msg.contains("opens a map of its own") && err.msg.contains(read_as), "{}: {}", form, err.msg);
        }
    }

    /// Rule 4, used files: a file is judged by whether its entries fit where its `use`
    /// stands — there is no kind of pack to declare. The refusal is located at the `use`,
    /// the line to edit, and names the entry in the used file.
    #[test]
    fn a_used_file_that_does_not_fit_the_position_of_its_use_is_refused() {
        let typed_forms = [
            ("a typed pack inside a resource type map", "estate t\n\ngoogle_org_policy_policy {\n  use \"typed.satz\"\n}\n", "inside `google_org_policy_policy { … }`"),
            ("a typed pack as a resource type", "estate t\n\nuse \"typed.satz\" as google_org_policy_policy\n", "inside `google_org_policy_policy { … }`"),
            ("a typed pack as a resource type, in the folder map", "estate t\n\ngoogle_folder {\n  use \"typed.satz\" as google_org_policy_policy\n}\n", "inside `google_org_policy_policy { … }`"),
            // the hole: this compiled, and the pack's types became folder names
            ("a typed pack in the folder map", "estate t\n\ngoogle_folder {\n  use \"typed.satz\"\n}\n", "inside `google_folder { … }`"),
        ];
        for (form, estate, stands) in typed_forms {
            let err = refused(form, estate);
            assert_eq!(err.file, "t.satz", "{}: located at the `use`", form);
            assert!(err.msg.contains(stands), "{}: {}", form, err.msg);
            assert!(err.msg.contains("typed.satz:18"), "{}: the refusal names the entry in the used file — got {}", form, err.msg);
            assert!(err.msg.contains("`google_storage_bucket { … }` opens a map of its own"), "{}: {}", form, err.msg);
            assert!(err.msg.contains("it is written bare") && err.msg.contains("`use \"typed.satz\"`"), "{}: and the line to write — got {}", form, err.msg);
        }

        // an estate-level file holds statements and no entry: fine at the top level,
        // and nothing for a map of names to take
        compile("estate t\n\nuse \"core.satz\"\n").expect("an estate-level file is used at the top level");
        for (form, estate) in [
            ("an estate-level file in a resource type map", "estate t\n\ngoogle_storage_bucket {\n  use \"core.satz\"\n}\n"),
            ("an estate-level file in the folder map", "estate t\n\ngoogle_folder {\n  use \"core.satz\"\n}\n"),
        ] {
            let err = refused(form, estate);
            assert_eq!((err.file.as_str(), err.line), ("t.satz", 4), "{}", form);
            assert!(err.msg.contains("holds no entry — only `params`, `question`, which reach the estate from any position"), "{}: {}", form, err.msg);
            assert!(err.msg.contains("Write this one at the top level of the estate: `use \"core.satz\"`"), "{}: {}", form, err.msg);
        }

        // a bare list used bare: its labels are no resource types
        let err = refused("a bare list at the top level", "estate t\n\nuse \"list.satz\"\n");
        assert_eq!((err.file.as_str(), err.line), ("t.satz", 3));
        assert!(err.msg.contains("list.satz:13") && err.msg.contains("`all`"), "{}", err.msg);
        assert!(err.msg.contains("`<type> { use \"list.satz\" }`"), "{}", err.msg);

        // `google_project { … }` takes projects and no `use`
        let err = refused("a use in the project map", "estate t\n\ngoogle_project {\n  use \"typed.satz\"\n}\n");
        assert!(err.msg.contains("`use` directly inside `google_project { … }`"), "{}", err.msg);
    }

    /// A suppression is read from the estate's own file. A used file's was dropped
    /// without a word, at every position.
    #[test]
    fn a_suppress_in_a_used_file_is_refused() {
        let err = refused("suppress in a used file", "estate t\n\nuse \"typed.satz\"\nuse \"sup.satz\"\n");
        assert_eq!((err.file.as_str(), err.line), ("t.satz", 4));
        assert!(err.msg.contains("sup.satz:3 is a `suppress`") && err.msg.contains("read from the estate alone"), "{}", err.msg);
    }
}
