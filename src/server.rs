//! HTTP + SSE host. The browser client consumes the same agent loop as the CLI.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::agent::{self, EventSink};
use crate::approval::{self, ApprovalRequest, Approver};
use crate::session::Session;

const INDEX: &str = include_str!("index.html");
const MARKDOWN_JS: &str = include_str!("assets/markdown-it.min.js");
const HIGHLIGHT_JS: &str = include_str!("assets/highlight.min.js");
const PURIFY_JS: &str = include_str!("assets/purify.min.js");
const HIGHLIGHT_CSS: &str = include_str!("assets/highlight.css");
const TYPO_JS: &str = include_str!("assets/typo.js");
const EN_AU_AFF: &str = include_str!("assets/en_AU.aff");
const EN_AU_DIC: &str = include_str!("assets/en_AU.dic");

struct AppState {
    config: agent::Config,
    data_dir: PathBuf,
    token: String,
    http: reqwest::Client,
    slots: Semaphore,
    concurrency: usize,
    queued: AtomicUsize,
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    auto_approve: bool,
}

pub async fn run(
    config: agent::Config,
    data_dir: PathBuf,
    port: u16,
    token: Option<String>,
    open: bool,
    concurrency: usize,
    auto_approve: bool,
) -> Result<()> {
    let token = match token {
        Some(token) => token,
        None => random_token()?,
    };
    let state = Arc::new(AppState {
        config,
        data_dir,
        token: token.clone(),
        http: reqwest::Client::new(),
        slots: Semaphore::new(concurrency),
        concurrency,
        queued: AtomicUsize::new(0),
        pending_approvals: Arc::new(Mutex::new(HashMap::new())),
        auto_approve,
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/{file}", get(asset))
        .route("/api/health", get(health))
        .route("/api/models", get(models))
        .route("/api/chat", post(chat))
        .route("/api/approval", post(approval))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .context("binding 127.0.0.1")?;
    let address = listener.local_addr()?;
    println!("harness serve listening on http://{address}/?token={token}");
    if open {
        let url = format!("http://{address}/?token={token}");
        // Inside a Flatpak sandbox, ask the host to open the browser.
        if std::env::var_os("FLATPAK_ID").is_some() {
            let _ = std::process::Command::new("flatpak-spawn")
                .args(["--host", "xdg-open", &url])
                .spawn();
        } else {
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
        }
    }
    axum::serve(listener, app).await.context("serving HTTP")?;
    Ok(())
}

async fn index() -> Response {
    Html(INDEX).into_response()
}

async fn asset(Path(file): Path<String>) -> Response {
    let (content_type, content) = match file.as_str() {
        "markdown-it.min.js" => ("application/javascript; charset=utf-8", MARKDOWN_JS),
        "highlight.min.js" => ("application/javascript; charset=utf-8", HIGHLIGHT_JS),
        "purify.min.js" => ("application/javascript; charset=utf-8", PURIFY_JS),
        "highlight.css" => ("text/css; charset=utf-8", HIGHLIGHT_CSS),
        "typo.js" => ("application/javascript; charset=utf-8", TYPO_JS),
        "en_AU.aff" => ("text/plain; charset=utf-8", EN_AU_AFF),
        "en_AU.dic" => ("text/plain; charset=utf-8", EN_AU_DIC),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let mut response = content.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

async fn health() -> Response {
    Json(json!({ "status": "ok" })).into_response()
}

async fn models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let url = format!("{}/v1/models", state.config.endpoint.trim_end_matches('/'));
    match state.http.get(&url).send().await {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return (StatusCode::BAD_GATEWAY, text).into_response();
            }
            match response.json::<serde_json::Value>().await {
                Ok(value) => Json(value).into_response(),
                Err(error) => {
                    (StatusCode::BAD_GATEWAY, format!("invalid model list: {error}")).into_response()
                }
            }
        }
        Err(error) => {
            (StatusCode::BAD_GATEWAY, format!("model server unreachable: {error}")).into_response()
        }
    }
}

async fn chat(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<ChatBody>,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    if body.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "prompt is empty").into_response();
    }

    let (tx, rx) = mpsc::unbounded_channel::<SseEvent>();
    let position = state.queued.fetch_add(1, Ordering::SeqCst) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    let task_cancel = cancel.clone();
    let task_state = state.clone();
    tokio::spawn(async move {
        let _ = tx.send(SseEvent::Queued { position, slots: task_state.concurrency });
        // One permit per model slot; extra chats wait here.
        let _permit = task_state.slots.acquire().await;
        task_state.queued.fetch_sub(1, Ordering::SeqCst);
        if task_cancel.load(Ordering::SeqCst) {
            eprintln!("[chat] cancelled while queued");
            return;
        }
        let session = match Session::create(&task_state.data_dir, &task_state.config.model).await {
            Ok(session) => session,
            Err(error) => {
                let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                return;
            }
        };
        let mut config = task_state.config.clone();
        config.cancel = Some(task_cancel.clone());
        config.approver = if task_state.auto_approve {
            approval::auto()
        } else {
            Arc::new(SseApprover {
                tx: tx.clone(),
                pending: task_state.pending_approvals.clone(),
                session: session.id.clone(),
                counter: AtomicU64::new(0),
            })
        };
        let mut sink = ChannelSink { tx: tx.clone() };
        match agent::run(config, &session, body.prompt.as_str(), Vec::new(), &mut sink).await {
            Ok(outcome) => {
                let usage = outcome.usage;
                let cached_tokens = usage
                    .as_ref()
                    .and_then(|usage| usage.prompt_tokens_details.as_ref())
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0);
                let _ = tx.send(SseEvent::Done {
                    session: session.id.clone(),
                    prompt_tokens: usage.as_ref().map(|usage| usage.prompt_tokens).unwrap_or(0),
                    completion_tokens: usage.as_ref().map(|usage| usage.completion_tokens).unwrap_or(0),
                    cached_tokens,
                });
            }
            Err(error) => {
                let message = format!("{error:#}");
                if message.contains("cancelled") {
                    eprintln!("[chat] cancelled");
                } else {
                    let _ = tx.send(SseEvent::Error { message });
                }
            }
        }
    });

    // Dropping the response body (client abort) sets the flag so the agent
    // stops at the next chunk instead of generating into a dead stream.
    let guard = CancelOnDrop(cancel);
    let stream = UnboundedReceiverStream::new(rx).map(move |event| {
        let _ = &guard;
        Ok::<_, Infallible>(to_event(event))
    });
    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

