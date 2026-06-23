use std::{collections::HashMap, sync::Arc, time::Duration};

use ironclaw_host_api::ThreadId;
use ironclaw_product_workflow::{
    AutomationListRequest, AutomationProductFacade, ProductAgentBoundCaller, RebornAutomationInfo,
    RebornAutomationRecentRunInfo, RebornAutomationRecentRunStatus, RebornAutomationRunStatus,
    RebornAutomationSource, RebornAutomationState, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, TriggerRunThreadScope,
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
    /// Whether the background trigger poller is running. Surfaced to the WebUI
    /// so the panel can warn that listed automations will not fire while
    /// scheduling is off. Defaults to `true`; production wiring sets the real
    /// value from runtime readiness.
    scheduler_enabled: bool,
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
            scheduler_enabled: true,
        }
    }

    /// Set whether the background trigger poller (scheduler) is running. Wired
    /// by WebUI composition from runtime readiness.
    pub(crate) fn with_scheduler_enabled(mut self, scheduler_enabled: bool) -> Self {
        self.scheduler_enabled = scheduler_enabled;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_backend_timeout(
        trigger_repository: Arc<dyn TriggerRepository>,
        backend_timeout: Duration,
    ) -> Self {
        Self {
            trigger_repository,
            backend_timeout,
            scheduler_enabled: true,
        }
    }
}

#[async_trait::async_trait]
impl AutomationProductFacade for RebornAutomationProductFacade {
    fn scheduler_enabled(&self) -> bool {
        self.scheduler_enabled
    }

