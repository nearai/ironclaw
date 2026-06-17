use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriverDescriptor, CancellationPolicy, CapabilitySurfaceProfileId,
    CheckpointPolicy, CheckpointSchemaId, CheckpointStateRecord, CheckpointStateStore,
    ConcurrencyClass, ContextProfileId, EventCursor, GateRef, GetCheckpointStateRequest,
    GetLoopCheckpointRequest, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, InMemoryTurnStateStoreLimits, LoopCheckpointStateRef,
    LoopCheckpointStore, LoopDriverId, MAX_CHECKPOINT_STATE_PAYLOAD_BYTES, ModelProfileId,
    PutCheckpointStateRequest, PutLoopCheckpointRequest, RedactedCheckpointPayload,
    RedactedRunProfileProvenance, ReplyTargetBindingRef, ResolvedRunProfile, ResourceBudgetPolicy,
    ResourceBudgetTier, RunClassId, RunProfileFingerprint, RunProfileId, RunProfileVersion,
    RuntimeProfileConstraints, SchedulingClass, SourceBindingRef, SteeringPolicy, TurnActor,
    TurnCheckpointId, TurnCheckpointRecord, TurnEventKind, TurnId, TurnLifecycleEvent,
    TurnPersistenceSnapshot, TurnRunId, TurnRunProfile, TurnRunRecord, TurnRunState, TurnScope,
    TurnStatus, TurnTimestamp, run_profile::LoopCheckpointKind,
};

#[tokio::test]
async fn checkpoint_state_store_round_trips_scoped_state_ref() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-roundtrip");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let schema_id = CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap();
    let schema_version = RunProfileVersion::new(7);
    let payload = b"RAW_PROMPT_SENTINEL sk-secret /host/path tool_input".to_vec();

    let record = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope.clone(),
            turn_id,
            run_id,
            schema_id.clone(),
            schema_version,
            LoopCheckpointKind::BeforeModel,
            payload.clone(),
        ))
        .await
        .unwrap();

    assert!(record.state_ref.as_str().starts_with("checkpoint:"));
    assert!(!record.state_ref.as_str().contains(&turn_id.to_string()));
    assert!(!record.state_ref.as_str().contains(&run_id.to_string()));
    assert_eq!(record.scope, scope);
    assert_eq!(record.turn_id, turn_id);
    assert_eq!(record.run_id, run_id);
    assert_eq!(record.schema_id, schema_id);
    assert_eq!(record.schema_version, schema_version);
    assert_eq!(record.kind, LoopCheckpointKind::BeforeModel);
    assert_eq!(record.payload.as_bytes(), payload.as_slice());

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("stored checkpoint state should be returned for matching scope/run");

    assert_eq!(loaded, record);
}

#[tokio::test]
async fn loop_checkpoint_store_maps_checkpoint_ids_to_staged_state_refs() {
    let state_store = InMemoryCheckpointStateStore::default();
    let checkpoint_store = InMemoryLoopCheckpointStore::default();
    let scope = turn_scope("thread-loop-checkpoint-roundtrip");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_record = put_test_state(&state_store, scope.clone(), turn_id, run_id).await;

    let checkpoint = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_record.state_ref.clone(),
            schema_id: state_record.schema_id.clone(),
            schema_version: state_record.schema_version,
            kind: state_record.kind,
            gate_ref: None,
        })
        .await
        .unwrap();

    assert_eq!(checkpoint.state_ref, state_record.state_ref);
    assert_eq!(checkpoint.schema_id, state_record.schema_id);
    assert_eq!(checkpoint.kind, state_record.kind);

    let loaded = checkpoint_store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap()
        .expect("checkpoint id should resolve to state ref");

    assert_eq!(loaded, checkpoint);
}

#[tokio::test]
async fn loop_checkpoint_store_handles_parallel_puts() {
    let state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let scope = turn_scope("thread-loop-checkpoint-parallel");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let write = |suffix: &'static str| {
        let state_store = Arc::clone(&state_store);
        let checkpoint_store = Arc::clone(&checkpoint_store);
        let scope = scope.clone();
        async move {
            let state_record = put_test_state(&state_store, scope.clone(), turn_id, run_id).await;
            checkpoint_store
                .put_loop_checkpoint(PutLoopCheckpointRequest {
                    scope,
                    turn_id,
                    run_id,
                    state_ref: state_record.state_ref,
                    schema_id: state_record.schema_id,
                    schema_version: state_record.schema_version,
                    kind: state_record.kind,
                    gate_ref: None,
                })
                .await
                .unwrap_or_else(|error| panic!("{suffix} checkpoint put failed: {error}"))
        }
    };

    let (first, second, third, fourth) = tokio::join!(
        write("first"),
        write("second"),
        write("third"),
        write("fourth")
    );

    let checkpoint_ids = [
        first.checkpoint_id,
        second.checkpoint_id,
        third.checkpoint_id,
        fourth.checkpoint_id,
    ];
    for checkpoint_id in checkpoint_ids {
        let loaded = checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: scope.clone(),
                turn_id,
                run_id,
                checkpoint_id,
            })
            .await
            .unwrap();
        assert!(loaded.is_some());
    }
}

