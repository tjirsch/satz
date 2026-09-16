//! Google Workspace admin roles through the Admin SDK Directory API — the one day-0
//! prerequisite IAM can neither grant nor test: the Groups Admin role the IaC service
//! account needs before it manages the estate's Cloud Identity groups.
//!
//! Assigning it runs as the operator, never as the service account (which cannot give
//! itself an admin role), and it only works when the operator's login carries the
//! role-management scope. A user's Application Default Credentials carry the scopes
//! chosen at `gcloud auth application-default login`, whatever a program requests, so
//! the default login cannot make this call. Every failure is classified into the one
//! thing the operator does next.

const ADMIN: &str = "https://admin.googleapis.com/admin/directory/v1";
const IAM: &str = "https://iam.googleapis.com/v1";
/// The Directory API's system role for groups administration.
pub(crate) const GROUPS_ADMIN_ROLE: &str = "_GROUPS_ADMIN_ROLE";
pub(crate) const SCOPE: &str = "https://www.googleapis.com/auth/admin.directory.rolemanagement";

/// The admin-console path, for when satz may not do it.
pub(crate) const CONSOLE_PATH: &str = "Google Workspace admin console → Account → Admin roles → Groups Admin → Admins → Assign service accounts";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GroupsAdmin {
    /// the service account already holds the role
    Held,
    /// satz assigned it now
    Assigned,
    /// satz could not tell or could not assign: the reason, and what to do
    NotDone(String),
}

/// Why a Google API call was refused, in the terms the operator acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// the token lacks the role-management scope: a login with it is needed
    Scope,
    /// the Admin SDK API is not enabled on the quota project
    ServiceDisabled,
    /// the caller is not allowed — not a Workspace admin who may assign roles
    Denied(String),
    /// anything else, with the body
    Other(String),
}

/// Classify a Google API error body by its `ErrorInfo` reason, falling back to status.
pub(crate) fn classify(status: u16, body: &str) -> Refusal {
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    let reasons: Vec<&str> = error["details"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["reason"].as_str())
        .chain(error["errors"].as_array().into_iter().flatten().filter_map(|e| e["reason"].as_str()))
        .collect();
    let message = error["message"].as_str().unwrap_or(body).to_string();
    if reasons.contains(&"ACCESS_TOKEN_SCOPE_INSUFFICIENT") || message.contains("insufficient authentication scopes") {
        Refusal::Scope
    } else if reasons.contains(&"SERVICE_DISABLED") || reasons.contains(&"accessNotConfigured") {
        Refusal::ServiceDisabled
    } else if status == 403 || status == 401 {
        Refusal::Denied(message)
    } else {
        Refusal::Other(format!("HTTP {}: {}", status, message))
    }
}

/// The operator's next step for a refusal, with the estate's own values.
pub(crate) fn next_step(refusal: &Refusal, sa_email: &str, quota_project: Option<&str>) -> String {
    match refusal {
        Refusal::Scope => format!(
            "your login does not carry the Workspace role-management scope. Either log in with it and run this again:\n    \
             gcloud auth application-default login --scopes=https://www.googleapis.com/auth/cloud-platform,{}\n  \
             or assign it by hand: {}: {}",
            SCOPE, CONSOLE_PATH, sa_email
        ),
        Refusal::ServiceDisabled => format!(
            "the Admin SDK API is not enabled on the quota project{} — `gcloud services enable admin.googleapis.com{}`, then run this again; or assign it by hand: {}: {}",
            quota_project.map(|p| format!(" {}", p)).unwrap_or_default(),
            quota_project.map(|p| format!(" --project {}", p)).unwrap_or_default(),
            CONSOLE_PATH,
            sa_email
        ),
        Refusal::Denied(why) => format!(
            "your account may not assign Workspace admin roles ({}) — a super admin assigns it: {}: {}",
            why, CONSOLE_PATH, sa_email
        ),
        Refusal::Other(why) => format!("{} — assign it by hand: {}: {}", why, CONSOLE_PATH, sa_email),
    }
}

