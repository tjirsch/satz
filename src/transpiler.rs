use std::collections::HashMap;
use crate::config::{Config, Folder, Project};
use crate::schema::ResourceRegistry;

#[derive(Debug)]
/// Converter-only since M3: `migrate-to-satz` proves a conversion by comparing
/// `main_tf` before and after, so the other outputs are produced but unread.
#[allow(dead_code)]
pub struct GeneratedProject {
    pub main_tf: String,
    pub providers_tf: String,
    pub variables_tf: String,
    pub tfvars: String,
    pub imports_tf: String,
}

pub struct Transpiler<'a> {
    config: &'a Config,
    registry: Option<ResourceRegistry>,
    auto_explode: Vec<String>,
    validation_level: String,
    variables: HashMap<String, serde_yaml::Value>,
    provider_sources: HashMap<String, String>,
    provider_versions: HashMap<String, String>,
    /// Org/customer-scoped declarations collected during the walk (see HoistedResources).
    /// RefCell because the walk methods take `&self`; drained at the end of `transpile`.
    hoisted: std::cell::RefCell<HoistedResources>,
}

// ---------------------------------------------------------------------------
// Cloud Identity group naming
//
// Shared with `crate::cloud_identity`, which has to address and look up exactly the
// resources this module emits. Keeping the derivation in one place is what stops the
// importer from computing a group email or a Terraform address that the generated HCL
// does not actually use.
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
            if let Some(member_raw) = member_val.as_str() {
                let entry = aggregated.entry(member_raw.to_string()).or_default();
                for role in roles {
                    entry.insert((*role).to_string());
                }
            }
        }
    }
    aggregated
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
#[allow(dead_code)]
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

