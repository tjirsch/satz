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

/// What an estate declares about the identity its live calls use, read off its
/// params — no credential needed, so `--offline` answers it too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EstateDeclaration {
    /// The estate as it was named: the argument `satz migrate` takes.
    pub(crate) path: String,
    /// `deployment_mode`, `local` when the estate declares none — as the emitter
    /// reads it.
    pub(crate) mode: String,
    /// `{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com`, when the
    /// estate declares both.
    pub(crate) service_account: Option<String>,
}

impl EstateDeclaration {
    /// From an estate's params, looked up by their kebab-case names.
    pub(crate) fn from_params(path: String, get: impl Fn(&str) -> Option<String>) -> Self {
        let mode = get("deployment-mode").filter(|m| !m.is_empty()).unwrap_or_else(|| "local".into());
        let service_account = match (get("svc-iac-account"), get("infra-project-name")) {
            (Some(a), Some(p)) if !a.is_empty() && !p.is_empty() => {
                Some(format!("{}@{}.iam.gserviceaccount.com", a, p))
            }
            _ => None,
        };
        Self { path, mode, service_account }
    }

    /// The account live calls impersonate: the declared one, in cloud mode only —
    /// the emitter's provider rule.
    pub(crate) fn impersonation_target(&self) -> Option<&str> {
        if self.mode == "cloud" { self.service_account.as_deref() } else { None }
    }
}

/// The estate the answer is for: what it declares, whether its live calls
/// impersonate the account it declares, and whether this credential may.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EstateIdentity {
    /// The estate as it was named: the argument `satz migrate` takes.
    pub path: String,
    /// `cloud` or `local`, as the estate declares it; `local` when it declares none.
    pub deployment_mode: String,
    /// The IaC service account the estate declares (`svc_iac_account` at
    /// `infra_project_name`). Absent when it declares no such pair.
    pub service_account: Option<String>,
    /// true: live calls run as `service_account`, impersonated by the ADC account.
    /// false: they run as the ADC account itself — in local mode, where
    /// `satz migrate <path> --mode cloud` switches to impersonation, or in cloud
    /// mode under `--no-impersonate`.
    pub impersonated: bool,
    /// `None` when not checked: offline, or nothing is impersonated. Checked with
    /// one `generateAccessToken` whose token is discarded — the same call every
    /// live command makes, so a failure here is the failure that command would hit.
    pub may_impersonate: Option<bool>,
    pub error: Option<String>,
}

