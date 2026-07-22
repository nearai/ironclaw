// arch-exempt: large_file, channel-neutral timeout and trigger-target regressions reuse the shared delivery harness, plan #4088
//! Contract rows for the generic run-delivery components (§5.4, 9b): the
//! live observer and the triggered driver, driven with scripted run states
//! and a scripted channel adapter, asserting at the coordinator/store seam.
//! The channel-level regression net (the vendor e2e scenarios through the
//! real ingress mount) re-points onto these components at the cutover.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ScopedPath, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    DeliveredGateRouteStore, DeliveryDefaultScope, FilesystemOutboundStateStore,
    OutboundStateStore, TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef,
    TriggerSourceKind, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryStore,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthPromptView, AuthRequirement, ChannelAdapter, ChannelError,
    DeliveryReport, ExternalActorRef, ExternalConversationRef, ExternalEventId, InboundOutcome,
    OutboundEnvelope, OutboundPart, ParsedProductInbound, PartDeliveryOutcome, ProductAdapterError,
    ProductAdapterId, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductTriggerReason, ProtocolAuthEvidence,
    TrustedInboundContext, UserMessagePayload, VerifiedInbound,
};
use ironclaw_product_workflow::{
    BlockedAuthPromptRequest, BlockedAuthPromptSource, ChannelConnectionNoticePolicy,
    ChannelDeliveryResolver, DeliveryCoordinator, DeliveryReplyContextSource, DeliveryRetryPolicy,
    PreferenceTargetCodec, ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind,
    ProjectFsError, ProjectFsStat, ResolvedChannelDelivery, RunDeliveryObserver,
    RunDeliveryServices, RunDeliverySettings, TriggeredRunDeliveryDriver,
    TriggeredRunDeliveryRequest, WorkspaceFile,
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
    MessageContent, SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError, TurnId, TurnRunId,
    TurnRunState, TurnScope, TurnStatus,
};

// ── Scripted fakes ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct ScriptedRunState {
    status: TurnStatus,
    gate_ref: Option<GateRef>,
}

fn scripted_state(status: TurnStatus, gate_ref: Option<&str>) -> ScriptedRunState {
    ScriptedRunState {
        status,
        gate_ref: gate_ref.map(|s| GateRef::new(s).expect("gate ref")),
    }
}

struct ScriptedTurnCoordinator {
    states: Vec<ScriptedRunState>,
    clamp_at_last: bool,
    calls: Mutex<usize>,
    cancel_calls: Mutex<Vec<TurnRunId>>,
}

impl ScriptedTurnCoordinator {
    fn with_states(states: Vec<ScriptedRunState>) -> Self {
        assert!(!states.is_empty());
        Self {
            states,
            clamp_at_last: true,
            calls: Mutex::new(0),
            cancel_calls: Mutex::new(Vec::new()),
        }
    }

    fn cancel_call_count(&self) -> usize {
        self.cancel_calls.lock().expect("cancel calls").len()
    }
}

#[async_trait]
impl TurnCoordinator for ScriptedTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "scripted".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "scripted".to_string(),
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "scripted".to_string(),
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let mut calls = self.calls.lock().expect("calls");
        let idx = if self.clamp_at_last {
            (*calls).min(self.states.len() - 1)
        } else {
            *calls % self.states.len()
        };
        *calls += 1;
        let scripted = self.states[idx].clone();
        Ok(TurnRunState {
            scope: request.scope.clone(),
            actor: None,
            turn_id: TurnId::new(),
            run_id: request.run_id,
            status: scripted.status,
            accepted_message_ref: AcceptedMessageRef::new("msg:scripted").expect("ref"),
            source_binding_ref: SourceBindingRef::new("src:scripted").expect("ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:test:scripted")
                .expect("ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            model_usage: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: scripted.gate_ref,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            product_context: None,
            resume_disposition: None,
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.cancel_calls
            .lock()
            .expect("cancel calls")
            .push(request.run_id);
        Ok(CancelRunResponse {
            run_id: request.run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor::default(),
            already_terminal: false,
            actor: None,
        })
    }
}

/// Scripted channel adapter recording every envelope; pops one report per
/// deliver, defaulting to a single `Sent` with a fresh vendor ref.
struct RecordingChannelAdapter {
    envelopes: Mutex<Vec<OutboundEnvelope>>,
    reports: Mutex<VecDeque<DeliveryReport>>,
    counter: Mutex<u64>,
    block_deliveries: AtomicBool,
    started_deliveries: AtomicUsize,
    delivery_release: tokio::sync::Semaphore,
}

impl RecordingChannelAdapter {
    fn new() -> Self {
        Self {
            envelopes: Mutex::new(Vec::new()),
            reports: Mutex::new(VecDeque::new()),
            counter: Mutex::new(0),
            block_deliveries: AtomicBool::new(false),
            started_deliveries: AtomicUsize::new(0),
            delivery_release: tokio::sync::Semaphore::new(0),
        }
    }

    fn block_deliveries(&self) {
        self.block_deliveries.store(true, Ordering::SeqCst);
    }

    async fn wait_for_started_deliveries(&self, expected: usize) {
        while self.started_deliveries.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    }

    fn release_deliveries(&self, count: usize) {
        self.delivery_release.add_permits(count);
    }

    fn envelopes(&self) -> Vec<OutboundEnvelope> {
        self.envelopes.lock().expect("envelopes").clone()
    }

    fn texts(&self) -> Vec<String> {
        self.envelopes()
            .iter()
            .flat_map(|envelope| {
                envelope.parts.iter().filter_map(|part| match part {
                    OutboundPart::Text(text) => Some(text.clone()),
                    _ => None,
                })
            })
            .collect()
    }

    fn retracted_refs(&self) -> Vec<String> {
        self.envelopes()
            .iter()
            .flat_map(|envelope| {
                envelope.parts.iter().filter_map(|part| match part {
                    OutboundPart::Retract { vendor_message_ref } => {
                        Some(vendor_message_ref.clone())
                    }
                    _ => None,
                })
            })
            .collect()
    }
}

#[async_trait]
impl ChannelAdapter for RecordingChannelAdapter {
    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Ok(InboundOutcome::Ignore)
    }

    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        _egress: &dyn ironclaw_host_api::RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        self.envelopes
            .lock()
            .expect("envelopes")
            .push(envelope.clone());
        if self.block_deliveries.load(Ordering::SeqCst) {
            self.started_deliveries.fetch_add(1, Ordering::SeqCst);
            let permit = self
                .delivery_release
                .acquire()
                .await
                .expect("test delivery semaphore remains open");
            permit.forget();
        }
        if let Some(report) = self.reports.lock().expect("reports").pop_front() {
            return Ok(report);
        }
        let mut counter = self.counter.lock().expect("counter");
        *counter += 1;
        Ok(DeliveryReport {
            parts: envelope
                .parts
                .iter()
                .map(|_| PartDeliveryOutcome::Sent {
                    vendor_message_ref: Some(format!("ts-{}", *counter)),
                })
                .collect(),
        })
    }
}

