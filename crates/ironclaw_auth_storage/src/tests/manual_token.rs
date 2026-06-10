use super::*;

#[tokio::test]
async fn filesystem_manual_token_submit_stores_secret_and_dedupes_replay() {
    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected manual token challenge");
    };

    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.status, CredentialAccountStatus::Configured);

    let account = service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            result.account_id,
        ))
        .await
        .unwrap()
        .expect("manual token submit should create account");
    let access_secret = account.access_secret.expect("manual token secret handle");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_secret)
            .await
            .unwrap()
            .is_some()
    );

    let replay = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .expect_err("manual token submit should be one-shot");
    assert_eq!(replay, AuthProductError::UnknownOrExpiredFlow);
}

#[tokio::test]
async fn filesystem_manual_token_submit_rotates_existing_reusable_account() {
    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let first_challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: first_interaction,
        ..
    } = first_challenge
    else {
        panic!("expected manual token challenge");
    };
    let first = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: first_interaction,
                secret: SecretString::from("first-manual-token"),
            },
        )
        .await
        .unwrap();
    let first_account = service
        .read_account(&scope, first.account_id)
        .await
        .unwrap()
        .expect("first account")
        .0;
    let first_handle = first_account.access_secret.expect("first secret handle");

    let second_challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: second_interaction,
        ..
    } = second_challenge
    else {
        panic!("expected manual token challenge");
    };
    let second = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: second_interaction,
                secret: SecretString::from("second-manual-token"),
            },
        )
        .await
        .unwrap();

    assert_eq!(second.account_id, first.account_id);
    let accounts = service.accounts_for_owner(&scope).await.unwrap();
    assert_eq!(accounts.len(), 1);
    let updated = accounts.into_iter().next().unwrap();
    let second_handle = updated.access_secret.expect("second secret handle");
    assert_ne!(second_handle, first_handle);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &second_handle)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &first_handle)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_manual_token_completion_persists_auth_flow_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
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

    let completed = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: submitted.account_id,
            },
        )
        .await
        .unwrap();

    assert_eq!(completed.id, flow.id);
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(completed.credential_account_id, Some(submitted.account_id));
}

#[tokio::test]
async fn filesystem_manual_token_completion_rejects_invalid_completed_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() + Duration::minutes(5)).await;

    let missing = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: CredentialAccountId::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();
    let unconfigured = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: account.id,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unconfigured, AuthProductError::CrossScopeDenied);

    let mut foreign_scope = scope.clone();
    foreign_scope.resource.user_id = UserId::new("bob").unwrap();
    let foreign = service
        .create_account(NewCredentialAccount {
            scope: foreign_scope,
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("foreign-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();
    let cross_scope = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: foreign.id,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);
}

#[tokio::test]
async fn filesystem_manual_token_completion_expires_stale_auth_flow() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() - Duration::minutes(1)).await;

    let submitted = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(submitted, AuthProductError::UnknownOrExpiredFlow);

    let err = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: CredentialAccountId::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, AuthProductError::UnknownOrExpiredFlow);
    let flows = service.flows_for_scope(&scope).await.unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].0.status, AuthFlowStatus::Expired);
}

#[tokio::test]
async fn filesystem_manual_token_cancel_marks_flow_canceled_and_is_idempotent() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() + Duration::minutes(5)).await;

    let canceled = service
        .cancel_manual_token(&scope, interaction_id)
        .await
        .unwrap()
        .expect("manual-token flow should be canceled");
    assert_eq!(canceled.status, AuthFlowStatus::Canceled);
    let still_canceled = service
        .cancel_manual_token(&scope, interaction_id)
        .await
        .unwrap()
        .expect("terminal flow should still be returned");
    assert_eq!(still_canceled.status, AuthFlowStatus::Canceled);
    let unknown = service
        .cancel_manual_token(&scope, AuthInteractionId::new())
        .await
        .unwrap();
    assert!(unknown.is_none());
}

