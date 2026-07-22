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

#[tokio::test]
async fn auth_flow_wire_writes_only_the_canonical_state_and_resolution_marker() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("wire");
    let flow = oauth_flow(&services, owner.clone()).await;
    let resolved = services
        .cancel_flow(&owner, flow.id)
        .await
        .expect("flow resolves as user-aborted");
    let delivered_at = "2026-07-20T12:00:00Z".parse().expect("timestamp");
    let resolved = services
        .mark_resolution_delivered(&owner, resolved.id, delivered_at)
        .await
        .expect("resolution delivery is acknowledged");
    let record = serde_json::to_value(resolved).expect("resolved flow serializes");
    let object = record.as_object().expect("flow record is an object");
    assert_eq!(object.get("state"), Some(&serde_json::json!("resolved")));
    assert_eq!(
        object.get("outcome"),
        Some(&serde_json::json!({"type": "user_aborted"}))
    );
    assert_eq!(
        object.get("resolution_delivered_at"),
        Some(&serde_json::json!("2026-07-20T12:00:00Z"))
    );
    for removed in [
        "status",
        "credential_account_id",
        "error",
        "continuation_emitted_at",
    ] {
        assert!(
            !object.contains_key(removed),
            "legacy field {removed} was written"
        );
    }

    let decoded: ironclaw_auth::AuthFlowRecord =
        serde_json::from_value(record).expect("canonical resolved flow decodes");
    assert_eq!(
        decoded.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
    );
}

#[tokio::test]
async fn mixed_canonical_and_legacy_lifecycle_fields_fail_closed() {
    let services = InMemoryAuthProductServices::new();
    let canonical = serde_json::to_value(oauth_flow(&services, scope("mixed-wire")).await)
        .expect("sample flow serializes");
    let base = canonical.as_object().expect("record object");

    let mut consistent = base.clone();
    consistent.insert("status".to_string(), serde_json::json!("awaiting_user"));
    assert!(
        serde_json::from_value::<ironclaw_auth::AuthFlowRecord>(serde_json::Value::Object(
            consistent
        ))
        .is_err(),
        "even equivalent canonical and legacy lifecycle fields are ambiguous"
    );

    let mut contradictory = base.clone();
    contradictory.insert("state".to_string(), serde_json::json!("resolved"));
    contradictory.insert(
        "outcome".to_string(),
        serde_json::json!({"type": "provider_denied"}),
    );
    contradictory.insert("status".to_string(), serde_json::json!("completed"));
    contradictory.insert(
        "credential_account_id".to_string(),
        serde_json::json!(CredentialAccountId::new()),
    );
    assert!(
        serde_json::from_value::<ironclaw_auth::AuthFlowRecord>(serde_json::Value::Object(
            contradictory
        ))
        .is_err(),
        "contradictory terminal lifecycle representations must fail closed"
    );

    let mut malformed_canonical_with_valid_legacy = base.clone();
    malformed_canonical_with_valid_legacy
        .insert("state".to_string(), serde_json::json!("resolved"));
    malformed_canonical_with_valid_legacy.remove("outcome");
    malformed_canonical_with_valid_legacy
        .insert("status".to_string(), serde_json::json!("canceled"));
    assert!(
        serde_json::from_value::<ironclaw_auth::AuthFlowRecord>(serde_json::Value::Object(
            malformed_canonical_with_valid_legacy
        ))
        .is_err(),
        "a malformed canonical lifecycle cannot fall through to valid legacy fields"
    );

    let mut neither = base.clone();
    neither.remove("state");
    neither.remove("outcome");
    assert!(
        serde_json::from_value::<ironclaw_auth::AuthFlowRecord>(serde_json::Value::Object(neither))
            .is_err(),
        "a record with neither lifecycle representation must fail closed"
    );
}

