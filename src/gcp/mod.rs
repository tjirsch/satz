//! Thin clients for the Google Cloud APIs satz talks to directly (REST over
//! `reqwest`, ADC bearer token). One module per API; the pure parts — page
//! merging, matching, error classification — are separate functions so they
//! can be tested without a network.

pub(crate) mod billing;
pub(crate) mod discovery_doc;
pub(crate) mod iam_policy;
pub(crate) mod identity;
pub(crate) mod resourcemanager;
pub(crate) mod serviceusage;
pub(crate) mod storage;
pub(crate) mod workspace;

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
/// One process, one identity — for the CLI, where it is free: one command, one
/// estate. `satz mcp` is long-lived and works through estates in turn, so it
/// does NOT use this. It scopes the identity to each call instead
/// ([`with_identity`]), which is the same rule stated per call rather than per
/// process.
static IMPERSONATE: std::sync::OnceLock<Identity> = std::sync::OnceLock::new();

tokio::task_local! {
    /// The identity for the CALL in progress, when something scoped one.
    ///
    /// A task-local rather than a second global because the guarantee needed is
    /// per call, not per process: whatever else the server is doing, the work
    /// inside this scope runs as this service account for its whole duration.
    /// `LocalKey::scope` binds the value to the FUTURE, so it holds across every
    /// await inside it and cannot leak into a call running beside it — and
    /// nothing in this tree spawns a task, so nothing escapes the scope.
    static CALL_IDENTITY: Option<String>;
}

/// Run `f` as `sa`. Everything the future touches — every `access_token`, every
/// client built inside it — mints as that service account, and a call scoped to
/// a different one at the same time is unaffected.
///
/// This is what lets one server serve many estates without the process ever
/// having a single answer to "who am I": the question is only ever asked inside
/// a call, and a call always knows.
pub(crate) async fn with_identity<T>(sa: Option<String>, f: impl std::future::Future<Output = T>) -> T {
    CALL_IDENTITY.scope(sa, f).await
}

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

/// Whether `--no-impersonate` pinned the process to the plain ADC — the one reason
/// a cloud-mode estate's calls run as the credentials themselves.
pub(crate) fn impersonation_disabled() -> bool {
    matches!(IMPERSONATE.get(), Some(Identity::Disabled))
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

/// Who the calls in flight run as: the scope's answer if this code is inside
/// one, the process binding otherwise.
///
/// `--no-impersonate` is checked FIRST and outranks both. The operator asked for
/// the plain ADC, and a per-call scope is no more entitled to override that than
/// an estate binding was.
pub(crate) fn impersonation_target() -> Option<String> {
    if impersonation_disabled() {
        return None;
    }
    if let Ok(scoped) = CALL_IDENTITY.try_with(Clone::clone) {
        return scoped;
    }
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
    let base = base_access_token().await?;
    ensure_quota_project(&base).await?;
    let token = match plan_impersonation(adc_impersonation_target().as_deref(), impersonation_target().as_deref())? {
        Exchange::UseBase => base,
        Exchange::Mint(sa) => cached_impersonated_token(&base, &sa).await?,
    };
    identity::announce(&token).await;
    Ok(token)
}

/// The credential's OWN token, before any estate exchange.
///
/// `whoami` needs it to answer both halves: who you logged in as is a question
/// about the base credential, and the impersonation check has to be made AS that
/// credential — asking the estate's account whether it may impersonate itself
/// answers a different question.
///
/// One credentials object per process: google-cloud-auth caches the token inside
/// it and refreshes it before it expires, so a command that builds five clients
/// mints once instead of five times. A failed request drops the object: a failed
/// refresh latches inside it, and the next call has to read the ADC file again —
/// which is what a `gcloud auth application-default login` during a long-lived
/// `satz mcp` replaced.
pub(crate) async fn base_access_token() -> Result<String, String> {
    match base_credentials()?.access_token().await {
        Ok(t) => Ok(t.token),
        Err(e) => {
            *BASE_CREDENTIALS.lock().map_err(|_| "the credentials cache lock is poisoned".to_string())? = None;
            Err(e.to_string())
        }
    }
}

/// A token of the credential's OWN identity requesting `scopes` — a separate credential,
/// so the one every command mints through keeps its cloud-platform scope. A user's ADC
/// carries the scopes chosen at login whatever is requested here; a service account or
/// an external account gets exactly these. For the one call satz makes outside
/// cloud-platform: assigning a Workspace admin role.
pub(crate) async fn scoped_base_token(scopes: &[&str]) -> Result<String, String> {
    google_cloud_auth::credentials::Builder::default()
        .with_scopes(scopes.iter().map(|s| s.to_string()))
        .build_access_token_credentials()
        .map_err(|e| e.to_string())?
        .access_token()
        .await
        .map(|t| t.token)
        .map_err(|e| e.to_string())
}

static BASE_CREDENTIALS: std::sync::Mutex<Option<google_cloud_auth::credentials::AccessTokenCredentials>> =
    std::sync::Mutex::new(None);

fn base_credentials() -> Result<google_cloud_auth::credentials::AccessTokenCredentials, String> {
    let mut cached = BASE_CREDENTIALS.lock().map_err(|_| "the credentials cache lock is poisoned".to_string())?;
    if let Some(c) = cached.as_ref() {
        return Ok(c.clone());
    }
    let c = google_cloud_auth::credentials::Builder::default()
        .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
        .build_access_token_credentials()
        .map_err(|e| e.to_string())?;
    *cached = Some(c.clone());
    Ok(c)
}

/// Impersonated tokens by service account, with the Unix second they expire.
/// A token is reused until five minutes before its `expireTime`, so a report
/// that builds five clients mints once per account instead of five times.
static IMPERSONATED: std::sync::Mutex<Option<std::collections::HashMap<String, (String, u64)>>> =
    std::sync::Mutex::new(None);

/// Seconds before `expireTime` a cached token is replaced.
const TOKEN_MARGIN_SECS: u64 = 300;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn cached_impersonated_token(base: &str, sa: &str) -> Result<String, String> {
    let now = unix_now();
    let cached = IMPERSONATED
        .lock()
        .map_err(|_| "the impersonation cache lock is poisoned".to_string())?
        .as_ref()
        .and_then(|m| m.get(sa).cloned());
    if let Some((token, expires)) = cached {
        if now + TOKEN_MARGIN_SECS < expires {
            return Ok(token);
        }
    }
    let (token, expires) = mint_impersonated(base, sa).await?;
    IMPERSONATED
        .lock()
        .map_err(|_| "the impersonation cache lock is poisoned".to_string())?
        .get_or_insert_with(Default::default)
        .insert(sa.to_string(), (token.clone(), expires));
    Ok(token)
}

/// `expireTime` of a `generateAccessToken` reply — RFC 3339 in UTC,
/// `2026-09-11T13:45:07Z` or with fractional seconds — as Unix seconds.
pub(crate) fn parse_expire_time(t: &str) -> Option<u64> {
    let t = t.strip_suffix('Z')?;
    let (date, time) = t.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|x| x.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let time = time.split('.').next()?;
    let mut h = time.splitn(3, ':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (h.next()??, h.next()??, h.next()??);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days from 1970-01-01 to y-mo-da (Howard Hinnant's days_from_civil)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3_600 + mm * 60 + ss).ok()
}

