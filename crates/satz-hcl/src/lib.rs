//! The HCL input shape of `satz import` (roadmap Phase 3): a directory of
//! `.tf` — hand-written, `gcloud … bulk-export --resource-format=terraform`,
//! `tofu plan -generate-config-out` — becomes a Satz estate.
//!
//! Three tiers, and every input block ends in exactly one of them.
//!
//! **Promote**: a `variable` with a literal `default`, and a `locals` entry
//! whose value is a literal or a template over other constants, become Satz
//! **params**. Params *are* Satz's variables, so this is the language's own
//! model rather than a rewrite, and the imported estate stays
//! re-parameterisable — which is what turning an import into a pack needs. A
//! `variable` declared WITHOUT a default is named in the header instead of
//! given a value: `satz transpile` then stops with `unknown param`, the honest
//! gate for a value the source never supplied either.
//!
//! **Translate**: a `resource` block of a schema-known type becomes a Satz
//! resource when every value is a literal, a promoted param, or a reference to
//! a managed resource — the last carried verbatim as Satz `${{…}}`, which the
//! emitter renders back byte-identically. `provider` is a Satz body key, so the
//! alias travels with the resource; `depends_on` is not one, because satz
//! derives a plan's ordering from the estate itself, so the edge is dropped and
//! reported — and only to a block this import carries. Positional types are placed: a
//! folder whose `parent` is the organisation goes to the top, one whose parent
//! references another folder nests under it; a project nests under the folder
//! its `folder_id` references (or sits at the top when its `org_id` is the
//! organisation); a project's services, IAM grants and project-scoped
//! resources that reference it by `project` move inside it; folder and
//! organisation grants likewise. A variable used as the organisation (`org_id
//! = var.x`, `parent = "organizations/${var.x}"`) is bound to
//! `customer_organization_id` — to the organisation `--organization` names
//! when it has no default. A `count = length(<promoted list>)` whose entry is
//! read as `<list>[count.index]` or `element(<list>, count.index)` expands into
//! one resource per distinct entry.
//!
//! **The default provider.** A translated resource that names no provider is
//! emitted with the estate's own, so what it took from the source's default
//! `google` provider is carried: the default project as placement (into the
//! imported project of that id, the id folded through params and templates) or
//! as the resource's own `project`, the region and zone into the estate's
//! `providers` block. A default satz cannot write wraps each resource that
//! relied on it, naming it.
//!
//! **Wrap**: everything else is carried verbatim as
//! `hcl trust "imported from <file>:<line>" { … }` — it deploys exactly as
//! written, the compliance plane cannot see into it, the fold cannot compose
//! it — and the report says why. A block whose parent is wrapped is wrapped
//! too (closure by dependency), its reason naming the block that started the
//! chain, and a promoted declaration a wrapped block still reads is carried
//! verbatim beside it. The `terraform` block is dropped: the emitter owns
//! `providers.tf`. A `provider` block is carried verbatim when a block that
//! stays verbatim, or a resource naming it, uses its configuration — the
//! emitter's own providers are all aliased — and dropped otherwise, with what
//! was carried from it in its row.
//!
//! An import may be partial, never silent. Note that *translated* is not
//! *proven*: a `${…}` reference is opaque to the compliance plane.
//!
//! A translated resource may reference only another TRANSLATED one. satz emits
//! no address for a verbatim block — `hcl trust` is text, and the emission
//! manifest does not hold it — so a `${…}` that crosses from a translated block
//! to a wrapped one, or out of the import altogether, is an estate
//! `satz transpile` refuses. The import refuses first, names both sides, and
//! writes nothing.
//!
//! This crate keeps `hcl-rs`/`hcl-edit` out of satz-core.

use std::collections::{BTreeMap, BTreeSet};

use hcl::edit::expr::{Expression, ObjectKey, Traversal, TraversalOperator};
use hcl::edit::parser::parse_body;
use hcl::edit::structure::{Block, Body, Structure};
use hcl::edit::template::{Element, StringTemplate};
use hcl::edit::visit::{self, Visit};
use hcl::edit::Span;

use satz_core::migrate;

/// What happened to one top-level block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file: String,
    pub line: usize,
    /// `resource "google_folder" "x"`, `module "vpc"`, `locals`, …
    pub what: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// a Satz resource now
    Translated,
    /// `count` over a promoted list: one Satz resource per entry
    Expanded(usize),
    /// consumed into `params` — neither wrapped nor dropped
    Promoted(String),
    /// verbatim inside `hcl trust`, and why
    Wrapped(String),
    /// `terraform` / `provider`: the emitter writes providers.tf itself
    Dropped(String),
}

#[derive(Debug)]
pub struct Imported {
    pub satz: String,
    pub rows: Vec<Row>,
    /// Facts that are not about one block: params the reviewer must bind,
    /// how many ordering edges were dropped.
    pub notes: Vec<String>,
    /// Per translated block, the `depends_on` it no longer writes: satz derives
    /// a plan's ordering from the estate, so the edge is not carried. `--verbose`
    /// prints them.
    pub ordering_dropped: Vec<(String, Vec<String>)>,
}

/// One input file: its path (as shown in provenance) and text.
pub struct Input {
    pub path: String,
    pub text: String,
}

/// What the provider schema knows. The importer asks two questions of it: does
/// this type exist, and does it carry this attribute (which decides whether a
/// resource can inherit a dropped provider's default project).
pub trait Schema {
    fn has_type(&self, tf_type: &str) -> bool;
    fn has_attr(&self, tf_type: &str, attr: &str) -> bool;
    /// The type's required attributes — what decides which `*_iam_member`
    /// types pin their scope in the map (`bucket`, `service_account_id`).
    fn required_attrs(&self, tf_type: &str) -> Vec<String>;
}

use satz_core::pipeline::GrantForm;

/// How a grant type is written, by satz-core's one rule over the schema.
fn grant_form_of(schema: &dyn Schema, tf_type: &str) -> GrantForm {
    let required = schema.required_attrs(tf_type);
    let required: Vec<&str> = required.iter().map(String::as_str).collect();
    satz_core::pipeline::grant_form(tf_type, &required)
}

/// A grant written as a member map — scoped by the organization, by its node,
/// or pinned to a scope named in the map.
fn is_map_form(form: &GrantForm) -> bool {
    matches!(form, GrantForm::Org | GrantForm::Node | GrantForm::Pinned(_))
}

/// Whether a type's Satz form is the resource body, so a `provider` in it
/// reaches the emission. A folder, a project and its service list, a group and
/// its memberships, and every grant written as a member map are built from a
/// form of their own, which takes the alias from the estate and reads none from
/// the body — there an alias that is not the estate's own cannot be written.
fn body_carries_provider(tf_type: &str, form: &GrantForm) -> bool {
    !is_map_form(form)
        && !matches!(
            tf_type,
            "google_folder"
                | "google_project"
                | "google_project_service"
                | "google_cloud_identity_group"
                | "google_cloud_identity_group_membership"
        )
}

/// What a grant map reads as a member rather than as the scope it pins: the
/// front end's own rule (`satz_core::pipeline::is_scope_attr_key`), so a member
/// the import cannot write as one wraps instead of becoming an estate the front
/// end refuses.
fn is_member(s: &str) -> bool {
    !satz_core::pipeline::is_scope_attr_key(s)
}

const META_BLOCKS: &[&str] = &["dynamic", "provisioner", "connection"];
/// The meta-arguments no Satz form carries. `count` is expanded where it is
/// Terraform's "one per entry" idiom (`count_expansion`) and wraps otherwise.
/// `provider` and `depends_on` are NOT here: `provider` is a Satz body key and
/// `depends_on` is ordering satz derives itself.
const UNEXPRESSIBLE_META_ATTRS: &[&str] = &["count", "for_each"];
/// The provider reference the emitter puts on every resource of an estate this
/// import writes, because `base_estate` declares `google` with alias `google`.
/// A resource carrying exactly this gets it back whether or not the Satz body
/// says so — which is what lets a grant map, which has no room for a provider,
/// still translate. `base_estate_declares_the_default_provider` proves the two
/// agree.
const DEFAULT_PROVIDER: &str = "google.google";
/// The param an estate names its organisation by.
const ORGANIZATION_PARAM: &str = "customer_organization_id";
/// Why a `terraform` block, or a `provider` block nothing still uses, is dropped.
const TERRAFORM_DROPPED: &str = "the emitter writes providers.tf from the estate's `terraform` and `providers` blocks";
/// The reason `classify` gives a block that still carries a `count`; the
/// expansion replaces it with why that `count` did not expand.
const COUNT_REASON: &str = "uses `count`";

/// Where a translated resource lives.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    /// top level (the organisation)
    Top,
    Folder(String),
    Project(String),
}

/// A resource block after classification.
struct Res {
    tf_type: String,
    label: String,
    file: String,
    line: usize,
    what: String,
    text: String,
    /// literal body (parent/scope attrs removed), when translatable
    body: Option<serde_yaml::Mapping>,
    place: Place,
    /// why it cannot translate on its own
    reason: Option<String>,
    /// what it read: promoted params, and verbatim `<type>.<label>` references
    uses: Uses,
    /// a `google_project`'s `project_id` resolved to a literal, for matching a
    /// dropped provider's default project against the projects in this import
    project_id: Option<String>,
    /// one copy of an expanded `count`: the block already has its own row, so
    /// this resource does not add a second
    expanded: bool,
    /// the provider configuration the source block uses (`google`,
    /// `google-beta`, `google.<alias>`): its `provider`, or its type's own
    provider_ref: String,
}

// ---------------------------------------------------------------------------
// promoted constants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    VarDefault,
    /// declared without a default: a param the reviewer must bind
    VarRequired,
    Local,
}

#[derive(Debug, Clone)]
struct Const {
    /// the name the `params` block declares
    param: String,
    /// the value, `None` for a `VarRequired`. May itself be an interpolation
    /// over other params (a local built from other constants).
    value: Option<serde_yaml::Value>,
    origin: Origin,
    /// index into the declaration list — which block declared it
    decl: usize,
    /// set when the name is unusable; every reader wraps with THIS text
    conflict: Option<String>,
    /// the expression as written — a `variable`'s default, a local's value —
    /// which [`Consts::eval`] folds to a plain string where it can
    expr: Option<Expression>,
    /// the organisation this name stands for: a variable used as an `org_id`
    /// or an `organizations/…` parent ([`bind_organization_variables`]). Its
    /// param is written as `customer_organization_id`, and a placement reads
    /// it as this number.
    organization: Option<String>,
}

/// ONE namespace, keyed by the HCL name as written: `var.x` and `local.x`
/// collapse here on purpose, because Satz has one `params` namespace, so the
/// collision has to surface rather than resolve.
#[derive(Debug, Default)]
struct Consts {
    by_name: BTreeMap<String, Const>,
}

impl Consts {
    /// The constant behind `var.name` / `local.name`, or the reason a block
    /// reading it must wrap.
    fn get(&self, kind: &str, name: &str) -> Result<&Const, String> {
        match self.by_name.get(name) {
            None => Err(format!("a reference to `{}.{}`, which this import does not declare", kind, name)),
            Some(c) => match &c.conflict {
                Some(why) => Err(why.clone()),
                None => Ok(c),
            },
        }
    }

    /// The constant's value as a plain string, when it folds to one — a
    /// literal, or a template over other constants that do (`"${var.prefix}${var.org_id}"`).
    /// The only form a placement decision can be made from.
    fn literal_string(&self, name: &str) -> Option<String> {
        self.eval_name(name, 0)
    }

    fn eval_name(&self, name: &str, depth: usize) -> Option<String> {
        let c = self.by_name.get(name)?;
        if let Some(org) = &c.organization {
            return Some(org.clone());
        }
        self.eval(c.expr.as_ref()?, depth + 1)
    }