#[tokio::test]
async fn turn_state_loop_checkpoint_store_survives_persistence_snapshot() {
    let state_store = InMemoryCheckpointStateStore::default();
    let checkpoint_store = InMemoryTurnStateStore::default();
    let scope = turn_scope("thread-loop-checkpoint-snapshot");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_record = put_test_state(&state_store, scope.clone(), turn_id, run_id).await;

    let checkpoint = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_record.state_ref.clone(),
            schema_id: state_record.schema_id.clone(),
            schema_version: state_record.schema_version,
            kind: state_record.kind,
            gate_ref: None,
        })
        .await
        .unwrap();

    let snapshot = checkpoint_store.persistence_snapshot();
    assert_eq!(snapshot.loop_checkpoints.len(), 1);

    let reopened = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap()
        .expect("turn-state-backed checkpoint id should survive snapshot reload");

    assert_eq!(loaded, checkpoint);
}

#[tokio::test]
async fn turn_state_loop_checkpoint_store_rejects_cross_scope_after_snapshot_reload() {
    let state_store = InMemoryCheckpointStateStore::default();
    let checkpoint_store = InMemoryTurnStateStore::default();
    let scope = turn_scope("thread-loop-checkpoint-snapshot-scope-a");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_record = put_test_state(&state_store, scope.clone(), turn_id, run_id).await;
    let checkpoint = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_record.state_ref,
            schema_id: state_record.schema_id,
            schema_version: state_record.schema_version,
            kind: state_record.kind,
            gate_ref: None,
        })
        .await
        .unwrap();

    let reopened = InMemoryTurnStateStore::from_persistence_snapshot(
        checkpoint_store.persistence_snapshot(),
        InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: turn_scope("thread-loop-checkpoint-snapshot-scope-b"),
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[tokio::test]
async fn loop_checkpoint_store_rejects_cross_run_checkpoint_id() {
    let state_store = InMemoryCheckpointStateStore::default();
    let checkpoint_store = InMemoryLoopCheckpointStore::default();
    let scope = turn_scope("thread-loop-checkpoint-cross-run");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_record = put_test_state(&state_store, scope.clone(), turn_id, run_id).await;
    let checkpoint = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_record.state_ref,
            schema_id: state_record.schema_id,
            schema_version: state_record.schema_version,
            kind: state_record.kind,
            gate_ref: None,
        })
        .await
        .unwrap();

    let loaded = checkpoint_store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id: TurnRunId::new(),
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[tokio::test]
async fn checkpoint_state_store_rejects_cross_scope_ref() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-scope-a");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = put_test_state(&store, scope.clone(), turn_id, run_id).await;

    let cross_scope = turn_scope("thread-checkpoint-scope-b");
    let loaded = store
        .get_checkpoint_state(get_request(&record, cross_scope, turn_id, run_id))
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[tokio::test]
async fn checkpoint_state_store_rejects_cross_run_ref() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-run");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = put_test_state(&store, scope.clone(), turn_id, run_id).await;

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, TurnRunId::new()))
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[tokio::test]
async fn checkpoint_state_store_rejects_schema_version_or_kind_mismatch() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-schema");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = put_test_state(&store, scope.clone(), turn_id, run_id).await;

    let mut wrong_schema = get_request(&record, scope.clone(), turn_id, run_id);
    wrong_schema.schema_id = CheckpointSchemaId::new("other_checkpoint_v1").unwrap();
    assert!(
        store
            .get_checkpoint_state(wrong_schema)
            .await
            .unwrap()
            .is_none()
    );

    let mut wrong_version = get_request(&record, scope.clone(), turn_id, run_id);
    wrong_version.schema_version = RunProfileVersion::new(2);
    assert!(
        store
            .get_checkpoint_state(wrong_version)
            .await
            .unwrap()
            .is_none()
    );

    let mut wrong_kind = get_request(&record, scope, turn_id, run_id);
    wrong_kind.kind = LoopCheckpointKind::BeforeModel;
    assert!(
        store
            .get_checkpoint_state(wrong_kind)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn checkpoint_state_store_rejects_oversized_payload() {
    let store = InMemoryCheckpointStateStore::default();
    let error = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            turn_scope("thread-checkpoint-oversized"),
            TurnId::new(),
            TurnRunId::new(),
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeSideEffect,
            vec![b'x'; MAX_CHECKPOINT_STATE_PAYLOAD_BYTES + 1],
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ironclaw_turns::TurnError::InvalidRequest { .. }
    ));
    assert!(!format!("{error:?}").contains("xxxx"));
}

