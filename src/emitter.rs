//! Stage B emitter: `satz_core::algebra::Folded` → HCL (docs/stage-b.md).
//! Every block shape comes from `emit_shared` — the same builders the walk
//! transpiler calls — so parity is by construction, not by imitation. What this
//! module owns is only the mapping from folded entities to builder inputs:
//! structural context (folder/project chain) comes from `Entity::node_path`
//! instead of walk position — scope as data, not as interception.
//!
use satz_core::algebra::{Body, Entity, Folded, Slot};
use satz_core::pipeline::{Env, BILLING_ID_TYPE, GRANT_SCOPE_SEP};

/// Config-level facts the emitter needs. Derived from the estate's resolved
/// parameter environment plus (for estates) the loaded schema registry.
pub(crate) struct EmitCtx<'a> {
    pub customer_id: String,
    pub customer_domain: String,
    pub org_id: String,
    /// The conventional billing fallback (`billing-account-infra`) as a YAML
    /// value, exactly like the walk's `variables` lookup.
    pub billing_fallback: Option<serde_yaml::Value>,
    /// The project every provider call is billed to, so the project an API has to
    /// be enabled on whatever the resource's own scope is. Empty when unbound.
    pub infra_project: String,
    pub registry: Option<&'a crate::schema::ResourceRegistry>,
}

impl EmitCtx<'_> {
    pub(crate) fn from_env(env: &Env) -> Self {
        let get = |k: &str| env.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        EmitCtx {
            customer_id: get("customer_id"),
            customer_domain: get("customer_domain"),
            org_id: get("customer_organization_id"),
            billing_fallback: env.get("billing_account_infra").cloned(),
            infra_project: get("infra_project_name"),
            registry: None,
        }
    }
}

fn last_with_prefix<'a>(path: &'a [String], prefix: &str) -> Option<&'a str> {
    path.iter().rev().find_map(|e| e.strip_prefix(prefix))
}

/// An address prefix that is a scope pin (`<attr>=<value>`) rather than a
/// structural path. The attribute is a Terraform identifier, so the split is
/// unambiguous: a folder or project label never looks like one.
fn scope_pin_of(prefix: &str) -> Option<(String, String)> {
    let (attr, value) = prefix.split_once('=')?;
    if attr.is_empty() || value.is_empty() {
        return None;
    }
    let ident = attr.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && attr.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    ident.then(|| (attr.to_string(), value.to_string()))
}

/// The attributes of an `*_iam_member` type that could name its scope —
/// everything but the three every such type carries. Used to make a mistyped
/// scope attribute a compile error that names the alternatives.
fn scope_attr_candidates(schema: &crate::schema::ResourceSchema) -> Vec<String> {
    let mut v: Vec<String> = schema
        .block
        .attributes
        .keys()
        .filter(|k| !matches!(k.as_str(), "role" | "member" | "id" | "etag" | "condition"))
        .cloned()
        .collect();
    v.sort();
    v
}

/// The provider alias the walk would hand a resource at this position:
/// per-project alias inside a project, the root alias everywhere else.
fn alias_for(path: &[String]) -> String {
    match last_with_prefix(path, "project:") {
        Some(p) => format!("google.project_{}", p.replace('-', "_")),
        None => "google.google".to_string(),
    }
}

/// The walk's narrowest-context facts, derived from structural position.
fn res_ctx(path: &[String], ctx: &EmitCtx, folded: &Folded) -> crate::emit_shared::ResCtx {
    let mut rc = crate::emit_shared::ResCtx {
        org_id: Some(ctx.org_id.clone()),
        org_ref: Some(format!("organizations/{}", ctx.org_id)),
        ..Default::default()
    };
    if let Some(f) = last_with_prefix(path, "folder:") {
        rc.folder_ref = Some(format!("google_folder.{}.name", f.replace('-', "_")));
    }
    if let Some(p) = last_with_prefix(path, "project:") {
        rc.project_ref = Some(format!("google_project.{}.project_id", p.replace('-', "_")));
        // the project's real id, from its own folded entity
        let addr = satz_core::Address { tf_type: "google_project".into(), label: p.to_string() };
        if let Some(Slot::Ok(e)) = folded.slots.get(&addr) {
            if let Body::Attrs(serde_yaml::Value::Mapping(m)) = &e.body {
                rc.project_id = m
                    .get(serde_yaml::Value::String("project_id".into()))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
    }
    rc
}

/// main.tf + imports.tf from the folded estate. Types this emitter does not
/// cover yet are an explicit error — the differential report surfaces them as
/// the next parity item, never as silently missing output.
pub(crate) struct EmitOut {
    pub main_tf: String,
    pub imports_tf: String,
    /// The emitted resources as structure — built from the same blocks
    /// `main_tf` renders, so consumers never parse the text back.
    pub manifest: crate::manifest::Manifest,
    /// Per emitted resource that lacks one: the arguments and blocks its schema
    /// requires and the emitted block does not carry. Checked on what is
    /// emitted, after every derived attribute is in.
    pub missing_required: Vec<MissingRequired>,
    /// Per emitted attribute the provider will refuse by its shape.
    pub wrong_shapes: Vec<WrongShape>,
    /// Per resource declared outside the project or folder its type is scoped to, which
    /// sets none itself.
    pub unscoped: Vec<Unscoped>,
}

/// A resource whose type takes a `project` (or a `folder`), declared where no project
/// (or folder) encloses it, and naming none itself. A project-scoped one goes to the
/// project the provider block names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unscoped {
    pub address: String,
    /// `project` or `folder`
    pub scope: &'static str,
    /// where the declaring block starts
    pub origin: Option<(String, u32)>,
}

/// A resource the provider will refuse: required arguments or blocks absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingRequired {
    pub address: String,
    pub missing: Vec<String>,
    /// where the declaring block starts, when it was declared rather than derived
    pub origin: Option<(String, u32)>,
}

/// An emitted attribute whose value the provider refuses by its SHAPE: a string where
/// the schema types a set, a list where it types a string. Found on a customer estate —
/// a list param answered with one address became `notification_emails = "…"`, and
/// nothing said so until `tofu apply`, naming a line of generated HCL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrongShape {
    pub address: String,
    /// the attribute, dotted through the nested blocks it sits in
    pub attribute: String,
    /// the schema's type, as `set(string)`, `number`
    pub expected: String,
    /// the emitted value's shape: `string`, `number`, `bool`, `list`, `map`
    pub got: &'static str,
    /// where the declaring block starts
    pub origin: Option<(String, u32)>,
}

/// The shape of an emitted literal; `None` for anything Terraform only knows at plan
/// time — a reference, an interpolation, a function call.
fn literal_shape(expr: &hcl::Expression) -> Option<(&'static str, Option<&str>)> {
    match expr {
        hcl::Expression::String(s) if !s.contains("${") => Some(("string", Some(s.as_str()))),
        hcl::Expression::Number(_) => Some(("number", None)),
        hcl::Expression::Bool(_) => Some(("bool", None)),
        hcl::Expression::Array(_) => Some(("list", None)),
        hcl::Expression::Object(_) => Some(("map", None)),
        _ => None,
    }
}

/// The schema's type expression as Terraform writes it: `string`, `set(string)`.
fn type_text(t: &serde_json::Value) -> String {
    match t {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => match parts.as_slice() {
            [kind, inner] => format!("{}({})", kind.as_str().unwrap_or("?"), type_text(inner)),
            _ => t.to_string(),
        },
        other => other.to_string(),
    }
}

/// Whether Terraform converts a literal of this shape to the schema's type. Only what
/// it certainly refuses is a mismatch: a scalar for a collection, a collection for a
/// scalar, a string that spells no number or bool where one is wanted.
fn shape_fits(t: &serde_json::Value, shape: &str, text: Option<&str>) -> bool {
    match t {
        serde_json::Value::String(kind) => match kind.as_str() {
            "string" => matches!(shape, "string" | "number" | "bool"),
            "number" => shape == "number" || text.is_some_and(|s| s.trim().parse::<f64>().is_ok()),
            "bool" => shape == "bool" || matches!(text, Some("true" | "false")),
            _ => true, // dynamic
        },
        serde_json::Value::Array(parts) => match parts.first().and_then(|k| k.as_str()) {
            Some("list" | "set" | "tuple") => shape == "list",
            Some("map" | "object") => shape == "map",
            _ => true,
        },
        _ => true,
    }
}

