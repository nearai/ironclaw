use super::*;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    OAuthAuthorizationCode, PkceVerifierHash, PkceVerifierSecret,
};
use ironclaw_host_api::{
    InvocationId, RuntimeHttpEgressError, RuntimeHttpEgressResponse, TenantId, UserId,
};
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata,
    SecretStoreError,
};
use std::sync::Mutex;

#[test]
fn authorization_code_body_adds_provider_resource_only_when_configured() {
    let with_resource = authorization_code_body(
        &notion_spec(),
        "client-id",
        "https://app.example/callback",
        None,
        "code",
        "pkce",
    );
    let without_resource = authorization_code_body(
        &google_spec(),
        "client-id",
        "https://app.example/callback",
        None,
        "code",
        "pkce",
    );

    let with_resource = form_params(&with_resource);
    let without_resource = form_params(&without_resource);
    assert_eq!(
        with_resource.get("resource").map(String::as_str),
        Some("https://mcp.notion.com/mcp")
    );
    assert!(!without_resource.contains_key("resource"));
}

#[tokio::test]
async fn token_sink_rolls_back_access_token_when_refresh_write_fails() {
    let store = Arc::new(RecordingSecretStore::failing_refresh_put());
    let client = HostOAuthProviderClient::new(
        notion_spec(),
        Arc::new(NoopEgress),
        store.clone(),
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("client-id").unwrap(),
        OAuthRedirectUri::new("https://app.example/callback").unwrap(),
    )
    .unwrap();
    let result = client
        .store_tokens(
            sample_scope(),
            AuthFlowId::new(),
            OAuthTokenResponse::new(
                SecretString::from("access-token".to_string()),
                Some(SecretString::from("refresh-token".to_string())),
                Some("workspace"),
                None,
            )
            .unwrap(),
        )
        .await;

    assert_eq!(
        result.expect_err("refresh write failure").code(),
        ironclaw_auth::AuthErrorCode::BackendUnavailable
    );
    assert!(
        store
            .deleted_handles()
            .iter()
            .any(|handle| handle.contains("access")),
        "access token written before the failed refresh token must be cleaned up"
    );
}

#[tokio::test]
async fn google_exchange_fails_closed_when_response_omits_scope() {
    let egress = Arc::new(RecordingEgress::ok(
        br#"{"access_token":"access-token","refresh_token":"refresh-token","expires_in":3600}"#
            .to_vec(),
    ));
    let store = Arc::new(RecordingSecretStore::recording());
    let client = HostOAuthProviderClient::new(
        google_spec(),
        egress,
        store.clone(),
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("google-client").unwrap(),
        OAuthRedirectUri::new("https://app.example/callback").unwrap(),
    )
    .unwrap();

    let error = client
        .exchange_callback(
            exchange_context(),
            callback_request("google", "work google", &["gmail.readonly"]),
        )
        .await
        .expect_err("google must not trust requested scopes when provider omits scope");

    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::TokenExchangeFailed
    );
    assert!(store.put_handles().is_empty());
}

#[tokio::test]
async fn exchange_maps_provider_5xx_to_retryable_backend_unavailable() {
    let egress = Arc::new(RecordingEgress::with_status(
        503,
        br#"{"error":"temporarily_unavailable"}"#.to_vec(),
    ));
    let client = HostOAuthProviderClient::new(
        google_spec(),
        egress,
        Arc::new(RecordingSecretStore::recording()),
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("google-client").unwrap(),
        OAuthRedirectUri::new("https://app.example/callback").unwrap(),
    )
    .unwrap();

    let error = client
        .exchange_callback(
            exchange_context(),
            callback_request("google", "work google", &["gmail.readonly"]),
        )
        .await
        .expect_err("provider 5xx should be retryable");

    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::BackendUnavailable
    );
}

#[tokio::test]
async fn exchange_request_includes_client_secret_and_derived_network_policy_host() {
    let egress = Arc::new(RecordingEgress::ok(
            br#"{"access_token":"access-token","refresh_token":"refresh-token","scope":"gmail.readonly","expires_in":3600}"#.to_vec(),
        ));
    let client = HostOAuthProviderClient::new(
        google_spec(),
        egress.clone(),
        Arc::new(RecordingSecretStore::recording()),
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("google-client").unwrap(),
        OAuthRedirectUri::new("https://app.example/callback").unwrap(),
    )
    .unwrap()
    .with_client_secret(SecretString::from("google-secret".to_string()));

    client
        .exchange_callback(
            exchange_context(),
            callback_request("google", "work google", &["gmail.readonly"]),
        )
        .await
        .expect("exchange succeeds");

    let request = egress.single_request();
    assert_eq!(request.url, "https://oauth2.googleapis.com/token");
    assert_eq!(
        request
            .network_policy
            .allowed_targets
            .first()
            .map(|target| target.host_pattern.as_str()),
        Some("oauth2.googleapis.com")
    );
    let body = form_params(&request.body);
    assert_eq!(
        body.get("client_secret").map(String::as_str),
        Some("google-secret")
    );
}

