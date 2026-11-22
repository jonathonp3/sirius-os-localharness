//! Explicit, command-backed plugin tools loaded from a JSON file.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default = "default_sandbox")]
    pub sandbox: bool,
}

fn default_sandbox() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct PluginFile {
    #[serde(default)]
    plugins: Vec<Plugin>,
}

pub fn load(path: &Path) -> Result<Vec<Plugin>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading plugins {}", path.display()))?;
    let file: PluginFile = serde_json::from_str(&text)
        .with_context(|| format!("parsing plugins {}", path.display()))?;
    let mut names = std::collections::HashSet::new();
    for plugin in &file.plugins {
        let valid = !plugin.name.is_empty()
            && plugin
                .name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-');
        if !valid {
            bail!("invalid plugin name `{}` (use letters, digits, _ or -)", plugin.name);
        }
        if !names.insert(plugin.name.clone()) {
            bail!("duplicate plugin name `{}`", plugin.name);
        }
        if plugin.command.trim().is_empty() {
            bail!("plugin `{}` has an empty command", plugin.name);
        }
    }
    Ok(file.plugins)
}
