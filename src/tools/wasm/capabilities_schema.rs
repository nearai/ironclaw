//! JSON schema for WASM tool capabilities files.
//!
//! External WASM tools declare their required capabilities via a sidecar JSON file
//! (e.g., `slack.capabilities.json`). This module defines the schema for those files
//! and provides conversion to runtime [`Capabilities`].
//!
//! # Example Capabilities File
//!
//! ```json
//! {
//!   "http": {
//!     "allowlist": [
//!       { "host": "slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
//!     ],
//!     "credentials": {
//!       "slack_bot_token": {
//!         "secret_name": "slack_bot_token",
//!         "location": { "type": "bearer" },
//!         "host_patterns": ["slack.com"]
//!       }
//!     },
//!     "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
//!   },
//!   "secrets": {
//!     "allowed_names": ["slack_bot_token"]
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::secrets::{CredentialLocation, CredentialMapping};
use crate::tools::wasm::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WorkspaceCapability,
};

/// Root schema for a capabilities JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesFile {
    /// HTTP request capability.
    #[serde(default)]
    pub http: Option<HttpCapabilitySchema>,

    /// Secret existence checks.
    #[serde(default)]
    pub secrets: Option<SecretsCapabilitySchema>,

    /// Tool invocation via aliases.
    #[serde(default)]
    pub tool_invoke: Option<ToolInvokeCapabilitySchema>,

    /// Workspace file read access.
    #[serde(default)]
    pub workspace: Option<WorkspaceCapabilitySchema>,

    /// Authentication setup instructions.
    /// Used by `ironclaw config` to guide users through auth setup.
    #[serde(default)]
    pub auth: Option<AuthCapabilitySchema>,

    /// Nested capabilities wrapper for channel-level JSON compatibility.
    ///
    /// Channel capabilities files nest tool capabilities under a `"capabilities"` key.
    /// This allows `from_json()`/`from_bytes()` to also parse channel-level JSON;
    /// inner fields are promoted into top-level fields by `resolve_nested()`.
    /// Always `None` after construction via the public parse methods.
    #[serde(default, skip_serializing)]
    pub capabilities: Option<Box<CapabilitiesFile>>,
}

impl CapabilitiesFile {
    /// Parse from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<Self>(json).map(Self::resolve_nested)
    }

    /// Parse from JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<Self>(bytes).map(Self::resolve_nested)
    }

    /// Merge nested `capabilities` wrapper into top-level fields.
    ///
    /// Channel-level JSON nests tool capabilities under `"capabilities"`.
    /// This promotes the inner fields so callers can access them uniformly.
    fn resolve_nested(mut self) -> Self {
        if let Some(inner) = self.capabilities.take() {
            let inner = inner.resolve_nested();
            self.http = self.http.or(inner.http);
            self.secrets = self.secrets.or(inner.secrets);
            self.tool_invoke = self.tool_invoke.or(inner.tool_invoke);
            self.workspace = self.workspace.or(inner.workspace);
            self.auth = self.auth.or(inner.auth);
        }
        self
    }

    /// Convert to runtime Capabilities.
    pub fn to_capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::default();

        if let Some(http) = &self.http {
            caps.http = Some(http.to_http_capability());
        }

        if let Some(secrets) = &self.secrets {
            caps.secrets = Some(SecretsCapability {
                allowed_names: secrets.allowed_names.clone(),
            });
        }

        if let Some(tool_invoke) = &self.tool_invoke {
            caps.tool_invoke = Some(ToolInvokeCapability {
                aliases: tool_invoke.aliases.clone(),
                rate_limit: tool_invoke
                    .rate_limit
                    .as_ref()
                    .map(|r| r.to_rate_limit_config())
                    .unwrap_or_default(),
            });
        }

        if let Some(workspace) = &self.workspace {
            caps.workspace_read = Some(WorkspaceCapability {
                allowed_prefixes: workspace.allowed_prefixes.clone(),
                reader: None, // Injected at runtime
            });
        }

        caps
    }
}

