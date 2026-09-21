//! The IAM policy of the resource a `*_iam_member` grant is made on — what
//! adoption reads to tell a grant that is live (import it) from one that is not
//! (apply creates it). One reader per API that serves the parent; each asks for
//! policy version 3, so conditional bindings come back with their condition.
//!
//! `Ok(None)` is a 404: the parent itself does not exist. Every other failure is
//! an error — an unreadable policy is never read as an empty one.

use super::ApiError;

/// The API that holds a parent's IAM policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyApi {
    /// Cloud Resource Manager: `organizations/<n>`, `folders/<n>`, `projects/<id>`.
    ResourceManager,
    /// Cloud Billing: `billingAccounts/<id>`.
    Billing,
    /// Cloud Storage: `b/<bucket>`.
    Storage,
    /// IAM: `projects/<p>/serviceAccounts/<email>`.
    ServiceAccount,
    /// Pub/Sub: `projects/<p>/topics/<t>` and `projects/<p>/subscriptions/<s>`.
    PubSub,
    /// BigQuery: `projects/<p>/datasets/<d>`. A dataset has no IAM policy
    /// endpoint; its `access` list is the policy, read as the provider reads it.
    BigQueryDataset,
}

/// The parent's policy, `None` when the parent does not exist.
pub(crate) async fn read(
    client: &reqwest::Client,
    token: &str,
    api: PolicyApi,
    resource: &str,
) -> Result<Option<serde_json::Value>, ApiError> {
    let got = match api {
        PolicyApi::ResourceManager => super::resourcemanager::get_iam_policy(client, token, resource).await,
        PolicyApi::Billing => get(client, token, &format!("https://cloudbilling.googleapis.com/v1/{}:getIamPolicy", resource), "options.requestedPolicyVersion").await,
        PolicyApi::Storage => get(
            client,
            token,
            &format!("https://storage.googleapis.com/storage/v1/b/{}/iam", resource.trim_start_matches("b/")),
            "optionsRequestedPolicyVersion",
        )
        .await,
        PolicyApi::ServiceAccount => {
            let res = client
                .post(format!("https://iam.googleapis.com/v1/{}:getIamPolicy", resource))
                .query(&[("options.requestedPolicyVersion", "3")])
                .bearer_auth(token)
                .send()
                .await
                .map_err(ApiError::transport)?;
            json(res).await
        }
        PolicyApi::PubSub => get(client, token, &format!("https://pubsub.googleapis.com/v1/{}:getIamPolicy", resource), "options.requestedPolicyVersion").await,
        PolicyApi::BigQueryDataset => get_dataset(client, token, resource).await.map(|d| dataset_access_to_policy(&d)),
    };
    match got {
        Ok(p) => Ok(Some(p)),
        Err(e) if e.status == 404 => Ok(None),
        Err(e) => Err(e),
    }
}

async fn get(client: &reqwest::Client, token: &str, url: &str, version_param: &str) -> Result<serde_json::Value, ApiError> {
    let res = client
        .get(url)
        .query(&[(version_param, "3")])
        .bearer_auth(token)
        .send()
        .await
        .map_err(ApiError::transport)?;
    json(res).await
}

async fn get_dataset(client: &reqwest::Client, token: &str, resource: &str) -> Result<serde_json::Value, ApiError> {
    let res = client
        .get(format!("https://bigquery.googleapis.com/bigquery/v2/{}", resource))
        .bearer_auth(token)
        .send()
        .await
        .map_err(ApiError::transport)?;
    json(res).await
}

async fn json(res: reqwest::Response) -> Result<serde_json::Value, ApiError> {
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    res.json().await.map_err(ApiError::transport)
}

/// A dataset's `access` list as an IAM policy, by the provider's own mapping: the
/// three primitive roles become `roles/bigquery.data{Owner,Editor,Viewer}`, a
/// `groupByEmail` a `group:`, a `domain` a `domain:`, a `userByEmail` a
/// `serviceAccount:` when the address is a service account's and a `user:`
/// otherwise, and an `iamMember` or `specialGroup` stays as written. An entry
/// that grants to a view, a dataset or a routine names no member and is no grant.
pub(crate) fn dataset_access_to_policy(dataset: &serde_json::Value) -> serde_json::Value {
    let mut bindings: Vec<(String, Vec<String>)> = Vec::new();
    for entry in dataset.get("access").and_then(|a| a.as_array()).into_iter().flatten() {
        let Some(role) = entry.get("role").and_then(|r| r.as_str()) else { continue };
        let role = match role {
            "OWNER" => "roles/bigquery.dataOwner",
            "WRITER" => "roles/bigquery.dataEditor",
            "READER" => "roles/bigquery.dataViewer",
            other => other,
        };
        let s = |k: &str| entry.get(k).and_then(|v| v.as_str());
        let member = if let Some(g) = s("groupByEmail") {
            format!("group:{}", g)
        } else if let Some(d) = s("domain") {
            format!("domain:{}", d)
        } else if let Some(m) = s("specialGroup").or_else(|| s("iamMember")) {
            m.to_string()
        } else if let Some(u) = s("userByEmail") {
            if u.contains("gserviceaccount") {
                format!("serviceAccount:{}", u)
            } else {
                format!("user:{}", u)
            }
        } else {
            continue;
        };
        match bindings.iter_mut().find(|(r, _)| r == role) {
            Some((_, members)) => members.push(member),
            None => bindings.push((role.to_string(), vec![member])),
        }
    }
    serde_json::json!({
        "bindings": bindings
            .into_iter()
            .map(|(role, members)| serde_json::json!({ "role": role, "members": members }))
            .collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dataset_access_list_reads_as_the_provider_reads_it() {
        let dataset = serde_json::json!({ "access": [
            { "role": "READER", "groupByEmail": "auditors@example.com" },
            { "role": "READER", "userByEmail": "svc-a@example-proj.iam.gserviceaccount.com" },
            { "role": "WRITER", "userByEmail": "alice@example.com" },
            { "role": "roles/bigquery.metadataViewer", "domain": "example.com" },
            { "view": { "projectId": "p", "datasetId": "d", "tableId": "t" } },
        ]});
        let policy = dataset_access_to_policy(&dataset);
        assert_eq!(
            policy,
            serde_json::json!({ "bindings": [
                { "role": "roles/bigquery.dataViewer", "members": ["group:auditors@example.com", "serviceAccount:svc-a@example-proj.iam.gserviceaccount.com"] },
                { "role": "roles/bigquery.dataEditor", "members": ["user:alice@example.com"] },
                { "role": "roles/bigquery.metadataViewer", "members": ["domain:example.com"] },
            ]})
        );
    }
}
