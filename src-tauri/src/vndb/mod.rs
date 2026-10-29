//! VNDB API v2 (Kana) client.
//!
//! Endpoint and behaviour follow <https://api.vndb.org/kana>. The parts that
//! matter for this app, as documented there:
//!
//! * queries are `POST` with a JSON body of `{filters, fields, sort, reverse,
//!   results, page}`,
//! * `results` is capped at 100,
//! * the service is limited to roughly 200 requests per 5 minutes, and long
//!   requests are aborted after 3 seconds,
//! * `429` means throttled, `401` means a bad token, `400` means a bad query,
//! * "too much data selected" is an error caused by asking for too many fields
//!   or results at once.
//!
//! Nothing here invents endpoints: only `POST /vn`, `POST /character` and
//! `GET /authinfo` are used, and each is named after the documented path.

pub mod cache;
pub mod dto;
pub mod engine;
pub mod images;
pub mod import;
pub mod limiter;
pub mod mapping;
pub mod screenshots;
pub mod sync;
pub mod ulist;

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::vndb::dto::{VnQuery, VnQueryResult};
use crate::vndb::limiter::RateLimiter;

/// Base URL for the production API.
pub const API_BASE: &str = "https://api.vndb.org/kana";

/// The API asks clients to identify themselves; a descriptive agent is also how
/// problems get diagnosed on their side.
pub const USER_AGENT: &str = concat!(
    "VN-Tracker/",
    env!("CARGO_PKG_VERSION"),
    " (offline-first VN library; +https://vndb.org/d17)"
);

/// Failures a VNDB call can produce, mapped to what the UI should say.
/// Turn a non-success HTTP status into the error the caller should branch on.
///
/// One mapping for every request path: a 404 used to arrive as a generic
/// [`VndbError::Server`], indistinguishable from a 500, so one VN that VNDB had
/// deleted failed an entire refresh instead of falling back to its cache.
fn error_for_status(status: u16, body: &str) -> VndbError {
    match status {
        404 => VndbError::NotFound,
        status => VndbError::Server {
            status,
            body: truncate(body),
        },
    }
}

#[derive(Debug, Error)]
pub enum VndbError {
    /// The network is unreachable (offline, DNS, TLS).
    #[error("network unavailable: {0}")]
    Offline(String),
    /// The server said the request was malformed.
    #[error("invalid query: {0}")]
    BadRequest(String),
    /// The API token was rejected.
    #[error("the VNDB token was rejected")]
    Unauthorized,
    /// Still throttled after all retries were spent.
    #[error("VNDB is throttling requests; try again in a moment")]
    Throttled,
    /// The entry does not exist upstream any more.
    ///
    /// Distinct from a server error: a VN deleted from VNDB must fall back to the
    /// cache and be counted as skipped, not fail an entire refresh run. Callers that
    /// only need "not there" can treat this as a permanent, non-retryable answer.
    #[error("VNDB has no entry for this id (HTTP 404)")]
    NotFound,
    /// The server returned an error status we do not special-case.
    #[error("VNDB returned HTTP {status}: {body}")]
    Server {
        /// HTTP status code.
        status: u16,
        /// Response body, truncated for display.
        body: String,
    },
    /// The response was not the JSON shape we expect.
    #[error("unexpected response from VNDB: {0}")]
    Decode(String),
    /// The query asked for more data than the API allows.
    #[error("query selects too much data; narrow the fields or results")]
    TooMuchData,
}

/// A configured client. Cheap to clone; the limiter is shared.
#[derive(Debug, Clone)]
pub struct VndbClient {
    http: reqwest::Client,
    base: String,
    limiter: RateLimiter,
}

/// Return an HTTP [`reqwest::ClientBuilder`] pre-configured with root
/// certificates on Android so `rustls` uses pure-Rust `WebPKI` verification
/// instead of calling into `rustls-platform-verifier`'s uninitialised JNI
/// bridge.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = {
        let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok());
        builder.tls_certs_only(certs)
    };
    builder
}

impl VndbClient {
    /// Build a client against the production endpoint.
    ///
    /// # Errors
    /// Fails when the TLS client cannot be constructed.
    pub fn new() -> Result<Self, VndbError> {
        Self::with_base(API_BASE)
    }

