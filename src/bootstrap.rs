use std::path::{Path, PathBuf};
use serde_yaml::Value;
use google_cloud_auth::credentials::Builder;

/// What happened to one bootstrap step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatus {
    Created,
    AlreadyExisted,
    Skipped(String),
    /// Carries the API's own message, verbatim.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StepOutcome {
    label: String,
    status: StepStatus,
}

/// Every bootstrap action is idempotent and re-run on each invocation, so a failure is
/// recorded and the run continues rather than aborting — otherwise fixing one cause only
/// reveals the next. The exception is a permission denial, which every later step would
/// hit too.
#[derive(Debug, Default)]
struct RunSummary {
    steps: Vec<StepOutcome>,
}

impl RunSummary {
    /// Record a step, echoing it as it happens so a long run still streams progress.
    fn record(&mut self, label: impl Into<String>, status: StepStatus) {
        let outcome = StepOutcome { label: label.into(), status };
        match &outcome.status {
            StepStatus::Created => println!("  created  {}", outcome.label),
            StepStatus::AlreadyExisted => println!("  exists   {}", outcome.label),
            StepStatus::Skipped(why) => eprintln!("  skipped  {}: {}", outcome.label, why),
            StepStatus::Failed(err) => eprintln!("  FAILED   {}: {}", outcome.label, err),
        }
        self.steps.push(outcome);
    }

    fn failed(&self) -> usize {
        self.steps.iter().filter(|s| matches!(s.status, StepStatus::Failed(_))).count()
    }

    /// True once a step has failed for lack of permission. Later steps need the same or
    /// broader access, so continuing would only repeat the same denial.
    fn has_permission_failure(&self) -> bool {
        self.steps.iter().any(|s| match &s.status {
            StepStatus::Failed(err) => is_permission_error(err),
            _ => false,
        })
    }
}

/// Recognise a permission denial in an API error body.
fn is_permission_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("permission_denied")
        || e.contains("permission denied")
        || e.contains("forbidden")
        || e.contains("\"code\": 403")
        || e.contains("\"code\":403")
        || e.contains("does not have")
}

/// Render the end-of-run report. Pure, so its shape is pinned by tests.
fn render_summary(steps: &[StepOutcome]) -> String {
    let mut out = String::from("\n=== Bootstrap summary ===\n");

    let applied: Vec<&StepOutcome> = steps
        .iter()
        .filter(|s| matches!(s.status, StepStatus::Created | StepStatus::AlreadyExisted))
        .collect();
    if !applied.is_empty() {
        out.push_str("Applied:\n");
        for s in applied {
            let verb = if s.status == StepStatus::Created { "created" } else { "exists " };
            out.push_str(&format!("  {} {}\n", verb, s.label));
        }
    }

    let skipped: Vec<(&str, &str)> = steps
        .iter()
        .filter_map(|s| match &s.status {
            StepStatus::Skipped(why) => Some((s.label.as_str(), why.as_str())),
            _ => None,
        })
        .collect();
    if !skipped.is_empty() {
        out.push_str("Skipped:\n");
        for (label, why) in skipped {
            out.push_str(&format!("  {}: {}\n", label, why));
        }
    }

    let errors: Vec<(&str, &str)> = steps
        .iter()
        .filter_map(|s| match &s.status {
            StepStatus::Failed(err) => Some((s.label.as_str(), err.as_str())),
            _ => None,
        })
        .collect();
    if !errors.is_empty() {
        out.push_str("Errors:\n");
        for (label, err) in &errors {
            out.push_str(&format!("  {}: {}\n", label, err));
        }
        out.push_str(&format!(
            "{} step(s) failed. Bootstrap is idempotent — fix the causes and re-run.\n",
            errors.len()
        ));
    } else {
        out.push_str("All steps completed.\n");
    }

    out
}

