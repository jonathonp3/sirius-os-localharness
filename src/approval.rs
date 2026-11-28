//! Interactive approval for destructive commands.

use std::sync::Arc;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub name: String,
    pub arguments: String,
    pub reason: String,
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest) -> bool;
}

pub struct AutoApprover;

#[async_trait]
impl Approver for AutoApprover {
    async fn approve(&self, _request: &ApprovalRequest) -> bool {
        true
    }
}

pub struct DenyApprover;

#[async_trait]
impl Approver for DenyApprover {
    async fn approve(&self, _request: &ApprovalRequest) -> bool {
        false
    }
}

/// Approves read-only health-check commands for unattended automation.
pub struct AutomationApprover;

fn safe_automation_command(arguments: &str) -> bool {
    let command = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| value.get("command").and_then(|value| value.as_str()).map(str::to_string))
        .unwrap_or_default();
    let command = command.trim();
    if command.is_empty()
        || command.contains(';')
        || command.contains('`')
        || command.contains("$(")
        || command.contains("&&")
    {
        return false;
    }
    let base = command.split('|').next().unwrap_or(command).trim();
    let base = base.strip_suffix(" 2>&1").unwrap_or(base).trim();
    let base = base.strip_suffix(" 2>/dev/null").unwrap_or(base).trim();
    let base = base.strip_suffix(" || true").unwrap_or(base).trim();
    const SAFE: &[&str] = &[
        "systemctl --user is-active",
        "systemctl --user status",
        "systemctl --user show",
        "systemctl --user list-units",
        "systemctl --user list-timers",
        "podman ps",
        "distrobox list",
        "df ",
        "rpm-ostree status",
        "dnf makecache",
        "journalctl --user",
        "journalctl -u",
        "/var/home/jonathon/.local/bin/local-harness-health",
        "/home/jonathon/.local/bin/local-harness-health",
    ];
    SAFE.iter().any(|prefix| base == *prefix || base.starts_with(&format!("{prefix} ")))
}

#[async_trait]
impl Approver for AutomationApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        if request.name.ends_with("log_check") {
            return true;
        }
        if request.name == "host_bash" || request.name == "bash" {
            return safe_automation_command(&request.arguments);
        }
        false
    }
}

pub struct StdioApprover;

#[async_trait]
impl Approver for StdioApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        eprintln!(
            "\n[approval] {}\n  {} {}\nRun it? [y/N] ",
            request.reason, request.name, request.arguments
        );
        let answer = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line
        })
        .await
        .unwrap_or_default();
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

pub fn auto() -> Arc<dyn Approver> {
    Arc::new(AutoApprover)
}

pub fn deny() -> Arc<dyn Approver> {
    Arc::new(DenyApprover)
}

pub fn stdio() -> Arc<dyn Approver> {
    Arc::new(StdioApprover)
}

pub fn automation() -> Arc<dyn Approver> {
    Arc::new(AutomationApprover)
}
