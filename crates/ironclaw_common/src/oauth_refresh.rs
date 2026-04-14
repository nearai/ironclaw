//! OAuth refresh configuration (pure data, shared across crates).

use std::collections::HashMap;

/// Configuration needed to refresh an expired OAuth access token.
///
/// Extracted at tool load time from the capabilities file's `auth.oauth`
/// section. Passed into `resolve_host_credentials()` so it can transparently
/// refresh tokens before WASM execution.
#[derive(Debug, Clone)]
pub struct OAuthRefreshConfig {
    /// OAuth token exchange URL (e.g., "https://oauth2.googleapis.com/token").
    pub token_url: String,
    /// OAuth client_id.
    pub client_id: String,
    /// OAuth client_secret (optional, some providers use PKCE without a secret).
    pub client_secret: Option<String>,
    /// Hosted OAuth proxy base URL (e.g., "http://host.docker.internal:8080").
    pub exchange_proxy_url: Option<String>,
    /// OAuth proxy auth token for authenticating with the hosted OAuth proxy.
    /// Kept as `gateway_token` for public API compatibility.
    pub gateway_token: Option<String>,
    /// Secret name of the access token (e.g., "google_oauth_token").
    /// The refresh token lives at `{secret_name}_refresh_token`.
    pub secret_name: String,
    /// Provider hint stored alongside the refreshed secret.
    pub provider: Option<String>,
    /// Extra form parameters appended during refresh requests.
    pub extra_refresh_params: HashMap<String, String>,
}

impl OAuthRefreshConfig {
    pub fn oauth_proxy_auth_token(&self) -> Option<&str> {
        self.gateway_token.as_deref()
    }
}
