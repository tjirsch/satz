//! Which identity the Application Default Credentials resolve to — and one
//! line that says so before the first API call of every live command.
//!
//! The fleet experience this exists for: switching ADCs per customer, every
//! wrong login surfaced only as a downstream 403 (`invalid_rapt`, a denied
//! `orgpolicy.policy.get`, a CAI quota error). A printed
//! `credentials: <who> (<type>), quota project <p>` catches the wrong account
//! before the first call; `satz whoami` is the explicit check.

use google_cloud_auth::credentials::Builder;

/// What kind of credential the ADC file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredKind {
    /// `gcloud auth application-default login` — the file stores no identity.
    UserAdc,
    /// `--impersonate-service-account` ADC — the target is in the file.
    ImpersonatedSa,
    /// A service-account key file.
    SaKey,
    /// No file, or a shape this code does not know (e.g. GCE metadata).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CredentialInfo {
    pub(crate) email: Option<String>,
    pub(crate) kind: CredKind,
    pub(crate) quota_project: Option<String>,
}

/// What the ADC file alone says: kind, identity (absent for user ADC — that
/// is why the online path exists), and the quota project (env overrides
/// first, like every client). `None` when there is no parseable ADC file.
pub(crate) fn credential_info_offline() -> Option<CredentialInfo> {
    let path = crate::org_policy::adc_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let kind = match json.get("type").and_then(|t| t.as_str()) {
        Some("authorized_user") => CredKind::UserAdc,
        Some("impersonated_service_account") => CredKind::ImpersonatedSa,
        Some("service_account") => CredKind::SaKey,
        _ => CredKind::Unknown,
    };
    Some(CredentialInfo {
        email: email_from_adc_json(&json),
        kind,
        quota_project: crate::org_policy::resolve_quota_project(),
    })
}

/// The best available picture: the ADC file first, then — only when the file
/// carries no identity — ONE tokeninfo call on the token already in hand.
pub(crate) async fn credential_info(token: &str) -> CredentialInfo {
    let mut info = credential_info_offline().unwrap_or(CredentialInfo {
        email: None,
        kind: CredKind::Unknown,
        quota_project: crate::org_policy::resolve_quota_project(),
    });
    if info.email.is_none() {
        // The token goes in the form body rather than the query string so it
        // cannot leak into proxy or CDN access logs.
        let client = reqwest::Client::new();
        if let Ok(res) = client
            .post("https://oauth2.googleapis.com/tokeninfo")
            .form(&[("access_token", token)])
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(body) = res.json::<serde_json::Value>().await {
                    info.email = email_from_identity_json(&body);
                }
            }
        }
    }
    info
}

/// The one line. Pure, so its shape is pinned by tests.
pub(crate) fn render_credential_line(info: &CredentialInfo) -> String {
    let kind = match info.kind {
        CredKind::UserAdc => "user ADC",
        CredKind::ImpersonatedSa => "impersonated service account",
        CredKind::SaKey => "service account key",
        CredKind::Unknown => "unknown credential type",
    };
    let who = info.email.as_deref().unwrap_or("(identity unknown)");
    match &info.quota_project {
        Some(q) => format!("credentials: {} ({}), quota project {}", who, kind, q),
        None => format!("credentials: {} ({}), no quota project", who, kind),
    }
}

static ANNOUNCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Print the credential line — once per process, however many clients a
/// command builds. Sits behind `gcp::access_token()`, so every live command
/// gets the line without knowing about it.
pub(crate) async fn announce(token: &str) {
    if ANNOUNCED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let info = credential_info(token).await;
    println!("{}", render_credential_line(&info));
}

