//! Product-facing authentication contracts for IronClaw Reborn.
//!
//! This crate is the contract-first slice for #3289 / #3810. It defines the
//! typed auth-flow, secure interaction, credential-account, provider exchange,
//! continuation, and cleanup boundaries used by Reborn product surfaces.
//!
//! Behavior may remain compatible with legacy product UX, but code paths must
//! stay Reborn-native: this crate does not depend on V1 route handlers, V1
//! pending maps, V1 extension manager authority, or V1 secret stores.

mod cleanup;
mod credential;
mod error;
mod fakes;
mod flow;
mod ids;
mod interaction;
mod provider;
mod scope;

pub use cleanup::{
    SecretCleanupAction, SecretCleanupReport, SecretCleanupRequest, SecretCleanupService,
};
pub use credential::{
    CredentialAccount, CredentialAccountListPage, CredentialAccountListRequest,
    CredentialAccountMutation, CredentialAccountProjection, CredentialAccountSelectionRequest,
    CredentialAccountService, CredentialAccountStatus, CredentialAccountUpdate,
    CredentialOwnership, CredentialSetupService, NewCredentialAccount,
};
pub use error::{AuthErrorCode, AuthProductError};
pub use fakes::InMemoryAuthProductServices;
pub use flow::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthFlowKind, AuthFlowManager,
    AuthFlowRecord, AuthFlowStatus, CredentialAccountUpdateBinding, NewAuthFlow,
    OAuthCallbackClaimRequest, OAuthCallbackInput, ProviderCallbackOutcome,
};
pub use ids::{
    AuthFlowId, AuthGateRef, AuthInteractionId, AuthProviderId, AuthSessionId,
    AuthorizationCodeHash, CredentialAccountId, CredentialAccountLabel, LifecyclePackageRef,
    OAuthAuthorizationUrl, OpaqueStateHash, PkceVerifierHash, ProductActionRef, ProviderScope,
    TurnRunRef,
};
pub use interaction::{
    AuthInteractionService, ManualTokenSetupRequest, SecretSubmitRequest, SecretSubmitResult,
};
pub use provider::{
    AuthProviderClient, OAuthAuthorizationCode, OAuthProviderCallbackRequest,
    OAuthProviderExchange, PkceVerifierSecret,
};
pub use scope::{AuthProductScope, AuthSurface};

/// Canonical timestamp type for auth product contracts.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

fn validate_public_text(
    value: impl Into<String>,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, AuthProductError> {
    let value = value.into();
    if value.is_empty() {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must not be empty"
        )));
    }
    if value.trim() != value {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must not contain leading or trailing whitespace"
        )));
    }
    if value.len() > max_bytes {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must be at most {max_bytes} bytes"
        )));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    Ok(value)
}

fn scope_matches(left: &AuthProductScope, right: &AuthProductScope) -> bool {
    left == right
}

fn is_terminal_status(status: AuthFlowStatus) -> bool {
    matches!(
        status,
        AuthFlowStatus::Completed
            | AuthFlowStatus::Failed
            | AuthFlowStatus::Expired
            | AuthFlowStatus::Canceled
    )
}
