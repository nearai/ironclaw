use std::collections::HashMap;

use ironclaw_events::{EventLogEntry, RuntimeEvent, RuntimeEventKind, sanitize_error_kind};
use ironclaw_host_api::InvocationId;

use crate::{
    CapabilityActivityProjection, CapabilityActivityStatus, RunProjectionStatus,
    RunStatusProjection,
};

#[derive(Default)]
pub(crate) struct RuntimeProjectionState {
    runs: HashMap<InvocationId, RunStatusProjection>,
    capability_activities: HashMap<InvocationId, CapabilityActivityProjection>,
}

impl RuntimeProjectionState {
    pub(crate) fn apply(&mut self, entry: &EventLogEntry<RuntimeEvent>) {
        apply_run_event(&mut self.runs, entry);
        apply_capability_activity_event(&mut self.capability_activities, entry);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (Vec<RunStatusProjection>, Vec<CapabilityActivityProjection>) {
        (
            self.runs.into_values().collect(),
            self.capability_activities.into_values().collect(),
        )
    }
}

pub(crate) fn sort_runs_for_projection(runs: &mut [RunStatusProjection]) {
    runs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.last_cursor.cmp(&left.last_cursor))
            .then_with(|| {
                left.invocation_id
                    .as_uuid()
                    .cmp(&right.invocation_id.as_uuid())
            })
    });
}

pub(crate) fn sort_capability_activities_for_projection(
    activities: &mut [CapabilityActivityProjection],
) {
    activities.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.last_cursor.cmp(&left.last_cursor))
            .then_with(|| {
                left.invocation_id
                    .as_uuid()
                    .cmp(&right.invocation_id.as_uuid())
            })
    });
}

fn apply_run_event(
    runs: &mut HashMap<InvocationId, RunStatusProjection>,
    entry: &EventLogEntry<RuntimeEvent>,
) {
    let event = &entry.record;
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
        .or_insert_with(|| CapabilityActivityProjection {
            invocation_id: event.scope.invocation_id,
            capability_id: event.capability_id.clone(),
            thread_id: event.scope.thread_id.clone(),
            status,
            provider: event.provider.clone(),
            runtime: event.runtime,
            process_id: event.process_id,
            output_bytes: event.output_bytes,
            error_kind: sanitized_error_kind.clone(),
            last_cursor: entry.cursor,
            updated_at: event.timestamp,
        });

    activity.status = status;
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

fn capability_activity_status_for_event(
    kind: RuntimeEventKind,
    current_status: Option<CapabilityActivityStatus>,
    has_active_process: bool,
) -> Option<CapabilityActivityStatus> {
    match kind {
        RuntimeEventKind::DispatchRequested => Some(CapabilityActivityStatus::Started),
        RuntimeEventKind::RuntimeSelected | RuntimeEventKind::ProcessStarted => {
            Some(CapabilityActivityStatus::Running)
        }
        RuntimeEventKind::DispatchSucceeded
            if has_active_process && current_status == Some(CapabilityActivityStatus::Running) =>
        {
            Some(CapabilityActivityStatus::Running)
        }
        RuntimeEventKind::DispatchSucceeded
            if has_active_process
                && matches!(
                    current_status,
                    Some(CapabilityActivityStatus::Failed) | Some(CapabilityActivityStatus::Killed)
                ) =>
        {
            current_status
        }
        RuntimeEventKind::DispatchSucceeded | RuntimeEventKind::ProcessCompleted => {
            Some(CapabilityActivityStatus::Completed)
        }
        RuntimeEventKind::DispatchFailed | RuntimeEventKind::ProcessFailed => {
            Some(CapabilityActivityStatus::Failed)
        }
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
    match kind {
        RuntimeEventKind::DispatchRequested
        | RuntimeEventKind::RuntimeSelected
        | RuntimeEventKind::ModelStarted
        | RuntimeEventKind::ModelCompleted
        | RuntimeEventKind::ModelFailed
        | RuntimeEventKind::ProcessStarted => RunProjectionStatus::Running,
        RuntimeEventKind::DispatchSucceeded
            if has_active_process && current_status == Some(RunProjectionStatus::Running) =>
        {
            RunProjectionStatus::Running
        }
        RuntimeEventKind::DispatchSucceeded
            if has_active_process
                && matches!(
                    current_status,
                    Some(RunProjectionStatus::Failed) | Some(RunProjectionStatus::Killed)
                ) =>
        {
            current_status.unwrap_or(RunProjectionStatus::Failed)
        }
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
        | RuntimeEventKind::HookFailed => current_status.unwrap_or(RunProjectionStatus::Running),
    }
}
