use chrono::Utc;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::ids::{AgentId, CapabilityId, ProcessId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{
    LoopMessageRef, LoopResultRef, TurnActor, TurnGateRef, TurnRunId, TurnScope,
};
use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
use ironclaw_processes::{
    OpenProcessDependencyRequest, ProcessDependencyRecord, ProcessDependencyState,
    ProcessJournalStore, ProcessKind, ProcessSubmissionPort, SubmitProcessRequest,
    in_memory_backed_process_store,
};
use uuid::Uuid;

use super::*;

fn edge_fixture() -> (TurnRunId, TurnRunId, AwaitEdge) {
    let tenant_id = TenantId::new("store-transition-tenant").expect("tenant");
    let user_id = UserId::new("store-transition-user").expect("user");
    let agent_id = AgentId::new("store-transition-agent").expect("agent");
    let parent_thread_id = ThreadId::new("store-transition-parent-thread").expect("parent thread");
    let child_thread_id = ThreadId::new("store-transition-child-thread").expect("child thread");
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();
    let parent_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        parent_thread_id.clone(),
        Some(user_id.clone()),
    );
    let child_scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        None,
        child_thread_id.clone(),
        Some(user_id.clone()),
    );
    let mut parent_run_context =
        ironclaw_agent_loop::test_support::test_run_context("await-store-transition");
    parent_run_context.scope = parent_scope;
    parent_run_context.thread_id = parent_thread_id.clone();
    parent_run_context.run_id = parent_run_id;
    parent_run_context.actor = Some(TurnActor::new(user_id));
    (
        parent_run_id,
        child_run_id,
        AwaitEdge {
            child_scope,
            child_thread_id,
            parent_thread_id,
            parent_run_context,
            tree_root_run_id: parent_run_id,
            gate_ref: TurnGateRef::new("gate:store-transition").expect("gate"),
            subagent_kind: SubagentKindId::new("general").expect("kind"),
            spawn_capability_id: CapabilityId::new(
                ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .expect("capability"),
            spawn_provider_call_id: Some("spawn-call-store-transition".to_string()),
            result_ref: LoopResultRef::new("result:store-transition").expect("result"),
            mode: SpawnSubagentMode::Blocking,
            state: AwaitEdgeState::Open,
            terminal_kind: None,
            terminal_byte_len: None,
            terminal_reason: None,
            reservation_release: ReservationReleaseState::Unclaimed,
            appended_message_ref: None,
            attention_outcome: None,
            created_at: Utc::now(),
            settled_at: None,
        },
    )
}

/// The blob every production edge is actually opened with
/// (`subagent_spawn_port.rs`'s `AwaitedChildSetRecord`) — never the
/// `AwaitEdge` shape itself. Shared by every fixture that opens a
/// dependency through the real port, so the tests exercise the shape
/// production writes instead of one only `serde_json::to_value(&edge)`
/// happens to round-trip through.
fn awaited_child_set_record(child_run_id: TurnRunId, edge: &AwaitEdge) -> AwaitedChildSetRecord {
    AwaitedChildSetRecord {
        gate_ref: edge.gate_ref.clone(),
        parent_run_context: edge.parent_run_context.clone(),
        tree_root_run_id: edge.tree_root_run_id,
        child_scope: edge.child_scope.clone(),
        child_run_id,
        child_thread_id: edge.child_thread_id.clone(),
        subagent_kind: edge.subagent_kind.clone(),
        spawn_capability_id: edge.spawn_capability_id.clone(),
        spawn_provider_call_id: edge.spawn_provider_call_id.clone(),
        result_ref: edge.result_ref.clone(),
        mode: edge.mode,
    }
}

fn record(
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
    edge: &AwaitEdge,
    state: ProcessDependencyState,
    status: ProcessLifecycleStatus,
) -> ProcessDependencyRecord {
    ProcessDependencyRecord {
        dependent_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
        dependency_process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
        root_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
        scope: edge.child_scope.to_resource_scope(),
        group_ref: Some(edge.gate_ref.as_str().to_string()),
        state,
        terminal: Some(ProcessTerminalEvidence {
            status,
            output_bytes: Some(42),
            sanitized_reason: Some("terminal-reason".to_string()),
        }),
        created_at: edge.created_at,
        settled_at: Some(Utc::now()),
        consumed_at: None,
        transitioned_at: None,
        metadata: serde_json::to_value(awaited_child_set_record(child_run_id, edge))
            .expect("serialize the production blob"),
    }
}

