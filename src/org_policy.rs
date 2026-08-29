//! Organization Policy alignment: export / diff / report, plus the Org Policy
//! client `satz adopt` uses to activate managed constraints and check which
//! policies are live.
//!
//! These subcommands manage GCP Organization Policies (`google_org_policy_policy`,
//! Org Policy API v2) end-to-end. The hard part is GCP **managed constraints**
//! (constraint name contains `.managed.`): depending on the org state they must be
//! *activated*, then *imported as-is*, and only then *modified*. `adopt` sequences
//! the first two; `tofu apply` does the third.
//!
//! The pure diff layer (`compute_diff`, `normalize_spec`, helpers) is IO-free and
//! unit-tested without touching GCP. The `OrgPolicyClient` is the only IO surface.
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ToolConfig;

type BoxErr = Box<dyn std::error::Error>;

// ---------------------------------------------------------------------------
// Small helpers (pure)
// ---------------------------------------------------------------------------

/// Strip a `<parent>/policies/` prefix (or service prefix) to get the bare
/// constraint name, e.g. `organizations/123/policies/iam.managed.x` -> `iam.managed.x`.
pub fn constraint_name(full_or_bare: &str) -> String {
    if let Some(idx) = full_or_bare.rfind("/policies/") {
        full_or_bare[idx + "/policies/".len()..].to_string()
    } else {
        full_or_bare.to_string()
    }
}

/// A constraint is "managed" when its name contains `.managed.`.
pub fn is_managed(constraint: &str) -> bool {
    constraint.contains(".managed.")
}

/// Build the full policy resource name used as the Terraform import id and API name.
pub fn full_policy_name(parent: &str, constraint: &str) -> String {
    format!("{}/policies/{}", parent.trim_end_matches('/'), constraint)
}

/// Normalize a raw org/parent identifier into a fully-qualified parent string.
/// Mirrors `bootstrap.rs` org-id normalization.
pub fn normalize_parent(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with("organizations/")
        || raw.starts_with("folders/")
        || raw.starts_with("projects/")
    {
        raw.to_string()
    } else if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) {
        format!("organizations/{}", raw)
    } else {
        // Could be a project id or already-qualified value; pass through and let the
        // API reject if invalid.
        raw.to_string()
    }
}

/// Turn a constraint name into the label style used by the packs,
/// e.g. `iam.managed.disableServiceAccountKeyCreation`
///   -> `iam-managed-disableServiceAccountKeyCreation`.
pub fn sanitize_yaml_key(constraint: &str) -> String {
    constraint.replace('.', "-")
}

// ---------------------------------------------------------------------------
// Spec normalization (canonical form for equality)
// ---------------------------------------------------------------------------

