//! Checking GitHub Releases for a newer build, and installing one on request.
//!
//! The app has no signed update channel: releases are published by a tag push and
//! every one of them is a pre-release, so `/releases/latest` (which skips them) is
//! the wrong endpoint and the version numbers have to be compared here.
//!
//! Nothing here runs on startup on its own. The library asks once per launch
//! through [`update_check`], which answers from a cached result most of the time.

use std::io::Write as _;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// Where releases are listed.
const RELEASES_URL: &str = "https://api.github.com/repos/Atho64/VN-Asyik/releases?per_page=20";
/// How long a check is trusted. GitHub's anonymous quota is 60 requests an hour.
const CACHE_MS: i64 = 6 * 60 * 60 * 1000;
/// The version compiled into this build.
const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Key the cached answer lives under in `sync_state`.
const CACHE_KEY: &str = "update.cache";

/// Current UTC epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(0))
}

/// One downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// File name, as GitHub spells it.
    pub name: String,
    /// Download URL.
    pub url: String,
    /// Size in bytes, when GitHub reports one.
    pub size: Option<u64>,
}

/// What the UI needs to decide whether to show an update button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// Version of this build.
    pub current: String,
    /// Highest published version, when one could be read.
    pub latest: Option<String>,
    /// Whether `latest` is newer than `current`.
    pub available: bool,
    /// Release page, for reading what changed.
    pub url: Option<String>,
    /// The installer this platform would use.
    pub asset: Option<Asset>,
    /// When the answer was fetched, in UTC epoch milliseconds.
    pub checked_at: Option<i64>,
    /// Why the check failed, when it did. A failure is not an error dialog: the
    /// library keeps working and simply shows no update button.
    pub error: Option<String>,
    /// Whether this build can install the download itself.
    pub can_install: bool,
}

/// Cached part of the answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cache {
    /// When the check ran.
    checked_at: i64,
    /// Highest version seen, tag without the leading `v`.
    latest: Option<String>,
    /// Release page.
    url: Option<String>,
    /// Windows installer, when the release carries one.
    setup: Option<Asset>,
    /// Android package, when the release carries one.
    apk: Option<Asset>,
}

/// The shape of one GitHub release, as far as this needs it.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Compare two dotted versions numerically.
///
/// Any suffix (`-beta.1`) is ignored, because every release here is a pre-release
/// and the tag is what the user sees. A non-numeric segment counts as 0 rather
/// than aborting the comparison.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |text: &str| -> Vec<u64> {
        text.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (left, right) = (parse(candidate), parse(current));
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// Read the cached answer, when it is still fresh.
async fn cached(pool: &sqlx::SqlitePool, now: i64) -> Option<Cache> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM sync_state WHERE key = ?1")
        .bind(CACHE_KEY)
        .fetch_optional(pool)
        .await
        .ok()?;
    let cache: Cache = serde_json::from_str(row?.0.as_str()).ok()?;
    (now - cache.checked_at < CACHE_MS).then_some(cache)
}

/// Store the answer for the next launch.
async fn remember(pool: &sqlx::SqlitePool, cache: &Cache) {
    let Ok(json) = serde_json::to_string(cache) else {
        return;
    };
    let _ = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(CACHE_KEY)
    .bind(json)
    .execute(pool)
    .await;
}

/// Ask GitHub what the newest release is.
async fn fetch(client: &reqwest::Client) -> Result<Cache, String> {
    let response = client
        .get(RELEASES_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, "VN-Asyik")
        .send()
        .await
        .map_err(|_| "offline")?;
    match response.status().as_u16() {
        200..=299 => {}
        403 | 429 => return Err("rate_limited".into()),
        _ => return Err("provider_error".into()),
    }
    let releases: Vec<Release> = response.json().await.map_err(|_| "provider_error")?;
    let newest = releases
        .into_iter()
        .filter(|release| !release.draft)
        .map(|release| (release.tag_name.trim_start_matches('v').to_owned(), release))
        .fold(
            None,
            |held: Option<(String, Release)>, candidate| match held {
                Some((version, release)) if !is_newer(&candidate.0, &version) => {
                    Some((version, release))
                }
                _ => Some(candidate),
            },
        );
    let Some((version, release)) = newest else {
        return Err("no_release".into());
    };
    let pick = |predicate: &dyn Fn(&str) -> bool| -> Option<Asset> {
        release
            .assets
            .iter()
            .find(|asset| predicate(&asset.name))
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                size: asset.size,
            })
    };
    Ok(Cache {
        checked_at: 0,
        latest: Some(version),
        url: release.html_url,
        // Extension checks go through `Path`, as the download does with the name,
        // and ignore case: a release asset may be spelled `.APK`.
        setup: pick(&|name| has_extension(name, &["exe"])),
        apk: pick(&|name| has_extension(name, &["apk"])),
    })
}