/// Every attribute of `block`, nested blocks included, whose literal the schema's type
/// refuses: `(attribute path, expected type, emitted shape)`. Empty for a type the
/// registry does not know.
pub(crate) fn wrong_shapes(block: &hcl::Block, registry: &crate::schema::ResourceRegistry) -> Vec<(String, String, &'static str)> {
    fn walk(body: &hcl::Body, schema: &crate::schema::BlockSchema, prefix: &str, out: &mut Vec<(String, String, &'static str)>) {
        for attr in body.attributes() {
            let (Some(a), Some((shape, text))) = (schema.attributes.get(attr.key()), literal_shape(attr.expr())) else { continue };
            let Some(t) = &a.type_ else { continue };
            if !shape_fits(t, shape, text) {
                out.push((format!("{}{}", prefix, attr.key()), type_text(t), shape));
            }
        }
        for b in body.blocks() {
            if let Some(bt) = schema.block_types.get(b.identifier()) {
                walk(b.body(), &bt.block, &format!("{}{}.", prefix, b.identifier()), out);
            }
        }
    }
    let labels: Vec<&str> = block.labels().iter().map(|l| l.as_str()).collect();
    let [tf_type, _] = labels.as_slice() else { return Vec::new() };
    let Some((_, schema)) = registry.resources.get(*tf_type) else { return Vec::new() };
    let mut out = Vec::new();
    walk(block.body(), &schema.block, "", &mut out);
    out
}

/// The required top-level arguments and blocks (`min_items` ≥ 1) of `block`'s
/// resource type that `block` does not carry. Empty when the type is unknown to
/// the registry: no schema, no verdict.
pub(crate) fn missing_required(block: &hcl::Block, registry: &crate::schema::ResourceRegistry) -> Vec<String> {
    let labels: Vec<&str> = block.labels().iter().map(|l| l.as_str()).collect();
    let [tf_type, _] = labels.as_slice() else { return Vec::new() };
    let Some((_, schema)) = registry.resources.get(*tf_type) else { return Vec::new() };
    let have_attrs: std::collections::BTreeSet<&str> = block.body().attributes().map(|a| a.key()).collect();
    let have_blocks: std::collections::BTreeSet<&str> = block.body().blocks().map(|b| b.identifier()).collect();
    let mut missing: Vec<String> = schema
        .block
        .attributes
        .iter()
        .filter(|(k, a)| a.required && !have_attrs.contains(k.as_str()))
        .map(|(k, _)| k.clone())
        .chain(
            schema
                .block
                .block_types
                .iter()
                .filter(|(k, b)| b.min_items.unwrap_or(0) >= 1 && !have_blocks.contains(k.as_str()))
                .map(|(k, _)| format!("{} {{ … }}", k)),
        )
        .collect();
    missing.sort();
    missing
}

/// A conditional grant edge carries its condition as canonical YAML text. Parse
/// it back so the emitted label hashes EXACTLY what the walk hashed (the label
/// is the Terraform address — it must not move for existing state), and so the
/// `condition { … }` block renders identically. A condition that does not parse
/// is an error, never `None`: dropping it would emit the grant unconditional and
/// move its address in the same stroke.
fn edge_condition(edge: &satz_core::algebra::GrantEdge) -> Result<Option<serde_yaml::Value>, String> {
    if edge.condition.is_empty() {
        return Ok(None);
    }
    serde_yaml::from_str(&edge.condition).map(Some).map_err(|e| {
        format!(
            "grant {} → {}: its condition is not readable ({}); refusing to emit the binding without it",
            edge.member, edge.role, e
        )
    })
}

/// `<type>.<label>` of a resource block.
fn block_address(b: &hcl::Block) -> Option<String> {
    if b.identifier() != "resource" {
        return None;
    }
    match b.labels() {
        [t, l] => Some(format!("{}.{}", t.as_str(), l.as_str())),
        _ => None,
    }
}

/// Grants and group memberships that name a service account the estate declares
/// carry its email as a string, not a reference, so tofu would run them beside the
/// account: on create the API refuses a member that does not exist yet, and on
/// destroy the account can go first and leave `deleted:serviceAccount:…` bindings
/// behind. Each one gets a `depends_on` on the account.
fn order_after_service_accounts(blocks: &mut [hcl::Block]) {
    let manifest = crate::manifest::Manifest::from_blocks(blocks.iter());
    let accounts: std::collections::BTreeMap<String, String> = manifest
        .of_type("google_service_account")
        .filter_map(|r| {
            let email = format!("{}@{}.iam.gserviceaccount.com", r.attrs.get("account_id")?, manifest.project_of(r)?);
            Some((email, r.address()))
        })
        .collect();
    if accounts.is_empty() {
        return;
    }
    for b in blocks.iter_mut() {
        let Some(address) = block_address(b) else { continue };
        let Some(r) = manifest.resources.get(&address) else { continue };
        let named = r
            .attrs
            .get("member")
            .and_then(|m| m.strip_prefix("serviceAccount:"))
            .or_else(|| r.nested.get("preferred_member_key.id").map(String::as_str));
        let Some(account) = named.and_then(|email| accounts.get(email)) else { continue };
        add_depends_on(b, account);
    }
}

/// A grant on a group the estate declares names it by its email string
/// (`group:<key>@<domain>`), so tofu would create the grant beside the group:
/// the IAM API refuses a member that does not exist yet, and on a fresh
/// organisation the first refusal stops the groups still queued from ever
/// being created. Each such grant — and a membership whose member key names
/// such a group — gets a `depends_on` on the group, the way a grant on a
/// declared service account does; on destroy the grant then goes before the
/// group, leaving no `deleted:group:…` binding behind.
fn order_after_groups(blocks: &mut [hcl::Block]) {
    let manifest = crate::manifest::Manifest::from_blocks(blocks.iter());
    let groups: std::collections::BTreeMap<String, String> = manifest
        .of_type("google_cloud_identity_group")
        .filter_map(|r| Some((r.nested.get("group_key.id")?.clone(), r.address())))
        .collect();
    if groups.is_empty() {
        return;
    }
    for b in blocks.iter_mut() {
        let Some(address) = block_address(b) else { continue };
        let Some(r) = manifest.resources.get(&address) else { continue };
        let named = r
            .attrs
            .get("member")
            .and_then(|m| m.strip_prefix("group:"))
            .or_else(|| r.nested.get("preferred_member_key.id").map(String::as_str));
        let Some(group) = named.and_then(|email| groups.get(email)) else { continue };
        add_depends_on(b, group);
    }
}

/// A policy on a custom constraint names the constraint by its string
/// (`…/policies/custom.x`), so tofu would create the policy beside the
/// constraint, and the API refuses a policy on a constraint that does not exist
/// yet. A policy on a constraint the estate declares gets a `depends_on` on it.
fn order_after_custom_constraints(blocks: &mut [hcl::Block]) {
    let manifest = crate::manifest::Manifest::from_blocks(blocks.iter());
    let constraints: std::collections::BTreeMap<String, String> = manifest
        .of_type("google_org_policy_custom_constraint")
        .filter_map(|r| Some((r.attrs.get("name")?.clone(), r.address())))
        .collect();
    if constraints.is_empty() {
        return;
    }
    for b in blocks.iter_mut() {
        let Some(address) = block_address(b) else { continue };
        let Some(r) = manifest.resources.get(&address).filter(|r| r.tf_type == "google_org_policy_policy") else { continue };
        let Some(constraint) = r.attrs.get("name").and_then(|n| n.rsplit_once("/policies/")).map(|(_, c)| c) else { continue };
        if let Some(declared) = constraints.get(constraint) {
            add_depends_on(b, declared);
        }
    }
}

/// The Org Policy API serialises the writes to one parent's policies, and refuses a
/// second one that arrives while the first is in flight: `409 Creating policy failed.
/// Please retry the request … reason: CONCURRENT_POLICY_CHANGES`. tofu writes ten
/// resources at a time, and the CIS baseline alone puts thirty policies on one
/// organisation with nothing ordering them, so a fresh apply collides with itself by
/// construction — and a retry clears it, which is why it read as a fluke.
///
/// The collision is a property of the PARENT, not of a pack: the baseline's policies,
/// its extensions' and the estate's own all land on the same organisation, and only
/// the emitter sees all of them. So the policies on one parent are chained, each
/// waiting for the one before it in address order. `depends_on` changes no plan; it
/// serialises the policy writes on that parent and nothing else — policies on
/// different parents, and every other resource, still apply in parallel.
fn order_org_policies_per_parent(blocks: &mut [hcl::Block]) {
    let manifest = crate::manifest::Manifest::from_blocks(blocks.iter());
    let mut by_parent: std::collections::BTreeMap<&str, Vec<String>> = std::collections::BTreeMap::new();
    for r in manifest.of_type("google_org_policy_policy") {
        let Some(parent) = r.attrs.get("parent") else { continue };
        by_parent.entry(parent.as_str()).or_default().push(r.address());
    }
    let mut previous: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for addresses in by_parent.into_values() {
        // `resources` is a BTreeMap, so each parent's addresses arrive sorted
        for pair in addresses.windows(2) {
            // an edge into what the earlier policy already reaches would close a loop
            if !manifest.closure_of(&pair[0]).contains(&pair[1]) {
                previous.insert(pair[1].clone(), pair[0].clone());
            }
        }
    }
    if previous.is_empty() {
        return;
    }
    for b in blocks.iter_mut() {
        let Some(address) = block_address(b) else { continue };
        if let Some(before) = previous.get(&address) {
            add_depends_on(b, before);
        }
    }
}

/// A resource whose API is not on yet fails the apply, and `google_project_service`
/// carries no ordering of its own: inside one apply Terraform can create the
/// resource before the service that enables it. Bootstrap escapes that race by
/// enabling the day-0 APIs imperatively before `tofu` runs, but a pack adopted on
/// day 40 has no bootstrap pass — its service block and the resources that need it
/// land in the same apply, unordered, and it passes or fails on scheduling luck.
///
/// Every resource whose type the prerequisite table knows gets a `depends_on` on
/// the services that enable its APIs, bounded to the two projects that can matter:
/// the project the resource lives in — which is also the project its calls are
/// billed to when a per-project alias serves it — and the infra project the
/// default provider bills to (`user_project_override` + `billing_project`). A
/// service on a third project is nothing to it.
///
/// No edge is added into a service block's own dependency closure, and a service
/// is never ordered after a service. That is what keeps the graph acyclic: the
/// infra project references nothing, but the services on it reference IT, and the
/// folder that contains it is reached through the project — so both would
/// otherwise wait for a service that waits for them.
fn order_after_project_services(blocks: &mut [hcl::Block], infra_project: &str) {
    let manifest = crate::manifest::Manifest::from_blocks(blocks.iter());
    struct Service {
        address: String,
        project: Option<String>,
        closure: std::collections::BTreeSet<String>,
    }
    let mut by_api: std::collections::BTreeMap<&str, Vec<Service>> = std::collections::BTreeMap::new();
    for r in manifest.of_type("google_project_service") {
        let Some(service) = r.attrs.get("service") else { continue };
        let address = r.address();
        by_api.entry(service.as_str()).or_default().push(Service {
            project: manifest.project_of(r),
            closure: manifest.closure_of(&address),
            address,
        });
    }
    if by_api.is_empty() {
        return;
    }
    for b in blocks.iter_mut() {
        let Some(address) = block_address(b) else { continue };
        let Some(r) = manifest.resources.get(&address) else { continue };
        // one API is never ordered behind another: `serviceusage` would wait for
        // itself, and enabling an API is bootstrap's imperative business
        if r.tf_type == "google_project_service" {
            continue;
        }
        let own_project = manifest.project_of(r);
        for api in crate::prerequisites::apis_for(&r.tf_type).unwrap_or(&[]) {
            for s in by_api.get(api).into_iter().flatten() {
                let Some(project) = &s.project else { continue };
                let relevant = project == infra_project || own_project.as_ref() == Some(project);
                if relevant && !s.closure.contains(&address) {
                    add_depends_on(b, &s.address);
                }
            }
        }
    }
}

/// Add `address` to the block's `depends_on`, creating the attribute when absent.
fn add_depends_on(b: &mut hcl::Block, address: &str) {
    let dep = crate::emit_shared::traversal_expr(address);
    match b.body.0.iter_mut().find_map(|st| match st {
        hcl::Structure::Attribute(a) if a.key() == "depends_on" => Some(a),
        _ => None,
    }) {
        Some(existing) => {
            if let hcl::Expression::Array(items) = &mut existing.expr {
                if !items.contains(&dep) {
                    items.push(dep);
                }
            }
        }
        None => b.body.0.push(hcl::Structure::Attribute(hcl::Attribute::new("depends_on", hcl::Expression::Array(vec![dep])))),
    }
}

/// A Terraform `import { to id }` block — the carried result of adoption.
fn import_block(to: &str, id: &str) -> hcl::Block {
    hcl::Block::builder("import")
        .add_attribute(("to", crate::emit_shared::parse_expr(to)))
        .add_attribute(("id", id.to_string()))
        .build()
}

/// Grant edges by binding identity (member, role, condition), each with the one
/// `"import-id"` declared for it. `import_id` is not part of `GrantEdge`'s
/// identity for a reason: the same binding declared twice — once by a pack,
/// once by the estate that adopts it — is one resource. Two DIFFERENT ids for
/// one binding is a contradiction and refuses.
pub(crate) fn reconciled_edges(
    edges: &std::collections::BTreeSet<satz_core::algebra::GrantEdge>,
) -> Result<Vec<satz_core::algebra::GrantEdge>, String> {
    let mut by_identity: std::collections::BTreeMap<(String, String, String), satz_core::algebra::GrantEdge> =
        std::collections::BTreeMap::new();
    for e in edges {
        let key = (e.member.clone(), e.role.clone(), e.condition.clone());
        match by_identity.get_mut(&key) {
            None => {
                by_identity.insert(key, e.clone());
            }
            Some(existing) => {
                if existing.import_id.is_empty() {
                    existing.import_id = e.import_id.clone();
                } else if !e.import_id.is_empty() && existing.import_id != e.import_id {
                    return Err(format!(
                        "binding {} {} declares two different import-ids: {} and {}",
                        e.member, e.role, existing.import_id, e.import_id
                    ));
                }
            }
        }
    }
    Ok(by_identity.into_values().collect())
}

/// An entity without its `private` key, when it has one, and whether it is private.
/// `private` is `true` or `false`; anything else is refused.
fn without_private(entity: &Entity) -> Result<(Option<Entity>, bool), String> {
    let Body::Attrs(serde_yaml::Value::Mapping(attrs)) = &entity.body else { return Ok((None, false)) };
    let key = serde_yaml::Value::String("private".into());
    let Some(v) = attrs.get(&key) else { return Ok((None, false)) };
    let private = v.as_bool().ok_or_else(|| format!("`private = {}` — it is `true` or `false`", serde_yaml::to_string(v).unwrap_or_default().trim()))?;
    let mut e = entity.clone();
    if let Body::Attrs(serde_yaml::Value::Mapping(m)) = &mut e.body {
        m.remove(&key);
    }
    Ok((Some(e), private))
}

pub(crate) fn emit(folded: &Folded, ctx: &EmitCtx) -> Result<EmitOut, String> {
    let mut blocks: Vec<hcl::Block> = Vec::new();
    let mut imports: Vec<hcl::Block> = Vec::new();
    let attr_import = |attrs: &serde_yaml::Mapping| -> Option<String> {
        attrs
            .get(serde_yaml::Value::String("import-id".into()))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    // (address, file, line): where each directly-declared block came from, so
    // `adopt --write` can put a resolved id back into the source.
    let mut origins: Vec<(String, String, u32)> = Vec::new();
    // (address, `project` | `folder`): a resource outside the scope its type takes
    let mut unscoped: Vec<(String, &'static str)> = Vec::new();
    // the addresses of the resources marked `private = true`
    let mut private: Vec<String> = Vec::new();
    for (addr, slot) in &folded.slots {
        let entity = match slot {
            Slot::Ok(e) => e,
            Slot::Bottom(c) => {
                return Err(format!(
                    "conflict at {}.{} ({} candidates)",
                    c.addr.tf_type,
                    c.addr.label,
                    c.candidates.len()
                ))
            }
        };
        // `private` is satz's, and never an attribute: taken off the body before any arm
        // reads it
        let (stripped, is_private) = without_private(entity).map_err(|e| format!("{}.{}: {}", addr.tf_type, addr.label, e))?;
        let entity = stripped.as_ref().unwrap_or(entity);
        let path = &entity.node_path;
        let alias = alias_for(path);
        let first_block = blocks.len();
        match (addr.tf_type.as_str(), &entity.body) {
            ("google_folder", Body::Attrs(serde_yaml::Value::Mapping(attrs))) => {
                let display_name = attrs
                    .get(serde_yaml::Value::String("display_name".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&addr.label);
                let explicit_parent = attrs.get(serde_yaml::Value::String("parent".into())).and_then(|v| v.as_str());
                let parent = match (last_with_prefix(path, "folder:"), explicit_parent) {
                    (Some(f), None) => crate::emit_shared::traversal_expr(&format!(
                        "google_folder.{}.name",
                        f.replace('-', "_")
                    )),
                    (Some(f), Some(p)) => {
                        return Err(format!(
                            "folder `{}`: `parent = \"{}\"` is declared, but the folder is nested under `{}` — the parent is the nesting; remove the attribute",
                            addr.label, p, f
                        ))
                    }
                    // a top-level folder whose parent is another folder outside
                    // this estate (a partial import) says so explicitly
                    (None, Some(p)) => hcl::Expression::from(p.to_string()),
                    (None, None) => hcl::Expression::from(format!("organizations/{}", ctx.org_id)),
                };
                if let Some(id) = attr_import(attrs) {
                    imports.push(import_block(&format!("google_folder.{}", addr.label.replace('-', "_")), &id));
                }
                // Attributes the block builder does not place itself. Nested
                // resources are separate entities by now; a `folder`/`project`
                // or `google_*` key here would be one the fold left behind, not
                // an attribute.
                let mut extra: Vec<(String, serde_yaml::Value)> = attrs
                    .iter()
                    .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), v.clone())))
                    .filter(|(k, _)| {
                        !matches!(k.as_str(), "display_name" | "parent" | "labels" | "lifecycle" | "import-id" | "folder" | "project")
                            && !k.starts_with("google_")
                    })
                    .collect();
                extra.sort_by(|a, b| a.0.cmp(&b.0));
                blocks.push(crate::emit_shared::folder_block(
                    &addr.label.replace('-', "_"),
                    display_name,
                    parent,
                    Some(&alias),
                    attrs.get(serde_yaml::Value::String("labels".into())),
                    attrs
                        .get(serde_yaml::Value::String("lifecycle".into()))
                        .and_then(|v| crate::emit_shared::lifecycle_block(v, &|_| None)),
                    &extra,
                ));
            }
            ("google_cloud_identity_group", Body::Attrs(serde_yaml::Value::Mapping(attrs))) => {
                // Hoisted to customer scope: the walk emits these with the root alias.
                if let Some(id) = attr_import(attrs) {
                    imports.push(import_block(
                        &crate::emit_shared::group_resource_address(&addr.label),
                        &id,
                    ));
                }
                blocks.push(crate::emit_shared::group_block(
                    &addr.label,
                    attrs,
                    &ctx.customer_id,
                    &ctx.customer_domain,
                    Some("google.google"),
                    crate::emit_shared::group_lifecycle(attrs),
                ));
                blocks.extend(crate::emit_shared::membership_blocks(
                    &addr.label,
                    attrs,
                    Some("google.google"),
                ));
                for (member_raw, id) in crate::emit_shared::group_member_import_ids(attrs) {
                    imports.push(import_block(
                        &crate::emit_shared::membership_resource_address(&addr.label, &member_raw),
                        &id,
                    ));
                }
            }
            ("google_organization_iam_member", Body::Grant(edges)) => {
                for e in reconciled_edges(edges)? {
                    let cond = edge_condition(&e)?;
                    let label = crate::emit_shared::iam_member_label(&e.member, &e.role, cond.as_ref(), "");
                    let cond_block = cond.as_ref().and_then(|cv| crate::emit_shared::render_block("condition", cv, None, &|_| None));
                    blocks.push(crate::emit_shared::iam_member_block(
                        "google_organization_iam_member",
                        &label,
                        &e.role,
                        crate::emit_shared::string_to_hcl_expr(&e.member),
                        "org_id",
                        crate::emit_shared::string_to_hcl_expr(&ctx.org_id),
                        cond_block,
                        Some("google.google"),
                    ));
                    if !e.import_id.is_empty() {
                        imports.push(import_block(&format!("google_organization_iam_member.{}", label), &e.import_id));
                    }
                }
            }
            (BILLING_ID_TYPE, _) => {} // consumed below, never emitted
            ("google_billing_account_iam_member", Body::Grant(edges)) => {
                // Explicit pin from a fragment wins; else the conventional variable.
                let pinned = folded
                    .slots
                    .get(&satz_core::Address { tf_type: BILLING_ID_TYPE.into(), label: "billing_account_id".into() })
                    .and_then(|s| match s {
                        Slot::Ok(e) => match &e.body {
                            Body::Attrs(v) => v.as_str().map(|x| x.to_string()),
                            _ => None,
                        },
                        _ => None,
                    });
                let fallback = ctx.billing_fallback.as_ref().and_then(|v| v.as_str());
                let Some(billing_id) = pinned.as_deref().or(fallback) else {
                    return Err(
                        "google_billing_account_iam_member: no billing account to bind on — pin `billing_account_id` in the estate or bind the `billing_account_infra` param"
                            .to_string(),
                    );
                };
                for e in reconciled_edges(edges)? {
                    let cond = edge_condition(&e)?;
                    let label = crate::emit_shared::iam_member_label(&e.member, &e.role, cond.as_ref(), "");
                    let cond_block = cond.as_ref().and_then(|cv| crate::emit_shared::render_block("condition", cv, None, &|_| None));
                    blocks.push(crate::emit_shared::iam_member_block(
                        "google_billing_account_iam_member",
                        &label,
                        &e.role,
                        crate::emit_shared::string_to_hcl_expr(&e.member),
                        "billing_account_id",
                        crate::emit_shared::string_to_hcl_expr(billing_id),
                        cond_block,
                        Some("google.google"),
                    ));
                    if !e.import_id.is_empty() {
                        imports.push(import_block(&format!("google_billing_account_iam_member.{}", label), &e.import_id));
                    }
                }
            }
            // Node-scoped grant maps (project_iam_member inside a project, …):
            // the label carries the structural path before the separator.
            (t, Body::Grant(edges)) => {
                // The address prefix is either an explicit scope pin
                // (`service_account_id=projects/p/serviceAccounts/x@y`) or the
                // structural path the grant was written in.
                let prefix = addr.label.split_once(GRANT_SCOPE_SEP).map(|(p, _)| p);
                let pin = prefix.and_then(scope_pin_of);
                let grant_path: Vec<String> = match (&pin, prefix) {
                    (Some(_), _) => path.clone(),
                    (None, Some(p)) => p.split('/').map(|s| s.to_string()).collect(),
                    (None, None) => path.clone(),
                };
                let rc = res_ctx(&grant_path, ctx, folded);
                let galias = alias_for(&grant_path);
                let (id_attr, parent) = if let Some((attr, value)) = &pin {
                    if let Some(schema) = ctx.registry.and_then(|r| r.find_resource(t)).map(|(_, s)| s) {
                        if !schema.block.attributes.contains_key(attr.as_str()) {
                            return Err(format!(
                                "{}: `{}` is not an attribute of this type — its scope attribute is one of: {}",
                                t,
                                attr,
                                scope_attr_candidates(schema).join(", ")
                            ));
                        }
                    }
                    (attr.as_str(), Some(value.clone()))
                } else if t.contains("project") {
                    ("project", rc.project_ref.clone().or(rc.project_id.clone()))
                } else if t.contains("folder") {
                    ("folder", rc.folder_ref.clone())
                } else {
                    // a bucket / service-account / … scoped grant has no scope
                    // to inherit from the node path; the map form used to emit
                    // an `id = ""` block that could never plan
                    return Err(format!(
                        "{}: the member map form (`\"member\" = [roles…]`) needs the scope written in the map for this type; \
                         add its scope attribute beside the members (`bucket = …`, `service_account_id = …`), or write `{}` as a labelled resource",
                        t, t
                    ));
                };
                let scope_key = pin.as_ref().map(|(_, v)| v.as_str()).unwrap_or("");
                let parent_expr = crate::emit_shared::parse_expr(parent.as_deref().unwrap_or(""));
                for e in reconciled_edges(edges)? {
                    let cond = edge_condition(&e)?;
                    let label = crate::emit_shared::iam_member_label(&e.member, &e.role, cond.as_ref(), scope_key);
                    let cond_block = cond.as_ref().and_then(|cv| crate::emit_shared::render_block("condition", cv, None, &|_| None));
                    blocks.push(crate::emit_shared::iam_member_block(
                        t,
                        &label,
                        &e.role,
                        crate::emit_shared::string_to_hcl_expr(&e.member),
                        id_attr,
                        parent_expr.clone(),
                        cond_block,
                        Some(&galias),
                    ));
                    if !e.import_id.is_empty() {
                        imports.push(import_block(&format!("{}.{}", t, label), &e.import_id));
                    }
                }
            }
            ("google_project", Body::Attrs(serde_yaml::Value::Mapping(attrs))) => {
                if let Some(id) = attr_import(attrs) {
                    imports.push(import_block(&format!("google_project.{}", addr.label.replace('-', "_")), &id));
                }
                emit_project(&mut blocks, &mut imports, addr, attrs, path, ctx)?;
            }
            (t, Body::Attrs(serde_yaml::Value::Mapping(attrs))) => {
                let schema = ctx.registry.and_then(|r| r.find_resource(t)).map(|(_, s)| s);
                let rc = res_ctx(path, ctx, folded);
                let crate::emit_shared::ResourceBlock { block, import_id, label, needs_scope } = crate::emit_shared::single_resource_block(
                    t,
                    &addr.label,
                    attrs,
                    schema,
                    &rc,
                    Some(&alias),
                    &|_| None,
                )
                .map_err(|e| e.to_string())?;
                if let Some(id) = import_id {
                    imports.push(import_block(&format!("{}.{}", t, label), &id));
                }
                if let Some(scope) = needs_scope {
                    unscoped.push((format!("{}.{}", t, label), scope));
                }
                blocks.push(block);
            }
            (t, _) => return Err(format!("emitter has no rule for {} yet (label {})", t, addr.label)),
        }
        // The entity's own block is the first one its arm pushed; blocks after
        // it (memberships, services, exploded grants) are derived and have no
        // line of their own, so they point at the line they derive from — the
        // group, the project, the grant map's member line. `adopt --execute`
        // rewrites the list entry it finds there into the object form.
        if is_private {
            if let Some(a) = blocks.get(first_block).and_then(block_address) {
                private.push(a);
            }
        }
        if let Some(span) = entity.provenance.first() {
            for b in &blocks[first_block..] {
                if let Some(a) = block_address(b) {
                    origins.push((a, span.file.clone(), span.line));
                }
            }
        }
    }

    // Two blocks with one Terraform address is invalid HCL, and the IAM member
    // label is a hash of member+role+condition — distinct scopes that grant the
    // same pair would otherwise collide silently.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for b in &blocks {
        if let Some(a) = block_address(b) {
            if !seen.insert(a.clone()) {
                return Err(format!(
                    "two resources emit the same address `{}` — one would silently overwrite the other in main.tf",
                    a
                ));
            }
        }
    }

    order_after_service_accounts(&mut blocks);
    order_after_groups(&mut blocks);
    order_after_custom_constraints(&mut blocks);
    order_after_project_services(&mut blocks, &ctx.infra_project);
    order_org_policies_per_parent(&mut blocks);

    let mut manifest = crate::manifest::Manifest::from_blocks(&blocks);
    manifest.attach_imports(&imports);
    for (a, f, l) in &origins {
        manifest.set_origin(a, f, *l);
    }
    manifest.private.extend(private);
    let missing_required: Vec<MissingRequired> = match ctx.registry {
        Some(registry) => blocks
            .iter()
            .filter(|b| b.identifier() == "resource")
            .filter_map(|b| {
                let missing = missing_required(b, registry);
                let address = block_address(b)?;
                (!missing.is_empty()).then(|| MissingRequired {
                    origin: manifest.resources.get(&address).and_then(|r| r.origin.clone()),
                    address,
                    missing,
                })
            })
            .collect(),
        None => Vec::new(),
    };
    let wrong_shapes: Vec<WrongShape> = match ctx.registry {
        Some(registry) => blocks
            .iter()
            .filter(|b| b.identifier() == "resource")
            .flat_map(|b| {
                let address = block_address(b).unwrap_or_default();
                let origin = manifest.resources.get(&address).and_then(|r| r.origin.clone());
                wrong_shapes(b, registry).into_iter().map(move |(attribute, expected, got)| WrongShape {
                    address: address.clone(),
                    attribute,
                    expected,
                    got,
                    origin: origin.clone(),
                })
            })
            .collect(),
        None => Vec::new(),
    };
    let unscoped: Vec<Unscoped> = unscoped
        .into_iter()
        .map(|(address, scope)| Unscoped {
            origin: manifest.resources.get(&address).and_then(|r| r.origin.clone()),
            address,
            scope,
        })
        .collect();
    let main_tf = render_main_tf(blocks, &manifest)?;
    // dedup rendered import blocks, like the walk
    let mut import_body = hcl::Body::builder();
    let mut seen = std::collections::HashSet::new();
    for b in imports {
        let rendered = hcl::to_string(&hcl::Body::builder().add_block(b.clone()).build()).unwrap_or_default();
        if seen.insert(rendered) {
            import_body = import_body.add_block(b);
        }
    }
    let imports_tf = hcl::to_string(&import_body.build()).map_err(|e| e.to_string())?;
    Ok(EmitOut { main_tf, imports_tf, manifest, missing_required, wrong_shapes, unscoped })
}

