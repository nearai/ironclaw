use chrono::Utc;
use ironclaw_host_api::ThreadId;
use ironclaw_product_workflow::{
    FinalReplyView, GatePromptView, ProductOutboundPayload, ProductProjectionItem,
    ProductProjectionState, ProgressKind, ProgressUpdateView, ProjectionCursor,
    RebornCancelRunResponse, RebornGetRunStateResponse, RebornSubmitTurnResponse,
};
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, RunProfileId, RunProfileVersion, SanitizedFailure, TurnRunId,
    TurnStatus,
};
use ironclaw_webui_v2::{WebChatV2Event, WebChatV2EventFrame};
use serde_json::Value;

fn cursor() -> ProjectionCursor {
    ProjectionCursor::new("cursor:webchat:v2:1").expect("cursor")
}

fn run_id() -> TurnRunId {
    TurnRunId::new()
}

fn progress(kind: ProgressKind) -> ProgressUpdateView {
    ProgressUpdateView {
        turn_run_id: run_id(),
        kind,
        generated_at: Utc::now(),
    }
}

fn final_reply() -> FinalReplyView {
    FinalReplyView {
        turn_run_id: run_id(),
        text: "done".to_string(),
        generated_at: Utc::now(),
    }
}

fn gate_prompt() -> GatePromptView {
    GatePromptView {
        turn_run_id: run_id(),
        gate_ref: "gate:approval".to_string(),
        headline: "Approve action".to_string(),
        body: "Review the requested action.".to_string(),
    }
}

fn accepted_ack() -> RebornSubmitTurnResponse {
    RebornSubmitTurnResponse::Submitted {
        thread_id: ThreadId::new("thread-alpha").expect("thread"),
        accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("message ref"),
        turn_id: "turn-alpha".to_string(),
        run_id: run_id(),
        status: TurnStatus::Queued,
        resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
        resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
        event_cursor: EventCursor(1),
    }
}

fn cancelled_response() -> RebornCancelRunResponse {
    RebornCancelRunResponse {
        run_id: run_id(),
        status: TurnStatus::Cancelled,
        event_cursor: EventCursor(2),
        already_terminal: false,
    }
}

fn failed_run_state() -> RebornGetRunStateResponse {
    RebornGetRunStateResponse {
        turn_id: "turn-failed".to_string(),
        run_id: run_id(),
        status: TurnStatus::Failed,
        event_cursor: EventCursor(3),
        accepted_message_ref: AcceptedMessageRef::new("msg:failed").expect("message ref"),
        resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
        resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        failure: Some(SanitizedFailure::new("model_unavailable").expect("sanitized failure")),
    }
}

fn projection_state() -> ProductProjectionState {
    ProductProjectionState::new(
        "thread-alpha",
        vec![
            ProductProjectionItem::Text {
                id: "message-1".to_string(),
                body: "hello".to_string(),
            },
            ProductProjectionItem::RunStatus {
                run_id: run_id(),
                status: "running".to_string(),
            },
        ],
    )
    .expect("projection state")
}

#[test]
fn webchat_v2_event_schema_has_stable_wire_names() {
    let cases = vec![
        (
            WebChatV2Event::Accepted {
                ack: accepted_ack(),
            },
            "accepted",
        ),
        (
            WebChatV2Event::Running {
                progress: progress(ProgressKind::Typing),
            },
            "running",
        ),
        (
            WebChatV2Event::CapabilityProgress {
                progress: progress(ProgressKind::ToolRunning),
            },
            "capability_progress",
        ),
        (
            WebChatV2Event::Gate {
                prompt: gate_prompt(),
            },
            "gate",
        ),
        (
            WebChatV2Event::FinalReply {
                reply: final_reply(),
            },
            "final_reply",
        ),
        (
            WebChatV2Event::Cancelled {
                response: cancelled_response(),
            },
            "cancelled",
        ),
        (
            WebChatV2Event::Failed {
                run_state: failed_run_state(),
            },
            "failed",
        ),
        (
            WebChatV2Event::ProjectionSnapshot {
                state: projection_state(),
            },
            "projection_snapshot",
        ),
        (
            WebChatV2Event::ProjectionUpdate {
                state: projection_state(),
            },
            "projection_update",
        ),
        (WebChatV2Event::KeepAlive, "keep_alive"),
    ];

    for (event, expected_type) in cases {
        assert_eq!(event.event_name(), expected_type);
        let frame = WebChatV2EventFrame {
            cursor: cursor(),
            event,
        };
        let json = serde_json::to_value(&frame).expect("serialize frame");
        assert_eq!(json["cursor"], "cursor:webchat:v2:1");
        assert_eq!(json["type"], expected_type);
        assert_no_forbidden_metadata(&json);
    }
}

#[test]
fn outbound_payload_mapping_splits_tool_progress_from_generic_running() {
    let typing = WebChatV2Event::from(ProductOutboundPayload::Progress(progress(
        ProgressKind::Typing,
    )));
    assert_eq!(typing.event_name(), "running");

    let tool = WebChatV2Event::from(ProductOutboundPayload::Progress(progress(
        ProgressKind::ToolRunning,
    )));
    assert_eq!(tool.event_name(), "capability_progress");

    let reply = WebChatV2Event::from(ProductOutboundPayload::FinalReply(final_reply()));
    assert_eq!(reply.event_name(), "final_reply");
}

fn assert_no_forbidden_metadata(json: &Value) {
    let rendered = serde_json::to_string(json).expect("json string");
    for forbidden in [
        "adapter_id",
        "installation_id",
        "target",
        "reply_target_binding_ref",
        "external_conversation_ref",
        "delivery_attempt_id",
        "SECRET_SENTINEL",
        "HOST_PATH_SENTINEL",
        "RAW_PROVIDER_ERROR_SENTINEL",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "browser event schema must not expose {forbidden}: {rendered}"
        );
    }
}
