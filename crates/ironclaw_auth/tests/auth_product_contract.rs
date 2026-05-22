use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowKind, AuthFlowManager,
    AuthFlowStatus, AuthGateRef, AuthInteractionService, AuthProductError, AuthProductScope,
    AuthProviderClient, AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccount, CredentialAccountLabel, CredentialAccountListRequest,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialOwnership, CredentialSetupService,
    InMemoryAuthProductServices, LifecyclePackageRef, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthCallbackInput,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OpaqueStateHash, PkceVerifierHash,
    PkceVerifierSecret, ProviderCallbackOutcome, ProviderScope, SecretCleanupAction,
    SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest, SecretSubmitResult,
    TurnRunRef,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, SecretHandle, UserId};
use secrecy::SecretString;

fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).expect("valid user"), InvocationId::new())
            .expect("valid scope"),
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new(format!("session-{user}")).expect("valid session"))
}

fn provider() -> AuthProviderId {
    AuthProviderId::new("github").expect("valid provider")
}

fn label(value: &str) -> CredentialAccountLabel {
    CredentialAccountLabel::new(value).expect("valid label")
}

fn state_hash(value: &str) -> OpaqueStateHash {
    OpaqueStateHash::new(value).expect("valid state hash")
}

fn pkce_hash(value: &str) -> PkceVerifierHash {
    PkceVerifierHash::new(value).expect("valid pkce hash")
}

fn code_hash(value: &str) -> AuthorizationCodeHash {
    AuthorizationCodeHash::new(value).expect("valid code hash")
}

fn authorization_url(value: &str) -> OAuthAuthorizationUrl {
    OAuthAuthorizationUrl::new(value).expect("valid authorization url")
}

fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).expect("valid provider scope")
}

fn provider_scopes(values: &[&str]) -> Vec<ProviderScope> {
    values.iter().map(|value| provider_scope(value)).collect()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_string())
}

fn account_request(
    owner: AuthProductScope,
    label_value: &str,
    status: CredentialAccountStatus,
) -> NewCredentialAccount {
    NewCredentialAccount {
        update_account_id: None,
        scope: owner,
        provider: provider(),
        label: label(label_value),
        status,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: None,
        refresh_secret: None,
        scopes: Vec::new(),
    }
}

fn update_binding(account: &CredentialAccount) -> CredentialAccountUpdateBinding {
    CredentialAccountUpdateBinding {
        account_id: account.id,
        ownership: account.ownership,
        owner_extension: account.owner_extension.clone(),
        granted_extensions: account.granted_extensions.clone(),
    }
}

