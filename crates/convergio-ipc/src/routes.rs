//! HTTP API routes for IPC.
//!
//! - GET  /api/ipc/status   — IPC stats (agents, messages, channels)
//! - GET  /api/ipc/agents   — registered agents list
//! - GET  /api/ipc/channels — channel list
//! - GET  /api/ipc/context  — shared context entries
//! - POST /api/ipc/context  — set a shared context key/value
//! - GET  /api/ipc/messages — message history (query: agent, channel, limit)
//! - GET  /api/ipc/stream   — SSE event stream (query: agent filter)
//! - GET  /api/events/stream — SSE event stream (legacy path)
//! - POST /api/ipc/send     — send a direct message

#[path = "routes_handlers.rs"]
mod routes_handlers;

use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Query, State};
use axum::response::sse::{self, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::Notify;

use convergio_db::pool::ConnPool;

use crate::sse::{create_sse_stream, EventBus};
use crate::websocket;

pub struct IpcState {
    pub pool: ConnPool,
    pub notify: Arc<Notify>,
    pub event_bus: Arc<EventBus>,
    pub rate_limit: u32,
}

pub fn ipc_routes(state: Arc<IpcState>) -> Router {
    let bus = Arc::clone(&state.event_bus);
    Router::new()
        .route("/api/ipc/status", get(routes_handlers::handle_status))
        .route("/api/ipc/agents", get(routes_handlers::handle_agents))
        .route("/api/ipc/channels", get(routes_handlers::handle_channels))
        .route(
            "/api/ipc/context",
            get(routes_handlers::handle_context).post(routes_handlers::handle_context_set),
        )
        .route("/api/ipc/messages", get(routes_handlers::handle_messages))
        .route("/api/ipc/stream", get(routes_handlers::handle_stream))
        .route("/api/ws/agent-chat", get(ws_agent_chat_handler))
        .route("/api/ipc/send", post(routes_handlers::handle_send))
        .route(
            "/api/ipc/agents/register",
            post(routes_handlers::handle_register_agent),
        )
        .route(
            "/api/ipc/agents/heartbeat",
            post(routes_handlers::handle_agent_heartbeat),
        )
        .route(
            "/api/ipc/agents/:name",
            axum::routing::delete(routes_handlers::handle_unregister_agent),
        )
        .with_state(state)
        .merge(event_routes(bus))
}

/// Legacy SSE stream at /api/events/stream (from main branch).
pub fn event_routes(bus: Arc<EventBus>) -> Router {
    Router::new()
        .route("/api/events/stream", get(legacy_stream_handler))
        .with_state(bus)
}

#[derive(serde::Deserialize, Default)]
pub struct LegacyStreamQuery {
    agent_filter: Option<String>,
}

async fn legacy_stream_handler(
    State(bus): State<Arc<EventBus>>,
    Query(q): Query<LegacyStreamQuery>,
) -> Sse<impl futures_core::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    Sse::new(create_sse_stream(bus, q.agent_filter)).keep_alive(sse::KeepAlive::default())
}

/// WebSocket upgrade handler for agent chat.
async fn ws_agent_chat_handler(
    State(state): State<Arc<IpcState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let bus = Arc::clone(&state.event_bus);
    ws.on_upgrade(move |socket| websocket::handle_ws(socket, bus))
}

fn local_hostname() -> String {
    let mut buf = [0u8; 256];
    #[cfg(unix)]
    unsafe {
        libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}
