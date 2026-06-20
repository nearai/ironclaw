use std::{cmp::Ordering, collections::HashMap};

use ironclaw_events::{EventLogEntry, RuntimeEvent, RuntimeEventKind, sanitize_error_kind};
use ironclaw_host_api::InvocationId;

use crate::{
    CapabilityActivityProjection, CapabilityActivityStatus, RunProjectionStatus,
    RunStatusProjection,
};

pub(crate) struct RuntimeProjectionState {
    runs: HashMap<InvocationId, RunStatusProjection>,
    capability_activities: HashMap<InvocationId, CapabilityActivityProjection>,
    capability_activity_output_limit: Option<usize>,
}

impl RuntimeProjectionState {
    pub(crate) fn without_capability_activity_output_limit() -> Self {
        Self {
            runs: HashMap::new(),
            capability_activities: HashMap::new(),
            capability_activity_output_limit: None,
        }
    }

    pub(crate) fn with_capability_activity_output_limit(limit: usize) -> Self {
        Self {
            runs: HashMap::new(),
            capability_activities: HashMap::new(),
            capability_activity_output_limit: Some(limit),
        }
    }

    pub(crate) fn apply(&mut self, entry: &EventLogEntry<RuntimeEvent>) {
        apply_run_event(&mut self.runs, entry);
        apply_capability_activity_event(&mut self.capability_activities, entry);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (Vec<RunStatusProjection>, Vec<CapabilityActivityProjection>) {
        let mut runs = self.runs.into_values().collect::<Vec<_>>();
        let mut capability_activities = self.capability_activities.into_values().collect();
        sort_runs_for_projection(&mut runs);
        enforce_capability_activity_output_limit(
            &mut capability_activities,
            self.capability_activity_output_limit,
        );
        (runs, capability_activities)
    }
}

fn enforce_capability_activity_output_limit(
    activities: &mut Vec<CapabilityActivityProjection>,
    limit: Option<usize>,
) {
    let Some(limit) = limit else {
        return;
    };
    if activities.len() > limit {
        let split_index = limit.saturating_sub(1);
        activities
            .select_nth_unstable_by(split_index, compare_capability_activities_for_output_window);
        activities.truncate(limit);
    }
    sort_capability_activities_for_projection(activities);
}

fn sort_runs_for_projection(runs: &mut [RunStatusProjection]) {
    runs.sort_by(compare_runs_for_projection);
}

fn sort_capability_activities_for_projection(activities: &mut [CapabilityActivityProjection]) {
    activities.sort_by(compare_capability_activities_for_projection);
}

fn compare_runs_for_projection(
    left: &RunStatusProjection,
    right: &RunStatusProjection,
) -> Ordering {
    compare_projection_order(
        &left.updated_at,
        &right.updated_at,
        left.last_cursor,
        right.last_cursor,
        &left.invocation_id,
        &right.invocation_id,
    )
}

fn compare_capability_activities_for_projection(
    left: &CapabilityActivityProjection,
    right: &CapabilityActivityProjection,
) -> Ordering {
    compare_projection_order_ascending(
        left.activity_order_cursor(),
        right.activity_order_cursor(),
        &left.invocation_id,
        &right.invocation_id,
    )
}

fn compare_capability_activities_for_output_window(
    left: &CapabilityActivityProjection,
    right: &CapabilityActivityProjection,
) -> Ordering {
    compare_projection_order(
        &left.updated_at,
        &right.updated_at,
        left.last_cursor,
        right.last_cursor,
        &left.invocation_id,
        &right.invocation_id,
    )
}

fn compare_projection_order(
    left_updated_at: &ironclaw_host_api::Timestamp,
    right_updated_at: &ironclaw_host_api::Timestamp,
    left_cursor: ironclaw_events::EventCursor,
    right_cursor: ironclaw_events::EventCursor,
    left_invocation_id: &InvocationId,
    right_invocation_id: &InvocationId,
) -> Ordering {
    right_updated_at
        .cmp(left_updated_at)
        .then_with(|| right_cursor.cmp(&left_cursor))
        .then_with(|| {
            left_invocation_id
                .as_uuid()
                .cmp(&right_invocation_id.as_uuid())
        })
}

fn compare_projection_order_ascending(
    left_cursor: ironclaw_events::EventCursor,
    right_cursor: ironclaw_events::EventCursor,
    left_invocation_id: &InvocationId,
    right_invocation_id: &InvocationId,
) -> Ordering {
    left_cursor.cmp(&right_cursor).then_with(|| {
        left_invocation_id
            .as_uuid()
            .cmp(&right_invocation_id.as_uuid())
    })
}

fn preserve_status_on_dispatch_success<S>(
    has_active_process: bool,
    current_status: Option<S>,
    running_status: S,
    terminal_statuses: &[S],
) -> Option<S>
where
    S: Copy + PartialEq,
{
    if !has_active_process {
        return None;
    }
    if current_status == Some(running_status) {
        return current_status;
    }
    if current_status.is_some_and(|status| terminal_statuses.contains(&status)) {
        return current_status;
    }
    None
}

pub(crate) fn capability_activity_transition_for_entry(
    entry: &EventLogEntry<RuntimeEvent>,
) -> Option<CapabilityActivityProjection> {
    let status = capability_activity_status_for_event(entry.record.kind, None, false)?;
    if !matches!(
        status,
        CapabilityActivityStatus::Started | CapabilityActivityStatus::Running
    ) {
        return None;
    }
    Some(capability_activity_projection_for_entry(entry, status))
}

fn apply_run_event(
    runs: &mut HashMap<InvocationId, RunStatusProjection>,
    entry: &EventLogEntry<RuntimeEvent>,
) {
    let event = &entry.record;
    if matches!(
        event.kind,
        RuntimeEventKind::CapabilityActivityRequested
            | RuntimeEventKind::CapabilityActivitySucceeded
            | RuntimeEventKind::CapabilityActivityFailed
    ) {
        return;
    }
    let existing = runs.get(&event.scope.invocation_id);
    let status = run_status_for_event(
        event.kind,
        existing.map(|run| run.status),
        existing.and_then(|run| run.process_id).is_some(),
    );
    let sanitized_error_kind = event.error_kind.clone().map(sanitize_error_kind);
    let run = runs
        .entry(event.scope.invocation_id)
        .or_insert_with(|| RunStatusProjection {
            invocation_id: event.scope.invocation_id,
            capability_id: event.capability_id.clone(),
            thread_id: event.scope.thread_id.clone(),
            status,
            provider: event.provider.clone(),
            runtime: event.runtime,
            process_id: event.process_id,
            error_kind: sanitized_error_kind.clone(),
            last_cursor: entry.cursor,
            updated_at: event.timestamp,
        });

    run.status = status;
    if !matches!(
        event.kind,
        RuntimeEventKind::AssistantReplyFinalized
            | RuntimeEventKind::LoopCompleted
            | RuntimeEventKind::LoopCancelled
            | RuntimeEventKind::LoopFailed
    ) {
        run.capability_id = event.capability_id.clone();
    }
    run.thread_id = event.scope.thread_id.clone();
    if event.provider.is_some() {
        run.provider = event.provider.clone();
    }
    if event.runtime.is_some() {
        run.runtime = event.runtime;
    }
    if event.process_id.is_some() {
        run.process_id = event.process_id;
    }
    if matches!(
        event.kind,
        RuntimeEventKind::AssistantReplyFinalized
            | RuntimeEventKind::LoopCompleted
            | RuntimeEventKind::LoopCancelled
    ) {
        run.error_kind = None;
    } else if sanitized_error_kind.is_some() {
        run.error_kind = sanitized_error_kind;
    }
    run.last_cursor = entry.cursor;
    run.updated_at = event.timestamp;
}

fn apply_capability_activity_event(
    activities: &mut HashMap<InvocationId, CapabilityActivityProjection>,
    entry: &EventLogEntry<RuntimeEvent>,
) {
    let event = &entry.record;
    let existing = activities.get(&event.scope.invocation_id);
    let Some(status) = capability_activity_status_for_event(
        event.kind,
        existing.map(|activity| activity.status),
        existing.and_then(|activity| activity.process_id).is_some(),
    ) else {
        return;
    };
    let sanitized_error_kind = event.error_kind.clone().map(sanitize_error_kind);
    let activity = activities
        .entry(event.scope.invocation_id)
        .or_insert_with(|| capability_activity_projection_for_entry(entry, status));

    activity.status = status;
    if event.parent_invocation_id.is_some() {
        activity.run_id = event.parent_invocation_id;
    }
    activity.capability_id = event.capability_id.clone();
    activity.thread_id = event.scope.thread_id.clone();
    if event.provider.is_some() {
        activity.provider = event.provider.clone();
    }
    if event.runtime.is_some() {
        activity.runtime = event.runtime;
    }
    if event.process_id.is_some() {
        activity.process_id = event.process_id;
    }
    if event.output_bytes.is_some() {
        activity.output_bytes = event.output_bytes;
    }
    if matches!(
        status,
        CapabilityActivityStatus::Started
            | CapabilityActivityStatus::Running
            | CapabilityActivityStatus::Completed
    ) {
        activity.error_kind = None;
    } else if sanitized_error_kind.is_some() {
        activity.error_kind = sanitized_error_kind;
    }
    activity.last_cursor = entry.cursor;
    activity.updated_at = event.timestamp;
}

fn capability_activity_projection_for_entry(
    entry: &EventLogEntry<RuntimeEvent>,
    status: CapabilityActivityStatus,
) -> CapabilityActivityProjection {
    let event = &entry.record;
    CapabilityActivityProjection {
        invocation_id: event.scope.invocation_id,
        run_id: event.parent_invocation_id,
        capability_id: event.capability_id.clone(),
        thread_id: event.scope.thread_id.clone(),
        status,
        provider: event.provider.clone(),
        runtime: event.runtime,
        process_id: event.process_id,
        output_bytes: event.output_bytes,
        error_kind: event.error_kind.clone().map(sanitize_error_kind),
        first_cursor: entry.cursor,
        last_cursor: entry.cursor,
        updated_at: event.timestamp,
    }
}

fn capability_activity_status_for_event(
    kind: RuntimeEventKind,
    current_status: Option<CapabilityActivityStatus>,
    has_active_process: bool,
) -> Option<CapabilityActivityStatus> {
    if matches!(
        kind,
        RuntimeEventKind::DispatchSucceeded | RuntimeEventKind::CapabilityActivitySucceeded
    ) && let Some(status) = preserve_status_on_dispatch_success(
        has_active_process,
        current_status,
        CapabilityActivityStatus::Running,
        &[
            CapabilityActivityStatus::Failed,
            CapabilityActivityStatus::Killed,
        ],
    ) {
        return Some(status);
    }
    match kind {
        RuntimeEventKind::DispatchRequested | RuntimeEventKind::CapabilityActivityRequested => {
            Some(CapabilityActivityStatus::Started)
        }
        RuntimeEventKind::RuntimeSelected | RuntimeEventKind::ProcessStarted => {
            Some(CapabilityActivityStatus::Running)
        }
        RuntimeEventKind::DispatchSucceeded
        | RuntimeEventKind::CapabilityActivitySucceeded
        | RuntimeEventKind::ProcessCompleted => Some(CapabilityActivityStatus::Completed),
        RuntimeEventKind::DispatchFailed
        | RuntimeEventKind::CapabilityActivityFailed
        | RuntimeEventKind::ProcessFailed => Some(CapabilityActivityStatus::Failed),
        RuntimeEventKind::ProcessKilled => Some(CapabilityActivityStatus::Killed),
        RuntimeEventKind::ModelStarted
        | RuntimeEventKind::ModelCompleted
        | RuntimeEventKind::ModelFailed
        | RuntimeEventKind::AssistantReplyFinalized
        | RuntimeEventKind::LoopCompleted
        | RuntimeEventKind::LoopCancelled
        | RuntimeEventKind::LoopFailed
        | RuntimeEventKind::HookDispatched
        | RuntimeEventKind::HookDecisionEmitted
        | RuntimeEventKind::HookFailed => None,
    }
}

fn run_status_for_event(
    kind: RuntimeEventKind,
    current_status: Option<RunProjectionStatus>,
    has_active_process: bool,
) -> RunProjectionStatus {
    if matches!(kind, RuntimeEventKind::DispatchSucceeded)
        && let Some(status) = preserve_status_on_dispatch_success(
            has_active_process,
            current_status,
            RunProjectionStatus::Running,
            &[RunProjectionStatus::Failed, RunProjectionStatus::Killed],
        )
    {
        return status;
    }
    match kind {
        RuntimeEventKind::DispatchRequested
        | RuntimeEventKind::RuntimeSelected
        | RuntimeEventKind::ModelStarted
        | RuntimeEventKind::ModelCompleted
        | RuntimeEventKind::ModelFailed
        | RuntimeEventKind::ProcessStarted => RunProjectionStatus::Running,
        RuntimeEventKind::DispatchSucceeded
        | RuntimeEventKind::AssistantReplyFinalized
        | RuntimeEventKind::LoopCompleted
        | RuntimeEventKind::ProcessCompleted => RunProjectionStatus::Completed,
        RuntimeEventKind::LoopCancelled => RunProjectionStatus::Cancelled,
        RuntimeEventKind::DispatchFailed
        | RuntimeEventKind::LoopFailed
        | RuntimeEventKind::ProcessFailed => RunProjectionStatus::Failed,
        RuntimeEventKind::ProcessKilled => RunProjectionStatus::Killed,
        RuntimeEventKind::HookDispatched
        | RuntimeEventKind::HookDecisionEmitted
        | RuntimeEventKind::HookFailed
        | RuntimeEventKind::CapabilityActivityRequested
        | RuntimeEventKind::CapabilityActivitySucceeded
        | RuntimeEventKind::CapabilityActivityFailed => {
            current_status.unwrap_or(RunProjectionStatus::Running)
        }
    }
}
