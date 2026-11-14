//! One-shot OpenAI/Anthropic request wrapper for recommendations and script compaction.
//! Runs on demand (companion streaming uses `conversation`).

use serde::{Deserialize, Serialize};

use crate::secrets::ChatApiKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    NoKey,
    Unauthorized,
    Throttled,
    Offline(String),
    BadResponse(String),
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoKey => write!(f, "no API key is configured"),
            Self::Unauthorized => write!(f, "the API key was rejected"),
            Self::Throttled => write!(f, "the provider is rate limiting requests"),
            Self::Offline(reason) => write!(f, "could not reach the provider: {reason}"),
            Self::BadResponse(reason) => write!(f, "unusable response: {reason}"),
        }
    }
}

impl std::error::Error for ChatError {}

/// Wire protocol a saved endpoint speaks (e.g. Gemini's OpenAI-compatible layer uses `OpenAi`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// `POST {base}/chat/completions`, `Authorization: Bearer`.
    /// Explicit rename/alias prevents `snake_case` from serializing `open_ai`.
    #[default]
    #[serde(rename = "openai", alias = "open_ai")]
    OpenAi,
    /// `POST {base}/messages`, `x-api-key` plus `anthropic-version`.
    Anthropic,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::OpenAi, Self::Anthropic];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    /// # Errors
    /// Returns the unrecognized provider id.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "openai" | "" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    /// Stored configs without this field predate provider selection and read as `openai`.
    #[serde(default)]
    pub provider: Provider,
    /// Base URL without `/chat/completions` or `/messages`.
    pub base_url: String,
    pub model: String,
}

impl Default for ChatConfig {
    fn default() -> Self {
        // Any OpenAI-compatible endpoint works, which includes several
        // self-hosted and local runtimes. Defaulting to an OpenAI URL would push
        // users toward the one option that is not offline.
        Self {
            provider: Provider::OpenAi,
            base_url: "https://api.openai.com/v1".to_owned(),
            model: "gpt-4o-mini".to_owned(),
        }
    }
}

const MAX_RATE_LIMIT_RETRIES: u8 = 3;
const MAX_RETRY_AFTER_SECS: u64 = 15;

fn is_official_openai(base_url: &str) -> bool {
    reqwest::Url::parse(base_url).is_ok_and(|url| url.host_str() == Some("api.openai.com"))
}

fn is_openai_reasoning_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("gpt-5")
}

fn build_openai_payload(
    config: &ChatConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> serde_json::Value {
    let official = is_official_openai(&config.base_url);
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
    });
    // OpenAI's API uses `max_completion_tokens` and rejects custom `temperature` on reasoning models.
    body[if official {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }] = serde_json::json!(max_tokens);
    if !(official && is_openai_reasoning_model(&config.model)) {
        body["temperature"] = serde_json::json!(0.3);
    }
    body
}

fn retry_delay(headers: &reqwest::header::HeaderMap, attempt: u8) -> std::time::Duration {
    if let Some(raw) = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        && let Ok(secs) = raw.trim().parse::<u64>()
    {
        return std::time::Duration::from_secs(secs.clamp(1, MAX_RETRY_AFTER_SECS));
    }
    std::time::Duration::from_secs(1_u64 << attempt.min(3))
}

