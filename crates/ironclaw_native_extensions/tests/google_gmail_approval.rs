//! Approval-gate, scope, credential, refresh, redaction, egress-routing, and
//! shared-credential lifecycle tests for the Gmail package.
//!
//! Approval gating for write capabilities is descriptor-level: the manifest
//! marks the four write capabilities `PermissionMode::Ask` with an
//! `ExternalWrite` effect, and the host authorization layer is what blocks an
//! unapproved write. These tests therefore assert the descriptor contract and
//! exercise a host-level gate seam, rather than re-implementing approval inside
//! the handler.

mod support;

use std::sync::Arc;

use ironclaw_host_api::{
    EffectKind, PermissionMode, RuntimeCredentialSource, RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::FirstPartyCapabilityHandler;
use ironclaw_native_extensions::google::credential::{
    GOOGLE_CREDENTIAL_NAME, GoogleCredentialResolver,
};
use ironclaw_native_extensions::google::gmail::handlers::{
    GetMessageHandler, ListMessagesHandler, SendMessageHandler,
};
use ironclaw_native_extensions::google::gmail::manifest::{
    GMAIL_CAPABILITIES, GmailCapabilityKind, capability_id, gmail_package,
};
use ironclaw_native_extensions::google::scopes;
use ironclaw_secrets::InMemorySecretStore;
use serde_json::{Value, json};

use support::{
    FakeEgress, build_gmail_deps, gmail_extension_id, gmail_request, gmail_request_without_egress,
    seed_expiring_token, seed_token, test_provider, test_scope,
};

// ---------------------------------------------------------------------------
// Package / descriptor assertions.
// ---------------------------------------------------------------------------

#[test]
fn gmail_package_declares_six_capabilities() {
    let package = gmail_package().expect("gmail package builds");
    assert_eq!(package.id.as_str(), "gmail");
    assert_eq!(package.capabilities.len(), 6);
    assert_eq!(package.manifest.capabilities.len(), 6);
    assert_eq!(package.root.as_str(), "/system/extensions/gmail");
}

#[test]
fn write_capabilities_require_approval_and_external_write() {
    let package = gmail_package().expect("gmail package builds");
    let write_names = [
        "send_message",
        "create_draft",
        "reply_to_message",
        "trash_message",
    ];
    for (short_name, _, kind) in GMAIL_CAPABILITIES {
        let id = capability_id(short_name);
        let descriptor = package
            .capabilities
            .iter()
            .find(|cap| cap.id.as_str() == id)
            .unwrap_or_else(|| panic!("descriptor for {id} present"));
        match kind {
            GmailCapabilityKind::Write => {
                assert!(
                    write_names.contains(short_name),
                    "{short_name} is a write cap"
                );
                // RequiresApproval -> PermissionMode::Ask.
                assert_eq!(
                    descriptor.default_permission,
                    PermissionMode::Ask,
                    "{short_name} must require approval"
                );
                assert!(
                    descriptor.effects.contains(&EffectKind::ExternalWrite),
                    "{short_name} must declare ExternalWrite"
                );
            }
            GmailCapabilityKind::Read => {
                assert_eq!(
                    descriptor.default_permission,
                    PermissionMode::Allow,
                    "{short_name} is read-only"
                );
                assert!(
                    !descriptor.effects.contains(&EffectKind::ExternalWrite),
                    "{short_name} must not declare ExternalWrite"
                );
            }
        }
        // Every capability uses the shared Google secret and network.
        assert!(descriptor.effects.contains(&EffectKind::UseSecret));
        assert!(descriptor.effects.contains(&EffectKind::Network));
    }
}

// ---------------------------------------------------------------------------
// Egress routing: handlers must go through the host runtime-egress boundary.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handler_fails_closed_when_runtime_http_egress_is_unavailable() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);

    // No runtime_http_egress wired -> the handler cannot reach the network and
    // must fail closed instead of falling back to a self-built transport.
    let error = handler
        .dispatch(gmail_request_without_egress(
            "list_messages",
            scope,
            json!({}),
        ))
        .await
        .expect_err("missing host egress must fail");
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
}

