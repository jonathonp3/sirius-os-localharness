//! `harness` — a local-first agent harness for a llama.cpp OpenAI-compatible server.

mod agent;
mod approval;
mod governor;
mod llm;
mod mcp;
mod plugins;
mod registry;
mod server;
mod session;
mod tools;
mod web;
mod workspaces;
mod workflow;

use std::io::Write as _;
use std::sync::Arc;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use agent::EventSink;

#[derive(Parser, Debug)]
#[command(name = "harness", version, about = "Local-first agent harness (llama.cpp backend)")]
struct Cli {
    /// Registry profile to use (defaults to the registry's `default`).
    #[arg(long, global = true, env = "HARNESS_PROFILE")]
    profile: Option<String>,
    /// Path to models.json (defaults to ~/.config/local-harness/models.json).
    #[arg(long, global = true, env = "HARNESS_REGISTRY")]
    registry: Option<PathBuf>,
    /// llama.cpp base URL; overrides the profile.
    #[arg(long, global = true, env = "HARNESS_ENDPOINT")]
    endpoint: Option<String>,
    /// Model id; overrides the profile.
    #[arg(long, global = true, env = "HARNESS_MODEL")]
    model: Option<String>,
    /// Environment variable holding the API key for remote OpenAI-compatible providers.
    #[arg(long, global = true, env = "HARNESS_API_KEY_ENV")]
    api_key_env: Option<String>,
    #[arg(long, global = true, env = "HARNESS_WORKSPACE")]
    workspace: Option<PathBuf>,
    #[arg(long, global = true, env = "HARNESS_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Prompt-token ceiling the governor enforces (clamped to the model context).
    #[arg(long, global = true, env = "HARNESS_CONTEXT_LIMIT")]
    context_limit: Option<usize>,
    /// Sampling temperature; overrides the profile (default 0.0).
    #[arg(long, global = true, env = "HARNESS_TEMPERATURE")]
    temperature: Option<f32>,
    /// Report mutating tool calls without executing them.
    #[arg(long, global = true, env = "HARNESS_DRY_RUN")]
    dry_run: bool,
    /// Auto-approve destructive commands (no prompt).
    #[arg(long, global = true, env = "HARNESS_YES")]
    yes: bool,
    /// Enable read-only web search/fetch tools (each call asks for approval).
    #[arg(long, global = true, env = "HARNESS_ALLOW_WEB")]
    allow_web: bool,
    /// Tool policy: read-only, write, or full.
    #[arg(long, global = true, env = "HARNESS_TOOL_POLICY")]
    tool_policy: Option<String>,
    /// JSON file with command-backed plugin tools (requires full policy).
    #[arg(long, global = true, env = "HARNESS_PLUGINS")]
    plugins: Option<PathBuf>,
    /// JSON file with MCP servers (stdio command or HTTP url; requires full policy).
    #[arg(long, global = true, env = "HARNESS_MCP")]
    mcp: Option<PathBuf>,
    /// Comma-separated tool allowlist; `prefix*` wildcards are supported.
    #[arg(long, global = true, env = "HARNESS_ALLOW_TOOLS")]
    allow_tools: Option<String>,
    /// Comma-separated tool denylist; `prefix*` wildcards are supported.
    #[arg(long, global = true, env = "HARNESS_DENY_TOOLS")]
    deny_tools: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run one prompt through the agent loop.
    Run {
        /// The task or question.
        prompt: String,
        /// Continue an existing session by id.
        #[arg(long, conflicts_with = "fork")]
        resume: Option<String>,
        /// Start a new session seeded with an existing session's history.
        #[arg(long, conflicts_with = "resume")]
        fork: Option<String>,
        /// Maximum model/tool steps before giving up.
        #[arg(long, default_value_t = 8)]
        max_steps: usize,
        /// Maximum tokens to generate per model call.
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Enable the write_file tool.
        #[arg(long)]
        allow_write: bool,
        /// Enable the bash tool.
        #[arg(long)]
        allow_shell: bool,
        /// Enable the model's thinking/reasoning mode.
        #[arg(long)]
        think: bool,
        /// Maximum delegation depth (1 means children cannot spawn).
        #[arg(long, default_value_t = 1)]
        max_depth: usize,
        /// Maximum child agents started by one run.
        #[arg(long, default_value_t = 4)]
        max_children: usize,
    },
    /// Serve the browser client over HTTP + SSE on loopback.
    Serve {
        /// Loopback port to listen on.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Bearer/query token; generated when omitted.
        #[arg(long)]
        token: Option<String>,
        /// Open the browser URL after binding.
        #[arg(long)]
        open: bool,
        /// Concurrent chats; default uses the model server's slot count.
        #[arg(long)]
        concurrency: Option<usize>,
        /// Maximum model/tool steps per chat.
        #[arg(long, default_value_t = 8)]
        max_steps: usize,
        /// Maximum tokens to generate per model call.
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Enable the write_file tool.
        #[arg(long)]
        allow_write: bool,
        /// Enable the bash tool.
        #[arg(long)]
        allow_shell: bool,
        /// Enable the model's thinking/reasoning mode.
        #[arg(long)]
        think: bool,
        /// Maximum delegation depth (1 means children cannot spawn).
        #[arg(long, default_value_t = 1)]
        max_depth: usize,
        /// Maximum child agents started by one run.
        #[arg(long, default_value_t = 4)]
        max_children: usize,
    },
    /// Run a sequential workflow from a JSON file.
    Workflow {
        /// Workflow JSON file.
        file: PathBuf,
        /// Maximum model/tool steps per workflow step.
        #[arg(long, default_value_t = 4)]
        max_steps: usize,
        /// Maximum tokens per workflow step.
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Enable the write_file tool.
        #[arg(long)]
        allow_write: bool,
        /// Enable the bash tool.
        #[arg(long)]
        allow_shell: bool,
        /// Enable the model's thinking/reasoning mode.
        #[arg(long)]
        think: bool,
        /// Maximum delegation depth (workflows need at least 1).
        #[arg(long, default_value_t = 1)]
        max_depth: usize,
    },
    /// Manage versioned command-backed plugin packages.
    Plugins {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// List registry profiles and models exposed by the server.
    Models,
    /// List local session logs.
    Sessions,
    /// Show the active profile, model, hardware, context budget, and health.
    Doctor,
}

#[derive(Subcommand, Debug)]
enum PluginAction {
    /// List installed plugins and their enabled state.
    List,
    /// Install a plugin package directory into the local registry.
    Install { path: PathBuf },
    /// Install a new version, preserving the current one for rollback.
    Update { path: PathBuf },
    /// Restore the previous version of a plugin.
    Rollback { name: String },
    /// Enable an installed plugin.
    Enable { name: String },
    /// Disable an installed plugin without removing it.
    Disable { name: String },
    /// Remove an installed plugin and its files.
    Remove { name: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    reset_sigpipe();
    let cli = Cli::parse();

    let workspace = match &cli.workspace {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("reading current directory")?,
    };
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("resolving workspace {}", workspace.display()))?;