struct DenyAllEgress;

#[async_trait]
impl ironclaw_host_api::RestrictedEgress for DenyAllEgress {
    async fn send(
        &self,
        _request: ironclaw_host_api::RestrictedEgressRequest,
    ) -> Result<ironclaw_host_api::RestrictedEgressResponse, ironclaw_host_api::RestrictedEgressError>
    {
        Err(ironclaw_host_api::RestrictedEgressError::PolicyDenied)
    }
}

struct StaticResolver {
    adapter: Arc<RecordingChannelAdapter>,
}

impl ChannelDeliveryResolver for StaticResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        Some(ResolvedChannelDelivery {
            extension_id: extension_id.to_string(),
            installation_id: "install_alpha".to_string(),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(DenyAllEgress),
        })
    }
}

struct NoStoredReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for NoStoredReplyContext {
    async fn reply_context(&self, _: &str, _: &str, _: &str) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Default)]
struct ScriptedProjectFilesystemReader {
    files: Mutex<HashMap<String, Result<WorkspaceFile, ProjectFsError>>>,
    reads: Mutex<Vec<String>>,
}

impl ScriptedProjectFilesystemReader {
    fn insert_file(&self, path: &str, mime_type: &str, bytes: &[u8]) {
        self.files.lock().expect("files").insert(
            path.to_string(),
            Ok(WorkspaceFile {
                path: ScopedPath::new(path).expect("scoped workspace path"),
                filename: path.rsplit('/').next().map(str::to_string),
                mime_type: mime_type.to_string(),
                bytes: bytes.to_vec(),
            }),
        );
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("reads").clone()
    }
}

#[async_trait]
impl ProjectFilesystemReader for ScriptedProjectFilesystemReader {
    async fn list_dir(
        &self,
        _thread_scope: &ThreadScope,
        _path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
        Err(ProjectFsError::NotADirectory)
    }

