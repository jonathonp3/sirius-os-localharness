//! MCP client with stdio and Streamable HTTP transports.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub call_name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpServer {
    pub name: String,
    pub tools: Vec<McpTool>,
    transport: Transport,
}

enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

struct StdioTransport {
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    #[allow(dead_code)]
    child: Mutex<tokio::process::Child>,
}

struct HttpTransport {
    http: reqwest::Client,
    url: String,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
}

pub async fn load(path: &Path) -> Result<Vec<Arc<McpServer>>> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading MCP config {}", path.display()))?;
    let config: McpConfig = serde_json::from_str(&text)
        .with_context(|| format!("parsing MCP config {}", path.display()))?;

    let mut servers = Vec::new();
    let mut call_names = HashSet::new();
    for server in config.servers {
        match (&server.command, &server.url) {
            (Some(_), Some(_)) => bail!(
                "MCP server `{}` sets both `command` and `url`; choose one",
                server.name
            ),
            (None, None) => bail!(
                "MCP server `{}` needs either `command` (stdio) or `url` (HTTP)",
                server.name
            ),
            _ => {}
        }
        let started = Arc::new(
            McpServer::start(server.clone())
                .await
                .with_context(|| format!("starting MCP server `{}`", server.name))?,
        );
        for tool in &started.tools {
            if !call_names.insert(tool.call_name.clone()) {
                bail!("duplicate MCP tool name `{}`", tool.call_name);
            }
        }
        servers.push(started);
    }
    Ok(servers)
}

impl McpServer {
    pub async fn start(config: McpServerConfig) -> Result<Self> {
        let transport = if let Some(url) = config.url.clone() {
            if url.starts_with("https://") {
                bail!("MCP HTTP server `{}` uses https, but this build has no TLS", config.name);
            }
            Transport::Http(HttpTransport {
                http: reqwest::Client::builder()
                    .build()
                    .context("building MCP HTTP client")?,
                url,
                session: Mutex::new(None),
                next_id: AtomicU64::new(1),
            })
        } else {
            Transport::Stdio(StdioTransport::start(&config).await?)
        };

        let mut server = Self { name: config.name, tools: Vec::new(), transport };
        server
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "local-harness", "version": env!("CARGO_PKG_VERSION") }
                }),
                Duration::from_secs(20),
            )
            .await
            .with_context(|| format!("initializing MCP server `{}`", server.name))?;
        server
            .notify("notifications/initialized", json!({}))
            .await?;

        let listed = server
            .request("tools/list", json!({}), Duration::from_secs(20))
            .await?;
        server.tools = parse_tools(&listed, &server.name);
        Ok(server)
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        match &self.transport {
            Transport::Stdio(transport) => transport.request(method, params, timeout).await,
            Transport::Http(transport) => transport.request(method, params, timeout).await,
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio(transport) => transport.notify(method, params).await,
            Transport::Http(transport) => transport.notify(method, params).await,
        }
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                Duration::from_secs(60),
            )
            .await?;

        let mut text = String::new();
        if let Some(items) = result.get("content").and_then(Value::as_array) {
            for item in items {
                if let Some(part) = item.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(part);
                }
            }
        }
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            bail!("MCP tool error: {text}");
        }
        if text.is_empty() {
            text = serde_json::to_string(&result).unwrap_or_default();
        }
        Ok(text)
    }
}

impl StdioTransport {
    async fn start(config: &McpServerConfig) -> Result<Self> {
        let command = config.command.as_deref().context("missing MCP command")?;
        let mut process = Command::new(command);
        process
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = process
            .spawn()
            .with_context(|| format!("spawning `{command}`"))?;
        let stdin = child.stdin.take().context("MCP stdin unavailable")?;
        let stdout = child.stdout.take().context("MCP stdout unavailable")?;

        let pending: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                if let Some(sender) = reader_pending.lock().await.remove(&id) {
                    let _ = sender.send(value);
                }
            }
        });

        Ok(Self {
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            child: Mutex::new(child),
        })
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        let response = tokio::time::timeout(timeout, receiver)
            .await
            .with_context(|| format!("MCP `{method}` timed out"))?
            .context("MCP server closed")?;
        if let Some(error) = response.get("error") {
            bail!("MCP `{method}` failed: {error}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn write_line(&self, message: &Value) -> Result<()> {
        let mut line = serde_json::to_string(message).context("encoding MCP request")?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}

impl HttpTransport {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let response = tokio::time::timeout(timeout, self.send(&body, Some(id)))
            .await
            .with_context(|| format!("MCP HTTP `{method}` timed out"))??;
        let response = response.context("MCP HTTP returned no response")?;
        if let Some(error) = response.get("error") {
            bail!("MCP `{method}` failed: {error}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send(&body, None).await?;
        Ok(())
    }

    async fn send(&self, body: &Value, expect_id: Option<u64>) -> Result<Option<Value>> {
        let mut request = self
            .http
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
            .json(body);
        if let Some(session) = self.session.lock().await.clone() {
            request = request.header("Mcp-Session-Id", session);
        }

        let response = request.send().await.context("POSTing to MCP HTTP server")?;
        let status = response.status();
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            *self.session.lock().await = Some(session.to_string());
        }
        if status.as_u16() == 202 {
            return Ok(None);
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("MCP HTTP {status}: {text}");
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = response.bytes().await.context("reading MCP HTTP response")?;

        if content_type.contains("text/event-stream") {
            let value = parse_sse(&bytes, expect_id)?;
            Ok(Some(value))
        } else {
            let value: Value = serde_json::from_slice(&bytes)
                .context("MCP HTTP response is not JSON")?;
            Ok(Some(value))
        }
    }
}

fn parse_tools(listed: &Value, server_name: &str) -> Vec<McpTool> {
    listed
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.to_string();
                    let description = tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("MCP tool")
                        .to_string();
                    let input_schema = tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                    Some(McpTool {
                        call_name: format!(
                            "mcp__{}__{}",
                            sanitize(server_name),
                            sanitize(&name)
                        ),
                        name,
                        description,
                        input_schema,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_sse(bytes: &[u8], expect_id: Option<u64>) -> Result<Value> {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        match expect_id {
            Some(id) if value.get("id").and_then(Value::as_u64) != Some(id) => continue,
            _ => return Ok(value),
        }
    }
    bail!("MCP SSE response did not contain the expected result")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
