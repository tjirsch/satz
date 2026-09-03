//! Block builders shared by the walk transpiler and the stage-B emitter
//! (docs/stage-b.md). Parity by construction: both pipelines call THESE
//! functions, so a block's shape is defined exactly once. Extracted verbatim
//! from the walk; the corpus snapshots gate the extraction.


/// Terraform label for an IAM member binding: hash of member + role (+ condition
/// debug rendering when present). `DefaultHasher::new()` has a fixed key, so the
/// label is stable across runs — and across pipelines.
pub(crate) fn iam_member_label(
    member: &str,
    role: &str,
    condition: Option<&serde_yaml::Value>,
    scope: &str,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    member.hash(&mut hasher);
    role.hash(&mut hasher);
    if let Some(cv) = condition {
        format!("{:?}", cv).hash(&mut hasher);
    }
    // Only an explicitly pinned scope enters the hash. Grants scoped by the
    // organisation or by their structural node pass "" and keep the label they
    // have always had: it is the Terraform resource name, and renaming an IAM
    // binding is a destroy-and-create on every estate.
    if !scope.is_empty() {
        scope.hash(&mut hasher);
    }
    // Strip interpolation syntax from the label: "${google_project.x.number}"
    // would otherwise leak braces and dollars into the resource name.
    let label_src = member.replace("${", "").replace('}', "");
    format!("iam_{}_{:x}", label_src.replace(&['@', '.', ':', '-'][..], "_"), hasher.finish())
}

/// The IAM member resource block. `member_expr` arrives prebuilt (members may
/// carry `${…}` references — the caller routes them through its template
/// helper); `condition_block` likewise when present.
#[allow(clippy::too_many_arguments)]
pub(crate) fn iam_member_block(
    resource_type: &str,
    label: &str,
    role: &str,
    member_expr: hcl::Expression,
    id_attribute: &str,
    parent_expr: hcl::Expression,
    condition_block: Option<hcl::Block>,
    provider_alias: Option<&str>,
) -> hcl::Block {
    let mut rb = hcl::Block::builder("resource")
        .add_label(resource_type)
        .add_label(label)
        .add_attribute(("role", string_to_hcl_expr(role)))
        .add_attribute(("member", member_expr))
        .add_attribute((id_attribute, parent_expr));
    if let Some(cond) = condition_block {
        rb = rb.add_block(cond);
    }
    if let Some(alias) = provider_alias {
        if let Ok(expr) = alias.parse::<hcl::Expression>() {
            rb = rb.add_attribute(("provider", expr));
        }
    }
    rb.build()
}

/// The Cloud Identity group resource block, defaults included: group_key from
/// id/email/domain, `parent = customers/<id>`, the two standard labels,
/// display_name falling back to the YAML key, `initial_group_config = EMPTY`.
pub(crate) fn group_block(
    group_name: &str,
    attrs: &serde_yaml::Mapping,
    customer_id: &str,
    customer_domain: &str,
    provider_alias: Option<&str>,
    lifecycle_block: Option<hcl::Block>,
) -> hcl::Block {
    let resource_name = group_resource_label(group_name);
    let mut builder =
        hcl::Block::builder("resource").add_label("google_cloud_identity_group").add_label(resource_name);

    if let Some(alias) = provider_alias {
        if let Ok(expr) = alias.parse::<hcl::Expression>() {
            builder = builder.add_attribute(("provider", expr));
        }
    }

    let email = group_email(group_name, attrs, customer_domain);
    builder = builder
        .add_block(hcl::Block::builder("group_key").add_attribute(("id", email)).build())
        .add_attribute(("parent", format!("customers/{}", customer_id)));

    let mut labels = hcl::Map::new();
    labels.insert("cloudidentity.googleapis.com/groups.discussion_forum".to_string(), hcl::Value::from(""));
    labels.insert("cloudidentity.googleapis.com/groups.security".to_string(), hcl::Value::from(""));
    builder = builder.add_attribute(("labels", hcl::Value::from(labels)));

    if let Some(dn) =
        attrs.get(serde_yaml::Value::String("display_name".to_string())).and_then(|v| v.as_str())
    {
        builder = builder.add_attribute(("display_name", dn.to_owned()));
    } else {
        builder = builder.add_attribute(("display_name", group_name.to_owned()));
    }
    if let Some(desc) =
        attrs.get(serde_yaml::Value::String("description".to_string())).and_then(|v| v.as_str())
    {
        builder = builder.add_attribute(("description", desc.to_owned()));
    }
    let igc = attrs
        .get(serde_yaml::Value::String("initial_group_config".to_string()))
        .and_then(|v| v.as_str())
        .unwrap_or("EMPTY");
    builder = builder.add_attribute(("initial_group_config", igc.to_owned()));

    if let Some(lc) = lifecycle_block {
        builder = builder.add_block(lc);
    }
    builder.build()
}

