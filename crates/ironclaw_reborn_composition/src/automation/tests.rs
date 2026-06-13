mod resolver_tests;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_product_workflow::{
    AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller,
    RebornAutomationRecentRunStatus, RebornAutomationRunStatus, RebornAutomationSource,
    RebornAutomationState, RebornServicesErrorCode, RebornServicesErrorKind,
};
use ironclaw_triggers::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClearActiveFireRequest,
    FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, InMemoryTriggerRepository, TriggerError,
    TriggerId, TriggerRecord, TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord,
    TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::TurnRunId;

use super::RebornAutomationProductFacade;

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

fn caller() -> ProductAgentBoundCaller {
    ProductAgentBoundCaller {
        tenant_id: TenantId::new("tenant-alpha").expect("valid tenant"),
        user_id: UserId::new("user-alpha").expect("valid user"),
        agent_id: AgentId::new("agent-alpha").expect("valid agent"),
        project_id: Some(ProjectId::new("project-alpha").expect("valid project")),
    }
}

fn automation_list_request(limit: usize, run_limit: usize) -> AutomationListRequest {
    AutomationListRequest { limit, run_limit }
}

fn now() -> Timestamp {
    chrono::Utc::now()
}

fn make_record(
    trigger_id: TriggerId,
    caller: &ProductAgentBoundCaller,
    state: TriggerState,
    name: &str,
    cron: &str,
) -> TriggerRecord {
    TriggerRecord {
        trigger_id,
        tenant_id: caller.tenant_id.clone(),
        creator_user_id: caller.user_id.clone(),
        agent_id: Some(caller.agent_id.clone()),
        project_id: caller.project_id.clone(),
        name: name.to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::Cron {
            expression: cron.to_string(),
            timezone: "UTC".to_string(),
        },
        completion_policy: ironclaw_triggers::TriggerCompletionPolicy::Recurring,
        prompt: "run the daily task".to_string(),
        state,
        next_run_at: now(),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: now(),
    }
}

fn make_run_record(trigger_id: TriggerId, status: TriggerRunHistoryStatus) -> TriggerRunRecord {
    let tenant_id = TenantId::new("tenant-alpha").expect("valid tenant");
    let fire_slot = now();
    TriggerRunRecord {
        tenant_id,
        trigger_id,
        fire_slot,
        run_id: Some(TurnRunId::new()),
        // Use a canonical UUID thread_id to represent a post-acceptance run.
        // Pre-acceptance rows would have thread_id: None.
        thread_id: Some(
            ThreadId::new("01890f0f-test-7000-8000-000000000001")
                .expect("valid canonical thread id"),
        ),
        status,
        submitted_at: now(),
        completed_at: None,
    }
}

// -------------------------------------------------------------------------
// Failing repository for error-path tests
// -------------------------------------------------------------------------

/// Single configurable mock covering every error/hang path the facade
/// exercises. `scoped` scripts `list_scoped_triggers`; `batch` scripts
/// `list_trigger_run_history_batch`; `thread_lookup` scripts
/// `find_trigger_run_by_thread_id`. All other trait methods are never
/// called by the facade and return a backend error.
#[allow(dead_code)]
enum ScriptedOutcome {
    Records(Vec<TriggerRecord>),
    Runs(HashMap<TriggerId, Vec<TriggerRunRecord>>),
    /// Used by `thread_lookup` — returns the given pair or None.
    ThreadResult(Box<Option<(TriggerRecord, TriggerRunRecord)>>),
    FailBackend,
    NotFound,
    Hang,
}

