use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha256};

use crate::{
    AdvanceWorkflowRunInput, CompleteWorkflowStepInput, CompleteWorkflowStepOutcome,
    CreateOrGetWorkflowStepInput, CreateOrGetWorkflowStepOutcome, CreateStageRunInput,
    CreateStageRunOutcome, EngineeredWorkflowSnapshot, GithubIssueBlockKind, GithubIssueBlockState,
    GithubIssuePlanItemStatus, GithubIssueProviderActionRunner, GithubIssueSnapshot,
    GithubIssueStage, GithubIssueWorkflowError, GithubIssueWorkflowEvent,
    GithubIssueWorkflowEventType, GithubIssueWorkflowMode, GithubIssueWorkflowPort,
    GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubIssueWorkflowRunId,
    GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId,
    GithubRepositorySelector, ListWorkflowEventsAfterInput, PrepareWorkflowWorkspaceRequest,
    ProviderActionRunOutcome, ProviderContentSummary, RepositorySnapshot, StageCompletedPayload,
    StageConstraintSnapshot, StageTurnIdentity, StageTurnSubmitter, SubmitStageTurnOutcome,
    SubmitStageTurnRequest, TransitionOutcome, WorkflowActorScope, WorkflowClock,
    WorkflowIdempotencyKey, WorkflowProjectAccess, WorkflowProjectAccessRequest,
    WorkflowPromptContent, WorkflowRunTransition, WorkflowStateSnapshot, WorkflowStepRunId,
    WorkflowWorkerId, WorkflowWorkspaceManager, WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
    WorkflowWorkspaceSnapshot, render_stage_prompt, stage_result_schema_version, stage_slug,
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

enum PrepareWorkspaceStepOutcome {
    Prepared {
        session: GithubIssueWorkspaceSession,
        step: WorkflowStepRun,
    },
    NotReady {
        step: WorkflowStepRun,
    },
}

pub trait GithubIssueWorkflowPolicyPorts: Send + Sync {
    type Clock: WorkflowClock + ?Sized;
    type GithubPort: GithubIssueWorkflowPort + ?Sized;
    type ProjectAccess: WorkflowProjectAccess + ?Sized;
    type Repository: GithubIssueWorkflowRepository + ?Sized;
    type StageTurnSubmitter: StageTurnSubmitter + ?Sized;
    type WorkspaceManager: WorkflowWorkspaceManager + ?Sized;

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
                let (workspace_session, workspace_step) =
                    match self.prepare_workspace_step(&run).await? {
                        PrepareWorkspaceStepOutcome::Prepared { session, step } => (session, step),
                        PrepareWorkspaceStepOutcome::NotReady { step } => {
                            return Ok(WorkflowPolicyTickOutcome {
                                run,
                                processed_event_count: 0,
                                steps: vec![step],
                            });
                        }
                    };
                let (run, start_step) = self
                    .start_stage_step(
                        run,
                        GithubIssueStage::Implementation,
                        Some(workspace_session.clone()),
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
                            workspace_session: Some(workspace_session),
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
    ) -> Result<PrepareWorkspaceStepOutcome, GithubIssueWorkflowError> {
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
            let session = decode_prepare_workspace_outcome(
                result,
                run,
                step.completed_at.unwrap_or(step.started_at),
            )?;
            return Ok(PrepareWorkspaceStepOutcome::Prepared { session, step });
        }
        let now = self.ports.clock().now();
        if workflow_step_waits_for_retry(&step, now) {
            return Ok(PrepareWorkspaceStepOutcome::NotReady { step });
        }

        let outcome = match self
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
                requested_at: now,
            })
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if !workspace_prepare_error_is_retryable(&error) {
                    return Err(error);
                }
                let retry = self
                    .retry_step(
                        step,
                        None,
                        Some(json!({ "reason": error.to_string() })),
                        now + Duration::seconds(DEFAULT_BUSY_RETRY_SECONDS),
                    )
                    .await?;
                return Ok(PrepareWorkspaceStepOutcome::NotReady { step: retry });
            }
        };
        let session = outcome.session.clone();
        let result = serde_json::to_value(&outcome).map_err(policy_serde_error)?;
        let completed = self
            .complete_step(step, WorkflowStepStatus::Succeeded, Some(result), None)
            .await?;
        Ok(PrepareWorkspaceStepOutcome::Prepared {
            session,
            step: completed,
        })
    }

    async fn start_stage_step(
        &self,
        run: GithubIssueWorkflowRun,
        stage: GithubIssueStage,
        workspace_session: Option<GithubIssueWorkspaceSession>,
    ) -> Result<(GithubIssueWorkflowRun, WorkflowStepRun), GithubIssueWorkflowError> {
        let workspace_mount_ref = workspace_session
            .as_ref()
            .map(|session| session.mount_ref.clone());
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
        let prompt_run = run_with_workspace_session(&run, workspace_session.as_ref());
        let prompt = self.stage_prompt(&prompt_run, &stage, workspace_mount_ref.as_ref())?;
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
                prompt,
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

    fn stage_prompt(
        &self,
        run: &GithubIssueWorkflowRun,
        stage: &GithubIssueStage,
        workspace_mount_ref: Option<&WorkflowWorkspaceMountRef>,
    ) -> Result<WorkflowPromptContent, GithubIssueWorkflowError> {
        let snapshot = Self::workflow_stage_snapshot(run, stage, workspace_mount_ref);
        render_stage_prompt(stage.clone(), &snapshot).map(WorkflowPromptContent::from)
    }

    fn workflow_stage_snapshot(
        run: &GithubIssueWorkflowRun,
        stage: &GithubIssueStage,
        workspace_mount_ref: Option<&WorkflowWorkspaceMountRef>,
    ) -> EngineeredWorkflowSnapshot {
        let issue = &run.issue_ref;
        EngineeredWorkflowSnapshot {
            issue: GithubIssueSnapshot {
                owner: issue.owner.clone(),
                repo: issue.repo.clone(),
                number: issue.number,
                title: format!("{}/{}#{}", issue.owner, issue.repo, issue.number),
                url: issue.url.clone(),
                default_branch: issue.default_branch.clone(),
                state: "unknown".to_string(),
                labels: Vec::new(),
                summary: format!(
                    "GitHub issue {} is being processed by workflow run {}. Use scoped GitHub read capabilities for provider details not present in this engineered snapshot.",
                    issue.url, run.workflow_run_id
                ),
                provider_content_summaries: vec![ProviderContentSummary {
                    source_ref: format!(
                        "github:issue:{}/{}#{}",
                        issue.owner, issue.repo, issue.number
                    ),
                    author: None,
                    summary: "Workflow-owned issue reference metadata only; raw provider content is not embedded in this prompt.".to_string(),
                    trust: "workflow_metadata".to_string(),
                }],
            },
            workflow: WorkflowStateSnapshot {
                workflow_run_id: run.workflow_run_id.as_str().to_string(),
                workflow_policy_key: run.workflow_policy_key.clone(),
                workflow_policy_version: run.workflow_policy_version.clone(),
                status: Self::workflow_run_status_slug(&run.status).to_string(),
                mode: Self::workflow_mode_slug(&run.workflow_state.mode).to_string(),
                active_stage_run_id: run
                    .active_stage_run_id
                    .as_ref()
                    .map(|stage_run_id| stage_run_id.as_str().to_string()),
                event_cursor: run.event_cursor,
                workflow_run_version: run.workflow_run_version,
                active_block_summary: run
                    .workflow_state
                    .active_block
                    .as_ref()
                    .map(|block| block.reason.clone()),
                plan: run
                    .workflow_state
                    .plan
                    .iter()
                    .map(|item| {
                        format!(
                            "{} ({})",
                            item.title,
                            Self::plan_item_status_slug(&item.status)
                        )
                    })
                    .collect(),
            },
            repository: RepositorySnapshot {
                owner: issue.owner.clone(),
                name: issue.repo.clone(),
                default_branch: issue.default_branch.clone(),
                base_ref: Some(issue.default_branch.clone()),
                base_sha: None,
                working_branch: run
                    .workflow_state
                    .primary_pr
                    .as_ref()
                    .map(|pr| pr.head_branch.clone()),
                head_sha: run
                    .workflow_state
                    .primary_pr
                    .as_ref()
                    .and_then(|pr| pr.head_sha.clone()),
                primary_pr_url: run
                    .workflow_state
                    .primary_pr
                    .as_ref()
                    .map(|pr| pr.url.clone()),
            },
            previous_stage_results: Vec::new(),
            workspace: Self::workflow_workspace_snapshot(run, workspace_mount_ref),
            constraints: StageConstraintSnapshot {
                stage: stage.clone(),
                stage_goal: Self::stage_goal(stage).to_string(),
                allowed_capabilities: vec!["builtin.workflow_report_stage_result".to_string()],
                disallowed_capabilities: Vec::new(),
                result_schema_version: stage_result_schema_version(stage).to_string(),
                completion_tool: "builtin.workflow_report_stage_result".to_string(),
                provider_write_policy: "provider writes are performed by workflow-owned provider actions".to_string(),
            },
        }
    }

    fn workflow_workspace_snapshot(
        run: &GithubIssueWorkflowRun,
        workspace_mount_ref: Option<&WorkflowWorkspaceMountRef>,
    ) -> Option<WorkflowWorkspaceSnapshot> {
        let workspace_ref = run.workflow_state.current_workspace_ref.as_ref();
        let workspace_mount_ref =
            workspace_mount_ref.or(run.workflow_state.current_workspace_mount_ref.as_ref());
        if workspace_ref.is_none()
            && workspace_mount_ref.is_none()
            && run.workspace_session_id.is_none()
        {
            return None;
        }

        let mount_alias = workspace_mount_ref.map(|mount| mount.alias.clone());
        Some(WorkflowWorkspaceSnapshot {
            workspace_session_id: workspace_ref
                .and_then(|workspace| workspace.workspace_session_id.as_ref())
                .or(run.workspace_session_id.as_ref())
                .map(|workspace_session_id| workspace_session_id.as_str().to_string()),
            thread_id: workspace_ref
                .and_then(|workspace| workspace.thread_id.as_ref())
                .map(ToString::to_string),
            turn_run_id: workspace_ref
                .and_then(|workspace| workspace.turn_run_id.as_ref())
                .map(ToString::to_string),
            mount_alias: mount_alias.clone(),
            virtual_root: mount_alias.unwrap_or_else(|| "/workspace".to_string()),
            changed_files: Vec::new(),
        })
    }

    fn workflow_run_status_slug(status: &GithubIssueWorkflowRunStatus) -> &'static str {
        match status {
            GithubIssueWorkflowRunStatus::Active => "active",
            GithubIssueWorkflowRunStatus::Blocked => "blocked",
            GithubIssueWorkflowRunStatus::Succeeded => "succeeded",
            GithubIssueWorkflowRunStatus::Failed => "failed",
            GithubIssueWorkflowRunStatus::Cancelled => "cancelled",
        }
    }

    fn workflow_mode_slug(mode: &GithubIssueWorkflowMode) -> &'static str {
        match mode {
            GithubIssueWorkflowMode::New => "new",
            GithubIssueWorkflowMode::Claimed => "claimed",
            GithubIssueWorkflowMode::Triage => "triage",
            GithubIssueWorkflowMode::Planning => "planning",
            GithubIssueWorkflowMode::Implementation => "implementation",
            GithubIssueWorkflowMode::PrSynthesis => "pr_synthesis",
            GithubIssueWorkflowMode::PrOpen => "pr_open",
            GithubIssueWorkflowMode::CiRepair => "ci_repair",
            GithubIssueWorkflowMode::ReviewResponse => "review_response",
            GithubIssueWorkflowMode::Done => "done",
        }
    }

    fn plan_item_status_slug(status: &GithubIssuePlanItemStatus) -> &'static str {
        match status {
            GithubIssuePlanItemStatus::Pending => "pending",
            GithubIssuePlanItemStatus::InProgress => "in_progress",
            GithubIssuePlanItemStatus::Completed => "completed",
            GithubIssuePlanItemStatus::Skipped => "skipped",
        }
    }

    fn stage_goal(stage: &GithubIssueStage) -> &'static str {
        match stage {
            GithubIssueStage::Triage => {
                "Decide whether the GitHub issue is actionable and choose the next workflow stage."
            }
            GithubIssueStage::Planning => {
                "Produce a focused implementation plan and test strategy for the issue."
            }
            GithubIssueStage::Implementation => {
                "Implement the planned fix in the prepared workspace and report evidence."
            }
            GithubIssueStage::PrSynthesis => {
                "Prepare pull request synthesis details from the completed implementation."
            }
            GithubIssueStage::CiRepair => {
                "Diagnose failing checks and repair the implementation within workflow constraints."
            }
            GithubIssueStage::ReviewResponse => {
                "Address review feedback and report the validated response outcome."
            }
        }
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PrepareWorkflowWorkspaceOutcomeWire {
    Current {
        session: GithubIssueWorkspaceSession,
    },
    Legacy {
        workspace_session_id: GithubIssueWorkspaceSessionId,
        workspace_ref: WorkflowWorkspaceRef,
        mount_ref: WorkflowWorkspaceMountRef,
    },
}

