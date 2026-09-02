//! Recursive org-policy audit: sweep the resource hierarchy (organization, folders,
//! projects) with their DECLARED org policies via Cloud Asset Inventory, and classify
//! each node-level policy against a framework baseline.
//!
//! Design constraints (see the flat diff in `org_policy.rs` for the counterpart):
//! - One CAI `list_assets` call fetches tree and policies together, so every
//!   classification comes from a single consistent snapshot. CAI can lag live changes
//!   by minutes; renderers carry that caveat.
//! - Policies are requested as `orgpolicy.googleapis.com/Policy` RESOURCE assets, whose
//!   `resource.data` is the raw v2 REST JSON (`spec.rules[]`, `dryRunSpec`). The SDK's
//!   `ContentType::OrgPolicy` is deliberately NOT used — it returns the legacy v1 shape
//!   (listPolicy/booleanPolicy) which cannot express rules, conditions or dry-run.
//! - Declared policies only: a node that inherits the baseline unchanged is invisible
//!   here by design. There is no effective-policy computation.
//! - Classify-only: node entries carry no planned action; whether a divergence is a
//!   sanctioned exception is the reader's call.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::org_policy::{canonical_policy, constraint_name, Classification, DesiredPolicy};

type BoxErr = Box<dyn std::error::Error>;

const CRM_ORGANIZATION: &str = "cloudresourcemanager.googleapis.com/Organization";
const CRM_FOLDER: &str = "cloudresourcemanager.googleapis.com/Folder";
const CRM_PROJECT: &str = "cloudresourcemanager.googleapis.com/Project";
const ORGPOLICY_POLICY: &str = "orgpolicy.googleapis.com/Policy";

// ---------------------------------------------------------------------------
// Sweep input model (IO-free seam for tests)
// ---------------------------------------------------------------------------

/// The subset of a CAI asset the tree needs, decoupled from the SDK type so assembly
/// and classification are testable without a network.
#[derive(Debug, Clone)]
pub struct RawAsset {
    /// Full asset name, e.g. `//cloudresourcemanager.googleapis.com/folders/456` or
    /// `//orgpolicy.googleapis.com/folders/456/policies/compute.requireOsLogin`.
    pub name: String,
    pub asset_type: String,
    /// Ancestry from the asset itself up to the org root, e.g.
    /// `["folders/456", "folders/123", "organizations/1"]`. Projects appear by NUMBER.
    pub ancestors: Vec<String>,
    /// `resource.data`: CRM node JSON, or the v2 policy JSON for policy assets.
    pub data: Value,
}

// ---------------------------------------------------------------------------
// Tree model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Organization,
    Folder,
    Project,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Canonical id: `organizations/N` | `folders/N` | `projects/<projectId>`.
    pub id: String,
    pub kind: NodeKind,
    /// Organization: domain; folder: display name; project: projectId.
    pub display_name: String,
    pub parent: Option<String>,
    /// Sorted: folders first, then projects, each by display name then id.
    pub children: Vec<String>,
    /// Bare constraint name -> raw v2 policy JSON as declared on this node.
    pub policies: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct PolicyTree {
    pub root: String,
    pub nodes: BTreeMap<String, TreeNode>,
    /// Non-fatal oddities: policies on unknown nodes, unresolvable project numbers.
    pub warnings: Vec<String>,
}

impl PolicyTree {
    /// Display-name path from the root to `id`, inclusive.
    pub fn path(&self, id: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut cursor = Some(id.to_string());
        while let Some(current) = cursor {
            match self.nodes.get(&current) {
                Some(node) => {
                    segments.push(node.display_name.clone());
                    cursor = node.parent.clone();
                }
                None => break,
            }
        }
        segments.reverse();
        segments
    }
}

// ---------------------------------------------------------------------------
// Report model (serialized inside DiffReport for --format json)
// ---------------------------------------------------------------------------

/// Value-level difference between a node's declared list policy and the baseline.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct OverrideDelta {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_allowed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_allowed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_denied: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_denied: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl OverrideDelta {
    fn is_empty(&self) -> bool {
        self.added_allowed.is_empty()
            && self.removed_allowed.is_empty()
            && self.added_denied.is_empty()
            && self.removed_denied.is_empty()
            && self.notes.is_empty()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeEntry {
    pub constraint: String,
    pub classification: Classification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<OverrideDelta>,
    /// Canonicalized declared policy on this node.
    pub declared_spec: Value,
    /// Canonicalized framework baseline, when the constraint is part of the framework.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_spec: Option<Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeReport {
    pub node: String,
    pub kind: NodeKind,
    pub display_name: String,
    /// Display names from the root down to this node.
    pub path: Vec<String>,
    pub entries: Vec<NodeEntry>,
}

/// A maximal subtree in which no node declares any policy — collapsed in rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CleanSubtree {
    pub under: String,
    pub path: Vec<String>,
    pub folders: usize,
    pub projects: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TreeSummary {
    pub total_folders: usize,
    pub total_projects: usize,
    pub nodes_with_overrides: usize,
    /// classification label -> count across all node entries.
    pub counts: BTreeMap<String, usize>,
    pub clean_subtrees: Vec<CleanSubtree>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Sweep (the module's only IO)
// ---------------------------------------------------------------------------

/// Fetch the resource hierarchy and every declared org policy in ONE paginated
/// `list_assets` call. `org` may be `organizations/N` or a bare numeric id.
pub async fn sweep(org: &str, quota_project: Option<String>) -> Result<Vec<RawAsset>, BoxErr> {
    use google_cloud_asset_v1::model::ContentType;
    use google_cloud_gax::options::RequestOptionsBuilder;
    use google_cloud_gax::paginator::ItemPaginator;

    let org_id = org.trim_start_matches("organizations/");
    let client = crate::gcp::asset_service().await?;

    let mut builder = client
        .list_assets()
        .set_parent(format!("organizations/{}", org_id))
        .set_asset_types(vec![
            CRM_ORGANIZATION.to_string(),
            CRM_FOLDER.to_string(),
            CRM_PROJECT.to_string(),
            ORGPOLICY_POLICY.to_string(),
        ])
        .set_content_type(ContentType::Resource)
        .set_page_size(1000);
    if let Some(qp) = quota_project {
        builder = builder.with_quota_project(qp);
    }

    let mut stream = builder.by_item();
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(asset) => out.push(RawAsset {
                name: asset.name.clone(),
                asset_type: asset.asset_type.clone(),
                ancestors: asset.ancestors.clone(),
                data: asset
                    .resource
                    .as_ref()
                    .and_then(|r| r.data.clone())
                    .map(Value::Object)
                    .unwrap_or(Value::Null),
            }),
            // Unlike discovery (best-effort config generation), a partial sweep here
            // would produce a health check that silently omits part of the tree.
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("PERMISSION_DENIED") || msg.contains("403") {
                    return Err(format!(
                        "recursive audit requires roles/cloudasset.viewer on organizations/{} \
                         (separate from roles/orgpolicy.policyViewer used by the flat diff): {}",
                        org_id, msg
                    )
                    .into());
                }
                return Err(format!("Cloud Asset Inventory sweep failed: {}", msg).into());
            }
        }
    }
    Ok(out)
}

