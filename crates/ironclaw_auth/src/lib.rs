//! Product-facing authentication contracts for IronClaw Reborn.
//!
//! This crate is intentionally contract-first. It defines the typed auth-flow,
//! auth-interaction, credential-account, continuation, provider-client, and
//! cleanup boundaries that product surfaces should use before production V1
//! routes are migrated. It does not own durable secret encryption, runtime
//! credential injection, extension lifecycle mutation, or low-level OAuth HTTP
//! transport.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Canonical timestamp type for auth product contracts.
pub type Timestamp = DateTime<Utc>;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

uuid_id!(AuthFlowId);
uuid_id!(CredentialAccountId);
uuid_id!(AuthInteractionId);

/// Validate bounded product metadata that may be rendered to adapters.
fn validate_public_text(
    value: impl Into<String>,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, AuthProductError> {
    let value = value.into();
    if value.is_empty() {
        return Err(AuthProductError::InvalidRequest {
            reason: format!("{label} must not be empty"),
        });
    }
    if value.len() > max_bytes {
        return Err(AuthProductError::InvalidRequest {
            reason: format!("{label} must be at most {max_bytes} bytes"),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(AuthProductError::InvalidRequest {
            reason: format!("{label} must not contain NUL/control characters"),
        });
    }
    Ok(value)
}

/// Provider/integration identifier shown in redacted product state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthProviderId(String);

impl AuthProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(value, "auth provider", 128)?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuthProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// User-facing account label. This is metadata only, never secret material.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialAccountLabel(String);

impl CredentialAccountLabel {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(
            value,
            "credential account label",
            256,
        )?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Non-secret typed reference to a product action that should resume after auth.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProductActionRef(String);

impl ProductActionRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(
            value,
            "product action ref",
            256,
        )?))
    }
}

/// Non-secret typed reference to a lifecycle package/install target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LifecyclePackageRef(String);

impl LifecyclePackageRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(
            value,
            "lifecycle package ref",
            256,
        )?))
    }
}

/// Non-secret typed reference to a blocked turn run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnRunRef(String);

impl TurnRunRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(value, "turn run ref", 256)?))
    }
}

/// Non-secret typed reference to an auth gate inside a turn/run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthGateRef(String);

impl AuthGateRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        Ok(Self(validate_public_text(value, "auth gate ref", 256)?))
    }
}

/// Product surface that initiated or renders an auth flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthSurface {
    Chat,
    Web,
    Cli,
    Tui,
    Api,
    SetupAdmin,
    Callback,
}

/// Scoped product auth owner. Durable implementations should key records by
/// this scope plus the opaque flow/interaction/account id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthProductScope {
    pub resource: ResourceScope,
    pub surface: AuthSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl AuthProductScope {
    pub fn new(resource: ResourceScope, surface: AuthSurface) -> Self {
        Self {
            resource,
            surface,
            session_id: None,
        }
    }

    pub fn with_session_id(
        mut self,
        session_id: impl Into<String>,
    ) -> Result<Self, AuthProductError> {
        self.session_id = Some(validate_public_text(session_id, "session id", 256)?);
        Ok(self)
    }
}

/// Auth flow kind. Identity login is represented for future shared substrate
/// support but credential-account semantics apply only to integration flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFlowKind {
    IntegrationCredential,
    IdentityLogin,
}

/// Durable auth-flow lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFlowStatus {
    Pending,
    AwaitingUser,
    CallbackReceived,
    Completing,
    Completed,
    Failed,
    Expired,
    Canceled,
}

/// Stable recoverable auth challenge rendered by adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthChallenge {
    OAuthUrl {
        flow_id: AuthFlowId,
        auth_url: String,
        expires_at: Timestamp,
    },
    ManualTokenRequired {
        interaction_id: AuthInteractionId,
        provider: AuthProviderId,
        label: CredentialAccountLabel,
        expires_at: Timestamp,
    },
    AccountSelectionRequired {
        provider: AuthProviderId,
        account_ids: Vec<CredentialAccountId>,
    },
    SetupRequired {
        provider: AuthProviderId,
        message: String,
    },
    ReauthorizeRequired {
        account_id: CredentialAccountId,
        provider: AuthProviderId,
    },
}

