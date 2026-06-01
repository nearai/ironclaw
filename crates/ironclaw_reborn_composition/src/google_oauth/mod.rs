use std::sync::Arc;

use ironclaw_auth::{AuthProductError, AuthProviderClient};
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use ironclaw_secrets::SecretStore;

use crate::RebornBuildError;
use crate::input::OAuthClientConfig;

mod client;
mod policy_authorizer;
mod secret_sink;
mod token_request;
mod token_response;

use client::GoogleProviderClient;
use policy_authorizer::ObligationGoogleEgressPolicyAuthorizer;
use secret_sink::SecretStoreGoogleTokenSink;

pub(crate) fn google_provider_client(
    config: OAuthClientConfig,
    secret_store: Arc<dyn SecretStore>,
    runtime_ports: ProductAuthProviderRuntimePorts,
) -> Result<Arc<dyn AuthProviderClient>, RebornBuildError> {
    let mut client = GoogleProviderClient::new(
        runtime_ports.runtime_http_egress(),
        Arc::new(SecretStoreGoogleTokenSink {
            store: secret_store,
        }),
        Arc::new(ObligationGoogleEgressPolicyAuthorizer {
            handler: runtime_ports.obligation_handler(),
        }),
        config.client_id,
        config.redirect_uri,
    )
    .map_err(auth_provider_config_error)?;
    if let Some(client_secret) = config.client_secret {
        client = client.with_client_secret(client_secret);
    }
    Ok(Arc::new(client))
}