    async fn read_file(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<WorkspaceFile, ProjectFsError> {
        self.reads.lock().expect("reads").push(path.to_string());
        self.files
            .lock()
            .expect("files")
            .get(path)
            .cloned()
            .unwrap_or(Err(ProjectFsError::NotFound))
    }

    async fn stat(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        match self
            .files
            .lock()
            .expect("files")
            .get(path)
            .cloned()
            .unwrap_or(Err(ProjectFsError::NotFound))
        {
            Ok(file) => Ok(ProjectFsStat {
                path: file.path.as_str().to_string(),
                kind: ProjectFsEntryKind::File,
                size_bytes: file.bytes.len() as u64,
                mime_type: file.mime_type,
            }),
            Err(error) => Err(error),
        }
    }
}

struct StaticBindingService {
    binding: ironclaw_product_workflow::ResolvedBinding,
    fail: bool,
}

#[async_trait]
impl ironclaw_product_workflow::ConversationBindingService for StaticBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product_workflow::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_workflow::ResolvedBinding,
        ironclaw_product_workflow::ProductWorkflowError,
    > {
        if self.fail {
            return Err(
                ironclaw_product_workflow::ProductWorkflowError::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product_workflow::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_workflow::ResolvedBinding,
        ironclaw_product_workflow::ProductWorkflowError,
    > {
        if self.fail {
            return Err(
                ironclaw_product_workflow::ProductWorkflowError::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }
}

struct OAuthPromptSource {
    authorization_url: Option<String>,
}

#[async_trait]
impl BlockedAuthPromptSource for OAuthPromptSource {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        Ok(AuthPromptView {
            turn_run_id: request.run_id,
            auth_request_ref: request.gate_ref.to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: request.body,
            challenge_kind: None,
            provider: None,
            account_label: None,
            authorization_url: self.authorization_url.clone(),
            expires_at: None,
            connection: None,
        })
    }
}

struct StaticCodec {
    conversation: ExternalConversationRef,
    personal_dm: bool,
}

impl PreferenceTargetCodec for StaticCodec {
    fn conversation_for_target(
        &self,
        _target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        Some(self.conversation.clone())
    }

    fn is_personal_direct_message(&self, _target: &ReplyTargetBindingRef) -> bool {
        self.personal_dm
    }

    fn direct_message_actor_for_target(&self, _target: &ReplyTargetBindingRef) -> Option<String> {
        None
    }

    fn encode_shared_conversation_target(
        &self,
        _request: ironclaw_product_workflow::PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }

    fn encode_personal_direct_message_target(
        &self,
        _request: ironclaw_product_workflow::PreferenceTargetEncodeRequest<'_>,
        _external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }
}

// ── Fixture helpers ────────────────────────────────────────────────────────

const EXTENSION_ID: &str = "acme";

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn user() -> UserId {
    UserId::new("user-a").expect("user")
}

fn agent() -> AgentId {
    AgentId::new("agent-a").expect("agent")
}

fn binding() -> ironclaw_product_workflow::ResolvedBinding {
    ironclaw_product_workflow::ResolvedBinding {
        tenant_id: tenant(),
        actor_user_id: user(),
        subject_user_id: Some(user()),
        thread_id: ThreadId::new("thread-a").expect("thread"),
        agent_id: Some(agent()),
        project_id: None,
    }
}

fn binding_scope() -> TurnScope {
    TurnScope::new_with_owner(
        tenant(),
        Some(agent()),
        None,
        ThreadId::new("thread-a").expect("thread"),
        Some(user()),
    )
}

fn fallback_scope() -> TurnScope {
    TurnScope::new_with_owner(
        tenant(),
        Some(agent()),
        None,
        ThreadId::new("channel-notices").expect("thread"),
        Some(user()),
    )
}

fn envelope_for_conversation(
    payload: ProductInboundPayload,
    event_id: &str,
    conversation_id: &str,
) -> ProductInboundEnvelope {
    let adapter_id = ProductAdapterId::new("acme_v1").expect("adapter");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("installation");
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Test-Signature".to_string(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        Utc::now(),
        &evidence,
    )
    .expect("trusted context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(event_id).expect("event"),
        ExternalActorRef::new("acme_user", "U-1", None::<String>).expect("actor"),
        ExternalConversationRef::new(Some("space-1"), conversation_id, None, None)
            .expect("conversation"),
        payload,
    )
    .expect("parsed");
    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

fn envelope(payload: ProductInboundPayload, event_id: &str) -> ProductInboundEnvelope {
    envelope_for_conversation(payload, event_id, "conv-1")
}

fn user_message_envelope(trigger: ProductTriggerReason, event_id: &str) -> ProductInboundEnvelope {
    envelope(
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", Vec::new(), trigger).expect("payload"),
        ),
        event_id,
    )
}

fn user_message_envelope_for_conversation(
    trigger: ProductTriggerReason,
    event_id: &str,
    conversation_id: &str,
) -> ProductInboundEnvelope {
    envelope_for_conversation(
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", Vec::new(), trigger).expect("payload"),
        ),
        event_id,
        conversation_id,
    )
}

fn accepted_ack(run_id: TurnRunId) -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("ref"),
        submitted_run_id: run_id,
    }
}

struct Harness {
    observer: Arc<RunDeliveryObserver>,
    connection_notices: ChannelConnectionNoticePolicy,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    route_store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    turns: Arc<ScriptedTurnCoordinator>,
    threads: Arc<InMemorySessionThreadService>,
    project_files: Arc<ScriptedProjectFilesystemReader>,
}

