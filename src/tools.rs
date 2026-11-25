//! Built-in tools plus command-backed plugins, gated by a tool policy.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::mcp::McpServer;
use crate::plugins::Plugin;

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
    dry_run: bool,
    web: bool,
}

impl ToolSet {
    pub fn new(
        root: PathBuf,
        policy: ToolPolicy,
        plugins: Arc<Vec<Plugin>>,
        mcp: Arc<Vec<Arc<McpServer>>>,
        scopes: ToolScopes,
        dry_run: bool,
        web: bool,
    ) -> Self {
        Self { root, policy, plugins, mcp, scopes, dry_run, web }
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = vec!["read_file".to_string(), "list_dir".to_string()];
        if self.web {
            names.push("web_search".to_string());
            names.push("web_fetch".to_string());
        }
        if self.policy.allows_write() {
            names.push("write_file".to_string());
        }
        if self.policy.allows_shell() {
            names.push("bash".to_string());
            names.push("host_bash".to_string());
            names.push("log_check".to_string());
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
        if self.web {
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "web_search",
                    "description": "Read-only DuckDuckGo web search. Returns titles, URLs, and snippets. No downloads.",
                    "parameters": {
                        "type": "object",
                        "properties": { "query": { "type": "string", "description": "Search terms" } },
                        "required": ["query"]
                    }
                }
            }));
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "web_fetch",
                    "description": "Read-only HTTPS page fetch. Returns extracted text and the final URL. No downloads and no scripts.",
                    "parameters": {
                        "type": "object",
                        "properties": { "url": { "type": "string", "description": "Public HTTPS URL" } },
                        "required": ["url"]
                    }
                }
            }));
        }
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
                    "description": "Run a non-interactive bash command inside the workspace (configurable timeout, 300 s default).",
                    "parameters": {
                        "type": "object",
                        "properties": { "command": { "type": "string", "description": "Command to run with bash -lc" } },
                        "required": ["command"]
                    }
                }
            }));
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "host_bash",
                    "description": "Run a non-interactive bash command on the host, outside the Flatpak sandbox. Use for systemctl, journalctl, podman, flatpak, rpm-ostree, and host hardware or network inspection. For llama-server and local-harness use systemctl --user and journalctl --user.",
                    "parameters": {
                        "type": "object",
                        "properties": { "command": { "type": "string", "description": "Command to run with host bash -lc" } },
                        "required": ["command"]
                    }
                }
            }));
            definitions.push(json!({
                "type": "function",
                "function": {
                    "name": "log_check",
                    "description": "Check a systemd user unit's current state, recent log lines, and warning/error entries. Use this for questions about logs or errors instead of reading the beginning of a huge journal.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "unit": { "type": "string", "description": "Systemd user unit, e.g. llama-server" },
                            "since": { "type": "string", "description": "journalctl --since value (default: 24 hours ago)" },
                            "lines": { "type": "integer", "description": "Recent lines to show (default 120, max 500)" }
                        },
                        "required": ["unit"]
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

        if self.dry_run {
            if name == "write_file" {
                if !self.policy.allows_write() {
                    bail!("write_file is disabled by the tool policy (use --allow-write or --tool-policy write)");
                }
                let path = string_arg(&arguments, "path")?;
                let content = string_arg(&arguments, "content")?;
                return Ok(format!("DRY-RUN — NOT EXECUTED. Would write {} bytes to {path}. Nothing was changed.", content.len()));
            }
            if name == "bash" || name == "host_bash" {
                if !self.policy.allows_shell() {
                    bail!("bash is disabled by the tool policy (use --allow-shell or --tool-policy full)");
                }
                let command = string_arg(&arguments, "command")?;
                return Ok(format!("DRY-RUN — NOT EXECUTED. Would run on {}: {command}. Nothing was run.", if name == "host_bash" { "host" } else { "sandbox" }));
            }
            if name.starts_with("plugin__") || name.starts_with("mcp__") {
                if !self.policy.allows_shell() {
                    bail!("plugin and MCP tools require --tool-policy full (or --allow-shell)");
                }
                return Ok(format!("DRY-RUN — NOT EXECUTED. Would call {name} with {arguments}. Nothing was run."));
            }
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
            "web_search" => {
                if !self.web {
                    bail!("web tools are disabled (use --allow-web)");
                }
                crate::web::search(&string_arg(&arguments, "query")?).await
            }
            "web_fetch" => {
                if !self.web {
                    bail!("web tools are disabled (use --allow-web)");
                }
                crate::web::fetch(&string_arg(&arguments, "url")?).await
            }
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
            "log_check" => {
                if !self.policy.allows_shell() {
                    bail!("log_check is disabled by the tool policy (use --tool-policy full)");
                }
                let unit = string_arg(&arguments, "unit")?;
                if !unit.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@:".contains(c)) {
                    bail!("invalid unit name");
                }
                let since = arguments
                    .get("since")
                    .and_then(Value::as_str)
                    .unwrap_or("24 hours ago")
                    .to_string();
                if !since.chars().all(|c| c.is_ascii_alphanumeric() || " :_-+".contains(c)) {
                    bail!("invalid since value");
                }
                let lines = arguments
                    .get("lines")
                    .and_then(Value::as_u64)
                    .unwrap_or(120)
                    .clamp(10, 500);
                let unit_arg = format!("'{}'", unit);
                let since_arg = format!("'{}'", since);
                let recent = run_bash(
                    &format!("journalctl --user -u {unit_arg} --since {since_arg} -n {lines} --no-pager"),
                    true,
                    &self.root,
                ).await?;
                let warnings = run_bash(
                    &format!("journalctl --user -u {unit_arg} --since {since_arg} -p warning -n 200 --no-pager"),
                    true,
                    &self.root,
                ).await?;
                let status = run_bash(
                    &format!("systemctl --user show {unit_arg} -p ActiveState -p NRestarts -p ActiveEnterTimestamp --no-pager"),
                    true,
                    &self.root,
                ).await?;
                Ok(truncate(&format!(
                    "SERVICE\n{}\n\nRECENT (last {} lines since {})\n{}\n\nWARNINGS/ERRORS (priority warning and above)\n{}",
                    String::from_utf8_lossy(&status.stdout).trim(),
                    lines,
                    since,
                    String::from_utf8_lossy(&recent.stdout).trim(),
                    String::from_utf8_lossy(&warnings.stdout).trim(),
                )))
            }
            "bash" | "host_bash" => {
                if !self.policy.allows_shell() {
                    bail!("bash is disabled by the tool policy (use --allow-shell or --tool-policy full)");
                }
                let host = name == "host_bash";
                let command = string_arg(&arguments, "command")?;
                if host && (command.to_ascii_lowercase().contains("llama-server")
                    || command.to_ascii_lowercase().contains("local-harness"))
                    && (command.contains("systemctl") || command.contains("journalctl"))
                    && !command.contains("--user")
                {
                    bail!("user units llama-server and local-harness require --user: use systemctl --user or journalctl --user");
                }
                if let Some(word) = first_command_word(&command) {
                    if !command_available(&word, host).await? {
                        bail!(
                            "command `{word}` is not available on the {}. Do not retry it blindly; use the system profile or {}.",
                            if host { "host" } else { "sandbox" },
                            if host { "a different host tool" } else { "host_bash for host commands" }
                        );
                    }
                }
                let timeout = bash_timeout();
                let output = tokio::time::timeout(timeout, run_bash(&command, host, &self.root))
                    .await
                    .with_context(|| format!("bash timed out after {} s", timeout.as_secs()))??;
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
        // Nested bwrap is denied inside the Flatpak sandbox; the Flatpak
        // itself is the isolation boundary there.
        let use_bwrap = plugin.sandbox && std::env::var_os("FLATPAK_ID").is_none();
        let mut process = if use_bwrap {
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

fn first_command_word(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?;
    if first.is_empty() || first.contains([';', '|', '&', '<', '>', '(', ')', '$', '`', '=', '{', '}']) {
        return None;
    }
    const KEYWORDS: &[&str] = &[
        "for", "while", "if", "then", "do", "case", "function", "fi", "done", "else", "elif",
        "cd", "export", "source", ".", ":", "true", "false", "echo", "printf", "test", "[", "[[",
    ];
    if KEYWORDS.contains(&first) {
        return None;
    }
    Some(first.to_string())
}

async fn command_available(word: &str, host: bool) -> Result<bool> {
    let mut command = if host && std::env::var_os("FLATPAK_ID").is_some() {
        let mut command = tokio::process::Command::new("flatpak-spawn");
        command.arg("--host").arg("bash").arg("-lc");
        command
    } else {
        let mut command = tokio::process::Command::new("bash");
        command.arg("-lc");
        command
    };
    command.arg("command -v -- \"$1\" >/dev/null 2>&1").arg("_").arg(word);
    let status = command.kill_on_drop(true).status().await?;
    Ok(status.success())
}

async fn run_bash(command_text: &str, host: bool, root: &Path) -> Result<std::process::Output> {
    let mut command = if host && std::env::var_os("FLATPAK_ID").is_some() {
        let mut command = tokio::process::Command::new("flatpak-spawn");
        command.arg("--host").arg("bash").arg("-lc").arg(command_text);
        command
    } else {
        let mut command = tokio::process::Command::new("bash");
        command.arg("-lc").arg(command_text).current_dir(root);
        command
    };
    command.kill_on_drop(true).output().await.context("running bash")
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

fn max_output() -> usize {
    std::env::var("HARNESS_MAX_OUTPUT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000)
}

fn bash_timeout() -> Duration {
    std::env::var("HARNESS_BASH_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(300))
}

fn truncate(text: &str) -> String {
    let limit = max_output();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("\n...[truncated]");
    out
}

/// Conservative destructive-command detection for the bash tool.
pub fn destructive_reason(name: &str, arguments: &str) -> Option<String> {
    if name != "bash" && name != "host_bash" {
        return None;
    }
    let value: Value = serde_json::from_str(arguments).ok()?;
    let command = value.get("command")?.as_str()?.to_string();
    let lower = command.to_ascii_lowercase();

    for segment in lower.split([';', '&', '|', '\n']) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        let Some(first) = words.first().copied() else { continue };
        let reason = match first {
            "rm" | "rmdir" | "shred" | "dd" | "wipefs" | "fdisk" | "parted" | "truncate"
            | "sudo" | "doas" | "pkexec" | "shutdown" | "reboot" | "poweroff"
            | "systemctl" | "service" | "dnf" | "apt" | "apt-get" | "rpm-ostree"
            | "snap" | "kill" | "pkill" | "killall" => Some(first.to_string()),
            "mkfs" => Some("mkfs".to_string()),
            _ if first.starts_with("mkfs.") => Some("mkfs".to_string()),
            "git" => match words.get(1).copied() {
                Some("clean") if words.iter().any(|word| word.starts_with('-') && word.contains('f')) => {
                    Some("git clean -f".to_string())
                }
                Some("reset") if words.contains(&"--hard") => Some("git reset --hard".to_string()),
                Some("push") if words.iter().any(|word| *word == "-f" || word.starts_with("--force")) => {
                    Some("git push --force".to_string())
                }
                Some("checkout") if words.contains(&"--") => Some("git checkout --".to_string()),
                Some("restore") => Some("git restore".to_string()),
                _ => None,
            },
            "flatpak" => match words.get(1).copied() {
                Some("uninstall") | Some("remove") => Some("flatpak uninstall".to_string()),
                _ => None,
            },
            "find" if words.contains(&"-delete") => Some("find -delete".to_string()),
            "xargs" if words.contains(&"rm") => Some("xargs rm".to_string()),
            "chmod" if words.iter().any(|word| *word == "-R" || *word == "--recursive") => {
                Some("recursive chmod".to_string())
            }
            "chown" if words.iter().any(|word| *word == "-R" || *word == "--recursive") => {
                Some("recursive chown".to_string())
            }
            _ => None,
        };
        if let Some(reason) = reason {
            return Some(format!("destructive pattern: {reason}"));
        }
    }

    if lower.contains("| sh") || lower.contains("| bash") {
        return Some("piping content into a shell".to_string());
    }
    if lower.contains("> /dev/sd") || lower.contains("> /etc/") {
        return Some("writing to a sensitive path".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::destructive_reason;

    #[test]
    fn flags_destructive_commands() {
        for command in ["rm -rf /tmp/x", "sudo dnf install x", "git push --force", "find . -delete"] {
            assert!(destructive_reason("bash", &format!("{{\"command\":\"{command}\"}}")).is_some(), "{command}");
        }
    }

    #[test]
    fn allows_read_only_commands() {
        for command in ["ls -l .local", "git status", "cat README.md", "rg TODO src"] {
            assert!(destructive_reason("bash", &format!("{{\"command\":\"{command}\"}}")).is_none(), "{command}");
        }
    }
}
