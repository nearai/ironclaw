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
    /// Apple Sign In configuration.
    pub apple: Option<AppleOAuthConfig>,
    /// NEAR wallet authentication (signature-based, not OAuth).
    pub near: Option<NearAuthConfig>,
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

/// Apple Sign In configuration.
///
/// Apple uses OIDC but requires a JWT `client_secret` signed with an ES256
/// private key from the Apple Developer portal.
#[derive(Debug, Clone)]
pub struct AppleOAuthConfig {
    /// Services ID (e.g. `com.example.myapp`).
    pub client_id: String,
    /// Apple Developer Team ID (10-character string).
    pub team_id: String,
    /// Key ID from the Apple Developer portal.
    pub key_id: String,
    /// ES256 private key in PEM format (contents of the `.p8` file).
    pub private_key_pem: SecretString,
}

/// NEAR wallet authentication configuration.
///
/// Uses NEP-413 signature verification instead of OAuth. The server generates
/// a challenge nonce, the client signs it with a NEAR wallet, and the server
/// verifies the Ed25519 signature and confirms the public key belongs to the
/// claimed account via NEAR RPC.
#[derive(Debug, Clone)]
pub struct NearAuthConfig {
    /// NEAR network: `mainnet` or `testnet`.
    pub network: String,
    /// NEAR RPC endpoint URL.
    pub rpc_url: String,
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

        let apple = match (
            optional_env("APPLE_CLIENT_ID")?,
            optional_env("APPLE_TEAM_ID")?,
            optional_env("APPLE_KEY_ID")?,
        ) {
            (Some(client_id), Some(team_id), Some(key_id)) => {
                // Read the private key from a file path or directly from env.
                let pem = if let Some(path) = optional_env("APPLE_PRIVATE_KEY_PATH")? {
                    std::fs::read_to_string(&path).map_err(|e| ConfigError::InvalidValue {
                        key: "APPLE_PRIVATE_KEY_PATH".to_string(),
                        message: format!("failed to read Apple private key from '{path}': {e}"),
                    })?
                } else if let Some(pem) = optional_env("APPLE_PRIVATE_KEY_PEM")? {
                    pem
                } else {
                    return Err(ConfigError::InvalidValue {
                        key: "APPLE_PRIVATE_KEY_PATH".to_string(),
                        message: "either APPLE_PRIVATE_KEY_PATH or APPLE_PRIVATE_KEY_PEM is \
                                  required when APPLE_CLIENT_ID is set"
                            .to_string(),
                    });
                };
                Some(AppleOAuthConfig {
                    client_id,
                    team_id,
                    key_id,
                    private_key_pem: SecretString::from(pem),
                })
            }
            _ => None,
        };

        let near = if parse_bool_env("NEAR_AUTH_ENABLED", false)? {
            let network =
                optional_env("NEAR_AUTH_NETWORK")?.unwrap_or_else(|| "mainnet".to_string());
            let rpc_url =
                optional_env("NEAR_AUTH_RPC_URL")?.unwrap_or_else(|| match network.as_str() {
                    "testnet" => "https://rpc.testnet.near.org".to_string(),
                    _ => "https://rpc.mainnet.near.org".to_string(),
                });
            Some(NearAuthConfig { network, rpc_url })
        } else {
            None
        };

        Ok(Self {
            enabled,
            base_url,
            allowed_domains,
            google,
            github,
            apple,
            near,
        })
    }
}