#[tokio::test]
async fn exchange_uses_dynamic_client_material_and_binds_refresh_secret() {
    let egress = Arc::new(RecordingEgress::ok(
        br#"{"access_token":"access-token","refresh_token":"refresh-token","expires_in":3600}"#
            .to_vec(),
    ));
    let material_source = Arc::new(RecordingMaterialSource::new());
    let client = HostOAuthProviderClient::new_with_client_material(
        notion_spec(),
        egress.clone(),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(NoopObligationHandler),
        material_source.clone(),
    )
    .unwrap();
    let context = exchange_context();
    let flow_id = context.flow_id;

    client
        .exchange_callback(context, callback_request("notion", "notion", &[]))
        .await
        .expect("exchange succeeds");

    let request = egress.single_request();
    assert_eq!(request.url, "https://issuer.example/token");
    let body = form_params(&request.body);
    assert_eq!(
        body.get("client_id").map(String::as_str),
        Some("dcr-client")
    );
    assert_eq!(
        material_source.bound_flow_ids(),
        vec![flow_id],
        "refresh-capable exchanges must bind the DCR client material to the refresh secret"
    );
}

#[tokio::test]
async fn refresh_request_uses_stored_refresh_token_and_preserves_scope_fallback() {
    let egress = Arc::new(RecordingEgress::ok(
            br#"{"access_token":"new-access-token","refresh_token":"new-refresh-token","expires_in":3600}"#
                .to_vec(),
        ));
    let store = Arc::new(InMemorySecretStore::new());
    let scope = sample_scope();
    let refresh_secret = SecretHandle::new("google-refresh-input").unwrap();
    store
        .put(
            scope.clone(),
            refresh_secret.clone(),
            SecretString::from("stored-refresh-token".to_string()),
        )
        .await
        .expect("store refresh token");
    let client = HostOAuthProviderClient::new(
        google_spec(),
        egress.clone(),
        store,
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("google-client").unwrap(),
        OAuthRedirectUri::new("https://app.example/callback").unwrap(),
    )
    .unwrap();

    let refresh = client
        .refresh_token(OAuthProviderRefreshRequest {
            scope: AuthProductScope::new(scope, AuthSurface::Callback),
            provider: AuthProviderId::new("google").unwrap(),
            account_id: CredentialAccountId::new(),
            refresh_secret,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        })
        .await
        .expect("refresh succeeds");

    assert_eq!(
        refresh.scopes,
        vec![ProviderScope::new("gmail.readonly").unwrap()]
    );
    let request = egress.single_request();
    let body = form_params(&request.body);
    assert_eq!(
        body.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        body.get("refresh_token").map(String::as_str),
        Some("stored-refresh-token")
    );
}

#[test]
fn token_endpoint_host_is_derived_and_rejects_non_https_endpoints() {
    assert_eq!(
        oauth_endpoint_host("https://oauth2.googleapis.com/token").unwrap(),
        "oauth2.googleapis.com"
    );
    assert_eq!(
        oauth_endpoint_host("http://oauth2.googleapis.com/token")
            .expect_err("http endpoints are rejected")
            .code(),
        ironclaw_auth::AuthErrorCode::BackendUnavailable
    );
}

fn form_params(body: &[u8]) -> std::collections::BTreeMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

fn exchange_context() -> OAuthProviderExchangeContext {
    OAuthProviderExchangeContext {
        scope: AuthProductScope::new(sample_scope(), AuthSurface::Callback),
        flow_id: AuthFlowId::new(),
    }
}

