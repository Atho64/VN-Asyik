//! Anthropic Messages API: streaming function calling.
//!
//! The Messages API is not OpenAI-shaped, so this translates in both directions:
//!
//! * the conversation the chat loop keeps is OpenAI-shaped (`tool_calls` on an
//!   assistant turn, `role: "tool"` for a result), which maps to `tool_use` and
//!   `tool_result` blocks;
//! * the stream is a typed event stream (`content_block_start`,
//!   `content_block_delta`, ...) rather than `choices[].delta`, which this reads
//!   back into the same [`CompletionStream`] the `OpenAI` path produces, so the
//!   loop above does not care which provider answered.
//!
//! No provider response body is ever logged.

use crate::ai::chat::{ANTHROPIC_VERSION, ChatConfig};
use crate::secrets::ChatApiKey;
use serde_json::{Value, json};
use vn_core::chat::{CompletionStream, ToolCall, tool_definitions};

pub async fn complete(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    messages: &[Value],
    online: bool,
    on_delta: &mut impl FnMut(String),
) -> Result<CompletionStream, String> {
    let url = format!("{}/messages", config.base_url.trim_end_matches('/'));
    let (system, turns) = convert(messages);
    let body = json!({
        "model": config.model,
        "max_tokens": 4096,
        "system": system,
        "messages": turns,
        "tools": tools(online),
        "tool_choice": {"type": "auto"},
        "stream": true,
    });
    let mut request = client
        .post(url)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body);
    if let Some(key) = key {
        request = request.header("x-api-key", key.expose());
    }
    let mut response = request.send().await.map_err(|_| "provider_offline")?;
    match response.status().as_u16() {
        200..=299 => (),
        401 | 403 => return Err("provider_unauthorized".into()),
        // 529 is Anthropic's "overloaded", which is throttling with a different name.
        400 | 404 | 422 => return Err("provider_incompatible".into()),
        429 | 529 => return Err("provider_throttled".into()),
        _ => return Err("provider_error".into()),
    }

    let mut stream = CompletionStream::default();
    let mut buffer = Vec::new();
    let mut total = 0_usize;
    let mut event = String::new();
    // Which stream index holds which tool call, so a partial JSON fragment lands
    // on the call it belongs to.
    while let Some(chunk) = response.chunk().await.map_err(|_| "provider_offline")? {
        total += chunk.len();
        if total > 2_000_000 {
            return Err("provider_response_too_large".into());
        }
        buffer.extend_from_slice(&chunk);
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "invalid_provider_utf8")?
                .trim_end_matches(['\r', '\n']);
            let Some(data) = line.strip_prefix("data:") else {
                // `event:` lines name the payload that follows; the payload's own
                // `type` field is what this reads.
                if let Some(name) = line.strip_prefix("event:") {
                    name.trim().clone_into(&mut event);
                }
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(data).map_err(|_| "invalid_provider_json")?;
            apply(&mut stream, &value, &event, on_delta)?;
        }
    }
    if stream.finish.is_none() {
        // A closed connection without `message_stop` is a truncated answer, not a
        // complete one, unless the content and the calls are both whole.
        return Err("incomplete_response".into());
    }
    stream.validate_finished()?;
    Ok(stream)
}

