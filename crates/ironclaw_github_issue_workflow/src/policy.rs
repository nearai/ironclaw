use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};

use crate::{
    AdvanceWorkflowRunInput, CompleteWorkflowStepInput, CompleteWorkflowStepOutcome,
    CreateOrGetWorkflowStepInput, CreateOrGetWorkflowStepOutcome, CreateStageRunInput,
    CreateStageRunOutcome, GithubIssueBlockKind, GithubIssueBlockState,
    GithubIssueProviderActionRunner, GithubIssueStage, GithubIssueWorkflowError,
    GithubIssueWorkflowEvent, GithubIssueWorkflowEventType, GithubIssueWorkflowMode,
    GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRun,
    GithubIssueWorkflowRunId, GithubIssueWorkflowRunStatus, ListWorkflowEventsAfterInput,
    PrepareWorkflowWorkspaceOutcome, PrepareWorkflowWorkspaceRequest, ProviderActionRunOutcome,
    StageCompletedPayload, StageTurnIdentity, StageTurnSubmitter, SubmitStageTurnOutcome,
    SubmitStageTurnRequest, TransitionOutcome, WorkflowActorScope, WorkflowClock,
    WorkflowIdempotencyKey, WorkflowProjectAccess, WorkflowProjectAccessRequest,
    WorkflowPromptContentRef, WorkflowRunTransition, WorkflowStepRunId, WorkflowWorkerId,
    WorkflowWorkspaceManager, WorkflowWorkspaceMountRef, stage_slug,
};