/// Whether this build may launch the downloaded installer itself.
const fn can_install() -> bool {
    cfg!(any(target_os = "windows", target_os = "android"))
}

/// Report the newest published version.
///
/// # Errors
/// Database failures only. A failed check is reported in `error`, not raised: a
/// network problem must not become an error dialog on a library screen.
#[tauri::command]
pub async fn update_check(state: State<'_, AppState>, force: bool) -> CommandResult<UpdateView> {
    let pool = state.db().pool();
    let now = now_ms();
    let mut cache = match cached(pool, now).await {
        Some(cache) if !force => Some(cache),
        _ => None,
    };
    let mut error = None;
    if cache.is_none() {
        let client = crate::vndb::http_client_builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| CommandError::new("provider_offline", "no http client"))?;
        match fetch(&client).await {
            Ok(mut fresh) => {
                fresh.checked_at = now;
                remember(pool, &fresh).await;
                cache = Some(fresh);
            }
            Err(reason) => error = Some(reason),
        }
    }
    let asset = cache.as_ref().and_then(|cache| {
        if cfg!(target_os = "android") {
            cache.apk.clone()
        } else if cfg!(target_os = "windows") {
            cache.setup.clone()
        } else {
            None
        }
    });
    Ok(UpdateView {
        current: CURRENT.to_owned(),
        latest: cache.as_ref().and_then(|cache| cache.latest.clone()),
        available: cache
            .as_ref()
            .and_then(|cache| cache.latest.as_deref())
            .is_some_and(|latest| is_newer(latest, CURRENT)),
        url: cache.as_ref().and_then(|cache| cache.url.clone()),
        asset,
        checked_at: cache.as_ref().map(|cache| cache.checked_at),
        error,
        can_install: can_install(),
    })
}

/// Download the installer and start it.
///
/// Returns the file it wrote, so the UI can say where it went. On Android the
/// download is handed to the system package installer instead of being executed.
///
/// # Errors
/// `no_update`, `download_failed`, `launch_failed` or `unsupported`.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let view = update_check(state, true).await?;
    let Some(asset) = view.asset.filter(|_| view.available) else {
        return Err(CommandError::new(
            "no_update",
            "no newer release is available",
        ));
    };
    if !view.can_install {
        return Err(CommandError::new(
            "unsupported",
            "this build cannot install the update itself",
        ));
    }
    let file = download(&app, &asset).await?;
    launch(&app, &file)?;
    Ok(file.display().to_string())
}

/// Whether a file name ends in one of `extensions`, ignoring case.
fn has_extension(name: &str, extensions: &[&str]) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|wanted| extension.eq_ignore_ascii_case(wanted))
        })
}

/// The file name an asset download may use.
///
/// Asset names come from the release, which is not a trusted path: a name with a
/// separator, a parent link or a leading dot is refused rather than joined onto
/// the download folder.
fn safe_file_name(name: &str) -> Option<&str> {
    let base = std::path::Path::new(name).file_name()?.to_str()?;
    // `is_separator` covers both separators on Windows and the one elsewhere,
    // so no backslash literal appears in this file.
    if base.is_empty()
        || base.starts_with('.')
        || base != name
        || name.chars().any(std::path::is_separator)
    {
        return None;
    }
    Some(base)
}

/// Download one asset into the user's downloads folder.
async fn download(
    app: &tauri::AppHandle,
    asset: &Asset,
) -> Result<std::path::PathBuf, CommandError> {
    use tauri::Manager as _;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().temp_dir())
        .map_err(|err| CommandError::new("download_failed", err.to_string()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|err| CommandError::new("download_failed", err.to_string()))?;
    let name = safe_file_name(&asset.name)
        .ok_or_else(|| CommandError::new("download_failed", "unusable asset name"))?;
    let target = dir.join(name);

    let client = crate::vndb::http_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_mins(10))
        .build()
        .map_err(|_| CommandError::new("download_failed", "no http client"))?;
    let response = client
        .get(&asset.url)
        .header(reqwest::header::USER_AGENT, "VN-Asyik")
        .send()
        .await
        .map_err(|_| CommandError::new("download_failed", "the download did not start"))?;
    if !response.status().is_success() {
        return Err(CommandError::new(
            "download_failed",
            format!("the server answered {}", response.status().as_u16()),
        ));
    }
    let mut body = response;
    let mut file = std::fs::File::create(&target)
        .map_err(|err| CommandError::new("download_failed", err.to_string()))?;
    while let Some(chunk) = body
        .chunk()
        .await
        .map_err(|_| CommandError::new("download_failed", "the download was interrupted"))?
    {
        file.write_all(&chunk)
            .map_err(|err| CommandError::new("download_failed", err.to_string()))?;
    }
    file.flush()
        .map_err(|err| CommandError::new("download_failed", err.to_string()))?;
    Ok(target)
}

