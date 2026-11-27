//! Built-in local and OpenAI-compatible provider profiles.
//! Secrets are read from the environment or a 0600 secrets file; never from provider.json.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
    pub context_limit: usize,
    pub max_tokens: u32,
    pub key_env: Option<&'static str>,
}

pub const LOCAL: Spec = Spec {
    id: "local",
    label: "Local llama.cpp",
    endpoint: "",
    model: "",
    context_limit: 32_768,
    max_tokens: 512,
    key_env: None,
};

pub const PROVIDERS: &[Spec] = &[
    LOCAL,
    Spec {
        id: "deepseek",
        label: "DeepSeek API",
        endpoint: "https://api.deepseek.com",
        model: "deepseek-chat",
        context_limit: 65_536,
        max_tokens: 4096,
        key_env: Some("DEEPSEEK_API_KEY"),
    },
    Spec {
        id: "kimi",
        label: "Kimi API",
        endpoint: "https://api.moonshot.ai/v1",
        model: "kimi-k2-0711-preview",
        context_limit: 131_072,
        max_tokens: 4096,
        key_env: Some("MOONSHOT_API_KEY"),
    },
];

pub fn spec(id: &str) -> Option<Spec> {
    PROVIDERS.iter().copied().find(|provider| provider.id == id)
}

fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join("local-harness"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/local-harness"))
}

pub fn selection_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("provider.json"))
}

pub fn secrets_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("secrets.env"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Selection {
    #[serde(default)]
    active: String,
}

pub fn active() -> Result<String> {
    let path = selection_path()?;
    if !path.exists() {
        return Ok("local".to_string());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let selection: Selection = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    if spec(&selection.active).is_some() {
        Ok(selection.active)
    } else {
        Ok("local".to_string())
    }
}

pub fn select(id: &str) -> Result<()> {
    let Some(provider) = spec(id) else {
        bail!("unknown provider `{id}`");
    };
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = selection_path()?;
    let selection = Selection { active: provider.id.to_string() };
    let text = serde_json::to_string_pretty(&selection)?;
    std::fs::write(&path, format!("{text}\n"))?;
    Ok(())
}

fn read_secret_file(key: &str) -> Result<Option<String>> {
    let path = secrets_path()?;
    if !path.exists() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)?.permissions().mode();
        if mode & 0o077 != 0 {
            bail!("{} must not be readable by group or others; use chmod 600", path.display());
        }
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return Ok(Some(value.to_string()));
                }
            }
        }
    }
    Ok(None)
}

pub fn api_key(provider: &Spec) -> Result<Option<String>> {
    let Some(key) = provider.key_env else {
        return Ok(None);
    };
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            return Ok(Some(value.trim().to_string()));
        }
    }
    read_secret_file(key)
}

pub fn key_present(provider: &Spec) -> bool {
    api_key(provider).ok().flatten().is_some()
}
