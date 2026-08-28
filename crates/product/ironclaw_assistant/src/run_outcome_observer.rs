use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    Timestamp,
    execution_policy::{ResultDeliveryPolicy, TurnExecutionPolicy},
    output::OutputContract,
    turn::{TurnExecutionOutcome, TurnOriginKind, TurnRunId},
};
use ironclaw_notifications::{
    LifecycleRef, ListNotificationsRequest, NOTIFICATION_PAGE_LIMIT_MAX, NotificationAction,
    NotificationId, NotificationInboxError, NotificationInboxStorePort, NotificationInitialState,
    NotificationKind, NotificationMutationRequest, NotificationRecipient, NotificationSeverity,
    NotificationSource, PublishNotificationRequest,
};
use ironclaw_processes::{
    GetProcessSnapshotRequest, JournaledProcessSnapshot, ProcessJournalCommit,
    ProcessJournalCommitObserver, ProcessJournalKind, ProcessJournalSource, ProcessKind,
    ProcessLifecycleStatus, ProcessSuspensionKind,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::GateResumeDisposition;
use serde::Deserialize;

/// Materializes durable background-run outcomes from the authoritative
/// process journal. It does not observe delivery watchers or poll run state.
pub struct RunOutcomeProcessCommitObserver {
    inbox: Arc<dyn NotificationInboxStorePort>,
    thread_service: Arc<dyn SessionThreadService>,
    process_journal_source:
        Option<Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>>,
}

impl RunOutcomeProcessCommitObserver {
    pub fn new(
        inbox: Arc<dyn NotificationInboxStorePort>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            inbox,
            thread_service,
            process_journal_source: None,
        }
    }

    pub fn with_process_journal_source(
        mut self,
        process_journal_source: Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
    ) -> Self {
        self.process_journal_source = Some(process_journal_source);
        self
    }

    async fn publish(
        &self,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        kind: NotificationKind,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let thread_id = snapshot
            .scope
            .thread_id
            .clone()
            .ok_or_else(|| "eligible run outcome has no thread id".to_string())?;
        let owner_user_id = snapshot
            .owner_user_id
            .clone()
            .ok_or_else(|| "eligible run outcome has no owner".to_string())?;
        let severity = match kind {
            NotificationKind::RunCompleted => NotificationSeverity::Success,
            NotificationKind::RunFailed | NotificationKind::DeliveryFailed => {
                NotificationSeverity::Error
            }
            NotificationKind::ApprovalRequired
            | NotificationKind::AuthenticationRequired
            | NotificationKind::RunBlocked => NotificationSeverity::Warning,
        };
        let notification_id = outcome_notification_id(run_id, kind)?;
        self.inbox
            .publish(PublishNotificationRequest {
                id: notification_id,
                recipient: NotificationRecipient {
                    tenant_id: snapshot.scope.tenant_id.clone(),
                    user_id: owner_user_id,
                },
                kind,
                severity,
                source: NotificationSource {
                    thread_id: thread_id.clone(),
                    turn_run_id: Some(run_id),
                    lifecycle_ref: Some(outcome_lifecycle_ref("process-terminal")?),
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::OpenThread { thread_id },
                initial_state: NotificationInitialState::Resolved,
                occurred_at,
            })
            .await
            .map_err(|error| format!("publish run outcome notification failed: {error}"))?;
        Ok(())
    }

    async fn publish_authentication_required(
        inbox: &dyn NotificationInboxStorePort,
        process_journal_source: Option<
            &dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>,
        >,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let Some(suspension) = snapshot.suspension.as_ref() else {
            return Ok(());
        };
        if suspension.kind != ProcessSuspensionKind::Authorization {
            return Ok(());
        }
        let Some(gate_ref) = suspension.gate_ref.as_ref() else {
            // A gate-less auth suspension cannot be resumed from the product
            // surface, so publishing an actionable notification would strand
            // an item the user cannot complete.
            return Ok(());
        };
        if !Self::auth_gate_is_current(process_journal_source, snapshot, gate_ref).await? {
            return Self::resolve_authentication_required(
                inbox,
                snapshot,
                run_id,
                gate_ref,
                occurred_at,
            )
            .await;
        }
        let thread_id = snapshot
            .scope
            .thread_id
            .clone()
            .ok_or_else(|| "eligible auth suspension has no thread id".to_string())?;
        let owner_user_id = snapshot
            .owner_user_id
            .clone()
            .ok_or_else(|| "eligible auth suspension has no owner".to_string())?;
        let lifecycle_ref = LifecycleRef::new(gate_ref.as_str())
            .map_err(|error| format!("build auth lifecycle reference failed: {error}"))?;
        let notification_id = crate::run_delivery::run_notification_inbox_id(
            run_id,
            NotificationKind::AuthenticationRequired,
            Some(gate_ref.as_str()),
        )
        .map_err(|error| format!("build auth notification id failed: {error}"))?;
        let notification_id_for_reopen = notification_id.clone();
        let recipient = NotificationRecipient {
            tenant_id: snapshot.scope.tenant_id.clone(),
            user_id: owner_user_id,
        };
        inbox
            .publish(PublishNotificationRequest {
                id: notification_id,
                recipient: recipient.clone(),
                kind: NotificationKind::AuthenticationRequired,
                severity: NotificationSeverity::Warning,
                source: NotificationSource {
                    thread_id: thread_id.clone(),
                    turn_run_id: Some(run_id),
                    lifecycle_ref: Some(lifecycle_ref),
                    credential_providers: suspension
                        .credential_requirements
                        .iter()
                        .map(|requirement| requirement.provider.clone())
                        .collect(),
                },
                action: NotificationAction::OpenThread { thread_id },
                initial_state: NotificationInitialState::Open,
                occurred_at,
            })
            .await
            .map_err(|error| format!("publish auth notification failed: {error}"))?;
        // A verified resume may be followed by the same deterministic gate
        // becoming current again. Publication must remain a no-op for ordinary
        // retries, so the authoritative Suspended transition explicitly
        // re-activates the settled lifecycle record.
        inbox
            .reopen(NotificationMutationRequest {
                recipient,
                notification_id: notification_id_for_reopen,
                occurred_at,
            })
            .await
            .map_err(|error| format!("reopen auth notification failed: {error}"))?;
        // The continuation can settle the gate between the pre-publication
        // state read and the Inbox CAS. Re-read after the write and retire the
        // just-published record if recovery won that race.
        if !Self::auth_gate_is_current(process_journal_source, snapshot, gate_ref).await? {
            Self::resolve_authentication_required(inbox, snapshot, run_id, gate_ref, occurred_at)
                .await?;
        }
        Ok(())
    }

    async fn auth_gate_is_current(
        process_journal_source: Option<
            &dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>,
        >,
        snapshot: &JournaledProcessSnapshot,
        gate_ref: &ironclaw_host_api::turn::TurnGateRef,
    ) -> Result<bool, String> {
        let Some(source) = process_journal_source else {
            return Ok(true);
        };
        Ok(Self::current_auth_gate_ref(source, snapshot)
            .await?
            .as_ref()
            == Some(gate_ref))
    }

    async fn current_auth_gate_ref(
        source: &dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>,
        snapshot: &JournaledProcessSnapshot,
    ) -> Result<Option<ironclaw_host_api::turn::TurnGateRef>, String> {
        let current = source
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: snapshot.scope.clone(),
                process_id: snapshot.process_id,
            })
            .await
            .map_err(|error| format!("read current auth process state failed: {error}"))?;
        if current.status != ProcessLifecycleStatus::Suspended {
            return Ok(None);
        }
        let Some(suspension) = current.suspension else {
            return Ok(None);
        };
        if suspension.kind != ProcessSuspensionKind::Authorization {
            return Ok(None);
        }
        Ok(suspension.gate_ref)
    }

    async fn resolve_authentication_required(
        inbox: &dyn NotificationInboxStorePort,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        gate_ref: &ironclaw_host_api::turn::TurnGateRef,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let Some(owner_user_id) = snapshot.owner_user_id.clone() else {
            return Ok(());
        };
        let notification_id = crate::run_delivery::run_notification_inbox_id(
            run_id,
            NotificationKind::AuthenticationRequired,
            Some(gate_ref.as_str()),
        )
        .map_err(|error| format!("build auth notification id failed: {error}"))?;
        match inbox
            .resolve(NotificationMutationRequest {
                recipient: NotificationRecipient {
                    tenant_id: snapshot.scope.tenant_id.clone(),
                    user_id: owner_user_id,
                },
                notification_id,
                occurred_at,
            })
            .await
        {
            Ok(_) | Err(NotificationInboxError::NotificationNotFound) => Ok(()),
            Err(error) => Err(format!("resolve auth notification failed: {error}")),
        }
    }

    async fn resolve_run_authentication_required(
        &self,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        // A Resumed process snapshot no longer carries the suspension's gate
        // reference. Reconcile by the notification's stable run identity;
        // the recipient Inbox is bounded, and paging preserves older or
        // archived records that may have survived a transient write outage.
        let Some(owner_user_id) = snapshot.owner_user_id.clone() else {
            return Ok(());
        };
        let recipient = NotificationRecipient {
            tenant_id: snapshot.scope.tenant_id.clone(),
            user_id: owner_user_id,
        };
        // A later auth gate can already be current by the time this older
        // Resumed commit reaches the observer. Preserve that gate's stable
        // record: actionable retries intentionally do not reopen records that
        // were resolved by an earlier lifecycle transition.
        let current_auth_gate_ref = match self.process_journal_source.as_deref() {
            Some(source) => Self::current_auth_gate_ref(source, snapshot).await?,
            None => None,
        };
        let mut cursor = None;
        loop {
            let page = self
                .inbox
                .list(ListNotificationsRequest {
                    recipient: recipient.clone(),
                    limit: NOTIFICATION_PAGE_LIMIT_MAX,
                    cursor: cursor.clone(),
                    include_archived: true,
                })
                .await
                .map_err(|error| {
                    format!("list auth notifications for resumed run failed: {error}")
                })?;
            for notification in page.notifications {
                let is_current_auth_gate = current_auth_gate_ref.as_ref().is_some_and(|gate_ref| {
                    notification
                        .source
                        .lifecycle_ref
                        .as_ref()
                        .is_some_and(|lifecycle_ref| lifecycle_ref.as_str() == gate_ref.as_str())
                });
                if notification.kind == NotificationKind::AuthenticationRequired
                    && notification.source.turn_run_id == Some(run_id)
                    && notification.resolved_at.is_none()
                    && !is_current_auth_gate
                {
                    self.inbox
                        .resolve(NotificationMutationRequest {
                            recipient: recipient.clone(),
                            notification_id: notification.id,
                            occurred_at,
                        })
                        .await
                        .map_err(|error| {
                            format!("resolve auth notification for resumed run failed: {error}")
                        })?;
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                return Ok(());
            };
            if cursor.as_ref() == Some(&next_cursor) {
                return Err("auth notification pagination cursor did not advance".to_string());
            }
            cursor = Some(next_cursor);
        }
    }

    async fn reconcile_resource_block(
        &self,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let owner_user_id = snapshot
            .owner_user_id
            .clone()
            .ok_or_else(|| "eligible resource-blocked run has no owner".to_string())?;
        let recipient = NotificationRecipient {
            tenant_id: snapshot.scope.tenant_id.clone(),
            user_id: owner_user_id,
        };
        let active_lifecycle = match (snapshot.status, snapshot.suspension.as_ref()) {
            (ProcessLifecycleStatus::Suspended, Some(suspension))
                if suspension.kind == ProcessSuspensionKind::Resource =>
            {
                let gate_ref = suspension
                    .gate_ref
                    .as_ref()
                    .ok_or_else(|| "resource-blocked run has no gate reference".to_string())?;
                Some(resource_block_lifecycle_ref(gate_ref.as_str())?)
            }
            _ => None,
        };

        self.resolve_stale_resource_blocks(
            &recipient,
            run_id,
            active_lifecycle.as_ref(),
            occurred_at,
        )
        .await?;

        let Some(lifecycle_ref) = active_lifecycle else {
            return Ok(());
        };
        let thread_id = snapshot
            .scope
            .thread_id
            .clone()
            .ok_or_else(|| "eligible resource-blocked run has no thread id".to_string())?;
        let notification_id = crate::run_delivery::run_notification_inbox_id(
            run_id,
            NotificationKind::RunBlocked,
            Some(lifecycle_ref.as_str()),
        )
        .map_err(|error| format!("build resource-block notification id failed: {error}"))?;
        self.inbox
            .publish(PublishNotificationRequest {
                id: notification_id.clone(),
                recipient: recipient.clone(),
                kind: NotificationKind::RunBlocked,
                severity: NotificationSeverity::Warning,
                source: NotificationSource {
                    thread_id: thread_id.clone(),
                    turn_run_id: Some(run_id),
                    lifecycle_ref: Some(lifecycle_ref),
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::OpenThread { thread_id },
                initial_state: NotificationInitialState::Open,
                occurred_at,
            })
            .await
            .map_err(|error| format!("publish resource-block notification failed: {error}"))?;
        self.inbox
            .reopen(NotificationMutationRequest {
                recipient,
                notification_id,
                occurred_at,
            })
            .await
            .map_err(|error| format!("reopen resource-block notification failed: {error}"))?;
        Ok(())
    }

    async fn resolve_stale_resource_blocks(
        &self,
        recipient: &NotificationRecipient,
        run_id: TurnRunId,
        active_lifecycle: Option<&LifecycleRef>,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let mut cursor = None;
        loop {
            let page = self
                .inbox
                .list(ListNotificationsRequest {
                    recipient: recipient.clone(),
                    limit: NOTIFICATION_PAGE_LIMIT_MAX,
                    cursor,
                    include_archived: true,
                })
                .await
                .map_err(|error| format!("list resource-block notifications failed: {error}"))?;
            for notification in &page.notifications {
                let Some(lifecycle_ref) = notification.source.lifecycle_ref.as_ref() else {
                    continue;
                };
                if notification.kind != NotificationKind::RunBlocked
                    || notification.source.turn_run_id != Some(run_id)
                    || notification.resolved_at.is_some()
                    || !is_resource_block_lifecycle_ref(lifecycle_ref)
                    || active_lifecycle == Some(lifecycle_ref)
                {
                    continue;
                }
                self.inbox
                    .resolve(NotificationMutationRequest {
                        recipient: recipient.clone(),
                        notification_id: notification.id.clone(),
                        occurred_at,
                    })
                    .await
                    .map_err(|error| {
                        format!("resolve resource-block notification failed: {error}")
                    })?;
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Ok(())
    }
}

impl RunOutcomeProcessCommitObserver {
    /// Retire the actionable block a delivery timeout left behind. That
    /// publisher returns straight after recording it, so no watcher observes
    /// the run again — a terminal fact is the only thing that can close it.
    /// A run that never timed out simply has no such record.
    async fn resolve_timed_out_block(
        &self,
        snapshot: &JournaledProcessSnapshot,
        run_id: TurnRunId,
        occurred_at: Timestamp,
    ) -> Result<(), String> {
        let Some(owner_user_id) = snapshot.owner_user_id.clone() else {
            return Ok(());
        };
        let notification_id = crate::run_delivery::run_notification_inbox_id(
            run_id,
            NotificationKind::RunBlocked,
            Some(crate::run_delivery::TIMEOUT_LIFECYCLE_REF),
        )
        .map_err(|error| format!("build timed-out run block id failed: {error}"))?;
        match self
            .inbox
            .resolve(NotificationMutationRequest {
                recipient: NotificationRecipient {
                    tenant_id: snapshot.scope.tenant_id.clone(),
                    user_id: owner_user_id,
                },
                notification_id,
                occurred_at,
            })
            .await
        {
            // Most runs never time out, so there is usually nothing to retire;
            // the outcome itself is not interesting here, only that the store
            // was reached.
            Ok(_) | Err(NotificationInboxError::NotificationNotFound) => Ok(()),
            Err(error) => Err(format!("resolve timed-out run block failed: {error}")),
        }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for RunOutcomeProcessCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        // Keep the original cursor: changing this observer's identity would
        // replay every historical terminal outcome, not just auth or resource gates.
        "run-outcome-inbox-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        let Some(metadata) = eligible_user_run(&commit.state) else {
            return Ok(());
        };
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        let occurred_at = commit.occurred_at.unwrap_or(commit.state.created_at);
        if commit.kind == ProcessJournalKind::Resumed
            && metadata.resume_disposition != Some(GateResumeDisposition::Denied)
        {
            self.resolve_run_authentication_required(&commit.state, run_id, occurred_at)
                .await?;
        }
        if commit.kind == ProcessJournalKind::Suspended
            && commit.state.status == ProcessLifecycleStatus::Suspended
        {
            Self::publish_authentication_required(
                self.inbox.as_ref(),
                self.process_journal_source.as_deref(),
                &commit.state,
                run_id,
                occurred_at,
            )
            .await?;
        }
        if resource_block_reconciliation_needed(&commit) {
            self.reconcile_resource_block(&commit.state, run_id, occurred_at)
                .await?;
        }
        if !is_background_run(&metadata) {
            return Ok(());
        }
        if commit.state.status.is_terminal() {
            self.resolve_timed_out_block(&commit.state, run_id, occurred_at)
                .await?;
        }
        match (commit.kind, commit.state.status) {
            (ProcessJournalKind::Completed, ProcessLifecycleStatus::Completed) => {
                if !metadata.output_contract.is_assistant_message() {
                    return Ok(());
                }
                if metadata.execution_outcome == Some(TurnExecutionOutcome::NothingToReport)
                    && metadata
                        .product_context
                        .as_ref()
                        .and_then(|context| context.execution_policy.as_ref())
                        .is_some_and(|policy| {
                            policy.result_delivery
                                == ResultDeliveryPolicy::SuppressWhenNothingToReport
                        })
                {
                    return Ok(());
                }
                let Some(thread_scope) = thread_scope_for_snapshot(&commit.state) else {
                    return Ok(());
                };
                let thread_id = commit
                    .state
                    .scope
                    .thread_id
                    .clone()
                    .ok_or_else(|| "eligible completed run has no thread id".to_string())?;
                let final_reply = self
                    .thread_service
                    .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                        scope: thread_scope,
                        thread_id,
                        turn_run_id: run_id.to_string(),
                    })
                    .await
                    .map_err(|error| format!("read finalized run reply failed: {error}"))?;
                let Some(_final_reply) = final_reply else {
                    // The loop host appends the finalized reply, with retry,
                    // before the turn can report completion, so an absent one
                    // here is a contract miss rather than a routine race —
                    // returning an error retains the durable observer cursor
                    // so replay can try again after the reply is persisted.
                    return Err(format!(
                        "completed background run {run_id} has no finalized assistant reply"
                    ));
                };
                self.publish(
                    &commit.state,
                    run_id,
                    NotificationKind::RunCompleted,
                    occurred_at,
                )
                .await?;
            }
            (ProcessJournalKind::Failed, ProcessLifecycleStatus::Failed)
            | (ProcessJournalKind::RecoveryRequired, ProcessLifecycleStatus::RecoveryRequired) => {
                self.publish(
                    &commit.state,
                    run_id,
                    NotificationKind::RunFailed,
                    occurred_at,
                )
                .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// One-time durable replay for auth suspensions introduced after the primary
/// outcome observer had already advanced past them. Its separate cursor is
/// intentionally narrow: terminal outcomes stay owned by the original v1
/// observer and cannot consume Inbox capacity during an upgrade.
pub struct AuthNotificationBackfillProcessCommitObserver {
    inbox: Arc<dyn NotificationInboxStorePort>,
    process_journal_source: Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
}

impl AuthNotificationBackfillProcessCommitObserver {
    pub fn new(
        inbox: Arc<dyn NotificationInboxStorePort>,
        process_journal_source: Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
    ) -> Self {
        Self {
            inbox,
            process_journal_source,
        }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for AuthNotificationBackfillProcessCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "run-auth-inbox-backfill-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if commit.kind != ProcessJournalKind::Suspended
            || commit.state.status != ProcessLifecycleStatus::Suspended
            || eligible_user_run(&commit.state).is_none()
        {
            return Ok(());
        }
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        let occurred_at = commit.occurred_at.unwrap_or(commit.state.created_at);
        RunOutcomeProcessCommitObserver::publish_authentication_required(
            self.inbox.as_ref(),
            Some(self.process_journal_source.as_ref()),
            &commit.state,
            run_id,
            occurred_at,
        )
        .await
    }
}

/// One-time durable replay for resource suspensions introduced after the
/// primary outcome observer had already advanced past them. Its separate
/// cursor is intentionally narrow: terminal outcomes remain owned by the v1
/// observer and cannot consume Inbox capacity during an upgrade.
pub struct ResourceBlockBackfillProcessCommitObserver {
    outcome_observer: RunOutcomeProcessCommitObserver,
    process_journal_source: Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
}

impl ResourceBlockBackfillProcessCommitObserver {
    pub fn new(
        inbox: Arc<dyn NotificationInboxStorePort>,
        thread_service: Arc<dyn SessionThreadService>,
        process_journal_source: Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
    ) -> Self {
        Self {
            outcome_observer: RunOutcomeProcessCommitObserver::new(inbox, thread_service),
            process_journal_source,
        }
    }

    async fn current_snapshot(
        &self,
        snapshot: &JournaledProcessSnapshot,
    ) -> Result<JournaledProcessSnapshot, String> {
        self.process_journal_source
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: snapshot.scope.clone(),
                process_id: snapshot.process_id,
            })
            .await
            .map_err(|error| format!("read current resource-block process state failed: {error}"))
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for ResourceBlockBackfillProcessCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "run-resource-block-inbox-backfill-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if commit.kind != ProcessJournalKind::Suspended
            || commit.state.status != ProcessLifecycleStatus::Suspended
            || eligible_user_run(&commit.state).is_none()
        {
            return Ok(());
        }
        let Some(historical_gate_ref) = commit.state.suspension.as_ref().and_then(|suspension| {
            (suspension.kind == ProcessSuspensionKind::Resource)
                .then_some(suspension.gate_ref.as_ref())
                .flatten()
        }) else {
            return Ok(());
        };
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        let mut current = self.current_snapshot(&commit.state).await?;
        let mut reconciliation_at =
            if current_resource_gate_ref(&current) == Some(historical_gate_ref.as_str()) {
                commit.occurred_at.unwrap_or(commit.state.created_at)
            } else {
                // A previous attempt may have published this historical gate and
                // then crashed while compensating after a post-write state read.
                // Reconcile even when replay starts stale so the durable cursor
                // cannot acknowledge the commit while leaving that record open.
                chrono::Utc::now()
            };

        loop {
            self.outcome_observer
                .reconcile_resource_block(&current, run_id, reconciliation_at)
                .await?;

            // Recovery or another replacement gate can win between every
            // pre-publication state read and Inbox CAS. Do not acknowledge
            // the historical commit until a post-write read still names the
            // state just reconciled.
            let latest = self.current_snapshot(&commit.state).await?;
            if current_resource_gate_ref(&latest) == current_resource_gate_ref(&current) {
                break;
            }
            current = latest;
            reconciliation_at = chrono::Utc::now();
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OutcomeMetadataEnvelope {
    agent_turn: OutcomeMetadata,
}

#[derive(Debug, Deserialize)]
struct OutcomeMetadata {
    #[serde(default)]
    output_contract: OutputContract,
    #[serde(default)]
    execution_outcome: Option<TurnExecutionOutcome>,
    #[serde(default)]
    subagent_depth: u32,
    #[serde(default)]
    ownerless_thread: bool,
    #[serde(rename = "auth_resume_disposition", default)]
    resume_disposition: Option<GateResumeDisposition>,
    product_context: Option<OutcomeProductContext>,
}

#[derive(Debug, Deserialize)]
struct OutcomeProductContext {
    origin: TurnOriginKind,
    #[serde(default)]
    execution_policy: Option<TurnExecutionPolicy>,
}

fn eligible_user_run(snapshot: &JournaledProcessSnapshot) -> Option<OutcomeMetadata> {
    if snapshot.process_kind != ProcessKind::AgentTurn
        || snapshot.parent_process_id.is_some()
        || snapshot.owner_user_id.is_none()
        || snapshot.scope.thread_id.is_none()
        || snapshot.scope.agent_id.is_none()
    {
        return None;
    }
    // silent-ok: eligibility screening. Journal metadata this observer cannot
    // read describes a process it does not own, so the absence of a notification
    // is the correct outcome rather than a failure to report; the cause is
    // recorded above before it is dropped.
    let envelope = serde_json::from_value::<OutcomeMetadataEnvelope>(snapshot.metadata.clone())
        .map_err(|error| {
            tracing::debug!(
                process_id = %snapshot.process_id,
                %error,
                "run outcome observer ignored malformed agent-turn metadata"
            );
        })
        .ok()?;
    let metadata = envelope.agent_turn;
    if metadata.subagent_depth != 0 || metadata.ownerless_thread {
        return None;
    }
    Some(metadata)
}

fn is_background_run(metadata: &OutcomeMetadata) -> bool {
    metadata
        .product_context
        .as_ref()
        .is_some_and(|context| context.origin == TurnOriginKind::ScheduledTrigger)
}

fn resource_block_reconciliation_needed(commit: &ProcessJournalCommit) -> bool {
    matches!(
        (commit.state.status, commit.state.suspension.as_ref()),
        (ProcessLifecycleStatus::Suspended, Some(suspension))
            if suspension.kind == ProcessSuspensionKind::Resource
    ) || matches!(
        commit.kind,
        ProcessJournalKind::Resumed
            | ProcessJournalKind::RecoveryRequired
            | ProcessJournalKind::Suspended
            | ProcessJournalKind::Stopped
            | ProcessJournalKind::Cancelled
            | ProcessJournalKind::Completed
            | ProcessJournalKind::Failed
            | ProcessJournalKind::Killed
    )
}

fn current_resource_gate_ref(snapshot: &JournaledProcessSnapshot) -> Option<&str> {
    match (snapshot.status, snapshot.suspension.as_ref()) {
        (ProcessLifecycleStatus::Suspended, Some(suspension))
            if suspension.kind == ProcessSuspensionKind::Resource =>
        {
            suspension.gate_ref.as_ref().map(|gate| gate.as_str())
        }
        _ => None,
    }
}

fn resource_block_lifecycle_ref(gate_ref: &str) -> Result<LifecycleRef, String> {
    LifecycleRef::new(gate_ref)
        .map_err(|error| format!("build resource-block lifecycle reference failed: {error}"))
}

fn is_resource_block_lifecycle_ref(lifecycle_ref: &LifecycleRef) -> bool {
    lifecycle_ref.as_str() != crate::run_delivery::TIMEOUT_LIFECYCLE_REF
}

fn thread_scope_for_snapshot(snapshot: &JournaledProcessSnapshot) -> Option<ThreadScope> {
    Some(ThreadScope {
        tenant_id: snapshot.scope.tenant_id.clone(),
        agent_id: snapshot.scope.agent_id.clone()?,
        project_id: snapshot.scope.project_id.clone(),
        owner_user_id: snapshot.owner_user_id.clone(),
        mission_id: snapshot.scope.mission_id.clone(),
    })
}

fn outcome_lifecycle_ref(value: &'static str) -> Result<LifecycleRef, String> {
    LifecycleRef::new(value)
        .map_err(|error| format!("build run outcome lifecycle reference failed: {error}"))
}

fn outcome_notification_id(
    run_id: TurnRunId,
    kind: NotificationKind,
) -> Result<NotificationId, String> {
    let kind = kind.stable_key();
    NotificationId::new(format!("run:{run_id}:{kind}"))
        .map_err(|error| format!("build run outcome notification id failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use chrono::{TimeDelta, Utc};
    use ironclaw_filesystem::{
        Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, ScopedFilesystem,
    };
    use ironclaw_host_api::{
        ids::{AgentId, ProcessId, TenantId, ThreadId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
        turn::{TurnGateRef, TurnRunId},
    };
    use ironclaw_notifications::{
        LifecycleRef, ListNotificationsRequest, MarkAllNotificationsReadRequest,
        NotificationAction, NotificationInboxError, NotificationInboxStore,
        NotificationInboxStorePort, NotificationInitialState, NotificationKind,
        NotificationMutationOutcome, NotificationMutationRequest, NotificationPage,
        NotificationRecipient, NotificationRecord, NotificationSeverity, NotificationSource,
        PublishNotificationRequest,
    };
    use ironclaw_processes::{
        ClaimProcessesRequest, GetProcessSnapshotRequest, JournaledProcessSnapshot,
        ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalCursor,
        ProcessJournalKind, ProcessJournalObserverRegistry, ProcessJournalPage,
        ProcessJournalSource, ProcessJournalStore, ProcessKind, ProcessLeaseRequest,
        ProcessLifecycleStatus, ProcessStateTransitionRequest, ProcessSubmissionPort,
        ProcessSuspension, ProcessSuspensionKind, ProcessTransitionPort, ProcessWorkerId,
        SubmitProcessRequest,
    };
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent, SessionThreadService, ThreadScope,
    };
    use serde_json::json;

    use super::{
        AuthNotificationBackfillProcessCommitObserver, ResourceBlockBackfillProcessCommitObserver,
        RunOutcomeProcessCommitObserver,
    };

    struct CurrentProcessSource {
        snapshot: Mutex<JournaledProcessSnapshot>,
    }

    impl CurrentProcessSource {
        fn new(snapshot: JournaledProcessSnapshot) -> Self {
            Self {
                snapshot: Mutex::new(snapshot),
            }
        }

        fn replace(&self, snapshot: JournaledProcessSnapshot) {
            *self.snapshot.lock().expect("current process lock") = snapshot;
        }
    }

    #[async_trait::async_trait]
    impl ProcessJournalSource for CurrentProcessSource {
        type Error = ironclaw_turns::TurnError;

        async fn get_process_snapshot(
            &self,
            _request: GetProcessSnapshotRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(self.snapshot.lock().expect("current process lock").clone())
        }

        async fn read_process_journal_after(
            &self,
            _scope: &ResourceScope,
            _owner_user_id: Option<&UserId>,
            _after: Option<ProcessJournalCursor>,
            _limit: usize,
        ) -> Result<ProcessJournalPage, Self::Error> {
            panic!("journal paging is not used by the current-state test")
        }

        async fn read_process_journal_log_after(
            &self,
            _after: Option<ProcessJournalCursor>,
            _limit: usize,
        ) -> Result<ProcessJournalPage, Self::Error> {
            panic!("global journal paging is not used by the current-state test")
        }
    }

    struct CurrentProcessJournalSource {
        current: Mutex<VecDeque<JournaledProcessSnapshot>>,
    }

    impl CurrentProcessJournalSource {
        fn new(current: JournaledProcessSnapshot) -> Self {
            Self {
                current: Mutex::new(VecDeque::from([current])),
            }
        }

        fn set_current(&self, current: JournaledProcessSnapshot) {
            *self.current.lock().expect("current process lock") = VecDeque::from([current]);
        }

        fn set_sequence(&self, current: impl IntoIterator<Item = JournaledProcessSnapshot>) {
            let current = current.into_iter().collect::<VecDeque<_>>();
            assert!(!current.is_empty(), "current process sequence is non-empty");
            *self.current.lock().expect("current process lock") = current;
        }
    }

    #[async_trait::async_trait]
    impl ProcessJournalSource for CurrentProcessJournalSource {
        type Error = ironclaw_turns::TurnError;

        async fn get_process_snapshot(
            &self,
            _request: GetProcessSnapshotRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            let mut current = self.current.lock().expect("current process lock");
            let snapshot = current.front().expect("current process snapshot").clone();
            if current.len() > 1 {
                current.pop_front();
            }
            Ok(snapshot)
        }

        async fn read_process_journal_after(
            &self,
            _scope: &ResourceScope,
            _owner_user_id: Option<&UserId>,
            _after: Option<ProcessJournalCursor>,
            _limit: usize,
        ) -> Result<ProcessJournalPage, Self::Error> {
            Ok(ProcessJournalPage {
                entries: Vec::new(),
                next_cursor: ProcessJournalCursor(0),
                truncated: false,
                rebase_required: None,
            })
        }

        async fn read_process_journal_log_after(
            &self,
            _after: Option<ProcessJournalCursor>,
            _limit: usize,
        ) -> Result<ProcessJournalPage, Self::Error> {
            Ok(ProcessJournalPage {
                entries: Vec::new(),
                next_cursor: ProcessJournalCursor(0),
                truncated: false,
                rebase_required: None,
            })
        }
    }

    struct FailFirstResolveInbox {
        inner: Arc<dyn NotificationInboxStorePort>,
        fail_next_resolve: AtomicBool,
    }

    impl FailFirstResolveInbox {
        fn new(inner: Arc<dyn NotificationInboxStorePort>) -> Self {
            Self {
                inner,
                fail_next_resolve: AtomicBool::new(true),
            }
        }
    }

    #[async_trait::async_trait]
    impl NotificationInboxStorePort for FailFirstResolveInbox {
        async fn publish(
            &self,
            request: PublishNotificationRequest,
        ) -> Result<NotificationRecord, NotificationInboxError> {
            self.inner.publish(request).await
        }

        async fn list(
            &self,
            request: ListNotificationsRequest,
        ) -> Result<NotificationPage, NotificationInboxError> {
            self.inner.list(request).await
        }

        async fn mark_read(
            &self,
            request: NotificationMutationRequest,
        ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
            self.inner.mark_read(request).await
        }

        async fn mark_all_read(
            &self,
            request: MarkAllNotificationsReadRequest,
        ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
            self.inner.mark_all_read(request).await
        }

        async fn resolve(
            &self,
            request: NotificationMutationRequest,
        ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
            if self.fail_next_resolve.swap(false, Ordering::SeqCst) {
                return Err(NotificationInboxError::Backend {
                    reason: "injected post-publish compensation failure".to_string(),
                });
            }
            self.inner.resolve(request).await
        }

        async fn reopen(
            &self,
            request: NotificationMutationRequest,
        ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
            self.inner.reopen(request).await
        }

        async fn archive(
            &self,
            request: NotificationMutationRequest,
        ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
            self.inner.archive(request).await
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("outcome-observer-tenant").expect("tenant")
    }

    fn user() -> UserId {
        UserId::new("outcome-observer-user").expect("user")
    }

    fn agent() -> AgentId {
        AgentId::new("outcome-observer-agent").expect("agent")
    }

    fn thread() -> ThreadId {
        ThreadId::new("outcome-observer-thread").expect("thread")
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: tenant(),
            agent_id: agent(),
            project_id: None,
            owner_user_id: Some(user()),
            mission_id: None,
        }
    }

    fn inbox() -> Arc<NotificationInboxStore<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("notification mount alias"),
            VirtualPath::new("/engine/test/run-outcome-notifications")
                .expect("notification mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("notification mount view");
        Arc::new(NotificationInboxStore::new(
            Arc::new(ScopedFilesystem::with_fixed_view(
                Arc::new(InMemoryBackend::new()),
                mounts,
            )),
            ironclaw_notifications::NOTIFICATION_INBOX_MAX_RECORDS,
        ))
    }

    fn inbox_with_capacity(capacity: usize) -> Arc<NotificationInboxStore<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("notification mount alias"),
            VirtualPath::new("/engine/test/run-auth-backfill-capacity")
                .expect("notification mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("notification mount view");
        Arc::new(NotificationInboxStore::new(
            Arc::new(ScopedFilesystem::with_fixed_view(
                Arc::new(InMemoryBackend::new()),
                mounts,
            )),
            capacity,
        ))
    }

    fn inbox_failing_first_write() -> Arc<NotificationInboxStore<FaultInjecting<InMemoryBackend>>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("notification mount alias"),
            VirtualPath::new("/engine/test/run-outcome-faulted-notifications")
                .expect("notification mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("notification mount view");
        let backend = Arc::new(
            FaultInjecting::new(InMemoryBackend::new()).with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .nth(1)
                    .backend("notification backend temporarily unavailable"),
            ),
        );
        Arc::new(NotificationInboxStore::new(
            Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)),
            ironclaw_notifications::NOTIFICATION_INBOX_MAX_RECORDS,
        ))
    }

    fn inbox_failing_second_write() -> Arc<NotificationInboxStore<FaultInjecting<InMemoryBackend>>>
    {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("notification mount alias"),
            VirtualPath::new("/engine/test/run-outcome-resolve-fault-notifications")
                .expect("notification mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("notification mount view");
        let backend = Arc::new(
            FaultInjecting::new(InMemoryBackend::new()).with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .nth(2)
                    .backend("notification backend temporarily unavailable"),
            ),
        );
        Arc::new(NotificationInboxStore::new(
            Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)),
            ironclaw_notifications::NOTIFICATION_INBOX_MAX_RECORDS,
        ))
    }

    fn process_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("process mount alias"),
            VirtualPath::new("/engine/test/run-outcome-processes").expect("process mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("process mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    fn commit(
        run_id: TurnRunId,
        status: ProcessLifecycleStatus,
        kind: ProcessJournalKind,
        origin: &str,
    ) -> ProcessJournalCommit {
        let now = Utc::now();
        ProcessJournalCommit {
            state: JournaledProcessSnapshot {
                process_id: ProcessId::from_uuid(run_id.as_uuid()),
                process_kind: ProcessKind::AgentTurn,
                scope: ResourceScope {
                    tenant_id: tenant(),
                    user_id: user(),
                    agent_id: Some(agent()),
                    project_id: None,
                    mission_id: None,
                    thread_id: Some(thread()),
                    invocation_id: ironclaw_host_api::ids::InvocationId::new(),
                },
                status,
                suspension: None,
                checkpoint_ref: None,
                checkpoint_kind: None,
                input_ref: None,
                failure: None,
                journal_cursor: ProcessJournalCursor(1),
                lease: None,
                crash_reclaim_count: 0,
                created_at: now,
                owner_user_id: Some(user()),
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                metadata: json!({
                    "agent_turn": {
                        "product_context": { "origin": origin },
                        "execution_outcome": "result_available"
                    }
                }),
            },
            kind,
            sanitized_reason: None,
            occurred_at: Some(now),
        }
    }

    /// A commit whose agent-turn metadata carries the exclusions the observer
    /// screens on, so a future edit to either predicate fails a test instead of
    /// quietly publishing for a child or ownerless run.
    fn excluded_commit(
        run_id: TurnRunId,
        subagent_depth: u32,
        ownerless_thread: bool,
    ) -> ProcessJournalCommit {
        let mut commit = commit(
            run_id,
            ProcessLifecycleStatus::Failed,
            ProcessJournalKind::Failed,
            "scheduled_trigger",
        );
        commit.state.metadata = json!({
            "agent_turn": {
                "product_context": { "origin": "scheduled_trigger" },
                "execution_outcome": "result_available",
                "subagent_depth": subagent_depth,
                "ownerless_thread": ownerless_thread,
            }
        });
        commit
    }

    fn resource_block_commit(
        run_id: TurnRunId,
        origin: &str,
        gate_ref: &str,
    ) -> ProcessJournalCommit {
        let mut commit = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            origin,
        );
        commit.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Resource,
            gate_ref: Some(TurnGateRef::new(gate_ref).expect("resource gate ref")),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: Some("sensitive budget policy detail".to_string()),
        });
        commit
    }

    async fn records(inbox: &dyn NotificationInboxStorePort) -> Vec<NotificationKind> {
        inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: false,
            })
            .await
            .expect("list notifications")
            .notifications
            .into_iter()
            .map(|record| record.kind)
            .collect()
    }

    #[tokio::test]
    async fn a_webui_auth_gate_publishes_one_actionable_notification() {
        let inbox = inbox();
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:webui-extension-auth").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref.clone()),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });

        observer
            .observe_process_commit(suspended.clone())
            .await
            .expect("publish auth notification");
        observer
            .observe_process_commit(suspended)
            .await
            .expect("replayed auth suspension is idempotent");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: false,
            })
            .await
            .expect("list auth notification");
        assert_eq!(page.notifications.len(), 1);
        let notification = &page.notifications[0];
        assert_eq!(notification.kind, NotificationKind::AuthenticationRequired);
        assert_eq!(notification.source.thread_id, thread());
        assert_eq!(notification.source.turn_run_id, Some(run_id));
        assert_eq!(
            notification
                .source
                .lifecycle_ref
                .as_ref()
                .map(LifecycleRef::as_str),
            Some(gate_ref.as_str())
        );
        assert!(notification.resolved_at.is_none());
        assert_eq!(
            notification.action,
            NotificationAction::OpenThread {
                thread_id: thread()
            }
        );
    }

    #[tokio::test]
    async fn observer_upgrade_does_not_publish_a_historical_gate_that_already_resumed() {
        let inbox = inbox();
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:recovered-before-upgrade").expect("gate ref");
        let mut historical_suspension = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        historical_suspension.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let mut current = historical_suspension.state.clone();
        current.status = ProcessLifecycleStatus::Queued;
        current.suspension = None;
        let primary = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        let observer = AuthNotificationBackfillProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(CurrentProcessSource::new(current)),
        );

        assert_eq!(
            primary.process_observer_id(),
            "run-outcome-inbox-commit-observer-v1"
        );
        assert_eq!(
            observer.process_observer_id(),
            "run-auth-inbox-backfill-observer-v1"
        );
        observer
            .observe_process_commit(historical_suspension)
            .await
            .expect("historical suspension reconciliation");

        assert!(records(inbox.as_ref()).await.is_empty());
    }

    #[tokio::test]
    async fn auth_backfill_does_not_consume_capacity_with_historical_outcomes() {
        let inbox = inbox_with_capacity(1);
        let auth_run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:pending-during-upgrade").expect("gate ref");
        let mut suspended = commit(
            auth_run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let observer = AuthNotificationBackfillProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(CurrentProcessSource::new(suspended.state.clone())),
        );

        observer
            .observe_process_commit(commit(
                TurnRunId::new(),
                ProcessLifecycleStatus::Completed,
                ProcessJournalKind::Completed,
                "scheduled_trigger",
            ))
            .await
            .expect("historical outcome is outside auth backfill scope");
        observer
            .observe_process_commit(suspended)
            .await
            .expect("pending auth suspension uses the remaining Inbox capacity");

        assert_eq!(
            records(inbox.as_ref()).await,
            vec![NotificationKind::AuthenticationRequired]
        );
    }

    #[tokio::test]
    async fn failed_publication_replay_does_not_revive_auth_after_the_run_recovers() {
        let inbox = inbox_failing_first_write();
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:recovered-during-outage").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let current = Arc::new(CurrentProcessSource::new(suspended.state.clone()));
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        )
        .with_process_journal_source(Arc::clone(&current)
            as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>);

        observer
            .observe_process_commit(suspended.clone())
            .await
            .expect_err("the first durable Inbox write is unavailable");

        let mut recovered = suspended.state.clone();
        recovered.status = ProcessLifecycleStatus::Queued;
        recovered.suspension = None;
        current.replace(recovered);
        observer
            .observe_process_commit(suspended)
            .await
            .expect("replay reconciles against the recovered process state");

        assert!(records(inbox.as_ref()).await.is_empty());
    }

    #[tokio::test]
    async fn stale_auth_publication_resolves_at_the_committed_transition_time() {
        let inbox = inbox();
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:recovered-before-replay").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let publisher = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        publisher
            .observe_process_commit(suspended.clone())
            .await
            .expect("publish auth notification");

        let mut recovered = suspended.state.clone();
        recovered.status = ProcessLifecycleStatus::Queued;
        recovered.suspension = None;
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        )
        .with_process_journal_source(Arc::new(CurrentProcessSource::new(recovered)));
        let committed_at = Utc::now() + TimeDelta::hours(1);
        suspended.occurred_at = Some(committed_at);
        observer
            .observe_process_commit(suspended)
            .await
            .expect("stale publication reconciles against recovered state");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list resolved auth notification");
        assert_eq!(page.notifications[0].resolved_at, Some(committed_at));
    }

    #[tokio::test]
    async fn resumed_commit_retries_auth_resolution_after_an_inbox_outage() {
        let inbox = inbox_failing_second_write();
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:resume-after-inbox-outage").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        observer
            .observe_process_commit(suspended.clone())
            .await
            .expect("publish auth notification");

        let mut resumed = suspended;
        resumed.kind = ProcessJournalKind::Resumed;
        resumed.state.status = ProcessLifecycleStatus::Queued;
        resumed.state.suspension = None;
        observer
            .observe_process_commit(resumed.clone())
            .await
            .expect_err("the first durable resolution attempt fails");
        observer
            .observe_process_commit(resumed)
            .await
            .expect("durable observer replay resolves the notification");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list reconciled auth notification");
        assert_eq!(page.notifications.len(), 1);
        assert!(page.notifications[0].resolved_at.is_some());
    }

    #[tokio::test]
    async fn resumed_old_gate_does_not_resolve_the_current_auth_gate() {
        let inbox = inbox();
        let run_id = TurnRunId::new();
        let old_gate = TurnGateRef::new("gate:old-auth").expect("old gate");
        let current_gate = TurnGateRef::new("gate:current-auth").expect("current gate");

        let mut old_suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        old_suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(old_gate),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let publisher = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        publisher
            .observe_process_commit(old_suspended.clone())
            .await
            .expect("publish old gate");

        let mut current_suspended = old_suspended.clone();
        current_suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(current_gate.clone()),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        publisher
            .observe_process_commit(current_suspended.clone())
            .await
            .expect("publish current gate before old resume is delivered");

        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        )
        .with_process_journal_source(Arc::new(CurrentProcessSource::new(current_suspended.state)));
        let mut resumed_old = old_suspended;
        resumed_old.kind = ProcessJournalKind::Resumed;
        resumed_old.state.status = ProcessLifecycleStatus::Queued;
        resumed_old.state.suspension = None;
        observer
            .observe_process_commit(resumed_old)
            .await
            .expect("reconcile old resume");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list auth notifications");
        let mut old_is_resolved = false;
        let mut current_is_open = false;
        for notification in page.notifications {
            let lifecycle_ref = notification
                .source
                .lifecycle_ref
                .as_ref()
                .map(LifecycleRef::as_str);
            if lifecycle_ref == Some("gate:old-auth") {
                old_is_resolved = notification.resolved_at.is_some();
            }
            if lifecycle_ref == Some(current_gate.as_str()) {
                current_is_open = notification.resolved_at.is_none();
            }
        }
        assert!(old_is_resolved, "the resumed old gate must be closed");
        assert!(
            current_is_open,
            "an older resume must not close the current gate notification"
        );
    }

    #[tokio::test]
    async fn denied_resume_keeps_auth_actionable_until_verified_recovery() {
        let inbox = inbox();
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:denied-then-authorized").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        observer
            .observe_process_commit(suspended)
            .await
            .expect("publish auth notification");

        let mut denied = commit(
            run_id,
            ProcessLifecycleStatus::Queued,
            ProcessJournalKind::Resumed,
            "web_ui",
        );
        denied.state.metadata["agent_turn"]["auth_resume_disposition"] = json!("denied");
        observer
            .observe_process_commit(denied)
            .await
            .expect("observe denied resume");
        let denied_page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list after denial");
        assert!(
            denied_page.notifications[0].resolved_at.is_none(),
            "denial does not prove credential recovery"
        );

        observer
            .observe_process_commit(commit(
                run_id,
                ProcessLifecycleStatus::Queued,
                ProcessJournalKind::Resumed,
                "web_ui",
            ))
            .await
            .expect("observe verified resume");
        let recovered_page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list after recovery");
        assert!(
            recovered_page.notifications[0].resolved_at.is_some(),
            "verified recovery closes the notification"
        );
    }

    #[tokio::test]
    async fn same_auth_gate_reopens_when_it_blocks_again_after_a_verified_resume() {
        let inbox = inbox();
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:resume-then-reblock").expect("gate ref");
        let mut suspended = commit(
            run_id,
            ProcessLifecycleStatus::Suspended,
            ProcessJournalKind::Suspended,
            "web_ui",
        );
        suspended.state.suspension = Some(ProcessSuspension {
            kind: ProcessSuspensionKind::Authorization,
            gate_ref: Some(gate_ref),
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        });
        let current = Arc::new(CurrentProcessSource::new(suspended.state.clone()));
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        )
        .with_process_journal_source(Arc::clone(&current)
            as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>);

        observer
            .observe_process_commit(suspended.clone())
            .await
            .expect("publish initial auth notification");

        let mut resumed = suspended.clone();
        resumed.kind = ProcessJournalKind::Resumed;
        resumed.state.status = ProcessLifecycleStatus::Queued;
        resumed.state.suspension = None;
        current.replace(resumed.state.clone());
        observer
            .observe_process_commit(resumed)
            .await
            .expect("verified resume closes the auth notification");

        current.replace(suspended.state.clone());
        observer
            .observe_process_commit(suspended)
            .await
            .expect("same gate blocks again");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list re-blocked auth notification");
        assert_eq!(page.notifications.len(), 1);
        assert!(
            page.notifications[0].resolved_at.is_none(),
            "the same stable gate must become actionable again when it re-blocks"
        );
    }

    /// A timed-out fire publishes an actionable block and the delivery watcher
    /// then returns, so nothing else ever observes that run again. Only a
    /// terminal fact can retire the record.
    #[tokio::test]
    async fn a_terminal_run_resolves_the_block_left_behind_by_a_delivery_timeout() {
        for status in [
            ProcessLifecycleStatus::Completed,
            ProcessLifecycleStatus::Failed,
            ProcessLifecycleStatus::Cancelled,
        ] {
            let inbox = inbox();
            let threads = Arc::new(InMemorySessionThreadService::default());
            threads
                .ensure_thread(EnsureThreadRequest {
                    scope: thread_scope(),
                    thread_id: Some(thread()),
                    created_by_actor_id: user().to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .expect("thread");
            let run_id = TurnRunId::new();
            let recipient = NotificationRecipient {
                tenant_id: tenant(),
                user_id: user(),
            };

            inbox
                .publish(PublishNotificationRequest {
                    id: crate::run_delivery::run_notification_inbox_id(
                        run_id,
                        NotificationKind::RunBlocked,
                        Some(crate::run_delivery::TIMEOUT_LIFECYCLE_REF),
                    )
                    .expect("timeout block id"),
                    recipient: recipient.clone(),
                    kind: NotificationKind::RunBlocked,
                    severity: NotificationSeverity::Warning,
                    source: NotificationSource {
                        thread_id: thread(),
                        turn_run_id: Some(run_id),
                        lifecycle_ref: Some(
                            LifecycleRef::new(crate::run_delivery::TIMEOUT_LIFECYCLE_REF)
                                .expect("timeout lifecycle ref"),
                        ),
                        credential_providers: Vec::new(),
                    },
                    action: NotificationAction::OpenThread {
                        thread_id: thread(),
                    },
                    initial_state: NotificationInitialState::Open,
                    occurred_at: Utc::now(),
                })
                .await
                .expect("seed the timed-out block");

            if status == ProcessLifecycleStatus::Completed {
                threads
                    .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                        scope: thread_scope(),
                        thread_id: thread(),
                        turn_run_id: run_id.to_string(),
                        content: MessageContent::text("durable final reply"),
                    })
                    .await
                    .expect("completed run final reply");
            }

            let observer = RunOutcomeProcessCommitObserver::new(
                Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
                Arc::clone(&threads) as Arc<dyn SessionThreadService>,
            );
            observer
                .observe_process_commit(commit(
                    run_id,
                    status,
                    ProcessJournalKind::Completed,
                    "scheduled_trigger",
                ))
                .await
                .expect("terminal commit");

            let page = inbox
                .list(ListNotificationsRequest {
                    recipient,
                    limit: 10,
                    cursor: None,
                    include_archived: true,
                })
                .await
                .expect("list");
            let block = page
                .notifications
                .iter()
                .find(|record| record.kind == NotificationKind::RunBlocked)
                .expect("the block record survives");
            assert!(
                block.resolved_at.is_some(),
                "{status:?} must retire the block a delivery timeout left open",
            );
        }
    }

    #[tokio::test]
    async fn webui_resource_block_reconciles_after_observer_restart_and_recovery() {
        const GATE: &str = "gate:budget-00000000-0000-0000-0000-000000000001";
        let inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let blocked = resource_block_commit(run_id, "web_ui", GATE);

        let first_observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        );
        first_observer
            .observe_process_commit(blocked.clone())
            .await
            .expect("publish resource block");
        first_observer
            .observe_process_commit(blocked)
            .await
            .expect("replay resource block");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list resource block");
        assert_eq!(page.notifications.len(), 1, "replay reuses one identity");
        assert_eq!(page.notifications[0].kind, NotificationKind::RunBlocked);
        assert!(page.notifications[0].resolved_at.is_none());
        assert_eq!(
            page.notifications[0]
                .source
                .lifecycle_ref
                .as_ref()
                .map(LifecycleRef::as_str),
            Some(GATE),
            "the existing gate-derived identity remains stable without exposing suspension detail"
        );

        drop(first_observer);
        let restarted_observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
        );
        restarted_observer
            .observe_process_commit(commit(
                run_id,
                ProcessLifecycleStatus::Queued,
                ProcessJournalKind::Resumed,
                "web_ui",
            ))
            .await
            .expect("reconcile recovered resource block");

        let recovered = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list recovered resource block");
        assert!(
            recovered.notifications[0].resolved_at.is_some(),
            "durable recovery must resolve the resource block after restart"
        );
    }

    #[test]
    fn primary_outcome_observer_preserves_its_v1_cursor_identity() {
        let observer = RunOutcomeProcessCommitObserver::new(
            inbox() as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );

        assert_eq!(
            observer.process_observer_id(),
            "run-outcome-inbox-commit-observer-v1",
            "resource-block rollout must not replay every historical terminal outcome"
        );
    }

    #[tokio::test]
    async fn resource_backfill_only_materializes_a_still_current_resource_gate() {
        const GATE: &str = "gate:budget-00000000-0000-0000-0000-000000000002";
        let notification_inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let blocked = resource_block_commit(run_id, "web_ui", GATE);
        let recovered = commit(
            run_id,
            ProcessLifecycleStatus::Queued,
            ProcessJournalKind::Resumed,
            "web_ui",
        );
        let source = Arc::new(CurrentProcessJournalSource::new(recovered.state.clone()));
        let observer = ResourceBlockBackfillProcessCommitObserver::new(
            Arc::clone(&notification_inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::clone(&threads) as Arc<dyn SessionThreadService>,
            Arc::clone(&source) as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );

        observer
            .observe_process_commit(blocked.clone())
            .await
            .expect("settled historical resource block is ignored");
        observer
            .observe_process_commit(commit(
                run_id,
                ProcessLifecycleStatus::Failed,
                ProcessJournalKind::Failed,
                "scheduled_trigger",
            ))
            .await
            .expect("terminal outcomes are outside the narrow backfill");
        assert!(
            records(notification_inbox.as_ref()).await.is_empty(),
            "upgrade replay must not resurrect settled blocks or terminal notifications"
        );

        source.set_current(blocked.state.clone());
        observer
            .observe_process_commit(blocked.clone())
            .await
            .expect("current resource block is backfilled");
        assert_eq!(
            records(notification_inbox.as_ref()).await,
            vec![NotificationKind::RunBlocked]
        );

        let race_inbox = inbox();
        source.set_sequence([blocked.state, recovered.state]);
        let race_observer = ResourceBlockBackfillProcessCommitObserver::new(
            Arc::clone(&race_inbox) as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
            source as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );
        race_observer
            .observe_process_commit(resource_block_commit(run_id, "web_ui", GATE))
            .await
            .expect("recovery racing the backfill is reconciled");
        let raced = race_inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list raced resource block");
        assert_eq!(raced.notifications.len(), 1);
        assert!(
            raced.notifications[0].resolved_at.is_some(),
            "a block recovered during backfill must not remain actionable"
        );
    }

    #[tokio::test]
    async fn resource_backfill_replay_retires_a_stale_gate_after_compensation_failed() {
        const GATE: &str = "gate:budget-00000000-0000-0000-0000-000000000003";
        let backing_inbox = inbox();
        let failing_inbox = Arc::new(FailFirstResolveInbox::new(
            Arc::clone(&backing_inbox) as Arc<dyn NotificationInboxStorePort>
        ));
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let blocked = resource_block_commit(run_id, "web_ui", GATE);
        let recovered = commit(
            run_id,
            ProcessLifecycleStatus::Queued,
            ProcessJournalKind::Resumed,
            "web_ui",
        );
        let source = Arc::new(CurrentProcessJournalSource::new(blocked.state.clone()));
        source.set_sequence([blocked.state.clone(), recovered.state.clone()]);
        let first_observer = ResourceBlockBackfillProcessCommitObserver::new(
            Arc::clone(&failing_inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::clone(&threads) as Arc<dyn SessionThreadService>,
            Arc::clone(&source) as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );

        first_observer
            .observe_process_commit(blocked.clone())
            .await
            .expect_err("post-publish compensation is unavailable");
        let stranded = backing_inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list stranded resource block");
        assert!(stranded.notifications[0].resolved_at.is_none());

        source.set_current(recovered.state);
        let restarted_observer = ResourceBlockBackfillProcessCommitObserver::new(
            failing_inbox as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
            source as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );
        restarted_observer
            .observe_process_commit(blocked)
            .await
            .expect("stale-on-entry replay reconciles the previously published gate");

        let reconciled = backing_inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list reconciled resource block");
        assert!(
            reconciled.notifications[0].resolved_at.is_some(),
            "replay must retire the stale record even when it did not publish it"
        );
    }

    #[tokio::test]
    async fn resource_backfill_rechecks_replacement_gates_until_current() {
        const HISTORICAL_GATE: &str = "gate:budget-00000000-0000-0000-0000-000000000004";
        const REPLACEMENT_GATE_B: &str = "gate:budget-00000000-0000-0000-0000-000000000005";
        const REPLACEMENT_GATE_C: &str = "gate:budget-00000000-0000-0000-0000-000000000006";
        let notification_inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let historical = resource_block_commit(run_id, "web_ui", HISTORICAL_GATE);
        let replacement_b = resource_block_commit(run_id, "web_ui", REPLACEMENT_GATE_B);
        let replacement_c = resource_block_commit(run_id, "web_ui", REPLACEMENT_GATE_C);
        let recovered = commit(
            run_id,
            ProcessLifecycleStatus::Queued,
            ProcessJournalKind::Resumed,
            "web_ui",
        );
        let source = Arc::new(CurrentProcessJournalSource::new(
            replacement_b.state.clone(),
        ));
        source.set_sequence([replacement_b.state, replacement_c.state, recovered.state]);
        let observer = ResourceBlockBackfillProcessCommitObserver::new(
            Arc::clone(&notification_inbox) as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
            source as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );

        observer
            .observe_process_commit(historical)
            .await
            .expect("replacement gate recovery is reconciled");

        let reconciled = notification_inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list replacement resource block");
        assert_eq!(reconciled.notifications.len(), 2);
        for gate_ref in [REPLACEMENT_GATE_B, REPLACEMENT_GATE_C] {
            let notification = reconciled
                .notifications
                .iter()
                .find(|notification| {
                    notification
                        .source
                        .lifecycle_ref
                        .as_ref()
                        .is_some_and(|lifecycle_ref| lifecycle_ref.as_str() == gate_ref)
                })
                .expect("replacement gate notification");
            assert!(
                notification.resolved_at.is_some(),
                "replacement gate {gate_ref} must be resolved"
            );
        }
        assert!(
            reconciled
                .notifications
                .iter()
                .all(|notification| notification.resolved_at.is_some()),
            "no replacement gate may remain actionable after recovery"
        );
    }

    #[tokio::test]
    async fn resource_backfill_reopens_a_current_gate_resolved_by_a_stale_iteration() {
        const HISTORICAL_GATE: &str = "gate:budget-00000000-0000-0000-0000-000000000007";
        const STALE_GATE_B: &str = "gate:budget-00000000-0000-0000-0000-000000000008";
        const CURRENT_GATE_D: &str = "gate:budget-00000000-0000-0000-0000-000000000009";
        let notification_inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let historical = resource_block_commit(run_id, "web_ui", HISTORICAL_GATE);
        let stale_b = resource_block_commit(run_id, "web_ui", STALE_GATE_B);
        let current_d = resource_block_commit(run_id, "web_ui", CURRENT_GATE_D);
        let primary_observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&notification_inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        );
        primary_observer
            .observe_process_commit(current_d.clone())
            .await
            .expect("primary observer publishes the current gate first");

        let source = Arc::new(CurrentProcessJournalSource::new(stale_b.state.clone()));
        source.set_sequence([stale_b.state, current_d.state.clone(), current_d.state]);
        let backfill_observer = ResourceBlockBackfillProcessCommitObserver::new(
            Arc::clone(&notification_inbox) as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
            source as Arc<dyn ProcessJournalSource<Error = ironclaw_turns::TurnError>>,
        );
        backfill_observer
            .observe_process_commit(historical)
            .await
            .expect("stale backfill converges on the current gate");

        let reconciled = notification_inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list reconciled replacement gates");
        let current = reconciled
            .notifications
            .iter()
            .find(|notification| {
                notification
                    .source
                    .lifecycle_ref
                    .as_ref()
                    .is_some_and(|lifecycle_ref| lifecycle_ref.as_str() == CURRENT_GATE_D)
            })
            .expect("current replacement gate notification");
        assert!(
            current.resolved_at.is_none(),
            "the authoritative current gate must be actionable after stale reconciliation"
        );
        assert!(
            reconciled.notifications.iter().any(|notification| {
                notification
                    .source
                    .lifecycle_ref
                    .as_ref()
                    .is_some_and(|lifecycle_ref| lifecycle_ref.as_str() == STALE_GATE_B)
                    && notification.resolved_at.is_some()
            }),
            "the stale replacement gate must remain resolved"
        );
    }

    #[tokio::test]
    async fn completed_background_run_publishes_only_after_exact_reply_is_finalized() {
        let inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        threads
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope(),
                thread_id: Some(thread()),
                created_by_actor_id: user().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread");
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        );

        let completion = commit(
            run_id,
            ProcessLifecycleStatus::Completed,
            ProcessJournalKind::Completed,
            "scheduled_trigger",
        );
        let committed_at = completion.occurred_at.expect("committed timestamp");
        let error = observer
            .observe_process_commit(completion.clone())
            .await
            .expect_err("missing final reply must retain the observer cursor for retry");
        assert!(error.contains("no finalized assistant reply"), "{error}");
        assert!(records(inbox.as_ref()).await.is_empty());

        threads
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope(),
                thread_id: thread(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text("durable final reply"),
            })
            .await
            .expect("final reply");
        observer
            .observe_process_commit(completion)
            .await
            .expect("completion notification");

        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: tenant(),
                    user_id: user(),
                },
                limit: 10,
                cursor: None,
                include_archived: false,
            })
            .await
            .expect("list completion notification");
        assert_eq!(page.notifications.len(), 1);
        assert_eq!(page.notifications[0].kind, NotificationKind::RunCompleted);
        assert_eq!(page.notifications[0].resolved_at, Some(committed_at));
        assert_eq!(
            page.notifications[0].created_at, committed_at,
            "completion ordering uses the committed journal transition timestamp"
        );
    }

    #[tokio::test]
    async fn durable_journal_retries_completion_after_the_final_reply_is_persisted() {
        let inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        threads
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope(),
                thread_id: Some(thread()),
                created_by_actor_id: user().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread");
        let process_filesystem = process_filesystem();
        let store = ProcessJournalStore::new(Arc::clone(&process_filesystem));
        store
            .subscribe_process_observer(Arc::new(RunOutcomeProcessCommitObserver::new(
                Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
                Arc::clone(&threads) as Arc<dyn SessionThreadService>,
            )))
            .expect("subscribe outcome observer");
        let run_id = TurnRunId::new();
        let process_id = ProcessId::from_uuid(run_id.as_uuid());
        let resource_scope = ResourceScope {
            tenant_id: tenant(),
            user_id: user(),
            agent_id: Some(agent()),
            project_id: None,
            mission_id: None,
            thread_id: Some(thread()),
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        };
        store
            .submit_process(SubmitProcessRequest {
                process_id,
                process_kind: ProcessKind::AgentTurn,
                scope: resource_scope.clone(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id: Some(user()),
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: json!({
                    "agent_turn": {
                        "product_context": { "origin": "scheduled_trigger" },
                        "execution_outcome": "result_available"
                    }
                }),
            })
            .await
            .expect("submit process");
        let claimed = store
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: ProcessWorkerId::from_trusted("outcome-observer-worker"),
                scope_filter: Some(resource_scope),
                process_id_filter: Some(process_id),
                process_kind_filter: Some(ProcessKind::AgentTurn),
                max_processes: 1,
            })
            .await
            .expect("claim process")
            .pop()
            .expect("process is claimable");
        store
            .complete_process(ProcessStateTransitionRequest {
                lease: ProcessLeaseRequest {
                    process_id,
                    worker_id: claimed.worker_id,
                    lease_token: claimed.lease_token,
                },
                metadata: None,
            })
            .await
            .expect("terminal process commit remains successful");
        let committed_at = store
            .read_process_journal_log_after(None, 16)
            .await
            .expect("read committed journal row")
            .entries
            .into_iter()
            .find(|entry| {
                entry.process_id == process_id && entry.kind == ProcessJournalKind::Completed
            })
            .and_then(|entry| entry.occurred_at)
            .expect("committed journal timestamp");
        assert!(
            records(inbox.as_ref()).await.is_empty(),
            "the first delivery attempt cannot publish before the exact reply exists"
        );

        threads
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope(),
                thread_id: thread(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text("durable final reply"),
            })
            .await
            .expect("persist final reply before observer retry");

        let page = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let page = inbox
                    .list(ListNotificationsRequest {
                        recipient: NotificationRecipient {
                            tenant_id: tenant(),
                            user_id: user(),
                        },
                        limit: 10,
                        cursor: None,
                        include_archived: false,
                    })
                    .await
                    .expect("list completion notification");
                if !page.notifications.is_empty() {
                    break page;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("durable observer retries the commit");
        assert_eq!(page.notifications.len(), 1);
        assert_eq!(page.notifications[0].kind, NotificationKind::RunCompleted);
        assert_eq!(page.notifications[0].created_at, committed_at);
        assert_eq!(page.notifications[0].resolved_at, Some(committed_at));
    }

    #[tokio::test]
    async fn failed_background_run_is_deduplicated_and_foreground_completion_is_ignored() {
        let inbox = inbox();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            threads as Arc<dyn SessionThreadService>,
        );
        let failed_run = TurnRunId::new();
        let failed = commit(
            failed_run,
            ProcessLifecycleStatus::Failed,
            ProcessJournalKind::Failed,
            "scheduled_trigger",
        );

        observer
            .observe_process_commit(failed.clone())
            .await
            .expect("failed notification");
        observer
            .observe_process_commit(failed)
            .await
            .expect("replayed failure");
        observer
            .observe_process_commit(commit(
                TurnRunId::new(),
                ProcessLifecycleStatus::Completed,
                ProcessJournalKind::Completed,
                "web_ui",
            ))
            .await
            .expect("foreground completion is ignored");

        assert_eq!(
            records(inbox.as_ref()).await,
            vec![NotificationKind::RunFailed]
        );
    }
    #[tokio::test]
    async fn a_recovery_required_run_publishes_the_failure_outcome() {
        let inbox = inbox();
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );

        observer
            .observe_process_commit(commit(
                TurnRunId::new(),
                ProcessLifecycleStatus::RecoveryRequired,
                ProcessJournalKind::RecoveryRequired,
                "scheduled_trigger",
            ))
            .await
            .expect("recovery-required commit");

        assert_eq!(
            records(inbox.as_ref()).await,
            vec![NotificationKind::RunFailed],
            "a run needing recovery is reported as a failed outcome"
        );
    }

    #[tokio::test]
    async fn a_child_or_ownerless_run_publishes_nothing() {
        let inbox = inbox();
        let observer = RunOutcomeProcessCommitObserver::new(
            Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>,
            Arc::new(InMemorySessionThreadService::default()) as Arc<dyn SessionThreadService>,
        );

        observer
            .observe_process_commit(excluded_commit(TurnRunId::new(), 1, false))
            .await
            .expect("a subagent turn is screened out, not an error");
        observer
            .observe_process_commit(excluded_commit(TurnRunId::new(), 0, true))
            .await
            .expect("an ownerless thread is screened out, not an error");

        assert!(
            records(inbox.as_ref()).await.is_empty(),
            "neither a child run nor an ownerless thread reaches a user's inbox"
        );
    }
}
