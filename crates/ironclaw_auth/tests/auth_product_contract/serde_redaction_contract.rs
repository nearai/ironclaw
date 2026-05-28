use ironclaw_auth::{
    AuthProviderClient, OAuthAuthorizationCode, OAuthProviderCallbackRequest,
    OAuthProviderExchangeContext, PkceVerifierSecret,
};

use crate::common::*;

#[test]
fn serde_contracts_are_validated_snake_case_and_redacted() {
    assert!(serde_json::from_str::<AuthProviderId>("\"bad\nprovider\"").is_err());
    assert!(serde_json::from_str::<AuthSessionId>("\" session \"").is_err());
    assert!(serde_json::from_str::<ProviderScope>("\" repo \"").is_err());
    assert!(OpaqueStateHash::new("raw-state-value").is_err());
    assert!(PkceVerifierHash::new("raw-pkce-verifier").is_err());
    assert!(AuthorizationCodeHash::new("raw-auth-code").is_err());
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

    let selection_challenge = AuthChallenge::AccountSelectionRequired {
        provider: provider(),
        accounts: vec![CredentialAccountProjection {
            id: ironclaw_auth::CredentialAccountId::new(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            secret_handle_count: 2,
        }],
    };
    let challenge_wire = serde_json::to_value(&selection_challenge).expect("serialize challenge");
    assert_eq!(
        challenge_wire["type"],
        serde_json::json!("account_selection_required")
    );
    assert_eq!(
        challenge_wire["accounts"][0]["label"],
        serde_json::json!("work")
    );
    assert!(challenge_wire.get("account_ids").is_none());
    assert!(!challenge_wire.to_string().contains("github-work-secret"));

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

#[test]
fn backend_failures_are_reported_as_stable_sanitized_codes() {
    let backend_sentinel = "RAW_PROVIDER_ERROR_SENTINEL /host/private sk-live-secret lease-123";
    for error in [
        AuthProductError::BackendUnavailable,
        AuthProductError::TokenExchangeFailed,
        AuthProductError::RefreshFailed,
    ] {
        let rendered = error.to_string();
        let serialized_code = serde_json::to_string(&error.code()).expect("serialize error code");
        assert!(!rendered.contains(backend_sentinel));
        assert!(!rendered.contains("RAW_PROVIDER_ERROR_SENTINEL"));
        assert!(!rendered.contains("/host/private"));
        assert!(!rendered.contains("sk-live-secret"));
        assert!(!rendered.contains("lease-123"));
        assert!(!serialized_code.contains(backend_sentinel));
        assert!(!serialized_code.contains("RAW_PROVIDER_ERROR_SENTINEL"));
        assert!(!serialized_code.contains("/host/private"));
        assert!(!serialized_code.contains("sk-live-secret"));
        assert!(!serialized_code.contains("lease-123"));
    }

    assert_eq!(
        serde_json::to_value(AuthProductError::BackendUnavailable.code()).expect("serialize"),
        serde_json::json!("backend_unavailable")
    );
    assert_eq!(
        serde_json::to_value(AuthProductError::TokenExchangeFailed.code()).expect("serialize"),
        serde_json::json!("token_exchange_failed")
    );
    assert_eq!(
        serde_json::to_value(AuthProductError::RefreshFailed.code()).expect("serialize"),
        serde_json::json!("refresh_failed")
    );
}

#[tokio::test]
async fn serializable_records_never_include_raw_oauth_or_token_material() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = oauth_flow(&services, owner.clone()).await;
    let exchange = services
        .exchange_callback(
            OAuthProviderExchangeContext {
                scope: owner.clone(),
                flow_id: flow.id,
            },
            OAuthProviderCallbackRequest {
                provider: provider(),
                account_label: label("work github"),
                authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                    .expect("valid code"),
                authorization_code_hash: code_hash("code-hash"),
                pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                    .expect("valid verifier"),
                pkce_verifier_hash: pkce_hash("pkce-hash"),
                scopes: provider_scopes(&["repo"]),
            },
        )
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
    assert!(serialized.contains(&fake_digest("code-hash")));

    let account = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            completed
                .credential_account_id
                .expect("completed flow has account"),
        ))
        .await
        .expect("lookup")
        .expect("account");
    let account_debug = format!("{account:?}");
    assert!(!account_debug.contains("oauth-access"));
    assert!(!account_debug.contains("oauth-refresh"));
    assert!(account_debug.contains("[REDACTED]"));
}
