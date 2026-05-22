//! Shared integration-test scaffolding for the Google Calendar package.
//!
//! Provides a fake [`RuntimeHttpEgress`] that records outbound requests and
//! returns scripted responses, plus helpers to build the handler dependency
//! graph and `FirstPartyCapabilityRequest`s without a live host.
//!
//! The fake egress mirrors the real `HostHttpEgressService` seam: calendar
//! handlers issue HTTP through `request.services.runtime_http_egress`, so the
//! integration tests inject this fake to keep all outbound traffic local.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ResourceScope, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, UserId,
};
use ironclaw_host_runtime::FirstPartyCapabilityRequest;
use ironclaw_native_extensions::EnvConfig;
use ironclaw_native_extensions::google::calendar::handlers::CalendarHandlerDeps;
use ironclaw_native_extensions::google::calendar::manifest::capability_id;
use ironclaw_native_extensions::google::credential::{
    GOOGLE_CREDENTIAL_NAME, GoogleCredentialResolver,
};
use ironclaw_native_extensions::google::gmail::handlers::GmailHandlerDeps;
use ironclaw_native_extensions::google::gmail::manifest::capability_id as gmail_capability_id;
use ironclaw_native_extensions::google::oauth_provider::GoogleProvider;
use ironclaw_oauth::{OAuthProvider, TokenPersister, TokenSet};
use ironclaw_secrets::InMemorySecretStore;
use serde_json::{Value, json};

/// A single scripted HTTP response keyed by a URL substring.
pub struct ScriptedResponse {
    pub url_contains: String,
    pub status: u16,
    pub body: Vec<u8>,
}

/// Fake [`RuntimeHttpEgress`] for integration tests.
///
/// It records every outbound [`RuntimeHttpEgressRequest`] and answers each
/// call with the first scripted response whose `url_contains` substring
/// matches; if none match it falls back to a generic 200 with an empty JSON
/// object.
#[derive(Clone)]
pub struct FakeEgress {
    responses: Arc<Vec<ScriptedResponse>>,
    recorded: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl FakeEgress {
    pub fn new(responses: Vec<ScriptedResponse>) -> Self {
        Self {
            responses: Arc::new(responses),
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Single-response convenience constructor (matches any URL).
    pub fn single(status: u16, body: Value) -> Self {
        Self::new(vec![ScriptedResponse {
            url_contains: String::new(),
            status,
            body: serde_json::to_vec(&body).expect("fixture serializes"),
        }])
    }

    /// Snapshot of every request the handler issued, in order.
    pub fn recorded(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.recorded.lock().expect("lock poisoned").clone()
    }

    fn into_dyn(self) -> Arc<dyn RuntimeHttpEgress> {
        Arc::new(self)
    }
}

impl RuntimeHttpEgress for FakeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let url = request.url.clone();
        let request_bytes = request.body.len() as u64;
        self.recorded.lock().expect("lock poisoned").push(request);
        let scripted = self
            .responses
            .iter()
            .find(|response| url.contains(&response.url_contains));
        let (status, body) = match scripted {
            Some(response) => (response.status, response.body.clone()),
            None => (200, b"{}".to_vec()),
        };
        let response_bytes = body.len() as u64;
        Ok(RuntimeHttpEgressResponse {
            status,
            headers: Vec::new(),
            body,
            request_bytes,
            response_bytes,
            redaction_applied: false,
        })
    }
}

/// Build a scripted response that returns a JSON fixture for matching URLs.
pub fn scripted(url_contains: &str, status: u16, body: Value) -> ScriptedResponse {
    ScriptedResponse {
        url_contains: url_contains.to_string(),
        status,
        body: serde_json::to_vec(&body).expect("fixture serializes"),
    }
}

/// A stable test resource scope.
pub fn test_scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("ada").expect("user id"), InvocationId::new())
        .expect("resource scope")
}

/// A `google` `OAuthProvider` for the broker-mode baked-in client.
pub fn test_provider() -> Arc<dyn OAuthProvider> {
    GoogleProvider::from_config(&EnvConfig {
        oauth_broker_active: true,
        google_client_id: None,
        google_client_secret: None,
        google_allowed_hd: None,
    })
    .expect("provider builds")
    .expect("provider present")
}

/// Persist a Google OAuth token with the given granted scopes for `scope`.
pub async fn seed_token(
    secrets: &Arc<InMemorySecretStore>,
    scope: &ResourceScope,
    granted_scopes: &[&str],
) {
    TokenPersister::new(secrets.clone())
        .persist(
            scope,
            GOOGLE_CREDENTIAL_NAME,
            &TokenSet::from_expires_in(
                "ada-access-token",
                Some("ada-refresh-token".to_string()),
                Some(3600),
                granted_scopes.iter().map(|s| s.to_string()).collect(),
            ),
        )
        .await
        .expect("token persists");
}

