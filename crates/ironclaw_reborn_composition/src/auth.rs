use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowManager,
    AuthFlowStatus, AuthInteractionService, AuthProductError, AuthProductScope, AuthProviderClient,
    CredentialAccountId, CredentialAccountService, CredentialSetupService,
    InMemoryAuthProductServices, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
    OAuthCallbackInput, OAuthProviderCallbackRequest, OpaqueStateHash, ProviderCallbackOutcome,
    SecretCleanupService,
};
use ironclaw_product_workflow::{
    AuthContinuationRejectionKind, ProductAuthTurnGateResumeDispatcher, ProductWorkflowError,
};
use ironclaw_turns::{TurnCoordinator, TurnErrorCategory};
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait RebornAuthContinuationDispatcher: Send + Sync {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError>;
}

#[derive(Debug, Default)]
struct NoopAuthContinuationDispatcher;

#[async_trait]
impl RebornAuthContinuationDispatcher for NoopAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

pub(crate) struct RebornProductWorkflowAuthContinuationDispatcher {
    turn_gate_dispatcher: ProductAuthTurnGateResumeDispatcher,
}

impl RebornProductWorkflowAuthContinuationDispatcher {
    pub(crate) fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self {
            turn_gate_dispatcher: ProductAuthTurnGateResumeDispatcher::new(turn_coordinator),
        }
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RebornProductWorkflowAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if matches!(
            &event.continuation,
            AuthContinuationRef::TurnGateResume { .. }
        ) {
            let flow_id = event.flow_id;
            self.turn_gate_dispatcher
                .dispatch_turn_gate_resume(event)
                .await
                .map(|_| ())
                .map_err(|error| {
                    let auth_error = auth_error_for_continuation_dispatch(&error);
                    tracing::debug!(
                        %flow_id,
                        auth_error_code = ?auth_error.code(),
                        workflow_error_kind = workflow_error_kind(&error),
                        "product auth turn-gate continuation dispatch failed"
                    );
                    auth_error
                })
        } else {
            tracing::debug!(
                flow_id = %event.flow_id,
                continuation_kind = continuation_kind(&event.continuation),
                "non-turn auth continuation deferred to follow-up handler"
            );
            Ok(())
        }
    }
}

fn continuation_kind(continuation: &AuthContinuationRef) -> &'static str {
    match continuation {
        AuthContinuationRef::SetupOnly => "setup_only",
        AuthContinuationRef::LifecycleActivation { .. } => "lifecycle_activation",
        AuthContinuationRef::ProductActionResume { .. } => "product_action_resume",
        AuthContinuationRef::TurnGateResume { .. } => "turn_gate_resume",
    }
}

fn auth_error_for_continuation_dispatch(error: &ProductWorkflowError) -> AuthProductError {
    match error {
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unavailable =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Conflict =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unauthorized =>
        {
            AuthProductError::CrossScopeDenied
        }
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::ScopeNotFound =>
        {
            AuthProductError::UnknownOrExpiredFlow
        }
        ProductWorkflowError::TurnSubmissionFailed { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume failed".to_string(),
        },
        ProductWorkflowError::Transient { .. } => AuthProductError::BackendUnavailable,
        ProductWorkflowError::TurnResumeDenied { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume denied".to_string(),
        },
        ProductWorkflowError::AuthContinuationRejected { kind } => {
            AuthProductError::InvalidRequest {
                reason: kind.sanitized_reason().to_string(),
            }
        }
        ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation rejected".to_string(),
        },
        _ => AuthProductError::InvalidRequest {
            reason: "auth continuation dispatch failed".to_string(),
        },
    }
}

