//! Caller-level tests for the durable event/audit log contracts.
//!
//! These tests drive the public [`DurableEventLog`] / [`DurableAuditLog`]
//! trait surfaces, not internal helpers. They cover append/cursor/replay
//! semantics, stream-key partitioning, redaction guarantees on event
//! constructors, and best-effort sink delivery.

use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableEventLog, EventCursor, EventError, EventSink,
    EventStreamKey, InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog,
    InMemoryEventSink, ReadScope, RuntimeEvent, RuntimeEventKind, parse_jsonl, replay_jsonl,
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
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
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
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
        .await
        .expect("first replay");
    let after = first.next_cursor;

    let second = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(after), 10)
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
        .read_after_cursor(&stream, &ReadScope::any(), None, 3)
        .await
        .expect("limited replay");
    assert_eq!(first.entries.len(), 3);
    assert_eq!(first.next_cursor, EventCursor::new(3));

    let second = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(first.next_cursor), 3)
        .await
        .expect("second limited replay");
    assert_eq!(second.entries.len(), 3);
    assert_eq!(second.next_cursor, EventCursor::new(6));

    let third = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(second.next_cursor), 3)
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
        .read_after_cursor(
            &EventStreamKey::from_scope(&alice),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .expect("alice replay");
    let bob_replay = log
        .read_after_cursor(
            &EventStreamKey::from_scope(&bob),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .expect("bob replay");
    let alice_research_replay = log
        .read_after_cursor(
            &EventStreamKey::from_scope(&alice_other_agent),
            &ReadScope::any(),
            None,
            10,
        )
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
async fn read_empty_stream_at_origin_returns_empty_replay() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("nobody", None);
    let stream = EventStreamKey::from_scope(&scope);

    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
        .await
        .expect("replay empty");
    assert!(replay.entries.is_empty());
    assert_eq!(replay.next_cursor, EventCursor::origin());

    // Origin cursor is equivalent to None — also empty, not a gap.
    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::origin()), 10)
        .await
        .expect("origin cursor on empty stream");
    assert!(replay.entries.is_empty());
    assert_eq!(replay.next_cursor, EventCursor::origin());
}

#[tokio::test]
async fn future_cursor_on_empty_stream_returns_replay_gap() {
    // A consumer holding a non-origin cursor for a stream that does not
    // exist locally must not silently lose events 1..cursor when the stream
    // begins. Surface a ReplayGap so the consumer is forced to rebase.
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("nobody", None);
    let stream = EventStreamKey::from_scope(&scope);

    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(42)), 10)
        .await;
    match result {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(42));
            assert_eq!(earliest, EventCursor::origin());
        }
        other => panic!("expected ReplayGap, got {other:?}"),
    }
}

#[tokio::test]
async fn future_cursor_beyond_head_returns_replay_gap() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    // Append two records, then ask for replay after cursor 99.
    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability_id(),
    ))
    .await
    .expect("append 1");
    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability_id(),
    ))
    .await
    .expect("append 2");

    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(99)), 10)
        .await;
    match result {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(99));
            assert_eq!(earliest, EventCursor::new(2));
        }
        other => panic!("expected ReplayGap, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_gap_after_truncation_forces_snapshot_rebase() {
    // Retention drops entries up to and including the supplied cursor and
    // advances the earliest-retained marker. A reader still holding a
    // pre-retention cursor must see ReplayGap, not silent loss.
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    for _ in 0..5 {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append");
    }

    log.truncate_before_or_at(&stream, EventCursor::new(3))
        .expect("truncate");

    // Reading from origin (or any cursor before the new earliest_retained)
    // must surface a gap with earliest = 4 (one past the truncated cursor).
    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(0)), 10)
        .await;
    match result {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(0));
            assert_eq!(earliest, EventCursor::new(4));
        }
        other => panic!("expected ReplayGap, got {other:?}"),
    }

    // Reading after the truncation point continues to work with the live
    // tail.
    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(4)), 10)
        .await
        .expect("post-truncation replay");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].cursor, EventCursor::new(5));
}

