use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, IdempotencyKey, InMemoryRunProfileResolver,
    ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef, TurnActiveRunRefState, TurnActor,
    TurnError, TurnRunId, TurnScope, TurnStateStore, TurnStatus,
};

struct FailingTurnStateStore;

#[async_trait::async_trait]
impl TurnStateStore for FailingTurnStateStore {
    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        unreachable!("active_run_ref_state only calls get_run_state")
    }

    async fn resume_turn(
        &self,
        _request: ironclaw_turns::ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        unreachable!("active_run_ref_state only calls get_run_state")
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        // WS-3 implements this.
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn request_cancel(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        unreachable!("active_run_ref_state only calls get_run_state")
    }

    async fn get_run_state(
        &self,
        _request: ironclaw_turns::GetRunStateRequest,
    ) -> Result<ironclaw_turns::TurnRunState, TurnError> {
        Err(TurnError::Unavailable {
            reason: "backend unavailable".to_string(),
        })
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn turn_actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

fn submit_request_for(
    scope: TurnScope,
    idempotency_key: &str,
) -> ironclaw_turns::SubmitTurnRequest {
    ironclaw_turns::SubmitTurnRequest {
        scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idempotency_key}"))
            .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

#[tokio::test]
async fn active_run_ref_state_classifies_missing_nonterminal_and_terminal() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("active-run-ref-state");

    assert_eq!(
        ironclaw_turns::active_run_ref_state(&store, scope.clone(), None)
            .await
            .unwrap(),
        TurnActiveRunRefState::Missing
    );

    let accepted = store
        .submit_turn(
            submit_request_for(scope.clone(), "active-run-ref-state"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let ironclaw_turns::SubmitTurnResponse::Accepted { run_id, status, .. } = accepted;
    assert_eq!(status, TurnStatus::Queued);

    assert_eq!(
        ironclaw_turns::active_run_ref_state(&store, scope.clone(), Some(run_id))
            .await
            .unwrap(),
        TurnActiveRunRefState::Nonterminal
    );

    let cancel = store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: scope.clone(),
            actor: turn_actor(),
            run_id,
            reason: ironclaw_turns::SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("active-run-ref-state-cancel").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancel.status, TurnStatus::Cancelled);

    assert_eq!(
        ironclaw_turns::active_run_ref_state(&store, scope.clone(), Some(run_id))
            .await
            .unwrap(),
        TurnActiveRunRefState::Terminal
    );
}

#[tokio::test]
async fn active_run_ref_state_treats_missing_lookup_as_missing() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let scope = turn_scope("active-run-ref-missing");

    assert_eq!(
        ironclaw_turns::active_run_ref_state(&store, scope, Some(TurnRunId::new()))
            .await
            .unwrap(),
        TurnActiveRunRefState::Missing
    );
}

#[tokio::test]
async fn active_run_ref_state_propagates_non_scope_not_found_errors() {
    let result = ironclaw_turns::active_run_ref_state(
        &FailingTurnStateStore,
        turn_scope("active-run-ref-error"),
        Some(TurnRunId::new()),
    )
    .await;

    assert!(matches!(result, Err(TurnError::Unavailable { .. })));
}