/// `main.tf` as text: the blocks as `hcl` renders a body of them, one blank line
/// apart, with a comment above each org policy declared reset.
///
/// The HCL is handed over, and whoever applies it without satz meets what `run_tf`
/// answers for `satz plan` and `satz apply` (ADR 0011): while the state holds the
/// policy's rules, the API refuses the in-place update, and only a replace switches it
/// to reset. The comment names that replace. It is decided from the manifest, the
/// same `reset` `reset_replacements` reads, and being a comment it moves no address,
/// no attribute and no plan.
fn render_main_tf(blocks: Vec<hcl::Block>, manifest: &crate::manifest::Manifest) -> Result<String, String> {
    let mut out = String::new();
    for b in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        let reset = block_address(&b)
            .filter(|a| manifest.resources.get(a).is_some_and(|r| r.tf_type == "google_org_policy_policy" && r.reset));
        if let Some(address) = reset {
            out.push_str(&reset_replace_comment(&address));
        }
        out.push_str(&hcl::to_string(&hcl::Body::builder().add_block(b).build()).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// The two lines above an org policy declared reset: why a bare apply can be refused
/// on it, and the `-replace` that applies it.
fn reset_replace_comment(address: &str) -> String {
    format!(
        "# Declared reset: while the state holds its rules, the API refuses the in-place update (Cannot set PolicyRules if reset is true).\n\
         # satz plan and satz apply add the replace; a bare apply needs: tofu apply -replace={}\n",
        address
    )
}

/// google_project + its google_project_service children — mirrors the walk's
/// transpile_google_project (context comes from node_path, not walk position).
fn emit_project(
    blocks: &mut Vec<hcl::Block>,
    imports: &mut Vec<hcl::Block>,
    addr: &satz_core::Address,
    attrs: &serde_yaml::Mapping,
    path: &[String],
    ctx: &EmitCtx,
) -> Result<(), String> {
    let get = |k: &str| attrs.get(serde_yaml::Value::String(k.into()));
    let project_id = get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("google_project.{} has no project_id", addr.label))?;
    let resource_name = addr.label.replace('-', "_");
    let mut b = hcl::Block::builder("resource")
        .add_label("google_project")
        .add_label(resource_name.as_str())
        .add_attribute(hcl::Attribute::new("project_id", project_id.to_owned()))
        .add_attribute(hcl::Attribute::new(
            "name",
            get("name").and_then(|v| v.as_str()).unwrap_or(project_id).to_owned(),
        ));
    // The project itself carries the enclosing context's alias (the per-project
    // alias applies to resources INSIDE the project, not the project block).
    let outer_alias = alias_for(path);
    if let Ok(expr) = outer_alias.parse::<hcl::Expression>() {
        b = b.add_attribute(("provider", expr));
    }
    if let Some(ba) = get("billing_account").and_then(|v| v.as_str()) {
        b = b.add_attribute(hcl::Attribute::new("billing_account", ba.to_owned()));
    } else if let Some(ba) = ctx.billing_fallback.as_ref() {
        if let Some(val) = crate::emit_shared::render_value(ba, &|_| None) {
            b = b.add_attribute(hcl::Attribute::new("billing_account", val));
        }
    }
    // The project's parent, stated or inherited. A `folder_id`, `org_id` or `org`
    // whose value is EMPTY is not stated: the enclosing node decides, exactly as
    // if the key were absent. That is what lets a pack carry its own parent in a
    // param — the default says nothing, and a value names the folder, so the pack
    // emits the same project wherever its `use` stands.
    let stated = |k: &str| get(k).filter(|v| v.as_str() != Some(""));
    let has_org = stated("org_id").is_some() || stated("org").is_some() || stated("folder_id").is_some();
    // A parent stated by hand is written as declared, and it is written the way an
    // inherited one is: a dotted path is a reference to a node the estate declares
    // (`google_folder.shared.name`), anything else a literal id.
    for k in ["org_id", "folder_id"] {
        let Some(v) = stated(k) else { continue };
        if let Some(s) = v.as_str() {
            b = b.add_attribute(hcl::Attribute::new(k, crate::emit_shared::parse_expr(s)));
        } else if let Some(val) = crate::emit_shared::render_value(v, &|_| None) {
            b = b.add_attribute(hcl::Attribute::new(k, val));
        }
    }
    if !has_org {
        if let Some(folder) = last_with_prefix(path, "folder:") {
            b = b.add_attribute(hcl::Attribute::new(
                "folder_id",
                crate::emit_shared::traversal_expr(&format!(
                    "google_folder.{}.name",
                    folder.replace('-', "_")
                )),
            ));
        } else {
            b = b.add_attribute(hcl::Attribute::new("org_id", ctx.org_id.clone()));
        }
    }
    for (k, v) in attrs {
        let Some(k) = k.as_str() else { continue };
        if matches!(
            k,
            "project_id" | "name" | "billing_account" | "org_id" | "org" | "folder_id"
                | "project_service" | "import-id" | "lifecycle"
        ) {
            continue;
        }
        let is_block = matches!(v, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_))
            && !matches!(k, "labels" | "metadata" | "annotations");
        if is_block {
            if let Some(nb) = crate::emit_shared::render_block(k, v, None, &|_| None) {
                b = b.add_block(nb);
            }
        } else if let Some(val) = crate::emit_shared::render_value(v, &|_| None) {
            b = b.add_attribute(hcl::Attribute::new(k, val));
        }
    }
    if let Some(lc) = get("lifecycle").and_then(|v| crate::emit_shared::lifecycle_block(v, &|_| None)) {
        b = b.add_block(lc);
    }
    blocks.push(b.build());

    if let Some(serde_yaml::Value::Sequence(services)) = get("project_service") {
        for service_val in services {
            let (service, service_attrs): (String, Option<&serde_yaml::Mapping>) = match service_val {
                serde_yaml::Value::String(svc) => (svc.clone(), None),
                serde_yaml::Value::Mapping(m) => match m.get(serde_yaml::Value::String("service".into())) {
                    Some(serde_yaml::Value::String(svc)) => (svc.clone(), Some(m)),
                    _ => {
                        let Some((serde_yaml::Value::String(svc), mv)) = m.iter().next() else {
                            continue;
                        };
                        (svc.clone(), mv.as_mapping())
                    }
                },
                _ => continue,
            };
            let label = format!("{}_{}", resource_name, service.replace('.', "_"));
            blocks.push(crate::emit_shared::project_service_block(
                &label,
                crate::emit_shared::traversal_expr(&format!(
                    "google_project.{}.project_id",
                    resource_name
                )),
                &service,
                service_attrs,
                Some(&alias_for(path)),
                &|_| None,
            ));
            if let Some(id) = service_attrs
                .and_then(|m| m.get(serde_yaml::Value::String("import-id".into())))
                .and_then(|v| v.as_str())
            {
                imports.push(import_block(&format!("google_project_service.{}", label), id));
            }
        }
    }
    Ok(())
}