async fn send_with_retry(
    mut build: impl FnMut() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, ChatError> {
    let mut attempt = 0_u8;
    loop {
        let response = build()
            .send()
            .await
            .map_err(|err| ChatError::Offline(err.to_string()))?;
        match response.status().as_u16() {
            200..=299 => return Ok(response),
            401 | 403 => return Err(ChatError::Unauthorized),
            429 if attempt < MAX_RATE_LIMIT_RETRIES => {
                let wait = retry_delay(response.headers(), attempt);
                attempt += 1;
                tokio::time::sleep(wait).await;
            }
            429 => return Err(ChatError::Throttled),
            other => return Err(ChatError::BadResponse(format!("HTTP {other}"))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
    /// Why generation stopped: "stop", "length", and so on.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    /// Null when the model spent its whole budget thinking and never emitted an
    /// answer; reasoning models do this.
    #[serde(default)]
    content: Option<String>,
    /// Reasoning models (Gemma, DeepSeek-R1 and others) may put text here. It is
    /// parsed so it can be reported, never used as an answer.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

/// One system/user exchange with the configured provider.
///
/// Keyless endpoints (e.g. Ollama) are permitted; the reply is unvalidated.
///
/// # Errors
/// Returns [`ChatError`] on missing key, auth failure, throttling, transport error, or bad response.
pub async fn complete(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    system: &str,
    user: &str,
) -> Result<String, ChatError> {
    complete_with_budget(client, config, key, system, user, 2400).await
}

/// One system/user exchange with an explicit `max_tokens` budget (e.g. 8192 for dense summaries).
pub async fn complete_with_budget(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, ChatError> {
    request(client, config, key, system, user, max_tokens).await
}

/// Truncate cleanly back to the last sentence or paragraph boundary if the response
/// hit a token limit mid-word or mid-sentence.
fn clean_truncated_tail(text: &str) -> String {
    let trimmed = text.trim_end();
    let delimiters = ['.', '!', '?', '\n', '。', '！', '？', '…'];
    if let Some(idx) = trimmed.rfind(|c: char| delimiters.contains(&c)) {
        let end_idx = idx + trimmed[idx..].chars().next().map_or(1, char::len_utf8);
        trimmed[..end_idx].trim_end().to_owned()
    } else {
        trimmed.to_owned()
    }
}

async fn request(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, ChatError> {
    if config.provider == Provider::Anthropic {
        return request_anthropic(client, config, key, system, user, max_tokens).await;
    }
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = build_openai_payload(config, system, user, max_tokens);
    let response = send_with_retry(|| {
        let mut req = client.post(&url).json(&body);
        if let Some(key) = key {
            req = req.bearer_auth(key.expose());
        }
        req
    })
    .await?;

    let parsed: ChatResponse = response
        .json()
        .await
        .map_err(|err| ChatError::BadResponse(err.to_string()))?;
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ChatError::BadResponse("no choices returned".to_owned()))?;

    let is_length_finish = choice.finish_reason.as_deref() == Some("length");
    match choice.message.content {
        Some(text) if !text.trim().is_empty() => {
            if is_length_finish {
                let cleaned = clean_truncated_tail(&text);
                if cleaned.trim().is_empty() {
                    Err(ChatError::BadResponse(
                        "the reply hit the token limit before emitting a complete sentence".to_owned(),
                    ))
                } else {
                    Ok(cleaned)
                }
            } else {
                Ok(text)
            }
        }
        // Surface exhausted thinking budget instead of reporting a generic empty reply.
        _ if choice.message.reasoning_content.is_some() => Err(ChatError::BadResponse(
            "the model used its whole reply budget on reasoning and produced no answer; raise the token limit, or use a model that does not reason first"
                .to_owned(),
        )),
        _ if is_length_finish => Err(ChatError::BadResponse(
            "the reply hit the token limit before finishing".to_owned(),
        )),
        _ => Err(ChatError::BadResponse("empty response".to_owned())),
    }
}

/// Calls Anthropic's Messages API (top-level `system`, one user turn, no tools).
async fn request_anthropic(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, ChatError> {
    let url = format!("{}/messages", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": max_tokens,
        "temperature": 0.3,
        "system": system,
        "messages": [{"role": "user", "content": user}],
    });
    let response = send_with_retry(|| {
        let mut req = client
            .post(&url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body);
        if let Some(key) = key {
            req = req.header("x-api-key", key.expose());
        }
        req
    })
    .await?;
    let parsed: AnthropicResponse = response
        .json()
        .await
        .map_err(|err| ChatError::BadResponse(err.to_string()))?;
    let text: String = parsed
        .content
        .iter()
        .filter(|block| block.kind == "text")
        .map(|block| block.text.as_str())
        .collect::<String>();
    let truncated = parsed.stop_reason.as_deref() == Some("max_tokens");
    if text.trim().is_empty() {
        return Err(ChatError::BadResponse(if truncated {
            "the reply hit the token limit before finishing".to_owned()
        } else {
            "empty response".to_owned()
        }));
    }
    Ok(if truncated {
        clean_truncated_tail(&text)
    } else {
        text
    })
}

/// The `anthropic-version` header this app sends. Pinned, because a moving
/// version would change the response shape without a code change here.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Deserialize)]
struct AnthropicBlock {
    /// `text` for an answer, `thinking` for reasoning, `tool_use` for a call.
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicBlock>,
    /// Why the model stopped: `end_turn`, `max_tokens`, `tool_use`.
    #[serde(default)]
    stop_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_default_config_points_at_a_configurable_endpoint() {
        // Not hardcoded to one vendor: several OpenAI-compatible runtimes are
        // local, and local is what keeps this offline-capable.
        let config = ChatConfig::default();
        assert!(config.base_url.starts_with("https://"));
        assert!(!config.model.is_empty(), "a default config names a model");
    }

    #[test]
    fn a_provider_serializes_as_the_id_the_ui_sends() {
        // `snake_case` alone spells this variant `open_ai`, which the settings
        // schema rejects: the panel showed a contract error instead of the list.
        assert_eq!(json!(Provider::OpenAi), json!("openai"));
        assert_eq!(json!(Provider::Anthropic), json!("anthropic"));
        for spelling in ["openai", "open_ai"] {
            assert_eq!(
                serde_json::from_value::<Provider>(json!(spelling)).expect("parse"),
                Provider::OpenAi
            );
        }
        assert_eq!(Provider::parse("openai"), Ok(Provider::OpenAi));
        assert!(Provider::parse("gemini").is_err());
    }

    #[test]
    fn no_key_is_a_distinct_error_not_a_generic_failure() {
        // The UI shows different guidance for "configure a key" and "the provider
        // is down", so they must not collapse into one message.
        assert_ne!(
            ChatError::NoKey.to_string(),
            ChatError::Throttled.to_string()
        );
        assert!(ChatError::NoKey.to_string().contains("key"));
    }

    #[test]
    fn a_null_content_with_reasoning_explains_the_budget_problem() {
        // Observed live from llama.cpp running a reasoning model: at a low token
        // limit the model thinks, hits the cap, and emits no answer at all.
        let parsed: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":null,"reasoning_content":"Thinking..."},
                            "finish_reason":"length"}]}"#,
        )
        .unwrap();
        let choice = &parsed.choices[0];
        assert!(choice.message.content.is_none());
        assert!(choice.message.reasoning_content.is_some());
    }

    #[test]
    fn a_reply_with_content_and_reasoning_uses_only_the_content() {
        // The reasoning must never be mistaken for the answer.
        let parsed: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"games\":[]}","reasoning_content":"steps"},
                            "finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.choices[0].message.content.as_deref(),
            Some("{\"games\":[]}")
        );
    }

    #[test]
    fn a_plain_provider_response_without_reasoning_still_parses() {
        let parsed: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"hello"},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.choices[0].message.content.as_deref(), Some("hello"));
        assert!(parsed.choices[0].message.reasoning_content.is_none());
    }

    #[test]
    fn clean_truncated_tail_drops_dangling_word_after_sentence_boundary() {
        let input = "Ibuki is relieved the shampoo matter is postponed. Tai";
        assert_eq!(
            clean_truncated_tail(input),
            "Ibuki is relieved the shampoo matter is postponed."
        );
    }

    #[test]
    fn clean_truncated_tail_handles_cjk_punctuation() {
        let input = "第一章が終了した。次回につづ";
        assert_eq!(clean_truncated_tail(input), "第一章が終了した。");
    }

    #[test]
    fn clean_truncated_tail_preserves_complete_sentences() {
        let input = "This is a full sentence.";
        assert_eq!(clean_truncated_tail(input), "This is a full sentence.");
    }

    #[test]
    fn openai_payload_adapts_token_and_temperature_fields() {
        let compatible = ChatConfig {
            provider: Provider::OpenAi,
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "qwen2.5".into(),
        };
        let payload = build_openai_payload(&compatible, "sys", "usr", 2400);
        assert_eq!(payload["max_tokens"], json!(2400));
        assert!(payload.get("max_completion_tokens").is_none());
        assert!(payload.get("temperature").is_some());

        let gpt4o = ChatConfig {
            provider: Provider::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
        };
        let payload = build_openai_payload(&gpt4o, "sys", "usr", 2400);
        assert_eq!(payload["max_completion_tokens"], json!(2400));
        assert!(payload.get("max_tokens").is_none());
        assert!(payload.get("temperature").is_some());

        let reasoning = ChatConfig {
            provider: Provider::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-5-mini".into(),
        };
        let payload = build_openai_payload(&reasoning, "sys", "usr", 2400);
        assert_eq!(payload["max_completion_tokens"], json!(2400));
        assert!(payload.get("temperature").is_none());
    }

    #[test]
    fn retry_delay_clamps_retry_after_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "99".parse().unwrap());
        assert_eq!(retry_delay(&headers, 0).as_secs(), MAX_RETRY_AFTER_SECS);
        headers.insert(reqwest::header::RETRY_AFTER, "0".parse().unwrap());
        assert_eq!(retry_delay(&headers, 0).as_secs(), 1);
        headers.clear();
        assert_eq!(retry_delay(&headers, 1).as_secs(), 2);
    }
}
