//! One agent loop: assemble prompt, call the model, run tools, repeat.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Result};
use serde_json::{Value, json};

use crate::governor::Governor;
use crate::mcp::McpServer;
use crate::plugins::Plugin;
use crate::llm::{Client, Message, Usage};
use crate::session::Session;
use crate::tools::{ToolPolicy, ToolScopes, ToolSet};

/// Receives streaming progress. CLI prints it; the server forwards it as SSE.
pub trait EventSink: Send {
    fn session(&mut self, _id: &str) {}
    fn notice(&mut self, _text: &str) {}
    fn step(&mut self, _step: usize) {}
    fn token(&mut self, _text: &str) {}
    fn tool_call(&mut self, _name: &str, _arguments: &str) {}
    fn tool_result(&mut self, _chars: usize) {}
    fn usage(&mut self, _prompt: u64, _completion: u64, _cached: u64) {}
    fn finish(&mut self) {}
}

#[derive(Clone)]
pub struct Config {
    pub endpoint: String,
    pub model: String,
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub max_steps: usize,
    pub max_tokens: u32,
    pub temperature: f32,
    pub policy: ToolPolicy,
    pub plugins: Arc<Vec<Plugin>>,
    pub mcp: Arc<Vec<Arc<McpServer>>>,
    pub scopes: ToolScopes,
    pub think: bool,
    pub governor: Governor,
    pub cancel: Option<Arc<AtomicBool>>,
    pub depth: usize,
    pub max_depth: usize,
    pub max_children: usize,
}

pub struct Outcome {
    pub content: String,
    pub usage: Option<Usage>,
}

pub async fn run(
    config: Config,
    session: &Session,
    prompt: &str,
    history: Vec<Message>,
    sink: &mut dyn EventSink,
) -> Result<Outcome> {
    let client = Client::new(&config.endpoint, &config.model)?;
    let tools = ToolSet::new(config.workspace.clone(), config.policy, config.plugins.clone(), config.mcp.clone(), config.scopes.clone());
    let can_delegate = config.depth < config.max_depth;
    let mut definitions = tools.definitions();
    let mut names = tools.names();
    if can_delegate {
        definitions.push(subagent_definition());
        names.push("subagent".to_string());
    }
    let system = system_prompt(&config.workspace, &names, config.depth);

    // A fresh session starts with system+user. Resume/fork histories keep their
    // messages, get the current system prompt, and append the new user turn.
    let fresh = history.is_empty();
    let mut messages = history;
    if fresh {
        messages.push(Message::system(system));
        messages.push(Message::user(prompt));
    } else {
        if messages
            .first()
            .map(|message| message.role == "system")
            .unwrap_or(false)
        {
            messages[0] = Message::system(system);
        } else {
            messages.insert(0, Message::system(system));
        }
        messages.push(Message::user(prompt));
    }

    sink.session(&session.id);
    session
        .append(serde_json::json!({
            "type": "policy",
            "policy": config.policy.label(),
            "allow": &config.scopes.allow,
            "deny": &config.scopes.deny,
        }))
        .await?;
    if fresh {
        for message in &messages {
            session
                .append(serde_json::json!({ "type": "message", "message": message }))
                .await?;
        }
    } else if let Some(message) = messages.last() {
        session
            .append(serde_json::json!({ "type": "message", "message": message }))
            .await?;
    }

    let mut final_usage: Option<Usage> = None;
    let mut children = 0usize;

    for step in 1..=config.max_steps {
        if config
            .cancel
            .as_ref()
            .map(|flag| flag.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            bail!("cancelled");
        }
        sink.step(step);

        if config.governor.estimate(&messages, &definitions) > config.governor.prompt_budget() {
            if let Some(summary) = compact(&config, &client, &mut messages, session).await? {
                sink.notice(&format!(
                    "compacted earlier history into a {} char summary",
                    summary.chars().count()
                ));
            }
        }

        let report = config.governor.fit(&mut messages, &definitions);
        if report.changed() {
            sink.notice(&format!("context: {}", report.summary()));
        }
        if report.estimate > report.budget {
            bail!(
                "prompt (~{} tokens) exceeds the local budget ({}); raise --context-limit or lower --max-tokens",
                report.estimate,
                report.budget
            );
        }

        {
            let mut on_token = |text: &str| sink.token(text);
            let turn = client
                .complete(&messages, &definitions, config.max_tokens, config.temperature, config.think, config.cancel.as_ref(), &mut on_token)
                .await?;
            drop(on_token);

            if let Some(usage) = &turn.usage {
                let cached = usage
                    .prompt_tokens_details
                    .as_ref()
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0);
                sink.usage(usage.prompt_tokens, usage.completion_tokens, cached);
                final_usage = Some(usage.clone());
            }

            let assistant = Message::assistant(turn.content.clone(), turn.tool_calls.clone());
            messages.push(assistant.clone());
            session
                .append(serde_json::json!({ "type": "message", "message": assistant }))
                .await?;

            if turn.tool_calls.is_empty() {
                if turn.finish_reason.as_deref() == Some("length") {
                    sink.notice("output truncated at max_tokens");
                }
                return Ok(Outcome { content: turn.content, usage: final_usage });
            }

            for call in &turn.tool_calls {
                sink.tool_call(&call.function.name, &call.function.arguments);
                let result = if call.function.name == "subagent" {
                    handle_subagent(&config, &call.function.arguments, session, sink, &mut children).await
                } else {
                    match tools.execute(&call.function.name, &call.function.arguments).await {
                        Ok(output) => output,
                        Err(error) => format!("tool error: {error:#}"),
                    }
                };
                sink.tool_result(result.chars().count());
                let message = Message::tool(call.id.clone(), result);
                messages.push(message.clone());
                session
                    .append(serde_json::json!({ "type": "message", "message": message }))
                    .await?;
            }
        }
    }

    bail!("reached max_steps={} without a final answer", config.max_steps)
}


