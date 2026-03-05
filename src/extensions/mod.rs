//! Lifecycle management for extensions: discovery, installation, authentication,
//! and activation of channels, tools, and MCP servers.
//!
//! Extensions are the user-facing abstraction that unifies three runtime kinds:
//! - **Channels** (Telegram, Slack, Discord) — messaging integrations (WASM)
//! - **Tools** — sandboxed capabilities (WASM)
//! - **MCP servers** — external API integrations via Model Context Protocol
//!
//! The agent can search a built-in registry (or discover online), install,
//! authenticate, and activate extensions at runtime without CLI commands.
//!
//! ```text
//!  User: "add telegram"
//!    -> tool_search("telegram")    -> finds channel in registry
//!    -> tool_install("telegram")   -> copies bundled WASM to channels dir
//!    -> tool_activate("telegram")  -> configures credentials, starts channel
//! ```

pub mod discovery;
pub mod manager;
pub mod registry;

pub use discovery::OnlineDiscovery;
pub use manager::ExtensionManager;
pub use registry::ExtensionRegistry;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

/// The kind of extension, determining how it's installed, authenticated, and activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    /// Hosted MCP server, HTTP transport, OAuth 2.1 auth.
    McpServer,
    /// Sandboxed WASM module, file-based, capabilities auth.
    WasmTool,
    /// WASM channel module with hot-activation support.
    WasmChannel,
}

impl std::fmt::Display for ExtensionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionKind::McpServer => write!(f, "mcp_server"),
            ExtensionKind::WasmTool => write!(f, "wasm_tool"),
            ExtensionKind::WasmChannel => write!(f, "wasm_channel"),
        }
    }
}

/// A registry entry describing a known or discovered extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Unique identifier (e.g., "notion", "weather", "telegram").
    pub name: String,
    /// Human-readable name (e.g., "Notion", "Weather Tool").
    pub display_name: String,
    /// What kind of extension this is.
    pub kind: ExtensionKind,
    /// Short description of what this extension does.
    pub description: String,
    /// Search keywords beyond the name.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Where to get this extension.
    pub source: ExtensionSource,
    /// Fallback source when the primary source fails (e.g., download 404 → build from source).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_source: Option<Box<ExtensionSource>>,
    /// How authentication works.
    pub auth_hint: AuthHint,
}

/// Where the extension binary or server lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionSource {
    /// URL to a hosted MCP server.
    McpUrl { url: String },
    /// Downloadable WASM binary.
    WasmDownload {
        wasm_url: String,
        #[serde(default)]
        capabilities_url: Option<String>,
    },
    /// Build from local source directory.
    WasmBuildable {
        #[serde(alias = "repo_url")]
        source_dir: String,
        #[serde(default)]
        build_dir: Option<String>,
        /// Crate name used to locate the build artifact binary.
        #[serde(default)]
        crate_name: Option<String>,
    },
    /// Discovered online (not yet validated for a specific source type).
    Discovered { url: String },
}

/// Hint about what authentication method is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthHint {
    /// MCP server supports Dynamic Client Registration (zero-config OAuth).
    Dcr,
    /// MCP server needs a pre-configured OAuth client_id.
    OAuthPreConfigured {
        /// URL where the user can create an OAuth app.
        setup_url: String,
    },
    /// WASM tool has auth defined in its capabilities.json file.
    CapabilitiesAuth,
    /// No authentication needed.
    None,
}

/// Where a search result came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultSource {
    /// From the built-in curated registry.
    Registry,
    /// From online discovery (validated).
    Discovered,
}

/// Result of searching for extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The registry entry.
    #[serde(flatten)]
    pub entry: RegistryEntry,
    /// Where this result came from.
    pub source: ResultSource,
    /// Whether the endpoint was validated (for discovered entries).
    #[serde(default)]
    pub validated: bool,
}

/// Result of installing an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub name: String,
    pub kind: ExtensionKind,
    pub message: String,
}

