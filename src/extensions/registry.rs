//! Curated in-memory catalog of known extensions with fuzzy search.
//!
//! The registry holds well-known channels, tools, and MCP servers that can be
//! installed via conversational commands. Online discoveries are cached here too.

use tokio::sync::RwLock;

use crate::extensions::{
    naming::canonicalize_extension_name,
    AuthHint, ExtensionKind, ExtensionSource, RegistryEntry, ResultSource, SearchResult,
};

/// Curated extension registry with fuzzy search.
pub struct ExtensionRegistry {
    /// Built-in curated entries.
    entries: Vec<RegistryEntry>,
    /// Cached entries from online discovery (session-lived).
    discovery_cache: RwLock<Vec<RegistryEntry>>,
}

impl ExtensionRegistry {
    /// Create a new registry populated with known extensions.
    pub fn new() -> Self {
        Self {
            entries: canonicalize_entries(builtin_entries()),
            discovery_cache: RwLock::new(Vec::new()),
        }
    }

    /// Create a new registry merging builtin entries with catalog-provided entries.
    ///
    /// Deduplicates by `(name, kind)` pair -- a builtin MCP "slack" and a registry
    /// WASM "slack" can coexist since they're different kinds.
    pub fn new_with_catalog(catalog_entries: Vec<RegistryEntry>) -> Self {
        let mut entries = canonicalize_entries(builtin_entries());
        for entry in canonicalize_entries(catalog_entries) {
            if !entries
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                entries.push(entry);
            }
        }
        Self {
            entries,
            discovery_cache: RwLock::new(Vec::new()),
        }
    }

    /// Search the registry by query string. Returns results sorted by relevance.
    ///
    /// Splits the query into lowercase tokens and scores each entry by matches
    /// in name, keywords, and description.
    pub async fn search(&self, query: &str) -> Vec<SearchResult> {
        let tokens: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if tokens.is_empty() {
            // Return all entries when query is empty
            return self
                .entries
                .iter()
                .map(|e| SearchResult {
                    entry: e.clone(),
                    source: ResultSource::Registry,
                    validated: true,
                })
                .collect();
        }

        let mut scored: Vec<(SearchResult, u32)> = Vec::new();

        // Score built-in entries
        for entry in &self.entries {
            let score = score_entry(entry, &tokens);
            if score > 0 {
                scored.push((
                    SearchResult {
                        entry: entry.clone(),
                        source: ResultSource::Registry,
                        validated: true,
                    },
                    score,
                ));
            }
        }

        // Score cached discoveries
        let cache = self.discovery_cache.read().await;
        for entry in cache.iter() {
            let score = score_entry(entry, &tokens);
            if score > 0 {
                scored.push((
                    SearchResult {
                        entry: entry.clone(),
                        source: ResultSource::Discovered,
                        validated: true,
                    },
                    score,
                ));
            }
        }

        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        scored.into_iter().map(|(r, _)| r).collect()
    }

    /// Look up an entry by exact name.
    ///
    /// NOTE: Prefer [`get_with_kind`] when a kind hint is available, to avoid
    /// returning the wrong entry when two entries share a name but differ in kind.
    pub async fn get(&self, name: &str) -> Option<RegistryEntry> {
        let name = canonicalize_extension_name(name).ok()?;
        if let Some(entry) = self.entries.iter().find(|e| e.name == name) {
            return Some(entry.clone());
        }
        let cache = self.discovery_cache.read().await;
        cache.iter().find(|e| e.name == name).cloned()
    }

    /// Look up an entry by exact name, filtering by kind when provided.
    ///
    /// When `kind` is `Some(...)`, only returns an entry matching both name and
    /// kind — never falls back to a different kind. When `kind` is `None`,
    /// returns the first name match (same as [`get`]).
    pub async fn get_with_kind(
        &self,
        name: &str,
        kind: Option<ExtensionKind>,
    ) -> Option<RegistryEntry> {
        let name = canonicalize_extension_name(name).ok()?;
        if let Some(kind) = kind {
            if let Some(entry) = self
                .entries
                .iter()
                .find(|e| e.name == name && e.kind == kind)
            {
                return Some(entry.clone());
            }
            let cache = self.discovery_cache.read().await;
            if let Some(entry) = cache.iter().find(|e| e.name == name && e.kind == kind) {
                return Some(entry.clone());
            }
            // Kind was specified but no entry matches — don't fall back to a
            // different kind, as that would silently misroute the install.
            return None;
        }
        self.get(&name).await
    }

    /// Return all registry entries (builtins + cached discoveries).
    pub async fn all_entries(&self) -> Vec<RegistryEntry> {
        let mut entries = self.entries.clone();
        let cache = self.discovery_cache.read().await;
        for entry in cache.iter() {
            if !entries
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                entries.push(entry.clone());
            }
        }
        entries
    }

    /// Add discovered entries to the cache.
    pub async fn cache_discovered(&self, entries: Vec<RegistryEntry>) {
        let mut cache = self.discovery_cache.write().await;
        for entry in canonicalize_entries(entries) {
            // Deduplicate by (name, kind) — same pair as new_with_catalog()
            if !cache
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                cache.push(entry);
            }
        }
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn canonicalize_entries(entries: Vec<RegistryEntry>) -> Vec<RegistryEntry> {
    entries
        .into_iter()
        .map(|mut entry| {
            if let Ok(name) = canonicalize_extension_name(&entry.name) {
                entry.name = name;
            }
            entry
        })
        .collect()
}

