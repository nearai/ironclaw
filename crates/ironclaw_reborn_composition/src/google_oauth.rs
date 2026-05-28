use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderClient, GOOGLE_PROVIDER_ID, GOOGLE_TOKEN_ENDPOINT,
    GoogleProviderEgressPolicyAuthorizer, GoogleProviderStoredTokens, GoogleProviderTokenSet,
    GoogleProviderTokenSink, GoogleProviderTokenStorageRequest, OAuthClientId,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, OAuthRedirectUri, OAuthTokenResponse,
    ProviderScope, validate_provider_callback_request,
};
use ironclaw_capabilities::{
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_events::InMemoryAuditSink;
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, CorrelationId, ExtensionId, MountView, NetworkMethod,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Obligation, ResourceEstimate,
    ResourceScope, RuntimeCredentialInjection, RuntimeHttpEgress, RuntimeHttpEgressRequest,
    RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_host_runtime::BuiltinObligationServices;
use ironclaw_resources::ResourceGovernor;
use ironclaw_secrets::SecretStore;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::form_urlencoded::Serializer;

use crate::RebornBuildError;
use crate::input::OAuthClientConfig;

const GOOGLE_OAUTH_CAPABILITY: &str = "ironclaw_auth.google_oauth";
const DEFAULT_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 16 * 1024;

/// Concrete Google OAuth token-exchange client for Reborn composition.
///
/// The auth crate owns the provider-exchange contracts; this adapter owns the
/// host-runtime HTTP egress, network-policy handoff, and secret-store bridge.
#[derive(Clone)]
pub(crate) struct GoogleProviderClient {
    egress: Arc<dyn RuntimeHttpEgress>,
    token_sink: Arc<dyn GoogleProviderTokenSink>,
    egress_policy_authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
    client_id: OAuthClientId,
    client_secret: Option<SecretString>,
    redirect_uri: OAuthRedirectUri,
    runtime: RuntimeKind,
    capability_id: CapabilityId,
    timeout_ms: u32,
    response_body_limit: u64,
}

impl GoogleProviderClient {
    pub(crate) fn new(
        egress: Arc<dyn RuntimeHttpEgress>,
        token_sink: Arc<dyn GoogleProviderTokenSink>,
        egress_policy_authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer>,
        client_id: OAuthClientId,
        redirect_uri: OAuthRedirectUri,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            egress,
            token_sink,
            egress_policy_authorizer,
            client_id,
            client_secret: None,
            redirect_uri,
            runtime: RuntimeKind::System,
            capability_id: CapabilityId::new(GOOGLE_OAUTH_CAPABILITY)
                .map_err(|_| AuthProductError::BackendUnavailable)?,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            response_body_limit: DEFAULT_RESPONSE_BODY_LIMIT,
        })
    }

    #[cfg(test)]
    fn with_runtime(mut self, runtime: RuntimeKind) -> Self {
        self.runtime = runtime;
        self
    }

    #[cfg(test)]
    fn with_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    #[cfg(test)]
    fn with_response_body_limit(mut self, response_body_limit: u64) -> Self {
        self.response_body_limit = response_body_limit;
        self
    }

    pub(crate) fn with_client_secret(mut self, client_secret: SecretString) -> Self {
        self.client_secret = Some(client_secret);
        self
    }
}

impl fmt::Debug for GoogleProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleProviderClient")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("runtime", &self.runtime)
            .field("capability_id", &self.capability_id)
            .field("timeout_ms", &self.timeout_ms)
            .field("response_body_limit", &self.response_body_limit)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("egress", &"Arc<dyn RuntimeHttpEgress>")
            .field("token_sink", &"Arc<dyn GoogleProviderTokenSink>")
            .field(
                "egress_policy_authorizer",
                &"Arc<dyn GoogleProviderEgressPolicyAuthorizer>",
            )
            .finish()
    }
}

#[async_trait]
impl AuthProviderClient for GoogleProviderClient {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        if request.provider.as_str() != GOOGLE_PROVIDER_ID {
            return Err(AuthProductError::TokenExchangeFailed);
        }
        validate_provider_callback_request(&request)?;
        let callback_scope = context.scope.resource.clone();
        if callback_scope.is_system() {
            return Err(AuthProductError::CrossScopeDenied);
        }

        let body = serialize_token_request(
            self.client_id.as_str(),
            self.redirect_uri.as_str(),
            self.client_secret.as_ref(),
            request.authorization_code.expose_secret(),
            request.pkce_verifier.expose_secret(),
        );
        let network_policy = google_token_network_policy(self.response_body_limit);
        self.egress_policy_authorizer
            .authorize_google_token_exchange(&callback_scope, &self.capability_id, &network_policy)
            .await?;

