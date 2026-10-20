//! Self-update from GitHub releases: check the latest release, download the
//! `TextGlow-<tag>.exe` asset, swap it over the running binary, and relaunch.
//!
//! Windows lets a running exe be renamed (but not deleted or overwritten), so
//! the swap is: rename the current exe aside, move the staged download into
//! place, spawn the new exe with `--updated` (it waits for the single-instance
//! mutex this dying process still holds), then exit. The next start deletes
//! the `.old` file left behind.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use serde::Deserialize;

/// A download smaller than this is not a real build of this app.
const MIN_EXE_BYTES: u64 = 1024 * 1024;

const LATEST_API: &str = "https://api.github.com/repos/jaltez/textglow/releases/latest";

/// This build's version tag. Release builds get the git tag baked in via
/// `TEXTGLOW_VERSION` (see release.yml); dev builds fall back to Cargo's version.
pub fn current_version() -> &'static str {
    match option_env!("TEXTGLOW_VERSION") {
        Some(v) if !v.is_empty() => v,
        _ => concat!("v", env!("CARGO_PKG_VERSION")),
    }
}

/// An installable release newer than the running build.
#[derive(Debug, Clone)]
pub struct Release {
    /// Git tag, e.g. "v0.2.0"; also the display version.
    pub version: String,
    pub download_url: String,
    /// Expected asset size in bytes, from the release metadata.
    pub size: u64,
}

/// Strictly-newer comparison of dotted versions ("v0.2.10" > "v0.2.9").
/// Non-numeric components count as 0; a shorter list is padded with zeros.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    fn numbers(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    }
    let cur = numbers(current);
    let new = numbers(candidate);
    for i in 0..cur.len().max(new.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let n = new.get(i).copied().unwrap_or(0);
        if n != c {
            return n > c;
        }
    }
    false
}

/// The asset for this exact tag, or any TextGlow-*.exe fallback.
fn pick_asset<'a>(tag: &str, assets: &'a [GhAsset]) -> Option<&'a GhAsset> {
    let exact = format!("TextGlow-{tag}.exe");
    assets.iter().find(|a| a.name == exact).or_else(|| {
        assets
            .iter()
            .find(|a| a.name.starts_with("TextGlow-") && a.name.ends_with(".exe"))
    })
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// Ask GitHub whether a newer release exists. `Ok(None)` = up to date (also
/// when no releases are published yet).
pub fn spawn_check() -> Receiver<Result<Option<Release>, String>> {
    let (tx, rx) = mpsc::channel();
    crate::llm::runtime().spawn(async move {
        let _ = tx.send(check().await);
    });
    rx
}

async fn check() -> Result<Option<Release>, String> {
    let resp = crate::llm::client()
        .get(LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("connection error: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // no releases published yet
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let rel: GhRelease = resp
        .json()
        .await
        .map_err(|e| format!("invalid release metadata: {e}"))?;
    if !is_newer(current_version(), &rel.tag_name) {
        return Ok(None);
    }
    let Some(asset) = pick_asset(&rel.tag_name, &rel.assets) else {
        return Err("the new release has no TextGlow-*.exe asset".into());
    };
    Ok(Some(Release {
        version: rel.tag_name,
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
    }))
}

/// Download the release asset and stage it next to the running exe.
pub fn spawn_download(rel: Release) -> Receiver<Result<(Release, PathBuf), String>> {
    let (tx, rx) = mpsc::channel();
    crate::llm::runtime().spawn(async move {
        let _ = tx.send(download(rel).await);
    });
    rx
}

async fn download(rel: Release) -> Result<(Release, PathBuf), String> {
    let resp = crate::llm::client()
        .get(&rel.download_url)
        .send()
        .await
        .map_err(|e| format!("connection error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} while downloading", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if bytes.starts_with(b"MZ") && (bytes.len() as u64) < MIN_EXE_BYTES {
        return Err("downloaded file is suspiciously small — try again".into());
    }
    if !bytes.starts_with(b"MZ") {
        return Err("downloaded file is not a Windows executable".into());
    }
    if bytes.len() as u64 != rel.size {
        return Err(format!(
            "downloaded file has the wrong size (got {}, expected {})",
            bytes.len(),
            rel.size
        ));
    }
    let staged = staging_path()?;
    std::fs::write(&staged, &bytes).map_err(|e| format!("cannot write the update: {e}"))?;
    Ok((rel, staged))
}

/// Where the downloaded exe waits until it is swapped into place. Same
/// directory as the running exe, so the final swap is a same-volume rename.
fn staging_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate own exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "own exe has no directory".to_string())?;
    Ok(dir.join("textglow.exe.new"))
}

/// Swap the staged download over the running exe and spawn the new build with
/// `--updated`. The caller exits right after; the child waits for the
/// single-instance mutex this process still holds.
pub fn apply_and_restart(staged: &PathBuf) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("own exe has no directory"))?;
    let old = dir.join("textglow.exe.old");
    let _ = std::fs::remove_file(&old);
    // Renaming a running exe is allowed; overwriting it is not.
    std::fs::rename(&exe, &old)?;
    if let Err(e) = std::fs::rename(staged, &exe) {
        // Roll back so the current install stays launchable.
        let _ = std::fs::rename(&old, &exe);
        return Err(anyhow::anyhow!("cannot move the update into place: {e}"));
    }
    std::process::Command::new(&exe)
        .arg("--updated")
        .spawn()
        .map_err(|e| anyhow::anyhow!("cannot start the new version: {e}"))?;
    Ok(())
}

/// Best-effort: delete the binary a previous update renamed aside. The delete
/// can fail while that old process is still exiting; the next start retries.
pub fn cleanup_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::fs::remove_file(dir.join("textglow.exe.old"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_compare_numerically() {
        assert!(is_newer("v0.1.3", "v0.2.0"));
        assert!(is_newer("v0.1.3", "v0.1.10")); // 10 > 9; string compare would fail
        assert!(is_newer("v0.1.3", "v1.0.0"));
        assert!(!is_newer("v0.1", "v0.1.0")); // padded to equal, not newer
        assert!(!is_newer("v0.2.0", "v0.2.0"));
        assert!(!is_newer("v0.2.0", "v0.1.3"));
        assert!(!is_newer("v0.2.0", "v0.1.99"));
        assert!(!is_newer("v0.1.3", "nonsense"));
        assert!(!is_newer("v0.1.3", ""));
    }

    #[test]
    fn assets_prefer_exact_tag_name() {
        let mk = |name: &str| GhAsset {
            name: name.into(),
            size: 1,
            browser_download_url: "https://x".into(),
        };
        let assets = vec![
            mk("TextGlow-v0.1.0.exe"),
            mk("TextGlow-v0.2.0.exe"),
            mk("notes.txt"),
        ];
        assert_eq!(pick_asset("v0.2.0", &assets).unwrap().name, "TextGlow-v0.2.0.exe");
        // No exact match: first TextGlow-*.exe in list order wins.
        assert_eq!(pick_asset("v9.9.9", &assets).unwrap().name, "TextGlow-v0.1.0.exe");
        let empty: Vec<GhAsset> = vec![];
        assert!(pick_asset("v9.9.9", &empty).is_none());
    }

    #[test]
    fn version_string_is_tag_shaped() {
        assert!(current_version().starts_with('v'));
    }
}
