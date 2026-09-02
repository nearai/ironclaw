//! Journal-commit adapter for run-completion notifications (2026-08-13
//! design §5.2, §13).
//!
//! The kernel-facing half of ingest: it filters committed process batches to
//! terminal successful `Completed` top-level agent-turn processes with an
//! owner user and a thread scope, then hands typed observations to
//! [`RunCompletionIngest`]. It lives here, beside the other product journal
//! observers (`run_outcome_observer`, `suggestions_observer`), because which
//! runs notify is product policy; composition only constructs and subscribes
//! it. The §13 seam is preserved — when `AgentExecution::subscribe` becomes
//! the canonical observation facade, a new adapter maps onto the same ingest
//! port and nothing downstream moves.
//!
//! Retry contract: `Err` is returned only for retryable ingest failures, so
//! the journal store's durable observer cursor holds position and replays.
//! Ineligible and permanently-unresolvable runs return `Ok` (sanitized
//! anomaly metrics only) — one malformed historical run must not wedge the
//! shared cursor forever.

use std::sync::Arc;

use crate::run_completions::TRACE_TARGET;
use crate::run_completions::ingest::{
    CompletionIngestOutcome, CompletionObservation, RunCompletionIngest,
};
use async_trait::async_trait;
use ironclaw_host_api::resource::SYSTEM_RESERVED_ID;
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use ironclaw_processes::{ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalKind};

/// The production observer composition registers on the process journal; the
/// integration harness registers the same type on the real journal.
pub struct RunCompletionJournalObserver {
    ingest: Arc<RunCompletionIngest>,
}

impl RunCompletionJournalObserver {
    pub fn new(ingest: Arc<RunCompletionIngest>) -> Self {
        Self { ingest }
    }