        let egress = Arc::clone(&self.egress);
        let egress_request = RuntimeHttpEgressRequest {
            runtime: self.runtime,
            scope: callback_scope.clone(),
            capability_id: self.capability_id.clone(),
            method: NetworkMethod::Post,
            url: GOOGLE_TOKEN_ENDPOINT.to_string(),
            headers: vec![
                (
                    "content-type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
                ("accept".to_string(), "application/json".to_string()),
            ],
            body,
            network_policy,
            credential_injections: Vec::<RuntimeCredentialInjection>::new(),
            response_body_limit: Some(self.response_body_limit),
            save_body_to: None,
            timeout_ms: Some(self.timeout_ms),
        };
        let response = tokio::task::spawn_blocking(move || egress.execute(egress_request))
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        let response = response.map_err(|_| AuthProductError::BackendUnavailable)?;

        if !(200..300).contains(&response.status) {
            return Err(AuthProductError::TokenExchangeFailed);
        }

        let token_response = parse_token_response(&response.body)?;
        let scopes = scopes_for_exchange(&token_response)?;
        let token_sink = Arc::clone(&self.token_sink);
        let stored_tokens = token_sink
            .store_tokens(GoogleProviderTokenStorageRequest {
                scope: callback_scope,
                flow_id: context.flow_id,
                tokens: GoogleProviderTokenSet {
                    access_token: token_response.response.access_token,
                    refresh_token: token_response.response.refresh_token,
                },
            })
            .await?;

        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: stored_tokens.access_secret,
            refresh_secret: stored_tokens.refresh_secret,
            scopes,
            account_id: None,
        })
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(AuthProductError::RefreshFailed)
    }
}

pub(crate) fn google_provider_client(
    config: OAuthClientConfig,
    secret_store: Arc<dyn SecretStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
) -> Result<Arc<dyn AuthProviderClient>, RebornBuildError> {
    let obligation_services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        secret_store.clone(),
        resource_governor,
    );
    let egress: Arc<dyn RuntimeHttpEgress> = Arc::new(obligation_services.host_http_egress(
        ironclaw_network::PolicyNetworkHttpEgress::new(
            ironclaw_network::ReqwestNetworkTransport::default(),
        ),
    ));
    let token_sink: Arc<dyn GoogleProviderTokenSink> = Arc::new(SecretStoreGoogleTokenSink {
        store: secret_store,
    });
    let authorizer: Arc<dyn GoogleProviderEgressPolicyAuthorizer> =
        Arc::new(ObligationGoogleEgressPolicyAuthorizer {
            handler: Arc::new(obligation_services.obligation_handler()),
        });
    let mut client = GoogleProviderClient::new(
        egress,
        token_sink,
        authorizer,
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

struct SecretStoreGoogleTokenSink {
    store: Arc<dyn SecretStore>,
}

#[async_trait]
impl GoogleProviderTokenSink for SecretStoreGoogleTokenSink {
    async fn store_tokens(
        &self,
        request: GoogleProviderTokenStorageRequest,
    ) -> Result<GoogleProviderStoredTokens, AuthProductError> {
        let access_secret = google_token_handle(&request, "access")?;
        let refresh_handle = request
            .tokens
            .refresh_token
            .as_ref()
            .map(|_| google_token_handle(&request, "refresh"))
            .transpose()?;
        let GoogleProviderTokenStorageRequest {
            scope,
            tokens,
            flow_id: _,
        } = request;
        let GoogleProviderTokenSet {
            access_token,
            refresh_token,
        } = tokens;
        self.store
            .put(scope.clone(), access_secret.clone(), access_token)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?;

        let refresh_secret = match (refresh_handle, refresh_token) {
            (Some(handle), Some(refresh_token)) => {
                self.store
                    .put(scope.clone(), handle.clone(), refresh_token)
                    .await
                    .map_err(|_| AuthProductError::BackendUnavailable)?;
                Some(handle)
            }
            (None, None) => None,
            _ => return Err(AuthProductError::BackendUnavailable),
        };

        Ok(GoogleProviderStoredTokens {
            access_secret,
            refresh_secret,
        })
    }
}

fn google_token_handle(
    request: &GoogleProviderTokenStorageRequest,
    token_kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "google-oauth-{token_kind}-{}-{}",
        request.flow_id, request.scope.invocation_id
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

struct ObligationGoogleEgressPolicyAuthorizer {
    handler: Arc<dyn CapabilityObligationHandler>,
}

#[async_trait]
impl GoogleProviderEgressPolicyAuthorizer for ObligationGoogleEgressPolicyAuthorizer {
    async fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), AuthProductError> {
        let context = google_oauth_execution_context(scope.clone())?;
        let estimate = ResourceEstimate {
            network_egress_bytes: policy.max_egress_bytes,
            ..ResourceEstimate::default()
        };
        self.handler
            .satisfy(CapabilityObligationRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id,
                estimate: &estimate,
                obligations: &[Obligation::ApplyNetworkPolicy {
                    policy: policy.clone(),
                }],
            })
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)
    }
}