#[test]
fn checkpoint_state_record_debug_redacts_payload() {
    let raw_payload =
        b"RAW_PROMPT_SENTINEL sk-secret /host/path tool_input provider_error".to_vec();
    let payload = RedactedCheckpointPayload::new(raw_payload.clone()).unwrap();
    let record = CheckpointStateRecord {
        state_ref: LoopCheckpointStateRef::new("checkpoint:debug-redaction").unwrap(),
        scope: turn_scope("thread-checkpoint-debug"),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
        schema_version: RunProfileVersion::new(1),
        kind: LoopCheckpointKind::BeforeModel,
        payload: payload.clone(),
        created_at: fixed_time(),
    };
    let request = PutCheckpointStateRequest::new(
        turn_scope("thread-checkpoint-debug-request"),
        TurnId::new(),
        TurnRunId::new(),
        CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
        RunProfileVersion::new(1),
        LoopCheckpointKind::BeforeModel,
        raw_payload,
    );
    let debug = format!("{payload:?}{record:?}{request:?}");

    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "sk-secret",
        "/host/path",
        "tool_input",
        "provider_error",
    ] {
        assert!(!debug.contains(forbidden), "debug leaked {forbidden}");
    }
    assert!(debug.contains("redacted"));
}

#[test]
fn redacted_checkpoint_payload_is_not_serializable() {
    static_assertions::assert_not_impl_any!(
        RedactedCheckpointPayload: serde::Serialize, serde::de::DeserializeOwned
    );
}

#[test]
fn turn_run_state_actor_is_serde_backward_compatible() {
    let actor = TurnActor::new(UserId::new("user-run-state-serde").unwrap());
    let state = TurnRunState {
        scope: turn_scope("thread-run-state-serde"),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("accepted-run-state-serde").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-run-state-serde").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-run-state-serde").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: fixed_time(),
        checkpoint_id: None,
        gate_ref: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: None,
        resume_disposition: None,
    };

    let legacy_wire = serde_json::to_value(&state).unwrap();
    assert!(
        !legacy_wire.as_object().unwrap().contains_key("actor"),
        "actor: None must stay omitted for legacy wire compatibility"
    );
    let decoded: TurnRunState = serde_json::from_value(legacy_wire).unwrap();
    assert!(decoded.actor.is_none());

    let decoded_with_actor: TurnRunState = serde_json::from_value(
        serde_json::to_value(TurnRunState {
            actor: Some(actor.clone()),
            ..state
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(decoded_with_actor.actor, Some(actor));
}

#[test]
fn turn_checkpoint_public_status_does_not_expose_checkpoint_payload() {
    let payload = b"RAW_CHECKPOINT_PAYLOAD sk-secret /host/path tool_input".to_vec();
    let payload = RedactedCheckpointPayload::new(payload).unwrap();
    let scope = turn_scope("thread-checkpoint-public-status");
    let checkpoint_id = TurnCheckpointId::new();
    let run_id = TurnRunId::new();

    let state = TurnRunState {
        scope: scope.clone(),
        actor: Some(TurnActor::new(
            UserId::new("user-checkpoint-public").unwrap(),
        )),
        turn_id: TurnId::new(),
        run_id,
        status: TurnStatus::BlockedApproval,
        accepted_message_ref: AcceptedMessageRef::new("accepted-checkpoint-public").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-checkpoint-public").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-checkpoint-public").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: fixed_time(),
        checkpoint_id: Some(checkpoint_id),
        gate_ref: Some(GateRef::new("gate-checkpoint-public").unwrap()),
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: None,
        resume_disposition: None,
    };
    let event = TurnLifecycleEvent {
        cursor: EventCursor(2),
        scope: scope.clone(),
        occurred_at: Some(fixed_time()),
        owner_user_id: Some(UserId::new("user-checkpoint-public").unwrap()),
        run_id,
        status: TurnStatus::BlockedApproval,
        kind: TurnEventKind::Blocked,
        blocked_gate: Some(ironclaw_turns::TurnBlockedGateMetadata {
            gate_ref: GateRef::new("gate-checkpoint-public").unwrap(),
            gate_kind: ironclaw_turns::TurnBlockedGateKind::Approval,
            credential_requirements: Vec::new(),
        }),
        sanitized_reason: Some("checkpointed".to_string()),
    };
    let snapshot = TurnPersistenceSnapshot {
        checkpoints: vec![TurnCheckpointRecord {
            checkpoint_id,
            run_id,
            scope: Some(scope.clone()),
            sequence: 1,
            status: TurnStatus::BlockedApproval,
            gate_ref: GateRef::new("gate-checkpoint-public").unwrap(),
            kind: LoopCheckpointKind::BeforeBlock,
            state_ref: LoopCheckpointStateRef::new("checkpoint:public-status").unwrap(),
            created_at: fixed_time(),
        }],
        events: vec![event.clone()],
        ..TurnPersistenceSnapshot::default()
    };

    let public_wire = format!(
        "{}{}{}{:?}",
        serde_json::to_string(&state).unwrap(),
        serde_json::to_string(&event).unwrap(),
        serde_json::to_string(&snapshot).unwrap(),
        payload,
    );

    for forbidden in [
        "RAW_CHECKPOINT_PAYLOAD",
        "sk-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(
            !public_wire.contains(forbidden),
            "public checkpoint/status surface leaked {forbidden}"
        );
    }
}

