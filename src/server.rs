//! HTTP + SSE host. The browser client consumes the same agent loop as the CLI.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use anyhow::{bail, Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::{AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE}};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::agent::{self, EventSink};
use crate::approval::{self, ApprovalRequest, Approver};
use crate::session::{self, Session};
use crate::workspaces;

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
    session_lock: Arc<Mutex<()>>,
    active_sessions: Arc<StdMutex<HashSet<String>>>,
    mcp_failures: Arc<Vec<crate::mcp::McpFailure>>,
}

pub async fn run(
    config: agent::Config,
    data_dir: PathBuf,
    port: u16,
    token: Option<String>,
    open: bool,
    concurrency: usize,
    auto_approve: bool,
    mcp_failures: Arc<Vec<crate::mcp::McpFailure>>,
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
        session_lock: Arc::new(Mutex::new(())),
        active_sessions: Arc::new(StdMutex::new(HashSet::new())),
        mcp_failures,
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/{file}", get(asset))
        .route("/api/health", get(health))
        .route("/api/models", get(models))
        .route("/api/local-models", get(local_models))
        .route("/api/local-models/select", post(select_local_model))
        .route("/api/model-catalog/search", get(model_catalog_search))
        .route("/api/model-catalog/files", get(model_catalog_files))
        .route("/api/model-catalog/install", post(model_catalog_install))
        .route("/api/model-catalog/status", get(model_catalog_status))
        .route("/api/model-catalog/cancel", post(model_catalog_cancel))
        .route("/api/mcp/status", get(mcp_status))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/export", get(export_sessions))
        .route("/api/sessions/{id}/export", get(export_session))
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route(
            "/api/sessions/{id}",
            get(get_session).patch(patch_session).delete(delete_session),
        )
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

async fn mcp_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let servers: Vec<Value> = state
        .config
        .mcp
        .iter()
        .map(|server| {
            json!({
                "name": server.name,
                "transport": server.transport_name(),
                "tools": server.tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "call_name": tool.call_name,
                    "description": tool.description,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let failures: Vec<Value> = state
        .mcp_failures
        .iter()
        .map(|failure| json!({ "name": failure.name, "error": failure.error }))
        .collect();
    Json(json!({
        "servers": servers,
        "failures": failures,
        "tool_count": state.config.mcp.iter().map(|server| server.tools.len()).sum::<usize>(),
    }))
    .into_response()
}

async fn modelctl(args: &[&str]) -> Result<String> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let helper = format!("{home}/.local/bin/local-harness-modelctl");
    let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
        let mut command = tokio::process::Command::new("flatpak-spawn");
        command.arg("--host").arg(&helper);
        command
    } else {
        tokio::process::Command::new(&helper)
    };
    command.args(args);
    let output = tokio::time::timeout(Duration::from_secs(180), command.output())
        .await
        .context("model helper timed out")?
        .context("running model helper")?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("{}", if stderr.is_empty() { stdout } else { stderr });
    }
    Ok(stdout)
}

async fn local_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    match modelctl(&["list"]).await {
        Ok(output) => match serde_json::from_str::<Value>(&output) {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("invalid model helper output: {error}") }))).into_response(),
        },
        Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

#[derive(Deserialize)]
struct ModelSelectBody {
    path: String,
}

async fn select_local_model(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<ModelSelectBody>,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let active = state
        .active_sessions
        .lock()
        .map(|active| !active.is_empty())
        .unwrap_or(true);
    if active {
        return (StatusCode::CONFLICT, Json(json!({ "error": "a chat is running; stop it before switching models" }))).into_response();
    }
    match modelctl(&["select", &body.path]).await {
        Ok(output) => match serde_json::from_str::<Value>(&output) {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("invalid model helper output: {error}") }))).into_response(),
        },
        Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

#[derive(Deserialize)]
struct CatalogQuery {
    query: Option<String>,
    repo: Option<String>,
}

#[derive(Deserialize)]
struct CatalogInstallBody {
    repo: String,
    file: String,
}

fn catalog_response(result: Result<String>) -> Response {
    match result {
        Ok(output) => match serde_json::from_str::<Value>(&output) {
            Ok(value) => Json(value).into_response(),
            Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("invalid model helper output: {error}") }))).into_response(),
        },
        Err(error) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

async fn model_catalog_search(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
    Query(catalog): Query<CatalogQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    let query = catalog.query.unwrap_or_default();
    if query.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "query is required" }))).into_response();
    }
    catalog_response(modelctl(&["search", query.trim()]).await)
}

async fn model_catalog_files(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
    Query(catalog): Query<CatalogQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    let repo = catalog.repo.unwrap_or_default();
    if repo.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "repo is required" }))).into_response();
    }
    catalog_response(modelctl(&["files", repo.trim()]).await)
}

