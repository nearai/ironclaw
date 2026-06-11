use std::{collections::HashMap, sync::Arc, time::Duration};

use ironclaw_host_api::ThreadId;
use ironclaw_product_workflow::{
    AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller, RebornAutomationInfo,
    RebornAutomationRecentRunInfo, RebornAutomationRecentRunStatus, RebornAutomationRunStatus,
    RebornAutomationSource, RebornAutomationState, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind,
};
use ironclaw_triggers::{
    TriggerError, TriggerId, TriggerRecord, TriggerRepository, TriggerRunHistoryStatus,
    TriggerRunRecord, TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};

const AUTOMATION_BACKEND_TIMEOUT: Duration = Duration::from_secs(30);

/// WebUI panel facade for automation (trigger) listing.
///
/// ## Dual-access design
///
/// The model/agent-loop path uses the `builtin.trigger_list` capability with
/// the full pipeline (trust evaluation, approval gates) in
/// `ironclaw_host_runtime` first_party_tools::trigger_management. The panel
/// path (this facade) calls scoped repository methods directly, which is
/// correct for a user-direct fetch-and-render surface where the approval
/// pipeline would be wrong by design. Both paths converge on the same scoping
/// contract: tenant + creator_user + agent + project.
///
/// ## Future panel mutations
///
/// Any panel mutation added here must append an audit `RuntimeEvent` before
/// returning (precedent: `RebornRuntime::append_webui_loop_cancelled` in
/// `runtime.rs`).
#[derive(Clone)]
pub struct RebornAutomationProductFacade {
    trigger_repository: Arc<dyn TriggerRepository>,
    backend_timeout: Duration,
}

impl std::fmt::Debug for RebornAutomationProductFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornAutomationProductFacade")
            .field("trigger_repository", &"Arc<dyn TriggerRepository>")
            .finish()
    }
}

