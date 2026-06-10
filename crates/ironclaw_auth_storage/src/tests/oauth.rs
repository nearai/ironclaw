use super::*;

#[tokio::test]
async fn filesystem_oauth_callback_claim_is_one_shot_and_completion_persists() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

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
            opaque_state_hash: Some(state_hash("state")),
            pkce_verifier_hash: Some(pkce_hash("pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let claim = OAuthCallbackClaimRequest {
        flow_id: flow.id,
        opaque_state_hash: state_hash("state"),
        provider: google_provider(),
        pkce_verifier_hash: pkce_hash("pkce"),
    };

    let claimed = service
        .claim_oauth_callback(&scope, claim.clone())
        .await
        .unwrap();
    assert_eq!(claimed.status, AuthFlowStatus::CallbackReceived);

    let second_claim = service
        .claim_oauth_callback(&scope, claim.clone())
        .await
        .expect_err("in-flight callback claim must be one-shot");
    assert_eq!(second_claim, AuthProductError::FlowAlreadyTerminal);

    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code"),
                        pkce_verifier_hash: pkce_hash("pkce"),
                        access_secret: SecretHandle::new("oauth-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("oauth-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.credential_account_id.is_some());

    let emitted_at = Utc::now();
    service
        .mark_continuation_dispatched(&scope, flow.id, emitted_at)
        .await
        .unwrap();

    let recreated = test_service(Arc::clone(&filesystem), secret_store);
    let stored = recreated
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("completed flow should be durable");
    assert_eq!(stored.status, AuthFlowStatus::Completed);
    assert_eq!(stored.continuation_emitted_at, Some(emitted_at));

    let completed_replay = recreated
        .claim_oauth_callback(&scope, claim)
        .await
        .expect("completed callback replay should not reclaim provider exchange");
    assert_eq!(completed_replay.status, AuthFlowStatus::Completed);
    assert_eq!(completed_replay.continuation_emitted_at, Some(emitted_at));
}

// ─── fix: mark_continuation_dispatched is idempotent ─────────────────────────

#[tokio::test]
async fn filesystem_oauth_continuation_marker_is_idempotent() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

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
            opaque_state_hash: Some(state_hash("s")),
            pkce_verifier_hash: Some(pkce_hash("p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Complete the flow so mark_continuation_dispatched is valid.
    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("p"),
            },
        )
        .await
        .unwrap();
    service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c"),
                        pkce_verifier_hash: pkce_hash("p"),
                        access_secret: SecretHandle::new("access").unwrap(),
                        refresh_secret: None,
                        scopes: vec![],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    let first_at = Utc::now();
    let first = service
        .mark_continuation_dispatched(&scope, flow.id, first_at)
        .await
        .unwrap();
    assert_eq!(first.continuation_emitted_at, Some(first_at));

    // Second call with a different timestamp must NOT overwrite.
    let second_at = first_at + Duration::seconds(1);
    let second = service
        .mark_continuation_dispatched(&scope, flow.id, second_at)
        .await
        .unwrap();
    assert_eq!(
        second.continuation_emitted_at,
        Some(first_at),
        "idempotent: second call must not overwrite the first emitted_at"
    );
}

// ─── zmanian follow-up #1: OAuth re-auth must purge previous secret handles ──

#[tokio::test]
async fn filesystem_oauth_reauth_purges_previous_provider_secrets() {
    // After a successful OAuth re-auth through a bound flow, the OLD access
    // and refresh secret handles must be deleted from SecretStore so repeated
    // re-auths do not accumulate dead handles. Host OAuth provider clients
    // return exchange.account_id == None, so the durable flow must use the
    // update_binding account id rather than rejecting the callback.
    use ironclaw_auth::{CredentialAccountUpdateBinding, ProviderCallbackOutcome};
    use ironclaw_secrets::SecretMaterial;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // ── Step 1: initial OAuth flow creates a new account ─────────────────────
    let flow1 = service
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
            opaque_state_hash: Some(state_hash("state1")),
            pkce_verifier_hash: Some(pkce_hash("pkce1")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce1"),
            },
        )
        .await
        .unwrap();

    let access_v1 = SecretHandle::new("oauth-access-v1").unwrap();
    let refresh_v1 = SecretHandle::new("oauth-refresh-v1").unwrap();
    // Pre-populate SecretStore to simulate provider client having stored these
    // handles; this lets us verify they are purged on re-auth.
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v1.clone(),
            SecretMaterial::from("access-token-v1"),
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh_v1.clone(),
            SecretMaterial::from("refresh-token-v1"),
        )
        .await
        .unwrap();

    let completed1 = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code1"),
                        pkce_verifier_hash: pkce_hash("pkce1"),
                        access_secret: access_v1.clone(),
                        refresh_secret: Some(refresh_v1.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    let account_id = completed1
        .credential_account_id
        .expect("first OAuth flow must produce a credential account");

    // v1 handles must be present before re-auth.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_some(),
        "v1 access handle must exist before re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v1)
            .await
            .unwrap()
            .is_some(),
        "v1 refresh handle must exist before re-auth"
    );

    // ── Step 2: re-auth flow bound to the existing account ───────────────────
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
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("state2")),
            pkce_verifier_hash: Some(pkce_hash("pkce2")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce2"),
            },
        )
        .await
        .unwrap();

    let access_v2 = SecretHandle::new("oauth-access-v2").unwrap();
    let refresh_v2 = SecretHandle::new("oauth-refresh-v2").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v2.clone(),
            SecretMaterial::from("access-token-v2"),
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh_v2.clone(),
            SecretMaterial::from("refresh-token-v2"),
        )
        .await
        .unwrap();

    service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code2"),
                        pkce_verifier_hash: pkce_hash("pkce2"),
                        access_secret: access_v2.clone(),
                        refresh_secret: Some(refresh_v2.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    // Old handles must have been purged from SecretStore.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_none(),
        "v1 access handle must be purged from SecretStore after re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v1)
            .await
            .unwrap()
            .is_none(),
        "v1 refresh handle must be purged from SecretStore after re-auth"
    );

    // New handles must remain.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v2)
            .await
            .unwrap()
            .is_some(),
        "v2 access handle must be present in SecretStore after re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v2)
            .await
            .unwrap()
            .is_some(),
        "v2 refresh handle must be present in SecretStore after re-auth"
    );
}

