//! Everything this tool asks of github.com, in one place: the satz repository and the
//! repositories config.toml names as pack sources or as the preset library, the client
//! that talks to them, and — the reason this module exists — a refusal rendered as
//! something a human can act on: a rate limit, or a private repository without a sign-in.
//!
//! The unauthenticated REST quota is 60 requests/hour and is SHARED with
//! `self-update`, so a preset sweep across a fleet can lock the same user out of
//! updating their binary. That makes both halves here load-bearing: ask for as
//! little as possible, and when the quota does run out, say so.

use std::path::Path;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
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
/// The documentation site — README, language reference and preset docs,
/// rendered from this repository's Markdown on every release tag
/// (`.github/workflows/pages.yml`).
pub(crate) const DOCS_URL: &str = "https://tjirsch.github.io/satz/";
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
        Some(t) => bearer(req, &t),
        None => req,
    }
}

/// The token as a sensitive `Authorization` header. A token with bytes a header cannot
/// carry is not worth failing the whole command over — the request goes without it,
/// and the refusal it meets says what to do.
fn bearer(req: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    match HeaderValue::from_str(&format!("Bearer {token}")) {
        Ok(mut v) => {
            v.set_sensitive(true);
            req.header(AUTHORIZATION, v)
        }
        Err(_) => req,
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
    #[serde(default)]
    sha: String,
}

/// A GitHub repository satz reads files from: the satz repository, or one config.toml
/// names as a pack source or as the preset library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repo {
    /// `owner/repository`
    pub name: String,
    /// the branch or tag; the repository's default branch when none
    pub git_ref: Option<String>,
    /// read with this machine's GitHub sign-in: a repository config.toml names, which may
    /// be private. The satz repository is public and is read without it.
    pub signed_in: bool,
}

impl Repo {
    pub(crate) fn satz() -> Repo {
        Repo { name: REPO.to_string(), git_ref: Some(crate::pack_sources::BUILTIN_REF.to_string()), signed_in: false }
    }

    pub(crate) fn configured(name: &str, git_ref: Option<&str>) -> Repo {
        Repo { name: name.to_string(), git_ref: git_ref.map(str::to_string), signed_in: true }
    }