/// Canonicalize an Org Policy `spec` (or `dry_run_spec`) into a stable JSON shape so
/// that a live policy (camelCase, `enforce: true`, `parameters` object) and a desired
/// policy (snake_case, `enforce: "TRUE"`, `parameters` JSON string) compare equal when
/// they are semantically the same.
pub fn normalize_spec(spec: &Value) -> Value {
    let obj = match spec.as_object() {
        Some(o) => o,
        None => return Value::Null,
    };

    let mut out = serde_json::Map::new();

    // rules (camelCase and snake_case are identical here: "rules")
    if let Some(rules) = obj.get("rules").and_then(|r| r.as_array()) {
        let norm_rules: Vec<Value> = rules.iter().map(normalize_rule).collect();
        out.insert("rules".to_string(), Value::Array(norm_rules));
    }

    // inheritFromParent / inherit_from_parent — only record when true
    let inherit = obj
        .get("inheritFromParent")
        .or_else(|| obj.get("inherit_from_parent"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if inherit {
        out.insert("inherit_from_parent".to_string(), Value::Bool(true));
    }

    // reset — only record when true
    let reset = obj.get("reset").and_then(|v| v.as_bool()).unwrap_or(false);
    if reset {
        out.insert("reset".to_string(), Value::Bool(true));
    }

    Value::Object(out)
}

fn normalize_rule(rule: &Value) -> Value {
    let obj = match rule.as_object() {
        Some(o) => o,
        None => return Value::Null,
    };
    let mut out = serde_json::Map::new();

    // enforce: bool | "TRUE"/"FALSE"
    if let Some(e) = obj.get("enforce") {
        if let Some(b) = coerce_bool(e) {
            out.insert("enforce".to_string(), Value::Bool(b));
        }
    }

    // allowAll / denyAll — only record when true, matching the inherit/reset
    // convention in normalize_spec. Dropping these (as this fn used to) made a
    // rule like {"allowAll": true} normalize to {}, so a policy opening a
    // constraint wide compared equal to one that sets nothing.
    for (camel, snake) in [("allowAll", "allow_all"), ("denyAll", "deny_all")] {
        let val = obj
            .get(camel)
            .or_else(|| obj.get(snake))
            .and_then(coerce_bool)
            .unwrap_or(false);
        if val {
            out.insert(snake.to_string(), Value::Bool(true));
        }
    }

    // values.allowedValues / allowed_values (+ denied)
    if let Some(values) = obj.get("values").and_then(|v| v.as_object()) {
        let mut vout = serde_json::Map::new();
        if let Some(av) = values
            .get("allowedValues")
            .or_else(|| values.get("allowed_values"))
        {
            vout.insert("allowed_values".to_string(), sorted_string_array(av));
        }
        if let Some(dv) = values
            .get("deniedValues")
            .or_else(|| values.get("denied_values"))
        {
            vout.insert("denied_values".to_string(), sorted_string_array(dv));
        }
        if !vout.is_empty() {
            out.insert("values".to_string(), Value::Object(vout));
        }
    }

    // parameters: object | JSON string
    if let Some(p) = obj.get("parameters") {
        let parsed = match p {
            Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::String(s.clone())),
            other => other.clone(),
        };
        out.insert("parameters".to_string(), canonical_json(&parsed));
    }

    // condition: object (expression/title/description/location)
    if let Some(c) = obj.get("condition").filter(|c| !c.is_null()) {
        out.insert("condition".to_string(), canonical_json(c));
    }

    Value::Object(out)
}

fn coerce_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_uppercase().as_str() {
            "TRUE" => Some(true),
            "FALSE" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn sorted_string_array(v: &Value) -> Value {
    if let Some(arr) = v.as_array() {
        let mut items: Vec<String> = arr
            .iter()
            .map(|x| x.as_str().map(|s| s.to_string()).unwrap_or_else(|| x.to_string()))
            .collect();
        items.sort();
        Value::Array(items.into_iter().map(Value::String).collect())
    } else {
        v.clone()
    }
}

/// Recursively sort object keys so two semantically-equal objects serialize identically.
fn canonical_json(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map
                .iter()
                .filter(|(_, val)| !val.is_null())
                .map(|(k, val)| (k.clone(), canonical_json(val)))
                .collect();
            let mut out = serde_json::Map::new();
            for (k, val) in sorted {
                out.insert(k, val);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// Extract the full policy body (spec + dry_run_spec) in canonical form for diffing.
pub(crate) fn canonical_policy(policy: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(spec) = policy.get("spec") {
        out.insert("spec".to_string(), normalize_spec(spec));
    }
    if let Some(dry) = policy.get("dryRunSpec").or_else(|| policy.get("dry_run_spec")) {
        out.insert("dry_run_spec".to_string(), normalize_spec(dry));
    }
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// Diff model (pure)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    MissingNeedsActivation,
    MissingCreatable,
    PresentMatches,
    PresentDiffers,
    CurrentOnly,
    // Tree-mode variants: how a policy DECLARED on a folder/project relates to the
    // framework's root baseline for the same constraint. Classify-only — no verdicts
    // and no planned actions; whether a divergence is sanctioned is the reader's call.
    /// Node redeclares the baseline verbatim (redundant, but not a divergence).
    OverrideMatchesBaseline,
    /// Boolean-form comparison shows the node loosens the baseline (e.g. enforce true→false).
    OverrideWeaker,
    /// Boolean-form comparison shows the node tightens the baseline.
    OverrideStronger,
    /// Differs from the baseline in a way with no defined ordering (list values,
    /// conditions, parameters, dry-run, multi-rule) — see the attached delta.
    OverrideDivergent,
    /// `spec.reset: true` — reverts to the constraint default, breaking inheritance.
    NodeReset,
    /// Declared on the node but not part of the framework at all.
    NodeOnly,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlannedAction {
    ActivateThenImportThenApply,
    CreateViaApply,
    ImportThenApply,
    /// Reserved: present-and-differing where state membership is already known.
    #[allow(dead_code)]
    UpdateViaApply,
    NoOp,
    Ignore,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConstraintDiff {
    pub constraint: String,
    pub parent: String,
    pub yaml_key: String,
    pub managed: bool,
    pub current_spec: Option<Value>,
    pub desired_spec: Option<Value>,
    pub classification: Classification,
    pub action: PlannedAction,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffReport {
    pub parent: String,
    pub preset: String,
    pub generated_at: String,
    pub entries: Vec<ConstraintDiff>,
    /// Recursive mode only: per-node override reports for folders/projects that declare
    /// policies. `None` in flat mode, and skipped from JSON so the flat output shape is
    /// unchanged for existing consumers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<crate::policy_tree::NodeReport>>,
    /// Recursive mode only: hierarchy totals and collapsed clean subtrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_summary: Option<crate::policy_tree::TreeSummary>,
}

/// A desired policy parsed from a preset / config.
#[derive(Debug, Clone)]
pub struct DesiredPolicy {
    pub yaml_key: String,
    /// Bare constraint name; redundant with the map key but handy for tests/diagnostics.
    #[allow(dead_code)]
    pub constraint: String,
    pub parent: String,
    /// Full policy body as JSON (with `spec` / `dry_run_spec`).
    pub policy: Value,
}

/// Compute the diff between live `current` policies (keyed by bare constraint name,
/// value = raw live policy JSON) and `desired` policies (keyed by bare constraint name).
/// Pure and deterministic — no IO.
pub fn compute_diff(
    parent: &str,
    preset: &str,
    generated_at: &str,
    current: &BTreeMap<String, Value>,
    desired: &BTreeMap<String, DesiredPolicy>,
) -> DiffReport {
    let mut entries = Vec::new();

    // Desired-driven entries
    for (constraint, dp) in desired {
        let managed = is_managed(constraint);
        let desired_canon = canonical_policy(&dp.policy);

        let (classification, action, current_spec) = match current.get(constraint) {
            None => {
                if managed {
                    (
                        Classification::MissingNeedsActivation,
                        PlannedAction::ActivateThenImportThenApply,
                        None,
                    )
                } else {
                    (
                        Classification::MissingCreatable,
                        PlannedAction::CreateViaApply,
                        None,
                    )
                }
            }
            Some(live) => {
                let live_canon = canonical_policy(live);
                if live_canon == desired_canon {
                    (
                        Classification::PresentMatches,
                        PlannedAction::NoOp,
                        Some(live_canon),
                    )
                } else {
                    (
                        Classification::PresentDiffers,
                        PlannedAction::ImportThenApply,
                        Some(live_canon),
                    )
                }
            }
        };

        entries.push(ConstraintDiff {
            constraint: constraint.clone(),
            parent: dp.parent.clone(),
            yaml_key: dp.yaml_key.clone(),
            managed,
            current_spec,
            desired_spec: Some(desired_canon),
            classification,
            action,
        });
    }

    // Current-only entries (live but not desired)
    for (constraint, live) in current {
        if desired.contains_key(constraint) {
            continue;
        }
        // The live policy's own name says where it is set; the report-level parent is
        // only a fallback. `current` can hold policies fetched from several parents, so
        // attributing them all to the report parent mislabelled folder-sourced entries.
        let entry_parent = live
            .get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| n.split("/policies/").next())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| parent.to_string());
        entries.push(ConstraintDiff {
            constraint: constraint.clone(),
            parent: entry_parent,
            yaml_key: sanitize_yaml_key(constraint),
            managed: is_managed(constraint),
            current_spec: Some(canonical_policy(live)),
            desired_spec: None,
            classification: Classification::CurrentOnly,
            action: PlannedAction::Ignore,
        });
    }

    entries.sort_by(|a, b| a.constraint.cmp(&b.constraint));

    DiffReport {
        parent: parent.to_string(),
        preset: preset.to_string(),
        generated_at: generated_at.to_string(),
        entries,
        nodes: None,
        tree_summary: None,
    }
}

// ---------------------------------------------------------------------------
// Report rendering (pure)
// ---------------------------------------------------------------------------

pub(crate) fn classification_label(c: &Classification) -> &'static str {
    match c {
        Classification::MissingNeedsActivation => "MISSING (needs activation)",
        Classification::MissingCreatable => "MISSING (creatable)",
        Classification::PresentMatches => "MATCHES",
        Classification::PresentDiffers => "DIFFERS",
        Classification::CurrentOnly => "CURRENT-ONLY",
        Classification::OverrideMatchesBaseline => "OVERRIDE (matches root)",
        Classification::OverrideWeaker => "OVERRIDE (weaker)",
        Classification::OverrideStronger => "OVERRIDE (stronger)",
        Classification::OverrideDivergent => "OVERRIDE (divergent)",
        Classification::NodeReset => "RESET",
        Classification::NodeOnly => "NODE-ONLY",
    }
}

fn action_label(a: &PlannedAction) -> &'static str {
    match a {
        PlannedAction::ActivateThenImportThenApply => "activate -> import -> apply",
        PlannedAction::CreateViaApply => "create (apply)",
        PlannedAction::ImportThenApply => "import -> apply",
        PlannedAction::UpdateViaApply => "update (apply)",
        PlannedAction::NoOp => "no-op",
        PlannedAction::Ignore => "ignore (not desired)",
    }
}

pub fn render_console(report: &DiffReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Org Policy diff — parent: {}  preset: {}\n",
        report.parent, report.preset
    ));
    s.push_str(&format!(
        "{:<60} {:<8} {:<26} {}\n",
        "CONSTRAINT", "MANAGED", "STATUS", "ACTION"
    ));
    s.push_str(&"-".repeat(120));
    s.push('\n');
    for e in &report.entries {
        s.push_str(&format!(
            "{:<60} {:<8} {:<26} {}\n",
            e.constraint,
            if e.managed { "yes" } else { "no" },
            classification_label(&e.classification),
            action_label(&e.action),
        ));
    }
    s.push_str(&summary_line(report));
    s
}

fn summary_line(report: &DiffReport) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in &report.entries {
        *counts.entry(classification_label(&e.classification)).or_insert(0) += 1;
    }
    let mut parts: Vec<String> = counts.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    parts.sort();
    format!("\nSummary: {}\n", parts.join(", "))
}

