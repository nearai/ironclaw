//! Agent-callable tools for managing skills (prompt-level extensions).
//!
//! Four tools for discovering, installing, listing, and removing skills
//! entirely through conversation, following the extension_tools pattern.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, EngineCompatibility, Tool, ToolError, ToolOutput, require_str,
};
use ironclaw_skills::catalog::{
    SkillCatalog, catalog_entry_is_installed, resolve_catalog_slug_for_name,
};
use ironclaw_skills::registry::SkillRegistry;

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
                let is_installed =
                    catalog_entry_is_installed(&entry.slug, &entry.name, &installed_names);
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

async fn resolve_catalog_download_key(
    catalog: &SkillCatalog,
    name: &str,
    slug: Option<&str>,
) -> Result<String, ToolError> {
    if let Some(slug) = slug.filter(|s| !s.is_empty()) {
        return Ok(slug.to_string());
    }

    if name.contains('/') {
        return Ok(name.to_string());
    }

    let outcome = catalog.search(name).await;
    match resolve_catalog_slug_for_name(name, &outcome.results) {
        Ok(Some(resolved)) => Ok(resolved),
        Ok(None) => {
            let reason = outcome
                .error
                .unwrap_or_else(|| "no unique catalog match was found".to_string());
            Err(ToolError::ExecutionFailed(format!(
                "Could not resolve skill name '{}' to a catalog slug: {}",
                name, reason
            )))
        }
        Err(e) => Err(ToolError::ExecutionFailed(e.to_string())),
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
                "slug": {
                    "type": "string",
                    "description": "Registry slug from catalog search results; preferred when installing from ClawHub"
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
        let mut requested_identifier = params
            .get("slug")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let content = if let Some(raw) = params.get("content").and_then(|v| v.as_str()) {
            // Direct content provided
            raw.to_string()
        } else if let Some(url) = params
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            // Fetch from explicit URL
            fetch_skill_content(url).await?
        } else {
            // Look up in catalog and fetch
            let download_key = resolve_catalog_download_key(
                self.catalog.as_ref(),
                name,
                requested_identifier.as_deref(),
            )
            .await?;
            requested_identifier = Some(download_key.clone());
            let download_url = ironclaw_skills::catalog::skill_download_url(
                self.catalog.registry_url(),
                &download_key,
            );
            fetch_skill_content(&download_url).await?
        };

        let normalized = ironclaw_skills::normalize_line_endings(&content);

        // Check for duplicates and get install_dir under a brief read lock.
        let (user_dir, skill_name_from_parse, install_content) = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;

            let (skill_name, install_content) =
                ironclaw_skills::registry::SkillRegistry::resolve_install_content(
                    &normalized,
                    requested_identifier.as_deref(),
                )
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            if guard.has(&skill_name) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' already exists",
                    skill_name
                )));
            }

            (
                guard.install_target_dir().to_path_buf(),
                skill_name,
                install_content,
            )
        };

        // Perform async I/O (write to disk, validate round-trip) with no lock held.
        let (skill_name, loaded_skill) =
            ironclaw_skills::registry::SkillRegistry::prepare_install_to_disk(
                &user_dir,
                &skill_name_from_parse,
                &install_content,
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
pub fn validate_fetch_url(url_str: &str) -> Result<reqwest::Url, ToolError> {
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    // Require HTTPS
    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    // Check if host is an IP address and reject private ranges.
    // Use reqwest::Url host variants to get proper IpAddr values -- host_str()
    // returns bracketed IPv6 (e.g. "[::1]") which IpAddr cannot parse.
    // Unwrap IPv4-mapped IPv6 addresses (e.g. ::ffff:192.168.1.1) to catch
    // SSRF bypasses that encode private IPv4 addresses as IPv6.
    if let Some(ip) = host_ip_addr(&host) {
        validate_fetch_ip(&ip, &host.to_string())?;
    }

    // Reject common internal hostnames, including FQDN forms with a trailing dot.
    let host_lower = normalize_domain(host.to_string().as_str()).to_lowercase();
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

    Ok(parsed)
}

fn host_ip_addr(host: &url::Host<&str>) -> Option<std::net::IpAddr> {
    match host {
        url::Host::Ipv4(v4) => Some(std::net::IpAddr::V4(*v4)),
        url::Host::Ipv6(v6) => Some(normalize_ip(std::net::IpAddr::V6(*v6))),
        url::Host::Domain(_) => None,
    }
}

fn normalize_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(v6)),
        other => other,
    }
}

