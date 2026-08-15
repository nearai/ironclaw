// arch-exempt: large_file, edge-submission denial cases belong with materialized-state transitions, plan #7598
use std::collections::HashSet;

use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    resource::ResourceScope,
};
use serde_json::json;

use super::*;
use crate::{
    ClaimProcessesRequest, CloseProcessDependencyRequest, OpenProcessDependencyRequest,
    ProcessCheckpointPayload, ProcessConcurrencyClass, ProcessDependencyState,
    ProcessDependencySubmission, ProcessInputPayload, ProcessInputRef, ProcessInputSubmission,
    ProcessLeaseRequest, ProcessOperationId, ProcessSubmissionEdge, ProcessSuspension,
    ProcessSuspensionKind, ProcessTerminalEvidence, ProcessWorkerId, PruneReleasedProcessRequest,
    RecordProcessCheckpointRequest, RecoverExpiredProcessLeasesRequest, ReleaseProcessTreeRequest,
    ReserveProcessTreeRequest, SettleProcessDependencyRequest, StateTransitionCase,
    SubmitProcessAtEdgeRequest, SubmitProcessRequest, assert_state_transition_table,
    journal_store::{ProcessControlMutation, ProcessTransitionMutation, StoredProcessCommand},
};

fn scope(label: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).expect("tenant"),
        user_id: UserId::new(format!("user-{label}")).expect("user"),
        agent_id: Some(AgentId::new(format!("agent-{label}")).expect("agent")),
        project_id: Some(ProjectId::new(format!("project-{label}")).expect("project")),
        mission_id: None,
        thread_id: Some(ThreadId::new(format!("thread-{label}")).expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn snapshot(
    process_id: ProcessId,
    status: ProcessLifecycleStatus,
    scope: ResourceScope,
) -> JournaledProcessSnapshot {
    JournaledProcessSnapshot {
        process_id,
        process_kind: ProcessKind::Internal,
        scope,
        status,
        suspension: None,
        checkpoint_ref: None,
        checkpoint_kind: None,
        input_ref: None,
        failure: None,
        journal_cursor: ProcessJournalCursor(1),
        lease: None,
        crash_reclaim_count: 0,
        created_at: Utc::now(),
        owner_user_id: None,
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        metadata: serde_json::Value::Null,
    }
}

fn submit_request(process_id: ProcessId, request_scope: ResourceScope) -> SubmitProcessRequest {
    SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::Internal,
        scope: request_scope,
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
    }
}

fn error_class(error: ProcessJournalStoreError) -> &'static str {
    match error {
        ProcessJournalStoreError::UnknownProcess { .. } => "unknown",
        ProcessJournalStoreError::ProcessAlreadyExists { .. } => "exists",
        ProcessJournalStoreError::ActiveProcessConflict { .. } => "active",
        ProcessJournalStoreError::InvalidTransition { .. } => "transition",
        ProcessJournalStoreError::InvalidLease { .. } => "lease",
        ProcessJournalStoreError::StaleSnapshot { .. } => "stale",
        ProcessJournalStoreError::UnauthorizedScope => "scope",
        ProcessJournalStoreError::InvalidRequest(_) => "request",
        ProcessJournalStoreError::ProcessTreeCapacityExceeded { .. } => "capacity",
        ProcessJournalStoreError::InvalidPath(_) => "path",
        ProcessJournalStoreError::Filesystem(_) => "filesystem",
        ProcessJournalStoreError::Serialization(_) => "serialization",
        ProcessJournalStoreError::Deserialization(_) => "deserialization",
        ProcessJournalStoreError::Observer(_) => "observer",
        ProcessJournalStoreError::MigrationRequired => "migration",
        ProcessJournalStoreError::GroupCommitFailed { .. } => "group_commit",
    }
}

fn assert_error(
    result: Result<StoredCommandOutcome, ProcessJournalStoreError>,
    expected: &'static str,
) {
    assert_eq!(
        error_class(result.expect_err("transition must fail")),
        expected
    );
}

#[test]
fn submit_at_edge_materializes_only_the_requested_lifecycle_entry() {
    let cases = [
        (
            ProcessSubmissionEdge::Completed,
            ProcessLifecycleStatus::Completed,
            ProcessJournalKind::Completed,
        ),
        (
            ProcessSubmissionEdge::Failed {
                failure: SanitizedFailure::from_trusted_static("capability_failed"),
            },
            ProcessLifecycleStatus::Failed,
            ProcessJournalKind::Failed,
        ),
    ];

    for (edge, expected_status, expected_kind) in cases {
        let mut state = ProcessJournalMaterializedState::default();
        let mut submission = submit_request(ProcessId::new(), scope("edge"));
        submission.process_kind = ProcessKind::CapabilityInvocationState;
        let outcome = state
            .apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
                SubmitProcessAtEdgeRequest { submission, edge },
            )))
            .expect("edge submission");
        let StoredCommandOutcome::Submitted(snapshot, true) = outcome else {
            panic!("expected new edge submission");
        };

        assert_eq!(snapshot.status, expected_status);
        assert_eq!(snapshot.lease, None);
        assert_eq!(state.journal.len(), 1);
        assert_eq!(state.journal[0].kind, expected_kind);
    }

    let mut state = ProcessJournalMaterializedState::default();
    let checkpoint_ref = ProcessCheckpointRef::from_trusted("invocation-edge");
    let suspension = ProcessSuspension {
        kind: ProcessSuspensionKind::Approval,
        gate_ref: None,
        activity_id: None,
        credential_requirements: Vec::new(),
        detail: None,
    };
    let mut submission = submit_request(ProcessId::new(), scope("suspended-edge"));
    submission.process_kind = ProcessKind::CapabilityInvocationState;
    submission.checkpoint_ref = Some(checkpoint_ref.clone());
    let outcome = state
        .apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
            SubmitProcessAtEdgeRequest {
                submission,
                edge: ProcessSubmissionEdge::Suspended {
                    suspension: suspension.clone(),
                },
            },
        )))
        .expect("suspended edge submission");
    let StoredCommandOutcome::Submitted(snapshot, true) = outcome else {
        panic!("expected new suspended edge submission");
    };
    assert_eq!(snapshot.status, ProcessLifecycleStatus::Suspended);
    assert_eq!(snapshot.checkpoint_ref, Some(checkpoint_ref));
    assert_eq!(snapshot.suspension, Some(suspension));
    assert_eq!(state.journal.len(), 1);
    assert_eq!(state.journal[0].kind, ProcessJournalKind::Suspended);

    assert_error(
        ProcessJournalMaterializedState::default().apply_command(
            StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
                submission: submit_request(ProcessId::new(), scope("invalid-edge-kind")),
                edge: ProcessSubmissionEdge::Completed,
            })),
        ),
        "request",
    );
}
#[test]
fn submit_at_edge_rejects_incompatible_submissions() {
    let suspension = ProcessSuspension {
        kind: ProcessSuspensionKind::Approval,
        gate_ref: None,
        activity_id: None,
        credential_requirements: Vec::new(),
        detail: None,
    };
    let mut missing_checkpoint = submit_request(ProcessId::new(), scope("missing-checkpoint"));
    missing_checkpoint.process_kind = ProcessKind::CapabilityInvocationState;
    assert_error(
        ProcessJournalMaterializedState::default().apply_command(
            StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
                submission: missing_checkpoint,
                edge: ProcessSubmissionEdge::Suspended { suspension },
            })),
        ),
        "request",
    );

    for edge in [
        ProcessSubmissionEdge::Completed,
        ProcessSubmissionEdge::Failed {
            failure: SanitizedFailure::from_trusted_static("edge_failed"),
        },
    ] {
        let mut terminal_checkpoint =
            submit_request(ProcessId::new(), scope("terminal-checkpoint"));
        terminal_checkpoint.process_kind = ProcessKind::CapabilityInvocationState;
        terminal_checkpoint.checkpoint_ref =
            Some(ProcessCheckpointRef::from_trusted("terminal-checkpoint"));
        assert_error(
            ProcessJournalMaterializedState::default().apply_command(
                StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
                    submission: terminal_checkpoint,
                    edge,
                })),
            ),
            "request",
        );
    }

    let incompatible_shapes = [
        (
            "exclusive",
            (|submission: &mut SubmitProcessRequest| {
                submission.exclusive_within_scope = true;
            }) as fn(&mut SubmitProcessRequest),
        ),
        ("parent", |submission: &mut SubmitProcessRequest| {
            submission.parent_process_id = Some(ProcessId::new());
        }),
        ("root", |submission: &mut SubmitProcessRequest| {
            submission.root_process_id = Some(ProcessId::new());
        }),
        ("tree-cap", |submission: &mut SubmitProcessRequest| {
            submission.spawn_tree_descendant_cap = Some(1);
        }),
        ("dependency", |submission: &mut SubmitProcessRequest| {
            submission.dependency = Some(ProcessDependencySubmission {
                dependent_process_id: ProcessId::new(),
                root_process_id: ProcessId::new(),
                group_ref: None,
                metadata: serde_json::Value::Null,
            });
        }),
        ("input", |submission: &mut SubmitProcessRequest| {
            submission.input = Some(ProcessInputSubmission {
                input_ref: ProcessInputRef::from_trusted("edge-input"),
                payload: ProcessInputPayload::new(b"input".to_vec()).expect("bounded input"),
            });
        }),
    ];
    for (label, mutate) in incompatible_shapes {
        let mut submission = submit_request(ProcessId::new(), scope(label));
        submission.process_kind = ProcessKind::CapabilityInvocationState;
        mutate(&mut submission);
        assert_error(
            ProcessJournalMaterializedState::default().apply_command(
                StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
                    submission,
                    edge: ProcessSubmissionEdge::Completed,
                })),
            ),
            "request",
        );
    }
}

