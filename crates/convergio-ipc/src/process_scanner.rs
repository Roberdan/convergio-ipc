//! Process scanner — discovers external AI agents (Claude Code, Copilot, MLX)
//! by inspecting OS processes and registers them in ipc_agents.

use convergio_db::pool::ConnPool;
use std::time::Duration;

/// Agent type tags for externally discovered processes.
const TYPE_CLAUDE: &str = "external-claude";
const TYPE_COPILOT: &str = "external-copilot";
const TYPE_MLX: &str = "external-mlx";

/// Scan interval: 15 seconds.
const SCAN_INTERVAL: Duration = Duration::from_secs(15);

/// Max agent age before pruning (5 minutes without refresh).
const STALE_TTL_SECS: u64 = 300;

struct FoundAgent {
    name: String,
    agent_type: &'static str,
    pid: u32,
    raw_cmd: String,
    metadata: String,
}

/// Spawn a background task that periodically scans for AI agent processes.
pub fn spawn_process_scanner(pool: ConnPool) {
    tokio::spawn(async move {
        tracing::info!("process scanner started");
        loop {
            tokio::time::sleep(SCAN_INTERVAL).await;
            let p = pool.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || scan_round(&p)).await {
                tracing::error!("process scanner panicked: {e}");
            }
        }
    });
}

fn scan_round(pool: &ConnPool) {
    let host = hostname();
    let agents = discover_agents();
    for agent in &agents {
        let meta = Some(agent.metadata.as_str());
        if let Err(e) = crate::agents::register(
            pool,
            &agent.name,
            agent.agent_type,
            Some(agent.pid),
            &host,
            meta,
            None,
        ) {
            tracing::warn!(name = %agent.name, "scanner register failed: {e}");
        }
    }
    // Prune agents whose PIDs are gone
    let _ = crate::agents::prune_dead_pids(pool, &host);
    // Prune agents not seen for STALE_TTL_SECS
    let _ = crate::agents::prune_stale(pool, STALE_TTL_SECS);
}

fn discover_agents() -> Vec<FoundAgent> {
    let output = match std::process::Command::new("ps").args(["aux"]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return vec![],
    };
    let mut agents = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 11 {
            continue;
        }
        let pid: u32 = match parts[1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let cmd = parts[10..].join(" ");
        if let Some(agent) = classify_process(pid, &cmd) {
            agents.push(agent);
        }
    }
    dedup_by_command(&mut agents);
    agents
}

/// Deduplicate agents with identical commands — keep only the highest PID
/// (the leaf child process). Prevents registering both parent wrapper and
/// child process for the same Copilot/Claude session.
fn dedup_by_command(agents: &mut Vec<FoundAgent>) {
    agents.sort_by(|a, b| {
        a.agent_type
            .cmp(b.agent_type)
            .then(a.raw_cmd.cmp(&b.raw_cmd))
            .then(b.pid.cmp(&a.pid))
    });
    agents.dedup_by(|a, b| a.agent_type == b.agent_type && a.raw_cmd == b.raw_cmd);
}

fn classify_process(pid: u32, cmd: &str) -> Option<FoundAgent> {
    // Skip our own grep/ps processes
    if cmd.contains("grep") || cmd.contains("process_scanner") {
        return None;
    }
    // Claude Code sessions
    if is_claude_process(cmd) {
        let flags = extract_flags(cmd);
        return Some(FoundAgent {
            name: format!("claude-{pid}"),
            agent_type: TYPE_CLAUDE,
            pid,
            raw_cmd: cmd.to_string(),
            metadata: serde_json::json!({"cmd": truncate(cmd, 200), "flags": flags}).to_string(),
        });
    }
    // GitHub Copilot — actual binary only, not `gh` wrapper or unrelated apps
    if is_copilot_process(cmd) {
        return Some(FoundAgent {
            name: format!("copilot-{pid}"),
            agent_type: TYPE_COPILOT,
            pid,
            raw_cmd: cmd.to_string(),
            metadata: serde_json::json!({"cmd": truncate(cmd, 200)}).to_string(),
        });
    }
    // MLX / local model servers
    if cmd.contains("mlx_lm") || cmd.contains("omlx serve") || cmd.contains("mlx-server") {
        let model = extract_model_name(cmd);
        return Some(FoundAgent {
            name: format!("mlx-{pid}"),
            agent_type: TYPE_MLX,
            pid,
            raw_cmd: cmd.to_string(),
            metadata: serde_json::json!({"cmd": truncate(cmd, 200), "model": model}).to_string(),
        });
    }
    None
}

