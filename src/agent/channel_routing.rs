//! Per-channel tool routing.
//!
//! Loads `~/.ironclaw/channel-routing.json` and filters which tools
//! (MCP and built-in) the LLM can see based on the originating channel.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::llm::ToolDefinition;

/// Channel-to-tool-group routing configuration.
///
/// Loaded from `channel-routing.json` in the IronClaw base directory.
/// When present, each incoming message's channel name is mapped to a group,
/// and only tools belonging to that group's allowed MCP servers (plus any
/// whitelisted built-in tools) are shown to the LLM.
#[derive(Debug, Clone, Deserialize, Serialize)]
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

    /// Pre-computed MCP server prefixes sorted by length descending.
    /// Populated by `precompute_prefixes()` after deserialization.
    #[serde(skip)]
    sorted_prefixes: Vec<String>,
}

impl PartialEq for ChannelRoutingConfig {
    fn eq(&self, other: &Self) -> bool {
        self.groups == other.groups
            && self.builtin_whitelist == other.builtin_whitelist
            && self.channels == other.channels
            && self.default_group == other.default_group
    }
}

impl Eq for ChannelRoutingConfig {}

/// Exact channel names that identify direct messages (bypass routing entirely).
const DM_EXACT: &[&str] = &["gateway", "cli", "repl"];

