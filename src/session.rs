//! Append-only JSONL session log. One file per session; every message is recorded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionIndex {
    #[serde(default)]
    pub sessions: HashMap<String, SessionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub pinned: bool,
    pub message_count: usize,
    pub workspace: String,
    pub workspace_name: String,
}

#[derive(Debug, Clone)]
pub struct SessionWorkspace {
    pub id: String,
    pub name: String,
    pub path: String,
}

impl SessionIndex {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("sessions").join("index.json")
    }

    pub async fn load(data_dir: &Path) -> Result<Self> {
        let path = Self::path(data_dir);
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub async fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        let dir = path.parent().context("session index path")?;
        tokio::fs::create_dir_all(dir).await?;
        let temp = dir.join("index.json.tmp");
        let text = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&temp, text).await?;
        tokio::fs::rename(&temp, &path).await?;
        Ok(())
    }
}

impl Session {
    pub async fn summary(&self, _data_dir: &Path, index: &SessionIndex) -> Result<SessionSummary> {
        let text = tokio::fs::read_to_string(&self.file).await?;
        let mut model = String::new();
        let mut created_at = 0u64;
        let mut message_count = 0usize;
        let mut first_user: Option<String> = None;
        let mut workspace = "home".to_string();
        let mut workspace_name = "Home".to_string();
        for line in text.lines() {
            let value: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            match value.get("type").and_then(Value::as_str) {
                Some("session") => {
                    if let Some(value) = value.get("model").and_then(Value::as_str) {
                        model = value.to_string();
                    }
                    if let Some(value) = value.get("created_at").and_then(Value::as_str) {
                        created_at = value.parse().unwrap_or(0);
                    }
                }
                Some("workspace") => {
                    if let Some(value) = value.get("id").and_then(Value::as_str) {
                        workspace = value.to_string();
                    }
                    if let Some(value) = value.get("name").and_then(Value::as_str) {
                        workspace_name = value.to_string();
                    }
                }
                Some("message") => {
                    message_count += 1;
                    if first_user.is_none() {
                        if let Some(message) = value.get("message") {
                            if message.get("role").and_then(Value::as_str) == Some("user") {
                                first_user = message
                                    .get("content")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let metadata = tokio::fs::metadata(&self.file).await?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        if created_at == 0 {
            created_at = metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0);
        }
        let meta = index.sessions.get(&self.id);
        let title = meta
            .and_then(|meta| meta.title.clone())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                let raw = first_user.unwrap_or_default();
                let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                if one_line.is_empty() {
                    "New chat".to_string()
                } else {
                    let mut chars = one_line.chars();
                    let short: String = chars.by_ref().take(80).collect();
                    if chars.next().is_some() {
                        format!("{short}…")
                    } else {
                        short
                    }
                }
            });
        Ok(SessionSummary {
            id: self.id.clone(),
            title,
            model,
            created_at,
            updated_at: modified,
            pinned: meta.map(|meta| meta.pinned).unwrap_or(false),
            message_count,
            workspace,
            workspace_name,
        })
    }

    pub async fn workspace(&self) -> Result<Option<SessionWorkspace>> {
        let text = tokio::fs::read_to_string(&self.file).await?;
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
            if value.get("type").and_then(Value::as_str) != Some("workspace") {
                continue;
            }
            let id = value.get("id").and_then(Value::as_str).unwrap_or("home");
            let name = value.get("name").and_then(Value::as_str).unwrap_or("Home");
            let path = value.get("path").and_then(Value::as_str).unwrap_or("");
            return Ok(Some(SessionWorkspace {
                id: id.to_string(),
                name: name.to_string(),
                path: path.to_string(),
            }));
        }
        Ok(None)
    }

    pub async fn trash(&self, data_dir: &Path) -> Result<()> {
        let trash = data_dir.join("sessions").join("trash");
        tokio::fs::create_dir_all(&trash).await?;
        let name = self
            .file
            .file_name()
            .context("session file name")?
            .to_owned();
        tokio::fs::rename(&self.file, trash.join(name)).await?;
        Ok(())
    }
}

pub async fn list_sessions(data_dir: &Path) -> Result<(Vec<SessionSummary>, SessionIndex)> {
    let dir = data_dir.join("sessions");
    tokio::fs::create_dir_all(&dir).await?;
    let index = SessionIndex::load(data_dir).await?;
    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut sessions = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(session) = Session::open(data_dir, id).await else {
            continue;
        };
        if let Ok(summary) = session.summary(data_dir, &index).await {
            sessions.push(summary);
        }
    }
    sessions.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    Ok((sessions, index))
}
