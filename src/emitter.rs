//! Stage B emitter: `satz_core::algebra::Folded` → HCL (docs/stage-b.md).
//! Every block shape comes from `emit_shared` — the same builders the walk
//! transpiler calls — so parity is by construction, not by imitation. What this
//! module owns is only the mapping from folded entities to builder inputs:
//! structural context (folder/project chain) comes from `Entity::node_path`
//! instead of walk position — scope as data, not as interception.
//!
use satz_core::algebra::{Body, Folded, Slot};
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
}

/// A conditional grant edge carries its condition as canonical YAML text. Parse
/// it back so the emitted label hashes EXACTLY what the walk hashed (the label
/// is the Terraform address — it must not move for existing state), and so the
/// `condition { … }` block renders identically.
fn edge_condition(edge: &satz_core::algebra::GrantEdge) -> Option<serde_yaml::Value> {
    if edge.condition.is_empty() {
        return None;
    }
    serde_yaml::from_str(&edge.condition).ok()
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
                    let cond = edge_condition(&e);
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
                let fallback = ctx.billing_fallback.as_ref().and_then(|v| v.as_str()).unwrap_or("");
                let billing_id = pinned.as_deref().unwrap_or(fallback);
                for e in reconciled_edges(edges)? {
                    let cond = edge_condition(&e);
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
                    ("folder", rc.folder_ref.clone().or(rc.folder_id.clone()))
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
                    let cond = edge_condition(&e);
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
                let (block, import_id, label) = crate::emit_shared::single_resource_block(
                    t,
                    &addr.label,
                    attrs,
                    schema,
                    &rc,
                    Some(&alias),
                    ctx.billing_fallback.as_ref(),
                    &|_| None,
                    None,
                )
                .map_err(|e| e.to_string())?;
                if let Some(id) = import_id {
                    imports.push(import_block(&format!("{}.{}", t, label), &id));
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

    let mut manifest = crate::manifest::Manifest::from_blocks(&blocks);
    manifest.attach_imports(&imports);
    for (a, f, l) in &origins {
        manifest.set_origin(a, f, *l);
    }
    let mut body = hcl::Body::builder();
    for b in blocks {
        body = body.add_block(b);
    }
    let main_tf = hcl::to_string(&body.build()).map_err(|e| e.to_string())?;
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
    Ok(EmitOut { main_tf, imports_tf, manifest })
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
    let has_org = get("org_id").is_some() || get("org").is_some() || get("folder_id").is_some();
    // An explicit parent on the project is written as declared — it used to
    // be dropped silently (skipped below), leaving the project without one.
    for k in ["org_id", "folder_id"] {
        if let Some(v) = get(k).and_then(|v| crate::emit_shared::render_value(v, &|_| None)) {
            b = b.add_attribute(hcl::Attribute::new(k, v));
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
    let get_env = |k: &str| env.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mode = {
        let m = get_env("deployment_mode");
        if m.is_empty() { "local".to_string() } else { m }
    };
    let deps = crate::emit_shared::GoogleProviderDeps {
        infra_project: env.get("infra_project_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
        impersonate: if mode == "cloud" {
            match (
                env.get("svc_iac_account").and_then(|v| v.as_str()),
                env.get("infra_project_name").and_then(|v| v.as_str()),
            ) {
                (Some(a), Some(p)) => Some(format!("{}@{}.iam.gserviceaccount.com", a, p)),
                _ => None,
            }
        } else {
            None
        },
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
                            if let serde_yaml::Value::Mapping(c_map) = be_config {
                                for (ck, cv) in c_map {
                                    if let (Some(cks), Some(cval)) =
                                        (ck.as_str(), crate::emit_shared::render_value(cv, &|_| None))
                                    {
                                        be_builder = be_builder.add_attribute((cks, cval));
                                    }
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

/// variables.tf: one declaration per accumulated param, typed by value shape.
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
        let ty = match v {
            serde_yaml::Value::Sequence(_) => "list(string)",
            serde_yaml::Value::Mapping(_) => "map(string)",
            serde_yaml::Value::Bool(_) => "bool",
            serde_yaml::Value::Number(_) => "number",
            _ => "string",
        };
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