#[allow(clippy::too_many_arguments)]
fn build_harness(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
    max_wait: Duration,
) -> Harness {
    build_harness_with_settings(
        states,
        bind_fails,
        auth_url,
        RunDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait,
            max_concurrent_deliveries: NonZeroUsize::new(4).expect("nz"),
            max_pending_deliveries: NonZeroUsize::new(8).expect("nz"),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn build_harness_with_settings(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
    settings: RunDeliverySettings,
) -> Harness {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(states));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        Arc::new(StaticResolver {
            adapter: Arc::clone(&adapter),
        }),
        Arc::new(NoStoredReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 2,
            backoff: Duration::ZERO,
        },
    ));
    let services = RunDeliveryServices {
        binding_service: Arc::new(StaticBindingService {
            binding: binding(),
            fail: bind_fails,
        }),
        thread_service: Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        turn_coordinator: Arc::clone(&turns) as Arc<dyn TurnCoordinator>,
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        route_store: Arc::clone(&route_store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        project_filesystem: Arc::clone(&project_files) as Arc<dyn ProjectFilesystemReader>,
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts: auth_url.map(|url| {
            Arc::new(OAuthPromptSource {
                authorization_url: Some(url.to_string()),
            }) as Arc<dyn BlockedAuthPromptSource>
        }),
        auth_flow_cancel: None,
    };
    let connection_notices = ChannelConnectionNoticePolicy::generic("Acme");
    let observer = Arc::new(RunDeliveryObserver::with_settings_and_connection_notices(
        services,
        settings,
        connection_notices.clone(),
    ));
    Harness {
        observer,
        connection_notices,
        adapter,
        store,
        route_store,
        turns,
        threads,
        project_files,
    }
}

async fn seed_final_message(threads: &InMemorySessionThreadService, run_id: TurnRunId, text: &str) {
    let thread_scope = ThreadScope {
        tenant_id: tenant(),
        agent_id: agent(),
        project_id: None,
        owner_user_id: Some(user()),
        mission_id: None,
    };
    threads
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(ThreadId::new("thread-a").expect("thread")),
            created_by_actor_id: "user-a".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    threads
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: thread_scope,
            thread_id: ThreadId::new("thread-a").expect("thread"),
            turn_run_id: run_id.to_string(),
            content: MessageContent::text(text),
        })
        .await
        .expect("finalized");
}

// ── Observer rows ──────────────────────────────────────────────────────────

#[tokio::test]
async fn observer_delivers_final_reply_through_the_coordinator() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "hello from the run").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-final"),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts, vec!["hello from the run".to_string()]);
    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes[0].target.conversation.conversation_id(), "conv-1");
    assert_eq!(envelopes[0].extension_id, EXTENSION_ID);
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
    assert!(
        harness.project_files.reads().is_empty(),
        "text-only replies must not touch the workspace filesystem"
    );
}

#[tokio::test]
async fn observer_final_reply_materializes_valid_workspace_files_in_first_reference_order() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    harness
        .project_files
        .insert_file("/workspace/report.csv", "text/csv", b"a,b\n1,2\n");
    harness
        .project_files
        .insert_file("/workspace/charts/summary.png", "image/png", b"png");
    let run_id = TurnRunId::new();
    let text = "Files: /workspace/report.csv, `/workspace/secret.txt`, https://example.test/workspace/url.pdf, /workspace/../outside.txt, /workspace/report.csv, and /workspace/charts/summary.png.";
    seed_final_message(&harness.threads, run_id, text).await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-files"),
            accepted_ack(run_id),
        )
        .await;

    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert!(matches!(&envelopes[0].parts[0], OutboundPart::Text(body) if body == text));
    assert!(matches!(
        &envelopes[0].parts[1],
        OutboundPart::File(file)
            if file.path.as_str() == "/workspace/report.csv"
                && file.bytes == b"a,b\n1,2\n"
    ));
    assert!(matches!(
        &envelopes[0].parts[2],
        OutboundPart::File(file)
            if file.path.as_str() == "/workspace/charts/summary.png"
                && file.bytes == b"png"
    ));
    assert_eq!(
        harness.project_files.reads(),
        vec![
            "/workspace/report.csv".to_string(),
            "/workspace/charts/summary.png".to_string()
        ]
    );
}

/// Regression (the channel-host e2e race, made deterministic): a
/// gate-resolution ack carries the same submitted run id as the original
/// user-message ack. When it lands AFTER the original delivery loop already
/// posted the final reply and exited, the observer's delivered-run ledger
/// must skip it — the in-flight single-flight set alone cannot (the loop's
/// guard is gone by then), and the duplicate loop would immediately see the
/// run `Completed` and re-post the final reply.
#[tokio::test]
async fn observer_skips_resolution_ack_after_final_reply_was_delivered() {
    let harness = build_harness(
        vec![
            scripted_state(TurnStatus::Completed, None),
            scripted_state(TurnStatus::Completed, None),
        ],
        false,
        None,
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "approved and finished").await;

    // The original user-message loop delivers the final reply and exits,
    // releasing its single-flight claim.
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-user-msg"),
            accepted_ack(run_id),
        )
        .await;
    assert_eq!(
        harness.adapter.texts(),
        vec!["approved and finished".to_string()]
    );

    // The approval-resolution ack for the SAME run arrives after that exit.
    // Without the delivered-run ledger this spawned a second loop that saw
    // `Completed` and re-posted the final reply.
    let approve_envelope = envelope(
        ProductInboundPayload::ApprovalResolution(
            ironclaw_product_adapters::ApprovalResolutionPayload::new(
                "gate-1",
                ironclaw_product_adapters::ApprovalDecision::ApproveOnce,
            )
            .expect("payload"),
        ),
        "evt-approve",
    );
    harness
        .observer
        .observe_ack(approve_envelope, accepted_ack(run_id))
        .await;

    assert_eq!(
        harness.adapter.texts(),
        vec!["approved and finished".to_string()],
        "a resolution ack landing after delivery must not re-post the final reply"
    );
}

