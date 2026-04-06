//! MCP server configuration.
//!
//! Stores configuration for connecting to hosted MCP servers.
//! Configuration is persisted at ~/.ironclaw/mcp-servers.json.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::bootstrap::ironclaw_base_dir;
use crate::tools::tool::ToolError;

/// Transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransportConfig {
    /// HTTP/HTTPS transport (uses the `url` field on McpServerConfig).
    Http,
    /// Stdio transport — spawns a child process.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// Unix domain socket transport.
    Unix { socket_path: String },
}

/// Configuration for connecting to a remote MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server (e.g., "notion", "github").
    pub name: String,

    /// Server URL (must be HTTPS for remote servers).
    pub url: String,

    /// Transport configuration. If `None`, defaults to Http using `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<McpTransportConfig>,

    /// Custom headers to include in every HTTP request.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,

    /// OAuth configuration (if server requires authentication).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,

    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional description for the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

impl McpServerConfig {
    /// Create a new MCP server configuration.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            transport: None,
            headers: HashMap::new(),
            oauth: None,
            enabled: true,
            description: None,
        }
    }

    /// Create a new stdio transport MCP server configuration.
    pub fn new_stdio(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            url: String::new(),
            transport: Some(McpTransportConfig::Stdio {
                command: command.into(),
                args,
                env,
            }),
            headers: HashMap::new(),
            oauth: None,
            enabled: true,
            description: None,
        }
    }

    /// Create a new Unix socket transport MCP server configuration.
    pub fn new_unix(name: impl Into<String>, socket_path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: String::new(),
            transport: Some(McpTransportConfig::Unix {
                socket_path: socket_path.into(),
            }),
            headers: HashMap::new(),
            oauth: None,
            enabled: true,
            description: None,
        }
    }

    /// Set OAuth configuration.
    pub fn with_oauth(mut self, oauth: OAuthConfig) -> Self {
        self.oauth = Some(oauth);
        self
    }

    /// Set description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set custom headers.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Get the effective transport type.
    pub fn effective_transport(&self) -> EffectiveTransport<'_> {
        match &self.transport {
            Some(McpTransportConfig::Http) | None => EffectiveTransport::Http,
            Some(McpTransportConfig::Stdio { command, args, env }) => {
                EffectiveTransport::Stdio { command, args, env }
            }
            Some(McpTransportConfig::Unix { socket_path }) => {
                EffectiveTransport::Unix { socket_path }
            }
        }
    }

    /// Validate the server configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.is_empty() {
            return Err(ConfigError::InvalidConfig {
                reason: "Server name cannot be empty".to_string(),
            });
        }

        match self.effective_transport() {
            EffectiveTransport::Http => {
                if self.url.is_empty() {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Server URL cannot be empty".to_string(),
                    });
                }

                // Remote servers must use HTTPS (localhost is allowed for development)
                let is_localhost = is_localhost_url(&self.url);
                if !is_localhost && !self.url.to_lowercase().starts_with("https://") {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Remote MCP servers must use HTTPS".to_string(),
                    });
                }
            }
            EffectiveTransport::Stdio { command, .. } => {
                if command.is_empty() {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Stdio transport command cannot be empty".to_string(),
                    });
                }
            }
            EffectiveTransport::Unix { socket_path } => {
                if socket_path.is_empty() {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Unix socket path cannot be empty".to_string(),
                    });
                }
            }
        }

        // Validate custom header names and values using the http crate's RFC 9110
        // token validation (catches CRLF, spaces, colons, null bytes, etc.)
        for (name, value) in &self.headers {
            if name.is_empty() {
                return Err(ConfigError::InvalidConfig {
                    reason: "Header name cannot be empty".to_string(),
                });
            }
            if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(ConfigError::InvalidConfig {
                    reason: format!(
                        "Header name '{}' is not a valid HTTP header name (RFC 9110)",
                        name
                    ),
                });
            }
            if reqwest::header::HeaderValue::from_str(value).is_err() {
                return Err(ConfigError::InvalidConfig {
                    reason: format!("Header value for '{}' contains invalid characters", name),
                });
            }
        }

        Ok(())
    }

    /// Check if any custom header sets an Authorization value.
    ///
    /// Used to skip OAuth token injection when the user has explicitly
    /// configured an Authorization header (e.g. for API-key-based servers).
    pub fn has_custom_auth_header(&self) -> bool {
        self.headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"))
    }

    /// Check if this server requires authentication.
    ///
    /// Returns true if OAuth is pre-configured OR if this is a remote HTTPS server
    /// (which likely supports Dynamic Client Registration even without pre-configured OAuth).
    ///
    /// Non-HTTP transports (stdio, unix) never require auth.
    pub fn requires_auth(&self) -> bool {
        // Non-HTTP transports don't use HTTP auth
        if !matches!(self.effective_transport(), EffectiveTransport::Http) {
            return false;
        }

        if self.oauth.is_some() {
            return true;
        }
        // Remote HTTPS servers need auth handling (DCR, token refresh, 401 detection).
        // Localhost/127.0.0.1 servers are assumed to be dev servers without auth.
        let url_lower = self.url.to_lowercase();
        let is_localhost = is_localhost_url(&url_lower);
        url_lower.starts_with("https://") && !is_localhost
    }

    /// Get the secret name used to store the access token.
    pub fn token_secret_name(&self) -> String {
        format!("mcp_{}_access_token", self.name)
    }

    /// Get the secret name used to store the refresh token.
    ///
    /// Matches the convention used by the hosted OAuth flow in
    /// `store_oauth_tokens`: `{token_secret_name}_refresh_token`.
    pub fn refresh_token_secret_name(&self) -> String {
        format!("{}_refresh_token", self.token_secret_name())
    }

    /// Legacy secret name for refresh tokens (pre-v0.22).
    ///
    /// Earlier versions stored refresh tokens as `mcp_{name}_refresh_token`
    /// instead of `{token_secret_name}_refresh_token`. Used as a fallback
    /// during lookup to avoid forcing re-auth on existing users.
    pub fn legacy_refresh_token_secret_name(&self) -> String {
        format!("mcp_{}_refresh_token", self.name)
    }

    /// Get the secret name used to store the DCR client ID.
    pub fn client_id_secret_name(&self) -> String {
        format!("mcp_{}_client_id", self.name)
    }

    /// Get the secret name used to store the DCR client secret.
    pub fn client_secret_secret_name(&self) -> String {
        format!("mcp_{}_client_secret", self.name)
    }
}