#[tokio::test]
async fn handler_declares_staged_credential_injection_for_the_token() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let egress = FakeEgress::single(200, json!({ "messages": [] }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);

    handler
        .dispatch(gmail_request(
            "list_messages",
            scope,
            json!({}),
            egress.clone(),
        ))
        .await
        .expect("list_messages succeeds");

    let recorded = egress.recorded();
    let injection = &recorded[0].credential_injections[0];
    assert_eq!(injection.handle.as_str(), GOOGLE_CREDENTIAL_NAME);
    assert!(injection.required);
    // Production HostHttpEgressService rejects direct secret-store leases for
    // runtime egress; the handler must declare a staged obligation.
    assert!(
        matches!(
            injection.source,
            RuntimeCredentialSource::StagedObligation { .. }
        ),
        "credential injection must use a staged obligation"
    );
}

// ---------------------------------------------------------------------------
// Approval gate: blocked -> approved -> succeeds; approval-unreachable fails
// closed. The descriptor `PermissionMode::Ask` is the gate the host evaluates
// before dispatch; this models that host gate seam.
// ---------------------------------------------------------------------------

/// Models the host-level authorization decision for a write capability.
enum ApprovalDecision {
    Granted,
    Denied,
    /// The approval service could not be reached — must fail closed.
    Unreachable,
}

/// Mimics the host gate: a write capability may only be dispatched once
/// approval is `Granted`. `Denied`/`Unreachable` must block the dispatch.
async fn dispatch_send_with_gate(
    decision: ApprovalDecision,
    handler: &SendMessageHandler,
    request: ironclaw_host_runtime::FirstPartyCapabilityRequest,
) -> Result<(), RuntimeDispatchErrorKind> {
    match decision {
        ApprovalDecision::Granted => handler
            .dispatch(request)
            .await
            .map(|_| ())
            .map_err(|e| e.kind()),
        // Fail closed: an undecided/unreachable approval never reaches the
        // handler, so the side-effecting send is not silently executed.
        ApprovalDecision::Denied | ApprovalDecision::Unreachable => {
            Err(RuntimeDispatchErrorKind::Client)
        }
    }
}

#[tokio::test]
async fn send_message_blocked_then_approved_then_succeeds() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_SEND]).await;
    let egress = FakeEgress::single(
        200,
        json!({ "id": "msg-sent-700", "threadId": "thr-sent-700", "labelIds": ["SENT"] }),
    );
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_SEND]);
    let handler = SendMessageHandler::new(deps);

    let make_request = || {
        gmail_request(
            "send_message",
            scope.clone(),
            json!({
                "to": ["bob@example.com"],
                "subject": "Sprint planning",
                "body": "Notes attached."
            }),
            egress.clone(),
        )
    };

    // Blocked: approval denied -> handler never runs, no network call.
    let blocked = dispatch_send_with_gate(ApprovalDecision::Denied, &handler, make_request()).await;
    assert!(blocked.is_err());
    assert!(
        egress.recorded().is_empty(),
        "denied send must not call the API"
    );

    // Approved: handler runs, the send reaches the API.
    let approved =
        dispatch_send_with_gate(ApprovalDecision::Granted, &handler, make_request()).await;
    assert!(approved.is_ok(), "approved send succeeds");
    let recorded = egress.recorded();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].url.contains("/users/me/messages/send"));
}

#[tokio::test]
async fn send_message_fails_closed_when_approval_unreachable() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_SEND]).await;
    let egress = FakeEgress::single(200, json!({ "id": "msg-sent-700" }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_SEND]);
    let handler = SendMessageHandler::new(deps);

    let result = dispatch_send_with_gate(
        ApprovalDecision::Unreachable,
        &handler,
        gmail_request(
            "send_message",
            scope,
            json!({ "to": ["x@example.com"], "subject": "x", "body": "x" }),
            egress.clone(),
        ),
    )
    .await;

    // Fail closed: unreachable approval blocks the send entirely.
    assert!(result.is_err());
    assert!(
        egress.recorded().is_empty(),
        "unreachable approval must not let the send through"
    );
}

// ---------------------------------------------------------------------------
// Scope mismatch and missing credential.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scope_mismatch_fails_with_client_error() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    // The token only granted gmail.readonly; send_message requires gmail.send.
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let egress = FakeEgress::single(200, json!({ "id": "msg" }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_SEND]);
    let handler = SendMessageHandler::new(deps);

    let error = handler
        .dispatch(gmail_request(
            "send_message",
            scope,
            json!({ "to": ["bob@example.com"], "subject": "Hi", "body": "Hello" }),
            egress.clone(),
        ))
        .await
        .expect_err("scope mismatch must fail");
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    // The scope check happens before any network call.
    assert!(egress.recorded().is_empty());
}

