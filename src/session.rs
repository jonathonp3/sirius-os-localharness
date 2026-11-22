//! Append-only JSONL session log. One file per session; every message is recorded.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::llm::Message;

pub struct Session {
    pub id: String,
    file: PathBuf,
}

impl Session {
    pub async fn create(data_dir: &Path, model: &str) -> Result<Self> {
        let dir = data_dir.join("sessions");
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating {}", dir.display()))?;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before epoch")?
            .as_millis();
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("{millis:x}-{:x}-{sequence:x}", std::process::id());
        let file = dir.join(format!("{id}.jsonl"));
        let session = Self { id, file };

        session
            .append(serde_json::json!({
                "type": "session",
                "model": model,
                "created_at": millis.to_string(),
            }))
            .await?;
        Ok(session)
    }

    pub async fn open(data_dir: &Path, id: &str) -> Result<Self> {
        let id = id.trim_end_matches(".jsonl");
        if id.is_empty()
            || id.contains('/')
            || id.contains('\\')
            || id.contains("..")
        {
            bail!("invalid session id: {id}");
        }
        let file = data_dir.join("sessions").join(format!("{id}.jsonl"));
        if !file.exists() {
            bail!("session not found: {id}");
        }
        Ok(Self { id: id.to_string(), file })
    }

    pub async fn load_messages(&self) -> Result<Vec<Message>> {
        let text = tokio::fs::read_to_string(&self.file)
            .await
            .with_context(|| format!("reading {}", self.file.display()))?;
        let mut messages: Vec<Message> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line).with_context(|| {
                format!("parsing {} line {}", self.file.display(), index + 1)
            })?;
            if value.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            let Some(message) = value.get("message") else { continue };
            messages.push(serde_json::from_value(message.clone()).with_context(|| {
                format!("decoding message in {} line {}", self.file.display(), index + 1)
            })?);
        }
        // An interrupted run can leave a tool call without its result; a resumed
        // conversation must not send that to the model.
        while let Some(last) = messages.last() {
            if last.role == "assistant" && last.tool_calls.is_some() {
                messages.pop();
            } else {
                break;
            }
        }
        Ok(messages)
    }

    pub async fn append(&self, event: Value) -> Result<()> {
        let mut line = serde_json::to_string(&event).context("serializing session event")?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .await
            .with_context(|| format!("opening {}", self.file.display()))?;
        file.write_all(line.as_bytes()).await?;
        Ok(())
    }
}
