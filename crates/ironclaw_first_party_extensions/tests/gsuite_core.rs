use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE,
    GOOGLE_GMAIL_SEND_SCOPE, InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
};
use ironclaw_first_party_extensions::gsuite::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
    CALENDAR_GET_EVENT_CAPABILITY_ID, CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
    CALENDAR_LIST_EVENTS_CAPABILITY_ID, CALENDAR_SET_REMINDER_CAPABILITY_ID,
    CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID, GSUITE_OUTPUT_BYTES_LIMIT, GSUITE_RESPONSE_BODY_LIMIT,
    GsuiteDispatchError, GsuiteDispatchRequest, GsuiteExecutor, google_provider_id,
    gsuite_package_specs, gsuite_resource_profile,
};
use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, NetworkScheme,
    RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED, ResourceScope, RuntimeDispatchErrorKind,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    SecretHandle, UserId,
};
use serde_json::json;

#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
    responses: Mutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
}

impl RecordingEgress {
    fn with_responses(responses: Vec<RuntimeHttpEgressResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
        }
    }

    fn with_errors(errors: Vec<RuntimeHttpEgressError>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(errors.into_iter().map(Err).collect()),
        }
    }

    fn json(body: serde_json::Value) -> RuntimeHttpEgressResponse {
        Self::json_with_request_bytes(body, 123)
    }

    fn json_with_request_bytes(
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

    fn malformed_json() -> RuntimeHttpEgressResponse {
        RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            request_bytes: 123,
            response_bytes: 1,
            body: b"{".to_vec(),
            redaction_applied: true,
        }
    }

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
        self.responses
            .lock()
            .expect("response lock")
            .pop_front()
            .unwrap_or_else(|| Ok(RecordingEgress::json(json!({"id":"sent-message"}))))
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

async fn auth_with_google_account_status(
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

async fn add_google_account(
    auth: &InMemoryAuthProductServices,
    scope: &ResourceScope,
    label: &str,
    scopes: Vec<ProviderScope>,
    status: CredentialAccountStatus,
    include_access_secret: bool,
) {
    auth.create_account(NewCredentialAccount {
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
    })
    .await
    .unwrap();
}

async fn dispatch_ok(
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
async fn gsuite_handler_applies_google_api_network_policy() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(GMAIL_SEND_MESSAGE_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::default());

    executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({ "message": { "raw": "base64url-rfc822" } }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    let policy = &requests[0].network_policy;
    assert!(policy.deny_private_ip_ranges);
    assert_eq!(policy.max_egress_bytes, Some(10 * 1024 * 1024));
    let allowed = policy
        .allowed_targets
        .iter()
        .map(|target| (target.scheme, target.host_pattern.as_str()))
        .collect::<Vec<_>>();
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "www.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "gmail.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "calendar.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "oauth2.googleapis.com")));
    assert!(allowed.contains(&(Some(NetworkScheme::Https), "accounts.google.com")));
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

#[tokio::test]
async fn gsuite_handler_allows_reply_to_message_with_send_scope_only() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::default());

    dispatch_ok(
        auth,
        scope,
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822", "threadId": "thread-1" } }),
        egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
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

#[tokio::test]
async fn gsuite_handler_rejects_malformed_egress_response() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::malformed_json(),
    ]));

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress,
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
}

