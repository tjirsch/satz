//! Cloud Identity groups: the `!import-include` transpile-time import hook.
//!
//! `google_cloud_identity_group` resources are declared by name in a preset (see
//! `presets/security-group-models/`), but Terraform can only adopt an existing group if it
//! is told the group's opaque `groups/<id>` name. Historically that meant looking each
//! group up in the admin console and pasting the id into an `import-id:` line. This module
//! does that lookup over the Cloud Identity API instead, so a preset stays free of
//! tenant-specific ids.
//!
//! Groups are simpler than org policies: there is no "managed constraint" activation step.
//! A group either exists (import it) or does not (leave it for `tofu apply` to create).
//!
//! The pure layer (`desired_groups_from_config`) is IO-free and unit-tested;
//! `CloudIdentityClient` is the only IO surface.
//!
//! **Status after M3 (2026-08-23):** the `!import-include` hook was a YAML-dialect
//! feature and is no longer reachable — satz reads Satz, which has no
//! equivalent yet. The code below is deliberately NOT deleted: it is the working
//! implementation of live group lookup / constraint activation / import that the
//! Satz adoption mechanism (R1) needs, and rebuilding it from scratch to satisfy a
//! dead-code warning would be pure churn. It is dead until R1 adopts it; if R1
//! lands with a different design, THEN delete it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::Config;
use crate::include_processor::IncludeBinding;
use crate::transpiler::{
    aggregate_group_members, group_email, group_resource_address, member_email,
    membership_resource_address,
};
use crate::ToolConfig;

type BoxErr = Box<dyn std::error::Error>;

const CLOUD_IDENTITY_HOST: &str = "https://cloudidentity.googleapis.com";

/// The YAML key a groups preset is included under. The long `google_cloud_identity_group`
/// form is a different (generic, raw-Terraform) code path and is not importable this way.
pub(crate) const GROUP_WRAPPER_KEY: &str = "cloud_identity_group";

// ---------------------------------------------------------------------------
// Pure layer
// ---------------------------------------------------------------------------

/// A group the config declares, resolved to everything needed to import it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredGroup {
    pub yaml_key: String,
    /// `group_key.id` — the email the API is queried with.
    pub email: String,
    /// Terraform address the group is imported to.
    pub address: String,
    /// Only the members this config declares. Members that exist live but are not listed
    /// here are deliberately left unmanaged, so importing can never make `tofu apply`
    /// propose deleting somebody the config never mentioned.
    pub memberships: Vec<DesiredMembership>,
}

/// A single `member`/`manager`/`owner` entry, resolved to its Terraform address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredMembership {
    /// The raw YAML string (`user:a@example.com`), which is what the resource label hashes.
    pub member_raw: String,
    /// The bare email the API is queried with.
    pub email: String,
    pub address: String,
}