/// HTTP capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpCapabilitySchema {
    /// Allowed endpoint patterns.
    #[serde(default)]
    pub allowlist: Vec<EndpointPatternSchema>,

    /// Credential mappings (key is an identifier, not the secret name).
    #[serde(default)]
    pub credentials: HashMap<String, CredentialMappingSchema>,

    /// Rate limiting configuration.
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,

    /// Maximum request body size in bytes.
    #[serde(default)]
    pub max_request_bytes: Option<usize>,

    /// Maximum response body size in bytes.
    #[serde(default)]
    pub max_response_bytes: Option<usize>,

    /// Request timeout in seconds.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl HttpCapabilitySchema {
    fn to_http_capability(&self) -> HttpCapability {
        let mut cap = HttpCapability {
            allowlist: self
                .allowlist
                .iter()
                .map(|p| p.to_endpoint_pattern())
                .collect(),
            credentials: self
                .credentials
                .values()
                .map(|m| (m.secret_name.clone(), m.to_credential_mapping()))
                .collect(),
            rate_limit: self
                .rate_limit
                .as_ref()
                .map(|r| r.to_rate_limit_config())
                .unwrap_or_default(),
            ..Default::default()
        };

        if let Some(max) = self.max_request_bytes {
            cap.max_request_bytes = max;
        }
        if let Some(max) = self.max_response_bytes {
            cap.max_response_bytes = max;
        }
        if let Some(secs) = self.timeout_secs {
            cap.timeout = Duration::from_secs(secs);
        }

        cap
    }
}

/// Endpoint pattern schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointPatternSchema {
    /// Hostname (e.g., "api.slack.com" or "*.slack.com").
    pub host: String,

    /// Optional path prefix (e.g., "/api/").
    #[serde(default)]
    pub path_prefix: Option<String>,

    /// Allowed HTTP methods (empty = all).
    #[serde(default)]
    pub methods: Vec<String>,
}

impl EndpointPatternSchema {
    fn to_endpoint_pattern(&self) -> EndpointPattern {
        EndpointPattern {
            host: self.host.clone(),
            path_prefix: self.path_prefix.clone(),
            methods: self.methods.clone(),
        }
    }
}

/// Credential mapping schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMappingSchema {
    /// Name of the secret to inject.
    pub secret_name: String,

    /// Where to inject the credential.
    pub location: CredentialLocationSchema,

    /// Host patterns this credential applies to.
    #[serde(default)]
    pub host_patterns: Vec<String>,
}

impl CredentialMappingSchema {
    fn to_credential_mapping(&self) -> CredentialMapping {
        CredentialMapping {
            secret_name: self.secret_name.clone(),
            location: self.location.to_credential_location(),
            host_patterns: self.host_patterns.clone(),
        }
    }
}

/// Credential injection location schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialLocationSchema {
    /// Bearer token in Authorization header.
    Bearer,

    /// Basic auth (password from secret, username in config).
    Basic { username: String },

    /// Custom header.
    Header {
        #[serde(alias = "header_name")]
        name: String,
        #[serde(default)]
        prefix: Option<String>,
    },

    /// Query parameter.
    QueryParam { name: String },

    /// URL/path placeholder replacement.
    UrlPath { placeholder: String },
}

impl CredentialLocationSchema {
    fn to_credential_location(&self) -> CredentialLocation {
        match self {
            CredentialLocationSchema::Bearer => CredentialLocation::AuthorizationBearer,
            CredentialLocationSchema::Basic { username } => {
                CredentialLocation::AuthorizationBasic {
                    username: username.clone(),
                }
            }
            CredentialLocationSchema::Header { name, prefix } => CredentialLocation::Header {
                name: name.clone(),
                prefix: prefix.clone(),
            },
            CredentialLocationSchema::QueryParam { name } => {
                CredentialLocation::QueryParam { name: name.clone() }
            }
            CredentialLocationSchema::UrlPath { placeholder } => CredentialLocation::UrlPath {
                placeholder: placeholder.clone(),
            },
        }
    }
}

/// Rate limit schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitSchema {
    /// Maximum requests per minute.
    #[serde(default = "default_requests_per_minute")]
    pub requests_per_minute: u32,

    /// Maximum requests per hour.
    #[serde(default = "default_requests_per_hour")]
    pub requests_per_hour: u32,
}

fn default_requests_per_minute() -> u32 {
    60
}

fn default_requests_per_hour() -> u32 {
    1000
}

