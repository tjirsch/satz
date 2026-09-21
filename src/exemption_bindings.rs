//! Undeclared exemption bindings: the live bindings of the estate's exemption tag key
//! that the estate does not declare.
//!
//! An exemption is a tag binding (ADR 0015). A binding the estate declares is in the
//! repository with its owner and reason, and `require` prints it under its control. A
//! binding made in the console is invisible to both — the control reads enforced and
//! something is let out that nobody wrote down. `report-compliance` lists every
//! `cloudresourcemanager.googleapis.com/TagBinding` of the key through Cloud Asset
//! Inventory, subtracts the declared ones, and reports the rest.
//!
//! Everything here but [`live_tag_bindings`] and [`project_numbers`] is pure, so the
//! comparison is tested over recorded Cloud Asset responses.

use std::collections::{BTreeMap, BTreeSet};

use crate::manifest::{EmittedResource, Manifest};

/// The address of the key `presets/exemptions/exemption-tag.satz` declares. Only this
/// key's values are exemptions; a binding of any other key is out of scope.
pub(crate) const EXEMPTION_KEY: &str = "google_tags_tag_key.exemption";

/// The Cloud Asset Inventory type of a tag binding.
pub(crate) const TAG_BINDING_ASSET: &str = "cloudresourcemanager.googleapis.com/TagBinding";

/// The value a declared binding binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BoundValue {
    /// a value of the exemption key the estate declares, by its namespaced name
    /// (`<org>/<key short name>/<value short name>`)
    Named(String),
    /// a literal `tagValues/<id>`
    Id(String),
}

/// One `google_tags_tag_binding` of the exemption key, as the estate declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredBinding {
    pub address: String,
    pub value: BoundValue,
    /// The bound target's full resource name with every reference satz can follow
    /// resolved; `Err` says why it cannot be compared with a live binding.
    pub parent: Result<String, String>,
}

/// The estate's exemption key, its values and the bindings it declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Vocabulary {
    /// `<org>/<key short name>` — the namespaced name of the key
    pub key: String,
    /// value address → namespaced name
    pub values: BTreeMap<String, String>,
    pub declared: Vec<DeclaredBinding>,
}

impl Vocabulary {
    /// Whether a live binding's namespaced value name belongs to this key.
    fn owns(&self, namespaced: &str) -> bool {
        namespaced.strip_prefix(&self.key).is_some_and(|rest| rest.starts_with('/'))
    }
}

/// The estate's exemption vocabulary, from the emission manifest. `Ok(None)` when the
/// estate declares no exemption key — nothing to check, and nothing to say.
pub(crate) fn vocabulary(manifest: &Manifest, org_id: Option<&str>) -> Result<Option<Vocabulary>, String> {
    let Some(key) = manifest.resources.get(EXEMPTION_KEY) else { return Ok(None) };
    let short = literal(key, "short_name").ok_or_else(|| {
        format!("{} has no literal `short_name`, so its bindings cannot be recognised live", EXEMPTION_KEY)
    })?;
    let org = match literal(key, "parent") {
        Some(p) => p
            .strip_prefix("organizations/")
            .map(str::to_string)
            .ok_or_else(|| format!("{} is parented under `{}`, not an organisation", EXEMPTION_KEY, p))?,
        None => org_id
            .map(str::to_string)
            .ok_or_else(|| format!("{} names no organisation, and neither does the estate", EXEMPTION_KEY))?,
    };
    let key_name = format!("{}/{}", org, short);
    let parent_refs = [format!("{}.name", EXEMPTION_KEY), format!("{}.id", EXEMPTION_KEY)];
    let mut values = BTreeMap::new();
    for v in manifest.of_type("google_tags_tag_value") {
        if !v.refs.get("parent").is_some_and(|r| parent_refs.contains(r)) {
            continue;
        }
        let vs = literal(v, "short_name")
            .ok_or_else(|| format!("{} has no literal `short_name`", v.address()))?;
        values.insert(v.address(), format!("{}/{}", key_name, vs));
    }
    let mut declared = Vec::new();
    for b in manifest.of_type("google_tags_tag_binding") {
        let value = if let Some(r) = b.refs.get("tag_value") {
            let target = r.strip_suffix(".name").or_else(|| r.strip_suffix(".id")).unwrap_or(r);
            match values.get(target) {
                Some(ns) => BoundValue::Named(ns.clone()),
                // a value of another key: not an exemption
                None => continue,
            }
        } else if let Some(id) = literal(b, "tag_value").filter(|v| v.starts_with("tagValues/")) {
            BoundValue::Id(id)
        } else {
            continue;
        };
        let parent = match b.attrs.get("parent") {
            Some(p) => resolve_parent(p, manifest),
            None => Err(format!("{} has no `parent` satz can read", b.address())),
        };
        declared.push(DeclaredBinding { address: b.address(), value, parent });
    }
    Ok(Some(Vocabulary { key: key_name, values, declared }))
}