#[tokio::test]
async fn filesystem_manual_token_submit_allows_only_one_concurrent_consumer() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = Arc::new(test_service(filesystem, secret_store));

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected manual token challenge");
    };

    let mut tasks = JoinSet::new();
    for value in ["first-token", "second-token"] {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        tasks.spawn(async move {
            service
                .submit_manual_token(
                    &scope,
                    SecretSubmitRequest {
                        interaction_id,
                        secret: SecretString::from(value),
                    },
                )
                .await
        });
    }

    let mut successes = 0;
    let mut consumed_rejections = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(_) => successes += 1,
            Err(AuthProductError::UnknownOrExpiredFlow) => consumed_rejections += 1,
            Err(error) => panic!("unexpected submit error: {error:?}"),
        }
    }

    assert_eq!(successes, 1);
    assert_eq!(consumed_rejections, 1);
}

// ─── fix: manual-token submit cleans up secret on write failure ───────────────

#[tokio::test]
async fn filesystem_manual_token_rotation_removes_previous_secret() {
    // Tests the update_binding path in create_or_update_manual_token_account:
    // after a successful token rotation the OLD access secret must be purged
    // from SecretStore so it does not accumulate orphaned material.
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, ManualTokenSetupRequest, SecretSubmitRequest,
    };

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // --- First submit: create the account via the no-binding path. ---
    let challenge1 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: iid1,
        ..
    } = challenge1
    else {
        panic!("expected ManualTokenRequired");
    };
    let result1 = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: iid1,
                secret: SecretString::from("token-v1"),
            },
        )
        .await
        .unwrap();
    let account_id = result1.account_id;

    // Grab the first-generation secret handle.
    let account_after_v1 = service
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let old_handle = account_after_v1
        .access_secret
        .clone()
        .expect("v1 access_secret");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_handle)
            .await
            .unwrap()
            .is_some(),
        "v1 secret must exist in store"
    );

    // --- Second submit: rotate via update_binding to the same account. ---
    let challenge2 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: iid2,
        ..
    } = challenge2
    else {
        panic!("expected ManualTokenRequired for rotation");
    };
    service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: iid2,
                secret: SecretString::from("token-v2"),
            },
        )
        .await
        .unwrap();

    // The old handle must have been purged from SecretStore after the rotation.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_handle)
            .await
            .unwrap()
            .is_none(),
        "v1 secret must be purged from SecretStore after rotation"
    );

    // The new handle must be present.
    let account_after_v2 = service
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let new_handle = account_after_v2.access_secret.expect("v2 access_secret");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &new_handle)
            .await
            .unwrap()
            .is_some(),
        "v2 secret must be present in SecretStore"
    );
}

// ─── fix: manual-token expiry branch ─────────────────────────────────────────

#[tokio::test]
async fn filesystem_manual_token_submit_rejects_expired_interaction() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // Create an interaction that is already past its expiry.
    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            // Expired immediately.
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected ManualTokenRequired");
    };

    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("too-late"),
            },
        )
        .await
        .expect_err("expired interaction must be rejected");
    assert_eq!(
        err,
        AuthProductError::UnknownOrExpiredFlow,
        "expired interaction must return UnknownOrExpiredFlow"
    );
}

// ─── [High · tests] manual-token submit cleans up secret on account write fail

#[tokio::test]
async fn filesystem_manual_token_submit_cleans_up_secret_when_account_write_fails() {
    // create_or_update_manual_token_account (None path) stores the secret first,
    // then calls create_account_with_id(CasExpectation::Absent). If the write
    // fails the newly-stored secret must be deleted from SecretStore so it does
    // not orphan in the store.
    //
    // Failure injection: derive the account ID that submit_manual_token will use
    // (CredentialAccountId::from_uuid(interaction_id.as_uuid())) and write a
    // dummy record at that path before submitting, causing CasExpectation::Absent
    // to return VersionMismatch → BackendConflict.
    use ironclaw_auth::CredentialAccountId;
    use ironclaw_filesystem::CasExpectation;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // Request an interaction so we know its ID (and can derive the account path).
    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected ManualTokenRequired");
    };

    // Derive the same account ID the submit path will use.
    let account_id = CredentialAccountId::from_uuid(interaction_id.as_uuid());

    // Write a dummy record at that path so create_account_with_id(Absent) fails.
    let dummy_account = ironclaw_auth::CredentialAccount {
        id: account_id,
        scope: scope.clone(),
        provider: google_provider(),
        label: account_label(),
        status: ironclaw_auth::CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: vec![],
        access_secret: None,
        refresh_secret: None,
        scopes: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let path = super::paths::account_path(&scope, account_id)
        .expect("account path derivation must succeed");
    let json = serde_json::to_vec(&dummy_account).expect("serialization must succeed");
    use ironclaw_filesystem::{ContentType, Entry};
    let entry = Entry::bytes(json).with_content_type(ContentType::json());
    filesystem
        .put(&scope.resource, &path, entry, CasExpectation::Absent)
        .await
        .expect("pre-create dummy account must succeed");

    // Submit the token — account write will fail; cleanup must run.
    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("token-value"),
            },
        )
        .await;
    assert!(result.is_err(), "submit must fail when account write fails");

    // The secret stored before the failing write must have been purged.
    let access_handle = super::paths::manual_token_secret_handle(account_id, interaction_id)
        .expect("handle derivation must succeed");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_handle)
            .await
            .unwrap()
            .is_none(),
        "orphaned secret must be purged from SecretStore after failed account write"
    );
}