/// Auth readiness state for the extensions list UI.
///
/// Used by `check_tool_auth_status` and `check_channel_auth_status` to
/// communicate a tool's credential state to the list handler without
/// ambiguous `(bool, bool)` tuples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAuthState {
    /// Token/credentials are present — ready to use.
    Ready,
    /// Auth section exists but the access token is missing (OAuth not completed).
    NeedsAuth,
    /// Setup credentials (client_id/secret) must be configured before OAuth can start.
    NeedsSetup,
    /// No auth configuration at all (no capabilities or auth section).
    NoAuth,
}

/// The typed auth status, carrying only the data relevant to each state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    /// Authentication is complete; no further action needed.
    Authenticated,
    /// No authentication is required for this extension.
    NoAuthRequired,
    /// OAuth flow started — user must open `auth_url` in their browser.
    AwaitingAuthorization {
        auth_url: String,
        callback_type: String,
    },
    /// Waiting for user to provide a token/key manually.
    AwaitingToken {
        instructions: String,
        setup_url: Option<String>,
    },
    /// OAuth client credentials need to be configured before auth can proceed.
    NeedsSetup {
        instructions: String,
        setup_url: Option<String>,
    },
}

impl AuthStatus {
    /// The wire-format status string (backward-compatible with JS consumers).
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthStatus::Authenticated => "authenticated",
            AuthStatus::NoAuthRequired => "no_auth_required",
            AuthStatus::AwaitingAuthorization { .. } => "awaiting_authorization",
            AuthStatus::AwaitingToken { .. } => "awaiting_token",
            AuthStatus::NeedsSetup { .. } => "needs_setup",
        }
    }
}

/// Result of authenticating an extension.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub name: String,
    pub kind: ExtensionKind,
    pub status: AuthStatus,
}

impl AuthResult {
    // ── Constructors ──────────────────────────────────────────────────

    pub fn authenticated(name: impl Into<String>, kind: ExtensionKind) -> Self {
        Self {
            name: name.into(),
            kind,
            status: AuthStatus::Authenticated,
        }
    }

    pub fn no_auth_required(name: impl Into<String>, kind: ExtensionKind) -> Self {
        Self {
            name: name.into(),
            kind,
            status: AuthStatus::NoAuthRequired,
        }
    }

    pub fn awaiting_authorization(
        name: impl Into<String>,
        kind: ExtensionKind,
        auth_url: String,
        callback_type: String,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            status: AuthStatus::AwaitingAuthorization {
                auth_url,
                callback_type,
            },
        }
    }

    pub fn awaiting_token(
        name: impl Into<String>,
        kind: ExtensionKind,
        instructions: String,
        setup_url: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            status: AuthStatus::AwaitingToken {
                instructions,
                setup_url,
            },
        }
    }

    pub fn needs_setup(
        name: impl Into<String>,
        kind: ExtensionKind,
        instructions: String,
        setup_url: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            status: AuthStatus::NeedsSetup {
                instructions,
                setup_url,
            },
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────

    pub fn is_authenticated(&self) -> bool {
        matches!(self.status, AuthStatus::Authenticated)
    }

    pub fn auth_url(&self) -> Option<&str> {
        match &self.status {
            AuthStatus::AwaitingAuthorization { auth_url, .. } => Some(auth_url),
            _ => None,
        }
    }

    pub fn callback_type(&self) -> Option<&str> {
        match &self.status {
            AuthStatus::AwaitingAuthorization { callback_type, .. } => Some(callback_type),
            _ => None,
        }
    }

    pub fn instructions(&self) -> Option<&str> {
        match &self.status {
            AuthStatus::AwaitingToken { instructions, .. }
            | AuthStatus::NeedsSetup { instructions, .. } => Some(instructions),
            _ => None,
        }
    }

    pub fn setup_url(&self) -> Option<&str> {
        match &self.status {
            AuthStatus::AwaitingToken { setup_url, .. }
            | AuthStatus::NeedsSetup { setup_url, .. } => setup_url.as_deref(),
            _ => None,
        }
    }

    pub fn is_awaiting_token(&self) -> bool {
        matches!(self.status, AuthStatus::AwaitingToken { .. })
    }

    pub fn status_str(&self) -> &'static str {
        self.status.as_str()
    }
}