/// A top-level attribute that is a literal string.
fn literal(r: &EmittedResource, attr: &str) -> Option<String> {
    r.attrs.get(attr).filter(|v| !crate::manifest::has_interpolation(v) && !v.is_empty()).cloned()
}

/// A declared binding's `parent` with its `${…}` references replaced by what they
/// resolve to. A project resolves to its `project_id`, a folder to the `folders/<n>` it
/// was adopted as, a service account to its email; anything else cannot be known before
/// an apply and is an `Err` naming the reference.
fn resolve_parent(raw: &str, manifest: &Manifest) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| format!("`{}` has an unterminated reference", raw))?;
        out.push_str(&resolve_ref(after[..end].trim(), manifest)?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn resolve_ref(traversal: &str, manifest: &Manifest) -> Result<String, String> {
    let unknown = || format!("`${{{}}}` is not known before an apply", traversal);
    let mut parts = traversal.splitn(3, '.');
    let (Some(t), Some(l), Some(attr)) = (parts.next(), parts.next(), parts.next()) else { return Err(unknown()) };
    let r = manifest.resources.get(&format!("{}.{}", t, l)).ok_or_else(unknown)?;
    match (t, attr) {
        ("google_project", "project_id" | "number") => literal(r, "project_id").ok_or_else(unknown),
        ("google_project", "id") => literal(r, "project_id").map(|p| format!("projects/{}", p)).ok_or_else(unknown),
        ("google_folder", "name" | "id") => r.import_id.clone().ok_or_else(unknown),
        ("google_folder", "folder_id") => {
            r.import_id.as_deref().map(|i| i.trim_start_matches("folders/").to_string()).ok_or_else(unknown)
        }
        ("google_service_account", "email" | "name" | "id") => {
            let account = literal(r, "account_id").ok_or_else(unknown)?;
            let project = manifest.project_of(r).filter(|p| !crate::manifest::has_interpolation(p)).ok_or_else(unknown)?;
            let email = format!("{}@{}.iam.gserviceaccount.com", account, project);
            Ok(if attr == "email" { email } else { format!("projects/{}/serviceAccounts/{}", project, email) })
        }
        _ => Err(unknown()),
    }
}

/// Every project id a declared target names (`…/projects/<id>/…`), which Cloud Asset
/// reports by NUMBER for a project and may for anything under one.
pub(crate) fn project_ids(v: &Vocabulary) -> BTreeSet<String> {
    v.declared.iter().filter_map(|d| d.parent.as_ref().ok()).filter_map(|p| project_segment(p)).collect()
}

/// The project id in a full resource name, when it is one (not already a number).
fn project_segment(parent: &str) -> Option<String> {
    let at = parent.find("projects/")? + "projects/".len();
    let id = parent[at..].split('/').next()?;
    (!id.is_empty() && !id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

/// The forms a declared target can take in Cloud Asset: as the estate writes it, and
/// with its project id replaced by the project's number.
fn target_forms(parent: &str, numbers: &BTreeMap<String, String>) -> Vec<String> {
    let mut forms = vec![parent.to_string()];
    if let Some(id) = project_segment(parent) {
        if let Some(n) = numbers.get(&id) {
            forms.push(parent.replacen(&format!("projects/{}", id), &format!("projects/{}", n), 1));
        }
    }
    forms
}

/// One live binding of the exemption key that no declaration accounts for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Undeclared {
    /// `<org>/<key>/<value>`
    pub value: String,
    /// `tagValues/<id>`
    pub value_id: String,
    /// the bound target's full resource name, as Cloud Asset reports it
    pub target: String,
    /// declared bindings of the same value whose target satz could not resolve, so
    /// one of them may be this binding
    pub unresolved_declared: Vec<String>,
    /// `<control> (<policy address>)` for every claimed control whose witness
    /// policy conditions on this value; filled by the report
    pub controls: Vec<String>,
}

/// What the check found: how many live bindings of the key there are, and which of
/// them no declaration accounts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comparison {
    pub live: usize,
    pub undeclared: Vec<Undeclared>,
}

/// Subtract the declared bindings from the live ones.
///
/// `live` is the resource data of every `TagBinding` asset of the organisation. A
/// binding of another key is ignored. A live binding matches a declared one when both
/// the value (namespaced name, or `tagValues/<id>`) and the target (as written, or with
/// its project id replaced by the number) agree.
pub(crate) fn compare(
    v: &Vocabulary,
    live: &[serde_json::Value],
    numbers: &BTreeMap<String, String>,
) -> Result<Comparison, String> {
    let text = |b: &serde_json::Value, k: &str| b.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mut out = Comparison { live: 0, undeclared: Vec::new() };
    for b in live {
        let value = text(b, "tagValueNamespacedName");
        if value.is_empty() {
            return Err(format!("a TagBinding asset carries no tagValueNamespacedName: {}", b));
        }
        if !v.owns(&value) {
            continue;
        }
        out.live += 1;
        let (value_id, target) = (text(b, "tagValue"), text(b, "parent"));
        let (value, value_id) = (value.as_str(), value_id.as_str());
        let same_value = |d: &DeclaredBinding| match &d.value {
            BoundValue::Named(ns) => ns == value,
            BoundValue::Id(id) => id == value_id,
        };
        let declared = v.declared.iter().filter(|d| same_value(d)).any(|d| match &d.parent {
            Ok(p) => target_forms(p, numbers).contains(&target),
            Err(_) => false,
        });
        if declared {
            continue;
        }
        out.undeclared.push(Undeclared {
            value: value.to_string(),
            value_id: value_id.to_string(),
            target,
            unresolved_declared: v
                .declared
                .iter()
                .filter(|d| same_value(d) && d.parent.is_err())
                .map(|d| d.address.clone())
                .collect(),
            controls: Vec::new(),
        });
    }
    out.undeclared.sort_by(|a, b| (&a.value, &a.target).cmp(&(&b.value, &b.target)));
    Ok(out)
}

/// The exemption values a policy conditions on, by the namespaced names or
/// `tagValues/<id>` a live binding carries. Read from the expressions of its
/// conditional rules: `${google_tags_tag_value.<x>.name}` references,
/// `resource.matchTag('<org>/<key>', '<value>')`, and literal `tagValues/<id>`.
pub(crate) fn conditioned_values(policy: &EmittedResource, v: &Vocabulary) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for e in &policy.condition_expressions {
        for (addr, ns) in &v.values {
            if e.contains(&format!("${{{}.name}}", addr)) || e.contains(&format!("${{{}.id}}", addr)) {
                out.insert(ns.clone());
            }
        }
        let mut rest = e.as_str();
        while let Some(at) = rest.find("tagValues/") {
            let id: String = rest[at + "tagValues/".len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !id.is_empty() {
                out.insert(format!("tagValues/{}", id));
            }
            rest = &rest[at + "tagValues/".len()..];
        }
        let mut rest = e.as_str();
        while let Some(at) = rest.find("matchTag(") {
            rest = &rest[at + "matchTag(".len()..];
            let args: Vec<&str> = rest.split(')').next().unwrap_or("").split(',').map(|a| a.trim().trim_matches(['\'', '"'])).collect();
            if let [key, value] = args.as_slice() {
                if *key == v.key {
                    out.insert(format!("{}/{}", key, value));
                }
            }
        }
    }
    out
}

/// Every `TagBinding` asset of the organisation, as its resource data.
pub(crate) async fn live_tag_bindings(org_id: &str) -> Result<Vec<serde_json::Value>, String> {
    let types = BTreeSet::from([(TAG_BINDING_ASSET.to_string(), crate::compliance::LiveContent::Resource)]);
    let inventory = crate::compliance::live_inventory(org_id, &types).await.map_err(|e| e.to_string())?;
    Ok(inventory.into_values().flat_map(|ids| ids.into_values()).collect())
}

/// Project id → number for the declared targets. A project that does not exist yet
/// has no number and is compared as written; a refused lookup is an error.
pub(crate) async fn project_numbers(ids: &BTreeSet<String>) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let token = crate::gcp::access_token().await?;
    let client = reqwest::Client::new();
    for id in ids {
        if let Some(name) = crate::gcp::resourcemanager::get_project_number(&client, &token, id).await? {
            out.insert(id.clone(), name.trim_start_matches("projects/").to_string());
        }
    }
    Ok(out)
}

