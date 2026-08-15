//! Product orchestration for unbound prepared-turn submissions (unbound-turns
//! design §4): the accept-door + submit pair and the terminal
//! read-back that product surfaces (OpenAI-compat chat completions today)
//! delegate to. One service serves both halves of the lane so the accept axes
//! and the read-back axes can never drift: `accept_and_submit` seeds the
//! caller-authored context onto a caller-owned unbound thread (public id ==
//! thread id) and submits the unbound turn; `wait_for_completion` resolves the
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
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope, TurnStatus};
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
pub enum UnboundTurnError {
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
    /// payload). Not caller-correctable; the reason is for operators, never
    /// the wire.
    #[error("unbound turn internal error: {reason}")]
    Internal { reason: String },
}

/// Multiplier applied to the effective per-chunk `read_tool_result_record`
/// cap ([`effective_tool_result_read_max_bytes`]) to bound the TOTAL bytes
/// the structured-result paging loop will accumulate. Chosen as a few
/// chunks' worth of headroom for a model-produced JSON result — generous for
/// legitimate payloads, small enough that a misbehaving backend (or a
/// pagination bug) cannot grow the buffer without limit.
const STRUCTURED_RESULT_TOTAL_BYTES_FACTOR: u64 = 8;

impl UnboundTurnError {
    fn internal(reason: impl Into<String>) -> Self {
        Self::Internal {
            reason: reason.into(),
        }
    }
}

/// Terminal outcome of an unbound prepared turn: the output text plus the
/// run evidence a wire surface reports (the model that actually ran and the
/// provider-reported usage).
#[derive(Debug, Clone, PartialEq)]
pub struct UnboundTurnOutcome {
    pub text: String,
    /// From the run's resolved model route; `None` when no route evidence
    /// was persisted (replay stubs).
    pub effective_model: Option<String>,
    pub model_usage: Option<ironclaw_loop_contracts::LoopModelUsage>,
}

/// One prepared-turn submission in the engine vocabulary.
#[derive(Debug, Clone)]
pub struct UnboundTurnSubmission {
    pub actor_user_id: UserId,
    /// Public id doubling as the unbound thread id, exactly as the caller's
    /// retrieval path will use it.
    pub public_id: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    /// Visible-surface selection journaled with the declarations. Empty
    /// means "no tools".
    pub tools: Vec<ironclaw_host_api::ids::CapabilityId>,
    pub output: OutputContract,
    pub requested_model: Option<String>,
    /// Caller-owned idempotency key for the accept; the submit key derives
    /// from it (`{key}:submit`). The caller owns its namespace — this
    /// service serves any product surface, so it must not bake one in.
    pub idempotency_key: String,
}

/// Prepared-context door + unbound-run resolver over the SAME thread service
/// and coordinator the runtime's conversation path uses, scoped to the
/// deployment's default agent/project axes with the authenticated caller as
/// the thread owner. The owner keeps unbound threads sharded per-user (never
/// the tenant `__system__` slot) and lets the run-state scope check reject
/// cross-user reads; hiding from conversation listings comes from the
/// prepared-context stamp, not from ownerlessness.
pub struct UnboundTurnService {
    thread_service: Arc<dyn SessionThreadService>,
    coordinator: Arc<dyn TurnCoordinator>,
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

impl UnboundTurnService {
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

    fn thread_scope(&self, owner: &UserId) -> ThreadScope {
        ThreadScope {
            tenant_id: self.tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        }
    }

    fn turn_scope(&self, thread_id: &ThreadId, owner: &UserId) -> TurnScope {
        TurnScope::new_with_owner(
            self.tenant_id.clone(),
            Some(self.agent_id.clone()),
            self.project_id.clone(),
            thread_id.clone(),
            Some(owner.clone()),
        )
    }

