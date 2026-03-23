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
    /// External channel via channel-relay service (Slack, etc.).
    ChannelRelay,
    /// ACP-compliant coding agent (Goose, Codex, Gemini CLI, etc.).
    AcpAgent,
}

impl std::fmt::Display for ExtensionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionKind::McpServer => write!(f, "mcp_server"),
            ExtensionKind::WasmTool => write!(f, "wasm_tool"),
            ExtensionKind::WasmChannel => write!(f, "wasm_channel"),
            ExtensionKind::ChannelRelay => write!(f, "channel_relay"),
            ExtensionKind::AcpAgent => write!(f, "acp_agent"),
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
    /// Extension version (semver), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
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
    /// External channel via channel-relay service.
    ChannelRelay { relay_url: String },
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
    /// OAuth via channel-relay service.
    ChannelRelayOAuth,
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

/// Result of upgrading one or more extensions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpgradeResult {
    /// Per-extension upgrade outcomes.
    pub results: Vec<UpgradeOutcome>,
    /// Summary message.
    pub message: String,
}

/// Outcome for a single extension upgrade.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpgradeOutcome {
    pub name: String,
    pub kind: ExtensionKind,
    /// What happened: "upgraded", "already_up_to_date", "failed", "not_in_registry".
    pub status: String,
    /// Human-readable detail.
    pub detail: String,
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

