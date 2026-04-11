//! MCP tool definitions for the IPC extension.

use convergio_types::extension::McpToolDef;
use serde_json::json;

pub fn ipc_tools() -> Vec<McpToolDef> {
    vec![McpToolDef {
        name: "cvg_list_active_agents".into(),
        description: "List agents currently registered and active on this node.".into(),
        method: "GET".into(),
        path: "/api/ipc/agents".into(),
        input_schema: json!({"type": "object", "properties": {}}),
        min_ring: "sandboxed".into(),
        path_params: vec![],
    }]
}