#[tokio::test]
async fn observer_posts_working_indicator_and_retracts_it_after_final_reply() {
    let harness = build_harness(
        vec![
            // First entry feeds the foreign-run guard's existence check; the
            // wait loop then observes Running (posts the indicator) and
            // Completed.
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::Completed, None),
        ],
        false,
        None,
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "done thinking").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-working"),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(
        texts,
        vec![
            "Ironclaw is thinking...".to_string(),
            "done thinking".to_string()
        ]
    );
    // The working indicator's vendor ref came back through the coordinator
    // outcome and was retracted after the final reply (Cleanup intent).
    let retracted = harness.adapter.retracted_refs();
    assert_eq!(retracted.len(), 1, "exactly one retraction");
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    // working + final + cleanup, all coordinator-persisted.
    assert_eq!(attempts.len(), 3);
    assert!(
        attempts
            .iter()
            .all(|a| a.status == ironclaw_outbound::OutboundDeliveryStatus::Delivered)
    );
}

#[tokio::test(start_paused = true)]
async fn observer_keeps_watching_a_healthy_run_past_the_previous_two_minute_cutoff() {
    let settings = RunDeliverySettings::default();
    assert!(
        settings.max_wait > Duration::from_secs(2 * 60),
        "the live channel watcher must outlive a healthy run that exceeds the old two-minute cutoff"
    );

    // The foreign-run existence guard consumes the first state. The wait
    // loop then remains Running for more than two minutes of virtual time
    // before observing Completed. This is channel-neutral: every adapter
    // reaches final replies through this observer.
    let mut states = vec![scripted_state(TurnStatus::Running, None)];
    states.extend(std::iter::repeat_with(|| scripted_state(TurnStatus::Running, None)).take(32));
    states.push(scripted_state(TurnStatus::Completed, None));
    let harness = build_harness_with_settings(states, false, None, settings);
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "slow run finished").await;

    let started = tokio::time::Instant::now();
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-slow-run"),
            accepted_ack(run_id),
        )
        .await;

    assert!(
        tokio::time::Instant::now().duration_since(started) > Duration::from_secs(2 * 60),
        "the scripted run must cross the previous delivery deadline"
    );
    assert_eq!(
        harness.adapter.texts(),
        vec![
            "Ironclaw is thinking...".to_string(),
            "slow run finished".to_string()
        ]
    );
    assert_eq!(harness.adapter.retracted_refs().len(), 1);
}

#[tokio::test]
async fn observer_retracts_working_indicator_and_auth_prompt_after_auth_completion() {
    let harness = build_harness(
        vec![
            // Existence guard, first blocked state, resumed running state,
            // then the terminal state that owns cleanup.
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-cleanup")),
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::Completed, None),
        ],
        false,
        Some("https://provider.example/oauth"),
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "authenticated and finished").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-auth-cleanup"),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 3, "auth prompt + working + final reply");
    assert!(texts[0].contains("Authentication required"));
    assert_eq!(texts[1], "Ironclaw is thinking...");
    assert_eq!(texts[2], "authenticated and finished");
    assert_eq!(
        harness.adapter.retracted_refs(),
        vec!["ts-2".to_string(), "ts-1".to_string()],
        "terminal delivery retracts the working indicator and then the stale auth prompt"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 5, "three posts plus two cleanup calls");
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.status == ironclaw_outbound::OutboundDeliveryStatus::Delivered)
    );
}

#[tokio::test]
async fn observer_records_gate_route_after_approval_prompt() {
    let harness = build_harness(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000001"),
        )],
        false,
        None,
        Duration::from_millis(40),
    );
    let run_id = TurnRunId::new();

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-gate"),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1, "exactly one approval prompt");
    assert!(texts[0].contains("Approval needed"), "{}", texts[0]);
    assert!(
        texts[0].contains("`approve` or `deny`"),
        "reply instruction present: {}",
        texts[0]
    );
    let route = harness
        .route_store
        .load_delivered_gate_route(
            &tenant(),
            &user(),
            "gate:approval-00000000000000000000000000000001",
        )
        .await
        .expect("route lookup")
        .expect("gate route recorded");
    assert_eq!(route.run_id, run_id);
    assert!(
        !route.delivered_conversation_fingerprints.is_empty(),
        "fingerprints recorded"
    );
    // The source conversation (bare replies next to the prompt) routes too.
    let source_fingerprint =
        ironclaw_conversations::ExternalConversationRef::new(Some("space-1"), "conv-1", None, None)
            .expect("conversation")
            .conversation_fingerprint();
    assert!(
        route
            .delivered_conversation_fingerprints
            .contains(&source_fingerprint),
        "source conversation fingerprint recorded"
    );
}