/// Shared entry point for `diff --recursive` and `report --recursive`.
pub async fn sweep_and_assemble(org: &str) -> Result<PolicyTree, BoxErr> {
    let assets = sweep(org, crate::org_policy::resolve_quota_project()).await?;
    assemble_tree(org, assets)
}

// ---------------------------------------------------------------------------
// Tree assembly (pure)
// ---------------------------------------------------------------------------

/// Strip the `//<service>/` prefix from a full asset name.
fn relative_name(name: &str) -> &str {
    for marker in ["organizations/", "folders/", "projects/"] {
        if let Some(idx) = name.find(marker) {
            return &name[idx..];
        }
    }
    name
}

pub fn assemble_tree(org: &str, assets: Vec<RawAsset>) -> Result<PolicyTree, BoxErr> {
    let org_id = org.trim_start_matches("organizations/");
    let root_id = format!("organizations/{}", org_id);
    let mut nodes: BTreeMap<String, TreeNode> = BTreeMap::new();
    let mut number_to_project_id: HashMap<String, String> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    // Pass 1: nodes.
    for asset in &assets {
        match asset.asset_type.as_str() {
            CRM_ORGANIZATION => {
                let id = relative_name(&asset.name).to_string();
                let display = asset
                    .data
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                nodes.insert(id.clone(), TreeNode {
                    id,
                    kind: NodeKind::Organization,
                    display_name: display,
                    parent: None,
                    children: Vec::new(),
                    policies: BTreeMap::new(),
                });
            }
            CRM_FOLDER => {
                let id = relative_name(&asset.name).to_string();
                let display = asset
                    .data
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let parent = asset.ancestors.get(1).cloned();
                nodes.insert(id.clone(), TreeNode {
                    id,
                    kind: NodeKind::Folder,
                    display_name: display,
                    parent,
                    children: Vec::new(),
                    policies: BTreeMap::new(),
                });
            }
            CRM_PROJECT => {
                // Canonical id uses projectId; ancestors and policy scopes use the
                // number, so record the mapping for pass 2.
                let project_id = match asset.data.get("projectId").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        warnings.push(format!(
                            "project asset '{}' has no projectId; skipped",
                            asset.name
                        ));
                        continue;
                    }
                };
                let number = asset
                    .data
                    .get("projectNumber")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .or_else(|| {
                        asset
                            .ancestors
                            .first()
                            .map(|a| a.trim_start_matches("projects/").to_string())
                    });
                let id = format!("projects/{}", project_id);
                if let Some(num) = number {
                    number_to_project_id.insert(num, project_id.clone());
                }
                let parent = asset.ancestors.get(1).cloned();
                nodes.insert(id.clone(), TreeNode {
                    id,
                    kind: NodeKind::Project,
                    display_name: project_id,
                    parent,
                    children: Vec::new(),
                    policies: BTreeMap::new(),
                });
            }
            _ => {}
        }
    }

    // The org asset itself can be absent (e.g. no resourcemanager.organizations.get);
    // synthesize the root so the tree still hangs together.
    if !nodes.contains_key(&root_id) {
        warnings.push(format!(
            "organization asset for {} not returned; using the bare id as its display name",
            root_id
        ));
        nodes.insert(root_id.clone(), TreeNode {
            id: root_id.clone(),
            kind: NodeKind::Organization,
            display_name: root_id.clone(),
            parent: None,
            children: Vec::new(),
            policies: BTreeMap::new(),
        });
    }

    // Pass 2: attach policies. Scope comes from the asset name's prefix; project scopes
    // arrive by number and are normalized through the map built in pass 1.
    for asset in &assets {
        if asset.asset_type != ORGPOLICY_POLICY {
            continue;
        }
        let rel = relative_name(&asset.name);
        let Some((scope_raw, _)) = rel.split_once("/policies/") else {
            warnings.push(format!("unparseable policy asset name '{}'", asset.name));
            continue;
        };
        let scope = if let Some(num) = scope_raw.strip_prefix("projects/") {
            if num.chars().all(|c| c.is_ascii_digit()) {
                match number_to_project_id.get(num) {
                    Some(pid) => format!("projects/{}", pid),
                    None => scope_raw.to_string(),
                }
            } else {
                scope_raw.to_string()
            }
        } else {
            scope_raw.to_string()
        };

        let constraint = constraint_name(rel);
        match nodes.get_mut(&scope) {
            Some(node) => {
                node.policies.insert(constraint, asset.data.clone());
            }
            None => warnings.push(format!(
                "policy '{}' is declared on '{}', which is not part of the visible tree",
                constraint, scope
            )),
        }
    }

    // Children, deterministic: folders first, then projects, by display name then id.
    let mut by_parent: HashMap<String, Vec<String>> = HashMap::new();
    for node in nodes.values() {
        if let Some(parent) = &node.parent {
            by_parent.entry(parent.clone()).or_default().push(node.id.clone());
        }
    }
    for (parent, mut kids) in by_parent {
        kids.sort_by(|a, b| {
            let (na, nb) = (&nodes[a], &nodes[b]);
            let rank = |k: NodeKind| if k == NodeKind::Folder { 0 } else { 1 };
            rank(na.kind)
                .cmp(&rank(nb.kind))
                .then_with(|| na.display_name.cmp(&nb.display_name))
                .then_with(|| na.id.cmp(&nb.id))
        });
        match nodes.get_mut(&parent) {
            Some(p) => p.children = kids,
            // the parent asset was not returned (no permission on it, or a
            // folder outside the sweep): the children would otherwise vanish
            // from every renderer while still being counted
            None => {
                warnings.push(format!(
                    "{} node(s) under '{}' are attached to the root: their parent is not part of the visible tree",
                    kids.len(),
                    parent
                ));
                if let Some(root) = nodes.get_mut(&root_id) {
                    root.children.extend(kids);
                }
            }
        }
    }

    Ok(PolicyTree { root: root_id, nodes, warnings })
}

