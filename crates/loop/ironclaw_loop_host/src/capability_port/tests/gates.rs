use super::*;

/// #6287 IronLoop: when the owning persist future is cancelled (dropped) mid
/// `save`, the in-flight gate-resolution reservation must be cleared and its
/// waiters woken — else a same-key replay hangs forever on an orphaned
/// reservation. The RAII `GateResolutionReservationGuard` does that on drop.
/// This cancels the first invocation while it is parked in `save`, then
/// asserts a replay re-owns the reservation, completes without hanging, and
/// persists exactly one record.
#[tokio::test]
async fn cancelled_gate_persist_clears_reservation_so_replay_can_re_own() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    let store = Arc::new(BlockingGateRecordStore::new());
    let port = Arc::new(
        runtime_capability_port_with_gate_store(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
            )),
            Arc::new(RecordingResultWriter::default()),
            dummy_milestone_sink(),
            store.clone(),
            "thread-cancelled-gate-persist",
        )
        .await,
    );

    let invocation = visible_runtime_invocation(&port).await;

    // First invocation parks in the blocked `save`, then is cancelled.
    let spawn_port = Arc::clone(&port);
    let spawn_invocation = invocation.clone();
    let handle = tokio::spawn(async move { spawn_port.invoke_capability(spawn_invocation).await });
    store
        .entered
        .acquire()
        .await
        .expect("save entered")
        .forget();
    handle.abort();
    let _ = handle.await;

    // Replay must NOT hang on the orphaned reservation: it re-owns, saves, and
    // returns the gate resolution.
    let replayed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        port.invoke_capability(invocation),
    )
    .await
    .expect("replay must not hang on an orphaned in-flight reservation")
    .expect("replay gate outcome");
    assert!(
        matches!(&replayed, Resolution::Blocked(Blocked::Approval(_))),
        "replay must surface the gate, got {replayed:?}"
    );
    assert_eq!(
        store.saved().len(),
        1,
        "the replay must persist exactly one gate record after the cancelled attempt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_duplicate_gate_invocations_share_one_persisted_resolution() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    let store = Arc::new(BlockingGateRecordStore::new());
    let port = Arc::new(
        runtime_capability_port_with_gate_store(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
            )),
            Arc::new(RecordingResultWriter::default()),
            dummy_milestone_sink(),
            store.clone(),
            "thread-concurrent-gate-persist",
        )
        .await,
    );
    let invocation = visible_runtime_invocation(&port).await;

    let owner_port = Arc::clone(&port);
    let owner_invocation = invocation.clone();
    let owner = tokio::spawn(async move { owner_port.invoke_capability(owner_invocation).await });
    store
        .entered
        .acquire()
        .await
        .expect("owner save entered")
        .forget();

    let waiter_port = Arc::clone(&port);
    let waiter = tokio::spawn(async move { waiter_port.invoke_capability(invocation).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiter_holds_reservation = port
                .persisted_gate_resolutions
                .lock()
                .expect("gate resolution reservations lock")
                .values()
                .any(|state| {
                    matches!(
                        state,
                        GateResolutionState::InFlight(notify)
                            if Arc::strong_count(notify) >= 3
                    )
                });
            if waiter_holds_reservation {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("waiter must park on the in-flight reservation");
    store.release.notify_one();

    let owner_resolution = tokio::time::timeout(std::time::Duration::from_secs(5), owner)
        .await
        .expect("owner must finish")
        .expect("owner task")
        .expect("owner resolution");
    let waiter_resolution = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .expect("waiter must finish")
        .expect("waiter task")
        .expect("waiter resolution");

    assert_eq!(
        gate_ref_for_resolution(&owner_resolution),
        gate_ref_for_resolution(&waiter_resolution),
        "the waiter must receive the owner's persisted gate resolution"
    );
    assert_eq!(
        store.saved().len(),
        1,
        "concurrent duplicates must persist one gate record"
    );
}

/// Slice C result-side seam (§5.3): a gate outcome produced by the capability
/// seam persists the durable, model-visible `GateRecord` a later resume turn
/// renders from, keyed by the minted `GateRef` on the `Resolution` channel,
/// while the loop still receives the unchanged `CapabilityOutcome` (its resume
/// token intact). Drives the production caller (`invoke_capability`) and
/// asserts at the store seam that the record round-trips. The durable
/// `GateRecordStore` round-trip is covered separately by
/// `ironclaw_approvals`'s `gate_record_store_contract`.
#[tokio::test]
async fn approval_gate_outcome_persists_gate_record_at_the_seam() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    let store = Arc::new(RecordingGateRecordStore::default());
    let port = runtime_capability_port_with_gate_store(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
        )),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        store.clone(),
        "thread-approval-gate-persist",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("approval gate outcome should be produced");

    // Behavior preserved: the loop still receives the ApprovalRequired outcome
    // (resume token intact); the seam persists alongside, it does not replace.
    assert!(
        matches!(&outcome, Resolution::Blocked(Blocked::Approval(_))),
        "expected ApprovalRequired, got {outcome:?}"
    );

    // The seam persisted exactly one gate record, keyed by the minted GateRef
    // on the Resolution channel, and it round-trips via the store.
    let saved = store.saved();
    assert_eq!(saved.len(), 1, "exactly one gate record persisted");
    let (scope, gate_ref, record) = saved.into_iter().next().expect("one saved record");
    assert!(
        matches!(record, GateRecord::Approval { .. }),
        "expected GateRecord::Approval, got {record:?}"
    );
    // Regression (#6287 IronLoop): the record must be keyed by the gate ref
    // the RETURNED Resolution carries — not merely one the seam happened to
    // save. The approval/resource/dependent/external gates mint a FRESH random
    // `GateRef` on every `capability_outcome_to_resolution` call, so mapping
    // the outcome a second time to build the return value (as the pre-fix
    // `invoke_capability` did) handed the executor a gate ref no record was
    // ever saved under, and the resume could never load it. `invoke_capability`
    // now maps ONCE and persists/returns the same `MappedResolution`.
    let resolution_gate_ref =
        gate_ref_for_resolution(&outcome).expect("blocked resolution carries a gate ref");
    assert_eq!(
        resolution_gate_ref, gate_ref,
        "the returned Resolution's gate ref must equal the persisted record's key"
    );
    assert_eq!(
        store
            .load(&scope, gate_ref)
            .await
            .expect("gate record load"),
        Some(record),
        "persisted gate record must round-trip via the store"
    );
}

