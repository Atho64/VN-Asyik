//! Opt-in upload of presence images to a public image host.
//!
//! Ported behaviour from `EwE`'s image-host feature, which uploads a cover and
//! sends the hosted URL so Discord can render it where it cannot fetch a plain
//! asset: on the Android Gateway, or for a local custom cover on any platform.
//!
//! Two hard rules from `AGENTS.md` §10b hold here: nothing is uploaded while
//! the host is `None`, and the original bytes of explicit (or unrated) art
//! never leave the device — the blur is applied locally before any network
//! call, so what reaches the host is the same concealed image the library
//! shows under the blur setting.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use vn_core::presence::RpcImageHost;

/// An upload that failed is not retried until this much time has passed, so a
/// dead host does not turn every two-second poll into a network attempt.
const FAILURE_COOLDOWN: Duration = Duration::from_mins(5);

/// Errors from the upload path.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The host never returned a usable URL.
    #[error("image host returned no URL: {0}")]
    NoUrl(String),
    /// The request itself failed.
    #[error("image host request failed: {0}")]
    Request(String),
    /// imgbb requires an API key and none is stored.
    #[error("imgbb upload requires an API key")]
    NoApiKey,
}

/// Uploads one image to the configured host, returning its public URL.
///
/// Endpoints match `EwE`'s `upload_image_file`: catbox takes a multipart form
/// and answers with the bare URL; imgbb takes the bytes base64-encoded in a
/// form and answers with JSON.
///
/// # Errors
/// Fails on request, decode or host-refusal; never on "host is None" — the
/// caller decides that before uploading.
pub async fn upload_to_host(
    host: RpcImageHost,
    api_key: Option<&str>,
    bytes: Vec<u8>,
    filename: &str,
) -> Result<String, UploadError> {
    let client = crate::vndb::http_client_builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_mins(1))
        .build()
        .map_err(|err| UploadError::Request(err.to_string()))?;
    match host {
        RpcImageHost::Catbox => {
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(filename.to_owned())
                .mime_str("image/jpeg")
                .map_err(|err| UploadError::Request(err.to_string()))?;
            let form = reqwest::multipart::Form::new()
                .text("reqtype", "fileupload")
                .part("fileToUpload", part);
            let response = client
                .post("https://catbox.moe/user/api.php")
                .multipart(form)
                .send()
                .await
                .map_err(|err| UploadError::Request(err.to_string()))?;
            finish(response, |body| body).await
        }
        RpcImageHost::Imgbb => {
            let key = api_key.ok_or(UploadError::NoApiKey)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let response = client
                .post("https://api.imgbb.com/1/upload")
                .form(&[("key", key), ("image", encoded.as_str())])
                .send()
                .await
                .map_err(|err| UploadError::Request(err.to_string()))?;
            finish(response, |body| {
                serde_json::from_str::<ImgbbReply>(&body)
                    .ok()
                    .filter(|reply| reply.success)
                    .and_then(|reply| reply.data.url)
                    .unwrap_or_default()
            })
            .await
        }
        // Unreachable by contract: the caller never uploads with no host, and a
        // host-less upload would silently publish nothing anyway.
        RpcImageHost::None => Err(UploadError::NoUrl("no image host is configured".into())),
    }
}

/// Turns a response into a URL, sharing the "empty body means refused" rule.
async fn finish(
    response: reqwest::Response,
    extract: impl Fn(String) -> String,
) -> Result<String, UploadError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| UploadError::Request(err.to_string()))?;
    // Keep a slice of the body before handing it to the extractor: it names
    // what the host refused, without echoing an unbounded response into the log.
    let head: String = body.chars().take(200).collect();
    let url = extract(body);
    let url = url.trim();
    if status.is_success() && (url.starts_with("https://") || url.starts_with("http://")) {
        Ok(url.to_owned())
    } else {
        Err(UploadError::NoUrl(head))
    }
}

#[derive(serde::Deserialize)]
struct ImgbbReply {
    success: bool,
    data: ImgbbData,
}

#[derive(serde::Deserialize)]
struct ImgbbData {
    url: Option<String>,
}

/// Session cache so the poll loop does not re-upload the same image forever.
///
/// Keyed by the caller's digest of the exact bytes being sent: a changed cover,
/// a changed host or a blur flip is a different key. Concurrent polls share one
/// in-flight attempt, successes live for the session, and a failure cools down
/// for [`FAILURE_COOLDOWN`] instead of retrying on every tick.
#[derive(Debug, Default)]
pub struct UploadCache {
    done: Mutex<HashMap<String, String>>,
    pending: Mutex<HashMap<String, std::sync::Arc<tokio::sync::OnceCell<Option<String>>>>>,
    failed_at: Mutex<HashMap<String, Instant>>,
}