async fn approval(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<ApprovalBody>,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let sender = state.pending_approvals.lock().await.remove(&body.id);
    match sender {
        Some(sender) => {
            let _ = sender.send(body.decision);
            (StatusCode::OK, "ok").into_response()
        }
        None => (StatusCode::NOT_FOUND, "unknown or expired approval").into_response(),
    }
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

fn authorized(state: &AppState, query: &AuthQuery, headers: &HeaderMap) -> bool {
    if let Some(token) = &query.token {
        if token_eq(token, &state.token) {
            return true;
        }
    }
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()) {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if token_eq(token, &state.token) {
                return true;
            }
        }
    }
    false
}

fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for index in 0..a.len() {
        diff |= a[index] ^ b[index];
    }
    diff == 0
}

fn random_token() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .context("reading /dev/urandom")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(""))
}

#[derive(Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
struct ChatBody {
    prompt: String,
}

#[derive(Deserialize)]
struct ApprovalBody {
    id: String,
    decision: bool,
}

enum SseEvent {
    Queued { position: usize, slots: usize },
    Token(String),
    Step(usize),
    ToolCall { name: String, arguments: String },
    ToolResult { chars: usize, preview: String },
    Notice(String),
    Approval { id: String, name: String, arguments: String, reason: String },
    Done { session: String, prompt_tokens: u64, completion_tokens: u64, cached_tokens: u64 },
    Error { message: String },
}

fn to_event(event: SseEvent) -> Event {
    match event {
        SseEvent::Queued { position, slots } => Event::default().event("queued").data(json!({ "position": position, "slots": slots }).to_string()),
        SseEvent::Token(text) => Event::default().event("token").data(json!({ "text": text }).to_string()),
        SseEvent::Step(step) => Event::default().event("step").data(json!({ "step": step }).to_string()),
        SseEvent::ToolCall { name, arguments } => Event::default()
            .event("tool_call")
            .data(json!({ "name": name, "arguments": arguments }).to_string()),
        SseEvent::ToolResult { chars, preview } => Event::default()
            .event("tool_result")
            .data(json!({ "chars": chars, "preview": preview }).to_string()),
        SseEvent::Notice(message) => Event::default().event("notice").data(json!({ "message": message }).to_string()),
        SseEvent::Approval { id, name, arguments, reason } => Event::default()
            .event("approval")
            .data(json!({ "id": id, "name": name, "arguments": arguments, "reason": reason }).to_string()),
        SseEvent::Done { session, prompt_tokens, completion_tokens, cached_tokens } => Event::default()
            .event("done")
            .data(
                json!({
                    "session": session,
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "cached_tokens": cached_tokens,
                })
                .to_string(),
            ),
        SseEvent::Error { message } => Event::default()
            .event("error")
            .data(json!({ "message": message }).to_string()),
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct SseApprover {
    tx: mpsc::UnboundedSender<SseEvent>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    session: String,
    counter: AtomicU64,
}

#[async_trait::async_trait]
impl Approver for SseApprover {
    async fn approve(&self, request: &ApprovalRequest) -> bool {
        let id = format!(
            "{}-{}",
            self.session,
            self.counter.fetch_add(1, Ordering::SeqCst)
        );
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);
        let _ = self.tx.send(SseEvent::Approval {
            id: id.clone(),
            name: request.name.clone(),
            arguments: request.arguments.clone(),
            reason: request.reason.clone(),
        });
        let decision = tokio::time::timeout(Duration::from_secs(120), receiver)
            .await
            .ok()
            .and_then(|result| result.ok())
            .unwrap_or(false);
        self.pending.lock().await.remove(&id);
        decision
    }
}

struct ChannelSink {
    tx: mpsc::UnboundedSender<SseEvent>,
}

impl EventSink for ChannelSink {
    fn step(&mut self, step: usize) {
        let _ = self.tx.send(SseEvent::Step(step));
    }

    fn token(&mut self, text: &str) {
        let _ = self.tx.send(SseEvent::Token(text.to_string()));
    }

    fn tool_call(&mut self, name: &str, arguments: &str) {
        let _ = self.tx.send(SseEvent::ToolCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
        });
    }

    fn tool_result(&mut self, result: &str) {
        let chars = result.chars().count();
        let preview: String = result.chars().take(4_000).collect();
        let _ = self.tx.send(SseEvent::ToolResult { chars, preview });
    }

    fn notice(&mut self, text: &str) {
        let _ = self.tx.send(SseEvent::Notice(text.to_string()));
    }

    fn usage(&mut self, _prompt: u64, _completion: u64, _cached: u64) {}
}
