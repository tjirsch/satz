//! Which identity the Application Default Credentials resolve to — and one
//! line that says so before the first API call of every live command.
//!
//! The fleet experience this exists for: switching ADCs per customer, every
//! wrong login surfaced only as a downstream 403 (`invalid_rapt`, a denied
//! `orgpolicy.policy.get`, a CAI quota error). A printed
//! `credentials: <who> (<type>), quota project <p>` catches the wrong account
//! before the first call; `satz whoami` is the explicit check.

// schemars comes through rmcp: one version in the tree
use rmcp::schemars;
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

/// Which identity the line should name. With impersonation configured it is
/// the identity the calls actually run as — the estate's IaC SA — not the
/// human behind it.
async fn announce_info(token: &str) -> CredentialInfo {
    if let Some(sa) = crate::gcp::impersonation_target() {
        return CredentialInfo {
            email: Some(sa),
            kind: CredKind::ImpersonatedSa,
            quota_project: crate::org_policy::resolve_quota_project(),
        };
    }
    credential_info(token).await
}

/// Write the credential line to `sink`. Split out from [`announce`] so a test
/// can read the line back without capturing a process stream.
pub(crate) fn announce_to(sink: &mut impl std::io::Write, info: &CredentialInfo) {
    // If the diagnostic stream itself is gone there is nothing to report it to.
    let _ = writeln!(sink, "{}", render_credential_line(info));
}

/// Announce the credential line — once per process, however many clients a
/// command builds. Sits behind `gcp::access_token()`, so every live command
/// gets the line without knowing about it.
///
/// It goes to STDERR for the same reason the version banner does: stdout
/// carries the ANSWER. Under `satz mcp` stdout carries the JSON-RPC stream, so
/// a line printed there is not untidy output — it is a protocol error that
/// breaks the client on the first live tool call.
pub(crate) async fn announce(token: &str) {
    if ANNOUNCED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let info = announce_info(token).await;
    announce_to(&mut std::io::stderr(), &info);
}

/// Suppress the automatic line for a command that prints its own picture
/// (whoami).
pub(crate) fn mark_announced() {
    ANNOUNCED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// `satz whoami [--offline]`: the explicit check that the ADC is the account you
/// think it is — the one-line answer to a fleet of per-customer logins.
///
/// BOTH halves, because a live command uses both. The ADC is who you are to
/// Google; the estate's service account is who satz then acts as, and after init
/// that is what every read and write runs as. Reporting only the first cost a
/// round-trip on two organisations: the answer looked right and the next command
/// failed for a reason the line did not mention.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct Credential {
    /// Absent for a user ADC file read offline: the file stores no identity.
    pub account: Option<String>,
    /// user-adc | impersonated-sa | sa-key | unknown
    pub kind: &'static str,
    pub file: Option<String>,
}

/// The identity an estate's live calls run as, and whether this credential may
/// actually become it.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EstateIdentity {
    pub service_account: String,
    /// `None` when not checked (offline). Checked with one `generateAccessToken`
    /// whose token is discarded — the same call every live command makes, so a
    /// failure here is the failure that command would hit.
    pub may_impersonate: Option<bool>,
    pub error: Option<String>,
}

/// The project every API call is billed and quota'd against.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct QuotaProject {
    pub id: String,
    /// `None` when not checked (offline).
    pub reachable: Option<bool>,
    pub error: Option<String>,
}

/// Who the credentials say we are, as data.
///
/// The `note` is the one thing the human line carries that the fields do not: a
/// user ADC file stores no identity, so `--offline` cannot answer the question it
/// was asked, and saying so is more useful than an empty account.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct WhoamiReport {
    pub adc: Credential,
    /// Absent when no estate is in play — then the ADC identity is the answer.
    pub estate: Option<EstateIdentity>,
    pub quota_project: Option<QuotaProject>,
    /// true when the answer came from the file alone, without minting a token
    pub offline: bool,
    pub note: Option<String>,
}

