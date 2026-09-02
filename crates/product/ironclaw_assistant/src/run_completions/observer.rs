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
}
