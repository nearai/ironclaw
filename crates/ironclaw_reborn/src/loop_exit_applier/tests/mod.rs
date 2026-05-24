use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId, ThreadId};
use ironclaw_threads::{
    AppendAssistantDraftRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, ToolResultSafeSummary,
};
use ironclaw_turns::{
    LoopBlockedKind, LoopCheckpointKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopFailed,
    LoopFailureKind, LoopMessageRef, LoopResultRef, TurnCheckpointId, TurnError, TurnId, TurnRunId,
    TurnScope, TurnStateStore, TurnStatus,
};

use super::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    InMemoryLoopExitEvidencePort, LoopExitApplier, LoopExitEvidencePort,
    ThreadCheckpointLoopExitEvidencePort, verify_tool_result_ref,
};

mod support;

use support::*;

#[tokio::test]
async fn loop_exit_applier_rejects_driver_supplied_evidence_policy() {
    let fixture = Fixture::new(InMemoryLoopExitEvidencePort::new());
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn completed_exit_requires_same_run_finalized_message_ref() {
    let evidence = InMemoryLoopExitEvidencePort::all_verified();
    let fixture = Fixture::new(evidence);
    let final_checkpoint = TurnCheckpointId::new();
    let exit = completed_exit(
        vec![LoopMessageRef::new("msg:reply").expect("valid")],
        Some(final_checkpoint),
    );

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Completed);
}

#[tokio::test]
async fn no_reply_completion_requires_profile_permission() {
    let evidence = InMemoryLoopExitEvidencePort::all_verified();
    let fixture = Fixture::new(evidence);
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::NoReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn result_only_completion_uses_verified_result_refs_without_no_reply_permission() {
    let evidence = InMemoryLoopExitEvidencePort::all_verified();
    let fixture = Fixture::new(evidence);
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::ResultOnly,
        reply_message_refs: vec![],
        result_refs: vec![LoopResultRef::new("result:tool-output").expect("valid")],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Completed);
}

#[tokio::test]
async fn production_completed_exit_requires_final_checkpoint() {
    let mut claimed = claimed_run();
    claimed
        .resolved_run_profile
        .checkpoint_policy
        .require_final_checkpoint = true;
    let transition = Arc::new(RecordingTransitionPort::new());
    let applier = Arc::new(LoopExitApplier::new(
        transition,
        Arc::new(InMemoryLoopExitEvidencePort::all_verified()),
    ));
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn blocked_exit_requires_before_block_checkpoint() {
    let fixture = Fixture::new(InMemoryLoopExitEvidencePort::new());
    let exit = blocked_exit(LoopBlockedKind::Approval);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn cancelled_exit_requires_observed_cancel_input() {
    let fixture =
        Fixture::new(InMemoryLoopExitEvidencePort::new().with_final_checkpoint_verified(true));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "interrupted_unexpectedly"
    );
}

#[tokio::test]
async fn observed_host_cancellation_still_requires_final_checkpoint_when_configured() {
    let mut claimed = claimed_run();
    claimed
        .resolved_run_profile
        .checkpoint_policy
        .require_final_checkpoint = true;
    let transition = Arc::new(RecordingTransitionPort::new());
    let applier = Arc::new(LoopExitApplier::new(
        transition,
        Arc::new(InMemoryLoopExitEvidencePort::new().with_cancellation_observed(true)),
    ));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_durable_cancel_requested_run() {
    let claimed = claimed_run();
    let mut observed_state = claimed.state.clone();
    observed_state.status = TurnStatus::CancelRequested;
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(observed_state)),
        Arc::new(PanicLoopCheckpointStore),
    ));
    let applier = Arc::new(LoopExitApplier::new(transition.clone(), evidence));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Cancelled);
    assert_eq!(transition.apply_count(), 1);
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_durable_cancelled_run() {
    let claimed = claimed_run();
    let mut observed_state = claimed.state.clone();
    observed_state.status = TurnStatus::Cancelled;
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(observed_state)),
        Arc::new(PanicLoopCheckpointStore),
    ));
    let applier = Arc::new(LoopExitApplier::new(transition.clone(), evidence));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Cancelled);
    assert_eq!(transition.apply_count(), 1);
}

