//! The run-lease fence on `ThreadBackedLoopTranscriptPort` asks the journal
//! whether this worker still owns the run before every transcript write. That
//! read lands on the two-connection process-journal pool, once per write.
//!
//! These tests pin the memo that removes most of those reads *and* the bound
//! that keeps the fence's safety argument intact: an affirmative journal answer
//! is reusable only while the lease that answer described is still live, minus
//! a skew margin. Recovery cannot requeue a run before its lease has expired
//! (`ironclaw_processes::journal_store::state`), so no write the memo admits
//! can land after a requeue.
//!
//! The zombie-worker case is the point of the whole fence: a worker whose
//! heartbeat loop died while its main task stayed blocked wakes up after
//! recovery handed its run to a replacement, and must be refused.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{TurnId, TurnLeaseToken, TurnRunId};
use ironclaw_loop_contracts::{
    AssistantReply, FinalizeAssistantMessage, InMemoryRunProfileResolver, LoopRunContext,
    LoopTranscriptPort, RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_loop_host::ThreadBackedLoopTranscriptPort;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, AgentTurnRuntimePort, AgentTurnSpawnTreeRuntimePort, CancelRunRequest,
    CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, SpawnTreeReservation, SubmitChildRunRequest,
    SubmitTurnRequest, SubmitTurnResponse, TurnAdmissionPolicy, TurnError, TurnRunProfile,
    TurnRunRecord, TurnRunState, TurnScope, TurnStatus,
};

/// A journal stand-in that serves one scripted run record and counts every
/// ownership read the fence issues. Swapping the record models what recovery
/// does to the journal underneath a worker that lost its lease.
struct ScriptedJournal {
    record: std::sync::Mutex<Option<TurnRunRecord>>,
    reads: AtomicUsize,
}

impl ScriptedJournal {
    fn new(record: Option<TurnRunRecord>) -> Self {
        Self {
            record: std::sync::Mutex::new(record),
            reads: AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn set_record(&self, record: Option<TurnRunRecord>) {
        *self.record.lock().expect("scripted record lock") = record;
    }
}

#[async_trait]
impl AgentTurnRuntimePort for ScriptedJournal {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("the fence never submits turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("the fence never resumes turns")
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        unreachable!("the fence never retries turns")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        unreachable!("the fence never cancels runs")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("the fence reads run records, not run state")
    }
}

#[async_trait]
impl AgentTurnSpawnTreeRuntimePort for ScriptedJournal {
    async fn submit_child_turn(
        &self,
        _request: SubmitChildRunRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("the fence never spawns children")
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }

    async fn get_run_record(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.record.lock().expect("scripted record lock").clone())
    }

    async fn reserve_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        unreachable!("the fence never reserves spawn-tree capacity")
    }

    async fn release_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        unreachable!("the fence never releases spawn-tree capacity")
    }

    async fn prune_released_child(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        unreachable!("the fence never prunes spawn-tree children")
    }
}

struct Fixture {
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    run_context: LoopRunContext,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let tenant_id = TenantId::new(format!("tenant-{label}")).unwrap();
        let agent_id = AgentId::new(format!("agent-{label}")).unwrap();
        let project_id = ProjectId::new(format!("project-{label}")).unwrap();
        let user_id = UserId::new(format!("user-{label}")).unwrap();
        let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            project_id: Some(project_id.clone()),
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id.as_str().to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(format!("event-{label}")),
                content: MessageContent::text("hello fence"),
            })
            .await
            .unwrap();
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let run_context = LoopRunContext::new(
            TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        );
        Self {
            thread_service,
            thread_scope,
            run_context,
        }
    }

    fn record(
        &self,
        lease_token: Option<TurnLeaseToken>,
        expires_at: Option<DateTime<Utc>>,
    ) -> TurnRunRecord {
        TurnRunRecord {
            subagent_activation_provenance: None,
            run_id: self.run_context.run_id,
            turn_id: self.run_context.turn_id,
            scope: self.run_context.scope.clone(),
            accepted_message_ref: AcceptedMessageRef::new("msg:fence").unwrap(),
            status: TurnStatus::Running,
            profile: TurnRunProfile::from_resolved(self.run_context.resolved_run_profile.clone()),
            resolved_model_route: None,
            model_usage: None,
            output_contract: Default::default(),
            execution_outcome: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token,
            lease_expires_at: expires_at,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    fn port(
        &self,
        journal: Arc<ScriptedJournal>,
        lease_token: TurnLeaseToken,
    ) -> ThreadBackedLoopTranscriptPort<InMemorySessionThreadService> {
        ThreadBackedLoopTranscriptPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            self.run_context.clone(),
        )
        .with_run_lease_fence(journal, lease_token)
    }
}

async fn finalize(
    port: &ThreadBackedLoopTranscriptPort<InMemorySessionThreadService>,
    content: &str,
) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
    port.finalize_assistant_message(FinalizeAssistantMessage {
        reply: AssistantReply {
            content: content.to_string(),
        },
    })
    .await
    .map(|_| ())
}

/// The saving: while the lease the journal vouched for is still comfortably
/// live, further transcript writes reuse that answer instead of paying another
/// read on the two-connection journal pool.
#[tokio::test]
async fn an_affirmative_answer_is_reused_while_the_observed_lease_is_live() {
    let fixture = Fixture::new("memo-reuse").await;
    let lease_token = TurnLeaseToken::new();
    let journal = Arc::new(ScriptedJournal::new(Some(fixture.record(
        Some(lease_token),
        Some(Utc::now() + chrono::Duration::seconds(90)),
    ))));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    for index in 0..5 {
        finalize(&port, &format!("reply {index}")).await.unwrap();
    }

    assert_eq!(
        journal.reads(),
        1,
        "five transcript writes under one live lease must cost one ownership read"
    );
}