/// Recorded `(method, limit)` pairs for asserting bounded lookups.
type RecordedLimits = Arc<Mutex<Vec<(&'static str, usize)>>>;

struct ScriptedRepository {
    scoped: ScriptedOutcome,
    batch: ScriptedOutcome,
    /// Scripts `find_trigger_run_by_thread_id`. Defaults to `Ok(None)` when
    /// not set (None here means "method not scripted", not "no result found").
    thread_lookup: Option<ScriptedOutcome>,
    limits: Option<RecordedLimits>,
}

impl ScriptedRepository {
    fn backend_error() -> TriggerError {
        TriggerError::Backend {
            reason: "internal details".to_string(),
        }
    }
}

#[async_trait]
impl TriggerRepository for ScriptedRepository {
    async fn upsert_trigger(&self, _: TriggerRecord) -> Result<(), TriggerError> {
        Err(Self::backend_error())
    }

    async fn get_trigger(
        &self,
        _: TenantId,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn list_triggers(&self, _: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn list_scoped_triggers(
        &self,
        _: TenantId,
        _: UserId,
        _: Option<AgentId>,
        _: Option<ProjectId>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if let Some(limits) = &self.limits {
            limits.lock().expect("limits").push(("scoped", limit));
        }
        match &self.scoped {
            ScriptedOutcome::Records(records) => Ok(records.clone()),
            ScriptedOutcome::Runs(_) | ScriptedOutcome::ThreadResult(_) => {
                Err(Self::backend_error())
            }
            ScriptedOutcome::FailBackend => Err(Self::backend_error()),
            ScriptedOutcome::NotFound => Err(TriggerError::NotFound),
            ScriptedOutcome::Hang => std::future::pending().await,
        }
    }

    async fn find_trigger_run_by_thread_id(
        &self,
        _: TenantId,
        _: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        let Some(outcome) = &self.thread_lookup else {
            return Ok(None);
        };
        match outcome {
            ScriptedOutcome::ThreadResult(pair) => Ok(*pair.clone()),
            ScriptedOutcome::FailBackend => Err(Self::backend_error()),
            ScriptedOutcome::NotFound => Err(TriggerError::NotFound),
            ScriptedOutcome::Hang => std::future::pending().await,
            ScriptedOutcome::Records(_) | ScriptedOutcome::Runs(_) => Err(Self::backend_error()),
        }
    }

    async fn list_trigger_run_history_batch(
        &self,
        _: TenantId,
        _: &[TriggerId],
        limit: usize,
    ) -> Result<std::collections::HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        if let Some(limits) = &self.limits {
            limits.lock().expect("limits").push(("batch", limit));
        }
        match &self.batch {
            ScriptedOutcome::Records(_) | ScriptedOutcome::ThreadResult(_) => Ok(HashMap::new()),
            ScriptedOutcome::Runs(runs) => Ok(runs.clone()),
            ScriptedOutcome::FailBackend => Err(Self::backend_error()),
            ScriptedOutcome::NotFound => Err(TriggerError::NotFound),
            ScriptedOutcome::Hang => std::future::pending().await,
        }
    }

    async fn remove_trigger(
        &self,
        _: TenantId,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn remove_scoped_trigger(
        &self,
        _: TenantId,
        _: UserId,
        _: Option<AgentId>,
        _: Option<ProjectId>,
        _: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn list_due_triggers(
        &self,
        _: Timestamp,
        _: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn list_active_triggers(&self, _: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn list_active_triggers_after(
        &self,
        _: Option<ActiveTriggerScanCursor>,
        _: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn claim_due_fire(
        &self,
        _: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        Err(Self::backend_error())
    }

    async fn mark_fire_accepted(
        &self,
        _: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn mark_fire_replayed(
        &self,
        _: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn mark_fire_retryable_failed(
        &self,
        _: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn mark_fire_permanently_failed(
        &self,
        _: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn mark_fire_terminally_failed(
        &self,
        _: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }

    async fn clear_active_fire(
        &self,
        _: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(Self::backend_error())
    }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn automation_facade_forwards_caller_scope_to_repository() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    // Matching record
    let matching_id = TriggerId::new();
    let matching = make_record(
        matching_id,
        &c,
        TriggerState::Scheduled,
        "Daily task",
        "0 9 * * *",
    );
    repo.upsert_trigger(matching)
        .await
        .expect("upsert matching");

    // Non-matching record (different agent_id)
    let other_agent = AgentId::new("agent-beta").expect("valid agent");
    let non_matching_id = TriggerId::new();
    let mut non_matching = make_record(
        non_matching_id,
        &c,
        TriggerState::Scheduled,
        "Other task",
        "0 10 * * *",
    );
    non_matching.agent_id = Some(other_agent);
    repo.upsert_trigger(non_matching)
        .await
        .expect("upsert non-matching");

    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request(25, 0))
        .await
        .expect("list automations");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].automation_id, matching_id.to_string());
    assert_eq!(
        result[0].source,
        RebornAutomationSource::Schedule {
            cron: "0 9 * * *".to_string(),
            timezone: "UTC".to_string(),
        }
    );
    assert_eq!(result[0].state, RebornAutomationState::Scheduled);
    assert!(result[0].next_run_at.is_some());
    assert!(!result[0].is_active);
}

#[tokio::test]
async fn automation_facade_maps_all_trigger_states() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    // Completed is terminal, so its stale next_run_at slot is suppressed
    // on the wire; Scheduled and Paused keep theirs.
    let states = [
        (
            TriggerState::Scheduled,
            RebornAutomationState::Scheduled,
            true,
        ),
        (TriggerState::Paused, RebornAutomationState::Paused, true),
        (
            TriggerState::Completed,
            RebornAutomationState::Completed,
            false,
        ),
    ];

    for (trigger_state, expected_state, expect_next_run_at) in &states {
        let id = TriggerId::new();
        let record = make_record(id, &c, *trigger_state, "Test trigger", "0 9 * * *");
        repo.upsert_trigger(record).await.expect("upsert");

        let facade = RebornAutomationProductFacade::new(repo.clone());
        let result = facade
            .list_automations(c.clone(), automation_list_request(100, 0))
            .await
            .expect("list automations");

        let found = result
            .iter()
            .find(|a| a.automation_id == id.to_string())
            .expect("record present");
        assert_eq!(found.state, *expected_state);
        assert_eq!(
            found.next_run_at.is_some(),
            *expect_next_run_at,
            "next_run_at presence mismatch for {trigger_state:?}"
        );
    }
}

#[tokio::test]
async fn automation_facade_maps_run_history_and_skips_batch_when_run_limit_zero() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let id = TriggerId::new();

    let record = make_record(id, &c, TriggerState::Scheduled, "Test trigger", "0 9 * * *");
    repo.upsert_trigger(record).await.expect("upsert");

    // run_limit=0 -> empty recent_runs even if runs exist
    let facade = RebornAutomationProductFacade::new(repo.clone());
    let result_zero = facade
        .list_automations(c.clone(), automation_list_request(10, 0))
        .await
        .expect("list automations run_limit=0");

    assert_eq!(result_zero.len(), 1);
    assert!(
        result_zero[0].recent_runs.is_empty(),
        "run_limit=0 must produce empty recent_runs"
    );

    // run_limit>=1 -> runs are fetched. Since InMemoryTriggerRepository
    // populates runs only through lifecycle methods (claim_due_fire etc.),
    // we assert the call succeeds and returns the record (run count may be 0
    // because we have no fired history yet).
    let result_with_runs = facade
        .list_automations(c.clone(), automation_list_request(10, 5))
        .await
        .expect("list automations run_limit=5");

    assert_eq!(result_with_runs.len(), 1);
    // No fires were submitted, so runs is empty — but the facade must still
    // return the automation record (not filter it out on empty runs).
    assert_eq!(result_with_runs[0].automation_id, id.to_string());

    // Verify mapped run fields by constructing a run record directly and
    // using the private mapping helper.
    let run = make_run_record(id, TriggerRunHistoryStatus::Ok);
    let mapped = super::map_recent_run(&run).expect("map_recent_run");
    assert_eq!(mapped.status, RebornAutomationRecentRunStatus::Ok);
    assert!(mapped.run_id.is_some());
    assert!(mapped.submitted_at <= chrono::Utc::now());
    assert!(mapped.completed_at.is_none());
    assert!(
        mapped.thread_id.is_some(),
        "post-acceptance run must carry a canonical thread_id"
    );
}

#[tokio::test]
async fn automation_facade_maps_trigger_run_status_and_last_status() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let id = TriggerId::new();

    let mut record = make_record(id, &c, TriggerState::Scheduled, "Status test", "0 9 * * *");
    record.last_status = Some(ironclaw_triggers::TriggerRunStatus::Ok);
    repo.upsert_trigger(record).await.expect("upsert");

    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request(10, 0))
        .await
        .expect("list automations");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].last_status, Some(RebornAutomationRunStatus::Ok));

    // Verify Running status mapping via run record helper
    let run = make_run_record(id, TriggerRunHistoryStatus::Running);
    let mapped = super::map_recent_run(&run).expect("map_recent_run");
    assert_eq!(mapped.status, RebornAutomationRecentRunStatus::Running);
}

// Resolver tests (resolve_run_thread_scope_*) live in
// `crates/ironclaw_reborn_composition/src/automation_resolver_tests.rs`
// to keep this file under the project's 800-900 line file-size target.

#[tokio::test]
async fn automation_facade_maps_backend_error_to_unavailable() {
    let repo = Arc::new(ScriptedRepository {
        scoped: ScriptedOutcome::FailBackend,
        batch: ScriptedOutcome::FailBackend,
        thread_lookup: None,
        limits: None,
    });
    let facade = RebornAutomationProductFacade::new(repo);

    let error = facade
        .list_automations(caller(), automation_list_request(10, 5))
        .await
        .expect_err("backend error should propagate as 503");

    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);

    // The backend reason string must not leak into the rendered error.
    let debug_repr = format!("{error:?}");
    assert!(
        !debug_repr.contains("internal details"),
        "backend reason must not appear in rendered error: {debug_repr}"
    );
}

#[tokio::test]
async fn automation_facade_times_out_stalled_repository() {
    let facade = RebornAutomationProductFacade::with_backend_timeout(
        Arc::new(ScriptedRepository {
            scoped: ScriptedOutcome::Hang,
            batch: ScriptedOutcome::Hang,
            thread_lookup: None,
            limits: None,
        }),
        std::time::Duration::from_millis(10),
    );

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        facade.list_automations(caller(), automation_list_request(10, 5)),
    )
    .await
    .expect("facade timeout should complete promptly")
    .expect_err("stalled repository should time out");

    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
}

