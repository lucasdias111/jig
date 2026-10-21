//! The Map's tools as agents get them: Jig's own executable, started with
//! [`FLAG`], is an MCP server beside the task tool, whose tools draw on
//! the Map the user is looking at, and read the project and answer for
//! the Map's actions on domains ([`PROJECT`], [`PROPOSE_DOMAINS`]). The server lives in the
//! app, which has the project's index to check the places named; here is
//! how agents are handed it, already allowed (the tools change no code,
//! and an agent run without a prompt refuses a tool it must ask about),
//! and how a call to one is told apart in what an agent reports.

use serde_json::{Value, json};

use crate::tasks;

/// Run Jig as the Map's tools: `Jig --map-mcp [--project <folder>]`.
pub const FLAG: &str = "--map-mcp";

/// The server's name; OpenCode calls its tools `jig-map_map_trace` and so
/// on, Claude Code `mcp__jig-map__map_trace`.
pub const NAME: &str = "jig-map";

/// The tool that hands over Jig's outline of the project.
pub const PROJECT: &str = "map_project";

/// The tool a Map action's answer comes in by.
pub const PROPOSE_DOMAINS: &str = "map_propose_domains";

/// The tools by name.
pub const TOOLS: [&str; 10] = [
    "map_highlight",
    "map_note",
    "map_trace",
    "map_tour",
    "map_group",
    "map_propose",
    "map_describe",
    "map_clear",
    PROJECT,
    PROPOSE_DOMAINS,
];

/// The tool as OpenCode's config takes it, under `mcp`.
pub fn opencode_server() -> Option<Value> {
    let command = tasks::mcp::command()?;
    Some(json!({
        "type": "local",
        "command": [command, FLAG],
        "enabled": true,
    }))
}

/// The server as an ACP conversation in `directory` takes it.
pub fn acp_server(directory: &str) -> Option<Value> {
    let command = tasks::mcp::command()?;
    Some(json!({
        "name": NAME,
        "command": command,
        "args": [FLAG, "--project", directory],
        "env": [],
    }))
}

/// Every server Jig hands an ACP conversation in `directory`: the tasks
/// and the Map.
pub fn acp_servers(directory: &str) -> Value {
    let mut servers = tasks::mcp::acp_servers(directory);
    if let (Some(list), Some(map)) = (servers.as_array_mut(), acp_server(directory)) {
        list.push(map);
    }
    servers
}

/// The permissions OpenCode checks for the Map's tools.
pub fn opencode_permissions() -> impl Iterator<Item = String> {
    TOOLS.iter().map(|tool| format!("{NAME}_{tool}"))
}

/// Which of the Map's tools an agent's name for a call is, if it's one:
/// OpenCode's `jig-map_map_trace`, Claude Code's `mcp__jig-map__map_trace`,
/// Gemini's `map_trace (jig-map MCP Server)`.
pub fn tool_of(name: &str) -> Option<&'static str> {
    let name = name.to_lowercase();
    if !name.contains(NAME) {
        return None;
    }
    TOOLS.iter().copied().find(|tool| {
        name.match_indices(tool).any(|(at, _)| {
            let after = name[at + tool.len()..].chars().next();
            !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_its_tools_by_each_agents_name_for_them() {
        assert_eq!(tool_of("jig-map_map_trace"), Some("map_trace"));
        assert_eq!(
            tool_of("mcp__jig-map__map_highlight"),
            Some("map_highlight")
        );
        assert_eq!(tool_of("map_tour (jig-map MCP Server)"), Some("map_tour"));
        assert_eq!(tool_of("map_trace"), None, "another server's");
        assert_eq!(tool_of("mcp__jig__create_task"), None);
        assert_eq!(tool_of("mcp__jig-map__map_tracer"), None);
        assert_eq!(
            tool_of("jig-map_map_propose_domains"),
            Some("map_propose_domains")
        );
        assert_eq!(tool_of("mcp__jig-map__map_propose"), Some("map_propose"));
        assert_eq!(tool_of("mcp__jig-map__map_project"), Some("map_project"));
    }
}
