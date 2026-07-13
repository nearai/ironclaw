// arch-exempt: large_file, shared product-auth callback and cleanup service, plan #5905
use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthChallenge, AuthContinuationDispatchClaimInput, AuthContinuationDispatchOutcome,
    AuthContinuationDispatchSettlementInput, AuthContinuationEvent, AuthContinuationRef,
    AuthErrorCode, AuthFlowId, AuthFlowKind, AuthFlowManager, AuthFlowOwnerScope, AuthFlowRecord,
    AuthFlowRecordSource, AuthFlowStatus, AuthGateRef, AuthInteractionId, AuthInteractionService,
    AuthProductError, AuthProductScope, AuthProviderClient, AuthProviderId,
    CredentialAccountChoiceRequest, CredentialAccountId, CredentialAccountLabel,
    CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
    CredentialAccountProjection, CredentialAccountRecordSource, CredentialAccountService,
    CredentialAccountStatus, CredentialAccountUpdateBinding, CredentialRecoveryProjection,
    CredentialRecoveryRequest, CredentialRefreshReport, CredentialRefreshRequest,
    CredentialSetupService, InMemoryAuthProductServices, ManualTokenSetupRequest, NewAuthFlow,
    OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
    OAuthCallbackInput, OAuthCompletionCompensationOutcome, OAuthCompletionCompensationRequest,
    OAuthExchangeCleanupRequest, OAuthProviderCallbackRequest, OAuthProviderExchangeContext,
    OAuthProviderIdentity, OpaqueStateHash, PkceVerifierHash,
    ProviderBackedCredentialAccountService, ProviderCallbackOutcome, ProviderScope,
    SecretCleanupReport, SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest,
    SecretSubmitResult, Timestamp, TurnGateAuthFlowQuery, TurnRunRef, scope_matches,
};
use ironclaw_events::{SecurityAuditEvent, SecurityAuditSink, SecurityBoundary, SecurityDecision};
use ironclaw_product_adapters::AuthPromptChallengeKind;
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::{ExtensionId, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::RebornBuildError;
use crate::product_auth::credentials::manual_token_flow::{
    PortBackedManualTokenFlowService, RebornManualTokenFlowService,
};
use crate::product_auth::credentials::runtime_credentials::host_managed_fallback::{
    HostManagedCredentialFallbackRule, HostManagedRuntimeCredentialAccountSelector,
};
use crate::product_auth::credentials::runtime_credentials::{
    ProductAuthRuntimeCredentialAccountRefresher, ProductAuthRuntimeCredentialAccountSelector,
    RuntimeCredentialAccountRefreshPort, RuntimeCredentialAccountRefreshService,
    RuntimeCredentialAccountSelectionService,
};
use crate::product_auth::oauth::oauth_dcr::{
    DcrGateChallengeRequest, DcrSetupFlowRequest, OAuthDcrProviderRegistry,
};
use crate::product_auth::oauth::oauth_gate::{
    OAuthGateChallengeRequest, OAuthGateProviderRegistry,
};
use crate::{AuthChallengeProvider, AuthChallengeView, BlockedAuthFlowCanceller};

pub(crate) const AUTH_CONTINUATION_DISPATCH_FAILED_CODE: &str = "auth_continuation_dispatch_failed";

/// Dispatches a typed continuation event once an OAuth callback flow has
/// completed.
///
/// # Idempotency contract
///
/// Implementations MUST be idempotent on `flow_id`.  The product-auth layer
/// guarantees *at-least-once* delivery: if `dispatch_auth_continuation`
/// succeeds but the subsequent `mark_continuation_dispatched` call fails
/// (e.g. a transient `BackendConflict` or `BackendUnavailable`), the caller
/// will retry the full callback path and dispatch the same `flow_id` again.
/// An implementation that assumes exactly-once delivery will process duplicate
/// continuations and is incorrect.
#[async_trait]
pub trait RebornAuthContinuationDispatcher: Send + Sync {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError>;

    /// Deny a turn gate whose backing auth flow was canceled by lifecycle
    /// cleanup. Non-turn continuations remain cancel-only.
    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct NoopAuthContinuationDispatcher;

#[cfg(test)]
#[async_trait]
impl RebornAuthContinuationDispatcher for NoopAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for ProductAuthTurnGateResumeDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_auth_continuation(self, event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_canceled_auth_continuation(self, event).await
    }
}

/// Parsed OAuth callback request handed from a host-owned HTTP route into the
/// Reborn product-auth boundary.
///
/// Raw query/body parsing and hashing are host-route responsibilities. This
/// type intentionally receives only the validated scope, flow id, state hash,
/// and one-shot provider exchange input. It is not serializable because the
/// authorized outcome can carry raw OAuth code/verifier material inside
/// [`OAuthProviderCallbackRequest`].
#[derive(Debug)]
pub struct RebornOAuthCallbackRequest {
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
    pub opaque_state_hash: OpaqueStateHash,
    pub outcome: RebornOAuthCallbackOutcome,
}

/// Typed setup OAuth start request after host-route parsing and hashing.
///
/// The browser-facing route chooses neither flow kind nor continuation. Those
/// product-auth semantics stay here with the auth service boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RebornOAuthStartFlowRequest {
    pub(crate) flow_id: Option<AuthFlowId>,
    pub(crate) scope: AuthProductScope,
    pub(crate) provider: AuthProviderId,
    pub(crate) authorization_url: OAuthAuthorizationUrl,
    pub(crate) opaque_state_hash: OpaqueStateHash,
    pub(crate) pkce_verifier_hash: PkceVerifierHash,
    pub(crate) continuation: AuthContinuationRef,
    pub(crate) update_binding: Option<CredentialAccountUpdateBinding>,
    pub(crate) expires_at: ironclaw_auth::Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "used by the webui-v2-beta extension OAuth route through product-auth route composition"
)]
pub(crate) struct RebornDcrOAuthStartFlowRequest {
    pub(crate) scope: AuthProductScope,
    pub(crate) provider: AuthProviderId,
    pub(crate) account_label: CredentialAccountLabel,
    pub(crate) provider_scopes: Vec<ProviderScope>,
    pub(crate) continuation: AuthContinuationRef,
    pub(crate) update_binding: Option<CredentialAccountUpdateBinding>,
    pub(crate) expires_at: ironclaw_auth::Timestamp,
}

/// Host-route OAuth callback parse result.
#[derive(Debug)]
pub enum RebornOAuthCallbackOutcome {
    Authorized {
        provider_request: OAuthProviderCallbackRequest,
    },
    ProviderDenied,
    Malformed,
}

/// Stable sanitized callback response safe for Web/CLI/API surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOAuthCallbackResponse {
    pub flow_id: AuthFlowId,
    pub status: AuthFlowStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_account_id: Option<CredentialAccountId>,
    pub continuation: AuthContinuationRef,
    #[serde(skip)]
    pub provider_identity: Option<OAuthProviderIdentity>,
}

/// Stable sanitized auth failure safe for route rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAuthProductError {
    pub code: AuthErrorCode,
    pub retryable: bool,
}

impl From<AuthProductError> for RebornAuthProductError {
    fn from(error: AuthProductError) -> Self {
        let code = error.code();
        Self {
            code,
            retryable: is_retryable_auth_error(code),
        }
    }
}

/// Stable sanitized callback failure safe for route rendering.
pub type RebornOAuthCallbackError = RebornAuthProductError;

/// Internal callback failure classification used by provider-owned route
/// cleanup. A continuation acknowledgement failure happens only after the
/// continuation side effect succeeded, so treating it as a terminal provider
/// failure would tear down a working connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebornOAuthCallbackFailureStage {
    Terminal,
    ContinuationRetryable,
    ContinuationSideEffect,
    ContinuationCompensation,
    ContinuationAcknowledgement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RebornOAuthCallbackAttemptError {
    error: RebornOAuthCallbackError,
    stage: RebornOAuthCallbackFailureStage,
}

impl RebornOAuthCallbackAttemptError {
    fn continuation_acknowledgement(error: AuthProductError) -> Self {
        Self {
            error: error.into(),
            stage: RebornOAuthCallbackFailureStage::ContinuationAcknowledgement,
        }
    }

    fn continuation_side_effect(error: AuthProductError) -> Self {
        Self {
            error: error.into(),
            stage: RebornOAuthCallbackFailureStage::ContinuationSideEffect,
        }
    }

    fn continuation_retryable(error: AuthProductError) -> Self {
        Self {
            error: error.into(),
            stage: RebornOAuthCallbackFailureStage::ContinuationRetryable,
        }
    }

    fn continuation_compensation(error: AuthProductError) -> Self {
        Self {
            error: error.into(),
            stage: RebornOAuthCallbackFailureStage::ContinuationCompensation,
        }
    }

    pub(crate) fn error(self) -> RebornOAuthCallbackError {
        self.error
    }

    #[allow(
        dead_code,
        reason = "used by feature-scoped product-auth route composition"
    )]
    pub(crate) fn stage(self) -> RebornOAuthCallbackFailureStage {
        self.stage
    }
}

impl From<AuthProductError> for RebornOAuthCallbackAttemptError {
    fn from(error: AuthProductError) -> Self {
        Self {
            error: error.into(),
            stage: RebornOAuthCallbackFailureStage::Terminal,
        }
    }
}

impl From<RebornOAuthCallbackError> for RebornOAuthCallbackAttemptError {
    fn from(error: RebornOAuthCallbackError) -> Self {
        Self {
            error,
            stage: RebornOAuthCallbackFailureStage::Terminal,
        }
    }
}

enum ContinuationDispatchFailure {
    SideEffect {
        error: AuthProductError,
        settled: Box<AuthFlowRecord>,
    },
    Acknowledgement(AuthProductError),
}

impl ContinuationDispatchFailure {
    fn into_auth_error(self) -> AuthProductError {
        match self {
            Self::SideEffect { error, .. } | Self::Acknowledgement(error) => error,
        }
    }
}

/// Compensating action returned by a provider-identity hook that committed
/// durable state (e.g. the Slack identity binding) before the flow completes.
/// Awaited only when `complete_oauth_callback` fails after the hook
/// succeeded; dropped unpolled on the success path. Infallible by contract —
/// implementations log their own failures.
pub(crate) type OAuthProviderIdentityBindingRollback = Pin<Box<dyn Future<Output = ()> + Send>>;
pub(crate) type OAuthProviderIdentityCheckFuture = Pin<
    Box<
        dyn Future<Output = Result<Option<OAuthProviderIdentityBindingRollback>, AuthProductError>>
            + Send,
    >,
>;
pub(crate) type OAuthProviderIdentityCheck =
    Box<dyn FnOnce(Option<OAuthProviderIdentity>) -> OAuthProviderIdentityCheckFuture + Send>;

/// Request to open a Reborn manual-token setup interaction.
///
/// This request is intentionally not serializable because the scope must be
/// constructed from trusted caller/session context, not copied from a browser
/// body. The raw token is submitted later through
/// [`RebornManualTokenSubmitRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornManualTokenSetupRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub continuation: AuthContinuationRef,
    pub update_binding: Option<CredentialAccountUpdateBinding>,
    pub expires_at: Timestamp,
}

impl RebornManualTokenSetupRequest {
    pub fn new(
        scope: AuthProductScope,
        provider: AuthProviderId,
        label: CredentialAccountLabel,
        continuation: AuthContinuationRef,
        expires_at: Timestamp,
    ) -> Self {
        Self {
            scope,
            provider,
            label,
            continuation,
            update_binding: None,
            expires_at,
        }
    }

    pub fn with_update_binding(mut self, update_binding: CredentialAccountUpdateBinding) -> Self {
        self.update_binding = Some(update_binding);
        self
    }
}

/// Manual-token challenge safe to render to Web/CLI/API surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornManualTokenChallenge {
    pub interaction_id: AuthInteractionId,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub expires_at: Timestamp,
}

/// Secure manual-token submit request.
///
/// This type intentionally does not implement serde serialization. Host-owned
/// routes may construct it after reading a dedicated secret input body, but raw
/// token material must not be written into product DTOs, projections, logs, or
/// model-visible messages.
pub struct RebornManualTokenSubmitRequest {
    pub scope: AuthProductScope,
    pub interaction_id: AuthInteractionId,
    pub secret: SecretString,
}

impl RebornManualTokenSubmitRequest {
    pub fn new(
        scope: AuthProductScope,
        interaction_id: AuthInteractionId,
        secret: SecretString,
    ) -> Self {
        Self {
            scope,
            interaction_id,
            secret,
        }
    }
}

impl std::fmt::Debug for RebornManualTokenSubmitRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornManualTokenSubmitRequest")
            .field("scope", &self.scope)
            .field("interaction_id", &self.interaction_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// Stable sanitized manual-token submit response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornManualTokenSubmitResponse {
    pub account_id: CredentialAccountId,
    pub status: CredentialAccountStatus,
    pub continuation: AuthContinuationRef,
}

/// Stable sanitized manual-token setup/submit failure safe for route rendering.
pub type RebornManualTokenError = RebornAuthProductError;

/// Stable sanitized lifecycle failure safe for Web/CLI/API surfaces.
pub type RebornCredentialLifecycleError = RebornAuthProductError;

fn is_retryable_auth_error(code: AuthErrorCode) -> bool {
    matches!(code, AuthErrorCode::BackendUnavailable)
}

#[derive(Debug)]
struct UnsupportedCredentialAccountRecordSource;

#[async_trait]
impl CredentialAccountRecordSource for UnsupportedCredentialAccountRecordSource {
    async fn accounts_for_owner(
        &self,
        _scope: &AuthProductScope,
    ) -> Result<Vec<ironclaw_auth::CredentialAccount>, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
}