#[test]
fn edge_projection_matrix_covers_every_dependency_and_terminal_state() {
    let (parent_run_id, child_run_id, edge) = edge_fixture();
    let cases = [
        (
            ProcessDependencyState::Open,
            AwaitEdgeState::Open,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Completed,
            Some(EdgeTerminalKind::Completed),
        ),
        (
            ProcessDependencyState::Settled,
            AwaitEdgeState::Settled,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Failed,
            Some(EdgeTerminalKind::Failed),
        ),
        // Each kernel delivery substate has its own loop-tier arm, and none
        // of them may look released: they are all still in flight.
        (
            ProcessDependencyState::ResultAppended,
            AwaitEdgeState::ResultAppended,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Completed,
            Some(EdgeTerminalKind::Completed),
        ),
        (
            ProcessDependencyState::AttentionScheduled,
            AwaitEdgeState::AttentionScheduled,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Completed,
            Some(EdgeTerminalKind::Completed),
        ),
        (
            ProcessDependencyState::AttentionDeferred,
            AwaitEdgeState::AttentionDeferredStreakCap,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Completed,
            Some(EdgeTerminalKind::Completed),
        ),
        (
            ProcessDependencyState::Consumed,
            AwaitEdgeState::Drained,
            ReservationReleaseState::Released,
            ProcessLifecycleStatus::Cancelled,
            Some(EdgeTerminalKind::Cancelled),
        ),
        (
            ProcessDependencyState::Abandoned,
            AwaitEdgeState::Abandoned,
            ReservationReleaseState::Released,
            ProcessLifecycleStatus::RecoveryRequired,
            Some(EdgeTerminalKind::RecoveryRequired),
        ),
        (
            ProcessDependencyState::Open,
            AwaitEdgeState::Open,
            ReservationReleaseState::Unclaimed,
            ProcessLifecycleStatus::Running,
            None,
        ),
    ];

    for (state, expected_state, expected_release, terminal, expected_terminal) in cases {
        let projected = AwaitEdgeStore::edge_from_record(record(
            parent_run_id,
            child_run_id,
            &edge,
            state,
            terminal,
        ))
        .expect("project edge");
        assert_eq!(projected.state, expected_state);
        assert_eq!(projected.reservation_release, expected_release);
        assert_eq!(projected.terminal_kind, expected_terminal);
        assert_eq!(projected.terminal_byte_len, Some(42));
        assert_eq!(
            projected.terminal_reason.as_deref(),
            Some("terminal-reason")
        );
    }
}

/// The projection decodes exactly one shape — the `AwaitedChildSetRecord`
/// blob production opens every edge with — and everything else fails closed.
/// A serialized `AwaitEdge` is deliberately *not* accepted: nothing writes
/// one, and accepting it was how the merged delivery keys got defaulted away
/// (commit 80663bea7).
#[test]
fn only_the_production_metadata_blob_decodes_and_anything_else_fails_closed() {
    let (parent_run_id, child_run_id, edge) = edge_fixture();
    let projected = AwaitEdgeStore::edge_from_record(record(
        parent_run_id,
        child_run_id,
        &edge,
        ProcessDependencyState::Open,
        ProcessLifecycleStatus::Completed,
    ))
    .expect("project the production blob");
    assert_eq!(projected.state, AwaitEdgeState::Open);
    assert_eq!(projected.gate_ref, edge.gate_ref);

    let mut malformed = record(
        parent_run_id,
        child_run_id,
        &edge,
        ProcessDependencyState::Open,
        ProcessLifecycleStatus::Completed,
    );
    malformed.metadata = serde_json::json!({"unexpected": true});
    assert!(AwaitEdgeStore::edge_from_record(malformed).is_err());

    let mut serialized_edge = record(
        parent_run_id,
        child_run_id,
        &edge,
        ProcessDependencyState::Open,
        ProcessLifecycleStatus::Completed,
    );
    serialized_edge.metadata = serde_json::to_value(&edge).expect("serialize edge");
    assert!(
        AwaitEdgeStore::edge_from_record(serialized_edge).is_err(),
        "the two shapes are disjoint: an `AwaitEdge` blob is not a metadata shape production \
         ever writes, so it must not decode"
    );

    // A blob that still parses as `AwaitedChildSetRecord` but carries junk
    // under one of the two keys the delivery chain merges in. Recovering
    // these is the whole point of reading them off the raw blob, so a junk
    // value must refuse — never silently default the key away — and must name
    // the key it choked on.
    for key in ["appended_message_ref", "attention_outcome"] {
        let mut poisoned = record(
            parent_run_id,
            child_run_id,
            &edge,
            ProcessDependencyState::ResultAppended,
            ProcessLifecycleStatus::Completed,
        );
        let mut blob = serde_json::to_value(awaited_child_set_record(child_run_id, &edge))
            .expect("serialize submitted record");
        blob[key] = serde_json::json!({"not": "a valid value"});
        poisoned.metadata = blob;

        let error = AwaitEdgeStore::edge_from_record(poisoned)
            .expect_err("a malformed merged delivery key must fail closed");
        let AwaitEdgeStoreError::Backend { reason } = &error else {
            panic!("edge_from_record only ever produces Backend, got: {error:?}");
        };
        assert!(
            reason.contains(key),
            "the refusal must name the offending key, got: {reason}"
        );
    }
}

