//! Curated in-memory catalog of known extensions with fuzzy search.
//!
//! The registry holds well-known MCP servers and WASM tools that can be installed
//! via conversational commands. Online discoveries are cached here too.

use tokio::sync::RwLock;

use crate::extensions::{
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
            entries: builtin_entries(),
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

        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(r, _)| r).collect()
    }

    /// Look up an entry by exact name.
    pub async fn get(&self, name: &str) -> Option<RegistryEntry> {
        if let Some(entry) = self.entries.iter().find(|e| e.name == name) {
            return Some(entry.clone());
        }
        let cache = self.discovery_cache.read().await;
        cache.iter().find(|e| e.name == name).cloned()
    }

    /// Add discovered entries to the cache.
    pub async fn cache_discovered(&self, entries: Vec<RegistryEntry>) {
        let mut cache = self.discovery_cache.write().await;
        for entry in entries {
            // Deduplicate by name
            if !cache.iter().any(|e| e.name == entry.name) {
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
fn builtin_entries() -> Vec<RegistryEntry> {
    vec![
        // -- MCP Servers --
        RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Notion for reading and writing pages, databases, and comments"
                .to_string(),
            keywords: vec![
                "notes".into(),
                "wiki".into(),
                "docs".into(),
                "pages".into(),
                "database".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.notion.com/mcp".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "linear".to_string(),
            display_name: "Linear".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Linear for issue tracking, project management, and team workflows"
                    .to_string(),
            keywords: vec![
                "issues".into(),
                "tickets".into(),
                "project".into(),
                "tracking".into(),
                "bugs".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.linear.app".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "google-calendar".to_string(),
            display_name: "Google Calendar".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Google Calendar for managing events, schedules, and reminders"
                .to_string(),
            keywords: vec![
                "calendar".into(),
                "events".into(),
                "schedule".into(),
                "meetings".into(),
                "google".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.google.com/calendar".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "google-drive".to_string(),
            display_name: "Google Drive".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Google Drive for file management, search, and document access"
                .to_string(),
            keywords: vec![
                "drive".into(),
                "files".into(),
                "documents".into(),
                "storage".into(),
                "google".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.google.com/drive".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "github".to_string(),
            display_name: "GitHub".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to GitHub for repository management, issues, PRs, and code search"
                    .to_string(),
            keywords: vec![
                "git".into(),
                "repos".into(),
                "code".into(),
                "pull-request".into(),
                "issues".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.github.com".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "slack".to_string(),
            display_name: "Slack".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Slack for messaging, channel management, and team communication"
                    .to_string(),
            keywords: vec![
                "messaging".into(),
                "chat".into(),
                "channels".into(),
                "team".into(),
                "communication".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.slack.com".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "sentry".to_string(),
            display_name: "Sentry".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Sentry for error tracking, performance monitoring, and debugging"
                    .to_string(),
            keywords: vec![
                "errors".into(),
                "monitoring".into(),
                "debugging".into(),
                "crashes".into(),
                "performance".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.sentry.dev/sse".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "stripe".to_string(),
            display_name: "Stripe".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Stripe for payment processing, subscriptions, and financial data"
                    .to_string(),
            keywords: vec![
                "payments".into(),
                "billing".into(),
                "subscriptions".into(),
                "invoices".into(),
                "finance".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.stripe.com".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "cloudflare".to_string(),
            display_name: "Cloudflare".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Cloudflare for DNS, Workers, KV, and infrastructure management"
                    .to_string(),
            keywords: vec![
                "cdn".into(),
                "dns".into(),
                "workers".into(),
                "hosting".into(),
                "infrastructure".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.cloudflare.com/sse".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "asana".to_string(),
            display_name: "Asana".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Asana for task management, projects, and team coordination"
                .to_string(),
            keywords: vec![
                "tasks".into(),
                "projects".into(),
                "management".into(),
                "team".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.asana.com".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "intercom".to_string(),
            display_name: "Intercom".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Intercom for customer messaging, support, and engagement"
                .to_string(),
            keywords: vec![
                "support".into(),
                "customers".into(),
                "messaging".into(),
                "chat".into(),
                "helpdesk".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.intercom.com".to_string(),
            },
            auth_hint: AuthHint::Dcr,
        },
    ]
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
            auth_hint: AuthHint::Dcr,
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
            auth_hint: AuthHint::Dcr,
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
            auth_hint: AuthHint::Dcr,
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
            auth_hint: AuthHint::Dcr,
        };

        let score = score_entry(&entry, &["xyzfoobar".to_string()]);
        assert_eq!(score, 0, "No match should score 0");
    }

    #[tokio::test]
    async fn test_search_returns_sorted() {
        let registry = ExtensionRegistry::new();
        let results = registry.search("notion").await;

        assert!(!results.is_empty(), "Should find notion in registry");
        assert_eq!(results[0].entry.name, "notion");
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_all() {
        let registry = ExtensionRegistry::new();
        let results = registry.search("").await;

        assert!(results.len() > 5, "Empty query should return all entries");
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let registry = ExtensionRegistry::new();
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
        let registry = ExtensionRegistry::new();

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
            auth_hint: AuthHint::Dcr,
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
            auth_hint: AuthHint::None,
        };

        registry.cache_discovered(vec![entry.clone()]).await;
        registry.cache_discovered(vec![entry]).await;

        let results = registry.search("dup").await;
        assert_eq!(results.len(), 1, "Should not duplicate cached entries");
    }
}