impl RateLimitSchema {
    fn to_rate_limit_config(&self) -> RateLimitConfig {
        RateLimitConfig {
            requests_per_minute: self.requests_per_minute,
            requests_per_hour: self.requests_per_hour,
        }
    }
}

/// Secrets capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretsCapabilitySchema {
    /// Secret names the tool can check existence of (supports glob).
    #[serde(default)]
    pub allowed_names: Vec<String>,
}

/// Tool invocation capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolInvokeCapabilitySchema {
    /// Mapping from alias to real tool name.
    #[serde(default)]
    pub aliases: HashMap<String, String>,

    /// Rate limiting for tool calls.
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,
}

/// Workspace read capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceCapabilitySchema {
    /// Allowed path prefixes (e.g., ["context/", "daily/"]).
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
}

/// Authentication setup schema.
///
/// Tools declare their auth requirements here. The agent uses this to provide
/// generic auth flows without needing service-specific code in the main codebase.
///
/// Supports two auth methods:
/// 1. **OAuth** - Browser-based login (preferred for user-facing services)
/// 2. **Manual** - Copy/paste token from provider's dashboard
///
/// # Example (OAuth)
///
/// ```json
/// {
///   "auth": {
///     "secret_name": "notion_api_token",
///     "display_name": "Notion",
///     "oauth": {
///       "authorization_url": "https://api.notion.com/v1/oauth/authorize",
///       "token_url": "https://api.notion.com/v1/oauth/token",
///       "client_id": "your-client-id",
///       "scopes": []
///     },
///     "env_var": "NOTION_TOKEN"
///   }
/// }
/// ```
///
/// # Example (Manual)
///
/// ```json
/// {
///   "auth": {
///     "secret_name": "openai_api_key",
///     "display_name": "OpenAI",
///     "instructions": "Get your API key from platform.openai.com/api-keys",
///     "setup_url": "https://platform.openai.com/api-keys",
///     "token_hint": "Starts with 'sk-'",
///     "env_var": "OPENAI_API_KEY"
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthCapabilitySchema {
    /// Name of the secret to store (e.g., "notion_api_token").
    /// Must match the secret_name in credentials if HTTP capability is used.
    pub secret_name: String,

    /// Human-readable name for the service (e.g., "Notion", "Slack").
    #[serde(default)]
    pub display_name: Option<String>,

    /// OAuth configuration for browser-based login.
    /// If present, OAuth flow is used instead of manual token entry.
    #[serde(default)]
    pub oauth: Option<OAuthConfigSchema>,

    /// Instructions shown to the user for obtaining credentials (manual flow).
    /// Can include markdown formatting.
    #[serde(default)]
    pub instructions: Option<String>,

    /// URL to open for setting up credentials (manual flow).
    #[serde(default)]
    pub setup_url: Option<String>,

    /// Hint about expected token format (e.g., "Starts with 'sk-'").
    /// Used for validation feedback.
    #[serde(default)]
    pub token_hint: Option<String>,

    /// Environment variable to check before prompting.
    /// If this env var is set, its value is used automatically.
    #[serde(default)]
    pub env_var: Option<String>,

    /// Provider hint for organizing secrets (e.g., "notion", "openai").
    #[serde(default)]
    pub provider: Option<String>,

    /// Validation endpoint to check if the token works.
    /// Tool can specify an endpoint to call for validation.
    #[serde(default)]
    pub validation_endpoint: Option<ValidationEndpointSchema>,
}

/// OAuth 2.0 configuration for browser-based login.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthConfigSchema {
    /// OAuth authorization URL (e.g., "https://api.notion.com/v1/oauth/authorize").
    pub authorization_url: String,

    /// OAuth token exchange URL (e.g., "https://api.notion.com/v1/oauth/token").
    pub token_url: String,

    /// OAuth client ID.
    /// Can be set here or via environment variable (see client_id_env).
    #[serde(default)]
    pub client_id: Option<String>,

    /// Environment variable containing the client ID.
    /// Checked if client_id is not set directly.
    #[serde(default)]
    pub client_id_env: Option<String>,

    /// OAuth client secret (optional, some providers don't require it with PKCE).
    /// Can be set here or via environment variable (see client_secret_env).
    #[serde(default)]
    pub client_secret: Option<String>,

    /// Environment variable containing the client secret.
    /// Checked if client_secret is not set directly.
    #[serde(default)]
    pub client_secret_env: Option<String>,

    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Use PKCE (Proof Key for Code Exchange). Defaults to true.
    /// Required for public clients (CLI tools).
    #[serde(default = "default_true")]
    pub use_pkce: bool,

    /// Additional parameters to include in the authorization URL.
    #[serde(default)]
    pub extra_params: std::collections::HashMap<String, String>,

    /// Field name in token response containing the access token.
    /// Defaults to "access_token".
    #[serde(default = "default_access_token_field")]
    pub access_token_field: String,
}

