//! Service Usage API: read and enable services on a project. `services:enable`
//! returns 200 whether or not the API was already on, so callers that want an
//! honest "changed vs. already there" read the state first.

use super::ApiError;

const BASE: &str = "https://serviceusage.googleapis.com/v1";

/// True when `service` is already enabled on `project_id`.
pub(crate) async fn service_enabled(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    service: &str,
) -> Result<bool, ApiError> {
    let res = client
        .get(format!("{}/projects/{}/services/{}", BASE, project_id, service))
        .bearer_auth(token)
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let v: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    Ok(service_is_enabled(&v))
}

/// `services:enable` on `project_id`.
pub(crate) async fn enable_service(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    service: &str,
) -> Result<(), ApiError> {
    let res = client
        .post(format!("{}/projects/{}/services/{}:enable", BASE, project_id, service))
        .bearer_auth(token)
        // An empty JSON body: without one the API answers 411 Length Required.
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    Ok(())
}

/// True when a serviceusage service resource reports itself as already enabled.
fn service_is_enabled(v: &serde_json::Value) -> bool {
    v.get("state").and_then(|s| s.as_str()) == Some("ENABLED")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_state_distinguishes_enabled_from_disabled() {
        // Without this, `services:enable` returning 200 made every re-run report
        // "created" for APIs that were already on.
        let jv = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert!(service_is_enabled(&jv(
            r#"{"name":"projects/1/services/iam.googleapis.com","state":"ENABLED"}"#
        )));
        assert!(!service_is_enabled(&jv(r#"{"state":"DISABLED"}"#)));
        assert!(!service_is_enabled(&jv(r#"{"state":"STATE_UNSPECIFIED"}"#)));
        assert!(!service_is_enabled(&jv("{}")));
    }
}
