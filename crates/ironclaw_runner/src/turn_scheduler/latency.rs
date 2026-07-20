use std::time::Instant;

use ironclaw_turns::{TurnRunId, TurnRunWake, TurnScope, runner::ClaimedTurnRun};

pub(super) struct RunFields {
    scope: ScopeFields,
    run_id: TurnRunId,
}

pub(super) struct ScopeFields {
    tenant_id: String,
    agent_id: String,
    project_id: String,
    thread_id: String,
    owner_user_id: String,
}

impl ScopeFields {
    fn from_scope(scope: &TurnScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            thread_id: scope.thread_id.as_str().to_string(),
            owner_user_id: scope
                .explicit_owner_user_id()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
        }
    }
}

pub(super) fn run_fields_from_wake(
    started_at: Option<Instant>,
    wake: &TurnRunWake,
) -> Option<RunFields> {
    started_at?;
    Some(RunFields {
        scope: ScopeFields::from_scope(&wake.scope),
        run_id: wake.run_id,
    })
}

pub(super) fn scope_fields(
    started_at: Option<Instant>,
    scope: Option<&TurnScope>,
) -> Option<ScopeFields> {
    started_at?;
    scope.map(ScopeFields::from_scope)
}

pub(super) fn operation_ok(
    operation: &'static str,
    scope: &TurnScope,
    run_id: TurnRunId,
    started_at: Option<Instant>,
) {
    if started_at.is_none() {
        return;
    }
    trace_ok(
        operation,
        Some(&ScopeFields::from_scope(scope)),
        Some(run_id),
        started_at,
    );
}

pub(super) fn operation_error(
    operation: &'static str,
    scope: &TurnScope,
    run_id: TurnRunId,
    started_at: Option<Instant>,
    error_kind: &'static str,
) {
    if started_at.is_none() {
        return;
    }
    trace_error(
        operation,
        Some(&ScopeFields::from_scope(scope)),
        Some(run_id),
        started_at,
        error_kind,
    );
}

pub(super) fn notify_queued_run_result<E>(
    fields: Option<&RunFields>,
    started_at: Option<Instant>,
    result: &Result<(), E>,
) {
    let Some(fields) = fields else {
        return;
    };

    match result {
        Ok(()) => trace_ok(
            "notify_queued_run",
            Some(&fields.scope),
            Some(fields.run_id),
            started_at,
        ),
        Err(_) => trace_error(
            "notify_queued_run",
            Some(&fields.scope),
            Some(fields.run_id),
            started_at,
            "notify_error",
        ),
    }
}

pub(super) fn claim_next_runs_result(
    scope_filter: Option<&ScopeFields>,
    started_at: Option<Instant>,
    claimed_runs: &[ClaimedTurnRun],
) {
    let Some(first) = claimed_runs.first() else {
        trace_ok("claim_next_runs_empty", scope_filter, None, started_at);
        return;
    };
    let run_id = first.state.run_id;
    let scope = ScopeFields::from_scope(&first.state.scope);
    let claimed_count = claimed_runs.len();
    let run_id = run_id.to_string();
    ironclaw_observability::live_latency_trace_ok!(
        "turn_scheduler",
        "claim_next_runs",
        started_at,
        tenant_id = scope.tenant_id.as_str(),
        agent_id = scope.agent_id.as_str(),
        project_id = scope.project_id.as_str(),
        thread_id = scope.thread_id.as_str(),
        owner_user_id = scope.owner_user_id.as_str(),
        run_id = run_id.as_str(),
        claimed_count,
        "turn scheduler batch claim completed",
    );
}

pub(super) fn claim_next_runs_error(
    scope_filter: Option<&ScopeFields>,
    started_at: Option<Instant>,
) {
    trace_error(
        "claim_next_runs",
        scope_filter,
        None,
        started_at,
        "claim_error",
    );
}

fn trace_ok(
    operation: &'static str,
    scope: Option<&ScopeFields>,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    let tenant_id = scope.map(|scope| scope.tenant_id.as_str()).unwrap_or("");
    let agent_id = scope.map(|scope| scope.agent_id.as_str()).unwrap_or("");
    let project_id = scope.map(|scope| scope.project_id.as_str()).unwrap_or("");
    let thread_id = scope.map(|scope| scope.thread_id.as_str()).unwrap_or("");
    let owner_user_id = scope
        .map(|scope| scope.owner_user_id.as_str())
        .unwrap_or("");
    ironclaw_observability::live_latency_trace_ok!(
        "turn_scheduler",
        operation,
        started_at,
        tenant_id = tenant_id,
        agent_id = agent_id,
        project_id = project_id,
        thread_id = thread_id,
        owner_user_id = owner_user_id,
        run_id = run_id.as_str(),
        "turn scheduler operation completed",
    );
}

fn trace_error(
    operation: &'static str,
    scope: Option<&ScopeFields>,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
    error_kind: &'static str,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    let tenant_id = scope.map(|scope| scope.tenant_id.as_str()).unwrap_or("");
    let agent_id = scope.map(|scope| scope.agent_id.as_str()).unwrap_or("");
    let project_id = scope.map(|scope| scope.project_id.as_str()).unwrap_or("");
    let thread_id = scope.map(|scope| scope.thread_id.as_str()).unwrap_or("");
    let owner_user_id = scope
        .map(|scope| scope.owner_user_id.as_str())
        .unwrap_or("");
    ironclaw_observability::live_latency_trace_error!(
        "turn_scheduler",
        operation,
        started_at,
        error_kind,
        tenant_id = tenant_id,
        agent_id = agent_id,
        project_id = project_id,
        thread_id = thread_id,
        owner_user_id = owner_user_id,
        run_id = run_id.as_str(),
        "turn scheduler operation failed",
    );
}
