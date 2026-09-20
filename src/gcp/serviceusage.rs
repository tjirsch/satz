//! Service Usage API: read and enable services on a project. `services:enable`
//! returns 200 whether or not the API was already on, so callers that want an
//! honest "changed vs. already there" read the state first.
//!
//! One service at a time for `bootstrap`, which reports a step per API as it
//! creates the project; in batches for the `plan`/`apply` preflight, which asks
//! about every API an estate declares at once.

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

/// How many services `services:batchGet` and `services:batchEnable` take in one
/// request. The API's own limit.
const BATCH: usize = 20;

/// Which of `services` are enabled on `project_id`, one entry per service the
/// API answered for.
///
/// `services:batchGet` rather than a `get` per service: an estate declares
/// twenty-odd APIs and each `get` is a round trip. The response names a service
/// by project NUMBER (`projects/123/services/iam.googleapis.com`) whatever the
/// request used, so a row is matched back by its last segment, never by the name
/// it was asked for.
pub(crate) async fn service_states(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    services: &[String],
) -> Result<std::collections::BTreeMap<String, bool>, ApiError> {
    let mut states = std::collections::BTreeMap::new();
    for chunk in services.chunks(BATCH) {
        let names: Vec<(&str, String)> = chunk
            .iter()
            .map(|s| ("names", format!("projects/{}/services/{}", project_id, s)))
            .collect();
        let res = client
            .get(format!("{}/projects/{}/services:batchGet", BASE, project_id))
            .query(&names)
            .bearer_auth(token)
            .send()
            .await
            .map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let v: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        for (id, enabled) in states_of(&v) {
            states.insert(id, enabled);
        }
    }
    Ok(states)
}

/// `services:batchEnable` on `project_id`, in chunks of twenty, each waited to
/// completion. Returns when every service is on — an operation that reports an
/// error is that error, and a half-finished batch is never reported as done.
pub(crate) async fn batch_enable(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    services: &[String],
) -> Result<(), ApiError> {
    for chunk in services.chunks(BATCH) {
        let res = client
            .post(format!("{}/projects/{}/services:batchEnable", BASE, project_id))
            .bearer_auth(token)
            .json(&serde_json::json!({ "serviceIds": chunk }))
            .send()
            .await
            .map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let op: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        let op = match op.get("done").and_then(|d| d.as_bool()) {
            Some(true) => op,
            _ => {
                let Some(name) = op.get("name").and_then(|n| n.as_str()) else {
                    return Err(ApiError::transport(
                        "the API accepted the request but returned no operation name",
                    ));
                };
                await_operation(client, token, name).await?
            }
        };
        if let Some(err) = op.get("error") {
            return Err(ApiError::from_operation_error(err));
        }
    }
    Ok(())
}

/// Poll a Service Usage operation until it reports `done`, or the deadline
/// passes. Without a deadline a stuck operation loops forever; without reading
/// the terminal object, a failed operation reads as a success.
async fn await_operation(
    client: &reqwest::Client,
    token: &str,
    op_name: &str,
) -> Result<serde_json::Value, ApiError> {
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let res = client
            .get(format!("{}/{}", BASE, op_name))
            .bearer_auth(token)
            .send()
            .await
            .map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let op: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        if op.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(op);
        }
    }
    Err(ApiError::transport(format!("enabling did not finish in two minutes ({})", op_name)))
}

/// The service id and state of every row in a `services:batchGet` response.
fn states_of(v: &serde_json::Value) -> Vec<(String, bool)> {
    v.get("services")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s.get("name").and_then(|n| n.as_str())?.rsplit('/').next()?;
            Some((id.to_string(), service_is_enabled(s)))
        })
        .collect()
}

/// True when a serviceusage service resource reports itself as already enabled.
fn service_is_enabled(v: &serde_json::Value) -> bool {
    v.get("state").and_then(|s| s.as_str()) == Some("ENABLED")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The batch response identifies a service by project NUMBER, so the id has
    /// to come out of the name. Matching the request's own spelling instead read
    /// every service as absent, which the preflight reports as "not answered for".
    #[test]
    fn a_batch_row_is_keyed_by_the_service_id_in_its_name() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"services":[
                 {"name":"projects/123456789012/services/iam.googleapis.com","state":"ENABLED"},
                 {"name":"projects/123456789012/services/cloudasset.googleapis.com","state":"DISABLED"},
                 {"state":"ENABLED"}
               ]}"#,
        )
        .unwrap();
        assert_eq!(
            states_of(&v),
            vec![
                ("iam.googleapis.com".to_string(), true),
                ("cloudasset.googleapis.com".to_string(), false),
            ]
        );
        assert!(states_of(&serde_json::json!({})).is_empty());
    }

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
