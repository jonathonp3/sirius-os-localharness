//! Named model profiles: endpoint, model, context limit, output budget, sampling.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub model: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub context_limit: Option<usize>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub think: Option<bool>,
    #[serde(default)]
    pub tool_policy: Option<String>,
    #[serde(default)]
    pub allow_tools: Option<Vec<String>>,
    #[serde(default)]
    pub deny_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, Profile>,
}

pub fn default_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join("local-harness/models.json"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/local-harness/models.json"))
}

/// Find the nearest `.harness/<file>` walking up from the workspace.
pub fn discover_named(start: &Path, file: &str) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(directory) = current {
        let candidate = directory.join(".harness").join(file);
        if candidate.exists() {
            return Some(candidate);
        }
        current = directory.parent();
    }
    None
}

pub fn discover(start: &Path) -> Option<PathBuf> {
    discover_named(start, "models.json")
}

pub fn load(path: &Path) -> Result<Option<Registry>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading registry {}", path.display()))?;
    let registry: Registry =
        serde_json::from_str(&text).with_context(|| format!("parsing registry {}", path.display()))?;
    Ok(Some(registry))
}