#[tokio::test]
async fn gsuite_handler_maps_runtime_egress_errors() {
    let cases = [
        (
            RuntimeHttpEgressError::Credential {
                reason: "missing".to_string(),
            },
            RuntimeDispatchErrorKind::Client,
        ),
        (
            RuntimeHttpEgressError::Request {
                reason: "denied".to_string(),
                request_bytes: 11,
                response_bytes: 0,
            },
            RuntimeDispatchErrorKind::InputEncode,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "offline".to_string(),
                request_bytes: 12,
                response_bytes: 0,
            },
            RuntimeDispatchErrorKind::NetworkDenied,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "bad response".to_string(),
                request_bytes: 13,
                response_bytes: 1,
            },
            RuntimeDispatchErrorKind::OutputDecode,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED.to_string(),
                request_bytes: 14,
                response_bytes: 1024,
            },
            RuntimeDispatchErrorKind::OutputTooLarge,
        ),
    ];

    for (error, expected_kind) in cases {
        let scope = scope();
        let auth =
            auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)]).await;
        let request_bytes = error.request_bytes();
        let egress = Arc::new(RecordingEgress::with_errors(vec![error]));

        let error = dispatch_error(
            auth,
            scope,
            GMAIL_SEND_MESSAGE_CAPABILITY_ID,
            json!({ "message": { "raw": "base64url-rfc822" } }),
            egress,
        )
        .await;

        assert_eq!(error.kind(), expected_kind);
        if request_bytes > 0 {
            assert_eq!(
                error
                    .usage()
                    .and_then(|usage| Some(usage.network_egress_bytes)),
                Some(request_bytes)
            );
        }
    }
}

#[tokio::test]
async fn gsuite_handler_fails_before_egress_when_google_account_is_missing_or_ambiguous() {
    let scope = scope();
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let egress = Arc::new(RecordingEgress::default());

    let error = dispatch_error(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.requests().is_empty());

    add_google_account(
        &auth,
        &scope,
        "work google",
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        true,
    )
    .await;
    add_google_account(
        &auth,
        &scope,
        "personal google",
        vec![provider_scope(GOOGLE_GMAIL_SEND_SCOPE)],
        CredentialAccountStatus::Configured,
        true,
    )
    .await;
    let error = dispatch_error(
        auth,
        scope,
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.requests().is_empty());
}

#[tokio::test]
async fn gsuite_handlers_build_expected_requests_for_each_capability() {
    let scope = scope();
    let all_scopes = vec![
        provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE),
        provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE),
        provider_scope(GOOGLE_GMAIL_READONLY_SCOPE),
        provider_scope(GOOGLE_GMAIL_SEND_SCOPE),
        provider_scope(GOOGLE_GMAIL_MODIFY_SCOPE),
    ];
    let auth = auth_with_google_account(&scope, all_scopes).await;
    let egress = Arc::new(RecordingEgress::default());

    let cases = [
        (
            CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
            json!({}),
            NetworkMethod::Get,
            "/users/me/calendarList",
        ),
        (
            CALENDAR_LIST_EVENTS_CAPABILITY_ID,
            json!({"calendar_id":"primary","time_min":"2026-05-21T00:00:00Z","max_results":10}),
            NetworkMethod::Get,
            "/calendars/primary/events",
        ),
        (
            CALENDAR_GET_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1"}),
            NetworkMethod::Get,
            "/events/evt-1",
        ),
        (
            CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
            json!({"timeMin":"2026-05-21T00:00:00Z","timeMax":"2026-05-22T00:00:00Z"}),
            NetworkMethod::Post,
            "/freeBusy",
        ),
        (
            CALENDAR_CREATE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event":{"summary":"Review"}}),
            NetworkMethod::Post,
            "/calendars/primary/events",
        ),
        (
            CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1","event":{"summary":"Updated"}}),
            NetworkMethod::Patch,
            "/events/evt-1",
        ),
        (
            CALENDAR_DELETE_EVENT_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1"}),
            NetworkMethod::Delete,
            "/events/evt-1",
        ),
        (
            CALENDAR_SET_REMINDER_CAPABILITY_ID,
            json!({"calendar_id":"primary","event_id":"evt-1","reminders":{"useDefault":false}}),
            NetworkMethod::Patch,
            "/events/evt-1",
        ),
        (
            GMAIL_LIST_MESSAGES_CAPABILITY_ID,
            json!({"query":"is:unread","label_ids":["INBOX"],"max_results":10}),
            NetworkMethod::Get,
            "/users/me/messages",
        ),
        (
            GMAIL_GET_MESSAGE_CAPABILITY_ID,
            json!({"message_id":"msg-1"}),
            NetworkMethod::Get,
            "/users/me/messages/msg-1?format=full",
        ),
        (
            GMAIL_SEND_MESSAGE_CAPABILITY_ID,
            json!({"message":{"raw":"base64url-rfc822"}}),
            NetworkMethod::Post,
            "/users/me/messages/send",
        ),
        (
            GMAIL_CREATE_DRAFT_CAPABILITY_ID,
            json!({"draft":{"message":{"raw":"base64url-rfc822"}}}),
            NetworkMethod::Post,
            "/users/me/drafts",
        ),
        (
            GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
            json!({"message":{"raw":"base64url-rfc822","threadId":"thread-1"}}),
            NetworkMethod::Post,
            "/users/me/messages/send",
        ),
        (
            GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
            json!({"message_id":"msg-1"}),
            NetworkMethod::Post,
            "/users/me/messages/msg-1/trash",
        ),
    ];

    for (capability, input, _, _) in &cases {
        dispatch_ok(
            auth.clone(),
            scope.clone(),
            capability,
            input.clone(),
            egress.clone(),
        )
        .await;
    }
    let add_egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json(json!({"attendees":[{"email":"existing@example.com"}]})),
        RecordingEgress::json(json!({"id":"evt-1"})),
    ]));
    dispatch_ok(
        auth,
        scope.clone(),
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        json!({
            "calendar_id":"primary",
            "event_id":"evt-1",
            "attendees":[{"email":"new@example.com"}]
        }),
        add_egress.clone(),
    )
    .await;

    let requests = egress.requests();
    assert_eq!(requests.len(), cases.len());
    for ((_, _, method, url_fragment), request) in cases.iter().zip(requests.iter()) {
        assert_eq!(&request.method, method);
        assert!(
            request.url.contains(url_fragment),
            "{} did not contain {url_fragment}",
            request.url
        );
        assert_eq!(request.credential_injections.len(), 1);
    }
    assert!(requests[8].url.contains("labelIds=INBOX"));

    let add_requests = add_egress.requests();
    assert_eq!(add_requests.len(), 2);
    let add_get = &add_requests[0];
    let add_patch = &add_requests[1];
    assert_eq!(add_get.method, NetworkMethod::Get);
    assert!(add_get.url.contains("/events/evt-1"));
    assert_eq!(add_patch.method, NetworkMethod::Patch);
    assert!(add_patch.url.contains("/events/evt-1"));
    let patch_body: serde_json::Value = serde_json::from_slice(&add_patch.body).unwrap();
    let attendees = patch_body["attendees"].as_array().unwrap();
    assert!(
        attendees
            .iter()
            .any(|attendee| attendee["email"] == "existing@example.com")
    );
    assert!(
        attendees
            .iter()
            .any(|attendee| attendee["email"] == "new@example.com")
    );
}