    async fn list_automations(
        &self,
        caller: ProductAgentBoundCaller,
        request: AutomationListRequest,
    ) -> Result<Vec<RebornAutomationInfo>, RebornServicesError> {
        // Both repository calls share one deadline so the panel read budget is
        // backend_timeout total, not per call.
        let deadline = tokio::time::Instant::now() + self.backend_timeout;
        // Soft-completed one-shots have fired and will never run again. By
        // default exclude them from the active automations panel so they do
        // not clutter the list with stale entries. When `include_completed` is
        // set the exclusion slice is empty, returning all states including
        // Completed. The exclusion is pushed to the SQL layer so LIMIT is
        // applied after filtering (fixes pagination undercount).
        //
        // Scheduled and Paused triggers are always kept; Paused triggers may be
        // resumed by the user and still have a meaningful next_run_at slot to
        // display.
        let excluded_states: &[TriggerState] = if request.include_completed {
            &[]
        } else {
            &[TriggerState::Completed]
        };
        let records = tokio::time::timeout_at(
            deadline,
            self.trigger_repository.list_scoped_triggers(
                caller.tenant_id.clone(),
                caller.user_id.clone(),
                Some(caller.agent_id.clone()),
                caller.project_id.clone(),
                request.limit,
                excluded_states,
            ),
        )
        .await
        .map_err(|_| backend_timeout_error())?
        .map_err(map_trigger_error)?;

        if records.is_empty() || request.run_limit == 0 {
            return Ok(records
                .into_iter()
                .map(|record| automation_info_from_record(record, &[]))
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
            .map(|record| {
                let runs = runs_by_trigger
                    .remove(&record.trigger_id)
                    .unwrap_or_default();
                automation_info_from_record(record, &runs)
            })
            .collect())
    }

    async fn resolve_run_thread_scope(
        &self,
        caller: ProductAgentBoundCaller,
        thread_id: &ThreadId,
    ) -> Result<Option<TriggerRunThreadScope>, RebornServicesError> {
        let deadline = tokio::time::Instant::now() + self.backend_timeout;

        // Direct thread-id-keyed lookup — O(1) repository query instead of the
        // prior O(triggers × runs) linear scan.
        let result = tokio::time::timeout_at(
            deadline,
            self.trigger_repository
                .find_trigger_run_by_thread_id(caller.tenant_id.clone(), thread_id),
        )
        .await
        .map_err(|_| backend_timeout_error())?
        .map_err(map_trigger_error)?;

        let Some((trigger, _run)) = result else {
            return Ok(None);
        };

        // Apply the same caller-visibility predicate that `list_scoped_triggers`
        // enforces: the trigger must match tenant + creator_user_id + agent_id +
        // project_id.  A mismatch means this caller cannot see the trigger in
        // `list_automations`, so it must not see it here either.
        if !trigger_is_caller_visible(&trigger, &caller) {
            return Ok(None);
        }

        Ok(Some(TriggerRunThreadScope {
            agent_id: trigger.agent_id,
            project_id: trigger.project_id,
            creator_user_id: trigger.creator_user_id,
        }))
    }
}

/// Returns `true` when `trigger` belongs to the caller — i.e. the trigger
/// matches the exact caller scope that `list_scoped_triggers` enforces
/// (tenant_id + creator_user_id + agent_id + project_id).  This mirrors the
/// filter in `InMemoryTriggerRepository::list_scoped_triggers` and the SQL
/// WHERE clause used by the libSQL and PostgreSQL backends:
///
/// ```text
/// WHERE tenant_id    = <caller.tenant_id>
///   AND creator_user_id = <caller.user_id>
///   AND agent_id    IS <caller.agent_id>   -- NULL-safe equality
///   AND project_id  IS <caller.project_id> -- NULL-safe equality
/// ```
///
/// **Visibility for listing vs. thread authorization are deliberately
/// decoupled.**  The default `list_automations` response excludes
/// `TriggerState::Completed` triggers (soft-completed fire-once triggers) to
/// avoid cluttering the active automations panel.  However, completed triggers
/// remain queryable (`include_completed = true` / `trigger_list` model tool)
/// and their run threads remain accessible — the history is retained user data.
/// This resolver intentionally does NOT filter on trigger state: a completed
/// trigger's run threads must stay resolvable so the user can always reach
/// their own trigger history.  Adding a `Completed` exclusion here would
/// regress access to run threads that are still valid retained data.
///
/// **None-agent triggers are never visible** through this path.
/// `ProductAgentBoundCaller` always carries a concrete `AgentId` (it is a
/// required field, not `Option`), so `list_scoped_triggers` is always called
/// with `agent_id = Some(caller.agent_id)`.  The NULL-safe comparison in the
/// storage backends therefore never returns a trigger whose stored `agent_id`
/// is NULL.  This predicate matches that contract: `trigger.agent_id ==
/// Some(caller.agent_id)` correctly excludes NULL-agent triggers rather than
/// granting phantom access.  The service-layer agent-id fallback in
/// `check_automation_trigger_access` is only reachable via non-production
/// facades that do not go through `ProductAgentBoundCaller`.
///
/// A `false` result causes `resolve_run_thread_scope` to return `Ok(None)` (404
/// upstream) without leaking the existence of the trigger to an unauthorized
/// caller.
fn trigger_is_caller_visible(trigger: &TriggerRecord, caller: &ProductAgentBoundCaller) -> bool {
    trigger.tenant_id == caller.tenant_id
        && trigger.creator_user_id == caller.user_id
        && trigger.agent_id == Some(caller.agent_id.clone())
        && trigger.project_id == caller.project_id
}

fn automation_info_from_record(
    record: TriggerRecord,
    runs: &[TriggerRunRecord],
) -> RebornAutomationInfo {
    let source = automation_source_from_record(&record);
    let is_active = record.has_active_fire();
    // Completed is terminal: the stored next_run_at is a stale past slot and
    // would render as a misleading "next run" date. Paused keeps its slot so
    // the panel can show when a resumed trigger would next fire.
    let next_run_at = match record.state {
        TriggerState::Completed => None,
        TriggerState::Scheduled | TriggerState::Paused => Some(record.next_run_at),
    };
    RebornAutomationInfo {
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
    }
}

/// Maps a trigger record's source kind + schedule to the wire DTO source.
///
/// This match is exhaustive on purpose: if `TriggerSourceKind` gains a new
/// variant or `TriggerSchedule` gains a new arm, the compiler rejects the
/// build here — preventing any new schedule type from being silently dropped.
fn automation_source_from_record(record: &TriggerRecord) -> RebornAutomationSource {
    match record.source {
        TriggerSourceKind::Schedule => match &record.schedule {
            TriggerSchedule::Cron {
                expression,
                timezone,
            } => RebornAutomationSource::Schedule {
                cron: expression.clone(),
                timezone: timezone.clone(),
            },
            TriggerSchedule::Once { at, timezone } => RebornAutomationSource::Once {
                at: at.to_rfc3339(),
                timezone: timezone.clone(),
            },
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
    Some(RebornAutomationRecentRunInfo {
        run_id: run.run_id,
        // `thread_id` is `None` until fire acceptance; pre-acceptance and
        // pre-submit-failure rows carry no canonical thread. The WebUI panel
        // must not render a chat link when this field is absent.
        thread_id: run.thread_id.clone(),
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
mod tests;
