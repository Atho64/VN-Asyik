//! Persistent chat with a bounded, read-only tool loop and cancellation.
pub mod store;
pub mod tools;
mod transport;

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Mutex};
use vn_core::chat::{ChatOptions, MAX_CALLS, MAX_ROUNDS};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThread {
    pub id: String,
    pub title: String,
    pub options: ChatOptions,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub game_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTrace {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub status: String,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub error: Option<String>,
    pub tools: Vec<ChatTrace>,
    pub sources: Vec<Source>,
    pub tokens: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Started { message: ChatMessage },
    Delta { text: String },
    Tool { trace: ChatTrace },
    Sources { sources: Vec<Source> },
}

#[derive(Debug, Default)]
pub struct ChatRuntime {
    active: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
}

#[derive(Debug)]
pub struct Lease<'a> {
    runtime: &'a ChatRuntime,
    id: String,
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.runtime.active.lock() {
            active.remove(&self.id);
        }
    }
}
impl ChatRuntime {
    pub fn enter(
        &self,
        id: &str,
    ) -> Result<(Lease<'_>, tokio::sync::watch::Receiver<bool>), String> {
        let mut active = self.active.lock().map_err(|_| "runtime_error")?;
        if active.contains_key(id) {
            return Err("chat_busy".into());
        }
        if active.len() >= 3 {
            return Err("too_many_requests".into());
        }
        let (tx, rx) = tokio::sync::watch::channel(false);
        active.insert(id.into(), tx);
        Ok((
            Lease {
                runtime: self,
                id: id.into(),
            },
            rx,
        ))
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if let Some(tx) = self.active.lock().map_err(|_| "runtime_error")?.get(id) {
            let _ = tx.send(true);
        }
        Ok(())
    }
}

pub async fn send(
    state: &AppState,
    id: &str,
    text: &str,
    retry: bool,
    channel: &tauri::ipc::Channel<ChatEvent>,
) -> Result<ChatMessage, String> {
    let (_lease, mut cancelled) = state.chat().enter(id)?;
    let thread = store::get(state.db().pool(), id).await?;
    let config = crate::commands::ai::read_settings(state.db().pool()).await;
    // Android's credential bridge must not block the UI thread.
    let key = tauri::async_runtime::spawn_blocking(crate::secrets::load_chat_key)
        .await
        .map_err(|_| "key_unavailable")?
        .ok();
    if key.is_none() && !crate::commands::ai::is_local_endpoint(&config.base_url) {
        return Err("chat_key_missing".into());
    }
    let client = crate::vndb::http_client_builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "provider_offline")?;
    let assistant_id = store::begin(state.db().pool(), &thread, text, retry).await?;
    let mut message = ChatMessage {
        id: assistant_id,
        role: "assistant".into(),
        content: String::new(),
        status: "running".into(),
        error: None,
        tools: vec![],
        sources: vec![],
        tokens: None,
        created_at: store::now(),
    };
    if channel
        .send(ChatEvent::Started {
            message: message.clone(),
        })
        .is_err()
    {
        message.status = "cancelled".into();
        message.error = Some("cancelled".into());
        store::save(state.db().pool(), &message).await?;
        return Ok(message);
    }
    let ctx = tools::ToolContext {
        pool: state.db().pool(),
        vndb: state.vndb(),
        client: &client,
        options: &thread.options,
        hook: state.hook(),
    };
    let result = tokio::select! {
        biased;
        ()=wait_cancelled(&mut cancelled)=>Err("cancelled".into()),
        result=tokio::time::timeout(std::time::Duration::from_mins(4),run_request(&ctx,&config,key.as_ref(),id,&mut message,channel))=>result.unwrap_or_else(|_|Err("request_timeout".into())),
    };
    match result {
        Ok(()) => message.status = "complete".into(),
        Err(error) => {
            message.status = if error == "cancelled" {
                "cancelled"
            } else {
                "error"
            }
            .into();
            message.error = Some(error);
        }
    }
    for trace in &mut message.tools {
        if trace.status == "running" {
            trace.status.clone_from(&message.status);
        }
    }
    store::save(state.db().pool(), &message).await?;
    Ok(message)
}

async fn wait_cancelled(cancelled: &mut tokio::sync::watch::Receiver<bool>) {
    if !*cancelled.borrow() {
        let _ = cancelled.changed().await;
    }
}