#[test]
fn edge_submissions_keep_only_compact_bindings_not_persisted_snapshots() {
    let mut state = ProcessJournalMaterializedState::default();
    let request_scope = scope("edge-bindings");
    let process_id = ProcessId::new();
    let mut submission = submit_request(process_id, request_scope);
    submission.process_kind = ProcessKind::CapabilityInvocationState;
    submission.operation_id = Some(ProcessOperationId::from_trusted("edge-op"));
    let command = || {
        StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
            submission: submission.clone(),
            edge: ProcessSubmissionEdge::Completed,
        }))
    };

    let StoredCommandOutcome::Submitted(_, true) = state
        .apply_command(command())
        .expect("initial edge submission")
    else {
        panic!("expected initial edge submission");
    };
    // The perf contract: an edge submission must not persist a full-snapshot
    // idempotency record that every later journal command re-serializes.
    // Only the compact in-memory binding is retained.
    assert!(
        state.submission_idempotency.is_empty(),
        "edge submissions must not grow the persisted idempotency map"
    );
    assert_eq!(
        state
            .edge_submission_bindings
            .values()
            .filter(|bound| **bound == process_id)
            .count(),
        1,
        "compact op-id binding must be recorded"
    );

    // Replay semantics are unchanged: the identical submission resolves from
    // the durable process row without a second journal entry, and a
    // same-operation submission for a different process is still rejected.
    let StoredCommandOutcome::Submitted(_, false) = state
        .apply_command(command())
        .expect("idempotent edge replay")
    else {
        panic!("expected idempotent edge replay");
    };
    assert_eq!(state.journal.len(), 1);

    let mut different_process = submission.clone();
    different_process.process_id = ProcessId::new();
    assert_error(
        state.apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
            SubmitProcessAtEdgeRequest {
                submission: different_process,
                edge: ProcessSubmissionEdge::Completed,
            },
        ))),
        "request",
    );
    assert_eq!(state.journal.len(), 1, "rejected replay must not write");
}

#[test]
fn submit_at_edge_replay_requires_matching_submission_and_writes_once() {
    let mut state = ProcessJournalMaterializedState::default();
    let mut submission = submit_request(ProcessId::new(), scope("edge-replay"));
    submission.process_kind = ProcessKind::CapabilityInvocationState;
    submission.operation_id = Some(ProcessOperationId::from_trusted("edge-operation"));
    let command = || {
        StoredProcessCommand::SubmitAtEdge(Box::new(SubmitProcessAtEdgeRequest {
            submission: submission.clone(),
            edge: ProcessSubmissionEdge::Completed,
        }))
    };

    let StoredCommandOutcome::Submitted(_, true) = state
        .apply_command(command())
        .expect("initial edge submission")
    else {
        panic!("expected initial edge submission");
    };
    let StoredCommandOutcome::Submitted(_, false) = state
        .apply_command(command())
        .expect("idempotent edge replay")
    else {
        panic!("expected idempotent edge replay");
    };
    assert_eq!(state.journal.len(), 1);

    let mut retried = submission.clone();
    retried.created_at += chrono::Duration::seconds(1);
    let StoredCommandOutcome::Submitted(_, false) = state
        .apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
            SubmitProcessAtEdgeRequest {
                submission: retried,
                edge: ProcessSubmissionEdge::Completed,
            },
        )))
        .expect("idempotent edge retry with a fresh attempt timestamp")
    else {
        panic!("expected idempotent edge retry");
    };

    let mismatched_submissions = [
        (|request: &mut SubmitProcessRequest| {
            request.process_id = ProcessId::new();
        }) as fn(&mut SubmitProcessRequest),
        |request: &mut SubmitProcessRequest| {
            request.owner_user_id = Some(UserId::new("different-owner").expect("owner"));
        },
        |request: &mut SubmitProcessRequest| {
            request.concurrency_class =
                Some(ProcessConcurrencyClass::from_trusted("different-class"));
        },
        |request: &mut SubmitProcessRequest| {
            request.checkpoint_ref =
                Some(ProcessCheckpointRef::from_trusted("different-checkpoint"));
        },
        |request: &mut SubmitProcessRequest| {
            request.metadata = json!({"record_type": "different"});
        },
    ];
    for mutate in mismatched_submissions {
        let mut mismatched = submission.clone();
        mutate(&mut mismatched);
        assert_error(
            state.apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
                SubmitProcessAtEdgeRequest {
                    submission: mismatched,
                    edge: ProcessSubmissionEdge::Completed,
                },
            ))),
            "request",
        );
    }
    assert_error(
        state.apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
            SubmitProcessAtEdgeRequest {
                submission,
                edge: ProcessSubmissionEdge::Failed {
                    failure: SanitizedFailure::from_trusted_static("different_edge"),
                },
            },
        ))),
        "request",
    );
}