/// A group's `lifecycle` block, always ignoring `initial_group_config`: it is a
/// create-only attribute the live group does not report, so without this an
/// ADOPTED group plans as "must be replaced" — destroyed and recreated, with
/// its memberships (live-run F15). Merged with whatever lifecycle the block
/// declares; `ignore_changes = all` is left alone.
pub(crate) fn group_lifecycle(attrs: &serde_yaml::Mapping) -> Option<hcl::Block> {
    let mut lc = match attrs.get(serde_yaml::Value::String("lifecycle".into())) {
        Some(serde_yaml::Value::Mapping(m)) => m.clone(),
        _ => serde_yaml::Mapping::new(),
    };
    let key = serde_yaml::Value::String("ignore_changes".into());
    let igc = serde_yaml::Value::String("initial_group_config".into());
    match lc.get_mut(&key) {
        Some(serde_yaml::Value::Sequence(seq)) => {
            if !seq.contains(&igc) {
                seq.push(igc);
            }
        }
        Some(serde_yaml::Value::String(_)) => {}
        _ => {
            lc.insert(key, serde_yaml::Value::Sequence(vec![igc]));
        }
    }
    lifecycle_block(&serde_yaml::Value::Mapping(lc), &|_| None)
}

/// The folder resource block: display_name, parent, provider, optional labels
/// and lifecycle.
pub(crate) fn folder_block(
    resource_name: &str,
    display_name: &str,
    parent_expr: hcl::Expression,
    provider_alias: Option<&str>,
    labels: Option<&serde_yaml::Value>,
    lifecycle_block: Option<hcl::Block>,
    extra_attrs: &[(String, serde_yaml::Value)],
) -> hcl::Block {
    let mut builder = hcl::Block::builder("resource")
        .add_label("google_folder")
        .add_label(resource_name)
        .add_attribute(("display_name", display_name.to_owned()))
        .add_attribute(hcl::Attribute::new("parent", parent_expr));
    // Every other attribute the folder declares (`deletion_protection`, `tags`,
    // …), rendered like any resource's — the fixed set above was defect #33.
    for (k, v) in extra_attrs {
        if let Some(expr) = render_value(v, &|_| None) {
            builder = builder.add_attribute(hcl::Attribute::new(k.as_str(), expr));
        }
    }
    if let Some(alias) = provider_alias {
        if let Ok(expr) = alias.parse::<hcl::Expression>() {
            builder = builder.add_attribute(("provider", expr));
        }
    }
    if let Some(serde_yaml::Value::Mapping(labels_map)) = labels {
        let mut sorted: Vec<_> =
            labels_map.iter().filter_map(|(k, v)| k.as_str().zip(v.as_str())).collect();
        sorted.sort_by_key(|(k, _)| *k);
        if !sorted.is_empty() {
            let map: hcl::Map<String, hcl::Value> =
                sorted.into_iter().map(|(k, v)| (k.to_string(), hcl::Value::from(v.to_string()))).collect();
            builder = builder.add_attribute(("labels", hcl::Value::from(map)));
        }
    }
    if let Some(lc) = lifecycle_block {
        builder = builder.add_block(lc);
    }
    builder.build()
}

/// Anchor/tag resolver hook: the walk resolves YAML anchors and custom tags,
/// pipeline B's bodies arrive pre-resolved (`|_| None`).
pub(crate) type ValueResolver<'a> = &'a dyn Fn(&serde_yaml::Value) -> Option<serde_yaml::Value>;

/// YAML value → HCL expression. Extracted verbatim from the walk's
/// `yaml_to_hcl_value`; both pipelines call THIS.
pub(crate) fn render_value(v: &serde_yaml::Value, resolve: ValueResolver) -> Option<hcl::Expression> {
    if let Some(resolved) = resolve(v) {
        return render_value(&resolved, resolve);
    }
    match v {
        serde_yaml::Value::Tagged(tagged) if tagged.tag == "!expr" => {
            if let serde_yaml::Value::String(s) = &tagged.value {
                s.parse::<hcl::Expression>().ok()
            } else {
                None
            }
        }
        serde_yaml::Value::String(s) => Some(string_to_hcl_expr(s)),
        serde_yaml::Value::Bool(b) => Some(hcl::Expression::from(*b)),
        serde_yaml::Value::Number(n) => {
            if n.is_i64() {
                Some(hcl::Expression::from(n.as_i64().unwrap()))
            } else if n.is_f64() {
                Some(hcl::Expression::from(n.as_f64().unwrap()))
            } else {
                None
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            let exprs: Vec<hcl::Expression> = seq.iter().filter_map(|v| render_value(v, resolve)).collect();
            Some(hcl::Expression::Array(exprs))
        }
        serde_yaml::Value::Mapping(map) => {
            let mut hcl_obj = hcl::Object::new();
            for (mk, mv) in map {
                if let serde_yaml::Value::String(mks) = mk {
                    if let Some(mve) = render_value(mv, resolve) {
                        hcl_obj.insert(hcl::ObjectKey::from(mks.clone()), mve);
                    }
                }
            }
            Some(hcl::Expression::Object(hcl_obj))
        }
        _ => None,
    }
}

/// YAML mapping → HCL block, schema-aware nesting with the walk's exact
/// no-schema heuristic. Extracted verbatim from `yaml_to_hcl_block`.
pub(crate) fn render_block(
    name: &str,
    v: &serde_yaml::Value,
    schema: Option<&crate::schema::BlockSchema>,
    resolve: ValueResolver,
) -> Option<hcl::Block> {
    if let serde_yaml::Value::Mapping(map) = v {
        let mut builder = hcl::Block::builder(name);
        for (bk, bv) in map {
            if let serde_yaml::Value::String(bks) = bk {
                let is_nested_block = if let Some(s) = schema {
                    s.block_types.contains_key(bks)
                } else {
                    // Heuristic if no schema
                    matches!(bv, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_))
                        && !matches!(bks.as_str(), "labels" | "metadata" | "annotations")
                };

                if is_nested_block {
                    let nested_schema = schema.and_then(|s| s.block_types.get(bks).map(|bts| &bts.block));
                    if let serde_yaml::Value::Sequence(seq) = bv {
                        for item in seq {
                            if let Some(nb) = render_block(bks, item, nested_schema, resolve) {
                                builder = builder.add_block(nb);
                            }
                        }
                    } else if let Some(nb) = render_block(bks, bv, nested_schema, resolve) {
                        builder = builder.add_block(nb);
                    }
                } else if let Some(val) = render_value(bv, resolve) {
                    builder = builder.add_attribute((bks.as_str(), val));
                }
            }
        }
        Some(builder.build())
    } else {
        None
    }
}