pub fn render_markdown(report: &DiffReport) -> String {
    let mut s = String::new();
    s.push_str("# Org Policy Diff Report\n\n");
    s.push_str(&format!("- **Parent:** `{}`\n", report.parent));
    s.push_str(&format!("- **Preset:** `{}`\n", report.preset));
    s.push_str(&format!("- **Generated:** {}\n\n", report.generated_at));

    s.push_str("| Constraint | Managed | Status | Action |\n");
    s.push_str("|---|---|---|---|\n");
    for e in &report.entries {
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            e.constraint,
            if e.managed { "yes" } else { "no" },
            classification_label(&e.classification),
            action_label(&e.action),
        ));
    }
    s.push('\n');

    // Detail sections for non-matching entries
    for e in &report.entries {
        if matches!(e.classification, Classification::PresentMatches) {
            continue;
        }
        s.push_str(&format!("## `{}`\n\n", e.constraint));
        s.push_str(&format!(
            "- managed: {} | status: {} | action: {}\n\n",
            e.managed,
            classification_label(&e.classification),
            action_label(&e.action),
        ));
        s.push_str("**Current:**\n\n```json\n");
        s.push_str(
            &e.current_spec
                .as_ref()
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .unwrap_or_else(|| "(not set)".to_string()),
        );
        s.push_str("\n```\n\n**Desired:**\n\n```json\n");
        s.push_str(
            &e.desired_spec
                .as_ref()
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .unwrap_or_else(|| "(not desired)".to_string()),
        );
        s.push_str("\n```\n\n");
    }
    s
}

pub fn render_report(report: &DiffReport, format: &str) -> String {
    match format {
        "json" => serde_json::to_string_pretty(report).unwrap_or_default(),
        "markdown" => render_markdown(report),
        _ => render_console(report),
    }
}

// ---------------------------------------------------------------------------
// Config / preset resolution (IO, but no GCP)
// ---------------------------------------------------------------------------

/// Read the resolved variables from the (include-expanded) main config and determine the
/// parent scope. `org_id_override` wins over the config's `customer-organization-id`.
/// The include-expanded config's variable table. Split out of `resolve_org_and_vars` so
/// callers that do not address the organization (Cloud Identity groups live under
/// `customers/<customer-id>`) are not forced to supply an organization id.
pub(crate) fn resolve_config_vars(
    config_path: &Path,
    include_paths: &[PathBuf],
) -> Result<HashMap<String, serde_yaml::Value>, BoxErr> {
    // The estate is read as Satz: the fragment pipeline's parameter table is
    // the one `transpile` emits tfvars from, so these commands and the emitted
    // HCL cannot disagree about what a param resolves to.
    if config_path.extension().and_then(|e| e.to_str()) != Some("satz") {
        return Err(format!(
            "{} is not a Satz estate — the org-policy commands read Satz only; convert with `satz migrate-to-satz`",
            config_path.display()
        )
        .into());
    }
    let dirs: Vec<String> = include_paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    crate::satz_estate_params(config_path, &dirs)
}

fn resolve_org_and_vars(
    config_path: &Path,
    include_paths: &[PathBuf],
    org_id_override: Option<&str>,
) -> Result<(String, HashMap<String, serde_yaml::Value>), BoxErr> {
    let mut vars = resolve_config_vars(config_path, include_paths)?;

    let org_id = if let Some(o) = org_id_override {
        o.to_string()
    } else {
        vars.get("customer-organization-id")
            .and_then(yaml_scalar_to_string)
            .ok_or("Missing 'customer-organization-id' (pass --customer-organization-id or set it in the config variables)")?
    };

    // Let an explicit override flow through to preset resolution too.
    vars.insert(
        "customer-organization-id".to_string(),
        serde_yaml::Value::String(org_id.clone()),
    );

    Ok((normalize_parent(&org_id), vars))
}

fn yaml_scalar_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Convert a parsed `Config.org_policy_policy` map into desired policies keyed by bare
/// constraint name.
/// The same mapping as `desired_from_config`, from (label, body) pairs rather
/// than a parsed `Config`. Both end at `DesiredPolicy`; only the source of the
/// bodies differs.
fn desired_from_bodies(
    bodies: Vec<(String, serde_yaml::Value)>,
) -> Result<BTreeMap<String, DesiredPolicy>, BoxErr> {
    let mut desired = BTreeMap::new();
    for (yaml_key, body) in bodies {
        let body_json: Value = serde_json::to_value(&body)?;
        let name = body_json.get("name").and_then(|v| v.as_str()).unwrap_or(&yaml_key);
        let constraint = constraint_name(name);
        let parent = body_json
            .get("parent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        desired.insert(
            constraint.clone(),
            DesiredPolicy {
                yaml_key,
                constraint,
                parent: normalize_parent(&parent),
                policy: body_json,
            },
        );
    }
    Ok(desired)
}

/// The desired policy set for `diff`/`report`: every `google_org_policy_policy`
/// the estate emits, read off the folded IR — what the emitter writes. (The
/// old `--preset <pack>` form compiled a pack standalone through the YAML twin;
/// a Satz estate `use`s its pack, so the estate's own set is the desired set.)
fn resolve_desired(
    config_path: &Path,
    runtime_config: &ToolConfig,
) -> Result<(BTreeMap<String, DesiredPolicy>, String), BoxErr> {
    let bodies = crate::satz_org_policy_bodies(config_path, runtime_config)?;
    let desired = desired_from_bodies(bodies)?;
    if desired.is_empty() {
        return Err("the estate declares no google_org_policy_policy to diff".into());
    }
    Ok((desired, format!("estate:{}", config_path.display())))
}