/// Suppress the automatic line for a command that prints its own picture
/// (whoami).
pub(crate) fn mark_announced() {
    ANNOUNCED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// `satz whoami [--offline]`: the explicit check that the ADC is the account
/// you think it is — the one-line answer to a fleet of per-customer logins.
pub(crate) async fn whoami(offline: bool) -> Result<(), Box<dyn std::error::Error>> {
    if offline {
        let Some(info) = credential_info_offline() else {
            return Err("no Application Default Credentials file found — run `gcloud auth \
                        application-default login`"
                .into());
        };
        println!("{}", render_credential_line(&info));
        if let Some(p) = crate::org_policy::adc_file_path() {
            println!("adc file: {}", p.display());
        }
        if info.email.is_none() && info.kind == CredKind::UserAdc {
            println!("note: a user ADC file stores no identity — run `satz whoami` without --offline to resolve it");
        }
        return Ok(());
    }

    mark_announced();
    let token = crate::gcp::access_token().await.map_err(|e| {
        format!(
            "could not get an Application Default Credentials token ({}) — run `gcloud auth \
             application-default login`",
            e
        )
    })?;
    let info = credential_info(&token).await;
    println!("{}", render_credential_line(&info));
    if let Some(p) = crate::org_policy::adc_file_path() {
        println!("adc file: {}", p.display());
    }
    if info.email.is_none() {
        return Err("could not determine the identity behind these credentials".into());
    }
    Ok(())
}

/// How the ADC identity was established, so an unexpected result can be
/// traced back to the mechanism that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrincipalSource {
    TokenInfo,
    Signer,
    UserInfo,
    AdcFile,
}

impl PrincipalSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PrincipalSource::TokenInfo => "token introspection",
            PrincipalSource::Signer => "credential signer",
            PrincipalSource::UserInfo => "userinfo endpoint",
            PrincipalSource::AdcFile => "ADC credentials file",
        }
    }
}

/// Read `email` out of a tokeninfo or userinfo response body.
fn email_from_identity_json(v: &serde_json::Value) -> Option<String> {
    v.get("email")
        .and_then(|e| e.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Read the service-account address out of an ADC credentials file: either a
/// key file's `client_email`, or the impersonation target in
/// `.../serviceAccounts/{email}:generateAccessToken`.
fn email_from_adc_json(v: &serde_json::Value) -> Option<String> {
    if let Some(email) = v
        .get("client_email")
        .and_then(|e| e.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(email.to_string());
    }

    let url = v.get("service_account_impersonation_url")?.as_str()?;
    let email = url.split("/serviceAccounts/").nth(1)?.split(':').next()?;
    if email.is_empty() { None } else { Some(email.to_string()) }
}

/// Determine which principal the Application Default Credentials represent.
///
/// Tried in cost order; `None` means no mechanism could tell us, which the
/// caller treats as "cannot verify" rather than "verified".
pub(crate) async fn resolve_adc_identity(
    client: &reqwest::Client,
    token: &str,
) -> Option<(String, PrincipalSource)> {
    // 1. Introspect the token we already hold. This covers `gcloud auth
    //    application-default login`, the credential type expected here, and needs no
    //    extra scope. The token goes in the form body rather than the query string so it
    //    cannot leak into proxy or CDN access logs.
    if let Ok(res) = client
        .post("https://oauth2.googleapis.com/tokeninfo")
        .form(&[("access_token", token)])
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(body) = res.json::<serde_json::Value>().await {
                if let Some(email) = email_from_identity_json(&body) {
                    return Some((email, PrincipalSource::TokenInfo));
                }
            }
        }
    }

    // 2. Service-account, impersonated-service-account and GCE metadata credentials
    //    expose the address directly and offline. `authorized_user` and
    //    `external_account` have no signer and fall through.
    if let Ok(signer) = Builder::default().build_signer() {
        if let Ok(email) = signer.client_email().await {
            if !email.trim().is_empty() {
                return Some((email, PrincipalSource::Signer));
            }
        }
    }

    // 3. userinfo needs a scope the primary credentials deliberately do not request.
    //    Build a separate credential for the probe: `with_scopes` is forwarded to the
    //    wire for every backend (user-account refresh, external-account STS exchange,
    //    metadata query), and STS in particular often accepts only cloud-platform — so
    //    widening the credential that mints the working token could break environments
    //    that work today. The worst case here is that we simply learn nothing.
    if let Ok(creds) = Builder::default()
        .with_scopes([
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
        ])
        .build_access_token_credentials()
    {
        if let Ok(probe) = creds.access_token().await {
            if let Ok(res) = client
                .get("https://www.googleapis.com/oauth2/v3/userinfo")
                .bearer_auth(&probe.token)
                .send()
                .await
            {
                if res.status().is_success() {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if let Some(email) = email_from_identity_json(&body) {
                            return Some((email, PrincipalSource::UserInfo));
                        }
                    }
                }
            }
        }
    }

    // 4. Last resort: read the ADC file off disk.
    credential_info_offline()?
        .email
        .map(|email| (email, PrincipalSource::AdcFile))
}

