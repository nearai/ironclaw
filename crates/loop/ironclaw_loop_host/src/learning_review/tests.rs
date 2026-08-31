use super::*;
use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_memory::{
    LearningAction, LearningCandidateInsert, LearningCandidateStoreError, LearningDecision,
    LearningExplicitness, MemoryLearningProposal, MemoryLearningProposalKind,
};
use ironclaw_product_contracts::operator_llm::MemoryWritePolicy;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendToolResultReferenceRequest, EnsureThreadRequest,
    InMemorySessionThreadService, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, ToolResultSafeSummary, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{EventCursor, TurnScope, TurnStatus};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Barrier, Notify, watch};

#[test]
fn parser_accepts_only_valid_bounded_reviews() {
    let review = LearningReview {
        memory: vec![MemoryLearningProposal {
            kind: MemoryLearningProposalKind::Fact,
            content: "The project uses Rust".to_string(),
            source_message_indices: vec![0],
            confidence_basis_points: 8_000,
            explicitness: LearningExplicitness::Explicit,
            tainted: false,
        }],
        skill: LearningDecision {
            action: LearningAction::Skip,
            reason: None,
            source_message_indices: Vec::new(),
            tainted: false,
        },
    };
    let json = serde_json::to_string(&review).expect("serialize");
    assert_eq!(parse_review(&json).expect("parse"), review);
    assert!(parse_review("```json\n{}\n```").is_err());
}

#[test]
fn host_rejects_unknown_model_source_references() {
    let review = LearningReview {
        memory: vec![MemoryLearningProposal {
            kind: MemoryLearningProposalKind::Fact,
            content: "Unsupported claim".to_string(),
            source_message_indices: vec![1],

            confidence_basis_points: 8_000,
            explicitness: LearningExplicitness::Inferred,
            tainted: false,
        }],
        skill: LearningDecision::skip(),
    };
    let transcript = FormattedTranscript {
        content: "[0] user: hello".to_string(),
        source_indices: BTreeSet::from([0]),
        tainted_indices: BTreeSet::new(),
    };
    assert!(seal_review_sources(review, &transcript).is_err());
}

#[test]
fn parser_rejects_oversized_provider_output_before_json_parse() {
    assert_eq!(
        parse_review(&"x".repeat(LEARNING_REVIEW_OUTPUT_MAX_BYTES + 1)),
        Err("output too large")
    );
}

#[test]
fn host_rejects_secret_bearing_candidate_text() {
    let token = format!("ghp_{}", "x".repeat(36));
    let review = LearningReview {
        memory: vec![MemoryLearningProposal {
            kind: MemoryLearningProposalKind::Fact,
            content: token,
            source_message_indices: vec![0],
            confidence_basis_points: 8_000,
            explicitness: LearningExplicitness::Explicit,
            tainted: false,
        }],
        skill: LearningDecision::skip(),
    };
    assert_eq!(
        reject_secret_bearing_candidates(review),
        Err("secret detected in learning candidate")
    );
}

#[test]
fn host_taints_skill_decisions_when_transcript_contains_untrusted_data() {
    let review = LearningReview {
        memory: Vec::new(),
        skill: LearningDecision {
            action: LearningAction::Distill,
            reason: Some("procedure".to_string()),
            source_message_indices: vec![0],
            tainted: false,
        },
    };
    let transcript = FormattedTranscript {
        content: "[0] system: child".to_string(),
        source_indices: BTreeSet::from([0]),
        tainted_indices: BTreeSet::from([0]),
    };
    let sealed = seal_review_sources(review, &transcript).expect("seal");
    assert!(sealed.skill.tainted);
}
#[test]
fn transcript_budget_keeps_only_complete_lines() {
    let thread_id = ThreadId::new("learning-thread").expect("thread");
    let message = |sequence, content| ThreadMessageRecord {
        message_id: ironclaw_threads::ThreadMessageId::new(),
        thread_id: thread_id.clone(),
        sequence,
        kind: MessageKind::User,
        status: ironclaw_threads::MessageStatus::Accepted,
        created_at: None,
        updated_at: None,
        actor_id: None,
        source_binding_id: None,
        reply_target_binding_id: None,
        turn_id: None,
        turn_run_id: None,
        tool_result_ref: None,
        tool_result_provider_call: None,
        content: Some(content),
        attachments: Vec::new(),
        redaction_ref: None,
    };
    let transcript = format_transcript(&[
        message(1, "first".to_string()),
        message(2, "x".repeat(TRANSCRIPT_MAX_BYTES)),
    ]);
    assert_eq!(transcript.content, "[0] user: first\n");
    assert_eq!(transcript.source_indices, BTreeSet::from([0]));
}

