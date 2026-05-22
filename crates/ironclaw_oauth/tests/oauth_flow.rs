use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, InvocationFingerprint, InvocationId, ResourceEstimate, ResourceScope,
    SecretHandle, UserId,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_oauth::{
    OAuthError, OAuthProvider, OAuthResumeNotifier, OAuthRuntime, ProviderMode, ProviderRegistry,
    RefreshScheduler, TokenPersister, TokenSet,
};
use ironclaw_run_state::{AuthRequiredPayload, InMemoryRunStateStore, RunStart, RunStateStore};
use ironclaw_secrets::{
    SecretLease, SecretLeaseId, SecretLeaseStatus, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use url::Url;

#[tokio::test]
async fn brokered_exchange_uses_legacy_broker_contract_without_client_secret() {
    let (runtime, egress, secrets) = runtime_with_mode(ProviderMode::Brokered {
        broker_url: Url::parse("https://broker.example.test").unwrap(),
        broker_auth: SecretString::from("gateway-token"),
    });
    egress.push_json(json!({
        "access_token": "access-from-broker",
        "refresh_token": "refresh-from-broker",
        "expires_in": 3600,
        "scope": "calendar.readonly gmail.readonly"
    }));

    let started = runtime
        .start(
            "fake",
            vec!["calendar.readonly".to_string()],
            sample_scope(),
        )
        .await
        .unwrap();
    runtime
        .exchange(
            "fake",
            "oauth-code".to_string(),
            state_from_url(&started.oauth_url),
        )
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://broker.example.test/oauth/exchange"
    );
    assert_header(&requests[0], "authorization", "Bearer gateway-token");
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("\"code\":\"oauth-code\""));
    assert!(body.contains("\"token_url\":\"https://oauth.example.test/token\""));
    assert!(!body.contains("client_secret"));
    assert_eq!(
        secrets
            .get_secret("google_oauth_token")
            .unwrap()
            .expose_secret(),
        "access-from-broker"
    );
    assert_eq!(
        secrets
            .get_secret("google_oauth_token_scopes")
            .unwrap()
            .expose_secret(),
        "[\"calendar.readonly\",\"gmail.readonly\"]"
    );
}

#[tokio::test]
async fn direct_exchange_posts_provider_secret_and_pkce_to_provider_token_url() {
    let (runtime, egress, secrets) = runtime_with_mode(ProviderMode::Direct);
    egress.push_json(json!({
        "access_token": "access-direct",
        "refresh_token": "refresh-direct",
        "expires_in": 600,
        "scope": "calendar.readonly"
    }));

    let started = runtime
        .start(
            "fake",
            vec!["calendar.readonly".to_string()],
            sample_scope(),
        )
        .await
        .unwrap();
    runtime
        .exchange(
            "fake",
            "direct-code".to_string(),
            state_from_url(&started.oauth_url),
        )
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://oauth.example.test/token");
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("code=direct-code"));
    assert!(body.contains("client_secret=direct-secret"));
    assert!(body.contains("code_verifier="));
    assert_eq!(
        secrets
            .get_secret("google_oauth_token")
            .unwrap()
            .expose_secret(),
        "access-direct"
    );
}

#[tokio::test]
async fn exchange_notifies_resume_waiters_after_token_persistence() {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FakeProvider)).unwrap();
    let egress = Arc::new(RecordingEgress::default());
    let secrets = Arc::new(MemorySecretStore::default());
    let (notifier, mut receiver) = OAuthResumeNotifier::channel(4);
    let runtime = OAuthRuntime::builder(registry.into(), egress.clone(), secrets.clone())
        .mode(ProviderMode::Direct)
        .resume(Arc::new(notifier))
        .redirect_base_url(Url::parse("https://app.example.test").unwrap())
        .build()
        .unwrap();
    egress.push_json(json!({
        "access_token": "access-direct",
        "refresh_token": "refresh-direct",
        "expires_in": 600,
        "scope": "calendar.readonly"
    }));

    let started = runtime
        .start(
            "fake",
            vec!["calendar.readonly".to_string()],
            sample_scope(),
        )
        .await
        .unwrap();
    runtime
        .exchange(
            "fake",
            "direct-code".to_string(),
            state_from_url(&started.oauth_url),
        )
        .await
        .unwrap();

    let signal = receiver.recv().await.unwrap();
    assert_eq!(signal.credential_name, "google_oauth_token");
    assert_eq!(signal.outcome.flow_id, started.flow_id);
    assert!(signal.outcome.success);
    assert_eq!(
        secrets
            .get_secret("google_oauth_token")
            .unwrap()
            .expose_secret(),
        "access-direct"
    );
}

