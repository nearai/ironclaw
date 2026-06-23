use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::sync::Mutex;

use crate::{
    AcceptStageResultInput, AcceptStageResultOutcome, AdvanceWorkflowRunInput,
    BlockWorkflowRunInput, BlockWorkflowRunOutcome, ClaimProviderActionInput,
    ClaimProviderActionOutcome, ClaimRunnableWorkflowRunsInput, CompleteProviderActionInput,
    CompleteProviderActionOutcome, CompleteWorkflowStepInput, CompleteWorkflowStepOutcome,
    CreateOrGetProviderActionInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
    CreateOrGetWorkflowStepInput, CreateOrGetWorkflowStepOutcome, CreateStageRunInput,
    CreateStageRunOutcome, FindLatestWorkflowEventForProviderInput, GithubIssueProviderActionId,
    GithubIssueProviderActionRecord, GithubIssueProviderBinding, GithubIssueProviderBindingId,
    GithubIssueStageRunId, GithubIssueWorkflowError, GithubIssueWorkflowEvent,
    GithubIssueWorkflowEventId, GithubIssueWorkflowMode, GithubIssueWorkflowRepository,
    GithubIssueWorkflowRun, GithubIssueWorkflowRunId, GithubIssueWorkflowRunKey,
    GithubIssueWorkflowRunStatus, GithubIssueWorkflowState, GithubProviderRef, LeaseReleaseOutcome,
    LeaseRenewalOutcome, ListActiveWorkflowRunsForRepositoryInput, ListWorkflowEventsAfterInput,
    ProviderActionStatus, RecordWorkflowEventInput, RecordWorkflowEventOutcome,
    ReleaseWorkflowRunLeaseInput, RenewWorkflowRunLeaseInput, TransitionOutcome,
    UpsertProviderBindingInput, WorkflowIdempotencyKey, WorkflowStepRun, WorkflowStepRunId,
    WorkflowStepStatus,
};
use ironclaw_host_api::TenantId;

#[derive(Debug)]
pub struct InMemoryGithubIssueWorkflowRepository {
    state: Mutex<InMemoryState>,
}

impl Default for InMemoryGithubIssueWorkflowRepository {
    fn default() -> Self {
        Self {
            state: Mutex::new(InMemoryState::default()),
        }
    }
}