struct RecordingInference {
    calls: AtomicUsize,
    users: Mutex<Vec<String>>,
    output: String,
}

#[async_trait]
impl LearningInferencePort for RecordingInference {
    async fn infer(&self, _system: &str, user: &str) -> Result<String, LearningInferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.users
            .lock()
            .expect("users lock")
            .push(user.to_string());
        Ok(self.output.clone())
    }
}
struct LimitExceededThreadService;

fn unused_thread_service_method() -> SessionThreadError {
    SessionThreadError::Backend("unused test double method".to_string())
}

#[async_trait]
impl SessionThreadService for LimitExceededThreadService {
    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<ironclaw_threads::AcceptedInboundMessageReplay>, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ironclaw_host_api::ids::ThreadId,
        _message_id: ironclaw_threads::ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ironclaw_host_api::ids::ThreadId,
        _message_id: ironclaw_threads::ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ironclaw_host_api::ids::ThreadId,
        _message_id: ironclaw_threads::ThreadMessageId,
        _content: ironclaw_threads::MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn redact_message(
        &self,
        _request: ironclaw_threads::RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn load_context_window(
        &self,
        _request: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ironclaw_threads::ContextWindow, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn load_context_messages(
        &self,
        _request: ironclaw_threads::LoadContextMessagesRequest,
    ) -> Result<ironclaw_threads::ContextMessages, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn list_thread_history(
        &self,
        _request: ironclaw_threads::ThreadHistoryRequest,
    ) -> Result<ironclaw_threads::ThreadHistory, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn create_summary_artifact(
        &self,
        _request: ironclaw_threads::CreateSummaryArtifactRequest,
    ) -> Result<ironclaw_threads::SummaryArtifact, SessionThreadError> {
        Err(unused_thread_service_method())
    }

    async fn list_completed_run_messages_bounded(
        &self,
        _request: CompletedRunMessagesRequest,
    ) -> Result<CompletedRunMessages, SessionThreadError> {
        Ok(CompletedRunMessages::LimitExceeded)
    }
}

struct ParkingInference {
    calls: AtomicUsize,
    parked: Arc<Barrier>,
    release: watch::Sender<bool>,
}

#[async_trait]
impl LearningInferencePort for ParkingInference {
    async fn infer(&self, _system: &str, _user: &str) -> Result<String, LearningInferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.parked.wait().await;
        let mut released = self.release.subscribe();
        while !*released.borrow() {
            released
                .changed()
                .await
                .map_err(|_| LearningInferenceError("release dropped".to_string()))?;
        }
        Ok(r#"{"memory":[],"skill":{"action":"skip"}}"#.to_string())
    }
}

#[derive(Default)]
struct RecordingCandidateStore {
    records: Mutex<Vec<LearningReviewRecord>>,
    inserted: Notify,
}

#[async_trait]
impl LearningCandidateStore for RecordingCandidateStore {
    async fn insert_if_absent(
        &self,
        record: &LearningReviewRecord,
    ) -> Result<LearningCandidateInsert, LearningCandidateStoreError> {
        self.records
            .lock()
            .expect("records lock")
            .push(record.clone());
        self.inserted.notify_one();
        Ok(LearningCandidateInsert::Created)
    }

    async fn get(
        &self,
        scope: &LearningScope,
        run_id: TurnRunId,
    ) -> Result<Option<LearningReviewRecord>, LearningCandidateStoreError> {
        Ok(self
            .records
            .lock()
            .expect("records lock")
            .iter()
            .find(|record| &record.scope == scope && record.run_id == run_id)
            .cloned())
    }

    async fn list_unresolved(
        &self,
        scope: &LearningScope,
    ) -> Result<Vec<LearningReviewRecord>, LearningCandidateStoreError> {
        Ok(self
            .records
            .lock()
            .expect("records lock")
            .iter()
            .filter(|record| &record.scope == scope)
            .take(MAX_LEARNING_UNRESOLVED_PROPOSALS as usize)
            .cloned()
            .collect())
    }
}

#[tokio::test]
async fn saturated_learning_review_tasks_drop_extra_runs() {
    let tenant_id = TenantId::new("learning-tenant").expect("tenant");
    let user_id = UserId::new("learning-user").expect("user");
    let agent_id = AgentId::new("learning-agent").expect("agent");
    let thread_id = ThreadId::new("learning-thread").expect("thread");
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let learning_scope =
        LearningScope::new(tenant_id.clone(), user_id.clone(), agent_id.clone(), None);
    let threads = Arc::new(InMemorySessionThreadService::default());
    threads
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");

    let run_ids = (0..=MAX_CONCURRENT_REVIEWS)
        .map(|_| TurnRunId::new())
        .collect::<Vec<_>>();
    for (index, run_id) in run_ids.iter().enumerate() {
        threads
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                intrinsic_outcome: None,
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: run_id.to_string(),
                result_ref: format!("result:learning-{index}"),
                safe_summary: ToolResultSafeSummary::new("bounded").expect("summary"),
                provider_call: None,
                model_observation: None,
            })
            .await
            .expect("append result");
    }