/// OAuth 2.1 configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// OAuth client ID.
    pub client_id: String,

    /// Authorization endpoint URL.
    /// If not provided, will be discovered from /.well-known/oauth-protected-resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,

    /// Token endpoint URL.
    /// If not provided, will be discovered from /.well-known/oauth-authorization-server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,

    /// Scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Whether to use PKCE (default: true, as required by OAuth 2.1).
    #[serde(default = "default_true")]
    pub use_pkce: bool,

    /// Extra parameters to include in the authorization request.
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
}

impl OAuthConfig {
    /// Create a new OAuth configuration with just a client ID.
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            authorization_url: None,
            token_url: None,
            scopes: Vec::new(),
            use_pkce: true,
            extra_params: HashMap::new(),
        }
    }

    /// Set authorization and token URLs.
    pub fn with_endpoints(
        mut self,
        authorization_url: impl Into<String>,
        token_url: impl Into<String>,
    ) -> Self {
        self.authorization_url = Some(authorization_url.into());
        self.token_url = Some(token_url.into());
        self
    }

    /// Set scopes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }
}

/// Configuration file containing all MCP servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersFile {
    /// List of configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,

    /// Schema version for future compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

fn default_schema_version() -> u32 {
    1
}

impl McpServersFile {
    /// Get a server by name.
    pub fn get(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// Get a mutable server by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut McpServerConfig> {
        self.servers.iter_mut().find(|s| s.name == name)
    }

    /// Add or update a server configuration.
    pub fn upsert(&mut self, config: McpServerConfig) {
        if let Some(existing) = self.get_mut(&config.name) {
            *existing = config;
        } else {
            self.servers.push(config);
        }
    }

    /// Remove a server by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let len_before = self.servers.len();
        self.servers.retain(|s| s.name != name);
        self.servers.len() < len_before
    }

    /// Get all enabled servers.
    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|s| s.enabled)
    }
}

/// Error type for MCP configuration operations.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("Server not found: {name}")]
    ServerNotFound { name: String },
}

impl From<ConfigError> for ToolError {
    fn from(err: ConfigError) -> Self {
        ToolError::ExternalService(err.to_string())
    }
}