#[tokio::test]
async fn exchange_queries_run_state_and_emits_invocation_specific_resume_signal() {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FakeProvider)).unwrap();
    let egress = Arc::new(RecordingEgress::default());
    let secrets = Arc::new(MemorySecretStore::default());
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let (notifier, mut receiver) = OAuthResumeNotifier::channel(4);
    let runtime = OAuthRuntime::builder(registry.into(), egress.clone(), secrets.clone())
        .mode(ProviderMode::Direct)
        .resume(Arc::new(notifier))
        .run_state(run_state.clone())
        .redirect_base_url(Url::parse("https://app.example.test").unwrap())
        .build()
        .unwrap();
    egress.push_json(json!({
        "access_token": "access-direct",
        "refresh_token": "refresh-direct",
        "expires_in": 600,
        "scope": "calendar.readonly"
    }));

    let scope = sample_scope();
    let capability_id = CapabilityId::new("google-calendar.list_events").unwrap();
    let invocation_fingerprint = InvocationFingerprint::for_dispatch(
        &scope,
        &capability_id,
        &ResourceEstimate::default(),
        &json!({"calendar": "primary"}),
    )
    .unwrap();
    let started = runtime
        .start("fake", vec!["calendar.readonly".to_string()], scope.clone())
        .await
        .unwrap();
    run_state
        .start(RunStart {
            invocation_id: scope.invocation_id,
            capability_id,
            scope: scope.clone(),
        })
        .await
        .unwrap();
    run_state
        .block_auth_required(
            &scope,
            scope.invocation_id,
            AuthRequiredPayload {
                provider_id: "fake".to_string(),
                credential_name: "google_oauth_token".to_string(),
                missing_scopes: vec!["calendar.readonly".to_string()],
                oauth_url: started.oauth_url.clone(),
                flow_id: started.flow_id,
                extension_id: ironclaw_host_api::ExtensionId::new("google-calendar").unwrap(),
                invocation_fingerprint,
            },
        )
        .await
        .unwrap();

    runtime
        .exchange(
            "fake",
            "direct-code".to_string(),
            state_from_url(&started.oauth_url),
        )
        .await
        .unwrap();

    let signal = receiver.recv().await.unwrap();
    assert_eq!(signal.invocation_id, Some(scope.invocation_id));
    assert_eq!(signal.credential_name, "google_oauth_token");
    assert_eq!(signal.outcome.flow_id, started.flow_id);
    assert!(signal.outcome.success);
}

#[tokio::test]
async fn oauth_http_failure_error_redacts_untrusted_response_body() {
    let (runtime, egress, _secrets) = runtime_with_mode(ProviderMode::Direct);
    egress.push_response(
        400,
        json!({
            "error": "invalid_grant",
            "access_token": "secret-access",
            "refresh_token": "secret-refresh",
            "client_secret": "secret-client"
        }),
    );

    let started = runtime
        .start(
            "fake",
            vec!["calendar.readonly".to_string()],
            sample_scope(),
        )
        .await
        .unwrap();
    let error = runtime
        .exchange(
            "fake",
            "direct-code".to_string(),
            state_from_url(&started.oauth_url),
        )
        .await
        .unwrap_err();
    let rendered = error.to_string();

    assert!(rendered.contains("invalid_grant"));
    assert!(!rendered.contains("secret-access"));
    assert!(!rendered.contains("secret-refresh"));
    assert!(!rendered.contains("secret-client"));
    assert!(!rendered.contains("direct-code"));
}

