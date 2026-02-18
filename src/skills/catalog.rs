//! Runtime skill catalog backed by ClawHub's public registry.
//!
//! Fetches skill listings from the ClawHub API (`/api/v1/search`) at runtime,
//! caching results in memory. No compile-time entries -- the catalog is always
//! up-to-date with the registry.
//!
//! Configuration:
//! - `CLAWHUB_REGISTRY` env var overrides the default base URL (`https://clawhub.ai`)

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Default ClawHub registry URL.
const DEFAULT_REGISTRY_URL: &str = "https://clawhub.ai";

/// How long cached search results remain valid (5 minutes).
const CACHE_TTL: Duration = Duration::from_secs(300);

/// Maximum number of results to return from a search.
const MAX_RESULTS: usize = 25;

/// HTTP request timeout for catalog queries.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A skill entry from the ClawHub catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Skill slug (unique identifier, e.g. "owner/skill-name").
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Short description.
    #[serde(default)]
    pub description: String,
    /// Skill version (semver).
    #[serde(default)]
    pub version: String,
    /// Relevance score from the search API.
    #[serde(default)]
    pub score: f64,
}

/// Cached search result with TTL.
struct CachedSearch {
    query: String,
    results: Vec<CatalogEntry>,
    fetched_at: Instant,
}

/// Runtime skill catalog that queries ClawHub's API.
pub struct SkillCatalog {
    /// Base URL for the registry (e.g. `https://clawhub.ai`).
    registry_url: String,
    /// HTTP client (reused across requests).
    client: reqwest::Client,
    /// In-memory search cache keyed by query string.
    cache: RwLock<Vec<CachedSearch>>,
}

impl SkillCatalog {
    /// Create a new catalog.
    ///
    /// Reads `CLAWHUB_REGISTRY` (or legacy `CLAWDHUB_REGISTRY`) from the
    /// environment, falling back to `https://clawhub.ai`.
    pub fn new() -> Self {
        let registry_url = std::env::var("CLAWHUB_REGISTRY")
            .or_else(|_| std::env::var("CLAWDHUB_REGISTRY"))
            .unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string());

        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("ironclaw/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();

        Self {
            registry_url,
            client,
            cache: RwLock::new(Vec::new()),
        }
    }

    /// Create a catalog with a custom registry URL (for testing).
    #[cfg(test)]
    pub fn with_url(url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("ironclaw/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();

        Self {
            registry_url: url.to_string(),
            client,
            cache: RwLock::new(Vec::new()),
        }
    }

    /// Search for skills in the catalog.
    ///
    /// First checks the in-memory cache. If not cached or expired, fetches
    /// from the ClawHub API. Returns an empty Vec on network errors (catalog
    /// search is best-effort, never blocks the agent).
    pub async fn search(&self, query: &str) -> Vec<CatalogEntry> {
        let query_lower = query.to_lowercase();

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.iter().find(|c| c.query == query_lower)
                && cached.fetched_at.elapsed() < CACHE_TTL
            {
                return cached.results.clone();
            }
        }

        // Fetch from API
        let results = self.fetch_search(&query_lower).await;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            // Remove stale entry for this query
            cache.retain(|c| c.query != query_lower);
            // Limit cache size to prevent unbounded growth
            if cache.len() >= 50 {
                cache.remove(0);
            }
            cache.push(CachedSearch {
                query: query_lower,
                results: results.clone(),
                fetched_at: Instant::now(),
            });
        }