/// Channel name prefixes (with delimiter) for relay DMs.
const DM_RELAY_PREFIXES: &[&str] = &["slack-dm-", "telegram-dm-"];

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
        match serde_json::from_str::<Self>(&content) {
            Ok(mut config) => {
                if let Err(e) = config.validate() {
                    tracing::warn!("Channel routing config validation failed: {}", e);
                    return None;
                }
                config.precompute_prefixes();
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

    /// Validate configuration at load time.
    fn validate(&self) -> Result<(), String> {
        // default_group must exist in groups
        if !self.groups.contains_key(&self.default_group) {
            return Err(format!(
                "default_group '{}' not found in groups",
                self.default_group
            ));
        }
        // All channel mappings must reference existing groups
        for (channel, group) in &self.channels {
            if !self.groups.contains_key(group) {
                return Err(format!(
                    "channel '{}' maps to group '{}' which does not exist",
                    channel, group
                ));
            }
        }
        // All builtin_whitelist keys must reference existing groups
        for group in self.builtin_whitelist.keys() {
            if !self.groups.contains_key(group) {
                return Err(format!(
                    "builtin_whitelist references group '{}' which does not exist",
                    group
                ));
            }
        }
        Ok(())
    }

    /// Pre-compute sorted MCP server prefixes (longest first) to avoid
    /// allocations on the hot path.
    fn precompute_prefixes(&mut self) {
        let mut all_servers: Vec<String> = self
            .groups
            .values()
            .flatten()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        all_servers.sort_by_key(|s| std::cmp::Reverse(s.len()));
        self.sorted_prefixes = all_servers;
    }

    /// Resolve which group a channel belongs to.
    pub fn resolve_group(&self, channel: &str) -> &str {
        self.channels
            .get(channel)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_group)
    }

    /// Whether this channel name represents a direct message (no filtering).
    pub fn is_dm(channel: &str, metadata: &serde_json::Value) -> bool {
        // Exact matches for web/CLI channels
        if DM_EXACT.contains(&channel) {
            return true;
        }
        // Prefix matches for relay DMs with delimiter
        if DM_RELAY_PREFIXES.iter().any(|p| channel.starts_with(p)) {
            return true;
        }
        // Slack DMs: channel name starts with 'D' (Slack convention)
        if channel == "slack" || channel == "slack-relay" {
            if metadata
                .get("channel")
                .and_then(|v| v.as_str())
                .is_some_and(|ch| ch.starts_with('D'))
            {
                return true;
            }
            if metadata
                .get("event_type")
                .and_then(|v| v.as_str())
                .is_some_and(|et| et == "direct_message")
            {
                return true;
            }
        }
        // Telegram DMs: chat_type == "private"
        if channel == "telegram"
            && metadata
                .get("chat_type")
                .and_then(|v| v.as_str())
                .is_some_and(|ct| ct == "private")
        {
            return true;
        }
        false
    }

    /// Filter tool definitions based on channel routing rules.
    ///
    /// MCP tools are identified by having an underscore-separated server prefix
    /// (e.g. `Notion_post_search` → server `Notion`). Tools without a known
    /// server prefix are treated as built-in tools.
    pub fn filter_tool_defs(
        &self,
        channel: &str,
        metadata: &serde_json::Value,
        tools: Vec<ToolDefinition>,
    ) -> Vec<ToolDefinition> {
        if Self::is_dm(channel, metadata) {
            return tools;
        }

        let group = self.resolve_group(channel);

        let allowed_servers = match self.groups.get(group) {
            Some(servers) => servers,
            None => {
                tracing::warn!(
                    group,
                    "Channel routing group not found, blocking all MCP tools"
                );
                // Fail safe: only allow built-in tools
                return tools
                    .into_iter()
                    .filter(|tool| self.extract_mcp_server(&tool.name).is_none())
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
    /// Uses pre-computed prefixes sorted by length descending to avoid
    /// `Kit` matching `KitchenAI_recipe_search`.
    fn extract_mcp_server<'a>(&self, tool_name: &'a str) -> Option<&'a str> {
        for server in &self.sorted_prefixes {
            if tool_name.len() > server.len()
                && tool_name.as_bytes()[server.len()] == b'_'
                && tool_name.starts_with(server.as_str())
            {
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
        let mut config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        config.precompute_prefixes();
        config
    }

    fn no_metadata() -> serde_json::Value {
        serde_json::json!({})
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
    fn test_is_dm_exact_matches() {
        let md = no_metadata();
        assert!(ChannelRoutingConfig::is_dm("gateway", &md));
        assert!(ChannelRoutingConfig::is_dm("cli", &md));
        assert!(ChannelRoutingConfig::is_dm("repl", &md));
        // "web" alone should NOT match (use "gateway" for web chat)
        assert!(!ChannelRoutingConfig::is_dm("web", &md));
        // "web-team-standup" must not bypass routing
        assert!(!ChannelRoutingConfig::is_dm("web-team-standup", &md));
        assert!(!ChannelRoutingConfig::is_dm("agentiffai-dev-issues", &md));
    }

    #[test]
    fn test_is_dm_slack_metadata() {
        // Slack DM via channel ID starting with 'D'
        let dm_meta = serde_json::json!({"channel": "D12345"});
        assert!(ChannelRoutingConfig::is_dm("slack", &dm_meta));
        assert!(ChannelRoutingConfig::is_dm("slack-relay", &dm_meta));

        // Slack channel message (not DM)
        let chan_meta = serde_json::json!({"channel": "C12345"});
        assert!(!ChannelRoutingConfig::is_dm("slack", &chan_meta));

        // Slack DM via event_type
        let event_meta = serde_json::json!({"event_type": "direct_message"});
        assert!(ChannelRoutingConfig::is_dm("slack", &event_meta));
    }

    #[test]
    fn test_is_dm_telegram_metadata() {
        let private_meta = serde_json::json!({"chat_type": "private"});
        assert!(ChannelRoutingConfig::is_dm("telegram", &private_meta));

        let group_meta = serde_json::json!({"chat_type": "group"});
        assert!(!ChannelRoutingConfig::is_dm("telegram", &group_meta));
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
        let mut config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        config.precompute_prefixes();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Kiro_run_task"),
            make_tool_def("Notion_post_search"),
            make_tool_def("Smartlead_send"),
        ];
        let md = no_metadata();
        let filtered = config.filter_tool_defs("agentiffai-dev-issues", &md, tools);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Archon_list_tasks"));
        assert!(names.contains(&"Kiro_run_task"));
        assert!(names.contains(&"Notion_post_search"));
        assert!(!names.contains(&"Smartlead_send"));
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
        let md = no_metadata();
        let filtered = config.filter_tool_defs("unmapped-channel", &md, tools);
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
        let md = no_metadata();
        let filtered = config.filter_tool_defs("agentiffai-dev-issues", &md, tools);
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
        let md = no_metadata();
        let filtered = config.filter_tool_defs("gateway", &md, tools);
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
    fn test_validate_rejects_bad_default_group() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "channels": {},
                "default_group": "typo"
            }"#,
        )
        .unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_validate_rejects_bad_channel_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "channels": {"some-channel": "nonexistent"},
                "default_group": "minimal"
            }"#,
        )
        .unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_validate_rejects_bad_builtin_whitelist_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-routing.json");
        std::fs::write(
            &path,
            r#"{
                "groups": {"minimal": ["Archon"]},
                "builtin_whitelist": {"minmal": ["shell"]},
                "channels": {},
                "default_group": "minimal"
            }"#,
        )
        .unwrap();
        let config = ChannelRoutingConfig::load(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_prefix_matching_longest_wins() {
        // Kit vs KitchenAI — KitchenAI must match first
        let json = r#"{
            "groups": {
                "all": ["Kit", "KitchenAI"]
            },
            "channels": {},
            "default_group": "all"
        }"#;
        let mut config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        config.precompute_prefixes();

        assert_eq!(
            config.extract_mcp_server("KitchenAI_recipe_search"),
            Some("KitchenAI")
        );
        assert_eq!(
            config.extract_mcp_server("Kit_list_subscribers"),
            Some("Kit")
        );
    }

    #[test]
    fn test_unknown_group_blocks_mcp_allows_builtins() {
        // If a group somehow isn't found, fail safe: block MCP, allow built-in
        let mut config = sample_config();
        // Force a channel to map to a nonexistent group (bypassing validation for test)
        config
            .channels
            .insert("hacked-channel".to_string(), "nonexistent".to_string());
        let tools = vec![make_tool_def("Archon_list_tasks"), make_tool_def("shell")];
        let md = no_metadata();
        let filtered = config.filter_tool_defs("hacked-channel", &md, tools);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"Archon_list_tasks")); // safety: test assertion in #[test] function
        assert!(names.contains(&"shell")); // safety: test assertion in #[test] function
    }

    #[test]
    fn test_partial_eq_detects_content_changes() {
        let config_a = sample_config();
        let mut config_b = sample_config();
        assert_eq!(config_a, config_b);

        config_b.default_group = "dev".to_string();
        assert_ne!(config_a, config_b);
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
        let mut config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        config.precompute_prefixes();

        let md = no_metadata();

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

        let content_tools = config.filter_tool_defs("agentiffai-marketing", &md, all_tools.clone());
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
        let dev_tools = config.filter_tool_defs("agentiffai-dev-issues", &md, all_tools.clone());
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

        // DM (gateway): everything
        let dm_tools = config.filter_tool_defs("gateway", &md, all_tools.clone());
        assert_eq!(dm_tools.len(), 7);

        // Slack DM via metadata: everything
        let slack_dm_meta = serde_json::json!({"channel": "D12345"});
        let slack_dm_tools = config.filter_tool_defs("slack", &slack_dm_meta, all_tools);
        assert_eq!(slack_dm_tools.len(), 7);
    }
}
