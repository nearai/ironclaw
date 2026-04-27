//! Caller-level tests for the durable event/audit log contracts.
//!
//! These tests drive the public [`DurableEventLog`] / [`DurableAuditLog`]
//! trait surfaces, not internal helpers. They cover append/cursor/replay
//! semantics, stream-key partitioning, redaction guarantees on event
//! constructors, and best-effort sink delivery.

use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableEventLog, EventCursor, EventError, EventSink,
    EventStreamKey, InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog,
    InMemoryEventSink, RuntimeEvent, RuntimeEventKind, parse_jsonl, replay_jsonl,
    sanitize_error_kind,
};
use ironclaw_host_api::{
    Action, ActionSummary, AgentId, ApprovalRequest, ApprovalRequestId, AuditEnvelope,
    CapabilityId, CorrelationId, DenyReason, ExecutionContext, ExtensionId, InvocationId,
    MountView, Principal, ProjectId, ResourceEstimate, ResourceScope, RuntimeKind, TenantId,
    UserId,
};

fn capability_id() -> CapabilityId {
    CapabilityId::new("demo.do_thing").expect("capability id")
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("demo").expect("extension id")
}

fn local_scope(user: &str, agent: Option<&str>) -> ResourceScope {
    let user_id = UserId::new(user).expect("user id");
    let agent_id = agent.map(|a| AgentId::new(a).expect("agent id"));
    ResourceScope {
        tenant_id: TenantId::new("default").expect("tenant id"),
        user_id,
        agent_id,
        project_id: Some(ProjectId::new("bootstrap").expect("project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[tokio::test]
async fn durable_event_log_appends_and_replays_in_order() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));

    let e1 = RuntimeEvent::dispatch_requested(scope.clone(), capability_id());
    let e2 = RuntimeEvent::runtime_selected(
        scope.clone(),
        capability_id(),
        extension_id(),
        RuntimeKind::Wasm,
    );
    let e3 = RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability_id(),
        extension_id(),
        RuntimeKind::Wasm,
        42,
    );

    let entry1 = log.append(e1).await.expect("append 1");
    let entry2 = log.append(e2).await.expect("append 2");
    let entry3 = log.append(e3).await.expect("append 3");

    assert_eq!(entry1.cursor, EventCursor::new(1));
    assert_eq!(entry2.cursor, EventCursor::new(2));
    assert_eq!(entry3.cursor, EventCursor::new(3));

    let stream = EventStreamKey::from_scope(&scope);
    let replay = log
        .read_after_cursor(&stream, None, 10)
        .await
        .expect("replay from origin");
    assert_eq!(replay.entries.len(), 3);
    assert_eq!(replay.next_cursor, EventCursor::new(3));
    assert_eq!(
        replay.entries[0].record.kind,
        RuntimeEventKind::DispatchRequested
    );
    assert_eq!(
        replay.entries[1].record.kind,
        RuntimeEventKind::RuntimeSelected
    );
    assert_eq!(
        replay.entries[2].record.kind,
        RuntimeEventKind::DispatchSucceeded
    );
}

#[tokio::test]
async fn read_after_next_cursor_returns_empty_replay() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability_id(),
    ))
    .await
    .expect("append");

    let first = log
        .read_after_cursor(&stream, None, 10)
        .await
        .expect("first replay");
    let after = first.next_cursor;

    let second = log
        .read_after_cursor(&stream, Some(after), 10)
        .await
        .expect("second replay");

    assert!(second.entries.is_empty());
    assert_eq!(second.next_cursor, after);
}