    let data_dir = match cli.data_dir.clone() {
        Some(path) => path,
        None => default_data_dir()?,
    };

    let registry_path = match &cli.registry {
        Some(path) => path.clone(),
        None => registry::discover(&workspace).unwrap_or(registry::default_path()?),
    };
    let loaded = registry::load(&registry_path)?;
    let profile_name = cli
        .profile
        .clone()
        .or_else(|| loaded.as_ref().and_then(|registry| registry.default.clone()));
    let profile = match (&profile_name, &loaded) {
        (Some(name), Some(registry)) => match registry.models.get(name) {
            Some(profile) => Some(profile.clone()),
            None => anyhow::bail!("profile `{name}` not found in {}", registry_path.display()),
        },
        (Some(name), None) => anyhow::bail!(
            "profile `{name}` requested but {} does not exist",
            registry_path.display()
        ),
        (None, _) => None,
    };

    let endpoint = cli
        .endpoint
        .clone()
        .or_else(|| profile.as_ref().and_then(|profile| profile.endpoint.clone()))
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let model = cli
        .model
        .clone()
        .or_else(|| profile.as_ref().map(|profile| profile.model.clone()))
        .unwrap_or_else(|| "Qwen3.5-9B-Q4_K_M".to_string());
    let context_limit = cli
        .context_limit
        .or_else(|| profile.as_ref().and_then(|profile| profile.context_limit))
        .unwrap_or(32_768);
    let temperature = cli
        .temperature
        .or_else(|| profile.as_ref().and_then(|profile| profile.temperature))
        .unwrap_or(0.0);
    let api_key_env = cli
        .api_key_env
        .clone()
        .or_else(|| profile.as_ref().and_then(|profile| profile.api_key_env.clone()));
    let api_key = api_key_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty());
    let remote = profile
        .as_ref()
        .and_then(|profile| profile.provider.as_deref())
        .map(|provider| provider == "openai" || provider == "remote")
        .unwrap_or(false)
        || api_key_env.is_some()
        || (!endpoint.contains("127.0.0.1") && !endpoint.contains("localhost"));
    let profile_label = profile_name.clone().unwrap_or_else(|| "none".to_string());
    let base_policy = match &cli.tool_policy {
        Some(name) => tools::ToolPolicy::parse(name).with_context(|| {
            format!("invalid tool policy `{name}` (use read-only, write, or full)")
        })?,
        None => profile
            .as_ref()
            .and_then(|profile| profile.tool_policy.as_deref())
            .and_then(tools::ToolPolicy::parse)
            .unwrap_or(tools::ToolPolicy::ReadOnly),
    };
    plugins::bootstrap_bundled()?;
    let plugins_path = cli
        .plugins
        .clone()
        .or_else(|| registry::discover_named(&workspace, "plugins.json"))
        .or_else(|| {
            let directory = workspace.join("plugins");
            directory.is_dir().then_some(directory)
        });
    let plugin_tools: Arc<Vec<plugins::Plugin>> = match plugins_path {
        Some(path) => Arc::new(plugins::load(&path)?),
        None => {
            let mut loaded = plugins::load_installed()?;
            let workspace_plugins = workspace.join("plugins");
            if workspace_plugins.is_dir() {
                loaded.extend(plugins::load(&workspace_plugins)?);
            }
            Arc::new(loaded)
        }
    };
    let mcp_path = cli
        .mcp
        .clone()
        .or_else(|| registry::discover_named(&workspace, "mcp.json"));
    let mcp_load = match mcp_path {
        Some(path) => mcp::load(&path).await?,
        None => mcp::McpLoad::default(),
    };
    let mcp_servers: Arc<Vec<Arc<mcp::McpServer>>> = Arc::new(mcp_load.servers);
    let mcp_failures = Arc::new(mcp_load.failures);
    let mcp_tool_count: usize = mcp_servers.iter().map(|server| server.tools.len()).sum();
    let scopes = tools::ToolScopes {
        allow: cli
            .allow_tools
            .as_deref()
            .map(split_patterns)
            .or_else(|| profile.as_ref().and_then(|profile| profile.allow_tools.clone()))
            .unwrap_or_default(),
        deny: cli
            .deny_tools
            .as_deref()
            .map(split_patterns)
            .or_else(|| profile.as_ref().and_then(|profile| profile.deny_tools.clone()))
            .unwrap_or_default(),
    };