/// The role id of `_GROUPS_ADMIN_ROLE` in a `roles.list` page, and the next page token.
pub(crate) fn role_id_in(page: &serde_json::Value) -> (Option<String>, Option<String>) {
    let id = page["items"].as_array().into_iter().flatten().find_map(|r| {
        (r["roleName"].as_str() == Some(GROUPS_ADMIN_ROLE))
            .then(|| r["roleId"].as_str().map(str::to_string).or_else(|| r["roleId"].as_i64().map(|n| n.to_string())))
            .flatten()
    });
    (id, page["nextPageToken"].as_str().map(str::to_string))
}

/// Whether a `roleAssignments.list` page assigns the role to `unique_id`, and the next
/// page token.
pub(crate) fn assigned_in(page: &serde_json::Value, unique_id: &str) -> (bool, Option<String>) {
    let held = page["items"].as_array().into_iter().flatten().any(|a| a["assignedTo"].as_str() == Some(unique_id));
    (held, page["nextPageToken"].as_str().map(str::to_string))
}

struct Directory {
    http: reqwest::Client,
    token: String,
    quota_project: Option<String>,
    customer: String,
}

impl Directory {
    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.bearer_auth(&self.token);
        match &self.quota_project {
            Some(qp) => rb.header("x-goog-user-project", qp),
            None => rb,
        }
    }

    async fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, Refusal> {
        let res = self.auth(self.http.get(url)).query(query).send().await.map_err(|e| Refusal::Other(e.to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(classify(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| Refusal::Other(format!("{}: {}", url, e)))
    }

    async fn role_id(&self) -> Result<String, Refusal> {
        let url = format!("{}/customer/{}/roles", ADMIN, self.customer);
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![("maxResults", "100")];
            if let Some(t) = &token {
                q.push(("pageToken", t));
            }
            let (id, next) = role_id_in(&self.get(&url, &q).await?);
            if let Some(id) = id {
                return Ok(id);
            }
            match next {
                Some(t) => token = Some(t),
                None => return Err(Refusal::Other(format!("the customer's roles list has no {}", GROUPS_ADMIN_ROLE))),
            }
        }
    }

    async fn holds(&self, role_id: &str, unique_id: &str) -> Result<bool, Refusal> {
        let url = format!("{}/customer/{}/roleassignments", ADMIN, self.customer);
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![("roleId", role_id), ("maxResults", "200")];
            if let Some(t) = &token {
                q.push(("pageToken", t));
            }
            let (held, next) = assigned_in(&self.get(&url, &q).await?, unique_id);
            if held {
                return Ok(true);
            }
            match next {
                Some(t) => token = Some(t),
                None => return Ok(false),
            }
        }
    }

    async fn assign(&self, role_id: &str, unique_id: &str) -> Result<(), Refusal> {
        let url = format!("{}/customer/{}/roleassignments", ADMIN, self.customer);
        let body = serde_json::json!({ "roleId": role_id, "assignedTo": unique_id, "scopeType": "CUSTOMER" });
        let res = self.auth(self.http.post(&url)).json(&body).send().await.map_err(|e| Refusal::Other(e.to_string()))?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(classify(status, &res.text().await.unwrap_or_default()))
    }
}

/// The service account's `uniqueId` — how the Directory API names a service account.
/// `Ok(None)`: it does not exist (yet).
async fn unique_id(http: &reqwest::Client, token: &str, quota_project: Option<&str>, sa_email: &str) -> Result<Option<String>, Refusal> {
    let url = format!("{}/projects/-/serviceAccounts/{}", IAM, sa_email);
    let mut rb = http.get(&url).bearer_auth(token);
    if let Some(qp) = quota_project {
        rb = rb.header("x-goog-user-project", qp);
    }
    let res = rb.send().await.map_err(|e| Refusal::Other(e.to_string()))?;
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(classify(status, &body));
    }
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| Refusal::Other(e.to_string()))?;
    Ok(json["uniqueId"].as_str().map(str::to_string))
}