#[tokio::test]
async fn replay_respects_limit_and_resumes_cleanly() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    for _ in 0..7 {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append");
    }

    let first = log
        .read_after_cursor(&stream, None, 3)
        .await
        .expect("limited replay");
    assert_eq!(first.entries.len(), 3);
    assert_eq!(first.next_cursor, EventCursor::new(3));

    let second = log
        .read_after_cursor(&stream, Some(first.next_cursor), 3)
        .await
        .expect("second limited replay");
    assert_eq!(second.entries.len(), 3);
    assert_eq!(second.next_cursor, EventCursor::new(6));

    let third = log
        .read_after_cursor(&stream, Some(second.next_cursor), 3)
        .await
        .expect("third limited replay");
    assert_eq!(third.entries.len(), 1);
    assert_eq!(third.next_cursor, EventCursor::new(7));
}

#[tokio::test]
async fn streams_partition_by_tenant_user_agent() {
    let log = InMemoryDurableEventLog::new();
    let alice = local_scope("alice", Some("default"));
    let bob = local_scope("bob", Some("default"));
    let alice_other_agent = local_scope("alice", Some("research"));

    log.append(RuntimeEvent::dispatch_requested(
        alice.clone(),
        capability_id(),
    ))
    .await
    .expect("alice append 1");
    log.append(RuntimeEvent::dispatch_requested(
        alice.clone(),
        capability_id(),
    ))
    .await
    .expect("alice append 2");
    log.append(RuntimeEvent::dispatch_requested(
        bob.clone(),
        capability_id(),
    ))
    .await
    .expect("bob append");
    log.append(RuntimeEvent::dispatch_requested(
        alice_other_agent.clone(),
        capability_id(),
    ))
    .await
    .expect("alice research append");

    let alice_replay = log
        .read_after_cursor(&EventStreamKey::from_scope(&alice), None, 10)
        .await
        .expect("alice replay");
    let bob_replay = log
        .read_after_cursor(&EventStreamKey::from_scope(&bob), None, 10)
        .await
        .expect("bob replay");
    let alice_research_replay = log
        .read_after_cursor(&EventStreamKey::from_scope(&alice_other_agent), None, 10)
        .await
        .expect("alice research replay");

    assert_eq!(alice_replay.entries.len(), 2);
    assert_eq!(bob_replay.entries.len(), 1);
    assert_eq!(alice_research_replay.entries.len(), 1);

    // Cursors are per-stream monotonic — every stream begins at 1 regardless
    // of global ordering.
    assert_eq!(alice_replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(bob_replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(alice_research_replay.entries[0].cursor, EventCursor::new(1));
}

#[tokio::test]
async fn read_empty_stream_returns_echoed_cursor() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("nobody", None);
    let stream = EventStreamKey::from_scope(&scope);

    let replay = log
        .read_after_cursor(&stream, None, 10)
        .await
        .expect("replay empty");
    assert!(replay.entries.is_empty());
    assert_eq!(replay.next_cursor, EventCursor::origin());

    let replay = log
        .read_after_cursor(&stream, Some(EventCursor::new(42)), 10)
        .await
        .expect("replay empty with cursor");
    assert!(replay.entries.is_empty());
    assert_eq!(replay.next_cursor, EventCursor::new(42));
}

#[tokio::test]
async fn replay_with_zero_limit_is_rejected() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    let result = log.read_after_cursor(&stream, None, 0).await;
    assert!(matches!(
        result,
        Err(EventError::InvalidReplayRequest { .. })
    ));
}

#[tokio::test]
async fn dispatch_failed_redacts_unsafe_error_kind() {
    let scope = local_scope("alice", Some("default"));

    // Long, free-form error text (paths, secrets, exception messages) is
    // exactly what must not survive into a durable event.
    let unsafe_message = "failed to read /etc/passwd: secret value abc123 leaked";
    let event = RuntimeEvent::dispatch_failed(
        scope,
        capability_id(),
        Some(extension_id()),
        Some(RuntimeKind::Wasm),
        unsafe_message,
    );

    assert_eq!(event.error_kind.as_deref(), Some("Unclassified"));
}

#[tokio::test]
async fn dispatch_failed_preserves_safe_classification_token() {
    let scope = local_scope("alice", Some("default"));

    let event = RuntimeEvent::dispatch_failed(
        scope,
        capability_id(),
        Some(extension_id()),
        Some(RuntimeKind::Wasm),
        "missing_runtime_backend",
    );

    assert_eq!(event.error_kind.as_deref(), Some("missing_runtime_backend"));
}