async fn oauth_flow(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
) -> ironclaw_auth::AuthFlowRecord {
    services
        .create_flow(NewAuthFlow {
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow")
}

async fn oauth_update_flow(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
    account: &CredentialAccount,
) -> ironclaw_auth::AuthFlowRecord {
    services
        .create_flow(NewAuthFlow {
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: Some(update_binding(account)),
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("update flow")
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
        .exchange_callback(request)
        .await
        .expect("provider exchange");
    let completed = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized { exchange },
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
async fn oauth_callback_updates_existing_account_from_provider_exchange() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let existing = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("renamed github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: access_secret.clone(),
                        refresh_secret: Some(refresh_secret.clone()),
                        scopes: provider_scopes(&["repo", "workflow"]),
                        account_id: Some(existing.id),
                    },
                },
            },
        )
        .await
        .expect("callback updates account");

    assert_eq!(completed.credential_account_id, Some(existing.id));
    let updated = services
        .get_account(&owner, existing.id)
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
                    exchange: OAuthProviderExchange {
                        provider: gitlab.clone(),
                        account_label: label("gitlab"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("gitlab-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["read_user"]),
                        account_id: None,
                    },
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("missing"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("missing-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(existing.id),
                    },
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("foreign"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("foreign-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(foreign.id),
                    },
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("wrong provider account"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(provider_mismatch.id),
                    },
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("renamed github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: Some(existing.id),
                    },
                },
            },
        )
        .await
        .expect("valid existing account update still works");
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
                    exchange: OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("work github"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("other-pkce-hash"),
                        access_secret: SecretHandle::new("github-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: None,
                    },
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

#[tokio::test]
async fn credential_setup_updates_only_explicit_authorized_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let first = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("create account");
    let access_secret = SecretHandle::new("github-updated-access").unwrap();
    let second = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: Some(first.id),
            scope: owner.clone(),
            provider: provider(),
            label: label("work renamed"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("update account");

    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.label, label("work renamed"));
    assert_eq!(second.status, CredentialAccountStatus::Configured);
    assert_eq!(second.access_secret, Some(access_secret));
    assert_eq!(second.scopes, provider_scopes(&["repo"]));

    let same_label_without_target_creates_new_account = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("work renamed"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("same label without target is create");
    assert_ne!(same_label_without_target_creates_new_account.id, first.id);

    let rejected_takeover = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: Some(first.id),
            scope: owner.clone(),
            provider: provider(),
            label: label("takeover"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ExtensionId::new("attacker").unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-takeover").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect_err("ownership changes require a separate authority flow");
    assert_eq!(rejected_takeover, AuthProductError::CrossScopeDenied);

    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 2);
    assert!(
        accounts
            .accounts
            .iter()
            .any(|account| account.id == first.id)
    );
    assert!(accounts.next_cursor.is_none());
}

#[tokio::test]
async fn credential_account_update_status_updates_owner_record_and_rejects_missing_or_cross_scope()
{
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "work",
            CredentialAccountStatus::Configured,
        ))
        .await
        .expect("create account");

    let updated = services
        .update_status(&owner, account.id, CredentialAccountStatus::RefreshFailed)
        .await
        .expect("update status");
    assert_eq!(updated.status, CredentialAccountStatus::RefreshFailed);

    let missing = services
        .update_status(
            &owner,
            ironclaw_auth::CredentialAccountId::new(),
            CredentialAccountStatus::Revoked,
        )
        .await
        .expect_err("missing account");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let cross_scope = services
        .update_status(&scope("bob"), account.id, CredentialAccountStatus::Revoked)
        .await
        .expect_err("cross-scope account");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let still_owner = services
        .get_account(&owner, account.id)
        .await
        .expect("lookup")
        .expect("owner account");
    assert_eq!(still_owner.status, CredentialAccountStatus::RefreshFailed);
}

#[tokio::test]
async fn credential_account_list_is_explicitly_paginated() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    for name in ["alpha", "beta", "gamma"] {
        services
            .create_account(account_request(
                owner.clone(),
                name,
                CredentialAccountStatus::Configured,
            ))
            .await
            .expect("create account");
    }

    let first_page = services
        .list_accounts(CredentialAccountListRequest::new(owner.clone(), provider()).with_limit(2))
        .await
        .expect("first page");
    assert_eq!(first_page.accounts.len(), 2);
    let cursor = first_page
        .next_cursor
        .expect("cursor for remaining account");

    let second_page = services
        .list_accounts(
            CredentialAccountListRequest::new(owner.clone(), provider())
                .with_limit(2)
                .with_cursor(cursor),
        )
        .await
        .expect("second page");
    assert_eq!(second_page.accounts.len(), 1);
    assert!(second_page.next_cursor.is_none());

    let zero_limit = services
        .list_accounts(CredentialAccountListRequest::new(owner.clone(), provider()).with_limit(0))
        .await
        .expect_err("zero limit rejected");
    assert_eq!(zero_limit.code(), AuthErrorCode::InvalidRequest);
    let too_large = services
        .list_accounts(
            CredentialAccountListRequest::new(owner, provider())
                .with_limit(CredentialAccountListRequest::MAX_LIMIT + 1),
        )
        .await
        .expect_err("oversized limit rejected");
    assert_eq!(too_large.code(), AuthErrorCode::InvalidRequest);
}

#[tokio::test]
async fn credential_account_selection_requires_user_choice_for_multiple_configured_accounts() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    let missing = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("no configured account");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("expired"),
            status: CredentialAccountStatus::RefreshFailed,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-expired").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("refresh-failed account");
    let still_missing = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("refresh-failed account is not selectable");
    assert_eq!(still_missing, AuthProductError::CredentialMissing);

    let work = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-work").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("work account");
    let selected = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect("single configured account");
    assert_eq!(selected.id, work.id);

    services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("personal"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-personal").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("personal account");
    let err = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("multiple accounts require choice");
    assert_eq!(err, AuthProductError::AccountSelectionRequired);
}

#[tokio::test]
async fn credential_account_selection_filters_by_requester_authority() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();

    let extension_owned = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(github_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("extension-owned account");

    let unauthorized = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("no requester cannot select extension-owned account");
    assert_eq!(unauthorized, AuthProductError::CrossScopeDenied);
    let wrong_requester = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner.clone(), provider())
                .for_extension(other_extension.clone()),
        )
        .await
        .expect_err("wrong requester cannot select extension-owned account");
    assert_eq!(wrong_requester, AuthProductError::CrossScopeDenied);

    let selected = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner.clone(), provider())
                .for_extension(github_extension.clone()),
        )
        .await
        .expect("owning requester can select account");
    assert_eq!(selected.id, extension_owned.id);

    services
        .update_status(&owner, extension_owned.id, CredentialAccountStatus::Revoked)
        .await
        .expect("hide extension-owned account for shared test");
    let shared = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![github_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("shared account");
    let selected_shared = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner, provider())
                .for_extension(github_extension),
        )
        .await
        .expect("granted requester can select shared account");
    assert_eq!(selected_shared.id, shared.id);
}

