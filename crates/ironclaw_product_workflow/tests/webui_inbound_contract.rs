//! Contract tests for route-independent WebUI inbound DTOs.

use base64::Engine;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    WebUiAuthenticatedCaller, WebUiCancelReason, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiGateResolution, WebUiInboundAttachment, WebUiInboundCommand, WebUiInboundValidationCode,
    WebUiResolveGateRequest, WebUiSendMessageRequest,
};
use ironclaw_turns::SanitizedCancelReason;
use serde_json::json;
use uuid::Uuid;

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("valid tenant"),
        UserId::new("user-alpha").expect("valid user"),
        Some(AgentId::new("agent-alpha").expect("valid agent")),
        Some(ProjectId::new("project-alpha").expect("valid project")),
    )
}

fn run_id() -> String {
    "3d54a1f0-0a7f-4b9c-a350-4258f2fa3e18".to_string()
}

#[test]
fn create_thread_maps_authenticated_caller_to_canonical_command() {
    let request: WebUiCreateThreadRequest = serde_json::from_value(json!({
        "client_action_id": "create-1",
        "requested_thread_id": "thread-alpha"
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");

    let WebUiInboundCommand::CreateThread {
        caller,
        client_action_id,
        requested_thread_id,
    } = command
    else {
        panic!("expected create-thread command");
    };
    assert_eq!(caller.user_id.as_str(), "user-alpha");
    assert_eq!(client_action_id.as_str(), "create-1");
    assert_eq!(
        requested_thread_id,
        Some(ThreadId::new("thread-alpha").expect("valid thread"))
    );
}

#[test]
fn send_message_maps_body_to_turn_scope_actor_and_content() {
    let request: WebUiSendMessageRequest = serde_json::from_value(json!({
        "client_action_id": "send-1",
        "thread_id": "thread-alpha",
        "content": "hello\nworld"
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");

    let WebUiInboundCommand::SendMessage {
        scope,
        actor,
        client_action_id,
        content,
    } = command
    else {
        panic!("expected send-message command");
    };
    assert_eq!(scope.tenant_id.as_str(), "tenant-alpha");
    assert_eq!(scope.agent_id.expect("agent").as_str(), "agent-alpha");
    assert_eq!(scope.project_id.expect("project").as_str(), "project-alpha");
    assert_eq!(scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(actor.user_id.as_str(), "user-alpha");
    assert_eq!(client_action_id.as_str(), "send-1");
    assert_eq!(content, "hello\nworld");
}

#[test]
fn cancel_run_maps_to_canonical_cancel_request() {
    let request: WebUiCancelRunRequest = serde_json::from_value(json!({
        "client_action_id": "cancel-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "reason": "operator_requested"
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");

    let WebUiInboundCommand::CancelRun { request } = command else {
        panic!("expected cancel-run command");
    };
    assert_eq!(request.scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(request.actor.user_id.as_str(), "user-alpha");
    assert_eq!(request.idempotency_key.as_str(), "cancel-1");
    assert_eq!(request.reason, SanitizedCancelReason::OperatorRequested);
}

#[test]
fn resolve_gate_maps_to_canonical_gate_command_without_raw_secret() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "credential_provided",
        "credential_ref": "credential-alpha"
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");

    let WebUiInboundCommand::ResolveGate {
        scope,
        actor,
        run_id: parsed_run_id,
        gate_ref,
        client_action_id,
        resolution,
    } = command
    else {
        panic!("expected resolve-gate command");
    };
    assert_eq!(scope.thread_id.as_str(), "thread-alpha");
    assert_eq!(actor.user_id.as_str(), "user-alpha");
    assert_eq!(
        parsed_run_id.as_uuid(),
        Uuid::parse_str(&run_id()).expect("uuid")
    );
    assert_eq!(gate_ref.as_str(), "gate-alpha");
    assert_eq!(client_action_id.as_str(), "gate-1");
    assert_eq!(
        resolution,
        WebUiGateResolution::CredentialProvided {
            credential_ref: "credential-alpha".to_string()
        }
    );
}

#[test]
fn missing_content_returns_stable_validation_error() {
    let request: WebUiSendMessageRequest = serde_json::from_value(json!({
        "client_action_id": "send-1",
        "thread_id": "thread-alpha"
    }))
    .expect("request json");

    let err = request.into_command(caller()).expect_err("missing content");

    assert_eq!(err.field, "content");
    assert_eq!(err.code, WebUiInboundValidationCode::MissingField);
    assert_eq!(
        serde_json::to_value(&err).expect("error json"),
        json!({"field":"content","code":"missing_field"})
    );
}

#[test]
fn blank_client_action_id_returns_stable_validation_error() {
    let request: WebUiSendMessageRequest = serde_json::from_value(json!({
        "client_action_id": "   ",
        "thread_id": "thread-alpha",
        "content": "hello"
    }))
    .expect("request json");

    let err = request.into_command(caller()).expect_err("blank action id");

    assert_eq!(err.field, "client_action_id");
    assert_eq!(err.code, WebUiInboundValidationCode::Blank);
}

#[test]
fn invalid_thread_id_returns_stable_validation_error() {
    let request: WebUiSendMessageRequest = serde_json::from_value(json!({
        "client_action_id": "send-1",
        "thread_id": "../other-thread",
        "content": "hello"
    }))
    .expect("request json");

    let err = request
        .into_command(caller())
        .expect_err("invalid thread id");

    assert_eq!(err.field, "thread_id");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidId);
}

#[test]
fn missing_run_id_returns_stable_validation_error_for_cancel() {
    let request: WebUiCancelRunRequest = serde_json::from_value(json!({
        "client_action_id": "cancel-1",
        "thread_id": "thread-alpha"
    }))
    .expect("request json");

    let err = request.into_command(caller()).expect_err("missing run id");

    assert_eq!(err.field, "run_id");
    assert_eq!(err.code, WebUiInboundValidationCode::MissingField);
}

#[test]
fn invalid_run_id_returns_stable_validation_error_for_gate_resolution() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": "not-a-uuid",
        "gate_ref": "gate-alpha",
        "resolution": "approved"
    }))
    .expect("request json");

    let err = request.into_command(caller()).expect_err("invalid run id");

    assert_eq!(err.field, "run_id");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidId);
}

#[test]
fn missing_gate_ref_returns_stable_validation_error() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "resolution": "denied"
    }))
    .expect("request json");

    let err = request
        .into_command(caller())
        .expect_err("missing gate ref");

    assert_eq!(err.field, "gate_ref");
    assert_eq!(err.code, WebUiInboundValidationCode::MissingField);
}

