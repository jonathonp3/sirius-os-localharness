//! Named workspaces. A workspace groups chats and supplies the tool working directory.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    workspaces: Vec<Workspace>,
}

fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join("workspaces.json")
}

async fn load(data_dir: &Path) -> Result<Store> {
    let path = store_path(data_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

async fn save(data_dir: &Path, store: &Store) -> Result<()> {
    let path = store_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, serde_json::to_string_pretty(store)?).await?;
    tokio::fs::rename(temp, path).await?;
    Ok(())
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "workspace".to_string() } else { out }
}

pub fn home_workspace(home: &Path) -> Workspace {
    Workspace {
        id: "home".to_string(),
        name: "Home".to_string(),
        path: home.to_string_lossy().to_string(),
        builtin: true,
    }
}

pub async fn list(data_dir: &Path, home: &Path) -> Result<Vec<Workspace>> {
    let mut items = vec![home_workspace(home)];
    items.extend(load(data_dir).await?.workspaces);
    Ok(items)
}

pub async fn get(data_dir: &Path, home: &Path, id: &str) -> Result<Option<Workspace>> {
    Ok(list(data_dir, home).await?.into_iter().find(|workspace| workspace.id == id))
}

pub async fn create(data_dir: &Path, home: &Path, name: &str, path: &str) -> Result<Workspace> {
    let name = name.trim();
    if name.is_empty() {
        bail!("workspace name is empty");
    }
    let home_canonical = tokio::fs::canonicalize(home).await.unwrap_or_else(|_| home.to_path_buf());
    let requested = path.trim();
    let candidate = if requested.is_empty() {
        home.join(slug(name))
    } else if let Some(rest) = requested.strip_prefix("~/") {
        home.join(rest)
    } else {
        let candidate = PathBuf::from(requested);
        if candidate.is_absolute() { candidate } else { home.join(candidate) }
    };
    tokio::fs::create_dir_all(&candidate)
        .await
        .with_context(|| format!("creating workspace {}", candidate.display()))?;
    let canonical = tokio::fs::canonicalize(&candidate)
        .await
        .with_context(|| format!("resolving workspace {}", candidate.display()))?;
    if !canonical.starts_with(&home_canonical) {
        bail!("workspace must be inside the home directory");
    }
    if !tokio::fs::metadata(&canonical).await?.is_dir() {
        bail!("workspace path is not a directory");
    }
    let mut store = load(data_dir).await?;
    let base = slug(name);
    let mut id = base.clone();
    let mut suffix = 2;
    while id == "home" || store.workspaces.iter().any(|workspace| workspace.id == id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    let workspace = Workspace {
        id,
        name: name.chars().take(60).collect(),
        path: canonical.to_string_lossy().to_string(),
        builtin: false,
    };
    store.workspaces.push(workspace.clone());
    save(data_dir, &store).await?;
    Ok(workspace)
}


pub async fn delete(data_dir: &Path, id: &str) -> Result<bool> {
    if id == "home" {
        bail!("the Home workspace cannot be deleted");
    }
    let mut store = load(data_dir).await?;
    let before = store.workspaces.len();
    store.workspaces.retain(|workspace| workspace.id != id);
    if store.workspaces.len() == before {
        return Ok(false);
    }
    save(data_dir, &store).await?;
    Ok(true)
}