/// Match actual Copilot agent processes, filtering out `gh` CLI wrappers
/// and unrelated apps (Teams, Edge) that contain "copilot" in URLs.
fn is_copilot_process(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    if !lower.contains("copilot") {
        return false;
    }
    // `gh copilot ...` is the parent CLI wrapper — skip it
    if lower.starts_with("gh ") {
        return false;
    }
    // Non-AI apps that coincidentally contain "copilot" in command/URLs
    if lower.contains(".app/")
        || lower.contains("microsoft")
        || lower.contains("webview")
        || lower.contains("teams")
    {
        return false;
    }
    // Detached dev servers are not agent sessions
    if lower.contains("copilot-detached-dev-server") {
        return false;
    }
    true
}

fn is_claude_process(cmd: &str) -> bool {
    // Match: `claude` binary or `node ... claude`
    // Exclude: `claude-daemon`, cargo/build references
    let lower = cmd.to_lowercase();
    (lower.starts_with("claude ") || lower.contains("/claude ") || lower.contains("/claude\0"))
        && !lower.contains("cargo")
        && !lower.contains("target/")
}

fn extract_flags(cmd: &str) -> String {
    cmd.split_whitespace()
        .filter(|w| w.starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_model_name(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == "--model-dir" || *part == "--model" {
            if let Some(path) = parts.get(i + 1) {
                return path.rsplit('/').next().unwrap_or(path).to_string();
            }
        }
    }
    "unknown".to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // SECURITY: find char boundary to avoid panic on multi-byte UTF-8
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i < max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &s[..boundary])
    }
}

fn hostname() -> String {
    crate::utils::hostname()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_claude() {
        let a = classify_process(1234, "claude --dangerously-skip-permissions");
        assert!(a.is_some());
        let a = a.unwrap();
        assert_eq!(a.agent_type, TYPE_CLAUDE);
        assert_eq!(a.name, "claude-1234");
    }

    #[test]
    fn classify_mlx() {
        let cmd = "/usr/bin/python omlx serve --model-dir /path/to/qwen --port 8321";
        let a = classify_process(5678, cmd).unwrap();
        assert_eq!(a.agent_type, TYPE_MLX);
        assert!(a.metadata.contains("qwen"));
    }

    #[test]
    fn classify_copilot() {
        let a = classify_process(99, "/Users/u/.local/share/gh/copilot/copilot --yolo");
        assert!(a.is_some());
        assert_eq!(a.unwrap().agent_type, TYPE_COPILOT);
    }

    #[test]
    fn classify_copilot_agent_stdio() {
        let a = classify_process(99, "copilot-agent --stdio");
        assert!(a.is_some());
        assert_eq!(a.unwrap().agent_type, TYPE_COPILOT);
    }

    #[test]
    fn classify_ignores_gh_wrapper() {
        assert!(classify_process(99, "gh copilot --yolo").is_none());
    }

    #[test]
    fn classify_ignores_teams_copilot() {
        let teams = "/Applications/Microsoft Teams.app/Contents/MacOS/Teams --copilot.teams.cloud";
        assert!(classify_process(99, teams).is_none());
    }

    #[test]
    fn dedup_keeps_highest_pid() {
        let cmd = "/Users/u/.local/share/gh/copilot/copilot --yolo";
        let mut agents = vec![
            classify_process(100, cmd).unwrap(),
            classify_process(101, cmd).unwrap(),
        ];
        dedup_by_command(&mut agents);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pid, 101);
    }

    #[test]
    fn classify_ignores_grep() {
        assert!(classify_process(1, "grep claude").is_none());
    }

    #[test]
    fn classify_ignores_cargo() {
        assert!(classify_process(1, "cargo test claude").is_none());
    }

    #[test]
    fn extract_model_from_cmd() {
        let m = extract_model_name("omlx serve --model-dir /models/qwen3.5-27b --port 8321");
        assert_eq!(m, "qwen3.5-27b");
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        let s = "hello 🌍 world";
        let t = truncate(s, 8);
        // Should not panic; cuts before the 4-byte emoji at byte index 6
        assert!(t.ends_with("..."));
        assert!(t.len() <= 14);
    }
}