#[tokio::test]
async fn sanitize_error_kind_collapses_long_or_unsafe_input() {
    assert_eq!(sanitize_error_kind(""), "Unclassified");
    assert_eq!(sanitize_error_kind("hello world"), "Unclassified"); // space
    assert_eq!(sanitize_error_kind("path/like/value"), "Unclassified"); // slash
    assert_eq!(sanitize_error_kind("a".repeat(129)), "Unclassified"); // length
    assert_eq!(sanitize_error_kind("ok_value-1.2:tag"), "ok_value-1.2:tag");
}

#[tokio::test]
async fn appended_event_payload_omits_raw_payloads_by_construction() {
    // The constructor surface intentionally does not accept raw input/output
    // payloads, paths, or secret material. This test pins that the wire
    // shape carries only typed metadata.
    let scope = local_scope("alice", Some("default"));
    let event = RuntimeEvent::dispatch_succeeded(
        scope,
        capability_id(),
        extension_id(),
        RuntimeKind::Wasm,
        128,
    );

    let json = serde_json::to_string(&event).expect("serialize event");

    // Spot-check that the serialized form contains expected typed fields and
    // does not contain any forbidden categories. (We can only assert what we
    // didn't put in; that's the point — the wire shape is the contract.)
    assert!(json.contains("\"kind\":\"dispatch_succeeded\""));
    assert!(json.contains("\"output_bytes\":128"));
    assert!(!json.contains("password"));
    assert!(!json.contains("token"));
}

#[tokio::test]
async fn best_effort_event_sink_records_emit_calls() {
    let sink = InMemoryEventSink::new();
    let scope = local_scope("alice", Some("default"));

    sink.emit(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability_id(),
    ))
    .await
    .expect("emit");
    sink.emit(RuntimeEvent::dispatch_succeeded(
        scope,
        capability_id(),
        extension_id(),
        RuntimeKind::Wasm,
        7,
    ))
    .await
    .expect("emit");

    let captured = sink.events();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[1].output_bytes, Some(7));
}

#[tokio::test]
async fn durable_audit_log_appends_and_replays() {
    let log = InMemoryDurableAuditLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    let ctx = ExecutionContext::local_default(
        scope.user_id.clone(),
        extension_id(),
        RuntimeKind::Wasm,
        ironclaw_host_api::TrustClass::FirstParty,
        Default::default(),
        MountView::default(),
    )
    .expect("local default execution context");

    let denied = AuditEnvelope::denied(
        &ctx,
        ironclaw_host_api::AuditStage::Denied,
        ActionSummary::from_action(&Action::Dispatch {
            capability: capability_id(),
            estimated_resources: ResourceEstimate::default(),
        }),
        DenyReason::MissingGrant,
    );

    let entry = log.append(denied).await.expect("append audit");
    assert_eq!(entry.cursor, EventCursor::new(1));

    let replay = log
        .read_after_cursor(&stream, None, 10)
        .await
        .expect("replay audit");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(replay.entries[0].record.decision.kind, "deny");
}

