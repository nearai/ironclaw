use std::fmt;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use crate::{
    AuthContinuationRef, AuthProductError, CredentialAccountId, CredentialAccountLabel,
    CredentialAccountStatus, Timestamp,
    ids::{AuthInteractionId, AuthProviderId},
    scope::AuthProductScope,
};

/// Request to open a secure manual-token interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualTokenSetupRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub continuation: AuthContinuationRef,
    pub expires_at: Timestamp,
}

/// Secure secret submit request. Debug output never includes the secret value.
pub struct SecretSubmitRequest {
    pub interaction_id: AuthInteractionId,
    pub secret: SecretString,
}

impl SecretSubmitRequest {
    pub(crate) fn validate_secret(&self) -> Result<(), AuthProductError> {
        let exposed = self.secret.expose_secret();
        if exposed.trim().is_empty() {
            return Err(AuthProductError::invalid_request(
                "secret value must not be empty",
            ));
        }
        if exposed.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(AuthProductError::invalid_request(
                "secret value must not contain NUL/control characters",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for SecretSubmitRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretSubmitRequest")
            .field("interaction_id", &self.interaction_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// Manual-token setup result safe for product surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSubmitResult {
    pub account_id: CredentialAccountId,
    pub status: CredentialAccountStatus,
    pub continuation: AuthContinuationRef,
}

#[async_trait]
pub trait AuthInteractionService: Send + Sync {
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<crate::AuthChallenge, AuthProductError>;

    async fn submit_manual_token(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError>;
}

#[derive(Debug, Clone)]
pub(crate) struct PendingSecretInteraction {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub continuation: AuthContinuationRef,
    pub expires_at: Timestamp,
}