fn google_oauth_execution_context(
    resource_scope: ResourceScope,
) -> Result<ironclaw_host_api::ExecutionContext, AuthProductError> {
    let context = ironclaw_host_api::ExecutionContext {
        invocation_id: resource_scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("ironclaw_auth")
            .map_err(|_| AuthProductError::BackendUnavailable)?,
        runtime: RuntimeKind::System,
        trust: TrustClass::System,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    };
    context
        .validate()
        .map_err(|_| AuthProductError::BackendUnavailable)?;
    Ok(context)
}

fn google_token_network_policy(response_body_limit: u64) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "oauth2.googleapis.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(response_body_limit),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct GoogleTokenResponseBody {
    access_token: SecretString,
    #[serde(default)]
    refresh_token: Option<SecretString>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug)]
struct ParsedGoogleTokenResponse {
    response: OAuthTokenResponse,
    scope_was_present: bool,
}

fn parse_token_response(body: &[u8]) -> Result<ParsedGoogleTokenResponse, AuthProductError> {
    let parsed: GoogleTokenResponseBody =
        serde_json::from_slice(body).map_err(|_| AuthProductError::TokenExchangeFailed)?;
    let response_scope = parsed
        .scope
        .as_deref()
        .filter(|scope| !scope.trim().is_empty());
    let scope_was_present = response_scope.is_some();
    let response = OAuthTokenResponse::new(
        parsed.access_token,
        parsed.refresh_token,
        response_scope,
        parsed.expires_in,
    )
    .map_err(|_| AuthProductError::TokenExchangeFailed)?;

    let _ = parsed.token_type;
    Ok(ParsedGoogleTokenResponse {
        response,
        scope_was_present,
    })
}

fn scopes_for_exchange(
    token_response: &ParsedGoogleTokenResponse,
) -> Result<Vec<ProviderScope>, AuthProductError> {
    if token_response.scope_was_present {
        Ok(token_response.response.scopes.clone())
    } else {
        Err(AuthProductError::TokenExchangeFailed)
    }
}

fn serialize_token_request(
    client_id: &str,
    redirect_uri: &str,
    client_secret: Option<&SecretString>,
    authorization_code: &str,
    pkce_verifier: &str,
) -> Vec<u8> {
    let mut serializer = Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", authorization_code)
        .append_pair("code_verifier", pkce_verifier)
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_auth::{
        AuthFlowId, AuthProductScope, AuthProviderId, AuthSurface, AuthorizationCodeHash,
        CredentialAccountLabel, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
        OAuthAuthorizationCode, PkceVerifierHash, PkceVerifierSecret,
    };
    use ironclaw_host_api::{
        InvocationId, RuntimeHttpEgressError, RuntimeHttpEgressResponse, UserId,
    };
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::ExposeSecret;
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

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
    async fn google_provider_refresh_token_returns_refresh_failed_without_egress() {
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

        let google_error = client
            .refresh_token(refresh_request(google_provider()))
            .await
            .expect_err("google refresh is unsupported");
        assert_eq!(google_error, AuthProductError::RefreshFailed);

        let non_google_error = client
            .refresh_token(refresh_request(provider()))
            .await
            .expect_err("non-google refresh is unsupported");
        assert_eq!(non_google_error, AuthProductError::RefreshFailed);
        assert!(egress.requests().is_empty());
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
        GoogleProviderTokenStorageRequest {
            scope,
            flow_id,
            tokens: GoogleProviderTokenSet {
                access_token: SecretString::from("access-token"),
                refresh_token: Some(SecretString::from("refresh-token")),
            },
        }
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
        OAuthProviderRefreshRequest {
            provider,
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
        flow_ids: Mutex<Vec<AuthFlowId>>,
        access_tokens: Mutex<Vec<String>>,
        refresh_tokens: Mutex<Vec<String>>,
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
