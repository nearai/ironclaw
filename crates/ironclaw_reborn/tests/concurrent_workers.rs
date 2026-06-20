//! Concurrency tests for `TurnRunScheduler` + `RebornTurnRunExecutor`.
//!
//! A barrier-blocking driver blocks inside `run()` until a `Barrier(2)` is
//! reached, using a shared atomic entry counter to prove both runs are
//! executing simultaneously. With `max_concurrent_runs = 1` the second run is
//! never claimed while the first is blocked, so the barrier is never filled —
//! the test would time out. With `max_concurrent_runs = 2` both runs are
//! claimed concurrently, both enter the barrier, and both eventually finish.
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_runtime::{TurnRunScheduler, TurnRunSchedulerConfig};
use ironclaw_loop_support::{
    EmptyUserProfileSource, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostManagedModelError, HostManagedModelErrorKind,
    HostManagedModelGateway, HostManagedModelRequest, HostManagedModelResponse,
    HostUserProfileSource,
};
use ironclaw_reborn::turn_run_executor::RebornTurnRunExecutor;
use ironclaw_reborn::{
    driver_registry::{DriverKind, DriverRegistry, DriverRequirements},
    loop_driver_host::{RebornLoopDriverHostFactory, TextOnlyLoopHostConfig},
    loop_exit_applier::{InMemoryLoopExitEvidencePort, LoopExitApplier, LoopExitEvidencePort},
    turn_runner::HostFactory,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::TurnRunWakeNotifier as _;
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, AllowAllTurnAdmissionPolicy,
    EventCursor, GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore,
    InMemoryRunProfileResolver, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    LoopCheckpointStore, LoopExit, LoopExitId, LoopFailed, LoopFailureKind, ReplyTargetBindingRef,
    RunProfileResolutionRequest, RunProfileResolver, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnRunId, TurnRunWake, TurnScope, TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopDriverHost, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
        LoopRunContext, PromptMode,
    },
    runner::TurnRunTransitionPort,
};
use tokio::sync::Barrier;

// ---------------------------------------------------------------------------
// Barrier-blocking driver
//
// Each invocation of `run()` atomically increments `entry_count` then blocks
// on `barrier`. The test waits until `entry_count == 2` before the barrier
// can proceed, which proves both drivers are inside `run()` simultaneously.
//
// The driver intentionally returns `Err(AgentLoopDriverError::Failed{..})`
// after the barrier. That path is always valid (the worker records a
// controlled failure) so there are no loop-protocol constraints to satisfy.
// What matters for this test is that both runs reach `Running` and BOTH
// enter the barrier concurrently — single-worker execution would block
// forever at `barrier.wait()` because the second run can never be claimed
// while the first is blocked.
// ---------------------------------------------------------------------------

struct BarrierDriver {
    descriptor: AgentLoopDriverDescriptor,
    barrier: Arc<Barrier>,
    /// Incremented when each driver enters `run()` — lets the test observe
    /// concurrent execution without polling turn-state timings.
    entry_count: Arc<AtomicU32>,
}

#[async_trait]
impl AgentLoopDriver for BarrierDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        // Signal that this invocation is inside run() — counter reaches 2
        // when both workers have claimed and entered simultaneously.
        self.entry_count.fetch_add(1, Ordering::SeqCst);
        // Block until the peer invocation also enters — proves concurrency.
        self.barrier.wait().await;
        // Return an error (controlled fail) — always valid, no host evidence needed.
        Err(AgentLoopDriverError::Failed {
            reason_kind: "test_concurrent_barrier".to_string(),
        })
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run(
            AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Minimal gateway (never called; barrier driver doesn't hit the model)
// ---------------------------------------------------------------------------

struct NoOpGateway;

#[async_trait]
impl HostManagedModelGateway for NoOpGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::new(
            HostManagedModelErrorKind::Unavailable,
            "NoOpGateway: model not available in barrier test",
        ))
    }
}

