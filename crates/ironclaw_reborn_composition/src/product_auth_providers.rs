use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderClient, GOOGLE_PROVIDER_ID, OAuthProviderCallbackRequest,
    OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderRefresh,
    OAuthProviderRefreshRequest,
};
use ironclaw_capabilities::CapabilityObligationHandler;
use ironclaw_host_api::RuntimeHttpEgress;
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use ironclaw_secrets::SecretStore;

use crate::RebornBuildError;
use crate::input::{OAuthDcrProviderBackendConfig, OAuthProviderBackendConfig};
use crate::oauth_dcr::{OAuthDcrProvider, OAuthDcrProviderRegistry};
use crate::oauth_gate::{GoogleOAuthGateProvider, GoogleOAuthGateProviderRegistry};
use crate::oauth_provider_client::HostOAuthProviderClient;

#[derive(Clone)]
pub(crate) struct OAuthProviderComposition {
    pub(crate) client: Option<Arc<dyn AuthProviderClient>>,
    pub(crate) dcr_registry: Option<Arc<OAuthDcrProviderRegistry>>,
    pub(crate) gate_registry: Option<Arc<GoogleOAuthGateProviderRegistry>>,
}

pub(crate) fn compose_provider_client(
    configs: Vec<OAuthProviderBackendConfig>,
    dcr_configs: Vec<OAuthDcrProviderBackendConfig>,
    secret_store: Arc<dyn SecretStore>,
    runtime_ports: ProductAuthProviderRuntimePorts,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    compose_provider_client_with_runtime(
        configs,
        dcr_configs,
        secret_store,
        OAuthProviderRuntimePorts::from_product_auth_ports(runtime_ports),
    )
}

fn compose_provider_client_with_runtime(
    configs: Vec<OAuthProviderBackendConfig>,
    dcr_configs: Vec<OAuthDcrProviderBackendConfig>,
    secret_store: Arc<dyn SecretStore>,
    runtime_ports: OAuthProviderRuntimePorts,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let mut clients = Vec::new();
    let mut gate_providers = Vec::new();
    for config in configs {
        let provider_id = config.spec.provider_id;
        if provider_id == GOOGLE_PROVIDER_ID {
            gate_providers.push(Arc::new(GoogleOAuthGateProvider::new(
                config.client.clone(),
                Arc::clone(&secret_store),
            )));
        }
        let mut client = HostOAuthProviderClient::new(
            config.spec,
            runtime_ports.runtime_http_egress(),
            Arc::clone(&secret_store),
            runtime_ports.obligation_handler(),
            config.client.client_id,
            config.client.redirect_uri,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "{provider_id} OAuth provider backend could not be configured: {error}"
            ),
        })?;
        if let Some(client_secret) = config.client.client_secret {
            client = client.with_client_secret(client_secret);
        }
        clients.push((provider_id, Arc::new(client) as Arc<dyn AuthProviderClient>));
    }
    let mut dcr_providers = Vec::new();
    for config in dcr_configs {
        let provider_id = config.config.spec.provider_id;
        let provider = Arc::new(
            OAuthDcrProvider::new(
                config.config,
                runtime_ports.runtime_http_egress(),
                Arc::clone(&secret_store),
                runtime_ports.obligation_handler(),
            )
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "{provider_id} DCR OAuth provider backend could not be configured: {error}"
                ),
            })?,
        );
        let client = HostOAuthProviderClient::new_with_client_material(
            provider.spec().clone(),
            runtime_ports.runtime_http_egress(),
            Arc::clone(&secret_store),
            runtime_ports.obligation_handler(),
            provider.clone(),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "{provider_id} DCR OAuth provider backend could not be configured: {error}"
            ),
        })?;
        clients.push((provider_id, Arc::new(client) as Arc<dyn AuthProviderClient>));
        dcr_providers.push(provider);
    }
    let dcr_registry =
        (!dcr_providers.is_empty()).then(|| Arc::new(OAuthDcrProviderRegistry::new(dcr_providers)));
    let gate_registry = (!gate_providers.is_empty())
        .then(|| Arc::new(GoogleOAuthGateProviderRegistry::new(gate_providers)));
    Ok(OAuthProviderComposition {
        client: compose_provider_clients(clients),
        dcr_registry,
        gate_registry,
    })
}

#[derive(Clone)]
struct OAuthProviderRuntimePorts {
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
}