#[test]
fn deployed_snapshot_import_maps_every_lifecycle_state_and_is_idempotent() {
    let cases = [
        (
            ProcessLifecycleStatus::Queued,
            ProcessJournalKind::Submitted,
        ),
        (ProcessLifecycleStatus::Running, ProcessJournalKind::Claimed),
        (
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
        ),
        (
            ProcessLifecycleStatus::StopRequested,
            ProcessJournalKind::StopRequested,
        ),
        (
            ProcessLifecycleStatus::CancelRequested,
            ProcessJournalKind::CancelRequested,
        ),
        (ProcessLifecycleStatus::Stopped, ProcessJournalKind::Stopped),
        (
            ProcessLifecycleStatus::Cancelled,
            ProcessJournalKind::Cancelled,
        ),
        (
            ProcessLifecycleStatus::Completed,
            ProcessJournalKind::Completed,
        ),
        (ProcessLifecycleStatus::Failed, ProcessJournalKind::Failed),
        (ProcessLifecycleStatus::Killed, ProcessJournalKind::Killed),
        (
            ProcessLifecycleStatus::RecoveryRequired,
            ProcessJournalKind::RecoveryRequired,
        ),
    ]
    .map(|(status, expected)| {
        StateTransitionCase::new("import lifecycle status", status, Ok(expected))
    });
    let mut state = ProcessJournalMaterializedState::default();

    assert_state_transition_table(&mut state, cases, |state, status| {
        *state = ProcessJournalMaterializedState::default();
        let process_id = ProcessId::new();
        let mut imported = snapshot(process_id, status, scope("import"));
        imported.journal_cursor = ProcessJournalCursor(0);
        state.import_deployed_snapshot(imported);
        state.import_deployed_snapshot(snapshot(process_id, status, scope("import")));
        Ok::<_, &'static str>(
            state
                .journal
                .last()
                .expect("import emits a journal entry")
                .kind,
        )
    });

    assert_eq!(state.processes.len(), 1);
    assert_eq!(state.journal.len(), 1);
    assert!(state.legacy_imported);
}

#[derive(Debug, Clone, Copy)]
struct ControlCase {
    from: ProcessLifecycleStatus,
    action: ProcessControlAction,
}

#[test]
fn control_transition_table_covers_every_action_class() {
    let cases = [
        StateTransitionCase::new(
            "resume suspended",
            ControlCase {
                from: ProcessLifecycleStatus::Suspended,
                action: ProcessControlAction::Resume,
            },
            Ok::<_, &'static str>((ProcessLifecycleStatus::Queued, true, false)),
        ),
        StateTransitionCase::new(
            "stop queued",
            ControlCase {
                from: ProcessLifecycleStatus::Queued,
                action: ProcessControlAction::Stop,
            },
            Ok((ProcessLifecycleStatus::Stopped, true, false)),
        ),
        StateTransitionCase::new(
            "cancel running",
            ControlCase {
                from: ProcessLifecycleStatus::Running,
                action: ProcessControlAction::Cancel,
            },
            Ok((ProcessLifecycleStatus::CancelRequested, true, false)),
        ),
        StateTransitionCase::new(
            "repeat cancel request",
            ControlCase {
                from: ProcessLifecycleStatus::CancelRequested,
                action: ProcessControlAction::Cancel,
            },
            Ok((ProcessLifecycleStatus::CancelRequested, false, false)),
        ),
        StateTransitionCase::new(
            "cancel queued",
            ControlCase {
                from: ProcessLifecycleStatus::Queued,
                action: ProcessControlAction::Cancel,
            },
            Ok((ProcessLifecycleStatus::Cancelled, true, false)),
        ),
        StateTransitionCase::new(
            "cancel terminal",
            ControlCase {
                from: ProcessLifecycleStatus::Completed,
                action: ProcessControlAction::Cancel,
            },
            Ok((ProcessLifecycleStatus::Completed, false, true)),
        ),
        StateTransitionCase::new(
            "kill queued",
            ControlCase {
                from: ProcessLifecycleStatus::Queued,
                action: ProcessControlAction::Kill,
            },
            Ok((ProcessLifecycleStatus::Killed, true, false)),
        ),
        StateTransitionCase::new(
            "stop terminal",
            ControlCase {
                from: ProcessLifecycleStatus::Failed,
                action: ProcessControlAction::Stop,
            },
            Ok((ProcessLifecycleStatus::Failed, false, true)),
        ),
        StateTransitionCase::new(
            "kill terminal",
            ControlCase {
                from: ProcessLifecycleStatus::Cancelled,
                action: ProcessControlAction::Kill,
            },
            Ok((ProcessLifecycleStatus::Cancelled, false, true)),
        ),
    ];
    let mut state = ProcessJournalMaterializedState::default();

    assert_state_transition_table(&mut state, cases, |state, case| {
        *state = ProcessJournalMaterializedState::default();
        let process_id = ProcessId::new();
        let request_scope = scope("control");
        state.processes.insert(
            process_id,
            snapshot(process_id, case.from, request_scope.clone()),
        );
        let outcome = state
            .apply_control(ProcessControlMutation {
                scope: request_scope,
                process_id,
                action: case.action,
                operation_id: None,
                expected_cursor: Some(ProcessJournalCursor(1)),
                reason: Some("state-table".to_string()),
                checkpoint_ref: None,
                metadata: Some(json!({"case": "control"})),
            })
            .map_err(error_class)?;
        match outcome {
            StoredCommandOutcome::Controlled(result) => {
                Ok((result.state.status, result.changed, result.already_terminal))
            }
            _ => panic!("control command returned a non-control outcome"),
        }
    });
}

#[test]
fn lifecycle_transition_cross_product_matches_the_complete_state_graph() {
    let statuses = [
        ProcessLifecycleStatus::Queued,
        ProcessLifecycleStatus::Running,
        ProcessLifecycleStatus::Suspended,
        ProcessLifecycleStatus::StopRequested,
        ProcessLifecycleStatus::CancelRequested,
        ProcessLifecycleStatus::Stopped,
        ProcessLifecycleStatus::Cancelled,
        ProcessLifecycleStatus::Completed,
        ProcessLifecycleStatus::Failed,
        ProcessLifecycleStatus::Killed,
        ProcessLifecycleStatus::RecoveryRequired,
    ];
    let mut cases = Vec::new();
    for from in statuses {
        for to in statuses {
            let explicitly_valid = matches!(
                (from, to),
                (
                    ProcessLifecycleStatus::Queued,
                    ProcessLifecycleStatus::Running
                ) | (
                    ProcessLifecycleStatus::Suspended,
                    ProcessLifecycleStatus::Queued
                ) | (
                    ProcessLifecycleStatus::Running,
                    ProcessLifecycleStatus::Suspended
                        | ProcessLifecycleStatus::Completed
                        | ProcessLifecycleStatus::Cancelled
                        | ProcessLifecycleStatus::Failed
                        | ProcessLifecycleStatus::Queued
                ) | (
                    ProcessLifecycleStatus::CancelRequested,
                    ProcessLifecycleStatus::Cancelled
                )
            );
            cases.push(StateTransitionCase::new(
                "lifecycle cross-product",
                (from, to),
                if from == to || explicitly_valid {
                    Ok(())
                } else {
                    Err("transition")
                },
            ));
        }
    }
    let mut state = ();

    assert_state_transition_table(&mut state, cases, |_, (from, to)| {
        ensure_transition(
            &snapshot(ProcessId::new(), from, scope("lifecycle-matrix")),
            to,
        )
        .map_err(error_class)
    });
}

