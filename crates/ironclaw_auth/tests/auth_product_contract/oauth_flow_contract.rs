use crate::common::*;

/// Cross-implementation conformance: the shared OAuth-callback state-machine
/// suite (`ironclaw_auth::test_support::conformance`) holds for the in-memory fake. The
/// durable `FilesystemAuthProductServices` runs the SAME suite from the root
/// `tests/integration/oauth_connect.rs` — together they turn the two
/// implementations' agreement into an enforced contract instead of a
/// coincidence of two disjoint test suites.
#[tokio::test]
async fn oauth_flow_state_machine_conformance_holds_for_in_memory_fake() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("conformance");
    ironclaw_auth::test_support::conformance::assert_auth_flow_callback_conformance(
        &services,
        &owner,
        &provider(),
    )
    .await;
}

async fn completed_oauth_flow_with_continuation(
    services: &InMemoryAuthProductServices,
    owner: &AuthProductScope,
    continuation: AuthContinuationRef,
) -> ironclaw_auth::AuthFlowRecord {
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("OAuth flow");

    services
        .complete_oauth_callback(
            owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("work github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new(format!("github-access-{}", flow.id))
                            .expect("access handle"),
                        refresh_secret: None,
                        scopes: Vec::new(),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("OAuth completion")
}

#[tokio::test]
async fn continuation_side_effect_failure_terminalizes_only_the_expected_completion() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;
    let completed = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("work github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: Vec::new(),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("OAuth completion");

    let claimed = services
        .claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("continuation claim");
    let failed = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claimed.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect("terminal continuation failure");
    assert_eq!(failed.status, AuthFlowStatus::Failed);
    assert_eq!(failed.error, Some(AuthErrorCode::BackendUnavailable));
    assert!(failed.continuation_emitted_at.is_none());

    let stale = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claimed.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect_err("stale failure cannot overwrite terminal state");
    assert_eq!(stale, AuthProductError::FlowAlreadyTerminal);
}

#[tokio::test]
async fn concurrent_continuation_dispatch_claims_have_exactly_one_active_owner() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let completed = completed_oauth_flow_with_continuation(
        &services,
        &owner,
        AuthContinuationRef::LifecycleActivation {
            package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
        },
    )
    .await;
    let first_claimed_at = Utc::now();
    let second_claimed_at = first_claimed_at + Duration::milliseconds(1);

    let (first, second) = tokio::join!(
        services.claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: first_claimed_at,
            },
        ),
        services.claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: second_claimed_at,
            },
        )
    );

    let (winner, loser_claimed_at, loser_error) = match (first, second) {
        (Ok(winner), Err(error)) => (winner, second_claimed_at, error),
        (Err(error), Ok(winner)) => (winner, first_claimed_at, error),
        (first, second) => panic!(
            "exactly one concurrent claim must own dispatch; first={first:?}, second={second:?}"
        ),
    };
    assert_eq!(loser_error, AuthProductError::BackendUnavailable);
    assert_eq!(winner.status, AuthFlowStatus::Completing);

    let stale_loser = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: loser_claimed_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: winner.updated_at + Duration::milliseconds(1),
                },
            },
        )
        .await
        .expect_err("a blocked claimant cannot settle another owner's claim");
    assert_eq!(stale_loser, AuthProductError::FlowAlreadyTerminal);

    let settled = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: winner.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: winner.updated_at + Duration::milliseconds(1),
                },
            },
        )
        .await
        .expect("the active claim owner settles dispatch");
    assert_eq!(settled.status, AuthFlowStatus::Completed);
    assert!(settled.continuation_emitted_at.is_some());
}

#[tokio::test]
async fn stale_continuation_dispatch_claim_can_be_reclaimed_and_fences_old_owner() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let completed = completed_oauth_flow_with_continuation(
        &services,
        &owner,
        AuthContinuationRef::LifecycleActivation {
            package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
        },
    )
    .await;
    let first = services
        .claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("first continuation claim");
    let reclaimed_at = first.updated_at
        + Duration::seconds(ironclaw_auth::AUTH_CONTINUATION_DISPATCH_LEASE_SECONDS);
    let reclaimed = services
        .claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: reclaimed_at,
            },
        )
        .await
        .expect("stale continuation claim is reclaimable");
    assert_eq!(reclaimed.status, AuthFlowStatus::Completing);
    assert_eq!(reclaimed.updated_at, reclaimed_at);

    let stale_settlement = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: first.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: reclaimed_at + Duration::milliseconds(1),
                },
            },
        )
        .await
        .expect_err("the reclaimed lease fences the old owner");
    assert_eq!(stale_settlement, AuthProductError::FlowAlreadyTerminal);

    let settled = services
        .settle_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: reclaimed.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: reclaimed_at + Duration::milliseconds(1),
                },
            },
        )
        .await
        .expect("reclaiming owner settles dispatch");
    assert_eq!(settled.status, AuthFlowStatus::Completed);
}

