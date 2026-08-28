//! Everything this tool asks of github.com, in one place: the upstream repo it
//! reads presets from, the client that talks to it, and — the reason this module
//! exists — a rate-limit failure rendered as something a human can act on.
//!
//! The unauthenticated REST quota is 60 requests/hour and is SHARED with
//! `self-update`, so a preset sweep across a fleet can lock the same user out of
//! updating their binary. That makes both halves here load-bearing: ask for as
//! little as possible, and when the quota does run out, say so.

use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde::Deserialize;

use crate::fsx;

type BoxErr = Box<dyn std::error::Error>;

/// The repository as GitHub currently resolves it. It must name a repo that
/// EXISTS: naming a future one 404s every preset command and `self-update`
/// until the rename happens, which is what v0.46.0 shipped. A rename is safe in
/// the other direction — GitHub 301-redirects a renamed repo on every API path,
/// including deep ones, so this keeps resolving after the repo becomes `satz`.
pub(crate) const REPO: &str = "tjirsch/satz";
pub(crate) const API_URL: &str = "https://api.github.com/repos";

/// Appended to a quota message by the three preset commands, all of which can
/// work from a local checkout instead. `self-update` cannot, so it passes "".
pub(crate) const PRISTINE_HINT: &str =
    ", or compare against a local checkout with `--pristine-dir <checkout>/presets`";

pub(crate) fn client(user_agent: &str) -> Result<reqwest::Client, BoxErr> {
    Ok(reqwest::Client::builder().user_agent(user_agent).build()?)
}

/// A GET against the API, carrying `GITHUB_TOKEN` when one is set — which is
/// what makes the "set GITHUB_TOKEN" advice in [`api_error`] true rather than
/// merely encouraging. Deliberately NOT applied to blob downloads: those go to
/// raw.githubusercontent.com, a different host that neither needs the token for
/// a public repo nor counts against the API quota.
pub(crate) fn api_get(http: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    let req = http.get(url);
    match token() {
        Some(t) => match HeaderValue::from_str(&format!("Bearer {t}")) {
            Ok(mut v) => {
                v.set_sensitive(true);
                req.header(AUTHORIZATION, v)
            }
            // A token with bytes a header cannot carry is not worth failing the
            // whole command over — proceed unauthenticated and let the quota
            // message explain itself if we run out.
            Err(_) => req,
        },
        None => req,
    }
}

