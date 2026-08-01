use super::*;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_loop_contracts::LoopInput;
use ironclaw_threads::ThreadScope;
use ironclaw_turns::TurnId;

use crate::InMemoryHostInputQueue;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageStatus, ThreadHistoryRequest,
};
use ironclaw_turns::{LoopMessageRef, TurnScope};

fn make_fs(backend: Arc<InMemoryBackend>) -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").unwrap(),
        VirtualPath::new("/turns").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn owner_scope() -> ResourceScope {
    TurnScope::new(
        TenantId::new("tenant-iq").unwrap(),
        Some(AgentId::new("agent-iq").unwrap()),
        Some(ProjectId::new("project-iq").unwrap()),
        ThreadId::new("thread-iq").unwrap(),
    )
    .to_resource_scope()
}

// ThreadScope carries no thread_id (the thread is addressed separately), so
// one fixed scope serves both the real-message and ghost-message tests.
fn ghost_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant-iq").unwrap(),
        agent_id: AgentId::new("agent-iq").unwrap(),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    }
}

fn steering(message_ref: &str) -> LoopInput {
    LoopInput::Steering {
        message_ref: LoopMessageRef::new(message_ref).unwrap(),
    }
}

fn origin() -> LoopInputCursorToken {
    LoopInputCursorToken::new("input-cursor:origin".to_string()).unwrap()
}

/// The two production backends behind one conformance bound: both shells wrap
/// the same `RunQueueModel`, and these parameterized tests prove the shells
/// preserve its behavior identically (the dual-backend parity rule in
/// `.claude/rules/database.md`).
trait ConformanceQueue: HostInputQueue + HostInputEnqueuePort + HostInputQueueReconcile {}
impl<Q: HostInputQueue + HostInputEnqueuePort + HostInputQueueReconcile> ConformanceQueue for Q {}

fn durable_queue(
    thread_service: Arc<InMemorySessionThreadService>,
) -> FilesystemHostInputQueue<InMemoryBackend> {
    FilesystemHostInputQueue::new(
        make_fs(Arc::new(InMemoryBackend::new())),
        owner_scope(),
        thread_service as Arc<dyn SessionThreadService>,
    )
}

fn in_memory_queue(thread_service: Arc<InMemorySessionThreadService>) -> InMemoryHostInputQueue {
    InMemoryHostInputQueue::new(thread_service as Arc<dyn SessionThreadService>)
}

#[tokio::test]
async fn durable_queue_survives_store_reconstruction() {
    // The core durability guarantee: a message queued before a restart is
    // still drainable after, and the reconstructed cursor/ack tokens match
    // the ones the loop's persisted input cursor references.
    let backend = Arc::new(InMemoryBackend::new());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let run_id = TurnRunId::new();
    let input = steering("msg:restart");

    // First "process": enqueue, then drop the queue object.
    let envelope = {
        let queue = FilesystemHostInputQueue::new(
            make_fs(Arc::clone(&backend)),
            owner_scope(),
            Arc::clone(&thread_service),
        );
        queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: ghost_scope(),
                thread_id: ThreadId::new("ghost").unwrap(),
                message_id: ThreadMessageId::new(),
                input: input.clone(),
            })
            .await
            .expect("enqueue")
    };

    // Second "process" (restart): a brand-new queue object over the SAME
    // durable backend must surface the queued input.
    let queue =
        FilesystemHostInputQueue::new(make_fs(Arc::clone(&backend)), owner_scope(), thread_service);
    let batch = queue
        .next_after(run_id, origin(), 8)
        .await
        .expect("poll after restart");
    assert_eq!(batch.inputs.len(), 1);
    assert_eq!(batch.inputs[0].input, input);
    assert_eq!(batch.inputs[0].ack_token, envelope.ack_token);
    assert_eq!(batch.inputs[0].cursor, envelope.cursor);
}

async fn conformance_enqueue_poll_ack<Q: ConformanceQueue>(
    queue: Q,
    thread_service: Arc<InMemorySessionThreadService>,
) {
    let scope = ghost_scope();
    let thread = thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-iq".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-iq".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("queued steering"),
        })
        .await
        .unwrap();
    let run_id = TurnRunId::new();
    thread_service
        .mark_message_queued(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            run_id.to_string(),
        )
        .await
        .unwrap();

    queue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: accepted.message_id,
            input: steering(&format!("msg:{}", accepted.message_id)),
        })
        .await
        .expect("enqueue");

    let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
    assert_eq!(batch.inputs.len(), 1);

    queue
        .ack_consumed(run_id, vec![batch.inputs[0].ack_token.clone()])
        .await
        .expect("ack");

    // Status durably flipped to Submitted...
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    // ...and the consumed input is not redelivered.
    let after = queue
        .next_after(run_id, batch.next_cursor, 8)
        .await
        .expect("poll after ack");
    assert!(after.inputs.is_empty());
}

