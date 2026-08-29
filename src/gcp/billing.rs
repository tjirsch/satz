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
        return Err(res.text().await.unwrap_or_default());
    }
    let info: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(info
        .get("billingAccountName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches("billingAccounts/").to_string()))
}