    match cli.command {
        Command::Run { prompt, resume, fork, max_steps, max_tokens, allow_write, allow_shell, think, max_depth, max_children } => {
            let max_tokens = max_tokens
                .or_else(|| profile.as_ref().and_then(|profile| profile.max_tokens))
                .unwrap_or(512);
            let think = think || profile.as_ref().and_then(|profile| profile.think).unwrap_or(false);
            let policy = if allow_shell {
                tools::ToolPolicy::Full
            } else if allow_write {
                tools::ToolPolicy::Write
            } else {
                base_policy
            };
            let governor = load_governor(&endpoint, &model, context_limit, max_tokens, remote).await;
            let mut history = Vec::new();
            let session = if let Some(id) = resume {
                let session = session::Session::open(&data_dir, &id).await?;
                history = session.load_messages().await?;
                session
            } else if let Some(id) = fork {
                let parent = session::Session::open(&data_dir, &id).await?;
                history = parent.load_messages().await?;
                let session = session::Session::create(&data_dir, &model).await?;
                session
                    .append(serde_json::json!({ "type": "fork", "parent": parent.id }))
                    .await?;
                for message in &history {
                    session
                        .append(serde_json::json!({ "type": "message", "message": message }))
                        .await?;
                }
                session
            } else {
                session::Session::create(&data_dir, &model).await?
            };
            let config = agent::Config {
                endpoint,
                model,
                api_key: api_key.clone(),
                remote,
                workspace,
                max_steps,
                max_tokens,
                temperature,
                policy,
                plugins: plugin_tools.clone(),
                mcp: mcp_servers.clone(),
                scopes: scopes.clone(),
                approver: if cli.yes { approval::auto() } else { approval::stdio() },
                dry_run: cli.dry_run,
                think,
                governor,
                cancel: None,
                data_dir: data_dir.clone(),
                depth: 0,
                max_depth,
                max_children,
                web: cli.allow_web,
            };
            let mut sink = CliSink::default();
            let result = agent::run(config, &session, &prompt, history, &mut sink).await;
            sink.finish();
            result?;
            Ok(())
        }
        Command::Serve { port, token, open, concurrency, max_steps, max_tokens, allow_write, allow_shell, think, max_depth, max_children } => {
            let max_tokens = max_tokens
                .or_else(|| profile.as_ref().and_then(|profile| profile.max_tokens))
                .unwrap_or(512);
            let think = think || profile.as_ref().and_then(|profile| profile.think).unwrap_or(false);
            let policy = if allow_shell {
                tools::ToolPolicy::Full
            } else if allow_write {
                tools::ToolPolicy::Write
            } else {
                base_policy
            };
            let governor = load_governor(&endpoint, &model, context_limit, max_tokens, remote).await;
            let concurrency = concurrency.unwrap_or(governor.slots.max(1)).max(1);
            let config = agent::Config {
                endpoint,
                model,
                api_key: api_key.clone(),
                remote,
                workspace,
                max_steps,
                max_tokens,
                temperature,
                policy,
                plugins: plugin_tools.clone(),
                mcp: mcp_servers.clone(),
                scopes: scopes.clone(),
                approver: if cli.yes { approval::auto() } else { approval::deny() },
                dry_run: cli.dry_run,
                think,
                governor,
                cancel: None,
                data_dir: data_dir.clone(),
                depth: 0,
                max_depth,
                max_children,
                web: cli.allow_web,
            };
            server::run(config, data_dir, port, token, open, concurrency, cli.yes, mcp_failures).await
        }
        Command::Workflow { file, max_steps, max_tokens, allow_write, allow_shell, think, max_depth } => {
            let max_tokens = max_tokens
                .or_else(|| profile.as_ref().and_then(|profile| profile.max_tokens))
                .unwrap_or(512);
            let think = think || profile.as_ref().and_then(|profile| profile.think).unwrap_or(false);
            let policy = if allow_shell {
                tools::ToolPolicy::Full
            } else if allow_write {
                tools::ToolPolicy::Write
            } else {
                base_policy
            };
            let governor = load_governor(&endpoint, &model, context_limit, max_tokens, remote).await;
            let session = session::Session::create(&data_dir, &model).await?;
            let config = agent::Config {
                endpoint,
                model,
                api_key: api_key.clone(),
                remote,
                workspace,
                data_dir: data_dir.clone(),
                max_steps,
                max_tokens,
                temperature,
                policy,
                plugins: plugin_tools.clone(),
                mcp: mcp_servers.clone(),
                scopes: scopes.clone(),
                approver: if cli.yes { approval::auto() } else { approval::stdio() },
                dry_run: cli.dry_run,
                think,
                governor,
                cancel: None,
                depth: 0,
                max_depth,
                max_children: 4,
                web: cli.allow_web,
            };
            eprintln!("[workflow] session {}", session.id);
            workflow::run(config, &file, &session).await
        }
        Command::Plugins { action } => {
            match action {
                PluginAction::List => {
                    let entries = plugins::list_installed()?;
                    if entries.is_empty() {
                        println!("no installed plugins");
                    }
                    for entry in entries {
                        println!(
                            "{} {} r{} {}{}",
                            if entry.enabled { "on " } else { "off" },
                            entry.name,
                            entry.revision,
                            entry.version,
                            if entry.enabled { "" } else { " (disabled)" }
                        );
                    }
                }
                PluginAction::Install { path } => {
                    let entry = plugins::install(&path)?;
                    println!("installed {} {} r{}", entry.name, entry.version, entry.revision);
                }
                PluginAction::Update { path } => {
                    let entry = plugins::install(&path)?;
                    println!("updated {} {} r{} (previous preserved)", entry.name, entry.version, entry.revision);
                }
                PluginAction::Rollback { name } => {
                    if !plugins::rollback(&name)? {
                        bail!("unknown plugin `{name}` or no previous version");
                    }
                    println!("rolled back {name}");
                }
                PluginAction::Enable { name } => {
                    if !plugins::set_enabled(&name, true)? {
                        bail!("unknown plugin `{name}`");
                    }
                    println!("enabled {name}");
                }
                PluginAction::Disable { name } => {
                    if !plugins::set_enabled(&name, false)? {
                        bail!("unknown plugin `{name}`");
                    }
                    println!("disabled {name}");
                }
                PluginAction::Remove { name } => {
                    if !plugins::remove(&name)? {
                        bail!("unknown plugin `{name}`");
                    }
                    println!("removed {name}");
                }
            }
            Ok(())
        }
        Command::Models => {
            if let Some(registry) = &loaded {
                let default = registry.default.as_deref().unwrap_or("-");
                println!("profiles from {} (default: {default})", registry_path.display());
                let mut names: Vec<&String> = registry.models.keys().collect();
                names.sort();
                for name in names {
                    let profile = &registry.models[name];
                    println!(
                        "  {name}{} model={} endpoint={} ctx={} max_tokens={} temp={} think={}",
                        if name.as_str() == default { " *" } else { "" },
                        profile.model,
                        profile.endpoint.as_deref().unwrap_or(endpoint.as_str()),
                        profile.context_limit.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                        profile.max_tokens.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                        profile.temperature.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                        profile.think.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                    );
                }
            } else {
                println!("no registry at {} (using flags/defaults)", registry_path.display());
            }
            let url = format!("{}/v1/models", endpoint.trim_end_matches('/'));
            match reqwest::get(&url).await {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(value) => {
                                if let Some(items) = value.get("data").and_then(|data| data.as_array()) {
                                    for item in items {
                                        if let Some(id) = item.get("id").and_then(|id| id.as_str()) {
                                            println!("{id}");
                                        }
                                    }
                                } else {
                                    println!("{text}");
                                }
                            }
                            Err(_) => println!("{text}"),
                        }
                    } else {
                        eprintln!("[warn] {url} returned {status}");
                    }
                }
                Err(error) => eprintln!("[warn] model server unreachable: {error}"),
            }
            Ok(())
        }
        Command::Sessions => {
            let dir = data_dir.join("sessions");
            if !dir.exists() {
                println!("no sessions in {}", dir.display());
                return Ok(());
            }
            let mut entries = tokio::fs::read_dir(&dir).await?;
            let mut files = Vec::new();
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jsonl") {
                    files.push(name);
                }
            }
            files.sort();
            for file in files {
                println!("{file}");
            }
            Ok(())
        }
        Command::Doctor => {
            let max_tokens = profile
                .as_ref()
                .and_then(|profile| profile.max_tokens)
                .unwrap_or(512);
            doctor(
                &endpoint,
                &model,
                context_limit,
                max_tokens,
                temperature,
                base_policy,
                plugin_tools.len(),
                mcp_servers.len(),
                mcp_tool_count,
                &scopes,
                &profile_label,
                &registry_path,
                remote,
                api_key.is_some(),
            )
            .await
        }
    }
}