/// Bare traversal expression (`google_project.x.project_id`) — the walk builds
/// these via `parse_hcl_expr`'s dotted-path branch; same construction here.
pub(crate) fn traversal_expr(path: &str) -> hcl::Expression {
    let parts: Vec<&str> = path.split('.').collect();
    if let Ok(var) = hcl::Variable::new(parts[0]) {
        let mut operators = Vec::new();
        for part in &parts[1..] {
            if let Ok(ident) = hcl::Identifier::new(*part) {
                operators.push(hcl::TraversalOperator::GetAttr(ident));
            }
        }
        hcl::Expression::from(hcl::Traversal { expr: hcl::Expression::Variable(var), operators })
    } else {
        hcl::Expression::from(path.to_string())
    }
}

/// One `google_project_service` block: project reference, service, optional
/// extra attrs (disable_on_destroy, …), provider.
pub(crate) fn project_service_block(
    label: &str,
    project_expr: hcl::Expression,
    service: &str,
    service_attrs: Option<&serde_yaml::Mapping>,
    provider_alias: Option<&str>,
    resolve: ValueResolver,
) -> hcl::Block {
    let mut service_builder = hcl::Block::builder("resource")
        .add_label("google_project_service")
        .add_label(label)
        .add_attribute(hcl::Attribute::new("project", project_expr))
        .add_attribute(("service", service.to_owned()));
    if let Some(alias) = provider_alias {
        if let Ok(expr) = alias.parse::<hcl::Expression>() {
            service_builder = service_builder.add_attribute(("provider", expr));
        }
    }
    if let Some(attrs) = service_attrs {
        for (k, v) in attrs {
            if let (serde_yaml::Value::String(k_str), Some(hcl_v)) = (k, render_value(v, resolve)) {
                if k_str == "service" || k_str == "project" || k_str == "import-id" {
                    continue;
                }
                service_builder = service_builder.add_attribute(hcl::Attribute::new(k_str.clone(), hcl_v));
            }
        }
    }
    service_builder.build()
}

/// The narrowest-context facts the generic emission inherits from. Field names
/// mirror the walk's ResourceContext so the extracted code reads unchanged.
#[derive(Default, Clone)]
pub(crate) struct ResCtx {
    pub org_id: Option<String>,
    pub org_ref: Option<String>,
    pub folder_id: Option<String>,
    pub folder_ref: Option<String>,
    pub project_id: Option<String>,
    pub project_ref: Option<String>,
}

/// `parse_hcl_expr` as a free function: interpolations are templates, dotted
/// paths are traversals, everything else a quoted string. Extracted verbatim.
pub(crate) fn parse_expr(s: &str) -> hcl::Expression {
    if s.contains("${") || s.contains("%{") {
        return string_to_hcl_expr(s);
    }
    if s.contains('.') && !s.contains('/') && !s.contains(':') {
        return traversal_expr(s);
    }
    hcl::Expression::from(s.to_string())
}

/// Provider *reference* (`google-beta`, `google.google-beta`) as an unquoted
/// identifier/traversal; quoted-string fallback for non-identifier segments.
pub(crate) fn provider_ref_expr(s: &str) -> hcl::Expression {
    let mut parts = s.split('.');
    let first = parts.next().unwrap_or_default();
    let Ok(var) = hcl::Variable::new(first) else {
        return hcl::Expression::from(s.to_string());
    };
    let mut operators = Vec::new();
    for part in parts {
        let Ok(ident) = hcl::Identifier::new(part) else {
            return hcl::Expression::from(s.to_string());
        };
        operators.push(hcl::TraversalOperator::GetAttr(ident));
    }
    if operators.is_empty() {
        hcl::Expression::Variable(var)
    } else {
        hcl::Expression::from(hcl::Traversal { expr: hcl::Expression::Variable(var), operators })
    }
}

/// Argument-order shims so the extracted code keeps its original call shape.
fn render_value_r(resolve: ValueResolver, v: &serde_yaml::Value) -> Option<hcl::Expression> {
    render_value(v, resolve)
}
fn render_block_r(
    resolve: ValueResolver,
    name: &str,
    v: &serde_yaml::Value,
    schema: Option<&crate::schema::BlockSchema>,
) -> Option<hcl::Block> {
    render_block(name, v, schema, resolve)
}

