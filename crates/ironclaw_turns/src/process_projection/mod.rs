//! Agent-turn adapter for the canonical process journal.
//!
//! The process journal vocabulary lives in `ironclaw_processes`. This module
//! maps agent-turn requests and projections onto that neutral process
//! vocabulary. The process journal is authoritative.

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use ironclaw_host_api::{ProcessId, ResourceScope, SYSTEM_RESERVED_ID};
use ironclaw_processes::{
    CancelProcessRequest, ClaimedProcess, GetProcessCheckpointRequest, GetProcessSnapshotRequest,
    JournaledProcessSnapshot, ProcessCheckpointId, ProcessCheckpointPort, ProcessCheckpointRef,
    ProcessConcurrencyClass, ProcessControlPort, ProcessJournalCommit,
    ProcessJournalCommitObserver, ProcessJournalCursor, ProcessJournalEntry, ProcessJournalKind,
    ProcessJournalPage, ProcessJournalSource, ProcessKind, ProcessLeaseSnapshot, ProcessLeaseToken,
    ProcessLifecycleStatus, ProcessOperationId, ProcessOutcome, ProcessRuntimePort,
    ProcessSnapshotSource, ProcessSubmissionPort, ProcessSuspension, ProcessSuspensionKind,
    ProcessTreePort, ProcessWorkerId, PruneReleasedProcessRequest, RecordProcessCheckpointRequest,
    ReleaseProcessTreeRequest, ReserveProcessTreeRequest, ResumeProcessRequest,
    SubmitProcessRequest, SubmitProcessWithCheckpointRequest,
};

use crate::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, BlockedReason, GateKind,
    GateResumeDisposition, ProductTurnContext, ReplyTargetBindingRef, ResolvedRunProfile,
    RunProfileId, RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver,
    RunProfileVersion, SourceBindingRef, SubmitChildRunRequest, TurnActor, TurnAdmissionPolicy,
    TurnCheckpointId, TurnCommittedEventObserver, TurnError, TurnEventKind, TurnEventSink, TurnId,
    TurnLifecycleEvent, TurnOriginKind, TurnRunId, TurnRunProfile, TurnRunRecord, TurnRunState,
    TurnRunnerId, TurnScope, TurnStatus,
    agent_turn_runtime::SpawnTreeReservation,
    events::{
        EventCursor, TurnBlockedGateKind, TurnBlockedGateMetadata, TurnEventPage,
        TurnEventProjectionSource,
    },
    request::{CancelRunRequest, ResumeTurnRequest, RetryTurnRequest, SubmitTurnRequest},
    response::{CancelRunResponse, ResumeTurnResponse, RetryTurnResponse, SubmitTurnResponse},
    run_profile::{LoopModelRouteSnapshot, LoopModelUsage},
    runner::{ClaimedTurnRun, TurnRunnerOutcome},
};

pub const AGENT_TURN_PROCESS_KIND: &str = "agent_turn";

mod loop_checkpoint;
mod store_adapter;
pub use loop_checkpoint::ProcessLoopCheckpointStore;
pub use store_adapter::{
    ProcessJournalStoreTurnAdapter, turn_error_from_process_journal_store_error,
};

pub struct AgentTurnProcessCommitObserver {
    required: Arc<dyn TurnCommittedEventObserver>,
    best_effort: Option<Arc<dyn TurnEventSink>>,
}