async fn load_governor(endpoint: &str, model: &str, context_limit: usize, max_tokens: u32, remote: bool) -> governor::Governor {
    if remote {
        return governor::Governor::new(
            governor::ModelInfo { model: model.to_string(), n_ctx: context_limit, slots: 1 },
            context_limit,
            max_tokens,
        );
    }
    match governor::probe_model(endpoint, model).await {
        Ok(info) => governor::Governor::new(info, context_limit, max_tokens),
        Err(error) => {
            eprintln!("[warn] model probe failed: {error:#}; using defaults");
            governor::Governor::new(governor::ModelInfo::default(), context_limit, max_tokens)
        }
    }
}

async fn doctor(
    endpoint: &str,
    model: &str,
    context_limit: usize,
    max_tokens: u32,
    temperature: f32,
    policy: tools::ToolPolicy,
    plugin_count: usize,
    mcp_servers: usize,
    mcp_tools: usize,
    scopes: &tools::ToolScopes,
    profile: &str,
    registry_path: &std::path::Path,
    remote: bool,
    key_configured: bool,
) -> Result<()> {
    let hardware = governor::Hardware::probe();
    let health = if remote {
        true
    } else {
        reqwest::get(format!("{}/health", endpoint.trim_end_matches('/')))
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    };
    let info = if remote {
        Ok(governor::ModelInfo { model: model.to_string(), n_ctx: context_limit, slots: 1 })
    } else {
        governor::probe_model(endpoint, model).await
    };

    println!("profile: {profile} ({})", registry_path.display());
    println!("endpoint: {endpoint}");
    println!("provider: {}", if remote { "remote (OpenAI-compatible)" } else { "local llama.cpp" });
    if remote {
        println!("api key: {}", if key_configured { "configured" } else { "missing" });
    }
    println!("health: {}", if health { "ok" } else { "unreachable" });
    if let Err(error) = &info {
        println!("model probe: {error:#}");
    }
    let governor = governor::Governor::new(info.unwrap_or_default(), context_limit, max_tokens);
    println!(
        "model: {} (requested {model})",
        if governor.model.is_empty() { model } else { governor.model.as_str() }
    );
    println!("context: {} tokens", governor.model_ctx);
    println!("slots: {}", governor.slots);
    println!("context limit: {}", governor.context_limit);
    println!("prompt budget: {} tokens", governor.prompt_budget());
    println!("max output: {} tokens", governor.max_tokens);
    println!("temperature: {temperature}");
    println!("tool policy: {}", policy.label());
    println!("plugins: {plugin_count}");
    println!("mcp: {mcp_servers} server(s), {mcp_tools} tool(s)");
    println!(
        "tool scopes: allow={} deny={}",
        if scopes.allow.is_empty() { "-".to_string() } else { scopes.allow.join(",") },
        if scopes.deny.is_empty() { "-".to_string() } else { scopes.deny.join(",") }
    );
    println!(
        "hardware: {} cpus, RAM {} MiB ({} available)",
        hardware.cpus, hardware.ram_total_mb, hardware.ram_available_mb
    );
    match (hardware.vram_total_mb, hardware.vram_used_mb) {
        (Some(total), Some(used)) => println!("vram: {used} / {total} MiB"),
        _ => println!("vram: unavailable"),
    }
    Ok(())
}