/// Terraform `lifecycle` meta-argument block (bare identifiers for
/// ignore_changes/replace_triggered_by). Extracted verbatim from the walk.
pub(crate) fn lifecycle_block(v: &serde_yaml::Value, resolve: ValueResolver) -> Option<hcl::Block> {
    let serde_yaml::Value::Mapping(lc_map) = v else { return None; };
    let mut lc_builder = hcl::Block::builder("lifecycle");
    for (lk, lv) in lc_map {
        if let serde_yaml::Value::String(lks) = lk {
            match lks.as_str() {
                "ignore_changes" | "replace_triggered_by" => {
                    match lv {
                        serde_yaml::Value::Sequence(seq) => {
                            let exprs: Vec<hcl::Expression> = seq
                                .iter()
                                .filter_map(|item| item.as_str())
                                .filter_map(|s| s.parse::<hcl::Expression>().ok())
                                .collect();
                            lc_builder = lc_builder.add_attribute((lks.as_str(), hcl::Expression::Array(exprs)));
                        }
                        // Scalar form, e.g. `ignore_changes: all` -> bare keyword.
                        serde_yaml::Value::String(s) => {
                            if let Ok(expr) = s.parse::<hcl::Expression>() {
                                lc_builder = lc_builder.add_attribute((lks.as_str(), expr));
                            }
                        }
                        _ => {}
                    }
                }
                // create_before_destroy, prevent_destroy, etc.
                _ => {
                    if let Some(val) = render_value(lv, resolve) {
                        lc_builder = lc_builder.add_attribute((lks.as_str(), val));
                    }
                }
            }
        }
    }
    Some(lc_builder.build())
}

