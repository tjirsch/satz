//! Cloud Storage JSON API: the state bucket. Uniform bucket-level access and
//! versioning are non-negotiable for a Terraform state bucket, so they are set
//! at creation, not patched in later.

use super::ApiError;

/// What `create_bucket` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BucketOutcome {
    Created,
    AlreadyExists,
}

/// Create `name` in `project_id` with UBLA + versioning; 409 means it exists.
pub(crate) async fn create_bucket(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    name: &str,
    location: &str,
) -> Result<BucketOutcome, ApiError> {
    let res = client
        .post(format!("https://storage.googleapis.com/storage/v1/b?project={}", project_id))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "location": location,
            "iamConfiguration": { "uniformBucketLevelAccess": { "enabled": true } },
            "versioning": { "enabled": true }
        }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if res.status().as_u16() == 409 {
        return Ok(BucketOutcome::AlreadyExists);
    }
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    Ok(BucketOutcome::Created)
}
