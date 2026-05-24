use std::sync::{Arc, Mutex};

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_first_party_extensions::gsuite::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
    CALENDAR_GET_EVENT_CAPABILITY_ID, CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
    CALENDAR_LIST_EVENTS_CAPABILITY_ID, CALENDAR_SET_REMINDER_CAPABILITY_ID,
    CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID, GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor,
    google_provider_id, gsuite_package_specs,
};
use ironclaw_host_api::{
    CapabilityId, InvocationId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle, UserId,
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

fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

async fn auth_with_google_account(
    scope: &ResourceScope,
    scopes: Vec<ProviderScope>,
) -> Arc<InMemoryAuthProductServices> {
    auth_with_google_account_status(scope, scopes, CredentialAccountStatus::Configured, true).await
}

async fn auth_with_google_account_status(
    scope: &ResourceScope,
    scopes: Vec<ProviderScope>,
    status: CredentialAccountStatus,
    include_access_secret: bool,
) -> Arc<InMemoryAuthProductServices> {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    auth.create_account(NewCredentialAccount {
        scope: auth_scope(scope),
        provider: google_provider_id().unwrap(),
        label: CredentialAccountLabel::new("work google").unwrap(),
        status,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: include_access_secret
            .then(|| SecretHandle::new("google-access-token").unwrap()),
        refresh_secret: None,
        scopes,
    })
    .await
    .unwrap();
    auth
}

async fn dispatch_error(
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

#[test]
fn gsuite_packages_declare_calendar_and_gmail_capabilities() {
    let packages = gsuite_package_specs();
    let ids = packages
        .iter()
        .map(|package| package.extension_id.to_string())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["google-calendar", "gmail"]);
    let capability_count = packages
        .iter()
        .map(|package| package.capabilities.len())
        .sum::<usize>();
    assert_eq!(capability_count, 15);
}

#[tokio::test]
async fn gsuite_handler_uses_selected_credential_handle_for_runtime_egress() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());

    let result = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    assert_eq!(result.output["status"], 200);
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_id, capability_id);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
    assert!(
        !requests[0]
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
    );
    assert_eq!(requests[0].credential_injections.len(), 1);
    assert_eq!(
        requests[0].credential_injections[0].handle,
        SecretHandle::new("google-access-token").unwrap()
    );
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_scope_is_missing() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(GMAIL_TRASH_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());

    let error = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message_id": "msg-1" }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert!(egress.requests().is_empty());
}

#[test]
fn gsuite_package_specs_include_core_capabilities() {
    let capability_ids = gsuite_package_specs()
        .iter()
        .flat_map(|package| {
            package
                .capabilities
                .iter()
                .map(|capability| format!("{}.{}", package.extension_id, capability.short_name))
        })
        .collect::<Vec<_>>();

    for id in [
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        CALENDAR_GET_EVENT_CAPABILITY_ID,
        CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
        CALENDAR_DELETE_EVENT_CAPABILITY_ID,
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        CALENDAR_SET_REMINDER_CAPABILITY_ID,
        GMAIL_LIST_MESSAGES_CAPABILITY_ID,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        GMAIL_CREATE_DRAFT_CAPABILITY_ID,
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
    ] {
        assert!(
            capability_ids.contains(&id.to_string()),
            "missing capability spec for {id}"
        );
    }
    assert!(AuthProviderId::new("google").is_ok());
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_account_is_not_configured() {
    let scope = scope();
    let auth = auth_with_google_account_status(
        &scope,
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::PendingSetup,
        true,
    )
    .await;
    let egress = Arc::new(RecordingEgress::default());

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_access_secret_is_missing() {
    let scope = scope();
    let auth = auth_with_google_account_status(
        &scope,
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        false,
    )
    .await;
    let egress = Arc::new(RecordingEgress::default());

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::Client
    );
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handler_rejects_missing_required_input() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)]).await;

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({}),
        Arc::new(RecordingEgress::default()),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode
    );
}

#[tokio::test]
async fn calendar_id_default_does_not_swallow_invalid_type() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE)])
            .await;

    let error = dispatch_error(
        auth,
        scope,
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        json!({ "calendar_id": false }),
        Arc::new(RecordingEgress::default()),
    )
    .await;

    assert_eq!(
        error.kind(),
        ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode
    );
}
