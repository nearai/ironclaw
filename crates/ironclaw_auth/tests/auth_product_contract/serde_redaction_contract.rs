use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ironclaw_auth::{
    AuthProviderClient, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    GOOGLE_TOKEN_ENDPOINT, GoogleProviderClient, GoogleProviderEgressPolicyAuthorizer,
    GoogleProviderStoredTokens, GoogleProviderTokenSink, GoogleProviderTokenStorageRequest,
    OAuthAuthorizationCode, OAuthClientId, OAuthProviderCallbackRequest, OAuthRedirectUri,
    PkceVerifierSecret,
};
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, ResourceScope, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    SecretHandle,
};
use secrecy::ExposeSecret;

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
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: owner.clone(),
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

#[tokio::test]
async fn google_provider_uses_host_egress_and_returns_secret_handles_only() {
    let owner = scope("google-provider");
    let resource_scope = owner.resource.clone();
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: br#"{
            "access_token":"provider-access-token",
            "refresh_token":"provider-refresh-token",
            "scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send",
            "expires_in":3600,
            "token_type":"Bearer"
        }"#
        .to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: true,
    }));
    let sink = Arc::new(RecordingTokenSink::new(
        SecretHandle::new("google-access-secret").expect("valid handle"),
        Some(SecretHandle::new("google-refresh-secret").expect("valid handle")),
    ));
    let policy_authorizer = Arc::new(RecordingPolicyAuthorizer::default());
    let client = GoogleProviderClient::new(
        egress.clone(),
        sink.clone(),
        policy_authorizer.clone(),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client")
    .with_response_body_limit(8 * 1024);

    let client_debug = format!("{client:?}");
    assert!(client_debug.contains("Arc<dyn RuntimeHttpEgress>"));
    assert!(client_debug.contains("Arc<dyn GoogleProviderTokenSink>"));
    assert!(client_debug.contains("Arc<dyn GoogleProviderEgressPolicyAuthorizer>"));

    let request = OAuthProviderCallbackRequest {
        scope: owner,
        provider: google_provider(),
        account_label: label("work gmail"),
        authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
            .expect("valid code"),
        authorization_code_hash: code_hash("code-hash"),
        pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
            .expect("valid verifier"),
        pkce_verifier_hash: pkce_hash("pkce-hash"),
        scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]),
    };
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains("raw-auth-code"));
    assert!(!request_debug.contains("raw-pkce-verifier"));

    let exchange = client.exchange_callback(request).await.expect("exchange");
    let exchange_debug = format!("{exchange:?}");
    assert!(!exchange_debug.contains("provider-access-token"));
    assert!(!exchange_debug.contains("provider-refresh-token"));
    assert_eq!(exchange.provider, google_provider());
    assert_eq!(exchange.account_label, label("work gmail"));
    assert_eq!(exchange.authorization_code_hash, code_hash("code-hash"));
    assert_eq!(exchange.pkce_verifier_hash, pkce_hash("pkce-hash"));
    assert_eq!(
        exchange.access_secret,
        SecretHandle::new("google-access-secret").unwrap()
    );
    assert_eq!(
        exchange.refresh_secret,
        Some(SecretHandle::new("google-refresh-secret").unwrap())
    );
    assert_eq!(
        exchange.scopes,
        provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE])
    );
    assert_eq!(exchange.account_id, None);

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.runtime, RuntimeKind::System);
    assert_eq!(request.scope, resource_scope);
    assert_eq!(request.capability_id.as_str(), "ironclaw_auth.google_oauth");
    assert_eq!(request.method, NetworkMethod::Post);
    assert_eq!(request.url, GOOGLE_TOKEN_ENDPOINT);
    assert!(request.network_policy.deny_private_ip_ranges);
    assert_eq!(request.network_policy.max_egress_bytes, Some(8 * 1024));
    assert_eq!(request.response_body_limit, Some(8 * 1024));
    assert_eq!(
        request
            .network_policy
            .allowed_targets
            .iter()
            .map(|target| (target.scheme, target.host_pattern.as_str()))
            .collect::<Vec<_>>(),
        vec![(Some(NetworkScheme::Https), "oauth2.googleapis.com")]
    );
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str()),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(name, _)| name == "accept")
            .map(|(_, value)| value.as_str()),
        Some("application/json")
    );
    let pairs = url::form_urlencoded::parse(&request.body)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), "raw-auth-code".to_string()),
            ("code_verifier".to_string(), "raw-pkce-verifier".to_string()),
            ("client_id".to_string(), "google-client-123".to_string()),
            (
                "redirect_uri".to_string(),
                "https://app.example/oauth/callback".to_string()
            ),
        ]
    );
    let authorizations = policy_authorizer.authorizations();
    assert_eq!(authorizations.len(), 1);
    assert_eq!(authorizations[0].scope, resource_scope);
    assert_eq!(
        authorizations[0].capability_id.as_str(),
        "ironclaw_auth.google_oauth"
    );
    assert_eq!(authorizations[0].network_policy, request.network_policy);
    assert_eq!(
        sink.access_tokens(),
        vec!["provider-access-token".to_string()]
    );
    assert_eq!(
        sink.refresh_tokens(),
        vec!["provider-refresh-token".to_string()]
    );
    assert_eq!(sink.scopes(), vec![resource_scope]);
}

