//! Cloud Billing API: which billing account a project is linked to, and the
//! link itself. The Resource Manager asset says nothing about billing, so an
//! imported project would otherwise plan `billing_account = null` — an unlink.

use super::ApiError;

/// `projects.getBillingInfo`: the billing account id (`XXXXXX-XXXXXX-XXXXXX`)
/// or `None` when the project has none.
pub(crate) async fn project_billing_account(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
) -> Result<Option<String>, ApiError> {
    let res = client
        .get(format!("https://cloudbilling.googleapis.com/v1/projects/{}/billingInfo", project_id))
        .bearer_auth(token)
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let info: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    Ok(info
        .get("billingAccountName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches("billingAccounts/").to_string()))
}

/// `projects.updateBillingInfo`: link `project_id` to the billing account
/// (bare id, no `billingAccounts/` prefix). An upsert — it returns 200 whether
/// or not anything changed, so callers that want an honest "changed vs.
/// already linked" read [`project_billing_account`] first.
pub(crate) async fn set_project_billing_account(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    billing_account: &str,
) -> Result<(), ApiError> {
    let res = client
        .put(format!("https://cloudbilling.googleapis.com/v1/projects/{}/billingInfo", project_id))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "billingAccountName": format!("billingAccounts/{}", billing_account)
        }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    Ok(())
}

/// `billingAccounts.budgets.list`: every budget of the account as (resource
/// name, display name). Paginated; an error is an error.
pub(crate) async fn list_budgets(
    client: &reqwest::Client,
    token: &str,
    billing_account: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get(format!("https://billingbudgets.googleapis.com/v1/billingAccounts/{}/budgets", billing_account))
            .bearer_auth(token)
            .query(&[("pageSize", "100")]);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let res = req.send().await.map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let page: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        for b in page.get("budgets").and_then(|v| v.as_array()).into_iter().flatten() {
            let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let display = b.get("displayName").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !name.is_empty() {
                out.push((name, display));
            }
        }
        page_token = page.get("nextPageToken").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string());
        if page_token.is_none() {
            break;
        }
    }
    Ok(out)
}