// ─── tests: validate_secret control-char branch ───────────────────────────────

#[tokio::test]
async fn filesystem_validate_secret_rejects_control_characters() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected ManualTokenRequired");
    };

    // NUL byte must be rejected without consuming the interaction.
    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("valid\x00nul"),
            },
        )
        .await
        .expect_err("NUL byte must be rejected");
    assert!(
        matches!(err, AuthProductError::InvalidRequest { .. }),
        "must return InvalidRequest for control characters"
    );

    // Interaction must NOT be consumed — replay still possible.
    let ok = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("clean-token"),
            },
        )
        .await;
    assert!(
        ok.is_ok(),
        "interaction must be usable after control-char rejection"
    );
}

// ─── fix: abbyshekit review — manual-token consume only after success ────────

#[tokio::test]
async fn filesystem_manual_token_consume_only_after_successful_account_write() {
    // If the account write fails, the interaction must NOT be marked consumed
    // so the user can retry without going through a full re-setup.
    use ironclaw_auth::CredentialAccountId;
    use ironclaw_filesystem::CasExpectation;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected ManualTokenRequired");
    };

    // Derive the account ID and pre-create a dummy record to force CAS failure.
    let account_id = CredentialAccountId::from_uuid(interaction_id.as_uuid());
    let dummy_account = ironclaw_auth::CredentialAccount {
        id: account_id,
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let path = super::paths::account_path(&scope, account_id)
        .expect("account path derivation must succeed");
    let json = serde_json::to_vec(&dummy_account).expect("serialization must succeed");
    use ironclaw_filesystem::{ContentType, Entry};
    let entry = Entry::bytes(json).with_content_type(ContentType::json());
    filesystem
        .put(&scope.resource, &path, entry, CasExpectation::Absent)
        .await
        .expect("pre-create dummy account must succeed");

    // First submit fails because account write hits CAS conflict.
    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("first-attempt"),
            },
        )
        .await
        .expect_err("submit must fail when account write fails");
    assert_eq!(
        err,
        AuthProductError::BackendConflict,
        "CAS conflict must surface as BackendConflict"
    );

    // Interaction must NOT be consumed — retry still possible.
    let retry_before_cleanup = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("retry-before-cleanup"),
            },
        )
        .await;
    assert!(
        retry_before_cleanup.is_err(),
        "retry must still fail because dummy account still blocks"
    );
    assert_eq!(
        retry_before_cleanup.unwrap_err(),
        AuthProductError::BackendConflict,
        "retry must still hit BackendConflict, not UnknownOrExpiredFlow"
    );

    // Remove the dummy record so retry succeeds.
    filesystem
        .delete(&scope.resource, &path)
        .await
        .expect("delete dummy account must succeed");

    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("retry-token"),
            },
        )
        .await;
    assert!(
        result.is_ok(),
        "retry must succeed after removing the blocking dummy record"
    );

    // Third attempt must now fail with UnknownOrExpiredFlow because consumed_at is set.
    let consumed_err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("third-attempt"),
            },
        )
        .await
        .expect_err("third submit must fail because interaction is consumed");
    assert_eq!(
        consumed_err,
        AuthProductError::UnknownOrExpiredFlow,
        "interaction must be consumed after successful retry"
    );
}
