//! Explicit, command-backed plugin tools loaded from a JSON file.

use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use ed25519_dalek::Verifier;

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
    #[serde(default = "default_api")]
    pub api: String,
    #[serde(default)]
    pub min_harness: String,
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(skip)]
    pub package_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Permissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default = "default_write")]
    pub write: String,
    #[serde(default)]
    pub host: bool,
    #[serde(default)]
    pub shell: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            network: false,
            write: default_write(),
            host: false,
            shell: false,
        }
    }
}

fn default_write() -> String {
    "workspace".to_string()
}

fn default_sandbox() -> bool {
    true
}

fn default_api() -> String {
    "1".to_string()
}

const PLUGIN_API: &str = "1";

const BUNDLED_LINUX_OPS_JSON: &[u8] = include_bytes!("../plugins/linux-ops/plugin.json");
const BUNDLED_LINUX_OPS_PY: &[u8] = include_bytes!("../plugins/linux-ops/bin/linux_ops.py");
const BUNDLED_SYSTEM_PROFILE_PY: &[u8] = include_bytes!("../plugins/linux-ops/bin/system_profile.py");
const BUNDLED_JOURNAL_SKILL: &[u8] = include_bytes!("../plugins/linux-ops/skills/journal-check/SKILL.md");

fn version_tuple(value: &str) -> (u64, u64, u64) {
    let mut parts = value.trim_start_matches('v').split('.');
    let major = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

#[derive(Debug, Deserialize)]
struct PluginTool {
    name: String,
    description: String,
    command: String,
    #[serde(default)]
    parameters: serde_json::Value,
    #[serde(default)]
    required: Vec<String>,
    #[serde(default = "default_sandbox")]
    sandbox: bool,
    #[serde(default)]
    permissions: Permissions,
}

#[derive(Debug, Deserialize)]
struct PackageFile {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    revision: u64,
    #[serde(default = "default_api")]
    api: String,
    #[serde(default)]
    min_harness: String,
    #[serde(default)]
    tools: Vec<PluginTool>,
    #[serde(default)]
    permissions: Permissions,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    mcp: Option<McpBundle>,
}

#[derive(Debug, Deserialize)]
struct McpBundle {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: std::collections::HashMap<String, String>,
    #[serde(default)]
    url: Option<String>,
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

fn verify_signature(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path.file_name().and_then(|name| name.to_str()).context("signature path")?;
    let signature_path = path.with_file_name(format!("{file_name}.sig"));
    if !signature_path.is_file() {
        return Ok(());
    }
    let key_hex = std::env::var("HARNESS_PLUGIN_PUBKEY")
        .context("signed plugin requires HARNESS_PLUGIN_PUBKEY")?;
    let key_bytes = hex::decode(key_hex.trim()).context("decoding HARNESS_PLUGIN_PUBKEY")?;
    let key_bytes: [u8; 32] = key_bytes.try_into().map_err(|_| anyhow::anyhow!("plugin public key must be 32 bytes"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).context("invalid plugin public key")?;
    let signature_hex = std::fs::read_to_string(&signature_path)
        .with_context(|| format!("reading {}", signature_path.display()))?;
    let signature = hex::decode(signature_hex.trim()).context("decoding plugin signature")?;
    let signature = ed25519_dalek::Signature::from_slice(&signature).context("invalid plugin signature length")?;
    key.verify(bytes, &signature).map_err(|_| anyhow::anyhow!("plugin signature verification failed"))?;
    Ok(())
}

fn read_signed(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    verify_signature(path, &bytes)?;
    Ok(bytes)
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
    if plugin.api != PLUGIN_API {
        bail!("plugin `{}` requires plugin API {}, this harness supports {}", plugin.name, plugin.api, PLUGIN_API);
    }
    if !plugin.min_harness.is_empty()
        && version_tuple(&plugin.min_harness) > version_tuple(env!("CARGO_PKG_VERSION"))
    {
        bail!(
            "plugin `{}` requires harness >= {}, this is {}",
            plugin.name,
            plugin.min_harness,
            env!("CARGO_PKG_VERSION")
        );
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Vec<Plugin>> {
    if path.is_dir() {
        let direct = path.join("plugin.json");
        if direct.is_file() {
            let bytes = read_signed(&direct)?;
            let text = String::from_utf8(bytes).context("plugin.json is not UTF-8")?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", direct.display()))?;
            if value.get("tools").is_some() || value.get("mcp").is_some() {
                return load_package_file(&direct, path);
            }
            let mut plugin: Plugin = serde_json::from_value(value)
                .with_context(|| format!("parsing {}", direct.display()))?;
            plugin.package = plugin.name.clone();
            plugin.package_dir = Some(path.to_path_buf());
            let mut names = std::collections::HashSet::new();
            validate(&plugin, &mut names)?;
            return Ok(vec![plugin]);
        }
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


fn load_package_file(path: &Path, package_dir: &Path) -> Result<Vec<Plugin>> {
    let package: PackageFile = serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let mut plugins = Vec::new();
    let mut names = std::collections::HashSet::new();
    for tool in package.tools {
        let plugin = Plugin {
            name: tool.name,
            description: tool.description,
            command: tool.command,
            parameters: tool.parameters,
            required: tool.required,
            sandbox: tool.sandbox,
            version: package.version.clone(),
            revision: package.revision,
            api: package.api.clone(),
            min_harness: package.min_harness.clone(),
            package: package.name.clone(),
            permissions: tool.permissions.clone(),
            skills: package.skills.clone(),
            package_dir: Some(package_dir.to_path_buf()),
        };
        validate(&plugin, &mut names)?;
        plugins.push(plugin);
    }
    Ok(plugins)
}

fn load_packages(root: &Path) -> Result<Vec<Plugin>> {
    let manifest_path = root.join("manifest.json");
    let mut plugins = Vec::new();
    let mut names = std::collections::HashSet::new();
    if manifest_path.is_file() {
        let raw = String::from_utf8(read_signed(&manifest_path)?).context("manifest is not UTF-8")?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        for package in manifest.packages {
            let plugin_path = root.join(&package.path);
            let bytes = read_signed(&plugin_path)?;
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
            if plugin.package.is_empty() {
                plugin.package = plugin.name.clone();
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
        let text = String::from_utf8(read_signed(&plugin_path)?).context("plugin.json is not UTF-8")?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", plugin_path.display()))?;
        if value.get("tools").is_some() || value.get("mcp").is_some() {
            let package_dir = plugin_path.parent().context("plugin directory")?;
            plugins.extend(load_package_file(&plugin_path, package_dir)?);
            continue;
        }
        let mut plugin: Plugin = serde_json::from_value(value)
            .with_context(|| format!("parsing {}", plugin_path.display()))?;
        if plugin.package.is_empty() {
            plugin.package = plugin.name.clone();
        }
        plugin.package_dir = plugin_path.parent().map(Path::to_path_buf);
        validate(&plugin, &mut names)?;
        plugins.push(plugin);
    }
    Ok(plugins)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default = "default_api")]
    pub api: String,
    #[serde(default)]
    pub min_harness: String,
    #[serde(default)]
    pub enabled: bool,
    pub path: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub previous: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    plugins: Vec<Installed>,
}

pub fn registry_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/local-harness/plugins")
}

pub fn bootstrap_bundled() -> Result<()> {
    let destination = registry_dir().join("linux-ops");
    let mut registry = load_registry()?;
    if registry.plugins.iter().any(|entry| entry.name == "linux-ops") {
        return Ok(());
    }
    if !destination.join("plugin.json").is_file() {
        std::fs::create_dir_all(destination.join("bin"))?;
        std::fs::write(destination.join("plugin.json"), BUNDLED_LINUX_OPS_JSON)?;
        std::fs::write(destination.join("bin/linux_ops.py"), BUNDLED_LINUX_OPS_PY)?;
        std::fs::write(destination.join("bin/system_profile.py"), BUNDLED_SYSTEM_PROFILE_PY)?;
        std::fs::create_dir_all(destination.join("skills/journal-check"))?;
        std::fs::write(destination.join("skills/journal-check/SKILL.md"), BUNDLED_JOURNAL_SKILL)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["bin/linux_ops.py", "bin/system_profile.py"] {
                let path = destination.join(name);
                if let Ok(metadata) = std::fs::metadata(&path) {
                    let mut permissions = metadata.permissions();
                    permissions.set_mode(0o755);
                    let _ = std::fs::set_permissions(path, permissions);
                }
            }
        }
    }
    let loaded = load(&destination)?;
    install_skills(&destination, &loaded)?;
    let Some(first) = loaded.first() else {
        bail!("bundled linux-ops package is empty");
    };
    registry.plugins.push(Installed {
        name: "linux-ops".to_string(),
        version: first.version.clone(),
        revision: first.revision,
        api: first.api.clone(),
        min_harness: first.min_harness.clone(),
        enabled: true,
        path: "linux-ops".to_string(),
        tools: loaded.iter().map(|plugin| plugin.name.clone()).collect(),
        previous: None,
    });
    save_registry(&registry)?;
    Ok(())
}

fn registry_path() -> PathBuf {
    registry_dir().join("registry.json")
}

fn load_registry() -> Result<Registry> {
    let path = registry_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn save_registry(registry: &Registry) -> Result<()> {
    let path = registry_path();
    std::fs::create_dir_all(path.parent().context("registry parent")?)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_string_pretty(registry)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub fn list_installed() -> Result<Vec<Installed>> {
    Ok(load_registry()?.plugins)
}

pub fn load_installed() -> Result<Vec<Plugin>> {
    let registry = load_registry()?;
    let mut plugins = Vec::new();
    for entry in registry.plugins.into_iter().filter(|entry| entry.enabled) {
        let path = registry_dir().join(&entry.path);
        for plugin in load(&path)? {
            plugins.push(plugin);
        }
    }
    Ok(plugins)
}

fn install_archive(source: &Path) -> Result<Installed> {
    let temp = std::env::temp_dir().join(format!("local-harness-plugin-{}", std::process::id()));
    if temp.exists() {
        std::fs::remove_dir_all(&temp)?;
    }
    std::fs::create_dir_all(&temp)?;
    let file = std::fs::File::open(source)
        .with_context(|| format!("opening {}", source.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.is_absolute() || path.components().any(|component| matches!(component, std::path::Component::ParentDir | std::path::Component::RootDir)) {
            bail!("archive entry escapes the install directory: {}", path.display());
        }
        entry.unpack_in(&temp)?;
    }
    let root = if temp.join("plugin.json").is_file() {
        temp.clone()
    } else {
        let candidates: Vec<PathBuf> = std::fs::read_dir(&temp)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("plugin.json").is_file())
            .collect();
        if candidates.len() != 1 {
            bail!("archive must contain one plugin package root");
        }
        candidates[0].clone()
    };
    let result = install_from_dir(&root);
    let _ = std::fs::remove_dir_all(&temp);
    result
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

async fn download_plugin(url: &str) -> Result<Vec<u8>> {
    if !url.starts_with("https://") {
        bail!("plugin URL must use https");
    }
    let response = reqwest::get(url).await.context("downloading plugin")?;
    if !response.status().is_success() {
        bail!("downloading plugin: HTTP {}", response.status());
    }
    if response.content_length().unwrap_or(0) > 50 * 1024 * 1024 {
        bail!("plugin archive is too large");
    }
    let bytes = response.bytes().await.context("reading plugin archive")?;
    if bytes.len() > 50 * 1024 * 1024 {
        bail!("plugin archive is too large");
    }
    Ok(bytes.to_vec())
}

fn install_archive_bytes(bytes: &[u8]) -> Result<Installed> {
    let temp = std::env::temp_dir().join(format!("local-harness-download-{}.tar.gz", std::process::id()));
    std::fs::write(&temp, bytes).with_context(|| format!("writing {}", temp.display()))?;
    let result = install_archive(&temp);
    let _ = std::fs::remove_file(&temp);
    result
}

pub async fn install_url(url: &str) -> Result<Installed> {
    install_archive_bytes(&download_plugin(url).await?)
}

pub const CATALOG_URL: &str = "https://raw.githubusercontent.com/jonathonp3/sirius-os-localharness/main/plugins.json";

pub async fn catalog(url: Option<&str>) -> Result<serde_json::Value> {
    let url = url.unwrap_or(CATALOG_URL);
    let response = reqwest::get(url).await.context("downloading plugin catalog")?;
    if !response.status().is_success() {
        bail!("catalog: HTTP {}", response.status());
    }
    let value: serde_json::Value = response.json().await.context("parsing plugin catalog")?;
    Ok(value)
}

pub async fn install_from_catalog(name: &str, version: Option<&str>, url: Option<&str>) -> Result<Installed> {
    let catalog = catalog(url).await?;
    let entry = catalog
        .get("plugins")
        .and_then(|plugins| plugins.get(name))
        .with_context(|| format!("plugin `{name}` is not in the catalog"))?;
    let version = version
        .map(ToString::to_string)
        .or_else(|| entry.get("latest").and_then(serde_json::Value::as_str).map(ToString::to_string))
        .context("catalog entry has no latest version")?;
    let package = entry
        .get("versions")
        .and_then(|versions| versions.get(&version))
        .with_context(|| format!("plugin `{name}` has no version `{version}`"))?;
    let archive = package
        .get("url")
        .and_then(serde_json::Value::as_str)
        .context("catalog entry has no archive URL")?;
    let bytes = download_plugin(archive).await?;
    if let Some(expected) = package.get("sha256").and_then(serde_json::Value::as_str) {
        let actual = sha256_hex(&bytes);
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!("catalog SHA-256 mismatch for `{name}`");
        }
    }
    install_archive_bytes(&bytes)
}

pub fn install(source: &Path) -> Result<Installed> {
    if source.is_file() {
        return install_archive(source);
    }
    install_from_dir(source)
}

fn skills_config_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/local-harness/skills"))
}

fn install_skill_files(package_dir: &Path, skills: &[String], fallback_name: &str) -> Result<Vec<String>> {
    let mut installed = Vec::new();
    for relative in skills {
        let relative = Path::new(relative);
        if relative.is_absolute() || relative.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            bail!("invalid skill path `{}`", relative.display());
        }
        let source = package_dir.join(relative);
        if !source.is_file() {
            bail!("skill file `{}` is missing", source.display());
        }
        let source = source.canonicalize().with_context(|| format!("resolving {}", source.display()))?;
        if !source.starts_with(package_dir) {
            bail!("skill path escapes package: {}", source.display());
        }
        let parent = relative.parent().filter(|parent| !parent.as_os_str().is_empty());
        let name = parent
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| *name != "skills")
            .unwrap_or(fallback_name)
            .to_string();
        let destination = skills_config_dir()?.join(&name);
        std::fs::create_dir_all(&destination)?;
        std::fs::copy(&source, destination.join("SKILL.md"))?;
        installed.push(name);
    }
    Ok(installed)
}

fn install_skills(package_dir: &Path, plugins: &[Plugin]) -> Result<Vec<String>> {
    let mut installed = Vec::new();
    for plugin in plugins {
        installed.extend(install_skill_files(package_dir, &plugin.skills, &plugin.name)?);
    }
    Ok(installed)
}

fn merge_mcp_config(bundle: &McpBundle) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let path = PathBuf::from(home).join(".harness/mcp.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut root: serde_json::Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?
    } else {
        serde_json::json!({ "servers": [] })
    };
    let servers = root
        .get_mut("servers")
        .and_then(serde_json::Value::as_array_mut)
        .context("mcp.json has no servers array")?;
    servers.retain(|server| server.get("name").and_then(serde_json::Value::as_str) != Some(&bundle.name));
    servers.push(serde_json::json!({
        "name": bundle.name,
        "command": bundle.command,
        "args": bundle.args,
        "env": bundle.env,
        "url": bundle.url,
    }));
    std::fs::write(&path, serde_json::to_string_pretty(&root)? + "\n")?;
    Ok(path)
}

fn install_mcp_bundle(source: &Path) -> Result<Installed> {
    let direct = source.join("plugin.json");
    let text = std::fs::read_to_string(&direct)
        .with_context(|| format!("reading {}", direct.display()))?;
    let package: PackageFile = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", direct.display()))?;
    let bundle = package.mcp.context("plugin has no mcp bundle")?;
    let destination = registry_dir().join(&package.name);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    copy_dir(source, &destination)?;
    install_skill_files(&destination, &package.skills, &package.name)?;
    merge_mcp_config(&bundle)?;
    let mut registry = load_registry()?;
    registry.plugins.retain(|entry| entry.name != package.name);
    let entry = Installed {
        name: package.name.clone(),
        version: package.version,
        revision: package.revision,
        api: package.api,
        min_harness: package.min_harness,
        enabled: true,
        path: package.name,
        tools: Vec::new(),
        previous: None,
    };
    registry.plugins.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

fn install_from_dir(source: &Path) -> Result<Installed> {
    let loaded = load(source)?;
    if loaded.is_empty() {
        return install_mcp_bundle(source);
    }
    let Some(first) = loaded.first() else {
        bail!("empty plugin package");
    };
    let package_name = first.package.clone();
    let destination = registry_dir().join(&package_name);
    let mut registry = load_registry()?;
    let previous = registry
        .plugins
        .iter()
        .find(|entry| entry.name == package_name)
        .map(|entry| entry.version.clone());
    if destination.exists() {
        if let Some(old_version) = previous.as_deref() {
            let backup = registry_dir().join("versions").join(&package_name).join(old_version);
            if backup.exists() {
                std::fs::remove_dir_all(&backup)?;
            }
            copy_dir(&destination, &backup)?;
        }
        std::fs::remove_dir_all(&destination)?;
    }
    copy_dir(source, &destination)?;
    install_skills(&destination, &loaded)?;
    registry.plugins.retain(|entry| entry.name != package_name);
    let entry = Installed {
        name: package_name.clone(),
        version: first.version.clone(),
        revision: first.revision,
        api: first.api.clone(),
        min_harness: first.min_harness.clone(),
        enabled: true,
        path: package_name,
        tools: loaded.iter().map(|plugin| plugin.name.clone()).collect(),
        previous,
    };
    registry.plugins.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

pub fn rollback(name: &str) -> Result<bool> {
    let mut registry = load_registry()?;
    let Some(index) = registry.plugins.iter().position(|entry| entry.name == name) else {
        return Ok(false);
    };
    let entry = registry.plugins[index].clone();
    let Some(previous) = entry.previous.clone() else {
        bail!("plugin `{name}` has no previous version");
    };
    let current = registry_dir().join(&entry.path);
    let backup = registry_dir().join("versions").join(name).join(&previous);
    if !backup.is_dir() {
        bail!("previous version `{previous}` for `{name}` is missing");
    }
    let current_version = entry.version.clone();
    if current.exists() {
        let save = registry_dir().join("versions").join(name).join(&current_version);
        if save.exists() {
            std::fs::remove_dir_all(&save)?;
        }
        copy_dir(&current, &save)?;
        std::fs::remove_dir_all(&current)?;
    }
    copy_dir(&backup, &current)?;
    let loaded = load(&current)?;
    let Some(first) = loaded.first() else {
        bail!("rolled-back package is empty");
    };
    let updated = Installed {
        name: name.to_string(),
        version: first.version.clone(),
        revision: first.revision,
        api: first.api.clone(),
        min_harness: first.min_harness.clone(),
        enabled: entry.enabled,
        path: entry.path,
        tools: loaded.iter().map(|plugin| plugin.name.clone()).collect(),
        previous: Some(current_version),
    };
    registry.plugins[index] = updated;
    save_registry(&registry)?;
    Ok(true)
}

pub fn set_enabled(name: &str, enabled: bool) -> Result<bool> {
    let mut registry = load_registry()?;
    let Some(entry) = registry.plugins.iter_mut().find(|entry| entry.name == name) else {
        return Ok(false);
    };
    entry.enabled = enabled;
    save_registry(&registry)?;
    Ok(true)
}

pub fn remove(name: &str) -> Result<bool> {
    let mut registry = load_registry()?;
    let before = registry.plugins.len();
    registry.plugins.retain(|entry| entry.name != name);
    if registry.plugins.len() == before {
        return Ok(false);
    }
    let destination = registry_dir().join(name);
    if destination.exists() {
        std::fs::remove_dir_all(destination)?;
    }
    let versions = registry_dir().join("versions").join(name);
    if versions.exists() {
        std::fs::remove_dir_all(versions)?;
    }
    save_registry(&registry)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(version_tuple("1.2.3") > version_tuple("1.2.2"));
        assert!(version_tuple("2.0.0") > version_tuple("1.9.9"));
        assert_eq!(version_tuple("1.2"), version_tuple("1.2.0"));
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn default_permissions_are_restrictive() {
        let permissions = Permissions::default();
        assert!(!permissions.network);
        assert!(!permissions.host);
        assert_eq!(permissions.write, "workspace");
    }
}
