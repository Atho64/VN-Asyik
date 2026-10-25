//! OpenAI-compatible streaming function calling. No provider response bodies are logged.
use crate::{ai::chat::ChatConfig, secrets::ChatApiKey};
use serde_json::{Value, json};
use vn_core::chat::{CompletionStream, tool_definitions};

pub async fn complete(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    messages: &[Value],
    online: bool,
    on_delta: &mut impl FnMut(String),
) -> Result<CompletionStream, String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut body = json!({"model":config.model,"messages":messages,"tools":tool_definitions(online),"tool_choice":"auto","stream":true});
    // OpenAI's current API uses max_completion_tokens; compatible local servers
    // commonly retain max_tokens. Never silently retry a billable generation.
    let official =
        reqwest::Url::parse(&config.base_url).is_ok_and(|u| u.host_str() == Some("api.openai.com"));
    body[if official {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }] = json!(4096);
    let mut request = client.post(url).json(&body);
    if let Some(key) = key {
        request = request.bearer_auth(key.expose());
    }
    let mut response = request.send().await.map_err(|_| "provider_offline")?;
    match response.status().as_u16() {
        200..=299 => (),
        401 | 403 => return Err("provider_unauthorized".into()),
        429 => return Err("provider_throttled".into()),
        400 | 404 | 422 => return Err("provider_incompatible".into()),
        _ => return Err("provider_error".into()),
    }
    let is_json = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("application/json"));
    let mut stream = CompletionStream::default();
    let mut json_bytes = Vec::new();
    let mut total = 0_usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| "provider_offline")? {
        total += chunk.len();
        if total > 2_000_000 {
            return Err("provider_response_too_large".into());
        }
        if is_json {
            json_bytes.extend_from_slice(&chunk);
        } else {
            for delta in stream.push(&chunk)? {
                on_delta(delta);
            }
            if stream.done {
                break;
            }
        }
    }
    if is_json {
        let value: Value =
            serde_json::from_slice(&json_bytes).map_err(|_| "invalid_provider_json")?;
        for delta in stream.chunk(&value)? {
            on_delta(delta);
        }
    }
    stream.validate_finished()?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn sends_real_tool_schema_and_reads_streamed_call() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut buffer = [0_u8; 8192];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let body = String::from_utf8(request)
                .unwrap()
                .split_once("\r\n\r\n")
                .unwrap()
                .1
                .to_owned();
            let sent: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(sent["stream"], true);
            // The five library tools plus the two companion tools
            // (recent_game_text, script_digest).
            assert_eq!(sent["tools"].as_array().unwrap().len(), 7);
            let event=json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_stats","type":"function","function":{"name":"library_stats","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}).to_string();
            let data = format!("data: {event}\n\ndata: [DONE]\n\n");
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",data.len()).as_bytes()).await.unwrap();
        });
        let config = ChatConfig {
            base_url: base,
            model: "test".into(),
        };
        let stream = complete(
            &crate::vndb::http_client_builder().build().unwrap(),
            &config,
            None,
            &[json!({"role":"user","content":"stats"})],
            false,
            &mut |_| {},
        )
        .await
        .unwrap();
        assert_eq!(stream.calls[&0].function.name, "library_stats");
        server.await.unwrap();
    }
}
