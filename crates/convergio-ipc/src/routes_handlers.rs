//! IPC route handlers — agent management, messaging, and status.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::sse::{self, Sse};
use axum::response::Json;
use serde::Deserialize;

use crate::sse::create_sse_stream;

use super::IpcState;

// ── Input validation limits ──────────────────────
const MAX_AGENT_NAME: usize = 128;
const MAX_CONTENT: usize = 65_536;
const MAX_KEY: usize = 256;
const MAX_VALUE: usize = 65_536;
const MAX_MSG_TYPE: usize = 64;
const MAX_METADATA: usize = 4_096;

fn validate_len(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{field} exceeds max length ({max})"));
    }
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(())
}

pub async fn handle_status(State(state): State<Arc<IpcState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({"error": e.to_string()})),
    };
    let agents: u64 = conn
        .query_row("SELECT count(*) FROM ipc_agents", [], |r| r.get(0))
        .unwrap_or(0);
    let messages: u64 = conn
        .query_row("SELECT count(*) FROM ipc_messages", [], |r| r.get(0))
        .unwrap_or(0);
    let channels: u64 = conn
        .query_row("SELECT count(*) FROM ipc_channels", [], |r| r.get(0))
        .unwrap_or(0);
    Json(serde_json::json!({
        "agents": agents,
        "messages": messages,
        "channels": channels,
    }))
}