const DEFAULT_STAGE_ATTEMPT: u32 = 1;
const DEFAULT_PROVIDER_ACTION_LEASE_SECONDS: i64 = 60;
const DEFAULT_BUSY_RETRY_SECONDS: i64 = 30;
const DEFAULT_CAPABILITY_PROFILE_ID: &str = "github_issue_workflow.stage.default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStepRun {
    pub step_run_id: WorkflowStepRunId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub step_name: String,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub input_hash: String,
    pub status: WorkflowStepStatus,
    pub result: Option<JsonValue>,
    pub error: Option<JsonValue>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepStatus {
    Pending,
    Running,
    Retryable,
    Succeeded,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPolicyTickOutcome {
    pub run: GithubIssueWorkflowRun,
    pub processed_event_count: usize,
    pub steps: Vec<WorkflowStepRun>,
}

pub trait GithubIssueWorkflowPolicyPorts: Send + Sync {
    type Clock: WorkflowClock;
    type GithubPort: GithubIssueWorkflowPort;
    type ProjectAccess: WorkflowProjectAccess;
    type Repository: GithubIssueWorkflowRepository;
    type StageTurnSubmitter: StageTurnSubmitter;
    type WorkspaceManager: WorkflowWorkspaceManager;

    fn clock(&self) -> Arc<Self::Clock>;
    fn github_port(&self) -> Arc<Self::GithubPort>;
    fn project_access(&self) -> Arc<Self::ProjectAccess>;
    fn repository(&self) -> Arc<Self::Repository>;
    fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter>;
    fn workspace_manager(&self) -> Arc<Self::WorkspaceManager>;
    fn worker_id(&self) -> WorkflowWorkerId;

    fn provider_action_lease_expires_at(&self) -> DateTime<Utc> {
        self.clock().now() + Duration::seconds(DEFAULT_PROVIDER_ACTION_LEASE_SECONDS)
    }
}

#[derive(Debug)]
pub struct GithubIssueWorkflowPolicy<P> {
    ports: P,
    policy_version: String,
}

impl<P> GithubIssueWorkflowPolicy<P>
where
    P: GithubIssueWorkflowPolicyPorts,
{
    pub fn new(ports: P, policy_version: impl Into<String>) -> Self {
        Self {
            ports,
            policy_version: policy_version.into(),
        }
    }

    pub fn ports(&self) -> &P {
        &self.ports
    }

    pub async fn tick(
        &self,
        run: GithubIssueWorkflowRun,
    ) -> Result<WorkflowPolicyTickOutcome, GithubIssueWorkflowError> {
        if run_is_terminal(&run.status) {
            return Ok(WorkflowPolicyTickOutcome {
                run,
                processed_event_count: 0,
                steps: Vec::new(),
            });
        }

        let repository = self.ports.repository();
        let mut events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id: run.workflow_run_id.clone(),
                after_sequence: run.event_cursor,
                limit: 1,
            })
            .await?;
        let Some(event) = events.pop() else {
            return Ok(WorkflowPolicyTickOutcome {
                run,
                processed_event_count: 0,
                steps: Vec::new(),
            });
        };

        match event.workflow_event_type {
            GithubIssueWorkflowEventType::GithubIssueDiscovered
                if run.workflow_state.mode == GithubIssueWorkflowMode::New =>
            {
                self.process_issue_discovered(run, event).await
            }
            GithubIssueWorkflowEventType::StageCompleted => {
                self.process_stage_completed(run, event).await
            }
            _ => {
                let run = self
                    .advance_run_cursor(run, event.sequence, WorkflowRunTransition::default())
                    .await?;
                Ok(WorkflowPolicyTickOutcome {
                    run,
                    processed_event_count: 1,
                    steps: Vec::new(),
                })
            }
        }
    }

    async fn process_issue_discovered(
        &self,
        run: GithubIssueWorkflowRun,
        event: GithubIssueWorkflowEvent,
    ) -> Result<WorkflowPolicyTickOutcome, GithubIssueWorkflowError> {
        let mut steps = Vec::new();
        let claim_step = self.claim_issue_step(&run).await?;
        let claim_succeeded = claim_step.status == WorkflowStepStatus::Succeeded;
        steps.push(claim_step);
        if !claim_succeeded {
            return Ok(WorkflowPolicyTickOutcome {
                run,
                processed_event_count: 0,
                steps,
            });
        }

        let (run, start_stage_step) = self
            .start_stage_step(run, GithubIssueStage::Triage, None)
            .await?;
        let stage_submitted = start_stage_step.status == WorkflowStepStatus::Succeeded;
        steps.push(start_stage_step);
        if !stage_submitted {
            return Ok(WorkflowPolicyTickOutcome {
                run,
                processed_event_count: 0,
                steps,
            });
        }

        let run = self
            .advance_run_cursor(
                run,
                event.sequence,
                WorkflowRunTransition {
                    mode: Some(GithubIssueWorkflowMode::Claimed),
                    clear_active_block: true,
                    ..WorkflowRunTransition::default()
                },
            )
            .await?;

        Ok(WorkflowPolicyTickOutcome {
            run,
            processed_event_count: 1,
            steps,
        })
    }

    async fn process_stage_completed(
        &self,
        run: GithubIssueWorkflowRun,
        event: GithubIssueWorkflowEvent,
    ) -> Result<WorkflowPolicyTickOutcome, GithubIssueWorkflowError> {
        let payload = stage_completed_payload(&event)?;
        match payload.stage {
            GithubIssueStage::Triage
                if matches!(
                    run.workflow_state.mode,
                    GithubIssueWorkflowMode::Claimed | GithubIssueWorkflowMode::Triage
                ) =>
            {
                let (run, step) = self
                    .start_stage_step(run, GithubIssueStage::Planning, None)
                    .await?;
                if step.status != WorkflowStepStatus::Succeeded {
                    return Ok(WorkflowPolicyTickOutcome {
                        run,
                        processed_event_count: 0,
                        steps: vec![step],
                    });
                }
                let run = self
                    .advance_run_cursor(
                        run,
                        event.sequence,
                        WorkflowRunTransition {
                            mode: Some(GithubIssueWorkflowMode::Planning),
                            clear_active_block: true,
                            ..WorkflowRunTransition::default()
                        },
                    )
                    .await?;
                Ok(WorkflowPolicyTickOutcome {
                    run,
                    processed_event_count: 1,
                    steps: vec![step],
                })
            }
            GithubIssueStage::Planning
                if run.workflow_state.mode == GithubIssueWorkflowMode::Planning =>
            {
                let (workspace, workspace_step) = self.prepare_workspace_step(&run).await?;
                let (run, start_step) = self
                    .start_stage_step(
                        run,
                        GithubIssueStage::Implementation,
                        Some(workspace.mount_ref),
                    )
                    .await?;
                if start_step.status != WorkflowStepStatus::Succeeded {
                    return Ok(WorkflowPolicyTickOutcome {
                        run,
                        processed_event_count: 0,
                        steps: vec![workspace_step, start_step],
                    });
                }
                let run = self
                    .advance_run_cursor(
                        run,
                        event.sequence,
                        WorkflowRunTransition {
                            mode: Some(GithubIssueWorkflowMode::Implementation),
                            clear_active_block: true,
                            workspace_session_id: Some(workspace.workspace_session_id.clone()),
                            ..WorkflowRunTransition::default()
                        },
                    )
                    .await?;
                Ok(WorkflowPolicyTickOutcome {
                    run,
                    processed_event_count: 1,
                    steps: vec![workspace_step, start_step],
                })
            }
            _ => {
                let run = self
                    .advance_run_cursor(run, event.sequence, WorkflowRunTransition::default())
                    .await?;
                Ok(WorkflowPolicyTickOutcome {
                    run,
                    processed_event_count: 1,
                    steps: Vec::new(),
                })
            }
        }
    }

    async fn claim_issue_step(
        &self,
        run: &GithubIssueWorkflowRun,
    ) -> Result<WorkflowStepRun, GithubIssueWorkflowError> {
        let input = json!({
            "workflow_run_id": run.workflow_run_id,
            "issue": run.issue_ref,
            "policy_version": self.policy_version,
        });
        let step = self.create_or_get_step(run, "claim_issue", &input).await?;
        if workflow_step_replays(&step.status) {
            return Ok(step);
        }
        let now = self.ports.clock().now();
        if workflow_step_waits_for_retry(&step, now) {
            return Ok(step);
        }

        let runner =
            GithubIssueProviderActionRunner::new(self.ports.repository(), self.ports.github_port());
        let outcome = runner
            .run_claim_comment(crate::RunClaimCommentProviderActionRequest {
                run: run.clone(),
                worker_id: self.ports.worker_id(),
                now,
                lease_expires_at: self.ports.provider_action_lease_expires_at(),
            })
            .await?;

        let status = match &outcome {
            ProviderActionRunOutcome::Succeeded { .. }
            | ProviderActionRunOutcome::Replayed { .. } => WorkflowStepStatus::Succeeded,
            ProviderActionRunOutcome::Busy { .. } => WorkflowStepStatus::Retryable,
            ProviderActionRunOutcome::NeedsReconciliation { .. } => WorkflowStepStatus::Blocked,
            ProviderActionRunOutcome::Failed { .. } => WorkflowStepStatus::Failed,
        };
        let result = serde_json::to_value(&outcome).map_err(policy_serde_error)?;
        if let ProviderActionRunOutcome::Busy { action } = outcome {
            let next_attempt_at = action
                .lease_expires_at
                .unwrap_or(now + Duration::seconds(DEFAULT_BUSY_RETRY_SECONDS));
            return self
                .retry_step(step, Some(result), None, next_attempt_at)
                .await;
        }

        self.complete_step(step, status, Some(result), None).await
    }

    async fn prepare_workspace_step(
        &self,
        run: &GithubIssueWorkflowRun,
    ) -> Result<(PrepareWorkflowWorkspaceOutcome, WorkflowStepRun), GithubIssueWorkflowError> {
        let input = json!({
            "workflow_run_id": run.workflow_run_id,
            "issue": run.issue_ref,
            "policy_version": self.policy_version,
        });
        let step = self
            .create_or_get_step(run, "prepare_workspace", &input)
            .await?;
        if workflow_step_replays(&step.status) {
            let Some(result) = step.result.clone() else {
                return Err(GithubIssueWorkflowError::Policy {
                    reason: "completed prepare_workspace step had no result".to_string(),
                });
            };
            let outcome = serde_json::from_value(result).map_err(policy_serde_error)?;
            return Ok((outcome, step));
        }

        let outcome = self
            .ports
            .workspace_manager()
            .prepare_workspace(PrepareWorkflowWorkspaceRequest {
                tenant_id: run.tenant_id.clone(),
                creator_user_id: run.creator_user_id.clone(),
                agent_id: run.agent_id.clone(),
                project_id: run.project_id.clone(),
                workflow_run_id: run.workflow_run_id.clone(),
                issue: run.issue_ref.clone(),
                base_branch: run.issue_ref.default_branch.clone(),
                requested_at: self.ports.clock().now(),
            })
            .await?;
        let result = serde_json::to_value(&outcome).map_err(policy_serde_error)?;
        let completed = self
            .complete_step(step, WorkflowStepStatus::Succeeded, Some(result), None)
            .await?;
        Ok((outcome, completed))
    }

    async fn start_stage_step(
        &self,
        run: GithubIssueWorkflowRun,
        stage: GithubIssueStage,
        workspace_mount_ref: Option<WorkflowWorkspaceMountRef>,
    ) -> Result<(GithubIssueWorkflowRun, WorkflowStepRun), GithubIssueWorkflowError> {
        let input = json!({
            "workflow_run_id": run.workflow_run_id,
            "stage": stage_slug(&stage),
            "workspace_mount_ref": workspace_mount_ref,
            "policy_version": self.policy_version,
        });
        let step_name = format!("start_stage:{}", stage_slug(&stage));
        let step = self.create_or_get_step(&run, &step_name, &input).await?;
        if workflow_step_replays(&step.status) {
            return Ok((run, step));
        }
        let now = self.ports.clock().now();
        if workflow_step_waits_for_retry(&step, now) {
            return Ok((run, step));
        }

        if let Err(error) = self
            .ports
            .project_access()
            .assert_workflow_project_access(WorkflowProjectAccessRequest {
                tenant_id: run.tenant_id.clone(),
                creator_user_id: run.creator_user_id.clone(),
                agent_id: run.agent_id.clone(),
                project_id: run.project_id.clone(),
                workflow_run_id: run.workflow_run_id.clone(),
                issue: run.issue_ref.clone(),
            })
            .await
        {
            let reason = error.to_string();
            let blocked_step = self
                .complete_step(
                    step,
                    WorkflowStepStatus::Blocked,
                    None,
                    Some(json!({ "reason": reason })),
                )
                .await?;
            let blocked_run = self.block_run(run, reason).await?;
            return Ok((blocked_run, blocked_step));
        }

        let repository = self.ports.repository();
        let stage_run = repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage: stage.clone(),
                now,
            })
            .await?;
        let (stage_run_id, run) = match stage_run {
            CreateStageRunOutcome::Created { stage_run_id, run }
            | CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id: stage_run_id,
                run,
            } => (stage_run_id, run),
            CreateStageRunOutcome::Terminal => {
                let blocked_step = self
                    .complete_step(
                        step,
                        WorkflowStepStatus::Blocked,
                        None,
                        Some(json!({ "reason": "workflow run is terminal" })),
                    )
                    .await?;
                return Ok((run, blocked_step));
            }
        };

        let stage_turn_identity = StageTurnIdentity::new(
            run.workflow_run_id.clone(),
            stage_run_id,
            stage.clone(),
            DEFAULT_STAGE_ATTEMPT,
            self.policy_version.clone(),
        );
        let content_ref = self.stage_content_ref(&run, &stage)?;
        let idempotency_key = stage_turn_identity.turn_idempotency_key();
        let outcome = self
            .ports
            .stage_turn_submitter()
            .submit_stage_turn(SubmitStageTurnRequest {
                stage_turn_identity,
                scope: WorkflowActorScope {
                    tenant_id: run.tenant_id.clone(),
                    creator_user_id: run.creator_user_id.clone(),
                    agent_id: run.agent_id.clone(),
                    project_id: run.project_id.clone(),
                    workflow_run_id: run.workflow_run_id.clone(),
                },
                content_ref,
                capability_profile_id: DEFAULT_CAPABILITY_PROFILE_ID.to_string(),
                workspace_mount_ref,
                idempotency_key,
            })
            .await?;

        let status = match &outcome {
            SubmitStageTurnOutcome::Submitted { .. } | SubmitStageTurnOutcome::Replayed { .. } => {
                WorkflowStepStatus::Succeeded
            }
            SubmitStageTurnOutcome::Busy { .. } => WorkflowStepStatus::Retryable,
        };
        let result = serde_json::to_value(&outcome).map_err(policy_serde_error)?;
        if let SubmitStageTurnOutcome::Busy { reason } = outcome {
            let completed = self
                .retry_step(
                    step,
                    Some(result),
                    Some(json!({ "reason": reason })),
                    now + Duration::seconds(DEFAULT_BUSY_RETRY_SECONDS),
                )
                .await?;
            return Ok((run, completed));
        }
        let completed = self.complete_step(step, status, Some(result), None).await?;

        Ok((run, completed))
    }

    fn stage_content_ref(
        &self,
        run: &GithubIssueWorkflowRun,
        stage: &GithubIssueStage,
    ) -> Result<WorkflowPromptContentRef, GithubIssueWorkflowError> {
        let input_snapshot_hash = workflow_input_hash(
            "stage_input",
            &json!({
                "workflow_run_id": run.workflow_run_id,
                "issue": run.issue_ref,
                "stage": stage_slug(stage),
                "policy_version": self.policy_version,
            }),
        )?;

        Ok(WorkflowPromptContentRef {
            prompt_ref: format!("github_issue_workflow/{}", stage_slug(stage)),
            prompt_version: self.policy_version.clone(),
            input_snapshot_hash,
        })
    }

    async fn create_or_get_step<T>(
        &self,
        run: &GithubIssueWorkflowRun,
        step_name: &str,
        input: &T,
    ) -> Result<WorkflowStepRun, GithubIssueWorkflowError>
    where
        T: Serialize,
    {
        let input_hash = workflow_input_hash(step_name, input)?;
        let idempotency_key = WorkflowIdempotencyKey::from_generated(format!(
            "policy-step:{}:{}:{}",
            self.policy_version, run.workflow_run_id, step_name
        ));
        let outcome = self
            .ports
            .repository()
            .create_or_get_workflow_step(CreateOrGetWorkflowStepInput {
                workflow_run_id: run.workflow_run_id.clone(),
                step_name: step_name.to_string(),
                idempotency_key,
                input_hash,
                now: self.ports.clock().now(),
            })
            .await?;

        Ok(match outcome {
            CreateOrGetWorkflowStepOutcome::Created { step }
            | CreateOrGetWorkflowStepOutcome::Existing { step } => step,
        })
    }

    async fn complete_step(
        &self,
        step: WorkflowStepRun,
        status: WorkflowStepStatus,
        result: Option<JsonValue>,
        error: Option<JsonValue>,
    ) -> Result<WorkflowStepRun, GithubIssueWorkflowError> {
        self.update_step(step, status, result, error, None).await
    }

    async fn retry_step(
        &self,
        step: WorkflowStepRun,
        result: Option<JsonValue>,
        error: Option<JsonValue>,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<WorkflowStepRun, GithubIssueWorkflowError> {
        self.update_step(
            step,
            WorkflowStepStatus::Retryable,
            result,
            error,
            Some(next_attempt_at),
        )
        .await
    }

    async fn update_step(
        &self,
        step: WorkflowStepRun,
        status: WorkflowStepStatus,
        result: Option<JsonValue>,
        error: Option<JsonValue>,
        next_attempt_at: Option<DateTime<Utc>>,
    ) -> Result<WorkflowStepRun, GithubIssueWorkflowError> {
        let outcome = self
            .ports
            .repository()
            .complete_workflow_step(CompleteWorkflowStepInput {
                step_run_id: step.step_run_id,
                status,
                result,
                error,
                next_attempt_at,
                now: self.ports.clock().now(),
            })
            .await?;

        Ok(match outcome {
            CompleteWorkflowStepOutcome::Completed { step }
            | CompleteWorkflowStepOutcome::AlreadyCompleted { step } => step,
        })
    }

    async fn advance_run_cursor(
        &self,
        run: GithubIssueWorkflowRun,
        next_event_cursor: i64,
        transition: WorkflowRunTransition,
    ) -> Result<GithubIssueWorkflowRun, GithubIssueWorkflowError> {
        let outcome = self
            .ports
            .repository()
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: self.ports.worker_id(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor,
                transition,
                now: self.ports.clock().now(),
            })
            .await?;

        match outcome {
            TransitionOutcome::Applied { run }
            | TransitionOutcome::VersionConflict { current: run } => Ok(run),
            TransitionOutcome::NotLeaseOwner => Err(GithubIssueWorkflowError::Policy {
                reason: format!(
                    "worker `{}` does not own workflow run `{}` lease",
                    self.ports.worker_id(),
                    run.workflow_run_id
                ),
            }),
            TransitionOutcome::Terminal => Ok(run),
        }
    }

    async fn block_run(
        &self,
        run: GithubIssueWorkflowRun,
        reason: String,
    ) -> Result<GithubIssueWorkflowRun, GithubIssueWorkflowError> {
        let outcome = self
            .ports
            .repository()
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: self.ports.worker_id(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor: run.event_cursor,
                transition: WorkflowRunTransition {
                    status: Some(GithubIssueWorkflowRunStatus::Blocked),
                    active_block: Some(GithubIssueBlockState {
                        kind: GithubIssueBlockKind::BlockedHuman,
                        reason,
                        blocked_at: self.ports.clock().now(),
                    }),
                    ..WorkflowRunTransition::default()
                },
                now: self.ports.clock().now(),
            })
            .await?;

        match outcome {
            TransitionOutcome::Applied { run }
            | TransitionOutcome::VersionConflict { current: run } => Ok(run),
            TransitionOutcome::NotLeaseOwner => Err(GithubIssueWorkflowError::Policy {
                reason: format!(
                    "worker `{}` does not own workflow run `{}` lease",
                    self.ports.worker_id(),
                    run.workflow_run_id
                ),
            }),
            TransitionOutcome::Terminal => Ok(run),
        }
    }
}

