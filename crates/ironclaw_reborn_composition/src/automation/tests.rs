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
    AutomationListRequest {
        limit,
        run_limit,
        include_completed: false,
    }
}

fn automation_list_request_with_completed(limit: usize, run_limit: usize) -> AutomationListRequest {
    AutomationListRequest {
        limit,
        run_limit,
        include_completed: true,
    }
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
        _excluded_states: &[ironclaw_triggers::TriggerState],
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
async fn automation_facade_maps_active_trigger_states() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    // Only Scheduled and Paused triggers appear in the active list.
    // Completed is a terminal state (soft-complete after first fire) and must
    // be excluded so finished one-shots do not clutter the panel. Paused
    // keeps its next_run_at so the panel can show when a resumed trigger
    // would next fire; Scheduled also keeps its slot.
    let active_states = [
        (
            TriggerState::Scheduled,
            RebornAutomationState::Scheduled,
            true,
        ),
        (TriggerState::Paused, RebornAutomationState::Paused, true),
    ];

    for (trigger_state, expected_state, expect_next_run_at) in &active_states {
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
async fn automation_facade_excludes_completed_triggers_from_active_list() {
    // A Completed trigger is a fired one-shot that has soft-completed. It must
    // not appear in the active automations panel so users only see automations
    // that are still running or can be resumed.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    let scheduled_id = TriggerId::new();
    let completed_id = TriggerId::new();

    repo.upsert_trigger(make_record(
        scheduled_id,
        &c,
        TriggerState::Scheduled,
        "Active routine",
        "0 9 * * *",
    ))
    .await
    .expect("upsert scheduled");

    repo.upsert_trigger(make_record(
        completed_id,
        &c,
        TriggerState::Completed,
        "Fired one-shot",
        "0 9 * * *",
    ))
    .await
    .expect("upsert completed");

    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request(100, 0))
        .await
        .expect("list automations");

    let ids: Vec<String> = result.iter().map(|a| a.automation_id.clone()).collect();
    assert!(
        ids.contains(&scheduled_id.to_string()),
        "scheduled trigger must appear in active list"
    );
    assert!(
        !ids.contains(&completed_id.to_string()),
        "completed one-shot must be excluded from active list; got: {ids:?}"
    );
    assert_eq!(result.len(), 1, "only the active trigger should be listed");
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

    let source = super::automation_source_from_record(&record);

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

    let source = super::automation_source_from_record(&record);

    assert_eq!(
        source,
        RebornAutomationSource::Schedule {
            cron: "0 9 * * *".to_string(),
            timezone: "America/New_York".to_string(),
        }
    );
}

/// Regression test for pagination undercount bug.
///
/// Previously, `list_scoped_triggers` returned all states up to LIMIT and the
/// Rust layer filtered out Completed afterwards. If the first N=limit rows were
/// all Completed, the list would return 0 results even though active triggers
/// existed beyond those rows.
///
/// The fix pushes the Completed exclusion into SQL so the LIMIT applies to
/// already-filtered rows.  Using InMemoryTriggerRepository (which now also
/// applies the exclusion before truncation) exercises the same invariant.
#[tokio::test]
async fn automation_facade_excludes_completed_even_when_filling_limit() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    let completed_id = TriggerId::new();
    let scheduled_id = TriggerId::new();

    // Insert Completed first so it comes before Scheduled in creation order.
    repo.upsert_trigger(make_record(
        completed_id,
        &c,
        TriggerState::Completed,
        "Fired one-shot",
        "0 9 * * *",
    ))
    .await
    .expect("upsert completed");

    repo.upsert_trigger(make_record(
        scheduled_id,
        &c,
        TriggerState::Scheduled,
        "Active routine",
        "0 10 * * *",
    ))
    .await
    .expect("upsert scheduled");

    // limit=1 — if Completed consumed the slot, Scheduled would be invisible.
    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request(1, 0))
        .await
        .expect("list automations");

    let ids: Vec<String> = result.iter().map(|a| a.automation_id.clone()).collect();
    assert_eq!(
        result.len(),
        1,
        "exactly one active trigger should be returned; got: {ids:?}"
    );
    assert!(
        ids.contains(&scheduled_id.to_string()),
        "the active (Scheduled) trigger must be present; got: {ids:?}"
    );
    assert!(
        !ids.contains(&completed_id.to_string()),
        "completed trigger must be excluded even when it would fill the limit; got: {ids:?}"
    );
}

/// `include_completed = true` causes Completed automations to be returned
/// alongside active ones. The Completed entry must have `next_run_at = None`
/// (its stored slot is a stale past date and must not render as a future run).
#[tokio::test]
async fn automation_facade_include_completed_returns_completed_automations() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    let scheduled_id = TriggerId::new();
    let completed_id = TriggerId::new();

    repo.upsert_trigger(make_record(
        scheduled_id,
        &c,
        TriggerState::Scheduled,
        "Active routine",
        "0 9 * * *",
    ))
    .await
    .expect("upsert scheduled");

    repo.upsert_trigger(make_record(
        completed_id,
        &c,
        TriggerState::Completed,
        "Fired one-shot",
        "0 9 * * *",
    ))
    .await
    .expect("upsert completed");

    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request_with_completed(100, 0))
        .await
        .expect("list automations include_completed=true");

    let ids: Vec<String> = result.iter().map(|a| a.automation_id.clone()).collect();
    assert_eq!(
        result.len(),
        2,
        "both scheduled and completed triggers should be returned; got: {ids:?}"
    );
    assert!(
        ids.contains(&scheduled_id.to_string()),
        "scheduled trigger must be present; got: {ids:?}"
    );
    assert!(
        ids.contains(&completed_id.to_string()),
        "completed trigger must be present when include_completed=true; got: {ids:?}"
    );

    let completed = result
        .iter()
        .find(|a| a.automation_id == completed_id.to_string())
        .expect("completed automation must be in result");
    assert_eq!(
        completed.state,
        RebornAutomationState::Completed,
        "completed automation must map to Completed state"
    );
    assert!(
        completed.next_run_at.is_none(),
        "completed automation must have next_run_at=None (stale slot suppressed)"
    );
}

/// `include_completed = false` (default) preserves existing behavior:
/// Completed automations are excluded at the SQL layer so pagination LIMIT
/// applies only to active rows.
#[tokio::test]
async fn automation_facade_default_excludes_completed_automations() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();

    let scheduled_id = TriggerId::new();
    let completed_id = TriggerId::new();

    repo.upsert_trigger(make_record(
        scheduled_id,
        &c,
        TriggerState::Scheduled,
        "Active routine",
        "0 9 * * *",
    ))
    .await
    .expect("upsert scheduled");

    repo.upsert_trigger(make_record(
        completed_id,
        &c,
        TriggerState::Completed,
        "Fired one-shot",
        "0 9 * * *",
    ))
    .await
    .expect("upsert completed");

    let facade = RebornAutomationProductFacade::new(repo);
    let result = facade
        .list_automations(c, automation_list_request(100, 0))
        .await
        .expect("list automations include_completed=false (default)");

    let ids: Vec<String> = result.iter().map(|a| a.automation_id.clone()).collect();
    assert_eq!(
        result.len(),
        1,
        "only the active trigger should be listed by default; got: {ids:?}"
    );
    assert!(
        ids.contains(&scheduled_id.to_string()),
        "scheduled trigger must be present; got: {ids:?}"
    );
    assert!(
        !ids.contains(&completed_id.to_string()),
        "completed trigger must be excluded by default; got: {ids:?}"
    );
}