/// MCP server id (`registry/mcp-servers/nearai.json` → `name`, or this if the catalog is empty).
pub const NEARAI_MCP_SERVER_NAME: &str = "nearai";

const NEARAI_MCP_REGISTRY_KEY: &str = "mcp-servers/nearai";

/// Human-readable title for extension lists (from the embedded manifest when present).
pub(crate) fn nearai_mcp_display_title() -> String {
    let catalog = crate::registry::embedded::load_embedded();
    catalog
        .get(NEARAI_MCP_REGISTRY_KEY)
        .map(|m| m.display_name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "NEAR AI".to_string())
}

fn derive_nearai_mcp_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{}/mcp", base)
}

fn nearai_mcp_server_from_env() -> Option<McpServerConfig> {
    let base_url = crate::config::helpers::env_or_override("NEARAI_BASE_URL")?;
    let api_key = crate::config::helpers::env_or_override("NEARAI_API_KEY")?;

    let catalog = crate::registry::embedded::load_embedded();
    let manifest = catalog.get(NEARAI_MCP_REGISTRY_KEY);
    let name = manifest
        .map(|m| m.name.as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(NEARAI_MCP_SERVER_NAME)
        .to_string();
    let description = manifest
        .and_then(|m| (!m.description.is_empty()).then_some(m.description.clone()))
        .unwrap_or_else(|| "Use NEAR AI built-in tools like web search".to_string());

    let headers = HashMap::from([("Authorization".to_string(), format!("Bearer {}", api_key))]);
    let server = McpServerConfig::new(name, derive_nearai_mcp_url(&base_url))
        .with_headers(headers)
        .with_description(description);

    match server.validate() {
        Ok(()) => Some(server),
        Err(err) => {
            tracing::warn!("Ignoring invalid NEAR AI MCP bootstrap config: {}", err);
            None
        }
    }
}

pub async fn bootstrap_nearai_mcp_server(
    db: Option<&dyn crate::db::Database>,
    user_id: &str,
) -> Result<bool, ConfigError> {
    let Some(server) = nearai_mcp_server_from_env() else {
        return Ok(false);
    };

    let mut servers = match db {
        Some(store) => load_mcp_servers_from_db(store, user_id).await?,
        None => load_mcp_servers().await?,
    };

    if servers.get(&server.name).is_some() {
        return Ok(false);
    }
    servers.upsert(server);

    match db {
        Some(store) => save_mcp_servers_to_db(store, user_id, &servers).await?,
        None => save_mcp_servers(&servers).await?,
    }
    Ok(true)
}

/// Get the default MCP servers configuration path.
pub fn default_config_path() -> PathBuf {
    ironclaw_base_dir().join("mcp-servers.json")
}

/// Load MCP server configurations from the default location.
pub async fn load_mcp_servers() -> Result<McpServersFile, ConfigError> {
    load_mcp_servers_from(default_config_path()).await
}

/// Load MCP server configurations from a specific path.
pub async fn load_mcp_servers_from(path: impl AsRef<Path>) -> Result<McpServersFile, ConfigError> {
    let path = path.as_ref();

    if !path.exists() {
        return Ok(McpServersFile::default());
    }

    let content = fs::read_to_string(path).await?;
    let config: McpServersFile = serde_json::from_str(&content)?;

    // Validate every server on load so corrupted configs are caught early
    for server in &config.servers {
        server.validate().map_err(|e| ConfigError::InvalidConfig {
            reason: format!("Server '{}': {}", server.name, e),
        })?;
    }

    Ok(config)
}

/// Save MCP server configurations to the default location.
pub async fn save_mcp_servers(config: &McpServersFile) -> Result<(), ConfigError> {
    save_mcp_servers_to(config, default_config_path()).await
}

/// Save MCP server configurations to a specific path.
pub async fn save_mcp_servers_to(
    config: &McpServersFile,
    path: impl AsRef<Path>,
) -> Result<(), ConfigError> {
    let path = path.as_ref();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let content = serde_json::to_string_pretty(config)?;

    // Write to a temporary file first, then atomically rename to avoid
    // corrupting the config if the process crashes during the write.
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).await?;
    fs::rename(&tmp_path, path).await?;

    Ok(())
}