#[tokio::test]
async fn resume_notifier_queries_blocked_auth_records_for_matching_credential_and_flow() {
    let run_state = InMemoryRunStateStore::new();
    let scope = sample_scope();
    let flow_id = uuid::Uuid::new_v4();
    let capability_id = CapabilityId::new("google-calendar.list_events").unwrap();
    let invocation_fingerprint = InvocationFingerprint::for_dispatch(
        &scope,
        &capability_id,
        &ResourceEstimate::default(),
        &json!({"calendar": "primary"}),
    )
    .unwrap();
    run_state
        .start(RunStart {
            invocation_id: scope.invocation_id,
            capability_id,
            scope: scope.clone(),
        })
        .await
        .unwrap();
    run_state
        .block_auth_required(
            &scope,
            scope.invocation_id,
            AuthRequiredPayload {
                provider_id: "google".to_string(),
                credential_name: "google_oauth_token".to_string(),
                missing_scopes: vec!["calendar.readonly".to_string()],
                oauth_url: "https://accounts.example.test/oauth".to_string(),
                flow_id,
                extension_id: ironclaw_host_api::ExtensionId::new("google-calendar").unwrap(),
                invocation_fingerprint,
            },
        )
        .await
        .unwrap();
    let (notifier, mut receiver) = OAuthResumeNotifier::channel(4);

    let sent = notifier
        .notify_blocked_auth(&run_state, "google_oauth_token", &scope, flow_id)
        .await
        .unwrap();

    assert_eq!(sent, 1);
    let signal = receiver.recv().await.unwrap();
    assert_eq!(signal.invocation_id, Some(scope.invocation_id));
    assert_eq!(signal.credential_name, "google_oauth_token");
    assert_eq!(signal.outcome.flow_id, flow_id);
}

#[tokio::test]
async fn refresh_scheduler_serializes_refresh_and_preserves_legacy_rows() {
    let (runtime, egress, secrets) = runtime_with_mode(ProviderMode::Brokered {
        broker_url: Url::parse("https://broker.example.test").unwrap(),
        broker_auth: SecretString::from("gateway-token"),
    });
    let scope = sample_scope();
    TokenPersister::new(secrets.clone())
        .persist(
            &scope,
            "google_oauth_token",
            &TokenSet::from_expires_in(
                "old-access",
                Some("refresh-existing".to_string()),
                Some(1),
                vec!["calendar.readonly".to_string()],
            ),
        )
        .await
        .unwrap();
    egress.push_json(json!({
        "access_token": "fresh-access",
        "expires_in": 3600,
        "scope": "calendar.readonly"
    }));

    let refreshed = RefreshScheduler::new(runtime.flow())
        .refresh("fake", &scope)
        .await
        .unwrap();

    assert_eq!(refreshed.access_token.expose_secret(), "fresh-access");
    let requests = egress.requests();
    assert_eq!(requests[0].url, "https://broker.example.test/oauth/refresh");
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("\"refresh_token\":\"refresh-existing\""));
    assert_eq!(
        secrets
            .get_secret("google_oauth_token")
            .unwrap()
            .expose_secret(),
        "fresh-access"
    );
    assert_eq!(
        secrets
            .get_secret("google_oauth_token_refresh_token")
            .unwrap()
            .expose_secret(),
        "refresh-existing"
    );
}

#[test]
fn broker_builder_rejects_loopback_without_test_fixture_override() {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FakeProvider)).unwrap();
    let result = OAuthRuntime::builder(
        Arc::new(registry),
        Arc::new(RecordingEgress::default()),
        Arc::new(MemorySecretStore::default()),
    )
    .mode(ProviderMode::Brokered {
        broker_url: Url::parse("http://127.0.0.1:9999").unwrap(),
        broker_auth: SecretString::from("gateway-token"),
    })
    .build();

    assert!(matches!(result, Err(OAuthError::UrlRejected { .. })));
}

#[cfg(feature = "test-fixtures")]
#[test]
fn broker_builder_allows_loopback_with_compile_gated_test_fixture_override() {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FakeProvider)).unwrap();
    OAuthRuntime::builder(
        Arc::new(registry),
        Arc::new(RecordingEgress::default()),
        Arc::new(MemorySecretStore::default()),
    )
    .mode(ProviderMode::Brokered {
        broker_url: Url::parse("http://127.0.0.1:9999").unwrap(),
        broker_auth: SecretString::from("gateway-token"),
    })
    .allow_loopback_broker_for_tests()
    .build()
    .unwrap();
}

fn runtime_with_mode(
    mode: ProviderMode,
) -> (OAuthRuntime, Arc<RecordingEgress>, Arc<MemorySecretStore>) {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FakeProvider)).unwrap();
    let egress = Arc::new(RecordingEgress::default());
    let secrets = Arc::new(MemorySecretStore::default());
    let runtime = OAuthRuntime::builder(registry.into(), egress.clone(), secrets.clone())
        .mode(mode)
        .redirect_base_url(Url::parse("https://app.example.test").unwrap())
        .build()
        .unwrap();
    (runtime, egress, secrets)
}

fn sample_scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap()
}

fn state_from_url(url: &str) -> String {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap()
}

