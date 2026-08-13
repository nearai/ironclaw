//! Journal-commit adapter for run-completion notifications (2026-08-13
//! design §5.2, §13).
//!
//! Composition owns the kernel-facing half: it filters committed process
//! batches to terminal successful `Completed` top-level agent-turn
//! processes with an owner user and a thread scope, then hands typed
//! observations to the product-owned `RunCompletionIngest`. Keeping the
//! `ironclaw_processes` vocabulary here preserves the §13 seam — when
//! `AgentExecution::subscribe` becomes the canonical observation facade, a
//! new adapter maps onto the same ingest port and nothing downstream moves.
//!
//! Retry contract: `Err` is returned only for retryable ingest failures, so
//! the journal store's durable observer cursor holds position and replays.
//! Ineligible and permanently-unresolvable runs return `Ok` (sanitized
//! anomaly metrics only) — one malformed historical run must not wedge the
//! shared cursor forever.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::run_completions::ingest::{
    CompletionIngestOutcome, CompletionObservation, RunCompletionIngest,
};
use ironclaw_host_api::resource::SYSTEM_RESERVED_ID;
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use ironclaw_processes::{ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalKind};

pub(crate) struct RunCompletionJournalObserver {
    ingest: Arc<RunCompletionIngest>,
}

impl RunCompletionJournalObserver {
    pub(crate) fn new(ingest: Arc<RunCompletionIngest>) -> Self {
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
            completed_at: commit.state.created_at,
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
                    target: "ironclaw::reborn::run_completions",
                    error = %error,
                    "terminal run-completion ingest failure; advancing observer cursor",
                );
                Ok(())
            }
        }
    }
}