impl EstateIdentity {
    fn new(d: EstateDeclaration, impersonated: bool, checked: Option<Result<(), String>>) -> Self {
        Self {
            path: d.path,
            deployment_mode: d.mode,
            service_account: d.service_account,
            impersonated,
            may_impersonate: checked.as_ref().map(Result::is_ok),
            error: checked.and_then(Result::err),
        }
    }
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
/// was asked, and saying so is more useful than an empty account. Given an estate,
/// `--offline` answers without any ADC file, and the note says there is none.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct WhoamiReport {
    pub adc: Credential,
    /// Absent when no estate is given — then the ADC identity is the answer.
    pub estate: Option<EstateIdentity>,
    pub quota_project: Option<QuotaProject>,
    /// true when the answer came from the file alone, without minting a token
    pub offline: bool,
    pub note: Option<String>,
    /// Given an estate, online: the permissions its resource types need, tested
    /// with the credential its live commands run as.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<crate::prerequisites::PermissionCheck>,
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
///
/// `estate` is what the estate given declares; the identity the calls actually run
/// as is the one bound for them, and the two must agree — a report that explained
/// one binding with another estate's declaration would answer the wrong question.
pub(crate) async fn whoami_report(
    offline: bool,
    estate: Option<EstateDeclaration>,
    probe: Option<crate::prerequisites::Probe>,
) -> Result<WhoamiReport, Box<dyn std::error::Error>> {
    let file = crate::org_policy::adc_file_path().map(|p| p.display().to_string());
    let kind_name = |k: &CredKind| match k {
        CredKind::UserAdc => "user-adc",
        CredKind::ImpersonatedSa => "impersonated-sa",
        CredKind::SaKey => "sa-key",
        CredKind::Unknown => "unknown",
    };
    let bound = crate::gcp::impersonation_target();
    let declared = match &estate {
        Some(d) if !crate::gcp::impersonation_disabled() => d.impersonation_target(),
        _ => None,
    };
    if bound.as_deref() != declared {
        let now = match (&estate, declared) {
            (Some(d), Some(sa)) => format!("{} declares {} — it changed after it was bound; open it again", d.path, sa),
            (Some(d), None) => format!("{} impersonates nothing — it changed after it was bound; open it again", d.path),
            (None, _) => "no estate is given".to_string(),
        };
        return Err(format!(
            "live calls are bound to run as {}, but {}",
            bound.as_deref().unwrap_or("the credentials themselves"),
            now
        )
        .into());
    }
    let quota = crate::org_policy::resolve_quota_project();

    if offline {
        // The file alone: no token, so neither check can run, and saying "not
        // checked" is the honest answer rather than an optimistic one.
        // With an estate given there is still an answer without a credential —
        // its mode and WHICH account it declares are read off the estate, not off
        // the ADC. Only when neither exists is there nothing to report.
        const NO_ADC: &str =
            "no Application Default Credentials file found — run `gcloud auth application-default login`";
        let found = credential_info_offline();
        if found.is_none() && estate.is_none() {
            return Err(NO_ADC.into());
        }
        let note = match &found {
            None => Some(NO_ADC.to_string()),
            Some(i) if i.email.is_none() && i.kind == CredKind::UserAdc => {
                Some("a user ADC file stores no identity — run without --offline to resolve it".to_string())
            }
            Some(_) => None,
        };
        let info = found.unwrap_or(CredentialInfo {
            email: None,
            kind: CredKind::Unknown,
            quota_project: None,
        });
        return Ok(WhoamiReport {
            adc: Credential { account: info.email, kind: kind_name(&info.kind), file },
            estate: estate.map(|d| EstateIdentity::new(d, bound.is_some(), None)),
            quota_project: quota.map(|id| QuotaProject { id, reachable: None, error: None }),
            offline: true,
            note,
            permissions: None,
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

    let checked = match &bound {
        None => None,
        Some(sa) => Some(crate::gcp::may_impersonate(&token, sa).await),
    };
    let estate = estate.map(|d| EstateIdentity::new(d, bound.is_some(), checked));

    // The estate's permissions last: they are tested as the identity its live
    // commands run as, which the two checks above just proved it can become.
    let permissions = match probe {
        Some(p) => Some(crate::prerequisites::test_live(&p).await),
        None => None,
    };

    Ok(WhoamiReport {
        adc: Credential { account: info.email, kind: kind_name(&info.kind), file },
        estate,
        quota_project,
        offline: false,
        note: None,
        permissions,
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
pub(crate) async fn whoami(
    offline: bool,
    estate: Option<EstateDeclaration>,
    probe: Option<crate::prerequisites::Probe>,
) -> Result<(), Box<dyn std::error::Error>> {
    let r = whoami_report(offline, estate, probe).await?;
    println!("{}", render_whoami(&r));
    // A credential that cannot become the estate's account, or a quota project
    // nothing can reach, makes every later command fail. `whoami` is the command
    // an operator runs to find that out, so it says so in its exit code too.
    let broken = r.estate.as_ref().is_some_and(|e| e.may_impersonate == Some(false))
        || r.quota_project.as_ref().is_some_and(|q| q.reachable == Some(false))
        || r.permissions.as_ref().is_some_and(|p| !p.missing.is_empty());
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

/// Who the calls run as, and the relation to the credential behind them: the ADC
/// account itself, or the service account it impersonates. Each case names what
/// decides it — no estate given, the estate's mode, `--no-impersonate` — so the
/// line says which one applies and what changes it.
fn runs_as(r: &WhoamiReport) -> String {
    let you = r.adc.account.as_deref().unwrap_or("the ADC identity");
    let Some(e) = &r.estate else {
        return format!("{} — no estate given; name one to see what it runs as", you);
    };
    match (&e.service_account, e.impersonated) {
        (Some(sa), true) => {
            let check = match (e.may_impersonate, &e.error) {
                (Some(true), _) => ", checked: allowed".to_string(),
                (Some(false), Some(why)) => format!("\n             CANNOT IMPERSONATE: {}", brief(why)),
                (Some(false), None) => ", checked: CANNOT IMPERSONATE".to_string(),
                (None, _) => ", not checked (--offline)".to_string(),
            };
            format!("{} — impersonated by {}{}", sa, you, check)
        }
        (Some(sa), false) if e.deployment_mode == "cloud" => {
            format!("{} — impersonation off (--no-impersonate); without it every run impersonates {}", you, sa)
        }
        (Some(sa), false) => format!(
            "{} — {} mode; `satz migrate {} --mode cloud` makes every run impersonate {}",
            you, e.deployment_mode, e.path, sa
        ),
        (None, _) => format!(
            "{} — {} declares no IaC service account (svc_iac_account, infra_project_name), so \
             nothing is impersonated",
            you, e.path
        ),
    }
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
    out.push_str(&format!("\nruns as:     {}", runs_as(r)));
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
    if let Some(p) = &r.permissions {
        if p.missing.is_empty() {
            out.push_str(&format!("\npermissions: {} tested for this estate's resource types — all held", p.tested));
        } else {
            out.push_str(&format!("\npermissions: {} of {} tested are MISSING:", p.missing.len(), p.tested));
            for line in crate::prerequisites::describe(&crate::prerequisites::cover(&p.missing)) {
                out.push_str(&format!("\n  {}", line));
            }
        }
        for n in &p.not_tested {
            out.push_str(&format!("\n  not tested: {}", brief(n)));
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
    /// The organization's display name — its primary domain, which is the
    /// customer's domain even when the identity's differs.
    pub(crate) org_display_name: Option<String>,
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
/// `org_hint` names the organization the caller already knows (an import's
/// sweep root): among many visible ones, that one is taken.
pub(crate) async fn live_defaults(
    need_org: bool,
    need_billing: bool,
    org_hint: Option<&str>,
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
    let (org_id, customer_id, org_display_name) = if !need_org {
        // Both values arrived as flags — no search, no ambiguity to trip on.
        (None, None, None)
    } else {
        let orgs = crate::gcp::resourcemanager::search_organizations(&client, &token)
            .await
            .map_err(|e| format!("could not search organizations: {}", e))?;
        let facts = |one: &serde_json::Value| {
            (
                one.get("name")
                    .and_then(|n| n.as_str())
                    .and_then(|n| n.strip_prefix("organizations/"))
                    .map(str::to_string),
                one.get("directoryCustomerId").and_then(|c| c.as_str()).map(str::to_string),
                one.get("displayName").and_then(|d| d.as_str()).map(str::to_string),
            )
        };
        let hinted = org_hint.and_then(|h| {
            let h = h.trim_start_matches("organizations/");
            orgs.iter().find(|o| o.get("name").and_then(|n| n.as_str()) == Some(&format!("organizations/{}", h)))
        });
        match (hinted, orgs.as_slice()) {
            (Some(one), _) => facts(one),
            (None, []) => (None, None, None),
            (None, [one]) => facts(one),
            (None, many) => {
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
        org_display_name,
        customer_id,
        billing_account,
    })
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
    use super::{
        Credential, EstateDeclaration, EstateIdentity, QuotaProject, WhoamiReport, brief, project_of,
        render_whoami,
    };

    const SA: &str = "svc-iac-001@acme-infra-001.iam.gserviceaccount.com";

    /// An estate as the resolver reports it: named `e.satz`, declaring `SA`.
    fn estate(mode: &str, impersonated: bool, checked: Option<Result<(), String>>) -> EstateIdentity {
        let d = EstateDeclaration {
            path: "e.satz".into(),
            mode: mode.into(),
            service_account: Some(SA.into()),
        };
        EstateIdentity::new(d, impersonated, checked)
    }

    /// The one line the three cases differ in.
    fn runs_as_line(r: &WhoamiReport) -> String {
        let out = render_whoami(r);
        let at = out.find("\nruns as:").unwrap_or_else(|| panic!("no runs-as line:\n{out}"));
        let rest = &out[at + 1..];
        // the line, plus a continuation line indented under it
        let end = rest.find("\nquota project:").unwrap_or(rest.len());
        rest[..end].to_string()
    }

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
            permissions: None,
        }
    }

    #[test]
    fn the_estates_permissions_are_named_with_the_role_that_carries_them() {
        use crate::prerequisites::{Need, PermissionCheck, Scope};
        let mut r = report();
        r.permissions = Some(PermissionCheck {
            tested: 3,
            missing: vec![Need {
                reason: vec!["google_cloudbuild_trigger".into()],
                permission: Some("cloudbuild.builds.create".into()),
                roles: vec!["roles/cloudbuild.builds.editor".into()],
                scope: Scope::Project,
            }],
            not_tested: vec!["Groups Admin (Google Workspace admin console) — not an IAM role".into()],
        });
        let s = render_whoami(&r);
        assert!(s.contains("permissions: 1 of 3 tested are MISSING:"), "{s}");
        assert!(s.contains("roles/cloudbuild.builds.editor at the organization — for google_cloudbuild_trigger"), "{s}");
        assert!(s.contains("not tested: Groups Admin"), "{s}");
        r.permissions = Some(PermissionCheck { tested: 18, ..Default::default() });
        assert!(render_whoami(&r).contains("permissions: 18 tested for this estate's resource types — all held"));
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

    /// Three cases, three lines: each says which case applies and what changes it.
    /// One line for "no estate" and "a local-mode estate" answered neither — the
    /// operator who had just bootstrapped expected to impersonate the account the
    /// Groups Admin check had named, and the line did not say why he did not.
    #[test]
    fn with_no_estate_the_credential_runs_as_itself_and_the_line_says_why() {
        assert_eq!(
            runs_as_line(&report()),
            "runs as:     person@example.com — no estate given; name one to see what it runs as"
        );
    }

    #[test]
    fn a_local_mode_estate_names_the_account_and_the_migration_that_makes_it_impersonate() {
        let mut r = report();
        r.estate = Some(estate("local", false, None));
        assert_eq!(
            runs_as_line(&r),
            "runs as:     person@example.com — local mode; `satz migrate e.satz --mode cloud` makes \
             every run impersonate svc-iac-001@acme-infra-001.iam.gserviceaccount.com"
        );
    }

    /// The relation on one line: the account the calls run as, and who becomes it.
    #[test]
    fn a_cloud_mode_estate_runs_as_its_account_impersonated_by_the_credential() {
        let mut r = report();
        r.estate = Some(estate("cloud", true, Some(Ok(()))));
        assert_eq!(
            runs_as_line(&r),
            "runs as:     svc-iac-001@acme-infra-001.iam.gserviceaccount.com — impersonated by \
             person@example.com, checked: allowed"
        );
    }

    /// `--no-impersonate` is the one reason a cloud-mode estate runs as the
    /// credentials; the line names it, not "local mode", which would be false.
    #[test]
    fn no_impersonate_on_a_cloud_mode_estate_names_the_switch() {
        let mut r = report();
        r.estate = Some(estate("cloud", false, None));
        assert_eq!(
            runs_as_line(&r),
            "runs as:     person@example.com — impersonation off (--no-impersonate); without it every \
             run impersonates svc-iac-001@acme-infra-001.iam.gserviceaccount.com"
        );
    }

    #[test]
    fn an_estate_that_declares_no_account_says_so_instead_of_naming_one() {
        let mut r = report();
        let d = EstateDeclaration { path: "e.satz".into(), mode: "local".into(), service_account: None };
        r.estate = Some(EstateIdentity::new(d, false, None));
        let line = runs_as_line(&r);
        assert!(line.starts_with("runs as:     person@example.com — e.satz declares no IaC service account"), "{line}");
        assert!(!line.contains("migrate"), "{line}");
    }

    #[test]
    fn a_credential_that_cannot_become_the_estate_says_so_loudly() {
        let mut r = report();
        r.estate = Some(estate("cloud", true, Some(Err("403 denied".into()))));
        let out = render_whoami(&r);
        assert!(out.contains("CANNOT IMPERSONATE"), "{}", out);
        assert!(out.contains("svc-iac-001@acme-infra-001"), "{}", out);
        assert!(out.contains("403 denied"), "{}", out);
        assert_eq!(
            runs_as_line(&r),
            "runs as:     svc-iac-001@acme-infra-001.iam.gserviceaccount.com — impersonated by \
             person@example.com\n             CANNOT IMPERSONATE: 403 denied"
        );
    }

    /// Mode and account travel as data too: `satz_whoami` returns this report, so
    /// an agent reads the same three cases the terminal prints.
    #[test]
    fn the_report_carries_the_mode_and_the_declared_account_in_every_case() {
        let mut r = report();
        r.estate = Some(estate("local", false, None));
        let v = serde_json::to_value(&r).expect("serialises");
        assert_eq!(v["estate"]["deployment_mode"], "local", "{v}");
        assert_eq!(v["estate"]["service_account"], SA, "{v}");
        assert_eq!(v["estate"]["impersonated"], false, "{v}");
        assert_eq!(v["estate"]["path"], "e.satz", "{v}");
        assert!(v["estate"]["may_impersonate"].is_null(), "{v}");
    }

    /// The report explains the binding with the estate's declaration, so the two
    /// must agree. They part when the estate file changes after it was bound — an
    /// estate `satz_open`ed and then migrated — and a report that explained one
    /// with the other would name an identity no call runs as.
    #[tokio::test]
    async fn a_binding_the_declaration_does_not_explain_is_refused() {
        let decl = |mode: &str| EstateDeclaration {
            path: "e.satz".into(),
            mode: mode.into(),
            service_account: Some(SA.into()),
        };
        let other = "svc-iac-001@bolt-infra-001.iam.gserviceaccount.com".to_string();
        for (bound, declared, says) in [
            (Some(other), decl("cloud"), "e.satz declares svc-iac-001@acme-infra-001"),
            (Some(SA.to_string()), decl("local"), "e.satz impersonates nothing"),
            (None, decl("cloud"), "the credentials themselves"),
        ] {
            let err = crate::gcp::with_identity(bound, super::whoami_report(true, Some(declared), None))
                .await
                .expect_err("a disagreeing binding must be refused");
            let err = err.to_string();
            assert!(err.contains(says), "{err}");
            assert!(err.contains("open it again"), "{err}");
        }
    }

    /// The declaration reads the params the way the emitter does: no mode is
    /// local, and only cloud mode with both halves of the account impersonates.
    #[test]
    fn the_declaration_reads_mode_and_account_as_the_emitter_does() {
        let read = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> =
                pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            EstateDeclaration::from_params("e.satz".into(), |k| {
                pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
            })
        };
        let both = [("svc-iac-account", "svc-iac-001"), ("infra-project-name", "acme-infra-001")];

        let none = read(&both);
        assert_eq!(none.mode, "local");
        assert_eq!(none.service_account.as_deref(), Some(SA));
        assert_eq!(none.impersonation_target(), None);

        let cloud = read(&[both[0], both[1], ("deployment-mode", "cloud")]);
        assert_eq!(cloud.impersonation_target(), Some(SA));

        let half = read(&[both[0], ("infra-project-name", ""), ("deployment-mode", "cloud")]);
        assert_eq!(half.service_account, None);
        assert_eq!(half.impersonation_target(), None);
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
        r.estate = Some(estate("cloud", true, None));
        r.quota_project =
            Some(QuotaProject { id: "acme-infra-001".into(), reachable: None, error: None });
        let out = render_whoami(&r);
        assert_eq!(out.matches("not checked (--offline)").count(), 2, "{}", out);
        assert!(!out.contains("CANNOT"), "{}", out);
        assert!(!out.contains("UNREACHABLE"), "{}", out);
        assert_eq!(
            runs_as_line(&r),
            "runs as:     svc-iac-001@acme-infra-001.iam.gserviceaccount.com — impersonated by \
             person@example.com, not checked (--offline)"
        );
    }

    /// Offline, a user ADC file names nobody — the three cases still read apart,
    /// because what tells them apart is the estate, not the credential.
    #[test]
    fn offline_with_an_anonymous_user_adc_the_three_cases_still_differ() {
        let mut r = report();
        r.offline = true;
        r.adc.account = None;
        assert_eq!(
            runs_as_line(&r),
            "runs as:     the ADC identity — no estate given; name one to see what it runs as"
        );
        r.estate = Some(estate("local", false, None));
        assert_eq!(
            runs_as_line(&r),
            "runs as:     the ADC identity — local mode; `satz migrate e.satz --mode cloud` makes \
             every run impersonate svc-iac-001@acme-infra-001.iam.gserviceaccount.com"
        );
        r.estate = Some(estate("cloud", true, None));
        assert_eq!(
            runs_as_line(&r),
            "runs as:     svc-iac-001@acme-infra-001.iam.gserviceaccount.com — impersonated by the \
             ADC identity, not checked (--offline)"
        );
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
