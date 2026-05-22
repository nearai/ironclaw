use ironclaw_host_api::HostApiError;
use ironclaw_network::NetworkHttpError;
use ironclaw_run_state::RunStateError;
use ironclaw_secrets::SecretStoreError;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("unknown OAuth provider {provider}")]
    UnknownProvider { provider: String },
    #[error("duplicate OAuth provider {provider}")]
    DuplicateProvider { provider: String },
    #[error("OAuth provider mismatch: expected {expected}, got {actual}")]
    ProviderMismatch { expected: String, actual: String },
    #[error("OAuth state is unknown or expired")]
    InvalidState,
    #[error("OAuth configuration is invalid: {reason}")]
    InvalidConfig { reason: String },
    #[error("OAuth configuration is incomplete for {provider}: {reason}")]
    IncompleteConfig { provider: String, reason: String },
    #[error("missing refresh token for {credential_name}")]
    MissingRefreshToken { credential_name: String },
    #[error("OAuth URL rejected: {url}: {reason}")]
    UrlRejected { url: String, reason: String },
    #[error("OAuth network request failed: {0}")]
    Network(#[from] NetworkHttpError),
    #[error("OAuth HTTP response failed: status {status}: {reason}")]
    HttpStatus { status: u16, reason: String },
    #[error("OAuth token response is invalid: {reason}")]
    InvalidTokenResponse { reason: String },
    #[error("OAuth secret store error: {0}")]
    SecretStore(#[from] SecretStoreError),
    #[error("OAuth run-state error: {0}")]
    RunState(#[from] RunStateError),
    #[error("OAuth host API error: {0}")]
    HostApi(#[from] HostApiError),
    #[error("OAuth serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl OAuthError {
    pub(crate) fn invalid_token(reason: impl Into<String>) -> Self {
        Self::InvalidTokenResponse {
            reason: reason.into(),
        }
    }

    pub(crate) fn rejected_url(url: &Url, reason: impl Into<String>) -> Self {
        Self::UrlRejected {
            url: url.to_string(),
            reason: reason.into(),
        }
    }
}