#[tokio::test]
async fn durable_enqueue_poll_ack_flips_status_and_stops_redelivery() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let queue = durable_queue(Arc::clone(&thread_service));
    conformance_enqueue_poll_ack(queue, thread_service).await;
}

#[tokio::test]
async fn in_memory_enqueue_poll_ack_flips_status_and_stops_redelivery() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let queue = in_memory_queue(Arc::clone(&thread_service));
    conformance_enqueue_poll_ack(queue, thread_service).await;
}

/// Terminal reconciliation (both backends: the durable queue here, the
/// in-memory queue below shares the helpers): `reject_unconsumed` claims
/// every undrained entry — flipping its row `Queued` → `RejectedBusy` and
/// stopping redelivery — while an entry the loop already consumed keeps
/// its `Submitted` row. Idempotent: a second call reconciles nothing.
async fn conformance_reject_unconsumed<Q: ConformanceQueue>(
    queue: Q,
    thread_service: Arc<InMemorySessionThreadService>,
) {
    let scope = ghost_scope();
    let thread = thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-iq".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let run_id = TurnRunId::new();
    let mut message_ids = Vec::new();
    for text in ["consumed before cancel", "stranded by cancel"] {
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-iq".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .unwrap();
        thread_service
            .mark_message_queued(
                &scope,
                &thread.thread_id,
                accepted.message_id,
                run_id.to_string(),
            )
            .await
            .unwrap();
        message_ids.push(accepted.message_id);
    }

    for message_id in &message_ids {
        queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                message_id: *message_id,
                input: steering(&format!("msg:{message_id}")),
            })
            .await
            .expect("enqueue");
    }

    // The loop consumes the FIRST input, then the run is cancelled.
    let batch = queue.next_after(run_id, origin(), 1).await.expect("poll");
    assert_eq!(batch.inputs.len(), 1);
    queue
        .ack_consumed(run_id, vec![batch.inputs[0].ack_token.clone()])
        .await
        .expect("ack first");

    let rejected = queue.reject_unconsumed(run_id).await.expect("reconcile");
    assert_eq!(rejected, vec![message_ids[1]]);

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    let status_of = |message_id| {
        history
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .expect("row in history")
            .status
    };
    assert_eq!(
        status_of(message_ids[0]),
        MessageStatus::Submitted,
        "the consumed row keeps its Submitted status"
    );
    assert_eq!(
        status_of(message_ids[1]),
        MessageStatus::RejectedBusy,
        "the stranded row flips to the resend affordance"
    );

    // The claimed entry is never redelivered, and reconciling again is a
    // no-op.
    let after = queue
        .next_after(run_id, origin(), 8)
        .await
        .expect("poll after reconcile");
    assert!(after.inputs.is_empty());
    let again = queue.reject_unconsumed(run_id).await.expect("idempotent");
    assert!(again.is_empty());
}

#[tokio::test]
async fn durable_reject_unconsumed_flips_stranded_rows_and_stops_redelivery() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let queue = durable_queue(Arc::clone(&thread_service));
    conformance_reject_unconsumed(queue, thread_service).await;
}

#[tokio::test]
async fn in_memory_reject_unconsumed_flips_stranded_rows_and_stops_redelivery() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let queue = in_memory_queue(Arc::clone(&thread_service));
    conformance_reject_unconsumed(queue, thread_service).await;
}

