//! Serde structs for extension registry manifests.
//!
//! Each manifest describes a single extension (tool or channel) with its source
//! location, build artifacts, authentication requirements, and tags.

use serde::{Deserialize, Serialize};

use crate::extensions::{AuthHint, ExtensionKind, ExtensionSource, RegistryEntry};

/// A single extension manifest loaded from `registry/{tools,channels,mcp-servers}/<name>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    /// Unique identifier (matches crate name stem, e.g. "slack").
    pub name: String,

    /// Human-readable name (e.g. "Slack").
    pub display_name: String,

    /// Whether this is a tool, channel, or MCP server.
    pub kind: ManifestKind,

    /// Semver version from Cargo.toml. Optional for MCP server manifests.
    #[serde(default)]
    pub version: Option<String>,

    /// One-line description.
    pub description: String,

    /// Search keywords beyond the name.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Source code location and build info. Absent for MCP server manifests.
    #[serde(default)]
    pub source: Option<SourceSpec>,

    /// Pre-built binary artifacts keyed by target triple.
    #[serde(default)]
    pub artifacts: std::collections::HashMap<String, ArtifactSpec>,

    /// Summary of authentication requirements.
    #[serde(default)]
    pub auth_summary: Option<AuthSummary>,

    /// Tags for filtering (e.g. "default", "messaging", "google").
    #[serde(default)]
    pub tags: Vec<String>,

    /// MCP server URL. Only present for `McpServer` manifests.
    #[serde(default)]
    pub url: Option<String>,

    /// MCP auth method: "dcr", "oauth_pre_configured:<setup_url>", or "none".
    /// Only present for `McpServer` manifests.
    #[serde(default)]
    pub auth: Option<String>,
}

/// Extension kind as declared in manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    Tool,
    Channel,
    McpServer,
}

impl From<ManifestKind> for ExtensionKind {
    fn from(kind: ManifestKind) -> Self {
        match kind {
            ManifestKind::Tool => ExtensionKind::WasmTool,
            ManifestKind::Channel => ExtensionKind::WasmChannel,
            ManifestKind::McpServer => ExtensionKind::McpServer,
        }
    }
}

impl std::fmt::Display for ManifestKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestKind::Tool => write!(f, "tool"),
            ManifestKind::Channel => write!(f, "channel"),
            ManifestKind::McpServer => write!(f, "mcp_server"),
        }
    }
}

/// Source code location for building from source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpec {
    /// Path relative to repo root (e.g. "tools-src/slack").
    pub dir: String,

    /// Capabilities filename relative to source dir.
    pub capabilities: String,

    /// Rust crate name for `cargo component build`.
    pub crate_name: String,
}

/// A pre-built binary artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSpec {
    /// Download URL (null until release).
    /// Can point to a `.wasm` file or a `.tar.gz` bundle containing both
    /// `{name}.wasm` and `{name}.capabilities.json`.
    pub url: Option<String>,

    /// Hex SHA256 of the downloaded artifact (null until release).
    pub sha256: Option<String>,

    /// Optional separate download URL for the capabilities file.
    /// Only needed when `url` points to a bare `.wasm` file instead of a bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities_url: Option<String>,
}

/// Summary of authentication requirements extracted from capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSummary {
    /// Auth method: "oauth", "manual", or "none".
    #[serde(default)]
    pub method: Option<String>,

    /// Display name for the auth provider (e.g. "Google", "Slack").
    #[serde(default)]
    pub provider: Option<String>,

    /// Secret names required by this extension.
    #[serde(default)]
    pub secrets: Vec<String>,

    /// If this extension shares auth with others (e.g. all Google tools share
    /// `google_oauth_token`), this is the shared secret name.
    #[serde(default)]
    pub shared_auth: Option<String>,

    /// URL where users can set up credentials.
    #[serde(default)]
    pub setup_url: Option<String>,
}

/// Bundle definition grouping related extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleDefinition {
    /// Human-readable name.
    pub display_name: String,

    /// Description of what this bundle contains.
    #[serde(default)]
    pub description: Option<String>,

    /// Extension references as "tools/<name>" or "channels/<name>".
    pub extensions: Vec<String>,

    /// Shared auth secret across bundle members (if any).
    #[serde(default)]
    pub shared_auth: Option<String>,

    /// Alternate names that should resolve to this bundle during discovery.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Top-level structure of `_bundles.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BundlesFile {
    pub bundles: std::collections::HashMap<String, BundleDefinition>,
}