/// The projection store together with the journal it projects over. A
/// fixture seeds the kernel side through the real port; nothing here
/// writes stored dependency state through a back door.
fn new_store() -> (AwaitEdgeStore, Arc<ProcessJournalStore<InMemoryBackend>>) {
    let journal = Arc::new(in_memory_backed_process_store());
    let dependencies =
        Arc::clone(&journal) as Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>;
    (AwaitEdgeStore::new(dependencies), journal)
}

/// Opens one background-mode await edge in the journal and settles it
/// through `AwaitEdgeStore::settle`, leaving it exactly where the delivery
/// chain starts.
async fn settled_background_edge(
    store: &AwaitEdgeStore,
    journal: &ProcessJournalStore<InMemoryBackend>,
) -> (TurnScope, TurnRunId, TurnRunId) {
    let (parent_run_id, child_run_id, mut edge) = edge_fixture();
    edge.mode = SpawnSubagentMode::Background;
    let scope = edge.child_scope.clone();
    let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());
    journal
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: edge.parent_run_context.scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: None,
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit parent process");
    journal
        .open_process_dependency(OpenProcessDependencyRequest {
            dependent_process_id: parent_process_id,
            dependency_process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
            root_process_id: parent_process_id,
            scope: scope.to_resource_scope(),
            group_ref: Some(format!("bg:{}", scope.thread_id)),
            created_at: edge.created_at,
            metadata: serde_json::to_value(awaited_child_set_record(child_run_id, &edge))
                .expect("serialize submitted record"),
        })
        .await
        .expect("open dependency");
    let settled = store
        .settle(
            &scope,
            parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(17),
            None,
        )
        .await
        .expect("settle edge")
        .expect("edge exists");
    assert_eq!(settled.state, AwaitEdgeState::Settled);
    (scope, parent_run_id, child_run_id)
}

#[tokio::test]
async fn background_sweep_query_skips_open_rows_and_honors_human_deferred_state() {
    let (store, journal) = new_store();
    let (scope, parent, settled_child) = settled_background_edge(&store, &journal).await;
    let (_, _, mut edge) = edge_fixture();
    edge.mode = SpawnSubagentMode::Background;
    let parent_process_id = ProcessId::from_uuid(parent.as_uuid());

    // More open rows than the sweep batch limit would otherwise return. The
    // state-prefix query must reach the settled row without consuming these
    // historical opens first.
    for value in 1..=33_u128 {
        let child = ProcessId::from_uuid(Uuid::from_u128(value));
        journal
            .open_process_dependency(OpenProcessDependencyRequest {
                dependent_process_id: parent_process_id,
                dependency_process_id: child,
                root_process_id: parent_process_id,
                scope: scope.to_resource_scope(),
                group_ref: Some(format!("bg:{}", scope.thread_id)),
                created_at: Utc::now(),
                metadata: serde_json::to_value(awaited_child_set_record(
                    TurnRunId::from_uuid(child.as_uuid()),
                    &edge,
                ))
                .expect("serialize open background edge"),
            })
            .await
            .expect("open background edge");
    }
    let actionable = store
        .list_background_for_thread(&scope, 1, false)
        .await
        .expect("list actionable background edges");
    assert_eq!(actionable.len(), 1);
    assert_eq!(
        actionable[0].1,
        TurnRunId::from_uuid(settled_child.as_uuid())
    );

    store
        .record_result_appended(
            &scope,
            parent,
            settled_child,
            LoopMessageRef::new("msg:deferred").expect("message ref"),
        )
        .await
        .expect("append result")
        .expect("settled edge exists");
    store
        .defer_streak_capped(&scope, parent, settled_child)
        .await
        .expect("defer result")
        .expect("appended edge exists");
    assert!(
        store
            .list_background_for_thread(&scope, 1, false)
            .await
            .expect("autonomous sweep list")
            .is_empty(),
        "autonomous starts must leave deferred attention parked"
    );
    assert_eq!(
        store
            .list_background_for_thread(&scope, 1, true)
            .await
            .expect("human sweep list")
            .len(),
        1,
        "human starts may retry deferred attention"
    );
}