#[tokio::test]
async fn non_lifecycle_continuation_dispatch_claim_is_rejected_without_mutation() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let completed =
        completed_oauth_flow_with_continuation(&services, &owner, AuthContinuationRef::SetupOnly)
            .await;

    let error = services
        .claim_continuation_dispatch(
            &owner,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect_err("only lifecycle activation may claim exclusive dispatch");
    assert_eq!(error, AuthProductError::FlowAlreadyTerminal);

    let persisted = services
        .get_flow(&owner, completed.id)
        .await
        .expect("flow lookup")
        .expect("completed flow remains");
    assert_eq!(persisted.status, AuthFlowStatus::Completed);
    assert_eq!(persisted.updated_at, completed.updated_at);
    assert!(persisted.continuation_emitted_at.is_none());
}

#[tokio::test]
async fn oauth_callback_exchanges_provider_code_then_completes_once() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;

    let request = OAuthProviderCallbackRequest {
        provider: provider(),
        account_label: label("work github"),
        authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
            .expect("valid code"),
        authorization_code_hash: code_hash("code-hash"),
        pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
            .expect("valid verifier"),
        pkce_verifier_hash: pkce_hash("pkce-hash"),
        scopes: provider_scopes(&["repo"]),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("raw-auth-code"));
    assert!(!debug.contains("raw-pkce-verifier"));

    let exchange = services
        .exchange_callback(
            OAuthProviderExchangeContext {
                scope: owner.clone(),
                flow_id: flow.id,
            },
            request,
        )
        .await
        .expect("provider exchange");
    let completed = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(exchange),
                },
            },
        )
        .await
        .expect("callback completes");

    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.credential_account_id.is_some());
    assert_eq!(services.continuations().len(), 1);

    let replay = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("terminal flow rejects callback replay");
    assert_eq!(replay, AuthProductError::FlowAlreadyTerminal);
    assert_eq!(services.continuations().len(), 1);
}

#[tokio::test]
async fn credential_selection_completes_account_selection_flow_once() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work github"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-work-secret").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("account");
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: provider(),
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow");

    let completed = services
        .complete_credential_selection(
            &owner,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect("credential selection completes");

    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(completed.credential_account_id, Some(account.id));
    assert_eq!(services.continuations().len(), 1);

    let replay = services
        .complete_credential_selection(
            &owner,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect("matching completed selection is idempotent");
    assert_eq!(replay.credential_account_id, Some(account.id));
    assert_eq!(services.continuations().len(), 1);
}

#[tokio::test]
async fn auth_flow_record_source_returns_stable_sorted_snapshot() {
    let services = InMemoryAuthProductServices::new();
    let alice = oauth_flow(&services, scope("alice")).await;
    let bob = oauth_flow(&services, scope("bob")).await;

    let snapshot = services.flow_records_snapshot();

    let ids = snapshot.iter().map(|flow| flow.id).collect::<Vec<_>>();
    let mut sorted = ids.clone();
    sorted.sort_by_key(|id| id.as_uuid());
    assert_eq!(ids, sorted);
    assert!(ids.contains(&alice.id));
    assert!(ids.contains(&bob.id));
}

#[tokio::test]
async fn credential_selection_rejects_unlisted_or_cross_scope_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work github"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-work-secret").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("account");
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: provider(),
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow");

    let unlisted = services
        .complete_credential_selection(
            &owner,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: CredentialAccountId::new(),
            },
        )
        .await
        .expect_err("unlisted account rejected");
    assert_eq!(unlisted, AuthProductError::CredentialMissing);

    let cross_scope = services
        .complete_credential_selection(
            &scope("bob"),
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect_err("cross-scope selection rejected");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);
    assert!(services.continuations().is_empty());
}

#[tokio::test]
async fn oauth_callback_updates_existing_account_from_provider_exchange() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let existing = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work github"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-old-access").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["read:user"]),
        })
        .await
        .expect("existing account");
    let flow = oauth_update_flow(&services, owner.clone(), &existing).await;
    let access_secret = SecretHandle::new("github-new-access").unwrap();
    let refresh_secret = SecretHandle::new("github-new-refresh").unwrap();

    let completed = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("renamed github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: access_secret.clone(),
                        refresh_secret: Some(refresh_secret.clone()),
                        scopes: provider_scopes(&["repo", "workflow"]),
                        account_id: Some(existing.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("callback updates account");

    assert_eq!(completed.credential_account_id, Some(existing.id));
    let updated = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            existing.id,
        ))
        .await
        .expect("lookup")
        .expect("updated account");
    assert_eq!(updated.id, existing.id);
    assert_eq!(updated.created_at, existing.created_at);
    assert_eq!(updated.label, label("renamed github"));
    assert_eq!(updated.status, CredentialAccountStatus::Configured);
    assert_eq!(updated.access_secret, Some(access_secret));
    assert_eq!(updated.refresh_secret, Some(refresh_secret));
    assert_eq!(updated.scopes, provider_scopes(&["repo", "workflow"]));
}