/// The generic resource emission: context inheritance (schema-driven narrowest
/// scope), org-policy name/parent/spec handling, lifecycle, attr-vs-block by
/// schema. Extracted verbatim from the walk's `transpile_single_resource`; both
/// pipelines call THIS. Returns (block, import-id, label).
#[allow(clippy::too_many_arguments)]
pub(crate) fn single_resource_block(
tf_type: &str,
res_name: &str,
attrs: &serde_yaml::Mapping,
resource_schema: Option<&crate::schema::ResourceSchema>,
ctx: &ResCtx,
provider_alias: Option<&str>,
billing_fallback: Option<&serde_yaml::Value>,
resolve: ValueResolver,
validate: Option<&dyn Fn(&serde_yaml::Mapping)>,
) -> Result<(hcl::Block, Option<String>, String), Box<dyn std::error::Error>> {
    let label = res_name.replace("-", "_");
    let mut block_builder = hcl::Block::builder("resource").add_label(tf_type).add_label(&label);

    if let Some(alias) = provider_alias {
        if !attrs.contains_key(serde_yaml::Value::String("provider".to_string())) {
            if let Ok(expr) = (alias).parse::<hcl::Expression>() {
                block_builder = block_builder.add_attribute(hcl::Attribute::new("provider", expr));
            }
        }
    }

    // Inheritance and Context Logic
    let mut final_attrs = attrs.clone();

    let import_id = final_attrs.remove(serde_yaml::Value::String("import-id".to_string()))
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    // Removal of import-existing logic (as requested by user)
    final_attrs.remove(serde_yaml::Value::String("import-existing".to_string()));

    if tf_type == "google_project" {
        let has_org = attrs.contains_key(serde_yaml::Value::String("org_id".to_string())) ||
                      attrs.contains_key(serde_yaml::Value::String("org".to_string()));
        let has_folder = attrs.contains_key(serde_yaml::Value::String("folder_id".to_string()));

        if !has_folder && !has_org {
            if let Some(f_ref) = &ctx.folder_ref {
                block_builder = block_builder.add_attribute(hcl::Attribute::new("folder_id", parse_expr(f_ref)));
                final_attrs.insert(serde_yaml::Value::String("folder_id".to_string()), serde_yaml::Value::String(f_ref.clone()));
            } else if let Some(org_id) = &ctx.org_id {
                block_builder = block_builder.add_attribute(hcl::Attribute::new("org_id", org_id.clone()));
                final_attrs.insert(serde_yaml::Value::String("org_id".to_string()), serde_yaml::Value::String(org_id.clone()));
            }
        }

        // Inject billing_account if missing and variable exists
        if !attrs.contains_key(serde_yaml::Value::String("billing_account".to_string())) {
            if let Some(ba) = billing_fallback {
                if let Some(val) = render_value_r(resolve, ba) {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new("billing_account", val));
                }
            }
        }
    } else if tf_type == "google_org_policy_policy" {
        let name_val = attrs.get(serde_yaml::Value::String("name".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!(
                "google_org_policy_policy '{}' is missing a string 'name' attribute (it is mandatory)",
                res_name
            ))?;

        let has_parent = attrs.contains_key(serde_yaml::Value::String("parent".to_string()));
        let (resolved_parent_expr, resolved_parent_str) = if has_parent {
            let v = attrs.get(serde_yaml::Value::String("parent".to_string())).unwrap();
            (render_value_r(resolve, v), v.as_str().map(|s| s.to_string()))
        } else if let Some(p_ref) = ctx.project_ref.as_ref().or(ctx.folder_ref.as_ref()).or(ctx.org_ref.as_ref()) {
            (Some(parse_expr(p_ref)), Some(p_ref.clone()))
        } else {
            let org_id = ctx.org_id.as_ref().ok_or_else(|| format!(
                "google_org_policy_policy '{}' has no 'parent' and no enclosing organization, folder or project to inherit one from",
                res_name
            ))?;
            (Some(hcl::Expression::from(format!("organizations/{}", org_id))), Some(format!("organizations/{}", org_id)))
        };

        // Calculate final name
        let final_name = if !name_val.contains('/') {
            match &resolved_parent_expr {
                Some(hcl::Expression::String(p_str)) => {
                    hcl::Expression::from(format!("{}/policies/{}", p_str, name_val))
                }
                Some(hcl::Expression::Traversal(_)) => {
                     if let Some(p_str) = resolved_parent_str {
                         // Interpolation, not a literal: without the helper hcl-rs
                         // would escape this to "$${...}/policies/...".
                         string_to_hcl_expr(&format!("${{{}}}/policies/{}", p_str, name_val))
                     } else {
                         string_to_hcl_expr(name_val)
                     }
                }
                _ => string_to_hcl_expr(name_val),
            }
        } else {
            string_to_hcl_expr(name_val)
        };

        block_builder = block_builder.add_attribute(("name", final_name));
        if let Some(p) = &resolved_parent_expr {
            block_builder = block_builder.add_attribute(("parent", p.clone()));
        }
    } else if let Some(schema) = resource_schema {
        // Narrowest Context Inheritance
        let project_params = ["project", "project_id"];
        let folder_params = ["folder", "folder_id"];
        let org_params = ["org_id", "organization"];

        let mut context_set = false;

        // 1. Try Project Context (Narrowest)
        if let Some(p_ref) = &ctx.project_ref {
            for p in project_params {
                if schema.block.attributes.contains_key(p) && !attrs.contains_key(serde_yaml::Value::String(p.to_string())) {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new(p, parse_expr(p_ref)));
                    final_attrs.insert(serde_yaml::Value::String(p.to_string()), serde_yaml::Value::String(p_ref.clone()));
                    context_set = true;
                    break;
                }
            }
        } else if let Some(p_id) = &ctx.project_id {
            for p in project_params {
                if schema.block.attributes.contains_key(p) && !attrs.contains_key(serde_yaml::Value::String(p.to_string())) {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new(p, p_id.clone()));
                    final_attrs.insert(serde_yaml::Value::String(p.to_string()), serde_yaml::Value::String(p_id.clone()));
                    context_set = true;
                    break;
                }
            }
        }

        // 2. Try Folder Context
        if !context_set {
            if let Some(f_ref) = &ctx.folder_ref {
                for f in folder_params {
                    if schema.block.attributes.contains_key(f) && !attrs.contains_key(serde_yaml::Value::String(f.to_string())) {
                        block_builder = block_builder.add_attribute(hcl::Attribute::new(f, parse_expr(f_ref)));
                        final_attrs.insert(serde_yaml::Value::String(f.to_string()), serde_yaml::Value::String(f_ref.clone()));
                        context_set = true;
                        break;
                    }
                }
            } else if let Some(f_id) = &ctx.folder_id {
                for f in folder_params {
                    if schema.block.attributes.contains_key(f) && !attrs.contains_key(serde_yaml::Value::String(f.to_string())) {
                        block_builder = block_builder.add_attribute(hcl::Attribute::new(f, f_id.clone()));
                        final_attrs.insert(serde_yaml::Value::String(f.to_string()), serde_yaml::Value::String(f_id.clone()));
                        context_set = true;
                        break;
                    }
                }
            }
        }

        // 3. Try Org Context
        if !context_set {
            if let Some(o_id) = &ctx.org_id {
                for o in org_params {
                    if schema.block.attributes.contains_key(o) && !attrs.contains_key(serde_yaml::Value::String(o.to_string())) {
                        block_builder = block_builder.add_attribute(hcl::Attribute::new(o, o_id.clone()));
                        final_attrs.insert(serde_yaml::Value::String(o.to_string()), serde_yaml::Value::String(o_id.clone()));
                        context_set = true;
                        break;
                    }
                }
            }
        }

        // Warning for missing required project/folder context if not set explicitly
        if !context_set {
            let needs_project = project_params.iter().any(|p| schema.block.attributes.contains_key(*p) && !attrs.contains_key(serde_yaml::Value::String(p.to_string())));
            let needs_folder = folder_params.iter().any(|f| schema.block.attributes.contains_key(*f) && !attrs.contains_key(serde_yaml::Value::String(f.to_string())));

            if needs_project {
                eprintln!("Warning: Resource '{}' ({}) requires a 'project' parameter but is defined outside a project context and no explicit project is provided.", res_name, tf_type);
            } else if needs_folder {
                eprintln!("Warning: Resource '{}' ({}) requires a 'folder' parameter but is defined outside a folder context and no explicit folder is provided.", res_name, tf_type);
            }
        }
    }

    for (k, v) in &final_attrs {
        if let serde_yaml::Value::String(k_str) = k {
            // Skip fields that were handled specially, but only if they were auto-injected
            // If they were explicitly provided in the YAML, we should process them
            let was_explicitly_provided = attrs.contains_key(k);
            let should_skip = if tf_type == "google_org_policy_policy" && (k_str == "name" || k_str == "constraint" || k_str == "parent") {
                true
            } else if ["project", "project_id", "folder", "folder_id", "org_id", "organization", "import-id", "import-existing"].contains(&k_str.as_str()) {
                // Only skip if it was auto-injected, not if explicitly provided
                !was_explicitly_provided
            } else {
                false
            };
            
            if should_skip {
                continue;
            }

            // Special handling for parameterized constraints in google_org_policy_policy
            // Supports both `spec` and `dry_run_spec` blocks with identical structure.
            if tf_type == "google_org_policy_policy" && (k_str == "spec" || k_str == "dry_run_spec") {
                if let serde_yaml::Value::Mapping(spec_map) = v {
                    if let Some(serde_yaml::Value::Sequence(rules_seq)) = spec_map.get(serde_yaml::Value::String("rules".to_string())) {
                        let mut spec_builder = hcl::Block::builder(k_str.as_str());

                        // Copy other spec fields
                        for (sk, sv) in spec_map {
                            if let serde_yaml::Value::String(sks) = sk {
                                if sks != "rules" {
                                    if let Some(val) = render_value_r(resolve, sv) {
                                        spec_builder = spec_builder.add_attribute((sks.as_str(), val));
                                    }
                                }
                            }
                        }

                        for rule in rules_seq {
                            if let serde_yaml::Value::Mapping(rule_map) = rule {
                                let mut rule_builder = hcl::Block::builder("rules");
                                for (rk, rv) in rule_map {
                                    if let serde_yaml::Value::String(rks) = rk {
                                        if rks == "parameters" {
                                            // Parameters must be a JSON string. If the user provided
                                            // a structured YAML value, JSON-encode it. If it's already
                                            // a string, pass it through unchanged.
                                            match rv {
                                                serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => {
                                                    if let Ok(json_str) = serde_json::to_string(rv) {
                                                        rule_builder = rule_builder.add_attribute((
                                                            "parameters",
                                                            hcl::Value::from(json_str),
                                                        ));
                                                    }
                                                }
                                                _ => {
                                                    if let Some(val) = render_value_r(resolve, rv) {
                                                        rule_builder = rule_builder.add_attribute((
                                                            "parameters",
                                                            val,
                                                        ));
                                                    }
                                                }
                                            }
                                        } else if rks == "values" {
                                            // `values` is a nested block whose fields (like
                                            // `allowed_values`, `denied_values`) must be
                                            // attributes, not nested blocks.
                                            if let serde_yaml::Value::Mapping(vmap) = rv {
                                                let mut values_builder = hcl::Block::builder("values");
                                                for (vk, vv) in vmap {
                                                    if let serde_yaml::Value::String(vks) = vk {
                                                        if let Some(val) = render_value_r(resolve, vv) {
                                                            values_builder = values_builder.add_attribute((
                                                                vks.as_str(),
                                                                val,
                                                            ));
                                                        }
                                                    }
                                                }
                                                rule_builder = rule_builder.add_block(values_builder.build());
                                            }
                                        } else if rks == "condition" {
                                            // `condition` remains a nested block with simple attributes.
                                            if let Some(blk) = render_block_r(resolve, rks, rv, None) {
                                                rule_builder = rule_builder.add_block(blk);
                                            }
                                        } else if let Some(val) = render_value_r(resolve, rv) {
                                            // Simple attributes like "enforce"
                                            rule_builder = rule_builder.add_attribute((rks.as_str(), val));
                                        } else if let Some(blk) = render_block_r(resolve, rks, rv, None) {
                                            rule_builder = rule_builder.add_block(blk);
                                        }
                                    }
                                }
                                spec_builder = spec_builder.add_block(rule_builder.build());
                            }
                        }
                        block_builder = block_builder.add_block(spec_builder.build());
                        continue; // Skip standard processing for spec
                    }
                }
            }

            // Special handling for the Terraform `lifecycle` meta-argument block.
            if k_str == "lifecycle" {
                if let Some(lc_block) = lifecycle_block(v, resolve) {
                    block_builder = block_builder.add_block(lc_block);
                    continue; // Skip standard processing for lifecycle
                }
            }

            let is_block = if let Some(schema) = resource_schema {
                schema.block.block_types.contains_key(k_str)
            } else {
                matches!(v, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)) && !matches!(k_str.as_str(), "labels" | "metadata" | "annotations")
            };

            if is_block {
                let nested_schema = resource_schema.and_then(|s| s.block.block_types.get(k_str).map(|bts| &bts.block));
                if let serde_yaml::Value::Sequence(seq) = v {
                    for item in seq {
                        if let Some(block) = render_block_r(resolve, k_str, item, nested_schema) {
                            block_builder = block_builder.add_block(block);
                        }
                    }
                } else if let Some(block) = render_block_r(resolve, k_str, v, nested_schema) {
                    block_builder = block_builder.add_block(block);
                }
            } else if k_str == "provider" {
                // A user-specified provider must render as a reference
                // (provider = google-beta), not a quoted string — the quoted form
                // is the pre-0.12 legacy syntax tofu warns about on every plan.
                if let serde_yaml::Value::String(p) = v {
                    block_builder = block_builder
                        .add_attribute(hcl::Attribute::new("provider", provider_ref_expr(p)));
                } else if let Some(val) = render_value_r(resolve, v) {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new("provider", val));
                }
            } else if let Some(val) = render_value_r(resolve, v) {
                block_builder = block_builder.add_attribute(hcl::Attribute::new(k_str.as_str(), val));
            }
        }
    }

    if let Some(validate) = validate {
        validate(&final_attrs);
    }

    Ok((block_builder.build(), import_id, label))
}