#[tokio::test]
async fn missing_credential_fails_closed() {
    let scope = test_scope();
    // No token seeded for this scope.
    let secrets = Arc::new(InMemorySecretStore::new());
    let egress = FakeEgress::single(200, json!({ "messages": [] }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);

    let error = handler
        .dispatch(gmail_request(
            "list_messages",
            scope,
            json!({}),
            egress.clone(),
        ))
        .await
        .expect_err("missing credential must fail");
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
    assert!(egress.recorded().is_empty());
}

#[tokio::test]
async fn gmail_error_response_maps_to_client_error() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    // Credential resolves, but Gmail rejects the call with 403.
    let egress = FakeEgress::single(403, json!({ "error": { "status": "PERMISSION_DENIED" } }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);

    let error = handler
        .dispatch(gmail_request("list_messages", scope, json!({}), egress))
        .await
        .expect_err("403 must fail");
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::Client);
}

// ---------------------------------------------------------------------------
// Refresh: a near-expired access token still resolves and the handler
// dispatches. The credential resolver flags `refresh_required`; the actual
// token refresh is owned by the host obligation layer, not the handler.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn near_expired_token_reports_refresh_required_but_still_dispatches() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    // Token expires inside the resolver's 60s refresh buffer.
    seed_expiring_token(&secrets, &scope, &[scopes::GMAIL_READONLY], 0).await;

    // The resolver reports the credential as refresh-required.
    let resolver = GoogleCredentialResolver::new(secrets.clone());
    let provider = test_provider();
    let credential = resolver
        .resolve(
            &scope,
            provider.as_ref(),
            &[scopes::GMAIL_READONLY.to_string()],
        )
        .await
        .expect("credential resolves even when refresh is due");
    assert!(
        credential.refresh_required,
        "a near-expired token must be flagged for refresh"
    );
    assert!(credential.missing_scopes.is_empty());

    // The read handler still dispatches: refreshing the token is the host
    // egress service's responsibility, transparent to the handler.
    let egress = FakeEgress::single(200, json!({ "messages": [], "resultSizeEstimate": 0 }));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);
    let result = handler
        .dispatch(gmail_request(
            "list_messages",
            scope,
            json!({}),
            egress.clone(),
        ))
        .await
        .expect("handler dispatches despite a refresh-due token");
    assert!(result.output.get("messages").is_some());
    assert_eq!(egress.recorded().len(), 1);
}

// ---------------------------------------------------------------------------
// Redaction: handler output must not leak the access token, internal ids, or
// the raw header set.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handler_output_redacts_token_internal_ids_and_raw_headers() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    // Raw Gmail message carries internal fields and an un-whitelisted header.
    let raw = json!({
        "id": "msg-1",
        "threadId": "thr-1",
        "internalDate": "1716200000000",
        "historyId": "55512345",
        "snippet": "Confidential briefing",
        "labelIds": ["INBOX"],
        "payload": {
            "headers": [
                { "name": "From", "value": "spy@example.com" },
                { "name": "Subject", "value": "Confidential briefing" },
                { "name": "X-Internal-Trace-Id", "value": "trace-leak-token" }
            ],
            "body": { "size": 12, "data": "SGVsbG8gd29ybGQ" }
        }
    });
    let egress = FakeEgress::single(200, raw);
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = GetMessageHandler::new(deps);

    let result = handler
        .dispatch(gmail_request(
            "get_message",
            scope,
            json!({ "message_id": "msg-1" }),
            egress.clone(),
        ))
        .await
        .expect("get_message succeeds");

    let serialized = serde_json::to_string(&result.output).expect("output serializes");
    // The OAuth access token must never appear in handler output.
    assert!(
        !serialized.contains("ada-access-token"),
        "output leaked the access token: {serialized}"
    );
    assert!(!serialized.to_lowercase().contains("bearer"));
    // Internal Gmail ids must be stripped from the projection.
    assert!(!serialized.contains("internalDate"));
    assert!(!serialized.contains("historyId"));
    // The raw header array must not be echoed: only whitelisted headers survive.
    assert!(!serialized.contains("X-Internal-Trace-Id"));
    assert!(!serialized.contains("trace-leak-token"));
    // Whitelisted fields are still present.
    assert!(serialized.contains("Confidential briefing"));
    assert!(serialized.contains("spy@example.com"));

    // The handler must not place the token on the outbound request itself; the
    // host egress injects the staged credential.
    let recorded = egress.recorded();
    let request_serialized = format!("{:?}", recorded[0].headers);
    assert!(!request_serialized.contains("ada-access-token"));
}

