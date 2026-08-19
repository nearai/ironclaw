//! Behavior pins for `TurnCoordinator::activate` — the single re-activation
//! primitive.
//!
//! `activate` is deliberately not a second admission path: it builds an
//! ordinary `SubmitTurnRequest` so one-active-run exclusivity, idempotency
//! replay, and busy rejection all behave exactly as they do for any other
//! submission. The only thing it adds is the provenance stamp, and these tests
//! assert that stamp at the durable-record seam rather than at the call.

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, ActivateThreadRequest, ActivationProvenance, AgentTurnSpawnTreeRuntimePort,
    CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator, GetRunStateRequest,
    IdempotencyKey, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId,
    TurnRunState, TurnScope, test_support::in_memory_agent_turn_runtime,
};
use std::sync::Arc;

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-activation").expect("tenant"),
        Some(AgentId::new("agent-activation").expect("agent")),
        Some(ProjectId::new("project-activation").expect("project")),
        ThreadId::new(thread).expect("thread"),
    )
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user-activation").expect("user"))
}

fn activate_request(
    scope: TurnScope,
    provenance: ActivationProvenance,
    key: &str,
) -> ActivateThreadRequest {
    ActivateThreadRequest {
        scope,
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new("accepted-activation").expect("accepted"),
        provenance,
        idempotency_key: IdempotencyKey::new(key).expect("idempotency key"),
        received_at: Utc::now(),
        requested_run_profile: None,
    }
}

fn submit_request(scope: TurnScope, key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope,
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new("accepted-activation").expect("accepted"),
        requested_run_profile: None,
        output_contract: None,
        requested_model: None,
        idempotency_key: IdempotencyKey::new(key).expect("idempotency key"),
        received_at: Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        subagent_activation_provenance: None,
    }
}

/// `activate` must reach the ordinary admission path and stamp the provenance
/// onto the run it creates, so the derived streak caps can later see it.
#[tokio::test]
async fn activate_stamps_provenance_on_the_created_run_record() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-activate-system");

    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            "activate-system-key",
        ))
        .await
        .expect("activate succeeds on an idle thread");

    let record = runtime
        .get_run_record(&scope, run_id)
        .await
        .expect("run record read")
        .expect("run record exists");

    assert_eq!(
        record.subagent_activation_provenance,
        Some(ActivationProvenance::System),
        "activate must stamp the provenance onto the durable run record"
    );
}

/// The control: an ordinary submission records no provenance, so an untagged
/// run can never be mistaken for an autonomous wake by the streak caps.
#[tokio::test]
async fn ordinary_submit_turn_records_no_activation_provenance() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-activate-plain");

    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(submit_request(scope.clone(), "activate-plain-key"))
        .await
        .expect("ordinary submit succeeds");

    let record = runtime
        .get_run_record(&scope, run_id)
        .await
        .expect("run record read")
        .expect("run record exists");

    assert_eq!(
        record.subagent_activation_provenance, None,
        "an ordinary submission must stay untagged"
    );
}

/// `ParentAgent` is a distinct tag from `System` and must survive the same way
/// — the two caps read disjoint windows, so a mixed-up tag would silently
/// change which budget a run consumes.
#[tokio::test]
async fn activate_distinguishes_parent_agent_from_system_provenance() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-activate-parent");

    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::ParentAgent,
            "activate-parent-key",
        ))
        .await
        .expect("activate succeeds");

    let record = runtime
        .get_run_record(&scope, run_id)
        .await
        .expect("run record read")
        .expect("run record exists");

    assert_eq!(
        record.subagent_activation_provenance,
        Some(ActivationProvenance::ParentAgent)
    );
}

/// A coordinator that has not opted into activation must refuse rather than
/// silently falling through to an untagged submission — an untagged autonomous
/// wake is invisible to the streak cap that exists to bound it.
#[tokio::test]
async fn default_activate_impl_refuses_rather_than_submitting_untagged() {
    struct NonActivatingCoordinator;

    #[async_trait]
    impl TurnCoordinator for NonActivatingCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            panic!("the default activate() must not fall through to submit_turn");
        }
        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn retry_turn(
            &self,
            _request: RetryTurnRequest,
        ) -> Result<RetryTurnResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unreachable!("not exercised by this test")
        }
    }

    let error = NonActivatingCoordinator
        .activate(activate_request(
            scope("thread-activate-default"),
            ActivationProvenance::System,
            "activate-default-key",
        ))
        .await
        .expect_err("a coordinator without activation support must refuse");

    assert!(
        matches!(error, TurnError::InvalidRequest { .. }),
        "the default activate() must fail closed, got {error:?}"
    );
}