#[tokio::test]
async fn approval_audit_records_partition_by_stream_key() {
    let log = InMemoryDurableAuditLog::new();
    let alice_scope = local_scope("alice", Some("default"));
    let bob_scope = local_scope("bob", Some("default"));

    let alice_request = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(alice_scope.user_id.clone()),
        action: Box::new(Action::Dispatch {
            capability: capability_id(),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "test approval".to_string(),
        reusable_scope: None,
    };
    let alice_audit = AuditEnvelope::approval_resolved(
        &alice_scope,
        &alice_request,
        Principal::User(alice_scope.user_id.clone()),
        "approved",
    );
    let bob_request = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(bob_scope.user_id.clone()),
        action: Box::new(Action::Dispatch {
            capability: capability_id(),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: "test approval".to_string(),
        reusable_scope: None,
    };
    let bob_audit = AuditEnvelope::approval_resolved(
        &bob_scope,
        &bob_request,
        Principal::User(bob_scope.user_id.clone()),
        "approved",
    );

    log.append(alice_audit).await.expect("alice audit");
    log.append(bob_audit).await.expect("bob audit");

    let alice_replay = log
        .read_after_cursor(&EventStreamKey::from_scope(&alice_scope), None, 10)
        .await
        .expect("alice replay");
    let bob_replay = log
        .read_after_cursor(&EventStreamKey::from_scope(&bob_scope), None, 10)
        .await
        .expect("bob replay");

    assert_eq!(alice_replay.entries.len(), 1);
    assert_eq!(bob_replay.entries.len(), 1);
    assert_eq!(alice_replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(bob_replay.entries[0].cursor, EventCursor::new(1));
}

#[tokio::test]
async fn best_effort_audit_sink_captures_records() {
    let sink = InMemoryAuditSink::new();
    let scope = local_scope("alice", Some("default"));
    let ctx = ExecutionContext::local_default(
        scope.user_id.clone(),
        extension_id(),
        RuntimeKind::Wasm,
        ironclaw_host_api::TrustClass::FirstParty,
        Default::default(),
        MountView::default(),
    )
    .expect("local default execution context");
    let record = AuditEnvelope::denied(
        &ctx,
        ironclaw_host_api::AuditStage::Denied,
        ActionSummary::from_action(&Action::Dispatch {
            capability: capability_id(),
            estimated_resources: ResourceEstimate::default(),
        }),
        DenyReason::PolicyDenied,
    );
    sink.emit_audit(record).await.expect("audit emit");

    assert_eq!(sink.records().len(), 1);
}

#[tokio::test]
async fn parse_jsonl_round_trips_runtime_events() {
    let scope = local_scope("alice", Some("default"));
    let event = RuntimeEvent::dispatch_requested(scope, capability_id());
    let line = serde_json::to_vec(&event).expect("serialize event");
    let mut bytes = line;
    bytes.push(b'\n');

    let parsed: Vec<RuntimeEvent> = parse_jsonl(&bytes).expect("parse jsonl");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].event_id, event.event_id);
}

#[tokio::test]
async fn parse_jsonl_rejects_malformed_line_rather_than_silently_skipping() {
    let bytes = b"{\"not\":\"a runtime event\"}\nsomething not even json\n";
    let result: Result<Vec<RuntimeEvent>, _> = parse_jsonl(bytes);
    assert!(matches!(result, Err(EventError::Serialize { .. })));
}

#[tokio::test]
async fn replay_jsonl_advances_cursor_with_limit() {
    let scope = local_scope("alice", Some("default"));
    let mut bytes = Vec::new();
    for _ in 0..5 {
        let event = RuntimeEvent::dispatch_requested(scope.clone(), capability_id());
        bytes.extend(serde_json::to_vec(&event).expect("serialize"));
        bytes.push(b'\n');
    }

    let first: ironclaw_events::EventReplay<RuntimeEvent> =
        replay_jsonl(&bytes, None, 2).expect("first replay");
    assert_eq!(first.entries.len(), 2);
    assert_eq!(first.next_cursor, EventCursor::new(2));

    let second: ironclaw_events::EventReplay<RuntimeEvent> =
        replay_jsonl(&bytes, Some(first.next_cursor), 10).expect("second replay");
    assert_eq!(second.entries.len(), 3);
    assert_eq!(second.next_cursor, EventCursor::new(5));
}

#[tokio::test]
async fn replay_jsonl_with_zero_limit_is_rejected() {
    let bytes = b"";
    let result: Result<ironclaw_events::EventReplay<RuntimeEvent>, _> =
        replay_jsonl(bytes, None, 0);
    assert!(matches!(
        result,
        Err(EventError::InvalidReplayRequest { .. })
    ));
}
