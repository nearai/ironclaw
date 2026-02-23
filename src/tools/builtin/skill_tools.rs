//! Agent-callable tools for managing skills (prompt-level extensions).
//!
//! Four tools for discovering, installing, listing, and removing skills
//! entirely through conversation, following the extension_tools pattern.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::skills::catalog::SkillCatalog;
use crate::skills::registry::SkillRegistry;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

// ── skill_list ──────────────────────────────────────────────────────────

pub struct SkillListTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
}

impl SkillListTool {
    pub fn new(registry: Arc<std::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "verbose": {
                    "type": "boolean",
                    "description": "Include extra detail (tags, content_hash, version)",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let verbose = params
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let guard = self
            .registry
            .read()
            .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;

        let skills: Vec<serde_json::Value> = guard
            .skills()
            .iter()
            .map(|s| {
                let mut entry = serde_json::json!({
                    "name": s.manifest.name,
                    "description": s.manifest.description,
                    "trust": s.trust.to_string(),
                    "source": format!("{:?}", s.source),
                    "keywords": s.manifest.activation.keywords,
                });

                if verbose && let Some(obj) = entry.as_object_mut() {
                    obj.insert(
                        "version".to_string(),
                        serde_json::Value::String(s.manifest.version.clone()),
                    );
                    obj.insert(
                        "tags".to_string(),
                        serde_json::json!(s.manifest.activation.tags),
                    );
                    obj.insert(
                        "content_hash".to_string(),
                        serde_json::Value::String(s.content_hash.clone()),
                    );
                    obj.insert(
                        "max_context_tokens".to_string(),
                        serde_json::json!(s.manifest.activation.max_context_tokens),
                    );
                }

                entry
            })
            .collect();

        let output = serde_json::json!({
            "skills": skills,
            "count": skills.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_search ────────────────────────────────────────────────────────

pub struct SkillSearchTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
}

impl SkillSearchTool {
    pub fn new(
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
    ) -> Self {
        Self { registry, catalog }
    }
}

#[async_trait]
impl Tool for SkillSearchTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (name, keyword, or description fragment)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;

        // Search the ClawHub catalog (async, best-effort)
        let catalog_outcome = self.catalog.search(query).await;
        let catalog_error = catalog_outcome.error.clone();

        // Enrich top results with detail data (stars, downloads, owner)
        let mut catalog_entries = catalog_outcome.results;
        self.catalog
            .enrich_search_results(&mut catalog_entries, 5)
            .await;

        // Search locally loaded skills
        let installed_names: Vec<String> = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .skills()
                .iter()
                .map(|s| s.manifest.name.clone())
                .collect()
        };

        // Mark catalog entries that are already installed
        let catalog_json: Vec<serde_json::Value> = catalog_entries
            .iter()
            .map(|entry| {
                let is_installed = installed_names.iter().any(|n| {
                    // Match by slug suffix or exact name
                    entry.slug.ends_with(n.as_str()) || entry.name == *n
                });
                serde_json::json!({
                    "slug": entry.slug,
                    "name": entry.name,
                    "description": entry.description,
                    "version": entry.version,
                    "score": entry.score,
                    "installed": is_installed,
                    "stars": entry.stars,
                    "downloads": entry.downloads,
                    "owner": entry.owner,
                })
            })
            .collect();

        // Find matching local skills (simple substring match)
        let query_lower = query.to_lowercase();
        let local_matches: Vec<serde_json::Value> = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                        || s.manifest
                            .activation
                            .keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&query_lower))
                })
                .map(|s| {
                    serde_json::json!({
                        "name": s.manifest.name,
                        "description": s.manifest.description,
                        "trust": s.trust.to_string(),
                    })
                })
                .collect()
        };

        let mut output = serde_json::json!({
            "catalog": catalog_json,
            "catalog_count": catalog_json.len(),
            "installed": local_matches,
            "installed_count": local_matches.len(),
            "registry_url": self.catalog.registry_url(),
        });
        if let Some(err) = catalog_error {
            output["catalog_error"] = serde_json::Value::String(err);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_install ───────────────────────────────────────────────────────

pub struct SkillInstallTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
}