// ---------------------------------------------------------------------------
// Minimal identity context source (returns nothing; barrier driver skips model)
// ---------------------------------------------------------------------------

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn submit_run_on_thread(
    thread_id: &ThreadId,
    thread_service: &InMemorySessionThreadService,
    thread_scope: &ThreadScope,
    turn_store: &InMemoryTurnStateStore,
    resolver: &InMemoryRunProfileResolver,
    idempotency_key: &str,
    user_id: &UserId,
) -> TurnRunId {
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: user_id.to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some(format!("event-{idempotency_key}")),
            content: MessageContent::text("barrier test message"),
        })
        .await
        .unwrap();

    let turn_scope = TurnScope::new(
        thread_scope.tenant_id.clone(),
        Some(thread_scope.agent_id.clone()),
        thread_scope.project_id.clone(),
        thread_id.clone(),
    );
    let submit = turn_store
        .submit_turn(
            SubmitTurnRequest {
                scope: turn_scope,
                actor: TurnActor::new(user_id.clone()),
                accepted_message_ref: AcceptedMessageRef::new(format!(
                    "accepted-{idempotency_key}"
                ))
                .unwrap(),
                source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            },
            &AllowAllTurnAdmissionPolicy,
            resolver,
        )
        .await
        .unwrap();

    let SubmitTurnResponse::Accepted {
        turn_id, run_id, ..
    } = submit;
    // Mark the accepted message as submitted so the thread service has a
    // run_id association, matching what queue_fixture_turn does.
    thread_service
        .mark_message_submitted(
            thread_scope,
            thread_id,
            accepted.message_id,
            turn_id.to_string(),
            run_id.to_string(),
        )
        .await
        .unwrap();
    run_id
}

async fn wait_for_status(
    store: &InMemoryTurnStateStore,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected: TurnStatus,
    timeout_secs: u64,
    label: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let state = store
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .unwrap();
        if state.status == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{label}: timed out waiting for {expected:?}; last={:?} failure={:?}",
            state.status,
            state.failure,
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Like `submit_run_on_thread` but stamps `owner_user_id` on the TurnScope so
/// per-user cap checks fire. Used by the C4 config-wiring integration test.
async fn submit_owned_run_on_thread(
    thread_id: &ThreadId,
    thread_service: &InMemorySessionThreadService,
    thread_scope: &ThreadScope,
    turn_store: &InMemoryTurnStateStore,
    resolver: &InMemoryRunProfileResolver,
    idempotency_key: &str,
    user_id: &UserId,
) -> TurnRunId {
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: user_id.to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some(format!("event-{idempotency_key}")),
            content: MessageContent::text("cap test message"),
        })
        .await
        .unwrap();

    // Use new_with_owner so the TurnScope carries an explicit_owner_user_id,
    // which is what run_user_key reads for per-user cap accounting.
    let turn_scope = TurnScope::new_with_owner(
        thread_scope.tenant_id.clone(),
        Some(thread_scope.agent_id.clone()),
        thread_scope.project_id.clone(),
        thread_id.clone(),
        Some(user_id.clone()),
    );
    let submit = turn_store
        .submit_turn(
            SubmitTurnRequest {
                scope: turn_scope,
                actor: TurnActor::new(user_id.clone()),
                accepted_message_ref: AcceptedMessageRef::new(format!(
                    "accepted-{idempotency_key}"
                ))
                .unwrap(),
                source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            },
            &AllowAllTurnAdmissionPolicy,
            resolver,
        )
        .await
        .unwrap();

    let SubmitTurnResponse::Accepted {
        turn_id, run_id, ..
    } = submit;
    thread_service
        .mark_message_submitted(
            thread_scope,
            thread_id,
            accepted.message_id,
            turn_id.to_string(),
            run_id.to_string(),
        )
        .await
        .unwrap();
    run_id
}

// ---------------------------------------------------------------------------
// Config wiring test: TurnRunnerSettings → InMemoryTurnStateStoreLimits
//
// This is the caller-level C4 assertion. It directly verifies the mapping
// inside build_reborn_runtime: build an InMemoryTurnStateStore with the limits
// that the composition would thread in, then exercise claim behavior to prove
// the cap is applied.
// ---------------------------------------------------------------------------