#[derive(Clone)]
pub struct RebornProductAuthServicePorts {
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
    manual_token_flow_service: Arc<dyn RebornManualTokenFlowService>,
    credential_setup_service: Arc<dyn CredentialSetupService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
    credential_account_record_source: Arc<dyn CredentialAccountRecordSource>,
    provider_client: Arc<dyn AuthProviderClient>,
    cleanup_service: Arc<dyn SecretCleanupService>,
}

impl std::fmt::Debug for RebornProductAuthServicePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornProductAuthServicePorts")
            .field("flow_manager", &"Arc<dyn AuthFlowManager>")
            .field("interaction_service", &"Arc<dyn AuthInteractionService>")
            .field(
                "manual_token_flow_service",
                &"Arc<dyn RebornManualTokenFlowService>",
            )
            .field(
                "credential_setup_service",
                &"Arc<dyn CredentialSetupService>",
            )
            .field(
                "credential_account_service",
                &"Arc<dyn CredentialAccountService>",
            )
            .field(
                "credential_account_record_source",
                &"Arc<dyn CredentialAccountRecordSource>",
            )
            .field("provider_client", &"Arc<dyn AuthProviderClient>")
            .field("cleanup_service", &"Arc<dyn SecretCleanupService>")
            .finish()
    }
}

impl RebornProductAuthServicePorts {
    pub fn new(
        flow_manager: Arc<dyn AuthFlowManager>,
        interaction_service: Arc<dyn AuthInteractionService>,
        credential_setup_service: Arc<dyn CredentialSetupService>,
        credential_account_service: Arc<dyn CredentialAccountService>,
        provider_client: Arc<dyn AuthProviderClient>,
        cleanup_service: Arc<dyn SecretCleanupService>,
    ) -> Self {
        let manual_token_flow_service = Arc::new(PortBackedManualTokenFlowService::new(
            flow_manager.clone(),
            interaction_service.clone(),
            credential_account_service.clone(),
        ));
        Self {
            flow_manager,
            interaction_service,
            manual_token_flow_service,
            credential_setup_service,
            credential_account_service,
            credential_account_record_source: Arc::new(UnsupportedCredentialAccountRecordSource),
            provider_client,
            cleanup_service,
        }
    }

    pub fn from_shared<T>(services: Arc<T>) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + CredentialAccountRecordSource
            + AuthProviderClient
            + SecretCleanupService
            + RebornManualTokenFlowService
            + 'static,
    {
        let provider_client: Arc<dyn AuthProviderClient> = services.clone();
        Self::from_shared_with_provider(services, provider_client)
    }

    pub fn from_shared_with_provider<T>(
        services: Arc<T>,
        provider_client: Arc<dyn AuthProviderClient>,
    ) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + CredentialAccountRecordSource
            + SecretCleanupService
            + RebornManualTokenFlowService
            + 'static,
    {
        let flow_manager: Arc<dyn AuthFlowManager> = services.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = services.clone();
        let manual_token_flow_service: Arc<dyn RebornManualTokenFlowService> = services.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = services.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = services.clone();
        let credential_account_record_source: Arc<dyn CredentialAccountRecordSource> =
            services.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = services;

        let mut ports = Self::new(
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
        );
        ports.manual_token_flow_service = manual_token_flow_service;
        ports.credential_account_record_source = credential_account_record_source;
        ports
    }

    pub fn credential_account_service(&self) -> Arc<dyn CredentialAccountService> {
        self.credential_account_service.clone()
    }

    pub(crate) fn into_services(
        self,
        continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
        secret_store: Arc<dyn ironclaw_secrets::SecretStore>,
    ) -> RebornProductAuthServices {
        // `secret_store` is required here (not defaulted) so the store that the
        // OAuth provider client writes access-token `expires_at` to is
        // structurally the same store the inline-refresh margin check (A2)
        // reads from. Defaulting it would silently split the read/write stores
        // and make the conditional-refresh skip a no-op in production.
        RebornProductAuthServices::new(
            self.flow_manager,
            self.interaction_service,
            self.credential_setup_service,
            self.credential_account_service,
            self.provider_client,
            self.cleanup_service,
            continuation_dispatcher,
        )
        .with_manual_token_flow_service(self.manual_token_flow_service)
        .with_credential_account_record_source(self.credential_account_record_source)
        .with_secret_store(secret_store)
    }

    pub fn with_provider_client(mut self, provider_client: Arc<dyn AuthProviderClient>) -> Self {
        self.credential_account_service = Arc::new(ProviderBackedCredentialAccountService::new(
            self.credential_account_service,
            self.credential_setup_service.clone(),
            provider_client.clone(),
        ));
        self.provider_client = provider_client;
        self
    }
}

/// Reborn product-auth service bundle exposed by the composition root.
///
/// This is the single composition seam for product-facing auth flows,
/// credential accounts, secure manual-token interactions, provider exchange,
/// and lifecycle cleanup. It deliberately exposes trait-shaped ports only:
/// WebUI/setup/extension callers should enter here instead of reaching into
/// lower auth stores, provider clients, or route-local state.
#[derive(Clone)]
pub struct RebornProductAuthServices {
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
    manual_token_flow_service: Arc<dyn RebornManualTokenFlowService>,
    credential_setup_service: Arc<dyn CredentialSetupService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
    credential_account_record_source: Arc<dyn CredentialAccountRecordSource>,
    provider_client: Arc<dyn AuthProviderClient>,
    cleanup_service: Arc<dyn SecretCleanupService>,
    continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    /// Secret store forwarded to the inline-refresh margin check (A2).
    secret_store: Arc<dyn ironclaw_secrets::SecretStore>,
    host_managed_nearai_credential_scope: Option<AuthProductScope>,
    dcr_oauth_registry: Option<Arc<OAuthDcrProviderRegistry>>,
    /// One generic blocked-gate OAuth provider registry covering every provider
    /// (Google + Slack personal). Unified from the former parallel per-provider
    /// registries via `OAuthGateProvider` + `OAuthGateFlowDriver`.
    oauth_gate_registry: Option<Arc<OAuthGateProviderRegistry>>,
    /// Optional read projection for WebUI/local-dev auth interactions.
    ///
    /// `RebornProductAuthServices` may still support OAuth callbacks,
    /// manual-token setup, credential refresh, and continuation dispatch
    /// without this port. When absent, runtime composition must expose the
    /// WebUI pending-auth interaction surface as explicitly unavailable
    /// instead of silently fabricating an unscoped read model.
    ///
    /// arch-exempt: optional Arc, durable auth-flow read projection is tracked
    /// by product-auth issue #4112 and remains genuinely optional until the
    /// durable backend exposes the same scoped projection as the in-memory port.
    flow_record_source: Option<Arc<dyn AuthFlowRecordSource>>,
}

impl std::fmt::Debug for RebornProductAuthServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = formatter.debug_struct("RebornProductAuthServices");
        dbg.field("flow_manager", &"Arc<dyn AuthFlowManager>")
            .field("interaction_service", &"Arc<dyn AuthInteractionService>")
            .field(
                "manual_token_flow_service",
                &"Arc<dyn RebornManualTokenFlowService>",
            )
            .field(
                "credential_setup_service",
                &"Arc<dyn CredentialSetupService>",
            )
            .field(
                "credential_account_service",
                &"Arc<dyn CredentialAccountService>",
            )
            .field(
                "credential_account_record_source",
                &"Arc<dyn CredentialAccountRecordSource>",
            )
            .field("provider_client", &"Arc<dyn AuthProviderClient>")
            .field("cleanup_service", &"Arc<dyn SecretCleanupService>")
            .field(
                "continuation_dispatcher",
                &"Arc<dyn RebornAuthContinuationDispatcher>",
            )
            .field("security_audit_sink", &self.security_audit_sink.is_some())
            .field("secret_store", &"<wired>")
            .field(
                "host_managed_nearai_credential_scope",
                &self.host_managed_nearai_credential_scope.is_some(),
            )
            .field("flow_record_source", &self.flow_record_source.is_some())
            .field("dcr_oauth_registry", &self.dcr_oauth_registry.is_some())
            .field("oauth_gate_registry", &self.oauth_gate_registry.is_some());
        dbg.finish()
    }
}

impl RebornProductAuthServices {
    pub fn new(
        flow_manager: Arc<dyn AuthFlowManager>,
        interaction_service: Arc<dyn AuthInteractionService>,
        credential_setup_service: Arc<dyn CredentialSetupService>,
        credential_account_service: Arc<dyn CredentialAccountService>,
        provider_client: Arc<dyn AuthProviderClient>,
        cleanup_service: Arc<dyn SecretCleanupService>,
        continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        let manual_token_flow_service = Arc::new(PortBackedManualTokenFlowService::new(
            flow_manager.clone(),
            interaction_service.clone(),
            credential_account_service.clone(),
        ));
        Self {
            flow_manager,
            interaction_service,
            manual_token_flow_service,
            credential_setup_service,
            credential_account_service,
            credential_account_record_source: Arc::new(UnsupportedCredentialAccountRecordSource),
            provider_client,
            cleanup_service,
            continuation_dispatcher,
            security_audit_sink: None,
            secret_store: Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            host_managed_nearai_credential_scope: None,
            dcr_oauth_registry: None,
            oauth_gate_registry: None,
            flow_record_source: None,
        }
    }

