//! Explicit, command-backed plugin tools loaded from a JSON file.

use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

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
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(skip)]
    pub package_dir: Option<PathBuf>,
}

fn default_sandbox() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct PluginFile {
    #[serde(default)]
    plugins: Vec<Plugin>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    packages: Vec<ManifestPackage>,
}

#[derive(Debug, Deserialize)]
struct ManifestPackage {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    revision: u64,
    path: String,
    sha256: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate(plugin: &Plugin, names: &mut std::collections::HashSet<String>) -> Result<()> {
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
    Ok(())
}

pub fn load(path: &Path) -> Result<Vec<Plugin>> {
    if path.is_dir() {
        return load_packages(path);
    }

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


fn load_packages(root: &Path) -> Result<Vec<Plugin>> {
    let manifest_path = root.join("manifest.json");
    let mut plugins = Vec::new();
    let mut names = std::collections::HashSet::new();
    if manifest_path.is_file() {
        let raw = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        for package in manifest.packages {
            let plugin_path = root.join(&package.path);
            let bytes = std::fs::read(&plugin_path)
                .with_context(|| format!("reading {}", plugin_path.display()))?;
            let actual = sha256_hex(&bytes);
            if actual != package.sha256 {
                bail!("plugin package `{}` failed SHA-256 verification", package.name);
            }
            let mut plugin: Plugin = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", plugin_path.display()))?;
            if plugin.version.is_empty() {
                plugin.version = package.version;
            }
            if plugin.revision == 0 {
                plugin.revision = package.revision;
            }
            plugin.package_dir = plugin_path.parent().map(Path::to_path_buf);
            validate(&plugin, &mut names)?;
            plugins.push(plugin);
        }
        return Ok(plugins);
    }
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("reading plugin directory {}", root.display()))?;
    for entry in entries.flatten() {
        let plugin_path = entry.path().join("plugin.json");
        if !plugin_path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&plugin_path)
            .with_context(|| format!("reading {}", plugin_path.display()))?;
        let mut plugin: Plugin = serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", plugin_path.display()))?;
        plugin.package_dir = plugin_path.parent().map(Path::to_path_buf);
        validate(&plugin, &mut names)?;
        plugins.push(plugin);
    }
    Ok(plugins)
}