/// Read the linked billing account out of a cloudbilling `billingInfo` response, e.g.
/// `billingAccounts/012345-6789AB-CDEF01`. Absent means the project has no billing link.
fn billing_account_from_json(v: &serde_json::Value) -> Option<String> {
    v.get("billingAccountName")
        .and_then(|b| b.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// True when a serviceusage service resource reports itself as already enabled.
fn service_is_enabled(v: &serde_json::Value) -> bool {
    v.get("state").and_then(|s| s.as_str()) == Some("ENABLED")
}

/// Poll a long-running operation until it reports `done`, or the deadline passes.
/// Without a deadline a stuck operation loops forever; without inspecting the terminal
/// object, a failed operation reads as a success.
async fn await_operation(
    client: &reqwest::Client,
    token: &str,
    op_name: &str,
    interval: std::time::Duration,
    max_polls: u32,
) -> Result<serde_json::Value, String> {
    for _ in 0..max_polls {
        tokio::time::sleep(interval).await;
        let res = client
            .get(format!("https://cloudresourcemanager.googleapis.com/v3/{}", op_name))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let op: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        if op.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(op);
        }
    }
    Err(format!("operation '{}' did not complete within the timeout", op_name))
}

/// How the ADC identity was established, so an unexpected result can be traced back to
/// the mechanism that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrincipalSource {
    TokenInfo,
    Signer,
    UserInfo,
    AdcFile,
}

