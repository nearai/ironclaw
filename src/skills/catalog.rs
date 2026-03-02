//! Runtime skill catalog backed by ClawHub's public registry.
//!
//! Fetches skill listings from the ClawHub API (`/api/v1/search`) at runtime,
//! caching results in memory. No compile-time entries -- the catalog is always
//! up-to-date with the registry.
//!
//! Configuration:
//! - `CLAWHUB_REGISTRY` env var overrides the default base URL

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Default ClawHub registry URL.
///
/// Points directly at the Convex backend, bypassing Vercel's edge which
/// rejects non-browser TLS fingerprints (JA3/JA4 filtering).
const DEFAULT_REGISTRY_URL: &str = "https://wry-manatee-359.convex.site";

/// How long cached search results remain valid (5 minutes).
const CACHE_TTL: Duration = Duration::from_secs(300);

/// Maximum number of results to return from a search.
const MAX_RESULTS: usize = 25;

/// HTTP request timeout for catalog queries.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Result of a catalog search, carrying both results and any error that occurred.
#[derive(Debug, Clone)]
pub struct CatalogSearchOutcome {
    /// Skill entries returned by the search (empty on error).
    pub results: Vec<CatalogEntry>,
    /// If the registry was unreachable or returned an error, a human-readable message.
    pub error: Option<String>,
}

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
    /// Last updated timestamp (epoch milliseconds from registry).
    #[serde(default)]
    pub updated_at: Option<u64>,
    /// Star count (populated via detail enrichment).
    #[serde(default)]
    pub stars: Option<u64>,
    /// Total download count (populated via detail enrichment).
    #[serde(default)]
    pub downloads: Option<u64>,
    /// Current install count (populated via detail enrichment).
    #[serde(default)]
    pub installs_current: Option<u64>,
    /// Owner handle (populated via detail enrichment).
    #[serde(default)]
    pub owner: Option<String>,
}

/// Top-level wrapper from the ClawHub `/api/v1/skills/{slug}` response.
///
/// The API returns `{"skill": {...}, "owner": {...}, "latestVersion": {...}}`.
#[derive(Debug, Clone, Deserialize)]
struct SkillDetailResponse {
    skill: SkillDetailInner,
    #[serde(default)]
    owner: Option<SkillOwner>,
}

/// Inner `skill` object within `SkillDetailResponse`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillDetailInner {
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub stats: Option<SkillStats>,
    #[serde(default)]
    pub updated_at: Option<u64>,
}

/// Detailed skill information from the ClawHub `/api/v1/skills/{slug}` endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub stats: Option<SkillStats>,
    #[serde(default)]
    pub owner: Option<SkillOwner>,
    #[serde(default)]
    pub updated_at: Option<u64>,
}

/// Statistics for a skill from the ClawHub detail endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStats {
    #[serde(default)]
    pub stars: Option<u64>,
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub installs_current: Option<u64>,
    #[serde(default)]
    pub installs_all_time: Option<u64>,
    #[serde(default)]
    pub versions: Option<u64>,
}

/// Owner information for a skill.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillOwner {
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
}

/// Cached search result with TTL.
struct CachedSearch {
    query: String,
    outcome: CatalogSearchOutcome,
    fetched_at: Instant,
}

/// Runtime skill catalog that queries ClawHub's API.
pub struct SkillCatalog {
    /// Base URL for the registry.
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
    /// environment, falling back to the Convex backend.
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
    /// from the ClawHub API. Returns a [`CatalogSearchOutcome`] that carries
    /// both results and any error that occurred (catalog search is best-effort,
    /// never blocks the agent).
    pub async fn search(&self, query: &str) -> CatalogSearchOutcome {
        let query_lower = query.to_lowercase();

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.iter().find(|c| c.query == query_lower)
                && cached.fetched_at.elapsed() < CACHE_TTL
            {
                return cached.outcome.clone();
            }
        }

