//! Completion ingest: the narrow product port the journal-commit observer
//! adapter calls (2026-08-13 design §5.2, §13).
//!
//! The observer (composition-owned) delivers already-filtered terminal
//! `Completed` top-level agent-turn observations; this service applies the
//! product eligibility rules — owner-visible thread, finalized assistant
//! reply for the exact run — and idempotently persists one notice. A
//! transcript/store backend failure is a retryable error so the shared
//! durable observer cursor holds position; a permanently unresolvable run
//! (no finalized reply on a successful commit) records a sanitized anomaly
//! and advances, because one malformed historical run must not wedge the
//! cursor forever.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadScope,
};

use super::RunCompletionSurfaceServices;
use super::records::{notice_id_for, thread_tag_for};
use super::store::{
    NewRunCompletionNotice, NoticeCreateOutcome, RunCompletionOwner, RunCompletionStoreError,
};

/// P0 arbitration intent-collection window (§5.4): a host constant, not a
/// user-facing knob.
pub const ARBITRATION_WINDOW_MS: i64 = 1_000;

/// One already-filtered terminal completion observed on the process journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionObservation {
    pub run_id: TurnRunId,
    pub scope: TurnScope,
    pub owner_user_id: UserId,
    pub completed_at: DateTime<Utc>,
}

/// Ingest outcomes, in the observer's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionIngestOutcome {
    /// A new notice was durably created.
    NoticeCreated,
    /// The notice already existed (duplicate journal delivery); the pending
    /// work was re-woken.
    AlreadyRecorded,
    /// The run is not eligible (thread not owner-visible). Not an error.
    Ineligible,
    /// A successful commit with no finalized assistant reply: sanitized
    /// anomaly, cursor advances, no notice (§5.2).
    NoFinalReply,
}

/// Retry classification for the durable observer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("run completion ingest failed ({}): {reason}", if *.retryable { "retryable" } else { "terminal" })]
pub struct CompletionIngestError {
    pub retryable: bool,
    pub reason: String,
}

/// Product-side completion ingest. Composition adapts the process-journal
/// commit observer onto this port; a future `AgentExecution::subscribe`
/// adapter maps onto the same port without touching anything downstream
/// (§13).
pub struct RunCompletionIngest {
    services: Arc<RunCompletionSurfaceServices>,
    thread_service: Arc<dyn SessionThreadService>,
    /// Sanitized anomaly counter (completed runs with no finalized reply).
    /// Operator-visible through logs/metrics only; never carries content.
    anomalies: AtomicU64,
}

