//! Per-channel tool routing.
//!
//! Loads `~/.ironclaw/channel-routing.json` and filters which tools
//! (MCP and built-in) the LLM can see based on the originating channel.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::llm::ToolDefinition;

/// Channel-to-tool-group routing configuration.
///
/// Loaded from `channel-routing.json` in the IronClaw base directory.
/// When present, each incoming message's channel name is mapped to a group,
/// and only tools belonging to that group's allowed MCP servers (plus any
/// whitelisted built-in tools) are shown to the LLM.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelRoutingConfig {
    /// MCP server allowlist per group. Key = group name, value = server names.
    /// A tool named `ServerName_tool` belongs to server `ServerName`.
    pub groups: HashMap<String, Vec<String>>,

    /// Built-in tool allowlist per group. If a group is absent from this map,
    /// all built-in tools are available. If present, only listed tools are kept.
    #[serde(default)]
    pub builtin_whitelist: HashMap<String, Vec<String>>,

    /// Channel name → group name mapping.
    pub channels: HashMap<String, String>,

    /// Fallback group for channels not listed in `channels`.
    pub default_group: String,
}

/// Prefixes that identify direct messages (bypass routing entirely).
const DM_PREFIXES: &[&str] = &["slack-dm", "telegram-dm", "cli", "repl", "web"];