/// The membership resources of one group: aggregated per member, sorted roles,
/// preferred_member_key + roles blocks. Extracted verbatim from the walk.
pub(crate) fn membership_blocks(
    group_name: &str,
    attrs: &serde_yaml::Mapping,
    provider_alias: Option<&str>,
) -> Vec<hcl::Block> {
    let resource_name = group_resource_label(group_name);
    let group_ref = format!("google_cloud_identity_group.{}.id", resource_name);
    let mut out = Vec::new();
    for (member_raw, roles_set) in aggregate_group_members(attrs) {
        let email = member_email(&member_raw);
        let membership_label = membership_resource_label(group_name, &member_raw);
        let mut mb = hcl::Block::builder("resource")
            .add_label("google_cloud_identity_group_membership")
            .add_label(&membership_label)
            .add_attribute(hcl::Attribute::new("group", parse_expr(&group_ref)))
            .add_block(
                hcl::Block::builder("preferred_member_key")
                    .add_attribute(("id", email.to_owned()))
                    .build(),
            );
        let mut sorted_roles: Vec<_> = roles_set.into_iter().collect();
        sorted_roles.sort();
        for role in sorted_roles {
            mb = mb.add_block(hcl::Block::builder("roles").add_attribute(("name", role)).build());
        }
        if let Some(alias) = provider_alias {
            if let Ok(expr) = alias.parse::<hcl::Expression>() {
                mb = mb.add_attribute(("provider", expr));
            }
        }
        out.push(mb.build());
    }
    out
}