#[test]
fn blank_credential_ref_returns_stable_validation_error() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "credential_provided",
        "credential_ref": ""
    }))
    .expect("request json");

    let err = request
        .into_command(caller())
        .expect_err("blank credential ref");

    assert_eq!(err.field, "credential_ref");
    assert_eq!(err.code, WebUiInboundValidationCode::Blank);
}

#[test]
fn command_serializes_with_stable_command_tag() {
    let request = WebUiSendMessageRequest {
        client_action_id: Some("send-1".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        content: Some("hello".to_string()),
        attachments: Vec::new(),
    };
    let command = request.into_command(caller()).expect("valid command");

    let value = serde_json::to_value(command).expect("command json");

    assert_eq!(value["command"], "send_message");
    assert_eq!(value["scope"]["thread_id"], "thread-alpha");
    assert_eq!(value["actor"]["user_id"], "user-alpha");
}

#[test]
fn token_fields_reject_control_characters() {
    let request = WebUiSendMessageRequest {
        client_action_id: Some("send\n1".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        content: Some("hello".to_string()),
        attachments: Vec::new(),
    };

    let err = request.into_command(caller()).expect_err("control char");

    assert_eq!(err.field, "client_action_id");
    assert_eq!(
        err.code,
        WebUiInboundValidationCode::InvalidControlCharacter
    );
}

#[test]
fn invalid_cancel_reason_returns_stable_validation_error() {
    let request = WebUiCancelRunRequest {
        client_action_id: Some("cancel-1".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        run_id: Some(run_id()),
        reason: Some("not_a_reason".to_string()),
    };

    let err = request.into_command(caller()).expect_err("invalid reason");

    assert_eq!(err.field, "reason");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn invalid_gate_resolution_returns_stable_validation_error() {
    let request = WebUiResolveGateRequest {
        client_action_id: Some("gate-1".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        run_id: Some(run_id()),
        gate_ref: Some("gate-alpha".to_string()),
        resolution: Some("not_a_resolution".to_string()),
        always: None,
        credential_ref: None,
    };

    let err = request
        .into_command(caller())
        .expect_err("invalid resolution");

    assert_eq!(err.field, "resolution");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn cancel_reason_defaults_to_user_requested() {
    let request = WebUiCancelRunRequest {
        client_action_id: Some("cancel-1".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        run_id: Some(run_id()),
        reason: None,
    };

    let WebUiInboundCommand::CancelRun { request } = request
        .into_command(caller())
        .expect("valid cancel command")
    else {
        panic!("expected cancel-run command");
    };

    assert_eq!(request.reason, SanitizedCancelReason::UserRequested);
}

#[test]
fn cancel_reason_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(WebUiCancelReason::Policy).expect("reason json"),
        json!("policy")
    );
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn send_with_attachments(attachments: Vec<WebUiInboundAttachment>) -> WebUiSendMessageRequest {
    WebUiSendMessageRequest {
        client_action_id: Some("send-att".to_string()),
        thread_id: Some("thread-alpha".to_string()),
        content: Some("see attached".to_string()),
        attachments,
    }
}

#[test]
fn decode_attachments_decodes_metadata_kind_and_bytes() {
    let request = send_with_attachments(vec![
        WebUiInboundAttachment {
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
            data_base64: b64(b"%PDF-1.7 body"),
        },
        WebUiInboundAttachment {
            // Uppercase + charset params normalize; kind derives from registry.
            mime_type: "IMAGE/PNG; charset=binary".to_string(),
            filename: None,
            data_base64: b64(&[0x89, 0x50, 0x4E, 0x47]),
        },
    ]);

    let decoded = request
        .decode_attachments()
        .expect("valid attachments decode");
    assert_eq!(decoded.len(), 2);

    // `kind`/`fallback_extension` are derived from `mime_type` inside the
    // landing bridge, so the decoded DTO carries only the raw upload fields.
    assert_eq!(decoded[0].mime_type, "application/pdf");
    assert_eq!(decoded[0].filename.as_deref(), Some("report.pdf"));
    assert_eq!(decoded[0].bytes, b"%PDF-1.7 body");

    assert_eq!(decoded[1].mime_type, "image/png");
    assert!(decoded[1].filename.is_none());
}

#[test]
fn decode_attachments_rejects_unsupported_mime() {
    let request = send_with_attachments(vec![WebUiInboundAttachment {
        mime_type: "image/svg+xml".to_string(),
        filename: None,
        data_base64: b64(b"<svg/>"),
    }]);
    let err = request
        .decode_attachments()
        .expect_err("svg is unsupported");
    assert_eq!(err.field, "attachments.mime_type");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn decode_attachments_rejects_malformed_base64() {
    let request = send_with_attachments(vec![WebUiInboundAttachment {
        mime_type: "application/pdf".to_string(),
        filename: None,
        data_base64: "not valid base64!!!".to_string(),
    }]);
    let err = request.decode_attachments().expect_err("bad base64");
    assert_eq!(err.field, "attachments.data_base64");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn decode_attachments_rejects_per_file_oversize() {
    let request = send_with_attachments(vec![WebUiInboundAttachment {
        mime_type: "application/pdf".to_string(),
        filename: None,
        data_base64: b64(&vec![0u8; 5 * 1024 * 1024 + 1]),
    }]);
    let err = request.decode_attachments().expect_err("over per-file cap");
    assert_eq!(err.field, "attachments");
    assert_eq!(err.code, WebUiInboundValidationCode::TooLong);
}

#[test]
fn decode_attachments_rejects_total_oversize() {
    let three_mib = vec![0u8; 3 * 1024 * 1024];
    let request = send_with_attachments(vec![
        WebUiInboundAttachment {
            mime_type: "application/pdf".to_string(),
            filename: None,
            data_base64: b64(&three_mib),
        };
        4 // 12 MiB total > 10 MiB cap
    ]);
    let err = request.decode_attachments().expect_err("over total cap");
    assert_eq!(err.field, "attachments");
    assert_eq!(err.code, WebUiInboundValidationCode::TooLong);
}

#[test]
fn decode_attachments_rejects_too_many() {
    let request = send_with_attachments(vec![
        WebUiInboundAttachment {
            mime_type: "text/plain".to_string(),
            filename: None,
            data_base64: b64(b"x"),
        };
        11 // > MAX_INLINE_ATTACHMENTS (10)
    ]);
    let err = request
        .decode_attachments()
        .expect_err("too many attachments");
    assert_eq!(err.field, "attachments");
    assert_eq!(err.code, WebUiInboundValidationCode::TooLong);
}

#[test]
fn decode_attachments_empty_is_ok() {
    let request = send_with_attachments(Vec::new());
    assert!(
        request
            .decode_attachments()
            .expect("empty decodes")
            .is_empty()
    );
}

/// Wire-stability: both legacy strings ("denied" and "cancelled") must still
/// deserialize to `WebUiGateResolution::Declined` after the unification.
/// This guards against accidental serde rename breakage.
#[test]
fn legacy_denied_wire_string_deserializes_to_declined() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "denied"
    }))
    .expect("denied request json");
    let command = request.into_command(caller()).expect("valid command");
    let WebUiInboundCommand::ResolveGate { resolution, .. } = command else {
        panic!("expected resolve-gate command");
    };
    assert_eq!(resolution, WebUiGateResolution::Declined);
}

#[test]
fn legacy_cancelled_wire_string_deserializes_to_declined() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "cancelled"
    }))
    .expect("cancelled request json");
    let command = request.into_command(caller()).expect("valid command");
    let WebUiInboundCommand::ResolveGate { resolution, .. } = command else {
        panic!("expected resolve-gate command");
    };
    assert_eq!(resolution, WebUiGateResolution::Declined);
}
