//! Tests for `RebornAutomationProductFacade::resolve_run_thread_scope`.
//!
//! Split from `automation.rs` to keep that file under the project's 800-900
//! line file-size target (architecture rule #5).  The listing-facade tests
//! remain in `automation.rs`; this file owns resolver-only coverage.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_product_workflow::{
    AutomationProductFacade, ProductAgentBoundCaller, RebornServicesErrorCode,
};
use ironclaw_triggers::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClearActiveFireRequest,
    FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, InMemoryTriggerRepository, TriggerError,
    TriggerId, TriggerRecord, TriggerRepository, TriggerRunRecord, TriggerSchedule,
    TriggerSourceKind, TriggerState,
};
use ironclaw_turns::TurnRunId;

use crate::automation::RebornAutomationProductFacade;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

fn tenant() -> TenantId {
    TenantId::new("tenant-alpha").expect("valid tenant")
}

fn caller() -> ProductAgentBoundCaller {
    ProductAgentBoundCaller {
        tenant_id: tenant(),
        user_id: UserId::new("user-alpha").expect("valid user"),
        agent_id: AgentId::new("agent-alpha").expect("valid agent"),
        project_id: Some(ProjectId::new("project-alpha").expect("valid project")),
    }
}

fn now() -> Timestamp {
    chrono::Utc::now()
}

fn make_record(trigger_id: TriggerId, caller: &ProductAgentBoundCaller) -> TriggerRecord {
    TriggerRecord {
        trigger_id,
        tenant_id: caller.tenant_id.clone(),
        creator_user_id: caller.user_id.clone(),
        agent_id: Some(caller.agent_id.clone()),
        project_id: caller.project_id.clone(),
        name: "Resolver test trigger".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::Cron {
            expression: "0 9 * * *".to_string(),
            timezone: "UTC".to_string(),
        },
        prompt: "run the daily task".to_string(),
        state: TriggerState::Scheduled,
        next_run_at: now(),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: now(),
    }
}

/// Seeds a trigger into the repo and marks a fire as accepted with the given
/// thread_id, returning the fired run_id.
async fn seed_accepted_run(
    repo: &InMemoryTriggerRepository,
    trigger_id: TriggerId,
    caller: &ProductAgentBoundCaller,
    thread_id: ThreadId,
) -> TurnRunId {
    let record = make_record(trigger_id, caller);
    repo.upsert_trigger(record.clone()).await.expect("upsert");
    let fire_slot = record.next_run_at;
    repo.claim_due_fire(ClaimDueFireRequest {
        tenant_id: caller.tenant_id.clone(),
        trigger_id,
        fire_slot,
        now: fire_slot,
    })
    .await
    .expect("claim due fire");
    let run_id = TurnRunId::new();
    repo.mark_fire_accepted(FireAcceptedRequest {
        tenant_id: caller.tenant_id.clone(),
        trigger_id,
        fire_slot,
        run_id,
        thread_id,
        submitted_at: fire_slot,
    })
    .await
    .expect("mark fire accepted");
    run_id
}

// ---------------------------------------------------------------------------
// Minimal scripted repository for error/timeout paths
// ---------------------------------------------------------------------------

/// A single-method scripted mock that fails or hangs on `find_trigger_run_by_thread_id`.
enum HangOrFail {
    Fail,
    Hang,
}

struct FailingThreadLookupRepository(HangOrFail);