/// Serialize `AuthResult` to the same flat JSON shape the JS frontend expects.
impl Serialize for AuthResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Count fields: name + kind + status + optional fields
        let optional_count = self.auth_url().is_some() as usize
            + self.callback_type().is_some() as usize
            + self.instructions().is_some() as usize
            + self.setup_url().is_some() as usize;
        let mut map = serializer.serialize_map(Some(4 + optional_count))?;

        map.serialize_entry("name", &self.name)?;
        map.serialize_entry("kind", &self.kind)?;
        if let Some(url) = self.auth_url() {
            map.serialize_entry("auth_url", url)?;
        }
        if let Some(cb) = self.callback_type() {
            map.serialize_entry("callback_type", cb)?;
        }
        if let Some(inst) = self.instructions() {
            map.serialize_entry("instructions", inst)?;
        }
        if let Some(url) = self.setup_url() {
            map.serialize_entry("setup_url", url)?;
        }
        map.serialize_entry("awaiting_token", &self.is_awaiting_token())?;
        map.serialize_entry("status", self.status_str())?;
        map.end()
    }
}

/// Deserialize from the flat JSON shape back into the typed enum.
impl<'de> Deserialize<'de> for AuthResult {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Flat helper matching the old JSON shape.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Raw {
            name: String,
            kind: ExtensionKind,
            #[serde(default)]
            auth_url: Option<String>,
            #[serde(default)]
            callback_type: Option<String>,
            #[serde(default)]
            instructions: Option<String>,
            #[serde(default)]
            setup_url: Option<String>,
            #[serde(default)]
            awaiting_token: bool,
            status: String,
        }

        let raw = Raw::deserialize(deserializer)?;
        let status = match raw.status.as_str() {
            "authenticated" => AuthStatus::Authenticated,
            "no_auth_required" => AuthStatus::NoAuthRequired,
            "awaiting_authorization" => AuthStatus::AwaitingAuthorization {
                auth_url: raw.auth_url.unwrap_or_default(),
                callback_type: raw.callback_type.unwrap_or_default(),
            },
            "awaiting_token" => AuthStatus::AwaitingToken {
                instructions: raw.instructions.unwrap_or_default(),
                setup_url: raw.setup_url,
            },
            "needs_setup" => AuthStatus::NeedsSetup {
                instructions: raw.instructions.unwrap_or_default(),
                setup_url: raw.setup_url,
            },
            other => {
                return Err(serde::de::Error::unknown_variant(
                    other,
                    &[
                        "authenticated",
                        "no_auth_required",
                        "awaiting_authorization",
                        "awaiting_token",
                        "needs_setup",
                    ],
                ));
            }
        };
        Ok(AuthResult {
            name: raw.name,
            kind: raw.kind,
            status,
        })
    }
}

/// Result of activating an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateResult {
    pub name: String,
    pub kind: ExtensionKind,
    /// Names of tools that were loaded/registered.
    pub tools_loaded: Vec<String>,
    pub message: String,
}

fn default_true() -> bool {
    true
}

/// An installed extension with its current status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub name: String,
    pub kind: ExtensionKind,
    /// Human-readable display name (e.g. "Telegram Channel" vs "Telegram Tool").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Server or source URL (e.g. MCP server endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub active: bool,
    /// Tool names if active.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether this extension has a setup schema (required_secrets) that can be configured.
    #[serde(default)]
    pub needs_setup: bool,
    /// Whether this extension has an auth configuration (OAuth or manual token).
    #[serde(default)]
    pub has_auth: bool,
    /// Whether this extension is installed locally (false = available in registry but not installed).
    #[serde(default = "default_true")]
    pub installed: bool,
    /// Last activation error for WASM channels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
}