/// Google-provider injections: central billing project, user_project_override,
/// cloud-mode impersonation, and the default region. Extracted verbatim from
/// the walk's configure_google_provider.
pub(crate) struct GoogleProviderDeps {
    pub infra_project: Option<String>,
    /// Precomputed impersonation SA email — Some only in cloud mode.
    pub impersonate: Option<String>,
}

pub(crate) fn configure_google_provider(
    mut builder: hcl::BlockBuilder,
    project_id: Option<String>,
    has_billing_project: bool,
    has_user_project_override: bool,
    deps: &GoogleProviderDeps,
) -> hcl::BlockBuilder {
    let infra_project = deps.infra_project.as_deref();
    if let Some(pid) = project_id {
        if !has_billing_project {
            let billing_pid = infra_project.unwrap_or(&pid);
            builder = builder.add_attribute(("billing_project", billing_pid.to_string()));
        }
        if !has_user_project_override {
            builder = builder.add_attribute(("user_project_override", true));
        }
    } else if let Some(infra_pid) = infra_project {
        if !has_billing_project {
            builder = builder.add_attribute(("billing_project", infra_pid.to_string()));
        }
        if !has_user_project_override {
            builder = builder.add_attribute(("user_project_override", true));
        }
    }
    if let Some(sa_email) = &deps.impersonate {
        builder = builder.add_attribute(("impersonate_service_account", sa_email.clone()));
    }
    builder
}

/// One root provider block from a config mapping (the walk's provider loop body).
pub(crate) fn provider_block_from_map(
    p_name: &str,
    map: &serde_yaml::Mapping,
    deps: &GoogleProviderDeps,
    resolve: ValueResolver,
) -> hcl::Block {
    let mut builder = hcl::Block::builder("provider").add_label(p_name);
    let mut has_alias = false;
    let mut project_id = None;
    let mut has_billing_project = false;
    let mut has_user_project_override = false;
    for (k, v) in map {
        if let serde_yaml::Value::String(k_str) = k {
            if k_str == "alias" { has_alias = true; }
            if k_str == "project" { project_id = v.as_str().map(|s| s.to_string()); }
            if k_str == "billing_project" { has_billing_project = true; }
            if k_str == "user_project_override" { has_user_project_override = true; }
            if let Some(val) = render_value(v, resolve) {
                builder = builder.add_attribute((k_str.as_str(), val));
            }
        }
    }
    if !has_alias {
        builder = builder.add_attribute(("alias", p_name));
    }
    if p_name == "google" || p_name == "google-beta" {
        builder = configure_google_provider(builder, project_id, has_billing_project, has_user_project_override, deps);
    }
    builder.build()
}

/// The per-project provider alias block (walk: transpile_google_project).
pub(crate) fn project_provider_block(
    project_key: &str,
    project_id: &str,
    deps: &GoogleProviderDeps,
) -> hcl::Block {
    let p_alias = format!("project_{}", project_key.replace('-', "_"));
    let builder = hcl::Block::builder("provider")
        .add_label("google")
        .add_attribute(("alias", p_alias))
        .add_attribute(("project", project_id.to_string()));
    let builder = configure_google_provider(builder, Some(project_id.to_string()), false, false, deps);
    builder.add_attribute(("region", "europe-west3")).build()
}

// ---------------------------------------------------------------------------
// Cloud Identity group naming + HCL string helpers (moved from the retired walk)
//
// Shared with the adopter, which has to address exactly the resources the
// emitter emits. Keeping the derivation in one place is what stops it from
// computing a group email or a Terraform address the generated HCL does not use.
// ---------------------------------------------------------------------------

/// Terraform label for a group's YAML key (`-` is not legal in an identifier).
pub(crate) fn group_resource_label(yaml_key: &str) -> String {
    yaml_key.replace('-', "_")
}

/// Full Terraform address of the `google_cloud_identity_group` emitted for `yaml_key`.
pub(crate) fn group_resource_address(yaml_key: &str) -> String {
    format!("google_cloud_identity_group.{}", group_resource_label(yaml_key))
}

/// The group's `group_key.id`: an explicit `id`, else an explicit `email`, else the YAML
/// key at the customer's primary domain.
pub(crate) fn group_email(
    yaml_key: &str,
    attrs: &serde_yaml::Mapping,
    customer_domain: &str,
) -> String {
    attrs
        .get(serde_yaml::Value::String("id".to_string()))
        .or_else(|| attrs.get(serde_yaml::Value::String("email".to_string())))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}@{}", yaml_key, customer_domain))
}