#[test]
fn gsuite_resource_profile_allows_wrapped_response_headroom() {
    let profile = gsuite_resource_profile();

    assert!(GSUITE_OUTPUT_BYTES_LIMIT > GSUITE_RESPONSE_BODY_LIMIT);
    assert_eq!(
        profile.default_estimate.output_bytes,
        Some(GSUITE_OUTPUT_BYTES_LIMIT)
    );
    assert_eq!(
        profile
            .hard_ceiling
            .as_ref()
            .and_then(|ceiling| ceiling.max_output_bytes),
        Some(GSUITE_OUTPUT_BYTES_LIMIT)
    );
}

#[tokio::test]
async fn add_attendees_reports_both_google_api_requests() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let executor = GsuiteExecutor::new(auth);
    let capability_id = capability_id(CALENDAR_ADD_ATTENDEES_CAPABILITY_ID);
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_with_request_bytes(
            json!({"attendees":[{"email":"existing@example.com"}]}),
            101,
        ),
        RecordingEgress::json_with_request_bytes(json!({"id":"evt-1"}), 211),
    ]));

    let result = executor
        .dispatch(GsuiteDispatchRequest {
            capability_id: &capability_id,
            scope: &scope,
            input: &json!({
                "calendar_id": "primary",
                "event_id": "evt-1",
                "attendees": [{"email": "new@example.com"}]
            }),
            runtime_http_egress: egress.clone(),
        })
        .await
        .unwrap();

    assert_eq!(egress.requests().len(), 2);
    assert_eq!(result.usage.network_egress_bytes, 312);
}