impl ExtensionManifest {
    /// Convert this manifest into a [`RegistryEntry`] for use with the in-chat
    /// extension discovery system.
    ///
    /// Returns `None` for MCP server manifests missing a `url` field.
    pub fn to_registry_entry(&self) -> Option<RegistryEntry> {
        if self.kind == ManifestKind::McpServer {
            return self.to_mcp_registry_entry();
        }

        Some(self.to_wasm_registry_entry())
    }

    /// Build a [`RegistryEntry`] for an MCP server manifest.
    fn to_mcp_registry_entry(&self) -> Option<RegistryEntry> {
        let url = match &self.url {
            Some(u) => u.clone(),
            None => {
                tracing::warn!(
                    "MCP server manifest '{}' is missing 'url' field, skipping",
                    self.name
                );
                return None;
            }
        };
        let auth_hint = match self.auth.as_deref() {
            Some("dcr") | None => AuthHint::Dcr,
            Some("none") => AuthHint::None,
            Some(other) if other.starts_with("oauth_pre_configured:") => {
                AuthHint::OAuthPreConfigured {
                    setup_url: other
                        .strip_prefix("oauth_pre_configured:")
                        .unwrap_or("")
                        .to_string(),
                }
            }
            _ => AuthHint::Dcr,
        };

        Some(RegistryEntry {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            kind: ExtensionKind::McpServer,
            description: self.description.clone(),
            keywords: self.keywords.clone(),
            source: ExtensionSource::McpUrl { url },
            fallback_source: None,
            auth_hint,
            version: self.version.clone(),
        })
    }

    /// Build a [`RegistryEntry`] for a WASM tool or channel manifest.
    fn to_wasm_registry_entry(&self) -> RegistryEntry {
        let source_spec = self.source.as_ref();

        let buildable = source_spec.map(|s| ExtensionSource::WasmBuildable {
            source_dir: s.dir.clone(),
            build_dir: Some(s.dir.clone()),
            crate_name: Some(s.crate_name.clone()),
        });

        // Prefer pre-built artifact download when a URL is available,
        // with build-from-source as fallback in case the download fails (e.g., 404).
        let (source, fallback_source) = if let Some(artifact) = self.artifacts.get("wasm32-wasip2")
        {
            if let Some(ref url) = artifact.url {
                (
                    ExtensionSource::WasmDownload {
                        wasm_url: url.clone(),
                        capabilities_url: artifact.capabilities_url.clone(),
                    },
                    buildable.map(Box::new),
                )
            } else if let Some(b) = buildable {
                (b, None)
            } else {
                // No source spec and no download URL — use a placeholder
                (
                    ExtensionSource::WasmBuildable {
                        source_dir: String::new(),
                        build_dir: None,
                        crate_name: None,
                    },
                    None,
                )
            }
        } else if let Some(b) = buildable {
            (b, None)
        } else {
            (
                ExtensionSource::WasmBuildable {
                    source_dir: String::new(),
                    build_dir: None,
                    crate_name: None,
                },
                None,
            )
        };

        let auth_hint = match self.auth_summary.as_ref().and_then(|a| a.method.as_deref()) {
            Some("oauth") => AuthHint::CapabilitiesAuth,
            Some("manual") => AuthHint::CapabilitiesAuth,
            Some("none") | None => AuthHint::None,
            Some(_) => AuthHint::CapabilitiesAuth,
        };

        RegistryEntry {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            kind: self.kind.into(),
            description: self.description.clone(),
            keywords: self.keywords.clone(),
            source,
            fallback_source,
            auth_hint,
            version: self.version.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_manifest() {
        let json = r#"{
            "name": "slack",
            "display_name": "Slack",
            "kind": "tool",
            "version": "0.1.0",
            "description": "Post messages via Slack API",
            "keywords": ["messaging"],
            "source": {
                "dir": "tools-src/slack",
                "capabilities": "slack-tool.capabilities.json",
                "crate_name": "slack-tool"
            },
            "artifacts": {
                "wasm32-wasip2": { "url": null, "sha256": null }
            },
            "auth_summary": {
                "method": "oauth",
                "provider": "Slack",
                "secrets": ["slack_bot_token"],
                "shared_auth": null,
                "setup_url": "https://api.slack.com/apps"
            },
            "tags": ["default", "messaging"]
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        assert_eq!(manifest.name, "slack");
        assert_eq!(manifest.kind, ManifestKind::Tool);
        assert_eq!(manifest.version.as_deref(), Some("0.1.0"));
        assert!(manifest.tags.contains(&"default".to_string()));

        let entry = manifest.to_registry_entry().unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmTool);
    }

    #[test]
    fn test_parse_channel_manifest() {
        let json = r#"{
            "name": "telegram",
            "display_name": "Telegram",
            "kind": "channel",
            "version": "0.1.0",
            "description": "Telegram Bot API channel",
            "source": {
                "dir": "channels-src/telegram",
                "capabilities": "telegram.capabilities.json",
                "crate_name": "telegram-channel"
            },
            "tags": ["messaging"]
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        assert_eq!(manifest.kind, ManifestKind::Channel);
        assert!(manifest.auth_summary.is_none());
        assert!(manifest.artifacts.is_empty());

        let entry = manifest.to_registry_entry().unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmChannel);
    }