    /// Builds a bundle from one object that implements every product-auth port.
    ///
    /// This is primarily for unified fakes such as
    /// [`InMemoryAuthProductServices`]. Production composition should prefer
    /// [`Self::new`] so storage, provider egress, interaction, and cleanup can
    /// be supplied by separate implementations.
    pub fn from_shared<T>(
        services: Arc<T>,
        continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + CredentialAccountRecordSource
            + AuthProviderClient
            + SecretCleanupService
            + RebornManualTokenFlowService
            + 'static,
    {
        let flow_manager: Arc<dyn AuthFlowManager> = services.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = services.clone();
        let manual_token_flow_service: Arc<dyn RebornManualTokenFlowService> = services.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = services.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = services.clone();
        let credential_account_record_source: Arc<dyn CredentialAccountRecordSource> =
            services.clone();
        let provider_client: Arc<dyn AuthProviderClient> = services.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = services;

        Self::new(
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
            continuation_dispatcher,
        )
        .with_manual_token_flow_service(manual_token_flow_service)
        .with_credential_account_record_source(credential_account_record_source)
    }

    #[cfg(test)]
    pub fn from_shared_with_noop_dispatcher_for_tests<T>(services: Arc<T>) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + CredentialAccountRecordSource
            + AuthProviderClient
            + SecretCleanupService
            + RebornManualTokenFlowService
            + 'static,
    {
        Self::from_shared(services, Arc::new(NoopAuthContinuationDispatcher))
    }

    pub fn flow_manager(&self) -> Arc<dyn AuthFlowManager> {
        self.flow_manager.clone()
    }

    /// Auth-flow read projection used only by product/WebUI interaction views.
    ///
    /// `None` is an intentional unsupported mode for bundles that can perform
    /// product-auth side effects but do not provide a scoped pending-auth
    /// projection. Callers must map it to a stable unavailable surface.
    pub(crate) fn flow_record_source(&self) -> Option<Arc<dyn AuthFlowRecordSource>> {
        self.flow_record_source.clone()
    }

    pub fn interaction_service(&self) -> Arc<dyn AuthInteractionService> {
        self.interaction_service.clone()
    }

    pub fn credential_setup_service(&self) -> Arc<dyn CredentialSetupService> {
        self.credential_setup_service.clone()
    }

    pub fn credential_account_service(&self) -> Arc<dyn CredentialAccountService> {
        self.credential_account_service.clone()
    }

    pub(crate) fn credential_account_record_source(
        &self,
    ) -> Arc<dyn CredentialAccountRecordSource> {
        self.credential_account_record_source.clone()
    }

    /// Test-support access to the owner-scoped credential account record source.
    ///
    /// Live fixture recorders use this to copy explicitly requested product-auth
    /// accounts from a developer's local Reborn store into an isolated test
    /// runtime without cloning the whole store.
    #[cfg(feature = "test-support")]
    pub fn credential_account_record_source_for_test(
        &self,
    ) -> Arc<dyn CredentialAccountRecordSource> {
        self.credential_account_record_source()
    }

    pub(crate) fn runtime_credential_account_selection_service(
        &self,
    ) -> Arc<dyn RuntimeCredentialAccountSelectionService> {
        let selector: Arc<dyn RuntimeCredentialAccountSelectionService> = Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
                self.credential_account_record_source(),
                Arc::new(
                    crate::extension_host::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy,
                ),
            ),
        );
        let Some(host_scope) = self.host_managed_nearai_credential_scope.clone() else {
            return selector;
        };
        // The host-managed NEAR AI MCP key is the only fallback rule today;
        // the generic `ProductAuthRuntimeCredentialAccountSelector` stays
        // provider-agnostic and this composition layer supplies the one
        // provider/extension pair that may fall back to it.
        let nearai_provider =
            AuthProviderId::new("nearai").expect("\"nearai\" is a valid AuthProviderId literal"); // safety: fixed literal, validation cannot fail
        let nearai_requester =
            ExtensionId::new("nearai").expect("\"nearai\" is a valid ExtensionId literal"); // safety: fixed literal, validation cannot fail
        let fallback =
            HostManagedCredentialFallbackRule::new(nearai_provider, nearai_requester, host_scope);
        Arc::new(HostManagedRuntimeCredentialAccountSelector::new(
            selector, fallback,
        ))
    }

    pub(crate) fn runtime_credential_account_refresh_service(
        self: &Arc<Self>,
    ) -> Arc<dyn RuntimeCredentialAccountRefreshService> {
        // Inline dispatch path: use the plain provider-backed service wrapped
        // only in the in-process `refresh_locks` guard that
        // `ProviderBackedCredentialAccountService` already owns. Cross-process
        // serialization is handled by the background keepalive worker's leader
        // lock (`CredentialRefreshLeaderLock`), not here.
        //
        // A2: Forward the secret store so the refresher can read `expires_at`
        // metadata and skip the token-endpoint round-trip when the access token
        // is still fresh. The margin is fixed at `DEFAULT_ACCESS_REFRESH_MARGIN`.
        let inner_port: Arc<dyn RuntimeCredentialAccountRefreshPort> = self.clone();
        let secret_store: Arc<dyn ironclaw_secrets::SecretStore> = self.secret_store.clone();
        Arc::new(ProductAuthRuntimeCredentialAccountRefresher::new(
            inner_port,
            secret_store,
        ))
    }

    pub fn provider_client(&self) -> Arc<dyn AuthProviderClient> {
        self.provider_client.clone()
    }

    pub fn cleanup_service(&self) -> Arc<dyn SecretCleanupService> {
        self.cleanup_service.clone()
    }

    pub fn with_provider_client(mut self, provider_client: Arc<dyn AuthProviderClient>) -> Self {
        self.credential_account_service = Arc::new(ProviderBackedCredentialAccountService::new(
            self.credential_account_service,
            self.credential_setup_service.clone(),
            provider_client.clone(),
        ));
        self.provider_client = provider_client;
        self
    }

    pub(crate) fn with_dcr_oauth_registry(
        mut self,
        registry: Arc<OAuthDcrProviderRegistry>,
    ) -> Self {
        self.dcr_oauth_registry = Some(registry);
        self
    }

    pub(crate) fn with_oauth_gate_registry(
        mut self,
        registry: Arc<OAuthGateProviderRegistry>,
    ) -> Self {
        self.oauth_gate_registry = Some(registry);
        self
    }

    fn with_manual_token_flow_service(
        mut self,
        service: Arc<dyn RebornManualTokenFlowService>,
    ) -> Self {
        self.manual_token_flow_service = service;
        self
    }

    fn with_credential_account_record_source(
        mut self,
        source: Arc<dyn CredentialAccountRecordSource>,
    ) -> Self {
        self.credential_account_record_source = source;
        self
    }

    pub fn with_continuation_dispatcher(
        mut self,
        dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        self.continuation_dispatcher = dispatcher;
        self
    }

    pub fn with_security_audit_sink(mut self, sink: Arc<dyn SecurityAuditSink>) -> Self {
        self.security_audit_sink = Some(sink);
        self
    }

    /// Wire the secret store used by the inline OAuth refresh margin check
    /// (A2). When set, the refresher reads `expires_at` metadata from the
    /// store and skips an unnecessary token-endpoint round-trip when the
    /// access token is still fresh. Defaults to an in-memory store (always
    /// refreshes unconditionally — safe, backward-compatible).
    pub fn with_secret_store(mut self, store: Arc<dyn ironclaw_secrets::SecretStore>) -> Self {
        self.secret_store = store;
        self
    }

    /// Wire the host-managed NEAR AI MCP credential fallback scope.
    ///
    /// Consuming builder — call before wrapping the bundle in `Arc`, so
    /// composition never depends on `Arc::get_mut` succeeding (which would
    /// silently start failing the moment any caller clones the `Arc` first).
    ///
    /// `scope` must be the process's own boot-time owner scope (composition
    /// derives it from `local_dev_nearai_mcp_owner_scope`), never a
    /// per-request, per-thread, or per-user scope — the fallback selector
    /// reuses it as the credential lookup target for every matching SSO
    /// caller. This rejects a mission/thread-scoped value as a fail-closed
    /// guard against an obviously wrong call site; it cannot prove the scope
    /// is *the host's* rather than some specific end user's, since an
    /// individual user's own owner-granularity scope has the identical
    /// shape (mission/thread both `None`). That stronger guarantee only
    /// exists by construction today: this builder must be called solely
    /// from boot-time product-auth composition in `factory.rs`, never from
    /// request-handling code.
    pub(crate) fn with_host_managed_nearai_credential_scope(
        mut self,
        scope: AuthProductScope,
    ) -> Result<Self, RebornBuildError> {
        if scope.resource.mission_id.is_some() || scope.resource.thread_id.is_some() {
            return Err(RebornBuildError::InvalidConfig {
                reason:
                    "host-managed NEAR AI credential scope must not carry mission/thread scoping"
                        .to_string(),
            });
        }
        self.host_managed_nearai_credential_scope = Some(scope);
        Ok(self)
    }

    /// Enable WebUI/local-dev auth-flow projection source.
    ///
    /// Exported `pub` so integration-test harnesses outside the crate can
    /// wire an in-memory fake. Not part of the stable product API — do not
    /// call this from production composition paths; use `as_auth_challenge_provider()`
    /// only when `product_auth` exposes a `flow_record_source` via the bundle.
    #[doc(hidden)]
    pub fn with_flow_record_source(mut self, source: Arc<dyn AuthFlowRecordSource>) -> Self {
        self.flow_record_source = Some(source);
        self
    }

    /// Expose this service as an `Arc<dyn AuthChallengeProvider>` so product
    /// surfaces can enrich `AuthPromptView` payloads with `challenge_kind`,
    /// `provider`, `account_label`, and `authorization_url`.
    ///
    /// Returns `None` when no `flow_record_source` is configured (meaning this
    /// bundle was built without the in-memory projection source, e.g. in
    /// production deployments that use durable DB backends not yet wired to
    /// `AuthFlowRecordSource`). Product auth prompts fall back to the plain
    /// 4-field view in that case, which is backward-compatible.
    #[doc(hidden)]
    pub fn as_auth_challenge_provider(self: &Arc<Self>) -> Option<Arc<dyn AuthChallengeProvider>> {
        self.has_flow_record_source()
            .then(|| Arc::clone(self) as Arc<dyn AuthChallengeProvider>)
    }

    /// Expose this service as an `Arc<dyn BlockedAuthFlowCanceller>` so the Slack
    /// delivery path can cancel the durable `AuthFlow` record alongside the run
    /// when it auto-denies a non-OAuth auth challenge (issue #4952).
    ///
    /// Returns `None` under the same condition as
    /// [`Self::as_auth_challenge_provider`] — both flow-backed facades gate on
    /// [`Self::has_flow_record_source`]. They stay separate accessors because they
    /// expose distinct capability ports (`AuthChallengeProvider` vs
    /// `BlockedAuthFlowCanceller`), but share the one wiring precondition.
    #[doc(hidden)]
    pub fn as_blocked_auth_flow_canceller(
        self: &Arc<Self>,
    ) -> Option<Arc<dyn BlockedAuthFlowCanceller>> {
        self.has_flow_record_source()
            .then(|| Arc::clone(self) as Arc<dyn BlockedAuthFlowCanceller>)
    }

    /// Shared precondition for the flow-backed facades: both
    /// [`Self::as_auth_challenge_provider`] and
    /// [`Self::as_blocked_auth_flow_canceller`] are only available when an
    /// `AuthFlowRecordSource` projection is wired in. Defined once so the gate
    /// cannot drift between the two accessors.
    fn has_flow_record_source(&self) -> bool {
        self.flow_record_source.is_some()
    }

    /// Refresh a credential account through the injected product-auth port.
    ///
    /// Concrete account services own the durable account update and provider
    /// egress wiring; callers enter here so WebUI/setup/lifecycle code does not
    /// reconstruct refresh authority locally.
    pub async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, RebornCredentialLifecycleError> {
        self.credential_account_service
            .refresh_account(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)
    }

    /// List redacted credential account projections through the injected
    /// account port.
    ///
    /// Routes/CLIs/extensions enter here so they never bypass the account
    /// port's grant filtering, status redaction, or extension-scoped
    /// visibility rules.
    pub async fn list_credential_accounts(
        &self,
        request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, RebornCredentialLifecycleError> {
        self.credential_account_service
            .list_accounts(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)
    }

    /// Select a single configured credential account through the injected
    /// account port.
    pub async fn select_credential_account(
        &self,
        request: CredentialAccountChoiceRequest,
    ) -> Result<CredentialAccountProjection, RebornCredentialLifecycleError> {
        self.credential_account_service
            .select_configured_account(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)
    }

    /// Project the stable credential recovery state for a provider through
    /// the injected account port. The projection drives WebUI/CLI/API
    /// recovery, refresh, and reauthorize prompts without exposing backend
    /// errors or secret handles.
    pub async fn project_credential_recovery(
        &self,
        request: CredentialRecoveryRequest,
    ) -> Result<CredentialRecoveryProjection, RebornCredentialLifecycleError> {
        self.credential_account_service
            .project_credential_recovery(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)
    }

    /// Apply ownership-aware credential cleanup for extension lifecycle events.
    ///
    /// This facade keeps lifecycle callers on the Reborn product-auth boundary
    /// instead of depending on V1 extension-manager cleanup or route-local
    /// secret authority.
    pub async fn cleanup_credentials_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, RebornCredentialLifecycleError> {
        let report = self
            .cleanup_service
            .cleanup_for_lifecycle(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)?;
        for event in &report.canceled_turn_gate_continuations {
            self.continuation_dispatcher
                .dispatch_canceled_auth_continuation(event.clone())
                .await
                .map_err(RebornCredentialLifecycleError::from)?;
            self.flow_manager
                .mark_continuation_dispatched(&event.scope, event.flow_id, event.emitted_at)
                .await
                .map_err(RebornCredentialLifecycleError::from)?;
        }
        Ok(report)
    }

    pub async fn handle_oauth_callback(
        &self,
        request: RebornOAuthCallbackRequest,
    ) -> Result<RebornOAuthCallbackResponse, RebornOAuthCallbackError> {
        self.handle_oauth_callback_with_optional_provider_identity_check(request, None)
            .await
            .map_err(RebornOAuthCallbackAttemptError::error)
    }

    pub(crate) async fn handle_oauth_callback_with_optional_provider_identity_check(
        &self,
        request: RebornOAuthCallbackRequest,
        mut provider_identity_check: Option<OAuthProviderIdentityCheck>,
    ) -> Result<RebornOAuthCallbackResponse, RebornOAuthCallbackAttemptError> {
        let mut provider_identity = None;
        let mut identity_binding_rollback: Option<OAuthProviderIdentityBindingRollback> = None;
        let (mut completed, should_dispatch_continuation) = match request.outcome {
            RebornOAuthCallbackOutcome::Authorized { provider_request } => {
                let claimed = self
                    .flow_manager
                    .claim_oauth_callback(
                        &request.scope,
                        OAuthCallbackClaimRequest {
                            flow_id: request.flow_id,
                            opaque_state_hash: request.opaque_state_hash.clone(),
                            provider: provider_request.provider.clone(),
                            pkce_verifier_hash: provider_request.pkce_verifier_hash.clone(),
                        },
                    )
                    .await
                    .map_err(RebornOAuthCallbackError::from)?;

                if claimed.status == AuthFlowStatus::Failed {
                    match self.reconcile_failed_lifecycle_oauth(&claimed).await {
                        Ok(_) => {
                            return Err(RebornOAuthCallbackAttemptError::continuation_side_effect(
                                AuthProductError::BackendUnavailable,
                            ));
                        }
                        Err(error) => {
                            return Err(
                                RebornOAuthCallbackAttemptError::continuation_compensation(error),
                            );
                        }
                    }
                }

                if matches!(
                    claimed.status,
                    AuthFlowStatus::Completed | AuthFlowStatus::Completing
                ) {
                    let should_dispatch = claimed.continuation_emitted_at.is_none();
                    (claimed, should_dispatch)
                } else {
                    let exchange = match self
                        .provider_client
                        .exchange_callback(
                            OAuthProviderExchangeContext {
                                scope: request.scope.clone(),
                                flow_id: request.flow_id,
                            },
                            provider_request,
                        )
                        .await
                    {
                        Ok(exchange) => exchange,
                        Err(error) => {
                            let error_code = error.code();
                            if let Err(fail_error) = self
                                .flow_manager
                                .fail_oauth_callback(
                                    &request.scope,
                                    OAuthCallbackFailureInput {
                                        flow_id: request.flow_id,
                                        opaque_state_hash: request.opaque_state_hash,
                                        error: error_code,
                                    },
                                )
                                .await
                            {
                                tracing::warn!(
                                    flow_id = %request.flow_id,
                                    exchange_error_code = ?error_code,
                                    fail_error_code = ?fail_error.code(),
                                    "reborn auth callback provider exchange failed and flow failure update failed"
                                );
                            }
                            return Err(error.into());
                        }
                    };
                    if let Some(check) = provider_identity_check.take() {
                        match check(exchange.provider_identity.clone()).await {
                            Ok(rollback) => identity_binding_rollback = rollback,
                            Err(error) => {
                                let error_code = error.code();
                                if let Err(cleanup_error) = self
                                    .provider_client
                                    .cleanup_exchange(
                                        OAuthProviderExchangeContext {
                                            scope: request.scope.clone(),
                                            flow_id: request.flow_id,
                                        },
                                        &exchange,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        flow_id = %request.flow_id,
                                        check_error_code = ?error_code,
                                        cleanup_error_code = ?cleanup_error.code(),
                                        "reborn auth callback provider identity check failed and token cleanup failed"
                                    );
                                    if let Err(retain_error) = self
                                        .cleanup_service
                                        .retain_oauth_exchange_for_cleanup(
                                            OAuthExchangeCleanupRequest {
                                                scope: request.scope.clone(),
                                                flow_id: request.flow_id,
                                                exchange: exchange.clone(),
                                            },
                                        )
                                        .await
                                    {
                                        tracing::warn!(
                                            flow_id = %request.flow_id,
                                            check_error_code = ?error_code,
                                            cleanup_error_code = ?cleanup_error.code(),
                                            retain_error_code = ?retain_error.code(),
                                            "failed to retain rejected OAuth exchange for lifecycle cleanup"
                                        );
                                    }
                                }
                                if let Err(fail_error) = self
                                    .flow_manager
                                    .fail_oauth_callback(
                                        &request.scope,
                                        OAuthCallbackFailureInput {
                                            flow_id: request.flow_id,
                                            opaque_state_hash: request.opaque_state_hash.clone(),
                                            error: error_code,
                                        },
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        flow_id = %request.flow_id,
                                        check_error_code = ?error_code,
                                        fail_error_code = ?fail_error.code(),
                                        "reborn auth callback provider identity check failed and flow failure update failed"
                                    );
                                }
                                return Err(error.into());
                            }
                        }
                    }
                    provider_identity = exchange.provider_identity.clone();
                    let exchange_for_cleanup = exchange.clone();
                    let completed = match self
                        .flow_manager
                        .complete_oauth_callback(
                            &request.scope,
                            OAuthCallbackInput {
                                flow_id: request.flow_id,
                                opaque_state_hash: request.opaque_state_hash.clone(),
                                outcome: ProviderCallbackOutcome::Authorized {
                                    exchange: Box::new(exchange),
                                },
                            },
                        )
                        .await
                    {
                        Ok(completed) => completed,
                        Err(error) => {
                            if let Err(cleanup_error) = self
                                .provider_client
                                .cleanup_exchange(
                                    OAuthProviderExchangeContext {
                                        scope: request.scope.clone(),
                                        flow_id: request.flow_id,
                                    },
                                    &exchange_for_cleanup,
                                )
                                .await
                            {
                                tracing::warn!(
                                    flow_id = %request.flow_id,
                                    completion_error_code = ?error.code(),
                                    cleanup_error_code = ?cleanup_error.code(),
                                    "reborn auth callback completion failed and token cleanup failed"
                                );
                                if let Err(retain_error) = self
                                    .cleanup_service
                                    .retain_oauth_exchange_for_cleanup(
                                        OAuthExchangeCleanupRequest {
                                            scope: request.scope.clone(),
                                            flow_id: request.flow_id,
                                            exchange: exchange_for_cleanup.clone(),
                                        },
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        flow_id = %request.flow_id,
                                        completion_error_code = ?error.code(),
                                        cleanup_error_code = ?cleanup_error.code(),
                                        retain_error_code = ?retain_error.code(),
                                        "failed to retain rejected OAuth exchange for lifecycle cleanup"
                                    );
                                }
                            }
                            // The identity hook committed durable state (the
                            // Slack binding is the user-visible "connected"
                            // signal) before this completion failure, and the
                            // completed-flow replay path never re-runs the
                            // hook — undo it so a failed completion cannot
                            // leave "connected with no usable credential".
                            if let Some(rollback) = identity_binding_rollback.take() {
                                rollback.await;
                            }
                            return Err(error.into());
                        }
                    };
                    (completed, true)
                }
            }
            RebornOAuthCallbackOutcome::ProviderDenied => self
                .flow_manager
                .complete_oauth_callback(
                    &request.scope,
                    OAuthCallbackInput {
                        flow_id: request.flow_id,
                        opaque_state_hash: request.opaque_state_hash,
                        outcome: ProviderCallbackOutcome::Denied,
                    },
                )
                .await
                .map(|completed| (completed, true))
                .map_err(RebornOAuthCallbackError::from)?,
            RebornOAuthCallbackOutcome::Malformed => {
                self.flow_manager
                    .fail_oauth_callback(
                        &request.scope,
                        OAuthCallbackFailureInput {
                            flow_id: request.flow_id,
                            opaque_state_hash: request.opaque_state_hash,
                            error: AuthErrorCode::MalformedCallback,
                        },
                    )
                    .await
                    .map_err(RebornOAuthCallbackError::from)?;
                return Err(AuthProductError::MalformedCallback.into());
            }
        };

        if should_dispatch_continuation {
            match self
                .dispatch_completed_continuation(completed.clone())
                .await
            {
                Ok(dispatched) => completed = dispatched,
                Err(ContinuationDispatchFailure::SideEffect { error, settled }) => {
                    if settled.status == AuthFlowStatus::Failed {
                        if let Err(cleanup_error) =
                            self.reconcile_failed_lifecycle_oauth(&settled).await
                        {
                            tracing::warn!(
                                flow_id = %settled.id,
                                error_code = ?cleanup_error.code(),
                                "extension activation failed and exact OAuth compensation remains pending"
                            );
                            return Err(
                                RebornOAuthCallbackAttemptError::continuation_compensation(
                                    cleanup_error,
                                ),
                            );
                        }
                        return Err(RebornOAuthCallbackAttemptError::continuation_side_effect(
                            error,
                        ));
                    }
                    return Err(RebornOAuthCallbackAttemptError::continuation_retryable(
                        error,
                    ));
                }
                Err(ContinuationDispatchFailure::Acknowledgement(error)) => {
                    return Err(
                        RebornOAuthCallbackAttemptError::continuation_acknowledgement(error),
                    );
                }
            }
        }

        Ok(RebornOAuthCallbackResponse {
            flow_id: completed.id,
            status: completed.status,
            credential_account_id: completed.credential_account_id,
            continuation: completed.continuation,
            provider_identity,
        })
    }

    async fn reconcile_failed_lifecycle_oauth(
        &self,
        flow: &AuthFlowRecord,
    ) -> Result<OAuthCompletionCompensationOutcome, AuthProductError> {
        if flow.status != AuthFlowStatus::Failed
            || !matches!(
                flow.continuation,
                AuthContinuationRef::LifecycleActivation { .. }
            )
        {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        let credential_account_id = flow
            .credential_account_id
            .ok_or(AuthProductError::BackendUnavailable)?;
        let expected_secret_fingerprint = flow
            .credential_secret_fingerprint
            .clone()
            .ok_or(AuthProductError::BackendUnavailable)?;
        let outcome = self
            .cleanup_service
            .compensate_oauth_completion(OAuthCompletionCompensationRequest {
                scope: flow.scope.clone(),
                flow_id: flow.id,
                provider: flow.provider.clone(),
                credential_account_id,
                expected_secret_fingerprint,
            })
            .await?;
        if outcome == OAuthCompletionCompensationOutcome::Superseded {
            tracing::debug!(
                flow_id = %flow.id,
                "skipped stale OAuth compensation because newer credential material exists"
            );
        }
        Ok(outcome)
    }

    #[allow(dead_code, reason = "used by upcoming Reborn OAuth setup route wiring")]
    pub(crate) async fn ensure_oauth_callback_flow_known(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthProviderId, RebornOAuthCallbackError> {
        let Some(record) = self
            .flow_manager
            .get_flow(scope, flow_id)
            .await
            .map_err(RebornOAuthCallbackError::from)?
        else {
            return Err(AuthProductError::UnknownOrExpiredFlow.into());
        };
        if record.expires_at <= Utc::now() {
            return Err(AuthProductError::UnknownOrExpiredFlow.into());
        }
        Ok(record.provider)
    }

    /// Load a caller-scoped flow for status reconciliation without applying the
    /// interactive callback expiry check. Failed terminal cleanup remains owed
    /// after `expires_at`, and status polling must be able to finish it without
    /// exposing cross-scope flow existence.
    #[allow(
        dead_code,
        reason = "used by the feature-scoped webui-v2-beta OAuth status route"
    )]
    pub(crate) async fn flow_record_for_status(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, RebornOAuthCallbackError> {
        match self.flow_manager.get_flow(scope, flow_id).await {
            Ok(Some(record)) => Ok(record),
            Ok(None) | Err(AuthProductError::CrossScopeDenied) => {
                Err(AuthProductError::UnknownOrExpiredFlow.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Reconcile a scoped OAuth flow's pending lifecycle work.
    ///
    /// Ownership is enforced by `get_flow`'s full-scope match: a flow owned by a
    /// different scope surfaces as `CrossScopeDenied`, which we deliberately
    /// remap to the same not-found signal as an unknown flow so the command
    /// cannot be used as a cross-user existence oracle. The command
    /// idempotently resumes a stale lifecycle dispatch or its exact
    /// compensation journal after a service restart. Observational reads use
    /// [`Self::flow_record_for_status`] instead.
    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta OAuth reconciliation command"
    )]
    pub(crate) async fn reconcile_oauth_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowStatus, RebornOAuthCallbackError> {
        match self.flow_manager.get_flow(scope, flow_id).await {
            Ok(Some(record)) => {
                if record.status == AuthFlowStatus::Failed
                    && record.credential_secret_fingerprint.is_some()
                {
                    self.reconcile_failed_lifecycle_oauth(&record)
                        .await
                        .map_err(RebornOAuthCallbackError::from)?;
                    return Ok(AuthFlowStatus::Failed);
                }
                if matches!(
                    record.status,
                    AuthFlowStatus::Completed | AuthFlowStatus::Completing
                ) && record.continuation_emitted_at.is_none()
                    && matches!(
                        record.continuation,
                        AuthContinuationRef::LifecycleActivation { .. }
                    )
                {
                    return match self.dispatch_completed_continuation(record).await {
                        Ok(dispatched) => Ok(dispatched.status),
                        Err(ContinuationDispatchFailure::SideEffect { settled, .. })
                            if settled.status == AuthFlowStatus::Failed =>
                        {
                            self.reconcile_failed_lifecycle_oauth(&settled)
                                .await
                                .map_err(RebornOAuthCallbackError::from)?;
                            Ok(AuthFlowStatus::Failed)
                        }
                        Err(error) => Err(error.into_auth_error().into()),
                    };
                }
                Ok(record.status)
            }
            Ok(None) => Err(AuthProductError::UnknownOrExpiredFlow.into()),
            // Never distinguish "owned by another scope" from "unknown": both
            // return not-found so a caller cannot probe another owner's flows.
            Err(AuthProductError::CrossScopeDenied) => {
                Err(AuthProductError::UnknownOrExpiredFlow.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta OAuth callback route when DCR fallback PKCE storage is enabled"
    )]
    pub(crate) async fn oauth_pkce_verifier_for_flow(
        &self,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
        flow_id: AuthFlowId,
    ) -> Result<Option<SecretString>, RebornOAuthCallbackError> {
        if let Some(registry) = &self.oauth_gate_registry
            && let Some(pkce) = registry
                .pkce_verifier_for_flow(scope, provider, flow_id)
                .await
                .map_err(RebornOAuthCallbackError::from)?
        {
            return Ok(Some(pkce));
        }
        let Some(registry) = &self.dcr_oauth_registry else {
            return Ok(None);
        };
        registry
            .pkce_verifier_for_flow(scope, provider, flow_id)
            .await
            .map_err(RebornOAuthCallbackError::from)
    }

    #[allow(dead_code, reason = "used by upcoming Reborn OAuth setup route wiring")]
    pub(crate) async fn start_setup_oauth_flow(
        &self,
        request: RebornOAuthStartFlowRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.flow_manager
            .create_flow(NewAuthFlow {
                id: request.flow_id,
                scope: request.scope,
                kind: AuthFlowKind::IntegrationCredential,
                provider: request.provider,
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: request.authorization_url,
                    expires_at: request.expires_at,
                },
                continuation: request.continuation,
                update_binding: request.update_binding,
                opaque_state_hash: Some(request.opaque_state_hash),
                pkce_verifier_hash: Some(request.pkce_verifier_hash),
                expires_at: request.expires_at,
            })
            .await
    }

    #[allow(
        dead_code,
        reason = "used by the webui-v2-beta extension OAuth route through product-auth route composition"
    )]
    pub(crate) async fn start_dcr_setup_oauth_flow(
        &self,
        request: RebornDcrOAuthStartFlowRequest,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(registry) = &self.dcr_oauth_registry else {
            return Ok(None);
        };
        registry
            .start_setup_flow(
                &self.flow_manager,
                DcrSetupFlowRequest {
                    scope: request.scope,
                    provider: request.provider,
                    account_label: request.account_label,
                    provider_scopes: request.provider_scopes,
                    continuation: request.continuation,
                    update_binding: request.update_binding,
                    expires_at: request.expires_at,
                },
            )
            .await
    }

    pub async fn request_manual_token_setup(
        &self,
        request: RebornManualTokenSetupRequest,
    ) -> Result<RebornManualTokenChallenge, RebornManualTokenError> {
        let challenge = self
            .manual_token_flow_service
            .request_manual_token_flow(ManualTokenSetupRequest {
                scope: request.scope,
                provider: request.provider,
                label: request.label,
                continuation: request.continuation,
                update_binding: request.update_binding,
                expires_at: request.expires_at,
            })
            .await
            .map_err(RebornManualTokenError::from)?;

        match challenge {
            ironclaw_auth::AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider,
                label,
                expires_at,
            } => Ok(RebornManualTokenChallenge {
                interaction_id,
                provider,
                label,
                expires_at,
            }),
            _ => Err(AuthProductError::InvalidRequest {
                reason: "manual token setup returned an unexpected challenge".to_string(),
            }
            .into()),
        }
    }

    pub async fn submit_manual_token(
        &self,
        request: RebornManualTokenSubmitRequest,
    ) -> Result<RebornManualTokenSubmitResponse, RebornManualTokenError> {
        let scope = request.scope;
        let interaction_id = request.interaction_id;
        let submit = self
            .manual_token_flow_service
            .submit_manual_token_flow(
                &scope,
                SecretSubmitRequest {
                    interaction_id,
                    secret: request.secret,
                },
            )
            .await;
        let (result, completed) = match submit {
            Ok(completed) => completed,
            Err(AuthProductError::UnknownOrExpiredFlow) => self
                .recover_completed_manual_token_submit(&scope, interaction_id)
                .await?
                .ok_or(AuthProductError::UnknownOrExpiredFlow)
                .map_err(RebornManualTokenError::from)?,
            Err(error) => return Err(RebornManualTokenError::from(error)),
        };
        self.dispatch_completed_continuation(completed)
            .await
            .map_err(ContinuationDispatchFailure::into_auth_error)
            .map_err(RebornManualTokenError::from)?;

        Ok(RebornManualTokenSubmitResponse {
            account_id: result.account_id,
            status: result.status,
            continuation: result.continuation,
        })
    }

    async fn recover_completed_manual_token_submit(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<Option<(SecretSubmitResult, AuthFlowRecord)>, RebornManualTokenError> {
        let Some(source) = &self.flow_record_source else {
            return Ok(None);
        };
        let Some(thread_id) = scope.resource.thread_id.clone() else {
            return Ok(None);
        };
        let flows = source
            .flows_for_owner(AuthFlowOwnerScope {
                tenant_id: scope.resource.tenant_id.clone(),
                user_id: scope.resource.user_id.clone(),
                agent_id: scope.resource.agent_id.clone(),
                project_id: scope.resource.project_id.clone(),
                thread_id,
            })
            .await
            .map_err(RebornManualTokenError::from)?;
        let Some(completed) = flows.into_iter().find(|flow| {
            flow.status == AuthFlowStatus::Completed
                && flow.continuation_emitted_at.is_none()
                && scope_matches(scope, &flow.scope)
                && matches!(
                    &flow.challenge,
                    Some(AuthChallenge::ManualTokenRequired { interaction_id: id, .. })
                        if id == &interaction_id
                )
        }) else {
            return Ok(None);
        };
        let Some(account_id) = completed.credential_account_id else {
            return Ok(None);
        };
        let account = self
            .credential_account_service
            .get_account(CredentialAccountLookupRequest::new(
                completed.scope.clone(),
                account_id,
            ))
            .await
            .map_err(RebornManualTokenError::from)?
            .ok_or(AuthProductError::CredentialMissing)
            .map_err(RebornManualTokenError::from)?;
        Ok(Some((
            SecretSubmitResult {
                account_id,
                status: account.status,
                continuation: completed.continuation.clone(),
            },
            completed,
        )))
    }

    pub async fn abandon_manual_token(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, RebornManualTokenError> {
        self.manual_token_flow_service
            .abandon_manual_token_flow(scope, interaction_id)
            .await
            .map_err(RebornManualTokenError::from)
    }

    async fn dispatch_completed_continuation(
        &self,
        completed: AuthFlowRecord,
    ) -> Result<AuthFlowRecord, ContinuationDispatchFailure> {
        let lifecycle = matches!(
            completed.continuation,
            AuthContinuationRef::LifecycleActivation { .. }
        );
        let completed = if lifecycle {
            self.flow_manager
                .get_flow(&completed.scope, completed.id)
                .await
                .map_err(ContinuationDispatchFailure::Acknowledgement)?
                .ok_or(ContinuationDispatchFailure::Acknowledgement(
                    AuthProductError::UnknownOrExpiredFlow,
                ))?
        } else {
            completed
        };
        if completed.continuation_emitted_at.is_some() {
            return Ok(completed);
        }
        let claimed = if lifecycle {
            self.flow_manager
                .claim_continuation_dispatch(
                    &completed.scope,
                    AuthContinuationDispatchClaimInput {
                        flow_id: completed.id,
                        claimed_at: Utc::now(),
                    },
                )
                .await
                .map_err(ContinuationDispatchFailure::Acknowledgement)?
        } else {
            completed
        };
        if claimed.continuation_emitted_at.is_some() {
            return Ok(claimed);
        }
        let emitted_at = Utc::now();
        let event = AuthContinuationEvent {
            flow_id: claimed.id,
            scope: claimed.scope.clone(),
            continuation: claimed.continuation.clone(),
            provider: claimed.provider.clone(),
            credential_account_id: claimed.credential_account_id,
            emitted_at,
        };
        if let Err(error) = self
            .continuation_dispatcher
            .dispatch_auth_continuation(event)
            .await
        {
            self.record_auth_continuation_dispatch_failure(&claimed);
            tracing::debug!(
                flow_id = %claimed.id,
                error_code = ?error.code(),
                "reborn auth flow completed but continuation dispatch failed"
            );
            let error = match error {
                AuthProductError::TokenExchangeFailed
                | AuthProductError::ProviderDenied
                | AuthProductError::MalformedCallback => AuthProductError::BackendUnavailable,
                error => error,
            };
            let settled = if lifecycle {
                self.settle_lifecycle_continuation(
                    &claimed,
                    AuthContinuationDispatchOutcome::TerminalFailure {
                        error: error.code(),
                    },
                )
                .await?
            } else {
                claimed
            };
            return Err(ContinuationDispatchFailure::SideEffect {
                error,
                settled: Box::new(settled),
            });
        }
        if lifecycle {
            self.settle_lifecycle_continuation(
                &claimed,
                AuthContinuationDispatchOutcome::Dispatched { emitted_at },
            )
            .await
        } else {
            self.flow_manager
                .mark_continuation_dispatched(&claimed.scope, claimed.id, emitted_at)
                .await
                .map_err(ContinuationDispatchFailure::Acknowledgement)
        }
    }

    async fn settle_lifecycle_continuation(
        &self,
        claimed: &AuthFlowRecord,
        outcome: AuthContinuationDispatchOutcome,
    ) -> Result<AuthFlowRecord, ContinuationDispatchFailure> {
        match self
            .flow_manager
            .settle_continuation_dispatch(
                &claimed.scope,
                AuthContinuationDispatchSettlementInput {
                    flow_id: claimed.id,
                    expected_claimed_at: claimed.updated_at,
                    outcome,
                },
            )
            .await
        {
            Ok(settled) => Ok(settled),
            Err(settlement_error) => {
                if self
                    .flow_manager
                    .settle_continuation_dispatch(
                        &claimed.scope,
                        AuthContinuationDispatchSettlementInput {
                            flow_id: claimed.id,
                            expected_claimed_at: claimed.updated_at,
                            outcome: AuthContinuationDispatchOutcome::RetryableFailure,
                        },
                    )
                    .await
                    .is_ok()
                {
                    return Err(ContinuationDispatchFailure::Acknowledgement(
                        settlement_error,
                    ));
                }
                // A stale claimant must never tear down the outcome owned by a
                // newer lease. Observe an already-acknowledged success;
                // otherwise report an acknowledgement-stage retry so provider
                // terminal hooks do not run from the losing callback.
                if let Ok(Some(current)) =
                    self.flow_manager.get_flow(&claimed.scope, claimed.id).await
                    && current.continuation_emitted_at.is_some()
                {
                    return Ok(current);
                }
                Err(ContinuationDispatchFailure::Acknowledgement(
                    settlement_error,
                ))
            }
        }
    }

    fn record_auth_continuation_dispatch_failure(&self, completed: &AuthFlowRecord) {
        if let Some(sink) = &self.security_audit_sink {
            sink.record(
                SecurityAuditEvent::new(
                    SecurityBoundary::AuthContinuation,
                    SecurityDecision::Blocked,
                    AUTH_CONTINUATION_DISPATCH_FAILED_CODE,
                )
                .with_scope(completed.scope.resource.clone()),
            );
        }
    }

    #[allow(
        dead_code,
        reason = "used by feature-scoped product-auth route tests that do not compile in every lib-test target"
    )]
    pub(crate) fn local_dev_in_memory(
        continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        let services = Arc::new(InMemoryAuthProductServices::new());
        RebornProductAuthServicePorts::from_shared(services.clone())
            .into_services(
                continuation_dispatcher,
                Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            )
            .with_flow_record_source(services)
    }
}