        results
    }

    /// Fetch search results from the ClawHub API.
    async fn fetch_search(&self, query: &str) -> Vec<CatalogEntry> {
        let url = format!("{}/api/v1/search", self.registry_url);

        let response = match self.client.get(&url).query(&[("q", query)]).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!("Catalog search failed (network): {}", e);
                return Vec::new();
            }
        };

        if !response.status().is_success() {
            tracing::debug!(
                "Catalog search returned status {}: {}",
                response.status(),
                response
                    .text()
                    .await
                    .unwrap_or_else(|_| "(no body)".to_string())
            );
            return Vec::new();
        }

        // Parse the response -- ClawHub returns an array of results.
        // We try the v1 format first (with slug, displayName, version, score),
        // then fall back to a simpler format.
        match response.json::<Vec<CatalogSearchResult>>().await {
            Ok(results) => results
                .into_iter()
                .take(MAX_RESULTS)
                .map(|r| CatalogEntry {
                    slug: r.slug,
                    name: r.display_name.unwrap_or_default(),
                    description: r.summary.unwrap_or_default(),
                    version: r.version.unwrap_or_default(),
                    score: r.score.unwrap_or(0.0),
                })
                .collect(),
            Err(e) => {
                tracing::debug!("Catalog search: failed to parse response: {}", e);
                Vec::new()
            }
        }
    }

    /// Get the registry base URL.
    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }

    /// Clear the search cache.
    pub async fn clear_cache(&self) {
        self.cache.write().await.clear();
    }
}

impl Default for SkillCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal type matching ClawHub's `/api/v1/search` response items.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogSearchResult {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    score: Option<f64>,
}

/// Construct the download URL for a skill's SKILL.md from the registry.
///
/// The slug is URL-encoded to prevent query string injection via special
/// characters like `&` or `#`.
pub fn skill_download_url(registry_url: &str, slug: &str) -> String {
    format!(
        "{}/api/v1/download?slug={}",
        registry_url,
        urlencoding::encode(slug)
    )
}

/// Convenience wrapper for creating a shared catalog.
pub fn shared_catalog() -> Arc<SkillCatalog> {
    Arc::new(SkillCatalog::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_url() {
        // When CLAWHUB_REGISTRY is not set, should use default
        let catalog = SkillCatalog::with_url(DEFAULT_REGISTRY_URL);
        assert_eq!(catalog.registry_url(), DEFAULT_REGISTRY_URL);
    }

    #[test]
    fn test_custom_registry_url() {
        let catalog = SkillCatalog::with_url("https://custom.registry.example");
        assert_eq!(catalog.registry_url(), "https://custom.registry.example");
    }

    #[tokio::test]
    async fn test_search_returns_empty_on_network_error() {
        // Point at an invalid URL to trigger a network error
        let catalog = SkillCatalog::with_url("http://127.0.0.1:1");
        let results = catalog.search("test").await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_cache_is_populated_after_search() {
        let catalog = SkillCatalog::with_url("http://127.0.0.1:1");

        // First search populates cache (even with empty results)
        catalog.search("cached-query").await;

        let cache = catalog.cache.read().await;
        assert!(cache.iter().any(|c| c.query == "cached-query"));
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let catalog = SkillCatalog::with_url("http://127.0.0.1:1");
        catalog.search("something").await;

        catalog.clear_cache().await;
        let cache = catalog.cache.read().await;
        assert!(cache.is_empty());
    }

    #[test]
    fn test_skill_download_url() {
        let url = skill_download_url("https://clawhub.ai", "owner/my-skill");
        assert_eq!(
            url,
            "https://clawhub.ai/api/v1/download?slug=owner%2Fmy-skill"
        );
    }

    #[test]
    fn test_skill_download_url_encodes_special_chars() {
        let url = skill_download_url("https://clawhub.ai", "foo&bar=baz#frag");
        assert!(url.contains("slug=foo%26bar%3Dbaz%23frag"));
    }

    #[test]
    fn test_catalog_entry_serde() {
        let entry = CatalogEntry {
            slug: "test/skill".to_string(),
            name: "Test Skill".to_string(),
            description: "A test".to_string(),
            version: "1.0.0".to_string(),
            score: 0.95,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.slug, "test/skill");
        assert_eq!(parsed.name, "Test Skill");
    }
}