impl OAuthProviderRuntimePorts {
    fn from_product_auth_ports(ports: ProductAuthProviderRuntimePorts) -> Self {
        Self {
            runtime_http_egress: ports.runtime_http_egress(),
            obligation_handler: ports.obligation_handler(),
        }
    }

    #[cfg(test)]
    fn new(
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
    ) -> Self {
        Self {
            runtime_http_egress,
            obligation_handler,
        }
    }

    fn runtime_http_egress(&self) -> Arc<dyn RuntimeHttpEgress> {
        Arc::clone(&self.runtime_http_egress)
    }

    fn obligation_handler(&self) -> Arc<dyn CapabilityObligationHandler> {
        Arc::clone(&self.obligation_handler)
    }
}

fn compose_provider_clients(
    clients: Vec<(&'static str, Arc<dyn AuthProviderClient>)>,
) -> Option<Arc<dyn AuthProviderClient>> {
    if clients.is_empty() {
        return None;
    }
    Some(Arc::new(MultiplexAuthProviderClient::from_clients(clients)))
}

#[derive(Default)]
struct MultiplexAuthProviderClient {
    providers: BTreeMap<String, Arc<dyn AuthProviderClient>>,
}

impl MultiplexAuthProviderClient {
    fn from_clients(clients: Vec<(&'static str, Arc<dyn AuthProviderClient>)>) -> Self {
        Self {
            providers: clients
                .into_iter()
                .map(|(provider, client)| (provider.to_string(), client))
                .collect(),
        }
    }

    fn client_for(&self, provider: &str) -> Result<Arc<dyn AuthProviderClient>, AuthProductError> {
        self.providers
            .get(provider)
            .cloned()
            .ok_or(AuthProductError::BackendUnavailable)
    }
}

impl fmt::Debug for MultiplexAuthProviderClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MultiplexAuthProviderClient")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[async_trait]
impl AuthProviderClient for MultiplexAuthProviderClient {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        self.client_for(request.provider.as_str())?
            .exchange_callback(context, request)
            .await
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.client_for(request.provider.as_str())?
            .refresh_token(request)
            .await
    }

    async fn cleanup_exchange(
        &self,
        context: OAuthProviderExchangeContext,
        exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        self.client_for(exchange.provider.as_str())?
            .cleanup_exchange(context, exchange)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OAuthClientConfig;
    use crate::google_oauth::google_provider_spec;
    use crate::notion_oauth::{NOTION_PROVIDER_ID, notion_provider_spec};
    use crate::oauth_gate::OAuthGateChallengeRequest;
    use ironclaw_auth::{
        AuthFlowManager, AuthFlowRecordSource, AuthGateRef, AuthProductScope, AuthProviderId,
        AuthSurface, AuthorizationCodeHash, CredentialAccountLabel, CredentialAccountLookupRequest,
        CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
        CredentialRefreshRequest, GOOGLE_CALENDAR_READONLY_SCOPE, InMemoryAuthProductServices,
        NewCredentialAccount, OAuthAuthorizationCode, OAuthClientId, OAuthRedirectUri,
        PkceVerifierHash, PkceVerifierSecret, ProviderBackedCredentialAccountService,
        ProviderScope,
    };
    use ironclaw_capabilities::{CapabilityObligationError, CapabilityObligationRequest};
    use ironclaw_host_api::{
        AgentId, ExtensionId, InvocationId, ResourceScope, RuntimeCredentialAccountProviderId,
        RuntimeCredentialAuthRequirement, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse, SecretHandle, TenantId, ThreadId, UserId,
    };
    use ironclaw_product_adapters::AuthPromptChallengeKind;
    use ironclaw_secrets::{InMemorySecretStore, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    use secrecy::SecretString;
    use std::sync::Mutex;

    #[test]
    fn compose_provider_clients_omits_mux_for_zero_clients() {
        assert!(compose_provider_clients(Vec::new()).is_none());
    }

    #[tokio::test]
    async fn compose_provider_clients_uses_mux_even_for_one_client() {
        let client = compose_provider_clients(vec![("google", Arc::new(PanicProviderClient))])
            .expect("one provider still returns mux");

        let error = client
            .exchange_callback(exchange_context(), callback_request("notion"))
            .await
            .expect_err("unknown provider must be rejected by mux before reaching client");

        assert_eq!(
            error.code(),
            ironclaw_auth::AuthErrorCode::BackendUnavailable
        );
    }

    #[tokio::test]
    async fn compose_provider_client_routes_notion_to_configured_provider_spec() {
        let egress = Arc::new(RecordingEgress::ok(
            br#"{"access_token":"notion-access","refresh_token":"notion-refresh","expires_in":3600}"#
                .to_vec(),
        ));
        let client = compose_provider_client_with_runtime(
            vec![
                OAuthProviderBackendConfig {
                    spec: google_provider_spec(),
                    client: oauth_client("google-client", "https://app.example/oauth/google"),
                },
                OAuthProviderBackendConfig {
                    spec: notion_provider_spec(),
                    client: oauth_client("notion-client", "https://app.example/oauth/notion"),
                },
            ],
            Vec::new(),
            Arc::new(InMemorySecretStore::new()),
            OAuthProviderRuntimePorts::new(egress.clone(), Arc::new(NoopObligationHandler)),
        )
        .expect("provider client composition")
        .client
        .expect("mux client");

        client
            .exchange_callback(exchange_context(), callback_request(NOTION_PROVIDER_ID))
            .await
            .expect("notion exchange should route to notion spec");

        let request = egress.single_request();
        assert_eq!(request.url, "https://mcp.notion.com/token");
        let body = form_params(&request.body);
        assert_eq!(
            body.get("client_id").map(String::as_str),
            Some("notion-client")
        );
        assert_eq!(
            body.get("resource").map(String::as_str),
            Some("https://mcp.notion.com/mcp")
        );
        assert_eq!(
            request
                .network_policy
                .allowed_targets
                .first()
                .map(|target| target.host_pattern.as_str()),
            Some("mcp.notion.com")
        );
    }

    #[tokio::test]
    async fn compose_provider_client_registers_google_oauth_gate_provider() {
        let composition = compose_provider_client_with_runtime(
            vec![OAuthProviderBackendConfig {
                spec: google_provider_spec(),
                client: oauth_client("google-client", "https://app.example/oauth/google"),
            }],
            Vec::new(),
            Arc::new(InMemorySecretStore::new()),
            OAuthProviderRuntimePorts::new(
                Arc::new(RecordingEgress::ok(Vec::new())),
                Arc::new(NoopObligationHandler),
            ),
        )
        .expect("provider composition");
        let registry = composition.gate_registry.expect("google gate registry");
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let flow_source: Arc<dyn AuthFlowRecordSource> = shared;
        let scope = TurnScope::new(
            TenantId::new("tenant-a").unwrap(),
            Some(AgentId::new("agent-a").unwrap()),
            None,
            ThreadId::new("thread-a").unwrap(),
        );
        let owner_user_id = UserId::new("user-a").unwrap();
        let run_id = TurnRunId::new();
        let gate_ref = AuthGateRef::new("gate:google-auth").unwrap();
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
            },
            requester_extension: ExtensionId::new("google-calendar").unwrap(),
            provider_scopes: vec![GOOGLE_CALENDAR_READONLY_SCOPE.to_string()],
        }];

        let view = registry
            .challenge_for_blocked_gate(OAuthGateChallengeRequest {
                flow_manager: &flow_manager,
                flow_source: &flow_source,
                requirements: &requirements,
                scope: &scope,
                owner_user_id: &owner_user_id,
                run_id,
                gate_ref: &gate_ref,
            })
            .await
            .expect("gate challenge")
            .expect("google oauth challenge");

        assert_eq!(view.kind, AuthPromptChallengeKind::OAuthUrl);
        assert_eq!(view.provider.as_str(), "google");
        assert!(
            view.authorization_url
                .as_ref()
                .is_some_and(|url| url.as_str().starts_with("https://accounts.google.com/"))
        );
    }

    #[tokio::test]
    async fn composed_google_provider_refreshes_account_through_credential_service() {
        let egress = Arc::new(RecordingEgress::ok(
            br#"{"access_token":"new-google-access","refresh_token":"new-google-refresh","expires_in":3600}"#
                .to_vec(),
        ));
        let secret_store = Arc::new(InMemorySecretStore::new());
        let resource_scope = sample_scope();
        let auth_scope = AuthProductScope::new(resource_scope.clone(), AuthSurface::Callback);
        let old_access = SecretHandle::new("google-old-access").unwrap();
        let old_refresh = SecretHandle::new("google-old-refresh").unwrap();
        secret_store
            .put(
                resource_scope,
                old_refresh.clone(),
                SecretString::from("stored-google-refresh".to_string()),
                None,
            )
            .await
            .expect("seed refresh token");
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let account = shared
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
                label: CredentialAccountLabel::new("work account").unwrap(),
                status: CredentialAccountStatus::Expired,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(old_access.clone()),
                refresh_secret: Some(old_refresh.clone()),
                scopes: vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()],
            })
            .await
            .expect("expired google account");
        let provider = compose_provider_client_with_runtime(
            vec![OAuthProviderBackendConfig {
                spec: google_provider_spec(),
                client: oauth_client("google-client", "https://app.example/oauth/google"),
            }],
            Vec::new(),
            secret_store,
            OAuthProviderRuntimePorts::new(egress.clone(), Arc::new(NoopObligationHandler)),
        )
        .expect("provider composition")
        .client
        .expect("google provider client");
        let auth =
            ProviderBackedCredentialAccountService::new(shared.clone(), shared.clone(), provider);

        let report = auth
            .refresh_account(CredentialRefreshRequest::new(
                auth_scope.clone(),
                AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
                account.id,
            ))
            .await
            .expect("google refresh succeeds");

        assert!(report.refreshed);
        assert_eq!(report.account.status, CredentialAccountStatus::Configured);
        let stored = shared
            .get_account(CredentialAccountLookupRequest::new(auth_scope, account.id))
            .await
            .expect("lookup")
            .expect("refreshed account");
        assert_eq!(stored.status, CredentialAccountStatus::Configured);
        let new_access = stored
            .access_secret
            .as_ref()
            .expect("refresh must persist a new access token handle");
        let new_refresh = stored
            .refresh_secret
            .as_ref()
            .expect("refresh must persist a new refresh token handle");
        assert_ne!(new_access, &old_access);
        assert_ne!(new_refresh, &old_refresh);
        assert_eq!(
            stored.scopes,
            vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()]
        );
        let request = egress.single_request();
        assert_eq!(request.url, "https://oauth2.googleapis.com/token");
        let body = form_params(&request.body);
        assert_eq!(
            body.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            body.get("client_id").map(String::as_str),
            Some("google-client")
        );
        assert_eq!(
            body.get("refresh_token").map(String::as_str),
            Some("stored-google-refresh")
        );

        let serialized = serde_json::to_string(&report).expect("serialize report");
        assert!(!serialized.contains("stored-google-refresh"));
        assert!(!serialized.contains("new-google-access"));
        assert!(!serialized.contains("new-google-refresh"));
    }

    #[tokio::test]
    async fn composed_google_provider_marks_revoked_on_invalid_grant_token_response() {
        // A3: invalid_grant responses must set status Revoked (permanent revocation),
        // not RefreshFailed (transient). The original test used the same body but
        // expected RefreshFailed — updated to reflect the new classification.
        let egress = Arc::new(RecordingEgress::with_status(
            400,
            br#"{"error":"invalid_grant"}"#.to_vec(),
        ));
        let secret_store = Arc::new(InMemorySecretStore::new());
        let resource_scope = sample_scope();
        let auth_scope = AuthProductScope::new(resource_scope.clone(), AuthSurface::Callback);
        let old_access = SecretHandle::new("google-old-access").unwrap();
        let old_refresh = SecretHandle::new("google-old-refresh").unwrap();
        secret_store
            .put(
                resource_scope,
                old_refresh.clone(),
                SecretString::from("stored-google-refresh".to_string()),
                None,
            )
            .await
            .expect("seed refresh token");
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let account = shared
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
                label: CredentialAccountLabel::new("work account").unwrap(),
                status: CredentialAccountStatus::Expired,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(old_access.clone()),
                refresh_secret: Some(old_refresh.clone()),
                scopes: vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()],
            })
            .await
            .expect("expired google account");
        let provider = compose_provider_client_with_runtime(
            vec![OAuthProviderBackendConfig {
                spec: google_provider_spec(),
                client: oauth_client("google-client", "https://app.example/oauth/google"),
            }],
            Vec::new(),
            secret_store,
            OAuthProviderRuntimePorts::new(egress.clone(), Arc::new(NoopObligationHandler)),
        )
        .expect("provider composition")
        .client
        .expect("google provider client");
        let auth =
            ProviderBackedCredentialAccountService::new(shared.clone(), shared.clone(), provider);

        let report = auth
            .refresh_account(CredentialRefreshRequest::new(
                auth_scope.clone(),
                AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
                account.id,
            ))
            .await
            .expect("google refresh failure is handled by the account service");

        assert!(!report.refreshed);
        assert_eq!(
            report.account.status,
            CredentialAccountStatus::Revoked,
            "invalid_grant must produce Revoked status (permanent revocation, not transient failure)"
        );
        let stored = shared
            .get_account(CredentialAccountLookupRequest::new(auth_scope, account.id))
            .await
            .expect("lookup")
            .expect("failed refresh account");
        assert_eq!(
            stored.status,
            CredentialAccountStatus::Revoked,
            "stored account must reflect Revoked status after invalid_grant"
        );
        assert_eq!(stored.access_secret, Some(old_access));
        assert_eq!(stored.refresh_secret, Some(old_refresh));
        assert_eq!(
            stored.scopes,
            vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()]
        );
        let request = egress.single_request();
        assert_eq!(request.url, "https://oauth2.googleapis.com/token");
        let body = form_params(&request.body);
        assert_eq!(
            body.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            body.get("client_id").map(String::as_str),
            Some("google-client")
        );
        assert_eq!(
            body.get("refresh_token").map(String::as_str),
            Some("stored-google-refresh")
        );
    }

    fn oauth_client(client_id: &str, redirect_uri: &str) -> OAuthClientConfig {
        OAuthClientConfig {
            client_id: OAuthClientId::new(client_id).unwrap(),
            client_secret: None,
            redirect_uri: OAuthRedirectUri::new(redirect_uri).unwrap(),
            hosted_domain_hint: None,
        }
    }

    fn exchange_context() -> OAuthProviderExchangeContext {
        OAuthProviderExchangeContext {
            scope: AuthProductScope::new(sample_scope(), AuthSurface::Callback),
            flow_id: ironclaw_auth::AuthFlowId::new(),
        }
    }

    fn callback_request(provider: &str) -> OAuthProviderCallbackRequest {
        OAuthProviderCallbackRequest {
            provider: AuthProviderId::new(provider).unwrap(),
            account_label: CredentialAccountLabel::new("work account").unwrap(),
            authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                "raw-auth-code".to_string(),
            ))
            .unwrap(),
            authorization_code_hash: AuthorizationCodeHash::new(fake_digest("code")).unwrap(),
            pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                "raw-pkce-verifier".to_string(),
            ))
            .unwrap(),
            pkce_verifier_hash: PkceVerifierHash::new(fake_digest("pkce")).unwrap(),
            scopes: vec![ProviderScope::new("workspace").unwrap()],
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn form_params(body: &[u8]) -> std::collections::BTreeMap<String, String> {
        url::form_urlencoded::parse(body).into_owned().collect()
    }

    fn fake_digest(value: &str) -> String {
        format!(
            "{:064x}",
            value.bytes().fold(0_u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u64::from(byte))
            })
        )
    }

    #[derive(Debug)]
    struct PanicProviderClient;

    #[async_trait]
    impl AuthProviderClient for PanicProviderClient {
        async fn exchange_callback(
            &self,
            _context: OAuthProviderExchangeContext,
            _request: OAuthProviderCallbackRequest,
        ) -> Result<OAuthProviderExchange, AuthProductError> {
            panic!("mux should reject unknown provider before invoking single configured client");
        }

        async fn refresh_token(
            &self,
            _request: OAuthProviderRefreshRequest,
        ) -> Result<OAuthProviderRefresh, AuthProductError> {
            panic!("not used");
        }
    }

    #[derive(Debug)]
    struct RecordingEgress {
        status: u16,
        response_body: Vec<u8>,
        requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
    }

    impl RecordingEgress {
        fn ok(response_body: Vec<u8>) -> Self {
            Self::with_status(200, response_body)
        }

        fn with_status(status: u16, response_body: Vec<u8>) -> Self {
            Self {
                status,
                response_body,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn single_request(&self) -> RuntimeHttpEgressRequest {
            let requests = self.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            requests[0].clone()
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for RecordingEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.requests.lock().unwrap().push(request);
            Ok(RuntimeHttpEgressResponse {
                status: self.status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: self.response_body.clone(),
                saved_body: None,
                request_bytes: 0,
                response_bytes: 0,
                redaction_applied: true,
            })
        }
    }

    #[derive(Debug)]
    struct NoopObligationHandler;

    #[async_trait]
    impl CapabilityObligationHandler for NoopObligationHandler {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), CapabilityObligationError> {
            Ok(())
        }
    }
}
