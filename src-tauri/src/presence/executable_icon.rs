//! Local executable-icon extraction and opt-in publication for the small asset.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use sha2::{Digest, Sha256};
use vn_core::presence::{PresenceSettings, RpcImageHost};

#[cfg(windows)]
mod windows;

type CachedIcon = Arc<tokio::sync::OnceCell<Option<Arc<Vec<u8>>>>>;
static CACHE: OnceLock<Mutex<HashMap<String, CachedIcon>>> = OnceLock::new();
const MAX_CACHE_ENTRIES: usize = 64;

#[derive(Debug)]
pub(crate) struct ExecutableIcon {
    png: Arc<Vec<u8>>,
    pub exe_name: String,
}

impl ExecutableIcon {
    pub fn data_url(&self) -> String {
        use base64::Engine;
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&*self.png)
        )
    }

    fn upload_key(&self, settings: &PresenceSettings) -> String {
        format!(
            "exe-icon:{:?}:{:x}",
            settings.rpc_image_host,
            Sha256::digest(&*self.png)
        )
    }

    pub async fn hosted_url(&self, settings: &PresenceSettings) -> Option<String> {
        if !settings.use_exe_icon
            || !settings.small_image.trim().is_empty()
            || settings.rpc_image_host == RpcImageHost::None
        {
            return None;
        }
        let key = self.upload_key(settings);
        super::upload::shared_cache()
            .get_or_upload(&key, async {
                let api_key = if settings.rpc_image_host == RpcImageHost::Imgbb {
                    crate::secrets::load_imgbb_key()
                        .ok()
                        .map(|key| key.expose().to_owned())
                } else {
                    None
                };
                super::upload::upload_to_host(
                    settings.rpc_image_host,
                    api_key.as_deref(),
                    (*self.png).clone(),
                    "exe-icon.png",
                )
                .await
            })
            .await
    }
}

/// Prefers the current tracked executable, then enabled mappings for this game.
pub(crate) async fn for_game(
    pool: &sqlx::SqlitePool,
    game_id: vn_core::EntityId,
    tracked_executable: Option<&str>,
    settings: &PresenceSettings,
) -> Option<ExecutableIcon> {
    if !cfg!(windows) || !settings.use_exe_icon || !settings.small_image.trim().is_empty() {
        return None;
    }
    if let Some(path) = tracked_executable
        && let Some(icon) = from_file(Path::new(path)).await
    {
        return Some(icon);
    }
    let mappings = crate::tracking::mapping::executables_for_game(pool, game_id)
        .await
        .ok()?;
    for mapping in mappings.into_iter().filter(|mapping| mapping.enabled) {
        if let Some(path) = mapping.exe_path
            && let Some(icon) = from_file(Path::new(&path)).await
        {
            return Some(icon);
        }
    }
    None
}

async fn from_file(path: &Path) -> Option<ExecutableIcon> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    // Same-size replacements also invalidate extraction; no executable bytes are read on a hit.
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let key = format!("{}:{}:{modified}", path.display(), metadata.len());
    let cell = {
        let mut cache = CACHE.get_or_init(Mutex::default).lock().ok()?;
        if !cache.contains_key(&key) && cache.len() >= MAX_CACHE_ENTRIES {
            cache.clear();
        }
        Arc::clone(cache.entry(key).or_default())
    };
    let png = cell
        .get_or_init(|| async {
            let path = path.to_owned();
            tauri::async_runtime::spawn_blocking(move || extract_png(&path))
                .await
                .ok()
                .flatten()
                .map(Arc::new)
        })
        .await
        .clone()?;
    Some(ExecutableIcon {
        png,
        exe_name: path.file_name()?.to_string_lossy().into_owned(),
    })
}

#[cfg(windows)]
fn extract_png(path: &Path) -> Option<Vec<u8>> {
    windows::extract_png(path)
}

#[cfg(not(windows))]
fn extract_png(_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_host_and_custom_or_disabled_icons_never_use_a_cached_upload() {
        let icon = ExecutableIcon {
            png: Arc::new(vec![1, 2, 3]),
            exe_name: "game.exe".into(),
        };
        for settings in [
            PresenceSettings::default(),
            PresenceSettings {
                rpc_image_host: RpcImageHost::Catbox,
                small_image: "custom".into(),
                ..Default::default()
            },
            PresenceSettings {
                rpc_image_host: RpcImageHost::Catbox,
                use_exe_icon: false,
                ..Default::default()
            },
        ] {
            super::super::upload::shared_cache()
                .seed(&icon.upload_key(&settings), "https://example.com/icon.png");
            assert_eq!(icon.hosted_url(&settings).await, None);
        }
    }

    #[tokio::test]
    async fn an_enabled_host_reuses_the_png_digest_without_network() {
        let icon = ExecutableIcon {
            png: Arc::new(vec![4, 5, 6]),
            exe_name: "game.exe".into(),
        };
        let settings = PresenceSettings {
            rpc_image_host: RpcImageHost::Catbox,
            ..Default::default()
        };
        super::super::upload::shared_cache().seed(
            &icon.upload_key(&settings),
            "https://example.com/hosted.png",
        );
        assert_eq!(
            icon.hosted_url(&settings).await.as_deref(),
            Some("https://example.com/hosted.png")
        );
        assert!(icon.data_url().starts_with("data:image/png;base64,"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn concurrent_extraction_reuses_the_local_png() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.ico");
        let (first, second) = tokio::join!(from_file(&path), from_file(&path));
        let first = first.expect("bundled icon");
        let second = second.expect("bundled icon");
        assert!(Arc::ptr_eq(&first.png, &second.png));
        assert_eq!(image::load_from_memory(&first.png).unwrap().width(), 32);
    }
}