impl RebornAutomationProductFacade {
    pub(crate) fn new(trigger_repository: Arc<dyn TriggerRepository>) -> Self {
        Self {
            trigger_repository,
            backend_timeout: AUTOMATION_BACKEND_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_backend_timeout(
        trigger_repository: Arc<dyn TriggerRepository>,
        backend_timeout: Duration,
    ) -> Self {
        Self {
            trigger_repository,
            backend_timeout,
        }
    }
}

#[async_trait::async_trait]
impl AutomationProductFacade for RebornAutomationProductFacade {
    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        // Both repository calls share one deadline so the panel read budget is
        // backend_timeout total, not per call.
        let deadline = tokio::time::Instant::now() + self.backend_timeout;
        let records = tokio::time::timeout_at(
            deadline,
            self.trigger_repository.list_scoped_triggers(
                caller.tenant_id.clone(),
                caller.user_id.clone(),
                Some(caller.agent_id.clone()),
                caller.project_id.clone(),
                request.limit,
            ),
        )
        .await
        .map_err(|_| backend_timeout_error())?
        .map_err(map_trigger_error)?;

        if records.is_empty() || request.run_limit == 0 {
            return Ok(records
                .into_iter()
                .filter_map(|record| automation_info_from_record(record, &[]))
                .collect());
        }

        let trigger_ids: Vec<TriggerId> = records.iter().map(|r| r.trigger_id).collect();
        let mut runs_by_trigger: HashMap<TriggerId, Vec<TriggerRunRecord>> =
            tokio::time::timeout_at(
                deadline,
                self.trigger_repository.list_trigger_run_history_batch(
                    caller.tenant_id.clone(),
                    &trigger_ids,
                    request.run_limit,
                ),
            )
            .await
            .map_err(|_| backend_timeout_error())?
            .map_err(map_trigger_error)?;

        Ok(records
            .into_iter()
            .filter_map(|record| {
                let runs = runs_by_trigger
                    .remove(&record.trigger_id)
                    .unwrap_or_default();
                automation_info_from_record(record, &runs)
            })
            .collect())
    }
}

fn automation_info_from_record(
    record: TriggerRecord,
    runs: &[TriggerRunRecord],
) -> Option<RebornAutomationInfo> {
    let source = automation_source_from_record(&record)?;
    let is_active = record.has_active_fire();
    // Completed is terminal: the stored next_run_at is a stale past slot and
    // would render as a misleading "next run" date. Paused keeps its slot so
    // the panel can show when a resumed trigger would next fire.
    let next_run_at = match record.state {
        TriggerState::Completed => None,
        TriggerState::Scheduled | TriggerState::Paused => Some(record.next_run_at),
    };
    Some(RebornAutomationInfo {
        automation_id: record.trigger_id.to_string(),
        name: record.name,
        source,
        state: map_trigger_state(record.state),
        next_run_at,
        last_run_at: record.last_run_at,
        last_status: record.last_status.map(map_trigger_run_status),
        recent_runs: runs.iter().filter_map(map_recent_run).collect(),
        is_active,
        created_at: Some(record.created_at),
    })
}

/// Maps a trigger record's source kind + schedule to the wire DTO source.
///
/// This match is exhaustive on purpose: if `TriggerSourceKind` gains a new
/// variant, this function must be updated rather than silently returning
/// `None` for an unknown variant.
fn automation_source_from_record(record: &TriggerRecord) -> Option<RebornAutomationSource> {
    match record.source {
        TriggerSourceKind::Schedule => match &record.schedule {
            TriggerSchedule::Cron {
                expression,
                timezone,
            } => Some(RebornAutomationSource::Schedule {
                cron: expression.clone(),
                timezone: timezone.clone(),
            }),
        },
    }
}

/// Maps the repository trigger state to the wire DTO state.
///
/// Exhaustive — no wildcard arm so a new `TriggerState` variant is a compile
/// error here rather than a silent mapping gap.
fn map_trigger_state(state: TriggerState) -> RebornAutomationState {
    match state {
        TriggerState::Scheduled => RebornAutomationState::Scheduled,
        TriggerState::Paused => RebornAutomationState::Paused,
        TriggerState::Completed => RebornAutomationState::Completed,
    }
}

/// Maps the repository run status to the wire DTO run status.
///
/// Exhaustive — no wildcard arm so a new `TriggerRunStatus` variant is a
/// compile error here rather than a silent mapping gap.
fn map_trigger_run_status(status: TriggerRunStatus) -> RebornAutomationRunStatus {
    match status {
        TriggerRunStatus::Ok => RebornAutomationRunStatus::Ok,
        TriggerRunStatus::Error => RebornAutomationRunStatus::Error,
    }
}

fn map_recent_run(run: &TriggerRunRecord) -> Option<RebornAutomationRecentRunInfo> {
    let status = match run.status {
        TriggerRunHistoryStatus::Running => RebornAutomationRecentRunStatus::Running,
        TriggerRunHistoryStatus::Ok => RebornAutomationRecentRunStatus::Ok,
        TriggerRunHistoryStatus::Error => RebornAutomationRecentRunStatus::Error,
    };
    // TriggerRouteThreadId is a validated lower-hex string; ThreadId accepts
    // any non-empty value without path separators or control chars, so this
    // conversion cannot fail for constructible repository rows.
    let thread_id = ThreadId::new(run.thread_id.as_str()).ok()?; // silent-ok: structurally unreachable; defensive drop if ThreadId validation ever tightens
    Some(RebornAutomationRecentRunInfo {
        run_id: run.run_id,
        thread_id,
        fire_slot: Some(run.fire_slot),
        status,
        submitted_at: run.submitted_at,
        completed_at: run.completed_at,
    })
}

/// Shared 503 for repository calls that exceed the panel read deadline.
fn backend_timeout_error() -> RebornServicesError {
    services_error(
        RebornServicesErrorCode::Unavailable,
        RebornServicesErrorKind::ServiceUnavailable,
        503,
        true,
    )
}

fn map_trigger_error(error: TriggerError) -> RebornServicesError {
    match error {
        TriggerError::Backend { .. } => services_error(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            true,
        ),
        TriggerError::NotFound => services_error(
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
            404,
            false,
        ),
        TriggerError::InvalidTriggerId { .. }
        | TriggerError::InvalidFireIdentityComponent { .. }
        | TriggerError::InvalidRecord { .. }
        | TriggerError::InvalidPollerConfig { .. }
        | TriggerError::InvalidSchedule { .. }
        | TriggerError::InvalidMaterialization { .. } => internal_invariant(),
    }
}

fn services_error(
    code: RebornServicesErrorCode,
    kind: RebornServicesErrorKind,
    status_code: u16,
    retryable: bool,
) -> RebornServicesError {
    RebornServicesError {
        code,
        kind,
        status_code,
        retryable,
        field: None,
        validation_code: None,
    }
}

fn internal_invariant() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
    use ironclaw_product_workflow::{
        AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller,
        RebornAutomationRecentRunStatus, RebornAutomationRunStatus, RebornAutomationSource,
        RebornAutomationState, RebornServicesErrorCode, RebornServicesErrorKind,
    };
    use ironclaw_triggers::{
        ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClearActiveFireRequest,
        FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
        FireRetryableFailedRequest, FireTerminalFailedRequest, InMemoryTriggerRepository,
        TriggerError, TriggerId, TriggerRecord, TriggerRepository, TriggerRunHistoryStatus,
        TriggerRunRecord, TriggerSchedule, TriggerSourceKind, TriggerState,
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
        use ironclaw_triggers::TriggerFireIdentity;
        let tenant_id = TenantId::new("tenant-alpha").expect("valid tenant");
        let fire_slot = now();
        let identity = TriggerFireIdentity::new(tenant_id.clone(), trigger_id, fire_slot);
        TriggerRunRecord {
            tenant_id,
            trigger_id,
            fire_slot,
            run_id: Some(TurnRunId::new()),
            thread_id: identity.route_thread_id,
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
    /// `list_trigger_run_history_batch`. All other trait methods are never
    /// called by the facade and return a backend error.
    enum ScriptedOutcome {
        Records(Vec<TriggerRecord>),
        FailBackend,
        NotFound,
        Hang,
    }

    struct ScriptedRepository {
        scoped: ScriptedOutcome,
        batch: ScriptedOutcome,
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
            _: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            match &self.scoped {
                ScriptedOutcome::Records(records) => Ok(records.clone()),
                ScriptedOutcome::FailBackend => Err(Self::backend_error()),
                ScriptedOutcome::NotFound => Err(TriggerError::NotFound),
                ScriptedOutcome::Hang => std::future::pending().await,
            }
        }

        async fn list_trigger_run_history_batch(
            &self,
            _: TenantId,
            _: &[TriggerId],
            _: usize,
        ) -> Result<std::collections::HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError>
        {
            match &self.batch {
                ScriptedOutcome::Records(_) => Ok(std::collections::HashMap::new()),
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
        let _ = mapped.thread_id; // valid ThreadId produced
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

    #[tokio::test]
    async fn automation_facade_maps_backend_error_to_unavailable() {
        let repo = Arc::new(ScriptedRepository {
            scoped: ScriptedOutcome::FailBackend,
            batch: ScriptedOutcome::FailBackend,
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
}