/// Add a new MCP server configuration.
pub async fn add_mcp_server(config: McpServerConfig) -> Result<(), ConfigError> {
    config.validate()?;

    let mut servers = load_mcp_servers().await?;
    servers.upsert(config);
    save_mcp_servers(&servers).await?;

    Ok(())
}

/// Remove an MCP server by name.
pub async fn remove_mcp_server(name: &str) -> Result<(), ConfigError> {
    let mut servers = load_mcp_servers().await?;

    if !servers.remove(name) {
        return Err(ConfigError::ServerNotFound {
            name: name.to_string(),
        });
    }

    save_mcp_servers(&servers).await?;

    Ok(())
}

/// Get a specific MCP server configuration.
pub async fn get_mcp_server(name: &str) -> Result<McpServerConfig, ConfigError> {
    let servers = load_mcp_servers().await?;

    servers
        .get(name)
        .cloned()
        .ok_or_else(|| ConfigError::ServerNotFound {
            name: name.to_string(),
        })
}

// ==================== Database-backed MCP server config ====================

/// Load MCP server configurations from the database settings table.
///
/// Falls back to the disk file if DB has no entry.
pub async fn load_mcp_servers_from_db(
    store: &dyn crate::db::Database,
    user_id: &str,
) -> Result<McpServersFile, ConfigError> {
    match store.get_setting(user_id, "mcp_servers").await {
        Ok(Some(value)) => {
            let config: McpServersFile = serde_json::from_value(value)?;
            // Validate every server on load so corrupted DB configs are caught early
            for server in &config.servers {
                server.validate().map_err(|e| ConfigError::InvalidConfig {
                    reason: format!("Server '{}': {}", server.name, e),
                })?;
            }
            Ok(config)
        }
        Ok(None) => {
            // No entry in DB, fall back to disk
            load_mcp_servers().await
        }
        Err(e) => {
            tracing::warn!(
                "Failed to load MCP servers from DB: {}, falling back to disk",
                e
            );
            load_mcp_servers().await
        }
    }
}

/// Save MCP server configurations to the database settings table.
pub async fn save_mcp_servers_to_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: &McpServersFile,
) -> Result<(), ConfigError> {
    let value = serde_json::to_value(config)?;
    store
        .set_setting(user_id, "mcp_servers", &value)
        .await
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// Add a new MCP server configuration (DB-backed).
pub async fn add_mcp_server_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: McpServerConfig,
) -> Result<(), ConfigError> {
    config.validate()?;

    let mut servers = load_mcp_servers_from_db(store, user_id).await?;
    servers.upsert(config);
    save_mcp_servers_to_db(store, user_id, &servers).await?;

    Ok(())
}

/// Remove an MCP server by name (DB-backed).
pub async fn remove_mcp_server_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    name: &str,
) -> Result<(), ConfigError> {
    let mut servers = load_mcp_servers_from_db(store, user_id).await?;

    if !servers.remove(name) {
        return Err(ConfigError::ServerNotFound {
            name: name.to_string(),
        });
    }

    save_mcp_servers_to_db(store, user_id, &servers).await?;
    Ok(())
}