fn validate_fetch_ip(ip: &std::net::IpAddr, display_host: &str) -> Result<(), ToolError> {
    if ip.is_loopback() || ip.is_unspecified() || is_private_ip(ip) || is_link_local_ip(ip) {
        return Err(ToolError::ExecutionFailed(format!(
            "URL points to a private/loopback/link-local address: {}",
            display_host
        )));
    }

    Ok(())
}

fn normalize_domain(host: &str) -> &str {
    host.trim_end_matches('.')
}

fn validate_resolved_addrs(host: &str, addrs: &[std::net::SocketAddr]) -> Result<(), ToolError> {
    if addrs.is_empty() {
        return Err(ToolError::ExecutionFailed(format!(
            "DNS resolution returned no addresses for {}",
            host
        )));
    }

    for addr in addrs {
        let ip = normalize_ip(addr.ip());
        validate_fetch_ip(&ip, host)?;
    }

    Ok(())
}

fn build_fetch_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("ironclaw/0.1")
        .redirect(reqwest::redirect::Policy::none())
}

async fn build_safe_fetch_client(parsed: &reqwest::Url) -> Result<reqwest::Client, ToolError> {
    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    match host {
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => build_fetch_client_builder()
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e))),
        url::Host::Domain(domain) => {
            let lookup_host = normalize_domain(domain);
            let port = parsed
                .port_or_known_default()
                .ok_or_else(|| ToolError::ExecutionFailed("URL has no valid port".to_string()))?;

            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((lookup_host, port))
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "DNS resolution failed for {}: {}",
                        lookup_host, e
                    ))
                })?
                .collect();

            validate_resolved_addrs(domain, &addrs)?;

            build_fetch_client_builder()
                .resolve_to_addrs(domain, &addrs)
                .build()
                .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e)))
        }
    }
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
    let parsed = validate_fetch_url(url)?;
    let client = build_safe_fetch_client(&parsed).await?;

    let response = client.get(parsed.clone()).send().await.map_err(|e| {
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
    if content.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill content too large: {} bytes (max {} bytes)",
            content.len(),
            ironclaw_skills::MAX_PROMPT_FILE_SIZE
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
        "Permanently remove an installed skill from disk. This action cannot be undone — \
         the skill files will be deleted."
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
        ironclaw_skills::registry::SkillRegistry::delete_skill_files(&skill_path)
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
        ApprovalRequirement::Always
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V1Only
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
        assert!(schema["properties"].get("slug").is_some());
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("content").is_some());
    }

    #[test]
    fn test_find_catalog_slug_for_display_name() {
        let entries = vec![ironclaw_skills::catalog::CatalogEntry {
            slug: "finance/mortgage-calculator".to_string(),
            name: "Mortgage Calculator".to_string(),
            description: String::new(),
            version: String::new(),
            score: 1.0,
            updated_at: None,
            stars: None,
            downloads: None,
            installs_current: None,
            owner: None,
        }];

        assert_eq!(
            resolve_catalog_slug_for_name("Mortgage Calculator", &entries)
                .unwrap()
                .as_deref(),
            Some("finance/mortgage-calculator")
        );
        assert_eq!(
            resolve_catalog_slug_for_name("mortgage-calculator", &entries)
                .unwrap()
                .as_deref(),
            Some("finance/mortgage-calculator")
        );
    }

    #[test]
    fn test_resolve_catalog_slug_for_display_name_is_ambiguous() {
        let entries = vec![
            ironclaw_skills::catalog::CatalogEntry {
                slug: "alice/mortgage-calculator".to_string(),
                name: "Mortgage Calculator".to_string(),
                description: String::new(),
                version: String::new(),
                score: 1.0,
                updated_at: None,
                stars: None,
                downloads: None,
                installs_current: None,
                owner: None,
            },
            ironclaw_skills::catalog::CatalogEntry {
                slug: "bob/mortgage-calculator".to_string(),
                name: "Mortgage Calculator".to_string(),
                description: String::new(),
                version: String::new(),
                score: 0.9,
                updated_at: None,
                stars: None,
                downloads: None,
                installs_current: None,
                owner: None,
            },
        ];

        assert!(resolve_catalog_slug_for_name("Mortgage Calculator", &entries).is_err());
    }

    #[test]
    fn test_skill_remove_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());
        assert_eq!(tool.name(), "skill_remove");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
    }

    #[test]
    fn skill_remove_always_requires_approval_regardless_of_params() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());

        let test_cases = vec![
            ("no params", serde_json::json!({})),
            ("empty name", serde_json::json!({"name": ""})),
            (
                "deployment skill",
                serde_json::json!({"name": "deployment"}),
            ),
            ("custom skill", serde_json::json!({"name": "custom-skill"})),
            (
                "with extra fields",
                serde_json::json!({"name": "skill", "extra": "field"}),
            ),
        ];

        for (case_name, params) in test_cases {
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "skill_remove must always require approval for case: {}",
                case_name
            );
        }
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
    fn test_validate_fetch_url_rejects_localhost_fqdn() {
        let err = super::validate_fetch_url("https://localhost./skill.md").unwrap_err();
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
    fn test_validate_fetch_url_rejects_ipv4_mapped_ipv6_loopback() {
        let err = super::validate_fetch_url("https://[::ffff:127.0.0.1]/skill.md").unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_ipv6_loopback() {
        let err = super::validate_fetch_url("https://[::1]/skill.md").unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_validate_resolved_addrs_rejects_loopback_hostname() {
        let addrs = vec![
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
            "[::1]:443".parse::<std::net::SocketAddr>().unwrap(),
        ];

        let err = super::validate_resolved_addrs("example.com", &addrs).unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_validate_resolved_addrs_allows_public_hostname() {
        let addrs = vec![
            "8.8.8.8:443".parse::<std::net::SocketAddr>().unwrap(),
            "[2606:4700:4700::1111]:443"
                .parse::<std::net::SocketAddr>()
                .unwrap(),
        ];

        assert!(super::validate_resolved_addrs("example.com", &addrs).is_ok());
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

    // ── ZIP extraction security regression tests ────────────────────────

    /// Helper: build a minimal ZIP local file header with Store compression.
    fn build_zip_entry_store(file_name: &str, content: &[u8]) -> Vec<u8> {
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        zip.extend_from_slice(&[0x0A, 0x00]); // version needed (1.0)
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store (0)
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&(content.len() as u32).to_le_bytes()); // compressed size
        zip.extend_from_slice(&(content.len() as u32).to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&(file_name.len() as u16).to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(file_name.as_bytes());
        zip.extend_from_slice(content);
        zip
    }

    #[test]
    fn test_zip_extract_valid_skill() {
        let content = b"---\nname: hello\n---\n# Hello Skill\nDoes things.\n";
        let zip = build_zip_entry_store("SKILL.md", content);
        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, std::str::from_utf8(content).unwrap());
    }

    #[test]
    fn test_zip_extract_ignores_non_skill_entries() {
        // ZIP with README.md and src/main.rs but no SKILL.md -- should error.
        let mut zip = Vec::new();
        zip.extend_from_slice(&build_zip_entry_store("README.md", b"# Readme"));
        zip.extend_from_slice(&build_zip_entry_store("src/main.rs", b"fn main() {}"));

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("does not contain SKILL.md"),
            "Expected 'does not contain SKILL.md' error, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_path_traversal_rejected() {
        // An entry named "../../SKILL.md" must NOT match the exact "SKILL.md" check.
        let content = b"---\nname: evil\n---\n# Malicious path traversal\n";
        let zip = build_zip_entry_store("../../SKILL.md", content);

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("does not contain SKILL.md"),
            "Path traversal entry should not match SKILL.md, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_nested_path_not_matched() {
        // An entry named "subdir/SKILL.md" must NOT match the exact "SKILL.md" check.
        let content = b"---\nname: nested\n---\n# Nested\n";
        let zip = build_zip_entry_store("subdir/SKILL.md", content);

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("does not contain SKILL.md"),
            "Nested path should not match SKILL.md, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_oversized_rejected() {
        // Create a ZIP entry whose declared uncompressed_size exceeds MAX_DECOMPRESSED (1 MB).
        let oversized_claim: u32 = 2 * 1024 * 1024; // 2 MB
        let small_body = b"tiny";

        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        zip.extend_from_slice(&[0x0A, 0x00]); // version needed
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&(small_body.len() as u32).to_le_bytes()); // compressed size (actual)
        zip.extend_from_slice(&oversized_claim.to_le_bytes()); // uncompressed size (forged)
        zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(small_body);

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("too large"),
            "Oversized entry should be rejected, got: {}",
            err
        );
    }

    // ── SSRF prevention regression tests ────────────────────────────────

    #[test]
    fn test_is_private_ip_blocks_loopback() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        // is_private_ip checks v4.is_private() which does NOT include loopback,
        // but validate_fetch_url checks is_loopback() separately. Test the full flow.
        assert!(loopback.is_loopback());
        // Also verify via validate_fetch_url
        assert!(super::validate_fetch_url("https://127.0.0.1/skill.md").is_err());
    }

    #[test]
    fn test_is_private_ip_blocks_private_ranges() {
        let cases: Vec<(&str, bool)> = vec![
            ("10.0.0.1", true),
            ("10.255.255.255", true),
            ("172.16.0.1", true),
            ("172.31.255.255", true),
            ("192.168.1.1", true),
            ("192.168.0.0", true),
        ];
        for (ip_str, expect_private) in cases {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            assert_eq!(
                super::is_private_ip(&ip),
                expect_private,
                "Expected is_private_ip({}) = {}",
                ip_str,
                expect_private
            );
        }
    }

    #[test]
    fn test_is_private_ip_blocks_link_local() {
        // 169.254.0.0/16 range (link-local)
        let cases = vec!["169.254.1.1", "169.254.0.1", "169.254.255.255"];
        for ip_str in cases {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            // is_private_ip includes v4.is_link_local()
            assert!(
                super::is_private_ip(&ip),
                "Expected is_private_ip({}) = true (link-local)",
                ip_str
            );
        }
    }

    #[test]
    fn test_is_private_ip_allows_public() {
        let public_ips = vec!["8.8.8.8", "1.1.1.1", "93.184.216.34", "151.101.1.67"];
        for ip_str in public_ips {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            assert!(
                !super::is_private_ip(&ip),
                "Expected is_private_ip({}) = false (public IP)",
                ip_str
            );
            assert!(!ip.is_loopback(), "Expected {} is not loopback", ip_str);
        }
    }

    #[test]
    fn test_is_private_ip_blocks_ipv4_mapped_ipv6() {
        // Test the IPv4-mapped unwrapping logic end-to-end through
        // validate_fetch_url. IPv6 URLs like https://[::ffff:127.0.0.1]/path
        // must be correctly detected as private/loopback.

        // ::ffff:127.0.0.1 mapped -> 127.0.0.1 (loopback) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:127.0.0.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped loopback should be blocked, got: {}",
            err
        );

        // ::ffff:192.168.1.1 mapped -> 192.168.1.1 (private) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:192.168.1.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped private should be blocked, got: {}",
            err
        );

        // ::ffff:10.0.0.1 mapped -> 10.0.0.1 (private) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:10.0.0.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped 10.x should be blocked, got: {}",
            err
        );

        // ::ffff:8.8.8.8 mapped -> 8.8.8.8 (public) -- must be allowed
        assert!(
            super::validate_fetch_url("https://[::ffff:8.8.8.8]/skill.md").is_ok(),
            "IPv4-mapped public IP should be allowed"
        );

        // Pure IPv6 loopback ::1 -- must be blocked
        let err = super::validate_fetch_url("https://[::1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv6 loopback should be blocked, got: {}",
            err
        );
    }

    #[test]
    fn test_is_restricted_host_blocks_metadata() {
        // Cloud metadata endpoint (AWS/GCP/Azure style)
        let err =
            super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("link-local"),
            "Metadata IP should be blocked, got: {}",
            err
        );

        // GCP metadata hostname
        let err =
            super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            "metadata.google.internal should be blocked, got: {}",
            err
        );

        // Generic .internal domain
        let err = super::validate_fetch_url("https://service.internal/api").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            ".internal domains should be blocked, got: {}",
            err
        );

        // .local domain
        let err = super::validate_fetch_url("https://myhost.local/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            ".local domains should be blocked, got: {}",
            err
        );
    }

    #[test]
    fn test_is_restricted_host_allows_normal() {
        let allowed = vec![
            "https://github.com/repo/SKILL.md",
            "https://clawhub.dev/api/v1/download?slug=foo",
            "https://raw.githubusercontent.com/user/repo/main/SKILL.md",
            "https://example.com/skills/deploy.md",
        ];
        for url in allowed {
            assert!(
                super::validate_fetch_url(url).is_ok(),
                "Expected validate_fetch_url({}) to succeed",
                url
            );
        }
    }

    #[test]
    fn test_empty_url_param_is_treated_as_absent() {
        // LLMs sometimes pass "" for optional parameters instead of omitting them.
        // Before the fix, url: "" would match Some("") and attempt to fetch from an
        // empty URL (failing with an invalid URL error) instead of falling through to
        // the catalog lookup. The full execute path cannot be tested here without a
        // real catalog and database, so this test verifies the parameter filtering
        // behaviour directly.
        let params = serde_json::json!({"name": "my-skill", "url": ""});
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        assert!(
            url.is_none(),
            "empty url string should be treated as absent"
        );
    }
}