/// The origin cursor is the unique run-start position: the first enqueued
/// input's cursor is sequence 1, strictly after origin (sequence 0), and a
/// cursor at or past the next-to-issue sequence is rejected as unissued
/// instead of read as an empty position.
async fn conformance_origin_cursor_semantics<Q: ConformanceQueue>(queue: Q) {
    let run_id = TurnRunId::new();
    let envelope = queue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: ghost_scope(),
            thread_id: ThreadId::new("thread-origin").unwrap(),
            message_id: ThreadMessageId::new(),
            input: steering("msg:origin-unique"),
        })
        .await
        .expect("enqueue");
    assert_ne!(
        envelope.cursor,
        origin(),
        "the first input's cursor must be strictly after the origin cursor"
    );

    let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
    assert_eq!(batch.inputs.len(), 1);
    // Polling from the entry's own cursor returns nothing (strict-after).
    let after = queue
        .next_after(run_id, batch.inputs[0].cursor.clone(), 8)
        .await
        .expect("poll strictly after");
    assert!(after.inputs.is_empty());
    // A cursor for a sequence that was never issued is rejected.
    let future = queue
        .next_after(
            run_id,
            LoopInputCursorToken::new("input-cursor:2".to_string()).expect("token"),
            8,
        )
        .await;
    assert!(
        matches!(future, Err(HostInputQueueError::InvalidCursor { .. })),
        "unissued future cursor must be rejected, got {future:?}"
    );
}

#[tokio::test]
async fn durable_origin_cursor_is_unique_and_future_cursors_are_rejected() {
    conformance_origin_cursor_semantics(durable_queue(Arc::new(
        InMemorySessionThreadService::default(),
    )))
    .await;
}

#[tokio::test]
async fn in_memory_origin_cursor_is_unique_and_future_cursors_are_rejected() {
    conformance_origin_cursor_semantics(in_memory_queue(Arc::new(
        InMemorySessionThreadService::default(),
    )))
    .await;
}

/// Regression: a corrupt persisted queue document surfaces as
/// `Unavailable` from BOTH `next_after` and `enqueue_queued_message`, and
/// the corrupt bytes are preserved for diagnosis — never silently
/// overwritten with a fresh document.
#[tokio::test]
async fn corrupt_durable_document_surfaces_unavailable_and_is_preserved() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem = make_fs(Arc::clone(&backend));
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let queue =
        FilesystemHostInputQueue::new(Arc::clone(&filesystem), owner_scope(), thread_service);
    let run_id = TurnRunId::new();
    let path = queue_path(run_id).expect("path");
    let corrupt = b"{not json".to_vec();
    filesystem
        .put(
            &owner_scope(),
            &path,
            Entry::bytes(corrupt.clone()).with_content_type(ContentType::json()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed corrupt document");

    let poll = queue.next_after(run_id, origin(), 8).await;
    assert!(
        matches!(poll, Err(HostInputQueueError::Unavailable { .. })),
        "corrupt document must surface Unavailable from next_after, got {poll:?}"
    );
    let enqueue = queue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: ghost_scope(),
            thread_id: ThreadId::new("thread-corrupt").unwrap(),
            message_id: ThreadMessageId::new(),
            input: steering("msg:corrupt"),
        })
        .await;
    assert!(
        matches!(enqueue, Err(HostInputQueueError::Unavailable { .. })),
        "corrupt document must surface Unavailable from enqueue, got {enqueue:?}"
    );
    let preserved = filesystem
        .get(&owner_scope(), &path)
        .await
        .expect("read back")
        .expect("document still present");
    assert_eq!(
        preserved.entry.body, corrupt,
        "the corrupt document must be preserved, not overwritten"
    );
}

/// CAS retry under real contention: concurrent enqueues (producer side)
/// against the same run document must each land exactly once — a lost
/// update would drop a queued message; a resurrected ack would redeliver.
#[tokio::test]
async fn concurrent_enqueues_and_acks_survive_cas_contention() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem = make_fs(backend);
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let queue = Arc::new(FilesystemHostInputQueue::new(
        filesystem,
        owner_scope(),
        thread_service,
    ));
    let run_id = TurnRunId::new();

    let mut joins = Vec::new();
    for index in 0..8 {
        let queue = Arc::clone(&queue);
        joins.push(tokio::spawn(async move {
            queue
                .enqueue_queued_message(EnqueueQueuedMessageRequest {
                    run_id,
                    turn_id: TurnId::new(),
                    scope: ghost_scope(),
                    thread_id: ThreadId::new("thread-contention").unwrap(),
                    message_id: ThreadMessageId::new(),
                    input: steering(&format!("msg:contended-{index}")),
                })
                .await
        }));
    }
    for join in joins {
        join.await.expect("task").expect("enqueue under contention");
    }

    let batch = queue.next_after(run_id, origin(), 16).await.expect("poll");
    assert_eq!(
        batch.inputs.len(),
        8,
        "every contended enqueue must land exactly once"
    );

    // Concurrent acks of disjoint halves: every input acked exactly once,
    // nothing redelivered afterward.
    let (first_half, second_half) = batch.inputs.split_at(4);
    let tokens_a: Vec<_> = first_half.iter().map(|e| e.ack_token.clone()).collect();
    let tokens_b: Vec<_> = second_half.iter().map(|e| e.ack_token.clone()).collect();
    let queue_a = Arc::clone(&queue);
    let queue_b = Arc::clone(&queue);
    let (result_a, result_b) = tokio::join!(
        queue_a.ack_consumed(run_id, tokens_a),
        queue_b.ack_consumed(run_id, tokens_b)
    );
    result_a.expect("ack half A under contention");
    result_b.expect("ack half B under contention");
    let after = queue
        .next_after(run_id, origin(), 16)
        .await
        .expect("poll after acks");
    assert!(
        after.inputs.is_empty(),
        "no input may be redelivered after concurrent acks, got {}",
        after.inputs.len()
    );
}