#[test]
fn malformed_submission_lineage_is_rejected_without_mutation() {
    let root_scope = scope("lineage");
    let other_scope = scope("other-lineage");
    let parent_id = ProcessId::new();
    let root_id = ProcessId::new();
    let child_id = ProcessId::new();

    let mut state = ProcessJournalMaterializedState::default();
    state.processes.insert(
        parent_id,
        snapshot(
            parent_id,
            ProcessLifecycleStatus::Queued,
            root_scope.clone(),
        ),
    );

    let mut duplicate = submit_request(parent_id, root_scope.clone());
    assert_error(state.apply_submit(duplicate.clone()), "exists");

    duplicate.process_id = child_id;
    duplicate.parent_process_id = Some(parent_id);
    duplicate.root_process_id = Some(parent_id);
    duplicate.spawn_tree_descendant_cap = Some(2);
    duplicate.dependency = Some(ProcessDependencySubmission {
        dependent_process_id: root_id,
        root_process_id: parent_id,
        group_ref: None,
        metadata: serde_json::Value::Null,
    });
    assert_error(state.apply_submit(duplicate.clone()), "request");

    duplicate.dependency = Some(ProcessDependencySubmission {
        dependent_process_id: parent_id,
        root_process_id: root_id,
        group_ref: None,
        metadata: serde_json::Value::Null,
    });
    assert_error(state.apply_submit(duplicate.clone()), "request");

    duplicate.dependency = None;
    duplicate.parent_process_id = Some(root_id);
    assert_error(state.apply_submit(duplicate.clone()), "unknown");

    duplicate.parent_process_id = Some(parent_id);
    duplicate.scope = other_scope;
    assert_error(state.apply_submit(duplicate.clone()), "scope");

    duplicate.scope = root_scope.clone();
    duplicate.root_process_id = Some(root_id);
    assert_error(state.apply_submit(duplicate.clone()), "request");

    duplicate.root_process_id = Some(parent_id);
    duplicate.spawn_tree_descendant_cap = None;
    assert_error(state.apply_submit(duplicate), "request");

    let mut malformed_root = submit_request(ProcessId::new(), root_scope);
    malformed_root.root_process_id = Some(parent_id);
    assert_error(state.apply_submit(malformed_root), "request");
    assert_eq!(state.processes.len(), 1);
}

#[test]
fn defensive_transition_matrix_fails_closed_without_corrupting_state() {
    let request_scope = scope("defensive");
    let other_scope = scope("defensive-other");
    let root_id = ProcessId::new();
    let child_id = ProcessId::new();
    let mut state = ProcessJournalMaterializedState::default();

    let mut imported = snapshot(
        ProcessId::new(),
        ProcessLifecycleStatus::Completed,
        request_scope.clone(),
    );
    imported.journal_cursor = ProcessJournalCursor(50);
    state.import_deployed_snapshot(imported);
    let mut colliding = snapshot(
        ProcessId::new(),
        ProcessLifecycleStatus::Failed,
        request_scope.clone(),
    );
    colliding.journal_cursor = ProcessJournalCursor(50);
    state.import_deployed_snapshot(colliding);

    state.processes.insert(
        root_id,
        snapshot(
            root_id,
            ProcessLifecycleStatus::Queued,
            request_scope.clone(),
        ),
    );
    let mut exclusive = submit_request(ProcessId::new(), request_scope.clone());
    exclusive.exclusive_within_scope = true;
    assert_error(state.apply_submit(exclusive), "active");

    state.tree_reservations.insert(
        root_id,
        ProcessTreeReservation {
            root_process_id: root_id,
            descendant_count: 1,
            released_processes: HashSet::new(),
        },
    );
    let mut over_cap = submit_request(child_id, request_scope.clone());
    over_cap.parent_process_id = Some(root_id);
    over_cap.root_process_id = Some(root_id);
    over_cap.spawn_tree_descendant_cap = Some(1);
    assert_error(state.apply_submit(over_cap), "capacity");

    assert_error(
        state.apply_control(ProcessControlMutation {
            scope: other_scope.clone(),
            process_id: root_id,
            action: ProcessControlAction::Stop,
            operation_id: None,
            expected_cursor: None,
            reason: None,
            checkpoint_ref: None,
            metadata: None,
        }),
        "unknown",
    );

    let unknown_dependency = OpenProcessDependencyRequest {
        dependent_process_id: ProcessId::new(),
        dependency_process_id: child_id,
        root_process_id: root_id,
        scope: request_scope.clone(),
        group_ref: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };
    assert_error(state.apply_open_dependency(unknown_dependency), "unknown");
    let mut open = OpenProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        root_process_id: root_id,
        scope: other_scope,
        group_ref: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };
    assert_error(state.apply_open_dependency(open.clone()), "scope");
    open.scope = request_scope.clone();
    open.root_process_id = ProcessId::new();
    assert_error(state.apply_open_dependency(open.clone()), "request");
    open.root_process_id = root_id;
    state
        .apply_open_dependency(open)
        .expect("open dependency for close guards");
    assert_error(
        state.apply_close_dependency(
            CloseProcessDependencyRequest {
                dependent_process_id: root_id,
                dependency_process_id: child_id,
                scope: scope("close-other"),
                closed_at: Utc::now(),
            },
            true,
        ),
        "scope",
    );

    state.tree_reservations.insert(
        root_id,
        ProcessTreeReservation {
            root_process_id: root_id,
            descendant_count: 0,
            released_processes: HashSet::new(),
        },
    );
    assert_error(
        state.apply_close_dependency(
            CloseProcessDependencyRequest {
                dependent_process_id: root_id,
                dependency_process_id: child_id,
                scope: request_scope.clone(),
                closed_at: Utc::now(),
            },
            true,
        ),
        "request",
    );

    let checkpoint_id = ProcessCheckpointId::from_trusted("defensive-checkpoint");
    let checkpoint =
        |process_id, checkpoint_scope, payload: &[u8]| RecordProcessCheckpointRequest {
            checkpoint_id: checkpoint_id.clone(),
            process_id,
            scope: checkpoint_scope,
            state_ref: ProcessCheckpointRef::from_trusted("defensive-state"),
            payload: ProcessCheckpointPayload::new(payload.to_vec()).expect("payload"),
            created_at: Utc::now(),
            link_to_process: true,
            kind: None,
            metadata: serde_json::Value::Null,
        };
    assert_error(
        state.apply_checkpoint(checkpoint(
            ProcessId::new(),
            request_scope.clone(),
            b"missing",
        )),
        "unknown",
    );
    assert_error(
        state.apply_checkpoint(checkpoint(
            root_id,
            scope("checkpoint-other"),
            b"wrong-scope",
        )),
        "unknown",
    );
    state
        .apply_checkpoint(checkpoint(root_id, request_scope.clone(), b"first"))
        .expect("first checkpoint");
    assert_error(
        state.apply_checkpoint(checkpoint(root_id, request_scope, b"conflict")),
        "request",
    );
    assert_eq!(
        state
            .process_mut(ProcessId::new())
            .map(|_| ())
            .map_err(error_class),
        Err("unknown")
    );
}