#[tokio::test]
async fn checkpoint_state_store_round_trips_empty_payload() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-empty");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();

    let record = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope.clone(),
            turn_id,
            run_id,
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeModel,
            Vec::<u8>::new(),
        ))
        .await
        .unwrap();

    assert!(record.payload.is_empty());

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("empty payload should round-trip");

    assert_eq!(loaded.payload.as_bytes(), &[] as &[u8]);
}

#[tokio::test]
async fn checkpoint_state_store_accepts_exact_max_size_payload() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-max-size");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let payload = vec![b'A'; MAX_CHECKPOINT_STATE_PAYLOAD_BYTES];

    let record = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope.clone(),
            turn_id,
            run_id,
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeSideEffect,
            payload.clone(),
        ))
        .await
        .unwrap();

    assert_eq!(record.payload.len(), MAX_CHECKPOINT_STATE_PAYLOAD_BYTES);

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("exact max-size payload should round-trip");

    assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
}

#[tokio::test]
async fn checkpoint_state_store_multiple_puts_produce_distinct_refs() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-multi");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();

    let payload = b"same".to_vec();

    let record_a = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope.clone(),
            turn_id,
            run_id,
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeModel,
            payload.clone(),
        ))
        .await
        .unwrap();

    let record_b = store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope.clone(),
            turn_id,
            run_id,
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeModel,
            payload.clone(),
        ))
        .await
        .unwrap();

    assert_ne!(
        record_a.state_ref, record_b.state_ref,
        "each put must produce a unique state_ref"
    );

    let loaded_a = store
        .get_checkpoint_state(get_request(&record_a, scope.clone(), turn_id, run_id))
        .await
        .unwrap()
        .expect("first record should be independently retrievable");
    assert_eq!(loaded_a.payload.as_bytes(), payload.as_slice());

    let loaded_b = store
        .get_checkpoint_state(get_request(&record_b, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("second record should be independently retrievable");
    assert_eq!(loaded_b.payload.as_bytes(), payload.as_slice());
}

#[tokio::test]
async fn checkpoint_state_store_rejects_cross_turn_id_ref() {
    let store = InMemoryCheckpointStateStore::default();
    let scope = turn_scope("thread-checkpoint-cross-turn");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = put_test_state(&store, scope.clone(), turn_id, run_id).await;

    let different_turn_id = TurnId::new();
    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, different_turn_id, run_id))
        .await
        .unwrap();

    assert!(
        loaded.is_none(),
        "checkpoint state must not be returned for a different turn_id"
    );

    let loaded = store
        .get_checkpoint_state(get_request(
            &record,
            turn_scope("thread-checkpoint-cross-all"),
            TurnId::new(),
            TurnRunId::new(),
        ))
        .await
        .unwrap();

    assert!(
        loaded.is_none(),
        "checkpoint state must not be returned when scope, turn_id, and run_id differ"
    );
}