/// Restore default SIGPIPE behavior so `harness ... | head` exits quietly
/// instead of panicking on a closed stdout.
fn reset_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn split_patterns(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn default_data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("local-harness"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/share/local-harness"))
}

#[derive(Default)]
struct CliSink {
    printed: bool,
}

impl EventSink for CliSink {
    fn session(&mut self, id: &str) {
        eprintln!("[session] {id}");
    }

    fn notice(&mut self, text: &str) {
        eprintln!("[notice] {text}");
    }

    fn step(&mut self, step: usize) {
        eprintln!("[step {step}]");
    }

    fn token(&mut self, text: &str) {
        print!("{text}");
        let _ = std::io::stdout().flush();
        self.printed = true;
    }

    fn tool_call(&mut self, name: &str, arguments: &str) {
        eprintln!("[tool] {name}({arguments})");
    }

    fn tool_result(&mut self, result: &str) {
        eprintln!("[tool] result: {} chars", result.chars().count());
    }

    fn usage(&mut self, prompt: u64, completion: u64, cached: u64) {
        if self.printed {
            println!();
            self.printed = false;
        }
        eprintln!("[tokens] prompt={prompt} completion={completion} cached={cached}");
    }

    fn finish(&mut self) {
        if self.printed {
            println!();
            self.printed = false;
        }
    }
}