fn workflow_step_replays(status: &WorkflowStepStatus) -> bool {
    matches!(
        status,
        WorkflowStepStatus::Succeeded | WorkflowStepStatus::Failed | WorkflowStepStatus::Blocked
    )
}

fn workflow_step_waits_for_retry(step: &WorkflowStepRun, now: DateTime<Utc>) -> bool {
    step.status == WorkflowStepStatus::Retryable
        && step
            .next_attempt_at
            .map(|next_attempt_at| next_attempt_at > now)
            .unwrap_or(false)
}

fn run_is_terminal(status: &GithubIssueWorkflowRunStatus) -> bool {
    matches!(
        status,
        GithubIssueWorkflowRunStatus::Succeeded
            | GithubIssueWorkflowRunStatus::Failed
            | GithubIssueWorkflowRunStatus::Cancelled
    )
}

fn stage_completed_payload(
    event: &GithubIssueWorkflowEvent,
) -> Result<StageCompletedPayload, GithubIssueWorkflowError> {
    serde_json::from_value(event.payload.clone()).map_err(policy_serde_error)
}

fn workflow_input_hash<T>(label: &str, input: &T) -> Result<String, GithubIssueWorkflowError>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(input).map_err(policy_serde_error)?;
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(&bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn policy_serde_error(error: serde_json::Error) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: error.to_string(),
    }
}
