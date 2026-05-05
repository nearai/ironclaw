//! Per-channel tool routing.
//!
//! Filters which tools (MCP and built-in) the LLM can see based on the
//! originating channel. Configuration is stored in the database-backed
//! [`crate::db::SettingsStore`] under the key `channel_routing` and is
//! loaded at startup via [`ChannelRoutingConfig::load_from_store`].
//! A file-based loader ([`ChannelRoutingConfig::load`]) is available as
//! a migration utility.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::db::SettingsStore;
use crate::llm::ToolDefinition;

/// Channel-to-tool-group routing configuration.
///
/// Stored in the database-backed settings system (key: `channel_routing`).
/// When present, each incoming message's channel name is mapped to a group,
/// and only tools belonging to that group's allowed MCP servers (plus any
/// whitelisted built-in tools) are shown to the LLM. Use
/// [`Self::load_from_store`] / [`Self::save_to_store`] for persistence.
#[derive(Debug, Clone, Serialize)]
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

    /// Per-group allowed-server sets for O(1) membership tests.
    /// Mirrors `groups` values; populated by `precompute_prefixes()`.
    #[serde(skip)]
    allowed_servers_sets: HashMap<String, HashSet<String>>,

    /// Per-group built-in whitelist sets for O(1) membership tests.
    /// Mirrors `builtin_whitelist` values; populated by `precompute_prefixes()`.
    #[serde(skip)]
    builtin_whitelist_sets: HashMap<String, HashSet<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChannelRoutingConfigSerde {
    groups: HashMap<String, Vec<String>>,
    #[serde(default)]
    builtin_whitelist: HashMap<String, Vec<String>>,
    channels: HashMap<String, String>,
    default_group: String,
}

impl<'de> Deserialize<'de> for ChannelRoutingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ChannelRoutingConfigSerde::deserialize(deserializer)?;
        let mut config = Self {
            groups: raw.groups,
            builtin_whitelist: raw.builtin_whitelist,
            channels: raw.channels,
            default_group: raw.default_group,
            sorted_prefixes: Vec::new(),
            allowed_servers_sets: HashMap::new(),
            builtin_whitelist_sets: HashMap::new(),
        };
        config.precompute_prefixes();
        Ok(config)
    }
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
const DM_EXACT: &[&str] = &["gateway", "cli", "repl", "tui", "http"];

/// Channel-owned metadata bit for "this message is a DM".
///
/// Trusted channel adapters set this after validating or normalizing the
/// inbound event. Routing should not rely on raw relay webhook fields alone.
pub const TRUSTED_DM_METADATA_KEY: &str = "channel_routing_dm";

impl ChannelRoutingConfig {
    /// Return an `Arc<RwLock<Option<Self>>>` initialised to `None`.
    ///
    /// Convenience constructor for `AgentDeps.channel_routing` at startup
    /// (before any config has been loaded) and for test helpers.
    pub fn none_arc() -> std::sync::Arc<tokio::sync::RwLock<Option<Self>>> {
        std::sync::Arc::new(tokio::sync::RwLock::new(None))
    }

    /// Load config from `store` and atomically replace the value in `routing`.
    ///
    /// Returns `true` if the stored config differed from the current value
    /// (using the semantic `PartialEq` impl — presence/absence *and* content
    /// changes both count as changed). Used at startup and by the SIGHUP
    /// handler for hot-reload.
    pub async fn reload_from_store(
        store: &(dyn SettingsStore + Send + Sync),
        user_id: &str,
        routing: &std::sync::Arc<tokio::sync::RwLock<Option<Self>>>,
    ) -> bool {
        let new_routing = Self::load_from_store(store, user_id).await;
        let mut guard = routing.write().await;
        let changed = new_routing != *guard;
        *guard = new_routing;
        changed
    }

