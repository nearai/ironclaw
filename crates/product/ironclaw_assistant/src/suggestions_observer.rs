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
        "suggestions-generation-commit-observer"
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
                    tracing::debug!(
                        %run_id,
                        %public_id,
                        "completed suggestion run has no structured finalization"
                    );
                    self.fail_matching(&scope, &public_id, run_id, SAFE_INVALID_OUTPUT_REASON)
                        .await?;
                    return Ok(());
                };
                let generated =
                    match serde_json::from_str::<GeneratedSuggestions>(&finalization.raw_json) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{AgentId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use ironclaw_processes::{
        JournaledProcessSnapshot, ProcessJournalCommit, ProcessJournalCommitObserver,
        ProcessJournalCursor, ProcessJournalKind, ProcessKind, ProcessLifecycleStatus,
    };
    use ironclaw_threads::{
        EnsureThreadRequest, InMemorySessionThreadService, SessionThreadService, ThreadScope,
    };
    use serde_json::json;

    use super::*;
    use crate::suggestions_store::{
        BeginGenerationRequest, FilesystemSuggestionsStore, GenerationId, SuggestionsStore,
    };

    fn resource_scope(thread_id: Option<ThreadId>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("observer-test-tenant").expect("valid tenant"),
            user_id: UserId::new("observer-test-user").expect("valid user"),
            agent_id: Some(AgentId::new("observer-test-agent").expect("valid agent")),
            project_id: Some(ProjectId::new("observer-test-project").expect("valid project")),
            mission_id: None,
            thread_id,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        }
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("observer-test-tenant").expect("valid tenant"),
            agent_id: AgentId::new("observer-test-agent").expect("valid agent"),
            project_id: Some(ProjectId::new("observer-test-project").expect("valid project")),
            owner_user_id: Some(UserId::new("observer-test-user").expect("valid user")),
            mission_id: None,
        }
    }

    fn store() -> FilesystemSuggestionsStore<InMemoryBackend> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/suggestions").expect("valid mount alias"),
            VirtualPath::new("/tenants/test/users/test/suggestions").expect("valid mount root"),
            MountPermissions::read_write(),
        )])
        .expect("valid mount view");
        FilesystemSuggestionsStore::new(Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        )))
    }

    /// A completed process without finalization evidence is a terminal
    /// invalid-output outcome, not a retryable observer error. Returning `Ok`
    /// is what lets the durable observer cursor advance past the commit.
    #[tokio::test]
    async fn completed_without_finalization_settles_matching_generation_failed() {
        let store = Arc::new(store());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let store_scope = resource_scope(None);
        let public_id = "suggestions-generation-without-finalization";
        let thread_id = ThreadId::new(public_id).expect("valid thread id");
        let process_scope = resource_scope(Some(thread_id.clone()));
        let thread_scope = thread_scope();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "observer-test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists for finalization lookup");

        let generation_id =
            GenerationId::new("generation-without-finalization").expect("valid generation id");
        let run_id = ironclaw_host_api::turn::TurnRunId::new();
        let lease_owner = "observer-test-lease";
        let now = Utc::now();
        store
            .begin_generation(
                &store_scope,
                BeginGenerationRequest {
                    generation_id,
                    public_id: public_id.to_string(),
                    accept_key: public_id.to_string(),
                    client_action_id: Some("observer-test-action".to_string()),
                    prompt_schema_version: 1,
                    lease_owner: lease_owner.to_string(),
                    lease_expires_at: now + chrono::Duration::minutes(1),
                    now,
                },
            )
            .await
            .expect("generation is claimed");
        store
            .bind_generation_run(
                &store_scope,
                &GenerationId::new("generation-without-finalization").expect("valid generation id"),
                lease_owner,
                run_id,
                now,
            )
            .await
            .expect("generation is bound to the process run");

        let snapshot = JournaledProcessSnapshot {
            process_id: ProcessId::from_uuid(run_id.as_uuid()),
            process_kind: ProcessKind::AgentTurn,
            scope: process_scope,
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
            owner_user_id: None,
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            metadata: json!({
                "agent_turn": {
                    "output_contract": {"name": SUGGESTIONS_OUTPUT_NAME}
                }
            }),
        };
        SuggestionsProcessCommitObserver::new(store.clone(), thread_service)
            .observe_process_commit(ProcessJournalCommit {
                state: snapshot,
                kind: ProcessJournalKind::Completed,
                occurred_at: Some(now),
                sanitized_reason: None,
            })
            .await
            .expect("missing finalization is settled, allowing cursor advancement");

        let document = store
            .read(&store_scope)
            .await
            .expect("suggestion document reads")
            .expect("suggestion document exists");
        assert!(matches!(
            document.generation,
            crate::suggestions_store::GenerationState::Failed {
                reason,
                ..
            } if reason == "suggestion generation returned invalid output"
        ));
        assert!(document.suggestions.is_empty());
    }
}
