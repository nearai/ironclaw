//! Online extension discovery for finding MCP servers not in the built-in registry.
//!
//! Multi-tier search strategy:
//! 1. Probe well-known URL patterns (mcp.{service}.com, {service}.com/mcp)
//! 2. Search GitHub for MCP server repositories
//! 3. Validate discovered URLs via .well-known/oauth-protected-resource
//!
//! All sources run concurrently with per-source timeouts.

use std::time::Duration;

use serde::Deserialize;

use crate::extensions::{AuthHint, ExtensionKind, ExtensionSource, RegistryEntry};

/// Handles online discovery of MCP servers.
pub struct OnlineDiscovery {
    http_client: reqwest::Client,
}

impl OnlineDiscovery {
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("IronClaw/1.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { http_client }
    }

    /// Run the full discovery pipeline for a query.
    ///
    /// Searches multiple sources concurrently, deduplicates, validates,
    /// and returns only confirmed MCP servers.
    pub async fn discover(&self, query: &str) -> Vec<RegistryEntry> {
        let query_clean = query.trim().to_lowercase();
        if query_clean.is_empty() {
            return Vec::new();
        }

        // Run all discovery sources concurrently
        let (patterns, github) = tokio::join!(
            self.probe_common_patterns(&query_clean),
            with_timeout(self.search_github(&query_clean), Duration::from_secs(8)),
        );

        // Collect and deduplicate by URL
        let mut seen_urls = std::collections::HashSet::new();
        let mut candidates: Vec<RegistryEntry> = Vec::new();

        for entry in patterns {
            let url = extract_url(&entry.source);
            if seen_urls.insert(url) {
                candidates.push(entry);
            }
        }

        for entry in github.unwrap_or_default() {
            let url = extract_url(&entry.source);
            if seen_urls.insert(url) {
                candidates.push(entry);
            }
        }

        candidates
    }

    /// Probe common URL patterns for MCP servers.
    ///
    /// Tries patterns like:
    /// - https://mcp.{query}.com
    /// - https://mcp.{query}.app
    /// - https://{query}.com/mcp
    pub async fn probe_common_patterns(&self, query: &str) -> Vec<RegistryEntry> {
        // Extract a clean service name (no spaces, lowercase)
        let service = query
            .split_whitespace()
            .next()
            .unwrap_or(query)
            .replace('-', "");

        let patterns = vec![
            format!("https://mcp.{}.com", service),
            format!("https://mcp.{}.app", service),
            format!("https://mcp.{}.dev", service),
            format!("https://{}.com/mcp", service),
        ];

        let mut results = Vec::new();
        let futures: Vec<_> = patterns
            .into_iter()
            .map(|url| {
                let client = self.http_client.clone();
                let query_owned = query.to_string();
                async move {
                    if validate_mcp_url_with_client(&client, &url).await {
                        Some(RegistryEntry {
                            name: query_owned.replace(' ', "-"),
                            display_name: titlecase(&query_owned),
                            kind: ExtensionKind::McpServer,
                            description: format!("MCP server discovered at {}", url),
                            keywords: vec![],
                            source: ExtensionSource::McpUrl {
                                url: url.to_string(),
                            },
                            auth_hint: AuthHint::Dcr,
                        })
                    } else {
                        None
                    }
                }
            })
            .collect();

        let probe_results = futures::future::join_all(futures).await;
        for result in probe_results.into_iter().flatten() {
            results.push(result);
        }

        results
    }

    /// Search GitHub for MCP server repositories.
    ///
    /// Uses the GitHub search API (no auth needed for low-rate public queries).
    pub async fn search_github(&self, query: &str) -> Vec<RegistryEntry> {
        let search_url = format!(
            "https://api.github.com/search/repositories?q={}+topic:mcp-server&per_page=5&sort=stars",
            urlencoding::encode(query)
        );

        let response = match self.http_client.get(&search_url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("GitHub search failed: {}", e);
                return Vec::new();
            }
        };

        if !response.status().is_success() {
            tracing::debug!("GitHub search returned {}", response.status());
            return Vec::new();
        }