    /// Load from `<base_dir>/channel-routing.json`.
    ///
    /// **File-based utility** — not used in the normal startup path. Useful for
    /// one-off migration of an existing `channel-routing.json` into the database
    /// via [`Self::save_to_store`]. Returns `None` if the file doesn't exist or
    /// can't be parsed (logged as warning).
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
                    tracing::warn!("Channel routing config validation failed: {}", e);
                    return None;
                }
                tracing::debug!(
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

    /// Load from database-backed SettingsStore.
    ///
    /// This provides hot-reload support — the settings system handles cache
    /// invalidation and the web UI can modify the config without restarts.
    pub async fn load_from_store(
        store: &(dyn SettingsStore + Send + Sync),
        user_id: &str,
    ) -> Option<Self> {
        match store.get_setting(user_id, "channel_routing").await {
            Ok(Some(value)) => Self::parse_stored_value(value, "database"),
            Ok(None) => {
                tracing::debug!("No channel routing config in database");
                None
            }
            Err(e) => {
                tracing::warn!("Failed to read channel routing from DB: {}", e);
                None
            }
        }
    }

    /// Load from a system-scoped database handle for a specific user.
    ///
    /// This is used by autonomous workers, which hold a [`SystemScope`]
    /// rather than a raw [`SettingsStore`] but still need to apply the same
    /// per-user routing rules as interactive dispatcher turns.
    pub async fn load_from_system_scope(
        store: &crate::tenant::SystemScope,
        user_id: &str,
    ) -> Option<Self> {
        match store.get_channel_routing(user_id).await {
            Ok(Some(value)) => Self::parse_stored_value(value, "database"),
            Ok(None) => {
                tracing::debug!("No channel routing config in database");
                None
            }
            Err(e) => {
                tracing::warn!("Failed to read channel routing from DB: {}", e);
                None
            }
        }
    }

    /// Persist to database-backed SettingsStore.
    ///
    /// Write counterpart to [`Self::load_from_store`]. Call this after
    /// modifying the config via the web UI or settings API to persist the
    /// updated routing rules to the database.
    pub async fn save_to_store(
        &self,
        store: &(dyn SettingsStore + Send + Sync),
        user_id: &str,
    ) -> Result<(), crate::error::DatabaseError> {
        let value = serde_json::to_value(self)
            .map_err(|e| crate::error::DatabaseError::Serialization(e.to_string()))?;
        store.set_setting(user_id, "channel_routing", &value).await
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
        // Sorted prefixes for longest-prefix-first MCP server extraction.
        let mut all_servers: Vec<String> = self
            .groups
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        all_servers.sort_by_key(|s| std::cmp::Reverse(s.len()));
        self.sorted_prefixes = all_servers;

        // Per-group HashSets for O(1) membership tests in filter_tool_defs.
        self.allowed_servers_sets = self
            .groups
            .iter()
            .map(|(group, servers)| (group.clone(), servers.iter().cloned().collect()))
            .collect();
        self.builtin_whitelist_sets = self
            .builtin_whitelist
            .iter()
            .map(|(group, tools)| (group.clone(), tools.iter().cloned().collect()))
            .collect();
    }

    fn parse_stored_value(value: serde_json::Value, source: &str) -> Option<Self> {
        match serde_json::from_value::<Self>(value) {
            Ok(config) => {
                if let Err(e) = config.validate() {
                    tracing::warn!("Channel routing config from {} invalid: {}", source, e);
                    return None;
                }
                tracing::debug!(
                    source,
                    groups = ?config.groups.keys().collect::<Vec<_>>(),
                    channels = config.channels.len(),
                    "Loaded channel routing config"
                );
                Some(config)
            }
            Err(e) => {
                tracing::warn!("Failed to parse channel routing from {}: {}", source, e);
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
    pub fn is_dm(channel: &str, metadata: &serde_json::Value) -> bool {
        // Exact matches for web/CLI channels
        if DM_EXACT.contains(&channel) {
            return true;
        }
        // Any trusted relay adapter can stamp this flag to bypass routing.
        // Set server-side after webhook/auth validation — not spoofable by clients.
        if metadata
            .get(TRUSTED_DM_METADATA_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return true;
        }
        // Slack DMs: channel name starts with 'D' (Slack convention)
        if channel == "slack" {
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
    /// (e.g. `Notion_post_search` → server `Notion`). Tools not matching any
    /// known MCP server prefix are checked against `builtin_names`; tools that
    /// are neither a known MCP server NOR a registered built-in are blocked
    /// (fail-closed) to prevent tools from unregistered MCP servers leaking
    /// through as apparent built-ins.
    pub fn filter_tool_defs(
        &self,
        channel: &str,
        metadata: &serde_json::Value,
        tools: Vec<ToolDefinition>,
        builtin_names: &std::collections::HashSet<String>,
    ) -> Vec<ToolDefinition> {
        if Self::is_dm(channel, metadata) {
            return tools;
        }

        let group = self.resolve_group(channel);

        let allowed_set = match self.allowed_servers_sets.get(group) {
            Some(set) => set,
            None => {
                let default_whitelist_set =
                    self.builtin_whitelist_sets.get(&self.default_group);
                tracing::warn!(
                    group,
                    default_group = %self.default_group,
                    "Channel routing group not found, blocking MCP tools and applying default built-in policy"
                );
                return tools
                    .into_iter()
                    .filter(|tool| {
                        if self.extract_mcp_server(&tool.name).is_some() {
                            return false;
                        }
                        // Fail-closed: unknown-source tools (not in builtin registry) are blocked.
                        if !builtin_names.contains(&tool.name) {
                            return false;
                        }
                        match default_whitelist_set {
                            Some(set) => set.contains(tool.name.as_str()),
                            None => true,
                        }
                    })
                    .collect();
            }
        };

        let builtin_set = self.builtin_whitelist_sets.get(group);

        tools
            .into_iter()
            .filter(|tool| {
                if let Some(server) = self.extract_mcp_server(&tool.name) {
                    allowed_set.contains(server)
                } else if builtin_names.contains(&tool.name) {
                    match builtin_set {
                        Some(set) => set.contains(tool.name.as_str()),
                        None => true,
                    }
                } else {
                    // Unknown MCP server not in any routing group — fail closed.
                    tracing::debug!(
                        tool_name = %tool.name,
                        channel,
                        "Channel routing: blocking unknown-source tool (not in any MCP group and not a built-in)"
                    );
                    false
                }
            })
            .collect()
    }

    /// Returns `true` if the named tool is permitted to execute on `channel`.
    ///
    /// Uses the same allowlist logic as [`Self::filter_tool_defs`] but for a
    /// single tool name, without requiring a full `ToolDefinition`. Does NOT
    /// check metadata-based DM status (no metadata is available at dispatch
    /// time) — channels in `DM_EXACT` still bypass routing. This is the
    /// execution-layer enforcement called from [`crate::tools::dispatch::ToolDispatcher`].
    pub fn is_tool_permitted(
        &self,
        channel: &str,
        tool_name: &str,
        builtin_names: &std::collections::HashSet<String>,
    ) -> bool {
        // DM_EXACT channels bypass routing at execution layer too.
        if DM_EXACT.contains(&channel) {
            return true;
        }

        let group = self.resolve_group(channel);

        let allowed_set = match self.allowed_servers_sets.get(group) {
            Some(set) => set,
            None => {
                // Unknown group — fail-closed: block MCP tools, apply default built-in policy.
                if self.extract_mcp_server(tool_name).is_some() {
                    return false;
                }
                if !builtin_names.contains(tool_name) {
                    return false;
                }
                return match self.builtin_whitelist_sets.get(&self.default_group) {
                    Some(set) => set.contains(tool_name),
                    None => true,
                };
            }
        };

        if let Some(server) = self.extract_mcp_server(tool_name) {
            allowed_set.contains(server)
        } else if builtin_names.contains(tool_name) {
            match self.builtin_whitelist_sets.get(group) {
                Some(set) => set.contains(tool_name),
                None => true,
            }
        } else {
            false
        }
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
                return tool_name.get(..server.len());
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

    /// Built-in tool names used across filter tests. MCP-prefixed tools
    /// (Archon_*, Notion_*, etc.) must NOT appear here.
    fn test_builtin_names() -> std::collections::HashSet<String> {
        [
            "memory_search",
            "memory_write",
            "create_job",
            "shell",
            "http_request",
            "web_search",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
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
        assert!(ChannelRoutingConfig::is_dm("tui", &md));
        assert!(ChannelRoutingConfig::is_dm("http", &md));
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
        assert!(!ChannelRoutingConfig::is_dm("slack-relay", &dm_meta));

        // Slack channel message (not DM)
        let chan_meta = serde_json::json!({"channel": "C12345"});
        assert!(!ChannelRoutingConfig::is_dm("slack", &chan_meta));

        // Slack DM via event_type
        let event_meta = serde_json::json!({"event_type": "direct_message"});
        assert!(ChannelRoutingConfig::is_dm("slack", &event_meta));
    }

    #[test]
    fn test_is_dm_trusted_flag_works_for_any_channel() {
        // Without the flag, relay channels are subject to routing.
        let untrusted_meta = serde_json::json!({"event_type": "direct_message"});
        assert!(!ChannelRoutingConfig::is_dm("slack-relay", &untrusted_meta));
        assert!(!ChannelRoutingConfig::is_dm(
            "telegram-relay",
            &untrusted_meta
        ));

        // With the trusted server-side flag, any relay channel bypasses routing.
        let trusted_meta = serde_json::json!({
            "event_type": "direct_message",
            TRUSTED_DM_METADATA_KEY: true,
        });
        assert!(ChannelRoutingConfig::is_dm("slack-relay", &trusted_meta));
        assert!(ChannelRoutingConfig::is_dm("telegram-relay", &trusted_meta));
        assert!(ChannelRoutingConfig::is_dm(
            "some-future-relay",
            &trusted_meta
        ));
    }

    #[test]
    fn test_is_dm_telegram_metadata() {
        let private_meta = serde_json::json!({"chat_type": "private"});
        assert!(ChannelRoutingConfig::is_dm("telegram", &private_meta));

        let group_meta = serde_json::json!({"chat_type": "group"});
        assert!(!ChannelRoutingConfig::is_dm("telegram", &group_meta));
    }

    #[test]
    fn test_filter_dm_returns_all_tools_for_tui_and_http() {
        let config = sample_config();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Smartlead_send"),
            make_tool_def("shell"),
        ];
        let md = no_metadata();
        let bn = test_builtin_names();
        assert_eq!(
            config
                .filter_tool_defs("tui", &md, tools.clone(), &bn)
                .len(),
            3
        );
        assert_eq!(config.filter_tool_defs("http", &md, tools, &bn).len(), 3);
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
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("Kiro_run_task"),
            make_tool_def("Notion_post_search"),
            make_tool_def("Smartlead_send"),
        ];
        let md = no_metadata();
        let filtered =
            config.filter_tool_defs("agentiffai-dev-issues", &md, tools, &test_builtin_names());
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
        let filtered =
            config.filter_tool_defs("unmapped-channel", &md, tools, &test_builtin_names());
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
        let filtered =
            config.filter_tool_defs("agentiffai-dev-issues", &md, tools, &test_builtin_names());
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
        let filtered = config.filter_tool_defs("gateway", &md, tools, &test_builtin_names());
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
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();

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
    fn test_deserialize_precomputes_prefixes() {
        let json = r#"{
            "groups": {
                "all": ["Kit", "KitchenAI"]
            },
            "channels": {},
            "default_group": "all"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.extract_mcp_server("KitchenAI_recipe_search"),
            Some("KitchenAI")
        );
    }

    #[test]
    fn test_unknown_group_blocks_mcp_and_applies_default_builtin_policy() {
        let mut config = sample_config();
        // Force a channel to map to a nonexistent group (bypassing validation for test)
        config
            .channels
            .insert("hacked-channel".to_string(), "nonexistent".to_string());
        let tools = vec![
            make_tool_def("Archon_list_tasks"),
            make_tool_def("memory_search"),
            make_tool_def("shell"),
        ];
        let md = no_metadata();
        let filtered = config.filter_tool_defs("hacked-channel", &md, tools, &test_builtin_names());
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"Archon_list_tasks"));
        assert!(names.contains(&"memory_search"));
        assert!(!names.contains(&"shell"));
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
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();

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

        let bn = test_builtin_names();
        let content_tools = config.filter_tool_defs("agentiffai-marketing", &md, all_tools.clone(), &bn);
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
        let dev_tools = config.filter_tool_defs("agentiffai-dev-issues", &md, all_tools.clone(), &bn);
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
        let dm_tools = config.filter_tool_defs("gateway", &md, all_tools.clone(), &bn);
        assert_eq!(dm_tools.len(), 7);

        // Slack DM via metadata: everything
        let slack_dm_meta = serde_json::json!({"channel": "D12345"});
        let slack_dm_tools = config.filter_tool_defs("slack", &slack_dm_meta, all_tools, &bn);
        assert_eq!(slack_dm_tools.len(), 7);
    }

    /// Tests for `reload_from_store` — exercises the SIGHUP hot-reload path
    /// end-to-end: persist → load → mutate → reload, asserting `changed` is
    /// correct at each step and the arc reflects the latest config.
    ///
    /// Requires the `libsql` feature because it uses an in-memory SQLite
    /// database. Run with: `cargo test --features libsql channel_routing`
    #[cfg(feature = "libsql")]
    mod reload_tests {
        use super::*;
        use crate::db::{Database, libsql::LibSqlBackend};

        /// Create a temporary local SQLite DB with migrations applied.
        ///
        /// In-memory databases do not share state between libSQL connections
        /// (each `connect()` call sees an empty DB), so tests that perform
        /// multiple operations must use a local file with a temp dir.
        async fn test_db() -> (LibSqlBackend, tempfile::TempDir) {
            let dir = tempfile::tempdir().unwrap();
            let db = LibSqlBackend::new_local(&dir.path().join("test.db"))
                .await
                .unwrap();
            db.run_migrations().await.unwrap();
            (db, dir) // keep `dir` alive so the temp file isn't deleted
        }

        #[tokio::test]
        async fn test_reload_from_store_detects_changes() {
            let (db, _dir) = test_db().await;
            let arc = ChannelRoutingConfig::none_arc();

            // Nothing in DB yet — arc stays None, reported as unchanged
            let changed = ChannelRoutingConfig::reload_from_store(&db, "test-user", &arc).await;
            assert!(!changed, "empty DB should be unchanged");
            assert!(arc.read().await.is_none());

            // Store a config; reload should transition None → Some (changed)
            let config = sample_config();
            config.save_to_store(&db, "test-user").await.unwrap();
            let changed = ChannelRoutingConfig::reload_from_store(&db, "test-user", &arc).await;
            assert!(changed, "None → Some must be reported as changed");
            assert!(arc.read().await.is_some());

            // Same config again — content identical, not changed
            let changed = ChannelRoutingConfig::reload_from_store(&db, "test-user", &arc).await;
            assert!(!changed, "identical reload must be unchanged");

            // Mutate default_group and reload — content changed
            let mut updated = sample_config();
            updated.default_group = "dev".to_string();
            updated.save_to_store(&db, "test-user").await.unwrap();
            let changed = ChannelRoutingConfig::reload_from_store(&db, "test-user", &arc).await;
            assert!(changed, "content change must be reported as changed");
            assert_eq!(arc.read().await.as_ref().unwrap().default_group, "dev");
        }

        #[tokio::test]
        async fn test_none_arc_initialises_to_none() {
            let arc = ChannelRoutingConfig::none_arc();
            assert!(arc.read().await.is_none());
        }

        #[tokio::test]
        async fn test_save_and_load_roundtrip() {
            let (db, _dir) = test_db().await;

            let config = sample_config();
            config.save_to_store(&db, "user1").await.unwrap();

            let loaded = ChannelRoutingConfig::load_from_store(&db, "user1")
                .await
                .expect("should load saved config");
            assert_eq!(loaded, config);
        }
    }

    #[test]
    fn test_extract_mcp_server_handles_multibyte_prefix() {
        // Regression: tool_name.as_bytes()[server.len()] byte-indexes into the
        // string. If `server` contains multi-byte chars (e.g. "Café"), its
        // `.len()` is the *byte* length (5), not char length (4). The check
        // `tool_name.as_bytes()[server.len()] == b'_'` correctly accesses byte 5,
        // and `tool_name.get(..server.len())` is the UTF-8-safe slice that
        // returns None rather than panicking on a non-char-boundary index.
        // Both operations must not panic on multi-byte server names.
        let json = r#"{
            "groups": {
                "all": ["Café"]
            },
            "channels": {},
            "default_group": "all"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();
        // "Café" is 5 bytes (UTF-8: C, a, f, 0xc3, 0xa9).
        // "Café_query" — byte 5 is '_'; server.len() == 5 → safe.
        assert_eq!(config.extract_mcp_server("Café_query"), Some("Café"));
        // Ensure non-matching multi-byte prefix doesn't panic either.
        assert_eq!(config.extract_mcp_server("Cafe_query"), None);
        assert_eq!(config.extract_mcp_server("Caf"), None);
    }

    #[test]
    fn test_is_tool_permitted_enforces_routing() {
        let json = r#"{
            "groups": {
                "minimal": ["Archon"],
                "dev": ["Archon", "Kiro"]
            },
            "builtin_whitelist": {
                "minimal": ["create_job"]
            },
            "channels": {
                "agentiffai-dev": "dev"
            },
            "default_group": "minimal"
        }"#;
        let config: ChannelRoutingConfig = serde_json::from_str(json).unwrap();

        let builtin_names: std::collections::HashSet<String> = [
            "shell", "create_job", "memory_search", "web_search",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        // DM_EXACT channels bypass routing entirely.
        assert!(config.is_tool_permitted("gateway", "Smartlead_send", &builtin_names));
        assert!(config.is_tool_permitted("cli", "shell", &builtin_names));

        // Dev channel: Archon + Kiro allowed, all builtins allowed (no whitelist).
        assert!(config.is_tool_permitted("agentiffai-dev", "Archon_list", &builtin_names));
        assert!(config.is_tool_permitted("agentiffai-dev", "Kiro_run", &builtin_names));
        assert!(!config.is_tool_permitted("agentiffai-dev", "Smartlead_send", &builtin_names));
        assert!(config.is_tool_permitted("agentiffai-dev", "shell", &builtin_names)); // no whitelist → all builtins

        // Minimal (default) channel: Archon only, create_job built-in only.
        assert!(config.is_tool_permitted("other-channel", "Archon_list", &builtin_names));
        assert!(!config.is_tool_permitted("other-channel", "Kiro_run", &builtin_names));
        assert!(config.is_tool_permitted("other-channel", "create_job", &builtin_names));
        assert!(!config.is_tool_permitted("other-channel", "shell", &builtin_names));

        // Unknown MCP server (not in any group, not a builtin) — fail closed.
        assert!(!config.is_tool_permitted("other-channel", "Serpstat_keywords", &builtin_names));
        assert!(!config.is_tool_permitted("agentiffai-dev", "Serpstat_keywords", &builtin_names));
    }
}
