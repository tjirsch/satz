//! Cloud Billing API: which billing account a project is linked to. The
//! Resource Manager asset says nothing about billing, so an imported project
//! would otherwise plan `billing_account = null` — an unlink.

/// `projects.getBillingInfo`: the billing account id (`XXXXXX-XXXXXX-XXXXXX`)
/// or `None` when the project has none.
pub(crate) async fn project_billing_account(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
) -> Result<Option<String>, String> {
    let res = client
        .get(format!("https://cloudbilling.googleapis.com/v1/projects/{}/billingInfo", project_id))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(http_error(res).await);
    }
    let info: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(info
        .get("billingAccountName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches("billingAccounts/").to_string()))
}

/// `403 Forbidden: <body>` — the status is part of the error, and an empty
/// body never yields an empty error.
async fn http_error(res: reqwest::Response) -> String {
    let status = res.status();
    let body = res.text().await.unwrap_or_else(|e| format!("(body unreadable: {})", e));
    let body = if body.trim().is_empty() { "(empty body)".to_string() } else { body };
    format!("{} {}: {}", status.as_u16(), status.canonical_reason().unwrap_or(""), body)
}
