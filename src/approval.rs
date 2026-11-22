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
