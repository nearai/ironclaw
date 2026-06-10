use super::*;

#[tokio::test]
async fn filesystem_flow_record_source_projects_session_scoped_manual_flows() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut scope = test_scope();
    scope.surface = AuthSurface::Callback;
    scope.resource.thread_id = Some(ThreadId::new("thread-auth-flow").unwrap());
    scope.session_id = Some(AuthSessionId::new("session-auth-flow").unwrap());
    let service = FilesystemAuthProductServices::new(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at,
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at: challenge_expires_at,
    } = challenge
    else {
        panic!("expected manual token challenge");
    };
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider,
                label,
                expires_at: challenge_expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();

    let submitted = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap();
    service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: submitted.account_id,
            },
        )
        .await
        .unwrap();

    let owner = AuthFlowOwnerScope {
        tenant_id: scope.resource.tenant_id.clone(),
        user_id: scope.resource.user_id.clone(),
        agent_id: scope.resource.agent_id.clone(),
        project_id: scope.resource.project_id.clone(),
        thread_id: scope.resource.thread_id.clone().unwrap(),
    };
    let snapshot = service.flows_for_owner(owner).await.unwrap();
    let projected = snapshot
        .iter()
        .find(|record| record.id == flow.id)
        .expect("session-scoped flow should be projected for auth gates");

    assert_eq!(projected.status, AuthFlowStatus::Completed);
    assert_eq!(projected.scope.session_id, scope.session_id);
    assert_eq!(
        projected.credential_account_id,
        Some(submitted.account_id),
        "manual-token completion must remain visible to the auth read model"
    );
}

#[tokio::test]
async fn filesystem_flow_for_turn_gate_finds_session_scoped_flow_without_owner_snapshot() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut scope = test_scope();
    scope.surface = AuthSurface::Callback;
    scope.resource.thread_id = Some(ThreadId::new("thread-turn-gate").unwrap());
    scope.session_id = Some(AuthSessionId::new("session-turn-gate").unwrap());
    let service = FilesystemAuthProductServices::new(filesystem, secret_store);
    let turn_run_ref = TurnRunRef::new("run-turn-gate").unwrap();
    let gate_ref = AuthGateRef::new("gate-product-auth").unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::SetupRequired {
                provider: google_provider(),
                message: "connect account".to_string(),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: turn_run_ref.clone(),
                gate_ref: gate_ref.clone(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();

    let owner = AuthFlowOwnerScope {
        tenant_id: scope.resource.tenant_id.clone(),
        user_id: scope.resource.user_id.clone(),
        agent_id: scope.resource.agent_id.clone(),
        project_id: scope.resource.project_id.clone(),
        thread_id: scope.resource.thread_id.clone().unwrap(),
    };
    let projected = service
        .flow_for_turn_gate(TurnGateAuthFlowQuery {
            owner,
            turn_run_ref,
            gate_ref,
            include_terminal: false,
        })
        .await
        .unwrap()
        .expect("session-scoped turn-gate flow should be projected");

    assert_eq!(projected.id, flow.id);
    assert_eq!(projected.scope.session_id, scope.session_id);
}

// ─── tests: cancel_flow, fail_oauth_callback, complete_credential_selection ───

#[tokio::test]
async fn filesystem_cancel_flow_and_terminal_state_rejection() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("cancel-s")),
            pkce_verifier_hash: Some(pkce_hash("cancel-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    let cancelled = service.cancel_flow(&scope, flow.id).await.unwrap();
    assert_eq!(cancelled.status, AuthFlowStatus::Canceled);

    // Second cancel on already-terminal flow returns Canceled error.
    let err = service
        .cancel_flow(&scope, flow.id)
        .await
        .expect_err("second cancel must fail");
    assert_eq!(err, AuthProductError::Canceled);
}

#[tokio::test]
async fn filesystem_fail_oauth_callback_marks_flow_failed() {
    use ironclaw_auth::{AuthErrorCode, OAuthCallbackFailureInput};
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("fail-s")),
            pkce_verifier_hash: Some(pkce_hash("fail-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("fail-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("fail-p"),
            },
        )
        .await
        .unwrap();

    let failed = service
        .fail_oauth_callback(
            &scope,
            OAuthCallbackFailureInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("fail-s"),
                error: AuthErrorCode::ProviderDenied,
            },
        )
        .await
        .unwrap();
    assert_eq!(failed.status, AuthFlowStatus::Failed);
    assert_eq!(failed.error, Some(AuthErrorCode::ProviderDenied));
}

