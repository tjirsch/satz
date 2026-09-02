use std::path::{Path, PathBuf};
use serde_yaml::Value;
use google_cloud_auth::credentials::Builder;

use crate::gcp::ErrorClass;

/// What happened to one bootstrap step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatus {
    Created,
    AlreadyExisted,
    Skipped(String),
    /// Carries the failure's classification and the API's own message, verbatim.
    Failed(ErrorClass, String),
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
            StepStatus::Failed(class, err) => {
                eprintln!("  FAILED   {}: {}", outcome.label, err);
                if *class == ErrorClass::QuotaProject {
                    eprintln!("           hint: {}", QUOTA_HINT);
                }
            }
        }
        self.steps.push(outcome);
    }

    fn failed(&self) -> usize {
        self.steps.iter().filter(|s| matches!(s.status, StepStatus::Failed(..))).count()
    }

    /// True once a step has failed for lack of permission. Later steps need the same or
    /// broader access, so continuing would only repeat the same denial.
    fn has_permission_failure(&self) -> bool {
        self.steps
            .iter()
            .any(|s| matches!(s.status, StepStatus::Failed(ErrorClass::PermissionDenied, _)))
    }
}

/// One shared hint for quota-class 403s: the failure names the billed project,
/// not the caller's permissions, and the fix is one gcloud command.
const QUOTA_HINT: &str = "this 403 is about the billed (quota) project, not a missing permission — \
                          run `gcloud auth application-default set-quota-project <project>`";

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

    let errors: Vec<(&str, ErrorClass, &str)> = steps
        .iter()
        .filter_map(|s| match &s.status {
            StepStatus::Failed(class, err) => Some((s.label.as_str(), *class, err.as_str())),
            _ => None,
        })
        .collect();
    if !errors.is_empty() {
        out.push_str("Errors:\n");
        for (label, class, err) in &errors {
            out.push_str(&format!("  {}: {}\n", label, err));
            if *class == ErrorClass::QuotaProject {
                out.push_str(&format!("    hint: {}\n", QUOTA_HINT));
            }
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

/// Stop the run when a step failed for lack of permission: later steps need
/// the same or broader access, so continuing would only repeat the same denial.
fn permission_gate(summary: &RunSummary) -> Result<(), Box<dyn std::error::Error>> {
    if summary.has_permission_failure() {
        print!("{}", render_summary(&summary.steps));
        return Err("bootstrap stopped: the ADC identity lacks a required permission".into());
    }
    Ok(())
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
    // The estate reads as Satz. Bootstrap only ever addresses the parameter
    // table — every `lookup` below names a param, never a document position —
    // so the table synthesized into the two shapes the lookups expect serves
    // it exactly.
    if config_file.extension().and_then(|e| e.to_str()) != Some("satz") {
        return Err(format!(
            "{} is not a Satz estate — bootstrap reads Satz only; convert with `satz import <file>.yaml`",
            config_file.display()
        )
        .into());
    }
    let params = crate::satz_estate_params(config_file, include_dirs)?;
    let mut flat = serde_yaml::Mapping::new();
    for (k, v) in params {
        flat.insert(Value::String(k), v);
    }
    let mut with_block = serde_yaml::Mapping::new();
    with_block.insert(Value::String("variables".into()), Value::Mapping(flat.clone()));
    Ok((Value::Mapping(with_block), Value::Mapping(flat)))
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
        println!("Dry run: nothing will be created; the identity check and permission pre-flight are read-only.");
    }

    // 1. Get Authentication Token. On a dry run, missing credentials end the
    // run with a NAMED skip: the plan above is still useful offline, but a
    // pre-flight that silently did not happen must never read as one that
    // passed.
    println!("Authenticating using Application Default Credentials...");
    let token = match crate::gcp::access_token().await {
        Ok(t) => t,
        Err(e) if dry_run => {
            println!(
                "pre-flight: SKIPPED — no usable Application Default Credentials ({}). Run `gcloud auth \
                 application-default login`, then re-run --dry-run to check permissions.",
                e
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

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

    // The identity is resolved even when the config cannot be verified against
    // it: the pre-flight needs the principal for its self-grant member string.
    let resolved_identity = resolve_adc_identity(&client, &token).await;
    match (&expected_admin, &resolved_identity) {
        (Some(expected), None) => {
            // These messages are printed rather than carried in the error, because `main`
            // renders a returned error with `Debug`, which would escape the newlines.
            eprintln!(
                "\nCould not determine the identity of your Application Default Credentials,\n\
                 so it cannot be checked against the configured admin '{expected}'.\n\n\
                 Authenticate with:  gcloud auth application-default login {expected}\n"
            );
            return Err("could not verify the Application Default Credentials identity".into());
        }
        (Some(expected), Some((actual, source))) => {
            if !identity_matches(expected, actual) {
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
        (None, _) => {
            eprintln!(
                "Warning: this config defines no 'first-admin' / 'customer-domain', so the identity \
                 of your Application Default Credentials cannot be verified. Continuing."
            );
        }
    }

    // 2. Pre-flight: the REQUIRED permissions on the scope root and the
    // billing account, tested before anything is created — and self-granted
    // where the caller holds setIamPolicy there. Read-only on a dry run.
    let infra_folder_name = lookup_str(&["infra-folder-name"]).filter(|s| !s.is_empty());
    let infra_folder_name = infra_folder_name.as_deref();
    let principal = resolved_identity.as_ref().map(|(email, _)| email.as_str());
    crate::preflight::run(
        &client,
        &token,
        &parent,
        infra_folder_name.is_some(),
        &bid,
        principal,
        dry_run,
    )
    .await?;

    if dry_run {
        println!("Dry run complete: nothing was created.");
        return Ok(());
    }

    println!("Starting bootstrap process...");
    let mut summary = RunSummary::default();

    // 2. Create Folder (if specified)    // 3. Create Folder (if specified)
    let mut current_parent = parent.clone();

    if let Some(folder_display_name) = infra_folder_name {
        let step = format!("Folder \"{}\"", folder_display_name);
        println!("Checking for existing Infrastructure Folder: {}...", folder_display_name);

        // 3a. Search for folder by display name in the parent — every page,
        // and exactly one match. A failed search must not fall through to
        // creation: that would try to create a folder that may already exist,
        // and the real cause (usually a denied list call) would never be reported.
        match crate::gcp::resourcemanager::list_folders(&client, &token, &parent).await {
            Err(e) => summary.record(
                &step,
                StepStatus::Failed(e.class(), format!("could not list folders under {}: {}", parent, e)),
            ),
            Ok(folders) => {
                match crate::gcp::resourcemanager::find_folder_by_display_name(&folders, folder_display_name) {
                    Err(err) => summary.record(&step, StepStatus::Failed(ErrorClass::Other, err)),
                    Ok(Some(folder_id)) => {
                        current_parent = folder_id;
                        summary.record(&step, StepStatus::AlreadyExisted);
                    }
                    Ok(None) => {
                        // 3b. Not found, proceed with creation.
                        println!("Creating Infrastructure Folder: {}...", folder_display_name);
                        match crate::gcp::resourcemanager::create_folder(&client, &token, &parent, folder_display_name).await {
                            Ok(name) => {
                                current_parent = name;
                                summary.record(&step, StepStatus::Created);
                            }
                            Err(e) => summary.record(&step, StepStatus::Failed(e.class(), e.into())),
                        }
                    }
                }
            }
        }
    }

    // Later steps all target resources inside this folder, so a permission problem here
    // would just repeat itself with a less informative message.
    permission_gate(&summary)?;

    // 4. Create Project Shell
    println!("Creating Project: {}...", project_id);
    let project_step = format!("Project {}", project_id);
    // Billing, API enablement and the bucket all live inside this project, so track
    // whether it exists explicitly rather than inferring it from the last recorded step.
    let mut project_usable = true;
    // The project NUMBER: the create operation's response carries it, and it
    // is the id every folder-scoped or project-scoped adoption needs. It used
    // to be discarded here — the only place the tool ever had it.
    let mut project_number: Option<String> = None;
    match crate::gcp::resourcemanager::create_project(&client, &token, &project_id, Some(&current_parent)).await {
        Ok(crate::gcp::resourcemanager::ProjectOutcome::Created { number }) => {
            project_number = number;
            summary.record(&project_step, StepStatus::Created);
        }
        Ok(crate::gcp::resourcemanager::ProjectOutcome::AlreadyExists) => {
            summary.record(&project_step, StepStatus::AlreadyExisted);
            match crate::gcp::resourcemanager::get_project_number(&client, &token, &project_id).await {
                Ok(n) => project_number = n,
                Err(e) => eprintln!("warning: could not read project {}: {}", project_id, e),
            }
        }
        Err(e) => {
            project_usable = false;
            summary.record(&project_step, StepStatus::Failed(e.class(), e.into()));
        }
    }
    if let Some(n) = &project_number {
        println!("Project number: {}", n.trim_start_matches("projects/"));
    }

    permission_gate(&summary)?;

    // 5. Link Billing Account
    let billing_step = format!("Billing link {} -> {}", project_id, bid);
    if !project_usable {
        summary.record(&billing_step, StepStatus::Skipped(format!("{} was not created", project_id)));
    } else {
        // `PUT billingInfo` is an upsert that returns 200 whether or not anything changed,
        // so read first — otherwise every re-run reports the link as newly created.
        // An unreadable current state is not evidence of anything; fall through to
        // the write rather than reporting a state we did not confirm.
        let current = crate::gcp::billing::project_billing_account(&client, &token, &project_id)
            .await
            .ok()
            .flatten();
        if current.as_deref() == Some(bid.as_str()) {
            summary.record(&billing_step, StepStatus::AlreadyExisted);
        } else {
            println!("Linking Billing Account: {}...", bid);
            match crate::gcp::billing::set_project_billing_account(&client, &token, &project_id, &bid).await {
                Ok(()) => summary.record(&billing_step, StepStatus::Created),
                Err(e) => summary.record(&billing_step, StepStatus::Failed(e.class(), e.into())),
            }
        }
    }
    permission_gate(&summary)?;

    // 6. Enable Foundation APIs (The "Chicken-and-Egg" Fix)
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

        // `services:enable` succeeds whether or not the API was already on, so read the
        // current state first to keep the summary honest about what this run changed.
        // A failed read is not evidence of anything; fall through to the enable, which
        // reports the real error.
        let already_enabled = crate::gcp::serviceusage::service_enabled(&client, &token, &project_id, service)
            .await
            .unwrap_or(false);
        if already_enabled {
            summary.record(&step, StepStatus::AlreadyExisted);
            continue;
        }

        println!("Enabling core service: {}...", service);
        // Collected rather than fatal, so one run reports every API that could not be
        // enabled instead of stopping at the first.
        match crate::gcp::serviceusage::enable_service(&client, &token, &project_id, service).await {
            Ok(()) => summary.record(&step, StepStatus::Created),
            Err(e) => summary.record(&step, StepStatus::Failed(e.class(), e.into())),
        }
    }
    permission_gate(&summary)?;

    // 7. Create GCS State Bucket
    let bucket_step = format!("State bucket {}", bucket_name);
    if !project_usable {
        summary.record(&bucket_step, StepStatus::Skipped(format!("{} was not created", project_id)));
    } else {
        println!("Creating GCS State Bucket: {}...", bucket_name);
        match crate::gcp::storage::create_bucket(&client, &token, &project_id, &bucket_name, &r).await {
            Ok(crate::gcp::storage::BucketOutcome::Created) => summary.record(&bucket_step, StepStatus::Created),
            Ok(crate::gcp::storage::BucketOutcome::AlreadyExists) => summary.record(&bucket_step, StepStatus::AlreadyExisted),
            Err(e) => summary.record(&bucket_step, StepStatus::Failed(e.class(), e.into())),
        }
    }

    // The summary is printed before the automatic setup below so that, if a step failed,
    // the cause is visible above whatever transpile/init/import go on to report.
    print!("{}", render_summary(&summary.steps));
    if summary.failed() > 0 {
        return Err(format!("bootstrap finished with {} failed step(s)", summary.failed()).into());
    }
    println!("Core Infrastructure (Folder, Project, Billing, Foundation APIs, State Bucket) is now ready.");

    // 8. Automatic setup: Transpile -> Init -> Import
    println!("Running automatic setup...");

    // 8a. Transpile
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

    // 8b. Init
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
            outcome("API storage.googleapis.com", StepStatus::Failed(ErrorClass::QuotaProject, "SERVICE_DISABLED".into())),
            outcome("State bucket acme", StepStatus::Failed(ErrorClass::PermissionDenied, "PERMISSION_DENIED".into())),
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
        // A quota-class failure carries its hint — the raw 403 alone reads like a denial.
        assert!(out.contains("set-quota-project"), "{out}");
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

        s.record("Bucket", StepStatus::Failed(ErrorClass::Other, "boom".into()));
        s.record("API x", StepStatus::Failed(ErrorClass::Other, "boom".into()));
        assert_eq!(s.failed(), 2);
    }

    #[test]
    fn permission_failure_is_detected_only_for_permission_denials() {
        let mut s = RunSummary::default();
        s.record("Project", StepStatus::Failed(ErrorClass::Conflict, "ALREADY_EXISTS".into()));
        assert!(!s.has_permission_failure());

        // A quota-class 403 is not a permission problem and must not stop the run.
        s.record("API x", StepStatus::Failed(ErrorClass::QuotaProject, "SERVICE_DISABLED".into()));
        assert!(!s.has_permission_failure());

        s.record("Folder", StepStatus::Failed(ErrorClass::PermissionDenied, "PERMISSION_DENIED on folders.create".into()));
        assert!(s.has_permission_failure(), "a real denial must stop the run");
    }
}