#[tokio::test]
async fn edge_walks_the_full_background_delivery_lifecycle() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;

    let appended = store
        .record_result_appended(
            &scope,
            parent,
            child,
            LoopMessageRef::new("msg:child-1").expect("valid ref"),
        )
        .await
        .expect("append recorded")
        .expect("edge exists");
    assert_eq!(appended.state, AwaitEdgeState::ResultAppended);
    assert_eq!(
        appended.appended_message_ref.as_ref().map(|r| r.as_str()),
        Some("msg:child-1")
    );
    assert_eq!(
        appended.reservation_release,
        ReservationReleaseState::Unclaimed,
        "an in-flight delivery step must not look like a released reservation"
    );

    let attended = store
        .record_attention(&scope, parent, child, AttentionOutcome::Queued)
        .await
        .expect("attention recorded")
        .expect("edge exists");
    assert_eq!(attended.state, AwaitEdgeState::AttentionScheduled);
    assert_eq!(attended.attention_outcome, Some(AttentionOutcome::Queued));
    assert_eq!(
        attended.appended_message_ref.as_ref().map(|r| r.as_str()),
        Some("msg:child-1"),
        "a later delivery step must merge into the edge, never replace it"
    );

    // The whole walk is durable, not an in-memory artifact of the caller.
    let reread = store
        .peek(&scope, parent, child)
        .await
        .expect("peek edge")
        .expect("edge exists");
    assert_eq!(reread.state, AwaitEdgeState::AttentionScheduled);
    assert_eq!(reread.attention_outcome, Some(AttentionOutcome::Queued));
    assert_eq!(reread.terminal_kind, Some(EdgeTerminalKind::Completed));
}

#[tokio::test]
async fn the_delivery_chain_refuses_to_skip_the_append() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;

    // Attention before the result is durably appended would make the parent
    // attentive to nothing. `Settled` is not a legal predecessor of
    // `AttentionScheduled`, so the kernel CAS refuses it no matter what this
    // caller asks for — the guard lives there, not in this caller's choice of
    // arguments — and the store propagates that refusal with the cause intact.
    let error = store
        .record_attention(&scope, parent, child, AttentionOutcome::Activated)
        .await
        .expect_err("attention before the append must be refused");
    let AwaitEdgeStoreError::Backend { reason } = &error else {
        panic!("record_attention only ever produces Backend, got: {error:?}");
    };
    assert!(
        reason.contains("AttentionScheduled")
            && reason.contains("ResultAppended")
            && reason.contains("Settled"),
        "refusal must carry the kernel's cause, got: {reason}"
    );

    assert_eq!(
        store
            .peek(&scope, parent, child)
            .await
            .expect("peek edge")
            .expect("edge exists")
            .state,
        AwaitEdgeState::Settled,
        "a refused transition must leave the edge exactly where it was"
    );
    assert_eq!(
        store
            .peek(&scope, parent, child)
            .await
            .expect("peek edge")
            .expect("edge exists")
            .attention_outcome,
        None,
        "a refused transition must not record its payload either"
    );
}

#[tokio::test]
async fn recording_an_append_twice_keeps_the_first_ref() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;
    let first = LoopMessageRef::new("msg:first").expect("valid ref");
    let second = LoopMessageRef::new("msg:second").expect("valid ref");

    store
        .record_result_appended(&scope, parent, child, first)
        .await
        .expect("first append")
        .expect("edge exists");
    let replay = store
        .record_result_appended(&scope, parent, child, second)
        .await
        .expect("replay")
        .expect("edge exists");

    assert_eq!(
        replay.appended_message_ref.as_ref().map(|r| r.as_str()),
        Some("msg:first"),
        "replay must return the ref already durably recorded"
    );
}