/// Summarize all turns before the latest user message, then replace them with
/// one summary message. Returns the summary when compaction happened.
async fn compact(
    config: &Config,
    client: &Client,
    messages: &mut Vec<Message>,
    session: &Session,
) -> Result<Option<String>> {
    let Some(index) = messages.iter().rposition(|message| message.role == "user") else {
        return Ok(None);
    };
    if index <= 1 {
        return Ok(None);
    }

    let mut transcript = String::new();
    for message in &messages[1..index] {
        let content = message.content.as_deref().unwrap_or("");
        if content.is_empty() {
            continue;
        }
        transcript.push_str(message.role.as_str());
        transcript.push_str(": ");
        transcript.push_str(content);
        transcript.push('\n');
        if transcript.chars().count() > 6_000 {
            break;
        }
    }
    if transcript.trim().is_empty() {
        return Ok(None);
    }

    let system = "You compress conversation history for a local coding agent. Summarize the transcript below. Preserve facts, numbers, file paths, decisions, and unresolved tasks. Write at most 120 words. Output only the summary.";
    let request = vec![Message::system(system), Message::user(transcript)];
    let mut ignore = |_text: &str| {};
    let turn = client
        .complete(&request, &[], 200, 0.0, false, config.cancel.as_ref(), &mut ignore)
        .await?;
    let summary = turn.content.trim().to_string();
    if summary.is_empty() {
        return Ok(None);
    }

    messages.splice(
        1..index,
        std::iter::once(Message::user(format!(
            "[Summary of earlier conversation]\n{summary}"
        ))),
    );
    session
        .append(serde_json::json!({
            "type": "compaction",
            "removed_messages": index - 1,
            "summary": summary,
        }))
        .await?;
    Ok(Some(summary))
}

async fn handle_subagent(
    config: &Config,
    arguments: &str,
    parent: &Session,
    sink: &mut dyn EventSink,
    children: &mut usize,
) -> String {
    if config.depth >= config.max_depth {
        return "subagent error: delegation depth limit reached".to_string();
    }
    if *children >= config.max_children {
        return format!(
            "subagent error: max children ({}) reached for this run",
            config.max_children
        );
    }
    let task = match serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| value.get("task").and_then(Value::as_str).map(ToString::to_string))
    {
        Some(task) if !task.trim().is_empty() => task,
        _ => return "subagent error: missing `task`".to_string(),
    };

    *children += 1;
    sink.notice(&format!(
        "subagent {}/{} started: {}",
        *children,
        config.max_children,
        summary(&task, 80)
    ));

    let child_session = match Session::create(&config.data_dir, &config.model).await {
        Ok(session) => session,
        Err(error) => return format!("subagent error: {error:#}"),
    };
    let _ = child_session
        .append(serde_json::json!({
            "type": "subagent",
            "parent": parent.id,
            "task": task,
        }))
        .await;

    let child_config = Config {
        depth: config.depth + 1,
        max_steps: config.max_steps.min(4),
        max_tokens: config.max_tokens.min(512),
        ..config.clone()
    };
    let mut child_sink = NullSink;
    match Box::pin(run(
        child_config,
        &child_session,
        &task,
        Vec::new(),
        &mut child_sink,
    ))
    .await
    {
        Ok(outcome) => {
            sink.notice(&format!("subagent {} finished", child_session.id));
            let content = outcome.content.trim();
            let content = if content.is_empty() {
                "(subagent returned no text)"
            } else {
                content
            };
            format!(
                "subagent {} result:\n{}",
                child_session.id,
                truncate(content, 8_000)
            )
        }
        Err(error) => {
            sink.notice(&format!("subagent {} failed", child_session.id));
            format!("subagent error: {error:#}")
        }
    }
}

fn subagent_definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "subagent",
            "description": "Delegate ONE self-contained task to a child agent with its own context and return its final text. Use it to isolate noisy work; do not use it for tasks you can answer directly.",
            "parameters": {
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Complete, self-contained task for the child agent" }
                },
                "required": ["task"]
            }
        }
    })
}

fn system_prompt(workspace: &std::path::Path, tools: &[String], depth: usize) -> String {
    let role = if depth == 0 {
        "You are a local coding agent running inside the workspace"
    } else {
        "You are a subagent working for a parent agent inside the workspace"
    };
    let delegation = if depth == 0 {
        ""
    } else {
        "- You are a subagent: return a concise, self-contained result and do not ask follow-up questions.\n"
    };
    format!(
        "{role} `{}`.\n\
         Tools available: {}.\n\
         Use a tool whenever the task requires reading, writing, listing, or running something; otherwise answer directly.\n\
         Rules:\n\
         - Keep answers short and concrete. The user runs a local 9B model at about 10 tokens per second.\n\
         - Never invent tool output. If a tool fails, report the error.\n\
         - Read a file before editing it.\n\
         - Shell commands must be non-interactive.\n\
         {delegation}\
         - Answer directly; do not produce long deliberation.\n\
         Formatting:\n\
         - Use Markdown: short paragraphs, \"- \" bullet lists with one item per line, and fenced code blocks with a language tag.\n\
         - Preserve line breaks; never collapse a list or command output into one sentence.\n\
         - When listing directory contents, put one path per bullet.",
        workspace.display(),
        if tools.is_empty() { "none".to_string() } else { tools.join(", ") }
    )
}

fn summary(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&compact, limit)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("…");
    out
}

struct NullSink;

impl EventSink for NullSink {}