#[tokio::test]
async fn extension_owned_accounts_require_owner_and_cleanup_is_action_specific() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let orphan = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("orphan"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-orphan").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect_err("extension owned requires owner");
    assert_eq!(orphan.code(), AuthErrorCode::InvalidRequest);

    let owned = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("owned account");
    let reusable = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: owner.clone(),
            provider: provider(),
            label: label("reusable"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("github-reusable").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("reusable account");

    let deactivate = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .expect("deactivate");
    assert!(deactivate.retained_accounts.contains(&owned.id));
    assert!(deactivate.removed_grants.contains(&reusable.id));
    assert!(deactivate.revoked_accounts.is_empty());

    let uninstall = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner,
            extension_id: extension,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("uninstall");
    assert!(uninstall.revoked_accounts.contains(&owned.id));
}

#[tokio::test]
async fn cleanup_for_lifecycle_ignores_cross_scope_accounts() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let foreign_owner = scope("bob");
    let extension = ExtensionId::new("github").unwrap();

    let foreign_owned = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: foreign_owner.clone(),
            provider: provider(),
            label: label("foreign owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-foreign-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("foreign owned account");
    let foreign_granted = services
        .create_account(NewCredentialAccount {
            update_account_id: None,
            scope: foreign_owner.clone(),
            provider: provider(),
            label: label("foreign granted"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("github-foreign-granted").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("foreign granted account");

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner,
            extension_id: extension.clone(),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup");
    assert!(report.revoked_accounts.is_empty());
    assert!(report.retained_accounts.is_empty());
    assert!(report.removed_grants.is_empty());

    let owned_after = services
        .get_account(&foreign_owner, foreign_owned.id)
        .await
        .expect("lookup")
        .expect("foreign owned remains");
    assert_eq!(owned_after.status, CredentialAccountStatus::Configured);
    assert_eq!(owned_after.owner_extension, Some(extension.clone()));
    let granted_after = services
        .get_account(&foreign_owner, foreign_granted.id)
        .await
        .expect("lookup")
        .expect("foreign granted remains");
    assert_eq!(granted_after.granted_extensions, vec![extension]);
}

#[test]
fn serde_contracts_are_validated_snake_case_and_redacted() {
    assert!(serde_json::from_str::<AuthProviderId>("\"bad\nprovider\"").is_err());
    assert!(serde_json::from_str::<AuthSessionId>("\" session \"").is_err());
    assert!(serde_json::from_str::<ProviderScope>("\" repo \"").is_err());
    assert!(
        serde_json::from_str::<OAuthAuthorizationUrl>("\"http://provider.example/oauth\"").is_err()
    );
    assert!(OAuthAuthorizationUrl::new("https://:443/oauth").is_err());
    assert!(OAuthAuthorizationUrl::new("https://user@provider.example/oauth").is_err());
    assert!(OAuthAuthorizationUrl::new("https://provider example/oauth").is_err());
    assert_eq!(
        serde_json::to_value(authorization_url("https://provider.example/oauth")).expect("url"),
        serde_json::json!("https://provider.example/oauth")
    );

    let code = serde_json::to_value(AuthErrorCode::InvalidRequest).expect("serialize");
    assert_eq!(code, serde_json::json!("invalid_request"));
    assert_eq!(
        AuthProductError::RefreshFailed.code(),
        AuthErrorCode::RefreshFailed
    );

    let continuation = AuthContinuationRef::TurnGateResume {
        turn_run_ref: TurnRunRef::new("run-ref").unwrap(),
        gate_ref: AuthGateRef::new("gate-ref").unwrap(),
    };
    let rendered = serde_json::to_string(&continuation).expect("serialize");
    assert!(rendered.contains("turn_gate_resume"));
    assert!(!rendered.contains("raw prompt"));

    let submit_result = SecretSubmitResult {
        account_id: ironclaw_auth::CredentialAccountId::new(),
        status: CredentialAccountStatus::Configured,
        continuation: AuthContinuationRef::SetupOnly,
    };
    let submit_result_json = serde_json::to_string(&submit_result).expect("serialize result");
    assert!(submit_result_json.contains("configured"));
    let round_trip: SecretSubmitResult =
        serde_json::from_str(&submit_result_json).expect("deserialize result");
    assert_eq!(round_trip, submit_result);
}

#[tokio::test]
async fn serializable_records_never_include_raw_oauth_or_token_material() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;
    let exchange = services
        .exchange_callback(OAuthProviderCallbackRequest {
            provider: provider(),
            account_label: label("work github"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("exchange");
    let completed = services
        .complete_oauth_callback(
            &owner,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized { exchange },
            },
        )
        .await
        .expect("complete");
    let serialized = serde_json::to_string(&completed).expect("serialize record");
    assert!(!serialized.contains("raw-auth-code"));
    assert!(!serialized.contains("raw-pkce-verifier"));
    assert!(!serialized.contains("ghp_"));
    assert!(serialized.contains("code-hash"));

    let account = services
        .get_account(
            &owner,
            completed
                .credential_account_id
                .expect("completed flow has account"),
        )
        .await
        .expect("lookup")
        .expect("account");
    let account_debug = format!("{account:?}");
    assert!(!account_debug.contains("oauth-access"));
    assert!(!account_debug.contains("oauth-refresh"));
    assert!(account_debug.contains("[REDACTED]"));
}