fn callback_request(provider: &str, label: &str, scopes: &[&str]) -> OAuthProviderCallbackRequest {
    OAuthProviderCallbackRequest {
        provider: AuthProviderId::new(provider).unwrap(),
        account_label: CredentialAccountLabel::new(label).unwrap(),
        authorization_code: OAuthAuthorizationCode::new(SecretString::from(
            "raw-auth-code".to_string(),
        ))
        .unwrap(),
        authorization_code_hash: AuthorizationCodeHash::new(fake_digest("code")).unwrap(),
        pkce_verifier: PkceVerifierSecret::new(SecretString::from("raw-pkce-verifier".to_string()))
            .unwrap(),
        pkce_verifier_hash: PkceVerifierHash::new(fake_digest("pkce")).unwrap(),
        scopes: scopes
            .iter()
            .map(|scope| ProviderScope::new(*scope).unwrap())
            .collect(),
    }
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

fn google_spec() -> HostOAuthProviderSpec {
    HostOAuthProviderSpec {
        provider_id: "google",
        capability_id: "ironclaw_auth.google_oauth",
        token_endpoint: "https://oauth2.googleapis.com/token",
        secret_handle_prefix: "google",
        resource: None,
        exchange_scope_policy: ExchangeScopePolicy::RequireProviderScope,
    }
}

fn notion_spec() -> HostOAuthProviderSpec {
    HostOAuthProviderSpec {
        provider_id: "notion",
        capability_id: "ironclaw_auth.notion_oauth",
        token_endpoint: "https://mcp.notion.com/token",
        secret_handle_prefix: "notion",
        resource: Some("https://mcp.notion.com/mcp"),
        exchange_scope_policy: ExchangeScopePolicy::FallbackToRequested,
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

struct RecordingSecretStore {
    puts: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
    fail_refresh_put: bool,
}

impl RecordingSecretStore {
    fn recording() -> Self {
        Self {
            puts: Mutex::new(Vec::new()),
            deleted: Mutex::new(Vec::new()),
            fail_refresh_put: false,
        }
    }

    fn failing_refresh_put() -> Self {
        Self {
            puts: Mutex::new(Vec::new()),
            deleted: Mutex::new(Vec::new()),
            fail_refresh_put: true,
        }
    }

    fn deleted_handles(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }

    fn put_handles(&self) -> Vec<String> {
        self.puts.lock().unwrap().clone()
    }
}

#[async_trait]
impl SecretStore for RecordingSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        _material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let handle_string = handle.as_str().to_string();
        self.puts.lock().unwrap().push(handle_string.clone());
        if self.fail_refresh_put && handle_string.contains("refresh") {
            return Err(SecretStoreError::StoreUnavailable {
                reason: "refresh write failed".to_string(),
            });
        }
        Ok(SecretMetadata { scope, handle })
    }

    async fn metadata(
        &self,
        _scope: &ResourceScope,
        _handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(None)
    }

    async fn delete(
        &self,
        _scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.deleted
            .lock()
            .unwrap()
            .push(handle.as_str().to_string());
        Ok(true)
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        Err(SecretStoreError::UnknownSecret {
            scope: Box::new(scope.clone()),
            handle: handle.clone(),
        })
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        Err(SecretStoreError::UnknownLease {
            scope: Box::new(scope.clone()),
            lease_id,
        })
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        Err(SecretStoreError::UnknownLease {
            scope: Box::new(scope.clone()),
            lease_id,
        })
    }

    async fn leases_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct NoopEgress;

#[async_trait]
impl RuntimeHttpEgress for NoopEgress {
    async fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<
        ironclaw_host_api::RuntimeHttpEgressResponse,
        ironclaw_host_api::RuntimeHttpEgressError,
    > {
        Err(ironclaw_host_api::RuntimeHttpEgressError::Network {
            reason: "not used".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })
    }
}

#[derive(Debug)]
struct RecordingEgress {
    status: u16,
    response_body: Vec<u8>,
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
}

#[derive(Debug)]
struct RecordingMaterialSource {
    bound_flow_ids: Mutex<Vec<AuthFlowId>>,
}

impl RecordingMaterialSource {
    fn new() -> Self {
        Self {
            bound_flow_ids: Mutex::new(Vec::new()),
        }
    }

    fn bound_flow_ids(&self) -> Vec<AuthFlowId> {
        self.bound_flow_ids.lock().unwrap().clone()
    }

    fn material(&self) -> OAuthClientMaterial {
        OAuthClientMaterial {
            client_id: OAuthClientId::new("dcr-client").unwrap(),
            client_secret: None,
            redirect_uri: OAuthRedirectUri::new("https://app.example/callback").unwrap(),
            token_endpoint: "https://issuer.example/token".to_string(),
        }
    }
}

#[async_trait]
impl OAuthClientMaterialSource for RecordingMaterialSource {
    async fn exchange_material(
        &self,
        _scope: &ResourceScope,
        _flow_id: AuthFlowId,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        Ok(self.material())
    }

    async fn refresh_material(
        &self,
        _scope: &ResourceScope,
        _refresh_secret: &SecretHandle,
    ) -> Result<OAuthClientMaterial, AuthProductError> {
        Ok(self.material())
    }

    async fn bind_refresh_material(
        &self,
        _scope: &ResourceScope,
        flow_id: AuthFlowId,
        _refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        self.bound_flow_ids.lock().unwrap().push(flow_id);
        Ok(())
    }

    async fn cleanup_exchange_material(
        &self,
        _scope: &ResourceScope,
        _flow_id: AuthFlowId,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
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
    ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
        Ok(())
    }
}
