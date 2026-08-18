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

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::ids::{AgentId, ProjectId, ThreadId};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope, TurnStatus};
use ironclaw_host_api::{output::OutputContract, prepared_context::PreparedTurnDeclarations};
use ironclaw_product_contracts::{inbound::ProductInboundAck, surface::ProductSurfaceCaller};
use ironclaw_threads::{
    FinalizedAssistantMessageByRunRequest, PreparedContextRequest,
    ReadStructuredFinalizationRequest, SessionThreadError, SessionThreadService, ThreadScope,
    agent_message::AgentMessage,
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
    pub caller: ProductSurfaceCaller,
    /// Public id doubling as the unbound thread id, exactly as the caller's
    /// retrieval path will use it.
    pub public_id: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    /// Optional visible-surface selection journaled with the declarations.
    /// Empty means no caller-selected tools.
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
    default_agent_id: AgentId,
    default_project_id: Option<ProjectId>,
}

impl UnboundTurnService {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        coordinator: Arc<dyn TurnCoordinator>,
        default_agent_id: AgentId,
        default_project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            thread_service,
            coordinator,
            default_agent_id,
            default_project_id,
        }
    }

    fn resolved_thread_scope(&self, caller: &ProductSurfaceCaller) -> ThreadScope {
        ThreadScope {
            tenant_id: caller.tenant_id.clone(),
            agent_id: caller
                .agent_id
                .clone()
                .unwrap_or_else(|| self.default_agent_id.clone()),
            project_id: caller
                .project_id
                .clone()
                .or_else(|| self.default_project_id.clone()),
            owner_user_id: Some(caller.user_id.clone()),
            mission_id: None,
        }
    }

    fn resolved_turn_scope(
        &self,
        thread_id: &ThreadId,
        caller: &ProductSurfaceCaller,
    ) -> TurnScope {
        let thread_scope = self.resolved_thread_scope(caller);
        TurnScope::new_with_owner(
            thread_scope.tenant_id,
            Some(thread_scope.agent_id),
            thread_scope.project_id,
            thread_id.clone(),
            Some(caller.user_id.clone()),
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
        let output_contract = submission.output.clone();
        let accepted = self
            .thread_service
            .accept_prepared_context(PreparedContextRequest {
                scope: self.resolved_thread_scope(&submission.caller),
                actor_id: submission.caller.user_id.as_str().to_string(),
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
                scope: self.resolved_turn_scope(&thread_id, &submission.caller),
                actor: TurnActor::new(submission.caller.user_id),
                accepted_message_ref: accepted.accepted_message_ref,
                requested_run_profile: None,
                output_contract: Some(output_contract),
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
        caller: &ProductSurfaceCaller,
        run_id: TurnRunId,
        poll_interval: Duration,
    ) -> Result<UnboundTurnOutcome, UnboundTurnError> {
        let thread_id = ThreadId::new(public_id.to_string())
            .map_err(|error| UnboundTurnError::internal(format!("invalid thread id: {error}")))?;
        let turn_scope = self.resolved_turn_scope(&thread_id, caller);
        let thread_scope = self.resolved_thread_scope(caller);
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
        let output_contract = self
            .thread_service
            .read_prepared_context(thread_scope, thread_id)
            .await
            .map_err(map_thread_read_error)?
            .map(|record| record.declarations.output);
        match output_contract {
            Some(contract) if contract.is_structured_output() => {
                let record = self
                    .thread_service
                    .read_structured_finalization(ReadStructuredFinalizationRequest {
                        scope: thread_scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: run_id,
                    })
                    .await
                    .map_err(map_thread_read_error)?
                    .ok_or_else(|| {
                        UnboundTurnError::internal(
                            "durable structured finalization record is missing",
                        )
                    })?;
                Ok(record.raw_json)
            }
            Some(OutputContract::AssistantMessage) => {
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
                message.content.ok_or_else(|| {
                    UnboundTurnError::internal("finalized assistant message has no content")
                })
            }
            None => Err(UnboundTurnError::internal(
                "prepared context record is missing for completed unbound run",
            )),
            Some(_) => Err(UnboundTurnError::internal(
                "completed unbound run has an unsupported output contract",
            )),
        }
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
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_host_api::turn::{TurnId, TurnRunId, TurnScope};
    use ironclaw_host_api::{output::OutputContract, prepared_context::PreparedTurnDeclarations};
    use ironclaw_threads::{
        AppendAssistantDraftRequest, InMemorySessionThreadService, MessageContent,
        PreparedContextRequest, PutStructuredFinalizationRequest, StructuredFinalizationAccounting,
        StructuredFinalizationRecord,
    };

    use super::*;

    fn scope() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::from_trusted("tenant-native".to_string()),
            UserId::from_trusted("user-native".to_string()),
            Some(AgentId::from_trusted("agent-native".to_string())),
            None,
        )
    }

    fn thread_scope(scope: &ProductSurfaceCaller) -> ThreadScope {
        ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone().expect("test agent"),
            project_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            mission_id: None,
        }
    }

    fn run_id() -> TurnRunId {
        TurnRunId::parse("11111111-1111-1111-1111-111111111111").expect("test run id")
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "integer" } },
            "required": ["value"],
            "additionalProperties": false
        })
    }

    #[test]
    fn unbound_scope_uses_caller_axes_and_only_defaults_missing_axes() {
        let service = UnboundTurnService::new(
            Arc::new(InMemorySessionThreadService::default()),
            Arc::new(StubTurnCoordinator::default()),
            AgentId::from_trusted("agent-default".to_string()),
            Some(ironclaw_host_api::ids::ProjectId::from_trusted(
                "project-default".to_string(),
            )),
        );
        let caller = ProductSurfaceCaller::new(
            TenantId::from_trusted("tenant-caller".to_string()),
            UserId::from_trusted("user-caller".to_string()),
            Some(AgentId::from_trusted("agent-caller".to_string())),
            Some(ironclaw_host_api::ids::ProjectId::from_trusted(
                "project-caller".to_string(),
            )),
        );
        let resolved = service.resolved_thread_scope(&caller);
        assert_eq!(resolved.tenant_id.as_str(), "tenant-caller");
        assert_eq!(resolved.agent_id.as_str(), "agent-caller");
        assert_eq!(
            resolved.project_id.as_ref().map(|id| id.as_str()),
            Some("project-caller")
        );
        assert_eq!(
            resolved.owner_user_id.as_ref().map(|id| id.as_str()),
            Some("user-caller")
        );

        let missing_axes = ProductSurfaceCaller::new(caller.tenant_id, caller.user_id, None, None);
        let resolved_defaults = service.resolved_thread_scope(&missing_axes);
        assert_eq!(resolved_defaults.agent_id.as_str(), "agent-default");
        assert_eq!(
            resolved_defaults.project_id.as_ref().map(|id| id.as_str()),
            Some("project-default")
        );
    }

    async fn readback_service(
        content: &str,
        output: OutputContract,
    ) -> (
        UnboundTurnService,
        ProductSurfaceCaller,
        ThreadId,
        TurnRunId,
    ) {
        let caller_scope = scope();
        let stored_scope = thread_scope(&caller_scope);
        let thread_id = ThreadId::from_trusted("native-readback-thread".to_string());
        let run_id = run_id();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let structured_output = output.is_structured_output();
        threads
            .accept_prepared_context(PreparedContextRequest {
                scope: stored_scope.clone(),
                actor_id: caller_scope.user_id.as_str().to_string(),
                system_prompt: "Return structured output.".to_string(),
                messages: vec![AgentMessage {
                    role: ironclaw_threads::agent_message::AgentMessageRole::User,
                    content: vec![ironclaw_threads::agent_message::ContentPart::text(
                        "Produce a value.",
                    )],
                }],
                declarations: PreparedTurnDeclarations {
                    tools: Vec::new(),
                    output,
                    limits: Default::default(),
                },
                idempotency_key: "native-readback-accept".to_string(),
                thread_id: thread_id.clone(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("prepared context accepted");
        let draft = threads
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: stored_scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text(content),
            })
            .await
            .expect("assistant draft appended");
        threads
            .finalize_assistant_message(
                &stored_scope,
                &thread_id,
                draft.message_id,
                MessageContent::text(content),
            )
            .await
            .expect("assistant message finalized");
        if structured_output && serde_json::from_str::<serde_json::Value>(content).is_ok() {
            threads
                .put_structured_finalization(PutStructuredFinalizationRequest {
                    record: StructuredFinalizationRecord {
                        scope: stored_scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_id: TurnId::new(),
                        turn_run_id: run_id,
                        contract_name: "test_output".to_string(),
                        schema_digest: "test-schema-digest".to_string(),
                        candidate: "test candidate".to_string(),
                        raw_json: content.to_string(),
                        accounting: StructuredFinalizationAccounting::default(),
                        owner_fence: "test-owner-fence".to_string(),
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    },
                })
                .await
                .expect("structured record persisted");
        }
        let service = UnboundTurnService::new(
            threads,
            Arc::new(StubTurnCoordinator::default()),
            AgentId::from_trusted("agent-default".to_string()),
            None,
        );
        (service, caller_scope, thread_id, run_id)
    }

    #[tokio::test]
    async fn native_structured_readback_preserves_provider_native_json() {
        let (service, caller_scope, thread_id, run_id) =
            readback_service(" { \"value\": 7 } ", OutputContract::json_schema(schema())).await;
        let result = service
            .resolve_completed_output(&thread_scope(&caller_scope), &thread_id, run_id)
            .await
            .expect("valid native output reads back");
        assert_eq!(result, " { \"value\": 7 } ");
    }

    #[tokio::test]
    async fn native_structured_readback_does_not_fall_back_to_transcript_scraping() {
        let (service, caller_scope, thread_id, run_id) =
            readback_service("not json", OutputContract::json_schema(schema())).await;
        let error = service
            .resolve_completed_output(&thread_scope(&caller_scope), &thread_id, run_id)
            .await
            .expect_err("a transcript row is not structured-output evidence");
        assert!(
            matches!(error, UnboundTurnError::Internal { reason } if reason.contains("record is missing"))
        );
    }

    #[tokio::test]
    async fn native_structured_readback_does_not_retry_or_revalidate_schema() {
        let (service, caller_scope, thread_id, run_id) = readback_service(
            r#"{"value":7}"#,
            OutputContract::json_schema(serde_json::json!({ "type": "not-a-json-schema-type" })),
        )
        .await;
        let result = service
            .resolve_completed_output(&thread_scope(&caller_scope), &thread_id, run_id)
            .await
            .expect("provider-native output is already schema-enforced");
        assert_eq!(result, r#"{"value":7}"#);
    }

    #[tokio::test]
    async fn assistant_readback_preserves_finalized_text() {
        let (service, caller_scope, thread_id, run_id) =
            readback_service("plain assistant reply", OutputContract::AssistantMessage).await;
        let result = service
            .resolve_completed_output(&thread_scope(&caller_scope), &thread_id, run_id)
            .await
            .expect("assistant output reads back");
        assert_eq!(result, "plain assistant reply");
    }

    #[derive(Default)]
    struct StubTurnCoordinator {
        submitted: Arc<Mutex<Option<SubmitTurnRequest>>>,
    }

    impl StubTurnCoordinator {
        fn captured_submission(&self) -> Option<SubmitTurnRequest> {
            self.submitted
                .lock()
                .expect("submission capture lock")
                .clone()
        }
    }

    #[async_trait]
    impl TurnCoordinator for StubTurnCoordinator {
        async fn prepare_turn(
            &self,
            _scope: TurnScope,
        ) -> Result<TurnRunId, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by native readback tests")
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, ironclaw_turns::TurnError> {
            let accepted_message_ref = request.accepted_message_ref.clone();
            *self.submitted.lock().expect("submission capture lock") = Some(request);
            Ok(SubmitTurnResponse::Accepted {
                turn_id: TurnId::new(),
                run_id: run_id(),
                status: TurnStatus::Queued,
                resolved_run_profile_id: ironclaw_turns::RunProfileId::unbound_default(),
                resolved_run_profile_version: ironclaw_turns::RunProfileVersion::new(1),
                event_cursor: ironclaw_turns::EventCursor(0),
                accepted_message_ref,
            })
        }

        async fn resume_turn(
            &self,
            _request: ironclaw_turns::ResumeTurnRequest,
        ) -> Result<ironclaw_turns::ResumeTurnResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by native readback tests")
        }

        async fn retry_turn(
            &self,
            _request: ironclaw_turns::RetryTurnRequest,
        ) -> Result<ironclaw_turns::RetryTurnResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by native readback tests")
        }

        async fn cancel_run(
            &self,
            _request: ironclaw_turns::CancelRunRequest,
        ) -> Result<ironclaw_turns::CancelRunResponse, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by native readback tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<ironclaw_turns::TurnRunState, ironclaw_turns::TurnError> {
            unimplemented!("not exercised by native readback tests")
        }
    }

    #[tokio::test]
    async fn accept_and_submit_forwards_declared_output_contract() {
        let caller_scope = scope();
        let threads = Arc::new(InMemorySessionThreadService::default());
        let coordinator = Arc::new(StubTurnCoordinator::default());
        let service = UnboundTurnService::new(
            threads,
            Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
            AgentId::from_trusted("agent-default".to_string()),
            None,
        );
        let output = OutputContract::try_json_schema("forwarded_output", schema()).expect("schema");
        service
            .accept_and_submit(UnboundTurnSubmission {
                caller: caller_scope,
                public_id: "forwarded-output-contract".to_string(),
                system_prompt: "Return one structured value.".to_string(),
                messages: vec![AgentMessage {
                    role: ironclaw_threads::agent_message::AgentMessageRole::User,
                    content: vec![ironclaw_threads::agent_message::ContentPart::text(
                        "produce a value",
                    )],
                }],
                tools: Vec::new(),
                output: output.clone(),
                requested_model: None,
                idempotency_key: "forwarded-output-contract-key".to_string(),
            })
            .await
            .expect("unbound submission");

        let captured = coordinator
            .captured_submission()
            .expect("coordinator captured submit");
        assert_eq!(captured.output_contract, Some(output));
    }
}