#[tokio::test]
async fn legacy_auth_flow_wire_decodes_into_the_canonical_model_and_fails_closed() {
    let services = InMemoryAuthProductServices::new();
    let canonical = serde_json::to_value(oauth_flow(&services, scope("legacy-wire")).await)
        .expect("sample flow serializes");
    let mut legacy = canonical.as_object().expect("record object").clone();
    legacy.remove("state");
    legacy.remove("outcome");
    legacy.remove("resolution_delivered_at");
    legacy.insert("status".to_string(), serde_json::json!("failed"));
    legacy.insert("error".to_string(), serde_json::json!("provider_denied"));
    legacy.insert(
        "continuation_emitted_at".to_string(),
        serde_json::json!("2026-07-20T12:00:00Z"),
    );
    let migrated: ironclaw_auth::AuthFlowRecord =
        serde_json::from_value(serde_json::Value::Object(legacy.clone()))
            .expect("legacy provider denial decodes");
    assert_eq!(
        migrated.state,
        AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied)
    );
    assert_eq!(
        migrated.resolution_delivered_at,
        Some("2026-07-20T12:00:00Z".parse().expect("timestamp"))
    );

    legacy.insert("status".to_string(), serde_json::json!("completed"));
    legacy.remove("error");
    assert!(
        serde_json::from_value::<ironclaw_auth::AuthFlowRecord>(serde_json::Value::Object(legacy))
            .is_err(),
        "legacy completed flow without an account must fail closed"
    );
}

#[tokio::test]
async fn legacy_failed_lifecycle_delivery_preserves_the_authorized_account() {
    let services = InMemoryAuthProductServices::new();
    let canonical = serde_json::to_value(oauth_flow(&services, scope("legacy-lifecycle")).await)
        .expect("sample flow serializes");
    let account_id = CredentialAccountId::new();
    let mut legacy = canonical.as_object().expect("record object").clone();
    legacy.remove("state");
    legacy.remove("outcome");
    legacy.remove("resolution_delivered_at");
    legacy.insert("status".to_string(), serde_json::json!("failed"));
    legacy.insert(
        "error".to_string(),
        serde_json::json!(AuthErrorCode::BackendUnavailable),
    );
    legacy.insert(
        "credential_account_id".to_string(),
        serde_json::json!(account_id),
    );
    legacy.insert(
        "credential_secret_fingerprint".to_string(),
        serde_json::json!("c".repeat(64)),
    );
    legacy.insert(
        "continuation".to_string(),
        serde_json::json!({
            "type": "lifecycle_activation",
            "package_ref": "slack"
        }),
    );

    let migrated: ironclaw_auth::AuthFlowRecord =
        serde_json::from_value(serde_json::Value::Object(legacy))
            .expect("legacy committed lifecycle failure decodes");

    assert_eq!(
        migrated.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized { account_id }),
        "the provider authorization succeeded; only its old delivery protocol failed"
    );
    assert!(migrated.resolution_delivered_at.is_none());
}

#[tokio::test]
async fn every_supported_legacy_lifecycle_shape_maps_to_one_canonical_state() {
    let services = InMemoryAuthProductServices::new();
    let canonical = serde_json::to_value(oauth_flow(&services, scope("legacy-matrix")).await)
        .expect("sample flow serializes");
    let base = canonical.as_object().expect("record object").clone();
    let account_id = CredentialAccountId::new();
    let cases = [
        ("pending", None, None, AuthFlowState::Open),
        ("awaiting_user", None, None, AuthFlowState::Open),
        ("callback_received", None, None, AuthFlowState::Processing),
        ("completing", None, None, AuthFlowState::Processing),
        (
            "completing",
            Some(account_id),
            None,
            AuthFlowState::Processing,
        ),
        (
            "completed",
            Some(account_id),
            None,
            AuthFlowState::Resolved(AuthFlowOutcome::Authorized { account_id }),
        ),
        (
            "failed",
            None,
            Some(AuthErrorCode::ProviderDenied),
            AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied),
        ),
        (
            "failed",
            None,
            Some(AuthErrorCode::BackendUnavailable),
            AuthFlowState::Resolved(AuthFlowOutcome::Failed {
                error: AuthErrorCode::BackendUnavailable,
            }),
        ),
        (
            "expired",
            None,
            None,
            AuthFlowState::Resolved(AuthFlowOutcome::Expired),
        ),
        (
            "canceling",
            None,
            None,
            AuthFlowState::Resolved(AuthFlowOutcome::UserAborted),
        ),
        (
            "canceled",
            None,
            None,
            AuthFlowState::Resolved(AuthFlowOutcome::UserAborted),
        ),
    ];

    for (status, legacy_account, error, expected) in cases {
        let mut wire = base.clone();
        wire.remove("state");
        wire.remove("outcome");
        wire.remove("resolution_delivered_at");
        wire.insert("status".to_string(), serde_json::json!(status));
        if let Some(account_id) = legacy_account {
            wire.insert(
                "credential_account_id".to_string(),
                serde_json::json!(account_id),
            );
        }
        if let Some(error) = error {
            wire.insert("error".to_string(), serde_json::json!(error));
        }
        let migrated: ironclaw_auth::AuthFlowRecord =
            serde_json::from_value(serde_json::Value::Object(wire))
                .unwrap_or_else(|error| panic!("legacy {status} must decode: {error}"));
        assert_eq!(migrated.state, expected, "legacy {status}");
    }
}

