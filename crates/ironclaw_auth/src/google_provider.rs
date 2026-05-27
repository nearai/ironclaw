use std::fmt;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, NetworkPolicy, ResourceScope, SecretHandle};
use secrecy::SecretString;

use crate::{AuthFlowId, AuthProductError};

/// Boundary for turning provider token material into durable secret handles.
///
/// `ironclaw_auth` intentionally does not own durable secret storage; adapter
/// crates inject the storage boundary via this trait.
#[async_trait]
pub trait GoogleProviderTokenSink: Send + Sync {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError>;
}

/// Boundary for staging/authorizing the Google token-exchange network policy.
///
/// Production Reborn egress uses staged policy handoffs instead of trusting
/// request-carried fallback policy data.
#[async_trait]
pub trait GoogleProviderEgressPolicyAuthorizer: Send + Sync {
    async fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), AuthProductError>;
}

/// Raw Google token material passed exactly once to the injected storage
/// boundary. This type intentionally does not implement serde.
pub struct GoogleProviderTokenSet {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
}

impl fmt::Debug for GoogleProviderTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Scoped token-storage request. Raw provider token material must be bound to
/// the already-claimed callback scope and flow before it reaches storage.
pub struct GoogleProviderTokenStorageRequest {
    pub scope: ResourceScope,
    pub flow_id: AuthFlowId,
    pub tokens: GoogleProviderTokenSet,
}

impl fmt::Debug for GoogleProviderTokenStorageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderTokenStorageRequest")
            .field("scope", &self.scope)
            .field("flow_id", &self.flow_id)
            .field("tokens", &self.tokens)
            .finish()
    }
}

/// Durable secret handles produced after Google OAuth token material is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleProviderStoredTokens {
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
}
