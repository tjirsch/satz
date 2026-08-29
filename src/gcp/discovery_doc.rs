//! Google API Discovery Documents: the API-side schema of a Cloud Asset
//! Inventory asset type. `storage.googleapis.com/Bucket` → the `storage`
//! API's preferred version → its `schemas.Bucket`. Documents are cached
//! under `<presets_dir>/.discovery/` — they change rarely and the directory
//! listing is one more request per run otherwise.

use std::path::{Path, PathBuf};

const DIRECTORY: &str = "https://www.googleapis.com/discovery/v1/apis";

/// (service, type name) of an asset type: `orgpolicy.googleapis.com/Policy`
/// → (`orgpolicy`, `Policy`).
pub(crate) fn split_asset_type(asset_type: &str) -> Option<(String, String)> {
    let (host, name) = asset_type.split_once('/')?;
    let service = host.strip_suffix(".googleapis.com")?;
    Some((service.to_string(), name.to_string()))
}

/// The preferred Discovery Document of `service`, from cache or the
/// directory. Returns the document and its `revision`.
pub(crate) async fn document(client: &reqwest::Client, cache_dir: &Path, service: &str) -> Result<serde_json::Value, String> {
    let cache = cache_dir.join(format!("{}.json", service));
    if cache.exists() {
        let text = std::fs::read_to_string(&cache).map_err(|e| format!("{}: {}", cache.display(), e))?;
        return serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("{}: cached Discovery Document is not JSON ({}) — delete the file to refetch", cache.display(), e));
    }
    let dir: serde_json::Value = client
        .get(DIRECTORY)
        .query(&[("name", service), ("preferred", "true")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let url = dir
        .get("items")
        .and_then(|i| i.as_array())
        .and_then(|a| a.first())
        .and_then(|i| i.get("discoveryRestUrl"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| format!("no Discovery Document listed for API `{}`", service))?;
    let doc: serde_json::Value = client.get(url).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    std::fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    std::fs::write(&cache, serde_json::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(doc)
}

/// The schema named `type_name` in a document — by exact id, else the one
/// whose id ends with it (newer APIs prefix schema ids:
/// `GoogleCloudOrgpolicyV2Policy` for `Policy`). Several suffix matches are
/// ambiguous, none is an error.
pub(crate) fn schema_for<'a>(doc: &'a serde_json::Value, type_name: &str) -> Result<(&'a str, &'a serde_json::Value), String> {
    let schemas = doc.get("schemas").and_then(|s| s.as_object()).ok_or("document has no `schemas`")?;
    if let Some((k, s)) = schemas.get_key_value(type_name) {
        return Ok((k.as_str(), s));
    }
    let hits: Vec<(&str, &serde_json::Value)> = schemas
        .iter()
        .filter(|(k, _)| k.ends_with(type_name) && k[..k.len() - type_name.len()].chars().last().is_none_or(|c| c.is_ascii_digit() || c.is_ascii_lowercase()))
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("no schema named or ending in `{}`", type_name)),
        many => Err(format!(
            "several schemas end in `{}`: {} — name the one you mean in the row's `api_schema`",
            type_name,
            many.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
        )),
    }
}

pub(crate) fn cache_dir(presets_dir: &str) -> PathBuf {
    Path::new(presets_dir).join(".discovery")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_types_split_into_service_and_type() {
        assert_eq!(split_asset_type("storage.googleapis.com/Bucket"), Some(("storage".into(), "Bucket".into())));
        assert_eq!(split_asset_type("cloudresourcemanager.googleapis.com/Folder"), Some(("cloudresourcemanager".into(), "Folder".into())));
        assert_eq!(split_asset_type("TODO/UNKNOWN"), None);
    }

    #[test]
    fn schema_lookup_is_exact_then_unique_suffix() {
        let doc = serde_json::json!({"schemas": {
            "Bucket": {"type": "object"},
            "GoogleCloudOrgpolicyV2Policy": {"type": "object"},
            "GoogleCloudOrgpolicyV2PolicySpec": {"type": "object"},
            "GoogleCloudEssentialcontactsV1Contact": {"type": "object"}
        }});
        assert_eq!(schema_for(&doc, "Bucket").unwrap().0, "Bucket");
        assert_eq!(schema_for(&doc, "Policy").unwrap().0, "GoogleCloudOrgpolicyV2Policy");
        assert_eq!(schema_for(&doc, "Contact").unwrap().0, "GoogleCloudEssentialcontactsV1Contact");
        assert!(schema_for(&doc, "Nothing").is_err());
    }
}
