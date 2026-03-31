//! OAuth provider configuration for direct social login.

use secrecy::SecretString;

use crate::config::helpers::{optional_env, parse_bool_env};
use crate::error::ConfigError;

/// OAuth/social login configuration.
///
/// Disabled by default. When enabled, the gateway exposes `/auth/*` routes
/// for OAuth login flows. Each provider is independently configured via
/// env vars — only providers with both `CLIENT_ID` and `CLIENT_SECRET` are
/// activated.
#[derive(Debug, Clone, Default)]
pub struct OAuthConfig {
    /// Whether OAuth social login is enabled.
    pub enabled: bool,
    /// Base URL for constructing OAuth callback URLs
    /// (e.g. `https://myapp.example.com`).
    /// Falls back to `http://localhost:{gateway_port}` if unset.
    pub base_url: Option<String>,
    /// Google OAuth configuration (OIDC).
    pub google: Option<GoogleOAuthConfig>,
    /// GitHub OAuth configuration.
    pub github: Option<GitHubOAuthConfig>,
}

/// Google OAuth 2.0 / OIDC configuration.
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: SecretString,
}

/// GitHub OAuth 2.0 configuration.
#[derive(Debug, Clone)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: SecretString,
}

impl OAuthConfig {
    pub fn resolve() -> Result<Self, ConfigError> {
        let enabled = parse_bool_env("OAUTH_ENABLED", false)?;
        if !enabled {
            return Ok(Self {
                enabled: false,
                base_url: None,
                google: None,
                github: None,
            });
        }

        let base_url = optional_env("OAUTH_BASE_URL")?;

        let google = match (
            optional_env("GOOGLE_CLIENT_ID")?,
            optional_env("GOOGLE_CLIENT_SECRET")?,
        ) {
            (Some(id), Some(secret)) => Some(GoogleOAuthConfig {
                client_id: id,
                client_secret: SecretString::from(secret),
            }),
            _ => None,
        };

        let github = match (
            optional_env("GITHUB_CLIENT_ID")?,
            optional_env("GITHUB_CLIENT_SECRET")?,
        ) {
            (Some(id), Some(secret)) => Some(GitHubOAuthConfig {
                client_id: id,
                client_secret: SecretString::from(secret),
            }),
            _ => None,
        };

        Ok(Self {
            enabled,
            base_url,
            google,
            github,
        })
    }
}