#[async_trait]
impl RuntimeCredentialAccountRefreshPort for RebornProductAuthServices {
    async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        RebornProductAuthServices::refresh_credential_account(self, request)
            .await
            .map_err(auth_product_error_from_reborn_error)
    }
}

fn auth_product_error_from_reborn_error(error: RebornAuthProductError) -> AuthProductError {
    match error.code {
        AuthErrorCode::UnknownOrExpiredFlow => AuthProductError::UnknownOrExpiredFlow,
        AuthErrorCode::CrossScopeDenied => AuthProductError::CrossScopeDenied,
        AuthErrorCode::ProviderDenied => AuthProductError::ProviderDenied,
        AuthErrorCode::TokenExchangeFailed => AuthProductError::TokenExchangeFailed,
        AuthErrorCode::RefreshFailed => AuthProductError::RefreshFailed,
        AuthErrorCode::CredentialMissing => AuthProductError::CredentialMissing,
        AuthErrorCode::AccountSelectionRequired => AuthProductError::AccountSelectionRequired,
        AuthErrorCode::BackendUnavailable => AuthProductError::BackendUnavailable,
        AuthErrorCode::ProviderIdentityAlreadyConnected => {
            AuthProductError::ProviderIdentityAlreadyConnected
        }
        AuthErrorCode::ConnectionConflict => AuthProductError::BackendConflict,
        AuthErrorCode::MalformedConfig => AuthProductError::MalformedConfig,
        AuthErrorCode::MalformedCallback => AuthProductError::MalformedCallback,
        AuthErrorCode::Canceled => AuthProductError::Canceled,
        AuthErrorCode::FlowAlreadyTerminal => AuthProductError::FlowAlreadyTerminal,
        AuthErrorCode::InvalidRequest => AuthProductError::InvalidRequest {
            reason: "runtime credential refresh request rejected".to_string(),
        },
    }
}

