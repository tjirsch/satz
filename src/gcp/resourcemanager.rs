//! Cloud Resource Manager v3: folders and projects.
//!
//! The lookups adoption needs — "the folder called X under parent P" and "the
//! number of project Y" — both live here. Folder listing is paginated:
//! `folders.list` returns at most 300 per page, and the first version of
//! `bootstrap` read only the first page, so an org with many folders under one
//! parent could miss an existing folder and fall through to creating it.

const BASE: &str = "https://cloudresourcemanager.googleapis.com/v3";

/// Every folder directly under `parent` (`organizations/<id>` or
/// `folders/<id>`), across all pages.
pub(crate) async fn list_folders(
    client: &reqwest::Client,
    token: &str,
    parent: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get(format!("{}/folders", BASE))
            .query(&[("parent", parent), ("pageSize", "300")])
            .bearer_auth(token);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let res = req.send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(res.text().await.unwrap_or_default());
        }
        let page: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        page_token = next_page(&mut out, &page);
        if page_token.is_none() {
            return Ok(out);
        }
    }
}

/// Append one `folders.list` page and return the token of the next, if any.
fn next_page(out: &mut Vec<serde_json::Value>, page: &serde_json::Value) -> Option<String> {
    if let Some(list) = page.get("folders").and_then(|v| v.as_array()) {
        out.extend(list.iter().cloned());
    }
    page.get("nextPageToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// The folder whose `displayName` equals `display_name`, as `folders/<number>`.
/// Exactly one match resolves; several is an error — GCP does not enforce
/// display-name uniqueness under a parent, and guessing here would adopt the
/// wrong folder.
pub(crate) fn find_folder_by_display_name(
    folders: &[serde_json::Value],
    display_name: &str,
) -> Result<Option<String>, String> {
    let matches: Vec<&str> = folders
        .iter()
        .filter(|f| f.get("displayName").and_then(|v| v.as_str()) == Some(display_name))
        .filter_map(|f| f.get("name").and_then(|v| v.as_str()))
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).to_string())),
        many => Err(format!(
            "{} folders are called {:?}: {} — pin the one you mean with \"import-id\"",
            many.len(),
            display_name,
            many.join(", ")
        )),
    }
}

/// `projects.get`: the project's resource name `projects/<number>`, or `None`
/// when it does not exist (404). Anything else is an error.
pub(crate) async fn get_project_number(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
) -> Result<Option<String>, String> {
    let res = client
        .get(format!("{}/projects/{}", BASE, project_id))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(res.text().await.unwrap_or_default());
    }
    let project: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(project.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pages_are_merged_until_the_token_runs_out() {
        let mut out = Vec::new();
        let t1 = next_page(&mut out, &json!({"folders": [{"name": "folders/1"}], "nextPageToken": "p2"}));
        assert_eq!(t1.as_deref(), Some("p2"));
        let t2 = next_page(&mut out, &json!({"folders": [{"name": "folders/2"}], "nextPageToken": ""}));
        assert_eq!(t2, None);
        let t3 = next_page(&mut out, &json!({}));
        assert_eq!(t3, None);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn display_name_match_is_exact_and_unique() {
        let folders = vec![
            json!({"name": "folders/1", "displayName": "Infra"}),
            json!({"name": "folders/2", "displayName": "infra"}),
            json!({"name": "folders/3", "displayName": "Workloads"}),
        ];
        assert_eq!(find_folder_by_display_name(&folders, "Infra").unwrap().as_deref(), Some("folders/1"));
        assert_eq!(find_folder_by_display_name(&folders, "Nope").unwrap(), None);
        let dup = vec![
            json!({"name": "folders/1", "displayName": "Infra"}),
            json!({"name": "folders/9", "displayName": "Infra"}),
        ];
        let err = find_folder_by_display_name(&dup, "Infra").unwrap_err();
        assert!(err.contains("folders/1, folders/9"), "{}", err);
    }
}