fn workflow_error_kind(error: &ProductWorkflowError) -> &'static str {
    match error {
        ProductWorkflowError::TurnSubmissionRejected { .. } => "turn_submission_rejected",
        ProductWorkflowError::TurnSubmissionFailed { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_admission_rejected",
            TurnErrorCategory::ScopeNotFound => "turn_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_invalid_request",
            TurnErrorCategory::Unavailable => "turn_unavailable",
            TurnErrorCategory::Conflict => "turn_conflict",
        },
        ProductWorkflowError::TurnResumeRejected { .. } => "turn_resume_rejected",
        ProductWorkflowError::AuthContinuationRejected { kind } => match kind {
            AuthContinuationRejectionKind::NotTurnGateResume => {
                "auth_continuation_not_turn_gate_resume"
            }
            AuthContinuationRejectionKind::MissingThreadScope => {
                "auth_continuation_missing_thread_scope"
            }
            AuthContinuationRejectionKind::InvalidTurnRunRef => {
                "auth_continuation_invalid_turn_run_ref"
            }
            AuthContinuationRejectionKind::InvalidGateRef => "auth_continuation_invalid_gate_ref",
            AuthContinuationRejectionKind::InvalidIdempotencyKey => {
                "auth_continuation_invalid_idempotency_key"
            }
            AuthContinuationRejectionKind::InvalidBindingRef => {
                "auth_continuation_invalid_binding_ref"
            }
            AuthContinuationRejectionKind::UnauthorizedBlockedGate => {
                "auth_continuation_unauthorized_blocked_gate"
            }
        },
        ProductWorkflowError::TurnResumeDenied { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_resume_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_resume_admission_rejected",
            TurnErrorCategory::ScopeNotFound => "turn_resume_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_resume_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_resume_invalid_request",
            TurnErrorCategory::Unavailable => "turn_resume_unavailable",
            TurnErrorCategory::Conflict => "turn_resume_conflict",
        },
        ProductWorkflowError::Transient { .. } => "transient",
        _ => "workflow_error",
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

/// Stable sanitized callback failure safe for route rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornOAuthCallbackError {
    pub code: AuthErrorCode,
    pub retryable: bool,
}

impl From<AuthProductError> for RebornOAuthCallbackError {
    fn from(error: AuthProductError) -> Self {
        let code = error.code();
        Self {
            code,
            retryable: matches!(code, AuthErrorCode::BackendUnavailable),
        }
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
    credential_setup_service: Arc<dyn CredentialSetupService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
    provider_client: Arc<dyn AuthProviderClient>,
    cleanup_service: Arc<dyn SecretCleanupService>,
    continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
}

impl std::fmt::Debug for RebornProductAuthServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornProductAuthServices")
            .field("flow_manager", &"Arc<dyn AuthFlowManager>")
            .field("interaction_service", &"Arc<dyn AuthInteractionService>")
            .field(
                "credential_setup_service",
                &"Arc<dyn CredentialSetupService>",
            )
            .field(
                "credential_account_service",
                &"Arc<dyn CredentialAccountService>",
            )
            .field("provider_client", &"Arc<dyn AuthProviderClient>")
            .field("cleanup_service", &"Arc<dyn SecretCleanupService>")
            .field(
                "continuation_dispatcher",
                &"Arc<dyn RebornAuthContinuationDispatcher>",
            )
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
        Self {
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
            continuation_dispatcher,
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
            + AuthProviderClient
            + SecretCleanupService
            + 'static,
    {
        let flow_manager: Arc<dyn AuthFlowManager> = services.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = services.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = services.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = services.clone();
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
    }

    pub fn from_shared_with_noop_dispatcher_for_tests<T>(services: Arc<T>) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + AuthProviderClient
            + SecretCleanupService
            + 'static,
    {
        Self::from_shared(services, Arc::new(NoopAuthContinuationDispatcher))
    }

    pub fn flow_manager(&self) -> Arc<dyn AuthFlowManager> {
        self.flow_manager.clone()
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

    pub fn provider_client(&self) -> Arc<dyn AuthProviderClient> {
        self.provider_client.clone()
    }

    pub fn cleanup_service(&self) -> Arc<dyn SecretCleanupService> {
        self.cleanup_service.clone()
    }

    pub fn with_provider_client(mut self, provider_client: Arc<dyn AuthProviderClient>) -> Self {
        self.provider_client = provider_client;
        self
    }

    pub fn with_continuation_dispatcher(
        mut self,
        dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        self.continuation_dispatcher = dispatcher;
        self
    }

    pub async fn handle_oauth_callback(
        &self,
        request: RebornOAuthCallbackRequest,
    ) -> Result<RebornOAuthCallbackResponse, RebornOAuthCallbackError> {
        let completed = match request.outcome {
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
                    claimed
                } else {
                    let exchange = match self
                        .provider_client
                        .exchange_callback(provider_request)
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
                                tracing::debug!(
                                    flow_id = %request.flow_id,
                                    exchange_error_code = ?error_code,
                                    fail_error_code = ?fail_error.code(),
                                    "reborn auth callback provider exchange failed and flow failure update failed"
                                );
                            }
                            return Err(error.into());
                        }
                    };
                    self.flow_manager
                        .complete_oauth_callback(
                            &request.scope,
                            OAuthCallbackInput {
                                flow_id: request.flow_id,
                                opaque_state_hash: request.opaque_state_hash,
                                outcome: ProviderCallbackOutcome::Authorized { exchange },
                            },
                        )
                        .await
                        .map_err(RebornOAuthCallbackError::from)?
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
                .map_err(RebornOAuthCallbackError::from)?,
            RebornOAuthCallbackOutcome::Malformed => {
                return Err(AuthProductError::MalformedCallback.into());
            }
        };