/// The estate's `deployment_mode`, which selects the backend `providers.tf` carries:
/// `local` the state file, `cloud` the `gcs` bucket, and `local` when the estate binds
/// none. Any other value is refused naming the two — the emitter has no backend for it.
///
/// `cloud` is refused, naming what is missing, when `svc_iac_account` or
/// `infra_project_name` has no value: cloud mode runs as the account the two name, and
/// without them it would run as whoever is logged in.
pub(crate) fn deployment_mode(env: &Env) -> Result<&'static str, String> {
    match env.get("deployment_mode") {
        None => Ok("local"),
        Some(serde_yaml::Value::String(s)) if s == "local" => Ok("local"),
        Some(serde_yaml::Value::String(s)) if s == "cloud" => {
            let named = |k: &str| env.get(k).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
            let missing: Vec<String> = ["svc_iac_account", "infra_project_name"]
                .into_iter()
                .filter(|k| !named(k))
                .map(|k| format!("`{}`", k))
                .collect();
            if missing.is_empty() {
                return Ok("cloud");
            }
            Err(format!(
                "`deployment_mode = \"cloud\"` without a value for {}: cloud mode runs every live call and \
                 `tofu` as `{{svc_iac_account}}@{{infra_project_name}}.iam.gserviceaccount.com`, so the estate \
                 binds both — bind {} in `params {{}}`, or keep `deployment_mode = \"local\"`",
                missing.join(" or "),
                if missing.len() == 1 { "it" } else { "them" }
            ))
        }
        Some(v) => {
            let given = match v {
                serde_yaml::Value::String(s) => format!("\"{}\"", s),
                other => serde_yaml::to_string(other).map(|s| s.trim().to_string()).unwrap_or_default(),
            };
            Err(format!(
                "`deployment_mode = {}`: the mode is \"local\" (the state in a file) or \"cloud\" \
                 (the state in the gcs bucket), and no backend is emitted for anything else",
                given
            ))
        }
    }
}

