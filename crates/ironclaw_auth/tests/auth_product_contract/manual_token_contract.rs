use crate::common::*;

#[tokio::test]
async fn manual_token_submit_is_secure_scoped_and_rejects_invalid_inputs() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    let submit = SecretSubmitRequest {
        interaction_id,
        secret: secret("ghp_super_secret_token"),
    };
    let debug = format!("{submit:?}");
    assert!(!debug.contains("ghp_super_secret_token"));
    assert!(debug.contains("[REDACTED]"));

    let cross_scope = services
        .submit_manual_token(
            &scope("bob"),
            SecretSubmitRequest {
                interaction_id,
                secret: secret("attacker-token"),
            },
        )
        .await
        .expect_err("cross-scope submit denied");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let empty = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("   "),
            },
        )
        .await
        .expect_err("empty secret rejected before consumption");
    assert_eq!(empty.code(), AuthErrorCode::InvalidRequest);

    let result = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_super_secret_token"),
            },
        )
        .await
        .expect("owner submit");
    assert_eq!(result.status, CredentialAccountStatus::Configured);

    let replay = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_second_submit"),
            },
        )
        .await
        .expect_err("manual-token interaction is consumed once");
    assert_eq!(replay, AuthProductError::UnknownOrExpiredFlow);
}

#[tokio::test]
async fn manual_token_submit_consumes_interaction_once() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_first_submit"),
            },
        )
        .await
        .expect("first submit succeeds");
    let second_submit = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_second_submit"),
            },
        )
        .await
        .expect_err("interaction is consumed after success");
    assert_eq!(second_submit, AuthProductError::UnknownOrExpiredFlow);

    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 1);
}

#[tokio::test]
async fn manual_token_submit_rejects_control_characters_without_consuming_interaction() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    let invalid = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_bad\nsecret"),
            },
        )
        .await
        .expect_err("control characters are rejected");
    assert_eq!(invalid.code(), AuthErrorCode::InvalidRequest);

    let result = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_valid_after_invalid"),
            },
        )
        .await
        .expect("valid retry succeeds because invalid input did not consume interaction");
    assert_eq!(result.status, CredentialAccountStatus::Configured);
}

#[tokio::test]
async fn manual_token_interaction_can_be_abandoned_after_route_submit_failure() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let other = scope("bob");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    let cross_scope = services
        .abandon_manual_token(&other, interaction_id)
        .await
        .expect_err("cross-scope abandon denied");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    assert!(
        services
            .abandon_manual_token(&owner, interaction_id)
            .await
            .expect("owner can abandon pending interaction")
    );
    assert!(
        !services
            .abandon_manual_token(&owner, interaction_id)
            .await
            .expect("abandon is idempotent for missing interaction")
    );

    let submit = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_after_abandon"),
            },
        )
        .await
        .expect_err("abandoned interaction cannot be submitted");
    assert_eq!(submit, AuthProductError::UnknownOrExpiredFlow);
}

#[tokio::test]
async fn expired_manual_token_interaction_fails_closed() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .expect("manual challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };
    let expired = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("valid-but-expired"),
            },
        )
        .await
        .expect_err("expired");
    assert_eq!(expired, AuthProductError::UnknownOrExpiredFlow);
}

#[tokio::test]
async fn manual_token_submit_can_update_bound_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "old manual github",
            CredentialAccountStatus::Expired,
        ))
        .await
        .expect("existing account");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("updated manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(update_binding(&account)),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual update challenge");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    let result = services
        .submit_manual_token(
            &owner,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_updated_token"),
            },
        )
        .await
        .expect("manual update submit");

    assert_eq!(result.account_id, account.id);
    assert_eq!(result.status, CredentialAccountStatus::Configured);
    let updated = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("account lookup")
        .expect("updated account");
    assert_eq!(updated.id, account.id);
    assert_eq!(updated.label, label("updated manual github"));
    assert_eq!(updated.status, CredentialAccountStatus::Configured);
    assert!(updated.access_secret.is_some());
    assert_eq!(updated.created_at, account.created_at);
    assert!(updated.updated_at >= account.updated_at);

    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, account.id);
}

#[tokio::test]
async fn manual_token_reconnect_updates_bound_account_across_a_different_thread() {
    // Regression (#4935 defect A, manual-token path): a manual-token reconnect
    // arriving from a different thread/invocation than the one the account was
    // created in must UPDATE the bound account at owner granularity, not fail.
    // The OAuth path already had this guarantee; the manual-token apply path was
    // still using full `scope_matches`, so setup accepted the binding but submit
    // rejected it with CrossScopeDenied (and would have re-forked the account).
    let services = InMemoryAuthProductServices::new();
    let create_scope = scope("alice");
    let account = services
        .create_account(account_request(
            create_scope.clone(),
            "old manual github",
            CredentialAccountStatus::Expired,
        ))
        .await
        .expect("existing account");

    let reauth_scope = reconnect_scope("alice", "thread-reauth");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: reauth_scope.clone(),
            provider: provider(),
            label: label("updated manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(update_binding(&account)),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("manual reconnect challenge across a different thread");
    let interaction_id = match challenge {
        AuthChallenge::ManualTokenRequired { interaction_id, .. } => interaction_id,
        other => panic!("unexpected challenge {other:?}"),
    };

    let result = services
        .submit_manual_token(
            &reauth_scope,
            SecretSubmitRequest {
                interaction_id,
                secret: secret("ghp_reconnect_token"),
            },
        )
        .await
        .expect("manual reconnect submit must update the bound account, not fork");

    assert_eq!(result.account_id, account.id);
    assert_eq!(result.status, CredentialAccountStatus::Configured);

    // No fork: still exactly one account for the owner.
    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(create_scope, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, account.id);
}

#[tokio::test]
async fn manual_token_update_binding_is_scope_checked_before_challenge() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "manual github",
            CredentialAccountStatus::Expired,
        ))
        .await
        .expect("existing account");

    let error = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope("bob"),
            provider: provider(),
            label: label("attacker manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(update_binding(&account)),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("cross-scope update binding is rejected before challenge");

    assert_eq!(error, AuthProductError::CrossScopeDenied);
    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, account.id);
}

#[tokio::test]
async fn manual_token_update_binding_rejects_missing_account_before_challenge() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    let error = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id: ironclaw_auth::CredentialAccountId::new(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
            }),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("missing update target is rejected before challenge");

    assert_eq!(error, AuthProductError::CredentialMissing);
    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert!(accounts.accounts.is_empty());
}

#[tokio::test]
async fn manual_token_update_binding_rejects_provider_mismatch_before_challenge() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "manual github",
            CredentialAccountStatus::Expired,
        ))
        .await
        .expect("existing account");

    let error = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner,
            provider: AuthProviderId::new("gitlab").expect("valid provider"),
            label: label("manual gitlab"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(update_binding(&account)),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("provider mismatch is rejected before challenge");

    assert_eq!(error.code(), AuthErrorCode::InvalidRequest);
}
