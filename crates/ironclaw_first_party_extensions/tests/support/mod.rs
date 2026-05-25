#![allow(dead_code)]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_first_party_extensions::{
    GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor, google_provider_id,
};
use ironclaw_host_api::{
    CapabilityId, InvocationId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle, UserId,
};
use serde_json::json;

#[derive(Default)]
pub(crate) struct RecordingEgress {
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
    responses: Mutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
}

impl RecordingEgress {
    pub(crate) fn with_responses(responses: Vec<RuntimeHttpEgressResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
        }
    }

    pub(crate) fn with_errors(errors: Vec<RuntimeHttpEgressError>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(errors.into_iter().map(Err).collect()),
        }
    }

    pub(crate) fn json(body: serde_json::Value) -> RuntimeHttpEgressResponse {
        Self::json_with_request_bytes(body, 123)
    }

    pub(crate) fn json_status(status: u16, body: serde_json::Value) -> RuntimeHttpEgressResponse {
        let body = serde_json::to_vec(&body).expect("response body serializes");
        RuntimeHttpEgressResponse {
            status,
            headers: Vec::new(),
            request_bytes: 123,
            response_bytes: body.len() as u64,
            body,
            redaction_applied: true,
        }
    }

    pub(crate) fn empty(status: u16) -> RuntimeHttpEgressResponse {
        RuntimeHttpEgressResponse {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            request_bytes: 123,
            response_bytes: 0,
            redaction_applied: true,
        }
    }

    pub(crate) fn json_with_request_bytes(
        body: serde_json::Value,
        request_bytes: u64,
    ) -> RuntimeHttpEgressResponse {
        let body = serde_json::to_vec(&body).expect("response body serializes");
        RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            request_bytes,
            response_bytes: body.len() as u64,
            body,
            redaction_applied: true,
        }
    }

    pub(crate) fn malformed_json() -> RuntimeHttpEgressResponse {
        RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            request_bytes: 123,
            response_bytes: 1,
            body: b"{".to_vec(),
            redaction_applied: true,
        }
    }

    pub(crate) fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().expect("egress lock").clone()
    }
}

impl RuntimeHttpEgress for RecordingEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().expect("egress lock").push(request);
        self.responses
            .lock()
            .expect("response lock")
            .pop_front()
            .unwrap_or_else(|| Ok(RecordingEgress::json(json!({"id":"sent-message"}))))
    }
}

pub(crate) fn scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap()
}

fn auth_scope(scope: &ResourceScope) -> AuthProductScope {
    AuthProductScope::new(scope.clone(), AuthSurface::Api)
}

pub(crate) fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

pub(crate) fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

pub(crate) fn fixture(area: &str, name: &str) -> serde_json::Value {
    let path = format!(
        "{}/tests/fixtures/google_api/{area}/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse fixture {path}: {e}"))
}

pub(crate) async fn auth_with_google_account(
    scope: &ResourceScope,
    scopes: Vec<ProviderScope>,
) -> Arc<InMemoryAuthProductServices> {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    add_google_account(
        &auth,
        scope,
        "work google",
        scopes,
        CredentialAccountStatus::Configured,
        true,
    )
    .await;
    auth
}

pub(crate) async fn auth_with_google_account_status(
    scope: &ResourceScope,
    scopes: Vec<ProviderScope>,
    status: CredentialAccountStatus,
    include_access_secret: bool,
) -> Arc<InMemoryAuthProductServices> {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    add_google_account(
        &auth,
        scope,
        "work google",
        scopes,
        status,
        include_access_secret,
    )
    .await;
    auth
}

pub(crate) async fn add_google_account(
    auth: &InMemoryAuthProductServices,
    scope: &ResourceScope,
    label: &str,
    scopes: Vec<ProviderScope>,
    status: CredentialAccountStatus,
    include_access_secret: bool,
) {
    ironclaw_auth::CredentialAccountService::create_account(
        auth,
        NewCredentialAccount {
            scope: auth_scope(scope),
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new(label).unwrap(),
            status,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: include_access_secret
                .then(|| SecretHandle::new("google-access-token").unwrap()),
            refresh_secret: None,
            scopes,
        },
    )
    .await
    .unwrap();
}

pub(crate) async fn dispatch_ok(
    auth: Arc<InMemoryAuthProductServices>,
    scope: ResourceScope,
    capability: &str,
    input: serde_json::Value,
    egress: Arc<RecordingEgress>,
) -> serde_json::Value {
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(capability);
    executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &input,
            runtime_http_egress: egress,
        })
        .await
        .unwrap()
        .output
}

pub(crate) async fn dispatch_error(
    auth: Arc<InMemoryAuthProductServices>,
    scope: ResourceScope,
    capability: &str,
    input: serde_json::Value,
    egress: Arc<RecordingEgress>,
) -> GsuiteDispatchError {
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(capability);
    executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &input,
            runtime_http_egress: egress,
        })
        .await
        .unwrap_err()
}
