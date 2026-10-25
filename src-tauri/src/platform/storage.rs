//! User-selected documents and Android Keystore bridge. No arbitrary frontend paths.

use crate::commands::{CommandError, CommandResult};

const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(target_os = "android")]
mod mobile {
    use std::sync::OnceLock;
    use tauri::plugin::PluginHandle;

    static HANDLE: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

    pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
        tauri::plugin::Builder::<tauri::Wry, ()>::new("android-storage")
            .setup(|_app, api| {
                let handle =
                    api.register_android_plugin("com.atho.vnasyik", "AndroidStoragePlugin")?;
                let _ = HANDLE.set(handle);
                Ok(())
            })
            .build()
    }

    pub fn call<T: serde::de::DeserializeOwned>(
        command: &str,
        payload: serde_json::Value,
    ) -> Result<T, String> {
        HANDLE
            .get()
            .ok_or_else(|| "Android storage is not ready".to_owned())?
            .run_mobile_plugin(command, payload)
            .map_err(|err| err.to_string())
    }
}

#[cfg(target_os = "android")]
pub use mobile::init;

#[cfg(not(target_os = "android"))]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::<tauri::Wry, ()>::new("android-storage").build()
}

#[cfg(target_os = "android")]
pub fn read_secret(name: &str) -> Result<Option<String>, String> {
    #[derive(serde::Deserialize)]
    struct Reply {
        value: Option<String>,
    }
    mobile::call::<Reply>("readSecret", serde_json::json!({"name": name})).map(|reply| reply.value)
}

#[cfg(target_os = "android")]
pub fn write_secret(name: &str, value: &str) -> Result<(), String> {
    mobile::call(
        "writeSecret",
        serde_json::json!({"name": name, "value": value}),
    )
}

#[cfg(target_os = "android")]
pub fn remove_secret(name: &str) -> Result<(), String> {
    mobile::call("removeSecret", serde_json::json!({"name": name}))
}

/// A user-selected document resolved to a readable file. On Android a
/// `content://` URI from the system picker is copied through the content
/// resolver into a private cache file (AGENTS.md §9: the URI is never treated
/// as a filesystem path); the copy is deleted when this guard drops. Every
/// other path is used as-is.
///
/// `display_name` is the file's human-facing name as the picker reported it —
/// the name to sort multi-file imports by and to show in the UI. It is never
/// derived from a `content://` URI: those are opaque ids whose order says
/// nothing about the files they name.
#[derive(Debug)]
pub struct ResolvedDocument {
    path: std::path::PathBuf,
    display_name: String,
    cache_copy: Option<std::path::PathBuf>,
}

impl ResolvedDocument {
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

impl Drop for ResolvedDocument {
    fn drop(&mut self) {
        if let Some(copy) = self.cache_copy.take() {
            let _ = std::fs::remove_file(copy);
        }
    }
}

/// Resolve what a picker handed back into a readable file. Non-`content://`
/// strings are returned unchanged, so desktop paths and plain Android file
/// paths keep working.
///
/// On Android this performs a blocking bridge call; callers must not hold the
/// UI thread (the command layer wraps it in `spawn_blocking`).
///
/// # Errors
/// On Android, `String` errors from the copy (document gone, over 64 MiB,
/// storage bridge not ready).
#[cfg(target_os = "android")]
pub fn resolve_document_uri(uri: &str) -> Result<ResolvedDocument, String> {
    if !uri.starts_with("content://") {
        return Ok(ResolvedDocument {
            display_name: fallback_name(uri),
            path: uri.into(),
            cache_copy: None,
        });
    }
    #[derive(serde::Deserialize)]
    struct Reply {
        path: String,
        name: Option<String>,
    }
    let reply = mobile::call::<Reply>("readUriToCache", serde_json::json!({"uri": uri}))
        .map_err(|err| err.to_string())?;
    // The plugin queries OpenableColumns.DISPLAY_NAME; if it could not answer,
    // a percent-decoded last URI segment is better than sorting by the cache
    // copy's random temp name.
    let display_name = reply
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| fallback_name(uri));
    Ok(ResolvedDocument {
        path: reply.path.clone().into(),
        display_name,
        cache_copy: Some(reply.path.into()),
    })
}

/// Last path segment, percent-decoded, as a name of last resort.
#[cfg(target_os = "android")]
fn fallback_name(path: &str) -> String {
    let last = path.rsplit(['/', '\\']).next().unwrap_or(path);
    percent_decode(last)
}

#[cfg(target_os = "android")]
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(not(target_os = "android"))]
pub fn resolve_document_uri(uri: &str) -> Result<ResolvedDocument, String> {
    Ok(ResolvedDocument {
        display_name: std::path::Path::new(uri).file_name().map_or_else(
            || uri.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        path: uri.into(),
        cache_copy: None,
    })
}

pub async fn save_document(
    app: tauri::AppHandle,
    filename: String,
    contents: String,
) -> CommandResult<bool> {
    if contents.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(CommandError::new(
            "file_too_large",
            "Document exceeds 64 MiB",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || save(&app, &filename, &contents))
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?
}

pub async fn open_document(app: tauri::AppHandle) -> CommandResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || open(&app))
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?
}

#[cfg(target_os = "android")]
fn save(_app: &tauri::AppHandle, filename: &str, contents: &str) -> CommandResult<bool> {
    #[derive(serde::Deserialize)]
    struct Reply {
        saved: bool,
    }
    mobile::call::<Reply>(
        "saveDocument",
        serde_json::json!({"filename": filename, "contents": contents}),
    )
    .map(|reply| reply.saved)
    .map_err(|err| CommandError::new("document", err))
}

#[cfg(not(target_os = "android"))]
fn save(app: &tauri::AppHandle, filename: &str, contents: &str) -> CommandResult<bool> {
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("VN Asyik", &["json"])
        .set_file_name(filename)
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = file
        .into_path()
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    std::fs::write(path, contents).map_err(|err| CommandError::new("document", err.to_string()))?;
    Ok(true)
}

#[cfg(target_os = "android")]
fn open(_app: &tauri::AppHandle) -> CommandResult<Option<String>> {
    #[derive(serde::Deserialize)]
    struct Reply {
        contents: Option<String>,
    }
    mobile::call::<Reply>("openDocument", serde_json::json!({}))
        .map(|reply| reply.contents)
        .map_err(|err| CommandError::new("document", err))
}

#[cfg(not(target_os = "android"))]
fn open(app: &tauri::AppHandle) -> CommandResult<Option<String>> {
    use std::io::Read;
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("VN Asyik", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    let file =
        std::fs::File::open(path).map_err(|err| CommandError::new("document", err.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(CommandError::new(
            "file_too_large",
            "Document exceeds 64 MiB",
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| CommandError::new("invalid_file", "Choose a UTF-8 JSON file"))
}

#[cfg(target_os = "android")]
pub async fn share_document(filename: String, contents: String) -> CommandResult<()> {
    #[cfg(target_os = "android")]
    {
        if contents.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(CommandError::new(
                "file_too_large",
                "Document exceeds 64 MiB",
            ));
        }
        tauri::async_runtime::spawn_blocking(move || {
            mobile::call(
                "shareDocument",
                serde_json::json!({"filename": filename, "contents": contents}),
            )
            .map_err(|err| CommandError::new("document", err))
        })
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?
    }
}

#[cfg(not(target_os = "android"))]
pub fn share_document(
    _filename: String,
    _contents: String,
) -> std::future::Ready<CommandResult<()>> {
    std::future::ready(Err(CommandError::new(
        "unsupported_platform",
        "Use Save file on this platform",
    )))
}