#[test]
fn tree_and_dependency_transition_boundaries_preserve_accounting() {
    let request_scope = scope("dependency");
    let other_scope = scope("other-dependency");
    let root_id = ProcessId::new();
    let child_id = ProcessId::new();
    let missing_id = ProcessId::new();
    let mut state = ProcessJournalMaterializedState::default();
    state.processes.insert(
        root_id,
        snapshot(
            root_id,
            ProcessLifecycleStatus::Running,
            request_scope.clone(),
        ),
    );

    let zero_reservation = state.apply_reserve_tree(ReserveProcessTreeRequest {
        scope: request_scope.clone(),
        root_process_id: root_id,
        delta: 0,
        cap: 1,
    });
    assert_error(zero_reservation, "request");

    state.tree_reservations.insert(
        root_id,
        ProcessTreeReservation {
            root_process_id: root_id,
            descendant_count: u64::MAX,
            released_processes: HashSet::new(),
        },
    );
    assert_error(
        state.apply_reserve_tree(ReserveProcessTreeRequest {
            scope: request_scope.clone(),
            root_process_id: root_id,
            delta: 1,
            cap: u32::MAX,
        }),
        "capacity",
    );
    state.tree_reservations.clear();

    let missing = state
        .apply_settle_dependency(crate::SettleProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: missing_id,
            scope: request_scope.clone(),
            terminal: ProcessTerminalEvidence {
                status: ProcessLifecycleStatus::Completed,
                output_bytes: None,
                sanitized_reason: None,
            },
            settled_at: Utc::now(),
        })
        .expect("missing dependency settlement is idempotent");
    assert!(matches!(missing, StoredCommandOutcome::Dependency(None)));

    let open = crate::OpenProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        root_process_id: root_id,
        scope: request_scope.clone(),
        group_ref: Some("group".to_string()),
        created_at: Utc::now(),
        metadata: json!({"edge": true}),
    };
    state
        .apply_open_dependency(open.clone())
        .expect("open dependency");
    state
        .apply_open_dependency(open)
        .expect("opening the same dependency replays");

    let non_terminal = state.apply_settle_dependency(crate::SettleProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        scope: request_scope.clone(),
        terminal: ProcessTerminalEvidence {
            status: ProcessLifecycleStatus::Running,
            output_bytes: None,
            sanitized_reason: None,
        },
        settled_at: Utc::now(),
    });
    assert_error(non_terminal, "request");

    let wrong_scope = state.apply_settle_dependency(crate::SettleProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        scope: other_scope,
        terminal: ProcessTerminalEvidence {
            status: ProcessLifecycleStatus::Completed,
            output_bytes: None,
            sanitized_reason: None,
        },
        settled_at: Utc::now(),
    });
    assert_error(wrong_scope, "scope");

    let close = crate::CloseProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        scope: request_scope.clone(),
        closed_at: Utc::now(),
    };
    assert_error(
        state.apply_close_dependency(close.clone(), false),
        "request",
    );

    state
        .apply_settle_dependency(crate::SettleProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: child_id,
            scope: request_scope,
            terminal: ProcessTerminalEvidence {
                status: ProcessLifecycleStatus::Completed,
                output_bytes: Some(7),
                sanitized_reason: None,
            },
            settled_at: Utc::now(),
        })
        .expect("settle dependency");
    state
        .apply_close_dependency(close.clone(), false)
        .expect("consume dependency");
    let replay = state
        .apply_close_dependency(close, false)
        .expect("consume replay");
    assert!(matches!(
        replay,
        StoredCommandOutcome::Dependency(Some(record))
            if record.state == ProcessDependencyState::Consumed
    ));

    let missing_close = state
        .apply_close_dependency(
            crate::CloseProcessDependencyRequest {
                dependent_process_id: root_id,
                dependency_process_id: missing_id,
                scope: scope("dependency"),
                closed_at: Utc::now(),
            },
            true,
        )
        .expect("missing close is idempotent");
    assert!(matches!(
        missing_close,
        StoredCommandOutcome::Dependency(None)
    ));
}

#[test]
fn replay_imports_and_bounded_idempotency_maps_cover_replacement_and_eviction() {
    let process_id = ProcessId::new();
    let request_scope = scope("replay");
    let queued = snapshot(
        process_id,
        ProcessLifecycleStatus::Queued,
        request_scope.clone(),
    );
    let mut state = ProcessJournalMaterializedState::default();

    state
        .import_deployed_submit_idempotency("submit-op", queued.clone())
        .expect("import submission replay");
    state.import_deployed_control_idempotency("stop", "control-op", queued.clone());
    state.import_deployed_checkpoint(crate::ProcessCheckpointRecord {
        checkpoint_id: ProcessCheckpointId::from_trusted("checkpoint-import"),
        process_id,
        scope: request_scope,
        state_ref: ProcessCheckpointRef::from_trusted("state-ref"),
        payload: ProcessCheckpointPayload::new(Vec::new()).expect("checkpoint payload"),
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    });
    state.import_deployed_tree_reservation(ProcessTreeReservation {
        root_process_id: process_id,
        descendant_count: 1,
        released_processes: HashSet::new(),
    });

    let control_key = state
        .control_idempotency
        .keys()
        .next()
        .expect("control replay key")
        .clone();
    state.remember_control_result(
        Some(control_key),
        ProcessControlResult {
            state: queued.clone(),
            changed: true,
            already_terminal: false,
        },
    );
    let submission_key = state
        .submission_idempotency
        .keys()
        .next()
        .expect("submission replay key")
        .clone();
    state.remember_submission_result(Some(submission_key), queued.clone());

    state.control_idempotency.clear();
    state.control_idempotency_order.clear();
    for index in 0..MAX_IDEMPOTENCY_RECORDS {
        let key = format!("control-{index}");
        state.control_idempotency_order.push_back(key.clone());
        state.control_idempotency.insert(
            key,
            ProcessControlResult {
                state: queued.clone(),
                changed: false,
                already_terminal: false,
            },
        );
    }
    state.remember_control_result(
        Some("control-new".to_string()),
        ProcessControlResult {
            state: queued.clone(),
            changed: false,
            already_terminal: false,
        },
    );
    assert_eq!(state.control_idempotency.len(), MAX_IDEMPOTENCY_RECORDS);
    assert!(!state.control_idempotency.contains_key("control-0"));

    state.submission_idempotency.clear();
    state.submission_idempotency_order.clear();
    for index in 0..MAX_IDEMPOTENCY_RECORDS {
        let key = format!("submit-{index}");
        state.submission_idempotency_order.push_back(key.clone());
        state.submission_idempotency.insert(key, queued.clone());
    }
    state.remember_submission_result(Some("submit-new".to_string()), queued);
    assert_eq!(state.submission_idempotency.len(), MAX_IDEMPOTENCY_RECORDS);
    assert!(!state.submission_idempotency.contains_key("submit-0"));
    assert!(state.legacy_imported);
}