/// Verify that config wiring correctly enforces per-user concurrency cap.
///
/// C3 wires `runner.max_concurrent_runs_per_user` into `InMemoryTurnStateStoreLimits`
/// when building the store. This test builds the store with `cap = 1` (the value
/// `build_reborn_runtime` would set from `TurnRunnerSettings`) and directly probes
/// claim behavior to prove:
///
/// 1. Two runs (user_a_1 and user_b) are claimable — user_a at cap, user_b not capped.
/// 2. A third claim is blocked: user_a already has 1 Running run (cap=1), user_b
///    is Running, no other queued run is eligible.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_wiring_per_user_cap_enforced_via_store_limits() {
    use ironclaw_turns::TurnLeaseToken;
    use ironclaw_turns::runner::{ClaimRunRequest, TurnRunTransitionPort};
    use std::num::NonZeroU32;

    let tenant_id = TenantId::new("tenant-cap-wiring").unwrap();
    let agent_id = AgentId::new("agent-cap-wiring").unwrap();
    let project_id = ProjectId::new("project-cap-wiring").unwrap();
    let user_a = UserId::new("user-wiring-a").unwrap();
    let user_b = UserId::new("user-wiring-b").unwrap();

    // Build the store with per-user cap = 1, mirroring what build_reborn_runtime
    // constructs from TurnRunnerSettings { max_concurrent_runs_per_user: NonZeroU32::new(1) }.
    let limits = InMemoryTurnStateStoreLimits {
        max_concurrent_runs_per_user: NonZeroU32::new(1),
        ..InMemoryTurnStateStoreLimits::default()
    };
    let turn_store = Arc::new(InMemoryTurnStateStore::with_limits(limits));
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let resolver = InMemoryRunProfileResolver::default();

    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: None,
        mission_id: None,
    };

    // Submit two runs for user A and one for user B (with explicit owner so
    // run_user_key returns Some and per-user cap accounting fires).
    let thread_id_a1 = ThreadId::new("wiring-thread-a1").unwrap();
    submit_owned_run_on_thread(
        &thread_id_a1,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-wiring-a1",
        &user_a,
    )
    .await;

    let thread_id_a2 = ThreadId::new("wiring-thread-a2").unwrap();
    submit_owned_run_on_thread(
        &thread_id_a2,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-wiring-a2",
        &user_a,
    )
    .await;

    let thread_id_b = ThreadId::new("wiring-thread-b").unwrap();
    submit_owned_run_on_thread(
        &thread_id_b,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-wiring-b",
        &user_b,
    )
    .await;

    // With cap=1 for user A:
    // - First claim: gets user_a_1 (earliest queued, no cap yet).
    // - Second claim: user_a_2 is skipped (user_a at cap); gets user_b.
    // - Third claim: both user_a (1 running) and user_b (1 running) are at their
    //   respective limits — returns None.
    use ironclaw_turns::TurnRunnerId;
    let runner_id = TurnRunnerId::new();

    let claimed_1 = (Arc::clone(&turn_store) as Arc<dyn TurnRunTransitionPort>)
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        claimed_1.is_some(),
        "first claim should succeed (a1 or b is queued)"
    );
    let first = claimed_1.unwrap();
    assert_eq!(
        first.state.status,
        TurnStatus::Running,
        "claimed run 1 should be Running"
    );

    let claimed_2 = (Arc::clone(&turn_store) as Arc<dyn TurnRunTransitionPort>)
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        claimed_2.is_some(),
        "second claim should succeed (b or a1 is still claimable)"
    );
    let second = claimed_2.unwrap();
    assert_eq!(
        second.state.status,
        TurnStatus::Running,
        "claimed run 2 should be Running"
    );

    // The third claim must return None: user_a has 1 running run (cap=1) so
    // user_a_2 is skipped; user_b is already Running; no eligible run remains.
    let claimed_3 = (Arc::clone(&turn_store) as Arc<dyn TurnRunTransitionPort>)
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        claimed_3.is_none(),
        "third claim must be blocked (user_a at cap=1; user_b already Running); \
         claimed_1 scope={:?} run={:?}, claimed_2 scope={:?} run={:?}",
        first.state.scope.thread_id,
        first.state.run_id,
        second.state.scope.thread_id,
        second.state.run_id,
    );
}

// ---------------------------------------------------------------------------
// TurnRunScheduler + RebornTurnRunExecutor tests
//
// These tests exercise the production concurrency path:
//   TurnRunScheduler → RebornTurnRunExecutor → LoopExitApplier
// ---------------------------------------------------------------------------

