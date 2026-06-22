use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_github_issue_workflow::{
    GithubIssueStage, GithubIssueWorkflowError, StageTurnSubmitter, SubmitStageTurnOutcome,
    SubmitStageTurnRequest, WorkflowActorScope,
};
use ironclaw_host_api::{AgentId, ThreadId, UserId};
use ironclaw_product_context::InboundClassification;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageStatus,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService, ThreadMessageId,
    ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, IdempotencyKey, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnScope, TurnSurfaceType,
};
use serde_json::json;

const WORKFLOW_ADAPTER_ID: &str = "github_issue_workflow";

pub(crate) struct IronClawStageTurnSubmitter {
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    actor_user_id: UserId,
    default_agent_id: AgentId,
}

impl IronClawStageTurnSubmitter {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        actor_user_id: UserId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            thread_service,
            turn_coordinator,
            actor_user_id,
            default_agent_id,
        }
    }

    fn thread_scope(&self, scope: &WorkflowActorScope) -> ThreadScope {
        ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.default_agent_id.clone()),
            project_id: scope.project_id.clone(),
            owner_user_id: Some(scope.creator_user_id.clone()),
            mission_id: None,
        }
    }

    fn actor_id(&self) -> String {
        self.actor_user_id.as_str().to_string()
    }
}

#[async_trait]
impl StageTurnSubmitter for IronClawStageTurnSubmitter {
    async fn submit_stage_turn(
        &self,
        request: SubmitStageTurnRequest,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
        let thread_scope = self.thread_scope(&request.scope);
        let thread_id = deterministic_stage_thread_id(&request)?;
        let source_binding_id = request.stage_turn_identity.source_binding_ref();
        let external_event_id = request.idempotency_key.as_str().to_string();

        let thread = self
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id),
                created_by_actor_id: self.actor_id(),
                title: Some(stage_thread_title(&request.stage_turn_identity.stage).to_string()),
                metadata_json: Some(stage_thread_metadata(&request)?),
            })
            .await
            .map_err(map_thread_error)?;

        if let Some(replay) = self
            .thread_service
            .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                scope: thread_scope.clone(),
                actor_id: self.actor_id(),
                source_binding_id: source_binding_id.clone(),
                external_event_id: external_event_id.clone(),
            })
            .await
            .map_err(map_thread_error)?
        {
            match replay.status {
                MessageStatus::Submitted => {
                    if let Some(turn_run_id) = replay.turn_run_id.as_deref() {
                        return Ok(SubmitStageTurnOutcome::Replayed {
                            thread_id: replay.thread_id,
                            turn_run_id: parse_turn_run_id(turn_run_id)?,
                        });
                    }
                    return Err(GithubIssueWorkflowError::Policy {
                        reason: "submitted stage turn message is missing turn_run_id".to_string(),
                    });
                }
                MessageStatus::RejectedBusy => {
                    return Ok(SubmitStageTurnOutcome::Busy {
                        reason:
                            "stage turn message was already rejected because the thread was busy"
                                .to_string(),
                    });
                }
                MessageStatus::Accepted => {
                    return self
                        .submit_accepted_message(
                            request,
                            thread_scope,
                            replay.thread_id,
                            replay.message_id,
                        )
                        .await;
                }
                _ => {}
            }
        }

        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id,
                actor_id: self.actor_id(),
                source_binding_id: Some(source_binding_id),
                reply_target_binding_id: Some(
                    request.stage_turn_identity.reply_target_binding_ref(),
                ),
                external_event_id: Some(external_event_id),
                content: MessageContent::text(request.prompt.content.clone()),
            })
            .await
            .map_err(map_thread_error)?;

        self.submit_accepted_message(
            request,
            thread_scope,
            accepted.thread_id,
            accepted.message_id,
        )
        .await
    }
}