/// The region the estate's own `google` provider works in, read from the
/// `providers` block it wrote (already `{param}`-resolved by the front end, so
/// `region = default_region` arrives here as the bound value). Every
/// per-project alias carries it, and an estate whose provider names no region
/// gets aliases that name none.
fn google_provider_region(config: &std::collections::BTreeMap<String, serde_yaml::Value>) -> Option<String> {
    config
        .get("providers")?
        .as_mapping()?
        .get(serde_yaml::Value::String("google".into()))?
        .as_mapping()?
        .get(serde_yaml::Value::String("region".into()))?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// providers.tf from the estate config + folded projects: terraform block
/// (mode-matched backend, required_providers), root providers, one alias per
/// project — the same shapes the walk emits, from the same shared builders.
pub(crate) fn emit_providers(
    config: &std::collections::BTreeMap<String, serde_yaml::Value>,
    folded: &Folded,
    env: &Env,
    provider_sources: &std::collections::HashMap<String, String>,
    provider_versions: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let mode = deployment_mode(env)?;
    let deps = crate::emit_shared::GoogleProviderDeps {
        infra_project: env.get("infra_project_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
        // `deployment_mode` refuses cloud mode without both params, so cloud mode always names one
        impersonate: if mode == "cloud" {
            crate::prerequisites::service_account_of(|k| env.get(k).and_then(|v| v.as_str()).map(str::to_string))
        } else {
            None
        },
        region: google_provider_region(config),
    };

    let mut blocks: Vec<hcl::Block> = Vec::new();

    // terraform block
    let tf_val = config
        .get("terraform")
        .ok_or("Missing 'terraform' block in the estate configuration")?;
    let mut tf_block = hcl::Block::builder("terraform");
    let mut has_required_providers = false;
    if let serde_yaml::Value::Mapping(map) = tf_val {
        for (k, v) in map {
            let Some(k_str) = k.as_str() else { continue };
            if k_str == "backend" {
                if let serde_yaml::Value::Mapping(be_map) = v {
                    for (be_type, be_config) in be_map {
                        let Some(be_type_str) = be_type.as_str() else { continue };
                        if (mode == "local" && be_type_str == "local") || (mode == "cloud" && be_type_str == "gcs") {
                            let mut be_builder = hcl::Block::builder("backend").add_label(be_type_str);
                            let mut declares_impersonation = false;
                            if let serde_yaml::Value::Mapping(c_map) = be_config {
                                for (ck, cv) in c_map {
                                    if let (Some(cks), Some(cval)) =
                                        (ck.as_str(), crate::emit_shared::render_value(cv, &|_| None))
                                    {
                                        if cks == "impersonate_service_account" {
                                            declares_impersonation = true;
                                        }
                                        be_builder = be_builder.add_attribute((cks, cval));
                                    }
                                }
                            }
                            // The state bucket gets the same identity as the
                            // resources. Without this the backend authenticates
                            // as the HUMAN while every resource operation runs as
                            // the service account — a split identity inside one
                            // `tofu apply`, and a standing requirement that every
                            // operator keep object access on the state bucket.
                            //
                            // Only `gcs`: the `local` backend is a file, and only
                            // when the estate has not said otherwise, matching how
                            // `configure_google_provider` yields to a declared
                            // `billing_project`.
                            if be_type_str == "gcs" && !declares_impersonation {
                                if let Some(sa) = &deps.impersonate {
                                    be_builder = be_builder
                                        .add_attribute(("impersonate_service_account", sa.clone()));
                                }
                            }
                            tf_block = tf_block.add_block(be_builder.build());
                        }
                    }
                }
            } else if k_str == "required_providers" {
                has_required_providers = true;
                if let Some(rp_block) = crate::emit_shared::render_block("required_providers", v, None, &|_| None) {
                    tf_block = tf_block.add_block(rp_block);
                }
            } else if let Some(val) = crate::emit_shared::render_value(v, &|_| None) {
                tf_block = tf_block.add_attribute((k_str, val));
            }
        }
    }
    let providers_cfg = config.get("providers").and_then(|v| v.as_mapping());
    if !has_required_providers {
        if let Some(providers) = providers_cfg {
            let mut rp_builder = hcl::Block::builder("required_providers");
            for p_name in providers.keys().filter_map(|k| k.as_str()) {
                if let Some(source) = provider_sources.get(p_name) {
                    let mut p_map = hcl::Map::new();
                    p_map.insert("source".to_string(), hcl::Value::from(source.clone()));
                    if let Some(ver) = provider_versions.get(p_name) {
                        p_map.insert("version".to_string(), hcl::Value::from(ver.clone()));
                    }
                    rp_builder = rp_builder.add_attribute((p_name, hcl::Value::from(p_map)));
                }
            }
            tf_block = tf_block.add_block(rp_builder.build());
        }
    }
    blocks.push(tf_block.build());

    // root providers from config
    if let Some(providers) = providers_cfg {
        for (p_name, p_val) in providers {
            let Some(p_name) = p_name.as_str() else { continue };
            match p_val {
                serde_yaml::Value::Sequence(seq) => {
                    for item in seq {
                        if let serde_yaml::Value::Mapping(m) = item {
                            blocks.push(crate::emit_shared::provider_block_from_map(p_name, m, &deps, &|_| None));
                        }
                    }
                }
                serde_yaml::Value::Mapping(m) => {
                    blocks.push(crate::emit_shared::provider_block_from_map(p_name, m, &deps, &|_| None));
                }
                _ => {}
            }
        }
    }

    // one alias per project entity
    for (addr, slot) in &folded.slots {
        if addr.tf_type != "google_project" {
            continue;
        }
        if let Slot::Ok(e) = slot {
            if let Body::Attrs(serde_yaml::Value::Mapping(m)) = &e.body {
                if let Some(pid) = m.get(serde_yaml::Value::String("project_id".into())).and_then(|v| v.as_str()) {
                    blocks.push(crate::emit_shared::project_provider_block(&addr.label, pid, &deps));
                }
            }
        }
    }

    let mut body = hcl::Body::builder();
    for b in blocks {
        body = body.add_block(b);
    }
    hcl::to_string(&body.build()).map_err(|e| e.to_string())
}

/// The type `variables.tf` declares for a param, from its value — the element
/// shapes as well as the container's. A collection of scalars is `list(string)` or
/// `map(string)`, into which Terraform converts a bool or a number. A collection
/// holding a collection is `any`: `terraform.tfvars` carries the value as it is,
/// and its elements need not share one shape — the CIS baseline's
/// `cis_sa_key_creation_rules` is `[ { enforce = "TRUE" } ]`, and an exemption
/// adds a second rule with a `condition` the first does not have, which no single
/// `object({…})` accepts.
pub(crate) fn variable_type(v: &serde_yaml::Value) -> &'static str {
    use serde_yaml::Value;
    let scalar = |v: &Value| !matches!(v, Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_));
    match v {
        Value::Sequence(items) if items.iter().all(scalar) => "list(string)",
        Value::Mapping(m) if m.values().all(scalar) => "map(string)",
        Value::Sequence(_) | Value::Mapping(_) => "any",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        _ => "string",
    }
}

/// variables.tf: one declaration per accumulated param, typed by `variable_type`.
/// `descriptions` comes from the questions the packs declare: a param worth
/// asking about is worth describing in the generated `variables.tf`, and the
/// prompt is already the one-line human sentence for it. `why` stays out — it is
/// prose, and belongs on the pack's page rather than in every estate.
pub(crate) fn emit_variables(
    tfvars: &Env,
    descriptions: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut body = hcl::Body::builder();
    for (name, v) in tfvars {
        let ty = variable_type(v);
        let mut blk = hcl::Block::builder("variable")
            .add_label(name.replace('_', "-"))
            .add_attribute(("type", ty.parse::<hcl::Expression>().unwrap()));
        if let Some(d) = descriptions.get(name) {
            blk = blk.add_attribute(("description", d.clone()));
        }
        body = body.add_block(blk.build());
    }
    hcl::to_string(&body.build()).unwrap_or_default()
}

/// The `data` blocks a project estate's values read from the interface files it uses, once
/// each, in the order the lookups need one another — the same data sources the HCL form of
/// the interface reads, through the provider the estate's top-level resources are written
/// with. Each carries the resource of the central estate it reads back and the read
/// permission the project's plan needs.
pub(crate) fn interface_lookups_tf(lookups: &[satz_core::satz::OfferedLookup], files: &[satz_core::pipeline::UsedInterfaceFile]) -> String {
    let mut out = String::new();
    for l in lookups {
        let from = files.iter().find(|f| f.interface.lookups.iter().any(|x| x.address == l.address));
        let (source, label) = l.source_and_label();
        // through the provider every resource at the top of the estate is written with
        let provider: hcl::Expression = alias_for(&[]).parse().expect("a provider alias is a traversal");
        let mut b = hcl::Block::builder("data").add_label(source).add_label(label).add_attribute(("provider", provider));
        for (k, v) in &l.arguments {
            b = b.add_attribute((k.as_str(), crate::emit_shared::string_to_hcl_expr(v)));
        }
        out.push('\n');
        out.push_str(&format!(
            "# reads back {} of the estate `{}` ({}); needs {}\n",
            l.reads,
            from.map(|f| f.interface.estate.as_str()).unwrap_or_default(),
            from.map(|f| f.file.as_str()).unwrap_or_default(),
            l.permission
        ));
        out.push_str(&hcl::to_string(&hcl::Body::builder().add_block(b.build()).build()).expect("a data block of string arguments renders"));
    }
    out
}