/// Score an entry against search tokens. Higher = better match.
fn score_entry(entry: &RegistryEntry, tokens: &[String]) -> u32 {
    let mut score = 0u32;
    let name_lower = entry.name.to_lowercase();
    let display_lower = entry.display_name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();
    let keywords_lower: Vec<String> = entry.keywords.iter().map(|k| k.to_lowercase()).collect();

    for token in tokens {
        // Exact name match is the strongest signal
        if name_lower == *token {
            score += 100;
        } else if name_lower.contains(token.as_str()) {
            score += 50;
        }

        // Display name match
        if display_lower.contains(token.as_str()) {
            score += 30;
        }

        // Keyword match
        for kw in &keywords_lower {
            if kw == token {
                score += 40;
            } else if kw.contains(token.as_str()) {
                score += 20;
            }
        }

        // Description match (weakest signal)
        if desc_lower.contains(token.as_str()) {
            score += 10;
        }
    }

    score
}

/// Well-known extensions that ship with ironclaw.
///
/// If `relay_url` is provided, a channel-relay Slack entry is included in the list.
/// Pass `None` when the relay is not configured.
pub fn builtin_entries() -> Vec<RegistryEntry> {
    builtin_entries_with_relay(std::env::var("CHANNEL_RELAY_URL").ok())
}