#[derive(Debug, Default)]
struct InMemoryState {
    runs_by_id: HashMap<GithubIssueWorkflowRunId, GithubIssueWorkflowRun>,
    run_ids_by_tenant_key: HashMap<TenantWorkflowRunKey, GithubIssueWorkflowRunId>,
    run_order: Vec<GithubIssueWorkflowRunId>,
    events_by_id: HashMap<GithubIssueWorkflowEventId, GithubIssueWorkflowEvent>,
    event_ids_by_run_key: HashMap<RunIdempotencyKey, GithubIssueWorkflowEventId>,
    next_event_sequence_by_run: HashMap<GithubIssueWorkflowRunId, i64>,
    stage_runs_by_id: HashMap<GithubIssueStageRunId, InMemoryStageRun>,
    workflow_steps_by_id: HashMap<WorkflowStepRunId, WorkflowStepRun>,
    workflow_step_ids_by_run_key: HashMap<RunIdempotencyKey, WorkflowStepRunId>,
    provider_actions_by_id: HashMap<GithubIssueProviderActionId, GithubIssueProviderActionRecord>,
    provider_action_ids_by_run_key: HashMap<RunIdempotencyKey, GithubIssueProviderActionId>,
    provider_bindings_by_id: HashMap<GithubIssueProviderBindingId, GithubIssueProviderBinding>,
    provider_binding_ids_by_ref: HashMap<ProviderBindingKey, GithubIssueProviderBindingId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantWorkflowRunKey {
    tenant_id: TenantId,
    workflow_run_key: GithubIssueWorkflowRunKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunIdempotencyKey {
    workflow_run_id: GithubIssueWorkflowRunId,
    idempotency_key: WorkflowIdempotencyKey,
}

#[derive(Debug, Clone)]
struct InMemoryStageRun {
    workflow_run_id: GithubIssueWorkflowRunId,
    result: Option<JsonValue>,
    active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProviderBindingKey {
    system: String,
    resource_type: String,
    role: String,
    owner: String,
    repo: String,
    provider_id: String,
}

impl ProviderBindingKey {
    fn from_provider_ref(provider_ref: &GithubProviderRef, role: &str) -> Self {
        Self {
            system: provider_ref.system.clone(),
            resource_type: provider_ref.resource_type.clone(),
            role: role.to_string(),
            owner: provider_ref.owner.clone(),
            repo: provider_ref.repo.clone(),
            provider_id: provider_ref.provider_id.clone(),
        }
    }
}

#[async_trait]
impl GithubIssueWorkflowRepository for InMemoryGithubIssueWorkflowRepository {
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError> {
        let workflow_run_key = GithubIssueWorkflowRunKey::for_issue(&input.issue_ref)?;
        let tenant_key = TenantWorkflowRunKey {
            tenant_id: input.tenant_id.clone(),
            workflow_run_key: workflow_run_key.clone(),
        };

        let mut state = self.state.lock().await;
        if let Some(existing_run_id) = state.run_ids_by_tenant_key.get(&tenant_key) {
            let existing = state
                .runs_by_id
                .get(existing_run_id)
                .cloned()
                .ok_or_else(|| repository_error("workflow run index pointed to a missing run"))?;
            return Ok(CreateOrGetWorkflowRunOutcome::Existing { run: existing });
        }

        let workflow_run_id = GithubIssueWorkflowRunId::new();
        let run = GithubIssueWorkflowRun {
            workflow_run_id: workflow_run_id.clone(),
            workflow_run_key,
            tenant_id: input.tenant_id,
            creator_user_id: input.creator_user_id,
            agent_id: input.agent_id,
            project_id: input.project_id,
            provider_account_ref: input.provider_account_ref,
            issue_ref: input.issue_ref,
            workflow_policy_key: input.workflow_policy_key,
            workflow_policy_version: input.workflow_policy_version,
            status: GithubIssueWorkflowRunStatus::Active,
            workflow_state: GithubIssueWorkflowState::new(GithubIssueWorkflowMode::New),
            event_cursor: 0,
            workflow_run_version: 0,
            lease_owner: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            active_stage_run_id: None,
            workspace_session_id: None,
            created_at: input.now,
            updated_at: input.now,
        };

        state
            .run_ids_by_tenant_key
            .insert(tenant_key, workflow_run_id.clone());
        state.run_order.push(workflow_run_id.clone());
        state.runs_by_id.insert(workflow_run_id, run.clone());

        Ok(CreateOrGetWorkflowRunOutcome::Created { run })
    }

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        let run_event_key = RunIdempotencyKey {
            workflow_run_id: input.workflow_run_id.clone(),
            idempotency_key: input.envelope.idempotency_key.clone(),
        };
        if let Some(existing_event_id) = state.event_ids_by_run_key.get(&run_event_key) {
            let existing = state
                .events_by_id
                .get(existing_event_id)
                .cloned()
                .ok_or_else(|| {
                    repository_error("event idempotency index pointed to a missing event")
                })?;
            return Ok(RecordWorkflowEventOutcome::Duplicate { existing });
        }

        if let Some(existing) = superseding_event(&state, &input) {
            return Ok(RecordWorkflowEventOutcome::Superseded { existing });
        }

        let next_sequence = state
            .next_event_sequence_by_run
            .entry(input.workflow_run_id.clone())
            .and_modify(|sequence| *sequence += 1)
            .or_insert(1);
        let sequence = *next_sequence;
        let workflow_event_id = GithubIssueWorkflowEventId::new();
        let event = GithubIssueWorkflowEvent {
            workflow_event_id: workflow_event_id.clone(),
            workflow_run_id: input.workflow_run_id,
            sequence,
            workflow_event_type: input.workflow_event_type,
            idempotency_key: input.envelope.idempotency_key,
            source_kind: input.envelope.source_kind,
            source_delivery_id: input.envelope.source_delivery_id,
            provider: input.envelope.provider,
            provider_updated_at: input.envelope.provider_updated_at,
            observed_at: input.envelope.observed_at,
            supersedes_workflow_event_id: None,
            payload_schema: input.envelope.payload_schema,
            payload: input.envelope.payload,
            created_at: input.envelope.observed_at,
        };

        let event_key = RunIdempotencyKey {
            workflow_run_id: event.workflow_run_id.clone(),
            idempotency_key: event.idempotency_key.clone(),
        };
        state
            .event_ids_by_run_key
            .insert(event_key, workflow_event_id.clone());
        state.events_by_id.insert(workflow_event_id, event.clone());

        Ok(RecordWorkflowEventOutcome::Recorded { event })
    }

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        if input.limit == 0 {
            return Ok(Vec::new());
        }

        let state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        let mut events: Vec<_> = state
            .events_by_id
            .values()
            .filter(|event| {
                event.workflow_run_id == input.workflow_run_id
                    && event.sequence > input.after_sequence
            })
            .cloned()
            .collect();
        events.sort_by_key(|event| event.sequence);
        events.truncate(input.limit);
        Ok(events)
    }

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        if input.limit == 0 {
            return Ok(Vec::new());
        }

        let mut state = self.state.lock().await;
        let run_order = state.run_order.clone();
        let mut claimed = Vec::new();

        for workflow_run_id in run_order {
            if claimed.len() >= input.limit {
                break;
            }
            let Some(run) = state.runs_by_id.get_mut(&workflow_run_id) else {
                continue;
            };
            if run.tenant_id != input.tenant_id
                || run.status != GithubIssueWorkflowRunStatus::Active
                || !lease_is_claimable(run, input.now)
            {
                continue;
            }

            run.lease_owner = Some(input.worker_id.clone());
            run.lease_expires_at = Some(input.lease_expires_at);
            run.last_heartbeat_at = Some(input.now);
            run.claim_count = run.claim_count.saturating_add(1);
            run.workflow_run_version += 1;
            run.updated_at = input.now;
            claimed.push(run.clone());
        }

        Ok(claimed)
    }