/// A quota project nothing can reach fails EVERY api call, later and less
/// clearly — `UserProjectInvalid`, or "cannot create the authentication
/// headers", naming neither the project nor the fix. Checked once, here, where
/// every live command mints, so the failure arrives before the work instead of
/// during it.
///
/// Verified once per process: on success it is never re-checked, and on failure
/// every attempt fails, which is what a wrong quota project deserves.
static QUOTA_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

async fn ensure_quota_project(base: &str) -> Result<(), String> {
    if QUOTA_OK.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let Some(id) = crate::org_policy::resolve_quota_project() else {
        QUOTA_OK.store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(());
    };
    // The estate's own infra project is the right answer in every estate, so
    // name it rather than telling the operator to think of one.
    let bound = impersonation_target();
    let suggest = bound.as_deref().and_then(identity::project_of);
    identity::check_quota_project(base, &id, suggest).await?;
    QUOTA_OK.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Whether this credential may act as `sa`: one `generateAccessToken`, the token
/// discarded.
///
/// It is the very call every live command makes after init, so a failure here is
/// exactly the failure that command would hit — reported once, by name, instead
/// of as the first denied API call somewhere downstream.
pub(crate) async fn may_impersonate(base: &str, sa: &str) -> Result<(), String> {
    // Already that account: it does not need to impersonate itself, and asking
    // would report a 403 that means nothing.
    if adc_impersonation_target().as_deref() == Some(sa) {
        return Ok(());
    }
    impersonated_token(base, sa).await.map(|_| ())
}

/// What the token path has to do to end up acting as the estate's account.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Exchange {
    /// The credential is already the right identity.
    UseBase,
    /// Exchange it for this service account's token.
    Mint(String),
}

/// The service account the ADC FILE itself impersonates, if it is one of those.
/// `gcloud auth application-default login --impersonate-service-account` writes
/// exactly this shape.
fn adc_impersonation_target() -> Option<String> {
    let path = crate::org_policy::adc_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if json.get("type").and_then(|t| t.as_str()) != Some("impersonated_service_account") {
        return None;
    }
    identity::email_from_adc_json(&json)
}