/// Well-known extensions, with an optional relay URL for the channel-relay entry.
///
/// MCP server entries are loaded from `registry/mcp-servers/*.json` via the catalog
/// system. Only runtime-dependent entries (like channel-relay) remain here.
pub fn builtin_entries_with_relay(relay_url: Option<String>) -> Vec<RegistryEntry> {
    let mut entries = vec![];

    // Conditionally add channel-relay entries when relay URL is configured
    if let Some(relay_url) = relay_url {
        entries.push(RegistryEntry {
            name: crate::channels::relay::DEFAULT_RELAY_NAME.to_string(),
            display_name: "Slack".to_string(),
            kind: ExtensionKind::ChannelRelay,
            description: "Connect Slack workspace via channel relay".to_string(),
            keywords: vec![
                "slack".into(),
                "chat".into(),
                "messaging".into(),
                "relay".into(),
            ],
            source: ExtensionSource::ChannelRelay { relay_url },
            fallback_source: None,
            auth_hint: AuthHint::ChannelRelayOAuth,
            version: None,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use crate::extensions::registry::{ExtensionRegistry, score_entry};
    use crate::extensions::{AuthHint, ExtensionKind, ExtensionSource, RegistryEntry};

    #[test]
    fn test_score_exact_name_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };

        let score = score_entry(&entry, &["notion".to_string()]);
        assert!(
            score >= 100,
            "Exact name match should score >= 100, got {}",
            score
        );
    }

    #[test]
    fn test_score_partial_name_match() {
        let entry = RegistryEntry {
            name: "google-calendar".to_string(),
            display_name: "Google Calendar".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Calendar management".to_string(),
            keywords: vec!["events".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };

        let score = score_entry(&entry, &["calendar".to_string()]);
        assert!(
            score > 0,
            "Partial name match should score > 0, got {}",
            score
        );
    }

    #[test]
    fn test_score_keyword_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["wiki".into(), "notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };

        let score = score_entry(&entry, &["wiki".to_string()]);
        assert!(
            score >= 40,
            "Exact keyword match should score >= 40, got {}",
            score
        );
    }

    #[test]
    fn test_score_no_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };

        let score = score_entry(&entry, &["xyzfoobar".to_string()]);
        assert_eq!(score, 0, "No match should score 0");
    }

    /// Helper to create a registry with catalog entries (MCP servers come from catalog now).
    fn registry_with_catalog() -> ExtensionRegistry {
        let catalog = crate::registry::catalog::RegistryCatalog::load_or_embedded()
            .expect("catalog should load");
        let catalog_entries: Vec<RegistryEntry> = catalog
            .all()
            .iter()
            .filter_map(|m| m.to_registry_entry())
            .collect();
        ExtensionRegistry::new_with_catalog(catalog_entries)
    }

    #[tokio::test]
    async fn test_search_returns_sorted() {
        let registry = registry_with_catalog();
        let results = registry.search("notion").await;

        assert!(!results.is_empty(), "Should find notion in registry");
        assert_eq!(results[0].entry.name, "notion");
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_all() {
        let registry = registry_with_catalog();
        let results = registry.search("").await;

        assert!(results.len() > 5, "Empty query should return all entries");
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let registry = registry_with_catalog();
        let results = registry.search("issues tickets").await;

        assert!(
            !results.is_empty(),
            "Should find entries matching 'issues tickets'"
        );
        // Linear should be near the top since it has both keywords
        let linear_pos = results.iter().position(|r| r.entry.name == "linear");
        assert!(linear_pos.is_some(), "Linear should appear in results");
    }

    #[tokio::test]
    async fn test_get_exact_name() {
        let registry = registry_with_catalog();

        let entry = registry.get("notion").await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Notion");

        let missing = registry.get("nonexistent").await;
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_cache_discovered() {
        let registry = ExtensionRegistry::new();

        let discovered = RegistryEntry {
            name: "custom-mcp".to_string(),
            display_name: "Custom MCP".to_string(),
            kind: ExtensionKind::McpServer,
            description: "A custom MCP server".to_string(),
            keywords: vec![],
            source: ExtensionSource::McpUrl {
                url: "https://custom.example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };

        registry.cache_discovered(vec![discovered]).await;

        let entry = registry.get("custom-mcp").await;
        assert!(entry.is_some());

        let results = registry.search("custom").await;
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_cache_deduplication() {
        let registry = ExtensionRegistry::new();

        let entry = RegistryEntry {
            name: "dup".to_string(),
            display_name: "Dup".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Test".to_string(),
            keywords: vec![],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
            version: None,
        };

        registry.cache_discovered(vec![entry.clone()]).await;
        registry.cache_discovered(vec![entry]).await;

        let results = registry.search("dup").await;
        assert_eq!(results.len(), 1, "Should not duplicate cached entries");
    }

    #[tokio::test]
    async fn test_new_with_catalog() {
        let catalog_entries = vec![
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Telegram Bot API channel".to_string(),
                keywords: vec!["messaging".into(), "bot".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "channels-src/telegram".to_string(),
                    build_dir: Some("channels-src/telegram".to_string()),
                    crate_name: Some("telegram-channel".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
            // Two entries with same name but different kinds should coexist
            RegistryEntry {
                name: "dual-ext".to_string(),
                display_name: "Dual MCP".to_string(),
                kind: ExtensionKind::McpServer,
                description: "Dual extension MCP server".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::McpUrl {
                    url: "https://mcp.example.com".to_string(),
                },
                fallback_source: None,
                auth_hint: AuthHint::Dcr,
                version: None,
            },
            RegistryEntry {
                name: "dual-ext".to_string(),
                display_name: "Dual WASM".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Dual extension WASM tool".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "tools-src/dual".to_string(),
                    build_dir: Some("tools-src/dual".to_string()),
                    crate_name: Some("dual-tool".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        // Should find the new telegram entry
        let results = registry.search("telegram").await;
        assert!(!results.is_empty(), "Should find telegram from catalog");
        assert_eq!(results[0].entry.name, "telegram");

        // Should have both MCP and WASM entries with the same name
        let results = registry.search("dual-ext").await;
        let has_mcp = results
            .iter()
            .any(|r| r.entry.name == "dual-ext" && r.entry.kind == ExtensionKind::McpServer);
        let has_wasm = results
            .iter()
            .any(|r| r.entry.name == "dual-ext" && r.entry.kind == ExtensionKind::WasmTool);
        assert!(has_mcp, "Should have MCP dual-ext");
        assert!(has_wasm, "Should have WASM dual-ext");
    }

    #[tokio::test]
    async fn test_new_with_catalog_dedup_same_kind() {
        // When two catalog entries share name AND kind, only the first should be kept
        let catalog_entries = vec![
            RegistryEntry {
                name: "test-ext".to_string(),
                display_name: "Test First".to_string(),
                kind: ExtensionKind::McpServer,
                description: "First entry".to_string(),
                keywords: vec![],
                source: ExtensionSource::McpUrl {
                    url: "https://first.example.com".to_string(),
                },
                fallback_source: None,
                auth_hint: AuthHint::Dcr,
                version: None,
            },
            RegistryEntry {
                name: "test-ext".to_string(),
                display_name: "Test Duplicate".to_string(),
                kind: ExtensionKind::McpServer, // same kind
                description: "Should be skipped".to_string(),
                keywords: vec![],
                source: ExtensionSource::McpUrl {
                    url: "https://second.example.com".to_string(),
                },
                fallback_source: None,
                auth_hint: AuthHint::Dcr,
                version: None,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        let entry = registry.get("test-ext").await;
        assert!(entry.is_some());
        // Should be the first entry, not the duplicate
        assert_eq!(entry.unwrap().display_name, "Test First");
    }

    #[tokio::test]
    async fn test_get_with_kind_resolves_collision() {
        // Two entries with the same name but different kinds (the telegram collision scenario)
        let catalog_entries = vec![
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Tool".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Telegram MTProto tool".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "tools-src/telegram".to_string(),
                    build_dir: Some("tools-src/telegram".to_string()),
                    crate_name: Some("telegram-tool".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Channel".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Telegram Bot API channel".to_string(),
                keywords: vec!["messaging".into(), "bot".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "channels-src/telegram".to_string(),
                    build_dir: Some("channels-src/telegram".to_string()),
                    crate_name: Some("telegram-channel".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        // Without kind hint, get() returns the first match (WasmTool)
        let entry = registry.get("telegram").await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().kind, ExtensionKind::WasmTool);

        // With kind hint for WasmChannel, get_with_kind() returns the channel entry
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::WasmChannel))
            .await;
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmChannel);
        assert_eq!(entry.display_name, "Telegram Channel");

        // With kind hint for WasmTool, get_with_kind() returns the tool entry
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::WasmTool))
            .await;
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmTool);
        assert_eq!(entry.display_name, "Telegram Tool");

        // Without kind hint (None), get_with_kind() falls back to first match
        let entry = registry.get_with_kind("telegram", None).await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().kind, ExtensionKind::WasmTool);

        // Kind mismatch: no McpServer named "telegram" exists — must return None,
        // not silently fall back to the WasmTool entry.
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::McpServer))
            .await;
        assert!(
            entry.is_none(),
            "Should return None when kind doesn't match, not fall back to wrong kind"
        );
    }

    #[tokio::test]
    async fn test_get_with_kind_discovery_cache() {
        let registry = ExtensionRegistry::new();

        // Add two entries with the same name but different kinds to the discovery cache
        let tool_entry = RegistryEntry {
            name: "cached-ext".to_string(),
            display_name: "Cached Tool".to_string(),
            kind: ExtensionKind::WasmTool,
            description: "A cached tool".to_string(),
            keywords: vec![],
            source: ExtensionSource::WasmBuildable {
                source_dir: "tools-src/cached".to_string(),
                build_dir: None,
                crate_name: None,
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
            version: None,
        };
        let channel_entry = RegistryEntry {
            name: "cached-ext".to_string(),
            display_name: "Cached Channel".to_string(),
            kind: ExtensionKind::WasmChannel,
            description: "A cached channel".to_string(),
            keywords: vec![],
            source: ExtensionSource::WasmBuildable {
                source_dir: "channels-src/cached".to_string(),
                build_dir: None,
                crate_name: None,
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
            version: None,
        };

        registry
            .cache_discovered(vec![tool_entry, channel_entry])
            .await;

        // Kind-aware lookup should find the channel in the cache
        let entry = registry
            .get_with_kind("cached-ext", Some(ExtensionKind::WasmChannel))
            .await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Cached Channel");

        // Kind-aware lookup should find the tool in the cache
        let entry = registry
            .get_with_kind("cached-ext", Some(ExtensionKind::WasmTool))
            .await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Cached Tool");
    }

    // Channel tests (telegram, slack, discord, whatsapp) require the embedded catalog
    // to be loaded via new_with_catalog(). See test_new_with_catalog for catalog coverage.

    // === QA Plan P2 - 2.4: Extension registry collision tests ===

    #[tokio::test]
    async fn test_same_name_different_kind_both_discoverable() {
        // A WASM channel and WASM tool with the same name must coexist.
        let catalog_entries = vec![
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Channel".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Telegram messaging channel".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "channels-src/telegram".to_string(),
                    build_dir: None,
                    crate_name: None,
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Tool".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Telegram API tool".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "tools-src/telegram".to_string(),
                    build_dir: None,
                    crate_name: None,
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
                version: None,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);
        let all = registry.all_entries().await;

        // Both should exist since they have different kinds.
        let channel = all
            .iter()
            .find(|e| e.name == "telegram" && e.kind == ExtensionKind::WasmChannel);
        let tool = all
            .iter()
            .find(|e| e.name == "telegram" && e.kind == ExtensionKind::WasmTool);

        assert!(channel.is_some(), "Channel entry missing");
        assert!(tool.is_some(), "Tool entry missing");

        // Search should return both.
        let results = registry.search("telegram").await;
        let channel_hit = results
            .iter()
            .any(|r| r.entry.name == "telegram" && r.entry.kind == ExtensionKind::WasmChannel);
        let tool_hit = results
            .iter()
            .any(|r| r.entry.name == "telegram" && r.entry.kind == ExtensionKind::WasmTool);
        assert!(channel_hit, "Search should find channel");
        assert!(tool_hit, "Search should find tool");
    }

    #[tokio::test]
    async fn test_get_returns_first_match_regardless_of_kind() {
        // `get()` returns the first entry with a matching name. If a channel
        // and tool share a name, callers that need a specific kind should
        // filter by kind.
        let catalog_entries = vec![
            RegistryEntry {
                name: "myext".to_string(),
                display_name: "MyExt Channel".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Channel".to_string(),
                keywords: vec![],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "x".to_string(),
                    build_dir: None,
                    crate_name: None,
                },
                fallback_source: None,
                auth_hint: AuthHint::None,
                version: None,
            },
            RegistryEntry {
                name: "myext".to_string(),
                display_name: "MyExt Tool".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Tool".to_string(),
                keywords: vec![],
                source: ExtensionSource::WasmBuildable {
                    source_dir: "y".to_string(),
                    build_dir: None,
                    crate_name: None,
                },
                fallback_source: None,
                auth_hint: AuthHint::None,
                version: None,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        // get() is name-only, returns first match.
        let entry = registry.get("myext").await;
        assert!(entry.is_some());
        // The first catalog entry added is the channel.
        assert_eq!(entry.unwrap().kind, ExtensionKind::WasmChannel);
    }

    #[test]
    fn test_builtin_entries_with_relay_none_excludes_relay() {
        let entries = super::builtin_entries_with_relay(None);
        assert!(
            !entries
                .iter()
                .any(|e| e.kind == ExtensionKind::ChannelRelay),
            "No ChannelRelay entry when relay URL is None"
        );
    }

    #[test]
    fn test_builtin_entries_with_relay_some_includes_relay() {
        let entries =
            super::builtin_entries_with_relay(Some("http://relay.example.com".to_string()));
        let relay = entries
            .iter()
            .find(|e| e.kind == ExtensionKind::ChannelRelay);
        assert!(relay.is_some(), "ChannelRelay entry should be present");
        if let ExtensionSource::ChannelRelay { relay_url } = &relay.unwrap().source {
            assert_eq!(relay_url, "http://relay.example.com");
        } else {
            panic!("Expected ChannelRelay source");
        }
    }
}