impl RunCompletionIngest {
    pub fn new(
        services: Arc<RunCompletionSurfaceServices>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            services,
            thread_service,
            anomalies: AtomicU64::new(0),
        }
    }

    pub fn anomaly_count(&self) -> u64 {
        self.anomalies.load(Ordering::Relaxed)
    }

    pub async fn ingest(
        &self,
        observation: CompletionObservation,
    ) -> Result<CompletionIngestOutcome, CompletionIngestError> {
        let Some(agent_id) = observation.scope.agent_id.clone() else {
            // An agentless scope cannot be a user-visible WebUI thread.
            return Ok(CompletionIngestOutcome::Ineligible);
        };
        let thread_scope = ThreadScope {
            tenant_id: observation.scope.tenant_id.clone(),
            agent_id,
            project_id: observation.scope.project_id.clone(),
            owner_user_id: Some(observation.owner_user_id.clone()),
            mission_id: None,
        };

        // Owner visibility: the metadata-only existence probe. Unknown (or
        // owned by a different scope — implementations collapse both) means
        // ineligible; backend failure is retryable.
        match self
            .thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: observation.scope.thread_id.clone(),
            })
            .await
        {
            Ok(_) => {}
            Err(SessionThreadError::UnknownThread { .. }) => {
                return Ok(CompletionIngestOutcome::Ineligible);
            }
            Err(error) => {
                return Err(CompletionIngestError {
                    retryable: true,
                    reason: format!("thread visibility probe failed: {error}"),
                });
            }
        }

        // The exact finalized assistant reply for this run. The turn
        // contract finalizes it before `Completed`; its absence on a
        // successful commit is the §5.2 anomaly.
        let finalized = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: observation.scope.thread_id.clone(),
                turn_run_id: observation.run_id.to_string(),
            })
            .await
            .map_err(|error| CompletionIngestError {
                retryable: true,
                reason: format!("finalized reply lookup failed: {error}"),
            })?;
        if finalized.is_none() {
            self.anomalies.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                target: "ironclaw::reborn::run_completions",
                run_id = %observation.run_id,
                "completed run resolved to no finalized assistant reply; \
                 recording anomaly and advancing without a notice",
            );
            return Ok(CompletionIngestOutcome::NoFinalReply);
        }

        let owner = RunCompletionOwner {
            tenant_id: observation.scope.tenant_id.clone(),
            user_id: observation.owner_user_id.clone(),
        };
        let notice_id = notice_id_for(
            owner.tenant_id.as_str(),
            owner.user_id.as_str(),
            &observation.run_id.to_string(),
        );
        let thread_tag = thread_tag_for(
            owner.tenant_id.as_str(),
            owner.user_id.as_str(),
            observation.scope.thread_id.as_str(),
        );
        // Durable recovery first (§5.4): the owner lands in the per-tenant
        // due registry BEFORE the notice write, so a crash between the two
        // leaves at worst one extra empty scan, never an unscanned notice.
        // Registry overflow fails retryably and holds the observer cursor.
        self.services
            .notices
            .mark_owner_due(&owner)
            .await
            .map_err(|error| match error {
                RunCompletionStoreError::Unavailable { reason } => CompletionIngestError {
                    retryable: true,
                    reason,
                },
                // A shape rejection cannot heal by replay; advancing with a
                // sanitized anomaly beats wedging the shared cursor, same as
                // `create_notice` below.
                other => CompletionIngestError {
                    retryable: false,
                    reason: other.to_string(),
                },
            })?;
        let closes_at = Utc::now() + ChronoDuration::milliseconds(ARBITRATION_WINDOW_MS);
        let outcome = self
            .services
            .notices
            .create_notice(
                &owner,
                NewRunCompletionNotice {
                    notice_id: notice_id.clone(),
                    run_id: observation.run_id.to_string(),
                    thread_id: observation.scope.thread_id.as_str().to_string(),
                    agent_id: observation
                        .scope
                        .agent_id
                        .as_ref()
                        .map(|agent| agent.as_str().to_string()),
                    project_id: observation
                        .scope
                        .project_id
                        .as_ref()
                        .map(|project| project.as_str().to_string()),
                    thread_tag,
                    terminal_projection_ref: format!("run-completion/{notice_id}"),
                    completed_at: observation.completed_at,
                    arbitration_closes_at: closes_at,
                },
            )
            .await
            .map_err(|error| match error {
                RunCompletionStoreError::Unavailable { reason } => CompletionIngestError {
                    retryable: true,
                    reason,
                },
                other => CompletionIngestError {
                    retryable: false,
                    reason: other.to_string(),
                },
            })?;
        match outcome {
            NoticeCreateOutcome::Created(notice) => {
                self.services.hub.notice_written(&owner, &notice).await;
                self.services.wake_owner(&owner);
                Ok(CompletionIngestOutcome::NoticeCreated)
            }
            NoticeCreateOutcome::AlreadyRecorded(notice) => {
                // Duplicate journal delivery rewrites nothing and wakes the
                // existing notice (§5.2).
                self.services.hub.notice_written(&owner, &notice).await;
                self.services.wake_owner(&owner);
                Ok(CompletionIngestOutcome::AlreadyRecorded)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_completions::RunCompletionSurfaceServices;
    use crate::run_completions::store::{RunCompletionNoticeStore, RunCompletionNotices};
    use crate::run_completions::stream::RunCompletionStreamHub;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent,
    };

    fn services() -> Arc<RunCompletionSurfaceServices> {
        let store = Arc::new(RunCompletionNoticeStore::new(Arc::new(
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
        let hub = Arc::new(RunCompletionStreamHub::new(Arc::clone(&store)));
        Arc::new(RunCompletionSurfaceServices::new(store, hub))
    }

    fn scope(thread_id: &ThreadId) -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-alpha").expect("tenant"),
            Some(AgentId::new("agent-alpha").expect("agent")),
            None,
            thread_id.clone(),
            Some(UserId::new("user-alpha").expect("user")),
        )
    }

    fn observation(run_id: TurnRunId, thread_id: &ThreadId) -> CompletionObservation {
        CompletionObservation {
            run_id,
            scope: scope(thread_id),
            owner_user_id: UserId::new("user-alpha").expect("user"),
            completed_at: Utc::now(),
        }
    }

    async fn seeded_thread(
        threads: &InMemorySessionThreadService,
        thread_id: &str,
    ) -> (ThreadId, ThreadScope) {
        let thread_id = ThreadId::new(thread_id).expect("thread id");
        let thread_scope = ThreadScope {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            agent_id: AgentId::new("agent-alpha").expect("agent"),
            project_id: None,
            owner_user_id: Some(UserId::new("user-alpha").expect("user")),
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
        (thread_id, thread_scope)
    }

    #[tokio::test]
    async fn eligible_completion_creates_one_notice_and_replay_is_idempotent() {
        let services = services();
        let threads = InMemorySessionThreadService::default();
        let (thread_id, thread_scope) = seeded_thread(&threads, "thread-ingest").await;
        let run_id = TurnRunId::from_uuid(uuid::Uuid::new_v4());
        threads
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: thread_scope,
                thread_id: thread_id.clone(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text("final reply"),
            })
            .await
            .expect("finalized reply appended");
        let ingest = RunCompletionIngest::new(Arc::clone(&services), Arc::new(threads));

        let first = ingest
            .ingest(observation(run_id, &thread_id))
            .await
            .expect("ingest succeeds");
        assert_eq!(first, CompletionIngestOutcome::NoticeCreated);
        // Duplicate journal delivery converges on the same record.
        let replay = ingest
            .ingest(observation(run_id, &thread_id))
            .await
            .expect("replay succeeds");
        assert_eq!(replay, CompletionIngestOutcome::AlreadyRecorded);
        let owner = RunCompletionOwner {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
        };
        let unread = services
            .notices
            .unread_snapshot(&owner)
            .await
            .expect("snapshot");
        assert_eq!(unread.len(), 1, "exactly one notice per run");
        assert_eq!(unread[0].run_id, run_id.to_string());
        assert_eq!(
            unread[0].agent_id.as_deref(),
            Some("agent-alpha"),
            "the push fallback needs the typed scope halves"
        );
    }

    #[tokio::test]
    async fn unknown_thread_is_ineligible_not_an_error() {
        let services = services();
        let threads = InMemorySessionThreadService::default();
        let ingest = RunCompletionIngest::new(Arc::clone(&services), Arc::new(threads));
        let thread_id = ThreadId::new("thread-none").expect("thread id");
        let outcome = ingest
            .ingest(observation(
                TurnRunId::from_uuid(uuid::Uuid::new_v4()),
                &thread_id,
            ))
            .await
            .expect("ingest resolves");
        assert_eq!(
            outcome,
            CompletionIngestOutcome::Ineligible,
            "a thread the owner cannot see never notifies"
        );
    }

    #[tokio::test]
    async fn completed_run_without_finalized_reply_records_anomaly_and_advances() {
        let services = services();
        let threads = InMemorySessionThreadService::default();
        let (thread_id, _thread_scope) = seeded_thread(&threads, "thread-noreply").await;
        let ingest = RunCompletionIngest::new(Arc::clone(&services), Arc::new(threads));
        let outcome = ingest
            .ingest(observation(
                TurnRunId::from_uuid(uuid::Uuid::new_v4()),
                &thread_id,
            ))
            .await
            .expect("ingest resolves");
        assert_eq!(outcome, CompletionIngestOutcome::NoFinalReply);
        assert_eq!(ingest.anomaly_count(), 1, "sanitized anomaly is counted");
    }
}