fn token() -> Option<String> {
    let t = std::env::var("GITHUB_TOKEN").ok()?;
    let t = t.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Whole minutes until the quota is usable again: `x-ratelimit-reset` is an
/// absolute unix time, `retry-after` (sent for secondary limits) is a delta.
fn reset_minutes(headers: &HeaderMap, now: u64) -> Option<u64> {
    if let Some(reset) = header_u64(headers, "x-ratelimit-reset") {
        return Some(reset.saturating_sub(now).div_ceil(60));
    }
    header_u64(headers, "retry-after").map(|secs| secs.div_ceil(60))
}

/// Render a non-success API response.
///
/// Quota exhaustion used to reach the user as
/// `reqwest::Error { kind: Decode … "invalid type: map, expected a sequence" }`:
/// GitHub answers a 403 with a JSON OBJECT, the caller was deserializing into a
/// sequence, and so the user got the deserializer's complaint instead of the
/// limit that actually stopped the command. Check the status first, and name it.
pub(crate) fn api_error(what: &str, status: StatusCode, headers: &HeaderMap, hint: &str) -> String {
    api_error_at(what, status, headers, hint, now_secs(), token().is_some())
}

// `now` and `authed` are arguments so the rendering is testable without a clock
// or a process-wide env var — two tests setting GITHUB_TOKEN would race.
fn api_error_at(
    what: &str,
    status: StatusCode,
    headers: &HeaderMap,
    hint: &str,
    now: u64,
    authed: bool,
) -> String {
    // A 403 is not always the quota — it is also how GitHub answers a private
    // repo or a bad token, and telling that user to "wait an hour" would send
    // them nowhere. `x-ratelimit-remaining: 0` is what separates the two.
    let exhausted = matches!(status.as_u16(), 403 | 429)
        && header_u64(headers, "x-ratelimit-remaining") == Some(0);
    if !exhausted {
        return format!("{what}: GitHub returned {status}");
    }
    let limit = header_u64(headers, "x-ratelimit-limit").unwrap_or(60);
    let wait = match reset_minutes(headers, now) {
        Some(0) | None => "shortly".to_string(),
        Some(1) => "in ~1 minute".to_string(),
        Some(m) => format!("in ~{m} minutes"),
    };
    let mut msg = format!(
        "GitHub API rate limit reached ({limit} requests/hour, {}). Retry {wait}",
        if authed {
            "authenticated"
        } else {
            "unauthenticated"
        }
    );
    if !authed {
        msg.push_str(", set GITHUB_TOKEN");
    }
    msg.push_str(hint);
    msg.push('.');
    msg
}

#[derive(Deserialize)]
struct TreeResponse {
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    typ: String,
}

/// Download the repository's `presets/` tree (from `main`) into `dest`.
/// Shared by `get-presets`, `check-presets` and `merge-presets`.
///
/// ONE API request covers the whole subtree. The per-directory `contents` walk
/// this replaced cost one request per directory, so a fleet-wide sweep of a
/// dozen estates spent more than the hourly quota on directory listings alone.
/// Blobs come from raw.githubusercontent.com, which is not part of that quota.
pub(crate) async fn download_presets(dest: &Path) -> Result<u32, BoxErr> {
    let http = client("satz-get-presets")?;
    fsx::create_dir_all(dest)?;

    let url = format!("{API_URL}/{REPO}/git/trees/main?recursive=1");
    let resp = api_get(&http, &url).send().await?;
    if !resp.status().is_success() {
        return Err(api_error(
            "Failed to list upstream presets",
            resp.status(),
            resp.headers(),
            PRISTINE_HINT,
        )
        .into());
    }
    let tree: TreeResponse = resp.json().await?;
    if tree.truncated {
        // The API caps a tree response and says so. Half a preset library looks
        // exactly like an upstream that deleted packs, which would read as drift
        // on every estate — walk directory by directory instead.
        return download_presets_via_contents(&http, dest).await;
    }

    let mut count = 0u32;
    for entry in &tree.tree {
        if entry.typ != "blob" {
            continue;
        }
        let Some(rel) = entry.path.strip_prefix("presets/") else {
            continue;
        };
        let raw = format!(
            "https://raw.githubusercontent.com/{REPO}/main/{}",
            entry.path
        );
        let resp = http.get(&raw).send().await?;
        if !resp.status().is_success() {
            return Err(format!("Failed to download {}: {}", entry.path, resp.status()).into());
        }
        let content = resp.bytes().await?;
        write_blob(dest, rel, &content)?;
        count += 1;
    }
    Ok(count)
}

fn write_blob(dest: &Path, rel: &str, content: &[u8]) -> Result<(), BoxErr> {
    let dest_file = dest.join(rel);
    if let Some(p) = dest_file.parent() {
        fsx::create_dir_all(p)?;
    }
    fsx::write(&dest_file, content)?;
    Ok(())
}

/// The pre-trees walk, kept as the fallback for a truncated tree response.
async fn download_presets_via_contents(
    http: &reqwest::Client,
    dest: &Path,
) -> Result<u32, BoxErr> {
    #[derive(Deserialize)]
    struct ContentItem {
        #[serde(rename = "type")]
        typ: String,
        name: String,
        path: String,
        #[serde(default)]
        download_url: Option<String>,
    }

    let mut count = 0u32;
    let mut queue: Vec<(String, PathBuf)> = vec![("presets".to_string(), dest.to_path_buf())];
    while let Some((api_path, local_base)) = queue.pop() {
        let url = format!("{API_URL}/{REPO}/contents/{api_path}?ref=main");
        let resp = api_get(http, &url).send().await?;
        if !resp.status().is_success() {
            return Err(api_error(
                &format!("Failed to list {api_path}"),
                resp.status(),
                resp.headers(),
                PRISTINE_HINT,
            )
            .into());
        }
        let items: Vec<ContentItem> = resp.json().await?;
        for item in items {
            if item.typ == "file" {
                if let Some(download_url) = &item.download_url {
                    let resp = http.get(download_url).send().await?;
                    if !resp.status().is_success() {
                        return Err(
                            format!("Failed to download {}: {}", item.path, resp.status()).into()
                        );
                    }
                    let content = resp.bytes().await?;
                    let dest_file = local_base.join(&item.name);
                    if let Some(p) = dest_file.parent() {
                        fsx::create_dir_all(p)?;
                    }
                    fsx::write(&dest_file, &content)?;
                    count += 1;
                }
            } else if item.typ == "dir" {
                let sub_base = local_base.join(&item.name);
                fsx::create_dir_all(&sub_base)?;
                queue.push((item.path, sub_base));
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn render(status: u16, pairs: &[(&str, &str)], authed: bool) -> String {
        api_error_at(
            "Failed to list upstream presets",
            StatusCode::from_u16(status).unwrap(),
            &headers(pairs),
            PRISTINE_HINT,
            1_000_000,
            authed,
        )
    }

    #[test]
    fn an_exhausted_quota_names_the_limit_the_wait_and_both_ways_out() {
        // The whole point of P5: this used to surface as a serde decode error
        // about "invalid type: map, expected a sequence".
        let msg = render(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-limit", "60"),
                ("x-ratelimit-reset", "1002880"), // 48 minutes out
            ],
            false,
        );
        assert!(msg.contains("rate limit reached (60 requests/hour, unauthenticated)"), "{msg}");
        assert!(msg.contains("Retry in ~48 minutes"), "{msg}");
        assert!(msg.contains("set GITHUB_TOKEN"), "{msg}");
        assert!(msg.contains("--pristine-dir"), "{msg}");
    }

    #[test]
    fn a_token_holder_is_not_told_to_set_one() {
        let msg = render(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-limit", "5000"),
                ("x-ratelimit-reset", "1000060"),
            ],
            true,
        );
        assert!(msg.contains("(5000 requests/hour, authenticated)"), "{msg}");
        assert!(!msg.contains("set GITHUB_TOKEN"), "{msg}");
        assert!(msg.contains("Retry in ~1 minute"), "{msg}");
    }

    #[test]
    fn a_403_that_is_not_the_quota_does_not_claim_it_is() {
        // A private repo or a bad token answers 403 too; telling that user to
        // wait an hour would send them nowhere.
        let msg = render(403, &[("x-ratelimit-remaining", "57")], false);
        assert_eq!(msg, "Failed to list upstream presets: GitHub returned 403 Forbidden");
        let msg = render(404, &[], false);
        assert_eq!(msg, "Failed to list upstream presets: GitHub returned 404 Not Found");
    }

    #[test]
    fn a_secondary_limit_uses_retry_after_and_a_past_reset_says_shortly() {
        let msg = render(
            429,
            &[("x-ratelimit-remaining", "0"), ("retry-after", "90")],
            false,
        );
        assert!(msg.contains("Retry in ~2 minutes"), "{msg}");
        // A reset already in the past must not underflow into a huge wait.
        let msg = render(
            403,
            &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "999000")],
            false,
        );
        assert!(msg.contains("Retry shortly"), "{msg}");
    }
}
