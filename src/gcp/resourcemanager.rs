//! Cloud Resource Manager v3: folders and projects.
//!
//! The lookups adoption needs — "the folder called X under parent P" and "the
//! number of project Y" — both live here, as do the two creations bootstrap
//! performs. Folder listing is paginated: `folders.list` returns at most 300
//! per page, and the first version of `bootstrap` read only the first page, so
//! an org with many folders under one parent could miss an existing folder and
//! fall through to creating it.

use super::ApiError;

const BASE: &str = "https://cloudresourcemanager.googleapis.com/v3";

/// Every folder directly under `parent` (`organizations/<id>` or
/// `folders/<id>`), across all pages.
pub(crate) async fn list_folders(
    client: &reqwest::Client,
    token: &str,
    parent: &str,
) -> Result<Vec<serde_json::Value>, ApiError> {
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
        let res = req.send().await.map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let page: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
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
            "{} folders are called {:?}: {} — name the one you mean by its id",
            many.len(),
            display_name,
            many.join(", ")
        )),
    }
}

/// A display-name path from the organization (`"Shared Services/Prod"`) to
/// the `folders/<n>` it names, one segment at a time: under the resolved
/// parent exactly one folder may carry the name — none is an error, several
/// is AMBIGUOUS with the candidates listed. Never a guess.
pub(crate) async fn resolve_folder_path(
    client: &reqwest::Client,
    token: &str,
    organization: &str,
    path: &str,
) -> Result<String, String> {
    let mut parent = format!("organizations/{}", organization.trim_start_matches("organizations/"));
    for segment in path.split('/').map(str::trim).filter(|s| !s.is_empty()) {
        let folders = list_folders(client, token, &parent).await?;
        parent = find_folder_by_display_name(&folders, segment)?
            .ok_or_else(|| format!("no folder {:?} under {}", segment, parent))?;
    }
    if parent.starts_with("organizations/") {
        return Err(format!("folder path {:?} is empty", path));
    }
    Ok(parent)
}

/// `projects.get`: the project's resource name `projects/<number>`, or `None`
/// when it does not exist (404). Anything else is an error.
pub(crate) async fn get_project_number(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
) -> Result<Option<String>, ApiError> {
    let res = client
        .get(format!("{}/projects/{}", BASE, project_id))
        .bearer_auth(token)
        .send()
        .await
        .map_err(ApiError::transport)?;
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let project: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    Ok(project.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
}

/// Poll a long-running operation until it reports `done`, or the deadline
/// passes. Without a deadline a stuck operation loops forever; without
/// inspecting the terminal object, a failed operation reads as a success.
pub(crate) async fn await_operation(
    client: &reqwest::Client,
    token: &str,
    op_name: &str,
    interval: std::time::Duration,
    max_polls: u32,
) -> Result<serde_json::Value, ApiError> {
    for _ in 0..max_polls {
        tokio::time::sleep(interval).await;
        let res = client
            .get(format!("{}/{}", BASE, op_name))
            .bearer_auth(token)
            .send()
            .await
            .map_err(ApiError::transport)?;
        let op: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        if op.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(op);
        }
    }
    Err(ApiError::transport(format!(
        "operation '{}' did not complete within the timeout",
        op_name
    )))
}