/// Whether the credential can reach the project it names as its quota project.
///
/// The trap this exists for: an ADC file carrying a quota project the user
/// cannot see — a typo of the real one — is accepted by everything that merely
/// PRINTS it, and then fails every API call with `UserProjectInvalid`, or with
/// "cannot create the authentication headers", naming neither the project nor
/// the fix. Two organisations lost a round-trip to it.
pub(crate) async fn check_quota_project(
    token: &str,
    project: &str,
    suggest: Option<&str>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let fix = format!(
        "run `gcloud auth application-default set-quota-project {}`",
        suggest.unwrap_or("<the estate's infra_project_name>")
    );
    match crate::gcp::resourcemanager::get_project_number(&client, token, project).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!(
            "quota project {}: not accessible to these credentials (or does not exist) — {}",
            project, fix
        )),
        Err(e) => Err(format!("quota project {}: {} — {}", project, e, fix)),
    }
}

/// The answer to "who am I", resolved ONCE for both surfaces.
///
/// The terminal command and `satz_whoami` used to resolve it separately, and
/// they drifted: only the tool knew that a bound estate changes the answer, so
/// `satz whoami` could not be asked the question the tool could answer. One
/// resolver, two renderings — the divergence cannot come back.
pub(crate) async fn whoami_report(offline: bool) -> Result<WhoamiReport, Box<dyn std::error::Error>> {
    let file = crate::org_policy::adc_file_path().map(|p| p.display().to_string());
    let kind_name = |k: &CredKind| match k {
        CredKind::UserAdc => "user-adc",
        CredKind::ImpersonatedSa => "impersonated-sa",
        CredKind::SaKey => "sa-key",
        CredKind::Unknown => "unknown",
    };
    let bound = crate::gcp::impersonation_target();
    let quota = crate::org_policy::resolve_quota_project();

    if offline {
        // The file alone: no token, so neither check can run, and saying "not
        // checked" is the honest answer rather than an optimistic one.
        // With an estate in play there is still an answer without a credential —
        // WHICH account this estate runs as is read off the estate, not off the
        // ADC. Only when neither exists is there nothing to report.
        let found = credential_info_offline();
        if found.is_none() && bound.is_none() {
            return Err("no Application Default Credentials file found — run `gcloud auth \
                        application-default login`"
                .into());
        }
        let info = found.unwrap_or(CredentialInfo {
            email: None,
            kind: CredKind::Unknown,
            quota_project: None,
        });
        let note = (info.email.is_none() && info.kind == CredKind::UserAdc).then(|| {
            "a user ADC file stores no identity — run without --offline to resolve it".to_string()
        });
        return Ok(WhoamiReport {
            adc: Credential { account: info.email, kind: kind_name(&info.kind), file },
            estate: bound.map(|sa| EstateIdentity {
                service_account: sa,
                may_impersonate: None,
                error: None,
            }),
            quota_project: quota.map(|id| QuotaProject { id, reachable: None, error: None }),
            offline: true,
            note,
        });
    }

    // The BASE credential, never the impersonated one: the question "who am I"
    // is about the account you logged in as, and the estate half is reported
    // beside it rather than in place of it.
    mark_announced();
    let token = crate::gcp::base_access_token().await.map_err(|e| {
        format!(
            "could not get an Application Default Credentials token ({}) — run `gcloud auth \
             application-default login`",
            e
        )
    })?;
    let info = credential_info(&token).await;
    if info.email.is_none() {
        return Err("could not determine the identity behind these credentials".into());
    }

    let quota_project = match quota {
        None => None,
        Some(id) => {
            let checked = check_quota_project(&token, &id, bound.as_deref().and_then(project_of)).await;
            Some(QuotaProject {
                id,
                reachable: Some(checked.is_ok()),
                error: checked.err(),
            })
        }
    };

    let estate = match &bound {
        None => None,
        Some(sa) => {
            let allowed = crate::gcp::may_impersonate(&token, sa).await;
            Some(EstateIdentity {
                service_account: sa.clone(),
                may_impersonate: Some(allowed.is_ok()),
                error: allowed.err(),
            })
        }
    };

    Ok(WhoamiReport {
        adc: Credential { account: info.email, kind: kind_name(&info.kind), file },
        estate,
        quota_project,
        offline: false,
        note: None,
    })
}

/// The project an estate's service account lives in — the right answer to
/// "which project should the quota project be", because it is the one the
/// estate itself declares.
pub(crate) fn project_of(sa: &str) -> Option<&str> {
    sa.split_once('@')?.1.strip_suffix(".iam.gserviceaccount.com")
}

