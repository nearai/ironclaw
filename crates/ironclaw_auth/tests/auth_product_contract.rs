use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowKind, AuthFlowManager,
    AuthFlowStatus, AuthGateRef, AuthInteractionService, AuthProductError, AuthProductScope,
    AuthProviderClient, AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccountLabel, CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    CredentialSetupService, InMemoryAuthProductServices, LifecyclePackageRef,
    ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount, OAuthAuthorizationCode,
    OAuthCallbackInput, OAuthProviderCallbackRequest, OAuthProviderExchange, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderCallbackOutcome, SecretCleanupAction,
    SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest, TurnRunRef,
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

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_string())
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
                authorization_url: "https://provider.example/oauth".to_string(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow")
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
        scopes: vec!["repo".to_string()],
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
            scope: owner.clone(),
            provider: provider(),
            label: label("work github"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-old-access").unwrap()),
            refresh_secret: None,
            scopes: vec!["read:user".to_string()],
        })
        .await
        .expect("existing account");
    let flow = oauth_flow(&services, owner.clone()).await;
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
                        scopes: vec!["repo".to_string(), "workflow".to_string()],
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
    assert_eq!(
        updated.scopes,
        vec!["repo".to_string(), "workflow".to_string()]
    );
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
async fn credential_setup_create_or_update_reuses_matching_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let first = services
        .create_or_update_account(NewCredentialAccount {
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
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec!["repo".to_string()],
        })
        .await
        .expect("update account");

    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.status, CredentialAccountStatus::Configured);
    assert_eq!(second.access_secret, Some(access_secret));
    assert_eq!(second.scopes, vec!["repo".to_string()]);
    let accounts = services
        .list_accounts(&owner, &provider())
        .await
        .expect("list accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, first.id);
}

#[tokio::test]
async fn credential_account_selection_requires_user_choice_for_multiple_configured_accounts() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    let missing = services
        .select_unique_configured_account(&owner, &provider())
        .await
        .expect_err("no configured account");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    services
        .create_account(NewCredentialAccount {
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
        .select_unique_configured_account(&owner, &provider())
        .await
        .expect_err("refresh-failed account is not selectable");
    assert_eq!(still_missing, AuthProductError::CredentialMissing);

    let work = services
        .create_account(NewCredentialAccount {
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
        .select_unique_configured_account(&owner, &provider())
        .await
        .expect("single configured account");
    assert_eq!(selected.id, work.id);

    services
        .create_account(NewCredentialAccount {
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
        .select_unique_configured_account(&owner, &provider())
        .await
        .expect_err("multiple accounts require choice");
    assert_eq!(err, AuthProductError::AccountSelectionRequired);
}

#[tokio::test]
async fn extension_owned_accounts_require_owner_and_cleanup_is_action_specific() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let orphan = services
        .create_account(NewCredentialAccount {
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

#[test]
fn serde_contracts_are_validated_snake_case_and_redacted() {
    assert!(serde_json::from_str::<AuthProviderId>("\"bad\nprovider\"").is_err());
    assert!(serde_json::from_str::<AuthSessionId>("\" session \"").is_err());

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
            scopes: vec!["repo".to_string()],
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
}