#[tokio::test]
async fn automation_facade_maps_backend_error_on_run_history_batch_to_unavailable() {
    let c = caller();
    let record = make_record(
        TriggerId::new(),
        &c,
        TriggerState::Scheduled,
        "Daily task",
        "0 9 * * *",
    );
    let facade = RebornAutomationProductFacade::new(Arc::new(ScriptedRepository {
        scoped: ScriptedOutcome::Records(vec![record]),
        batch: ScriptedOutcome::FailBackend,
        thread_lookup: None,
        limits: None,
    }));

    let error = facade
        .list_automations(c, automation_list_request(10, 5))
        .await
        .expect_err("batch backend error should propagate as 503");

    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);

    let debug_repr = format!("{error:?}");
    assert!(
        !debug_repr.contains("internal details"),
        "backend reason must not appear in rendered error: {debug_repr}"
    );
}

#[tokio::test]
async fn automation_facade_times_out_stalled_run_history_batch() {
    let c = caller();
    let record = make_record(
        TriggerId::new(),
        &c,
        TriggerState::Scheduled,
        "Daily task",
        "0 9 * * *",
    );
    let facade = RebornAutomationProductFacade::with_backend_timeout(
        Arc::new(ScriptedRepository {
            scoped: ScriptedOutcome::Records(vec![record]),
            batch: ScriptedOutcome::Hang,
            thread_lookup: None,
            limits: None,
        }),
        std::time::Duration::from_millis(10),
    );

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        facade.list_automations(c, automation_list_request(10, 5)),
    )
    .await
    .expect("facade timeout should complete promptly")
    .expect_err("stalled batch call should time out");

    assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
}

