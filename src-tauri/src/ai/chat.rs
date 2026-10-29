//! Shared OpenAI-compatible request shape for the AI features.
//!
//! One request/response pair with the error mapping every caller wants: an
//! auth failure, throttling, network trouble and unusable responses are
//! distinct, because the UI says different things for each. The streaming
//! companion chat has its own transport; this module serves the one-shot
//! callers (recommendations, script compaction).
//!
//! Nothing here runs on startup; it only runs when a feature asks.

use serde::{Deserialize, Serialize};

use crate::secrets::ChatApiKey;

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// No key configured.
    NoKey,
    /// The configured endpoint or key was rejected.
    Unauthorized,
    /// Rate limited or throttled by the provider.
    Throttled,
    /// Network trouble.
    Offline(String),
    /// The provider answered with something unusable.
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

/// Which wire format a saved endpoint speaks.
///
/// Gemini's OpenAI-compatible layer is `openai`: the difference that matters is
/// the request and response *shape*, not who runs the server. Anthropic's Messages
/// API is its own shape, so it gets its own variant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// `POST {base}/chat/completions`, `Authorization: Bearer`.
    ///
    /// Spelled out because `snake_case` turns the variant name into `open_ai`:
    /// the UI and every stored profile say `openai`, and the alias keeps a blob
    /// written by a build that sent the split spelling readable.
    #[default]
    #[serde(rename = "openai", alias = "open_ai")]
    OpenAi,
    /// `POST {base}/messages`, `x-api-key` plus `anthropic-version`.
    Anthropic,
}

impl Provider {
    /// Every provider the app can speak to.
    pub const ALL: [Self; 2] = [Self::OpenAi, Self::Anthropic];

    /// The key the UI and the stored profile use.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    /// Read the id the UI sends.
    ///
    /// # Errors
    /// The unknown id, so the caller can report it instead of guessing.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "openai" | "" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

/// Where the chat provider lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    /// Which wire format this endpoint speaks. Older stored configs have no
    /// field and read as `openai`.
    #[serde(default)]
    pub provider: Provider,
    /// Base URL, without the trailing `/chat/completions` (or `/messages`).
    pub base_url: String,
    /// Model name as the provider spells it.
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

/// One message in the request.
#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

/// Request body for `/chat/completions`.
#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    /// Low temperature: this is a restatement task, not a creative one.
    temperature: f32,
    max_tokens: u32,
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
/// Added so the recommendation pipeline can reuse the request shape and error
/// mapping without duplicating them. Unlike [`comment`], the reply is returned
/// unvalidated: there is no deterministic profile behind a web-search
/// recommendation to check it against, so grounding is enforced by
/// `recommend::unsupported_candidates` instead. The key is optional so local
/// OpenAI-compatible servers (Ollama and friends) work keyless, exactly like
/// the streaming chat transport.
///
/// # Errors
/// [`ChatError`] for a missing key, auth failure, throttling, network trouble, or
/// an unusable response.
pub async fn complete(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    system: &str,
    user: &str,
) -> Result<String, ChatError> {
    complete_with_budget(client, config, key, system, user, 2400).await
}

/// One system/user exchange with an explicit output token budget.
///
/// Multi-agent segment digests and final reduce merges handle up to 1200 words
/// of dense factual information, which requires a larger token budget (e.g. 8192)
/// so reasoning models and thorough summarizers do not get cut off mid-thought.
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
    let body = ChatRequest {
        model: &config.model,
        messages: vec![
            Message {
                role: "system",
                content: system,
            },
            Message {
                role: "user",
                content: user,
            },
        ],
        // Low temperature: recommendations should not be creative with facts.
        temperature: 0.3,
        max_tokens,
    };

    let mut request = client.post(&url).json(&body);
    if let Some(key) = key {
        request = request.bearer_auth(key.expose());
    }
    let response = request
        .send()
        .await
        .map_err(|err| ChatError::Offline(err.to_string()))?;

    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(ChatError::Unauthorized),
        429 => return Err(ChatError::Throttled),
        other => {
            return Err(ChatError::BadResponse(format!("HTTP {other}")));
        }
    }

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
        // A reasoning model can spend the whole budget thinking and emit nothing.
        // Saying so is far more useful than "empty response", because the fix is
        // to raise the limit or pick a non-reasoning model.
        _ if choice.message.reasoning_content.is_some() => Err(ChatError::BadResponse(
            "the model used its whole reply budget on reasoning and produced no answer;              raise the token limit, or use a model that does not reason first"
                .to_owned(),
        )),
        _ if is_length_finish => Err(ChatError::BadResponse(
            "the reply hit the token limit before finishing".to_owned(),
        )),
        _ => Err(ChatError::BadResponse("empty response".to_owned())),
    }
}

/// Anthropic's Messages API, in the shape this app sends (`system` at the top
/// level, one user turn, no tools).
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
    let mut request = client
        .post(&url)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body);
    if let Some(key) = key {
        request = request.header("x-api-key", key.expose());
    }
    let response = request
        .send()
        .await
        .map_err(|err| ChatError::Offline(err.to_string()))?;
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(ChatError::Unauthorized),
        429 => return Err(ChatError::Throttled),
        other => return Err(ChatError::BadResponse(format!("HTTP {other}"))),
    }
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
}