#[tokio::test]
async fn invalid_exit_after_before_side_effect_requires_recovery() {
    let evidence = InMemoryLoopExitEvidencePort::new()
        .with_latest_checkpoint_kind(Some(LoopCheckpointKind::BeforeSideEffect));
    let fixture = Fixture::new(evidence);
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn recovery_required_keeps_active_thread_lock() {
    assert!(TurnStatus::RecoveryRequired.keeps_active_lock());
}

#[tokio::test]
async fn loop_exit_events_hide_raw_diagnostics() {
    let evidence = InMemoryLoopExitEvidencePort::new();
    let fixture = Fixture::new(evidence);
    let exit = LoopExit::Failed(LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: None,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
    assert_eq!(
        fixture.transition.raw_failure_texts(),
        vec!["driver_protocol_violation"]
    );
}

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_agentless_completion_refs_explicitly() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let err = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            reply_message_refs: &[LoopMessageRef::new("msg:reply").expect("valid")],
            result_refs: &[],
        })
        .await
        .expect_err("agentless scope should be rejected explicitly");

    assert!(matches!(err, TurnError::InvalidRequest { .. }));
    assert!(err.to_string().contains("agent-scoped"));
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_result_refs_with_durable_reply_ref() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let run_id = TurnRunId::new();
    let draft = thread_service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: thread_scope.clone(),
            thread_id: turn_scope.thread_id.clone(),
            turn_run_id: run_id.to_string(),
            content: MessageContent::text("reply after tool"),
        })
        .await
        .expect("draft");
    thread_service
        .finalize_assistant_message(
            &thread_scope,
            &turn_scope.thread_id,
            draft.message_id,
            MessageContent::text("reply after tool"),
        )
        .await
        .expect("finalized");
    let result_ref = LoopResultRef::new("result:tool-output").expect("valid result ref");
    append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        run_id,
        result_ref.clone(),
    )
    .await;
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );
    let message_ref =
        LoopMessageRef::new(format!("msg:{}", draft.message_id)).expect("valid message ref");

    let verified = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &turn_scope,
            turn_id: TurnId::new(),
            run_id,
            reply_message_refs: &[message_ref],
            result_refs: &[result_ref],
        })
        .await
        .expect("completion evidence should verify durable reply refs");

    assert!(verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_missing_result_ref_records() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );
    let run_id = TurnRunId::new();
    let result_ref = LoopResultRef::new("result:tool-output").expect("valid result ref");

    let verified = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &turn_scope,
            turn_id: TurnId::new(),
            run_id,
            reply_message_refs: &[],
            result_refs: &[result_ref],
        })
        .await
        .expect("missing result evidence should fail closed without checkpoint I/O");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_result_only_completion_with_durable_tool_ref() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let run_id = TurnRunId::new();
    let result_ref = LoopResultRef::new("result:tool-output").expect("valid result ref");
    append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        run_id,
        result_ref.clone(),
    )
    .await;
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );

    let verified = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &turn_scope,
            turn_id: TurnId::new(),
            run_id,
            reply_message_refs: &[],
            result_refs: &[result_ref],
        })
        .await
        .expect("durable result evidence should verify without checkpoint I/O");

    assert!(verified);
}

// `libsql_thread_checkpoint_evidence_verifies_result_ref_after_reopen` and
// `postgres_thread_checkpoint_evidence_verifies_result_ref_after_reopen_when_configured`
// have been removed alongside the legacy `LibSqlSessionThreadService` and
// `PostgresSessionThreadService` consumer-store backends; durable restart
// coverage now lives in `ironclaw_filesystem` where the libSQL/Postgres
// `RootFilesystem` backends own that contract.

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_tool_result_message_as_reply_ref() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let run_id = TurnRunId::new();
    let result_ref = LoopResultRef::new("result:tool-output").expect("valid result ref");
    let tool_result_message = append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        run_id,
        result_ref,
    )
    .await;
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );
    let reply_message_ref = LoopMessageRef::new(format!("msg:{}", tool_result_message.message_id))
        .expect("valid message ref");

    let verified = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &turn_scope,
            turn_id: TurnId::new(),
            run_id,
            reply_message_refs: &[reply_message_ref],
            result_refs: &[],
        })
        .await
        .expect("tool result message must not satisfy reply evidence");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_isolates_same_result_ref_across_runs() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let first_run_id = TurnRunId::new();
    let second_run_id = TurnRunId::new();
    let result_ref = LoopResultRef::new("result:shared-output").expect("valid result ref");
    let first_message = append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        first_run_id,
        result_ref.clone(),
    )
    .await;
    let second_message = append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        second_run_id,
        result_ref.clone(),
    )
    .await;
    assert_ne!(first_message.message_id, second_message.message_id);

    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );

    for run_id in [first_run_id, second_run_id] {
        let verified = evidence
            .verify_completion_refs(CompletionEvidenceRequest {
                scope: &turn_scope,
                turn_id: TurnId::new(),
                run_id,
                reply_message_refs: &[],
                result_refs: std::slice::from_ref(&result_ref),
            })
            .await
            .expect("same result ref should verify only against same-run durable evidence");

        assert!(verified);
    }
}

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_wrong_run_and_malformed_result_ref_records() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let thread_scope = ThreadScope {
        tenant_id: turn_scope.tenant_id.clone(),
        agent_id: turn_scope.agent_id.clone().expect("agent id"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(turn_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let expected_run_id = TurnRunId::new();
    let wrong_run_result = LoopResultRef::new("result:wrong-run").expect("valid result ref");
    append_tool_result_reference(
        thread_service.as_ref(),
        thread_scope.clone(),
        turn_scope.thread_id.clone(),
        TurnRunId::new(),
        wrong_run_result.clone(),
    )
    .await;
    let malformed_result = LoopResultRef::new("result:malformed").expect("valid result ref");
    let unsafe_summary_result =
        LoopResultRef::new("result:unsafe-summary").expect("valid result ref");
    let forged_history = {
        let mut history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: turn_scope.thread_id.clone(),
            })
            .await
            .expect("history");
        let next_sequence = history
            .messages
            .iter()
            .map(|message| message.sequence)
            .max()
            .unwrap_or(0)
            + 1;
        history.messages.push(ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: turn_scope.thread_id.clone(),
            sequence: next_sequence,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(expected_run_id.to_string()),
            tool_result_ref: Some(malformed_result.as_str().to_string()),
            tool_result_provider_call: None,
            content: Some("not-json".to_string()),
            redaction_ref: None,
        });
        history.messages.push(ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: turn_scope.thread_id.clone(),
            sequence: next_sequence + 1,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(expected_run_id.to_string()),
            tool_result_ref: Some(unsafe_summary_result.as_str().to_string()),
            tool_result_provider_call: None,
            content: Some(format!(
                r#"{{"version":1,"result_ref":"{}","safe_summary":"raw tool input includes secret"}}"#,
                unsafe_summary_result.as_str()
            )),
            redaction_ref: None,
        });
        history
    };
    assert!(ToolResultSafeSummary::new("raw tool input includes secret").is_err());
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        thread_scope,
    );

    let verified = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &turn_scope,
            turn_id: TurnId::new(),
            run_id: expected_run_id,
            reply_message_refs: &[],
            result_refs: &[wrong_run_result],
        })
        .await
        .expect("wrong-run result evidence should fail closed");

    assert!(!verified);
    assert!(!verify_tool_result_ref(
        &forged_history,
        &malformed_result,
        expected_run_id.to_string().as_str()
    ));
    assert!(!verify_tool_result_ref(
        &forged_history,
        &unsafe_summary_result,
        expected_run_id.to_string().as_str()
    ));
}