// ---------------------------------------------------------------------------
// Classification (pure)
// ---------------------------------------------------------------------------

/// The comparable single-rule form of a canonicalized policy's `spec`, when it has one.
#[derive(Debug, PartialEq)]
enum RuleForm {
    Enforce(bool),
    AllowAll,
    DenyAll,
    Values { allowed: Vec<String>, denied: Vec<String> },
}

/// Extract the single-rule form of `canon["spec"]`. `None` when there is no spec, more
/// than one rule, or the rule carries a condition (conditional rules have no ordering).
fn single_rule_form(canon: &Value) -> Option<RuleForm> {
    let rules = canon.get("spec")?.get("rules")?.as_array()?;
    if rules.len() != 1 {
        return None;
    }
    let rule = rules[0].as_object()?;
    if rule.contains_key("condition") {
        return None;
    }
    if let Some(b) = rule.get("enforce").and_then(|v| v.as_bool()) {
        return Some(RuleForm::Enforce(b));
    }
    if rule.get("allow_all").and_then(|v| v.as_bool()) == Some(true) {
        return Some(RuleForm::AllowAll);
    }
    if rule.get("deny_all").and_then(|v| v.as_bool()) == Some(true) {
        return Some(RuleForm::DenyAll);
    }
    if let Some(values) = rule.get("values").and_then(|v| v.as_object()) {
        let list = |key: &str| -> Vec<String> {
            values
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        return Some(RuleForm::Values { allowed: list("allowed_values"), denied: list("denied_values") });
    }
    None
}

fn spec_flag(canon: &Value, key: &str) -> bool {
    canon
        .get("spec")
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn set_difference(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

/// Classify a policy DECLARED on a folder/project against the framework's baseline for
/// the same constraint. Both sides are canonicalized here; callers pass raw JSON.
pub fn classify_override(
    baseline: Option<&DesiredPolicy>,
    node_policy: &Value,
) -> (Classification, Option<OverrideDelta>) {
    let node_canon = canonical_policy(node_policy);
    let mut delta = OverrideDelta::default();

    if spec_flag(&node_canon, "inherit_from_parent") {
        delta.notes.push("inherits_from_parent: rules merge with the parent's".to_string());
    }

    let Some(base) = baseline else {
        if spec_flag(&node_canon, "reset") {
            delta.notes.push("reset: reverts to the constraint default".to_string());
        }
        let d = if delta.is_empty() { None } else { Some(delta) };
        return (Classification::NodeOnly, d);
    };

    if spec_flag(&node_canon, "reset") {
        delta
            .notes
            .push("reset: reverts to the constraint default, discarding the baseline".to_string());
        return (Classification::NodeReset, Some(delta));
    }

    let base_canon = canonical_policy(&base.policy);
    if base_canon == node_canon {
        let d = if delta.is_empty() { None } else { Some(delta) };
        return (Classification::OverrideMatchesBaseline, d);
    }

    match (single_rule_form(&base_canon), single_rule_form(&node_canon)) {
        (Some(RuleForm::Enforce(b)), Some(RuleForm::Enforce(n))) if b != n => {
            delta.notes.push(format!("enforce {}→{}", b, n));
            let class = if b && !n {
                Classification::OverrideWeaker
            } else {
                Classification::OverrideStronger
            };
            (class, Some(delta))
        }
        (Some(RuleForm::DenyAll), Some(RuleForm::AllowAll)) => {
            delta.notes.push("deny_all→allow_all".to_string());
            (Classification::OverrideWeaker, Some(delta))
        }
        (Some(RuleForm::AllowAll), Some(RuleForm::DenyAll)) => {
            delta.notes.push("allow_all→deny_all".to_string());
            (Classification::OverrideStronger, Some(delta))
        }
        (
            Some(RuleForm::Values { allowed: ba, denied: bd }),
            Some(RuleForm::Values { allowed: na, denied: nd }),
        ) => {
            delta.added_allowed = set_difference(&na, &ba);
            delta.removed_allowed = set_difference(&ba, &na);
            delta.added_denied = set_difference(&nd, &bd);
            delta.removed_denied = set_difference(&bd, &nd);
            if delta.added_allowed.is_empty()
                && delta.removed_allowed.is_empty()
                && delta.added_denied.is_empty()
                && delta.removed_denied.is_empty()
            {
                // Same list values — the difference is elsewhere (dry-run, inherit).
                delta.notes.push("list values equal; differs in spec metadata".to_string());
            }
            (Classification::OverrideDivergent, Some(delta))
        }
        _ => {
            delta.notes.push(
                "no boolean/list ordering (conditions, parameters, dry-run or multi-rule differences)"
                    .to_string(),
            );
            (Classification::OverrideDivergent, Some(delta))
        }
    }
}

/// Walk the tree (root excluded — the root is the flat diff's job) and classify every
/// declared policy. Returns override reports for policy-bearing nodes plus a summary
/// with maximal clean subtrees for collapsed rendering.
pub fn classify_tree(
    tree: &PolicyTree,
    baseline: &BTreeMap<String, DesiredPolicy>,
) -> (Vec<NodeReport>, TreeSummary) {
    let mut reports = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut clean_subtrees = Vec::new();
    let mut total_folders = 0usize;
    let mut total_projects = 0usize;

    for node in tree.nodes.values() {
        match node.kind {
            NodeKind::Folder => total_folders += 1,
            NodeKind::Project => total_projects += 1,
            NodeKind::Organization => {}
        }
    }

    // subtree_has_policies, computed bottom-up. Every node must end up memoized — the
    // DFS below reads the memo for each child — so evaluate children eagerly rather
    // than letting `||`/`any` short-circuit past them.
    fn subtree_dirty(tree: &PolicyTree, id: &str, memo: &mut HashMap<String, bool>) -> bool {
        if let Some(v) = memo.get(id) {
            return *v;
        }
        let node = &tree.nodes[id];
        let children_dirty: Vec<bool> = node
            .children
            .iter()
            .map(|c| subtree_dirty(tree, c, memo))
            .collect();
        let dirty = !node.policies.is_empty() || children_dirty.into_iter().any(|d| d);
        memo.insert(id.to_string(), dirty);
        dirty
    }
    let mut memo = HashMap::new();
    subtree_dirty(tree, &tree.root, &mut memo);

    fn subtree_counts(tree: &PolicyTree, id: &str) -> (usize, usize) {
        let node = &tree.nodes[id];
        let mut folders = usize::from(node.kind == NodeKind::Folder);
        let mut projects = usize::from(node.kind == NodeKind::Project);
        for child in &node.children {
            let (f, p) = subtree_counts(tree, child);
            folders += f;
            projects += p;
        }
        (folders, projects)
    }

    // DFS in child order for deterministic, hierarchy-shaped output.
    let mut stack: Vec<String> = tree.nodes[&tree.root]
        .children
        .iter()
        .rev()
        .cloned()
        .collect();
    while let Some(id) = stack.pop() {
        let node = &tree.nodes[&id];
        if !memo.get(&id).copied().unwrap_or(false) {
            // Maximal clean subtree: parent is dirty (or root), this whole subtree is not.
            let (folders, projects) = subtree_counts(tree, &id);
            clean_subtrees.push(CleanSubtree {
                under: id.clone(),
                path: tree.path(&id),
                folders,
                projects,
            });
            continue; // nothing below can carry a policy
        }
        if !node.policies.is_empty() {
            let entries: Vec<NodeEntry> = node
                .policies
                .iter()
                .map(|(constraint, policy)| {
                    let (classification, delta) =
                        classify_override(baseline.get(constraint), policy);
                    *counts
                        .entry(crate::org_policy::classification_label(&classification).to_string())
                        .or_insert(0) += 1;
                    NodeEntry {
                        constraint: constraint.clone(),
                        classification,
                        delta,
                        declared_spec: canonical_policy(policy),
                        baseline_spec: baseline
                            .get(constraint)
                            .map(|b| canonical_policy(&b.policy)),
                    }
                })
                .collect();
            reports.push(NodeReport {
                node: node.id.clone(),
                kind: node.kind,
                display_name: node.display_name.clone(),
                path: tree.path(&id),
                entries,
            });
        }
        for child in node.children.iter().rev() {
            stack.push(child.clone());
        }
    }

    let summary = TreeSummary {
        total_folders,
        total_projects,
        nodes_with_overrides: reports.len(),
        counts,
        clean_subtrees,
        warnings: tree.warnings.clone(),
    };
    (reports, summary)
}

// ---------------------------------------------------------------------------
// Rendering (pure)
// ---------------------------------------------------------------------------

fn delta_line(delta: &Option<OverrideDelta>) -> String {
    let Some(d) = delta else { return String::new() };
    let mut parts = Vec::new();
    if !d.added_allowed.is_empty() {
        parts.push(format!("+allowed {}", d.added_allowed.join(",")));
    }
    if !d.removed_allowed.is_empty() {
        parts.push(format!("-allowed {}", d.removed_allowed.join(",")));
    }
    if !d.added_denied.is_empty() {
        parts.push(format!("+denied {}", d.added_denied.join(",")));
    }
    if !d.removed_denied.is_empty() {
        parts.push(format!("-denied {}", d.removed_denied.join(",")));
    }
    parts.extend(d.notes.iter().cloned());
    parts.join("; ")
}

/// Console tree appended to the flat diff output in `--recursive` mode. Nodes on a path
/// to an override are expanded; maximal clean subtrees collapse to one line.
pub fn render_console_tree(
    tree: &PolicyTree,
    reports: &[NodeReport],
    summary: &TreeSummary,
) -> String {
    let mut s = String::new();
    s.push_str("\nResource hierarchy overrides (source: Cloud Asset Inventory, may lag live changes):\n");

    let override_nodes: std::collections::HashSet<&str> =
        reports.iter().map(|r| r.node.as_str()).collect();
    let clean_roots: HashMap<&str, &CleanSubtree> =
        summary.clean_subtrees.iter().map(|c| (c.under.as_str(), c)).collect();
    let by_node: HashMap<&str, &NodeReport> =
        reports.iter().map(|r| (r.node.as_str(), r)).collect();

    let root = &tree.nodes[&tree.root];
    s.push_str(&format!("{} ({})\n", root.id, root.display_name));

    /// What every node of the rendered tree is looked up against.
    struct RenderCtx<'a> {
        override_nodes: &'a std::collections::HashSet<&'a str>,
        clean_roots: &'a HashMap<&'a str, &'a CleanSubtree>,
        by_node: &'a HashMap<&'a str, &'a NodeReport>,
    }

    fn walk(
        tree: &PolicyTree,
        id: &str,
        prefix: &str,
        is_last: bool,
        ctx: &RenderCtx<'_>,
        out: &mut String,
    ) {
        let RenderCtx { override_nodes, clean_roots, by_node } = ctx;
        let node = &tree.nodes[id];
        let branch = if is_last { "└─ " } else { "├─ " };
        let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "│  " });

        if let Some(clean) = clean_roots.get(id) {
            let mut counts = Vec::new();
            if clean.folders > 0 {
                counts.push(format!("{} folder{}", clean.folders, if clean.folders == 1 { "" } else { "s" }));
            }
            if clean.projects > 0 {
                counts.push(format!("{} project{}", clean.projects, if clean.projects == 1 { "" } else { "s" }));
            }
            let detail = if counts.is_empty() { String::new() } else { format!(" ({} collapsed)", counts.join(", ")) };
            out.push_str(&format!(
                "{}{}{} ({}) — no overrides{}\n",
                prefix, branch, node.id, node.display_name, detail
            ));
            return;
        }

        out.push_str(&format!("{}{}{} ({})\n", prefix, branch, node.id, node.display_name));
        if override_nodes.contains(id) {
            if let Some(report) = by_node.get(id) {
                for entry in &report.entries {
                    let label = crate::org_policy::classification_label(&entry.classification);
                    let detail = delta_line(&entry.delta);
                    if detail.is_empty() {
                        out.push_str(&format!("{}     {:<44} {}\n", child_prefix, entry.constraint, label));
                    } else {
                        out.push_str(&format!(
                            "{}     {:<44} {:<24} {}\n",
                            child_prefix, entry.constraint, label, detail
                        ));
                    }
                }
            }
        }
        let children = &node.children;
        for (i, child) in children.iter().enumerate() {
            walk(
                tree,
                child,
                &child_prefix,
                i + 1 == children.len(),
                ctx,
                out,
            );
        }
    }

    let children = &root.children;
    for (i, child) in children.iter().enumerate() {
        walk(
            tree,
            child,
            "",
            i + 1 == children.len(),
            &RenderCtx { override_nodes: &override_nodes, clean_roots: &clean_roots, by_node: &by_node },
            &mut s,
        );
    }

    let counts = summary
        .counts
        .iter()
        .map(|(label, n)| format!("{}={}", label, n))
        .collect::<Vec<_>>()
        .join(", ");
    let total = summary.total_folders + summary.total_projects;
    s.push_str(&format!(
        "\nHierarchy summary: {}; {}/{} nodes with declared policies\n",
        if counts.is_empty() { "no declared node policies".to_string() } else { counts },
        summary.nodes_with_overrides,
        total,
    ));
    for w in &summary.warnings {
        s.push_str(&format!("warning: {}\n", w));
    }
    s
}