/// Persist a Google OAuth token whose access token is already (near) expired,
/// so [`GoogleCredentialResolver::resolve`] reports `refresh_required = true`.
///
/// `expires_in_secs` is the lifetime fed to `TokenSet::from_expires_in`; pass a
/// small/zero value so the expiry falls inside the resolver's refresh buffer.
pub async fn seed_expiring_token(
    secrets: &Arc<InMemorySecretStore>,
    scope: &ResourceScope,
    granted_scopes: &[&str],
    expires_in_secs: u64,
) {
    TokenPersister::new(secrets.clone())
        .persist(
            scope,
            GOOGLE_CREDENTIAL_NAME,
            &TokenSet::from_expires_in(
                "ada-access-token",
                Some("ada-refresh-token".to_string()),
                Some(expires_in_secs),
                granted_scopes.iter().map(|s| s.to_string()).collect(),
            ),
        )
        .await
        .expect("token persists");
}

/// Build [`CalendarHandlerDeps`] over a secret store.
///
/// Handlers no longer own an HTTP transport; the fake egress is injected per
/// request via [`calendar_request`].
pub fn build_deps(
    secrets: Arc<InMemorySecretStore>,
    required_scopes: &[&str],
) -> CalendarHandlerDeps {
    let resolver = Arc::new(GoogleCredentialResolver::new(secrets));
    CalendarHandlerDeps::new(
        resolver,
        test_provider(),
        required_scopes.iter().map(|s| s.to_string()).collect(),
    )
}

/// Build a `FirstPartyCapabilityRequest` for a calendar capability short name,
/// carrying the supplied fake [`RuntimeHttpEgress`].
pub fn calendar_request(
    short_name: &str,
    scope: ResourceScope,
    input: Value,
    egress: FakeEgress,
) -> FirstPartyCapabilityRequest {
    FirstPartyCapabilityRequest::for_test(
        CapabilityId::new(capability_id(short_name)).expect("capability id"),
        scope,
        input,
        Some(egress.into_dyn()),
    )
}

/// Build a `FirstPartyCapabilityRequest` with no `runtime_http_egress` wired —
/// used to assert handlers fail closed when the host egress is unavailable.
pub fn calendar_request_without_egress(
    short_name: &str,
    scope: ResourceScope,
    input: Value,
) -> FirstPartyCapabilityRequest {
    FirstPartyCapabilityRequest::for_test(
        CapabilityId::new(capability_id(short_name)).expect("capability id"),
        scope,
        input,
        None,
    )
}

/// `ExtensionId` for the Google Calendar package.
pub fn calendar_extension_id() -> ExtensionId {
    ExtensionId::new("google-calendar").expect("extension id")
}

/// `ExtensionId` for the Gmail package.
pub fn gmail_extension_id() -> ExtensionId {
    ExtensionId::new("gmail").expect("extension id")
}

/// Build [`GmailHandlerDeps`] over a secret store.
///
/// Handlers no longer own an HTTP transport; the fake egress is injected per
/// request via [`gmail_request`].
pub fn build_gmail_deps(
    secrets: Arc<InMemorySecretStore>,
    required_scopes: &[&str],
) -> GmailHandlerDeps {
    let resolver = Arc::new(GoogleCredentialResolver::new(secrets));
    GmailHandlerDeps::new(
        resolver,
        test_provider(),
        required_scopes.iter().map(|s| s.to_string()).collect(),
    )
}

/// Build a `FirstPartyCapabilityRequest` for a Gmail capability short name,
/// carrying the supplied fake [`RuntimeHttpEgress`].
pub fn gmail_request(
    short_name: &str,
    scope: ResourceScope,
    input: Value,
    egress: FakeEgress,
) -> FirstPartyCapabilityRequest {
    FirstPartyCapabilityRequest::for_test(
        CapabilityId::new(gmail_capability_id(short_name)).expect("capability id"),
        scope,
        input,
        Some(egress.into_dyn()),
    )
}

/// Build a Gmail `FirstPartyCapabilityRequest` with no `runtime_http_egress`
/// wired — used to assert handlers fail closed when the host egress is
/// unavailable.
pub fn gmail_request_without_egress(
    short_name: &str,
    scope: ResourceScope,
    input: Value,
) -> FirstPartyCapabilityRequest {
    FirstPartyCapabilityRequest::for_test(
        CapabilityId::new(gmail_capability_id(short_name)).expect("capability id"),
        scope,
        input,
        None,
    )
}

/// Empty JSON object.
pub fn empty_input() -> Value {
    json!({})
}
