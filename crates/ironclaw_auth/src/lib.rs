//! Product-facing authentication contracts for IronClaw Reborn.
//!
//! This crate is the contract-first slice for #3289 / #3810 / #3883. It
//! defines the typed auth-flow, secure interaction, credential-account,
//! recovery/account-selection, provider exchange, continuation, and cleanup
//! boundaries used by Reborn product surfaces.
//!
//! Behavior may remain compatible with legacy product UX, but code paths must
//! stay Reborn-native: this crate does not depend on V1 route handlers, V1
//! pending maps, V1 extension manager authority, or V1 secret stores.

mod cleanup;
mod credential;
pub mod domain;
mod error;
mod fakes;
mod flow;
mod ids;
mod interaction;
mod loopback_oauth;
pub mod oauth;
mod provider;
mod scope;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use cleanup::{
    OAuthCompletionCompensationOutcome, OAuthCompletionCompensationRequest,
    OAuthExchangeCleanupRequest, SecretCleanupAction, SecretCleanupQuarantine,
    SecretCleanupQuarantineReason, SecretCleanupReport, SecretCleanupRequest, SecretCleanupService,
};
pub use credential::{
    CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountListPage,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountMutation,
    CredentialAccountOwnerScope, CredentialAccountProjection, CredentialAccountRecordSource,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialAccountUpdate, CredentialOwnership, CredentialRecoveryKind,
    CredentialRecoveryProjection, CredentialRecoveryReason, CredentialRecoveryRequest,
    CredentialRecoveryState, CredentialRefreshReport, CredentialRefreshRequest,
    CredentialSecretFingerprint, CredentialSetupService, NewCredentialAccount,
    ProviderBackedCredentialAccountService, binding_scope_owns_account,
};
pub use domain::select_latest_duplicate_user_reusable_account;
pub use error::{AuthErrorCode, AuthProductError};
pub use fakes::InMemoryAuthProductServices;
pub use flow::{
    AUTH_CONTINUATION_DISPATCH_LEASE_SECONDS, AuthChallenge, AuthContinuationDispatchClaimInput,
    AuthContinuationDispatchOutcome, AuthContinuationDispatchSettlementInput,
    AuthContinuationEvent, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthFlowOwnerScope,
    AuthFlowRecord, AuthFlowRecordSource, AuthFlowStatus, CredentialAccountUpdateBinding,
    CredentialSelectionInput, ManualTokenCompletionInput, NewAuthFlow, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, ProviderCallbackOutcome, TurnGateAuthFlowQuery,
    credential_status_for_completed_flow, flow_matches_durable_owner, flow_matches_turn_gate_query,
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
pub use oauth::{
    GOOGLE_AUTHORIZATION_ENDPOINT, GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE,
    GOOGLE_GMAIL_MODIFY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    GOOGLE_PROVIDER_ID, GOOGLE_TOKEN_ENDPOINT, GoogleOAuthCallbackState, GoogleOAuthRouteConfig,
    OAuthAuthorizationEndpoint, OAuthAuthorizeUrlRequest, OAuthCallbackState,
    OAuthCallbackStateKind, OAuthClientId, OAuthExtraParam, OAuthProviderIdentity,
    OAuthProviderIdentitySubject, OAuthRedirectUri, OAuthScopeParam, OAuthState,
    OAuthTokenResponse, PkceCodeChallenge, SLACK_PERSONAL_AUTHORIZATION_ENDPOINT,
    SLACK_PERSONAL_PROVIDER_ID, SLACK_PERSONAL_TOKEN_ENDPOINT, authorization_code_hash,
    build_authorization_url, build_authorization_url_with_scope_param,
    build_google_authorization_url, is_allowed_google_scope, opaque_state_hash,
    parse_google_callback_scopes, parse_google_requested_scopes, pkce_s256_challenge,
    pkce_verifier_hash, scope_text,
};
pub use provider::{
    AuthProviderClient, OAuthAuthorizationCode, OAuthProviderCallbackRequest,
    OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderRefresh,
    OAuthProviderRefreshRequest, PkceVerifierSecret, validate_provider_callback_request,
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

pub fn scope_matches(left: &AuthProductScope, right: &AuthProductScope) -> bool {
    left == right
}

pub fn is_terminal_status(status: AuthFlowStatus) -> bool {
    matches!(
        status,
        AuthFlowStatus::Completed
            | AuthFlowStatus::Failed
            | AuthFlowStatus::Expired
            | AuthFlowStatus::Canceled
    )
}
