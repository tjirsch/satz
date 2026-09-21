//! Google Workspace admin roles — the one day-0 prerequisite IAM can neither grant nor
//! test: the Groups Admin role the IaC service account needs before it manages the
//! estate's Cloud Identity groups.
//!
//! The service account is asked first, as itself: a `groups.list` on the customer's
//! directory succeeds for an account that holds the role and is refused for one that
//! does not. Only when it does not is the operator's login asked the same way, and only
//! then does the Admin SDK Directory API run.
//!
//! Assigning it runs as the operator, never as the service account (which cannot give
//! itself an admin role), and it only works when the operator's login carries the
//! role-management scope. A user's Application Default Credentials carry the scopes
//! chosen at `gcloud auth application-default login`, whatever a program requests, so
//! the default login cannot make this call. Every failure is classified into the one
//! thing the operator does next.

const ADMIN: &str = "https://admin.googleapis.com/admin/directory/v1";
const IAM: &str = "https://iam.googleapis.com/v1";
const CLOUD_IDENTITY: &str = "https://cloudidentity.googleapis.com/v1";
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

/// Who holds Groups Admin, asked in the order that decides what happens next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Holder {
    /// the IaC service account: nothing to do, and nothing to say about the login
    ServiceAccount,
    /// the service account does not, the operator's login does: it may assign it
    Login,
    /// neither: a Workspace admin assigns it to the service account
    Neither,
}

/// A test that gave no answer, and whose it was — never read as "does not hold it".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unanswered {
    ServiceAccount(Refusal),
    Login(Refusal),
}

/// The one decision every Groups Admin check makes: the service account is asked first,
/// and the operator's login only when the service account gives a clear no. `login` runs
/// only then, so a service account that holds the role asks nothing of the login.
pub(crate) async fn who_holds<F, Fut>(service_account: Result<bool, Refusal>, login: F) -> Result<Holder, Unanswered>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<bool, Refusal>>,
{
    match service_account {
        Ok(true) => Ok(Holder::ServiceAccount),
        Err(r) => Err(Unanswered::ServiceAccount(r)),
        Ok(false) => match login().await {
            Ok(true) => Ok(Holder::Login),
            Ok(false) => Ok(Holder::Neither),
            Err(r) => Err(Unanswered::Login(r)),
        },
    }
}

/// ErrorInfo reasons of a 403 that say nothing about the caller's directory rights: the
/// quota project, the API, the token. Such a refusal is no answer, not a "no".
const NOT_ABOUT_THE_CALLER: &[&str] = &[
    "USER_PROJECT_DENIED",
    "SERVICE_DISABLED",
    "accessNotConfigured",
    "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
    "CONSUMER_INVALID",
    "BILLING_DISABLED",
];

/// What a `groups.list` on the customer's directory says about the caller: a success
/// holds the groups privilege, a plain permission denial does not, and everything else —
/// a disabled API, a quota project the caller may not use, a missing customer — is no
/// answer.
pub(crate) fn probe_answer(status: u16, body: &str) -> Result<bool, Refusal> {
    if (200..300).contains(&status) {
        return Ok(true);
    }
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    let unrelated = error["details"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["reason"].as_str())
        .chain(error["errors"].as_array().into_iter().flatten().filter_map(|e| e["reason"].as_str()))
        .find(|r| NOT_ABOUT_THE_CALLER.contains(r));
    match (status, unrelated) {
        (403, None) => Ok(false),
        (403, Some(reason)) => Err(match classify(status, body) {
            Refusal::Denied(message) => Refusal::Other(format!("{}: {}", reason, message)),
            other => other,
        }),
        _ => Err(match classify(status, body) {
            Refusal::Denied(message) => Refusal::Other(format!("HTTP {}: {}", status, message)),
            other => other,
        }),
    }
}

