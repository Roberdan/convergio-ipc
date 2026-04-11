//! WebSocket handler for real-time agent chat and event streaming.
//!
//! Protocol:
//! - Client sends: `{"type":"subscribe","agent_id":"..."}` to watch an agent
//! - Client sends: `{"type":"message","agent_id":"...","content":"..."}` to send
//! - Server sends: `{"type":"agent_event","agent_id":"...","event":{...}}`
//! - Server sends: `{"type":"agent_message","agent_id":"...","content":"..."}`
//! - Heartbeat ping/pong every 30s

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::broadcast;

use crate::sse::{EventBus, IpcEvent};

#[derive(serde::Deserialize)]
struct WsClientMsg {
    #[serde(rename = "type")]
    msg_type: String,
    agent_id: Option<String>,
    content: Option<String>,
}

#[derive(serde::Serialize)]
struct WsServerMsg {
    #[serde(rename = "type")]
    msg_type: String,
    agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

/// Handle an upgraded WebSocket connection.
pub async fn handle_ws(mut socket: WebSocket, bus: Arc<EventBus>) {
    let mut rx = bus.subscribe();
    let mut agent_filter: Option<String> = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            // Incoming message from client
            maybe_msg = socket.recv() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<WsClientMsg>(&text) {
                            match msg.msg_type.as_str() {
                                "subscribe" => {
                                    agent_filter = msg.agent_id;
                                }
                                "message" => {
                                    if let (Some(aid), Some(content)) = (msg.agent_id, msg.content) {
                                        bus.publish(IpcEvent {
                                            from: "ws-client".into(),
                                            to: Some(aid),
                                            content,
                                            event_type: "direct".into(),
                                            ts: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // Outgoing events from EventBus
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Some(ref filter) = agent_filter {
                            let matches = event.from == *filter
                                || event.to.as_deref() == Some(filter);
                            if !matches { continue; }
                        }
                        let server_msg = WsServerMsg {
                            msg_type: "agent_event".into(),
                            agent_id: event.from.clone(),
                            event: serde_json::from_str(&event.content).ok(),
                            content: Some(event.content),
                        };
                        let json = serde_json::to_string(&server_msg).unwrap_or_default();
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Heartbeat
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_client_msg_deserialize() {
        let json = r#"{"type":"subscribe","agent_id":"copilot-1"}"#;
        let msg: WsClientMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.msg_type, "subscribe");
        assert_eq!(msg.agent_id.as_deref(), Some("copilot-1"));
    }

    #[test]
    fn ws_server_msg_serialize() {
        let msg = WsServerMsg {
            msg_type: "agent_event".into(),
            agent_id: "copilot-1".into(),
            event: None,
            content: Some("hello".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("agent_event"));
        assert!(json.contains("copilot-1"));
    }
}