async fn model_catalog_install(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<CatalogInstallBody>,
) -> Response {
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    catalog_response(modelctl(&["install", body.repo.trim(), body.file.trim()]).await)
}

async fn model_catalog_status(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    catalog_response(modelctl(&["download-status"]).await)
}

async fn model_catalog_cancel(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    catalog_response(modelctl(&["download-cancel"]).await)
}

async fn list_workspaces(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    match workspaces::list(&state.data_dir, &state.config.workspace).await {
        Ok(items) => Json(json!({ "workspaces": items })).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<WorkspaceBody>,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    match workspaces::create(
        &state.data_dir,
        &state.config.workspace,
        &body.name,
        &body.path,
    )
    .await
    {
        Ok(workspace) => Json(json!({ "workspace": workspace })).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

async fn export_text(data_dir: &std::path::Path, session: &Session) -> Result<String> {
    let index = session::SessionIndex::load(data_dir).await?;
    let summary = session.summary(data_dir, &index).await?;
    let messages = session.load_messages().await?;
    let mut out = format!(
        "=== {} ({}) ===\nModel: {}\nWorkspace: {} ({})\nCreated: {}\nUpdated: {}\n\n",
        summary.title,
        summary.id,
        summary.model,
        summary.workspace_name,
        summary.workspace,
        summary.created_at,
        summary.updated_at,
    );
    for message in messages {
        if message.role == "system" {
            continue;
        }
        let content = message.content.unwrap_or_default();
        match message.role.as_str() {
            "user" => out.push_str(&format!("--- user ---\n{content}\n\n")),
            "assistant" => {
                out.push_str(&format!("--- assistant ---\n{content}\n\n"));
                if let Some(calls) = message.tool_calls {
                    for call in calls {
                        out.push_str(&format!(
                            "[tool call] {} {}\n",
                            call.function.name, call.function.arguments
                        ));
                    }
                    out.push('\n');
                }
            }
            "tool" => out.push_str(&format!("--- tool result ---\n{content}\n\n")),
            other => out.push_str(&format!("--- {other} ---\n{content}\n\n")),
        }
    }
    Ok(out)
}

fn text_download(text: String, filename: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    let disposition = format!("attachment; filename=\"{filename}\"");
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    (headers, text).into_response()
}

async fn export_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let session = match Session::open(&state.data_dir, &id).await {
        Ok(session) => session,
        Err(error) => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    };
    match export_text(&state.data_dir, &session).await {
        Ok(text) => text_download(text, &format!("local-harness-{}.txt", session.id)),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

async fn export_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExportQuery>,
    headers: HeaderMap,
) -> Response {
    let ExportQuery { token, workspace, ids } = query;
    let auth = AuthQuery { token };
    if !authorized(&state, &auth, &headers) {
        return unauthorized();
    }
    let (sessions, _) = match session::list_sessions(&state.data_dir).await {
        Ok(sessions) => sessions,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    };
    let selected: Option<Vec<String>> = ids.map(|ids| {
        ids.split(',').map(str::trim).filter(|id| !id.is_empty()).map(str::to_string).collect()
    });
    let mut output = String::new();
    for summary in sessions {
        if let Some(workspace) = &workspace {
            if &summary.workspace != workspace {
                continue;
            }
        }
        if let Some(selected) = &selected {
            if !selected.iter().any(|id| id == &summary.id) {
                continue;
            }
        }
        let Ok(session) = Session::open(&state.data_dir, &summary.id).await else { continue };
        if let Ok(text) = export_text(&state.data_dir, &session).await {
            output.push_str(&text);
            output.push_str("\n\n");
        }
    }
    if output.is_empty() {
        output.push_str("No chats to export.\n");
    }
    text_download(output, "local-harness-chats.txt")
}

#[derive(Deserialize)]
struct WorkspaceBody {
    name: String,
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct ExportQuery {
    token: Option<String>,
    workspace: Option<String>,
    ids: Option<String>,
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let _guard = state.session_lock.lock().await;
    match session::list_sessions(&state.data_dir).await {
        Ok((sessions, _)) => Json(json!({ "sessions": sessions })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response(),
    }
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let session = match Session::open(&state.data_dir, &id).await {
        Ok(session) => session,
        Err(error) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let index = match session::SessionIndex::load(&state.data_dir).await {
        Ok(index) => index,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let summary = match session.summary(&state.data_dir, &index).await {
        Ok(summary) => summary,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let messages = match session.load_messages().await {
        Ok(messages) => messages,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    Json(json!({ "session": summary, "messages": messages })).into_response()
}

async fn patch_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<PatchSessionBody>,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let _guard = state.session_lock.lock().await;
    let session = match Session::open(&state.data_dir, &id).await {
        Ok(session) => session,
        Err(error) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let mut index = match session::SessionIndex::load(&state.data_dir).await {
        Ok(index) => index,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let mut meta = index.sessions.remove(&session.id).unwrap_or_default();
    if let Some(title) = body.title {
        let title: String = title.trim().chars().take(120).collect();
        meta.title = if title.is_empty() { None } else { Some(title) };
    }
    if let Some(pinned) = body.pinned {
        meta.pinned = pinned;
    }
    index.sessions.insert(session.id.clone(), meta);
    if let Err(error) = index.save(&state.data_dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response();
    }
    match session.summary(&state.data_dir, &index).await {
        Ok(summary) => Json(json!({ "session": summary })).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response(),
    }
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &query, &headers) {
        return unauthorized();
    }
    let session = match Session::open(&state.data_dir, &id).await {
        Ok(session) => session,
        Err(error) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    let active = state
        .active_sessions
        .lock()
        .map(|active| active.contains(&session.id))
        .unwrap_or(true);
    if active {
        return (StatusCode::CONFLICT, Json(json!({ "error": "session is running" }))).into_response();
    }
    let _guard = state.session_lock.lock().await;
    let mut index = match session::SessionIndex::load(&state.data_dir).await {
        Ok(index) => index,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    };
    if let Err(error) = session.trash(&state.data_dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response();
    }
    index.sessions.remove(&session.id);
    if let Err(error) = index.save(&state.data_dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{error:#}") }))).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct PatchSessionBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    pinned: Option<bool>,
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
        let (session, history, workspace_path) = match body.session.as_deref() {
            Some(id) => {
                let session = match Session::open(&task_state.data_dir, id).await {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                        return;
                    }
                };
                let history = match session.load_messages().await {
                    Ok(messages) => messages,
                    Err(error) => {
                        let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                        return;
                    }
                };
                let workspace = match session.workspace().await {
                    Ok(workspace) => workspace,
                    Err(error) => {
                        let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                        return;
                    }
                };
                (session, history, workspace.map(|workspace| workspace.path))
            }
            None => {
                let workspace = match body.workspace.as_deref() {
                    Some(id) => match workspaces::get(&task_state.data_dir, &task_state.config.workspace, id).await {
                        Ok(Some(workspace)) => workspace,
                        Ok(None) => {
                            let _ = tx.send(SseEvent::Error { message: format!("unknown workspace: {id}") });
                            return;
                        }
                        Err(error) => {
                            let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                            return;
                        }
                    },
                    None => workspaces::home_workspace(&task_state.config.workspace),
                };
                let session = match Session::create(&task_state.data_dir, &task_state.config.model).await {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                        return;
                    }
                };
                if let Err(error) = session
                    .append(json!({
                        "type": "workspace",
                        "id": workspace.id,
                        "name": workspace.name,
                        "path": workspace.path,
                    }))
                    .await
                {
                    let _ = tx.send(SseEvent::Error { message: format!("{error:#}") });
                    return;
                }
                (session, Vec::new(), Some(workspace.path))
            }
        };
        let _active = match ActiveSession::acquire(task_state.active_sessions.clone(), &session.id) {
            Some(active) => active,
            None => {
                let _ = tx.send(SseEvent::Error {
                    message: "this session is already running".to_string(),
                });
                return;
            }
        };
        let mut config = task_state.config.clone();
        if let Some(path) = workspace_path {
            if !std::path::Path::new(&path).is_dir() {
                let _ = tx.send(SseEvent::Error { message: format!("workspace is not a directory: {path}") });
                return;
            }
            config.workspace = PathBuf::from(path);
        }
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
        match agent::run(config, &session, body.prompt.as_str(), history, &mut sink).await {
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
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct ApprovalBody {
    id: String,
    decision: bool,
}

enum SseEvent {
    Session(String),
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
        SseEvent::Session(session) => Event::default()
            .event("session")
            .data(json!({ "session": session }).to_string()),
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

struct ActiveSession {
    id: String,
    set: Arc<StdMutex<HashSet<String>>>,
}

impl ActiveSession {
    fn acquire(set: Arc<StdMutex<HashSet<String>>>, id: &str) -> Option<Self> {
        let inserted = {
            let mut active = set.lock().ok()?;
            active.insert(id.to_string())
        };
        if !inserted {
            return None;
        }
        Some(Self { id: id.to_string(), set })
    }
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        if let Ok(mut active) = self.set.lock() {
            active.remove(&self.id);
        }
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
    fn session(&mut self, id: &str) {
        let _ = self.tx.send(SseEvent::Session(id.to_string()));
    }

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