#[tokio::test]
async fn ack_is_non_fatal_and_idempotent_when_status_flip_fails() {
    let backend = Arc::new(InMemoryBackend::new());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let queue = FilesystemHostInputQueue::new(make_fs(backend), owner_scope(), thread_service);
    let run_id = TurnRunId::new();
    let envelope = queue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: ghost_scope(),
            thread_id: ThreadId::new("ghost").unwrap(),
            message_id: ThreadMessageId::new(),
            input: steering("msg:ghost"),
        })
        .await
        .expect("enqueue");

    // Status flip fails (ghost thread) but the ack still commits durably.
    queue
        .ack_consumed(run_id, vec![envelope.ack_token.clone()])
        .await
        .expect("ack must be non-fatal when the status flip fails");
    // A redelivered ack for the same token is an idempotent no-op.
    queue
        .ack_consumed(run_id, vec![envelope.ack_token])
        .await
        .expect("idempotent ack");

    let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
    assert!(
        batch.inputs.is_empty(),
        "acked input must not be redelivered"
    );
}

#[tokio::test]
async fn enqueue_dedups_identical_input() {
    let backend = Arc::new(InMemoryBackend::new());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let queue = FilesystemHostInputQueue::new(make_fs(backend), owner_scope(), thread_service);
    let run_id = TurnRunId::new();
    let request = || EnqueueQueuedMessageRequest {
        run_id,
        turn_id: TurnId::new(),
        scope: ghost_scope(),
        thread_id: ThreadId::new("ghost").unwrap(),
        message_id: ThreadMessageId::new(),
        input: steering("msg:dup"),
    };
    let first = queue
        .enqueue_queued_message(request())
        .await
        .expect("first");
    let second = queue
        .enqueue_queued_message(request())
        .await
        .expect("second");
    assert_eq!(first.ack_token, second.ack_token, "identical input dedups");

    let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
    assert_eq!(batch.inputs.len(), 1, "dedup keeps a single queue entry");
}

#[tokio::test]
async fn ack_rejects_unknown_sequence_instead_of_poisoning_state() {
    // An ack token for a sequence that is neither live nor already acked
    // must fail loud rather than be committed into `acked`. Committing it
    // would poison durable state: when that sequence is later enqueued, its
    // now-pre-acked entry would be skipped forever by `next_after`.
    let backend = Arc::new(InMemoryBackend::new());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let queue =
        FilesystemHostInputQueue::new(make_fs(Arc::clone(&backend)), owner_scope(), thread_service);
    let run_id = TurnRunId::new();
    // Create the queue document with a single live entry at sequence 0.
    queue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: ghost_scope(),
            thread_id: ThreadId::new("ghost").unwrap(),
            message_id: ThreadMessageId::new(),
            input: steering("msg:live"),
        })
        .await
        .expect("enqueue");

    // Ack a forged token for a sequence that was never enqueued.
    let forged = LoopInputAckToken::new("input-ack:999".to_string()).unwrap();
    let result = queue.ack_consumed(run_id, vec![forged]).await;
    assert!(
        matches!(result, Err(HostInputQueueError::InvalidCursor { .. })),
        "unknown ack sequence must be rejected, got {result:?}"
    );

    // State is untouched: sequence 999 was NOT recorded as acked, so a
    // later real entry at that sequence would still be delivered.
    let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
    assert_eq!(
        batch.inputs.len(),
        1,
        "the live entry remains deliverable after a rejected forged ack"
    );
}
