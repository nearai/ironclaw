use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, TenantId, ThreadId},
    output::OutputContract,
    turn::{EventCursor, TurnId, TurnLeaseToken, TurnRunId, TurnScope, TurnStatus},
};
use ironclaw_loop_contracts::{
    AssistantReply, InMemoryRunProfileResolver, LoopModelUsage, LoopRunContext,
    PromptContextTokenBudget, RunProfileResolutionRequest, RunProfileResolver,
    SystemInferenceError, SystemInferencePort, SystemInferenceRequest, SystemInferenceResponse,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, InMemorySessionThreadService, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, StructuredFinalizationAccounting, StructuredFinalizationRecord,
    SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord,
    ThreadScope, UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    AgentTurnRuntimePort, AgentTurnSpawnTreeRuntimePort, GetRunStateRequest, ResumeTurnRequest,
    RetryTurnRequest, SubmitChildRunRequest, SubmitTurnRequest, TurnAdmissionPolicy, TurnError,
    TurnRunProfile, TurnRunRecord, TurnRunState,
};

use super::{
    FINALIZATION_DEADLINE_MS, StructuredFinalizationContextLimits,
    StructuredFinalizationCoordinator, StructuredFinalizationPort, finalization_max_input_tokens,
    finalization_system_prompt, record_matches_replay,
};

#[test]
fn finalization_deadline_stays_below_the_process_lease() {
    let lease_ms = u64::try_from(ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION.as_millis())
        .expect("default process lease fits in u64 milliseconds");

    assert!(FINALIZATION_DEADLINE_MS < lease_ms);
}

#[test]
fn json_object_finalization_preserves_mode_without_a_synthetic_schema() {
    let contract = OutputContract::JsonObject;
    let prompt = super::finalization_system_prompt(&contract).expect("object prompt");
    assert!(prompt.contains("valid JSON object"));
    assert!(!prompt.contains("Declared output schema"));
    let identity = super::contract_identity(&contract).expect("object identity");
    assert_eq!(identity.0, "json_object");
    assert_eq!(
        identity.1,
        blake3::hash(b"json_object").to_hex().to_string()
    );
}