    /// Accept the prepared context through the shared door and submit the
    /// unbound turn, both idempotent by the public id — a crash-retry returns
    /// the SAME run instead of minting an orphan. Returns the same
    /// `ProductInboundAck::Accepted` shape conversation submits produce so
    /// callers' replay machinery is shared.
    pub async fn accept_and_submit(
        &self,
        submission: UnboundTurnSubmission,
    ) -> Result<ProductInboundAck, UnboundTurnError> {
        let thread_id = ThreadId::new(submission.public_id.clone())
            .map_err(|error| UnboundTurnError::internal(format!("invalid thread id: {error}")))?;
        let accepted = self
            .thread_service
            .accept_prepared_context(PreparedContextRequest {
                scope: self.thread_scope(&submission.actor_user_id),
                actor_id: submission.actor_user_id.as_str().to_string(),
                system_prompt: submission.system_prompt,
                messages: submission.messages,
                declarations: PreparedTurnDeclarations {
                    tools: submission.tools,
                    output: submission.output,
                    limits: Default::default(),
                },
                idempotency_key: submission.idempotency_key.clone(),
                thread_id: thread_id.clone(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| match error {
                SessionThreadError::InvalidPreparedContext { reason } => {
                    UnboundTurnError::InvalidRequest { reason }
                }
                _ => {
                    tracing::debug!(%error, "unbound accept_prepared_context failed");
                    UnboundTurnError::Unavailable
                }
            })?;
        let response = self
            .coordinator
            .submit_turn(SubmitTurnRequest {
                scope: self.turn_scope(&thread_id, &submission.actor_user_id),
                actor: TurnActor::new(submission.actor_user_id),
                accepted_message_ref: accepted.accepted_message_ref,
                requested_run_profile: None,
                requested_model: submission.requested_model,
                idempotency_key: IdempotencyKey::new(format!(
                    "{}:submit",
                    submission.idempotency_key
                ))
                .map_err(|error| {
                    UnboundTurnError::internal(format!("invalid submit idempotency key: {error}"))
                })?,
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .map_err(|error| match error.category() {
                TurnErrorCategory::InvalidRequest => UnboundTurnError::InvalidRequest {
                    reason: error.to_string(),
                },
                _ => {
                    tracing::debug!(%error, "unbound submit_turn failed");
                    UnboundTurnError::Unavailable
                }
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
        actor_user_id: &UserId,
        run_id: TurnRunId,
        poll_interval: Duration,
    ) -> Result<UnboundTurnOutcome, UnboundTurnError> {
        let thread_id = ThreadId::new(public_id.to_string())
            .map_err(|error| UnboundTurnError::internal(format!("invalid thread id: {error}")))?;
        let turn_scope = self.turn_scope(&thread_id, actor_user_id);
        let thread_scope = self.thread_scope(actor_user_id);
        loop {
            let state = self
                .coordinator
                .get_run_state(GetRunStateRequest {
                    scope: turn_scope.clone(),
                    run_id,
                })
                .await
                .map_err(|error| {
                    tracing::debug!(%error, "unbound run-state read failed");
                    UnboundTurnError::Unavailable
                })?;
            match state.status {
                TurnStatus::Completed => {
                    let text = self
                        .resolve_completed_output(&thread_scope, &thread_id, run_id)
                        .await?;
                    return Ok(UnboundTurnOutcome {
                        text,
                        effective_model: state
                            .resolved_model_route
                            .as_ref()
                            .map(|route| route.model_id().to_string()),
                        model_usage: state.model_usage,
                    });
                }
                TurnStatus::Failed | TurnStatus::RecoveryRequired => {
                    return Err(UnboundTurnError::RunFailed {
                        category: state.failure.map(|failure| failure.category().to_string()),
                    });
                }
                TurnStatus::Cancelled => return Err(UnboundTurnError::RunCancelled),
                _ => tokio::time::sleep(poll_interval).await,
            }
        }
    }

    async fn resolve_completed_output(
        &self,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<String, UnboundTurnError> {
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
            .ok_or_else(|| UnboundTurnError::internal("expected row is missing"))?;
        Ok(message.content.unwrap_or_default())
    }

    /// The validated structured result: the run's own
    /// `builtin.structured_result` tool row, paged out of the durable
    /// tool-result record store in full.
    ///
    /// This locates the row by scanning the (bounded) unbound thread's
    /// history — acceptable for the polling read-back, and deliberately NOT
    /// generalized: the phase-2 run-observation façade's terminal event
    /// should carry the result ref and retire this scan. Do not add a second
    /// consumer.
    async fn structured_result_payload(
        &self,
        thread_scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<String, UnboundTurnError> {
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
            .ok_or_else(|| UnboundTurnError::internal("expected row is missing"))?;

        let per_chunk_bytes = effective_tool_result_read_max_bytes();
        // Bound the accumulated structured-result payload to a small multiple
        // of the effective per-chunk read cap. A structured result is a
        // model-produced JSON payload, not an arbitrary large blob, so a few
        // chunks' worth of headroom is generous while still preventing an
        // unbounded backend (or an offset that never advances) from growing
        // this buffer forever.
        let max_total_bytes =
            (per_chunk_bytes as u64).saturating_mul(STRUCTURED_RESULT_TOTAL_BYTES_FACTOR);
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
                    max_bytes: per_chunk_bytes,
                })
                .await
                .map_err(map_thread_read_error)?
                .ok_or_else(|| UnboundTurnError::internal("expected row is missing"))?;
            payload.extend_from_slice(&chunk.content);
            if payload.len() as u64 > max_total_bytes {
                return Err(UnboundTurnError::internal(format!(
                    "structured result payload exceeded the {max_total_bytes}-byte total cap"
                )));
            }
            match chunk.next_offset {
                Some(next) if next > offset => offset = next,
                Some(next) => {
                    return Err(UnboundTurnError::internal(format!(
                        "tool result record pagination did not advance: offset={offset}, next_offset={next}"
                    )));
                }
                None => break,
            }
        }
        String::from_utf8(payload).map_err(|error| {
            UnboundTurnError::internal(format!("stored result is not utf-8: {error}"))
        })
    }

    async fn load_provider_calls_for(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_ids: Vec<ThreadMessageId>,
    ) -> Result<HashMap<ThreadMessageId, ProviderToolCallReferenceEnvelope>, UnboundTurnError> {
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

fn map_thread_read_error(error: SessionThreadError) -> UnboundTurnError {
    match error {
        SessionThreadError::UnknownThread { .. } => {
            UnboundTurnError::internal("unbound thread is missing")
        }
        _ => {
            tracing::debug!(%error, "unbound thread read failed");
            UnboundTurnError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use ironclaw_host_api::ids::CapabilityId;
    use ironclaw_host_api::turn::{TurnRunId, TurnScope};
    use ironclaw_threads::{
        AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
        AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
        AppendToolResultReferenceRequest, ContextMessage, ContextMessages, ContextWindow,
        CreateSummaryArtifactRequest, EnsureThreadRequest, LoadContextWindowRequest,
        MessageContent, MessageStatus, RedactMessageRequest, ReplayAcceptedInboundMessageRequest,
        SessionThreadRecord, ThreadHistory, ToolResultRecordChunk, UpdateAssistantDraftRequest,
        UpdateToolResultReferenceRequest,
    };

    use super::*;

    fn tenant_id() -> TenantId {
        TenantId::from_trusted("tenant-1".to_string())
    }

    fn agent_id() -> AgentId {
        AgentId::from_trusted("agent-1".to_string())
    }

    fn owner_id() -> UserId {
        UserId::from_trusted("user-1".to_string())
    }

    fn thread_id() -> ThreadId {
        ThreadId::from_trusted("thread-1".to_string())
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: tenant_id(),
            agent_id: agent_id(),
            project_id: None,
            owner_user_id: Some(owner_id()),
            mission_id: None,
        }
    }

    fn structured_result_message() -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: thread_id(),
            sequence: 1,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            created_at: None,
            updated_at: None,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(RUN_REF.to_string()),
            tool_result_ref: Some("result-ref-1".to_string()),
            tool_result_provider_call: None,
            content: None,
            attachments: Vec::new(),
            redaction_ref: None,
        }
    }

    const RUN_REF: &str = "11111111-1111-1111-1111-111111111111";

    fn run_id() -> TurnRunId {
        TurnRunId::parse(RUN_REF).expect("valid run id")
    }

    /// Minimal `SessionThreadService` double that only implements the reads
    /// `structured_result_payload` exercises (`list_thread_history`,
    /// `load_context_messages`, `read_tool_result_record`); every other
    /// method panics because the paging-loop tests below never reach it.
    struct PagingStubThreadService {
        message: ThreadMessageRecord,
        /// Queue of `read_tool_result_record` outcomes, consumed in order.
        chunks: Mutex<Vec<Result<Option<ToolResultRecordChunk>, SessionThreadError>>>,
    }

    #[async_trait]
    impl SessionThreadService for PagingStubThreadService {
        async fn ensure_thread(
            &self,
            _request: EnsureThreadRequest,
        ) -> Result<SessionThreadRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn accept_inbound_message(
            &self,
            _request: AcceptInboundMessageRequest,
        ) -> Result<AcceptedInboundMessage, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn replay_accepted_inbound_message(
            &self,
            _request: ReplayAcceptedInboundMessageRequest,
        ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn mark_message_submitted(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ThreadMessageId,
            _turn_id: String,
            _turn_run_id: String,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn mark_message_rejected_busy(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ThreadMessageId,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn append_assistant_draft(
            &self,
            _request: AppendAssistantDraftRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn append_tool_result_reference(
            &self,
            _request: AppendToolResultReferenceRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn append_capability_display_preview(
            &self,
            _request: AppendCapabilityDisplayPreviewRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn update_tool_result_reference(
            &self,
            _request: UpdateToolResultReferenceRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn update_assistant_draft(
            &self,
            _request: UpdateAssistantDraftRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn finalize_assistant_message(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ThreadMessageId,
            _content: MessageContent,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn redact_message(
            &self,
            _request: RedactMessageRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn load_context_window(
            &self,
            _request: LoadContextWindowRequest,
        ) -> Result<ContextWindow, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn load_context_messages(
            &self,
            request: LoadContextMessagesRequest,
        ) -> Result<ContextMessages, SessionThreadError> {
            let capability_id =
                CapabilityId::new(STRUCTURED_RESULT_CAPABILITY_ID).expect("valid capability id");
            Ok(ContextMessages {
                thread_id: request.thread_id,
                messages: request
                    .message_ids
                    .into_iter()
                    .map(|message_id| ContextMessage {
                        message_id: Some(message_id),
                        summary_id: None,
                        sequence: 1,
                        kind: MessageKind::ToolResultReference,
                        tool_result_provider_call: Some(ProviderToolCallReferenceEnvelope {
                            provider_id: "test-provider".to_string(),
                            provider_model_id: "test-model".to_string(),
                            provider_turn_id: "turn-1".to_string(),
                            provider_call_id: "call-1".to_string(),
                            provider_tool_name: ironclaw_host_api::ids::ProviderToolName::new(
                                "structured_result",
                            )
                            .expect("valid provider tool name"),
                            capability_id: capability_id.clone(),
                            arguments: serde_json::Value::Null,
                            response_reasoning: None,
                            reasoning: None,
                            signature: None,
                        }),
                        content: String::new(),
                        image_attachments: Vec::new(),
                    })
                    .collect(),
            })
        }

        async fn list_thread_history(
            &self,
            request: ThreadHistoryRequest,
        ) -> Result<ThreadHistory, SessionThreadError> {
            Ok(ThreadHistory {
                thread: SessionThreadRecord {
                    scope: request.scope,
                    thread_id: request.thread_id,
                    created_by_actor_id: "actor-1".to_string(),
                    title: None,
                    metadata_json: None,
                    goal: None,
                    created_at: None,
                    updated_at: None,
                },
                messages: vec![self.message.clone()],
                summary_artifacts: Vec::new(),
            })
        }

        async fn read_tool_result_record(
            &self,
            _request: ReadToolResultRecordRequest,
        ) -> Result<Option<ToolResultRecordChunk>, SessionThreadError> {
            let mut chunks = self.chunks.lock().expect("chunk queue lock");
            if chunks.is_empty() {
                panic!(
                    "read_tool_result_record called more times than the test staged \
                     responses for — the paging loop must terminate on the guard \
                     conditions instead of looping forever"
                );
            }
            chunks.remove(0)
        }

        async fn create_summary_artifact(
            &self,
            _request: CreateSummaryArtifactRequest,
        ) -> Result<ironclaw_threads::SummaryArtifact, SessionThreadError> {
            unimplemented!("not exercised by structured-result paging tests")
        }
    }

    struct StubTurnCoordinator;

    #[async_trait]
    impl TurnCoordinator for StubTurnCoordinator {
        async fn prepare_turn(
            &self,
            _scope: TurnScope,
        ) -> Result<TurnRunId, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn resume_turn(
            &self,
            _request: ironclaw_turns::ResumeTurnRequest,
        ) -> Result<ironclaw_turns::ResumeTurnResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn retry_turn(
            &self,
            _request: ironclaw_turns::RetryTurnRequest,
        ) -> Result<ironclaw_turns::RetryTurnResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn cancel_run(
            &self,
            _request: ironclaw_turns::CancelRunRequest,
        ) -> Result<ironclaw_turns::CancelRunResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<ironclaw_turns::TurnRunState, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by structured-result paging tests")
        }
    }

    fn service_with_chunks(
        chunks: Vec<Result<Option<ToolResultRecordChunk>, SessionThreadError>>,
    ) -> UnboundTurnService {
        UnboundTurnService::new(
            Arc::new(PagingStubThreadService {
                message: structured_result_message(),
                chunks: Mutex::new(chunks),
            }),
            Arc::new(StubTurnCoordinator),
            tenant_id(),
            agent_id(),
            None,
        )
    }

    fn chunk(content: &[u8], next_offset: Option<u64>) -> ToolResultRecordChunk {
        ToolResultRecordChunk {
            content: content.to_vec(),
            total_bytes: content.len() as u64,
            next_offset,
        }
    }

    #[tokio::test]
    async fn structured_result_paging_rejects_non_advancing_offset() {
        // The first chunk reports `next_offset` equal to the offset it was
        // read at — a backend bug that would otherwise spin the loop
        // forever re-reading the same page.
        let service = service_with_chunks(vec![Ok(Some(chunk(b"{}", Some(0))))]);

        let error = service
            .structured_result_payload(&thread_scope(), &thread_id(), run_id())
            .await
            .expect_err("a non-advancing offset must be rejected");

        match error {
            UnboundTurnError::Internal { reason } => {
                assert!(
                    reason.contains("did not advance"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("expected Internal error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn structured_result_paging_rejects_oversized_total() {
        // Each chunk individually advances the offset and stays within the
        // per-chunk cap, but enough of them together exceed the total-bytes
        // bound the loop enforces.
        let per_chunk = effective_tool_result_read_max_bytes();
        let max_total = (per_chunk as u64) * STRUCTURED_RESULT_TOTAL_BYTES_FACTOR;
        let oversized_chunk_len = (max_total + 1) as usize;
        let content = vec![b'a'; oversized_chunk_len];

        let service = service_with_chunks(vec![Ok(Some(chunk(&content, Some(1))))]);

        let error = service
            .structured_result_payload(&thread_scope(), &thread_id(), run_id())
            .await
            .expect_err("an oversized total payload must be rejected");

        match error {
            UnboundTurnError::Internal { reason } => {
                assert!(reason.contains("total cap"), "unexpected reason: {reason}");
            }
            other => panic!("expected Internal error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn structured_result_paging_accumulates_across_pages() {
        // Sanity check that well-behaved, strictly-advancing pagination still
        // works and terminates on `next_offset: None`.
        let service = service_with_chunks(vec![
            Ok(Some(chunk(b"ab", Some(2)))),
            Ok(Some(chunk(b"cd", None))),
        ]);

        let payload = service
            .structured_result_payload(&thread_scope(), &thread_id(), run_id())
            .await
            .expect("well-behaved pagination succeeds");

        assert_eq!(payload, "abcd");
    }
}