impl ChannelRoutingConfig {
    /// Load from `<base_dir>/channel-routing.json`. Returns `None` if the file
    /// doesn't exist or can't be parsed (logged as warning).
    pub fn load(base_dir: &Path) -> Option<Self> {
        let path = base_dir.join("channel-routing.json");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!("Failed to read {}: {}", path.display(), e);
                return None;
            }
        };
        match serde_json::from_str(&content) {
            Ok(config) => {
                let config: Self = config;
                tracing::info!(
                    groups = ?config.groups.keys().collect::<Vec<_>>(),
                    channels = config.channels.len(),
                    "Loaded channel routing config"
                );
                Some(config)
            }
            Err(e) => {
                tracing::warn!("Failed to parse {}: {}", path.display(), e);
                None
            }
        }
    }

    /// Resolve which group a channel belongs to.
    pub fn resolve_group(&self, channel: &str) -> &str {
        self.channels
            .get(channel)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_group)
    }

    /// Whether this channel name represents a direct message (no filtering).
    pub fn is_dm(channel: &str) -> bool {
        DM_PREFIXES.iter().any(|p| channel.starts_with(p))
    }

    /// Filter tool definitions based on channel routing rules.
    ///
    /// MCP tools are identified by having an underscore-separated server prefix
    /// (e.g. `Notion_post_search` → server `Notion`). Tools without a known
    /// server prefix are treated as built-in tools.
    pub fn filter_tool_defs(
        &self,
        channel: &str,
        tools: Vec<ToolDefinition>,
    ) -> Vec<ToolDefinition> {
        if Self::is_dm(channel) {
            return tools;
        }

        let group = self.resolve_group(channel);

        let allowed_servers = match self.groups.get(group) {
            Some(servers) => servers,
            None => {
                tracing::warn!(
                    group,
                    "Channel routing group not found — blocking all MCP tools (restrictive default)"
                );
                // Restrictive: only pass through built-in tools when group is misconfigured.
                return tools
                    .into_iter()
                    .filter(|t| self.extract_mcp_server(&t.name).is_none())
                    .collect();
            }
        };

        let builtin_whitelist = self.builtin_whitelist.get(group);

        tools
            .into_iter()
            .filter(|tool| {
                if let Some(server) = self.extract_mcp_server(&tool.name) {
                    allowed_servers.iter().any(|s| s == server)
                } else {
                    match builtin_whitelist {
                        Some(whitelist) => whitelist.iter().any(|w| w == &tool.name),
                        None => true,
                    }
                }
            })
            .collect()
    }

    /// Try to extract the MCP server name from a tool name.
    ///
    /// MCP tools are named `ServerName_tool_name`. We check if the prefix
    /// before the first `_` matches any known server name across all groups.
    fn extract_mcp_server<'a>(&self, tool_name: &'a str) -> Option<&'a str> {
        for server in self.groups.values().flatten() {
            let prefix = format!("{}_", server);
            if tool_name.starts_with(&prefix) {
                return Some(&tool_name[..server.len()]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> ChannelRoutingConfig {
        let json = r#"{
            "groups": {
                "minimal": ["Archon"],
                "dev": ["Archon", "Kiro", "Notion"]
            },
            "builtin_whitelist": {
                "minimal": ["memory_search", "create_job"]
            },
            "channels": {
                "agentiffai-dev-issues": "dev"
            },
            "default_group": "minimal"
        }"#;
        serde_json::from_str(json).unwrap()
    }

    fn make_tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }
    }

    #[test]
    fn test_deserialize_config() {
        let config = sample_config();
        assert_eq!(config.groups.len(), 2);
        assert_eq!(config.default_group, "minimal");
        assert_eq!(config.channels["agentiffai-dev-issues"], "dev");
    }

    #[test]
    fn test_resolve_group_mapped_channel() {
        let config = sample_config();
        assert_eq!(config.resolve_group("agentiffai-dev-issues"), "dev");
    }

    #[test]
    fn test_resolve_group_unmapped_falls_to_default() {
        let config = sample_config();
        assert_eq!(config.resolve_group("random-channel"), "minimal");
    }

    #[test]
    fn test_is_dm() {
        assert!(ChannelRoutingConfig::is_dm("slack-dm"));
        assert!(ChannelRoutingConfig::is_dm("telegram-dm"));
        assert!(ChannelRoutingConfig::is_dm("cli"));
        assert!(ChannelRoutingConfig::is_dm("repl"));
        assert!(ChannelRoutingConfig::is_dm("web"));
        assert!(!ChannelRoutingConfig::is_dm("agentiffai-dev-issues"));
    }

    #[test]
    fn test_filter_keeps_allowed_mcp_tools() {
        // Add Smartlead to a different group so it's recognized as MCP
        let json = r#"{
            "groups": {
                "minimal": ["Archon"],
                "dev": ["Archon", "Kiro", "Notion"],
                "leads": ["Archon", "Smartlead"]
            },
            "builtin_whitelist": {
                "minimal": ["memory_search", "create_job"]
            },
            "channels": {
                "agentiffai-dev-issues": "dev"
            },
            "default_group": "minimal"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test-only helper
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Kiro_run_task"),
            make_tool_def("Notion_post_search"),
            make_tool_def("Smartlead_send"),
        ];
        let filtered = config.filter_tool_defs("agentiffai-dev-issues", tools);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Archon_list_tasks")); // safety: test assertion
        assert!(names.contains(&"Kiro_run_task")); // safety: test assertion
        assert!(names.contains(&"Notion_post_search")); // safety: test assertion
        assert!(!names.contains(&"Smartlead_send")); // safety: test assertion
    }

    #[test]
    fn test_filter_restricts_builtins_when_whitelisted() {
        let config = sample_config();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("memory_search"),
            make_tool_def("create_job"),
            make_tool_def("shell"),
            make_tool_def("http_request"),
        ];
        let filtered = config.filter_tool_defs("unmapped-channel", tools);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Archon_list_tasks"));
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"create_job"));
        assert!(!names.contains(&"shell"));
        assert!(!names.contains(&"http_request"));
    }

    #[test]
    fn test_filter_allows_all_builtins_when_no_whitelist() {
        let config = sample_config();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("shell"),
            make_tool_def("memory_search"),
        ];
        let filtered = config.filter_tool_defs("agentiffai-dev-issues", tools);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_filter_dm_returns_all_tools() {
        let config = sample_config();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Smartlead_send"),
            make_tool_def("shell"),
        ];
        let filtered = config.filter_tool_defs("slack-dm", tools);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_load_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_load_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "channels": {},
                "default_group": "minimal"
            }"#,
        )
        .unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_some());
        assert_eq!(config.unwrap().default_group, "minimal");
    }

    #[test]
    fn test_load_returns_none_for_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-routing.json");
        std::fs::write(&path, "not json").unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_full_routing_scenario() {
        let json = r#"{
            "groups": {
                "minimal": ["Archon"],
                "content": ["Archon", "Notion", "Kit"],
                "dev": ["Archon", "Kiro", "Notion"]
            },
            "builtin_whitelist": {
                "content": ["memory_search", "memory_write", "create_job"]
            },
            "channels": {
                "agentiffai-marketing": "content",
                "agentiffai-dev-issues": "dev"
            },
            "default_group": "minimal"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();

        // Content channel: only Archon+Notion+Kit MCP tools + whitelisted builtins
        let all_tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Notion_post_search"),
            make_tool_def("Kit_list_subscribers"),
            make_tool_def("Kiro_run_task"),
            make_tool_def("memory_search"),
            make_tool_def("shell"),
            make_tool_def("create_job"),
        ];

        let content_tools = config.filter_tool_defs("agentiffai-marketing", all_tools.clone());
        let content_names: Vec<&str> = content_tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            content_names,
            vec![
                "Archon_list_tasks",
                "Notion_post_search",
                "Kit_list_subscribers",
                "memory_search",
                "create_job",
            ]
        );

        // Dev channel: Archon+Kiro+Notion MCP tools, all builtins (no whitelist)
        let dev_tools = config.filter_tool_defs("agentiffai-dev-issues", all_tools.clone());
        let dev_names: Vec<&str> = dev_tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            dev_names,
            vec![
                "Archon_list_tasks",
                "Notion_post_search",
                "Kiro_run_task",
                "memory_search",
                "shell",
                "create_job",
            ]
        );

        // DM: everything
        let dm_tools = config.filter_tool_defs("slack-dm", all_tools);
        assert_eq!(dm_tools.len(), 7);
    }
}