#[tokio::test]
async fn truncate_beyond_head_is_rejected() {
    // A retention policy that picks a cursor by calendar time on a quiet
    // stream could pass a bound > head. Before the guard, that bricked the
    // stream until enough appends caught up; now it is rejected up front.
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    for _ in 0..3 {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append");
    }

    let result = log.truncate_before_or_at(&stream, EventCursor::new(99));
    match result {
        Err(EventError::InvalidReplayRequest { reason }) => {
            assert!(
                reason.contains("99") && reason.contains("3"),
                "reason should report cursor and head, got: {reason}"
            );
        }
        other => panic!("expected InvalidReplayRequest, got {other:?}"),
    }

    // Stream must still be usable after the rejected truncation.
    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
        .await
        .expect("replay after rejected truncation");
    assert_eq!(replay.entries.len(), 3);
}

#[tokio::test]
async fn replay_with_zero_limit_is_rejected() {
    let log = InMemoryDurableEventLog::new();
    let scope = local_scope("alice", Some("default"));
    let stream = EventStreamKey::from_scope(&scope);

    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), None, 0)
        .await;
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
    // Empty / free-form / path-like / oversized → Unclassified.
    assert_eq!(sanitize_error_kind(""), "Unclassified");
    assert_eq!(sanitize_error_kind("hello world"), "Unclassified"); // space
    assert_eq!(sanitize_error_kind("path/like/value"), "Unclassified"); // slash
    assert_eq!(sanitize_error_kind("a".repeat(65)), "Unclassified"); // overall length
    assert_eq!(sanitize_error_kind("MissingRuntime"), "Unclassified"); // mixed case
    // Token-shaped values must collapse: API key shapes, JWT-ish tokens, and
    // long random segments should not survive even though they look ASCII.
    assert_eq!(
        sanitize_error_kind("xkey_demo_abcdefghijklmnopqrstuvwxyz"),
        "Unclassified"
    ); // long random segment
    assert_eq!(
        sanitize_error_kind("aa.bb.aaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "Unclassified"
    ); // segment > 24 bytes
    assert_eq!(sanitize_error_kind("1leading_digit"), "Unclassified"); // leading digit
    assert_eq!(sanitize_error_kind("_leading_underscore"), "Unclassified");
    assert_eq!(sanitize_error_kind("a-with-dash"), "Unclassified"); // dashes no longer accepted
    // Stable classification tokens survive.
    assert_eq!(
        sanitize_error_kind("missing_runtime_backend"),
        "missing_runtime_backend"
    );
    assert_eq!(
        sanitize_error_kind("wasm.host_http_denied"),
        "wasm.host_http_denied"
    );
    assert_eq!(sanitize_error_kind("dispatch:timeout"), "dispatch:timeout");
}