    let parked = Arc::new(Barrier::new(MAX_CONCURRENT_REVIEWS + 1));
    let (release, _release_guard) = watch::channel(false);
    let inference = Arc::new(ParkingInference {
        calls: AtomicUsize::new(0),
        parked: Arc::clone(&parked),
        release,
    });
    let store = Arc::new(RecordingCandidateStore::default());
    let controller = Arc::new(LearningRuntimeControllerImpl::new(LearningSettings {
        enabled: true,
        model: Some("learning-model".to_string()),
        memory_write_policy: MemoryWritePolicy::Staged,
    }));
    let tasks = LearningReviewTasks::new();
    let make_job = |run_id| LearningReviewJob {
        thread_service: threads.clone(),
        inference: inference.clone(),
        candidate_store: store.clone(),
        controller: controller.clone(),
        thread_scope: thread_scope.clone(),
        thread_id: thread_id.clone(),
        run_id,
        learning_scope: learning_scope.clone(),
    };

    for run_id in run_ids.iter().take(MAX_CONCURRENT_REVIEWS) {
        tasks.spawn(make_job(*run_id));
    }
    parked.wait().await;

    let extra_run_id = run_ids[MAX_CONCURRENT_REVIEWS];
    tasks.spawn(make_job(extra_run_id));
    assert_eq!(
        inference.calls.load(Ordering::SeqCst),
        MAX_CONCURRENT_REVIEWS
    );
    assert!(
        store.records.lock().expect("records lock").is_empty(),
        "the saturated run must not persist a candidate"
    );

    inference
        .release
        .send(true)
        .expect("release parked reviews");
    tasks.wait().await;

    let records = store.records.lock().expect("records lock");
    assert_eq!(records.len(), MAX_CONCURRENT_REVIEWS);
    assert!(
        records.iter().all(|record| record.run_id != extra_run_id),
        "the saturated run must not be persisted"
    );
}

#[tokio::test]
async fn bounded_transcript_limit_skips_inference_and_persistence() {
    let tenant_id = TenantId::new("learning-tenant").expect("tenant");
    let user_id = UserId::new("learning-user").expect("user");
    let agent_id = AgentId::new("learning-agent").expect("agent");
    let thread_id = ThreadId::new("learning-thread").expect("thread");
    let run_id = TurnRunId::new();
    let inference = Arc::new(RecordingInference {
        calls: AtomicUsize::new(0),
        users: Mutex::new(Vec::new()),
        output: r#"{"memory":[],"skill":{"action":"skip"}}"#.to_string(),
    });
    let store = Arc::new(RecordingCandidateStore::default());
    let controller = Arc::new(LearningRuntimeControllerImpl::new(LearningSettings {
        enabled: true,
        model: Some("learning-model".to_string()),
        memory_write_policy: MemoryWritePolicy::Staged,
    }));
    let tasks = Arc::new(LearningReviewTasks::new());
    let sink = LearningReviewTurnEventSink::new(
        Arc::new(LimitExceededThreadService),
        inference.clone(),
        store.clone(),
        Arc::clone(&tasks),
        controller,
    );

    sink.publish(TurnLifecycleEvent {
        cursor: EventCursor::default(),
        scope: TurnScope::new_with_owner(
            tenant_id,
            Some(agent_id),
            None,
            thread_id,
            Some(user_id.clone()),
        ),
        occurred_at: None,
        owner_user_id: Some(user_id),
        run_id,
        status: TurnStatus::Completed,
        kind: TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        detail: None,
        retryable: None,
    })
    .await
    .expect("publish");
    tasks.wait().await;

    assert_eq!(inference.calls.load(Ordering::SeqCst), 0);
    assert!(store.records.lock().expect("records lock").is_empty());
}