// ---------------------------------------------------------------------------
// Shared-credential lifecycle: install Gmail + (simulated) Calendar both add
// refs; uninstalling Gmail keeps the credential row alive while Calendar still
// holds a ref.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shared_credential_survives_gmail_uninstall_while_calendar_holds_ref() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let resolver = GoogleCredentialResolver::new(secrets.clone());

    let gmail = gmail_extension_id();
    let calendar = ironclaw_host_api::ExtensionId::new("google-calendar").expect("calendar id");

    // Install Gmail, then (simulated) Calendar — both register a ref.
    resolver
        .add_ref(&scope, &gmail)
        .await
        .expect("gmail ref added");
    let refs = resolver
        .add_ref(&scope, &calendar)
        .await
        .expect("calendar ref added");
    assert_eq!(refs.len(), 2, "both extensions hold a credential ref");

    // Uninstall Gmail — Calendar still holds a ref, so the row survives.
    let remaining = resolver
        .remove_ref(&scope, &gmail)
        .await
        .expect("gmail ref removed");
    assert_eq!(remaining, vec![calendar.clone()]);

    // The shared credential is still resolvable for Calendar.
    let provider = test_provider();
    let credential = resolver
        .resolve(
            &scope,
            provider.as_ref(),
            &[scopes::GMAIL_READONLY.to_string()],
        )
        .await
        .expect("credential row still alive for Calendar");
    assert!(!credential.granted_scopes.is_empty());

    // Uninstall Calendar too — refs empty, credential row is deleted.
    let empty = resolver
        .remove_ref(&scope, &calendar)
        .await
        .expect("calendar ref removed");
    assert!(empty.is_empty());
    let after = resolver
        .resolve(
            &scope,
            provider.as_ref(),
            &[scopes::GMAIL_READONLY.to_string()],
        )
        .await;
    assert!(
        after.is_err(),
        "credential row must be gone once all refs are released"
    );
}

// ---------------------------------------------------------------------------
// reply_to_message: preserves the thread and references the original message.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reply_to_message_preserves_thread_and_threading_headers() {
    use ironclaw_native_extensions::google::gmail::handlers::ReplyToMessageHandler;

    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(
        &secrets,
        &scope,
        &[scopes::GMAIL_SEND, scopes::GMAIL_MODIFY],
    )
    .await;
    // First call fetches the original metadata; second call is the send.
    let egress = FakeEgress::new(vec![
        support::scripted(
            "format=metadata",
            200,
            json!({
                "id": "msg-001",
                "threadId": "thr-001",
                "payload": {
                    "headers": [
                        { "name": "From", "value": "ada@example.com" },
                        { "name": "Subject", "value": "Q2 summary" },
                        { "name": "Message-ID", "value": "<orig-001@mail.example.com>" }
                    ]
                }
            }),
        ),
        support::scripted(
            "messages/send",
            200,
            json!({ "id": "msg-reply-9", "threadId": "thr-001", "labelIds": ["SENT"] }),
        ),
    ]);
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_SEND, scopes::GMAIL_MODIFY]);
    let handler = ReplyToMessageHandler::new(deps);

    let result = handler
        .dispatch(gmail_request(
            "reply_to_message",
            scope,
            json!({ "message_id": "msg-001", "body": "Thanks, looks good." }),
            egress.clone(),
        ))
        .await
        .expect("reply_to_message succeeds");

    assert_eq!(
        result.output.get("thread_id").and_then(Value::as_str),
        Some("thr-001")
    );

    let recorded = egress.recorded();
    assert_eq!(recorded.len(), 2);
    // The send carries the original threadId so the reply stays in-thread.
    let send = &recorded[1];
    assert!(send.url.contains("/users/me/messages/send"));
    let sent_body: Value = serde_json::from_slice(&send.body).expect("send body is json");
    assert_eq!(
        sent_body.get("threadId").and_then(Value::as_str),
        Some("thr-001")
    );
    assert!(sent_body.get("raw").and_then(Value::as_str).is_some());
}
