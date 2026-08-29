//! Thin clients for the Google Cloud APIs satz talks to directly (REST over
//! `reqwest`, ADC bearer token). One module per API; the pure parts — page
//! merging, matching — are separate functions so they can be tested without
//! a network.

pub(crate) mod billing;
pub(crate) mod resourcemanager;

/// An ADC bearer token for the cloud-platform scope.
pub(crate) async fn access_token() -> Result<String, String> {
    use google_cloud_auth::credentials::Builder;
    let credentials = Builder::default()
        .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
        .build_access_token_credentials()
        .map_err(|e| e.to_string())?;
    Ok(credentials.access_token().await.map_err(|e| e.to_string())?.token)
}
