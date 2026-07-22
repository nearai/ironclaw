use crate::common::*;

#[tokio::test]
async fn auth_flow_uses_open_processing_and_exact_resolved_outcomes() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("state-contract");
    let flow = oauth_flow(&services, owner.clone()).await;
    assert_eq!(flow.state, AuthFlowState::Open);

    let claimed = services
        .claim_oauth_callback(
            &owner,
            ironclaw_auth::OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state-hash"),
                provider: provider(),
                pkce_verifier_hash: pkce_hash("pkce-hash"),
            },
        )
        .await
        .expect("callback claim");
    let OAuthCallbackClaim::Acquired(claimed) = claimed else {
        panic!("first callback must acquire provider exchange ownership");
    };
    assert_eq!(claimed.state, AuthFlowState::Processing);

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
        .expect_err("provider denial remains a product error");
    assert_eq!(denied, AuthProductError::ProviderDenied);
    let resolved = services
        .get_flow(&owner, flow.id)
        .await
        .expect("flow lookup")
        .expect("flow remains durable");
    assert_eq!(
        resolved.state,
        AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied)
    );
}

#[tokio::test]
async fn callback_claim_distinguishes_the_exchange_owner_from_an_inflight_replay() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("claim-ownership");
    let flow = oauth_flow(&services, owner.clone()).await;
    let request = ironclaw_auth::OAuthCallbackClaimRequest {
        flow_id: flow.id,
        opaque_state_hash: state_hash("state-hash"),
        provider: provider(),
        pkce_verifier_hash: pkce_hash("pkce-hash"),
    };

    let acquired = services
        .claim_oauth_callback(&owner, request.clone())
        .await
        .expect("first callback claim");
    let replay = services
        .claim_oauth_callback(&owner, request)
        .await
        .expect("inflight callback replay");

    assert!(matches!(
        acquired,
        ironclaw_auth::OAuthCallbackClaim::Acquired(_)
    ));
    assert!(matches!(
        replay,
        ironclaw_auth::OAuthCallbackClaim::Existing(_)
    ));
}

#[tokio::test]
async fn terminal_actions_resolve_to_their_exact_outcomes() {
    let services = InMemoryAuthProductServices::new();

    let authorized_owner = scope("authorized");
    let authorized_flow = oauth_flow(&services, authorized_owner.clone()).await;
    let account_id = services
        .complete_oauth_callback(
            &authorized_owner,
            OAuthCallbackInput {
                flow_id: authorized_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider(),
                        account_label: label("authorized account"),
                        authorization_code_hash: code_hash("code-hash"),
                        pkce_verifier_hash: pkce_hash("pkce-hash"),
                        access_secret: SecretHandle::new("authorized-access").unwrap(),
                        refresh_secret: None,
                        scopes: provider_scopes(&["repo"]),
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("authorization resolves")
        .state;
    assert!(matches!(
        account_id,
        AuthFlowState::Resolved(AuthFlowOutcome::Authorized { .. })
    ));

    let aborted_owner = scope("aborted");
    let aborted_flow = oauth_flow(&services, aborted_owner.clone()).await;
    let aborted = services
        .cancel_flow(&aborted_owner, aborted_flow.id)
        .await
        .expect("user abort resolves");
    assert_eq!(
        aborted.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
    );

    let failed_owner = scope("failed");
    let failed_flow = oauth_flow(&services, failed_owner.clone()).await;
    let failed = services
        .fail_oauth_callback(
            &failed_owner,
            ironclaw_auth::OAuthCallbackFailureInput {
                flow_id: failed_flow.id,
                opaque_state_hash: state_hash("state-hash"),
                error: AuthErrorCode::TokenExchangeFailed,
            },
        )
        .await
        .expect("callback failure resolves");
    assert_eq!(
        failed.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Failed {
            error: AuthErrorCode::TokenExchangeFailed,
        })
    );
}

#[tokio::test]
async fn callback_claim_replay_rejects_every_non_authorized_terminal_outcome() {
    let services = InMemoryAuthProductServices::new();

    let denied_owner = scope("claim-provider-denied");
    let denied = oauth_flow(&services, denied_owner.clone()).await;
    let denial = services
        .complete_oauth_callback(
            &denied_owner,
            OAuthCallbackInput {
                flow_id: denied.id,
                opaque_state_hash: state_hash("state-hash"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("provider denial resolves the flow");
    assert_eq!(denial, AuthProductError::ProviderDenied);

    let aborted_owner = scope("claim-user-aborted");
    let aborted = oauth_flow(&services, aborted_owner.clone()).await;
    services
        .cancel_flow(&aborted_owner, aborted.id)
        .await
        .expect("user abort resolves the flow");

    let failed_owner = scope("claim-failed");
    let failed = oauth_flow(&services, failed_owner.clone()).await;
    services
        .fail_oauth_callback(
            &failed_owner,
            ironclaw_auth::OAuthCallbackFailureInput {
                flow_id: failed.id,
                opaque_state_hash: state_hash("state-hash"),
                error: AuthErrorCode::TokenExchangeFailed,
            },
        )
        .await
        .expect("callback failure resolves the flow");

    let expired_owner = scope("claim-expired");
    let expired = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: expired_owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() - Duration::minutes(1),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() - Duration::minutes(1),
        })
        .await
        .expect("expired flow record is created");
    let first_expired_claim = services
        .claim_oauth_callback(
            &expired_owner,
            ironclaw_auth::OAuthCallbackClaimRequest {
                flow_id: expired.id,
                opaque_state_hash: state_hash("state-hash"),
                provider: provider(),
                pkce_verifier_hash: pkce_hash("pkce-hash"),
            },
        )
        .await
        .expect_err("first claim marks an expired flow terminal");
    assert_eq!(first_expired_claim, AuthProductError::UnknownOrExpiredFlow);

    for (label, owner, flow_id, expected_error) in [
        (
            "provider denied",
            denied_owner,
            denied.id,
            AuthProductError::FlowAlreadyTerminal,
        ),
        (
            "user aborted",
            aborted_owner,
            aborted.id,
            AuthProductError::Canceled,
        ),
        (
            "failed",
            failed_owner,
            failed.id,
            AuthProductError::FlowAlreadyTerminal,
        ),
        (
            "expired",
            expired_owner,
            expired.id,
            AuthProductError::FlowAlreadyTerminal,
        ),
    ] {
        let error = services
            .claim_oauth_callback(
                &owner,
                ironclaw_auth::OAuthCallbackClaimRequest {
                    flow_id,
                    opaque_state_hash: state_hash("state-hash"),
                    provider: provider(),
                    pkce_verifier_hash: pkce_hash("pkce-hash"),
                },
            )
            .await
            .expect_err("every non-authorized terminal outcome rejects callback claims");
        assert_eq!(error, expected_error, "{label}");
    }
}
