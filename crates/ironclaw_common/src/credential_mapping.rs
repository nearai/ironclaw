//! Credential injection mapping types.
//!
//! Describes where in an HTTP request a secret should be injected. Shared
//! between the WASM sandbox, the HTTP builtin tool, and capability schema
//! parsing, so it lives here (free of `wasm-sandbox`/wasmtime dependencies).

use serde::{Deserialize, Serialize};

/// Where a credential should be injected in an HTTP request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CredentialLocation {
    /// Inject as Authorization header (e.g., "Bearer {secret}")
    #[default]
    AuthorizationBearer,
    /// Inject as Authorization header with Basic auth
    AuthorizationBasic { username: String },
    /// Inject as a custom header
    Header {
        name: String,
        prefix: Option<String>,
    },
    /// Inject as a query parameter
    QueryParam { name: String },
    /// Inject by replacing a placeholder in URL or body templates
    UrlPath { placeholder: String },
}

/// Mapping from a secret name to where it should be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMapping {
    /// Name of the secret to use.
    pub secret_name: String,
    /// Where to inject the credential.
    pub location: CredentialLocation,
    /// Host patterns this credential applies to (glob syntax).
    pub host_patterns: Vec<String>,
    /// When `true`, the tool may run without this credential — the host
    /// is allowed to skip the mapping if the secret cannot be resolved.
    /// **Defaults to `false` (required)** so a tool that simply declares
    /// a credential without explicitly opting into "optional" cannot be
    /// silently downgraded to an unauthenticated request.
    #[serde(default)]
    pub optional: bool,
}

impl CredentialMapping {
    pub fn bearer(secret_name: impl Into<String>, host_pattern: impl Into<String>) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec![host_pattern.into()],
            optional: false,
        }
    }

    pub fn header(
        secret_name: impl Into<String>,
        header_name: impl Into<String>,
        host_pattern: impl Into<String>,
    ) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::Header {
                name: header_name.into(),
                prefix: None,
            },
            host_patterns: vec![host_pattern.into()],
            optional: false,
        }
    }
}