#[test]
fn successor_lease_can_adopt_matching_durable_finalization() {
    let record = StructuredFinalizationRecord {
        scope: ThreadScope {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            agent_id: AgentId::new("agent").expect("agent"),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        },
        thread_id: ThreadId::new("thread").expect("thread"),
        turn_id: TurnId::new(),
        turn_run_id: TurnRunId::new(),
        contract_name: "suggestions".to_string(),
        schema_digest: "schema-digest".to_string(),
        candidate: "ordinary terminal candidate".to_string(),
        raw_json: r#"{"suggestions":[]}"#.to_string(),
        accounting: StructuredFinalizationAccounting {
            usage: None,
            elapsed_ms: 1,
            model_profile_id: None,
            provider_id: None,
            model_id: None,
        },
        owner_fence: "predecessor-lease".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    assert!(record_matches_replay(
        &record,
        "ordinary terminal candidate",
        "suggestions",
        "schema-digest",
    ));
    assert!(!record_matches_replay(
        &record,
        "different candidate",
        "suggestions",
        "schema-digest",
    ));
}

#[derive(Clone)]
struct LeaseRuntime {
    scope: TurnScope,
    run_id: TurnRunId,
    lease_token: TurnLeaseToken,
    profile: TurnRunProfile,
}

impl LeaseRuntime {
    fn record(&self) -> TurnRunRecord {
        TurnRunRecord {
            subagent_activation_provenance: None,
            run_id: self.run_id,
            turn_id: TurnId::new(),
            scope: self.scope.clone(),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("accepted")
                .expect("accepted ref"),
            status: TurnStatus::Running,
            profile: self.profile.clone(),
            output_contract: OutputContract::json_schema(serde_json::json!({
                "type": "object"
            })),
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(0),
            runner_id: None,
            lease_token: Some(self.lease_token),
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 1,
            received_at: Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }
}

#[async_trait]
impl AgentTurnRuntimePort for LeaseRuntime {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<ironclaw_host_api::turn::SubmitTurnResponse, TurnError> {
        unreachable!("coordinator test runtime does not submit turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        unreachable!("coordinator test runtime does not resume turns")
    }

    async fn retry_turn(
        &self,
        _request: RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        unreachable!("coordinator test runtime does not retry turns")
    }

    async fn request_cancel(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        unreachable!("coordinator test runtime does not cancel turns")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("coordinator test runtime does not read run state")
    }
}

#[async_trait]
impl AgentTurnSpawnTreeRuntimePort for LeaseRuntime {
    async fn submit_child_turn(
        &self,
        _request: SubmitChildRunRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<ironclaw_host_api::turn::SubmitTurnResponse, TurnError> {
        unreachable!("coordinator test runtime does not submit child turns")
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        Ok((scope == &self.scope && run_id == self.run_id).then(|| self.record()))
    }

    async fn reserve_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _cap: u32,
    ) -> Result<ironclaw_turns::SpawnTreeReservation, TurnError> {
        unreachable!("coordinator test runtime does not reserve descendants")
    }

    async fn release_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }

    async fn prune_released_child(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }
}

struct CountingInference {
    calls: AtomicUsize,
    max_input_tokens: AtomicU64,
}

#[async_trait]
impl SystemInferencePort for CountingInference {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.max_input_tokens
            .store(request.max_input_tokens, Ordering::SeqCst);
        assert!(matches!(
            request.output_contract,
            Some(OutputContract::JsonSchema { .. })
        ));
        Ok(SystemInferenceResponse {
            task_id: request.task_id,
            output_text: r#"{"items":[]}"#.to_string(),
            elapsed_ms: 9,
            usage: Some(LoopModelUsage {
                input_tokens: 13,
                output_tokens: 5,
                cache_read_input_tokens: 2,
                cache_creation_input_tokens: 1,
            }),
        })
    }
}

/// Commits a competing record, then reports the CAS conflict that the
/// real backend would return to the losing writer. This keeps the test at
/// the caller's post-inference persistence seam instead of only testing
/// replay identity helpers or the read-side adoption branch.
struct ConflictInjectingThreadService {
    inner: Arc<InMemorySessionThreadService>,
    mismatch: bool,
    injected: AtomicUsize,
}

impl ConflictInjectingThreadService {
    fn new(inner: Arc<InMemorySessionThreadService>, mismatch: bool) -> Self {
        Self {
            inner,
            mismatch,
            injected: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl SessionThreadService for ConflictInjectingThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.inner.replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ironclaw_host_api::ids::ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_submitted(scope, thread_id, message_id, turn_id, turn_run_id)
            .await
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ironclaw_host_api::ids::ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_rejected_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_capability_display_preview(request).await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_tool_result_reference(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ironclaw_host_api::ids::ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.inner.load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.inner.load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.inner.create_summary_artifact(request).await
    }

    async fn read_structured_finalization(
        &self,
        request: ironclaw_threads::ReadStructuredFinalizationRequest,
    ) -> Result<Option<StructuredFinalizationRecord>, SessionThreadError> {
        self.inner.read_structured_finalization(request).await
    }

    async fn put_structured_finalization(
        &self,
        request: ironclaw_threads::PutStructuredFinalizationRequest,
    ) -> Result<StructuredFinalizationRecord, SessionThreadError> {
        if self.injected.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut competing = request.record.clone();
            if self.mismatch {
                competing.candidate = "competing candidate".to_string();
            }
            self.inner
                .put_structured_finalization(ironclaw_threads::PutStructuredFinalizationRequest {
                    record: competing,
                })
                .await?;
            return Err(SessionThreadError::StructuredFinalizationConflict {
                turn_run_id: request.record.turn_run_id,
            });
        }
        self.inner.put_structured_finalization(request).await
    }

    async fn publish_structured_finalization_message(
        &self,
        request: ironclaw_threads::PublishStructuredFinalizationMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .publish_structured_finalization_message(request)
            .await
    }
}

async fn run_post_inference_conflict_case(
    mismatch: bool,
) -> (
    Result<String, ironclaw_loop_contracts::AgentLoopHostError>,
    usize,
) {
    let tenant_id = TenantId::new("tenant-conflict-test").expect("tenant");
    let agent_id = AgentId::new("agent-conflict-test").expect("agent");
    let project_id =
        ironclaw_host_api::ids::ProjectId::new("project-conflict-test").expect("project");
    let user_id = ironclaw_host_api::ids::UserId::new("user-conflict-test").expect("user");
    let thread_id = ThreadId::new("thread-conflict-test").expect("thread");
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let turn_scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        Some(project_id),
        thread_id.clone(),
        Some(user_id.clone()),
    );
    let inner = Arc::new(InMemorySessionThreadService::default());
    inner
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    inner
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: user_id.as_str().to_string(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("seed context"),
        })
        .await
        .expect("seed message");

    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile");
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let run_context = LoopRunContext::new(turn_scope.clone(), turn_id, run_id, resolved.clone())
        .with_output_contract(
            OutputContract::try_json_schema(
                "test_output",
                serde_json::json!({"type": "object", "properties": {"items": {"type": "array"}}}),
            )
            .expect("schema"),
        );
    let profile = TurnRunProfile::from_resolved(resolved);
    let inference = Arc::new(CountingInference {
        calls: AtomicUsize::new(0),
        max_input_tokens: AtomicU64::new(0),
    });
    let lease = TurnLeaseToken::new();
    let runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort> = Arc::new(LeaseRuntime {
        scope: turn_scope,
        run_id,
        lease_token: lease,
        profile,
    });
    let service = Arc::new(ConflictInjectingThreadService::new(inner, mismatch));
    let coordinator = StructuredFinalizationCoordinator::new(
        service,
        thread_scope,
        run_context,
        Arc::clone(&inference) as Arc<dyn SystemInferencePort>,
        runtime,
        lease,
        StructuredFinalizationContextLimits {
            max_messages: 16,
            token_budget: PromptContextTokenBudget::default(),
        },
    );
    let result = coordinator
        .finalize_candidate(&AssistantReply {
            content: "ordinary candidate".to_string(),
        })
        .await;
    (result, inference.calls.load(Ordering::SeqCst))
}