fn assert_header(request: &NetworkHttpRequest, name: &str, value: &str) {
    assert!(
        request
            .headers
            .iter()
            .any(|(header_name, header_value)| header_name == name && header_value == value)
    );
}

struct FakeProvider;

#[async_trait]
impl OAuthProvider for FakeProvider {
    fn provider_id(&self) -> &str {
        "fake"
    }

    fn auth_url(&self) -> &str {
        "https://accounts.example.test/oauth"
    }

    fn token_url(&self) -> &str {
        "https://oauth.example.test/token"
    }

    fn credential_name(&self) -> &str {
        "google_oauth_token"
    }

    fn public_client_id(&self) -> &str {
        "public-client"
    }

    fn direct_client_secret(&self) -> Option<&SecretString> {
        static SECRET: std::sync::OnceLock<SecretString> = std::sync::OnceLock::new();
        Some(SECRET.get_or_init(|| SecretString::from("direct-secret")))
    }

    fn build_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        scopes: &[String],
        redirect_uri: &str,
    ) -> String {
        let mut url = Url::parse(self.auth_url()).unwrap();
        url.query_pairs_mut()
            .append_pair("client_id", self.public_client_id())
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256");
        url.to_string()
    }

    fn parse_token_response(&self, body: &serde_json::Value) -> Result<TokenSet, OAuthError> {
        let access_token = body
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OAuthError::InvalidTokenResponse {
                reason: "access_token missing".to_string(),
            })?;
        let refresh_token = body
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let expires_in = body.get("expires_in").and_then(serde_json::Value::as_u64);
        let scopes = body
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
        Ok(TokenSet::from_expires_in(
            access_token.to_string(),
            refresh_token,
            expires_in,
            scopes,
        ))
    }

    fn detect_scope_mismatch(&self, stored: &[String], required: &[String]) -> Vec<String> {
        required
            .iter()
            .filter(|scope| !stored.contains(scope))
            .cloned()
            .collect()
    }
}

#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<NetworkHttpRequest>>,
    responses: Mutex<VecDeque<NetworkHttpResponse>>,
}

impl RecordingEgress {
    fn push_json(&self, body: serde_json::Value) {
        self.push_response(200, body);
    }

    fn push_response(&self, status: u16, body: serde_json::Value) {
        self.responses
            .lock()
            .unwrap()
            .push_back(NetworkHttpResponse {
                status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: serde_json::to_vec(&body).unwrap(),
                usage: NetworkUsage::default(),
            });
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl NetworkHttpEgress for RecordingEgress {
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| NetworkHttpError::Transport {
                reason: "missing fake OAuth response".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            })
    }
}

#[derive(Default)]
struct MemorySecretStore {
    values: Mutex<HashMap<String, SecretMaterial>>,
    leases: Mutex<HashMap<SecretLeaseId, String>>,
}

impl MemorySecretStore {
    fn get_secret(&self, name: &str) -> Option<SecretMaterial> {
        self.values.lock().unwrap().get(name).cloned()
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.values
            .lock()
            .unwrap()
            .insert(handle.as_str().to_string(), material);
        Ok(SecretMetadata { scope, handle })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .contains_key(handle.as_str())
            .then(|| SecretMetadata {
                scope: scope.clone(),
                handle: handle.clone(),
            }))
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        if !self.values.lock().unwrap().contains_key(handle.as_str()) {
            return Err(SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            });
        }
        let id = SecretLeaseId::new();
        self.leases
            .lock()
            .unwrap()
            .insert(id, handle.as_str().to_string());
        Ok(SecretLease {
            id,
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        })
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let handle = self.leases.lock().unwrap().remove(&lease_id).ok_or(
            SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            },
        )?;
        self.values
            .lock()
            .unwrap()
            .get(&handle)
            .cloned()
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: SecretHandle::new(handle).unwrap(),
            })
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let handle = self.leases.lock().unwrap().remove(&lease_id).ok_or(
            SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            },
        )?;
        Ok(SecretLease {
            id: lease_id,
            scope: scope.clone(),
            handle: SecretHandle::new(handle).unwrap(),
            status: SecretLeaseStatus::Revoked,
        })
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(self
            .leases
            .lock()
            .unwrap()
            .iter()
            .map(|(id, handle)| SecretLease {
                id: *id,
                scope: scope.clone(),
                handle: SecretHandle::new(handle.clone()).unwrap(),
                status: SecretLeaseStatus::Active,
            })
            .collect())
    }
}