impl SkillInstallTool {
    pub fn new(
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
    ) -> Self {
        Self { registry, catalog }
    }
}

#[async_trait]
impl Tool for SkillInstallTool {
    fn name(&self) -> &str {
        "skill_install"
    }

    fn description(&self) -> &str {
        "Install a skill from SKILL.md content, a URL, or by name from the ClawHub catalog."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name or slug (from search results)"
                },
                "url": {
                    "type": "string",
                    "description": "Direct URL to a SKILL.md file"
                },
                "content": {
                    "type": "string",
                    "description": "Raw SKILL.md content to install directly"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;

        let content = if let Some(raw) = params.get("content").and_then(|v| v.as_str()) {
            // Direct content provided
            raw.to_string()
        } else if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
            // Fetch from explicit URL
            fetch_skill_content(url).await?
        } else {
            // Look up in catalog and fetch
            let download_url =
                crate::skills::catalog::skill_download_url(self.catalog.registry_url(), name);
            fetch_skill_content(&download_url).await?
        };

        // Check for duplicates and get install_dir under a brief read lock.
        let (user_dir, skill_name_from_parse) = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;

            // Parse to extract the name (cheap, in-memory)
            let normalized = crate::skills::normalize_line_endings(&content);
            let parsed = crate::skills::parser::parse_skill_md(&normalized)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let skill_name = parsed.manifest.name.clone();

            if guard.has(&skill_name) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' already exists",
                    skill_name
                )));
            }

            (guard.install_target_dir().to_path_buf(), skill_name)
        };

        // Perform async I/O (write to disk, validate round-trip) with no lock held.
        let (skill_name, loaded_skill) =
            crate::skills::registry::SkillRegistry::prepare_install_to_disk(
                &user_dir,
                &skill_name_from_parse,
                &crate::skills::normalize_line_endings(&content),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Commit the in-memory addition under a brief write lock.
        let installed_name = {
            let mut guard = self
                .registry
                .write()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .commit_install(&skill_name, loaded_skill)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            skill_name
        };

        let output = serde_json::json!({
            "name": installed_name,
            "status": "installed",
            "trust": "installed",
            "message": format!(
                "Skill '{}' installed successfully. It will activate when matching keywords are detected.",
                installed_name
            ),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

/// Validate that a URL is safe to fetch (SSRF prevention).
///
/// Rejects:
/// - Non-HTTPS URLs (except in tests)
/// - URLs pointing to private, loopback, or link-local IP addresses
/// - URLs without a host
pub fn validate_fetch_url(url_str: &str) -> Result<(), ToolError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    // Require HTTPS
    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    // Check if host is an IP address and reject private ranges.
    // Unwrap IPv4-mapped IPv6 addresses (e.g. ::ffff:192.168.1.1) to catch
    // SSRF bypasses that encode private IPv4 addresses as IPv6.
    if let Ok(raw_ip) = host.parse::<std::net::IpAddr>() {
        let ip = match raw_ip {
            std::net::IpAddr::V6(v6) => v6
                .to_ipv4_mapped()
                .map(std::net::IpAddr::V4)
                .unwrap_or(std::net::IpAddr::V6(v6)),
            other => other,
        };
        if ip.is_loopback() || ip.is_unspecified() || is_private_ip(&ip) || is_link_local_ip(&ip) {
            return Err(ToolError::ExecutionFailed(format!(
                "URL points to a private/loopback/link-local address: {}",
                host
            )));
        }
    }

    // Reject common internal hostnames
    let host_lower = host.to_lowercase();
    if host_lower == "localhost"
        || host_lower == "metadata.google.internal"
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".local")
    {
        return Err(ToolError::ExecutionFailed(format!(
            "URL points to an internal hostname: {}",
            host
        )));
    }

    Ok(())
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16
            v4.is_private() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => {
            // Unique local (fc00::/7)
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn is_link_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            // fe80::/10
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Fetch SKILL.md content from a URL with SSRF protection.
///
/// The ClawHub registry returns skill downloads as ZIP archives containing
/// `SKILL.md` and `_meta.json`. This function detects ZIP responses (by the
/// `PK\x03\x04` magic bytes) and extracts `SKILL.md` automatically. Plain
/// text responses are returned as-is.
pub async fn fetch_skill_content(url: &str) -> Result<String, ToolError> {
    validate_fetch_url(url)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("ironclaw/0.1")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e)))?;

    let response = client.get(url).send().await.map_err(|e| {
        ToolError::ExecutionFailed(format!("Failed to fetch skill from {}: {}", url, e))
    })?;

    if !response.status().is_success() {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill fetch returned HTTP {}: {}",
            response.status(),
            url
        )));
    }

    // Limit download size to prevent memory exhaustion from large responses.
    const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024; // 10 MB
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response body: {}", e)))?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(ToolError::ExecutionFailed(format!(
            "Response too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_DOWNLOAD_BYTES
        )));
    }

    // Detect ZIP archive (PK\x03\x04 magic) and extract SKILL.md
    let content = if bytes.starts_with(b"PK\x03\x04") {
        extract_skill_from_zip(&bytes)?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|e| {
            ToolError::ExecutionFailed(format!("Response is not valid UTF-8: {}", e))
        })?
    };

    // Basic size check
    if content.len() as u64 > crate::skills::MAX_PROMPT_FILE_SIZE {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill content too large: {} bytes (max {} bytes)",
            content.len(),
            crate::skills::MAX_PROMPT_FILE_SIZE
        )));
    }

    Ok(content)
}