#[tokio::test]
async fn resolution_delivery_marker_is_terminal_only_and_idempotent() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;
    let open_error = services
        .mark_resolution_delivered(&owner, flow.id, Utc::now())
        .await
        .expect_err("open flow cannot be marked delivered");
    assert_eq!(open_error, AuthProductError::FlowAlreadyTerminal);

    let resolved = services
        .cancel_flow(&owner, flow.id)
        .await
        .expect("flow resolves");
    let first_delivery = Utc::now();
    let delivered = services
        .mark_resolution_delivered(&owner, resolved.id, first_delivery)
        .await
        .expect("resolved flow can be marked delivered");
    assert_eq!(delivered.resolution_delivered_at, Some(first_delivery));

    let replay = services
        .mark_resolution_delivered(&owner, resolved.id, first_delivery + Duration::seconds(1))
        .await
        .expect("delivery marker is idempotent");
    assert_eq!(replay.resolution_delivered_at, Some(first_delivery));
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

    assert!(matches!(
        completed.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized { .. })
    ));
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

    assert_eq!(
        completed.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
            account_id: account.id,
        })
    );
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
    assert_eq!(replay.state, completed.state);
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

    assert_eq!(
        completed.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
            account_id: existing.id,
        })
    );
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

    assert_eq!(
        completed.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
            account_id: existing.id,
        })
    );
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
    assert_eq!(
        canceled.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
    );

    let second_cancel = services
        .cancel_flow(&owner, flow.id)
        .await
        .expect("identical terminal cancel is idempotent");
    assert_eq!(second_cancel, canceled);

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
async fn terminal_flow_state_is_not_rewritten_after_expiry() {
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
    assert_eq!(
        record.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
    );
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
    assert_eq!(
        record.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Expired)
    );
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

/// Create a setup flow with an explicit continuation and provider — the shape
/// the web "Connect" and extension-card connect buttons both mint.
async fn setup_flow_with(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
    flow_provider: AuthProviderId,
    continuation: AuthContinuationRef,
) -> ironclaw_auth::AuthFlowRecord {
    services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: flow_provider,
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
        .expect("setup flow")
}

fn turn_gate_continuation(gate: &str) -> AuthContinuationRef {
    AuthContinuationRef::TurnGateResume {
        turn_run_ref: TurnRunRef::new(uuid::Uuid::new_v4().to_string()).expect("turn run ref"),
        gate_ref: AuthGateRef::new(gate).expect("gate ref"),
    }
}

fn lifecycle_continuation() -> AuthContinuationRef {
    AuthContinuationRef::LifecycleActivation {
        package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
    }
}

async fn flow_state(
    services: &InMemoryAuthProductServices,
    owner: &AuthProductScope,
    flow_id: ironclaw_auth::AuthFlowId,
) -> AuthFlowState {
    services
        .get_flow(owner, flow_id)
        .await
        .expect("lookup")
        .expect("record")
        .state
}