// ─── fix: OAuth callback CAS-conflict re-read branch ─────────────────────────

#[tokio::test]
async fn filesystem_oauth_callback_cas_conflict_reuses_concurrent_account() {
    // Pre-create an account with the deterministic id that complete_oauth_callback
    // derives from flow_id (CredentialAccountId::from_uuid(flow_id.as_uuid())).
    // This simulates a concurrent callback that already created the account.
    // The CAS-conflict branch should re-read, validate, update, and succeed.
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

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
            opaque_state_hash: Some(state_hash("s2")),
            pkce_verifier_hash: Some(pkce_hash("p2")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Pre-seed the account with the deterministic id.
    let preseeded_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    service
        .create_account_with_id(
            preseeded_id,
            NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: Some(SecretHandle::new("pre-seeded-access").unwrap()),
                refresh_secret: None,
                scopes: vec![],
            },
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s2"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("p2"),
            },
        )
        .await
        .unwrap();

    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s2"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c2"),
                        pkce_verifier_hash: pkce_hash("p2"),
                        access_secret: SecretHandle::new("new-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("new-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        completed.credential_account_id,
        Some(preseeded_id),
        "CAS-conflict branch must reuse the pre-seeded account id"
    );
    assert_eq!(completed.status, AuthFlowStatus::Completed);
}

// ─── fix: abbyshekit review — OAuth CAS-conflict branch purges old secrets ───

#[tokio::test]
async fn filesystem_oauth_cas_conflict_branch_purges_previous_secrets() {
    // When the None-path CAS-conflict branch re-reads and overwrites an existing
    // account, the previous access/refresh secret handles must be deleted from
    // SecretStore so repeated re-auths do not accumulate dead handles.
    use ironclaw_auth::ProviderCallbackOutcome;
    use ironclaw_secrets::SecretMaterial;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

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
            opaque_state_hash: Some(state_hash("cas-s")),
            pkce_verifier_hash: Some(pkce_hash("cas-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Pre-seed the account with old secrets.
    let preseeded_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    let old_access = SecretHandle::new("old-access").unwrap();
    let old_refresh = SecretHandle::new("old-refresh").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            old_access.clone(),
            SecretMaterial::from("old-access-token"),
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            old_refresh.clone(),
            SecretMaterial::from("old-refresh-token"),
        )
        .await
        .unwrap();

    service
        .create_account_with_id(
            preseeded_id,
            NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: Some(old_access.clone()),
                refresh_secret: Some(old_refresh.clone()),
                scopes: vec![],
            },
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("cas-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("cas-p"),
            },
        )
        .await
        .unwrap();

    let new_access = SecretHandle::new("new-access").unwrap();
    let new_refresh = SecretHandle::new("new-refresh").unwrap();
    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("cas-s"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("cas-c"),
                        pkce_verifier_hash: pkce_hash("cas-p"),
                        access_secret: new_access.clone(),
                        refresh_secret: Some(new_refresh.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        completed.credential_account_id,
        Some(preseeded_id),
        "CAS-conflict branch must reuse pre-seeded account"
    );

    // Old secrets must be purged from SecretStore.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_access)
            .await
            .unwrap()
            .is_none(),
        "old access secret must be purged after CAS-conflict update"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_refresh)
            .await
            .unwrap()
            .is_none(),
        "old refresh secret must be purged after CAS-conflict update"
    );
}