fn auth_challenge_to_view(
    challenge: &ironclaw_auth::AuthChallenge,
    flow: &ironclaw_auth::AuthFlowRecord,
) -> AuthChallengeView {
    match challenge {
        ironclaw_auth::AuthChallenge::OAuthUrl {
            authorization_url,
            expires_at,
        } => AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: flow.provider.clone(),
            account_label: None,
            authorization_url: Some(authorization_url.clone()),
            expires_at: Some(*expires_at),
        },
        ironclaw_auth::AuthChallenge::ManualTokenRequired {
            provider,
            label,
            expires_at,
            ..
        } => AuthChallengeView {
            kind: AuthPromptChallengeKind::ManualToken,
            provider: provider.clone(),
            account_label: Some(label.clone()),
            authorization_url: None,
            expires_at: Some(*expires_at),
        },
        ironclaw_auth::AuthChallenge::AccountSelectionRequired { .. }
        | ironclaw_auth::AuthChallenge::ReauthorizeRequired { .. }
        | ironclaw_auth::AuthChallenge::SetupRequired { .. } => AuthChallengeView {
            kind: AuthPromptChallengeKind::Other,
            provider: flow.provider.clone(),
            account_label: None,
            authorization_url: None,
            expires_at: None,
        },
    }
}

#[async_trait]
impl AuthChallengeProvider for RebornProductAuthServices {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let gate_ref = AuthGateRef::new(gate_ref.to_string())
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        let Some(source) = self.flow_record_source.as_ref() else {
            return Ok(None);
        };
        if let Some(registry) = &self.oauth_gate_registry
            && let Some(view) = registry
                .challenge_for_blocked_gate(OAuthGateChallengeRequest {
                    flow_manager: &self.flow_manager,
                    flow_source: source,
                    requirements: credential_requirements,
                    scope,
                    owner_user_id,
                    run_id,
                    gate_ref: &gate_ref,
                })
                .await?
        {
            return Ok(Some(view));
        }
        if let Some(registry) = &self.dcr_oauth_registry
            && let Some(view) = registry
                .challenge_for_blocked_gate(DcrGateChallengeRequest {
                    flow_manager: &self.flow_manager,
                    flow_source: source,
                    requirements: credential_requirements,
                    scope,
                    owner_user_id,
                    run_id,
                    gate_ref: &gate_ref,
                })
                .await?
        {
            return Ok(Some(view));
        }
        // The flow source may include records from multiple product surfaces;
        // query by stable owner and gate continuation before exposing metadata.
        let flow = source
            .flow_for_turn_gate(TurnGateAuthFlowQuery {
                owner: AuthFlowOwnerScope {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: owner_user_id.clone(),
                    agent_id: scope.agent_id.clone(),
                    project_id: scope.project_id.clone(),
                    thread_id: scope.thread_id.clone(),
                },
                turn_run_ref: TurnRunRef::new(run_id.to_string())
                    .map_err(|_| AuthProductError::BackendUnavailable)?,
                gate_ref,
                include_terminal: false,
            })
            .await?;
        let Some(flow) = flow else {
            return Ok(None);
        };
        let Some(challenge) = flow.challenge.as_ref() else {
            return Ok(None);
        };
        Ok(Some(auth_challenge_to_view(challenge, &flow)))
    }
}