/// tfvars from the front-end's accumulated params: kebab-cased names, scalar
/// renderings matching the walk's output.
pub(crate) fn emit_tfvars(env: &Env) -> String {
    let mut out = String::new();
    for (name, v) in env {
        let rendered = match v {
            serde_yaml::Value::String(s) => {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
            }
            serde_yaml::Value::Number(n) => n.to_string(),
            serde_yaml::Value::Bool(b) => b.to_string(),
            // Lists/maps must be HCL, never YAML — the walk rendered these
            // through the expression layer too.
            other => match crate::emit_shared::render_value(other, &|_| None) {
                Some(expr) => expr.to_string(),
                None => continue,
            },
        };
        out.push_str(&format!("{} = {}\n", name.replace('_', "-"), rendered));
    }
    out
}

#[cfg(test)]
mod backend_identity_tests {
    //! Who the STATE bucket is read and written as.
    //!
    //! The provider block has carried `impersonate_service_account` since cloud
    //! mode existed, so resource operations run as the estate's IaC service
    //! account. The backend did not: its attributes are copied verbatim from the
    //! estate, which declares only `bucket` and `prefix`. One `tofu apply` then
    //! used two principals — the service account for every resource, the human
    //! for the state — and every operator needed standing object access on the
    //! state bucket for as long as the estate lived.
    //!
    //! `providers.tf` is not in the corpus snapshots (those are `main.tf` plus
    //! tfvars), so nothing else gates this.

    use super::*;
    use std::collections::{BTreeMap, HashMap};

    fn config_with_both_backends() -> BTreeMap<String, serde_yaml::Value> {
        let tf: serde_yaml::Value = serde_yaml::from_str(
            "backend:\n  local:\n    path: terraform.tfstate\n  gcs:\n    bucket: corp-infra-001-state\n    prefix: hcl/state\n",
        )
        .expect("valid test YAML");
        BTreeMap::from([("terraform".to_string(), tf)])
    }

    fn env(mode: &str) -> Env {
        BTreeMap::from([
            ("deployment_mode".to_string(), serde_yaml::Value::from(mode)),
            ("svc_iac_account".to_string(), serde_yaml::Value::from("svc-iac-001")),
            ("infra_project_name".to_string(), serde_yaml::Value::from("corp-infra-001")),
        ])
    }