async fn prepare(
    config_path: &Path,
    org_id_override: Option<&str>,
    runtime_config: &ToolConfig,
) -> Result<DiffReport, BoxErr> {
    let include_paths: Vec<PathBuf> =
        runtime_config.include_dirs.iter().map(PathBuf::from).collect();
    let (parent, _vars) = resolve_org_and_vars(config_path, &include_paths, org_id_override)?;
    let (desired, label) = resolve_desired(config_path, runtime_config)?;

    let client = OrgPolicyClient::new().await?;
    // Query each distinct desired parent (policies may target folders/projects).
    let mut current: BTreeMap<String, Value> = BTreeMap::new();
    let mut parents: Vec<String> = desired.values().map(|d| d.parent.clone()).collect();
    parents.push(parent.clone());
    parents.sort();
    parents.dedup();
    for p in &parents {
        if p.is_empty() {
            continue;
        }
        for (k, v) in fetch_current(&client, p).await? {
            current.entry(k).or_insert(v);
        }
    }

    Ok(compute_diff(&parent, &label, &now_stamp(), &current, &desired))
}

/// Recursive counterpart of `prepare`: same desired resolution, but the live side —
/// root diff AND per-node overrides — comes from one Cloud Asset Inventory sweep, so
/// every classification reflects a single consistent snapshot. (Mixing the REST API for
/// the root with CAI for the tree could report one policy as both "missing" at root and
/// "overridden" at a folder; a slightly stale-but-consistent view is the lesser evil.)
async fn prepare_recursive(
    config_path: &Path,
    org_id_override: Option<&str>,
    runtime_config: &ToolConfig,
) -> Result<(DiffReport, crate::policy_tree::PolicyTree), BoxErr> {
    let include_paths: Vec<PathBuf> =
        runtime_config.include_dirs.iter().map(PathBuf::from).collect();
    let (parent, _vars) = resolve_org_and_vars(config_path, &include_paths, org_id_override)?;
    let (desired, label) = resolve_desired(config_path, runtime_config)?;

    let tree = crate::policy_tree::sweep_and_assemble(&parent).await?;

    // Root-level flat diff, sourced from the tree with the same parent-merge semantics
    // as `prepare` (sorted parents, first insertion wins).
    let mut current: BTreeMap<String, Value> = BTreeMap::new();
    let mut parents: Vec<String> = desired.values().map(|d| d.parent.clone()).collect();
    parents.push(parent.clone());
    parents.sort();
    parents.dedup();
    for p in &parents {
        if p.is_empty() {
            continue;
        }
        if let Some(node) = tree.nodes.get(p) {
            for (k, v) in &node.policies {
                current.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }

    let mut report = compute_diff(&parent, &label, &now_stamp(), &current, &desired);
    let (nodes, summary) = crate::policy_tree::classify_tree(&tree, &desired);
    report.nodes = Some(nodes);
    report.tree_summary = Some(summary);
    // The tree rides along for the console/markdown renderers, which need the full
    // hierarchy (including clean nodes) to draw collapsed subtrees; the JSON contract
    // is the DiffReport alone.
    Ok((report, tree))
}

// ---------------------------------------------------------------------------
// Org Policy REST client (Org Policy API v2)
// ---------------------------------------------------------------------------

const ORGPOLICY_HOST: &str = "https://orgpolicy.googleapis.com";

/// Locate the Application Default Credentials JSON file, honoring the standard overrides.
pub(crate) fn adc_file_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(cfg) = std::env::var("CLOUDSDK_CONFIG") {
        if !cfg.trim().is_empty() {
            return Some(PathBuf::from(cfg).join("application_default_credentials.json"));
        }
    }
    #[cfg(windows)]
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(
            PathBuf::from(appdata)
                .join("gcloud")
                .join("application_default_credentials.json"),
        );
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json"),
    )
}

/// Resolve the quota/billing project: env vars first, then the ADC file's
/// `quota_project_id` (written by `gcloud auth application-default set-quota-project`).
pub(crate) fn resolve_quota_project() -> Option<String> {
    for key in [
        "GOOGLE_CLOUD_QUOTA_PROJECT",
        "GOOGLE_CLOUD_PROJECT",
        "GCLOUD_PROJECT",
    ] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    let path = adc_file_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&content).ok()?;
    json.get("quota_project_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

pub struct OrgPolicyClient {
    http: reqwest::Client,
    token: String,
    /// Billing/quota project sent as `x-goog-user-project`. The orgpolicy.googleapis.com
    /// API requires this when authenticating with user ADC; without it GCP bills the
    /// request to gcloud's shared default project and returns 403 (SERVICE_DISABLED).
    quota_project: Option<String>,
}

impl OrgPolicyClient {
    pub async fn new() -> Result<Self, BoxErr> {
        Self::with_quota_project(None).await
    }

    /// Build a client. `quota_override` (e.g. a `--quota-project` flag) wins; otherwise the
    /// quota project is resolved from `GOOGLE_CLOUD_QUOTA_PROJECT`/`GOOGLE_CLOUD_PROJECT`
    /// or the Application Default Credentials file's `quota_project_id`.
    pub async fn with_quota_project(quota_override: Option<String>) -> Result<Self, BoxErr> {
        use google_cloud_auth::credentials::Builder;
        let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
        let credentials = Builder::default()
            .with_scopes(scopes)
            .build_access_token_credentials()?;
        let token = credentials.access_token().await?;

        let quota_project = quota_override
            .filter(|s| !s.trim().is_empty())
            .or_else(resolve_quota_project);
        match &quota_project {
            Some(qp) => println!("Using quota project: {}", qp),
            None => eprintln!(
                "Warning: no quota project found (set GOOGLE_CLOUD_QUOTA_PROJECT, or run \
                 `gcloud auth application-default set-quota-project <project>`). \
                 orgpolicy.googleapis.com requires one and will likely return 403."
            ),
        }

        Ok(Self {
            http: reqwest::Client::new(),
            token: token.token,
            quota_project,
        })
    }

    /// Apply bearer auth and, when known, the `x-goog-user-project` quota header.
    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.bearer_auth(&self.token);
        match &self.quota_project {
            Some(qp) => rb.header("x-goog-user-project", qp),
            None => rb,
        }
    }

    /// List all explicitly-set policies under `parent`.
    pub async fn list_policies(&self, parent: &str) -> Result<Vec<Value>, BoxErr> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let url = format!("{}/v2/{}/policies", ORGPOLICY_HOST, parent);
            let mut req = self.auth(self.http.get(&url));
            if let Some(tok) = &page_token {
                req = req.query(&[("pageToken", tok)]);
            }
            let res = req.send().await?;
            if !res.status().is_success() {
                let status = res.status();
                let body = res.text().await.unwrap_or_default();
                return Err(format!("list_policies {} failed ({}): {}", parent, status, body).into());
            }
            let json: Value = res.json().await?;
            if let Some(arr) = json.get("policies").and_then(|p| p.as_array()) {
                out.extend(arr.iter().cloned());
            }
            page_token = json
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Get a single explicitly-set policy; `None` on 404 (not set).
    #[allow(dead_code)]
    pub async fn get_policy(&self, parent: &str, constraint: &str) -> Result<Option<Value>, BoxErr> {
        let url = format!("{}/v2/{}/policies/{}", ORGPOLICY_HOST, parent, constraint);
        let res = self.auth(self.http.get(&url)).send().await?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("get_policy {} failed ({}): {}", constraint, status, body).into());
        }
        Ok(Some(res.json().await?))
    }

    /// Create (activate) a policy. 409 (already exists) is treated as success.
    pub async fn create_policy(
        &self,
        parent: &str,
        constraint: &str,
        spec: Value,
    ) -> Result<(), BoxErr> {
        let url = format!("{}/v2/{}/policies", ORGPOLICY_HOST, parent);
        let body = serde_json::json!({
            "name": full_policy_name(parent, constraint),
            "spec": spec,
        });
        let res = self
            .auth(self.http.post(&url))
            .json(&body)
            .send()
            .await?;
        if res.status().is_success() || res.status().as_u16() == 409 {
            return Ok(());
        }
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        Err(format!("create_policy {} failed ({}): {}", constraint, status, text).into())
    }

    /// List available constraints under `parent` (for explanatory text in reports).
    pub async fn list_constraints(&self, parent: &str) -> Result<Vec<Value>, BoxErr> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let url = format!("{}/v2/{}/constraints", ORGPOLICY_HOST, parent);
            let mut req = self.auth(self.http.get(&url));
            if let Some(tok) = &page_token {
                req = req.query(&[("pageToken", tok)]);
            }
            let res = req.send().await?;
            if !res.status().is_success() {
                // Constraints listing is best-effort; return what we have.
                break;
            }
            let json: Value = res.json().await?;
            if let Some(arr) = json.get("constraints").and_then(|p| p.as_array()) {
                out.extend(arr.iter().cloned());
            }
            page_token = json
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