/// Result of configuring secrets for an extension.
///
/// Returned by `ExtensionManager::configure()`, the single entrypoint
/// for providing secrets to any extension (chat auth, gateway setup, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationChallenge {
    /// One-time code the user must send back to the integration.
    pub code: String,
    /// Human-readable instructions for completing verification.
    pub instructions: String,
    /// Deep-link or shortcut URL that prefills the verification payload when supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigureResult {
    /// Human-readable status message.
    pub message: String,
    /// Whether the extension was successfully activated after configuration.
    pub activated: bool,
    /// Whether a restart is required for the new configuration to take effect.
    pub restart_required: bool,
    /// OAuth authorization URL (if OAuth flow was started).
    pub auth_url: Option<String>,
    /// Pending manual verification challenge (for Telegram owner binding, etc.).
    pub verification: Option<VerificationChallenge>,
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
    /// Whether this extension has a setup schema (required_secrets/required_fields) that can be configured.
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
    /// Extension version from capabilities file (semver).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
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

    #[error("Server does not support OAuth: {0}")]
    AuthNotSupported(String),

    #[error("Activation failed: {0}")]
    ActivationFailed(String),

    #[error("Authentication required")]
    AuthRequired,

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

    #[error("Token validation failed: {0}")]
    ValidationFailed(String),

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

    // ── ExtensionKind ────────────────────────────────────────────────

    #[test]
    fn extension_kind_display() {
        assert_eq!(ExtensionKind::McpServer.to_string(), "mcp_server");
        assert_eq!(ExtensionKind::WasmTool.to_string(), "wasm_tool");
        assert_eq!(ExtensionKind::WasmChannel.to_string(), "wasm_channel");
    }

    #[test]
    fn extension_kind_serde_roundtrip() {
        for kind in [
            ExtensionKind::McpServer,
            ExtensionKind::WasmTool,
            ExtensionKind::WasmChannel,
        ] {
            let json = serde_json::to_value(kind).unwrap();
            let back: ExtensionKind = serde_json::from_value(json).unwrap();
            assert_eq!(back, kind);
        }
        // Verify the serialized strings match rename_all = "snake_case"
        assert_eq!(
            serde_json::to_value(ExtensionKind::McpServer).unwrap(),
            "mcp_server"
        );
        assert_eq!(
            serde_json::to_value(ExtensionKind::WasmTool).unwrap(),
            "wasm_tool"
        );
        assert_eq!(
            serde_json::to_value(ExtensionKind::WasmChannel).unwrap(),
            "wasm_channel"
        );
    }

    // ── ExtensionSource ──────────────────────────────────────────────

    #[test]
    fn extension_source_serde_mcp_url() {
        let src = ExtensionSource::McpUrl {
            url: "https://mcp.example.com".to_string(),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["type"], "mcp_url");
        assert_eq!(json["url"], "https://mcp.example.com");
        let back: ExtensionSource = serde_json::from_value(json).unwrap();
        assert!(
            matches!(back, ExtensionSource::McpUrl { url } if url == "https://mcp.example.com")
        );
    }

    #[test]
    fn extension_source_serde_wasm_download() {
        let src = ExtensionSource::WasmDownload {
            wasm_url: "https://cdn.example.com/tool.wasm".to_string(),
            capabilities_url: Some("https://cdn.example.com/caps.json".to_string()),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["type"], "wasm_download");
        assert_eq!(json["wasm_url"], "https://cdn.example.com/tool.wasm");
        assert_eq!(
            json["capabilities_url"],
            "https://cdn.example.com/caps.json"
        );
        let back: ExtensionSource = serde_json::from_value(json).unwrap();
        assert!(
            matches!(back, ExtensionSource::WasmDownload { capabilities_url: Some(c), .. } if c.contains("caps.json"))
        );
    }

    #[test]
    fn extension_source_serde_wasm_buildable() {
        let src = ExtensionSource::WasmBuildable {
            source_dir: "/home/user/tools/my-tool".to_string(),
            build_dir: Some("target/wasm32-wasip2/release".to_string()),
            crate_name: Some("my_tool".to_string()),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["type"], "wasm_buildable");
        assert_eq!(json["source_dir"], "/home/user/tools/my-tool");
        let back: ExtensionSource = serde_json::from_value(json).unwrap();
        assert!(
            matches!(back, ExtensionSource::WasmBuildable { source_dir, .. } if source_dir.contains("my-tool"))
        );
    }

    #[test]
    fn extension_source_serde_discovered() {
        let src = ExtensionSource::Discovered {
            url: "https://discovered.example.com".to_string(),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["type"], "discovered");
        let back: ExtensionSource = serde_json::from_value(json).unwrap();
        assert!(matches!(back, ExtensionSource::Discovered { url } if url.contains("discovered")));
    }

    // ── AuthHint ─────────────────────────────────────────────────────

    #[test]
    fn auth_hint_serde_all_variants() {
        // Dcr
        let json = serde_json::to_value(&AuthHint::Dcr).unwrap();
        assert_eq!(json["type"], "dcr");
        let back: AuthHint = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AuthHint::Dcr));

        // OAuthPreConfigured
        let hint = AuthHint::OAuthPreConfigured {
            setup_url: "https://dev.example.com/apps".to_string(),
        };
        let json = serde_json::to_value(&hint).unwrap();
        assert_eq!(json["type"], "o_auth_pre_configured");
        assert_eq!(json["setup_url"], "https://dev.example.com/apps");
        let back: AuthHint = serde_json::from_value(json).unwrap();
        assert!(
            matches!(back, AuthHint::OAuthPreConfigured { setup_url } if setup_url.contains("dev.example"))
        );

        // CapabilitiesAuth
        let json = serde_json::to_value(&AuthHint::CapabilitiesAuth).unwrap();
        assert_eq!(json["type"], "capabilities_auth");
        let back: AuthHint = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AuthHint::CapabilitiesAuth));

        // None
        let json = serde_json::to_value(&AuthHint::None).unwrap();
        assert_eq!(json["type"], "none");
        let back: AuthHint = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AuthHint::None));
    }

    // ── SearchResult ─────────────────────────────────────────────────

    #[test]
    fn search_result_serde_registry_source() {
        // SearchResult uses #[serde(flatten)] on entry, which means
        // RegistryEntry.source (ExtensionSource) and SearchResult.source
        // (ResultSource) collide on the "source" key. The last writer wins
        // during serialization, so we test serialize-only (no roundtrip).
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Notion integration".to_string(),
            keywords: vec!["notes".to_string(), "wiki".to_string()],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.notion.so".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
            version: None,
        };
        let sr = SearchResult {
            entry,
            source: ResultSource::Registry,
            validated: false,
        };
        let json = serde_json::to_value(&sr).unwrap();
        assert_eq!(json["name"], "notion");
        assert_eq!(json["kind"], "mcp_server");
        assert_eq!(json["description"], "Notion integration");
        assert_eq!(json["validated"], false);
        // The flattened entry fields are present at the top level
        assert!(json.get("auth_hint").is_some());
        assert_eq!(json["keywords"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn search_result_serde_discovered_source() {
        let entry = RegistryEntry {
            name: "custom-api".to_string(),
            display_name: "Custom API".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Discovered MCP server".to_string(),
            keywords: vec![],
            source: ExtensionSource::Discovered {
                url: "https://custom.example.com/.well-known/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
            version: None,
        };
        let sr = SearchResult {
            entry,
            source: ResultSource::Discovered,
            validated: true,
        };
        let json = serde_json::to_value(&sr).unwrap();
        assert_eq!(json["name"], "custom-api");
        assert_eq!(json["display_name"], "Custom API");
        assert_eq!(json["validated"], true);
        assert!(json.get("keywords").is_some());
    }

    // ── InstallResult ────────────────────────────────────────────────

    #[test]
    fn install_result_serde_roundtrip() {
        let ir = InstallResult {
            name: "weather".to_string(),
            kind: ExtensionKind::WasmTool,
            message: "Installed successfully".to_string(),
        };
        let json = serde_json::to_value(&ir).unwrap();
        assert_eq!(json["name"], "weather");
        assert_eq!(json["kind"], "wasm_tool");
        assert_eq!(json["message"], "Installed successfully");
        let back: InstallResult = serde_json::from_value(json).unwrap();
        assert_eq!(back.name, "weather");
        assert_eq!(back.kind, ExtensionKind::WasmTool);
    }

    // ── ActivateResult ───────────────────────────────────────────────

    #[test]
    fn activate_result_serde_roundtrip() {
        let ar = ActivateResult {
            name: "slack".to_string(),
            kind: ExtensionKind::WasmChannel,
            tools_loaded: vec!["send_message".to_string(), "read_channel".to_string()],
            message: "Activated with 2 tools".to_string(),
        };
        let json = serde_json::to_value(&ar).unwrap();
        assert_eq!(json["name"], "slack");
        assert_eq!(json["kind"], "wasm_channel");
        assert_eq!(json["tools_loaded"].as_array().unwrap().len(), 2);
        let back: ActivateResult = serde_json::from_value(json).unwrap();
        assert_eq!(back.tools_loaded, vec!["send_message", "read_channel"]);
    }

    // ── InstalledExtension ───────────────────────────────────────────

    #[test]
    fn installed_extension_serde_defaults() {
        // Minimal JSON: optional fields absent, defaults kick in
        let json = serde_json::json!({
            "name": "echo",
            "kind": "wasm_tool",
            "authenticated": false,
            "active": false,
        });
        let ext: InstalledExtension = serde_json::from_value(json).unwrap();
        assert_eq!(ext.name, "echo");
        assert!(ext.installed, "installed should default to true");
        assert!(!ext.needs_setup, "needs_setup should default to false");
        assert!(!ext.has_auth);
        assert!(ext.tools.is_empty());
        assert!(ext.display_name.is_none());
        assert!(ext.description.is_none());
        assert!(ext.url.is_none());
        assert!(ext.activation_error.is_none());
    }

    #[test]
    fn installed_extension_serde_all_fields() {
        let ext = InstalledExtension {
            name: "gmail".to_string(),
            kind: ExtensionKind::WasmTool,
            display_name: Some("Gmail Tool".to_string()),
            description: Some("Read and send emails".to_string()),
            url: Some("https://gmail.example.com".to_string()),
            authenticated: true,
            active: true,
            tools: vec!["send_email".to_string(), "read_inbox".to_string()],
            needs_setup: true,
            has_auth: true,
            installed: false,
            activation_error: Some("token expired".to_string()),
            version: None,
        };
        let json = serde_json::to_value(&ext).unwrap();
        assert_eq!(json["display_name"], "Gmail Tool");
        assert_eq!(json["description"], "Read and send emails");
        assert_eq!(json["url"], "https://gmail.example.com");
        assert_eq!(json["needs_setup"], true);
        assert_eq!(json["installed"], false);
        assert_eq!(json["activation_error"], "token expired");

        let back: InstalledExtension = serde_json::from_value(json).unwrap();
        assert_eq!(back.name, "gmail");
        assert_eq!(back.tools.len(), 2);
        assert!(back.needs_setup);
        assert!(!back.installed);
        assert_eq!(back.activation_error.as_deref(), Some("token expired"));
    }

    // ── ExtensionError Display ───────────────────────────────────────

    #[test]
    fn extension_error_display_all_variants() {
        let cases: Vec<(ExtensionError, &str)> = vec![
            (
                ExtensionError::NotFound("foo".into()),
                "Extension not found: foo",
            ),
            (
                ExtensionError::AlreadyInstalled("bar".into()),
                "Extension already installed: bar",
            ),
            (
                ExtensionError::NotInstalled("baz".into()),
                "Extension not installed: baz",
            ),
            (
                ExtensionError::AuthFailed("bad token".into()),
                "Authentication failed: bad token",
            ),
            (
                ExtensionError::ActivationFailed("crash".into()),
                "Activation failed: crash",
            ),
            (
                ExtensionError::InstallFailed("disk full".into()),
                "Installation failed: disk full",
            ),
            (
                ExtensionError::DiscoveryFailed("timeout".into()),
                "Discovery failed: timeout",
            ),
            (
                ExtensionError::InvalidUrl("not a url".into()),
                "Invalid URL: not a url",
            ),
            (
                ExtensionError::DownloadFailed("404".into()),
                "Download failed: 404",
            ),
            (
                ExtensionError::Config("missing key".into()),
                "Config error: missing key",
            ),
            (ExtensionError::AuthRequired, "Authentication required"),
            (
                ExtensionError::Other("something broke".into()),
                "something broke",
            ),
            (
                ExtensionError::FallbackFailed {
                    primary: Box::new(ExtensionError::DownloadFailed("404".into())),
                    fallback: Box::new(ExtensionError::InstallFailed("no cargo".into())),
                },
                "Primary install failed: Download failed: 404; fallback install also failed: Installation failed: no cargo",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    // ── ToolAuthState ────────────────────────────────────────────────

    #[test]
    fn tool_auth_state_equality() {
        assert_eq!(ToolAuthState::Ready, ToolAuthState::Ready);
        assert_eq!(ToolAuthState::NeedsAuth, ToolAuthState::NeedsAuth);
        assert_eq!(ToolAuthState::NeedsSetup, ToolAuthState::NeedsSetup);
        assert_eq!(ToolAuthState::NoAuth, ToolAuthState::NoAuth);

        assert_ne!(ToolAuthState::Ready, ToolAuthState::NeedsAuth);
        assert_ne!(ToolAuthState::NeedsSetup, ToolAuthState::NoAuth);
        assert_ne!(ToolAuthState::Ready, ToolAuthState::NoAuth);
    }

    // ── ResultSource ─────────────────────────────────────────────────

    #[test]
    fn result_source_serde() {
        let json = serde_json::to_value(ResultSource::Registry).unwrap();
        assert_eq!(json, "registry");
        let back: ResultSource = serde_json::from_value(json).unwrap();
        assert_eq!(back, ResultSource::Registry);

        let json = serde_json::to_value(ResultSource::Discovered).unwrap();
        assert_eq!(json, "discovered");
        let back: ResultSource = serde_json::from_value(json).unwrap();
        assert_eq!(back, ResultSource::Discovered);
    }

    // ── AuthResult::status_str ───────────────────────────────────────

    #[test]
    fn auth_result_status_str_all_variants() {
        assert_eq!(
            AuthResult::authenticated("a", ExtensionKind::McpServer).status_str(),
            "authenticated"
        );
        assert_eq!(
            AuthResult::no_auth_required("b", ExtensionKind::WasmTool).status_str(),
            "no_auth_required"
        );
        assert_eq!(
            AuthResult::awaiting_authorization(
                "c",
                ExtensionKind::WasmChannel,
                "https://x.com".into(),
                "local".into(),
            )
            .status_str(),
            "awaiting_authorization"
        );
        assert_eq!(
            AuthResult::awaiting_token("d", ExtensionKind::WasmTool, "paste token".into(), None)
                .status_str(),
            "awaiting_token"
        );
        assert_eq!(
            AuthResult::needs_setup(
                "e",
                ExtensionKind::McpServer,
                "configure oauth".into(),
                Some("https://setup.example.com".into()),
            )
            .status_str(),
            "needs_setup"
        );
    }
}