    #[test]
    fn test_parse_bundles() {
        let json = r#"{
            "bundles": {
                "google": {
                    "display_name": "Google Suite",
                    "description": "All Google tools",
                    "extensions": ["tools/gmail", "tools/google-calendar"],
                    "shared_auth": "google_oauth_token",
                    "aliases": ["gws", "gsuite"]
                },
                "default": {
                    "display_name": "Recommended Set",
                    "extensions": ["tools/github", "tools/slack"]
                }
            }
        }"#;

        let bundles: BundlesFile = serde_json::from_str(json).expect("parse bundles");
        assert_eq!(bundles.bundles.len(), 2);
        assert_eq!(
            bundles.bundles["google"].shared_auth.as_deref(),
            Some("google_oauth_token")
        );
        assert_eq!(bundles.bundles["google"].aliases, vec!["gws", "gsuite"]);
        assert!(bundles.bundles["default"].shared_auth.is_none());
    }

    #[test]
    fn test_manifest_kind_display() {
        assert_eq!(ManifestKind::Tool.to_string(), "tool");
        assert_eq!(ManifestKind::Channel.to_string(), "channel");
        assert_eq!(ManifestKind::McpServer.to_string(), "mcp_server");
    }

    /// When a manifest has a download URL in artifacts, to_registry_entry()
    /// should set WasmDownload as primary source and WasmBuildable as fallback.
    #[test]
    fn test_manifest_with_download_url_has_buildable_fallback() {
        let json = r#"{
            "name": "gmail",
            "display_name": "Gmail",
            "kind": "tool",
            "version": "0.1.0",
            "description": "Gmail tool",
            "keywords": ["email"],
            "source": {
                "dir": "tools-src/gmail",
                "capabilities": "gmail-tool.capabilities.json",
                "crate_name": "gmail-tool"
            },
            "artifacts": {
                "wasm32-wasip2": {
                    "url": "https://github.com/nearai/ironclaw/releases/latest/download/gmail-wasm32-wasip2.tar.gz",
                    "sha256": null
                }
            },
            "tags": ["default"]
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        let entry = manifest.to_registry_entry().unwrap();

        // Primary source should be WasmDownload
        assert!(
            matches!(&entry.source, ExtensionSource::WasmDownload { .. }),
            "Primary source should be WasmDownload, got {:?}",
            entry.source
        );

        // Fallback should be WasmBuildable with the source dir info
        let fallback = entry
            .fallback_source
            .as_ref()
            .expect("Should have fallback_source when download URL is set");
        match fallback.as_ref() {
            ExtensionSource::WasmBuildable {
                build_dir,
                crate_name,
                ..
            } => {
                assert_eq!(build_dir.as_deref(), Some("tools-src/gmail"));
                assert_eq!(crate_name.as_deref(), Some("gmail-tool"));
            }
            other => panic!("Fallback should be WasmBuildable, got {:?}", other),
        }
    }

    /// When a manifest has null URL in artifacts, the primary source should be
    /// WasmBuildable with no fallback.
    #[test]
    fn test_manifest_with_null_url_no_fallback() {
        let json = r#"{
            "name": "slack",
            "display_name": "Slack",
            "kind": "tool",
            "version": "0.1.0",
            "description": "Slack tool",
            "keywords": [],
            "source": {
                "dir": "tools-src/slack",
                "capabilities": "slack-tool.capabilities.json",
                "crate_name": "slack-tool"
            },
            "artifacts": {
                "wasm32-wasip2": { "url": null, "sha256": null }
            },
            "tags": []
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        let entry = manifest.to_registry_entry().unwrap();

        assert!(
            matches!(&entry.source, ExtensionSource::WasmBuildable { .. }),
            "Should use WasmBuildable when URL is null"
        );
        assert!(
            entry.fallback_source.is_none(),
            "Should have no fallback when already using WasmBuildable"
        );
    }

    /// When a manifest has no artifacts section, should use WasmBuildable with no fallback.
    #[test]
    fn test_manifest_no_artifacts_no_fallback() {
        let json = r#"{
            "name": "custom",
            "display_name": "Custom",
            "kind": "tool",
            "version": "0.1.0",
            "description": "Custom tool",
            "keywords": [],
            "source": {
                "dir": "tools-src/custom",
                "capabilities": "custom.capabilities.json",
                "crate_name": "custom-tool"
            },
            "tags": []
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        let entry = manifest.to_registry_entry().unwrap();

        assert!(
            matches!(&entry.source, ExtensionSource::WasmBuildable { .. }),
            "Should use WasmBuildable when no artifacts"
        );
        assert!(
            entry.fallback_source.is_none(),
            "Should have no fallback when already using WasmBuildable"
        );
    }

    #[test]
    fn test_parse_mcp_server_manifest() {
        let json = r#"{
            "name": "notion",
            "display_name": "Notion",
            "kind": "mcp_server",
            "description": "Connect to Notion for reading and writing pages, databases, and comments",
            "keywords": ["notes", "wiki", "docs", "pages", "database"],
            "url": "https://mcp.notion.com/mcp",
            "auth": "dcr"
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        assert_eq!(manifest.name, "notion");
        assert_eq!(manifest.kind, ManifestKind::McpServer);
        assert!(manifest.version.is_none());
        assert!(manifest.source.is_none());
        assert_eq!(manifest.url.as_deref(), Some("https://mcp.notion.com/mcp"));
        assert_eq!(manifest.auth.as_deref(), Some("dcr"));

        let entry = manifest.to_registry_entry().unwrap();
        assert_eq!(entry.kind, ExtensionKind::McpServer);
        assert!(
            matches!(&entry.source, ExtensionSource::McpUrl { url } if url == "https://mcp.notion.com/mcp")
        );
        assert!(matches!(&entry.auth_hint, AuthHint::Dcr));
        assert!(entry.fallback_source.is_none());
    }

    #[test]
    fn test_mcp_server_oauth_pre_configured() {
        let json = r#"{
            "name": "custom-mcp",
            "display_name": "Custom MCP",
            "kind": "mcp_server",
            "description": "Custom MCP server",
            "keywords": [],
            "url": "https://mcp.example.com",
            "auth": "oauth_pre_configured:https://example.com/setup"
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        let entry = manifest.to_registry_entry().unwrap();

        assert!(matches!(
            &entry.auth_hint,
            AuthHint::OAuthPreConfigured { setup_url } if setup_url == "https://example.com/setup"
        ));
    }

    #[test]
    fn test_mcp_server_auth_none() {
        let json = r#"{
            "name": "local-mcp",
            "display_name": "Local MCP",
            "kind": "mcp_server",
            "description": "Local MCP server",
            "keywords": [],
            "url": "http://localhost:8080/mcp",
            "auth": "none"
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        let entry = manifest.to_registry_entry().unwrap();

        assert!(matches!(&entry.auth_hint, AuthHint::None));
    }

    #[test]
    fn test_mcp_server_missing_url_returns_none() {
        let json = r#"{
            "name": "broken-mcp",
            "display_name": "Broken MCP",
            "kind": "mcp_server",
            "description": "MCP server with no URL",
            "keywords": []
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).expect("parse manifest");
        assert!(
            manifest.to_registry_entry().is_none(),
            "MCP manifest without url should return None"
        );
    }
}