/// Typed continuation emitted after auth completion. It intentionally stores
/// references only, never raw prompt/message content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthContinuationRef {
    SetupOnly,
    LifecycleActivation {
        package_ref: LifecyclePackageRef,
    },
    TurnGateResume {
        turn_run_ref: TurnRunRef,
        gate_ref: AuthGateRef,
    },
    ProductActionResume {
        action_ref: ProductActionRef,
    },
}

/// Stable sanitized auth error vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum AuthErrorCode {
    #[error("unknown_or_expired_flow")]
    UnknownOrExpiredFlow,
    #[error("cross_scope_denied")]
    CrossScopeDenied,
    #[error("provider_denied")]
    ProviderDenied,
    #[error("token_exchange_failed")]
    TokenExchangeFailed,
    #[error("refresh_failed")]
    RefreshFailed,
    #[error("credential_missing")]
    CredentialMissing,
    #[error("account_selection_required")]
    AccountSelectionRequired,
    #[error("backend_unavailable")]
    BackendUnavailable,
    #[error("malformed_callback")]
    MalformedCallback,
    #[error("canceled")]
    Canceled,
}

/// Auth product service failures. Messages are stable and sanitized; raw
/// provider/backend bodies and secret values do not belong in this type.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthProductError {
    #[error("auth flow is unknown or expired")]
    UnknownOrExpiredFlow,
    #[error("auth record belongs to another scope")]
    CrossScopeDenied,
    #[error("auth callback is malformed")]
    MalformedCallback,
    #[error("provider denied authorization")]
    ProviderDenied,
    #[error("token exchange failed")]
    TokenExchangeFailed,
    #[error("credential is missing")]
    CredentialMissing,
    #[error("account selection required")]
    AccountSelectionRequired,
    #[error("backend unavailable")]
    BackendUnavailable,
    #[error("invalid auth request: {reason}")]
    InvalidRequest { reason: String },
}

impl AuthProductError {
    pub fn code(&self) -> AuthErrorCode {
        match self {
            Self::UnknownOrExpiredFlow => AuthErrorCode::UnknownOrExpiredFlow,
            Self::CrossScopeDenied => AuthErrorCode::CrossScopeDenied,
            Self::MalformedCallback => AuthErrorCode::MalformedCallback,
            Self::ProviderDenied => AuthErrorCode::ProviderDenied,
            Self::TokenExchangeFailed => AuthErrorCode::TokenExchangeFailed,
            Self::CredentialMissing => AuthErrorCode::CredentialMissing,
            Self::AccountSelectionRequired => AuthErrorCode::AccountSelectionRequired,
            Self::BackendUnavailable => AuthErrorCode::BackendUnavailable,
            Self::InvalidRequest { .. } => AuthErrorCode::BackendUnavailable,
        }
    }
}

/// Durable scoped auth flow record. OAuth state/verifier values are represented
/// by hashes/fingerprints only; raw state, authorization code, and PKCE verifier
/// must not be stored in adapter-visible records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlowRecord {
    pub id: AuthFlowId,
    pub scope: AuthProductScope,
    pub kind: AuthFlowKind,
    pub status: AuthFlowStatus,
    pub provider: AuthProviderId,
    pub challenge: Option<AuthChallenge>,
    pub continuation: AuthContinuationRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_account_id: Option<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opaque_state_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkce_verifier_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AuthErrorCode>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub expires_at: Timestamp,
}

/// Input used to create an auth flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuthFlow {
    pub scope: AuthProductScope,
    pub kind: AuthFlowKind,
    pub provider: AuthProviderId,
    pub challenge: AuthChallenge,
    pub continuation: AuthContinuationRef,
    pub opaque_state_hash: Option<String>,
    pub pkce_verifier_hash: Option<String>,
    pub expires_at: Timestamp,
}

/// Provider callback result after the public route has parsed query params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCallbackResult {
    Authorized { authorization_code_hash: String },
    Denied { error_code: String },
}

/// Typed OAuth callback completion input. Raw code/state/verifier material must
/// be validated/hashed before entering this contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackInput {
    pub flow_id: AuthFlowId,
    pub opaque_state_hash: String,
    pub provider_result: ProviderCallbackResult,
}

#[async_trait]
pub trait AuthFlowManager: Send + Sync {
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError>;

    async fn get_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError>;

    async fn complete_oauth_callback(
        &self,
        scope: &AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError>;

    async fn cancel_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError>;
}

/// Credential account status projected to product surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialAccountStatus {
    Configured,
    Missing,
    Expired,
    RefreshFailed,
    Revoked,
    PendingSetup,
}