/// The terminal rendering of `whoami_report` — the same answer `satz_whoami`
/// returns as data.
pub(crate) async fn whoami(offline: bool) -> Result<(), Box<dyn std::error::Error>> {
    let r = whoami_report(offline).await?;
    println!("{}", render_whoami(&r));
    // A credential that cannot become the estate's account, or a quota project
    // nothing can reach, makes every later command fail. `whoami` is the command
    // an operator runs to find that out, so it says so in its exit code too.
    let broken = r.estate.as_ref().is_some_and(|e| e.may_impersonate == Some(false))
        || r.quota_project.as_ref().is_some_and(|q| q.reachable == Some(false));
    if broken {
        return Err("the credentials cannot do what this estate needs — see above".into());
    }
    Ok(())
}

/// A Google API error carries its whole JSON body, which is fifteen lines of
/// braces around one sentence. `whoami` exists to be READ, so the terminal gets
/// the head and the tail — the tail is where satz appends the remedy — and the
/// report keeps the error whole for anything that wants it.
fn brief(msg: &str) -> String {
    let flat = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 240 {
        return flat;
    }
    let head: String = flat.chars().take(110).collect();
    let tail: String = flat.chars().skip(flat.chars().count().saturating_sub(110)).collect();
    // Start the tail at a word, not mid-token: a cut inside a role name reads as
    // a different role.
    let tail = tail.split_once(' ').map_or(tail.as_str(), |(_, rest)| rest);
    format!("{} […] {}", head.trim_end(), tail.trim_start())
}

/// Both halves, one block. The ADC line keeps the wording every live command
/// prints, so the two are recognisably the same fact.
pub(crate) fn render_whoami(r: &WhoamiReport) -> String {
    let kind = match r.adc.kind {
        "user-adc" => "user ADC",
        "impersonated-sa" => "impersonated service account",
        "sa-key" => "service account key",
        _ => "unknown credential type",
    };
    let mut out = format!(
        "credentials: {} ({})",
        r.adc.account.as_deref().unwrap_or("(identity unknown)"),
        kind
    );
    if let Some(f) = &r.adc.file {
        out.push_str(&format!("\nadc file:    {}", f));
    }
    match &r.estate {
        None => out.push_str(
            "\nruns as:     the credentials themselves — no estate, or a local-mode one",
        ),
        Some(e) => {
            out.push_str(&format!("\nruns as:     {}", e.service_account));
            match (e.may_impersonate, &e.error) {
                (Some(true), _) => out.push_str(" — may impersonate"),
                (Some(false), Some(why)) => {
                    out.push_str(&format!("\n             CANNOT IMPERSONATE: {}", brief(why)))
                }
                (Some(false), None) => out.push_str(" — CANNOT IMPERSONATE"),
                (None, _) => out.push_str(" — not checked (--offline)"),
            }
        }
    }
    match &r.quota_project {
        None => out.push_str("\nquota project: none — API calls are billed to the caller's own"),
        Some(q) => {
            out.push_str(&format!("\nquota project: {}", q.id));
            match (q.reachable, &q.error) {
                (Some(true), _) => out.push_str(" — reachable"),
                (Some(false), Some(why)) => {
                    out.push_str(&format!("\n             UNREACHABLE: {}", brief(why)))
                }
                (Some(false), None) => out.push_str(" — UNREACHABLE"),
                (None, _) => out.push_str(" — not checked (--offline)"),
            }
        }
    }
    if let Some(n) = &r.note {
        out.push_str(&format!("\nnote: {}", n));
    }
    out
}

/// Everything `init --from-live` can derive from the ADC alone.
pub(crate) struct LiveDefaults {
    /// Local part of the ADC identity.
    pub(crate) first_admin: String,
    pub(crate) customer_domain: String,
    /// Bare organization number — `None` on a greenfield tenant.
    pub(crate) org_id: Option<String>,
    /// The directory customer id (`C0…`).
    pub(crate) customer_id: Option<String>,
    /// Bare billing account id — only when exactly ONE open account is
    /// visible; anything else is never guessed.
    pub(crate) billing_account: Option<String>,
}

