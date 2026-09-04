//! Proves that a child agent-turn's approval suspension reaches the owner's
//! inbox through the *real* process journal observer registry, not through a
//! direct call into `RunOutcomeProcessCommitObserver::observe_process_commit`.
//!
//! What this test proves: a real `ProcessJournalStore` subscribes a real
//! `RunOutcomeProcessCommitObserver` via `ProcessJournalObserverRegistry`,
//! then a child-shaped `AgentTurn` process (`parent_process_id` set,
//! `subagent_depth: 1`) is driven, through the store's own claim/suspend
//! transitions, to a `ProcessJournalKind::Suspended` commit carrying a
//! `ProcessSuspension { kind: Approval, .. }`. Delivery goes through the
//! store's cursor-based fan-out to every subscribed observer — the same path
//! a production journal store uses for every commit, not a synthetic
//! `ProcessJournalCommit` handed to the observer directly. The resulting
//! inbox item is then read back through a real `NotificationInboxStore`
//! rather than an in-memory `Vec` a test controls.
//!
//! What this test deliberately does not prove: that production composition
//! wires this observer onto a store that a *real spawned subagent* writes
//! to. `spawn_subagent` is deny-filtered by default
//! (`default_disabled_capability_ids`,
//! `crates/loop/ironclaw_turn_runner/src/runtime.rs:294-299`), and
//! `ironclaw_composition::build_runtime` reads that deny list internally with
//! no input seam to override it
//! (`crates/app/ironclaw_composition/src/runtime.rs:4055`), so no
//! composition-tier test can spawn a real child process today. That
//! end-to-end proof — a live `spawn_subagent` call landing a suspension in
//! this same store under production wiring — waits on the R9 enable slice /
//! R4 harness enablement work tracked alongside subagent rollout.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_assistant::RunOutcomeProcessCommitObserver;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProcessId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
    turn::TurnGateRef,
};
use ironclaw_notifications::{
    ListNotificationsRequest, NOTIFICATION_INBOX_MAX_RECORDS, NotificationAction,
    NotificationInboxStore, NotificationInboxStorePort, NotificationKind, NotificationRecipient,
};
use ironclaw_processes::{
    ClaimProcessesRequest, ProcessCheckpointRef, ProcessJournalObserverRegistry,
    ProcessJournalStore, ProcessKind, ProcessSubmissionPort, ProcessSuspension,
    ProcessSuspensionKind, ProcessTransitionPort, ProcessWorkerId, SubmitProcessRequest,
    SuspendProcessRequest,
};
use ironclaw_threads::InMemorySessionThreadService;
use serde_json::json;

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-child-gate").expect("tenant"),
        user_id: UserId::new("owner-child-gate").expect("user"),
        agent_id: Some(AgentId::new("agent-child-gate").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: Some(ThreadId::new("child-thread-child-gate").expect("child thread")),
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    }
}

fn process_journal_store() -> ProcessJournalStore<InMemoryBackend> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("process mount alias"),
        VirtualPath::new("/engine/test/child-gate-processes").expect("process mount target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("process mount view");
    ProcessJournalStore::new(Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    )))
}

fn notification_inbox() -> Arc<NotificationInboxStore<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/notifications").expect("notification mount alias"),
        VirtualPath::new("/engine/test/child-gate-notifications")
            .expect("notification mount target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("notification mount view");
    Arc::new(NotificationInboxStore::new(
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        )),
        NOTIFICATION_INBOX_MAX_RECORDS,
    ))
}

