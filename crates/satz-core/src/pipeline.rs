//! Stage B pipeline: Satz sources → per-file `Fragment`s → `algebra::fold` →
//! emission from `Folded`. (docs/stage-b.md is the plan of record.)
//!
//! The front-end is deliberately Satz-first: the YAML dialect composes textually
//! (packs reference their includer's anchors and cannot parse standalone), so
//! per-file fragments are only possible where parameters have real scoping —
//! which Satz has. Params resolve here, in the front-end; the fold never sees a
//! parameter, only canonical bodies.
//!
//! v0 (increment I0) covers: param resolution (pack defaults, user of the pack
//! wins), `use` recursion (one Fragment per source file), the `folder` tree,
//! entity and grant bodies. Emission is a minimal deterministic rendering —
//! byte-parity with the walk emitter is increment I1+ and is gated by the
//! differential harness, not by this module's tests.

use crate::algebra::{fold, Body, Entity, Folded, Fragment, GrantEdge, TypeTable};
use crate::satz::{self, Entry, File, Key, StrPart, Value};
use crate::{Address, MergeClass, Scope, Span};
use std::collections::BTreeMap;

/// How a source-level mapping key becomes a Terraform type. Schema-driven in
/// production (the harness/bin supplies it); explicit tables in tests. Returning
/// `None` marks the key as config-layer (terraform/providers/…) — not an entity.
pub trait TypeResolver {
    fn resolve(&self, key: &str) -> Option<ResolvedType>;
}

#[derive(Debug, Clone)]
pub struct ResolvedType {
    pub tf_type: String,
    pub class: MergeClass,
    pub scope: Scope,
}

#[derive(Debug)]
pub struct PipelineError {
    pub file: String,
    pub line: usize,
    pub msg: String,
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.msg)
    }
}
impl std::error::Error for PipelineError {}

fn perr<T>(file: &str, line: usize, msg: impl Into<String>) -> Result<T, PipelineError> {
    Err(PipelineError { file: file.to_string(), line, msg: msg.into() })
}

/// A resolved parameter environment: name → canonical value. Built once per
/// source file — the using file's bindings win over the pack's defaults
/// (Default < Set: the using document's binding wins).
pub type Env = BTreeMap<String, serde_yaml::Value>;

fn resolve_str(parts: &[StrPart], env: &Env, file: &str, line: usize) -> Result<String, PipelineError> {
    let mut out = String::new();
    for p in parts {
        match p {
            StrPart::Lit(s) => out.push_str(s),
            StrPart::Param(name) => match env.get(name) {
                Some(serde_yaml::Value::String(s)) => out.push_str(s),
                Some(serde_yaml::Value::Number(n)) => out.push_str(&n.to_string()),
                Some(serde_yaml::Value::Bool(b)) => out.push_str(if *b { "true" } else { "false" }),
                Some(v) => {
                    return perr(file, line, format!("param '{}' is not a scalar ({:?}) — cannot interpolate", name, v))
                }
                None => return perr(file, line, format!("unknown param '{}'", name)),
            },
        }
    }
    Ok(out)
}

fn resolve_value(v: &Value, env: &Env, file: &str, line: usize) -> Result<serde_yaml::Value, PipelineError> {
    Ok(match v {
        Value::Str(parts) => serde_yaml::Value::String(resolve_str(parts, env, file, line)?),
        Value::Num(n) => serde_yaml::from_str(n)
            .map_err(|e| PipelineError { file: file.to_string(), line, msg: format!("bad number '{}': {}", n, e) })?,
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        Value::Ref(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| PipelineError { file: file.to_string(), line, msg: format!("unknown param '{}'", name) })?,
        Value::List(items) => serde_yaml::Value::Sequence(
            items.iter().map(|i| resolve_value(i, env, file, line)).collect::<Result<_, _>>()?,
        ),
        Value::Obj(entries) => serde_yaml::Value::Mapping(resolve_obj(entries, env, file)?),
    })
}

fn resolve_key(k: &Key, env: &Env, file: &str, line: usize) -> Result<String, PipelineError> {
    match k {
        Key::Ident(s) => Ok(s.clone()),
        Key::Str(parts) => resolve_str(parts, env, file, line),
    }
}

fn resolve_obj(entries: &[Entry], env: &Env, file: &str) -> Result<serde_yaml::Mapping, PipelineError> {
    let mut map = serde_yaml::Mapping::new();
    for e in entries {
        match e {
            Entry::Attr { key, value, line } => {
                map.insert(
                    serde_yaml::Value::String(resolve_key(key, env, file, *line)?),
                    resolve_value(value, env, file, *line)?,
                );
            }
            Entry::Map { key, name, body, line } => {
                let inner = serde_yaml::Value::Mapping(resolve_obj(body, env, file)?);
                let k = resolve_key(key, env, file, *line)?;
                match name {
                    None => {
                        map.insert(serde_yaml::Value::String(k), inner);
                    }
                    Some(n) => {
                        let mut named = serde_yaml::Mapping::new();
                        named.insert(serde_yaml::Value::String(resolve_key(n, env, file, *line)?), inner);
                        map.insert(serde_yaml::Value::String(k), serde_yaml::Value::Mapping(named));
                    }
                }
            }
            Entry::Use { line, .. } => {
                return perr(file, *line, "use inside a resource body is not supported")
            }
        }
    }
    Ok(map)
}

/// Build the file's param environment: `outer` (the user of this file) wins over
/// the file's own defaults; params may reference earlier params of either origin.
fn build_env(file: &File, outer: &Env, file_name: &str) -> Result<Env, PipelineError> {
    let mut env = outer.clone();
    // Params may reference each other regardless of declaration order — the same
    // dependency-ordered resolution the YAML emitter uses.
    for (name, v, line) in satz::sort_params_by_deps(&file.params) {
        if env.contains_key(name) {
            continue; // the using document's binding wins (Default < Set)
        }
        let resolved = resolve_value(v, &env, file_name, *line)?;
        env.insert(name.clone(), resolved);
    }
    Ok(env)
}

/// Front-end result for an estate: its fragments and its resolved parameter
/// environment (the emitter needs config-level facts like customer ids).
pub struct FrontEnd {
    pub fragments: Vec<Fragment>,
    pub env: Env,
    /// Estate-level config blocks (terraform, providers), resolved.
    pub config: BTreeMap<String, serde_yaml::Value>,
    /// Union of every included file's declared params (resolved values), first
    /// definition wins — the estate declares before its packs. This mirrors the
    /// YAML dialect's merged `variables:` blocks and feeds tfvars emission.
    pub tfvars: Env,
    /// Resolved estate-level suppressions (subtractive override channel).
    pub suppressions: Vec<ResolvedSuppression>,
    /// Raw `hcl { … }` blocks, in source order, from the estate and every file it
    /// uses. They bypass the fold entirely — that is what "opaque to the proof
    /// layer" means — and are appended verbatim at emission.
    pub hcl: Vec<HclPassthrough>,
    /// Claims declared by the estate and by every file it actually `use`s, in
    /// source order. This is the compliance plane's input: the packs that are
    /// really in this estate, with the witnesses they claim.
    pub claims: Vec<PackClaims>,
}

/// One file's declared claims, carried with the file that declared them.
///
/// The pack NAME is not a sufficient key: a fork (`X.local.satz`) declares the
/// same `pack` name as its pristine twin but may claim different witnesses, so
/// looking claims up by name after the fact can attribute the pristine pack's
/// claims to an estate running the fork. Travelling with the used file removes
/// that whole class of mistake.
#[derive(Debug, Clone, PartialEq)]
pub struct PackClaims {
    pub pack: String,
    pub version: String,
    pub file: String,
    pub claims: Vec<satz::ClaimDecl>,
}

/// One `hcl { … }` block with the file it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct HclPassthrough {
    pub file: String,
    pub body: String,
    pub trust: Option<String>,
    pub line: usize,
}

/// A suppress statement with its interpolations resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSuppression {
    pub tf_type: String,
    pub label: String,
    pub role: Option<String>,
    /// the estate file that declares it — errors point there, not at "suppress"
    pub file: String,
    pub line: usize,
}