/// Derive the init parameters from the credentials: tokeninfo → identity,
/// `organizations:search` → org id + directory customer id,
/// `billingAccounts.list` → the single open account. Ambiguity is an error or
/// a named gap, never a guess — and only what is actually MISSING is queried,
/// so explicit flags keep working on accounts that see many organizations.
pub(crate) async fn live_defaults(
    need_org: bool,
    need_billing: bool,
) -> Result<LiveDefaults, String> {
    let token = crate::gcp::access_token().await?;
    let info = credential_info(&token).await;
    let Some(email) = info.email else {
        return Err(
            "could not determine the ADC identity — run `gcloud auth application-default login`"
                .to_string(),
        );
    };
    let Some((local, domain)) = email.split_once('@') else {
        return Err(format!("ADC identity {:?} is not an email address", email));
    };

    let client = reqwest::Client::new();
    let (org_id, customer_id) = if !need_org {
        // Both values arrived as flags — no search, no ambiguity to trip on.
        (None, None)
    } else {
        let orgs = crate::gcp::resourcemanager::search_organizations(&client, &token)
            .await
            .map_err(|e| format!("could not search organizations: {}", e))?;
        match orgs.as_slice() {
            [] => (None, None),
            [one] => (
                one.get("name")
                    .and_then(|n| n.as_str())
                    .and_then(|n| n.strip_prefix("organizations/"))
                    .map(str::to_string),
                one.get("directoryCustomerId").and_then(|c| c.as_str()).map(str::to_string),
            ),
            many => {
                return Err(format!(
                    "{} organizations are visible to {} — pass --customer-organization-id and \
                     --customer-id explicitly: {}",
                    many.len(),
                    email,
                    many.iter()
                        .filter_map(|o| o.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    };

    let billing_account = if !need_billing {
        None
    } else {
        let accounts = crate::gcp::billing::list_billing_accounts(&client, &token)
            .await
            .map_err(|e| format!("could not list billing accounts: {}", e))?;
        let open: Vec<&(String, String, bool)> = accounts.iter().filter(|(_, _, o)| *o).collect();
        match open.as_slice() {
            [one] => Some(one.0.clone()),
            [] => {
                eprintln!("no open billing account is visible — pass --billing-account-infra");
                None
            }
            many => {
                eprintln!(
                    "{} open billing accounts are visible — pass --billing-account-infra explicitly:",
                    many.len()
                );
                for (id, name, _) in many {
                    eprintln!("  {} ({})", id, name);
                }
                None
            }
        }
    };

    let not_derived = |needed: bool, v: &Option<String>, absent: &'static str| -> String {
        match v {
            Some(v) => v.clone(),
            None if needed => absent.to_string(),
            None => "(passed explicitly)".to_string(),
        }
    };
    println!(
        "derived from the ADC: first_admin {}@{}, organization {}, customer id {}, billing account {}",
        local,
        domain,
        not_derived(need_org, &org_id, "(none visible)"),
        not_derived(need_org, &customer_id, "(unknown)"),
        not_derived(need_billing, &billing_account, "(not settled)")
    );
    Ok(LiveDefaults {
        first_admin: local.to_string(),
        customer_domain: domain.to_string(),
        org_id,
        customer_id,
        billing_account,
    })
}

/// Ask for the one value that cannot be derived. Interactive terminals only;
/// a scripted run must pass --customer-shortname.
pub(crate) fn prompt_shortname() -> Result<String, String> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Err(
            "--from-live needs --customer-shortname when not run interactively (every name \
             derives from it)"
                .to_string(),
        );
    }
    print!("customer_shortname (every derived name builds on it, e.g. acme): ");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s).map_err(|e| e.to_string())?;
    let s = s.trim().to_string();
    if s.is_empty() {
        return Err("customer_shortname must not be empty".to_string());
    }
    Ok(s)
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
pub(crate) fn email_from_adc_json(v: &serde_json::Value) -> Option<String> {
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

    /// The line is diagnostics and must be writable to a stream that is not
    /// stdout — under `satz mcp`, stdout is the JSON-RPC transport and a stray
    /// line there breaks the client rather than just looking untidy.
    #[test]
    fn the_credential_line_goes_to_the_sink_it_is_given() {
        let info = CredentialInfo {
            email: Some("svc-iac@acme-infra-001.iam.gserviceaccount.com".into()),
            kind: CredKind::ImpersonatedSa,
            quota_project: Some("acme-infra-001".into()),
        };
        let mut sink: Vec<u8> = Vec::new();
        announce_to(&mut sink, &info);
        assert_eq!(
            String::from_utf8(sink).expect("utf-8"),
            "credentials: svc-iac@acme-infra-001.iam.gserviceaccount.com \
             (impersonated service account), quota project acme-infra-001\n"
        );
    }

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

#[cfg(test)]
mod whoami_render_tests {
    //! `whoami` is the command an operator runs when something is already wrong,
    //! so what it PRINTS is the feature. Two organisations lost a round-trip to a
    //! line that reported the ADC and nothing else: the credential was fine, and
    //! the thing that was broken — the account it has to become, the project it
    //! bills — was not on screen.
    use super::{Credential, EstateIdentity, QuotaProject, WhoamiReport, brief, project_of, render_whoami};

    fn report() -> WhoamiReport {
        WhoamiReport {
            adc: Credential {
                account: Some("person@example.com".into()),
                kind: "user-adc",
                file: Some("/adc.json".into()),
            },
            estate: None,
            quota_project: None,
            offline: false,
            note: None,
        }
    }

    #[test]
    fn both_halves_are_named_even_when_there_is_no_estate() {
        let out = render_whoami(&report());
        assert!(out.contains("person@example.com (user ADC)"), "{}", out);
        assert!(out.contains("/adc.json"), "{}", out);
        // Silence about the second half is what cost the round-trips: say that
        // the credentials are the answer, rather than leaving the line out.
        assert!(out.contains("runs as:"), "{}", out);
        assert!(out.contains("quota project:"), "{}", out);
    }

    #[test]
    fn a_credential_that_cannot_become_the_estate_says_so_loudly() {
        let mut r = report();
        r.estate = Some(EstateIdentity {
            service_account: "svc-iac-001@acme-infra-001.iam.gserviceaccount.com".into(),
            may_impersonate: Some(false),
            error: Some("403 denied".into()),
        });
        let out = render_whoami(&r);
        assert!(out.contains("CANNOT IMPERSONATE"), "{}", out);
        assert!(out.contains("svc-iac-001@acme-infra-001"), "{}", out);
        assert!(out.contains("403 denied"), "{}", out);
    }

    #[test]
    fn an_unreachable_quota_project_says_so_loudly() {
        let mut r = report();
        r.quota_project = Some(QuotaProject {
            id: "acme-infra-001".into(),
            reachable: Some(false),
            error: Some("not accessible — run `gcloud …`".into()),
        });
        let out = render_whoami(&r);
        assert!(out.contains("UNREACHABLE"), "{}", out);
        assert!(out.contains("acme-infra-001"), "{}", out);
    }

    /// `--offline` mints no token, so neither check can run. "not checked" is the
    /// honest word; reporting them as fine would be the more useful-looking lie.
    #[test]
    fn offline_reports_the_checks_as_not_made() {
        let mut r = report();
        r.offline = true;
        r.estate = Some(EstateIdentity {
            service_account: "svc-iac-001@acme-infra-001.iam.gserviceaccount.com".into(),
            may_impersonate: None,
            error: None,
        });
        r.quota_project =
            Some(QuotaProject { id: "acme-infra-001".into(), reachable: None, error: None });
        let out = render_whoami(&r);
        assert_eq!(out.matches("not checked (--offline)").count(), 2, "{}", out);
        assert!(!out.contains("CANNOT"), "{}", out);
        assert!(!out.contains("UNREACHABLE"), "{}", out);
    }

    /// The remedy satz appends is at the END of an API error, so a truncation
    /// that keeps only the head throws away the useful half.
    #[test]
    fn shortening_an_api_error_keeps_the_remedy() {
        let long = format!("cannot impersonate x ({} ) — run `gcloud auth foo`", "{\"error\": 1} ".repeat(60));
        let short = brief(&long);
        assert!(short.contains("cannot impersonate x"), "{}", short);
        assert!(short.ends_with("run `gcloud auth foo`"), "{}", short);
        assert!(short.contains("[…]"), "{}", short);
        assert!(!short.contains('\n'), "{}", short);
        // A short message is passed through, not padded with an elision.
        assert_eq!(brief("plain and short"), "plain and short");
    }

    /// The suggested quota project is the estate's own infra project, read off
    /// the service account rather than asked for.
    #[test]
    fn the_infra_project_is_read_off_the_service_account() {
        assert_eq!(
            project_of("svc-iac-001@acme-infra-001.iam.gserviceaccount.com"),
            Some("acme-infra-001")
        );
        assert_eq!(project_of("person@example.com"), None);
        assert_eq!(project_of("nonsense"), None);
    }
}