#[tokio::test]
async fn a_child_approval_suspension_reaches_the_owner_inbox_through_the_observer_registry() {
    let store = process_journal_store();
    let inbox = notification_inbox();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let observer = Arc::new(RunOutcomeProcessCommitObserver::new(
        Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
        thread_service as Arc<dyn ironclaw_threads::SessionThreadService>,
    ));
    // Subscribe through the real registry trait — the exact seam the finding
    // says no test exercises.
    ProcessJournalObserverRegistry::subscribe_process_observer(&store, observer)
        .expect("subscribe run-outcome observer on the real registry");

    let scope = scope();
    let parent_process_id = ProcessId::new();
    let child_process_id = ProcessId::new();

    // A child's `parent_process_id` must reference a process the store
    // already knows (`apply_submit`,
    // crates/kernel/ironclaw_processes/src/journal_store/state.rs:334-339),
    // so the parent is submitted first, as a standalone internal bookkeeping
    // process in the same lineage scope.
    store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
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

    let metadata = json!({
        "agent_turn": {
            "product_context": { "origin": "web_ui" },
            "execution_outcome": "result_available",
            // Both signals the observer's `child_gate_run` predicate accepts
            // are set so this fixture is unambiguously "a child".
            "subagent_depth": 1,
        }
    });

    // Submitting a fresh AgentTurn process (child-shaped: parent_process_id
    // set, owner + thread + agent present) durably commits a `Submitted`
    // journal row and is fanned out to the subscribed observer, which
    // screens it out (no suspension yet) without publishing anything.
    store
        .submit_process(SubmitProcessRequest {
            process_id: child_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(parent_process_id),
            root_process_id: Some(parent_process_id),
            spawn_tree_descendant_cap: Some(1),
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: metadata.clone(),
        })
        .await
        .expect("submit child agent-turn process");

    // Claim-then-suspend is the route that is guaranteed to emit a
    // `ProcessJournalKind::Suspended` journal row. `submit_process_at_edge`
    // was considered and ruled out: it is hard-restricted to standalone
    // `CapabilityInvocationState` bookkeeping processes and rejects any
    // submission with `parent_process_id` set
    // (`apply_submit_at_edge`, crates/kernel/ironclaw_processes/src/journal_store/state.rs:495-506),
    // so it cannot represent a child `AgentTurn` at all, let alone drive one
    // to `Suspended`.
    let worker_id = ProcessWorkerId::from_trusted("child-gate-worker");
    let claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: worker_id.clone(),
            scope_filter: Some(scope.clone()),
            process_id_filter: Some(child_process_id),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim the child agent-turn process")
        .pop()
        .expect("child process is claimable");
    assert_eq!(claimed.state.process_id, child_process_id);

    let gate_ref = TurnGateRef::new("gate:child-approval-1").expect("gate ref");
    let suspended = store
        .suspend_process(SuspendProcessRequest {
            process_id: child_process_id,
            worker_id,
            lease_token: claimed.lease_token,
            checkpoint_ref: ProcessCheckpointRef::from_trusted("child-gate-checkpoint"),
            suspension: ProcessSuspension {
                kind: ProcessSuspensionKind::Approval,
                gate_ref: Some(gate_ref.clone()),
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
            metadata: Some(metadata),
        })
        .await
        .expect("suspend the child agent-turn process on an approval gate");
    assert_eq!(
        suspended.status,
        ironclaw_processes::ProcessLifecycleStatus::Suspended
    );

    // Read the inbox back through its own store API — not a test-owned
    // `Vec` the observer was handed a reference to.
    let recipient = NotificationRecipient {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
    };
    let page = inbox
        .list(ListNotificationsRequest {
            recipient,
            limit: 16,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list owner inbox");

    assert_eq!(
        page.notifications.len(),
        1,
        "exactly one notification for the one child approval gate, got {:?}",
        page.notifications
    );
    let item = &page.notifications[0];
    assert_eq!(item.kind, NotificationKind::ApprovalRequired);
    assert_eq!(
        item.source.turn_run_id,
        Some(ironclaw_host_api::turn::TurnRunId::from_uuid(
            child_process_id.as_uuid()
        ))
    );
    assert_eq!(item.source.thread_id, scope.thread_id.clone());
    assert_eq!(
        item.action,
        NotificationAction::OpenThread {
            thread_id: scope.thread_id.clone().expect("child thread id")
        }
    );
    assert!(
        item.resolved_at.is_none(),
        "a pending child approval gate stays actionable in the owner's inbox"
    );
    assert!(
        item.source.credential_providers.is_empty(),
        "an approval gate is not an auth gate; it must not carry credential providers"
    );
}