#[tokio::test]
async fn automation_facade_maps_not_found_trigger_error_to_404() {
    let facade = RebornAutomationProductFacade::new(Arc::new(ScriptedRepository {
        scoped: ScriptedOutcome::NotFound,
        batch: ScriptedOutcome::NotFound,
        thread_lookup: None,
        limits: None,
    }));

    let error = facade
        .list_automations(caller(), automation_list_request(10, 5))
        .await
        .expect_err("not-found error should propagate as 404");

    assert_eq!(error.code, RebornServicesErrorCode::NotFound);
    assert_eq!(error.kind, RebornServicesErrorKind::NotFound);
    assert_eq!(error.status_code, 404);
    assert!(!error.retryable);
}

#[tokio::test]
async fn automation_source_from_record_maps_cron_schedule() {
    let c = caller();
    let id = TriggerId::new();
    let record = make_record(id, &c, TriggerState::Scheduled, "Cron test", "*/5 * * * *");

    let source = super::automation_source_from_record(&record)
        .expect("cron schedule must map to Schedule source");

    assert_eq!(
        source,
        RebornAutomationSource::Schedule {
            cron: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
        }
    );
}

#[tokio::test]
async fn automation_source_from_record_includes_non_utc_timezone() {
    use ironclaw_triggers::TriggerSchedule;
    let c = caller();
    let id = TriggerId::new();
    let mut record = make_record(id, &c, TriggerState::Scheduled, "TZ test", "0 9 * * *");
    record.schedule = TriggerSchedule::cron_with_timezone("0 9 * * *", "America/New_York")
        .expect("valid tz schedule");

    let source = super::automation_source_from_record(&record)
        .expect("cron schedule must map to Schedule source");

    assert_eq!(
        source,
        RebornAutomationSource::Schedule {
            cron: "0 9 * * *".to_string(),
            timezone: "America/New_York".to_string(),
        }
    );
}