/// Read `cloud_identity_group` out of a parsed preset config.
///
/// The key has no `Config` field of its own, so it arrives in the `extra` catch-all.
/// Email and address derivation are the transpiler's, not re-implemented here — the
/// importer must address exactly what the generated HCL declares.
pub(crate) fn desired_groups_from_config(config: &Config) -> Vec<DesiredGroup> {
    let customer_domain = config
        .extra
        .get("customer-domain")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let Some(serde_yaml::Value::Mapping(groups)) = config.extra.get(GROUP_WRAPPER_KEY) else {
        return Vec::new();
    };

    groups
        .iter()
        .filter_map(|(k, v)| match (k, v) {
            (serde_yaml::Value::String(name), serde_yaml::Value::Mapping(attrs)) => {
                let memberships = aggregate_group_members(attrs)
                    .into_keys()
                    .map(|member_raw| DesiredMembership {
                        email: member_email(&member_raw).to_string(),
                        address: membership_resource_address(name, &member_raw),
                        member_raw,
                    })
                    .collect();
                Some(DesiredGroup {
                    yaml_key: name.clone(),
                    email: group_email(name, attrs, customer_domain),
                    address: group_resource_address(name),
                    memberships,
                })
            }
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// IO layer
// ---------------------------------------------------------------------------

/// Why a lookup could not answer "does this group exist".
enum LookupError {
    /// The API refused the caller. Ambiguous: some tenants return this for a group that
    /// does not exist (anti-enumeration) as well as for a genuine permission problem.
    Forbidden(String),
    Other(BoxErr),
}

pub(crate) struct CloudIdentityClient {
    http: reqwest::Client,
    token: String,
    quota_project: Option<String>,
}

/// The natural-key → live-id resolver `satz adopt` uses for groups and
/// memberships: the client plus the list-once fallback for a refused lookup.
/// `Ok(None)` is "provably absent — apply will create it"; a refusal that
/// cannot be disambiguated is an error, never "absent".
pub(crate) struct GroupResolver {
    client: CloudIdentityClient,
    customer_id: String,
    listed_groups: Option<BTreeMap<String, String>>,
    listed_memberships: BTreeMap<String, BTreeMap<String, String>>,
}

impl GroupResolver {
    pub(crate) async fn new(customer_id: &str) -> Result<Self, BoxErr> {
        Ok(Self {
            client: CloudIdentityClient::new().await?,
            customer_id: customer_id.to_string(),
            listed_groups: None,
            listed_memberships: BTreeMap::new(),
        })
    }

    /// `groups/<id>` for a group email.
    pub(crate) async fn group(&mut self, email: &str) -> Result<Option<String>, String> {
        if email.ends_with('@') || !email.contains('@') {
            return Err(format!("'{}' is not a group email (set customer-domain, or an explicit id/email)", email));
        }
        match self.client.lookup_group(email).await {
            Ok(found) => Ok(found),
            Err(LookupError::Other(e)) => Err(format!("groups:lookup {}: {}", email, e)),
            Err(LookupError::Forbidden(body)) => {
                if self.listed_groups.is_none() {
                    if self.customer_id.is_empty() {
                        return Err(format!(
                            "groups:lookup denied for {} and customer-id is unset, so the tenant cannot be listed instead \
                             (enable cloudidentity.googleapis.com, grant roles/cloudidentity.groups.readonly): {}",
                            email,
                            body.trim()
                        ));
                    }
                    let map = self.client.list_groups(&self.customer_id).await.map_err(|e| {
                        format!("groups:lookup denied for {} and listing customers/{} failed: {}", email, self.customer_id, e)
                    })?;
                    self.listed_groups = Some(map);
                }
                Ok(self.listed_groups.as_ref().and_then(|m| m.get(&email.to_lowercase())).cloned())
            }
        }
    }

    /// `groups/<g>/memberships/<m>` for a member email of `group_name`.
    pub(crate) async fn membership(&mut self, group_name: &str, email: &str) -> Result<Option<String>, String> {
        match self.client.lookup_membership(group_name, email).await {
            Ok(found) => Ok(found),
            Err(LookupError::Other(e)) => Err(format!("memberships:lookup {} in {}: {}", email, group_name, e)),
            Err(LookupError::Forbidden(_)) => {
                if !self.listed_memberships.contains_key(group_name) {
                    let map = self
                        .client
                        .list_memberships(group_name)
                        .await
                        .map_err(|e| format!("memberships:lookup denied and listing {} failed: {}", group_name, e))?;
                    self.listed_memberships.insert(group_name.to_string(), map);
                }
                Ok(self.listed_memberships[group_name].get(&email.to_lowercase()).cloned())
            }
        }
    }
}

impl CloudIdentityClient {
    pub(crate) async fn new() -> Result<Self, BoxErr> {
        use google_cloud_auth::credentials::Builder;
        // `cloud-platform` covers cloudidentity.googleapis.com. Deliberately not also
        // requesting the narrower cloud-identity.groups scopes: user ADC refresh tokens
        // are minted pre-scoped by `gcloud auth application-default login` and ignore
        // what is asked for here, while a service account that was never granted the
        // narrow scopes would start failing.
        let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
        let credentials = Builder::default()
            .with_scopes(scopes)
            .build_access_token_credentials()?;
        let token = credentials.access_token().await?;

        Ok(Self {
            http: reqwest::Client::new(),
            token: token.token,
            quota_project: crate::org_policy::resolve_quota_project(),
        })
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.bearer_auth(&self.token);
        match &self.quota_project {
            Some(qp) => rb.header("x-goog-user-project", qp),
            None => rb,
        }
    }

    /// Resolve a group email to its `groups/<id>` resource name. `Ok(None)` means the
    /// group provably does not exist.
    async fn lookup_group(&self, email: &str) -> Result<Option<String>, LookupError> {
        let url = format!("{}/v1/groups:lookup", CLOUD_IDENTITY_HOST);
        let res = self
            .auth(self.http.get(&url))
            .query(&[("groupKey.id", email)])
            .send()
            .await
            .map_err(|e| LookupError::Other(e.into()))?;

        let status = res.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if status.as_u16() == 403 {
            return Err(LookupError::Forbidden(res.text().await.unwrap_or_default()));
        }
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(LookupError::Other(
                format!("groups:lookup {} failed ({}): {}", email, status, body).into(),
            ));
        }

        let json: Value = res.json().await.map_err(|e| LookupError::Other(e.into()))?;
        Ok(json
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    /// Resolve a member of `group_name` (a `groups/<id>`) to its
    /// `groups/<id>/memberships/<id>` resource name. `Ok(None)` means not a member.
    async fn lookup_membership(
        &self,
        group_name: &str,
        member_email: &str,
    ) -> Result<Option<String>, LookupError> {
        let url = format!("{}/v1/{}/memberships:lookup", CLOUD_IDENTITY_HOST, group_name);
        let res = self
            .auth(self.http.get(&url))
            .query(&[("memberKey.id", member_email)])
            .send()
            .await
            .map_err(|e| LookupError::Other(e.into()))?;

        let status = res.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if status.as_u16() == 403 {
            return Err(LookupError::Forbidden(res.text().await.unwrap_or_default()));
        }
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(LookupError::Other(
                format!(
                    "memberships:lookup {} in {} failed ({}): {}",
                    member_email, group_name, status, body
                )
                .into(),
            ));
        }

        let json: Value = res.json().await.map_err(|e| LookupError::Other(e.into()))?;
        Ok(json
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    /// Every membership of `group_name`, as `member email -> groups/<id>/memberships/<id>`.
    /// The fallback for a refused `memberships:lookup`, mirroring `list_groups`.
    async fn list_memberships(&self, group_name: &str) -> Result<BTreeMap<String, String>, BoxErr> {
        let mut out = BTreeMap::new();
        let mut page_token: Option<String> = None;
        loop {
            let url = format!("{}/v1/{}/memberships", CLOUD_IDENTITY_HOST, group_name);
            let mut req = self.auth(self.http.get(&url)).query(&[("view", "BASIC")]);
            if let Some(tok) = &page_token {
                req = req.query(&[("pageToken", tok)]);
            }
            let res = req.send().await?;
            if !res.status().is_success() {
                let status = res.status();
                let body = res.text().await.unwrap_or_default();
                return Err(
                    format!("memberships.list {} failed ({}): {}", group_name, status, body).into(),
                );
            }
            let json: Value = res.json().await?;
            if let Some(arr) = json.get("memberships").and_then(|m| m.as_array()) {
                for m in arr {
                    let name = m.get("name").and_then(|v| v.as_str());
                    let email = m
                        .get("preferredMemberKey")
                        .and_then(|k| k.get("id"))
                        .and_then(|v| v.as_str());
                    if let (Some(name), Some(email)) = (name, email) {
                        out.insert(email.to_lowercase(), name.to_string());
                    }
                }
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

    /// Every group in the customer's tenant, as `email -> groups/<id>`. Used only as the
    /// fallback when `lookup_group` is refused, so one 403 does not force every group to
    /// be skipped.
    async fn list_groups(&self, customer_id: &str) -> Result<BTreeMap<String, String>, BoxErr> {
        let mut out = BTreeMap::new();
        let mut page_token: Option<String> = None;
        let parent = format!("customers/{}", customer_id);
        loop {
            let url = format!("{}/v1/groups", CLOUD_IDENTITY_HOST);
            let mut req = self
                .auth(self.http.get(&url))
                .query(&[("parent", parent.as_str()), ("view", "BASIC")]);
            if let Some(tok) = &page_token {
                req = req.query(&[("pageToken", tok)]);
            }
            let res = req.send().await?;
            if !res.status().is_success() {
                let status = res.status();
                let body = res.text().await.unwrap_or_default();
                return Err(format!("groups.list {} failed ({}): {}", parent, status, body).into());
            }
            let json: Value = res.json().await?;
            if let Some(arr) = json.get("groups").and_then(|g| g.as_array()) {
                for g in arr {
                    let name = g.get("name").and_then(|v| v.as_str());
                    let email = g.get("groupKey").and_then(|k| k.get("id")).and_then(|v| v.as_str());
                    if let (Some(name), Some(email)) = (name, email) {
                        out.insert(email.to_lowercase(), name.to_string());
                    }
                }
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

// ---------------------------------------------------------------------------
// Transpile-time hook
// ---------------------------------------------------------------------------

/// Import the Cloud Identity groups contributed by `!import-include` directives.
///
/// Called by `transpile` after the HCL has been written and `tofu init` has run. For each
/// declared group: if it exists live it is `tofu import`ed into state; if not it is left
/// for the user's `tofu apply` to create. Idempotent and safe to re-run.
pub(crate) async fn run_import_includes(
    bindings: &[IncludeBinding],
    config_path: &Path,
    include_paths: &[PathBuf],
    runtime_config: &ToolConfig,
    hcl_dir: &Path,
) -> Result<(), BoxErr> {
    let import_paths: Vec<&PathBuf> = bindings.iter().map(|b| &b.path).collect();
    if import_paths.is_empty() {
        return Ok(());
    }

    println!(
        "\n!import-include: importing Cloud Identity groups from {} preset(s)...",
        import_paths.len()
    );

    let vars = crate::org_policy::resolve_config_vars(config_path, include_paths)?;

    // Gather desired groups from every !import-include preset, de-duplicated by YAML key
    // so the same preset included twice does not import twice.
    let mut desired: BTreeMap<String, DesiredGroup> = BTreeMap::new();
    for path in &import_paths {
        let config = crate::org_policy::resolve_preset_config(
            path,
            GROUP_WRAPPER_KEY,
            &vars,
            include_paths,
        )?;
        for dg in desired_groups_from_config(&config) {
            desired.insert(dg.yaml_key.clone(), dg);
        }
    }
    if desired.is_empty() {
        println!("!import-include: no cloud_identity_group entries found; nothing to import.");
        return Ok(());
    }

    let customer_id = vars
        .get("customer-id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let client = CloudIdentityClient::new().await?;

    // Populated only if a lookup is refused; see LookupError::Forbidden.
    let mut listed: Option<BTreeMap<String, String>> = None;

    let (mut imported, mut create_on_apply, mut skipped) = (0usize, 0usize, 0usize);
    let (mut m_imported, mut m_create_on_apply, mut m_skipped) = (0usize, 0usize, 0usize);
    for dg in desired.values() {
        // An empty customer-domain yields "name@", which is not a group key worth querying.
        if dg.email.ends_with('@') || !dg.email.contains('@') {
            eprintln!(
                "  warning: skipping {} — derived group key '{}' is not a valid email \
                 (set 'customer-domain', or an explicit id/email on the group)",
                dg.yaml_key, dg.email
            );
            skipped += 1;
            continue;
        }

        let name = match client.lookup_group(&dg.email).await {
            Ok(found) => found,
            Err(LookupError::Other(e)) => {
                eprintln!("  warning: lookup failed for {}: {}", dg.email, e);
                skipped += 1;
                continue;
            }
            Err(LookupError::Forbidden(body)) => {
                // 403 is ambiguous — it can mean "no permission" or, in some tenants,
                // "does not exist". Listing the tenant once disambiguates it for this and
                // every remaining group; treating it as absent would let `tofu apply` try
                // to create a group that is already there.
                if listed.is_none() {
                    if customer_id.is_empty() {
                        eprintln!(
                            "  warning: groups:lookup was denied for {} and 'customer-id' is not \
                             set, so the group list cannot be used instead. Enable \
                             cloudidentity.googleapis.com and grant \
                             roles/cloudidentity.groups.readonly. ({})",
                            dg.email,
                            body.trim()
                        );
                        skipped += 1;
                        continue;
                    }
                    match client.list_groups(&customer_id).await {
                        Ok(map) => {
                            println!(
                                "  note: groups:lookup denied; listed {} group(s) in customers/{} instead",
                                map.len(),
                                customer_id
                            );
                            listed = Some(map);
                        }
                        Err(e) => {
                            eprintln!(
                                "  warning: groups:lookup denied for {} and listing failed too: {}. \
                                 Enable cloudidentity.googleapis.com and grant \
                                 roles/cloudidentity.groups.readonly.",
                                dg.email, e
                            );
                            skipped += 1;
                            continue;
                        }
                    }
                }
                listed
                    .as_ref()
                    .and_then(|m| m.get(&dg.email.to_lowercase()))
                    .cloned()
            }
        };

        let Some(name) = name else {
            // The group does not exist yet, so neither do its memberships.
            create_on_apply += 1;
            m_create_on_apply += dg.memberships.len();
            continue;
        };

        if crate::bootstrap::run_import(&runtime_config.tf_tool, hcl_dir, &dg.address, &name) {
            imported += 1;
        } else {
            skipped += 1;
        }

        // Only the members this config declares — see DesiredGroup::memberships.
        let (mi, mc, ms) =
            import_declared_memberships(&client, dg, &name, runtime_config, hcl_dir).await;
        m_imported += mi;
        m_create_on_apply += mc;
        m_skipped += ms;
    }

    println!(
        "!import-include: groups imported={}, create-on-apply={}, skipped={}; \
         memberships imported={}, create-on-apply={}, skipped={}. \
         Run `{} -chdir={} apply` to roll out changes.",
        imported,
        create_on_apply,
        skipped,
        m_imported,
        m_create_on_apply,
        m_skipped,
        runtime_config.tf_tool,
        hcl_dir.display()
    );
    if imported > 0 {
        println!(
            "  note: only the members declared in the config are imported. Anyone else who is \
             already in one of these groups stays unmanaged — apply will not remove them."
        );
    }
    Ok(())
}

/// Import the memberships `dg` declares that already exist in the live group `group_name`.
/// Returns `(imported, create_on_apply, skipped)`.
///
/// Undeclared live members are never looked at: adopting them would make the next
/// `tofu apply` propose deleting people the config does not mention.
async fn import_declared_memberships(
    client: &CloudIdentityClient,
    dg: &DesiredGroup,
    group_name: &str,
    runtime_config: &ToolConfig,
    hcl_dir: &Path,
) -> (usize, usize, usize) {
    let (mut imported, mut create_on_apply, mut skipped) = (0usize, 0usize, 0usize);
    // Same 403-is-ambiguous problem as groups; list this group's members once if refused.
    let mut listed: Option<BTreeMap<String, String>> = None;

    for m in &dg.memberships {
        if !m.email.contains('@') {
            eprintln!(
                "  warning: skipping member '{}' of {} — not an email",
                m.member_raw, dg.yaml_key
            );
            skipped += 1;
            continue;
        }

        let name = match client.lookup_membership(group_name, &m.email).await {
            Ok(found) => found,
            Err(LookupError::Other(e)) => {
                eprintln!("  warning: membership lookup failed for {}: {}", m.email, e);
                skipped += 1;
                continue;
            }
            Err(LookupError::Forbidden(body)) => {
                if listed.is_none() {
                    match client.list_memberships(group_name).await {
                        Ok(map) => listed = Some(map),
                        Err(e) => {
                            eprintln!(
                                "  warning: memberships:lookup denied for {} and listing {} \
                                 failed too: {} ({})",
                                m.email,
                                dg.yaml_key,
                                e,
                                body.trim()
                            );
                            skipped += 1;
                            continue;
                        }
                    }
                }
                listed
                    .as_ref()
                    .and_then(|map| map.get(&m.email.to_lowercase()))
                    .cloned()
            }
        };

        match name {
            Some(name) => {
                if crate::bootstrap::run_import(
                    &runtime_config.tf_tool,
                    hcl_dir,
                    &m.address,
                    &name,
                ) {
                    imported += 1;
                } else {
                    skipped += 1;
                }
            }
            None => create_on_apply += 1,
        }
    }

    (imported, create_on_apply, skipped)
}

// ---------------------------------------------------------------------------
// Tests (pure layer only — no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, serde_yaml::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), serde_yaml::Value::String(v.to_string())))
            .collect()
    }

    /// The shipped group preset is the real shape this feature has to handle: its keys are
    /// YAML aliases, it declares its own `variables:` block, its values use `!format`, and
    /// those `!format` args reference anchors that only the *including* config defines.
    /// All four have to survive being wrapped under `cloud_identity_group:`.
    #[test]
    fn group_preset_resolves_with_merged_variables() {
        let preset = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("presets/security-group-models/s1-group-definitions.satz");
        let vars = vars(&[
            ("customer-domain", "example.com"),
            ("customer-id", "C01234567"),
            ("svc-iac-account", "svc-iac"),
            ("infra-project-name", "infra-001"),
            ("first-admin", "admin"),
        ]);

        let config = resolve_preset_config_for_test(&preset, &vars);
        let groups = desired_groups_from_config(&config);

        assert_eq!(groups.len(), 5, "preset declares five groups: {:?}", groups);

        let admins = groups
            .iter()
            .find(|g| g.yaml_key == "gcp-organization-admins")
            .expect("gcp-organization-admins present");
        assert_eq!(admins.email, "gcp-organization-admins@example.com");
        assert_eq!(
            admins.address,
            "google_cloud_identity_group.gcp_organization_admins"
        );

        // The preset's own `variables:` block must not survive as a phantom group.
        assert!(
            !groups.iter().any(|g| g.yaml_key == "variables"),
            "variables block leaked into the group set: {:?}",
            groups
        );
    }

    fn resolve_preset_config_for_test(
        preset: &Path,
        vars: &HashMap<String, serde_yaml::Value>,
    ) -> Config {
        crate::org_policy::resolve_preset_config(preset, GROUP_WRAPPER_KEY, vars, &[])
            .expect("preset resolves")
    }

    /// The variable table is extracted from the *include-expanded* config, so it already
    /// contains the preset's own variables. Re-emitting those as anchors used to make every
    /// aliased group key resolve to the last-defined anchor, collapsing all five groups onto
    /// one key and failing the parse with a duplicate-entry error.
    #[test]
    fn preset_owned_anchors_are_not_redefined_by_the_prelude() {
        let preset = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("presets/security-group-models/s1-group-definitions.satz");
        let vars = vars(&[
            ("customer-domain", "example.com"),
            ("customer-id", "C01234567"),
            ("svc-iac-account", "svc-iac"),
            ("infra-project-name", "infra-001"),
            ("first-admin", "admin"),
            // These are the preset's own anchors, echoed back as they would be in a real run.
            ("gcp-organization-admins-name", "gcp-organization-admins"),
            ("gcp-project-admins-name", "gcp-project-admins"),
            ("gcp-security-admins-name", "gcp-security-admins"),
            ("gcp-security-viewers-name", "gcp-security-viewers"),
            ("gcp-billing-admins-name", "gcp-billing-admins"),
        ]);

        let config = resolve_preset_config_for_test(&preset, &vars);
        let groups = desired_groups_from_config(&config);
        assert_eq!(groups.len(), 5, "all five groups survive: {:?}", groups);
    }

    #[test]
    fn explicit_id_and_email_win_over_the_derived_default() {
        let yaml = r#"
customer-domain: example.com
cloud_identity_group:
  derived:
    display_name: D
  explicit-email:
    email: real@example.org
  explicit-id:
    id: id@example.org
    email: ignored@example.org
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let groups = desired_groups_from_config(&config);
        let by_key = |k: &str| groups.iter().find(|g| g.yaml_key == k).unwrap().email.clone();

        assert_eq!(by_key("derived"), "derived@example.com");
        assert_eq!(by_key("explicit-email"), "real@example.org");
        assert_eq!(by_key("explicit-id"), "id@example.org");
    }

    /// Only what the config declares gets an address. A member who exists live but is not
    /// listed here must never be looked up, or apply would propose deleting them.
    #[test]
    fn only_declared_members_become_memberships() {
        let yaml = r#"
customer-domain: example.com
cloud_identity_group:
  admins:
    display_name: A
    owner:
      - "serviceAccount:svc@p.iam.gserviceaccount.com"
    member:
      - "user:a@example.com"
      - "user:b@example.com"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let groups = desired_groups_from_config(&config);
        let g = &groups[0];

        let mut emails: Vec<_> = g.memberships.iter().map(|m| m.email.clone()).collect();
        emails.sort();
        assert_eq!(
            emails,
            vec!["a@example.com", "b@example.com", "svc@p.iam.gserviceaccount.com"],
            "prefixes stripped, all three declared members present"
        );
    }

    /// A member listed under two role keys is one resource in the HCL, so it must be one
    /// import here too — a second import of the same address would fail.
    #[test]
    fn a_member_in_two_role_lists_is_imported_once() {
        let yaml = r#"
customer-domain: example.com
cloud_identity_group:
  admins:
    display_name: A
    member:
      - "user:a@example.com"
    manager:
      - "user:a@example.com"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let groups = desired_groups_from_config(&config);
        assert_eq!(groups[0].memberships.len(), 1);
    }

    /// The importer must address exactly the label the transpiler emitted; the label is a
    /// hash, so a drift here would silently import nothing.
    #[test]
    fn membership_address_matches_the_generated_resource() {
        let yaml = r#"
customer-domain: example.com
cloud_identity_group:
  gcp-billing-admins:
    display_name: B
    member:
      - "user:a@example.com"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let groups = desired_groups_from_config(&config);
        let addr = &groups[0].memberships[0].address;
        assert_eq!(
            *addr,
            crate::transpiler::membership_resource_address("gcp-billing-admins", "user:a@example.com")
        );
        assert!(
            addr.starts_with("google_cloud_identity_group_membership.membership_gcp_billing_admins_"),
            "unexpected address: {addr}"
        );
    }

    #[test]
    fn shipped_preset_groups_carry_their_declared_members() {
        let preset = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("presets/security-group-models/s1-group-definitions.satz");
        let vars = vars(&[
            ("customer-domain", "example.com"),
            ("customer-id", "C01234567"),
            ("svc-iac-account", "svc-iac"),
            ("infra-project-name", "infra-001"),
            ("first-admin", "admin"),
        ]);
        let config = resolve_preset_config_for_test(&preset, &vars);
        let groups = desired_groups_from_config(&config);

        for g in &groups {
            // v1.1: presets define groups, humans grant membership — the only
            // shipped membership is the owning IaC service account.
            assert_eq!(g.memberships.len(), 1, "{} memberships: {:?}", g.yaml_key, g.memberships);
        }
        let admins = groups.iter().find(|g| g.yaml_key == "gcp-organization-admins").unwrap();
        let emails: Vec<_> = admins.memberships.iter().map(|m| m.email.clone()).collect();
        assert_eq!(emails, vec!["svc-iac@infra-001.iam.gserviceaccount.com"]);
    }

    #[test]
    fn missing_group_key_yields_no_groups() {
        let config: Config = serde_yaml::from_str("customer-domain: example.com\n").unwrap();
        assert!(desired_groups_from_config(&config).is_empty());
    }
}