/// Reconcile the identity the CREDENTIAL already carries with the one the
/// ESTATE asks for. Pure, so the four cases are testable without a network.
///
/// The two token paths used to answer this differently. REST always exchanged,
/// so an ADC that already impersonated the estate's account asked
/// `generateAccessToken` to mint that account a token for ITSELF — a 403 unless
/// it holds TokenCreator on itself. gRPC returned such an ADC unwrapped but
/// ignored the requested account entirely, so a credential impersonating one
/// service account silently served an estate that declared another.
pub(crate) fn plan_impersonation(
    adc_target: Option<&str>,
    estate_target: Option<&str>,
) -> Result<Exchange, String> {
    match (adc_target, estate_target) {
        // Nothing asked for: whatever the credential is, is what we are.
        (_, None) => Ok(Exchange::UseBase),
        // The ordinary path: a human credential, exchanged for the estate's account.
        (None, Some(sa)) => Ok(Exchange::Mint(sa.to_string())),
        // Already that account. Exchanging again would be self-impersonation.
        (Some(have), Some(want)) if have == want => Ok(Exchange::UseBase),
        // Two different accounts is not a chain to build — it is a mistake to report.
        (Some(have), Some(want)) => Err(format!(
            "the credentials impersonate {} but this estate runs as {} — log in without \
             --impersonate-service-account, or point satz at the estate whose account that is",
            have, want
        )),
    }
}

/// Exchange the caller's token for one minted AS the service account —
/// iamcredentials `generateAccessToken`, the same call an impersonating ADC
/// makes. A denial names the missing TokenCreator grant and the opt-out.
async fn impersonated_token(base: &str, sa: &str) -> Result<String, String> {
    mint_impersonated(base, sa).await.map(|(token, _)| token)
}

/// One `generateAccessToken`: the token and the Unix second it expires.
async fn mint_impersonated(base: &str, sa: &str) -> Result<(String, u64), String> {
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
    let token = v
        .get("accessToken")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "generateAccessToken returned no accessToken".to_string())?;
    let expires = v
        .get("expireTime")
        .and_then(|t| t.as_str())
        .and_then(parse_expire_time)
        .ok_or_else(|| "generateAccessToken returned no readable expireTime".to_string())?;
    Ok((token, expires))
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
/// the existing ADC as the source.
///
/// An ADC that ALREADY impersonates is kept as-is — re-wrapping would chain
/// impersonations — but only when it names the same account. It used to be
/// kept whatever account it named, which silently served the estate with a
/// different service account's credentials; [`plan_impersonation`] now decides,
/// and it is the same decision the REST path makes.
fn impersonated_credential_json(sa: &str) -> Result<serde_json::Value, String> {
    let path = crate::org_policy::adc_file_path().ok_or(
        "no ADC file to impersonate from — run `gcloud auth application-default login`, or pass --no-impersonate",
    )?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let source: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
    if source.get("type").and_then(|t| t.as_str()) == Some("impersonated_service_account") {
        // Errors on a mismatch, returns UseBase when it is already this account.
        plan_impersonation(identity::email_from_adc_json(&source).as_deref(), Some(sa))?;
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

    // --- reconciling the credential's identity with the estate's --------------

    const ACME: &str = "svc-iac-001@acme-infra-001.iam.gserviceaccount.com";
    const BOLT: &str = "svc-iac-001@bolt-infra-001.iam.gserviceaccount.com";

    /// The ordinary case: a human credential, exchanged for the estate's account.
    #[test]
    fn a_human_credential_is_exchanged_for_the_estate_account() {
        assert_eq!(plan_impersonation(None, Some(ACME)), Ok(Exchange::Mint(ACME.to_string())));
    }

    /// The REST path used to exchange unconditionally, so a credential that was
    /// ALREADY the estate's account asked `generateAccessToken` to mint that
    /// account a token for itself — a 403 unless it holds TokenCreator on itself.
    #[test]
    fn a_credential_that_is_already_the_estate_account_is_not_exchanged_again() {
        assert_eq!(plan_impersonation(Some(ACME), Some(ACME)), Ok(Exchange::UseBase));
    }

    /// The gRPC path used to accept any impersonating credential and ignore the
    /// account the estate asked for — so one customer's credentials quietly
    /// served another customer's estate.
    #[test]
    fn a_credential_for_a_different_account_is_refused_and_names_both() {
        let err = plan_impersonation(Some(BOLT), Some(ACME))
            .expect_err("a mismatched credential must not be used");
        assert!(err.contains("bolt-infra-001"), "the error hides the credential: {}", err);
        assert!(err.contains("acme-infra-001"), "the error hides the estate: {}", err);
    }

    /// Nothing asked for: whatever the credential is, is what we are. This is
    /// `--no-impersonate` and every local-mode estate.
    #[test]
    fn with_no_estate_target_the_credential_is_used_as_it_is() {
        assert_eq!(plan_impersonation(None, None), Ok(Exchange::UseBase));
        assert_eq!(plan_impersonation(Some(ACME), None), Ok(Exchange::UseBase));
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

#[cfg(test)]
mod expire_time_tests {
    use super::parse_expire_time;

    #[test]
    fn rfc3339_utc_reads_as_unix_seconds() {
        assert_eq!(parse_expire_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_expire_time("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_expire_time("2026-09-11T13:45:07Z"), Some(1_789_134_307));
        assert_eq!(parse_expire_time("2026-09-11T13:45:07.123456Z"), Some(1_789_134_307));
        assert_eq!(parse_expire_time("2026-09-11T13:45:07+02:00"), None);
        assert_eq!(parse_expire_time("not a time"), None);
    }
}