#[tokio::test]
async fn filesystem_complete_credential_selection_completes_flow() {
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: google_provider(),
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    let completed = service
        .complete_credential_selection(
            &scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(completed.credential_account_id, Some(account.id));
}

// ─── tests: create_flow update_binding validation ─────────────────────────────

#[tokio::test]
async fn filesystem_create_flow_rejects_invalid_update_binding() {
    use ironclaw_auth::CredentialAccountUpdateBinding;
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // Non-existent account in update_binding → CredentialMissing.
    let err = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id: CredentialAccountId::new(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("ubv-s")),
            pkce_verifier_hash: Some(pkce_hash("ubv-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("non-existent binding account must return CredentialMissing");
    assert_eq!(err, AuthProductError::CredentialMissing);
}

// ─── fix: abbyshekit review — expired flow mutation persisted ────────────────

#[tokio::test]
async fn filesystem_expired_flow_status_persisted_before_returning_error() {
    // When claim_oauth_callback / complete_oauth_callback / fail_oauth_callback
    // encounter an expired flow, the Expired status must be written to disk
    // before returning UnknownOrExpiredFlow so durable state matches the contract.
    use ironclaw_auth::{
        AuthErrorCode, OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput,
        ProviderCallbackOutcome,
    };

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("exp-s")),
            pkce_verifier_hash: Some(pkce_hash("exp-p")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();

    // claim_oauth_callback must persist Expired before returning error.
    let err = service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("exp-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("exp-p"),
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(err, AuthProductError::UnknownOrExpiredFlow);

    let persisted = service
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("flow must still exist");
    assert_eq!(persisted.status, AuthFlowStatus::Expired);
    assert_eq!(persisted.error, Some(AuthErrorCode::UnknownOrExpiredFlow));

    // fail_oauth_callback on already-expired flow returns FlowAlreadyTerminal
    // because Expired is a terminal status; the record was already persisted
    // as Expired by claim_oauth_callback above.
    let err2 = service
        .fail_oauth_callback(
            &scope,
            OAuthCallbackFailureInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("exp-s"),
                error: AuthErrorCode::ProviderDenied,
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(
        err2,
        AuthProductError::FlowAlreadyTerminal,
        "already-expired flow returns FlowAlreadyTerminal"
    );

    let persisted2 = service
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("flow must still exist");
    assert_eq!(persisted2.status, AuthFlowStatus::Expired);
    assert_eq!(persisted2.error, Some(AuthErrorCode::UnknownOrExpiredFlow));

    // complete_oauth_callback on a fresh expired flow (never claimed) must also
    // persist the Expired status before returning error.
    let flow2 = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("exp2-s")),
            pkce_verifier_hash: Some(pkce_hash("exp2-p")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();

    let err3 = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("exp2-s"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(
        err3,
        AuthProductError::UnknownOrExpiredFlow,
        "complete_oauth_callback on expired flow returns UnknownOrExpiredFlow"
    );

    let persisted3 = service
        .get_flow(&scope, flow2.id)
        .await
        .unwrap()
        .expect("flow2 must still exist");
    assert_eq!(persisted3.status, AuthFlowStatus::Expired);
    assert_eq!(persisted3.error, Some(AuthErrorCode::UnknownOrExpiredFlow));
}