/// Extract `SKILL.md` from a ZIP archive returned by the ClawHub download API.
///
/// Walks ZIP local file headers looking for an entry named `SKILL.md`.
/// Supports Store (method 0) and Deflate (method 8) compression.
fn extract_skill_from_zip(data: &[u8]) -> Result<String, ToolError> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    // SKILL.md files should never be larger than 1 MB.
    const MAX_DECOMPRESSED: usize = 1_024 * 1_024;

    let mut offset = 0;
    while offset + 30 <= data.len() {
        // Local file header signature = PK\x03\x04
        if data[offset..offset + 4] != [0x50, 0x4B, 0x03, 0x04] {
            break;
        }

        let compression = u16::from_le_bytes([data[offset + 8], data[offset + 9]]);
        let compressed_size = u32::from_le_bytes([
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            data[offset + 22],
            data[offset + 23],
            data[offset + 24],
            data[offset + 25],
        ]) as usize;
        let name_len = u16::from_le_bytes([data[offset + 26], data[offset + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[offset + 28], data[offset + 29]]) as usize;

        let name_start = offset + 30;
        let name_end = name_start + name_len;
        if name_end > data.len() {
            break;
        }
        let file_name = std::str::from_utf8(&data[name_start..name_end]).unwrap_or("");

        let data_start = name_end
            .checked_add(extra_len)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header offset overflow".to_string()))?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header size overflow".to_string()))?;

        if file_name == "SKILL.md" {
            if data_end > data.len() {
                return Err(ToolError::ExecutionFailed(
                    "ZIP archive truncated".to_string(),
                ));
            }

            if uncompressed_size > MAX_DECOMPRESSED {
                return Err(ToolError::ExecutionFailed(
                    "ZIP entry too large to decompress safely".to_string(),
                ));
            }

            let raw = &data[data_start..data_end];
            let decompressed = match compression {
                0 => raw.to_vec(), // Store
                8 => {
                    // Deflate -- wrap with a read limit to guard against ZIP bombs
                    // where the declared size is small but decompressed output is huge.
                    let mut decoder = DeflateDecoder::new(raw).take(MAX_DECOMPRESSED as u64);
                    let mut buf = Vec::with_capacity(uncompressed_size.min(MAX_DECOMPRESSED));
                    decoder.read_to_end(&mut buf).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to decompress SKILL.md: {}", e))
                    })?;
                    buf
                }
                other => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Unsupported ZIP compression method: {}",
                        other
                    )));
                }
            };

            return String::from_utf8(decompressed).map_err(|e| {
                ToolError::ExecutionFailed(format!("SKILL.md in archive is not valid UTF-8: {}", e))
            });
        }

        // Skip to next entry
        offset = data_end;
    }

    Err(ToolError::ExecutionFailed(
        "ZIP archive does not contain SKILL.md".to_string(),
    ))
}