/// What the check did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// `--no-live`
    Skipped,
    /// the estate names no organisation to list bindings in
    NoOrganizationId,
    /// the estate's vocabulary or the live read failed (the reason) — never "none"
    Unavailable(String),
    Checked(Comparison),
}

/// The check for one estate: the key it covers, how many bindings the estate
/// declares, and what the live read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Check {
    /// `<org>/<key short name>`, when the vocabulary could be read
    pub key: Option<String>,
    pub declared: usize,
    pub outcome: Outcome,
}

impl Check {
    /// The undeclared bindings found, empty unless the check ran.
    pub(crate) fn undeclared(&self) -> &[Undeclared] {
        match &self.outcome {
            Outcome::Checked(c) => &c.undeclared,
            _ => &[],
        }
    }

    pub(crate) fn undeclared_mut(&mut self) -> &mut [Undeclared] {
        match &mut self.outcome {
            Outcome::Checked(c) => &mut c.undeclared,
            _ => &mut [],
        }
    }

    fn status(&self) -> &'static str {
        match self.outcome {
            Outcome::Skipped => "skipped",
            Outcome::NoOrganizationId => "no-organization-id",
            Outcome::Unavailable(_) => "unavailable",
            Outcome::Checked(_) => "checked",
        }
    }

    /// The evidence record's `exemption_bindings` object.
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let (live, reason) = match &self.outcome {
            Outcome::Checked(c) => (Some(c.live), None),
            Outcome::Unavailable(why) => (None, Some(why.clone())),
            _ => (None, None),
        };
        serde_json::json!({
            "status": self.status(),
            "key": self.key,
            "declared": self.declared,
            "live": live,
            "reason": reason,
            // null unless the check ran: a check that could not read is not "none found"
            "undeclared": matches!(self.outcome, Outcome::Checked(_)).then(|| self.undeclared()),
        })
    }

    /// The report's section.
    pub(crate) fn render(&self) -> String {
        let key = self.key.as_deref().map(|k| format!(" of `{}`", k)).unwrap_or_default();
        let mut md = format!("\n## Undeclared exemption bindings{}\n\n", key);
        match &self.outcome {
            Outcome::Skipped => md.push_str("Not checked: `--no-live`.\n"),
            Outcome::NoOrganizationId => {
                md.push_str("**NOT CHECKED** — the estate declares no customer-organization-id.\n")
            }
            Outcome::Unavailable(why) => md.push_str(&format!("**NOT CHECKED** — {}.\n", why)),
            Outcome::Checked(c) if c.undeclared.is_empty() => md.push_str(&format!(
                "None. Cloud Asset Inventory lists {} binding(s) of the key; the estate declares {}, and each live one is among them.\n",
                c.live, self.declared
            )),
            Outcome::Checked(c) => {
                md.push_str(&format!(
                    "Cloud Asset Inventory lists {} binding(s) of the key; {} of them the estate does not declare. \
                     Each lets its target out of every policy that conditions on the value.\n\n\
                     | Value | Bound to | Claimed controls conditioned on it |\n|---|---|---|\n",
                    c.live,
                    c.undeclared.len()
                ));
                for u in &c.undeclared {
                    let mut controls = if u.controls.is_empty() { "–".to_string() } else { u.controls.join("<br>") };
                    if !u.unresolved_declared.is_empty() {
                        controls.push_str(&format!(
                            "<br><small>the estate declares {} on this value with a target satz cannot resolve before an apply</small>",
                            u.unresolved_declared.iter().map(|a| format!("`{}`", a)).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    md.push_str(&format!("| `{}` | `{}` | {} |\n", u.value, u.target, controls));
                }
            }
        }
        md
    }
}

/// Run the check for an estate. `None` when the estate declares no exemption key.
pub(crate) async fn check(manifest: &Manifest, org_id: Option<&str>, no_live: bool) -> Option<Check> {
    check_with(manifest, org_id, no_live, |org, ids| async move {
        let numbers = project_numbers(&ids)
            .await
            .map_err(|e| format!("the declared targets' project numbers could not be read: {}", e))?;
        let live = live_tag_bindings(&org)
            .await
            .map_err(|e| format!("Cloud Asset Inventory refused the tag bindings: {}", e))?;
        Ok((numbers, live))
    })
    .await
}

/// The check over a given live read — `read(org, project ids)` answers the declared
/// targets' project numbers and every `TagBinding` asset of the organisation. A read
/// that fails makes the check `Unavailable`, never an empty list.
async fn check_with<F, Fut>(manifest: &Manifest, org_id: Option<&str>, no_live: bool, read: F) -> Option<Check>
where
    F: FnOnce(String, BTreeSet<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(BTreeMap<String, String>, Vec<serde_json::Value>), String>>,
{
    let vocab = match vocabulary(manifest, org_id) {
        Ok(None) => return None,
        Ok(Some(v)) => v,
        Err(why) => return Some(Check { key: None, declared: 0, outcome: Outcome::Unavailable(why) }),
    };
    let (key, declared) = (Some(vocab.key.clone()), vocab.declared.len());
    let outcome = match org_id {
        _ if no_live => Outcome::Skipped,
        None => Outcome::NoOrganizationId,
        Some(org) => match read(org.to_string(), project_ids(&vocab)).await {
            Ok((numbers, live)) => match compare(&vocab, &live, &numbers) {
                Ok(c) => Outcome::Checked(c),
                Err(why) => Outcome::Unavailable(why),
            },
            Err(why) => Outcome::Unavailable(why),
        },
    };
    Some(Check { key, declared, outcome })
}

/// Attach the undeclared bindings that let one claimed control's witnesses out: each
/// witness policy that conditions on a bound value records the control on the binding,
/// and the row gets `{value, target, policy}` per pair.
pub(crate) fn attach(
    check: &mut Check,
    control: &str,
    witnesses: &[String],
    conditioned: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<serde_json::Value> {
    let mut row = Vec::new();
    for u in check.undeclared_mut() {
        for w in witnesses {
            if conditioned.get(w).is_some_and(|vs| vs.contains(&u.value) || vs.contains(&u.value_id)) {
                u.controls.push(format!("{} (`{}`)", control, w));
                row.push(serde_json::json!({ "value": u.value, "target": u.target, "policy": w }));
            }
        }
    }
    row
}

/// Per org-policy address, the exemption values its conditional rules name.
pub(crate) fn conditioned_policies(manifest: &Manifest, org_id: Option<&str>) -> BTreeMap<String, BTreeSet<String>> {
    let Ok(Some(v)) = vocabulary(manifest, org_id) else { return BTreeMap::new() };
    manifest
        .of_type("google_org_policy_policy")
        .map(|p| (p.address(), conditioned_values(p, &v)))
        .filter(|(_, vs)| !vs.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estate: the exemption pack's key with two values, a CIS-shaped policy whose
    /// conditional rule names one of them, a declared binding of it on the infra
    /// project, and a key of another kind with a binding of its own.
    fn estate() -> Manifest {
        Manifest::parse(
            r#"
resource "google_tags_tag_key" "exemption" {
  parent     = "organizations/123456789012"
  short_name = "acme-exemption"
}
resource "google_tags_tag_value" "exempt_service_account_keys" {
  parent     = google_tags_tag_key.exemption.name
  short_name = "service-account-keys"
}
resource "google_tags_tag_value" "exempt_public_storage" {
  parent     = "${google_tags_tag_key.exemption.name}"
  short_name = "public-storage"
}
resource "google_tags_tag_key" "env" {
  parent     = "organizations/123456789012"
  short_name = "env"
}
resource "google_tags_tag_value" "prod" {
  parent     = google_tags_tag_key.env.name
  short_name = "prod"
}
resource "google_tags_tag_binding" "infra_keys" {
  parent    = "//cloudresourcemanager.googleapis.com/projects/acme-infra-001"
  tag_value = "${google_tags_tag_value.exempt_service_account_keys.name}"
}
resource "google_tags_tag_binding" "infra_prod" {
  parent    = "//cloudresourcemanager.googleapis.com/projects/acme-infra-001"
  tag_value = "${google_tags_tag_value.prod.name}"
}
resource "google_org_policy_policy" "sa_keys" {
  name   = "organizations/123456789012/policies/iam.managed.disableServiceAccountKeyCreation"
  parent = "organizations/123456789012"
  spec {
    rules {
      enforce = "FALSE"
      condition {
        title      = "exempted service accounts"
        expression = "resource.matchTagId('${google_tags_tag_key.exemption.name}', '${google_tags_tag_value.exempt_service_account_keys.name}')"
      }
    }
    rules {
      enforce = "TRUE"
    }
  }
}
"#,
        )
    }

    /// The asset Cloud Asset Inventory returned for a binding on a project, recorded
    /// on a test organisation and rewritten to the example values.
    fn recorded(value: &str, value_id: &str, target: &str) -> serde_json::Value {
        serde_json::json!({
            "tagKey": "tagKeys/281480000000001",
            "tagValue": value_id,
            "tagValueNamespacedName": value,
            "parent": target,
            "name": format!("tagBindings/{}/{}", target.replace('/', "%2F"), value_id),
        })
    }

    const INFRA: &str = "//cloudresourcemanager.googleapis.com/projects/100000000001";

    fn numbers() -> BTreeMap<String, String> {
        BTreeMap::from([("acme-infra-001".to_string(), "100000000001".to_string())])
    }

    #[test]
    fn the_vocabulary_is_the_exemption_key_and_its_values_only() {
        let v = vocabulary(&estate(), None).expect("reads").expect("declared");
        assert_eq!(v.key, "123456789012/acme-exemption");
        assert_eq!(v.values.len(), 2, "{:?}", v.values);
        assert_eq!(v.declared.len(), 1, "the env binding is not an exemption: {:?}", v.declared);
        assert_eq!(v.declared[0].value, BoundValue::Named("123456789012/acme-exemption/service-account-keys".into()));
        assert_eq!(project_ids(&v), BTreeSet::from(["acme-infra-001".to_string()]));
    }

    #[test]
    fn an_estate_without_the_key_has_nothing_to_check() {
        let m = Manifest::parse(r#"resource "google_tags_tag_key" "env" {
  parent     = "organizations/123456789012"
  short_name = "env"
}"#);
        assert_eq!(vocabulary(&m, Some("123456789012")), Ok(None));
    }

    #[test]
    fn a_declared_binding_is_subtracted_by_its_project_number() {
        let v = vocabulary(&estate(), None).unwrap().unwrap();
        let live = [recorded("123456789012/acme-exemption/service-account-keys", "tagValues/281480000000002", INFRA)];
        let c = compare(&v, &live, &numbers()).expect("compares");
        assert_eq!(c.live, 1);
        assert!(c.undeclared.is_empty(), "{:?}", c.undeclared);
    }

    /// Without the number the declared target does not match, and the binding is
    /// reported rather than assumed to be the declared one.
    #[test]
    fn an_unresolved_project_number_does_not_match() {
        let v = vocabulary(&estate(), None).unwrap().unwrap();
        let live = [recorded("123456789012/acme-exemption/service-account-keys", "tagValues/281480000000002", INFRA)];
        assert_eq!(compare(&v, &live, &BTreeMap::new()).unwrap().undeclared.len(), 1);
    }

    #[test]
    fn an_undeclared_binding_is_reported_with_its_value_and_target() {
        let v = vocabulary(&estate(), None).unwrap().unwrap();
        let bucket = "//storage.googleapis.com/projects/_/buckets/acme-public-site";
        let live = [
            recorded("123456789012/acme-exemption/service-account-keys", "tagValues/281480000000002", INFRA),
            recorded("123456789012/acme-exemption/public-storage", "tagValues/281480000000003", bucket),
            // the declared value, bound somewhere the estate does not say
            recorded("123456789012/acme-exemption/service-account-keys", "tagValues/281480000000002",
                "//cloudresourcemanager.googleapis.com/projects/200000000002"),
        ];
        let c = compare(&v, &live, &numbers()).unwrap();
        assert_eq!(c.live, 3);
        let got: Vec<(&str, &str)> = c.undeclared.iter().map(|u| (u.value.as_str(), u.target.as_str())).collect();
        assert_eq!(
            got,
            [
                ("123456789012/acme-exemption/public-storage", bucket),
                ("123456789012/acme-exemption/service-account-keys", "//cloudresourcemanager.googleapis.com/projects/200000000002"),
            ]
        );
    }

    /// A tag of another key is not an exemption, and neither is a key whose name only
    /// starts like the exemption key's.
    #[test]
    fn a_binding_of_another_key_is_ignored() {
        let v = vocabulary(&estate(), None).unwrap().unwrap();
        let live = [
            recorded("123456789012/env/prod", "tagValues/281480000000009", INFRA),
            recorded("123456789012/acme-exemption-old/public-storage", "tagValues/281480000000010", INFRA),
        ];
        let c = compare(&v, &live, &numbers()).unwrap();
        assert_eq!((c.live, c.undeclared.len()), (0, 0));
    }

    #[test]
    fn a_policy_conditions_on_the_values_its_expression_names() {
        let m = estate();
        let v = vocabulary(&m, None).unwrap().unwrap();
        let p = &m.resources["google_org_policy_policy.sa_keys"];
        assert_eq!(
            conditioned_values(p, &v),
            BTreeSet::from(["123456789012/acme-exemption/service-account-keys".to_string()])
        );
        let literal = Manifest::parse(r#"resource "google_org_policy_policy" "q" {
  spec {
    rules {
      enforce = "FALSE"
      condition { expression = "resource.matchTag('123456789012/acme-exemption', 'public-storage') || resource.matchTagId('tagKeys/1', 'tagValues/77')" }
    }
    rules { enforce = "TRUE" }
  }
}"#);
        assert_eq!(
            conditioned_values(&literal.resources["google_org_policy_policy.q"], &v),
            BTreeSet::from(["123456789012/acme-exemption/public-storage".to_string(), "tagValues/77".to_string()])
        );
    }

    /// A target satz cannot resolve is not compared, and the live binding it may be is
    /// reported with that declaration named beside it.
    #[test]
    fn an_unresolvable_declared_target_is_named_beside_the_live_binding() {
        let m = Manifest::parse(r#"
resource "google_tags_tag_key" "exemption" {
  parent     = "organizations/123456789012"
  short_name = "acme-exemption"
}
resource "google_tags_tag_value" "v" {
  parent     = google_tags_tag_key.exemption.name
  short_name = "vm-image"
}
resource "google_tags_tag_binding" "b" {
  parent    = "//compute.googleapis.com/projects/acme-infra-001/zones/europe-west3-a/instances/${google_compute_instance.x.instance_id}"
  tag_value = "${google_tags_tag_value.v.name}"
}"#);
        let v = vocabulary(&m, None).unwrap().unwrap();
        assert!(v.declared[0].parent.is_err());
        let live = [recorded("123456789012/acme-exemption/vm-image", "tagValues/5", "//compute.googleapis.com/projects/100000000001/zones/europe-west3-a/instances/9")];
        let c = compare(&v, &live, &numbers()).unwrap();
        assert_eq!(c.undeclared[0].unresolved_declared, ["google_tags_tag_binding.b"]);
    }

    #[test]
    fn a_folder_resolves_through_its_import_id() {
        let mut m = Manifest::parse(r#"
resource "google_folder" "shared" {
  display_name = "Shared"
}
resource "google_tags_tag_key" "exemption" {
  parent     = "organizations/123456789012"
  short_name = "acme-exemption"
}
resource "google_tags_tag_value" "v" {
  parent     = google_tags_tag_key.exemption.name
  short_name = "encryption"
}
resource "google_tags_tag_binding" "b" {
  parent    = "//cloudresourcemanager.googleapis.com/${google_folder.shared.name}"
  tag_value = "${google_tags_tag_value.v.name}"
}"#);
        let v = vocabulary(&m, None).unwrap().unwrap();
        assert!(v.declared[0].parent.is_err(), "no import id: not known before an apply");
        m.resources.get_mut("google_folder.shared").unwrap().import_id = Some("folders/123456789".into());
        let v = vocabulary(&m, None).unwrap().unwrap();
        assert_eq!(v.declared[0].parent, Ok("//cloudresourcemanager.googleapis.com/folders/123456789".into()));
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().expect("a runtime").block_on(f)
    }

    /// A refused read is a failure the report states, never "no undeclared binding".
    #[test]
    fn a_refused_inventory_read_fails_the_section() {
        let c = block_on(check_with(&estate(), Some("123456789012"), false, |_, _| async {
            Err("Cloud Asset Inventory refused the tag bindings: PERMISSION_DENIED".to_string())
        }))
        .expect("the estate declares the key");
        assert!(matches!(c.outcome, Outcome::Unavailable(ref why) if why.contains("PERMISSION_DENIED")), "{:?}", c.outcome);
        let j = c.to_json();
        assert_eq!(j["status"], "unavailable");
        assert!(j["undeclared"].is_null(), "a failed read reports no list: {}", j);
        let md = c.render();
        assert!(md.contains("**NOT CHECKED**") && md.contains("PERMISSION_DENIED"), "{}", md);
        assert!(!md.contains("None."), "{}", md);
    }

    #[test]
    fn a_completed_read_reports_its_counts_and_bindings() {
        let bucket = "//storage.googleapis.com/projects/_/buckets/acme-public-site";
        let c = block_on(check_with(&estate(), Some("123456789012"), false, |org, ids| async move {
            assert_eq!(org, "123456789012");
            assert_eq!(ids, BTreeSet::from(["acme-infra-001".to_string()]));
            Ok((
                numbers(),
                vec![
                    recorded("123456789012/acme-exemption/service-account-keys", "tagValues/281480000000002", INFRA),
                    recorded("123456789012/acme-exemption/public-storage", "tagValues/281480000000003", bucket),
                ],
            ))
        }))
        .unwrap();
        let j = c.to_json();
        assert_eq!((j["status"].as_str(), j["live"].as_u64(), j["declared"].as_u64()), (Some("checked"), Some(2), Some(1)));
        assert_eq!(j["undeclared"][0]["target"], bucket);
        assert!(c.render().contains("| `123456789012/acme-exemption/public-storage` | `//storage.googleapis.com/projects/_/buckets/acme-public-site` |"));
    }

    /// `--no-live` and a missing organisation read nothing and say so; an estate
    /// without the key has no section at all.
    #[test]
    fn nothing_is_read_without_live_or_an_organisation() {
        let never = |_: String, _: BTreeSet<String>| async { panic!("nothing may be read") };
        let skipped = block_on(check_with(&estate(), Some("123456789012"), true, never)).unwrap();
        assert_eq!(skipped.to_json()["status"], "skipped");
        let no_org_estate = Manifest::parse(r#"resource "google_tags_tag_key" "exemption" { short_name = "acme-exemption" }"#);
        let c = block_on(check_with(&no_org_estate, None, false, never)).unwrap();
        assert!(matches!(c.outcome, Outcome::Unavailable(_)), "the key names no organisation: {:?}", c.outcome);
        let c = block_on(check_with(&estate(), None, false, never)).unwrap();
        assert_eq!(c.outcome, Outcome::NoOrganizationId);
        let none = Manifest::parse(r#"resource "google_tags_tag_key" "env" { short_name = "env" }"#);
        assert!(block_on(check_with(&none, Some("123456789012"), false, never)).is_none());
    }

    #[test]
    fn a_binding_asset_without_a_value_name_is_an_error() {
        let v = vocabulary(&estate(), None).unwrap().unwrap();
        assert!(compare(&v, &[serde_json::json!({"parent": INFRA})], &numbers()).is_err());
    }
}