/// `folders.create` under `parent`, waited to completion. Returns the new
/// folder's resource name (`folders/<number>`).
pub(crate) async fn create_folder(
    client: &reqwest::Client,
    token: &str,
    parent: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let res = client
        .post(format!("{}/folders", BASE))
        .bearer_auth(token)
        .json(&serde_json::json!({ "displayName": display_name, "parent": parent }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let info: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    let Some(op_name) = info.get("name").and_then(|v| v.as_str()) else {
        return Err(ApiError::transport(
            "the API accepted the request but returned no operation name",
        ));
    };
    println!("Folder creation in progress ({})...", op_name);
    let op = await_operation(client, token, op_name, std::time::Duration::from_secs(2), 60).await?;
    if let Some(err) = op.get("error") {
        return Err(ApiError::from_operation_error(err));
    }
    op.get("response")
        .and_then(|r| r.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::transport("creation finished with neither a response nor an error"))
}

/// What `create_project` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectOutcome {
    /// Newly created; the project NUMBER (`projects/<number>`) when the
    /// operation's response carried it.
    Created { number: Option<String> },
    /// 409: a project with this id already exists.
    AlreadyExists,
}

/// `projects.create`, waited to completion. `parent: None` creates the project
/// parentless — on a Workspace/Cloud Identity account that is the documented
/// trigger for organization auto-provisioning.
pub(crate) async fn create_project(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    parent: Option<&str>,
) -> Result<ProjectOutcome, ApiError> {
    let mut body = serde_json::json!({ "projectId": project_id, "displayName": project_id });
    if let Some(p) = parent {
        body["parent"] = serde_json::Value::String(p.to_string());
    }
    let res = client
        .post(format!("{}/projects", BASE))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(ApiError::transport)?;
    if res.status().as_u16() == 409 {
        return Ok(ProjectOutcome::AlreadyExists);
    }
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let info: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    let Some(op_name) = info.get("name").and_then(|v| v.as_str()) else {
        return Err(ApiError::transport(
            "the API accepted the request but returned no operation name",
        ));
    };
    println!("Project creation in progress ({})...", op_name);
    let op = await_operation(client, token, op_name, std::time::Duration::from_secs(3), 60).await?;
    // The operation's `error` field was once never inspected, so a failed
    // creation still reported "Project shell created."
    if let Some(err) = op.get("error") {
        return Err(ApiError::from_operation_error(err));
    }
    let number = op
        .get("response")
        .and_then(|r| r.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(ProjectOutcome::Created { number })
}

/// `testIamPermissions` on `resource` (`organizations/N`, `folders/N` or
/// `projects/ID`): the granted subset of `permissions`.
pub(crate) async fn test_permissions(
    client: &reqwest::Client,
    token: &str,
    resource: &str,
    permissions: &[&str],
) -> Result<Vec<String>, ApiError> {
    super::test_iam_permissions(
        client,
        token,
        &format!("{}/{}:testIamPermissions", BASE, resource),
        permissions,
    )
    .await
}

/// The resource's IAM policy, requested at version 3 so conditional bindings
/// are visible — a read-modify-write on a lower version would destroy them.
pub(crate) async fn get_iam_policy(
    client: &reqwest::Client,
    token: &str,
    resource: &str,
) -> Result<serde_json::Value, ApiError> {
    let res = client
        .post(format!("{}/{}:getIamPolicy", BASE, resource))
        .bearer_auth(token)
        .json(&serde_json::json!({ "options": { "requestedPolicyVersion": 3 } }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    res.json().await.map_err(ApiError::transport)
}

/// Replace the resource's IAM policy. `policy` carries the `etag` from the
/// read; a concurrent change makes this a 409 the caller retries from a fresh
/// read.
pub(crate) async fn set_iam_policy(
    client: &reqwest::Client,
    token: &str,
    resource: &str,
    policy: &serde_json::Value,
) -> Result<(), ApiError> {
    let res = client
        .post(format!("{}/{}:setIamPolicy", BASE, resource))
        .bearer_auth(token)
        .json(&serde_json::json!({ "policy": policy }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    Ok(())
}

/// `organizations:search` — every organization visible to the caller, across
/// all pages. An empty result is the greenfield signal, not an error.
pub(crate) async fn search_organizations(
    client: &reqwest::Client,
    token: &str,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get(format!("{}/organizations:search", BASE))
            .bearer_auth(token);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let res = req.send().await.map_err(ApiError::transport)?;
        if !res.status().is_success() {
            return Err(super::api_error(res).await);
        }
        let page: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
        if let Some(list) = page.get("organizations").and_then(|v| v.as_array()) {
            out.extend(list.iter().cloned());
        }
        page_token = page
            .get("nextPageToken")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if page_token.is_none() {
            return Ok(out);
        }
    }
}

/// `projects.move` under `destination_parent`, waited to completion.
pub(crate) async fn move_project(
    client: &reqwest::Client,
    token: &str,
    project_id: &str,
    destination_parent: &str,
) -> Result<(), ApiError> {
    let res = client
        .post(format!("{}/projects/{}:move", BASE, project_id))
        .bearer_auth(token)
        .json(&serde_json::json!({ "destinationParent": destination_parent }))
        .send()
        .await
        .map_err(ApiError::transport)?;
    if !res.status().is_success() {
        return Err(super::api_error(res).await);
    }
    let info: serde_json::Value = res.json().await.map_err(ApiError::transport)?;
    let Some(op_name) = info.get("name").and_then(|v| v.as_str()) else {
        return Err(ApiError::transport(
            "the API accepted the request but returned no operation name",
        ));
    };
    let op = await_operation(client, token, op_name, std::time::Duration::from_secs(3), 60).await?;
    if let Some(err) = op.get("error") {
        return Err(ApiError::from_operation_error(err));
    }
    Ok(())
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