/// Apply the estate's suppressions to the folded result. Each one must match —
/// a suppress that stops matching (typo, upstream rename) is a hard error, so
/// stale subtractive config can never silently deploy what it meant to remove.
/// Suppressed resources vanish from emission; any compliance claim whose
/// witness they were then surfaces as broken in `require` — deliberately.
pub fn apply_suppressions(
    folded: &mut Folded,
    suppressions: &[ResolvedSuppression],
) -> Result<(), PipelineError> {
    for sup in suppressions {
        let tf = if sup.tf_type.starts_with("google_") || sup.tf_type.starts_with("__") {
            sup.tf_type.clone()
        } else {
            format!("google_{}", sup.tf_type)
        };
        // Node-scoped grants namespace their labels by structural path; match on
        // the member part. Plain entities match the label exactly.
        let matching: Vec<Address> = folded
            .slots
            .keys()
            .filter(|a| {
                a.tf_type == tf
                    && (a.label == sup.label
                        // `suppress t "folder-a/prj-b::member"` names one node's grant;
                        // a bare member matches that member on every node
                        || a.label
                            .split_once(GRANT_SCOPE_SEP)
                            .is_some_and(|(node, m)| m == sup.label || format!("{}::{}", node, m) == sup.label))
            })
            .cloned()
            .collect();
        if matching.is_empty() {
            return Err(PipelineError {
                file: sup.file.clone(),
                line: sup.line,
                msg: format!(
                    "suppress {} \"{}\" matches nothing — stale suppression (typo or upstream rename)",
                    sup.tf_type, sup.label
                ),
            });
        }
        for addr in matching {
            match &sup.role {
                None => {
                    folded.slots.remove(&addr);
                }
                Some(role) => {
                    let Some(crate::algebra::Slot::Ok(entity)) = folded.slots.get_mut(&addr) else {
                        return Err(PipelineError {
                            file: sup.file.clone(),
                            line: sup.line,
                            msg: format!(
                                "suppress … role on {} \"{}\": the address is in conflict (⊥); suppress the whole member or resolve the conflict first",
                                sup.tf_type, sup.label
                            ),
                        });
                    };
                    let Body::Grant(edges) = &mut entity.body else {
                        return Err(PipelineError {
                            file: sup.file.clone(),
                            line: sup.line,
                            msg: format!(
                                "suppress … role on {} \"{}\": not a grant",
                                sup.tf_type, sup.label
                            ),
                        });
                    };
                    let before = edges.len();
                    edges.retain(|e| &e.role != role);
                    if edges.len() == before {
                        return Err(PipelineError {
                            file: sup.file.clone(),
                            line: sup.line,
                            msg: format!(
                                "suppress {} \"{}\" role \"{}\" matches no edge",
                                sup.tf_type, sup.label, role
                            ),
                        });
                    }
                    if edges.is_empty() {
                        folded.slots.remove(&addr);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Compile an estate source to per-file fragments plus the estate environment.
pub fn compile_estate(
    file_name: &str,
    src: &str,
    types: &dyn TypeResolver,
    load: &dyn Fn(&str) -> Result<String, String>,
) -> Result<FrontEnd, PipelineError> {
    let file = satz::parse(src)
        .map_err(|e| PipelineError { file: file_name.to_string(), line: e.line, msg: e.msg })?;
    let env = build_env(&file, &Env::new(), file_name)?;
    let mut own = Fragment::default();
    let mut all = Vec::new();
    let mut w = Walk { types, load, genv: env.clone(), config: BTreeMap::new(), hcl: Vec::new(), claims: Vec::new(), use_chain: vec![file_name.to_string()] };
    w.items(&file.items, file_name, &mut own, &mut all, &[])?;
    let tfvars = w.genv;
    let config = w.config;
    let mut hcl: Vec<HclPassthrough> = file
        .hcl_blocks
        .iter()
        .map(|h| HclPassthrough {
            file: file_name.to_string(),
            body: h.body.clone(),
            trust: h.trust.clone(),
            line: h.line,
        })
        .collect();
    hcl.extend(w.hcl);
    let mut suppressions = Vec::new();
    for sup in &file.suppressions {
        suppressions.push(ResolvedSuppression {
            tf_type: sup.tf_type.clone(),
            label: resolve_str(&sup.label, &env, file_name, sup.line)?,
            role: sup
                .role
                .as_ref()
                .map(|r| resolve_str(r, &env, file_name, sup.line))
                .transpose()?,
            file: file_name.to_string(),
            line: sup.line,
        });
    }
    all.insert(0, own);
    // The estate's own claims come first: an estate may claim a control it
    // implements inline, exactly as a pack does.
    let mut claims = Vec::new();
    if !file.claims.is_empty() {
        claims.push(pack_claims(&file, file_name));
    }
    claims.extend(w.claims);
    Ok(FrontEnd { fragments: all, env, config, tfvars, suppressions, hcl, claims })
}

fn pack_claims(file: &satz::File, file_name: &str) -> PackClaims {
    PackClaims {
        pack: file.estate.clone().unwrap_or_default(),
        version: file.version.clone().unwrap_or_else(|| "unversioned".to_string()),
        file: file_name.to_string(),
        claims: file.claims.clone(),
    }
}

/// The estate's merged parameter table: its own params plus those of every file
/// it `use`s, first definition wins. Commands that need only the variable table
/// — the org-policy trio, bootstrap — can stop here instead of routing the
/// estate through generated YAML to read a `variables:` block back out.
///
/// It resolves no types at all, and that is the point: a parameter table does
/// not depend on merge class or intrinsic scope, so demanding a loaded schema
/// registry would be dead weight — and a resolver that GUESSES instead is worse
/// than none. Two attempts at guessing failed on real estates: one called
/// `google_organization_iam_member` an Entity and rejected every grant pack in
/// the fleet, the other read `labels { … }` inside a project as a resource map
/// named `google_labels`. Nested attribute blocks and resource maps are the same
/// syntax; only a schema tells them apart, and this walk does not need to know.
///
/// `both_routes_agree_on_the_variable_table` in the bin pins the result against
/// the YAML route it replaced.
pub fn estate_params(
    file_name: &str,
    src: &str,
    load: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Env, PipelineError> {
    let file = satz::parse(src)
        .map_err(|e| PipelineError { file: file_name.to_string(), line: e.line, msg: e.msg })?;
    let mut env = build_env(&file, &Env::new(), file_name)?;
    collect_params(&file.items, file_name, load, &mut env, 0)?;
    Ok(env)
}

/// Depth cap for the `use` recursion. It cannot fire for a real estate — the
/// fleet's deepest chain is two — and exists only so a cyclic `use` reports an
/// error instead of overflowing the stack.
const MAX_USE_DEPTH: usize = 64;

/// Absorb params from every `use`d file, depth-first in document order, first
/// definition wins. Descends through resource maps and folders because `use` is
/// legal in all three positions, but deliberately does NOT resolve types: a
/// resource body's nested attribute blocks (`labels { … }`, `spec { … }`) are
/// indistinguishable from resource maps without a schema, and guessing made this
/// walk reject `google_labels` on estates `transpile` compiles happily.
fn collect_params(
    items: &[Entry],
    file_name: &str,
    load: &dyn Fn(&str) -> Result<String, String>,
    env: &mut Env,
    depth: usize,
) -> Result<(), PipelineError> {
    if depth > MAX_USE_DEPTH {
        return perr(file_name, 0, format!("`use` nested more than {} deep — cyclic?", MAX_USE_DEPTH));
    }
    for item in items {
        match item {
            Entry::Attr { .. } => {}
            Entry::Use { path, when, line, .. } => {
                if let Some(p) = when {
                    if !env.contains_key(p) {
                        return perr(file_name, *line, format!("use … when {}: unknown param `{}` — a `when` on a param nobody declares would silently drop the pack", p, p));
                    }
                    if !truthy(env.get(p)) {
                        continue;
                    }
                }
                if path.ends_with(".yaml") || path.ends_with(".yml") {
                    return perr(file_name, *line, format!("use \"{}\": packs are Satz — convert it first: `satz import {} --kind pack`", path, path));
                }
                let src = (load)(path)
                    .map_err(|e| PipelineError { file: file_name.to_string(), line: *line, msg: e })?;
                let used = satz::parse(&src)
                    .map_err(|e| PipelineError { file: path.to_string(), line: e.line, msg: e.msg })?;
                for (name, v, pline) in satz::sort_params_by_deps(&used.params) {
                    if env.contains_key(name) {
                        continue;
                    }
                    let resolved = resolve_value(v, env, path, *pline)?;
                    env.insert(name.clone(), resolved);
                }
                collect_params(&used.items, path, load, env, depth + 1)?;
            }
            Entry::Map { body, .. } => collect_params(body, file_name, load, env, depth)?,
        }
    }
    Ok(())
}

/// A Satz resource key as its Terraform type name.
pub fn normalized_tf_type(key: &str) -> String {
    if key.starts_with("google_") {
        key.to_string()
    } else {
        format!("google_{}", key)
    }
}

/// The message for a resource key the type table does not know.
///
/// Satz names Terraform types in full. When the `google_`-prefixed form WOULD
/// resolve, the key is almost certainly the YAML dialect's shorthand, so say so
/// and give the exact replacement instead of just refusing.
fn unknown_type_msg(types: &dyn TypeResolver, key: &str, what: &str) -> String {
    let full = normalized_tf_type(key);
    if full != key && types.resolve(&full).is_some() {
        format!(
            "{} `{}`: unknown resource type. Satz names Terraform types in full — write `{}`. \
             (Leaving the provider prefix off is a YAML-dialect shorthand; it is not Satz.)",
            what, key, full
        )
    } else {
        format!("{} `{}`: unknown resource type", what, key)
    }
}

/// Merge class and intrinsic scope for a resolved Terraform type.
///
/// Whether a key names a resource at all is a schema question and stays with the
/// registry; how that resource COMPOSES is this one. Split out so the fact lives
/// in one place instead of inline in a resolver, where a second resolver would
/// be free to disagree with it.
pub fn type_facts(tf_type: &str) -> (crate::MergeClass, crate::Scope) {
    use crate::{MergeClass, Scope};
    match tf_type {
        "google_cloud_identity_group" => (MergeClass::Entity, Scope::Customer),
        "google_organization_iam_member" => (MergeClass::Grant, Scope::Org),
        "google_billing_account_iam_member" => (MergeClass::Grant, Scope::Billing),
        t if t.ends_with("iam_member") => (MergeClass::Grant, Scope::Node),
        _ => (MergeClass::Entity, Scope::Node),
    }
}

/// One fragment per source file, recursively over `use`. `load` maps a use-path
/// to source text (the bin supplies file access; this module stays pure).
pub fn fragments_from_source(
    file_name: &str,
    src: &str,
    outer_env: &Env,
    types: &dyn TypeResolver,
    load: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Vec<Fragment>, PipelineError> {
    let file = satz::parse(src)
        .map_err(|e| PipelineError { file: file_name.to_string(), line: e.line, msg: e.msg })?;
    let env = build_env(&file, outer_env, file_name)?;
    let mut own = Fragment::default();
    let mut all = Vec::new();
    let mut w = Walk { types, load, genv: env, config: BTreeMap::new(), hcl: Vec::new(), claims: Vec::new(), use_chain: vec![file_name.to_string()] };
    w.items(&file.items, file_name, &mut own, &mut all, &[])?;
    all.insert(0, own);
    Ok(all)
}

fn truthy(v: Option<&serde_yaml::Value>) -> bool {
    match v {
        Some(serde_yaml::Value::Bool(b)) => *b,
        Some(serde_yaml::Value::String(s)) => !s.is_empty() && s != "false",
        Some(serde_yaml::Value::Null) | None => false,
        Some(_) => true,
    }
}

/// The recursive walker. A struct so the `use` arm can thread the tfvars
/// accumulator without widening every signature.
struct Walk<'a> {
    types: &'a dyn TypeResolver,
    load: &'a dyn Fn(&str) -> Result<String, String>,
    /// The accumulated parameter namespace. The YAML dialect's variables merge
    /// into ONE document-ordered namespace (first definition wins, packs see
    /// every earlier file's params) — pipeline B mirrors that exactly. True
    /// lexical pack scoping is a deliberate future semantics change, not a
    /// parity item.
    genv: Env,
    /// Estate-level config blocks (terraform, providers) — resolved values,
    /// consumed by the providers/variables emitters.
    config: BTreeMap<String, serde_yaml::Value>,
    /// The `use` chain from the estate down to the file being walked — a path
    /// already on it is a cycle, reported with the chain instead of a stack
    /// overflow.
    use_chain: Vec<String>,
    /// Raw HCL collected from every file the walk visits.
    hcl: Vec<HclPassthrough>,
    claims: Vec<PackClaims>,
}

impl Walk<'_> {
    /// Absorb a file's declared params: first definition wins, later files see
    /// earlier definitions. Returns the file's declared names (tfvars dedup is
    /// implicit — genv IS the tfvars namespace).
    /// Raw `hcl { … }` blocks from a used file, kept in visit order.
    fn absorb_hcl(&mut self, file: &satz::File, file_name: &str) {
        for h in &file.hcl_blocks {
            self.hcl.push(HclPassthrough {
                file: file_name.to_string(),
                body: h.body.clone(),
                trust: h.trust.clone(),
                line: h.line,
            });
        }
    }

    /// Claims of a `use`d file. Called only after the `when` guard passed, so a
    /// pack the estate did not actually pull in contributes no claims — an
    /// unused pack must never make a control look satisfied.
    fn absorb_claims(&mut self, file: &satz::File, file_name: &str) {
        if file.claims.is_empty() {
            return;
        }
        self.claims.push(pack_claims(file, file_name));
    }

    fn absorb_params(&mut self, file: &File, file_name: &str) -> Result<(), PipelineError> {
        for (name, v, line) in satz::sort_params_by_deps(&file.params) {
            if self.genv.contains_key(name) {
                continue;
            }
            let resolved = resolve_value(v, &self.genv, file_name, *line)?;
            self.genv.insert(name.clone(), resolved);
        }
        Ok(())
    }
}

impl Walk<'_> {
    /// `use … when <param>`: a param nobody declares is an error, not `false`
    /// — a typo would otherwise drop the pack without a word.
    fn when_holds(&self, param: &str, file_name: &str, line: usize) -> Result<bool, PipelineError> {
        if !self.genv.contains_key(param) {
            return perr(
                file_name,
                line,
                format!("use … when {}: unknown param `{}` — a `when` on a param nobody declares would silently drop the pack", param, param),
            );
        }
        Ok(truthy(self.genv.get(param)))
    }

    /// Load and parse a `use`d file, absorb its params/hcl/claims, and put it
    /// on the chain. A path already on the chain is a cycle, named in full.
    /// The caller pops the chain after descending.
    fn enter_use(&mut self, use_path: &str, file_name: &str, line: usize) -> Result<satz::File, PipelineError> {
        if use_path.ends_with(".yaml") || use_path.ends_with(".yml") {
            return perr(file_name, line, format!("use \"{}\": packs are Satz — convert it first: `satz import {} --kind pack`", use_path, use_path));
        }
        if self.use_chain.iter().any(|f| f == use_path) {
            return perr(
                file_name,
                line,
                format!("cyclic `use`: {} → {}", self.use_chain.join(" → "), use_path),
            );
        }
        let src = (self.load)(use_path).map_err(|e| PipelineError { file: file_name.to_string(), line, msg: e })?;
        let file = satz::parse(&src).map_err(|e| PipelineError { file: use_path.to_string(), line: e.line, msg: e.msg })?;
        self.absorb_params(&file, use_path)?;
        self.absorb_hcl(&file, use_path);
        self.absorb_claims(&file, use_path);
        self.use_chain.push(use_path.to_string());
        Ok(file)
    }

    fn items(
        &mut self,
        items: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        all: &mut Vec<Fragment>,
        path: &[String],
    ) -> Result<(), PipelineError> {
        for item in items {
            match item {
                // Estate-level scalar attrs (customer ids …) are config layer in v0.
                // A bare attribute at the top of a file belongs to nothing —
                // silently ignoring it once made 22 project services vanish.
                Entry::Attr { key, line, .. } => {
                    return perr(
                        file_name,
                        *line,
                        format!("`{:?}` is an attribute at the top level of the file — attributes live inside a resource block", key),
                    );
                }
                Entry::Use { path: use_path, as_key, when, line } => {
                    if let Some(p) = when {
                        if !self.when_holds(p, file_name, *line)? {
                            continue;
                        }
                    }
                    let file = self.enter_use(use_path, file_name, *line)?;
                    let mut child_own = Fragment::default();
                    match as_key {
                        // `use "…" as key`: the pack's top-level entries are the
                        // CONTENT of a resource map keyed by `key`.
                        Some(k) => match self.types.resolve(k) {
                            Some(rt) => self.resource_map(&rt, None, &file.items, use_path, &mut child_own, *line, path, None)?,
                            None => {
                                return perr(file_name, *line, unknown_type_msg(self.types, k, "use … as"))
                            }
                        },
                        None => self.items(&file.items, use_path, &mut child_own, all, path)?,
                    }
                    self.use_chain.pop();
                    all.push(child_own);
                }
                Entry::Map { key, name, body, line } => {
                    let k = resolve_key(key, &self.genv, file_name, *line)?;
                    // Folders and projects are STRUCTURAL — they open a scope
                    // rather than merely declaring a resource — but they are not
                    // magic bare words. Satz has no keyword resource types: the
                    // truth about what a type is called lives in the provider
                    // schemas, so these are named there like everything else and
                    // a bare `google_folder {` falls through to the same "write
                    // google_folder" error as any other shorthand.
                    if k == "google_folder" {
                        self.folder(name.as_ref(), body, file_name, own, all, *line, path)?;
                        continue;
                    }
                    if k == "google_project" {
                        self.project(name.as_ref(), body, file_name, own, all, *line, path)?;
                        continue;
                    }
                    match self.types.resolve(&k) {
                        None => {
                            if k == "terraform" || k == "providers" {
                                let resolved = serde_yaml::Value::Mapping(resolve_obj(body, &self.genv, file_name)?);
                                if self.config.contains_key(&k) {
                                    return perr(file_name, *line, format!("`{}` is declared twice — one block per estate", k));
                                }
                                self.config.insert(k, resolved);
                            } else {
                                // Previously ignored. Silently dropping a block the
                                // author wrote means a typo — or a dialect shorthand —
                                // deletes infrastructure with no diagnostic at all.
                                return perr(file_name, *line, unknown_type_msg(self.types, &k, "block"));
                            }
                        }
                        Some(rt) => {
                            self.resource_map(&rt, name.as_ref(), body, file_name, own, *line, path, None)?
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn folder(
        &mut self,
        name: Option<&Key>,
        body: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        all: &mut Vec<Fragment>,
        line: usize,
        path: &[String],
    ) -> Result<(), PipelineError> {
        match name {
            // `google_folder { a {…} b {…} }` — each named child is a folder node.
            None => {
                for e in body {
                    match e {
                        Entry::Map { key, name: None, body, line } => {
                            let fname = resolve_key(key, &self.genv, file_name, *line)?;
                            self.folder_node(&fname, body, file_name, own, all, *line, path)?;
                        }
                        Entry::Map { key, name: Some(n), body, line } => {
                            let fname = resolve_key(n, &self.genv, file_name, *line)?;
                            let _ = resolve_key(key, &self.genv, file_name, *line)?;
                            self.folder_node(&fname, body, file_name, own, all, *line, path)?;
                        }
                        // `google_folder { use "…" }` — the pack's items are folder-map
                        // content (named folder nodes), in their own fragment.
                        Entry::Use { path: use_path, as_key, when, line } => {
                            if let Some(p) = when {
                                if !self.when_holds(p, file_name, *line)? {
                                    continue;
                                }
                            }
                            let file = self.enter_use(use_path, file_name, *line)?;
                            let mut child_own = Fragment::default();
                            match as_key {
                                // `use "…" as <type>` inside a folder: the pack is the
                                // content of that resource map, scoped to the folder
                                Some(k) => match self.types.resolve(k) {
                                    Some(rt) => self.resource_map(&rt, None, &file.items, use_path, &mut child_own, *line, path, None)?,
                                    None => return perr(file_name, *line, unknown_type_msg(self.types, k, "use … as")),
                                },
                                None => self.folder(None, &file.items, use_path, &mut child_own, all, *line, path)?,
                            }
                            self.use_chain.pop();
                            all.push(child_own);
                        }
                        other => {
                            let l = match other {
                                Entry::Attr { line, .. } => *line,
                                Entry::Map { line, .. } | Entry::Use { line, .. } => *line,
                            };
                            return perr(file_name, l, "unexpected entry directly under `folder`");
                        }
                    }
                }
                Ok(())
            }
            Some(n) => {
                let fname = resolve_key(n, &self.genv, file_name, line)?;
                self.folder_node(&fname, body, file_name, own, all, line, path)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn folder_node(
        &mut self,
        fname: &str,
        body: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        all: &mut Vec<Fragment>,
        line: usize,
        path: &[String],
    ) -> Result<(), PipelineError> {
        // The folder itself is a Node-scoped entity. Entries whose key names a
        // resource type (or `folder`/`project`/`use`) are children; everything
        // else — attrs, labels-style maps — is folder body.
        let (attrs, children) = self.split_body(body, file_name)?;
        insert_entity(
            own,
            Address { tf_type: "google_folder".to_string(), label: fname.to_string() },
            Scope::Node,
            Body::Attrs(serde_yaml::Value::Mapping(attrs)),
            file_name,
            line,
            path,
        )?;
        let mut child_path = path.to_vec();
        child_path.push(format!("folder:{}", fname));
        self.items(&children, file_name, own, all, &child_path)
    }

    /// Split a structural node's body into (body attrs, child resource entries).
    fn split_body(
        &mut self,
        body: &[Entry],
        file_name: &str,
    ) -> Result<(serde_yaml::Mapping, Vec<Entry>), PipelineError> {
        let mut attrs = serde_yaml::Mapping::new();
        let mut children = Vec::new();
        for e in body {
            match e {
                Entry::Attr { key, value, line } => {
                    attrs.insert(
                        serde_yaml::Value::String(resolve_key(key, &self.genv, file_name, *line)?),
                        resolve_value(value, &self.genv, file_name, *line)?,
                    );
                }
                Entry::Use { .. } => children.push(e.clone()),
                Entry::Map { key, line, .. } => {
                    let k = resolve_key(key, &self.genv, file_name, *line)?;
                    // Routing, not validation. A key that resolves only in its
                    // `google_`-prefixed form is still ROUTED to the children —
                    // where `items` rejects it by name with the exact
                    // replacement. Deciding "attribute" here instead would file
                    // a resource block away as a nested attribute of its parent
                    // and emit nothing, which is how the first cut of this
                    // change quietly deleted 60 folder_iam_member bindings from
                    // one estate's bindings and most of another's project resources: transpile
                    // returned 0 and the HCL just got smaller.
                    // A key spelled like a provider type is a child whether or
                    // not it resolves: an unknown `google_…` is a typo `items`
                    // rejects by name, never an attribute of its parent.
                    let is_child = k == "google_folder"
                        || k == "google_project"
                        || k.starts_with("google_")
                        || (k != "project_service"
                            && (self.types.resolve(&k).is_some()
                                || self.types.resolve(&normalized_tf_type(&k)).is_some()));
                    if is_child {
                        children.push(e.clone());
                    } else if let Entry::Map { body, .. } = e {
                        attrs.insert(
                            serde_yaml::Value::String(k),
                            serde_yaml::Value::Mapping(resolve_obj(body, &self.genv, file_name)?),
                        );
                    }
                }
            }
        }
        Ok((attrs, children))
    }

    /// `google_project { name {…} }` — structural like folder: the project entity's body
    /// keeps attrs (incl. labels / project_service); resource children recurse
    /// with the project pushed onto the node path.
    #[allow(clippy::too_many_arguments)]
    fn project(
        &mut self,
        name: Option<&Key>,
        body: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        all: &mut Vec<Fragment>,
        line: usize,
        path: &[String],
    ) -> Result<(), PipelineError> {
        let named: Vec<(String, &[Entry], usize)> = match name {
            Some(n) => vec![(resolve_key(n, &self.genv, file_name, line)?, body, line)],
            None => {
                let mut v = Vec::new();
                for e in body {
                    match e {
                        Entry::Map { key, name: None, body, line } => {
                            v.push((resolve_key(key, &self.genv, file_name, *line)?, body.as_slice(), *line))
                        }
                        other => {
                            let l = match other {
                                Entry::Attr { line, .. } | Entry::Use { line, .. } => *line,
                                Entry::Map { line, .. } => *line,
                            };
                            return perr(file_name, l, "unexpected entry directly under `project`");
                        }
                    }
                }
                v
            }
        };
        for (pname, pbody, pline) in named {
            let (mut attrs, children) = self.split_body(pbody, file_name)?;
            // project_service may arrive as an Attr list (already in attrs) or as
            // a Map — split_body routed non-resource maps into attrs already.
            if !attrs.contains_key(serde_yaml::Value::String("project_id".into())) {
                attrs.insert(
                    serde_yaml::Value::String("project_id".into()),
                    serde_yaml::Value::String(pname.clone()),
                );
            }
            insert_entity(
                own,
                Address { tf_type: "google_project".to_string(), label: pname.to_string() },
                Scope::Node,
                Body::Attrs(serde_yaml::Value::Mapping(attrs)),
                file_name,
                pline,
                path,
            )?;
            let mut child_path = path.to_vec();
            child_path.push(format!("project:{}", pname));
            self.items(&children, file_name, own, all, &child_path)?;
        }
        Ok(())
    }
}

impl Walk<'_> {
    /// The scope attribute a grant map pins, if any — read before the members
    /// so declaration order does not matter. `google_billing_account_iam_member`
    /// keeps its own channel: its account is estate-wide, pinned once as a
    /// synthetic entity (idempotent, conflicting pins = ⊥) with a fallback to
    /// `billing_account_infra`, not per-map like every other scope.
    fn collect_scope_pin(
        &mut self,
        rt: &ResolvedType,
        name: Option<&Key>,
        body: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        path: &[String],
    ) -> Result<Option<ScopePin>, PipelineError> {
        if rt.class != MergeClass::Grant || name.is_some() {
            return Ok(None);
        }
        let mut pin: Option<(ScopePin, usize)> = None;
        for e in body {
            let Entry::Attr { key, value, line } = e else { continue };
            let key = resolve_key(key, &self.genv, file_name, *line)?;
            if !is_scope_attr_key(&key) {
                continue;
            }
            let value = resolve_value(value, &self.genv, file_name, *line)?;
            let Some(value) = value.as_str() else {
                return perr(
                    file_name,
                    *line,
                    format!("`{}` is a scope attribute of `{}`, so its value must be a string — a list here reads as a member's roles, and `{}` is not a member (a member is `<type>:<value>`)", key, rt.tf_type, key),
                );
            };
            if rt.tf_type == "google_billing_account_iam_member" && key == "billing_account_id" {
                insert_entity(
                    own,
                    Address { tf_type: BILLING_ID_TYPE.to_string(), label: "billing_account_id".to_string() },
                    Scope::Billing,
                    Body::Attrs(serde_yaml::Value::String(value.to_string())),
                    file_name,
                    *line,
                    path,
                )?;
                continue;
            }
            if rt.scope == Scope::Org {
                return perr(
                    file_name,
                    *line,
                    format!("`{}` takes its scope from the estate's `customer_organization_id`; remove `{}`", rt.tf_type, key),
                );
            }
            if scoped_by_node(&rt.tf_type) && !path.is_empty() {
                return perr(
                    file_name,
                    *line,
                    format!(
                        "`{}` inside `{}` is already scoped by that node; remove `{}` or move the map out of the node",
                        rt.tf_type,
                        path.join("/"),
                        key
                    ),
                );
            }
            if let Some((prev, first)) = &pin {
                if prev.attr != key || prev.value != value {
                    return perr(
                        file_name,
                        *line,
                        format!(
                            "`{}` pins two scopes in one map (`{} = \"{}\"` at line {}, `{} = \"{}\"` here) — one scope per map; repeat the map for the second",
                            rt.tf_type, prev.attr, prev.value, first, key, value
                        ),
                    );
                }
                continue;
            }
            pin = Some((ScopePin { attr: key, value: value.to_string() }, *line));
        }
        Ok(pin.map(|(p, _)| p))
    }

    /// One resource-type map: named entities, grant edges, or `use` lines whose
    /// pack content lands in THIS map. Grant addresses of Node-scoped types are
    /// namespaced by their structural path so grants of different projects never
    /// merge; intrinsic scopes (Org/Billing/Customer) hoist globally.
    #[allow(clippy::too_many_arguments)]
    fn resource_map(
        &mut self,
        rt: &ResolvedType,
        name: Option<&Key>,
        body: &[Entry],
        file_name: &str,
        own: &mut Fragment,
        line: usize,
        path: &[String],
        pin: Option<&ScopePin>,
    ) -> Result<(), PipelineError> {
        // A scope attribute applies to the whole map regardless of where it is
        // written, and to the content of any pack `use`d inside it; a pack that
        // pins its own scope wins for its own content.
        let own_pin = self.collect_scope_pin(rt, name, body, file_name, own, path)?;
        let pin = own_pin.as_ref().or(pin);
        let named: Vec<(String, &[Entry], usize)> = match name {
            Some(n) => vec![(resolve_key(n, &self.genv, file_name, line)?, body, line)],
            None => {
                let mut v = Vec::new();
                for e in body {
                    match e {
                        Entry::Map { key, name: None, body, line } => {
                            v.push((resolve_key(key, &self.genv, file_name, *line)?, body.as_slice(), *line))
                        }
                        Entry::Attr { key, value, line } if rt.class == MergeClass::Grant => {
                            let member = resolve_key(key, &self.genv, file_name, *line)?;
                            // Scope attributes were consumed by the pre-pass.
                            if is_scope_attr_key(&member) {
                                continue;
                            }
                            let roles = resolve_value(value, &self.genv, file_name, *line)?;
                            insert_grant(own, rt, &member, &roles, file_name, *line, path, pin)?;
                            continue;
                        }
                        Entry::Use { path: use_path, as_key, when, line } => {
                            if let Some(p) = when {
                                if !self.when_holds(p, file_name, *line)? {
                                    continue;
                                }
                            }
                            // inside a resource map the map's type IS the key; an
                            // `as` naming another type cannot be honoured here
                            if let Some(k) = as_key {
                                if self.types.resolve(k).map(|t| t.tf_type) != Some(rt.tf_type.clone()) {
                                    return perr(
                                        file_name,
                                        *line,
                                        format!("use … as {} inside `{} {{ … }}`: the pack is this map's content; move the `use` to the folder or top level to re-key it", k, rt.tf_type),
                                    );
                                }
                            }
                            let file = self.enter_use(use_path, file_name, *line)?;
                            // pack content lands in this same fragment-map scope;
                            // its own file identity is preserved via provenance.
                            self.resource_map(rt, None, &file.items, use_path, own, *line, path, pin)?;
                            self.use_chain.pop();
                            continue;
                        }
                        other => {
                            let l = match other {
                                Entry::Attr { line, .. } | Entry::Use { line, .. } => *line,
                                Entry::Map { line, .. } => *line,
                            };
                            return perr(file_name, l, format!("unexpected entry under resource map '{}'", rt.tf_type));
                        }
                    }
                }
                v
            }
        };
        for (label, entries, line) in named {
            let body = Body::Attrs(serde_yaml::Value::Mapping(resolve_obj(entries, &self.genv, file_name)?));
            insert_entity(
                own,
                Address { tf_type: rt.tf_type.clone(), label },
                rt.scope.clone(),
                body,
                file_name,
                line,
                path,
            )?;
        }
        Ok(())
    }
}

/// Separator between a Node-scoped grant's structural path and its member key
/// in the fold address label. The emitter splits on it.
pub const GRANT_SCOPE_SEP: char = '\u{1}';

/// An explicit scope for a grant map: the resource's own scope attribute and
/// its value (`service_account_id = "projects/p/serviceAccounts/x@y"`). Types
/// whose scope is neither the organisation nor the structural node it sits in
/// carry it this way, so the member-map form works for every `*_iam_member`
/// type and not only the hierarchy ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePin {
    pub attr: String,
    pub value: String,
}

impl ScopePin {
    /// The address prefix a pinned grant is namespaced by — visible, because
    /// `suppress` addresses one with `"<attr>=<value>::<member>"`.
    fn prefix(&self) -> String {
        format!("{}={}", self.attr, self.value)
    }
}

/// A key inside a grant map is the scope attribute rather than a member when it
/// cannot be an IAM member: every member is `<type>:<value>` except the two
/// reserved all-principal forms, and no Terraform attribute name contains `:`.
fn is_scope_attr_key(key: &str) -> bool {
    !key.contains(':') && key != "allUsers" && key != "allAuthenticatedUsers"
}

/// Whether a grant type takes its scope from the folder/project it is written
/// in. Mirrors the emitter's test, so both agree on what a pin may override.
fn scoped_by_node(tf_type: &str) -> bool {
    tf_type.contains("project") || tf_type.contains("folder")
}

/// Synthetic address type for an explicitly pinned billing account id
/// (mirrors the walk's BILLING_ID_TYPE) — consumed by the emitter, never emitted.
pub const BILLING_ID_TYPE: &str = "__billing_account_id";

#[allow(clippy::too_many_arguments)]
fn insert_grant(
    own: &mut Fragment,
    rt: &ResolvedType,
    member: &str,
    roles: &serde_yaml::Value,
    file_name: &str,
    line: usize,
    path: &[String],
    pin: Option<&ScopePin>,
) -> Result<(), PipelineError> {
    let list = match roles {
        serde_yaml::Value::Sequence(s) => s.clone(),
        one @ serde_yaml::Value::String(_) => vec![one.clone()],
        other => return perr(file_name, line, format!("grant roles must be a list, got {:?}", other)),
    };
    let mut edges = std::collections::BTreeSet::new();
    for r in list {
        let (role, condition, import_id) = match r {
            serde_yaml::Value::String(s) => (s, String::new(), String::new()),
            // Conditional binding. Satz writes `{ role = "…", condition = { … } }`;
            // the legacy YAML dialect puts the role in the key with a null value
            // (`- roles/x:` followed by a sibling `condition:`). Both are accepted.
            // The condition is part of the binding's IDENTITY — the emitted label
            // hashes it — so it travels with the edge through the fold.
            serde_yaml::Value::Mapping(m) => {
                let mut role = String::new();
                let mut condition = String::new();
                let mut import_id = String::new();
                for (k, v) in &m {
                    let Some(k) = k.as_str() else { continue };
                    match k {
                        "condition" => {
                            condition = serde_yaml::to_string(v)
                                .map_err(|e| PipelineError { file: file_name.to_string(), line, msg: format!("condition is not serialisable: {}", e) })?
                                .trim_end()
                                .to_string();
                        }
                        // Adoption of an existing binding: rides the edge to the
                        // emitter, which writes the `import` block. Used to be
                        // dropped here without a word.
                        "import-id" => {
                            import_id = match v.as_str() {
                                Some(id) => id.to_string(),
                                None => return perr(file_name, line, format!("grant: \"import-id\" must be a string, got {:?}", v)),
                            }
                        }
                        "role" => {
                            role = match v.as_str() {
                                Some(r) => r.to_string(),
                                None => return perr(file_name, line, format!("grant: `role` must be a string, got {:?}", v)),
                            }
                        }
                        // the legacy dialect's null-valued role key beside a
                        // `condition` — the ONLY other key accepted
                        other if v.is_null() && role.is_empty() => role = other.to_string(),
                        other => {
                            return perr(
                                file_name,
                                line,
                                format!("grant: unknown key `{}` in a conditional grant object — the keys are `role`, `condition`, \"import-id\"", other),
                            )
                        }
                    }
                }
                if role.is_empty() {
                    return perr(file_name, line, format!("conditional grant has no role: {:?}", m));
                }
                (role, condition, import_id)
            }
            other => return perr(file_name, line, format!("grant role must be a string or a conditional mapping, got {:?}", other)),
        };
        edges.insert(GrantEdge { member: member.to_string(), role, condition, import_id });
    }
    // An explicit scope namespaces the address, so two maps pinning different
    // scopes never merge into one grant; otherwise a Node-scoped grant is
    // namespaced by the structural path, exactly as before (no address moves).
    let label = match pin {
        Some(p) => format!("{}{}{}", p.prefix(), GRANT_SCOPE_SEP, member),
        None if rt.scope == Scope::Node && !path.is_empty() => {
            // `=` is what tells a pinned prefix from a structural one, so a node
            // label may not contain one. Quoted labels make this reachable.
            if let Some(bad) = path.iter().find(|seg| seg.contains('=')) {
                return perr(
                    file_name,
                    line,
                    format!("`{}` is not a usable folder or project label for a grant: `=` separates a grant's own scope from its member", bad),
                );
            }
            format!("{}{}{}", path.join("/"), GRANT_SCOPE_SEP, member)
        }
        None => member.to_string(),
    };
    let addr = Address { tf_type: rt.tf_type.clone(), label };
    let span = Span { file: file_name.to_string(), line: line as u32 };
    match own.entities.get_mut(&addr) {
        Some(Entity { body: Body::Grant(existing), provenance, .. }) => {
            existing.extend(edges);
            provenance.push(span);
        }
        Some(_) => return perr(file_name, line, "grant address already carries a non-grant body"),
        None => {
            own.entities.insert(
                addr.clone(),
                Entity { addr, scope: rt.scope.clone(), body: Body::Grant(edges), provenance: vec![span], node_path: path.to_vec() },
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_entity(
    own: &mut Fragment,
    addr: Address,
    scope: Scope,
    body: Body,
    file: &str,
    line: usize,
    node_path: &[String],
) -> Result<(), PipelineError> {
    let span = Span { file: file.to_string(), line: line as u32 };
    // Within one source file the same address may recur only with the SAME
    // body (idempotent, provenance accumulates); a different body is the
    // conflict the fold would raise across files, raised here with both lines.
    match own.entities.get_mut(&addr) {
        Some(existing) if existing.body == body => existing.provenance.push(span),
        Some(existing) => {
            let first = existing.provenance.first().map(|s| s.line).unwrap_or(0);
            return perr(
                file,
                line,
                format!("{}.{} is declared twice in this file with different bodies (first at line {})", addr.tf_type, addr.label, first),
            );
        }
        None => {
            own.entities.insert(
                addr.clone(),
                Entity { addr, scope, body, provenance: vec![span], node_path: node_path.to_vec() },
            );
        }
    }
    Ok(())
}

/// Fold the fragments. Thin wrapper so callers never touch `algebra` directly.
pub fn fold_fragments(table: &dyn TypeTable, frags: &[Fragment]) -> Folded {
    fold(table, frags)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Table;
    impl TypeResolver for Table {
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            match key {
                "google_cloud_identity_group" => Some(ResolvedType {
                    tf_type: "google_cloud_identity_group".into(),
                    class: MergeClass::Entity,
                    scope: Scope::Customer,
                }),
                "google_organization_iam_member" => Some(ResolvedType {
                    tf_type: "google_organization_iam_member".into(),
                    class: MergeClass::Grant,
                    scope: Scope::Org,
                }),
                _ => None,
            }
        }
    }
    impl TypeTable for Table {
        fn merge_class(&self, t: &str) -> MergeClass {
            if t.ends_with("_iam_member") { MergeClass::Grant } else { MergeClass::Entity }
        }
        fn scope(&self, _t: &str) -> Scope {
            Scope::Node
        }
    }

    const ESTATE: &str = r#"estate t

params {
  customer_domain = "example.com"
}

google_folder {
  a {
    display_name = "A"
    google_cloud_identity_group {
      "log-admins" { display_name = "Log Admins" }
    }
    google_organization_iam_member {
      "group:log-admins@{customer_domain}" = ["roles/logging.admin"]
    }
  }
  b {
    display_name = "B"
    google_cloud_identity_group {
      "log-admins" { display_name = "Log Admins" }
    }
    google_organization_iam_member {
      "group:log-admins@{customer_domain}" = ["roles/logging.admin", "roles/monitoring.admin"]
    }
  }
}
"#;

    #[test]
    fn identical_groups_across_folders_are_one_entity_and_grants_union() {
        let frags = fragments_from_source("t.satz", ESTATE, &Env::new(), &Table, &|p| {
            Err(format!("no loads expected, got {}", p))
        })
        .unwrap();
        assert_eq!(frags.len(), 1);
        let folded = fold_fragments(&Table, &frags);
        assert!(folded.conflicts().is_empty(), "{:?}", folded.conflicts());
        // 2 folders + 1 group (hoisted by idempotence) + 1 grant address
        assert_eq!(folded.slots.len(), 4);
        let grant = folded
            .slots
            .get(&Address {
                tf_type: "google_organization_iam_member".into(),
                label: "group:log-admins@example.com".into(),
            })
            .unwrap();
        match grant {
            crate::algebra::Slot::Ok(Entity { body: Body::Grant(edges), .. }) => {
                let roles: Vec<&str> = edges.iter().map(|e| e.role.as_str()).collect();
                assert_eq!(roles, vec!["roles/logging.admin", "roles/monitoring.admin"]);
            }
            other => panic!("expected grant, got {:?}", other),
        }
    }

    #[test]
    fn pack_defaults_lose_to_the_using_documents_binding() {
        let pack = r#"pack p

params {
  who = "default@example.com"
}

google_organization_iam_member {
  "user:{who}" = ["roles/viewer"]
}
"#;
        let estate = r#"estate t

params {
  who = "set@example.com"
}

use "p.satz"
"#;
        let frags = fragments_from_source("t.satz", estate, &Env::new(), &Table, &|p| {
            assert_eq!(p, "p.satz");
            Ok(pack.to_string())
        })
        .unwrap();
        assert_eq!(frags.len(), 2, "estate fragment + pack fragment");
        let folded = fold_fragments(&Table, &frags);
        assert!(folded
            .slots
            .contains_key(&Address {
                tf_type: "google_organization_iam_member".into(),
                label: "user:set@example.com".into()
            }));
    }

    #[test]
    fn conflicting_entity_bodies_across_files_bottom_out_with_both_origins() {
        let pack = r#"pack p

google_cloud_identity_group {
  "g" { display_name = "Pack view" }
}
"#;
        let estate = r#"estate t

google_cloud_identity_group {
  "g" { display_name = "Estate view" }
}

use "p.satz"
"#;
        let frags = fragments_from_source("t.satz", estate, &Env::new(), &Table, &|_| Ok(pack.to_string())).unwrap();
        let folded = fold_fragments(&Table, &frags);
        let conflicts = folded.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].candidates.len(), 2);
    }
}

#[cfg(test)]
mod estate_channel_tests {
    use super::*;
    use crate::algebra::TypeTable;

    struct Table;
    impl TypeResolver for Table {
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            match key {
                "google_org_policy_policy" => Some(ResolvedType {
                    tf_type: "google_org_policy_policy".into(),
                    class: MergeClass::Entity,
                    scope: Scope::Node,
                }),
                "google_organization_iam_member" => Some(ResolvedType {
                    tf_type: "google_organization_iam_member".into(),
                    class: MergeClass::Grant,
                    scope: Scope::Org,
                }),
                _ => None,
            }
        }
    }
    impl TypeTable for Table {
        fn merge_class(&self, t: &str) -> MergeClass {
            if t.ends_with("iam_member") { MergeClass::Grant } else { MergeClass::Entity }
        }
        fn scope(&self, _t: &str) -> Scope {
            Scope::Node
        }
    }

    const ESTATE: &str = r#"estate t

params {
  who = "sec@example.com"
}

google_org_policy_policy {
  "keep-me" { name = "keep.me" }
  "drop-me" { name = "drop.me" }
}

google_organization_iam_member {
  "group:{who}" = ["roles/viewer", "roles/browser"]
}

suppress google_org_policy_policy "drop-me"
suppress google_organization_iam_member "group:{who}" role "roles/browser"
"#;

    /// Claims must reach the front end from EVERY `use` site, not just the
    /// top-level one. Estates pull packs in as resource-map content
    /// (`google_org_policy_policy { use "cis.satz" }`) and as folder content — both
    /// arms load and parse the pack, and both must absorb its claims. Missing
    /// this made every CIS control read "unmet" while the witnesses were
    /// emitted perfectly.
    #[test]
    fn claims_are_absorbed_from_every_use_site() {
        let pack = r#"pack cis version "1.2"

claim "cis-gcp" "4.0" "1.4" implements {
  resources = ["google_org_policy_policy.no_keys"]
}

"no_keys" { name = "iam.disableServiceAccountKeyCreation" }
"#;
        let estate = r#"estate t

google_org_policy_policy {
  use "cis.satz"
}
"#;
        let fe = compile_estate("t.satz", estate, &Table, &|p| {
            if p == "cis.satz" { Ok(pack.to_string()) } else { Err(format!("no load: {}", p)) }
        })
        .unwrap();
        assert_eq!(fe.claims.len(), 1, "pack claims must reach the front end");
        assert_eq!(fe.claims[0].pack, "cis");
        assert_eq!(fe.claims[0].version, "1.2");
        assert_eq!(fe.claims[0].claims[0].control, "1.4");
    }

    /// A pack the `when` guard skipped was never pulled in, so its claims must
    /// not make a control look satisfied.
    #[test]
    fn claims_of_a_skipped_pack_are_not_absorbed() {
        let pack = r#"pack cis version "1.0"

claim "cis-gcp" "4.0" "1.4" implements { resources = ["google_org_policy_policy.no_keys"] }

"no_keys" { name = "iam.disableServiceAccountKeyCreation" }
"#;
        let estate = r#"estate t

params { want_cis = false }

google_org_policy_policy {
  use "cis.satz" when want_cis
}
"#;
        let fe = compile_estate("t.satz", estate, &Table, &|p| {
            if p == "cis.satz" { Ok(pack.to_string()) } else { Err(format!("no load: {}", p)) }
        })
        .unwrap();
        assert!(fe.claims.is_empty(), "a skipped pack contributes no claims");
    }

    fn folded_after_suppress(src: &str) -> Result<Folded, PipelineError> {
        let fe = compile_estate("t.satz", src, &Table, &|p| Err(format!("no load: {}", p)))?;
        let mut folded = fold_fragments(&Table, &fe.fragments);
        apply_suppressions(&mut folded, &fe.suppressions)?;
        Ok(folded)
    }

    #[test]
    fn suppress_removes_the_resource_and_the_edge_and_interpolates() {
        let folded = folded_after_suppress(ESTATE).unwrap();
        assert!(folded.slots.contains_key(&Address {
            tf_type: "google_org_policy_policy".into(),
            label: "keep-me".into()
        }));
        assert!(!folded.slots.contains_key(&Address {
            tf_type: "google_org_policy_policy".into(),
            label: "drop-me".into()
        }));
        let grant = folded
            .slots
            .get(&Address {
                tf_type: "google_organization_iam_member".into(),
                label: "group:sec@example.com".into(),
            })
            .unwrap();
        match grant {
            crate::algebra::Slot::Ok(Entity { body: Body::Grant(edges), .. }) => {
                let roles: Vec<&str> = edges.iter().map(|e| e.role.as_str()).collect();
                assert_eq!(roles, vec!["roles/viewer"]);
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn stale_suppression_is_a_hard_error() {
        let src = ESTATE.replace(r#"suppress google_org_policy_policy "drop-me""#, r#"suppress google_org_policy_policy "renamed-upstream""#);
        let err = folded_after_suppress(&src).unwrap_err();
        assert!(err.msg.contains("matches nothing"), "{}", err.msg);
    }

    #[test]
    fn suppressing_the_last_edge_removes_the_grant_entity() {
        let extra = concat!(
            r#"suppress google_organization_iam_member "group:{who}" role "roles/browser""#,
            "\n",
            r#"suppress google_organization_iam_member "group:{who}" role "roles/viewer""#,
        );
        let src = ESTATE.replace(
            r#"suppress google_organization_iam_member "group:{who}" role "roles/browser""#,
            extra,
        );
        let folded = folded_after_suppress(&src).unwrap();
        assert!(!folded
            .slots
            .keys()
            .any(|a| a.tf_type == "google_organization_iam_member"));
    }

    #[test]
    fn hcl_passthrough_is_collected_from_estate_and_packs_in_order() {
        let estate = concat!(
            "estate e\n",
            "params { customer_organization_id = \"1\" }\n",
            "hcl { output \"from_estate\" { value = 1 } }\n",
            "use \"p.satz\"\n",
        );
        let pack = concat!(
            "pack p version \"1.0\"\n",
            "hcl trust \"vendor snippet, reviewed\" { output \"from_pack\" { value = 2 } }\n",
        );
        let fe = compile_estate("t.satz", estate, &Table, &|_| Ok(pack.to_string())).unwrap();
        assert_eq!(fe.hcl.len(), 2);
        // estate's own block first, then the used pack's
        assert_eq!(fe.hcl[0].file, "t.satz");
        assert!(fe.hcl[0].body.contains("from_estate"));
        assert!(fe.hcl[0].trust.is_none());
        assert_eq!(fe.hcl[1].file, "p.satz");
        assert!(fe.hcl[1].body.contains("from_pack"));
        assert_eq!(fe.hcl[1].trust.as_deref(), Some("vendor snippet, reviewed"));
        // Opaque to the proof layer: raw HCL never becomes an entity.
        let folded = fold_fragments(&Table, &fe.fragments);
        assert!(folded.slots.is_empty(), "{:?}", folded.slots.keys().collect::<Vec<_>>());
    }


    #[test]
    fn conditional_grants_carry_their_condition_through_the_fold() {
        // Both spellings must produce the same edge: Satz's explicit
        // `{ role = …, condition = … }` and the legacy YAML dialect's
        // null-valued role key with a sibling `condition:`.
        let satz_form = concat!(
            "estate e\n",
            "params { customer_organization_id = \"1\" }\n",
            "google_organization_iam_member {\n",
            "  \"serviceAccount:svc@p.iam.gserviceaccount.com\" = [\n",
            "    \"roles/viewer\",\n",
            "    {\n",
            "      role = \"roles/storage.objectViewer\"\n",
            "      condition {\n",
            "        title = \"Read reports only\"\n",
            "        expression = \"resource.name.startsWith(x)\"\n",
            "      }\n",
            "    },\n",
            "  ]\n",
            "}\n",
        );
        let fe = compile_estate("t.satz", satz_form, &Table, &|p| Err(format!("no load: {}", p))).unwrap();
        let folded = fold_fragments(&Table, &fe.fragments);
        let edges = folded
            .slots
            .values()
            .find_map(|s| match s {
                crate::algebra::Slot::Ok(e) => match &e.body {
                    crate::algebra::Body::Grant(edges) => Some(edges.clone()),
                    _ => None,
                },
                _ => None,
            })
            .expect("a grant entity");
        assert_eq!(edges.len(), 2, "{:?}", edges);
        let plain = edges.iter().find(|e| e.role == "roles/viewer").expect("plain role");
        assert!(plain.condition.is_empty());
        let cond = edges
            .iter()
            .find(|e| e.role == "roles/storage.objectViewer")
            .expect("conditional role");
        assert!(cond.condition.contains("Read reports only"), "{}", cond.condition);
        assert!(cond.condition.contains("resource.name.startsWith"), "{}", cond.condition);
        // The condition is part of the binding's identity: same member+role with a
        // different condition must be a DIFFERENT edge, not a duplicate.
        assert_ne!(plain.role, cond.role);
    }

    #[test]
    fn a_conditional_grant_without_a_role_is_an_error() {
        let src = concat!(
            "estate e\n",
            "params { customer_organization_id = \"1\" }\n",
            "google_organization_iam_member {\n",
            "  \"user:a@b.c\" = [ { condition { title = \"t\" } } ]\n",
            "}\n",
        );
        let msg = match compile_estate("t.satz", src, &Table, &|p| Err(format!("no load: {}", p))) {
            Err(e) => e.msg,
            Ok(_) => panic!("expected a conditional grant without a role to be rejected"),
        };
        assert!(msg.contains("no role"), "{}", msg);
    }

}

#[cfg(test)]
mod full_type_name_tests {
    //! Satz names Terraform types in full. The YAML dialect's implicit `google_`
    //! prefix is not Satz, and — this is the part with teeth — a key that fails
    //! to resolve must be REJECTED, not quietly dropped.
    use super::*;

    struct Google;
    impl TypeResolver for Google {
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            match key {
                "terraform" | "providers" | "variables" | "include" => return None,
                _ => {}
            }
            // Stands in for the schema registry: an explicit allowlist, NOT
            // "anything starting with google_". That shortcut made the stub
            // claim `google_labels` exists, which sent a genuine `labels { … }`
            // attribute block down the resource path — a fake failure the real
            // registry cannot produce.
            const TYPES: &[&str] = &[
                "google_org_policy_policy",
                "google_project_iam_member",
                "google_folder_iam_member",
                "google_cloud_identity_group",
            ];
            if !TYPES.contains(&key) {
                return None;
            }
            let (class, scope) = type_facts(key);
            Some(ResolvedType { tf_type: key.to_string(), class, scope })
        }
    }
    impl crate::algebra::TypeTable for Google {
        fn merge_class(&self, t: &str) -> crate::MergeClass { type_facts(t).0 }
        fn scope(&self, t: &str) -> crate::Scope { type_facts(t).1 }
    }

    fn compile(src: &str) -> Result<FrontEnd, PipelineError> {
        compile_estate("main.satz", src, &Google, &|p| Err(format!("no such use: {}", p)))
    }

    #[test]
    fn a_shorthand_type_name_is_rejected_with_the_full_name() {
        let Err(err) = compile(
            "estate e\n\nparams {\n  a = \"x\"\n}\n\norg_policy_policy {\n  p {\n    name = \"c\"\n  }\n}\n",
        ) else {
            panic!("the shorthand must not compile")
        };
        assert!(err.msg.contains("google_org_policy_policy"), "{}", err.msg);
        assert!(err.msg.contains("in full"), "{}", err.msg);
    }

    #[test]
    fn the_full_name_compiles() {
        let fe = compile(
            "estate e\n\nparams {\n  a = \"x\"\n}\n\ngoogle_org_policy_policy {\n  p {\n    name = \"c\"\n  }\n}\n",
        )
        .unwrap_or_else(|e| panic!("full name must compile: {:?}", e));
        let n: usize = fe.fragments.iter().map(|f| f.entities.len()).sum();
        assert_eq!(n, 1, "expected the policy to be emitted");
    }

    /// The regression that made this dangerous: inside a `project`/`folder` body
    /// a shorthand key was filed away as a nested ATTRIBUTE of the parent and
    /// emitted nothing at all. Silently — transpile exited 0 and the HCL simply
    /// got smaller. It has to reach the same rejection as at the top level.
    #[test]
    fn a_shorthand_nested_in_a_project_is_rejected_not_absorbed() {
        let Err(err) = compile(
            "estate e\n\nparams {\n  a = \"x\"\n}\n\n\
             google_project {\n  p1 {\n    name = \"p1\"\n    \
             project_iam_member {\n      \"user:a@b.c\" = [\n        \"roles/viewer\",\n      ]\n    }\n  }\n}\n",
        ) else {
            panic!("a shorthand inside a project must not be absorbed as an attribute")
        };
        assert!(err.msg.contains("google_project_iam_member"), "{}", err.msg);
    }

    /// …while a genuine nested attribute block, which no `google_` type shadows,
    /// must still be an attribute.
    #[test]
    fn a_real_nested_attribute_block_still_works() {
        compile(
            "estate e\n\nparams {\n  a = \"x\"\n}\n\n\
             google_project {\n  p1 {\n    name = \"p1\"\n    \
             labels {\n      costcenter = \"cc1\"\n    }\n  }\n}\n",
        )
        .unwrap_or_else(|e| panic!("labels is an attribute block, not a resource type: {:?}", e));
    }
}

#[cfg(test)]
mod review_2026_08_29_tests {
    //! The review found a family of "not understood → dropped" paths in the
    //! front-end. Each is now an error; these pin them.
    use super::*;

    struct Table;
    impl TypeResolver for Table {
        fn resolve(&self, key: &str) -> Option<ResolvedType> {
            let entity = |t: &str, scope: Scope| Some(ResolvedType { tf_type: t.into(), class: MergeClass::Entity, scope });
            match key {
                "google_folder" => entity("google_folder", Scope::Node),
                "google_storage_bucket" => entity("google_storage_bucket", Scope::Node),
                "google_cloud_identity_group" => entity("google_cloud_identity_group", Scope::Customer),
                "google_org_policy_policy" => entity("google_org_policy_policy", Scope::Node),
                "google_organization_iam_member" => Some(ResolvedType { tf_type: "google_organization_iam_member".into(), class: MergeClass::Grant, scope: Scope::Org }),
                "google_storage_bucket_iam_member" | "google_service_account_iam_member" | "google_project_iam_member" => {
                    Some(ResolvedType { tf_type: key.into(), class: MergeClass::Grant, scope: Scope::Node })
                }
                _ => None,
            }
        }
    }
    impl TypeTable for Table {
        fn merge_class(&self, t: &str) -> MergeClass {
            if t.ends_with("_iam_member") { MergeClass::Grant } else { MergeClass::Entity }
        }
        fn scope(&self, _t: &str) -> Scope {
            Scope::Node
        }
    }

    fn compile_with(src: &str, files: &[(&str, &str)]) -> Result<FrontEnd, PipelineError> {
        let load = |p: &str| files.iter().find(|(n, _)| *n == p).map(|(_, s)| s.to_string()).ok_or_else(|| format!("no such file {}", p));
        compile_estate("main.satz", src, &Table, &load)
    }
    fn compile(src: &str) -> Result<FrontEnd, PipelineError> {
        compile_with(src, &[])
    }
    trait MustFail {
        fn must_fail(self, why: &str) -> PipelineError;
    }
    impl MustFail for Result<FrontEnd, PipelineError> {
        fn must_fail(self, why: &str) -> PipelineError {
            match self {
                Err(e) => e,
                Ok(_) => panic!("{}", why),
            }
        }
    }
    const HEAD: &str = "estate e\nparams { customer_organization_id = \"1\" }\n";

    #[test]
    fn an_unknown_full_type_name_nested_in_a_folder_is_rejected_not_absorbed() {
        let src = format!("{}google_folder {{ f {{ display_name = \"F\" google_stroage_bucket {{ b {{ name = \"b\" }} }} }} }}\n", HEAD);
        let err = compile(&src).must_fail("a typo'd nested type must not compile");
        assert!(err.msg.contains("google_stroage_bucket"), "{}", err.msg);
    }

    #[test]
    fn use_as_inside_a_folder_keys_the_pack_by_that_type() {
        let src = format!("{}google_folder {{ f {{ display_name = \"F\" use \"g.satz\" as google_cloud_identity_group }} }}\n", HEAD);
        let fe = compile_with(&src, &[("g.satz", "pack g\n\"log-admins\" { display_name = \"LA\" }\n")]).unwrap();
        let folded = fold_fragments(&Table, &fe.fragments);
        let kinds: Vec<&str> = folded.slots.keys().map(|a| a.tf_type.as_str()).collect();
        assert!(kinds.contains(&"google_cloud_identity_group"), "{:?}", kinds);
        assert!(!folded.slots.keys().any(|a| a.tf_type == "google_folder" && a.label == "log-admins"), "the pack must not become folders");
    }

    #[test]
    fn use_as_another_type_inside_a_resource_map_is_an_error() {
        let src = format!("{}google_org_policy_policy {{ use \"g.satz\" as google_cloud_identity_group }}\n", HEAD);
        let err = compile_with(&src, &[("g.satz", "pack g\n")]).must_fail("must not be silently ignored");
        assert!(err.msg.contains("use … as"), "{}", err.msg);
    }

    #[test]
    fn when_on_an_undeclared_param_is_an_error() {
        let src = format!("{}use \"p.satz\" when want_cs\n", HEAD);
        let err = compile_with(&src, &[("p.satz", "pack p\n")]).must_fail("typo must not drop the pack");
        assert!(err.msg.contains("unknown param `want_cs`"), "{}", err.msg);
    }

    #[test]
    fn an_unknown_key_in_a_grant_object_is_an_error() {
        let src = format!("{}google_organization_iam_member {{ \"user:a@b.c\" = [ {{ role = \"roles/viewer\" description = \"why\" }} ] }}\n", HEAD);
        let err = compile(&src).must_fail("the role must not become `description`");
        assert!(err.msg.contains("unknown key `description`"), "{}", err.msg);
    }

    #[test]
    fn the_same_address_twice_in_one_file_with_different_bodies_is_an_error() {
        // inside ONE map block the parser already refuses the repeated label
        let src = format!("{}google_org_policy_policy {{ p {{ name = \"first\" }} p {{ name = \"second\" }} }}\n", HEAD);
        let err = compile(&src).must_fail("second body must not be dropped");
        assert!(err.msg.contains("given twice"), "{}", err.msg);
        // across two blocks of the same map it is the fold's same-address rule
        let src = format!("{}google_org_policy_policy {{ p {{ name = \"first\" }} }}\ngoogle_org_policy_policy {{ p {{ name = \"second\" }} }}\n", HEAD);
        let err = compile(&src).must_fail("second body must not be dropped");
        assert!(err.msg.contains("declared twice"), "{}", err.msg);
        let same = format!("{}google_org_policy_policy {{ p {{ name = \"x\" }} }}\ngoogle_org_policy_policy {{ p {{ name = \"x\" }} }}\n", HEAD);
        compile(&same).expect("an identical repeat is idempotent");
    }


    // --- scope pins: the member-map form for types whose scope is neither the
    // organisation nor the folder/project they are written in. -----------------

    fn grant_labels(fe: &FrontEnd, tf_type: &str) -> Vec<String> {
        let folded = fold_fragments(&Table, &fe.fragments);
        let mut v: Vec<String> = folded
            .slots
            .keys()
            .filter(|a| a.tf_type == tf_type)
            .map(|a| a.label.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_scope_attribute_pins_a_grant_map_and_namespaces_its_address() {
        // the same member and role in two scopes: two grants, not one
        let src = format!(
            "{}google_storage_bucket_iam_member {{ bucket = \"audit\" \"group:a@b.c\" = [\"roles/storage.objectViewer\"] }}\n\
             google_storage_bucket_iam_member {{ bucket = \"logs\" \"group:a@b.c\" = [\"roles/storage.objectViewer\"] }}\n",
            HEAD
        );
        let fe = compile(&src).expect("two pinned maps must compile");
        assert_eq!(
            grant_labels(&fe, "google_storage_bucket_iam_member"),
            vec![
                format!("bucket=audit{}group:a@b.c", GRANT_SCOPE_SEP),
                format!("bucket=logs{}group:a@b.c", GRANT_SCOPE_SEP),
            ]
        );
    }

    #[test]
    fn a_scope_pin_is_read_before_the_members_whatever_the_order() {
        let src = format!(
            "{}google_service_account_iam_member {{ \"group:a@b.c\" = [\"roles/iam.workloadIdentityUser\"] service_account_id = \"projects/p/serviceAccounts/x@y\" }}\n",
            HEAD
        );
        let fe = compile(&src).expect("a pin after the members must still apply");
        assert_eq!(
            grant_labels(&fe, "google_service_account_iam_member"),
            vec![format!("service_account_id=projects/p/serviceAccounts/x@y{}group:a@b.c", GRANT_SCOPE_SEP)]
        );
    }

    #[test]
    fn a_pinned_grant_map_passes_its_scope_to_used_pack_content() {
        let src = format!("{}google_storage_bucket_iam_member {{ bucket = \"audit\" use \"g.satz\" }}\n", HEAD);
        let fe = compile_with(&src, &[("g.satz", "pack g\n\"group:a@b.c\" = [\"roles/storage.objectViewer\"]\n")])
            .expect("pack content must inherit the map's scope");
        assert_eq!(
            grant_labels(&fe, "google_storage_bucket_iam_member"),
            vec![format!("bucket=audit{}group:a@b.c", GRANT_SCOPE_SEP)]
        );
    }

    #[test]
    fn two_scopes_in_one_grant_map_is_an_error_naming_both() {
        let src = format!(
            "{}google_storage_bucket_iam_member {{ bucket = \"audit\" buckett = \"logs\" \"group:a@b.c\" = [\"roles/storage.objectViewer\"] }}\n",
            HEAD
        );
        let err = compile(&src).must_fail("one map cannot carry two scopes");
        assert!(err.msg.contains("pins two scopes"), "{}", err.msg);
        assert!(err.msg.contains("bucket = \"audit\"") && err.msg.contains("buckett"), "{}", err.msg);
    }

    #[test]
    fn a_scope_pin_on_an_organisation_grant_is_an_error() {
        let src = format!("{}google_organization_iam_member {{ org_id = \"9\" \"group:a@b.c\" = [\"roles/browser\"] }}\n", HEAD);
        let err = compile(&src).must_fail("the org scope comes from customer_organization_id");
        assert!(err.msg.contains("customer_organization_id"), "{}", err.msg);
    }

    #[test]
    fn a_scope_pin_inside_the_node_that_already_scopes_it_is_an_error() {
        let src = format!(
            "{}google_folder {{ f {{ display_name = \"F\" google_project_iam_member {{ project = \"p\" \"group:a@b.c\" = [\"roles/viewer\"] }} }} }}\n",
            HEAD
        );
        let err = compile(&src).must_fail("explicit and inherited scope must not compete");
        assert!(err.msg.contains("already scoped by that node"), "{}", err.msg);
    }

    #[test]
    fn the_two_all_principal_members_are_members_not_scopes() {
        let src = format!(
            "{}google_storage_bucket_iam_member {{ bucket = \"public\" allUsers = [\"roles/storage.objectViewer\"] }}\n",
            HEAD
        );
        let fe = compile(&src).expect("allUsers is a member");
        assert_eq!(
            grant_labels(&fe, "google_storage_bucket_iam_member"),
            vec![format!("bucket=public{}allUsers", GRANT_SCOPE_SEP)]
        );
    }

    #[test]
    fn a_scope_attribute_given_a_list_says_so() {
        let src = format!(
            "{}google_storage_bucket_iam_member {{ bucket = [\"audit\"] \"group:a@b.c\" = [\"roles/storage.objectViewer\"] }}\n",
            HEAD
        );
        let err = compile(&src).must_fail("a scope is one string");
        assert!(err.msg.contains("must be a string"), "{}", err.msg);
    }

    #[test]
    fn a_cyclic_use_is_reported_with_its_chain() {
        let src = format!("{}use \"a.satz\"\n", HEAD);
        let err = compile_with(&src, &[("a.satz", "pack a\nuse \"b.satz\"\n"), ("b.satz", "pack b\nuse \"a.satz\"\n")]).must_fail("must not overflow");
        assert!(err.msg.contains("cyclic `use`: main.satz → a.satz → b.satz → a.satz"), "{}", err.msg);
    }

    #[test]
    fn using_a_yaml_pack_names_the_converter() {
        let src = format!("{}use \"old-pack.yaml\"\n", HEAD);
        let err = compile_with(&src, &[("old-pack.yaml", "variables:\n  a: 1\n")]).must_fail("a YAML pack must not be parsed as Satz");
        assert!(err.msg.contains("satz import old-pack.yaml --kind pack"), "{}", err.msg);
    }

    #[test]
    fn a_second_terraform_block_is_an_error() {
        let src = format!("{}terraform {{ backend {{ local {{ path = \"a\" }} }} }}\nterraform {{ backend {{ local {{ path = \"b\" }} }} }}\n", HEAD);
        let err = compile(&src).must_fail("second block must not be dropped");
        assert!(err.msg.contains("declared twice"), "{}", err.msg);
    }
}
