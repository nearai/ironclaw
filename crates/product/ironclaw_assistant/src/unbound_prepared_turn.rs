//! Product orchestration for unbound prepared-turn submissions (unbound-turns
//! design §4): the accept-door + refless-submit pair and the terminal
//! read-back that product surfaces (OpenAI-compat chat completions today)
//! delegate to. One service serves both halves of the lane so the accept axes
//! and the read-back axes can never drift: `accept_and_submit` seeds the
//! caller-authored context onto an ownerless unbound thread (public id ==
//! thread id) and submits reflessly; `wait_for_completion` resolves the
//! terminal outcome from run state plus the unbound thread's rows.
//!
//! Composition wires the service; route crates own only their wire DTOs and
//! map them onto the seed vocabulary re-exported by `ironclaw_threads`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::prepared_context::{
    OutputContract, PreparedTurnDeclarations, STRUCTURED_RESULT_CAPABILITY_ID,
};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope, TurnStatus, TurnThreadOwner};
use ironclaw_product_contracts::inbound::ProductInboundAck;
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, LoadContextMessagesRequest, MessageKind,
    PreparedContextRequest, ProviderToolCallReferenceEnvelope, ReadToolResultRecordRequest,
    SessionThreadError, SessionThreadService, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, agent_message::AgentMessage,
    effective_tool_result_read_max_bytes,
};
use ironclaw_turns::{
    GetRunStateRequest, IdempotencyKey, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator,
    TurnErrorCategory,
};

/// Typed failure surface for the unbound prepared-turn lane. Route adapters
/// map these onto their own wire errors; the categories match the decisions
/// a caller can act on (fix the request, retry, give up).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnboundPreparedTurnError {
    /// The submission itself is invalid (bad seed content, bad public id).
    #[error("invalid unbound prepared turn submission: {reason}")]
    InvalidRequest { reason: String },
    /// The backing services are temporarily unavailable; retryable.
    #[error("unbound prepared turn services are unavailable")]
    Unavailable,
    /// The run reached a terminal failure.
    #[error("unbound run failed")]
    RunFailed { category: Option<String> },
    /// The run was cancelled before completing.
    #[error("unbound run was cancelled")]
    RunCancelled,
    /// An invariant the lane relies on did not hold (missing rows, undecodable
    /// payload). Not caller-correctable.
    #[error("unbound prepared turn internal error")]
    Internal,
}

/// Terminal outcome of an unbound prepared turn: the output text plus the
/// run evidence a wire surface reports (the model that actually ran and the
/// provider-reported usage).
#[derive(Debug, Clone, PartialEq)]
pub struct UnboundCompletionOutcome {
    pub text: String,
    /// From the run's resolved model route; `None` when no route evidence
    /// was persisted (replay stubs).
    pub effective_model: Option<String>,
    pub model_usage: Option<ironclaw_loop_contracts::LoopModelUsage>,
}

/// One prepared-turn submission in the engine vocabulary.
#[derive(Debug, Clone)]
pub struct UnboundPreparedTurnSubmission {
    pub actor_user_id: UserId,
    /// Public id doubling as the unbound thread id, exactly as the caller's
    /// retrieval path will use it.
    pub public_id: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub output: OutputContract,
    pub requested_model: Option<String>,
}