impl PrincipalSource {
    fn label(self) -> &'static str {
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

/// Read the service-account address out of an ADC credentials file: either a key file's
/// `client_email`, or the impersonation target in
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

/// True when the credentials in use are the ones the config names. Addresses are
/// compared case-insensitively; the local part of an email is technically
/// case-sensitive, but no identity provider in this path treats it that way, and a
/// spurious mismatch would block a legitimate run.
fn identity_matches(expected: &str, actual: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(actual.trim())
}

/// Determine which principal the Application Default Credentials represent.
///
/// Tried in cost order; `None` means no mechanism could tell us, which the caller
/// treats as "cannot verify" rather than "verified".
async fn resolve_adc_identity(
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
    let path = crate::org_policy::adc_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    email_from_adc_json(&json).map(|email| (email, PrincipalSource::AdcFile))
}

/// Read a bootstrap config the same way `transpile` does: expand `!include` directives,
/// then resolve the `!join` / `!format` custom tags.
///
/// Reading it any other way is what made bootstrap fail on configs that include the
/// shipped presets: those reference anchors defined in the parent file, so an
/// un-expanded parse dies on an undefined alias, and their `!format` members stay as
/// tagged values that `as_str()` refuses to read.
///
/// Returns `(vars_view, resolved)`. `merge_variables` strips `variables:` blocks and
/// promotes them to the root, so `vars_view` is kept as the pre-merge view for lookups
/// that address `variables:` directly; `resolved` is the merged document.
fn load_bootstrap_yaml(
    config_file: &Path,
    include_dirs: &[String],
) -> Result<(Value, Value), Box<dyn std::error::Error>> {
    // A satz estate reads as satz. Bootstrap only ever addresses the variable
    // table — every `lookup` below names a variable, never a document position —
    // so the parameter table synthesized into the same two shapes serves it
    // exactly, without compiling `.gen.yaml` siblings into a repo that, for the
    // first command a new customer runs, does not have a `.gitignore` yet.
    if config_file.extension().and_then(|e| e.to_str()) == Some("satz") {
        let params = crate::satz_estate_params(config_file, include_dirs)?;
        let mut flat = serde_yaml::Mapping::new();
        for (k, v) in params {
            flat.insert(Value::String(k), v);
        }
        let mut with_block = serde_yaml::Mapping::new();
        with_block.insert(Value::String("variables".into()), Value::Mapping(flat.clone()));
        return Ok((Value::Mapping(with_block), Value::Mapping(flat)));
    }
    let include_paths: Vec<PathBuf> = include_dirs.iter().map(PathBuf::from).collect();
    let (processed, _ops) =
        crate::include_processor::process_includes_with_ops(config_file, &include_paths)?;

    let raw: Value = serde_yaml::from_str(&processed).inspect_err(|e| {
        crate::print_yaml_error_context(&processed, e);
    })?;

    let vars_view = crate::resolve_yaml_custom_tags(raw.clone());
    let resolved = crate::resolve_yaml_custom_tags(crate::merge_variables(raw));
    Ok((vars_view, resolved))
}

pub async fn bootstrap(
    config_file: PathBuf,
    dry_run: bool,
    runtime_config: crate::ToolConfig,
    cli_config: Option<PathBuf>,
    cli_validation: Option<String>,
    cli_verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Loading configuration from {}...", config_file.display());
    let (vars_view, resolved) = load_bootstrap_yaml(&config_file, &runtime_config.include_dirs)?;

    // A value may live in the config's own `variables:` block, or — once includes are
    // expanded and merged — at the root of the merged document. Check both, trying each
    // accepted spelling in turn.
    let lookup = |names: &[&str]| -> Option<Value> {
        let block = vars_view.get("variables").and_then(|v| v.as_mapping());
        for name in names {
            if let Some(v) = block.and_then(|m| m.get(Value::String((*name).to_string()))) {
                return Some(v.clone());
            }
            if let Some(v) = resolved.get(*name) {
                return Some(v.clone());
            }
        }
        None
    };
    let lookup_str = |names: &[&str]| -> Option<String> {
        lookup(names).and_then(|v| match v {
            // Numeric ids (org, billing) are routinely written unquoted in YAML.
            Value::String(s) => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };

    let sn = lookup_str(&["customer-shortname", "shortname"])
        .ok_or_else(|| format!("Missing 'customer-shortname' in {}", config_file.display()))?;
    let bid = lookup_str(&["billing-account-infra", "billing_id"])
        .ok_or_else(|| format!("Missing 'billing-account-infra' in {}", config_file.display()))?;
    let r = lookup_str(&["default-region", "region"]).unwrap_or_else(|| "europe-west3".to_string());
    let oid_val = lookup_str(&["customer-organization-id"])
        .ok_or("Missing org_id in configuration (required for bootstrap)")?;
    let final_proj_id = lookup_str(&["infra-project-name"]);
    let final_bucket = lookup_str(&["infra-bucket-name"]);

    let parent = crate::org_policy::normalize_parent(&oid_val);

    let project_id = final_proj_id.unwrap_or_else(|| format!("{}-iac-infra", sn));
    let bucket_name = final_bucket.unwrap_or_else(|| project_id.clone());
    let sa_name = "svc-iac-001";

    println!("--- Bootstrap Plan ---");
    println!("Parent:          {}", parent);
    println!("Shortname:       {}", sn);
    println!("Billing ID:      {}", bid);
    println!("Region:          {}", r);
    println!("Project ID:      {}", project_id);
    println!("Bucket:          {}", bucket_name);
    println!("Service Account: {}.iam.gserviceaccount.com", sa_name);
    println!("----------------------");

    if dry_run {
        println!("Dry run enabled. No resources will be created.");
        return Ok(());
    }

    println!("Starting bootstrap process...");
    let mut summary = RunSummary::default();

    // 1. Get Authentication Token
    println!("Authenticating using Application Default Credentials...");
    let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
    let credentials = Builder::default()
        .with_scopes(scopes)
        .build_access_token_credentials()?;
    let token = credentials.access_token().await?;

    let client = reqwest::Client::new();

    // 1.5 Verify we are running as the admin this config expects.
    // Every call below is made as the ADC principal, so running as anyone else either
    // fails on a missing permission or, worse, succeeds against the wrong identity.
    // Check before touching anything.
    let expected_admin = match (lookup_str(&["first-admin"]), lookup_str(&["customer-domain"])) {
        (Some(local), Some(domain)) if !local.is_empty() && !domain.is_empty() => {
            // `first-admin` is normally the local part, but accept a full address too.
            Some(if local.contains('@') { local } else { format!("{}@{}", local, domain) })
        }
        _ => None,
    };

    match &expected_admin {
        Some(expected) => {
            // These messages are printed rather than carried in the error, because `main`
            // renders a returned error with `Debug`, which would escape the newlines.
            let resolved = resolve_adc_identity(&client, &token.token).await;
            let Some((actual, source)) = resolved else {
                eprintln!(
                    "\nCould not determine the identity of your Application Default Credentials,\n\
                     so it cannot be checked against the configured admin '{expected}'.\n\n\
                     Authenticate with:  gcloud auth application-default login {expected}\n"
                );
                return Err("could not verify the Application Default Credentials identity".into());
            };

            if !identity_matches(expected, &actual) {
                eprintln!(
                    "\nApplication Default Credentials belong to '{actual}' (determined via {}),\n\
                     but this config expects '{expected}'.\n\
                     Bootstrap performs every action as the ADC identity, so it will not continue.\n\n\
                     Authenticate with:  gcloud auth application-default login {expected}\n",
                    source.label()
                );
                return Err(format!(
                    "ADC identity '{actual}' does not match the configured admin '{expected}'"
                )
                .into());
            }

            println!("Running as {} (via {}), matching the configured admin.", actual, source.label());
        }
        None => {
            eprintln!(
                "Warning: this config defines no 'first-admin' / 'customer-domain', so the identity \
                 of your Application Default Credentials cannot be verified. Continuing."
            );
        }
    }

    // 2. Create Folder (if specified)
    let infra_folder_name = lookup_str(&["infra-folder-name"]).filter(|s| !s.is_empty());
    let infra_folder_name = infra_folder_name.as_deref();

    let mut current_parent = parent.clone();

    if let Some(folder_display_name) = infra_folder_name {
        let step = format!("Folder \"{}\"", folder_display_name);
        println!("Checking for existing Infrastructure Folder: {}...", folder_display_name);

        // 2a. Search for folder by display name in the parent — every page,
        // and exactly one match. A failed search must not fall through to
        // creation: that would try to create a folder that may already exist,
        // and the real cause (usually a denied list call) would never be reported.
        let lookup = match crate::gcp::resourcemanager::list_folders(&client, &token.token, &parent).await {
            Ok(folders) => crate::gcp::resourcemanager::find_folder_by_display_name(&folders, folder_display_name),
            Err(e) => Err(format!("could not list folders under {}: {}", parent, e)),
        };

        if let Err(err) = lookup {
            summary.record(&step, StepStatus::Failed(err));
        } else if let Ok(Some(folder_id)) = lookup {
            current_parent = folder_id.clone();
            summary.record(&step, StepStatus::AlreadyExisted);
        } else {
            // 2b. Not found, proceed with creation
            println!("Creating Infrastructure Folder: {}...", folder_display_name);
            let url = "https://cloudresourcemanager.googleapis.com/v3/folders";
            let body = serde_json::json!({
                "displayName": folder_display_name,
                "parent": parent
            });

            let res = client.post(url)
                .bearer_auth(&token.token)
                .json(&body)
                .send()
                .await?;

            if res.status().is_success() {
                let info: serde_json::Value = res.json().await?;
                match info.get("name").and_then(|v| v.as_str()) {
                    Some(op_name) => {
                        println!("Folder creation in progress ({})...", op_name);
                        match await_operation(&client, &token.token, op_name, std::time::Duration::from_secs(2), 60).await {
                            Ok(op) => {
                                if let Some(err) = op.get("error") {
                                    summary.record(&step, StepStatus::Failed(err.to_string()));
                                } else if let Some(name) = op.get("response").and_then(|r| r.get("name")).and_then(|v| v.as_str()) {
                                    current_parent = name.to_string();
                                    summary.record(&step, StepStatus::Created);
                                } else {
                                    summary.record(&step, StepStatus::Failed(
                                        "creation finished with neither a response nor an error".to_string(),
                                    ));
                                }
                            }
                            Err(e) => summary.record(&step, StepStatus::Failed(e)),
                        }
                    }
                    None => summary.record(&step, StepStatus::Failed(
                        "the API accepted the request but returned no operation name".to_string(),
                    )),
                }
            } else {
                let err = res.text().await?;
                summary.record(&step, StepStatus::Failed(err));
            }
        }
    }

    // Later steps all target resources inside this folder, so a permission problem here
    // would just repeat itself with a less informative message.
    if summary.has_permission_failure() {
        print!("{}", render_summary(&summary.steps));
        return Err("bootstrap stopped: the ADC identity lacks a required permission".into());
    }

    // 3. Create Project Shell
    println!("Creating Project: {}...", project_id);
    let url = "https://cloudresourcemanager.googleapis.com/v3/projects";
    let body = serde_json::json!({
        "projectId": project_id,
        "displayName": project_id,
        "parent": current_parent
    });

    let res = client.post(url)
        .bearer_auth(&token.token)
        .json(&body)
        .send()
        .await?;

    let project_step = format!("Project {}", project_id);
    // Billing, API enablement and the bucket all live inside this project, so track
    // whether it exists explicitly rather than inferring it from the last recorded step.
    let mut project_usable = true;
    // The project NUMBER: the create operation's response carries it, and it
    // is the id every folder-scoped or project-scoped adoption needs. It used
    // to be discarded here — the only place the tool ever had it.
    let mut project_number: Option<String> = None;
    if res.status().is_success() {
        let info: serde_json::Value = res.json().await?;
        match info.get("name").and_then(|v| v.as_str()) {
            Some(op_name) => {
                println!("Project creation in progress ({})...", op_name);
                match await_operation(&client, &token.token, op_name, std::time::Duration::from_secs(3), 60).await {
                    Ok(op) => {
                        // The operation's `error` field was previously never inspected, so a
                        // failed creation still reported "Project shell created."
                        if let Some(err) = op.get("error") {
                            project_usable = false;
                            summary.record(&project_step, StepStatus::Failed(err.to_string()));
                        } else {
                            project_number = op
                                .get("response")
                                .and_then(|r| r.get("name"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            summary.record(&project_step, StepStatus::Created);
                        }
                    }
                    Err(e) => {
                        project_usable = false;
                        summary.record(&project_step, StepStatus::Failed(e));
                    }
                }
            }
            None => {
                project_usable = false;
                summary.record(&project_step, StepStatus::Failed(
                    "the API accepted the request but returned no operation name".to_string(),
                ));
            }
        }
    } else if res.status().as_u16() == 409 {
        summary.record(&project_step, StepStatus::AlreadyExisted);
        match crate::gcp::resourcemanager::get_project_number(&client, &token.token, &project_id).await {
            Ok(n) => project_number = n,
            Err(e) => eprintln!("warning: could not read project {}: {}", project_id, e),
        }
    } else {
        let err = res.text().await?;
        project_usable = false;
        summary.record(&project_step, StepStatus::Failed(err));
    }
    if let Some(n) = &project_number {
        println!("Project number: {}", n.trim_start_matches("projects/"));
    }

    if summary.has_permission_failure() {
        print!("{}", render_summary(&summary.steps));
        return Err("bootstrap stopped: the ADC identity lacks a required permission".into());
    }

    // 4. Link Billing Account
    let billing_step = format!("Billing link {} -> {}", project_id, bid);
    let desired_billing = format!("billingAccounts/{}", bid);
    if !project_usable {
        summary.record(&billing_step, StepStatus::Skipped(format!("{} was not created", project_id)));
    } else {
        let url = format!("https://cloudbilling.googleapis.com/v1/projects/{}/billingInfo", project_id);

        // `PUT billingInfo` is an upsert that returns 200 whether or not anything changed,
        // so read first — otherwise every re-run reports the link as newly created.
        let current = match client.get(&url).bearer_auth(&token.token).send().await {
            Ok(res) if res.status().is_success() => res
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| billing_account_from_json(&v)),
            // An unreadable current state is not evidence of anything; fall through to
            // the write rather than reporting a state we did not confirm.
            _ => None,
        };

        if current.as_deref() == Some(desired_billing.as_str()) {
            summary.record(&billing_step, StepStatus::AlreadyExisted);
        } else {
            println!("Linking Billing Account: {}...", bid);
            let res = client.put(&url)
                .bearer_auth(&token.token)
                .json(&serde_json::json!({ "billingAccountName": desired_billing }))
                .send()
                .await?;

            if res.status().is_success() {
                summary.record(&billing_step, StepStatus::Created);
            } else {
                let err = res.text().await?;
                summary.record(&billing_step, StepStatus::Failed(err));
            }
        }
    }

    // 5. Enable Foundation APIs (The "Chicken-and-Egg" Fix)
    let core_services = vec![
        "serviceusage.googleapis.com",
        "cloudresourcemanager.googleapis.com",
        "iam.googleapis.com",
        "iamcredentials.googleapis.com",
        "storage.googleapis.com",
        "cloudbilling.googleapis.com",
        "cloudidentity.googleapis.com",
        "cloudasset.googleapis.com",
        "logging.googleapis.com",
        "orgpolicy.googleapis.com",
        "essentialcontacts.googleapis.com",
    ];

    for service in core_services {
        let step = format!("API {}", service);
        if !project_usable {
            summary.record(&step, StepStatus::Skipped(format!("{} was not created", project_id)));
            continue;
        }

        let url = format!(
            "https://serviceusage.googleapis.com/v1/projects/{}/services/{}",
            project_id, service
        );

        // `services:enable` succeeds whether or not the API was already on, so read the
        // current state first to keep the summary honest about what this run changed.
        let already_enabled = match client.get(&url).bearer_auth(&token.token).send().await {
            Ok(res) if res.status().is_success() => res
                .json::<serde_json::Value>()
                .await
                .is_ok_and(|v| service_is_enabled(&v)),
            _ => false,
        };

        if already_enabled {
            summary.record(&step, StepStatus::AlreadyExisted);
            continue;
        }

        println!("Enabling core service: {}...", service);
        let res = client.post(format!("{}:enable", url))
            .bearer_auth(&token.token)
            .json(&serde_json::json!({})) // Fix 411 Length Required (empty body)
            .send()
            .await?;

        // Collected rather than fatal, so one run reports every API that could not be
        // enabled instead of stopping at the first.
        if res.status().is_success() {
            summary.record(&step, StepStatus::Created);
        } else {
            let err_body = res.text().await?;
            summary.record(&step, StepStatus::Failed(err_body));
        }
    }

    // 6. Create GCS State Bucket
    let bucket_step = format!("State bucket {}", bucket_name);
    if !project_usable {
        summary.record(&bucket_step, StepStatus::Skipped(format!("{} was not created", project_id)));
    } else {
        println!("Creating GCS State Bucket: {}...", bucket_name);
        let url = format!("https://storage.googleapis.com/storage/v1/b?project={}", project_id);
        let body = serde_json::json!({
            "name": bucket_name,
            "location": r,
            "iamConfiguration": {
                "uniformBucketLevelAccess": {
                    "enabled": true
                }
            },
            "versioning": {
                "enabled": true
            }
        });

        let res = client.post(&url)
            .bearer_auth(&token.token)
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            summary.record(&bucket_step, StepStatus::Created);
        } else if res.status().as_u16() == 409 {
            summary.record(&bucket_step, StepStatus::AlreadyExisted);
        } else {
            let err = res.text().await?;
            summary.record(&bucket_step, StepStatus::Failed(err));
        }
    }

    // The summary is printed before the automatic setup below so that, if a step failed,
    // the cause is visible above whatever transpile/init/import go on to report.
    print!("{}", render_summary(&summary.steps));
    if summary.failed() > 0 {
        return Err(format!("bootstrap finished with {} failed step(s)", summary.failed()).into());
    }
    println!("Core Infrastructure (Folder, Project, Billing, Foundation APIs, State Bucket) is now ready.");

    // 7. Automatic setup: Transpile -> Init -> Import
    println!("Running automatic setup...");

    // 7a. Transpile
    println!("Transpiling to HCL...");
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&exe);

    // Forward global CLI flags so relative paths and validation behave consistently
    if let Some(cfg_path) = cli_config {
        cmd.arg("--config").arg(cfg_path);
    }
    if let Some(validation) = cli_validation {
        cmd.arg("--validation").arg(validation);
    }
    if cli_verbose {
        cmd.arg("--verbose");
    }

    // `config_file` is already yaml_dir-joined, and transpile would join a relative
    // positional onto yaml_dir a second time. Hand it an absolute path, which transpile
    // passes through untouched. Taking `file_name()` instead discarded every directory
    // component — defeating the absolute paths bootstrap explicitly accepts — and
    // panicked outright on a path ending in "..".
    let transpile_target =
        std::fs::canonicalize(&config_file).unwrap_or_else(|_| config_file.clone());

    let status = cmd
        .arg("transpile")
        .arg(&transpile_target)
        .current_dir(std::env::current_dir()?) // Run from the original working directory
        .status()?;

    if !status.success() {
        return Err("Transpilation failed. Cannot proceed with imports.".into());
    }

    // 7b. Init
    let target_hcl_dir = std::path::Path::new(&runtime_config.hcl_dir);
    if target_hcl_dir.exists() && target_hcl_dir.is_dir() {
        println!("Initializing OpenTofu/Terraform in {}...", target_hcl_dir.display());
        let status = std::process::Command::new(&runtime_config.tf_tool)
            .current_dir(target_hcl_dir)
            .arg("init")
            .status()?;

        if !status.success() {
             return Err(format!("{} init failed. Cannot proceed with imports.", runtime_config.tf_tool).into());
        }

        println!("Detected existing HCL directory at {}. Running automatic imports...", target_hcl_dir.display());

        // Import Folder
        if current_parent.starts_with("folders/") {
            run_import(&runtime_config.tf_tool, target_hcl_dir, "google_folder.infra_folder", &current_parent);
        }

        // Import Project
        run_import(&runtime_config.tf_tool, target_hcl_dir, "google_project.infra", &project_id);

        // Import Bucket
        run_import(&runtime_config.tf_tool, target_hcl_dir, "google_storage_bucket.state", &bucket_name);
    } else {
        println!("Warning: HCL directory not found after transpilation. Skipping imports.");
    }

    Ok(())
}

pub(crate) fn run_import(tf_tool: &str, working_dir: &std::path::Path, resource_address: &str, resource_id: &str) -> bool {
    println!("Importing {} (ID: {})...", resource_address, resource_id);
    let output = std::process::Command::new(tf_tool)
        .current_dir(working_dir)
        .arg("import")
        .arg(resource_address)
        .arg(resource_id)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                println!("- {}: Successfully imported.", resource_address);
                true
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.contains("Resource already managed by OpenTofu") {
                    println!("- {}: Already managed by OpenTofu.", resource_address);
                    true
                } else {
                    println!("- {}: Import failed or skipped. (stderr: {})", resource_address, stderr.trim());
                    false
                }
            }
        }
        Err(e) => {
            println!("- {}: Failed to execute {} import: {}", resource_address, tf_tool, e);
            false
        }
    }
}