#[test]
fn command_dispatch_covers_submit_claim_heartbeat_transition_and_control_replay() {
    let request_scope = scope("dispatch");
    let process_id = ProcessId::new();
    let checkpoint_id = ProcessCheckpointId::from_trusted("dispatch-checkpoint");
    let checkpoint_ref = ProcessCheckpointRef::from_trusted("dispatch-state");
    let checkpoint = crate::RecordProcessCheckpointRequest {
        checkpoint_id: checkpoint_id.clone(),
        process_id,
        scope: request_scope.clone(),
        state_ref: checkpoint_ref,
        payload: ProcessCheckpointPayload::new(b"checkpoint".to_vec()).expect("payload"),
        created_at: Utc::now(),
        link_to_process: true,
        kind: None,
        metadata: json!({"checkpoint": true}),
    };
    let mut submission = submit_request(process_id, request_scope.clone());
    submission.operation_id = Some(ProcessOperationId::from_trusted("dispatch-submit"));
    submission.checkpoint_ref = Some(ProcessCheckpointRef::from_trusted(
        checkpoint_id.as_str().to_string(),
    ));
    let mut state = ProcessJournalMaterializedState::default();

    let bad_checkpoint = crate::RecordProcessCheckpointRequest {
        process_id: ProcessId::new(),
        ..checkpoint.clone()
    };
    assert_error(
        state.apply_command(StoredProcessCommand::SubmitWithCheckpoint {
            request: Box::new(submission.clone()),
            checkpoint: Box::new(bad_checkpoint),
        }),
        "request",
    );

    let submitted = state
        .apply_command(StoredProcessCommand::SubmitWithCheckpoint {
            request: Box::new(submission.clone()),
            checkpoint: Box::new(checkpoint),
        })
        .expect("submit with checkpoint");
    assert!(matches!(
        submitted,
        StoredCommandOutcome::Submitted(_, true)
    ));
    let replay = state
        .apply_command(StoredProcessCommand::Submit(Box::new(submission)))
        .expect("submission replay");
    assert!(matches!(replay, StoredCommandOutcome::Submitted(_, false)));

    let worker_id = ProcessWorkerId::from_trusted("dispatch-worker");
    let claimed = state
        .apply_command(StoredProcessCommand::Claim {
            request: ClaimProcessesRequest {
                worker_id: worker_id.clone(),
                scope_filter: Some(request_scope.clone()),
                process_id_filter: Some(process_id),
                process_kind_filter: Some(ProcessKind::Internal),
                max_processes: 1,
            },
            now: Utc::now(),
            lease_duration_millis: 5_000,
            lease_nonce: ProcessId::new(),
            limits: crate::ProcessConcurrencyLimits::default(),
        })
        .expect("claim");
    let StoredCommandOutcome::Claimed(mut claims) = claimed else {
        panic!("claim command returned wrong outcome");
    };
    let claim = claims.pop().expect("one claim");
    let lease = ProcessLeaseRequest {
        process_id,
        worker_id,
        lease_token: claim.lease_token,
    };

    state
        .apply_command(StoredProcessCommand::Heartbeat {
            request: lease.clone(),
            now: Utc::now(),
            lease_duration_millis: 5_000,
        })
        .expect("heartbeat");
    state
        .apply_command(StoredProcessCommand::LeasedTransition {
            request: lease,
            mutation: ProcessTransitionMutation {
                status: ProcessLifecycleStatus::Suspended,
                kind: ProcessJournalKind::Suspended,
                suspension: None,
                checkpoint_ref: None,
                failure: None,
                failure_recovery: crate::ProcessFailureRecovery::Terminal,
                metadata: Some(json!({"suspended": true})),
            },
        })
        .expect("suspend");

    let operation_id = ProcessOperationId::from_trusted("dispatch-resume");
    let resume = ProcessControlMutation {
        scope: request_scope.clone(),
        process_id,
        action: ProcessControlAction::Resume,
        operation_id: Some(operation_id.clone()),
        expected_cursor: None,
        reason: None,
        checkpoint_ref: None,
        metadata: None,
    };
    state
        .apply_command(StoredProcessCommand::Control(resume.clone()))
        .expect("resume");
    state
        .apply_command(StoredProcessCommand::Control(resume.clone()))
        .expect("resume replay");
    let mut wrong_scope = resume;
    wrong_scope.scope = scope("dispatch-other");
    assert_error(
        state.apply_command(StoredProcessCommand::Control(wrong_scope)),
        "unknown",
    );

    let stale = ProcessControlMutation {
        scope: request_scope,
        process_id,
        action: ProcessControlAction::Stop,
        operation_id: None,
        expected_cursor: Some(ProcessJournalCursor(1)),
        reason: None,
        checkpoint_ref: None,
        metadata: None,
    };
    assert_error(
        state.apply_command(StoredProcessCommand::Control(stale)),
        "stale",
    );
}

