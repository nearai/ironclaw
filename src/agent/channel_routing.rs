//! Per-channel tool routing.
//!
//! Filters which tools (MCP and built-in) the LLM can see based on the
//! originating channel. Config is loaded from `channel-routing.json`
//! or the database-backed SettingsStore.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::llm::ToolDefinition;

/// Channel-to-tool-group routing configuration.
///
/// When present, each incoming message's channel name is mapped to a group,
/// and only tools belonging to that group's allowed MCP servers (plus any
/// whitelisted built-in tools) are shown to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Key used for metadata-based channel resolution (ignored, for compat).
    #[serde(default)]
    pub metadata_key: Option<String>,
}

impl ChannelRoutingConfig {
    /// Load from `<base_dir>/channel-routing.json`. Returns `None` if the file
    /// doesn't exist, can't be parsed, or fails validation.
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
        match serde_json::from_str::<Self>(&content) {
            Ok(config) => {
                if let Err(e) = config.validate() {
                    tracing::error!("Channel routing config invalid: {}", e);
                    return None;
                }
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

    /// Validate that group references are consistent.
    fn validate(&self) -> Result<(), String> {
        // default_group must exist in groups
        if !self.groups.contains_key(&self.default_group) {
            return Err(format!(
                "default_group '{}' not found in groups (available: {:?})",
                self.default_group,
                self.groups.keys().collect::<Vec<_>>()
            ));
        }
        // Every channel mapping must reference an existing group
        for (channel, group) in &self.channels {
            if !self.groups.contains_key(group) {
                return Err(format!(
                    "channel '{}' maps to group '{}' which doesn't exist (available: {:?})",
                    channel,
                    group,
                    self.groups.keys().collect::<Vec<_>>()
                ));
            }
        }
        Ok(())
    }

    /// Resolve which group a channel belongs to.
    pub fn resolve_group(&self, channel: &str) -> &str {
        self.channels
            .get(channel)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_group)
    }

    /// Whether this channel name represents a direct message (bypass routing).
    ///
    /// Uses exact match for short names and prefix-with-delimiter for DM channels.
    pub fn is_dm(channel: &str) -> bool {
        // Exact matches for CLI/REPL/web gateway
        matches!(channel, "cli" | "repl" | "web")
            // Prefix matches for Slack/Telegram DMs (e.g. "slack-dm-U12345")
            || channel == "slack-dm"
            || channel.starts_with("slack-dm-")
            || channel == "telegram-dm"
            || channel.starts_with("telegram-dm-")
    }

    /// Collect all unique MCP server names across all groups, sorted by
    /// length descending. This ensures `KitchenAI_` is matched before `Kit_`.
    fn sorted_server_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .groups
            .values()
            .flatten()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        names.sort_by_key(|s| std::cmp::Reverse(s.len()));
        names
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
                let sorted = self.sorted_server_names();
                return tools
                    .into_iter()
                    .filter(|t| Self::extract_mcp_server_from(&t.name, &sorted).is_none())
                    .collect();
            }
        };

        let builtin_whitelist = self.builtin_whitelist.get(group);
        let sorted = self.sorted_server_names();

        tools
            .into_iter()
            .filter(|tool| {
                if let Some(server) = Self::extract_mcp_server_from(&tool.name, &sorted) {
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
    /// Checks against a pre-sorted list (longest first) to avoid prefix
    /// collisions (e.g. `Kit` matching `KitchenAI_recipe_search`).
    fn extract_mcp_server_from<'a>(
        tool_name: &'a str,
        sorted_servers: &[String],
    ) -> Option<&'a str> {
        for server in sorted_servers {
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
        serde_json::from_str(json).unwrap() // safety: test helper
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
        assert_eq!(config.groups.len(), 2); // safety: test assertion
        assert_eq!(config.default_group, "minimal"); // safety: test assertion
        assert_eq!(config.channels["agentiffai-dev-issues"], "dev"); // safety: test assertion
    }

    #[test]
    fn test_resolve_group_mapped_channel() {
        let config = sample_config();
        assert_eq!(config.resolve_group("agentiffai-dev-issues"), "dev"); // safety: test assertion
    }

    #[test]
    fn test_resolve_group_unmapped_falls_to_default() {
        let config = sample_config();
        assert_eq!(config.resolve_group("random-channel"), "minimal"); // safety: test assertion
    }

    #[test]
    fn test_is_dm() {
        // Exact matches
        assert!(ChannelRoutingConfig::is_dm("slack-dm")); // safety: test assertion
        assert!(ChannelRoutingConfig::is_dm("telegram-dm")); // safety: test assertion
        assert!(ChannelRoutingConfig::is_dm("cli")); // safety: test assertion
        assert!(ChannelRoutingConfig::is_dm("repl")); // safety: test assertion
        assert!(ChannelRoutingConfig::is_dm("web")); // safety: test assertion
        // Prefix with delimiter
        assert!(ChannelRoutingConfig::is_dm("slack-dm-U12345")); // safety: test assertion
        assert!(ChannelRoutingConfig::is_dm("telegram-dm-12345")); // safety: test assertion
        // NOT DMs
        assert!(!ChannelRoutingConfig::is_dm("agentiffai-dev-issues")); // safety: test assertion
        assert!(!ChannelRoutingConfig::is_dm("web-team-standup")); // safety: test assertion
        assert!(!ChannelRoutingConfig::is_dm("cli-tools")); // safety: test assertion
        assert!(!ChannelRoutingConfig::is_dm("repl-server")); // safety: test assertion
        assert!(!ChannelRoutingConfig::is_dm("webhook")); // safety: test assertion
    }

    #[test]
    fn test_prefix_matching_longest_first() {
        let json = r#"{
            "groups": {
                "short": ["Kit"],
                "long": ["KitchenAI"]
            },
            "channels": { "ch1": "short" },
            "default_group": "short"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test
        let sorted = config.sorted_server_names();
        // KitchenAI should come before Kit (longer first)
        let ki_idx = sorted.iter().position(|s| s == "KitchenAI"); // safety: test
        let k_idx = sorted.iter().position(|s| s == "Kit"); // safety: test
        assert!(ki_idx.unwrap() < k_idx.unwrap()); // safety: test assertion

        // KitchenAI_recipe_search should match KitchenAI, not Kit
        let server =
            ChannelRoutingConfig::extract_mcp_server_from("KitchenAI_recipe_search", &sorted);
        assert_eq!(server, Some("KitchenAI")); // safety: test assertion

        // Kit_list_subscribers should still match Kit
        let server2 =
            ChannelRoutingConfig::extract_mcp_server_from("Kit_list_subscribers", &sorted);
        assert_eq!(server2, Some("Kit")); // safety: test assertion
    }

    #[test]
    fn test_validate_valid_config() {
        let config = sample_config();
        assert!(config.validate().is_ok()); // safety: test assertion
    }

    #[test]
    fn test_validate_invalid_default_group() {
        let json = r#"{
            "groups": { "minimal": ["Archon"] },
            "channels": {},
            "default_group": "nonexistent"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test
        assert!(config.validate().is_err()); // safety: test assertion
        assert!(config.validate().unwrap_err().contains("nonexistent")); // safety: test assertion
    }

    #[test]
    fn test_validate_invalid_channel_group() {
        let json = r#"{
            "groups": { "minimal": ["Archon"] },
            "channels": { "ch1": "typo_group" },
            "default_group": "minimal"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test
        assert!(config.validate().is_err()); // safety: test assertion
        assert!(config.validate().unwrap_err().contains("typo_group")); // safety: test assertion
    }

    #[test]
    fn test_load_validates_config() {
        let dir = tempfile::tempdir().unwrap(); // safety: test
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "channels": {},
                "default_group": "nonexistent"
            }"#,
        )
        .unwrap(); // safety: test
        // Should return None because validation fails
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none()); // safety: test assertion
    }

    #[test]
    fn test_filter_keeps_allowed_mcp_tools() {
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
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test
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
        assert!(names.contains(&"Archon_list_tasks")); // safety: test assertion
        assert!(names.contains(&"memory_search")); // safety: test assertion
        assert!(names.contains(&"create_job")); // safety: test assertion
        assert!(!names.contains(&"shell")); // safety: test assertion
        assert!(!names.contains(&"http_request")); // safety: test assertion
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
        assert_eq!(filtered.len(), 3); // safety: test assertion
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
        assert_eq!(filtered.len(), 3); // safety: test assertion
    }

    #[test]
    fn test_load_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap(); // safety: test
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none()); // safety: test assertion
    }

    #[test]
    fn test_load_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap(); // safety: test
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "channels": {},
                "default_group": "minimal"
            }"#,
        )
        .unwrap(); // safety: test
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_some()); // safety: test assertion
        assert_eq!(config.unwrap().default_group, "minimal"); // safety: test assertion
    }

    #[test]
    fn test_load_returns_none_for_invalid_json() {
        let dir = tempfile::tempdir().unwrap(); // safety: test
        let path = dir.path().join("channel-routing.json");
        std::fs::write(&path, "not json").unwrap(); // safety: test
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none()); // safety: test assertion
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
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap(); // safety: test

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
                "create_job"
            ]
        ); // safety: test assertion
    }
}