#[tokio::test]
async fn google_provider_preserves_requested_scopes_when_response_omits_scope() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: br#"{"access_token":"provider-access-token","expires_in":3600}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: true,
    }));
    let client = GoogleProviderClient::new(
        egress,
        Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-access-secret").expect("valid handle"),
            None,
        )),
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let requested_scopes = provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]);
    let exchange = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-scope-fallback"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: requested_scopes.clone(),
        })
        .await
        .expect("exchange");

    assert_eq!(exchange.scopes, requested_scopes);
}

#[tokio::test]
async fn google_provider_rejects_system_scoped_callbacks_before_side_effects() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"access_token":"provider-access-token"}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let sink = Arc::new(RecordingTokenSink::new(
        SecretHandle::new("google-access-secret").expect("valid handle"),
        None,
    ));
    let policy_authorizer = Arc::new(RecordingPolicyAuthorizer::default());
    let client = GoogleProviderClient::new(
        egress.clone(),
        sink.clone(),
        policy_authorizer.clone(),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let error = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: AuthProductScope::new(ResourceScope::system(), AuthSurface::Callback),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect_err("system-scoped callback is rejected");

    assert_eq!(error, ironclaw_auth::AuthProductError::CrossScopeDenied);
    assert!(egress.requests().is_empty());
    assert!(policy_authorizer.authorizations().is_empty());
    assert!(sink.access_tokens().is_empty());
}

#[tokio::test]
async fn google_provider_sanitizes_provider_errors() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 400,
        headers: vec![],
        body: br#"{"error":"invalid_grant","error_description":"raw provider body"}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let sink = Arc::new(RecordingTokenSink::new(
        SecretHandle::new("google-access-secret").expect("valid handle"),
        None,
    ));
    let client = GoogleProviderClient::new(
        egress,
        sink,
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let error = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-errors"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect_err("non-2xx response is sanitized");
    assert_eq!(error, ironclaw_auth::AuthProductError::TokenExchangeFailed);
    assert!(!error.to_string().contains("raw provider body"));

    let malformed_egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly","expires_in":3600"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let malformed_client = GoogleProviderClient::new(
        malformed_egress,
        Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-access-secret").expect("valid handle"),
            None,
        )),
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let malformed_error = malformed_client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-malformed"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect_err("malformed response is sanitized");
    assert_eq!(
        malformed_error,
        ironclaw_auth::AuthProductError::TokenExchangeFailed
    );
    assert!(
        !malformed_error
            .to_string()
            .contains("provider-access-token")
    );
}

#[tokio::test]
async fn google_provider_maps_egress_failures_to_backend_unavailable() {
    let egress = Arc::new(RecordingEgress::err(RuntimeHttpEgressError::Network {
        reason: "RAW_PROVIDER_ERROR /host/private sk-live-secret".to_string(),
        request_bytes: 0,
        response_bytes: 0,
    }));
    let sink = Arc::new(RecordingTokenSink::new(
        SecretHandle::new("google-access-secret").expect("valid handle"),
        None,
    ));
    let client = GoogleProviderClient::new(
        egress,
        sink,
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let error = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-egress"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect_err("egress failures are sanitized");
    assert_eq!(error, ironclaw_auth::AuthProductError::BackendUnavailable);
    assert!(!error.to_string().contains("RAW_PROVIDER_ERROR"));
    assert!(!error.to_string().contains("sk-live-secret"));
}

#[tokio::test]
async fn google_provider_rejects_non_google_provider_before_side_effects() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"access_token":"provider-access-token"}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let sink = Arc::new(RecordingTokenSink::new(
        SecretHandle::new("google-access-secret").expect("valid handle"),
        None,
    ));
    let policy_authorizer = Arc::new(RecordingPolicyAuthorizer::default());
    let client = GoogleProviderClient::new(
        egress.clone(),
        sink.clone(),
        policy_authorizer.clone(),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let error = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-rejects"),
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
        .expect_err("non-google provider is rejected");

    assert_eq!(error, ironclaw_auth::AuthProductError::TokenExchangeFailed);
    assert!(egress.requests().is_empty());
    assert!(policy_authorizer.authorizations().is_empty());
    assert!(sink.access_tokens().is_empty());
}

