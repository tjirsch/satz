//! Thin clients for the Google Cloud APIs satz talks to directly (REST over
//! `reqwest`, ADC bearer token). One module per API; the pure parts — page
//! merging, matching, error classification — are separate functions so they
//! can be tested without a network.

pub(crate) mod billing;
pub(crate) mod discovery_doc;
pub(crate) mod identity;
pub(crate) mod resourcemanager;
pub(crate) mod serviceusage;
pub(crate) mod storage;

/// What the process has been bound to.
///
/// `Disabled` is `--no-impersonate`: a deliberate decision that outranks any
/// estate and is never a conflict. `Bound` is the estate's answer — `Some(sa)`
/// for a `deployment_mode = "cloud"` estate, `None` for a local one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identity {
    Disabled,
    Bound(Option<String>),
}

/// The service account every live call runs as, bound at dispatch for estate
/// commands (deployment_mode "cloud" → the estate's IaC SA, exactly the
/// identity tofu applies with) and consulted by the token chokepoint and the
/// Cloud Asset client.
///
/// One process, one identity. That is free for the CLI — one command, one
/// estate — but `satz mcp` is long-lived and its tools each name an estate, so
/// a second, DIFFERENT binding is refused rather than ignored. It used to be
/// ignored, which meant a tool call on estate B silently ran as estate A's
/// service account: deterministic, silent, and cross-customer.
static IMPERSONATE: std::sync::OnceLock<Identity> = std::sync::OnceLock::new();

/// Bind the identity, or confirm it is already what the caller wants.
///
/// Rebinding to the same target is fine (a command builds several clients).
/// Rebinding to a different one is an error naming both, because there is no
/// answer that is right for both callers.
pub(crate) fn configure_impersonation(sa: Option<String>) -> Result<(), String> {
    describe_conflict(IMPERSONATE.get_or_init(|| Identity::Bound(sa.clone())), &sa)
}

/// `--no-impersonate`: pin the process to the plain ADC. It wins over any
/// later estate binding instead of colliding with it.
pub(crate) fn disable_impersonation() {
    let _ = IMPERSONATE.set(Identity::Disabled);
}

/// Pure so the four cases are testable without touching the global.
fn describe_conflict(current: &Identity, wanted: &Option<String>) -> Result<(), String> {
    match current {
        // The operator asked for the plain ADC; an estate does not override that.
        Identity::Disabled => Ok(()),
        Identity::Bound(bound) if bound == wanted => Ok(()),
        Identity::Bound(bound) => Err(format!(
            "this process is already acting as {}, and this estate needs {} — one process \
             serves one identity. Run the command again for the other estate, or restart \
             `satz mcp` pointed at it.",
            bound.as_deref().unwrap_or("the plain ADC (a local-mode estate)"),
            wanted.as_deref().unwrap_or("the plain ADC (a local-mode estate)"),
        )),
    }
}

pub(crate) fn impersonation_target() -> Option<String> {
    match IMPERSONATE.get() {
        Some(Identity::Bound(sa)) => sa.clone(),
        Some(Identity::Disabled) | None => None,
    }
}

/// An ADC bearer token for the cloud-platform scope — minted AS the
/// configured impersonation target when one is set. The chokepoint every
/// live command mints through — which is what lets [`identity::announce`]
/// print the credential line exactly once, before the first API call, without
/// any command knowing about it.
pub(crate) async fn access_token() -> Result<String, String> {
    use google_cloud_auth::credentials::Builder;
    let credentials = Builder::default()
        .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
        .build_access_token_credentials()
        .map_err(|e| e.to_string())?;
    let base = credentials.access_token().await.map_err(|e| e.to_string())?.token;
    let token = match impersonation_target() {
        None => base,
        Some(sa) => impersonated_token(&base, &sa).await?,
    };
    identity::announce(&token).await;
    Ok(token)
}