/// The YAML keys that contribute members, and the roles each implies.
pub(crate) const GROUP_ROLE_KEYS: [(&str, &[&str]); 3] = [
    ("member", &["MEMBER"]),
    ("manager", &["MEMBER", "MANAGER"]),
    ("owner", &["MEMBER", "OWNER"]),
];

/// One entry of a group's `member` / `manager` / `owner` list: the raw member
/// string (`user:a@example.com`), or an object `{ id = "user:a@…",
/// "import-id" = "groups/<g>/memberships/<m>" }` when the membership is adopted.
fn group_member_entry(v: &serde_yaml::Value) -> Option<(String, Option<String>)> {
    match v {
        serde_yaml::Value::String(s) => Some((s.clone(), None)),
        serde_yaml::Value::Mapping(m) => {
            let raw = m.get(serde_yaml::Value::String("id".into()))?.as_str()?.to_string();
            let import_id = m
                .get(serde_yaml::Value::String("import-id".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some((raw, import_id))
        }
        _ => None,
    }
}

/// Members a group declares, as `raw YAML string -> roles`. Keyed by the raw string
/// (`user:a@example.com`), matching how the emitted resource label is derived.
pub(crate) fn aggregate_group_members(
    attrs: &serde_yaml::Mapping,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut aggregated: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (key, roles) in GROUP_ROLE_KEYS {
        let Some(val) = attrs.get(serde_yaml::Value::String(key.to_string())) else {
            continue;
        };
        let members_vals = match val {
            serde_yaml::Value::Sequence(seq) => seq.clone(),
            serde_yaml::Value::String(s) => vec![serde_yaml::Value::String(s.clone())],
            _ => continue,
        };
        for member_val in members_vals {
            if let Some((member_raw, _)) = group_member_entry(&member_val) {
                let entry = aggregated.entry(member_raw).or_default();
                for role in roles {
                    entry.insert((*role).to_string());
                }
            }
        }
    }
    aggregated
}

/// The memberships a group adopts: `raw member string -> import id`, from the
/// object form of a member entry. Both pipelines emit one `import` block per
/// entry, addressed by `membership_resource_address`.
pub(crate) fn group_member_import_ids(
    attrs: &serde_yaml::Mapping,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (key, _) in GROUP_ROLE_KEYS {
        let Some(serde_yaml::Value::Sequence(seq)) = attrs.get(serde_yaml::Value::String(key.to_string())) else {
            continue;
        };
        for v in seq {
            if let Some((raw, Some(id))) = group_member_entry(v) {
                out.insert(raw, id);
            }
        }
    }
    out
}

/// Strip a `user:` / `group:` / `serviceAccount:` prefix to get the bare member email.
pub(crate) fn member_email(member_raw: &str) -> &str {
    match member_raw.find(':') {
        Some(idx) => &member_raw[idx + 1..],
        None => member_raw,
    }
}

/// Terraform label for a membership. Hashed because a member email is not a legal
/// identifier; `DefaultHasher::new()` has a fixed key, so this is stable across runs.
pub(crate) fn membership_resource_label(group_yaml_key: &str, member_raw: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    group_yaml_key.hash(&mut hasher);
    member_raw.hash(&mut hasher);
    format!(
        "membership_{}_{:x}",
        group_resource_label(group_yaml_key),
        hasher.finish()
    )
}

/// Full Terraform address of the membership emitted for `member_raw` in `group_yaml_key`.
pub(crate) fn membership_resource_address(group_yaml_key: &str, member_raw: &str) -> String {
    format!(
        "google_cloud_identity_group_membership.{}",
        membership_resource_label(group_yaml_key, member_raw)
    )
}

/// Escape a string for use as HCL quoted-template source (`TemplateExpr::QuotedString`),
/// which the hcl crate emits between quotes verbatim. Literal `"` and `\\` must arrive
/// pre-escaped or the output is invalid HCL (`filter = "metric.type="…""`). Content
/// inside `${…}`/`%{…}` is expression context where quotes are legal raw, so it is left
/// untouched; `$${`/`%%{` are the literal escape sequences, not interpolation starts.
fn escape_template_literals(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 8);
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if depth == 0
            && (c == '$' || c == '%')
            && i + 2 < b.len()
            && b[i + 1] == b[i]
            && b[i + 2] == b'{'
        {
            out.push(c);
            out.push(c);
            out.push('{');
            i += 3;
            continue;
        }
        if (c == '$' || c == '%') && i + 1 < b.len() && b[i + 1] == b'{' {
            depth += 1;
            out.push(c);
            out.push('{');
            i += 2;
            continue;
        }
        if c == '}' && depth > 0 {
            depth -= 1;
            out.push('}');
            i += 1;
            continue;
        }
        if depth == 0 && (c == '"' || c == '\\') {
            out.push('\\');
        }
        out.push(c);
        i += 1;
    }
    out
}

/// A user-supplied string as an HCL expression: a template when it carries
/// `${…}`/`%{…}` interpolation, else a plain string literal — serialising a
/// reference as a literal would escape `${` to `$${` and silently break it.
pub(crate) fn string_to_hcl_expr(s: &str) -> hcl::Expression {
    if s.contains("${") || s.contains("%{") {
        hcl::Expression::TemplateExpr(Box::new(hcl::TemplateExpr::QuotedString(escape_template_literals(s))))
    } else {
        hcl::Expression::from(s.to_string())
    }
}
