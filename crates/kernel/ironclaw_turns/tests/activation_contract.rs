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
use ironclaw_processes::{
    ClaimProcessesRequest, ProcessKind, ProcessLeaseRequest, ProcessStateTransitionRequest,
    ProcessWorkerId,
};
use ironclaw_turns::{
    AcceptedMessageRef, ActivateThreadRequest, ActivationProvenance, AdmissionRejection,
    AdmissionRejectionReason, AgentTurnRuntimePort, AgentTurnSpawnTreeRuntimePort,
    CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator, GetRunStateRequest,
    IdempotencyKey, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse,
    SYSTEM_WAKE_STREAK_CAP, SYSTEM_WAKE_WINDOW_OVERFETCH, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnRunState, TurnScope,
    test_support::{InMemoryAgentTurnProcessSystem, in_memory_agent_turn_process_system},
};
use std::sync::Arc;

/// Drive the thread's single active run to a terminal state so the next
/// activation is admitted on an idle thread rather than rejected as busy.
async fn complete_active_run(system: &InMemoryAgentTurnProcessSystem, scope: &TurnScope) {
    let transitions = system.transitions();
    let claimed = transitions
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("activation-contract-worker"),
            scope_filter: Some(scope.to_resource_scope()),
            process_id_filter: None,
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim succeeds")
        .pop()
        .expect("a run is claimable");
    transitions
        .complete_process(ProcessStateTransitionRequest {
            lease: ProcessLeaseRequest {
                process_id: claimed.state.process_id,
                worker_id: claimed.worker_id.clone(),
                lease_token: claimed.lease_token.clone(),
            },
            metadata: None,
        })
        .await
        .expect("run completes");
}

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
        // Derived from the key so distinct activations carry distinct accepted
        // messages (as production does — each background wake references the
        // settled child result that triggered it), while a deliberate retry
        // reusing the key also reuses the message, which is what makes it a
        // retry rather than a new activation.
        accepted_message_ref: AcceptedMessageRef::new(format!("accepted-activation-{key}"))
            .expect("accepted"),
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
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
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
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
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
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
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

/// The cap must be enforced at the caller, not only in the predicate, and a
/// refusal must not create a run. Sixteen consecutive System activations
/// saturate the streak; the seventeenth is refused.
#[tokio::test]
async fn activate_refuses_a_system_wake_past_the_streak_cap() {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-streak-saturated");

    for index in 0..SYSTEM_WAKE_STREAK_CAP {
        let SubmitTurnResponse::Accepted { .. } = coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::System,
                &format!("streak-key-{index}"),
            ))
            .await
            .unwrap_or_else(|error| panic!("wake {index} must be admitted, got {error:?}"));
        // Drive each run terminal so the thread is idle for the next wake;
        // otherwise the second activation would be refused as busy and this
        // test would pass for the wrong reason.
        complete_active_run(&system, &scope).await;
    }

    let error = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            "streak-key-over",
        ))
        .await
        .expect_err("a saturated System streak must refuse the next wake");

    assert!(
        matches!(
            error,
            TurnError::AdmissionRejected(AdmissionRejection {
                reason: AdmissionRejectionReason::SystemWakeStreak,
                ..
            })
        ),
        "the cap refusal must carry its own reason, distinguishable from \
         'activation unsupported' and 'window unreadable'; got {error:?}"
    );
    // The refusal must also create no run. Asserting only the error would let a
    // regression that admits the run and *then* errors still pass.
    let after = AgentTurnRuntimePort::recent_runs_for_thread(
        runtime.as_ref(),
        &scope,
        SYSTEM_WAKE_STREAK_CAP.saturating_mul(2),
    )
    .await
    .expect("window read");
    assert_eq!(
        after.len() as u32,
        SYSTEM_WAKE_STREAK_CAP,
        "a refused wake must not have created a run"
    );
}

/// A full raw fetch that cannot yield a cap-sized non-ParentAgent window means
/// the streak could not be established — unknown, not absent. Admitting there
/// is the fail-open hole the over-fetch alone left; this pins the fail-closed
/// behavior. A genuinely short fetch is a young thread and still admits.
#[tokio::test]
async fn a_window_crowded_out_by_parent_agent_runs_fails_closed() {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-streak-crowded");

    // Fill the entire raw fetch window with ParentAgent runs, so no
    // non-ParentAgent record survives the filter.
    let fetch_limit = SYSTEM_WAKE_STREAK_CAP.saturating_mul(SYSTEM_WAKE_WINDOW_OVERFETCH);
    for index in 0..fetch_limit {
        coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::ParentAgent,
                &format!("crowded-parent-{index}"),
            ))
            .await
            .unwrap_or_else(|error| panic!("parent activation {index} admitted, got {error:?}"));
        complete_active_run(&system, &scope).await;
    }

    let error = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            "crowded-system",
        ))
        .await
        .expect_err("an unestablished window must fail closed, not admit");

    assert!(
        matches!(
            error,
            TurnError::AdmissionRejected(AdmissionRejection {
                reason: AdmissionRejectionReason::SystemWakeStreak,
                ..
            })
        ),
        "expected the streak refusal, got {error:?}"
    );
}