#[async_trait]
impl TriggerRepository for FailingThreadLookupRepository {
    async fn upsert_trigger(&self, _: TriggerRecord) -> Result<(), TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn get_trigger(
        &self,
        _: TenantId,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn list_triggers(&self, _: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn list_scoped_triggers(
        &self,
        _: TenantId,
        _: UserId,
        _: Option<AgentId>,
        _: Option<ProjectId>,
        _: usize,
        _: &[ironclaw_triggers::TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn remove_trigger(
        &self,
        _: TenantId,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn remove_scoped_trigger(
        &self,
        _: TenantId,
        _: UserId,
        _: Option<AgentId>,
        _: Option<ProjectId>,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn set_scoped_trigger_state(
        &self,
        _: TenantId,
        _: UserId,
        _: Option<AgentId>,
        _: Option<ProjectId>,
        _: TriggerId,
        _: ironclaw_triggers::TriggerState,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn list_due_triggers(
        &self,
        _: Timestamp,
        _: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn list_active_triggers(&self, _: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn list_active_triggers_after(
        &self,
        _: Option<ActiveTriggerScanCursor>,
        _: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn claim_due_fire(
        &self,
        _: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn mark_fire_accepted(
        &self,
        _: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn mark_fire_replayed(
        &self,
        _: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn mark_fire_retryable_failed(
        &self,
        _: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn mark_fire_permanently_failed(
        &self,
        _: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn mark_fire_terminally_failed(
        &self,
        _: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn clear_active_fire(
        &self,
        _: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "stub".to_string(),
        })
    }
    async fn find_trigger_run_by_thread_id(
        &self,
        _: TenantId,
        _: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        match self.0 {
            HangOrFail::Fail => Err(TriggerError::Backend {
                reason: "injected backend error".to_string(),
            }),
            HangOrFail::Hang => std::future::pending().await,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_run_thread_scope_returns_matching_trigger_scope_via_direct_lookup() {
    // Positive path: a run row with a known thread_id exists and the trigger
    // belongs to the caller.  Uses InMemoryTriggerRepository (the real impl)
    // to verify the full chain from facade → repository → returned scope.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let trigger_id = TriggerId::new();
    let record = make_record(trigger_id, &c);
    let thread_id = ThreadId::new("01890f0f-test-7000-8000-000000000001").expect("valid thread id");
    seed_accepted_run(&repo, trigger_id, &c, thread_id.clone()).await;

    let facade = RebornAutomationProductFacade::new(repo);
    let resolved = facade
        .resolve_run_thread_scope(c.clone(), &thread_id)
        .await
        .expect("resolver succeeds")
        .expect("thread run is visible");

    assert_eq!(resolved.agent_id, record.agent_id);
    assert_eq!(resolved.project_id, record.project_id);
    assert_eq!(resolved.creator_user_id, record.creator_user_id);
}

#[tokio::test]
async fn resolve_run_thread_scope_returns_none_for_run_belonging_to_different_creator() {
    // Visibility-predicate test: a run exists in the repo, but its trigger was
    // created by a different user than the caller.  The resolver must return
    // Ok(None) — no authz bypass.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller(); // caller is user-alpha

    let other_caller = ProductAgentBoundCaller {
        tenant_id: c.tenant_id.clone(),
        user_id: UserId::new("user-beta").expect("valid user"),
        agent_id: c.agent_id.clone(),
        project_id: c.project_id.clone(),
    };
    let trigger_id = TriggerId::new();
    // The trigger belongs to user-beta (other_caller).
    let thread_id = ThreadId::new("01890f0f-test-7000-8000-000000000099").expect("valid thread id");
    seed_accepted_run(&repo, trigger_id, &other_caller, thread_id.clone()).await;

    // The caller (user-alpha) asks about a thread that belongs to user-beta's
    // trigger.  Must get None, not the scope of user-beta's trigger.
    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .resolve_run_thread_scope(c, &thread_id)
        .await
        .expect("resolver must not error");

    assert!(
        result.is_none(),
        "resolver must return None when the trigger belongs to a different creator_user_id"
    );
}

#[tokio::test]
async fn resolve_run_thread_scope_returns_none_for_unknown_thread_id() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let unknown_thread =
        ThreadId::new("01890f0f-test-7000-8000-000000009999").expect("valid thread id");
    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .resolve_run_thread_scope(c, &unknown_thread)
        .await
        .expect("resolver must not error on unknown thread");
    assert!(result.is_none());
}

#[tokio::test]
async fn resolve_run_thread_scope_backend_error_maps_to_unavailable() {
    let facade = RebornAutomationProductFacade::new(Arc::new(FailingThreadLookupRepository(
        HangOrFail::Fail,
    )));
    let thread_id = ThreadId::new("01890f0f-test-7000-8000-000000000001").expect("valid thread id");
    let error = facade
        .resolve_run_thread_scope(caller(), &thread_id)
        .await
        .expect_err("backend error must propagate as 503");
    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
}

/// Pin the contract that a trigger whose `agent_id` is NULL is NOT visible
/// to any `ProductAgentBoundCaller`.
///
/// `ProductAgentBoundCaller.agent_id` is a required (non-Option) field, so
/// `list_scoped_triggers` is always called with `agent_id = Some(caller_agent)`.
/// The NULL-safe equality in every backend therefore never returns a NULL-agent
/// trigger for such a caller, and `trigger_is_caller_visible` must match that
/// by requiring `trigger.agent_id == Some(caller.agent_id)`.  A NULL-agent
/// trigger has no caller that can own it through this path.
#[tokio::test]
async fn resolve_run_thread_scope_returns_none_for_trigger_with_no_agent_id() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    // Seed a trigger whose agent_id is None — simulating a trigger that was
    // stored without an explicit agent binding.
    let trigger_id = TriggerId::new();
    let mut null_agent_record = make_record(trigger_id, &c);
    null_agent_record.agent_id = None;
    repo.upsert_trigger(null_agent_record.clone())
        .await
        .expect("upsert null-agent trigger");

    // Manually claim and mark the fire accepted so a run row with a thread_id
    // exists — the resolver's thread lookup must actually find the run row
    // before the visibility predicate gates it.
    let fire_slot = null_agent_record.next_run_at;
    repo.claim_due_fire(ClaimDueFireRequest {
        tenant_id: c.tenant_id.clone(),
        trigger_id,
        fire_slot,
        now: fire_slot,
    })
    .await
    .expect("claim due fire");
    let run_id = TurnRunId::new();
    let thread_id = ThreadId::new("01890f0f-test-7000-8000-000000null01").expect("valid thread id");
    repo.mark_fire_accepted(FireAcceptedRequest {
        tenant_id: c.tenant_id.clone(),
        trigger_id,
        fire_slot,
        run_id,
        thread_id: thread_id.clone(),
        submitted_at: fire_slot,
    })
    .await
    .expect("mark fire accepted");

    // The resolver must return None: even though the run row exists and the
    // caller's tenant/user/project match, the NULL trigger agent_id does not
    // equal Some(caller.agent_id), so the visibility predicate rejects it.
    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .resolve_run_thread_scope(c, &thread_id)
        .await
        .expect("resolver must not error");

    assert!(
        result.is_none(),
        "NULL-agent trigger must be invisible to any ProductAgentBoundCaller"
    );
}

#[tokio::test]
async fn resolve_run_thread_scope_timeout_maps_to_unavailable() {
    let facade = RebornAutomationProductFacade::with_backend_timeout(
        Arc::new(FailingThreadLookupRepository(HangOrFail::Hang)),
        Duration::from_millis(10),
    );
    let thread_id = ThreadId::new("01890f0f-test-7000-8000-000000000001").expect("valid thread id");
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        facade.resolve_run_thread_scope(caller(), &thread_id),
    )
    .await
    .expect("facade timeout should complete promptly")
    .expect_err("stalled repository should time out");
    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
}