/// Check if a URL points to a loopback address (localhost, 127.0.0.1, [::1]).
///
/// Uses `url::Url` for proper parsing so edge cases (IPv6, userinfo, ports)
/// are handled correctly without manual string splitting.
pub(crate) fn is_localhost_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Resolved transport type (borrows from config).
#[derive(Debug)]
pub enum EffectiveTransport<'a> {
    Http,
    Stdio {
        command: &'a str,
        args: &'a [String],
        env: &'a HashMap<String, String>,
    },
    Unix {
        socket_path: &'a str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_localhost_url() {
        assert!(is_localhost_url("http://localhost:3000/path"));
        assert!(is_localhost_url("https://localhost/path"));
        assert!(is_localhost_url("http://127.0.0.1:8080"));
        assert!(is_localhost_url("http://127.0.0.1"));
        assert!(!is_localhost_url("https://notlocalhost.com/path"));
        assert!(!is_localhost_url("https://example-localhost.io"));
        assert!(!is_localhost_url("https://mcp.notion.com"));
        assert!(is_localhost_url("http://user:pass@localhost:3000/path"));
        // IPv6 loopback
        assert!(is_localhost_url("http://[::1]:8080/path"));
        assert!(is_localhost_url("http://[::1]/path"));
        assert!(!is_localhost_url("http://[::2]:8080/path"));
    }

    #[test]
    fn test_server_config_validation() {
        // Valid HTTPS server
        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert!(config.validate().is_ok());

        // Valid localhost (allowed for dev)
        let config = McpServerConfig::new("local", "http://localhost:8080");
        assert!(config.validate().is_ok());

        // Invalid: empty name
        let config = McpServerConfig::new("", "https://example.com");
        assert!(config.validate().is_err());

        // Invalid: HTTP for remote server
        let config = McpServerConfig::new("remote", "http://mcp.example.com");
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_oauth_config_builder() {
        let oauth = OAuthConfig::new("client-123")
            .with_endpoints(
                "https://auth.example.com/authorize",
                "https://auth.example.com/token",
            )
            .with_scopes(vec!["read".to_string(), "write".to_string()]);

        assert_eq!(oauth.client_id, "client-123");
        assert!(oauth.authorization_url.is_some());
        assert!(oauth.token_url.is_some());
        assert_eq!(oauth.scopes.len(), 2);
        assert!(oauth.use_pkce);
    }

    #[test]
    fn test_servers_file_operations() {
        let mut file = McpServersFile::default();

        // Add a server
        file.upsert(McpServerConfig::new("notion", "https://mcp.notion.com"));
        assert_eq!(file.servers.len(), 1);

        // Update the server
        let mut updated = McpServerConfig::new("notion", "https://mcp.notion.com/v2");
        updated.enabled = false;
        file.upsert(updated);
        assert_eq!(file.servers.len(), 1);
        assert!(!file.get("notion").unwrap().enabled);

        // Add another server
        file.upsert(McpServerConfig::new("github", "https://mcp.github.com"));
        assert_eq!(file.servers.len(), 2);

        // Remove a server
        assert!(file.remove("notion"));
        assert_eq!(file.servers.len(), 1);
        assert!(file.get("notion").is_none());

        // Remove non-existent server
        assert!(!file.remove("nonexistent"));
    }

    #[tokio::test]
    async fn test_load_save_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mcp-servers.json");

        // Save a configuration
        let mut config = McpServersFile::default();
        config.upsert(
            McpServerConfig::new("notion", "https://mcp.notion.com").with_oauth(
                OAuthConfig::new("client-123")
                    .with_scopes(vec!["read".to_string(), "write".to_string()]),
            ),
        );

        save_mcp_servers_to(&config, &path).await.unwrap();

        // Load it back
        let loaded = load_mcp_servers_from(&path).await.unwrap();
        assert_eq!(loaded.servers.len(), 1);

        let server = loaded.get("notion").unwrap();
        assert_eq!(server.url, "https://mcp.notion.com");
        assert!(server.oauth.is_some());
        assert_eq!(server.oauth.as_ref().unwrap().client_id, "client-123");
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");

        let config = load_mcp_servers_from(&path).await.unwrap();
        assert!(config.servers.is_empty());
    }

    #[tokio::test]
    async fn test_load_rejects_corrupted_headers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mcp-servers.json");

        // Write a config with an invalid header name directly to disk,
        // bypassing the add_mcp_server() validation path.
        let corrupted = serde_json::json!({
            "servers": [{
                "name": "bad-server",
                "url": "https://mcp.example.com",
                "enabled": true,
                "headers": { "X Bad": "value" }
            }]
        });
        tokio::fs::write(&path, corrupted.to_string())
            .await
            .unwrap();

        let result = load_mcp_servers_from(&path).await;
        assert!(result.is_err(), "Load should reject corrupted headers");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("bad-server"),
            "Error should name the offending server, got: {err}"
        );
    }

    #[test]
    fn test_token_secret_names() {
        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert_eq!(config.token_secret_name(), "mcp_notion_access_token");
        // Refresh token name follows the hosted OAuth convention:
        // {token_secret_name}_refresh_token
        assert_eq!(
            config.refresh_token_secret_name(),
            "mcp_notion_access_token_refresh_token"
        );
        // Legacy name used before v0.22 — fallback lookup prevents forced re-auth
        assert_eq!(
            config.legacy_refresh_token_secret_name(),
            "mcp_notion_refresh_token"
        );
        assert_eq!(config.client_id_secret_name(), "mcp_notion_client_id");
        assert_eq!(
            config.client_secret_secret_name(),
            "mcp_notion_client_secret"
        );
    }

    #[test]
    fn test_requires_auth_with_oauth() {
        let config = McpServerConfig::new("notion", "https://mcp.notion.com")
            .with_oauth(OAuthConfig::new("client-123"));
        assert!(config.requires_auth());
    }

    #[test]
    fn test_requires_auth_remote_https_without_oauth() {
        // Remote HTTPS servers need auth even without pre-configured OAuth (DCR)
        let config = McpServerConfig::new("github-copilot", "https://api.githubcopilot.com/mcp/");
        assert!(config.requires_auth());

        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert!(config.requires_auth());
    }

    #[test]
    fn test_requires_auth_localhost_no_auth() {
        // Localhost servers are dev servers, no auth needed
        let config = McpServerConfig::new("local", "http://localhost:8080");
        assert!(!config.requires_auth());

        let config = McpServerConfig::new("local", "http://127.0.0.1:3000/mcp");
        assert!(!config.requires_auth());

        // Even HTTPS localhost doesn't require auth
        let config = McpServerConfig::new("local", "https://localhost:8443");
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_requires_auth_http_remote_no_auth() {
        // HTTP remote servers won't pass validation, but if they existed
        // they wouldn't trigger HTTPS auth detection
        let config = McpServerConfig::new("bad", "http://mcp.example.com");
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_stdio_config_creation() {
        let env = HashMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let config = McpServerConfig::new_stdio(
            "my-server",
            "npx",
            vec!["-y".to_string(), "@modelcontextprotocol/server".to_string()],
            env.clone(),
        );

        assert_eq!(config.name, "my-server");
        assert!(config.url.is_empty());
        assert!(config.enabled);
        assert!(config.oauth.is_none());
        assert!(config.headers.is_empty());

        match &config.transport {
            Some(McpTransportConfig::Stdio {
                command,
                args,
                env: e,
            }) => {
                assert_eq!(command, "npx");
                assert_eq!(
                    args,
                    &["-y".to_string(), "@modelcontextprotocol/server".to_string()]
                );
                assert_eq!(e, &env);
            }
            other => panic!("Expected Stdio transport, got {:?}", other),
        }
    }

    #[test]
    fn test_unix_config_creation() {
        let config = McpServerConfig::new_unix("local-server", "/tmp/mcp.sock");

        assert_eq!(config.name, "local-server");
        assert!(config.url.is_empty());
        assert!(config.enabled);

        match &config.transport {
            Some(McpTransportConfig::Unix { socket_path }) => {
                assert_eq!(socket_path, "/tmp/mcp.sock");
            }
            other => panic!("Expected Unix transport, got {:?}", other),
        }
    }

    #[test]
    fn test_stdio_validation() {
        // Valid stdio config
        let config = McpServerConfig::new_stdio("server", "npx", vec![], HashMap::new());
        assert!(config.validate().is_ok());

        // Invalid: empty command
        let config = McpServerConfig::new_stdio("server", "", vec![], HashMap::new());
        assert!(config.validate().is_err());
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("command"),
            "Error should mention command: {}",
            err
        );

        // Invalid: empty name
        let config = McpServerConfig::new_stdio("", "npx", vec![], HashMap::new());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_unix_validation() {
        // Valid unix config
        let config = McpServerConfig::new_unix("server", "/tmp/mcp.sock");
        assert!(config.validate().is_ok());

        // Invalid: empty socket path
        let config = McpServerConfig::new_unix("server", "");
        assert!(config.validate().is_err());
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("socket"),
            "Error should mention socket: {}",
            err
        );

        // Invalid: empty name
        let config = McpServerConfig::new_unix("", "/tmp/mcp.sock");
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_requires_auth_stdio_never() {
        // Stdio transport should never require auth, even with OAuth configured
        let mut config = McpServerConfig::new_stdio("server", "npx", vec![], HashMap::new());
        assert!(!config.requires_auth());

        // Even if OAuth is set, stdio doesn't use HTTP auth
        config.oauth = Some(OAuthConfig::new("client-123"));
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_requires_auth_unix_never() {
        // Unix transport should never require auth
        let mut config = McpServerConfig::new_unix("server", "/tmp/mcp.sock");
        assert!(!config.requires_auth());

        config.oauth = Some(OAuthConfig::new("client-123"));
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_header_crlf_injection_rejected() {
        let mut headers = HashMap::new();
        headers.insert("X-Good".to_string(), "safe".to_string());
        headers.insert("X-Bad\r\nInjected: true".to_string(), "value".to_string());

        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("not a valid HTTP header name"),
            "Expected RFC 9110 error, got: {err}"
        );
    }

    #[test]
    fn test_header_value_crlf_injection_rejected() {
        let mut headers = HashMap::new();
        headers.insert(
            "X-Header".to_string(),
            "value\r\nInjected: true".to_string(),
        );

        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("invalid characters"),
            "Expected invalid characters error, got: {err}"
        );
    }

    #[test]
    fn test_header_name_with_space_rejected() {
        let headers = HashMap::from([("X Bad".to_string(), "value".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_header_name_with_colon_rejected() {
        let headers = HashMap::from([("X:Bad".to_string(), "value".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_header_name_with_null_byte_rejected() {
        let headers = HashMap::from([("X-Bad\0".to_string(), "value".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_header_empty_name_rejected() {
        let mut headers = HashMap::new();
        headers.insert(String::new(), "value".to_string());

        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("empty"),
            "Expected empty name error, got: {err}"
        );
    }

    #[test]
    fn test_has_custom_auth_header_case_insensitive() {
        let headers = HashMap::from([("authorization".to_string(), "Bearer token".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(config.has_custom_auth_header());

        let headers = HashMap::from([("AUTHORIZATION".to_string(), "Bearer token".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(config.has_custom_auth_header());

        let headers = HashMap::from([("X-Api-Key".to_string(), "key".to_string())]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers);
        assert!(!config.has_custom_auth_header());
    }

    #[test]
    fn test_custom_headers() {
        let headers = HashMap::from([
            ("X-Api-Key".to_string(), "secret".to_string()),
            ("Authorization".to_string(), "Bearer token".to_string()),
        ]);
        let config =
            McpServerConfig::new("server", "https://mcp.example.com").with_headers(headers.clone());

        assert_eq!(config.headers, headers);
        assert_eq!(config.headers.get("X-Api-Key").unwrap(), "secret");
    }

    #[test]
    fn test_transport_config_serde_http() {
        let transport = McpTransportConfig::Http;
        let json = serde_json::to_string(&transport).unwrap();
        assert!(json.contains("\"transport\":\"http\""));

        let parsed: McpTransportConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, McpTransportConfig::Http));
    }

    #[test]
    fn test_transport_config_serde_stdio() {
        let transport = McpTransportConfig::Stdio {
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "server".to_string()],
            env: HashMap::from([("KEY".to_string(), "val".to_string())]),
        };
        let json = serde_json::to_string(&transport).unwrap();
        assert!(json.contains("\"transport\":\"stdio\""));
        assert!(json.contains("\"command\":\"npx\""));

        let parsed: McpTransportConfig = serde_json::from_str(&json).unwrap();
        match parsed {
            McpTransportConfig::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, vec!["-y".to_string(), "server".to_string()]);
                assert_eq!(env.get("KEY").unwrap(), "val");
            }
            other => panic!("Expected Stdio, got {:?}", other),
        }
    }

    #[test]
    fn test_transport_config_serde_unix() {
        let transport = McpTransportConfig::Unix {
            socket_path: "/tmp/mcp.sock".to_string(),
        };
        let json = serde_json::to_string(&transport).unwrap();
        assert!(json.contains("\"transport\":\"unix\""));
        assert!(json.contains("\"socket_path\":\"/tmp/mcp.sock\""));

        let parsed: McpTransportConfig = serde_json::from_str(&json).unwrap();
        match parsed {
            McpTransportConfig::Unix { socket_path } => {
                assert_eq!(socket_path, "/tmp/mcp.sock");
            }
            other => panic!("Expected Unix, got {:?}", other),
        }
    }

    #[test]
    fn test_backward_compat_no_transport_field() {
        // Existing configs without transport field should still deserialize
        let json = r#"{
            "name": "notion",
            "url": "https://mcp.notion.com",
            "enabled": true
        }"#;
        let config: McpServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "notion");
        assert_eq!(config.url, "https://mcp.notion.com");
        assert!(config.transport.is_none());
        assert!(config.headers.is_empty());
        assert!(matches!(
            config.effective_transport(),
            EffectiveTransport::Http
        ));
    }

    #[test]
    fn test_config_roundtrip_with_transport() {
        // Test full roundtrip with stdio transport
        let config = McpServerConfig::new_stdio(
            "test-server",
            "node",
            vec!["server.js".to_string()],
            HashMap::from([("NODE_ENV".to_string(), "production".to_string())]),
        )
        .with_description("A test server");

        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test-server");
        assert!(parsed.url.is_empty());
        assert_eq!(parsed.description.as_deref(), Some("A test server"));

        match &parsed.transport {
            Some(McpTransportConfig::Stdio { command, args, env }) => {
                assert_eq!(command, "node");
                assert_eq!(args, &["server.js".to_string()]);
                assert_eq!(env.get("NODE_ENV").unwrap(), "production");
            }
            other => panic!("Expected Stdio transport, got {:?}", other),
        }

        // Test full roundtrip with unix transport
        let config = McpServerConfig::new_unix("unix-server", "/var/run/mcp.sock");
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "unix-server");
        match &parsed.transport {
            Some(McpTransportConfig::Unix { socket_path }) => {
                assert_eq!(socket_path, "/var/run/mcp.sock");
            }
            other => panic!("Expected Unix transport, got {:?}", other),
        }

        // Test roundtrip with HTTP + headers
        let headers = HashMap::from([("X-Custom".to_string(), "value".to_string())]);
        let config =
            McpServerConfig::new("http-server", "https://mcp.example.com").with_headers(headers);
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "http-server");
        assert!(parsed.transport.is_none());
        assert_eq!(parsed.headers.get("X-Custom").unwrap(), "value");
    }

    // --- Issue 3 regression: is_localhost_url rejects attacker subdomains ---

    #[test]
    fn test_is_localhost_url_rejects_attacker_subdomain() {
        // Before the fix, url.contains("localhost") matched this.
        assert!(
            !is_localhost_url("http://evil.localhost.attacker.com:8080/mcp"),
            "attacker subdomain containing 'localhost' must not be treated as local"
        );
    }

    #[test]
    fn test_is_localhost_url_accepts_real_localhost() {
        assert!(is_localhost_url("http://localhost:8080/mcp"));
        assert!(is_localhost_url("https://localhost/path"));
    }

    #[test]
    fn test_is_localhost_url_accepts_loopback_ip() {
        assert!(is_localhost_url("http://127.0.0.1:3000"));
        assert!(is_localhost_url("http://[::1]:3000"));
    }

    #[test]
    fn test_is_localhost_url_rejects_remote() {
        assert!(!is_localhost_url("https://mcp.example.com"));
        assert!(!is_localhost_url("http://192.168.1.1:8080"));
    }

    #[test]
    fn test_derive_nearai_mcp_url_strips_v1_and_trailing_slash() {
        assert_eq!(
            derive_nearai_mcp_url("https://cloud-api.near.ai/v1"),
            "https://cloud-api.near.ai/mcp"
        );
        assert_eq!(
            derive_nearai_mcp_url("https://cloud-api.near.ai/v1/"),
            "https://cloud-api.near.ai/mcp"
        );
        assert_eq!(
            derive_nearai_mcp_url("https://private.near.ai"),
            "https://private.near.ai/mcp"
        );
    }

    #[test]
    fn test_nearai_mcp_server_from_env_builds_standard_server() {
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: Tests serialize env access with lock_env().
        unsafe {
            std::env::set_var("NEARAI_BASE_URL", "https://cloud-api.near.ai/v1/");
            std::env::set_var("NEARAI_API_KEY", "test-nearai-key");
        }

        let server = nearai_mcp_server_from_env().expect("server from env");
        let catalog = crate::registry::embedded::load_embedded();
        let expected_description = catalog
            .get(super::NEARAI_MCP_REGISTRY_KEY)
            .and_then(|m| (!m.description.is_empty()).then_some(m.description.clone()))
            .unwrap_or_else(|| "Use NEAR AI built-in tools like web search".to_string());
        assert_eq!(server.name, NEARAI_MCP_SERVER_NAME);
        assert_eq!(
            server.description.as_deref(),
            Some(expected_description.as_str())
        );
        assert_eq!(server.url, "https://cloud-api.near.ai/mcp");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Bearer test-nearai-key")
        );

        // SAFETY: Tests serialize env access with lock_env().
        unsafe {
            std::env::remove_var("NEARAI_BASE_URL");
            std::env::remove_var("NEARAI_API_KEY");
        }
    }
}