/// Whether the holder of `token` may read the customer's groups — what Groups Admin
/// gives, tested with the call the provider itself makes, as the identity itself.
async fn reads_groups(http: &reqwest::Client, token: &str, quota_project: Option<&str>, customer: &str) -> Result<bool, Refusal> {
    let parent = format!("customers/{}", customer);
    let mut rb = http
        .get(format!("{}/groups", CLOUD_IDENTITY))
        .bearer_auth(token)
        .query(&[("parent", parent.as_str()), ("view", "BASIC"), ("pageSize", "1")]);
    if let Some(qp) = quota_project {
        rb = rb.header("x-goog-user-project", qp);
    }
    let res = rb.send().await.map_err(|e| Refusal::Other(e.to_string()))?;
    let status = res.status().as_u16();
    probe_answer(status, &res.text().await.unwrap_or_default())
}

/// A refusal of the Cloud Identity probe, as the reason it gave no answer.
fn unanswered(refusal: &Refusal, quota_project: Option<&str>) -> String {
    match refusal {
        Refusal::ServiceDisabled => format!(
            "the Cloud Identity API is not enabled on the quota project{} — `gcloud services enable cloudidentity.googleapis.com{}`, then run this again",
            quota_project.map(|p| format!(" {}", p)).unwrap_or_default(),
            quota_project.map(|p| format!(" --project {}", p)).unwrap_or_default(),
        ),
        Refusal::Scope => "the token lacks the cloud-platform scope".to_string(),
        Refusal::Denied(why) | Refusal::Other(why) => why.clone(),
    }
}

