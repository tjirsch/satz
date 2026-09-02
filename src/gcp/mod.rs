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
}