impl AgentTurnProcessCommitObserver {
    pub fn new(
        required: Arc<dyn TurnCommittedEventObserver>,
        best_effort: Option<Arc<dyn TurnEventSink>>,
    ) -> Self {
        Self {
            required,
            best_effort,
        }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for AgentTurnProcessCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "agent-turn-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if commit.state.process_kind != ProcessKind::AgentTurn {
            return Ok(());
        }
        let event = turn_lifecycle_event_from_process_journal_entry(ProcessJournalEntry {
            cursor: commit.state.journal_cursor,
            process_id: commit.state.process_id,
            process_kind: commit.state.process_kind,
            scope: commit.state.scope,
            occurred_at: Some(Utc::now()),
            owner_user_id: commit.state.owner_user_id,
            status: commit.state.status,
            kind: commit.kind,
            suspension: commit.state.suspension,
            sanitized_reason: commit.sanitized_reason,
            retryable: None,
            detail: None,
            metadata: commit.state.metadata,
            committed_state: None,
        })
        .map_err(|error| error.to_string())?;
        if self.required.observes_event(&event) {
            self.required
                .observe_committed_event(event.clone())
                .await
                .map_err(|error| error.to_string())?;
        }
        if let Some(sink) = &self.best_effort
            && let Err(error) = sink.publish(event).await
        {
            tracing::debug!(%error, "turn projection sink rejected process commit");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct AgentTurnProcessRuntime {
    submission: Arc<dyn ProcessSubmissionPort<Error = TurnError>>,
    journal: Arc<dyn ProcessJournalSource<Error = TurnError>>,
    controls: Arc<dyn ProcessControlPort<Error = TurnError>>,
    trees: Arc<dyn ProcessTreePort<Error = TurnError>>,
    snapshots: Arc<dyn ProcessSnapshotSource<Error = TurnError>>,
    checkpoints: Arc<dyn ProcessCheckpointPort<Error = TurnError>>,
}

impl AgentTurnProcessRuntime {
    pub fn from_process_runtime(runtime: Arc<dyn ProcessRuntimePort>) -> Self {
        Self::from_process_adapter(Arc::new(ProcessJournalStoreTurnAdapter::new(runtime)))
    }

    pub fn from_process_adapter(adapter: Arc<ProcessJournalStoreTurnAdapter>) -> Self {
        Self::new(
            Arc::clone(&adapter) as Arc<dyn ProcessSubmissionPort<Error = TurnError>>,
            Arc::clone(&adapter) as Arc<dyn ProcessJournalSource<Error = TurnError>>,
            Arc::clone(&adapter) as Arc<dyn ProcessControlPort<Error = TurnError>>,
            Arc::clone(&adapter) as Arc<dyn ProcessTreePort<Error = TurnError>>,
            Arc::clone(&adapter) as Arc<dyn ProcessSnapshotSource<Error = TurnError>>,
            adapter as Arc<dyn ProcessCheckpointPort<Error = TurnError>>,
        )
    }

    pub fn new(
        submission: Arc<dyn ProcessSubmissionPort<Error = TurnError>>,
        journal: Arc<dyn ProcessJournalSource<Error = TurnError>>,
        controls: Arc<dyn ProcessControlPort<Error = TurnError>>,
        trees: Arc<dyn ProcessTreePort<Error = TurnError>>,
        snapshots: Arc<dyn ProcessSnapshotSource<Error = TurnError>>,
        checkpoints: Arc<dyn ProcessCheckpointPort<Error = TurnError>>,
    ) -> Self {
        Self {
            submission,
            journal,
            controls,
            trees,
            snapshots,
            checkpoints,
        }
    }

    pub async fn get_run_state(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<TurnRunState, TurnError> {
        let snapshot = self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(run_id),
            })
            .await?;
        turn_run_state_from_process_snapshot(snapshot)
    }

    pub async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        admission_policy
            .check_submit(&request)
            .map_err(TurnError::AdmissionRejected)?;
        let resolved = run_profile_resolver
            .resolve_run_profile(RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..RunProfileResolutionRequest::interactive_default()
            })
            .await
            .map_err(profile_resolution_error_to_turn_error)?;
        let profile = TurnRunProfile::from_resolved(resolved.clone());
        let run_id = request.requested_run_id.unwrap_or_default();
        let turn_id = TurnId::new();
        let metadata = AgentTurnProcessStateMetadata {
            turn_id,
            actor: Some(request.actor.clone()),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            resolved_run_profile_id: profile.id.clone(),
            resolved_run_profile_version: profile.version,
            resolved_run_profile: Some(resolved),
            resolved_model_route: request
                .requested_model
                .as_deref()
                .and_then(LoopModelRouteSnapshot::advisory),
            model_usage: None,
            subagent_depth: 0,
            spawn_tree_descendant_cap: None,
            product_context: request.product_context,
            resume_disposition: None,
        };
        let concurrency_class = process_concurrency_class(metadata.product_context.as_ref());
        let snapshot = self
            .submission
            .submit_process(SubmitProcessRequest {
                process_id: process_id_from_turn_run_id(run_id),
                process_kind: ProcessKind::AgentTurn,
                scope: request.scope.to_resource_scope(),
                exclusive_within_scope: true,
                operation_id: Some(ProcessOperationId::from_trusted(
                    request.idempotency_key.as_str(),
                )),
                owner_user_id: Some(request.actor.user_id),
                concurrency_class,
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: request.received_at,
                metadata: json!({ "agent_turn": metadata }),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(snapshot)?;
        Ok(SubmitTurnResponse::Accepted {
            turn_id: state.turn_id,
            run_id: state.run_id,
            status: state.status,
            resolved_run_profile_id: state.resolved_run_profile_id,
            resolved_run_profile_version: state.resolved_run_profile_version,
            event_cursor: state.event_cursor,
            accepted_message_ref: state.accepted_message_ref,
            reply_target_binding_ref: state.reply_target_binding_ref,
        })
    }

    pub async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let parent = self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: request.parent_scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(request.parent_run_id),
            })
            .await?;
        let parent_metadata = agent_turn_metadata_from_process_snapshot(&parent)?;
        let subagent_depth = parent_metadata
            .subagent_depth
            .checked_add(1)
            .ok_or_else(|| TurnError::InvalidRequest {
                reason: "subagent depth would overflow".to_string(),
            })?;
        let root_process_id = parent.root_process_id.unwrap_or(parent.process_id);
        let submit_template = SubmitTurnRequest {
            requested_model: None,
            scope: request.child_scope.clone(),
            actor: request.actor.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            requested_run_profile: request.requested_run_profile.clone(),
            idempotency_key: request.idempotency_key.clone(),
            received_at: request.received_at,
            requested_run_id: request.requested_run_id,
            parent_run_id: Some(request.parent_run_id),
            subagent_depth,
            spawn_tree_root_run_id: Some(turn_run_id_from_process_id(root_process_id)),
            product_context: parent_metadata.product_context.clone(),
        };
        admission_policy
            .check_submit(&submit_template)
            .map_err(TurnError::AdmissionRejected)?;
        let resolved = run_profile_resolver
            .resolve_run_profile(RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..RunProfileResolutionRequest::interactive_default()
            })
            .await
            .map_err(profile_resolution_error_to_turn_error)?;
        let profile = TurnRunProfile::from_resolved(resolved.clone());
        let run_id = request.requested_run_id.unwrap_or_default();
        let metadata = AgentTurnProcessStateMetadata {
            turn_id: TurnId::new(),
            actor: Some(request.actor.clone()),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            resolved_run_profile_id: profile.id.clone(),
            resolved_run_profile_version: profile.version,
            resolved_run_profile: Some(resolved),
            resolved_model_route: None,
            model_usage: None,
            subagent_depth,
            spawn_tree_descendant_cap: Some(request.spawn_tree_descendant_cap),
            product_context: parent_metadata.product_context,
            resume_disposition: None,
        };
        let concurrency_class = process_concurrency_class(metadata.product_context.as_ref());
        let snapshot = self
            .submission
            .submit_process(SubmitProcessRequest {
                process_id: process_id_from_turn_run_id(run_id),
                process_kind: ProcessKind::AgentTurn,
                scope: request.child_scope.to_resource_scope(),
                exclusive_within_scope: true,
                operation_id: Some(ProcessOperationId::from_trusted(format!(
                    "child:{}",
                    request.idempotency_key.as_str()
                ))),
                owner_user_id: Some(request.actor.user_id.clone()),
                concurrency_class,
                parent_process_id: Some(parent.process_id),
                root_process_id: Some(root_process_id),
                spawn_tree_descendant_cap: Some(request.spawn_tree_descendant_cap),
                dependency: request.process_dependency,
                checkpoint_ref: None,
                input: request.process_input,
                created_at: request.received_at,
                metadata: json!({ "agent_turn": metadata }),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(snapshot)?;
        Ok(SubmitTurnResponse::Accepted {
            turn_id: state.turn_id,
            run_id: state.run_id,
            status: state.status,
            resolved_run_profile_id: state.resolved_run_profile_id,
            resolved_run_profile_version: state.resolved_run_profile_version,
            event_cursor: state.event_cursor,
            accepted_message_ref: state.accepted_message_ref,
            reply_target_binding_ref: state.reply_target_binding_ref,
        })
    }

    pub async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let snapshot = self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: request.scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(request.run_id),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(snapshot.clone())?;
        if state.actor.as_ref() != Some(&request.actor) {
            return Err(TurnError::Unauthorized);
        }
        if snapshot.status == ProcessLifecycleStatus::Suspended {
            let resumable = request.precondition.required_status().map_or_else(
                || {
                    matches!(
                        state.status,
                        TurnStatus::BlockedApproval
                            | TurnStatus::BlockedAuth
                            | TurnStatus::BlockedResource
                    )
                },
                |required| state.status == required,
            );
            if !resumable {
                return Err(TurnError::InvalidTransition {
                    from: state.status,
                    to: TurnStatus::Queued,
                });
            }
            if state.gate_ref.as_ref() != Some(&request.gate_resolution_ref) {
                return Err(TurnError::InvalidRequest {
                    reason: "gate resolution reference mismatch".to_string(),
                });
            }
        }
        let metadata = resumed_agent_turn_metadata(&snapshot, &request)?;
        let result = self
            .controls
            .resume_process(ResumeProcessRequest {
                scope: request.scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(request.run_id),
                operation_id: Some(ProcessOperationId::from_trusted(
                    request.idempotency_key.as_str(),
                )),
                expected_cursor: Some(snapshot.journal_cursor),
                checkpoint_ref: snapshot.checkpoint_ref,
                metadata: Some(metadata),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(result.state)?;
        Ok(ResumeTurnResponse {
            run_id: state.run_id,
            status: state.status,
            event_cursor: state.event_cursor,
        })
    }

    pub async fn cancel_run(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let snapshot = self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: request.scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(request.run_id),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(snapshot)?;
        if state.actor.as_ref() != Some(&request.actor) {
            return Err(TurnError::Unauthorized);
        }
        let result = self
            .controls
            .request_cancel_process(CancelProcessRequest {
                scope: request.scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(request.run_id),
                operation_id: Some(ProcessOperationId::from_trusted(
                    request.idempotency_key.as_str(),
                )),
                reason: Some(request.reason.category().to_string()),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(result.state)?;
        Ok(CancelRunResponse {
            run_id: state.run_id,
            status: state.status,
            event_cursor: state.event_cursor,
            already_terminal: result.already_terminal,
            actor: state.actor,
        })
    }

    pub async fn retry_turn(
        &self,
        request: RetryTurnRequest,
    ) -> Result<RetryTurnResponse, TurnError> {
        let resource_scope = request.scope.to_resource_scope();
        let snapshot = self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: resource_scope.clone(),
                process_id: process_id_from_turn_run_id(request.run_id),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(snapshot.clone())?;
        if state.actor.as_ref() != Some(&request.actor) {
            return Err(TurnError::Unauthorized);
        }
        if !matches!(
            state.status,
            TurnStatus::Failed | TurnStatus::RecoveryRequired
        ) {
            return Err(TurnError::RunNotRetryable {
                run_id: request.run_id,
            });
        }
        let mut metadata = agent_turn_metadata_from_process_snapshot(&snapshot)?;
        let latest = self
            .snapshots
            .process_snapshots(&resource_scope)
            .await?
            .into_iter()
            .filter(|candidate| candidate.process_kind == ProcessKind::AgentTurn)
            .filter_map(|candidate| {
                let candidate_metadata =
                    agent_turn_metadata_from_process_snapshot(&candidate).ok()?;
                (candidate_metadata.turn_id == metadata.turn_id).then_some(candidate)
            })
            .max_by_key(|candidate| candidate.journal_cursor.0);
        if latest
            .as_ref()
            .is_some_and(|latest| latest.process_id != snapshot.process_id)
        {
            return Err(TurnError::RunNotRetryable {
                run_id: request.run_id,
            });
        }
        metadata.source_binding_ref = request.source_binding_ref;
        metadata.reply_target_binding_ref = request.reply_target_binding_ref;
        metadata.model_usage = None;
        metadata.resume_disposition = None;
        let concurrency_class = process_concurrency_class(metadata.product_context.as_ref());
        let run_id = TurnRunId::new();
        let retry_process_id = process_id_from_turn_run_id(run_id);
        let initial_checkpoint = if let Some(source_ref) = snapshot.checkpoint_ref.as_ref() {
            let source = self
                .checkpoints
                .get_process_checkpoint(GetProcessCheckpointRequest {
                    checkpoint_id: ProcessCheckpointId::from_trusted(source_ref.as_str()),
                    process_id: snapshot.process_id,
                    scope: resource_scope.clone(),
                })
                .await?
                .ok_or(TurnError::RunNotRetryable {
                    run_id: request.run_id,
                })?;
            let checkpoint_id = TurnCheckpointId::new();
            let checkpoint_ref = process_checkpoint_ref(checkpoint_id);
            Some((
                checkpoint_ref.clone(),
                RecordProcessCheckpointRequest {
                    checkpoint_id: ProcessCheckpointId::from_trusted(
                        checkpoint_ref.as_str().to_string(),
                    ),
                    process_id: retry_process_id,
                    scope: resource_scope.clone(),
                    state_ref: source.state_ref,
                    payload: source.payload,
                    created_at: Utc::now(),
                    metadata: source.metadata,
                },
            ))
        } else {
            None
        };
        let checkpoint_ref = initial_checkpoint
            .as_ref()
            .map(|(checkpoint_ref, _)| checkpoint_ref.clone());
        let retried = self
            .submission
            .submit_process_with_checkpoint(SubmitProcessWithCheckpointRequest {
                submission: SubmitProcessRequest {
                    process_id: retry_process_id,
                    process_kind: ProcessKind::AgentTurn,
                    scope: resource_scope,
                    exclusive_within_scope: true,
                    operation_id: Some(ProcessOperationId::from_trusted(format!(
                        "retry:{}",
                        request.idempotency_key.as_str()
                    ))),
                    owner_user_id: snapshot.owner_user_id,
                    concurrency_class,
                    parent_process_id: snapshot.parent_process_id,
                    root_process_id: snapshot.root_process_id,
                    spawn_tree_descendant_cap: metadata.spawn_tree_descendant_cap,
                    dependency: None,
                    checkpoint_ref,
                    input: None,
                    created_at: Utc::now(),
                    metadata: json!({ "agent_turn": metadata }),
                },
                checkpoint: initial_checkpoint.map(|(_, checkpoint)| checkpoint),
            })
            .await?;
        let state = turn_run_state_from_process_snapshot(retried)?;
        Ok(RetryTurnResponse {
            run_id: state.run_id,
            status: state.status,
            event_cursor: state.event_cursor,
        })
    }
}

#[async_trait]
impl crate::AgentTurnRuntimePort for AgentTurnProcessRuntime {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        AgentTurnProcessRuntime::submit_turn(self, request, admission_policy, run_profile_resolver)
            .await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        AgentTurnProcessRuntime::resume_turn(self, request).await
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        AgentTurnProcessRuntime::retry_turn(self, request).await
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        AgentTurnProcessRuntime::cancel_run(self, request).await
    }

    async fn get_run_state(
        &self,
        request: crate::GetRunStateRequest,
    ) -> Result<TurnRunState, TurnError> {
        AgentTurnProcessRuntime::get_run_state(self, &request.scope, request.run_id).await
    }
}

#[async_trait]
impl crate::AgentTurnSpawnTreeRuntimePort for AgentTurnProcessRuntime {
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        AgentTurnProcessRuntime::submit_child_turn(
            self,
            request,
            admission_policy,
            run_profile_resolver,
        )
        .await
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        self.trees
            .child_processes(
                &scope.to_resource_scope(),
                process_id_from_turn_run_id(run_id),
            )
            .await?
            .into_iter()
            .map(turn_run_record_from_process_snapshot)
            .collect()
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        match self
            .journal
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope.to_resource_scope(),
                process_id: process_id_from_turn_run_id(run_id),
            })
            .await
        {
            Ok(snapshot) => turn_run_record_from_process_snapshot(snapshot).map(Some),
            Err(TurnError::ScopeNotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        let reservation = self
            .trees
            .reserve_process_tree(ReserveProcessTreeRequest {
                scope: scope.to_resource_scope(),
                root_process_id: process_id_from_turn_run_id(root_run_id),
                delta,
                cap,
            })
            .await?;
        Ok(SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id,
            descendant_count: reservation.descendant_count,
            released_children: reservation
                .released_processes
                .into_iter()
                .map(turn_run_id_from_process_id)
                .collect(),
        })
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        self.trees
            .release_process_tree(ReleaseProcessTreeRequest {
                scope: scope.to_resource_scope(),
                root_process_id: process_id_from_turn_run_id(root_run_id),
                delta,
                idempotency_process_id: process_id_from_turn_run_id(idempotency_key),
            })
            .await
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.trees
            .prune_released_process(PruneReleasedProcessRequest {
                scope: scope.to_resource_scope(),
                root_process_id: process_id_from_turn_run_id(root_run_id),
                process_id: process_id_from_turn_run_id(child_run_id),
            })
            .await
    }
}