    fn token(&self) -> Option<&'static str> {
        if self.signed_in {
            machine_token()
        } else {
            None
        }
    }

    /// An API request: with this machine's sign-in for a repository config.toml names,
    /// otherwise as [`api_get`] sends it.
    fn api(&self, http: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
        match self.token() {
            Some(t) => bearer(http.get(url), t),
            None => api_get(http, url),
        }
    }

    /// A refused request, said so that the reader knows what to do. A repository that
    /// answers 404 or 403 without a sign-in may be private, and `gh auth login` is the
    /// way in; with one, the sign-in has no access.
    fn refusal(&self, what: &str, status: StatusCode, headers: &HeaderMap, hint: &str) -> String {
        let exhausted = matches!(status.as_u16(), 403 | 429) && header_u64(headers, "x-ratelimit-remaining") == Some(0);
        if exhausted || !self.signed_in {
            return api_error(what, status, headers, hint);
        }
        match status.as_u16() {
            401 | 403 | 404 if self.token().is_none() => format!(
                "{what}: GitHub answered {status} — github.com/{} does not exist, or it is private and this machine has no GitHub sign-in: run `gh auth login`, then run this again",
                self.name
            ),
            401 | 403 | 404 => format!(
                "{what}: GitHub answered {status} — github.com/{} does not exist, or the GitHub sign-in of this machine cannot read it (`gh auth status` says who is signed in)",
                self.name
            ),
            422 => format!("{what}: github.com/{} has no branch or tag `{}`", self.name, self.git_ref.as_deref().unwrap_or("HEAD")),
            _ => api_error(what, status, headers, hint),
        }
    }

    /// The commit the branch or tag points at now — the default branch's without one.
    /// One API request.
    pub(crate) async fn commit(&self, http: &reqwest::Client, hint: &str) -> Result<String, String> {
        let at = self.git_ref.as_deref().unwrap_or("HEAD");
        let what = format!("Failed to read github.com/{} at {at}", self.name);
        let url = format!("{API_URL}/{}/commits/{at}", self.name);
        let resp = self.api(http, &url).header(ACCEPT, "application/vnd.github.sha").send().await.map_err(|e| format!("{what}: {e}"))?;
        if !resp.status().is_success() {
            return Err(self.refusal(&what, resp.status(), resp.headers(), hint));
        }
        let sha = resp.text().await.map_err(|e| format!("{what}: {e}"))?.trim().to_string();
        if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{what}: GitHub answered with no commit"));
        }
        Ok(sha)
    }

    /// Every file under `prefix/` at `commit`, each as its path from the repository's
    /// top. One API request; a listing GitHub cuts short is read again from the folder
    /// down, and refused when that is cut short too, since half a listing reads as files
    /// the repository deleted.
    pub(crate) async fn files(&self, http: &reqwest::Client, commit: &str, prefix: &str, hint: &str) -> Result<Vec<String>, String> {
        let what = format!("Failed to list {prefix}/ of github.com/{}", self.name);
        let tree = self.tree(http, commit, true, &what, hint).await?;
        let under = format!("{prefix}/");
        if !tree.truncated {
            return Ok(tree.tree.into_iter().filter(|e| e.typ == "blob" && e.path.starts_with(&under)).map(|e| e.path).collect());
        }
        // walk down to the folder's own tree, one listing per segment, and list that
        let mut sha = commit.to_string();
        for segment in prefix.split('/') {
            let level = self.tree(http, &sha, false, &what, hint).await?;
            match level.tree.into_iter().find(|e| e.typ == "tree" && e.path == segment) {
                Some(e) => sha = e.sha,
                None => return Ok(Vec::new()),
            }
        }
        let tree = self.tree(http, &sha, true, &what, hint).await?;
        if tree.truncated {
            return Err(format!("GitHub cut the file listing of {prefix}/ in github.com/{} short, so the files it holds cannot be listed{hint}", self.name));
        }
        Ok(tree.tree.into_iter().filter(|e| e.typ == "blob").map(|e| format!("{under}{}", e.path)).collect())
    }

    async fn tree(&self, http: &reqwest::Client, sha: &str, recursive: bool, what: &str, hint: &str) -> Result<TreeResponse, String> {
        let url = format!("{API_URL}/{}/git/trees/{sha}{}", self.name, if recursive { "?recursive=1" } else { "" });
        let resp = self.api(http, &url).send().await.map_err(|e| format!("{what}: {e}"))?;
        if !resp.status().is_success() {
            return Err(self.refusal(what, resp.status(), resp.headers(), hint));
        }
        resp.json().await.map_err(|e| format!("{what}: {e}"))
    }

    /// One file at `commit`, or `None` where the repository holds no such file. Without a
    /// sign-in it comes from raw.githubusercontent.com, which costs no API request; with
    /// one, through the API, which reads a private repository.
    pub(crate) async fn file(&self, http: &reqwest::Client, commit: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        let what = format!("Failed to read {path} from github.com/{}", self.name);
        let req = match self.token() {
            Some(t) => bearer(http.get(format!("{API_URL}/{}/contents/{path}?ref={commit}", self.name)), t).header(ACCEPT, "application/vnd.github.raw"),
            None => http.get(format!("https://raw.githubusercontent.com/{}/{commit}/{path}", self.name)),
        };
        let resp = req.send().await.map_err(|e| format!("{what}: {e}"))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(self.refusal(&what, resp.status(), resp.headers(), ""));
        }
        resp.bytes().await.map(|b| Some(b.to_vec())).map_err(|e| format!("{what}: {e}"))
    }
}

