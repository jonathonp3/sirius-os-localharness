//! Hardware/model probing and prompt-budget enforcement.

use std::fs;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::llm::Message;

const SAFETY_TOKENS: usize = 4_096;

#[derive(Debug, Clone, Default)]
pub struct Hardware {
    pub cpus: usize,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub vram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
}

impl Hardware {
    pub fn probe() -> Self {
        let (ram_total_mb, ram_available_mb) = read_meminfo();
        let (vram_total_mb, vram_used_mb) = read_vram();
        Self {
            cpus: std::thread::available_parallelism().map(|value| value.get()).unwrap_or(0),
            ram_total_mb,
            ram_available_mb,
            vram_total_mb,
            vram_used_mb,
        }
    }
}

fn read_meminfo() -> (u64, u64) {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 0;
    let mut available = 0;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = kb(rest);
        }
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = kb(rest);
        }
    }
    (total, available)
}

fn kb(value: &str) -> u64 {
    value
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        / 1024
}

fn read_vram() -> (Option<u64>, Option<u64>) {
    let mut best: Option<(u64, u64)> = None;
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_card = name
                .strip_prefix("card")
                .map(|rest| !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit()))
                .unwrap_or(false);
            if !is_card {
                continue;
            }
            let device = entry.path().join("device");
            let total = fs::read_to_string(device.join("mem_info_vram_total"))
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok());
            if let Some(total) = total {
                let used = fs::read_to_string(device.join("mem_info_vram_used"))
                    .ok()
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                if best.map(|(best_total, _)| total > best_total).unwrap_or(true) {
                    best = Some((total, used));
                }
            }
        }
    }
    match best {
        Some((total, used)) => (Some(total / (1024 * 1024)), Some(used / (1024 * 1024))),
        None => (None, None),
    }
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model: String,
    pub n_ctx: usize,
    pub slots: usize,
}

impl Default for ModelInfo {
    fn default() -> Self {
        Self { model: String::new(), n_ctx: 32_768, slots: 1 }
    }
}

pub async fn probe_model(endpoint: &str, model: &str) -> Result<ModelInfo> {
    let url = format!("{}/props", endpoint.trim_end_matches('/'));
    let value: Value = reqwest::get(&url)
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("{url} returned an error"))?
        .json()
        .await
        .context("parsing /props")?;
    let n_ctx = value
        .get("default_generation_settings")
        .and_then(|settings| settings.get("n_ctx"))
        .and_then(Value::as_u64)
        .or_else(|| value.get("n_ctx").and_then(Value::as_u64))
        .map(|value| value as usize)
        .unwrap_or(32_768);
    let slots = value
        .get("total_slots")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(1);
    let alias = value
        .get("model_alias")
        .and_then(Value::as_str)
        .unwrap_or(model)
        .to_string();
    Ok(ModelInfo { model: alias, n_ctx, slots })
}

#[derive(Debug, Clone)]
pub struct Governor {
    pub model: String,
    pub model_ctx: usize,
    pub slots: usize,
    pub context_limit: usize,
    pub max_tokens: usize,
}

#[derive(Debug, Default)]
pub struct FitReport {
    pub dropped_turns: usize,
    pub truncated_tools: usize,
    pub estimate: usize,
    pub budget: usize,
}

impl FitReport {
    pub fn changed(&self) -> bool {
        self.dropped_turns > 0 || self.truncated_tools > 0
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.dropped_turns > 0 {
            parts.push(format!("dropped {} old turn(s)", self.dropped_turns));
        }
        if self.truncated_tools > 0 {
            parts.push(format!("truncated {} tool result(s)", self.truncated_tools));
        }
        parts.push(format!("~{} / {} prompt tokens", self.estimate, self.budget));
        parts.join(", ")
    }
}

impl Governor {
    pub fn new(info: ModelInfo, context_limit: usize, max_tokens: u32) -> Self {
        Self {
            model: info.model,
            model_ctx: info.n_ctx,
            slots: info.slots,
            context_limit: context_limit.max(512),
            max_tokens: max_tokens as usize,
        }
    }

    pub fn prompt_budget(&self) -> usize {
        let window = self.context_limit.min(self.model_ctx);
        window.saturating_sub(self.max_tokens + SAFETY_TOKENS).max(256)
    }

    pub fn estimate(&self, messages: &[Message], tools: &[Value]) -> usize {
        let message_chars: usize = messages
            .iter()
            .map(|message| {
                let content = message.content.as_deref().map(str::len).unwrap_or(0);
                let calls: usize = message
                    .tool_calls
                    .as_ref()
                    .map(|calls| {
                        calls
                            .iter()
                            .map(|call| call.function.name.len() + call.function.arguments.len())
                            .sum()
                    })
                    .unwrap_or(0);
                content + calls + 16
            })
            .sum();
        let tool_chars: usize = tools.iter().map(|tool| tool.to_string().len()).sum();
        (message_chars + tool_chars) / 2 + 32
    }

    /// Keep the prompt inside the budget: trim tool results, then drop the
    /// oldest complete turns. Never breaks a tool_call/tool_result pair.
    pub fn fit(&self, messages: &mut Vec<Message>, tools: &[Value]) -> FitReport {
        let budget = self.prompt_budget();
        let mut report = FitReport { budget, ..FitReport::default() };

        if self.estimate(messages, tools) > budget {
            for message in messages.iter_mut().filter(|message| message.role == "tool") {
                if let Some(content) = &message.content {
                    if content.chars().count() > 12_000 {
                        message.content = Some(truncate_chars(content, 12_000));
                        report.truncated_tools += 1;
                    }
                }
            }
        }

        loop {
            let estimate = self.estimate(messages, tools);
            if estimate <= budget {
                break;
            }
            let next_user = messages
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, message)| message.role == "user")
                .map(|(index, _)| index);
            let Some(index) = next_user else { break };
            if index <= 1 {
                break;
            }
            messages.drain(1..index);
            report.dropped_turns += 1;
        }

        if self.estimate(messages, tools) > budget {
            for message in messages.iter_mut().filter(|message| message.role == "tool") {
                if let Some(content) = &message.content {
                    if content.chars().count() > 800 {
                        message.content = Some(truncate_chars(content, 800));
                        report.truncated_tools += 1;
                    }
                }
            }
        }

        report.estimate = self.estimate(messages, tools);
        report
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("\n...[trimmed to fit the local context budget]");
    out
}