fn profile_resolution_error_to_turn_error(error: RunProfileResolutionError) -> TurnError {
    let reason = match error {
        RunProfileResolutionError::Unauthorized { .. } => AdmissionRejectionReason::Unauthorized,
        RunProfileResolutionError::ProfileUnavailable { .. }
        | RunProfileResolutionError::InvalidRequest { .. } => {
            AdmissionRejectionReason::ProfileRejected
        }
    };
    TurnError::AdmissionRejected(AdmissionRejection::new(reason))
}

fn resumed_agent_turn_metadata(
    snapshot: &JournaledProcessSnapshot,
    request: &ResumeTurnRequest,
) -> Result<Value, TurnError> {
    let mut metadata = agent_turn_metadata_from_process_snapshot(snapshot)?;
    metadata.source_binding_ref = request.source_binding_ref.clone();
    metadata.reply_target_binding_ref = request.reply_target_binding_ref.clone();
    metadata.resume_disposition = request.resume_disposition.clone();
    Ok(json!({ "agent_turn": metadata }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessMetadata {
    pub turn_id: crate::TurnId,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<GateResumeDisposition>,
}

impl AgentTurnProcessMetadata {
    fn from_record(record: &TurnRunRecord) -> Self {
        Self {
            turn_id: record.turn_id,
            accepted_message_ref: record.accepted_message_ref.clone(),
            source_binding_ref: record.source_binding_ref.clone(),
            reply_target_binding_ref: record.reply_target_binding_ref.clone(),
            resolved_run_profile_id: record.profile.id.clone(),
            resolved_run_profile_version: record.profile.version,
            resolved_model_route: record.resolved_model_route.clone(),
            model_usage: record.model_usage,
            subagent_depth: record.subagent_depth,
            product_context: record.product_context.clone(),
            resume_disposition: record.resume_disposition.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessStateMetadata {
    pub turn_id: crate::TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_run_profile: Option<ResolvedRunProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_descendant_cap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<GateResumeDisposition>,
}

impl AgentTurnProcessStateMetadata {
    fn from_state(state: &TurnRunState) -> Self {
        Self {
            turn_id: state.turn_id,
            actor: state.actor.clone(),
            accepted_message_ref: state.accepted_message_ref.clone(),
            source_binding_ref: state.source_binding_ref.clone(),
            reply_target_binding_ref: state.reply_target_binding_ref.clone(),
            resolved_run_profile_id: state.resolved_run_profile_id.clone(),
            resolved_run_profile_version: state.resolved_run_profile_version,
            resolved_run_profile: None,
            resolved_model_route: state.resolved_model_route.clone(),
            model_usage: state.model_usage,
            subagent_depth: 0,
            spawn_tree_descendant_cap: None,
            product_context: state.product_context.clone(),
            resume_disposition: state.resume_disposition.clone(),
        }
    }

    fn from_claimed(claimed: &ClaimedTurnRun) -> Self {
        Self {
            resolved_run_profile: Some(claimed.resolved_run_profile.clone()),
            ..Self::from_state(&claimed.state)
        }
    }
}

pub(crate) fn agent_turn_metadata_from_claimed(
    claimed: &ClaimedTurnRun,
    model_usage: Option<LoopModelUsage>,
) -> Value {
    let mut metadata = AgentTurnProcessStateMetadata::from_claimed(claimed);
    if model_usage.is_some() {
        metadata.model_usage = model_usage;
    }
    json!({ "agent_turn": metadata })
}

pub trait TurnRunProcessExt {
    fn to_process_snapshot(&self) -> JournaledProcessSnapshot;
}

impl TurnRunProcessExt for TurnRunRecord {
    fn to_process_snapshot(&self) -> JournaledProcessSnapshot {
        JournaledProcessSnapshot {
            process_id: process_id_from_turn_run_id(self.run_id),
            process_kind: ProcessKind::AgentTurn,
            scope: self.scope.to_resource_scope(),
            status: process_status_from_turn_status(self.status),
            suspension: process_suspension_from_record(self),
            checkpoint_ref: self.checkpoint_id.map(process_checkpoint_ref),
            input_ref: None,
            failure: self.failure.clone(),
            journal_cursor: ProcessJournalCursor(self.event_cursor.0),
            lease: process_lease_from_record(self),
            crash_reclaim_count: 0,
            created_at: self.received_at,
            owner_user_id: self.scope.explicit_owner_user_id().cloned(),
            concurrency_class: process_concurrency_class(self.product_context.as_ref()),
            parent_process_id: self.parent_run_id.map(process_id_from_turn_run_id),
            root_process_id: self.spawn_tree_root_run_id.map(process_id_from_turn_run_id),
            metadata: json!({ "agent_turn": AgentTurnProcessMetadata::from_record(self) }),
        }
    }
}

pub trait TurnRunStateProcessExt {
    fn to_process_state_snapshot(&self) -> JournaledProcessSnapshot;
}

impl TurnRunStateProcessExt for TurnRunState {
    fn to_process_state_snapshot(&self) -> JournaledProcessSnapshot {
        JournaledProcessSnapshot {
            process_id: process_id_from_turn_run_id(self.run_id),
            process_kind: ProcessKind::AgentTurn,
            scope: self.scope.to_resource_scope(),
            status: process_status_from_turn_status(self.status),
            suspension: process_suspension_from_state(self),
            checkpoint_ref: self.checkpoint_id.map(process_checkpoint_ref),
            input_ref: None,
            failure: self.failure.clone(),
            journal_cursor: ProcessJournalCursor(self.event_cursor.0),
            lease: None,
            crash_reclaim_count: 0,
            created_at: self.received_at,
            owner_user_id: self
                .scope
                .explicit_owner_user_id()
                .cloned()
                .or_else(|| self.actor.as_ref().map(|actor| actor.user_id.clone())),
            concurrency_class: process_concurrency_class(self.product_context.as_ref()),
            parent_process_id: None,
            root_process_id: None,
            metadata: json!({ "agent_turn": AgentTurnProcessStateMetadata::from_state(self) }),
        }
    }
}

pub trait TurnLifecycleProcessExt {
    fn to_process_journal_entry(&self) -> ProcessJournalEntry;
}

impl TurnLifecycleProcessExt for TurnLifecycleEvent {
    fn to_process_journal_entry(&self) -> ProcessJournalEntry {
        ProcessJournalEntry {
            cursor: ProcessJournalCursor(self.cursor.0),
            process_id: process_id_from_turn_run_id(self.run_id),
            process_kind: ProcessKind::AgentTurn,
            scope: self.scope.to_resource_scope(),
            occurred_at: self.occurred_at,
            owner_user_id: self.owner_user_id.clone(),
            status: process_status_from_turn_status(self.status),
            kind: process_journal_kind_from_turn_event_kind(self.kind.clone()),
            suspension: process_suspension_from_event(self),
            sanitized_reason: self.sanitized_reason.clone(),
            retryable: self.retryable,
            detail: self.detail.clone(),
            metadata: Value::Null,
            committed_state: None,
        }
    }
}

pub fn process_id_from_turn_run_id(run_id: TurnRunId) -> ProcessId {
    ProcessId::from_uuid(run_id.as_uuid())
}

fn process_checkpoint_ref(checkpoint_id: TurnCheckpointId) -> ProcessCheckpointRef {
    ProcessCheckpointRef::from_trusted(checkpoint_id.as_uuid().to_string())
}

fn process_concurrency_class(
    product_context: Option<&ProductTurnContext>,
) -> Option<ProcessConcurrencyClass> {
    match product_context?.origin {
        TurnOriginKind::ScheduledTrigger => {
            Some(ProcessConcurrencyClass::from_trusted("scheduled_trigger"))
        }
        TurnOriginKind::WebUi | TurnOriginKind::Inbound => {
            Some(ProcessConcurrencyClass::from_trusted("conversation"))
        }
    }
}

pub fn process_status_from_turn_status(status: TurnStatus) -> ProcessLifecycleStatus {
    match status {
        TurnStatus::Queued => ProcessLifecycleStatus::Queued,
        TurnStatus::Running => ProcessLifecycleStatus::Running,
        TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool => ProcessLifecycleStatus::Suspended,
        TurnStatus::CancelRequested => ProcessLifecycleStatus::CancelRequested,
        TurnStatus::Cancelled => ProcessLifecycleStatus::Cancelled,
        TurnStatus::Completed => ProcessLifecycleStatus::Completed,
        TurnStatus::Failed => ProcessLifecycleStatus::Failed,
        TurnStatus::RecoveryRequired => ProcessLifecycleStatus::RecoveryRequired,
    }
}

pub fn process_suspension_kind_from_gate_kind(kind: GateKind) -> ProcessSuspensionKind {
    match kind {
        GateKind::Approval => ProcessSuspensionKind::Approval,
        GateKind::Auth => ProcessSuspensionKind::Authorization,
        GateKind::Resource => ProcessSuspensionKind::Resource,
        GateKind::AwaitDependentRun => ProcessSuspensionKind::AwaitingChildProcess,
        GateKind::ExternalTool => ProcessSuspensionKind::ExternalTool,
    }
}

pub fn process_suspension_kind_from_turn_blocked_gate_kind(
    kind: TurnBlockedGateKind,
) -> ProcessSuspensionKind {
    match kind {
        TurnBlockedGateKind::Approval => ProcessSuspensionKind::Approval,
        TurnBlockedGateKind::Auth => ProcessSuspensionKind::Authorization,
        TurnBlockedGateKind::Resource => ProcessSuspensionKind::Resource,
        TurnBlockedGateKind::AwaitDependentRun => ProcessSuspensionKind::AwaitingChildProcess,
        TurnBlockedGateKind::ExternalTool => ProcessSuspensionKind::ExternalTool,
    }
}

pub fn process_journal_kind_from_turn_event_kind(kind: TurnEventKind) -> ProcessJournalKind {
    match kind {
        TurnEventKind::Submitted => ProcessJournalKind::Submitted,
        TurnEventKind::Resumed => ProcessJournalKind::Resumed,
        TurnEventKind::RunnerClaimed => ProcessJournalKind::Claimed,
        TurnEventKind::RunnerHeartbeat => ProcessJournalKind::Heartbeat,
        TurnEventKind::RecoveryRequired => ProcessJournalKind::RecoveryRequired,
        TurnEventKind::Blocked => ProcessJournalKind::Suspended,
        TurnEventKind::CancelRequested => ProcessJournalKind::CancelRequested,
        TurnEventKind::Cancelled => ProcessJournalKind::Cancelled,
        TurnEventKind::Completed => ProcessJournalKind::Completed,
        TurnEventKind::Failed => ProcessJournalKind::Failed,
    }
}

pub(crate) fn process_suspension_from_record(record: &TurnRunRecord) -> Option<ProcessSuspension> {
    let kind = GateKind::from_status(record.status).map(process_suspension_kind_from_gate_kind)?;
    Some(ProcessSuspension {
        kind,
        gate_ref: record.gate_ref.clone(),
        activity_id: record.blocked_activity_id,
        credential_requirements: record.credential_requirements.clone(),
        detail: None,
    })
}

fn process_suspension_from_state(state: &TurnRunState) -> Option<ProcessSuspension> {
    let kind = GateKind::from_status(state.status).map(process_suspension_kind_from_gate_kind)?;
    Some(ProcessSuspension {
        kind,
        gate_ref: state.gate_ref.clone(),
        activity_id: state.blocked_activity_id,
        credential_requirements: state.credential_requirements.clone(),
        detail: None,
    })
}

fn process_suspension_from_event(event: &TurnLifecycleEvent) -> Option<ProcessSuspension> {
    let gate = event.blocked_gate.as_ref()?;
    Some(ProcessSuspension {
        kind: process_suspension_kind_from_turn_blocked_gate_kind(gate.gate_kind),
        gate_ref: Some(gate.gate_ref.clone()),
        activity_id: gate.activity_id,
        credential_requirements: gate.credential_requirements.clone(),
        detail: None,
    })
}

fn process_lease_from_record(record: &TurnRunRecord) -> Option<ProcessLeaseSnapshot> {
    Some(ProcessLeaseSnapshot {
        worker_id: ProcessWorkerId::from_trusted(record.runner_id?.to_wire_string()),
        lease_token: ProcessLeaseToken::from_trusted(record.lease_token?.to_wire_string()),
        lease_expires_at: record.lease_expires_at,
        last_heartbeat_at: record.last_heartbeat_at,
        claim_count: record.claim_count,
    })
}

trait TurnUuidWire {
    fn to_wire_string(self) -> String;
}

impl TurnUuidWire for TurnRunnerId {
    fn to_wire_string(self) -> String {
        self.as_uuid().to_string()
    }
}

impl TurnUuidWire for crate::TurnLeaseToken {
    fn to_wire_string(self) -> String {
        self.as_uuid().to_string()
    }
}

fn turn_runner_id_from_worker(worker_id: &ProcessWorkerId) -> Result<TurnRunnerId, TurnError> {
    Uuid::parse_str(worker_id.as_str())
        .map(TurnRunnerId::from_uuid)
        .map_err(|error| TurnError::InvalidRequest {
            reason: format!("invalid process worker id: {error}"),
        })
}

fn turn_lease_token_from_process(
    lease_token: &ProcessLeaseToken,
) -> Result<crate::TurnLeaseToken, TurnError> {
    Uuid::parse_str(lease_token.as_str())
        .map(crate::TurnLeaseToken::from_uuid)
        .map_err(|error| TurnError::InvalidRequest {
            reason: format!("invalid process lease token: {error}"),
        })
}

fn turn_run_id_from_process_id(process_id: ProcessId) -> TurnRunId {
    TurnRunId::from_uuid(process_id.as_uuid())
}

fn turn_checkpoint_id_from_process_ref(
    checkpoint_ref: ProcessCheckpointRef,
) -> Result<TurnCheckpointId, TurnError> {
    Uuid::parse_str(checkpoint_ref.as_str())
        .map(TurnCheckpointId::from_uuid)
        .map_err(|error| TurnError::InvalidRequest {
            reason: format!("invalid process checkpoint ref: {error}"),
        })
}

fn agent_turn_metadata_from_process_snapshot(
    snapshot: &JournaledProcessSnapshot,
) -> Result<AgentTurnProcessStateMetadata, TurnError> {
    let Some(metadata) = snapshot.metadata.get("agent_turn") else {
        return Err(TurnError::InvalidRequest {
            reason: "agent-turn process snapshot missing agent_turn metadata".to_string(),
        });
    };
    serde_json::from_value(metadata.clone()).map_err(|error| TurnError::InvalidRequest {
        reason: format!("invalid agent-turn process metadata: {error}"),
    })
}

fn turn_run_record_from_process_snapshot(
    snapshot: JournaledProcessSnapshot,
) -> Result<TurnRunRecord, TurnError> {
    let metadata = agent_turn_metadata_from_process_snapshot(&snapshot)?;
    let resolved =
        metadata
            .resolved_run_profile
            .clone()
            .ok_or_else(|| TurnError::InvalidRequest {
                reason: "agent-turn process record missing resolved_run_profile metadata"
                    .to_string(),
            })?;
    let mut profile = TurnRunProfile::from_resolved(resolved);
    profile.id = metadata.resolved_run_profile_id.clone();
    profile.version = metadata.resolved_run_profile_version;
    let lease = snapshot.lease.clone();
    let state = turn_run_state_from_process_snapshot(snapshot.clone())?;
    Ok(TurnRunRecord {
        run_id: state.run_id,
        turn_id: state.turn_id,
        scope: state.scope,
        accepted_message_ref: state.accepted_message_ref,
        source_binding_ref: state.source_binding_ref,
        reply_target_binding_ref: state.reply_target_binding_ref,
        status: state.status,
        profile,
        resolved_model_route: state.resolved_model_route,
        model_usage: state.model_usage,
        checkpoint_id: state.checkpoint_id,
        gate_ref: state.gate_ref,
        blocked_activity_id: state.blocked_activity_id,
        credential_requirements: state.credential_requirements,
        failure: state.failure,
        event_cursor: state.event_cursor,
        runner_id: lease
            .as_ref()
            .map(|lease| turn_runner_id_from_worker(&lease.worker_id))
            .transpose()?,
        lease_token: lease
            .as_ref()
            .map(|lease| turn_lease_token_from_process(&lease.lease_token))
            .transpose()?,
        lease_expires_at: lease.as_ref().and_then(|lease| lease.lease_expires_at),
        last_heartbeat_at: lease.as_ref().and_then(|lease| lease.last_heartbeat_at),
        claim_count: lease.as_ref().map(|lease| lease.claim_count).unwrap_or(0),
        received_at: state.received_at,
        parent_run_id: snapshot.parent_process_id.map(turn_run_id_from_process_id),
        subagent_depth: metadata.subagent_depth,
        spawn_tree_root_run_id: snapshot.root_process_id.map(turn_run_id_from_process_id),
        product_context: state.product_context,
        resume_disposition: state.resume_disposition,
    })
}

pub fn turn_run_state_from_process_snapshot(
    snapshot: JournaledProcessSnapshot,
) -> Result<TurnRunState, TurnError> {
    if snapshot.process_kind != ProcessKind::AgentTurn {
        return Err(TurnError::InvalidRequest {
            reason: "process snapshot is not an agent turn".to_string(),
        });
    }
    let metadata = agent_turn_metadata_from_process_snapshot(&snapshot)?;
    let status = turn_status_from_process_status(snapshot.status, snapshot.suspension.as_ref())?;
    Ok(TurnRunState {
        scope: turn_scope_from_process_scope(snapshot.scope)?,
        actor: metadata.actor,
        turn_id: metadata.turn_id,
        run_id: turn_run_id_from_process_id(snapshot.process_id),
        status,
        accepted_message_ref: metadata.accepted_message_ref,
        source_binding_ref: metadata.source_binding_ref,
        reply_target_binding_ref: metadata.reply_target_binding_ref,
        resolved_run_profile_id: metadata.resolved_run_profile_id,
        resolved_run_profile_version: metadata.resolved_run_profile_version,
        resolved_model_route: metadata.resolved_model_route,
        model_usage: metadata.model_usage,
        received_at: snapshot.created_at,
        checkpoint_id: snapshot
            .checkpoint_ref
            .map(turn_checkpoint_id_from_process_ref)
            .transpose()?,
        gate_ref: snapshot
            .suspension
            .as_ref()
            .and_then(|suspension| suspension.gate_ref.clone()),
        blocked_activity_id: snapshot
            .suspension
            .as_ref()
            .and_then(|suspension| suspension.activity_id),
        credential_requirements: snapshot
            .suspension
            .map(|suspension| suspension.credential_requirements)
            .unwrap_or_default(),
        failure: snapshot.failure,
        event_cursor: EventCursor(snapshot.journal_cursor.0),
        product_context: metadata.product_context,
        resume_disposition: metadata.resume_disposition,
    })
}

pub fn claimed_turn_run_from_process_claim(
    claimed: ClaimedProcess,
) -> Result<ClaimedTurnRun, TurnError> {
    let metadata = agent_turn_metadata_from_process_snapshot(&claimed.state)?;
    let Some(resolved_run_profile) = metadata.resolved_run_profile else {
        return Err(TurnError::InvalidRequest {
            reason: "claimed agent-turn process missing resolved_run_profile metadata".to_string(),
        });
    };
    let state = turn_run_state_from_process_snapshot(claimed.state)?;
    Ok(ClaimedTurnRun {
        state,
        resolved_run_profile,
        runner_id: turn_runner_id_from_worker(&claimed.worker_id)?,
        lease_token: turn_lease_token_from_process(&claimed.lease_token)?,
    })
}

impl From<&ClaimedTurnRun> for ClaimedProcess {
    fn from(claimed: &ClaimedTurnRun) -> Self {
        let mut state = claimed.state.to_process_state_snapshot();
        state.metadata =
            json!({ "agent_turn": AgentTurnProcessStateMetadata::from_claimed(claimed) });
        Self {
            state,
            worker_id: ProcessWorkerId::from_trusted(claimed.runner_id.to_wire_string()),
            lease_token: ProcessLeaseToken::from_trusted(claimed.lease_token.to_wire_string()),
        }
    }
}

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
        owner_user_id: Option<&ironclaw_host_api::UserId>,
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

fn turn_scope_from_process_scope(scope: ResourceScope) -> Result<TurnScope, TurnError> {
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
        run_id: turn_run_id_from_process_id(entry.process_id),
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

pub fn process_outcome_from_turn_runner_outcome(outcome: TurnRunnerOutcome) -> ProcessOutcome {
    match outcome {
        TurnRunnerOutcome::Completed => ProcessOutcome::Completed,
        TurnRunnerOutcome::Cancelled => ProcessOutcome::Cancelled,
        TurnRunnerOutcome::Blocked {
            checkpoint_id,
            reason,
            ..
        } => ProcessOutcome::Suspended {
            checkpoint_ref: process_checkpoint_ref(checkpoint_id),
            suspension: process_suspension_from_blocked_reason(reason),
        },
        TurnRunnerOutcome::Failed { failure } => ProcessOutcome::Failed { failure },
    }
}

fn process_suspension_from_blocked_reason(reason: BlockedReason) -> ProcessSuspension {
    let kind = process_suspension_kind_from_gate_kind(reason.gate_kind());
    ProcessSuspension {
        kind,
        gate_ref: Some(reason.gate_ref().clone()),
        activity_id: None,
        credential_requirements: reason.credential_requirements().to_vec(),
        detail: None,
    }
}

#[cfg(test)]
mod tests;
