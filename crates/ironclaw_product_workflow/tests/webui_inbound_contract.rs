//! Contract tests for route-independent WebUI inbound DTOs.

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    WebUiAuthenticatedCaller, WebUiCancelReason, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiGateResolution, WebUiInboundCommand, WebUiInboundValidationCode, WebUiResolveGateRequest,
    WebUiSendMessageRequest,
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
fn resolve_gate_maps_attested_proof_to_canonical_command() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-att-1",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "attested",
        "attested_proof_kind": "injected_wallet",
        "attested_approved_tx_hash": "ab".repeat(32),
        "attested_proof": {
            "scheme": "solana",
            "claimed_signer": "deadbeef",
            "signature": "00".repeat(64),
            "approved_tx_hash": "ab".repeat(32),
            "public_key": "11".repeat(32)
        }
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");
    let WebUiInboundCommand::ResolveGate { resolution, .. } = command else {
        panic!("expected resolve-gate command");
    };
    let WebUiGateResolution::Attested {
        kind,
        approved_tx_hash_hex,
        proof_json,
    } = resolution
    else {
        panic!("expected attested resolution");
    };
    assert_eq!(
        kind,
        ironclaw_product_workflow::AttestedProofKind::InjectedWallet
    );
    assert_eq!(approved_tx_hash_hex, "ab".repeat(32));
    assert!(proof_json.is_object());
}

#[test]
fn attested_resolution_normalizes_0x_prefixed_uppercase_hash() {
    // A documented `0x`-prefixed (and here uppercase) hash must be canonicalized
    // to the lowercase, prefix-free form the resume port compares against, so an
    // otherwise-valid proof is not rejected as a binding mismatch.
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-att-0x",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "attested",
        "attested_proof_kind": "injected_wallet",
        "attested_approved_tx_hash": format!("0x{}", "AB".repeat(32)),
        "attested_proof": {
            "scheme": "solana",
            "claimed_signer": "deadbeef",
            "signature": "00".repeat(64),
            "approved_tx_hash": "ab".repeat(32),
            "public_key": "11".repeat(32)
        }
    }))
    .expect("request json");

    let command = request.into_command(caller()).expect("valid command");
    let WebUiInboundCommand::ResolveGate { resolution, .. } = command else {
        panic!("expected resolve-gate command");
    };
    let WebUiGateResolution::Attested {
        approved_tx_hash_hex,
        ..
    } = resolution
    else {
        panic!("expected attested resolution");
    };
    assert_eq!(approved_tx_hash_hex, "ab".repeat(32));
}

#[test]
fn attested_resolution_rejects_wrong_length_hash() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-att-bad",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "attested",
        "attested_proof_kind": "injected_wallet",
        "attested_approved_tx_hash": "abcd",
        "attested_proof": {"k": "v"}
    }))
    .expect("request json");

    let err = request
        .into_command(caller())
        .expect_err("short hash must reject");
    assert_eq!(err.field, "attested_approved_tx_hash");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn attested_resolution_rejects_unknown_proof_kind() {
    let request: WebUiResolveGateRequest = serde_json::from_value(json!({
        "client_action_id": "gate-att-2",
        "thread_id": "thread-alpha",
        "run_id": run_id(),
        "gate_ref": "gate-alpha",
        "resolution": "attested",
        "attested_proof_kind": "bitcoin",
        "attested_approved_tx_hash": "ab".repeat(32),
        "attested_proof": {"k": "v"}
    }))
    .expect("request json");

    let err = request
        .into_command(caller())
        .expect_err("unknown proof kind must reject");
    assert_eq!(err.field, "attested_proof_kind");
    assert_eq!(err.code, WebUiInboundValidationCode::InvalidValue);
}

#[test]
fn attested_resolution_rejects_non_object_proof() {
    // `attested_proof` must be a JSON object; array / string / number / bool /
    // null all fail closed as an InvalidValue on `attested_proof`.
    for bad in [
        json!([1, 2, 3]),
        json!("not-an-object"),
        json!(42),
        json!(true),
        json!(null),
    ] {
        let request: WebUiResolveGateRequest = serde_json::from_value(json!({
            "client_action_id": "gate-att-nonobj",
            "thread_id": "thread-alpha",
            "run_id": run_id(),
            "gate_ref": "gate-alpha",
            "resolution": "attested",
            "attested_proof_kind": "injected_wallet",
            "attested_approved_tx_hash": "ab".repeat(32),
            "attested_proof": bad.clone(),
        }))
        .expect("request json");

        // `null` deserializes to `None` -> MissingField; every other non-object
        // shape reaches the `is_object()` guard -> InvalidValue. Both fail closed
        // on the `attested_proof` field.
        let err = request
            .into_command(caller())
            .expect_err("non-object attested_proof must reject");
        assert_eq!(err.field, "attested_proof", "input: {bad}");
        assert!(
            matches!(
                err.code,
                WebUiInboundValidationCode::InvalidValue | WebUiInboundValidationCode::MissingField
            ),
            "input {bad} produced unexpected code {:?}",
            err.code
        );
    }
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
        ..Default::default()
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
