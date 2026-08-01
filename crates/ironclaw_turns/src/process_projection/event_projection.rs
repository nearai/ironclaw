//! Durable process-journal entries projected into the agent-turn event view.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::resource::{ResourceScope, SYSTEM_RESERVED_ID};
use ironclaw_processes::{
    ProcessJournalCursor, ProcessJournalEntry, ProcessJournalKind, ProcessJournalPage,
    ProcessJournalSource, ProcessKind, ProcessLifecycleStatus, ProcessSuspension,
    ProcessSuspensionKind,
};

use crate::{
    EventCursor, TurnError, TurnEventKind, TurnLifecycleEvent, TurnRunId, TurnScope, TurnStatus,
    events::{
        TurnBlockedGateKind, TurnBlockedGateMetadata, TurnEventPage, TurnEventProjectionSource,
    },
};

#[derive(Clone)]
pub struct TurnEventProjectionFromProcessJournal {
    source: Arc<dyn ProcessJournalSource<Error = TurnError>>,
}

impl TurnEventProjectionFromProcessJournal {
    pub fn new(source: Arc<dyn ProcessJournalSource<Error = TurnError>>) -> Self {
        Self { source }
    }
}

#[async_trait]
impl TurnEventProjectionSource for TurnEventProjectionFromProcessJournal {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&ironclaw_host_api::ids::UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let page = self
            .source
            .read_process_journal_after(
                &scope.to_resource_scope(),
                owner_user_id,
                after.map(|cursor| ProcessJournalCursor(cursor.0)),
                limit,
            )
            .await?;
        turn_event_page_from_process_journal(page)
    }

    async fn read_turn_event_log_after(
        &self,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let page = self
            .source
            .read_process_journal_log_after(
                after.map(|cursor| ProcessJournalCursor(cursor.0)),
                limit,
            )
            .await?;
        turn_event_page_from_process_journal(page)
    }
}

pub(super) fn turn_scope_from_process_scope(scope: ResourceScope) -> Result<TurnScope, TurnError> {
    let Some(thread_id) = scope.thread_id else {
        return Err(TurnError::InvalidRequest {
            reason: "process scope filter for agent turns requires thread_id".to_string(),
        });
    };
    if scope.user_id.as_str() == SYSTEM_RESERVED_ID {
        Ok(TurnScope::new(
            scope.tenant_id,
            scope.agent_id,
            scope.project_id,
            thread_id,
        ))
    } else {
        Ok(TurnScope::new_with_owner(
            scope.tenant_id,
            scope.agent_id,
            scope.project_id,
            thread_id,
            Some(scope.user_id),
        ))
    }
}

pub fn turn_event_page_from_process_journal(
    page: ProcessJournalPage,
) -> Result<TurnEventPage, TurnError> {
    Ok(TurnEventPage {
        entries: page
            .entries
            .into_iter()
            .filter(|entry| entry.process_kind == ProcessKind::AgentTurn)
            .map(turn_lifecycle_event_from_process_journal_entry)
            .collect::<Result<Vec<_>, _>>()?,
        next_cursor: EventCursor(page.next_cursor.0),
        truncated: page.truncated,
        rebase_required: page.rebase_required.map(|cursor| EventCursor(cursor.0)),
    })
}

pub fn turn_lifecycle_event_from_process_journal_entry(
    entry: ProcessJournalEntry,
) -> Result<TurnLifecycleEvent, TurnError> {
    if entry.process_kind != ProcessKind::AgentTurn {
        return Err(TurnError::InvalidRequest {
            reason: "process journal entry is not an agent turn".to_string(),
        });
    }
    let status = turn_status_from_process_status(entry.status, entry.suspension.as_ref())?;
    let kind = turn_event_kind_from_process_journal_kind(entry.kind);
    let scope = turn_scope_from_process_scope(entry.scope)?;
    let blocked_gate = if kind == TurnEventKind::Blocked {
        entry
            .suspension
            .map(turn_blocked_gate_metadata_from_process_suspension)
            .transpose()?
    } else {
        None
    };
    Ok(TurnLifecycleEvent {
        cursor: EventCursor(entry.cursor.0),
        scope,
        occurred_at: entry.occurred_at,
        owner_user_id: entry.owner_user_id,
        run_id: TurnRunId::from_uuid(entry.process_id.as_uuid()),
        status,
        kind,
        blocked_gate,
        sanitized_reason: entry.sanitized_reason,
        retryable: entry.retryable,
        detail: entry.detail,
    })
}