/// The duplicate-address guard: every Terraform address may be emitted once.
///
/// Fragments include the same org-level resource from several positions (an audit
/// config, a sink, a contact — "highlander" resources GCP allows exactly once anyway).
/// Byte-identical duplicates collapse to one emission with a printed note — a repeated
/// definition is read as "this resource should exist". Same address with *different*
/// content has no right answer and aborts before anything is written. This is
/// deliberately shallow: attribute-level merging would only hand the same conflict one
/// recursion level down.
fn dedup_resource_blocks(blocks: Vec<hcl::Block>) -> Result<Vec<hcl::Block>, Box<dyn std::error::Error>> {
    use satz_core::algebra::{fold, Body, Entity, Fragment, Slot};
    fn render(b: &hcl::Block) -> String {
        hcl::to_string(&hcl::Body::builder().add_block(b.clone()).build()).unwrap_or_default()
    }

    // One fragment per emitted block; the fold's Entity lattice supplies the verdicts:
    // canonical-equal duplicates are idempotent (a law, not a check), different content
    // at one address is ⊥ carrying every origin.
    let mut fragments = Vec::with_capacity(blocks.len());
    let mut addrs: Vec<Option<satz_core::Address>> = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        if block.identifier.as_str() != "resource" || block.labels.len() < 2 {
            addrs.push(None);
            continue;
        }
        let addr = satz_core::Address {
            tf_type: block.labels[0].as_str().to_string(),
            label: block.labels[1].as_str().to_string(),
        };
        let mut f = Fragment::default();
        f.entities.insert(
            addr.clone(),
            Entity {
                node_path: Vec::new(),
                addr: addr.clone(),
                scope: satz_core::Scope::Node,
                body: Body::Attrs(serde_yaml::Value::String(render(block))),
                provenance: vec![satz_core::Span { file: format!("emitted block #{}", i), line: i as u32 }],
            },
        );
        fragments.push(f);
        addrs.push(Some(addr));
    }

    struct AllEntities;
    impl satz_core::algebra::TypeTable for AllEntities {
        fn merge_class(&self, _t: &str) -> satz_core::MergeClass {
            satz_core::MergeClass::Entity
        }
        fn scope(&self, _t: &str) -> satz_core::Scope {
            satz_core::Scope::Node
        }
    }
    let folded = fold(&AllEntities, &fragments);

    let mut errors = Vec::new();
    for c in folded.conflicts() {
        let texts: Vec<&str> = c
            .candidates
            .iter()
            .filter_map(|(b, _)| match b {
                Body::Attrs(serde_yaml::Value::String(t)) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let diff = match texts.as_slice() {
            [a, b, ..] => a
                .lines()
                .zip(b.lines())
                .find(|(x, y)| x != y)
                .map(|(x, y)| format!(" (first difference: `{}` vs `{}`)", x.trim(), y.trim()))
                .unwrap_or_default(),
            _ => String::new(),
        };
        errors.push(format!(
            "{}.{} is defined twice with different content{}",
            c.addr.tf_type, c.addr.label, diff
        ));
    }
    if !errors.is_empty() {
        return Err(format!("Conflicting duplicate resources:\n  - {}", errors.join("\n  - ")).into());
    }

    // Emission preserves first-occurrence order; later canonical-equal duplicates are
    // the idempotence law in action and collapse with a note.
    let mut seen: std::collections::HashSet<&satz_core::Address> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(blocks.len());
    for (block, addr) in blocks.iter().zip(addrs.iter()) {
        match addr {
            None => out.push(block.clone()),
            Some(a) => {
                debug_assert!(matches!(folded.slots.get(a), Some(Slot::Ok(_))));
                if seen.insert(a) {
                    out.push(block.clone());
                } else {
                    println!(
                        "note: {}.{} is defined more than once with identical content — emitted once",
                        a.tf_type, a.label
                    );
                }
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Default)]
struct ResourceContext {
    org_id: Option<String>,
    folder_id: Option<String>,
    project_id: Option<String>,
    org_ref: Option<String>,
    folder_ref: Option<String>,
    project_ref: Option<String>,
    provider_alias: Option<String>,
}

// ---------------------------------------------------------------------------
// Hoisted scopes
//
// Some resource types have one intrinsic scope no matter where they are written:
// `cloud_identity_group` is customer-scoped, `organization_iam_member` is org-scoped.
// Declaring them inside a folder or project block is therefore *grouping for humans*
// (keep a project's org-level companions in the project's file), not placement. During
// the tree walk they are collected here instead of emitted in place, then emitted once
// at their real scope — otherwise two fragments declaring the same group or the same
// grant would emit duplicate resources with identical labels, which is invalid HCL.
// ---------------------------------------------------------------------------

/// Collector for hoisted-scope contributions. Since the satz-core swap, this holds
/// raw per-site fragments and defers ALL merge semantics to the algebra's ⊕ fold at
/// drain time — grant union, deep-equal idempotence and conflict-as-⊥ are the
/// property-tested laws in satz_core::algebra, not bespoke logic here.
#[derive(Default)]
struct HoistedResources {
    fragments: Vec<satz_core::algebra::Fragment>,
}

/// Merge classes for the hoisted fold: IAM types are additive grants, everything
/// else (groups, the billing-account-id pseudo entity) is flat-lattice Entity.
struct HoistTable;
impl satz_core::algebra::TypeTable for HoistTable {
    fn merge_class(&self, tf_type: &str) -> satz_core::MergeClass {
        if tf_type.ends_with("iam_member") {
            satz_core::MergeClass::Grant
        } else {
            satz_core::MergeClass::Entity
        }
    }
    fn scope(&self, _tf_type: &str) -> satz_core::Scope {
        satz_core::Scope::Node
    }
}

/// Everything the drain hands back to the emitters, in the shapes they consume.
struct HoistedDrained {
    groups: std::collections::BTreeMap<String, serde_yaml::Value>,
    org_iam: std::collections::BTreeMap<String, Vec<serde_yaml::Value>>,
    billing_iam: std::collections::BTreeMap<String, Vec<serde_yaml::Value>>,
    billing_account_id: Option<String>,
}

/// Synthetic address for the at-most-one explicit billing account id.
const BILLING_ID_TYPE: &str = "__billing_account_id";

impl HoistedResources {
    fn one(&mut self, entity: satz_core::algebra::Entity) {
        let mut f = satz_core::algebra::Fragment::default();
        f.entities.insert(entity.addr.clone(), entity);
        self.fragments.push(f);
    }

    fn insert_group(&mut self, key: &str, body: &serde_yaml::Value, provenance: String) {
        self.one(satz_core::algebra::Entity {
            node_path: Vec::new(),
            addr: satz_core::Address { tf_type: "cloud_identity_group".into(), label: key.into() },
            scope: satz_core::Scope::Customer,
            body: satz_core::algebra::Body::Attrs(body.clone()),
            provenance: vec![satz_core::Span { file: provenance, line: 0 }],
        });
    }

    fn insert_org_iam(&mut self, member: &str, entries: &[serde_yaml::Value]) {
        self.insert_grant("google_organization_iam_member", member, entries);
    }

    fn insert_billing_iam(&mut self, member: &str, entries: &[serde_yaml::Value]) {
        self.insert_grant("google_billing_account_iam_member", member, entries);
    }

    /// Role entries ride the fold as canonicalized-YAML grant edges; the drain
    /// parses them back. Union + dedup are the Grant laws, not code here.
    fn insert_grant(&mut self, tf_type: &str, member: &str, entries: &[serde_yaml::Value]) {
        let edges = entries
            .iter()
            .map(|e| satz_core::algebra::GrantEdge {
                member: member.to_string(),
                role: serde_yaml::to_string(e).unwrap_or_default(),
                condition: String::new(),
            })
            .collect();
        self.one(satz_core::algebra::Entity {
            node_path: Vec::new(),
            addr: satz_core::Address { tf_type: tf_type.into(), label: member.into() },
            scope: satz_core::Scope::Org,
            body: satz_core::algebra::Body::Grant(edges),
            provenance: vec![satz_core::Span { file: "walk".into(), line: 0 }],
        });
    }

    fn set_billing_account_id(&mut self, id: &str, provenance: String) {
        self.one(satz_core::algebra::Entity {
            node_path: Vec::new(),
            addr: satz_core::Address { tf_type: BILLING_ID_TYPE.into(), label: String::new() },
            scope: satz_core::Scope::Billing,
            body: satz_core::algebra::Body::Attrs(serde_yaml::Value::String(id.into())),
            provenance: vec![satz_core::Span { file: provenance, line: 0 }],
        });
    }

    /// Fold all contributions. Conflicts become the user-facing errors this
    /// collector used to accumulate imperatively; Ok slots split back into the
    /// per-kind maps the emitters consume.
    fn drain(self) -> Result<HoistedDrained, Box<dyn std::error::Error>> {
        use satz_core::algebra::{fold, Body, Slot};
        let folded = fold(&HoistTable, &self.fragments);

        let mut errors = Vec::new();
        for c in folded.conflicts() {
            let provs: Vec<String> = c
                .candidates
                .iter()
                .filter_map(|(_, spans)| spans.first().map(|s| s.file.clone()))
                .collect();
            if c.addr.tf_type == BILLING_ID_TYPE {
                let vals: Vec<String> = c
                    .candidates
                    .iter()
                    .filter_map(|(b, _)| match b {
                        Body::Attrs(serde_yaml::Value::String(v)) => Some(v.clone()),
                        _ => None,
                    })
                    .collect();
                errors.push(format!(
                    "billing_account_id set to '{}' at {} but to '{}' at {}",
                    vals.first().cloned().unwrap_or_default(),
                    provs.first().cloned().unwrap_or_default(),
                    vals.get(1).cloned().unwrap_or_default(),
                    provs.get(1).cloned().unwrap_or_default(),
                ));
            } else {
                errors.push(format!(
                    "{} '{}' is defined differently at {} and at {}",
                    c.addr.tf_type,
                    c.addr.label,
                    provs.first().cloned().unwrap_or_default(),
                    provs.get(1).cloned().unwrap_or_default(),
                ));
            }
        }
        if !errors.is_empty() {
            return Err(format!("Conflicting hoisted definitions:\n  - {}", errors.join("\n  - ")).into());
        }

        let mut out = HoistedDrained {
            groups: Default::default(),
            org_iam: Default::default(),
            billing_iam: Default::default(),
            billing_account_id: None,
        };
        for (addr, slot) in folded.slots {
            let Slot::Ok(entity) = slot else { unreachable!("conflicts handled above") };
            match (addr.tf_type.as_str(), entity.body) {
                ("cloud_identity_group", Body::Attrs(v)) => {
                    out.groups.insert(addr.label, v);
                }
                (BILLING_ID_TYPE, Body::Attrs(serde_yaml::Value::String(v))) => {
                    out.billing_account_id = Some(v);
                }
                (t, Body::Grant(edges)) => {
                    // Parse role entries back; sort with the emitter's historical key
                    // so generated output stays byte-stable.
                    let mut entries: Vec<serde_yaml::Value> = edges
                        .iter()
                        .filter_map(|e| serde_yaml::from_str(&e.role).ok())
                        .collect();
                    entries.sort_by_key(|e| format!("{:?}", e));
                    let map = if t == "google_organization_iam_member" {
                        &mut out.org_iam
                    } else {
                        &mut out.billing_iam
                    };
                    map.insert(addr.label, entries);
                }
                other => unreachable!("unexpected hoisted slot {:?}", other.0),
            }
        }
        Ok(out)
    }
}

/// Where in the tree a hoisted declaration was found — only used in conflict messages.
fn hoist_provenance(ctx: &ResourceContext) -> String {
    // Refs are internal expressions like `google_folder.observability.name`; show the
    // label the user wrote, not the plumbing around it.
    fn trim_ref(r: &str) -> &str {
        r.strip_prefix("google_folder.")
            .or_else(|| r.strip_prefix("google_project."))
            .map(|rest| rest.split('.').next().unwrap_or(rest))
            .unwrap_or(r)
    }
    if let Some(p) = ctx.project_id.as_deref().or(ctx.project_ref.as_deref()) {
        format!("project '{}'", trim_ref(p))
    } else if let Some(f) = ctx.folder_id.as_deref().or(ctx.folder_ref.as_deref()) {
        format!("folder '{}'", trim_ref(f))
    } else {
        "the root config".to_string()
    }
}

impl<'a> Transpiler<'a> {
    pub fn new(
        config: &'a Config,
        registry: Option<ResourceRegistry>,
        auto_explode: Vec<String>,
        validation_level: String,
        variables: HashMap<String, serde_yaml::Value>,
        provider_sources: HashMap<String, String>,
        provider_versions: HashMap<String, String>,
    ) -> Self {
        Self {
            config,
            registry,
            auto_explode,
            validation_level,
            variables,
            provider_sources,
            provider_versions,
            hoisted: std::cell::RefCell::new(HoistedResources::default()),
        }
    }

    /// Turn a plain string into an HCL expression.
    ///
    /// A string containing `${...}` (or the directive form `%{...}`) is meant to be
    /// interpolated by Terraform, so it has to become a `TemplateExpr`. Using
    /// `Expression::from(String)` would yield `Expression::String`, which hcl-rs
    /// serialises as a *literal* — it escapes `${` to `$${` and the reference silently
    /// stops working in the generated HCL.
    ///
    /// This is the single place that distinguishes the two, so callers can hand over any
    /// user-supplied string without thinking about it.
    pub(crate) fn string_to_hcl_expr(s: &str) -> hcl::Expression {
        if s.contains("${") || s.contains("%{") {
            hcl::Expression::TemplateExpr(Box::new(hcl::TemplateExpr::QuotedString(
                escape_template_literals(s),
            )))
        } else {
            hcl::Expression::from(s.to_string())
        }
    }

    fn parse_hcl_expr(&self, s: &str) -> hcl::Expression {
        // Interpolation wins over the traversal heuristic below: a string like
        // "projects/${google_project.x.project_id}" is a template, not a traversal.
        if s.contains("${") || s.contains("%{") {
            return Self::string_to_hcl_expr(s);
        }
        if s.contains('.') && !s.contains('/') && !s.contains(':') {
            let parts: Vec<&str> = s.split('.').collect();
            if let Ok(var) = hcl::Variable::new(parts[0]) {
                let mut operators = Vec::new();
                for part in &parts[1..] {
                    if let Ok(ident) = hcl::Identifier::new(*part) {
                        operators.push(hcl::TraversalOperator::GetAttr(ident));
                    } else {
                        return Self::string_to_hcl_expr(s);
                    }
                }
                return hcl::Expression::Traversal(Box::new(hcl::Traversal::new(var, operators)));
            }
        }
        Self::string_to_hcl_expr(s)
    }

    pub fn transpile(&self) -> Result<GeneratedProject, Box<dyn std::error::Error>> {
        let mut main_blocks: Vec<hcl::Block> = Vec::new();
        let mut provider_blocks: Vec<hcl::Block> = Vec::new();
        let mut variable_blocks: Vec<hcl::Block> = Vec::new();
        let mut import_blocks: Vec<hcl::Block> = Vec::new();
        let mut tfvars_lines: Vec<String> = Vec::new();

        // Terraform Block (Backend)
        // Terraform Block (Backend & Settings)
        if let Some(tf_val) = &self.config.terraform {
            let mut tf_block = hcl::Block::builder("terraform");
            let mut has_required_providers = false;

            if let serde_yaml::Value::Mapping(map) = tf_val {
                let mode = self.get_deployment_mode();
                for (k, v) in map {
                    if let serde_yaml::Value::String(k_str) = k {
                         if k_str == "backend" {
                             if let serde_yaml::Value::Mapping(be_map) = v {
                                 for (be_type, be_config) in be_map {
                                     if let serde_yaml::Value::String(be_type_str) = be_type {
                                         // Only include the backend block that matches the current mode
                                         if (mode == "local" && be_type_str == "local") || (mode == "cloud" && be_type_str == "gcs") {
                                             let mut be_builder = hcl::Block::builder("backend").add_label(be_type_str);
                                             if let serde_yaml::Value::Mapping(c_map) = be_config {
                                                 for (ck, cv) in c_map {
                                                     if let serde_yaml::Value::String(cks) = ck {
                                                         if let Some(cval) = self.yaml_to_hcl_value(cv) {
                                                             be_builder = be_builder.add_attribute((cks.as_str(), cval));
                                                         }
                                                     }
                                                 }
                                             }
                                             tf_block = tf_block.add_block(be_builder.build());
                                         }
                                     }
                                 }
                             }
                         } else if k_str == "required_providers" {
                              has_required_providers = true;
                              if let Some(rp_block) = self.yaml_to_hcl_block("required_providers", v, None) {
                                  tf_block = tf_block.add_block(rp_block);
                              }
                         } else if let Some(val) = self.yaml_to_hcl_value(v) {
                             tf_block = tf_block.add_attribute((k_str.as_str(), val));
                         }
                    }
                }
            }

            // Add automatic required_providers if missing and we have providers
            if !has_required_providers {
                if let Some(providers) = &self.config.providers {
                    let mut rp_builder = hcl::Block::builder("required_providers");
                    for p_name in providers.keys() {
                        if let Some(source) = self.provider_sources.get(p_name) {
                            let mut p_map = hcl::Map::new();
                            p_map.insert("source".to_string(), hcl::Value::from(source.clone()));
                            if let Some(ver) = self.provider_versions.get(p_name) {
                                p_map.insert("version".to_string(), hcl::Value::from(ver.clone()));
                            }
                            rp_builder = rp_builder.add_attribute((p_name.as_str(), hcl::Value::from(p_map)));
                        }
                    }
                    tf_block = tf_block.add_block(rp_builder.build());
                }
            }
            provider_blocks.push(tf_block.build());
        } else {
            return Err("Missing 'terraform' block in YAML configuration. This is required for backend configuration.".into());
        }

        // Providers
        if let Some(providers) = &self.config.providers {
            let mut sorted_providers: Vec<_> = providers.keys().collect();
            sorted_providers.sort();

            for p_name in sorted_providers {
                let p_val = providers.get(p_name).unwrap();
                match p_val {
                    serde_yaml::Value::Sequence(seq) => {
                        for item in seq {
                            let mut builder = hcl::Block::builder("provider").add_label(p_name);
                            if let serde_yaml::Value::Mapping(map) = item {
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

                                        if let Some(val) = self.yaml_to_hcl_value(v) {
                                            builder = builder.add_attribute((k_str.as_str(), val));
                                        }
                                    }
                                }
                                 if !has_alias {
                                     builder = builder.add_attribute(("alias", p_name.as_str()));
                                 }

                                 if p_name == "google" || p_name == "google-beta" {
                                     builder = self.configure_google_provider(builder, project_id, has_billing_project, has_user_project_override);
                                 }

                                 provider_blocks.push(builder.build());
                            }
                        }
                    }
                    serde_yaml::Value::Mapping(map) => {
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

                                if let Some(val) = self.yaml_to_hcl_value(v) {
                                     builder = builder.add_attribute((k_str.as_str(), val));
                                }
                            }
                        }
                        if !has_alias {
                            builder = builder.add_attribute(("alias", p_name.as_str()));
                        }

                        if p_name == "google" || p_name == "google-beta" {
                            builder = self.configure_google_provider(builder, project_id, has_billing_project, has_user_project_override);
                        }

                        provider_blocks.push(builder.build());
                    }
                    _ => {}
                }
            }
        }

        // Root Context
        let cust_org_id = self.config.extra.get("customer-organization-id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'customer-organization-id' in configuration. Define it under `variables:` in your YAML config.")?;

        let root_ctx = ResourceContext {
            org_id: Some(cust_org_id.to_string()),
            org_ref: Some(format!("organizations/{}", cust_org_id)),
            provider_alias: Some("google.google".to_string()),
            ..Default::default()
        };

        // Organization Policies (google_org_policy_policy)
        if let Some(policies) = &self.config.org_policy_policy {
            let schema = if let Some(reg) = &self.registry {
                reg.find_resource("google_org_policy_policy")
                    .map(|(_, s)| s)
            } else {
                None
            };

            // org_policy_policy is modeled as HashMap<String, Value> in Config,
            // but transpile_mapping_resources expects a serde_yaml::Mapping.
            // Convert it on the fly so we can reuse the generic mapping logic.
            let mut map = serde_yaml::Mapping::new();
            for (k, v) in policies {
                map.insert(serde_yaml::Value::String(k.clone()), v.clone());
            }

            self.transpile_mapping_resources(
                &mut main_blocks,
                &mut provider_blocks,
                &mut import_blocks,
                "google_org_policy_policy",
                &map,
                schema,
                &root_ctx,
                root_ctx.provider_alias.as_deref(),
            )?;
        }

        // Organization IAM — collected, not emitted: fragments elsewhere in the tree may
        // contribute to the same members, and all of it must emit exactly once.
        if let Some(iam_members) = &self.config.organization_iam_member {
            let mut hoisted = self.hoisted.borrow_mut();
            let mut members: Vec<_> = iam_members.iter().collect();
            members.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (member, roles) in members {
                hoisted.insert_org_iam(member, roles);
            }
        }

        // Billing Account IAM — collected like org IAM: fragments anywhere in the tree
        // may contribute grants, all of it emits once at the billing account.
        if let Some(val) = &self.config.billing_account_iam_member {
            if let serde_yaml::Value::Mapping(map) = val {
                let mut hoisted = self.hoisted.borrow_mut();
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by_key(|(k, _)| format!("{:?}", k));
                for (k, v) in entries {
                    if let serde_yaml::Value::String(k_str) = k {
                        if k_str == "billing_account_id" {
                            if let serde_yaml::Value::String(id) = v {
                                hoisted.set_billing_account_id(id, "the root config".to_string());
                            }
                        } else if let serde_yaml::Value::Sequence(seq) = v {
                            hoisted.insert_billing_iam(k_str, seq);
                        }
                    }
                }
            }
        }

        // Folders and Projects

        // Folders and Projects
        if let Some(folders) = &self.config.folder {
            self.transpile_google_folder(&mut main_blocks, &mut provider_blocks, &mut import_blocks, folders, &root_ctx)?;
        }

        // Root Projects
        if let Some(projects) = &self.config.project {
            self.transpile_google_project(&mut main_blocks, &mut provider_blocks, &mut import_blocks, projects, &root_ctx)?;
        }

        // Root Generic Resources
        // Use google.google as default root provider to match ci.py and state
        self.transpile_generic_resources(&mut main_blocks, &mut provider_blocks, &mut import_blocks, &self.config.extra, &root_ctx, Some("google.google"))?;

        // Hoisted scopes: everything the walk collected is folded by the satz-core
        // algebra (grant union, deep-equal idempotence, conflict = ⊥ with provenance —
        // the property-tested laws) and emitted exactly once at its intrinsic scope, in
        // sorted order. Conflicts abort before any file is written.
        {
            let hoisted = std::mem::take(&mut *self.hoisted.borrow_mut());
            let drained = hoisted.drain()?;

            if !drained.groups.is_empty() {
                let mut merged = serde_yaml::Mapping::new();
                for (key, body) in &drained.groups {
                    merged.insert(serde_yaml::Value::String(key.clone()), body.clone());
                }
                // Same provider alias root-level groups always had.
                self.transpile_cloud_identity_groups(&mut main_blocks, &mut import_blocks, &merged, Some("google.google"));
            }

            if !drained.org_iam.is_empty() {
                let merged: HashMap<String, Vec<serde_yaml::Value>> =
                    drained.org_iam.into_iter().collect();
                self.transpile_iam_members(&mut main_blocks, &mut import_blocks, &merged, "google_organization_iam_member", "org_id", &root_ctx, root_ctx.provider_alias.as_deref(), None);
            }

            if !drained.billing_iam.is_empty() {
                // Explicit id from a fragment wins; else the conventional variable.
                let explicit_id = drained.billing_account_id.or_else(|| {
                    self.variables
                        .get("billing-account-infra")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                });
                let merged: HashMap<String, Vec<serde_yaml::Value>> =
                    drained.billing_iam.into_iter().collect();
                self.transpile_iam_members(&mut main_blocks, &mut import_blocks, &merged, "google_billing_account_iam_member", "billing_account_id", &root_ctx, root_ctx.provider_alias.as_deref(), explicit_id);
            }
        }

        // Variables
        let mut sorted_vars: Vec<_> = self.variables.keys().collect();
        sorted_vars.sort();
        for key in sorted_vars {
            let val = self.variables.get(key).unwrap();

            // vars.tf: variable "key" { type = string }
            // For now, assume everything is a string or let terraform infer 'any'
            // But usually string is safe for what we see in the yaml
            variable_blocks.push(hcl::Block::builder("variable")
                .add_label(key)
                .add_attribute(("type", hcl::Expression::Variable(hcl::Variable::new("string").unwrap())))
                .build());

            // .tfvars: key = "value"
            if let Some(hcl_val) = self.yaml_to_hcl_value(val) {
                 tfvars_lines.push(format!("{} = {}", key, hcl_val));
            }
        }

        let main_blocks = dedup_resource_blocks(main_blocks)?;

        let mut main_body = hcl::Body::builder();
        for block in main_blocks { main_body = main_body.add_block(block); }

        let mut prov_body = hcl::Body::builder();
        for block in provider_blocks { prov_body = prov_body.add_block(block); }

        let mut var_body = hcl::Body::builder();
        for block in variable_blocks { var_body = var_body.add_block(block); }

        let mut import_body = hcl::Body::builder();
        let mut seen_imports = std::collections::HashSet::new();
        for block in import_blocks {
            let rendered = hcl::to_string(&hcl::Body::builder().add_block(block.clone()).build()).unwrap_or_default();
            if seen_imports.insert(rendered) {
                import_body = import_body.add_block(block);
            }
        }

        Ok(GeneratedProject {
            main_tf: hcl::to_string(&main_body.build())?,
            providers_tf: hcl::to_string(&prov_body.build())?,
            variables_tf: hcl::to_string(&var_body.build())?,
            tfvars: tfvars_lines.join("\n"),
            imports_tf: hcl::to_string(&import_body.build())?,
        })
    }

    fn transpile_google_folder(
        &self,
        blocks: &mut Vec<hcl::Block>,
        provider_blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        folders: &HashMap<String, Folder>,
        ctx: &ResourceContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sorted_keys: Vec<_> = folders.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let folder = folders.get(key).unwrap();
            let resource_name = key.as_str().replace("-", "_");

            // Conditional Folders: If display_name is empty, skip folder creation and promote children to current context.
            if folder.display_name.trim().is_empty() {
                if let Some(sub_folders) = &folder.folder {
                    self.transpile_google_folder(blocks, provider_blocks, import_blocks, sub_folders, ctx)?;
                }
                if let Some(projects) = &folder.project {
                    self.transpile_google_project(blocks, provider_blocks, import_blocks, projects, ctx)?;
                }
                self.transpile_generic_resources(blocks, provider_blocks, import_blocks, &folder.extra, ctx, None)?;
                continue;
            }

            let parent_val_expr = if let Some(pref) = &ctx.folder_ref {
                self.parse_hcl_expr(pref)
            } else {
                let org_ref = ctx.org_ref.as_ref().ok_or_else(|| format!(
                    "folder '{}' has no parent: it is not nested under a folder and no enclosing organization is set",
                    resource_name
                ))?;
                hcl::Expression::from(org_ref.clone())
            };

            let folder_block = crate::emit_shared::folder_block(
                &resource_name,
                &folder.display_name,
                parent_val_expr,
                ctx.provider_alias.as_deref(),
                folder.extra.get("labels"),
                folder.extra.get("lifecycle").and_then(|v| self.yaml_to_lifecycle_block(v)),
            );

            blocks.push(folder_block);

            // Generate Import Block if requested
            if let Some(id) = &folder.import_id {
                import_blocks.push(hcl::Block::builder("import")
                    .add_attribute(("to", self.parse_hcl_expr(&format!("google_folder.{}", resource_name))))
                    .add_attribute(("id", id.clone()))
                    .build());
            }

            let current_hcl_ref = format!("google_folder.{}.name", resource_name);
            let mut folder_ctx = ctx.clone();
            folder_ctx.folder_id = Some(current_hcl_ref.clone()); // Simplification: we use HCL ref as identifier in YAML usually
            folder_ctx.folder_ref = Some(current_hcl_ref);

            // Generic Resources (includes CEX_ and others in extra)
            self.transpile_generic_resources(blocks, provider_blocks, import_blocks, &folder.extra, &folder_ctx, folder_ctx.provider_alias.as_deref())?;

            if let Some(sub_folders) = &folder.folder {
                self.transpile_google_folder(blocks, provider_blocks, import_blocks, sub_folders, &folder_ctx)?;
            }
            if let Some(projects) = &folder.project {
                self.transpile_google_project(blocks, provider_blocks, import_blocks, projects, &folder_ctx)?;
            }
        }
        Ok(())
    }

    fn transpile_google_project(
        &self,
        blocks: &mut Vec<hcl::Block>,
        provider_blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        projects: &HashMap<String, Project>,
        ctx: &ResourceContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sorted_keys: Vec<_> = projects.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let project = projects.get(key).unwrap();
            let resource_name = key.as_str().replace("-", "_");

            let mut block_builder = hcl::Block::builder("resource")
                .add_label("google_project")
                .add_label(&resource_name)
                .add_attribute(hcl::Attribute::new("project_id", project.project_id.clone()))
                .add_attribute(hcl::Attribute::new("name", project.name.clone().unwrap_or_else(|| project.project_id.clone())));

            if let Some(alias) = &ctx.provider_alias {
                if let Ok(expr) = alias.parse::<hcl::Expression>() {
                    block_builder = block_builder.add_attribute(("provider", expr));
                }
            }

            // Emit billing_account: explicit YAML value takes priority, then variable fallback
            if let Some(ba) = &project.billing_account {
                block_builder = block_builder.add_attribute(hcl::Attribute::new("billing_account", ba.clone()));
            } else if !project.extra.contains_key("billing_account") {
                if let Some(ba) = self.variables.get("billing-account-infra") {
                    if let Some(val) = self.yaml_to_hcl_value(ba) {
                        block_builder = block_builder.add_attribute(hcl::Attribute::new("billing_account", val));
                    }
                }
            }

            let has_org = project.extra.contains_key("org_id") || project.extra.contains_key("org") || project.extra.contains_key("folder_id");
            if !has_org {
                if let Some(f_ref) = &ctx.folder_ref {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new("folder_id", self.parse_hcl_expr(f_ref)));
                } else if let Some(oid) = &ctx.org_id {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new("org_id", oid.clone()));
                }
            }

            // Emit explicit Project fields that serde captures outside of `extra`
            if let Some(labels) = &project.labels {
                if !labels.is_empty() {
                    let mut sorted: Vec<_> = labels.iter().collect();
                    sorted.sort_by_key(|(k, _)| k.as_str());
                    let map: hcl::Map<String, hcl::Value> = sorted
                        .into_iter()
                        .map(|(k, v)| (k.clone(), hcl::Value::from(v.clone())))
                        .collect();
                    block_builder = block_builder.add_attribute(("labels", hcl::Value::from(map)));
                }
            }
            if let Some(dp) = &project.deletion_policy {
                block_builder = block_builder.add_attribute(("deletion_policy", dp.clone()));
            }
            if let Some(tags) = &project.tags {
                if !tags.is_empty() {
                    let seq: Vec<hcl::Value> = tags.iter().map(|t| hcl::Value::from(t.clone())).collect();
                    block_builder = block_builder.add_attribute(("tags", hcl::Value::from(seq)));
                }
            }

            // Add attributes from extra
            let (_, resource_schema) = if let Some(reg) = &self.registry {
                reg.find_resource("google_project").map(|(p, s)| (p, Some(s))).unwrap_or(("google", None))
            } else {
                ("google", None)
            };

            for (k, v) in &project.extra {
                // Filter out keys that are actually resources handled later
                let is_resource = if let Some(reg) = &self.registry {
                    reg.find_resource(k).is_some()
                } else {
                    false // Without registry, we can't verify, so be conservative
                };

                if is_resource { continue; }

                // Terraform `lifecycle` meta-argument block.
                if k == "lifecycle" {
                    if let Some(lc_block) = self.yaml_to_lifecycle_block(v) {
                        block_builder = block_builder.add_block(lc_block);
                    }
                    continue;
                }

                let is_block = if let Some(schema) = resource_schema {
                    schema.block.block_types.contains_key(k)
                } else {
                    matches!(v, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)) && !matches!(k.as_str(), "labels" | "metadata" | "annotations")
                };

                if is_block {
                    if let Some(block) = self.yaml_to_hcl_block(k, v, None) {
                        block_builder = block_builder.add_block(block);
                    }
                } else if let Some(val) = self.yaml_to_hcl_value(v) {
                    block_builder = block_builder.add_attribute(hcl::Attribute::new(k.as_str(), val));
                }
            }

            blocks.push(block_builder.build());

            // Generate Import Block if requested
            if let Some(id) = &project.import_id {
                import_blocks.push(hcl::Block::builder("import")
                    .add_attribute(("to", self.parse_hcl_expr(&format!("google_project.{}", resource_name))))
                    .add_attribute(("id", id.clone()))
                    .build());
            }

            if let Some(reg) = &self.registry {
                if let Some((_, schema)) = reg.find_resource("google_project") {
                    let mut validation_attrs = project.extra.clone();
                    validation_attrs.insert("project_id".to_string(), serde_yaml::Value::String(project.project_id.clone()));
                    if let Some(name) = &project.name {
                        validation_attrs.insert("name".to_string(), serde_yaml::Value::String(name.clone()));
                    } else {
                        validation_attrs.insert("name".to_string(), serde_yaml::Value::String(project.project_id.clone()));
                    }
                    if let Some(fid) = &ctx.folder_id {
                        validation_attrs.insert("folder_id".to_string(), serde_yaml::Value::String(fid.clone()));
                    } else if let Some(oid) = &ctx.org_id {
                        validation_attrs.insert("org_id".to_string(), serde_yaml::Value::String(oid.clone()));
                    }

                    self.validate_resource("google_project", &resource_name, &validation_attrs, schema);
                }
            }

            let project_id_ref = format!("google_project.{}.project_id", resource_name);
            let mut project_ctx = ctx.clone();
            project_ctx.project_id = Some(project.project_id.clone());
            project_ctx.project_ref = Some(project_id_ref);

            // Project specific provider for project resources
            let p_alias = format!("project_{}", key.replace("-", "_"));
            let mut p_builder = hcl::Block::builder("provider")
                .add_label("google")
                .add_attribute(("alias", p_alias.clone()))
                .add_attribute(("project", project.project_id.clone()));

            p_builder = self.configure_google_provider(p_builder, Some(project.project_id.clone()), false, false);

            // Default region if not specified (could be improved to come from project config)
            p_builder = p_builder.add_attribute(("region", "europe-west3"));

            provider_blocks.push(p_builder.build());

            let p_ref = format!("google.{}", p_alias);

            // Project Services
            if let Some(services) = &project.project_service {
                for service_val in services {
                    let project_id_ref = format!("google_project.{}.project_id", resource_name);
                    self.transpile_google_project_service(blocks, import_blocks, &project_id_ref, service_val, ctx.provider_alias.as_deref(), &resource_name);
                }
            }

            // Generic Resources (includes CEX_ and others in extra)
            self.transpile_generic_resources(blocks, provider_blocks, import_blocks, &project.extra, &project_ctx, Some(&p_ref))?;
        }
        Ok(())
    }

    fn transpile_generic_resources(
        &self,
        blocks: &mut Vec<hcl::Block>,
        provider_blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        extra: &HashMap<String, serde_yaml::Value>,
        ctx: &ResourceContext,
        provider_alias: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sorted_types: Vec<_> = extra.keys().collect();
        sorted_types.sort();

        for resource_type in sorted_types {
            let value = extra.get(resource_type).unwrap();

            // Skip known non-resource keys
            if resource_type == "variables" {
                continue;
            }

            // Skip keys that are known resource parameters (never Terraform resource types)
            const KNOWN_ATTRIBUTE_KEYS: &[&str] = &[
                "labels", "deletion_protection", "deletion_policy", "metadata", "annotations",
                "name", "project_id", "billing_account", "tags", "display_name", "parent",
                "lifecycle",
            ];
            if KNOWN_ATTRIBUTE_KEYS.contains(&resource_type.as_str()) {
                continue;
            }

            // Only treat Mapping values as potential resources (attributes/variables are usually simple values)
            // Skip if value is not a Mapping or Sequence (which would indicate it's not a resource)
            if !matches!(value, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)) {
                continue;
            }

            // Only consider keys that look like Terraform resource types (contain underscore or start with google_)
            // This avoids false "unknown resource" errors for attribute-like keys (e.g. labels, deletion_protection)
            let looks_like_resource_type = resource_type.contains('_') || resource_type.starts_with("google_");
            if !looks_like_resource_type {
                continue;
            }

            // Handle CEX_ prefix for "compact" resources that need explosion
            if let Some(actual_type) = resource_type.strip_prefix("CEX_") {
                let tf_type = if actual_type.starts_with("google_") {
                    actual_type.to_string()
                } else {
                    format!("google_{}", actual_type)
                };

                if let serde_yaml::Value::Mapping(map) = value {
                    for (key_val, items_val) in map {
                        if let (serde_yaml::Value::String(key), serde_yaml::Value::Sequence(items)) = (key_val, items_val) {
                            if tf_type.contains("iam_member") {
                                // Special case for IAM members: key is member, items are roles.
                                // Org-scoped grants go to the hoisting collector like the
                                // non-CEX path; the rest emits in place.
                                if tf_type == "google_organization_iam_member" {
                                    self.hoisted.borrow_mut().insert_org_iam(key, items);
                                    continue;
                                }
                                if tf_type == "google_billing_account_iam_member" {
                                    self.hoisted.borrow_mut().insert_billing_iam(key, items);
                                    continue;
                                }
                                let mut iam_map = HashMap::new();
                                iam_map.insert(key.clone(), items.clone());

                                let id_attr = if tf_type.contains("project") { "project" }
                                             else if tf_type.contains("folder") { "folder" }
                                             else { "id" };

                                if ctx.project_ref.as_ref().or(ctx.folder_ref.as_ref()).or(ctx.org_ref.as_ref()).is_some() {
                                    self.transpile_iam_members(blocks, import_blocks, &iam_map, &tf_type, id_attr, ctx, provider_alias, None);
                                }
                            } else {
                                // TODO: Generic explosion for non-IAM resources
                            }
                        }
                    }
                }
                continue;
            }

            // Compact Cloud Identity Group Expansion — customer-scoped, so collected for
            // one emission at root no matter where in the tree the block appears.
            // `cloud_identity_group` is not a `google_` shorthand — it is a
            // satz abstraction that EXPANDS into a group resource (group_key,
            // parent = customers/<id>, the discussion_forum/security labels).
            // The full name is accepted too because Satz names Terraform types in
            // full, so a compiled twin writes `google_cloud_identity_group:`; the
            // dialect's own short spelling keeps working.
            if resource_type == "cloud_identity_group" || resource_type == "google_cloud_identity_group" {
                if let serde_yaml::Value::Mapping(groups) = value {
                    let mut hoisted = self.hoisted.borrow_mut();
                    for (g_key, g_body) in groups {
                        if let serde_yaml::Value::String(g_key) = g_key {
                            hoisted.insert_group(g_key, g_body, hoist_provenance(ctx));
                        }
                    }
                }
                continue;
            }

            // Normal processing for non-prefixed or non-exploded resources
            let (tf_type, resource_schema) = if let Some(reg) = &self.registry {
                if let Some((_, schema)) = reg.find_resource(resource_type) {
                    let resolved_name = if reg.resources.contains_key(resource_type) {
                        resource_type.to_string()
                    } else if resource_type.starts_with("google_") {
                        resource_type.to_string()
                    } else {
                        format!("google_{}", resource_type)
                    };
                    (resolved_name, Some(schema))
                } else {
                    // Resource type not found in registry - only generate error if value is a Mapping/Sequence
                    // (which would indicate it's meant to be a resource, not just an attribute)
                    let resolved_name = if resource_type.starts_with("google_") {
                        resource_type.to_string()
                    } else {
                        format!("google_{}", resource_type)
                    };
                    if matches!(value, serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)) {
                        eprintln!("Error: Unknown resource type '{}' (resolved as '{}'). This resource type does not exist in the Terraform provider schema. Please check the resource name or use a valid Terraform resource type.", resource_type, resolved_name);
                    }
                    (resolved_name, None)
                }
            } else if resource_type.starts_with("google_") {
                (resource_type.to_string(), None)
            } else {
                (format!("google_{}", resource_type), None)
            };

            if let Some(map) = value.as_mapping() {
                self.transpile_mapping_resources(blocks, provider_blocks, import_blocks, &tf_type, map, resource_schema, ctx, provider_alias)?;
            } else if let Some(seq) = value.as_sequence() {
                for (i, item) in seq.iter().enumerate() {
                    if let Some(attrs) = item.as_mapping() {
                        let res_name = attrs.get(serde_yaml::Value::String("name".to_string()))
                            .or_else(|| attrs.get(serde_yaml::Value::String("constraint".to_string())))
                            .and_then(|v| v.as_str())
                            .map(|s| s.replace(".", "_"))
                            .unwrap_or_else(|| i.to_string());

                        self.transpile_single_resource(blocks, import_blocks, &tf_type, &res_name, attrs, resource_schema, ctx, provider_alias)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn transpile_mapping_resources(
        &self,
        blocks: &mut Vec<hcl::Block>,
        _provider_blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        tf_type: &str,
        map: &serde_yaml::Mapping,
        resource_schema: Option<&crate::schema::ResourceSchema>,
        ctx: &ResourceContext,
        provider_alias: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Hoisted-scope IAM types are collected, never emitted in place — and the
        // collection must not depend on auto_explode settings or on which entry happens
        // to be first in the mapping (the explode gate below only inspects the first
        // value's shape).
        if tf_type == "google_organization_iam_member" || tf_type == "google_billing_account_iam_member" {
            let billing = tf_type == "google_billing_account_iam_member";
            let mut hoisted = self.hoisted.borrow_mut();
            for (m_val, r_val) in map {
                match (m_val, r_val) {
                    (serde_yaml::Value::String(m), serde_yaml::Value::Sequence(r)) => {
                        if billing {
                            hoisted.insert_billing_iam(m, r);
                        } else {
                            hoisted.insert_org_iam(m, r);
                        }
                    }
                    // A fragment may pin the billing account explicitly.
                    (serde_yaml::Value::String(k), serde_yaml::Value::String(id))
                        if billing && k == "billing_account_id" =>
                    {
                        hoisted.set_billing_account_id(id, hoist_provenance(ctx));
                    }
                    _ => {}
                }
            }
            return Ok(());
        }

        // Check if this tf_type is in the auto_explode list
        let mut should_explode = false;
        for pattern in &self.auto_explode {
            if self.matches_pattern(pattern, tf_type) {
                should_explode = true;
                break;
            }
        }

        if should_explode {
            // ... (rest of explode logic)
            if let Some((_, first_val)) = map.iter().next() {
                if first_val.is_sequence() {
                    if tf_type.contains("iam_member") {
                        let mut iam_map = HashMap::new();
                        for (m_val, r_val) in map {
                            if let (serde_yaml::Value::String(m), serde_yaml::Value::Sequence(r)) = (m_val, r_val) {
                                iam_map.insert(m.clone(), r.clone());
                            }
                        }
                        let id_attr = if tf_type.contains("project") { "project" }
                                     else if tf_type.contains("folder") { "folder" }
                                     else { "id" };
                        self.transpile_iam_members(blocks, import_blocks, &iam_map, tf_type, id_attr, ctx, provider_alias, None);
                        return Ok(());
                    } else if tf_type == "google_project_service" {
                        for (project_ref_val, s_val) in map {
                            if let (serde_yaml::Value::String(project_ref), serde_yaml::Value::Sequence(services)) = (project_ref_val, s_val) {
                                for service_val in services {
                                    let safe_project = project_ref.replace(&['.', ':'][..], "_");
                                    self.transpile_google_project_service(blocks, import_blocks, project_ref, service_val, provider_alias, &safe_project);
                                }
                            }
                        }
                        return Ok(());
                    }
                }
            }
        }

        for (res_name_val, res_attrs_val) in map {
            if let (serde_yaml::Value::String(res_name), serde_yaml::Value::Mapping(attrs)) = (res_name_val, res_attrs_val) {
                self.transpile_single_resource(blocks, import_blocks, tf_type, res_name, attrs, resource_schema, ctx, provider_alias)?;
            }
        }
        Ok(())
    }

    fn transpile_single_resource(
        &self,
        blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        tf_type: &str,
        res_name: &str,
        attrs: &serde_yaml::Mapping,
        resource_schema: Option<&crate::schema::ResourceSchema>,
        ctx: &ResourceContext,
        provider_alias: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let shared_ctx = crate::emit_shared::ResCtx {
            org_id: ctx.org_id.clone(),
            org_ref: ctx.org_ref.clone(),
            folder_id: ctx.folder_id.clone(),
            folder_ref: ctx.folder_ref.clone(),
            project_id: ctx.project_id.clone(),
            project_ref: ctx.project_ref.clone(),
        };
        let validate = |final_attrs: &serde_yaml::Mapping| {
            if let Some(schema) = resource_schema {
                let mut val_attrs = HashMap::new();
                for (k, v) in final_attrs {
                    if let serde_yaml::Value::String(ks) = k {
                        val_attrs.insert(ks.clone(), v.clone());
                    }
                }
                self.validate_resource(tf_type, res_name, &val_attrs, schema);
            }
        };
        let (block, import_id, label) = crate::emit_shared::single_resource_block(
            tf_type,
            res_name,
            attrs,
            resource_schema,
            &shared_ctx,
            provider_alias,
            self.variables.get("billing-account-infra"),
            &|val| self.resolve_anchor_reference(val),
            Some(&validate),
        )?;
        blocks.push(block);
        if let Some(id) = import_id {
            import_blocks.push(hcl::Block::builder("import")
                .add_attribute(("to", self.parse_hcl_expr(&format!("{}.{}", tf_type, label))))
                .add_attribute(("id", id))
                .build());
        }
        Ok(())
    }

    fn transpile_iam_members(
        &self,
        blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        iam_members: &HashMap<String, Vec<serde_yaml::Value>>,
        resource_type: &str,
        id_attribute: &str,
        ctx: &ResourceContext,
        provider_alias: Option<&str>,
        explicit_parent_id: Option<String>,
    ) {
        let parent_expr_str_option = match id_attribute {
            "project" | "project_id" => ctx.project_ref.as_deref().or(ctx.project_id.as_deref()),
            "folder" | "folder_id" => ctx.folder_ref.as_deref().or(ctx.folder_id.as_deref()),
            "org_id" => ctx.org_id.as_deref().or(ctx.org_ref.as_deref()),
            _ => None,
        };

        let parent_val_expr = if let Some(explicit) = explicit_parent_id {
            self.parse_hcl_expr(&explicit)
        } else {
            self.parse_hcl_expr(parent_expr_str_option.unwrap_or(""))
        };

        // HashMap iteration order is random per process; sort so the emitted block order
        // is stable across runs.
        let mut sorted_members: Vec<_> = iam_members.iter().collect();
        sorted_members.sort_by_key(|(m, _)| m.as_str());
        for (member, roles) in sorted_members {
            for role_val in roles {
                let (role, condition_val, import_id) = match role_val {
                    serde_yaml::Value::String(s) => (s.clone(), None, None),
                    serde_yaml::Value::Mapping(m) => {
                        let mut role = String::new();
                        let mut condition_val = None;
                        let mut import_id = None;
                        for (k, v) in m {
                            if let serde_yaml::Value::String(k_str) = k {
                                if k_str == "condition" {
                                    condition_val = Some(v);
                                } else if k_str == "import-id" {
                                    import_id = v.as_str().map(|s| s.to_string());
                                } else if k_str == "role" {
                                    // Explicit form `{ role: <name>, condition: {…} }` — what
                                    // Satz emits. The legacy YAML form puts the role in the
                                    // key with a null value; both are accepted.
                                    if let Some(rv) = v.as_str() {
                                        role = rv.to_string();
                                    }
                                } else {
                                    role = k_str.clone();
                                }
                            }
                        }
                        if role.is_empty() {
                            continue;
                        }
                        (role, condition_val, import_id)
                    }
                    _ => {
                        eprintln!("DEBUG: Role value is not string or mapping: {:?}", role_val);
                        continue;
                    }
                };

                let label = crate::emit_shared::iam_member_label(member, &role, condition_val);
                blocks.push(crate::emit_shared::iam_member_block(
                    resource_type,
                    &label,
                    &role,
                    // Members may carry ${...} references — route through the helper so
                    // they are emitted as templates, not as escaped literals.
                    Self::string_to_hcl_expr(member),
                    id_attribute,
                    parent_val_expr.clone(),
                    condition_val.and_then(|cv| self.yaml_to_hcl_block("condition", cv, None)),
                    provider_alias,
                ));

                // Generate Import Block if requested
                if let Some(id) = import_id {
                    import_blocks.push(hcl::Block::builder("import")
                        .add_attribute(("to", self.parse_hcl_expr(&format!("{}.{}", resource_type, label))))
                        .add_attribute(("id", id))
                        .build());
                }
            }
        }
    }

    fn validate_resource(&self, tf_type: &str, name: &str, attrs: &HashMap<String, serde_yaml::Value>, schema: &crate::schema::ResourceSchema) {
        if self.validation_level == "none" { return; }

        for (attr_name, attr_schema) in &schema.block.attributes {
            if attr_schema.required && !attrs.contains_key(attr_name) {
                // Special case for project/project_id which might be injected
                if (attr_name == "project" || attr_name == "project_id") && (attrs.contains_key("project") || attrs.contains_key("project_id")) {
                    continue;
                }

                let msg = format!("Missing mandatory parameter '{}' for resource '{}' ({})", attr_name, name, tf_type);
                if self.validation_level == "error" {
                    eprintln!("Error: {}", msg);
                    std::process::exit(1);
                } else {
                    eprintln!("Warning: {}", msg);
                }
            }
        }

        for (block_name, block_schema) in &schema.block.block_types {
            if let Some(min) = block_schema.min_items {
                if min > 0 && !attrs.contains_key(block_name) {
                    let msg = format!("Missing mandatory block '{}' for resource '{}' ({})", block_name, name, tf_type);
                    if self.validation_level == "error" {
                        eprintln!("Error: {}", msg);
                        std::process::exit(1);
                    } else {
                        eprintln!("Warning: {}", msg);
                    }
                }
            }
        }

        // Check for unknown fields
        for attr_name in attrs.keys() {
            // Special cases for meta-arguments and handled fields
            if attr_name == "depends_on" || attr_name == "lifecycle" || attr_name == "provider" || attr_name == "count" || attr_name == "for_each" {
                 continue;
            }
            if tf_type == "google_org_policy_policy" && (attr_name == "constraint" || attr_name == "type") {
                 continue;
            }
            if tf_type == "google_project" && (attr_name == "storage_bucket" || attr_name == "service_account" || attr_name == "project_iam_member" || attr_name == "project_service" || attr_name == "bigquery_dataset") {
                 continue;
            }

            // Automatically ignore if it's a resource type (nested resource)
            if let Some(reg) = &self.registry {
                if reg.find_resource(attr_name).is_some() {
                    continue;
                }
            }

            let is_known_attr = schema.block.attributes.contains_key(attr_name);
            let is_known_block = schema.block.block_types.contains_key(attr_name);

            if !is_known_attr && !is_known_block {
                // If not known, check if it's a parentage hint (project/project_id) which we allow even if not in schema
                if attr_name == "project" || attr_name == "project_id" {
                    continue;
                }

                let msg = format!("Unknown field '{}' for resource '{}' ({})", attr_name, name, tf_type);
                if self.validation_level == "error" {
                    eprintln!("Error: {}", msg);
                    std::process::exit(1);
                } else {
                    eprintln!("Warning: {}", msg);
                }
            }
        }
    }

    fn resolve_anchor_reference(&self, v: &serde_yaml::Value) -> Option<serde_yaml::Value> {
        // Check if the value is a string that looks like an anchor reference (starts with *)
        if let serde_yaml::Value::String(s) = v {
            if s.starts_with('*') {
                let anchor_name = s.strip_prefix('*')?;
                // Look up the anchor in the variables map
                if let Some(resolved_value) = self.variables.get(anchor_name) {
                    return Some(resolved_value.clone());
                } else {
                    // Anchor reference found but not resolved - this is an error
                    eprintln!("Warning: Anchor reference '*{}' was not found in variables. The anchor may not be defined or may not be in the 'variables' section.", anchor_name);
                    return None;
                }
            }
        }
        None
    }

    fn yaml_to_hcl_value(&self, v: &serde_yaml::Value) -> Option<hcl::Expression> {
        crate::emit_shared::render_value(v, &|val| self.resolve_anchor_reference(val))
    }

    /// Build a Terraform `lifecycle` meta-argument block from a YAML mapping.
    ///
    /// `ignore_changes` and `replace_triggered_by` are rendered as bare HCL
    /// identifiers/expressions (e.g. `initial_group_config`, `labels["env"]`),
    /// NOT quoted strings, by parsing each element as an HCL expression. The
    /// scalar form `ignore_changes: all` renders the bare `all` keyword. Other
    /// keys (`create_before_destroy`, `prevent_destroy`, ...) pass through
    /// `yaml_to_hcl_value`. Returns `None` if `v` is not a mapping.
    ///
    /// Shared by every resource-rendering path (the generic
    /// `transpile_single_resource` and the dedicated builders) so `lifecycle`
    /// works uniformly across all resource types.
    fn yaml_to_lifecycle_block(&self, v: &serde_yaml::Value) -> Option<hcl::Block> {
        crate::emit_shared::lifecycle_block(v, &|val| self.resolve_anchor_reference(val))
    }

    fn yaml_to_hcl_block(&self, name: &str, v: &serde_yaml::Value, schema: Option<&crate::schema::BlockSchema>) -> Option<hcl::Block> {
        crate::emit_shared::render_block(name, v, schema, &|val| self.resolve_anchor_reference(val))
    }

    fn transpile_cloud_identity_groups(&self, blocks: &mut Vec<hcl::Block>, import_blocks: &mut Vec<hcl::Block>, groups: &serde_yaml::Mapping, provider_alias: Option<&str>) {
        let customer_id = self.config.extra.get("customer-id").and_then(|v| v.as_str()).unwrap_or("");
        let customer_domain = self.config.extra.get("customer-domain").and_then(|v| v.as_str()).unwrap_or("");

        for (g_name_val, g_attrs_val) in groups {
            if let (serde_yaml::Value::String(group_name), serde_yaml::Value::Mapping(attrs)) = (g_name_val, g_attrs_val) {
                let resource_name = group_resource_label(group_name);
                blocks.push(crate::emit_shared::group_block(
                    group_name,
                    attrs,
                    customer_id,
                    customer_domain,
                    provider_alias,
                    attrs.get(serde_yaml::Value::String("lifecycle".to_string()))
                        .and_then(|v| self.yaml_to_lifecycle_block(v)),
                ));

                // Generate Import Block if requested
                if let Some(id) = attrs.get(serde_yaml::Value::String("import-id".to_string())).and_then(|v| v.as_str()) {
                    import_blocks.push(hcl::Block::builder("import")
                        .add_attribute(("to", self.parse_hcl_expr(&group_resource_address(group_name))))
                        .add_attribute(("id", id.to_string()))
                        .build());
                }

            // Handle Memberships - Aggregate roles by unique member email
            let _ = resource_name;
            blocks.extend(crate::emit_shared::membership_blocks(group_name, attrs, provider_alias));
        }
    }
}

    fn transpile_google_project_service(
        &self,
        blocks: &mut Vec<hcl::Block>,
        import_blocks: &mut Vec<hcl::Block>,
        project_ref: &str,
        service_val: &serde_yaml::Value,
        provider_alias: Option<&str>,
        safe_project_name: &str,
    ) {
        let service_configs = match service_val {
            serde_yaml::Value::String(s) => vec![(s.clone(), None)],
            serde_yaml::Value::Mapping(m) => {
                if let serde_yaml::Value::String(s) = m.get(serde_yaml::Value::String("service".to_string())).unwrap_or(&serde_yaml::Value::Null) {
                    // Flat format: { service: "...", disable_on_destroy: ... }
                    vec![(s.clone(), Some(m))]
                } else {
                    // Nested format: { "service_name": { "disable_on_destroy": ... } }
                    let mut v = Vec::new();
                    for (mk, mv) in m {
                        if let serde_yaml::Value::String(ms) = mk {
                            v.push((ms.clone(), mv.as_mapping()));
                        }
                    }
                    v
                }
            }
            _ => return,
        };

        for (service, service_attrs) in service_configs {
            let safe_service = service.replace(".", "_");
            let label = format!("{}_{}", safe_project_name, safe_service);
            blocks.push(crate::emit_shared::project_service_block(
                &label,
                self.parse_hcl_expr(project_ref),
                &service,
                service_attrs,
                provider_alias,
                &|val| self.resolve_anchor_reference(val),
            ));

            // Generate Import Block if requested
            if let Some(attrs) = service_attrs {
                if let Some(id) = attrs.get(serde_yaml::Value::String("import-id".to_string())).and_then(|v| v.as_str()) {
                    import_blocks.push(hcl::Block::builder("import")
                        .add_attribute(("to", self.parse_hcl_expr(&format!("google_project_service.{}", label))))
                        .add_attribute(("id", id.to_string()))
                        .build());
                }
            }
        }
    }

    fn google_provider_deps(&self) -> crate::emit_shared::GoogleProviderDeps {
        let infra_project = self.config.extra.get("infra-project-name").and_then(|v| v.as_str()).map(|s| s.to_string());
        let impersonate = if self.get_deployment_mode() == "cloud" {
            match (
                self.config.extra.get("svc-iac-account").and_then(|v| v.as_str()),
                self.config.extra.get("infra-project-name").and_then(|v| v.as_str()),
            ) {
                (Some(account), Some(proj)) => Some(format!("{}@{}.iam.gserviceaccount.com", account, proj)),
                _ => None,
            }
        } else {
            None
        };
        crate::emit_shared::GoogleProviderDeps { infra_project, impersonate }
    }

    fn configure_google_provider(&self, builder: hcl::BlockBuilder, project_id: Option<String>, has_billing_project: bool, has_user_project_override: bool) -> hcl::BlockBuilder {
        crate::emit_shared::configure_google_provider(builder, project_id, has_billing_project, has_user_project_override, &self.google_provider_deps())
    }

    fn get_deployment_mode(&self) -> String {
        self.config.extra.get("deployment-mode")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .to_string()
    }

    fn matches_pattern(&self, pattern: &str, text: &str) -> bool {
        if pattern.starts_with(".*") {
            text.ends_with(&pattern[2..])
        } else if pattern.ends_with(".*") {
            text.starts_with(&pattern[..pattern.len() - 2])
        } else {
            pattern == text
        }
    }
}

// Tests (pure layer only — no network, no filesystem).
#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(pairs: &[(&str, &str)]) -> serde_yaml::Mapping {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    serde_yaml::Value::String(k.to_string()),
                    serde_yaml::Value::String(v.to_string()),
                )
            })
            .collect()
    }

    #[test]
    fn group_email_prefers_id_then_email_then_the_domain_default() {
        assert_eq!(
            group_email("admins", &mapping(&[("display_name", "A")]), "example.com"),
            "admins@example.com"
        );
        assert_eq!(
            group_email("admins", &mapping(&[("email", "real@example.org")]), "example.com"),
            "real@example.org"
        );
        assert_eq!(
            group_email(
                "admins",
                &mapping(&[("id", "id@example.org"), ("email", "ignored@example.org")]),
                "example.com"
            ),
            "id@example.org"
        );
    }

    #[test]
    fn group_email_without_a_domain_is_left_visibly_incomplete() {
        // The importer keys off this: `name@` is not a group it should go looking for.
        assert_eq!(group_email("admins", &mapping(&[]), ""), "admins@");
    }

    #[test]
    fn group_address_replaces_dashes_the_way_the_resource_label_does() {
        assert_eq!(
            group_resource_address("gcp-organization-admins"),
            "google_cloud_identity_group.gcp_organization_admins"
        );
    }

    #[test]
    fn group_hcl_uses_the_shared_derivation() {
        // Guards the extraction of the helpers: what the importer computes and what lands
        // in the HCL must stay the same string.
        let yaml = format!(
            "{}\ncustomer-organization-id: \"123456789\"\ncustomer-domain: example.com\ncustomer-id: C01234567\n\
             cloud_identity_group:\n  gcp-security-admins:\n    display_name: Sec\n    \
             import-id: groups/00abc\n",
            MINIMAL_TERRAFORM
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        assert!(
            project.main_tf.contains("resource \"google_cloud_identity_group\" \"gcp_security_admins\""),
            "{}",
            project.main_tf
        );
        assert!(
            project.main_tf.contains("gcp-security-admins@example.com"),
            "{}",
            project.main_tf
        );
        assert!(
            project.imports_tf.contains("google_cloud_identity_group.gcp_security_admins"),
            "{}",
            project.imports_tf
        );
    }

    const HOIST_VARS: &str = "customer-organization-id: \"123456789\"\ncustomer-id: C01234567\ncustomer-domain: example.com\n";

    /// Two fragments in different folders declaring the same group and overlapping org
    /// grants must merge: one group resource, one resource per distinct grant. Before
    /// hoisting this silently emitted duplicate labels — invalid HCL.
    #[test]
    fn hoisted_org_resources_from_two_folders_emit_once() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             folder:\n\
             \x20 shared:\n\
             \x20   display_name: Shared\n\
             \x20   cloud_identity_group:\n\
             \x20     log-admins:\n\
             \x20       display_name: Log Admins\n\
             \x20   organization_iam_member:\n\
             \x20     \"group:log-admins@example.com\":\n\
             \x20       - roles/logging.admin\n\
             \x20 observability:\n\
             \x20   display_name: Observability\n\
             \x20   cloud_identity_group:\n\
             \x20     log-admins:\n\
             \x20       display_name: Log Admins\n\
             \x20   organization_iam_member:\n\
             \x20     \"group:log-admins@example.com\":\n\
             \x20       - roles/logging.admin\n\
             \x20       - roles/monitoring.admin\n"
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        let group_count = project
            .main_tf
            .matches("resource \"google_cloud_identity_group\" \"log_admins\"")
            .count();
        assert_eq!(group_count, 1, "group must emit once:\n{}", project.main_tf);
        assert_eq!(
            project.main_tf.matches("roles/logging.admin").count(),
            1,
            "shared grant must dedup:\n{}",
            project.main_tf
        );
        assert_eq!(project.main_tf.matches("roles/monitoring.admin").count(), 1);
    }

    #[test]
    fn conflicting_group_definitions_error_names_both_locations() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             folder:\n\
             \x20 a:\n\
             \x20   display_name: A\n\
             \x20   cloud_identity_group:\n\
             \x20     ops:\n\
             \x20       display_name: Ops Team\n\
             \x20 b:\n\
             \x20   display_name: B\n\
             \x20   cloud_identity_group:\n\
             \x20     ops:\n\
             \x20       display_name: Operations\n"
        );
        let err = transpile_yaml(&yaml).expect_err("conflicting bodies must not merge");
        let msg = err.to_string();
        assert!(msg.contains("'ops'"), "names the group: {msg}");
        assert!(
            msg.contains("folder") && msg.matches("folder").count() >= 2,
            "names both locations: {msg}"
        );
    }

    /// The same grant written at root and inside a folder is one grant.
    #[test]
    fn identical_grant_at_root_and_in_folder_dedups() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             organization_iam_member:\n\
             \x20 \"user:a@example.com\":\n\
             \x20   - roles/viewer\n\
             folder:\n\
             \x20 x:\n\
             \x20   display_name: X\n\
             \x20   organization_iam_member:\n\
             \x20     \"user:a@example.com\":\n\
             \x20       - roles/viewer\n"
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        assert_eq!(
            project.main_tf.matches("google_organization_iam_member").count(),
            1,
            "one resource, not two with the same label:\n{}",
            project.main_tf
        );
    }

    /// Emission must not depend on HashMap iteration or folder walk order.
    #[test]
    fn hoisted_output_is_deterministic() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             organization_iam_member:\n\
             \x20 \"user:c@example.com\": [roles/viewer]\n\
             \x20 \"user:a@example.com\": [roles/editor, roles/viewer]\n\
             \x20 \"user:b@example.com\": [roles/browser]\n"
        );
        let first = transpile_yaml(&yaml).expect("transpiles").main_tf;
        for _ in 0..4 {
            assert_eq!(first, transpile_yaml(&yaml).expect("transpiles").main_tf);
        }
    }

    /// "Highlander" resources (GCP allows exactly one org audit config per service, one
    /// sink per name, ...) get included from several fragments. Identical definitions
    /// mean "this resource should exist" and collapse to one emission; conflicting ones
    /// abort — attribute merging would only hand the conflict one recursion level down.
    #[test]
    fn identical_duplicate_resources_emit_once() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             folder:\n\
             \x20 a:\n\
             \x20   display_name: A\n\
             \x20   google_organization_iam_audit_config:\n\
             \x20     org_all_services:\n\
             \x20       org_id: \"123456789\"\n\
             \x20       service: allServices\n\
             \x20 b:\n\
             \x20   display_name: B\n\
             \x20   google_organization_iam_audit_config:\n\
             \x20     org_all_services:\n\
             \x20       org_id: \"123456789\"\n\
             \x20       service: allServices\n"
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        assert_eq!(
            project.main_tf.matches("org_all_services").count(),
            1,
            "identical duplicate must collapse:\n{}",
            project.main_tf
        );
    }

    #[test]
    fn conflicting_duplicate_resources_abort_with_the_address() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             folder:\n\
             \x20 a:\n\
             \x20   display_name: A\n\
             \x20   google_organization_iam_audit_config:\n\
             \x20     org_all_services:\n\
             \x20       org_id: \"123456789\"\n\
             \x20       service: allServices\n\
             \x20 b:\n\
             \x20   display_name: B\n\
             \x20   google_organization_iam_audit_config:\n\
             \x20     org_all_services:\n\
             \x20       org_id: \"123456789\"\n\
             \x20       service: storage.googleapis.com\n"
        );
        let err = transpile_yaml(&yaml).expect_err("conflicting bodies must not merge");
        let msg = err.to_string();
        assert!(
            msg.contains("google_organization_iam_audit_config.org_all_services"),
            "names the address: {msg}"
        );
        assert!(msg.contains("allServices"), "shows the difference: {msg}");
    }

    /// Billing IAM is billing-account-scoped like groups are customer-scoped: declared
    /// inside a folder/project it must hoist, not emit an empty `id = ""` in place
    /// (which is what happened before it joined the scope table).
    #[test]
    fn billing_iam_hoists_from_nested_blocks_with_variable_fallback() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             folder:\n\
             \x20 finance:\n\
             \x20   display_name: Finance\n\
             \x20   google_billing_account_iam_member:\n\
             \x20     \"group:billing@example.com\":\n\
             \x20       - roles/billing.admin\n"
        );
        let config: Config = serde_yaml::from_str(&yaml).unwrap();
        let mut vars = HashMap::new();
        vars.insert(
            "billing-account-infra".to_string(),
            serde_yaml::Value::String("01ABCD-234567-89EFGH".to_string()),
        );
        let transpiler = Transpiler::new(
            &config,
            None,
            vec!["google_project_service".to_string(), ".*_iam_member".to_string()],
            "none".to_string(),
            vars,
            HashMap::new(),
            HashMap::new(),
        );
        let project = transpiler.transpile().expect("transpiles");
        assert!(
            project.main_tf.contains("billing_account_id = \"01ABCD-234567-89EFGH\""),
            "{}",
            project.main_tf
        );
        assert!(!project.main_tf.contains("id = \"\""), "{}", project.main_tf);
    }

    #[test]
    fn billing_iam_root_and_nested_merge_and_dedup() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             google_billing_account_iam_member:\n\
             \x20 billing_account_id: \"01ABCD-234567-89EFGH\"\n\
             \x20 \"group:billing@example.com\":\n\
             \x20   - roles/billing.admin\n\
             folder:\n\
             \x20 finance:\n\
             \x20   display_name: Finance\n\
             \x20   google_billing_account_iam_member:\n\
             \x20     \"group:billing@example.com\":\n\
             \x20       - roles/billing.admin\n\
             \x20       - roles/billing.viewer\n"
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        assert_eq!(
            project.main_tf.matches("roles/billing.admin").count(),
            1,
            "shared grant dedups:\n{}",
            project.main_tf
        );
        assert_eq!(project.main_tf.matches("roles/billing.viewer").count(), 1);
        assert!(project.main_tf.contains("billing_account_id = \"01ABCD-234567-89EFGH\""));
    }

    #[test]
    fn conflicting_explicit_billing_account_ids_abort() {
        let yaml = format!(
            "{MINIMAL_TERRAFORM}\n{HOIST_VARS}\
             google_billing_account_iam_member:\n\
             \x20 billing_account_id: \"AAAA\"\n\
             \x20 \"group:a@example.com\": [roles/billing.admin]\n\
             folder:\n\
             \x20 finance:\n\
             \x20   display_name: Finance\n\
             \x20   google_billing_account_iam_member:\n\
             \x20     billing_account_id: \"BBBB\"\n\
             \x20     \"group:b@example.com\": [roles/billing.viewer]\n"
        );
        let err = transpile_yaml(&yaml).expect_err("two billing accounts must not merge");
        let msg = err.to_string();
        assert!(msg.contains("AAAA") && msg.contains("BBBB"), "{msg}");
    }

    /// The monitoring presets embed quoted metric types around a live `${...}` reference:
    /// `metric.type="logging.googleapis.com/user/${google_logging_metric.x.name}"`.
    /// The literal quotes must be escaped in the emitted HCL while the interpolation
    /// stays live — unescaped they truncate the attribute string (invalid HCL).
    #[test]
    fn template_strings_escape_literal_quotes_but_keep_interpolation() {
        let s = "metric.type=\"logging.googleapis.com/user/${google_logging_metric.m.name}\"";
        let expr = Transpiler::string_to_hcl_expr(s);
        let body = hcl::Body::builder()
            .add_attribute(("filter", expr))
            .build();
        let out = hcl::to_string(&body).unwrap();
        assert_eq!(
            out.trim(),
            "filter = \"metric.type=\\\"logging.googleapis.com/user/${google_logging_metric.m.name}\\\"\"",
            "emitted: {out}"
        );
        // …and the result must round-trip through an HCL parser.
        assert!(hcl::parse(&out).is_ok(), "not valid HCL: {out}");
    }

    #[test]
    fn escape_template_literals_edge_cases() {
        // Quotes inside the interpolation are expression context — untouched.
        assert_eq!(
            escape_template_literals("${replace(x, \"a\", \"b\")} \"lit\""),
            "${replace(x, \"a\", \"b\")} \\\"lit\\\""
        );
        // $${ is the literal escape sequence, not an interpolation start.
        assert_eq!(escape_template_literals("$${not} \"q\""), "$${not} \\\"q\\\"");
        // Backslashes are escaped too.
        assert_eq!(escape_template_literals("a\\b ${x}"), "a\\\\b ${x}");
    }

    /// The membership label is a hash, so the importer can only find the resource if it
    /// computes the identical string. Assert against real generated HCL, not the helper.
    #[test]
    fn membership_label_helper_matches_emitted_hcl() {
        let yaml = format!(
            "{}\ncustomer-organization-id: \"123456789\"\ncustomer-domain: example.com\n\
             customer-id: C01234567\ncloud_identity_group:\n  gcp-billing-admins:\n    \
             display_name: B\n    member:\n      - \"user:a@example.com\"\n",
            MINIMAL_TERRAFORM
        );
        let project = transpile_yaml(&yaml).expect("transpiles");
        let expected = membership_resource_label("gcp-billing-admins", "user:a@example.com");
        assert!(
            project.main_tf.contains(&format!(
                "resource \"google_cloud_identity_group_membership\" \"{}\"",
                expected
            )),
            "expected label {expected} in:\n{}",
            project.main_tf
        );
    }

    #[test]
    fn membership_output_order_is_stable() {
        // Was a HashMap, so block order changed between runs of the same binary; state
        // adoption keys off these labels, so churn here is not cosmetic.
        let yaml = format!(
            "{}\ncustomer-organization-id: \"123456789\"\ncustomer-domain: example.com\n\
             customer-id: C01234567\ncloud_identity_group:\n  g:\n    display_name: G\n    \
             member:\n      - \"user:a@example.com\"\n      - \"user:b@example.com\"\n      \
             - \"user:c@example.com\"\n",
            MINIMAL_TERRAFORM
        );
        let first = transpile_yaml(&yaml).expect("transpiles").main_tf;
        for _ in 0..4 {
            assert_eq!(first, transpile_yaml(&yaml).expect("transpiles").main_tf);
        }
    }

    /// Transpile a root-level YAML document, i.e. the shape `merge_variables` produces
    /// after variables have been promoted to the root.
    fn transpile_yaml(yaml: &str) -> Result<GeneratedProject, Box<dyn std::error::Error>> {
        let config: Config = serde_yaml::from_str(yaml)?;
        let transpiler = Transpiler::new(
            &config,
            None,
            // Mirror ToolConfig's default_auto_explode so tests see production routing
            // (IAM maps explode into per-grant resources instead of being skipped).
            vec!["google_project_service".to_string(), ".*_iam_member".to_string()],
            "none".to_string(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );
        transpiler.transpile()
    }

    const MINIMAL_TERRAFORM: &str = r#"
terraform:
  backend:
    local:
      path: "terraform.tfstate"
"#;

    #[test]
    fn missing_customer_organization_id_is_an_error_not_a_panic() {
        // Previously `panic!`, which aborts outright under the release profile's
        // panic = "abort" instead of printing a usable message.
        let err = transpile_yaml(MINIMAL_TERRAFORM)
            .expect_err("a config without customer-organization-id must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("customer-organization-id"),
            "the error should name the missing key: {msg}"
        );
    }

    #[test]
    fn org_policy_without_name_is_an_error_not_a_panic() {
        // Previously `.expect(...)`. Reachable from any preset or hand-written config.
        let yaml = format!(
            "{MINIMAL_TERRAFORM}
customer-organization-id: \"123456789012\"
org_policy_policy:
  my_broken_policy:
    spec:
      rules:
        - enforce: true
"
        );
        let err = transpile_yaml(&yaml).expect_err("an org policy without 'name' must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("my_broken_policy"), "the error should name the offending policy: {msg}");
        assert!(msg.contains("name"), "the error should name the missing attribute: {msg}");
    }

    #[test]
    fn minimal_valid_config_transpiles() {
        // Guards the error paths above against over-reach.
        let yaml = format!("{MINIMAL_TERRAFORM}\ncustomer-organization-id: \"123456789012\"\n");
        let out = transpile_yaml(&yaml).expect("a minimal valid config should transpile");
        assert!(out.providers_tf.contains("terraform"), "expected a terraform block:\n{}", out.providers_tf);
    }

    #[test]
    fn interpolated_strings_become_templates_not_escaped_literals() {
        // hcl-rs serialises Expression::String by escaping `${` to `$${`, which turns a
        // Terraform reference into dead text. The helper must route interpolations into
        // a TemplateExpr instead — and leave plain strings alone.
        let tpl = Transpiler::string_to_hcl_expr("serviceAccount:${google_service_account.x.email}");
        assert!(matches!(tpl, hcl::Expression::TemplateExpr(_)), "interpolation must be a template");

        let plain = Transpiler::string_to_hcl_expr("roles/viewer");
        assert!(matches!(plain, hcl::Expression::String(_)), "plain strings stay literal");

        let directive = Transpiler::string_to_hcl_expr("%{ if x }a%{ endif }");
        assert!(matches!(directive, hcl::Expression::TemplateExpr(_)), "%{{}} directives too");
    }

    #[test]
    fn explicit_provider_renders_as_reference_not_quoted_string() {
        // provider = "google-beta" is the pre-0.12 legacy form; tofu warns about it on
        // every plan. A user-specified provider must come out as an unquoted reference.
        let yaml = format!(
            "{MINIMAL_TERRAFORM}
customer-organization-id: \"123456789012\"
google_project_service_identity:
  pubsub:
    provider: google-beta
    service: pubsub.googleapis.com
"
        );
        let out = transpile_yaml(&yaml).expect("transpiles");
        assert!(
            out.main_tf.contains("provider = google-beta"),
            "provider must be an unquoted reference:\n{}",
            out.main_tf
        );
        assert!(
            !out.main_tf.contains(r#"provider = "google-beta""#),
            "quoted legacy form must be gone:\n{}",
            out.main_tf
        );
    }

    #[test]
    fn provider_ref_expr_handles_aliases_and_falls_back_safely() {
        // Aliased reference: google.google-beta -> traversal.
        let aliased = crate::emit_shared::provider_ref_expr("google.google-beta");
        assert!(matches!(aliased, hcl::Expression::Traversal(_)));
        // Not a valid identifier: keep the (quoted) string rather than emit broken HCL.
        let odd = crate::emit_shared::provider_ref_expr("weird provider!");
        assert!(matches!(odd, hcl::Expression::String(_)));
    }

    #[test]
    fn expr_tag_resolves_through_the_full_pipeline() {
        // `!expr a.b.c` must survive as a KEY: it resolves to "${a.b.c}", deserializes
        // into Config as a plain string key, and the exploded IAM member renders as a
        // template. Previously the Tagged value reached serde and broke deserialization
        // outright ("untagged and internally tagged enums do not support enum input").
        let yaml = format!(
            "{MINIMAL_TERRAFORM}
customer-organization-id: \"123456789012\"
google_organization_iam_member:
  !expr 'google_project_service_identity.pubsub.member':
    - roles/viewer
"
        );
        let raw: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("parses");
        let resolved = crate::resolve_yaml_custom_tags(crate::merge_variables(raw));
        let config: Config = serde_yaml::from_value(resolved)
            .expect("!expr key must deserialize after tag resolution");
        let transpiler = Transpiler::new(
            &config, None, Vec::new(), "none".to_string(),
            HashMap::new(), HashMap::new(), HashMap::new(),
        );
        let out = transpiler.transpile().expect("transpiles");
        assert!(
            out.main_tf.contains(r#"member = "${google_project_service_identity.pubsub.member}""#),
            "member must render as a template reference:\n{}",
            out.main_tf
        );
        assert!(!out.main_tf.contains("$${"), "no escaped interpolation:\n{}", out.main_tf);
    }

    #[test]
    fn expr_tag_wraps_bare_expressions_and_passes_interpolations_through() {
        let resolve = |s: &str| {
            let v: serde_yaml::Value = serde_yaml::from_str(s).unwrap();
            crate::resolve_yaml_custom_tags(v)
        };
        assert_eq!(
            resolve("!expr google_service_account.x.email"),
            serde_yaml::Value::String("${google_service_account.x.email}".to_string())
        );
        // Already-interpolated input must not be double-wrapped.
        assert_eq!(
            resolve("!expr 'prefix-${var.name}'"),
            serde_yaml::Value::String("prefix-${var.name}".to_string())
        );
    }

    #[test]
    fn iam_member_with_reference_renders_unescaped() {
        // End-to-end pin of the silent-bug class: the transpile succeeds either way, and
        // only the generated HCL reveals whether the reference survived.
        let yaml = format!(
            "{MINIMAL_TERRAFORM}
customer-organization-id: \"123456789012\"
google_organization_iam_member:
  serviceAccount:${{google_service_account.provisioner.email}}:
    - roles/viewer
"
        );
        let out = transpile_yaml(&yaml).expect("config with interpolated member should transpile");
        assert!(
            out.main_tf.contains(r#"member = "serviceAccount:${google_service_account.provisioner.email}""#),
            "member must keep its interpolation:\n{}",
            out.main_tf
        );
        assert!(!out.main_tf.contains("$${"), "no escaped interpolation anywhere:\n{}", out.main_tf);
        // The generated label must not leak interpolation syntax.
        assert!(
            out.main_tf.contains("iam_serviceAccount_google_service_account_provisioner_email_"),
            "label should be sanitized:\n{}",
            out.main_tf
        );
    }
}
