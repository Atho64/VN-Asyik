//! User-selected documents and Android Keystore bridge. No arbitrary frontend paths.

use crate::commands::{CommandError, CommandResult};

const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Ceiling for a library export, which now carries the reader's screenshots
/// inline as base64.
///
/// The 64 MiB document cap exists so a picker cannot hand the app a file that
/// exhausts memory. A library with a few hundred captures legitimately exceeds
/// it, so this path gets its own, larger ceiling rather than silently dropping
/// pictures from a backup.
const MAX_LIBRARY_BYTES: u64 = 512 * 1024 * 1024;

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

/// Save a JSON document through the platform picker.
///
/// Every caller of this is a backup, transfer or sync payload, so the filter and
/// MIME type are fixed. A readable export (the journal) uses
/// [`save_document_as`] instead of widening this one.
pub async fn save_document(
    app: tauri::AppHandle,
    filename: String,
    contents: String,
) -> CommandResult<bool> {
    save_document_as(app, filename, contents, "application/json", "json").await
}

/// Save a document of any text format through the platform picker.
///
/// The MIME type matters on Android: the picker decides the file's type from it,
/// and a Markdown journal saved as `application/json` opens in nothing. On desktop
/// it becomes the dialog's extension filter, so the file the user writes keeps the
/// extension the app promised.
pub async fn save_document_as(
    app: tauri::AppHandle,
    filename: String,
    contents: String,
    mime: &'static str,
    extension: &'static str,
) -> CommandResult<bool> {
    save_document_with_limit(app, filename, contents, mime, extension, MAX_DOCUMENT_BYTES).await
}

/// Save a library export, which carries screenshots and is allowed to be large.
///
/// # Errors
/// `file_too_large` past [`MAX_LIBRARY_BYTES`], or a document failure.
pub async fn save_library_document(
    app: tauri::AppHandle,
    filename: String,
    contents: String,
) -> CommandResult<bool> {
    save_document_with_limit(
        app,
        filename,
        contents,
        "application/json",
        "json",
        MAX_LIBRARY_BYTES,
    )
    .await
}

/// Ceiling for a generated share card. A 1080x1350 PNG is far below this; the
/// limit exists so a malformed data URL cannot exhaust memory.
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Decode the PNG data URL a share card produces.
///
/// The prefix, the base64 and the PNG signature are all checked here rather than
/// trusted: this is the boundary between a canvas in the webview and a file on
/// disk, and writing arbitrary bytes under a `.png` name would be silent
/// corruption.
///
/// # Errors
/// `invalid_image` for a non-PNG data URL or bad base64; `file_too_large` past
/// [`MAX_IMAGE_BYTES`].
pub fn decode_png_data_url(data_url: &str) -> CommandResult<Vec<u8>> {
    use base64::Engine as _;
    const PREFIX: &str = "data:image/png;base64,";
    let encoded = data_url
        .strip_prefix(PREFIX)
        .ok_or_else(|| CommandError::new("invalid_image", "Expected a PNG data URL"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| CommandError::new("invalid_image", "The image data is not valid base64"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(CommandError::new("file_too_large", "Image exceeds 16 MiB"));
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(CommandError::new("invalid_image", "The image is not a PNG"));
    }
    Ok(bytes)
}

/// Save a generated PNG through the platform picker.
///
/// # Errors
/// `file_too_large` past [`MAX_IMAGE_BYTES`], or a picker/write failure.
pub async fn save_image(
    app: tauri::AppHandle,
    filename: String,
    bytes: Vec<u8>,
) -> CommandResult<bool> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(CommandError::new("file_too_large", "Image exceeds 16 MiB"));
    }
    tauri::async_runtime::spawn_blocking(move || save_image_blocking(&app, &filename, &bytes))
        .await
        .map_err(|err| CommandError::new("image", err.to_string()))?
}

/// Share a generated PNG through the Android share sheet.
///
/// Android is the only platform with a system share sheet this app can reach;
/// desktop callers get a clear unsupported result and use Save image instead.
///
/// # Errors
/// `file_too_large` past [`MAX_IMAGE_BYTES`], or a share failure.
#[cfg(target_os = "android")]
pub async fn share_image(filename: String, bytes: Vec<u8>) -> CommandResult<()> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(CommandError::new("file_too_large", "Image exceeds 16 MiB"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        mobile::call::<()>(
            "shareDocument",
            serde_json::json!({
                "filename": filename,
                "contents": encoded,
                "mime": "image/png",
                "base64": true,
            }),
        )
        .map_err(|err| CommandError::new("image", err))
    })
    .await
    .map_err(|err| CommandError::new("image", err.to_string()))?
}

/// Desktop has no share sheet; Save image is the supported path.
#[cfg(not(target_os = "android"))]
pub fn share_image(_filename: String, _bytes: Vec<u8>) -> std::future::Ready<CommandResult<()>> {
    std::future::ready(Err(CommandError::new(
        "unsupported_platform",
        "Use Save image on this platform",
    )))
}