#[test]
fn command_dispatch_covers_tree_dependency_checkpoint_and_legacy_import() {
    let request_scope = scope("dispatch-tree");
    let root_id = ProcessId::new();
    let child_id = ProcessId::new();
    let imported = ProcessJournalMaterializedState {
        next_cursor: 9,
        ..ProcessJournalMaterializedState::default()
    };
    let mut state = ProcessJournalMaterializedState::default();
    let imported_outcome = state
        .apply_command(StoredProcessCommand::ImportLegacyState(Box::new(imported)))
        .expect("import legacy state");
    assert!(matches!(imported_outcome, StoredCommandOutcome::Imported));
    assert!(state.legacy_imported);
    state
        .apply_command(StoredProcessCommand::ImportLegacyState(Box::default()))
        .expect("legacy import replay");

    state
        .apply_command(StoredProcessCommand::Submit(Box::new(submit_request(
            root_id,
            request_scope.clone(),
        ))))
        .expect("submit root");
    let mut child = submit_request(child_id, request_scope.clone());
    child.parent_process_id = Some(root_id);
    child.root_process_id = Some(root_id);
    child.spawn_tree_descendant_cap = Some(2);
    child.operation_id = Some(ProcessOperationId::from_trusted("dispatch-tree-child"));
    child.input = Some(ProcessInputSubmission {
        input_ref: ProcessInputRef::from_trusted("dispatch-input"),
        payload: ProcessInputPayload::new(b"input".to_vec()).expect("input payload"),
    });
    child.dependency = Some(ProcessDependencySubmission {
        dependent_process_id: root_id,
        root_process_id: root_id,
        group_ref: Some("dispatch-group".to_string()),
        metadata: json!({"dependency": true}),
    });
    state
        .apply_command(StoredProcessCommand::Submit(Box::new(child)))
        .expect("submit child with atomic dependency");
    assert!(state.inputs.contains_key(&child_id));
    assert_eq!(
        state
            .tree_reservations
            .get(&root_id)
            .expect("tree reservation")
            .descendant_count,
        1
    );

    let settlement = SettleProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        scope: request_scope.clone(),
        terminal: ProcessTerminalEvidence {
            status: ProcessLifecycleStatus::Completed,
            output_bytes: Some(5),
            sanitized_reason: None,
        },
        settled_at: Utc::now(),
    };
    state
        .apply_command(StoredProcessCommand::SettleDependency(settlement.clone()))
        .expect("settle dependency");
    state
        .apply_command(StoredProcessCommand::SettleDependency(settlement))
        .expect("settlement replay");
    let close = CloseProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: child_id,
        scope: request_scope.clone(),
        closed_at: Utc::now(),
    };
    state
        .apply_command(StoredProcessCommand::ConsumeDependency(close.clone()))
        .expect("consume dependency");
    state
        .apply_command(StoredProcessCommand::AbandonDependency(close))
        .expect("closed dependency replay");

    let explicit_child = ProcessId::new();
    state
        .apply_command(StoredProcessCommand::OpenDependency(
            OpenProcessDependencyRequest {
                dependent_process_id: root_id,
                dependency_process_id: explicit_child,
                root_process_id: root_id,
                scope: request_scope.clone(),
                group_ref: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            },
        ))
        .expect("open explicit dependency");
    state
        .apply_command(StoredProcessCommand::AbandonDependency(
            CloseProcessDependencyRequest {
                dependent_process_id: root_id,
                dependency_process_id: explicit_child,
                scope: request_scope.clone(),
                closed_at: Utc::now(),
            },
        ))
        .expect("abandon explicit dependency");

    state
        .apply_command(StoredProcessCommand::ReserveTree(
            ReserveProcessTreeRequest {
                scope: request_scope.clone(),
                root_process_id: root_id,
                delta: 2,
                cap: 3,
            },
        ))
        .expect("reserve tree");
    assert_error(
        state.apply_command(StoredProcessCommand::ReleaseTree(
            ReleaseProcessTreeRequest {
                scope: request_scope.clone(),
                root_process_id: root_id,
                delta: 3,
                idempotency_process_id: ProcessId::new(),
            },
        )),
        "request",
    );
    let release_id = ProcessId::new();
    let release = ReleaseProcessTreeRequest {
        scope: request_scope.clone(),
        root_process_id: root_id,
        delta: 2,
        idempotency_process_id: release_id,
    };
    state
        .apply_command(StoredProcessCommand::ReleaseTree(release.clone()))
        .expect("release tree");
    state
        .apply_command(StoredProcessCommand::ReleaseTree(release))
        .expect("release replay without a reservation");
    state
        .apply_command(StoredProcessCommand::PruneTree(
            PruneReleasedProcessRequest {
                scope: request_scope.clone(),
                root_process_id: root_id,
                process_id: release_id,
            },
        ))
        .expect("prune release marker");
    state
        .apply_command(StoredProcessCommand::PruneTree(
            PruneReleasedProcessRequest {
                scope: request_scope.clone(),
                root_process_id: ProcessId::new(),
                process_id: release_id,
            },
        ))
        .expect("pruning a missing root is idempotent");

    let checkpoint = RecordProcessCheckpointRequest {
        checkpoint_id: ProcessCheckpointId::from_trusted("dispatch-tree-checkpoint"),
        process_id: root_id,
        scope: request_scope,
        state_ref: ProcessCheckpointRef::from_trusted("dispatch-tree-state"),
        payload: ProcessCheckpointPayload::new(b"state".to_vec()).expect("checkpoint payload"),
        created_at: Utc::now(),
        link_to_process: true,
        kind: None,
        metadata: serde_json::Value::Null,
    };
    state
        .apply_command(StoredProcessCommand::RecordCheckpoint(checkpoint.clone()))
        .expect("record checkpoint");
    state
        .apply_command(StoredProcessCommand::RecordCheckpoint(checkpoint))
        .expect("checkpoint replay");
}