/// Markdown section appended to the flat diff markdown in `--recursive` mode.
pub fn render_markdown_nodes(reports: &[NodeReport], summary: &TreeSummary) -> String {
    let mut s = String::new();
    s.push_str("\n## Hierarchy overrides\n\n");
    s.push_str("_Source: Cloud Asset Inventory (may lag live changes). Declared policies only._\n\n");
    let total = summary.total_folders + summary.total_projects;
    s.push_str(&format!(
        "{} of {} nodes ({} folders, {} projects) declare policies.\n\n",
        summary.nodes_with_overrides, total, summary.total_folders, summary.total_projects
    ));

    for report in reports {
        s.push_str(&format!("### {} — {}\n\n", report.node, report.display_name));
        s.push_str(&format!("_Path: {}_\n\n", report.path.join(" / ")));
        for entry in &report.entries {
            let label = crate::org_policy::classification_label(&entry.classification);
            s.push_str(&format!("- **`{}`** — {}", entry.constraint, label));
            let detail = delta_line(&entry.delta);
            if !detail.is_empty() {
                s.push_str(&format!(" ({})", detail));
            }
            s.push('\n');
            if let Some(base) = &entry.baseline_spec {
                s.push_str(&format!(
                    "\n  baseline:\n\n  ```json\n{}\n  ```\n",
                    serde_json::to_string_pretty(base).unwrap_or_default()
                ));
            }
            s.push_str(&format!(
                "\n  declared:\n\n  ```json\n{}\n  ```\n\n",
                serde_json::to_string_pretty(&entry.declared_spec).unwrap_or_default()
            ));
        }
    }

    if !summary.clean_subtrees.is_empty() {
        s.push_str("### Clean subtrees\n\n");
        for clean in &summary.clean_subtrees {
            s.push_str(&format!(
                "- `{}` ({}) — no declared policies ({} folders, {} projects)\n",
                clean.under,
                clean.path.join(" / "),
                clean.folders,
                clean.projects
            ));
        }
    }
    for w in &summary.warnings {
        s.push_str(&format!("\n> warning: {}\n", w));
    }
    s
}