/// Fetch current policies for `parent` keyed by bare constraint name.
pub(crate) async fn fetch_current(
    client: &OrgPolicyClient,
    parent: &str,
) -> Result<BTreeMap<String, Value>, BoxErr> {
    let mut map = BTreeMap::new();
    for p in client.list_policies(parent).await? {
        if let Some(name) = p.get("name").and_then(|v| v.as_str()) {
            map.insert(constraint_name(name), p);
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Conversions for export / activation
// ---------------------------------------------------------------------------

/// Convert a live policy's `spec` (camelCase, `enforce: bool`, parameters object) into
/// the snake_case YAML spec the transpiler consumes (`enforce: "TRUE"`, parameters as a
/// JSON string).
fn live_spec_to_yaml(spec: &Value) -> serde_yaml::Value {
    let mut m = serde_yaml::Mapping::new();
    if let Some(rules) = spec.get("rules").and_then(|r| r.as_array()) {
        let mut out_rules = Vec::new();
        for rule in rules {
            let mut rm = serde_yaml::Mapping::new();
            if let Some(b) = rule.get("enforce").and_then(coerce_bool) {
                rm.insert(
                    serde_yaml::Value::String("enforce".into()),
                    serde_yaml::Value::String(if b { "TRUE" } else { "FALSE" }.into()),
                );
            }
            if let Some(values) = rule.get("values").and_then(|v| v.as_object()) {
                let mut vm = serde_yaml::Mapping::new();
                if let Some(av) = values.get("allowedValues").or_else(|| values.get("allowed_values"))
                {
                    vm.insert(serde_yaml::Value::String("allowed_values".into()), json_to_yaml(av));
                }
                if let Some(dv) = values.get("deniedValues").or_else(|| values.get("denied_values")) {
                    vm.insert(serde_yaml::Value::String("denied_values".into()), json_to_yaml(dv));
                }
                rm.insert(
                    serde_yaml::Value::String("values".into()),
                    serde_yaml::Value::Mapping(vm),
                );
            }
            if let Some(p) = rule.get("parameters").filter(|p| !p.is_null()) {
                let as_str = match p {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                rm.insert(
                    serde_yaml::Value::String("parameters".into()),
                    serde_yaml::Value::String(as_str),
                );
            }
            if let Some(c) = rule.get("condition").filter(|c| !c.is_null()) {
                rm.insert(serde_yaml::Value::String("condition".into()), json_to_yaml(c));
            }
            out_rules.push(serde_yaml::Value::Mapping(rm));
        }
        m.insert(
            serde_yaml::Value::String("rules".into()),
            serde_yaml::Value::Sequence(out_rules),
        );
    }
    serde_yaml::Value::Mapping(m)
}

fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    serde_yaml::to_value(v).unwrap_or(serde_yaml::Value::Null)
}


/// Default output base name `<Cxxxx>` from `customer-id` var, falling back to org id.
fn output_basename(parent: &str, vars: &HashMap<String, serde_yaml::Value>) -> String {
    vars.get("customer-id")
        .and_then(yaml_scalar_to_string)
        .unwrap_or_else(|| parent.replace('/', "-"))
}

fn now_stamp() -> String {
    // Runtime clock is intentionally avoided to keep outputs reproducible in tests.
    "generated by satz".to_string()
}

// ---------------------------------------------------------------------------
// Command: export-organizational-policies
// ---------------------------------------------------------------------------

pub async fn export_org_policies(
    config_path: PathBuf,
    org_id_override: Option<String>,
    output: Option<PathBuf>,
    runtime_config: ToolConfig,
) -> Result<(), BoxErr> {
    let include_paths: Vec<PathBuf> = runtime_config.include_dirs.iter().map(PathBuf::from).collect();
    let (parent, vars) =
        resolve_org_and_vars(&config_path, &include_paths, org_id_override.as_deref())?;

    println!("Exporting org policies for {}...", parent);
    let client = OrgPolicyClient::new().await?;
    let current = fetch_current(&client, &parent).await?;

    let mut root = serde_yaml::Mapping::new();
    let mut policies = serde_yaml::Mapping::new();
    for (constraint, live) in &current {
        let key = sanitize_yaml_key(constraint);
        let mut entry = serde_yaml::Mapping::new();
        entry.insert(
            serde_yaml::Value::String("name".into()),
            serde_yaml::Value::String(constraint.clone()),
        );
        entry.insert(
            serde_yaml::Value::String("parent".into()),
            serde_yaml::Value::String(parent.clone()),
        );
        if let Some(spec) = live.get("spec") {
            entry.insert(
                serde_yaml::Value::String("spec".into()),
                live_spec_to_yaml(spec),
            );
        }
        if let Some(dry) = live.get("dryRunSpec") {
            entry.insert(
                serde_yaml::Value::String("dry_run_spec".into()),
                live_spec_to_yaml(dry),
            );
        }
        policies.insert(serde_yaml::Value::String(key), serde_yaml::Value::Mapping(entry));
    }
    root.insert(
        serde_yaml::Value::String("org_policy_policy".into()),
        serde_yaml::Value::Mapping(policies),
    );

    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(root))?;
    let header = format!(
        "# Org policies exported from {} ({}).\n# Re-importable preset: diff with diff-organizational-policies, or adopt with `satz adopt`.\n",
        parent,
        now_stamp()
    );

    // The export is a YAML preset, so it belongs in yaml_dir whether or not --output was
    // given. Previously only the default landed there and an explicit path went to the
    // caller's directory.
    let out_path = crate::resolve_against(
        &runtime_config.yaml_dir,
        output.unwrap_or_else(|| {
            PathBuf::from(format!("{}-orgpolicies.yaml", output_basename(&parent, &vars)))
        }),
    );
    if let Some(dir) = out_path.parent() {
        crate::fsx::create_dir_all(dir)?;
    }
    crate::fsx::write(&out_path, format!("{}{}", header, yaml))?;
    println!("Wrote {} policies to {}", current.len(), out_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Command: diff-organizational-policies
// ---------------------------------------------------------------------------

pub async fn diff_org_policies(
    config_path: PathBuf,
    org_id_override: Option<String>,
    report: Option<PathBuf>,
    format: String,
    recursive: bool,
    runtime_config: ToolConfig,
) -> Result<(), BoxErr> {
    let (report_obj, tree) = if recursive {
        let (r, t) = prepare_recursive(&config_path, org_id_override.as_deref(), &runtime_config).await?;
        (r, Some(t))
    } else {
        let r = prepare(&config_path, org_id_override.as_deref(), &runtime_config).await?;
        (r, None)
    };

    // The JSON contract is the DiffReport itself (nodes/tree_summary ride inside it);
    // console and markdown additionally need the full tree to draw collapsed subtrees.
    let mut rendered = render_report(&report_obj, &format);
    if let (Some(t), Some(nodes), Some(summary)) =
        (&tree, &report_obj.nodes, &report_obj.tree_summary)
    {
        match format.as_str() {
            "json" => {}
            "markdown" => rendered.push_str(&crate::policy_tree::render_markdown_nodes(nodes, summary)),
            _ => rendered.push_str(&crate::policy_tree::render_console_tree(t, nodes, summary)),
        }
    }

    if let Some(path) = report {
        if let Some(dir) = path.parent() {
            crate::fsx::create_dir_all(dir)?;
        }
        crate::fsx::write(&path, &rendered)?;
        println!("Wrote report to {}", path.display());
        // Always echo the console summary to stdout too.
        let mut echoed = render_console(&report_obj);
        if let (Some(t), Some(nodes), Some(summary)) =
            (&tree, &report_obj.nodes, &report_obj.tree_summary)
        {
            echoed.push_str(&crate::policy_tree::render_console_tree(t, nodes, summary));
        }
        println!("{}", echoed);
    } else {
        println!("{}", rendered);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Command: report-organizational-policies
// ---------------------------------------------------------------------------

pub async fn report_org_policies(
    config_path: PathBuf,
    org_id_override: Option<String>,
    scope: String,
    format: String,
    report: Option<PathBuf>,
    recursive: bool,
    runtime_config: ToolConfig,
) -> Result<(), BoxErr> {
    let include_paths: Vec<PathBuf> = runtime_config.include_dirs.iter().map(PathBuf::from).collect();
    let (parent, vars) =
        resolve_org_and_vars(&config_path, &include_paths, org_id_override.as_deref())?;

    // Recursive: one CAI sweep replaces the per-parent policy fetch; constraint
    // descriptions still come from the one org-level list_constraints call. The
    // "available but not set" scope only makes sense org-wide, so scope handling
    // stays a root-level concern (documented in the flag help).
    if recursive {
        let tree = crate::policy_tree::sweep_and_assemble(&parent).await?;
        let client = OrgPolicyClient::new().await?;
        let constraints = client.list_constraints(&parent).await.unwrap_or_default();
        let mut descriptions: HashMap<String, (String, String)> = HashMap::new();
        for c in &constraints {
            if let Some(name) = c.get("name").and_then(|v| v.as_str()) {
                let bare = constraint_name(name);
                let display = c.get("displayName").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                descriptions.insert(bare, (display, desc));
            }
        }

        let out_path = report.unwrap_or_else(|| {
            let ext = if format == "json" { "json" } else { "md" };
            PathBuf::from(format!(
                "{}-orgpolicies-tree-report.{}",
                output_basename(&parent, &vars),
                ext
            ))
        });
        if let Some(dir) = out_path.parent() {
            crate::fsx::create_dir_all(dir)?;
        }

        if format == "json" {
            let json = crate::policy_tree::render_tree_inventory_json(&tree, &scope);
            crate::fsx::write(&out_path, serde_json::to_string_pretty(&json)?)?;
        } else {
            let md = crate::policy_tree::render_tree_inventory_markdown(&tree, &descriptions);
            crate::fsx::write(&out_path, &md)?;
            if format == "pdf" {
                try_pandoc_pdf(&out_path);
            }
        }
        println!("Wrote report to {}", out_path.display());
        return Ok(());
    }

    let client = OrgPolicyClient::new().await?;
    let current = fetch_current(&client, &parent).await?;
    let constraints = client.list_constraints(&parent).await.unwrap_or_default();

    // Build description lookup keyed by bare constraint name.
    let mut descriptions: HashMap<String, (String, String)> = HashMap::new();
    for c in &constraints {
        if let Some(name) = c.get("name").and_then(|v| v.as_str()) {
            let bare = constraint_name(name);
            let display = c
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let desc = c
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            descriptions.insert(bare, (display, desc));
        }
    }

    let md = render_inventory_markdown(&parent, &scope, &current, &descriptions);

    // A report is human-readable output, not a definition, so it lands in the working
    // directory where the caller is looking — matching an explicitly passed --report.
    // Previously the default went to yaml_dir while an explicit path did not.
    let out_path = report.unwrap_or_else(|| {
        let ext = if format == "json" { "json" } else { "md" };
        PathBuf::from(format!(
            "{}-orgpolicies-report.{}",
            output_basename(&parent, &vars),
            ext
        ))
    });
    if let Some(dir) = out_path.parent() {
        crate::fsx::create_dir_all(dir)?;
    }

    if format == "json" {
        let json = serde_json::json!({
            "parent": parent,
            "scope": scope,
            "policies": current.values().cloned().collect::<Vec<_>>(),
        });
        crate::fsx::write(&out_path, serde_json::to_string_pretty(&json)?)?;
    } else {
        crate::fsx::write(&out_path, &md)?;
        if format == "pdf" {
            try_pandoc_pdf(&out_path);
        }
    }
    println!("Wrote report to {}", out_path.display());
    Ok(())
}

fn render_inventory_markdown(
    parent: &str,
    scope: &str,
    current: &BTreeMap<String, Value>,
    descriptions: &HashMap<String, (String, String)>,
) -> String {
    let mut s = String::new();
    s.push_str("# Organization Policies Report\n\n");
    s.push_str(&format!("- **Parent:** `{}`\n", parent));
    s.push_str(&format!("- **Scope:** {}\n\n", scope));

    let include_active = scope == "active" || scope == "full";
    let include_inactive = scope == "inactive" || scope == "full";

    if include_active {
        s.push_str("## Set policies\n\n");
        for (constraint, live) in current {
            let (display, desc) = descriptions.get(constraint).cloned().unwrap_or_default();
            s.push_str(&format!("### `{}`", constraint));
            if !display.is_empty() {
                s.push_str(&format!(" — {}", display));
            }
            s.push('\n');
            if is_managed(constraint) {
                s.push_str("\n_Managed constraint._\n");
            }
            if !desc.is_empty() {
                s.push_str(&format!("\n{}\n", desc));
            }
            s.push_str("\n```json\n");
            s.push_str(&serde_json::to_string_pretty(&canonical_policy(live)).unwrap_or_default());
            s.push_str("\n```\n\n");
        }
    }

    if include_inactive {
        s.push_str("## Available but not set\n\n");
        let mut names: Vec<&String> = descriptions.keys().collect();
        names.sort();
        for name in names {
            if current.contains_key(name) {
                continue;
            }
            let (display, desc) = descriptions.get(name).cloned().unwrap_or_default();
            s.push_str(&format!("- `{}`", name));
            if !display.is_empty() {
                s.push_str(&format!(" — {}", display));
            }
            if !desc.is_empty() {
                let short: String = desc.chars().take(160).collect();
                s.push_str(&format!(": {}", short));
            }
            s.push('\n');
        }
        s.push('\n');
    }

    s
}

pub(crate) fn try_pandoc_pdf(md_path: &Path) {
    let pdf_path = md_path.with_extension("pdf");
    let status = std::process::Command::new("pandoc")
        .arg(md_path)
        .arg("-o")
        .arg(&pdf_path)
        .status();
    match status {
        Ok(s) if s.success() => println!("Wrote PDF to {}", pdf_path.display()),
        _ => println!(
            "Note: PDF generation needs `pandoc` on PATH; kept markdown at {}",
            md_path.display()
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests (pure layer only — no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn jv(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn helpers_basic() {
        assert_eq!(
            constraint_name("organizations/123/policies/iam.managed.x"),
            "iam.managed.x"
        );
        assert_eq!(constraint_name("iam.managed.x"), "iam.managed.x");
        assert!(is_managed("iam.managed.x"));
        assert!(!is_managed("iam.allowedPolicyMemberDomains"));
        assert_eq!(normalize_parent("123456"), "organizations/123456");
        assert_eq!(normalize_parent("organizations/9"), "organizations/9");
        assert_eq!(normalize_parent("folders/9"), "folders/9");
        assert_eq!(
            sanitize_yaml_key("iam.managed.disableServiceAccountKeyCreation"),
            "iam-managed-disableServiceAccountKeyCreation"
        );
        assert_eq!(
            full_policy_name("organizations/123", "iam.managed.x"),
            "organizations/123/policies/iam.managed.x"
        );
    }

    #[test]
    fn enforce_string_and_bool_compare_equal() {
        let live = jv(r#"{"rules":[{"enforce":true}]}"#);
        let desired = jv(r#"{"rules":[{"enforce":"TRUE"}]}"#);
        assert_eq!(normalize_spec(&live), normalize_spec(&desired));
    }

    #[test]
    fn allowed_values_order_insensitive() {
        let live = jv(r#"{"rules":[{"values":{"allowedValues":["b","a"]}}]}"#);
        let desired = jv(r#"{"rules":[{"values":{"allowed_values":["a","b"]}}]}"#);
        assert_eq!(normalize_spec(&live), normalize_spec(&desired));
    }

    #[test]
    fn parameters_string_vs_object_compare_equal() {
        let live = jv(r#"{"rules":[{"enforce":true,"parameters":{"allowedDomains":["@example.net"]}}]}"#);
        let desired =
            jv(r#"{"rules":[{"enforce":"TRUE","parameters":"{\"allowedDomains\":[\"@example.net\"]}"}]}"#);
        assert_eq!(normalize_spec(&live), normalize_spec(&desired));
    }

    #[test]
    fn dry_run_spec_normalized() {
        let a = jv(
            r#"{"spec":{"rules":[{"enforce":true}]},"dryRunSpec":{"rules":[{"enforce":false}]}}"#,
        );
        let b = jv(
            r#"{"spec":{"rules":[{"enforce":"TRUE"}]},"dry_run_spec":{"rules":[{"enforce":"FALSE"}]}}"#,
        );
        assert_eq!(canonical_policy(&a), canonical_policy(&b));
    }

    fn desired_map(entries: &[(&str, &str, &str)]) -> BTreeMap<String, DesiredPolicy> {
        // (constraint, parent, policy-json)
        let mut m = BTreeMap::new();
        for (c, p, body) in entries {
            m.insert(
                c.to_string(),
                DesiredPolicy {
                    yaml_key: sanitize_yaml_key(c),
                    constraint: c.to_string(),
                    parent: p.to_string(),
                    policy: jv(body),
                },
            );
        }
        m
    }

    #[test]
    fn classify_missing_managed_vs_creatable() {
        let current = BTreeMap::new();
        let desired = desired_map(&[
            ("iam.managed.x", "organizations/1", r#"{"spec":{"rules":[{"enforce":"TRUE"}]}}"#),
            ("compute.x", "organizations/1", r#"{"spec":{"rules":[{"enforce":"TRUE"}]}}"#),
        ]);
        let r = compute_diff("organizations/1", "p", "t", &current, &desired);
        let managed = r.entries.iter().find(|e| e.constraint == "iam.managed.x").unwrap();
        assert_eq!(managed.classification, Classification::MissingNeedsActivation);
        assert_eq!(managed.action, PlannedAction::ActivateThenImportThenApply);
        let plain = r.entries.iter().find(|e| e.constraint == "compute.x").unwrap();
        assert_eq!(plain.classification, Classification::MissingCreatable);
        assert_eq!(plain.action, PlannedAction::CreateViaApply);
    }

    #[test]
    fn classify_matches_and_differs_and_current_only() {
        let mut current = BTreeMap::new();
        current.insert(
            "compute.x".to_string(),
            jv(r#"{"name":"organizations/1/policies/compute.x","spec":{"rules":[{"enforce":true}]}}"#),
        );
        current.insert(
            "compute.y".to_string(),
            jv(r#"{"name":"organizations/1/policies/compute.y","spec":{"rules":[{"enforce":true}]}}"#),
        );
        current.insert(
            "stray.z".to_string(),
            jv(r#"{"name":"organizations/1/policies/stray.z","spec":{"rules":[{"enforce":true}]}}"#),
        );
        let desired = desired_map(&[
            ("compute.x", "organizations/1", r#"{"spec":{"rules":[{"enforce":"TRUE"}]}}"#),
            ("compute.y", "organizations/1", r#"{"spec":{"rules":[{"enforce":"FALSE"}]}}"#),
        ]);
        let r = compute_diff("organizations/1", "p", "t", &current, &desired);
        let x = r.entries.iter().find(|e| e.constraint == "compute.x").unwrap();
        assert_eq!(x.classification, Classification::PresentMatches);
        assert_eq!(x.action, PlannedAction::NoOp);
        let y = r.entries.iter().find(|e| e.constraint == "compute.y").unwrap();
        assert_eq!(y.classification, Classification::PresentDiffers);
        assert_eq!(y.action, PlannedAction::ImportThenApply);
        let z = r.entries.iter().find(|e| e.constraint == "stray.z").unwrap();
        assert_eq!(z.classification, Classification::CurrentOnly);
        assert_eq!(z.action, PlannedAction::Ignore);
    }

    #[test]
    fn live_spec_round_trips_to_yaml() {
        let spec = jv(r#"{"rules":[{"enforce":true}]}"#);
        let y = live_spec_to_yaml(&spec);
        let rules = y.get("rules").unwrap().as_sequence().unwrap();
        let enforce = rules[0].get("enforce").unwrap().as_str().unwrap();
        assert_eq!(enforce, "TRUE");
    }

    #[test]
    fn cis_pack_resolves_through_the_estate_that_uses_it() {
        // The desired set is read off the estate's folded IR, so the pack's
        // `{customer_organization_id}` / `{customer_domain}` params resolve
        // from the estate's `params` — the only source they ever had.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let tmp = std::env::temp_dir().join(format!("satz-cis-desired-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let estate = tmp.join("main.satz");
        std::fs::write(
            &estate,
            "estate cis_test\n\nparams {\n  customer_organization_id = \"123456789\"\n  customer_id = \"C0abcd123\"\n  customer_domain = \"example.com\"\n}\n\nterraform {\n  backend {\n    local { path = \"t.tfstate\" }\n  }\n}\n\ngoogle_org_policy_policy {\n  use \"presets/CIS-GCP-Foundation-4.0.satz\"\n}\n",
        )
        .unwrap();
        let mut cfg: crate::ToolConfig = toml::from_str("").unwrap();
        cfg.schema_dir = root.join("tests/schemas").to_string_lossy().into_owned();
        cfg.include_dirs = vec![root.to_string_lossy().into_owned()];

        let bodies = crate::satz_org_policy_bodies(&estate, &cfg).expect("CIS pack should compile");
        let desired = desired_from_bodies(bodies).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        // A managed boolean constraint is present and flagged managed.
        let m = desired
            .get("iam.managed.disableServiceAccountKeyCreation")
            .expect("managed constraint present");
        assert!(is_managed(&m.constraint));
        assert_eq!(m.parent, "organizations/123456789");

        // The parameterized managed constraint resolved its JSON parameters.
        let ec = desired
            .get("essentialcontacts.managed.allowedContactDomains")
            .expect("parameterized constraint present");
        let params = ec.policy["spec"]["rules"][0]["parameters"]
            .as_str()
            .expect("parameters is a JSON string");
        assert!(params.contains("@example.com"), "params were: {}", params);
    }

    #[test]
    fn flat_json_has_no_recursive_keys() {
        // The flat DiffReport serialization is the public JSON contract; the recursive
        // fields must vanish entirely when unset, not appear as null.
        let report = compute_diff("organizations/1", "p", "now", &BTreeMap::new(), &desired_map(&[]));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("\"nodes\""), "flat JSON grew a key: {json}");
        assert!(!json.contains("\"tree_summary\""), "flat JSON grew a key: {json}");
    }

    #[test]
    fn current_only_parent_derived_from_policy_name() {
        // A live policy fetched from a folder must be attributed to that folder, not to
        // the report-level parent (previously every current-only entry claimed the org).
        let mut current = BTreeMap::new();
        current.insert(
            "compute.requireOsLogin".to_string(),
            jv(r#"{"name":"folders/999/policies/compute.requireOsLogin","spec":{"rules":[{"enforce":true}]}}"#),
        );
        let report = compute_diff("organizations/1", "p", "now", &current, &desired_map(&[]));
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].parent, "folders/999");
        assert_eq!(report.entries[0].classification, Classification::CurrentOnly);
    }

    #[test]
    fn normalize_rule_preserves_allow_all_deny_all() {
        // {"allowAll": true} used to normalize to an empty rule, so a policy opening a
        // constraint wide compared equal to one that sets nothing at all.
        let open = normalize_spec(&jv(r#"{"rules":[{"allowAll":true}]}"#));
        let empty = normalize_spec(&jv(r#"{"rules":[{}]}"#));
        assert_ne!(open, empty, "allowAll must survive normalization");
        assert_eq!(open["rules"][0]["allow_all"], true);

        let deny = normalize_spec(&jv(r#"{"rules":[{"deny_all":"TRUE"}]}"#));
        assert_eq!(deny["rules"][0]["deny_all"], true, "snake_case + string coercion");
    }

    #[test]
    fn recursive_report_json_includes_nodes_and_summary() {
        let mut report = compute_diff("organizations/1", "p", "now", &BTreeMap::new(), &desired_map(&[]));
        report.nodes = Some(Vec::new());
        report.tree_summary = Some(crate::policy_tree::TreeSummary {
            total_folders: 1,
            total_projects: 2,
            nodes_with_overrides: 0,
            counts: BTreeMap::new(),
            clean_subtrees: Vec::new(),
            warnings: Vec::new(),
        });
        let json: Value = serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
        assert!(json.get("nodes").is_some());
        assert_eq!(json["tree_summary"]["total_projects"], 2);
    }
}