    async fn list_active_workflow_runs_for_repository(
        &self,
        input: ListActiveWorkflowRunsForRepositoryInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        if input.limit == 0 {
            return Ok(Vec::new());
        }

        let state = self.state.lock().await;
        let mut runs: Vec<_> = state
            .run_order
            .iter()
            .filter_map(|run_id| state.runs_by_id.get(run_id))
            .filter(|run| {
                run.tenant_id == input.tenant_id
                    && run.issue_ref.owner == input.repository.owner
                    && run.issue_ref.repo == input.repository.repo
                    && !matches!(
                        run.status,
                        GithubIssueWorkflowRunStatus::Succeeded
                            | GithubIssueWorkflowRunStatus::Failed
                            | GithubIssueWorkflowRunStatus::Cancelled
                    )
            })
            .take(input.limit)
            .cloned()
            .collect();
        runs.sort_by_key(|run| run.created_at);
        Ok(runs)
    }

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let run = state
            .runs_by_id
            .get_mut(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        if is_terminal(&run.status) {
            return Ok(LeaseRenewalOutcome::Terminal);
        }
        if !lease_is_owned_by(run, &input.worker_id, input.now) {
            return Ok(LeaseRenewalOutcome::NotLeaseOwner);
        }

        run.lease_expires_at = Some(input.lease_expires_at);
        run.last_heartbeat_at = Some(input.now);
        run.workflow_run_version += 1;
        run.updated_at = input.now;

        Ok(LeaseRenewalOutcome::Renewed { run: run.clone() })
    }

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let run = state
            .runs_by_id
            .get_mut(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        if is_terminal(&run.status) {
            return Ok(LeaseReleaseOutcome::Terminal);
        }
        if run.lease_owner.as_ref() != Some(&input.worker_id) {
            return Ok(LeaseReleaseOutcome::NotLeaseOwner);
        }

        run.lease_owner = None;
        run.lease_expires_at = None;
        run.last_heartbeat_at = Some(input.now);
        run.workflow_run_version += 1;
        run.updated_at = input.now;

        Ok(LeaseReleaseOutcome::Released { run: run.clone() })
    }

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let run = state
            .runs_by_id
            .get_mut(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        if is_terminal(&run.status) {
            return Ok(BlockWorkflowRunOutcome::Terminal);
        }
        if !lease_is_owned_by(run, &input.worker_id, input.now) {
            return Ok(BlockWorkflowRunOutcome::NotLeaseOwner);
        }

        run.status = GithubIssueWorkflowRunStatus::Blocked;
        run.workflow_state.active_block = Some(input.active_block);
        run.lease_owner = None;
        run.lease_expires_at = None;
        run.active_stage_run_id = None;
        run.last_heartbeat_at = Some(input.now);
        run.workflow_run_version += 1;
        run.updated_at = input.now;

        Ok(BlockWorkflowRunOutcome::Blocked { run: run.clone() })
    }

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        let state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        Ok(state
            .events_by_id
            .values()
            .filter(|event| {
                event.workflow_run_id == input.workflow_run_id
                    && event.provider == input.provider
                    && input
                        .workflow_event_types
                        .iter()
                        .any(|event_type| event_type == &event.workflow_event_type)
            })
            .max_by(|left, right| {
                left.provider_updated_at
                    .cmp(&right.provider_updated_at)
                    .then_with(|| left.sequence.cmp(&right.sequence))
            })
            .cloned())
    }

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let run = state
            .runs_by_id
            .get_mut(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        if is_terminal(&run.status) {
            return Ok(TransitionOutcome::Terminal);
        }
        if !lease_is_owned_by(run, &input.worker_id, input.now) {
            return Ok(TransitionOutcome::NotLeaseOwner);
        }
        if run.workflow_run_version != input.expected_workflow_run_version
            || run.event_cursor != input.expected_event_cursor
        {
            return Ok(TransitionOutcome::VersionConflict {
                current: run.clone(),
            });
        }
        if input.next_event_cursor < input.expected_event_cursor {
            return Err(repository_error(
                "next event cursor must not move backwards",
            ));
        }

        run.event_cursor = input.next_event_cursor;
        if let Some(status) = input.transition.status {
            run.status = status;
        }
        if let Some(mode) = input.transition.mode {
            run.workflow_state.mode = mode;
        }
        if input.transition.clear_active_block {
            run.workflow_state.active_block = None;
        }
        if let Some(active_block) = input.transition.active_block {
            run.workflow_state.active_block = Some(active_block);
        }
        if let Some(workspace_session) = input.transition.workspace_session {
            run.workspace_session_id = Some(workspace_session.workspace_session_id);
            run.workflow_state.current_workspace_ref = Some(workspace_session.workspace_ref);
            run.workflow_state.current_workspace_mount_ref = Some(workspace_session.mount_ref);
        }
        if let Some(primary_pr) = input.transition.primary_pr {
            run.workflow_state.primary_pr = Some(primary_pr);
        }

        run.workflow_run_version += 1;
        run.updated_at = input.now;
        if is_terminal(&run.status) {
            run.lease_owner = None;
            run.lease_expires_at = None;
            run.active_stage_run_id = None;
        }

        Ok(TransitionOutcome::Applied { run: run.clone() })
    }

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError> {
        let CreateStageRunInput {
            workflow_run_id,
            stage: _stage,
            now,
        } = input;
        let mut state = self.state.lock().await;
        let run = state.runs_by_id.get_mut(&workflow_run_id).ok_or_else(|| {
            repository_error(format!("workflow run `{}` does not exist", workflow_run_id))
        })?;

        if is_terminal(&run.status) {
            return Ok(CreateStageRunOutcome::Terminal);
        }
        if let Some(active_stage_run_id) = run.active_stage_run_id.clone() {
            return Ok(CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id: active_stage_run_id,
                run: run.clone(),
            });
        }

        let stage_run_id = GithubIssueStageRunId::new();
        run.active_stage_run_id = Some(stage_run_id.clone());
        run.workflow_run_version += 1;
        run.updated_at = now;
        let updated_run = run.clone();

        state.stage_runs_by_id.insert(
            stage_run_id.clone(),
            InMemoryStageRun {
                workflow_run_id,
                result: None,
                active: true,
            },
        );

        Ok(CreateStageRunOutcome::Created {
            stage_run_id,
            run: updated_run,
        })
    }

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let run = state
            .runs_by_id
            .get(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        if is_terminal(&run.status) {
            return Ok(AcceptStageResultOutcome::Terminal);
        }
        if run.active_stage_run_id.as_ref() != Some(&input.stage_run_id) {
            return Ok(AcceptStageResultOutcome::NotActiveStage { run: run.clone() });
        }
        let stage_is_active_for_run = state
            .stage_runs_by_id
            .get(&input.stage_run_id)
            .map(|stage_run| stage_run.workflow_run_id == input.workflow_run_id && stage_run.active)
            .unwrap_or(false);
        if !stage_is_active_for_run {
            return Ok(AcceptStageResultOutcome::NotActiveStage { run: run.clone() });
        }

        let run = state
            .runs_by_id
            .get_mut(&input.workflow_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow run `{}` does not exist",
                    input.workflow_run_id
                ))
            })?;

        run.active_stage_run_id = None;
        run.workflow_run_version += 1;
        run.updated_at = input.now;
        let updated_run = run.clone();

        let stage_run = state
            .stage_runs_by_id
            .get_mut(&input.stage_run_id)
            .ok_or_else(|| repository_error("active stage run was missing"))?;
        stage_run.result = Some(input.result);
        stage_run.active = false;

        Ok(AcceptStageResultOutcome::Accepted { run: updated_run })
    }

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        let step_key = RunIdempotencyKey {
            workflow_run_id: input.workflow_run_id.clone(),
            idempotency_key: input.idempotency_key.clone(),
        };
        if let Some(existing_step_id) = state.workflow_step_ids_by_run_key.get(&step_key) {
            let existing = state
                .workflow_steps_by_id
                .get(existing_step_id)
                .cloned()
                .ok_or_else(|| {
                    repository_error("workflow step idempotency index pointed to a missing step")
                })?;
            if existing.input_hash != input.input_hash {
                return Err(repository_error(format!(
                    "workflow step `{}` input hash mismatch for idempotency key `{}`",
                    existing.step_name, existing.idempotency_key
                )));
            }
            return Ok(CreateOrGetWorkflowStepOutcome::Existing { step: existing });
        }

        let step_run_id = WorkflowStepRunId::new();
        let step = WorkflowStepRun {
            step_run_id: step_run_id.clone(),
            workflow_run_id: input.workflow_run_id,
            step_name: input.step_name,
            idempotency_key: input.idempotency_key,
            input_hash: input.input_hash,
            status: WorkflowStepStatus::Pending,
            result: None,
            error: None,
            started_at: input.now,
            completed_at: None,
            next_attempt_at: None,
        };
        let step_key = RunIdempotencyKey {
            workflow_run_id: step.workflow_run_id.clone(),
            idempotency_key: step.idempotency_key.clone(),
        };
        state
            .workflow_step_ids_by_run_key
            .insert(step_key, step_run_id.clone());
        state.workflow_steps_by_id.insert(step_run_id, step.clone());

        Ok(CreateOrGetWorkflowStepOutcome::Created { step })
    }

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let step = state
            .workflow_steps_by_id
            .get_mut(&input.step_run_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "workflow step `{}` does not exist",
                    input.step_run_id
                ))
            })?;

        if workflow_step_is_complete(&step.status) {
            return Ok(CompleteWorkflowStepOutcome::AlreadyCompleted { step: step.clone() });
        }

        step.status = input.status;
        step.result = input.result;
        step.error = input.error;
        step.next_attempt_at = input.next_attempt_at;
        step.completed_at = if workflow_step_is_complete(&step.status) {
            Some(input.now)
        } else {
            None
        };

        Ok(CompleteWorkflowStepOutcome::Completed { step: step.clone() })
    }

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        let action_key = RunIdempotencyKey {
            workflow_run_id: input.workflow_run_id.clone(),
            idempotency_key: input.idempotency_key.clone(),
        };
        if let Some(existing_action_id) = state.provider_action_ids_by_run_key.get(&action_key) {
            let existing = state
                .provider_actions_by_id
                .get(existing_action_id)
                .cloned()
                .ok_or_else(|| {
                    repository_error(
                        "provider action idempotency index pointed to a missing action",
                    )
                })?;
            return Ok(existing);
        }

        let provider_action_id = GithubIssueProviderActionId::new();
        let record = GithubIssueProviderActionRecord {
            provider_action_id: provider_action_id.clone(),
            workflow_run_id: input.workflow_run_id,
            stage_run_id: input.stage_run_id,
            step_run_id: input.step_run_id,
            name: input.name,
            kind: input.kind,
            idempotency_key: input.idempotency_key,
            input_hash: input.input_hash,
            status: ProviderActionStatus::Pending,
            provider_ref: None,
            stable_marker: input.stable_marker,
            reconciliation_strategy: input.reconciliation_strategy,
            lease_owner: None,
            lease_expires_at: None,
            attempt_count: 0,
            next_attempt_at: None,
            last_reconciled_at: None,
            result: None,
            redacted_failure_kind: None,
            created_at: input.now,
            updated_at: input.now,
        };
        let action_key = RunIdempotencyKey {
            workflow_run_id: record.workflow_run_id.clone(),
            idempotency_key: record.idempotency_key.clone(),
        };
        state
            .provider_action_ids_by_run_key
            .insert(action_key, provider_action_id.clone());
        state
            .provider_actions_by_id
            .insert(provider_action_id, record.clone());

        Ok(record)
    }

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let action = state
            .provider_actions_by_id
            .get_mut(&input.provider_action_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "provider action `{}` does not exist",
                    input.provider_action_id
                ))
            })?;

        if provider_action_is_complete(&action.status) {
            return Ok(ClaimProviderActionOutcome::AlreadyCompleted {
                action: action.clone(),
            });
        }
        if !provider_action_lease_is_claimable(action, input.now) {
            return Ok(ClaimProviderActionOutcome::Busy {
                action: action.clone(),
            });
        }

        action.status = ProviderActionStatus::Running;
        action.lease_owner = Some(input.worker_id);
        action.lease_expires_at = Some(input.lease_expires_at);
        action.attempt_count = action.attempt_count.saturating_add(1);
        action.updated_at = input.now;

        Ok(ClaimProviderActionOutcome::Claimed {
            action: action.clone(),
        })
    }

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        let action = state
            .provider_actions_by_id
            .get_mut(&input.provider_action_id)
            .ok_or_else(|| {
                repository_error(format!(
                    "provider action `{}` does not exist",
                    input.provider_action_id
                ))
            })?;

        if provider_action_is_complete(&action.status) {
            return Ok(CompleteProviderActionOutcome::AlreadyCompleted {
                action: action.clone(),
            });
        }
        if action.lease_owner.as_ref() != Some(&input.worker_id) {
            return Ok(CompleteProviderActionOutcome::NotLeaseOwner {
                action: action.clone(),
            });
        }

        action.status = input.status;
        action.provider_ref = input.provider_ref;
        action.stable_marker = input.stable_marker;
        action.result = input.result;
        action.redacted_failure_kind = input.redacted_failure_kind;
        action.next_attempt_at = None;
        action.last_reconciled_at = match action.status {
            ProviderActionStatus::NeedsReconciliation | ProviderActionStatus::Reconciling => {
                Some(input.now)
            }
            _ => action.last_reconciled_at,
        };
        action.lease_owner = None;
        action.lease_expires_at = None;
        action.updated_at = input.now;

        Ok(CompleteProviderActionOutcome::Completed {
            action: action.clone(),
        })
    }

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError> {
        let mut state = self.state.lock().await;
        if !state.runs_by_id.contains_key(&input.workflow_run_id) {
            return Err(repository_error(format!(
                "workflow run `{}` does not exist",
                input.workflow_run_id
            )));
        }

        let binding_key = ProviderBindingKey::from_provider_ref(&input.provider_ref, &input.role);
        if let Some(existing_binding_id) = state.provider_binding_ids_by_ref.get(&binding_key) {
            let existing = state
                .provider_bindings_by_id
                .get(existing_binding_id)
                .cloned()
                .ok_or_else(|| {
                    repository_error("provider binding route index pointed to a missing binding")
                })?;
            return Ok(existing);
        }

        let binding_id = GithubIssueProviderBindingId::new();
        let binding = GithubIssueProviderBinding {
            binding_id: binding_id.clone(),
            workflow_run_id: input.workflow_run_id,
            system: input.provider_ref.system,
            resource_type: input.provider_ref.resource_type,
            role: input.role,
            owner: input.provider_ref.owner,
            repo: input.provider_ref.repo,
            provider_id: input.provider_ref.provider_id,
            provider_url: input.provider_ref.provider_url,
            created_by_provider_action_id: input.created_by_provider_action_id,
            created_at: input.created_at,
        };

        state
            .provider_binding_ids_by_ref
            .insert(binding_key, binding_id.clone());
        state
            .provider_bindings_by_id
            .insert(binding_id, binding.clone());

        Ok(binding)
    }
}