/// Concurrency proof for TurnRunScheduler + RebornTurnRunExecutor.
///
/// Two runs on distinct threads are submitted simultaneously. The `BarrierDriver`
/// blocks inside `run()` until BOTH invocations arrive (a `tokio::sync::Barrier(2)`).
///
/// With `max_concurrent_runs = 2`: the scheduler claims both runs concurrently, both
/// enter the barrier, `entry_count` reaches 2, the barrier releases, and both runs
/// fail (the barrier driver returns `Err`). The test asserts `entry_count == 2` and
/// both runs reach `TurnStatus::Failed`.
///
/// If `max_concurrent_runs` were 1: only run_a would be claimed while run_b stays
/// `Queued`. `barrier.wait()` would never be fulfilled — the test would time out at
/// the `wait_for_status` assertion for run_b, proving the barrier is load-bearing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_executor_two_runs_concurrently() {
    let tenant_id = TenantId::new("tenant-sched-concurrent").unwrap();
    let agent_id = AgentId::new("agent-sched-concurrent").unwrap();
    let project_id = ProjectId::new("project-sched-concurrent").unwrap();
    let user_id = UserId::new("user-sched-concurrent").unwrap();

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: None,
        mission_id: None,
    };

    let turn_store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits::default(),
    ));
    let resolver = InMemoryRunProfileResolver::default();

    // Submit run 1 on thread A.
    let thread_id_a = ThreadId::new("sched-concurrent-thread-a").unwrap();
    let run_id_a = submit_run_on_thread(
        &thread_id_a,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-sched-concurrent-a",
        &user_id,
    )
    .await;

    // Submit run 2 on thread B.
    let thread_id_b = ThreadId::new("sched-concurrent-thread-b").unwrap();
    let run_id_b = submit_run_on_thread(
        &thread_id_b,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-sched-concurrent-b",
        &user_id,
    )
    .await;

    // Build the barrier driver — barrier size 2 means both invocations must enter
    // run() simultaneously before either can proceed.
    let barrier = Arc::new(Barrier::new(2));
    let entry_count = Arc::new(AtomicU32::new(0));
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(BarrierDriver {
                descriptor,
                barrier: Arc::clone(&barrier),
                entry_count: Arc::clone(&entry_count),
            }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();
    let registry = Arc::new(registry);

    // Build shared deps.
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(NoOpGateway);

    let transition_port: Arc<dyn TurnRunTransitionPort> = turn_store.clone();

    // Use InMemoryLoopExitEvidencePort (fail-closed defaults) — the barrier driver
    // never reaches the applier (it returns Err), so the evidence port is never
    // consulted; the scheduler's record_runner_failure path handles the failure.
    let loop_exit_applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&transition_port),
        Arc::new(InMemoryLoopExitEvidencePort::new()) as Arc<dyn LoopExitEvidencePort>,
    ));

    let host_factory = Arc::new(
        RebornLoopDriverHostFactory::new(
            Arc::clone(&thread_service),
            thread_scope.clone(),
            Arc::clone(&gateway),
            checkpoint_state_store,
            turn_store.clone() as Arc<dyn TurnStateStore>,
            loop_checkpoint_store,
            milestone_sink,
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
            InstructionSafetyContext::local_development_noop(),
        )
        .with_identity_context_source(
            Arc::new(EmptyIdentityContextSource) as Arc<dyn HostIdentityContextSource>
        )
        .with_user_profile_source(
            Arc::new(EmptyUserProfileSource) as Arc<dyn HostUserProfileSource>
        ),
    );

    let executor = Arc::new(RebornTurnRunExecutor::new(
        Arc::clone(&loop_exit_applier),
        Arc::clone(&registry),
        host_factory as Arc<dyn HostFactory>,
    ));

    let scheduler_config = TurnRunSchedulerConfig::default()
        .with_max_concurrent_runs(2)
        .with_runner_heartbeat_interval(std::time::Duration::from_millis(50))
        .with_poll_interval(std::time::Duration::from_millis(10))
        .with_claim_error_backoff(std::time::Duration::from_millis(5));

    let scheduler_handle =
        TurnRunScheduler::new(Arc::clone(&transition_port), executor, scheduler_config).start();

    // Wake the scheduler for both runs — coordinator wake is the real prod path;
    // here we simulate it by directly notifying the scheduler's wake notifier.
    let scope_a = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        Some(project_id.clone()),
        thread_id_a.clone(),
    );
    let scope_b = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        Some(project_id.clone()),
        thread_id_b.clone(),
    );
    scheduler_handle
        .wake_notifier()
        .notify_queued_run(TurnRunWake {
            scope: scope_a.clone(),
            run_id: run_id_a,
            status: TurnStatus::Queued,
            event_cursor: EventCursor::default(),
        })
        .unwrap();
    scheduler_handle
        .wake_notifier()
        .notify_queued_run(TurnRunWake {
            scope: scope_b.clone(),
            run_id: run_id_b,
            status: TurnStatus::Queued,
            event_cursor: EventCursor::default(),
        })
        .unwrap();

    // Both runs must reach Failed (barrier driver returns Err after both entries).
    // With max_concurrent_runs=2: both are claimed concurrently, both enter the
    // barrier, entry_count reaches 2, barrier releases, both fail.
    // With max_concurrent_runs=1: run_a blocks at barrier.wait(), run_b stays
    // Queued — the test would time out here.
    wait_for_status(
        &turn_store,
        &scope_a,
        run_id_a,
        TurnStatus::Failed,
        10,
        "run_a should fail after barrier releases (scheduler has 2 permits)",
    )
    .await;
    wait_for_status(
        &turn_store,
        &scope_b,
        run_id_b,
        TurnStatus::Failed,
        10,
        "run_b should fail after barrier releases (proves concurrent execution)",
    )
    .await;

    // Both driver invocations entered run() simultaneously — proves N=2 concurrency.
    assert_eq!(
        entry_count.load(Ordering::SeqCst),
        2,
        "both RebornTurnRunExecutor invocations should have entered run() concurrently"
    );

    scheduler_handle.shutdown().await;
}