/// Fold one streamed event into the accumulated completion.
fn apply(
    stream: &mut CompletionStream,
    value: &Value,
    event: &str,
    on_delta: &mut impl FnMut(String),
) -> Result<(), String> {
    let kind = value["type"].as_str().unwrap_or(event);
    match kind {
        "content_block_start" => {
            let index = value["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok());
            let block = &value["content_block"];
            if block["type"].as_str() == Some("tool_use") {
                let index = index.unwrap_or(stream.calls.len());
                if index >= vn_core::chat::MAX_CALLS {
                    return Err("tool_limit".into());
                }
                stream.calls.insert(
                    index,
                    ToolCall {
                        id: block["id"].as_str().unwrap_or_default().to_owned(),
                        kind: "function".into(),
                        function: vn_core::chat::FunctionCall {
                            name: block["name"].as_str().unwrap_or_default().to_owned(),
                            arguments: String::new(),
                        },
                        extra_content: None,
                    },
                );
            }
        }
        "content_block_delta" => {
            let index = value["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok());
            let delta = &value["delta"];
            match delta["type"].as_str() {
                Some("text_delta") => {
                    if let Some(text) = delta["text"].as_str() {
                        stream.content.push_str(text);
                        if stream.content.len() > 64_000 {
                            return Err("provider_response_too_large".into());
                        }
                        on_delta(text.to_owned());
                    }
                }
                Some("input_json_delta") => {
                    let Some(index) = index else {
                        return Err("invalid_provider_response".into());
                    };
                    let entry = stream
                        .calls
                        .get_mut(&index)
                        .ok_or("invalid_provider_response")?;
                    if let Some(partial) = delta["partial_json"].as_str() {
                        entry.function.arguments.push_str(partial);
                        if entry.function.arguments.len() > 8192 {
                            return Err("tool_arguments_too_large".into());
                        }
                    }
                }
                _ => {
                    // `thinking_delta` and signature deltas are reasoning, never
                    // the answer.
                }
            }
        }
        "message_delta" => {
            if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                stream.finish = Some(
                    match reason {
                        // Only the reasons this app acts on are named; anything
                        // else is a finished turn.
                        "tool_use" => "tool_calls",
                        "max_tokens" => "length",
                        "refusal" => "content_filter",
                        _ => "stop",
                    }
                    .to_owned(),
                );
            }
            if let Some(tokens) = value["usage"]["output_tokens"].as_u64() {
                stream.tokens = Some(stream.tokens.unwrap_or(0).saturating_add(tokens));
            }
        }
        "message_stop" => stream.done = true,
        "error" => return Err("provider_error".into()),
        _ => {}
    }
    Ok(())
}

/// Tools in the shape `functionDeclarations`-style APIs want: the schema is
/// `input_schema`, and a tool with no arguments omits it.
fn tools(online: bool) -> Vec<Value> {
    tool_definitions(online)
        .into_iter()
        .map(|tool| {
            let function = &tool["function"];
            let mut declared = json!({
                "name": function["name"],
                "description": function["description"],
            });
            if let Some(parameters) = function.get("parameters") {
                declared["input_schema"] = parameters.clone();
            } else {
                declared["input_schema"] = json!({"type": "object", "properties": {}});
            }
            declared
        })
        .collect()
}

/// Translate the chat loop's OpenAI-shaped history into system + turns.
///
/// `role: "tool"` becomes a `tool_result` block in a user turn, which is how the
/// Messages API reports a function result. Consecutive tool results share one
/// user turn because the API rejects a turn with an empty content list and one
/// turn per result would also break the call/result pairing it expects.
pub fn convert(messages: &[Value]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut turns: Vec<Value> = Vec::new();
    for message in messages {
        let role = message["role"].as_str().unwrap_or("user");
        match role {
            "system" => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(message["content"].as_str().unwrap_or_default());
            }
            "assistant" => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message["content"].as_str()
                    && !text.trim().is_empty()
                {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let arguments = call["function"]["arguments"].as_str().unwrap_or("{}");
                    // A provider that streamed no arguments leaves an empty string,
                    // which is not valid input JSON.
                    let input: Value = serde_json::from_str(arguments)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call["id"],
                        "name": call["function"]["name"],
                        "input": input,
                    }));
                }
                if blocks.is_empty() {
                    continue;
                }
                turns.push(json!({"role": "assistant", "content": blocks}));
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": message["tool_call_id"],
                    "content": message["content"].as_str().unwrap_or("{}"),
                });
                // Append to the previous user turn when it is already a result
                // turn, so a parallel set travels as one turn.
                match turns.last_mut() {
                    Some(last)
                        if last["role"] == "user"
                            && last["content"].as_array().is_some_and(|blocks| {
                                blocks.iter().all(|block| block["type"] == "tool_result")
                            }) =>
                    {
                        if let Some(blocks) = last["content"].as_array_mut() {
                            blocks.push(block);
                        }
                    }
                    _ => turns.push(json!({"role": "user", "content": [block]})),
                }
            }
            _ => {
                let text = message["content"].as_str().unwrap_or_default().to_owned();
                // A user turn must not be empty, and a whitespace-only turn is
                // empty as far as the API is concerned.
                if text.trim().is_empty() {
                    continue;
                }
                turns.push(json!({"role": "user", "content": text}));
            }
        }
    }
    (system, turns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_round_trip_converts_to_tool_use_and_tool_result_blocks() {
        let history = vec![
            json!({"role": "system", "content": "be brief"}),
            json!({"role": "user", "content": "stats?"}),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "toolu_1", "type": "function", "function": {"name": "library_stats", "arguments": "{}"}},
            ]}),
            json!({"role": "tool", "tool_call_id": "toolu_1", "content": "{\"played\":12}"}),
        ];
        let (system, turns) = convert(&history);
        assert_eq!(system, "be brief");
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1]["role"], "assistant");
        assert_eq!(turns[1]["content"][0]["type"], "tool_use");
        assert_eq!(turns[1]["content"][0]["input"], json!({}));
        assert_eq!(turns[2]["role"], "user");
        assert_eq!(turns[2]["content"][0]["type"], "tool_result");
        assert_eq!(turns[2]["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn parallel_tool_results_share_one_user_turn() {
        let history = vec![
            json!({"role": "user", "content": "both"}),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "a", "type": "function", "function": {"name": "library_stats", "arguments": "{}"}},
                {"id": "b", "type": "function", "function": {"name": "taste_profile", "arguments": "{}"}},
            ]}),
            json!({"role": "tool", "tool_call_id": "a", "content": "1"}),
            json!({"role": "tool", "tool_call_id": "b", "content": "2"}),
        ];
        let (_, turns) = convert(&history);
        assert_eq!(turns.len(), 3);
        let results = turns[2]["content"].as_array().expect("blocks");
        assert_eq!(results.len(), 2);
        assert_eq!(results[1]["tool_use_id"], "b");
    }

    #[test]
    fn malformed_tool_arguments_become_an_empty_object_not_a_crash() {
        let history = vec![
            json!({"role": "user", "content": "x"}),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "a", "type": "function", "function": {"name": "library_stats", "arguments": ""}},
            ]}),
        ];
        let (_, turns) = convert(&history);
        assert_eq!(turns[1]["content"][0]["input"], json!({}));
    }

    #[test]
    fn an_argumentless_tool_declares_an_empty_input_schema() {
        // The Messages API requires `input_schema`; Gemini's quirk of refusing an
        // empty parameter object does not apply here.
        let declared = tools(false);
        let stats = declared
            .iter()
            .find(|tool| tool["name"] == "library_stats")
            .expect("stats tool");
        assert_eq!(stats["input_schema"]["properties"], json!({}));
    }

    /// The streamed events of one reply, folded in order. This is the whole of the
    /// wire handling: text, a tool call whose JSON arrives in fragments, and the
    /// stop reason that says the turn was a call rather than an answer.
    #[test]
    fn streamed_text_and_tool_calls_read_back_as_one_completion() {
        let events = [
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me check."}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_9","name":"search_library","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"status\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"playing\"}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":17}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let mut stream = CompletionStream::default();
        let mut deltas = Vec::new();
        for event in events {
            let value: Value = serde_json::from_str(event).expect("event json");
            apply(&mut stream, &value, "", &mut |text: String| {
                deltas.push(text);
            })
            .expect("apply");
        }
        stream.validate_finished().expect("finished");
        assert_eq!(stream.content, "Let me check.");
        assert_eq!(deltas, ["Let me check."]);
        assert_eq!(stream.calls[&1].id, "toolu_9");
        assert_eq!(stream.calls[&1].function.name, "search_library");
        assert_eq!(
            stream.calls[&1].function.arguments,
            r#"{"status":"playing"}"#
        );
        assert_eq!(stream.finish.as_deref(), Some("tool_calls"));
        assert_eq!(stream.tokens, Some(17));
        assert!(stream.done);
    }

    #[test]
    fn reasoning_deltas_never_reach_the_answer() {
        let events = [
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Done."}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let mut stream = CompletionStream::default();
        for event in events {
            let value: Value = serde_json::from_str(event).expect("event json");
            apply(&mut stream, &value, "", &mut |_| {}).expect("apply");
        }
        stream.validate_finished().expect("finished");
        assert_eq!(stream.content, "Done.");
        assert_eq!(stream.finish.as_deref(), Some("stop"));
    }

    #[test]
    fn an_overloaded_or_failed_stream_is_an_error_not_a_short_answer() {
        let mut stream = CompletionStream::default();
        let value: Value =
            serde_json::from_str(r#"{"type":"error","error":{"type":"overloaded_error"}}"#)
                .expect("event json");
        assert_eq!(
            apply(&mut stream, &value, "", &mut |_| {}),
            Err("provider_error".into())
        );
        // A cut-off stream never claims a stop reason, so nobody reads a partial
        // answer as a whole one.
        let mut truncated = CompletionStream::default();
        let value: Value = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half"}}"#,
        )
        .expect("event json");
        apply(&mut truncated, &value, "", &mut |_| {}).expect("apply");
        assert_eq!(truncated.finish, None);
        assert_eq!(
            truncated.validate_finished(),
            Err("incomplete_response".into())
        );
    }
}