pub async fn handle_agents(State(state): State<Arc<IpcState>>) -> Json<serde_json::Value> {
    match crate::agents::list(&state.pool) {
        Ok(agents) => Json(serde_json::json!(agents)),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

pub async fn handle_channels(State(state): State<Arc<IpcState>>) -> Json<serde_json::Value> {
    match crate::channels::list_channels(&state.pool) {
        Ok(ch) => Json(serde_json::json!(ch)),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

pub async fn handle_context(State(state): State<Arc<IpcState>>) -> Json<serde_json::Value> {
    match crate::channels::context_list(&state.pool) {
        Ok(ctx) => Json(serde_json::json!(ctx)),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub agent: Option<String>,
    pub channel: Option<String>,
    pub limit: Option<u32>,
}

pub async fn handle_messages(
    State(state): State<Arc<IpcState>>,
    Query(params): Query<MessagesQuery>,
) -> Json<serde_json::Value> {
    let limit = params.limit.unwrap_or(50).min(200);
    match crate::messaging::history(
        &state.pool,
        params.agent.as_deref(),
        params.channel.as_deref(),
        limit,
        None,
    ) {
        Ok(msgs) => Json(serde_json::json!(msgs)),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    pub agent: Option<String>,
}

pub async fn handle_stream(
    State(state): State<Arc<IpcState>>,
    Query(params): Query<StreamQuery>,
) -> Sse<impl futures_core::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    Sse::new(create_sse_stream(
        Arc::clone(&state.event_bus),
        params.agent,
    ))
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub from: String,
    pub to: String,
    pub content: String,
    #[serde(default = "default_msg_type")]
    pub msg_type: String,
    #[serde(default)]
    pub priority: i32,
}

fn default_msg_type() -> String {
    "text".into()
}

pub async fn handle_send(
    State(state): State<Arc<IpcState>>,
    Json(body): Json<SendRequest>,
) -> Json<serde_json::Value> {
    if let Err(e) = validate_len("from", &body.from, MAX_AGENT_NAME) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Err(e) = validate_len("to", &body.to, MAX_AGENT_NAME) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Err(e) = validate_len("content", &body.content, MAX_CONTENT) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Err(e) = validate_len("msg_type", &body.msg_type, MAX_MSG_TYPE) {
        return Json(serde_json::json!({"error": e}));
    }
    let params = crate::messaging::SendParams {
        from: &body.from,
        to: &body.to,
        content: &body.content,
        msg_type: &body.msg_type,
        priority: body.priority,
        rate_limit: state.rate_limit,
    };
    match crate::messaging::send(&state.pool, &state.notify, &params) {
        Ok(id) => Json(serde_json::json!({"id": id})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Wake up all long-poll receivers — called by mesh sync when ipc_messages arrive.
/// POST /api/ipc/notify-sync
pub async fn handle_notify_sync(State(state): State<Arc<IpcState>>) -> Json<serde_json::Value> {
    state.notify.notify_waiters();
    Json(serde_json::json!({"ok": true}))
}

/// Long-poll: blocks until a message arrives for this agent (or timeout).
/// GET /api/ipc/receive?agent=opus-arch&from=opus-monitor&timeout=60
pub async fn handle_receive_wait(
    State(state): State<Arc<IpcState>>,
    Query(q): Query<ReceiveWaitQuery>,
) -> Json<serde_json::Value> {
    let agent = match q.agent {
        Some(a) if !a.is_empty() => a,
        _ => return Json(serde_json::json!({"error": "agent query param required"})),
    };
    let timeout = q.timeout.unwrap_or(30).min(300);
    let limit = q.limit.unwrap_or(10).min(50);
    match crate::messaging::receive_wait(
        &state.pool,
        &state.notify,
        &agent,
        q.from.as_deref(),
        q.channel.as_deref(),
        limit,
        timeout,
    )
    .await
    {
        Ok(msgs) => Json(serde_json::json!({"messages": msgs})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReceiveWaitQuery {
    pub agent: Option<String>,
    pub from: Option<String>,
    pub channel: Option<String>,
    pub timeout: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterAgentRequest {
    pub name: String,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    pub pid: Option<u32>,
    pub host: Option<String>,
    pub metadata: Option<String>,
    pub parent_agent: Option<String>,
}

fn default_agent_type() -> String {
    "claude".into()
}

#[derive(Debug, Deserialize)]
pub struct ContextSetRequest {
    pub key: String,
    pub value: String,
    #[serde(default = "default_context_set_by")]
    pub set_by: String,
}

fn default_context_set_by() -> String {
    "api".into()
}

pub async fn handle_context_set(
    State(state): State<Arc<IpcState>>,
    Json(body): Json<ContextSetRequest>,
) -> Json<serde_json::Value> {
    if let Err(e) = validate_len("key", &body.key, MAX_KEY) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Err(e) = validate_len("value", &body.value, MAX_VALUE) {
        return Json(serde_json::json!({"error": e}));
    }
    match crate::channels::context_set(&state.pool, &body.key, &body.value, &body.set_by) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

pub async fn handle_register_agent(
    State(state): State<Arc<IpcState>>,
    Json(body): Json<RegisterAgentRequest>,
) -> Json<serde_json::Value> {
    if let Err(e) = validate_len("name", &body.name, MAX_AGENT_NAME) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Err(e) = validate_len("agent_type", &body.agent_type, MAX_MSG_TYPE) {
        return Json(serde_json::json!({"error": e}));
    }
    if let Some(ref m) = body.metadata {
        if let Err(e) = validate_len("metadata", m, MAX_METADATA) {
            return Json(serde_json::json!({"error": e}));
        }
    }
    let host = body.host.unwrap_or_else(super::local_hostname);
    match crate::agents::register(
        &state.pool,
        &body.name,
        &body.agent_type,
        body.pid,
        &host,
        body.metadata.as_deref(),
        body.parent_agent.as_deref(),
    ) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

pub async fn handle_unregister_agent(
    State(state): State<Arc<IpcState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let host = super::local_hostname();
    match crate::agents::unregister(&state.pool, &name, &host) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatAgentRequest {
    pub name: String,
    pub host: Option<String>,
}

pub async fn handle_agent_heartbeat(
    State(state): State<Arc<IpcState>>,
    Json(body): Json<HeartbeatAgentRequest>,
) -> Json<serde_json::Value> {
    let host = body.host.unwrap_or_else(super::local_hostname);
    match crate::agents::heartbeat(&state.pool, &body.name, &host) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}