#[tokio::test]
async fn google_provider_sends_optional_client_secret_without_debug_leakage() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"access_token":"provider-access-token"}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let client = GoogleProviderClient::new(
        egress.clone(),
        Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-access-secret").expect("valid handle"),
            None,
        )),
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client")
    .with_client_secret(secret("raw-client-secret"));

    let client_debug = format!("{client:?}");
    assert!(client_debug.contains("client_secret"));
    assert!(!client_debug.contains("raw-client-secret"));

    client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-secret"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect("exchange");

    let request = egress.requests().pop().expect("request");
    let pairs = url::form_urlencoded::parse(&request.body)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    assert!(pairs.contains(&("client_secret".to_string(), "raw-client-secret".to_string())));
}

#[tokio::test]
async fn google_provider_propagates_token_sink_errors() {
    let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"access_token":"provider-access-token"}"#.to_vec(),
        request_bytes: 0,
        response_bytes: 0,
        saved_body: None,
        redaction_applied: false,
    }));
    let client = GoogleProviderClient::new(
        egress,
        Arc::new(FailingTokenSink {
            error: ironclaw_auth::AuthProductError::RefreshFailed,
        }),
        Arc::new(RecordingPolicyAuthorizer::default()),
        OAuthClientId::new("google-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
    )
    .expect("client");

    let error = client
        .exchange_callback(OAuthProviderCallbackRequest {
            scope: scope("google-provider-sink"),
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
        })
        .await
        .expect_err("sink error is propagated");

    assert_eq!(error, ironclaw_auth::AuthProductError::RefreshFailed);
}

struct RecordingEgress {
    responses: Mutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
}

impl RecordingEgress {
    fn ok(response: RuntimeHttpEgressResponse) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from([Ok(response)])),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn err(error: RuntimeHttpEgressError) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from([Err(error)])),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl RuntimeHttpEgress for RecordingEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().expect("requests").push(request);
        self.responses
            .lock()
            .expect("responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(RuntimeHttpEgressError::Network {
                    reason: "missing response".to_string(),
                    request_bytes: 0,
                    response_bytes: 0,
                })
            })
    }
}

struct RecordingTokenSink {
    scopes: Mutex<Vec<ResourceScope>>,
    access_tokens: Mutex<Vec<String>>,
    refresh_tokens: Mutex<Vec<String>>,
    access_handle: SecretHandle,
    refresh_handle: Option<SecretHandle>,
}

impl RecordingTokenSink {
    fn new(access_handle: SecretHandle, refresh_handle: Option<SecretHandle>) -> Self {
        Self {
            scopes: Mutex::new(Vec::new()),
            access_tokens: Mutex::new(Vec::new()),
            refresh_tokens: Mutex::new(Vec::new()),
            access_handle,
            refresh_handle,
        }
    }

    fn access_tokens(&self) -> Vec<String> {
        self.access_tokens.lock().expect("access tokens").clone()
    }

    fn scopes(&self) -> Vec<ResourceScope> {
        self.scopes.lock().expect("scopes").clone()
    }

    fn refresh_tokens(&self) -> Vec<String> {
        self.refresh_tokens.lock().expect("refresh tokens").clone()
    }
}

#[async_trait::async_trait]
impl GoogleProviderTokenSink for RecordingTokenSink {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, ironclaw_auth::AuthProductError> {
        self.scopes
            .lock()
            .expect("scopes")
            .push(request.scope.clone());
        let tokens = request.tokens;
        self.access_tokens
            .lock()
            .expect("access tokens")
            .push(tokens.access_token.expose_secret().to_string());
        if let Some(refresh_token) = tokens.refresh_token {
            self.refresh_tokens
                .lock()
                .expect("refresh tokens")
                .push(refresh_token.expose_secret().to_string());
        }
        Ok(GoogleProviderStoredTokens {
            access_secret: self.access_handle.clone(),
            refresh_secret: self.refresh_handle.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyAuthorization {
    scope: ResourceScope,
    capability_id: CapabilityId,
    network_policy: NetworkPolicy,
}

#[derive(Default)]
struct RecordingPolicyAuthorizer {
    authorizations: Mutex<Vec<PolicyAuthorization>>,
}

impl RecordingPolicyAuthorizer {
    fn authorizations(&self) -> Vec<PolicyAuthorization> {
        self.authorizations.lock().expect("authorizations").clone()
    }
}

#[async_trait::async_trait]
impl GoogleProviderEgressPolicyAuthorizer for RecordingPolicyAuthorizer {
    async fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), ironclaw_auth::AuthProductError> {
        self.authorizations
            .lock()
            .expect("authorizations")
            .push(PolicyAuthorization {
                scope: scope.clone(),
                capability_id: capability_id.clone(),
                network_policy: policy.clone(),
            });
        Ok(())
    }
}

struct FailingTokenSink {
    error: ironclaw_auth::AuthProductError,
}

#[async_trait::async_trait]
impl GoogleProviderTokenSink for FailingTokenSink {
    async fn store_tokens(
        &self,
        _request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, ironclaw_auth::AuthProductError> {
        Err(self.error.clone())
    }
}