/// The GitHub sign-in of this machine: what `gh auth token` prints, else `GITHUB_TOKEN`.
/// Asked once per process and never written anywhere.
fn machine_token() -> Option<&'static str> {
    static TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TOKEN
        .get_or_init(|| {
            let gh = std::process::Command::new("gh")
                .args(["auth", "token", "--hostname", "github.com"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            if let Ok(out) = gh {
                let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() && !t.is_empty() {
                    return Some(t);
                }
            }
            token()
        })
        .as_deref()
}

/// Download the `presets/` folder of `repo` into `dest`, the files under it at their
/// paths below it. Shared by `get-presets`, `check-presets` and `merge-presets`. Two API
/// requests — the commit and the listing; the files come from raw.githubusercontent.com,
/// which is not part of that quota, or through the API for a repository read with the
/// machine's sign-in.
pub(crate) async fn download_presets(repo: &Repo, dest: &Path) -> Result<u32, String> {
    let http = client("satz-get-presets").map_err(|e| e.to_string())?;
    fsx::create_dir_all(dest).map_err(|e| e.to_string())?;
    let commit = repo.commit(&http, PRISTINE_HINT).await?;
    let blobs = repo.files(&http, &commit, "presets", PRISTINE_HINT).await?;
    // A hundred-odd files one request at a time, with nothing printed until the last
    // one landed, looked exactly like a hang. Say what is happening, once, and fetch
    // several at a time.
    // stderr: `satz_check_presets` reaches this, and over MCP stdout is the protocol.
    eprintln!("fetching {} preset file(s) from github.com/{} at {} …", blobs.len(), repo.name, &commit[..12]);
    let progress = std::sync::Arc::new(Progress::new(blobs.len()));

    use futures_util::stream::{self, StreamExt, TryStreamExt};
    // owned: the downloads run concurrently inside a future the MCP tools must be
    // able to send, and borrowed entries make that future not general enough
    let fetched: Vec<()> = stream::iter(blobs.into_iter().map(|path| {
        let http = http.clone();
        let progress = progress.clone();
        let dest = dest.to_path_buf();
        let repo = repo.clone();
        let commit = commit.clone();
        async move {
            let content = repo.file(&http, &commit, &path).await?.ok_or_else(|| format!("Failed to download {path}: GitHub listed it and holds no such file"))?;
            let rel = path.strip_prefix("presets/").unwrap_or(&path);
            write_blob(&dest, rel, &content).map_err(|e| e.to_string())?;
            progress.tick();
            Ok::<(), String>(())
        }
    }))
    .buffer_unordered(8)
    .try_collect()
    .await?;
    progress.done();
    Ok(fetched.len() as u32)
}

/// One line that counts, on a terminal; nothing per file anywhere else, because a log
/// or an MCP client's stderr is not a place for a hundred carriage returns.
struct Progress {
    total: usize,
    done: std::sync::atomic::AtomicUsize,
    live: bool,
}

impl Progress {
    fn new(total: usize) -> Self {
        use std::io::IsTerminal;
        Progress { total, done: std::sync::atomic::AtomicUsize::new(0), live: std::io::stderr().is_terminal() }
    }

    fn tick(&self) {
        let n = self.done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if self.live {
            eprint!("\r  {n}/{}", self.total);
        }
    }

    fn done(&self) {
        if self.live {
            eprintln!();
        }
    }
}

/// Appended to a quota message by the workload-pack commands, which can take their
/// packs from a folder instead.
pub(crate) const PACKS_HINT: &str = ", or take the packs from a local checkout: `satz pack-source add <name> --dir <checkout>/packs`";

fn write_blob(dest: &Path, rel: &str, content: &[u8]) -> Result<(), BoxErr> {
    let dest_file = dest.join(rel);
    if let Some(p) = dest_file.parent() {
        fsx::create_dir_all(p)?;
    }
    fsx::write_verbatim(&dest_file, content)?;
    Ok(())
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