fn auth_provider_config_error(error: AuthProductError) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("Google OAuth provider backend could not be configured: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_oauth::policy_authorizer::{
        GoogleProviderEgressPolicyAuthorizer, ObligationGoogleEgressPolicyAuthorizer,
    };
    use crate::google_oauth::secret_sink::{
        GoogleProviderRefreshTokenStorageRequest, GoogleProviderStoredTokens,
        GoogleProviderTokenSet, GoogleProviderTokenSink, GoogleProviderTokenStorageRequest,
        SecretStoreGoogleTokenSink,
    };
    use crate::google_oauth::token_response::{parse_token_response, scopes_for_exchange};
    use crate::{RebornAuthContinuationDispatcher, RebornProductAuthServices};
    use async_trait::async_trait;
    use ironclaw_auth::{
        AuthContinuationEvent, AuthFlowId, AuthProductScope, AuthProviderId, AuthSurface,
        AuthorizationCodeHash, CredentialAccountLabel, CredentialAccountLookupRequest,
        CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
        CredentialRefreshRequest, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
        GOOGLE_PROVIDER_ID, GOOGLE_TOKEN_ENDPOINT, InMemoryAuthProductServices,
        NewCredentialAccount, OAuthAuthorizationCode, OAuthClientId, OAuthProviderCallbackRequest,
        OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderRefreshRequest,
        OAuthRedirectUri, PkceVerifierHash, PkceVerifierSecret, ProviderScope,
    };
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_capabilities::{
        CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
    };
    use ironclaw_extensions::ExtensionRegistry;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{
        CapabilityId, ExtensionId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme,
        NetworkTargetPattern, Obligation, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind, SecretHandle, TrustClass,
        UserId,
    };
    use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::ProcessServices;
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::{InMemorySecretStore, SecretStore};
    use secrecy::{ExposeSecret, SecretString};
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct NoopContinuationDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopContinuationDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    #[test]
    fn response_body_parses_to_token_response() {
        let response = parse_token_response(
            br#"{"access_token":"access","refresh_token":"refresh","scope":"repo gmail.readonly","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .expect("response");
        assert!(response.scope_was_present);
        assert_eq!(response.response.scopes.len(), 2);
        assert_eq!(
            scopes_for_exchange(&response).expect("scopes"),
            response.response.scopes
        );
        assert_eq!(response.response.expires_in_seconds, Some(3600));
    }

    #[test]
    fn missing_or_blank_response_scope_fails_closed() {
        for body in [
            br#"{"access_token":"access","expires_in":3600}"#.as_slice(),
            br#"{"access_token":"access","scope":"","expires_in":3600}"#.as_slice(),
            br#"{"access_token":"access","scope":"   ","expires_in":3600}"#.as_slice(),
        ] {
            let response = parse_token_response(body).expect("response");
            assert!(!response.scope_was_present);
            assert_eq!(
                scopes_for_exchange(&response).expect_err("missing response scope must fail"),
                AuthProductError::TokenExchangeFailed
            );
        }
    }

    #[test]
    fn token_response_rejects_empty_or_missing_access_token() {
        assert_eq!(
            parse_token_response(b"").expect_err("empty response"),
            AuthProductError::TokenExchangeFailed
        );
        assert_eq!(
            parse_token_response(br#"{"refresh_token":"refresh"}"#)
                .expect_err("missing access token"),
            AuthProductError::TokenExchangeFailed
        );
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
        .with_response_body_limit(8 * 1024)
        .with_runtime(RuntimeKind::FirstParty)
        .with_timeout_ms(12_345);

        let client_debug = format!("{client:?}");
        assert!(client_debug.contains("Arc<dyn RuntimeHttpEgress>"));
        assert!(client_debug.contains("Arc<dyn GoogleProviderTokenSink>"));
        assert!(client_debug.contains("Arc<dyn GoogleProviderEgressPolicyAuthorizer>"));

        let request = callback_request(google_provider(), label("work gmail"));
        let request_debug = format!("{request:?}");
        assert!(!request_debug.contains("raw-auth-code"));
        assert!(!request_debug.contains("raw-pkce-verifier"));

        let flow_id = AuthFlowId::new();
        let exchange = client
            .exchange_callback(exchange_context(owner, flow_id), request)
            .await
            .expect("exchange");
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
        assert_eq!(request.runtime, RuntimeKind::FirstParty);
        assert_eq!(request.timeout_ms, Some(12_345));
        assert_eq!(request.scope, resource_scope);
        assert_eq!(request.capability_id.as_str(), "ironclaw_auth.google_oauth");
        assert_eq!(request.method, NetworkMethod::Post);
        assert_eq!(request.url, GOOGLE_TOKEN_ENDPOINT);
        let form = token_request_form(&request.body);
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(form.get("code").map(String::as_str), Some("raw-auth-code"));
        assert_eq!(
            form.get("code_verifier").map(String::as_str),
            Some("raw-pkce-verifier")
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("google-client-123")
        );
        assert_eq!(
            form.get("redirect_uri").map(String::as_str),
            Some("https://app.example/oauth/callback")
        );
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
        assert_eq!(sink.flow_ids(), vec![flow_id]);
    }

    #[tokio::test]
    async fn google_provider_fails_closed_when_response_omits_scope() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{"access_token":"provider-access-token","expires_in":3600}"#.to_vec(),
            request_bytes: 0,
            response_bytes: 0,
            saved_body: None,
            redaction_applied: true,
        }));
        let sink = Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-access-secret").expect("valid handle"),
            None,
        ));
        let client = GoogleProviderClient::new(
            egress,
            sink.clone(),
            Arc::new(RecordingPolicyAuthorizer::default()),
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client");

        let requested_scopes =
            provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]);
        let owner = scope("google-provider-scope-fallback");
        let error = client
            .exchange_callback(
                exchange_context(owner, AuthFlowId::new()),
                OAuthProviderCallbackRequest {
                    scopes: requested_scopes.clone(),
                    ..callback_request(google_provider(), label("work gmail"))
                },
            )
            .await
            .expect_err("missing provider scope fails closed");

        assert_eq!(error, AuthProductError::TokenExchangeFailed);
        assert!(sink.access_tokens().is_empty());
    }

    #[tokio::test]
    async fn google_provider_rejects_system_scoped_callbacks_before_side_effects() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![],
            body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"}"#.to_vec(),
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
            .exchange_callback(
                OAuthProviderExchangeContext {
                    scope: AuthProductScope::new(ResourceScope::system(), AuthSurface::Callback),
                    flow_id: AuthFlowId::new(),
                },
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("system-scoped callback is rejected");

        assert_eq!(error, AuthProductError::CrossScopeDenied);
        assert!(egress.requests().is_empty());
        assert!(policy_authorizer.authorizations().is_empty());
        assert!(sink.access_tokens().is_empty());
    }

    #[tokio::test]
    async fn google_provider_rejects_policy_authorizer_failure_before_egress() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![],
            body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"}"#.to_vec(),
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
            Arc::new(FailingPolicyAuthorizer),
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client");

        let error = client
            .exchange_callback(
                exchange_context(scope("google-provider-policy"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("policy failure stops exchange");

        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(egress.requests().is_empty());
    }

    #[tokio::test]
    async fn google_provider_fails_closed_when_policy_is_not_staged() {
        let network = RecordingNetwork::google_token_response();
        let network_requests = network.requests_handle();
        let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let services = test_host_runtime_services()
            .with_secret_store_dyn(Arc::clone(&secret_store))
            .try_with_host_http_egress(network)
            .expect("host egress should wire with graph secret store");
        let runtime_ports = services
            .product_auth_provider_runtime_ports()
            .expect("runtime ports");
        let client = GoogleProviderClient::new(
            runtime_ports.runtime_http_egress(),
            Arc::new(RecordingTokenSink::new(
                SecretHandle::new("google-access-secret").expect("valid handle"),
                None,
            )),
            Arc::new(NoopPolicyAuthorizer),
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client");

        let error = client
            .exchange_callback(
                exchange_context(scope("google-provider-policy-missing"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("unstaged policy must fail closed");

        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(
            network_requests
                .lock()
                .expect("network requests")
                .is_empty()
        );
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
        let client = google_client(egress, Arc::new(RecordingPolicyAuthorizer::default()));

        let error = client
            .exchange_callback(
                exchange_context(scope("google-provider-errors"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("non-2xx response is sanitized");
        assert_eq!(error, AuthProductError::TokenExchangeFailed);
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
        let malformed_client = google_client(
            malformed_egress,
            Arc::new(RecordingPolicyAuthorizer::default()),
        );

        let malformed_error = malformed_client
            .exchange_callback(
                exchange_context(scope("google-provider-malformed"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("malformed response is sanitized");
        assert_eq!(malformed_error, AuthProductError::TokenExchangeFailed);
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
        let client = google_client(egress, Arc::new(RecordingPolicyAuthorizer::default()));

        let error = client
            .exchange_callback(
                exchange_context(scope("google-provider-egress"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("egress failures are sanitized");
        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(!error.to_string().contains("RAW_PROVIDER_ERROR"));
        assert!(!error.to_string().contains("sk-live-secret"));
    }

    #[tokio::test]
    async fn google_provider_rejects_non_google_provider_before_side_effects() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![],
            body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"}"#.to_vec(),
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
            .exchange_callback(
                exchange_context(scope("google-provider-rejects"), AuthFlowId::new()),
                callback_request(provider(), label("work github")),
            )
            .await
            .expect_err("non-google provider is rejected");

        assert_eq!(error, AuthProductError::TokenExchangeFailed);
        assert!(egress.requests().is_empty());
        assert!(policy_authorizer.authorizations().is_empty());
        assert!(sink.access_tokens().is_empty());
    }

    #[tokio::test]
    async fn google_provider_sends_optional_client_secret_without_debug_leakage() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![],
            body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"}"#.to_vec(),
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
            .exchange_callback(
                exchange_context(scope("google-provider-secret"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
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
            body: br#"{"access_token":"provider-access-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"}"#.to_vec(),
            request_bytes: 0,
            response_bytes: 0,
            saved_body: None,
            redaction_applied: false,
        }));
        let client = GoogleProviderClient::new(
            egress,
            Arc::new(FailingTokenSink {
                error: AuthProductError::RefreshFailed,
            }),
            Arc::new(RecordingPolicyAuthorizer::default()),
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client");

        let error = client
            .exchange_callback(
                exchange_context(scope("google-provider-sink"), AuthFlowId::new()),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect_err("sink error is propagated");

        assert_eq!(error, AuthProductError::RefreshFailed);
    }

    #[tokio::test]
    async fn google_token_sink_stores_unique_handles_per_flow() {
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let sink = SecretStoreGoogleTokenSink {
            store: Arc::clone(&store),
        };
        let scope = resource_scope("google-token-owner");
        let first = token_storage_request(scope.clone(), AuthFlowId::new());
        let second = token_storage_request(scope.clone(), AuthFlowId::new());

        let first_stored = sink.store_tokens(first).await.expect("first stored");
        let second_stored = sink.store_tokens(second).await.expect("second stored");

        assert_ne!(first_stored.access_secret, second_stored.access_secret);
        assert_ne!(first_stored.refresh_secret, second_stored.refresh_secret);
        assert!(
            store
                .metadata(&scope, &first_stored.access_secret)
                .await
                .expect("first access metadata")
                .is_some()
        );
        assert!(
            store
                .metadata(&scope, &second_stored.access_secret)
                .await
                .expect("second access metadata")
                .is_some()
        );
    }

    #[tokio::test]
    async fn google_token_sink_stores_access_only_tokens_without_refresh_secret() {
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let sink = SecretStoreGoogleTokenSink {
            store: Arc::clone(&store),
        };
        let scope = resource_scope("google-token-owner-access-only");

        let stored = sink
            .store_tokens(token_storage_request_with_refresh(
                scope.clone(),
                AuthFlowId::new(),
                None,
            ))
            .await
            .expect("access-only token set stored");

        assert!(stored.refresh_secret.is_none());
        assert!(
            store
                .metadata(&scope, &stored.access_secret)
                .await
                .expect("access metadata")
                .is_some()
        );
    }

    #[tokio::test]
    async fn google_token_sink_writes_access_before_refresh() {
        let store = Arc::new(RecordingSecretStore::fail_on_first_put());
        let sink = SecretStoreGoogleTokenSink {
            store: store.clone(),
        };
        let error = sink
            .store_tokens(token_storage_request(
                resource_scope("google-token-store-failure"),
                AuthFlowId::new(),
            ))
            .await
            .expect_err("access write fails before refresh write");

        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert_eq!(store.put_handles(), vec!["google-oauth-access"]);
    }

    #[tokio::test]
    async fn google_token_sink_reports_refresh_write_failure_after_access_write() {
        let store = Arc::new(RecordingSecretStore::fail_on_second_put());
        let sink = SecretStoreGoogleTokenSink {
            store: store.clone(),
        };
        let error = sink
            .store_tokens(token_storage_request(
                resource_scope("google-token-store-refresh-failure"),
                AuthFlowId::new(),
            ))
            .await
            .expect_err("refresh write failure is surfaced");

        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert_eq!(
            store.put_handles(),
            vec!["google-oauth-access", "google-oauth-refresh"]
        );
    }

    #[tokio::test]
    async fn google_provider_client_factory_wires_runtime_ports_and_secret_sink() {
        let network = RecordingNetwork::google_token_response();
        let network_requests = network.requests_handle();
        let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let services = test_host_runtime_services()
            .with_secret_store_dyn(Arc::clone(&secret_store))
            .try_with_host_http_egress(network)
            .expect("host egress should wire with graph secret store");
        let client = google_provider_client(
            oauth_config(),
            Arc::clone(&secret_store),
            services
                .product_auth_provider_runtime_ports()
                .expect("runtime ports"),
        )
        .expect("provider client");
        let owner = scope("google-provider-factory");
        let resource_scope = owner.resource.clone();
        let flow_id = AuthFlowId::new();

        let exchange = client
            .exchange_callback(
                exchange_context(owner.clone(), flow_id),
                callback_request(google_provider(), label("work gmail")),
            )
            .await
            .expect("exchange");

        {
            let requests = network_requests.lock().expect("network requests");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url, GOOGLE_TOKEN_ENDPOINT);
        }
        assert!(
            secret_store
                .metadata(&resource_scope, &exchange.access_secret)
                .await
                .expect("access metadata")
                .is_some()
        );
        let refresh_secret = exchange.refresh_secret.expect("refresh secret");
        assert!(
            secret_store
                .metadata(&resource_scope, &refresh_secret)
                .await
                .expect("refresh metadata")
                .is_some()
        );
    }

    #[tokio::test]
    async fn google_provider_refresh_uses_host_egress_and_stores_access_handle_only() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{
                "access_token":"provider-refreshed-access-token",
                "scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send",
                "expires_in":3600,
                "token_type":"Bearer"
            }"#
            .to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: true,
        }));
        let sink = Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-refreshed-access-secret").expect("valid handle"),
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
        .expect("client")
        .with_runtime(RuntimeKind::FirstParty)
        .with_timeout_ms(12_345);

        let refresh = client
            .refresh_token(refresh_request(google_provider()))
            .await
            .expect("refresh");

        assert_eq!(refresh.provider, google_provider());
        assert_eq!(
            refresh.access_secret,
            SecretHandle::new("google-refreshed-access-secret").unwrap()
        );
        assert!(refresh.refresh_secret.is_none());
        assert_eq!(
            refresh.scopes,
            provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE])
        );
        assert_eq!(
            sink.access_tokens(),
            vec!["provider-refreshed-access-token".to_string()]
        );
        assert!(sink.refresh_tokens().is_empty());

        let requests = egress.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.runtime, RuntimeKind::FirstParty);
        assert_eq!(request.timeout_ms, Some(12_345));
        assert_eq!(request.method, NetworkMethod::Post);
        assert_eq!(request.url, GOOGLE_TOKEN_ENDPOINT);
        let form = token_request_form(&request.body);
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            form.get("refresh_token").map(String::as_str),
            Some("stored-refresh-token")
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("google-client-123")
        );
        assert!(request.network_policy.deny_private_ip_ranges);
        assert_eq!(
            request
                .network_policy
                .allowed_targets
                .iter()
                .map(|target| (target.scheme, target.host_pattern.as_str()))
                .collect::<Vec<_>>(),
            vec![(Some(NetworkScheme::Https), "oauth2.googleapis.com")]
        );
        assert_eq!(policy_authorizer.authorizations().len(), 1);
    }

    #[tokio::test]
    async fn google_provider_refresh_rejects_system_scope_before_side_effects() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{
                "access_token":"provider-refreshed-access-token",
                "scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send",
                "expires_in":3600,
                "token_type":"Bearer"
            }"#
            .to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: true,
        }));
        let sink = Arc::new(RecordingTokenSink::new(
            SecretHandle::new("google-refreshed-access-secret").expect("valid handle"),
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
            .refresh_token(OAuthProviderRefreshRequest {
                provider: google_provider(),
                scope: AuthProductScope::new(ResourceScope::system(), AuthSurface::Callback),
                account_id: ironclaw_auth::CredentialAccountId::new(),
                refresh_secret: SecretHandle::new("google-refresh-secret").expect("valid handle"),
                scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]),
            })
            .await
            .expect_err("system-scoped refresh is rejected");

        assert_eq!(error, AuthProductError::CrossScopeDenied);
        assert!(egress.requests().is_empty());
        assert!(policy_authorizer.authorizations().is_empty());
        assert!(sink.access_tokens().is_empty());
        assert!(sink.deleted_handles().is_empty());
    }

    #[tokio::test]
    async fn google_provider_refresh_maps_http_5xx_to_backend_unavailable() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 503,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{"error":"unavailable"}"#.to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        }));
        let client = google_client(egress, Arc::new(RecordingPolicyAuthorizer::default()));

        let error = client
            .refresh_token(refresh_request(google_provider()))
            .await
            .expect_err("5xx response is backend unavailable");
        assert_eq!(error, AuthProductError::BackendUnavailable);
    }

    #[tokio::test]
    async fn product_auth_refresh_uses_concrete_google_provider_and_updates_account() {
        let owner = scope("google-product-refresh");
        let auth = Arc::new(InMemoryAuthProductServices::new());
        let old_access = SecretHandle::new("google-product-old-access").expect("valid handle");
        let old_refresh = SecretHandle::new("google-product-old-refresh").expect("valid handle");
        let account = CredentialAccountService::create_account(
            auth.as_ref(),
            NewCredentialAccount {
                scope: owner.clone(),
                provider: google_provider(),
                label: label("work gmail"),
                status: CredentialAccountStatus::Expired,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(old_access.clone()),
                refresh_secret: Some(old_refresh.clone()),
                scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
            },
        )
        .await
        .expect("account");
        let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        secret_store
            .put(
                owner.resource.clone(),
                old_refresh.clone(),
                SecretString::from("stored-refresh-token"),
            )
            .await
            .expect("store refresh secret");
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{
                "access_token":"provider-product-refreshed-access-token",
                "scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send",
                "expires_in":3600,
                "token_type":"Bearer"
            }"#
            .to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: true,
        }));
        let google_client: Arc<dyn AuthProviderClient> = Arc::new(
            GoogleProviderClient::new(
                egress.clone(),
                Arc::new(SecretStoreGoogleTokenSink {
                    store: secret_store,
                }),
                Arc::new(RecordingPolicyAuthorizer::default()),
                OAuthClientId::new("google-client-123").expect("client id"),
                OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
            )
            .expect("client"),
        );
        let services = RebornProductAuthServices::from_shared(
            auth.clone(),
            Arc::new(NoopContinuationDispatcher),
        )
        .with_provider_client(google_client);

        let report = services
            .refresh_credential_account(CredentialRefreshRequest::new(
                owner.clone(),
                google_provider(),
                account.id,
            ))
            .await
            .expect("product auth refresh");

        assert!(report.refreshed);
        assert_eq!(report.account.status, CredentialAccountStatus::Configured);
        let stored = CredentialAccountService::get_account(
            auth.as_ref(),
            CredentialAccountLookupRequest::new(owner.clone(), account.id),
        )
        .await
        .expect("stored account")
        .expect("account exists");
        assert_eq!(stored.status, CredentialAccountStatus::Configured);
        assert_ne!(stored.access_secret, Some(old_access));
        assert_eq!(stored.refresh_secret, Some(old_refresh));
        assert_eq!(
            stored.scopes,
            provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE])
        );
        let requests = egress.requests();
        assert_eq!(requests.len(), 1);
        let form = token_request_form(&requests[0].body);
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            form.get("refresh_token").map(String::as_str),
            Some("stored-refresh-token")
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("google-client-123")
        );
    }

    #[tokio::test]
    async fn google_provider_refresh_sanitizes_provider_and_egress_failures() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 400,
            headers: Vec::new(),
            body: br#"{"error":"invalid_grant","error_description":"raw refresh body"}"#.to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        }));
        let client = google_client(
            Arc::clone(&egress),
            Arc::new(RecordingPolicyAuthorizer::default()),
        );

        let google_error = client
            .refresh_token(refresh_request(google_provider()))
            .await
            .expect_err("google refresh failure is sanitized");
        assert_eq!(google_error, AuthProductError::RefreshFailed);
        assert!(!google_error.to_string().contains("raw refresh body"));

        let egress_error = Arc::new(RecordingEgress::err(RuntimeHttpEgressError::Network {
            reason: "raw refresh network secret".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        }));
        let client = google_client(egress_error, Arc::new(RecordingPolicyAuthorizer::default()));
        let error = client
            .refresh_token(refresh_request(google_provider()))
            .await
            .expect_err("egress failure is sanitized");
        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert!(!error.to_string().contains("raw refresh network secret"));
    }

    #[tokio::test]
    async fn google_provider_refresh_rejects_non_google_provider_before_egress() {
        let egress = Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        }));
        let client = google_client(
            Arc::clone(&egress),
            Arc::new(RecordingPolicyAuthorizer::default()),
        );
        let non_google_error = client
            .refresh_token(refresh_request(provider()))
            .await
            .expect_err("non-google refresh is rejected");
        assert_eq!(non_google_error, AuthProductError::RefreshFailed);
        assert!(egress.requests().is_empty());
    }

    #[tokio::test]
    async fn google_provider_cleanup_exchange_deletes_google_handles_and_skips_non_google() {
        let access_handle =
            SecretHandle::new("google-cleanup-access-secret").expect("valid handle");
        let refresh_handle =
            SecretHandle::new("google-cleanup-refresh-secret").expect("valid handle");
        let sink = Arc::new(RecordingTokenSink::new(
            access_handle.clone(),
            Some(refresh_handle.clone()),
        ));
        let client = GoogleProviderClient::new(
            Arc::new(RecordingEgress::ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: vec![],
                body: Vec::new(),
                request_bytes: 0,
                response_bytes: 0,
                saved_body: None,
                redaction_applied: false,
            })),
            sink.clone(),
            Arc::new(RecordingPolicyAuthorizer::default()),
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client");

        let google_exchange = OAuthProviderExchange {
            provider: google_provider(),
            account_label: label("work gmail"),
            authorization_code_hash: code_hash("cleanup-code-hash"),
            pkce_verifier_hash: pkce_hash("cleanup-pkce-hash"),
            access_secret: access_handle.clone(),
            refresh_secret: Some(refresh_handle.clone()),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]),
            account_id: None,
        };
        client
            .cleanup_exchange(
                exchange_context(scope("google-cleanup"), AuthFlowId::new()),
                &google_exchange,
            )
            .await
            .expect("google cleanup");
        assert_eq!(
            sink.deleted_handles(),
            vec![vec![access_handle.clone(), refresh_handle.clone()]]
        );

        sink.clear_deleted_handles();

        let non_google_exchange = OAuthProviderExchange {
            provider: provider(),
            account_label: label("work github"),
            authorization_code_hash: code_hash("cleanup-non-google-code-hash"),
            pkce_verifier_hash: pkce_hash("cleanup-non-google-pkce-hash"),
            access_secret: access_handle,
            refresh_secret: Some(refresh_handle),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE]),
            account_id: None,
        };
        client
            .cleanup_exchange(
                exchange_context(scope("google-cleanup-non-google"), AuthFlowId::new()),
                &non_google_exchange,
            )
            .await
            .expect("non-google cleanup");
        assert!(sink.deleted_handles().is_empty());
    }

    #[tokio::test]
    async fn google_egress_authorizer_stages_policy_as_system_auth_capability() {
        let handler = Arc::new(RecordingObligationHandler::default());
        let handler_dyn: Arc<dyn CapabilityObligationHandler> = handler.clone();
        let authorizer = ObligationGoogleEgressPolicyAuthorizer {
            handler: handler_dyn,
        };
        let scope = resource_scope("google-egress-owner");
        let capability_id = CapabilityId::new("ironclaw_auth.google_oauth").expect("capability");
        let policy = NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "oauth2.googleapis.com".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        };

        authorizer
            .authorize_google_token_exchange(&scope, &capability_id, &policy)
            .await
            .expect("policy staged");

        let requests = handler.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.phase, CapabilityObligationPhase::Invoke);
        assert_eq!(request.resource_scope, scope);
        assert_eq!(request.extension_id.as_str(), "ironclaw_auth");
        assert_eq!(request.runtime, RuntimeKind::System);
        assert_eq!(request.trust, TrustClass::System);
        assert_eq!(request.capability_id, capability_id);
        assert_eq!(
            request.obligations,
            vec![Obligation::ApplyNetworkPolicy { policy }]
        );
    }

    fn google_client(
        egress: Arc<RecordingEgress>,
        authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
    ) -> GoogleProviderClient {
        GoogleProviderClient::new(
            egress,
            Arc::new(RecordingTokenSink::new(
                SecretHandle::new("google-access-secret").expect("valid handle"),
                None,
            )),
            authorizer,
            OAuthClientId::new("google-client-123").expect("client id"),
            OAuthRedirectUri::new("https://app.example/oauth/callback").expect("redirect uri"),
        )
        .expect("client")
    }

    fn token_storage_request(
        scope: ResourceScope,
        flow_id: AuthFlowId,
    ) -> GoogleProviderTokenStorageRequest {
        token_storage_request_with_refresh(
            scope,
            flow_id,
            Some(SecretString::from("refresh-token")),
        )
    }

    fn token_storage_request_with_refresh(
        scope: ResourceScope,
        flow_id: AuthFlowId,
        refresh_token: Option<SecretString>,
    ) -> GoogleProviderTokenStorageRequest {
        GoogleProviderTokenStorageRequest {
            scope,
            flow_id,
            tokens: GoogleProviderTokenSet {
                access_token: SecretString::from("access-token"),
                refresh_token,
            },
        }
    }

    fn oauth_config() -> OAuthClientConfig {
        OAuthClientConfig {
            client_id: OAuthClientId::new("google-client-123").expect("client id"),
            client_secret: None,
            redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/callback")
                .expect("redirect uri"),
        }
    }

    fn test_host_runtime_services() -> HostRuntimeServices<
        LocalFilesystem,
        InMemoryResourceGovernor,
        ironclaw_processes::InMemoryProcessStore,
        ironclaw_processes::InMemoryProcessResultStore,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        )
    }

    fn exchange_context(
        scope: AuthProductScope,
        flow_id: AuthFlowId,
    ) -> OAuthProviderExchangeContext {
        OAuthProviderExchangeContext { scope, flow_id }
    }

    fn callback_request(
        provider: AuthProviderId,
        account_label: CredentialAccountLabel,
    ) -> OAuthProviderCallbackRequest {
        OAuthProviderCallbackRequest {
            provider,
            account_label,
            authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code"))
                .expect("valid code"),
            authorization_code_hash: code_hash("code-hash"),
            pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier"))
                .expect("valid verifier"),
            pkce_verifier_hash: pkce_hash("pkce-hash"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]),
        }
    }

    fn refresh_request(provider: AuthProviderId) -> OAuthProviderRefreshRequest {
        let scope = scope("google-refresh");
        OAuthProviderRefreshRequest {
            provider,
            scope,
            account_id: ironclaw_auth::CredentialAccountId::new(),
            refresh_secret: SecretHandle::new("google-refresh-secret").expect("valid handle"),
            scopes: provider_scopes(&[GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]),
        }
    }

    fn token_request_form(body: &[u8]) -> BTreeMap<String, String> {
        url::form_urlencoded::parse(body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    fn scope(user: &str) -> AuthProductScope {
        AuthProductScope::new(resource_scope(user), AuthSurface::Callback)
    }

    fn resource_scope(user: &str) -> ResourceScope {
        ResourceScope::local_default(UserId::new(user).expect("valid user"), InvocationId::new())
            .expect("valid scope")
    }

    fn google_provider() -> AuthProviderId {
        AuthProviderId::new(GOOGLE_PROVIDER_ID).expect("provider")
    }

    fn provider() -> AuthProviderId {
        AuthProviderId::new("github").expect("provider")
    }

    fn label(value: &str) -> CredentialAccountLabel {
        CredentialAccountLabel::new(value).expect("label")
    }

    fn code_hash(value: &str) -> AuthorizationCodeHash {
        AuthorizationCodeHash::new(fake_digest(value)).expect("code hash")
    }

    fn pkce_hash(value: &str) -> PkceVerifierHash {
        PkceVerifierHash::new(fake_digest(value)).expect("pkce hash")
    }

    fn fake_digest(value: &str) -> String {
        format!(
            "{:064x}",
            value.bytes().fold(0_u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u64::from(byte))
            })
        )
    }

    fn provider_scopes(values: &[&str]) -> Vec<ProviderScope> {
        values
            .iter()
            .map(|value| ProviderScope::new(*value).expect("scope"))
            .collect()
    }

    fn secret(value: &str) -> SecretString {
        SecretString::from(value.to_string())
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

    #[async_trait::async_trait]
    impl RuntimeHttpEgress for RecordingEgress {
        async fn execute(
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

    struct RecordingNetwork {
        response_body: Vec<u8>,
        requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
    }

    impl RecordingNetwork {
        fn google_token_response() -> Self {
            Self {
                response_body: br#"{"access_token":"provider-access-token","refresh_token":"provider-refresh-token","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send","expires_in":3600,"token_type":"Bearer"}"#.to_vec(),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests_handle(&self) -> Arc<Mutex<Vec<NetworkHttpRequest>>> {
            Arc::clone(&self.requests)
        }
    }

    #[async_trait::async_trait]
    impl NetworkHttpEgress for RecordingNetwork {
        async fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.requests
                .lock()
                .expect("network requests")
                .push(request.clone());
            Ok(NetworkHttpResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: self.response_body.clone(),
                usage: NetworkUsage {
                    request_bytes: request.body.len() as u64,
                    response_bytes: self.response_body.len() as u64,
                    resolved_ip: None,
                },
            })
        }
    }

    struct RecordingTokenSink {
        scopes: Mutex<Vec<ResourceScope>>,
        flow_ids: Mutex<Vec<AuthFlowId>>,
        access_tokens: Mutex<Vec<String>>,
        refresh_tokens: Mutex<Vec<String>>,
        deleted_handles: Mutex<Vec<Vec<SecretHandle>>>,
        access_handle: SecretHandle,
        refresh_handle: Option<SecretHandle>,
    }

    impl RecordingTokenSink {
        fn new(access_handle: SecretHandle, refresh_handle: Option<SecretHandle>) -> Self {
            Self {
                scopes: Mutex::new(Vec::new()),
                flow_ids: Mutex::new(Vec::new()),
                access_tokens: Mutex::new(Vec::new()),
                refresh_tokens: Mutex::new(Vec::new()),
                deleted_handles: Mutex::new(Vec::new()),
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

        fn flow_ids(&self) -> Vec<AuthFlowId> {
            self.flow_ids.lock().expect("flow ids").clone()
        }

        fn refresh_tokens(&self) -> Vec<String> {
            self.refresh_tokens.lock().expect("refresh tokens").clone()
        }

        fn deleted_handles(&self) -> Vec<Vec<SecretHandle>> {
            self.deleted_handles
                .lock()
                .expect("deleted handles")
                .clone()
        }

        fn clear_deleted_handles(&self) {
            self.deleted_handles
                .lock()
                .expect("deleted handles")
                .clear();
        }
    }

    #[async_trait]
    impl GoogleProviderTokenSink for RecordingTokenSink {
        async fn store_tokens(
            &self,
            request: GoogleProviderTokenStorageRequest,
        ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
            self.scopes
                .lock()
                .expect("scopes")
                .push(request.scope.clone());
            self.flow_ids
                .lock()
                .expect("flow ids")
                .push(request.flow_id);
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

        async fn store_refreshed_tokens(
            &self,
            request: GoogleProviderRefreshTokenStorageRequest,
        ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
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

        async fn load_refresh_token(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<SecretString, AuthProductError> {
            Ok(secret("stored-refresh-token"))
        }

        async fn delete_tokens(
            &self,
            _scope: &ResourceScope,
            handles: &[SecretHandle],
        ) -> Result<(), AuthProductError> {
            self.deleted_handles
                .lock()
                .expect("deleted handles")
                .push(handles.to_vec());
            Ok(())
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

    #[async_trait]
    impl GoogleProviderEgressPolicyAuthorizer for RecordingPolicyAuthorizer {
        async fn authorize_google_token_exchange(
            &self,
            scope: &ResourceScope,
            capability_id: &CapabilityId,
            policy: &NetworkPolicy,
        ) -> Result<(), AuthProductError> {
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

    struct FailingPolicyAuthorizer;

    #[async_trait]
    impl GoogleProviderEgressPolicyAuthorizer for FailingPolicyAuthorizer {
        async fn authorize_google_token_exchange(
            &self,
            _scope: &ResourceScope,
            _capability_id: &CapabilityId,
            _policy: &NetworkPolicy,
        ) -> Result<(), AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }
    }

    struct NoopPolicyAuthorizer;

    #[async_trait]
    impl GoogleProviderEgressPolicyAuthorizer for NoopPolicyAuthorizer {
        async fn authorize_google_token_exchange(
            &self,
            _scope: &ResourceScope,
            _capability_id: &CapabilityId,
            _policy: &NetworkPolicy,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    struct FailingTokenSink {
        error: AuthProductError,
    }

    #[async_trait]
    impl GoogleProviderTokenSink for FailingTokenSink {
        async fn store_tokens(
            &self,
            _request: GoogleProviderTokenStorageRequest,
        ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
            Err(self.error.clone())
        }

        async fn store_refreshed_tokens(
            &self,
            _request: GoogleProviderRefreshTokenStorageRequest,
        ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
            Err(self.error.clone())
        }

        async fn load_refresh_token(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<SecretString, AuthProductError> {
            Ok(secret("stored-refresh-token"))
        }

        async fn delete_tokens(
            &self,
            _scope: &ResourceScope,
            _handles: &[SecretHandle],
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct RecordingSecretStore {
        fail_on_put: Option<usize>,
        put_handles: Mutex<Vec<String>>,
    }

    impl RecordingSecretStore {
        fn fail_on_first_put() -> Self {
            Self {
                fail_on_put: Some(1),
                put_handles: Mutex::new(Vec::new()),
            }
        }

        fn fail_on_second_put() -> Self {
            Self {
                fail_on_put: Some(2),
                put_handles: Mutex::new(Vec::new()),
            }
        }

        fn put_handles(&self) -> Vec<&'static str> {
            self.put_handles
                .lock()
                .expect("put handles")
                .iter()
                .map(|handle| {
                    if handle.contains("refresh") {
                        "google-oauth-refresh"
                    } else {
                        "google-oauth-access"
                    }
                })
                .collect()
        }
    }

    #[async_trait]
    impl SecretStore for RecordingSecretStore {
        async fn put(
            &self,
            _scope: ResourceScope,
            handle: SecretHandle,
            _material: SecretString,
        ) -> Result<ironclaw_secrets::SecretMetadata, ironclaw_secrets::SecretStoreError> {
            let mut handles = self.put_handles.lock().expect("put handles");
            handles.push(handle.to_string());
            if self.fail_on_put == Some(handles.len()) {
                return Err(ironclaw_secrets::SecretStoreError::StoreUnavailable {
                    reason: "injected failure".to_string(),
                });
            }
            Ok(ironclaw_secrets::SecretMetadata {
                scope: resource_scope("recording-secret-store"),
                handle,
            })
        }

        async fn metadata(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<Option<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            Ok(None)
        }

        async fn delete(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<bool, ironclaw_secrets::SecretStoreError> {
            Ok(true)
        }

        async fn lease_once(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unimplemented!("not used by google oauth token-sink tests")
        }

        async fn consume(
            &self,
            _scope: &ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<SecretString, ironclaw_secrets::SecretStoreError> {
            unimplemented!("not used by google oauth token-sink tests")
        }

        async fn revoke(
            &self,
            _scope: &ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unimplemented!("not used by google oauth token-sink tests")
        }

        async fn leases_for_scope(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretLease>, ironclaw_secrets::SecretStoreError>
        {
            unimplemented!("not used by google oauth token-sink tests")
        }
    }

    #[derive(Debug, Default)]
    struct RecordingObligationHandler {
        requests: Mutex<Vec<RecordedObligationRequest>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedObligationRequest {
        phase: CapabilityObligationPhase,
        resource_scope: ResourceScope,
        extension_id: ExtensionId,
        runtime: RuntimeKind,
        trust: TrustClass,
        capability_id: CapabilityId,
        obligations: Vec<Obligation>,
    }

    impl RecordingObligationHandler {
        fn requests(&self) -> Vec<RecordedObligationRequest> {
            self.requests.lock().expect("requests").clone()
        }
    }

    #[async_trait]
    impl CapabilityObligationHandler for RecordingObligationHandler {
        async fn satisfy(
            &self,
            request: CapabilityObligationRequest<'_>,
        ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
            self.requests
                .lock()
                .expect("requests")
                .push(RecordedObligationRequest {
                    phase: request.phase,
                    resource_scope: request.context.resource_scope.clone(),
                    extension_id: request.context.extension_id.clone(),
                    runtime: request.context.runtime,
                    trust: request.context.trust,
                    capability_id: request.capability_id.clone(),
                    obligations: request.obligations.to_vec(),
                });
            Ok(())
        }
    }
}