/// Start the downloaded installer.
///
/// Windows runs the setup program, which shows its own window: this app never
/// installs anything silently. Android hands the APK to the system installer.
/// A platform without a package to run gets an error instead of a mystery file.
fn launch(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), CommandError> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| CommandError::new("launch_failed", err.to_string()))
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_opener::OpenerExt as _;
        app.opener()
            .open_path(path.to_string_lossy().to_string(), None::<&str>)
            .map_err(|err| CommandError::new("launch_failed", err.to_string()))
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, path);
        Err(CommandError::new(
            "unsupported",
            "this build cannot install the update itself",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_patch_minor_or_major_is_newer() {
        assert!(is_newer("0.1.7", "0.1.6"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.6", "0.1.6"));
        assert!(!is_newer("0.1.5", "0.1.6"));
        // A pre-release tag compares as the version it names.
        assert!(is_newer("0.1.7-beta.1", "0.1.6"));
        assert!(!is_newer("0.1.6-beta.1", "0.1.6"));
    }

    #[test]
    fn a_prerelease_list_keeps_the_highest_version() {
        let releases: Vec<Release> = serde_json::from_str(
            r#"[
                {"tag_name":"v0.1.6","html_url":"https://x/6","assets":[]},
                {"tag_name":"v0.1.10","html_url":"https://x/10","assets":[]},
                {"tag_name":"v0.1.9","html_url":"https://x/9","assets":[]},
                {"tag_name":"v0.1.11","draft":true,"assets":[]}
            ]"#,
        )
        .expect("releases");
        let newest = releases.into_iter().filter(|release| !release.draft).fold(
            None,
            |held: Option<(String, Release)>, release| {
                let version = release.tag_name.trim_start_matches('v').to_owned();
                match held {
                    Some((best, current)) if !is_newer(&version, &best) => Some((best, current)),
                    _ => Some((version, release)),
                }
            },
        );
        let (version, release) = newest.expect("a release");
        // 0.1.10 sorts above 0.1.9 only if the parts are compared as numbers.
        assert_eq!(version, "0.1.10");
        assert_eq!(release.html_url.as_deref(), Some("https://x/10"));
    }

    #[test]
    fn the_windows_and_android_assets_are_recognised() {
        let release: Release = serde_json::from_str(
            r#"{"tag_name":"v0.1.7","assets":[
                {"name":"VN.Asyik_0.1.7_x64_en-US.msi","browser_download_url":"https://x/a.msi","size":1},
                {"name":"VN.Asyik_0.1.7_x64-setup.exe","browser_download_url":"https://x/a.exe","size":2},
                {"name":"app-universal-release.apk","browser_download_url":"https://x/a.apk","size":3}
            ]}"#,
        )
        .expect("release");
        let setup = release
            .assets
            .iter()
            .find(|asset| asset.name.contains("x64-setup"))
            .expect("setup");
        assert_eq!(setup.browser_download_url, "https://x/a.exe");
        let apk = release
            .assets
            .iter()
            .find(|asset| has_extension(&asset.name, &["apk"]))
            .expect("apk");
        assert_eq!(apk.size, Some(3));
    }

    #[test]
    fn an_asset_name_cannot_climb_out_of_the_download_folder() {
        assert_eq!(
            safe_file_name("VN.Asyik_0.1.7_x64-setup.exe"),
            Some("VN.Asyik_0.1.7_x64-setup.exe")
        );
        assert_eq!(safe_file_name("../evil.exe"), None);
        assert_eq!(safe_file_name("/etc/passwd"), None);
        assert_eq!(safe_file_name(r"..\evil.exe"), None);
        assert_eq!(safe_file_name(".hidden"), None);
        assert_eq!(safe_file_name(""), None);
    }
}