#[tokio::test]
async fn observer_connect_nudge_posts_only_for_direct_chat_binding_required() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Running, None)],
        true,
        None,
        Duration::from_millis(20),
    );
    let rejected = ProductInboundAck::Rejected(ProductRejection::permanent(
        ProductRejectionKind::BindingRequired,
        "unbound",
    ));

    // Shared-channel origin: no nudge.
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::BotMention, "evt-shared"),
            rejected.clone(),
        )
        .await;
    assert!(harness.adapter.texts().is_empty(), "no nudge into shared");

    // 1:1 direct chat origin: nudge posted under the fallback notice scope.
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-dm"),
            rejected.clone(),
        )
        .await;
    // A distinct transport event in the same conversation stays throttled.
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-dm-2"),
            rejected.clone(),
        )
        .await;
    // A distinct direct conversation owns an independent reservation.
    harness
        .observer
        .observe_ack(
            user_message_envelope_for_conversation(
                ProductTriggerReason::DirectChat,
                "evt-dm-other",
                "conv-2",
            ),
            rejected,
        )
        .await;
    let texts = harness.adapter.texts();
    assert_eq!(
        texts,
        vec![
            harness.connection_notices.connect_required.clone(),
            harness.connection_notices.connect_required.clone(),
        ]
    );
    let attempts = harness
        .store
        .list_delivery_attempts(fallback_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2, "one nudge attempt per conversation");
    assert_eq!(
        attempts[0].candidate.kind,
        ironclaw_outbound::OutboundPushKind::DeliveryStatus
    );
}

#[tokio::test]
async fn observer_connect_nudge_reopens_after_connected_message_is_accepted() {
    let run_id = TurnRunId::new();
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        true,
        None,
        Duration::from_millis(20),
    );
    let rejected = ProductInboundAck::Rejected(ProductRejection::permanent(
        ProductRejectionKind::BindingRequired,
        "unbound",
    ));

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-before-connect"),
            rejected.clone(),
        )
        .await;
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-connected"),
            accepted_ack(run_id),
        )
        .await;
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-after-disconnect"),
            rejected,
        )
        .await;

    let connect_notices = harness
        .adapter
        .texts()
        .into_iter()
        .filter(|text| text == &harness.connection_notices.connect_required)
        .count();
    assert_eq!(
        connect_notices, 2,
        "a successful connected message must close the prior unbound throttle epoch"
    );
}

#[tokio::test]
async fn observer_connect_nudge_releases_failed_delivery_reservation_for_retry() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Running, None)],
        true,
        None,
        Duration::from_millis(20),
    );
    harness
        .adapter
        .reports
        .lock()
        .expect("reports lock")
        .push_back(DeliveryReport {
            parts: vec![PartDeliveryOutcome::Permanent {
                reason: "scripted failure".to_string(),
            }],
        });
    let rejected = ProductInboundAck::Rejected(ProductRejection::permanent(
        ProductRejectionKind::BindingRequired,
        "unbound",
    ));

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-failed"),
            rejected.clone(),
        )
        .await;
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-retry"),
            rejected,
        )
        .await;

    let envelopes = harness.adapter.envelopes();
    assert_eq!(
        envelopes.len(),
        2,
        "failed evidence must release reservation"
    );
    assert!(envelopes.iter().all(|envelope| {
        matches!(
            envelope.parts.as_slice(),
            [OutboundPart::Text(text)] if text == &harness.connection_notices.connect_required
        )
    }));
    let attempts = harness
        .store
        .list_delivery_attempts(fallback_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2);
    assert!(matches!(
        attempts.last().map(|attempt| attempt.status),
        Some(ironclaw_outbound::OutboundDeliveryStatus::Delivered)
    ));
}

#[tokio::test]
async fn observer_connect_nudge_saturation_does_not_evict_in_flight_reservations() {
    const RESERVATION_CAP: usize = 1024;

    let harness = build_harness(
        vec![scripted_state(TurnStatus::Running, None)],
        true,
        None,
        Duration::from_millis(20),
    );
    harness.adapter.block_deliveries();
    let rejected = ProductInboundAck::Rejected(ProductRejection::permanent(
        ProductRejectionKind::BindingRequired,
        "unbound",
    ));
    let mut deliveries = Vec::with_capacity(RESERVATION_CAP);
    for index in 0..RESERVATION_CAP {
        let observer = Arc::clone(&harness.observer);
        let rejected = rejected.clone();
        deliveries.push(tokio::spawn(async move {
            observer
                .observe_ack(
                    user_message_envelope_for_conversation(
                        ProductTriggerReason::DirectChat,
                        &format!("evt-cap-{index}"),
                        &format!("conv-cap-{index}"),
                    ),
                    rejected,
                )
                .await;
        }));
    }
    tokio::time::timeout(
        Duration::from_secs(10),
        harness.adapter.wait_for_started_deliveries(RESERVATION_CAP),
    )
    .await
    .expect("all capped reservations reach the blocked delivery seam");

    let observer = Arc::clone(&harness.observer);
    let mut overflow = tokio::spawn(async move {
        observer
            .observe_ack(
                user_message_envelope_for_conversation(
                    ProductTriggerReason::DirectChat,
                    "evt-cap-overflow",
                    "conv-cap-overflow",
                ),
                rejected,
            )
            .await;
    });
    let overflow_reached_delivery = tokio::select! {
        result = &mut overflow => {
            result.expect("overflow observer task completes");
            false
        }
        () = harness.adapter.wait_for_started_deliveries(RESERVATION_CAP + 1) => true,
    };

    harness.adapter.release_deliveries(RESERVATION_CAP + 1);
    for delivery in deliveries {
        delivery.await.expect("capped observer task completes");
    }
    if overflow_reached_delivery {
        overflow
            .await
            .expect("overflow observer task completes after release");
    }

    assert!(
        !overflow_reached_delivery,
        "a full reservation map must fail closed instead of evicting an in-flight nudge"
    );
}