fn superseding_event(
    state: &InMemoryState,
    input: &RecordWorkflowEventInput,
) -> Option<GithubIssueWorkflowEvent> {
    let provider_updated_at = input.envelope.provider_updated_at?;
    state
        .events_by_id
        .values()
        .filter(|event| {
            event.workflow_run_id == input.workflow_run_id
                && event.workflow_event_type == input.workflow_event_type
                && event.provider == input.envelope.provider
        })
        .find(|event| {
            event
                .provider_updated_at
                .map(|existing_provider_updated_at| {
                    existing_provider_updated_at >= provider_updated_at
                })
                .unwrap_or(false)
        })
        .cloned()
}

fn is_terminal(status: &GithubIssueWorkflowRunStatus) -> bool {
    matches!(
        status,
        GithubIssueWorkflowRunStatus::Succeeded
            | GithubIssueWorkflowRunStatus::Failed
            | GithubIssueWorkflowRunStatus::Cancelled
    )
}

fn lease_is_claimable(run: &GithubIssueWorkflowRun, now: chrono::DateTime<chrono::Utc>) -> bool {
    run.lease_owner.is_none()
        || run
            .lease_expires_at
            .map(|expires_at| expires_at <= now)
            .unwrap_or(true)
}

fn lease_is_owned_by(
    run: &GithubIssueWorkflowRun,
    worker_id: &crate::WorkflowWorkerId,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    run.lease_owner.as_ref() == Some(worker_id)
        && run
            .lease_expires_at
            .map(|expires_at| expires_at > now)
            .unwrap_or(false)
}

fn provider_action_is_complete(status: &ProviderActionStatus) -> bool {
    matches!(
        status,
        ProviderActionStatus::Succeeded
            | ProviderActionStatus::Failed
            | ProviderActionStatus::NeedsReconciliation
    )
}

fn workflow_step_is_complete(status: &WorkflowStepStatus) -> bool {
    matches!(
        status,
        WorkflowStepStatus::Succeeded | WorkflowStepStatus::Failed | WorkflowStepStatus::Blocked
    )
}

fn provider_action_lease_is_claimable(
    action: &GithubIssueProviderActionRecord,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    action.lease_owner.is_none()
        || action
            .lease_expires_at
            .map(|expires_at| expires_at <= now)
            .unwrap_or(true)
}

fn repository_error(reason: impl Into<String>) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Repository {
        reason: reason.into(),
    }
}