impl IronClawStageTurnSubmitter {
    async fn submit_accepted_message(
        &self,
        request: SubmitStageTurnRequest,
        thread_scope: ThreadScope,
        thread_id: ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
        let turn_scope = TurnScope::new_with_owner(
            thread_scope.tenant_id.clone(),
            Some(thread_scope.agent_id.clone()),
            thread_scope.project_id.clone(),
            thread_id.clone(),
            thread_scope.owner_user_id.clone(),
        );
        let actor = TurnActor::new(self.actor_user_id.clone());
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let source_binding_ref =
            SourceBindingRef::new(request.stage_turn_identity.source_binding_ref())
                .map_err(invalid_ref)?;
        let reply_target_binding_ref =
            ReplyTargetBindingRef::new(request.stage_turn_identity.reply_target_binding_ref())
                .map_err(invalid_ref)?;
        let requested_run_profile =
            RunProfileRequest::new(request.capability_profile_id.clone()).map_err(invalid_ref)?;
        let idempotency_key = IdempotencyKey::new(request.idempotency_key.as_str().to_string())
            .map_err(invalid_ref)?;
        let product_context = workflow_product_context(&turn_scope, &actor)?;

        let submit_result = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: turn_scope,
                actor,
                accepted_message_ref,
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile: Some(requested_run_profile),
                idempotency_key,
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: Some(product_context),
            })
            .await;

        match submit_result {
            Ok(SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            }) => {
                self.thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &thread_id,
                        message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                    .map_err(map_thread_error)?;
                Ok(SubmitStageTurnOutcome::Submitted {
                    thread_id,
                    turn_run_id: run_id,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                self.thread_service
                    .mark_message_rejected_busy(&thread_scope, &thread_id, message_id)
                    .await
                    .map_err(map_thread_error)?;
                Ok(SubmitStageTurnOutcome::Busy {
                    reason: format!(
                        "thread already has active run {} with status {:?}",
                        busy.active_run_id, busy.status
                    ),
                })
            }
            Err(error) => Err(map_turn_error(error)),
        }
    }
}

fn deterministic_stage_thread_id(
    request: &SubmitStageTurnRequest,
) -> Result<ThreadId, GithubIssueWorkflowError> {
    ThreadId::new(request.stage_turn_identity.thread_id_seed()).map_err(|error| {
        GithubIssueWorkflowError::Policy {
            reason: format!("invalid deterministic stage thread id: {error}"),
        }
    })
}

fn accepted_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, GithubIssueWorkflowError> {
    AcceptedMessageRef::new(message_id.to_string()).map_err(invalid_ref)
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, GithubIssueWorkflowError> {
    TurnRunId::parse(value).map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("invalid replayed turn run id: {error}"),
    })
}

fn workflow_product_context(
    turn_scope: &TurnScope,
    actor: &TurnActor,
) -> Result<ProductTurnContext, GithubIssueWorkflowError> {
    let adapter = RunOriginAdapter::new(WORKFLOW_ADAPTER_ID).map_err(map_turn_error)?;
    Ok(ironclaw_product_context::resolve_inbound(
        InboundClassification::TrustedOther,
        adapter,
        Some(TurnSurfaceType::Direct),
        turn_scope.product_owner(actor),
    ))
}

fn stage_thread_title(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "GitHub issue workflow: triage",
        GithubIssueStage::Planning => "GitHub issue workflow: planning",
        GithubIssueStage::Implementation => "GitHub issue workflow: implementation",
        GithubIssueStage::PrSynthesis => "GitHub issue workflow: PR synthesis",
        GithubIssueStage::CiRepair => "GitHub issue workflow: CI repair",
        GithubIssueStage::ReviewResponse => "GitHub issue workflow: review response",
    }
}

fn stage_label(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage",
        GithubIssueStage::Planning => "planning",
        GithubIssueStage::Implementation => "implementation",
        GithubIssueStage::PrSynthesis => "pr_synthesis",
        GithubIssueStage::CiRepair => "ci_repair",
        GithubIssueStage::ReviewResponse => "review_response",
    }
}

fn stage_thread_metadata(
    request: &SubmitStageTurnRequest,
) -> Result<String, GithubIssueWorkflowError> {
    serde_json::to_string(&json!({
        "kind": "github_issue_workflow_stage",
        "workflow_run_id": request.stage_turn_identity.workflow_run_id.as_str(),
        "stage_run_id": request.stage_turn_identity.stage_run_id.as_str(),
        "stage": stage_label(&request.stage_turn_identity.stage),
        "attempt": request.stage_turn_identity.attempt,
        "workflow_policy_version": request.stage_turn_identity.workflow_policy_version.as_str(),
        "prompt_ref": request.prompt.content_ref.prompt_ref.as_str(),
        "prompt_version": request.prompt.content_ref.prompt_version.as_str(),
        "input_snapshot_hash": request.prompt.content_ref.input_snapshot_hash.as_str(),
        "prompt_content_hash": request.prompt.content_hash.as_str(),
    }))
    .map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("failed to serialize stage thread metadata: {error}"),
    })
}

fn map_thread_error(error: SessionThreadError) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("stage turn thread operation failed: {error}"),
    }
}

fn map_turn_error(error: TurnError) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("stage turn submit failed: {error}"),
    }
}

fn invalid_ref(error: impl std::fmt::Display) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: format!("invalid stage turn request reference: {error}"),
    }
}