// ── skill_remove ────────────────────────────────────────────────────────

pub struct SkillRemoveTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
}

impl SkillRemoveTool {
    pub fn new(registry: Arc<std::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillRemoveTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed skill by name. Only user-installed skills can be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to remove"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;

        // Validate removal and get the filesystem path under a brief read lock.
        let skill_path = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .validate_remove(name)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
        };

        // Delete files from disk (async I/O, no lock held).
        crate::skills::registry::SkillRegistry::delete_skill_files(&skill_path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Remove from in-memory registry under a brief write lock.
        {
            let mut guard = self
                .registry
                .write()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .commit_remove(name)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        }

        let output = serde_json::json!({
            "name": name,
            "status": "removed",
            "message": format!("Skill '{}' has been removed.", name),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> Arc<std::sync::RwLock<SkillRegistry>> {
        let dir = tempfile::tempdir().unwrap();
        // Keep the tempdir so it lives for the test duration
        let path = dir.keep();
        Arc::new(std::sync::RwLock::new(SkillRegistry::new(path)))
    }

    fn test_catalog() -> Arc<SkillCatalog> {
        Arc::new(SkillCatalog::with_url("http://127.0.0.1:1"))
    }

    #[test]
    fn test_skill_list_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillListTool::new(test_registry());
        assert_eq!(tool.name(), "skill_list");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
    }

    #[test]
    fn test_skill_search_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillSearchTool::new(test_registry(), test_catalog());
        assert_eq!(tool.name(), "skill_search");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("query").is_some());
    }

    #[test]
    fn test_skill_install_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillInstallTool::new(test_registry(), test_catalog());
        assert_eq!(tool.name(), "skill_install");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("content").is_some());
    }

    #[test]
    fn test_skill_remove_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());
        assert_eq!(tool.name(), "skill_remove");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
    }

    #[test]
    fn test_validate_fetch_url_allows_https() {
        assert!(super::validate_fetch_url("https://clawhub.ai/api/v1/download?slug=foo").is_ok());
    }

    #[test]
    fn test_validate_fetch_url_rejects_http() {
        let err = super::validate_fetch_url("http://example.com/skill.md").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_private_ip() {
        let err = super::validate_fetch_url("https://192.168.1.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_loopback() {
        let err = super::validate_fetch_url("https://127.0.0.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_localhost() {
        let err = super::validate_fetch_url("https://localhost/skill.md").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_metadata_endpoint() {
        let err =
            super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_internal_domain() {
        let err =
            super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_file_scheme() {
        let err = super::validate_fetch_url("file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_extract_skill_from_zip_deflate() {
        // Build a real ZIP with flate2 + manual header construction.
        use flate2::Compression;
        use flate2::write::DeflateEncoder;
        use std::io::Write;

        let skill_md = b"---\nname: test\n---\n# Test Skill\n";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(skill_md).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut zip = Vec::new();
        // Local file header
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        zip.extend_from_slice(&[0x14, 0x00]); // version needed (2.0)
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x08, 0x00]); // compression: deflate
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32 (unused)
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes()); // compressed size
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(&compressed);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: test\n---\n# Test Skill\n");
    }

    #[test]
    fn test_extract_skill_from_zip_store() {
        let skill_md = b"---\nname: stored\n---\n# Stored\n";

        let mut zip = Vec::new();
        // Local file header
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]); // version needed (1.0)
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // compressed = uncompressed
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(skill_md);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: stored\n---\n# Stored\n");
    }

    #[test]
    fn test_extract_skill_from_zip_missing_skill_md() {
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]); // version
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&2u32.to_le_bytes()); // compressed size
        zip.extend_from_slice(&2u32.to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&10u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"_meta.json");
        zip.extend_from_slice(b"{}");

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(err.to_string().contains("does not contain SKILL.md"));
    }
}