/// `activate()` advertises ordinary submission idempotency. Retrying an
/// already-accepted activation at the cap boundary must replay that
/// submission, not be refused by the run its own first attempt created.
#[tokio::test]
async fn replaying_an_accepted_activation_at_the_cap_is_not_refused() {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-streak-replay");

    let mut last_run = None;
    for index in 0..SYSTEM_WAKE_STREAK_CAP {
        let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::System,
                &format!("replay-key-{index}"),
            ))
            .await
            .unwrap_or_else(|error| panic!("wake {index} admitted, got {error:?}"));
        last_run = Some(run_id);
        complete_active_run(&system, &scope).await;
    }

    // Same idempotency key AND same accepted message as the last accepted
    // activation: this is a retry, not a 17th wake.
    let replayed = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            &format!("replay-key-{}", SYSTEM_WAKE_STREAK_CAP - 1),
        ))
        .await
        .expect("a retry of an accepted activation must replay, not hit the cap");

    let SubmitTurnResponse::Accepted { run_id, .. } = replayed;
    assert_eq!(
        Some(run_id),
        last_run,
        "the replay must return the originally accepted run"
    );
}

/// A `Human` activation resets the streak, so a thread that a person has come
/// back to can autonomously wake again.
#[tokio::test]
async fn a_human_activation_lets_system_wakes_resume_after_saturation() {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-streak-reset");

    for index in 0..SYSTEM_WAKE_STREAK_CAP {
        let SubmitTurnResponse::Accepted { .. } = coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::System,
                &format!("reset-key-{index}"),
            ))
            .await
            .expect("wake admitted");
        complete_active_run(&system, &scope).await;
    }

    let SubmitTurnResponse::Accepted { .. } = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::Human,
            "reset-key-human",
        ))
        .await
        .expect("a Human activation is never capped");
    complete_active_run(&system, &scope).await;

    coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            "reset-key-after-human",
        ))
        .await
        .expect("a Human activation must reset the System streak");
}

/// A runtime that cannot answer the recent-window question must refuse a
/// System activation rather than admit it. An empty window reads as "streak
/// not established", so defaulting to one would silently disable the cap.
#[tokio::test]
async fn a_runtime_without_a_recent_window_refuses_system_activation() {
    let runtime = Arc::new(WindowlessRuntime(
        in_memory_agent_turn_process_system().runtime(),
    ));
    let coordinator = DefaultTurnCoordinator::new(runtime);

    let error = coordinator
        .activate(activate_request(
            scope("thread-no-window"),
            ActivationProvenance::System,
            "no-window-key",
        ))
        .await
        .expect_err("a runtime that cannot read the window must refuse");

    assert!(
        matches!(error, TurnError::InvalidRequest { .. }),
        "expected a fail-closed refusal, got {error:?}"
    );
}

/// Wraps a real runtime but declines to answer the recent-window query, taking
/// the trait's fail-closed default.
struct WindowlessRuntime(ironclaw_turns::process_projection::AgentTurnProcessRuntime);

#[async_trait]
impl ironclaw_turns::AgentTurnRuntimePort for WindowlessRuntime {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.0
            .submit_turn(request, admission_policy, run_profile_resolver)
            .await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        ironclaw_turns::AgentTurnRuntimePort::resume_turn(&self.0, request).await
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        ironclaw_turns::AgentTurnRuntimePort::retry_turn(&self.0, request).await
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        ironclaw_turns::AgentTurnRuntimePort::request_cancel(&self.0, request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        ironclaw_turns::AgentTurnRuntimePort::get_run_state(&self.0, request).await
    }
}

/// Design section 8.3 requires ParentAgent runs to be excluded from the FETCH,
/// not filtered afterwards. Filtering a K-sized fetch yields fewer than K
/// records whenever ParentAgent runs are interleaved, and a short window reads
/// as "streak not established" — which silently disables the cap on exactly
/// the human-free interleaved sequences it exists to bound.
///
/// This drives the design's required assertion (c): an interleaved ParentAgent
/// activation neither resets nor counts toward the System streak.
#[tokio::test]
async fn interleaved_parent_agent_runs_do_not_disable_the_system_streak_cap() {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime.clone());
    let scope = scope("thread-streak-interleaved");

    // Alternate System / ParentAgent. No Human ever touches this thread, so the
    // System streak must still saturate at SYSTEM_WAKE_STREAK_CAP.
    for index in 0..SYSTEM_WAKE_STREAK_CAP {
        coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::System,
                &format!("interleaved-system-{index}"),
            ))
            .await
            .unwrap_or_else(|error| panic!("system wake {index} admitted, got {error:?}"));
        complete_active_run(&system, &scope).await;

        coordinator
            .activate(activate_request(
                scope.clone(),
                ActivationProvenance::ParentAgent,
                &format!("interleaved-parent-{index}"),
            ))
            .await
            .unwrap_or_else(|error| panic!("parent extend {index} admitted, got {error:?}"));
        complete_active_run(&system, &scope).await;
    }

    let error = coordinator
        .activate(activate_request(
            scope.clone(),
            ActivationProvenance::System,
            "interleaved-system-over",
        ))
        .await
        .expect_err(
            "ParentAgent runs must be excluded from the window, so the System streak \
             still saturates and the next wake is refused",
        );

    assert!(
        matches!(
            error,
            TurnError::AdmissionRejected(AdmissionRejection {
                reason: AdmissionRejectionReason::SystemWakeStreak,
                ..
            })
        ),
        "expected the streak-cap refusal, got {error:?}"
    );
}