/// Whether `sa_email` holds Groups Admin in the directory `customer` (`C0…`, or
/// `my_customer` for the caller's own), and — with `assign` — assign it when not. Runs
/// with the operator's own base credential, never an impersonated one.
pub(crate) async fn groups_admin(customer: &str, sa_email: &str, assign: bool) -> GroupsAdmin {
    let quota_project = crate::org_policy::resolve_quota_project();
    let done = |r: Refusal| GroupsAdmin::NotDone(next_step(&r, sa_email, quota_project.as_deref()));
    let token = match crate::gcp::scoped_base_token(&["https://www.googleapis.com/auth/cloud-platform", SCOPE]).await {
        Ok(t) => t,
        Err(e) => return done(Refusal::Other(format!("no token: {}", e))),
    };
    let http = reqwest::Client::new();
    let uid = match unique_id(&http, &token, quota_project.as_deref(), sa_email).await {
        Ok(Some(uid)) => uid,
        Ok(None) => {
            return GroupsAdmin::NotDone(format!(
                "{} does not exist yet — the apply that creates it comes first, then assign Groups Admin: {}",
                sa_email, CONSOLE_PATH
            ))
        }
        Err(r) => return done(r),
    };
    let dir = Directory { http, token, quota_project: quota_project.clone(), customer: customer.to_string() };
    let role_id = match dir.role_id().await {
        Ok(id) => id,
        Err(r) => return done(r),
    };
    match dir.holds(&role_id, &uid).await {
        Ok(true) => GroupsAdmin::Held,
        Ok(false) if !assign => GroupsAdmin::NotDone(format!("{} does not hold Groups Admin — `satz migrate <estate> --mode cloud` assigns it, or {}", sa_email, CONSOLE_PATH)),
        Ok(false) => match dir.assign(&role_id, &uid).await {
            Ok(()) => GroupsAdmin::Assigned,
            Err(r) => done(r),
        },
        Err(r) => done(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_classified_by_its_reason() {
        let scope = r#"{"error":{"code":403,"message":"Request had insufficient authentication scopes.","status":"PERMISSION_DENIED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#;
        assert_eq!(classify(403, scope), Refusal::Scope);
        let disabled = r#"{"error":{"code":403,"message":"Admin SDK API has not been used in project 1 before or it is disabled.","errors":[{"reason":"accessNotConfigured"}],"details":[{"reason":"SERVICE_DISABLED"}]}}"#;
        assert_eq!(classify(403, disabled), Refusal::ServiceDisabled);
        let denied = r#"{"error":{"code":403,"message":"Not Authorized to access this resource/api","errors":[{"reason":"forbidden"}]}}"#;
        assert_eq!(classify(403, denied), Refusal::Denied("Not Authorized to access this resource/api".into()));
        assert!(matches!(classify(500, "boom"), Refusal::Other(_)));
    }

    #[test]
    fn the_scope_refusal_names_the_login_that_carries_it_and_the_console_path() {
        let text = next_step(&Refusal::Scope, "svc-iac-001@acme-infra-001.iam.gserviceaccount.com", Some("acme-infra-001"));
        assert!(text.contains("gcloud auth application-default login --scopes=https://www.googleapis.com/auth/cloud-platform,https://www.googleapis.com/auth/admin.directory.rolemanagement"), "{text}");
        assert!(text.contains("Assign service accounts: svc-iac-001@acme-infra-001.iam.gserviceaccount.com"), "{text}");
        let disabled = next_step(&Refusal::ServiceDisabled, "sa", Some("acme-infra-001"));
        assert!(disabled.contains("gcloud services enable admin.googleapis.com --project acme-infra-001"), "{disabled}");
    }

    #[test]
    fn the_role_and_the_assignment_are_found_across_pages() {
        let roles = serde_json::json!({"items": [
            {"roleId": "111", "roleName": "_SEED_ADMIN_ROLE"},
            {"roleId": "222", "roleName": "_GROUPS_ADMIN_ROLE"}
        ]});
        assert_eq!(role_id_in(&roles), (Some("222".into()), None));
        let first = serde_json::json!({"items": [{"roleId": "222", "assignedTo": "100"}], "nextPageToken": "p2"});
        assert_eq!(assigned_in(&first, "200"), (false, Some("p2".into())));
        let second = serde_json::json!({"items": [{"roleId": "222", "assignedTo": "200"}]});
        assert_eq!(assigned_in(&second, "200"), (true, None));
    }
}