/// Prepared-context door + unbound-run resolver over the SAME thread service
/// and coordinator the runtime's conversation path uses, scoped to the
/// deployment's default agent/project axes with the ownerless unbound owner.
pub struct UnboundPreparedTurnService {
    thread_service: Arc<dyn SessionThreadService>,
    coordinator: Arc<dyn TurnCoordinator>,
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

impl UnboundPreparedTurnService {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        coordinator: Arc<dyn TurnCoordinator>,
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            thread_service,
            coordinator,
            tenant_id,
            agent_id,
            project_id,
        }
    }

    fn thread_scope(&self) -> ThreadScope {
        ThreadScope {
            tenant_id: self.tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            owner_user_id: None,
            mission_id: None,
        }
    }

    fn turn_scope(&self, thread_id: &ThreadId) -> TurnScope {
        let mut scope = TurnScope::new(
            self.tenant_id.clone(),
            Some(self.agent_id.clone()),
            self.project_id.clone(),
            thread_id.clone(),
        );
        scope.thread_owner = TurnThreadOwner::Ownerless;
        scope
    }

    /// Accept the prepared context through the shared door and submit the
    /// refless turn, both idempotent by the public id — a crash-retry returns
    /// the SAME run instead of minting an orphan. Returns the same
    /// `ProductInboundAck::Accepted` shape conversation submits produce so
    /// callers' replay machinery is shared.
    pub async fn accept_and_submit(
        &self,
        submission: UnboundPreparedTurnSubmission,
    ) -> Result<ProductInboundAck, UnboundPreparedTurnError> {
        let thread_id = ThreadId::new(submission.public_id.clone())
            .map_err(|_| UnboundPreparedTurnError::Internal)?;
        let accepted = self
            .thread_service
            .accept_prepared_context(PreparedContextRequest {
                scope: self.thread_scope(),
                actor_id: submission.actor_user_id.as_str().to_string(),
                system_prompt: submission.system_prompt,
                messages: submission.messages,
                declarations: PreparedTurnDeclarations {
                    tools: Vec::new(),
                    output: submission.output,
                    limits: Default::default(),
                },
                idempotency_key: format!("openai-chat:{}", submission.public_id),
                thread_id: Some(thread_id.clone()),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| match error {
                SessionThreadError::InvalidPreparedContext { reason } => {
                    UnboundPreparedTurnError::InvalidRequest { reason }
                }
                _ => UnboundPreparedTurnError::Unavailable,
            })?;
        let response = self
            .coordinator
            .submit_turn(SubmitTurnRequest {
                scope: self.turn_scope(&thread_id),
                actor: TurnActor::new(submission.actor_user_id),
                accepted_message_ref: accepted.accepted_message_ref,
                requested_run_profile: None,
                requested_model: submission.requested_model,
                idempotency_key: IdempotencyKey::new(format!(
                    "openai-chat-submit:{}",
                    submission.public_id
                ))
                .map_err(|_| UnboundPreparedTurnError::Internal)?,
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .map_err(|error| match error.category() {
                TurnErrorCategory::InvalidRequest => UnboundPreparedTurnError::InvalidRequest {
                    reason: error.to_string(),
                },
                _ => UnboundPreparedTurnError::Unavailable,
            })?;
        let SubmitTurnResponse::Accepted {
            run_id,
            accepted_message_ref,
            ..
        } = response;
        Ok(ProductInboundAck::Accepted {
            accepted_message_ref,
            submitted_run_id: run_id,
            submission: None,
        })
    }

    /// Resolve one terminal unbound run into its output text: structured runs
    /// return the validated result payload; assistant runs return the
    /// finalized reply text. Polls run state until terminal.
    pub async fn wait_for_completion(
        &self,
        public_id: &str,
        run_id: TurnRunId,
        poll_interval: Duration,
    ) -> Result<UnboundCompletionOutcome, UnboundPreparedTurnError> {
        let thread_id =
            ThreadId::new(public_id.to_string()).map_err(|_| UnboundPreparedTurnError::Internal)?;
        let turn_scope = self.turn_scope(&thread_id);
        let thread_scope = self.thread_scope();
        loop {
            let state = self
                .coordinator
                .get_run_state(GetRunStateRequest {
                    scope: turn_scope.clone(),
                    run_id,
                })
                .await
                .map_err(|_| UnboundPreparedTurnError::Unavailable)?;
            match state.status {
                TurnStatus::Completed => {
                    let text = self
                        .resolve_completed_output(&thread_scope, &thread_id, run_id)
                        .await?;
                    return Ok(UnboundCompletionOutcome {
                        text,
                        effective_model: state
                            .resolved_model_route
                            .as_ref()
                            .map(|route| route.model_id().to_string()),
                        model_usage: state.model_usage,
                    });
                }
                TurnStatus::Failed | TurnStatus::RecoveryRequired => {
                    return Err(UnboundPreparedTurnError::RunFailed {
                        category: state.failure.map(|failure| failure.category().to_string()),
                    });
                }
                TurnStatus::Cancelled => return Err(UnboundPreparedTurnError::RunCancelled),
                _ => tokio::time::sleep(poll_interval).await,
            }
        }
    }

    async fn resolve_completed_output(
        &self,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<String, UnboundPreparedTurnError> {
        let structured = matches!(
            self.thread_service
                .read_prepared_context(thread_scope, thread_id)
                .await
                .map_err(map_thread_read_error)?
                .map(|record| record.declarations.output),
            Some(OutputContract::JsonSchema { .. })
        );
        if structured {
            return self
                .structured_result_payload(thread_scope, thread_id, run_id)
                .await;
        }
        let message = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: run_id.to_string(),
            })
            .await
            .map_err(map_thread_read_error)?
            .ok_or(UnboundPreparedTurnError::Internal)?;
        Ok(message.content.unwrap_or_default())
    }

    /// The validated structured result: the run's own
    /// `builtin.structured_result` tool row, paged out of the durable
    /// tool-result record store in full.
    async fn structured_result_payload(
        &self,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<String, UnboundPreparedTurnError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(map_thread_read_error)?;
        let run_ref = run_id.to_string();
        let candidates: Vec<&ThreadMessageRecord> = history
            .messages
            .iter()
            .filter(|message| {
                message.kind == MessageKind::ToolResultReference
                    && message.turn_run_id.as_deref() == Some(run_ref.as_str())
                    && message.tool_result_ref.is_some()
            })
            .collect();
        let provider_calls = self
            .load_provider_calls_for(
                thread_scope,
                thread_id,
                candidates
                    .iter()
                    .map(|message| message.message_id)
                    .collect(),
            )
            .await?;
        let result_ref = candidates
            .iter()
            .find(|message| {
                provider_calls.get(&message.message_id).is_some_and(|call| {
                    call.capability_id.as_str() == STRUCTURED_RESULT_CAPABILITY_ID
                })
            })
            .and_then(|message| message.tool_result_ref.clone())
            .ok_or(UnboundPreparedTurnError::Internal)?;

        let mut payload = Vec::new();
        let mut offset = 0u64;
        loop {
            let chunk = self
                .thread_service
                .read_tool_result_record(ReadToolResultRecordRequest {
                    scope: thread_scope.clone(),
                    thread_id: thread_id.clone(),
                    result_ref: result_ref.clone(),
                    offset,
                    max_bytes: effective_tool_result_read_max_bytes(),
                })
                .await
                .map_err(map_thread_read_error)?
                .ok_or(UnboundPreparedTurnError::Internal)?;
            payload.extend_from_slice(&chunk.content);
            match chunk.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        String::from_utf8(payload).map_err(|_| UnboundPreparedTurnError::Internal)
    }

    async fn load_provider_calls_for(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_ids: Vec<ThreadMessageId>,
    ) -> Result<HashMap<ThreadMessageId, ProviderToolCallReferenceEnvelope>, UnboundPreparedTurnError>
    {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let context = self
            .thread_service
            .load_context_messages(LoadContextMessagesRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                message_ids,
            })
            .await
            .map_err(map_thread_read_error)?;
        Ok(context
            .messages
            .into_iter()
            .filter_map(|message| Some((message.message_id?, message.tool_result_provider_call?)))
            .collect())
    }
}

fn map_thread_read_error(error: SessionThreadError) -> UnboundPreparedTurnError {
    match error {
        SessionThreadError::UnknownThread { .. } => UnboundPreparedTurnError::Internal,
        _ => UnboundPreparedTurnError::Unavailable,
    }
}