/// Supersede-on-start lives INSIDE `create_flow`: minting a setup-class flow
/// is itself the seam that cancels the prior live setup-class flows for the
/// same owner root + provider, so no start route can forget to supersede
/// (the #6130 DCR/Notion gap class becomes unrepresentable). Gate flows, other
/// providers, and other owners are bystanders the creation must not disturb.
#[tokio::test]
async fn create_flow_supersedes_prior_live_setup_class_flows() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let bob = scope("bob");
    let other_provider = AuthProviderId::new("gmail").expect("valid provider");

    let setup_only = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        AuthContinuationRef::SetupOnly,
    )
    .await;
    let lifecycle = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        lifecycle_continuation(),
    )
    .await;
    let turn_gate = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        turn_gate_continuation("gate:parked-turn"),
    )
    .await;
    let other_prov = setup_flow_with(
        &services,
        owner.clone(),
        other_provider,
        AuthContinuationRef::SetupOnly,
    )
    .await;
    let other_owner = setup_flow_with(
        &services,
        bob.clone(),
        provider(),
        AuthContinuationRef::SetupOnly,
    )
    .await;

    // The re-opened "Connect" popup mints its flow: creation supersedes.
    let reopened = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        AuthContinuationRef::SetupOnly,
    )
    .await;

    assert_eq!(
        flow_state(&services, &owner, reopened.id).await,
        AuthFlowState::Open,
        "the freshly created flow must not supersede itself"
    );
    assert_eq!(
        flow_state(&services, &owner, setup_only.id).await,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted),
        "creating a setup-class flow must cancel the prior SetupOnly flow"
    );
    assert_eq!(
        flow_state(&services, &owner, lifecycle.id).await,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted),
        "creating a setup-class flow must cancel the prior LifecycleActivation flow"
    );
    assert_eq!(
        flow_state(&services, &owner, turn_gate.id).await,
        AuthFlowState::Open,
        "a parked turn's auth gate is not a setup flow and must survive creation"
    );
    assert_eq!(
        flow_state(&services, &owner, other_prov.id).await,
        AuthFlowState::Open
    );
    assert_eq!(
        flow_state(&services, &bob, other_owner.id).await,
        AuthFlowState::Open
    );
}

/// The exclusion that keeps parked turns alive cuts both ways: creating a
/// `TurnGateResume` flow is not a setup start, so it must not cancel a live
/// setup-class flow for the same owner+provider (and vice versa is pinned
/// above). Both classes stay live side by side.
#[tokio::test]
async fn create_flow_for_a_parked_turn_gate_does_not_supersede_setup_flows() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    let setup_only = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        AuthContinuationRef::SetupOnly,
    )
    .await;
    let turn_gate = setup_flow_with(
        &services,
        owner.clone(),
        provider(),
        turn_gate_continuation("gate:parked-turn"),
    )
    .await;

    assert_eq!(
        flow_state(&services, &owner, setup_only.id).await,
        AuthFlowState::Open,
        "a gate flow's creation must never cancel the setup surface's flow"
    );
    assert_eq!(
        flow_state(&services, &owner, turn_gate.id).await,
        AuthFlowState::Open
    );
}

/// The `create_flow` supersede contract must hold under CONCURRENCY, not just
/// sequentially: two Connect clicks racing each other must still leave
/// exactly one live setup-class flow. The cancel walk and the insert happen
/// under one state lock, so no interleaving can let two creates each observe
/// "no live predecessor" and both survive. Multi-threaded runtime + a start
/// barrier + repeated rounds to actually exercise the interleavings.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_setup_creates_leave_exactly_one_live_flow() {
    for round in 0..20 {
        let services = std::sync::Arc::new(InMemoryAuthProductServices::new());
        let owner = scope("alice");
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
        let mut racers = Vec::new();
        for _ in 0..8 {
            let services = std::sync::Arc::clone(&services);
            let owner = owner.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            racers.push(tokio::spawn(async move {
                barrier.wait().await;
                setup_flow_with(&services, owner, provider(), AuthContinuationRef::SetupOnly).await
            }));
        }
        for racer in racers {
            racer.await.expect("racer task completes");
        }
        let live = services
            .flow_records_snapshot()
            .into_iter()
            .filter(|flow| flow.state == AuthFlowState::Open)
            .count();
        assert_eq!(
            live, 1,
            "round {round}: concurrent setup creates must leave exactly one live flow"
        );
    }
}
