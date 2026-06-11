use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId,
    AuthFlowKind, AuthFlowManager, AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource,
    AuthFlowStatus, AuthGateRef, AuthInteractionId, AuthInteractionService, AuthProductError,
    AuthProductScope, AuthProviderClient, AuthProviderId, CredentialAccountChoiceRequest,
    CredentialAccountId, CredentialAccountLabel, CredentialAccountListPage,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountProjection,
    CredentialAccountRecordSource, CredentialAccountService, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialRecoveryProjection, CredentialRecoveryRequest,
    CredentialRefreshReport, CredentialRefreshRequest, CredentialSetupService,
    InMemoryAuthProductServices, ManualTokenSetupRequest, NewAuthFlow, OAuthAuthorizationUrl,
    OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput,
    OAuthProviderCallbackRequest, OAuthProviderExchangeContext, OpaqueStateHash, PkceVerifierHash,
    ProviderBackedCredentialAccountService, ProviderCallbackOutcome, ProviderScope,
    SecretCleanupReport, SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest,
    SecretSubmitResult, Timestamp, TurnGateAuthFlowQuery, TurnRunRef, scope_matches,
};
use ironclaw_product_adapters::AuthPromptChallengeKind;
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::UserId;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::manual_token_flow::{PortBackedManualTokenFlowService, RebornManualTokenFlowService};
use crate::oauth_dcr::{DcrGateChallengeRequest, DcrSetupFlowRequest, OAuthDcrProviderRegistry};
use crate::oauth_gate::{GoogleOAuthGateProviderRegistry, OAuthGateChallengeRequest};
use crate::product_auth_runtime_credentials::{
    ProductAuthRuntimeCredentialAccountRefresher, ProductAuthRuntimeCredentialAccountSelector,
    RuntimeCredentialAccountRefreshPort, RuntimeCredentialAccountRefreshService,
    RuntimeCredentialAccountSelectionService,
};
use crate::{AuthChallengeProvider, AuthChallengeView};

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
    ) -> RebornProductAuthServices {
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
    dcr_oauth_registry: Option<Arc<OAuthDcrProviderRegistry>>,
    oauth_gate_registry: Option<Arc<GoogleOAuthGateProviderRegistry>>,
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
        formatter
            .debug_struct("RebornProductAuthServices")
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
            .field(
                "continuation_dispatcher",
                &"Arc<dyn RebornAuthContinuationDispatcher>",
            )
            .field("flow_record_source", &self.flow_record_source.is_some())
            .field("dcr_oauth_registry", &self.dcr_oauth_registry.is_some())
            .field("oauth_gate_registry", &self.oauth_gate_registry.is_some())
            .finish()
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

    pub(crate) fn runtime_credential_account_selection_service(
        &self,
    ) -> Arc<dyn RuntimeCredentialAccountSelectionService> {
        Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
                self.credential_account_record_source(),
                Arc::new(crate::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
            ),
        )
    }

    pub(crate) fn runtime_credential_account_refresh_service(
        self: &Arc<Self>,
    ) -> Arc<dyn RuntimeCredentialAccountRefreshService> {
        let refresh_port: Arc<dyn RuntimeCredentialAccountRefreshPort> = self.clone();
        Arc::new(ProductAuthRuntimeCredentialAccountRefresher::new(
            refresh_port,
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
        registry: Arc<GoogleOAuthGateProviderRegistry>,
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
        if self.flow_record_source.is_some() {
            Some(Arc::clone(self) as Arc<dyn AuthChallengeProvider>)
        } else {
            None
        }
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
        self.cleanup_service
            .cleanup_for_lifecycle(request)
            .await
            .map_err(RebornCredentialLifecycleError::from)
    }

    pub async fn handle_oauth_callback(
        &self,
        request: RebornOAuthCallbackRequest,
    ) -> Result<RebornOAuthCallbackResponse, RebornOAuthCallbackError> {
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

                if claimed.status == AuthFlowStatus::Completed {
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
                    let exchange_for_cleanup = exchange.clone();
                    let completed = match self
                        .flow_manager
                        .complete_oauth_callback(
                            &request.scope,
                            OAuthCallbackInput {
                                flow_id: request.flow_id,
                                opaque_state_hash: request.opaque_state_hash.clone(),
                                outcome: ProviderCallbackOutcome::Authorized { exchange },
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
                return Err(AuthProductError::MalformedCallback.into());
            }
        };

        if should_dispatch_continuation {
            completed = self
                .dispatch_completed_continuation(completed)
                .await
                .map_err(RebornOAuthCallbackError::from)?;
        }

        Ok(RebornOAuthCallbackResponse {
            flow_id: completed.id,
            status: completed.status,
            credential_account_id: completed.credential_account_id,
            continuation: completed.continuation,
        })
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
                continuation: AuthContinuationRef::SetupOnly,
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
    ) -> Result<AuthFlowRecord, AuthProductError> {
        if completed.continuation_emitted_at.is_some() {
            return Ok(completed);
        }
        let emitted_at = Utc::now();
        let event = AuthContinuationEvent {
            flow_id: completed.id,
            scope: completed.scope.clone(),
            continuation: completed.continuation.clone(),
            credential_account_id: completed.credential_account_id,
            emitted_at,
        };
        if let Err(error) = self
            .continuation_dispatcher
            .dispatch_auth_continuation(event)
            .await
        {
            tracing::debug!(
                flow_id = %completed.id,
                error_code = ?error.code(),
                "reborn auth flow completed but continuation dispatch failed"
            );
            let error = match error {
                AuthProductError::TokenExchangeFailed
                | AuthProductError::ProviderDenied
                | AuthProductError::MalformedCallback => AuthProductError::BackendUnavailable,
                error => error,
            };
            return Err(error);
        }
        self.flow_manager
            .mark_continuation_dispatched(&completed.scope, completed.id, emitted_at)
            .await
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
            .into_services(continuation_dispatcher)
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

    fn arc_data_ptr<T: ?Sized>(arc: &Arc<T>) -> *const () {
        Arc::as_ptr(arc) as *const ()
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

    #[test]
    fn reborn_product_auth_ports_from_shared_with_provider_uses_separate_provider_client() {
        let shared = Arc::new(SharedAuthTestDouble);
        let provider_client: Arc<dyn AuthProviderClient> = Arc::new(SharedAuthTestDouble);
        let shared_ptr = arc_data_ptr(&shared);
        let provider_ptr = arc_data_ptr(&provider_client);

        let ports =
            RebornProductAuthServicePorts::from_shared_with_provider(shared, provider_client);
        let services = ports.into_services(Arc::new(NoopAuthContinuationDispatcher));

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
        async fn cleanup_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, AuthProductError> {
            unreachable!("constructor tests do not call cleanup methods")
        }
    }
}