impl UploadCache {
    /// Returns the cached URL, or runs `upload` once for this key.
    ///
    /// `upload` is created eagerly by the caller but polled only when this call
    /// is the one that owns the attempt.
    pub async fn get_or_upload<F>(&self, key: &str, upload: F) -> Option<String>
    where
        F: std::future::Future<Output = Result<String, UploadError>>,
    {
        if let Some(url) = self.done.lock().expect("cache").get(key) {
            return Some(url.clone());
        }
        if let Some(failed) = self.failed_at.lock().expect("cache").get(key)
            && failed.elapsed() < FAILURE_COOLDOWN
        {
            return None;
        }
        let cell = {
            let mut pending = self.pending.lock().expect("cache");
            pending.get(key).cloned().unwrap_or_else(|| {
                let cell = std::sync::Arc::new(tokio::sync::OnceCell::new());
                pending.insert(key.to_owned(), std::sync::Arc::clone(&cell));
                cell
            })
        };
        // A cell that already holds a value resolves immediately; only the
        // caller that inserted the fresh cell polls `upload`.
        let result = cell
            .get_or_init(|| async { upload.await.ok() })
            .await
            .clone();
        self.pending.lock().expect("cache").remove(key);
        let Some(url) = result else {
            self.failed_at
                .lock()
                .expect("cache")
                .insert(key.to_owned(), Instant::now());
            return None;
        };
        self.done
            .lock()
            .expect("cache")
            .insert(key.to_owned(), url.clone());
        Some(url)
    }
}

/// The process-wide cache. Presence polls are periodic, so a static fits.
#[must_use]
pub fn shared_cache() -> &'static UploadCache {
    static CACHE: OnceLock<UploadCache> = OnceLock::new();
    CACHE.get_or_init(UploadCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(tag: &str) -> String {
        format!("catbox:false:{tag}")
    }

    // The signature mirrors a real network uploader: callers receive a Future.
    #[allow(clippy::unused_async)]
    async fn counting_upload(calls: &AtomicUsize, fail: bool) -> Result<String, UploadError> {
        calls.fetch_add(1, Ordering::SeqCst);
        if fail {
            Err(UploadError::NoUrl("refused".into()))
        } else {
            Ok("https://catbox.moe/x.jpg".to_owned())
        }
    }

    #[tokio::test]
    async fn concurrent_polls_share_one_upload() {
        let cache = UploadCache::default();
        let calls = AtomicUsize::new(0);
        let k = key("shared");

        // Two awaits of the same future shape: only one upload may run.
        let (a, b) = tokio::join!(
            cache.get_or_upload(&k, counting_upload(&calls, false)),
            cache.get_or_upload(&k, counting_upload(&calls, false)),
        );
        assert_eq!(a, Some("https://catbox.moe/x.jpg".to_owned()));
        assert_eq!(b, Some("https://catbox.moe/x.jpg".to_owned()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A later poll reads the cache without uploading again.
        let again = cache
            .get_or_upload(&k, counting_upload(&calls, false))
            .await;
        assert_eq!(again, Some("https://catbox.moe/x.jpg".to_owned()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failure_is_not_retried_inside_the_cooldown() {
        let cache = UploadCache::default();
        let calls = AtomicUsize::new(0);
        let k = key("failing");

        let first = cache.get_or_upload(&k, counting_upload(&calls, true)).await;
        assert_eq!(first, None);
        let second = cache.get_or_upload(&k, counting_upload(&calls, true)).await;
        assert_eq!(second, None);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "cooldown swallowed the retry"
        );
    }

    #[tokio::test]
    async fn a_new_source_digest_uploads_again() {
        let cache = UploadCache::default();
        let calls = AtomicUsize::new(0);
        cache
            .get_or_upload(&key("a"), counting_upload(&calls, false))
            .await;
        cache
            .get_or_upload(&key("b"), counting_upload(&calls, false))
            .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn imgbb_success_extracts_the_hosted_url() {
        let body = r#"{"success":true,"data":{"url":"https://i.ibb.co/x.jpg"}}"#;
        let reply: ImgbbReply = serde_json::from_str(body).expect("reply");
        assert_eq!(reply.data.url.as_deref(), Some("https://i.ibb.co/x.jpg"));
    }
}