#[async_trait]
impl BlockedAuthFlowCanceller for RebornProductAuthServices {
    async fn cancel_blocked_auth_flow(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), AuthProductError> {
        let gate_ref = AuthGateRef::new(gate_ref.to_string()).map_err(|err| {
            AuthProductError::InvalidRequest {
                reason: format!("invalid gate ref for auth-flow cancel: {err}"),
            }
        })?;
        let Some(source) = self.flow_record_source.as_ref() else {
            // No projection source wired in: nothing to cancel here.
            return Ok(());
        };
        // `include_terminal: false` means an already-terminal flow (or a missing
        // one) resolves to `None`, so the OAuth-callback race — where the flow
        // completes just before auto-deny — is a graceful no-op rather than an
        // error. We only ever cancel a flow that is still non-terminal.
        let flow = source
            .flow_for_turn_gate(TurnGateAuthFlowQuery {
                owner: AuthFlowOwnerScope {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: owner_user_id.clone(),
                    agent_id: scope.agent_id.clone(),
                    project_id: scope.project_id.clone(),
                    thread_id: scope.thread_id.clone(),
                },
                turn_run_ref: TurnRunRef::new(run_id.to_string()).map_err(|err| {
                    AuthProductError::InvalidRequest {
                        reason: format!("invalid turn run ref for auth-flow cancel: {err}"),
                    }
                })?,
                gate_ref,
                include_terminal: false,
            })
            .await?;
        let Some(flow) = flow else {
            return Ok(());
        };
        match self.flow_manager.cancel_flow(&flow.scope, flow.id).await {
            Ok(_) => Ok(()),
            // The flow terminalized between our non-terminal read above and this
            // cancel (a concurrent OAuth callback or another canceller). Already
            // terminal is the desired end state, so honor the documented graceful
            // no-op contract instead of surfacing the race as an error. Real
            // lookup/scope/backend errors still propagate.
            Err(AuthProductError::Canceled | AuthProductError::FlowAlreadyTerminal) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_auth::{
        AuthChallenge, AuthFlowId, AuthFlowRecord, AuthProductError, AuthProductScope,
        CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountId,
        CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
        CredentialAccountMutation, CredentialAccountProjection, CredentialAccountSelectionRequest,
        CredentialAccountStatus, CredentialRecoveryProjection, CredentialRecoveryRequest,
        CredentialRefreshReport, CredentialRefreshRequest, NewAuthFlow, NewCredentialAccount,
        OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput,
        OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderRefresh,
        OAuthProviderRefreshRequest, SecretCleanupReport, SecretCleanupRequest,
        SecretSubmitRequest, SecretSubmitResult,
    };

    struct SharedAuthTestDouble;

    struct RejectLifecycleContinuation;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RejectLifecycleContinuation {
        async fn dispatch_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }

        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    fn arc_data_ptr<T: ?Sized>(arc: &Arc<T>) -> *const () {
        Arc::as_ptr(arc) as *const ()
    }

    #[tokio::test]
    async fn malformed_oauth_callback_terminalizes_the_known_flow() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let scope = test_auth_product_scope();
        let state_hash = OpaqueStateHash::new("a".repeat(64)).expect("state hash");
        let expires_at = Utc::now() + chrono::Duration::minutes(5);
        let flow = shared
            .create_flow(NewAuthFlow {
                id: None,
                scope: scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("google").expect("provider"),
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://provider.example/authorize",
                    )
                    .expect("authorization URL"),
                    expires_at,
                },
                continuation: AuthContinuationRef::SetupOnly,
                update_binding: None,
                opaque_state_hash: Some(state_hash.clone()),
                pkce_verifier_hash: None,
                expires_at,
            })
            .await
            .expect("create flow");
        let services = RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthContinuationDispatcher),
        );

        let error = services
            .handle_oauth_callback(RebornOAuthCallbackRequest {
                scope: scope.clone(),
                flow_id: flow.id,
                opaque_state_hash: state_hash,
                outcome: RebornOAuthCallbackOutcome::Malformed,
            })
            .await
            .expect_err("malformed callback remains a route failure");

        assert_eq!(error.code, AuthErrorCode::MalformedCallback);
        let persisted = shared
            .get_flow(&scope, flow.id)
            .await
            .expect("load flow")
            .expect("flow exists");
        assert_eq!(persisted.status, AuthFlowStatus::Failed);
        assert_eq!(persisted.error, Some(AuthErrorCode::MalformedCallback));
    }

    #[tokio::test]
    async fn abandoned_lifecycle_lease_can_terminalize_and_compensate() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let scope = test_auth_product_scope();
        let provider = AuthProviderId::new("slack_personal").expect("provider");
        let account = shared
            .create_account(NewCredentialAccount {
                scope: scope.clone(),
                provider: provider.clone(),
                label: CredentialAccountLabel::new("personal slack").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::SecretHandle::new("stale-access").expect("secret"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .expect("create account");
        let state_hash = OpaqueStateHash::new("a".repeat(64)).expect("state hash");
        let pkce_hash = PkceVerifierHash::new("b".repeat(64)).expect("PKCE hash");
        let flow = shared
            .create_flow(NewAuthFlow {
                id: None,
                scope: scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider.clone(),
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                        .expect("authorization URL"),
                    expires_at: Utc::now() + chrono::Duration::minutes(5),
                },
                continuation: AuthContinuationRef::LifecycleActivation {
                    package_ref: ironclaw_auth::LifecyclePackageRef::new("slack")
                        .expect("package ref"),
                },
                update_binding: Some(CredentialAccountUpdateBinding::from_projection(
                    &account.projection(),
                )),
                opaque_state_hash: Some(state_hash.clone()),
                pkce_verifier_hash: Some(pkce_hash.clone()),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            })
            .await
            .expect("create flow");
        shared
            .claim_oauth_callback(
                &scope,
                OAuthCallbackClaimRequest {
                    flow_id: flow.id,
                    opaque_state_hash: state_hash.clone(),
                    provider: provider.clone(),
                    pkce_verifier_hash: pkce_hash.clone(),
                },
            )
            .await
            .expect("claim callback");
        let completed = shared
            .complete_oauth_callback(
                &scope,
                OAuthCallbackInput {
                    flow_id: flow.id,
                    opaque_state_hash: state_hash,
                    outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                        exchange: Box::new(OAuthProviderExchange {
                            provider: provider.clone(),
                            account_label: CredentialAccountLabel::new("personal slack")
                                .expect("label"),
                            authorization_code_hash: ironclaw_auth::AuthorizationCodeHash::new(
                                "c".repeat(64),
                            )
                            .expect("authorization code hash"),
                            pkce_verifier_hash: pkce_hash,
                            access_secret: ironclaw_host_api::SecretHandle::new("completed-access")
                                .expect("secret"),
                            refresh_secret: None,
                            scopes: Vec::new(),
                            account_id: Some(account.id),
                            provider_identity: None,
                        }),
                    },
                },
            )
            .await
            .expect("complete callback");
        shared
            .claim_continuation_dispatch(
                &scope,
                AuthContinuationDispatchClaimInput {
                    flow_id: completed.id,
                    claimed_at: Utc::now()
                        - chrono::Duration::seconds(
                            ironclaw_auth::AUTH_CONTINUATION_DISPATCH_LEASE_SECONDS + 1,
                        ),
                },
            )
            .await
            .expect("seed abandoned lease");
        let services = RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(RejectLifecycleContinuation),
        )
        .with_flow_record_source(shared.clone());

        assert_eq!(
            services
                .reconcile_oauth_flow(&scope, completed.id)
                .await
                .expect("stale lease recovery"),
            AuthFlowStatus::Failed
        );
        let failed = shared
            .get_flow(&scope, completed.id)
            .await
            .expect("flow lookup")
            .expect("failed flow");
        assert_eq!(failed.status, AuthFlowStatus::Failed);
        let compensated = shared
            .get_account(CredentialAccountLookupRequest::new(scope, account.id))
            .await
            .expect("account lookup")
            .expect("compensated account");
        assert_eq!(compensated.status, CredentialAccountStatus::Revoked);
        assert!(compensated.access_secret.is_none());
    }

    #[test]
    fn reborn_product_auth_services_new_accepts_separate_impls() {
        let flow_manager: Arc<dyn AuthFlowManager> = Arc::new(SharedAuthTestDouble);
        let interaction_service: Arc<dyn AuthInteractionService> = Arc::new(SharedAuthTestDouble);
        let credential_setup_service: Arc<dyn CredentialSetupService> =
            Arc::new(SharedAuthTestDouble);
        let credential_account_service: Arc<dyn CredentialAccountService> =
            Arc::new(SharedAuthTestDouble);
        let provider_client: Arc<dyn AuthProviderClient> = Arc::new(SharedAuthTestDouble);
        let cleanup_service: Arc<dyn SecretCleanupService> = Arc::new(SharedAuthTestDouble);

        let services = RebornProductAuthServices::new(
            flow_manager.clone(),
            interaction_service.clone(),
            credential_setup_service.clone(),
            credential_account_service.clone(),
            provider_client.clone(),
            cleanup_service.clone(),
            Arc::new(NoopAuthContinuationDispatcher),
        );

        assert_eq!(
            arc_data_ptr(&services.flow_manager()),
            arc_data_ptr(&flow_manager)
        );
        assert_eq!(
            arc_data_ptr(&services.interaction_service()),
            arc_data_ptr(&interaction_service)
        );
        assert_eq!(
            arc_data_ptr(&services.credential_setup_service()),
            arc_data_ptr(&credential_setup_service)
        );
        assert_eq!(
            arc_data_ptr(&services.credential_account_service()),
            arc_data_ptr(&credential_account_service)
        );
        assert_eq!(
            arc_data_ptr(&services.provider_client()),
            arc_data_ptr(&provider_client)
        );
        assert_eq!(
            arc_data_ptr(&services.cleanup_service()),
            arc_data_ptr(&cleanup_service)
        );
    }

    #[test]
    fn with_host_managed_nearai_credential_scope_rejects_thread_scoped_value() {
        let shared = Arc::new(SharedAuthTestDouble);
        let services = RebornProductAuthServices::from_shared(
            shared,
            Arc::new(NoopAuthContinuationDispatcher),
        );

        let error = services
            .with_host_managed_nearai_credential_scope(test_auth_product_scope())
            .expect_err("thread-scoped value must not be accepted as a host-managed scope");

        assert!(matches!(error, RebornBuildError::InvalidConfig { .. }));
    }

    #[test]
    fn with_host_managed_nearai_credential_scope_accepts_owner_granularity_scope() {
        use ironclaw_auth::AuthSurface;
        use ironclaw_host_api::{AgentId, ResourceScope, TenantId, UserId};

        let shared = Arc::new(SharedAuthTestDouble);
        let services = RebornProductAuthServices::from_shared(
            shared,
            Arc::new(NoopAuthContinuationDispatcher),
        );
        let owner_scope = AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("host-tenant").expect("tenant"),
                user_id: UserId::new("host-owner").expect("user"),
                agent_id: Some(AgentId::new("host-agent").expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: ironclaw_host_api::InvocationId::new(),
            },
            AuthSurface::Api,
        );

        assert!(
            services
                .with_host_managed_nearai_credential_scope(owner_scope)
                .is_ok()
        );
    }

    #[test]
    fn reborn_product_auth_services_from_shared_clones_arc_per_trait() {
        let shared = Arc::new(SharedAuthTestDouble);
        let shared_ptr = arc_data_ptr(&shared);

        let services = RebornProductAuthServices::from_shared(
            shared,
            Arc::new(NoopAuthContinuationDispatcher),
        );

        assert_eq!(arc_data_ptr(&services.flow_manager()), shared_ptr);
        assert_eq!(arc_data_ptr(&services.interaction_service()), shared_ptr);
        assert_eq!(
            arc_data_ptr(&services.credential_setup_service()),
            shared_ptr
        );
        assert_eq!(
            arc_data_ptr(&services.credential_account_service()),
            shared_ptr
        );
        assert_eq!(arc_data_ptr(&services.provider_client()), shared_ptr);
        assert_eq!(arc_data_ptr(&services.cleanup_service()), shared_ptr);
    }

    #[tokio::test]
    async fn oauth_completion_failure_retains_exchange_when_provider_cleanup_fails() {
        use ironclaw_auth::{AuthorizationCodeHash, OAuthAuthorizationCode, PkceVerifierSecret};
        use ironclaw_host_api::SecretHandle;

        struct CancelOnExchangeProviderClient {
            flows: Arc<InMemoryAuthProductServices>,
        }

        #[async_trait::async_trait]
        impl AuthProviderClient for CancelOnExchangeProviderClient {
            async fn exchange_callback(
                &self,
                context: OAuthProviderExchangeContext,
                request: OAuthProviderCallbackRequest,
            ) -> Result<OAuthProviderExchange, AuthProductError> {
                self.flows
                    .cancel_flow(&context.scope, context.flow_id)
                    .await?;
                Ok(OAuthProviderExchange {
                    provider: request.provider,
                    account_label: request.account_label,
                    authorization_code_hash: request.authorization_code_hash,
                    pkce_verifier_hash: request.pkce_verifier_hash,
                    access_secret: SecretHandle::new("completion-failed-access")
                        .expect("access handle"),
                    refresh_secret: Some(
                        SecretHandle::new("completion-failed-refresh").expect("refresh handle"),
                    ),
                    scopes: request.scopes,
                    account_id: None,
                    provider_identity: None,
                })
            }

            async fn refresh_token(
                &self,
                _request: OAuthProviderRefreshRequest,
            ) -> Result<OAuthProviderRefresh, AuthProductError> {
                unreachable!("completion cleanup test does not refresh tokens")
            }

            async fn cleanup_exchange(
                &self,
                _context: OAuthProviderExchangeContext,
                _exchange: &OAuthProviderExchange,
            ) -> Result<(), AuthProductError> {
                Err(AuthProductError::BackendUnavailable)
            }
        }

        fn digest(byte: &str) -> String {
            byte.repeat(64)
        }

        let shared = Arc::new(InMemoryAuthProductServices::new());
        let scope = test_auth_product_scope();
        let provider = AuthProviderId::new("completion-failure-provider").expect("provider");
        let opaque_state_hash = OpaqueStateHash::new(digest("a")).expect("state hash");
        let pkce_verifier_hash = PkceVerifierHash::new(digest("b")).expect("pkce hash");
        let expires_at = Utc::now() + chrono::Duration::minutes(5);
        let flow = shared
            .create_flow(NewAuthFlow {
                id: None,
                scope: scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider.clone(),
                challenge: AuthChallenge::OAuthUrl {
                    authorization_url: OAuthAuthorizationUrl::new(
                        "https://provider.example/authorize",
                    )
                    .expect("authorization URL"),
                    expires_at,
                },
                continuation: AuthContinuationRef::SetupOnly,
                update_binding: None,
                opaque_state_hash: Some(opaque_state_hash.clone()),
                pkce_verifier_hash: Some(pkce_verifier_hash.clone()),
                expires_at,
            })
            .await
            .expect("create OAuth flow");
        let services = RebornProductAuthServices::new(
            shared.clone(),
            shared.clone(),
            shared.clone(),
            shared.clone(),
            Arc::new(CancelOnExchangeProviderClient {
                flows: shared.clone(),
            }),
            shared.clone(),
            Arc::new(NoopAuthContinuationDispatcher),
        );

        let error = services
            .handle_oauth_callback(RebornOAuthCallbackRequest {
                scope: scope.clone(),
                flow_id: flow.id,
                opaque_state_hash,
                outcome: RebornOAuthCallbackOutcome::Authorized {
                    provider_request: OAuthProviderCallbackRequest {
                        provider,
                        account_label: CredentialAccountLabel::new("completion failure")
                            .expect("account label"),
                        authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                            "authorization-code",
                        ))
                        .expect("authorization code"),
                        authorization_code_hash: AuthorizationCodeHash::new(digest("c"))
                            .expect("authorization code hash"),
                        pkce_verifier: PkceVerifierSecret::new(SecretString::from("pkce-verifier"))
                            .expect("PKCE verifier"),
                        pkce_verifier_hash,
                        scopes: Vec::new(),
                    },
                },
            })
            .await
            .expect_err("canceled completion must fail");

        assert_eq!(error.code, AuthErrorCode::Canceled);
        let cleanup_account_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
        let retained = shared
            .accounts_for_owner(&scope)
            .await
            .expect("list retained cleanup accounts")
            .into_iter()
            .find(|account| account.id == cleanup_account_id)
            .expect("failed cleanup must retain the exchanged handles");
        assert_eq!(retained.status, CredentialAccountStatus::Revoked);
        assert_eq!(
            retained.access_secret.as_ref().map(SecretHandle::as_str),
            Some("completion-failed-access")
        );
        assert_eq!(
            retained.refresh_secret.as_ref().map(SecretHandle::as_str),
            Some("completion-failed-refresh")
        );
    }

    #[test]
    fn reborn_product_auth_ports_from_shared_with_provider_uses_separate_provider_client() {
        let shared = Arc::new(SharedAuthTestDouble);
        let provider_client: Arc<dyn AuthProviderClient> = Arc::new(SharedAuthTestDouble);
        let shared_ptr = arc_data_ptr(&shared);
        let provider_ptr = arc_data_ptr(&provider_client);

        let ports =
            RebornProductAuthServicePorts::from_shared_with_provider(shared, provider_client);
        let services = ports.into_services(
            Arc::new(NoopAuthContinuationDispatcher),
            Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
        );

        assert_eq!(arc_data_ptr(&services.flow_manager()), shared_ptr);
        assert_eq!(arc_data_ptr(&services.interaction_service()), shared_ptr);
        assert_eq!(
            arc_data_ptr(&services.credential_setup_service()),
            shared_ptr
        );
        assert_eq!(
            arc_data_ptr(&services.credential_account_service()),
            shared_ptr
        );
        assert_eq!(arc_data_ptr(&services.provider_client()), provider_ptr);
        assert_eq!(arc_data_ptr(&services.cleanup_service()), shared_ptr);
    }

    #[async_trait::async_trait]
    impl AuthFlowManager for SharedAuthTestDouble {
        async fn create_flow(
            &self,
            _request: NewAuthFlow,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn get_flow(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
        ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn claim_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _request: OAuthCallbackClaimRequest,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn complete_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _input: OAuthCallbackInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn complete_credential_selection(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::CredentialSelectionInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn complete_manual_token(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::ManualTokenCompletionInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn cancel_manual_token(
            &self,
            _scope: &AuthProductScope,
            _interaction_id: AuthInteractionId,
        ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn fail_oauth_callback(
            &self,
            _scope: &AuthProductScope,
            _input: OAuthCallbackFailureInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn claim_continuation_dispatch(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::AuthContinuationDispatchClaimInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn settle_continuation_dispatch(
            &self,
            _scope: &AuthProductScope,
            _input: ironclaw_auth::AuthContinuationDispatchSettlementInput,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn cancel_flow(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }

        async fn mark_continuation_dispatched(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
            _emitted_at: ironclaw_auth::Timestamp,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("constructor tests do not call auth-flow methods")
        }
    }

    #[async_trait::async_trait]
    impl AuthInteractionService for SharedAuthTestDouble {
        async fn request_secret_input(
            &self,
            _request: ironclaw_auth::ManualTokenSetupRequest,
        ) -> Result<AuthChallenge, AuthProductError> {
            unreachable!("constructor tests do not call auth-interaction methods")
        }

        async fn submit_manual_token(
            &self,
            _scope: &AuthProductScope,
            _request: SecretSubmitRequest,
        ) -> Result<SecretSubmitResult, AuthProductError> {
            unreachable!("constructor tests do not call auth-interaction methods")
        }

        async fn abandon_manual_token(
            &self,
            _scope: &AuthProductScope,
            _interaction_id: AuthInteractionId,
        ) -> Result<bool, AuthProductError> {
            unreachable!("constructor tests do not call auth-interaction methods")
        }
    }

    #[async_trait::async_trait]
    impl CredentialSetupService for SharedAuthTestDouble {
        async fn create_or_update_account(
            &self,
            _request: CredentialAccountMutation,
        ) -> Result<CredentialAccount, AuthProductError> {
            unreachable!("constructor tests do not call credential-setup methods")
        }
    }

    #[async_trait::async_trait]
    impl CredentialAccountService for SharedAuthTestDouble {
        async fn create_account(
            &self,
            _request: NewCredentialAccount,
        ) -> Result<CredentialAccount, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn get_account(
            &self,
            _request: CredentialAccountLookupRequest,
        ) -> Result<Option<CredentialAccount>, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn list_accounts(
            &self,
            _request: CredentialAccountListRequest,
        ) -> Result<CredentialAccountListPage, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn update_status(
            &self,
            _scope: &AuthProductScope,
            _account_id: CredentialAccountId,
            _status: CredentialAccountStatus,
        ) -> Result<CredentialAccount, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn select_unique_configured_account(
            &self,
            _request: CredentialAccountSelectionRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn project_credential_recovery(
            &self,
            _request: CredentialRecoveryRequest,
        ) -> Result<CredentialRecoveryProjection, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn select_configured_account(
            &self,
            _request: CredentialAccountChoiceRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }

        async fn refresh_account(
            &self,
            _request: CredentialRefreshRequest,
        ) -> Result<CredentialRefreshReport, AuthProductError> {
            unreachable!("constructor tests do not call credential-account methods")
        }
    }

    #[async_trait::async_trait]
    impl CredentialAccountRecordSource for SharedAuthTestDouble {
        async fn accounts_for_owner(
            &self,
            _scope: &AuthProductScope,
        ) -> Result<Vec<CredentialAccount>, AuthProductError> {
            unreachable!("constructor tests do not call credential-account read-model methods")
        }
    }

    #[async_trait::async_trait]
    impl RebornManualTokenFlowService for SharedAuthTestDouble {
        async fn request_manual_token_flow(
            &self,
            _request: ironclaw_auth::ManualTokenSetupRequest,
        ) -> Result<AuthChallenge, AuthProductError> {
            unreachable!("constructor tests do not call manual-token flow methods")
        }

        async fn submit_manual_token_flow(
            &self,
            _scope: &AuthProductScope,
            _request: SecretSubmitRequest,
        ) -> Result<(SecretSubmitResult, AuthFlowRecord), AuthProductError> {
            unreachable!("constructor tests do not call manual-token flow methods")
        }

        async fn abandon_manual_token_flow(
            &self,
            _scope: &AuthProductScope,
            _interaction_id: AuthInteractionId,
        ) -> Result<bool, AuthProductError> {
            unreachable!("constructor tests do not call manual-token flow methods")
        }
    }

    #[async_trait::async_trait]
    impl AuthProviderClient for SharedAuthTestDouble {
        async fn exchange_callback(
            &self,
            _context: OAuthProviderExchangeContext,
            _request: OAuthProviderCallbackRequest,
        ) -> Result<OAuthProviderExchange, AuthProductError> {
            unreachable!("constructor tests do not call provider-client methods")
        }

        async fn refresh_token(
            &self,
            _request: OAuthProviderRefreshRequest,
        ) -> Result<OAuthProviderRefresh, AuthProductError> {
            unreachable!("constructor tests do not call provider-client methods")
        }
    }

    #[async_trait::async_trait]
    impl SecretCleanupService for SharedAuthTestDouble {
        async fn retain_oauth_exchange_for_cleanup(
            &self,
            _request: OAuthExchangeCleanupRequest,
        ) -> Result<CredentialAccountId, AuthProductError> {
            unreachable!("constructor tests do not call cleanup methods")
        }

        async fn compensate_oauth_completion(
            &self,
            _request: ironclaw_auth::OAuthCompletionCompensationRequest,
        ) -> Result<ironclaw_auth::OAuthCompletionCompensationOutcome, AuthProductError> {
            unreachable!("constructor tests do not call cleanup methods")
        }

        async fn cleanup_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, AuthProductError> {
            unreachable!("constructor tests do not call cleanup methods")
        }
    }

    // ── cancel_blocked_auth_flow facade tests ─────────────────────────────────

    /// Build a minimal `RebornProductAuthServices` for `cancel_blocked_auth_flow`
    /// tests.  The `flow_manager` is backed by `InMemoryAuthProductServices` so
    /// callers can inspect whether `cancel_flow` was actually invoked (by checking
    /// the flow's status after the call).  All other ports use `SharedAuthTestDouble`
    /// (they are never called by `cancel_blocked_auth_flow`).
    fn make_auth_services_with_flow_source(
        auth_svc: Arc<InMemoryAuthProductServices>,
    ) -> RebornProductAuthServices {
        let double = Arc::new(SharedAuthTestDouble);
        RebornProductAuthServices::new(
            auth_svc.clone() as Arc<dyn AuthFlowManager>,
            double.clone() as Arc<dyn AuthInteractionService>,
            double.clone() as Arc<dyn CredentialSetupService>,
            double.clone() as Arc<dyn CredentialAccountService>,
            double.clone() as Arc<dyn AuthProviderClient>,
            double.clone() as Arc<dyn SecretCleanupService>,
            Arc::new(NoopAuthContinuationDispatcher),
        )
        .with_flow_record_source(auth_svc as Arc<dyn AuthFlowRecordSource>)
    }

    /// Build an `AuthProductScope` that is consistent with a `personal_turn_scope`-like
    /// `TurnScope` used by `cancel_blocked_auth_flow`.
    fn test_auth_product_scope() -> AuthProductScope {
        use ironclaw_auth::AuthSurface;
        use ironclaw_host_api::{AgentId, ResourceScope, TenantId, ThreadId, UserId};

        let resource = ResourceScope {
            tenant_id: TenantId::new("test-tenant").expect("tenant"),
            user_id: UserId::new("creator-user").expect("user"),
            agent_id: Some(AgentId::new("test-agent").expect("agent")),
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("test-thread").expect("thread")),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        AuthProductScope::new(resource, AuthSurface::Chat)
    }

    /// Build a minimal non-terminal `AuthFlowRecord` whose continuation matches
    /// a `TurnGateAuthFlowQuery` for `run_id` / `gate_ref`.
    async fn create_test_flow(
        auth_svc: &InMemoryAuthProductServices,
        scope: AuthProductScope,
        run_id: TurnRunId,
        gate_ref_str: &str,
    ) -> AuthFlowRecord {
        let gate_ref = AuthGateRef::new(gate_ref_str.to_string()).expect("gate ref");
        let turn_run_ref = TurnRunRef::new(run_id.to_string()).expect("turn run ref");
        auth_svc
            .create_flow(NewAuthFlow {
                id: None,
                scope,
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("test-provider").expect("provider"),
                challenge: AuthChallenge::SetupRequired {
                    provider: AuthProviderId::new("test-provider").expect("provider"),
                    message: "test".to_string(),
                },
                continuation: AuthContinuationRef::TurnGateResume {
                    turn_run_ref,
                    gate_ref,
                },
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            })
            .await
            .expect("create test flow")
    }

    /// `cancel_blocked_auth_flow` must cancel a non-terminal flow via `flow_manager`
    /// when `flow_record_source` returns one for the queried run/gate.
    #[tokio::test]
    async fn cancel_blocked_auth_flow_cancels_non_terminal_flow() {
        use ironclaw_host_api::UserId;
        use ironclaw_turns::TurnScope;

        let auth_svc = Arc::new(InMemoryAuthProductServices::new());
        let services = Arc::new(make_auth_services_with_flow_source(Arc::clone(&auth_svc)));

        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:cancel-test";
        let scope_resource = test_auth_product_scope();
        let flow = create_test_flow(&auth_svc, scope_resource, run_id, gate_ref_str).await;

        // Sanity: flow is non-terminal before the call.
        assert_eq!(
            flow.status,
            AuthFlowStatus::AwaitingUser,
            "pre-condition: flow must be non-terminal"
        );

        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");

        services
            .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, gate_ref_str)
            .await
            .expect("cancel_blocked_auth_flow must succeed");

        // The flow must now be terminal (Canceled).
        let flows = auth_svc.flow_records_snapshot();
        let updated = flows
            .iter()
            .find(|f| f.id == flow.id)
            .expect("flow must still exist after cancel");
        assert_eq!(
            updated.status,
            AuthFlowStatus::Canceled,
            "cancel_blocked_auth_flow must have cancelled the flow via flow_manager"
        );
    }

    /// `cancel_blocked_auth_flow` is a no-op (returns `Ok`) when the
    /// `flow_record_source` returns `None` for the queried run/gate (flow absent
    /// or already terminal).
    #[tokio::test]
    async fn cancel_blocked_auth_flow_is_noop_when_flow_absent() {
        use ironclaw_host_api::UserId;
        use ironclaw_turns::{TurnRunId, TurnScope};

        let auth_svc = Arc::new(InMemoryAuthProductServices::new());
        let services = Arc::new(make_auth_services_with_flow_source(Arc::clone(&auth_svc)));

        // No flow is seeded — `flow_for_turn_gate` returns None.
        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");
        let run_id = TurnRunId::new();

        let result = services
            .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, "gate:absent")
            .await;

        assert!(
            result.is_ok(),
            "cancel_blocked_auth_flow must return Ok when flow is absent; got: {result:?}"
        );
        // No flows were created, so nothing to check in auth_svc.
        assert!(
            auth_svc.flow_records_snapshot().is_empty(),
            "no flow must exist after a no-op cancel"
        );
    }

    /// `cancel_blocked_auth_flow` must treat `Err(AuthProductError::Canceled)` and
    /// `Err(AuthProductError::FlowAlreadyTerminal)` from `flow_manager.cancel_flow`
    /// as `Ok(())` — these represent a concurrent terminal race where the flow
    /// completed between the non-terminal read and the cancel call.
    ///
    /// Also asserts a negative case: a real backend error (e.g. `BackendUnavailable`)
    /// still propagates as `Err` to confirm the normalization is not over-broad.
    #[tokio::test]
    async fn cancel_blocked_auth_flow_treats_terminal_race_as_ok() {
        use ironclaw_auth::{
            AuthFlowId, AuthFlowRecord, AuthProductError, AuthProductScope,
            OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput, Timestamp,
        };
        use ironclaw_host_api::UserId;
        use ironclaw_turns::TurnScope;

        /// A `AuthFlowManager` whose `cancel_flow` returns a caller-supplied error
        /// while all other methods forward to the real in-memory store.  Used to
        /// simulate the terminal race without needing to actually put the flow in
        /// a terminal state before the call.
        struct TerminalRaceFlowManager {
            inner: Arc<InMemoryAuthProductServices>,
            cancel_error: tokio::sync::Mutex<Option<AuthProductError>>,
        }

        impl TerminalRaceFlowManager {
            fn returning(
                inner: Arc<InMemoryAuthProductServices>,
                error: AuthProductError,
            ) -> Arc<Self> {
                Arc::new(Self {
                    inner,
                    cancel_error: tokio::sync::Mutex::new(Some(error)),
                })
            }
        }

        #[async_trait::async_trait]
        impl AuthFlowManager for TerminalRaceFlowManager {
            async fn create_flow(
                &self,
                request: NewAuthFlow,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                self.inner.create_flow(request).await
            }

            async fn get_flow(
                &self,
                scope: &AuthProductScope,
                flow_id: AuthFlowId,
            ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
                self.inner.get_flow(scope, flow_id).await
            }

            async fn cancel_flow(
                &self,
                _scope: &AuthProductScope,
                _flow_id: AuthFlowId,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                let err = self
                    .cancel_error
                    .lock()
                    .await
                    .take()
                    .expect("cancel_flow called more than once on TerminalRaceFlowManager");
                Err(err)
            }

            async fn claim_oauth_callback(
                &self,
                _scope: &AuthProductScope,
                _request: OAuthCallbackClaimRequest,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call claim_oauth_callback")
            }

            async fn complete_oauth_callback(
                &self,
                _scope: &AuthProductScope,
                _input: OAuthCallbackInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call complete_oauth_callback")
            }

            async fn complete_credential_selection(
                &self,
                _scope: &AuthProductScope,
                _input: ironclaw_auth::CredentialSelectionInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call complete_credential_selection")
            }

            async fn complete_manual_token(
                &self,
                _scope: &AuthProductScope,
                _input: ironclaw_auth::ManualTokenCompletionInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call complete_manual_token")
            }

            async fn cancel_manual_token(
                &self,
                _scope: &AuthProductScope,
                _interaction_id: AuthInteractionId,
            ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
                unreachable!("terminal-race test does not call cancel_manual_token")
            }

            async fn fail_oauth_callback(
                &self,
                _scope: &AuthProductScope,
                _input: OAuthCallbackFailureInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call fail_oauth_callback")
            }

            async fn claim_continuation_dispatch(
                &self,
                scope: &AuthProductScope,
                input: ironclaw_auth::AuthContinuationDispatchClaimInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                self.inner.claim_continuation_dispatch(scope, input).await
            }

            async fn settle_continuation_dispatch(
                &self,
                scope: &AuthProductScope,
                input: ironclaw_auth::AuthContinuationDispatchSettlementInput,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                self.inner.settle_continuation_dispatch(scope, input).await
            }

            async fn mark_continuation_dispatched(
                &self,
                _scope: &AuthProductScope,
                _flow_id: AuthFlowId,
                _emitted_at: Timestamp,
            ) -> Result<AuthFlowRecord, AuthProductError> {
                unreachable!("terminal-race test does not call mark_continuation_dispatched")
            }
        }

        // Helper: build services with a custom flow_manager but real flow_record_source.
        let build_services_with_manager =
            |auth_svc: Arc<InMemoryAuthProductServices>, manager: Arc<dyn AuthFlowManager>| {
                let double = Arc::new(SharedAuthTestDouble);
                RebornProductAuthServices::new(
                    manager,
                    double.clone() as Arc<dyn AuthInteractionService>,
                    double.clone() as Arc<dyn CredentialSetupService>,
                    double.clone() as Arc<dyn CredentialAccountService>,
                    double.clone() as Arc<dyn AuthProviderClient>,
                    double.clone() as Arc<dyn SecretCleanupService>,
                    Arc::new(NoopAuthContinuationDispatcher),
                )
                .with_flow_record_source(auth_svc as Arc<dyn AuthFlowRecordSource>)
            };

        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");
        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:terminal-race-test";
        let scope_resource = test_auth_product_scope();

        // ── Case 1: cancel_flow returns Err(FlowAlreadyTerminal) → Ok(()) ───────────
        {
            let auth_svc = Arc::new(InMemoryAuthProductServices::new());
            // Seed a non-terminal flow so flow_record_source returns Some(…).
            create_test_flow(&auth_svc, scope_resource.clone(), run_id, gate_ref_str).await;
            let manager = TerminalRaceFlowManager::returning(
                Arc::clone(&auth_svc),
                AuthProductError::FlowAlreadyTerminal,
            );
            let services = Arc::new(build_services_with_manager(auth_svc, manager));

            let result = services
                .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, gate_ref_str)
                .await;
            assert!(
                result.is_ok(),
                "FlowAlreadyTerminal from cancel_flow must be normalized to Ok(()); got: {result:?}"
            );
        }

        // ── Case 2: cancel_flow returns Err(Canceled) → Ok(()) ──────────────────────
        {
            let auth_svc = Arc::new(InMemoryAuthProductServices::new());
            create_test_flow(&auth_svc, scope_resource.clone(), run_id, gate_ref_str).await;
            let manager = TerminalRaceFlowManager::returning(
                Arc::clone(&auth_svc),
                AuthProductError::Canceled,
            );
            let services = Arc::new(build_services_with_manager(auth_svc, manager));

            let result = services
                .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, gate_ref_str)
                .await;
            assert!(
                result.is_ok(),
                "Canceled from cancel_flow must be normalized to Ok(()); got: {result:?}"
            );
        }

        // ── Negative case: cancel_flow returns a real error → Err propagates ─────────
        {
            let auth_svc = Arc::new(InMemoryAuthProductServices::new());
            create_test_flow(&auth_svc, scope_resource, run_id, gate_ref_str).await;
            let manager = TerminalRaceFlowManager::returning(
                Arc::clone(&auth_svc),
                AuthProductError::BackendUnavailable,
            );
            let services = Arc::new(build_services_with_manager(auth_svc, manager));

            let result = services
                .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, gate_ref_str)
                .await;
            assert!(
                matches!(result, Err(AuthProductError::BackendUnavailable)),
                "BackendUnavailable from cancel_flow must propagate as Err; got: {result:?}"
            );
        }
    }

    /// `cancel_blocked_auth_flow` is a no-op (returns `Ok`) when the service
    /// was built without a `flow_record_source`.
    #[tokio::test]
    async fn cancel_blocked_auth_flow_is_noop_without_flow_record_source() {
        use ironclaw_host_api::UserId;
        use ironclaw_turns::{TurnRunId, TurnScope};

        let double = Arc::new(SharedAuthTestDouble);
        // Build WITHOUT `.with_flow_record_source` — `flow_record_source` is None.
        let services = Arc::new(RebornProductAuthServices::new(
            double.clone() as Arc<dyn AuthFlowManager>,
            double.clone() as Arc<dyn AuthInteractionService>,
            double.clone() as Arc<dyn CredentialSetupService>,
            double.clone() as Arc<dyn CredentialAccountService>,
            double.clone() as Arc<dyn AuthProviderClient>,
            double.clone() as Arc<dyn SecretCleanupService>,
            Arc::new(NoopAuthContinuationDispatcher),
        ));

        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");
        let run_id = TurnRunId::new();

        let result = services
            .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, "gate:no-source")
            .await;

        assert!(
            result.is_ok(),
            "cancel_blocked_auth_flow must return Ok when flow_record_source is absent; got: {result:?}"
        );
        // SharedAuthTestDouble's cancel_flow panics with unreachable! — if we reach
        // here without panic, cancel_flow was never called (as required).
    }

    /// `cancel_blocked_auth_flow` must return `Err(AuthProductError::InvalidRequest)`
    /// when the supplied `gate_ref` string fails `AuthGateRef::new` validation.
    ///
    /// `AuthGateRef` delegates to `validate_public_text`, which rejects empty
    /// strings ("must not be empty"). An empty `gate_ref` is therefore the
    /// simplest value that always fails at the facade boundary — regardless of
    /// whether any flow or source is present.
    #[tokio::test]
    async fn cancel_blocked_auth_flow_rejects_invalid_gate_ref() {
        use ironclaw_host_api::UserId;
        use ironclaw_turns::{TurnRunId, TurnScope};

        let auth_svc = Arc::new(InMemoryAuthProductServices::new());
        let services = Arc::new(make_auth_services_with_flow_source(Arc::clone(&auth_svc)));

        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");
        let run_id = TurnRunId::new();

        // Empty string is rejected by `validate_public_text` ("must not be empty").
        let result = services
            .cancel_blocked_auth_flow(&turn_scope, &owner_user_id, run_id, "")
            .await;

        match result {
            Err(AuthProductError::InvalidRequest { reason }) => {
                assert!(
                    !reason.is_empty(),
                    "InvalidRequest reason must be non-empty for an invalid gate ref"
                );
                assert!(
                    reason.contains("invalid gate ref for auth-flow cancel"),
                    "reason must include the caller-supplied context string; got: {reason}"
                );
            }
            other => panic!("expected Err(InvalidRequest) for empty gate_ref, got: {other:?}"),
        }
    }

    /// `cancel_blocked_auth_flow` must propagate `Err` returned by the
    /// `flow_record_source` — a backend lookup failure must not be silently
    /// swallowed.
    ///
    /// Uses a minimal local stub whose `flow_for_turn_gate` always returns
    /// `Err(AuthProductError::BackendUnavailable)`.  This exercises the `?`
    /// on the `source.flow_for_turn_gate(…).await?` call site.
    #[tokio::test]
    async fn cancel_blocked_auth_flow_propagates_flow_source_error() {
        use ironclaw_host_api::UserId;
        use ironclaw_turns::{TurnRunId, TurnScope};

        /// A flow record source that always errors out.
        struct AlwaysFailingFlowSource;

        #[async_trait::async_trait]
        impl AuthFlowRecordSource for AlwaysFailingFlowSource {
            async fn flow_for_turn_gate(
                &self,
                _query: ironclaw_auth::TurnGateAuthFlowQuery,
            ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
                Err(AuthProductError::BackendUnavailable)
            }

            async fn flow_for_owner_by_id(
                &self,
                _owner_scope: &ironclaw_auth::AuthProductScope,
                _flow_id: ironclaw_auth::AuthFlowId,
            ) -> Result<Option<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
                unreachable!("flow-source-error test does not call flow_for_owner_by_id")
            }

            async fn flows_for_owner(
                &self,
                _owner: ironclaw_auth::AuthFlowOwnerScope,
            ) -> Result<Vec<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
                unreachable!("flow-source-error test does not call flows_for_owner")
            }
        }

        let double = Arc::new(SharedAuthTestDouble);
        let services = Arc::new(
            RebornProductAuthServices::new(
                double.clone() as Arc<dyn AuthFlowManager>,
                double.clone() as Arc<dyn AuthInteractionService>,
                double.clone() as Arc<dyn CredentialSetupService>,
                double.clone() as Arc<dyn CredentialAccountService>,
                double.clone() as Arc<dyn AuthProviderClient>,
                double.clone() as Arc<dyn SecretCleanupService>,
                Arc::new(NoopAuthContinuationDispatcher),
            )
            .with_flow_record_source(
                Arc::new(AlwaysFailingFlowSource) as Arc<dyn AuthFlowRecordSource>
            ),
        );

        let turn_scope = TurnScope::new_with_owner(
            ironclaw_host_api::TenantId::new("test-tenant").expect("tenant"),
            Some(ironclaw_host_api::AgentId::new("test-agent").expect("agent")),
            None,
            ironclaw_host_api::ThreadId::new("test-thread").expect("thread"),
            Some(UserId::new("creator-user").expect("owner")),
        );
        let owner_user_id = UserId::new("creator-user").expect("owner");
        let run_id = TurnRunId::new();

        // A valid gate_ref so the validation step is not the rejection point —
        // the error must come from the source lookup.
        let result = services
            .cancel_blocked_auth_flow(
                &turn_scope,
                &owner_user_id,
                run_id,
                "gate:source-error-test",
            )
            .await;

        assert!(
            matches!(result, Err(AuthProductError::BackendUnavailable)),
            "BackendUnavailable from flow_record_source must propagate; got: {result:?}"
        );
    }
}