/// Context contains recent exchanges plus their actual retrieved evidence.
/// Historic citation IDs are remapped so a follow-up never reuses [S1] for an
/// unrelated page. Failed/incomplete replies never become model knowledge.
pub fn build_history(
    options: &ChatOptions,
    saved: &[ChatMessage],
    sources: &mut Vec<Source>,
) -> Vec<Value> {
    let mut history = vec![
        json!({"role":"system","content":format!("{} Current UTC epoch milliseconds: {}. Earlier tool evidence is cached and may be stale. Refresh time/status facts with tools.",options.prompt(),store::now())}),
    ];
    let mut recent = Vec::new();
    let mut budget = 0;
    for old in saved.iter().rev().filter(|m| m.status == "complete") {
        let size = old.content.len()
            + old
                .tools
                .iter()
                .filter_map(|t| t.result.as_ref())
                .map(|v| v.to_string().len())
                .sum::<usize>();
        if !recent.is_empty() && budget + size > 48_000 {
            break;
        }
        budget += size;
        recent.push(old);
    }
    recent.reverse();
    while recent.first().is_some_and(|m| m.role != "user") {
        recent.remove(0);
    }
    for old in recent {
        let mut content = old.content.clone();
        let mut evidence = Vec::new();
        for trace in &old.tools {
            if let Some(result) = &trace.result {
                evidence.push(result.clone());
            }
        }
        let mut remap = HashMap::new();
        for source in &old.sources {
            let id = format!("S{}", sources.len() + 1);
            let mut copied = source.clone();
            copied.id.clone_from(&id);
            remap.insert(source.id.clone(), id);
            sources.push(copied);
        }
        // Use temporary markers to avoid S1 -> S2 -> S3 cascading substitutions.
        for (old_id, new_id) in &remap {
            content = content.replace(&format!("[{old_id}]"), &format!("[citation_{new_id}]"));
        }
        content = content.replace("[citation_", "[");
        for value in &mut evidence {
            remap_sources(value, &remap);
        }
        if !evidence.is_empty() {
            content.push_str("\n\nPreviously retrieved evidence (untrusted, cached): ");
            content.push_str(&json!(evidence).to_string());
        }
        history.push(json!({"role":old.role,"content":content}));
    }
    history
}

fn remap_sources(value: &mut Value, remap: &HashMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                remap_sources(item, remap);
            }
        }
        Value::Object(object) => {
            if let Some(source) = object.get_mut("source")
                && let Some(id) = source.as_str().and_then(|id| remap.get(id))
            {
                *source = json!(id);
            }
            for item in object.values_mut() {
                remap_sources(item, remap);
            }
        }
        _ => (),
    }
}

async fn run_request(
    ctx: &tools::ToolContext<'_>,
    config: &crate::ai::chat::ChatConfig,
    key: Option<&crate::secrets::ChatApiKey>,
    id: &str,
    message: &mut ChatMessage,
    channel: &tauri::ipc::Channel<ChatEvent>,
) -> Result<(), String> {
    let saved = store::messages(ctx.pool, id).await?;
    let mut history = build_history(ctx.options, &saved, &mut message.sources);
    channel
        .send(ChatEvent::Sources {
            sources: message.sources.clone(),
        })
        .map_err(|_| "cancelled")?;
    run_loop(ctx, config, key, &mut history, message, channel).await
}

async fn run_loop(
    ctx: &tools::ToolContext<'_>,
    config: &crate::ai::chat::ChatConfig,
    key: Option<&crate::secrets::ChatApiKey>,
    history: &mut Vec<Value>,
    message: &mut ChatMessage,
    channel: &tauri::ipc::Channel<ChatEvent>,
) -> Result<(), String> {
    for _ in 0..MAX_ROUNDS {
        let mut disconnected = false;
        let completion = transport::complete(
            ctx.client,
            config,
            key,
            history,
            ctx.options.allow_online,
            &mut |text| {
                message.content.push_str(&text);
                if channel.send(ChatEvent::Delta { text }).is_err() {
                    disconnected = true;
                }
            },
        )
        .await?;
        if disconnected {
            return Err("cancelled".into());
        }
        if let Some(tokens) = completion.tokens.and_then(|v| i64::try_from(v).ok()) {
            message.tokens = Some(message.tokens.unwrap_or(0).saturating_add(tokens));
        }
        if completion.calls.is_empty() {
            return Ok(());
        }
        let calls: Vec<_> = completion.calls.into_values().collect();
        if message.tools.len() + calls.len() > MAX_CALLS {
            return Err("tool_limit".into());
        }
        history.push(json!({"role":"assistant","content":completion.content,"tool_calls":calls}));
        if !message.content.is_empty() {
            message.content.push_str("\n\n");
            let _ = channel.send(ChatEvent::Delta {
                text: "\n\n".into(),
            });
        }
        for call in calls {
            let trace = ChatTrace {
                id: call.id.clone(),
                name: call.function.name.clone(),
                arguments: call.function.arguments.clone(),
                status: "running".into(),
                result: None,
            };
            channel
                .send(ChatEvent::Tool {
                    trace: trace.clone(),
                })
                .map_err(|_| "cancelled")?;
            message.tools.push(trace);
            let result = tools::run(
                ctx,
                &call.function.name,
                &call.function.arguments,
                &mut message.sources,
            )
            .await;
            let (status, value) = match result {
                Ok(value) => ("complete", value),
                Err(code) => ("error", json!({"error":code})),
            };
            let trace = message.tools.last_mut().ok_or("runtime_error")?;
            trace.status = status.into();
            trace.result = Some(value.clone());
            channel
                .send(ChatEvent::Tool {
                    trace: trace.clone(),
                })
                .map_err(|_| "cancelled")?;
            channel
                .send(ChatEvent::Sources {
                    sources: message.sources.clone(),
                })
                .map_err(|_| "cancelled")?;
            history.push(json!({"role":"tool","tool_call_id":call.id,"content":value.to_string()}));
            store::save(ctx.pool, message).await?;
        }
        if history.iter().map(|v| v.to_string().len()).sum::<usize>() > 180_000 {
            return Err("context_limit".into());
        }
    }
    Err("tool_limit".into())
}