pub fn turn_status_from_process_status(
    status: ProcessLifecycleStatus,
    suspension: Option<&ProcessSuspension>,
) -> Result<TurnStatus, TurnError> {
    Ok(match status {
        ProcessLifecycleStatus::Queued => TurnStatus::Queued,
        ProcessLifecycleStatus::Running => TurnStatus::Running,
        ProcessLifecycleStatus::Suspended => {
            let Some(suspension) = suspension else {
                return Err(TurnError::InvalidRequest {
                    reason: "suspended agent-turn process requires suspension metadata".to_string(),
                });
            };
            turn_status_from_process_suspension_kind(suspension.kind)
        }
        ProcessLifecycleStatus::CancelRequested | ProcessLifecycleStatus::StopRequested => {
            TurnStatus::CancelRequested
        }
        ProcessLifecycleStatus::Stopped | ProcessLifecycleStatus::Completed => {
            TurnStatus::Completed
        }
        ProcessLifecycleStatus::Cancelled | ProcessLifecycleStatus::Killed => TurnStatus::Cancelled,
        ProcessLifecycleStatus::Failed => TurnStatus::Failed,
        ProcessLifecycleStatus::RecoveryRequired => TurnStatus::RecoveryRequired,
    })
}

fn turn_status_from_process_suspension_kind(kind: ProcessSuspensionKind) -> TurnStatus {
    match kind {
        ProcessSuspensionKind::Approval => TurnStatus::BlockedApproval,
        ProcessSuspensionKind::Authorization => TurnStatus::BlockedAuth,
        ProcessSuspensionKind::Resource => TurnStatus::BlockedResource,
        ProcessSuspensionKind::AwaitingChildProcess => TurnStatus::BlockedDependentRun,
        ProcessSuspensionKind::ExternalTool
        | ProcessSuspensionKind::ExternalProcess
        | ProcessSuspensionKind::ExtensionDefined => TurnStatus::BlockedExternalTool,
    }
}

fn turn_event_kind_from_process_journal_kind(kind: ProcessJournalKind) -> TurnEventKind {
    match kind {
        ProcessJournalKind::Submitted | ProcessJournalKind::Spawned => TurnEventKind::Submitted,
        ProcessJournalKind::Resumed => TurnEventKind::Resumed,
        ProcessJournalKind::Claimed => TurnEventKind::RunnerClaimed,
        ProcessJournalKind::Heartbeat => TurnEventKind::RunnerHeartbeat,
        ProcessJournalKind::RecoveryRequired => TurnEventKind::RecoveryRequired,
        ProcessJournalKind::Suspended => TurnEventKind::Blocked,
        ProcessJournalKind::CancelRequested | ProcessJournalKind::StopRequested => {
            TurnEventKind::CancelRequested
        }
        ProcessJournalKind::Cancelled
        | ProcessJournalKind::Stopped
        | ProcessJournalKind::Killed => TurnEventKind::Cancelled,
        ProcessJournalKind::Completed => TurnEventKind::Completed,
        ProcessJournalKind::Failed => TurnEventKind::Failed,
    }
}

fn turn_blocked_gate_metadata_from_process_suspension(
    suspension: ProcessSuspension,
) -> Result<TurnBlockedGateMetadata, TurnError> {
    let Some(gate_ref) = suspension.gate_ref else {
        return Err(TurnError::InvalidRequest {
            reason: "blocked agent-turn process requires gate_ref".to_string(),
        });
    };
    Ok(TurnBlockedGateMetadata {
        gate_ref,
        gate_kind: turn_blocked_gate_kind_from_process_suspension_kind(suspension.kind),
        activity_id: suspension.activity_id,
        credential_requirements: suspension.credential_requirements,
    })
}

fn turn_blocked_gate_kind_from_process_suspension_kind(
    kind: ProcessSuspensionKind,
) -> TurnBlockedGateKind {
    match kind {
        ProcessSuspensionKind::Approval => TurnBlockedGateKind::Approval,
        ProcessSuspensionKind::Authorization => TurnBlockedGateKind::Auth,
        ProcessSuspensionKind::Resource => TurnBlockedGateKind::Resource,
        ProcessSuspensionKind::AwaitingChildProcess => TurnBlockedGateKind::AwaitDependentRun,
        ProcessSuspensionKind::ExternalTool
        | ProcessSuspensionKind::ExternalProcess
        | ProcessSuspensionKind::ExtensionDefined => TurnBlockedGateKind::ExternalTool,
    }
}