/// `report --recursive` markdown: per-node inventory of declared policies, no baseline.
pub fn render_tree_inventory_markdown(
    tree: &PolicyTree,
    descriptions: &HashMap<String, (String, String)>,
) -> String {
    let mut s = String::new();
    let root = &tree.nodes[&tree.root];
    s.push_str(&format!(
        "# Organization Policies Report (recursive) — {} ({})\n\n",
        root.id, root.display_name
    ));
    s.push_str("_Source: Cloud Asset Inventory (may lag live changes). Declared policies only._\n\n");

    let mut clean_nodes = 0usize;
    // BTreeMap iteration is stable; use DFS order instead for hierarchy shape.
    let mut order: Vec<String> = vec![tree.root.clone()];
    let mut stack: Vec<String> = root.children.iter().rev().cloned().collect();
    while let Some(id) = stack.pop() {
        order.push(id.clone());
        for child in tree.nodes[&id].children.iter().rev() {
            stack.push(child.clone());
        }
    }

    for id in &order {
        let node = &tree.nodes[id];
        if node.policies.is_empty() {
            clean_nodes += 1;
            continue;
        }
        s.push_str(&format!("## {} — {}\n\n", node.id, node.display_name));
        s.push_str(&format!("_Path: {}_\n\n", tree.path(id).join(" / ")));
        for (constraint, policy) in &node.policies {
            s.push_str(&format!("### `{}`\n\n", constraint));
            if let Some((display, description)) = descriptions.get(constraint) {
                if !display.is_empty() {
                    s.push_str(&format!("**{}**\n\n", display));
                }
                if !description.is_empty() {
                    s.push_str(&format!("{}\n\n", description));
                }
            }
            if crate::org_policy::is_managed(constraint) {
                s.push_str("_Managed constraint._\n\n");
            }
            s.push_str(&format!(
                "```json\n{}\n```\n\n",
                serde_json::to_string_pretty(&canonical_policy(policy)).unwrap_or_default()
            ));
        }
    }

    s.push_str(&format!(
        "---\n\n{} of {} nodes declare no policies (inherit only).\n",
        clean_nodes,
        order.len()
    ));
    for w in &tree.warnings {
        s.push_str(&format!("\n> warning: {}\n", w));
    }
    s
}