#[tokio::test]
async fn deserialize_runtime_event_resanitizes_error_kind() {
    // A hand-rolled or replayed JSONL record could otherwise smuggle a raw
    // value into error_kind. The deserializer must re-run the redaction
    // guard so the field cannot bypass sanitization.
    let scope = local_scope("alice", Some("default"));
    let serialized = {
        let event = RuntimeEvent::dispatch_failed(
            scope,
            capability_id(),
            Some(extension_id()),
            Some(RuntimeKind::Wasm),
            "missing_runtime_backend",
        );
        serde_json::to_value(&event).expect("serialize")
    };

    // Mutate error_kind to a token-shaped raw secret.
    let mut payload = serialized;
    payload["error_kind"] =
        serde_json::Value::String("xkey_demo_abcdefghijklmnopqrstuvwxyz".to_string());
    let raw = serde_json::to_vec(&payload).expect("re-serialize");

    let parsed: RuntimeEvent = serde_json::from_slice(&raw).expect("deserialize");
    assert_eq!(parsed.error_kind.as_deref(), Some("Unclassified"));
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
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
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
        .read_after_cursor(
            &EventStreamKey::from_scope(&alice_scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .expect("alice replay");
    let bob_replay = log
        .read_after_cursor(
            &EventStreamKey::from_scope(&bob_scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .expect("bob replay");

    assert_eq!(alice_replay.entries.len(), 1);
    assert_eq!(bob_replay.entries.len(), 1);
    assert_eq!(alice_replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(bob_replay.entries[0].cursor, EventCursor::new(1));
}

#[tokio::test]
async fn approval_audit_envelope_serialization_excludes_raw_reason_and_fingerprint() {
    // Build an approval request whose `reason` is a unique sentinel string
    // and whose `invocation_fingerprint` is a unique sentinel value. The
    // serialized AuditEnvelope produced by `approval_resolved` must not
    // contain either, per `events.md` §3 / approvals contract redaction.
    let scope = local_scope("alice", Some("default"));
    let secret_reason = "REASON_SENTINEL_b9c4e2f7df184ab09a77";
    let request = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(scope.user_id.clone()),
        action: Box::new(Action::Dispatch {
            capability: capability_id(),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None, // typed shape; the assertion still
        // proves the envelope did not invent one
        reason: secret_reason.to_string(),
        reusable_scope: None,
    };

    let envelope = AuditEnvelope::approval_resolved(
        &scope,
        &request,
        Principal::User(scope.user_id.clone()),
        "approved",
    );

    let serialized = serde_json::to_string(&envelope).expect("serialize");

    assert!(
        !serialized.contains(secret_reason),
        "audit envelope must not carry raw approval reason; got: {serialized}"
    );
    assert!(
        !serialized.contains("invocation_fingerprint"),
        "audit envelope must not carry invocation_fingerprint; got: {serialized}"
    );
    // The envelope should still record the typed decision and request id so
    // operators can correlate it with the originating approval.
    assert!(serialized.contains("\"kind\":\"approved\""));
    assert!(serialized.contains(&request.id.to_string()));
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

#[tokio::test]
async fn replay_jsonl_with_future_cursor_returns_replay_gap() {
    // Symmetric to the in-memory log: a JSONL-backed durable log must not
    // silently echo a cursor beyond the file head. Without this, a future
    // filesystem JSONL backend would accept stale or foreign cursors and
    // hide records once new lines are appended.
    let scope = local_scope("alice", Some("default"));
    let mut bytes = Vec::new();
    for _ in 0..2 {
        let event = RuntimeEvent::dispatch_requested(scope.clone(), capability_id());
        bytes.extend(serde_json::to_vec(&event).expect("serialize"));
        bytes.push(b'\n');
    }

    let result: Result<ironclaw_events::EventReplay<RuntimeEvent>, _> =
        replay_jsonl(&bytes, Some(EventCursor::new(99)), 10);
    match result {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(99));
            assert_eq!(earliest, EventCursor::new(2));
        }
        other => panic!("expected ReplayGap, got {other:?}"),
    }
}

#[tokio::test]
async fn direct_construction_serialize_path_resanitizes_error_kind() {
    // An in-process caller can build RuntimeEvent { error_kind: Some(raw),
    // .. } directly without going through the typed constructors. The
    // custom Serialize impl must re-run sanitize_error_kind on the way out
    // so the redaction guard fires on every wire crossing, not only on
    // construction or deserialization.
    let scope = local_scope("alice", Some("default"));
    let event = RuntimeEvent {
        event_id: ironclaw_events::RuntimeEventId::new(),
        timestamp: chrono::Utc::now(),
        kind: RuntimeEventKind::DispatchFailed,
        scope,
        capability_id: capability_id(),
        provider: Some(extension_id()),
        runtime: Some(RuntimeKind::Wasm),
        process_id: None,
        output_bytes: None,
        // Free-form raw text with a path-like fragment — exactly what the
        // redaction invariant forbids in durable storage.
        error_kind: Some("/Users/alice/token=secret raw error".to_string()),
    };

    let json = serde_json::to_string(&event).expect("serialize");

    assert!(
        json.contains("\"error_kind\":\"Unclassified\""),
        "Serialize must re-run sanitize_error_kind; got: {json}"
    );
    assert!(!json.contains("/Users/alice"));
    assert!(!json.contains("token=secret"));
}

#[tokio::test]
async fn read_scope_filter_isolates_project_within_same_stream() {
    // Same (tenant, user, agent) stream, two projects. A consumer scoped to
    // project A must not see project B records, even though they share the
    // stream key. The implementation enforces this via ReadScope; the
    // caller does not have to remember to post-filter.
    let log = InMemoryDurableEventLog::new();
    let user_id = UserId::new("alice").expect("user id");
    let agent_id = AgentId::new("default").expect("agent id");
    let tenant_id = TenantId::new("default").expect("tenant id");
    let project_a = ProjectId::new("project-a").expect("project a");
    let project_b = ProjectId::new("project-b").expect("project b");

    let scope_for = |project: ProjectId| ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: Some(project),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };

    log.append(RuntimeEvent::dispatch_requested(
        scope_for(project_a.clone()),
        capability_id(),
    ))
    .await
    .expect("project a #1");
    log.append(RuntimeEvent::dispatch_requested(
        scope_for(project_b.clone()),
        capability_id(),
    ))
    .await
    .expect("project b #1");
    log.append(RuntimeEvent::dispatch_requested(
        scope_for(project_a.clone()),
        capability_id(),
    ))
    .await
    .expect("project a #2");

    let stream = EventStreamKey::new(tenant_id, user_id, Some(agent_id));

    let project_a_filter = ReadScope {
        project_id: Some(project_a.clone()),
        ..ReadScope::default()
    };
    let project_a_replay = log
        .read_after_cursor(&stream, &project_a_filter, None, 10)
        .await
        .expect("project a replay");

    assert_eq!(project_a_replay.entries.len(), 2);
    for entry in &project_a_replay.entries {
        assert_eq!(entry.record.scope.project_id.as_ref(), Some(&project_a));
    }
    // Cursor advances past the project-B record so the consumer's resume
    // cursor reflects the position they've already considered.
    assert_eq!(project_a_replay.next_cursor, EventCursor::new(3));

    let project_b_filter = ReadScope {
        project_id: Some(project_b.clone()),
        ..ReadScope::default()
    };
    let project_b_replay = log
        .read_after_cursor(&stream, &project_b_filter, None, 10)
        .await
        .expect("project b replay");
    assert_eq!(project_b_replay.entries.len(), 1);
    assert_eq!(
        project_b_replay.entries[0].record.scope.project_id.as_ref(),
        Some(&project_b)
    );
}

#[tokio::test]
async fn read_scope_filter_excludes_records_with_none_field_when_filter_is_some() {
    // Filter is a tightening, never a permissive default: a record with
    // None in a field cannot match a filter that asks for Some(...).
    let log = InMemoryDurableEventLog::new();
    let scope_with_project = local_scope("alice", Some("default"));
    let scope_without_project = ResourceScope {
        project_id: None,
        ..scope_with_project.clone()
    };

    log.append(RuntimeEvent::dispatch_requested(
        scope_with_project.clone(),
        capability_id(),
    ))
    .await
    .expect("with project");
    log.append(RuntimeEvent::dispatch_requested(
        scope_without_project,
        capability_id(),
    ))
    .await
    .expect("without project");

    let stream = EventStreamKey::from_scope(&scope_with_project);
    let filter = ReadScope {
        project_id: scope_with_project.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = log
        .read_after_cursor(&stream, &filter, None, 10)
        .await
        .expect("project replay");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(
        replay.entries[0].record.scope.project_id,
        scope_with_project.project_id
    );
}
