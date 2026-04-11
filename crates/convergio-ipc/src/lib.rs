//! convergio-ipc — Message bus, SSE event streaming, agent registry.
//!
//! Implements Extension: owns ipc_agents, ipc_messages, ipc_subscriptions, etc.

pub mod agents;
pub mod budget;
pub mod channels;
pub mod ext;
pub mod locks;
pub mod mcp_defs;
pub mod messaging;
pub mod models;
pub mod process_scanner;
pub mod routes;
pub mod schema;
pub mod skills;
pub mod sse;
pub mod types;
pub mod websocket;

pub use ext::IpcExtension;
pub use types::{IpcError, IpcResult};