/// `report --recursive --format json`.
pub fn render_tree_inventory_json(tree: &PolicyTree, scope: &str) -> Value {
    let nodes: Vec<Value> = {
        let mut order: Vec<&TreeNode> = Vec::new();
        let mut stack: Vec<&String> = vec![&tree.root];
        while let Some(id) = stack.pop() {
            let node = &tree.nodes[id];
            order.push(node);
            for child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        order
            .into_iter()
            .filter(|n| !n.policies.is_empty())
            .map(|n| {
                let policies: serde_json::Map<String, Value> = n
                    .policies
                    .iter()
                    .map(|(c, p)| (c.clone(), canonical_policy(p)))
                    .collect();
                serde_json::json!({
                    "node": n.id,
                    "kind": n.kind,
                    "display_name": n.display_name,
                    "path": tree.path(&n.id),
                    "policies": policies,
                })
            })
            .collect()
    };
    serde_json::json!({
        "parent": tree.root,
        "scope": scope,
        "recursive": true,
        "nodes": nodes,
        "warnings": tree.warnings,
    })
}

// Tests (pure layer only — no network).
#[cfg(test)]
mod tests {
    use super::*;

    fn jv(s: &str) -> Value {
        serde_json::from_str(s).expect("valid test JSON")
    }

    fn asset(name: &str, asset_type: &str, ancestors: &[&str], data: &str) -> RawAsset {
        RawAsset {
            name: name.to_string(),
            asset_type: asset_type.to_string(),
            ancestors: ancestors.iter().map(|s| s.to_string()).collect(),
            data: jv(data),
        }
    }

    /// org 1 ── folder 10 "Prod" ── project p-one (number 111, has a policy)
    ///       └─ folder 20 "Dev"  ── project p-two (number 222, clean)
    /// folder 10 declares an enforce-false override; policies on p-one arrive by NUMBER.
    fn fixture() -> Vec<RawAsset> {
        vec![
            asset(
                "//cloudresourcemanager.googleapis.com/organizations/1",
                CRM_ORGANIZATION,
                &["organizations/1"],
                r#"{"displayName":"example.com"}"#,
            ),
            asset(
                "//cloudresourcemanager.googleapis.com/folders/10",
                CRM_FOLDER,
                &["folders/10", "organizations/1"],
                r#"{"displayName":"Prod"}"#,
            ),
            asset(
                "//cloudresourcemanager.googleapis.com/folders/20",
                CRM_FOLDER,
                &["folders/20", "organizations/1"],
                r#"{"displayName":"Dev"}"#,
            ),
            asset(
                "//cloudresourcemanager.googleapis.com/projects/111",
                CRM_PROJECT,
                &["projects/111", "folders/10", "organizations/1"],
                r#"{"projectId":"p-one","projectNumber":"111"}"#,
            ),
            asset(
                "//cloudresourcemanager.googleapis.com/projects/222",
                CRM_PROJECT,
                &["projects/222", "folders/20", "organizations/1"],
                r#"{"projectId":"p-two","projectNumber":"222"}"#,
            ),
            asset(
                "//orgpolicy.googleapis.com/organizations/1/policies/iam.disableServiceAccountKeyCreation",
                ORGPOLICY_POLICY,
                &["organizations/1"],
                r#"{"name":"organizations/1/policies/iam.disableServiceAccountKeyCreation","spec":{"rules":[{"enforce":true}]}}"#,
            ),
            asset(
                "//orgpolicy.googleapis.com/folders/10/policies/iam.disableServiceAccountKeyCreation",
                ORGPOLICY_POLICY,
                &["folders/10", "organizations/1"],
                r#"{"name":"folders/10/policies/iam.disableServiceAccountKeyCreation","spec":{"rules":[{"enforce":false}]}}"#,
            ),
            asset(
                "//orgpolicy.googleapis.com/projects/111/policies/compute.vmExternalIpAccess",
                ORGPOLICY_POLICY,
                &["projects/111", "folders/10", "organizations/1"],
                r#"{"name":"projects/111/policies/compute.vmExternalIpAccess","spec":{"rules":[{"values":{"allowedValues":["projects/p-one/zones/z/instances/i"]}}]}}"#,
            ),
        ]
    }

    fn desired(constraint: &str, policy_json: &str) -> DesiredPolicy {
        DesiredPolicy {
            yaml_key: constraint.replace('.', "-"),
            constraint: constraint.to_string(),
            parent: "organizations/1".to_string(),
            policy: jv(policy_json),
        }
    }

    fn baseline_map(entries: &[(&str, &str)]) -> BTreeMap<String, DesiredPolicy> {
        entries
            .iter()
            .map(|(c, p)| (c.to_string(), desired(c, p)))
            .collect()
    }

    // --- assembly ---------------------------------------------------------------

    #[test]
    fn assemble_tree_builds_hierarchy_from_ancestors() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        assert_eq!(tree.root, "organizations/1");
        assert_eq!(tree.nodes.len(), 5);
        assert_eq!(tree.nodes["folders/10"].parent.as_deref(), Some("organizations/1"));
        assert_eq!(tree.nodes["projects/p-one"].parent.as_deref(), Some("folders/10"));
        // Children sorted: folders before projects, by display name (Dev < Prod).
        assert_eq!(tree.nodes["organizations/1"].children, vec!["folders/20".to_string(), "folders/10".to_string()]);
        assert_eq!(tree.path("projects/p-one"), vec!["example.com", "Prod", "p-one"]);
    }

    #[test]
    fn assemble_tree_normalizes_project_numbers() {
        // The policy asset is scoped projects/111 (number); it must land on projects/p-one.
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        assert!(tree.nodes["projects/p-one"].policies.contains_key("compute.vmExternalIpAccess"));
        assert!(tree.warnings.is_empty(), "unexpected warnings: {:?}", tree.warnings);
    }

    #[test]
    fn assemble_tree_orphan_policy_scope_warns() {
        let mut assets = fixture();
        assets.push(asset(
            "//orgpolicy.googleapis.com/folders/999/policies/compute.requireOsLogin",
            ORGPOLICY_POLICY,
            &["folders/999", "organizations/1"],
            r#"{"name":"folders/999/policies/compute.requireOsLogin","spec":{"rules":[{"enforce":true}]}}"#,
        ));
        let tree = assemble_tree("organizations/1", assets).unwrap();
        assert_eq!(tree.warnings.len(), 1, "orphan scope should warn, not fail: {:?}", tree.warnings);
        assert!(tree.warnings[0].contains("folders/999"), "{:?}", tree.warnings);
    }

    #[test]
    fn assemble_tree_synthesizes_missing_root() {
        let assets: Vec<RawAsset> = fixture().into_iter().filter(|a| a.asset_type != CRM_ORGANIZATION).collect();
        let tree = assemble_tree("1", assets).unwrap();
        assert!(tree.nodes.contains_key("organizations/1"));
        assert!(!tree.warnings.is_empty());
    }

    // --- classification ---------------------------------------------------------

    #[test]
    fn classify_override_boolean_weaker_and_stronger() {
        let base = desired("iam.x", r#"{"spec":{"rules":[{"enforce":true}]}}"#);
        let (class, delta) =
            classify_override(Some(&base), &jv(r#"{"spec":{"rules":[{"enforce":false}]}}"#));
        assert_eq!(class, Classification::OverrideWeaker);
        assert!(delta.unwrap().notes[0].contains("true→false"));

        let base = desired("iam.x", r#"{"spec":{"rules":[{"enforce":false}]}}"#);
        let (class, _) =
            classify_override(Some(&base), &jv(r#"{"spec":{"rules":[{"enforce":"TRUE"}]}}"#));
        assert_eq!(class, Classification::OverrideStronger, "string TRUE must coerce");
    }

    #[test]
    fn classify_override_matches_baseline() {
        // camelCase vs snake_case and value order must not matter.
        let base = desired("iam.x", r#"{"spec":{"rules":[{"values":{"allowed_values":["b","a"]}}]}}"#);
        let (class, _) = classify_override(
            Some(&base),
            &jv(r#"{"spec":{"rules":[{"values":{"allowedValues":["a","b"]}}]}}"#),
        );
        assert_eq!(class, Classification::OverrideMatchesBaseline);
    }

    #[test]
    fn classify_override_list_divergent_computes_delta() {
        let base = desired("gcp.resourceLocations", r#"{"spec":{"rules":[{"values":{"allowedValues":["in:eu-locations"]}}]}}"#);
        let (class, delta) = classify_override(
            Some(&base),
            &jv(r#"{"spec":{"rules":[{"values":{"allowedValues":["in:eu-locations","in:us-locations"]}}]}}"#),
        );
        assert_eq!(class, Classification::OverrideDivergent);
        let delta = delta.unwrap();
        assert_eq!(delta.added_allowed, vec!["in:us-locations"]);
        assert!(delta.removed_allowed.is_empty());
    }

    #[test]
    fn classify_override_reset_and_node_only() {
        let base = desired("iam.x", r#"{"spec":{"rules":[{"enforce":true}]}}"#);
        let (class, delta) = classify_override(Some(&base), &jv(r#"{"spec":{"reset":true}}"#));
        assert_eq!(class, Classification::NodeReset);
        assert!(delta.unwrap().notes[0].contains("reset"));

        let (class, _) = classify_override(None, &jv(r#"{"spec":{"rules":[{"enforce":true}]}}"#));
        assert_eq!(class, Classification::NodeOnly);
    }

    #[test]
    fn classify_override_allow_all_vs_deny_all() {
        let base = desired("x.y", r#"{"spec":{"rules":[{"denyAll":true}]}}"#);
        let (class, _) = classify_override(Some(&base), &jv(r#"{"spec":{"rules":[{"allowAll":true}]}}"#));
        assert_eq!(class, Classification::OverrideWeaker);

        let base = desired("x.y", r#"{"spec":{"rules":[{"allowAll":true}]}}"#);
        let (class, _) = classify_override(Some(&base), &jv(r#"{"spec":{"rules":[{"denyAll":true}]}}"#));
        assert_eq!(class, Classification::OverrideStronger);
    }

    #[test]
    fn classify_override_conditions_are_divergent_not_ordered() {
        let base = desired("iam.x", r#"{"spec":{"rules":[{"enforce":true}]}}"#);
        let (class, delta) = classify_override(
            Some(&base),
            &jv(r#"{"spec":{"rules":[{"enforce":false,"condition":{"expression":"resource.matchTag('env','dev')"}}]}}"#),
        );
        assert_eq!(class, Classification::OverrideDivergent, "conditional rules have no ordering");
        assert!(delta.unwrap().notes.iter().any(|n| n.contains("conditions")));
    }

    // --- tree classification + collapse ------------------------------------------

    #[test]
    fn classify_tree_skips_root_and_collapses_clean_subtrees() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        let baseline = baseline_map(&[(
            "iam.disableServiceAccountKeyCreation",
            r#"{"spec":{"rules":[{"enforce":true}]}}"#,
        )]);
        let (reports, summary) = classify_tree(&tree, &baseline);

        // Root's own policy is the flat diff's job — not reported here.
        assert!(reports.iter().all(|r| r.node != "organizations/1"));
        // folder 10 (weaker override) and p-one (node-only) carry entries.
        assert_eq!(reports.len(), 2);
        let folder = reports.iter().find(|r| r.node == "folders/10").unwrap();
        assert_eq!(folder.entries[0].classification, Classification::OverrideWeaker);
        let project = reports.iter().find(|r| r.node == "projects/p-one").unwrap();
        assert_eq!(project.entries[0].classification, Classification::NodeOnly);

        // folder 20's whole subtree is clean and collapses to ONE record.
        assert_eq!(summary.clean_subtrees.len(), 1);
        let clean = &summary.clean_subtrees[0];
        assert_eq!(clean.under, "folders/20");
        assert_eq!((clean.folders, clean.projects), (1, 1));

        assert_eq!(summary.nodes_with_overrides, 2);
        assert_eq!(summary.total_folders, 2);
        assert_eq!(summary.total_projects, 2);
    }

    // --- rendering ----------------------------------------------------------------

    #[test]
    fn render_console_tree_collapses_clean_subtrees() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        let baseline = baseline_map(&[(
            "iam.disableServiceAccountKeyCreation",
            r#"{"spec":{"rules":[{"enforce":true}]}}"#,
        )]);
        let (reports, summary) = classify_tree(&tree, &baseline);
        let out = render_console_tree(&tree, &reports, &summary);

        assert!(out.contains("organizations/1 (example.com)"), "{out}");
        assert!(out.contains("OVERRIDE (weaker)"), "{out}");
        assert!(out.contains("NODE-ONLY"), "{out}");
        assert!(out.contains("no overrides (1 folder, 1 project collapsed)"), "{out}");
        // p-two must not be listed individually — it is inside the collapsed subtree.
        assert!(!out.contains("p-two"), "collapsed subtree leaked a node:\n{out}");
    }

    #[test]
    fn render_markdown_recursive_headings_unique_per_node() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        let baseline = baseline_map(&[(
            "iam.disableServiceAccountKeyCreation",
            r#"{"spec":{"rules":[{"enforce":true}]}}"#,
        )]);
        let (reports, summary) = classify_tree(&tree, &baseline);
        let out = render_markdown_nodes(&reports, &summary);

        assert!(out.contains("### folders/10 — Prod"), "{out}");
        assert!(out.contains("### projects/p-one — p-one"), "{out}");
        assert!(out.contains("_Path: example.com / Prod_"), "{out}");
        assert!(out.contains("baseline:"), "{out}");
    }

    #[test]
    fn tree_inventory_markdown_has_per_node_sections() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        let out = render_tree_inventory_markdown(&tree, &HashMap::new());
        assert!(out.contains("## organizations/1 — example.com"), "{out}");
        assert!(out.contains("## folders/10 — Prod"), "{out}");
        assert!(out.contains("## projects/p-one — p-one"), "{out}");
        assert!(!out.contains("## folders/20"), "clean node should not get a section:\n{out}");
        assert!(out.contains("declare no policies"), "{out}");
    }

    #[test]
    fn tree_inventory_json_shape() {
        let tree = assemble_tree("organizations/1", fixture()).unwrap();
        let json = render_tree_inventory_json(&tree, "active");
        assert_eq!(json["recursive"], true);
        assert_eq!(json["parent"], "organizations/1");
        let nodes = json["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "org + folder 10 + p-one declare policies");
        assert!(nodes.iter().any(|n| n["node"] == "projects/p-one"));
    }
}