    /// An expression folded to a plain string: a string or number literal, a
    /// `var.`/`local.` naming a constant that folds, or a template of those.
    /// `None` for anything else — the caller keeps its own reason.
    fn eval(&self, e: &Expression, depth: usize) -> Option<String> {
        // a local that names itself through others is not a value
        if depth > 16 {
            return None;
        }
        match e {
            Expression::String(s) => Some(s.value().to_string()),
            Expression::Number(n) => Some(n.value().to_string()),
            Expression::Parenthesis(p) => self.eval(p.inner(), depth),
            Expression::Traversal(t) => {
                let (_, name) = const_name(t)?;
                self.eval_name(&name, depth)
            }
            Expression::StringTemplate(t) => {
                let mut out = String::new();
                for el in t.iter() {
                    match el {
                        Element::Literal(l) => out.push_str(l.value()),
                        Element::Interpolation(i) => out.push_str(&self.eval(&i.expr, depth)?),
                        Element::Directive(_) => return None,
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }
}

/// `var.x` / `local.x` as its kind and name.
fn const_name(t: &Traversal) -> Option<(String, String)> {
    let Expression::Variable(v) = &t.expr else { return None };
    let kind = v.as_str();
    if kind != "var" && kind != "local" {
        return None;
    }
    let [op] = t.operators.as_slice() else { return None };
    let TraversalOperator::GetAttr(k) = op.value() else { return None };
    Some((kind.to_string(), k.as_str().to_string()))
}

/// One `variable` / `locals` block and what became of it.
struct Decl {
    file: String,
    line: usize,
    what: String,
    text: String,
    /// names it successfully promoted
    promoted: Vec<String>,
    /// names it declares that could NOT be promoted (a non-literal local)
    unresolved: Vec<String>,
    /// why nothing was promoted, when nothing was
    reason: Option<String>,
    /// the block must also be carried verbatim, because a wrapped block reads it
    carry: bool,
}

// ---------------------------------------------------------------------------
// the import
// ---------------------------------------------------------------------------

/// The outcome of one pass. A name can only leave the constants table (never
/// join it), so the retry loop is monotone and terminates.
enum Pass {
    Done(Box<Imported>),
    /// this promoted name has to stay a Terraform variable; try again without it
    Retry(String),
}

/// A block's wrap reason, keyed by where it came from. Carried across a retry
/// so the FIRST reason wins: a later pass sees the same block fail on the
/// mechanical "that name is not a param any more", which is true but hides the
/// defect the reviewer actually has to look at.
type Reasons = BTreeMap<(String, usize), String>;

/// Import every file. `wrap_all` carries the input verbatim and promotes
/// nothing — a param is a translation, and under that flag none happens.
///
/// `organization` is `--organization <n>`: the organisation the estate is bound
/// to when the configuration names none, and a check on the one it names. An
/// estate is written bound to exactly one organisation or not at all
/// ([`bound_organization`]).
pub fn import(
    inputs: &[Input],
    name: &str,
    wrap_all: bool,
    organization: Option<&str>,
    schema: &dyn Schema,
) -> Result<Imported, String> {
    // parse once: one error site, and the pre-pass and the block loop share it
    let mut parsed: Vec<(&Input, Body)> = Vec::new();
    for input in inputs {
        let body = parse_body(&input.text).map_err(|e| format!("{}: {}", input.path, e))?;
        parsed.push((input, body));
    }

    if wrap_all {
        return wrap_everything(&parsed, name, organization);
    }

    let mut forced: BTreeSet<String> = BTreeSet::new();
    let mut reasons: Reasons = Reasons::new();
    loop {
        match one_pass(&parsed, name, organization, schema, &forced, &mut reasons)? {
            Pass::Done(mut imported) => {
                if !forced.is_empty() {
                    imported.notes.push(format!(
                        "{} name(s) had to stay a Terraform variable rather than become a param, because a block that stays verbatim reads them: {}",
                        forced.len(),
                        forced.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                return Ok(*imported);
            }
            Pass::Retry(n) => {
                if !forced.insert(n) {
                    return Err("the import did not settle: a name was forced out twice".into());
                }
            }
        }
    }
}

fn one_pass(
    parsed: &[(&Input, Body)],
    name: &str,
    organization: Option<&str>,
    schema: &dyn Schema,
    forced: &BTreeSet<String>,
    reasons: &mut Reasons,
) -> Result<Pass, String> {
    let (mut consts, mut decls) = collect_consts(parsed, forced)?;
    let mut org_ids: BTreeSet<String> = bind_organization_variables(parsed, &mut consts, organization);

    // the projects of this input by id: a literal `project = "<id>"` places
    // under one of them the way a reference does
    let projects = project_ids(parsed, &consts);

    let mut rows: Vec<Row> = Vec::new();
    let mut verbatim: Vec<(String, usize, String)> = Vec::new();
    let mut resources: Vec<Res> = Vec::new();
    // every `<type>.<label>` this input declares, whatever becomes of it
    let mut declared: BTreeSet<String> = BTreeSet::new();
    // the input's `provider` blocks, judged once every block's fate is known
    let mut providers: Vec<SourceProvider> = Vec::new();
    // the provider configurations the always-verbatim blocks (`data`,
    // `module`, …) use; `*` for a module, which inherits every default one
    let mut verbatim_providers: BTreeSet<String> = BTreeSet::new();
    let mut notes: Vec<String> = Vec::new();

    for (input, body) in parsed {
        for s in body.iter() {
            let Some(span) = s.span() else {
                return Err(format!("{}: a structure without a span (not parsed from text?)", input.path));
            };
            let text = input.text[span.clone()].trim_end().to_string();
            let line = input.text[..span.start].matches('\n').count() + 1;
            let what = describe(s);

            let Some(block) = s.as_block() else {
                verbatim.push((input.path.clone(), line, text));
                rows.push(Row {
                    file: input.path.clone(),
                    line,
                    what,
                    action: Action::Wrapped("a top-level attribute".into()),
                });
                continue;
            };
            let ident = block.ident.to_string();
            if ident == "terraform" {
                rows.push(Row { file: input.path.clone(), line, what, action: Action::Dropped(TERRAFORM_DROPPED.into()) });
                continue;
            }
            if ident == "provider" {
                providers.push(SourceProvider::new(input.path.clone(), line, what, text, block));
                continue;
            }
            if ident == "variable" || ident == "locals" {
                // accounted for by the pre-pass; the row is written from `decls`
                continue;
            }
            if ident != "resource" {
                match ident.as_str() {
                    "data" => {
                        if let Some(t) = block.labels.first() {
                            verbatim_providers.insert(provider_of(block, t.as_str()));
                        }
                    }
                    "module" => {
                        verbatim_providers.insert("*".into());
                    }
                    _ => {}
                }
                verbatim.push((input.path.clone(), line, text));
                rows.push(Row {
                    file: input.path.clone(),
                    line,
                    what,
                    action: Action::Wrapped(format!("`{}` blocks stay verbatim", ident)),
                });
                continue;
            }
            let (tf_type, label) = match block.labels.as_slice() {
                [t, l] => (t.as_str().to_string(), l.as_str().to_string()),
                _ => {
                    if let Some(t) = block.labels.first() {
                        verbatim_providers.insert(provider_of(block, t.as_str()));
                    }
                    verbatim.push((input.path.clone(), line, text));
                    rows.push(Row {
                        file: input.path.clone(),
                        line,
                        what,
                        action: Action::Wrapped("a resource block needs exactly two labels".into()),
                    });
                    continue;
                }
            };
            let provider_ref = provider_of(block, &tf_type);
            declared.insert(format!("{}.{}", tf_type, label));
            let project_id = if tf_type == "google_project" {
                attr_string(block, "project_id", &consts)
            } else {
                None
            };
            // `count = length(<promoted list>)` is one resource per entry, which
            // is what Satz writes; expand it rather than carrying the block.
            let mut count_why: Option<String> = None;
            let expansion = match count_expansion(block, &consts) {
                None => None,
                Some(Err(why)) => {
                    count_why = Some(why);
                    None
                }
                Some(Ok((list, elements))) => {
                    // a list naming one entry twice is one resource: Terraform
                    // creates two that manage the same thing, and Satz cannot
                    // write two resources under one address
                    let mut distinct: Vec<serde_yaml::Value> = Vec::new();
                    let mut repeated: Vec<String> = Vec::new();
                    for e in elements {
                        if distinct.contains(&e) {
                            repeated.push(scalar_text(&e));
                        } else {
                            distinct.push(e);
                        }
                    }
                    // the copy is classified from the block WITHOUT its `count`: the
                    // meta-argument is what expanded, and every gate downstream is
                    // right to refuse one it still sees
                    let mut without_count = block.clone();
                    without_count.body.remove_attribute("count");
                    let mut copies = Vec::new();
                    let mut labels: BTreeSet<String> = BTreeSet::new();
                    for (i, element) in distinct.iter().enumerate() {
                        let mut cx = Cx {
                            consts: &consts,
                            schema,
                            uses: Uses::default(),
                            index: Some((list.clone(), element.clone())),
                        };
                        let c = classify(&tf_type, &label, &without_count, &mut cx, &mut org_ids, &projects);
                        // one copy that cannot be written leaves the whole block
                        // wrapped: half an expansion is worse than none
                        if let Some(why) = &c.reason {
                            count_why = Some(format!(
                                "`count` over `{}` does not expand: its entry `{}` cannot be written — {}",
                                list,
                                scalar_text(element),
                                why
                            ));
                            break;
                        }
                        // two entries that shorten to the same label take their index
                        let mut copy_label = expanded_label(&label, element, i);
                        if !labels.insert(copy_label.clone()) {
                            copy_label = format!("{}_{}", label, i);
                            labels.insert(copy_label.clone());
                        }
                        copies.push((copy_label, c, cx.uses));
                    }
                    if count_why.is_some() {
                        None
                    } else {
                        if !repeated.is_empty() {
                            notes.push(format!(
                                "{}:{} {}: `{}` names {} more than once, and each entry is one resource — the repeat is folded, {} resource(s) for {} entries",
                                input.path,
                                line,
                                what,
                                list,
                                repeated.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", "),
                                copies.len(),
                                copies.len() + repeated.len()
                            ));
                        }
                        Some(copies)
                    }
                }
            };
            if let Some(copies) = expansion {
                rows.push(Row {
                    file: input.path.clone(),
                    line,
                    what: what.clone(),
                    action: Action::Expanded(copies.len()),
                });
                for (copy_label, classified, uses) in copies {
                    resources.push(Res {
                        tf_type: tf_type.clone(),
                        label: copy_label,
                        file: input.path.clone(),
                        line,
                        what: what.clone(),
                        text: text.clone(),
                        body: classified.body,
                        place: classified.place,
                        reason: classified.reason,
                        uses,
                        project_id: project_id.clone(),
                        expanded: true,
                        provider_ref: provider_ref.clone(),
                    });
                }
                continue;
            }
            let mut cx = Cx { consts: &consts, schema, uses: Uses::default(), index: None };
            let mut classified = classify(&tf_type, &label, block, &mut cx, &mut org_ids, &projects);
            // a `count` that did not expand says why, not only that it is there
            if let (Some(why), Some(r)) = (count_why, classified.reason.as_mut()) {
                if r == COUNT_REASON {
                    *r = why;
                }
            }
            resources.push(Res {
                tf_type,
                label,
                file: input.path.clone(),
                line,
                what,
                text,
                body: classified.body,
                place: classified.place,
                reason: classified.reason,
                uses: cx.uses,
                project_id,
                expanded: false,
                provider_ref,
            });
        }
    }

    // what a translated resource that names no provider relied on: the default
    // `google` provider's project, region and zone
    let mut estate_provider = serde_yaml::Mapping::new();
    let carried = match providers.iter().find(|p| p.name == "google" && p.alias.is_none()) {
        Some(sp) => carry_provider_defaults(sp, &consts, schema, &mut resources, &mut estate_provider),
        None => Vec::new(),
    };

    // `depends_on` is dropped, so the edge has to be one the emitter can put
    // back from the estate — which it can only do for a block that is IN the
    // estate. An edge to something this import never saw is ordering nothing
    // recovers, and the block keeps it by staying verbatim.
    let mut dropped_edges: Vec<(String, Vec<String>)> = Vec::new();
    for r in resources.iter_mut() {
        if r.reason.is_some() || r.uses.ordering.is_empty() {
            continue;
        }
        match r.uses.ordering.iter().find(|a| !declared.contains(*a)) {
            Some(missing) => {
                r.reason = Some(format!(
                    "`depends_on` names `{}`, which no block in this import declares — satz derives a plan's ordering from the estate and cannot re-derive an edge to a resource it does not hold",
                    missing
                ))
            }
            None => dropped_edges
                .push((format!("{}.{}", r.tf_type, r.label), r.uses.ordering.iter().cloned().collect())),
        }
    }

    // closure by dependency: a resource under a wrapped container is wrapped
    let mut wrapped_idx = closure(&mut resources);

    // a translated resource whose EMITTED label is derived cannot be referenced
    // by a verbatim block — wrap it too rather than emit a dangling reference
    loop {
        let referenced: BTreeSet<String> = resources
            .iter()
            .enumerate()
            .filter(|(i, _)| wrapped_idx.contains(i))
            .flat_map(|(_, r)| r.uses.refs.iter().cloned())
            .chain(verbatim_reads(parsed).addrs)
            .collect();
        let mut changed = false;
        for (i, r) in resources.iter_mut().enumerate() {
            if wrapped_idx.contains(&i) || !derives_label(&r.tf_type, schema) {
                continue;
            }
            let addr = format!("{}.{}", r.tf_type, r.label);
            if referenced.contains(&addr) {
                r.reason = Some(format!(
                    "a verbatim block references `{}`, whose emitted label a translated {} does not keep",
                    addr, r.tf_type
                ));
                changed = true;
            }
        }
        if !changed {
            break;
        }
        wrapped_idx = closure(&mut resources);
    }

    // record why each block wrapped BEFORE any retry can rewrite it: a later
    // pass would report "that name is no longer a param", which is true and
    // useless next to the defect this pass just found
    for (i, r) in resources.iter().enumerate() {
        if wrapped_idx.contains(&i) {
            reasons
                .entry((r.file.clone(), r.line))
                .or_insert_with(|| r.reason.clone().unwrap_or_default());
        }
    }

    // A `provider` block stays in the estate, verbatim, when something the
    // estate emits still uses it: a block that stays verbatim and names no
    // provider (or names this one), or a translated resource naming it. The
    // emitter's own providers are all aliased, so without it those blocks would
    // deploy with an unconfigured default provider.
    let mut used: BTreeSet<String> = verbatim_providers;
    for (i, r) in resources.iter().enumerate() {
        let names_one = r.body.as_ref().is_some_and(|b| b.contains_key(serde_yaml::Value::String("provider".into())));
        if wrapped_idx.contains(&i) || names_one {
            used.insert(r.provider_ref.clone());
        }
    }
    let mut provider_texts: Vec<String> = Vec::new();
    for sp in &providers {
        let action = if sp.collides() {
            Action::Dropped(sp.collision_note())
        } else {
            // what the translated resources took from it, on either path
            let took = if sp.name == "google" && sp.alias.is_none() && !carried.is_empty() {
                format!(" — {}", carried.join("; "))
            } else {
                String::new()
            };
            if used.contains(&sp.reference()) || (used.contains("*") && sp.alias.is_none()) {
                verbatim.push((sp.file.clone(), sp.line, sp.text.clone()));
                provider_texts.push(sp.text.clone());
                Action::Wrapped(format!(
                    "carried verbatim: a block that stays verbatim, or a resource naming `{}`, uses its configuration, and the emitter's own providers are aliased{}",
                    sp.reference(),
                    took
                ))
            } else {
                Action::Dropped(format!("{}{}", TERRAFORM_DROPPED, took))
            }
        };
        rows.push(Row { file: sp.file.clone(), line: sp.line, what: sp.what.clone(), action });
    }

    // which declarations must be carried verbatim beside the blocks that read
    // them, and which names have to leave the table because carrying them would
    // declare the same `variable` twice
    let mut read_by_verbatim = verbatim_reads(parsed);
    for text in resources.iter().enumerate().filter(|(i, _)| wrapped_idx.contains(i)).map(|(_, r)| &r.text).chain(&provider_texts) {
        let mut rd = Reads::default();
        rd.scan_text(text);
        read_by_verbatim.merge(rd);
    }
    for d in decls.iter_mut() {
        d.carry = !d.unresolved.is_empty();
    }
    // a carried declaration reads what it reads too — a carried `locals`
    // interpolating a variable needs that variable carried — so this runs
    // until no newly carried declaration reads anything new
    let mut scanned: BTreeSet<usize> = BTreeSet::new();
    loop {
        for read in read_by_verbatim.names(&consts) {
            let Some(c) = consts.by_name.get(&read) else { continue };
            if c.conflict.is_some() {
                // already not a param; its declaration is carried below on its own
                decls[c.decl].carry = true;
                continue;
            }
            // `emit_variables` writes a param as `variable "<name, _ → ->"`. A name
            // that survives that rewrite unchanged would collide with the carried
            // `variable` block of the same name, so it cannot be both.
            if c.origin != Origin::Local && !read.contains('_') {
                return Ok(Pass::Retry(read));
            }
            decls[c.decl].carry = true;
        }
        let mut more = false;
        for (i, d) in decls.iter().enumerate() {
            if (d.carry || d.reason.is_some() || d.promoted.is_empty()) && scanned.insert(i) {
                let mut rd = Reads::default();
                rd.scan_text(&d.text);
                read_by_verbatim.merge(rd);
                more = true;
            }
        }
        if !more {
            break;
        }
    }

    // rows for the declarations
    for d in &decls {
        let action = match (&d.reason, d.promoted.is_empty()) {
            (Some(why), _) => Action::Wrapped(why.clone()),
            (None, true) => Action::Wrapped("it declares nothing satz can promote".into()),
            (None, false) => Action::Promoted(describe_promotion(d, &consts)),
        };
        if d.carry || d.reason.is_some() || d.promoted.is_empty() {
            // a variable bound to the organisation, carried because a verbatim
            // block reads it, is carried with that organisation as its default:
            // the source asked for it, and the estate knows it
            let org = d.promoted.iter().find_map(|n| {
                consts.by_name.get(n).filter(|c| c.origin == Origin::VarRequired).and_then(|c| c.organization.clone())
            });
            let text = match org {
                Some(org) => with_default(&d.text, &org),
                None => d.text.clone(),
            };
            verbatim.push((d.file.clone(), d.line, text));
        }
        rows.push(Row { file: d.file.clone(), line: d.line, what: d.what.clone(), action });
    }

    // rows and verbatim text for the resources
    for (i, r) in resources.iter().enumerate() {
        if wrapped_idx.contains(&i) {
            verbatim.push((r.file.clone(), r.line, r.text.clone()));
            // the first pass's reason is the one that names the real defect
            let why = reasons
                .entry((r.file.clone(), r.line))
                .or_insert_with(|| r.reason.clone().unwrap_or_default())
                .clone();
            rows.push(Row {
                file: r.file.clone(),
                line: r.line,
                what: r.what.clone(),
                action: Action::Wrapped(why),
            });
        } else if !r.expanded {
            rows.push(Row { file: r.file.clone(), line: r.line, what: r.what.clone(), action: Action::Translated });
        }
    }
    rows.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    verbatim.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));

    // the tree
    let translated: Vec<&Res> = resources.iter().enumerate().filter(|(i, _)| !wrapped_idx.contains(i)).map(|(_, r)| r).collect();
    let mut top = base_estate()?;
    if let Some(google) = top
        .get_mut("providers")
        .and_then(|p| p.get_mut("google"))
        .and_then(|g| g.as_mapping_mut())
    {
        google.extend(estate_provider);
    }
    for r in &translated {
        if r.place == Place::Top {
            place_into(&mut top, r, &translated, schema);
        }
    }

    // params: the inferred organisation id, then every promoted constant
    let mut params: Vec<(String, String)> = Vec::new();
    let mut required: Vec<String> = Vec::new();
    for (hcl_name, c) in &consts.by_name {
        if c.conflict.is_some() {
            continue;
        }
        match (&c.value, c.origin) {
            (Some(v), _) => {
                params.push((c.param.clone(), migrate::param_value(v).map_err(|e| e.to_string())?));
            }
            (None, Origin::VarRequired) => required.push(hcl_name.clone()),
            (None, _) => {}
        }
    }

    // header and notes
    let mut header = vec![
        "Imported from existing Terraform. `variable` and `locals` became params; resource".to_string(),
        "blocks whose values are literals, params or `${…}` references are Satz resources,".to_string(),
        "placed by the folder/project they reference. Everything else is carried verbatim".to_string(),
        "inside `hcl trust` (it deploys as written; the compliance plane cannot see into it)".to_string(),
        "— the import report says why, per block. `satz transpile`, then `tofu plan` against".to_string(),
        "the source's state must show no changes; `satz adopt` resolves the import ids after.".to_string(),
        "A `${…}` reference is opaque to the compliance plane: translated is not proven.".to_string(),
    ];
    if !required.is_empty() {
        header.push(String::new());
        header.push("Bind these before transpiling — the source declared them without a default,".to_string());
        header.push("so satz stops with `unknown param` until they are in `params`:".to_string());
        for r in &required {
            header.push(format!("  {}", migrate::param_name(r)));
        }
        notes.push(format!(
            "{} param(s) have no default in the source and must be bound before transpiling: {}",
            required.len(),
            required.iter().map(|r| migrate::param_name(r)).collect::<Vec<_>>().join(", ")
        ));
    }
    let known: BTreeSet<String> =
        translated.iter().map(|r| format!("{}.{}", r.tf_type, r.label)).collect();
    // A `${…}` from a translated block to one that is NOT translated is an
    // estate satz refuses: `hcl trust` is text, the emission manifest holds no
    // address for it, and `satz transpile` reports `written-reference`. Refuse
    // here instead, where both sides can still be named.
    let mut crossings: Vec<String> = Vec::new();
    for r in &translated {
        for address in r.uses.refs.iter().filter(|a| !known.contains(*a)) {
            let other = resources.iter().enumerate().find(|(_, o)| format!("{}.{}", o.tf_type, o.label) == *address);
            let fate = match other {
                Some((i, o)) if wrapped_idx.contains(&i) => format!(
                    ", which stays verbatim inside `hcl trust` ({}:{} — {})",
                    o.file,
                    o.line,
                    reasons.get(&(o.file.clone(), o.line)).cloned().unwrap_or_default()
                ),
                Some(_) => ", which this estate does not emit under that address".to_string(),
                None if declared.contains(address) => {
                    ", which `count` expands into one resource per entry, so that address is gone".to_string()
                }
                None => ", which no block in this import declares".to_string(),
            };
            crossings.push(format!(
                "  {}:{} `{}.{}` references `{}`{}",
                r.file, r.line, r.tf_type, r.label, address, fate
            ));
        }
    }
    if !crossings.is_empty() {
        crossings.sort();
        return Err(format!(
            "the import would write an estate `satz transpile` refuses, so nothing was written: a translated resource references an address this estate does not emit.\n{}\nEither make the referenced block translatable (its reason is above), import the file that declares it too, or carry everything verbatim with `--wrap-all`.",
            crossings.join("\n")
        ));
    }
    // a block the closure wrapped afterwards kept its `depends_on` in its text
    dropped_edges.retain(|(from, _)| known.contains(from));
    dropped_edges.sort();
    if !dropped_edges.is_empty() {
        notes.push(format!(
            "{} translated block(s) no longer write the `depends_on` they carried — satz derives a plan's ordering from the estate itself; `--verbose` lists them",
            dropped_edges.len()
        ));
    }

    // the organisation last among the refusals: a reference across the verbatim
    // boundary says more about the input than a missing `--organization` does
    let org = bound_organization(&org_ids, organization)?;
    params.insert(
        0,
        ("customer_organization_id".to_string(), migrate::param_value(&serde_yaml::Value::String(org)).map_err(|e| e.to_string())?),
    );
    let satz = render(&top, name, &params, &header, &verbatim)?;
    Ok(Pass::Done(Box::new(Imported { satz, rows, notes, ordering_dropped: dropped_edges })))
}

/// The organisation an imported estate is bound to: the one the configuration
/// names (a literal `organizations/<n>` parent or `org_id`, an org policy's
/// parent), or `--organization` when it names none — never both disagreeing, never
/// two, never none. The same refusals as the state shape's: an estate written
/// without one carries `organizations/` and `""` where an id belongs.
fn bound_organization(found: &BTreeSet<String>, flag: Option<&str>) -> Result<String, String> {
    let named: Vec<&String> = found.iter().collect();
    match (named.as_slice(), flag) {
        ([], None) => Err("import: no organization id — nothing in the configuration names one (a literal \
             `organizations/<n>` parent, an `org_id`, an org policy's parent), and a folder's parent and an \
             organization grant's `org_id` are written from `customer_organization_id`. Nothing written. Name it \
             with `--organization <n>`."
            .to_string()),
        ([], Some(flag)) => Ok(flag.to_string()),
        ([one], None) => Ok((*one).clone()),
        ([one], Some(flag)) if *one == flag => Ok(flag.to_string()),
        ([one], Some(flag)) => Err(format!(
            "--organization {} but the configuration names organization {} — one of the two is wrong, and satz does not pick. Nothing written.",
            flag, one
        )),
        (many, _) => Err(format!(
            "import: the configuration names {} organizations ({}), and an estate is bound to one — import each organisation's files on their own. Nothing written.",
            many.len(),
            many.iter().map(|o| o.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn base_estate() -> Result<serde_yaml::Mapping, String> {
    let mut top = serde_yaml::Mapping::new();
    top.insert("terraform".into(), serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n").map_err(|e: serde_yaml::Error| e.to_string())?);
    top.insert(
        "providers".into(),
        serde_yaml::from_str("google:\n  alias: google\ngoogle-beta:\n  alias: google-beta\n").map_err(|e: serde_yaml::Error| e.to_string())?,
    );
    Ok(top)
}

fn render(
    top: &serde_yaml::Mapping,
    name: &str,
    params: &[(String, String)],
    header: &[String],
    verbatim: &[(String, usize, String)],
) -> Result<String, String> {
    let mut satz = migrate::convert_value(top, "estate", &sanitize_name(name), params, header).map_err(|e| e.to_string())?;
    if !satz.ends_with("\n\n") {
        satz.push('\n');
    }
    for (path, line, text) in verbatim {
        satz.push_str(&wrap_block(path, *line, text));
    }
    Ok(satz)
}

/// `--wrap-all`: every block verbatim except the two the emitter owns.
fn wrap_everything(parsed: &[(&Input, Body)], name: &str, organization: Option<&str>) -> Result<Imported, String> {
    // nothing is translated, so nothing names an organisation but the flag
    let org = organization.ok_or(
        "import: no organization id — --wrap-all translates nothing, so nothing in the configuration names one, and every \
         estate is bound to one: a folder's parent and an organization grant's `org_id` are written from \
         `customer_organization_id`. Nothing written. Name it with `--organization <n>`.",
    )?;
    let params = vec![(
        "customer_organization_id".to_string(),
        migrate::param_value(&serde_yaml::Value::String(org.to_string())).map_err(|e| e.to_string())?,
    )];
    let mut rows = Vec::new();
    let mut verbatim = Vec::new();
    // what the estate deploys differently from the source
    let mut changed: Vec<String> = Vec::new();
    for (input, body) in parsed {
        for s in body.iter() {
            let Some(span) = s.span() else {
                return Err(format!("{}: a structure without a span (not parsed from text?)", input.path));
            };
            let text = input.text[span.clone()].trim_end().to_string();
            let line = input.text[..span.start].matches('\n').count() + 1;
            let what = describe(s);
            if let Some(b) = s.as_block() {
                let ident = b.ident.to_string();
                if ident == "terraform" {
                    rows.push(Row { file: input.path.clone(), line, what, action: Action::Dropped(TERRAFORM_DROPPED.into()) });
                    continue;
                }
                // every block is verbatim, so every provider configuration is
                // still used — except one the estate declares itself
                if ident == "provider" {
                    let sp = SourceProvider::new(input.path.clone(), line, what, text, b);
                    if sp.collides() {
                        let note = sp.collision_note();
                        changed.push(format!("{}:{} {}: {}", sp.file, sp.line, sp.what, note));
                        rows.push(Row { file: sp.file, line: sp.line, what: sp.what, action: Action::Dropped(note) });
                        continue;
                    }
                    verbatim.push((sp.file.clone(), sp.line, sp.text));
                    rows.push(Row {
                        file: sp.file,
                        line: sp.line,
                        what: sp.what,
                        action: Action::Wrapped("--wrap-all: carried verbatim, the verbatim blocks use its configuration".into()),
                    });
                    continue;
                }
            }
            verbatim.push((input.path.clone(), line, text));
            rows.push(Row { file: input.path.clone(), line, what, action: Action::Wrapped("--wrap-all".into()) });
        }
    }
    rows.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    let mut header = vec![
        "Imported from existing Terraform with --wrap-all: every block is carried verbatim".to_string(),
        "inside `hcl trust`, its `provider` blocks included, nothing is translated and no".to_string(),
        "params are promoted beyond the organisation `--organization` names. The emitter".to_string(),
        "writes the `terraform` block.".to_string(),
    ];
    if changed.is_empty() {
        header.push("It deploys as written; `tofu plan` against the source's state must show no changes.".to_string());
    } else {
        header.push("It deploys as written except where a `provider` block is not carried:".to_string());
        header.extend(changed.iter().map(|c| format!("  {}", c)));
    }
    let satz = render(&base_estate()?, name, &params, &header, &verbatim)?;
    let notes = changed.into_iter().map(|c| format!("not carried: {}", c)).collect();
    Ok(Imported { satz, rows, notes, ordering_dropped: Vec::new() })
}

/// Closure by dependency, recomputed from the current reasons.
fn closure(resources: &mut [Res]) -> BTreeSet<usize> {
    let mut wrapped: BTreeSet<usize> =
        resources.iter().enumerate().filter(|(_, r)| r.reason.is_some()).map(|(i, _)| i).collect();
    loop {
        let mut changed = false;
        for i in 0..resources.len() {
            if wrapped.contains(&i) {
                continue;
            }
            let parent = match &resources[i].place {
                Place::Top => None,
                Place::Folder(l) => resources.iter().position(|r| r.tf_type == "google_folder" && &r.label == l),
                Place::Project(l) => resources.iter().position(|r| r.tf_type == "google_project" && &r.label == l),
            };
            match (&resources[i].place, parent) {
                (Place::Top, _) => {}
                (place, None) => {
                    resources[i].reason =
                        Some(format!("its parent {:?} is not among the imported resources", place_name(place)));
                    wrapped.insert(i);
                    changed = true;
                }
                (_, Some(p)) if wrapped.contains(&p) => {
                    // the reason that STARTED the chain: "its parent is wrapped"
                    // sends the reviewer one hop, and the hop repeats the sentence
                    let (root_what, root_why) = root_cause(resources, p);
                    resources[i].reason = Some(if root_what == resources[p].what {
                        format!("its parent {} is wrapped: {}", root_what, root_why)
                    } else {
                        format!("its parent {} is wrapped, because {} is: {}", resources[p].what, root_what, root_why)
                    });
                    wrapped.insert(i);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            return wrapped;
        }
    }
}

/// The block at the start of a chain of wrapped parents, and its own reason.
fn root_cause(resources: &[Res], mut i: usize) -> (String, String) {
    for _ in 0..resources.len() {
        let parent = match &resources[i].place {
            Place::Top => None,
            Place::Folder(l) => resources.iter().position(|r| r.tf_type == "google_folder" && &r.label == l),
            Place::Project(l) => resources.iter().position(|r| r.tf_type == "google_project" && &r.label == l),
        };
        match parent {
            Some(p) if resources[p].reason.is_some() && resources[i].reason.as_deref().is_some_and(|r| r.starts_with("its parent ")) => i = p,
            _ => break,
        }
    }
    (resources[i].what.clone(), resources[i].reason.clone().unwrap_or_default())
}

/// Types whose translated form does not keep the source label: a project's
/// services become a bare list, a grant map's entries get a hashed label.
fn derives_label(tf_type: &str, schema: &dyn Schema) -> bool {
    tf_type == "google_project_service" || is_map_form(&grant_form_of(schema, tf_type))
}

/// The projects this input declares, by their literal `project_id` — what a
/// resource naming its project by id (as `gcloud … bulk-export` and
/// `tofu plan -generate-config-out` write it) is placed under.
fn project_ids(parsed: &[(&Input, Body)], consts: &Consts) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_, body) in parsed {
        for s in body.iter() {
            let Some(b) = s.as_block() else { continue };
            if b.ident.to_string() != "resource" {
                continue;
            }
            let [t, l] = b.labels.as_slice() else { continue };
            if t.as_str() != "google_project" {
                continue;
            }
            if let Some(id) = attr_string(b, "project_id", consts) {
                out.insert(id, l.as_str().to_string());
            }
        }
    }
    out
}

fn describe_promotion(d: &Decl, consts: &Consts) -> String {
    let mut out = if d.promoted.len() == 1 {
        let n = &d.promoted[0];
        match consts.by_name.get(n) {
            Some(Const { value: Some(v), origin, organization, .. }) => format!(
                "param {} = {} ({})",
                migrate::param_name(n),
                migrate::param_value(v).unwrap_or_else(|_| "…".into()),
                match (origin, organization) {
                    (Origin::VarRequired, Some(_)) => {
                        "variable without a default, used as the organisation: bound to the one `--organization` names"
                    }
                    (_, Some(_)) => "used as the organisation, which its value names",
                    (Origin::VarDefault, None) => "variable default",
                    (Origin::Local, None) => "locals",
                    (Origin::VarRequired, None) => "variable",
                }
            ),
            _ => format!("param {} (variable without a default — bind it before deploying)", migrate::param_name(n)),
        }
    } else {
        format!(
            "params {}",
            d.promoted.iter().map(|n| migrate::param_name(n)).collect::<Vec<_>>().join(", ")
        )
    };
    if !d.unresolved.is_empty() {
        out.push_str(&format!(
            " — carried verbatim too, {} is not a literal",
            d.unresolved.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
        ));
    } else if d.carry {
        let bound = d.promoted.iter().find_map(|n| {
            consts.by_name.get(n).filter(|c| c.origin == Origin::VarRequired).and_then(|c| c.organization.as_deref())
        });
        match bound {
            Some(org) => out.push_str(&format!(
                " — carried verbatim too, with `default = \"{}\"`, a wrapped block still reads it",
                org
            )),
            None => out.push_str(" — carried verbatim too, a wrapped block still reads it"),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the pre-pass
// ---------------------------------------------------------------------------

/// Collect the constants `variable` and `locals` declare. Names are gathered
/// first so a local may reference any other constant regardless of order (which
/// is the language's own guarantee for params), then values are computed.
fn collect_consts(
    parsed: &[(&Input, Body)],
    forced: &BTreeSet<String>,
) -> Result<(Consts, Vec<Decl>), String> {
    struct Pending {
        name: String,
        origin: Origin,
        expr: Option<Expression>,
        decl: usize,
    }
    let mut decls: Vec<Decl> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();

    for (input, body) in parsed {
        for s in body.iter() {
            let Some(block) = s.as_block() else { continue };
            let ident = block.ident.to_string();
            if ident != "variable" && ident != "locals" {
                continue;
            }
            let Some(span) = s.span() else { continue };
            let text = input.text[span.clone()].trim_end().to_string();
            let line = input.text[..span.start].matches('\n').count() + 1;
            let idx = decls.len();
            let mut decl = Decl {
                file: input.path.clone(),
                line,
                what: describe(s),
                text,
                promoted: Vec::new(),
                unresolved: Vec::new(),
                reason: None,
                carry: false,
            };
            if ident == "variable" {
                let Some(vname) = block.labels.first().map(|l| l.as_str().to_string()) else {
                    decl.reason = Some("a `variable` block needs a name".into());
                    decls.push(decl);
                    continue;
                };
                let default = block.body.iter().find_map(|s| match s {
                    Structure::Attribute(a) if a.key.to_string() == "default" => Some(a.value.clone()),
                    _ => None,
                });
                let origin = if default.is_some() { Origin::VarDefault } else { Origin::VarRequired };
                pending.push(Pending { name: vname, origin, expr: default, decl: idx });
            } else {
                for s in block.body.iter() {
                    match s {
                        Structure::Attribute(a) => pending.push(Pending {
                            name: a.key.to_string(),
                            origin: Origin::Local,
                            expr: Some(a.value.clone()),
                            decl: idx,
                        }),
                        Structure::Block(b) => {
                            decl.reason = Some(format!("`locals` holds a `{}` block, not values", b.ident));
                        }
                    }
                }
            }
            decls.push(decl);
        }
    }

    // pass 1: the namespace, with collisions recorded rather than resolved
    let mut consts = Consts::default();
    for p in &pending {
        let param = migrate::param_name(&p.name);
        let clash = consts.by_name.iter().find(|(n, c)| *n != &p.name && c.param == param).map(|(n, _)| n.clone());
        let entry = Const {
            param: param.clone(),
            value: None,
            origin: p.origin,
            decl: p.decl,
            conflict: if forced.contains(&p.name) {
                Some(format!(
                    "`{}` is read by a block that stays verbatim, so it stays a Terraform variable",
                    p.name
                ))
            } else {
                None
            },
            expr: p.expr.clone(),
            organization: None,
        };
        match consts.by_name.get_mut(&p.name) {
            Some(existing) => {
                existing.conflict = Some(format!(
                    "`{}` is declared twice in this import (as a `variable` and in `locals`, or twice over) — satz has one `params` namespace",
                    p.name
                ));
            }
            None => {
                let mut e = entry;
                if let Some(other) = clash {
                    e.conflict = Some(format!(
                        "`{}` and `{}` both become the param `{}` — satz has one `params` namespace",
                        p.name, other, param
                    ));
                    if let Some(o) = consts.by_name.get_mut(&other) {
                        o.conflict = e.conflict.clone();
                    }
                }
                consts.by_name.insert(p.name.clone(), e);
            }
        }
    }

    // pass 2: values. A local may reference any other constant.
    let names_only = Consts {
        by_name: consts
            .by_name
            .iter()
            .map(|(n, c)| (n.clone(), Const { value: Some(serde_yaml::Value::Null), ..c.clone() }))
            .collect(),
    };
    let all = Everything;
    for p in &pending {
        let Some(expr) = &p.expr else { continue };
        let unusable = consts.by_name.get(&p.name).is_some_and(|c| c.conflict.is_some());
        if unusable {
            decls[p.decl].unresolved.push(p.name.clone());
            continue;
        }
        let mut cx = Cx { consts: &names_only, schema: &all, uses: Uses::default(), index: None };
        match cx.literal(expr) {
            Ok(v) => {
                if let Some(c) = consts.by_name.get_mut(&p.name) {
                    c.value = Some(v);
                }
                decls[p.decl].promoted.push(p.name.clone());
            }
            Err(_) => {
                consts.by_name.remove(&p.name);
                decls[p.decl].unresolved.push(p.name.clone());
            }
        }
    }
    for p in &pending {
        if p.origin == Origin::VarRequired && consts.by_name.contains_key(&p.name) {
            decls[p.decl].promoted.push(p.name.clone());
        }
    }
    for d in decls.iter_mut() {
        if d.promoted.is_empty() && d.reason.is_none() && !d.unresolved.is_empty() {
            d.reason = Some(format!(
                "{} is not a literal, a param or a `${{…}}` reference",
                d.unresolved.iter().map(|n| format!("`{}`", n)).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok((consts, decls))
}

/// A schema that knows every type — used only while folding constants, where
/// a `${…}` reference is carried verbatim whatever it points at.
struct Everything;
impl Schema for Everything {
    fn has_type(&self, _: &str) -> bool {
        true
    }
    fn has_attr(&self, _: &str, _: &str) -> bool {
        false
    }
    fn required_attrs(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// One attribute of a block as a plain string, folding `var.`/`local.` and
/// templates over them ([`Consts::eval`]).
fn attr_string(block: &Block, key: &str, consts: &Consts) -> Option<String> {
    consts.eval(attr_expr(block, key)?, 0)
}

/// One attribute's expression, as written.
fn attr_expr<'b>(block: &'b Block, key: &str) -> Option<&'b Expression> {
    block.body.iter().find_map(|s| match s {
        Structure::Attribute(a) if a.key.to_string() == key => Some(&a.value),
        _ => None,
    })
}

/// `var.x` / `local.x` folded to a plain string.
fn const_string(t: &Traversal, consts: &Consts) -> Option<String> {
    let (_, name) = const_name(t)?;
    consts.literal_string(&name)
}

/// A `provider` block of the input: which configuration it is, and its text.
struct SourceProvider {
    file: String,
    line: usize,
    what: String,
    text: String,
    /// `google`, `google-beta`
    name: String,
    alias: Option<String>,
    block: Block,
}

impl SourceProvider {
    fn new(file: String, line: usize, what: String, text: String, block: &Block) -> Self {
        let name = block.labels.first().map(|l| l.as_str().to_string()).unwrap_or_default();
        let alias = attr_expr(block, "alias").and_then(|e| match e {
            Expression::String(s) => Some(s.value().to_string()),
            _ => None,
        });
        SourceProvider { file, line, what, text, name, alias, block: block.clone() }
    }

    /// How a resource names this configuration: `google`, `google.<alias>`.
    fn reference(&self) -> String {
        match &self.alias {
            Some(a) => format!("{}.{}", self.name, a),
            None => self.name.clone(),
        }
    }

    /// The estate declares this configuration itself (`base_estate`), so a
    /// second block of the same name and alias cannot stand beside it.
    fn collides(&self) -> bool {
        matches!(self.name.as_str(), "google" | "google-beta") && self.alias.as_deref() == Some(self.name.as_str())
    }

    /// What a colliding block takes with it: the resources naming it get the
    /// estate's provider, which does not carry the source's settings.
    fn collision_note(&self) -> String {
        let settings: Vec<String> = self
            .block
            .body
            .iter()
            .filter_map(|s| s.as_attribute().map(|a| a.key.to_string()))
            .filter(|k| k != "alias")
            .collect();
        let mut why = format!(
            "the estate declares `{}` itself and the emitter writes it; a resource naming it gets the estate's provider",
            self.reference()
        );
        if !settings.is_empty() {
            why.push_str(&format!(
                ", which does not set this block's {} — set them in the estate's `providers` block",
                settings.iter().map(|k| format!("`{}`", k)).collect::<Vec<_>>().join(", ")
            ));
        }
        why
    }
}

/// The provider configuration a `resource` or `data` block uses: the one its
/// `provider` names, else the default configuration of its type's provider
/// (`google_storage_bucket` → `google`).
fn provider_of(block: &Block, tf_type: &str) -> String {
    match attr_expr(block, "provider") {
        Some(Expression::String(s)) => s.value().to_string(),
        Some(e) => provider_reference(e).unwrap_or_else(|_| e.to_string().trim().to_string()),
        None => tf_type.split('_').next().unwrap_or(tf_type).to_string(),
    }
}

/// The source's default `google` provider set a project, region or zone that a
/// translated resource naming no provider relied on. The emitter writes such a
/// resource with the estate's own aliased provider, so each setting is carried
/// where the estate can say it: the project as placement (the resource moves
/// into the imported project of that id) or, when no imported project has it,
/// as the resource's own `project`; the region and zone into the estate's
/// `providers` block, which the per-project aliases take theirs from. A setting
/// satz cannot write wraps each resource that relied on it, naming it. Returns
/// what was carried, for the provider's row.
fn carry_provider_defaults(
    sp: &SourceProvider,
    consts: &Consts,
    schema: &dyn Schema,
    resources: &mut [Res],
    estate_provider: &mut serde_yaml::Mapping,
) -> Vec<String> {
    let key = |s: &str| serde_yaml::Value::String(s.to_string());
    let relies = |r: &Res, attr: &str| {
        r.reason.is_none()
            && !r.body.as_ref().is_some_and(|b| b.contains_key(key("provider")) || b.contains_key(key(attr)))
            && schema.has_attr(&r.tf_type, attr)
    };
    let writable = |e: &Expression| -> Result<serde_yaml::Value, String> {
        let mut cx = Cx { consts, schema, uses: Uses::default(), index: None };
        let v = cx.literal(e)?;
        if !cx.uses.refs.is_empty() {
            return Err("a reference to a resource".into());
        }
        Ok(v)
    };
    let mut carried = Vec::new();
    if let Some(expr) = attr_expr(&sp.block, "project") {
        let shown = expr.to_string().trim().to_string();
        let default = consts.eval(expr, 0);
        let host = default.as_ref().and_then(|d| {
            resources
                .iter()
                .find(|r| r.tf_type == "google_project" && r.reason.is_none() && r.project_id.as_ref() == Some(d))
                .map(|r| r.label.clone())
        });
        let value = writable(expr);
        let (mut placed, mut given) = (0, 0);
        for r in resources.iter_mut() {
            if r.place != Place::Top || matches!(r.tf_type.as_str(), "google_project" | "google_folder") || !relies(r, "project") {
                continue;
            }
            match (&host, &value) {
                (Some(h), _) => {
                    r.place = Place::Project(h.clone());
                    placed += 1;
                }
                (None, Ok(v)) => {
                    if let Some(b) = r.body.as_mut() {
                        b.insert(key("project"), v.clone());
                    }
                    given += 1;
                }
                (None, Err(why)) => {
                    r.reason = Some(format!(
                        "it names no `project` and relies on the provider's default project `{}`, which satz cannot write: {}",
                        shown, why
                    ))
                }
            }
        }
        if placed > 0 {
            carried.push(format!(
                "its default project {} is carried as placement: {} resource(s) moved into google_project.{}",
                shown,
                placed,
                host.as_deref().unwrap_or_default()
            ));
        }
        if given > 0 {
            carried.push(format!("its default project {} is carried as the `project` of {} resource(s)", shown, given));
        }
    }
    for attr in ["region", "zone"] {
        let Some(expr) = attr_expr(&sp.block, attr) else { continue };
        let shown = expr.to_string().trim().to_string();
        match writable(expr) {
            Ok(v) => {
                estate_provider.insert(key(attr), v);
                carried.push(format!("its {} {} is carried into the estate's `providers` block", attr, shown));
            }
            Err(why) => {
                for r in resources.iter_mut().filter(|r| relies(r, attr)) {
                    r.reason = Some(format!(
                        "it names no `{}` and relies on the provider's default `{}`, which satz cannot write: {}",
                        attr, shown, why
                    ));
                }
            }
        }
    }
    carried
}

/// Bind every constant the configuration uses as the organisation — `org_id =
/// var.x` on any resource, `parent = "organizations/${var.x}"` — to the
/// organisation the estate is bound to: a variable declared without a default
/// to the one `--organization` names, any other constant to the number it
/// folds to. Its param is then written as `customer_organization_id`, so the
/// estate names the organisation once, and a placement reads the number.
/// Returns the organisations a constant's own value names, which bind the
/// estate the way a literal `org_id` does.
fn bind_organization_variables(
    parsed: &[(&Input, Body)],
    consts: &mut Consts,
    organization: Option<&str>,
) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (_, body) in parsed {
        for s in body.iter() {
            let Some(b) = s.as_block() else { continue };
            if b.ident.to_string() != "resource" {
                continue;
            }
            for st in b.body.iter() {
                let Structure::Attribute(a) = st else { continue };
                let name = match (a.key.to_string().as_str(), &a.value) {
                    ("org_id", Expression::Traversal(t)) => const_name(t),
                    ("parent", e) => organizations_parent(e),
                    _ => None,
                };
                if let Some((_, n)) = name {
                    used.insert(n);
                }
            }
        }
    }
    let mut named = BTreeSet::new();
    for n in used {
        let org = match consts.by_name.get(&n) {
            None => continue,
            Some(c) if c.origin == Origin::VarRequired => match organization {
                Some(flag) => flag.to_string(),
                None => continue,
            },
            Some(_) => {
                let Some(v) = consts.literal_string(&n) else { continue };
                let digits = v.strip_prefix("organizations/").unwrap_or(&v).to_string();
                if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                named.insert(digits.clone());
                digits
            }
        };
        if let Some(c) = consts.by_name.get_mut(&n) {
            c.value = Some(migrate::param_ref(ORGANIZATION_PARAM));
            c.organization = Some(org);
        }
    }
    named
}

/// `"organizations/${var.x}"`: the constant a parent written that way names.
fn organizations_parent(e: &Expression) -> Option<(String, String)> {
    let Expression::StringTemplate(t) = e else { return None };
    match t.iter().collect::<Vec<_>>().as_slice() {
        [Element::Literal(l), Element::Interpolation(i)] if l.value() == "organizations/" => match &i.expr {
            Expression::Traversal(t) => const_name(t),
            _ => None,
        },
        _ => None,
    }
}

/// A carried `variable` block with a `default` added before its closing brace.
fn with_default(text: &str, value: &str) -> String {
    let (Some(open), Some(close)) = (text.find('{'), text.rfind('}')) else { return text.to_string() };
    let inner = text[open + 1..close].trim();
    if inner.contains('\n') {
        return format!("{}\n  default = {:?}\n}}", text[..close].trim_end(), value);
    }
    // a one-line block opens up, so the added line stands on its own
    let body = if inner.is_empty() { String::new() } else { format!("  {}\n", inner) };
    format!("{}{{\n{}  default = {:?}\n}}", &text[..open], body, value)
}

/// A list entry as the report shows it.
fn scalar_text(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

// ---------------------------------------------------------------------------
// reading references back out of verbatim text
// ---------------------------------------------------------------------------

/// The `var.`/`local.` names and `<type>.<label>` addresses a block reads.
#[derive(Default)]
struct Reads {
    vars: BTreeSet<String>,
    locals: BTreeSet<String>,
    addrs: BTreeSet<String>,
    /// a `var[expr]` lookup: which names are meant cannot be known, so every
    /// constant of that kind counts as read
    dynamic: bool,
}

impl Reads {
    fn merge(&mut self, other: Reads) {
        self.vars.extend(other.vars);
        self.locals.extend(other.locals);
        self.addrs.extend(other.addrs);
        self.dynamic |= other.dynamic;
    }

    /// Parse a verbatim block's own text and collect what it reads. The text is
    /// exactly what will be emitted, so this is the honest question to ask.
    fn scan_text(&mut self, text: &str) {
        if let Ok(body) = parse_body(text) {
            visit::visit_body(self, &body);
        } else {
            // unparsable in isolation (it parsed as part of its file, so this
            // means a fragment): be conservative
            self.dynamic = true;
        }
    }

    /// Every constant name this reader pins.
    fn names(&self, consts: &Consts) -> Vec<String> {
        if self.dynamic {
            return consts.by_name.keys().cloned().collect();
        }
        self.vars.iter().chain(self.locals.iter()).cloned().collect()
    }
}

impl Visit for Reads {
    fn visit_traversal(&mut self, node: &Traversal) {
        if let Expression::Variable(v) = &node.expr {
            let root = v.as_str();
            if root == "var" || root == "local" {
                match node.operators.first().map(|o| o.value()) {
                    Some(TraversalOperator::GetAttr(k)) => {
                        let n = k.as_str().to_string();
                        if root == "var" {
                            self.vars.insert(n);
                        } else {
                            self.locals.insert(n);
                        }
                    }
                    _ => self.dynamic = true,
                }
            } else if node.operators.len() >= 2 {
                if let Some(TraversalOperator::GetAttr(l)) = node.operators.first().map(|o| o.value()) {
                    self.addrs.insert(format!("{}.{}", root, l.as_str()));
                }
            }
        }
        visit::visit_traversal(self, node);
    }
}

/// What the blocks that are always verbatim — everything that is not a
/// `resource`, `variable`, `locals`, `terraform` or `provider` — read.
fn verbatim_reads(parsed: &[(&Input, Body)]) -> Reads {
    let mut reads = Reads::default();
    for (_, body) in parsed {
        for s in body.iter() {
            let keep = match s.as_block() {
                Some(b) => !matches!(b.ident.to_string().as_str(), "resource" | "variable" | "locals" | "terraform" | "provider"),
                None => true,
            };
            if keep {
                visit::visit_structure(&mut reads, s);
            }
        }
    }
    reads
}

// ---------------------------------------------------------------------------
// placement into the tree
// ---------------------------------------------------------------------------

fn place_name(p: &Place) -> String {
    match p {
        Place::Top => "the organisation".into(),
        Place::Folder(l) => format!("google_folder.{}", l),
        Place::Project(l) => format!("google_project.{}", l),
    }
}

/// Put `r` into `into` (a folder body, a project body, or the top level),
/// with its own children placed under it.
/// One grant edge as the member map holds it: the member key and the roles
/// entry — the bare role, or the object form carrying a `condition`.
fn grant_edge(body: &serde_yaml::Mapping) -> (serde_yaml::Value, serde_yaml::Value) {
    let key = |s: &str| serde_yaml::Value::String(s.to_string());
    // `classify` guarantees a resolvable `member` and `role`; a `condition`
    // rides along in the object form — dropping it would widen the grant
    let member = body.get("member").cloned().expect("classified iam member has a member");
    let role = body.get("role").cloned().expect("classified iam member has a role");
    let entry = match body.get("condition") {
        Some(cond) => {
            let mut o = serde_yaml::Mapping::new();
            o.insert(key("role"), role);
            // an HCL block is a list of one object; the grant form takes the object
            let cond = match cond {
                serde_yaml::Value::Sequence(items) if items.len() == 1 => items[0].clone(),
                other => other.clone(),
            };
            o.insert(key("condition"), cond);
            serde_yaml::Value::Mapping(o)
        }
        None => role,
    };
    (member, entry)
}

fn push_edge(members: &mut serde_yaml::Mapping, member: serde_yaml::Value, entry: serde_yaml::Value) {
    let roles = members.entry(member).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
    if let serde_yaml::Value::Sequence(seq) = roles {
        if !seq.contains(&entry) {
            seq.push(entry);
        }
    }
}

fn place_into(into: &mut serde_yaml::Mapping, r: &Res, all: &[&Res], schema: &dyn Schema) {
    let mut body = r.body.clone().unwrap_or_default();
    let key = |s: &str| serde_yaml::Value::String(s.to_string());
    let form = grant_form_of(schema, &r.tf_type);
    match r.tf_type.as_str() {
        "google_folder" => {
            for c in all.iter().filter(|c| c.place == Place::Folder(r.label.clone())) {
                place_into(&mut body, c, all, schema);
            }
            let m = into.entry(key("google_folder")).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
        "google_project" => {
            for c in all.iter().filter(|c| c.place == Place::Project(r.label.clone())) {
                place_into(&mut body, c, all, schema);
            }
            let m = into.entry(key("google_project")).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
        "google_project_service" => {
            // `classify` guarantees a resolvable `service`
            let svc = body.get("service").cloned().expect("classified project_service has a service");
            let list = into.entry(key("project_service")).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
            if let serde_yaml::Value::Sequence(seq) = list {
                seq.push(svc);
            }
        }
        t if matches!(form, GrantForm::Org | GrantForm::Node) => {
            let (member, entry) = grant_edge(&body);
            let m = into.entry(key(t)).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                push_edge(m, member, entry);
            }
        }
        t if matches!(form, GrantForm::Pinned(_)) => {
            // one map per scope value, pinned by the type's scope attribute
            // (`bucket = …`); the maps travel as a list under the type key
            let GrantForm::Pinned(attr) = form else { unreachable!("matched a pinned form") };
            let scope = body.get(attr.as_str()).cloned().expect("classified pinned grant has its scope");
            let (member, entry) = grant_edge(&body);
            let maps = into.entry(key(t)).or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
            if let serde_yaml::Value::Sequence(maps) = maps {
                let at = maps.iter().position(|m| m.as_mapping().and_then(|m| m.get(attr.as_str())) == Some(&scope));
                let map = match at {
                    Some(i) => &mut maps[i],
                    None => {
                        let mut m = serde_yaml::Mapping::new();
                        m.insert(key(&attr), scope);
                        maps.push(serde_yaml::Value::Mapping(m));
                        maps.last_mut().expect("just pushed")
                    }
                };
                if let Some(m) = map.as_mapping_mut() {
                    push_edge(m, member, entry);
                }
            }
        }
        t => {
            let m = into.entry(key(t)).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let serde_yaml::Value::Mapping(m) = m {
                m.insert(key(&r.label), serde_yaml::Value::Mapping(body));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// classification
// ---------------------------------------------------------------------------

struct Classified {
    body: Option<serde_yaml::Mapping>,
    place: Place,
    reason: Option<String>,
}

/// The forms of `count` that expand, as the report names them.
const COUNT_FORMS: &str = "only `count = length(<list>)` over a promoted list of scalars expands, each entry read as `<list>[count.index]` or `element(<list>, count.index)`";

/// `count = length(var.admins)` over a list this import promoted: the list's
/// name and its elements. Terraform's own idiom for "one of these per entry",
/// and in Satz it IS one resource per entry — so the block expands instead of
/// being carried verbatim. Only this shape: the count must be the length of one
/// promoted list of scalars, and every `count.index` in the block must index
/// THAT list, as `<list>[count.index]` or `element(<list>, count.index)` (a
/// `count.index` anywhere else leaves the block wrapped rather than
/// half-translated). `None` when the block has no `count`; the `Err` is why the
/// one it has does not expand.
fn count_expansion(block: &Block, consts: &Consts) -> Option<Result<(String, Vec<serde_yaml::Value>), String>> {
    let count = attr_expr(block, "count")?;
    let shown = count.to_string().trim().to_string();
    let not_this = || Err(format!("uses `count = {}`, which does not expand — {}", shown, COUNT_FORMS));
    let Expression::FuncCall(call) = count else { return Some(not_this()) };
    // `length(...)`, not a namespaced function of the same name
    if !call.name.namespace.is_empty() || call.name.name.as_str() != "length" {
        return Some(not_this());
    }
    let [Expression::Traversal(t)] = call.args.iter().collect::<Vec<_>>()[..] else { return Some(not_this()) };
    let Some((_, name)) = const_name(t) else { return Some(not_this()) };
    let Some(c) = consts.by_name.get(&name) else {
        return Some(Err(format!("uses `count = {}`, and this import does not declare `{}`", shown, name)));
    };
    if let Some(why) = &c.conflict {
        return Some(Err(format!("uses `count = {}`, and {}", shown, why)));
    }
    Some(match c.value.as_ref() {
        None => Err(format!(
            "uses `count = {}` over `{}`, a variable declared without a default — the entries are not in the input",
            shown, name
        )),
        Some(serde_yaml::Value::Sequence(items)) if !items.is_empty() => {
            // a list of scalars: an element that is itself a list or a map has no
            // single value to substitute into an attribute
            if items.iter().all(|i| i.is_string() || i.is_number() || i.is_bool()) {
                Ok((name, items.clone()))
            } else {
                Err(format!("uses `count = {}` over `{}`, whose entries are not all scalars", shown, name))
            }
        }
        Some(_) => Err(format!("uses `count = {}` over `{}`, which is not a non-empty list", shown, name)),
    })
}

/// The label one expanded copy takes: the element when it can be a Satz
/// identifier (`roles/viewer` -> `viewer`, `europe-west3` -> `europe_west3`),
/// else the index — a label is a name in the estate, and a reader should
/// recognise which entry it is.
fn expanded_label(base: &str, element: &serde_yaml::Value, i: usize) -> String {
    let raw = match element {
        serde_yaml::Value::String(s) => s.rsplit('/').next().unwrap_or(s).to_string(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    };
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    let suffix = if cleaned.is_empty() || cleaned.starts_with(|c: char| c.is_ascii_digit()) {
        i.to_string()
    } else {
        cleaned
    };
    format!("{}_{}", base, suffix)
}

fn wrapped(reason: String) -> Classified {
    Classified { body: None, place: Place::Top, reason: Some(reason) }
}

/// `provider = google.beta` / `provider = google-beta` as the alias Satz writes
/// in the body. Only the reference forms: the quoted one is Terraform's pre-0.12
/// legacy spelling, and rewriting it into a reference would change what the
/// block emits.
fn provider_reference(e: &Expression) -> Result<String, String> {
    let (root, segs) = match e {
        Expression::Variable(v) => (v.as_str().to_string(), Vec::new()),
        Expression::Traversal(t) => {
            let Expression::Variable(v) = &t.expr else {
                return Err(format!("`{}`, which is not a provider reference", t.expr.to_string().trim()));
            };
            let mut segs = Vec::new();
            for op in t.operators.iter() {
                match op.value() {
                    TraversalOperator::GetAttr(k) => segs.push(k.as_str().to_string()),
                    _ => return Err(format!("`{}`, which is not a provider reference", e.to_string().trim())),
                }
            }
            (v.as_str().to_string(), segs)
        }
        Expression::String(_) => {
            return Err("the quoted form, which Terraform reads as a legacy provider name".into())
        }
        other => return Err(format!("`{}`, which is not a provider reference", other.to_string().trim())),
    };
    if segs.len() > 1 {
        return Err(format!("`{}.{}`, which is deeper than <provider>.<alias>", root, segs.join(".")));
    }
    Ok(match segs.first() {
        Some(alias) => format!("{}.{}", root, alias),
        None => root,
    })
}

/// `depends_on = [google_project_service.x, …]` as the addresses it names. Satz
/// has no `depends_on`: the emitter derives the ordering a plan needs from the
/// estate — a grant on a declared service account waits for it, the policies on
/// one parent are chained, a resource waits for the services that enable its
/// APIs — so the edge is dropped rather than carried. `one_pass` refuses to drop
/// one that points out of the import, where nothing can re-derive it.
fn ordering_targets(e: &Expression, schema: &dyn Schema) -> Result<Vec<String>, String> {
    let Expression::Array(items) = e else {
        return Err(format!("`{}`, which is not a list of addresses", e.to_string().trim()));
    };
    let mut out = Vec::new();
    for item in items.iter() {
        let Expression::Traversal(t) = item else {
            return Err(format!("`{}`, which is not a resource address", item.to_string().trim()));
        };
        let Expression::Variable(root) = &t.expr else {
            return Err(format!("`{}`, which is not a resource address", item.to_string().trim()));
        };
        let mut segs = Vec::new();
        for op in t.operators.iter() {
            match op.value() {
                TraversalOperator::GetAttr(k) => segs.push(k.as_str().to_string()),
                _ => return Err(format!("`{}`, which is not a resource address", item.to_string().trim())),
            }
        }
        let [label] = segs.as_slice() else {
            return Err(format!("`{}`, which is not a resource address", item.to_string().trim()));
        };
        if !schema.has_type(root.as_str()) {
            return Err(format!("`{}.{}`, which is not a resource of the provider schema", root.as_str(), label));
        }
        out.push(format!("{}.{}", root.as_str(), label));
    }
    Ok(out)
}

/// Classify one resource block: translatable (with its body and place) or the
/// reason it is not.
fn classify(
    tf_type: &str,
    label: &str,
    block: &Block,
    cx: &mut Cx,
    org_ids: &mut BTreeSet<String>,
    projects: &BTreeMap<String, String>,
) -> Classified {
    if !cx.schema.has_type(tf_type) {
        return wrapped(format!("`{}` is not in the provider schema", tf_type));
    }
    if !is_identifier(label) {
        return wrapped(format!("label `{}` is not a Satz identifier (a rename would break references)", label));
    }
    if matches!(tf_type, "google_cloud_identity_group" | "google_cloud_identity_group_membership") {
        return wrapped(format!("`{}` has a derived Satz form (group_key, parent, the standard labels) that HCL does not carry", tf_type));
    }
    if tf_type.ends_with("_iam_binding") || tf_type.ends_with("_iam_policy") || tf_type.ends_with("_iam_audit_config") {
        return wrapped(format!(
            "`{}` is authoritative — it owns every binding on its target, which a plain Satz resource would hide from the fold",
            tf_type
        ));
    }
    if tf_type == "google_billing_account_iam_member" {
        return wrapped(
            "`google_billing_account_iam_member` hoists to the estate's billing scope, where the account is pinned once — a copy taken from HCL would sit outside that scope".into(),
        );
    }

    let form = grant_form_of(cx.schema, tf_type);

    // meta-arguments first: they are the reason, and naming a later attribute
    // instead would send the reviewer to the wrong line
    let mut provider: Option<String> = None;
    for s in block.body.iter() {
        match s {
            Structure::Attribute(a) => {
                let k = a.key.to_string();
                if k == "count" {
                    return wrapped(COUNT_REASON.into());
                }
                if UNEXPRESSIBLE_META_ATTRS.contains(&k.as_str()) {
                    return wrapped(format!("uses `{}`", k));
                }
                if k == "provider" {
                    match provider_reference(&a.value) {
                        Ok(p) => provider = Some(p),
                        Err(why) => return wrapped(format!("`provider` is {}", why)),
                    }
                }
                if k == "depends_on" {
                    match ordering_targets(&a.value, cx.schema) {
                        Ok(targets) => cx.uses.ordering.extend(targets),
                        Err(why) => return wrapped(format!("`depends_on` names {}", why)),
                    }
                }
            }
            Structure::Block(b) => {
                let id = b.ident.to_string();
                if META_BLOCKS.contains(&id.as_str()) {
                    return wrapped(format!("uses `{}`", id));
                }
            }
        }
    }
    // the two the body carries away from the schema's own vocabulary: the
    // provider alias travels IN the body where the form has room for it, the
    // ordering edge is dropped.
    let dropped_meta = |k: &str| k == "depends_on" || k == "provider";
    if let Some(p) = &provider {
        if p != DEFAULT_PROVIDER && !body_carries_provider(tf_type, &form) {
            return wrapped(format!(
                "`provider = {}` has no place in the Satz form of `{}`, which is built from the estate's own provider",
                p, tf_type
            ));
        }
    }

    // the attributes Satz's special forms are keyed by must resolve
    let resolvable = |k: &str, cx: &mut Cx| -> Option<Result<serde_yaml::Value, String>> {
        block.body.iter().find_map(|s| match s {
            Structure::Attribute(a) if a.key.to_string() == k => Some(cx.literal(&a.value)),
            _ => None,
        })
    };
    if is_map_form(&form) {
        let pin = match &form {
            GrantForm::Pinned(attr) => Some(attr.as_str()),
            _ => None,
        };
        for k in ["member", "role"].into_iter().chain(pin) {
            match resolvable(k, cx) {
                None => return wrapped(format!("`{}` is missing", k)),
                Some(Err(why)) => return wrapped(format!("`{}` is {}", k, why)),
                Some(Ok(serde_yaml::Value::String(v))) if k == "member" && !is_member(&v) => {
                    return wrapped(format!(
                        "`member = \"{}\"` is not a member (`<type>:<value>`), and a grant map reads a key with no `:` as the scope it pins",
                        v
                    ))
                }
                Some(Ok(_)) => {}
            }
        }
        // anything beyond member/role/condition, the scope attribute and the
        // pin is not part of Satz's grant form — say so rather than drop it
        for s in block.body.iter() {
            let k = match s {
                Structure::Attribute(a) => a.key.to_string(),
                Structure::Block(b) => b.ident.to_string(),
            };
            if dropped_meta(&k) {
                continue;
            }
            if !matches!(k.as_str(), "member" | "role" | "condition") && !is_scope_attr(tf_type, &k) && Some(k.as_str()) != pin {
                return wrapped(format!("`{}` has no place in a Satz grant", k));
            }
        }
    }
    if tf_type == "google_project_service" {
        match resolvable("service", cx) {
            None => return wrapped("`service` is missing".into()),
            Some(Err(why)) => return wrapped(format!("`service` is {}", why)),
            Some(Ok(_)) => {}
        }
        // the estate form is a bare service list under the project; any other
        // attribute (disable_on_destroy, …) has no place in it
        for s in block.body.iter() {
            let k = match s {
                Structure::Attribute(a) => a.key.to_string(),
                Structure::Block(b) => b.ident.to_string(),
            };
            if dropped_meta(&k) {
                continue;
            }
            if !matches!(k.as_str(), "service" | "project") {
                return wrapped(format!("`{}` has no place in a project's service list", k));
            }
        }
    }

    // every value must resolve
    for s in block.body.iter() {
        match s {
            Structure::Attribute(a) => {
                let k = a.key.to_string();
                if is_scope_attr(tf_type, &k) || dropped_meta(&k) {
                    continue; // judged above, or judged below
                }
                if let Err(why) = cx.literal(&a.value) {
                    return wrapped(format!("`{}` is {}", k, why));
                }
            }
            Structure::Block(b) => {
                let id = b.ident.to_string();
                if id == "lifecycle" {
                    if let Err(why) = cx.lifecycle(&b.body) {
                        return wrapped(format!("a `lifecycle` block satz cannot express: {}", why));
                    }
                    continue;
                }
                if !b.labels.is_empty() {
                    return wrapped(format!("nested block `{}` carries labels", id));
                }
                if let Err(why) = cx.body(&b.body, &|_| false) {
                    return wrapped(format!("nested block `{}` holds {}", id, why));
                }
            }
        }
    }
    let mut body = match cx.body(&block.body, &|k| is_scope_attr(tf_type, k) || dropped_meta(k)) {
        Ok(b) => b,
        Err(why) => return wrapped(why),
    };
    // the alias as a Satz body key: the emitter renders the string back as the
    // reference it was. The estate's own provider is left out — the emitter
    // writes it on every resource that names no other, so carrying it would
    // only move the attribute down the block.
    if let Some(p) = &provider {
        if p != DEFAULT_PROVIDER {
            body.insert("provider".into(), serde_yaml::Value::String(p.clone()));
        }
    }

    // the scope attribute decides the place
    let scope = scope_attr_value(tf_type, block, cx.consts);
    if let Some(ScopeRef::Unbound(attr, name)) = &scope {
        return wrapped(if attr == "org_id" || (attr == "parent" && tf_type == "google_folder") {
            format!(
                "`{}` = {}, a variable declared without a default — name the organisation with `--organization <n>` and it binds to `{}`",
                attr, name, ORGANIZATION_PARAM
            )
        } else {
            format!("`{}` = {}, a variable declared without a default, which satz cannot place", attr, name)
        });
    }
    let place = match tf_type {
        "google_folder" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            None => return wrapped("a folder needs a literal `parent`".into()),
            Some(other) => return wrapped(format!("`parent` = {} cannot be placed", other.describe())),
        },
        "google_project" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            None => Place::Top,
            Some(other @ ScopeRef::Folderish(_)) => {
                return wrapped(format!(
                    "`folder_id` = {} cannot be placed: a folder number is not a folder in this input",
                    other.describe()
                ))
            }
            Some(other) => {
                return wrapped(format!("`folder_id`/`org_id` = {} cannot be placed", other.describe()))
            }
        },
        "google_organization_iam_member" => match scope {
            Some(ScopeRef::Org(o)) => {
                org_ids.insert(o);
                Place::Top
            }
            _ => return wrapped("an organisation grant needs a literal `org_id`".into()),
        },
        "google_folder_iam_member" => match scope {
            Some(ScopeRef::Folder(f)) => Place::Folder(f),
            _ => return wrapped("a folder grant must reference a folder in this input".into()),
        },
        "google_project_iam_member" | "google_project_service" => match scope {
            Some(ScopeRef::Project(p)) => Place::Project(p),
            // a literal id of a project in this input places the same way
            Some(ScopeRef::Literal(v)) if projects.contains_key(&v) => Place::Project(projects[&v].clone()),
            _ => return wrapped(format!("`{}` must reference a project in this input", tf_type)),
        },
        _ => match scope {
            Some(ScopeRef::Project(p)) => Place::Project(p),
            Some(ScopeRef::Literal(v)) if projects.contains_key(&v) => Place::Project(projects[&v].clone()),
            Some(ScopeRef::Literal(v)) => {
                // a project-scoped resource naming a project this input does
                // not declare stays where it is, with the literal — Satz is
                // position-independent there
                body.insert("project".into(), serde_yaml::Value::String(v));
                Place::Top
            }
            None => Place::Top,
            // a scope written as an expression satz cannot place — dropping it
            // would move the resource to the organisation
            Some(other) => return wrapped(format!("scope attribute = {} cannot be placed", other.describe())),
        },
    };
    if tf_type == "google_org_policy_policy" {
        if let Some(p) = body.get("parent").and_then(|v| v.as_str()).and_then(|p| p.strip_prefix("organizations/")) {
            org_ids.insert(p.to_string());
        }
    }
    Classified { body: Some(body), place, reason: None }
}

enum ScopeRef {
    Org(String),
    Folder(String),
    Project(String),
    Literal(String),
    /// a folder written by its number (`folders/<n>`), not by a reference
    Folderish(String),
    /// the attribute, and the `var.x` declared without a default it names
    Unbound(String, String),
    Other(String),
}

impl ScopeRef {
    fn describe(&self) -> String {
        match self {
            ScopeRef::Org(o) => format!("organizations/{}", o),
            ScopeRef::Folder(f) => format!("google_folder.{}", f),
            ScopeRef::Project(p) => format!("google_project.{}", p),
            ScopeRef::Literal(v) | ScopeRef::Folderish(v) | ScopeRef::Other(v) => v.clone(),
            ScopeRef::Unbound(_, name) => name.clone(),
        }
    }
}

/// The attribute that carries a resource's place.
fn is_scope_attr(tf_type: &str, key: &str) -> bool {
    match tf_type {
        "google_folder" => key == "parent",
        "google_project" => key == "folder_id" || key == "org_id",
        "google_organization_iam_member" => key == "org_id",
        "google_folder_iam_member" => key == "folder",
        "google_org_policy_policy" => false,
        _ => key == "project",
    }
}

/// A scope attribute written as a string — including one a promoted constant
/// resolves to, which is why placement has to consult the constants table and
/// not only `Cx::literal`.
fn scope_from_string(key: &str, v: &str) -> ScopeRef {
    if let Some(o) = v.strip_prefix("organizations/") {
        ScopeRef::Org(o.to_string())
    } else if key == "org_id" && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
        ScopeRef::Org(v.to_string())
    } else if v.starts_with("folders/") || (key == "folder_id" && v.chars().all(|c| c.is_ascii_digit())) {
        ScopeRef::Folderish(v.to_string())
    } else {
        ScopeRef::Literal(v.to_string())
    }
}

/// The scope attribute's meaning: the organisation, a folder or project of this
/// input (by identity reference), a literal, or something else.
fn scope_attr_value(tf_type: &str, block: &Block, consts: &Consts) -> Option<ScopeRef> {
    for s in block.body.iter() {
        let Structure::Attribute(a) = s else { continue };
        let k = a.key.to_string();
        if !is_scope_attr(tf_type, &k) {
            continue;
        }
        return Some(match &a.value {
            Expression::String(s) => scope_from_string(&k, s.value()),
            e @ Expression::StringTemplate(_) => match (consts.eval(e, 0), organizations_parent(e)) {
                (Some(v), _) => scope_from_string(&k, &v),
                (None, Some((kind, name)))
                    if consts.by_name.get(&name).is_some_and(|c| c.origin == Origin::VarRequired && c.organization.is_none()) =>
                {
                    ScopeRef::Unbound(k.clone(), format!("{}.{}", kind, name))
                }
                (None, _) => ScopeRef::Other(e.to_string().trim().to_string()),
            },
            Expression::Traversal(t) => {
                if let Some(v) = const_string(t, consts) {
                    scope_from_string(&k, &v)
                } else if let Some((kind, name)) = const_name(t).filter(|(_, n)| {
                    consts.by_name.get(n).is_some_and(|c| c.origin == Origin::VarRequired && c.organization.is_none())
                }) {
                    ScopeRef::Unbound(k.clone(), format!("{}.{}", kind, name))
                } else {
                    let text = a.value.to_string().trim().to_string();
                    let parts: Vec<&str> = text.split('.').collect();
                    match parts.as_slice() {
                        ["google_folder", l, "name" | "id" | "folder_id"] => ScopeRef::Folder((*l).to_string()),
                        ["google_project", l, "project_id" | "id" | "name"] => ScopeRef::Project((*l).to_string()),
                        _ => ScopeRef::Other(text),
                    }
                }
            }
            e => ScopeRef::Other(e.to_string().trim().to_string()),
        });
    }
    None
}

// ---------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------

/// What one block read while it was classified.
#[derive(Debug, Default)]
struct Uses {
    /// promoted constants, by their HCL name
    params: BTreeSet<String>,
    /// `<type>.<label>` addresses carried verbatim as `${…}`
    refs: BTreeSet<String>,
    /// `<type>.<label>` addresses its `depends_on` named — dropped from the
    /// body, kept here so `one_pass` can check each one is a block this import
    /// carries and report the edge
    ordering: BTreeSet<String>,
}

/// One piece of a Satz string. `Text` carries literal characters — including a
/// `${resource.attr}` the importer built, because the printer's `esc` doubles
/// the braces and the Satz lexer folds them back to exactly this. Source text
/// can never contain a bare `${` here: `literal` refuses it.
enum Part {
    Text(String),
    /// the constant's HCL name; the printer applies the `_` normalisation
    Param(String),
}

fn parts_value(parts: Vec<Part>) -> serde_yaml::Value {
    if parts.len() == 1 {
        return match &parts[0] {
            Part::Text(t) => serde_yaml::Value::String(t.clone()),
            Part::Param(p) => migrate::param_ref(p),
        };
    }
    migrate::interpolation(
        parts
            .into_iter()
            .map(|p| match p {
                Part::Text(t) => serde_yaml::Value::String(t),
                Part::Param(p) => migrate::param_ref(&p),
            })
            .collect(),
    )
}

/// The context one block is classified in.
struct Cx<'a> {
    consts: &'a Consts,
    schema: &'a dyn Schema,
    uses: Uses,
    /// while a `count` is being expanded: the list and the element this copy
    /// stands for, so `var.<list>[count.index]` resolves to it
    index: Option<(String, serde_yaml::Value)>,
}

impl Cx<'_> {
    /// A value as a Satz value. The `Err` IS the wrap reason, phrased as a noun
    /// so the caller can say ``` `x` is <that> ```.
    fn literal(&mut self, e: &Expression) -> Result<serde_yaml::Value, String> {
        Ok(match e {
            Expression::Null(_) => serde_yaml::Value::Null,
            Expression::Bool(b) => serde_yaml::Value::Bool(*b.value()),
            Expression::Number(n) => match (n.value().as_i64(), n.value().as_f64()) {
                (Some(i), _) => serde_yaml::Value::Number(i.into()),
                (None, Some(f)) => serde_yaml::Value::Number(f.into()),
                _ => return Err("a number satz cannot represent".into()),
            },
            Expression::String(s) => {
                guard_literal(s.value())?;
                serde_yaml::Value::String(s.value().to_string())
            }
            Expression::Array(a) => serde_yaml::Value::Sequence(
                a.iter().map(|e| self.literal(e)).collect::<Result<Vec<_>, _>>()?,
            ),
            Expression::Object(o) => {
                let mut m = serde_yaml::Mapping::new();
                for (k, v) in o.iter() {
                    let key = match k {
                        ObjectKey::Ident(i) => i.to_string(),
                        ObjectKey::Expression(Expression::String(s)) => {
                            guard_literal(s.value())?;
                            s.value().to_string()
                        }
                        _ => return Err("an object with a computed key".into()),
                    };
                    m.insert(serde_yaml::Value::String(key), self.literal(v.expr())?);
                }
                serde_yaml::Value::Mapping(m)
            }
            Expression::Traversal(t) => parts_value(self.traversal(t)?),
            Expression::StringTemplate(t) => parts_value(self.template(t)?),
            Expression::Parenthesis(p) => return self.literal(p.inner()),
            Expression::Variable(v) => {
                return Err(format!("the bare name `{}`, which satz cannot resolve", v.as_str()))
            }
            Expression::HeredocTemplate(_) => return Err("a heredoc template".into()),
            Expression::Conditional(_) => return Err("a conditional expression (`? :`)".into()),
            Expression::FuncCall(f) => return self.call(f),
            Expression::BinaryOp(_) => return Err("an arithmetic or comparison expression".into()),
            Expression::UnaryOp(_) => return Err("a unary expression".into()),
            Expression::ForExpr(_) => return Err("a `for` expression".into()),
        })
    }

    /// A function call: only `element(<list>, count.index)` inside an expanded
    /// copy, which IS that copy's entry — the form Terraform writes beside
    /// `<list>[count.index]`.
    fn call(&mut self, f: &hcl::edit::expr::FuncCall) -> Result<serde_yaml::Value, String> {
        let fname = f.name.name.as_str();
        if f.name.namespace.is_empty() && fname == "element" {
            let args: Vec<&Expression> = f.args.iter().collect();
            if let (Some((list, element)), [Expression::Traversal(t), ix]) = (&self.index, args.as_slice()) {
                if const_name(t).is_some_and(|(_, n)| &n == list) && is_count_index(ix) {
                    return Ok(serde_yaml::Value::String(scalar_text(element)));
                }
            }
        }
        Err(format!("a call to `{}()`", fname))
    }

    /// A traversal: a promoted param, or a managed resource reference carried
    /// verbatim. Nothing else — `data.*`, `module.*`, `each.*` and unknown
    /// roots are named and wrapped.
    fn traversal(&mut self, t: &Traversal) -> Result<Vec<Part>, String> {
        let Expression::Variable(v) = &t.expr else {
            return Err(format!("a traversal on `{}`", t.expr.to_string().trim()));
        };
        let root = v.as_str().to_string();
        let mut segs: Vec<String> = Vec::new();
        for op in t.operators.iter() {
            match op.value() {
                TraversalOperator::GetAttr(k) => segs.push(k.as_str().to_string()),
                TraversalOperator::Index(ix) => {
                    // `var.admins[count.index]` inside an expanded copy IS that
                    // copy's element; any other index is still unresolvable
                    match (&self.index, segs.as_slice()) {
                        (Some((list, element)), [name]) if name == list && is_count_index(ix) => {
                            return Ok(vec![Part::Text(scalar_text(element))]);
                        }
                        _ => return Err(format!("an indexed lookup into `{}`", root)),
                    }
                }
                TraversalOperator::LegacyIndex(_) => return Err(format!("an indexed lookup into `{}`", root)),
                _ => return Err(format!("a splat over `{}`", root)),
            }
        }
        match root.as_str() {
            "var" | "local" => {
                let [name] = segs.as_slice() else {
                    return Err(format!("a nested lookup `{}.{}`", root, segs.join(".")));
                };
                // resolves, or the error is this block's wrap reason
                self.consts.get(&root, name)?;
                self.uses.params.insert(name.clone());
                Ok(vec![Part::Param(name.clone())])
            }
            t0 if self.schema.has_type(t0) && segs.len() >= 2 => {
                self.uses.refs.insert(format!("{}.{}", t0, segs[0]));
                Ok(vec![Part::Text(format!("${{{}.{}}}", t0, segs.join(".")))])
            }
            other => Err(format!(
                "a reference to `{}`, which is neither a promoted param nor a resource of the provider schema",
                other
            )),
        }
    }

    /// A string template: literal chunks and interpolations, in order. A param
    /// inside a multi-part template must be a scalar — `resolve_str` refuses a
    /// list, and finding that out at transpile time would be a worse error.
    fn template(&mut self, t: &StringTemplate) -> Result<Vec<Part>, String> {
        let mut parts: Vec<Part> = Vec::new();
        for el in t.iter() {
            match el {
                Element::Literal(lit) => {
                    guard_literal(lit.value())?;
                    parts.push(Part::Text(lit.value().to_string()));
                }
                Element::Interpolation(i) => match &i.expr {
                    Expression::Traversal(tr) => parts.extend(self.traversal(tr)?),
                    other => {
                        let inner = self.literal(other)?;
                        match inner {
                            serde_yaml::Value::String(s) => parts.push(Part::Text(s)),
                            _ => return Err("an interpolation of something that is not a string".into()),
                        }
                    }
                },
                Element::Directive(_) => return Err("a template directive (`%{…}`)".into()),
            }
        }
        if parts.len() > 1 {
            for p in &parts {
                if let Part::Param(name) = p {
                    if let Some(c) = self.consts.by_name.get(name) {
                        if matches!(
                            c.value,
                            Some(serde_yaml::Value::Sequence(_)) | Some(serde_yaml::Value::Mapping(_))
                        ) {
                            return Err(format!(
                                "a string interpolating `{}`, which is a list or object and cannot be interpolated",
                                name
                            ));
                        }
                    }
                }
            }
        }
        Ok(parts)
    }

    /// A body whose values all resolve, as a mapping: attributes as written,
    /// nested blocks as lists of objects (repeated blocks append), `lifecycle`
    /// as a mapping.
    fn body(&mut self, body: &Body, skip: &dyn Fn(&str) -> bool) -> Result<serde_yaml::Mapping, String> {
        let mut m = serde_yaml::Mapping::new();
        for s in body.iter() {
            match s {
                Structure::Attribute(a) => {
                    let k = a.key.to_string();
                    if skip(&k) {
                        continue;
                    }
                    let v = self.literal(&a.value)?;
                    m.insert(serde_yaml::Value::String(k), v);
                }
                Structure::Block(b) => {
                    let id = b.ident.to_string();
                    if id == "lifecycle" {
                        let lc = self.lifecycle(&b.body)?;
                        m.insert("lifecycle".into(), serde_yaml::Value::Mapping(lc));
                        continue;
                    }
                    if !b.labels.is_empty() {
                        return Err(format!("a nested block `{}` with labels", id));
                    }
                    let inner = serde_yaml::Value::Mapping(self.body(&b.body, &|_| false)?);
                    let key = serde_yaml::Value::String(id);
                    match m.get_mut(&key) {
                        Some(serde_yaml::Value::Sequence(seq)) => seq.push(inner),
                        _ => {
                            m.insert(key, serde_yaml::Value::Sequence(vec![inner]));
                        }
                    }
                }
            }
        }
        Ok(m)
    }

    /// `lifecycle { ignore_changes = [a, b] prevent_destroy = true }` — the list
    /// items are bare traversals in HCL and strings in Satz.
    fn lifecycle(&mut self, body: &Body) -> Result<serde_yaml::Mapping, String> {
        let mut m = serde_yaml::Mapping::new();
        for s in body.iter() {
            let a = s.as_attribute().ok_or("a nested block")?;
            let k = a.key.to_string();
            let v = match (k.as_str(), &a.value) {
                ("ignore_changes" | "replace_triggered_by", Expression::Array(arr)) => serde_yaml::Value::Sequence(
                    arr.iter().map(|e| serde_yaml::Value::String(e.to_string().trim().to_string())).collect(),
                ),
                ("ignore_changes", Expression::Variable(v)) => serde_yaml::Value::String(v.to_string()),
                (_, e) => self.literal(e)?,
            };
            m.insert(serde_yaml::Value::String(k), v);
        }
        Ok(m)
    }
}

/// A literal chunk that already contains the interpolation syntax cannot be
/// expressed: hcl-edit decodes the source's `$${` to `${`, and Satz's only
/// spelling for a literal brace pair is the one that means an interpolation.
/// Carrying it would silently turn escaped text into a live reference.
/// Is this index expression Terraform's `count.index`?
fn is_count_index(e: &Expression) -> bool {
    let Expression::Traversal(t) = e else { return false };
    let Expression::Variable(v) = &t.expr else { return false };
    if v.as_str() != "count" {
        return false;
    }
    let ops: Vec<String> = t
        .operators
        .iter()
        .filter_map(|op| match op.value() {
            TraversalOperator::GetAttr(k) => Some(k.as_str().to_string()),
            _ => None,
        })
        .collect();
    ops == ["index"]
}

fn guard_literal(s: &str) -> Result<(), String> {
    if s.contains("${") || s.contains("%{") {
        return Err(
            "a string holding a literal `${` or `%{` (written `$${`/`%%{` in HCL), which satz has no spelling for"
                .to_string(),
        );
    }
    Ok(())
}

fn wrap_block(path: &str, line: usize, text: &str) -> String {
    format!("hcl trust \"imported from {}:{}\" {{\n{}\n}}\n\n", path, line, indent(text))
}

/// A label a Satz resource can be written under, emitted back under the same
/// name. The emitter's label is the Satz one with `-` replaced by `_`
/// (`emit_shared::single_resource_block`), so a label already spelled with `_`
/// survives unchanged — including the mixed case satz itself emits for an org
/// policy (`compute_managed_requireOsLogin`) and for a grant's hashed label.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn describe(s: &Structure) -> String {
    match s {
        Structure::Attribute(a) => format!("{} = …", a.key),
        Structure::Block(b) => {
            let labels: Vec<String> = b.labels.iter().map(|l| format!("\"{}\"", l.as_str())).collect();
            if labels.is_empty() {
                b.ident.to_string()
            } else {
                format!("{} {}", b.ident, labels.join(" "))
            }
        }
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("  {}", l) }).collect::<Vec<_>>().join("\n")
}

fn sanitize_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        "imported_hcl".to_string()
    } else {
        out
    }
}

/// Human summary of the rows. The `N block(s) translated` phrasing is asserted
/// by the smoke matrix — keep it.
pub fn summary(rows: &[Row]) -> String {
    let count = |f: &dyn Fn(&Action) -> bool| rows.iter().filter(|r| f(&r.action)).count();
    format!(
        "import: {} block(s) translated to Satz, {} promoted to params, {} wrapped verbatim, {} dropped (terraform/provider)",
        count(&|a| *a == Action::Translated) + rows.iter().filter_map(|r| match r.action {
            Action::Expanded(n) => Some(n),
            _ => None,
        }).sum::<usize>(),
        count(&|a| matches!(a, Action::Promoted(_))),
        count(&|a| matches!(a, Action::Wrapped(_))),
        count(&|a| matches!(a, Action::Dropped(_))),
    )
}

// ---------------------------------------------------------------------------
// A consumer's HCL, read for `satz check-consumer`
// ---------------------------------------------------------------------------

/// One top-level attribute of a consumer's block, as `satz check-consumer` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerValue {
    /// a string with no interpolation
    Literal(String),
    /// anything else: every traversal it reads, dotted (`module.satz.vpc`,
    /// `google_project.team.number`), in the order written
    Reads(Vec<String>),
}

/// One `resource`, `data` or `module` block of a consumer's configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerBlock {
    /// `resource`, `data`, `module`
    pub kind: String,
    pub labels: Vec<String>,
    pub file: String,
    /// 1-based, the block's first line
    pub line: usize,
    pub attrs: BTreeMap<String, ConsumerValue>,
}

/// Every `resource`, `data` and `module` block of the inputs, with its top-level
/// attributes. Nested blocks are not read: what an attachment joins and what an
/// authoritative grant covers are top-level arguments.
pub fn consumer_blocks(inputs: &[Input]) -> Result<Vec<ConsumerBlock>, String> {
    let mut out = Vec::new();
    for input in inputs {
        let body = parse_body(&input.text).map_err(|e| format!("{}: {}", input.path, e))?;
        for s in body.iter() {
            let Some(block) = s.as_block() else { continue };
            let kind = block.ident.to_string();
            if !matches!(kind.as_str(), "resource" | "data" | "module") {
                continue;
            }
            let Some(span) = s.span() else {
                return Err(format!("{}: a structure without a span (not parsed from text?)", input.path));
            };
            let line = input.text[..span.start].matches('\n').count() + 1;
            let labels = block.labels.iter().map(|l| l.as_str().to_string()).collect();
            let mut attrs = BTreeMap::new();
            for st in block.body.iter() {
                let Structure::Attribute(a) = st else { continue };
                let value = match &a.value {
                    Expression::String(s) => ConsumerValue::Literal(s.value().to_string()),
                    other => {
                        let mut reads = Dotted::default();
                        reads.visit_expr(other);
                        ConsumerValue::Reads(reads.0)
                    }
                };
                attrs.insert(a.key.to_string(), value);
            }
            out.push(ConsumerBlock { kind, labels, file: input.path.clone(), line, attrs });
        }
    }
    Ok(out)
}

/// Every traversal an expression reads, dotted up to its first operator that is not
/// an attribute name.
#[derive(Default)]
struct Dotted(Vec<String>);

impl Visit for Dotted {
    fn visit_traversal(&mut self, node: &Traversal) {
        if let Expression::Variable(v) = &node.expr {
            let mut s = v.as_str().to_string();
            for op in node.operators.iter() {
                match op.value() {
                    TraversalOperator::GetAttr(k) => {
                        s.push('.');
                        s.push_str(k.as_str());
                    }
                    _ => break,
                }
            }
            self.0.push(s);
        }
        visit::visit_traversal(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_consumer_s_blocks_carry_their_literals_and_what_they_read() {
        let text = "module \"satz\" {\n  source = \"../estate/interfaces/team-a/team-a/hcl\"\n}\n\nresource \"google_project_iam_member\" \"x\" {\n  project = module.satz.project_id\n  role    = \"roles/viewer\"\n  member  = \"projects/${module.satz.number}/x\"\n  condition {\n    title = \"t\"\n  }\n}\nvariable \"v\" {}\n";
        let blocks = consumer_blocks(&[Input { path: "main.tf".into(), text: text.into() }]).unwrap();
        assert_eq!(blocks.len(), 2, "a variable is not read");
        assert_eq!((blocks[0].kind.as_str(), blocks[0].labels.as_slice(), blocks[0].line), ("module", &["satz".to_string()][..], 1));
        assert_eq!(blocks[0].attrs["source"], ConsumerValue::Literal("../estate/interfaces/team-a/team-a/hcl".into()));
        assert_eq!(blocks[1].line, 5);
        assert_eq!(blocks[1].attrs["project"], ConsumerValue::Reads(vec!["module.satz.project_id".into()]));
        assert_eq!(blocks[1].attrs["member"], ConsumerValue::Reads(vec!["module.satz.number".into()]));
        assert!(!blocks[1].attrs.contains_key("condition"), "a nested block is not an argument");
    }

    const TF: &str = r#"terraform {
  required_providers {
    google = { source = "hashicorp/google" }
  }
}

provider "google" {
  project = "acme-infra-001"
}

resource "google_folder" "workloads" {
  display_name = "Workloads"
  parent       = "organizations/123456789012"
}

resource "google_folder" "team" {
  display_name = "Team"
  parent       = google_folder.workloads.name
}

resource "google_project" "infra" {
  name       = "Infra"
  project_id = "acme-infra-001"
  folder_id  = google_folder.team.name
}

resource "google_project" "orphan" {
  name       = "Orphan"
  project_id = "acme-orphan-001"
  folder_id  = "folders/999"
}

resource "google_project_service" "infra_iam" {
  project = google_project.infra.project_id
  service = "iam.googleapis.com"
}

resource "google_project_iam_member" "infra_viewer" {
  project = google_project.infra.project_id
  role    = "roles/viewer"
  member  = "group:auditors@example.com"
}

resource "google_folder_iam_member" "team_viewer" {
  folder = google_folder.team.name
  role   = "roles/viewer"
  member = "group:team@example.com"
}

resource "google_organization_iam_member" "org_admin" {
  org_id = "123456789012"
  role   = "roles/resourcemanager.organizationAdmin"
  member = "group:admins@example.com"
}

resource "google_storage_bucket" "logs" {
  name     = "acme-logs-001"
  location = "EU"
  project  = google_project.infra.project_id
  lifecycle_rule {
    action { type = "Delete" }
    condition { age = 30 }
  }
}

resource "google_storage_bucket" "elsewhere" {
  name     = "acme-elsewhere"
  location = "EU"
  project  = "acme-other-project"
}

# a literal project id of a project in this input — what bulk-export writes
resource "google_service_account" "onboarding" {
  account_id = "onboarding"
  project    = "acme-infra-001"
}

resource "google_storage_bucket" "orphaned" {
  name     = "acme-orphaned"
  location = "EU"
  project  = google_project.orphan.project_id
}

resource "google_org_policy_policy" "skip_default" {
  name   = "organizations/123456789012/policies/compute.skipDefaultNetworkCreation"
  parent = "organizations/123456789012"
  spec {
    rules { enforce = "TRUE" }
  }
}

module "vpc" {
  source = "./modules/vpc"
}
"#;

    /// The trimmed schema the older tests were written against.
    struct Known;
    impl Schema for Known {
        fn has_type(&self, t: &str) -> bool {
            matches!(
                t,
                "google_folder"
                    | "google_project"
                    | "google_project_service"
                    | "google_project_iam_member"
                    | "google_folder_iam_member"
                    | "google_organization_iam_member"
                    | "google_storage_bucket"
                    | "google_storage_bucket_iam_member"
                    | "google_service_account"
                    | "google_service_account_iam_member"
                    | "google_org_policy_policy"
                    | "google_iam_workload_identity_pool"
                    | "google_organization_iam_custom_role"
            )
        }
        fn has_attr(&self, t: &str, a: &str) -> bool {
            match a {
                "project" => matches!(
                    t,
                    "google_storage_bucket"
                        | "google_project_service"
                        | "google_project_iam_member"
                        | "google_service_account"
                        | "google_iam_workload_identity_pool"
                ),
                _ => false,
            }
        }
        fn required_attrs(&self, t: &str) -> Vec<String> {
            let attrs: &[&str] = match t {
                "google_storage_bucket_iam_member" => &["bucket", "role", "member"],
                "google_service_account_iam_member" => &["service_account_id", "role", "member"],
                "google_project_iam_member" => &["project", "role", "member"],
                "google_folder_iam_member" => &["folder", "role", "member"],
                "google_organization_iam_member" => &["org_id", "role", "member"],
                _ => &[],
            };
            attrs.iter().map(|s| s.to_string()).collect()
        }
    }

    fn one(text: &str) -> Vec<Input> {
        vec![Input { path: "main.tf".into(), text: text.into() }]
    }

    fn squash(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn by_what(imported: &Imported) -> std::collections::BTreeMap<&str, &Action> {
        imported.rows.iter().map(|r| (r.what.as_str(), &r.action)).collect()
    }

    #[test]
    fn resources_are_placed_by_the_folder_and_project_they_reference() {
        let imported = import(&one(TF), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        assert!(s.contains("customer_organization_id = \"123456789012\""), "{}", s);
        // team nests under workloads, infra under team, the bucket/service/grant under infra
        let i_workloads = s.find("  workloads {").expect("workloads");
        let i_team = s.find("    team {").expect("team nested");
        let i_infra = s.find("      infra {").expect("infra nested under team");
        assert!(i_workloads < i_team && i_team < i_infra, "{}", s);
        let c = squash(s);
        assert!(c.contains("project_service=[\"iam.googleapis.com\",]"), "{}", s);
        assert!(c.contains("google_project_iam_member{\"group:auditors@example.com\"=[\"roles/viewer\",]}"), "{}", s);
        assert!(c.contains("google_folder_iam_member{\"group:team@example.com\"=[\"roles/viewer\",]}"), "{}", s);
        assert!(
            c.contains("google_organization_iam_member{\"group:admins@example.com\"=[\"roles/resourcemanager.organizationAdmin\",]}"),
            "{}",
            s
        );
        assert!(
            c.contains("google_storage_bucket{logs{name=\"acme-logs-001\"location=\"EU\"lifecycle_rule=[{action=[{type=\"Delete\"},]condition=[{age=30},]},]}}"),
            "bucket inside the project:\n{}",
            s
        );
        let satz_part = squash(&s[..s.find("hcl trust").unwrap_or(s.len())]);
        assert!(!satz_part.contains("project=\"acme-infra-001\""), "the project reference became placement, not an attribute:\n{}", s);
        // the verbatim blocks deploy with the source's default provider, which
        // the emitter's aliased providers are not
        assert!(c.contains("hcltrust\"importedfrommain.tf:7\"{provider\"google\"{project=\"acme-infra-001\"}}"), "{}", s);
        assert!(
            c.contains("google_storage_bucket{elsewhere{name=\"acme-elsewhere\"location=\"EU\"project=\"acme-other-project\"}}"),
            "a literal id of a project not in this input stays explicit at the top:\n{}",
            s
        );
        assert!(
            s.find("        google_service_account {").is_some_and(|i| i > i_infra),
            "a literal id of a project in this input places under it:\n{}",
            s
        );
        assert!(c.contains("google_service_account{onboarding{account_id=\"onboarding\"}}"), "{}", s);
        assert!(c.contains("google_org_policy_policy{skip_default{"), "{}", s);
        // wrapped, with reasons
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_project\" \"orphan\""], Action::Wrapped(r) if r.contains("folder number")),
            "{:?}",
            by["resource \"google_project\" \"orphan\""]
        );
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"orphaned\""], Action::Wrapped(r) if r.contains("is wrapped")),
            "closure by dependency: {:?}",
            by["resource \"google_storage_bucket\" \"orphaned\""]
        );
        assert!(matches!(by["module \"vpc\""], Action::Wrapped(_)));
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 11);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn wrap_all_wraps_everything_that_is_not_dropped() {
        let imported = import(&one(TF), "acme", true, Some("123456789012"), &Known).unwrap();
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 0);
        assert_eq!(imported.rows.iter().filter(|r| matches!(r.action, Action::Promoted(_))).count(), 0);
        // the provider block too: every resource is verbatim and uses it
        assert_eq!(imported.rows.iter().filter(|r| matches!(r.action, Action::Wrapped(_))).count(), 15);
        assert_eq!(imported.rows.iter().filter(|r| matches!(r.action, Action::Dropped(_))).count(), 1, "the terraform block");
        assert!(imported.satz.contains("provider \"google\" {\n    project = \"acme-infra-001\""), "{}", imported.satz);
        assert!(imported.satz.contains("It deploys as written;"), "{}", imported.satz);
        satz_core::satz::parse(&imported.satz).unwrap();
    }

    /// An estate is bound to one organisation. The hcl shape writes one only when
    /// it knows which: named by the configuration, or by `--organization` — the
    /// flag filling what the configuration does not say, never overruling it.
    #[test]
    fn the_estate_is_written_bound_to_one_organisation_or_not_at_all() {
        let no_org = "resource \"google_storage_bucket\" \"b\" {\n  name     = \"corp-logs-001\"\n  location = \"EU\"\n}\n";
        let e = import(&one(no_org), "e", false, None, &Known).unwrap_err();
        assert!(e.contains("no organization id") && e.contains("--organization <n>"), "{e}");
        let e = import(&one(no_org), "e", true, None, &Known).unwrap_err();
        assert!(e.contains("--wrap-all translates nothing") && e.contains("--organization <n>"), "{e}");
        for wrap_all in [false, true] {
            let s = import(&one(no_org), "e", wrap_all, Some("123456789012"), &Known).unwrap().satz;
            assert!(s.contains("customer_organization_id = \"123456789012\""), "wrap_all={wrap_all}: {s}");
        }
        // the configuration names one: the flag agrees or is refused
        let named = "resource \"google_folder\" \"f\" {\n  display_name = \"Shared\"\n  parent       = \"organizations/123456789012\"\n}\n";
        import(&one(named), "e", false, None, &Known).expect("the configuration names it");
        let e = import(&one(named), "e", false, Some("222222222222"), &Known).unwrap_err();
        assert!(e.contains("--organization 222222222222") && e.contains("names organization 123456789012"), "{e}");
        // two organisations are two estates
        let two = format!("{}resource \"google_folder\" \"g\" {{\n  display_name = \"Other\"\n  parent       = \"organizations/222222222222\"\n}}\n", named);
        let e = import(&one(&two), "e", false, None, &Known).unwrap_err();
        assert!(e.contains("names 2 organizations") && e.contains("222222222222"), "{e}");
    }

    #[test]
    fn a_syntax_error_names_the_file() {
        let e = import(&[Input { path: "bad.tf".into(), text: "resource \"x\" {".into() }], "e", true, Some("123456789012"), &Known).unwrap_err();
        assert!(e.starts_with("bad.tf: "), "{}", e);
    }

    /// The review found a conditional binding imported as an unconditional one,
    /// reported Translated — a silent privilege widening.
    #[test]
    fn a_conditional_grant_keeps_its_condition_and_a_partial_one_is_wrapped() {
        let tf = r#"
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_project_iam_member" "cond" {
  project = google_project.p.project_id
  role    = "roles/viewer"
  member  = "group:auditors@example.com"
  condition {
    title      = "office-hours"
    expression = "request.time.getHours(\"Europe/Berlin\") >= 8"
  }
}
resource "google_project_iam_member" "no_member" {
  project = google_project.p.project_id
  role    = "roles/viewer"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let c = squash(&imported.satz);
        assert!(
            c.contains("\"group:auditors@example.com\"=[{role=\"roles/viewer\"condition{title=\"office-hours\""),
            "{:?}\n{}",
            imported.rows,
            imported.satz
        );
        let row = imported.rows.iter().find(|r| r.what.contains("no_member")).unwrap();
        assert!(matches!(&row.action, Action::Wrapped(r) if r.contains("`member`")), "{:?}", row.action);
    }

    /// Terraform's own "one of these per entry" is `count = length(var.list)`
    /// with `var.list[count.index]`. Satz writes one resource per entry, so the
    /// block expands instead of being carried verbatim inside `hcl trust`.
    #[test]
    fn a_count_over_a_promoted_list_becomes_one_resource_per_entry() {
        let tf = r#"
variable "viewers" {
  type    = list(string)
  default = ["group:auditors@example.com", "group:security@example.com"]
}

resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}

resource "google_project_iam_member" "viewers" {
  count   = length(var.viewers)
  project = google_project.p.project_id
  role    = "roles/viewer"
  member  = var.viewers[count.index]
}
"#;
        let imported = import(&one(tf), "e", false, Some("123456789012"), &Known).unwrap();
        let expanded: Vec<&Row> = imported.rows.iter().filter(|r| matches!(r.action, Action::Expanded(_))).collect();
        assert_eq!(expanded.len(), 1, "{:?}", imported.rows);
        assert_eq!(expanded[0].action, Action::Expanded(2), "one resource per list entry");
        assert!(
            !imported.rows.iter().any(|r| matches!(&r.action, Action::Wrapped(_))),
            "something was carried verbatim: {:?}",
            imported.rows
        );
        // a grant map keyed by member: both entries, each with the role
        assert!(imported.satz.contains("group:auditors@example.com"), "{}", imported.satz);
        assert!(imported.satz.contains("group:security@example.com"), "{}", imported.satz);
        assert_eq!(imported.satz.matches("roles/viewer").count(), 2, "{}", imported.satz);
    }

    /// A labelled type takes the entry into its label, so a reader can tell the
    /// copies apart — and the resource is referenceable.
    #[test]
    fn an_expanded_label_names_the_entry() {
        let tf = r#"
variable "regions" {
  type    = list(string)
  default = ["europe-west3", "europe-west4"]
}

resource "google_storage_bucket" "state" {
  count    = length(var.regions)
  name     = "acme-state-${var.regions[count.index]}"
  location = var.regions[count.index]
}
"#;
        let imported = import(&one(tf), "e", false, Some("123456789012"), &Known).unwrap();
        assert!(imported.satz.contains("state_europe_west3"), "{}", imported.satz);
        assert!(imported.satz.contains("state_europe_west4"), "{}", imported.satz);
        assert!(imported.satz.contains("acme-state-europe-west3"), "the template took the entry:\n{}", imported.satz);
        assert!(!imported.satz.contains("count.index"), "{}", imported.satz);
    }

    /// Only that shape. A `count` over something this import cannot resolve, or a
    /// `count.index` used anywhere else, leaves the block wrapped — half an
    /// expansion would be a guess about what the source meant.
    #[test]
    fn a_count_the_import_cannot_resolve_still_wraps() {
        let unknown = r#"
variable "names" { type = list(string) }

resource "google_storage_bucket" "b" {
  count    = length(var.names)
  name     = var.names[count.index]
  location = "EU"
}
"#;
        let imported = import(&one(unknown), "e", false, Some("123456789012"), &Known).unwrap();
        assert!(
            imported.rows.iter().any(|r| matches!(&r.action, Action::Wrapped(w) if w.contains("count"))),
            "{:?}",
            imported.rows
        );

        // the index used for something else: not this list
        let other = r#"
variable "names" { default = ["a", "b"] }
variable "others" { default = ["x", "y"] }

resource "google_storage_bucket" "b" {
  count    = length(var.names)
  name     = var.others[count.index]
  location = "EU"
}
"#;
        let imported = import(&one(other), "e", false, Some("123456789012"), &Known).unwrap();
        assert!(
            imported.rows.iter().any(|r| matches!(&r.action, Action::Wrapped(_))),
            "an index into another list is not this expansion: {:?}",
            imported.rows
        );
    }

    #[test]
    fn variables_and_locals_become_params_and_the_blocks_are_reported_promoted() {
        let tf = r#"
variable "org_id" { default = "123456789012" }
variable "pid" { default = "acme-infra-001" }
variable "billing" { description = "no default on purpose" }
locals {
  pretty = "Infra"
}
resource "google_project" "infra" {
  name       = local.pretty
  project_id = var.pid
  org_id     = var.org_id
  billing_account = var.billing
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        // promoted, with values
        assert!(s.contains("pid = \"acme-infra-001\""), "{}", s);
        assert!(s.contains("pretty = \"Infra\""), "{}", s);
        // the org id also lands as the inferred customer_organization_id
        assert!(s.contains("customer_organization_id = \"123456789012\""), "{}", s);
        // a variable without a default is NOT given a value; it is named in the header
        assert!(!s.contains("billing = \"\""), "a shimmed empty default:\n{}", s);
        assert!(s.contains("// Bind these before transpiling"), "{}", s);
        assert!(s.contains("//   billing"), "{}", s);
        assert!(imported.notes.iter().any(|n| n.contains("must be bound before transpiling")), "{:?}", imported.notes);
        // the resource references params bare, never a literal or var.x
        let c = squash(s);
        assert!(c.contains("name=pretty"), "{}", s);
        assert!(c.contains("project_id=pid"), "{}", s);
        assert!(c.contains("billing_account=billing"), "{}", s);
        assert!(!s.contains("var."), "{}", s);
        // the declarations are consumed, not wrapped
        let by = by_what(&imported);
        assert!(matches!(by["variable \"pid\""], Action::Promoted(d) if d.contains("acme-infra-001")), "{:?}", by["variable \"pid\""]);
        assert!(matches!(by["locals"], Action::Promoted(d) if d.contains("pretty")), "{:?}", by["locals"]);
        assert!(matches!(by["variable \"billing\""], Action::Promoted(d) if d.contains("bind it")), "{:?}", by["variable \"billing\""]);
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 1);
        assert!(!imported.satz.contains("hcl trust \"imported"), "nothing should wrap:\n{}", imported.satz);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn a_template_becomes_an_interpolation_and_a_traversal_stays_verbatim() {
        let tf = r#"
variable "org_id" { default = "123456789012" }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = var.org_id
}
resource "google_service_account" "sa" {
  project    = google_project.p.project_id
  account_id = "svc-iac"
}
resource "google_organization_iam_member" "grant" {
  org_id = var.org_id
  role   = "organizations/${var.org_id}/roles/custom"
  member = "serviceAccount:${google_service_account.sa.email}"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        // a mixed template: the param interpolates, the resource ref is verbatim
        assert!(s.contains(r#""organizations/{org_id}/roles/custom""#), "{}", s);
        assert!(s.contains(r#""serviceAccount:${{google_service_account.sa.email}}""#), "{}", s);
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 3);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn a_whole_value_traversal_is_carried_and_a_list_param_is_referenced_bare() {
        let tf = r#"
variable "roles" { default = ["roles/viewer", "roles/browser"] }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_storage_bucket" "b" {
  name     = "acme-b"
  location = "EU"
  project  = google_project.p.project_id
}
resource "google_storage_bucket_iam_member" "m" {
  bucket = google_storage_bucket.b.name
  role   = "roles/storage.objectViewer"
  member = "group:auditors@example.com"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        // a bucket grant is the scope-pinned member map: the bucket beside the members
        assert!(
            squash(s).contains("google_storage_bucket_iam_member{bucket=\"${{google_storage_bucket.b.name}}\"\"group:auditors@example.com\"=[\"roles/storage.objectViewer\",]}"),
            "{}",
            s
        );
        assert!(s.contains(r#"bucket = "${{google_storage_bucket.b.name}}""#), "{}", s);
        // the unread list param still renders as a list
        assert!(squash(s).contains("roles=[\"roles/viewer\",\"roles/browser\",]"), "{}", s);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn a_list_param_cannot_be_interpolated_into_a_string() {
        let tf = r#"
variable "roles" { default = ["a", "b"] }
resource "google_storage_bucket" "b" {
  name     = "acme-${var.roles}"
  location = "EU"
  project  = "p"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let row = imported.rows.iter().find(|r| r.what.contains("google_storage_bucket")).unwrap();
        assert!(
            matches!(&row.action, Action::Wrapped(r) if r.contains("list or object")),
            "{:?}",
            row.action
        );
    }

    #[test]
    fn an_unknown_or_colliding_name_wraps_with_a_precise_reason() {
        let tf = r#"
variable "dup" { default = "x" }
locals { dup = "y" }
resource "google_storage_bucket" "a" {
  name     = var.nope
  location = "EU"
  project  = "p"
}
resource "google_storage_bucket" "b" {
  name     = var.dup
  location = "EU"
  project  = "p"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"a\""], Action::Wrapped(r) if r.contains("does not declare")),
            "{:?}",
            by["resource \"google_storage_bucket\" \"a\""]
        );
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"b\""], Action::Wrapped(r) if r.contains("declared twice")),
            "{:?}",
            by["resource \"google_storage_bucket\" \"b\""]
        );
    }

    #[test]
    fn a_literal_dollar_brace_is_refused_rather_than_turned_into_a_reference() {
        let tf = r#"
resource "google_storage_bucket" "b" {
  name     = "acme-$${not_a_ref}"
  location = "EU"
  project  = "p"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let row = imported.rows.iter().find(|r| r.what.contains("google_storage_bucket")).unwrap();
        assert!(matches!(&row.action, Action::Wrapped(r) if r.contains("literal `${`")), "{:?}", row.action);
    }

    #[test]
    fn a_declaration_a_wrapped_block_still_reads_is_carried_verbatim_too() {
        let tf = r#"
variable "svc_list" { default = ["a.googleapis.com"] }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_project_service" "s" {
  count   = 2
  project = google_project.p.project_id
  service = var.svc_list[count.index]
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        let by = by_what(&imported);
        // the count block wraps, naming count — not a later attribute — and the
        // forms that do expand
        assert!(
            matches!(by["resource \"google_project_service\" \"s\""], Action::Wrapped(r) if r.starts_with("uses `count = 2`, which does not expand") && r.contains("element(<list>, count.index)")),
            "{:?}",
            by["resource \"google_project_service\" \"s\""]
        );
        // and the variable it reads is promoted AND carried verbatim
        assert!(
            matches!(by["variable \"svc_list\""], Action::Promoted(d) if d.contains("carried verbatim too")),
            "{:?}",
            by["variable \"svc_list\""]
        );
        assert!(s.contains("svc_list = ["), "the param is declared:\n{}", s);
        assert!(s.contains("variable \"svc_list\""), "the declaration is carried:\n{}", s);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    #[test]
    fn a_dropped_provider_default_project_places_the_resources_that_relied_on_it() {
        let tf = r#"
variable "pid" { default = "acme-infra-001" }
provider "google" {
  project = var.pid
}
resource "google_project" "p" {
  name       = "p"
  project_id = var.pid
  org_id     = "123456789012"
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        // the service account has no `project` of its own; it inherits the
        // provider default, which resolves to the imported project
        let i_p = s.find("  p {").expect("project");
        let i_sa = s.find("google_service_account").expect("sa");
        assert!(i_p < i_sa, "the service account must sit inside the project:\n{}", s);
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 2);
        let dropped = imported.rows.iter().find(|r| matches!(r.action, Action::Dropped(_))).unwrap();
        assert!(
            matches!(&dropped.action, Action::Dropped(d) if d.contains("carried as placement")),
            "the dropped row must name what it carried: {:?}",
            dropped.action
        );
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    /// satz puts `provider` and `depends_on` on nearly every resource it emits,
    /// so refusing both is refusing satz's own output. `provider` is a Satz body
    /// key; `depends_on` is ordering the emitter derives from the estate.
    #[test]
    fn a_provider_alias_travels_in_the_body_and_an_ordering_edge_is_dropped() {
        let tf = r#"
resource "google_project" "p" {
  provider   = google.google
  name       = "p"
  project_id = "corp-infra-001"
  org_id     = "123456789012"
}
resource "google_project_service" "iam" {
  provider   = google.google
  project    = google_project.p.project_id
  service    = "iam.googleapis.com"
}
resource "google_service_account" "sa" {
  provider   = google-beta
  project    = google_project.p.project_id
  account_id = "svc-iac"
  depends_on = [google_project_service.iam]
}
"#;
        let imported = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap();
        assert_eq!(
            imported.rows.iter().filter(|r| r.action == Action::Translated).count(),
            3,
            "{:?}",
            imported.rows
        );
        let s = &imported.satz;
        // a non-default alias travels as a body key, which the emitter renders
        // back as the reference it was
        assert!(squash(s).contains(r#"provider="google-beta""#), "{}", s);
        // the estate's own provider is left to the emitter, which writes it on
        // every resource that names no other
        assert!(!s.contains(r#""google.google""#), "the default alias was carried:\n{}", s);
        // no ordering in the estate, and the edge is reported
        assert!(!s.contains("depends_on"), "{}", s);
        assert_eq!(
            imported.ordering_dropped,
            vec![("google_service_account.sa".to_string(), vec!["google_project_service.iam".to_string()])]
        );
        assert!(imported.notes.iter().any(|n| n.contains("no longer write the `depends_on`")), "{:?}", imported.notes);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    /// A folder, a project's service list and a grant map are built from a form
    /// of their own, which takes the alias from the estate: an alias that is not
    /// the estate's own has nowhere to go there.
    #[test]
    fn a_form_with_no_room_for_an_alias_wraps_naming_it() {
        let tf = r#"
resource "google_folder" "f" {
  provider     = google-beta
  display_name = "F"
  parent       = "organizations/123456789012"
}
resource "google_organization_iam_member" "admins" {
  provider = google-beta
  org_id   = "123456789012"
  role     = "roles/resourcemanager.organizationViewer"
  member   = "group:gcp-organization-admins@example.com"
}
"#;
        let imported = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        for what in ["resource \"google_folder\" \"f\"", "resource \"google_organization_iam_member\" \"admins\""] {
            assert!(
                matches!(by[what], Action::Wrapped(r) if r.contains("`provider = google-beta` has no place")),
                "{}: {:?}",
                what,
                by[what]
            );
        }
    }

    /// The edge is dropped because the emitter can put it back from the estate.
    /// It cannot put back an edge to a resource the estate never holds.
    #[test]
    fn an_ordering_edge_that_leaves_the_import_wraps() {
        let tf = r#"
resource "google_storage_bucket" "b" {
  name       = "corp-b"
  location   = "EU"
  project    = "corp-other-001"
  depends_on = [google_project_service.elsewhere]
}
resource "google_storage_bucket" "c" {
  name       = "corp-c"
  location   = "EU"
  project    = "corp-other-001"
  depends_on = [module.vpc]
}
"#;
        let imported = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"b\""], Action::Wrapped(r) if r.contains("`depends_on` names `google_project_service.elsewhere`, which no block in this import declares")),
            "{:?}",
            by["resource \"google_storage_bucket\" \"b\""]
        );
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"c\""], Action::Wrapped(r) if r.contains("not a resource of the provider schema")),
            "{:?}",
            by["resource \"google_storage_bucket\" \"c\""]
        );
    }

    /// `hcl trust` is text: satz emits no address for it, so a `${…}` from a
    /// translated block to a wrapped one is an estate `satz transpile` refuses
    /// with `written-reference`. The import refuses first and writes nothing.
    #[test]
    fn a_reference_from_a_translated_block_to_a_wrapped_one_refuses() {
        let tf = r#"
resource "google_project" "p" {
  name       = "p"
  project_id = "corp-infra-001"
  org_id     = "123456789012"
}
resource "google_storage_bucket" "ring" {
  for_each = var.nothing
  name     = "corp-ring"
  location = "EU"
  project  = google_project.p.project_id
}
resource "google_service_account" "key" {
  project     = google_project.p.project_id
  account_id  = "svc-key"
  description = "beside ${google_storage_bucket.ring.name}"
}
"#;
        let e = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap_err();
        assert!(e.contains("so nothing was written"), "{}", e);
        assert!(
            e.contains(
                "`google_service_account.key` references `google_storage_bucket.ring`, which stays verbatim inside `hcl trust`"
            ),
            "{}",
            e
        );
        assert!(e.contains("uses `for_each`"), "the other side's own reason: {}", e);
        assert!(e.contains("--wrap-all"), "{}", e);
        // and it is the import that refuses: --wrap-all carries both
        import(&one(tf), "corp", true, Some("123456789012"), &Known).unwrap();
    }

    /// A reference to an address no block declares was a note and a written
    /// estate that does not transpile.
    #[test]
    fn a_reference_that_leaves_the_import_refuses_too() {
        let tf = r#"
resource "google_storage_bucket" "b" {
  name     = "corp-b"
  location = "EU"
  project  = "corp-other-001"
  labels   = { ring = "${google_project.elsewhere.project_id}" }
}
"#;
        let e = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap_err();
        assert!(e.contains("references `google_project.elsewhere`, which no block in this import declares"), "{}", e);
    }

    /// The label satz itself emits for an org policy is mixed case
    /// (`compute_managed_requireOsLogin`), and a grant's is the hashed one. Both
    /// are Satz labels, and the emitter writes them back unchanged.
    #[test]
    fn a_label_satz_itself_emits_is_a_label_this_import_keeps() {
        let tf = r#"
resource "google_org_policy_policy" "compute_managed_requireOsLogin" {
  name   = "organizations/123456789012/policies/compute.managed.requireOsLogin"
  parent = "organizations/123456789012"
  spec {
    rules { enforce = "TRUE" }
  }
}
"#;
        let imported = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap();
        assert_eq!(imported.rows.iter().filter(|r| r.action == Action::Translated).count(), 1, "{:?}", imported.rows);
        assert!(imported.satz.contains("compute_managed_requireOsLogin"), "{}", imported.satz);
        satz_core::satz::parse(&imported.satz).unwrap();
    }

    /// A grant map is keyed by member, and satz-core reads a key with no `:` as
    /// the scope the map pins — so a member that is a bare `${…}` reference
    /// would become a scope attribute and the front end would refuse the estate.
    #[test]
    fn a_grant_whose_member_is_not_a_member_wraps() {
        let tf = r#"
resource "google_project" "p" {
  name       = "p"
  project_id = "corp-infra-001"
  org_id     = "123456789012"
}
resource "google_organization_iam_member" "sink_writer" {
  org_id = "123456789012"
  role   = "roles/logging.bucketWriter"
  member = "${google_service_account.sa.email}"
}
resource "google_service_account" "sa" {
  project    = google_project.p.project_id
  account_id = "svc-iac"
}
"#;
        let imported = import(&one(tf), "corp", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_organization_iam_member\" \"sink_writer\""], Action::Wrapped(r) if r.contains("is not a member (`<type>:<value>`)")),
            "{:?}",
            by["resource \"google_organization_iam_member\" \"sink_writer\""]
        );
    }

    /// Vendor-generated onboarding asks for the organisation as a variable with no
    /// default and uses it as the project's `org_id`. `--organization` names that
    /// organisation, so the variable is bound to `customer_organization_id` —
    /// rather than wrapping the project and, through the reference closure,
    /// refusing the whole input.
    #[test]
    fn an_unbound_organisation_variable_binds_to_the_organisation_the_flag_names() {
        let tf = r#"
variable "org_id" { type = string }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = var.org_id
}
resource "google_iam_workload_identity_pool" "pool" {
  project                   = google_project.p.project_id
  workload_identity_pool_id = "pool"
}
resource "google_organization_iam_custom_role" "reader" {
  org_id  = var.org_id
  role_id = "acmeReader"
  title   = "Reader"
}
resource "google_organization_iam_binding" "viewer" {
  org_id  = var.org_id
  role    = "roles/viewer"
  members = ["group:auditors@example.com"]
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        let by = by_what(&imported);
        assert_eq!(by["resource \"google_project\" \"p\""], &Action::Translated, "{:?}", imported.rows);
        assert!(squash(s).contains("org_id=customer_organization_id"), "the variable names the organisation param:\n{}", s);
        assert!(!s.contains("Bind these"), "a bound variable is not asked for:\n{}", s);
        assert!(
            matches!(by["variable \"org_id\""], Action::Promoted(d) if d.contains("bound to the one `--organization` names")),
            "{:?}",
            by["variable \"org_id\""]
        );
        // the verbatim grant still reads `var.org_id`, so the declaration is
        // carried — with the organisation as its default
        assert!(s.contains("variable \"org_id\" {\n    type = string\n    default = \"123456789012\"\n  }"), "{}", s);
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));

        // a default that is the organisation binds the same way; one that is
        // another organisation is refused like a literal
        let with_default = tf.replace("variable \"org_id\" { type = string }", "variable \"org_id\" { default = \"123456789012\" }");
        let s = import(&one(&with_default), "acme", false, Some("123456789012"), &Known).unwrap().satz;
        assert!(squash(&s).contains("org_id=customer_organization_id"), "{}", s);
        let e = import(&one(&with_default), "acme", false, Some("222222222222"), &Known).unwrap_err();
        assert!(e.contains("names organization 123456789012"), "{e}");
        // bound by its default, the variable still binds the estate when only
        // an attribute that places nothing reads it
        let role_only = "variable \"org_number\" { default = \"123456789012\" }\nresource \"google_organization_iam_custom_role\" \"r\" {\n  org_id  = var.org_number\n  role_id = \"acmeReader\"\n  title   = \"Reader\"\n}\n";
        let e = import(&one(role_only), "acme", false, Some("222222222222"), &Known).unwrap_err();
        assert!(e.contains("names organization 123456789012"), "{e}");
    }

    /// Without `--organization` the project cannot be placed, and the reason says
    /// what to do — not that "a folder number is not a folder in this input". A
    /// block the closure wraps names the block that STARTED the chain, and so
    /// does the refusal a crossing reference ends in.
    #[test]
    fn a_wrap_and_a_refusal_name_the_root_reason() {
        let tf = r#"
variable "org_id" { type = string }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = var.org_id
}
resource "google_iam_workload_identity_pool" "pool" {
  project                   = google_project.p.project_id
  workload_identity_pool_id = "pool"
}
resource "google_storage_bucket" "b" {
  name     = "acme-b"
  location = "EU"
  project  = "acme-log-001"
  labels   = { pool = "${google_iam_workload_identity_pool.pool.name}" }
}
"#;
        let e = import(&one(tf), "acme", false, None, &Known).unwrap_err();
        let root = "`org_id` = var.org_id, a variable declared without a default — name the organisation with `--organization <n>`";
        assert!(e.contains(&format!("its parent resource \"google_project\" \"p\" is wrapped: {}", root)), "{e}");
        assert!(!e.contains("folder number"), "{e}");

        // two hops: the folder wraps, the project under it, the bucket under that
        let chain = r#"
resource "google_folder" "f" {
  display_name = "F"
  parent       = "organizations/123456789012"
  for_each     = var.nothing
}
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  folder_id  = google_folder.f.name
}
resource "google_storage_bucket" "b" {
  name     = "acme-b"
  location = "EU"
  project  = google_project.p.project_id
}
"#;
        let imported = import(&one(chain), "acme", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_storage_bucket\" \"b\""], Action::Wrapped(r) if r == "its parent resource \"google_project\" \"p\" is wrapped, because resource \"google_folder\" \"f\" is: uses `for_each`"),
            "{:?}",
            by["resource \"google_storage_bucket\" \"b\""]
        );
    }

    /// The provider's default project is a local built by interpolation over
    /// other constants. It folds to the id of an imported project, so the
    /// resources that name no project move into that one.
    #[test]
    fn an_interpolated_provider_default_places_the_resources_that_relied_on_it() {
        let tf = r#"
variable "org_id" { type = string }
variable "prefix" { default = "acme-infra-" }
locals {
  pid = "${var.prefix}${var.org_id}"
}
provider "google" {
  project = local.pid
  region  = "europe-west3"
}
resource "google_project" "p" {
  name       = "p"
  project_id = local.pid
  org_id     = var.org_id
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        let i_p = s.find("google_project {").expect("project");
        let i_sa = s.find("google_service_account {").expect("sa");
        assert!(i_p < i_sa && s[i_p..i_sa].contains("  p {"), "the service account must sit inside the project:\n{}", s);
        // the region travels into the estate's provider, which the project's
        // alias takes its own from
        assert!(squash(s).contains("providers{google{alias=\"google\"region=\"europe-west3\"}"), "{}", s);
        let row = imported.rows.iter().find(|r| r.what == "provider \"google\"").unwrap();
        assert!(
            matches!(&row.action, Action::Dropped(d) if d.contains("carried as placement: 1 resource(s) moved into google_project.p") && d.contains("region \"europe-west3\"")),
            "{:?}",
            row.action
        );
        satz_core::satz::parse(s).unwrap_or_else(|e| panic!("{}\n---\n{}", e, s));
    }

    /// No imported project has the default's id: each resource that relied on
    /// it is given it as its own `project`. A default satz cannot write wraps
    /// each of them, naming it — never a resource emitted without a project.
    #[test]
    fn a_provider_default_is_written_on_the_resource_or_wraps_it() {
        let tf = r#"
variable "pid" { default = "acme-log-001" }
provider "google" {
  project = "${var.pid}"
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
}
resource "google_storage_bucket" "b" {
  name     = "acme-b"
  location = "EU"
  project  = "acme-infra-001"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let c = squash(&imported.satz);
        assert!(c.contains("google_service_account{sa{account_id=\"svc-iac\"project=pid}}"), "{}", imported.satz);
        assert!(c.contains("project=\"acme-infra-001\""), "a resource naming its own project keeps it: {}", imported.satz);

        let unwritable = r#"
provider "google" {
  project = data.google_project.current.project_id
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
}
"#;
        let imported = import(&one(unwritable), "acme", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert!(
            matches!(by["resource \"google_service_account\" \"sa\""], Action::Wrapped(r) if r.starts_with("it names no `project` and relies on the provider's default project `data.google_project.current.project_id`")),
            "{:?}",
            by["resource \"google_service_account\" \"sa\""]
        );
        // it deploys as written: with the provider it relied on
        assert!(matches!(by["provider \"google\""], Action::Wrapped(r) if r.starts_with("carried verbatim")), "{:?}", by["provider \"google\""]);
        assert!(imported.satz.contains("provider \"google\" {\n    project = data.google_project.current.project_id"), "{}", imported.satz);
    }

    /// `--wrap-all` carries the provider blocks with the rest, so the verbatim
    /// resources keep their default project and region; the one the estate
    /// declares itself cannot be carried, and the header says so.
    #[test]
    fn wrap_all_keeps_the_provider_defaults_or_names_what_it_does_not_carry() {
        let tf = r#"
provider "google" {
  project = "acme-infra-001"
  region  = "europe-west3"
}
provider "google-beta" {
  alias   = "google-beta"
  project = "acme-infra-001"
}
resource "google_service_account" "sa" {
  account_id = "svc-iac"
}
"#;
        let imported = import(&one(tf), "acme", true, Some("123456789012"), &Known).unwrap();
        let s = &imported.satz;
        assert!(s.contains("provider \"google\" {\n    project = \"acme-infra-001\"\n    region  = \"europe-west3\""), "{}", s);
        assert!(!s.contains("It deploys as written;"), "{}", s);
        assert!(s.contains("except where a `provider` block is not carried"), "{}", s);
        assert!(s.contains("`google-beta.google-beta`") && s.contains("`project`"), "the header names the block and its setting:\n{}", s);
        assert!(imported.notes.iter().any(|n| n.starts_with("not carried: ")), "{:?}", imported.notes);
        satz_core::satz::parse(s).unwrap();
    }

    /// Microsoft's generated scripts write the entry as `element(list,
    /// count.index)`; it is the same entry as `list[count.index]`.
    #[test]
    fn element_over_count_index_expands_like_an_index() {
        let tf = r#"
variable "apis" { default = ["iam.googleapis.com", "sts.googleapis.com"] }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_project_service" "s" {
  count   = length(var.apis)
  project = google_project.p.project_id
  service = element(var.apis, count.index)
}
resource "google_storage_bucket" "b" {
  count    = length(var.apis)
  name     = "acme-${element(var.apis, count.index)}"
  location = "EU"
  project  = google_project.p.project_id
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert_eq!(by["resource \"google_project_service\" \"s\""], &Action::Expanded(2), "{:?}", imported.rows);
        assert_eq!(by["resource \"google_storage_bucket\" \"b\""], &Action::Expanded(2), "{:?}", imported.rows);
        let c = squash(&imported.satz);
        assert!(c.contains("project_service=[\"iam.googleapis.com\",\"sts.googleapis.com\",]"), "{}", imported.satz);
        assert!(c.contains("name=\"acme-sts.googleapis.com\""), "{}", imported.satz);
        assert!(!imported.satz.contains("element("), "{}", imported.satz);
    }

    /// A list naming one entry twice would expand into two resources under one
    /// address, which `satz transpile` refuses. The repeat folds, and the note
    /// says which entry and how many resources the list became.
    #[test]
    fn a_repeated_list_entry_folds_into_one_resource_with_a_note() {
        let tf = r#"
variable "apis" { default = ["iam.googleapis.com", "logging.googleapis.com", "logging.googleapis.com"] }
variable "regions" { default = ["europe-west3", "europe-west3"] }
resource "google_project" "p" {
  name       = "p"
  project_id = "acme-infra-001"
  org_id     = "123456789012"
}
resource "google_project_service" "s" {
  count   = length(var.apis)
  project = google_project.p.project_id
  service = element(var.apis, count.index)
}
resource "google_storage_bucket" "b" {
  count    = length(var.regions)
  name     = "acme-${var.regions[count.index]}"
  location = var.regions[count.index]
  project  = google_project.p.project_id
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        let by = by_what(&imported);
        assert_eq!(by["resource \"google_project_service\" \"s\""], &Action::Expanded(2));
        assert_eq!(by["resource \"google_storage_bucket\" \"b\""], &Action::Expanded(1));
        assert_eq!(imported.satz.matches("\"logging.googleapis.com\",").count(), 3, "the param keeps both, the list one:\n{}", imported.satz);
        assert!(
            imported.notes.iter().any(|n| n.contains("`apis` names `logging.googleapis.com` more than once") && n.contains("2 resource(s) for 3 entries")),
            "{:?}",
            imported.notes
        );
        assert_eq!(imported.satz.matches("b_europe_west3 {").count(), 1, "{}", imported.satz);
    }

    /// Two different entries that shorten to one label keep distinct labels.
    #[test]
    fn two_entries_with_one_short_label_keep_their_own_addresses() {
        let tf = r#"
variable "roles" { default = ["roles/viewer", "organizations/123456789012/roles/viewer"] }
resource "google_storage_bucket" "b" {
  count    = length(var.roles)
  name     = "acme-b"
  location = var.roles[count.index]
  project  = "acme-infra-001"
}
"#;
        let imported = import(&one(tf), "acme", false, Some("123456789012"), &Known).unwrap();
        assert!(imported.satz.contains("b_viewer {") && imported.satz.contains("b_1 {"), "{}", imported.satz);
    }

    /// `DEFAULT_PROVIDER` is what the emitter writes for the estate this import
    /// creates, so the two have to say the same thing.
    #[test]
    fn base_estate_declares_the_default_provider() {
        let top = base_estate().unwrap();
        let google = top
            .get("providers")
            .and_then(|p| p.get("google"))
            .and_then(|g| g.get("alias"))
            .and_then(|a| a.as_str())
            .expect("base_estate declares a google provider with an alias");
        assert_eq!(DEFAULT_PROVIDER, format!("google.{}", google));
    }
}
