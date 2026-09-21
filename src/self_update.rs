//! Keeping the binary current, and opening what it points at.
//!
//! `satz self-update` asks the GitHub releases API for the latest version, verifies the
//! installer against the SHA-256 the release publishes, and runs it. The background check
//! `maybe_check_for_updates` is the same query without the install, run per the frequency in
//! `~/.config/satz/satz.toml` and never for a command that owns stdout as a protocol.

use crate::fsx;
use crate::github::{api_error, api_get, API_URL, DOCS_URL, REPO};
use crate::settings::GlobalSettings;
use crate::{Commands};
use crate::settings::{save_global_settings};
use serde::Deserialize;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

/// Fetches latest release from GitHub and returns (latest_version, html_url) if an update is available.
pub(crate) async fn check_update_available(client: &reqwest::Client) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
    let url = format!("{}/{}/releases/latest", API_URL, REPO);
    let response = api_get(client, &url).send().await?;
    if !response.status().is_success() {
        // Surfaced, not swallowed: a 404 here once hid a repo that had no
        // releases at all. The caller prints it and continues with the user's command.
        return Err(api_error("Update check failed", response.status(), response.headers(), "").into());
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
    }
    let release: Release = response.json().await?;
    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let current = env!("CARGO_PKG_VERSION");
    if compare_versions(current, &latest_version)? < 0 {
        Ok(Some((latest_version, release.html_url)))
    } else {
        Ok(None)
    }
}

/// If global settings say so, run a check-only update check and optionally persist last_update_check (daily).
/// Whether a command runs the background update check. Not the update itself, not
/// `init`, not `whoami` — and not the two protocol servers: `mcp` and `lsp` speak
/// JSON-RPC on stdout and are started by a client, so a network round trip to
/// GitHub before the first message is a cost with no reader, and the notice would
/// have nobody to read it but the client's parser.
pub(crate) fn checks_for_updates(cmd: &Commands) -> bool {
    !matches!(
        cmd,
        Commands::SelfUpdate { .. } | Commands::Init { .. } | Commands::Whoami { .. } | Commands::Mcp { .. } | Commands::Lsp
    )
}

pub(crate) async fn maybe_check_for_updates(settings: &mut GlobalSettings) -> Result<(), Box<dyn std::error::Error>> {
    let freq = settings.self_update_frequency.as_str();
    if freq == "never" {
        return Ok(());
    }
    if freq == "daily" {
        if let Some(ref last) = settings.last_update_check {
            let last_ts: u64 = last.parse().unwrap_or(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now.saturating_sub(last_ts) < 86400 {
                return Ok(());
            }
        }
    }
    let client = reqwest::Client::builder()
        .user_agent("satz-update-checker")
        .build()?;
    let update = match check_update_available(&client).await {
        Ok(update) => update,
        Err(e) => {
            eprintln!("⚠️  {} (set self_update_frequency = \"never\" in ~/.config/satz/satz.toml to silence)", e);
            None
        }
    };
    if freq == "daily" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        settings.last_update_check = Some(now.to_string());
        save_global_settings(settings)?;
    }
    if let Some((version, url)) = update {
        // stderr: stdout is a command's output — the JSON of `--format json`, or the
        // protocol of `mcp` and `lsp` — and a notice in front of it breaks every reader.
        eprintln!("⚠️  Update available: {} (current: {}). Run `satz self-update` to install. {}", version, env!("CARGO_PKG_VERSION"), url);
    }
    Ok(())
}

