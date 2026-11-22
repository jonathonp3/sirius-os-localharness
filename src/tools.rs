//! Built-in tools plus command-backed plugins, gated by a tool policy.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::mcp::McpServer;
use crate::plugins::Plugin;

const MAX_OUTPUT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    ReadOnly,
    Write,
    Full,
}

impl ToolPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read-only" | "read" => Some(Self::ReadOnly),
            "write" => Some(Self::Write),
            "full" | "shell" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Write => "write",
            Self::Full => "full",
        }
    }

    pub fn allows_write(self) -> bool {
        matches!(self, Self::Write | Self::Full)
    }

    pub fn allows_shell(self) -> bool {
        self == Self::Full
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolScopes {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl ToolScopes {
    pub fn permits(&self, name: &str) -> bool {
        if self.deny.iter().any(|pattern| matches_pattern(pattern, name)) {
            return false;
        }
        if self.allow.is_empty() {
            return true;
        }
        self.allow.iter().any(|pattern| matches_pattern(pattern, name))
    }
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

pub struct ToolSet {
    root: PathBuf,
    policy: ToolPolicy,
    plugins: Arc<Vec<Plugin>>,
    mcp: Arc<Vec<Arc<McpServer>>>,
    scopes: ToolScopes,
}

impl ToolSet {
    pub fn new(
        root: PathBuf,
        policy: ToolPolicy,
        plugins: Arc<Vec<Plugin>>,
        mcp: Arc<Vec<Arc<McpServer>>>,
        scopes: ToolScopes,
    ) -> Self {
        Self { root, policy, plugins, mcp, scopes }
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = vec!["read_file".to_string(), "list_dir".to_string()];
        if self.policy.allows_write() {
            names.push("write_file".to_string());
        }
        if self.policy.allows_shell() {
            names.push("bash".to_string());
            for plugin in self.plugins.iter() {
                names.push(plugin_tool_name(&plugin.name));
            }
            for server in self.mcp.iter() {
                for tool in &server.tools {
                    names.push(tool.call_name.clone());
                }
            }
        }
        names.retain(|name| self.scopes.permits(name));
        names
    }

    pub fn definitions(&self) -> Vec<Value> {
        let mut definitions = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a UTF-8 file from the workspace.",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string", "description": "Workspace-relative file path" } },
                        "required": ["path"]
                    }
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "list_dir",
                    "description": "List the entries of a workspace directory.",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string", "description": "Workspace-relative directory path" } },
                        "required": ["path"]
                    }
                }
            }),
        ];
        if self.policy.allows_write() {
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "write_file",
                    "description": "Write a UTF-8 file inside the workspace, creating parent directories.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Workspace-relative file path" },
                            "content": { "type": "string", "description": "Full file content" }
                        },
                        "required": ["path", "content"]
                    }
                }
            }));
        }
        if self.policy.allows_shell() {
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "bash",
                    "description": "Run a non-interactive bash command inside the workspace (60 s timeout).",
                    "parameters": {
                        "type": "object",
                        "properties": { "command": { "type": "string", "description": "Command to run with bash -lc" } },
                        "required": ["command"]
                    }
                }
            }));
            for plugin in self.plugins.iter() {
                definitions.push(json!({
                    "type": "function",
                    "function": {
                        "name": plugin_tool_name(&plugin.name),
                        "description": plugin.description,
                        "parameters": {
                            "type": "object",
                            "properties": plugin.parameters,
                            "required": plugin.required
                        }
                    }
                }));
            }
            for server in self.mcp.iter() {
                for tool in &server.tools {
                    definitions.push(json!({
                        "type": "function",
                        "function": {
                            "name": tool.call_name,
                            "description": tool.description,
                            "parameters": tool.input_schema
                        }
                    }));
                }
            }
        }
        definitions.retain(|definition| {
            definition
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(|name| self.scopes.permits(name))
                .unwrap_or(true)
        });
        definitions
    }

    pub async fn execute(&self, name: &str, arguments: &str) -> Result<String> {
        let arguments: Value =
            serde_json::from_str(arguments).context("tool arguments are not valid JSON")?;

        if !self.scopes.permits(name) {
            bail!("tool `{name}` is not permitted by the session scopes");
        }

        if name.starts_with("mcp__") {
            if !self.policy.allows_shell() {
                bail!("MCP tools require --tool-policy full (or --allow-shell)");
            }
            for server in self.mcp.iter() {
                if let Some(tool) = server.tools.iter().find(|tool| tool.call_name == name) {
                    return server.call_tool(&tool.name, arguments).await;
                }
            }
            bail!("unknown MCP tool `{name}`");
        }

        if let Some(plugin_name) = name.strip_prefix("plugin__") {
            if !self.policy.allows_shell() {
                bail!("plugin tools require --tool-policy full (or --allow-shell)");
            }
            let plugin = self
                .plugins
                .iter()
                .find(|plugin| plugin.name == plugin_name)
                .with_context(|| format!("unknown plugin `{plugin_name}`"))?;
            return self.execute_plugin(plugin, &arguments).await;
        }

        match name {
            "read_file" => {
                let path = self.existing(&string_arg(&arguments, "path")?)?;
                let text = tokio::fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("reading {}", path.display()))?;
                Ok(truncate(&text))
            }
            "write_file" => {
                if !self.policy.allows_write() {
                    bail!("write_file is disabled by the tool policy (use --allow-write or --tool-policy write)");
                }
                let path = self.writable(&string_arg(&arguments, "path")?)?;
                let content = string_arg(&arguments, "content")?;
                let parent = path.parent().context("path has no parent directory")?;
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("creating {}", parent.display()))?;
                let parent = parent
                    .canonicalize()
                    .with_context(|| format!("resolving {}", parent.display()))?;
                if !parent.starts_with(&self.root) {
                    bail!("path escapes the workspace: {}", parent.display());
                }
                let path = parent.join(path.file_name().context("path has no file name")?);
                tokio::fs::write(&path, &content)
                    .await
                    .with_context(|| format!("writing {}", path.display()))?;
                Ok(format!("wrote {} bytes to {}", content.len(), path.display()))
            }
            "list_dir" => {
                let path = self.existing(&string_arg(&arguments, "path")?)?;
                let mut entries = Vec::new();
                let mut dir = tokio::fs::read_dir(&path)
                    .await
                    .with_context(|| format!("listing {}", path.display()))?;
                while let Some(entry) = dir.next_entry().await? {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().await?.is_dir();
                    entries.push(if is_dir { format!("{name}/") } else { name });
                }
                entries.sort();
                Ok(truncate(&entries.join("\n")))
            }
            "bash" => {
                if !self.policy.allows_shell() {
                    bail!("bash is disabled by the tool policy (use --allow-shell or --tool-policy full)");
                }
                let command = string_arg(&arguments, "command")?;
                let output = tokio::time::timeout(
                    Duration::from_secs(60),
                    tokio::process::Command::new("bash")
                        .arg("-lc")
                        .arg(&command)
                        .current_dir(&self.root)
                        .kill_on_drop(true)
                        .output(),
                )
                .await
                .context("bash timed out after 60 s")??;
                Ok(render_output(&output))
            }
            other => bail!("unknown tool `{other}`"),
        }
    }

    async fn execute_plugin(&self, plugin: &Plugin, arguments: &Value) -> Result<String> {
        for required in &plugin.required {
            if arguments.get(required).is_none() {
                bail!("missing required plugin argument `{required}`");
            }
        }
        let mut command = plugin.command.clone();
        if let Some(object) = arguments.as_object() {
            for (key, value) in object {
                let text = value
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| value.to_string());
                command = command.replace(&format!("{{{key}}}"), &shell_quote(&text));
            }
        }
        let mut process = if plugin.sandbox {
            let mut sandbox = tokio::process::Command::new("bwrap");
            sandbox
                .arg("--ro-bind").arg("/usr").arg("/usr")
                .arg("--symlink").arg("usr/bin").arg("/bin")
                .arg("--symlink").arg("usr/lib").arg("/lib")
                .arg("--symlink").arg("usr/lib64").arg("/lib64")
                .arg("--ro-bind").arg("/etc").arg("/etc")
                .arg("--dev").arg("/dev")
                .arg("--proc").arg("/proc")
                .arg("--tmpfs").arg("/tmp")
                .arg("--unshare-net")
                .arg("--unshare-pid")
                .arg("--die-with-parent")
                .arg("--bind").arg(&self.root).arg(&self.root)
                .arg("bash").arg("-lc").arg(&command);
            sandbox
        } else {
            let mut direct = tokio::process::Command::new("bash");
            direct.arg("-lc").arg(&command);
            direct
        };
        let output = tokio::time::timeout(
            Duration::from_secs(60),
            process.current_dir(&self.root).kill_on_drop(true).output(),
        )
        .await
        .with_context(|| format!("plugin `{}` timed out after 60 s", plugin.name))??;
        Ok(render_output(&output))
    }

    fn existing(&self, path: &str) -> Result<PathBuf> {
        let candidate = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };
        let resolved = candidate
            .canonicalize()
            .with_context(|| format!("resolving {}", candidate.display()))?;
        if !resolved.starts_with(&self.root) {
            bail!("path escapes the workspace: {}", resolved.display());
        }
        Ok(resolved)
    }

    fn writable(&self, path: &str) -> Result<PathBuf> {
        let candidate = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };
        // Normalize lexically before creating anything, so `..` cannot escape.
        let mut normalized = PathBuf::new();
        for component in candidate.components() {
            match component {
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        if !normalized.starts_with(&self.root) {
            bail!("path escapes the workspace: {}", normalized.display());
        }
        let name = normalized.file_name().context("path has no file name")?;
        let parent = normalized.parent().context("path has no parent directory")?;
        Ok(parent.join(name))
    }
}

fn plugin_tool_name(name: &str) -> String {
    format!("plugin__{name}")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn render_output(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        text.push_str("\n[stderr]\n");
        text.push_str(&stderr);
    }
    if !output.status.success() {
        text.push_str(&format!("\n[exit {}]", output.status.code().unwrap_or(-1)));
    }
    truncate(&text)
}

fn string_arg(arguments: &Value, name: &str) -> Result<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .with_context(|| format!("missing string argument `{name}`"))
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_OUTPUT).collect();
    out.push_str("\n...[truncated]");
    out
}
