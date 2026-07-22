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

mod account_state;
mod cleanup;
mod credential;
pub mod domain;
mod engine;
mod error;
mod fakes;
mod flow;
mod ids;
mod interaction;
pub mod loopback_oauth;
pub mod oauth;
mod provider;
mod scope;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use account_state::{AuthAccountLastError, AuthAccountState, project_auth_account_state};
pub use cleanup::{
    CanceledCleanupFlow, SecretCleanupAction, SecretCleanupQuarantine,
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
    CredentialSetupService, NewCredentialAccount, ProviderBackedCredentialAccountService,
    binding_scope_owns_account,
};
pub use domain::select_latest_duplicate_user_reusable_account;
pub use engine::keepalive;
pub use engine::keepalive::{
    AlwaysLeaderKeepaliveLock, KEEPALIVE_SWEEP_SHUTDOWN_TIMEOUT, KeepaliveCandidateSource,
    KeepaliveLeaderLock, KeepaliveRefreshPort, KeepaliveSweepDeps, KeepaliveSweepFuture,
    KeepaliveSweepHandle, KeepaliveSweepSettings, LeaderOutcome, spawn_keepalive_sweep,
};
pub use engine::{
    AuthEngine, AuthEngineDeps, AuthRecipeResolver, DCR_CLIENT_HANDLE_PREFIX, EngineCallbackBase,
    EngineOAuthClientMaterial, EngineOAuthConfigurationSource, PrepareOAuthFlowRequest,
    PreparedOAuthFlow, ResolvedVendorAuthRecipe, StaticAuthRecipeResolver,
};
pub use error::{AuthErrorCode, AuthProductError};
pub use fakes::InMemoryAuthProductServices;
pub use flow::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthFlowOutcome,
    AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource, AuthFlowState, AuthFlowStatus,
    AuthResolved, CredentialAccountUpdateBinding, CredentialSelectionInput,
    ManualTokenCompletionInput, NewAuthFlow, OAuthCallbackClaim, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, ProviderCallbackOutcome, TurnGateAuthFlowQuery,
    credential_status_for_completed_flow, flow_matches_durable_owner, flow_matches_turn_gate_query,
    flow_shares_setup_owner_root, is_setup_class_continuation,
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
    GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE,
    GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE, GOOGLE_PROVIDER_ID, OAuthCallbackState,
    OAuthCallbackStateKind, OAuthClientId, OAuthProviderIdentity, OAuthProviderIdentitySubject,
    OAuthRedirectUri, OAuthState, OAuthTokenResponse, PkceCodeChallenge, authorization_code_hash,
    opaque_state_hash, pkce_s256_challenge, pkce_verifier_hash, scope_text,
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

pub fn is_terminal_state(state: AuthFlowState) -> bool {
    matches!(state, AuthFlowState::Resolved(_))
}