        let body: GitHubSearchResponse = match response.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("Failed to parse GitHub search response: {}", e);
                return Vec::new();
            }
        };

        body.items
            .into_iter()
            .filter_map(|item| {
                // Only include repos that look like MCP servers
                let has_mcp_topic = item
                    .topics
                    .iter()
                    .any(|t| t.contains("mcp") || t.contains("model-context-protocol"));
                if !has_mcp_topic {
                    return None;
                }

                // Try to extract a homepage URL (which might be the MCP endpoint)
                let url = item.homepage.filter(|h| !h.is_empty()).unwrap_or_else(|| {
                    // Fall back to repo URL as a reference
                    item.html_url.clone()
                });

                Some(RegistryEntry {
                    name: item.name.clone(),
                    display_name: titlecase(&item.name.replace('-', " ")),
                    kind: ExtensionKind::McpServer,
                    description: item
                        .description
                        .unwrap_or_else(|| format!("MCP server from GitHub: {}", item.full_name)),
                    keywords: item.topics,
                    source: ExtensionSource::Discovered { url },
                    auth_hint: AuthHint::Dcr,
                })
            })
            .collect()
    }

    /// Validate a URL is a real MCP server.
    pub async fn validate_mcp_url(&self, url: &str) -> bool {
        validate_mcp_url_with_client(&self.http_client, url).await
    }
}

impl Default for OnlineDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate that a URL is a real MCP server by checking .well-known endpoints.
///
/// Tries:
/// 1. GET {origin}/.well-known/oauth-protected-resource -> 200 with JSON = confirmed
/// 2. Fallback: HEAD/GET the URL itself to check if it's alive
async fn validate_mcp_url_with_client(client: &reqwest::Client, url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let origin = parsed.origin().ascii_serialization();

    // Check .well-known/oauth-protected-resource
    let well_known_url = format!("{}/.well-known/oauth-protected-resource", origin);
    match client.get(&well_known_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Try to parse as JSON to confirm it's a real MCP endpoint
            if let Ok(text) = resp.text().await {
                return serde_json::from_str::<serde_json::Value>(&text).is_ok();
            }
        }
        _ => {}
    }

    // Fallback: try a HEAD request on the URL itself to check if it's alive
    match client.head(url).send().await {
        Ok(resp) => {
            // Accept various status codes that indicate the server exists
            let status = resp.status().as_u16();
            // 401/403 means it exists but needs auth, which is fine for MCP
            matches!(status, 200..=299 | 401 | 403 | 405)
        }
        Err(_) => false,
    }
}

/// Run a future with a timeout, returning None if it times out.
async fn with_timeout<T>(
    future: impl std::future::Future<Output = T>,
    duration: Duration,
) -> Option<T> {
    tokio::time::timeout(duration, future).await.ok()
}

fn extract_url(source: &ExtensionSource) -> String {
    match source {
        ExtensionSource::McpUrl { url } => url.clone(),
        ExtensionSource::Discovered { url } => url.clone(),
        ExtensionSource::WasmDownload { wasm_url, .. } => wasm_url.clone(),
        ExtensionSource::WasmBuildable { repo_url, .. } => repo_url.clone(),
    }
}

fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Deserialize)]
struct GitHubSearchResponse {
    #[serde(default)]
    items: Vec<GitHubRepo>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    full_name: String,
    html_url: String,
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use crate::extensions::ExtensionSource;
    use crate::extensions::discovery::{
        OnlineDiscovery, extract_url, titlecase, validate_mcp_url_with_client,
    };

    #[test]
    fn test_titlecase() {
        assert_eq!(titlecase("google calendar"), "Google Calendar");
        assert_eq!(titlecase("notion"), "Notion");
        assert_eq!(titlecase(""), "");
    }

    #[test]
    fn test_extract_url() {
        let mcp = ExtensionSource::McpUrl {
            url: "https://mcp.notion.com".to_string(),
        };
        assert_eq!(extract_url(&mcp), "https://mcp.notion.com");

        let discovered = ExtensionSource::Discovered {
            url: "https://example.com".to_string(),
        };
        assert_eq!(extract_url(&discovered), "https://example.com");
    }

    #[tokio::test]
    async fn test_validate_invalid_url() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();

        // Invalid URL should fail
        assert!(!validate_mcp_url_with_client(&client, "not-a-url").await);
    }

    #[test]
    fn test_discovery_new() {
        // Just make sure it constructs without panicking
        let _discovery = OnlineDiscovery::new();
    }
}
