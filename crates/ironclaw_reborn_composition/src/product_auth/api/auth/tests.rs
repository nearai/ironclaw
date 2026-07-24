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
    OAuthProviderRefreshRequest, SecretCleanupReport, SecretCleanupRequest, SecretSubmitRequest,
    SecretSubmitResult,
};

struct SharedAuthTestDouble;

fn arc_data_ptr<T: ?Sized>(arc: &Arc<T>) -> *const () {
    Arc::as_ptr(arc) as *const ()
}

#[test]
fn reborn_product_auth_services_new_accepts_separate_impls() {
    let flow_manager: Arc<dyn AuthFlowManager> = Arc::new(SharedAuthTestDouble);
    let interaction_service: Arc<dyn AuthInteractionService> = Arc::new(SharedAuthTestDouble);
    let credential_setup_service: Arc<dyn CredentialSetupService> = Arc::new(SharedAuthTestDouble);
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
    let services =
        RebornProductAuthServices::from_shared(shared, Arc::new(NoopAuthContinuationDispatcher));

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
    let services =
        RebornProductAuthServices::from_shared(shared, Arc::new(NoopAuthContinuationDispatcher));
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

    let services =
        RebornProductAuthServices::from_shared(shared, Arc::new(NoopAuthContinuationDispatcher));

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

    let ports = RebornProductAuthServicePorts::from_shared_with_provider(shared, provider_client);
    let services = ports.into_services(
        Arc::new(NoopAuthContinuationDispatcher),
        Arc::new(ironclaw_secrets::SecretStore::ephemeral()),
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
    async fn create_flow(&self, _request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
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

    async fn fail_completed_continuation(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
        _error: ironclaw_auth::AuthErrorCode,
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

/// Cleanup double that reports one canceled flow so the composition-layer
/// eager durable-verifier drop can be exercised.
struct ReportingCleanupService {
    canceled: Vec<ironclaw_auth::CanceledCleanupFlow>,
}

#[async_trait::async_trait]
impl SecretCleanupService for ReportingCleanupService {
    async fn cleanup_for_lifecycle(
        &self,
        _request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        Ok(SecretCleanupReport {
            canceled_flows: self.canceled.clone(),
            ..SecretCleanupReport::default()
        })
    }
}

/// The uninstall/disconnect race: cleanup walks a pending setup flow
/// terminal, so its durable PKCE verifier must be dropped eagerly rather
/// than lingering to TTL. Pins the `report.canceled_flows` → discard loop
/// in `cleanup_credentials_for_lifecycle`.
#[tokio::test]
async fn lifecycle_cleanup_drops_canceled_flows_durable_pkce_verifier() {
    use ironclaw_host_api::ExtensionId;

    let secret_store: Arc<dyn ironclaw_secrets::SecretStorePort> =
        Arc::new(ironclaw_secrets::SecretStore::ephemeral());
    let scope = test_auth_product_scope();
    let flow_id = AuthFlowId::new();
    let double = Arc::new(SharedAuthTestDouble);
    let cleanup = Arc::new(ReportingCleanupService {
        canceled: vec![ironclaw_auth::CanceledCleanupFlow {
            scope: scope.clone(),
            flow_id,
        }],
    });
    let services = RebornProductAuthServices::new(
        Arc::new(InMemoryAuthProductServices::new()) as Arc<dyn AuthFlowManager>,
        double.clone() as Arc<dyn AuthInteractionService>,
        double.clone() as Arc<dyn CredentialSetupService>,
        double.clone() as Arc<dyn CredentialAccountService>,
        double.clone() as Arc<dyn AuthProviderClient>,
        cleanup as Arc<dyn SecretCleanupService>,
        Arc::new(NoopAuthContinuationDispatcher),
    )
    .with_secret_store(Arc::clone(&secret_store));

    // A durable verifier exists for the flow about to be canceled.
    services
        .store_setup_pkce_verifier(
            &scope,
            flow_id,
            secrecy::SecretString::from("verifier-abc".to_string()),
            Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .expect("store verifier");
    assert!(
        services
            .consume_setup_pkce_verifier(&scope, flow_id)
            .await
            .expect("read verifier")
            .is_some(),
        "precondition: the verifier is durably present"
    );
    // Re-store it (the assertion above consumed the one-shot copy).
    services
        .store_setup_pkce_verifier(
            &scope,
            flow_id,
            secrecy::SecretString::from("verifier-abc".to_string()),
            Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .expect("restore verifier");

    services
        .cleanup_credentials_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ExtensionId::new("vendorco").expect("extension"),
            provider: None,
            lifecycle_package: None,
            action: ironclaw_auth::SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup succeeds");

    assert!(
        services
            .consume_setup_pkce_verifier(&scope, flow_id)
            .await
            .expect("read verifier after cleanup")
            .is_none(),
        "cleanup must have dropped the canceled flow's durable verifier"
    );
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
        AuthFlowId, AuthFlowRecord, AuthProductError, AuthProductScope, OAuthCallbackClaimRequest,
        OAuthCallbackFailureInput, OAuthCallbackInput, Timestamp,
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

        async fn mark_continuation_dispatched(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
            _emitted_at: Timestamp,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("terminal-race test does not call mark_continuation_dispatched")
        }

        async fn fail_completed_continuation(
            &self,
            _scope: &AuthProductScope,
            _flow_id: AuthFlowId,
            _error: ironclaw_auth::AuthErrorCode,
        ) -> Result<AuthFlowRecord, AuthProductError> {
            unreachable!("terminal-race test does not call fail_completed_continuation")
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
        let manager =
            TerminalRaceFlowManager::returning(Arc::clone(&auth_svc), AuthProductError::Canceled);
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

        async fn flows_for_owner(
            &self,
            _owner: ironclaw_auth::AuthFlowOwnerScope,
        ) -> Result<Vec<ironclaw_auth::AuthFlowRecord>, AuthProductError> {
            unreachable!("flow-source-error test does not call flows_for_owner")
        }
    }

    let double = Arc::new(SharedAuthTestDouble);
    let services =
        Arc::new(
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