    fn observation(commit: &ProcessJournalCommit) -> Option<CompletionObservation> {
        // Terminal successful commit only — `Stopped` projects to a
        // cancelled turn kind and never notifies.
        if commit.kind != ProcessJournalKind::Completed {
            return None;
        }
        if commit.state.process_kind != ironclaw_processes::ProcessKind::AgentTurn {
            return None;
        }
        // Top-level only: subagents always carry parent/root process ids.
        if commit.state.parent_process_id.is_some() || commit.state.root_process_id.is_some() {
            return None;
        }
        let owner_user_id = commit.state.owner_user_id.clone()?;
        if owner_user_id.as_str() == SYSTEM_RESERVED_ID
            || commit.state.scope.user_id.as_str() == SYSTEM_RESERVED_ID
        {
            return None;
        }
        // Thread-backed only; system inference and detached maintenance
        // work never carry a thread scope.
        let thread_id = commit.state.scope.thread_id.clone()?;
        let scope = TurnScope::new_with_owner(
            commit.state.scope.tenant_id.clone(),
            commit.state.scope.agent_id.clone(),
            commit.state.scope.project_id.clone(),
            thread_id,
            Some(owner_user_id.clone()),
        );
        Some(CompletionObservation {
            run_id: TurnRunId::from_uuid(commit.state.process_id.as_uuid()),
            scope,
            owner_user_id,
            // The journal's per-commit occurrence instant when recorded
            // (post-#7700); snapshot creation time for older commits.
            completed_at: commit.occurred_at.unwrap_or(commit.state.created_at),
        })
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for RunCompletionJournalObserver {
    fn process_observer_id(&self) -> &'static str {
        "web-app-run-completion-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        let Some(observation) = Self::observation(&commit) else {
            return Ok(());
        };
        match self.ingest.ingest(observation).await {
            Ok(
                CompletionIngestOutcome::NoticeCreated
                | CompletionIngestOutcome::AlreadyRecorded
                | CompletionIngestOutcome::Ineligible
                | CompletionIngestOutcome::NoFinalReply,
            ) => Ok(()),
            Err(error) if error.retryable => Err(error.to_string()),
            Err(error) => {
                // Terminal ingest failure: advancing is deliberate — the
                // sanitized reason is already recorded, and wedging the
                // shared cursor would starve every later completion.
                tracing::debug!(
                    target: TRACE_TARGET,
                    error = %error,
                    "terminal run-completion ingest failure; advancing observer cursor",
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_host_api::ids::{AgentId, ProcessId, TenantId, ThreadId, UserId};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_processes::{
        JournaledProcessSnapshot, ProcessJournalCursor, ProcessKind, ProcessLifecycleStatus,
    };

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("user id")
    }

    /// A terminal successful top-level agent turn on an owner-visible thread:
    /// the one shape that must produce an observation.
    fn eligible_commit() -> ProcessJournalCommit {
        let now = Utc::now();
        ProcessJournalCommit {
            state: JournaledProcessSnapshot {
                process_id: ProcessId::from_uuid(uuid::Uuid::new_v4()),
                process_kind: ProcessKind::AgentTurn,
                scope: ResourceScope {
                    tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                    user_id: user("user-alpha"),
                    agent_id: Some(AgentId::new("agent-alpha").expect("agent")),
                    project_id: None,
                    mission_id: None,
                    thread_id: Some(ThreadId::new("thread-alpha").expect("thread")),
                    invocation_id: ironclaw_host_api::ids::InvocationId::new(),
                },
                status: ProcessLifecycleStatus::Completed,
                suspension: None,
                checkpoint_ref: None,
                checkpoint_kind: None,
                input_ref: None,
                failure: None,
                journal_cursor: ProcessJournalCursor(1),
                lease: None,
                crash_reclaim_count: 0,
                created_at: now,
                owner_user_id: Some(user("user-alpha")),
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                metadata: serde_json::Value::Null,
            },
            kind: ProcessJournalKind::Completed,
            sanitized_reason: None,
            occurred_at: Some(now),
        }
    }

    #[test]
    fn only_completed_top_level_owned_thread_turns_are_observed() {
        let eligible = eligible_commit();
        let observation = RunCompletionJournalObserver::observation(&eligible)
            .expect("a completed top-level owned turn notifies");
        assert_eq!(
            observation.run_id.as_uuid(),
            eligible.state.process_id.as_uuid(),
            "the run identity is the process identity"
        );
        assert_eq!(observation.owner_user_id.as_str(), "user-alpha");
        assert_eq!(observation.scope.thread_id.as_str(), "thread-alpha");
        assert_eq!(
            observation.completed_at,
            eligible.occurred_at.expect("journal instant"),
            "the journal's own occurrence instant is the completion time"
        );

        // Every exclusion the observer screens on, each one alone.
        let mut stopped = eligible_commit();
        stopped.kind = ProcessJournalKind::Stopped;
        let mut not_a_turn = eligible_commit();
        not_a_turn.state.process_kind = ProcessKind::CapabilityInvocation;
        let mut subagent = eligible_commit();
        subagent.state.parent_process_id = Some(ProcessId::from_uuid(uuid::Uuid::new_v4()));
        let mut nested = eligible_commit();
        nested.state.root_process_id = Some(ProcessId::from_uuid(uuid::Uuid::new_v4()));
        let mut ownerless = eligible_commit();
        ownerless.state.owner_user_id = None;
        let mut system_owned = eligible_commit();
        system_owned.state.owner_user_id = Some(ResourceScope::system().user_id);
        let mut system_scope = eligible_commit();
        system_scope.state.scope.user_id = ResourceScope::system().user_id;
        let mut threadless = eligible_commit();
        threadless.state.scope.thread_id = None;
        for (label, commit) in [
            ("stopped turns never notify", stopped),
            ("non-turn processes never notify", not_a_turn),
            ("subagent turns never notify", subagent),
            ("nested turns never notify", nested),
            ("ownerless turns never notify", ownerless),
            ("system-owned turns never notify", system_owned),
            ("system-scoped turns never notify", system_scope),
            ("thread-less turns never notify", threadless),
        ] {
            assert!(
                RunCompletionJournalObserver::observation(&commit).is_none(),
                "{label}"
            );
        }
    }

    #[test]
    fn older_commits_without_an_occurrence_instant_use_snapshot_creation_time() {
        let mut commit = eligible_commit();
        commit.occurred_at = None;
        let observation =
            RunCompletionJournalObserver::observation(&commit).expect("still eligible");
        assert_eq!(observation.completed_at, commit.state.created_at);
    }

    // ---- retry contract through the journal-facing caller ----

    use crate::run_completions::RunCompletionSurfaceServices;
    use crate::run_completions::ingest::RunCompletionIngest;
    use crate::run_completions::records::{
        CompletionDeliveryStateKind, CompletionIntentRecord, CompletionReadEvidence,
        RunCompletionNotice,
    };
    use crate::run_completions::store::{
        NewGrant, NewRunCompletionNotice, NoticeCreateOutcome, RunCompletionNoticeStore,
        RunCompletionNotices, RunCompletionOwner, RunCompletionStoreError,
    };
    use crate::run_completions::stream::RunCompletionStreamHub;
    use chrono::{DateTime, Utc as ChronoUtc};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent, SessionThreadService, ThreadScope,
    };
    use std::sync::atomic::{AtomicU8, Ordering};

    const DUE_OK: u8 = 0;
    const DUE_UNAVAILABLE: u8 = 1;
    const DUE_CONFLICT: u8 = 2;

    /// The real in-memory store with a scripted `mark_owner_due` outcome, so
    /// the observer sees a retryable or a terminal ingest failure on the
    /// first store write of an otherwise eligible completion.
    struct ScriptedDueStore {
        inner: Arc<dyn RunCompletionNotices>,
        due_mode: AtomicU8,
    }

    #[async_trait]
    impl RunCompletionNotices for ScriptedDueStore {
        async fn create_notice(
            &self,
            owner: &RunCompletionOwner,
            new_notice: NewRunCompletionNotice,
        ) -> Result<NoticeCreateOutcome, RunCompletionStoreError> {
            self.inner.create_notice(owner, new_notice).await
        }
        async fn get(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
        ) -> Result<Option<RunCompletionNotice>, RunCompletionStoreError> {
            self.inner.get(owner, notice_id).await
        }
        async fn record_intent(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            intent: CompletionIntentRecord,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner.record_intent(owner, notice_id, intent).await
        }
        async fn mark_read(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            evidence: CompletionReadEvidence,
            read_at: DateTime<ChronoUtc>,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner
                .mark_read(owner, notice_id, evidence, read_at)
                .await
        }
        async fn issue_grant(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            grant: NewGrant,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner.issue_grant(owner, notice_id, grant).await
        }
        async fn acknowledge_presented(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            grant_id: &str,
            presented_at: DateTime<ChronoUtc>,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner
                .acknowledge_presented(owner, notice_id, grant_id, presented_at)
                .await
        }
        async fn regress_expired_grant(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            grant_id: &str,
            closes_at: DateTime<ChronoUtc>,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner
                .regress_expired_grant(owner, notice_id, grant_id, closes_at)
                .await
        }
        async fn claim_push(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            delivery_id: &str,
            claimed_at: DateTime<ChronoUtc>,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner
                .claim_push(owner, notice_id, delivery_id, claimed_at)
                .await
        }
        async fn settle_no_target(
            &self,
            owner: &RunCompletionOwner,
            notice_id: &str,
            settled_at: DateTime<ChronoUtc>,
        ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
            self.inner
                .settle_no_target(owner, notice_id, settled_at)
                .await
        }
        async fn list_after(
            &self,
            owner: &RunCompletionOwner,
            after_sequence: Option<u64>,
            limit: usize,
        ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
            self.inner.list_after(owner, after_sequence, limit).await
        }
        async fn unread_snapshot(
            &self,
            owner: &RunCompletionOwner,
        ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
            self.inner.unread_snapshot(owner).await
        }
        async fn unread_for_thread(
            &self,
            owner: &RunCompletionOwner,
            thread_id: &str,
            limit: usize,
        ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
            self.inner.unread_for_thread(owner, thread_id, limit).await
        }
        async fn in_delivery_state(
            &self,
            owner: &RunCompletionOwner,
            state: CompletionDeliveryStateKind,
            limit: usize,
        ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
            self.inner.in_delivery_state(owner, state, limit).await
        }
        async fn mark_owner_due(
            &self,
            owner: &RunCompletionOwner,
        ) -> Result<(), RunCompletionStoreError> {
            match self.due_mode.load(Ordering::SeqCst) {
                DUE_UNAVAILABLE => Err(RunCompletionStoreError::Unavailable {
                    reason: "scripted outage".to_string(),
                }),
                DUE_CONFLICT => Err(RunCompletionStoreError::Conflict {
                    reason: "scripted shape rejection",
                }),
                _ => self.inner.mark_owner_due(owner).await,
            }
        }
        async fn clear_owner_due(
            &self,
            owner: &RunCompletionOwner,
        ) -> Result<(), RunCompletionStoreError> {
            self.inner.clear_owner_due(owner).await
        }
        async fn due_owners(
            &self,
            scope_owner: &RunCompletionOwner,
        ) -> Result<Vec<RunCompletionOwner>, RunCompletionStoreError> {
            self.inner.due_owners(scope_owner).await
        }
        async fn head_sequence(
            &self,
            owner: &RunCompletionOwner,
        ) -> Result<u64, RunCompletionStoreError> {
            self.inner.head_sequence(owner).await
        }
    }

    fn scripted_services() -> (Arc<RunCompletionSurfaceServices>, Arc<ScriptedDueStore>) {
        let inner = Arc::new(RunCompletionNoticeStore::new(Arc::new(
            ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope: &ResourceScope| {
                MountView::new(vec![
                    MountGrant::new(
                        MountAlias::new(crate::run_completions::store::RUN_NOTICES_MOUNT_ALIAS)?,
                        VirtualPath::new(format!(
                            "/tenants/{}/users/{}/run-notices",
                            scope.tenant_id, scope.user_id
                        ))?,
                        MountPermissions::read_write_list_delete(),
                    ),
                    MountGrant::new(
                        MountAlias::new("/tenant-shared")?,
                        VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id))?,
                        MountPermissions::read_write(),
                    ),
                ])
            }),
        ))) as Arc<dyn RunCompletionNotices>;
        let scripted = Arc::new(ScriptedDueStore {
            inner,
            due_mode: AtomicU8::new(DUE_OK),
        });
        let store: Arc<dyn RunCompletionNotices> = scripted.clone();
        let hub = Arc::new(RunCompletionStreamHub::new(Arc::clone(&store)));
        (
            Arc::new(RunCompletionSurfaceServices::new(
                store,
                hub,
                Arc::new(ironclaw_notifications::NoopNotificationInboxStore),
            )),
            scripted,
        )
    }

    /// The journal store keeps its durable observer cursor only while the
    /// observer returns `Err`: a backend outage must replay, a shape
    /// rejection must advance so one bad run never starves every later
    /// completion. Driven through the observer, the only journal-facing
    /// caller.
    #[tokio::test]
    async fn retryable_ingest_failures_hold_the_cursor_and_terminal_failures_advance() {
        let (services, scripted) = scripted_services();
        let threads = InMemorySessionThreadService::default();
        let commit = eligible_commit();
        let thread_id = commit
            .state
            .scope
            .thread_id
            .clone()
            .expect("eligible commit names a thread");
        let thread_scope = ThreadScope {
            tenant_id: commit.state.scope.tenant_id.clone(),
            agent_id: commit
                .state
                .scope
                .agent_id
                .clone()
                .expect("eligible commit names an agent"),
            project_id: commit.state.scope.project_id.clone(),
            owner_user_id: commit.state.owner_user_id.clone(),
            mission_id: None,
        };
        threads
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "user-alpha".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread ensured");
        threads
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope,
                thread_id,
                turn_run_id: TurnRunId::from_uuid(commit.state.process_id.as_uuid()).to_string(),
                content: MessageContent::text("final reply"),
            })
            .await
            .expect("finalized reply appended");
        let observer = RunCompletionJournalObserver::new(Arc::new(RunCompletionIngest::new(
            Arc::clone(&services),
            Arc::new(threads),
        )));

        scripted.due_mode.store(DUE_UNAVAILABLE, Ordering::SeqCst);
        assert!(
            observer
                .observe_process_commit(commit.clone())
                .await
                .is_err(),
            "a backend outage holds the cursor for replay"
        );

        scripted.due_mode.store(DUE_CONFLICT, Ordering::SeqCst);
        assert!(
            observer
                .observe_process_commit(commit.clone())
                .await
                .is_ok(),
            "a shape rejection advances the cursor with a sanitized anomaly"
        );

        scripted.due_mode.store(DUE_OK, Ordering::SeqCst);
        observer
            .observe_process_commit(commit)
            .await
            .expect("the replayed commit ingests once the backend is back");
        let notices = services
            .notices
            .unread_snapshot(&RunCompletionOwner {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                user_id: user("user-alpha"),
            })
            .await
            .expect("snapshot");
        assert_eq!(
            notices.len(),
            1,
            "exactly one notice for the replayed completion"
        );
    }
}