    fn providers_tf(config: &BTreeMap<String, serde_yaml::Value>, env: &Env) -> String {
        emit_providers(
            config,
            &Folded { slots: BTreeMap::new() },
            env,
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("providers.tf emits")
    }

    #[test]
    fn the_cloud_backend_runs_as_the_estate_service_account() {
        let out = providers_tf(&config_with_both_backends(), &env("cloud"));
        assert!(out.contains(r#"backend "gcs""#), "the gcs backend was not selected:\n{}", out);
        assert!(
            out.contains(
                r#"impersonate_service_account = "svc-iac-001@corp-infra-001.iam.gserviceaccount.com""#
            ),
            "the state bucket is still read as the human:\n{}",
            out
        );
    }

    #[test]
    fn the_local_backend_is_a_file_and_impersonates_nothing() {
        let out = providers_tf(&config_with_both_backends(), &env("local"));
        assert!(out.contains(r#"backend "local""#), "the local backend was not selected:\n{}", out);
        assert!(
            !out.contains("impersonate_service_account"),
            "local mode must not impersonate anywhere:\n{}",
            out
        );
    }

    /// An estate that names its own backend identity keeps it — the same
    /// deference `configure_google_provider` shows a declared `billing_project`.
    #[test]
    fn a_declared_backend_identity_is_not_overwritten() {
        let tf: serde_yaml::Value = serde_yaml::from_str(
            "backend:\n  gcs:\n    bucket: corp-infra-001-state\n    impersonate_service_account: state-reader@corp-infra-001.iam.gserviceaccount.com\n",
        )
        .expect("valid test YAML");
        let config = BTreeMap::from([("terraform".to_string(), tf)]);
        let out = providers_tf(&config, &env("cloud"));
        assert!(
            out.contains("state-reader@corp-infra-001"),
            "the estate's own choice was lost:\n{}",
            out
        );
        assert!(
            !out.contains("svc-iac-001@corp-infra-001"),
            "the derived account was added beside the declared one:\n{}",
            out
        );
    }

    /// `local`, `cloud`, or nothing bound — which is `local`. Every other value, a string
    /// or not, is refused naming what was bound, and `providers.tf` is not emitted
    /// without the backend it would have selected.
    #[test]
    fn the_mode_is_local_or_cloud_and_nothing_else() {
        let bound = |v: serde_yaml::Value| BTreeMap::from([("deployment_mode".to_string(), v)]);
        assert_eq!(deployment_mode(&Env::new()), Ok("local"));
        assert_eq!(deployment_mode(&bound("local".into())), Ok("local"));
        assert_eq!(deployment_mode(&env("cloud")), Ok("cloud"));
        for (v, shown) in [
            (serde_yaml::Value::from("boot"), "`deployment_mode = \"boot\"`"),
            (serde_yaml::Value::from(""), "`deployment_mode = \"\"`"),
            (serde_yaml::Value::from("Cloud"), "`deployment_mode = \"Cloud\"`"),
            (serde_yaml::Value::Bool(true), "`deployment_mode = true`"),
        ] {
            let e = deployment_mode(&bound(v)).unwrap_err();
            assert!(e.starts_with(shown) && e.contains("\"local\"") && e.contains("\"cloud\""), "{}", e);
        }
        let e = emit_providers(
            &config_with_both_backends(),
            &Folded { slots: BTreeMap::new() },
            &bound("boot".into()),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(e.contains("\"boot\""), "{}", e);
    }

    /// Cloud mode without the two params that name the account has no identity to run
    /// as — it would run as whoever is logged in — so it is refused naming what is
    /// missing, and `providers.tf` is not emitted. An empty value names nothing either.
    #[test]
    fn cloud_mode_without_the_params_is_refused() {
        let cloud = |pairs: &[(&str, &str)]| -> Env {
            let mut env = BTreeMap::from([("deployment_mode".to_string(), serde_yaml::Value::from("cloud"))]);
            for (k, v) in pairs {
                env.insert(k.to_string(), serde_yaml::Value::from(*v));
            }
            env
        };
        for (env, missing) in [
            (cloud(&[]), "without a value for `svc_iac_account` or `infra_project_name`: "),
            (cloud(&[("infra_project_name", "corp-infra-001")]), "without a value for `svc_iac_account`: "),
            (cloud(&[("svc_iac_account", "svc-iac-001")]), "without a value for `infra_project_name`: "),
            (
                cloud(&[("svc_iac_account", "svc-iac-001"), ("infra_project_name", "")]),
                "without a value for `infra_project_name`: ",
            ),
        ] {
            let e = deployment_mode(&env).unwrap_err();
            assert!(e.starts_with("`deployment_mode = \"cloud\"` ") && e.contains(missing), "{}", e);
            let emitted = emit_providers(
                &config_with_both_backends(),
                &Folded { slots: BTreeMap::new() },
                &env,
                &HashMap::new(),
                &HashMap::new(),
            );
            assert_eq!(emitted, Err(e));
        }
    }
}

#[cfg(test)]
mod project_alias_tests {
    //! The per-project provider alias — `provider "google" { alias =
    //! project_<label> }`, which serves every resource written inside that
    //! project node.
    //!
    //! It used to carry `region = "europe-west3"` as a literal and
    //! `billing_project = <infra project>`: an estate working in another region
    //! created its regional resources in Frankfurt, and a resource whose own
    //! project had its API on was refused with a 403 naming the infra project,
    //! whose API was off. `providers.tf` is in no corpus snapshot, so these
    //! tests are its gate.

    use super::*;
    use std::collections::{BTreeMap, HashMap};

    /// An estate: the backend, a `google` provider working in `region`, and one
    /// project entity beside the infra project.
    fn config(region: &str) -> BTreeMap<String, serde_yaml::Value> {
        let tf: serde_yaml::Value =
            serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n").expect("valid test YAML");
        let providers: serde_yaml::Value = serde_yaml::from_str(&format!(
            "google:\n  alias: google\n  project: corp-infra-001\n  region: {}\n  user_project_override: true\n  billing_project: corp-infra-001\n",
            region
        ))
        .expect("valid test YAML");
        BTreeMap::from([("terraform".to_string(), tf), ("providers".to_string(), providers)])
    }

    fn folded_projects(projects: &[(&str, &str)]) -> Folded {
        let mut slots = BTreeMap::new();
        for (label, project_id) in projects {
            let addr = satz_core::Address { tf_type: "google_project".into(), label: (*label).into() };
            let body: serde_yaml::Value =
                serde_yaml::from_str(&format!("project_id: {}\nname: {}\n", project_id, label)).expect("valid test YAML");
            slots.insert(
                addr.clone(),
                Slot::Ok(satz_core::algebra::Entity {
                    addr,
                    scope: satz_core::Scope::Node,
                    body: Body::Attrs(body),
                    provenance: Vec::new(),
                    node_path: Vec::new(),
                }),
            );
        }
        Folded { slots }
    }

    fn env() -> Env {
        BTreeMap::from([
            ("deployment_mode".to_string(), serde_yaml::Value::from("local")),
            ("infra_project_name".to_string(), serde_yaml::Value::from("corp-infra-001")),
        ])
    }

    fn providers_tf(region: &str, projects: &[(&str, &str)]) -> String {
        emit_providers(&config(region), &folded_projects(projects), &env(), &HashMap::new(), &HashMap::new())
            .expect("providers.tf emits")
    }

    /// The alias block, by its alias, as its lines.
    fn alias_block(out: &str, alias: &str) -> Vec<String> {
        let body = hcl::parse(out).expect("emitted providers.tf parses");
        for b in body.blocks() {
            if b.identifier() != "provider" || b.labels().first().map(|l| l.as_str()) != Some("google") {
                continue;
            }
            let is_it = b.body().attributes().any(|a| {
                a.key() == "alias" && matches!(a.expr(), hcl::Expression::String(s) if s == alias)
            });
            if is_it {
                return b
                    .body()
                    .attributes()
                    .map(|a| format!("{} = {}", a.key(), hcl::format::to_string(a.expr()).unwrap().trim()))
                    .collect();
            }
        }
        panic!("no provider block with alias {}:\n{}", alias, out);
    }

    /// The region is the estate's, wherever the estate works.
    #[test]
    fn every_alias_works_in_the_region_the_estate_declares() {
        for region in ["europe-west3", "us-central1", "australia-southeast2"] {
            let out = providers_tf(region, &[("infra", "corp-infra-001"), ("data", "corp-data-001")]);
            for alias in ["project_infra", "project_data"] {
                assert!(
                    alias_block(&out, alias).contains(&format!("region = \"{}\"", region)),
                    "alias {} does not work in {}:\n{}",
                    alias,
                    region,
                    out
                );
            }
            assert!(
                !out.contains("europe-west3") || region == "europe-west3",
                "a region literal survived in:\n{}",
                out
            );
        }
    }

    /// An estate whose `google` provider names no region gets aliases that name
    /// none: a regional resource that writes no `region` is then refused by the
    /// provider, rather than created where the estate never said.
    #[test]
    fn an_estate_that_names_no_region_gets_aliases_that_name_none() {
        let tf: serde_yaml::Value =
            serde_yaml::from_str("backend:\n  local:\n    path: terraform.tfstate\n").expect("valid test YAML");
        let providers: serde_yaml::Value =
            serde_yaml::from_str("google:\n  alias: google\n  project: corp-infra-001\n").expect("valid test YAML");
        let config = BTreeMap::from([("terraform".to_string(), tf), ("providers".to_string(), providers)]);
        let out = emit_providers(
            &config,
            &folded_projects(&[("data", "corp-data-001")]),
            &env(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("providers.tf emits");
        assert!(
            !alias_block(&out, "project_data").iter().any(|l| l.starts_with("region =")),
            "an alias invented a region:\n{}",
            out
        );
    }

    /// The quota project of a project's alias is that project. Billing it to the
    /// infra project asks Google for the API on a project the resource has
    /// nothing to do with, and the create is refused with a 403 naming it.
    #[test]
    fn a_projects_alias_bills_to_that_project() {
        let out = providers_tf("europe-west3", &[("infra", "corp-infra-001"), ("data", "corp-data-001")]);
        assert!(
            alias_block(&out, "project_data").contains(&"billing_project = \"corp-data-001\"".to_string()),
            "the alias for corp-data-001 does not bill to it:\n{}",
            out
        );
        assert!(
            alias_block(&out, "project_data").contains(&"user_project_override = true".to_string()),
            "the quota project is named and not switched on:\n{}",
            out
        );
        // the estate's own default provider is untouched: org-scoped calls have
        // no project of their own and are billed to the infra project
        assert!(
            alias_block(&out, "google").contains(&"billing_project = \"corp-infra-001\"".to_string()),
            "the default provider stopped billing to the infra project:\n{}",
            out
        );
    }

    /// Cloud mode: the alias acts as the estate's IaC service account, like
    /// every other block in the file.
    #[test]
    fn an_alias_impersonates_the_estates_service_account() {
        let mut env = env();
        env.insert("deployment_mode".to_string(), serde_yaml::Value::from("cloud"));
        env.insert("svc_iac_account".to_string(), serde_yaml::Value::from("svc-iac-001"));
        let out = emit_providers(
            &config("europe-west3"),
            &folded_projects(&[("data", "corp-data-001")]),
            &env,
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("providers.tf emits");
        assert!(
            alias_block(&out, "project_data").contains(
                &"impersonate_service_account = \"svc-iac-001@corp-infra-001.iam.gserviceaccount.com\"".to_string()
            ),
            "the alias runs as whoever is logged in:\n{}",
            out
        );
    }
}

#[cfg(test)]
mod service_account_order_tests {
    //! A grant naming a service account by email ran beside the account's
    //! creation and failed; on destroy it outlived the account.
    use super::*;

    fn blocks(tf: &str) -> Vec<hcl::Block> {
        hcl::parse(tf).expect("fixture is valid HCL").blocks().cloned().collect()
    }

    fn depends_on(b: &hcl::Block) -> Option<String> {
        b.body().attributes().find(|a| a.key() == "depends_on").map(|a| hcl::format::to_string(a.expr()).unwrap().split_whitespace().collect())
    }

    /// Found applying the CIS baseline to a customer organisation: `409
    /// CONCURRENT_POLICY_CHANGES` on one of twenty-eight unordered policies.
    #[test]
    fn the_policies_on_one_parent_apply_one_at_a_time() {
        let mut bs = blocks(
            r#"
resource "google_org_policy_policy" "c_policy" {
  name = "organizations/123456789012/policies/c"
  parent = "organizations/123456789012"
}
resource "google_org_policy_policy" "a_policy" {
  name = "organizations/123456789012/policies/a"
  parent = "organizations/123456789012"
}
resource "google_org_policy_policy" "b_policy" {
  name = "organizations/123456789012/policies/b"
  parent = "organizations/123456789012"
  depends_on = [google_org_policy_custom_constraint.b]
}
resource "google_org_policy_policy" "folder_policy" {
  name = "folders/111/policies/a"
  parent = "folders/111"
}
resource "google_storage_bucket" "state" {
  name = "acme-infra-001-state"
}
"#,
        );
        order_org_policies_per_parent(&mut bs);
        let by = |label: &str| bs.iter().find(|b| b.labels()[1].as_str() == label).unwrap();
        // in address order, each waits for the one before it on the same parent
        assert_eq!(depends_on(by("a_policy")), None);
        assert_eq!(depends_on(by("b_policy")).as_deref(), Some("[google_org_policy_custom_constraint.b,google_org_policy_policy.a_policy]"));
        assert_eq!(depends_on(by("c_policy")).as_deref(), Some("[google_org_policy_policy.b_policy]"));
        // another parent is another queue, and nothing else is ordered
        assert_eq!(depends_on(by("folder_policy")), None);
        assert_eq!(depends_on(by("state")), None);
    }

    #[test]
    fn a_resource_waits_for_the_service_that_enables_its_api() {
        let mut bs = blocks(
            r#"
resource "google_project" "infra" {
  project_id = "acme-infra-001"
}
resource "google_project_service" "infra_billingbudgets_googleapis_com" {
  project = google_project.infra.project_id
  service = "billingbudgets.googleapis.com"
}
resource "google_project_service" "infra_logging_googleapis_com" {
  project = google_project.infra.project_id
  service = "logging.googleapis.com"
}
resource "google_billing_budget" "global_budget" {
  billing_account = "012345-6789AB-CDEF01"
  display_name = "Global Budget"
}
resource "google_logging_organization_sink" "audit" {
  name = "audit"
  org_id = "123456789012"
}
resource "google_storage_bucket" "state" {
  name = "acme-infra-001-state"
  project = "acme-infra-001"
}
"#,
        );
        order_after_project_services(&mut bs, "acme-infra-001");
        let by = |label: &str| bs.iter().find(|b| b.labels()[1].as_str() == label).unwrap();
        // the budget hangs off the billing account and still waits for the API on
        // the infra project — every call the provider makes is billed there
        assert_eq!(
            depends_on(by("global_budget")).as_deref(),
            Some("[google_project_service.infra_billingbudgets_googleapis_com]")
        );
        assert_eq!(depends_on(by("audit")).as_deref(), Some("[google_project_service.infra_logging_googleapis_com]"));
        // an API the estate declares nowhere orders nothing: the compile reports it
        assert_eq!(depends_on(by("state")), None);
    }

    /// The three edges that would make tofu refuse the whole graph rather than one
    /// resource: a service waiting for a service, the project a service is declared
    /// on, and the folder that project sits in — reached through the project, which
    /// is why the rule is the reference closure and not a list of types.
    #[test]
    fn the_ordering_pass_never_closes_a_cycle() {
        let mut bs = blocks(
            r#"
resource "google_folder" "infra_folder" {
  display_name = "Infrastructure"
  parent = "organizations/123456789012"
}
resource "google_project" "infra" {
  project_id = "acme-infra-001"
  folder_id = google_folder.infra_folder.name
}
resource "google_project_service" "infra_serviceusage_googleapis_com" {
  project = google_project.infra.project_id
  service = "serviceusage.googleapis.com"
}
resource "google_project_service" "infra_cloudresourcemanager_googleapis_com" {
  project = google_project.infra.project_id
  service = "cloudresourcemanager.googleapis.com"
}
resource "google_project_service" "infra_cloudbilling_googleapis_com" {
  project = google_project.infra.project_id
  service = "cloudbilling.googleapis.com"
}
"#,
        );
        order_after_project_services(&mut bs, "acme-infra-001");
        for b in &bs {
            assert_eq!(depends_on(b), None, "{:?} was ordered and must not be", b.labels());
        }
    }

    /// A service on a project this resource has nothing to do with is not its
    /// dependency: the two that can matter are its own project and the project the
    /// call is billed to.
    #[test]
    fn a_service_on_an_unrelated_project_is_no_dependency() {
        let mut bs = blocks(
            r#"
resource "google_project" "infra" {
  project_id = "acme-infra-001"
}
resource "google_project" "other" {
  project_id = "acme-other-001"
}
resource "google_project_service" "other_storage_googleapis_com" {
  project = google_project.other.project_id
  service = "storage.googleapis.com"
}
resource "google_project_service" "infra_storage_googleapis_com" {
  project = google_project.infra.project_id
  service = "storage.googleapis.com"
}
resource "google_storage_bucket" "elsewhere" {
  name = "acme-other-001-data"
  project = "acme-other-001"
}
"#,
        );
        order_after_project_services(&mut bs, "acme-infra-001");
        let by = |label: &str| bs.iter().find(|b| b.labels()[1].as_str() == label).unwrap();
        // its own project's service AND the infra project's, both and no more
        assert_eq!(
            depends_on(by("elsewhere")).as_deref(),
            Some("[google_project_service.infra_storage_googleapis_com,google_project_service.other_storage_googleapis_com]")
        );
    }

    #[test]
    fn grants_and_memberships_naming_a_declared_account_wait_for_it() {
        let mut bs = blocks(
            r#"
resource "google_project" "infra" {
  project_id = "acme-infra-001"
}
resource "google_service_account" "iac" {
  account_id = "svc-iac-001"
  project = google_project.infra.project_id
}
resource "google_service_account" "runner" {
  account_id = "satz-runner"
  project = "acme-infra-001"
}
resource "google_organization_iam_member" "iac_viewer" {
  role = "roles/viewer"
  member = "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com"
  org_id = "123456789012"
}
resource "google_project_iam_member" "runner_viewer" {
  role = "roles/viewer"
  member = "serviceAccount:satz-runner@acme-infra-001.iam.gserviceaccount.com"
  project = "acme-infra-001"
  depends_on = [google_project.infra]
}
resource "google_organization_iam_member" "elsewhere" {
  role = "roles/viewer"
  member = "serviceAccount:other@acme-infra-001.iam.gserviceaccount.com"
  org_id = "123456789012"
}
resource "google_cloud_identity_group_membership" "owner" {
  group = "groups/x"
  preferred_member_key {
    id = "svc-iac-001@acme-infra-001.iam.gserviceaccount.com"
  }
}
"#,
        );
        order_after_service_accounts(&mut bs);
        let by = |label: &str| bs.iter().find(|b| b.labels()[1].as_str() == label).unwrap();
        // the project reference resolves to the project's id
        assert_eq!(depends_on(by("iac_viewer")).as_deref(), Some("[google_service_account.iac]"));
        // an existing depends_on is extended, not replaced
        assert_eq!(depends_on(by("runner_viewer")).as_deref(), Some("[google_project.infra,google_service_account.runner]"));
        assert_eq!(depends_on(by("owner")).as_deref(), Some("[google_service_account.iac]"));
        // an account the estate does not declare orders nothing
        assert_eq!(depends_on(by("elsewhere")), None);
        assert_eq!(depends_on(by("iac")), None);
    }

    #[test]
    fn grants_and_memberships_naming_a_declared_group_wait_for_it() {
        // a grant names its group by email; on a fresh organisation the grant
        // used to run beside the group's creation and the first refusal
        // stopped the groups still queued
        let mut bs = blocks(
            r#"
resource "google_cloud_identity_group" "gcp_auditors" {
  display_name = "Auditors"
  parent = "customers/C0example"
  group_key {
    id = "gcp-auditors@example.com"
  }
}
resource "google_organization_iam_member" "auditors_viewer" {
  role = "roles/viewer"
  member = "group:gcp-auditors@example.com"
  org_id = "123456789012"
}
resource "google_project_iam_member" "auditors_logs" {
  role = "roles/logging.viewer"
  member = "group:gcp-auditors@example.com"
  project = "acme-infra-001"
  depends_on = [google_project.infra]
}
resource "google_organization_iam_member" "elsewhere" {
  role = "roles/viewer"
  member = "group:other@example.com"
  org_id = "123456789012"
}
resource "google_cloud_identity_group_membership" "nested" {
  group = "groups/x"
  preferred_member_key {
    id = "gcp-auditors@example.com"
  }
}
"#,
        );
        order_after_groups(&mut bs);
        let by = |label: &str| bs.iter().find(|b| b.labels()[1].as_str() == label).unwrap();
        assert_eq!(depends_on(by("auditors_viewer")).as_deref(), Some("[google_cloud_identity_group.gcp_auditors]"));
        // an existing depends_on is extended, not replaced
        assert_eq!(depends_on(by("auditors_logs")).as_deref(), Some("[google_project.infra,google_cloud_identity_group.gcp_auditors]"));
        // a group nested in another group waits for it too
        assert_eq!(depends_on(by("nested")).as_deref(), Some("[google_cloud_identity_group.gcp_auditors]"));
        // a group the estate does not declare orders nothing
        assert_eq!(depends_on(by("elsewhere")), None);
        assert_eq!(depends_on(by("gcp_auditors")), None);
    }

    #[test]
    fn a_policy_on_a_declared_custom_constraint_waits_for_it() {
        let mut bs = blocks(
            r#"
resource "google_org_policy_custom_constraint" "sql_protection" {
  name = "custom.cisCloudSqlDeletionProtection"
  parent = "organizations/123456789012"
}
resource "google_org_policy_policy" "sql_protection" {
  name = "organizations/123456789012/policies/custom.cisCloudSqlDeletionProtection"
  parent = "organizations/123456789012"
}
resource "google_org_policy_policy" "managed" {
  name = "organizations/123456789012/policies/compute.managed.vmCanIpForward"
  parent = "organizations/123456789012"
}
"#,
        );
        order_after_custom_constraints(&mut bs);
        let by = |label: &str| bs.iter().find(|b| b.labels()[0].as_str() == "google_org_policy_policy" && b.labels()[1].as_str() == label).unwrap();
        assert_eq!(depends_on(by("sql_protection")).as_deref(), Some("[google_org_policy_custom_constraint.sql_protection]"));
        assert_eq!(depends_on(by("managed")), None);
    }
}

#[cfg(test)]
mod required_argument_tests {
    //! A converted estate carried a custom role with no `role_id`: the provider
    //! requires it, nothing in the compile said so, and `tofu plan` was the first
    //! to refuse. The check reads what is emitted, so derived arguments count.
    use super::*;

    fn registry() -> crate::schema::ResourceRegistry {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
        crate::schema::ResourceRegistry::load_all(&dir.to_string_lossy()).expect("schema fixture")
    }

    #[test]
    fn a_resource_missing_a_required_argument_is_named_and_a_complete_one_is_not() {
        let body = hcl::parse(
            r#"
resource "google_organization_iam_custom_role" "no_id" {
  org_id      = "123456789012"
  title       = "Application owner"
  permissions = ["resourcemanager.projects.get"]
}
resource "google_organization_iam_custom_role" "complete" {
  org_id      = "123456789012"
  role_id     = "ApplicationOwner"
  title       = "Application owner"
  permissions = ["resourcemanager.projects.get"]
}
resource "not_a_type_the_registry_knows" "x" {}
"#,
        )
        .unwrap();
        let reg = registry();
        let blocks: Vec<&hcl::Block> = body.blocks().collect();
        assert_eq!(missing_required(blocks[0], &reg), vec!["role_id".to_string()]);
        assert!(missing_required(blocks[1], &reg).is_empty());
        // no schema, no verdict
        assert!(missing_required(blocks[2], &reg).is_empty());
    }

    /// The customer's apply: one address for a set of addresses.
    #[test]
    fn a_literal_the_schema_s_type_refuses_is_named_and_a_convertible_one_is_not() {
        let body = hcl::parse(
            r#"
resource "google_organization_access_approval_settings" "one_address" {
  organization_id     = "123456789012"
  notification_emails = "security@example.com"
}
resource "google_organization_access_approval_settings" "a_list" {
  organization_id     = "123456789012"
  notification_emails = ["security@example.com"]
}
resource "google_storage_bucket" "converts" {
  name     = "acme-logs"
  location = 42
  project  = "${var.project}"
}
resource "google_storage_bucket" "a_list_for_a_string" {
  name     = ["acme-logs"]
  location = "EU"
}
"#,
        )
        .unwrap();
        let reg = registry();
        let blocks: Vec<&hcl::Block> = body.blocks().collect();
        assert_eq!(
            wrong_shapes(blocks[0], &reg),
            vec![("notification_emails".to_string(), "set(string)".to_string(), "string")]
        );
        assert!(wrong_shapes(blocks[1], &reg).is_empty());
        // Terraform converts a number to a string, and an interpolation is known at plan
        assert!(wrong_shapes(blocks[2], &reg).is_empty());
        assert_eq!(wrong_shapes(blocks[3], &reg), vec![("name".to_string(), "string".to_string(), "list")]);
    }
}


#[cfg(test)]
mod grant_condition_tests {
    //! A conditional grant is emitted with its condition or not at all. The
    //! condition is part of the binding's identity and of its scope: losing it
    //! would widen the grant and move its address in one stroke.

    use super::*;

    fn edge(condition: &str) -> satz_core::algebra::GrantEdge {
        satz_core::algebra::GrantEdge {
            member: "group:gcp-viewers@example.com".into(),
            role: "roles/viewer".into(),
            condition: condition.into(),
            import_id: String::new(),
        }
    }

    #[test]
    fn an_absent_condition_is_none() {
        assert_eq!(edge_condition(&edge("")).expect("no condition is not an error"), None);
    }

    #[test]
    fn a_canonical_condition_parses_back() {
        let cond = edge_condition(&edge("expression: request.time < timestamp('2027-01-01T00:00:00Z')\ntitle: expires\n"))
            .expect("canonical YAML parses")
            .expect("a condition was given");
        assert_eq!(cond["title"].as_str(), Some("expires"));
    }

    #[test]
    fn a_condition_that_does_not_parse_refuses_the_grant() {
        let err = edge_condition(&edge("title: [unclosed")).expect_err("must not degrade to an unconditional grant");
        assert!(err.contains("roles/viewer") && err.contains("condition"), "{err}");
    }
}

#[cfg(test)]
mod billing_grant_tests {
    //! A billing-account grant names the account it binds on, or is refused.
    //! Neither a pinned `billing_account_id` nor the `billing_account_infra`
    //! param is required elsewhere, so this was the one place an empty string
    //! reached the provider as a resource id.

    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn billing_grant() -> Folded {
        let addr = satz_core::Address { tf_type: "google_billing_account_iam_member".into(), label: "billing".into() };
        let edge = satz_core::algebra::GrantEdge {
            member: "group:gcp-billing-admins@example.com".into(),
            role: "roles/billing.admin".into(),
            condition: String::new(),
            import_id: String::new(),
        };
        let entity = satz_core::algebra::Entity {
            addr: addr.clone(),
            scope: satz_core::Scope::Billing,
            body: Body::Grant(BTreeSet::from([edge])),
            provenance: Vec::new(),
            node_path: Vec::new(),
        };
        Folded { slots: BTreeMap::from([(addr, Slot::Ok(entity))]) }
    }

    fn ctx(fallback: Option<&str>) -> EmitCtx<'static> {
        EmitCtx {
            customer_id: String::new(),
            customer_domain: String::new(),
            org_id: String::new(),
            billing_fallback: fallback.map(serde_yaml::Value::from),
            infra_project: String::new(),
            registry: None,
        }
    }

    #[test]
    fn no_account_anywhere_is_refused_not_emitted_empty() {
        let err = emit(&billing_grant(), &ctx(None)).err().expect("an empty billing_account_id must not reach the provider");
        assert!(err.contains("billing_account_infra") && err.contains("billing_account_id"), "{err}");
    }

    #[test]
    fn the_conventional_param_is_the_account() {
        let out = emit(&billing_grant(), &ctx(Some("example-billing"))).expect("the param supplies the account");
        assert!(out.main_tf.contains("example-billing"), "{}", out.main_tf);
    }
}

#[cfg(test)]
mod reset_comment_tests {
    //! `satz plan` and `satz apply` replace an org policy the state holds with rules
    //! while the estate declares it reset (ADR 0011). A bare `tofu apply` of the
    //! handed-over HCL does not, so `main.tf` names the replace above the policy.

    use super::*;

    const BLOCKS: &str = "resource \"google_org_policy_policy\" \"twin_superseded\" {\n  name = \"a\"\n  spec {\n    reset = true\n  }\n}\n\
        resource \"google_org_policy_policy\" \"enforced\" {\n  name = \"c\"\n  spec {\n    rules {\n      enforce = \"TRUE\"\n    }\n  }\n}\n\
        resource \"google_folder\" \"f\" {\n  display_name = \"f\"\n}\n";

    #[test]
    fn a_reset_policy_names_its_replace_and_nothing_else_moves() {
        let blocks: Vec<hcl::Block> = hcl::parse(BLOCKS).unwrap().blocks().cloned().collect();
        let manifest = crate::manifest::Manifest::from_blocks(&blocks);
        let text = render_main_tf(blocks.clone(), &manifest).unwrap();

        // directly above the reset policy, naming its own address
        let above = format!(
            "{}resource \"google_org_policy_policy\" \"twin_superseded\" {{\n",
            reset_replace_comment("google_org_policy_policy.twin_superseded")
        );
        assert!(text.starts_with(&above), "{text}");
        assert!(text.contains("# satz plan and satz apply add the replace; a bare apply needs: tofu apply -replace=google_org_policy_policy.twin_superseded\n"), "{text}");
        // and on nothing else: the policy that keeps its rules carries none
        assert_eq!(text.matches("-replace=").count(), 1, "{text}");
        assert!(text.contains("}\n\nresource \"google_org_policy_policy\" \"enforced\" {\n"), "{text}");

        // the comment is the whole difference: without it, the text is the body as
        // `hcl` renders it, and it parses back to the same manifest
        let body = blocks.iter().cloned().fold(hcl::Body::builder(), |b, x| b.add_block(x)).build();
        let bare: String = text.lines().filter(|l| !l.starts_with('#')).map(|l| format!("{l}\n")).collect();
        assert_eq!(bare, hcl::to_string(&body).unwrap());
        assert_eq!(crate::manifest::Manifest::parse(&text), manifest);
    }
}

#[cfg(test)]
mod interface_lookup_tests {
    use satz_core::satz::OfferedLookup;

    /// A lookup a project reads is one `data` block, with its arguments as the interface
    /// file writes them — a key that reads another lookup a template of its address — and
    /// the provider the estate's top-level resources carry.
    #[test]
    fn a_lookup_is_one_data_block_through_the_estate_s_provider() {
        let l = |label: &str, parent: &str| OfferedLookup {
            address: format!("data.google_active_folder.{}", label),
            reads: format!("google_folder.{}", label),
            permission: "resourcemanager.folders.list on the parent".into(),
            arguments: vec![("display_name".into(), label.to_uppercase()), ("parent".into(), parent.into())],
            line: 1,
        };
        let tf = super::interface_lookups_tf(&[l("infra", "organizations/123456789012"), l("pay", "${data.google_active_folder.infra.name}")], &[]);
        let body = hcl::parse(&tf).expect("the data blocks are HCL");
        let blocks: Vec<&hcl::Block> = body.blocks().collect();
        assert_eq!(blocks.len(), 2, "{}", tf);
        assert!(tf.contains("data \"google_active_folder\" \"pay\" {") && tf.contains("parent = \"${data.google_active_folder.infra.name}\""), "{}", tf);
        assert!(tf.contains("provider = google.google"), "{}", tf);
        assert!(tf.contains("# reads back google_folder.pay"), "{}", tf);
    }
}