/// `close` must refuse a `ResultAppended` edge outright: the result is
/// durably appended to the parent thread but the parent has not yet been
/// made attentive to it, so closing here would strand the result and hold
/// the descendant reservation forever.
#[tokio::test]
async fn close_refuses_an_edge_with_an_undelivered_appended_result() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;
    store
        .record_result_appended(
            &scope,
            parent,
            child,
            LoopMessageRef::new("msg:child-1").expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");

    let error = store
        .close(&scope, parent, child)
        .await
        .expect_err("close must refuse an edge holding an undelivered result");
    assert_eq!(
        error,
        AwaitEdgeStoreError::UndeliveredResult {
            state: AwaitEdgeState::ResultAppended,
        }
    );
    assert_eq!(
        store
            .peek(&scope, parent, child)
            .await
            .expect("peek edge")
            .expect("edge exists")
            .state,
        AwaitEdgeState::ResultAppended,
        "a refused close must leave the edge exactly where it was"
    );
}

/// The deferred branch end to end: parked, un-closeable while parked,
/// drained forward by a later attention sweep, and only then closeable.
/// A state you can enter but not leave would be a trap for the slice that
/// builds the sweep.
#[tokio::test]
async fn a_streak_capped_edge_parks_unclosed_until_attention_drains_it() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;
    store
        .record_result_appended(
            &scope,
            parent,
            child,
            LoopMessageRef::new("msg:child-1").expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");

    let deferred = store
        .defer_streak_capped(&scope, parent, child)
        .await
        .expect("deferral recorded")
        .expect("edge exists");
    assert_eq!(deferred.state, AwaitEdgeState::AttentionDeferredStreakCap);

    let unclosed = store.list_unclosed_for_scope(&scope).await.expect("query");
    assert_eq!(unclosed.len(), 1, "a deferred edge must remain claimable");

    // `close` must refuse a parked edge: the kernel refuses to consume one,
    // because closing it would strand the undelivered result.
    let close_error = store
        .close(&scope, parent, child)
        .await
        .expect_err("close must refuse a parked edge");
    assert_eq!(
        close_error,
        AwaitEdgeStoreError::UndeliveredResult {
            state: AwaitEdgeState::AttentionDeferredStreakCap,
        }
    );
    assert_eq!(
        store
            .peek(&scope, parent, child)
            .await
            .expect("peek edge")
            .expect("edge exists")
            .state,
        AwaitEdgeState::AttentionDeferredStreakCap
    );

    // The sweep that drains the park: `AttentionDeferred -> AttentionScheduled`.
    let drained = store
        .record_attention(&scope, parent, child, AttentionOutcome::Activated)
        .await
        .expect("deferred edge drains forward")
        .expect("edge exists");
    assert_eq!(drained.state, AwaitEdgeState::AttentionScheduled);
    assert_eq!(drained.attention_outcome, Some(AttentionOutcome::Activated));
    assert_eq!(
        drained.appended_message_ref.as_ref().map(|r| r.as_str()),
        Some("msg:child-1"),
        "draining the park must not lose the already-appended result"
    );

    // Only now is it closeable — and closing it releases the reservation.
    store
        .close(&scope, parent, child)
        .await
        .expect("a drained edge closes");
    assert!(
        store
            .peek(&scope, parent, child)
            .await
            .expect("peek closed edge")
            .is_none(),
        "closing must consume the edge, not leave it claimable"
    );
    assert!(
        journal
            .unresolved_process_dependencies()
            .await
            .expect("unresolved dependencies")
            .is_empty(),
        "the deferred branch must end with no dependency left unresolved"
    );
}

/// Recording attention on an edge already standing on `AttentionScheduled`
/// is a replay, not an advance: the kernel's idempotency rule returns the
/// stored record untouched, so the first outcome wins.
#[tokio::test]
async fn recording_attention_twice_keeps_the_first_outcome() {
    let (store, journal) = new_store();
    let (scope, parent, child) = settled_background_edge(&store, &journal).await;
    store
        .record_result_appended(
            &scope,
            parent,
            child,
            LoopMessageRef::new("msg:child-1").expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    store
        .record_attention(&scope, parent, child, AttentionOutcome::Queued)
        .await
        .expect("first attention")
        .expect("edge exists");

    let replay = store
        .record_attention(&scope, parent, child, AttentionOutcome::Activated)
        .await
        .expect("replay")
        .expect("edge exists");
    assert_eq!(replay.state, AwaitEdgeState::AttentionScheduled);
    assert_eq!(
        replay.attention_outcome,
        Some(AttentionOutcome::Queued),
        "replay must return the outcome already durably recorded"
    );
}