fn get_request(
    record: &CheckpointStateRecord,
    scope: TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
) -> GetCheckpointStateRequest {
    GetCheckpointStateRequest {
        scope,
        turn_id,
        run_id,
        state_ref: record.state_ref.clone(),
        schema_id: record.schema_id.clone(),
        schema_version: record.schema_version,
        kind: record.kind,
    }
}

async fn put_test_state(
    store: &InMemoryCheckpointStateStore,
    scope: TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
) -> CheckpointStateRecord {
    store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            scope,
            turn_id,
            run_id,
            CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::BeforeBlock,
            b"checkpoint-state".to_vec(),
        ))
        .await
        .unwrap()
}

#[test]
fn turn_persistence_snapshot_legacy_run_defaults_resume_disposition_to_none() {
    // Simulate a legacy TurnPersistenceSnapshot JSON where TurnRunRecord objects were
    // persisted before resume_disposition (stored under "auth_resume_disposition") was
    // added. Deserializing such a snapshot must succeed and the field must default to
    // None (via #[serde(default)]).
    //
    // Because resume_disposition uses skip_serializing_if = "Option::is_none",
    // serializing a record with None naturally produces JSON without the key —
    // which is exactly what a legacy snapshot looks like.
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let scope = turn_scope("thread-legacy-auth-resume");

    // Build a minimal ResolvedRunProfile so we can construct a TurnRunProfile.
    let descriptor = AgentLoopDriverDescriptor {
        id: LoopDriverId::new("legacy-auth-resume-driver").expect("valid driver id"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(
            CheckpointSchemaId::new("legacy_auth_resume_checkpoint").expect("valid"),
        ),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let resolved = ResolvedRunProfile {
        run_class_id: RunClassId::new("legacy-auth-resume-class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor
            .checkpoint_schema_id
            .clone()
            .expect("descriptor checkpoint id"),
        checkpoint_schema_version: descriptor
            .checkpoint_schema_version
            .expect("descriptor checkpoint version"),
        model_profile_id: ModelProfileId::new("legacy-auth-resume-model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new(
            "legacy-auth-resume-capabilities",
        )
        .expect("valid"),
        context_profile_id: ContextProfileId::new("legacy-auth-resume-context").expect("valid"),
        steering_policy: SteeringPolicy {
            allow_steering: false,
            allow_interrupt: false,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: false,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: false,
            require_before_block: false,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("legacy-auth-resume-tier").expect("valid"),
            max_model_calls: 16,
            max_capability_invocations: 32,
        },
        personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive").expect("valid"),
        concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
        resolution_fingerprint: RunProfileFingerprint::new("legacy-auth-resume-fingerprint")
            .expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    };

    // Build a minimal TurnRunRecord to serialize. We use serde_json directly to
    // confirm the key is absent (proving the legacy simulation is correct) then
    // deserialize the full snapshot.
    let record = TurnRunRecord {
        run_id,
        turn_id,
        scope: scope.clone(),
        accepted_message_ref: AcceptedMessageRef::new("accepted-legacy").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-legacy").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-legacy").unwrap(),
        status: TurnStatus::Completed,
        profile: TurnRunProfile::from_resolved(resolved),
        resolved_model_route: None,
        checkpoint_id: None,
        gate_ref: None,
        credential_requirements: vec![],
        failure: None,
        event_cursor: EventCursor(0),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: fixed_time(),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        resume_disposition: None,
    };

    // Serialize the snapshot — auth_resume_disposition (the wire key) must be absent
    // in the output when resume_disposition is None.
    let snapshot = TurnPersistenceSnapshot {
        runs: vec![record],
        ..TurnPersistenceSnapshot::default()
    };
    let mut json_val = serde_json::to_value(&snapshot).expect("serialize snapshot");

    // Verify the key is indeed absent (proving our legacy simulation is accurate).
    let run_obj = json_val["runs"][0]
        .as_object_mut()
        .expect("runs[0] must be an object");
    assert!(
        !run_obj.contains_key("auth_resume_disposition"),
        "auth_resume_disposition must be absent in serialized JSON when None (skip_serializing_if)"
    );

    // Now forcibly remove the key (belt-and-suspenders for any future serializer change)
    // and deserialize the full snapshot.
    run_obj.remove("auth_resume_disposition");
    let deserialized: TurnPersistenceSnapshot =
        serde_json::from_value(json_val).expect("deserialize legacy snapshot");

    assert_eq!(
        deserialized.runs.len(),
        1,
        "snapshot must contain exactly one run"
    );
    assert_eq!(
        deserialized.runs[0].resume_disposition, None,
        "resume_disposition must default to None when absent in legacy snapshot"
    );
}

#[test]
fn turn_persistence_snapshot_legacy_run_preserves_denied_resume_disposition() {
    // Mirror of turn_persistence_snapshot_legacy_run_defaults_resume_disposition_to_none,
    // but instead of an absent key we inject the LEGACY wire key
    // "auth_resume_disposition": "denied" into the serialized run JSON.
    // This proves the serde rename provides back-compat for persisted records
    // that carried a non-None value under the old field name.
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let scope = turn_scope("thread-legacy-denied-resume");

    let descriptor = AgentLoopDriverDescriptor {
        id: LoopDriverId::new("legacy-denied-resume-driver").expect("valid driver id"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(
            CheckpointSchemaId::new("legacy_denied_resume_checkpoint").expect("valid"),
        ),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let resolved = ResolvedRunProfile {
        run_class_id: RunClassId::new("legacy-denied-resume-class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor
            .checkpoint_schema_id
            .clone()
            .expect("descriptor checkpoint id"),
        checkpoint_schema_version: descriptor
            .checkpoint_schema_version
            .expect("descriptor checkpoint version"),
        model_profile_id: ModelProfileId::new("legacy-denied-resume-model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new(
            "legacy-denied-resume-capabilities",
        )
        .expect("valid"),
        context_profile_id: ContextProfileId::new("legacy-denied-resume-context").expect("valid"),
        steering_policy: SteeringPolicy {
            allow_steering: false,
            allow_interrupt: false,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: false,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: false,
            require_before_block: false,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("legacy-denied-resume-tier").expect("valid"),
            max_model_calls: 16,
            max_capability_invocations: 32,
        },
        personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive").expect("valid"),
        concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
        resolution_fingerprint: RunProfileFingerprint::new("legacy-denied-resume-fingerprint")
            .expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    };

    // Build a record with resume_disposition = None so the key is absent in
    // serialized form (skip_serializing_if = "Option::is_none"), then inject
    // the legacy wire key before deserializing.
    let record = TurnRunRecord {
        run_id,
        turn_id,
        scope: scope.clone(),
        accepted_message_ref: AcceptedMessageRef::new("accepted-legacy-denied").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-legacy-denied").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-legacy-denied").unwrap(),
        status: TurnStatus::Completed,
        profile: TurnRunProfile::from_resolved(resolved),
        resolved_model_route: None,
        checkpoint_id: None,
        gate_ref: None,
        credential_requirements: vec![],
        failure: None,
        event_cursor: EventCursor(0),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: fixed_time(),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        resume_disposition: None,
    };

    let snapshot = TurnPersistenceSnapshot {
        runs: vec![record],
        ..TurnPersistenceSnapshot::default()
    };
    let mut json_val = serde_json::to_value(&snapshot).expect("serialize snapshot");

    // Inject the legacy key into the run object to simulate a persisted record
    // that carried auth_resume_disposition = "denied" under the old field name.
    let run_obj = json_val["runs"][0]
        .as_object_mut()
        .expect("runs[0] must be an object");
    assert!(
        !run_obj.contains_key("auth_resume_disposition"),
        "auth_resume_disposition must be absent before injection"
    );
    run_obj.insert(
        "auth_resume_disposition".to_string(),
        serde_json::json!("denied"),
    );

    let deserialized: TurnPersistenceSnapshot =
        serde_json::from_value(json_val).expect("deserialize legacy snapshot with denied key");

    assert_eq!(
        deserialized.runs.len(),
        1,
        "snapshot must contain exactly one run"
    );
    assert_eq!(
        deserialized.runs[0].resume_disposition,
        Some(ironclaw_turns::GateResumeDisposition::Denied),
        "resume_disposition must be Some(Denied) when legacy auth_resume_disposition key is present"
    );
}

fn turn_scope(thread_id: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-checkpoint").unwrap(),
        Some(AgentId::new("agent-checkpoint").unwrap()),
        Some(ProjectId::new("project-checkpoint").unwrap()),
        ThreadId::new(thread_id).unwrap(),
    )
}

fn fixed_time() -> TurnTimestamp {
    chrono::DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}
