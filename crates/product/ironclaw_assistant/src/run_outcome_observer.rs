use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    Timestamp,
    execution_policy::{ResultDeliveryPolicy, TurnExecutionPolicy},
    output::OutputContract,
    turn::{TurnExecutionOutcome, TurnOriginKind, TurnRunId},
};
use ironclaw_notifications::{
    LifecycleRef, NotificationAction, NotificationId, NotificationInboxError,
    NotificationInboxStorePort, NotificationKind, NotificationMutationRequest,
    NotificationRecipient, NotificationSeverity, NotificationSource, PublishNotificationRequest,
};
use ironclaw_processes::{
    JournaledProcessSnapshot, ProcessJournalCommit, ProcessJournalCommitObserver,
    ProcessJournalKind, ProcessKind, ProcessLifecycleStatus,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, SessionThreadService, ThreadScope};
use serde::Deserialize;

/// Materializes durable background-run outcomes from the authoritative
/// process journal. It does not observe delivery watchers or poll run state.
pub struct RunOutcomeProcessCommitObserver {
    inbox: Arc<dyn NotificationInboxStorePort>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl RunOutcomeProcessCommitObserver {
    pub fn new(
        inbox: Arc<dyn NotificationInboxStorePort>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            inbox,
            thread_service,
        }
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
                },
                action: NotificationAction::OpenThread { thread_id },
                occurred_at,
            })
            .await
            .map_err(|error| format!("publish run outcome notification failed: {error}"))?;
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
        "run-outcome-inbox-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        let Some(metadata) = eligible_background_run(&commit.state) else {
            return Ok(());
        };
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        let occurred_at = commit.occurred_at.unwrap_or(commit.state.created_at);
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
    product_context: Option<OutcomeProductContext>,
}

#[derive(Debug, Deserialize)]
struct OutcomeProductContext {
    origin: TurnOriginKind,
    #[serde(default)]
    execution_policy: Option<TurnExecutionPolicy>,
}

fn eligible_background_run(snapshot: &JournaledProcessSnapshot) -> Option<OutcomeMetadata> {
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
    if metadata.subagent_depth != 0
        || metadata.ownerless_thread
        || metadata
            .product_context
            .as_ref()
            .is_none_or(|context| context.origin != TurnOriginKind::ScheduledTrigger)
    {
        return None;
    }
    Some(metadata)
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
    let kind = match kind {
        NotificationKind::RunCompleted => "completed",
        NotificationKind::RunFailed => "failed",
        NotificationKind::DeliveryFailed => "delivery-failed",
        NotificationKind::ApprovalRequired => "approval",
        NotificationKind::AuthenticationRequired => "authentication",
        NotificationKind::RunBlocked => "blocked",
    };
    NotificationId::new(format!("run:{run_id}:{kind}"))
        .map_err(|error| format!("build run outcome notification id failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{AgentId, ProcessId, TenantId, ThreadId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
        turn::TurnRunId,
    };
    use ironclaw_notifications::{
        LifecycleRef, ListNotificationsRequest, NotificationAction, NotificationInboxStore,
        NotificationInboxStorePort, NotificationKind, NotificationRecipient, NotificationSeverity,
        NotificationSource, PublishNotificationRequest,
    };
    use ironclaw_processes::{
        ClaimProcessesRequest, JournaledProcessSnapshot, ProcessJournalCommit,
        ProcessJournalCommitObserver, ProcessJournalCursor, ProcessJournalKind,
        ProcessJournalObserverRegistry, ProcessJournalSource, ProcessJournalStore, ProcessKind,
        ProcessLeaseRequest, ProcessLifecycleStatus, ProcessStateTransitionRequest,
        ProcessSubmissionPort, ProcessTransitionPort, ProcessWorkerId, SubmitProcessRequest,
    };
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent, SessionThreadService, ThreadScope,
    };
    use serde_json::json;

    use super::RunOutcomeProcessCommitObserver;

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
                    },
                    action: NotificationAction::OpenThread {
                        thread_id: thread(),
                    },
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
