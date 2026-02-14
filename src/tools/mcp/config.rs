//! MCP server configuration.
//!
//! Stores configuration for connecting to hosted MCP servers.
//! Configuration is persisted at ~/.ironclaw/mcp-servers.json.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::tool::ToolError;

/// Configuration for connecting to a remote MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server (e.g., "notion", "github").
    pub name: String,

    /// Server URL (must be HTTPS for remote servers).
    pub url: String,

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

    /// Validate the server configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.is_empty() {
            return Err(ConfigError::InvalidConfig {
                reason: "Server name cannot be empty".to_string(),
            });
        }

        if self.url.is_empty() {
            return Err(ConfigError::InvalidConfig {
                reason: "Server URL cannot be empty".to_string(),
            });
        }

        // Remote servers must use HTTPS (localhost is allowed for development)
        let url_lower = self.url.to_lowercase();
        let is_localhost = url_lower.contains("localhost") || url_lower.contains("127.0.0.1");
        if !is_localhost && !url_lower.starts_with("https://") {
            return Err(ConfigError::InvalidConfig {
                reason: "Remote MCP servers must use HTTPS".to_string(),
            });
        }

        Ok(())
    }

    /// Check if this server requires authentication.
    ///
    /// Returns true if OAuth is pre-configured OR if this is a remote HTTPS server
    /// (which likely supports Dynamic Client Registration even without pre-configured OAuth).
    pub fn requires_auth(&self) -> bool {
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
    pub fn refresh_token_secret_name(&self) -> String {
        format!("mcp_{}_refresh_token", self.name)
    }

    /// Get the secret name used to store the DCR client ID.
    pub fn client_id_secret_name(&self) -> String {
        format!("mcp_{}_client_id", self.name)
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

/// Get the default MCP servers configuration path.
pub fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("mcp-servers.json")
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
    fs::write(path, content).await?;

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
fn is_localhost_url(url: &str) -> bool {
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

    #[test]
    fn test_token_secret_names() {
        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert_eq!(config.token_secret_name(), "mcp_notion_access_token");
        assert_eq!(
            config.refresh_token_secret_name(),
            "mcp_notion_refresh_token"
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
}
