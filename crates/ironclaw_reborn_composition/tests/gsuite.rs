use std::sync::{Arc, Mutex};

use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountService,
    CredentialAccountStatus, CredentialOwnership, GOOGLE_GMAIL_SEND_SCOPE,
    InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_extensions::{ExtensionRuntime, ManifestSource};
use ironclaw_first_party_extensions::gsuite::{
    CALENDAR_LIST_CALENDARS_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID, google_provider_id,
};
use ironclaw_host_api::{
    CapabilityId, InvocationId, ResourceScope, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle,
    TrustClass, UserId,
};
use ironclaw_host_runtime::FirstPartyCapabilityRequest;
use ironclaw_reborn_composition::{
    bundled_gsuite_extension_packages, bundled_gsuite_first_party_handlers,
};
use serde_json::json;

#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
}

impl RecordingEgress {
    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().expect("egress lock").clone()
    }
}

impl RuntimeHttpEgress for RecordingEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().expect("egress lock").push(request);
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"id":"sent-message"}"#.to_vec(),
            request_bytes: 123,
            response_bytes: 21,
            redaction_applied: true,
        })
    }
}

fn scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap()
}

fn auth_scope(scope: &ResourceScope) -> AuthProductScope {
    AuthProductScope::new(scope.clone(), AuthSurface::Api)
}

fn cap_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

async fn auth_with_google_account(scope: &ResourceScope) -> Arc<InMemoryAuthProductServices> {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    auth.create_account(NewCredentialAccount {
        scope: auth_scope(scope),
        provider: google_provider_id().unwrap(),
        label: CredentialAccountLabel::new("work google").unwrap(),
        status: CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
        refresh_secret: None,
        scopes: vec![ProviderScope::new(GOOGLE_GMAIL_SEND_SCOPE).unwrap()],
    })
    .await
    .unwrap();
    auth
}

#[test]
fn bundled_gsuite_packages_are_host_bundled_but_not_registered_by_default() {
    let packages = bundled_gsuite_extension_packages().unwrap();

    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].id.as_str(), "google-calendar");
    assert_eq!(packages[1].id.as_str(), "gmail");
    for package in &packages {
        assert_eq!(package.manifest.source, ManifestSource::HostBundled);
        assert!(matches!(
            package.manifest.runtime,
            ExtensionRuntime::FirstParty { .. }
        ));
        assert_eq!(
            package.manifest.descriptor_trust_default,
            TrustClass::Sandbox
        );
    }
    let capability_count = packages
        .iter()
        .map(|package| package.capabilities.len())
        .sum::<usize>();
    assert_eq!(capability_count, 15);
}

#[tokio::test]
async fn bundled_gsuite_handlers_register_and_forward_runtime_egress() {
    let scope = scope();
    let auth = auth_with_google_account(&scope).await;
    let registry = bundled_gsuite_first_party_handlers(auth).unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());
    let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
    let handler = registry.get(&capability_id).expect("handler registered");

    let output = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id.clone(),
            scope.clone(),
            json!({ "message": { "raw": "base64url-rfc822" } }),
            Some(egress_port),
        ))
        .await
        .unwrap()
        .output;

    assert_eq!(output["status"], 200);
    assert!(registry.contains_handler(&cap_id(CALENDAR_LIST_CALENDARS_CAPABILITY_ID)));
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_id, capability_id);
    assert_eq!(requests[0].scope, scope);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
}

#[tokio::test]
async fn bundled_gsuite_handler_fails_closed_without_runtime_egress() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let registry = bundled_gsuite_first_party_handlers(auth).unwrap();
    let capability_id = cap_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let handler = registry.get(&capability_id).expect("handler registered");

    let error = handler
        .dispatch(FirstPartyCapabilityRequest::request_for_test(
            capability_id,
            scope,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            None,
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
}