#[tokio::test]
async fn observer_busy_hint_deduplicates_per_conversation_event_pair() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Running, None)],
        false,
        None,
        Duration::from_millis(20),
    );
    let active_run = TurnRunId::new();
    let busy = ProductInboundAck::RejectedBusy {
        accepted_message_ref: AcceptedMessageRef::new("msg:busy").expect("ref"),
        active_run_id: Some(active_run),
    };

    let envelope = user_message_envelope(ProductTriggerReason::DirectChat, "evt-busy");
    harness
        .observer
        .observe_ack(envelope.clone(), busy.clone())
        .await;
    // Transport retry of the same event: suppressed.
    harness.observer.observe_ack(envelope, busy.clone()).await;
    assert_eq!(harness.adapter.texts().len(), 1, "one hint per event");

    // A NEW event for the same conversation gets a fresh hint.
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-busy-2"),
            busy,
        )
        .await;
    assert_eq!(harness.adapter.texts().len(), 2, "fresh event, fresh hint");
}

#[tokio::test]
async fn observer_auth_prompt_includes_setup_link_only_in_direct_chat() {
    // Direct chat: the OAuth URL survives.
    let harness = build_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-1"))],
        false,
        Some("https://provider.example/oauth"),
        Duration::from_millis(40),
    );
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-auth-dm"),
            accepted_ack(TurnRunId::new()),
        )
        .await;
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Authentication required"), "{}", texts[0]);
    assert!(
        texts[0].contains("Setup link: https://provider.example/oauth"),
        "{}",
        texts[0]
    );

    // Shared-channel origin: URL stripped, prompt still posted.
    let harness = build_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-2"))],
        false,
        Some("https://provider.example/oauth"),
        Duration::from_millis(40),
    );
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::BotMention, "evt-auth-shared"),
            accepted_ack(TurnRunId::new()),
        )
        .await;
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Authentication required"), "{}", texts[0]);
    assert!(!texts[0].contains("Setup link:"), "{}", texts[0]);
}

#[tokio::test]
async fn observer_non_oauth_auth_block_cancels_run_and_posts_unavailable_notice() {
    // No auth-prompt source wired → fail closed: cancel + notice.
    let harness = build_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-3"))],
        false,
        None,
        Duration::from_millis(40),
    );
    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-auth-deny"),
            accepted_ack(TurnRunId::new()),
        )
        .await;
    assert_eq!(harness.turns.cancel_call_count(), 1, "run cancelled");
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Ironclaw web app"), "{}", texts[0]);
}

// ── Triggered rows ─────────────────────────────────────────────────────────

fn trigger_context() -> TriggerCommunicationContext {
    TriggerCommunicationContext {
        trigger_origin_ref: TriggerOriginRef::new("trigger:test").expect("origin"),
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot: TriggerFireSlot::new("2026-07-12T09:00:00Z").expect("slot"),
    }
}

fn triggered_request(run_id: TurnRunId, project_scoped: bool) -> TriggeredRunDeliveryRequest {
    TriggeredRunDeliveryRequest {
        run_id,
        scope: binding_scope(),
        creator_user_id: user(),
        project_scoped,
        prompt: "watch the deploys".to_string(),
        delivery_target: None,
        trigger_context: trigger_context(),
    }
}

struct TriggeredHarness {
    driver: TriggeredRunDeliveryDriver,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    delivery_store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    turns: Arc<ScriptedTurnCoordinator>,
    threads: Arc<InMemorySessionThreadService>,
    project_files: Arc<ScriptedProjectFilesystemReader>,
}

fn build_triggered_harness(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    personal_dm_target: bool,
) -> TriggeredHarness {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let delivery_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(states));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        Arc::new(StaticResolver {
            adapter: Arc::clone(&adapter),
        }),
        Arc::new(NoStoredReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 2,
            backoff: Duration::ZERO,
        },
    ));
    let services = RunDeliveryServices {
        binding_service: Arc::new(StaticBindingService {
            binding: binding(),
            fail: true,
        }),
        thread_service: Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        turn_coordinator: Arc::clone(&turns) as Arc<dyn TurnCoordinator>,
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        route_store: Arc::clone(&route_store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        project_filesystem: Arc::clone(&project_files) as Arc<dyn ProjectFilesystemReader>,
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts: auth_url.map(|url| {
            Arc::new(OAuthPromptSource {
                authorization_url: Some(url.to_string()),
            }) as Arc<dyn BlockedAuthPromptSource>
        }),
        auth_flow_cancel: None,
    };
    let driver = TriggeredRunDeliveryDriver::with_settings(
        services,
        RunDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait: Duration::from_millis(60),
            max_concurrent_deliveries: NonZeroUsize::new(4).expect("nz"),
            max_pending_deliveries: NonZeroUsize::new(8).expect("nz"),
        },
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        Arc::new(StaticCodec {
            conversation: ExternalConversationRef::new(Some("space-1"), "dm-creator", None, None)
                .expect("conversation"),
            personal_dm: personal_dm_target,
        }),
        agent(),
    );
    TriggeredHarness {
        driver,
        adapter,
        store,
        delivery_store,
        turns,
        threads,
        project_files,
    }
}