/// Error type for extension operations.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Extension not found: {0}")]
    NotFound(String),

    #[error("Extension already installed: {0}")]
    AlreadyInstalled(String),

    #[error("Extension not installed: {0}")]
    NotInstalled(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Activation failed: {0}")]
    ActivationFailed(String),

    #[error("Installation failed: {0}")]
    InstallFailed(String),

    #[error("Discovery failed: {0}")]
    DiscoveryFailed(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Primary install failed: {primary}; fallback install also failed: {fallback}")]
    FallbackFailed {
        primary: Box<ExtensionError>,
        fallback: Box<ExtensionError>,
    },

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_result_authenticated_round_trip() {
        let result = AuthResult::authenticated("gmail", ExtensionKind::WasmTool);
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["status"], "authenticated");
        assert_eq!(json["name"], "gmail");
        assert_eq!(json["kind"], "wasm_tool");
        assert_eq!(json["awaiting_token"], false);
        assert!(json.get("auth_url").is_none());
        assert!(json.get("instructions").is_none());

        let back: AuthResult = serde_json::from_value(json).unwrap();
        assert!(back.is_authenticated());
        assert!(back.auth_url().is_none());
    }

    #[test]
    fn auth_result_awaiting_authorization_round_trip() {
        let result = AuthResult::awaiting_authorization(
            "google-drive",
            ExtensionKind::WasmTool,
            "https://accounts.google.com/o/oauth2/v2/auth?state=abc".to_string(),
            "local".to_string(),
        );
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["status"], "awaiting_authorization");
        assert_eq!(
            json["auth_url"],
            "https://accounts.google.com/o/oauth2/v2/auth?state=abc"
        );
        assert_eq!(json["callback_type"], "local");
        assert_eq!(json["awaiting_token"], false);

        let back: AuthResult = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.auth_url(),
            Some("https://accounts.google.com/o/oauth2/v2/auth?state=abc")
        );
        assert_eq!(back.callback_type(), Some("local"));
        assert!(!back.is_authenticated());
    }

    #[test]
    fn auth_result_awaiting_token_round_trip() {
        let result = AuthResult::awaiting_token(
            "telegram",
            ExtensionKind::WasmChannel,
            "Enter your bot token".to_string(),
            None,
        );
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["status"], "awaiting_token");
        assert_eq!(json["instructions"], "Enter your bot token");
        assert_eq!(json["awaiting_token"], true);
        assert!(json.get("auth_url").is_none());

        let back: AuthResult = serde_json::from_value(json).unwrap();
        assert!(back.is_awaiting_token());
        assert_eq!(back.instructions(), Some("Enter your bot token"));
    }

    #[test]
    fn auth_result_needs_setup_round_trip() {
        let result = AuthResult::needs_setup(
            "custom-tool",
            ExtensionKind::WasmTool,
            "Configure OAuth credentials in the Setup tab.".to_string(),
            Some("https://console.cloud.google.com".to_string()),
        );
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["status"], "needs_setup");
        assert_eq!(json["setup_url"], "https://console.cloud.google.com");
        assert_eq!(json["awaiting_token"], false);

        let back: AuthResult = serde_json::from_value(json).unwrap();
        assert!(!back.is_authenticated());
        assert!(!back.is_awaiting_token());
        assert_eq!(back.setup_url(), Some("https://console.cloud.google.com"));
    }

    #[test]
    fn auth_result_no_auth_required_round_trip() {
        let result = AuthResult::no_auth_required("echo", ExtensionKind::WasmTool);
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["status"], "no_auth_required");
        assert_eq!(json["awaiting_token"], false);

        let back: AuthResult = serde_json::from_value(json).unwrap();
        assert!(!back.is_authenticated());
        assert_eq!(back.status, AuthStatus::NoAuthRequired);
    }

    #[test]
    fn auth_status_type_safety() {
        // AwaitingAuthorization always has auth_url
        let result = AuthResult::awaiting_authorization(
            "test",
            ExtensionKind::WasmTool,
            "https://example.com".to_string(),
            "local".to_string(),
        );
        assert!(result.auth_url().is_some());
        assert!(!result.is_awaiting_token());

        // Authenticated never has auth_url
        let result = AuthResult::authenticated("test", ExtensionKind::WasmTool);
        assert!(result.auth_url().is_none());
        assert!(result.instructions().is_none());
        assert!(result.setup_url().is_none());
    }
}