/// One row of the lease-recovery matrix.
struct RecoveryCase {
    label: &'static str,
    status: ProcessLifecycleStatus,
    checkpointed: bool,
    checkpoint_kind: Option<ProcessCheckpointKind>,
    claim_count: u64,
    /// How long before `now` the lease expired.
    expired_for: chrono::Duration,
    /// `None` means recovery must leave the process alone.
    expected: Option<(ProcessLifecycleStatus, Option<&'static str>)>,
}

#[test]
fn expired_lease_recovery_covers_cancel_requeue_grace_and_bounded_failure_states() {
    const LEASE_TTL: chrono::Duration = chrono::Duration::seconds(90);
    let request_scope = scope("recovery");
    let now = Utc::now();
    let worker_id = ProcessWorkerId::from_trusted("recovery-worker");
    let past_grace = LEASE_TTL + chrono::Duration::seconds(1);
    let within_grace = chrono::Duration::seconds(1);
    let cases = [
        RecoveryCase {
            label: "cancel-requested is cancelled as soon as the lease expires",
            status: ProcessLifecycleStatus::CancelRequested,
            checkpointed: false,
            checkpoint_kind: None,
            claim_count: 1,
            expired_for: within_grace,
            expected: Some((ProcessLifecycleStatus::Cancelled, None)),
        },
        RecoveryCase {
            label: "checkpointless work is requeued immediately, without a grace window",
            status: ProcessLifecycleStatus::Running,
            checkpointed: false,
            checkpoint_kind: None,
            claim_count: 1,
            expired_for: within_grace,
            expected: Some((ProcessLifecycleStatus::Queued, None)),
        },
        RecoveryCase {
            label: "a side-effect checkpoint is never resumed — resuming would re-run the effect",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: Some(ProcessCheckpointKind::BeforeSideEffect),
            claim_count: 1,
            expired_for: past_grace,
            expected: Some((ProcessLifecycleStatus::Failed, Some("lease_expired"))),
        },
        RecoveryCase {
            label: "an unknown checkpoint kind fails closed like a side-effect checkpoint",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: None,
            claim_count: 1,
            expired_for: past_grace,
            expected: Some((ProcessLifecycleStatus::Failed, Some("lease_expired"))),
        },
        RecoveryCase {
            label: "a before-model checkpoint inside the grace window is left for a later sweep",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: Some(ProcessCheckpointKind::BeforeModel),
            claim_count: 1,
            expired_for: within_grace,
            expected: None,
        },
        RecoveryCase {
            label: "a before-model checkpoint past the grace window is requeued",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: Some(ProcessCheckpointKind::BeforeModel),
            claim_count: 1,
            expired_for: past_grace,
            expected: Some((ProcessLifecycleStatus::Queued, None)),
        },
        RecoveryCase {
            label: "a before-block checkpoint past the grace window is requeued",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: Some(ProcessCheckpointKind::BeforeBlock),
            claim_count: 1,
            expired_for: past_grace,
            expected: Some((ProcessLifecycleStatus::Queued, None)),
        },
        RecoveryCase {
            label: "checkpointed requeue shares the crash-reclaim budget and fails when spent",
            status: ProcessLifecycleStatus::Running,
            checkpointed: true,
            checkpoint_kind: Some(ProcessCheckpointKind::BeforeModel),
            claim_count: MAX_CRASH_RECOVERY_RECLAIMS,
            expired_for: past_grace,
            expected: Some((ProcessLifecycleStatus::Failed, Some("lease_expired"))),
        },
        RecoveryCase {
            label: "checkpointless requeue keeps its own bounded budget",
            status: ProcessLifecycleStatus::Running,
            checkpointed: false,
            checkpoint_kind: None,
            claim_count: MAX_CRASH_RECOVERY_RECLAIMS,
            expired_for: within_grace,
            expected: Some((
                ProcessLifecycleStatus::Failed,
                Some("crash_retry_exhausted"),
            )),
        },
    ];
    let mut state = ProcessJournalMaterializedState::default();
    let mut process_ids = Vec::new();
    for case in &cases {
        let process_id = ProcessId::new();
        process_ids.push(process_id);
        let mut process = snapshot(process_id, case.status, request_scope.clone());
        process.checkpoint_ref = case
            .checkpointed
            .then(|| ProcessCheckpointRef::from_trusted("checkpoint"));
        process.checkpoint_kind = case.checkpoint_kind;
        process.lease = Some(ProcessLeaseSnapshot {
            worker_id: worker_id.clone(),
            lease_token: ProcessLeaseToken::from_trusted(process_id.to_string()),
            lease_expires_at: Some(now - case.expired_for),
            last_heartbeat_at: Some(now - case.expired_for - chrono::Duration::seconds(1)),
            claim_count: case.claim_count,
        });
        state.processes.insert(process_id, process);
    }

    let recovered = state
        .apply_command(StoredProcessCommand::RecoverExpired {
            request: RecoverExpiredProcessLeasesRequest {
                now,
                scope_filter: Some(request_scope),
                process_kind_filter: Some(ProcessKind::Internal),
            },
            lease_duration_millis: u64::try_from(LEASE_TTL.num_milliseconds())
                .expect("lease ttl millis"),
        })
        .expect("recover expired leases");
    let StoredCommandOutcome::Recovered(response) = recovered else {
        panic!("recovery command returned wrong outcome");
    };
    let expected_recovered = cases.iter().filter(|case| case.expected.is_some()).count();
    assert_eq!(response.recovered.len(), expected_recovered);
    for (process_id, case) in process_ids.into_iter().zip(&cases) {
        let recovered = state.processes.get(&process_id).expect("recovered process");
        match case.expected {
            Some((expected_status, failure)) => {
                assert_eq!(recovered.status, expected_status, "{}", case.label);
                assert_eq!(
                    recovered.failure.as_ref().map(SanitizedFailure::category),
                    failure,
                    "{}",
                    case.label
                );
                assert!(recovered.lease.is_none(), "{}", case.label);
                if expected_status == ProcessLifecycleStatus::Queued {
                    assert_eq!(
                        recovered.crash_reclaim_count, case.claim_count,
                        "requeue must carry the claim count forward as the reclaim budget: {}",
                        case.label
                    );
                }
            }
            None => {
                assert_eq!(recovered.status, case.status, "{}", case.label);
                assert!(
                    recovered.lease.is_some(),
                    "a process held for the grace window keeps its lease: {}",
                    case.label
                );
                assert!(recovered.failure.is_none(), "{}", case.label);
            }
        }
    }
}

#[test]
fn runner_failure_recovery_covers_terminal_checkpoint_cancel_and_bounded_redrive_states() {
    let request_scope = scope("runner-failure-recovery");
    let worker_id = ProcessWorkerId::from_trusted("runner-failure-worker");
    let cases = [
        (
            "terminal",
            ProcessLifecycleStatus::Running,
            None,
            1,
            crate::ProcessFailureRecovery::Terminal,
            ProcessLifecycleStatus::Failed,
            Some("runner_failure"),
            0,
        ),
        (
            "checkpointless-redrive",
            ProcessLifecycleStatus::Running,
            None,
            1,
            crate::ProcessFailureRecovery::RedriveIfCheckpointless,
            ProcessLifecycleStatus::Queued,
            None,
            1,
        ),
        (
            "checkpointed",
            ProcessLifecycleStatus::Running,
            Some(ProcessCheckpointRef::from_trusted("checkpoint")),
            1,
            crate::ProcessFailureRecovery::RedriveIfCheckpointless,
            ProcessLifecycleStatus::Failed,
            Some("runner_failure"),
            0,
        ),
        (
            "bounded",
            ProcessLifecycleStatus::Running,
            None,
            MAX_CRASH_RECOVERY_RECLAIMS,
            crate::ProcessFailureRecovery::RedriveIfCheckpointless,
            ProcessLifecycleStatus::Failed,
            Some("runner_failure"),
            0,
        ),
        (
            "cancel-requested",
            ProcessLifecycleStatus::CancelRequested,
            None,
            1,
            crate::ProcessFailureRecovery::RedriveIfCheckpointless,
            ProcessLifecycleStatus::Cancelled,
            None,
            0,
        ),
    ];

    for (
        label,
        status,
        checkpoint_ref,
        claim_count,
        recovery,
        expected_status,
        expected_failure,
        expected_reclaim_count,
    ) in cases
    {
        let process_id = ProcessId::new();
        let lease_token = ProcessLeaseToken::from_trusted(format!("{label}-lease"));
        let mut process = snapshot(process_id, status, request_scope.clone());
        process.checkpoint_ref = checkpoint_ref;
        process.lease = Some(ProcessLeaseSnapshot {
            worker_id: worker_id.clone(),
            lease_token: lease_token.clone(),
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count,
        });
        let mut state = ProcessJournalMaterializedState::default();
        state.processes.insert(process_id, process);

        state
            .apply_command(StoredProcessCommand::LeasedTransition {
                request: ProcessLeaseRequest {
                    process_id,
                    worker_id: worker_id.clone(),
                    lease_token,
                },
                mutation: ProcessTransitionMutation {
                    status: ProcessLifecycleStatus::Failed,
                    kind: ProcessJournalKind::Failed,
                    suspension: None,
                    checkpoint_ref: None,
                    failure: Some(
                        ironclaw_host_api::turn::SanitizedFailure::new("runner_failure")
                            .expect("failure"),
                    ),
                    failure_recovery: recovery,
                    metadata: None,
                },
            })
            .unwrap_or_else(|error| panic!("{label}: transition failed: {error}"));

        let result = state.processes.get(&process_id).expect("process retained");
        assert_eq!(result.status, expected_status, "{label}");
        assert_eq!(
            result
                .failure
                .as_ref()
                .map(ironclaw_host_api::turn::SanitizedFailure::category),
            expected_failure,
            "{label}"
        );
        assert_eq!(
            result.crash_reclaim_count, expected_reclaim_count,
            "{label}"
        );
        assert!(result.lease.is_none(), "{label}");
    }
}

#[test]
fn failed_journal_projection_preserves_reason_detail_and_retryability() {
    let mut failed = snapshot(
        ProcessId::new(),
        ProcessLifecycleStatus::Failed,
        scope("failed-projection"),
    );
    failed.failure = Some(
        SanitizedFailure::new("provider_unavailable")
            .expect("failure")
            .with_detail("safe provider diagnostic"),
    );
    failed.checkpoint_ref = Some(ProcessCheckpointRef::from_trusted("retry-checkpoint"));

    let entry = ProcessJournalEntry::from_snapshot(
        &failed,
        ProcessJournalCursor(99),
        ProcessJournalKind::Failed,
    );

    assert_eq!(
        entry.sanitized_reason.as_deref(),
        Some("provider_unavailable")
    );
    assert_eq!(entry.detail.as_deref(), Some("safe provider diagnostic"));
    assert_eq!(entry.retryable, Some(true));
}