// Tests (pure layer only — no network, no filesystem mutation).
#[cfg(test)]
mod tests {
    use super::*;

    fn jv(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("valid test JSON")
    }

    // --- ADC identity extraction -------------------------------------------------

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

    // --- read-before-write state probes ------------------------------------------

    #[test]
    fn reads_the_linked_billing_account() {
        let v = jv(r#"{"name":"projects/p/billingInfo","projectId":"p","billingAccountName":"billingAccounts/012345-6789AB-CDEF01","billingEnabled":true}"#);
        assert_eq!(
            billing_account_from_json(&v).as_deref(),
            Some("billingAccounts/012345-6789AB-CDEF01")
        );
    }

    #[test]
    fn unlinked_project_reports_no_billing_account() {
        // A project with no billing link omits the field entirely.
        assert_eq!(billing_account_from_json(&jv(r#"{"projectId":"p","billingEnabled":false}"#)), None);
        assert_eq!(billing_account_from_json(&jv(r#"{"billingAccountName":""}"#)), None);
    }

    #[test]
    fn service_state_distinguishes_enabled_from_disabled() {
        // Without this, `services:enable` returning 200 made every re-run report
        // "created" for APIs that were already on.
        assert!(service_is_enabled(&jv(r#"{"name":"projects/1/services/iam.googleapis.com","state":"ENABLED"}"#)));
        assert!(!service_is_enabled(&jv(r#"{"state":"DISABLED"}"#)));
        assert!(!service_is_enabled(&jv(r#"{"state":"STATE_UNSPECIFIED"}"#)));
        assert!(!service_is_enabled(&jv("{}")));
    }

    // --- identity comparison -----------------------------------------------------

    #[test]
    fn identity_matches_exactly_and_ignoring_case() {
        assert!(identity_matches("admin@example.com", "admin@example.com"));
        assert!(identity_matches("Admin@Example.com", "admin@example.com"));
        assert!(identity_matches(" admin@example.com ", "admin@example.com"));
    }

    #[test]
    fn identity_mismatch_is_rejected() {
        // The reported bug: bootstrap ran as one identity while the config named another.
        assert!(!identity_matches("admin@example.com", "someone.else@example.com"));
        assert!(!identity_matches("admin@example.com", "admin@example.org"));
        assert!(!identity_matches("admin@example.com", ""));
    }

    // --- permission detection ----------------------------------------------------

    #[test]
    fn recognises_permission_denials() {
        assert!(is_permission_error(
            r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"Permission denied"}}"#
        ));
        assert!(is_permission_error(
            "user first.admin@example.com does not have resourcemanager.folders.create access"
        ));
        assert!(is_permission_error("403 Forbidden"));
    }

    #[test]
    fn other_failures_are_not_permission_errors() {
        assert!(!is_permission_error(r#"{"error":{"code":409,"status":"ALREADY_EXISTS"}}"#));
        assert!(!is_permission_error("operation timed out"));
    }

    // --- summary rendering -------------------------------------------------------

    fn outcome(label: &str, status: StepStatus) -> StepOutcome {
        StepOutcome { label: label.to_string(), status }
    }

    #[test]
    fn summary_groups_outcomes_and_counts_failures() {
        let steps = vec![
            outcome("Folder \"Infrastructure\"", StepStatus::Created),
            outcome("Project acme-iac-infra", StepStatus::AlreadyExisted),
            outcome("Billing link", StepStatus::Skipped("project was not created".into())),
            outcome("API storage.googleapis.com", StepStatus::Failed("SERVICE_DISABLED".into())),
            outcome("State bucket acme", StepStatus::Failed("PERMISSION_DENIED".into())),
        ];
        let out = render_summary(&steps);

        assert!(out.contains("Applied:"), "{out}");
        assert!(out.contains("Folder \"Infrastructure\""), "{out}");
        assert!(out.contains("Project acme-iac-infra"), "{out}");
        assert!(out.contains("Skipped:"), "{out}");
        assert!(out.contains("project was not created"), "{out}");
        assert!(out.contains("Errors:"), "{out}");
        // The API's own message must survive verbatim — it is the actionable part.
        assert!(out.contains("SERVICE_DISABLED"), "{out}");
        assert!(out.contains("2 step(s) failed"), "{out}");
    }

    #[test]
    fn summary_omits_empty_sections() {
        let steps = vec![outcome("Project p", StepStatus::Created)];
        let out = render_summary(&steps);
        assert!(out.contains("Applied:"), "{out}");
        assert!(!out.contains("Errors:"), "{out}");
        assert!(!out.contains("Skipped:"), "{out}");
        assert!(out.contains("All steps completed."), "{out}");
    }

    #[test]
    fn failed_count_drives_the_exit_code() {
        let mut s = RunSummary::default();
        s.record("Folder", StepStatus::Created);
        s.record("Project", StepStatus::AlreadyExisted);
        s.record("Billing", StepStatus::Skipped("no project".into()));
        assert_eq!(s.failed(), 0, "non-failures must not make bootstrap exit non-zero");

        s.record("Bucket", StepStatus::Failed("boom".into()));
        s.record("API x", StepStatus::Failed("boom".into()));
        assert_eq!(s.failed(), 2);
    }

    #[test]
    fn permission_failure_is_detected_only_for_permission_errors() {
        let mut s = RunSummary::default();
        s.record("Project", StepStatus::Failed("ALREADY_EXISTS".into()));
        assert!(!s.has_permission_failure());

        s.record("Folder", StepStatus::Failed("PERMISSION_DENIED on folders.create".into()));
        assert!(s.has_permission_failure(), "a 403 must stop the run");
    }
}