#[cfg(target_os = "android")]
fn save_image_blocking(
    _app: &tauri::AppHandle,
    filename: &str,
    bytes: &[u8],
) -> CommandResult<bool> {
    use base64::Engine as _;
    #[derive(serde::Deserialize)]
    struct Reply {
        saved: bool,
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    mobile::call::<Reply>(
        "saveDocument",
        serde_json::json!({
            "filename": filename,
            "contents": encoded,
            "mime": "image/png",
            "base64": true,
        }),
    )
    .map(|reply| reply.saved)
    .map_err(|err| CommandError::new("image", err))
}

#[cfg(not(target_os = "android"))]
fn save_image_blocking(
    app: &tauri::AppHandle,
    filename: &str,
    bytes: &[u8],
) -> CommandResult<bool> {
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("PNG image", &["png"])
        .set_file_name(filename)
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = file
        .into_path()
        .map_err(|err| CommandError::new("image", err.to_string()))?;
    std::fs::write(path, bytes).map_err(|err| CommandError::new("image", err.to_string()))?;
    Ok(true)
}

async fn save_document_with_limit(
    app: tauri::AppHandle,
    filename: String,
    contents: String,
    mime: &'static str,
    extension: &'static str,
    limit: u64,
) -> CommandResult<bool> {
    if contents.len() as u64 > limit {
        return Err(CommandError::new(
            "file_too_large",
            format!("Document exceeds {} MiB", limit / (1024 * 1024)),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || save(&app, &filename, &contents, mime, extension))
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?
}

pub async fn open_document(app: tauri::AppHandle) -> CommandResult<Option<String>> {
    open_document_with_limit(app, MAX_DOCUMENT_BYTES).await
}

/// Open a library export, which may carry screenshots and so may be large.
///
/// # Errors
/// `file_too_large` past [`MAX_LIBRARY_BYTES`], or a document failure.
pub async fn open_library_document(app: tauri::AppHandle) -> CommandResult<Option<String>> {
    open_document_with_limit(app, MAX_LIBRARY_BYTES).await
}

async fn open_document_with_limit(
    app: tauri::AppHandle,
    limit: u64,
) -> CommandResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || open(&app, limit))
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?
}

#[cfg(target_os = "android")]
fn save(
    _app: &tauri::AppHandle,
    filename: &str,
    contents: &str,
    mime: &str,
    _extension: &str,
) -> CommandResult<bool> {
    #[derive(serde::Deserialize)]
    struct Reply {
        saved: bool,
    }
    mobile::call::<Reply>(
        "saveDocument",
        serde_json::json!({"filename": filename, "contents": contents, "mime": mime}),
    )
    .map(|reply| reply.saved)
    .map_err(|err| CommandError::new("document", err))
}

#[cfg(not(target_os = "android"))]
fn save(
    app: &tauri::AppHandle,
    filename: &str,
    contents: &str,
    _mime: &str,
    extension: &str,
) -> CommandResult<bool> {
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("VN Asyik", &[extension])
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
fn open(_app: &tauri::AppHandle, _limit: u64) -> CommandResult<Option<String>> {
    #[derive(serde::Deserialize)]
    struct Reply {
        contents: Option<String>,
    }
    mobile::call::<Reply>("openDocument", serde_json::json!({}))
        .map(|reply| reply.contents)
        .map_err(|err| CommandError::new("document", err))
}

#[cfg(not(target_os = "android"))]
fn open(app: &tauri::AppHandle, limit: u64) -> CommandResult<Option<String>> {
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
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(CommandError::new(
            "file_too_large",
            format!("Document exceeds {} MiB", limit / (1024 * 1024)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn png_data_url(bytes: &[u8]) -> String {
        use base64::Engine as _;
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    #[test]
    fn a_png_data_url_decodes_to_the_original_bytes() {
        let bytes = b"\x89PNG\r\n\x1a\npayload";
        assert_eq!(
            decode_png_data_url(&png_data_url(bytes)).expect("decode"),
            bytes
        );
    }

    #[test]
    fn a_non_png_data_url_is_refused() {
        let err = decode_png_data_url("data:image/jpeg;base64,AAAA").expect_err("refuse");
        assert_eq!(err.code, "invalid_image");
    }

    #[test]
    fn bad_base64_is_refused() {
        let err = decode_png_data_url("data:image/png;base64,not base64!").expect_err("refuse");
        assert_eq!(err.code, "invalid_image");
    }

    #[test]
    fn a_decoded_payload_without_the_png_signature_is_refused() {
        let err = decode_png_data_url(&png_data_url(b"not a png")).expect_err("refuse");
        assert_eq!(err.code, "invalid_image");
    }
}