/// A replayed invocation (same idempotency key) returns the CACHED gate
/// outcome from the dispatch records — it must NOT persist a second gate
/// record: gate records are write-once with no removal API, so a duplicate
/// persist per retry would accumulate orphaned records under freshly-minted
/// `GateRef`s (2026-07-18 ironloopai review finding on #6245).
#[tokio::test]
async fn replayed_gate_invocation_does_not_persist_a_duplicate_record() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    let store = Arc::new(RecordingGateRecordStore::default());
    // Exactly ONE runtime outcome queued: the second invoke must be served
    // from the dispatch cache, not the runtime.
    let port = runtime_capability_port_with_gate_store(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
        )),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        store.clone(),
        "thread-replayed-gate-no-duplicate",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    let first = port
        .invoke_capability(invocation.clone())
        .await
        .expect("first gate outcome");
    let replayed = port
        .invoke_capability(invocation)
        .await
        .expect("replayed gate outcome");
    assert!(
        matches!(&first, Resolution::Blocked(Blocked::Approval(_)))
            && matches!(&replayed, Resolution::Blocked(Blocked::Approval(_))),
        "both invocations must surface the gate"
    );

    let saved = store.saved();
    assert_eq!(
        saved.len(),
        1,
        "a replayed gate invocation must not persist a duplicate gate record"
    );

    // Regression (#6287 IronLoop): the replay must return the SAME gate ref
    // the single record was persisted under — not a freshly-minted one. The
    // mapping mints a random `GateRef` per call, so without the replay
    // resolution cache the replayed `Resolution` would carry an unloadable
    // ref while the one saved record sits under the first invocation's ref.
    let first_ref = gate_ref_for_resolution(&first).expect("first resolution gate ref");
    let replayed_ref = gate_ref_for_resolution(&replayed).expect("replayed resolution gate ref");
    assert_eq!(
        first_ref, replayed_ref,
        "the replay must return the first invocation's gate ref, not a fresh mint"
    );
    let (scope, saved_ref, record) = saved.into_iter().next().expect("one saved record");
    assert_eq!(
        replayed_ref, saved_ref,
        "the replayed gate ref must equal the persisted record's key"
    );
    assert_eq!(
        store
            .load(&scope, replayed_ref)
            .await
            .expect("gate record load by replayed ref"),
        Some(record),
        "the record must be loadable by the gate ref the replayed Resolution carries"
    );
}

/// A transient store fault must not permanently skip persistence: the
/// replay-guard entry is rolled back on a failed save, so the next replay
/// of the same invocation retries the persist and succeeds.
#[tokio::test]
async fn failed_gate_record_persist_is_retried_on_replay() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    let store = Arc::new(FailOnceGateRecordStore::default());
    let port = runtime_capability_port_with_gate_store(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
        )),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        store.clone(),
        "thread-failed-gate-persist-retry",
    )
    .await;

    let invocation = visible_runtime_invocation(&port).await;
    // First attempt: the store fails once → the seam fails closed.
    port.invoke_capability(invocation.clone())
        .await
        .expect_err("first persist attempt must fail closed on the store fault");
    // Replay: the dispatch cache serves the gate outcome and the rolled-back
    // guard lets the persist retry — exactly one record lands.
    let replayed = port
        .invoke_capability(invocation)
        .await
        .expect("replayed invocation persists and returns the gate");
    assert!(matches!(
        replayed,
        Resolution::Blocked(Blocked::Approval(_))
    ));
    assert_eq!(
        store.inner.saved().len(),
        1,
        "the retried persist must land exactly one record"
    );
}

/// The transitional no-op default (`NoopGateRecordStore`) is behavior-
/// preserving: a gate outcome through a factory that never called
/// `with_gate_record_store` still returns the gate to the loop (it does not
/// fail closed), so an unwired composition path keeps producing gates exactly
/// as before the seam. The durable write turns on only once a store is wired.
#[tokio::test]
async fn gate_outcome_through_unwired_default_is_inert_and_non_regressing() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let gate = ironclaw_host_runtime::RuntimeApprovalGate {
        approval_request_id: ironclaw_host_api::ids::ApprovalRequestId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::ApprovalRequired,
    };
    // `runtime_capability_port` builds the factory WITHOUT `with_gate_record_store`,
    // so the port holds the transitional no-op default.
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::ApprovalRequired(gate))],
        )),
        Arc::new(RecordingResultWriter::default()),
        dummy_milestone_sink(),
        "thread-unwired-gate-inert",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("unwired gate store must not fail the gate outcome");
    assert!(
        matches!(&outcome, Resolution::Blocked(Blocked::Approval(_))),
        "expected ApprovalRequired, got {outcome:?}"
    );
}