#[tokio::test]
async fn oauth_callback_with_no_provider_account_id_updates_bound_account_across_thread() {
    // Regression (#4935, fake fidelity): a provider exchange that returns NO
    // account_id but whose flow carries an update_binding is a reconnect of the
    // bound account, and must update it at owner granularity — exactly as the
    // durable production callback (`update_bound_oauth_account`) does. The
    // in-memory fake previously routed `account_id: None` straight to
    // create-account (rejecting the binding), so tests could not exercise the
    // production reconnect contract. This drives that path across a different
    // thread than the account was created in.
    let services = InMemoryAuthProductServices::new();
    let create_scope = scope("alice");
    let existing = services
        .create_account(NewCredentialAccount {
            scope: create_scope.clone(),
            provider: provider(),
            label: label("work github"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-old-access").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["read:user"]),
        })
        .await
        .expect("existing account");

    let reauth_scope = reconnect_scope("alice", "thread-reauth");
    let flow = oauth_update_flow(&services, reauth_scope.clone(), &existing).await;
    let access_secret = SecretHandle::new("github-new-access").unwrap();

    let completed = services
        .complete_oauth_callback(
            &reauth_scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("renamed github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: access_secret.clone(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("no-account-id reconnect must update the bound account, not fork");

    assert_eq!(completed.credential_account_id, Some(existing.id));
    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(create_scope, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, existing.id);
}

#[tokio::test]
async fn oauth_callback_rejects_mismatched_provider_and_invalid_existing_account_exchange() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let foreign_owner = scope("bob");
    let existing = services
        .create_account(account_request(
            owner.clone(),
            "work github",
            CredentialAccountStatus::PendingSetup,
        ))
        .await
        .expect("owner account");
    let foreign = services
        .create_account(account_request(
            foreign_owner,
            "foreign github",
            CredentialAccountStatus::PendingSetup,
        ))
        .await
        .expect("foreign account");
    let gitlab = AuthProviderId::new("gitlab").expect("valid provider");
    let mut provider_mismatch_request = account_request(
        owner.clone(),
        "gitlab account",
        CredentialAccountStatus::PendingSetup,
    );
    provider_mismatch_request.provider = gitlab.clone();
    let provider_mismatch = services
        .create_account(provider_mismatch_request)
        .await
        .expect("other provider account");

    let provider_mismatch_flow = oauth_flow(&services, owner.clone()).await;
    let provider_mismatch_err = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: provider_mismatch_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: gitlab.clone(),
                        account_label: label("gitlab"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("gitlab-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["read_user"]),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect_err("flow provider must match exchange provider");
    assert_eq!(provider_mismatch_err, AuthProductError::TokenExchangeFailed);

    let unbound_account_flow = oauth_flow(&services, owner.clone()).await;
    let unbound_account_err = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: unbound_account_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("missing"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("missing-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(existing.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect_err("unbound account id is rejected");
    assert_eq!(unbound_account_err, AuthProductError::CrossScopeDenied);

    let cross_scope_flow = oauth_update_flow(&services, owner.clone(), &existing).await;
    let cross_scope_err = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: cross_scope_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("foreign"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("foreign-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(foreign.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect_err("callback account id must match bound update target");
    assert_eq!(cross_scope_err, AuthProductError::CrossScopeDenied);

    let unbound_provider_mismatch_flow = oauth_flow(&services, owner.clone()).await;
    let unbound_provider_mismatch_err = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: unbound_provider_mismatch_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("wrong provider account"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(provider_mismatch.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect_err("unbound provider-mismatch account id is rejected");
    assert_eq!(
        unbound_provider_mismatch_err,
        AuthProductError::CrossScopeDenied
    );

    let valid_update_flow = oauth_update_flow(&services, owner.clone(), &existing).await;
    services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: valid_update_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("renamed github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(existing.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("valid existing account update still works");
}

#[tokio::test]
async fn create_flow_rejects_invalid_update_binding() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let foreign_owner = scope("bob");
    let existing = services
        .create_account(account_request(
            owner.clone(),
            "work github",
            CredentialAccountStatus::PendingSetup,
        ))
        .await
        .expect("owner account");
    let foreign = services
        .create_account(account_request(
            foreign_owner,
            "foreign github",
            CredentialAccountStatus::PendingSetup,
        ))
        .await
        .expect("foreign account");
    let gitlab = AuthProviderId::new("gitlab").expect("valid provider");
    let mut provider_mismatch_request = account_request(
        owner.clone(),
        "gitlab account",
        CredentialAccountStatus::PendingSetup,
    );
    provider_mismatch_request.provider = gitlab.clone();
    let provider_mismatch = services
        .create_account(provider_mismatch_request)
        .await
        .expect("provider mismatch account");

    let missing = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id: ironclaw_auth::CredentialAccountId::new(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
            }),
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("missing update target is rejected");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let cross_scope = try_oauth_update_flow(&services, owner.clone(), &foreign)
        .await
        .expect_err("cross-scope update target is rejected at create time");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let provider_mismatch_err = try_oauth_update_flow(&services, owner.clone(), &provider_mismatch)
        .await
        .expect_err("provider mismatch is rejected at create time");
    assert_eq!(provider_mismatch_err.code(), AuthErrorCode::InvalidRequest);

    let attacker_binding = CredentialAccountUpdateBinding {
        account_id: existing.id,
        ownership: CredentialOwnership::ExtensionOwned,
        owner_extension: Some(ExtensionId::new("attacker").unwrap()),
        granted_extensions: Vec::new(),
    };
    let authority_mismatch = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(attacker_binding),
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("authority mismatch is rejected at create time");
    assert_eq!(authority_mismatch, AuthProductError::CrossScopeDenied);
}

#[tokio::test]
async fn oauth_callback_rejects_cross_scope_stale_malformed_and_denied() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;

    let cross_scope = services
        .complete_oauth_callback(
            &scope("bob"),
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("foreign scope denied");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let wrong_state = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("other-state"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("wrong state denied");
    assert_eq!(wrong_state, AuthProductError::CrossScopeDenied);

    let wrong_pkce = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("work github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("other-pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect_err("pkce verifier hash must match stored flow hash");
    assert_eq!(wrong_pkce, AuthProductError::CrossScopeDenied);

    let malformed_code = OAuthAuthorizationCode::new(secret("   "))
        .expect_err("empty raw code is malformed before exchange");
    assert_eq!(malformed_code.code(), AuthErrorCode::InvalidRequest);
    let padded_verifier = PkceVerifierSecret::new(secret(" verifier "))
        .expect_err("raw verifier must be caller-clean");
    assert_eq!(padded_verifier.code(), AuthErrorCode::InvalidRequest);

    let denied = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("provider denied");
    assert_eq!(denied, AuthProductError::ProviderDenied);
}

#[tokio::test]
async fn cancel_flow_preserves_terminal_state_and_blocks_callback() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;

    let canceled = services
        .cancel_flow(&owner, flow.id)
        .await
        .expect("owner cancel");
    assert_eq!(canceled.status, AuthFlowStatus::Canceled);

    let second_cancel = services
        .cancel_flow(&owner, flow.id)
        .await
        .expect_err("terminal cancel rejected");
    assert_eq!(second_cancel, AuthProductError::Canceled);

    let callback = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("callback after cancel rejected");
    assert_eq!(callback, AuthProductError::Canceled);
}

#[tokio::test]
async fn terminal_flow_status_is_not_rewritten_after_expiry() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() - Duration::seconds(1),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .expect("expired flow");
    services
        .cancel_flow(&owner, flow.id)
        .await
        .expect("terminal cancel");

    let callback = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("terminal status wins over expiry");
    assert_eq!(callback, AuthProductError::Canceled);
    let record = services
        .get_flow(&owner, flow.id)
        .await
        .expect("lookup")
        .expect("flow remains");
    assert_eq!(record.status, AuthFlowStatus::Canceled);
}

#[tokio::test]
async fn oauth_callback_marks_expired_flow_and_rejects_completion() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() - Duration::seconds(1),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .expect("expired flow");

    let expired = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("expired flow rejects completion");
    assert_eq!(expired, AuthProductError::UnknownOrExpiredFlow);
    let record = services
        .get_flow(&owner, flow.id)
        .await
        .expect("lookup")
        .expect("flow remains");
    assert_eq!(record.status, AuthFlowStatus::Expired);
    assert_eq!(record.error, Some(AuthErrorCode::UnknownOrExpiredFlow));
}

#[tokio::test]
async fn get_flow_returns_none_owner_record_and_cross_scope_denial() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;

    let found = services
        .get_flow(&owner, flow.id)
        .await
        .expect("lookup")
        .expect("record");
    assert_eq!(found.id, flow.id);
    assert!(
        services
            .get_flow(&owner, ironclaw_auth::AuthFlowId::new())
            .await
            .expect("missing lookup")
            .is_none()
    );
    let cross_scope = services
        .get_flow(&scope("bob"), flow.id)
        .await
        .expect_err("cross scope");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);
}