/// The installer verifies the archive it downloads with `sha256sum`, and SKIPS that check —
/// printing one line — when the command is not on PATH, which is the case on macOS before
/// 14 and on a minimal Linux. The installer script itself is verified here against its
/// sidecar; the archive is the installer's business. So where `sha256sum` is missing and
/// `shasum` is present, a shim that runs `shasum -a 256` goes first on the installer's PATH
/// (`Some(PATH)`), and where both are missing the update is refused unless the operator
/// asked to skip checksums. `None`: the PATH as it is.
#[cfg(unix)]
pub(crate) fn installer_checksum_path(
    temp_dir: &Path,
    path: &std::ffi::OsStr,
    skip_checksum: bool,
) -> Result<Option<std::ffi::OsString>, String> {
    use std::os::unix::fs::PermissionsExt;
    let on_path = |name: &str| {
        std::env::split_paths(path).any(|d| {
            std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    };
    if on_path("sha256sum") {
        return Ok(None);
    }
    if on_path("shasum") {
        let bin = temp_dir.join("checksum-bin");
        std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {}", bin.display(), e))?;
        let shim = bin.join("sha256sum");
        fsx::write(&shim, b"#!/bin/sh\nexec shasum -a 256 \"$@\"\n").map_err(|e| e.to_string())?;
        fsx::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        let mut dirs = vec![bin];
        dirs.extend(std::env::split_paths(path));
        return std::env::join_paths(dirs).map(Some).map_err(|e| e.to_string());
    }
    if skip_checksum {
        eprintln!("⚠️  neither sha256sum nor shasum is on PATH: the installer cannot verify the archive (--skip-checksum)");
        return Ok(None);
    }
    Err("neither sha256sum nor shasum is on PATH, so the installer cannot verify the archive it downloads — \
         install one (coreutils provides sha256sum), or re-run with --skip-checksum to update without the check"
        .to_string())
}

// On Windows the update is refused before any download, so the installer's options are
// never read there.
#[cfg_attr(windows, allow(unused_variables))]
pub(crate) async fn run_self_update( open_docs: bool, check_only: bool, skip_checksum: bool) -> Result<(), Box<dyn std::error::Error>> {

    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", current_version);

    let client = reqwest::Client::builder()
        .user_agent("satz-update-checker")
        .build()?;

    let url = format!("{}/{}/releases/latest", API_URL, REPO);
    let response = api_get(&client, &url).send().await?;

    if !response.status().is_success() {
        // `self-update` shares the 60/hour quota with the preset commands, so a
        // fleet-wide sweep can be what actually stopped this. Say which.
        return Err(api_error(
            "Failed to fetch release info",
            response.status(),
            response.headers(),
            "",
        )
        .into());
    }

    // the assets are read by the installer path alone, which is unix-only
    #[derive(Deserialize)]
    #[cfg_attr(windows, allow(dead_code))]
    struct Asset {
        name: String,
        browser_download_url: String,
    }

    #[derive(Deserialize)]
    #[cfg_attr(windows, allow(dead_code))]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        assets: Vec<Asset>,
    }

    let release: Release = response.json().await?;
    let latest_version = release.tag_name.trim_start_matches('v');
    println!("Latest version: {}", latest_version);

    if compare_versions(current_version, latest_version)? < 0 {
        println!("\n⚠️  A new version is available!");
        println!("   Current: {}", current_version);
        println!("   Latest:  {}", latest_version);
        println!("   Release: {}", release.html_url);
        if check_only {
            println!("\nRun `satz self-update` to install.");
            return Ok(());
        }
        // The release's installer is a shell script. Refused before anything is
        // downloaded, rather than after a download and a checksum it then cannot run.
        #[cfg(windows)]
        {
            return Err(format!(
                "self-update installs through a shell script and does not run on Windows — install {} with PowerShell:\n  \
                 powershell -ExecutionPolicy Bypass -c \"irm https://github.com/{}/releases/latest/download/satz-installer.ps1 | iex\"",
                latest_version, REPO
            )
            .into());
        }
        // everything below runs the release's shell installer, which Windows cannot
        #[cfg(unix)]
        {
            println!("\n📥 Installing update...");

            // Installer and sidecar both come from THIS release object, never from
            // `/releases/latest/download/` — a release published between the API
            // call and the download would otherwise pair one release's installer
            // with another's checksum.
            let installer_asset = release.assets.iter()
                .find(|a| a.name == "satz-installer.sh")
                .ok_or_else(|| format!(
                    "Release {} has no satz-installer.sh asset — the release build did not finish. Aborting.",
                    release.html_url
                ))?;

            // Download installer as bytes for checksum verification
            let installer_bytes = client
                .get(&installer_asset.browser_download_url)
                .send()
                .await?
                .error_for_status()
                .map_err(|e| format!("installer download failed: {}", e))?
                .bytes()
                .await?;

            // Checksum verification
            let checksum_asset = release.assets.iter()
                .find(|a| a.name == "satz-installer.sh.sha256");
            match checksum_asset {
                Some(asset) => {
                    let expected_raw = client
                        .get(&asset.browser_download_url)
                        .send()
                        .await?
                        .error_for_status()
                        .map_err(|e| format!("checksum download failed: {}", e))?
                        .text()
                        .await?;
                    let expected = expected_raw.split_whitespace().next().unwrap_or("").to_lowercase();
                    use sha2::{Digest, Sha256};
                    let actual = hex::encode(Sha256::digest(&installer_bytes));
                    if actual != expected {
                        return Err(format!(
                            "Checksum mismatch — installer may have been tampered with.\n\
                             Expected: {}\n\
                             Got:      {}\n\
                             Aborting. Download the release manually from {}",
                            expected, actual, release.html_url
                        ).into());
                    }
                    println!("✅ Checksum verified");
                }
                None if skip_checksum => {
                    eprintln!(
                        "⚠️  No checksum file found in this release. \
                         Proceeding without verification (--skip-checksum)."
                    );
                }
                None => {
                    return Err(
                        "No checksum file (satz-installer.sh.sha256) found in this release.\n\
                         Cannot verify installer integrity. Aborting.\n\
                         If you are confident in the download, re-run with --skip-checksum."
                        .into()
                    );
                }
            }

            // Write to temp file and execute
            // a private, unpredictable directory: on a shared /tmp a pre-created
            // file at a guessable path could be swapped between write and run
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let temp_dir = std::env::temp_dir().join(format!("satz-self-update-{}-{}", std::process::id(), nonce));
            {
                let mut b = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    b.mode(0o700);
                }
                b.create(&temp_dir).map_err(|e| format!("{}: {}", temp_dir.display(), e))?;
            }
            let temp_file = temp_dir.join("satz-installer.sh");
            fsx::write(&temp_file, &installer_bytes)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fsx::set_permissions(&temp_file, std::fs::Permissions::from_mode(0o755))?;

                let path_for_installer = match installer_checksum_path(
                    &temp_dir,
                    &std::env::var_os("PATH").unwrap_or_default(),
                    skip_checksum,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&temp_dir);
                        return Err(e.into());
                    }
                };
                let mut installer = std::process::Command::new("sh");
                installer.arg(&temp_file);
                if let Some(path) = path_for_installer {
                    installer.env("PATH", path);
                }
                let status = installer.status()?;
                let _ = std::fs::remove_dir_all(&temp_dir);

                if status.success() {
                    println!("✅ Update installed successfully!");
                    println!("   Please restart your terminal or run: source ~/.profile");

                    println!("   Documentation: {}", DOCS_URL);
                    if open_docs {
                        open_url(DOCS_URL)?;
                    }
                } else {
                    return Err("Failed to run installer script".into());
                }
            }
        }

    } else {
        println!("✅ You are running the latest version!");
    }

    Ok(())
}