async fn seed_preference(
    store: &FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
) {
    store
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant(), user()),
            final_reply_target: Some(ReplyTargetBindingRef::new("reply:pref").expect("ref")),
            progress_target: None,
            approval_prompt_target: Some(ReplyTargetBindingRef::new("reply:pref").expect("ref")),
            auth_prompt_target: Some(ReplyTargetBindingRef::new("reply:pref").expect("ref")),
            default_modality: Some(CommunicationModality::Text),
            updated_at: Utc::now(),
            updated_by: user(),
        })
        .await
        .expect("preference");
}

async fn wait_for_outcome(
    store: &FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
    run_id: TurnRunId,
) -> TriggeredRunDeliveryOutcomeKind {
    for _ in 0..500 {
        if let Some(record) = store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load outcome")
        {
            return record.outcome;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("no triggered delivery outcome recorded for {run_id}");
}

#[tokio::test]
async fn triggered_project_scoped_fire_is_denied_without_delivery() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        true,
    );
    let run_id = TurnRunId::new();
    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, true))
        .await;
    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Denied);
    assert!(harness.adapter.texts().is_empty(), "nothing delivered");
}

#[tokio::test]
async fn triggered_final_reply_reaches_the_preference_target_with_footer() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        true,
    );
    // Preferences live on the outbound store; seed the creator's target and
    // pin that the driver resolves from the SAME store handle.
    assert!(Arc::ptr_eq(
        &(Arc::clone(&harness.store) as Arc<dyn CommunicationPreferenceRepository>),
        &harness.driver.communication_preferences()
    ));
    seed_preference(&harness.store).await;
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "deploy watch complete").await;

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;
    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].starts_with("deploy watch complete"),
        "{}",
        texts[0]
    );
    assert!(
        texts[0].contains("From a triggered event: “watch the deploys”."),
        "footer present: {}",
        texts[0]
    );
    let envelopes = harness.adapter.envelopes();
    assert_eq!(
        envelopes[0].target.conversation.conversation_id(),
        "dm-creator",
        "delivered to the decoded preference target"
    );
}

#[tokio::test]
async fn triggered_final_reply_materializes_workspace_files_before_adapter_delivery() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        true,
    );
    seed_preference(&harness.store).await;
    harness.project_files.insert_file(
        "/workspace/trigger.json",
        "application/json",
        br#"{"ok":true}"#,
    );
    let run_id = TurnRunId::new();
    seed_final_message(
        &harness.threads,
        run_id,
        "trigger complete: /workspace/trigger.json",
    )
    .await;

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;
    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    let envelopes = harness.adapter.envelopes();
    assert!(matches!(
        &envelopes[0].parts[1],
        OutboundPart::File(file)
            if file.path.as_str() == "/workspace/trigger.json"
                && file.bytes == br#"{"ok":true}"#
    ));
}

#[tokio::test]
async fn triggered_final_reply_honors_per_trigger_target_without_global_default() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        true,
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "pinned route complete").await;
    let mut request = triggered_request(run_id, false);
    request.delivery_target =
        Some(ReplyTargetBindingRef::new("reply:pinned-trigger").expect("target"));

    harness.driver.on_trigger_submitted(request).await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_eq!(harness.adapter.texts().len(), 1);
}

#[tokio::test]
async fn triggered_delivery_is_skipped_when_the_pending_queue_is_full() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        true,
    );
    // Exhaust the pending-admission queue (capacity 8 in this harness).
    let mut held = Vec::new();
    while let Some(permit) = harness.driver.try_acquire_pending_permit() {
        held.push(permit);
    }
    let run_id = TurnRunId::new();
    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;
    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Skipped);
    assert!(harness.adapter.texts().is_empty(), "nothing delivered");
}

#[tokio::test]
async fn triggered_oauth_prompt_to_non_dm_target_cancels_and_notifies() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-t"))],
        Some("https://provider.example/oauth"),
        false,
    );
    seed_preference(&harness.store).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;
    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_eq!(
        harness.turns.cancel_call_count(),
        1,
        "blocked run cancelled"
    );
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1, "only the auth-unavailable notice");
    assert!(!texts[0].contains("Setup link:"), "{}", texts[0]);
    assert!(texts[0].contains("Ironclaw web app"), "{}", texts[0]);
}
