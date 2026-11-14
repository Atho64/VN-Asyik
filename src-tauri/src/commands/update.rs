//! GitHub Releases update checker and optional Windows installer launcher.
//! Compares semver manually against pre-releases (since `/releases/latest` skips tag pre-releases); only triggered explicitly via [`update_check`].

use std::io::Write as _;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// Where releases are listed.
const RELEASES_URL: &str = "https://api.github.com/repos/Atho64/VN-Asyik/releases?per_page=20";
/// Fallback release page when a release entry omits `html_url`.
const RELEASES_PAGE_URL: &str = "https://github.com/Atho64/VN-Asyik/releases";
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
        url: release.html_url.filter(|url| !url.trim().is_empty()),
        // Extension checks go through `Path`, as the download does with the name,
        // and ignore case: a release asset may be spelled `.APK`.
        setup: pick(&|name| has_extension(name, &["exe"])),
        apk: pick(&|name| has_extension(name, &["apk"])),
    })
}

/// Whether this build may download and launch an installer itself.
///
/// Only Windows ships an interactive setup binary (`setup.exe`). Android,
/// Linux and macOS open the GitHub release page instead of running an
/// error-prone in-app download and package handoff.
const fn can_install() -> bool {
    cfg!(target_os = "windows")
}

/// Installer asset used by `update_install` on platforms that support it.
fn platform_asset(cache: &Cache) -> Option<Asset> {
    if cfg!(target_os = "windows") {
        cache.setup.clone()
    } else {
        None
    }
}

/// Shape the UI-facing update status from a cached GitHub check.
fn view_from_cache(
    current: &str,
    cache: Option<&Cache>,
    error: Option<String>,
    installable: bool,
    asset: Option<Asset>,
) -> UpdateView {
    let available = cache
        .and_then(|cache| cache.latest.as_deref())
        .is_some_and(|latest| is_newer(latest, current));
    let url = cache.and_then(|cache| {
        cache
            .url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| Some(RELEASES_PAGE_URL.to_owned()))
    });
    UpdateView {
        current: current.to_owned(),
        latest: cache.and_then(|cache| cache.latest.clone()),
        available,
        url,
        asset: if installable { asset } else { None },
        checked_at: cache.map(|cache| cache.checked_at),
        error,
        can_install: installable,
    }
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
    let asset = cache.as_ref().and_then(platform_asset);
    Ok(view_from_cache(
        CURRENT,
        cache.as_ref(),
        error,
        can_install(),
        asset,
    ))
}

/// Download the installer and start it.
///
/// Returns the file it wrote, so the UI can say where it went. Supported on
/// Windows only; Android and other platforms open the GitHub release page
/// via `open_external` instead.
///
/// # Errors
/// `no_update`, `download_failed`, `launch_failed` or `unsupported`.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    if !can_install() {
        return Err(CommandError::new(
            "unsupported",
            "this build cannot install the update itself",
        ));
    }
    let view = update_check(state, true).await?;
    let Some(asset) = view.asset.filter(|_| view.available) else {
        return Err(CommandError::new(
            "no_update",
            "no newer release is available",
        ));
    };
    let file = download(&app, &asset).await?;
    launch(&file)?;
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

/// Sanitize a release asset filename, rejecting path separators, traversal segments, and leading dots.
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
/// installs anything silently. Other platforms open the GitHub release page
/// instead and return `unsupported` if invoked directly.
fn launch(path: &std::path::Path) -> Result<(), CommandError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| CommandError::new("launch_failed", err.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
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

    #[test]
    fn non_installable_platforms_use_release_page_instead_of_asset() {
        let cache = Cache {
            checked_at: 123,
            latest: Some("9.9.9".to_owned()),
            url: Some("https://github.com/Atho64/VN-Asyik/releases/tag/v9.9.9".to_owned()),
            setup: Some(Asset {
                name: "VN.Asyik_9.9.9_x64-setup.exe".to_owned(),
                url: "https://github.com/Atho64/VN-Asyik/releases/download/v9.9.9/setup.exe"
                    .to_owned(),
                size: Some(42),
            }),
            apk: Some(Asset {
                name: "app-universal-release.apk".to_owned(),
                url: "https://github.com/Atho64/VN-Asyik/releases/download/v9.9.9/app.apk"
                    .to_owned(),
                size: Some(99),
            }),
        };

        let link_only = view_from_cache("0.2.4", Some(&cache), None, false, cache.apk.clone());
        assert!(link_only.available);
        assert!(!link_only.can_install);
        assert_eq!(link_only.asset, None);
        assert_eq!(
            link_only.url.as_deref(),
            Some("https://github.com/Atho64/VN-Asyik/releases/tag/v9.9.9")
        );

        let fallback = Cache {
            url: None,
            ..cache.clone()
        };
        let fallback_view = view_from_cache("0.2.4", Some(&fallback), None, false, None);
        assert_eq!(fallback_view.url.as_deref(), Some(RELEASES_PAGE_URL));

        let windows = view_from_cache("0.2.4", Some(&cache), None, true, cache.setup.clone());
        assert!(windows.can_install);
        assert_eq!(
            windows.asset.as_ref().map(|a| a.name.as_str()),
            Some("VN.Asyik_9.9.9_x64-setup.exe")
        );
        assert_eq!(can_install(), cfg!(target_os = "windows"));
    }
}