/// Exchange the caller's token for one minted AS the service account —
/// iamcredentials `generateAccessToken`, the same call an impersonating ADC
/// makes. A denial names the missing TokenCreator grant and the opt-out.
async fn impersonated_token(base: &str, sa: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let res = client
        .post(format!(
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateAccessToken",
            sa
        ))
        .bearer_auth(base)
        .json(&serde_json::json!({ "scope": ["https://www.googleapis.com/auth/cloud-platform"] }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let e = api_error(res).await;
        if e.class() == ErrorClass::PermissionDenied {
            return Err(format!(
                "cannot impersonate {} ({}) — the caller needs roles/iam.serviceAccountTokenCreator \
                 on it (normally via membership in the svc-iac-users group), or pass --no-impersonate",
                sa, e
            ));
        }
        return Err(format!("cannot impersonate {}: {}", sa, e));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    v.get("accessToken")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "generateAccessToken returned no accessToken".to_string())
}

/// A Cloud Asset client honoring the configured impersonation: with a target
/// set, the credentials are shaped exactly like an impersonating ADC file
/// (the on-disk ADC as `source_credentials`); otherwise the default ADC.
pub(crate) async fn asset_service() -> Result<google_cloud_asset_v1::client::AssetService, String> {
    let builder = google_cloud_asset_v1::client::AssetService::builder();
    match impersonation_target() {
        None => builder.build().await.map_err(|e| e.to_string()),
        Some(sa) => {
            let creds = google_cloud_auth::credentials::impersonated::Builder::new(
                impersonated_credential_json(&sa)?,
            )
            .build()
            .map_err(|e| e.to_string())?;
            builder.with_credentials(creds).build().await.map_err(|e| e.to_string())
        }
    }
}

/// The impersonating-ADC JSON, composed in memory: what `gcloud auth
/// application-default login --impersonate-service-account` would write, with
/// the existing ADC as the source. An ADC that already impersonates is kept
/// as-is — re-wrapping would chain impersonations.
fn impersonated_credential_json(sa: &str) -> Result<serde_json::Value, String> {
    let path = crate::org_policy::adc_file_path().ok_or(
        "no ADC file to impersonate from — run `gcloud auth application-default login`, or pass --no-impersonate",
    )?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let source: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
    if source.get("type").and_then(|t| t.as_str()) == Some("impersonated_service_account") {
        return Ok(source);
    }
    Ok(serde_json::json!({
        "type": "impersonated_service_account",
        "service_account_impersonation_url": format!(
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateAccessToken",
            sa
        ),
        "source_credentials": source,
    }))
}

/// What a failed API call means for the caller. Derived from the HTTP status
/// and the error body's own machine-readable fields — never from substring
/// matching, which is what once made a missing quota project read as a
/// missing permission (both arrive as 403).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorClass {
    /// The identity genuinely lacks a permission.
    PermissionDenied,
    /// The request was billed to no (or a broken) quota project — the API is
    /// fine, the `x-goog-user-project` side is not.
    QuotaProject,
    /// 409: the resource already exists (or a concurrent change collided).
    Conflict,
    /// Everything else, including transport errors.
    Other,
}

/// A failed API call: the HTTP status plus what the error body says about
/// itself (`error.status`, the ErrorInfo `reason`), kept structured so
/// callers can classify without re-parsing strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiError {
    /// HTTP status; 0 means the request never got an HTTP response.
    pub(crate) status: u16,
    /// `error.status`, e.g. `PERMISSION_DENIED`.
    pub(crate) grpc_status: Option<String>,
    /// The ErrorInfo `reason`, e.g. `SERVICE_DISABLED`, `USER_PROJECT_DENIED`.
    pub(crate) reason: Option<String>,
    /// The response body (or transport error message), verbatim.
    pub(crate) body: String,
}

impl ApiError {
    /// A request that failed before an HTTP response existed (connect error,
    /// unreadable body, poll timeout). Classified `Other`.
    pub(crate) fn transport(e: impl std::fmt::Display) -> Self {
        ApiError { status: 0, grpc_status: None, reason: None, body: e.to_string() }
    }

    pub(crate) fn class(&self) -> ErrorClass {
        classify(self.status, self.reason.as_deref())
    }

