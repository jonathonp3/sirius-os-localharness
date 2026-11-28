//! Minimal streaming client for llama.cpp's OpenAI-compatible chat endpoint.

use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self { role: "system".into(), content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self { role: "user".into(), content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }

    pub fn assistant(content: String, tool_calls: Vec<ToolCall>) -> Self {
        let content = if content.is_empty() { None } else { Some(content) };
        let tool_calls = if tool_calls.is_empty() { None } else { Some(tool_calls) };
        Self { role: "assistant".into(), content, tool_calls, tool_call_id: None }
    }

    pub fn tool(tool_call_id: String, content: String) -> Self {
        Self { role: "tool".into(), content: Some(content), tool_calls: None, tool_call_id: Some(tool_call_id) }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Default)]
pub struct Turn {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Default, Clone)]
struct CallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

pub struct Client {
    http: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    remote: bool,
}

static SHARED_HTTP: OnceLock<reqwest::Client> = OnceLock::new();

fn shared_http() -> Result<reqwest::Client> {
    if let Some(client) = SHARED_HTTP.get() {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .build()
        .context("building HTTP client")?;
    let _ = SHARED_HTTP.set(client.clone());
    Ok(client)
}

impl Client {
    pub fn new(endpoint: &str, model: &str, api_key: Option<&str>, remote: bool) -> Result<Self> {
        let http = shared_http()?;
        Ok(Self {
            http,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.map(ToString::to_string),
            remote,
        })
    }

    fn chat_url(&self) -> String {
        if self.endpoint.ends_with("/v1") {
            format!("{}/chat/completions", self.endpoint)
        } else {
            format!("{}/v1/chat/completions", self.endpoint)
        }
    }

    pub async fn count_tokens(&self, messages: &[Message], tools: &[Value]) -> Result<usize> {
        if self.remote {
            bail!("exact token counting is unavailable for remote providers");
        }
        let root = self.endpoint.trim_end_matches('/').trim_end_matches("/v1");
        let url = format!("{root}/tokenize");
        let content = serde_json::to_string(&json!({ "messages": messages, "tools": tools }))
            .context("serializing prompt for tokenize")?;
        let response = self
            .http
            .post(&url)
            .json(&json!({ "content": content }))
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("tokenize returned {status}: {}", truncate(&text, 500));
        }
        let value: Value = response.json().await.context("parsing tokenize response")?;
        if let Some(tokens) = value.get("tokens").and_then(Value::as_array) {
            return Ok(tokens.len());
        }
        if let Some(count) = value.get("count").and_then(Value::as_u64) {
            return Ok(count as usize);
        }
        bail!("tokenize response had no token count");
    }

    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[Value],
        max_tokens: u32,
        temperature: f32,
        think: bool,
        cancel: Option<&Arc<AtomicBool>>,
        on_delta: &mut (dyn FnMut(bool, &str) + Send),
    ) -> Result<Turn> {
        if is_cancelled(cancel) {
            bail!("cancelled");
        }
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !self.remote {
            body["chat_template_kwargs"] = json!({ "enable_thinking": think });
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
            body["tool_choice"] = json!("auto");
        }

        let url = self.chat_url();
        let mut request = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            if !key.is_empty() {
                request = request.bearer_auth(key);
            }
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("model server returned {status}: {}", truncate(&text, 2000));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut turn = Turn::default();
        let mut calls: Vec<CallAccumulator> = Vec::new();

        while let Some(chunk) = stream.next().await {
            if is_cancelled(cancel) {
                bail!("cancelled");
            }
            let chunk = chunk.context("reading stream chunk")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline) = buffer.find('\n') {
                let line: String = buffer.drain(..=newline).collect();
                let line = line.trim();
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" {
                    continue;
                }
                let value: Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
                    if let Ok(parsed) = serde_json::from_value::<Usage>(usage.clone()) {
                        turn.usage = Some(parsed);
                    }
                }

                let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) else { continue };
                if let Some(delta) = choice.get("delta") {
                    if let Some(text) = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            on_delta(true, text);
                        }
                    }
                    if let Some(text) = delta.get("content").and_then(Value::as_str) {
                        if !text.is_empty() {
                            turn.content.push_str(text);
                            on_delta(false, text);
                        }
                    }
                    if let Some(items) = delta.get("tool_calls").and_then(Value::as_array) {
                        for item in items {
                            let index = item.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                            while calls.len() <= index {
                                calls.push(CallAccumulator::default());
                            }
                            let call = &mut calls[index];
                            if let Some(id) = item.get("id").and_then(Value::as_str) {
                                if !id.is_empty() {
                                    call.id = id.to_string();
                                }
                            }
                            if let Some(function) = item.get("function") {
                                if let Some(name) = function.get("name").and_then(Value::as_str) {
                                    if !name.is_empty() {
                                        call.name = name.to_string();
                                    }
                                }
                                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                                    call.arguments.push_str(arguments);
                                }
                            }
                        }
                    }
                }
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    if !reason.is_empty() {
                        turn.finish_reason = Some(reason.to_string());
                    }
                }
            }
        }

        turn.tool_calls = calls
            .into_iter()
            .enumerate()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| ToolCall {
                id: if call.id.is_empty() { format!("call_{index}") } else { call.id },
                kind: "function".to_string(),
                function: FunctionCall { name: call.name, arguments: call.arguments },
            })
            .collect();

        Ok(turn)
    }
}

fn is_cancelled(cancel: Option<&Arc<AtomicBool>>) -> bool {
    cancel
        .map(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or(false)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("...");
    out
}