fn decode_prepare_workspace_outcome(
    result: JsonValue,
    run: &GithubIssueWorkflowRun,
    created_at: DateTime<Utc>,
) -> Result<GithubIssueWorkspaceSession, GithubIssueWorkflowError> {
    let wire = serde_json::from_value::<PrepareWorkflowWorkspaceOutcomeWire>(result)
        .map_err(policy_serde_error)?;
    Ok(match wire {
        PrepareWorkflowWorkspaceOutcomeWire::Current { session } => session,
        PrepareWorkflowWorkspaceOutcomeWire::Legacy {
            workspace_session_id,
            workspace_ref,
            mount_ref,
        } => GithubIssueWorkspaceSession {
            workspace_session_id,
            workflow_run_id: run.workflow_run_id.clone(),
            repository: GithubRepositorySelector::new(
                run.issue_ref.owner.clone(),
                run.issue_ref.repo.clone(),
            )?,
            base_branch: run.issue_ref.default_branch.clone(),
            base_sha: None,
            working_branch: format!("ironclaw/github-bug/{}", run.workflow_run_id),
            current_head_sha: None,
            workspace_ref,
            mount_ref,
            created_at,
        },
    })
}

fn workspace_prepare_error_is_retryable(error: &GithubIssueWorkflowError) -> bool {
    matches!(
        error,
        GithubIssueWorkflowError::ProviderRead { .. }
            | GithubIssueWorkflowError::ProviderRateLimited { .. }
            | GithubIssueWorkflowError::Repository { .. }
    )
}

fn run_with_workspace_session(
    run: &GithubIssueWorkflowRun,
    workspace_session: Option<&GithubIssueWorkspaceSession>,
) -> GithubIssueWorkflowRun {
    let Some(workspace_session) = workspace_session else {
        return run.clone();
    };
    let mut run = run.clone();
    run.workspace_session_id = Some(workspace_session.workspace_session_id.clone());
    run.workflow_state.current_workspace_ref = Some(workspace_session.workspace_ref.clone());
    run.workflow_state.current_workspace_mount_ref = Some(workspace_session.mount_ref.clone());
    run
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