// Tests (pure layer only — no network, no filesystem).
#[cfg(test)]
mod tests {
    use super::*;

    fn jv(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("valid test JSON")
    }

    // --- ADC identity extraction -------------------------------------------

    #[test]
    fn email_from_tokeninfo_body() {
        let v = jv(r#"{"azp":"1.apps.googleusercontent.com","scope":"...cloud-platform","email":"admin@example.com","email_verified":"true"}"#);
        assert_eq!(email_from_identity_json(&v).as_deref(), Some("admin@example.com"));
    }

    #[test]
    fn email_from_userinfo_body() {
        let v = jv(r#"{"sub":"117","email":"admin@example.com","email_verified":true}"#);
        assert_eq!(email_from_identity_json(&v).as_deref(), Some("admin@example.com"));
    }

    #[test]
    fn email_absent_empty_or_null_is_none() {
        assert_eq!(email_from_identity_json(&jv(r#"{"scope":"x"}"#)), None);
        assert_eq!(email_from_identity_json(&jv(r#"{"email":""}"#)), None);
        assert_eq!(email_from_identity_json(&jv(r#"{"email":"   "}"#)), None);
        assert_eq!(email_from_identity_json(&jv(r#"{"email":null}"#)), None);
    }

    #[test]
    fn email_from_service_account_key_file() {
        let v = jv(r#"{"type":"service_account","client_email":"svc@p.iam.gserviceaccount.com"}"#);
        assert_eq!(email_from_adc_json(&v).as_deref(), Some("svc@p.iam.gserviceaccount.com"));
    }

    #[test]
    fn email_from_impersonation_url() {
        let v = jv(r#"{"type":"impersonated_service_account","service_account_impersonation_url":"https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/target@p.iam.gserviceaccount.com:generateAccessToken"}"#);
        assert_eq!(email_from_adc_json(&v).as_deref(), Some("target@p.iam.gserviceaccount.com"));
    }

    #[test]
    fn authorized_user_adc_carries_no_email() {
        // gcloud user ADC has no identity in the file; that is why the REST probes exist.
        let v = jv(r#"{"type":"authorized_user","client_id":"x","refresh_token":"y"}"#);
        assert_eq!(email_from_adc_json(&v), None);
    }

    // --- the credential line -----------------------------------------------

    #[test]
    fn credential_line_shapes_are_pinned() {
        let user = CredentialInfo {
            email: Some("admin@example.com".into()),
            kind: CredKind::UserAdc,
            quota_project: Some("acme-infra-001".into()),
        };
        assert_eq!(
            render_credential_line(&user),
            "credentials: admin@example.com (user ADC), quota project acme-infra-001"
        );

        let imp = CredentialInfo {
            email: Some("svc-iac@acme-infra-001.iam.gserviceaccount.com".into()),
            kind: CredKind::ImpersonatedSa,
            quota_project: None,
        };
        assert_eq!(
            render_credential_line(&imp),
            "credentials: svc-iac@acme-infra-001.iam.gserviceaccount.com \
             (impersonated service account), no quota project"
        );

        let unknown = CredentialInfo { email: None, kind: CredKind::Unknown, quota_project: None };
        assert_eq!(
            render_credential_line(&unknown),
            "credentials: (identity unknown) (unknown credential type), no quota project"
        );
    }
}