/// Open a URL in the default browser (never an editor).
pub(crate) fn open_url(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Opening {}", url);
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let status: std::io::Result<std::process::ExitStatus> = Err(std::io::Error::other("no browser opener on this platform"));
    match status {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => Err(format!("could not open {}: the opener exited with {}", url, st).into()),
        Err(e) => Err(format!("could not open {}: {}", url, e).into()),
    }
}


/// Ordering of two dotted numeric versions. A component that is not a number
/// (`0.46.15-rc1`, a malformed tag) is an error — read as 0 it would call a
/// newer release "older" and print "you are on the latest version".
pub(crate) fn compare_versions(v1: &str, v2: &str) -> Result<i32, String> {
    let parse_version = |v: &str| -> Result<Vec<u32>, String> {
        v.split('.')
            .map(|s| s.parse::<u32>().map_err(|_| format!("version `{}` has a non-numeric component `{}`", v, s)))
            .collect()
    };
    let v1_parts = parse_version(v1)?;
    let v2_parts = parse_version(v2)?;
    let max_len = v1_parts.len().max(v2_parts.len());
    for i in 0..max_len {
        let v1_val = v1_parts.get(i).copied().unwrap_or(0);
        let v2_val = v2_parts.get(i).copied().unwrap_or(0);
        if v1_val < v2_val {
            return Ok(-1);
        }
        if v1_val > v2_val {
            return Ok(1);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod checksum_tests {
    /// The self-update path verifies the downloaded installer against the release's
    /// `.sha256` asset (see `run_self_update`). Pin the digest to known vectors so a `sha2` major
    /// upgrade cannot silently change what that comparison computes — a wrong hash here
    /// either blocks every update or, worse, passes something it should not.
    #[test]
    fn sha256_matches_known_vectors() {
        use sha2::{Digest, Sha256};

        assert_eq!(
            hex::encode(Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex::encode(Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

#[cfg(all(test, unix))]
mod checksum_path_tests {
    //! The installer skips its archive check when `sha256sum` is not on PATH, which
    //! was the case on macOS before 14. `self-update` makes the check run wherever
    //! `shasum` exists, and refuses where neither does.
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir_with(name: &str, tools: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("satz-checksum-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (tool, body) in tools {
            let p = d.join(tool);
            fsx::write(&p, body.as_bytes()).unwrap();
            fsx::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
    }

    #[test]
    fn with_sha256sum_the_path_stays_as_it_is() {
        let tools = dir_with("has", &[("sha256sum", "#!/bin/sh\n")]);
        let temp = dir_with("has-temp", &[]);
        assert_eq!(installer_checksum_path(&temp, tools.as_os_str(), false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&tools);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn with_only_shasum_a_shim_runs_it_with_the_installer_s_arguments() {
        let tools = dir_with("shasum", &[("shasum", "#!/bin/sh\necho \"shasum $*\"\n")]);
        let temp = dir_with("shasum-temp", &[]);
        let path = installer_checksum_path(&temp, tools.as_os_str(), false).unwrap().expect("a PATH with the shim");
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, temp.join("checksum-bin"));
        // exactly the installer's call: `sha256sum -b "$_file" | awk '{printf $1}'`
        // `/bin/sh` by path: the PATH under test holds only the two tool directories
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", "sha256sum -b archive.tar.xz"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "shasum -a 256 -b archive.tar.xz");
        let _ = std::fs::remove_dir_all(&tools);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn with_neither_the_update_is_refused_unless_the_check_is_skipped() {
        let empty = dir_with("none", &[]);
        let temp = dir_with("none-temp", &[]);
        let err = installer_checksum_path(&temp, empty.as_os_str(), false).unwrap_err();
        assert!(err.contains("cannot verify the archive") && err.contains("--skip-checksum"), "{err}");
        assert_eq!(installer_checksum_path(&temp, empty.as_os_str(), true).unwrap(), None);
        let _ = std::fs::remove_dir_all(&empty);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
