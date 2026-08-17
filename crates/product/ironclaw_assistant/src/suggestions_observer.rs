//! Durable materialization of suggestion-generation turn results.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{resource::ResourceScope, turn::TurnRunId};
use ironclaw_processes::{
    JournaledProcessSnapshot, ProcessJournalCommit, ProcessJournalCommitObserver,
    ProcessJournalKind, ProcessKind, ProcessLifecycleStatus,
};
use ironclaw_threads::{ReadStructuredFinalizationRequest, SessionThreadService, ThreadScope};

use crate::{
    suggestions::{GeneratedSuggestions, SUGGESTIONS_OUTPUT_NAME, generated_records},
    suggestions_store::{SuggestionsStore, terminal_generation_correlation},
};

const SUGGESTIONS_THREAD_PREFIX: &str = "suggestions-";
const SAFE_FAILURE_REASON: &str = "suggestion generation run failed";
const SAFE_INVALID_OUTPUT_REASON: &str = "suggestion generation returned invalid output";

/// Bridges the durable AgentTurn process journal to the suggestion document.
///
/// The observer never owns model execution and never waits on a request.  The
/// process scheduler owns the run; this component only reads terminal,
/// run-scoped structured-finalization evidence and applies an idempotent CAS.
pub struct SuggestionsProcessCommitObserver {
    store: Arc<dyn SuggestionsStore>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl SuggestionsProcessCommitObserver {
    pub fn new(
        store: Arc<dyn SuggestionsStore>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            store,
            thread_service,
        }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for SuggestionsProcessCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "suggestions-generation-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if !is_suggestion_process(&commit.state) {
            return Ok(());
        }
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        let public_id = commit
            .state
            .scope
            .thread_id
            .as_ref()
            .map(ToString::to_string)
            .ok_or_else(|| "suggestion process is missing its public thread id".to_string())?;
        let scope = commit.state.scope.clone();
        match commit.kind {
            ProcessJournalKind::Completed
                if commit.state.status == ProcessLifecycleStatus::Completed =>
            {
                let thread_id = commit
                    .state
                    .scope
                    .thread_id
                    .clone()
                    .ok_or_else(|| "suggestion process is missing its thread id".to_string())?;
                let thread_scope = thread_scope_for_snapshot(&commit.state)
                    .ok_or_else(|| "suggestion process is missing its agent scope".to_string())?;
                let finalization = self
                    .thread_service
                    .read_structured_finalization(ReadStructuredFinalizationRequest {
                        scope: thread_scope,
                        thread_id,
                        turn_run_id: run_id,
                    })
                    .await
                    .map_err(|error| format!("read suggestion finalization failed: {error}"))?;
                let Some(finalization) = finalization else {
                    return Err(
                        "completed suggestion run has no structured finalization".to_string()
                    );
                };
                let generated =
                    match serde_json::from_value::<GeneratedSuggestions>(finalization.parsed) {
                        Ok(generated) => generated,
                        Err(_) => {
                            self.fail_matching(
                                &scope,
                                &public_id,
                                run_id,
                                SAFE_INVALID_OUTPUT_REASON,
                            )
                            .await?;
                            return Ok(());
                        }
                    };
                let generation_id = match self
                    .read_matching_document(&scope, &public_id, run_id)
                    .await?
                {
                    Some(generation_id) => generation_id,
                    None => return Ok(()),
                };
                let records = match generated_records(&generation_id, generated) {
                    Ok(records) => records,
                    Err(_) => {
                        self.fail_matching(&scope, &public_id, run_id, SAFE_INVALID_OUTPUT_REASON)
                            .await?;
                        return Ok(());
                    }
                };
                self.store
                    .complete_generation_for_run(&scope, &public_id, run_id, records, Utc::now())
                    .await
                    .map_err(|error| format!("settle suggestion generation failed: {error}"))?;
            }
            ProcessJournalKind::Failed
            | ProcessJournalKind::Cancelled
            | ProcessJournalKind::Stopped
            | ProcessJournalKind::Killed
            | ProcessJournalKind::RecoveryRequired => {
                self.fail_matching(&scope, &public_id, run_id, SAFE_FAILURE_REASON)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl SuggestionsProcessCommitObserver {
    async fn read_matching_document(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
    ) -> Result<Option<crate::suggestions_store::GenerationId>, String> {
        let Some(document) = self
            .store
            .read(scope)
            .await
            .map_err(|error| format!("read suggestion document failed: {error}"))?
        else {
            return Ok(None);
        };
        if let Some(correlation) =
            terminal_generation_correlation(&document.generation, public_id, run_id)
        {
            return Ok(Some(correlation.generation_id));
        }
        Ok(None)
    }

    async fn fail_matching(
        &self,
        scope: &ResourceScope,
        public_id: &str,
        run_id: TurnRunId,
        reason: &str,
    ) -> Result<(), String> {
        self.store
            .fail_generation_for_run(scope, public_id, run_id, reason.to_string(), Utc::now())
            .await
            .map_err(|error| format!("record suggestion failure failed: {error}"))?;
        Ok(())
    }
}

fn is_suggestion_process(snapshot: &JournaledProcessSnapshot) -> bool {
    if snapshot.process_kind != ProcessKind::AgentTurn {
        return false;
    }
    let Some(thread_id) = snapshot.scope.thread_id.as_ref() else {
        return false;
    };
    if !thread_id.as_str().starts_with(SUGGESTIONS_THREAD_PREFIX) {
        return false;
    }
    matches!(
        snapshot
            .metadata
            .get("agent_turn")
            .and_then(|metadata| metadata.get("output_contract"))
            .and_then(|contract| contract.get("name"))
            .and_then(serde_json::Value::as_str),
        Some(SUGGESTIONS_OUTPUT_NAME)
    )
}

fn thread_scope_for_snapshot(snapshot: &JournaledProcessSnapshot) -> Option<ThreadScope> {
    Some(ThreadScope {
        tenant_id: snapshot.scope.tenant_id.clone(),
        agent_id: snapshot.scope.agent_id.clone()?,
        project_id: snapshot.scope.project_id.clone(),
        owner_user_id: Some(snapshot.scope.user_id.clone()),
        mission_id: None,
    })
}
