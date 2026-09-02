//! Cloud Identity groups: the live lookups `satz adopt` uses for groups and
//! memberships.
//!
//! `google_cloud_identity_group` resources are declared by name; Terraform can
//! only adopt an existing group if it is told the group's opaque `groups/<id>`
//! name, and a membership its `groups/<id>/memberships/<id>`. Historically that
//! meant looking each one up in the admin console and pasting the id into an
//! `"import-id"` line. `GroupResolver` does the lookup over the Cloud Identity
//! API instead — by group email and member email, the keys the emitted HCL
//! carries — so an estate stays free of tenant-specific ids until `adopt`
//! writes the verified ones in.
//!
//! Groups are simpler than org policies: there is no "managed constraint"
//! activation step. A group either exists (import it) or does not (leave it for
//! `tofu apply` to create). One subtlety survives: a refused `groups:lookup`
//! (403) is ambiguous in some tenants — permission problem or "does not
//! exist" — so the resolver lists the tenant once and answers from that.
//! Treating 403 as absent would let `apply` try to create a group that is
//! already there.

use std::collections::BTreeMap;

use serde_json::Value;


type BoxErr = Box<dyn std::error::Error>;

const CLOUD_IDENTITY_HOST: &str = "https://cloudidentity.googleapis.com";

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
        // `cloud-platform` covers cloudidentity.googleapis.com — the shared
        // `gcp::access_token()` requests exactly that scope. Deliberately not the
        // narrower cloud-identity.groups scopes: user ADC refresh tokens are minted
        // pre-scoped by `gcloud auth application-default login` and ignore what is
        // asked for here, while a service account that was never granted the narrow
        // scopes would start failing.
        let token = crate::gcp::access_token().await?;

        Ok(Self {
            http: reqwest::Client::new(),
            token,
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