#[tokio::test]
async fn post_inference_conflicts_adopt_matching_record_or_reject_mismatch() {
    let (matching, matching_calls) = run_post_inference_conflict_case(false).await;
    assert_eq!(
        matching.expect("matching conflict adopts record"),
        r#"{"items":[]}"#
    );
    assert_eq!(matching_calls, 1, "conflict recovery must not infer twice");

    let (mismatch, mismatch_calls) = run_post_inference_conflict_case(true).await;
    assert_eq!(
        mismatch
            .expect_err("mismatched conflict must fail closed")
            .kind,
        ironclaw_loop_contracts::AgentLoopHostErrorKind::TranscriptWriteFailed
    );
    assert_eq!(mismatch_calls, 1, "mismatch recovery must not infer twice");
}

#[tokio::test]
async fn coordinator_replays_adopts_and_rejects_conflicts_without_second_inference() {
    let tenant_id = TenantId::new("tenant-coordinator-test").expect("tenant");
    let agent_id = AgentId::new("agent-coordinator-test").expect("agent");
    let project_id =
        ironclaw_host_api::ids::ProjectId::new("project-coordinator-test").expect("project");
    let user_id = ironclaw_host_api::ids::UserId::new("user-coordinator-test").expect("user");
    let thread_id = ThreadId::new("thread-coordinator-test").expect("thread");
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let turn_scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        Some(project_id),
        thread_id.clone(),
        Some(user_id.clone()),
    );
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: user_id.as_str().to_string(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("seed context"),
        })
        .await
        .expect("seed message");

    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile");
    let run_id = TurnRunId::new();
    let run_context =
        LoopRunContext::new(turn_scope.clone(), TurnId::new(), run_id, resolved.clone())
            .with_output_contract(
            OutputContract::try_json_schema(
                "test_output",
                serde_json::json!({"type": "object", "properties": {"items": {"type": "array"}}}),
            )
            .expect("schema"),
        );
    let profile = TurnRunProfile::from_resolved(resolved);
    let inference = Arc::new(CountingInference {
        calls: AtomicUsize::new(0),
        max_input_tokens: AtomicU64::new(0),
    });
    let first_lease = TurnLeaseToken::new();
    let first_runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort> = Arc::new(LeaseRuntime {
        scope: turn_scope.clone(),
        run_id,
        lease_token: first_lease,
        profile: profile.clone(),
    });
    let first = StructuredFinalizationCoordinator::new(
        Arc::clone(&thread_service),
        thread_scope.clone(),
        run_context.clone(),
        Arc::clone(&inference) as Arc<dyn SystemInferencePort>,
        first_runtime,
        first_lease,
        StructuredFinalizationContextLimits {
            max_messages: 16,
            token_budget: PromptContextTokenBudget::default(),
        },
    );
    let candidate = AssistantReply {
        content: "ordinary candidate".to_string(),
    };
    assert_eq!(
        first
            .finalize_candidate(&candidate)
            .await
            .expect("first finalization"),
        r#"{"items":[]}"#
    );
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        first.supplemental_model_usage(),
        Some(LoopModelUsage {
            input_tokens: 13,
            output_tokens: 5,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 1,
        })
    );

    // A successor lease adopts the durable immutable record. It must not
    // issue a second logical inference, but must restore its usage
    // snapshot so the outer run exit still accounts for the work.
    let successor_lease = TurnLeaseToken::new();
    let successor: Arc<dyn AgentTurnSpawnTreeRuntimePort> = Arc::new(LeaseRuntime {
        scope: turn_scope.clone(),
        run_id,
        lease_token: successor_lease,
        profile: profile.clone(),
    });
    let adopted = StructuredFinalizationCoordinator::new(
        Arc::clone(&thread_service),
        thread_scope.clone(),
        run_context.clone(),
        Arc::clone(&inference) as Arc<dyn SystemInferencePort>,
        successor,
        successor_lease,
        StructuredFinalizationContextLimits {
            max_messages: 16,
            token_budget: PromptContextTokenBudget::default(),
        },
    );
    assert_eq!(
        adopted
            .finalize_candidate(&candidate)
            .await
            .expect("adopted finalization"),
        r#"{"items":[]}"#
    );
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        adopted.supplemental_model_usage(),
        first.supplemental_model_usage()
    );

    let conflict_lease = TurnLeaseToken::new();
    let conflicting = StructuredFinalizationCoordinator::new(
        Arc::clone(&thread_service),
        thread_scope,
        run_context,
        Arc::clone(&inference) as Arc<dyn SystemInferencePort>,
        Arc::new(LeaseRuntime {
            scope: turn_scope,
            run_id,
            lease_token: conflict_lease,
            profile,
        }),
        conflict_lease,
        StructuredFinalizationContextLimits {
            max_messages: 16,
            token_budget: PromptContextTokenBudget::default(),
        },
    );
    let error = conflicting
        .finalize_candidate(&AssistantReply {
            content: "different candidate".to_string(),
        })
        .await
        .expect_err("a mismatched immutable replay must fail closed");
    assert_eq!(
        error.kind,
        ironclaw_loop_contracts::AgentLoopHostErrorKind::TranscriptWriteFailed
    );
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn terminal_reply_finalization_reads_exact_row_and_publishes_idempotently() {
    let tenant_id = TenantId::new("tenant-terminal-finalization").expect("tenant");
    let agent_id = AgentId::new("agent-terminal-finalization").expect("agent");
    let user_id = ironclaw_host_api::ids::UserId::new("user-terminal-finalization").expect("user");
    let thread_id = ThreadId::new("thread-terminal-finalization").expect("thread");
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let turn_scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        None,
        thread_id.clone(),
        Some(user_id.clone()),
    );
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");

    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile");
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let run_context = LoopRunContext::new(turn_scope.clone(), turn_id, run_id, resolved.clone())
        .with_output_contract(
            OutputContract::try_json_schema(
                "terminal_output",
                serde_json::json!({"type": "object", "properties": {"items": {"type": "array"}}}),
            )
            .expect("schema"),
        );
    let finalized = thread_service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: run_id.to_string(),
            content: MessageContent::text("ordinary terminal candidate"),
        })
        .await
        .expect("draft");
    let finalized = thread_service
        .finalize_assistant_message(
            &thread_scope,
            &thread_id,
            finalized.message_id,
            MessageContent::text("ordinary terminal candidate"),
        )
        .await
        .expect("finalized assistant");
    let message_ref =
        ironclaw_host_api::turn::LoopMessageRef::new(format!("msg:{}", finalized.message_id))
            .expect("message ref");

    let inference = Arc::new(CountingInference {
        calls: AtomicUsize::new(0),
        max_input_tokens: AtomicU64::new(0),
    });
    let token_budget = PromptContextTokenBudget::new(1_024, 128, 0);
    let system_prompt = finalization_system_prompt(&run_context.output_contract)
        .expect("structured finalization prompt");
    let expected_max_input_tokens =
        finalization_max_input_tokens(token_budget.context_limit_tokens, &system_prompt);
    let profile = TurnRunProfile::from_resolved(resolved);
    let lease = TurnLeaseToken::new();
    let runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort> = Arc::new(LeaseRuntime {
        scope: turn_scope,
        run_id,
        lease_token: lease,
        profile,
    });
    let coordinator = StructuredFinalizationCoordinator::new(
        Arc::clone(&thread_service),
        thread_scope.clone(),
        run_context,
        Arc::clone(&inference) as Arc<dyn SystemInferencePort>,
        runtime,
        lease,
        StructuredFinalizationContextLimits {
            max_messages: 16,
            token_budget,
        },
    );

    coordinator
        .finalize_terminal_reply(&message_ref)
        .await
        .expect("terminal structured finalization");
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        inference.max_input_tokens.load(Ordering::SeqCst),
        expected_max_input_tokens
    );
    let published = thread_service
        .read_thread_message(&thread_scope, &thread_id, finalized.message_id)
        .await
        .expect("message read")
        .expect("message exists");
    assert_eq!(published.message_id, finalized.message_id);
    assert_eq!(published.content.as_deref(), Some(r#"{"items":[]}"#));

    // The durable record is replayed and the CAS publication is idempotent;
    // neither retry changes the row identity nor issues another inference.
    coordinator
        .finalize_terminal_reply(&message_ref)
        .await
        .expect("idempotent terminal publication");
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    let replayed = thread_service
        .read_thread_message(&thread_scope, &thread_id, finalized.message_id)
        .await
        .expect("message reread")
        .expect("message remains");
    assert_eq!(replayed.message_id, finalized.message_id);
    assert_eq!(replayed.content, published.content);
}