        let event = AuthContinuationEvent {
            flow_id: completed.id,
            scope: completed.scope.clone(),
            continuation: completed.continuation.clone(),
            credential_account_id: completed.credential_account_id,
            emitted_at: Utc::now(),
        };
        if let Err(error) = self
            .continuation_dispatcher
            .dispatch_auth_continuation(event)
            .await
        {
            tracing::debug!(
                flow_id = %completed.id,
                error_code = ?error.code(),
                "reborn auth callback completed but continuation dispatch failed"
            );
            return Err(error.into());
        }

        Ok(RebornOAuthCallbackResponse {
            flow_id: completed.id,
            status: completed.status,
            credential_account_id: completed.credential_account_id,
            continuation: completed.continuation,
        })
    }

    pub(crate) fn local_dev_in_memory(
        continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        Self::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            continuation_dispatcher,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_auth::{
        AuthChallenge, AuthFlowId, AuthFlowRecord, AuthProductError, AuthProductScope,
        CredentialAccount, CredentialAccountId, CredentialAccountListPage,
        CredentialAccountListRequest, CredentialAccountMutation, CredentialAccountProjection,
        CredentialAccountSelectionRequest, CredentialAccountStatus, NewAuthFlow,
        NewCredentialAccount, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
        OAuthCallbackInput, OAuthProviderCallbackRequest, OAuthProviderExchange,
        SecretCleanupReport, SecretCleanupRequest, SecretSubmitRequest, SecretSubmitResult,
    };
    use ironclaw_turns::TurnError;

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
    fn auth_error_for_continuation_dispatch_preserves_retryable_resume_denials() {
        for error in [
            TurnError::Unavailable {
                reason: "turn coordinator offline".to_string(),
            },
            TurnError::LeaseMismatch,
        ] {
            let auth_error =
                auth_error_for_continuation_dispatch(&ProductWorkflowError::TurnResumeDenied {
                    error,
                });

            assert_eq!(auth_error.code(), AuthErrorCode::BackendUnavailable);
        }
    }

    #[test]
    fn auth_error_for_continuation_dispatch_maps_transient_and_catch_all() {
        let transient = auth_error_for_continuation_dispatch(&ProductWorkflowError::Transient {
            reason: "store timeout".to_string(),
        });
        assert_eq!(transient.code(), AuthErrorCode::BackendUnavailable);

        let catch_all =
            auth_error_for_continuation_dispatch(&ProductWorkflowError::UnknownInstallation);
        assert_eq!(catch_all.code(), AuthErrorCode::InvalidRequest);
        assert!(matches!(
            catch_all,
            AuthProductError::InvalidRequest { reason }
                if reason == "auth continuation dispatch failed"
        ));
    }

    #[test]
    fn auth_continuation_rejection_kind_returns_stable_static_strings() {
        for (kind, expected) in [
            (
                AuthContinuationRejectionKind::NotTurnGateResume,
                "auth continuation is not a turn-gate resume",
            ),
            (
                AuthContinuationRejectionKind::MissingThreadScope,
                "invalid auth continuation scope",
            ),
            (
                AuthContinuationRejectionKind::InvalidTurnRunRef,
                "invalid auth continuation run reference",
            ),
            (
                AuthContinuationRejectionKind::InvalidGateRef,
                "invalid auth continuation gate reference",
            ),
            (
                AuthContinuationRejectionKind::InvalidIdempotencyKey,
                "invalid auth continuation idempotency key",
            ),
            (
                AuthContinuationRejectionKind::InvalidBindingRef,
                "invalid auth continuation binding ref",
            ),
            (
                AuthContinuationRejectionKind::UnauthorizedBlockedGate,
                "auth continuation does not match an authorized blocked auth gate",
            ),
        ] {
            let auth_error = auth_error_for_continuation_dispatch(
                &ProductWorkflowError::AuthContinuationRejected { kind },
            );

            assert!(matches!(
                auth_error,
                AuthProductError::InvalidRequest { reason } if reason == expected
            ));
        }
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
            _scope: &AuthProductScope,
            _account_id: CredentialAccountId,
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
    }

    #[async_trait::async_trait]
    impl AuthProviderClient for SharedAuthTestDouble {
        async fn exchange_callback(
            &self,
            _request: OAuthProviderCallbackRequest,
        ) -> Result<OAuthProviderExchange, AuthProductError> {
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