fn default_true() -> bool {
    true
}

fn default_access_token_field() -> String {
    "access_token".to_string()
}

/// Schema for token validation endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationEndpointSchema {
    /// URL to call for validation (e.g., "https://api.notion.com/v1/users/me").
    pub url: String,

    /// HTTP method (defaults to GET).
    #[serde(default = "default_method")]
    pub method: String,

    /// Expected HTTP status code for success (defaults to 200).
    #[serde(default = "default_success_status")]
    pub success_status: u16,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_success_status() -> u16 {
    200
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::capabilities_schema::{CapabilitiesFile, CredentialLocationSchema};

    #[test]
    fn test_parse_minimal() {
        let json = "{}";
        let caps = CapabilitiesFile::from_json(json).unwrap();
        assert!(caps.http.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_parse_http_allowlist() {
        let json = r#"{
            "http": {
                "allowlist": [
                    { "host": "api.slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
                ]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(http.allowlist.len(), 1);
        assert_eq!(http.allowlist[0].host, "api.slack.com");
        assert_eq!(http.allowlist[0].path_prefix, Some("/api/".to_string()));
        assert_eq!(http.allowlist[0].methods, vec!["GET", "POST"]);
    }

    #[test]
    fn test_parse_credentials() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "slack.com" }],
                "credentials": {
                    "slack": {
                        "secret_name": "slack_bot_token",
                        "location": { "type": "bearer" },
                        "host_patterns": ["slack.com", "*.slack.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(http.credentials.len(), 1);
        let cred = http.credentials.get("slack").unwrap();
        assert_eq!(cred.secret_name, "slack_bot_token");
        assert!(matches!(cred.location, CredentialLocationSchema::Bearer));
        assert_eq!(cred.host_patterns, vec!["slack.com", "*.slack.com"]);
    }

    #[test]
    fn test_parse_custom_header_credential() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.example.com" }],
                "credentials": {
                    "api_key": {
                        "secret_name": "my_api_key",
                        "location": { "type": "header", "name": "X-API-Key", "prefix": "Key " },
                        "host_patterns": ["api.example.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("api_key").unwrap();
        match &cred.location {
            CredentialLocationSchema::Header { name, prefix } => {
                assert_eq!(name, "X-API-Key");
                assert_eq!(prefix, &Some("Key ".to_string()));
            }
            _ => panic!("Expected Header location"),
        }
    }

    #[test]
    fn test_parse_url_path_credential() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.telegram.org" }],
                "credentials": {
                    "telegram_bot": {
                        "secret_name": "telegram_bot_token",
                        "location": {
                            "type": "url_path",
                            "placeholder": "{TELEGRAM_BOT_TOKEN}"
                        },
                        "host_patterns": ["api.telegram.org"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("telegram_bot").unwrap();
        match &cred.location {
            CredentialLocationSchema::UrlPath { placeholder } => {
                assert_eq!(placeholder, "{TELEGRAM_BOT_TOKEN}");
            }
            _ => panic!("Expected UrlPath location"),
        }
    }

    #[test]
    fn test_parse_secrets_capability() {
        let json = r#"{
            "secrets": {
                "allowed_names": ["slack_*", "openai_key"]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let secrets = caps.secrets.unwrap();
        assert_eq!(secrets.allowed_names, vec!["slack_*", "openai_key"]);
    }

    #[test]
    fn test_parse_tool_invoke() {
        let json = r#"{
            "tool_invoke": {
                "aliases": {
                    "search": "brave_search",
                    "calc": "calculator"
                },
                "rate_limit": {
                    "requests_per_minute": 10,
                    "requests_per_hour": 100
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let tool_invoke = caps.tool_invoke.unwrap();
        assert_eq!(
            tool_invoke.aliases.get("search"),
            Some(&"brave_search".to_string())
        );
        let rate = tool_invoke.rate_limit.unwrap();
        assert_eq!(rate.requests_per_minute, 10);
    }

    #[test]
    fn test_parse_workspace() {
        let json = r#"{
            "workspace": {
                "allowed_prefixes": ["context/", "daily/"]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let workspace = caps.workspace.unwrap();
        assert_eq!(workspace.allowed_prefixes, vec!["context/", "daily/"]);
    }

    #[test]
    fn test_to_capabilities() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.slack.com", "path_prefix": "/api/" }],
                "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 500 }
            },
            "secrets": {
                "allowed_names": ["slack_token"]
            }
        }"#;

        let file = CapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert!(caps.http.is_some());
        let http = caps.http.unwrap();
        assert_eq!(http.allowlist.len(), 1);
        assert_eq!(http.rate_limit.requests_per_minute, 50);

        assert!(caps.secrets.is_some());
        let secrets = caps.secrets.unwrap();
        assert!(secrets.is_allowed("slack_token"));
    }

    #[test]
    fn test_full_slack_example() {
        let json = r#"{
            "http": {
                "allowlist": [
                    { "host": "slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
                ],
                "credentials": {
                    "slack_bot_token": {
                        "secret_name": "slack_bot_token",
                        "location": { "type": "bearer" },
                        "host_patterns": ["slack.com"]
                    }
                },
                "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
            },
            "secrets": {
                "allowed_names": ["slack_bot_token"]
            }
        }"#;

        let file = CapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        let http = caps.http.unwrap();
        assert_eq!(http.allowlist[0].host, "slack.com");
        assert!(http.credentials.contains_key("slack_bot_token"));

        let secrets = caps.secrets.unwrap();
        assert!(secrets.is_allowed("slack_bot_token"));
    }

    #[test]
    fn test_parse_auth_capability() {
        let json = r#"{
            "auth": {
                "secret_name": "notion_api_token",
                "display_name": "Notion",
                "instructions": "Create an integration at notion.so/my-integrations",
                "setup_url": "https://www.notion.so/my-integrations",
                "token_hint": "Starts with 'secret_' or 'ntn_'",
                "env_var": "NOTION_TOKEN",
                "provider": "notion",
                "validation_endpoint": {
                    "url": "https://api.notion.com/v1/users/me",
                    "method": "GET",
                    "success_status": 200
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let auth = caps.auth.unwrap();
        assert_eq!(auth.secret_name, "notion_api_token");
        assert_eq!(auth.display_name, Some("Notion".to_string()));
        assert_eq!(auth.env_var, Some("NOTION_TOKEN".to_string()));
        assert_eq!(auth.provider, Some("notion".to_string()));

        let validation = auth.validation_endpoint.unwrap();
        assert_eq!(validation.url, "https://api.notion.com/v1/users/me");
        assert_eq!(validation.method, "GET");
        assert_eq!(validation.success_status, 200);
    }

    #[test]
    fn test_parse_auth_minimal() {
        let json = r#"{
            "auth": {
                "secret_name": "my_api_key"
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let auth = caps.auth.unwrap();
        assert_eq!(auth.secret_name, "my_api_key");
        assert!(auth.display_name.is_none());
        assert!(auth.setup_url.is_none());
    }

    // ── Category 1: Header field name alias ─────────────────────────────

    #[test]
    fn test_header_location_with_name_field() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "discord.com" }],
                "credentials": {
                    "bot_token": {
                        "secret_name": "discord_bot_token",
                        "location": { "type": "header", "name": "Authorization", "prefix": "Bot " },
                        "host_patterns": ["discord.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("bot_token").unwrap();
        match &cred.location {
            CredentialLocationSchema::Header { name, prefix } => {
                assert_eq!(name, "Authorization");
                assert_eq!(prefix, &Some("Bot ".to_string()));
            }
            _ => panic!("Expected Header location"),
        }
    }

    #[test]
    fn test_header_location_with_header_name_alias() {
        // Uses "header_name" instead of "name" — should parse via serde alias
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "discord.com" }],
                "credentials": {
                    "bot_token": {
                        "secret_name": "discord_bot_token",
                        "location": { "type": "header", "header_name": "Authorization", "prefix": "Bot " },
                        "host_patterns": ["discord.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("bot_token").unwrap();
        match &cred.location {
            CredentialLocationSchema::Header { name, prefix } => {
                assert_eq!(name, "Authorization");
                assert_eq!(prefix, &Some("Bot ".to_string()));
            }
            _ => panic!("Expected Header location"),
        }
    }

    #[test]
    fn test_discord_capabilities_file_parses() {
        // Full Discord capabilities JSON — tests end-to-end parsing
        let json = r#"{
            "type": "channel",
            "name": "discord",
            "description": "Discord channel",
            "setup": {
                "required_secrets": [
                    {
                        "name": "discord_bot_token",
                        "prompt": "Enter your Discord Bot Token",
                        "optional": false
                    },
                    {
                        "name": "discord_public_key",
                        "prompt": "Enter your Discord Public Key",
                        "optional": false
                    }
                ]
            },
            "capabilities": {
                "http": {
                    "allowlist": [{ "host": "discord.com", "path_prefix": "/api/v10" }],
                    "credentials": {
                        "discord_bot_token": {
                            "secret_name": "discord_bot_token",
                            "location": { "type": "header", "name": "Authorization", "prefix": "Bot " },
                            "host_patterns": ["discord.com"]
                        }
                    }
                }
            },
            "config": {
                "require_signature_verification": true
            }
        }"#;

        // This must not panic — parsing should succeed
        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert!(http.credentials.contains_key("discord_bot_token"));
    }

    #[test]
    fn test_header_location_missing_name_fails() {
        // Neither "name" nor "header_name" provided — should fail
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "example.com" }],
                "credentials": {
                    "api_key": {
                        "secret_name": "my_key",
                        "location": { "type": "header", "prefix": "Key " },
                        "host_patterns": ["example.com"]
                    }
                }
            }
        }"#;

        assert!(
            CapabilitiesFile::from_json(json).is_err(),
            "Header without name or header_name should fail deserialization"
        );
    }

    // ── resolve_nested tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_nested_outer_takes_precedence() {
        // Outer http should win over inner http
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "outer.example.com" }]
            },
            "capabilities": {
                "http": {
                    "allowlist": [{ "host": "inner.example.com" }]
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(
            http.allowlist[0].host, "outer.example.com",
            "Outer http should take precedence over inner"
        );
    }

    #[test]
    fn test_resolve_nested_doubly_nested() {
        // capabilities.capabilities.http should resolve to top-level
        let json = r#"{
            "capabilities": {
                "capabilities": {
                    "http": {
                        "allowlist": [{ "host": "deep.example.com" }]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(
            http.allowlist[0].host, "deep.example.com",
            "Doubly-nested capabilities should be resolved"
        );
    }

    #[test]
    fn test_resolve_nested_all_fields_promoted() {
        // Inner has secrets, workspace, and auth — all should be promoted
        let json = r#"{
            "capabilities": {
                "secrets": {
                    "allowed_names": ["my_secret"]
                },
                "workspace": {
                    "allowed_prefixes": ["data/"]
                },
                "auth": {
                    "secret_name": "my_auth_token"
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        assert!(caps.secrets.is_some(), "secrets should be promoted");
        assert!(caps.workspace.is_some(), "workspace should be promoted");
        assert!(caps.auth.is_some(), "auth should be promoted");

        assert_eq!(caps.secrets.unwrap().allowed_names, vec!["my_secret"]);
        assert_eq!(caps.workspace.unwrap().allowed_prefixes, vec!["data/"]);
        assert_eq!(caps.auth.unwrap().secret_name, "my_auth_token");
    }

    #[test]
    fn test_resolve_nested_empty_capabilities_noop() {
        // Empty inner capabilities should not clobber outer http
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "preserved.example.com" }]
            },
            "capabilities": {}
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(
            http.allowlist[0].host, "preserved.example.com",
            "Empty inner capabilities should not clobber outer http"
        );
    }
}
