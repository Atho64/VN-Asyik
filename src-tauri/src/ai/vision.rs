//! Transient image parts and a bounded, tool-free visual completion.
//! No redirects, retries, response logging or automatic capability probes.
use super::chat::{ANTHROPIC_VERSION, ChatConfig, Provider};
use crate::secrets::ChatApiKey;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Image {
    bytes: Vec<u8>,
}
impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Image(<transient>)")
    }
}
impl Image {
    pub fn jpeg(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > 1_048_576 || !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return Err("vision_invalid_image".into());
        }
        Ok(Self { bytes })
    }
    pub fn data_url(&self) -> String {
        format!("data:image/jpeg;base64,{}", STANDARD.encode(&self.bytes))
    }
}
#[derive(Debug)]
pub enum Part<'a> {
    Text(&'a str),
    Image(&'a Image),
}
pub fn content(parts: &[Part<'_>]) -> Value {
    Value::Array(
        parts
            .iter()
            .map(|p| match p {
                Part::Text(text) => json!({"type":"text","text":text}),
                Part::Image(image) => {
                    json!({"type":"image_url","image_url":{"url":image.data_url()}})
                }
            })
            .collect(),
    )
}
/// Strictly converts only bounded inline images emitted by the transient layer.
pub fn anthropic_content(value: &Value) -> Result<Value, String> {
    if value.is_string() {
        return Ok(value.clone());
    }
    let blocks = value.as_array().ok_or("vision_invalid_image")?;
    if blocks.len() > 8 {
        return Err("vision_invalid_image".into());
    }
    blocks.iter().map(|b|match b["type"].as_str(){
        Some("text") if b["text"].is_string()=>Ok(json!({"type":"text","text":b["text"]})),
        Some("image_url")=>{
            let url=b["image_url"]["url"].as_str().ok_or("vision_invalid_image")?;
            let (media,data)=if let Some(d)=url.strip_prefix("data:image/jpeg;base64,"){("image/jpeg",d)}
                else if let Some(d)=url.strip_prefix("data:image/png;base64,"){("image/png",d)}else{return Err("vision_invalid_image".into());};
            if data.len()>1_398_104{return Err("vision_invalid_image".into());}
            let bytes=STANDARD.decode(data).map_err(|_|"vision_invalid_image")?;
            if bytes.len()>1_048_576 || !(if media=="image/png"{bytes.starts_with(b"\x89PNG\r\n\x1a\n")}else{bytes.starts_with(&[0xff,0xd8,0xff])}) {return Err("vision_invalid_image".into());}
            Ok(json!({"type":"image","source":{"type":"base64","media_type":media,"data":data}}))
        },_=>Err("vision_invalid_image".into())
    }).collect::<Result<Vec<_>,_>>().map(Value::Array)
}

pub async fn complete(
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    messages: &[Value],
    tokens: u32,
) -> Result<String, String> {
    let client = crate::vndb::http_client_builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_mins(1))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "provider_offline")?;
    let (path, body) = match config.provider {
        Provider::OpenAi => (
            "chat/completions",
            json!({"model":config.model,"messages":messages,"max_tokens":tokens,"stream":false}),
        ),
        Provider::Anthropic => {
            let (system, turns) = super::conversation::anthropic::convert(messages)?;
            (
                "messages",
                json!({"model":config.model,"system":system,"messages":turns,"max_tokens":tokens,"stream":false}),
            )
        }
    };
    let mut req = client
        .post(format!("{}/{path}", config.base_url.trim_end_matches('/')))
        .json(&body);
    if config.provider == Provider::Anthropic {
        req = req.header("anthropic-version", ANTHROPIC_VERSION);
    }
    if let Some(key) = key {
        req = if config.provider == Provider::Anthropic {
            req.header("x-api-key", key.expose())
        } else {
            req.bearer_auth(key.expose())
        };
    }
    let mut res = req.send().await.map_err(|_| "provider_offline")?;
    match res.status().as_u16() {
        200..=299 => (),
        401 | 403 => return Err("provider_unauthorized".into()),
        400 | 404 | 422 => return Err("provider_incompatible".into()),
        429 | 529 => return Err("provider_throttled".into()),
        _ => return Err("provider_error".into()),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| "provider_offline")? {
        if bytes.len() + chunk.len() > 65536 {
            return Err("provider_response_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let res: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_provider_json")?;
    let text = match config.provider {
        Provider::OpenAi => {
            if res["choices"][0]["finish_reason"] != "stop"
                || res["choices"][0]["message"]
                    .get("tool_calls")
                    .is_some_and(|c| !c.is_null() && c.as_array().is_none_or(|a| !a.is_empty()))
            {
                return Err("incomplete_response".into());
            }
            res["choices"][0]["message"]["content"]
                .as_str()
                .ok_or("invalid_provider_response")?
                .to_owned()
        }
        Provider::Anthropic => {
            if res["stop_reason"] != "end_turn" {
                return Err("incomplete_response".into());
            }
            let blocks = res["content"]
                .as_array()
                .ok_or("invalid_provider_response")?;
            if blocks.iter().any(|b| b["type"] == "tool_use") {
                return Err("invalid_provider_response".into());
            }
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<String>()
        }
    };
    if text.trim().is_empty() || text.len() > 16000 {
        return Err("invalid_provider_response".into());
    }
    Ok(text)
}