/// Ownership class determines uninstall/deactivate cleanup behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialOwnership {
    ExtensionOwned,
    UserReusable,
    SharedAdminManaged,
    System,
}

/// Durable credential account metadata. Secret values live behind handles and
/// never appear in this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccount {
    pub id: CredentialAccountId,
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_extension: Option<ExtensionId>,
    #[serde(default)]
    pub granted_extensions: Vec<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_secret: Option<SecretHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_secret: Option<SecretHandle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl CredentialAccount {
    pub fn projection(&self) -> CredentialAccountProjection {
        let secret_handle_count =
            self.access_secret.iter().count() + self.refresh_secret.iter().count();
        CredentialAccountProjection {
            id: self.id,
            provider: self.provider.clone(),
            label: self.label.clone(),
            status: self.status,
            ownership: self.ownership,
            owner_extension: self.owner_extension.clone(),
            granted_extensions: self.granted_extensions.clone(),
            secret_handle_count,
        }
    }
}

/// Adapter-safe account projection. It does not include raw secret material or
/// backend secret handle names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccountProjection {
    pub id: CredentialAccountId,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_extension: Option<ExtensionId>,
    #[serde(default)]
    pub granted_extensions: Vec<ExtensionId>,
    pub secret_handle_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCredentialAccount {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    pub owner_extension: Option<ExtensionId>,
    pub granted_extensions: Vec<ExtensionId>,
    pub access_secret: Option<SecretHandle>,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<String>,
}

#[async_trait]
pub trait CredentialAccountService: Send + Sync {
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError>;

