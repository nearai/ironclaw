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
async fn expired_manual_token_interaction_fails_closed() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let challenge = services
        .request_secret_input(ManualTokenSetupRequest {
            scope: owner.clone(),
            provider: provider(),
            label: label("manual github"),
            continuation: AuthContinuationRef::SetupOnly,
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