/// Whether the IaC service account `sa_email` holds Groups Admin in the directory
/// `customer` (`C0…`), and — with `assign` — assign it when it does not. The service
/// account is asked first, as itself; only when it does not hold the role is the
/// operator's login asked, and only then does the Directory API run, as the operator
/// (the service account cannot give itself an admin role).
pub(crate) async fn groups_admin(customer: Option<&str>, sa_email: &str, assign: bool) -> GroupsAdmin {
    let Some(customer) = customer else {
        return GroupsAdmin::NotDone(
            "the estate sets no customer_id — the directory its groups live in, and the one the check reads".to_string(),
        );
    };
    let quota_project = crate::org_policy::resolve_quota_project();
    let qp = quota_project.as_deref();
    let http = reqwest::Client::new();
    let base = match crate::gcp::base_access_token().await {
        Ok(t) => t,
        Err(e) => return GroupsAdmin::NotDone(format!("no token: {}", e)),
    };
    let uid = match unique_id(&http, &base, qp, sa_email).await {
        Ok(Some(uid)) => uid,
        Ok(None) => {
            return GroupsAdmin::NotDone(format!(
                "{} does not exist yet — the apply that creates it comes first, then assign Groups Admin: {}",
                sa_email, CONSOLE_PATH
            ))
        }
        Err(r) => return GroupsAdmin::NotDone(format!("looking up {}: {}", sa_email, unanswered(&r, qp))),
    };
    let service_account = match crate::gcp::token_as(sa_email).await {
        Ok(token) => reads_groups(&http, &token, qp, customer).await,
        Err(e) => Err(Refusal::Other(format!("acting as {}: {}", sa_email, e))),
    };
    let holder = who_holds(service_account, || reads_groups(&http, &base, qp, customer)).await;
    match holder {
        Ok(Holder::ServiceAccount) => GroupsAdmin::Held,
        Err(Unanswered::ServiceAccount(r)) => {
            GroupsAdmin::NotDone(format!("could not test whether {} holds Groups Admin: {}", sa_email, unanswered(&r, qp)))
        }
        Err(Unanswered::Login(r)) => GroupsAdmin::NotDone(format!(
            "{} does not hold Groups Admin, and testing whether your login does failed: {}",
            sa_email,
            unanswered(&r, qp)
        )),
        Ok(Holder::Neither) => GroupsAdmin::NotDone(format!(
            "neither {} nor your login holds Groups Admin — a Workspace super admin assigns it: {}: {}",
            sa_email, CONSOLE_PATH, sa_email
        )),
        Ok(Holder::Login) if !assign => GroupsAdmin::NotDone(format!(
            "{} does not hold Groups Admin — your login does, and `satz migrate <estate> --mode cloud` assigns it when your login may assign admin roles, or {}: {}",
            sa_email, CONSOLE_PATH, sa_email
        )),
        Ok(Holder::Login) => {
            let done = |r: Refusal| GroupsAdmin::NotDone(next_step(&r, sa_email, qp));
            let token = match crate::gcp::scoped_base_token(&["https://www.googleapis.com/auth/cloud-platform", SCOPE]).await {
                Ok(t) => t,
                Err(e) => return done(Refusal::Other(format!("no token: {}", e))),
            };
            let dir = Directory { http, token, quota_project: quota_project.clone(), customer: customer.to_string() };
            match dir.role_id().await {
                Ok(role_id) => match dir.assign(&role_id, &uid).await {
                    Ok(()) => GroupsAdmin::Assigned,
                    Err(r) => done(r),
                },
                Err(r) => done(r),
            }
        }
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
    fn the_groups_admin_role_is_found_by_its_system_name() {
        let roles = serde_json::json!({"items": [
            {"roleId": "111", "roleName": "_SEED_ADMIN_ROLE"},
            {"roleId": "222", "roleName": "_GROUPS_ADMIN_ROLE"}
        ]});
        assert_eq!(role_id_in(&roles), (Some("222".into()), None));
    }

    /// The login is asked only after the service account gave a clear no — this is what
    /// the counter proves: a login that is never asked can ask for nothing.
    #[tokio::test]
    async fn the_service_account_is_asked_first_and_the_login_only_when_it_lacks_the_role() {
        let asked = std::cell::Cell::new(0);
        let login = |answer: Result<bool, Refusal>| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                std::future::ready(answer)
            }
        };
        assert_eq!(who_holds(Ok(true), login(Ok(false))).await, Ok(Holder::ServiceAccount));
        assert_eq!(asked.get(), 0, "a service account that holds it asks nothing of the login");
        assert_eq!(who_holds(Ok(false), login(Ok(true))).await, Ok(Holder::Login));
        assert_eq!(who_holds(Ok(false), login(Ok(false))).await, Ok(Holder::Neither));
        assert_eq!(asked.get(), 2);
    }

    #[tokio::test]
    async fn a_test_that_gave_no_answer_surfaces_and_is_never_read_as_lacking_the_role() {
        let asked = std::cell::Cell::new(false);
        let failed = Refusal::Other("HTTP 500: backend".into());
        let held = who_holds(Err(failed.clone()), || {
            asked.set(true);
            std::future::ready(Ok(true))
        })
        .await;
        assert_eq!(held, Err(Unanswered::ServiceAccount(failed.clone())));
        assert!(!asked.get(), "a failed service-account test does not move on to the login");
        let login = who_holds(Ok(false), || std::future::ready(Err(failed.clone()))).await;
        assert_eq!(login, Err(Unanswered::Login(failed)));
    }

    #[test]
    fn only_a_plain_permission_denial_reads_as_lacking_the_role() {
        assert_eq!(probe_answer(200, r#"{"groups":[]}"#), Ok(true));
        let denied = r#"{"error":{"code":403,"message":"Error(2028): Permission denied for resource customers/C0example (or it may not exist).","status":"PERMISSION_DENIED"}}"#;
        assert_eq!(probe_answer(403, denied), Ok(false));
        let disabled = r#"{"error":{"code":403,"message":"Cloud Identity API has not been used in project 1 before or it is disabled.","status":"PERMISSION_DENIED","details":[{"reason":"SERVICE_DISABLED"}]}}"#;
        assert_eq!(probe_answer(403, disabled), Err(Refusal::ServiceDisabled));
        let quota = r#"{"error":{"code":403,"message":"Caller does not have required permission to use project acme-infra-001.","status":"PERMISSION_DENIED","details":[{"reason":"USER_PROJECT_DENIED"}]}}"#;
        assert!(matches!(probe_answer(403, quota), Err(Refusal::Other(m)) if m.starts_with("USER_PROJECT_DENIED")));
        assert!(matches!(probe_answer(401, r#"{"error":{"code":401,"message":"expired"}}"#), Err(Refusal::Other(_))));
        assert!(matches!(probe_answer(404, "not found"), Err(Refusal::Other(_))));
    }
}