    async fn update_status(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError>;

    async fn list_accounts(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<Vec<CredentialAccountProjection>, AuthProductError>;

    async fn select_unique_configured_account(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<CredentialAccountProjection, AuthProductError>;
}

/// Request to create a secure secret-input interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretInputRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub ownership: CredentialOwnership,
    pub owner_extension: Option<ExtensionId>,
    pub continuation: AuthContinuationRef,
    pub expires_at: Timestamp,
}

/// Submitted secret value. Debug is intentionally redacted because this type is
/// the secure-input boundary for manual tokens.
pub struct SecretSubmitRequest {
    pub interaction_id: AuthInteractionId,
    pub secret: SecretString,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretSubmitResult {
    pub interaction_id: AuthInteractionId,
    pub account_id: CredentialAccountId,
    pub status: CredentialAccountStatus,
    pub continuation: AuthContinuationRef,
}

#[async_trait]
pub trait AuthInteractionService: Send + Sync {
    async fn request_secret_input(
        &self,
        request: SecretInputRequest,
    ) -> Result<AuthChallenge, AuthProductError>;

    async fn submit_secret(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError>;
}

#[async_trait]
pub trait CredentialSetupService: Send + Sync {
    async fn begin_manual_token_setup(
        &self,
        request: SecretInputRequest,
    ) -> Result<AuthChallenge, AuthProductError>;

    async fn submit_manual_token(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError>;
}

/// Sanitized token metadata returned by an auth provider client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenMetadata {
    pub provider: AuthProviderId,
    pub access_secret: SecretHandle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_secret: Option<SecretHandle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderCallbackRequest {
    pub provider: AuthProviderId,
    pub authorization_code_hash: String,
    pub flow_id: AuthFlowId,
}

#[async_trait]
pub trait AuthProviderClient: Send + Sync {
    async fn exchange_callback(
        &self,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthTokenMetadata, AuthProductError>;
}

/// Product workflow sink for typed continuations.
#[async_trait]
pub trait ProductWorkflowContinuationSink: Send + Sync {
    async fn enqueue_continuation(
        &self,
        scope: &AuthProductScope,
        continuation: AuthContinuationRef,
    ) -> Result<(), AuthProductError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretCleanupRequest {
    pub scope: AuthProductScope,
    pub extension_id: ExtensionId,
    pub action: SecretCleanupAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretCleanupAction {
    Deactivate,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretCleanupReport {
    pub action: SecretCleanupAction,
    pub revoked_accounts: Vec<CredentialAccountId>,
    pub retained_accounts: Vec<CredentialAccountId>,
    pub removed_grants: Vec<CredentialAccountId>,
    pub quarantine_diagnostics: Vec<String>,
}

impl SecretCleanupReport {
    fn empty(action: SecretCleanupAction) -> Self {
        Self {
            action,
            revoked_accounts: Vec::new(),
            retained_accounts: Vec::new(),
            removed_grants: Vec::new(),
            quarantine_diagnostics: Vec::new(),
        }
    }
}

#[async_trait]
pub trait SecretCleanupService: Send + Sync {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError>;
}

#[derive(Debug, Clone)]
struct PendingSecretInteraction {
    scope: AuthProductScope,
    provider: AuthProviderId,
    label: CredentialAccountLabel,
    ownership: CredentialOwnership,
    owner_extension: Option<ExtensionId>,
    continuation: AuthContinuationRef,
    expires_at: Timestamp,
}

#[derive(Debug, Default)]
struct AuthProductState {
    flows: HashMap<AuthFlowId, AuthFlowRecord>,
    accounts: HashMap<CredentialAccountId, CredentialAccount>,
    interactions: HashMap<AuthInteractionId, PendingSecretInteraction>,
    continuations: Vec<(AuthProductScope, AuthContinuationRef)>,
}

/// In-memory fake that implements the product auth contracts for contract tests
/// and early integration planning. It is not a durable production repository.
#[derive(Debug, Default)]
pub struct InMemoryAuthProductServices {
    state: Mutex<AuthProductState>,
}

impl InMemoryAuthProductServices {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, AuthProductState>, AuthProductError> {
        self.state
            .lock()
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    pub fn continuation_count(&self) -> Result<usize, AuthProductError> {
        Ok(self.lock_state()?.continuations.len())
    }
}

fn ensure_same_scope(
    expected: &AuthProductScope,
    actual: &AuthProductScope,
) -> Result<(), AuthProductError> {
    if expected == actual {
        Ok(())
    } else {
        Err(AuthProductError::CrossScopeDenied)
    }
}

fn now() -> Timestamp {
    Utc::now()
}

fn generated_secret_handle(prefix: &str, id: Uuid) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{prefix}-{id}")).map_err(|_| AuthProductError::InvalidRequest {
        reason: "generated secret handle was invalid".to_string(),
    })
}

#[async_trait]
impl AuthFlowManager for InMemoryAuthProductServices {
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        let mut state = self.lock_state()?;
        let timestamp = now();
        let id = match request.challenge {
            AuthChallenge::OAuthUrl { flow_id, .. } => flow_id,
            _ => AuthFlowId::new(),
        };
        let record = AuthFlowRecord {
            id,
            scope: request.scope,
            kind: request.kind,
            status: AuthFlowStatus::AwaitingUser,
            provider: request.provider,
            challenge: Some(request.challenge),
            continuation: request.continuation,
            credential_account_id: None,
            opaque_state_hash: request.opaque_state_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            error: None,
            created_at: timestamp,
            updated_at: timestamp,
            expires_at: request.expires_at,
        };
        state.flows.insert(record.id, record.clone());
        Ok(record)
    }

    async fn get_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let state = self.lock_state()?;
        let Some(record) = state.flows.get(&flow_id) else {
            return Ok(None);
        };
        ensure_same_scope(scope, &record.scope)?;
        Ok(Some(record.clone()))
    }

    async fn complete_oauth_callback(
        &self,
        scope: &AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mut state = self.lock_state()?;
        let record = state
            .flows
            .get_mut(&input.flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        ensure_same_scope(scope, &record.scope)?;

        if matches!(
            record.status,
            AuthFlowStatus::Completed
                | AuthFlowStatus::Failed
                | AuthFlowStatus::Expired
                | AuthFlowStatus::Canceled
        ) {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }

        if record.expires_at <= now() {
            record.status = AuthFlowStatus::Expired;
            record.error = Some(AuthErrorCode::UnknownOrExpiredFlow);
            record.updated_at = now();
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }

        match record.opaque_state_hash.as_deref() {
            Some(expected) if expected == input.opaque_state_hash => {}
            _ => {
                record.status = AuthFlowStatus::Failed;
                record.error = Some(AuthErrorCode::MalformedCallback);
                record.updated_at = now();
                return Err(AuthProductError::MalformedCallback);
            }
        }

        record.status = AuthFlowStatus::CallbackReceived;
        record.updated_at = now();

        match input.provider_result {
            ProviderCallbackResult::Authorized { .. } => {
                record.status = AuthFlowStatus::Completed;
                record.error = None;
                let continuation = (record.scope.clone(), record.continuation.clone());
                let completed = record.clone();
                state.continuations.push(continuation);
                Ok(completed)
            }
            ProviderCallbackResult::Denied { .. } => {
                record.status = AuthFlowStatus::Failed;
                record.error = Some(AuthErrorCode::ProviderDenied);
                Err(AuthProductError::ProviderDenied)
            }
        }
    }

    async fn cancel_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mut state = self.lock_state()?;
        let record = state
            .flows
            .get_mut(&flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        ensure_same_scope(scope, &record.scope)?;
        record.status = AuthFlowStatus::Canceled;
        record.error = Some(AuthErrorCode::Canceled);
        record.updated_at = now();
        Ok(record.clone())
    }
}

#[async_trait]
impl CredentialAccountService for InMemoryAuthProductServices {
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        let mut state = self.lock_state()?;
        let timestamp = now();
        let account = CredentialAccount {
            id: CredentialAccountId::new(),
            scope: request.scope,
            provider: request.provider,
            label: request.label,
            status: request.status,
            ownership: request.ownership,
            owner_extension: request.owner_extension,
            granted_extensions: request.granted_extensions,
            access_secret: request.access_secret,
            refresh_secret: request.refresh_secret,
            scopes: request.scopes,
            created_at: timestamp,
            updated_at: timestamp,
        };
        state.accounts.insert(account.id, account.clone());
        Ok(account)
    }

    async fn update_status(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError> {
        let mut state = self.lock_state()?;
        let account = state
            .accounts
            .get_mut(&account_id)
            .ok_or(AuthProductError::CredentialMissing)?;
        ensure_same_scope(scope, &account.scope)?;
        account.status = status;
        account.updated_at = now();
        Ok(account.clone())
    }

    async fn list_accounts(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<Vec<CredentialAccountProjection>, AuthProductError> {
        let state = self.lock_state()?;
        Ok(state
            .accounts
            .values()
            .filter(|account| &account.scope == scope && &account.provider == provider)
            .map(CredentialAccount::projection)
            .collect())
    }

    async fn select_unique_configured_account(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let mut configured = self
            .list_accounts(scope, provider)
            .await?
            .into_iter()
            .filter(|account| account.status == CredentialAccountStatus::Configured)
            .collect::<Vec<_>>();
        if configured.len() == 1 {
            Ok(configured.remove(0))
        } else {
            Err(AuthProductError::AccountSelectionRequired)
        }
    }
}

#[async_trait]
impl AuthInteractionService for InMemoryAuthProductServices {
    async fn request_secret_input(
        &self,
        request: SecretInputRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        let mut state = self.lock_state()?;
        let interaction_id = AuthInteractionId::new();
        let challenge = AuthChallenge::ManualTokenRequired {
            interaction_id,
            provider: request.provider.clone(),
            label: request.label.clone(),
            expires_at: request.expires_at,
        };
        state.interactions.insert(
            interaction_id,
            PendingSecretInteraction {
                scope: request.scope,
                provider: request.provider,
                label: request.label,
                ownership: request.ownership,
                owner_extension: request.owner_extension,
                continuation: request.continuation,
                expires_at: request.expires_at,
            },
        );
        Ok(challenge)
    }

    async fn submit_secret(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        if request.secret.expose_secret().trim().is_empty() {
            return Err(AuthProductError::InvalidRequest {
                reason: "credential cannot be empty".to_string(),
            });
        }

        let mut state = self.lock_state()?;
        let interaction = state
            .interactions
            .get(&request.interaction_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        ensure_same_scope(scope, &interaction.scope)?;
        if interaction.expires_at <= now() {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        let interaction = state
            .interactions
            .remove(&request.interaction_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;

        let account_id = CredentialAccountId::new();
        let timestamp = now();
        let account = CredentialAccount {
            id: account_id,
            scope: interaction.scope,
            provider: interaction.provider,
            label: interaction.label,
            status: CredentialAccountStatus::Configured,
            ownership: interaction.ownership,
            owner_extension: interaction.owner_extension,
            granted_extensions: Vec::new(),
            access_secret: Some(generated_secret_handle(
                "manual-token",
                account_id.as_uuid(),
            )?),
            refresh_secret: None,
            scopes: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        };
        state.accounts.insert(account_id, account);
        state
            .continuations
            .push((scope.clone(), interaction.continuation.clone()));

        Ok(SecretSubmitResult {
            interaction_id: request.interaction_id,
            account_id,
            status: CredentialAccountStatus::Configured,
            continuation: interaction.continuation,
        })
    }
}

#[async_trait]
impl CredentialSetupService for InMemoryAuthProductServices {
    async fn begin_manual_token_setup(
        &self,
        request: SecretInputRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        self.request_secret_input(request).await
    }

    async fn submit_manual_token(
        &self,
        scope: &AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        self.submit_secret(scope, request).await
    }
}

#[async_trait]
impl AuthProviderClient for InMemoryAuthProductServices {
    async fn exchange_callback(
        &self,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthTokenMetadata, AuthProductError> {
        if request.authorization_code_hash.is_empty() {
            return Err(AuthProductError::MalformedCallback);
        }
        Ok(OAuthTokenMetadata {
            provider: request.provider,
            access_secret: generated_secret_handle("oauth-access", request.flow_id.as_uuid())?,
            refresh_secret: Some(generated_secret_handle(
                "oauth-refresh",
                request.flow_id.as_uuid(),
            )?),
            scopes: Vec::new(),
            expires_at: None,
        })
    }
}

#[async_trait]
impl ProductWorkflowContinuationSink for InMemoryAuthProductServices {
    async fn enqueue_continuation(
        &self,
        scope: &AuthProductScope,
        continuation: AuthContinuationRef,
    ) -> Result<(), AuthProductError> {
        self.lock_state()?
            .continuations
            .push((scope.clone(), continuation));
        Ok(())
    }
}

#[async_trait]
impl SecretCleanupService for InMemoryAuthProductServices {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        let mut state = self.lock_state()?;
        let mut report = SecretCleanupReport::empty(request.action);
        for account in state.accounts.values_mut() {
            if account.scope != request.scope {
                continue;
            }

            let is_owner = account.owner_extension.as_ref() == Some(&request.extension_id);
            let had_grant = account.granted_extensions.contains(&request.extension_id);
            if !is_owner && !had_grant {
                continue;
            }

            account
                .granted_extensions
                .retain(|extension| extension != &request.extension_id);
            if had_grant {
                report.removed_grants.push(account.id);
            }

            match (request.action, account.ownership, is_owner) {
                (SecretCleanupAction::Uninstall, CredentialOwnership::ExtensionOwned, true) => {
                    if account.status != CredentialAccountStatus::Revoked {
                        account.status = CredentialAccountStatus::Revoked;
                        account.updated_at = now();
                        report.revoked_accounts.push(account.id);
                    }
                }
                (SecretCleanupAction::Deactivate, _, true | false)
                | (SecretCleanupAction::Uninstall, CredentialOwnership::UserReusable, _)
                | (SecretCleanupAction::Uninstall, CredentialOwnership::SharedAdminManaged, _)
                | (SecretCleanupAction::Uninstall, CredentialOwnership::System, _) => {
                    report.retained_accounts.push(account.id);
                }
                (SecretCleanupAction::Uninstall, CredentialOwnership::ExtensionOwned, false) => {}
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};

    fn scope(user: &str) -> AuthProductScope {
        AuthProductScope::new(
            ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
            AuthSurface::Web,
        )
    }

    fn provider() -> AuthProviderId {
        AuthProviderId::new("github").unwrap()
    }

    fn label() -> CredentialAccountLabel {
        CredentialAccountLabel::new("GitHub account").unwrap()
    }

    fn oauth_challenge(flow_id: AuthFlowId, expires_at: Timestamp) -> AuthChallenge {
        AuthChallenge::OAuthUrl {
            flow_id,
            auth_url: "https://github.com/login/oauth/authorize?client_id=redacted".to_string(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn oauth_callback_success_consumes_scoped_flow_and_enqueues_continuation() {
        let services = InMemoryAuthProductServices::new();
        let flow_id = AuthFlowId::new();
        let owner = scope("alice");
        let expires_at = Utc::now() + Duration::minutes(5);
        let record = services
            .create_flow(NewAuthFlow {
                scope: owner.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider(),
                challenge: oauth_challenge(flow_id, expires_at),
                continuation: AuthContinuationRef::LifecycleActivation {
                    package_ref: LifecyclePackageRef::new("github.tool").unwrap(),
                },
                opaque_state_hash: Some("state-hash".to_string()),
                pkce_verifier_hash: Some("verifier-hash".to_string()),
                expires_at,
            })
            .await
            .unwrap();

        assert_eq!(record.status, AuthFlowStatus::AwaitingUser);
        let completed = services
            .complete_oauth_callback(
                &owner,
                OAuthCallbackInput {
                    flow_id,
                    opaque_state_hash: "state-hash".to_string(),
                    provider_result: ProviderCallbackResult::Authorized {
                        authorization_code_hash: "code-hash".to_string(),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(completed.status, AuthFlowStatus::Completed);
        assert_eq!(services.continuation_count().unwrap(), 1);

        let replay = services
            .complete_oauth_callback(
                &owner,
                OAuthCallbackInput {
                    flow_id,
                    opaque_state_hash: "state-hash".to_string(),
                    provider_result: ProviderCallbackResult::Authorized {
                        authorization_code_hash: "code-hash".to_string(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(replay, AuthProductError::UnknownOrExpiredFlow);
        assert_eq!(services.continuation_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn oauth_callback_denies_cross_scope_and_stale_callbacks() {
        let services = InMemoryAuthProductServices::new();
        let flow_id = AuthFlowId::new();
        let owner = scope("alice");
        let other = scope("bob");
        let expires_at = Utc::now() + Duration::minutes(5);
        services
            .create_flow(NewAuthFlow {
                scope: owner.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider(),
                challenge: oauth_challenge(flow_id, expires_at),
                continuation: AuthContinuationRef::SetupOnly,
                opaque_state_hash: Some("state-hash".to_string()),
                pkce_verifier_hash: None,
                expires_at,
            })
            .await
            .unwrap();

        let err = services
            .complete_oauth_callback(
                &other,
                OAuthCallbackInput {
                    flow_id,
                    opaque_state_hash: "state-hash".to_string(),
                    provider_result: ProviderCallbackResult::Authorized {
                        authorization_code_hash: "code-hash".to_string(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err, AuthProductError::CrossScopeDenied);

        let expired_flow = AuthFlowId::new();
        let expired_at = Utc::now() - Duration::minutes(1);
        services
            .create_flow(NewAuthFlow {
                scope: owner.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider(),
                challenge: oauth_challenge(expired_flow, expired_at),
                continuation: AuthContinuationRef::SetupOnly,
                opaque_state_hash: Some("expired-state".to_string()),
                pkce_verifier_hash: None,
                expires_at: expired_at,
            })
            .await
            .unwrap();
        let err = services
            .complete_oauth_callback(
                &owner,
                OAuthCallbackInput {
                    flow_id: expired_flow,
                    opaque_state_hash: "expired-state".to_string(),
                    provider_result: ProviderCallbackResult::Authorized {
                        authorization_code_hash: "code-hash".to_string(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err, AuthProductError::UnknownOrExpiredFlow);
    }

    #[tokio::test]
    async fn oauth_callback_maps_malformed_and_provider_denied_to_stable_errors() {
        let services = InMemoryAuthProductServices::new();
        let flow_id = AuthFlowId::new();
        let owner = scope("alice");
        let expires_at = Utc::now() + Duration::minutes(5);
        services
            .create_flow(NewAuthFlow {
                scope: owner.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider(),
                challenge: oauth_challenge(flow_id, expires_at),
                continuation: AuthContinuationRef::SetupOnly,
                opaque_state_hash: Some("state-hash".to_string()),
                pkce_verifier_hash: None,
                expires_at,
            })
            .await
            .unwrap();

        let malformed = services
            .complete_oauth_callback(
                &owner,
                OAuthCallbackInput {
                    flow_id,
                    opaque_state_hash: "wrong-state".to_string(),
                    provider_result: ProviderCallbackResult::Authorized {
                        authorization_code_hash: "code-hash".to_string(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(malformed, AuthProductError::MalformedCallback);

        let denied_flow = AuthFlowId::new();
        services
            .create_flow(NewAuthFlow {
                scope: owner.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider(),
                challenge: oauth_challenge(denied_flow, expires_at),
                continuation: AuthContinuationRef::SetupOnly,
                opaque_state_hash: Some("denied-state".to_string()),
                pkce_verifier_hash: None,
                expires_at,
            })
            .await
            .unwrap();
        let denied = services
            .complete_oauth_callback(
                &owner,
                OAuthCallbackInput {
                    flow_id: denied_flow,
                    opaque_state_hash: "denied-state".to_string(),
                    provider_result: ProviderCallbackResult::Denied {
                        error_code: "access_denied".to_string(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(denied, AuthProductError::ProviderDenied);
    }

    #[tokio::test]
    async fn manual_token_submit_is_secure_and_redacted() {
        let services = InMemoryAuthProductServices::new();
        let owner = scope("alice");
        let challenge = services
            .begin_manual_token_setup(SecretInputRequest {
                scope: owner.clone(),
                provider: provider(),
                label: label(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                continuation: AuthContinuationRef::SetupOnly,
                expires_at: Utc::now() + Duration::minutes(5),
            })
            .await
            .unwrap();

        let interaction_id = match challenge {
            AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
            other => panic!("unexpected challenge: {other:?}"),
        };
        let submit = SecretSubmitRequest {
            interaction_id,
            secret: SecretString::new("ghp_super_secret_token".into()),
        };
        let debug = format!("{submit:?}");
        assert!(!debug.contains("ghp_super_secret_token"));
        assert!(debug.contains("[REDACTED]"));

        let cross_scope = services
            .submit_manual_token(
                &scope("bob"),
                SecretSubmitRequest {
                    interaction_id,
                    secret: SecretString::new("ghp_attacker_token".into()),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

        let result = services.submit_manual_token(&owner, submit).await.unwrap();
        assert_eq!(result.status, CredentialAccountStatus::Configured);
        let accounts = services.list_accounts(&owner, &provider()).await.unwrap();
        assert_eq!(accounts.len(), 1);
        let projection_debug = format!("{:?}", accounts[0]);
        assert!(!projection_debug.contains("ghp_super_secret_token"));
        assert_eq!(accounts[0].secret_handle_count, 1);
    }

    #[tokio::test]
    async fn credential_states_and_account_selection_are_recoverable() {
        let services = InMemoryAuthProductServices::new();
        let owner = scope("alice");
        let access = SecretHandle::new("github-access-token").unwrap();
        let first = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: CredentialAccountLabel::new("work").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(access),
                refresh_secret: None,
                scopes: vec!["repo".to_string()],
            })
            .await
            .unwrap();
        services
            .update_status(&owner, first.id, CredentialAccountStatus::RefreshFailed)
            .await
            .unwrap();
        let err = services
            .select_unique_configured_account(&owner, &provider())
            .await
            .unwrap_err();
        assert_eq!(err, AuthProductError::AccountSelectionRequired);

        let second = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: CredentialAccountLabel::new("personal").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("github-personal-token").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let selected = services
            .select_unique_configured_account(&owner, &provider())
            .await
            .unwrap();
        assert_eq!(selected.id, second.id);
    }

    #[tokio::test]
    async fn continuation_refs_do_not_store_raw_prompt_or_replay_content() {
        let continuation = AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new("turn-run-1").unwrap(),
            gate_ref: AuthGateRef::new("auth-gate-1").unwrap(),
        };
        let rendered = format!("{continuation:?}");
        assert!(!rendered.contains("send this original prompt"));
        assert!(rendered.contains("TurnGateResume"));
    }

    #[tokio::test]
    async fn cleanup_is_ownership_aware_and_idempotent() {
        let services = InMemoryAuthProductServices::new();
        let owner = scope("alice");
        let extension = ExtensionId::new("github").unwrap();
        let owned = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: CredentialAccountLabel::new("extension owned").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(extension.clone()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("github-owned-token").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let reusable = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: CredentialAccountLabel::new("user reusable").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![extension.clone()],
                access_secret: Some(SecretHandle::new("github-reusable-token").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();

        let report = services
            .cleanup_for_lifecycle(SecretCleanupRequest {
                scope: owner.clone(),
                extension_id: extension.clone(),
                action: SecretCleanupAction::Uninstall,
            })
            .await
            .unwrap();
        assert!(report.revoked_accounts.contains(&owned.id));
        assert!(report.retained_accounts.contains(&reusable.id));
        assert!(report.removed_grants.contains(&reusable.id));

        let again = services
            .cleanup_for_lifecycle(SecretCleanupRequest {
                scope: owner,
                extension_id: extension,
                action: SecretCleanupAction::Uninstall,
            })
            .await
            .unwrap();
        assert!(again.revoked_accounts.is_empty());
        assert!(again.removed_grants.is_empty());
        assert!(again.quarantine_diagnostics.is_empty());
    }
}