    /// Build a client against an arbitrary base URL (used by tests and the
    /// VNDB sandbox).
    ///
    /// # Errors
    /// Fails when the HTTP client cannot be constructed.
    pub fn with_base(base: impl Into<String>) -> Result<Self, VndbError> {
        let http = http_client_builder()
            .user_agent(USER_AGENT)
            // The API aborts requests taking longer than 3 seconds; give the
            // connection a little more room than that, then give up.
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|err| VndbError::Offline(err.to_string()))?;
        Ok(Self {
            http,
            base: base.into(),
            limiter: RateLimiter::new(),
        })
    }

    /// Search visual novels by title.
    ///
    /// # Errors
    /// Propagates network, throttling and decoding failures.
    pub async fn search_vns(&self, query: &VnQuery) -> Result<VnQueryResult, VndbError> {
        self.post_json("vn", query).await
    }

    /// Fetch one visual novel by id, or `None` when it does not exist.
    ///
    /// # Errors
    /// Propagates network, throttling and decoding failures.
    pub async fn get_vn(&self, vn_id: &str, fields: &str) -> Result<Option<dto::Vn>, VndbError> {
        // `searchrank` sorting is rejected by the API unless the top-level filter
        // is a `search` (verified against the live endpoint), so an id lookup
        // sorts by `id` instead.
        let query = VnQuery {
            filters: serde_json::json!(["id", "=", vn_id]),
            fields: fields.to_owned(),
            results: 1,
            sort: "id".to_owned(),
            ..VnQuery::default()
        };
        let result = self.search_vns(&query).await?;
        Ok(result.results.into_iter().next())
    }

    /// The cast for a VN, with the role each character plays in *this* VN.
    ///
    /// **One request.** `POST /character` accepts a `vn` filter and returns the
    /// whole cast with `vns.role` included, which is the only way to get a role:
    /// `POST /vn` reaches characters through `va` (one row per voice actor) and
    /// omits the role entirely.
    ///
    /// An earlier version asked for one character at a time, having found that the
    /// documented `id` filter rejects an array. That worked but cost a request per
    /// character — 23 for CLANNAD — and at the 1.5 s documented spacing it made a
    /// metadata refresh take minutes per game. The `vn` filter does the same job in
    /// one call.
    ///
    /// `va` is still used as the fallback: it carries a portrait, and the character
    /// endpoint is asked for one too, but a VN whose `va` is present while the cast
    /// query fails should still show something.
    ///
    /// # Errors
    /// Propagates network, throttling and decoding failures.
    pub async fn get_vn_cast(
        &self,
        vn_id: &str,
        va: &[dto::VoiceActor],
    ) -> Result<Vec<dto::Character>, VndbError> {
        let mut cast: Vec<dto::Character> = Vec::new();
        let mut page = 1_u32;
        loop {
            let query = dto::CharacterQuery {
                filters: serde_json::json!(["vn", "=", ["id", "=", vn_id]]),
                // Traits and measurements come from this same endpoint; asking
                // for them here is what keeps the cast to one request per VN.
                fields: "id,name,original,image.url,vns.id,vns.role,traits.id,traits.name,traits.group_name,traits.spoiler,traits.sexual,traits.lie,birthday,sex,gender,height,weight,blood_type"
                    .to_owned(),
                results: 100,
                page,
            };
            let result: dto::CharacterQueryResult = self.post_json("character", &query).await?;
            for entry in &result.results {
                let role = entry
                    .vns
                    .iter()
                    .find(|v| v.id.as_deref() == Some(vn_id))
                    .and_then(|v| v.role.clone());
                cast.push(dto::Character {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    original: entry.original.clone(),
                    role,
                    image: entry.image.clone(),
                    traits: entry.traits.clone(),
                    birthday: entry.birthday.clone(),
                    height: entry.height,
                    weight: entry.weight,
                    blood_type: entry.blood_type.clone(),
                    voiced_by: None,
                });
            }
            if !result.more || page >= 10 {
                break;
            }
            page += 1;
        }

        // Fall back to the `va` list when the cast query came back empty but the VN
        // did report voice actors: a blank card is worse than a partial one.
        if cast.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for entry in va {
                if let Some(character) = entry.character.clone()
                    && let Some(id) = character.id.clone()
                    && seen.insert(id)
                {
                    cast.push(character);
                }
            }
        }

        // Voice actors live on the VN's `va` relation, not on the character, so
        // they are joined here rather than costing another request per person.
        for character in &mut cast {
            let Some(id) = character.id.as_deref() else {
                continue;
            };
            character.voiced_by = va
                .iter()
                .find(|entry| {
                    entry
                        .character
                        .as_ref()
                        .and_then(|c| c.id.as_deref())
                        .is_some_and(|cid| cid == id)
                })
                .and_then(|entry| entry.staff.as_ref())
                .and_then(|staff| staff.name.clone().or_else(|| staff.original.clone()));
        }
        Ok(cast)
    }

    /// Validate a token and return the account it belongs to.
    ///
    /// # Errors
    /// Returns [`VndbError::Unauthorized`] for a rejected token.
    pub async fn authinfo(&self, token: &str) -> Result<dto::AuthInfo, VndbError> {
        self.limiter.acquire().await;
        let response = self
            .http
            .get(format!("{}/authinfo", self.base))
            .header("Authorization", format!("token {token}"))
            .send()
            .await
            .map_err(|err| map_transport(&err))?;

        match response.status().as_u16() {
            200 => response
                .json::<dto::AuthInfo>()
                .await
                .map_err(|err| VndbError::Decode(err.to_string())),
            401 => Err(VndbError::Unauthorized),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(error_for_status(status, &body))
            }
        }
    }

    /// Download an image into memory, used for cover caching.
    ///
    /// # Errors
    /// Returns [`VndbError::Offline`] when the image cannot be fetched.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, VndbError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| map_transport(&err))?;
        if !response.status().is_success() {
            return Err(error_for_status(response.status().as_u16(), ""));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|err| VndbError::Offline(err.to_string()))
    }

    /// Fetch one page of a user's visual novel list.
    ///
    /// # Errors
    /// Propagates network, throttling, auth and decoding failures.
    pub async fn ulist_page(
        &self,
        token: &str,
        query: &ulist::UlistQuery,
    ) -> Result<ulist::UlistResponse, VndbError> {
        self.post_json_auth("ulist", query, token).await
    }

    /// Fetch the list labels for a user.
    ///
    /// # Errors
    /// Propagates network, throttling, auth and decoding failures.
    pub async fn ulist_labels(
        &self,
        token: &str,
        user_id: &str,
    ) -> Result<ulist::LabelsResponse, VndbError> {
        self.limiter.acquire().await;
        // The user id is a `u` followed by digits, so it needs no escaping; the
        // URL is built directly to avoid depending on reqwest's `query` feature
        // for a single call.
        let url = format!(
            "{}/ulist_labels?user={}",
            self.base,
            sanitise_query(user_id)
        );
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("token {token}"))
            .send()
            .await
            .map_err(|err| map_transport(&err))?;

        match response.status().as_u16() {
            200 => response
                .json::<ulist::LabelsResponse>()
                .await
                .map_err(|err| VndbError::Decode(err.to_string())),
            401 => Err(VndbError::Unauthorized),
            other => {
                let body = response.text().await.unwrap_or_default();
                Err(error_for_status(other, &body))
            }
        }
    }

    /// Add or update a VN on the user's list.
    ///
    /// `PATCH` returns `204 No Content` on success. The endpoint always adds the
    /// VN to the list if it is absent, which is documented behaviour and exactly
    /// what a push wants.
    ///
    /// # Errors
    /// Propagates network, throttling, auth and bad-request failures.
    pub async fn ulist_patch(
        &self,
        token: &str,
        vn_id: &str,
        patch: &ulist::UlistPatch,
    ) -> Result<(), VndbError> {
        self.send_patch(&format!("ulist/{vn_id}"), patch, token)
            .await
    }

    /// Remove a VN from the user's list.
    ///
    /// # Errors
    /// Propagates network, throttling and auth failures.
    pub async fn ulist_delete(&self, token: &str, vn_id: &str) -> Result<(), VndbError> {
        self.limiter.acquire().await;
        let response = self
            .http
            .delete(format!("{}/ulist/{vn_id}", self.base))
            .header("Authorization", format!("token {token}"))
            .send()
            .await
            .map_err(|err| map_transport(&err))?;

        match response.status().as_u16() {
            200 | 204 => Ok(()),
            401 => Err(VndbError::Unauthorized),
            429 => Err(VndbError::Throttled),
            other => {
                let body = response.text().await.unwrap_or_default();
                Err(error_for_status(other, &body))
            }
        }
    }

    async fn send_patch<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
        token: &str,
    ) -> Result<(), VndbError> {
        let url = format!("{}/{path}", self.base);
        let mut attempt = 0u32;

        loop {
            self.limiter.acquire().await;
            let response = self
                .http
                .patch(&url)
                .header("Authorization", format!("token {token}"))
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(body).map_err(|err| VndbError::Decode(err.to_string()))?)
                .send()
                .await
                .map_err(|err| map_transport(&err))?;

            match response.status().as_u16() {
                200 | 204 => return Ok(()),
                429 | 500 | 502 | 503 => {
                    attempt += 1;
                    if attempt > 4 {
                        return Err(VndbError::Throttled);
                    }
                    let delay = Duration::from_millis(500 * (1u64 << (attempt - 1)));
                    tokio::time::sleep(delay).await;
                }
                400 => {
                    let body = response.text().await.unwrap_or_default();
                    return Err(VndbError::BadRequest(truncate(&body)));
                }
                401 => return Err(VndbError::Unauthorized),
                other => {
                    let body = response.text().await.unwrap_or_default();
                    return Err(error_for_status(other, &body));
                }
            }
        }
    }

    /// POST a query body with a token, retrying throttling and transient server
    /// errors with exponential backoff.
    async fn post_json_auth<T, B>(&self, path: &str, body: &B, token: &str) -> Result<T, VndbError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let url = format!("{}/{path}", self.base);
        let mut attempt = 0u32;

        loop {
            self.limiter.acquire().await;
            let response = self
                .http
                .post(&url)
                .header("Authorization", format!("token {token}"))
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(body).map_err(|err| VndbError::Decode(err.to_string()))?)
                .send()
                .await
                .map_err(|err| map_transport(&err))?;

            match response.status().as_u16() {
                200 => {
                    return response
                        .json::<T>()
                        .await
                        .map_err(|err| VndbError::Decode(err.to_string()));
                }
                429 | 500 | 502 | 503 => {
                    attempt += 1;
                    if attempt > 4 {
                        return Err(VndbError::Throttled);
                    }
                    let delay = Duration::from_millis(500 * (1u64 << (attempt - 1)));
                    tokio::time::sleep(delay).await;
                }
                400 => {
                    let body = response.text().await.unwrap_or_default();
                    if body.contains("Too much data selected") {
                        return Err(VndbError::TooMuchData);
                    }
                    return Err(VndbError::BadRequest(truncate(&body)));
                }
                401 => return Err(VndbError::Unauthorized),
                other => {
                    let body = response.text().await.unwrap_or_default();
                    return Err(error_for_status(other, &body));
                }
            }
        }
    }

    /// POST a query body and decode the response, retrying throttling and
    /// transient server errors with exponential backoff.
    pub(crate) async fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, VndbError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let url = format!("{}/{path}", self.base);
        let mut attempt = 0u32;

        loop {
            self.limiter.acquire().await;
            let response = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(body).map_err(|err| VndbError::Decode(err.to_string()))?)
                .send()
                .await
                .map_err(|err| map_transport(&err))?;

            let status = response.status().as_u16();
            match status {
                200 => {
                    return response
                        .json::<T>()
                        .await
                        .map_err(|err| VndbError::Decode(err.to_string()));
                }
                429 | 500 | 502 | 503 => {
                    attempt += 1;
                    if attempt > 4 {
                        return Err(VndbError::Throttled);
                    }
                    // Exponential backoff: 0.5 s, 1 s, 2 s, 4 s.
                    let delay = Duration::from_millis(500 * (1u64 << (attempt - 1)));
                    tokio::time::sleep(delay).await;
                }
                400 => {
                    let body = response.text().await.unwrap_or_default();
                    if body.contains("Too much data selected") {
                        return Err(VndbError::TooMuchData);
                    }
                    return Err(VndbError::BadRequest(truncate(&body)));
                }
                401 => return Err(VndbError::Unauthorized),
                other => {
                    let body = response.text().await.unwrap_or_default();
                    return Err(error_for_status(other, &body));
                }
            }
        }
    }
}

/// Every transport-level failure is treated as "offline": from the user's point
/// of view an unreachable server, a DNS failure and a TLS error all mean the same
/// thing, and the cached-data fallback is the same response.
fn map_transport(err: &reqwest::Error) -> VndbError {
    VndbError::Offline(err.to_string())
}

/// Keep only characters that are safe in a query value.
fn sanitise_query(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(64)
        .collect()
}

fn truncate(text: &str) -> String {
    const LIMIT: usize = 300;
    if text.len() <= LIMIT {
        return text.to_owned();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod error_for_status_tests {
    use super::{VndbError, error_for_status};

    #[test]
    fn a_404_is_its_own_error_not_a_server_error() {
        // The two used to be the same variant, so a VN deleted from VNDB failed the
        // whole refresh with the same shape as a 500 and could not be skipped.
        assert!(matches!(error_for_status(404, ""), VndbError::NotFound));
    }

    #[test]
    fn a_500_still_carries_its_status_for_display() {
        match error_for_status(500, "boom") {
            VndbError::Server { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "boom");
            }
            other => panic!("expected Server, got {other:?}"),
        }
    }
}