/// The zombie-worker case, and the bound that makes the memo safe.
///
/// The worker's lease is about to expire. It writes once (memoizing the
/// journal's "yes"), then stalls past that expiry — the shape of a worker whose
/// heartbeat loop died while its main task stayed blocked. Recovery meanwhile
/// requeued the run, so the journal no longer records this worker's token.
///
/// The memo must not carry the stale "yes" across the lease expiry: the second
/// write has to re-ask and be refused. Recovery can only requeue a run whose
/// lease has *already* expired, so a memo that never outlives the observed
/// expiry can never admit a write that lands after a requeue.
#[tokio::test]
async fn a_zombie_worker_is_refused_once_its_observed_lease_expiry_passes() {
    let fixture = Fixture::new("memo-zombie").await;
    let lease_token = TurnLeaseToken::new();
    // One second past the fence's skew margin gives loaded runners enough
    // scheduling slack while still exercising the real expiry arithmetic.
    let expires_at = Utc::now() + chrono::Duration::milliseconds(6_000);
    let journal = Arc::new(ScriptedJournal::new(Some(
        fixture.record(Some(lease_token), Some(expires_at)),
    )));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    finalize(&port, "before the lease expired").await.unwrap();
    assert_eq!(journal.reads(), 1);

    // Recovery reclaims: the run is requeued and carries no lease at all.
    journal.set_record(Some(fixture.record(None, None)));
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let error = finalize(&port, "after the lease expired")
        .await
        .expect_err("a lease-reclaimed worker must be fenced out of transcript writes");
    assert_eq!(
        error.kind,
        ironclaw_loop_contracts::AgentLoopHostErrorKind::TranscriptWriteFailed
    );
    assert_eq!(
        journal.reads(),
        2,
        "the memo must expire with the lease and force a fresh ownership read"
    );
}

/// Stop and Kill clear a still-live lease immediately. The memo intentionally
/// accepts that terminal control is observed only after its bounded window:
/// no replacement worker can claim a terminal run, and the cancellation port
/// remains the prompt path for stopping the loop itself.
#[tokio::test]
async fn a_terminal_control_action_is_observed_once_the_memo_lapses() {
    let fixture = Fixture::new("memo-terminal-control").await;
    let lease_token = TurnLeaseToken::new();
    // Keep the memo window wide enough that setup and the cached write remain
    // stable on loaded runners, then sleep past it below.
    let expires_at = Utc::now() + chrono::Duration::milliseconds(6_000);
    let journal = Arc::new(ScriptedJournal::new(Some(
        fixture.record(Some(lease_token), Some(expires_at)),
    )));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    finalize(&port, "before cancellation").await.unwrap();

    let mut cancelled = fixture.record(None, None);
    cancelled.status = TurnStatus::Cancelled;
    journal.set_record(Some(cancelled));

    finalize(&port, "inside the accepted memo window")
        .await
        .expect("the cached affirmative answer remains usable until its deadline");
    assert_eq!(journal.reads(), 1);

    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let error = finalize(&port, "after the memo window")
        .await
        .expect_err("terminal control must be observed once the memo lapses");
    assert_eq!(
        error.kind,
        ironclaw_loop_contracts::AgentLoopHostErrorKind::TranscriptWriteFailed
    );
    assert_eq!(
        journal.reads(),
        2,
        "the first write after the deadline must re-read terminal journal state"
    );
}

/// A replacement worker holding a different token is refused, and the refusal is
/// never memoized — every subsequent write re-asks and is refused again.
#[tokio::test]
async fn a_replacement_lease_refuses_every_write_and_is_never_memoized() {
    let fixture = Fixture::new("memo-replacement").await;
    let lease_token = TurnLeaseToken::new();
    let replacement = TurnLeaseToken::new();
    let journal = Arc::new(ScriptedJournal::new(Some(fixture.record(
        Some(replacement),
        Some(Utc::now() + chrono::Duration::seconds(90)),
    ))));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    finalize(&port, "first").await.unwrap_err();
    finalize(&port, "second").await.unwrap_err();

    assert_eq!(
        journal.reads(),
        2,
        "a refusal must never be cached; every write re-asks the journal"
    );
}

/// A record with no lease expiry says nothing about how long ownership holds,
/// so it must not be memoized at all.
#[tokio::test]
async fn a_record_without_a_lease_expiry_is_never_memoized() {
    let fixture = Fixture::new("memo-no-expiry").await;
    let lease_token = TurnLeaseToken::new();
    let journal = Arc::new(ScriptedJournal::new(Some(
        fixture.record(Some(lease_token), None),
    )));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    finalize(&port, "first").await.unwrap();
    finalize(&port, "second").await.unwrap();

    assert_eq!(
        journal.reads(),
        2,
        "without a known expiry the fence must keep asking per write"
    );
}

/// A lease whose remaining life is inside the skew margin is treated as already
/// gone: no memo, a fresh read per write. This is the fail-closed edge of the
/// bound — the margin absorbs clock skew between this worker and the journal.
#[tokio::test]
async fn a_lease_expiring_inside_the_skew_margin_is_never_memoized() {
    let fixture = Fixture::new("memo-margin").await;
    let lease_token = TurnLeaseToken::new();
    let journal = Arc::new(ScriptedJournal::new(Some(fixture.record(
        Some(lease_token),
        Some(Utc::now() + chrono::Duration::seconds(2)),
    ))));
    let port = fixture.port(Arc::clone(&journal), lease_token);

    finalize(&port, "first").await.unwrap();
    finalize(&port, "second").await.unwrap();

    assert_eq!(
        journal.reads(),
        2,
        "a lease inside the skew margin must not be memoized"
    );
}