#[tokio::test]
async fn applier_rejects_agentless_transcript_evidence_before_transition() {
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let applier = LoopExitApplier::new(transition.clone(), Arc::new(evidence));
    let claimed = claimed_run();
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let err = applier
        .apply(&claimed, exit)
        .await
        .expect_err("agentless transcript evidence should stop before transition");

    assert!(matches!(err, TurnError::InvalidRequest { .. }));
    assert_eq!(transition.apply_count(), 0);
}

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_stored_thread_scope_mismatch() {
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let requested_scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        Some(AgentId::new("agent-request").expect("valid")),
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let stored_scope = ThreadScope {
        tenant_id: requested_scope.tenant_id.clone(),
        agent_id: AgentId::new("agent-stored").expect("valid"),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: stored_scope.clone(),
            thread_id: Some(requested_scope.thread_id.clone()),
            created_by_actor_id: "user:test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let run_id = TurnRunId::new();
    let draft = thread_service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: stored_scope.clone(),
            thread_id: requested_scope.thread_id.clone(),
            turn_run_id: run_id.to_string(),
            content: MessageContent::text("wrong-scope reply"),
        })
        .await
        .expect("draft");
    thread_service
        .finalize_assistant_message(
            &stored_scope,
            &requested_scope.thread_id,
            draft.message_id,
            MessageContent::text("wrong-scope reply"),
        )
        .await
        .expect("finalized");
    let evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_service,
        Arc::new(ironclaw_turns::InMemoryTurnStateStore::default()) as Arc<dyn TurnStateStore>,
        Arc::new(PanicLoopCheckpointStore),
        stored_scope,
    );
    let message_ref =
        LoopMessageRef::new(format!("msg:{}", draft.message_id)).expect("valid message ref");

    let err = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &requested_scope,
            turn_id: TurnId::new(),
            run_id,
            reply_message_refs: &[message_ref],
            result_refs: &[],
        })
        .await
        .expect_err("stored thread scope must match request scope before history is trusted");

    assert!(matches!(err, TurnError::InvalidRequest { .. }));
    assert!(err.to_string().contains("scope does not match"));
}

#[tokio::test]
async fn thread_checkpoint_evidence_does_not_read_checkpoint_for_blocked_claims() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let exit = blocked_exit(LoopBlockedKind::Approval);
    let LoopExit::Blocked(blocked) = &exit else {
        unreachable!("blocked helper returns blocked exit")
    };
    let verified = evidence
        .verify_blocked_evidence(BlockedEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            blocked,
        })
        .await
        .expect("blocked evidence should fail closed without checkpoint I/O");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_fails_closed_for_failure_evidence() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let failed = LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: None,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: test_exit_id(),
    };
    let verified = evidence
        .verify_failure_evidence(FailureEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            failed: &failed,
        })
        .await
        .expect("missing diagnostics store should fail closed");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_assumes_recovery_when_latest_checkpoint_unknown() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let latest = evidence
        .latest_checkpoint_kind(
            &claimed.state.scope,
            claimed.state.turn_id,
            claimed.state.run_id,
        )
        .await
        .expect("latest checkpoint fallback should not read store");

    assert_eq!(latest, Some(LoopCheckpointKind::BeforeSideEffect));
}
