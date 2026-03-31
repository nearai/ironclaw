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
    /// Restrict OAuth login to users with verified emails from these domains.
    /// Empty means allow all domains. Applied to all OAuth providers and OIDC.
    /// Parsed from `OAUTH_ALLOWED_DOMAINS` (comma-separated, e.g. `company.com,partner.org`).
    pub allowed_domains: Vec<String>,
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
    /// Restrict to a specific Google Workspace (G Suite) hosted domain.
    /// Sets the `hd` parameter in the authorization URL so Google only
    /// shows accounts from this domain. Also validated server-side after
    /// code exchange. Parsed from `GOOGLE_ALLOWED_HD`.
    pub allowed_hd: Option<String>,
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
            return Ok(Self::default());
        }

        let base_url = optional_env("OAUTH_BASE_URL")?;

        let allowed_domains: Vec<String> = optional_env("OAUTH_ALLOWED_DOMAINS")?
            .map(|s| {
                s.split(',')
                    .map(|d| d.trim().to_ascii_lowercase())
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let google = match (
            optional_env("GOOGLE_CLIENT_ID")?,
            optional_env("GOOGLE_CLIENT_SECRET")?,
        ) {
            (Some(id), Some(secret)) => Some(GoogleOAuthConfig {
                client_id: id,
                client_secret: SecretString::from(secret),
                allowed_hd: optional_env("GOOGLE_ALLOWED_HD")?,
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
            allowed_domains,
            google,
            github,
        })
    }
}