        // Fetch from API
        let outcome = self.fetch_search(&query_lower).await;

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
                outcome: outcome.clone(),
                fetched_at: Instant::now(),
            });
        }

        outcome
    }

    /// Fetch search results from the ClawHub API.
    async fn fetch_search(&self, query: &str) -> CatalogSearchOutcome {
        let url = format!("{}/api/v1/search", self.registry_url);

        let response = match self.client.get(&url).query(&[("q", query)]).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Catalog search failed (network): {}", e);
                return CatalogSearchOutcome {
                    results: Vec::new(),
                    error: Some("Registry unreachable".to_string()),
                };
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            tracing::debug!(
                "Catalog search returned status {}: {}",
                status,
                response
                    .text()
                    .await
                    .unwrap_or_else(|_| "(no body)".to_string())
            );
            return CatalogSearchOutcome {
                results: Vec::new(),
                error: Some(format!("Registry returned status {status}")),
            };
        }

        // Parse the response body as text first so we can try multiple formats.
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("Catalog search: failed to read response body: {}", e);
                return CatalogSearchOutcome {
                    results: Vec::new(),
                    error: Some("Failed to read registry response".to_string()),
                };
            }
        };

        // Try wrapped format first: {"results": [...]}
        // Then fall back to bare array: [...]
        let raw_results = if let Ok(envelope) = serde_json::from_str::<CatalogSearchEnvelope>(&body)
        {
            envelope.results
        } else if let Ok(arr) = serde_json::from_str::<Vec<CatalogSearchResult>>(&body) {
            arr
        } else {
            let preview = body.get(..200).unwrap_or(&body);
            tracing::debug!("Catalog search: failed to parse response: {}", preview);
            return CatalogSearchOutcome {
                results: Vec::new(),
                error: Some("Invalid response from registry".to_string()),
            };
        };

        CatalogSearchOutcome {
            results: raw_results
                .into_iter()
                .take(MAX_RESULTS)
                .map(|r| CatalogEntry {
                    slug: r.slug,
                    name: r.display_name.unwrap_or_default(),
                    description: r.summary.unwrap_or_default(),
                    version: r.version.unwrap_or_default(),
                    score: r.score.unwrap_or(0.0),
                    updated_at: r.updated_at,
                    stars: None,
                    downloads: None,
                    installs_current: None,
                    owner: None,
                })
                .collect(),
            error: None,
        }
    }

    /// Fetch detailed information for a single skill by slug.
    ///
    /// Calls `GET /api/v1/skills/{slug}` and returns the detail if available.
    /// Returns `None` on any network or parse error (best-effort).
    pub async fn fetch_skill_detail(&self, slug: &str) -> Option<SkillDetail> {
        let url = format!(
            "{}/api/v1/skills/{}",
            self.registry_url,
            urlencoding::encode(slug)
        );

        let response = self.client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            tracing::debug!(
                "Skill detail for '{}' returned status {}",
                slug,
                response.status()
            );
            return None;
        }

        let wrapper = response.json::<SkillDetailResponse>().await.ok()?;
        let inner = wrapper.skill;
        Some(SkillDetail {
            slug: inner.slug,
            display_name: inner.display_name,
            summary: inner.summary,
            version: None, // not returned in detail response
            stats: inner.stats,
            owner: wrapper.owner,
            updated_at: inner.updated_at,
        })
    }

    /// Enrich catalog entries with detail data (stars, downloads, owner).
    ///
    /// Fetches detail for up to `max` entries in parallel. Best-effort: entries
    /// that fail to enrich keep their `None` values.
    pub async fn enrich_search_results(&self, entries: &mut [CatalogEntry], max: usize) {
        let count = entries.len().min(max);
        if count == 0 {
            return;
        }

        let futures: Vec<_> = entries[..count]
            .iter()
            .map(|e| self.fetch_skill_detail(&e.slug))
            .collect();

        let details = futures::future::join_all(futures).await;

        for (entry, detail) in entries[..count].iter_mut().zip(details.into_iter()) {
            if let Some(detail) = detail {
                if let Some(ref stats) = detail.stats {
                    entry.stars = stats.stars;
                    entry.downloads = stats.downloads;
                    entry.installs_current = stats.installs_current;
                }
                if let Some(ref owner) = detail.owner {
                    entry.owner = owner.handle.clone().or_else(|| owner.display_name.clone());
                }
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

/// Wrapper for ClawHub's `{"results": [...]}` envelope.
#[derive(Debug, Deserialize)]
struct CatalogSearchEnvelope {
    results: Vec<CatalogSearchResult>,
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
    #[serde(default)]
    updated_at: Option<u64>,
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
    async fn test_search_returns_error_on_network_failure() {
        // Point at an invalid URL to trigger a network error.
        // Note: with some proxies (e.g., system HTTP proxies), connection may succeed
        // but return an error status like 502/503/504.
        let catalog = SkillCatalog::with_url("http://192.0.2.1:9999");
        let outcome = catalog.search("test").await;
        assert!(outcome.results.is_empty());
        assert!(outcome.error.is_some());
        // Error should indicate a connection problem or bad gateway response
        let error = outcome.error.unwrap();
        assert!(
            error.contains("Registry unreachable") ||
            error.contains("connect") ||
            error.contains("502") ||
            error.contains("503") ||
            error.contains("504"),
            "Expected connection error, got: {}",
            error
        );
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
    fn test_parse_wrapped_response() {
        // ClawHub returns {"results": [...]} format
        let json = r#"{"results":[{"slug":"markdown","displayName":"Markdown","summary":"A skill","version":"1.0.0","score":3.5}]}"#;
        let envelope: CatalogSearchEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.results.len(), 1);
        assert_eq!(envelope.results[0].slug, "markdown");
        assert_eq!(
            envelope.results[0].display_name.as_deref(),
            Some("Markdown")
        );
    }

    #[test]
    fn test_parse_bare_array_response() {
        // Fallback: bare array format
        let json = r#"[{"slug":"markdown","displayName":"Markdown","summary":"A skill","version":"1.0.0","score":3.5}]"#;
        let results: Vec<CatalogSearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "markdown");
    }

    #[test]
    fn test_parse_skill_detail() {
        // Response format matches the actual ClawHub API: {"skill": {...}, "owner": {...}}
        let json = r#"{
            "skill": {
                "slug": "steipete/markdown-writer",
                "displayName": "Markdown Writer",
                "summary": "Write markdown docs",
                "stats": {
                    "stars": 142,
                    "downloads": 8400,
                    "installsCurrent": 55,
                    "installsAllTime": 200,
                    "versions": 5
                },
                "updatedAt": 1700000000000
            },
            "owner": {
                "handle": "steipete",
                "displayName": "Peter S."
            },
            "latestVersion": {
                "version": "1.2.3",
                "createdAt": 1700000000000,
                "changelog": ""
            }
        }"#;

        let wrapper: SkillDetailResponse = serde_json::from_str(json).unwrap();
        let inner = &wrapper.skill;
        assert_eq!(inner.slug, "steipete/markdown-writer");
        assert_eq!(inner.display_name.as_deref(), Some("Markdown Writer"));

        let stats = inner.stats.as_ref().unwrap();
        assert_eq!(stats.stars, Some(142));
        assert_eq!(stats.downloads, Some(8400));
        assert_eq!(stats.installs_current, Some(55));

        let owner = wrapper.owner.as_ref().unwrap();
        assert_eq!(owner.handle.as_deref(), Some("steipete"));
    }

    #[tokio::test]
    async fn test_fetch_skill_detail_returns_none_on_error() {
        let catalog = SkillCatalog::with_url("http://127.0.0.1:1");
        let result = catalog.fetch_skill_detail("nonexistent/skill").await;
        assert!(result.is_none());
    }

    #[test]
    fn test_catalog_entry_serde() {
        let entry = CatalogEntry {
            slug: "test/skill".to_string(),
            name: "Test Skill".to_string(),
            description: "A test".to_string(),
            version: "1.0.0".to_string(),
            score: 0.95,
            updated_at: Some(1700000000000),
            stars: Some(42),
            downloads: Some(1000),
            installs_current: None,
            owner: Some("tester".to_string()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: CatalogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.slug, "test/skill");
        assert_eq!(parsed.name, "Test Skill");
    }
}