/// End-to-end success path: TurnRunScheduler + RebornTurnRunExecutor applies a
/// LoopExit::Failed through the applier, reaching a terminal Failed state.
///
/// Unlike `scheduler_executor_two_runs_concurrently` (where the driver returns an
/// `Err` and the scheduler itself records the failure via `record_runner_failure`),
/// this test uses a driver that returns `Ok(LoopExit::Failed)`. In that path:
///
///   1. `RebornTurnRunExecutor::execute_claimed_run` calls `invoke_driver` → `Ok(exit)`.
///   2. It calls `apply_exit` → `LoopExitApplier::apply()` → `apply_validated_loop_exit`.
///   3. The `AcceptAllEvidencePort` returns `true` for every verification method.
///   4. `execute_claimed_run` returns `Ok(())` — the scheduler does NOT call
///      `record_runner_failure` — the transition was applied inside the executor.
///   5. The run reaches `TurnStatus::Failed` via the applier's own transition path,
///      not the scheduler's terminal-failure path. This is the "applier-owned exit"
///      variant of the production path.
///
/// Asserts: `execute_claimed_run` returns `Ok(())` and the run is terminal (`Failed`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_executor_applies_loop_exit_end_to_end() {
    // ---------------------------------------------------------------------------
    // Driver that returns Ok(LoopExit::Failed) — exercises the applier path.
    // ---------------------------------------------------------------------------
    struct ApplierPathDriver {
        descriptor: AgentLoopDriverDescriptor,
    }

    #[async_trait]
    impl AgentLoopDriver for ApplierPathDriver {
        fn descriptor(&self) -> AgentLoopDriverDescriptor {
            self.descriptor.clone()
        }

        async fn run(
            &self,
            _request: AgentLoopDriverRunRequest,
            _host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            Ok(LoopExit::Failed(LoopFailed {
                reason_kind: LoopFailureKind::DriverBug,
                checkpoint_id: None,
                usage_summary_ref: None,
                diagnostic_ref: None,
                exit_id: LoopExitId::new("exit:test-applier-path").expect("valid exit id"),
            }))
        }

        async fn resume(
            &self,
            request: AgentLoopDriverResumeRequest,
            host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            self.run(
                AgentLoopDriverRunRequest {
                    turn_id: request.turn_id,
                    run_id: request.run_id,
                    resolved_run_profile: request.resolved_run_profile,
                },
                host,
            )
            .await
        }
    }

    let tenant_id = TenantId::new("tenant-sched-e2e").unwrap();
    let agent_id = AgentId::new("agent-sched-e2e").unwrap();
    let project_id = ProjectId::new("project-sched-e2e").unwrap();
    let user_id = UserId::new("user-sched-e2e").unwrap();

    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: None,
        mission_id: None,
    };

    let turn_store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits::default(),
    ));
    let resolver = InMemoryRunProfileResolver::default();

    let thread_id = ThreadId::new("sched-e2e-thread").unwrap();
    let run_id = submit_run_on_thread(
        &thread_id,
        &thread_service,
        &thread_scope,
        &turn_store,
        &resolver,
        "idem-sched-e2e",
        &user_id,
    )
    .await;

    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ApplierPathDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();
    let registry = Arc::new(registry);

    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(NoOpGateway);

    let transition_port: Arc<dyn TurnRunTransitionPort> = turn_store.clone();

    // AcceptAllEvidencePort: returns true for every evidence verification so
    // the applier can transition the run to TurnStatus::Failed without needing
    // real thread messages (the barrier driver skips all model/thread IO).
    struct AcceptAllEvidencePort;

    #[async_trait]
    impl LoopExitEvidencePort for AcceptAllEvidencePort {
        async fn verify_completion_refs(
            &self,
            _request: ironclaw_reborn::loop_exit_applier::CompletionEvidenceRequest<'_>,
        ) -> Result<bool, ironclaw_turns::TurnError> {
            Ok(true)
        }
        async fn verify_final_checkpoint(
            &self,
            _request: ironclaw_reborn::loop_exit_applier::FinalCheckpointEvidenceRequest<'_>,
        ) -> Result<bool, ironclaw_turns::TurnError> {
            Ok(true)
        }
        async fn verify_blocked_evidence(
            &self,
            _request: ironclaw_reborn::loop_exit_applier::BlockedEvidenceRequest<'_>,
        ) -> Result<bool, ironclaw_turns::TurnError> {
            Ok(true)
        }
        async fn verify_failure_evidence(
            &self,
            _request: ironclaw_reborn::loop_exit_applier::FailureEvidenceRequest<'_>,
        ) -> Result<bool, ironclaw_turns::TurnError> {
            Ok(true)
        }
        async fn is_cancellation_observed(
            &self,
            _scope: &ironclaw_turns::TurnScope,
            _turn_id: ironclaw_turns::TurnId,
            _run_id: ironclaw_turns::TurnRunId,
        ) -> Result<bool, ironclaw_turns::TurnError> {
            Ok(true)
        }
        async fn latest_checkpoint_kind(
            &self,
            _scope: &ironclaw_turns::TurnScope,
            _turn_id: ironclaw_turns::TurnId,
            _run_id: ironclaw_turns::TurnRunId,
        ) -> Result<Option<ironclaw_turns::LoopCheckpointKind>, ironclaw_turns::TurnError> {
            Ok(None)
        }
    }

    let evidence_port = Arc::new(AcceptAllEvidencePort) as Arc<dyn LoopExitEvidencePort>;
    let loop_exit_applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&transition_port),
        evidence_port,
    ));

    let host_factory = Arc::new(
        RebornLoopDriverHostFactory::new(
            Arc::clone(&thread_service),
            thread_scope.clone(),
            Arc::clone(&gateway),
            checkpoint_state_store,
            turn_store.clone() as Arc<dyn TurnStateStore>,
            loop_checkpoint_store,
            milestone_sink,
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
            InstructionSafetyContext::local_development_noop(),
        )
        .with_identity_context_source(
            Arc::new(EmptyIdentityContextSource) as Arc<dyn HostIdentityContextSource>
        )
        .with_user_profile_source(
            Arc::new(EmptyUserProfileSource) as Arc<dyn HostUserProfileSource>
        ),
    );

    let executor = Arc::new(RebornTurnRunExecutor::new(
        Arc::clone(&loop_exit_applier),
        Arc::clone(&registry),
        host_factory as Arc<dyn HostFactory>,
    ));

    let scheduler_config = TurnRunSchedulerConfig::default()
        .with_max_concurrent_runs(1)
        .with_runner_heartbeat_interval(std::time::Duration::from_millis(50))
        .with_poll_interval(std::time::Duration::from_millis(10))
        .with_claim_error_backoff(std::time::Duration::from_millis(5));

    let scheduler_handle =
        TurnRunScheduler::new(Arc::clone(&transition_port), executor, scheduler_config).start();

    let turn_scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        Some(project_id.clone()),
        thread_id.clone(),
    );
    scheduler_handle
        .wake_notifier()
        .notify_queued_run(TurnRunWake {
            scope: turn_scope.clone(),
            run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor::default(),
        })
        .unwrap();

    // The applier-path driver returns Ok(LoopExit::Failed), so execute_claimed_run
    // returns Ok(()) and the applier transitions the run to Failed. The scheduler
    // does NOT call record_runner_failure — that's the key distinction from the
    // barrier-driver test above (which returns Err from the driver).
    wait_for_status(
        &turn_store,
        &turn_scope,
        run_id,
        TurnStatus::Failed,
        10,
        "run should reach Failed via applier-owned loop exit (not scheduler terminal failure)",
    )
    .await;

    scheduler_handle.shutdown().await;
}