#[tokio::test]
async fn completed_owned_run_routes_and_persists_a_candidate_record() {
    let tenant_id = TenantId::new("learning-tenant").expect("tenant");
    let user_id = UserId::new("learning-user").expect("user");
    let agent_id = AgentId::new("learning-agent").expect("agent");
    let thread_id = ThreadId::new("learning-thread").expect("thread");
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    let run_id = TurnRunId::new();
    let threads = Arc::new(InMemorySessionThreadService::default());
    threads
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope,
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");
    threads
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: ThreadScope {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: None,
                owner_user_id: Some(user_id.clone()),
                mission_id: None,
            },
            thread_id: thread_id.clone(),
            turn_run_id: run_id.to_string(),
            result_ref: "result:learning".to_string(),
            safe_summary: ToolResultSafeSummary::new("The user prefers concise status reports")
                .expect("summary"),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("append result");

    let inference = Arc::new(RecordingInference {
        calls: AtomicUsize::new(0),
        users: Mutex::new(Vec::new()),
        output: serde_json::json!({
            "memory": [{
                "kind": "preference",
                "content": "The user prefers concise status reports",
                "source_message_indices": [0],
                "confidence_basis_points": 9000,
                "explicitness": "explicit",
                "tainted": false
            }],
            "skill": {
                "action": "skip",
                "reason": null,
                "source_message_indices": []
            }
        })
        .to_string(),
    });
    let store = Arc::new(RecordingCandidateStore::default());
    store.records.lock().expect("records lock").push(
        LearningReviewRecord::new(
            TurnRunId::new(),
            LearningScope::new(tenant_id.clone(), user_id.clone(), agent_id.clone(), None),
            LearningReview {
                memory: vec![MemoryLearningProposal {
                    kind: MemoryLearningProposalKind::Fact,
                    content: "An unresolved prior candidate".to_string(),
                    source_message_indices: vec![0],
                    confidence_basis_points: 7_000,
                    explicitness: LearningExplicitness::Inferred,
                    tainted: false,
                }],
                skill: LearningDecision::skip(),
            },
        )
        .expect("prior record"),
    );
    let controller = Arc::new(LearningRuntimeControllerImpl::new(LearningSettings {
        enabled: true,
        model: Some("learning-model".to_string()),
        memory_write_policy: MemoryWritePolicy::Staged,
    }));
    let tasks = Arc::new(LearningReviewTasks::new());
    let sink = LearningReviewTurnEventSink::new(
        threads,
        inference.clone(),
        store.clone(),
        Arc::clone(&tasks),
        controller,
    );
    let event = TurnLifecycleEvent {
        cursor: EventCursor::default(),
        scope: TurnScope::new_with_owner(
            tenant_id,
            Some(agent_id),
            None,
            thread_id,
            Some(user_id.clone()),
        ),
        occurred_at: None,
        owner_user_id: Some(user_id),
        run_id,
        status: TurnStatus::Completed,
        kind: TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        detail: None,
        retryable: None,
    };
    sink.publish(event.clone()).await.expect("publish");
    sink.publish(event).await.expect("replay publish");

    tokio::time::timeout(std::time::Duration::from_secs(1), store.inserted.notified())
        .await
        .expect("candidate insert");
    tasks.wait().await;
    assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    {
        let users = inference.users.lock().expect("users lock");
        let input: serde_json::Value =
            serde_json::from_str(&users[0]).expect("learning input JSON");
        assert_eq!(
            input["unresolved_proposals"]
                .as_array()
                .expect("unresolved proposals")
                .len(),
            1
        );
    }
    {
        let records = store.records.lock().expect("records lock");
        assert_eq!(records.len(), 2);
        let record = records.last().expect("new record");
        assert_eq!(record.run_id, run_id);
        assert_eq!(record.scope.user_id().as_str(), "learning-user");
        assert_eq!(record.review.memory.len(), 1);
        assert!(
            record.review.memory[0].tainted,
            "the host must taint tool-derived proposals even when the model says false"
        );
    }
    tasks.shutdown().await;
}
