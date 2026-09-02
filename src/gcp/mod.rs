//! Thin clients for the Google Cloud APIs satz talks to directly (REST over
//! `reqwest`, ADC bearer token). One module per API; the pure parts — page
//! merging, matching, error classification — are separate functions so they
//! can be tested without a network.

pub(crate) mod billing;
pub(crate) mod discovery_doc;
pub(crate) mod resourcemanager;
pub(crate) mod serviceusage;
pub(crate) mod storage;

/// An ADC bearer token for the cloud-platform scope.
pub(crate) async fn access_token() -> Result<String, String> {
    use google_cloud_auth::credentials::Builder;
    let credentials = Builder::default()
        .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
        .build_access_token_credentials()
        .map_err(|e| e.to_string())?;
    Ok(credentials.access_token().await.map_err(|e| e.to_string())?.token)
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