    /// A long-running operation's terminal `error` object (`google.rpc.Status`:
    /// numeric `code`, `message`, `details`). The two codes bootstrap can act
    /// on map onto their HTTP equivalents; everything else stays `Other`.
    pub(crate) fn from_operation_error(err: &serde_json::Value) -> Self {
        let code = err.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
        let (status, grpc_status) = match code {
            6 => (409, Some("ALREADY_EXISTS".to_string())),
            7 => (403, Some("PERMISSION_DENIED".to_string())),
            _ => (0, None),
        };
        let reason = err
            .get("details")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.iter().find_map(error_info_reason));
        ApiError { status, grpc_status, reason, body: err.to_string() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let body = if self.body.trim().is_empty() { "(empty body)" } else { self.body.as_str() };
        if self.status == 0 {
            return write!(f, "{}", body);
        }
        let canonical = reqwest::StatusCode::from_u16(self.status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("");
        match self.reason.as_deref().or(self.grpc_status.as_deref()) {
            Some(tag) => write!(f, "{} {} [{}]: {}", self.status, canonical, tag, body),
            None => write!(f, "{} {}: {}", self.status, canonical, body),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<ApiError> for String {
    fn from(e: ApiError) -> String {
        e.to_string()
    }
}

/// Read a failed response into an [`ApiError`]. Consumes the body.
pub(crate) async fn api_error(res: reqwest::Response) -> ApiError {
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_else(|e| format!("(body unreadable: {})", e));
    let (grpc_status, reason) = parse_error_body(&body);
    ApiError { status, grpc_status, reason, body }
}

/// `error.status` and the first machine-readable `reason` out of a Google
/// error body — the ErrorInfo detail on current APIs, `error.errors[0].reason`
/// on the legacy shape (GCS). Unparseable bodies yield nothing; they never
/// fail.
pub(crate) fn parse_error_body(body: &str) -> (Option<String>, Option<String>) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let Some(err) = v.get("error") else { return (None, None) };
    let grpc = err.get("status").and_then(|s| s.as_str()).map(str::to_string);
    let reason = err
        .get("details")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.iter().find_map(error_info_reason))
        .or_else(|| {
            err.get("errors")
                .and_then(|e| e.as_array())
                .and_then(|a| a.first())
                .and_then(|e| e.get("reason"))
                .and_then(|r| r.as_str())
                .map(str::to_string)
        });
    (grpc, reason)
}

/// The `reason` of one `google.rpc.ErrorInfo` detail, if that is what it is.
fn error_info_reason(detail: &serde_json::Value) -> Option<String> {
    if detail.get("@type").and_then(|t| t.as_str())
        != Some("type.googleapis.com/google.rpc.ErrorInfo")
    {
        return None;
    }
    detail.get("reason").and_then(|r| r.as_str()).map(str::to_string)
}

/// Classify a failure. Pure, so the table is pinned by tests.
pub(crate) fn classify(status: u16, reason: Option<&str>) -> ErrorClass {
    match status {
        409 => ErrorClass::Conflict,
        403 => match reason {
            // The quota-project family: the request was billed to a project
            // that does not exist, is not set, or has the API off. Legacy APIs
            // spell it accessNotConfigured.
            Some("SERVICE_DISABLED" | "USER_PROJECT_DENIED" | "CONSUMER_INVALID" | "accessNotConfigured") => {
                ErrorClass::QuotaProject
            }
            _ => ErrorClass::PermissionDenied,
        },
        _ => ErrorClass::Other,
    }
}

/// POST a `testIamPermissions` request to `endpoint` and return the granted
/// subset. An absent `permissions` field in the response means none of the
/// asked-for permissions are granted.
pub(crate) async fn test_iam_permissions(
    client: &reqwest::Client,
    token: &str,
    endpoint: &str,
    permissions: &[&str],
) -> Result<Vec<String>, ApiError> {
    let res = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({ "permissions": permissions }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(api_error(res).await);
    }
    let v: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    Ok(v.get("permissions")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default())
}

/// Add `member` to `role`'s UNCONDITIONAL binding in an IAM policy, creating
/// the binding when absent. Bindings that carry a `condition` are never
/// touched — appending a member there would silently subject them to the
/// condition. `etag` and `version` pass through untouched, so a
/// read-modify-write keeps its optimistic-concurrency guard. Returns `false`
/// when the member is already bound (nothing to write).
pub(crate) fn add_binding(policy: &mut serde_json::Value, role: &str, member: &str) -> bool {
    let obj = policy.as_object_mut().expect("an IAM policy is a JSON object");
    let bindings = obj
        .entry("bindings")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .expect("policy bindings are a JSON array");
    for b in bindings.iter_mut() {
        if b.get("role").and_then(|r| r.as_str()) != Some(role) || b.get("condition").is_some() {
            continue;
        }
        let members = b
            .as_object_mut()
            .expect("a binding is a JSON object")
            .entry("members")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .expect("binding members are a JSON array");
        if members.iter().any(|m| m.as_str() == Some(member)) {
            return false;
        }
        members.push(serde_json::Value::String(member.to_string()));
        return true;
    }
    bindings.push(serde_json::json!({ "role": role, "members": [member] }));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- which identity the process is bound to ------------------------------

    fn sa(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    /// Binding twice to the same service account is ordinary: one command builds
    /// several clients, and each asks.
    #[test]
    fn rebinding_the_same_identity_is_not_a_conflict() {
        let bound = Identity::Bound(sa("svc-iac@acme-infra-001.iam.gserviceaccount.com"));
        assert!(
            describe_conflict(&bound, &sa("svc-iac@acme-infra-001.iam.gserviceaccount.com")).is_ok()
        );
        assert!(describe_conflict(&Identity::Bound(None), &None).is_ok());
    }

    /// The defect this replaced: the second binding was silently DROPPED, so a
    /// tool call on estate B ran as estate A's service account. Deterministic,
    /// invisible, and across two customers.
    #[test]
    fn a_second_different_identity_is_refused_and_names_both() {
        let bound = Identity::Bound(sa("svc-iac@acme-infra-001.iam.gserviceaccount.com"));
        let err = describe_conflict(&bound, &sa("svc-iac@globex-infra-001.iam.gserviceaccount.com"))
            .expect_err("a different service account must not be silently ignored");
        assert!(err.contains("acme-infra-001"), "the error hides who we already are: {}", err);
        assert!(err.contains("globex-infra-001"), "the error hides who was asked for: {}", err);
    }

    /// A local-mode estate wants the plain ADC; that is still an identity, and
    /// mixing it with a cloud-mode estate in one process is still a conflict.
    #[test]
    fn plain_adc_and_a_service_account_conflict_in_both_directions() {
        let none = Identity::Bound(None);
        let some = Identity::Bound(sa("svc-iac@acme-infra-001.iam.gserviceaccount.com"));
        assert!(describe_conflict(&none, &sa("svc-iac@acme-infra-001.iam.gserviceaccount.com")).is_err());
        assert!(describe_conflict(&some, &None).is_err());
    }

    /// `--no-impersonate` is the operator overruling the estate. It must not then
    /// collide with the estate it overruled.
    #[test]
    fn no_impersonate_outranks_every_estate_instead_of_conflicting() {
        assert!(describe_conflict(&Identity::Disabled, &None).is_ok());
        assert!(
            describe_conflict(
                &Identity::Disabled,
                &sa("svc-iac@acme-infra-001.iam.gserviceaccount.com")
            )
            .is_ok(),
            "--no-impersonate must stay a decision, not become a conflict"
        );
    }

    #[test]
    fn classification_separates_quota_from_permission() {
        assert_eq!(classify(403, Some("SERVICE_DISABLED")), ErrorClass::QuotaProject);
        assert_eq!(classify(403, Some("USER_PROJECT_DENIED")), ErrorClass::QuotaProject);
        assert_eq!(classify(403, Some("accessNotConfigured")), ErrorClass::QuotaProject);
        assert_eq!(classify(403, Some("IAM_PERMISSION_DENIED")), ErrorClass::PermissionDenied);
        assert_eq!(classify(403, None), ErrorClass::PermissionDenied);
        assert_eq!(classify(409, None), ErrorClass::Conflict);
        assert_eq!(classify(500, None), ErrorClass::Other);
        assert_eq!(classify(0, None), ErrorClass::Other);
    }

    #[test]
    fn error_body_yields_status_and_error_info_reason() {
        let body = r#"{"error":{"code":403,"message":"Cloud Resource Manager API has not been used in project 424242 before or it is disabled.","status":"PERMISSION_DENIED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SERVICE_DISABLED","domain":"googleapis.com","metadata":{"service":"cloudresourcemanager.googleapis.com"}}]}}"#;
        let (grpc, reason) = parse_error_body(body);
        assert_eq!(grpc.as_deref(), Some("PERMISSION_DENIED"));
        assert_eq!(reason.as_deref(), Some("SERVICE_DISABLED"));
        // The whole point: this SERVICE_DISABLED 403 is a quota problem, not a denial.
        assert_eq!(classify(403, reason.as_deref()), ErrorClass::QuotaProject);
    }

    #[test]
    fn legacy_error_shape_yields_its_reason() {
        let body = r#"{"error":{"errors":[{"domain":"usageLimits","reason":"accessNotConfigured","message":"Access Not Configured."}],"code":403,"message":"Access Not Configured."}}"#;
        let (grpc, reason) = parse_error_body(body);
        assert_eq!(grpc, None);
        assert_eq!(reason.as_deref(), Some("accessNotConfigured"));
    }

    #[test]
    fn unparseable_bodies_yield_nothing() {
        assert_eq!(parse_error_body("<html>gateway timeout</html>"), (None, None));
        assert_eq!(parse_error_body(""), (None, None));
        assert_eq!(parse_error_body(r#"{"message":"no error wrapper"}"#), (None, None));
    }

    #[test]
    fn display_carries_status_reason_and_body() {
        let e = ApiError {
            status: 403,
            grpc_status: Some("PERMISSION_DENIED".into()),
            reason: Some("SERVICE_DISABLED".into()),
            body: "{...}".into(),
        };
        assert_eq!(e.to_string(), "403 Forbidden [SERVICE_DISABLED]: {...}");
        let plain = ApiError { status: 404, grpc_status: None, reason: None, body: "".into() };
        assert_eq!(plain.to_string(), "404 Not Found: (empty body)");
        let transport = ApiError::transport("connection refused");
        assert_eq!(transport.to_string(), "connection refused");
        assert_eq!(transport.class(), ErrorClass::Other);
    }

    #[test]
    fn operation_errors_map_grpc_codes() {
        let denied = ApiError::from_operation_error(&serde_json::json!({
            "code": 7, "message": "The caller does not have permission"
        }));
        assert_eq!(denied.class(), ErrorClass::PermissionDenied);
        let exists = ApiError::from_operation_error(&serde_json::json!({
            "code": 6, "message": "Requested entity already exists"
        }));
        assert_eq!(exists.class(), ErrorClass::Conflict);
        let odd = ApiError::from_operation_error(&serde_json::json!({
            "code": 13, "message": "internal"
        }));
        assert_eq!(odd.class(), ErrorClass::Other);
        assert!(odd.to_string().contains("internal"));
    }

    // --- IAM policy read-modify-write ---------------------------------------

    #[test]
    fn add_binding_appends_dedups_and_creates() {
        let mut policy = serde_json::json!({
            "version": 1,
            "etag": "BwX1234=",
            "bindings": [
                { "role": "roles/viewer", "members": ["user:a@example.com"] }
            ]
        });
        assert!(add_binding(&mut policy, "roles/viewer", "user:b@example.com"));
        assert!(!add_binding(&mut policy, "roles/viewer", "user:b@example.com"), "dedup");
        assert!(add_binding(&mut policy, "roles/editor", "user:b@example.com"), "new binding");
        assert_eq!(policy["bindings"][0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(policy["bindings"][1]["role"], "roles/editor");
        // The optimistic-concurrency guard survives the modification.
        assert_eq!(policy["etag"], "BwX1234=");
        assert_eq!(policy["version"], 1);
    }

    #[test]
    fn add_binding_never_touches_conditional_bindings() {
        let mut policy = serde_json::json!({
            "etag": "BwX1234=",
            "bindings": [
                {
                    "role": "roles/viewer",
                    "members": ["user:a@example.com"],
                    "condition": { "title": "expires", "expression": "request.time < timestamp('2027-01-01T00:00:00Z')" }
                }
            ]
        });
        // Same role, but the existing binding is conditional: a NEW
        // unconditional binding is created instead of appending there.
        assert!(add_binding(&mut policy, "roles/viewer", "user:b@example.com"));
        let bindings = policy["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0]["members"].as_array().unwrap().len(), 1, "conditional binding untouched");
        assert!(bindings[1].get("condition").is_none());
    }

    #[test]
    fn add_binding_handles_a_policy_without_bindings() {
        let mut policy = serde_json::json!({ "etag": "BwX1234=" });
        assert!(add_binding(&mut policy, "roles/viewer", "user:a@example.com"));
        assert_eq!(policy["bindings"][0]["members"][0], "user:a@example.com");
    }
}
